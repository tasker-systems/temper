use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue};
use axum::Json;
use uuid::Uuid;

use crate::middleware::auth::AnyPrincipal;
use crate::middleware::surface::RequestSurface;
use temper_core::types::data_artifact::{
    ArtifactCommitRequest, ArtifactCommitResponse, ArtifactCursor, ArtifactListParams, ArtifactView,
};
use temper_core::types::ids::{DataArtifactId, ProfileId, ResourceId};
use temper_services::backend::substrate_read::ArtifactPageFilter;
use temper_services::backend::DbBackend;
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::state::AppState;
use temper_workflow::operations::{Backend, CommitDataArtifact};

/// List artifacts for a resource, or counts with counts=true
///
/// Without `counts=true`: returns fully hydrated artifacts (metadata + content).
/// With `counts=true`: returns per-family counts only, no content hydration —
/// for surfaces that need "3 measurements, 1 extraction" without fetching payloads.
///
/// With `limit`, returns at most that many artifacts in `(created, id)` order: oldest first, or
/// newest first with `order=desc`. The body is the same bare array either way; when more remain,
/// the response carries a `Link: <...>; rel="next"` header whose target is this request with the
/// next page's `after_created` and `after_id` (and the caller's own filters and `order`), and the
/// header is absent on the last page and on every response without `limit`. A `limit` of 0 or above the server ceiling `MAX_ARTIFACT_PAGE_ROWS` is
/// refused, not clamped. Without `limit`, returns the whole list: the unbounded form, which a
/// later release retires.
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
        (
            status = 200,
            description = "Artifacts owned by the resource (full or counts)",
            body = Vec<ArtifactView>,
            headers(
                ("Link" = String, description = "Present only when `limit` was given and more artifacts remain: `<target>; rel=\"next\"`, where the target is this route with the next page's query (`after_created`, `after_id`, `limit`, `order`, and the caller's own filters), percent-encoded.")
            )
        ),
        (status = 400, description = "`limit` is 0 or above the page ceiling, `after_created` and `after_id` are not sent together with `limit`, `order` is sent without `limit`, or `order` is neither `asc` nor `desc` (that last one a plain-text rejection naming the parameter, not an ErrorBody)", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    Path(resource_id): Path<Uuid>,
    Query(params): Query<ArtifactListParams>,
) -> ApiResult<(HeaderMap, Json<serde_json::Value>)> {
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
        return Ok((HeaderMap::new(), Json(serde_json::to_value(counts)?)));
    }

    let Some(request) = params
        .page_request()
        .map_err(|refusal| ApiError::BadRequest(refusal.to_string()))?
    else {
        let artifacts = temper_services::backend::substrate_read::list_artifacts(
            &state.pool,
            profile,
            resource,
            params.kind.as_deref(),
            params.intent.as_deref(),
            include_folded,
        )
        .await?;
        return Ok((HeaderMap::new(), Json(serde_json::to_value(artifacts)?)));
    };

    let page = temper_services::backend::substrate_read::list_artifacts_page(
        &state.pool,
        profile,
        resource,
        ArtifactPageFilter {
            kind: params.kind.as_deref(),
            intent: params.intent.as_deref(),
            include_folded,
        },
        request,
    )
    .await?;
    let mut headers = HeaderMap::new();
    if let Some(cursor) = page.next {
        headers.insert(header::LINK, next_link(resource_id, &params, cursor)?);
    }
    Ok((headers, Json(serde_json::to_value(page.artifacts)?)))
}

