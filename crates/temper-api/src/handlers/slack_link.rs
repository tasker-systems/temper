//! The Slack account-link flow. Two endpoints, two audiences.
//!
//! `create_link_intent` is server-to-server (the mention agent, HMAC-gated). `callback` is
//! browser-facing and renders HTML, never JSON — a human is looking at it.

use std::time::Duration;

use axum::extract::{Query, State};
use axum::response::{Html, IntoResponse, Response};
use axum::Json;
use jsonwebtoken::{decode, TokenData};

use temper_auth::{build_authorize_url, generate_pkce_pair, AuthorizeParams};
use temper_core::types::AuthenticatedProfile;
use temper_services::auth::{AuthzError, RawJwtClaims};
use temper_services::error::ApiError;
use temper_services::services::slack_link_service;
use temper_services::state::AppState;
use temper_services::{link_provider, oauth_client};

/// How long a link URL stays usable. Long enough for a human to notice the ephemeral
/// message and finish a browser login; short enough to bound a stolen one.
const INTENT_TTL: Duration = Duration::from_secs(15 * 60);

/// The scopes the link grant requests. `offline_access` is what makes the exchange return a
/// refresh token — T2 obtains the grant; T3 vaults it.
///
/// `email` and `profile` are here because the email ladder is a HARD failure and runs before
/// resolution: with `openid` alone, Auth0's `/userinfo` returns no email, so the ladder's last
/// rung cannot succeed and every link attempt dies there. They match the Auth0-mode CLI
/// (`temper-cli/src/commands/init.rs`), and the temper AS advertises all four in
/// `scopes_supported` (`packages/temper-cloud/src/oauth/metadata.ts`), so this is safe in
/// both modes.
const LINK_SCOPES: [&str; 4] = ["openid", "profile", "email", "offline_access"];

/// Cap on `slack_principal_id`, matching `kb_profile_auth_links.auth_provider_user_id`'s
/// `VARCHAR(128)`. The intents table is TEXT, so without this a too-long principal mints an
/// intent, survives the exchange, BURNS the state, and only then fails at the final upsert —
/// the user sees a save error and has to start over. Reject at the door instead.
const MAX_SLACK_PRINCIPAL_LEN: usize = 128;

/// The prefix every eve Slack principal carries, whatever its segment count.
const SLACK_PRINCIPAL_PREFIX: &str = "slack:";

#[derive(Debug, serde::Deserialize)]
pub struct CreateLinkIntentRequest {
    /// The WHOLE opaque principal from `attributes` — 2-4 segments, never split.
    pub slack_principal_id: String,
}

#[derive(Debug, serde::Serialize)]
pub struct CreateLinkIntentResponse {
    pub authorize_url: String,
}

/// `POST /internal/slack/link-intents` — mint a PKCE pair + opaque state, return the IdP URL.
///
/// Gated by `require_slack_link_signature`. The signature covers THIS call, not the URL the
/// user later clicks: `internal_sig`'s skew window is 30s and a human clicks minutes later,
/// so signing the user-facing URL would force us to loosen a gate that is tight for good
/// reason. What the user receives is the IdP's own authorize URL with an opaque state.
pub async fn create_link_intent(
    State(state): State<AppState>,
    Json(req): Json<CreateLinkIntentRequest>,
) -> Result<Json<CreateLinkIntentResponse>, ApiError> {
    validate_slack_principal(&req.slack_principal_id)?;

    let cfg = state
        .config
        .slack_link
        .as_ref()
        .ok_or_else(|| ApiError::Unauthorized("slack link disabled".to_string()))?;
    let provider = link_provider::derive(&state.config.auth, cfg);

    let (verifier, challenge) = generate_pkce_pair();
    let state_nonce = slack_link_service::create_intent(
        &state.pool,
        &req.slack_principal_id,
        &verifier,
        INTENT_TTL,
    )
    .await?;

    let authorize_url = build_authorize_url(&AuthorizeParams {
        authorize_url: provider.authorize_url,
        client_id: provider.client_id,
        audience: Some(state.config.auth.audience.clone()),
        redirect_uri: provider.redirect_uri,
        scopes: LINK_SCOPES.iter().map(|s| (*s).to_string()).collect(),
        state: state_nonce,
        code_challenge: challenge,
    })
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    Ok(Json(CreateLinkIntentResponse { authorize_url }))
}

