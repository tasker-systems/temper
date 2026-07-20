//! `POST /internal/slack/mint` — vend an act-as-the-human access token to the mention agent.
//!
//! Its own module rather than a third handler on `slack_link.rs`: that file is the *link* flow
//! (intent, callback, link-state) and is already long. This is the mention path's read of an
//! established link, and it is the one endpoint in the Slack surface that hands back a
//! credential.

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};

use temper_services::error::ApiResult;
use temper_services::services::slack_grant_vault_service::MintOutcome;
use temper_services::services::slack_mint_service;
use temper_services::state::AppState;

use super::slack_link::validate_slack_principal;

/// The mention agent's mint request: one opaque principal, exactly as
/// [`crate::handlers::slack_link::SlackLinkStateRequest`] carries it.
#[derive(Debug, Deserialize)]
pub struct SlackMintRequest {
    /// The WHOLE opaque principal (`slack:<team>:<user>`), never split.
    pub slack_principal_id: String,
}

/// What the agent should do next, as a tagged union rather than a nullable token.
///
/// Mirrors [`MintOutcome`]'s three-way shape deliberately: "no grant on file" and "the grant was
/// revoked" are different facts a user needs different sentences for, and collapsing them into
/// `null` would force the agent to say something vague about both. Neither is an error, so
/// neither is an HTTP failure — a 200 carrying `not_vaulted` is the honest encoding of *"the
/// request was fine; there is nothing to mint."*
///
/// `Debug` is hand-written to REDACT the token: this is the exact value the mention path handles,
/// and a stray `?response` in a log would write an act-as-the-human credential to disk. The same
/// reasoning as `MintOutcome`'s own `Debug`, applied at the wire boundary.
#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SlackMintResponse {
    /// A live token the agent may present to temper as this human.
    Token {
        access_token: String,
        /// Absolute expiry, epoch **milliseconds** — the unit eve's `TokenResult.expiresAt`
        /// expects (`Date.now()`-comparable). Converted here rather than in the agent so the
        /// wire contract is unit-explicit and the TS side does no arithmetic on it.
        expires_at_ms: i64,
    },
    /// A vault row exists but is not mintable: explicitly revoked, or the profile deactivated.
    /// The user must re-link; retrying will never succeed.
    Revoked,
    /// No grant is vaulted for this principal — linked before T3 shipped, or the IdP returned no
    /// refresh token at link time (`slack_link.rs`, where the directory row stands and only a
    /// `warn!` fires). **This is reachable for a user whom `link-state` calls `linked`**, which is
    /// exactly why it is its own arm: the agent must not tell such a user things are working.
    NotVaulted,
}

impl std::fmt::Debug for SlackMintResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Token { expires_at_ms, .. } => {
                write!(f, "Token(redacted, expires_at_ms={expires_at_ms})")
            }
            Self::Revoked => f.write_str("Revoked"),
            Self::NotVaulted => f.write_str("NotVaulted"),
        }
    }
}

impl From<MintOutcome> for SlackMintResponse {
    fn from(outcome: MintOutcome) -> Self {
        match outcome {
            MintOutcome::Token {
                access_token,
                expires_at,
            } => Self::Token {
                access_token,
                expires_at_ms: expires_at.timestamp_millis(),
            },
            MintOutcome::Revoked => Self::Revoked,
            MintOutcome::NotVaulted => Self::NotVaulted,
        }
    }
}

/// `POST /internal/slack/mint` — mint an access token for a mentioning Slack user.
///
/// **Gated by `require_slack_mint_signature`, on a key distinct from `SLACK_LINK_SECRET`.** That
/// gate is not incidental: it is the whole of what enforces *"naming a principal must not be
/// sufficient to mint its token."* The principal in the body is trusted precisely because only a
/// holder of the mint secret could have put it there, and the sole holder derives it from eve's
/// signature-verified `app_mention` rather than from anything a Slack user can type. See
/// `slack_mint_service` for why this cannot instead be a predicate in the service layer.
///
/// Thin by intent (`temper-api` is transport): validate the principal's shape, dispatch to the
/// service, map the outcome. No SQL, no cipher, no provider derivation here.
pub async fn slack_mint(
    State(state): State<AppState>,
    Json(req): Json<SlackMintRequest>,
) -> ApiResult<Json<SlackMintResponse>> {
    validate_slack_principal(&req.slack_principal_id)?;

    let outcome =
        slack_mint_service::mint_for_mention(&state.pool, &state.config, &req.slack_principal_id)
            .await?;

    // Deliberately NOT logged with the principal at info level: a mint is per-mention, so an
    // info-per-mint would build a per-user activity trail in the platform log. The ledger is
    // where act-attribution belongs.
    tracing::debug!(outcome = ?outcome, "slack mint");

    Ok(Json(outcome.into()))
}
