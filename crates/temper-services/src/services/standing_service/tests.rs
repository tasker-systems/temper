use super::*;
use sqlx::PgPool;

async fn profile(pool: &PgPool, handle: &str) -> ProfileId {
    let id: uuid::Uuid = crate::test_support::profile::insert(pool, handle).await;
    ProfileId::from(id)
}

/// Seed a live refresh-token row the way the AS writes one, for `owner`.
async fn live_refresh_token(pool: &PgPool, owner: Option<ProfileId>, token_hash: &str) {
    sqlx::query(
        "INSERT INTO kb_oauth_refresh_tokens \
           (token_hash, client_id, claims, expires_at, chain_expires_at, profile_id) \
         VALUES ($1, 'cli', '{}'::jsonb, now() + interval '30 days', \
                 now() + interval '90 days', $2)",
    )
    .bind(token_hash)
    .bind(owner.map(|o| *o))
    .execute(pool)
    .await
    .unwrap();
}

async fn is_revoked(pool: &PgPool, token_hash: &str) -> bool {
    sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
        "SELECT revoked_at FROM kb_oauth_refresh_tokens WHERE token_hash = $1",
    )
    .bind(token_hash)
    .fetch_one(pool)
    .await
    .unwrap()
    .is_some()
}

/// Whether the row claims it was retired BY ROTATION — a claim only the AS may make.
async fn claims_rotation(pool: &PgPool, token_hash: &str) -> bool {
    sqlx::query_scalar::<_, Option<chrono::DateTime<chrono::Utc>>>(
        "SELECT rotated_at FROM kb_oauth_refresh_tokens WHERE token_hash = $1",
    )
    .bind(token_hash)
    .fetch_one(pool)
    .await
    .unwrap()
    .is_some()
}

/// Bring a principal to `Approved` through the seam, so the fixture is a real transition
/// history rather than a hand-written row.
async fn approved(pool: &PgPool, handle: &str, admin: ProfileId) -> ProfileId {
    let p = profile(pool, handle).await;
    for (act, authority, actor) in [
        (
            Act::Provision {
                path: Provisioner::Saml,
            },
            ActorAuthority::Credential,
            None,
        ),
        (Act::Approve, ActorAuthority::Admin, Some(admin)),
    ] {
        apply(
            pool,
            ApplyStandingParams {
                subject: p,
                act,
                actor,
                authority,
            },
        )
        .await
        .unwrap();
    }
    p
}

/// Ending a principal's admission ends their live refresh chains, through the single standing
/// writer, so every terminal path inherits it rather than each remembering to do it.
///
/// The second principal is the regression arm, not decoration. De-provisioning one human must
/// not touch another, and a `WHERE` clause that over-matched would still pass an
/// assertion that only looked at the subject.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn revoking_a_principal_ends_their_live_refresh_chains_and_only_theirs(pool: PgPool) {
    let admin = profile(&pool, "chain-admin").await;
    let departing = approved(&pool, "chain-departing", admin).await;
    let remaining = approved(&pool, "chain-remaining", admin).await;

    live_refresh_token(&pool, Some(departing), "hash-departing-a").await;
    live_refresh_token(&pool, Some(departing), "hash-departing-b").await;
    live_refresh_token(&pool, Some(remaining), "hash-remaining").await;
    // A chain minted before an owner could be resolved. Un-endable here BY CONSTRUCTION, and
    // asserted so, because that limit is stated in the migration and must not quietly change.
    live_refresh_token(&pool, None, "hash-ownerless").await;

    apply(
        &pool,
        ApplyStandingParams {
            subject: departing,
            act: Act::Revoke {
                reason: "left the company".into(),
            },
            actor: Some(admin),
            authority: ActorAuthority::Admin,
        },
    )
    .await
    .unwrap();

    assert!(is_revoked(&pool, "hash-departing-a").await);
    assert!(is_revoked(&pool, "hash-departing-b").await);
    assert!(
        !is_revoked(&pool, "hash-remaining").await,
        "revoking one principal must not touch another's chain"
    );
    assert!(
        !is_revoked(&pool, "hash-ownerless").await,
        "an ownerless chain is bounded by its absolute lifetime, not by this hook"
    );

    // This hook writes `revoked_at`, and so does the AS's rotation — which is exactly why the
    // AS also writes `rotated_at`, so that a token presented again after being spent can be
    // told from one an administrator ended (20260826000140). `revoked_at` has five writers and
    // only one is rotation, so that distinction is only worth anything if each of the other
    // four never makes the rotation claim. Asserted here, in the crate that owns this writer,
    // because the AS's own suite cannot see it: a stray `rotated_at` added to the UPDATE above
    // would turn every administrative revoke into a permanent, unfalsifiable report that the
    // user's credential had been stolen.
    assert!(
        !claims_rotation(&pool, "hash-departing-a").await,
        "an administrator's revoke must not present itself as a rotation"
    );
}

/// `Deactivate` is the other terminal, and it is reachable from states `Revoke` is not.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn deactivating_a_principal_also_ends_their_live_refresh_chains(pool: PgPool) {
    let admin = profile(&pool, "deact-admin").await;
    let subject = approved(&pool, "deact-subject", admin).await;
    live_refresh_token(&pool, Some(subject), "hash-deact").await;

    apply(
        &pool,
        ApplyStandingParams {
            subject,
            act: Act::Deactivate,
            actor: Some(admin),
            authority: ActorAuthority::Admin,
        },
    )
    .await
    .unwrap();

    assert!(is_revoked(&pool, "hash-deact").await);
}

