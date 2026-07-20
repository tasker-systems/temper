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
/// outright, and D9 means an already-linked victim is never issued a URL to steal in the first
/// place. Only "victim not yet linked, attacker steals their first-link message" remains.
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
pub async fn link_slack_principal(
    conn: &mut sqlx::PgConnection,
    profile_id: Uuid,
    slack_principal_id: &str,
) -> ApiResult<SlackLinkOutcome> {
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
mod tests {
    use super::*;

    const PRINCIPAL: &str = "slack:T0BHAHEN79C:U0BH6A3L6JF";

    /// Bind a principal on a one-off pooled connection.
    ///
    /// `link_slack_principal` takes `&mut PgConnection` so the callback can run it in the same
    /// transaction as the vault write (see its docs); these tests exercise the upsert's semantics,
    /// not that atomicity, so they hand it a connection straight off the pool.
    async fn link(pool: &PgPool, profile_id: Uuid, principal: &str) -> SlackLinkOutcome {
        let mut conn = pool.acquire().await.expect("acquire");
        link_slack_principal(&mut conn, profile_id, principal)
            .await
            .expect("link")
    }

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
        link(&pool, profile_id, PRINCIPAL).await;

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
        link(&pool, profile_id, PRINCIPAL).await;

        assert!(lookup_linked_handle(&pool, "slack:T0BHAHEN79C:UOTHER")
            .await
            .unwrap()
            .is_none());
    }

    /// **There is no rebind.** Binding a principal that is already bound to a DIFFERENT
    /// profile is refused, and refused WITHOUT writing.
    ///
    /// The return value is only half the assertion. A regression that reports the refusal and
    /// still lets the write land — the old `DO UPDATE SET profile_id` with a bolted-on check,
    /// say — would pass a return-value-only test while silently doing the exact thing this
    /// refusal exists to prevent. So the row itself is the load-bearing assertion: it must
    /// still name the ORIGINAL profile.
    #[sqlx::test(migrations = "../../migrations")]
    async fn binding_to_a_different_profile_is_refused_and_writes_nothing(pool: PgPool) {
        let first = insert_profile(&pool, "first-owner").await;
        let second = insert_profile(&pool, "second-owner").await;

        assert_eq!(
            link(&pool, first, PRINCIPAL).await,
            SlackLinkOutcome::Linked,
        );
        assert_eq!(
            link(&pool, second, PRINCIPAL).await,
            SlackLinkOutcome::AlreadyLinkedToAnotherProfile,
            "a principal binds once — moving it to another profile must be refused",
        );

        assert_eq!(
            lookup_linked_handle(&pool, PRINCIPAL).await.unwrap(),
            Some("first-owner".to_string()),
            "the refusal must leave the row pointing at the ORIGINAL profile — asserting only \
             the return value would pass even if the write landed",
        );
    }

    /// The same-profile case is idempotent: it succeeds, and it keeps exactly one row.
    ///
    /// This is the half of the conditional upsert the refusal above must not break. The row
    /// count is asserted directly because `lookup_linked_handle` uses `fetch_optional` and
    /// would happily report the handle off the first of two duplicate rows.
    #[sqlx::test(migrations = "../../migrations")]
    async fn binding_the_same_profile_twice_is_idempotent(pool: PgPool) {
        let profile_id = insert_profile(&pool, "same-owner").await;

        for attempt in 1..=2 {
            assert_eq!(
                link(&pool, profile_id, PRINCIPAL).await,
                SlackLinkOutcome::Linked,
                "attempt {attempt} for the same profile must succeed",
            );
        }

        let rows: i64 = sqlx::query_scalar!(
            "SELECT count(*) FROM kb_profile_auth_links
              WHERE auth_provider = $1 AND auth_provider_user_id = $2",
            SLACK_AUTH_PROVIDER,
            PRINCIPAL,
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .unwrap_or_default();
        assert_eq!(rows, 1, "re-linking the same profile must not duplicate");
    }

    /// **One profile, two workspaces.** The reverse lookup must return BOTH principals.
    ///
    /// There is no `UNIQUE(profile_id, auth_provider)`, so this is a legitimate state, not a
    /// corruption: the already-linked refusal is keyed on the principal, so a human's second
    /// Slack workspace links normally. A reverse lookup that answered with one row would let a
    /// self-serve disconnect sever an arbitrary binding and report success while the other
    /// grant stayed live — which is exactly the bug this function replaced.
    ///
    /// The assertion is on the whole vector, not on `len() >= 1`: a `fetch_optional`-shaped
    /// regression returns a one-element vector, and only comparing the full ordered set
    /// falsifies it.
    #[sqlx::test(migrations = "../../migrations")]
    async fn lookup_by_profile_returns_every_slack_principal(pool: PgPool) {
        let profile_id = insert_profile(&pool, "two-workspaces").await;
        let first = "slack:T0BHAHEN79C:U0BH6A3L6JF";
        let second = "slack:T99OTHERWS:U0BH6A3L6JF";

        link(&pool, profile_id, first).await;
        link(&pool, profile_id, second).await;

        let mut got = lookup_slack_principals_for_profile(&pool, profile_id)
            .await
            .unwrap();
        got.sort();
        assert_eq!(
            got,
            vec![first.to_string(), second.to_string()],
            "both workspaces' principals must come back — one row is the severed-the-wrong-one bug",
        );
    }

    /// The reverse lookup is scoped to the profile AND to the slack provider — a link under a
    /// different provider (the ordinary SSO row every profile has) must not be reported as a
    /// Slack principal, or disconnect would try to unbind the user's login.
    #[sqlx::test(migrations = "../../migrations")]
    async fn lookup_by_profile_ignores_other_providers_and_other_profiles(pool: PgPool) {
        let mine = insert_profile(&pool, "mine").await;
        let theirs = insert_profile(&pool, "theirs").await;

        link(&pool, mine, PRINCIPAL).await;
        link(&pool, theirs, "slack:T0BHAHEN79C:UOTHER").await;
        sqlx::query!(
            "INSERT INTO kb_profile_auth_links (id, profile_id, auth_provider, auth_provider_user_id)
             VALUES ($1, $2, 'auth0', $3)",
            Uuid::now_v7(),
            mine,
            "auth0|abc123",
        )
        .execute(&pool)
        .await
        .unwrap();

        assert_eq!(
            lookup_slack_principals_for_profile(&pool, mine)
                .await
                .unwrap(),
            vec![PRINCIPAL.to_string()],
        );
    }

    /// **FINDING C, the identity half.** `link_slack_principal` writes on the CALLER's connection,
    /// so the directory row shares that transaction's fate.
    ///
    /// **Why this bites:** it rolls the transaction back and asserts the row is GONE. Against the
    /// old `&PgPool` signature the upsert was its own autocommit and survived — which is precisely
    /// what let the callback commit a link and then lose the grant, leaving a user permanently
    /// "connected" and permanently unable to mint, with the state nonce already burned.
    #[sqlx::test(migrations = "../../migrations")]
    async fn link_is_rolled_back_with_its_caller_transaction(pool: PgPool) {
        let profile_id = insert_profile(&pool, "rolled-back").await;

        let mut tx = pool.begin().await.expect("begin");
        assert_eq!(
            link_slack_principal(&mut tx, profile_id, PRINCIPAL)
                .await
                .expect("link"),
            SlackLinkOutcome::Linked,
        );
        tx.rollback().await.expect("rollback");

        assert!(
            lookup_linked_handle(&pool, PRINCIPAL)
                .await
                .unwrap()
                .is_none(),
            "the directory row must share the caller's transaction — surviving a rollback is the \
             autocommit shape that made the callback's dual write unrecoverable",
        );
    }

    /// A deactivated profile is still LINKED. Reporting `None` here would loop the user into a
    /// link flow the callback then refuses — see the comment on `lookup_linked_handle`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn lookup_still_reports_a_deactivated_profile_as_linked(pool: PgPool) {
        let profile_id = insert_profile(&pool, "gone-away").await;
        link(&pool, profile_id, PRINCIPAL).await;
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
