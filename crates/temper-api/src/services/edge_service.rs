//! Edge service — extracts relationship declarations from frontmatter,
//! resolves targets, and manages edges in `kb_resource_edges`.
//!
//! All SQL lives here per the "service layer owns SQL" rule. Resolution and
//! mutation are batched — each `extract_and_upsert_edges` / `reconcile_edges`
//! call fires at most O(1) DB round-trips per operation, not O(N) over the
//! declaration list.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;
use temper_core::types::graph::{
    EdgeReconciliation, EdgeType, ResolvedEdge, ResourceRelationships, TargetRef,
};
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};

// ─── Target Resolution ──────────────────────────────────────────────────────

/// Resolve a batch of `(EdgeType, TargetRef)` declarations into `ResolvedEdge`s.
///
/// For `ParentOf` edges the direction is reversed: the resolved target becomes
/// the source (parent) and the declaring resource becomes the target (child).
///
/// Returns `(resolved, unresolved)` — unresolved refs are forward references
/// that should be deferred.
///
/// Performance: one DB round-trip regardless of declaration count. The query
/// unnests the declaration list into a `refs` table, joins it against the
/// visible resource set, and returns every (ord, candidate_id, same_ctx) row.
/// Resolution semantics match the prior per-ref implementation:
///
///   * UUID ref → unique match or none (PK lookup).
///   * Slug + at least one same-context candidate → first same-context wins.
///   * Slug + no same-context + exactly one cross-context candidate → use it.
///   * Slug + no same-context + multiple cross-context candidates → unresolved
///     + tracing warning.
pub async fn resolve_declarations(
    pool: &PgPool,
    profile_id: &ProfileId,
    context_id: &ContextId,
    resource_id: &ResourceId,
    declarations: &[(EdgeType, TargetRef)],
) -> ApiResult<(Vec<ResolvedEdge>, Vec<(EdgeType, TargetRef)>)> {
    if declarations.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    let (ords, kinds, refs): (Vec<i32>, Vec<String>, Vec<String>) = declarations
        .iter()
        .enumerate()
        .map(|(i, (_, t))| match t {
            TargetRef::Id(uuid) => (i as i32, "uuid".to_string(), uuid.to_string()),
            TargetRef::Slug(slug) => (i as i32, "slug".to_string(), slug.clone()),
        })
        .fold(
            (Vec::new(), Vec::new(), Vec::new()),
            |(mut o, mut k, mut r), (ord, kind, text)| {
                o.push(ord);
                k.push(kind);
                r.push(text);
                (o, k, r)
            },
        );

    // One round-trip: for each declaration ord, emit every visible candidate
    // with a same_ctx flag. UUID matches cast the ref_text; slug matches use
    // string equality. Bounded by the count of visible resources matching
    // each ref (typically 0–1 for UUIDs, small for slugs).
    let rows = sqlx::query!(
        r#"
        WITH refs(ord, ref_kind, ref_text) AS (
            SELECT * FROM UNNEST($2::int[], $3::text[], $4::text[])
        )
        SELECT refs.ord                     AS "ord!: i32",
               r.id                         AS "resource_id!: Uuid",
               (r.kb_context_id = $5)       AS "same_ctx!: bool"
          FROM refs
          JOIN kb_resources r ON (
               (refs.ref_kind = 'uuid' AND r.id = refs.ref_text::uuid)
            OR (refs.ref_kind = 'slug' AND r.slug = refs.ref_text)
          )
          JOIN resources_visible_to($1, NULL, '{}') rv ON rv.resource_id = r.id
         ORDER BY refs.ord, (r.kb_context_id = $5) DESC, r.id
        "#,
        **profile_id,
        &ords,
        &kinds,
        &refs,
        **context_id,
    )
    .fetch_all(pool)
    .await?;

    // Group candidates by ord. `rows` is already ordered (ord ASC, same_ctx
    // DESC, id ASC) so same_ctx candidates come first for each ord.
    let mut by_ord: std::collections::HashMap<i32, Vec<(Uuid, bool)>> =
        std::collections::HashMap::new();
    for row in rows {
        by_ord
            .entry(row.ord)
            .or_default()
            .push((row.resource_id, row.same_ctx));
    }

    let mut resolved = Vec::new();
    let mut unresolved = Vec::new();
    for (i, (edge_type, target)) in declarations.iter().enumerate() {
        let candidates = by_ord.remove(&(i as i32)).unwrap_or_default();
        let chosen = select_candidate(&candidates);
        match chosen {
            Resolution::One(target_uuid) => {
                let (source, dest) = if *edge_type == EdgeType::ParentOf {
                    (target_uuid, **resource_id)
                } else {
                    (**resource_id, target_uuid)
                };
                if source == dest {
                    tracing::warn!(
                        resource_id = %resource_id,
                        edge_type = %edge_type,
                        "self-edge detected, skipping"
                    );
                    continue;
                }
                resolved.push(ResolvedEdge {
                    source_resource_id: source,
                    target_resource_id: dest,
                    edge_type: *edge_type,
                    weight: 1.0,
                    metadata: serde_json::json!({"provenance": "frontmatter"}),
                });
            }
            Resolution::Ambiguous => {
                if let TargetRef::Slug(slug) = target {
                    tracing::warn!(
                        slug = %slug,
                        matches = candidates.len(),
                        "ambiguous slug reference — resolved to multiple visible resources, skipping"
                    );
                }
                unresolved.push((*edge_type, target.clone()));
            }
            Resolution::None => {
                unresolved.push((*edge_type, target.clone()));
            }
        }
    }

    Ok((resolved, unresolved))
}

