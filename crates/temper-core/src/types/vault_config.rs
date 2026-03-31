use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::config::MergePolicy;

/// Server-side vault configuration stored in `kb_profiles.vault_config`.
///
/// Describes sync subscriptions, per-device overrides, and the vault path.
/// Stored as JSONB — existing empty `{}` values deserialize to defaults.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct VaultConfig {
    /// Managed vault root path
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_path: Option<String>,
    /// What this profile syncs
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subscriptions: Vec<Subscription>,
    /// Per-device overrides keyed by X-Temper-Device-Id
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub per_device: HashMap<String, DeviceOverrides>,
}

/// A sync subscription — scopes which resources materialize locally.
///
/// Each subscription is self-contained with its own sync and merge settings.
/// `local_paths` and `repos` enable CWD-to-context inference.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct Subscription {
    /// Which kb_context this subscription targets
    pub context: String,
    /// Team-owned context (None = profile-owned)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    /// Doc type filter (None = all types)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub doc_types: Option<Vec<String>>,
    /// Run local-only manifest pre-flight on every temper command
    #[serde(default)]
    pub auto_sync: bool,
    /// Conflict resolution policy for this subscription
    #[serde(default)]
    pub merge_policy: MergePolicy,
    /// Local directories mapped to this context
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub local_paths: Vec<String>,
    /// Git repos associated with this context (owner/repo or local paths)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repos: Vec<String>,
}

/// Per-device configuration overrides keyed by X-Temper-Device-Id.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct DeviceOverrides {
    /// Device-specific vault location
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_path: Option<String>,
    /// Subscription-level overrides keyed by context name
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub subscription_overrides: HashMap<String, SubscriptionOverride>,
}

/// Overrides for a specific subscription on a specific device.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct SubscriptionOverride {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_sync: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_policy: Option<MergePolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_paths: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_json_deserializes_to_default() {
        let config: VaultConfig = serde_json::from_str("{}").unwrap();
        assert!(config.vault_path.is_none());
        assert!(config.subscriptions.is_empty());
        assert!(config.per_device.is_empty());
    }

    #[test]
    fn full_config_round_trips() {
        let config = VaultConfig {
            vault_path: Some("~/projects/knowledge".to_string()),
            subscriptions: vec![
                Subscription {
                    context: "temper".to_string(),
                    team: None,
                    doc_types: None,
                    auto_sync: true,
                    merge_policy: MergePolicy::Manual,
                    local_paths: vec!["~/projects/tasker-systems/temper".to_string()],
                    repos: vec!["tasker-systems/temper".to_string()],
                },
                Subscription {
                    context: "storyteller".to_string(),
                    team: Some("narrative-team".to_string()),
                    doc_types: Some(vec!["research".to_string(), "concept".to_string()]),
                    auto_sync: false,
                    merge_policy: MergePolicy::Auto,
                    local_paths: vec![],
                    repos: vec![],
                },
            ],
            per_device: HashMap::from([(
                "macbook-abc123".to_string(),
                DeviceOverrides {
                    vault_path: Some("/alt/vault".to_string()),
                    subscription_overrides: HashMap::from([(
                        "temper".to_string(),
                        SubscriptionOverride {
                            auto_sync: Some(false),
                            merge_policy: None,
                            local_paths: None,
                        },
                    )]),
                },
            )]),
        };

        let json = serde_json::to_string(&config).unwrap();
        let roundtripped: VaultConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(roundtripped.vault_path, config.vault_path);
        assert_eq!(roundtripped.subscriptions.len(), 2);
        assert_eq!(roundtripped.subscriptions[0].context, "temper");
        assert!(roundtripped.subscriptions[0].auto_sync);
        assert_eq!(
            roundtripped.subscriptions[1].team.as_deref(),
            Some("narrative-team")
        );
        assert_eq!(roundtripped.per_device.len(), 1);
        assert!(roundtripped.per_device.contains_key("macbook-abc123"));
    }

    #[test]
    fn default_serializes_to_empty_object() {
        let config = VaultConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        assert_eq!(json, "{}");
    }

    #[test]
    fn subscription_skips_none_fields() {
        let sub = Subscription {
            context: "temper".to_string(),
            team: None,
            doc_types: None,
            auto_sync: false,
            merge_policy: MergePolicy::Manual,
            local_paths: vec![],
            repos: vec![],
        };
        let json = serde_json::to_string(&sub).unwrap();
        assert!(!json.contains("team"));
        assert!(!json.contains("doc_types"));
        assert!(!json.contains("local_paths"));
        assert!(!json.contains("repos"));
    }
}