/// Reject a malformed principal HERE, not at the final INSERT.
///
/// Three checks, all shape and none semantic: non-empty, within the storage column's width,
/// and carrying the `slack:` prefix. The principal is OPAQUE — 2 to 4 segments depending on
/// whether a team id is present and whether the author is a bot — so it is deliberately
/// NEVER split on ':'. A prefix check plus a length check is the whole of what is knowable
/// without parsing something we have no business parsing.
fn validate_slack_principal(principal: &str) -> Result<(), ApiError> {
    if principal.is_empty() {
        return Err(ApiError::BadRequest(
            "slack_principal_id must not be empty".to_string(),
        ));
    }
    if principal.len() > MAX_SLACK_PRINCIPAL_LEN {
        return Err(ApiError::BadRequest(format!(
            "slack_principal_id exceeds the maximum length of {MAX_SLACK_PRINCIPAL_LEN} bytes"
        )));
    }
    if !principal.starts_with(SLACK_PRINCIPAL_PREFIX) {
        return Err(ApiError::BadRequest(format!(
            "slack_principal_id must start with '{SLACK_PRINCIPAL_PREFIX}'"
        )));
    }
    Ok(())
}

#[derive(Debug, serde::Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
}

/// `GET /api/auth/slack/callback` — the registered redirect_uri.
///
/// Renders HTML on every path. Never JSON, and never a redirect back to Slack: the human is
/// already looking at this page, so it IS the confirmation. temper-api holds no Slack
/// credential and knows no channel.
pub async fn callback(State(state): State<AppState>, Query(q): Query<CallbackQuery>) -> Response {
    match run_callback(&state, q).await {
        Ok(slug) => page(
            "✅ Connected",
            &format!(
                "Linked as <strong>@{}</strong>. You can close this tab and go back to Slack.",
                html_escape(&slug)
            ),
        ),
        Err(message) => page("Not connected", &html_escape(&message)),
    }
}

/// The flow proper. Returns the linked profile's slug, or a user-actionable message.
///
/// `Profile` has NO `handle` field — it is `slug` (`temper-core/src/types/profile.rs:23`).
/// The `handle` on the team types is a different thing; do not reach for it.
///
/// Every `Err` string here is shown to a human, so none of them may reveal whether a given
/// profile exists.
async fn run_callback(state: &AppState, q: CallbackQuery) -> Result<String, String> {
    if let Some(err) = q.error {
        tracing::warn!(error = %err, "slack link: IdP returned an error");
        return Err("The sign-in was cancelled or refused. Mention @temper again to retry.".into());
    }

    let (Some(code), Some(state_nonce)) = (q.code, q.state) else {
        return Err("That link is incomplete. Mention @temper again to get a fresh one.".into());
    };

    let cfg = state
        .config
        .slack_link
        .as_ref()
        .ok_or_else(|| "Account linking is not configured on this instance.".to_string())?;
    let provider = link_provider::derive(&state.config.auth, cfg);

    // Single-use + TTL + unguessability, in one atomic burn. Unknown, expired and replayed
    // are indistinguishable here BY DESIGN — do not try to tell the user which it was.
    let intent = slack_link_service::consume_intent(&state.pool, &state_nonce)
        .await
        .map_err(|_| "Something went wrong. Mention @temper again to retry.".to_string())?
        .ok_or_else(|| {
            tracing::warn!("slack link: rejected (unknown, expired or replayed state)");
            "That link has expired or was already used. Mention @temper again to get a fresh one."
                .to_string()
        })?;

    let tokens = oauth_client::exchange_code(
        &provider.token_url,
        &provider.client_id,
        &code,
        &intent.code_verifier,
        &provider.redirect_uri,
    )
    .await
    .map_err(|_| "Sign-in could not be completed. Mention @temper again to retry.".to_string())?;

    // LOOKUP-ONLY. Connecting Slack is not a registration route (spec D3).
    let profile = resolve_existing(state, &tokens.access_token)
        .await
        .map_err(|_| {
            "No temper account is linked to this login. Sign in at temperkb.io first, then \
             mention @temper again to connect."
                .to_string()
        })?;

    // Auth before write: the profile is resolved and gated above this line.
    slack_link_service::upsert_slack_link(
        &state.pool,
        profile.profile.id,
        &intent.slack_principal_id,
    )
    .await
    .map_err(|_| "Something went wrong saving the link. Mention @temper again.".to_string())?;

    // T3 SEAM. The exchange requested `offline_access`, so `tokens.refresh_token` is the
    // per-user grant -- its own independent family, never an export of the user's CLI grant.
    // T3 encrypts and stores it here, keyed by slack_principal_id, and adds refresh.
    // T2 deliberately does not persist it: identity (the row above) and secret (T3's vault)
    // stay in separate tables, and kb_profile_auth_links must never grow a secret column.
    tracing::info!(
        profile_id = %profile.profile.id,
        grant_received = tokens.refresh_token.is_some(),
        "slack link: established",
    );

    Ok(profile.profile.slug.clone())
}

