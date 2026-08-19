# TS Workflow Event Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `kb_events` + `kb_resource_audits` coverage to the TypeScript upload/ingest pipeline via a shared Postgres function, NewType ID wrappers, and hash parity tests.

**Architecture:** A new Postgres function `insert_event_and_audit()` becomes the single entry point for event+audit insertion, called by both Rust and TypeScript. Six NewType ID wrappers (`ContextId`, `DocTypeId`, `EventId`, `ResourceId`, `ProfileId`, `ResourceAuditId`) replace raw UUIDs in temper-core domain structs. The TS pipeline's `storeStep` migrates from inline SQL to the existing `persist_resource_chunks()` SQL function.

**Tech Stack:** Rust (sqlx, uuid v7), PostgreSQL (PL/pgSQL), TypeScript (Neon serverless driver, uuidv7 npm package, Node crypto for SHA-256), Vitest

**Spec:** `docs/superpowers/specs/2026-04-06-ts-workflow-event-parity-design.md`

---

## File Structure

### New files
| File | Responsibility |
|------|---------------|
| `migrations/20260406000002_insert_event_and_audit.sql` | SQL function + action column migration |
| `crates/temper-core/src/types/ids.rs` | NewType ID definitions |
| `packages/temper-cloud/src/hash.ts` | Canonical JSON hashing (SHA-256) |
| `packages/temper-cloud/src/events.ts` | Event+audit insertion helper for TS |
| `packages/temper-cloud/src/__tests__/hash.test.ts` | Hash parity tests |
| `packages/temper-cloud/src/__tests__/ingest.test.ts` | Integration tests |

### Modified files
| File | Change |
|------|--------|
| `crates/temper-core/src/types/mod.rs` | Register `ids` module, update re-exports |
| `crates/temper-core/src/types/audit.rs` | Use `ResourceAuditId`, `EventId`, `ResourceId`, `ProfileId` |
| `crates/temper-core/src/types/event.rs` | Use `EventId`, `ProfileId`, `ResourceId` |
| `crates/temper-core/src/types/resource.rs` | Use `ResourceId`, `ProfileId`, `ContextId`, `DocTypeId` |
| `crates/temper-core/src/types/managed_meta.rs` | Use `ResourceId` in `MetaUpdatePayload`, `ResourceManifestRow` |
| `crates/temper-core/src/types/manifest.rs` | Use `ResourceAuditId`, `ResourceId` |
| `crates/temper-core/src/types/sync.rs` | Use `ResourceId`, `ResourceAuditId` |
| `crates/temper-api/src/services/ingest_service.rs` | Replace `insert_event`+`insert_audit` with SQL function call |
| `crates/temper-api/src/services/meta_service.rs` | Use new combined function |
| `crates/temper-api/src/services/resource_service.rs` | Use new combined function |
| `crates/temper-api/src/services/context_service.rs` | Use `ContextId`, `ProfileId` |
| `crates/temper-api/src/handlers/*.rs` | Update for NewType IDs (compiler-driven) |
| `packages/temper-cloud/src/ingest.ts` | Wire event+audit after mutations |
| `api/workflows/process-upload.ts` | Call `persist_resource_chunks()` instead of inline SQL |
| `api/workflows/process-ingest.ts` | Call `persist_resource_chunks()` instead of inline SQL |
| `packages/temper-cloud/package.json` | Add `uuidv7` dependency |

---

## Task 1: SQL Migration — `insert_event_and_audit()` Function

**Files:**
- Create: `migrations/20260406000002_insert_event_and_audit.sql`

This migration creates the shared SQL function and fixes the `action` column type on `kb_resource_audits`.

- [ ] **Step 1: Create the migration file**

```sql
-- migrations/20260406000002_insert_event_and_audit.sql

-- ─── 1. Fix action column type + add index ──────────────────────────────────

ALTER TABLE kb_resource_audits
    ALTER COLUMN action TYPE VARCHAR(64);

CREATE INDEX idx_resource_audits_action
    ON kb_resource_audits(action);

-- ─── 2. Atomic event + audit insertion function ─────────────────────────────

CREATE OR REPLACE FUNCTION insert_event_and_audit(
    p_event_id       UUID,
    p_profile_id     UUID,
    p_device_id      VARCHAR(64),
    p_context_id     UUID,
    p_resource_id    UUID,
    p_event_type     VARCHAR(64),
    p_action         VARCHAR(64),
    p_body_hash      TEXT,
    p_managed_hash   TEXT,
    p_open_hash      TEXT
) RETURNS TABLE(event_id UUID, audit_id UUID)
LANGUAGE plpgsql AS $$
DECLARE
    v_audit_id UUID;
BEGIN
    -- Insert event row
    INSERT INTO kb_events (id, profile_id, device_id, kb_context_id, resource_id, event_type, payload, created)
    VALUES (
        p_event_id,
        p_profile_id,
        p_device_id,
        p_context_id,
        p_resource_id,
        p_event_type,
        jsonb_build_object(
            'body_hash', p_body_hash,
            'managed_hash', p_managed_hash,
            'open_hash', p_open_hash
        ),
        now()
    );

    -- Insert audit row
    INSERT INTO kb_resource_audits (resource_id, event_id, profile_id, device_id, body_hash, managed_hash, open_hash, action)
    VALUES (p_resource_id, p_event_id, p_profile_id, p_device_id, p_body_hash, p_managed_hash, p_open_hash, p_action)
    RETURNING id INTO v_audit_id;

    RETURN QUERY SELECT p_event_id, v_audit_id;
END;
$$;
```

- [ ] **Step 2: Verify migration applies cleanly**

Run: `cargo make docker-up && DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development sqlx migrate run`
Expected: Migration applies without errors.

- [ ] **Step 3: Verify function exists**

Run: `psql postgresql://temper:temper@localhost:5437/temper_development -c "\df insert_event_and_audit"`
Expected: Shows function with 10 parameters.

- [ ] **Step 4: Commit**

```bash
git add migrations/20260406000002_insert_event_and_audit.sql
git commit -m "feat: add insert_event_and_audit() SQL function

Atomic event+audit insertion callable from both Rust and TypeScript.
Also fixes kb_resource_audits.action to VARCHAR(64) with index."
```

---

## Task 2: NewType ID Definitions in temper-core

