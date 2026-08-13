use axum::extract::State;
use axum::Json;

use crate::middleware::auth::AuthUser;
use temper_core::types::ids::ProfileId;
use temper_core::types::query::composition::Composition;
use temper_core::types::query::envelope::QueryResponse;
use temper_services::backend::query_read;
use temper_services::error::{ApiError, ApiResult, ErrorBody};
use temper_services::state::AppState;

#[utoipa::path(
    post,
    path = "/api/query",
    tag = "Query",
    request_body = Composition,
    security(("bearer_auth" = [])),
    responses(
        (
            status = 200,
            description = "One entry in `returned` per `outcome.returns` — no more, no fewer — \
                keyed by stage name rather than merged into one ordered list, so combining two \
                acts' rows takes a deliberate act by the caller. `trace` covers EVERY stage, \
                including those whose rows were not returned, because the pipe carries ids rather \
                than rows and an untraced composition is a black box with an answer at the end.",
            body = QueryResponse,
        ),
        (
            status = 400,
            description = "The composition will not run, with **every** static reason at once in \
                `error.details.refusals` under the code `PLAN_REFUSED` — never just the first, \
                because repairing a plan one refusal per round trip is the experience this \
                contract exists to avoid. A caller meets this response before they meet a 200, so \
                it is the door's most-read documentation.",
            body = ErrorBody,
        ),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 403, description = "System access required", body = ErrorBody),
    )
)]
/// `POST /api/query`.
///
/// The door onto the composition contract: a caller sends a plan, the server answers it or refuses
/// it. Everything before this route built a door that nothing could knock on.
///
/// **The pipeline is not assembled here.** [`query_read::prepare`] owns the order — shape-gate,
/// then embed, then validate — and is the only constructor of a `ValidatedComposition`, so this
/// handler cannot run an unvalidated plan even by mistake. Spelling the order out here would make
/// this the second place that knows it, and the day the MCP tool and the CLI arrive, the third and
/// fourth.
///
/// **The refusal branch is the only thing that differs from [`super::search::search`]**, whose
/// shape this otherwise copies: `search_select` takes params and answers, while `prepare` may
/// refuse first.
pub async fn query(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(composition): Json<Composition>,
) -> ApiResult<Json<QueryResponse>> {
    let validated = query_read::prepare(composition)
        .await
        .map_err(|refusals| ApiError::PlanRefused { refusals })?;

    let response = query_read::run_composition(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        &validated,
    )
    .await?;
    Ok(Json(response))
}
