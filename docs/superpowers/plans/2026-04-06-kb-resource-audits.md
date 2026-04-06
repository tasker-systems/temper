# kb_resource_audits Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an audit trail table (`kb_resource_audits`) that links every resource mutation to its event and captures hash state at mutation time, enabling precedence-based sync conflict resolution.

**Architecture:** New `kb_resource_audits` table joins `kb_events` and `kb_resources` with hash snapshots. All four mutation paths (ingest, update, update_meta, delete) insert an audit row inside their existing transaction. `insert_event()` is changed to return the event UUID so callers can reference it. Sync endpoints expose `last_audit_id` per resource. `ManifestEntry` gains an optional `last_audit_id` field for client-side tracking.

**Tech Stack:** Rust (sqlx, Axum, serde, uuid), PostgreSQL 18, cargo-nextest for testing

---

## File Structure

### New Files
| File | Responsibility |
|------|---------------|
| `migrations/20260406000001_resource_audits.sql` | DDL for `kb_resource_audits` table + indexes |
| `crates/temper-core/src/types/audit.rs` | `ResourceAuditRow` type (shared between API and client) |
| `tests/e2e/tests/audit_test.rs` | E2E tests for audit trail through all mutation paths |

### Modified Files
| File | Change |
|------|--------|
| `crates/temper-core/src/types/mod.rs` | Add `pub mod audit` + re-export `ResourceAuditRow` |
| `crates/temper-core/src/types/manifest.rs:25-59` | Add `last_audit_id: Option<Uuid>` to `ManifestEntry` |
| `crates/temper-core/src/types/sync.rs:108-118` | Add `last_audit_id: Option<Uuid>` to `SyncManifestItem` |
| `crates/temper-api/src/services/ingest_service.rs:45-71` | Change `insert_event()` return type from `ApiResult<()>` to `ApiResult<Uuid>` |
| `crates/temper-api/src/services/ingest_service.rs:150-245` | Add audit row insertion in `ingest()` |
| `crates/temper-api/src/services/ingest_service.rs:248-337` | Add audit row insertion in `update()` |
| `crates/temper-api/src/services/meta_service.rs:15-130` | Add audit row insertion in `update_meta()` |
| `crates/temper-api/src/services/resource_service.rs:198-223` | Rewrite `delete()` with transaction, event, and audit row |
| `crates/temper-api/src/services/sync_service.rs:184-242` | Add `last_audit_id` to `ManifestRow` and `fetch_manifest()` query |
| `tests/e2e/tests/common/mod.rs:104-193` | Add `kb_resource_audits` cleanup to `clean_and_seed()` |

---

## Task 1: Migration — Create kb_resource_audits table

**Files:**
- Create: `migrations/20260406000001_resource_audits.sql`

- [ ] **Step 1: Write the migration SQL**

```sql
-- kb_resource_audits: audit trail linking resource mutations to events with hash snapshots.
-- Used for precedence-based sync conflict resolution (not compliance — CASCADE on delete).

CREATE TABLE kb_resource_audits (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    resource_id UUID NOT NULL REFERENCES kb_resources(id) ON DELETE CASCADE,
    event_id    UUID NOT NULL REFERENCES kb_events(id) ON DELETE CASCADE,
    profile_id  UUID NOT NULL REFERENCES kb_profiles(id),
    device_id   TEXT NOT NULL,
    body_hash   TEXT NOT NULL,
    managed_hash TEXT NOT NULL,
    open_hash   TEXT NOT NULL,
    action      TEXT NOT NULL,
    created     TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(resource_id, event_id)
);

CREATE INDEX idx_resource_audits_resource ON kb_resource_audits(resource_id, created DESC);
CREATE INDEX idx_resource_audits_event ON kb_resource_audits(event_id);
```

- [ ] **Step 2: Run the migration**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development sqlx migrate run`
Expected: migration applied successfully, no errors.

- [ ] **Step 3: Verify the table exists**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development sqlx migrate info`
Expected: `20260406000001_resource_audits` shows as applied.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260406000001_resource_audits.sql
git commit -m "feat: add kb_resource_audits migration for audit trail"
```

---

## Task 2: Add ResourceAuditRow type to temper-core

**Files:**
- Create: `crates/temper-core/src/types/audit.rs`
- Modify: `crates/temper-core/src/types/mod.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/temper-core/src/types/audit.rs`:

```rust
//! Audit trail types — tracks resource mutations with hash snapshots.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

