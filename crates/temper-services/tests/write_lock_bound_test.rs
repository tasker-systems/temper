//! The write-side lock bound (task 01a0fd12-f4b7-7bd2-81d0-13c0814650d5).
//!
//! A floored write's transaction carries `lock_timeout` ([`write_floor::WRITE_LOCK_TIMEOUT_MS`]),
//! set by the floor before its row lock. A wait past it answers `TemperError::ResourceBusy` (`503
//! RESOURCE_BUSY` on the wire) with nothing applied. The erasure act takes no floor and pins
//! `lock_timeout = 0` on its function (migration `20261013100000`), so no bound on the pool, the
//! role or a session can stop it completing.
//!
//! The act's `FOR UPDATE` on the resource row is what a writer meets in practice. These witnesses
//! hold that lock from a side transaction rather than running a real act, so the hold lasts exactly
//! as long as each witness needs.
#![cfg(feature = "test-db")]

use std::time::{Duration, Instant};

use sqlx::PgPool;

use temper_core::error::TemperError;
use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{ContextId, ProfileId, ResourceId};
use temper_services::backend::write_floor::WRITE_LOCK_TIMEOUT_MS;
use temper_services::backend::DbBackend;
use temper_workflow::operations::{Backend, CreateResource, Surface, UpdateResource};
use temper_workflow::types::managed_meta::ManagedMeta;

/// Seed a substrate profile, its emitters, and a profile-owned context. Each test-target crate
/// keeps its own copy (`keyed_create_backend_test.rs`).
async fn seed_profile_with_context(pool: &PgPool, email: &str) -> (uuid::Uuid, uuid::Uuid) {
    let profile_id = uuid::Uuid::now_v7();
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
    let context_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,'locks','locks')",
    )
    .bind(context_id)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("seed context");
    (profile_id, context_id)
}

async fn seed(pool: &PgPool, email: &str) -> (DbBackend, uuid::Uuid, ResourceId) {
    let (profile, context) = seed_profile_with_context(pool, email).await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    let created = backend
        .create_resource(CreateResource {
            idempotency_key: None,
            slug: "bounded".to_string(),
            doctype: "research".to_string(),
            home: HomeAnchor::Context(ContextId::from(context)),
            title: "before".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            goal: None,
            origin_uri: Some("test://bounded".to_string()),
            chunks_packed: None,
            content_hash: None,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("create")
        .value;
    (backend, profile, created.id)
}

fn retitle(resource: ResourceId, title: &str) -> UpdateResource {
    UpdateResource {
        open_meta_add: None,
        resource,
        title: Some(title.to_string()),
        slug: None,
        body: None,
        managed_meta: None,
        open_meta: None,
        goal: None,
        move_to: None,
        context_ref: None,
        act: ActContext::default(),
        origin: Surface::ApiHttp,
    }
}

async fn title_of(pool: &PgPool, resource: ResourceId) -> String {
    sqlx::query_scalar("SELECT title FROM kb_resources WHERE id = $1")
        .bind(uuid::Uuid::from(resource))
        .fetch_one(pool)
        .await
        .expect("title")
}

/// Hold `FOR UPDATE` on the resource row, the lock the erasure act takes, for `hold`.
async fn hold_row_for_update(
    pool: &PgPool,
    resource: ResourceId,
    hold: Duration,
) -> tokio::task::JoinHandle<()> {
    let mut held = pool.begin().await.expect("begin");
    sqlx::query("SELECT 1 FROM kb_resources WHERE id = $1 FOR UPDATE")
        .bind(uuid::Uuid::from(resource))
        .execute(&mut *held)
        .await
        .expect("lock the row");
    tokio::spawn(async move {
        tokio::time::sleep(hold).await;
        held.commit().await.expect("release");
    })
}

/// **A write held past the bound is cut off.** The row is held well past the bound; the retitle
/// waits, is cancelled at the bound, answers `ResourceBusy`, and the title does not land.
///
/// FAILS IF the floor sets no `lock_timeout` (the retitle waits out the whole hold and lands), or
/// if a `55P03` maps to the generic `500` instead of `ResourceBusy`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_write_held_past_the_bound_answers_resource_busy_and_applies_nothing(pool: PgPool) {
    let (backend, _, resource) = seed(&pool, "busy@example.com").await;
    let bound = Duration::from_millis(WRITE_LOCK_TIMEOUT_MS);
    let mut held = pool.begin().await.expect("begin");
    sqlx::query("SELECT 1 FROM kb_resources WHERE id = $1 FOR UPDATE")
        .bind(uuid::Uuid::from(resource))
        .execute(&mut *held)
        .await
        .expect("lock the row");

    let asked = Instant::now();
    let answer = backend.update_resource(retitle(resource, "after")).await;
    let waited = asked.elapsed();

    assert!(
        matches!(answer, Err(TemperError::ResourceBusy)),
        "a write held past the bound answers ResourceBusy, got {answer:?}"
    );
    assert!(
        waited >= bound && waited < bound * 2,
        "the write was cut off at the bound ({bound:?}), not elsewhere: waited {waited:?}"
    );
    held.commit().await.expect("release");
    assert_eq!(
        title_of(&pool, resource).await,
        "before",
        "nothing was applied"
    );
}

/// **A legitimate slow write survives.** The row is held for well under the bound, about the time
/// the measured act takes on a resource with 20k block revisions; the retitle waits and lands.
///
/// FAILS IF the bound is set below a wait the measurement calls legitimate.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_write_waiting_less_than_the_bound_lands(pool: PgPool) {
    let (backend, _, resource) = seed(&pool, "patient@example.com").await;
    let release = hold_row_for_update(&pool, resource, Duration::from_millis(2_500)).await;

    backend
        .update_resource(retitle(resource, "after"))
        .await
        .expect("a write that waits less than the bound lands");
    release.await.unwrap();
    assert_eq!(title_of(&pool, resource).await, "after");
}