enum Resolution {
    One(Uuid),
    Ambiguous,
    None,
}

/// Pick a winner from the ordered candidate list for one declaration.
///
/// Rows arrive ordered by (same_ctx DESC, id ASC). Rules match the legacy
/// per-ref implementation:
///   * Any same-ctx candidate → first one wins (same-context always beats
///     cross-context, even if multiple same-ctx matches exist — matches the
///     prior `fetch_optional` semantics).
///   * No same-ctx + one cross-ctx → use it.
///   * No same-ctx + multiple cross-ctx → Ambiguous.
fn select_candidate(candidates: &[(Uuid, bool)]) -> Resolution {
    if let Some(&(id, _)) = candidates.iter().find(|(_, same)| *same) {
        return Resolution::One(id);
    }
    match candidates.len() {
        0 => Resolution::None,
        1 => Resolution::One(candidates[0].0),
        _ => Resolution::Ambiguous,
    }
}

// ─── Upsert & Defer ─────────────────────────────────────────────────────────

/// Upsert a batch of resolved edges in a single round-trip. Uses ON CONFLICT
/// to merge metadata, so re-running over the same declarations is idempotent.
///
/// Returns the count of edges processed (equal to the input length).
pub async fn upsert_edges(
    pool: &PgPool,
    edges: &[ResolvedEdge],
    profile_id: &ProfileId,
) -> ApiResult<usize> {
    if edges.is_empty() {
        return Ok(0);
    }

    let ids: Vec<Uuid> = edges.iter().map(|_| Uuid::now_v7()).collect();
    let sources: Vec<Uuid> = edges.iter().map(|e| e.source_resource_id).collect();
    let targets: Vec<Uuid> = edges.iter().map(|e| e.target_resource_id).collect();
    let edge_types: Vec<String> = edges.iter().map(|e| e.edge_type.to_string()).collect();
    let weights: Vec<f64> = edges.iter().map(|e| e.weight).collect();
    let metadata: Vec<serde_json::Value> = edges.iter().map(|e| e.metadata.clone()).collect();

    sqlx::query(
        "INSERT INTO kb_resource_edges
            (id, source_resource_id, target_resource_id, edge_type, weight, metadata, created_by_profile_id)
         SELECT u.id, u.source_id, u.target_id, u.edge_type::edge_type, u.weight, u.metadata, $7
           FROM UNNEST($1::uuid[], $2::uuid[], $3::uuid[], $4::text[], $5::float8[], $6::jsonb[])
             AS u(id, source_id, target_id, edge_type, weight, metadata)
         ON CONFLICT ON CONSTRAINT uq_resource_edge
         DO UPDATE SET weight = EXCLUDED.weight,
                       metadata = kb_resource_edges.metadata || EXCLUDED.metadata,
                       updated = now()",
    )
    .bind(&ids)
    .bind(&sources)
    .bind(&targets)
    .bind(&edge_types)
    .bind(&weights)
    .bind(&metadata)
    .bind(**profile_id)
    .execute(pool)
    .await?;

    Ok(edges.len())
}