/// Verify the freshly-exchanged access token and resolve the profile it names — **lookup-only**.
///
/// The token arrived over our own back-channel exchange, which is exactly the reasoning that
/// must not be relied on: a token we did not verify is a token we did not authenticate,
/// whatever channel delivered it. So this walks the identical JWKS path as
/// `middleware::auth::require_auth` — same key store, same algorithm-scoped validation, same
/// `decode` — before handing the verified claims to the seam.
///
/// `authenticate_token_existing_only`, never `authenticate_token`: the latter auto-provisions
/// a profile, which on a stray click would mint an account and confer auto-join team reach.
/// Linking an existing identity is not a registration route.
async fn resolve_existing(
    state: &AppState,
    access_token: &str,
) -> Result<AuthenticatedProfile, ApiError> {
    let vk = state.jwks_store.get_decoding_key().await.map_err(|e| {
        tracing::error!("slack link: JWKS key retrieval failed: {e}");
        ApiError::Unauthorized("Authentication service unavailable".to_string())
    })?;

    let issuer = &state.config.auth.issuer;
    let audience = state.config.auth.audience.as_str();
    let validation = state.jwks_store.validation(issuer, audience, vk.algorithm);

    let token_data: TokenData<RawJwtClaims> =
        decode(access_token, &vk.key, &validation).map_err(|e| {
            tracing::debug!("slack link: JWT verification failed: {e}");
            ApiError::Unauthorized("Invalid or expired token".to_string())
        })?;
    let raw = token_data.claims;

    temper_services::auth::authenticate_token_existing_only(state, &raw, access_token)
        .await
        .map_err(|e| {
            // Every arm collapses to ONE refusal, here and in the caller, so that no page
            // ever reveals whether a profile exists (D3). Mapping the arms to distinct
            // `ApiError`s would only look like it did something: `run_callback` discards
            // this value and renders a single fixed sentence. The seam has already logged
            // each reason with the `sub`.
            //
            // The one arm worth distinguishing is `Deactivated`, and what makes it worth it
            // is the log line, not the error — a deactivated profile reaching the link flow
            // is an operator-visible event.
            if let AuthzError::Deactivated { profile_id } = e {
                tracing::warn!(%profile_id, "slack link: rejected (profile is deactivated)");
            }
            ApiError::Unauthorized("Invalid or expired token".to_string())
        })
}

fn page(title: &str, body: &str) -> Response {
    Html(format!(
        "<!doctype html><html><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{title} · temper</title></head>\
         <body style=\"font-family:system-ui,sans-serif;max-width:32rem;margin:4rem auto;padding:0 1rem;line-height:1.5\">\
         <h1>{title}</h1><p>{body}</p></body></html>"
    ))
    .into_response()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All four principal shapes eve emits (2-4 segments) pass. If any of these were
    /// rejected the guard would be parsing, which is exactly what it must not do.
    #[test]
    fn accepts_every_shape_of_real_principal() {
        for p in [
            "slack:T123:U456",     // team + human
            "slack:T123:bot:U456", // team + bot
            "slack:U456",          // no team + human
            "slack:bot:U456",      // no team + bot
        ] {
            assert!(validate_slack_principal(p).is_ok(), "rejected {p}");
        }
    }

    #[test]
    fn rejects_empty() {
        assert!(validate_slack_principal("").is_err());
    }

    #[test]
    fn rejects_a_principal_wider_than_the_storage_column() {
        // 128 fits; 129 does not — the boundary is the VARCHAR(128) that would otherwise
        // fail at the upsert, after the state was already burned.
        let at_limit = format!("slack:{}", "u".repeat(MAX_SLACK_PRINCIPAL_LEN - 6));
        assert_eq!(at_limit.len(), MAX_SLACK_PRINCIPAL_LEN);
        assert!(validate_slack_principal(&at_limit).is_ok());

        let over = format!("slack:{}", "u".repeat(MAX_SLACK_PRINCIPAL_LEN));
        assert!(validate_slack_principal(&over).is_err());
    }

    #[test]
    fn rejects_a_foreign_prefix() {
        assert!(validate_slack_principal("discord:T123:U456").is_err());
        assert!(validate_slack_principal("U456").is_err());
    }
}