/// **The act completes under any session bound.** A writer holds the row `FOR KEY SHARE` (an
/// admitted write in flight) for longer than a tight `lock_timeout` set on the act's own session,
/// as a pool-wide or role-level default would set it. The act waits it out and erases.
///
/// FAILS IF `resource_erasure_execute` loses its `SET lock_timeout = 0` (a later
/// `CREATE OR REPLACE` that does not restate it): the act is cancelled with `55P03`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_act_completes_under_a_session_lock_timeout(pool: PgPool) {
    let (_, profile, resource) = seed(&pool, "erased@example.com").await;
    let operator: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id = $1 LIMIT 1")
            .bind(profile)
            .fetch_one(&pool)
            .await
            .expect("an entity");

    let mut writer = pool.begin().await.expect("begin");
    sqlx::query("SELECT 1 FROM kb_resources WHERE id = $1 FOR KEY SHARE")
        .bind(uuid::Uuid::from(resource))
        .execute(&mut *writer)
        .await
        .expect("the writer's floor lock");
    let release = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1_000)).await;
        writer.commit().await.expect("the writer commits");
    });

    let mut act = pool.acquire().await.expect("acquire");
    sqlx::query("SET lock_timeout = '100ms'")
        .execute(&mut *act)
        .await
        .expect("a tight session bound");
    let erased = sqlx::query("SELECT resource_erasure_execute($1, $2, $2, $3)")
        .bind(uuid::Uuid::from(resource))
        .bind(operator)
        .bind(uuid::Uuid::now_v7())
        .execute(&mut *act)
        .await;
    sqlx::query("RESET lock_timeout")
        .execute(&mut *act)
        .await
        .expect("reset");
    release.await.unwrap();

    erased.expect("the act completes whatever lock_timeout its session carries");
    let erased_at: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT erased_at FROM kb_resources WHERE id = $1")
            .bind(uuid::Uuid::from(resource))
            .fetch_one(&pool)
            .await
            .expect("erased_at");
    assert!(erased_at.is_some(), "the act erased the resource");
}

/// **A write queued behind another update is cut off too.** A side transaction holds the row
/// `FOR NO KEY UPDATE`, the lock an update takes at the head of `update_resource_in_tx`. The
/// retitle's floor (`FOR KEY SHARE`) does not conflict with it and is admitted; the update's own
/// head lock then waits, past the bound, inside the write itself rather than the floor.
///
/// FAILS IF the bound ends with the floor's statement, or if a `55P03` raised inside the write
/// (bridged by `tx_err`, not by the floor) maps to the generic `500`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_write_queued_behind_another_update_past_the_bound_answers_resource_busy(pool: PgPool) {
    let (backend, _, resource) = seed(&pool, "queued@example.com").await;
    let bound = Duration::from_millis(WRITE_LOCK_TIMEOUT_MS);
    let mut other_update = pool.begin().await.expect("begin");
    sqlx::query("SELECT 1 FROM kb_resources WHERE id = $1 FOR NO KEY UPDATE")
        .bind(uuid::Uuid::from(resource))
        .execute(&mut *other_update)
        .await
        .expect("another update holds the row");

    let asked = Instant::now();
    let answer = backend.update_resource(retitle(resource, "after")).await;
    let waited = asked.elapsed();

    assert!(
        matches!(answer, Err(TemperError::ResourceBusy)),
        "a write queued past the bound answers ResourceBusy, got {answer:?}"
    );
    assert!(
        waited >= bound && waited < bound * 2,
        "cut off at the bound ({bound:?}): waited {waited:?}"
    );
    other_update.commit().await.expect("release");
    assert_eq!(
        title_of(&pool, resource).await,
        "before",
        "nothing was applied"
    );
}