/// Store unresolved target references in `kb_deferred_edges` in a single round-trip.
pub async fn defer_edges(
    pool: &PgPool,
    resource_id: &ResourceId,
    context_id: &ContextId,
    profile_id: &ProfileId,
    unresolved: &[(EdgeType, TargetRef)],
) -> ApiResult<usize> {
    if unresolved.is_empty() {
        return Ok(0);
    }

    let ids: Vec<Uuid> = unresolved.iter().map(|_| Uuid::now_v7()).collect();
    let edge_types: Vec<String> = unresolved.iter().map(|(t, _)| t.to_string()).collect();
    let target_refs: Vec<String> = unresolved
        .iter()
        .map(|(_, t)| match t {
            TargetRef::Id(uuid) => uuid.to_string(),
            TargetRef::Slug(slug) => slug.clone(),
        })
        .collect();

    sqlx::query(
        "INSERT INTO kb_deferred_edges
            (id, source_resource_id, target_ref, target_context_id, edge_type, weight, metadata, created_by_profile_id)
         SELECT u.id, $2, u.target_ref, $3, u.edge_type::edge_type, 1.0, '{\"provenance\": \"frontmatter\"}'::jsonb, $4
           FROM UNNEST($1::uuid[], $5::text[], $6::text[])
             AS u(id, edge_type, target_ref)",
    )
    .bind(&ids)
    .bind(**resource_id)
    .bind(**context_id)
    .bind(**profile_id)
    .bind(&edge_types)
    .bind(&target_refs)
    .execute(pool)
    .await?;

    Ok(unresolved.len())
}

/// Attempt to resolve deferred edges that reference a newly-created resource.
///
/// Looks up deferred edges matching either the new resource's UUID or slug,
/// creates the resolved edges, and deletes the deferred records.
///
/// Returns the count of newly resolved edges.
pub async fn resolve_deferred_edges(
    pool: &PgPool,
    new_resource_id: &ResourceId,
    new_slug: Option<&str>,
    profile_id: &ProfileId,
) -> ApiResult<usize> {
    // Build the query to find matching deferred edges
    let rows: Vec<DeferredEdgeRow> = sqlx::query_as::<_, DeferredEdgeRow>(
        "SELECT id, source_resource_id, target_ref, edge_type::TEXT AS edge_type, weight, metadata
           FROM kb_deferred_edges
          WHERE target_ref = $1::TEXT
             OR ($2::TEXT IS NOT NULL AND target_ref = $2)",
    )
    .bind(**new_resource_id)
    .bind(new_slug)
    .fetch_all(pool)
    .await?;

    let mut edges_to_upsert: Vec<ResolvedEdge> = Vec::new();
    let mut deferred_ids_to_delete: Vec<Uuid> = Vec::new();

    for row in &rows {
        let edge_type: EdgeType =
            match serde_json::from_value(serde_json::Value::String(row.edge_type.clone())) {
                Ok(et) => et,
                Err(e) => {
                    tracing::warn!(
                        deferred_id = %row.id,
                        edge_type = %row.edge_type,
                        error = %e,
                        "failed to parse deferred edge type, skipping"
                    );
                    continue;
                }
            };

        // ParentOf means the deferred source declared a parent, so the new
        // resource is the parent (source) and the declaring resource is the
        // child (target).
        let (source, target) = if edge_type == EdgeType::ParentOf {
            (**new_resource_id, row.source_resource_id)
        } else {
            (row.source_resource_id, **new_resource_id)
        };

        if source == target {
            tracing::warn!(
                deferred_id = %row.id,
                resource_id = %source,
                "deferred edge would create self-edge, skipping"
            );
            continue;
        }

        edges_to_upsert.push(ResolvedEdge {
            source_resource_id: source,
            target_resource_id: target,
            edge_type,
            weight: row.weight,
            metadata: row.metadata.clone(),
        });
        deferred_ids_to_delete.push(row.id);
    }

    let resolved_count = edges_to_upsert.len();
    if resolved_count > 0 {
        upsert_edges(pool, &edges_to_upsert, profile_id).await?;
        // Delete the resolved deferred edges in a single round-trip.
        sqlx::query("DELETE FROM kb_deferred_edges WHERE id = ANY($1::uuid[])")
            .bind(&deferred_ids_to_delete)
            .execute(pool)
            .await?;

        tracing::info!(
            resource_id = %new_resource_id,
            resolved = resolved_count,
            "resolved deferred edges for new resource"
        );
    }

    Ok(resolved_count)
}

