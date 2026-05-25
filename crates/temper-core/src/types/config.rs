use serde::{Deserialize, Serialize};
use validator::Validate;

/// Environment variable that overrides the on-disk auth file location used by
/// `DiskTokenStore`. Resolution precedence (highest to lowest): this env var,
/// `auth.path` in `config.toml`, default (`~/.config/temper/auth.json`).
///
/// Cloud sessions never consult this — they read tokens from `TEMPER_TOKEN`
/// via `MemoryTokenStore` and must not touch disk regardless.
pub const TEMPER_AUTH_PATH_ENV: &str = "TEMPER_AUTH_PATH";

/// Merge policy for conflict resolution within a subscription scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum MergePolicy {
    /// Require explicit resolution via `temper sync resolve`
    #[default]
    Manual,
    /// Auto-merge: keep both contributions with section attribution
    Auto,
}

/// A sync subscription — defines which resources to materialize locally.
///
/// Subscriptions scope `temper sync` to specific contexts, teams, and/or
/// doc types. Resources matching any subscription are included in sync.
/// Stored in `config.toml` under `[[sync.subscriptions]]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSubscription {
    /// Context name to subscribe to (e.g., "temper", "tasker")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Team slug to subscribe to (e.g., "platform-team")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    /// Optional doc type filter (e.g., ["research", "concept"])
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub doc_types: Vec<String>,
    /// Merge policy for conflicts in this subscription scope
    #[serde(default)]
    pub merge: MergePolicy,
}

/// Sync configuration section of config.toml.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncConfig {
    /// Whether to run local-only manifest pre-flight on every temper command
    #[serde(default)]
    pub auto: bool,
    /// Resource subscriptions — what to materialize locally
    #[serde(default)]
    pub subscriptions: Vec<SyncSubscription>,
}

/// Cloud-aware configuration — `~/.config/temper/config.toml`.
///
/// Separate from the vault-local `temper.toml` (which configures vault
/// directories and indexing). This config holds auth, sync, and CLI preferences
/// for the cloud-connected temper experience.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudConfig {
    pub vault: CloudVaultConfig,
    #[serde(default)]
    pub sync: SyncConfig,
}

/// Vault path reference in cloud config.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CloudVaultConfig {
    /// Path to the local vault directory
    #[validate(length(min = 1, message = "vault path cannot be empty"))]
    pub path: String,
}

/// Sync subscriptions — which contexts are synced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncSubscriptions {
    #[serde(default)]
    pub contexts: Vec<String>,
}

/// Sync config — which contexts are synced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UnifiedSyncConfig {
    #[serde(default)]
    pub subscriptions: SyncSubscriptions,
}

/// Skill generation config.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct SkillConfig {
    #[serde(default = "default_skill_output")]
    #[validate(length(min = 1, message = "skill output path cannot be empty"))]
    pub output: String,
}

fn default_skill_output() -> String {
    "~/.claude/skills/temper".to_string()
}

impl Default for SkillConfig {
    fn default() -> Self {
        Self {
            output: default_skill_output(),
        }
    }
}

/// A single auth provider entry. Stored in `[[auth.providers]]` arrays in TOML.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct AuthProvider {
    /// Provider name — referenced by `auth.provider` to pick the active entry.
    #[validate(length(min = 1, message = "provider name cannot be empty"))]
    pub name: String,
    #[validate(url(message = "authorize_url must be a valid URL"))]
    pub authorize_url: String,
    #[validate(url(message = "token_url must be a valid URL"))]
    pub token_url: String,
    #[validate(length(min = 1, message = "client_id cannot be empty"))]
    pub client_id: String,
    #[validate(url(message = "audience must be a valid URL"))]
    pub audience: String,
    #[serde(default = "default_callback_url")]
    pub callback_url: String,
    #[serde(default)]
    pub scopes: Vec<String>,
}

fn default_callback_url() -> String {
    "https://temperkb.io/api/auth/cli-callback".to_string()
}

/// Auth configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct AuthConfig {
    #[serde(default = "default_auth_provider")]
    pub provider: String,
    #[serde(default)]
    #[validate(nested)]
    pub providers: Vec<AuthProvider>,
    /// Override for the on-disk auth file path. Tilde-expanded at resolution
    /// time. When `None`, falls back to `~/.config/temper/auth.json`. Has
    /// lower precedence than the `TEMPER_AUTH_PATH` env var.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