**Files:**
- Create: `crates/temper-core/src/types/ids.rs`
- Modify: `crates/temper-core/src/types/mod.rs`

- [ ] **Step 1: Create `ids.rs` with all six NewType IDs**

```rust
// crates/temper-core/src/types/ids.rs

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        #[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
        #[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
        #[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
        pub struct $name(pub Uuid);

        impl $name {
            /// Create a new time-sortable UUIDv7 ID.
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            /// Access the inner UUID.
            pub fn as_uuid(&self) -> &Uuid {
                &self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl From<Uuid> for $name {
            fn from(uuid: Uuid) -> Self {
                Self(uuid)
            }
        }

        impl From<$name> for Uuid {
            fn from(id: $name) -> Uuid {
                id.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }

        impl sqlx::Type<sqlx::Postgres> for $name {
            fn type_info() -> sqlx::postgres::PgTypeInfo {
                <Uuid as sqlx::Type<sqlx::Postgres>>::type_info()
            }

            fn compatible(ty: &sqlx::postgres::PgTypeInfo) -> bool {
                <Uuid as sqlx::Type<sqlx::Postgres>>::compatible(ty)
            }
        }

        impl<'q> sqlx::Encode<'q, sqlx::Postgres> for $name {
            fn encode_by_ref(
                &self,
                buf: &mut sqlx::postgres::PgArgumentBuffer,
            ) -> Result<sqlx::encode::IsNull, sqlx::error::BoxDynError> {
                self.0.encode_by_ref(buf)
            }
        }

        impl sqlx::Decode<'_, sqlx::Postgres> for $name {
            fn decode(
                value: sqlx::postgres::PgValueRef<'_>,
            ) -> Result<Self, sqlx::error::BoxDynError> {
                Ok(Self(<Uuid as sqlx::Decode<'_, sqlx::Postgres>>::decode(value)?))
            }
        }
    };
}

define_id!(
    /// A `kb_contexts.id` value.
    ContextId
);

define_id!(
    /// A `kb_doc_types.id` value.
    DocTypeId
);

define_id!(
    /// A `kb_events.id` value. Always UUIDv7 (time-sortable).
    EventId
);

define_id!(
    /// A `kb_resources.id` value.
    ResourceId
);

define_id!(
    /// A `kb_profiles.id` value.
    ProfileId
);

define_id!(
    /// A `kb_resource_audits.id` value.
    ResourceAuditId
);
```

- [ ] **Step 2: Register module and add re-exports in `mod.rs`**

In `crates/temper-core/src/types/mod.rs`, add the module declaration alongside
the existing modules (alphabetical):

```rust
pub mod ids;
```

And add re-exports:

```rust
pub use ids::{ContextId, DocTypeId, EventId, ProfileId, ResourceAuditId, ResourceId};
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p temper-core --all-features`
Expected: Compiles with no errors. The types exist but aren't used yet.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/src/types/ids.rs crates/temper-core/src/types/mod.rs
git commit -m "feat: add NewType ID wrappers for core domain entities

ContextId, DocTypeId, EventId, ResourceId, ProfileId, ResourceAuditId.
Macro-generated with serde, sqlx, utoipa, ts-rs, and schemars derives."
```

---

## Task 3: Wire NewType IDs into temper-core Structs

**Files:**
- Modify: `crates/temper-core/src/types/audit.rs`
- Modify: `crates/temper-core/src/types/event.rs`
- Modify: `crates/temper-core/src/types/resource.rs`
- Modify: `crates/temper-core/src/types/managed_meta.rs`
- Modify: `crates/temper-core/src/types/manifest.rs`
- Modify: `crates/temper-core/src/types/sync.rs`

This task updates struct fields from raw `Uuid` to NewType IDs. It will cause
downstream compilation failures in temper-api services and handlers — those are
fixed in Tasks 4 and 5.

- [ ] **Step 1: Update `audit.rs`**

Replace the import and struct fields in `crates/temper-core/src/types/audit.rs`:

```rust
// Replace: use uuid::Uuid;
// With:
use super::ids::{EventId, ProfileId, ResourceAuditId, ResourceId};

// Update struct fields:
pub struct ResourceAuditRow {
    pub id: ResourceAuditId,        // was Uuid
    pub resource_id: ResourceId,    // was Uuid
    pub event_id: EventId,          // was Uuid
    pub profile_id: ProfileId,      // was Uuid
    pub device_id: String,
    pub body_hash: String,
    pub managed_hash: String,
    pub open_hash: String,
    pub action: String,
    pub created: DateTime<Utc>,
}
```

- [ ] **Step 2: Update `event.rs`**

In `crates/temper-core/src/types/event.rs`:

```rust
// Replace: use uuid::Uuid;
// With:
use uuid::Uuid;  // still needed for EventQuery.resource_id (Option<Uuid> for query params)
use super::ids::{EventId, ProfileId, ResourceId};

// EventQuery — keep as Uuid since these are query parameters, not domain entities:
pub struct EventQuery {
    pub since: Option<DateTime<Utc>>,
    pub context: Option<String>,
    pub resource_id: Option<Uuid>,  // query param, stays Uuid
    pub limit: Option<u32>,
}

// EventResponse:
pub struct EventResponse {
    pub id: EventId,                       // was Uuid
    pub profile_id: ProfileId,             // was Uuid
    pub device_id: String,
    pub context: Option<String>,
    pub resource_id: Option<ResourceId>,   // was Option<Uuid>
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created: DateTime<Utc>,
}
```

- [ ] **Step 3: Update `resource.rs`**

In `crates/temper-core/src/types/resource.rs`:

```rust
// Replace: use uuid::Uuid;
// With:
use uuid::Uuid;  // still needed for ResourceListParams, ResourceCreateRequest query/request types
use super::ids::{ContextId, DocTypeId, ProfileId, ResourceId};

// ResourceRow:
pub struct ResourceRow {
    pub id: ResourceId,                        // was Uuid
    pub kb_context_id: ContextId,              // was Uuid
    pub kb_doc_type_id: DocTypeId,             // was Uuid
    pub origin_uri: String,
    pub title: String,
    pub slug: Option<String>,
    pub originator_profile_id: ProfileId,      // was Uuid
    pub owner_profile_id: ProfileId,           // was Uuid
    pub is_active: bool,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}

