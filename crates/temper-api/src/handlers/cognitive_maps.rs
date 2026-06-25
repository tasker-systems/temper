//! `PUT /api/cognitive-maps/{id}` — admin-gated, idempotent cognitive-map content reconcile.
//!
//! The request body is a PRE-EMBEDDED desired-state manifest (the operator CLI embeds client-side). The
//! handler enforces the root-team-cogmap write gate (Auth before writes), then dispatches ONE operations
//! command through the `Backend` trait — it never calls services or `sqlx::query!` directly for the write.

use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::backend::DbBackend;
use crate::error::{ApiError, ApiResult};
use crate::middleware::auth::AuthUser;
use crate::services::access_service;
use crate::state::AppState;

use temper_core::types::ids::ProfileId;
use temper_core::types::reconcile::{ReconcileCogmapRequest, ReconcileOutcome};
use temper_workflow::operations::{Backend, ReconcileCognitiveMap, Surface};

#[utoipa::path(
    put,
    path = "/api/cognitive-maps/{id}",
    tag = "Cognitive Maps",
    params(("id" = Uuid, Path, description = "Cognitive map ID")),
    security(("bearer_auth" = [])),
    request_body = ReconcileCogmapRequest,
    responses(
        (status = 200, description = "Reconcile applied", body = ReconcileOutcome),
        (status = 403, description = "Caller is not a system admin for this root-team map"),
        (status = 409, description = "A reconcile is already in progress on this map"),
    )
)]
pub async fn reconcile(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cogmap_id): Path<Uuid>,
    Json(request): Json<ReconcileCogmapRequest>,
) -> ApiResult<Json<ReconcileOutcome>> {
    // Auth before writes (Global Constraints): the root-team-cogmap write gate.
    access_service::require_cogmap_write_admin(&state.pool, auth.0.profile.id, cogmap_id).await?;

    let cmd = ReconcileCognitiveMap {
        cogmap_id,
        request,
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend
        .reconcile_cognitive_map(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(out.value))
}
