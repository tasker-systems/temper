//! Conditional meta writes through `DbBackend::update_resource`: a write that carries `expect` is
//! judged under the resource's writer lock, in its own transaction, after the caller's read
//! standing, and lands exactly as it would have without one or not at all.
//!
//! Every witness drives the backend command the HTTP doors build (`PUT /meta` and `PATCH`), so the
//! sequence under test is the one the doors run. The HTTP wire shape is witnessed in temper-api.
#![cfg(feature = "test-db")]

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sqlx::PgPool;

use temper_core::error::TemperError;
use temper_core::types::authorship::ActContext;
use temper_core::types::erasure::ScrubFieldKind;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_core::types::precondition::{MetaExpect, PreconditionFailure, PreconditionTier};
use temper_services::backend::{substrate_read, DbBackend};
use temper_services::services::field_scrub_service::{
    execute_field_scrub, list_field_scrub_families, FieldScrubOutcome, FieldScrubRequest,
};
use temper_services::test_support;
use temper_workflow::operations::{Backend, CreateResource, ShowResource, Surface, UpdateResource};
use temper_workflow::types::managed_meta::ManagedMeta;
use uuid::Uuid;

// ── fixtures ─────────────────────────────────────────────────────────────────────────────────────

/// Seed a substrate profile, its emitters, and a profile-owned context. Each test-target crate
/// keeps its own copy (`write_lock_bound_test.rs`).
async fn seed_profile(pool: &PgPool, email: &str) -> (Uuid, Uuid) {
    let profile_id = Uuid::now_v7();
    let local = email.split('@').next().unwrap_or("test-user");
    let handle = format!("{local}-{}", &profile_id.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1,$2,$3,$4)")
        .bind(profile_id)
        .bind(&handle)
        .bind(email)
        .bind(email)
        .execute(pool)
        .await
        .expect("seed profile");
    for surface in ["web", "cli", "mcp"] {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,$2,'{}'::jsonb)",
        )
        .bind(profile_id)
        .bind(format!("{handle}@{surface}"))
        .execute(pool)
        .await
        .expect("seed emitter entity");
    }
    let context_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,'claims','claims')",
    )
    .bind(context_id)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("seed context");
    (profile_id, context_id)
}

fn backend(pool: &PgPool, profile: Uuid) -> DbBackend {
    DbBackend::new(pool.clone(), ProfileId::from(profile))
}

/// A research resource in `context`, born with `open` as its open tier.
async fn create(pool: &PgPool, owner: Uuid, context: Uuid, open: Value) -> ResourceId {
    create_typed(pool, owner, context, "research", open).await
}

async fn create_typed(
    pool: &PgPool,
    owner: Uuid,
    context: Uuid,
    doctype: &str,
    open: Value,
) -> ResourceId {
    backend(pool, owner)
        .create_resource(CreateResource {
            idempotency_key: None,
            slug: "guarded".to_string(),
            doctype: doctype.to_string(),
            home: HomeAnchor::Context(ContextId::from(context)),
            title: "guarded".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: Some(open),
            goal: None,
            origin_uri: Some(format!("test://guarded/{}", Uuid::now_v7())),
            chunks_packed: None,
            content_hash: None,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("create")
        .value
        .id
}

fn expect(v: Value) -> MetaExpect {
    serde_json::from_value(v).expect("a well-formed expectation")
}

/// The command `PUT /api/resources/{id}/meta` builds (`handlers/meta.rs`).
fn put_meta(
    resource: ResourceId,
    managed: ManagedMeta,
    open: Value,
    guard: Option<MetaExpect>,
) -> UpdateResource {
    UpdateResource {
        resource,
        title: None,
        slug: None,
        body: None,
        managed_meta: Some(managed),
        open_meta: Some(open),
        open_meta_add: None,
        goal: None,
        move_to: None,
        context_ref: None,
        expect: guard,
        act: ActContext::default(),
        origin: Surface::ApiHttp,
    }
}

/// The command a meta-only `PATCH /api/resources/{id}` builds: open tier only, so the update reads
/// no identity row before its transaction.
fn patch_open(resource: ResourceId, open: Value, guard: Option<MetaExpect>) -> UpdateResource {
    UpdateResource {
        managed_meta: None,
        ..put_meta(resource, ManagedMeta::default(), open, guard)
    }
}

/// The open tier `GET /api/resources/{id}/meta` answers.
async fn open_tier(pool: &PgPool, reader: Uuid, resource: ResourceId) -> Value {
    substrate_read::get_meta_select(pool, ProfileId::from(reader), resource)
        .await
        .expect("read meta")
        .open_meta
        .unwrap_or(Value::Null)
}

fn failure(answer: Result<impl std::fmt::Debug, TemperError>) -> PreconditionFailure {
    match answer {
        Err(TemperError::PreconditionFailed(f)) => f,
        other => panic!("expected PreconditionFailed, got {other:?}"),
    }
}

fn bad_request(answer: Result<impl std::fmt::Debug, TemperError>) -> String {
    match answer {
        Err(TemperError::BadRequest(m)) => m,
        other => panic!("expected BadRequest, got {other:?}"),
    }
}

/// Every event on the ledger (each test has its own database).
async fn events(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(pool)
        .await
        .expect("count events")
}

/// R's property rows, live and folded, in a stable order.
async fn property_rows(pool: &PgPool, resource: ResourceId) -> Vec<(String, Value, bool)> {
    sqlx::query_as(
        "SELECT property_key, property_value, is_folded FROM kb_properties \
          WHERE owner_table = 'kb_resources' AND owner_id = $1 ORDER BY id",
    )
    .bind(Uuid::from(resource))
    .fetch_all(pool)
    .await
    .expect("property rows")
}

/// Grant `grantee` read and write on `context`, the second owner's standing.
async fn grant_context_write(pool: &PgPool, context: Uuid, grantee: Uuid, by: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, \
                                       can_read, can_write, granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, true, $3)",
    )
    .bind(context)
    .bind(grantee)
    .bind(by)
    .execute(pool)
    .await
    .expect("grant context write");
}

