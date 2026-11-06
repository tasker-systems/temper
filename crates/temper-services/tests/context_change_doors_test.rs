#![cfg(feature = "test-db")]
//! The change feed's door contract (`context_change_service`, 20261117000000): the refusal face, the
//! opaque position, and that a caller who ignores the feed is not burdened by it.
//!
//! The substrate's own witnesses (order, horizon, pointers, departures, admin-act silence) are in
//! `crates/temper-substrate/tests/context_change_feed.rs`. These are the door's:
//!
//! - **An unreadable context is refused as a context read is**, before anything else in the
//!   request is looked at, so a bad position, bound or kind on a context the reader cannot read
//!   gets the same answer as a good one.
//! - **A position is opaque and bound to its context.** It shows neither number, resumes where it
//!   left off, and is refused on another context, when damaged, and when ahead of the ledger. A
//!   refused position is never read as the beginning.
//! - **Bounds and kinds are refused, never clamped or ignored.**
//! - **Reading records nothing**, and the context cursor answers as it did.
//! - **Only an admin act's own transaction is withheld.** An authored change that shares an act's
//!   correlation id and emitter from another transaction is served.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::{ContextId, ProfileId};
use temper_core::types::query_params::ContextChangesQuery;
use temper_services::error::ApiError;
use temper_services::services::context_change_service::{
    changes_after, position_for_test, POSITION_AHEAD_REFUSAL, POSITION_REFUSAL,
};
use temper_services::services::{context_service, event_service};

struct Reader {
    profile: Uuid,
    emitter: Uuid,
}

async fn reader(pool: &PgPool, local: &str) -> Reader {
    let profile = Uuid::now_v7();
    let handle = format!("{local}-{}", &profile.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1,$2,$2,$3)")
        .bind(profile)
        .bind(&handle)
        .bind(format!("{handle}@x.test"))
        .execute(pool)
        .await
        .expect("seed profile");
    let emitter: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,$2,'{}'::jsonb) RETURNING id",
    )
    .bind(profile)
    .bind(format!("{handle}@web"))
    .fetch_one(pool)
    .await
    .expect("seed emitter");
    Reader { profile, emitter }
}

async fn own_context(pool: &PgPool, owner: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles',$1,$2,$2) RETURNING id",
    )
    .bind(owner)
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("seed context")
}

/// One committed domain change in `context`, as a fixture: a `property_set` naming a resource.
async fn change(pool: &PgPool, context: Uuid, emitter: Uuid) -> Uuid {
    let subject = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, \
                                producing_anchor_id, correlation_id, payload) \
         SELECT id, $1, 'kb_contexts', $2, gen_random_uuid(), jsonb_build_object('resource_id', $3::uuid) \
           FROM kb_event_types WHERE name = 'property_set'",
    )
    .bind(emitter)
    .bind(context)
    .bind(subject)
    .execute(pool)
    .await
    .expect("seed a change");
    subject
}

