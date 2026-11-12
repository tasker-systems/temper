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
    description = "The newest event id on the context among the events its change feed would serve you, or null. **A hint that can miss a change, not a freshness proof.** It is ordered by `occurred_at`, when the acting transaction began, so a long transaction that begins before a short one and commits after it never moves the cursor past the short one's event. To see every change, read the change feed (`GET /api/events/{kb_context_id}/changes`), which is commit-ordered and does not have this race.",
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
    description = "A context's changes after a position, in commit order. Each entry is a pointer naming the kind of change, the thing changed (`subject`), the acting entity, the time and the act. No entry carries content: read the subject through its own endpoint, under your own standing. A subject you can no longer read is still pointed at, and its own read refuses it. A change to one of a resource's content blocks names the resource as its `subject` and the block as `block`.\n\nOmit `after` to start: the answer has no entries and a `through` at the present, never a tail of history. Hand `through` back as `after` to resume. A position is opaque and is valid only for the context it was issued for. Every answer says how far it looked (`through`) and whether more exists (`has_more`).\n\n**This is a change hint, not a cache-invalidation guarantee.** The feed never announces an admin act, such as an erasure or a scrub. A consumer that mirrors content must periodically re-read the subjects it holds and drop whatever is refused: that re-read is how erased or scrubbed content leaves a mirror.\n\n**The consumer contract.**\n- Record `through` *before* reading the bodies an answer points at, never after. A change that commits between the bodies read and a later `through` is behind the recorded position, and is lost. A change seen again after an early `through` costs one redundant re-read.\n- An edge entry names the edge (`kb_edges` and the edge id), not its endpoints. To learn which resources it touches, read the edge.\n- An edge appears in the feed of its own home context, not in its endpoints' contexts. An edge homed elsewhere that touches a resource homed here is not in this feed.\n- `resource_departed` is a deletion or a move, and does not say which. Treat it as \"re-resolve this resource\": it may still exist in another context you can read.\n- A scrub of a facet or other property is not announced. Re-read `GET /api/resources/{id}/facets` for the resources you hold on the same periodic cadence, not only on a feed hint.\n- `GET /api/resources?ids=` (up to 256 ids) is the cheap periodic sweep: it returns `updated`, `body_hash` and `body_storage` for exactly the ids you can still read, and leaves out the rest, without downloading a body.",
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