/// Hold R's row `FOR NO KEY UPDATE`, the lock every writer of R takes, on a side transaction.
async fn hold_writer_lock(
    pool: &PgPool,
    resource: ResourceId,
) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut held = pool.begin().await.expect("begin");
    sqlx::query("SELECT 1 FROM kb_resources WHERE id = $1 FOR NO KEY UPDATE")
        .bind(Uuid::from(resource))
        .execute(&mut *held)
        .await
        .expect("take the writer lock");
    held
}

/// Poll until `n` backends wait on R's writer lock: a row-lock wait (`tuple` or `transactionid`)
/// while running `lock_resource_for_write`'s statement. Fails the moment any backend waits on an
/// advisory lock instead, which would mean the writers serialized on the act queue rather than on
/// the lock the precondition depends on. Ported from temper-api's husk-write witness.
async fn writers_wait_on_the_row_lock(pool: &PgPool, n: i64) {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let advisory: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
              WHERE datname = current_database() AND wait_event_type = 'Lock' \
                AND wait_event = 'advisory'",
        )
        .fetch_one(pool)
        .await
        .expect("poll advisory waits");
        assert_eq!(advisory, 0, "a writer waited on an advisory lock");
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
              WHERE datname = current_database() AND wait_event_type = 'Lock' \
                AND wait_event IN ('tuple', 'transactionid') \
                AND query ILIKE '%FROM kb_resources WHERE id = $1 FOR NO KEY UPDATE%'",
        )
        .fetch_one(pool)
        .await
        .expect("poll row-lock waits");
        if waiting >= n {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{waiting} of {n} writers waited on the writer lock within 4 seconds"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// A map in the admin-only regime (bound to the gating team), born with a telos owned by someone
/// else, and a system admin who may therefore modify the telos but is no member of the team and so
/// cannot read it. Returns `(admin, telos)`. The standing is asserted, so the fixture cannot drift
/// into an ordinary reader.
async fn modify_but_not_read(pool: &PgPool) -> (Uuid, ResourceId) {
    let (admin, _) = seed_profile(pool, "regime-admin@example.com").await;
    test_support::approved_admin(pool, admin).await;
    let (owner, _) = seed_profile(pool, "regime-owner@example.com").await;
    let cogmap = Uuid::now_v7();
    let telos = Uuid::now_v7();
    sqlx::query(
        "SELECT cogmap_genesis($1, '{}'::jsonb, \
             (SELECT e.id FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id \
               WHERE p.handle = 'system' AND e.name = 'system'))",
    )
    .bind(json!({
        "cogmap_id": cogmap,
        "name": format!("regime-{cogmap}"),
        "owner_profile_id": owner,
        "telos": {"resource_id": telos, "title": "telos",
                  "origin_uri": format!("temper://test/{cogmap}/telos"), "blocks": []},
    }))
    .execute(pool)
    .await
    .expect("birth the map");
    let team = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_teams (id, slug, name) VALUES ($1, 'regime-gate', 'regime-gate')")
        .bind(team)
        .execute(pool)
        .await
        .expect("seed the gating team");
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .expect("bind the map");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'regime-gate' WHERE id = 1")
        .execute(pool)
        .await
        .expect("point the gate at it");
    let (modify, read): (bool, bool) = sqlx::query_as(
        "SELECT can_modify_resource($1, $2), \
                EXISTS (SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2)",
    )
    .bind(admin)
    .bind(telos)
    .fetch_one(pool)
    .await
    .expect("standing");
    assert!(
        modify && !read,
        "the fixture is modify-but-not-read: modify {modify}, read {read}"
    );
    (admin, ResourceId::from(telos))
}

// ── the race ─────────────────────────────────────────────────────────────────────────────────────

/// **Two concurrent claimants, two owners: exactly one lands.** A side transaction holds R's writer
/// lock; A and B each claim `claim_id` expecting it absent, and both are seen waiting on that row
/// lock (never on an advisory lock) before it is released. One answers 200; the other answers 412
/// with `current` equal to the winner's claim, and the winner's claim is what is stored.
///
/// FAILS IF the precondition is judged on a read taken before the transaction, or after
/// `begin_floored` but before `lock_resource_for_write`: both claimants then read "absent" and
/// both land.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn two_claimants_with_two_owners_exactly_one_lands(pool: PgPool) {
    let (alice, context) = seed_profile(&pool, "alice@example.com").await;
    let (bob, _) = seed_profile(&pool, "bob@example.com").await;
    grant_context_write(&pool, context, bob, alice).await;
    let resource = create(&pool, alice, context, json!({"topic": "x"})).await;

    let held = hold_writer_lock(&pool, resource).await;
    let claim = |who: Uuid| {
        let pool = pool.clone();
        tokio::spawn(async move {
            backend(&pool, who)
                .update_resource(put_meta(
                    resource,
                    ManagedMeta::default(),
                    json!({"claim_id": who.to_string()}),
                    Some(expect(json!({"open": {"claim_id": null}}))),
                ))
                .await
        })
    };
    let a = claim(alice);
    let b = claim(bob);
    writers_wait_on_the_row_lock(&pool, 2).await;
    held.commit().await.expect("release the writer lock");
    let answers = [(alice, a.await.unwrap()), (bob, b.await.unwrap())];

    let landed: Vec<Uuid> = answers
        .iter()
        .filter(|(_, r)| r.is_ok())
        .map(|(who, _)| *who)
        .collect();
    assert_eq!(landed.len(), 1, "exactly one claim lands: {answers:?}");
    let winner = landed[0];
    let (_, refused) = answers.into_iter().find(|(who, _)| *who != winner).unwrap();
    let f = failure(refused);
    assert_eq!(f.failed.len(), 1);
    assert_eq!(f.failed[0].key.as_deref(), Some("claim_id"));
    assert_eq!(f.failed[0].expected, Value::Null);
    assert_eq!(f.failed[0].current, json!(winner.to_string()));
    assert_eq!(
        open_tier(&pool, alice, resource).await["claim_id"],
        json!(winner.to_string())
    );
}

