//! Subscriptions — a team/context/cogmap subscribes to an aspect of a connection (S2 chunk A
//! of "external systems as subscribed emitters"). Out of the OpenAPI contract (plain
//! `.route()` mounting), like `/api/connections` and `/api/machine-clients`: this is an admin
//! surface, not a public one.
//!
//! **Authorization lives in the service, not here** — `subscription_service` calls
//! `require_manage_on_team` (owner/maintainer on the authoring team) + `kb_access_grants`
//! reach-grant read. As with connections, that check is load-bearing rather than
//! defense-in-depth.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::subscription::{CreateSubscriptionRequest, Subscription};
use temper_services::error::ApiResult;
use temper_services::services::subscription_service;
use temper_services::state::AppState;

use crate::middleware::auth::AuthUser;

/// Query flags for `GET /api/subscriptions`.
#[derive(Debug, Deserialize)]
pub struct ListQuery {
    #[serde(default)]
    pub include_revoked: bool,
    /// Optional filter: only subscriptions against this connection.
    pub connection_id: Option<Uuid>,
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateSubscriptionRequest>,
) -> ApiResult<Json<Subscription>> {
    let caller = ProfileId::from(auth.0.profile().id);
    let subscription = subscription_service::create(&state.pool, caller, &body).await?;
    Ok(Json(subscription))
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<Subscription>>> {
    let caller = ProfileId::from(auth.0.profile().id);
    Ok(Json(
        subscription_service::list(&state.pool, caller, q.include_revoked, q.connection_id).await?,
    ))
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Subscription>> {
    let caller = ProfileId::from(auth.0.profile().id);
    Ok(Json(
        subscription_service::get_for_caller(&state.pool, caller, id).await?,
    ))
}

pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Subscription>> {
    let caller = ProfileId::from(auth.0.profile().id);
    Ok(Json(
        subscription_service::revoke(&state.pool, caller, id).await?,
    ))
}
