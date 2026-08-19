# R5: Indexing, Sync & Resource Management — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the R5 type system and schema additions that express the API contract for sync, vault management, conflict resolution, and resource transfer.

**Architecture:** Extends `src/cloud/types/` with new modules for each R5 domain (sync protocol, manifest, config, vault, upload, search, transfer, conflict, device). Adds schema migration for transfer and device tracking tables. All types follow the R4 pattern: `Debug, Clone, serde::{Serialize, Deserialize}` for API types; `sqlx::FromRow` for database-backed types; `sqlx::Type` for Postgres enums.

**Tech Stack:** Rust, serde (JSON/TOML serialization), sqlx (Postgres type mapping), chrono (timestamps), uuid (UUIDv7)

---

### Task 1: Schema Migration — Transfers and Device Sync Tracking

**Files:**
- Modify: `migrations/20260326000001_r2_schema.sql`

This task adds two new tables and one new enum to the unified migration: `transfer_status` enum, `kb_transfers` table for resource ownership transfer lifecycle, and `kb_device_sync_state` table for per-device sync tracking.

- [ ] **Step 1: Add transfer_status enum and kb_transfers table**

Append after the `kb_team_invitations` table (line 218) and before the access control functions (line 220):

```sql
CREATE TYPE transfer_status AS ENUM ('pending', 'accepted', 'declined', 'cancelled');

CREATE TABLE kb_transfers (
    id                      UUID PRIMARY KEY,              -- UUIDv7
    resource_id             UUID NOT NULL REFERENCES resources(id),
    from_profile_id         UUID NOT NULL REFERENCES kb_profiles(id),
    to_profile_id           UUID NOT NULL REFERENCES kb_profiles(id),
    status                  transfer_status NOT NULL DEFAULT 'pending',
    created                 TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at             TIMESTAMPTZ,
    UNIQUE(resource_id, from_profile_id, to_profile_id, status)
);

CREATE INDEX idx_transfers_to_profile ON kb_transfers(to_profile_id) WHERE status = 'pending';
CREATE INDEX idx_transfers_from_profile ON kb_transfers(from_profile_id) WHERE status = 'pending';
CREATE INDEX idx_transfers_resource ON kb_transfers(resource_id);
```

- [ ] **Step 2: Add kb_device_sync_state table**

Append immediately after `kb_transfers`:

```sql
CREATE TABLE kb_device_sync_state (
    id                      UUID PRIMARY KEY,              -- UUIDv7
    profile_id              UUID NOT NULL REFERENCES kb_profiles(id),
    client_id               VARCHAR(64) NOT NULL,
    last_sync_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    manifest_hash           VARCHAR(64),
    UNIQUE(profile_id, client_id)
);

CREATE INDEX idx_device_sync_profile ON kb_device_sync_state(profile_id);
```

- [ ] **Step 3: Verify the migration parses correctly**

Run: `cargo build --all-features 2>&1 | tail -5`
Expected: compilation succeeds (sqlx offline mode — migration is validated at deploy time, not compile time)

- [ ] **Step 4: Commit**

```bash
git add migrations/20260326000001_r2_schema.sql
git commit -m "feat: add transfer and device sync tracking tables to R2 schema"
```

---

### Task 2: Transfer Types

**Files:**
- Create: `src/cloud/types/transfer.rs`
- Modify: `src/cloud/types/mod.rs`

- [ ] **Step 1: Write the transfer type tests**

Create `src/cloud/types/transfer.rs` with types and inline tests:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Transfer status — lifecycle of a resource ownership transfer.
///
/// Maps directly to the `transfer_status` Postgres enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, Serialize, Deserialize)]
#[sqlx(type_name = "transfer_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TransferStatus {
    Pending,
    Accepted,
    Declined,
    Cancelled,
}

/// A pending or resolved ownership transfer of a resource.
///
/// Two-step offer/accept for personal transfers. The offerer creates
/// the transfer, the recipient accepts or declines. The offerer can
/// cancel before resolution.
///
/// Constraints:
/// - Only the current `owner_profile_id` can initiate a transfer
/// - One pending transfer per resource at a time (enforced by unique constraint)
/// - Acceptance updates `resources.owner_profile_id` to `to_profile_id`
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct ResourceTransfer {
    pub id: Uuid,
    pub resource_id: Uuid,
    pub from_profile_id: Uuid,
    pub to_profile_id: Uuid,
    pub status: TransferStatus,
    pub created: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

/// API request to initiate a resource transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferRequest {
    pub resource_id: Uuid,
    pub to_profile_id: Uuid,
}

