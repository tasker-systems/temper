use super::*;
use sqlx::PgPool;
use uuid::Uuid;

// **These tests assert on SURVIVORS, never on a swept count**, and that is a correctness
// property of the suite rather than a style choice. A reaper has no scoping predicate a test
// can narrow — it deletes from the whole table by construction — so `assert_eq!(swept, 1)` is
// an assertion about every row in the database, including rows the test did not create. Under
// `cargo nextest`'s parallelism that made the suite flaky, failing 5-of-9 in the direction of
// "more rows than I seeded". The survivor assertions carry the identical evidence, name the
// rows they mean, and cannot be moved by a row belonging to someone else.

/// A consumed assertion, `age_seconds` old, stamped the way `guardReplay` stamps one: an
/// `expires_at` of `created + REPLAY_TTL_SECONDS`, where that TTL is the AS's literal 600.
async fn seed_replay(pool: &PgPool, id: &str, age_seconds: i64) {
    sqlx::query(
        "INSERT INTO kb_saml_replay (assertion_id, expires_at) \
         VALUES ($1, now() - make_interval(secs => $2::double precision) + interval '600 seconds')",
    )
    .bind(id)
    .bind(age_seconds as f64)
    .execute(pool)
    .await
    .expect("seed replay row");
}

async fn replay_ids(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT assertion_id FROM kb_saml_replay ORDER BY assertion_id")
        .fetch_all(pool)
        .await
        .expect("read back replay rows")
}

/// **The retention floor, asserted in the direction that matters.**
///
/// FAILS IF: the sweep reaps on `expires_at` alone. `recent` is 20 minutes old, so its stamped
/// `expires_at` (created + 600s) passed ten minutes ago — an `expires_at < now()` predicate
/// deletes it. But the IdP here issues a one-hour assertion, so it is still replayable, and
/// deleting it re-opens the exact hole `kb_saml_replay` exists to close.
#[sqlx::test(migrations = "../../migrations")]
async fn a_replay_row_inside_the_assertion_window_is_not_deleted(pool: PgPool) {
    let window = 3600.0;

    // Past its stamped expires_at, inside the real assertion window.
    seed_replay(&pool, "inside-window", 1200).await;
    // Past both.
    seed_replay(&pool, "outside-window", 7200).await;
    // Not even past its stamp.
    seed_replay(&pool, "unexpired", 60).await;

    reap_saml_replay(&pool, window).await.expect("sweep");

    assert_eq!(
        replay_ids(&pool).await,
        vec!["inside-window".to_string(), "unexpired".to_string()],
        "an assertion still inside the IdP's validity window must still be remembered",
    );
}

/// FAILS IF: the floor is read off the row's stamp rather than derived. Widening the configured
/// window must move the floor for rows ALREADY on disk — that is what makes it survive a
/// lagging writer still using the 600s literal.
#[sqlx::test(migrations = "../../migrations")]
async fn widening_the_configured_window_spares_rows_already_stamped(pool: PgPool) {
    seed_replay(&pool, "two-hours-old", 7200).await;

    reap_saml_replay(&pool, 10_800.0).await.expect("sweep");
    assert!(
        replay_ids(&pool)
            .await
            .contains(&"two-hours-old".to_string()),
        "a 3h window must spare a 2h-old row whatever its stamp says",
    );

    reap_saml_replay(&pool, 3600.0).await.expect("sweep");
    assert!(
        !replay_ids(&pool)
            .await
            .contains(&"two-hours-old".to_string()),
        "a 1h window reaps the same row",
    );
}

/// Seeds a refresh token. `expired` and `chain_dead` are ages in days past the respective
/// deadlines; negative means still live.
async fn seed_token(
    pool: &PgPool,
    hash: &str,
    expired_days: i64,
    chain_dead_days: i64,
    rotated: bool,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_oauth_refresh_tokens \
             (token_hash, client_id, claims, expires_at, chain_expires_at, rotated_at, revoked_at) \
         VALUES ($1, 'test-client', '{}'::jsonb, \
                 now() - make_interval(days => $2::int), \
                 now() - make_interval(days => $3::int), \
                 CASE WHEN $4 THEN now() ELSE NULL END, \
                 CASE WHEN $4 THEN now() ELSE NULL END) \
         RETURNING id",
    )
    .bind(hash)
    .bind(expired_days as i32)
    .bind(chain_dead_days as i32)
    .bind(rotated)
    .fetch_one(pool)
    .await
    .expect("seed refresh token")
}

async fn token_hashes(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar("SELECT token_hash FROM kb_oauth_refresh_tokens ORDER BY token_hash")
        .fetch_all(pool)
        .await
        .expect("read back tokens")
}