// ── the comparison ───────────────────────────────────────────────────────────────────────────────

/// **A stale meta write never lands**, for a wrong value and for "absent" expected on a present
/// key. Nothing is written and no event is recorded.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_stale_meta_write_never_lands(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "stale@example.com").await;
    let resource = create(&pool, owner, context, json!({"lease": "mine"})).await;
    let be = backend(&pool, owner);
    let rows = property_rows(&pool, resource).await;
    let before = events(&pool).await;

    for (guard, current) in [
        (json!({"open": {"lease": "theirs"}}), json!("mine")),
        (json!({"open": {"lease": null}}), json!("mine")),
    ] {
        let f = failure(
            be.update_resource(put_meta(
                resource,
                ManagedMeta::default(),
                json!({"lease": "next"}),
                Some(expect(guard)),
            ))
            .await,
        );
        assert_eq!(f.failed[0].current, current);
    }
    assert_eq!(
        property_rows(&pool, resource).await,
        rows,
        "nothing was written"
    );
    assert_eq!(events(&pool).await, before, "nothing was recorded");
}

/// **"Absent" expected on an absent key holds**, and the write lands.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn absent_expected_on_an_absent_key_holds(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "absent@example.com").await;
    let resource = create(&pool, owner, context, json!({"topic": "x"})).await;
    backend(&pool, owner)
        .update_resource(patch_open(
            resource,
            json!({"claim_id": "c1"}),
            Some(expect(json!({"open": {"claim_id": null}}))),
        ))
        .await
        .expect("an absent key is absent");
    assert_eq!(open_tier(&pool, owner, resource).await["claim_id"], "c1");
}

