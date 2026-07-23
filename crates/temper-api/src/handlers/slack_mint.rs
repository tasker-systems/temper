//! `POST /internal/slack/mint` — vend an act-as-the-human access token to the mention agent.
//!
//! Its own module rather than a third handler on `slack_link.rs`: that file is the *link* flow
//! (intent, callback, link-state) and is already long. This is the mention path's read of an
//! established link, and it is the one endpoint in the Slack surface that hands back a
//! credential.

use axum::extract::State;
use axum::{Extension, Json};
use serde::Serialize;

use temper_core::types::slack::LinkRefusal;
use temper_services::error::ApiResult;
use temper_services::services::slack_grant_vault_service::MintOutcome;
use temper_services::services::slack_mint_service::{self, VerifiedSlackPrincipal};
use temper_services::state::AppState;

/// What the agent should do next, as a tagged union rather than a nullable token.
///
/// Two arms, mirroring [`MintOutcome`]: a `Token`, or a `Refused` carrying the typed
/// [`LinkRefusal`]. None is an error, so none is an HTTP failure — a 200 carrying a refusal is the
/// honest encoding of *"the request was fine; there is nothing to mint, and here is exactly why."*
/// The refusal reason is what lets the agent say something true and specific — "ask an admin" for a
/// standing refusal, "reconnect" for a missing grant — instead of one vague sentence for all of them.
///
/// The outer tag is `status`; `LinkRefusal`'s own tag is `reason`; `Refusal`'s is `kind`. Three
/// distinct discriminators nest without collision: `{"status":"refused","reason":"standing",
/// "refusal":{"kind":"denied"}}`.
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
    /// No token, and the typed reason. `not_linked` / `not_vaulted` / `standing` each carry a
    /// different remedy — critically, a standing refusal is fixed by an admin approval, never by
    /// re-linking, which is the false remedy the former flat `revoked` arm shipped.
    Refused(LinkRefusal),
}

impl std::fmt::Debug for SlackMintResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Token { expires_at_ms, .. } => {
                write!(f, "Token(redacted, expires_at_ms={expires_at_ms})")
            }
            // `LinkRefusal` carries no secret, so its derived `Debug` is safe to surface.
            Self::Refused(reason) => write!(f, "Refused({reason:?})"),
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
            MintOutcome::Refused(refusal) => Self::Refused(refusal),
        }
    }
}

/// `POST /internal/slack/mint` — mint an access token for a mentioning Slack user.
///
/// **The principal arrives as a sealed [`VerifiedSlackPrincipal`], not a request body.** The mint
/// signature gate (`require_slack_mint_signature`) verifies the HMAC against `SLACK_MINT_SECRET`,
/// validates the principal's shape, and seals the proof inside temper-services before this handler
/// runs — so there is no client-supplied principal field here to trust or re-validate, and "naming
/// a principal must not be sufficient to mint its token" is enforced structurally, not by a check
/// this handler must remember. See `slack_mint_service::verify_mint_request` for the seal.
///
/// Thin by intent (`temper-api` is transport): read the verified principal, dispatch to the
/// service, map the outcome. No SQL, no cipher, no provider derivation, no validation here.
pub async fn slack_mint(
    State(state): State<AppState>,
    Extension(principal): Extension<VerifiedSlackPrincipal>,
) -> ApiResult<Json<SlackMintResponse>> {
    let outcome =
        slack_mint_service::mint_for_mention(&state.pool, &state.config, principal.id()).await?;

    // Deliberately NOT logged with the principal at info level: a mint is per-mention, so an
    // info-per-mint would build a per-user activity trail in the platform log. The ledger is
    // where act-attribution belongs.
    tracing::debug!(outcome = ?outcome, "slack mint");

    Ok(Json(outcome.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use temper_principal::Refusal;

    fn json(r: &SlackMintResponse) -> String {
        serde_json::to_string(r).expect("serialize")
    }

    #[test]
    fn each_arm_serializes_to_a_distinct_status_and_reason() {
        let not_linked = json(&SlackMintResponse::Refused(LinkRefusal::NotLinked));
        let not_vaulted = json(&SlackMintResponse::Refused(LinkRefusal::NotVaulted));
        let standing = json(&SlackMintResponse::Refused(LinkRefusal::Standing {
            refusal: Refusal::Denied,
        }));

        assert!(not_linked.contains(r#""status":"refused""#), "{not_linked}");
        assert!(
            not_linked.contains(r#""reason":"not_linked""#),
            "{not_linked}"
        );
        assert!(
            not_vaulted.contains(r#""reason":"not_vaulted""#),
            "{not_vaulted}"
        );
        assert!(standing.contains(r#""reason":"standing""#), "{standing}");

        // Three genuinely different wire values — the whole point of the typed refusal.
        assert_ne!(not_linked, not_vaulted);
        assert_ne!(not_linked, standing);
        assert_ne!(not_vaulted, standing);
    }

    #[test]
    fn the_standing_refusal_nests_without_a_colliding_tag() {
        // status / reason / kind are three distinct discriminators; they must not flatten onto
        // one another. A regression to a colliding tag would drop the inner refusal here.
        let standing = json(&SlackMintResponse::Refused(LinkRefusal::Standing {
            refusal: Refusal::Revoked,
        }));
        assert!(
            standing.contains(r#""refusal":{"kind":"revoked"}"#),
            "the standing refusal must nest under `refusal`: {standing}"
        );
    }

    #[test]
    fn from_mint_outcome_preserves_the_token_and_converts_to_millis() {
        let expires_at = chrono::DateTime::<chrono::Utc>::from_timestamp(1_700_000_000, 0).unwrap();
        let resp: SlackMintResponse = MintOutcome::Token {
            access_token: "tok".to_string(),
            expires_at,
        }
        .into();
        match resp {
            SlackMintResponse::Token {
                access_token,
                expires_at_ms,
            } => {
                assert_eq!(access_token, "tok");
                assert_eq!(expires_at_ms, 1_700_000_000_000);
            }
            other => panic!("expected Token, got {other:?}"),
        }
    }

    #[test]
    fn debug_redacts_the_token() {
        let dbg = format!(
            "{:?}",
            SlackMintResponse::Token {
                access_token: "super-secret".to_string(),
                expires_at_ms: 1,
            }
        );
        assert!(
            !dbg.contains("super-secret"),
            "token must be redacted: {dbg}"
        );
    }
}
