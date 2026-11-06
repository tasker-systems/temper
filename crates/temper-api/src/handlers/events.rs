use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AnyPrincipal;
use temper_core::types::api::{ContextChangesResponse, EventCursorResponse};
use temper_core::types::element_trail::{ElementKind, EventTrail};
use temper_core::types::ids::{ContextId, ProfileId};
use temper_core::types::query_params::ContextChangesQuery;
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::services::{context_change_service, event_service};
use temper_services::state::AppState;

/// Read the event cursor for a context
#[utoipa::path(
    get,
    path = "/api/events/{kb_context_id}/cursor",
    tag = "Events",
    params(
        ("kb_context_id" = Uuid, Path, description = "The context whose latest event id is requested"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Latest event id for the context", body = EventCursorResponse),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn cursor(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    Path(kb_context_id): Path<Uuid>,
) -> ApiResult<Json<EventCursorResponse>> {
    let latest_event_id = event_service::latest_event_id_for_context(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        ContextId::from(kb_context_id),
    )
    .await?;
    Ok(Json(EventCursorResponse { latest_event_id }))
}

/// Read a context's changes after a position
#[utoipa::path(
    get,
    operation_id = "context_changes",
    summary = "Read a context's changes after a position",
    description = "A context's changes after a position, in commit order. Each entry is a pointer naming the kind of change, the thing changed (`subject`), the acting entity, the time and the act. No entry carries content: read the subject through its own endpoint, under your own standing. A subject you can no longer read is still pointed at, and its own read refuses it. A change to one of a resource's content blocks names the resource as its `subject` and the block as `block`.\n\nOmit `after` to start: the answer has no entries and a `through` at the present, never a tail of history. Hand `through` back as `after` to resume. A position is opaque and is valid only for the context it was issued for. Every answer says how far it looked (`through`) and whether more exists (`has_more`).\n\n**This is a change hint, not a cache-invalidation guarantee.** The feed never announces an admin act, such as an erasure or a scrub. A consumer that mirrors content must periodically re-read the subjects it holds and drop whatever is refused: that re-read is how erased or scrubbed content leaves a mirror.",
    path = "/api/events/{kb_context_id}/changes",
    tag = "Events",
    params(
        ("kb_context_id" = Uuid, Path, description = "The context whose changes are read"),
        ContextChangesQuery,
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The changes after the position, and the position read through", body = ContextChangesResponse),
        (status = 400, description = "A position this deployment did not issue for this context, or one ahead of its ledger; a page bound outside 1..=1000; an unknown change kind. A refused position is never read as the beginning: omit `after` to start again", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "The context does not exist or is not readable, refused exactly as a context read is, before the position, bound or kinds are looked at", body = ErrorBody),
    )
)]
pub async fn changes(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    Path(kb_context_id): Path<Uuid>,
    axum::extract::Query(query): axum::extract::Query<ContextChangesQuery>,
) -> ApiResult<Json<ContextChangesResponse>> {
    context_change_service::changes_after(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        ContextId::from(kb_context_id),
        &query,
    )
    .await
    .map(Json)
}

/// Read a node or edge's event trail
#[utoipa::path(
    get,
    path = "/api/graph/elements/{kind}/{id}/trail",
    tag = "Events",
    params(
        ("kind" = String, Path, description = "node | edge"),
        ("id" = Uuid, Path, description = "resource id (node) or edge id")
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Event trail", body = EventTrail),
        (status = 400, description = "Unknown element kind")
    )
)]
pub async fn element_trail(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    Path((kind, id)): Path<(String, Uuid)>,
) -> ApiResult<Json<EventTrail>> {
    let element_kind = match kind.as_str() {
        "node" => ElementKind::Node,
        "edge" => ElementKind::Edge,
        _ => {
            return Err(ApiError::BadRequest(
                "element kind must be 'node' or 'edge'".into(),
            ))
        }
    };
    event_service::element_trail(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        element_kind,
        id,
    )
    .await
    .map(Json)
}