fn default_auth_provider() -> String {
    "auth0".to_string()
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            provider: default_auth_provider(),
            providers: vec![AuthProvider {
                name: "auth0".to_string(),
                authorize_url: "https://temperkb.us.auth0.com/authorize".to_string(),
                token_url: "https://temperkb.us.auth0.com/oauth/token".to_string(),
                client_id: "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF".to_string(),
                audience: "https://temperkb.io/api".to_string(),
                callback_url: default_callback_url(),
                scopes: vec![
                    "openid".to_string(),
                    "profile".to_string(),
                    "email".to_string(),
                    "offline_access".to_string(),
                ],
            }],
            path: None,
        }
    }
}

/// LLM provider type — used to route to the correct backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderType {
    #[default]
    Ollama,
    Claude,
    OpenAiCompatible,
}

/// LLM configuration section.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct LlmConfig {
    /// Which LLM backend to use.
    #[serde(default)]
    pub provider: LlmProviderType,
    /// Base URL for the LLM API (e.g. `http://localhost:11434` for ollama).
    /// Defaults to `http://localhost:11434` for ollama-compatible providers.
    #[serde(default)]
    pub url: String,
    /// Model identifier (e.g. `llama3.2:latest`, `claude-sonnet-4-5`).
    /// Defaults vary by provider.
    #[serde(default)]
    pub model: String,
    /// API key — read from `TEMPER_LLM_API_KEY` env var at call site when None.
    /// Only set this field if you want the key in the config file (not recommended).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// HTTP request timeout in seconds for LLM provider calls.
    /// Reasoning / large cloud models may need longer than the default.
    #[serde(default = "default_llm_timeout_secs")]
    pub request_timeout_secs: u64,
}

fn default_llm_timeout_secs() -> u64 {
    300
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            provider: LlmProviderType::default(),
            url: "http://localhost:11434".to_string(),
            model: "llama3.2:latest".to_string(),
            api_key: None,
            request_timeout_secs: default_llm_timeout_secs(),
        }
    }
}

impl LlmConfig {
    /// Load LLM config with env-var precedence over defaults.
    ///
    /// Precedence (highest to lowest):
    /// 1. `TEMPER_LLM_*` env vars override everything
    /// 2. Config file values
    /// 3. Provider-specific defaults (ollama: localhost:11434 / llama3.2:latest)
    ///
    /// This should be called **after** loading the config file, so that
    /// `file_config` contains the parsed file values and env vars can override them.
    pub fn load(file_config: &LlmConfig) -> Self {
        Self {
            provider: std::env::var("TEMPER_LLM_PROVIDER")
                .ok()
                .and_then(|s| serde_json::from_str::<LlmProviderType>(&format!("\"{s}\"")).ok())
                .unwrap_or(file_config.provider),
            url: std::env::var("TEMPER_LLM_URL")
                .ok()
                .unwrap_or_else(|| file_config.url.clone()),
            model: std::env::var("TEMPER_LLM_MODEL")
                .ok()
                .unwrap_or_else(|| file_config.model.clone()),
            api_key: std::env::var("TEMPER_LLM_API_KEY").ok(),
            request_timeout_secs: file_config.request_timeout_secs,
        }
    }
}

/// Cloud API section of the configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct CloudSection {
    /// API base URL (overridden by `TEMPER_API_URL` environment variable).
    #[serde(default = "default_api_url")]
    #[validate(url(message = "api_url must be a valid URL"))]
    pub api_url: String,
}

impl Default for CloudSection {
    fn default() -> Self {
        Self {
            api_url: default_api_url(),
        }
    }
}

fn default_api_url() -> String {
    "https://temperkb.io".to_string()
}

/// Canonical temper config — `~/.config/temper/config.toml`.
///
/// Single config file replacing the old split model (global config + vault temper.toml).
/// Imported by temper-cli, temper-client, temper-mcp, and any future crate.
#[derive(Debug, Clone, Serialize, Deserialize, Validate)]
pub struct TemperConfig {
    #[validate(nested)]
    pub vault: CloudVaultConfig,
    #[serde(default)]
    pub sync: UnifiedSyncConfig,
    #[serde(default)]
    #[validate(nested)]
    pub skill: SkillConfig,
    #[serde(default)]
    #[validate(nested)]
    pub auth: AuthConfig,
    #[serde(default)]
    #[validate(nested)]
    pub cloud: CloudSection,
    #[serde(default)]
    #[validate(nested)]
    pub llm: LlmConfig,
}

