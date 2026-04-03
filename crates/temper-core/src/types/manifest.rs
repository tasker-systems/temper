use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Per-resource sync state in the local manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManifestEntryState {
    /// Local hash = manifest hash = remote hash
    Clean,
    /// Local hash != manifest hash (local edits since last sync)
    LocalModified,
    /// Remote hash changed (detected on next sync/status check)
    RemoteModified,
    /// Both sides changed; `.conflict.md` materialized alongside
    Conflict,
    /// Subscribed but not yet materialized (new resource from server)
    Pending,
}

/// A single resource entry in the local manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Relative path within the vault (e.g., "temper/tickets/r5-indexing.md")
    pub path: String,
    /// SHA-256 hash of the local file body (frontmatter stripped) at last manifest update
    pub content_hash: String,
    /// SHA-256 hash of the remote content at last sync
    pub remote_hash: String,
    /// When this entry was last synced with the server
    pub synced_at: DateTime<Utc>,
    /// Current sync state
    pub state: ManifestEntryState,
    /// File mtime (seconds since epoch) at last manifest update.
    /// Used to skip rehashing unchanged files.
    #[serde(default)]
    pub mtime_secs: Option<i64>,
}

/// The local manifest — `<vault>/.temper/manifest.json`.
///
/// Maps resource UUIDs to their local file state. Used by `temper sync`
/// for three-way hash comparison (local file, manifest record, server).
/// Updated after every sync round and on local-only pre-flight checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Device identifier (UUIDv7, stored in auth.json)
    pub device_id: String,
    /// Timestamp of last completed sync round
    pub last_sync: Option<DateTime<Utc>>,
    /// Resource UUID → manifest entry
    pub entries: HashMap<Uuid, ManifestEntry>,
}

impl Manifest {
    /// Create a new empty manifest for a device.
    pub fn new(device_id: String) -> Self {
        Self {
            device_id,
            last_sync: None,
            entries: HashMap::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_entry_state_serde() {
        let states = [
            (ManifestEntryState::Clean, "\"clean\""),
            (ManifestEntryState::LocalModified, "\"local_modified\""),
            (ManifestEntryState::RemoteModified, "\"remote_modified\""),
            (ManifestEntryState::Conflict, "\"conflict\""),
            (ManifestEntryState::Pending, "\"pending\""),
        ];
        for (state, expected_json) in &states {
            let json = serde_json::to_string(state).unwrap();
            assert_eq!(&json, expected_json);
            let parsed: ManifestEntryState = serde_json::from_str(&json).unwrap();
            assert_eq!(*state, parsed);
        }
    }

    #[test]
    fn test_manifest_new() {
        let manifest = Manifest::new("device-123".to_string());
        assert_eq!(manifest.device_id, "device-123");
        assert!(manifest.last_sync.is_none());
        assert!(manifest.entries.is_empty());
    }

    #[test]
    fn test_manifest_json_roundtrip() {
        let mut manifest = Manifest::new("device-abc".to_string());
        let resource_id = Uuid::nil();
        manifest.entries.insert(
            resource_id,
            ManifestEntry {
                path: "temper/tickets/r5.md".to_string(),
                content_hash: "sha256:abc123".to_string(),
                remote_hash: "sha256:abc123".to_string(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: None,
            },
        );
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let parsed: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.device_id, "device-abc");
        assert_eq!(parsed.entries.len(), 1);
        let entry = parsed.entries.get(&resource_id).unwrap();
        assert_eq!(entry.path, "temper/tickets/r5.md");
        assert_eq!(entry.state, ManifestEntryState::Clean);
    }
}