/// Wait until every transaction id assigned so far is below the cluster-wide horizon. Other test
/// databases in the same cluster hold it while their transactions run.
async fn settle(pool: &PgPool) {
    let target: String = sqlx::query_scalar("SELECT pg_snapshot_xmax(pg_current_snapshot())::text")
        .fetch_one(pool)
        .await
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        let settled: bool =
            sqlx::query_scalar("SELECT pg_snapshot_xmin(pg_current_snapshot()) >= $1::text::xid8")
                .bind(&target)
                .fetch_one(pool)
                .await
                .unwrap();
        if settled {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the horizon did not pass xid {target} within 120 s"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

fn q(after: Option<&str>, limit: Option<i32>, kinds: Option<&str>) -> ContextChangesQuery {
    ContextChangesQuery {
        after: after.map(str::to_owned),
        limit,
        kinds: kinds.map(str::to_owned),
    }
}

async fn read(
    pool: &PgPool,
    who: Uuid,
    context: Uuid,
    query: ContextChangesQuery,
) -> Result<temper_core::types::api::ContextChangesResponse, ApiError> {
    changes_after(pool, ProfileId::from(who), ContextId::from(context), &query).await
}

fn bad_request(err: ApiError) -> String {
    match err {
        ApiError::BadRequest(m) => m,
        other => panic!("expected a bad request, got {other:?}"),
    }
}

/// FAILS IF: the door looks at the position, bound or kinds before the gate (bite probe: move the
/// `context_readable_by_profile` check below `parse_kinds`, and the unknown kind answers 400 for a
/// context the reader cannot read), or refuses with anything other than a context read's refusal.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_unreadable_context_is_refused_as_a_context_read_is_whatever_else_is_sent(pool: PgPool) {
    let owner = reader(&pool, "owner").await;
    let stranger = reader(&pool, "stranger").await;
    let private = own_context(&pool, owner.profile, "private").await;
    change(&pool, private, owner.emitter).await;

    let context_read = context_service::get_visible(
        &pool,
        ProfileId::from(stranger.profile),
        ContextId::from(private),
    )
    .await
    .expect_err("the stranger cannot read the context");
    let ApiError::NotFound(expected) = context_read else {
        panic!("a context read refuses with 404, got {context_read:?}");
    };

    let other = own_context(&pool, stranger.profile, "mine").await;
    let foreign = read(&pool, stranger.profile, other, q(None, None, None))
        .await
        .expect("the stranger reads their own context")
        .through;
    let nonexistent = Uuid::now_v7();
    for (context, query) in [
        (private, q(None, None, None)),
        (private, q(Some("not a position"), None, None)),
        (private, q(Some(&foreign), None, None)),
        (private, q(None, Some(0), None)),
        (private, q(None, None, Some("no_such_kind"))),
        (nonexistent, q(None, None, None)),
        (
            nonexistent,
            q(Some("not a position"), Some(5000), Some("no_such_kind")),
        ),
    ] {
        match read(&pool, stranger.profile, context, query).await {
            Err(ApiError::NotFound(m)) => assert_eq!(m, expected),
            other => panic!("expected the context read's refusal, got {other:?}"),
        }
    }
}

/// FAILS IF: a position exposes its numbers, a resumed read repeats or loses a change, or the first
/// read serves a tail of history (bite probe: pass `Some(0)` instead of `None` for a read with no
/// position, and the first answer carries the earlier change).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_position_resumes_where_it_left_off_and_shows_nothing_of_the_ledger(pool: PgPool) {
    let owner = reader(&pool, "owner").await;
    let ctx = own_context(&pool, owner.profile, "notes").await;
    change(&pool, ctx, owner.emitter).await;
    settle(&pool).await;

    let first = read(&pool, owner.profile, ctx, q(None, None, None))
        .await
        .unwrap();
    assert!(
        first.entries.is_empty(),
        "a read with no position is not a tail of history"
    );
    assert!(!first.has_more);
    assert!(first.through.starts_with("fp1_"), "{}", first.through);

    let (xact, seq): (String, i64) = sqlx::query_as(
        "SELECT xact_id::text, seq FROM kb_events WHERE producing_anchor_id = $1 ORDER BY seq DESC LIMIT 1",
    )
    .bind(ctx)
    .fetch_one(&pool)
    .await
    .unwrap();
    // Neither number in decimal, nor big-endian anywhere in the decoded bytes. Positions are read
    // at the present horizon, so `xact` here is close to, not equal to, the one sealed: the
    // nearby values are checked too.
    use base64::Engine as _;
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(first.through.trim_start_matches("fp1_"))
        .unwrap();
    let xact: u64 = xact.parse().unwrap();
    assert!(!first.through.contains(&xact.to_string()));
    for n in (xact.saturating_sub(64)..xact + 64).chain(0..(seq as u64 + 64)) {
        let be = n.to_be_bytes();
        assert!(
            !raw.windows(8).any(|w| w == be),
            "a position shows a ledger number ({n})"
        );
    }

    let a = change(&pool, ctx, owner.emitter).await;
    let b = change(&pool, ctx, owner.emitter).await;
    settle(&pool).await;
    let page = read(
        &pool,
        owner.profile,
        ctx,
        q(Some(&first.through), Some(1), None),
    )
    .await
    .unwrap();
    assert_eq!(page.entries.len(), 1);
    assert_eq!(page.entries[0].subject.id, a);
    assert_eq!(page.entries[0].kind, "property_set");
    assert!(page.has_more);
    let rest = read(
        &pool,
        owner.profile,
        ctx,
        q(Some(&page.through), None, None),
    )
    .await
    .unwrap();
    assert_eq!(
        rest.entries
            .iter()
            .map(|e| e.subject.id)
            .collect::<Vec<_>>(),
        vec![b]
    );
    assert!(!rest.has_more);
    let after = read(
        &pool,
        owner.profile,
        ctx,
        q(Some(&rest.through), None, None),
    )
    .await
    .unwrap();
    assert!(
        after.entries.is_empty(),
        "an empty answer still says how far it looked"
    );

    let wire = serde_json::to_value(&page).unwrap();
    let entry = wire["entries"][0].as_object().unwrap();
    let mut keys: Vec<&str> = entry.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["act", "actor", "kind", "occurred_at", "subject"],
        "no per-entry position"
    );
}