/// **The floor the draft did not know it needed.**
///
/// FAILS IF: the sweep reaps on `expires_at` alone. `live-chain` expired 40 days ago but its
/// chain runs for another 10 — `findRotatedToken` has no expiry filter, so presenting it today
/// is still detected as a replay and still ends the chain. Deleting it makes that a silent
/// "unknown token".
#[sqlx::test(migrations = "../../migrations")]
async fn a_spent_token_on_a_live_chain_is_not_deleted(pool: PgPool) {
    // Long expired, chain long dead — reapable.
    seed_token(&pool, "reapable", 400, 400, true).await;
    // Long expired, chain still live — detection still depends on it.
    seed_token(&pool, "live-chain", 40, -10, true).await;
    // Chain long dead, token itself only just expired.
    seed_token(&pool, "fresh-token", 1, 400, false).await;

    reap_refresh_tokens(&pool).await.expect("sweep");

    assert_eq!(
        token_hashes(&pool).await,
        vec!["fresh-token".to_string(), "live-chain".to_string()],
        "a token whose chain can still be replayed into must survive the sweep, and only the \
         one past BOTH deadlines by the forensic margin may go",
    );
}

/// **FAILS IF: the sweep's predicate stops holding evidence-bearing tokens out.**
///
/// This is the NORMAL path — the `NOT EXISTS` arm keeping the sweep away from a token that
/// carries a recorded replay. It is not the guarantee; see
/// `recorded_evidence_cannot_be_deleted_even_bypassing_the_predicate` for that.
#[sqlx::test(migrations = "../../migrations")]
async fn a_token_with_recorded_evidence_is_never_swept(pool: PgPool) {
    let token_id = seed_token(&pool, "replayed", 400, 400, true).await;

    sqlx::query(
        "INSERT INTO kb_oauth_refresh_replays \
             (token_id, chain_id, client_id, rotated_at, first_age_seconds) \
         VALUES ($1, gen_random_uuid(), 'test-client', now() - interval '1 hour', 3600)",
    )
    .bind(token_id)
    .execute(&pool)
    .await
    .expect("seed replay evidence");

    reap_refresh_tokens(&pool).await.expect("sweep");

    assert!(
        token_hashes(&pool).await.contains(&"replayed".to_string()),
        "an evidence-bearing token is out of scope however old it is",
    );
    let evidence: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_oauth_refresh_replays WHERE token_id = $1")
            .bind(token_id)
            .fetch_one(&pool)
            .await
            .expect("count evidence");
    assert_eq!(
        evidence, 1,
        "the replay record must survive the retention sweep"
    );
}

/// **The guarantee the predicate could not give.**
///
/// FAILS IF: `kb_oauth_refresh_replays.token_id` is `ON DELETE CASCADE`. Under that FK,
/// deleting a token silently destroys any replay recorded against it — the row an operator
/// reads through `vw_oauth_refresh_replays` when asking whether a credential was copied.
///
/// The sweep's own `NOT EXISTS` arm is *not* what this asserts, and deliberately so. That
/// filter races: under READ COMMITTED its subquery is evaluated against the statement's
/// snapshot, so a replay recorded while the sweep waits on the row's FK `KEY SHARE` lock is
/// cascaded away by a delete that had already decided to proceed. Reproduced before this
/// constraint existed. So this deletes DIRECTLY, bypassing the predicate exactly the way the
/// race does, and asserts the database itself refuses.
#[sqlx::test(migrations = "../../migrations")]
async fn recorded_evidence_cannot_be_deleted_even_bypassing_the_predicate(pool: PgPool) {
    let token_id = seed_token(&pool, "replayed", 400, 400, true).await;
    sqlx::query(
        "INSERT INTO kb_oauth_refresh_replays \
             (token_id, chain_id, client_id, rotated_at, first_age_seconds) \
         VALUES ($1, gen_random_uuid(), 'test-client', now() - interval '1 hour', 3600)",
    )
    .bind(token_id)
    .execute(&pool)
    .await
    .expect("seed replay evidence");

    let err = sqlx::query("DELETE FROM kb_oauth_refresh_tokens WHERE id = $1")
        .bind(token_id)
        .execute(&pool)
        .await
        .expect_err("the database must refuse to delete a token that carries evidence");

    // 23001 `restrict_violation`, NOT 23503 `foreign_key_violation` — and the distinction is
    // the assertion, not a detail. RESTRICT raises 23001 and is checked immediately; NO ACTION
    // raises 23503 and is deferrable, so a future migration relaxing this constraint to NO
    // ACTION would change the code and fail here. That is the intent: evidence must be
    // unloseable at the instant of the delete, not at some later commit.
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("23001"),
        "must be RESTRICT refusing immediately, not some other failure; got {err:?}",
    );
}