/// **All or nothing**: three keys written, two guarded, one guard failing. None of the three
/// lands, and the refusal names exactly the failing guard.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn all_or_nothing(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "allornothing@example.com").await;
    let resource = create(&pool, owner, context, json!({"a": 1, "b": 2})).await;
    let rows = property_rows(&pool, resource).await;
    let f = failure(
        backend(&pool, owner)
            .update_resource(patch_open(
                resource,
                json!({"a": 10, "b": 20, "c": 30}),
                Some(expect(json!({"open": {"a": 1, "b": 99}}))),
            ))
            .await,
    );
    assert_eq!(f.failed.len(), 1, "{f:?}");
    assert_eq!(f.failed[0].key.as_deref(), Some("b"));
    assert_eq!(f.failed[0].expected, json!(99));
    assert_eq!(f.failed[0].current, json!(2));
    assert_eq!(
        property_rows(&pool, resource).await,
        rows,
        "none of the three landed"
    );
}

/// **A guard may name a key the write does not write**, and it is judged either way.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_guard_may_name_an_unwritten_key(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "unwritten@example.com").await;
    let resource = create(&pool, owner, context, json!({"lease": "mine"})).await;
    let be = backend(&pool, owner);
    be.update_resource(patch_open(
        resource,
        json!({"step": "two"}),
        Some(expect(json!({"open": {"lease": "mine"}}))),
    ))
    .await
    .expect("a holding guard on an unwritten key admits the write");
    let f = failure(
        be.update_resource(patch_open(
            resource,
            json!({"step": "three"}),
            Some(expect(json!({"open": {"lease": "gone"}}))),
        ))
        .await,
    );
    assert_eq!(f.failed[0].key.as_deref(), Some("lease"));
    assert_eq!(open_tier(&pool, owner, resource).await["step"], "two");
}

/// Insert one more live row for `key` on R, newer than every other row, as a second assertion
/// that did not fold the first would leave it.
async fn insert_newer_live_row(pool: &PgPool, resource: ResourceId, key: &str, value: Value) {
    let event: Uuid = sqlx::query_scalar(
        "SELECT asserted_by_event_id FROM kb_properties WHERE owner_id = $1 LIMIT 1",
    )
    .bind(Uuid::from(resource))
    .fetch_one(pool)
    .await
    .expect("an event to attribute the row to");
    sqlx::query(
        "INSERT INTO kb_properties (owner_table, owner_id, property_key, property_value, \
                                    asserted_by_event_id, last_event_id, created) \
         VALUES ('kb_resources', $1, $2, $3, $4, $4, now() + interval '1 second')",
    )
    .bind(Uuid::from(resource))
    .bind(key)
    .bind(value)
    .bind(event)
    .execute(pool)
    .await
    .expect("insert a newer live row");
}

/// **Newest wins over two live rows for one key**, exactly as the read classifies them.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn newest_wins_over_two_live_rows(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "newest@example.com").await;
    let resource = create(&pool, owner, context, json!({"stage_note": "old"})).await;
    insert_newer_live_row(&pool, resource, "stage_note", json!("new")).await;
    assert_eq!(open_tier(&pool, owner, resource).await["stage_note"], "new");
    let be = backend(&pool, owner);
    let f = failure(
        be.update_resource(patch_open(
            resource,
            json!({"x": 1}),
            Some(expect(json!({"open": {"stage_note": "old"}}))),
        ))
        .await,
    );
    assert_eq!(f.failed[0].current, json!("new"));
    be.update_resource(patch_open(
        resource,
        json!({"x": 1}),
        Some(expect(json!({"open": {"stage_note": "new"}}))),
    ))
    .await
    .expect("the newest live row is the current value");
}