/// Row type matching the `kb_resource_audits` table.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "audit.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceAuditRow {
    pub id: Uuid,
    pub resource_id: Uuid,
    pub event_id: Uuid,
    pub profile_id: Uuid,
    pub device_id: String,
    pub body_hash: String,
    pub managed_hash: String,
    pub open_hash: String,
    pub action: String,
    pub created: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_audit_row_serde_roundtrip() {
        let row = ResourceAuditRow {
            id: Uuid::nil(),
            resource_id: Uuid::nil(),
            event_id: Uuid::nil(),
            profile_id: Uuid::nil(),
            device_id: "test-device".to_string(),
            body_hash: "sha256:abc".to_string(),
            managed_hash: "sha256:def".to_string(),
            open_hash: "sha256:ghi".to_string(),
            action: "create".to_string(),
            created: Utc::now(),
        };
        let json = serde_json::to_string(&row).unwrap();
        let parsed: ResourceAuditRow = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.resource_id, Uuid::nil());
        assert_eq!(parsed.action, "create");
        assert_eq!(parsed.device_id, "test-device");
    }
}
```

- [ ] **Step 2: Register the module in mod.rs**

In `crates/temper-core/src/types/mod.rs`, add `pub mod audit;` after line 10 (after `pub mod api;`), and add the re-export after line 37:

Add module declaration:
```rust
pub mod audit;
```

Add re-export (after the `pub use api::` line):
```rust
pub use audit::ResourceAuditRow;
```

- [ ] **Step 3: Run test to verify it passes**

Run: `cargo nextest run --workspace resource_audit_row_serde_roundtrip`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/src/types/audit.rs crates/temper-core/src/types/mod.rs
git commit -m "feat: add ResourceAuditRow type to temper-core"
```

---

## Task 3: Change insert_event() to return Uuid

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs:45-71`

- [ ] **Step 1: Write a unit test for insert_event returning Uuid**

This change is tested implicitly by compilation — `insert_event()` is called in 3 places that will fail to compile if the return type changes unexpectedly. The key verification is that all callers still compile. No separate unit test needed — the existing E2E tests cover the behavior.

- [ ] **Step 2: Change insert_event() return type**

In `crates/temper-api/src/services/ingest_service.rs`, change lines 45-71.

Old signature and return:
```rust
pub async fn insert_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    profile_id: Uuid,
    device_id: &str,
    context_id: Option<Uuid>,
    resource_id: Option<Uuid>,
    event_type: &str,
    payload: &serde_json::Value,
) -> ApiResult<()> {
    let event_id = Uuid::now_v7();
    sqlx::query(
        r#"
        INSERT INTO kb_events (id, profile_id, device_id, kb_context_id, resource_id, event_type, payload, created)
        VALUES ($1, $2, $3, $4, $5, $6, $7, now())
        "#,
    )
    .bind(event_id)
    .bind(profile_id)
    .bind(device_id)
    .bind(context_id)
    .bind(resource_id)
    .bind(event_type)
    .bind(payload)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
```

New signature and return:
```rust
pub async fn insert_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    profile_id: Uuid,
    device_id: &str,
    context_id: Option<Uuid>,
    resource_id: Option<Uuid>,
    event_type: &str,
    payload: &serde_json::Value,
) -> ApiResult<Uuid> {
    let event_id = Uuid::now_v7();
    sqlx::query(
        r#"
        INSERT INTO kb_events (id, profile_id, device_id, kb_context_id, resource_id, event_type, payload, created)
        VALUES ($1, $2, $3, $4, $5, $6, $7, now())
        "#,
    )
    .bind(event_id)
    .bind(profile_id)
    .bind(device_id)
    .bind(context_id)
    .bind(resource_id)
    .bind(event_type)
    .bind(payload)
    .execute(&mut **tx)
    .await?;
    Ok(event_id)
}
```

- [ ] **Step 3: Fix all callers to handle new return type**

There are three callers that currently use `.await?;` (discarding the `()`). They need to either capture the returned `Uuid` or explicitly ignore it with `let _ =`. For now, we'll use `let _ =` — the audit row insertion in later tasks will change these to capture the value.

In `ingest_service.rs` `ingest()` (line 231), change:
```rust
    insert_event(
        &mut tx,
        ...
    )
    .await?;
