//! Reads about the calling profile's own work, under `/api/me/<noun>`: answers that are the
//! caller's by construction and name no other principal, kept off the resource-shaped routes they
//! would otherwise widen (`/api/contexts` lists contexts; it does not carry what the caller did).

use axum::extract::State;
use axum::Json;

use crate::middleware::auth::AnyPrincipal;
use temper_core::types::ids::ProfileId;
use temper_services::error::ApiResult;
use temper_services::services::context_service::{self, ContextActivity};
use temper_services::state::AppState;

/// When you last changed something in each context you can read
#[utoipa::path(
    get,
    operation_id = "list_my_context_activity",
    path = "/api/me/context-activity",
    tag = "Me",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "One row per context you changed something in and can still read, the time of your latest authored change there, newest first", body = Vec<ContextActivity>),
    )
)]
pub async fn context_activity(
    State(state): State<AppState>,
    auth: AnyPrincipal,
) -> ApiResult<Json<Vec<ContextActivity>>> {
    let profile_id = ProfileId::from(auth.0.profile().id);
    context_service::activity_of(&state.pool, profile_id)
        .await
        .map(Json)
}
