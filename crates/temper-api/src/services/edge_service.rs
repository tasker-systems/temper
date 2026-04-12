//! Edge service — extracts relationship declarations from frontmatter,
//! resolves targets, and manages edges in `kb_resource_edges`.
//!
//! All SQL lives here per the "service layer owns SQL" rule.

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;
use temper_core::types::graph::{
    EdgeReconciliation, EdgeType, ResolvedEdge, ResourceRelationships, TargetRef,
};
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};

// ─── Target Resolution ──────────────────────────────────────────────────────

/// Resolve a single `TargetRef` to a resource UUID visible to the given profile.
///
/// - `TargetRef::Id(uuid)` — direct lookup against `kb_resources.id`
/// - `TargetRef::Slug(slug)` — same-context match first, then cross-context.
///   Must resolve to exactly one visible resource; ambiguous matches return None.
///
/// Returns `None` for unresolvable forward references.
pub async fn resolve_target(
    pool: &PgPool,
    profile_id: &ProfileId,
    context_id: &ContextId,
    target: &TargetRef,
) -> ApiResult<Option<Uuid>> {
    match target {
        TargetRef::Id(uuid) => {
            // Direct UUID lookup scoped to visibility
            let row = sqlx::query_scalar::<_, Uuid>(
                "SELECT r.id
                   FROM kb_resources r
                   JOIN resources_visible_to($1, NULL, '{}') rv ON rv.resource_id = r.id
                  WHERE r.id = $2",
            )
            .bind(**profile_id)
            .bind(uuid)
            .fetch_optional(pool)
            .await?;
            Ok(row)
        }
        TargetRef::Slug(slug) => {
            // Try same-context first
            let same_ctx = sqlx::query_scalar::<_, Uuid>(
                "SELECT r.id
                   FROM kb_resources r
                   JOIN resources_visible_to($1, NULL, '{}') rv ON rv.resource_id = r.id
                  WHERE r.slug = $2
                    AND r.kb_context_id = $3",
            )
            .bind(**profile_id)
            .bind(slug)
            .bind(**context_id)
            .fetch_optional(pool)
            .await?;

            if let Some(id) = same_ctx {
                return Ok(Some(id));
            }

            // Fall back to cross-context, but require exactly one match
            let cross_ctx: Vec<Uuid> = sqlx::query_scalar::<_, Uuid>(
                "SELECT r.id
                   FROM kb_resources r
                   JOIN resources_visible_to($1, NULL, '{}') rv ON rv.resource_id = r.id
                  WHERE r.slug = $2
                    AND r.kb_context_id != $3",
            )
            .bind(**profile_id)
            .bind(slug)
            .bind(**context_id)
            .fetch_all(pool)
            .await?;

            match cross_ctx.len() {
                0 => Ok(None),
                1 => Ok(Some(cross_ctx[0])),
                n => {
                    tracing::warn!(
                        slug = %slug,
                        matches = n,
                        "ambiguous slug reference — resolved to multiple visible resources, skipping"
                    );
                    Ok(None)
                }
            }
        }
    }
}

