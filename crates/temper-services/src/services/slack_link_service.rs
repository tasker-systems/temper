//! The Slack account-link flow's persistence: intent lifecycle + the directory row.
//!
//! All SQL for T2 lives here; the handlers dispatch and never touch the database.

use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};

/// The `auth_provider` value for every Slack link row. One constant so the write and
/// any future read can never disagree on the string.
pub const SLACK_AUTH_PROVIDER: &str = "slack";

/// Cap on `slack_principal_id`, matching `kb_profile_auth_links.auth_provider_user_id`'s
/// `VARCHAR(128)`. The intents table is TEXT, so without this a too-long principal mints an
/// intent, survives the exchange, BURNS the state, and only then fails at the final upsert.
pub const MAX_SLACK_PRINCIPAL_LEN: usize = 128;

/// The prefix every eve Slack principal carries, whatever its segment count.
pub const SLACK_PRINCIPAL_PREFIX: &str = "slack:";

/// Reject a malformed principal by SHAPE — non-empty, within the storage column's width, and
/// carrying the `slack:` prefix. The principal is OPAQUE (2–4 segments), so it is deliberately
/// never split on `:`; a prefix + length check is the whole of what is knowable without parsing
/// something we have no business parsing.
///
/// **Lives here, in temper-services, not at the surface** (it was in `slack_link.rs`): the mint
/// signature gate now seals a `VerifiedSlackPrincipal` in this crate
/// (`slack_mint_service::verify_mint_request`, spec §5.2), and the shape check has to run there,
/// before the proof is minted, in the same crate. The link-state and disconnect handlers call it
/// from here too. It returns `BadRequest`, and that is load-bearing — a malformed principal must be
/// a 400, distinct from the `Unauthorized` a bad signature yields (`mint_rejects_a_malformed_principal`).
pub fn validate_slack_principal(principal: &str) -> Result<(), ApiError> {
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

#[cfg(test)]
mod validate_tests {
    use super::*;

    #[test]
    fn accepts_every_shape_of_real_principal() {
        for p in [
            "slack:T123:U456",
            "slack:T123:bot:U456",
            "slack:U456",
            "slack:bot:U456",
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

/// What a successful consume yields: everything the callback needs to finish the exchange.
#[derive(Debug, Clone)]
pub struct ConsumedIntent {
    pub code_verifier: String,
    pub slack_principal_id: String,
}

/// Mint an intent and return its opaque `state_nonce`.
///
/// The nonce is a UUIDv7 rendered as text: unguessable, and time-sortable for reaping.
pub async fn create_intent(
    pool: &PgPool,
    slack_principal_id: &str,
    code_verifier: &str,
    ttl: Duration,
) -> ApiResult<String> {
    let state_nonce = Uuid::now_v7().to_string();
    let ttl_secs = ttl.as_secs() as f64;

    sqlx::query!(
        r#"
        INSERT INTO kb_slack_link_intents
            (id, state_nonce, code_verifier, slack_principal_id, expires_at)
        VALUES ($1, $2, $3, $4, now() + make_interval(secs => $5))
        "#,
        Uuid::now_v7(),
        state_nonce,
        code_verifier,
        slack_principal_id,
        ttl_secs,
    )
    .execute(pool)
    .await?;

    Ok(state_nonce)
}

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

/// Mint an intent for `slack_principal_id` and return the IdP authorize URL that carries it.
///
/// The one place a link URL is made. Two callers, and both deliver the URL ephemerally to the
/// mentioning Slack user:
/// - link-state's `unlinked` arm, the first link;
/// - the mint's `not_vaulted` refusal, a re-link for a principal that is already bound but whose
///   grant is missing or was refused by the IdP.
///
/// The second caller issues a URL for a principal that is already LINKED. That is safe because
/// [`link_slack_principal`] never rebinds: completing the URL under the same profile re-stamps the
/// link and `store_grant` replaces the dead grant, and completing it under any other login is
/// refused before anything is vaulted. A stolen re-link URL therefore cannot move the principal.
///
/// The URL is a credential. It is single-use and expires after `INTENT_TTL`, and the intent row's
/// state nonce is the only thing that ties it to this principal.
pub async fn issue_authorize_url(
    pool: &PgPool,
    auth: &crate::auth_config::AuthConfig,
    cfg: &crate::config::SlackLinkConfig,
    slack_principal_id: &str,
) -> ApiResult<String> {
    let provider = crate::link_provider::derive(auth, cfg);

    let (verifier, challenge) = temper_auth::generate_pkce_pair();
    let state_nonce = create_intent(pool, slack_principal_id, &verifier, INTENT_TTL).await?;

    temper_auth::build_authorize_url(&temper_auth::AuthorizeParams {
        authorize_url: provider.authorize_url,
        client_id: provider.client_id,
        audience: Some(auth.audience.clone()),
        redirect_uri: provider.redirect_uri,
        scopes: LINK_SCOPES.iter().map(|s| (*s).to_string()).collect(),
        state: state_nonce,
        code_challenge: challenge,
    })
    .map_err(|e| ApiError::Internal(e.to_string()))
}

/// Burn an intent and return its payload — atomically, exactly once.
///
/// The conditional UPDATE is the whole single-use mechanism: two concurrent callbacks race
/// on the same row and exactly one sees `consumed_at IS NULL`. `None` means unknown, expired
/// OR replayed — indistinguishably, which is the point. The caller must not try to tell them
/// apart, and must not say which it was.
pub async fn consume_intent(pool: &PgPool, state_nonce: &str) -> ApiResult<Option<ConsumedIntent>> {
    let row = sqlx::query!(
        r#"
        UPDATE kb_slack_link_intents
           SET consumed_at = now()
         WHERE state_nonce = $1
           AND consumed_at IS NULL
           AND expires_at > now()
        RETURNING code_verifier, slack_principal_id
        "#,
        state_nonce,
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| ConsumedIntent {
        code_verifier: r.code_verifier,
        slack_principal_id: r.slack_principal_id,
    }))
}

/// Look up the profile slug already bound to this Slack principal, if any.
///
/// This is the read that makes the mention agent's question answerable: "what do I say to this
/// person?" — rather than minting an intent for someone who linked months ago.
///
/// **Deliberately NOT filtered on `kb_profiles.is_active`.** The link genuinely exists, and a
/// deactivated profile is not an unlinked one. Reporting "unlinked" here would send the user
/// into a link flow whose callback then refuses them (`authenticate_token_existing_only`
/// rejects a deactivated profile) — an infinite, unexplained loop. Answering "linked" tells the
/// truth about the directory and lets the deactivation surface where it is actionable.
///
/// The principal is matched WHOLE. It has 2-4 segments and is never split on ':'.
///
/// Naming: the COLUMN is `kb_profiles.handle`; the Rust `Profile` maps it to `slug`
/// (`profile_service::get_by_id` selects `handle AS "slug!"`). This function returns that one
/// string, and the wire key is `handle` because that is the word a Slack user understands.
pub async fn lookup_linked_handle(
    pool: &PgPool,
    slack_principal_id: &str,
) -> ApiResult<Option<String>> {
    let row = sqlx::query!(
        r#"
        SELECT p.handle
          FROM kb_profile_auth_links l
          JOIN kb_profiles p ON p.id = l.profile_id
         WHERE l.auth_provider = $1
           AND l.auth_provider_user_id = $2
        "#,
        SLACK_AUTH_PROVIDER,
        slack_principal_id,
    )
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| r.handle))
}

/// Every Slack principal bound to `profile_id`. The mirror image of
/// [`lookup_linked_handle`], which goes principal → profile.
///
/// **Returns ALL of them, and that plurality is load-bearing.**
/// `kb_profile_auth_links` carries `UNIQUE(auth_provider, auth_provider_user_id)`
/// and nothing else, so one profile legitimately holds several Slack principals:
/// a human in two Slack workspaces has two distinct `slack:<team>:<user>` ids,
/// and the already-linked refusal is keyed on the *principal*, so the second
/// workspace links normally. A self-serve disconnect that took only the first
/// row would cut an arbitrary one, report success, and leave the other grant
/// live and minting act-as-the-human tokens.
///
/// `ORDER BY linked_at` (then `id`, which is the tiebreak that makes it a total
/// order — two links written in the same transaction share a `now()`) so the
/// result is deterministic rather than whatever the planner streams first.
///
/// Principals are returned WHOLE, never split on ':'.
pub async fn lookup_slack_principals_for_profile(
    pool: &PgPool,
    profile_id: Uuid,
) -> ApiResult<Vec<String>> {
    let rows = sqlx::query!(
        r#"
        SELECT auth_provider_user_id
          FROM kb_profile_auth_links
         WHERE profile_id = $1
           AND auth_provider = $2
         ORDER BY linked_at, id
        "#,
        profile_id,
        SLACK_AUTH_PROVIDER,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(|r| r.auth_provider_user_id).collect())
}

/// What a link attempt did. A typed outcome rather than a bool: the refusal is not an error
/// (nothing went wrong) and not a success (nothing was written), and the handler needs to say
/// something specific about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlackLinkOutcome {
    /// The row now binds this principal to the requested profile — freshly inserted, or
    /// already there and re-stamped.
    Linked,
    /// The principal is already bound to a DIFFERENT profile. Nothing was written; the
    /// existing binding stands.
    AlreadyLinkedToAnotherProfile,
}

