//! Knowledge-graph subgraph handler — serves `GET /api/graph/subgraph`.

use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;

use crate::error::{ApiError, ApiResult, ErrorBody};
use crate::middleware::auth::AuthUser;
use crate::services::graph_service::{aggregator_subgraph, AggregatorSubgraphParams};
use crate::state::AppState;
use temper_core::frontmatter::document::DocType;
use temper_core::types::graph::SubgraphResponse;

/// Query parameters for `GET /api/graph/subgraph`.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct SubgraphQuery {
    /// Profile handle to query. `"@me"` resolves to the caller's profile.
    pub owner: String,
    /// Context name (e.g., `"temper"`).
    pub context: String,
}

#[utoipa::path(
    get,
    path = "/api/graph/subgraph",
    tag = "Graph",
    params(SubgraphQuery),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Concept-centric subgraph", body = SubgraphResponse),
        (status = 400, description = "Bad query parameters", body = ErrorBody),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn get_subgraph(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<SubgraphQuery>,
) -> ApiResult<Json<SubgraphResponse>> {
    // Resolve `owner` — v1 only supports "@me" (caller's own vault).
    // Cross-owner querying is deferred; handles are left as a later migration.
    // A client-supplied handle other than "@me" is an invalid query parameter,
    // so we return 400 Bad Request rather than 404.
    if query.owner != "@me" {
        return Err(ApiError::BadRequest(format!(
            "owner handle '{}' not supported — only '@me' in v1",
            query.owner
        )));
    }

    let response = aggregator_subgraph(
        &state.pool,
        AggregatorSubgraphParams {
            caller_profile_id: auth.0.profile.id,
            context_name: &query.context,
            aggregator_types: &[DocType::Concept],
            depth: 2,
        },
    )
    .await?;

    Ok(Json(response))
}