/// **Overlapping writers cannot hold the act off.** A chain of floored writers on R, each holding
/// its floor for 300 ms and the next starting 150 ms after the previous, so some writer always
/// holds R's row from start to finish. The act arrives just after the first. A writer that arrives
/// while the act waits must queue behind it, so the act completes once the writers already in have
/// committed, not once the whole chain ends. Every later writer then reads R erased.
///
/// FAILS IF a writer's floor lock jumps the queue ahead of the act's waiting `FOR UPDATE` (a
/// `FOR KEY SHARE` that conflicts with no current holder is granted without queueing): the act
/// then waits out the whole chain.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn overlapping_writers_cannot_hold_the_act_off(pool: PgPool) {
    use temper_services::backend::write_floor::modify_floor_in_tx;

    let (_, profile, resource) = seed(&pool, "starved@example.com").await;
    let operator: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id = $1 LIMIT 1")
            .bind(profile)
            .fetch_one(&pool)
            .await
            .expect("an entity");
    const WRITERS: u32 = 14;
    let hold = Duration::from_millis(300);
    let stagger = Duration::from_millis(150);
    let chain_ends = stagger * (WRITERS - 1) + hold;

    let started = Instant::now();
    let mut writers = Vec::new();
    for n in 0..WRITERS {
        let pool = pool.clone();
        writers.push(tokio::spawn(async move {
            tokio::time::sleep(stagger * n).await;
            let mut tx = pool.begin().await.expect("begin");
            let floored = modify_floor_in_tx(&mut tx, ProfileId::from(profile), resource).await;
            if floored.is_ok() {
                tokio::time::sleep(hold).await;
            }
            tx.rollback().await.expect("end the writer");
            floored
        }));
    }

    tokio::time::sleep(Duration::from_millis(50)).await;
    sqlx::query("SELECT resource_erasure_execute($1, $2, $2, $3)")
        .bind(uuid::Uuid::from(resource))
        .bind(operator)
        .bind(uuid::Uuid::now_v7())
        .execute(&pool)
        .await
        .expect("the act completes");
    let act_done = started.elapsed();
    // The first writer arrived before the act, so the floor must have admitted it and it must
    // have held R. A floor that failed for any other reason (a missing function, say) would let
    // the act through at once and make this witness pass for the wrong reason.
    let mut outcomes = Vec::new();
    for w in writers {
        outcomes.push(w.await.expect("writer"));
    }
    assert!(
        outcomes[0].is_ok(),
        "the writer ahead of the act was admitted: {:?}",
        outcomes[0]
    );
    for later in &outcomes[1..] {
        assert!(
            matches!(
                later,
                Ok(()) | Err(TemperError::ResourceErased(_)) | Err(TemperError::Forbidden)
            ),
            "a later writer is admitted before the act or reads R erased after it: {later:?}"
        );
    }

    assert!(
        act_done < chain_ends / 2,
        "the act completed at {act_done:?}, behind writers that arrived after it; the chain ran \
         to {chain_ends:?}"
    );
}

/// **A bound hit outside the floor's own lock still answers `ResourceBusy`.** Another goal patch
/// holds this resource's goal-patch advisory lock past the bound. A goal clear waits on it inside
/// its floored transaction, is cancelled, and answers `ResourceBusy`, never the generic `500`.
///
/// FAILS IF a door maps that statement's error with a bridge that flattens a `55P03` to
/// `TemperError::Api` (it did: `lock_goal_rows` used `api_err`, which did not classify). Found by
/// the code review of this branch.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_goal_patch_waiting_past_the_bound_answers_resource_busy(pool: PgPool) {
    use temper_workflow::operations::GoalPatch;

    let (backend, _, resource) = seed(&pool, "goalbusy@example.com").await;
    let bound = Duration::from_millis(WRITE_LOCK_TIMEOUT_MS);
    let mut other_patch = pool.begin().await.expect("begin");
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 0))")
        .bind(format!("goal_patch:{}", uuid::Uuid::from(resource)))
        .execute(&mut *other_patch)
        .await
        .expect("another goal patch holds the lock");

    let mut clear = retitle(resource, "after");
    clear.title = None;
    clear.goal = Some(GoalPatch::Clear);
    let asked = Instant::now();
    let answer = backend.update_resource(clear).await;
    let waited = asked.elapsed();
    other_patch.rollback().await.expect("release");

    assert!(
        matches!(answer, Err(TemperError::ResourceBusy)),
        "a goal patch held past the bound answers ResourceBusy, got {answer:?}"
    );
    assert!(waited >= bound, "it waited out the bound: {waited:?}");
}