/// **A stored top-level `null` is absent.** The newest row for the key holds jsonb `null`, over an
/// older non-null live row. Expecting `null` holds; expecting the older value fails with
/// `current: null`, and the older value never surfaces.
///
/// FAILS IF the stored `null` is filtered out inside the newest-wins read rather than read as
/// absent after it: the older value then surfaces and the stale guard holds.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_stored_null_reads_as_absent(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "storednull@example.com").await;
    let resource = create(&pool, owner, context, json!({"legacy": "beneath"})).await;
    insert_newer_live_row(&pool, resource, "legacy", Value::Null).await;
    assert_eq!(
        open_tier(&pool, owner, resource).await.get("legacy"),
        Some(&Value::Null),
        "the read shows the key with value null"
    );
    let be = backend(&pool, owner);
    let f = failure(
        be.update_resource(patch_open(
            resource,
            json!({"x": 1}),
            Some(expect(json!({"open": {"legacy": "beneath"}}))),
        ))
        .await,
    );
    assert_eq!(f.failed[0].current, Value::Null);
    assert!(
        f.failed.iter().all(|e| e.current != json!("beneath")),
        "the older value beneath the stored null never surfaces"
    );
    be.update_resource(patch_open(
        resource,
        json!({"x": 1}),
        Some(expect(json!({"open": {"legacy": null}}))),
    ))
    .await
    .expect("a stored null is absent");
}

/// **Values compare as stored**: a bare-string `tags` guard matches its stored array, and `1`
/// matches `1.0`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn values_compare_after_normalization(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "normalized@example.com").await;
    let resource = create(&pool, owner, context, json!({"tags": "x", "n": 1})).await;
    assert_eq!(
        open_tier(&pool, owner, resource).await["tags"],
        json!(["x"])
    );
    let be = backend(&pool, owner);
    for guard in [
        json!({"open": {"tags": "x", "n": 1.0}}),
        json!({"open": {"tags": ["x"], "n": 1}}),
    ] {
        be.update_resource(patch_open(
            resource,
            json!({"y": 1}),
            Some(expect(guard.clone())),
        ))
        .await
        .unwrap_or_else(|e| panic!("{guard} holds: {e:?}"));
    }
}

/// **The managed tier is judged too**, and `null` means absent there as well.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_managed_guard_is_judged(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "managed@example.com").await;
    let resource = create_typed(&pool, owner, context, "task", json!({"topic": "x"})).await;
    let be = backend(&pool, owner);
    let stage = |s: &str| ManagedMeta {
        stage: Some(s.to_string()),
        ..ManagedMeta::default()
    };
    be.update_resource(put_meta(
        resource,
        stage("in-progress"),
        json!({}),
        Some(expect(
            json!({"managed": {"temper-stage": "backlog", "temper-pr": null}}),
        )),
    ))
    .await
    .expect("the defaulted stage, and an absent managed key");
    let f = failure(
        be.update_resource(put_meta(
            resource,
            stage("done"),
            json!({}),
            Some(expect(json!({"managed": {"temper-stage": "backlog"}}))),
        ))
        .await,
    );
    assert_eq!(f.failed[0].tier, PreconditionTier::Managed);
    assert_eq!(f.failed[0].key.as_deref(), Some("temper-stage"));
    assert_eq!(f.failed[0].current, json!("in-progress"));
}

// ── standing and order ───────────────────────────────────────────────────────────────────────────

/// **Read standing comes first.** A caller who may modify a resource it cannot read gets GET's
/// 404 for a guard that would hold and for one that would fail, never a 412; and the unlocked
/// pre-check answers it without taking the writer lock.
///
/// FAILS IF the unlocked pre-check is skipped: the request then waits on the held lock before the
/// locked gate refuses it.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_modifier_who_cannot_read_gets_the_404_get_answers(pool: PgPool) {
    let (admin, telos) = modify_but_not_read(&pool).await;
    let be = backend(&pool, admin);
    let get = be
        .show_resource(ShowResource {
            resource: telos,
            origin: Surface::ApiHttp,
        })
        .await
        .expect_err("the admin cannot read the telos");
    let get = get.to_string();

    let held = hold_writer_lock(&pool, telos).await;
    for guard in [
        json!({"open": {"claim": null}}),
        json!({"open": {"claim": "x"}}),
    ] {
        let asked = Instant::now();
        let answer = be
            .update_resource(patch_open(
                telos,
                json!({"claim": "x"}),
                Some(expect(guard)),
            ))
            .await;
        assert!(
            asked.elapsed() < Duration::from_secs(2),
            "the pre-check answered without waiting on the held writer lock"
        );
        match answer {
            Err(e @ TemperError::NotFound(_)) => assert_eq!(e.to_string(), get, "GET's 404"),
            other => panic!("expected GET's 404, got {other:?}"),
        }
        // The PUT /meta door's identity read refuses it as GET does, before the pre-check.
        match be
            .update_resource(put_meta(
                telos,
                ManagedMeta::default(),
                json!({"claim": "x"}),
                Some(expect(json!({"open": {"claim": null}}))),
            ))
            .await
        {
            Err(TemperError::NotFound(_)) => {}
            other => panic!("expected 404 on PUT /meta, got {other:?}"),
        }
    }
    held.commit().await.expect("release");
}

