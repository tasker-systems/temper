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
use temper_events::types::event::{Event, EventType};

/// Topic UUIDs seeded by migration 20260522100001.
pub const TOPIC_DECLARATION: &str = "019e3d6f-2300-7000-8000-000000000050";
pub const TOPIC_DEFORMATION: &str = "019e3d6f-2300-7000-8000-000000000051";
pub const TOPIC_JUDGMENT: &str = "019e3d6f-2300-7000-8000-000000000052";

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
                    // Attempt to resolve the slug to a resource visible to the
                    // emitter profile. If unresolved, project nothing.
                    // TODO(task-13): re-project pending-slug assertions when the
                    // target resource is created.
                    let resolved = sqlx::query_scalar!(
                        r#"
                        SELECT r.id AS "id!: Uuid"
                          FROM kb_resources r
                          JOIN resources_visible_to($1, NULL, '{}') rv
                            ON rv.resource_id = r.id
                         WHERE r.slug = $2
                           AND r.is_active
                         LIMIT 1
                        "#,
                        event.emitter_profile_id,
                        slug,
                    )
                    .fetch_optional(&mut *tx)
                    .await?;

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
                    weight        = EXCLUDED.weight,
                    last_event_id = EXCLUDED.last_event_id,
                    is_folded     = false,
                    updated       = now()
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