/// FAILS IF: a position is accepted on a context it was not issued for, or a damaged one is read as
/// the beginning (bite probe: drop the context from the associated data, and the other context's
/// position opens here).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_position_is_refused_on_another_context_and_when_damaged(pool: PgPool) {
    let owner = reader(&pool, "owner").await;
    let here = own_context(&pool, owner.profile, "here").await;
    let there = own_context(&pool, owner.profile, "there").await;
    let theirs = read(&pool, owner.profile, there, q(None, None, None))
        .await
        .unwrap()
        .through;
    let ours = read(&pool, owner.profile, here, q(None, None, None))
        .await
        .unwrap()
        .through;
    let mut damaged = ours.clone().into_bytes();
    let mid = damaged.len() / 2;
    damaged[mid] = if damaged[mid] == b'A' { b'B' } else { b'A' };
    let damaged = String::from_utf8(damaged).unwrap();

    for bad in [
        theirs.as_str(),
        damaged.as_str(),
        "",
        "fp1_",
        "0",
        "fp1_AAAA",
    ] {
        let err = read(&pool, owner.profile, here, q(Some(bad), None, None))
            .await
            .expect_err(bad);
        assert_eq!(bad_request(err), POSITION_REFUSAL, "{bad:?}");
    }
    read(&pool, owner.profile, here, q(Some(&ours), None, None))
        .await
        .expect("its own position still opens");
}

/// FAILS IF: a position above the present horizon is served (bite probe: drop the horizon check,
/// and the read answers with no entries and a `through` past every change still to come).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_position_ahead_of_the_ledger_is_refused(pool: PgPool) {
    let owner = reader(&pool, "owner").await;
    let ctx = own_context(&pool, owner.profile, "notes").await;
    let current: String = sqlx::query_scalar("SELECT pg_current_xact_id()::text")
        .fetch_one(&pool)
        .await
        .unwrap();
    let ahead = current.parse::<u64>().unwrap() + 1_000_000;
    let token = position_for_test(&pool, ContextId::from(ctx), ahead, 0).await;
    let err = read(&pool, owner.profile, ctx, q(Some(&token), None, None))
        .await
        .expect_err("ahead of the ledger");
    assert_eq!(bad_request(err), POSITION_AHEAD_REFUSAL);
}

/// A kind the feed never serves is refused as unknown: an admin kind, and a connection's traffic,
/// `webhook_received` and `subscription_delivery_disposed` (`_feed_withholds_kind`, migration
/// 20261117000020), which are domain kinds.
///
/// FAILS IF: a bound is clamped or a kind silently ignored (bite probe: replace the bound check with
/// `limit.clamp(1, MAX_LIMIT)`, and 0 and 1001 answer), or a connection's traffic is accepted.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_bound_or_kind_out_of_range_is_refused_never_clamped(pool: PgPool) {
    let owner = reader(&pool, "owner").await;
    let ctx = own_context(&pool, owner.profile, "notes").await;
    for limit in [0, -1, 1001] {
        let err = read(&pool, owner.profile, ctx, q(None, Some(limit), None))
            .await
            .expect_err("out of range");
        assert!(bad_request(err).contains("limit"), "{limit}");
    }
    // Anti-vacuity: the traffic kinds are registered domain kinds, so they are refused for being
    // withheld, not for being unknown.
    let registered: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_event_types WHERE category = 'domain' \
          AND name IN ('webhook_received', 'subscription_delivery_disposed')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        registered, 2,
        "both traffic kinds are registered domain kinds"
    );
    for kinds in [
        "",
        " , ",
        "property_set,no_such_kind",
        "resource_erased",
        "webhook_received",
        "subscription_delivery_disposed",
        "property_set,webhook_received",
        // A deletion is served as its departure (migration 20261117000040).
        "resource_deleted",
    ] {
        let err = read(&pool, owner.profile, ctx, q(None, None, Some(kinds)))
            .await
            .expect_err(kinds);
        assert!(bad_request(err).contains("kind"), "{kinds:?}");
    }
    for limit in [1, 1000] {
        read(
            &pool,
            owner.profile,
            ctx,
            q(None, Some(limit), Some("property_set, resource_departed")),
        )
        .await
        .expect("in range");
    }
}

