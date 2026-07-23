//! The mention path's act-as-the-human mint — T4's single sanctioned door to the grant vault.
//!
//! [`super::slack_grant_vault_service::mint_access_token`] is the persistence primitive: it locks the row, spends the refresh token, and re-seals what
//! rotation returned. This module is the *service* above it — it resolves the instance's Slack
//! config, derives the OAuth endpoints for the current auth mode, and maps the outcome. A surface
//! calls this; a surface never assembles a mint itself.
//!
//! # Why the authorization gate here is a *precondition*, not a predicate
//!
//! The disconnect path's admin gate lives in its service rather than its handler, deliberately, so
//! that the anticipated `@temper disconnect` Slack surface cannot ship without it
//! (`slack_disconnect.rs`). The instinct to mirror that here is right, but the thing being
//! enforced is not the same shape, and pretending otherwise would produce a gate that proves
//! nothing.
//!
//! An admin gate is a **predicate on an argument**: given `actor`, `is_system_admin(actor)` is
//! answerable from the value itself. The mint's rule is not — it is a claim about where the
//! principal *came from*:
//!
//! > Naming a principal must not be sufficient to mint its token.
//!
//! Provenance is extrinsic. Handed `"slack:T123:U456"`, no function can tell whether it was read
//! off a Slack-signed webhook or typed by an attacker; the string is identical either way. So
//! there is no argument predicate to write, and a service-side check would be theatre.
//!
//! What actually enforces it is the **transport**: `/internal/slack/mint` is mounted only inside
//! `slack_mint_internal_routes`, which is layered with `require_slack_mint_signature` at every
//! merge site, keyed on a secret distinct from `SLACK_LINK_SECRET`. Only a holder of that secret
//! can reach this code, and the sole holder is the mention agent, which derives the principal from
//! eve's verified `app_mention` rather than from any client-supplied field.
//!
//! **That rule is now also STRUCTURAL, via [`VerifiedSlackPrincipal`].** A newtype was once
//! considered and rejected here on the premise that "its constructor must be `pub` for the handler
//! (a different crate) to call it, so any code could mint the proof alongside the claim." That
//! premise was true when written and is now stale: the middleware→extensions pattern shipped five
//! commits later (`AuthenticatedProfile`, sealed in this crate and extracted in temper-api), and a
//! `pub` struct with a *private field* is nameable and extractable across the crate boundary while
//! being un-forgeable — a struct literal outside this module is a compile error (proven by
//! `tests/compile_fail/forge_verified_slack_principal.rs`). The honest shape the old comment named
//! as "a larger change than T4 warrants" is exactly what [`verify_mint_request`] now is: the HMAC
//! verify moved into THIS crate, so the sole way to obtain the proof is to pass a request that
//! verifies against the mint secret.
//!
//! What that does and does not buy, stated precisely (spec §5.3): it does NOT make the principal
//! string more trustworthy — provenance is still extrinsic. It makes **calling the mint with a
//! principal that did not come through the gate** unrepresentable — the enclosure's class of bug,
//! not a wire-level upgrade. Possession of `SLACK_MINT_SECRET` remains the wire-level enforcement.
//! `mint_for_mention` still authorizes nothing; a test that calls it directly still proves nothing
//! about authorization — the authorization now lives in `verify_mint_request` and the route.

use sqlx::PgPool;

use temper_core::internal_sig;

use super::slack_grant_vault_service::{self, MintOutcome};
use super::slack_link_service::validate_slack_principal;
use crate::config::ApiConfig;
use crate::error::{ApiError, ApiResult};
use crate::link_provider;