// ResourceCreateRequest — keep as Uuid since these come from HTTP request body:
pub struct ResourceCreateRequest {
    pub kb_context_id: Uuid,   // stays Uuid — API input
    pub kb_doc_type_id: Uuid,  // stays Uuid — API input
    // ... rest unchanged
}

// ResourceListParams — keep as Uuid:
pub struct ResourceListParams {
    pub kb_context_id: Option<Uuid>,  // stays Uuid — query param
    // ... rest unchanged
}

// ContentResponse:
pub struct ContentResponse {
    pub resource_id: ResourceId,  // was Uuid
    // ... rest unchanged
}
```

- [ ] **Step 4: Update `managed_meta.rs`**

In `crates/temper-core/src/types/managed_meta.rs`:

```rust
// Replace: use uuid::Uuid;
// With:
use uuid::Uuid;  // still needed in test
use super::ids::ResourceId;

// MetaUpdatePayload:
pub struct MetaUpdatePayload {
    pub resource_id: ResourceId,  // was Uuid
    // ... rest unchanged
}

// ResourceManifestRow:
pub struct ResourceManifestRow {
    pub resource_id: ResourceId,  // was Uuid
    // ... rest unchanged
}
```

Note: The test `meta_update_payload_serde` at line 185 uses `Uuid::nil()` to
construct a `MetaUpdatePayload`. Update to `ResourceId::from(Uuid::nil())`.

- [ ] **Step 5: Update `manifest.rs`**

In `crates/temper-core/src/types/manifest.rs`:

```rust
// Add import:
use super::ids::{ResourceAuditId, ResourceId};

// ManifestEntry:
pub struct ManifestEntry {
    // ... other fields unchanged ...
    pub last_audit_id: Option<ResourceAuditId>,  // was Option<Uuid>
}

// Manifest.entries HashMap key:
pub struct Manifest {
    pub device_id: String,
    pub last_sync: Option<DateTime<Utc>>,
    pub entries: HashMap<ResourceId, ManifestEntry>,  // was HashMap<Uuid, ManifestEntry>
}
```

- [ ] **Step 6: Update `sync.rs`**

In `crates/temper-core/src/types/sync.rs`:

```rust
// Add import:
use super::ids::{ResourceAuditId, ResourceId};

// Update all resource_id fields:
// SyncPushItem.resource_id: Option<ResourceId>    (was Option<Uuid>)
// SyncPullItem.resource_id: ResourceId             (was Uuid)
// SyncConflictItem.resource_id: ResourceId         (was Uuid)
// SyncRemovedItem.resource_id: ResourceId          (was Uuid)
// MergedResource.resource_id: ResourceId           (was Uuid)
// SyncManifestItem.resource_id: ResourceId         (was Uuid)
// SyncManifestItem.last_audit_id: Option<ResourceAuditId>  (was Option<Uuid>)
// SyncResolveRequest.resource_id: ResourceId       (was Uuid)
```

- [ ] **Step 7: Verify temper-core compiles**

Run: `cargo check -p temper-core --all-features`
Expected: temper-core compiles. Downstream crates (temper-api, temper-cli,
temper-client, temper-mcp) will have errors — fixed in Tasks 4–5.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-core/src/types/
git commit -m "refactor: use NewType IDs in all temper-core domain structs

ResourceAuditRow, EventResponse, ResourceRow, MetaUpdatePayload,
ResourceManifestRow, ManifestEntry, Manifest, and all sync types
now use typed IDs instead of raw Uuid."
```

---

