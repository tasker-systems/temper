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
//! **A newtype was considered and rejected.** `VerifiedSlackPrincipal(String)` would make the
//! contract type-level rather than documentary — but its constructor must be `pub` for the handler
//! (a different crate) to call it, so any code could mint the proof alongside the claim. That is
//! documentation with extra steps and a false sense of a compile-time guarantee. If temper ever
//! wants this structurally, the honest shape is for the signature middleware to insert a
//! non-constructible token into request extensions, which is a larger change than T4 warrants.
//!
//! The consequence, stated so it is not lost: **the test that proves this gate must drive the
//! route, not this function.** Calling `mint_for_mention` directly bypasses the only thing
//! enforcing the rule, so a test that does so and passes has proved nothing about authorization.

use sqlx::PgPool;

use super::slack_grant_vault_service::{self, MintOutcome};
use crate::config::ApiConfig;
use crate::error::{ApiError, ApiResult};
use crate::link_provider;

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
