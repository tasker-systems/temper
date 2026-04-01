use std::path::{Path, PathBuf};

use temper_core::types::config::UnifiedConfig;

use crate::error::{Result, TemperError};

// ---------------------------------------------------------------------------
// Global config type alias
// ---------------------------------------------------------------------------

/// The deserialized global config. Re-export for convenience.
pub type GlobalConfig = UnifiedConfig;

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
    pub skill_framework: String,
}

impl Config {
    /// Compute the directory for a given context + doc_type.
    /// Returns `vault_root/{context}/{doc_type}/`
    pub fn doc_type_dir(&self, context: &str, doc_type: &str) -> PathBuf {
        self.vault_root.join(context).join(doc_type)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Expand a leading `~/` to the user's home directory.
pub fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    } else if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    PathBuf::from(path)
}

/// Path to the global config file (~/.config/temper/config.toml).
pub fn global_config_path() -> PathBuf {
    if let Ok(p) = std::env::var("TEMPER_GLOBAL_CONFIG") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    expand_tilde("~/.config/temper/config.toml")
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
pub fn load_global_config() -> Result<GlobalConfig> {
    let path = global_config_path();
    if !path.exists() {
        return Err(TemperError::Config(format!(
            "global config not found: {}. Run 'temper init' first.",
            path.display()
        )));
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| TemperError::Config(format!("cannot read {}: {}", path.display(), e)))?;
    let cfg: GlobalConfig = toml::from_str(&content)?;
    Ok(cfg)
}

/// 3-step vault resolution (no CWD walk-up):
///   1. CLI --vault flag
///   2. TEMPER_VAULT env var
///   3. Global config [vault].path
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

/// Resolve vault + build Config from global config.
pub fn load(cli_vault: Option<&str>) -> Result<Config> {
    let global = load_global_config()?;

    let vault_root = if let Some(v) = cli_vault {
        expand_tilde(v)
    } else if let Ok(v) = std::env::var("TEMPER_VAULT") {
        if !v.is_empty() {
            expand_tilde(&v)
        } else {
            expand_tilde(&global.vault.path)
        }
    } else {
        expand_tilde(&global.vault.path)
    };

    Ok(Config {
        state_dir: vault_root.join(".temper"),
        vault_root,
        contexts: global.sync.subscriptions.contexts.clone(),
        skill_output: expand_tilde(&global.skill.output),
        skill_framework: global.skill.framework.clone(),
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