/// API request for bulk team reassignment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkReassignRequest {
    pub from_profile_id: Uuid,
    pub to_profile_id: Uuid,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transfer_status_serde_roundtrip() {
        let statuses = [
            TransferStatus::Pending,
            TransferStatus::Accepted,
            TransferStatus::Declined,
            TransferStatus::Cancelled,
        ];
        for status in &statuses {
            let json = serde_json::to_string(status).unwrap();
            let parsed: TransferStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(*status, parsed);
        }
    }

    #[test]
    fn test_transfer_status_json_format() {
        assert_eq!(
            serde_json::to_string(&TransferStatus::Pending).unwrap(),
            "\"pending\""
        );
        assert_eq!(
            serde_json::to_string(&TransferStatus::Cancelled).unwrap(),
            "\"cancelled\""
        );
    }

    #[test]
    fn test_transfer_request_serde() {
        let req = TransferRequest {
            resource_id: Uuid::nil(),
            to_profile_id: Uuid::nil(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: TransferRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.resource_id, req.resource_id);
        assert_eq!(parsed.to_profile_id, req.to_profile_id);
    }
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo test --lib cloud::types::transfer -- --nocapture 2>&1 | tail -10`
Expected: 3 tests pass

- [ ] **Step 3: Add transfer module to mod.rs**

In `src/cloud/types/mod.rs`, add after the `pub mod team;` line:

```rust
pub mod transfer;
```

And add to the re-exports:

```rust
pub use transfer::{BulkReassignRequest, ResourceTransfer, TransferRequest, TransferStatus};
```

- [ ] **Step 4: Verify compilation**

Run: `cargo build --all-features 2>&1 | tail -5`
Expected: compilation succeeds

- [ ] **Step 5: Commit**

```bash
git add src/cloud/types/transfer.rs src/cloud/types/mod.rs
git commit -m "feat: add resource transfer types — TransferStatus, ResourceTransfer, API request types"
```

---

### Task 3: Device Types

**Files:**
- Create: `src/cloud/types/device.rs`
- Modify: `src/cloud/types/mod.rs`

- [ ] **Step 1: Create device types with tests**

Create `src/cloud/types/device.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Per-device sync state tracked by the server.
///
/// Each device is identified by a `client_id` generated at `temper init` time.
/// The server records the last sync timestamp and manifest hash per device
/// to scope sync/status responses.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize)]
pub struct DeviceSyncState {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub client_id: String,
    pub last_sync_at: DateTime<Utc>,
    pub manifest_hash: Option<String>,
}

/// Local device identity, stored at `~/.config/temper/devices/<id>.json`.
///
/// Generated once at `temper init` time. Sent as `X-Temper-Client-Id` header
/// on all API calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub client_id: String,
    pub device_name: Option<String>,
    pub created: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_device_identity_serde_roundtrip() {
        let device = DeviceIdentity {
            client_id: "d7e8f9a0-1234-5678-9abc-def012345678".to_string(),
            device_name: Some("macbook-pro".to_string()),
            created: Utc::now(),
        };
        let json = serde_json::to_string(&device).unwrap();
        let parsed: DeviceIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.client_id, device.client_id);
        assert_eq!(parsed.device_name, device.device_name);
    }

    #[test]
    fn test_device_identity_optional_name() {
        let json = r#"{"client_id":"abc","device_name":null,"created":"2026-03-27T18:00:00Z"}"#;
        let device: DeviceIdentity = serde_json::from_str(json).unwrap();
        assert!(device.device_name.is_none());
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test --lib cloud::types::device -- --nocapture 2>&1 | tail -10`
Expected: 2 tests pass

- [ ] **Step 3: Add device module to mod.rs**

In `src/cloud/types/mod.rs`, add:

```rust
pub mod device;
```

And add to re-exports:

```rust
pub use device::{DeviceIdentity, DeviceSyncState};
```

- [ ] **Step 4: Verify compilation**

Run: `cargo build --all-features 2>&1 | tail -5`
Expected: compilation succeeds

- [ ] **Step 5: Commit**

```bash
git add src/cloud/types/device.rs src/cloud/types/mod.rs
git commit -m "feat: add device types — DeviceSyncState, DeviceIdentity"
```

---

### Task 4: Config Types — Subscriptions and Sync Preferences

**Files:**
- Create: `src/cloud/types/config.rs`
- Modify: `src/cloud/types/mod.rs`

These types represent the cloud-aware sections of `~/.config/temper/config.toml`. They are separate from the existing `src/config.rs` (which handles the vault-local `temper.toml`). The cloud config will eventually live in its own file; for now, the types are defined in the cloud module.

- [ ] **Step 1: Create config types with tests**

Create `src/cloud/types/config.rs`:

```rust
use serde::{Deserialize, Serialize};

/// Merge policy for conflict resolution within a subscription scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergePolicy {
    /// Require explicit resolution via `temper sync resolve`
    Manual,
    /// Auto-merge: keep both contributions with section attribution
    Auto,
}

