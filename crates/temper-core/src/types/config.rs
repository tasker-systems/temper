use std::collections::HashMap;

use serde::{Deserialize, Serialize};

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

/// CLI output preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CliConfig {
    /// Progress output format: "bar" (human-friendly) or "json" (JSONL stream)
    #[serde(default = "default_progress")]
    pub progress: String,
}

impl Default for CliConfig {
    fn default() -> Self {
        Self {
            progress: default_progress(),
        }
    }
}

fn default_progress() -> String {
    "bar".to_string()
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
    #[serde(default)]
    pub cli: CliConfig,
}

/// Vault path reference in cloud config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudVaultConfig {
    /// Path to the local vault directory
    pub path: String,
}

/// Auto-sync configuration — which doc types trigger auto-sync on create/update.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncAutoConfig {
    #[serde(default)]
    pub doctypes: Vec<String>,
}

/// Sync subscriptions — which contexts are synced.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncSubscriptionsConfig {
    #[serde(default)]
    pub contexts: Vec<String>,
}

/// New sync config with auto + subscriptions sub-sections.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UnifiedSyncConfig {
    #[serde(default)]
    pub auto: SyncAutoConfig,
    #[serde(default)]
    pub subscriptions: SyncSubscriptionsConfig,
}

/// Skill generation config.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillConfig {
    #[serde(default = "default_skill_output")]
    pub output: String,
    #[serde(default = "default_skill_framework")]
    pub framework: String,
}

fn default_skill_output() -> String {
    "~/.claude/commands/temper.md".to_string()
}

fn default_skill_framework() -> String {
    "superpowers".to_string()
}

impl Default for SkillConfig {
    fn default() -> Self {
        Self {
            output: default_skill_output(),
            framework: default_skill_framework(),
        }
    }
}

/// Auth provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthProviderConfig {
    pub authorize_url: String,
    pub token_url: String,
    pub client_id: String,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    #[serde(default = "default_auth_provider")]
    pub provider: String,
    #[serde(default)]
    pub providers: HashMap<String, AuthProviderConfig>,
}

fn default_auth_provider() -> String {
    "auth0".to_string()
}

impl Default for AuthConfig {
    fn default() -> Self {
        let mut providers = HashMap::new();
        providers.insert(
            "auth0".to_string(),
            AuthProviderConfig {
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
            },
        );
        Self {
            provider: default_auth_provider(),
            providers,
        }
    }
}

/// Cloud API section of the configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudSection {
    /// API base URL (overridden by `TEMPER_API_URL` environment variable).
    #[serde(default = "default_api_url")]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemperConfig {
    pub vault: CloudVaultConfig,
    #[serde(default)]
    pub sync: UnifiedSyncConfig,
    #[serde(default)]
    pub cli: CliConfig,
    #[serde(default)]
    pub skill: SkillConfig,
    #[serde(default)]
    pub auth: AuthConfig,
    #[serde(default)]
    pub cloud: CloudSection,
}

/// Backward-compatible alias for `TemperConfig`.
pub type UnifiedConfig = TemperConfig;

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

[cli]
progress = "bar"
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
        assert_eq!(config.cli.progress, "bar");
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
        assert_eq!(config.cli.progress, "bar");
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

    #[test]
    fn test_temper_config_toml_roundtrip() {
        let toml_str = r#"
[vault]
path = "~/projects/kb-vault"

[sync.auto]
doctypes = ["task", "goal", "session"]

[sync.subscriptions]
contexts = ["temper", "storyteller", "tasker", "writing"]

[cli]
progress = "bar"

[skill]
output = "~/.claude/commands/temper.md"
framework = "superpowers"

[auth]
provider = "auth0"

[auth.providers.auth0]
authorize_url = "https://temperkb.us.auth0.com/authorize"
token_url = "https://temperkb.us.auth0.com/oauth/token"
client_id = "mWp8znLw2MUJNCiZNl8wwBv6SPJI2mfF"
audience = "https://temperkb.io/api"
scopes = ["openid", "profile", "email", "offline_access"]

[cloud]
api_url = "https://api.example.com"
"#;
        let config: TemperConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.vault.path, "~/projects/kb-vault");
        assert_eq!(config.sync.auto.doctypes, vec!["task", "goal", "session"]);
        assert_eq!(
            config.sync.subscriptions.contexts,
            vec!["temper", "storyteller", "tasker", "writing"]
        );
        assert_eq!(config.cli.progress, "bar");
        assert_eq!(config.skill.output, "~/.claude/commands/temper.md");
        assert_eq!(config.skill.framework, "superpowers");
        assert_eq!(config.auth.provider, "auth0");
        assert!(config.auth.providers.contains_key("auth0"));
        assert_eq!(config.cloud.api_url, "https://api.example.com");
        let auth0 = config.auth.providers.get("auth0").unwrap();
        assert_eq!(
            auth0.callback_url,
            "https://temperkb.io/api/auth/cli-callback"
        );
    }

    #[test]
    fn test_temper_config_minimal() {
        let toml_str = r#"
[vault]
path = "~/vault"
"#;
        let config: TemperConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(config.vault.path, "~/vault");
        assert!(config.sync.auto.doctypes.is_empty());
        assert!(config.sync.subscriptions.contexts.is_empty());
        assert_eq!(config.cli.progress, "bar");
        assert_eq!(config.auth.provider, "auth0");
        // Cloud section defaults
        assert_eq!(config.cloud.api_url, "https://temperkb.io");
        // Auth provider defaults include callback_url
        let auth0 = config.auth.providers.get("auth0").unwrap();
        assert_eq!(
            auth0.callback_url,
            "https://temperkb.io/api/auth/cli-callback"
        );
    }
}
