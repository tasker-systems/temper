# Knowledge Graph Phase 2: Edge Extraction & Ingest Pipeline Integration

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Wire the knowledge graph into the ingest pipeline so that relationship fields declared in YAML frontmatter (`extends`, `depends_on`, `relates_to`, etc.) are automatically extracted, resolved to resource IDs, and materialized as edges in `kb_resource_edges` — with deferred storage for unresolved references and full reconciliation on update.

**Architecture:** Relationship fields land in `open_meta` via the existing `split_frontmatter_tiers()` tier split (they're user-owned, non-schema, non-`temper-*` fields). A new `edge_service` extracts `(EdgeType, TargetRef)` declarations from `open_meta`, resolves slug/UUID targets against `kb_resources`, upserts edges with `provenance: "frontmatter"` metadata, defers unresolved references to `kb_deferred_edges`, and attempts to resolve deferred edges whenever a new resource is created. On update, it diffs old vs new frontmatter edges and reconciles (add new, remove stale, preserve manual edges).

**Tech Stack:** Postgres 18, sqlx 0.8, serde, uuid v7, temper-core types (EdgeType, TargetRef, ResourceRelationships, ResolvedEdge, EdgeReconciliation)

**Source Research:** `R7: Vertex-Edge Knowledge Graph in Native Postgres` — sections "Ingest Pipeline Changes" (line ~845), "Edge Extraction Function" (line ~873), "Edge Upsert Logic" (line ~936), "Edge Diffing on Update" (line ~1010)

**Important codebase corrections vs R7:**
- Relationship fields are in `open_meta` (JSON), not raw YAML frontmatter — extraction deserializes from `serde_json::Value`, not `serde_yaml::Value`
- `ResourceRelationships::to_edge_declarations()` already exists in `temper-core/src/types/graph.rs` — reuse it, don't reimplement
- `resources_visible_to()` signature is `(p_profile_id UUID, p_team_id UUID DEFAULT NULL, p_resource_ids UUID[] DEFAULT '{}')` — slug resolution queries must use this for visibility scoping
- `IngestPayload.open_meta` is `Option<serde_json::Value>` — may be `None` for CLI `temper add` path (only populated by sync push)
- `parent` field produces a reversed edge: if doc A declares `parent: B`, the edge is `B --parent_of--> A` (source=B, target=A) — the caller must swap source/target
- The `create_resource_with_manifest()` function commits its own transaction — deferred edge resolution must happen after that commit, not inside it

---

## File Structure

### New Files
| File | Responsibility |
|------|---------------|
| `crates/temper-api/src/services/edge_service.rs` | All edge SQL logic: resolve targets, upsert edges, defer unresolved, reconcile on update, resolve deferred edges on create |
| `crates/temper-api/tests/edge_ingest_test.rs` | Integration tests: ingest-with-edges, update-reconciliation, deferred edge resolution, batch import |

### Modified Files
| File | Change |
|------|--------|
| `crates/temper-api/src/services/mod.rs` | Add `pub mod edge_service;` |
| `crates/temper-api/src/services/ingest_service.rs` | Call `edge_service::extract_and_upsert_edges()` after resource creation in `ingest()`, call `edge_service::reconcile_edges()` after manifest update in `update()`, call `edge_service::resolve_deferred_edges()` after every successful create |
| `crates/temper-api/tests/common/fixtures.rs` | Add `create_test_resource_with_manifest()` for tests that need open_meta populated |

---

## Task 1: Edge Service — Target Resolution

**Files:**
- Create: `crates/temper-api/src/services/edge_service.rs`
- Modify: `crates/temper-api/src/services/mod.rs`

- [ ] **Step 1: Create the edge service module and register it**

Add to `crates/temper-api/src/services/mod.rs`:
```rust
pub mod edge_service;
```

Create `crates/temper-api/src/services/edge_service.rs`:
```rust
//! Edge service — extracts relationship declarations from frontmatter,
//! resolves targets, and manages edges in `kb_resource_edges`.
//!
//! All SQL lives here per the "service layer owns SQL" rule.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;
use temper_core::types::graph::{EdgeType, ResolvedEdge, TargetRef};
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};

/// Resolve a single `TargetRef` to a resource UUID.
///
/// Resolution strategy:
/// 1. `TargetRef::Id` — direct lookup against `kb_resources.id`, scoped to
///    resources visible to the profile.
/// 2. `TargetRef::Slug` — first try same-context match, then cross-context.
///    Must resolve to exactly one visible resource.
///
/// Returns `None` if the target cannot be resolved (forward reference).
pub async fn resolve_target(
    pool: &PgPool,
    profile_id: ProfileId,
    context_id: ContextId,
    target: &TargetRef,
) -> ApiResult<Option<Uuid>> {
    match target {
        TargetRef::Id(uuid) => {
            let exists = sqlx::query_scalar!(
                r#"
                SELECT r.id
                  FROM kb_resources r
                  JOIN resources_visible_to($1, NULL, '{}') v
                    ON v.resource_id = r.id
                 WHERE r.id = $2 AND r.is_active = true
                "#,
                *profile_id,
                *uuid,
            )
            .fetch_optional(pool)
            .await?;
            Ok(exists)
        }
        TargetRef::Slug(slug) => {
            // Try same-context first
            let same_ctx = sqlx::query_scalar!(
                r#"
                SELECT r.id
                  FROM kb_resources r
                  JOIN resources_visible_to($1, NULL, '{}') v
                    ON v.resource_id = r.id
                 WHERE r.slug = $2
                   AND r.kb_context_id = $3
                   AND r.is_active = true
                "#,
                *profile_id,
                slug.as_str(),
                *context_id,
            )
            .fetch_optional(pool)
            .await?;

            if same_ctx.is_some() {
                return Ok(same_ctx);
            }

            // Cross-context: must be exactly one match
            let cross_ctx: Vec<Uuid> = sqlx::query_scalar!(
                r#"
                SELECT r.id
                  FROM kb_resources r
                  JOIN resources_visible_to($1, NULL, '{}') v
                    ON v.resource_id = r.id
                 WHERE r.slug = $2
                   AND r.is_active = true
                "#,
                *profile_id,
                slug.as_str(),
            )
            .fetch_all(pool)
            .await?;

            match cross_ctx.len() {
                1 => Ok(Some(cross_ctx[0])),
                _ => {
                    if cross_ctx.len() > 1 {
                        tracing::warn!(
                            slug = slug.as_str(),
                            matches = cross_ctx.len(),
                            "ambiguous cross-context slug resolution, deferring"
                        );
                    }
                    Ok(None)
                }
            }
        }
    }
}

/// Resolve a batch of `(EdgeType, TargetRef)` declarations into resolved
/// edges and unresolved refs.
///
/// For `ParentOf` edges from a `parent` field declaration, the direction is
/// reversed: the resolved target becomes the source (parent), and the
/// declaring resource becomes the target (child).
pub async fn resolve_declarations(
    pool: &PgPool,
    profile_id: ProfileId,
    context_id: ContextId,
    source_resource_id: ResourceId,
    declarations: &[(EdgeType, TargetRef)],
) -> ApiResult<(Vec<ResolvedEdge>, Vec<(EdgeType, TargetRef)>)> {
    let mut resolved = Vec::new();
    let mut unresolved = Vec::new();

    for (edge_type, target_ref) in declarations {
        match resolve_target(pool, profile_id, context_id, target_ref).await? {
            Some(target_id) => {
                let (src, tgt) = if *edge_type == EdgeType::ParentOf {
                    // Reverse: parent=target becomes source, this resource becomes target
                    (*target_id, **source_resource_id)
                } else {
                    (**source_resource_id, *target_id)
                };

                // Skip self-edges (the DB constraint would reject them anyway)
                if src == tgt {
                    tracing::warn!(
                        edge_type = %edge_type,
                        resource_id = %source_resource_id,
                        "skipping self-referencing edge declaration"
                    );
                    continue;
                }

                resolved.push(ResolvedEdge {
                    source_resource_id: src,
                    target_resource_id: tgt,
                    edge_type: *edge_type,
                    weight: 1.0,
                    metadata: serde_json::json!({"provenance": "frontmatter"}),
                });
            }
            None => {
                unresolved.push((*edge_type, target_ref.clone()));
            }
        }
    }

    Ok((resolved, unresolved))
}
```

- [ ] **Step 2: Verify services module is publicly accessible from tests**

Check `crates/temper-api/src/lib.rs` for `pub mod services`. If it shows
`mod services` (no `pub`), change it to `pub mod services` so integration
tests can call `temper_api::services::edge_service::*`.

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p temper-api`

Expected: Compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/src/services/edge_service.rs crates/temper-api/src/services/mod.rs
git commit -m "feat(graph): add edge_service with target resolution

Phase 2 — resolves TargetRef (UUID or slug) against visible
resources. Same-context slug match first, then cross-context
with ambiguity detection. ParentOf edges are direction-reversed."
```

---

## Task 2: Edge Service — Upsert, Defer, and Resolve Deferred

**Files:**
- Modify: `crates/temper-api/src/services/edge_service.rs`

- [ ] **Step 1: Add edge upsert function**

Append to `edge_service.rs`:

```rust
/// Upsert a single resolved edge. Uses ON CONFLICT to update weight/metadata
/// on duplicate `(source, target, edge_type)`.
pub async fn upsert_edge(pool: &PgPool, edge: &ResolvedEdge, profile_id: ProfileId) -> ApiResult<()> {
    let id = Uuid::now_v7();
    sqlx::query!(
        r#"
        INSERT INTO kb_resource_edges (
            id, source_resource_id, target_resource_id, edge_type,
            weight, metadata, created_by_profile_id
        )
        VALUES ($1, $2, $3, $4::edge_type, $5, $6, $7)
        ON CONFLICT ON CONSTRAINT uq_resource_edge
        DO UPDATE SET
            weight = EXCLUDED.weight,
            metadata = kb_resource_edges.metadata || EXCLUDED.metadata,
            updated = now()
        "#,
        id,
        edge.source_resource_id,
        edge.target_resource_id,
        edge.edge_type.to_string() as _,
        edge.weight,
        edge.metadata,
        *profile_id,
    )
    .execute(pool)
    .await?;
    Ok(())
}

/// Batch-upsert a set of resolved edges.
pub async fn upsert_edges(
    pool: &PgPool,
    edges: &[ResolvedEdge],
    profile_id: ProfileId,
) -> ApiResult<usize> {
    let mut count = 0;
    for edge in edges {
        upsert_edge(pool, edge, profile_id).await?;
        count += 1;
    }
    Ok(count)
}

/// Store unresolved edge declarations in `kb_deferred_edges` for later
/// resolution when the target resource is created.
pub async fn defer_edges(
    pool: &PgPool,
    source_resource_id: ResourceId,
    context_id: ContextId,
    profile_id: ProfileId,
    unresolved: &[(EdgeType, TargetRef)],
) -> ApiResult<usize> {
    let mut count = 0;
    for (edge_type, target_ref) in unresolved {
        let target_ref_str = match target_ref {
            TargetRef::Id(uuid) => uuid.to_string(),
            TargetRef::Slug(slug) => slug.clone(),
        };
        let id = Uuid::now_v7();
        sqlx::query!(
            r#"
            INSERT INTO kb_deferred_edges (
                id, source_resource_id, target_ref, target_context_id,
                edge_type, weight, metadata, created_by_profile_id
            )
            VALUES ($1, $2, $3, $4, $5::edge_type, 1.0, $6, $7)
            "#,
            id,
            **source_resource_id,
            target_ref_str,
            *context_id,
            edge_type.to_string() as _,
            serde_json::json!({"provenance": "frontmatter"}),
            *profile_id,
        )
        .execute(pool)
        .await?;
        count += 1;
    }
    Ok(count)
}

/// Attempt to resolve deferred edges that target a newly-created resource.
///
/// Matches by UUID string or slug (within the deferred edge's target_context_id
/// if set, otherwise any context). Resolved edges are inserted into
/// `kb_resource_edges` and removed from `kb_deferred_edges`.
///
/// Call this after every successful `create_resource_with_manifest()`.
pub async fn resolve_deferred_edges(
    pool: &PgPool,
    new_resource_id: ResourceId,
    new_slug: &str,
    profile_id: ProfileId,
) -> ApiResult<usize> {
    // Find deferred edges matching by UUID or slug
    let deferred: Vec<(Uuid, Uuid, String, Option<Uuid>, String)> = sqlx::query_as(
        r#"
        SELECT de.id, de.source_resource_id, de.target_ref,
               de.target_context_id, de.edge_type::TEXT
          FROM kb_deferred_edges de
         WHERE de.target_ref = $1::TEXT
            OR de.target_ref = $2
        "#,
    )
    .bind(*new_resource_id)
    .bind(new_slug)
    .fetch_all(pool)
    .await?;

    if deferred.is_empty() {
        return Ok(0);
    }

    let mut resolved_count = 0;
    for (deferred_id, source_id, _target_ref, _ctx_id, edge_type_str) in &deferred {
        let edge_type: EdgeType = serde_json::from_value(
            serde_json::Value::String(edge_type_str.clone()),
        )
        .unwrap_or(EdgeType::RelatesTo);

        // Skip if this would be a self-edge
        if *source_id == **new_resource_id {
            tracing::warn!(
                deferred_id = %deferred_id,
                "skipping deferred edge that would create self-reference"
            );
            continue;
        }

        let edge = ResolvedEdge {
            source_resource_id: *source_id,
            target_resource_id: **new_resource_id,
            edge_type,
            weight: 1.0,
            metadata: serde_json::json!({"provenance": "frontmatter", "resolved_from": "deferred"}),
        };

        upsert_edge(pool, &edge, profile_id).await?;

        sqlx::query!("DELETE FROM kb_deferred_edges WHERE id = $1", deferred_id)
            .execute(pool)
            .await?;

        resolved_count += 1;
    }

    if resolved_count > 0 {
        tracing::info!(
            resource_id = %new_resource_id,
            slug = new_slug,
            resolved = resolved_count,
            "resolved deferred edges for new resource"
        );
    }

    Ok(resolved_count)
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p temper-api`

Expected: Compiles cleanly.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-api/src/services/edge_service.rs
git commit -m "feat(graph): add edge upsert, deferral, and deferred resolution

Upsert with ON CONFLICT merge, deferred edge storage for
unresolved targets, and resolution trigger for newly-created
resources. Provenance metadata tracks frontmatter origin."
```

---

## Task 3: Edge Service — Extract from open_meta and Reconcile

**Files:**
- Modify: `crates/temper-api/src/services/edge_service.rs`

- [ ] **Step 1: Add extraction from open_meta**

Append to `edge_service.rs`:

```rust
use temper_core::types::graph::{EdgeReconciliation, ResourceRelationships};

/// Extract relationship declarations from the `open_meta` JSON value.
///
/// Deserializes `ResourceRelationships` fields from the JSON object.
/// Unrecognized fields are ignored (serde default). Returns an empty
/// vec if open_meta is null, not an object, or has no relationship fields.
pub fn extract_declarations_from_open_meta(
    open_meta: &serde_json::Value,
) -> Vec<(EdgeType, TargetRef)> {
    if !open_meta.is_object() {
        return Vec::new();
    }

    let rels: ResourceRelationships = match serde_json::from_value(open_meta.clone()) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    rels.to_edge_declarations()
}

/// Top-level entry point: extract edges from open_meta, resolve, upsert, defer.
///
/// Returns the number of edges created and deferred.
pub async fn extract_and_upsert_edges(
    pool: &PgPool,
    profile_id: ProfileId,
    context_id: ContextId,
    resource_id: ResourceId,
    open_meta: &serde_json::Value,
) -> ApiResult<(usize, usize)> {
    let declarations = extract_declarations_from_open_meta(open_meta);
    if declarations.is_empty() {
        return Ok((0, 0));
    }

    let (resolved, unresolved) =
        resolve_declarations(pool, profile_id, context_id, resource_id, &declarations).await?;

    let created = upsert_edges(pool, &resolved, profile_id).await?;

    let deferred = if !unresolved.is_empty() {
        defer_edges(pool, resource_id, context_id, profile_id, &unresolved).await?
    } else {
        0
    };

    if created > 0 || deferred > 0 {
        tracing::info!(
            resource_id = %resource_id,
            created,
            deferred,
            "extracted edges from frontmatter"
        );
    }

    Ok((created, deferred))
}
```

- [ ] **Step 2: Add reconciliation for updates**

Append to `edge_service.rs`:

```rust
/// Reconcile edges after a frontmatter update.
///
/// Compares the new declarations against existing frontmatter-provenance
/// edges for this resource. Adds new edges, removes stale ones, preserves
/// edges with `provenance != "frontmatter"` (manual or inferred edges).
pub async fn reconcile_edges(
    pool: &PgPool,
    profile_id: ProfileId,
    context_id: ContextId,
    resource_id: ResourceId,
    open_meta: &serde_json::Value,
) -> ApiResult<EdgeReconciliation> {
    let declarations = extract_declarations_from_open_meta(open_meta);

    // Fetch existing frontmatter-provenance edges for this source
    let existing: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        r#"
        SELECT id, target_resource_id, edge_type::TEXT
          FROM kb_resource_edges
         WHERE source_resource_id = $1
           AND metadata->>'provenance' = 'frontmatter'
        "#,
    )
    .bind(*resource_id)
    .fetch_all(pool)
    .await?;

    // Also fetch frontmatter-provenance edges where this resource is target
    // (from ParentOf reversals)
    let existing_as_target: Vec<(Uuid, Uuid, String)> = sqlx::query_as(
        r#"
        SELECT id, source_resource_id, edge_type::TEXT
          FROM kb_resource_edges
         WHERE target_resource_id = $1
           AND edge_type::TEXT = 'parent_of'
           AND metadata->>'provenance' = 'frontmatter'
        "#,
    )
    .bind(*resource_id)
    .fetch_all(pool)
    .await?;

    // Resolve new declarations
    let (resolved, unresolved) =
        resolve_declarations(pool, profile_id, context_id, resource_id, &declarations).await?;

    // Build sets for diffing: (source_id, target_id, edge_type_str)
    use std::collections::HashSet;

    let new_set: HashSet<(Uuid, Uuid, String)> = resolved
        .iter()
        .map(|e| (e.source_resource_id, e.target_resource_id, e.edge_type.to_string()))
        .collect();

    let mut existing_set: HashSet<(Uuid, Uuid, String)> = HashSet::new();
    let mut existing_id_map: std::collections::HashMap<(Uuid, Uuid, String), Uuid> =
        std::collections::HashMap::new();

    for (id, target_id, edge_type_str) in &existing {
        let key = (*resource_id.as_ref(), *target_id, edge_type_str.clone());
        existing_set.insert(key.clone());
        existing_id_map.insert(key, *id);
    }
    for (id, source_id, edge_type_str) in &existing_as_target {
        let key = (*source_id, *resource_id.as_ref(), edge_type_str.clone());
        existing_set.insert(key.clone());
        existing_id_map.insert(key, *id);
    }

    // Diff
    let to_add: Vec<&ResolvedEdge> = resolved
        .iter()
        .filter(|e| {
            !existing_set.contains(&(
                e.source_resource_id,
                e.target_resource_id,
                e.edge_type.to_string(),
            ))
        })
        .collect();

    let to_remove: Vec<Uuid> = existing_set
        .iter()
        .filter(|key| !new_set.contains(*key))
        .filter_map(|key| existing_id_map.get(key).copied())
        .collect();

    let unchanged = existing_set.intersection(&new_set).count();

    // Execute diff
    for edge in &to_add {
        upsert_edge(pool, edge, profile_id).await?;
    }

    for edge_id in &to_remove {
        sqlx::query!("DELETE FROM kb_resource_edges WHERE id = $1", edge_id)
            .execute(pool)
            .await?;
    }

    // Handle newly deferred
    let deferred = if !unresolved.is_empty() {
        // Clear old deferred edges for this source first
        sqlx::query!(
            "DELETE FROM kb_deferred_edges WHERE source_resource_id = $1",
            **resource_id,
        )
        .execute(pool)
        .await?;
        defer_edges(pool, resource_id, context_id, profile_id, &unresolved).await?
    } else {
        // Clear any stale deferred edges
        sqlx::query!(
            "DELETE FROM kb_deferred_edges WHERE source_resource_id = $1",
            **resource_id,
        )
        .execute(pool)
        .await?;
        0
    };

    let reconciliation = EdgeReconciliation {
        added: to_add.len(),
        removed: to_remove.len(),
        unchanged,
        deferred,
    };

    if reconciliation.added > 0 || reconciliation.removed > 0 {
        tracing::info!(
            resource_id = %resource_id,
            added = reconciliation.added,
            removed = reconciliation.removed,
            unchanged = reconciliation.unchanged,
            deferred = reconciliation.deferred,
            "reconciled frontmatter edges"
        );
    }

    Ok(reconciliation)
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p temper-api`

Expected: Compiles cleanly.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/src/services/edge_service.rs
git commit -m "feat(graph): add edge extraction from open_meta and reconciliation

extract_declarations_from_open_meta() deserializes relationship
fields, extract_and_upsert_edges() is the top-level create path,
reconcile_edges() diffs old vs new for updates. Manual edges
(provenance != frontmatter) are always preserved."
```

---

## Task 4: Wire Edge Service into Ingest Pipeline

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs`

- [ ] **Step 1: Add edge extraction to `ingest()` after resource creation**

In `crates/temper-api/src/services/ingest_service.rs`, locate the `ingest()` function.
After the `create_resource_with_manifest()` call (around line 442–459), add edge extraction
before the final `Ok(resource)`:

```rust
    // 6. Extract and upsert edges from frontmatter relationship fields
    if let Some(ref open) = payload.open_meta {
        let context_id = ContextId::from(*context_id);
        let resource_id_typed = ResourceId::from(resource.id);
        if let Err(e) = super::edge_service::extract_and_upsert_edges(
            pool,
            profile_id,
            context_id,
            resource_id_typed,
            open,
        )
        .await
        {
            // Edge extraction is non-fatal — log and continue
            tracing::warn!(
                resource_id = %resource.id,
                error = %e,
                "edge extraction failed during ingest"
            );
        }
    }

    // 7. Attempt to resolve deferred edges targeting this new resource
    if let Some(ref slug) = resource.slug {
        let resource_id_typed = ResourceId::from(resource.id);
        if let Err(e) = super::edge_service::resolve_deferred_edges(
            pool,
            resource_id_typed,
            slug,
            profile_id,
        )
        .await
        {
            tracing::warn!(
                resource_id = %resource.id,
                error = %e,
                "deferred edge resolution failed"
            );
        }
    }

    Ok(resource)
```

- [ ] **Step 2: Add edge reconciliation to `update()` after manifest update**

In the `update()` function, after `tx.commit().await?` (around line 648), add reconciliation
before the final re-fetch:

```rust
    // Reconcile edges from updated frontmatter
    if let Some(ref open) = payload.open_meta {
        let ctx_id = sqlx::query_scalar!(
            "SELECT kb_context_id FROM kb_resources WHERE id = $1",
            *resource_id,
        )
        .fetch_one(pool)
        .await?;

        if let Err(e) = super::edge_service::reconcile_edges(
            pool,
            profile_id,
            ContextId::from(ctx_id),
            resource_id,
            open,
        )
        .await
        {
            tracing::warn!(
                resource_id = %resource_id,
                error = %e,
                "edge reconciliation failed during update"
            );
        }
    }
```

- [ ] **Step 3: Add missing import for ContextId**

At the top of `ingest_service.rs`, the `ContextId` import already exists. Verify:
```rust
use temper_core::types::ids::{ContextId, EventId, ProfileId, ResourceAuditId, ResourceId};
```

If the `ContextId::from(*context_id)` call causes a type error because `context_id` is already
a `ContextId`, adjust accordingly — it may just be `context_id` directly depending on how
the variable is bound in the surrounding code. Check the type of `context_id` at the call site
(it's `ContextId` from `context.id` resolution on line 379).

- [ ] **Step 4: Verify it compiles**

Run: `cargo check -p temper-api`

Expected: Compiles cleanly. Watch for type issues with `context_id` — the variable from
`context_service::resolve_by_name()` returns a `ContextRow` whose `id` field is `ContextId`.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/services/ingest_service.rs
git commit -m "feat(graph): wire edge extraction into ingest and update paths

ingest() now extracts edges from open_meta after resource creation
and resolves deferred edges targeting the new resource. update()
reconciles edges (add new, remove stale, preserve manual). Both
paths are non-fatal — edge failures are logged, not propagated."
```

---

## Task 5: Test Fixtures — Resource with Manifest Helper

**Files:**
- Modify: `crates/temper-api/tests/common/fixtures.rs`

- [ ] **Step 1: Add helper to create a resource with manifest and open_meta**

Append to `fixtures.rs`:

```rust
/// Create a test resource with a manifest row (including open_meta) and return its UUID.
///
/// This is needed for edge integration tests because `extract_and_upsert_edges`
/// reads from open_meta, and `reconcile_edges` queries edges by source resource.
pub async fn create_test_resource_with_manifest(
    pool: &PgPool,
    owner_id: uuid::Uuid,
    title: &str,
    slug: &str,
    open_meta: serde_json::Value,
) -> uuid::Uuid {
    let id = uuid::Uuid::now_v7();
    let context_id = uuid::Uuid::parse_str(TEMPER_CONTEXT_ID).unwrap();
    let doc_type_id = uuid::Uuid::parse_str(RESEARCH_DOC_TYPE_ID).unwrap();
    let origin_uri = format!("test://{slug}");

    sqlx::query(
        r#"INSERT INTO kb_resources
            (id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
             originator_profile_id, owner_profile_id, is_active, created, updated)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $7, true, now(), now())"#,
    )
    .bind(id)
    .bind(context_id)
    .bind(doc_type_id)
    .bind(&origin_uri)
    .bind(title)
    .bind(slug)
    .bind(owner_id)
    .execute(pool)
    .await
    .expect("create test resource");

    sqlx::query(
        r#"INSERT INTO kb_resource_manifests
            (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
           VALUES ($1, 'test-hash', '{}', $2, 'test-mhash', 'test-ohash', now())"#,
    )
    .bind(id)
    .bind(&open_meta)
    .execute(pool)
    .await
    .expect("create test manifest");

    id
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p temper-api --features test-db --tests`

Expected: Compiles cleanly.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-api/tests/common/fixtures.rs
git commit -m "test(graph): add create_test_resource_with_manifest fixture

Supports edge integration tests that need open_meta populated
for extraction and reconciliation testing."
```

---

## Task 6: Integration Tests — Edge Extraction on Ingest

**Files:**
- Create: `crates/temper-api/tests/edge_ingest_test.rs`

- [ ] **Step 1: Write tests for edge extraction from open_meta**

Create `crates/temper-api/tests/edge_ingest_test.rs`:

```rust
#![cfg(feature = "test-db")]

mod common;

use serde_json::json;
use sqlx::PgPool;

/// Extracting edges from open_meta with slug references creates edges.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_extract_edges_from_open_meta(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "edge-extract@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "Doc B", "doc-b").await;

    let open_meta = json!({
        "extends": ["doc-b"],
        "custom_field": "ignored"
    });

    let profile_id = temper_core::types::ids::ProfileId::from(profile);
    let context_id = temper_core::types::ids::ContextId::from(
        uuid::Uuid::parse_str(common::fixtures::TEMPER_CONTEXT_ID).unwrap(),
    );
    let resource_id = temper_core::types::ids::ResourceId::from(r1);

    let (created, deferred) = temper_api::services::edge_service::extract_and_upsert_edges(
        &pool, profile_id, context_id, resource_id, &open_meta,
    )
    .await
    .expect("extract_and_upsert_edges");

    assert_eq!(created, 1, "should create one edge");
    assert_eq!(deferred, 0, "nothing to defer");

    // Verify the edge exists
    let edge_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1 AND target_resource_id = $2",
    )
    .bind(r1)
    .bind(r2)
    .fetch_one(&pool)
    .await
    .expect("count edges");

    assert_eq!(edge_count, 1);
}