impl Default for MergePolicy {
    fn default() -> Self {
        Self::Manual
    }
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    /// Whether to run local-only manifest pre-flight on every temper command
    #[serde(default)]
    pub auto: bool,
    /// Resource subscriptions — what to materialize locally
    #[serde(default)]
    pub subscriptions: Vec<SyncSubscription>,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            auto: false,
            subscriptions: Vec::new(),
        }
    }
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
```

- [ ] **Step 2: Run tests**

Run: `cargo test --lib cloud::types::config -- --nocapture 2>&1 | tail -15`
Expected: 5 tests pass

- [ ] **Step 3: Add config module to mod.rs**

In `src/cloud/types/mod.rs`, add:

```rust
pub mod config;
```

And add to re-exports:

```rust
pub use config::{CliConfig, CloudConfig, CloudVaultConfig, MergePolicy, SyncConfig, SyncSubscription};
```

- [ ] **Step 4: Verify compilation**

Run: `cargo build --all-features 2>&1 | tail -5`
Expected: compilation succeeds

- [ ] **Step 5: Commit**

```bash
git add src/cloud/types/config.rs src/cloud/types/mod.rs
git commit -m "feat: add cloud config types — subscriptions, merge policy, sync preferences"
```

---

### Task 5: Manifest Types

**Files:**
- Create: `src/cloud/types/manifest.rs`
- Modify: `src/cloud/types/mod.rs`

- [ ] **Step 1: Create manifest types with tests**

Create `src/cloud/types/manifest.rs`:

```rust
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
    /// SHA-256 hash of the local file content at last manifest update
    pub content_hash: String,
    /// SHA-256 hash of the remote content at last sync
    pub remote_hash: String,
    /// When this entry was last synced with the server
    pub synced_at: DateTime<Utc>,
    /// Current sync state
    pub state: ManifestEntryState,
}

/// The local manifest — `<vault>/.temper/manifest.json`.
///
/// Maps resource UUIDs to their local file state. Used by `temper sync`
/// for three-way hash comparison (local file, manifest record, server).
/// Updated after every sync round and on local-only pre-flight checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Device identifier (matches DeviceIdentity.client_id)
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
```

- [ ] **Step 2: Run tests**

Run: `cargo test --lib cloud::types::manifest -- --nocapture 2>&1 | tail -10`
Expected: 3 tests pass

- [ ] **Step 3: Add manifest module to mod.rs**

In `src/cloud/types/mod.rs`, add:

```rust
pub mod manifest;
```

And add to re-exports:

```rust
pub use manifest::{Manifest, ManifestEntry, ManifestEntryState};
```

- [ ] **Step 4: Verify compilation**

Run: `cargo build --all-features 2>&1 | tail -5`
Expected: compilation succeeds

- [ ] **Step 5: Commit**

```bash
git add src/cloud/types/manifest.rs src/cloud/types/mod.rs
git commit -m "feat: add manifest types — Manifest, ManifestEntry, ManifestEntryState"
```

---

### Task 6: Sync API Types

**Files:**
- Create: `src/cloud/types/sync.rs`
- Modify: `src/cloud/types/mod.rs`

These are the request/response types for the sync protocol endpoints.

- [ ] **Step 1: Create sync types with tests**

Create `src/cloud/types/sync.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::config::SyncSubscription;

/// A manifest entry sent to the server for comparison.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncManifestEntry {
    pub resource_id: Uuid,
    pub content_hash: String,
    pub updated_at: DateTime<Utc>,
}

/// Request body for `POST /api/sync/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatusRequest {
    pub subscriptions: Vec<SyncSubscription>,
    pub manifest_entries: Vec<SyncManifestEntry>,
}

/// A resource the client should pull (server has newer version or new resource).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPullEntry {
    pub resource_id: Uuid,
    pub content_hash: String,
    pub title: String,
}

/// A resource the client should push (local changes the server doesn't have).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPushEntry {
    pub resource_id: Uuid,
    pub reason: String,
}

/// A resource with conflicting changes on both sides.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflictEntry {
    pub resource_id: Uuid,
    pub local_hash: String,
    pub remote_hash: String,
}

/// A resource removed from visibility (deleted, unshared, or no longer matching).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRemovedEntry {
    pub resource_id: Uuid,
    pub reason: String,
}