/// **A bulk run's floor takes no act-queue entry per resource.** Flooring three resources in one
/// transaction through the bulk liveness floor leaves this backend holding no advisory lock; the
/// ordinary liveness floor holds one per resource. The queue lives in Postgres's shared lock table,
/// and a team reassignment floors every resource it moves in one transaction.
///
/// FAILS IF `liveness_floor_bulk_in_tx` joins the act queue.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_bulk_floor_holds_no_queue_entry_per_resource(pool: PgPool) {
    use temper_services::backend::write_floor::{liveness_floor_bulk_in_tx, liveness_floor_in_tx};

    let (backend, profile, first) = seed(&pool, "bulk@example.com").await;
    let mut resources = vec![first];
    for n in 0..2 {
        let created = backend
            .create_resource(CreateResource {
                idempotency_key: None,
                slug: format!("bulk-{n}"),
                doctype: "research".to_string(),
                home: HomeAnchor::Context(ContextId::from(
                    sqlx::query_scalar::<_, uuid::Uuid>(
                        "SELECT id FROM kb_contexts WHERE owner_id = $1",
                    )
                    .bind(profile)
                    .fetch_one(&pool)
                    .await
                    .expect("context"),
                )),
                title: format!("bulk {n}"),
                body: None,
                managed_meta: ManagedMeta::default(),
                open_meta: None,
                goal: None,
                origin_uri: Some(format!("test://bulk/{n}")),
                chunks_packed: None,
                content_hash: None,
                act: ActContext::default(),
                origin: Surface::ApiHttp,
            })
            .await
            .expect("create")
            .value;
        resources.push(created.id);
    }

    async fn advisory_held(tx: &mut sqlx::PgConnection) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM pg_locks WHERE pid = pg_backend_pid() AND locktype = 'advisory'",
        )
        .fetch_one(&mut *tx)
        .await
        .expect("count")
    }

    let mut bulk = pool.begin().await.expect("begin");
    for &r in &resources {
        liveness_floor_bulk_in_tx(&mut bulk, ProfileId::from(profile), r)
            .await
            .expect("bulk floor admits");
    }
    assert_eq!(
        advisory_held(&mut bulk).await,
        0,
        "the bulk floor joins no queue"
    );
    bulk.rollback().await.expect("end");

    let mut single = pool.begin().await.expect("begin");
    for &r in &resources {
        liveness_floor_in_tx(&mut single, ProfileId::from(profile), r)
            .await
            .expect("floor admits");
    }
    assert_eq!(
        advisory_held(&mut single).await,
        3,
        "the ordinary floor joins each resource's queue"
    );
    single.rollback().await.expect("end");
}

/// **A create whose goal is busy leaves nothing behind.** The goal's row is held `FOR UPDATE` (an
/// erasure act, a scrub or a delete) past the bound. A create linking to it answers
/// `ResourceBusy`, and no resource was committed, so re-sending the request (which temperkb-client
/// does for `RESOURCE_BUSY`) cannot mint a second one.
///
/// FAILS IF the create commits before its goal edge is asserted (it did: the edge ran in a second
/// transaction, so the `503` came back over a committed resource). Found by both reviews of this
/// branch.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_create_whose_goal_is_busy_answers_resource_busy_and_creates_nothing(pool: PgPool) {
    let (backend, profile, _) = seed(&pool, "creategoal@example.com").await;
    let context: uuid::Uuid = sqlx::query_scalar("SELECT id FROM kb_contexts WHERE owner_id = $1")
        .bind(profile)
        .fetch_one(&pool)
        .await
        .expect("context");
    let create = |slug: &str, doctype: &str, goal: Option<ResourceId>| CreateResource {
        idempotency_key: None,
        slug: slug.to_string(),
        doctype: doctype.to_string(),
        home: HomeAnchor::Context(ContextId::from(context)),
        title: slug.to_string(),
        body: None,
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        goal,
        origin_uri: Some(format!("test://{slug}")),
        chunks_packed: None,
        content_hash: None,
        act: ActContext::default(),
        origin: Surface::ApiHttp,
    };
    let goal = backend
        .create_resource(create("the-goal", "goal", None))
        .await
        .expect("create the goal")
        .value
        .id;
    let count = || async {
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM kb_resources WHERE origin_uri = $1")
            .bind("test://linked")
            .fetch_one(&pool)
            .await
            .expect("count")
    };

    let mut held = pool.begin().await.expect("begin");
    sqlx::query("SELECT 1 FROM kb_resources WHERE id = $1 FOR UPDATE")
        .bind(uuid::Uuid::from(goal))
        .execute(&mut *held)
        .await
        .expect("hold the goal");
    let answer = backend
        .create_resource(create("linked", "research", Some(goal)))
        .await;
    held.rollback().await.expect("release");

    assert!(
        matches!(answer, Err(TemperError::ResourceBusy)),
        "a create whose goal is held past the bound answers ResourceBusy, got {:?}",
        answer.map(|o| o.value.id)
    );
    assert_eq!(count().await, 0, "nothing was committed");

    // Released, the same create lands, goal edge and all.
    backend
        .create_resource(create("linked", "research", Some(goal)))
        .await
        .expect("the create lands once the goal is free");
    assert_eq!(count().await, 1);
}