/// Unresolvable slug targets are deferred.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_unresolved_targets_deferred(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "defer@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;

    let open_meta = json!({
        "depends_on": ["nonexistent-slug"]
    });

    let profile_id = temper_core::types::ids::ProfileId::from(profile);
    let context_id = temper_core::types::ids::ContextId::from(
        uuid::Uuid::parse_str(common::fixtures::TEMPER_CONTEXT_ID).unwrap(),
    );
    let resource_id = temper_core::types::ids::ResourceId::from(r1);

    let (created, deferred) = temper_api::services::edge_service::extract_and_upsert_edges(
        &pool, profile_id, context_id, resource_id, &open_meta,
    )
    .await
    .expect("extract_and_upsert_edges");

    assert_eq!(created, 0);
    assert_eq!(deferred, 1, "should defer one edge");

    let deferred_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_deferred_edges WHERE source_resource_id = $1")
            .bind(r1)
            .fetch_one(&pool)
            .await
            .expect("count deferred");

    assert_eq!(deferred_count, 1);
}

/// Deferred edges are resolved when the target resource is created.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_deferred_edge_resolution(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "resolve@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;

    // Create edge pointing to a slug that doesn't exist yet
    let open_meta = json!({"extends": ["future-doc"]});

    let profile_id = temper_core::types::ids::ProfileId::from(profile);
    let context_id = temper_core::types::ids::ContextId::from(
        uuid::Uuid::parse_str(common::fixtures::TEMPER_CONTEXT_ID).unwrap(),
    );
    let r1_id = temper_core::types::ids::ResourceId::from(r1);

    let (created, deferred) = temper_api::services::edge_service::extract_and_upsert_edges(
        &pool, profile_id, context_id, r1_id, &open_meta,
    )
    .await
    .expect("initial extract");

    assert_eq!(created, 0);
    assert_eq!(deferred, 1);

    // Now create the target resource
    let r2 =
        common::fixtures::create_test_resource(&pool, profile, "Future Doc", "future-doc").await;
    let r2_id = temper_core::types::ids::ResourceId::from(r2);

    // Resolve deferred edges
    let resolved = temper_api::services::edge_service::resolve_deferred_edges(
        &pool, r2_id, "future-doc", profile_id,
    )
    .await
    .expect("resolve_deferred_edges");

    assert_eq!(resolved, 1, "should resolve one deferred edge");

    // Verify the edge was created
    let edge_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1 AND target_resource_id = $2",
    )
    .bind(r1)
    .bind(r2)
    .fetch_one(&pool)
    .await
    .expect("count edges");

    assert_eq!(edge_count, 1);

    // Deferred table should be empty
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_deferred_edges WHERE source_resource_id = $1")
            .bind(r1)
            .fetch_one(&pool)
            .await
            .expect("count remaining deferred");

    assert_eq!(remaining, 0);
}