/// FAILS IF: reading the feed writes anything, or changes what the context cursor answers.
/// (What the cursor answers for an admin act is `an_admin_act_never_moves_the_cursor`.)
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn reading_records_nothing_and_the_cursor_answers_as_before(pool: PgPool) {
    let owner = reader(&pool, "owner").await;
    let ctx = own_context(&pool, owner.profile, "notes").await;
    change(&pool, ctx, owner.emitter).await;
    // Mint the key first: minting is the one write the door may make, once per deployment.
    read(&pool, owner.profile, ctx, q(None, None, None))
        .await
        .unwrap();
    settle(&pool).await;

    let cursor = |pool: PgPool| async move {
        event_service::latest_event_id_for_context(
            &pool,
            ProfileId::from(owner.profile),
            ContextId::from(ctx),
        )
        .await
        .unwrap()
    };
    let events = |pool: PgPool| async move {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM kb_events")
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    let (before_cursor, before_events) = (cursor(pool.clone()).await, events(pool.clone()).await);
    let start = read(&pool, owner.profile, ctx, q(None, None, None))
        .await
        .unwrap();
    read(
        &pool,
        owner.profile,
        ctx,
        q(Some(&start.through), None, None),
    )
    .await
    .unwrap();
    assert_eq!(cursor(pool.clone()).await, before_cursor);
    assert_eq!(events(pool.clone()).await, before_events);
    let keys: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_feed_position_keys")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(keys, 1, "one key per deployment");
}

/// Owner ruling 2026-11-06 (migration 20261117000040). An admin act's events never move the context
/// cursor, which reads them through the feed's own definition, `_event_is_admin_act`. An erasure
/// folds the erased resource's edges in their home context, and the cursor's uuidv7 dated an
/// erasure the feed never shows. That holds for a legacy act too: rows written before the order
/// columns (NULL `xact_id`) match a legacy act record by correlation id and emitter.
///
/// FAILS IF: the cursor reads every event anchored on the context (bite probe: drop the
/// `_event_is_admin_act` filter from `latest_event_id_for_context`, and the act's change becomes the
/// cursor), or the legacy span is not matched (bite probe: `=` for `IS NOT DISTINCT FROM` on
/// `xact_id`, and the legacy act's change becomes the cursor).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_admin_act_never_moves_the_cursor(pool: PgPool) {
    let admin = reader(&pool, "admin").await;
    let ctx = own_context(&pool, admin.profile, "work").await;
    let cursor = |pool: PgPool| async move {
        event_service::latest_event_id_for_context(
            &pool,
            ProfileId::from(admin.profile),
            ContextId::from(ctx),
        )
        .await
        .unwrap()
    };
    let event_of = |pool: PgPool, subject: Uuid| async move {
        sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM kb_events WHERE payload->>'resource_id' = $1::text",
        )
        .bind(subject)
        .fetch_one(&pool)
        .await
        .unwrap()
    };

    let corr = Uuid::now_v7();
    let before = correlated(&pool, ctx, admin.emitter, corr, false).await;
    assert_eq!(
        cursor(pool.clone()).await,
        Some(event_of(pool.clone(), before).await),
        "anti-vacuity: an authored change moves the cursor"
    );
    let within = correlated(&pool, ctx, admin.emitter, corr, true).await;
    assert_eq!(
        cursor(pool.clone()).await,
        Some(event_of(pool.clone(), before).await),
        "the act's own change ({within}) moved the cursor"
    );

    // A legacy act: the admin record and its change written as replay restores pre-order rows.
    let legacy_corr = Uuid::now_v7();
    let legacy = Uuid::now_v7();
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('temper.replaying', 'on', true)")
        .execute(&mut *tx)
        .await
        .unwrap();
    for (sql, what) in [
        (
            "INSERT INTO kb_events (event_type_id, emitter_entity_id, correlation_id, category, \
                                    payload, xact_id, seq) \
             SELECT id, $1, $2, 'admin', '{}'::jsonb, NULL, NULL \
               FROM kb_event_types WHERE name = 'resource_erased'",
            "a legacy admin record",
        ),
        (
            "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, \
                                    producing_anchor_id, correlation_id, payload, xact_id, seq) \
             SELECT id, $1, 'kb_contexts', $3, $2, jsonb_build_object('resource_id', $4::uuid), \
                    NULL, NULL \
               FROM kb_event_types WHERE name = 'property_set'",
            "a legacy change",
        ),
    ] {
        sqlx::query(sql)
            .bind(admin.emitter)
            .bind(legacy_corr)
            .bind(ctx)
            .bind(legacy)
            .execute(&mut *tx)
            .await
            .expect(what);
    }
    tx.commit().await.unwrap();
    let legacy_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events WHERE correlation_id = $1 AND xact_id IS NULL",
    )
    .bind(legacy_corr)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        legacy_rows, 2,
        "anti-vacuity: both legacy rows have no order"
    );
    assert_eq!(
        cursor(pool.clone()).await,
        Some(event_of(pool.clone(), before).await),
        "the legacy act's change ({legacy}) moved the cursor"
    );

    let after = correlated(&pool, ctx, admin.emitter, corr, false).await;
    assert_eq!(
        cursor(pool.clone()).await,
        Some(event_of(pool.clone(), after).await),
        "an authored change after the act moves the cursor"
    );
}

