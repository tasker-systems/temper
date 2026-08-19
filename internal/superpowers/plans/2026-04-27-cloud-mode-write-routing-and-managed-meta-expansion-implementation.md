# Cloud-Mode Write Routing & Managed-Meta Expansion — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land cloud-mode `create` and `update` write paths against a partial-update PATCH endpoint, plus the residual sync redirect / e2e / docs cleanup deferred from the 2026-04-22 cloud-first reframe.

**Architecture:** `match VaultState` at each command's divergence site (mirrors what Session A did for `show`). Server-side `ResourceUpdateRequest` grows from `{title, slug}` to a partial-update shape carrying `managed_meta`, `open_meta`, and an all-or-nothing body trio (`content` + `content_hash` + `chunks_packed`). CLI helpers (`build_ingest_payload`, body-trio extraction, doctype frontmatter construction) consolidate to a single source of truth so cloud and local share the same primitives.

**Tech Stack:** Rust workspace (axum + sqlx + utoipa for the API; clap + reqwest for the CLI). PostgreSQL 18 + pgvector. ts-rs for TypeScript codegen. cargo-nextest for tests. Pre-existing chunk-dedupe primitive from the ingest pipeline.

**Spec:** [`docs/superpowers/specs/2026-04-27-cloud-mode-write-routing-and-managed-meta-expansion-design.md`](../specs/2026-04-27-cloud-mode-write-routing-and-managed-meta-expansion-design.md)

---

## File Map

**Created:**
- `crates/temper-cli/src/actions/frontmatter.rs` — typed `build_managed_meta_for_create` builder shared by local-mode templated creators and cloud-mode ingest path.
- `tests/e2e/tests/cloud_writes_test.rs` — end-to-end coverage for cloud write paths.

**Modified — server-side:**
- `crates/temper-core/src/types/resource.rs` — `ResourceUpdateRequest` field expansion.
- `crates/temper-api/src/handlers/resources.rs` — body-trio all-or-nothing validation (400 on partial trio).
- `crates/temper-api/src/services/resource_service.rs` — `update()` fn grown for partial managed_meta + open_meta merge, body-trio path with chunk-dedupe short-circuit.
- `.sqlx/` — regenerated cache for new SQL.

**Modified — CLI helpers:**
- `crates/temper-cli/src/actions/ingest.rs` — `compute_body_chunks` extraction; `build_ingest_payload` signature growth.
- `crates/temper-cli/src/actions/sync.rs` — pass `None, None` to grown signature at call sites 1187, 1766.
- `crates/temper-cli/src/commands/add.rs` — pass `None, None` to grown signature at call sites 236, 886.

**Modified — per-doctype creators (frontmatter unification):**
- `crates/temper-cli/src/actions/{session,task,goal,research,concept,decision}.rs` (or wherever per-doctype create logic lives — verified during Task 10).

**Modified — CLI commands:**
- `crates/temper-cli/src/commands/resource.rs` — VaultState match in `create` (line 42) and `update` (located during Task 13).
- `crates/temper-cli/src/commands/sync.rs` — VaultState guard in `run`.

**Modified — docs:**
- `CLAUDE.md` — "Cloud mode operations" subsection.
- `/Users/petetaylor/.claude/skills/temper/reference.md` — cloud-mode command documentation.
- `/Users/petetaylor/.claude/skills/temper/subagent-guidance.md` — cloud-mode write principle.

**Deleted:**
- `docs/superpowers/plans/2026-04-19-unit-b-2-cloud-mode-dispatch.md` — superseded plan.

---

## Task 1: Expand `ResourceUpdateRequest` struct

**Files:**
- Modify: `crates/temper-core/src/types/resource.rs:126-133`
- Test: `crates/temper-core/src/types/resource.rs` (inline `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing test**

In `crates/temper-core/src/types/resource.rs`, append to the existing tests module (or create one if absent):

```rust
#[test]
fn resource_update_request_serde_round_trips_with_all_fields() {
    use serde_json::json;
    let req = ResourceUpdateRequest {
        title: Some("New Title".to_string()),
        slug: Some("new-slug".to_string()),
        managed_meta: Some(ManagedMeta {
            stage: Some("done".to_string()),
            ..Default::default()
        }),
        open_meta: Some(json!({"tags": ["rust"]})),
        content: Some("# Body\n".to_string()),
        content_hash: Some("sha256:abc".to_string()),
        chunks_packed: Some("base64-blob".to_string()),
    };
    let serialized = serde_json::to_string(&req).unwrap();
    let parsed: ResourceUpdateRequest = serde_json::from_str(&serialized).unwrap();
    assert_eq!(parsed.title.as_deref(), Some("New Title"));
    assert_eq!(parsed.managed_meta.as_ref().and_then(|m| m.stage.as_deref()), Some("done"));
    assert_eq!(parsed.content.as_deref(), Some("# Body\n"));
    assert_eq!(parsed.content_hash.as_deref(), Some("sha256:abc"));
    assert_eq!(parsed.chunks_packed.as_deref(), Some("base64-blob"));
}

#[test]
fn resource_update_request_omits_none_fields_on_serialize() {
    let req = ResourceUpdateRequest {
        title: None,
        slug: None,
        managed_meta: Some(ManagedMeta {
            stage: Some("done".to_string()),
            ..Default::default()
        }),
        open_meta: None,
        content: None,
        content_hash: None,
        chunks_packed: None,
    };
    let serialized = serde_json::to_string(&req).unwrap();
    assert!(!serialized.contains("\"title\""));
    assert!(!serialized.contains("\"content\""));
    assert!(serialized.contains("\"managed_meta\""));
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo nextest run -p temper-core resource_update_request_serde_round_trips_with_all_fields
```

Expected: FAIL with `error[E0560]: struct ResourceUpdateRequest has no field named managed_meta` (or similar — fields don't exist yet).

- [ ] **Step 3: Expand the struct**

In `crates/temper-core/src/types/resource.rs`, replace the existing `ResourceUpdateRequest` (currently `{title, slug}`) with:

```rust
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "resource.ts"))]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceUpdateRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slug: Option<String>,
    /// Partial managed_meta — only fields with `Some` apply.
    /// Untouched fields preserve their stored value. There is no in-band
    /// signal for "clear this field"; field-clearing is reserved for a
    /// future PUT endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_meta: Option<ManagedMeta>,
    /// Partial open_meta — incoming keys win; absent keys preserved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_meta: Option<serde_json::Value>,
    /// New body markdown. Required iff `content_hash` and `chunks_packed`
    /// are also `Some` (all-or-nothing trio).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// SHA-256 hash of `content`. Required iff `content` is `Some`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    /// Pre-computed chunks (base64-encoded MessagePack). Required iff
    /// `content` is `Some`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunks_packed: Option<String>,
}
```

Add the import for `ManagedMeta` near the top of the file if not already present:
```rust
use super::managed_meta::ManagedMeta;
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo nextest run -p temper-core resource_update_request
```

Expected: both tests PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/resource.rs
git commit -m "feat(core): expand ResourceUpdateRequest with managed_meta, open_meta, body trio"
```

---

## Task 2: Body-trio all-or-nothing handler validation

**Files:**
- Modify: `crates/temper-api/src/handlers/resources.rs:140-149`
- Test: `crates/temper-api/tests/resource_update_validation_test.rs` (new)

- [ ] **Step 1: Write the failing test**

Create `crates/temper-api/tests/resource_update_validation_test.rs`:

```rust
//! Body-trio validation: content + content_hash + chunks_packed are all-or-nothing.
#![cfg(feature = "test-db")]

use temper_core::types::ResourceUpdateRequest;
// ... harness imports modeled on existing handler integration tests in this crate

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn patch_returns_400_when_content_present_without_hash(pool: sqlx::PgPool) {
    // Arrange: existing resource owned by a test profile
    let (state, profile, resource_id) = setup_resource_with_profile(&pool).await;

    // Act: send PATCH with content but no content_hash
    let req = ResourceUpdateRequest {
        content: Some("new body".to_string()),
        content_hash: None,
        chunks_packed: Some("blob".to_string()),
        ..Default::default()
    };
    let response = patch_resource(&state, &profile, resource_id, &req).await;

    // Assert
    assert_eq!(response.status(), 400);
    let body: serde_json::Value = serde_json::from_slice(&response.into_body_bytes().await).unwrap();
    assert!(body["error"].as_str().unwrap().contains("content_hash"));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn patch_accepts_empty_body_trio(pool: sqlx::PgPool) {
    let (state, profile, resource_id) = setup_resource_with_profile(&pool).await;
    let req = ResourceUpdateRequest {
        managed_meta: Some(temper_core::types::ManagedMeta {
            stage: Some("done".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let response = patch_resource(&state, &profile, resource_id, &req).await;
    assert_eq!(response.status(), 200);
}
```