/// Response body for `POST /api/sync/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatusResponse {
    pub to_pull: Vec<SyncPullEntry>,
    pub to_push: Vec<SyncPushEntry>,
    pub conflicts: Vec<SyncConflictEntry>,
    pub removed: Vec<SyncRemovedEntry>,
}

/// Request body for `POST /api/sync/pull`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPullRequest {
    pub resource_ids: Vec<Uuid>,
}

/// Metadata sidecar for a pulled resource (included in the zip alongside markdown).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPullResourceMeta {
    pub resource_id: Uuid,
    pub title: String,
    pub context: String,
    pub doc_type: String,
    pub content_hash: String,
    pub tags: Vec<String>,
}

/// Request body for `POST /api/sync/complete`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncCompleteRequest {
    pub resource_ids: Vec<Uuid>,
    pub manifest_hash: String,
}

/// Response body for `POST /api/sync/complete`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncCompleteResponse {
    pub ok: bool,
    pub event_ids: Vec<Uuid>,
}

/// Conflict resolution type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionType {
    Local,
    Remote,
    Merged,
}

/// Request body for `POST /api/sync/resolve`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResolveRequest {
    pub resource_id: Uuid,
    pub resolution: ResolutionType,
    pub content_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_status_response_empty() {
        let resp = SyncStatusResponse {
            to_pull: vec![],
            to_push: vec![],
            conflicts: vec![],
            removed: vec![],
        };
        let json = serde_json::to_string(&resp).unwrap();
        let parsed: SyncStatusResponse = serde_json::from_str(&json).unwrap();
        assert!(parsed.to_pull.is_empty());
        assert!(parsed.conflicts.is_empty());
    }

    #[test]
    fn test_sync_status_request_serde() {
        let req = SyncStatusRequest {
            subscriptions: vec![SyncSubscription {
                context: Some("temper".to_string()),
                team: None,
                doc_types: vec![],
                merge: super::super::config::MergePolicy::Manual,
            }],
            manifest_entries: vec![SyncManifestEntry {
                resource_id: Uuid::nil(),
                content_hash: "sha256:abc".to_string(),
                updated_at: Utc::now(),
            }],
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: SyncStatusRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.subscriptions.len(), 1);
        assert_eq!(parsed.manifest_entries.len(), 1);
    }

    #[test]
    fn test_resolution_type_serde() {
        assert_eq!(
            serde_json::to_string(&ResolutionType::Local).unwrap(),
            "\"local\""
        );
        assert_eq!(
            serde_json::to_string(&ResolutionType::Merged).unwrap(),
            "\"merged\""
        );
    }

    #[test]
    fn test_sync_complete_response() {
        let resp = SyncCompleteResponse {
            ok: true,
            event_ids: vec![Uuid::nil()],
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("true"));
        let parsed: SyncCompleteResponse = serde_json::from_str(&json).unwrap();
        assert!(parsed.ok);
        assert_eq!(parsed.event_ids.len(), 1);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test --lib cloud::types::sync -- --nocapture 2>&1 | tail -10`
Expected: 4 tests pass

- [ ] **Step 3: Add sync module to mod.rs**

In `src/cloud/types/mod.rs`, add:

```rust
pub mod sync;
```

And add to re-exports:

```rust
pub use sync::{
    ResolutionType, SyncCompleteRequest, SyncCompleteResponse, SyncConflictEntry,
    SyncManifestEntry, SyncPullEntry, SyncPullRequest, SyncPullResourceMeta, SyncPushEntry,
    SyncRemovedEntry, SyncResolveRequest, SyncStatusRequest, SyncStatusResponse,
};
```

- [ ] **Step 4: Verify compilation**

Run: `cargo build --all-features 2>&1 | tail -5`
Expected: compilation succeeds

- [ ] **Step 5: Commit**

```bash
git add src/cloud/types/sync.rs src/cloud/types/mod.rs
git commit -m "feat: add sync protocol types — request/response types for all sync endpoints"
```

---

### Task 7: Upload and Search Types

**Files:**
- Create: `src/cloud/types/upload.rs`
- Create: `src/cloud/types/search.rs`
- Modify: `src/cloud/types/mod.rs`

- [ ] **Step 1: Create upload types with tests**

Create `src/cloud/types/upload.rs`:

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Request body for `POST /api/upload/init`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadInitRequest {
    pub filename: String,
    pub size: u64,
    pub mime: String,
}

/// Response body for `POST /api/upload/init`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadInitResponse {
    /// Presigned PUT URL for direct R2 upload
    pub upload_url: String,
    /// Object key in R2 (e.g., "{context_id}/{resource_id}/{filename}")
    pub key: String,
}

/// Request body for `POST /api/upload/complete`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadCompleteRequest {
    pub key: String,
    pub resource_ids: Vec<Uuid>,
    pub manifest_hash: String,
}

/// Processing status for an upload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadProcessingStatus {
    /// Upload received, not yet processed
    Queued,
    /// Chunking and embedding in progress
    Processing,
    /// Chunks and embeddings written to Neon
    Complete,
    /// Processing failed
    Failed,
}

/// Response body for `GET /api/upload/:key/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadStatusResponse {
    pub key: String,
    pub status: UploadProcessingStatus,
    pub resources_processed: u32,
    pub resources_total: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upload_init_request_serde() {
        let req = UploadInitRequest {
            filename: "sync-batch-2026-03-27.zip".to_string(),
            size: 1024 * 1024,
            mime: "application/zip".to_string(),
        };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: UploadInitRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.filename, req.filename);
        assert_eq!(parsed.size, 1024 * 1024);
    }

    #[test]
    fn test_upload_processing_status_serde() {
        assert_eq!(
            serde_json::to_string(&UploadProcessingStatus::Queued).unwrap(),
            "\"queued\""
        );
        assert_eq!(
            serde_json::to_string(&UploadProcessingStatus::Complete).unwrap(),
            "\"complete\""
        );
    }
}
```

- [ ] **Step 2: Create search types with tests**

Create `src/cloud/types/search.rs`:

```rust
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Search mode — determines the query strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    /// Cosine similarity against chunk embeddings (default)
    Semantic,
    /// Full-text search via Postgres tsvector
    Keyword,
    /// Graph traversal from nearest semantic matches
    Graph,
}