impl Default for TemperConfig {
    fn default() -> Self {
        Self {
            vault: CloudVaultConfig {
                path: "~/Documents/temper-vault".to_string(),
            },
            sync: Default::default(),
            skill: Default::default(),
            auth: Default::default(),
            cloud: Default::default(),
            llm: Default::default(),
        }
    }
}

/// Backward-compatible alias for `TemperConfig`.
pub type UnifiedConfig = TemperConfig;

// ---------------------------------------------------------------------------
// Shared config path resolution and loading
// ---------------------------------------------------------------------------

/// Expand a leading `~/` to the user's home directory.
pub fn expand_tilde(path: &str) -> std::path::PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(rest);
        }
    } else if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    std::path::PathBuf::from(path)
}

/// Path to the global config file.
///
/// Resolution order:
/// 1. `TEMPER_GLOBAL_CONFIG` env var
/// 2. `~/.config/temper/config.toml`
pub fn global_config_path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("TEMPER_GLOBAL_CONFIG") {
        if !p.is_empty() {
            return std::path::PathBuf::from(p);
        }
    }
    expand_tilde("~/.config/temper/config.toml")
}

/// Load and parse the global config from the config file.
///
/// Returns `TemperConfig` with defaults when file is absent.
/// Returns an error only if the file exists but cannot be parsed.
pub fn load_config() -> Result<TemperConfig, String> {
    load_config_from(&global_config_path())
}