/// UUID-based target references resolve directly.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_uuid_target_ref_resolves(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "uuid-ref@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "Doc B", "doc-b").await;

    let open_meta = json!({
        "references": [r2.to_string()]
    });

    let profile_id = temper_core::types::ids::ProfileId::from(profile);
    let context_id = temper_core::types::ids::ContextId::from(
        uuid::Uuid::parse_str(common::fixtures::TEMPER_CONTEXT_ID).unwrap(),
    );
    let r1_id = temper_core::types::ids::ResourceId::from(r1);

    let (created, deferred) = temper_api::services::edge_service::extract_and_upsert_edges(
        &pool, profile_id, context_id, r1_id, &open_meta,
    )
    .await
    .expect("extract");

    assert_eq!(created, 1);
    assert_eq!(deferred, 0);
}

/// open_meta with no relationship fields produces no edges.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_no_relationship_fields_no_edges(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "no-edges@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;

    let open_meta = json!({"some_custom_field": "value"});

    let profile_id = temper_core::types::ids::ProfileId::from(profile);
    let context_id = temper_core::types::ids::ContextId::from(
        uuid::Uuid::parse_str(common::fixtures::TEMPER_CONTEXT_ID).unwrap(),
    );
    let r1_id = temper_core::types::ids::ResourceId::from(r1);

    let (created, deferred) = temper_api::services::edge_service::extract_and_upsert_edges(
        &pool, profile_id, context_id, r1_id, &open_meta,
    )
    .await
    .expect("extract");

    assert_eq!(created, 0);
    assert_eq!(deferred, 0);
}
```

- [ ] **Step 2: Run tests to verify they pass**

Run: `cargo nextest run -p temper-api --features test-db -E 'test(edge_ingest)' --run-ignored all`

Expected: All 5 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-api/tests/edge_ingest_test.rs
git commit -m "test(graph): add edge extraction integration tests

Tests: slug resolution, UUID resolution, deferred storage,
deferred resolution on create, no-op for empty open_meta."
```

