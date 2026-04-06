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
    #[serde(alias = "content_hash")]
    pub body_hash: String,
    /// SHA-256 hash of the remote body at last sync
    #[serde(alias = "remote_hash")]
    pub remote_body_hash: String,
    /// SHA-256 hash of the local managed frontmatter (temper-* fields) at last manifest update
    #[serde(default)]
    pub managed_hash: String,
    /// SHA-256 hash of the local open frontmatter (user fields) at last manifest update
    #[serde(default)]
    pub open_hash: String,
    /// SHA-256 hash of the remote managed frontmatter at last sync
    #[serde(default)]
    pub remote_managed_hash: String,
    /// SHA-256 hash of the remote open frontmatter at last sync
    #[serde(default)]
    pub remote_open_hash: String,
    /// When this entry was last synced with the server
    pub synced_at: DateTime<Utc>,
    /// Current sync state
    pub state: ManifestEntryState,
    /// File mtime (seconds since epoch) at last manifest update.
    /// Used to skip rehashing unchanged files.
    #[serde(default)]
    pub mtime_secs: Option<i64>,
    /// Whether this entry has a locally-generated provisional ID that hasn't
    /// been confirmed by the server yet.  Provisional entries always POST
    /// (never PUT) and get rekeyed to the server ID after success.
    #[serde(default)]
    pub provisional: bool,
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
                body_hash: "sha256:abc123".to_string(),
                remote_body_hash: "sha256:abc123".to_string(),
                managed_hash: String::new(),
                open_hash: String::new(),
                remote_managed_hash: String::new(),
                remote_open_hash: String::new(),
                synced_at: Utc::now(),
                state: ManifestEntryState::Clean,
                mtime_secs: None,
                provisional: false,
            },
        );
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let parsed: Manifest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.device_id, "device-abc");
        assert_eq!(parsed.entries.len(), 1);
        let entry = parsed.entries.get(&resource_id).unwrap();
        assert_eq!(entry.path, "temper/tickets/r5.md");
        assert_eq!(entry.body_hash, "sha256:abc123");
        assert_eq!(entry.remote_body_hash, "sha256:abc123");
        assert_eq!(entry.state, ManifestEntryState::Clean);
    }

    #[test]
    fn test_manifest_entry_migration_from_old_format() {
        let old_json = serde_json::json!({
            "path": "temper/goals/my-goal.md",
            "content_hash": "sha256:oldcontent",
            "remote_hash": "sha256:oldremote",
            "synced_at": "2026-01-01T00:00:00Z",
            "state": "clean"
        });
        let entry: ManifestEntry = serde_json::from_value(old_json).unwrap();
        assert_eq!(entry.body_hash, "sha256:oldcontent");
        assert_eq!(entry.remote_body_hash, "sha256:oldremote");
        assert_eq!(entry.managed_hash, "");
        assert_eq!(entry.open_hash, "");
        assert_eq!(entry.remote_managed_hash, "");
        assert_eq!(entry.remote_open_hash, "");
        assert_eq!(entry.state, ManifestEntryState::Clean);
    }

    #[test]
    fn test_manifest_entry_new_format_roundtrip() {
        let entry = ManifestEntry {
            path: "temper/sessions/s1.md".to_string(),
            body_hash: "sha256:body".to_string(),
            remote_body_hash: "sha256:rbody".to_string(),
            managed_hash: "sha256:managed".to_string(),
            open_hash: "sha256:open".to_string(),
            remote_managed_hash: "sha256:rmanaged".to_string(),
            remote_open_hash: "sha256:ropen".to_string(),
            synced_at: Utc::now(),
            state: ManifestEntryState::LocalModified,
            mtime_secs: Some(1_700_000_000),
            provisional: false,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: ManifestEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.body_hash, "sha256:body");
        assert_eq!(parsed.remote_body_hash, "sha256:rbody");
        assert_eq!(parsed.managed_hash, "sha256:managed");
        assert_eq!(parsed.open_hash, "sha256:open");
        assert_eq!(parsed.remote_managed_hash, "sha256:rmanaged");
        assert_eq!(parsed.remote_open_hash, "sha256:ropen");
        assert_eq!(parsed.state, ManifestEntryState::LocalModified);
        assert_eq!(parsed.mtime_secs, Some(1_700_000_000));
        assert!(!parsed.provisional);
    }

    #[test]
    fn test_manifest_entry_provisional_defaults_false() {
        // Old JSON without the `provisional` field should deserialize to false
        let old_json = serde_json::json!({
            "path": "temper/goals/my-goal.md",
            "body_hash": "sha256:body",
            "remote_body_hash": "sha256:remote",
            "synced_at": "2026-01-01T00:00:00Z",
            "state": "clean"
        });
        let entry: ManifestEntry = serde_json::from_value(old_json).unwrap();
        assert!(
            !entry.provisional,
            "provisional should default to false for old manifests"
        );
    }

    #[test]
    fn test_manifest_entry_provisional_roundtrip() {
        // provisional: true should survive serialize/deserialize
        let entry = ManifestEntry {
            path: "temper/sessions/new.md".to_string(),
            body_hash: "sha256:body".to_string(),
            remote_body_hash: "sha256:rbody".to_string(),
            managed_hash: String::new(),
            open_hash: String::new(),
            remote_managed_hash: String::new(),
            remote_open_hash: String::new(),
            synced_at: Utc::now(),
            state: ManifestEntryState::Clean,
            mtime_secs: None,
            provisional: true,
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: ManifestEntry = serde_json::from_str(&json).unwrap();
        assert!(
            parsed.provisional,
            "provisional: true should survive roundtrip"
        );
    }
}