The `setup_resource_with_profile` and `patch_resource` helpers should follow the pattern of existing integration tests in `crates/temper-api/tests/`. If no resource-update integration test exists yet, model on the closest neighbor (e.g., `crates/temper-api/tests/resources_handler_test.rs` or whatever exists).

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo nextest run -p temper-api --features test-db patch_returns_400_when_content_present_without_hash
```

Expected: FAIL — test should fail because the handler doesn't yet validate the trio (will likely 200 or 500).

- [ ] **Step 3: Add the validation in the handler**

In `crates/temper-api/src/handlers/resources.rs`, modify the `update` handler (around line 140):

```rust
pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
    Json(req): Json<ResourceUpdateRequest>,
) -> ApiResult<Json<ResourceRow>> {
    // Body trio is all-or-nothing.
    let body_fields_present = [
        req.content.is_some(),
        req.content_hash.is_some(),
        req.chunks_packed.is_some(),
    ];
    if body_fields_present.iter().any(|&p| p) && !body_fields_present.iter().all(|&p| p) {
        return Err(ApiError::BadRequest(
            "content, content_hash, and chunks_packed must all be present together or all be absent".to_string(),
        ));
    }

    resource_service::update(&state.pool, auth.0.profile.id, resource_id, req)
        .await
        .map(Json)
}
```

If `ApiError::BadRequest` doesn't exist with a `String` arg, use the closest matching constructor in `crates/temper-api/src/error.rs`.

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo nextest run -p temper-api --features test-db patch_returns_400_when_content_present_without_hash patch_accepts_empty_body_trio
```

Expected: both PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/handlers/resources.rs crates/temper-api/tests/resource_update_validation_test.rs
git commit -m "feat(api): validate body trio all-or-nothing on PATCH /api/resources/{id}"
```

---

## Task 3: Service `update()` — managed_meta + open_meta partial merge

**Files:**
- Modify: `crates/temper-api/src/services/resource_service.rs:501-542`
- Test: `crates/temper-api/tests/resource_update_merge_test.rs` (new)

- [ ] **Step 1: Write the failing test**

Create `crates/temper-api/tests/resource_update_merge_test.rs`:

```rust
//! Partial managed_meta + open_meta merge semantics.
#![cfg(feature = "test-db")]

