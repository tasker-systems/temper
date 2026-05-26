//! Edge service — extracts relationship declarations from frontmatter,
//! resolves targets, and appends `relationship_*` events that project into
//! `kb_resource_edges`.
//!
//! All SQL lives here per the "service layer owns SQL" rule. Resolution is
//! batched in one DB round-trip; the per-edge writes run inside a single
//! transaction so the event ledger and the projection stay consistent.
//!
//! Provenance is carried on the *event* metadata (`kb_events.metadata->>'intent'`),
//! not on the (now-dropped) `kb_resource_edges.metadata` column. The
//! frontmatter rewire stamps `intent = 'derived'` (the edge was derived from
//! the resource's frontmatter); the migration genesis events use
//! `intent = 'migration'`; fixtures use `intent = 'fixture'`. The JSON key is
//! a placeholder for a future typed `intent` enum column on `kb_events`.

use chrono::Utc;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::error::ApiResult;
use crate::services::relationship_service::apply_relationship_event;
use temper_core::types::graph::{
    EdgeKind, EdgeReconciliation, EdgeType, Polarity, ResolvedEdge, ResourceRelationships,
    TargetRef,
};
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_core::types::relationship_events::{
    RelationshipAsserted, RelationshipFolded, TargetEndpoint,
};
use temper_events::ledger::append_event_tx;
use temper_events::types::event::{EventToWrite, EventType};

// Topic and scope ids seeded by migrations 20260522000001 / 20260522100001.
const DECLARATION_TOPIC_ID: &str = "019e3d6f-2300-7000-8000-000000000050";
const DEFORMATION_TOPIC_ID: &str = "019e3d6f-2300-7000-8000-000000000051";
const PUBLIC_SCOPE_ID: &str = "019e3d6f-2300-7000-8000-000000000010";

/// Event-metadata `intent` value stamped on `relationship_*` events that
/// originate from frontmatter reconciliation — the edge is *derived* from
/// the resource's open_meta declaration set.
const DERIVED_INTENT: &str = "derived";

// ─── Target Resolution ──────────────────────────────────────────────────────