---

## Task 7: Integration Tests — Edge Reconciliation on Update

**Files:**
- Modify: `crates/temper-api/tests/edge_ingest_test.rs`

- [ ] **Step 1: Add reconciliation tests**

Append to `edge_ingest_test.rs`:

```rust
/// Reconciliation adds new edges and removes stale ones.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_reconcile_adds_and_removes(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "reconcile@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "Doc B", "doc-b").await;
    let r3 = common::fixtures::create_test_resource(&pool, profile, "Doc C", "doc-c").await;

    let profile_id = temper_core::types::ids::ProfileId::from(profile);
    let context_id = temper_core::types::ids::ContextId::from(
        uuid::Uuid::parse_str(common::fixtures::TEMPER_CONTEXT_ID).unwrap(),
    );
    let r1_id = temper_core::types::ids::ResourceId::from(r1);

    // Initial: r1 extends r2
    let open_meta_v1 = json!({"extends": ["doc-b"]});
    let (created, _) = temper_api::services::edge_service::extract_and_upsert_edges(
        &pool, profile_id, context_id, r1_id, &open_meta_v1,
    )
    .await
    .expect("initial extract");
    assert_eq!(created, 1);

    // Update: r1 now extends r3 instead of r2
    let open_meta_v2 = json!({"extends": ["doc-c"]});
    let recon = temper_api::services::edge_service::reconcile_edges(
        &pool, profile_id, context_id, r1_id, &open_meta_v2,
    )
    .await
    .expect("reconcile");

    assert_eq!(recon.added, 1, "should add r1→r3");
    assert_eq!(recon.removed, 1, "should remove r1→r2");
    assert_eq!(recon.unchanged, 0);

    // Verify: r1→r2 gone, r1→r3 exists
    let edge_to_r2: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1 AND target_resource_id = $2",
    )
    .bind(r1).bind(r2)
    .fetch_one(&pool).await.expect("count");
    assert_eq!(edge_to_r2, 0, "old edge should be removed");

    let edge_to_r3: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_edges WHERE source_resource_id = $1 AND target_resource_id = $2",
    )
    .bind(r1).bind(r3)
    .fetch_one(&pool).await.expect("count");
    assert_eq!(edge_to_r3, 1, "new edge should exist");
}

/// Reconciliation preserves manual (non-frontmatter) edges.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_reconcile_preserves_manual_edges(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "manual@test.com").await;
    let r1 = common::fixtures::create_test_resource(&pool, profile, "Doc A", "doc-a").await;
    let r2 = common::fixtures::create_test_resource(&pool, profile, "Doc B", "doc-b").await;

    let profile_id = temper_core::types::ids::ProfileId::from(profile);
    let context_id = temper_core::types::ids::ContextId::from(
        uuid::Uuid::parse_str(common::fixtures::TEMPER_CONTEXT_ID).unwrap(),
    );
    let r1_id = temper_core::types::ids::ResourceId::from(r1);

    // Create a manual edge (not from frontmatter)
    let manual_id = uuid::Uuid::now_v7();
    sqlx::query(
        r#"INSERT INTO kb_resource_edges
            (id, source_resource_id, target_resource_id, edge_type,
             weight, metadata, created_by_profile_id)
           VALUES ($1, $2, $3, 'references'::edge_type, 1.0, '{"provenance": "manual"}', $4)"#,
    )
    .bind(manual_id)
    .bind(r1)
    .bind(r2)
    .bind(profile)
    .execute(&pool)
    .await
    .expect("create manual edge");

    // Reconcile with empty frontmatter — manual edge should survive
    let open_meta = json!({});
    let recon = temper_api::services::edge_service::reconcile_edges(
        &pool, profile_id, context_id, r1_id, &open_meta,
    )
    .await
    .expect("reconcile");

    assert_eq!(recon.removed, 0, "manual edge should not be removed");

    // Verify manual edge still exists
    let manual_exists: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_resource_edges WHERE id = $1")
            .bind(manual_id)
            .fetch_one(&pool)
            .await
            .expect("count");
    assert_eq!(manual_exists, 1, "manual edge must survive reconciliation");
}

/// ParentOf edge is direction-reversed: declaring `parent: doc-b` on doc-a
/// creates an edge `doc-b --parent_of--> doc-a`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn test_parent_of_direction_reversal(pool: PgPool) {
    common::fixtures::clean_and_seed(&pool).await;

    let profile = common::fixtures::create_test_profile(&pool, "parent@test.com").await;
    let child = common::fixtures::create_test_resource(&pool, profile, "Child", "child-doc").await;
    let parent =
        common::fixtures::create_test_resource(&pool, profile, "Parent", "parent-doc").await;

    let open_meta = json!({"parent": "parent-doc"});

    let profile_id = temper_core::types::ids::ProfileId::from(profile);
    let context_id = temper_core::types::ids::ContextId::from(
        uuid::Uuid::parse_str(common::fixtures::TEMPER_CONTEXT_ID).unwrap(),
    );
    let child_id = temper_core::types::ids::ResourceId::from(child);

    let (created, _) = temper_api::services::edge_service::extract_and_upsert_edges(
        &pool, profile_id, context_id, child_id, &open_meta,
    )
    .await
    .expect("extract");

    assert_eq!(created, 1);

    // Edge should be: parent → child (not child → parent)
    let edge: (uuid::Uuid, uuid::Uuid, String) = sqlx::query_as(
        "SELECT source_resource_id, target_resource_id, edge_type::TEXT FROM kb_resource_edges WHERE edge_type = 'parent_of'::edge_type",
    )
    .fetch_one(&pool)
    .await
    .expect("fetch parent_of edge");

    assert_eq!(edge.0, parent, "source should be the parent");
    assert_eq!(edge.1, child, "target should be the child");
    assert_eq!(edge.2, "parent_of");
}
```

