use axum::extract::State;
use axum::Json;

use crate::middleware::auth::AuthUser;
use temper_core::types::api::{SearchParams, SearchResponse};
use temper_core::types::ids::ProfileId;
use temper_services::error::{ApiResult, ErrorBody};
use temper_services::state::AppState;

#[utoipa::path(
    post,
    path = "/api/search",
    tag = "Search",
    request_body = SearchParams,
    security(("bearer_auth" = [])),
    responses(
        (
            status = 200,
            description = "Two arms that are never combined: `exact` (term matching, ordered by \
                `fts_norm`) and `wide` (embedding proximity, ordered by `vec_norm`), each with its \
                own `reason`, plus the `scope` they share. No field ranks one arm against the other.",
            body = SearchResponse,
        ),
        (status = 400, description = "Invalid request", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
// The `x-temper-search-diagnostics` response header is GONE. It carried `SearchDiagnostics`
// beside a body that was a bare `Vec<UnifiedSearchResultRow>`, and existed for exactly that
// reason; the body is an object now, so diagnostics live in it, per-arm, beside the hits they
// describe. That also retires the header's percent-encoding scar — non-ASCII hint text was
// arriving at clients as `%E2%80%94` because the serverless adapter encoded header bytes.
// Hints stay ASCII anyway, guarded by `every_emitted_hint_is_ascii`, since nothing is gained
// by relaxing it.
/// Search resources by text or embedding
///
/// Answers in two arms: exact (full-text) and wide (vector). Each arm carries its own diagnostics in the response body, beside the hits they describe.
pub async fn search(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(params): Json<SearchParams>,
) -> ApiResult<Json<SearchResponse>> {
    let response = temper_services::backend::substrate_read::search_select(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        params,
    )
    .await?;
    Ok(Json(response))
}
