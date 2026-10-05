//! Subscriptions — a team/context/cogmap subscribes to an aspect of a connection (S2 chunk A
//! of "external systems as subscribed emitters"), documented under the `Subscriptions` tag. Not
//! admin-only: a manager of the authoring team declares and revokes its own subscriptions, so the
//! family stays in `gated_routes`, apart from the admin group.
//!
//! **Authorization lives in the service, not here** — `subscription_service` resolves
//! `SubscriptionAuthority` (owner/maintainer on the authoring team, or a system admin), and
//! `create` additionally requires the authoring team to hold a `kb_access_grants` reach grant on
//! the connection. As with connections, that check is load-bearing rather than
//! defense-in-depth.

use axum::extract::{Path, Query, State};
use axum::Json;
use uuid::Uuid;

// `Subscription` publishes as `ConnectionSubscription` (`schema(as = …)` on the type): the
// contract already has a `Subscription`, the vault-config one.
use temper_core::types::subscription::{CreateSubscriptionRequest, Subscription};
use temper_services::error::{ApiResult, ErrorBody};
use temper_services::services::subscription_service;
use temper_services::state::AppState;

use crate::middleware::auth::AuthUser;
use temper_core::types::query_params::SubscriptionListQuery as ListQuery;

#[utoipa::path(
    post,
    operation_id = "create_subscription",
    summary = "Create a subscription",
    description = "Declares that a team, context or cogmap wants the events a connection emits that match `selector`. Requires a system admin or an owner or maintainer of `authoring_team_id`, and, for every caller, that the authoring team holds read-reach on the connection. A context or cogmap subscriber must be linked to the authoring team; a team subscriber must be the authoring team itself. A selector that can never match the connection's registered webhook events is refused.",
    path = "/api/subscriptions",
    tag = "Subscriptions",
    request_body = CreateSubscriptionRequest,
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The created subscription", body = Subscription),
        (status = 400, description = "Unknown `subscriber_table`, a subscriber not linked to the authoring team, or a selector that can never match", body = ErrorBody),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller is neither a system admin nor an owner or maintainer of the authoring team, the authoring team has no read-reach on the connection, or the caller lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 409, description = "A subscription with the same authoring team, connection and selector already exists, live or revoked", body = ErrorBody),
        (status = 422, description = "The body is JSON but not the expected shape, e.g. an unknown selector `kind` (a plain-text rejection, not an ErrorBody)"),
    )
)]
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateSubscriptionRequest>,
) -> ApiResult<Json<Subscription>> {
    let subscription = subscription_service::create(&state.pool, &auth.0, &body).await?;
    Ok(Json(subscription))
}

#[utoipa::path(
    get,
    operation_id = "list_subscriptions",
    summary = "List subscriptions",
    description = "Lists the subscriptions the caller may manage, newest first. A system admin sees every subscription; any other caller sees only those whose authoring team they own or maintain. Revoked subscriptions are omitted unless `include_revoked` is set.",
    path = "/api/subscriptions",
    tag = "Subscriptions",
    params(ListQuery),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The subscriptions visible to the caller", body = Vec<Subscription>),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<Subscription>>> {
    Ok(Json(
        subscription_service::list(&state.pool, &auth.0, q.include_revoked, q.connection_id)
            .await?,
    ))
}

#[utoipa::path(
    get,
    operation_id = "get_subscription",
    summary = "Get a subscription",
    description = "Returns one subscription. Requires a system admin or an owner or maintainer of the subscription's authoring team. Any other caller with system access is answered 404, exactly as for an id that does not exist.",
    path = "/api/subscriptions/{id}",
    tag = "Subscriptions",
    params(("id" = Uuid, Path, description = "Subscription ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The subscription", body = Subscription),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 404, description = "No such subscription, or one the caller may not act on: the two are answered identically", body = ErrorBody),
    )
)]
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Subscription>> {
    Ok(Json(
        subscription_service::get_for_caller(&state.pool, &auth.0, id).await?,
    ))
}

#[utoipa::path(
    delete,
    operation_id = "revoke_subscription",
    summary = "Revoke a subscription",
    description = "Revokes a subscription so it stops matching new events. The row is kept so past deliveries still resolve. Revoking an already-revoked subscription returns it unchanged. Requires a system admin or an owner or maintainer of the subscription's authoring team. Any other caller with system access is answered 404, exactly as for an id that does not exist.",
    path = "/api/subscriptions/{id}",
    tag = "Subscriptions",
    params(("id" = Uuid, Path, description = "Subscription ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The revoked subscription", body = Subscription),
        (status = 401, description = "Authentication required", body = ErrorBody),
        (status = 403, description = "Caller lacks system access (`SYSTEM_ACCESS_REQUIRED`)", body = ErrorBody),
        (status = 404, description = "No such subscription, or one the caller may not act on: the two are answered identically", body = ErrorBody),
    )
)]
pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Subscription>> {
    Ok(Json(
        subscription_service::revoke(&state.pool, &auth.0, id).await?,
    ))
}