/// **The block history scrub completes under any session bound, and writers queue behind it.** The
/// scrub is the erasure act at block grain and takes the same two guarantees: it pins
/// `lock_timeout = 0`, and it takes R's act queue exclusive. A writer holds R `FOR KEY SHARE` past
/// a tight bound on the scrub's own session; the scrub waits it out and completes. Meanwhile a
/// second writer arriving after the scrub must queue behind it.
///
/// FAILS IF `block_history_scrub_execute` loses its `SET lock_timeout = 0` (cancelled with
/// `55P03`) or its queue line (the second writer's floor is granted while the scrub waits).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_scrub_completes_under_a_session_lock_timeout_and_queues_writers(pool: PgPool) {
    use temper_services::backend::write_floor::modify_floor_in_tx;

    let (_, profile, resource) = seed(&pool, "scrubbed@example.com").await;
    let operator: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id = $1 LIMIT 1")
            .bind(profile)
            .fetch_one(&pool)
            .await
            .expect("an entity");
    let blocks: Vec<uuid::Uuid> =
        sqlx::query_scalar("SELECT id FROM kb_content_blocks WHERE resource_id = $1")
            .bind(uuid::Uuid::from(resource))
            .fetch_all(&pool)
            .await
            .expect("blocks");

    let mut first = pool.begin().await.expect("begin");
    modify_floor_in_tx(&mut first, ProfileId::from(profile), resource)
        .await
        .expect("the first writer is admitted");

    let scrub_pool = pool.clone();
    let scrub = tokio::spawn(async move {
        let mut conn = scrub_pool.acquire().await.expect("acquire");
        sqlx::query("SET lock_timeout = '100ms'")
            .execute(&mut *conn)
            .await
            .expect("a tight session bound");
        let done = sqlx::query("SELECT block_history_scrub_execute($1, $2, $3, $3, $4)")
            .bind(uuid::Uuid::from(resource))
            .bind(&blocks)
            .bind(operator)
            .bind(uuid::Uuid::now_v7())
            .execute(&mut *conn)
            .await
            .map(|_| ())
            .map_err(|e| e.to_string());
        let _ = sqlx::query("RESET lock_timeout").execute(&mut *conn).await;
        done
    });
    tokio::time::sleep(Duration::from_millis(300)).await;

    // A second writer arrives while the scrub waits: it must queue, not join the first.
    let second_pool = pool.clone();
    let second = tokio::spawn(async move {
        let mut tx = second_pool.begin().await.expect("begin");
        let asked = Instant::now();
        let floored = modify_floor_in_tx(&mut tx, ProfileId::from(profile), resource).await;
        let waited = asked.elapsed();
        tx.rollback().await.expect("end");
        (floored, waited)
    });
    tokio::time::sleep(Duration::from_millis(700)).await;
    first.rollback().await.expect("the first writer ends");

    scrub
        .await
        .expect("scrub task")
        .expect("the scrub completes whatever lock_timeout its session carries");
    let (floored, waited) = second.await.expect("second writer");
    assert!(
        floored.is_ok(),
        "the second writer is admitted after the scrub: {floored:?}"
    );
    assert!(
        waited >= Duration::from_millis(600),
        "the second writer queued behind the waiting scrub rather than joining the first writer: \
         waited {waited:?}"
    );
}