- [ ] **Step 2: Run all edge ingest tests**

Run: `cargo nextest run -p temper-api --features test-db -E 'test(edge_ingest)' --run-ignored all`

Expected: All 8 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-api/tests/edge_ingest_test.rs
git commit -m "test(graph): add reconciliation and parent_of integration tests

Tests: add+remove on update, manual edge preservation,
ParentOf direction reversal (parent declared in child's
frontmatter creates parent→child edge)."
```

---

## Task 8: Unit Tests — extract_declarations_from_open_meta

**Files:**
- Modify: `crates/temper-api/src/services/edge_service.rs`

- [ ] **Step 1: Add unit tests to edge_service**

Append to the bottom of `edge_service.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use temper_core::types::graph::EdgeType;

    #[test]
    fn extract_empty_object() {
        let decls = extract_declarations_from_open_meta(&json!({}));
        assert!(decls.is_empty());
    }

    #[test]
    fn extract_null_value() {
        let decls = extract_declarations_from_open_meta(&serde_json::Value::Null);
        assert!(decls.is_empty());
    }

    #[test]
    fn extract_no_relationship_fields() {
        let meta = json!({"custom_field": "value", "another": 42});
        let decls = extract_declarations_from_open_meta(&meta);
        assert!(decls.is_empty());
    }

    #[test]
    fn extract_single_extends() {
        let meta = json!({"extends": ["some-slug"]});
        let decls = extract_declarations_from_open_meta(&meta);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].0, EdgeType::Extends);
        assert_eq!(decls[0].1, TargetRef::Slug("some-slug".to_string()));
    }

    #[test]
    fn extract_multiple_types() {
        let meta = json!({
            "extends": ["doc-a"],
            "depends_on": ["doc-b", "doc-c"],
            "references": ["019d1d24-2000-7379-8f26-ae4ae87bc5c6"],
            "irrelevant": "ignored"
        });
        let decls = extract_declarations_from_open_meta(&meta);
        // extends: 1, depends_on: 2, references: 1 (UUID) = 4
        assert_eq!(decls.len(), 4);
    }

    #[test]
    fn extract_skips_urls() {
        let meta = json!({
            "references": ["https://example.com", "valid-slug"]
        });
        let decls = extract_declarations_from_open_meta(&meta);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].1, TargetRef::Slug("valid-slug".to_string()));
    }

    #[test]
    fn extract_parent_produces_parent_of() {
        let meta = json!({"parent": "parent-slug"});
        let decls = extract_declarations_from_open_meta(&meta);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].0, EdgeType::ParentOf);
    }
}
```

- [ ] **Step 2: Run unit tests**

Run: `cargo nextest run -p temper-api -E 'test(edge_service::tests)' --no-fail-fast`

Expected: All 7 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-api/src/services/edge_service.rs
git commit -m "test(graph): add unit tests for extract_declarations_from_open_meta

Tests: empty/null input, no relationship fields, single extends,
multiple types, URL filtering, parent→ParentOf mapping."
```