/// An approval is not a terminal, so it must leave a live chain alone. A hook that fired on
/// every transition would pass both tests above and log everyone out on approval; this is the
/// arm that says so.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_non_terminal_transition_leaves_the_chain_alone(pool: PgPool) {
    let admin = profile(&pool, "approve-admin").await;
    let subject = profile(&pool, "approve-subject").await;
    apply(
        &pool,
        ApplyStandingParams {
            subject,
            act: Act::Provision {
                path: Provisioner::Saml,
            },
            actor: None,
            authority: ActorAuthority::Credential,
        },
    )
    .await
    .unwrap();
    live_refresh_token(&pool, Some(subject), "hash-approve").await;

    apply(
        &pool,
        ApplyStandingParams {
            subject,
            act: Act::Approve,
            actor: Some(admin),
            authority: ActorAuthority::Admin,
        },
    )
    .await
    .unwrap();

    assert!(!is_revoked(&pool, "hash-approve").await);
}

/// The cross-language pin. `principal_may_refresh` (SQL, read by the Authorization Server) and
/// this hook's terminal set (Rust) are two readings of one decision, in two languages, with
/// nothing between them. This fails if they diverge in EITHER direction.
///
/// The `match` is exhaustive on purpose: a new `Standing` variant does not silently inherit an
/// answer here, it stops the build until someone decides which side it falls on. A new state
/// added to the column's CHECK but not to the enum fails at `parse` instead.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn the_sql_refresh_gate_matches_the_rust_terminal_set(pool: PgPool) {
    fn rust_says_may_refresh(s: Standing) -> bool {
        match s {
            Standing::Denied | Standing::Requested | Standing::Approved => true,
            Standing::Revoked | Standing::Deactivated => false,
        }
    }

    for literal in ["denied", "requested", "approved", "revoked", "deactivated"] {
        let p = profile(&pool, &format!("pin-{literal}")).await;
        // Written directly rather than through the machine: this is a test OF the predicate,
        // and reaching every state through legal transitions would test the machine instead.
        sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1, $2)")
            .bind(*p)
            .bind(literal)
            .execute(&pool)
            .await
            .unwrap();

        let sql_says: Option<bool> = sqlx::query_scalar("SELECT principal_may_refresh($1)")
            .bind(*p)
            .fetch_one(&pool)
            .await
            .unwrap();
        let standing = Standing::parse(literal).unwrap_or_else(|| {
            panic!("the column allows {literal:?} but Standing cannot parse it")
        });

        assert_eq!(
            sql_says,
            Some(rust_says_may_refresh(standing)),
            "principal_may_refresh and the Rust terminal set disagree about {literal:?}"
        );
    }

    // Absence denies, like every other standing predicate.
    let no_standing = profile(&pool, "pin-absent").await;
    let sql_says: Option<bool> = sqlx::query_scalar("SELECT principal_may_refresh($1)")
        .bind(*no_standing)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        sql_says,
        Some(false),
        "a principal with no standing row is not one we mint for"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn an_illegal_transition_is_refused_and_writes_nothing(pool: PgPool) {
    let p = profile(&pool, "illegal").await;
    let admin = profile(&pool, "illegal-admin").await;
    apply(
        &pool,
        ApplyStandingParams {
            subject: p,
            act: Act::Provision {
                path: Provisioner::OauthFirstLogin,
            },
            actor: None,
            authority: ActorAuthority::Credential,
        },
    )
    .await
    .unwrap();

    // Revoke from Denied — you cannot revoke what was never granted (spec §6).
    let err = apply(
        &pool,
        ApplyStandingParams {
            subject: p,
            act: Act::Revoke {
                reason: "no".into(),
            },
            actor: Some(admin),
            authority: ActorAuthority::Admin,
        },
    )
    .await
    .expect_err("must refuse");

    assert!(
        format!("{err}").contains("not legal"),
        "the refusal must carry a reason: {err}"
    );

    let state: String =
        sqlx::query_scalar("SELECT state FROM kb_principal_standing WHERE profile_id=$1")
            .bind(*p)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(state, "denied", "a refused act must write nothing");

    let logs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_principal_standing_events WHERE profile_id=$1 AND act='revoke'",
    )
    .bind(*p)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(logs, 0, "a refused act must not appear in the log");
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn reactivate_restores_the_prior_state_through_the_seam(pool: PgPool) {
    let p = profile(&pool, "react").await;
    let admin = profile(&pool, "react-admin").await;
    for (act, auth) in [
        (
            Act::Provision {
                path: Provisioner::OauthFirstLogin,
            },
            ActorAuthority::Credential,
        ),
        (Act::Approve, ActorAuthority::Admin),
        (Act::Deactivate, ActorAuthority::Admin),
    ] {
        apply(
            &pool,
            ApplyStandingParams {
                subject: p,
                act,
                actor: Some(admin),
                authority: auth,
            },
        )
        .await
        .unwrap();
    }

    let restored = apply(
        &pool,
        ApplyStandingParams {
            subject: p,
            act: Act::Reactivate { prior: None }, // the seam fills this in
            actor: Some(admin),
            authority: ActorAuthority::Admin,
        },
    )
    .await
    .unwrap();

    assert_eq!(
        restored,
        Standing::Approved,
        "Reactivate restores rather than guesses (§5)"
    );
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn admit_denies_a_principal_with_no_standing_row(pool: PgPool) {
    let p = profile(&pool, "nostanding").await;
    assert_eq!(admit(&pool, p).await, Err(Refusal::NoStanding));
}