/// One event per transaction, sharing correlation id `corr` and emitter `emitter`: a domain change
/// in `context` naming a fresh subject, preceded in the same transaction by an admin record when
/// `with_admin_record`. Returns the subject.
async fn correlated(
    pool: &PgPool,
    context: Uuid,
    emitter: Uuid,
    corr: Uuid,
    with_admin_record: bool,
) -> Uuid {
    let subject = Uuid::now_v7();
    let mut tx = pool.begin().await.unwrap();
    if with_admin_record {
        sqlx::query(
            "INSERT INTO kb_events (event_type_id, emitter_entity_id, correlation_id, category, payload) \
             SELECT id, $1, $2, 'admin', '{}'::jsonb FROM kb_event_types WHERE name = 'resource_erased'",
        )
        .bind(emitter)
        .bind(corr)
        .execute(&mut *tx)
        .await
        .expect("an admin record");
    }
    sqlx::query(
        "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, \
                                producing_anchor_id, correlation_id, payload) \
         SELECT id, $1, 'kb_contexts', $2, $3, jsonb_build_object('resource_id', $4::uuid) \
           FROM kb_event_types WHERE name = 'property_set'",
    )
    .bind(emitter)
    .bind(context)
    .bind(corr)
    .bind(subject)
    .execute(&mut *tx)
    .await
    .expect("a correlated change");
    tx.commit().await.unwrap();
    subject
}

/// FAILS IF: the feed withholds an authored change because it shares an admin act's correlation id
/// and emitter from another transaction (bite probe: drop `act.xact_id = p_xact` from
/// `_event_is_admin_act`, and the changes made before and after the act vanish), or serves a
/// change the act itself wrote.
///
/// Correlation ids are chosen by callers, so without the transaction rule an admin could hide
/// their own authored writes from every reader by giving them an act's correlation id.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn only_an_admin_acts_own_transaction_is_withheld(pool: PgPool) {
    let admin = reader(&pool, "admin").await;
    let ctx = own_context(&pool, admin.profile, "work").await;
    let start = read(&pool, admin.profile, ctx, q(None, None, None))
        .await
        .unwrap()
        .through;

    let corr = Uuid::now_v7();
    let before = correlated(&pool, ctx, admin.emitter, corr, false).await;
    let within = correlated(&pool, ctx, admin.emitter, corr, true).await;
    let after = correlated(&pool, ctx, admin.emitter, corr, false).await;
    settle(&pool).await;

    let served: Vec<Uuid> = read(&pool, admin.profile, ctx, q(Some(&start), None, None))
        .await
        .unwrap()
        .entries
        .iter()
        .map(|e| e.subject.id)
        .collect();
    assert_eq!(
        served,
        vec![before, after],
        "the act's own change ({within}) is withheld"
    );
}