/// **The locked gate decides.** A reader passes the pre-check and waits on the writer lock; its
/// read standing is withdrawn while it waits. Under the lock it gets GET's 404, never the 412 its
/// failing guard would earn.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn read_standing_lost_while_waiting_answers_404_not_412(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "gate-owner@example.com").await;
    let (guest, _) = seed_profile(&pool, "gate-guest@example.com").await;
    grant_context_write(&pool, context, guest, owner).await;
    let resource = create(&pool, owner, context, json!({"lease": "owner"})).await;

    let held = hold_writer_lock(&pool, resource).await;
    let request = {
        let pool = pool.clone();
        tokio::spawn(async move {
            backend(&pool, guest)
                .update_resource(patch_open(
                    resource,
                    json!({"lease": "guest"}),
                    Some(expect(json!({"open": {"lease": "nobody"}}))),
                ))
                .await
        })
    };
    writers_wait_on_the_row_lock(&pool, 1).await;
    sqlx::query("DELETE FROM kb_access_grants WHERE principal_id = $1")
        .bind(guest)
        .execute(&pool)
        .await
        .expect("withdraw the guest's standing");
    held.commit().await.expect("release");
    match request.await.unwrap() {
        Err(TemperError::NotFound(_)) => {}
        other => panic!("expected the locked gate's 404, got {other:?}"),
    }
    assert_eq!(open_tier(&pool, owner, resource).await["lease"], "owner");
}

/// **Each 400** an `expect` earns from its own shape, and **the order of refusals**: the modify
/// floor first, then the expectation's shape, then the update's existing validation, then
/// "nothing to write", then read standing, then the precondition.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn each_400_and_the_order_of_refusals(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "order@example.com").await;
    let (stranger, _) = seed_profile(&pool, "stranger@example.com").await;
    let resource = create(&pool, owner, context, json!({"lease": "mine"})).await;
    let be = backend(&pool, owner);
    let facet = Some(expect(json!({"open": {"facet": null}})));

    // 1. The modify floor precedes the shape rules.
    match backend(&pool, stranger)
        .update_resource(patch_open(resource, json!({"x": 1}), facet.clone()))
        .await
    {
        Err(TemperError::Forbidden) => {}
        other => panic!("a non-modifier is refused by the floor first, got {other:?}"),
    }

    // 2. Each shape 400.
    for (guard, add, names) in [
        (json!({"open": {"facet": null}}), None, "expect.open.facet"),
        (
            json!({"open": {"doc_type": "x"}}),
            None,
            "expect.open.doc_type",
        ),
        (
            json!({"open": {"temper-stage": "x"}}),
            None,
            "expect.open.temper-stage",
        ),
        (json!({"open": {"temper-title": "x"}}), None, "identity key"),
        (
            json!({"managed": {"lease": "x"}}),
            None,
            "expect.managed.lease",
        ),
        (json!({"open": {"lease": "a\u{0}b"}}), None, "U+0000"),
        (json!({}), None, "names no key"),
        (
            json!({"open": {"tags": ["a"]}}),
            Some(json!({"tags": ["b"]})),
            "open_meta_add",
        ),
    ] {
        let mut cmd = patch_open(resource, json!({"x": 1}), Some(expect(guard)));
        cmd.open_meta_add = add;
        let m = bad_request(be.update_resource(cmd).await);
        assert!(m.contains(names), "{names}: {m}");
    }

    // 3. The shape rules precede the update's own validation (an open-tier shape error).
    let m = bad_request(
        be.update_resource(patch_open(resource, json!({"tags": 7}), facet.clone()))
            .await,
    );
    assert!(m.contains("expect.open.facet"), "{m}");
    let holding = Some(expect(json!({"open": {"lease": "mine"}})));
    let m = bad_request(
        be.update_resource(patch_open(resource, json!({"tags": 7}), holding.clone()))
            .await,
    );
    assert!(m.contains("invalid open_meta shape"), "{m}");

    // 4. "Nothing to write" follows the update's validation and precedes read standing: an empty
    //    write is refused even for a caller who could not read the resource.
    let failing = Some(expect(json!({"open": {"lease": "theirs"}})));
    let m = bad_request(
        be.update_resource(put_meta(
            resource,
            ManagedMeta::default(),
            json!({}),
            failing.clone(),
        ))
        .await,
    );
    assert!(m.contains("writes nothing"), "{m}");
    let (admin, telos) = modify_but_not_read(&pool).await;
    let m = bad_request(
        backend(&pool, admin)
            .update_resource(patch_open(telos, json!({}), failing.clone()))
            .await,
    );
    assert!(m.contains("writes nothing"), "{m}");

    // 5. Read standing, then 6. the precondition.
    match backend(&pool, admin)
        .update_resource(patch_open(telos, json!({"x": 1}), failing.clone()))
        .await
    {
        Err(TemperError::NotFound(_)) => {}
        other => panic!("read standing precedes the precondition, got {other:?}"),
    }
    failure(
        be.update_resource(patch_open(resource, json!({"x": 1}), failing))
            .await,
    );
}

