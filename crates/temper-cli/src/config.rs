use std::path::{Path, PathBuf};

use temper_core::types::config::TemperConfig;
use temper_core::types::vault_config::Subscription;

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
    pub subscriptions: Vec<Subscription>,
    pub skill_output: PathBuf,
    /// The user's profile slug (cached from `client.profile().get()`),
    /// used by `lookup::find_resource` to scan the legacy
    /// `@<profile.slug>/` directory for files written during the
    /// PR #70 / PR #72 window. `None` until the first authenticated
    /// CLI invocation populates it (lazy-cache wiring deferred to a
    /// follow-up task; until then legacy fallback is a no-op).
    pub profile_slug: Option<String>,
}

impl Config {
    /// Look up the subscription for a given context name.
    /// Returns `None` if the context has no subscription configured.
    pub fn subscription_for_context(&self, context: &str) -> Option<&Subscription> {
        self.subscriptions.iter().find(|s| s.context == context)
    }

    /// Resolve the owner string for a given context via its subscription.
    /// Falls back to `@me` if no subscription is configured for the context.
    pub fn owner_for_context(&self, context: &str) -> String {
        self.subscription_for_context(context)
            .map(|s| s.resolved_owner())
            .unwrap_or_else(|| "@me".to_string())
    }
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
            "global config not found: {}. Run 'temper init' first.",
            path.display()
        )));
    }
    temper_core::types::config::load_config_from(&path).map_err(TemperError::Config)
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
        // Populated in a future session once vault_config sync lands; until
        // then owner_for_context falls back to "@me".
        subscriptions: Vec::new(),
        skill_output: expand_tilde(&global.skill.output),
        profile_slug: None,
    }
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
        // Populated in a future session once vault_config sync lands; until
        // then owner_for_context falls back to "@me".
        subscriptions: Vec::new(),
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
