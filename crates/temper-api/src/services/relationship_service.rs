//! Relationship service — appends `relationship_*` events to the ledger and
//! projects their edge deltas into `kb_resource_edges` within one transaction.
//!
//! The ledger is truth; `kb_resource_edges` is a rebuildable projection.
//! `apply_relationship_event` does the incremental delta; `rebuild_edge_projection`
//! replays the whole stream. See the limb-1 design spec.

use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::relationship_events::{
    RelationshipAsserted, RelationshipFolded, RelationshipRetyped, RelationshipReweighted,
    TargetEndpoint,
};
use temper_events::ledger::append_event_tx;
use temper_events::types::event::{Event, EventToWrite, EventType};

/// Topic UUIDs seeded by migration 20260522100001.
pub const TOPIC_DECLARATION: &str = "019e3d6f-2300-7000-8000-000000000050";
pub const TOPIC_DEFORMATION: &str = "019e3d6f-2300-7000-8000-000000000051";
pub const TOPIC_JUDGMENT: &str = "019e3d6f-2300-7000-8000-000000000052";

/// Look up a resource's UUID by its slug within a context.
///
/// Scoped to the given context_id so target resolution stays within the
/// same namespace as the source resource. Returns `None` when no active
/// resource with that slug exists in the context.
///
/// SQL lives here per "service layer owns SQL" rule.
pub async fn find_slug_in_context(
    tx: &mut sqlx::PgConnection,
    context_id: Uuid,
    slug: &str,
) -> ApiResult<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        r#"
        SELECT id AS "id!: Uuid"
          FROM kb_resources
         WHERE kb_context_id = $1
           AND slug = $2
           AND is_active
         LIMIT 1
        "#,
        context_id,
        slug,
    )
    .fetch_optional(&mut *tx)
    .await?;
    Ok(id)
}

/// Pool-accepting variant of [`find_slug_in_context`] for pre-transaction
/// read paths (e.g. the active-edge detection check in `assert_relationship`).
pub async fn find_slug_in_context_pool(
    pool: &sqlx::PgPool,
    context_id: Uuid,
    slug: &str,
) -> ApiResult<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        r#"
        SELECT id AS "id!: Uuid"
          FROM kb_resources
         WHERE kb_context_id = $1
           AND slug = $2
           AND is_active
         LIMIT 1
        "#,
        context_id,
        slug,
    )
    .fetch_optional(pool)
    .await?;
    Ok(id)
}

/// Minimal row shape for auth + retype lookup from `kb_resource_edges`.
pub struct EdgeAuthRow {
    pub source_resource_id: Uuid,
    pub label: String,
}

/// Minimal row shape for checking whether an active edge already exists,
/// returned by [`find_active_edge`].
pub struct ActiveEdgeRow {
    /// The existing correlation chain root (== `asserted_by_event_id` of the
    /// edge row, which is the id of the original `relationship_asserted` event).
    pub correlation_id: Uuid,
    /// Whether the edge has been folded (retracted).
    pub is_folded: bool,
}

/// Look up an existing edge by its natural unique key
/// `(source_resource_id, target_resource_id, edge_kind, label, polarity)`.
///
/// Returns `None` when no matching edge row exists (i.e. this would be a
/// fresh assertion). Returns `Some(ActiveEdgeRow)` when the row exists,
/// regardless of `is_folded` state — the caller decides whether to divert
/// or proceed.
///
/// SQL lives here per "service layer owns SQL" rule.
pub async fn find_active_edge(
    pool: &sqlx::PgPool,
    source_resource_id: Uuid,
    target_resource_id: Uuid,
    edge_kind: temper_core::types::graph::EdgeKind,
    label: &str,
    polarity: temper_core::types::graph::Polarity,
) -> ApiResult<Option<ActiveEdgeRow>> {
    let row = sqlx::query!(
        r#"
        SELECT asserted_by_event_id AS "correlation_id!: Uuid",
               is_folded            AS "is_folded!"
          FROM kb_resource_edges
         WHERE source_resource_id = $1
           AND target_resource_id = $2
           AND edge_kind          = $3
           AND label              = $4
           AND polarity           = $5
         LIMIT 1
        "#,
        source_resource_id,
        target_resource_id,
        edge_kind as temper_core::types::graph::EdgeKind,
        label,
        polarity as temper_core::types::graph::Polarity,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| ActiveEdgeRow {
        correlation_id: r.correlation_id,
        is_folded: r.is_folded,
    }))
}