/// Proof that a Slack principal arrived on a signature-verified mint request.
///
/// SEALED: the field is private and [`verify_mint_request`] is this module's only constructor, so a
/// struct-literal forgery outside `slack_mint_service` is a compile error. It is the same kind of
/// thing as [`crate::auth::SystemAdmin`] — a proof obtainable only from the gate that establishes
/// it — and it crosses the crate boundary the same way `AuthenticatedProfile` does: nameable and
/// `Clone`-able in temper-api (so the mint gate can insert it into request extensions and the
/// handler can extract it), un-constructible there.
///
/// `Clone` is required for axum's `Extension<T>` extractor; the payload is one opaque principal, no
/// secret, so a derived `Debug` is safe.
#[derive(Clone, Debug)]
pub struct VerifiedSlackPrincipal {
    id: String,
}

impl VerifiedSlackPrincipal {
    /// The verified Slack principal, WHOLE — the only value the mint may act on. Never re-parse or
    /// split it.
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// The mint request body. Parsed inside [`verify_mint_request`] rather than by the handler, so the
/// principal never exists as a client-supplied field the handler could read: the handler receives
/// only the sealed proof.
#[derive(serde::Deserialize)]
struct MintRequestBody {
    slack_principal_id: String,
}

/// Verify a mint request end to end and seal the principal — the **sole** constructor of
/// [`VerifiedSlackPrincipal`], and the whole of what enforces *"naming a principal must not be
/// sufficient to mint its token."*
///
/// The HMAC verify lives HERE, in temper-services, not in the temper-api middleware, and that is
/// what makes the seal real rather than decorative (spec §5.1): the only way for the transport
/// crate to obtain a proof is to pass a request that verifies against the mint secret. A `pub`
/// constructor that trusted pre-verified bytes would be forgeable by any temper-api code; this one
/// cannot be, because it does the verification itself.
///
/// Checks, in order: fresh timestamp, valid HMAC over the exact bytes, a parseable body, a
/// well-shaped principal. A stale timestamp or bad signature is [`ApiError::Unauthorized`]; a
/// malformed body or principal is [`ApiError::BadRequest`] (400) — the shape error stays distinct
/// from the auth error (`mint_rejects_a_malformed_principal`).
///
/// **Cross-gate containment is structural.** The other two signature gates never call this, so a
/// `SLACK_LINK_SECRET` or reconcile-secret holder cannot obtain a proof: the wrong secret fails the
/// HMAC verify here, and no other code path constructs one. That is separation, not a discipline
/// applied inside a shared helper — see the unit tests and `tests/compile_fail`.
pub fn verify_mint_request(
    secret: &str,
    timestamp: i64,
    now: i64,
    body: &[u8],
    signature: &str,
) -> ApiResult<VerifiedSlackPrincipal> {
    if !internal_sig::timestamp_is_fresh(timestamp, now) {
        return Err(ApiError::Unauthorized(
            "stale internal signature".to_string(),
        ));
    }
    if !internal_sig::verify(secret.as_bytes(), timestamp, body, signature) {
        return Err(ApiError::Unauthorized(
            "invalid internal signature".to_string(),
        ));
    }
    let parsed: MintRequestBody = serde_json::from_slice(body)
        .map_err(|_| ApiError::BadRequest("malformed mint request body".to_string()))?;
    validate_slack_principal(&parsed.slack_principal_id)?;
    Ok(VerifiedSlackPrincipal {
        id: parsed.slack_principal_id,
    })
}

/// Mint an access token for a Slack principal that arrived on a signature-verified mention.
///
/// `slack_principal_id` is the WHOLE opaque principal (`slack:<team>:<user>`), never split — the
/// same value `kb_profile_auth_links.auth_provider_user_id` is keyed on.
///
/// Returns [`ApiError::Unauthorized`] when the instance has no Slack link configuration at all:
/// without `SLACK_VAULT_ENC_KEY` there is no key to unseal a grant with, so minting is not
/// merely unconfigured but impossible. That mirrors `slack_link_state`'s treatment of the same
/// missing config.
pub async fn mint_for_mention(
    pool: &PgPool,
    config: &ApiConfig,
    slack_principal_id: &str,
) -> ApiResult<MintOutcome> {
    let cfg = config
        .slack_link
        .as_ref()
        .ok_or_else(|| ApiError::Unauthorized("slack link disabled".to_string()))?;

    // The same public client that minted the grant, resolved from THIS instance's auth mode
    // rather than stored on the row — a config change that invalidates old grants invalidates
    // them at the IdP too.
    let provider = link_provider::derive(&config.auth, cfg);

    slack_grant_vault_service::mint_access_token(
        pool,
        &cfg.vault_key,
        &provider.token_url,
        &provider.client_id,
        slack_principal_id,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINT_SECRET: &str = "mint-secret";
    const LINK_SECRET: &str = "link-secret-DIFFERENT";
    const PRINCIPAL: &str = "slack:T0BHAHEN79C:U0BH6A3L6JF";

    /// Build a signed mint request body + its headers, as the agent's `signIntentRequest` would.
    fn signed(secret: &str, timestamp: i64, principal: &str) -> (Vec<u8>, String) {
        let body = serde_json::to_vec(&serde_json::json!({ "slack_principal_id": principal }))
            .expect("serialize body");
        let signature = internal_sig::sign(secret.as_bytes(), timestamp, &body);
        (body, signature)
    }

    #[test]
    fn a_valid_mint_signature_yields_the_sealed_principal() {
        let now = 1_700_000_000;
        let (body, sig) = signed(MINT_SECRET, now, PRINCIPAL);
        let verified =
            verify_mint_request(MINT_SECRET, now, now, &body, &sig).expect("should verify");
        assert_eq!(verified.id(), PRINCIPAL);
    }

    /// Cross-gate containment: a request signed with a DIFFERENT secret (the link-state key) does
    /// not verify, so a `SLACK_LINK_SECRET` holder cannot obtain a mint proof. This is the negative
    /// test the design requires (spec §5.2) — the seal's only constructor gates on the mint secret.
    #[test]
    fn a_link_secret_signature_cannot_mint_a_proof() {
        let now = 1_700_000_000;
        // Signed with the LINK secret, verified against the MINT secret — the cross-gate attempt.
        let (body, sig) = signed(LINK_SECRET, now, PRINCIPAL);
        let err = verify_mint_request(MINT_SECRET, now, now, &body, &sig)
            .expect_err("a link-secret signature must not mint a proof");
        assert!(
            matches!(err, ApiError::Unauthorized(_)),
            "wrong secret must be Unauthorized, got {err:?}",
        );
    }

    #[test]
    fn a_stale_timestamp_is_refused() {
        let signed_at = 1_700_000_000;
        let now = signed_at + internal_sig::MAX_SKEW_SECS + 1;
        let (body, sig) = signed(MINT_SECRET, signed_at, PRINCIPAL);
        let err = verify_mint_request(MINT_SECRET, signed_at, now, &body, &sig)
            .expect_err("a stale timestamp must be refused");
        assert!(matches!(err, ApiError::Unauthorized(_)), "got {err:?}");
    }

    /// A well-signed request whose principal is malformed is a 400, NOT a 401 — the shape error must
    /// stay distinct from the auth error (`mint_rejects_a_malformed_principal` asserts the 400).
    #[test]
    fn a_malformed_principal_is_bad_request_not_unauthorized() {
        let now = 1_700_000_000;
        let (body, sig) = signed(MINT_SECRET, now, "not-a-slack-principal");
        let err = verify_mint_request(MINT_SECRET, now, now, &body, &sig)
            .expect_err("a malformed principal must be refused");
        assert!(
            matches!(err, ApiError::BadRequest(_)),
            "a malformed principal must be BadRequest, got {err:?}",
        );
    }

    #[test]
    fn a_body_that_is_not_json_is_bad_request() {
        let now = 1_700_000_000;
        let body = b"not json at all";
        let sig = internal_sig::sign(MINT_SECRET.as_bytes(), now, body);
        let err = verify_mint_request(MINT_SECRET, now, now, body, &sig)
            .expect_err("a non-JSON body must be refused");
        assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
    }
}
