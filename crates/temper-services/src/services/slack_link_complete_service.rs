//! Completing a Slack account link: the identity row and the sealed grant, as one write.
//!
//! The browser callback (`temper-api`'s `slack_link::run_callback`) verifies the IdP exchange and
//! resolves the profile. It then hands this command everything the write needs. The command owns
//! the transaction, and the handler renders the outcome. The handler opens no transaction.
//!
//! **Why one transaction.** The identity row (`kb_profile_auth_links`) and the vaulted grant are
//! one fact. A link with no grant can never mint. The state nonce that would let the user retry is
//! already burned (`slack_link_service::consume_intent`) by the time either write runs. As two
//! autocommits, a process death between them left the user linked with a refresh token held only
//! in a stack frame, unrecoverable, and with no page rendered. A rollback of both is recoverable:
//! the user mentions @temper and gets a fresh intent.
//!
//! **Write order** is deliberate and is what a later edit must keep:
//!
//! 1. Bind the principal (`link_slack_principal`). A principal already bound to a *different*
//!    profile is refused here, before anything else is written.
//! 2. With no refresh token there is nothing to vault, so the link is rolled back rather than left
//!    inert. That check comes after the bind so a principal bound elsewhere still answers
//!    [`CompleteLinkOutcome::AlreadyLinkedToAnotherProfile`].
//! 3. Revoke a superseded grant (AS mode only, see `revoke_superseded_grant`), then store the new
//!    one, then commit.

use sqlx::PgPool;

use super::grant_crypto::VaultKey;
use super::{slack_disconnect_service, slack_grant_vault_service, slack_link_service};
use crate::auth::HumanPrincipal;
use crate::auth_config::AuthMode;
use crate::error::ApiError;

/// What the IdP exchange produced, as the link write needs it.
///
/// No `Debug`: it carries the refresh and access tokens, and a derived `Debug` plus a stray `?`
/// would put the durable grant in a log.
pub struct LinkGrant<'a> {
    /// The refresh token, if the IdP returned one. `None` is a client misconfiguration
    /// (`offline_access` or refresh-token rotation not enabled on the link client).
    pub refresh_token: Option<&'a str>,
    /// The access token from the same exchange.
    pub access_token: &'a str,
    /// The exchange's `expires_in`.
    pub access_ttl_secs: Option<u64>,
}

/// What a completed link attempt did. Every arm but `Linked` wrote nothing: its transaction was
/// dropped without a commit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompleteLinkOutcome {
    /// The identity row and the grant are committed together.
    Linked,
    /// The principal is already bound to a different profile. The existing binding stands.
    AlreadyLinkedToAnotherProfile,
    /// The IdP returned no refresh token, so the link could never mint. Nothing was saved.
    NoRefreshToken,
}

/// Which phase a failure came from. The callback page words the two differently, so the phase is
/// part of the answer and the command keeps it.
#[derive(Debug)]
pub enum CompleteLinkError {
    /// Opening the transaction or binding the principal failed.
    Link(ApiError),
    /// Revoking the superseded grant, storing the new one, or the commit failed.
    Grant(ApiError),
}

/// Bind `slack_principal_id` to `linker` and vault `grant`, in one transaction.
///
/// Auth before write: `linker` is a [`HumanPrincipal`] the caller has already verified, and the
/// state nonce has already been consumed. This command authorizes nothing further.
pub async fn complete_link(
    pool: &PgPool,
    vault_key: &VaultKey,
    mode: AuthMode,
    linker: &HumanPrincipal,
    slack_principal_id: &str,
    grant: LinkGrant<'_>,
) -> Result<CompleteLinkOutcome, CompleteLinkError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| CompleteLinkError::Link(e.into()))?;

    let bound = slack_link_service::link_slack_principal(&mut tx, linker, slack_principal_id)
        .await
        .map_err(CompleteLinkError::Link)?;
    if bound == slack_link_service::SlackLinkOutcome::AlreadyLinkedToAnotherProfile {
        return Ok(CompleteLinkOutcome::AlreadyLinkedToAnotherProfile);
    }

    // Dropping `tx` rolls the directory row back too. Half-linked is the worst of the three
    // states: `lookup_linked_handle` reads the auth-link table, not the vault, so a link with no
    // grant reads as "connected" and the user's next mention spends a mint that answers
    // `not_vaulted`. Unlinked, their next mention gets a fresh intent.
    let Some(refresh_token) = grant.refresh_token else {
        return Ok(CompleteLinkOutcome::NoRefreshToken);
    };

    slack_disconnect_service::revoke_superseded_grant(
        &mut tx,
        vault_key,
        mode,
        slack_principal_id,
        refresh_token,
    )
    .await
    .map_err(|e| {
        // Logged here because the page shares one sentence across revoke, store and commit;
        // without this line a failed revoke reads the same as either of the others.
        tracing::error!(error = %e, "slack link: revoking the superseded grant failed");
        CompleteLinkError::Grant(e)
    })?;

    slack_grant_vault_service::store_grant(
        &mut tx,
        vault_key,
        slack_grant_vault_service::NewGrant {
            owner: linker,
            slack_principal_id,
            refresh_token,
            access_token: grant.access_token,
            access_ttl_secs: grant.access_ttl_secs,
        },
    )
    .await
    .map_err(CompleteLinkError::Grant)?;

    tx.commit()
        .await
        .map_err(|e| CompleteLinkError::Grant(e.into()))?;
    Ok(CompleteLinkOutcome::Linked)
}
