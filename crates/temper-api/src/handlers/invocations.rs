//! `/api/invocations` — the agent-invocation envelope surface (accountability grain).
//!
//! Writes (`open`/`close`) dispatch ONE operations command through the `Backend` trait — never calling
//! services or `sqlx::query!` directly. Auth-before-write lives INSIDE the backend
//! (`DbBackend::{open,close}_invocation` gate on `check_can_read_cogmap`), so the handlers do NOT add a
//! handler-level auth gate; they just dispatch and let the backend return `Forbidden`/404.
//!
//! Reads (`show`/`list`) are service-direct via the `substrate_read` wrappers (the Backend-trait
//! projections are lossy; reads are passthroughs). The readback's deny→`None` contract makes deny and
//! absent indistinguishable — both surface as 404 (leak-safe).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use temper_services::backend::{substrate_read, DbBackend};
use temper_services::error::{ApiError, ApiResult};
use temper_services::state::AppState;

use temper_core::types::ids::{CogmapId, ProfileId};
use temper_core::types::invocation::{InvocationSummary, InvocationView};
use temper_core::types::invocation_requests::{
    CloseInvocationRequest, InvocationAck, OpenInvocationRequest,
};
use temper_workflow::operations::{Backend, CloseInvocation, OpenInvocation, Surface};

/// Query params for the list read. Both filters are optional (omit → unfiltered).
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub cogmap: Option<Uuid>,
    pub status: Option<String>,
}

#[utoipa::path(
    post,
    path = "/api/invocations",
    tag = "Invocations",
    security(("bearer_auth" = [])),
    request_body = OpenInvocationRequest,
    responses(
        (status = 200, description = "Invocation opened", body = InvocationAck),
        (status = 403, description = "Caller cannot read the originating cogmap"),
    )
)]
pub async fn open(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(req): Json<OpenInvocationRequest>,
) -> ApiResult<Json<InvocationAck>> {
    // Auth-before-write lives inside DbBackend::open_invocation (check_can_read_cogmap) — just dispatch.
    let cmd = OpenInvocation {
        trigger_kind: req.trigger_kind,
        originating_cogmap: CogmapId::from(req.originating_cogmap),
        parent_cogmap: req.parent_cogmap.map(CogmapId::from),
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend.open_invocation(cmd).await.map_err(ApiError::from)?;
    Ok(Json(InvocationAck {
        invocation_id: out.value,
    }))
}

#[utoipa::path(
    post,
    path = "/api/invocations/{id}/close",
    tag = "Invocations",
    params(("id" = Uuid, Path, description = "Invocation ID")),
    security(("bearer_auth" = [])),
    request_body = CloseInvocationRequest,
    responses(
        (status = 204, description = "Invocation closed"),
        (status = 404, description = "Invocation not found, or not readable by the caller (uniform — no existence oracle)"),
        (status = 409, description = "Invocation is already closed (close is a one-shot terminal transition)"),
    )
)]
pub async fn close(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(req): Json<CloseInvocationRequest>,
) -> ApiResult<StatusCode> {
    // Auth + existence (uniform 404, no oracle) + terminal-state guard (409 on re-close) all live
    // inside DbBackend::close_invocation — just dispatch.
    let cmd = CloseInvocation {
        invocation: id,
        disposition: req.disposition,
        outcome: req.outcome,
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    backend
        .close_invocation(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    get,
    path = "/api/invocations/{id}",
    tag = "Invocations",
    params(("id" = Uuid, Path, description = "Invocation ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Invocation envelope plus its acts", body = InvocationView),
        (status = 404, description = "Not found (or not readable — leak-safe)"),
    )
)]
pub async fn show(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<InvocationView>> {
    // Deny and absent are indistinguishable (readback returns None for both) — both 404.
    let view =
        substrate_read::invocation_show_select(&state.pool, ProfileId::from(auth.0.profile.id), id)
            .await?
            .ok_or(ApiError::NotFound)?;
    Ok(Json(view))
}

#[utoipa::path(
    get,
    path = "/api/invocations",
    tag = "Invocations",
    params(
        ("cogmap" = Option<Uuid>, Query, description = "Filter by originating cogmap"),
        ("status" = Option<String>, Query, description = "Filter by lifecycle status"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Visible invocation summaries", body = Vec<InvocationSummary>),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<InvocationSummary>>> {
    let rows = substrate_read::invocation_list_select(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        q.cogmap,
        q.status,
    )
    .await?;
    Ok(Json(rows))
}