```
to:
```rust
    let _event_id = insert_event(
        &mut tx,
        ...
    )
    .await?;
```

In `ingest_service.rs` `update()` (line 323), same pattern:
```rust
    let _event_id = insert_event(
        &mut tx,
        ...
    )
    .await?;
```

In `meta_service.rs` `update_meta()` (line 113), same pattern:
```rust
    let _event_id = insert_event(
        &mut tx,
        ...
    )
    .await?;
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check --all-features`
Expected: compiles with no errors. May have warnings about unused `_event_id` which is fine (underscore prefix suppresses).

- [ ] **Step 5: Run existing tests**

Run: `cargo nextest run --workspace`
Expected: all existing tests pass unchanged.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/services/ingest_service.rs crates/temper-api/src/services/meta_service.rs
git commit -m "refactor: insert_event() returns event Uuid for audit linkage"
```

---

## Task 4: Add insert_audit() helper and wire into ingest()

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs`

- [ ] **Step 1: Add the insert_audit() helper function**

Add after `insert_event()` (after line 71) in `ingest_service.rs`:

```rust
/// Insert an audit trail row into kb_resource_audits.
pub async fn insert_audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    resource_id: Uuid,
    event_id: Uuid,
    profile_id: Uuid,
    device_id: &str,
    body_hash: &str,
    managed_hash: &str,
    open_hash: &str,
    action: &str,
) -> ApiResult<Uuid> {
    let audit_id: (Uuid,) = sqlx::query_as(
        r#"
        INSERT INTO kb_resource_audits
            (resource_id, event_id, profile_id, device_id, body_hash, managed_hash, open_hash, action)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING id
        "#,
    )
    .bind(resource_id)
    .bind(event_id)
    .bind(profile_id)
    .bind(device_id)
    .bind(body_hash)
    .bind(managed_hash)
    .bind(open_hash)
    .bind(action)
    .fetch_one(&mut **tx)
    .await?;
    Ok(audit_id.0)
}
```

- [ ] **Step 2: Wire insert_audit() into ingest()**

In the `ingest()` function, change the event insertion block (lines 230-240). Replace:
```rust
    let _event_id = insert_event(
        &mut tx,
        profile_id,
        "api",
        Some(context.id),
        Some(resource_id),
        "resource_created",
        &serde_json::json!({"body_hash": &payload.content_hash}),
    )
    .await?;
```

With:
```rust
    let event_id = insert_event(
        &mut tx,
        profile_id,
        "api",
        Some(context.id),
        Some(resource_id),
        "resource_created",
        &serde_json::json!({"body_hash": &payload.content_hash}),
    )
    .await?;

    insert_audit(
        &mut tx,
        resource_id,
        event_id,
        profile_id,
        "api",
        &payload.content_hash,
        &managed_hash,
        &open_hash,
        "create",
    )
    .await?;
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check --all-features`
Expected: compiles with no errors.

- [ ] **Step 4: Run existing tests**

Run: `cargo nextest run --workspace`
Expected: all existing tests pass (audit row insertion is additive).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/services/ingest_service.rs
git commit -m "feat: add insert_audit() helper, wire into ingest() for create action"
```

---

## Task 5: Wire audit into update() and update_meta()

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs:248-337`
- Modify: `crates/temper-api/src/services/meta_service.rs:15-130`

- [ ] **Step 1: Wire insert_audit() into update()**

In `ingest_service.rs` `update()`, replace the event insertion block (lines 322-332):

Old:
```rust
    let _event_id = insert_event(
        &mut tx,
        profile_id,
        "api",
        Some(resource.kb_context_id),
        Some(resource_id),
        "body_updated",
        &serde_json::json!({"body_hash": &payload.content_hash}),
    )
    .await?;
```

New:
```rust
    let event_id = insert_event(
        &mut tx,
        profile_id,
        "api",
        Some(resource.kb_context_id),
        Some(resource_id),
        "body_updated",
        &serde_json::json!({"body_hash": &payload.content_hash}),
    )
    .await?;

    insert_audit(
        &mut tx,
        resource_id,
        event_id,
        profile_id,
        "api",
        &payload.content_hash,
        &managed_hash,
        &open_hash,
        "update_body",
    )
    .await?;
