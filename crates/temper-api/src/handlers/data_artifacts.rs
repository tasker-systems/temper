use axum::extract::{Path, Query, State};
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use temper_core::types::data_artifact::{ArtifactListParams, ArtifactView};
use temper_core::types::ids::{DataArtifactId, ProfileId, ResourceId};
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::state::AppState;

/// List artifacts for a resource, or counts with counts=true
///
/// Without `counts=true`: returns fully hydrated artifacts (metadata + content).
/// With `counts=true`: returns per-family counts only, no content hydration —
/// for surfaces that need "3 measurements, 1 extraction" without fetching payloads.
#[utoipa::path(
    get,
    operation_id = "list_artifacts",
    path = "/api/resources/{id}/artifacts",
    tag = "Data Artifacts",
    params(
        ("id" = Uuid, Path, description = "Resource ID"),
        ArtifactListParams,
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Artifacts owned by the resource (full or counts)", body = Vec<ArtifactView>),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
    Query(params): Query<ArtifactListParams>,
) -> ApiResult<Json<serde_json::Value>> {
    let profile = ProfileId::from(auth.0.profile().id);
    let resource = ResourceId::from(resource_id);
    let include_folded = params.include_folded.unwrap_or(false);

    if params.counts.unwrap_or(false) {
        let counts = temper_services::backend::substrate_read::artifact_counts(
            &state.pool,
            profile,
            resource,
            include_folded,
        )
        .await?;
        Ok(Json(serde_json::to_value(counts)?))
    } else {
        let artifacts = temper_services::backend::substrate_read::list_artifacts(
            &state.pool,
            profile,
            resource,
            params.kind.as_deref(),
            params.intent.as_deref(),
            include_folded,
        )
        .await?;
        Ok(Json(serde_json::to_value(artifacts)?))
    }
}

/// Get a single artifact by ID under its owning resource
///
/// The resource ID in the path is the REST parent; visibility is gated on the
/// artifact's actual owning resource via `resources_visible_to`. Returns 404
/// if the artifact does not exist or is not visible to the caller.
#[utoipa::path(
    get,
    operation_id = "get_artifact",
    path = "/api/resources/{id}/artifacts/{artifact_id}",
    tag = "Data Artifacts",
    params(
        ("id" = Uuid, Path, description = "Resource ID (REST parent)"),
        ("artifact_id" = Uuid, Path, description = "Artifact ID"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The artifact with content", body = ArtifactView),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Not found or not visible", body = ErrorBody),
    )
)]
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((_resource_id, artifact_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<Json<ArtifactView>> {
    let artifact = temper_services::backend::substrate_read::get_artifact(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        DataArtifactId::from(artifact_id),
    )
    .await?
    .ok_or_else(|| ApiError::NotFound("artifact not found".to_string()))?;
    Ok(Json(artifact))
}