/// The `Link: <...>; rel="next"` value that continues a bounded list after `cursor`.
///
/// The target is the same route with the next request's query: the caller's own filters, `limit`
/// and `order`, plus the cursor as `after_created` and `after_id`. Carrying `order` is what keeps
/// a newest-first walk newest-first; the cursor itself is direction-free. It is serialized from
/// [`ArtifactListParams`] itself, the struct `Query` reads back, so the link a client follows is
/// by construction a request this route accepts. `counts` is dropped: a counts answer never
/// pages. Relative, because the route does not know the host the caller reached it by.
fn next_link(
    resource_id: Uuid,
    params: &ArtifactListParams,
    cursor: ArtifactCursor,
) -> ApiResult<HeaderValue> {
    let next = ArtifactListParams {
        counts: None,
        after_created: Some(cursor.created),
        after_id: Some(cursor.artifact_id.uuid()),
        ..params.clone()
    };
    let query = serde_urlencoded::to_string(&next)
        .map_err(|e| ApiError::Internal(format!("cannot encode the next page's query: {e}")))?;
    HeaderValue::from_str(&format!(
        "</api/resources/{resource_id}/artifacts?{query}>; rel=\"next\""
    ))
    .map_err(|e| ApiError::Internal(format!("cannot encode the next page's link: {e}")))
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
    auth: AnyPrincipal,
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

/// Get a single artifact by ID — the flat read.
///
/// The MCP `get_data_artifact` tool takes only the artifact id (flat, visibility-gated,
/// answers folded rows); until this route the flat read had no wire door — only the
/// nested [get](get) under the resource path, and the tool's declaration cannot grow a
/// `resource_id` field. Visibility is gated on the artifact's actual owning resource
/// via `resources_visible_to`, exactly the nested read's posture: 404 when the artifact
/// does not exist or is not visible to the caller, folded artifacts included.
#[utoipa::path(
    get,
    operation_id = "get_artifact_by_id",
    path = "/api/data-artifacts/{artifact_id}",
    tag = "Data Artifacts",
    params(
        ("artifact_id" = Uuid, Path, description = "Artifact ID"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The artifact with content", body = ArtifactView),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Not found or not visible", body = ErrorBody),
    )
)]
// The `[get](get)` link above is what rustdoc 1.99's `redundant_explicit_links` flags. The doc
// comment is also this operation's published OpenAPI description, so rewording it would move the
// contract; the allow keeps the contract byte-identical.
#[allow(rustdoc::redundant_explicit_links)]
pub async fn get_by_id(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    Path(artifact_id): Path<Uuid>,
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

/// Commit one data artifact to a resource
///
/// The content payload is JSON, hashed and stored verbatim. The hash is the proof —
/// the ledger carries only the hash, never the bytes. Auth-gated: the caller must
/// have write standing on the owning resource.
#[utoipa::path(
    post,
    operation_id = "commit_artifact",
    path = "/api/resources/{id}/artifacts",
    tag = "Data Artifacts",
    params(
        ("id" = Uuid, Path, description = "Resource ID (the artifact's owner)"),
    ),
    request_body = ArtifactCommitRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Committed artifact", body = ArtifactCommitResponse),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "No write standing on the owning resource", body = ErrorBody),
        (status = 404, description = "Resource not found or not visible", body = ErrorBody),
        (status = 410, description = "The resource was erased (code RESOURCE_ERASED); answered only to a caller who held standing on it, everyone else gets 403", body = ErrorBody),
    )
)]
pub async fn commit(
    State(state): State<AppState>,
    auth: AnyPrincipal,
    RequestSurface(surface): RequestSurface,
    Path(resource_id): Path<Uuid>,
    Json(req): Json<ArtifactCommitRequest>,
) -> ApiResult<Json<ArtifactCommitResponse>> {
    let act = req.act.into_act_context().map_err(ApiError::from)?;
    let cmd = CommitDataArtifact {
        resource: ResourceId::from(resource_id),
        kind: req.kind,
        kind_owner: req.kind_owner,
        intent: req.intent,
        precedence: req.precedence,
        content: req.content,
        supersedes: req.supersedes,
        act,
        origin: surface,
    };
    let backend = DbBackend::with_proof(state.pool.clone(), &auth.0);
    let out = backend
        .commit_data_artifact(cmd)
        .await
        .map_err(ApiError::from)?;
    Ok(Json(ArtifactCommitResponse {
        artifact_id: out.value.artifact_id,
        artifact: out.value,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::types::resource::SortOrder;

    fn cursor() -> ArtifactCursor {
        ArtifactCursor {
            created: "2026-10-19T12:30:45.123456Z"
                .parse()
                .expect("literal parses"),
            artifact_id: DataArtifactId::from(Uuid::from_u128(0x0123_4567_89ab_cdef)),
        }
    }

    fn params() -> ArtifactListParams {
        ArtifactListParams {
            // Characters that must be percent-encoded to survive a query string.
            kind: Some("a b&c=d".to_string()),
            intent: Some("pinned".to_string()),
            include_folded: Some(true),
            limit: Some(2),
            order: Some(SortOrder::Desc),
            ..Default::default()
        }
    }

    #[test]
    fn the_next_link_round_trips_through_the_clients_parser() {
        let resource = Uuid::from_u128(7);
        let link = next_link(resource, &params(), cursor()).expect("link formats");
        let parsed = temper_client::data_artifacts::parse_next_link(link.to_str().unwrap())
            .expect("the client reads the server's own link");
        assert_eq!(parsed, Some(cursor()));
    }

    #[test]
    fn the_next_link_targets_this_route_with_the_callers_filters_and_the_position() {
        let resource = Uuid::from_u128(7);
        let link = next_link(resource, &params(), cursor()).expect("link formats");
        let link = link.to_str().unwrap();
        let target = link
            .strip_prefix(&format!("</api/resources/{resource}/artifacts?"))
            .and_then(|rest| rest.strip_suffix(">; rel=\"next\""))
            .unwrap_or_else(|| panic!("unexpected link shape: {link}"));
        let query: ArtifactListParams =
            serde_urlencoded::from_str(target).expect("the route's own Query reads the target");
        assert_eq!(query.kind.as_deref(), Some("a b&c=d"));
        assert_eq!(query.intent.as_deref(), Some("pinned"));
        assert_eq!(query.include_folded, Some(true));
        assert_eq!(query.limit, Some(2));
        assert_eq!(
            query.order,
            Some(SortOrder::Desc),
            "the next page continues in the direction the first was asked in"
        );
        assert_eq!(query.counts, None);
        assert_eq!(query.after_created, Some(cursor().created));
        assert_eq!(query.after_id, Some(cursor().artifact_id.uuid()));
    }

    /// An `order` outside `asc`/`desc` is refused by name at the door, never read as either.
    /// The route reads its params through axum's `Query`, whose rejection is a 400 naming the
    /// field at fault (`serde_path_to_error`).
    #[test]
    fn an_unknown_order_is_refused_by_name() {
        let uri: axum::http::Uri = "/api/resources/x/artifacts?limit=2&order=sideways"
            .parse()
            .unwrap();
        let rejection = Query::<ArtifactListParams>::try_from_uri(&uri)
            .expect_err("an unknown order is not defaulted");
        assert_eq!(rejection.status(), axum::http::StatusCode::BAD_REQUEST);
        let said = rejection.body_text();
        assert!(
            said.contains("order") && said.contains("sideways"),
            "the refusal names the parameter and the value, got: {said}"
        );

        for spelled in ["asc", "desc"] {
            let uri: axum::http::Uri =
                format!("/api/resources/x/artifacts?limit=2&order={spelled}")
                    .parse()
                    .unwrap();
            let Query(read) = Query::<ArtifactListParams>::try_from_uri(&uri).expect("reads");
            assert!(read.order.is_some(), "{spelled} is a direction");
        }
    }
}
