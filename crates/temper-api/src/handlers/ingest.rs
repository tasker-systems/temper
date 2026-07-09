use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AuthUser;
use temper_services::backend::DbBackend;
use temper_services::error::{ApiError, ApiResult};
use temper_services::state::AppState;

use temper_core::context_ref::parse_context_ref;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{CogmapId, ProfileId, ResourceId};
use temper_core::types::ingest::{IngestPayload, SegmentedBeginResponse};
use temper_workflow::operations::{Backend, BodyUpdate, CreateResource, Surface, UpdateResource};
use temper_workflow::types::managed_meta::ManagedMeta;
use temper_workflow::types::resource::ResourceRow;

/// `POST /api/ingest` returns one of two shapes depending on `IngestPayload.segmented`:
/// the one-shot `ResourceRow` (unchanged small-body path), or a [`SegmentedBeginResponse`]
/// when the caller began a segmented (multi-block) ingest. `#[serde(untagged)]` — the client
/// discriminates by which fields are present (`SegmentedBeginResponse` always carries
/// `correlation_id`/`blocks`, which `ResourceRow` never does).
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
#[serde(untagged)]
pub enum IngestCreateResponse {
    // Boxed: ResourceRow is much larger than SegmentedBeginResponse (clippy large_enum_variant).
    OneShot(Box<ResourceRow>),
    Segmented(SegmentedBeginResponse),
}

impl IntoResponse for IngestCreateResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::OneShot(r) => Json(r).into_response(),
            Self::Segmented(r) => Json(r).into_response(),
        }
    }
}

#[utoipa::path(
    post,
    path = "/api/ingest",
    tag = "Ingest",
    security(("bearer_auth" = [])),
    request_body = IngestPayload,
    responses(
        (status = 200, description = "Resource created (or existing on dedup); a SegmentedBeginResponse when the payload set `segmented`", body = IngestCreateResponse),
        (status = 400, description = "Invalid payload"),
        (status = 404, description = "Context not found"),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(payload): Json<IngestPayload>,
) -> ApiResult<IngestCreateResponse> {
    let profile_id = ProfileId::from(auth.0.profile.id);
    // Segmented-begin metadata is consumed AFTER create lands block 0 (below); take it now so the
    // rest of the function can move `payload` field-by-field into the CreateResource command.
    let segmented = payload.segmented.clone();

    // Resolve the home anchor — exactly one of a cognitive map or a context.
    // The cogmap branch takes precedence: when `home_cogmap_id` is set the home
    // is that map and `context_ref` is ignored.
    let home = match payload.home_cogmap_id {
        Some(map) => {
            // Auth before writes: the producer gate (a named service seam delegating to
            // `cogmap_authorable_by_profile` = an explicit `can_write` grant, NOT membership — the
            // Q-A flip made authorship wholly explicit) runs and denies BEFORE any home-row write.
            // A fast-fail pre-check; `DbBackend::create_resource` re-enforces the same gate (F1).
            let cogmap = CogmapId::from(map);
            if !temper_services::services::cogmap_service::authorable_by_profile(
                &state.pool,
                profile_id,
                cogmap,
            )
            .await?
            {
                return Err(ApiError::Forbidden);
            }
            HomeAnchor::Cogmap(cogmap)
        }
        None => {
            // Parse the context ref string (UUID or @owner/slug). Bare names are rejected with 400.
            let cref = parse_context_ref(&payload.context_ref)
                .map_err(|e| ApiError::BadRequest(e.to_string()))?;
            // Resolve to a ContextId, visibility-gated to the calling principal.
            let context = temper_services::services::context_service::resolve_context_ref(
                &state.pool,
                profile_id,
                &cref,
            )
            .await?;
            HomeAnchor::Context(context)
        }
    };

    // Convert IngestPayload's Option<Value> managed_meta to typed ManagedMeta.
    // Parse failures (malformed JSON for ManagedMeta shape) surface as BadRequest.
    let managed_meta: ManagedMeta = match payload.managed_meta {
        Some(v) => serde_json::from_value(v).map_err(|e| {
            ApiError::BadRequest(format!(
                "invalid managed_meta: {e}. managed_meta is a closed vocabulary; \
                 caller-defined keys belong in open_meta"
            ))
        })?,
        None => ManagedMeta::default(),
    };

    let body = if payload.content.is_empty() {
        None
    } else {
        Some(BodyUpdate {
            content: payload.content,
            // Create passes chunks/hash separately on CreateResource (below), so the
            // BodyUpdate carries only content + provenance sources here.
            content_hash: None,
            chunks_packed: None,
            sources: payload.sources,
            // Ingest create writes a single body block; per-block addressing is the PATCH surface.
            content_block: None,
        })
    };

    let act = payload.act.into_act_context().map_err(ApiError::from)?;

    let cmd = CreateResource {
        home,
        doctype: payload.doc_type_name,
        // Slug is §7-dissolved (never stored; addressing is trailing-UUID-only), so it is
        // NOT a caller input — always derived from the title. (issue #307 Bug 2)
        slug: temper_workflow::operations::sluggify(&payload.title),
        title: payload.title,
        body,
        managed_meta,
        open_meta: payload.open_meta,
        // First-class goal link: the CLI/MCP resolved `--goal <ref>` to this id client-side; the
        // backend projects the live `advances`→goal edge after create.
        goal: payload.goal.map(ResourceId::from),
        origin_uri: Some(payload.origin_uri),
        chunks_packed: payload.chunks_packed,
        content_hash: payload.content_hash,
        act,
        origin: Surface::ApiHttp,
    };

    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));

    let Some(seg) = segmented else {
        // Unchanged one-shot path — no new round-trips, no regression (design §5/§13).
        let out = backend.create_resource(cmd).await.map_err(ApiError::from)?;
        return Ok(IngestCreateResponse::OneShot(Box::new(out.value)));
    };

    // Segmented begin is ONE command: create block 0, record the source row, read the landed set.
    // The handler dispatches; it does not compose.
    let out = backend
        .begin_segmented_ingest(cmd, seg)
        .await
        .map_err(ApiError::from)?;
    Ok(IngestCreateResponse::Segmented(out.value))
}

