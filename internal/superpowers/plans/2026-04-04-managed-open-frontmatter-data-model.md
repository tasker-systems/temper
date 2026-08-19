# Managed & Open Frontmatter Data Model — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement three-tier frontmatter persistence (body, managed meta, open meta) across temper-core types, database schema, API handlers, CLI templates, and sync protocol.

**Architecture:** New `kb_resource_manifests` table with managed_meta/open_meta JSONB columns and three-tier hashes. Shared `ManagedMeta` struct in temper-core drives both CLI YAML parsing and API JSON persistence. Sync protocol becomes tier-aware (body vs meta changes). Templates and CLI actions emit `temper-*` prefixed field names. API creates `kb_events` on state changes.

**Tech Stack:** Rust (serde, sqlx, axum, askama), PostgreSQL (JSONB, GIN indexes), JSON Schema

**Spec:** `docs/superpowers/specs/2026-04-04-managed-open-frontmatter-data-model-design.md`

**Build/test commands:**
- Quality checks: `cargo make check`
- Unit tests: `cargo make test`
- Single test: `cargo nextest run --workspace test_name`
- Integration tests: `cargo make docker-up && cargo make test-db`

---

## Task 1: ManagedMeta and MetaUpdatePayload types in temper-core

**Files:**
- Create: `crates/temper-core/src/types/managed_meta.rs`
- Modify: `crates/temper-core/src/types/mod.rs`

- [ ] **Step 1: Create managed_meta.rs with ManagedMeta struct**

```rust
// crates/temper-core/src/types/managed_meta.rs
use serde::{Deserialize, Serialize};

/// Managed frontmatter fields — temper-governed, varies by doctype.
/// Serialized to/from JSONB in kb_resource_manifests.managed_meta
/// and to/from YAML frontmatter in vault files.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ManagedMeta {
    #[serde(rename = "temper-type", skip_serializing_if = "Option::is_none")]
    pub doc_type: Option<String>,
    #[serde(rename = "temper-context", skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(rename = "temper-updated", skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    #[serde(rename = "temper-source", skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(rename = "temper-legacy-id", skip_serializing_if = "Option::is_none")]
    pub legacy_id: Option<String>,
    // task fields
    #[serde(rename = "temper-stage", skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(rename = "temper-mode", skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(rename = "temper-effort", skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(rename = "temper-goal", skip_serializing_if = "Option::is_none")]
    pub goal: Option<String>,
    #[serde(rename = "temper-seq", skip_serializing_if = "Option::is_none")]
    pub seq: Option<i64>,
    #[serde(rename = "temper-branch", skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(rename = "temper-pr", skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,
    // goal fields
    #[serde(rename = "temper-status", skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    // identity-tier transport (cascades to kb_resources on push)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
}

/// Lightweight payload for meta-only sync updates (no re-chunking).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct MetaUpdatePayload {
    pub resource_id: uuid::Uuid,
    pub managed_meta: serde_json::Value,
    pub open_meta: serde_json::Value,
    pub managed_hash: String,
    pub open_hash: String,
}

/// Server-side resource manifest row (maps to kb_resource_manifests).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ResourceManifestRow {
    pub resource_id: uuid::Uuid,
    pub body_hash: String,
    pub managed_meta: serde_json::Value,
    pub open_meta: serde_json::Value,
    pub managed_hash: String,
    pub open_hash: String,
    pub updated: chrono::DateTime<chrono::Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_meta_serde_roundtrip() {
        let meta = ManagedMeta {
            doc_type: Some("task".to_string()),
            stage: Some("backlog".to_string()),
            seq: Some(42),
            ..Default::default()
        };
        let json = serde_json::to_value(&meta).unwrap();
        assert_eq!(json["temper-type"], "task");
        assert_eq!(json["temper-stage"], "backlog");
        assert_eq!(json["temper-seq"], 42);
        // Fields with None should be absent
        assert!(json.get("temper-mode").is_none());

        let parsed: ManagedMeta = serde_json::from_value(json).unwrap();
        assert_eq!(parsed, meta);
    }

    #[test]
    fn managed_meta_yaml_roundtrip() {
        let meta = ManagedMeta {
            doc_type: Some("goal".to_string()),
            status: Some("active".to_string()),
            title: Some("My Goal".to_string()),
            slug: Some("my-goal".to_string()),
            ..Default::default()
        };
        let yaml = serde_yaml::to_string(&meta).unwrap();
        assert!(yaml.contains("temper-type: goal"));
        assert!(yaml.contains("temper-status: active"));
        assert!(yaml.contains("title: My Goal"));

        let parsed: ManagedMeta = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed, meta);
    }

    #[test]
    fn meta_update_payload_serde() {
        let payload = MetaUpdatePayload {
            resource_id: uuid::Uuid::nil(),
            managed_meta: serde_json::json!({"temper-stage": "done"}),
            open_meta: serde_json::json!({"tags": ["rust"]}),
            managed_hash: "sha256:abc".to_string(),
            open_hash: "sha256:def".to_string(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let parsed: MetaUpdatePayload = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.managed_hash, "sha256:abc");
    }
}
```

- [ ] **Step 2: Register in mod.rs**

In `crates/temper-core/src/types/mod.rs`, add:

```rust
pub mod managed_meta;
```

And in the re-export section:

```rust
pub use managed_meta::{ManagedMeta, MetaUpdatePayload, ResourceManifestRow};
```

- [ ] **Step 3: Run tests**

Run: `cargo nextest run --workspace managed_meta`
Expected: 3 tests pass

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/src/types/managed_meta.rs crates/temper-core/src/types/mod.rs
git commit -m "feat(core): add ManagedMeta, MetaUpdatePayload, ResourceManifestRow types"
```

---

## Task 2: Expand ManifestEntry to three-tier hashes with migration

**Files:**
- Modify: `crates/temper-core/src/types/manifest.rs`

- [ ] **Step 1: Write test for old-format manifest migration**

Add to `crates/temper-core/src/types/manifest.rs` tests:

```rust
#[test]
fn test_manifest_entry_migration_from_old_format() {
    // Old format JSON (content_hash / remote_hash)
    let old_json = serde_json::json!({
        "path": "temper/tasks/my-task.md",
        "content_hash": "abc123",
        "remote_hash": "abc123",
        "synced_at": "2026-04-04T00:00:00Z",
        "state": "clean",
        "mtime_secs": 1234567890
    });
    let entry: ManifestEntry = serde_json::from_value(old_json).unwrap();
    assert_eq!(entry.body_hash, "abc123");
    assert_eq!(entry.remote_body_hash, "abc123");
    assert_eq!(entry.managed_hash, "");
    assert_eq!(entry.open_hash, "");
    assert_eq!(entry.remote_managed_hash, "");
    assert_eq!(entry.remote_open_hash, "");
}