/// Internal row type for deferred edge queries.
#[derive(Debug, sqlx::FromRow)]
struct DeferredEdgeRow {
    id: Uuid,
    source_resource_id: Uuid,
    #[expect(dead_code)]
    target_ref: String,
    edge_type: String,
    weight: f64,
    metadata: serde_json::Value,
}

// ─── Extraction & Reconciliation ────────────────────────────────────────────

/// Extract edge declarations from a resource's full meta.
///
/// Reads relationship fields from `open_meta` (via `ResourceRelationships`)
/// and, for tasks, the `temper-goal` field from `managed_meta` which
/// yields a reversed `ParentOf` edge to the goal resource.
///
/// Pure function — no database access. Unknown fields in either
/// tier are ignored.
pub fn extract_declarations_from_resource(
    doc_type: &str,
    managed_meta: &serde_json::Value,
    open_meta: &serde_json::Value,
) -> Vec<(EdgeType, TargetRef)> {
    let mut edges = Vec::new();

    // Open-meta relationships (existing path)
    if open_meta.is_object() {
        match serde_json::from_value::<ResourceRelationships>(open_meta.clone()) {
            Ok(rels) => edges.extend(rels.to_edge_declarations()),
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "open_meta did not contain valid relationship fields"
                );
            }
        }
    }

    // Managed-meta derivations
    if doc_type == "task" {
        if let Some(goal_slug) = managed_meta
            .get("temper-goal")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            if let Some(target) = TargetRef::parse(goal_slug) {
                edges.push((EdgeType::ParentOf, target));
            }
        }
    }

    edges
}

/// Top-level entry point for the CREATE path: extract edge declarations from
/// open_meta, resolve targets, upsert resolved edges, and defer unresolved ones.
///
/// Returns `(created, deferred)` counts.
pub async fn extract_and_upsert_edges(
    pool: &PgPool,
    profile_id: &ProfileId,
    context_id: &ContextId,
    resource_id: &ResourceId,
    doc_type: &str,
    managed_meta: &serde_json::Value,
    open_meta: &serde_json::Value,
) -> ApiResult<(usize, usize)> {
    let declarations = extract_declarations_from_resource(doc_type, managed_meta, open_meta);
    if declarations.is_empty() {
        return Ok((0, 0));
    }

    let (resolved, unresolved) =
        resolve_declarations(pool, profile_id, context_id, resource_id, &declarations).await?;

    let created = upsert_edges(pool, &resolved, profile_id).await?;
    let deferred = defer_edges(pool, resource_id, context_id, profile_id, &unresolved).await?;

    tracing::info!(
        resource_id = %resource_id,
        created,
        deferred,
        "extracted and upserted edges from open_meta"
    );

    Ok((created, deferred))
}