#[utoipa::path(
    put,
    path = "/api/ingest/{id}",
    tag = "Ingest",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    request_body = IngestPayload,
    responses(
        (status = 200, description = "Resource updated", body = ResourceRow),
        (status = 400, description = "Invalid payload"),
        (status = 404, description = "Resource not found"),
    )
)]
pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
    Json(payload): Json<IngestPayload>,
) -> ApiResult<Json<ResourceRow>> {
    // Convert IngestPayload's Option<Value> managed_meta to typed ManagedMeta.
    let managed_meta: Option<ManagedMeta> = match payload.managed_meta {
        Some(v) => Some(serde_json::from_value(v).map_err(|e| {
            ApiError::BadRequest(format!(
                "invalid managed_meta: {e}. managed_meta is a closed vocabulary; \
                 caller-defined keys belong in open_meta"
            ))
        })?),
        None => None,
    };

    let body = if payload.content.is_empty() {
        None
    } else {
        Some(BodyUpdate {
            content: payload.content,
            // Forward caller-supplied pre-computed chunks so the translator
            // skips prepare_body_trio (and the ONNX pipeline) when they are
            // present. Matches the short-circuit in ingest_service::update.
            content_hash: payload.content_hash,
            chunks_packed: payload.chunks_packed,
            sources: payload.sources,
            // Ingest update revises the resource's sole body block; per-block addressing is the PATCH surface.
            content_block: None,
        })
    };

    let act = payload.act.into_act_context().map_err(ApiError::from)?;
    let cmd = UpdateResource {
        resource: ResourceId::from(resource_id),
        title: None,
        slug: None,
        body,
        managed_meta,
        open_meta: payload.open_meta,
        // The ingest update path revises body/chunks only; goal links travel via the PATCH
        // (`/api/resources/{id}`) surface, so nothing to project here.
        goal: None,
        move_to: None,
        context_ref: None,
        act,
        origin: Surface::ApiHttp,
    };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend.update_resource(cmd).await.map_err(ApiError::from)?;
    Ok(Json(out.value))
}
