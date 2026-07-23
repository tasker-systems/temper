use axum::extract::State;
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use crate::middleware::surface::RequestSurface;
use temper_core::types::facet_requests::{FacetAck, FacetSetRequest};
use temper_core::types::ids::{ProfileId, ResourceId};
use temper_services::backend::DbBackend;
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::state::AppState;
use temper_workflow::operations::{Backend, SetFacet};

// ─── Handlers ────────────────────────────────────────────────────────────────

#[utoipa::path(
    post,
    path = "/api/facets",
    tag = "Facets",
    security(("bearer_auth" = [])),
    request_body = FacetSetRequest,
    responses(
        (status = 200, description = "Facet set", body = FacetAck),
        (status = 400, description = "Invalid payload", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "Cannot modify resource", body = ErrorBody),
        (status = 404, description = "Resource not found", body = ErrorBody),
    )
)]
pub async fn set_facet(
    State(state): State<AppState>,
    auth: AuthUser,
    RequestSurface(surface): RequestSurface,
    Json(req): Json<FacetSetRequest>,
) -> ApiResult<Json<FacetAck>> {
    let act = req.act.into_act_context().map_err(ApiError::from)?;
    let cmd = SetFacet {
        resource: ResourceId::from(req.resource),
        values: req.values,
        weight: req.weight,
        act,
        origin: surface,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile().id));
    let out = backend.set_facet(cmd).await.map_err(ApiError::from)?;
    Ok(Json(FacetAck {
        id: Uuid::from(out.value),
        property_id: Uuid::from(out.value),
    }))
}