```

- [ ] **Step 2: Wire insert_audit() into update_meta()**

In `meta_service.rs`, first add the import. Change line 9 from:
```rust
use crate::services::ingest_service::insert_event;
```
to:
```rust
use crate::services::ingest_service::{insert_audit, insert_event};
```

Then replace the event insertion block (lines 112-125):

Old:
```rust
    let _event_id = insert_event(
        &mut tx,
        profile_id,
        "api",
        None,
        Some(resource_id),
        "managed_meta_updated",
        &serde_json::json!({
            "managed_hash": &payload.managed_hash,
            "open_hash": &payload.open_hash,
        }),
    )
    .await?;
```

New (we need to fetch `body_hash` for the audit row — read it from the manifest row we just updated):
```rust
    // Fetch current body_hash for audit snapshot
    let (body_hash,): (String,) = sqlx::query_as(
        "SELECT body_hash FROM kb_resource_manifests WHERE resource_id = $1",
    )
    .bind(resource_id)
    .fetch_one(&mut *tx)
    .await?;

    let event_id = insert_event(
        &mut tx,
        profile_id,
        "api",
        None,
        Some(resource_id),
        "managed_meta_updated",
        &serde_json::json!({
            "managed_hash": &payload.managed_hash,
            "open_hash": &payload.open_hash,
        }),
    )
    .await?;

    insert_audit(
        &mut tx,
        resource_id,
        event_id,
        profile_id,
        "api",
        &body_hash,
        &payload.managed_hash,
        &payload.open_hash,
        "update_meta",
    )
    .await?;
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check --all-features`
Expected: compiles with no errors.

- [ ] **Step 4: Run existing tests**

Run: `cargo nextest run --workspace`
Expected: all existing tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/services/ingest_service.rs crates/temper-api/src/services/meta_service.rs
git commit -m "feat: wire audit trail into update() and update_meta() mutations"
```

---

## Task 6: Add event + audit to resource_service::delete()

**Files:**
- Modify: `crates/temper-api/src/services/resource_service.rs:198-223`

- [ ] **Step 1: Rewrite delete() with transaction, event, and audit**

In `resource_service.rs`, add the necessary imports at the top (after line 3):
```rust
use crate::services::ingest_service::{insert_audit, insert_event};
```

Then replace the `delete()` function (lines 198-223):

Old:
```rust
pub async fn delete(pool: &PgPool, profile_id: Uuid, resource_id: Uuid) -> ApiResult<()> {
    let can_modify: bool = sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
        .bind(profile_id)
        .bind(resource_id)
        .fetch_one(pool)
        .await?;

    if !can_modify {
        return Err(ApiError::Forbidden);
    }

    sqlx::query(
        r#"
        UPDATE kb_resources
           SET is_active = false,
               updated   = now()
         WHERE id = $1
           AND is_active = true
        "#,
    )
    .bind(resource_id)
    .execute(pool)
    .await?;

    Ok(())
}
```

New:
```rust
pub async fn delete(pool: &PgPool, profile_id: Uuid, resource_id: Uuid) -> ApiResult<()> {
    let can_modify: bool = sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
        .bind(profile_id)
        .bind(resource_id)
        .fetch_one(pool)
        .await?;

    if !can_modify {
        return Err(ApiError::Forbidden);
    }

    let mut tx = pool.begin().await?;

    // Fetch current hashes for the audit snapshot before soft-delete
    let hashes: Option<(String, String, String)> = sqlx::query_as(
        "SELECT body_hash, managed_hash, open_hash FROM kb_resource_manifests WHERE resource_id = $1",
    )
    .bind(resource_id)
    .fetch_optional(&mut *tx)
    .await?;

    let (body_hash, managed_hash, open_hash) = hashes.unwrap_or_default();

    // Fetch context_id for the event
    let (context_id,): (Uuid,) = sqlx::query_as(
        "SELECT kb_context_id FROM kb_resources WHERE id = $1",
    )
    .bind(resource_id)
    .fetch_one(&mut *tx)
    .await?;

    // Soft-delete the resource
    sqlx::query(
        r#"
        UPDATE kb_resources
           SET is_active = false,
               updated   = now()
         WHERE id = $1
           AND is_active = true
        "#,
    )
    .bind(resource_id)
    .execute(&mut *tx)
    .await?;

    // Record event and audit
    let event_id = insert_event(
        &mut tx,
        profile_id,
        "api",
        Some(context_id),
        Some(resource_id),
        "resource_deleted",
        &serde_json::json!({}),
    )
    .await?;

    insert_audit(
        &mut tx,
        resource_id,
        event_id,
        profile_id,
        "api",
        &body_hash,
        &managed_hash,
        &open_hash,
        "delete",
    )
    .await?;

    tx.commit().await?;

    Ok(())
}
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check --all-features`
Expected: compiles with no errors.