/// Load config from a specific path (useful for tests).
pub fn load_config_from(path: &std::path::Path) -> Result<TemperConfig, String> {
    if !path.exists() {
        return Ok(TemperConfig::default());
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {}", path.display(), e))?;
    let cfg: TemperConfig = toml::from_str(&content)
        .map_err(|e| format!("config parse error in {}: {}", path.display(), e))?;
    if let Err(e) = cfg.validate() {
        tracing::warn!(
            path = %path.display(),
            error = %e,
            "config at {} has validation issues — run `temper config edit` to fix",
            path.display()
        );
    }
    Ok(cfg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_merge_policy_default() {
        assert_eq!(MergePolicy::default(), MergePolicy::Manual);
    }

    #[test]
    fn test_merge_policy_serde() {
        assert_eq!(
            serde_json::to_string(&MergePolicy::Auto).unwrap(),
            "\"auto\""
        );
        assert_eq!(
            serde_json::to_string(&MergePolicy::Manual).unwrap(),
            "\"manual\""
        );
    }

    #[test]
    fn test_cloud_config_toml_roundtrip() {
        let toml_str = r#"
[vault]
path = "~/projects/knowledge"

[sync]
auto = false

[[sync.subscriptions]]
context = "temper"
merge = "manual"

[[sync.subscriptions]]
team = "platform-team"
doc_types = ["research", "concept"]
merge = "auto"
"#;
        let config: CloudConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.vault.path, "~/projects/knowledge");
        assert!(!config.sync.auto);
        assert_eq!(config.sync.subscriptions.len(), 2);
        assert_eq!(
            config.sync.subscriptions[0].context.as_deref(),
            Some("temper")
        );
        assert_eq!(config.sync.subscriptions[0].merge, MergePolicy::Manual);
        assert_eq!(
            config.sync.subscriptions[1].team.as_deref(),
            Some("platform-team")
        );
        assert_eq!(config.sync.subscriptions[1].merge, MergePolicy::Auto);
        assert_eq!(
            config.sync.subscriptions[1].doc_types,
            vec!["research", "concept"]
        );
    }

    #[test]
    fn test_cloud_config_minimal_toml() {
        let toml_str = r#"
[vault]
path = "~/vault"
"#;
        let config: CloudConfig = toml::from_str(toml_str).unwrap();
        assert!(!config.sync.auto);
        assert!(config.sync.subscriptions.is_empty());
    }

    #[test]
    fn stale_cli_section_and_skill_framework_parse_without_error() {
        // Forward-compat guarantee: stale configs containing the removed
        // `[cli]` section and `skill.framework` field must still parse.
        //
        // This works because none of the config structs use
        // `#[serde(deny_unknown_fields)]` — serde's default behavior is to
        // ignore unknown fields. If a future contributor adds that attribute
        // to TemperConfig (or any nested struct), this test will fail as the
        // signal that the clean break in Task 8 broke forward compat and
        // either the attribute should come back off or a migration is needed.
        let toml_str = r#"
[vault]
path = "~/Documents/temper-vault"

[cli]
progress = "bar"

[skill]
output = "~/.claude/skills/temper"
framework = "superpowers"
"#;
        let cfg: TemperConfig = toml::from_str(toml_str).expect("stale config must parse");
        assert_eq!(cfg.vault.path, "~/Documents/temper-vault");
        assert_eq!(cfg.skill.output, "~/.claude/skills/temper");
    }

    #[test]
    fn test_subscription_context_only() {
        let sub = SyncSubscription {
            context: Some("temper".to_string()),
            team: None,
            doc_types: vec![],
            merge: MergePolicy::Manual,
        };
        let json = serde_json::to_string(&sub).unwrap();
        assert!(!json.contains("team"));
        assert!(!json.contains("doc_types"));
    }

    use validator::Validate;

    // --- new auth provider shape ---

    #[test]
    fn auth_providers_parse_as_array_of_tables() {
        let toml_str = r#"
[vault]
path = "~/projects/kb-vault"

[sync.subscriptions]
contexts = ["temper"]

[skill]
output = "~/.claude/skills/temper"

[auth]
provider = "auth0"

[[auth.providers]]
name = "auth0"
authorize_url = "https://temperkb.us.auth0.com/authorize"
token_url = "https://temperkb.us.auth0.com/oauth/token"
client_id = "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF"
audience = "https://temperkb.io/api"
scopes = ["openid", "profile", "email", "offline_access"]

[cloud]
api_url = "https://temperkb.io"
"#;
        let cfg: TemperConfig = toml::from_str(toml_str).expect("should parse");
        assert_eq!(cfg.vault.path, "~/projects/kb-vault");
        assert_eq!(cfg.auth.provider, "auth0");
        assert_eq!(cfg.auth.providers.len(), 1);
        assert_eq!(cfg.auth.providers[0].name, "auth0");
        assert_eq!(
            cfg.auth.providers[0].authorize_url,
            "https://temperkb.us.auth0.com/authorize"
        );
    }

    #[test]
    fn auth_providers_lookup_by_name() {
        let cfg = TemperConfig::default();
        let active = cfg
            .auth
            .providers
            .iter()
            .find(|p| p.name == cfg.auth.provider);
        assert!(
            active.is_some(),
            "default config should have its active provider"
        );
        assert_eq!(active.unwrap().name, "auth0");
    }

    #[test]
    fn default_vault_path_is_documents_temper_vault() {
        let cfg = TemperConfig::default();
        assert_eq!(cfg.vault.path, "~/Documents/temper-vault");
    }

    #[test]
    fn test_temper_config_minimal() {
        let toml_str = r#"
[vault]
path = "~/vault"
"#;
        let config: TemperConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.vault.path, "~/vault");
        assert!(config.sync.subscriptions.contexts.is_empty());
        assert_eq!(config.auth.provider, "auth0");
        // Cloud section defaults
        assert_eq!(config.cloud.api_url, "https://temperkb.io");
        // When `[auth]` is omitted entirely, `AuthConfig::default()` is used,
        // which seeds the built-in auth0 provider.
        assert_eq!(config.auth.providers.len(), 1);
        assert_eq!(config.auth.providers[0].name, "auth0");
        assert_eq!(
            config.auth.providers[0].callback_url,
            "https://temperkb.io/api/auth/cli-callback"
        );
    }

    // --- validator rules ---

    #[test]
    fn validate_accepts_default_config() {
        let cfg = TemperConfig::default();
        cfg.validate().expect("default config should validate");
    }

    #[test]
    fn validate_rejects_empty_vault_path() {
        let mut cfg = TemperConfig::default();
        cfg.vault.path = String::new();
        let err = cfg.validate().unwrap_err();
        let s = format!("{err}");
        assert!(s.contains("vault") || s.contains("path"), "got: {s}");
    }

    #[test]
    fn validate_rejects_malformed_api_url() {
        let mut cfg = TemperConfig::default();
        cfg.cloud.api_url = "not a url".to_string();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn validate_rejects_malformed_authorize_url_in_provider_vec() {
        let mut cfg = TemperConfig::default();
        cfg.auth.providers[0].authorize_url = "nope".to_string();
        let err = cfg.validate().unwrap_err();
        let s = format!("{err}");
        assert!(
            s.contains("authorize_url") || s.contains("provider"),
            "got: {s}"
        );
    }

    #[test]
    fn validate_rejects_empty_provider_client_id() {
        let mut cfg = TemperConfig::default();
        cfg.auth.providers[0].client_id = String::new();
        assert!(cfg.validate().is_err());
    }
}