/// **"Nothing to write" is judged on the assembled write.** A same-value set is a write (it fires
/// events); an unset is a write; an empty tier, or a null managed key the meta path drops, is not.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn nothing_to_write_is_judged_on_the_assembled_write(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "nothing@example.com").await;
    let resource = create(&pool, owner, context, json!({"lease": "mine", "gone": 1})).await;
    let be = backend(&pool, owner);
    let holds = || Some(expect(json!({"open": {"lease": "mine"}})));

    let before = events(&pool).await;
    be.update_resource(patch_open(resource, json!({"lease": "mine"}), holds()))
        .await
        .expect("a same-value set is a write");
    assert!(events(&pool).await > before, "and it fired events");
    be.update_resource(patch_open(resource, json!({"gone": null}), holds()))
        .await
        .expect("an unset is a write");

    let managed_null: ManagedMeta =
        serde_json::from_value(json!({"temper-stage": null})).expect("parses");
    for cmd in [
        patch_open(resource, json!({}), holds()),
        put_meta(resource, ManagedMeta::default(), json!({}), holds()),
        put_meta(resource, managed_null, json!({}), holds()),
    ] {
        let m = bad_request(be.update_resource(cmd).await);
        assert!(m.contains("writes nothing"), "{m}");
    }
}

// ── the refusal and what it leaves ───────────────────────────────────────────────────────────────

/// **The refusal**: every entry carries its key and the current value; the message names keys
/// only; and nothing Temper adds (no actor, time, event or id) appears anywhere in it.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_refusal_carries_values_and_nothing_else(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "refusal@example.com").await;
    let resource = create(
        &pool,
        owner,
        context,
        json!({"claim": "c-held", "lease": "l-held"}),
    )
    .await;
    let answer = backend(&pool, owner)
        .update_resource(put_meta(
            resource,
            ManagedMeta::default(),
            json!({"x": 1}),
            Some(expect(json!({
                "open": {"claim": "c-mine", "lease": "l-held", "absent": "a-mine"},
                "managed": {"temper-stage": "s-mine"},
            }))),
        ))
        .await;
    let f = failure(answer);
    let message = TemperError::PreconditionFailed(f.clone()).to_string();
    assert_eq!(
        message,
        "3 preconditions failed: open.absent, open.claim, managed.temper-stage"
    );
    for value in ["c-held", "c-mine", "a-mine", "s-mine", "l-held"] {
        assert!(
            !message.contains(value),
            "the message carries no value: {message}"
        );
    }
    let wire = serde_json::to_value(&f).unwrap();
    assert_eq!(
        wire,
        json!({"failed": [
            {"tier": "open", "key": "absent", "expected": "a-mine", "current": null},
            {"tier": "open", "key": "claim", "expected": "c-mine", "current": "c-held"},
            {"tier": "managed", "key": "temper-stage", "expected": "s-mine", "current": null},
        ]}),
        "uniform entries, a key on each, nothing added"
    );
    let text = wire.to_string();
    for id in [owner, context, Uuid::from(resource)] {
        assert!(!text.contains(&id.to_string()), "no id in the refusal");
    }
}