/// Look up the `source_resource_id` and current `label` for an edge identified
/// by its `asserted_by_event_id` (== `correlation_id` of the root assertion
/// event).
///
/// Returns `NotFound` when no matching active edge row exists.
pub async fn edge_auth_row(pool: &sqlx::PgPool, correlation_id: Uuid) -> ApiResult<EdgeAuthRow> {
    let row = sqlx::query!(
        r#"
        SELECT source_resource_id AS "source_resource_id!: Uuid",
               label              AS "label!: String"
          FROM kb_resource_edges
         WHERE asserted_by_event_id = $1
         LIMIT 1
        "#,
        correlation_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;
    Ok(EdgeAuthRow {
        source_resource_id: row.source_resource_id,
        label: row.label,
    })
}

/// Append a relationship event (already built as `EventToWrite`) and project
/// its edge delta into `kb_resource_edges` — all within the caller's
/// transaction.
///
/// The `intent = "explicit"` metadata must already be set on `write` by the
/// caller before passing it here.
pub async fn append_and_project(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    write: EventToWrite,
    event_type: EventType,
) -> ApiResult<Event> {
    let event = append_event_tx(tx, write)
        .await
        .map_err(|e| ApiError::Internal(format!("append_event_tx: {e}")))?;
    apply_relationship_event(&mut *tx, &event, event_type).await?;
    Ok(event)
}

/// Validation: a relationship label must be non-empty. The mandatory-label
/// rule stops `near` (and every kind) becoming a vague catch-all. An empty or
/// whitespace-only label is rejected for every kind.
pub fn validate_assertion_label(kind: EdgeKind, label: &str) -> Result<(), String> {
    let trimmed = label.trim();
    if trimmed.is_empty() {
        return Err("relationship label must be non-empty".to_string());
    }
    let _ = kind; // kind-specific banned-generic-label checks may tighten later
    Ok(())
}

/// Apply one relationship event's delta to `kb_resource_edges`.
///
/// Runs on the caller's transaction/connection so it commits atomically with
/// the ledger append. Called both from the live edge-service write path
/// (where the event was just appended) and from `rebuild_edge_projection`
/// (where events are replayed in ledger order).
///
/// - `RelationshipAsserted`: upsert an edge row keyed by the unique constraint
///   `uq_resource_edge`. If the target is a `Slug` that does not resolve to a
///   visible resource, projection is skipped — pending-target re-projection is
///   Task 13's scope.
/// - `RelationshipRetyped`: UPDATE the row whose `asserted_by_event_id` equals
///   the event's `correlation_id`, updating `edge_kind` and `polarity`.
/// - `RelationshipReweighted`: UPDATE weight on the correlated row.
/// - `RelationshipFolded`: set `is_folded = true` on the correlated row.
/// - `RelationshipDecayed | RelationshipCorrected`: no-op (phase-4 mechanics).
/// - Non-relationship variants: no-op (callers should not pass them, but
///   silently ignoring is safer than panicking in a projection replay).
pub async fn apply_relationship_event(
    tx: &mut sqlx::PgConnection,
    event: &Event,
    event_type: EventType,
) -> ApiResult<()> {
    match event_type {
        EventType::RelationshipAsserted => {
            let payload: RelationshipAsserted = serde_json::from_value(event.payload.clone())
                .map_err(|e| {
                    ApiError::Internal(format!("deserialize RelationshipAsserted payload: {e}"))
                })?;

            let target_resource_id = match &payload.target {
                TargetEndpoint::Resource(id) => *id,
                TargetEndpoint::Slug(slug) => {
                    // Resolve the slug within the source resource's own context —
                    // same scoping as the live-write path — so that drop+replay
                    // produces the same projection regardless of the emitter's
                    // cross-context visibility.
                    //
                    // Step 1: look up source's context_id. If the source was
                    // deleted after the event was emitted, skip projection.
                    let source_context_id = sqlx::query_scalar!(
                        r#"SELECT kb_context_id AS "kb_context_id!: Uuid"
                             FROM kb_resources
                            WHERE id = $1"#,
                        payload.source_resource_id,
                    )
                    .fetch_optional(&mut *tx)
                    .await?;

                    let Some(source_context_id) = source_context_id else {
                        tracing::debug!(
                            source_id = %payload.source_resource_id,
                            event_id = %event.id,
                            "source resource not found during rebuild — skipping edge projection"
                        );
                        return Ok(());
                    };

                    // Step 2: resolve the slug within the source's context.
                    // TODO(task-13): re-project pending-slug assertions when the
                    // target resource is created.
                    let resolved = find_slug_in_context(&mut *tx, source_context_id, slug).await?;

                    match resolved {
                        Some(id) => id,
                        None => {
                            tracing::debug!(
                                slug = %slug,
                                event_id = %event.id,
                                "slug target unresolved — skipping edge projection (Task 13)"
                            );
                            return Ok(());
                        }
                    }
                }
            };

            let edge_id = Uuid::now_v7();
            sqlx::query!(
                r#"
                INSERT INTO kb_resource_edges (
                    id, source_resource_id, target_resource_id,
                    edge_kind, polarity, label, weight,
                    asserted_by_event_id, last_event_id, is_folded
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8, false)
                ON CONFLICT ON CONSTRAINT uq_resource_edge
                DO UPDATE SET
                    -- Re-asserting a folded edge starts a new correlation chain;
                    -- ownership transfers to the new assertion event.
                    asserted_by_event_id = EXCLUDED.asserted_by_event_id,
                    weight               = EXCLUDED.weight,
                    last_event_id        = EXCLUDED.last_event_id,
                    is_folded            = false,
                    updated              = now()
                "#,
                edge_id,
                payload.source_resource_id,
                target_resource_id,
                payload.edge_kind as EdgeKind,
                payload.polarity as Polarity,
                payload.label,
                payload.weight,
                event.id,
            )
            .execute(&mut *tx)
            .await?;
        }

        EventType::RelationshipRetyped => {
            let payload: RelationshipRetyped = serde_json::from_value(event.payload.clone())
                .map_err(|e| {
                    ApiError::Internal(format!("deserialize RelationshipRetyped payload: {e}"))
                })?;

            // The asserted event's id == correlation_id (it is the root), so
            // asserted_by_event_id IS the correlation_id.
            sqlx::query!(
                r#"
                UPDATE kb_resource_edges
                   SET edge_kind     = $2,
                       polarity      = $3,
                       last_event_id = $4,
                       updated       = now()
                 WHERE asserted_by_event_id = $1
                "#,
                event.correlation_id,
                payload.edge_kind as EdgeKind,
                payload.polarity as Polarity,
                event.id,
            )
            .execute(&mut *tx)
            .await?;
            // Note: edge_kind+polarity are part of the unique constraint, so a
            // retype could conflict if another row exists with the new type.
            // For now the UPDATE proceeds; if uniqueness conflicts arise,
            // surface them as DONE_WITH_CONCERNS. (Plan accepts this loosely.)
        }

        EventType::RelationshipReweighted => {
            let payload: RelationshipReweighted = serde_json::from_value(event.payload.clone())
                .map_err(|e| {
                    ApiError::Internal(format!("deserialize RelationshipReweighted payload: {e}"))
                })?;

            sqlx::query!(
                r#"
                UPDATE kb_resource_edges
                   SET weight        = $2,
                       last_event_id = $3,
                       updated       = now()
                 WHERE asserted_by_event_id = $1
                "#,
                event.correlation_id,
                payload.weight,
                event.id,
            )
            .execute(&mut *tx)
            .await?;
        }

        EventType::RelationshipFolded => {
            let _payload: RelationshipFolded = serde_json::from_value(event.payload.clone())
                .map_err(|e| {
                    ApiError::Internal(format!("deserialize RelationshipFolded payload: {e}"))
                })?;

            sqlx::query!(
                r#"
                UPDATE kb_resource_edges
                   SET is_folded     = true,
                       last_event_id = $2,
                       updated       = now()
                 WHERE asserted_by_event_id = $1
                "#,
                event.correlation_id,
                event.id,
            )
            .execute(&mut *tx)
            .await?;
        }

        EventType::RelationshipDecayed | EventType::RelationshipCorrected => {
            // TODO: phase-4 mechanics — decay / correction not yet implemented.
            // See limb-1 design spec, phases 4+.
        }

        EventType::ConceptCreated | EventType::ConceptMutated => {
            // Non-relationship events — not expected here; no-op for safety.
        }
    }

    Ok(())
}

/// After a resource is created, project any `relationship_asserted` event whose
/// slug target now resolves to it. The event-sourced replacement for the
/// retired `kb_deferred_edges` holding table.
///
/// Returns the number of events that were re-projected (i.e. produced a new
/// edge row). Calling this function is idempotent — `apply_relationship_event`
/// uses `ON CONFLICT … DO UPDATE` so re-running against already-projected
/// events is safe.
///
/// **Context scoping:** Only events whose source resource lives in the same
/// context (`new_context_id`) as the newly-created target resource are
/// eligible. This matches the invariant established by the live-write path,
/// where slug resolution is scoped to the source's own context. Filtering at
/// SQL avoids fetching events for sources in other contexts only to skip them
/// inside `apply_relationship_event`.
///
/// **Connection:** The caller supplies `&mut PgConnection` rather than
/// `&PgPool` so the re-projection can be enlisted in the caller's
/// transaction. `ingest_service::ingest` opens a fresh short-lived
/// transaction just for this call, immediately after the resource row is
/// committed.
///
/// **Signature deviation from plan:** The plan omitted `new_context_id`. It
/// was added as an explicit parameter because (a) the caller already has it
/// in scope and (b) it avoids an extra `SELECT kb_context_id …` round-trip.
pub async fn reproject_pending_for_resource(
    tx: &mut sqlx::PgConnection,
    new_resource_id: Uuid,
    new_slug: &str,
    new_context_id: Uuid,
) -> ApiResult<usize> {
    // Fetch all relationship_asserted events whose target is Slug(new_slug)
    // and whose source resource lives in the same context as the new resource.
    // We filter at SQL to avoid loading cross-context events, then call
    // apply_relationship_event on each (which handles the upsert atomically).
    let rows = sqlx::query!(
        r#"
        SELECT
            e.id              AS "id!: Uuid",
            e.event_type_id   AS "event_type_id!: Uuid",
            e.profile_id      AS "emitter_profile_id!: Uuid",
            e.topic_id        AS "topic_id!: Uuid",
            e.scope_id        AS "scope_id!: Uuid",
            e.payload         AS "payload!",
            e.metadata        AS "metadata!",
            e.references      AS "references!",
            e.correlation_id  AS "correlation_id!: Uuid",
            e.occurred_at     AS "occurred_at!",
            e.created         AS "recorded_at!"
          FROM kb_events e
          JOIN kb_event_types et ON et.id = e.event_type_id
          JOIN kb_resources src ON src.id = (e.payload->>'source_resource_id')::uuid
         WHERE et.name = 'relationship_asserted'
           AND e.payload->'target'->>'kind' = 'slug'
           AND e.payload->'target'->>'value' = $1
           AND src.kb_context_id = $2
           AND src.is_active
         ORDER BY e.occurred_at ASC, e.id ASC
        "#,
        new_slug,
        new_context_id,
    )
    .fetch_all(&mut *tx)
    .await?;

    let mut projected: usize = 0;

    for r in rows {
        // Patch the payload so the Slug target resolves to the concrete UUID.
        // We do this by rewriting the target field before calling
        // apply_relationship_event, which keeps all the edge-upsert logic in
        // one place (no duplication of the INSERT … ON CONFLICT block).
        let mut payload = r.payload.clone();
        payload["target"] = serde_json::json!({
            "kind": "resource",
            "value": new_resource_id
        });

        let event = Event {
            id: r.id,
            event_type_id: r.event_type_id,
            emitter_profile_id: r.emitter_profile_id,
            topic_id: r.topic_id,
            scope_id: r.scope_id,
            payload,
            metadata: r.metadata,
            references: r.references,
            correlation_id: r.correlation_id,
            occurred_at: r.occurred_at,
            recorded_at: r.recorded_at,
        };

        apply_relationship_event(&mut *tx, &event, EventType::RelationshipAsserted).await?;
        projected += 1;

        tracing::debug!(
            event_id = %event.id,
            source_id = %event.payload["source_resource_id"],
            new_resource_id = %new_resource_id,
            slug = new_slug,
            "re-projected pending slug assertion"
        );
    }

    Ok(projected)
}

/// Row shape for the event replay in `rebuild_edge_projection`.
struct LedgerEventRow {
    event: Event,
    event_type_name: String,
}

/// Truncate `kb_resource_edges` and replay every `relationship_*` event from
/// `kb_events` in ledger order.
///
/// Idempotent — calling this twice produces the same projection. The
/// validation harness uses it to verify "drop + rebuild == identical snapshot".
pub async fn rebuild_edge_projection(tx: &mut sqlx::PgConnection) -> ApiResult<()> {
    sqlx::query!("TRUNCATE kb_resource_edges")
        .execute(&mut *tx)
        .await?;

    let rows = sqlx::query!(
        r#"
        SELECT
            e.id              AS "id!: Uuid",
            e.event_type_id   AS "event_type_id!: Uuid",
            e.profile_id      AS "emitter_profile_id!: Uuid",
            e.topic_id        AS "topic_id!: Uuid",
            e.scope_id        AS "scope_id!: Uuid",
            e.payload         AS "payload!",
            e.metadata        AS "metadata!",
            e.references      AS "references!",
            e.correlation_id  AS "correlation_id!: Uuid",
            e.occurred_at     AS "occurred_at!",
            e.created         AS "recorded_at!",
            et.name           AS "event_type_name!"
          FROM kb_events e
          JOIN kb_event_types et ON et.id = e.event_type_id
         WHERE et.name IN (
             'relationship_asserted',
             'relationship_retyped',
             'relationship_reweighted',
             'relationship_folded',
             'relationship_decayed',
             'relationship_corrected'
         )
         ORDER BY e.occurred_at ASC, e.id ASC
        "#,
    )
    .fetch_all(&mut *tx)
    .await?;

    let event_rows: Vec<LedgerEventRow> = rows
        .into_iter()
        .map(|r| LedgerEventRow {
            event: Event {
                id: r.id,
                event_type_id: r.event_type_id,
                emitter_profile_id: r.emitter_profile_id,
                topic_id: r.topic_id,
                scope_id: r.scope_id,
                payload: r.payload,
                metadata: r.metadata,
                references: r.references,
                correlation_id: r.correlation_id,
                occurred_at: r.occurred_at,
                recorded_at: r.recorded_at,
            },
            event_type_name: r.event_type_name,
        })
        .collect();

    for row in event_rows {
        let event_type = match row.event_type_name.as_str() {
            "relationship_asserted" => EventType::RelationshipAsserted,
            "relationship_retyped" => EventType::RelationshipRetyped,
            "relationship_reweighted" => EventType::RelationshipReweighted,
            "relationship_folded" => EventType::RelationshipFolded,
            "relationship_decayed" => EventType::RelationshipDecayed,
            "relationship_corrected" => EventType::RelationshipCorrected,
            other => {
                tracing::warn!(
                    event_type = other,
                    "unexpected relationship event type in rebuild — skipping"
                );
                continue;
            }
        };
        apply_relationship_event(&mut *tx, &row.event, event_type).await?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_label_is_rejected() {
        assert!(validate_assertion_label(EdgeKind::Near, "   ").is_err());
        assert!(validate_assertion_label(EdgeKind::Contains, "").is_err());
    }

    #[test]
    fn non_empty_label_is_accepted() {
        assert!(validate_assertion_label(EdgeKind::LeadsTo, "depends_on").is_ok());
    }
}