impl Default for SearchMode {
    fn default() -> Self {
        Self::Semantic
    }
}

/// Query parameters for `GET /api/search`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchRequest {
    pub q: String,
    #[serde(default)]
    pub mode: SearchMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// A single search result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub resource_id: Uuid,
    pub title: String,
    pub context: String,
    pub doc_type: String,
    pub score: f64,
    pub snippet: String,
}

/// Response body for `GET /api/search`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    pub results: Vec<SearchResult>,
    pub mode: SearchMode,
    pub total: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_search_mode_default() {
        assert_eq!(SearchMode::default(), SearchMode::Semantic);
    }

    #[test]
    fn test_search_mode_serde() {
        assert_eq!(
            serde_json::to_string(&SearchMode::Graph).unwrap(),
            "\"graph\""
        );
        assert_eq!(
            serde_json::to_string(&SearchMode::Keyword).unwrap(),
            "\"keyword\""
        );
    }

    #[test]
    fn test_search_request_minimal() {
        let req = SearchRequest {
            q: "sync protocol".to_string(),
            mode: SearchMode::default(),
            context: None,
            doc_type: None,
            team: None,
            depth: None,
            limit: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(!json.contains("context"));
        assert!(!json.contains("team"));
    }

    #[test]
    fn test_search_result_serde() {
        let result = SearchResult {
            resource_id: Uuid::nil(),
            title: "R5 Design".to_string(),
            context: "temper".to_string(),
            doc_type: "research".to_string(),
            score: 0.92,
            snippet: "The sync protocol uses...".to_string(),
        };
        let json = serde_json::to_string(&result).unwrap();
        let parsed: SearchResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.title, "R5 Design");
        assert!((parsed.score - 0.92).abs() < f64::EPSILON);
    }
}
```

- [ ] **Step 3: Run tests for both modules**

Run: `cargo test --lib cloud::types::upload cloud::types::search -- --nocapture 2>&1 | tail -10`
Expected: 6 tests pass

- [ ] **Step 4: Add both modules to mod.rs**

In `src/cloud/types/mod.rs`, add:

```rust
pub mod search;
pub mod upload;
```

And add to re-exports:

```rust
pub use search::{SearchMode, SearchRequest, SearchResponse, SearchResult};
pub use upload::{
    UploadCompleteRequest, UploadInitRequest, UploadInitResponse, UploadProcessingStatus,
    UploadStatusResponse,
};
```

- [ ] **Step 5: Verify compilation**

Run: `cargo build --all-features 2>&1 | tail -5`
Expected: compilation succeeds

- [ ] **Step 6: Commit**

```bash
git add src/cloud/types/upload.rs src/cloud/types/search.rs src/cloud/types/mod.rs
git commit -m "feat: add upload pipeline and unified search types"
```

---

### Task 8: Vault and Conflict Types

**Files:**
- Create: `src/cloud/types/vault.rs`
- Create: `src/cloud/types/conflict.rs`
- Modify: `src/cloud/types/mod.rs`

- [ ] **Step 1: Create vault types with tests**

Create `src/cloud/types/vault.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The source of an ingested (non-markdown) resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum IngestionSource {
    /// Local file extracted to markdown
    File { path: String },
    /// URL fetched and extracted to markdown
    Url { url: String },
}