---

## Task 9: Regenerate sqlx Cache and Run Full Test Suite

**Files:**
- Modify: `.sqlx/` (regenerated)

- [ ] **Step 1: Regenerate the sqlx offline cache**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo sqlx prepare --workspace -- --all-features`

Expected: `query data written to .sqlx in the workspace root`

- [ ] **Step 2: Run the full check suite**

Run: `cargo make check`

Expected: All checks pass (fmt, clippy, docs, machete, TypeScript).

- [ ] **Step 3: Run all integration tests**

Run: `cargo nextest run -p temper-api --features test-db --run-ignored all --no-fail-fast`

Expected: All tests pass — both the new edge_ingest tests and the existing graph_test / resources / auth / reconstitution tests.

- [ ] **Step 4: Commit the sqlx cache**

```bash
git add .sqlx/
git commit -m "chore: regenerate sqlx cache for edge service queries"
```

---

## Implementation Notes for Agents

### Where relationship fields live in the data flow

```
User's markdown file:
  ---
  extends: [doc-b]
  depends_on: [doc-c]
  ---

  ↓ temper sync push / temper add

CLI: parse_frontmatter() → split_frontmatter_tiers()
  managed_meta: {"temper-stage": "backlog", ...}
  open_meta:    {"extends": ["doc-b"], "depends_on": ["doc-c"]}

  ↓ POST /api/ingest (IngestPayload.open_meta)

