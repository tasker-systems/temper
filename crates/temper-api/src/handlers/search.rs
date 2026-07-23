use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue};
use axum::Json;

use crate::middleware::auth::AuthUser;
use temper_core::types::api::{SearchParams, UnifiedSearchResultRow};
use temper_core::types::ids::ProfileId;
use temper_services::error::{ApiResult, ErrorBody};
use temper_services::state::AppState;

/// The additive response header carrying scope-stage diagnostics (issue #360). Compact JSON of
/// [`temper_core::types::api::SearchDiagnostics`]. Kept out of the body so the `200` contract stays a
/// bare `Vec<UnifiedSearchResultRow>` — older clients ignore the header, newer ones read it.
const SEARCH_DIAGNOSTICS_HEADER: &str = "x-temper-search-diagnostics";

#[utoipa::path(
    post,
    path = "/api/search",
    tag = "Search",
    request_body = SearchParams,
    security(("bearer_auth" = [])),
    responses(
        (
            status = 200,
            description = "Ranked search results. Scope-stage diagnostics (issue #360) ride the \
                additive `x-temper-search-diagnostics` response header (compact JSON of \
                `SearchDiagnostics`); the body contract is unchanged.",
            body = Vec<UnifiedSearchResultRow>,
            headers(("x-temper-search-diagnostics" = String, description = "Compact JSON SearchDiagnostics")),
        ),
        (status = 400, description = "Invalid request", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn search(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(params): Json<SearchParams>,
) -> ApiResult<(HeaderMap, Json<Vec<UnifiedSearchResultRow>>)> {
    let response = temper_services::backend::substrate_read::search_select(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        params,
    )
    .await?;

    // Serialize diagnostics into the additive header. `from_bytes` (not `from_str`) so a hint with
    // non-ASCII (em dashes) rides as opaque UTF-8 rather than being dropped. Any failure to encode
    // just omits the header — diagnostics are best-effort metadata, never load-bearing for the body.
    let mut headers = HeaderMap::new();
    if let Some(diag) = response.diagnostics.as_ref() {
        if let Ok(json) = serde_json::to_string(diag) {
            if let Ok(value) = HeaderValue::from_bytes(json.as_bytes()) {
                headers.insert(SEARCH_DIAGNOSTICS_HEADER, value);
            }
        }
    }
    Ok((headers, Json(response.results)))
}
