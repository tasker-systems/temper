use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;
use temper_core::types::element_trail::{ElementEvent, ElementKind, EventTrail};
use temper_core::types::ids::{ContextId, ProfileId};

/// The most recent event id produced against a context the profile can read. Returns `None` when the
/// context has no events the profile may see. Post-collapse events anchor via `producing_anchor` (no
/// `kb_context_id`/`profile_id`/`resource_id` columns); the cursor is the context's own event stream,
/// gated by the canonical context read predicate.
///
/// This gate used to be an inline `EXISTS` over `kb_team_members` — the narrowest of the six copies of
/// the context-read rule that `20260712000010` collapsed. It ignored context shares and explicit
/// read-grants entirely, so a profile who could read a context by either route still got `None` here.
pub async fn latest_event_id_for_context(
    pool: &PgPool,
    profile_id: ProfileId,
    kb_context_id: ContextId,
) -> ApiResult<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        r#"
        SELECT e.id AS "id!: Uuid"
          FROM kb_events e
         WHERE e.producing_anchor_table = 'kb_contexts'
           AND e.producing_anchor_id = $2
           AND context_readable_by_profile($1, $2)
         ORDER BY e.occurred_at DESC
         LIMIT 1
        "#,
        *profile_id,
        *kb_context_id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(id)
}

/// One row from an element-trail SQL function (`element_trail_edge`/`_node`).
/// Both functions share this exact column set.
struct ElementEventRow {
    event_id: Uuid,
    kind: String,
    actor_entity_id: Uuid,
    actor_name: String,
    occurred_at: chrono::DateTime<chrono::Utc>,
    metadata: serde_json::Value,
    payload: serde_json::Value,
}

/// Strip heavy inline fields from a payload before it rides in the trail response.
/// `resource_created` embeds the full `blocks[]` (content) — useless in a trail and
/// potentially large — so drop it; the summary still shows title/doc_type.
fn trim_payload(kind: &str, mut payload: serde_json::Value) -> serde_json::Value {
    if kind == "resource_created" {
        if let Some(obj) = payload.as_object_mut() {
            obj.remove("blocks");
        }
    }
    payload
}

/// R5 element event-trail: the time-ordered history of events for a single node
/// (resource) or edge. Visibility is gated inside the SQL functions — edges enforce
/// `anchor_readable_by_profile(home)` AND `endpoint_readable_by_profile(source/target)`
/// (the full `edges_visible_to` predicate minus the folded filter, so a folded edge
/// still shows its trail); nodes gate via `resources_visible_to`. An
/// unreadable/nonexistent element yields an empty trail rather than an error.
///
/// The two element kinds dispatch to two SEPARATE static `query_as!` calls (not one
/// query with an interpolated function name): each query is compile-time-checked
/// against the schema and its visibility gate is greppable at the call site.
pub async fn element_trail(
    pool: &PgPool,
    profile_id: ProfileId,
    kind: ElementKind,
    element_id: Uuid,
) -> ApiResult<EventTrail> {
    let rows = match kind {
        ElementKind::Edge => {
            sqlx::query_as!(
                ElementEventRow,
                r#"SELECT event_id AS "event_id!", kind AS "kind!",
                          actor_entity_id AS "actor_entity_id!", actor_name AS "actor_name!",
                          occurred_at AS "occurred_at!", metadata AS "metadata!",
                          payload AS "payload!"
                     FROM element_trail_edge($1, $2)"#,
                *profile_id,
                element_id,
            )
            .fetch_all(pool)
            .await?
        }
        ElementKind::Node => {
            sqlx::query_as!(
                ElementEventRow,
                r#"SELECT event_id AS "event_id!", kind AS "kind!",
                          actor_entity_id AS "actor_entity_id!", actor_name AS "actor_name!",
                          occurred_at AS "occurred_at!", metadata AS "metadata!",
                          payload AS "payload!"
                     FROM element_trail_node($1, $2)"#,
                *profile_id,
                element_id,
            )
            .fetch_all(pool)
            .await?
        }
    };

    let events = rows
        .into_iter()
        .map(|row| {
            // metadata is AgentAuthorship-shaped for agent acts, {} for system acts.
            // The band is the bare lowercase string under `confidence` (NOT `confidence_band`).
            let confidence = row
                .metadata
                .get("confidence")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            ElementEvent {
                event_id: row.event_id,
                actor_entity_id: row.actor_entity_id,
                actor_name: row.actor_name,
                occurred_at: row.occurred_at.to_rfc3339(),
                confidence,
                payload: trim_payload(&row.kind, row.payload),
                kind: row.kind,
            }
        })
        .collect();

    Ok(EventTrail {
        element_kind: kind,
        element_id,
        events,
    })
}