use temper_core::types::{ManagedMeta, ResourceUpdateRequest};
// ... harness imports

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn managed_meta_partial_update_preserves_untouched_fields(pool: sqlx::PgPool) {
    // Arrange: resource with stored managed_meta { stage: "in-progress", mode: "build", goal: "g1" }
    let (state, profile, resource_id) = setup_resource_with_managed_meta(
        &pool,
        ManagedMeta {
            stage: Some("in-progress".to_string()),
            mode: Some("build".to_string()),
            goal: Some("g1".to_string()),
            ..Default::default()
        },
    ).await;

    // Act: PATCH only stage
    let req = ResourceUpdateRequest {
        managed_meta: Some(ManagedMeta {
            stage: Some("done".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let updated = call_update_service(&state, profile, resource_id, req).await.unwrap();

    // Assert: stage updated, mode + goal preserved
    let stored = fetch_managed_meta(&state.pool, resource_id).await;
    assert_eq!(stored.stage.as_deref(), Some("done"));
    assert_eq!(stored.mode.as_deref(), Some("build"));
    assert_eq!(stored.goal.as_deref(), Some("g1"));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn managed_meta_extra_bucket_merges_by_key(pool: sqlx::PgPool) {
    let stored = ManagedMeta {
        extra: [("date".to_string(), serde_json::json!("2026-04-13"))].into(),
        ..Default::default()
    };
    let (state, profile, resource_id) = setup_resource_with_managed_meta(&pool, stored).await;

    let incoming = ManagedMeta {
        extra: [("custom".to_string(), serde_json::json!("value"))].into(),
        ..Default::default()
    };
    let req = ResourceUpdateRequest { managed_meta: Some(incoming), ..Default::default() };
    call_update_service(&state, profile, resource_id, req).await.unwrap();

    let merged = fetch_managed_meta(&state.pool, resource_id).await;
    assert_eq!(merged.extra.get("date"), Some(&serde_json::json!("2026-04-13")));
    assert_eq!(merged.extra.get("custom"), Some(&serde_json::json!("value")));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn open_meta_partial_update_merges_by_key(pool: sqlx::PgPool) {
    let stored_open = serde_json::json!({"tags": ["rust"], "aliases": ["temper-cli"]});
    let (state, profile, resource_id) = setup_resource_with_open_meta(&pool, stored_open).await;

    let incoming = serde_json::json!({"tags": ["rust", "axum"]});
    let req = ResourceUpdateRequest { open_meta: Some(incoming), ..Default::default() };
    call_update_service(&state, profile, resource_id, req).await.unwrap();

    let merged = fetch_open_meta(&state.pool, resource_id).await;
    assert_eq!(merged["tags"], serde_json::json!(["rust", "axum"]));
    assert_eq!(merged["aliases"], serde_json::json!(["temper-cli"]));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn managed_hash_recomputes_after_merge(pool: sqlx::PgPool) {
    let (state, profile, resource_id) = setup_resource_with_managed_meta(
        &pool,
        ManagedMeta { stage: Some("in-progress".to_string()), ..Default::default() },
    ).await;
    let before = fetch_managed_hash(&state.pool, resource_id).await;

    let req = ResourceUpdateRequest {
        managed_meta: Some(ManagedMeta { stage: Some("done".to_string()), ..Default::default() }),
        ..Default::default()
    };
    call_update_service(&state, profile, resource_id, req).await.unwrap();

    let after = fetch_managed_hash(&state.pool, resource_id).await;
    assert_ne!(before, after, "managed_hash must change when managed_meta changes");
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo nextest run -p temper-api --features test-db -E 'test(/managed_meta_|open_meta_|managed_hash_/)'
```

Expected: all FAIL — the existing service `update()` only handles title/slug, ignores managed_meta and open_meta entirely.

- [ ] **Step 3: Expand the service `update()` fn for metadata merge**

In `crates/temper-api/src/services/resource_service.rs`, replace the body of `update()` (currently at line 501) with the metadata-merge path:

```rust
pub async fn update(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
    req: ResourceUpdateRequest,
) -> ApiResult<ResourceRow> {
    let can_modify = sqlx::query_scalar!(
        "SELECT can_modify_resource($1, $2)",
        profile_id,
        resource_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);

    if !can_modify {
        return Err(ApiError::Forbidden);
    }

    let mut tx = pool.begin().await?;

    // 1. Update title/slug on kb_resources (existing behavior).
    let current = get_visible(pool, profile_id, resource_id).await?;
    let new_title = req.title.as_deref().unwrap_or(&current.title);
    let new_slug = req.slug.as_deref().or(current.slug.as_deref());
    sqlx::query!(
        r#"UPDATE kb_resources SET title = $1, slug = $2, updated = now()
           WHERE id = $3 AND is_active = true"#,
        new_title, new_slug, resource_id,
    )
    .execute(&mut *tx)
    .await?;

    // 2. Merge managed_meta + open_meta into kb_resource_manifests.
    if req.managed_meta.is_some() || req.open_meta.is_some() {
        let stored = sqlx::query!(
            r#"SELECT managed_meta, open_meta FROM kb_resource_manifests WHERE resource_id = $1"#,
            resource_id,
        )
        .fetch_one(&mut *tx)
        .await?;

        let mut merged_managed: ManagedMeta =
            serde_json::from_value(stored.managed_meta).unwrap_or_default();
        if let Some(incoming) = req.managed_meta {
            apply_managed_meta_partial(&mut merged_managed, incoming);
        }

        let mut merged_open = stored.open_meta;
        if let Some(incoming_open) = req.open_meta {
            apply_open_meta_partial(&mut merged_open, incoming_open);
        }

        let managed_value = serde_json::to_value(&merged_managed)?;
        let managed_hash = temper_core::hash::compute_canonical_hash(&managed_value);
        let open_hash = temper_core::hash::compute_canonical_hash(&merged_open);

        sqlx::query!(
            r#"UPDATE kb_resource_manifests
               SET managed_meta = $1, managed_hash = $2,
                   open_meta = $3, open_hash = $4,
                   updated = now()
               WHERE resource_id = $5"#,
            managed_value, managed_hash, merged_open, open_hash, resource_id,
        )
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    get_visible(pool, profile_id, resource_id).await
}

/// Overlay `Some` fields from `incoming` onto `target`. `None` fields preserve target.
/// `extra` bucket merges by key (incoming wins).
fn apply_managed_meta_partial(target: &mut ManagedMeta, incoming: ManagedMeta) {
    if incoming.doc_type.is_some() { target.doc_type = incoming.doc_type; }
    if incoming.context.is_some() { target.context = incoming.context; }
    if incoming.updated.is_some() { target.updated = incoming.updated; }
    if incoming.source.is_some() { target.source = incoming.source; }
    if incoming.stage.is_some() { target.stage = incoming.stage; }
    if incoming.mode.is_some() { target.mode = incoming.mode; }
    if incoming.effort.is_some() { target.effort = incoming.effort; }
    if incoming.goal.is_some() { target.goal = incoming.goal; }
    if incoming.seq.is_some() { target.seq = incoming.seq; }
    if incoming.branch.is_some() { target.branch = incoming.branch; }
    if incoming.pr.is_some() { target.pr = incoming.pr; }
    if incoming.status.is_some() { target.status = incoming.status; }
    if incoming.provenance.is_some() { target.provenance = incoming.provenance; }
    if incoming.llm_model.is_some() { target.llm_model = incoming.llm_model; }
    if incoming.llm_run.is_some() { target.llm_run = incoming.llm_run; }
    if incoming.title.is_some() { target.title = incoming.title; }
    if incoming.slug.is_some() { target.slug = incoming.slug; }
    for (k, v) in incoming.extra { target.extra.insert(k, v); }
}

/// Merge incoming JSON object keys into target. Object types only.
fn apply_open_meta_partial(target: &mut serde_json::Value, incoming: serde_json::Value) {
    if let (Some(target_obj), Some(incoming_obj)) = (target.as_object_mut(), incoming.as_object()) {
        for (k, v) in incoming_obj {
            target_obj.insert(k.clone(), v.clone());
        }
    } else {
        // Either side is not an object — incoming replaces target (best-effort).
        *target = incoming;
    }
}
```

If `temper_core::hash::compute_canonical_hash` doesn't exist with that name, locate the existing canonical-hash helper used by `MetaUpdatePayload` and reuse it (search in `crates/temper-core/src/hash.rs` and `crates/temper-core/src/types/managed_meta.rs`).

- [ ] **Step 4: Regenerate SQL cache**

```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
```

Expected: clean diff in `.sqlx/`, no errors.

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo nextest run -p temper-api --features test-db -E 'test(/managed_meta_|open_meta_|managed_hash_/)'
```

Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/services/resource_service.rs crates/temper-api/tests/resource_update_merge_test.rs .sqlx/
git commit -m "feat(api): partial-merge managed_meta + open_meta in resource update"
```

---

## Task 4: Service `update()` — body trio path with chunk dedupe

**Files:**
- Modify: `crates/temper-api/src/services/resource_service.rs::update`
- Test: `crates/temper-api/tests/resource_update_body_test.rs` (new)

- [ ] **Step 1: Write the failing test**

Create `crates/temper-api/tests/resource_update_body_test.rs`:

```rust
#![cfg(feature = "test-db")]

// ... harness imports

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn body_update_changes_body_hash_and_persists_chunks(pool: sqlx::PgPool) {
    let (state, profile, resource_id, original_body_hash) =
        setup_resource_with_body(&pool, "# Original\n\nContent here.").await;

    let new_body = "# Updated\n\nNew content.";
    let new_hash = temper_core::hash::compute_body_hash(new_body);
    let chunks = pack_test_chunks(new_body); // helper that exercises the same chunking primitive

    let req = ResourceUpdateRequest {
        content: Some(new_body.to_string()),
        content_hash: Some(new_hash.clone()),
        chunks_packed: Some(chunks),
        ..Default::default()
    };
    call_update_service(&state, profile, resource_id, req).await.unwrap();

    let after_hash = fetch_body_hash(&state.pool, resource_id).await;
    assert_ne!(original_body_hash, after_hash);
    assert_eq!(after_hash, new_hash);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn body_update_with_unchanged_content_short_circuits(pool: sqlx::PgPool) {
    let body = "# Same body";
    let (state, profile, resource_id, original_body_hash) =
        setup_resource_with_body(&pool, body).await;
    let chunk_count_before = count_resource_chunks(&state.pool, resource_id).await;

    // Send PATCH with the SAME content + hash + chunks
    let req = ResourceUpdateRequest {
        content: Some(body.to_string()),
        content_hash: Some(original_body_hash.clone()),
        chunks_packed: Some(pack_test_chunks(body)),
        ..Default::default()
    };
    call_update_service(&state, profile, resource_id, req).await.unwrap();

    let chunk_count_after = count_resource_chunks(&state.pool, resource_id).await;
    assert_eq!(chunk_count_before, chunk_count_after, "no new chunks should be inserted");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn body_update_combined_with_managed_meta_in_one_tx(pool: sqlx::PgPool) {
    let (state, profile, resource_id, _) =
        setup_resource_with_body(&pool, "# Old").await;

    let new_body = "# New";
    let req = ResourceUpdateRequest {
        managed_meta: Some(ManagedMeta {
            stage: Some("done".to_string()),
            ..Default::default()
        }),
        content: Some(new_body.to_string()),
        content_hash: Some(temper_core::hash::compute_body_hash(new_body)),
        chunks_packed: Some(pack_test_chunks(new_body)),
        ..Default::default()
    };
    call_update_service(&state, profile, resource_id, req).await.unwrap();

    let stored_meta = fetch_managed_meta(&state.pool, resource_id).await;
    let stored_hash = fetch_body_hash(&state.pool, resource_id).await;
    assert_eq!(stored_meta.stage.as_deref(), Some("done"));
    assert_eq!(stored_hash, temper_core::hash::compute_body_hash(new_body));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo nextest run -p temper-api --features test-db -E 'test(/body_update_/)'
```

Expected: all FAIL — service ignores body fields entirely.

- [ ] **Step 3: Add body trio path to `update()`**

Extend the `update()` function from Task 3 with a body-trio handling block before `tx.commit()`. Locate the existing chunk-dedupe primitive used by the ingest service (search `crates/temper-api/src/services/` for chunk-dedupe / `kb_chunks` / `kb_resource_chunks` insert logic — likely in `ingest_service.rs` or a sibling). Reuse it; do not reimplement.

```rust
    // 3. Body trio path: persist + dedupe chunks if content present.
    if let (Some(content), Some(incoming_hash), Some(chunks_packed)) =
        (req.content, req.content_hash, req.chunks_packed)
    {
        let stored_body_hash: String = sqlx::query_scalar!(
            "SELECT body_hash FROM kb_resource_manifests WHERE resource_id = $1",
            resource_id,
        )
        .fetch_one(&mut *tx)
        .await?;

        if incoming_hash != stored_body_hash {
            // Reuse the existing ingest-side chunk dedupe primitive.
            // **IMPORTANT**: locate the actual primitive before writing this call.
            // It is named differently from the placeholder below — likely lives in
            // `crates/temper-api/src/services/ingest_service.rs` or a sibling
            // `chunk_store.rs`. Run:
            //
            //   grep -rn "INSERT INTO kb_chunks\|fn persist_chunks\|fn upsert_chunks\|kb_resource_chunks" \
            //     crates/temper-api/src/services/
            //
            // If no shared primitive exists today (i.e., chunk-insert logic is
            // inlined inside `ingest_service`), STOP and report BLOCKED — do not
            // duplicate chunk-insert logic in `resource_service`. Extracting a
            // shared `chunk_store` primitive is part of the work; raise it for
            // review before proceeding (per `feedback_subagent_escalate_not_soften`).
            shared_chunk_persist(
                &mut *tx,
                resource_id,
                &chunks_packed,
                &content,
            )
            .await?;

            sqlx::query!(
                r#"UPDATE kb_resource_manifests
                   SET body_hash = $1, updated = now()
                   WHERE resource_id = $2"#,
                incoming_hash, resource_id,
            )
            .execute(&mut *tx)
            .await?;
        }
        // else: hash matches stored → short-circuit, no chunk work.
    }
```

The exact primitive name (`persist_chunks_for_resource`) depends on what already exists. If no shared primitive exists, that is a finding — STOP and report BLOCKED rather than duplicating chunk-insert logic in the service file (per the `feedback_subagent_escalate_not_soften` memory).

- [ ] **Step 4: Regenerate SQL cache**

```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
```

- [ ] **Step 5: Run tests**

```bash
cargo nextest run -p temper-api --features test-db -E 'test(/body_update_/)'
```

Expected: all PASS.

- [ ] **Step 6: Augment `ResourceRow` with `body_hash`**

The cloud-mode update CLI prints `content_hash` to stdout for the next show-edit-cat cycle (per spec). `ResourceRow` does not currently expose `body_hash`. Add it:

In `crates/temper-core/src/types/resource.rs:18-40`, add a field to `ResourceRow`:

```rust
pub struct ResourceRow {
    // ... existing fields ...
    pub body_hash: Option<String>, // NEW — populated from kb_resource_manifests
}
```

In `crates/temper-api/src/services/resource_service.rs::get_visible` (or wherever `ResourceRow` rows are constructed from SQL), join against `kb_resource_manifests` to populate `body_hash`:

```sql
LEFT JOIN kb_resource_manifests m ON m.resource_id = r.id
```

Add `m.body_hash AS body_hash` to the SELECT list. Regenerate the SQL cache.

Add a test in `crates/temper-api/tests/resource_update_body_test.rs`:

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_response_includes_body_hash(pool: sqlx::PgPool) {
    let (state, profile, resource_id, _) = setup_resource_with_body(&pool, "# Body").await;
    let req = ResourceUpdateRequest {
        managed_meta: Some(ManagedMeta {
            stage: Some("done".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let resp = call_update_service(&state, profile, resource_id, req).await.unwrap();
    assert!(resp.body_hash.is_some(), "ResourceRow.body_hash should be populated");
}
```

Run the test to verify:
```bash
cargo nextest run -p temper-api --features test-db update_response_includes_body_hash
```

- [ ] **Step 7: Regenerate SQL cache after the SELECT change**

```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
```

- [ ] **Step 8: Commit**

```bash
git add crates/temper-core/src/types/resource.rs crates/temper-api/src/services/resource_service.rs crates/temper-api/tests/resource_update_body_test.rs .sqlx/
git commit -m "feat(api): body trio path + ResourceRow.body_hash for round-trip"
```

---

## Task 5: Regenerate TypeScript bindings

**Files:**
- Modify: `bindings/` (auto-generated)

- [ ] **Step 1: Regenerate**

```bash
cargo make generate-ts-types
```

- [ ] **Step 2: Verify the diff**

```bash
git diff bindings/
```

Expected: `ResourceUpdateRequest.ts` (or wherever it's generated) shows new optional fields matching the Rust struct. No unrelated diffs.

- [ ] **Step 3: Verify temper-ui still compiles**

```bash
cd packages/temper-ui && bun run check
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add bindings/
git commit -m "chore(bindings): regenerate ts-rs types for ResourceUpdateRequest expansion"
```

---

## Task 6: Extract `compute_body_chunks` helper

**Files:**
- Modify: `crates/temper-cli/src/actions/ingest.rs:118-156`
- Test: inline tests in `crates/temper-cli/src/actions/ingest.rs`

- [ ] **Step 1: Write the failing test**

In `crates/temper-cli/src/actions/ingest.rs`, append (or create) a `#[cfg(test)] mod tests` block:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(feature = "embed")]
    fn compute_body_chunks_returns_hash_and_packed_chunks() {
        let content = "# Heading\n\nParagraph one.\n\nParagraph two.";
        let result = compute_body_chunks(content).expect("compute should succeed");
        assert_eq!(result.content_hash, temper_core::hash::compute_body_hash(content));
        assert!(!result.chunks_packed.is_empty());
    }

    #[test]
    #[cfg(feature = "embed")]
    fn build_ingest_payload_uses_compute_body_chunks() {
        let content = "# Test\n\nBody.";
        let payload = build_ingest_payload(
            content, "Title", "ctx", "session", None, None, None,
        ).expect("payload");
        let direct = compute_body_chunks(content).expect("direct compute");
        assert_eq!(payload.content_hash.as_deref(), Some(direct.content_hash.as_str()));
        assert_eq!(payload.chunks_packed.as_deref(), Some(direct.chunks_packed.as_str()));
    }
}
```

(The second test references the grown signature from Task 7. If running tests in plan order, expect Task 6's first test to pass after Step 3 here, and Task 7's tests to compile after Task 7 lands.)

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo nextest run -p temper-cli --features embed compute_body_chunks_returns_hash_and_packed_chunks
```

Expected: FAIL with `cannot find function compute_body_chunks`.

- [ ] **Step 3: Extract the helper**

In `crates/temper-cli/src/actions/ingest.rs`, before the existing `build_ingest_payload`, add:

```rust
/// Body trio extracted from raw markdown — the chunk + hash output that
/// goes onto IngestPayload (cloud create) or ResourceUpdateRequest (cloud update).
pub struct BodyChunks {
    pub content_hash: String,
    pub chunks_packed: String,
}

/// Compute (content_hash, chunks_packed) from raw markdown without
/// vault/manifest side effects. Single source of truth for chunk + hash
/// extraction; used by both `build_ingest_payload` (cloud and local create)
/// and the cloud-mode update path.
#[cfg(feature = "embed")]
pub fn compute_body_chunks(content: &str) -> Result<BodyChunks> {
    use temper_core::types::ingest::pack_chunks;
    use temper_ingest::pipeline::prepare_markdown;

    let content_hash = temper_core::hash::compute_body_hash(content);
    let packed_chunks = prepare_markdown(content)
        .map_err(|e| TemperError::Extraction(format!("embedding failed: {e}")))?;
    let chunks_packed = pack_chunks(&packed_chunks)
        .map_err(|e| TemperError::Extraction(format!("chunk packing failed: {e}")))?;
    Ok(BodyChunks { content_hash, chunks_packed })
}
```

Then refactor the existing `build_ingest_payload` (line 122) to call it:

```rust
#[cfg(feature = "embed")]
pub fn build_ingest_payload(
    content: &str,
    title: &str,
    context: &str,
    doc_type: &str,
    metadata: Option<serde_json::Value>,
    // managed_meta + open_meta added in Task 7
) -> Result<temper_core::types::IngestPayload> {
    let slug = slug_from_title(title);
    let origin_uri = build_uri(context, doc_type, &slug);
    let body = compute_body_chunks(content)?;

    Ok(temper_core::types::IngestPayload {
        title: title.to_owned(),
        origin_uri,
        context_name: context.to_owned(),
        doc_type_name: doc_type.to_owned(),
        content_hash: Some(body.content_hash),
        slug,
        content: content.to_owned(),
        metadata,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(body.chunks_packed),
    })
}
```

- [ ] **Step 4: Run test**

```bash
cargo nextest run -p temper-cli --features embed compute_body_chunks_returns_hash_and_packed_chunks
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/actions/ingest.rs
git commit -m "refactor(cli): extract compute_body_chunks from build_ingest_payload"
```

---

## Task 7: Grow `build_ingest_payload` signature

**Files:**
- Modify: `crates/temper-cli/src/actions/ingest.rs:118-156`
- Modify: `crates/temper-cli/src/actions/sync.rs:1187,1766`
- Modify: `crates/temper-cli/src/commands/add.rs:236,886`

- [ ] **Step 1: Write the failing test**

In `crates/temper-cli/src/actions/ingest.rs` test module, add:

```rust
#[test]
#[cfg(feature = "embed")]
fn build_ingest_payload_attaches_managed_meta_when_some() {
    let mm = temper_core::types::ManagedMeta {
        stage: Some("backlog".to_string()),
        ..Default::default()
    };
    let payload = build_ingest_payload(
        "# Test\nBody",
        "Test Title",
        "temper",
        "task",
        None,
        Some(mm.clone()),
        None,
    ).expect("payload");
    assert_eq!(payload.managed_meta.as_ref().and_then(|m| m.stage.as_deref()), Some("backlog"));
    assert!(payload.open_meta.is_none());
}

#[test]
#[cfg(feature = "embed")]
fn build_ingest_payload_attaches_open_meta_when_some() {
    let om = serde_json::json!({"tags": ["rust"]});
    let payload = build_ingest_payload(
        "# X", "T", "ctx", "session", None, None, Some(om),
    ).expect("payload");
    assert_eq!(payload.open_meta.as_ref().and_then(|o| o.get("tags")), Some(&serde_json::json!(["rust"])));
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo nextest run -p temper-cli --features embed build_ingest_payload_attaches
```

Expected: FAIL — current signature takes 5 args, the tests pass 7.

- [ ] **Step 3: Grow the signature**

In `crates/temper-cli/src/actions/ingest.rs`, modify `build_ingest_payload`:

```rust
#[cfg(feature = "embed")]
pub fn build_ingest_payload(
    content: &str,
    title: &str,
    context: &str,
    doc_type: &str,
    metadata: Option<serde_json::Value>,
    managed_meta: Option<temper_core::types::ManagedMeta>,
    open_meta: Option<serde_json::Value>,
) -> Result<temper_core::types::IngestPayload> {
    let slug = slug_from_title(title);
    let origin_uri = build_uri(context, doc_type, &slug);
    let body = compute_body_chunks(content)?;

    Ok(temper_core::types::IngestPayload {
        title: title.to_owned(),
        origin_uri,
        context_name: context.to_owned(),
        doc_type_name: doc_type.to_owned(),
        content_hash: Some(body.content_hash),
        slug,
        content: content.to_owned(),
        metadata,
        managed_meta,
        open_meta,
        chunks_packed: Some(body.chunks_packed),
    })
}
```

- [ ] **Step 4: Update existing call sites**

In `crates/temper-cli/src/actions/sync.rs:1187`:
```rust
crate::actions::ingest::build_ingest_payload(body, &title, &context, &doc_type, None, None, None)?;
```

In `crates/temper-cli/src/actions/sync.rs:1766`:
```rust
let payload = ingest::build_ingest_payload(merged_body, &title, &context, &doc_type, None, None, None)?;
```

In `crates/temper-cli/src/commands/add.rs:236`:
```rust
let payload = ingest::build_ingest_payload(body, &title, &context, &doc_type, Some(metadata), None, None)?;
```

In `crates/temper-cli/src/commands/add.rs:886`:
```rust
let payload = ingest::build_ingest_payload(body, &title, &context, &doc_type, Some(metadata), None, None)?;
```

- [ ] **Step 5: Run tests**

```bash
cargo nextest run -p temper-cli --features embed build_ingest_payload
cargo build -p temper-cli --features embed
```

Expected: PASS; clean build.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/actions/ingest.rs crates/temper-cli/src/actions/sync.rs crates/temper-cli/src/commands/add.rs
git commit -m "feat(cli): build_ingest_payload accepts managed_meta and open_meta"
```

---

## Task 8: Build `actions::frontmatter::build_managed_meta_for_create`

**Files:**
- Create: `crates/temper-cli/src/actions/frontmatter.rs`
- Modify: `crates/temper-cli/src/actions/mod.rs` (add `pub mod frontmatter;`)
- Test: inline in `crates/temper-cli/src/actions/frontmatter.rs`

- [ ] **Step 1: Map current frontmatter construction**

Before writing tests, run the following to find where each per-doctype creator currently builds its initial frontmatter (this informs the builder's parameter shape):

```bash
grep -rn "ManagedMeta {\|temper-stage:\|temper-mode:\|temper-effort:" crates/temper-cli/src/actions/ crates/temper-cli/src/commands/resource.rs | head -30
```

Note the input parameters each per-doctype creator has (title, context, type-specific extras like `stage`, `mode`, `effort`, `goal`, `seq`, `status`).

- [ ] **Step 2: Write failing tests**

Create `crates/temper-cli/src/actions/frontmatter.rs`:

```rust
//! Typed builder for the managed_meta a templated resource starts with.
//!
//! Single source of truth — local-mode templated creators serialize the
//! returned struct to YAML for the file write; cloud-mode creators pass
//! it directly to `build_ingest_payload`.

use temper_core::types::ManagedMeta;

pub struct NewResourceArgs<'a> {
    pub doc_type: &'a str,
    pub context: &'a str,
    pub title: &'a str,
    // Task-specific (None for non-task types)
    pub mode: Option<&'a str>,
    pub effort: Option<&'a str>,
    pub goal: Option<&'a str>,
    pub stage: Option<&'a str>,
    pub seq: Option<i64>,
    // Goal-specific
    pub status: Option<&'a str>,
    // Provenance (LLM-discovered vs user-created)
    pub provenance: Option<&'a str>,
    pub llm_model: Option<&'a str>,
    pub llm_run: Option<&'a str>,
}

pub fn build_managed_meta_for_create(args: NewResourceArgs<'_>) -> ManagedMeta {
    ManagedMeta {
        doc_type: Some(args.doc_type.to_string()),
        context: Some(args.context.to_string()),
        title: Some(args.title.to_string()),
        stage: args.stage.map(str::to_string),
        mode: args.mode.map(str::to_string),
        effort: args.effort.map(str::to_string),
        goal: args.goal.map(str::to_string),
        seq: args.seq,
        status: args.status.map(str::to_string),
        provenance: args.provenance.map(str::to_string),
        llm_model: args.llm_model.map(str::to_string),
        llm_run: args.llm_run.map(str::to_string),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_managed_meta_carries_task_fields() {
        let mm = build_managed_meta_for_create(NewResourceArgs {
            doc_type: "task",
            context: "temper",
            title: "Wire up cloud writes",
            mode: Some("build"),
            effort: Some("medium"),
            goal: Some("temper-cloud-portable-memory"),
            stage: Some("backlog"),
            seq: Some(60),
            status: None,
            provenance: None,
            llm_model: None,
            llm_run: None,
        });
        assert_eq!(mm.doc_type.as_deref(), Some("task"));
        assert_eq!(mm.mode.as_deref(), Some("build"));
        assert_eq!(mm.effort.as_deref(), Some("medium"));
        assert_eq!(mm.goal.as_deref(), Some("temper-cloud-portable-memory"));
        assert_eq!(mm.stage.as_deref(), Some("backlog"));
        assert_eq!(mm.seq, Some(60));
        assert!(mm.status.is_none());
    }

    #[test]
    fn goal_managed_meta_carries_status() {
        let mm = build_managed_meta_for_create(NewResourceArgs {
            doc_type: "goal",
            context: "temper",
            title: "Land cloud-first",
            mode: None,
            effort: None,
            goal: None,
            stage: None,
            seq: None,
            status: Some("active"),
            provenance: None,
            llm_model: None,
            llm_run: None,
        });
        assert_eq!(mm.doc_type.as_deref(), Some("goal"));
        assert_eq!(mm.status.as_deref(), Some("active"));
    }

    #[test]
    fn session_managed_meta_minimal() {
        let mm = build_managed_meta_for_create(NewResourceArgs {
            doc_type: "session",
            context: "temper",
            title: "2026-04-27 Session D",
            mode: None,
            effort: None,
            goal: None,
            stage: None,
            seq: None,
            status: None,
            provenance: None,
            llm_model: None,
            llm_run: None,
        });
        assert_eq!(mm.doc_type.as_deref(), Some("session"));
        assert_eq!(mm.context.as_deref(), Some("temper"));
        assert!(mm.stage.is_none());
        assert!(mm.status.is_none());
    }
}
```

In `crates/temper-cli/src/actions/mod.rs`, add:
```rust
pub mod frontmatter;
```

- [ ] **Step 3: Run tests**

```bash
cargo nextest run -p temper-cli build_managed_meta_for_create
```

Wait — there's no test by that exact name. Use:
```bash
cargo nextest run -p temper-cli -E 'test(/task_managed_meta|goal_managed_meta|session_managed_meta/)'
```

Expected: all PASS (the builder is straightforward).

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/actions/frontmatter.rs crates/temper-cli/src/actions/mod.rs
git commit -m "feat(cli): typed builder for create-time managed_meta"
```

---

## Task 9: Migrate per-doctype creators to use the builder

**Files:**
- Modify: per-doctype creator files (located via grep)

- [ ] **Step 1: Locate the per-doctype creators**

```bash
grep -rln "fn create" crates/temper-cli/src/actions/ | head -10
grep -rln "doc_type:.*\"task\"\|doc_type:.*\"session\"\|doc_type:.*\"goal\"" crates/temper-cli/src/ | head
```

The per-doctype creator pattern likely produces a frontmatter object. List the files; for each, identify the inline frontmatter construction.

- [ ] **Step 2: Migrate each creator**

For each per-doctype creator (session, task, goal, research, concept, decision), replace the inline `ManagedMeta { ... }` construction with:

```rust
use crate::actions::frontmatter::{build_managed_meta_for_create, NewResourceArgs};

let managed_meta = build_managed_meta_for_create(NewResourceArgs {
    doc_type: "task", // or "session", etc.
    context,
    title,
    mode: mode.as_deref(),
    effort: effort.as_deref(),
    goal: goal.as_deref(),
    stage: Some("backlog"), // or whatever the type's default is
    seq: seq_value,
    status: None,
    provenance: provenance_arg,
    llm_model: llm_model_arg,
    llm_run: llm_run_arg,
});
```

The exact local-mode call site continues to serialize this struct to YAML for the file write. **Behavioral invariant**: bit-for-bit identical wire payload after migration. Task 12 is the regression test that verifies this.

- [ ] **Step 3: Run unit tests**

```bash
cargo nextest run -p temper-cli
```

Expected: all PASS, no regressions.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/actions/
git commit -m "refactor(cli): per-doctype creators use shared frontmatter builder"
```

---

## Task 10: Local-mode regression — wire payload bit-for-bit

**Files:**
- Test: `tests/e2e/tests/cloud_writes_test.rs` — *deferred to Task 18 setup*; for now add a focused test in an existing local-mode integration test file.
- Modify: `tests/e2e/tests/publish_tail_test.rs` (add a snapshot test alongside existing publish tests)

- [ ] **Step 1: Write the test**

In `tests/e2e/tests/publish_tail_test.rs`, add:

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn local_mode_create_unchanged_after_helper_refactor(pool: sqlx::PgPool) {
    // Snapshot the IngestPayload that local-mode session create produces.
    // This pins the wire output so the helper consolidation (Tasks 6-9) cannot
    // silently drift the on-the-wire shape.

    let app = E2eTestApp::start(pool.clone()).await;
    // ... existing harness setup mirrored from prior publish tests in this file

    // Drive a local-mode session create with a deterministic body
    let title = "Snapshot test session";
    let body = "## Goal\n\nDeterministic body.\n";
    let payload = build_local_mode_session_create_payload(&app, title, body).await;

    // Assert the wire payload's expected fields
    assert_eq!(payload.doc_type_name, "session");
    assert_eq!(payload.title, title);
    assert!(payload.managed_meta.is_some());
    let mm = payload.managed_meta.unwrap();
    assert_eq!(mm.doc_type.as_deref(), Some("session"));
    assert_eq!(mm.context.as_deref(), Some(/* test context */));
    assert!(mm.title.is_some());
    // Snapshot the serialized form for stronger pinning
    let serialized = serde_json::to_string(&mm).unwrap();
    insta::assert_snapshot!("local_mode_session_managed_meta", serialized);
}
```

If `insta` snapshot testing isn't already a dev-dependency on the e2e crate, do an exact-string assertion against an inline expected value instead.

- [ ] **Step 2: Run test**

```bash
cargo nextest run -p temper-e2e --features test-db local_mode_create_unchanged_after_helper_refactor
```

Expected: PASS (the refactor in Tasks 6-9 should be wire-equivalent).

If the test fails because of an unexpected diff, that is a finding — STOP and report BLOCKED. Do not adjust the snapshot to make the test pass.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/tests/publish_tail_test.rs
git commit -m "test(e2e): pin local-mode create wire payload after helper refactor"
```

---

## Task 11: Cloud-mode `create` branch + body source resolution

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs:42-118` (the `create` fn)
- Possibly create: `crates/temper-cli/src/actions/body_source.rs` (small helper for resolution rules)
- Test: `tests/e2e/tests/cloud_writes_test.rs` (created in Task 18)

- [ ] **Step 1: Write the body-source resolution helper test**

If creating a new helper file, add tests inline. Otherwise inline in `resource.rs::tests`:

```rust
#[cfg(test)]
mod body_source_tests {
    use super::body_source::resolve_body_source;
    use std::io::Cursor;

    #[test]
    fn resolves_body_at_path_explicit() {
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), "# From file").unwrap();
        let result = resolve_body_source(
            Some(format!("@{}", temp.path().display())),
            /*stdin_is_tty:*/ true,
            Cursor::new(b""),
        ).unwrap();
        assert_eq!(result.unwrap(), "# From file");
    }

    #[test]
    fn resolves_explicit_dash_reads_stdin() {
        let result = resolve_body_source(
            Some("-".to_string()),
            /*stdin_is_tty:*/ false,
            Cursor::new(b"# From stdin"),
        ).unwrap();
        assert_eq!(result.unwrap(), "# From stdin");
    }

    #[test]
    fn resolves_explicit_dash_errors_on_tty() {
        let result = resolve_body_source(
            Some("-".to_string()),
            /*stdin_is_tty:*/ true,
            Cursor::new(b""),
        );
        assert!(result.is_err());
    }

    #[test]
    fn implicit_uses_stdin_when_non_tty() {
        let result = resolve_body_source(
            None,
            /*stdin_is_tty:*/ false,
            Cursor::new(b"# Implicit"),
        ).unwrap();
        assert_eq!(result.unwrap(), "# Implicit");
    }

    #[test]
    fn implicit_returns_none_when_tty_and_no_flag() {
        let result = resolve_body_source(
            None,
            /*stdin_is_tty:*/ true,
            Cursor::new(b""),
        ).unwrap();
        assert!(result.is_none());
    }
}
```

- [ ] **Step 2: Run failing tests**

```bash
cargo nextest run -p temper-cli body_source
```

Expected: FAIL — helper doesn't exist.

- [ ] **Step 3: Implement the helper**

Create `crates/temper-cli/src/actions/body_source.rs`:

```rust
//! Body-source resolution for cloud-mode write commands.
//!
//! Resolution order, first match wins:
//! 1. `--body @<path>` — read file contents; ignore stdin.
//! 2. `--body -` — read stdin explicitly. Errors if stdin is a TTY.
//! 3. Implicit: stdin if non-TTY; else None (caller decides fallback).

use std::io::Read;
use crate::error::{Result, TemperError};

/// Returns Ok(Some(body)) if a body was resolved, Ok(None) for "no body
/// available" (TTY stdin, no flag), Err on resolution failure.
pub fn resolve_body_source<R: Read>(
    flag: Option<String>,
    stdin_is_tty: bool,
    mut stdin_reader: R,
) -> Result<Option<String>> {
    match flag.as_deref() {
        Some(s) if s.starts_with('@') => {
            let path = &s[1..];
            let content = std::fs::read_to_string(path)
                .map_err(|e| TemperError::Io(format!("read --body @{path}: {e}")))?;
            Ok(Some(content))
        }
        Some("-") => {
            if stdin_is_tty {
                return Err(TemperError::InvalidInput(
                    "--body - requires non-TTY stdin".to_string(),
                ));
            }
            let mut buf = String::new();
            stdin_reader.read_to_string(&mut buf)
                .map_err(|e| TemperError::Io(format!("read stdin: {e}")))?;
            Ok(Some(buf))
        }
        Some(other) => Err(TemperError::InvalidInput(
            format!("--body argument must be '-' or '@<path>', got: {other}"),
        )),
        None => {
            if !stdin_is_tty {
                let mut buf = String::new();
                stdin_reader.read_to_string(&mut buf)
                    .map_err(|e| TemperError::Io(format!("read stdin: {e}")))?;
                Ok(Some(buf))
            } else {
                Ok(None)
            }
        }
    }
}
```

Add `pub mod body_source;` to `crates/temper-cli/src/actions/mod.rs`. Add `--body <BODY>` to the relevant clap arg structs for `temper resource create` and `temper resource update`.

- [ ] **Step 4: Add cloud branch to `temper resource create`**

In `crates/temper-cli/src/commands/resource.rs::create` (around line 42), after argument parsing and before per-doctype dispatch:

```rust
use temper_core::types::config::VaultState;
use crate::actions::{body_source, frontmatter, ingest};
use std::io::IsTerminal;

let vault_state = VaultState::from_env();
match vault_state {
    VaultState::Cloud => {
        // Cloud-mode create: skip file write, build IngestPayload, POST /api/ingest.
        let stdin_is_tty = std::io::stdin().is_terminal();
        let body = body_source::resolve_body_source(
            args.body.clone(),
            stdin_is_tty,
            std::io::stdin(),
        )?
        .or_else(|| crate::actions::templates::doctype_template(&args.doc_type, &args.title))
        .ok_or_else(|| TemperError::InvalidInput(
            "cloud-mode create requires body via stdin, --body -, --body @<path>, or a doctype with a default template".to_string()
        ))?;

        let managed_meta = frontmatter::build_managed_meta_for_create(frontmatter::NewResourceArgs {
            doc_type: &args.doc_type,
            context: &args.context,
            title: &args.title,
            mode: args.mode.as_deref(),
            effort: args.effort.as_deref(),
            goal: args.goal.as_deref(),
            stage: stage_for_doctype(&args.doc_type),
            seq: args.seq,
            status: args.status.as_deref(),
            provenance: None,
            llm_model: None,
            llm_run: None,
        });

        let payload = ingest::build_ingest_payload(
            &body,
            &args.title,
            &args.context,
            &args.doc_type,
            None,
            Some(managed_meta),
            None,
        )?;

        let client = crate::runtime::client_from_env().await?;
        let resource = client.ingest().create(&payload).await?;

        println!("{}", serde_json::json!({
            "id": resource.id,
            "slug": resource.slug,
        }));
        return Ok(());
    }
    VaultState::Local => {
        // Existing local-mode flow falls through.
    }
}

// ... existing local-mode code path ...
```

The helper `crate::actions::templates::doctype_template` may need to be created/located — it should return the in-memory body template a doctype starts with if any.

- [ ] **Step 5: Run tests**

```bash
cargo nextest run -p temper-cli body_source
cargo build -p temper-cli --features embed
```

Expected: PASS; clean build.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/actions/body_source.rs crates/temper-cli/src/actions/mod.rs crates/temper-cli/src/commands/resource.rs
git commit -m "feat(cli): cloud-mode create branch with body-source resolution"
```

---

## Task 12: Cloud-mode `update` branch — meta only

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs::update`
- Test: deferred to Task 18 (e2e); rely on integration coverage there.

- [ ] **Step 1: Locate the existing `update` fn**

```bash
grep -n "fn update\|pub fn update" crates/temper-cli/src/commands/resource.rs
```

- [ ] **Step 2: Add cloud branch**

In `crates/temper-cli/src/commands/resource.rs::update`, before any vault resolution:

```rust
use temper_core::types::config::VaultState;
use temper_core::types::{ManagedMeta, ResourceUpdateRequest};

let vault_state = VaultState::from_env();
if matches!(vault_state, VaultState::Cloud) {
    // Resolve slug → id via API. Cloud-mode `show` (Session A) already wired
    // this up — consult `commands/resource.rs` around line 799 for the
    // canonical pattern (likely a `client.resources().list({slug, context})`
    // with a single-row constraint, or `resolve_by_uri` with a built URI).
    // Match whatever `show` does so resolution is consistent across cloud-mode
    // commands.
    let client = crate::runtime::client_from_env().await?;
    let resource = resolve_slug_to_resource_id_cloud(&client, &args.slug, &args.context, args.doc_type.as_deref()).await?;

    // Build partial managed_meta from CLI flags (only Some fields apply).
    let managed_meta = build_partial_managed_meta_from_args(&args);

    // Build partial open_meta from --tags, --aliases, --relates-to, etc.
    let open_meta = build_partial_open_meta_from_args(&args);

    // Body trio (Task 13 fills this in; for meta-only update, leave as None).
    let req = ResourceUpdateRequest {
        title: args.title.clone(),
        slug: args.slug.clone(),
        managed_meta,
        open_meta,
        content: None,
        content_hash: None,
        chunks_packed: None,
    };

    let updated = client.resources().update(resource.id, &req).await?;
    // ResourceRow does not currently expose body_hash. To surface it for the
    // next show-edit-cat cycle, either:
    //   (a) augment ResourceRow with `body_hash: Option<String>` (and update
    //       `crates/temper-api/src/services/resource_service.rs::get_visible`
    //       to populate it from `kb_resource_manifests.body_hash`), OR
    //   (b) introduce a sibling `ResourceUpdateResponse` type that wraps
    //       ResourceRow plus body_hash + managed_meta.
    // Pick (a) — smaller diff, keeps one canonical row type. Add the field to
    // `ResourceRow` in this task; regenerate ts bindings as part of Task 5.
    println!("{}", serde_json::json!({
        "slug": updated.slug,
        "content_hash": updated.body_hash, // requires (a) above
    }));
    return Ok(());
}

// ... existing local-mode update code path ...
```

`build_partial_managed_meta_from_args` and `build_partial_open_meta_from_args` are local helpers in this file — they read the CLI args and emit `Option<ManagedMeta>` / `Option<Value>` where only fields the user passed are `Some`.

- [ ] **Step 3: Add helpers**

In the same file:

```rust
fn build_partial_managed_meta_from_args(args: &UpdateArgs) -> Option<ManagedMeta> {
    if args.stage.is_none() && args.mode.is_none() && args.effort.is_none()
        && args.goal.is_none() && args.seq.is_none() && args.branch.is_none()
        && args.pr.is_none() && args.status.is_none()
    {
        return None;
    }
    Some(ManagedMeta {
        stage: args.stage.clone(),
        mode: args.mode.clone(),
        effort: args.effort.clone(),
        goal: args.goal.clone(),
        seq: args.seq,
        branch: args.branch.clone(),
        pr: args.pr.clone(),
        status: args.status.clone(),
        ..Default::default()
    })
}

fn build_partial_open_meta_from_args(args: &UpdateArgs) -> Option<serde_json::Value> {
    let mut obj = serde_json::Map::new();
    if let Some(tags) = &args.tags { obj.insert("tags".to_string(), serde_json::json!(tags)); }
    if let Some(aliases) = &args.aliases { obj.insert("aliases".to_string(), serde_json::json!(aliases)); }
    // ... repeat for --relates-to, --references, --depends-on, --extends, --preceded-by, --derived-from
    if obj.is_empty() { None } else { Some(serde_json::Value::Object(obj)) }
}
```

- [ ] **Step 4: Run build**

```bash
cargo build -p temper-cli --features embed
cargo nextest run -p temper-cli
```

Expected: clean build, all unit tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs
git commit -m "feat(cli): cloud-mode update branch — partial managed_meta + open_meta"
```

---

## Task 13: Cloud-mode `update` — body trio path

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs::update` (extend Task 12's branch)

- [ ] **Step 1: Extend the cloud branch**

In the cloud-mode update branch added in Task 12, before constructing `ResourceUpdateRequest`, resolve body source and (if present) compute the trio:

```rust
use crate::actions::{body_source, ingest};

let stdin_is_tty = std::io::stdin().is_terminal();
let body = body_source::resolve_body_source(
    args.body.clone(),
    stdin_is_tty,
    std::io::stdin(),
)?;

let (content, content_hash, chunks_packed) = match body {
    Some(b) => {
        let chunks = ingest::compute_body_chunks(&b)?;
        (Some(b), Some(chunks.content_hash), Some(chunks.chunks_packed))
    }
    None => (None, None, None),
};

let req = ResourceUpdateRequest {
    title: args.title.clone(),
    slug: args.slug.clone(),
    managed_meta,
    open_meta,
    content,
    content_hash,
    chunks_packed,
};
```

- [ ] **Step 2: Run build**

```bash
cargo build -p temper-cli --features embed
```

- [ ] **Step 3: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs
git commit -m "feat(cli): cloud-mode update — attach body trio when stdin/--body provides content"
```

---

## Task 14: `temper sync run` cloud guard

**Files:**
- Modify: `crates/temper-cli/src/commands/sync.rs::run`
- Test: `tests/e2e/tests/cloud_writes_test.rs` (deferred to Task 18 — `cloud_sync_run_redirects_with_message`)

- [ ] **Step 1: Add the guard**

In `crates/temper-cli/src/commands/sync.rs::run`, as the first action (before `resolve_vault()` or any manifest read):

```rust
use temper_core::types::config::VaultState;

if matches!(VaultState::from_env(), VaultState::Cloud) {
    return Err(TemperError::InvalidInput(
        "cloud mode has no local vault to sync — use 'temper resource create' and 'temper resource update' directly. To sync, switch to local mode.".to_string()
    ));
}
```

If the existing error variant isn't `InvalidInput`, use the closest one in `crates/temper-cli/src/error.rs`. The exact message text is part of the contract — don't paraphrase.

- [ ] **Step 2: Run build**

```bash
cargo build -p temper-cli --features embed
```

- [ ] **Step 3: Commit**

```bash
git add crates/temper-cli/src/commands/sync.rs
git commit -m "feat(cli): sync run cloud-mode guard with redirect message"
```

---

## Task 15: E2E test scaffolding — `cloud_writes_test.rs`

**Files:**
- Create: `tests/e2e/tests/cloud_writes_test.rs`

- [ ] **Step 1: Create the test file with harness boilerplate**

```rust
//! End-to-end coverage for cloud-mode write paths.
//!
//! These tests drive `TEMPER_VAULT_STATE=cloud` + a per-test `TEMPER_AUTH_PATH`
//! through the in-process Axum harness defined in `common::E2eTestApp`. No
//! vault directory exists for the path under test — write commands must
//! construct their payloads in-memory and post directly to the API.

#![cfg(feature = "test-db")]

mod common;

use common::E2eTestApp;
use temper_core::types::{ManagedMeta, ResourceUpdateRequest};

// Test cases follow in subsequent tasks.
```

- [ ] **Step 2: Verify the file compiles in isolation**

```bash
cargo build -p temper-e2e --features test-db --tests
```

Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/tests/cloud_writes_test.rs
git commit -m "test(e2e): scaffold cloud_writes_test harness"
```

---

## Task 16: E2E test — cloud create round-trip via show

**Files:**
- Modify: `tests/e2e/tests/cloud_writes_test.rs`

- [ ] **Step 1: Write the test**

Append to `cloud_writes_test.rs`:

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cloud_create_session_round_trip_via_show(pool: sqlx::PgPool) {
    let app = E2eTestApp::start(pool.clone()).await;
    let body = "## Goal\n\nE2E round-trip body.\n";

    // Cloud-mode create via the CLI surface (in-process)
    let created = app.cloud_resource_create_session(
        "Round trip session", "temper", body
    ).await.expect("cloud create");

    // Second cloud-mode show should retrieve the same body + managed_meta
    let fetched = app.cloud_resource_show(&created.slug).await.expect("cloud show");
    assert_eq!(fetched.title, "Round trip session");
    assert_eq!(fetched.body, body);
    let mm = fetched.managed_meta.expect("managed_meta on fetched");
    assert_eq!(mm.doc_type.as_deref(), Some("session"));
    assert_eq!(mm.context.as_deref(), Some("temper"));
}
```

The `cloud_resource_create_session` and `cloud_resource_show` helpers live on `E2eTestApp` — add them in `tests/e2e/tests/common/mod.rs` if not present, modeled on existing helpers.

- [ ] **Step 2: Run test to verify it fails or passes**

```bash
cargo nextest run -p temper-e2e --features test-db cloud_create_session_round_trip_via_show
```

Expected: PASS (Tasks 1-13 should make this work end-to-end).

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/tests/cloud_writes_test.rs tests/e2e/tests/common/mod.rs
git commit -m "test(e2e): cloud create session round-trip via show"
```

---

## Task 17: E2E test — cloud update meta-only partial managed_meta

**Files:**
- Modify: `tests/e2e/tests/cloud_writes_test.rs`

- [ ] **Step 1: Write the test**

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cloud_update_meta_only_partial_managed_meta(pool: sqlx::PgPool) {
    let app = E2eTestApp::start(pool.clone()).await;
    let created = app.cloud_resource_create_task(
        "Cloud update test",
        "temper",
        TaskCreateOpts {
            mode: Some("build"),
            effort: Some("medium"),
            goal: Some("temper-cloud-portable-memory"),
            stage: Some("backlog"),
            ..Default::default()
        },
        "## Body\n",
    ).await.expect("create");

    // Update only stage
    app.cloud_resource_update(&created.slug, UpdateOpts {
        stage: Some("done".to_string()),
        ..Default::default()
    }).await.expect("update");

    // Fetch and verify stage updated, mode + goal preserved
    let fetched = app.cloud_resource_show(&created.slug).await.expect("show");
    let mm = fetched.managed_meta.expect("managed_meta");
    assert_eq!(mm.stage.as_deref(), Some("done"));
    assert_eq!(mm.mode.as_deref(), Some("build"));
    assert_eq!(mm.effort.as_deref(), Some("medium"));
    assert_eq!(mm.goal.as_deref(), Some("temper-cloud-portable-memory"));
}
```

- [ ] **Step 2: Run test**

```bash
cargo nextest run -p temper-e2e --features test-db cloud_update_meta_only_partial_managed_meta
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/tests/cloud_writes_test.rs tests/e2e/tests/common/mod.rs
git commit -m "test(e2e): cloud update meta-only preserves untouched managed_meta fields"
```

---

## Task 18: E2E test — cloud update body + meta in one request

**Files:**
- Modify: `tests/e2e/tests/cloud_writes_test.rs`

- [ ] **Step 1: Write the test**

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cloud_update_body_and_meta_in_one_request(pool: sqlx::PgPool) {
    let app = E2eTestApp::start(pool.clone()).await;
    let original = "## Original\n";
    let created = app.cloud_resource_create_session(
        "Body+meta update",
        "temper",
        original,
    ).await.expect("create");
    let original_body_hash = created.body_hash.clone().expect("body_hash on create response");

    // Update body AND stage in one PATCH (body via stdin)
    let new_body = "## Updated\n\nNew content.\n";
    app.cloud_resource_update_with_body(&created.slug, UpdateOpts {
        stage: Some("done".to_string()),
        ..Default::default()
    }, new_body).await.expect("update");

    let fetched = app.cloud_resource_show(&created.slug).await.expect("show");
    assert_eq!(fetched.body, new_body);
    let fetched_body_hash = fetched.body_hash.expect("body_hash on fetched");
    assert_ne!(fetched_body_hash, original_body_hash);
    let mm = fetched.managed_meta.expect("mm");
    assert_eq!(mm.stage.as_deref(), Some("done"));
}
```

- [ ] **Step 2: Run test**

```bash
cargo nextest run -p temper-e2e --features test-db cloud_update_body_and_meta_in_one_request
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/tests/cloud_writes_test.rs tests/e2e/tests/common/mod.rs
git commit -m "test(e2e): cloud update with body + meta in single PATCH"
```

---

## Task 19: E2E test — body-only update, no managed_meta on wire

**Files:**
- Modify: `tests/e2e/tests/cloud_writes_test.rs`

- [ ] **Step 1: Write the test**

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cloud_update_body_only_no_managed_meta(pool: sqlx::PgPool) {
    let app = E2eTestApp::start(pool.clone()).await;
    let created = app.cloud_resource_create_session(
        "Body-only update", "temper", "## Original\n",
    ).await.expect("create");
    let original_mm = created.managed_meta.clone().expect("mm");

    // Body update with NO managed_meta-mutating flags
    let new_body = "## New body\n";
    app.cloud_resource_update_with_body(&created.slug, UpdateOpts::default(), new_body)
        .await.expect("update");

    let fetched = app.cloud_resource_show(&created.slug).await.expect("show");
    assert_eq!(fetched.body, new_body);
    let stored_mm = fetched.managed_meta.expect("mm");
    // Stored managed_meta unchanged (timestamps may update, but typed fields preserved)
    assert_eq!(stored_mm.doc_type, original_mm.doc_type);
    assert_eq!(stored_mm.context, original_mm.context);
    assert_eq!(stored_mm.title, original_mm.title);
}
```

- [ ] **Step 2: Run test**

```bash
cargo nextest run -p temper-e2e --features test-db cloud_update_body_only_no_managed_meta
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/tests/cloud_writes_test.rs
git commit -m "test(e2e): cloud body-only update leaves managed_meta untouched"
```

---

## Task 20: E2E test — chunk dedupe short-circuit

**Files:**
- Modify: `tests/e2e/tests/cloud_writes_test.rs`

- [ ] **Step 1: Write the test**

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cloud_update_chunk_dedupe_skips_unchanged(pool: sqlx::PgPool) {
    let app = E2eTestApp::start(pool.clone()).await;
    let body = "## Same body\n\nSame content.\n";
    let created = app.cloud_resource_create_session("Dedupe test", "temper", body)
        .await.expect("create");

    let chunks_before: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*)::int8 FROM kb_resource_chunks WHERE resource_id = $1",
        created.id,
    ).fetch_one(&pool).await.unwrap().unwrap_or(0);

    // Re-send the exact same body. Server should short-circuit on hash match.
    app.cloud_resource_update_with_body(&created.slug, UpdateOpts::default(), body)
        .await.expect("update");

    let chunks_after: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*)::int8 FROM kb_resource_chunks WHERE resource_id = $1",
        created.id,
    ).fetch_one(&pool).await.unwrap().unwrap_or(0);

    assert_eq!(chunks_before, chunks_after, "no chunk insert/rewire on hash match");
}
```

- [ ] **Step 2: Run test**

```bash
cargo nextest run -p temper-e2e --features test-db cloud_update_chunk_dedupe_skips_unchanged
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/tests/cloud_writes_test.rs
git commit -m "test(e2e): cloud update short-circuits chunk persistence on hash match"
```

---

## Task 21: E2E test — sync run cloud redirect

**Files:**
- Modify: `tests/e2e/tests/cloud_writes_test.rs`

- [ ] **Step 1: Write the test**

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cloud_sync_run_redirects_with_message(pool: sqlx::PgPool) {
    let app = E2eTestApp::start(pool.clone()).await;

    let result = app.cloud_sync_run().await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("cloud mode has no local vault to sync"),
        "expected cloud-redirect message, got: {err}",
    );
}
```

- [ ] **Step 2: Run test**

```bash
cargo nextest run -p temper-e2e --features test-db cloud_sync_run_redirects_with_message
```

Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/tests/cloud_writes_test.rs tests/e2e/tests/common/mod.rs
git commit -m "test(e2e): cloud sync run redirects with explicit message"
```

---

## Task 22: E2E test — cloud list returns remote-only resources

**Files:**
- Modify: `tests/e2e/tests/cloud_writes_test.rs`

- [ ] **Step 1: Write the test**

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cloud_list_returns_remote_only_resources(pool: sqlx::PgPool) {
    let app = E2eTestApp::start(pool.clone()).await;

    // Insert a resource directly into the DB (simulating a peer session's write)
    let inserted_id = app.insert_resource_directly(/* ... */).await;

    // Cloud-mode list should see it
    let listed = app.cloud_resource_list("session", "temper").await.expect("list");
    let ids: Vec<_> = listed.iter().map(|r| r.id).collect();
    assert!(ids.contains(&inserted_id), "cloud list missed remote-only resource");
}
```

- [ ] **Step 2: Run test**

```bash
cargo nextest run -p temper-e2e --features test-db cloud_list_returns_remote_only_resources
```

Expected: PASS (regression-guard for Session A's behavior).

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/tests/cloud_writes_test.rs
git commit -m "test(e2e): regression-guard cloud list returns remote-only resources"
```

---

## Task 23: Stale plan cleanup

**Files:**
- Delete: `docs/superpowers/plans/2026-04-19-unit-b-2-cloud-mode-dispatch.md`

- [ ] **Step 1: Grep for stragglers**

```bash
grep -rn "2026-04-19-unit-b-2-cloud-mode-dispatch" docs/ crates/ tests/ packages/
```

- [ ] **Step 2: Resolve stragglers**

For each hit:
- If it's in a session note (under `docs/sessions/` or similar historical record), leave it but add a one-line clarification at the top of the file: `> Note: the referenced plan was superseded by 2026-04-22-cloud-first-routing-implementation.md.`
- If it's an active reference in code or current docs, replace with a pointer to the 2026-04-22 plan or remove if no longer needed.

- [ ] **Step 3: Delete the file**

```bash
git rm docs/superpowers/plans/2026-04-19-unit-b-2-cloud-mode-dispatch.md
```

- [ ] **Step 4: Verify no remaining references**

```bash
grep -rn "2026-04-19-unit-b-2-cloud-mode-dispatch" docs/ crates/ tests/ packages/
```

Expected: empty output (or only historical session notes with the clarification).

- [ ] **Step 5: Commit**

```bash
git add docs/
git commit -m "chore: remove superseded 2026-04-19 cloud-mode dispatch plan"
```

---

## Task 24: CLAUDE.md — Cloud mode operations subsection

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Add the subsection**

Under "Key Patterns" in `CLAUDE.md`, append:

```markdown
- **Cloud mode operations** — When `TEMPER_VAULT_STATE=cloud`, write paths route directly through the API: `temper resource create` POSTs to `/api/ingest`; `temper resource update` PATCHes `/api/resources/{id}`. **Do not invoke `temper sync run`** in cloud mode — it errors with a redirect message. **Files on disk are derivative under cloud mode** — the `show` debounce cache and any scratch tmpfile from the show-edit-cat pattern are read-cache or base-then-update artifacts, never authoritative. To edit a resource's body in cloud mode, use the show-edit-cat idiom: `temper resource show <slug>` writes the current body to a temp path; modify it; then `cat tmpfile.md | temper resource update <slug> --stage done` posts the updated body + managed_meta in one request. Body source can also be supplied explicitly via `--body @<path>` (read file) or `--body -` (read stdin). Implicit stdin works when stdin is non-TTY.
```

- [ ] **Step 2: Verify rendering**

```bash
head -200 CLAUDE.md
```

- [ ] **Step 3: Commit (separate, per `feedback_keep_claudemd_current`)**

```bash
git add CLAUDE.md
git commit -m "docs(claude): cloud-mode operations subsection"
```

---

## Task 25: Skill guidance — reference.md and subagent-guidance.md

**Files:**
- Modify: `/Users/petetaylor/.claude/skills/temper/reference.md`
- Modify: `/Users/petetaylor/.claude/skills/temper/subagent-guidance.md`

- [ ] **Step 1: Update `reference.md`**

Add a "Cloud-mode operation" section near the existing CLI reference. Document:
- That `TEMPER_VAULT_STATE=cloud` (set automatically in Claude web / Cursor cloud agent sessions) puts the CLI into cloud mode.
- `temper resource create` / `temper resource update` route directly through the API.
- `temper sync run` is intentionally errored in cloud mode.
- The show-edit-cat idiom for body edits.
- `--body -` and `--body @<path>` flag usage.

- [ ] **Step 2: Update `subagent-guidance.md`**

Add one principle (numbered as the next available item):

```markdown
N. **Cloud-mode write paths route through the API directly.** When the active vault is in cloud mode (`TEMPER_VAULT_STATE=cloud`), use `temper resource create` and `temper resource update` for all writes. Do not invoke `temper sync push` or `temper sync run` — these will error in cloud mode. For body edits, use the show-edit-cat idiom: `temper resource show <slug>` to fetch current body, modify in place, then `cat tmpfile.md | temper resource update <slug> --stage done` to post body + managed_meta in one PATCH. Body source can be supplied via `--body -` (stdin) or `--body @<path>` (file).
```

- [ ] **Step 3: Commit (in the kb-vault repo, since these files live in the user's skill directory)**

These files are not part of the temper repo. Skip the git commit step here — the changes land in the user's `~/.claude/skills/temper/` directory, which is managed separately.

If a sync mechanism exists (e.g., a templated skill regeneration tied to a temper command), use it. Otherwise the edits are in-place and persist on disk.

---

## Task 26: End-of-unit verification

**Files:** none (verification only)

- [ ] **Step 1: Run the full check suite**

```bash
cargo make check
```

Expected: green (fmt, clippy with `-D warnings`, machete, biome).

- [ ] **Step 2: Run all tests**

```bash
cargo make test-all
```

Expected: green (Rust unit + integration; TS unit).

- [ ] **Step 3: Run e2e**

```bash
cargo make test-e2e
```

Expected: green, including all new `cloud_writes_test.rs` cases.

- [ ] **Step 4: Verify SQL cache is clean**

```bash
cargo sqlx prepare --workspace -- --all-features --check
```

Expected: no diff against committed `.sqlx/`.

- [ ] **Step 5: Verify TypeScript bindings clean**

```bash
cargo make generate-ts-types
git diff bindings/
```

Expected: clean diff (no uncommitted regenerations).

- [ ] **Step 6: Verify no parallel module returned**

```bash
test ! -f crates/temper-cli/src/commands/resource_cloud.rs && echo OK || echo FAIL
```

Expected: `OK`.

- [ ] **Step 7: Verify no stragglers from the deleted plan**

```bash
grep -rn "2026-04-19-unit-b-2-cloud-mode-dispatch" docs/ crates/ tests/ packages/
```

Expected: empty (or only historical session notes with clarification).

- [ ] **Step 8: Review the commit story**

```bash
git log main..HEAD --oneline
```

Expected: coherent narrative — server-side expansion → CLI helpers → cloud branches → e2e → cleanup → docs. Squash/rebase if any commit is mid-broken or off-narrative.

- [ ] **Step 9: Push the branch**

```bash
git push origin jct/temper-cloud-mode-portable-memory
```

- [ ] **Step 10: Open the PR**

`gh pr create` against `main`. Title and body capture the full Part 3 + Part 3B journey. Reference the closed predecessor task and this task in the body.
