//! Disconnect surfaces — self-serve, admin, and the intents reaper cron.
//!
//! Both `disconnect_me` and `admin_disconnect` dispatch to the one service chokepoint. The
//! self-serve arm never accepts a principal from the caller: it derives it from the caller's own
//! auth-link row, so naming someone else's principal is not expressible.

use axum::extract::State;
use axum::Json;
use temper_core::types::ids::ProfileId;
use temper_core::types::slack::{SlackDisconnectRequest, SlackDisconnectResponse};
use temper_services::error::{ApiError, ApiResult};
use temper_services::services::access_service;
use temper_services::services::slack_disconnect_service::{
    disconnect_slack_principal, DisconnectOutcome, DisconnectRequest,
};
use temper_services::services::slack_link_service::SLACK_AUTH_PROVIDER;
use temper_services::state::AppState;

use crate::middleware::auth::AuthUser;

fn to_response(outcome: DisconnectOutcome) -> SlackDisconnectResponse {
    SlackDisconnectResponse {
        was_linked: outcome.was_linked,
        grant_deleted: outcome.grant_deleted,
        intents_deleted: outcome.intents_deleted,
        idp_revoked: outcome.idp_revoked,
    }
}

/// Build the revocation URL for the active provider mode.
///
/// `LinkProvider` carries no mode field, so mode comes from `AuthConfig` directly, mirroring
/// `link_provider::derive`'s own `{issuer}/oauth/...` construction. In `TemperAs` mode this URL is
/// never dialled — the service revokes locally — but we still produce a well-formed value rather
/// than an `Option` the caller would have to unwrap.
fn revoke_url(state: &AppState) -> String {
    let base = state.config.auth.issuer.trim_end_matches('/');
    format!("{base}/oauth/revoke")
}

/// Disconnect the caller's own Slack link.
#[utoipa::path(
    delete,
    path = "/api/auth/slack/link/me",
    tag = "Slack Link",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Disconnect completed (idempotent)", body = SlackDisconnectResponse),
        (status = 401, description = "Authentication required", body = temper_services::error::ErrorBody),
        (status = 503, description = "Slack account linking is not configured", body = temper_services::error::ErrorBody),
    )
)]
pub async fn disconnect_me(
    State(state): State<AppState>,
    auth: AuthUser,
) -> ApiResult<Json<SlackDisconnectResponse>> {
    let cfg = state
        .config
        .slack_link
        .as_ref()
        .ok_or_else(|| ApiError::Unauthorized("slack link disabled".to_string()))?;

    let profile_id = ProfileId::from(auth.0.profile.id);

    // Derive the principal from the caller's OWN link row. This is the whole
    // authorization story for the self-serve arm: there is no input to forge.
    let principal: Option<String> = sqlx::query_scalar!(
        r#"
        SELECT auth_provider_user_id
          FROM kb_profile_auth_links
         WHERE profile_id = $1
           AND auth_provider = $2
        "#,
        auth.0.profile.id,
        SLACK_AUTH_PROVIDER
    )
    .fetch_optional(&state.pool)
    .await?;

    let Some(principal) = principal else {
        // Idempotent: nothing linked, nothing to do.
        return Ok(Json(SlackDisconnectResponse {
            was_linked: false,
            grant_deleted: false,
            intents_deleted: 0,
            idp_revoked: false,
        }));
    };

    tracing::info!(%profile_id, "self-serve slack disconnect requested");

    let outcome = disconnect_slack_principal(
        &state.pool,
        DisconnectRequest {
            slack_principal_id: &principal,
            key: &cfg.vault_key,
            mode: state.config.auth.mode,
            revoke_url: revoke_url(&state),
            client_id: &cfg.client_id,
        },
    )
    .await?;

    Ok(Json(to_response(outcome)))
}

/// Disconnect any principal. Operator path — offboarding and stuck users.
#[utoipa::path(
    post,
    path = "/api/admin/slack/links/disconnect",
    tag = "Slack Link",
    security(("bearer_auth" = [])),
    request_body = SlackDisconnectRequest,
    responses(
        (status = 200, description = "Disconnect completed (idempotent)", body = SlackDisconnectResponse),
        (status = 403, description = "Caller is not a system admin", body = temper_services::error::ErrorBody),
    )
)]
pub async fn admin_disconnect(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SlackDisconnectRequest>,
) -> ApiResult<Json<SlackDisconnectResponse>> {
    // Auth before any mutation. Load-bearing: the gated router admits everyone
    // under access_mode='open' (routes.rs:168-169), so this is the real gate.
    if !access_service::is_system_admin(&state.pool, ProfileId::from(auth.0.profile.id)).await? {
        return Err(ApiError::Forbidden);
    }

    let cfg = state
        .config
        .slack_link
        .as_ref()
        .ok_or_else(|| ApiError::Unauthorized("slack link disabled".to_string()))?;

    crate::handlers::slack_link::validate_slack_principal(&body.slack_principal_id)?;

    tracing::info!(
        principal = %body.slack_principal_id,
        actor = %auth.0.profile.id,
        "admin slack disconnect requested"
    );

    let outcome = disconnect_slack_principal(
        &state.pool,
        DisconnectRequest {
            slack_principal_id: &body.slack_principal_id,
            key: &cfg.vault_key,
            mode: state.config.auth.mode,
            revoke_url: revoke_url(&state),
            client_id: &cfg.client_id,
        },
    )
    .await?;

    Ok(Json(to_response(outcome)))
}

/// Response for the intents reaper cron.
#[derive(Debug, serde::Serialize)]
pub struct ReapSummary {
    pub swept: i64,
}

/// Cron: sweep expired and consumed Slack link intents.
///
/// Undocumented (no `#[utoipa::path]`) and mounted on the bare internal router, matching the embed
/// crons. Vercel Cron invokes with GET; POST exists for manual ops. Gated by the shared
/// `EMBED_DISPATCH_SECRET` bearer via `require_dispatch_secret` — see that function's doc comment
/// for why this reuses the embed crons' secret rather than minting a new one.
pub async fn reap_intents(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> ApiResult<Json<ReapSummary>> {
    crate::handlers::embed::require_dispatch_secret(&state, &headers, "slack intents reap")?;
    let swept =
        temper_services::services::slack_disconnect_service::reap_expired_intents(&state.pool)
            .await?;
    Ok(Json(ReapSummary { swept }))
}