- [ ] **Step 3: Run existing tests**

Run: `cargo nextest run --workspace`
Expected: all existing tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/src/services/resource_service.rs
git commit -m "feat: add event + audit trail to resource delete"
```

---

## Task 7: Add last_audit_id to ManifestEntry and SyncManifestItem

**Files:**
- Modify: `crates/temper-core/src/types/manifest.rs:25-59`
- Modify: `crates/temper-core/src/types/sync.rs:108-118`

- [ ] **Step 1: Write failing test for ManifestEntry backward compatibility**

In `crates/temper-core/src/types/manifest.rs`, add this test at the end of the `mod tests` block (before the closing `}`):

```rust
    #[test]
    fn test_manifest_entry_last_audit_id_defaults_none() {
        let old_json = serde_json::json!({
            "path": "temper/goals/my-goal.md",
            "body_hash": "sha256:body",
            "remote_body_hash": "sha256:remote",
            "synced_at": "2026-01-01T00:00:00Z",
            "state": "clean"
        });
        let entry: ManifestEntry = serde_json::from_value(old_json).unwrap();
        assert!(
            entry.last_audit_id.is_none(),
            "last_audit_id should default to None for old manifests"
        );
    }

    #[test]
    fn test_manifest_entry_last_audit_id_roundtrip() {
        let id = Uuid::now_v7();
        let entry = ManifestEntry {
            path: "temper/sessions/s1.md".to_string(),
            body_hash: "sha256:body".to_string(),
            remote_body_hash: "sha256:rbody".to_string(),
            managed_hash: String::new(),
            open_hash: String::new(),
            remote_managed_hash: String::new(),
            remote_open_hash: String::new(),
            synced_at: Utc::now(),
            state: ManifestEntryState::Clean,
            mtime_secs: None,
            provisional: false,
            last_audit_id: Some(id),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let parsed: ManifestEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.last_audit_id, Some(id));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run --workspace test_manifest_entry_last_audit`
Expected: FAIL — `last_audit_id` field doesn't exist yet.

- [ ] **Step 3: Add last_audit_id field to ManifestEntry**

In `crates/temper-core/src/types/manifest.rs`, add after line 58 (after the `provisional` field):

```rust
    /// Most recent kb_resource_audits.id this device is aware of.
    /// Used for precedence-based sync conflict resolution.
    #[serde(default)]
    pub last_audit_id: Option<Uuid>,
```

- [ ] **Step 4: Fix existing tests that construct ManifestEntry**

Update all existing `ManifestEntry` literals in `manifest.rs` tests to include `last_audit_id: None`. There are four test functions that construct `ManifestEntry`:

1. `test_manifest_json_roundtrip` (line ~122): add `last_audit_id: None,` after `provisional: false,`
2. `test_manifest_entry_new_format_roundtrip` (line ~168): add `last_audit_id: None,` after `provisional: false,`
3. `test_manifest_entry_provisional_roundtrip` (line ~214): add `last_audit_id: None,` after `provisional: true,`

- [ ] **Step 5: Add last_audit_id to SyncManifestItem**

In `crates/temper-core/src/types/sync.rs`, add after line 117 (after `uri` field in `SyncManifestItem`):

```rust
    /// Most recent audit ID for this resource on the server.
    #[serde(default)]
    pub last_audit_id: Option<Uuid>,
```

- [ ] **Step 6: Fix existing tests that construct SyncManifestItem**

In `sync.rs` tests, `sync_manifest_response_serde_roundtrip` (line ~232) constructs a `SyncManifestItem`. Add `last_audit_id: None,` after `uri`.

- [ ] **Step 7: Run all tests**

Run: `cargo nextest run --workspace`
Expected: all tests pass including the new backward-compatibility tests.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-core/src/types/manifest.rs crates/temper-core/src/types/sync.rs
git commit -m "feat: add last_audit_id to ManifestEntry and SyncManifestItem"
```

---

## Task 8: Expose last_audit_id in fetch_manifest()

**Files:**
- Modify: `crates/temper-api/src/services/sync_service.rs:184-242`

- [ ] **Step 1: Add last_audit_id to ManifestRow**

In `sync_service.rs`, change the `ManifestRow` struct (lines 185-194):

Old:
```rust
struct ManifestRow {
    resource_id: Uuid,
    context_name: String,
    doc_type_name: String,
    slug: String,
    body_hash: String,
    managed_hash: String,
    open_hash: String,
}
```

New:
```rust
struct ManifestRow {
    resource_id: Uuid,
    context_name: String,
    doc_type_name: String,
    slug: String,
    body_hash: String,
    managed_hash: String,
    open_hash: String,
    last_audit_id: Option<Uuid>,
}
```

- [ ] **Step 2: Update fetch_manifest() SQL query**

Replace the SQL in `fetch_manifest()` (lines 199-219):

Old:
```rust
    let rows = sqlx::query_as::<_, ManifestRow>(
        r#"
        SELECT r.id AS resource_id,
               c.name AS context_name,
               d.name AS doc_type_name,
               COALESCE(r.slug, '') AS slug,
               COALESCE(m.body_hash, '') AS body_hash,
               COALESCE(m.managed_hash, '') AS managed_hash,
               COALESCE(m.open_hash, '') AS open_hash
          FROM kb_resources r
          JOIN kb_contexts c ON c.id = r.kb_context_id
          JOIN kb_doc_types d ON d.id = r.kb_doc_type_id
          LEFT JOIN kb_resource_manifests m ON m.resource_id = r.id
         WHERE r.owner_profile_id = $1
           AND r.is_active = true
         ORDER BY c.name, d.name, r.slug
        "#,
    )
```

New:
```rust
    let rows = sqlx::query_as::<_, ManifestRow>(
        r#"
        SELECT r.id AS resource_id,
               c.name AS context_name,
               d.name AS doc_type_name,
               COALESCE(r.slug, '') AS slug,
               COALESCE(m.body_hash, '') AS body_hash,
               COALESCE(m.managed_hash, '') AS managed_hash,
               COALESCE(m.open_hash, '') AS open_hash,
               (SELECT a.id FROM kb_resource_audits a
                 WHERE a.resource_id = r.id
                 ORDER BY a.created DESC LIMIT 1) AS last_audit_id
          FROM kb_resources r
          JOIN kb_contexts c ON c.id = r.kb_context_id
          JOIN kb_doc_types d ON d.id = r.kb_doc_type_id
          LEFT JOIN kb_resource_manifests m ON m.resource_id = r.id
         WHERE r.owner_profile_id = $1
           AND r.is_active = true
         ORDER BY c.name, d.name, r.slug
        "#,
    )
```

- [ ] **Step 3: Pass last_audit_id through to SyncManifestItem**

Update the map closure in `fetch_manifest()` (lines 221-239). Change:

```rust
            SyncManifestItem {
                resource_id: row.resource_id,
                context: row.context_name,
                doc_type: row.doc_type_name,
                slug: row.slug,
                content_hash: row.body_hash,
                managed_hash: row.managed_hash,
                open_hash: row.open_hash,
                uri,
            }
```

To:
```rust
            SyncManifestItem {
                resource_id: row.resource_id,
                context: row.context_name,
                doc_type: row.doc_type_name,
                slug: row.slug,
                content_hash: row.body_hash,
                managed_hash: row.managed_hash,
                open_hash: row.open_hash,
                uri,
                last_audit_id: row.last_audit_id,
            }
```

- [ ] **Step 4: Verify compilation**

Run: `cargo check --all-features`
Expected: compiles with no errors.

- [ ] **Step 5: Run existing tests**

Run: `cargo nextest run --workspace`
Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/services/sync_service.rs
git commit -m "feat: expose last_audit_id in sync manifest endpoint"
```

---

## Task 9: Update sqlx offline cache

**Files:**
- Modify: `.sqlx/` directory (auto-generated)

- [ ] **Step 1: Run sqlx prepare**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo sqlx prepare --workspace`
Expected: updated `.sqlx/` query cache files.

- [ ] **Step 2: Verify clean build with offline cache**

Run: `cargo check --all-features`
Expected: compiles with no errors.

- [ ] **Step 3: Commit**

```bash
git add .sqlx/
git commit -m "chore: update sqlx offline cache for audit queries"
```

---

## Task 10: E2E tests for audit trail

**Files:**
- Create: `tests/e2e/tests/audit_test.rs`
- Modify: `tests/e2e/tests/common/mod.rs:104-193`

- [ ] **Step 1: Add kb_resource_audits cleanup to test fixtures**

In `tests/e2e/tests/common/mod.rs`, add a cleanup statement for `kb_resource_audits` **before** the `kb_events` cleanup (before line 105, since audits reference events via FK):

```rust
    sqlx::query("DELETE FROM kb_resource_audits")
        .execute(pool)
        .await
        .expect("clean kb_resource_audits");
```

- [ ] **Step 2: Write the E2E test file**

Create `tests/e2e/tests/audit_test.rs`:

```rust
#![cfg(feature = "test-db")]

mod common;

use temper_core::types::api::EventListParams;
use temper_core::types::ingest::{pack_chunks, IngestPayload};

/// Helper: create a context and ingest a resource, return (resource_id, context_name).
async fn ingest_test_resource(
    app: &common::E2eTestApp,
    suffix: &str,
) -> (uuid::Uuid, String) {
    let context_name = format!("e2e-audit-{suffix}");
    app.client
        .contexts()
        .create(&context_name)
        .await
        .expect("context create failed");

    let payload = IngestPayload {
        title: format!("Audit Test Doc {suffix}"),
        origin_uri: format!("test://e2e/audit-{suffix}"),
        context_name: context_name.clone(),
        doc_type_name: "research".to_string(),
        content_hash: format!(
            "audit{suffix}000000000000000000000000000000000000000000000000000000000"
        ),
        slug: format!("audit-test-{suffix}"),
        content: format!("# Audit Test {suffix}\n\nContent for audit testing."),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: pack_chunks(&[]).expect("encode empty chunks"),
    };

    let resource = app
        .client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest create failed");

    (resource.id, context_name)
}

/// Ingest creates a resource_created event and a corresponding audit row.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn audit_row_created_on_ingest(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client.profile().get().await.expect("profile pre-flight");

    let (resource_id, _ctx) = ingest_test_resource(&app, "create").await;

    // Verify audit row exists via direct DB query
    let audit_rows: Vec<(uuid::Uuid, String, String)> = sqlx::query_as(
        "SELECT id, action, body_hash FROM kb_resource_audits WHERE resource_id = $1 ORDER BY created",
    )
    .bind(resource_id)
    .fetch_all(&pool)
    .await
    .expect("query audit rows");

    assert_eq!(audit_rows.len(), 1, "expected exactly one audit row after ingest");
    assert_eq!(audit_rows[0].1, "create");
    assert!(!audit_rows[0].2.is_empty(), "body_hash should not be empty");
}

/// Updating a resource's body creates an update_body audit row.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn audit_row_created_on_update(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client.profile().get().await.expect("profile pre-flight");

    let (resource_id, ctx) = ingest_test_resource(&app, "update").await;

    // Update the resource
    let update_payload = IngestPayload {
        title: "Audit Test Doc update - Updated".to_string(),
        origin_uri: "test://e2e/audit-update".to_string(),
        context_name: ctx,
        doc_type_name: "research".to_string(),
        content_hash: "auditupd0000000000000000000000000000000000000000000000000000000"
            .to_string(),
        slug: "audit-test-update".to_string(),
        content: "# Updated\n\nNew content.".to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: pack_chunks(&[]).expect("encode empty chunks"),
    };

    app.client
        .ingest()
        .update(resource_id, &update_payload)
        .await
        .expect("ingest update failed");

    // Verify two audit rows: create + update_body
    let audit_rows: Vec<(String,)> = sqlx::query_as(
        "SELECT action FROM kb_resource_audits WHERE resource_id = $1 ORDER BY created",
    )
    .bind(resource_id)
    .fetch_all(&pool)
    .await
    .expect("query audit rows");

    assert_eq!(audit_rows.len(), 2);
    assert_eq!(audit_rows[0].0, "create");
    assert_eq!(audit_rows[1].0, "update_body");
}

/// Updating managed meta creates an update_meta audit row.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn audit_row_created_on_meta_update(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client.profile().get().await.expect("profile pre-flight");

    let (resource_id, _ctx) = ingest_test_resource(&app, "meta").await;

    // Update meta via the API
    let meta_payload = serde_json::json!({
        "managed_meta": {"title": "Updated Title"},
        "open_meta": {},
        "managed_hash": "sha256:newmanaged",
        "open_hash": "sha256:newopen",
    });

    let url = app.url(&format!("/api/resources/{resource_id}/meta"));
    let resp = app
        .reqwest_client
        .put(&url)
        .bearer_auth(&app.token)
        .json(&meta_payload)
        .send()
        .await
        .expect("meta update request failed");

    assert!(resp.status().is_success(), "meta update failed: {}", resp.status());

    // Verify audit rows: create + update_meta
    let audit_rows: Vec<(String,)> = sqlx::query_as(
        "SELECT action FROM kb_resource_audits WHERE resource_id = $1 ORDER BY created",
    )
    .bind(resource_id)
    .fetch_all(&pool)
    .await
    .expect("query audit rows");

    assert_eq!(audit_rows.len(), 2);
    assert_eq!(audit_rows[0].0, "create");
    assert_eq!(audit_rows[1].0, "update_meta");
}

/// Deleting a resource creates a delete audit row.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn audit_row_created_on_delete(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client.profile().get().await.expect("profile pre-flight");

    let (resource_id, _ctx) = ingest_test_resource(&app, "delete").await;

    // Delete the resource
    app.client
        .resources()
        .delete(resource_id)
        .await
        .expect("delete failed");

    // Verify audit rows: create + delete
    let audit_rows: Vec<(String,)> = sqlx::query_as(
        "SELECT action FROM kb_resource_audits WHERE resource_id = $1 ORDER BY created",
    )
    .bind(resource_id)
    .fetch_all(&pool)
    .await
    .expect("query audit rows");

    assert_eq!(audit_rows.len(), 2);
    assert_eq!(audit_rows[0].0, "create");
    assert_eq!(audit_rows[1].0, "delete");
}

/// Audit rows link to valid events (foreign key integrity).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn audit_row_references_valid_event(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client.profile().get().await.expect("profile pre-flight");

    let (resource_id, _ctx) = ingest_test_resource(&app, "fk").await;

    // Verify the audit row's event_id exists in kb_events
    let valid_count: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*) FROM kb_resource_audits a
        JOIN kb_events e ON e.id = a.event_id
        WHERE a.resource_id = $1
        "#,
    )
    .bind(resource_id)
    .fetch_one(&pool)
    .await
    .expect("join query");

    assert_eq!(valid_count.0, 1, "audit row should reference a valid event");
}

/// Fetch manifest includes last_audit_id for resources with audits.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn manifest_includes_last_audit_id(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client.profile().get().await.expect("profile pre-flight");

    let (resource_id, _ctx) = ingest_test_resource(&app, "manifest").await;

    // Fetch manifest via API
    let url = app.url("/api/sync/manifest");
    let resp = app
        .reqwest_client
        .get(&url)
        .bearer_auth(&app.token)
        .send()
        .await
        .expect("manifest request failed");

    assert!(resp.status().is_success());

    let body: serde_json::Value = resp.json().await.expect("parse manifest response");
    let items = body["items"].as_array().expect("items is array");

    let item = items
        .iter()
        .find(|i| i["resource_id"].as_str() == Some(&resource_id.to_string()))
        .expect("resource not found in manifest");

    assert!(
        item["last_audit_id"].is_string(),
        "last_audit_id should be present after ingest"
    );
}
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check --all-features -p temper-e2e`
Expected: compiles with no errors.

- [ ] **Step 4: Run the E2E tests**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_test cargo nextest run -p temper-e2e --features test-db audit`
Expected: all 6 audit tests pass.

- [ ] **Step 5: Commit**

```bash
git add tests/e2e/tests/audit_test.rs tests/e2e/tests/common/mod.rs
git commit -m "test: add E2E tests for kb_resource_audits audit trail"
```

---

## Task 11: Full verification

- [ ] **Step 1: Run clippy**

Run: `cargo clippy --all-features -- -D warnings`
Expected: no warnings.

- [ ] **Step 2: Run full test suite**

Run: `cargo make test-all-rust`
Expected: all tests pass.

- [ ] **Step 3: Run format check**

Run: `cargo fmt --check`
Expected: no formatting issues.

- [ ] **Step 4: Fix any issues found, re-verify, commit fixes**

If any issues found, fix them and create a commit for each fix.
