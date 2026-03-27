use serde::{Deserialize, Serialize};

/// Merge policy for conflict resolution within a subscription scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
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
}