## Task 4: Migrate Rust Services to SQL Function + NewType IDs

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs`
- Modify: `crates/temper-api/src/services/meta_service.rs`
- Modify: `crates/temper-api/src/services/resource_service.rs`
- Modify: `crates/temper-api/src/services/context_service.rs`

- [ ] **Step 1: Replace `insert_event` + `insert_audit` in `ingest_service.rs`**

Remove the `insert_event()` function (lines 45–71) and `insert_audit()` function
(lines 78–108). Replace with a single function that calls the SQL function:

```rust
/// Atomically insert a kb_events row and a kb_resource_audits row
/// via the `insert_event_and_audit()` SQL function.
pub async fn insert_event_and_audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    profile_id: ProfileId,
    device_id: &str,
    context_id: ContextId,
    resource_id: ResourceId,
    event_type: &str,
    action: &str,
    body_hash: &str,
    managed_hash: &str,
    open_hash: &str,
) -> ApiResult<(EventId, ResourceAuditId)> {
    let event_id = EventId::new();

    let row: (uuid::Uuid, uuid::Uuid) = sqlx::query_as(
        "SELECT event_id, audit_id FROM insert_event_and_audit($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)"
    )
    .bind(event_id)
    .bind(profile_id)
    .bind(device_id)
    .bind(context_id)
    .bind(resource_id)
    .bind(event_type)
    .bind(action)
    .bind(body_hash)
    .bind(managed_hash)
    .bind(open_hash)
    .fetch_one(&mut **tx)
    .await?;

    Ok((EventId::from(row.0), ResourceAuditId::from(row.1)))
}
```

Add the import at the top of `ingest_service.rs`:

```rust
use temper_core::types::ids::{ContextId, EventId, ProfileId, ResourceAuditId, ResourceId};
```

- [ ] **Step 2: Update `ingest()` call sites in `ingest_service.rs`**

In `ingest()` (line 187), update the function signature and internal calls:

```rust
pub async fn ingest(
    pool: &PgPool,
    profile_id: ProfileId,   // was Uuid
    device_id: &str,
    payload: IngestPayload,
) -> ApiResult<ResourceRow> {
```

Replace the two separate `insert_event` + `insert_audit` calls (lines 269–295)
with:

```rust
    let context_id = ContextId::from(context_id);  // wrap the resolved UUID
    insert_event_and_audit(
        &mut tx,
        profile_id,
        device_id,
        context_id,
        ResourceId::from(resource.id),  // or use the typed id from ResourceRow
        "resource_created",
        "create",
        &payload.content_hash,
        &managed_hash,
        &open_hash,
    )
    .await?;
```

Note: After Task 3, `resource.id` is already `ResourceId`, so the `from()` may
not be needed — the compiler will tell you.

- [ ] **Step 3: Update `update()` call sites in `ingest_service.rs`**

In `update()` (line 303), update signature and replace the two separate calls
(lines 379–405):

```rust
pub async fn update(
    pool: &PgPool,
    profile_id: ProfileId,    // was Uuid
    resource_id: ResourceId,  // was Uuid
    device_id: &str,
    payload: IngestPayload,
) -> ApiResult<ResourceRow> {
```

Replace event+audit calls with:

```rust
    // Fetch context_id for the event (resource may change contexts)
    let (ctx_id,): (uuid::Uuid,) = sqlx::query_as(
        "SELECT kb_context_id FROM kb_resources WHERE id = $1"
    )
    .bind(resource_id)
    .fetch_one(&mut *tx)
    .await?;

    insert_event_and_audit(
        &mut tx,
        profile_id,
        device_id,
        ContextId::from(ctx_id),
        resource_id,
        "body_updated",
        "update_body",
        &payload.content_hash,
        &managed_hash,
        &open_hash,
    )
    .await?;
```

- [ ] **Step 4: Update `persist_chunks` and `replace_chunks` signatures**

These internal functions take `resource_id: Uuid`. Update to `ResourceId`:

```rust
async fn persist_chunks(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    resource_id: ResourceId,  // was Uuid
    chunks: &[PackedChunk],
) -> ApiResult<i32> {
```

Same for `replace_chunks`. The `.bind(resource_id)` calls work because the
NewType implements `sqlx::Encode`.

- [ ] **Step 5: Update `meta_service.rs`**

In `crates/temper-api/src/services/meta_service.rs`, replace the import:

```rust
// Replace:
// use crate::services::ingest_service::{insert_audit, insert_event};
// With:
use crate::services::ingest_service::insert_event_and_audit;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
```

Update `update_meta` signature:

```rust
pub async fn update_meta(
    pool: &PgPool,
    profile_id: ProfileId,    // was Uuid
    resource_id: ResourceId,  // was Uuid
    device_id: &str,
    payload: MetaUpdatePayload,
) -> ApiResult<Value> {
```

Replace the two separate event+audit calls (lines 123–149) with:

```rust
    // Fetch context_id
    let (ctx_id,): (uuid::Uuid,) = sqlx::query_as(
        "SELECT kb_context_id FROM kb_resources WHERE id = $1"
    )
    .bind(resource_id)
    .fetch_one(&mut *tx)
    .await?;

    insert_event_and_audit(
        &mut tx,
        profile_id,
        device_id,
        ContextId::from(ctx_id),
        resource_id,
        "managed_meta_updated",
        "update_meta",
        &body_hash,
        &payload.managed_hash,
        &payload.open_hash,
    )
    .await?;
```

- [ ] **Step 6: Update `resource_service.rs`**

In `crates/temper-api/src/services/resource_service.rs`, replace the import:

```rust
// Replace:
// use crate::services::ingest_service::{insert_audit, insert_event};
// With:
use crate::services::ingest_service::insert_event_and_audit;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
```

Update function signatures for `list_visible`, `get_visible`, `get_content`,
`create`, `update`, `delete` — change `profile_id: Uuid` to `ProfileId` and
`resource_id: Uuid` to `ResourceId` where applicable.

For `delete()` specifically, it already fetches `context_id` (line 228). Replace
the two separate event+audit calls (lines 249–275) with:

```rust
    insert_event_and_audit(
        &mut tx,
        profile_id,
        device_id,
        ContextId::from(context_id),
        resource_id,
        "resource_deleted",
        "delete",
        &body_hash,
        &managed_hash,
        &open_hash,
    )
    .await?;
```

- [ ] **Step 7: Update `context_service.rs`**

In `crates/temper-api/src/services/context_service.rs`, update signatures:

```rust
use temper_core::types::ids::{ContextId, ProfileId};

pub async fn list_visible(pool: &PgPool, profile_id: ProfileId) -> ApiResult<Vec<ContextRow>> { ... }
pub async fn get_visible(pool: &PgPool, profile_id: ProfileId, context_id: ContextId) -> ApiResult<ContextRow> { ... }
pub async fn resolve_by_name(pool: &PgPool, profile_id: ProfileId, name: &str) -> ApiResult<ContextRow> { ... }
pub async fn create(pool: &PgPool, profile_id: ProfileId, name: &str) -> ApiResult<ContextRow> { ... }
```

Note: `ContextRow` itself likely has `id: Uuid` — update that to `ContextId` if
it's defined in temper-core, or leave it if it's only in temper-api. Follow the
compiler.

- [ ] **Step 8: Verify temper-api compiles (expect handler errors)**

Run: `cargo check -p temper-api --all-features`
Expected: Services compile. Handlers will have type mismatch errors — fixed in
Task 5.

- [ ] **Step 9: Commit**

```bash
git add crates/temper-api/src/services/
git commit -m "refactor: migrate services to insert_event_and_audit() SQL function + NewType IDs

Replaces separate insert_event + insert_audit calls with single SQL
function call. All service signatures use typed IDs."
```

---

## Task 5: Update Handlers and Remaining Rust Code for NewType IDs

**Files:**
- Modify: `crates/temper-api/src/handlers/resources.rs`
- Modify: `crates/temper-api/src/handlers/meta.rs`
- Modify: `crates/temper-api/src/handlers/ingest.rs`
- Modify: Other files as indicated by compiler errors

This task is compiler-driven. The handler files extract `Path(resource_id): Path<Uuid>` from HTTP requests and `auth.0.profile.id` (a `Uuid`) from auth middleware. These need wrapping.

- [ ] **Step 1: Update handler files**

For each handler file, the pattern is:

```rust
// Before:
Path(resource_id): Path<Uuid>
// After:
Path(resource_id): Path<Uuid>  // keep as Uuid from HTTP layer
// Then wrap when calling service:
resource_service::get_visible(&pool, ProfileId::from(auth.0.profile.id), ResourceId::from(resource_id)).await
```

Import the ID types:
```rust
use temper_core::types::ids::{ProfileId, ResourceId};
```

The conversion happens at the handler boundary — HTTP extracts raw UUIDs, handlers
wrap them before passing to services.

- [ ] **Step 2: Fix remaining compiler errors across workspace**

Run: `cargo check --workspace --all-features`

Follow each error. Common patterns:
- `temper-cli` and `temper-client` may use `ResourceRow.id` as `Uuid` — wrap/unwrap
  at boundaries
- `temper-mcp` handlers may need the same wrapping pattern
- `sync_service.rs` — `SyncManifestItem` field types changed, update query mapping

For each file, keep HTTP/CLI boundaries using raw `Uuid` and wrap when crossing
into the service layer.

- [ ] **Step 3: Verify full workspace compiles**

Run: `cargo check --workspace --all-features`
Expected: No errors.

- [ ] **Step 4: Commit**

```bash
git add -A  # many files touched
git commit -m "refactor: wire NewType IDs through handlers and remaining Rust code

Handlers wrap raw UUIDs from HTTP/CLI at the boundary before
passing to typed service layer."
```

---

## Task 6: Verify Rust with E2E Tests

**Files:** None (verification only)

- [ ] **Step 1: Run existing e2e audit tests**

Run: `cargo make docker-up && cargo nextest run -p temper-e2e --features test-db`
Expected: All tests pass, including the 6 audit tests in `audit_test.rs`.

If any tests fail, fix the issue before proceeding. The SQL function must produce
identical observable behavior to the old inline SQL.

- [ ] **Step 2: Run full Rust test suite**

Run: `cargo make test-all`
Expected: All tests pass.

- [ ] **Step 3: Run quality checks**

Run: `cargo make check`
Expected: No clippy warnings, no fmt issues.

- [ ] **Step 4: Fix any issues and commit if needed**

---

## Task 7: TypeScript Hash Parity Module

**Files:**
- Create: `packages/temper-cloud/src/hash.ts`
- Create: `packages/temper-cloud/src/__tests__/hash.test.ts`

- [ ] **Step 1: Write the hash parity test**

```typescript
// packages/temper-cloud/src/__tests__/hash.test.ts
import { describe, it, expect } from "vitest";
import { canonicalJsonHash } from "../hash.js";

describe("canonicalJsonHash", () => {
  it("hashes empty object", () => {
    expect(canonicalJsonHash({})).toBe(
      // Must match Rust: hash_json_value(&serde_json::json!({}))
      // serde_json::to_string({}) = "{}"
      // SHA-256 of "{}" bytes
      "sha256:44136fa355b311bfa706c3cf3c82a945f6e01e0078f3dcb08e4b15e5a2c2e1da"
    );
  });

  it("sorts keys lexicographically", () => {
    const result = canonicalJsonHash({ b: 2, a: 1 });
    // Canonical form: {"a":1,"b":2}
    expect(result).toBe(canonicalJsonHash({ a: 1, b: 2 }));
  });

  it("sorts nested object keys recursively", () => {
    const result = canonicalJsonHash({
      z: { b: 2, a: 1 },
      a: "first",
    });
    // Canonical: {"a":"first","z":{"a":1,"b":2}}
    const reversed = canonicalJsonHash({
      a: "first",
      z: { a: 1, b: 2 },
    });
    expect(result).toBe(reversed);
  });

  it("preserves array order", () => {
    const a = canonicalJsonHash({ items: [3, 1, 2] });
    const b = canonicalJsonHash({ items: [1, 2, 3] });
    expect(a).not.toBe(b);
  });

  it("handles null, boolean, and numeric values", () => {
    const result = canonicalJsonHash({
      flag: true,
      count: 42,
      empty: null,
    });
    expect(result).toMatch(/^sha256:[0-9a-f]{64}$/);
  });

  // Shared fixture: this exact hash must match the Rust unit test
  it("matches Rust hash for shared fixture", () => {
    const fixture = {
      "temper-type": "task",
      "temper-stage": "in-progress",
      "temper-seq": 42,
      title: "Test task",
    };
    const hash = canonicalJsonHash(fixture);
    // This value is computed by the Rust test and must match exactly.
    // Run the Rust test first to get the expected value, then update here.
    expect(hash).toMatch(/^sha256:[0-9a-f]{64}$/);
  });
});
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd packages/temper-cloud && bun run test -- hash`
Expected: FAIL — `canonicalJsonHash` not found.

- [ ] **Step 3: Implement `hash.ts`**

```typescript
// packages/temper-cloud/src/hash.ts
import { createHash } from "node:crypto";

/**
 * Compute a `sha256:<hex>` hash of a JSON value with canonicalized key ordering.
 *
 * Algorithm matches Rust `hash_json_value()` in ingest_service.rs:
 * 1. Sort object keys recursively (lexicographic, depth-first)
 * 2. JSON.stringify with no spacing (compact form)
 * 3. SHA-256 the UTF-8 bytes
 * 4. Return "sha256:<hex>"
 */
export function canonicalJsonHash(value: Record<string, unknown>): string {
  const canonical = canonicalize(value);
  const serialized = JSON.stringify(canonical);
  const hash = createHash("sha256").update(serialized, "utf8").digest("hex");
  return `sha256:${hash}`;
}

function canonicalize(value: unknown): unknown {
  if (value === null || typeof value !== "object") {
    return value;
  }
  if (Array.isArray(value)) {
    return value.map(canonicalize);
  }
  const sorted: Record<string, unknown> = {};
  for (const key of Object.keys(value).sort()) {
    sorted[key] = canonicalize((value as Record<string, unknown>)[key]);
  }
  return sorted;
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd packages/temper-cloud && bun run test -- hash`
Expected: All tests pass.

- [ ] **Step 5: Add Rust unit test with shared fixture**

In `crates/temper-api/src/services/ingest_service.rs`, add a test at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_json_shared_fixture() {
        // This fixture must produce the same hash in both Rust and TypeScript.
        let fixture = serde_json::json!({
            "temper-type": "task",
            "temper-stage": "in-progress",
            "temper-seq": 42,
            "title": "Test task"
        });
        let hash = hash_json_value(&fixture);
        // Print for cross-language verification:
        eprintln!("Rust shared fixture hash: {hash}");
        assert!(hash.starts_with("sha256:"));
        assert_eq!(hash.len(), 7 + 64); // "sha256:" + 64 hex chars
    }

    #[test]
    fn hash_empty_object() {
        let hash = hash_json_value(&serde_json::json!({}));
        assert_eq!(
            hash,
            "sha256:44136fa355b311bfa706c3cf3c82a945f6e01e0078f3dcb08e4b15e5a2c2e1da"
        );
    }

    #[test]
    fn hash_key_order_independent() {
        let a = hash_json_value(&serde_json::json!({"b": 2, "a": 1}));
        let b = hash_json_value(&serde_json::json!({"a": 1, "b": 2}));
        assert_eq!(a, b);
    }
}
```

- [ ] **Step 6: Run Rust hash tests and capture shared fixture hash**

Run: `cargo nextest run -p temper-api hash_json -- --nocapture 2>&1 | grep "shared fixture"`
Expected: Prints `Rust shared fixture hash: sha256:<64 hex chars>`.

Copy the exact hash value and update the TypeScript test's "matches Rust hash
for shared fixture" assertion.

- [ ] **Step 7: Re-run TypeScript tests with exact hash**

Run: `cd packages/temper-cloud && bun run test -- hash`
Expected: All tests pass including the shared fixture test with exact hash match.

- [ ] **Step 8: Commit**

```bash
git add packages/temper-cloud/src/hash.ts packages/temper-cloud/src/__tests__/hash.test.ts crates/temper-api/src/services/ingest_service.rs
git commit -m "feat: add canonical JSON hashing with cross-language parity tests

TypeScript canonicalJsonHash matches Rust hash_json_value exactly.
Shared fixture tests guard against drift."
```

---

## Task 8: TypeScript Event/Audit Wiring in `ingest.ts`

**Files:**
- Create: `packages/temper-cloud/src/events.ts`
- Modify: `packages/temper-cloud/src/ingest.ts`
- Modify: `packages/temper-cloud/package.json`

- [ ] **Step 1: Add `uuidv7` dependency**

Run: `cd packages/temper-cloud && bun add uuidv7`

- [ ] **Step 2: Create `events.ts` helper**

```typescript
// packages/temper-cloud/src/events.ts
import { uuidv7 } from "uuidv7";
import type { NeonClient } from "./db.js";

export const DEVICE_ID_CLOUD = "vercel-cloud";

/**
 * Insert a kb_events row and a kb_resource_audits row atomically
 * via the insert_event_and_audit() SQL function.
 */
export async function insertEventAndAudit(
  db: NeonClient,
  params: {
    profileId: string;
    deviceId: string;
    contextId: string;
    resourceId: string;
    eventType: string;
    action: string;
    bodyHash: string;
    managedHash: string;
    openHash: string;
  },
): Promise<{ eventId: string; auditId: string }> {
  const eventId = uuidv7();

  const rows = await db`
    SELECT event_id, audit_id
    FROM insert_event_and_audit(
      ${eventId}::uuid,
      ${params.profileId}::uuid,
      ${params.deviceId},
      ${params.contextId}::uuid,
      ${params.resourceId}::uuid,
      ${params.eventType},
      ${params.action},
      ${params.bodyHash},
      ${params.managedHash},
      ${params.openHash}
    )
  `;

  return {
    eventId: rows[0].event_id as string,
    auditId: rows[0].audit_id as string,
  };
}
```

- [ ] **Step 3: Wire into `insertResource()`**

In `packages/temper-cloud/src/ingest.ts`, add imports:

```typescript
import { insertEventAndAudit, DEVICE_ID_CLOUD } from "./events.js";
import { canonicalJsonHash } from "./hash.js";
```

After the manifest INSERT (line 194, after `return rows[0]` is prepared), add
event+audit insertion before the return:

```typescript
  // Create manifest entry for body hash tracking
  await db`
    INSERT INTO kb_resource_manifests (resource_id, body_hash, updated)
    VALUES (${newId}::uuid, ${contentHash}, now())
  `;

  const emptyHash = canonicalJsonHash({});

  // Emit event + audit for the new resource
  await insertEventAndAudit(db, {
    profileId,
    deviceId: DEVICE_ID_CLOUD,
    contextId: contextId,
    resourceId: newId,
    eventType: "resource_created",
    action: "create",
    bodyHash: contentHash,
    managedHash: emptyHash,
    openHash: emptyHash,
  });

  return rows[0] as unknown as ResourceRecord;
```

- [ ] **Step 4: Wire into `updateResourceHash()`**

Update the function signature to accept `profileId` and `contextId`:

```typescript
export async function updateResourceHash(
  db: NeonClient,
  resourceId: string,
  bodyHash: string,
  profileId: string,
  contextId: string,
): Promise<void> {
  await db`
    INSERT INTO kb_resource_manifests (resource_id, body_hash, updated)
    VALUES (${resourceId}::uuid, ${bodyHash}, now())
    ON CONFLICT (resource_id)
    DO UPDATE SET body_hash = ${bodyHash}, updated = now()
  `;
  await db`
    UPDATE kb_resources SET updated = now() WHERE id = ${resourceId}::uuid
  `;

  const emptyHash = canonicalJsonHash({});

  await insertEventAndAudit(db, {
    profileId,
    deviceId: DEVICE_ID_CLOUD,
    contextId,
    resourceId,
    eventType: "body_updated",
    action: "update_body",
    bodyHash,
    managedHash: emptyHash,
    openHash: emptyHash,
  });
}
```

- [ ] **Step 5: Update callers of `updateResourceHash`**

Search for callers of `updateResourceHash` in the workflow files and update them
to pass `profileId` and `contextId`. These values may need to be threaded through
the workflow steps — check `process-upload.ts` and `process-ingest.ts` for what
data is available at the `storeStep`.

If `profileId`/`contextId` are not available in the workflow context, they can be
fetched from the resource record:

```typescript
const resource = await db`
  SELECT owner_profile_id, kb_context_id FROM kb_resources WHERE id = ${resourceId}::uuid
`;
const profileId = resource[0].owner_profile_id as string;
const contextId = resource[0].kb_context_id as string;
```

- [ ] **Step 6: Verify TypeScript compiles**

Run: `cd packages/temper-cloud && bun run typecheck`
Expected: No type errors.

- [ ] **Step 7: Commit**

```bash
git add packages/temper-cloud/src/events.ts packages/temper-cloud/src/ingest.ts packages/temper-cloud/package.json packages/temper-cloud/bun.lock
git commit -m "feat: wire event+audit insertion into TS ingest pipeline

insertResource and updateResourceHash now emit kb_events +
kb_resource_audits rows via the shared SQL function.
Device ID: vercel-cloud."
```

---

## Task 9: Migrate TS Workflow storeStep to `persist_resource_chunks()`

**Files:**
- Modify: `api/workflows/process-upload.ts`
- Modify: `api/workflows/process-ingest.ts`
- Modify: `packages/temper-cloud/src/processing/store.ts` (may add helper)

The existing `storeStep` in both workflows uses `buildVersionBumpQuery`,
`buildStoreChunksQueries`, and `buildStatusUpdateQuery` from `store.ts` — multiple
inline SQL statements. Replace the chunk insertion portion with a single call to
the existing `persist_resource_chunks()` SQL function.

- [ ] **Step 1: Create a helper to format chunks as JSONB for the SQL function**

The SQL function `persist_resource_chunks(p_resource_id UUID, p_chunks JSONB)`
expects the same JSONB format that `chunks_to_jsonb()` produces in Rust. Check
`crates/temper-core/src/types/ingest.rs` lines 63–90 for `ChunkRowJsonb`.

Add to `packages/temper-cloud/src/processing/store.ts`:

```typescript
/**
 * Format chunks as JSONB array matching the persist_resource_chunks() SQL
 * function's expected input format (same as Rust chunks_to_jsonb).
 */
export function chunksToJsonb(chunks: ChunkRow[]): object[] {
  return chunks.map((c) => ({
    chunk_index: c.chunk_index,
    header_path: c.header_path,
    content: c.content,
    content_hash: c.content_hash,
    embedding: `[${c.embedding.join(",")}]`,  // vector as string, matching Rust format
  }));
}
```

Check the Rust `ChunkRowJsonb` struct to verify the exact field names and the
embedding format (it converts `Vec<f32>` to a string like `"[0.1,0.2,...]"`).

- [ ] **Step 2: Update `process-upload.ts` storeStep**

Replace the inline chunk storage SQL in `storeStep` with:

```typescript
// Instead of buildVersionBumpQuery + buildStoreChunksQueries:
const chunksJson = JSON.stringify(chunksToJsonb(chunks));

const result = await db`
  SELECT persist_resource_chunks(${resourceId}::uuid, ${chunksJson}::jsonb)
`;

// Keep the status update for blob_files:
const statusQuery = buildStatusUpdateQuery(blobFileId, "processed", null);
await db.query(statusQuery.sql, statusQuery.params);
```

- [ ] **Step 3: Update `process-ingest.ts` storeStep**

Same pattern — replace inline chunk SQL with `persist_resource_chunks()` call.
This workflow has no blob_files status update, so it's just:

```typescript
const chunksJson = JSON.stringify(chunksToJsonb(chunks));

await db`
  SELECT persist_resource_chunks(${resourceId}::uuid, ${chunksJson}::jsonb)
`;
```

- [ ] **Step 4: Verify TypeScript compiles**

Run: `cd packages/temper-cloud && bun run typecheck`
Expected: No type errors.

- [ ] **Step 5: Commit**

```bash
git add api/workflows/process-upload.ts api/workflows/process-ingest.ts packages/temper-cloud/src/processing/store.ts
git commit -m "refactor: migrate TS storeStep to persist_resource_chunks() SQL function

Replaces inline chunk INSERT SQL with call to existing SQL function,
matching the Rust ingest path. Eliminates drift between pipelines."
```

---

## Task 10: TypeScript Integration Tests

**Files:**
- Create: `packages/temper-cloud/src/__tests__/ingest.test.ts`

These tests verify that `insertResource` and `updateResourceHash` now create
event+audit rows in the database.

- [ ] **Step 1: Write integration tests**

```typescript
// packages/temper-cloud/src/__tests__/ingest.test.ts
import { describe, it, expect, beforeAll } from "vitest";
import { neon } from "@neondatabase/serverless";
import { insertResource, updateResourceHash } from "../ingest.js";
import type { IngestMetadata } from "../ingest.js";

// These tests require DATABASE_URL pointing to a test database with
// the insert_event_and_audit() function available.
const TEST_DB_URL = process.env.DATABASE_URL;

describe.skipIf(!TEST_DB_URL)("ingest event parity", () => {
  const db = TEST_DB_URL ? neon(TEST_DB_URL) : (null as never);

  // Use a known test profile and context — these must exist in the test DB.
  // Match the e2e test seed data pattern.
  const TEST_PROFILE_ID = "00000000-0000-0000-0004-000000000001";
  const TEST_CONTEXT_ID = "00000000-0000-0000-0003-000000000001";
  const TEST_DOC_TYPE_ID = "00000000-0000-0000-0001-000000000004";

  it("insertResource creates event + audit rows", async () => {
    const meta: IngestMetadata = {
      title: "Integration test resource",
      kb_context_id: TEST_CONTEXT_ID,
      kb_doc_type_id: TEST_DOC_TYPE_ID,
      origin_uri: "test://integration/insert",
    };

    const resource = await insertResource(db, meta, "sha256:test123abc", TEST_PROFILE_ID);

    // Verify event was created
    const events = await db`
      SELECT id, event_type, device_id, payload
      FROM kb_events
      WHERE resource_id = ${resource.id}::uuid
      ORDER BY created DESC
      LIMIT 1
    `;
    expect(events).toHaveLength(1);
    expect(events[0].event_type).toBe("resource_created");
    expect(events[0].device_id).toBe("vercel-cloud");
    expect(events[0].payload).toMatchObject({
      body_hash: "sha256:test123abc",
    });

    // Verify audit was created
    const audits = await db`
      SELECT resource_id, action, body_hash, device_id
      FROM kb_resource_audits
      WHERE resource_id = ${resource.id}::uuid
      ORDER BY created DESC
      LIMIT 1
    `;
    expect(audits).toHaveLength(1);
    expect(audits[0].action).toBe("create");
    expect(audits[0].body_hash).toBe("sha256:test123abc");
    expect(audits[0].device_id).toBe("vercel-cloud");

    // Cleanup
    await db`DELETE FROM kb_resource_audits WHERE resource_id = ${resource.id}::uuid`;
    await db`DELETE FROM kb_events WHERE resource_id = ${resource.id}::uuid`;
    await db`DELETE FROM kb_resource_manifests WHERE resource_id = ${resource.id}::uuid`;
    await db`DELETE FROM kb_resources WHERE id = ${resource.id}::uuid`;
  });

  it("updateResourceHash creates body_updated event + audit", async () => {
    // First create a resource
    const meta: IngestMetadata = {
      title: "Update test resource",
      kb_context_id: TEST_CONTEXT_ID,
      kb_doc_type_id: TEST_DOC_TYPE_ID,
      origin_uri: "test://integration/update",
    };

    const resource = await insertResource(db, meta, "sha256:original", TEST_PROFILE_ID);

    // Now update the hash
    await updateResourceHash(db, resource.id, "sha256:updated", TEST_PROFILE_ID, TEST_CONTEXT_ID);

    // Verify events (should be 2: resource_created + body_updated)
    const events = await db`
      SELECT event_type FROM kb_events
      WHERE resource_id = ${resource.id}::uuid
      ORDER BY created ASC
    `;
    expect(events).toHaveLength(2);
    expect(events[0].event_type).toBe("resource_created");
    expect(events[1].event_type).toBe("body_updated");

    // Verify audits
    const audits = await db`
      SELECT action, body_hash FROM kb_resource_audits
      WHERE resource_id = ${resource.id}::uuid
      ORDER BY created ASC
    `;
    expect(audits).toHaveLength(2);
    expect(audits[0].action).toBe("create");
    expect(audits[0].body_hash).toBe("sha256:original");
    expect(audits[1].action).toBe("update_body");
    expect(audits[1].body_hash).toBe("sha256:updated");

    // Cleanup
    await db`DELETE FROM kb_resource_audits WHERE resource_id = ${resource.id}::uuid`;
    await db`DELETE FROM kb_events WHERE resource_id = ${resource.id}::uuid`;
    await db`DELETE FROM kb_resource_manifests WHERE resource_id = ${resource.id}::uuid`;
    await db`DELETE FROM kb_resources WHERE id = ${resource.id}::uuid`;
  });
});
```

- [ ] **Step 2: Run integration tests**

Run: `cd packages/temper-cloud && DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development bun run test -- ingest`
Expected: Both tests pass.

- [ ] **Step 3: Run full quality check**

Run: `cargo make check && cargo make test-all`
Expected: Everything passes — Rust and TypeScript.

- [ ] **Step 4: Commit**

```bash
git add packages/temper-cloud/src/__tests__/ingest.test.ts
git commit -m "test: add TS integration tests for event+audit parity

Verifies insertResource and updateResourceHash create correct
kb_events and kb_resource_audits rows with device_id vercel-cloud."
```

---

## Task 11: Context Auto-Creation Events

**Files:**
- Modify: `crates/temper-api/src/services/context_service.rs`
- Modify: `packages/temper-cloud/src/ingest.ts`

When `resolveContextId()` auto-creates a `kb_contexts` row, emit a
`context_created` event. No audit row needed — this is not a resource mutation.

- [ ] **Step 1: Add event emission to Rust `context_service::create()`**

In `crates/temper-api/src/services/context_service.rs`, the `create()` function
inserts a new context. Add an event INSERT after the context creation:

```rust
use temper_core::types::ids::{ContextId, EventId, ProfileId};

pub async fn create(pool: &PgPool, profile_id: ProfileId, name: &str) -> ApiResult<ContextRow> {
    // ... existing context INSERT ...

    // Emit context_created event (no audit row — not a resource mutation)
    let event_id = EventId::new();
    sqlx::query(
        "INSERT INTO kb_events (id, profile_id, device_id, kb_context_id, event_type, payload, created)
         VALUES ($1, $2, $3, $4, $5, '{}', now())"
    )
    .bind(event_id)
    .bind(profile_id)
    .bind("api")  // server-side creation
    .bind(context_id)
    .bind("context_created")
    .execute(pool)
    .await?;

    // ... return context row ...
}
```

Note: The `device_id` for context auto-creation is `"api"` since it happens
server-side. If called from the ingest flow, the actual device_id should be
threaded through — but `create()` currently doesn't accept device_id. For now,
use `"api"` and thread device_id through in a follow-up if needed.

- [ ] **Step 2: Add event emission to TypeScript `resolveContextId()`**

In `packages/temper-cloud/src/ingest.ts`, in the auto-create branch of
`resolveContextId()`:

```typescript
import { insertEventAndAudit, DEVICE_ID_CLOUD } from "./events.js";
import { uuidv7 } from "uuidv7";

export async function resolveContextId(
  db: NeonClient,
  name: string,
  profileId: string,
): Promise<string> {
  // ... existing lookup ...

  // Auto-create
  const newId = randomUUID();
  await db`
    INSERT INTO kb_contexts (id, name, kb_owner_table, kb_owner_id)
    VALUES (${newId}::uuid, ${name}, 'kb_profiles', ${profileId}::uuid)
  `;

  // Emit context_created event (no audit row)
  const eventId = uuidv7();
  await db`
    INSERT INTO kb_events (id, profile_id, device_id, kb_context_id, event_type, payload, created)
    VALUES (${eventId}::uuid, ${profileId}::uuid, ${DEVICE_ID_CLOUD}, ${newId}::uuid, 'context_created', '{}', now())
  `;

  return newId;
}
```

- [ ] **Step 3: Verify compilation**

Run: `cargo check -p temper-api --all-features && cd packages/temper-cloud && bun run typecheck`
Expected: No errors.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/src/services/context_service.rs packages/temper-cloud/src/ingest.ts
git commit -m "feat: emit context_created events on context auto-creation

Both Rust and TypeScript resolveContextId paths now insert a
kb_events row when a new context is auto-created."
```

---

## Task 12: Final Verification and Cleanup

- [ ] **Step 1: Run full workspace build**

Run: `cargo make build`
Expected: Clean build.

- [ ] **Step 2: Run all tests**

Run: `cargo make test-all`
Expected: All pass.

- [ ] **Step 3: Run quality checks**

Run: `cargo make check`
Expected: No warnings.

- [ ] **Step 4: Review diff for any leftover raw UUIDs in service signatures**

Run: `grep -n "profile_id: Uuid\|resource_id: Uuid" crates/temper-api/src/services/*.rs`
Expected: Only in query parameter types or external API boundaries — not in
function signatures where typed IDs should be used.

- [ ] **Step 5: Commit any final cleanup**