/// YAML frontmatter injected into vault-managed markdown files.
///
/// This is the identity anchor — everything else (tags, behaviors, team
/// associations, access levels) lives in Postgres.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceFrontmatter {
    /// Resource UUID (UUIDv7, globally unique without coordination)
    #[serde(rename = "temper-id")]
    pub temper_id: Uuid,
    pub title: String,
    pub context: String,
    pub doc_type: String,
    /// Present only for ingested resources
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ingestion_source: Option<String>,
    pub created: DateTime<Utc>,
}

/// Result of a `temper vault add` operation for a single file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultAddResult {
    pub resource_id: Uuid,
    pub vault_path: String,
    pub was_copied: bool,
    pub was_extracted: bool,
    pub source: Option<IngestionSource>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_ingestion_source_file_serde() {
        let source = IngestionSource::File {
            path: "/home/user/paper.pdf".to_string(),
        };
        let json = serde_json::to_string(&source).unwrap();
        assert!(json.contains("\"type\":\"file\""));
        let parsed: IngestionSource = serde_json::from_str(&json).unwrap();
        match parsed {
            IngestionSource::File { path } => assert_eq!(path, "/home/user/paper.pdf"),
            _ => panic!("expected File variant"),
        }
    }

    #[test]
    fn test_ingestion_source_url_serde() {
        let source = IngestionSource::Url {
            url: "https://arxiv.org/abs/2401.12345".to_string(),
        };
        let json = serde_json::to_string(&source).unwrap();
        assert!(json.contains("\"type\":\"url\""));
    }

    #[test]
    fn test_frontmatter_yaml_roundtrip() {
        let fm = ResourceFrontmatter {
            temper_id: Uuid::nil(),
            title: "Test Resource".to_string(),
            context: "temper".to_string(),
            doc_type: "research".to_string(),
            ingestion_source: None,
            created: Utc::now(),
        };
        let yaml = serde_yaml::to_string(&fm).unwrap();
        assert!(yaml.contains("temper-id:"));
        let parsed: ResourceFrontmatter = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.title, "Test Resource");
    }

    #[test]
    fn test_frontmatter_with_ingestion_source() {
        let fm = ResourceFrontmatter {
            temper_id: Uuid::nil(),
            title: "Imported Paper".to_string(),
            context: "temper".to_string(),
            doc_type: "source".to_string(),
            ingestion_source: Some("https://arxiv.org/abs/2401.12345".to_string()),
            created: Utc::now(),
        };
        let yaml = serde_yaml::to_string(&fm).unwrap();
        assert!(yaml.contains("ingestion_source:"));
    }

    #[test]
    fn test_vault_add_result_serde() {
        let result = VaultAddResult {
            resource_id: Uuid::nil(),
            vault_path: "temper/source/paper.md".to_string(),
            was_copied: true,
            was_extracted: true,
            source: Some(IngestionSource::File {
                path: "/tmp/paper.pdf".to_string(),
            }),
        };
        let json = serde_json::to_string(&result).unwrap();
        let parsed: VaultAddResult = serde_json::from_str(&json).unwrap();
        assert!(parsed.was_copied);
        assert!(parsed.was_extracted);
    }
}
```

- [ ] **Step 2: Create conflict types with tests**

Create `src/cloud/types/conflict.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Metadata for an active conflict, stored in `.temper/conflicts/`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictRecord {
    pub resource_id: Uuid,
    /// Path to the local version
    pub local_path: String,
    /// Path to the `.conflict.md` file
    pub conflict_path: String,
    /// Hash of local content at conflict detection time
    pub local_hash: String,
    /// Hash of remote content at conflict detection time
    pub remote_hash: String,
    /// When the conflict was detected
    pub detected_at: DateTime<Utc>,
}

