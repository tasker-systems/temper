use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;
use temper_core::types::element_trail::{ElementEvent, ElementKind, EventTrail};
use temper_core::types::ids::{ContextId, ProfileId};

/// The most recent event id produced against a context the profile owns (directly, or via a team it
/// belongs to). Returns `None` when the context has no events the profile may see. Post-collapse events
/// anchor via `producing_anchor` (no `kb_context_id`/`profile_id`/`resource_id` columns); the cursor is
/// the context's own event stream, gated by context ownership.
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
           AND EXISTS (                                   -- context-ownership gate
             SELECT 1 FROM kb_contexts c
              WHERE c.id = $2 AND (
                (c.owner_table='kb_profiles' AND c.owner_id = $1)
                OR (c.owner_table='kb_teams' AND c.owner_id IN
                     (SELECT team_id FROM kb_team_members WHERE profile_id = $1))))
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

/// R5 element event-trail: the time-ordered history of events for a single node
/// (resource) or edge. Visibility is gated inside the SQL functions
/// (`anchor_readable_by_profile` for edges, `resources_visible_to` for nodes) — an
/// unreadable/nonexistent element yields an empty trail rather than an error.
pub async fn element_trail(
    pool: &PgPool,
    profile_id: ProfileId,
    kind: ElementKind,
    element_id: Uuid,
) -> ApiResult<EventTrail> {
    let fn_name = match kind {
        ElementKind::Edge => "element_trail_edge",
        ElementKind::Node => "element_trail_node",
    };
    let rows = sqlx::query_as::<
        _,
        (
            Uuid,
            String,
            Uuid,
            chrono::DateTime<chrono::Utc>,
            serde_json::Value,
        ),
    >(&format!(
        "SELECT event_id, kind, actor_entity_id, occurred_at, metadata FROM {fn_name}($1, $2)"
    ))
    .bind(profile_id.as_uuid())
    .bind(element_id)
    .fetch_all(pool)
    .await?;

    let events = rows
        .into_iter()
        .map(|(event_id, kind, actor_entity_id, occurred_at, metadata)| {
            // metadata is AgentAuthorship-shaped for agent acts, {} for system acts.
            // The band is the bare lowercase string under `confidence` (NOT `confidence_band`).
            let confidence = metadata
                .get("confidence")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            ElementEvent {
                event_id,
                kind,
                actor_entity_id,
                occurred_at: occurred_at.to_rfc3339(),
                confidence,
            }
        })
        .collect();

    Ok(EventTrail {
        element_kind: kind,
        element_id,
        events,
    })
}