/// Resolve a batch of `(EdgeType, TargetRef)` declarations into `ResolvedEdge`s.
///
/// For `ParentOf` edges the direction is reversed: the resolved target becomes
/// the source (parent) and the declaring resource becomes the target (child).
///
/// Returns `(resolved, unresolved)` — unresolved refs are forward references
/// that should be deferred.
pub async fn resolve_declarations(
    pool: &PgPool,
    profile_id: &ProfileId,
    context_id: &ContextId,
    resource_id: &ResourceId,
    declarations: &[(EdgeType, TargetRef)],
) -> ApiResult<(Vec<ResolvedEdge>, Vec<(EdgeType, TargetRef)>)> {
    let mut resolved = Vec::new();
    let mut unresolved = Vec::new();

    for (edge_type, target) in declarations {
        match resolve_target(pool, profile_id, context_id, target).await? {
            Some(target_uuid) => {
                // Determine source/target based on edge type
                let (source, dest) = if *edge_type == EdgeType::ParentOf {
                    // ParentOf is reversed: resolved target is the parent (source),
                    // declaring resource is the child (target)
                    (target_uuid, **resource_id)
                } else {
                    (**resource_id, target_uuid)
                };

                // Skip self-edges
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
            None => {
                unresolved.push((*edge_type, target.clone()));
            }
        }
    }

    Ok((resolved, unresolved))
}

// ─── Upsert & Defer ─────────────────────────────────────────────────────────

/// Insert or update a single edge. Uses ON CONFLICT to merge metadata.
pub async fn upsert_edge(
    pool: &PgPool,
    edge: &ResolvedEdge,
    profile_id: &ProfileId,
) -> ApiResult<()> {
    // Runtime query because of edge_type::edge_type cast
    sqlx::query(
        "INSERT INTO kb_resource_edges (id, source_resource_id, target_resource_id, edge_type, weight, metadata, created_by_profile_id)
         VALUES ($1, $2, $3, $4::edge_type, $5, $6, $7)
         ON CONFLICT ON CONSTRAINT uq_resource_edge
         DO UPDATE SET weight = EXCLUDED.weight,
                       metadata = kb_resource_edges.metadata || EXCLUDED.metadata,
                       updated = now()",
    )
    .bind(Uuid::now_v7())
    .bind(edge.source_resource_id)
    .bind(edge.target_resource_id)
    .bind(edge.edge_type.to_string())
    .bind(edge.weight)
    .bind(&edge.metadata)
    .bind(**profile_id)
    .execute(pool)
    .await?;

    Ok(())
}

/// Upsert a batch of resolved edges. Returns count of edges upserted.
pub async fn upsert_edges(
    pool: &PgPool,
    edges: &[ResolvedEdge],
    profile_id: &ProfileId,
) -> ApiResult<usize> {
    for edge in edges {
        upsert_edge(pool, edge, profile_id).await?;
    }
    Ok(edges.len())
}

/// Store unresolved target references in `kb_deferred_edges` for later resolution.
pub async fn defer_edges(
    pool: &PgPool,
    resource_id: &ResourceId,
    context_id: &ContextId,
    profile_id: &ProfileId,
    unresolved: &[(EdgeType, TargetRef)],
) -> ApiResult<usize> {
    for (edge_type, target) in unresolved {
        let target_ref_str = match target {
            TargetRef::Id(uuid) => uuid.to_string(),
            TargetRef::Slug(slug) => slug.clone(),
        };

        sqlx::query(
            "INSERT INTO kb_deferred_edges (id, source_resource_id, target_ref, target_context_id, edge_type, weight, metadata, created_by_profile_id)
             VALUES ($1, $2, $3, $4, $5::edge_type, $6, $7, $8)",
        )
        .bind(Uuid::now_v7())
        .bind(**resource_id)
        .bind(&target_ref_str)
        .bind(**context_id)
        .bind(edge_type.to_string())
        .bind(1.0_f64)
        .bind(serde_json::json!({"provenance": "frontmatter"}))
        .bind(**profile_id)
        .execute(pool)
        .await?;
    }

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

    let mut resolved_count = 0usize;

    for row in &rows {
        // Parse edge_type from stored TEXT
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

        // Determine direction: ParentOf means the deferred source declared a parent,
        // so the new resource is the parent (source) and the declaring resource is the child (target)
        let (source, target) = if edge_type == EdgeType::ParentOf {
            (**new_resource_id, row.source_resource_id)
        } else {
            (row.source_resource_id, **new_resource_id)
        };

        // Skip self-edges
        if source == target {
            tracing::warn!(
                deferred_id = %row.id,
                resource_id = %source,
                "deferred edge would create self-edge, skipping"
            );
            continue;
        }

        let edge = ResolvedEdge {
            source_resource_id: source,
            target_resource_id: target,
            edge_type,
            weight: row.weight,
            metadata: row.metadata.clone(),
        };

        upsert_edge(pool, &edge, profile_id).await?;

        // Delete the resolved deferred edge
        sqlx::query("DELETE FROM kb_deferred_edges WHERE id = $1")
            .bind(row.id)
            .execute(pool)
            .await?;

        resolved_count += 1;
    }

    if resolved_count > 0 {
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

/// Extract edge declarations from the `open_meta` JSON value.
///
/// Pure function — no database access. Deserializes as `ResourceRelationships`
/// (unknown fields are ignored by serde) and converts to `(EdgeType, TargetRef)` pairs.
pub fn extract_declarations_from_open_meta(
    open_meta: &serde_json::Value,
) -> Vec<(EdgeType, TargetRef)> {
    if !open_meta.is_object() {
        return Vec::new();
    }

    let rels: ResourceRelationships = match serde_json::from_value(open_meta.clone()) {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(
                error = %e,
                "open_meta did not contain valid relationship fields"
            );
            return Vec::new();
        }
    };

    rels.to_edge_declarations()
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
    open_meta: &serde_json::Value,
) -> ApiResult<(usize, usize)> {
    let declarations = extract_declarations_from_open_meta(open_meta);
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
    open_meta: &serde_json::Value,
) -> ApiResult<EdgeReconciliation> {
    let declarations = extract_declarations_from_open_meta(open_meta);

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

    // Execute additions
    let added = to_add.len();
    for edge in &to_add {
        upsert_edge(pool, edge, profile_id).await?;
    }

    // Execute removals
    let removed = to_remove.len();
    for edge_id in &to_remove {
        sqlx::query("DELETE FROM kb_resource_edges WHERE id = $1")
            .bind(edge_id)
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
pub async fn list_resource_edges(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
) -> ApiResult<Vec<temper_core::types::graph::GraphEdgeRow>> {
    // Verify the resource is visible to this profile
    let _resource =
        crate::services::resource_service::get_visible(pool, profile_id, resource_id).await?;

    let rows = sqlx::query_as::<_, temper_core::types::graph::GraphEdgeRow>(
        "SELECT * FROM graph_resource_edges($1, $2)",
    )
    .bind(profile_id)
    .bind(resource_id)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

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