/// Resolve a batch of `(EdgeType, TargetRef)` declarations into `ResolvedEdge`s.
///
/// For `ParentOf` edges the direction is reversed: the resolved target becomes
/// the source (parent) and the declaring resource becomes the target (child).
///
/// Returns `(resolved, unresolved)` — unresolved refs are forward references
/// that the caller will record as slug-target assertion events.
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
                let (edge_kind, polarity, label) = edge_type.legacy_mapping();
                resolved.push(ResolvedEdge {
                    source_resource_id: source,
                    target_resource_id: dest,
                    edge_kind,
                    polarity,
                    label: label.to_string(),
                    weight: 1.0,
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
///     cross-context).
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

// ─── Event emission + projection ────────────────────────────────────────────

fn declaration_topic_id() -> Uuid {
    Uuid::parse_str(DECLARATION_TOPIC_ID).expect("seeded declaration topic UUID parses")
}

fn deformation_topic_id() -> Uuid {
    Uuid::parse_str(DEFORMATION_TOPIC_ID).expect("seeded deformation topic UUID parses")
}

fn public_scope_id() -> Uuid {
    Uuid::parse_str(PUBLIC_SCOPE_ID).expect("seeded public scope UUID parses")
}

fn frontmatter_metadata() -> serde_json::Value {
    // The `metadata` sidecar carries provenance for the assertion event. The
    // `intent` key is the placeholder for a future typed enum column.
    serde_json::json!({ "intent": DERIVED_INTENT })
}

/// Build the `relationship_asserted` payload for a resolved edge.
///
/// Uses the typed `RelationshipAsserted` struct so the wire shape stays
/// compiler-checked against `temper-core::types::relationship_events`.
fn asserted_payload_resource(edge: &ResolvedEdge) -> ApiResult<serde_json::Value> {
    let payload = RelationshipAsserted {
        source_resource_id: edge.source_resource_id,
        target: TargetEndpoint::Resource(edge.target_resource_id),
        edge_kind: edge.edge_kind,
        polarity: edge.polarity,
        label: edge.label.clone(),
        weight: edge.weight,
    };
    serde_json::to_value(&payload)
        .map_err(|e| crate::error::ApiError::Internal(format!("serialize asserted payload: {e}")))
}

/// Build the `relationship_asserted` payload for an unresolved slug target.
///
/// Uses the typed `RelationshipAsserted` struct with a `Slug` target
/// endpoint — the projection builder will resolve it once a resource with
/// the slug becomes visible.
fn asserted_payload_slug(
    source_resource_id: Uuid,
    edge_kind: EdgeKind,
    polarity: Polarity,
    label: &str,
    target_slug: &str,
    weight: f64,
) -> ApiResult<serde_json::Value> {
    let payload = RelationshipAsserted {
        source_resource_id,
        target: TargetEndpoint::Slug(target_slug.to_string()),
        edge_kind,
        polarity,
        label: label.to_string(),
        weight,
    };
    serde_json::to_value(&payload).map_err(|e| {
        crate::error::ApiError::Internal(format!("serialize asserted slug payload: {e}"))
    })
}

/// Append a `relationship_asserted` event inside the given transaction and
/// project its edge delta into `kb_resource_edges` via `apply_relationship_event`.
async fn append_asserted_and_project(
    tx: &mut Transaction<'_, Postgres>,
    profile_id: Uuid,
    payload: serde_json::Value,
    metadata: serde_json::Value,
) -> ApiResult<Uuid> {
    let mut write = EventToWrite::new_root(
        EventType::RelationshipAsserted,
        profile_id,
        declaration_topic_id(),
        public_scope_id(),
        payload,
        Utc::now(),
    );
    write.metadata = metadata;
    let event = append_event_tx(tx, write)
        .await
        .map_err(|e| crate::error::ApiError::Internal(format!("append asserted: {e}")))?;
    apply_relationship_event(tx, &event, EventType::RelationshipAsserted).await?;
    Ok(event.id)
}

/// Append a `relationship_folded` event correlated with the original
/// assertion, inside the given transaction, and project the fold into
/// `kb_resource_edges` via `apply_relationship_event`.
async fn append_folded_and_project(
    tx: &mut Transaction<'_, Postgres>,
    profile_id: Uuid,
    correlation_id: Uuid,
    reason: Option<&str>,
) -> ApiResult<Uuid> {
    let payload_struct = RelationshipFolded {
        reason: reason.map(str::to_string),
    };
    let payload = serde_json::to_value(&payload_struct)
        .map_err(|e| crate::error::ApiError::Internal(format!("serialize folded payload: {e}")))?;
    let mut write = EventToWrite::new_correlated(
        EventType::RelationshipFolded,
        profile_id,
        deformation_topic_id(),
        public_scope_id(),
        payload,
        correlation_id,
        Utc::now(),
    );
    write.metadata = frontmatter_metadata();
    let event = append_event_tx(tx, write)
        .await
        .map_err(|e| crate::error::ApiError::Internal(format!("append folded: {e}")))?;
    apply_relationship_event(tx, &event, EventType::RelationshipFolded).await?;
    Ok(event.id)
}

/// Append `relationship_asserted` events + project edge rows for the supplied
/// resolved edges and slug-target assertions, all in one transaction.
///
/// Each event is appended via `append_asserted_and_project` which delegates
/// the `kb_resource_edges` projection to `apply_relationship_event`, keeping
/// the projection logic in a single authoritative place.
///
/// Returns `(projected, pending)` where:
///   * `projected` is the count of resolved edges whose row exists post-call.
///   * `pending`   is the count of slug-target assertions (forward refs).
async fn assert_edges_tx(
    tx: &mut Transaction<'_, Postgres>,
    profile_id: Uuid,
    resolved: &[ResolvedEdge],
    unresolved_slugs: &[(EdgeType, TargetRef)],
    source_resource_id: Uuid,
) -> ApiResult<(usize, usize)> {
    let metadata = frontmatter_metadata();

    for edge in resolved {
        let payload = asserted_payload_resource(edge)?;
        append_asserted_and_project(tx, profile_id, payload, metadata.clone()).await?;
    }

    let mut pending = 0usize;
    for (edge_type, target) in unresolved_slugs {
        let TargetRef::Slug(slug) = target else {
            // Unresolved UUID — the resource doesn't exist or isn't visible.
            // Record it as a slug assertion using the UUID's text form so the
            // forward-projection step can still find it once visibility opens
            // up. (UUID parse failures upstream filter these; we just preserve.)
            continue;
        };
        let (edge_kind, polarity, label) = edge_type.legacy_mapping();
        let payload =
            asserted_payload_slug(source_resource_id, edge_kind, polarity, label, slug, 1.0)?;
        append_asserted_and_project(tx, profile_id, payload, metadata.clone()).await?;
        pending += 1;
    }

    Ok((resolved.len(), pending))
}

// ─── Extraction & high-level entry points ───────────────────────────────────

/// Extract edge declarations from a resource's full meta.
///
/// Reads relationship fields from `open_meta` (via `ResourceRelationships`)
/// and, for tasks, the `temper-goal` field from `managed_meta` which
/// yields a reversed `ParentOf` edge to the goal resource.
///
/// Pure function — no database access. Unknown fields in either tier are
/// ignored.
pub fn extract_declarations_from_resource(
    doc_type: &str,
    managed_meta: &serde_json::Value,
    open_meta: &serde_json::Value,
) -> Vec<(EdgeType, TargetRef)> {
    let mut edges = Vec::new();

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
/// open_meta, resolve targets, and emit `relationship_asserted` events
/// (projecting resolved targets and recording slug targets as pending).
///
/// Returns `(projected, pending)` counts.
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

    let mut tx = pool.begin().await?;
    let (projected, pending) =
        assert_edges_tx(&mut tx, **profile_id, &resolved, &unresolved, **resource_id).await?;
    tx.commit().await?;

    tracing::info!(
        resource_id = %resource_id,
        projected,
        pending,
        "extracted and asserted edges from open_meta"
    );

    Ok((projected, pending))
}

/// Top-level entry point for the UPDATE path: extract new declarations from
/// open_meta and reconcile with existing frontmatter-asserted edges.
///
/// - Edges in new but not existing → emit `relationship_asserted` + project
/// - Edges in existing but not new → emit `relationship_folded` correlated
///   with the original assertion (the edge row is folded, not deleted)
/// - Edges in both → unchanged
/// - Manual / non-frontmatter edges are untouched (filtered by event source)
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

    // Existing frontmatter-asserted edges: join through the assertion event so
    // we filter on `kb_events.metadata->>'intent' = 'derived'`. The (now
    // dropped) `kb_resource_edges.metadata` column carried this before.
    let outgoing = sqlx::query_as!(
        ExistingEdgeRow,
        r#"
        SELECT e.id                   AS "id!: Uuid",
               e.source_resource_id   AS "source_resource_id!: Uuid",
               e.target_resource_id   AS "target_resource_id!: Uuid",
               e.label                AS "label!: String",
               e.asserted_by_event_id AS "asserted_by_event_id!: Uuid"
          FROM kb_resource_edges e
          JOIN kb_events ev ON ev.id = e.asserted_by_event_id
         WHERE e.source_resource_id = $1
           AND NOT e.is_folded
           AND ev.metadata->>'intent' = $2
        "#,
        **resource_id,
        DERIVED_INTENT,
    )
    .fetch_all(pool)
    .await?;

    let incoming_parent = sqlx::query_as!(
        ExistingEdgeRow,
        r#"
        SELECT e.id                   AS "id!: Uuid",
               e.source_resource_id   AS "source_resource_id!: Uuid",
               e.target_resource_id   AS "target_resource_id!: Uuid",
               e.label                AS "label!: String",
               e.asserted_by_event_id AS "asserted_by_event_id!: Uuid"
          FROM kb_resource_edges e
          JOIN kb_events ev ON ev.id = e.asserted_by_event_id
         WHERE e.target_resource_id = $1
           AND e.label = 'parent_of'
           AND NOT e.is_folded
           AND ev.metadata->>'intent' = $2
        "#,
        **resource_id,
        DERIVED_INTENT,
    )
    .fetch_all(pool)
    .await?;

    // Lookup: (source, target, label) → (edge_id, correlation_event_id)
    let mut existing_map: std::collections::HashMap<(Uuid, Uuid, String), (Uuid, Uuid)> =
        std::collections::HashMap::new();
    for row in outgoing.iter().chain(incoming_parent.iter()) {
        existing_map.insert(
            (
                row.source_resource_id,
                row.target_resource_id,
                row.label.clone(),
            ),
            (row.id, row.asserted_by_event_id),
        );
    }

    let (resolved, unresolved) =
        resolve_declarations(pool, profile_id, context_id, resource_id, &declarations).await?;

    let new_set: std::collections::HashSet<(Uuid, Uuid, String)> = resolved
        .iter()
        .map(|e| (e.source_resource_id, e.target_resource_id, e.label.clone()))
        .collect();

    let existing_set: std::collections::HashSet<(Uuid, Uuid, String)> =
        existing_map.keys().cloned().collect();

    let to_add: Vec<ResolvedEdge> = resolved
        .iter()
        .filter(|e| {
            !existing_set.contains(&(e.source_resource_id, e.target_resource_id, e.label.clone()))
        })
        .cloned()
        .collect();

    let to_fold: Vec<(Uuid, Uuid)> = existing_set
        .difference(&new_set)
        .filter_map(|key| existing_map.get(key).copied())
        .collect();

    let unchanged = existing_set.intersection(&new_set).count();
    let added = to_add.len();
    let removed = to_fold.len();

    let mut tx = pool.begin().await?;

    // Assertions (additions + slug-target forward refs).
    let (_projected, pending) =
        assert_edges_tx(&mut tx, **profile_id, &to_add, &unresolved, **resource_id).await?;

    // Retractions: fold the row + emit a correlated `relationship_folded`.
    for (_edge_id, correlation_event_id) in &to_fold {
        append_folded_and_project(&mut tx, **profile_id, *correlation_event_id, None).await?;
    }

    tx.commit().await?;

    tracing::info!(
        resource_id = %resource_id,
        added,
        removed,
        unchanged,
        pending,
        "reconciled edges from open_meta update"
    );

    Ok(EdgeReconciliation {
        added,
        removed,
        unchanged,
        pending,
    })
}

/// Row shape used to diff existing frontmatter-asserted edges during
/// `reconcile_edges`. The `asserted_by_event_id` is the lifecycle
/// `correlation_id` for any follow-on event (e.g. `relationship_folded`).
#[derive(Debug)]
struct ExistingEdgeRow {
    id: Uuid,
    source_resource_id: Uuid,
    target_resource_id: Uuid,
    label: String,
    asserted_by_event_id: Uuid,
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
    use temper_core::types::graph::GraphEdgeRow;

    let rows = sqlx::query!(
        r#"
        SELECT
            v.is_visible                 AS "is_visible!: bool",
            ge.edge_id                   AS "edge_id?: Uuid",
            ge.peer_resource_id          AS "peer_resource_id?: Uuid",
            ge.peer_title                AS "peer_title?: String",
            ge.peer_slug                 AS "peer_slug?: String",
            ge.edge_kind                 AS "edge_kind?: EdgeKind",
            ge.polarity                  AS "polarity?: Polarity",
            ge.label                     AS "label?: String",
            ge.direction                 AS "direction?: String",
            ge.weight                    AS "weight?: f64",
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

    let edges = rows
        .into_iter()
        .filter_map(|r| {
            Some(GraphEdgeRow {
                edge_id: r.edge_id?,
                peer_resource_id: r.peer_resource_id?,
                peer_title: r.peer_title?,
                peer_slug: r.peer_slug?,
                edge_kind: r.edge_kind?,
                polarity: r.polarity?,
                label: r.label?,
                direction: r.direction?,
                weight: r.weight?,
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
