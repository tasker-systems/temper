use super::*;

const PRINCIPAL: &str = "slack:T0BHAHEN79C:U0BH6A3L6JF";

/// Bind a principal on a one-off pooled connection.
///
/// `link_slack_principal` takes `&mut PgConnection` so the callback can run it in the same
/// transaction as the vault write (see its docs); these tests exercise the upsert's semantics,
/// not that atomicity, so they hand it a connection straight off the pool.
async fn link(pool: &PgPool, profile_id: Uuid, principal: &str) -> SlackLinkOutcome {
    let mut conn = pool.acquire().await.expect("acquire");
    link_slack_principal(
        &mut conn,
        &crate::test_support::human_principal_for(pool, profile_id).await,
        principal,
    )
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
    crate::test_support::profile::insert_with_id(pool, id, handle).await;
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
        link_slack_principal(
            &mut tx,
            &crate::test_support::human_principal_for(&pool, profile_id).await,
            PRINCIPAL
        )
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
    // Deactivate on the authoritative axis (Phase 2 repointed deactivation off the dropped
    // `kb_profiles.is_active` onto `kb_principal_standing`). `lookup_linked_handle` deliberately
    // does not filter on standing, so a deactivated profile is still reported as linked.
    sqlx::query!(
        "INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1, 'deactivated')
             ON CONFLICT (profile_id) DO UPDATE SET state = 'deactivated'",
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