/// A TEMPER-SYSTEM annotation block in a `.conflict.md` file.
///
/// Parsed by `temper merge` to produce the merged document with
/// section-level attribution headers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemperSystemAnnotation {
    /// Email of the profile who made the change
    pub author_email: String,
    /// When the change was made
    pub modified_at: DateTime<Utc>,
    /// Event ID for traceability
    pub event_id: Uuid,
    /// The changed content between start and end markers
    pub content: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn test_conflict_record_serde() {
        let record = ConflictRecord {
            resource_id: Uuid::nil(),
            local_path: "temper/research/sync.md".to_string(),
            conflict_path: "temper/research/sync.conflict.md".to_string(),
            local_hash: "sha256:aaa".to_string(),
            remote_hash: "sha256:bbb".to_string(),
            detected_at: Utc::now(),
        };
        let json = serde_json::to_string(&record).unwrap();
        let parsed: ConflictRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.local_path, "temper/research/sync.md");
        assert_eq!(parsed.conflict_path, "temper/research/sync.conflict.md");
    }

    #[test]
    fn test_temper_system_annotation_serde() {
        let annotation = TemperSystemAnnotation {
            author_email: "pete@example.com".to_string(),
            modified_at: Utc::now(),
            event_id: Uuid::nil(),
            content: "The sync protocol uses a three-phase approach...".to_string(),
        };
        let json = serde_json::to_string(&annotation).unwrap();
        let parsed: TemperSystemAnnotation = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.author_email, "pete@example.com");
    }
}
```

- [ ] **Step 3: Run tests for both modules**

Run: `cargo test --lib cloud::types::vault cloud::types::conflict -- --nocapture 2>&1 | tail -10`
Expected: 7 tests pass

- [ ] **Step 4: Add both modules to mod.rs**

In `src/cloud/types/mod.rs`, add:

```rust
pub mod conflict;
pub mod vault;
```

And add to re-exports:

```rust
pub use conflict::{ConflictRecord, TemperSystemAnnotation};
pub use vault::{IngestionSource, ResourceFrontmatter, VaultAddResult};
```

- [ ] **Step 5: Verify compilation**

Run: `cargo build --all-features 2>&1 | tail -5`
Expected: compilation succeeds

- [ ] **Step 6: Commit**

```bash
git add src/cloud/types/vault.rs src/cloud/types/conflict.rs src/cloud/types/mod.rs
git commit -m "feat: add vault onboarding and conflict resolution types"
```

---

### Task 9: Event Query Types

**Files:**
- Create: `src/cloud/types/event.rs`
- Modify: `src/cloud/types/mod.rs`

- [ ] **Step 1: Create event types with tests**

Create `src/cloud/types/event.rs`:

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Query parameters for `GET /api/events`.
///
/// Events are scoped by time-bounded resource visibility: you see events on
/// resources visible to you, but only events that occurred after the resource
/// became visible to you. You always see events you generated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// An event returned from the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventResponse {
    pub id: Uuid,
    pub profile_id: Uuid,
    pub client_id: String,
    pub context: Option<String>,
    pub resource_id: Option<Uuid>,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_query_minimal() {
        let query = EventQuery {
            since: None,
            context: None,
            resource_id: None,
            limit: Some(50),
        };
        let json = serde_json::to_string(&query).unwrap();
        assert!(!json.contains("since"));
        assert!(!json.contains("context"));
        assert!(json.contains("\"limit\":50"));
    }

    #[test]
    fn test_event_response_serde() {
        let event = EventResponse {
            id: Uuid::nil(),
            profile_id: Uuid::nil(),
            client_id: "device-abc".to_string(),
            context: Some("temper".to_string()),
            resource_id: Some(Uuid::nil()),
            event_type: "resource.modified".to_string(),
            payload: serde_json::json!({"content_hash": "sha256:abc"}),
            created: Utc::now(),
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: EventResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.event_type, "resource.modified");
        assert_eq!(parsed.context, Some("temper".to_string()));
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test --lib cloud::types::event -- --nocapture 2>&1 | tail -10`
Expected: 2 tests pass

- [ ] **Step 3: Add event module to mod.rs**

In `src/cloud/types/mod.rs`, add:

```rust
pub mod event;
```

And add to re-exports:

```rust
pub use event::{EventQuery, EventResponse};
```

- [ ] **Step 4: Verify compilation**

Run: `cargo build --all-features 2>&1 | tail -5`
Expected: compilation succeeds

- [ ] **Step 5: Commit**

```bash
git add src/cloud/types/event.rs src/cloud/types/mod.rs
git commit -m "feat: add event query types with time-bounded visibility model"
```

---

### Task 10: Full Build Verification and Clippy

**Files:**
- None (verification only)

- [ ] **Step 1: Run all cloud type tests**

Run: `cargo test --lib cloud -- --nocapture 2>&1 | tail -20`
Expected: All tests pass (transfer: 3, device: 2, config: 5, manifest: 3, sync: 4, upload: 2, search: 4, vault: 5, conflict: 2, event: 2 = 32 total)

- [ ] **Step 2: Run clippy**