/// Top-level entry point for the UPDATE path: extract new declarations from
/// open_meta and reconcile with existing frontmatter-provenance edges.
///
/// - Edges in new but not existing → added
/// - Edges in existing but not new → removed
/// - Edges in both → unchanged
/// - Manual edges (non-frontmatter provenance) are untouched
pub async fn reconcile_edges(
    pool: &PgPool,
    profile_id: &ProfileId,
    context_id: &ContextId,
    resource_id: &ResourceId,
    doc_type: &str,
    managed_meta: &serde_json::Value,
    open_meta: &serde_json::Value,
) -> ApiResult<EdgeReconciliation> {
    let declarations = extract_declarations_from_resource(doc_type, managed_meta, open_meta);

    // Fetch existing frontmatter-provenance edges where this resource is source
    let outgoing: Vec<ExistingEdgeRow> = sqlx::query_as::<_, ExistingEdgeRow>(
        "SELECT id, source_resource_id, target_resource_id, edge_type::TEXT AS edge_type
           FROM kb_resource_edges
          WHERE source_resource_id = $1
            AND metadata->>'provenance' = 'frontmatter'",
    )
    .bind(**resource_id)
    .fetch_all(pool)
    .await?;

    // Fetch ParentOf edges where this resource is the child (target)
    let incoming_parent: Vec<ExistingEdgeRow> = sqlx::query_as::<_, ExistingEdgeRow>(
        "SELECT id, source_resource_id, target_resource_id, edge_type::TEXT AS edge_type
           FROM kb_resource_edges
          WHERE target_resource_id = $1
            AND edge_type::TEXT = 'parent_of'
            AND metadata->>'provenance' = 'frontmatter'",
    )
    .bind(**resource_id)
    .fetch_all(pool)
    .await?;

    // Combine existing edges into a lookup: (source, target, edge_type) → edge_id
    let mut existing_map: std::collections::HashMap<(Uuid, Uuid, String), Uuid> =
        std::collections::HashMap::new();
    for row in outgoing.iter().chain(incoming_parent.iter()) {
        existing_map.insert(
            (
                row.source_resource_id,
                row.target_resource_id,
                row.edge_type.clone(),
            ),
            row.id,
        );
    }

    // Resolve new declarations
    let (resolved, unresolved) =
        resolve_declarations(pool, profile_id, context_id, resource_id, &declarations).await?;

    // Build the new set for diffing: (source, target, edge_type_str)
    let new_set: std::collections::HashSet<(Uuid, Uuid, String)> = resolved
        .iter()
        .map(|e| {
            (
                e.source_resource_id,
                e.target_resource_id,
                e.edge_type.to_string(),
            )
        })
        .collect();

    let existing_set: std::collections::HashSet<(Uuid, Uuid, String)> =
        existing_map.keys().cloned().collect();

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
        .difference(&new_set)
        .filter_map(|key| existing_map.get(key).copied())
        .collect();

    let unchanged = existing_set.intersection(&new_set).count();

    // Batch additions and removals into one round-trip each.
    let added = to_add.len();
    if added > 0 {
        let additions: Vec<ResolvedEdge> = to_add.iter().map(|&e| e.clone()).collect();
        upsert_edges(pool, &additions, profile_id).await?;
    }

    let removed = to_remove.len();
    if removed > 0 {
        sqlx::query("DELETE FROM kb_resource_edges WHERE id = ANY($1::uuid[])")
            .bind(&to_remove)
            .execute(pool)
            .await?;
    }

    // Clear old deferred edges for this source and store new unresolved
    sqlx::query("DELETE FROM kb_deferred_edges WHERE source_resource_id = $1")
        .bind(**resource_id)
        .execute(pool)
        .await?;

    let deferred = defer_edges(pool, resource_id, context_id, profile_id, &unresolved).await?;

    tracing::info!(
        resource_id = %resource_id,
        added,
        removed,
        unchanged,
        deferred,
        "reconciled edges from open_meta update"
    );

    Ok(EdgeReconciliation {
        added,
        removed,
        unchanged,
        deferred,
    })
}

/// Internal row type for existing edge queries.
#[derive(Debug, sqlx::FromRow)]
struct ExistingEdgeRow {
    id: Uuid,
    source_resource_id: Uuid,
    target_resource_id: Uuid,
    edge_type: String,
}

// ─── Listing ────────────────────────────────────────────────────────────────