/// Write the directory row `slack:<team>:<user> -> profile`. **The principal binds once.**
///
/// There is no rebind. A temper profile is only ever SAML/OAuth-provisioned and identity
/// converges on one profile per human (auth links + email reconcile), so there is no "other
/// account" for a principal to move to. The same-profile case is idempotent — re-running the
/// flow re-stamps `linked_at` and succeeds. A different-profile attempt is REFUSED:
/// `AlreadyLinkedToAnotherProfile`, no write. "Start fresh" is an explicit **disconnect**, a
/// separate affordance that does not exist yet — not a side effect of linking again.
///
/// **This is what closes the already-linked half of the URL-theft threat.** The residual attack
/// is: steal the victim's ephemeral link URL, complete the login as yourself, and bind their
/// principal to your profile so their future `@temper` writes land in your KB. That attack
/// *requires* rebind when the victim is already linked. Refusing it here closes that case
/// outright. It is also what makes it safe to issue an already-linked victim a URL at all: the
/// mint hands one out when their grant is missing or dead ([`issue_authorize_url`]), and a thief
/// who completes it as themselves lands here and is refused. Only "victim not yet linked,
/// attacker steals their first-link message" remains.
///
/// The guard is the `WHERE` on the `DO UPDATE`, which makes it **atomic** — no read-then-write
/// TOCTOU. On a different-profile conflict the update matches no row, so the statement returns
/// zero rows rather than raising: refusal is a row count, not an error. Verified against the
/// real database, not inferred.
///
/// `email` stays NULL: Slack supplies no email on the wire, which is exactly why the link is
/// keyed on the opaque principal.
///
/// **Takes `&mut PgConnection`, not a pool.** The identity row and the sealed grant
/// (`slack_grant_vault_service::store_grant`) are ONE fact — a link with no vaulted grant can
/// never mint, and the state nonce that would let the user retry is already burned by the time
/// either write runs. As two independent autocommits, a process death between them produced
/// exactly that unrecoverable state with no page ever rendering. The executor type forces the
/// callback to span both in one transaction.
///
/// **Takes `&HumanPrincipal`.** Linking binds a credential grant to a profile, a person's act: a
/// machine never links a Slack account.
pub async fn link_slack_principal(
    conn: &mut sqlx::PgConnection,
    linker: &crate::auth::HumanPrincipal,
    slack_principal_id: &str,
) -> ApiResult<SlackLinkOutcome> {
    let profile_id = linker.profile().id;
    let row = sqlx::query!(
        r#"
        INSERT INTO kb_profile_auth_links
            (id, profile_id, auth_provider, auth_provider_user_id)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (auth_provider, auth_provider_user_id)
        DO UPDATE SET linked_at = now()
        WHERE kb_profile_auth_links.profile_id = EXCLUDED.profile_id
        RETURNING profile_id
        "#,
        Uuid::now_v7(),
        profile_id,
        SLACK_AUTH_PROVIDER,
        slack_principal_id,
    )
    .fetch_optional(&mut *conn)
    .await?;

    Ok(match row {
        Some(_) => SlackLinkOutcome::Linked,
        None => SlackLinkOutcome::AlreadyLinkedToAnotherProfile,
    })
}

#[cfg(all(test, feature = "test-db"))]
mod tests;
