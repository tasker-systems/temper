//! Disconnect surfaces — self-serve, admin, and the intents reaper cron.
//!
//! Both `disconnect_me` and `admin_disconnect` dispatch to the one service chokepoint. The
//! self-serve arm never accepts a principal from the caller: it derives them from the caller's own
//! auth-link rows, so naming someone else's principal is not expressible.
//!
//! Both return the same `{ "disconnected": [...] }` shape — 0..n entries self-serve, 0..1 admin —
//! so an SDK consumer writes one code path. An empty list is a SUCCESS: disconnect is idempotent.

use axum::extract::State;
use axum::Json;
use temper_core::types::ids::ProfileId;
use temper_core::types::slack::{
    SlackDisconnectRequest, SlackDisconnectResponse, SlackDisconnectedPrincipal,
};
use temper_services::error::{ApiError, ApiResult};
use temper_services::link_provider;
use temper_services::services::slack_disconnect_service::{
    admin_disconnect_slack_principal, disconnect_slack_principal, DisconnectOutcome,
    DisconnectRequest,
};
use temper_services::services::slack_link_service;
use temper_services::state::AppState;

use crate::middleware::auth::AuthUser;

/// Project one principal's outcome onto the wire shape.
///
/// `was_linked` does not survive onto the wire: the response lists the principals that were
/// actually unbound, so "nothing was linked" is the empty vector rather than a flag. Each caller
/// decides whether an outcome earns an entry.
fn entry_for(principal: String, outcome: DisconnectOutcome) -> SlackDisconnectedPrincipal {
    SlackDisconnectedPrincipal {
        slack_principal_id: principal,
        grant_deleted: outcome.grant_deleted,
        intents_deleted: outcome.intents_deleted,
        idp_revocation: outcome.idp_revocation,
    }
}

/// Disconnect EVERY Slack principal bound to the caller's own profile.
///
/// Plural on purpose. `kb_profile_auth_links` has `UNIQUE(auth_provider, auth_provider_user_id)`
/// and nothing keyed on `(profile_id, auth_provider)`, so a human in two Slack workspaces holds
/// two rows — legitimately, because the already-linked refusal keys on the *principal*. Cutting
/// only one and answering "disconnected" would leave the other grant live and still minting
/// act-as-the-human tokens, which is the opposite of what the user asked for.
///
/// The 401 arm is the disabled-link branch. There is no 503: `ApiError` has no such variant, and
/// documenting one the code cannot return is worse than documenting nothing — it is baked into
/// `openapi.json`, the Ruby gem and `schema.ts`.
#[utoipa::path(
    delete,
    path = "/api/auth/slack/link/me",
    tag = "Slack Link",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Disconnect completed (idempotent); `disconnected` lists every principal unbound", body = SlackDisconnectResponse),
        (status = 401, description = "Authentication required, or Slack account linking is not configured on this instance", body = temper_services::error::ErrorBody),
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

    let profile_id = ProfileId::from(auth.0.profile().id);
    let provider = link_provider::derive(&state.config.auth, cfg);

    // Derive the principals from the caller's OWN link rows. This is the whole
    // authorization story for the self-serve arm: there is no input to forge.
    let principals =
        slack_link_service::lookup_slack_principals_for_profile(&state.pool, auth.0.profile().id)
            .await?;

    tracing::info!(
        %profile_id,
        count = principals.len(),
        "self-serve slack disconnect requested"
    );

    let mut disconnected = Vec::with_capacity(principals.len());
    for principal in principals {
        let outcome = disconnect_slack_principal(
            &state.pool,
            DisconnectRequest {
                slack_principal_id: &principal,
                key: &cfg.vault_key,
                mode: state.config.auth.mode,
                revoke_url: provider.revoke_url.clone(),
                client_id: &cfg.client_id,
                // Self-serve: the actor IS the subject. The principals were
                // derived from this profile's own link rows.
                actor: profile_id,
            },
        )
        .await?;

        // A principal the lookup just returned but that no longer has a row lost a race with a
        // concurrent disconnect. It is not something this call unbound, so it is not reported.
        if outcome.was_linked {
            disconnected.push(entry_for(principal, outcome));
        }
    }

    Ok(Json(SlackDisconnectResponse { disconnected }))
}

/// Disconnect any principal. Operator path — offboarding and stuck users.
#[utoipa::path(
    post,
    path = "/api/admin/slack/links/disconnect",
    tag = "Slack Link",
    security(("bearer_auth" = [])),
    request_body = SlackDisconnectRequest,
    responses(
        (status = 200, description = "Disconnect completed (idempotent); `disconnected` is empty when the principal was not linked", body = SlackDisconnectResponse),
        (status = 400, description = "Malformed Slack principal", body = temper_services::error::ErrorBody),
        (status = 401, description = "Authentication required, or Slack account linking is not configured on this instance", body = temper_services::error::ErrorBody),
        (status = 403, description = "Caller is not a system admin", body = temper_services::error::ErrorBody),
    )
)]
pub async fn admin_disconnect(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<SlackDisconnectRequest>,
) -> ApiResult<Json<SlackDisconnectResponse>> {
    let cfg = state
        .config
        .slack_link
        .as_ref()
        .ok_or_else(|| ApiError::Unauthorized("slack link disabled".to_string()))?;

    temper_services::services::slack_link_service::validate_slack_principal(
        &body.slack_principal_id,
    )?;

    let provider = link_provider::derive(&state.config.auth, cfg);
    // The admin gate lives in the SERVICE, not here — see
    // `admin_disconnect_slack_principal`. A gate in this handler would be one the
    // planned `@temper disconnect` Slack surface has to remember to re-add.
    let outcome = admin_disconnect_slack_principal(
        &state.pool,
        DisconnectRequest {
            slack_principal_id: &body.slack_principal_id,
            key: &cfg.vault_key,
            mode: state.config.auth.mode,
            revoke_url: provider.revoke_url,
            client_id: &cfg.client_id,
            // The operator, not the subject. The service gates on this field
            // and carries it into the disconnect.
            actor: ProfileId::from(auth.0.profile().id),
        },
    )
    .await?;

    // 0 or 1 entries. The same shape the self-serve arm returns, so an SDK consumer writes one
    // code path for both.
    //
    // `destroyed_something`, not `was_linked`: the admin arm is the repair path, and an orphan
    // grant or a stale intent with no identity row is exactly the state an operator reaches for
    // it in. Reporting an empty list while having destroyed a live grant would tell them the
    // opposite of what happened.
    let destroyed_something =
        outcome.was_linked || outcome.grant_deleted || outcome.intents_deleted > 0;
    let disconnected = if destroyed_something {
        vec![entry_for(body.slack_principal_id, outcome)]
    } else {
        Vec::new()
    };
    Ok(Json(SlackDisconnectResponse { disconnected }))
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