#[test]
fn test_manifest_entry_new_format_roundtrip() {
    let entry = ManifestEntry {
        path: "temper/tasks/my-task.md".to_string(),
        body_hash: "sha256:body".to_string(),
        managed_hash: "sha256:managed".to_string(),
        open_hash: "sha256:open".to_string(),
        remote_body_hash: "sha256:rbody".to_string(),
        remote_managed_hash: "sha256:rmanaged".to_string(),
        remote_open_hash: "sha256:ropen".to_string(),
        synced_at: Utc::now(),
        state: ManifestEntryState::Clean,
        mtime_secs: Some(123),
    };
    let json = serde_json::to_string(&entry).unwrap();
    let parsed: ManifestEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.body_hash, "sha256:body");
    assert_eq!(parsed.managed_hash, "sha256:managed");
    assert_eq!(parsed.remote_open_hash, "sha256:ropen");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run --workspace test_manifest_entry_migration`
Expected: FAIL — fields don't exist yet

- [ ] **Step 3: Update ManifestEntry struct**

Replace the `ManifestEntry` struct (lines 24-40) with:

```rust
/// A single resource entry in the local manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Relative path within the vault (e.g., "temper/tasks/r5-indexing.md")
    pub path: String,

    // -- Three-tier local hashes (computed from vault file) --

    /// SHA-256 hash of the markdown body (frontmatter stripped).
    /// Aliased from old `content_hash` for migration.
    #[serde(alias = "content_hash")]
    pub body_hash: String,
    /// SHA-256 hash of managed (temper-*) frontmatter fields.
    #[serde(default)]
    pub managed_hash: String,
    /// SHA-256 hash of open (user-owned) frontmatter fields.
    #[serde(default)]
    pub open_hash: String,

    // -- Three-tier remote hashes (from last sync) --

    /// Remote body hash. Aliased from old `remote_hash` for migration.
    #[serde(alias = "remote_hash")]
    pub remote_body_hash: String,
    /// Remote managed meta hash.
    #[serde(default)]
    pub remote_managed_hash: String,
    /// Remote open meta hash.
    #[serde(default)]
    pub remote_open_hash: String,

    // -- Sync state --

    /// When this entry was last synced with the server.
    pub synced_at: DateTime<Utc>,
    /// Current sync state.
    pub state: ManifestEntryState,
    /// File mtime (seconds since epoch) at last manifest update.
    #[serde(default)]
    pub mtime_secs: Option<i64>,
}
```

- [ ] **Step 4: Update existing tests to use new field names**

Update `test_manifest_json_roundtrip` (lines 98-119) — replace `content_hash`/`remote_hash` with `body_hash`/`remote_body_hash` and add the new hash fields:

```rust
#[test]
fn test_manifest_json_roundtrip() {
    let mut manifest = Manifest::new("device-abc".to_string());
    let resource_id = Uuid::nil();
    manifest.entries.insert(
        resource_id,
        ManifestEntry {
            path: "temper/tickets/r5.md".to_string(),
            body_hash: "sha256:abc123".to_string(),
            managed_hash: "sha256:managed".to_string(),
            open_hash: "sha256:open".to_string(),
            remote_body_hash: "sha256:abc123".to_string(),
            remote_managed_hash: "sha256:managed".to_string(),
            remote_open_hash: "sha256:open".to_string(),
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
    assert_eq!(entry.body_hash, "sha256:abc123");
    assert_eq!(entry.managed_hash, "sha256:managed");
}
```

- [ ] **Step 5: Run all manifest tests**

Run: `cargo nextest run --workspace test_manifest`
Expected: All pass (including migration test)

- [ ] **Step 6: Commit**

```bash
git add crates/temper-core/src/types/manifest.rs
git commit -m "feat(core): expand ManifestEntry to three-tier hashes with old-format migration"
```

---

## Task 3: Update IngestPayload with managed_meta and open_meta fields

**Files:**
- Modify: `crates/temper-core/src/types/ingest.rs`

- [ ] **Step 1: Add managed_meta and open_meta to IngestPayload**

Add two new optional fields after the existing `metadata` field (after line 26):

```rust
    /// Managed frontmatter (temper-* fields) as JSON.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_meta: Option<serde_json::Value>,
    /// Open frontmatter (user-owned fields) as JSON.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_meta: Option<serde_json::Value>,
```

- [ ] **Step 2: Update existing test that constructs IngestPayload**

In the `payload_serialization_roundtrip` test (line 124), add the new fields:

```rust
    metadata: None,
    managed_meta: Some(serde_json::json!({"temper-stage": "backlog"})),
    open_meta: Some(serde_json::json!({"tags": ["rust"]})),
    chunks_packed: pack_chunks(&sample_chunks()).unwrap(),
```

- [ ] **Step 3: Run tests**

Run: `cargo nextest run --workspace payload_serialization`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/src/types/ingest.rs
git commit -m "feat(core): add managed_meta and open_meta to IngestPayload"
```

---

## Task 4: Update ResourceRow — drop mimetype and resource_mode

**Files:**
- Modify: `crates/temper-core/src/types/resource.rs`

- [ ] **Step 1: Remove fields from ResourceRow**

Remove `mimetype` and `resource_mode` from the `ResourceRow` struct (lines 11-28). Also remove `content_hash` since it moves to `kb_resource_manifests`. The struct becomes:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ResourceRow {
    pub id: Uuid,
    pub kb_context_id: Uuid,
    pub kb_doc_type_id: Uuid,
    pub origin_uri: String,
    pub title: String,
    pub slug: Option<String>,
    pub originator_profile_id: Uuid,
    pub owner_profile_id: Uuid,
    pub is_active: bool,
    pub created: DateTime<Utc>,
    pub updated: DateTime<Utc>,
}
```

- [ ] **Step 2: Fix all compilation errors**

This will break:
- `crates/temper-api/src/services/ingest_service.rs` — SQL queries SELECT/INSERT these columns
- `crates/temper-api/src/services/resource_service.rs` — SQL queries reference these columns
- `crates/temper-api/src/services/sync_service.rs` — `content_hash` references
- `crates/temper-cli/src/actions/ingest.rs` — `resource.content_hash` references
- `crates/temper-cli/src/actions/sync.rs` — `resource.content_hash` references

**Do NOT fix these now** — they will be fixed in Tasks 5-8 alongside the SQL migration. For now, just update the struct. We'll fix compilation in a coordinated commit.

**Note to implementer:** Tasks 4-8 are a coordinated set. You'll need to complete all of them before the codebase compiles again. Run `cargo check` after Task 8 to verify.

- [ ] **Step 3: Commit (will not compile yet — that's expected)**

```bash
git add crates/temper-core/src/types/resource.rs
git commit -m "refactor(core): remove mimetype, resource_mode, content_hash from ResourceRow

These columns move to kb_resource_manifests. Compilation will break until
the migration and service updates are applied (Tasks 5-8)."
```

---

## Task 5: Database migration — kb_resource_manifests and kb_resources cleanup

**Files:**
- Create: `migrations/20260404000002_resource_manifests.sql`

- [ ] **Step 1: Write the migration**

```sql
-- =============================================================================
-- Resource Manifests — three-tier frontmatter persistence
-- =============================================================================
-- Creates kb_resource_manifests table and cleans up kb_resources.
-- Part of the managed/open frontmatter data model.

-- 1. Create kb_resource_manifests
CREATE TABLE kb_resource_manifests (
    resource_id    UUID PRIMARY KEY REFERENCES kb_resources(id) ON DELETE CASCADE,
    body_hash      VARCHAR(64) NOT NULL,
    managed_meta   JSONB NOT NULL DEFAULT '{}',
    open_meta      JSONB NOT NULL DEFAULT '{}',
    managed_hash   VARCHAR(64) NOT NULL,
    open_hash      VARCHAR(64) NOT NULL,
    updated        TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- 2. Populate from existing kb_resources
INSERT INTO kb_resource_manifests (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash)
SELECT id,
       COALESCE(content_hash, ''),
       '{}',
       '{}',
       'sha256:' || encode(sha256(convert_to('{}', 'UTF8')), 'hex'),
       'sha256:' || encode(sha256(convert_to('{}', 'UTF8')), 'hex')
  FROM kb_resources;

-- 3. Indexes on frequently-queried managed fields
CREATE INDEX idx_resource_manifests_stage
    ON kb_resource_manifests ((managed_meta->>'temper-stage'));
CREATE INDEX idx_resource_manifests_status
    ON kb_resource_manifests ((managed_meta->>'temper-status'));
CREATE INDEX idx_resource_manifests_goal
    ON kb_resource_manifests ((managed_meta->>'temper-goal'));

-- GIN index for open_meta array fields (tags, relates_to, etc.)
CREATE INDEX idx_resource_manifests_open_meta
    ON kb_resource_manifests USING GIN (open_meta jsonb_path_ops);

-- 4. Clean up kb_resources
ALTER TABLE kb_resources DROP COLUMN IF EXISTS mimetype;
ALTER TABLE kb_resources DROP COLUMN IF EXISTS resource_mode;
ALTER TABLE kb_resources DROP COLUMN IF EXISTS content_hash;
DROP INDEX IF EXISTS idx_kb_resources_mode;

-- Drop UNIQUE on origin_uri (keep column and regular index)
ALTER TABLE kb_resources DROP CONSTRAINT IF EXISTS kb_resources_origin_uri_key;

-- 5. Update sync_diff_for_device to use kb_resource_manifests for body_hash
CREATE OR REPLACE FUNCTION sync_diff_for_device(
    p_profile_id    UUID,
    p_context_names TEXT[],
    p_manifest      JSONB  -- [{"uri": "...", "body_hash": "...", "managed_hash": "...", "open_hash": "..."}]
) RETURNS TABLE (
    resource_id  UUID,
    kb_uri       TEXT,
    body_hash    VARCHAR(64),
    managed_hash VARCHAR(64),
    open_hash    VARCHAR(64),
    updated      TIMESTAMPTZ,
    diff_type    VARCHAR(32)
)
LANGUAGE SQL STABLE AS $$
    WITH
    visible AS (
        SELECT v.resource_id FROM resources_visible_to(p_profile_id) v
    ),
    manifest_entries AS (
        SELECT
            (entry->>'uri')::TEXT AS uri,
            (split_part(entry->>'uri', '/', 5))::UUID AS extracted_resource_id,
            COALESCE(entry->>'body_hash', entry->>'local_hash')::VARCHAR(64) AS local_body_hash,
            COALESCE(entry->>'managed_hash', '')::VARCHAR(64) AS local_managed_hash,
            COALESCE(entry->>'open_hash', '')::VARCHAR(64) AS local_open_hash
        FROM jsonb_array_elements(p_manifest) AS entry
    ),
    server_resources AS (
        SELECT r.id, kb_resource_uri(r.id) AS kb_uri,
               rm.body_hash, rm.managed_hash, rm.open_hash,
               r.updated, r.is_active
          FROM kb_resources r
          JOIN visible v ON v.resource_id = r.id
          JOIN kb_contexts c ON c.id = r.kb_context_id
          LEFT JOIN kb_resource_manifests rm ON rm.resource_id = r.id
         WHERE c.name = ANY(p_context_names)
    )
    -- Existing resources with manifest entries: compare three-tier hashes
    SELECT
        sr.id AS resource_id,
        sr.kb_uri,
        COALESCE(sr.body_hash, '') AS body_hash,
        COALESCE(sr.managed_hash, '') AS managed_hash,
        COALESCE(sr.open_hash, '') AS open_hash,
        sr.updated,
        CASE
            WHEN NOT sr.is_active THEN 'removed'::VARCHAR(32)
            -- Body changed locally, server unchanged
            WHEN me.local_body_hash != COALESCE(sr.body_hash, '') AND me.local_managed_hash = COALESCE(sr.managed_hash, '') THEN 'to_push_body'
            -- Meta changed locally, server unchanged
            WHEN me.local_body_hash = COALESCE(sr.body_hash, '') AND (me.local_managed_hash != COALESCE(sr.managed_hash, '') OR me.local_open_hash != COALESCE(sr.open_hash, '')) THEN 'to_push_meta'
            -- Both body and meta changed locally
            WHEN me.local_body_hash != COALESCE(sr.body_hash, '') THEN 'to_push_body'
            -- Nothing changed
            ELSE NULL
        END AS diff_type
      FROM server_resources sr
      JOIN manifest_entries me ON me.extracted_resource_id = sr.id
     WHERE sr.is_active = false
        OR me.local_body_hash != COALESCE(sr.body_hash, '')
        OR me.local_managed_hash != COALESCE(sr.managed_hash, '')
        OR me.local_open_hash != COALESCE(sr.open_hash, '')

    UNION ALL

    -- New remote resources (not in manifest)
    SELECT
        sr.id AS resource_id,
        sr.kb_uri,
        COALESCE(sr.body_hash, '') AS body_hash,
        COALESCE(sr.managed_hash, '') AS managed_hash,
        COALESCE(sr.open_hash, '') AS open_hash,
        sr.updated,
        'to_pull'::VARCHAR(32) AS diff_type
      FROM server_resources sr
      LEFT JOIN manifest_entries me ON me.extracted_resource_id = sr.id
     WHERE me.uri IS NULL
       AND sr.is_active = true

    UNION ALL

    -- New local resources (not on server)
    SELECT
        me.extracted_resource_id AS resource_id,
        me.uri AS kb_uri,
        me.local_body_hash AS body_hash,
        me.local_managed_hash AS managed_hash,
        me.local_open_hash AS open_hash,
        NULL::TIMESTAMPTZ AS updated,
        'to_push'::VARCHAR(32) AS diff_type
      FROM manifest_entries me
      LEFT JOIN server_resources sr ON sr.id = me.extracted_resource_id
     WHERE sr.id IS NULL
$$;

-- 6. Update resource_for_uri to not reference content_hash
CREATE OR REPLACE FUNCTION resource_for_uri(p_profile_id UUID, p_kb_uri TEXT)
RETURNS TABLE (
    resource_id  UUID,
    origin_uri   TEXT,
    body_hash    VARCHAR(64),
    updated      TIMESTAMPTZ,
    is_active    BOOLEAN,
    access_level VARCHAR(32),
    team_role    team_role
)
LANGUAGE SQL STABLE AS $$
    WITH parsed AS (
        SELECT (split_part(p_kb_uri, '/', 5))::UUID AS extracted_id
    )
    SELECT r.id AS resource_id,
           r.origin_uri,
           COALESCE(rm.body_hash, '') AS body_hash,
           r.updated,
           r.is_active,
           v.access_level,
           v.team_role
      FROM parsed p
      JOIN kb_resources r ON r.id = p.extracted_id
      LEFT JOIN kb_resource_manifests rm ON rm.resource_id = r.id
      JOIN resources_visible_to(p_profile_id, NULL, ARRAY[p.extracted_id]) v
        ON v.resource_id = r.id
$$;
```

- [ ] **Step 2: Commit**

```bash
git add migrations/20260404000002_resource_manifests.sql
git commit -m "feat(db): add kb_resource_manifests table, clean up kb_resources, update sync SQL functions"
```

---

## Task 6: Update temper-api ingest service for new schema

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs`

- [ ] **Step 1: Update ingest() function — remove dropped columns, add manifest insert**

Replace the `ingest()` function (lines 91-149) with:

```rust
pub async fn ingest(
    pool: &PgPool,
    profile_id: Uuid,
    payload: IngestPayload,
) -> ApiResult<ResourceRow> {
    // 1. Resolve context
    let context = context_service::resolve_by_name(pool, profile_id, &payload.context_name).await?;

    // 2. Resolve doc_type
    let doc_type_id = resolve_doc_type(pool, &payload.doc_type_name).await?;

    // 3. Content-hash dedup — check body_hash in kb_resource_manifests
    if let Some(existing) = find_by_body_hash(pool, profile_id, &payload.content_hash).await? {
        return Ok(existing);
    }

    // 4. Decode chunks
    let chunks = unpack_chunks(&payload.chunks_packed)
        .map_err(|e| ApiError::BadRequest(format!("invalid chunks_packed: {e}")))?;

    // 5. Insert resource + manifest + chunks in a transaction
    let mut tx = pool.begin().await?;

    let resource_id = Uuid::now_v7();
    let resource = sqlx::query_as::<_, ResourceRow>(
        r#"
        INSERT INTO kb_resources (
            id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
            originator_profile_id, owner_profile_id,
            created, updated
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $7, now(), now())
        RETURNING id, kb_context_id, kb_doc_type_id, origin_uri, title,
                  slug, originator_profile_id, owner_profile_id, is_active,
                  created, updated
        "#,
    )
    .bind(resource_id)
    .bind(context.id)
    .bind(doc_type_id)
    .bind(&payload.origin_uri)
    .bind(&payload.title)
    .bind(&payload.slug)
    .bind(profile_id)
    .fetch_one(&mut *tx)
    .await?;

    // 6. Insert manifest entry
    let managed_meta = payload.managed_meta.clone().unwrap_or(serde_json::json!({}));
    let open_meta = payload.open_meta.clone().unwrap_or(serde_json::json!({}));
    let managed_hash = hash_json_value(&managed_meta);
    let open_hash = hash_json_value(&open_meta);

    sqlx::query(
        r#"
        INSERT INTO kb_resource_manifests (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash)
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(resource_id)
    .bind(&payload.content_hash)
    .bind(&managed_meta)
    .bind(&open_meta)
    .bind(&managed_hash)
    .bind(&open_hash)
    .execute(&mut *tx)
    .await?;

    // 7. Insert chunks with embeddings
    insert_chunks(&mut tx, resource_id, &chunks).await?;

    // 8. Create kb_event
    insert_event(&mut tx, profile_id, &payload.slug, context.id, resource_id, "resource_created", &serde_json::json!({})).await?;

    tx.commit().await?;

    Ok(resource)
}
```

- [ ] **Step 2: Update find_by_content_hash → find_by_body_hash**

Replace `find_by_content_hash` (lines 27-52) with:

```rust
async fn find_by_body_hash(
    pool: &PgPool,
    profile_id: Uuid,
    body_hash: &str,
) -> ApiResult<Option<ResourceRow>> {
    let row = sqlx::query_as::<_, ResourceRow>(
        r#"
        WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
        SELECT r.id, r.kb_context_id, r.kb_doc_type_id, r.origin_uri, r.title,
               r.slug, r.originator_profile_id, r.owner_profile_id, r.is_active,
               r.created, r.updated
          FROM kb_resources r
          JOIN visible v ON v.resource_id = r.id
          JOIN kb_resource_manifests rm ON rm.resource_id = r.id
         WHERE rm.body_hash = $2
           AND r.is_active = true
         LIMIT 1
        "#,
    )
    .bind(profile_id)
    .bind(body_hash)
    .fetch_optional(pool)
    .await?;

    Ok(row)
}
```

- [ ] **Step 3: Update update() function — use manifests table for body_hash**

Replace the `update()` function (lines 152-235). The key changes:
- Update `kb_resource_manifests.body_hash` instead of `kb_resources.content_hash`
- Persist managed_meta and open_meta if provided
- Create kb_event

```rust
pub async fn update(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
    payload: IngestPayload,
) -> ApiResult<ResourceRow> {
    let can_modify: Option<(bool,)> =
        sqlx::query_as("SELECT true FROM can_modify_resource($1, $2)")
            .bind(profile_id)
            .bind(resource_id)
            .fetch_optional(pool)
            .await?;

    if can_modify.is_none() {
        return Err(ApiError::NotFound);
    }

    let chunks = unpack_chunks(&payload.chunks_packed)
        .map_err(|e| ApiError::BadRequest(format!("invalid chunks_packed: {e}")))?;

    let mut tx = pool.begin().await?;

    // Update resource timestamp
    let resource = sqlx::query_as::<_, ResourceRow>(
        r#"
        UPDATE kb_resources SET updated = now() WHERE id = $1
        RETURNING id, kb_context_id, kb_doc_type_id, origin_uri, title,
                  slug, originator_profile_id, owner_profile_id, is_active,
                  created, updated
        "#,
    )
    .bind(resource_id)
    .fetch_one(&mut *tx)
    .await?;

    // Update manifest
    let managed_meta = payload.managed_meta.clone().unwrap_or(serde_json::json!({}));
    let open_meta = payload.open_meta.clone().unwrap_or(serde_json::json!({}));
    let managed_hash = hash_json_value(&managed_meta);
    let open_hash = hash_json_value(&open_meta);

    sqlx::query(
        r#"
        INSERT INTO kb_resource_manifests (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
        VALUES ($1, $2, $3, $4, $5, $6, now())
        ON CONFLICT (resource_id) DO UPDATE SET
            body_hash = EXCLUDED.body_hash,
            managed_meta = EXCLUDED.managed_meta,
            open_meta = EXCLUDED.open_meta,
            managed_hash = EXCLUDED.managed_hash,
            open_hash = EXCLUDED.open_hash,
            updated = now()
        "#,
    )
    .bind(resource_id)
    .bind(&payload.content_hash)
    .bind(&managed_meta)
    .bind(&open_meta)
    .bind(&managed_hash)
    .bind(&open_hash)
    .execute(&mut *tx)
    .await?;

    // Version-bump old chunks
    sqlx::query("UPDATE kb_chunks SET is_current = false WHERE resource_id = $1 AND is_current = true")
        .bind(resource_id)
        .execute(&mut *tx)
        .await?;

    // Insert new chunks
    insert_chunks(&mut tx, resource_id, &chunks).await?;

    // Create kb_event
    insert_event(&mut tx, profile_id, "", resource.kb_context_id, resource_id, "body_updated", &serde_json::json!({})).await?;

    tx.commit().await?;

    Ok(resource)
}
```

- [ ] **Step 4: Add helper functions at the bottom of the file**

```rust
/// Compute SHA-256 hash of a canonical JSON value.
fn hash_json_value(value: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};
    let serialized = serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string());
    let result = Sha256::digest(serialized.as_bytes());
    format!("sha256:{}", hex::encode(result))
}

/// Insert a kb_events row.
async fn insert_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    profile_id: Uuid,
    device_id: &str,
    context_id: Uuid,
    resource_id: Uuid,
    event_type: &str,
    payload: &serde_json::Value,
) -> ApiResult<()> {
    sqlx::query(
        r#"
        INSERT INTO kb_events (id, profile_id, device_id, kb_context_id, resource_id, event_type, payload, created)
        VALUES ($1, $2, $3, $4, $5, $6, $7, now())
        "#,
    )
    .bind(Uuid::now_v7())
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

Add to imports at top of file:

```rust
use sha2::{Digest, Sha256};
```

And add `sha2` and `hex` to `temper-api/Cargo.toml` dependencies if not already present.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/services/ingest_service.rs
git commit -m "feat(api): update ingest service for kb_resource_manifests, kb_events, and new ResourceRow"
```

---

## Task 7: Update temper-api resource and sync services

**Files:**
- Modify: `crates/temper-api/src/services/resource_service.rs`
- Modify: `crates/temper-api/src/services/sync_service.rs`

- [ ] **Step 1: Update resource_service.rs SQL queries**

All `SELECT ... FROM kb_resources` queries that reference `content_hash`, `mimetype`, or `resource_mode` must be updated. The key changes:

In `list_visible()`: remove `content_hash`, `mimetype` from SELECT.
In `get_visible()`: remove `content_hash`, `mimetype` from SELECT.
In `get_content()`: unchanged (queries chunks, not resources).
In `create()`: remove `mimetype`, `resource_mode`, `content_hash` from INSERT.
In `update()`: remove `mimetype` from UPDATE SET.
In `delete()`: unchanged.

For each function, update the SQL column lists to match the new `ResourceRow` struct (no `content_hash`, `mimetype`, `resource_mode`).

- [ ] **Step 2: Update sync_service.rs**

Update `DiffRow` (lines 20-28) to match the new `sync_diff_for_device` return columns:

```rust
#[derive(Debug, sqlx::FromRow)]
struct DiffRow {
    resource_id: Option<Uuid>,
    kb_uri: String,
    body_hash: String,
    managed_hash: String,
    open_hash: String,
    #[expect(dead_code, reason = "returned by SQL but not used in categorization")]
    updated: Option<DateTime<Utc>>,
    diff_type: String,
}
```

Update `categorize_diff_rows()` to handle the new diff_type values (`to_push_body`, `to_push_meta`, `to_pull`, etc.). For now, map the new granular types back to the existing response categories:

```rust
fn categorize_diff_rows(rows: Vec<DiffRow>) -> SyncStatusResponse {
    let mut to_push = Vec::new();
    let mut to_pull = Vec::new();
    let mut conflicts = Vec::new();
    let mut removed = Vec::new();

    for row in rows {
        match row.diff_type.as_str() {
            "to_push" | "to_push_body" | "to_push_meta" => to_push.push(SyncPushItem {
                uri: row.kb_uri,
                resource_id: row.resource_id,
            }),
            "to_pull" | "to_pull_body" | "to_pull_meta" => to_pull.push(SyncPullItem {
                uri: row.kb_uri,
                resource_id: row.resource_id.expect("to_pull must have resource_id"),
                content_hash: row.body_hash,
            }),
            "conflict" | "conflict_body" | "conflict_meta" => conflicts.push(SyncConflictItem {
                uri: row.kb_uri,
                resource_id: row.resource_id.expect("conflict must have resource_id"),
                server_hash: row.body_hash,
            }),
            "removed" => removed.push(SyncRemovedItem {
                uri: row.kb_uri,
                resource_id: row.resource_id.expect("removed must have resource_id"),
            }),
            _ => {}
        }
    }

    SyncStatusResponse { to_push, to_pull, conflicts, removed }
}
```

Update `complete_sync_round()` (lines 111-168) — change the batch UPDATE to target `kb_resource_manifests.body_hash` instead of `kb_resources.content_hash`:

```rust
let result = sqlx::query(
    r#"
    UPDATE kb_resource_manifests rm
    SET body_hash = u.body_hash, updated = now()
    FROM unnest($1::uuid[], $2::text[]) AS u(resource_id, body_hash)
    WHERE rm.resource_id = u.resource_id
    "#,
)
.bind(&ids)
.bind(&hashes)
.execute(&mut *tx)
.await?;
```

Update `fetch_manifest()` (lines 182-221) — join `kb_resource_manifests` for `body_hash`:

```rust
let rows = sqlx::query_as::<_, ManifestRow>(
    r#"
    SELECT r.id AS resource_id,
           c.name AS context_name,
           d.name AS doc_type_name,
           COALESCE(r.slug, '') AS slug,
           COALESCE(rm.body_hash, '') AS content_hash
      FROM kb_resources r
      JOIN kb_contexts c ON c.id = r.kb_context_id
      JOIN kb_doc_types d ON d.id = r.kb_doc_type_id
      LEFT JOIN kb_resource_manifests rm ON rm.resource_id = r.id
     WHERE r.owner_profile_id = $1
       AND r.is_active = true
     ORDER BY c.name, d.name, r.slug
    "#,
)
```

- [ ] **Step 3: Add new meta update endpoint**

Create `crates/temper-api/src/handlers/meta.rs`:

```rust
use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::error::ApiResult;
use crate::middleware::auth::AuthUser;
use crate::state::AppState;
use temper_core::types::ManagedMeta;
use temper_core::types::managed_meta::MetaUpdatePayload;

pub async fn update_meta(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
    Json(payload): Json<MetaUpdatePayload>,
) -> ApiResult<Json<serde_json::Value>> {
    crate::services::meta_service::update_meta(
        &state.pool,
        auth.0.profile.id,
        resource_id,
        payload,
    )
    .await?;
    Ok(Json(serde_json::json!({"ok": true})))
}
```

Create `crates/temper-api/src/services/meta_service.rs`:

```rust
use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use temper_core::types::managed_meta::MetaUpdatePayload;

pub async fn update_meta(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
    payload: MetaUpdatePayload,
) -> ApiResult<()> {
    // Verify access
    let can_modify: Option<(bool,)> =
        sqlx::query_as("SELECT true FROM can_modify_resource($1, $2)")
            .bind(profile_id)
            .bind(resource_id)
            .fetch_optional(pool)
            .await?;

    if can_modify.is_none() {
        return Err(ApiError::NotFound);
    }

    let mut tx = pool.begin().await?;

    // Upsert manifest meta
    sqlx::query(
        r#"
        UPDATE kb_resource_manifests
        SET managed_meta = $1, open_meta = $2, managed_hash = $3, open_hash = $4, updated = now()
        WHERE resource_id = $5
        "#,
    )
    .bind(&payload.managed_meta)
    .bind(&payload.open_meta)
    .bind(&payload.managed_hash)
    .bind(&payload.open_hash)
    .bind(resource_id)
    .execute(&mut *tx)
    .await?;

    // Cascade identity fields from managed_meta to kb_resources
    cascade_identity_fields(&mut tx, resource_id, &payload.managed_meta).await?;

    // Insert event
    let context_id: Option<(Uuid,)> =
        sqlx::query_as("SELECT kb_context_id FROM kb_resources WHERE id = $1")
            .bind(resource_id)
            .fetch_optional(&mut *tx)
            .await?;

    if let Some((ctx_id,)) = context_id {
        crate::services::ingest_service::insert_event(
            &mut tx, profile_id, "", ctx_id, resource_id,
            "managed_meta_updated", &serde_json::json!({}),
        ).await?;
    }

    tx.commit().await?;
    Ok(())
}

/// Cascade identity-tier fields from managed_meta to kb_resources relational columns.
async fn cascade_identity_fields(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    resource_id: Uuid,
    managed_meta: &serde_json::Value,
) -> ApiResult<()> {
    // Cascade title
    if let Some(title) = managed_meta.get("title").and_then(|v| v.as_str()) {
        sqlx::query("UPDATE kb_resources SET title = $1, updated = now() WHERE id = $2")
            .bind(title)
            .bind(resource_id)
            .execute(&mut **tx)
            .await?;
    }
    // Cascade slug
    if let Some(slug) = managed_meta.get("slug").and_then(|v| v.as_str()) {
        sqlx::query("UPDATE kb_resources SET slug = $1, updated = now() WHERE id = $2")
            .bind(slug)
            .bind(resource_id)
            .execute(&mut **tx)
            .await?;
    }
    // Cascade temper-type → kb_doc_type_id
    if let Some(doc_type) = managed_meta.get("temper-type").and_then(|v| v.as_str()) {
        sqlx::query(
            r#"UPDATE kb_resources SET kb_doc_type_id = (SELECT id FROM kb_doc_types WHERE name = $1), updated = now() WHERE id = $2"#,
        )
        .bind(doc_type)
        .bind(resource_id)
        .execute(&mut **tx)
        .await?;
    }
    // Cascade temper-context → kb_context_id
    if let Some(context) = managed_meta.get("temper-context").and_then(|v| v.as_str()) {
        // Look up context by name for the resource's current owner
        let row: Option<(Uuid,)> = sqlx::query_as(
            r#"
            SELECT c.id FROM kb_contexts c
            JOIN kb_resources r ON r.owner_profile_id = c.kb_owner_id AND c.kb_owner_table = 'kb_profiles'
            WHERE c.name = $1 AND r.id = $2
            "#,
        )
        .bind(context)
        .bind(resource_id)
        .fetch_optional(&mut **tx)
        .await?;

        if let Some((ctx_id,)) = row {
            sqlx::query("UPDATE kb_resources SET kb_context_id = $1, updated = now() WHERE id = $2")
                .bind(ctx_id)
                .bind(resource_id)
                .execute(&mut **tx)
                .await?;
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Register the new handler and service**

In `crates/temper-api/src/handlers/mod.rs`, add `pub mod meta;`
In `crates/temper-api/src/services/mod.rs`, add `pub mod meta_service;`

In `crates/temper-api/src/routes.rs`, add the route (after line 49):

```rust
.route("/api/resources/{id}/meta", put(handlers::meta::update_meta))
```

Make `insert_event` in `ingest_service.rs` public (`pub async fn insert_event`).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/
git commit -m "feat(api): update resource/sync services for manifests table, add meta update endpoint"
```

---

## Task 8: Update temper-cli templates to emit temper-* field names

**Files:**
- Modify: `crates/temper-cli/templates/task.md`
- Modify: `crates/temper-cli/templates/goal.md`
- Modify: `crates/temper-cli/templates/session.md`
- Modify: `crates/temper-cli/templates/research.md`

- [ ] **Step 1: Update task.md**

```
---
temper-id: "{{ id }}"
temper-type: task
temper-context: "{{ context }}"
temper-created: {{ datetime }}
temper-updated: {{ datetime }}
title: "{{ title }}"
slug: "{{ slug }}"
temper-goal: "{{ goal }}"
temper-stage: backlog
temper-mode: {{ mode }}
temper-effort: {{ effort }}
temper-seq: {{ seq }}
temper-branch: null
temper-pr: null
---

# {{ title }}
```

- [ ] **Step 2: Update goal.md**

```
---
temper-id: "{{ id }}"
temper-type: goal
temper-context: "{{ context }}"
temper-created: {{ date }}
title: "{{ title }}"
slug: "{{ slug }}"
temper-seq: {{ seq }}
temper-status: active
---

# {{ title }}
```

- [ ] **Step 3: Update session.md**

```
---
temper-id: "{{ id }}"
temper-type: session
temper-context: ""
temper-created: {{ date }}
date: {{ date }}
---

# Session: {{ title }}

## Goal

What this session set out to accomplish.

## What happened

What was attempted, what worked, what didn't.

## Decisions

Significant choices made and why (alternatives considered).

## What connected

Concepts, patterns, or cross-project links noticed.

## To pick up

Next steps, open threads, things to investigate.
```

Note: session template needs a `context` variable added to its Askama struct — check `crates/temper-cli/src/templates.rs` for the struct and add context if missing.

- [ ] **Step 4: Update research.md**

```
---
temper-id: "{{ id }}"
temper-type: research
temper-context: "{{ project }}"
temper-created: {{ date }}
title: "{{ title }}"
slug: "{{ slug }}"
date: {{ date }}
---

# {{ title }}

## Topic

What question or area is being investigated.

## Findings

Key discoveries, data points, and conclusions.

## Sources

References, links, documentation consulted.

## Implications

How this affects current or planned work.

## Open Questions

What remains unknown or needs further investigation.
```

- [ ] **Step 5: Run tests to check template rendering**

Run: `cargo nextest run --workspace -E 'test(/template|skill/)' `
Expected: Tests may need updating if they assert on frontmatter content

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/templates/
git commit -m "feat(cli): update all Askama templates to emit temper-* prefixed field names"
```

---

## Task 9: Update CLI action types and frontmatter operations

**Files:**
- Modify: `crates/temper-cli/src/actions/types.rs`
- Modify: `crates/temper-cli/src/actions/task.rs`
- Modify: `crates/temper-cli/src/actions/goal.rs`
- Modify: `crates/temper-cli/src/actions/ingest.rs`

- [ ] **Step 1: Update TaskInfo with serde aliases**

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TaskInfo {
    pub title: String,
    pub slug: String,
    #[serde(alias = "context", alias = "temper-context")]
    pub context: String,
    #[serde(alias = "goal", alias = "temper-goal")]
    pub goal: String,
    #[serde(alias = "stage", alias = "temper-stage")]
    pub stage: String,
    #[serde(alias = "mode", alias = "temper-mode")]
    pub mode: Option<String>,
    #[serde(alias = "effort", alias = "temper-effort")]
    pub effort: Option<String>,
    #[serde(alias = "seq", alias = "temper-seq")]
    pub seq: u32,
    #[serde(alias = "branch", alias = "temper-branch")]
    pub branch: Option<String>,
    #[serde(alias = "pr", alias = "temper-pr")]
    pub pr: Option<String>,
}
```

- [ ] **Step 2: Update GoalInfo with serde aliases**

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GoalInfo {
    pub title: String,
    pub slug: String,
    #[serde(alias = "context", alias = "temper-context")]
    pub context: String,
    #[serde(alias = "seq", alias = "temper-seq")]
    pub seq: u32,
    #[serde(alias = "status", alias = "temper-status")]
    pub status: String,
}
```

- [ ] **Step 3: Update set_frontmatter_field calls in task.rs**

In `move_task()` (lines 194-290):
- Line 229: `"stage"` → `"temper-stage"`
- Line 246: `"goal"` → `"temper-goal"`
- Line 249: `"seq"` → `"temper-seq"`
- Line 253: `"mode"` → `"temper-mode"`
- Line 257: `"effort"` → `"temper-effort"`
- Line 261: `"updated"` → `"temper-updated"`

In `done()` (lines 293-331):
- Line 309: `"stage"` → `"temper-stage"`
- Line 310: `"updated"` → `"temper-updated"`
- Line 312: `"branch"` → `"temper-branch"`
- Line 315: `"pr"` → `"temper-pr"`

- [ ] **Step 4: Update set_frontmatter_field calls in goal.rs**

In `update()` (line 164): `"status"` → `"temper-status"`

- [ ] **Step 5: Update parse_source_frontmatter in ingest.rs**

Replace the function (lines 76-95) to check temper-* names first:

```rust
pub fn parse_source_frontmatter(content: &str) -> Option<ParsedFrontmatter> {
    let yaml = crate::vault::parse_frontmatter(content)?;

    let s = |key: &str| yaml.get(key).and_then(|v| v.as_str()).map(String::from);

    Some(ParsedFrontmatter {
        title: s("title"),
        doc_type: s("temper-type").or_else(|| s("doc_type")).or_else(|| s("type")),
        context: s("temper-context").or_else(|| s("context")).or_else(|| s("project")),
        slug: s("slug"),
        date: s("date").or_else(|| s("temper-created").map(|c| c[..10].to_string())).or_else(|| s("created").map(|c| c[..10].to_string())),
        legacy_id: s("temper-id").or_else(|| s("id")),
        goal: s("temper-goal").or_else(|| s("goal")),
        stage: s("temper-stage").or_else(|| s("stage")),
        mode: s("temper-mode").or_else(|| s("mode")),
        effort: s("temper-effort").or_else(|| s("effort")),
        status: s("temper-status").or_else(|| s("status")),
    })
}
```

- [ ] **Step 6: Update build_frontmatter in ingest.rs**

Replace the function (lines 431-453) to emit temper-* field names:

```rust
pub fn build_frontmatter(
    id: Uuid,
    title: &str,
    context: &str,
    doc_type: &str,
    ingestion_source: Option<&str>,
    extra_fields: Option<&[(&str, &str)]>,
) -> String {
    let now = chrono::Utc::now().to_rfc3339();
    let mut fm = format!(
        "---\ntemper-id: {id}\ntemper-type: {doc_type}\ntemper-context: {context}\ntemper-created: {now}\ntitle: \"{title}\"\n"
    );
    if let Some(source) = ingestion_source {
        fm.push_str(&format!("temper-source: \"{source}\"\n"));
    }
    if let Some(fields) = extra_fields {
        for (key, value) in fields {
            fm.push_str(&format!("{key}: \"{value}\"\n"));
        }
    }
    fm.push_str("---\n\n");
    fm
}
```

- [ ] **Step 7: Standardize compute_content_hash to sha256: prefix**

In `crates/temper-cli/src/actions/ingest.rs`, update `compute_content_hash()` (lines 20-28) to include the `sha256:` prefix for consistency with `compute_frontmatter_hashes`:

```rust
pub fn compute_content_hash(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let result = hasher.finalize();
    format!("sha256:{}", hex::encode(result))
}
```

Note: This changes the hash format from plain hex to `sha256:`-prefixed. All comparisons (manifest, sync, dedup) must use the same format. Since we're rebuilding the manifest anyway (Task 2 migration), this is safe.

- [ ] **Step 8: Run tests**

Run: `cargo nextest run --workspace -E 'test(/task|goal|ingest|types/)'`
Expected: Tests may need updating for new field names in assertions and new hash prefix

- [ ] **Step 9: Commit**

```bash
git add crates/temper-cli/src/actions/
git commit -m "feat(cli): update action types, frontmatter operations, and ingest to use temper-* field names"
```

---

## Task 10: Update CLI sync push/pull for three-tier hashes

**Files:**
- Modify: `crates/temper-cli/src/actions/sync.rs`
- Modify: `crates/temper-cli/src/actions/ingest.rs` (write_vault_file_and_register)

- [ ] **Step 1: Update rehash_manifest to compute three-tier hashes**

In `rehash_manifest()` (lines 43-77), after computing the body hash from stripped content, also compute managed and open hashes from the frontmatter:

```rust
// After computing body_hash from stripped content:
let (managed_hash, open_hash) = if let Some(fm) = crate::vault::parse_frontmatter(&content) {
    temper_core::schema::compute_frontmatter_hashes(&fm)
} else {
    (temper_core::schema::hash_empty(), temper_core::schema::hash_empty())
};

entry.body_hash = body_hash;
entry.managed_hash = managed_hash;
entry.open_hash = open_hash;
```

The empty hash is: `format!("sha256:{}", hex::encode(sha2::Sha256::digest(b"{}")))`. Compute it inline or add a `pub fn hash_empty() -> String` to `temper_core::schema`.

- [ ] **Step 2: Update push_resource to send managed_meta and open_meta**

In `push_resource()` (lines 470-542), after reading the file and stripping frontmatter, parse the frontmatter into managed and open tiers:

```rust
let fm = crate::vault::parse_frontmatter(&content);
let (managed_meta, open_meta) = if let Some(ref fm) = fm {
    let (managed, open) = split_frontmatter_tiers(fm);
    (Some(managed), Some(open))
} else {
    (None, None)
};
```

Add these to the `IngestPayload` construction. Add a helper function:

```rust
fn split_frontmatter_tiers(fm: &serde_yaml::Value) -> (serde_json::Value, serde_json::Value) {
    let Some(mapping) = fm.as_mapping() else {
        return (serde_json::json!({}), serde_json::json!({}));
    };
    let mut managed = serde_json::Map::new();
    let mut open = serde_json::Map::new();
    for (key, value) in mapping {
        let Some(key_str) = key.as_str() else { continue };
        let json_value = serde_json::to_value(value).unwrap_or(serde_json::Value::Null);
        if key_str.starts_with("temper-") || key_str == "title" || key_str == "slug" {
            managed.insert(key_str.to_string(), json_value);
        } else {
            open.insert(key_str.to_string(), json_value);
        }
    }
    (serde_json::Value::Object(managed), serde_json::Value::Object(open))
}
```

- [ ] **Step 3: Update pull_resource to write managed_meta and open_meta to frontmatter**

In `pull_resource()` (lines 544-636), after fetching resource and content, if the API response includes managed_meta and open_meta, merge them into the frontmatter.

- [ ] **Step 4: Update write_vault_file_and_register manifest fields**

In `ingest.rs` `write_vault_file_and_register()` (lines 465-525), update the `ManifestEntry` construction to use the new field names:

```rust
manifest.entries.insert(
    resource.id,
    temper_core::types::ManifestEntry {
        path: rel_path,
        body_hash: content_hash,
        managed_hash: String::new(), // computed on next rehash
        open_hash: String::new(),
        remote_body_hash: String::new(), // will be set after sync
        remote_managed_hash: String::new(),
        remote_open_hash: String::new(),
        synced_at: chrono::Utc::now(),
        state: temper_core::types::ManifestEntryState::Clean,
        mtime_secs,
    },
);
```

- [ ] **Step 5: Run sync tests**

Run: `cargo nextest run --workspace -E 'test(/sync/)'`
Expected: Tests pass after adjustments

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs crates/temper-cli/src/actions/ingest.rs
git commit -m "feat(cli): update sync push/pull and manifest for three-tier hashes"
```

---

## Task 11: Fix compilation and run full test suite

**Files:**
- Various — fix any remaining compilation errors across the workspace

- [ ] **Step 1: Run cargo check across the workspace**

Run: `cargo check --workspace`
Expected: Identify remaining compilation errors

- [ ] **Step 2: Fix each error**

Common fixes needed:
- Any remaining references to `resource.content_hash`, `resource.mimetype`, `resource.resource_mode` in tests or other code
- Missing imports for new types
- Test assertions that reference old field names
- The doctor tests that use old/new format frontmatter may need updates

- [ ] **Step 3: Run full quality checks**

Run: `cargo make check`
Expected: fmt, clippy, docs, machete, typecheck, biome all pass

- [ ] **Step 4: Run full test suite**

Run: `cargo make test`
Expected: All unit tests pass

- [ ] **Step 5: Commit any remaining fixes**

```bash
git add -A
git commit -m "fix: resolve compilation errors and test failures from three-tier frontmatter migration"
```

---

## Task 12: Update doctor tests and verify doctor fix compatibility

**Files:**
- Modify: `crates/temper-cli/tests/doctor_test.rs`

- [ ] **Step 1: Update test constants to verify both formats**

The doctor tests already have `VALID_TASK_FM` (temper-* format) and `LEGACY_TASK_FM` (old format). Verify that:
- `temper doctor` still detects legacy fields
- `temper doctor fix` still renames them correctly
- The new template-generated frontmatter passes validation

- [ ] **Step 2: Run doctor tests**

Run: `cargo nextest run --workspace -E 'test(/doctor/)'`
Expected: All doctor tests pass

- [ ] **Step 3: Commit if any changes needed**

```bash
git add crates/temper-cli/tests/
git commit -m "test: update doctor tests for three-tier frontmatter model"
```

---

## Task 13: Run integration tests and verify end-to-end

- [ ] **Step 1: Run full quality checks**

Run: `cargo make check`
Expected: All pass

- [ ] **Step 2: Run full unit test suite**

Run: `cargo make test`
Expected: All pass

- [ ] **Step 3: Run integration tests if Docker is available**

Run: `cargo make docker-up && cargo make test-db`
Expected: Integration tests pass (or identify DB-level failures from migration)

- [ ] **Step 4: Final commit with clean state**

Run: `cargo make check && cargo make test`
If all green, no additional commit needed.