/// **Nothing is held after a refusal**: no event, no row, and the next writer takes R's writer
/// lock at once.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn nothing_is_held_after_a_refusal(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "held@example.com").await;
    let resource = create(&pool, owner, context, json!({"lease": "mine"})).await;
    let rows = property_rows(&pool, resource).await;
    let before = events(&pool).await;
    failure(
        backend(&pool, owner)
            .update_resource(patch_open(
                resource,
                json!({"lease": "next"}),
                Some(expect(json!({"open": {"lease": "theirs"}}))),
            ))
            .await,
    );
    assert_eq!(events(&pool).await, before);
    assert_eq!(property_rows(&pool, resource).await, rows);
    let mut next = pool.begin().await.expect("begin");
    sqlx::query("SELECT 1 FROM kb_resources WHERE id = $1 FOR NO KEY UPDATE NOWAIT")
        .bind(Uuid::from(resource))
        .execute(&mut *next)
        .await
        .expect("the writer lock is free the moment the refusal is answered");
    next.rollback().await.unwrap();
}

/// The event types the most recent write recorded, in order.
async fn event_types_since(pool: &PgPool, after: i64) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT t.name FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          ORDER BY e.id OFFSET $1",
    )
    .bind(after)
    .fetch_all(pool)
    .await
    .expect("event types")
}

/// **A landed conditional write is ordinary**: the same events and the same answer as the same
/// write without a precondition, on a twin resource.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_landed_conditional_write_is_ordinary(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "ordinary@example.com").await;
    let guarded = create(&pool, owner, context, json!({"lease": "mine"})).await;
    let plain = create(&pool, owner, context, json!({"lease": "mine"})).await;
    let be = backend(&pool, owner);

    let n = events(&pool).await;
    let a = be
        .update_resource(put_meta(
            guarded,
            ManagedMeta::default(),
            json!({"lease": "next", "gone": null}),
            Some(expect(json!({"open": {"lease": "mine"}}))),
        ))
        .await
        .expect("guarded write")
        .value;
    let guarded_events = event_types_since(&pool, n).await;
    let n = events(&pool).await;
    let b = be
        .update_resource(put_meta(
            plain,
            ManagedMeta::default(),
            json!({"lease": "next", "gone": null}),
            None,
        ))
        .await
        .expect("plain write")
        .value;
    assert_eq!(
        event_types_since(&pool, n).await,
        guarded_events,
        "the same events"
    );
    assert_eq!(a.open_meta, b.open_meta, "the same answer");
    assert_eq!(a.managed_meta, b.managed_meta);
}

/// **No admin-act disclosure.** After an admin clears and scrubs an open key, the precondition
/// judges exactly what a read shows: expecting the scrubbed value fails with the read's current
/// value, and nothing the act removed comes back in the refusal.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_scrubbed_value_never_surfaces(pool: PgPool) {
    let (owner, context) = seed_profile(&pool, "scrubbed@example.com").await;
    let resource = create(&pool, owner, context, json!({"note": "hush-hush"})).await;
    let (operator, _) = seed_profile(&pool, "operator@example.com").await;
    test_support::approved_admin(&pool, operator).await;
    let proof = test_support::system_admin_proof_for(&pool, operator).await;
    let families = list_field_scrub_families(&pool, &proof, resource)
        .await
        .expect("families");
    let mut family = None;
    for f in families
        .families
        .iter()
        .filter(|f| f.field == ScrubFieldKind::Property)
    {
        let id = f.family.expect("a property family has a handle").uuid();
        let payload: String =
            sqlx::query_scalar("SELECT payload::text FROM kb_events WHERE id = $1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .expect("family event");
        if payload.contains("\"note\"") {
            family = Some(id);
        }
    }
    let outcome = execute_field_scrub(
        &pool,
        &proof,
        FieldScrubRequest {
            resource,
            field: ScrubFieldKind::Property,
            family: Some(family.expect("the note family")),
            clear: true,
            surface: Surface::ApiHttp,
        },
    )
    .await
    .expect("scrub");
    assert!(
        matches!(outcome, FieldScrubOutcome::Completed(_)),
        "{outcome:?}"
    );

    let read = open_tier(&pool, owner, resource).await;
    let shown = read.get("note").cloned().unwrap_or(Value::Null);
    let f = failure(
        backend(&pool, owner)
            .update_resource(patch_open(
                resource,
                json!({"x": 1}),
                Some(expect(json!({"open": {"note": "the-old-value"}}))),
            ))
            .await,
    );
    assert_eq!(f.failed[0].current, shown, "current is what the read shows");
    assert!(
        !serde_json::to_string(&f).unwrap().contains("hush-hush"),
        "the scrubbed value never comes back"
    );
}