/// List all edges connected to a resource, checking visibility.
///
/// Combines the visibility gate and the `graph_resource_edges` fetch into a
/// single round-trip via `LEFT JOIN LATERAL`. The outer subquery always
/// returns at least one row carrying the visibility flag, so we can still
/// distinguish NotFound (resource invisible to caller) from a visible
/// resource that happens to have no edges.
pub async fn list_resource_edges(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
) -> ApiResult<Vec<temper_core::types::graph::GraphEdgeRow>> {
    use crate::error::ApiError;
    use temper_core::types::graph::{EdgeType, GraphEdgeRow};

    let rows = sqlx::query!(
        r#"
        SELECT
            v.is_visible                 AS "is_visible!: bool",
            ge.edge_id                   AS "edge_id?: Uuid",
            ge.peer_resource_id          AS "peer_resource_id?: Uuid",
            ge.peer_title                AS "peer_title?: String",
            ge.peer_slug                 AS "peer_slug?: String",
            ge.edge_type                 AS "edge_type?: EdgeType",
            ge.direction                 AS "direction?: String",
            ge.weight                    AS "weight?: f64",
            ge.metadata                  AS "metadata?: serde_json::Value",
            ge.created                   AS "created?: chrono::DateTime<chrono::Utc>"
          FROM (
              SELECT EXISTS (
                  SELECT 1 FROM resources_visible_to($1, NULL, ARRAY[$2]::uuid[]) rv
                   WHERE rv.resource_id = $2
              ) AS is_visible
          ) v
          LEFT JOIN LATERAL graph_resource_edges($1, $2) ge ON v.is_visible
        "#,
        profile_id,
        resource_id,
    )
    .fetch_all(pool)
    .await?;

    if rows.first().is_none_or(|r| !r.is_visible) {
        return Err(ApiError::NotFound);
    }

    // Each visible-but-edgeless resource still produces one sentinel row
    // from the LEFT JOIN where every edge column is NULL; the filter_map
    // drops those and keeps only real edges.
    let edges = rows
        .into_iter()
        .filter_map(|r| {
            Some(GraphEdgeRow {
                edge_id: r.edge_id?,
                peer_resource_id: r.peer_resource_id?,
                peer_title: r.peer_title?,
                peer_slug: r.peer_slug?,
                edge_type: r.edge_type?,
                direction: r.direction?,
                weight: r.weight?,
                metadata: r.metadata?,
                created: r.created?,
            })
        })
        .collect();

    Ok(edges)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use temper_core::types::graph::EdgeType;

    #[test]
    fn extract_empty_object() {
        let decls = extract_declarations_from_resource("task", &json!({}), &json!({}));
        assert!(decls.is_empty());
    }

    #[test]
    fn extract_null_value() {
        let decls =
            extract_declarations_from_resource("task", &json!({}), &serde_json::Value::Null);
        assert!(decls.is_empty());
    }

    #[test]
    fn extract_no_relationship_fields() {
        let meta = json!({"custom_field": "value", "another": 42});
        let decls = extract_declarations_from_resource("task", &json!({}), &meta);
        assert!(decls.is_empty());
    }

    #[test]
    fn extract_single_extends() {
        let meta = json!({"extends": ["some-slug"]});
        let decls = extract_declarations_from_resource("task", &json!({}), &meta);
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
        let decls = extract_declarations_from_resource("task", &json!({}), &meta);
        // extends: 1, depends_on: 2, references: 1 (UUID) = 4
        assert_eq!(decls.len(), 4);
    }

    #[test]
    fn extract_skips_urls() {
        let meta = json!({
            "references": ["https://example.com", "valid-slug"]
        });
        let decls = extract_declarations_from_resource("task", &json!({}), &meta);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].1, TargetRef::Slug("valid-slug".to_string()));
    }

    #[test]
    fn extract_parent_produces_parent_of() {
        let meta = json!({"parent": "parent-slug"});
        let decls = extract_declarations_from_resource("task", &json!({}), &meta);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].0, EdgeType::ParentOf);
    }

    #[test]
    fn extract_task_with_temper_goal_produces_parent_edge() {
        let managed = json!({"temper-goal": "some-goal"});
        let open = json!({});
        let decls = extract_declarations_from_resource("task", &managed, &open);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].0, EdgeType::ParentOf);
        assert_eq!(decls[0].1, TargetRef::Slug("some-goal".to_string()));
    }

    #[test]
    fn extract_non_task_with_temper_goal_ignores_it() {
        let managed = json!({"temper-goal": "some-goal"});
        let open = json!({});
        let decls = extract_declarations_from_resource("research", &managed, &open);
        assert!(decls.is_empty(), "only tasks emit temper-goal → parent_of");
    }

    #[test]
    fn extract_task_without_temper_goal_produces_no_edge() {
        let managed = json!({});
        let open = json!({});
        let decls = extract_declarations_from_resource("task", &managed, &open);
        assert!(decls.is_empty());
    }

    #[test]
    fn extract_task_with_temper_goal_and_open_meta_refs_combines_both() {
        let managed = json!({"temper-goal": "some-goal"});
        let open = json!({"relates_to": ["other-task"]});
        let decls = extract_declarations_from_resource("task", &managed, &open);
        assert_eq!(decls.len(), 2);
        assert!(decls.iter().any(|(t, _)| *t == EdgeType::ParentOf));
        assert!(decls.iter().any(|(t, _)| *t == EdgeType::RelatesTo));
    }

    #[test]
    fn extract_task_with_empty_temper_goal_string_produces_no_edge() {
        let managed = json!({"temper-goal": ""});
        let open = json!({});
        let decls = extract_declarations_from_resource("task", &managed, &open);
        assert!(decls.is_empty(), "empty string is not a valid goal slug");
    }
}