Run: `cargo clippy --all-features -- -D warnings 2>&1 | tail -10`
Expected: No warnings

- [ ] **Step 3: Run full test suite**

Run: `cargo test --all-features 2>&1 | tail -20`
Expected: All existing tests plus new cloud type tests pass

- [ ] **Step 4: Verify formatting**

Run: `cargo fmt --check 2>&1`
Expected: No formatting issues

---

### Task 11: Research Note and Milestone Update

**Files:**
- External: knowledge vault research note
- External: milestone update

- [ ] **Step 1: Create R5 research note**

Pipe the research note content via stdin to temper:

```bash
cat <<'RESEARCH_EOF' | temper research save "R5 Indexing, Sync & Resource Management" --project temper
## Summary

R5 designed the operational layer that bridges Postgres-as-authority with files-on-disk as working artifacts. The approach was API-contract-first: define the interface between all clients and the server, then derive everything else from that contract.

## Key Findings

### API Contract as Foundation
Defining the API surface first (38 endpoints across resources, sync, teams, profiles, transfer, upload, search, events, auth) ensures every client (CLI, web UI, MCP) speaks the same language. The contract naturally aligns with the R4 crate split — temper-client wraps the API, temper-cli consumes temper-client.

### Bidirectional Sync Protocol
Single `temper sync` command replaces pull/push dichotomy. Three-phase protocol: (1) status check compares client manifest against server, returning four lists (to_pull, to_push, conflicts, removed), (2) push via batched zip through R2 presigned URL pipeline, (3) pull as zip of markdown + metadata sidecar. Subscription model in config.toml scopes which resources materialize locally.

### Manifest-Driven Reconciliation
Per-device manifest.json maps resource UUIDs to local file paths with content hashes. Five states: clean, local_modified, remote_modified, conflict, pending. Pre-flight checks compare local hash, manifest hash, and remote hash. Manifest is the bridge between "resources in Postgres" and "files on disk."

### Conflict Resolution
Side-by-side model: local file untouched, remote version materialized as `.conflict.md` with TEMPER-SYSTEM annotations (author, timestamp, event ID). Three resolution paths: pick-a-winner, merge (converts annotations to section headers with attribution), auto-merge (per-subscription policy). Partial sync continues around unresolved conflicts.

### Vault as Mutability Boundary
Files in the vault are temper-managed. `temper vault add` outside vault = copy + stamp frontmatter; inside = stamp in place. Non-markdown files (PDF, DOCX, HTML, URLs) extracted to markdown via kreuzberg — temper is a knowledge base, not a filestore. Minimal frontmatter: temper-id (UUIDv7), title, context, doc_type, created.

### CLI Primitive/Orchestration Split
CLI = CRUD primitives mapping 1:1 to API calls. Skill layer = workflow orchestration with judgment. Thin aliases (temper ticket create, temper milestone list) for ergonomics over the generic `temper resource create --type <t>`. All create commands accept stdin. All commands accept --context scoping.

## Artifacts

- Design spec: `docs/superpowers/specs/2026-03-27-r5-indexing-sync-resource-management-design.md`
- Implementation plan: `docs/superpowers/plans/2026-03-27-r5-indexing-sync-resource-management.md`
- Schema additions: kb_transfers table, kb_device_sync_state table (in R2 migration)
- Rust type stubs: 9 new modules in src/cloud/types/ (32 tests)

## Decisions

- **Hybrid sync orientation**: reads always cloud API, writes explicit, auto-sync opt-in
- **Single temper sync**: bidirectional reconcile, no pull/push split
- **Subscription model**: config.toml with context/team/doc_type filters, remote profile.preferences as default
- **Per-subscription merge policy**: manual (default) or auto
- **Server-side embedding only**: CLI never embeds, background worker via R2 upload pipeline
- **Time-bounded event visibility**: see events after resource became visible + always see own
- **Two-step transfer**: offer/accept for personal, bulk reassign for team/deactivation
- **Search unification**: single temper search with mode flag, context command becomes alias

## Open Questions for I-Phase

- Token refresh middleware implementation in temper-client
- Profile caching strategy in axum state (every API call needs profile lookup)
- Chunking diff strategy (full re-chunk vs diff-aware) — deferred to implementation
- R2 bucket lifecycle policy for uploaded zips after processing
RESEARCH_EOF
```

- [ ] **Step 2: Update milestone**

Run: `temper milestone list --project temper` to see current state, then mark R5 as complete in the milestone.

- [ ] **Step 3: Mark R5 ticket as done**

```bash
temper ticket done 2026-03-27-r5-indexing-sync-resource-management --project temper
```