/// FAILS IF: the `RESTRICT` that protects evidence also blocks deleting a **profile**.
///
/// `kb_profiles` cascades into `kb_oauth_refresh_tokens`, so a profile delete deletes its
/// tokens — and meets that constraint if one carries evidence. It does not, because
/// `recordRefreshReplay` takes the replay's `profile_id` from the token row, so an owned
/// token's evidence is owned too and cascades beside it. This pins that: it is an invariant
/// held by one line of TypeScript and by no constraint, and an erasure path is the obvious
/// thing to build next.
#[sqlx::test(migrations = "../../migrations")]
async fn deleting_a_profile_still_works_when_its_token_carries_evidence(pool: PgPool) {
    let profile_id = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, 'Probe')")
        .bind(profile_id)
        .bind(format!("probe-{profile_id}"))
        .execute(&pool)
        .await
        .expect("seed profile");

    let token_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_oauth_refresh_tokens \
             (token_hash, client_id, claims, expires_at, chain_expires_at, profile_id) \
         VALUES ('owned-h', 'c', '{}'::jsonb, now(), now(), $1) RETURNING id",
    )
    .bind(profile_id)
    .fetch_one(&pool)
    .await
    .expect("seed owned token");

    // Attributed exactly the way `recordRefreshReplay` attributes it: the replay inherits the
    // token's owner. Writing NULL here instead would construct a state the AS cannot produce.
    sqlx::query(
        "INSERT INTO kb_oauth_refresh_replays \
             (token_id, chain_id, profile_id, client_id, rotated_at, first_age_seconds) \
         VALUES ($1, gen_random_uuid(), $2, 'c', now(), 1)",
    )
    .bind(token_id)
    .bind(profile_id)
    .execute(&pool)
    .await
    .expect("seed attributed evidence");

    sqlx::query("DELETE FROM kb_profiles WHERE id = $1")
        .bind(profile_id)
        .execute(&pool)
        .await
        .expect("deleting a profile must not be blocked by its own token's evidence");
}

/// FAILS IF: `status` leaks into the flow predicate. An abandoned `pending_saml` flow is as
/// dead as a consumed one once expired, and keying on status would keep the abandoned ones —
/// the very rows an abandoned-login backlog is made of — forever.
#[sqlx::test(migrations = "../../migrations")]
async fn expired_flows_are_swept_whatever_their_status(pool: PgPool) {
    for (relay, status, age_days) in [
        ("old-pending", "pending_saml", 5),
        ("old-consumed", "consumed", 5),
        ("old-issued", "code_issued", 5),
        ("recent-consumed", "consumed", 0),
    ] {
        sqlx::query(
            "INSERT INTO kb_oauth_flow \
                 (relay_state, status, client_id, redirect_uri, code_challenge, \
                  code_challenge_method, oauth_state, audience, expires_at) \
             VALUES ($1, $2, 'c', 'https://example.test/cb', 'ch', 'S256', 'st', 'aud', \
                     now() - make_interval(days => $3::int))",
        )
        .bind(relay)
        .bind(status)
        .bind(age_days)
        .execute(&pool)
        .await
        .expect("seed flow");
    }

    reap_oauth_flows(&pool).await.expect("sweep");

    let survivors: Vec<String> =
        sqlx::query_scalar("SELECT relay_state FROM kb_oauth_flow ORDER BY relay_state")
            .fetch_all(&pool)
            .await
            .expect("read back flows");
    assert_eq!(
        survivors,
        vec!["recent-consumed".to_string()],
        "every flow a day past expiry goes regardless of status; the recent one stays",
    );
}

/// FAILS IF: the sweep is not idempotent. A second pass over the same state must delete nothing
/// and must not error — the cron runs it on a schedule forever.
#[sqlx::test(migrations = "../../migrations")]
async fn a_second_sweep_over_the_same_state_deletes_nothing(pool: PgPool) {
    seed_replay(&pool, "ancient", 100_000).await;
    seed_token(&pool, "ancient-token", 400, 400, true).await;

    reap_expired_as_rows(&pool).await.expect("first sweep");
    assert!(
        replay_ids(&pool).await.is_empty() && token_hashes(&pool).await.is_empty(),
        "the first sweep clears what is past every floor",
    );

    let second = reap_expired_as_rows(&pool).await.expect("second sweep");
    assert_eq!(second, AsReapSummary::default(), "a re-run is a no-op");
}