Server: ingest() → create_resource_with_manifest()
  ↓
Server: edge_service::extract_and_upsert_edges(open_meta)
  → extract_declarations_from_open_meta()  // deserialize ResourceRelationships
  → resolve_declarations()                  // slug/UUID lookup
  → upsert_edges() + defer_edges()         // DB writes
  ↓
Server: edge_service::resolve_deferred_edges()  // check for waiting edges
```

### Key types (all in temper-core/src/types/graph.rs — DO NOT redefine)

- `EdgeType` — enum with `sqlx::Type` + serde + Display
- `TargetRef` — `Id(Uuid)` or `Slug(String)`, has `parse()` method
- `ResourceRelationships` — serde struct with all relationship fields, has `to_edge_declarations()` method
- `ResolvedEdge` — ready for DB insert: source_id, target_id, edge_type, weight, metadata
- `EdgeReconciliation` — diff result: added, removed, unchanged, deferred

### Error handling strategy

Edge extraction/resolution is **non-fatal** in the ingest pipeline. If edge
extraction fails, the resource is still created successfully — the failure is
logged with `tracing::warn!`. This prevents edge-related bugs from breaking
the core ingest flow.

### SQL note

All queries in `edge_service.rs` use runtime `sqlx::query()` / `sqlx::query_scalar()`
(not the compile-time `sqlx::query!()` macro) for the `edge_type::TEXT` cast
pattern, which the sqlx macro doesn't handle well with custom enums. This
follows the same pattern as `search_service.rs` which uses runtime queries for
similar reasons (pgvector `::vector` casts).

**Exception:** Where straightforward column queries work with the macro
(e.g., simple INSERT/DELETE without enum casts in WHERE), prefer the macro.
The resolve_target queries use the macro because they don't filter on edge_type.

### Batch import optimization (deferred to Phase 2b)

The R7 research doc describes a 5-phase batch approach for `temper add --dir`
that resolves all slugs in a single batch query. This plan processes edges
per-resource as each is ingested. This is **correct** — the deferred edge
mechanism handles forward references during bulk import — but not N+1
optimized. A batch optimization (collect all declarations, resolve in one
query, batch INSERT) is a performance enhancement for Phase 2b if bulk
import latency becomes a problem. For now, the per-resource approach is
simpler and testable.
