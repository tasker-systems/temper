use std::path::{Path, PathBuf};

use temper_core::types::config::TemperConfig;

use crate::error::{Result, TemperError};

// ---------------------------------------------------------------------------
// Re-exports from temper-core for backward compatibility
// ---------------------------------------------------------------------------

pub use temper_core::types::config::{expand_tilde, global_config_path};

// ---------------------------------------------------------------------------
// Global config type alias
// ---------------------------------------------------------------------------

/// The deserialized global config. Re-export for convenience.
pub type GlobalConfig = TemperConfig;

// ---------------------------------------------------------------------------
// Resolved runtime config
// ---------------------------------------------------------------------------

/// Resolved runtime configuration built from GlobalConfig.
#[derive(Debug, Clone)]
pub struct Config {
    pub vault_root: PathBuf,
    pub state_dir: PathBuf,
    pub contexts: Vec<String>,
    pub skill_output: PathBuf,
    /// The user's profile slug (cached from `client.profile().get()`).
    /// `None` until the first authenticated CLI invocation populates it.
    pub profile_slug: Option<String>,
}

/// Load the device UUID from auth.json's `device_id` field.
///
/// Returns `None` when not authenticated or if the stored auth predates
/// the device_id field.
pub fn load_device_id() -> Option<String> {
    temper_client::auth::load_device_id()
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Load and parse the global config from ~/.config/temper/config.toml.
///
/// Unlike `temper_core::types::config::load_config()` which returns defaults
/// when the file is absent, this returns an error directing the user to run
/// `temper init` — appropriate for the CLI where config must exist.
pub fn load_global_config() -> Result<GlobalConfig> {
    let path = global_config_path();
    if !path.exists() {
        return Err(TemperError::Config(format!(
            "global config not found: {}. Run 'temper init' first \
             (or name the vault with --vault / TEMPER_VAULT).",
            path.display()
        )));
    }
    temper_core::types::config::load_config_from(&path).map_err(TemperError::Config)
}

/// Load the global config, defaulting only when the file is ABSENT.
///
/// `temper_core::types::config::load_config` already returns defaults for an
/// absent file, so an `Err` here means the file exists but could not be read or
/// parsed. Substituting defaults for that case would silently replace the
/// user's configured format/color/limits with the built-in ones, so the error
/// propagates instead — naming the file and the parse reason.
pub fn load_global_config_defaulting_when_absent() -> Result<GlobalConfig> {
    temper_core::types::config::load_config().map_err(TemperError::Config)
}

/// 3-step vault resolution (no CWD walk-up):
///   1. CLI --vault flag
///   2. TEMPER_VAULT env var
///   3. Global config `[vault].path`
pub fn resolve_vault(cli_vault: Option<&str>) -> Result<PathBuf> {
    if let Some(v) = cli_vault {
        return Ok(expand_tilde(v));
    }
    if let Ok(v) = std::env::var("TEMPER_VAULT") {
        if !v.is_empty() {
            return Ok(expand_tilde(&v));
        }
    }
    let global = load_global_config()?;
    Ok(expand_tilde(&global.vault.path))
}

/// Build Config from an explicit TemperConfig + vault override (no disk reads).
pub fn load_from(global: &TemperConfig, cli_vault: Option<&str>) -> Config {
    let vault_root = cli_vault
        .map(expand_tilde)
        .unwrap_or_else(|| expand_tilde(&global.vault.path));
    Config {
        state_dir: vault_root.join(".temper"),
        vault_root,
        contexts: global.sync.subscriptions.contexts.clone(),
        skill_output: expand_tilde(&global.skill.output),
        profile_slug: None,
    }
}

/// Resolve vault + build Config from global config.
///
/// When the vault is named explicitly (`--vault` or `TEMPER_VAULT`), an absent
/// config file reads as defaults: everything else it holds is optional, so a
/// host configured only through the environment (a cloud agent session:
/// `TEMPER_VAULT`, `TEMPER_API_URL`, `TEMPER_AUTH`) needs no `temper init`. A
/// file that exists but does not parse is still refused. With no vault named,
/// the file is required — the default vault path is a guess, and `init` is
/// where a person picks the real one.
pub fn load(cli_vault: Option<&str>) -> Result<Config> {
    let explicit_vault = match cli_vault {
        Some(v) => Some(expand_tilde(v)),
        None => std::env::var("TEMPER_VAULT")
            .ok()
            .filter(|v| !v.is_empty())
            .map(|v| expand_tilde(&v)),
    };

    let global = if explicit_vault.is_some() {
        load_global_config_defaulting_when_absent()?
    } else {
        load_global_config()?
    };

    let vault_root = explicit_vault.unwrap_or_else(|| expand_tilde(&global.vault.path));

    Ok(Config {
        state_dir: vault_root.join(".temper"),
        vault_root,
        contexts: global.sync.subscriptions.contexts.clone(),
        skill_output: expand_tilde(&global.skill.output),
        profile_slug: None,
    })
}

/// Safe-write protocol: read -> validate original TOML -> apply transform ->
/// validate result -> atomic write via tmp + rename.
pub fn safe_write(path: &Path, transform: impl FnOnce(String) -> String) -> Result<()> {
    let original = std::fs::read_to_string(path)
        .map_err(|e| TemperError::Config(format!("safe_write read error: {}", e)))?;

    // Validate original parses as TOML
    toml::from_str::<toml::Value>(&original)?;

    let transformed = transform(original);

    // Validate transformed parses as TOML
    toml::from_str::<toml::Value>(&transformed)?;

    // Atomic write: write to tmp, then rename
    let tmp_path = path.with_extension("toml.tmp");
    std::fs::write(&tmp_path, &transformed)
        .map_err(|e| TemperError::Config(format!("safe_write tmp write error: {}", e)))?;
    std::fs::rename(&tmp_path, path)
        .map_err(|e| TemperError::Config(format!("safe_write rename error: {}", e)))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config that exists but cannot be parsed must be refused, naming the
    /// file — not silently replaced by defaults, which would swap the user's
    /// configured format/color/limits for the built-in ones.
    #[test]
    fn malformed_config_errors_naming_the_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, "[vault\npath = \"x\"\n").unwrap();

        let result = temp_env::with_vars(
            [("TEMPER_GLOBAL_CONFIG", Some(config_path.to_str().unwrap()))],
            load_global_config_defaulting_when_absent,
        );

        let err = result.expect_err("a malformed config must not read as defaults");
        let msg = format!("{err}");
        assert!(
            msg.contains(config_path.to_str().unwrap()),
            "the error must name the config file: {msg}"
        );
    }

    /// The absent-file arm is deliberate: no config file is the ordinary fresh
    /// state, and it must keep reading as defaults.
    #[test]
    fn an_absent_config_still_defaults() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("nope.toml");

        let result = temp_env::with_vars(
            [("TEMPER_GLOBAL_CONFIG", Some(config_path.to_str().unwrap()))],
            load_global_config_defaulting_when_absent,
        );

        let cfg = result.expect("an absent config file must still default");
        assert_eq!(cfg.vault.path, TemperConfig::default().vault.path);
    }

    /// A named vault makes the config file optional: an env-only host (a cloud
    /// agent session) runs vault-backed commands without `temper init`.
    #[test]
    fn a_named_vault_needs_no_config_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("absent.toml");
        let vault = dir.path().join("vault");

        let cfg = temp_env::with_vars(
            [
                ("TEMPER_GLOBAL_CONFIG", Some(config_path.to_str().unwrap())),
                ("TEMPER_VAULT", Some(vault.to_str().unwrap())),
            ],
            || load(None),
        )
        .expect("TEMPER_VAULT with no config file must load defaults");

        assert_eq!(cfg.vault_root, vault);
        assert_eq!(cfg.state_dir, vault.join(".temper"));
        assert_eq!(
            cfg.contexts,
            TemperConfig::default().sync.subscriptions.contexts
        );

        let cfg = temp_env::with_vars(
            [
                ("TEMPER_GLOBAL_CONFIG", Some(config_path.to_str().unwrap())),
                ("TEMPER_VAULT", None),
            ],
            || load(Some(vault.to_str().unwrap())),
        )
        .expect("--vault with no config file must load defaults");
        assert_eq!(cfg.vault_root, vault);
    }

    /// With no vault named, the default path is a guess: the file stays
    /// required, and the refusal names both ways out.
    #[test]
    fn no_named_vault_still_requires_the_config_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("absent.toml");

        let err = temp_env::with_vars(
            [
                ("TEMPER_GLOBAL_CONFIG", Some(config_path.to_str().unwrap())),
                ("TEMPER_VAULT", None),
            ],
            || load(None),
        )
        .expect_err("no vault and no config must be refused");

        let msg = format!("{err}");
        assert!(msg.contains("temper init"), "names init: {msg}");
        assert!(msg.contains("TEMPER_VAULT"), "names TEMPER_VAULT: {msg}");
    }

    /// A named vault does not launder a broken file into defaults.
    #[test]
    fn a_named_vault_still_refuses_a_malformed_config() {
        let dir = tempfile::TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, "[vault\npath = \"x\"\n").unwrap();

        let result = temp_env::with_vars(
            [
                ("TEMPER_GLOBAL_CONFIG", Some(config_path.to_str().unwrap())),
                ("TEMPER_VAULT", Some(dir.path().to_str().unwrap())),
            ],
            || load(None),
        );

        let err = result.expect_err("a malformed config must not read as defaults");
        assert!(format!("{err}").contains(config_path.to_str().unwrap()));
    }
}
