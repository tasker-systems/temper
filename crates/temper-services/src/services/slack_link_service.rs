//! The Slack account-link flow's persistence: intent lifecycle + the directory row.
//!
//! All SQL for T2 lives here; the handlers dispatch and never touch the database.

use std::time::Duration;

use sqlx::PgPool;
use uuid::Uuid;

use crate::error::ApiResult;

/// The `auth_provider` value for every Slack link row. One constant so the write and
/// any future read can never disagree on the string.
pub const SLACK_AUTH_PROVIDER: &str = "slack";

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

/// Write the directory row `slack:<team>:<user> -> profile`.
///
/// Idempotent on re-link via `UNIQUE(auth_provider, auth_provider_user_id)`. A conflict that
/// carries a DIFFERENT profile_id is a rebind, and it is allowed by design: binding requires
/// authenticating AS the target profile, so a principal can only ever bind to the
/// authenticator's own profile. See spec D4.
///
/// `email` stays NULL: Slack supplies no email on the wire, which is exactly why the link is
/// keyed on the opaque principal.
pub async fn upsert_slack_link(
    pool: &PgPool,
    profile_id: Uuid,
    slack_principal_id: &str,
) -> ApiResult<()> {
    sqlx::query!(
        r#"
        INSERT INTO kb_profile_auth_links
            (id, profile_id, auth_provider, auth_provider_user_id)
        VALUES ($1, $2, $3, $4)
        ON CONFLICT (auth_provider, auth_provider_user_id)
        DO UPDATE SET profile_id = EXCLUDED.profile_id, linked_at = now()
        "#,
        Uuid::now_v7(),
        profile_id,
        SLACK_AUTH_PROVIDER,
        slack_principal_id,
    )
    .execute(pool)
    .await?;

    Ok(())
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;

    const PRINCIPAL: &str = "slack:T0BHAHEN79C:U0BH6A3L6JF";

    #[sqlx::test(migrations = "../../migrations")]
    async fn consume_returns_the_verifier_and_principal_once(pool: PgPool) {
        let nonce = create_intent(&pool, PRINCIPAL, "verifier-abc", Duration::from_secs(600))
            .await
            .unwrap();

        let first = consume_intent(&pool, &nonce).await.unwrap().unwrap();
        assert_eq!(first.code_verifier, "verifier-abc");
        assert_eq!(first.slack_principal_id, PRINCIPAL);
    }

    /// The single-use invariant. A replayed state must not resolve.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_second_consume_of_the_same_nonce_yields_none(pool: PgPool) {
        let nonce = create_intent(&pool, PRINCIPAL, "verifier-abc", Duration::from_secs(600))
            .await
            .unwrap();

        assert!(consume_intent(&pool, &nonce).await.unwrap().is_some());
        assert!(consume_intent(&pool, &nonce).await.unwrap().is_none());
    }

    /// TTL. An expired intent is indistinguishable from an unknown one.
    #[sqlx::test(migrations = "../../migrations")]
    async fn an_expired_intent_yields_none(pool: PgPool) {
        let nonce = create_intent(&pool, PRINCIPAL, "v", Duration::from_secs(0))
            .await
            .unwrap();
        // ttl=0 => expires_at == now(); the `expires_at > now()` predicate excludes it.
        assert!(consume_intent(&pool, &nonce).await.unwrap().is_none());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn an_unknown_nonce_yields_none(pool: PgPool) {
        assert!(consume_intent(&pool, "never-issued")
            .await
            .unwrap()
            .is_none());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn nonces_are_unique_across_intents(pool: PgPool) {
        let a = create_intent(&pool, PRINCIPAL, "v", Duration::from_secs(600))
            .await
            .unwrap();
        let b = create_intent(&pool, PRINCIPAL, "v", Duration::from_secs(600))
            .await
            .unwrap();
        assert_ne!(a, b);
    }

    /// Insert a bare profile and return its id. Deliberately minimal: this suite tests the
    /// lookup's join, not provisioning, and `create_new_profile_and_link` would drag the whole
    /// auth seam in. The e2e tier provisions through the real login path.
    async fn insert_profile(pool: &PgPool, handle: &str) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query!(
            "INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $3)",
            id,
            handle,
            handle,
        )
        .execute(pool)
        .await
        .unwrap();
        id
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn lookup_yields_the_handle_when_a_link_row_exists(pool: PgPool) {
        let profile_id = insert_profile(&pool, "j-cole-taylor").await;
        upsert_slack_link(&pool, profile_id, PRINCIPAL)
            .await
            .unwrap();

        assert_eq!(
            lookup_linked_handle(&pool, PRINCIPAL).await.unwrap(),
            Some("j-cole-taylor".to_string()),
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn lookup_yields_none_for_an_unlinked_principal(pool: PgPool) {
        assert!(lookup_linked_handle(&pool, PRINCIPAL)
            .await
            .unwrap()
            .is_none());
    }

    /// The principal is the key, WHOLE. A different Slack user must not read another's link
    /// just because a prefix matches.
    #[sqlx::test(migrations = "../../migrations")]
    async fn lookup_does_not_match_a_different_principal(pool: PgPool) {
        let profile_id = insert_profile(&pool, "someone-else").await;
        upsert_slack_link(&pool, profile_id, PRINCIPAL)
            .await
            .unwrap();

        assert!(lookup_linked_handle(&pool, "slack:T0BHAHEN79C:UOTHER")
            .await
            .unwrap()
            .is_none());
    }

    /// D4's rebind, at the layer that implements it: binding the principal to a DIFFERENT
    /// profile moves the link rather than duplicating it. Tested here because the mention flow
    /// no longer offers a linked user a fresh challenge, so the e2e tier cannot reach this.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_rebind_moves_the_link_rather_than_duplicating_it(pool: PgPool) {
        let first = insert_profile(&pool, "first-owner").await;
        let second = insert_profile(&pool, "second-owner").await;

        upsert_slack_link(&pool, first, PRINCIPAL).await.unwrap();
        upsert_slack_link(&pool, second, PRINCIPAL).await.unwrap();

        assert_eq!(
            lookup_linked_handle(&pool, PRINCIPAL).await.unwrap(),
            Some("second-owner".to_string()),
            "the rebind must win — UNIQUE(auth_provider, auth_provider_user_id) keeps one row",
        );
    }

    /// A deactivated profile is still LINKED. Reporting `None` here would loop the user into a
    /// link flow the callback then refuses — see the comment on `lookup_linked_handle`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn lookup_still_reports_a_deactivated_profile_as_linked(pool: PgPool) {
        let profile_id = insert_profile(&pool, "gone-away").await;
        upsert_slack_link(&pool, profile_id, PRINCIPAL)
            .await
            .unwrap();
        sqlx::query!(
            "UPDATE kb_profiles SET is_active = false WHERE id = $1",
            profile_id
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            lookup_linked_handle(&pool, PRINCIPAL).await.unwrap(),
            Some("gone-away".to_string()),
        );
    }
}
