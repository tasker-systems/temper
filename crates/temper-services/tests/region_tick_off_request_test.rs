//! Integration test — the region clocks left the request path (goal 019fc46c, clause
//! `a-write-returns-without-waiting-on-projection`).
//!
//! **What makes this a witness and not a shape assertion.** Asserting that a job row appears would
//! pass just as well if the inline tick were *also* still running — it would observe the new
//! mechanism without observing the removal of the old one. So the load-bearing assertion here is a
//! NEGATIVE one: after a create that crosses the formation threshold, **no `region_materialized`
//! event exists yet**. Against the pre-change code that create settles inline and the event is
//! there when `create_resource` returns, so this test fails on `main` for the right reason — the
//! write did the projection work.
//!
//! The positive half (the drain then settles it) is asserted separately, because the two can fail
//! independently: a write could stop settling while nothing ever picks the work up, which is a
//! regression wearing this test's green.
#![cfg(feature = "test-db")]

use sqlx::PgPool;

use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{CogmapId, ContextId, ProfileId};
use temper_core::types::ingest::{pack_chunks, PackedChunk};
use temper_services::backend::DbBackend;
use temper_services::services::region_service;
use temper_workflow::operations::{Backend, BodyUpdate, CreateResource, Surface};
use temper_workflow::types::managed_meta::ManagedMeta;

/// Seed a profile plus its three surface emitter entities — the write path resolves
/// `<handle>@<surface>` through `resolve_profile` + `resolve_emitter`.
async fn seed_profile(pool: &PgPool, email: &str) -> uuid::Uuid {
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
    profile_id
}

async fn seed_profile_with_context(pool: &PgPool, email: &str) -> (uuid::Uuid, uuid::Uuid) {
    let profile_id = seed_profile(pool, email).await;
    let context_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,'temper','temper')",
    )
    .bind(context_id)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("seed context");
    (profile_id, context_id)
}

/// The `system` profile's emitter entity — `cogmap_genesis` births a map under the system actor.
async fn system_emitter(pool: &PgPool) -> uuid::Uuid {
    sqlx::query_scalar(
        "SELECT e.id FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id \
          WHERE p.handle = 'system' AND e.name = 'system'",
    )
    .fetch_one(pool)
    .await
    .expect("system emitter")
}

/// Birth a cognitive map owned by `owner` through `cogmap_genesis` — the same helper
/// `cogmap_homed_edge_assert_test` uses.
///
/// A raw `INSERT INTO kb_cogmaps` does NOT work: the table has no owner columns at all (ownership
/// lives in grants), and `telos_resource_id` is `NOT NULL` with an FK, so genesis is what makes the
/// map both well-formed and authorable by `owner`.
async fn birth_cogmap(pool: &PgPool, owner: uuid::Uuid, name: &str) -> uuid::Uuid {
    let cogmap = uuid::Uuid::now_v7();
    let telos = uuid::Uuid::now_v7();
    let emitter = system_emitter(pool).await;
    sqlx::query("SELECT cogmap_genesis($1, $2, $3)")
        .bind(serde_json::json!({
            "cogmap_id": cogmap,
            "name": name,
            "owner_profile_id": owner,
            "telos": {
                "resource_id": telos,
                "title": format!("{name} telos"),
                "origin_uri": format!("temper://test/{name}/telos"),
                "blocks": [],
            },
        }))
        .bind(serde_json::json!({}))
        .bind(emitter)
        .execute(pool)
        .await
        .expect("birth cogmap");
    // Genesis births the map; it does NOT make it authorable. `cogmap_authorable_by_profile` is
    // `profile_explicit_grant(profile, 'write', 'kb_cogmaps', cogmap)` — an explicit
    // `kb_access_grants` row, either to the profile or to a team it reaches. Without this the
    // create returns `Forbidden` at `check_cogmap_authorable`, long before any clock is queued.
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (subject_table, subject_id, principal_table, principal_id, \
            can_read, can_write, granted_by_profile_id) \
         VALUES ('kb_cogmaps', $1, 'kb_profiles', $2, true, true, $2)",
    )
    .bind(cogmap)
    .bind(owner)
    .execute(pool)
    .await
    .expect("grant write on cogmap");
    cogmap
}

fn one_chunk_packed(text: &str, hash_seed: &str) -> String {
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: text.to_owned(),
        content_hash: format!("{hash_seed:0>64}"),
        embedding: vec![0.1_f32; 768],
        embedded_with: None,
    };
    pack_chunks(&[chunk]).expect("pack chunk")
}

fn create_cmd(home: HomeAnchor, slug: &str, hash_seed: &str) -> CreateResource {
    let content = format!("body of {slug}");
    CreateResource {
        idempotency_key: None,
        slug: slug.to_string(),
        doctype: "research".to_string(),
        home,
        title: slug.to_string(),
        body: Some(BodyUpdate {
            content: content.clone(),
            content_hash: None,
            chunks_packed: None,
            sources: Vec::new(),
            content_block: None,
        }),
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        goal: None,
        origin_uri: Some(format!("test://{slug}")),
        chunks_packed: Some(one_chunk_packed(&content, hash_seed)),
        content_hash: None,
        act: ActContext::default(),
        origin: Surface::ApiHttp,
    }
}

/// Push the anchor to one formation event below the threshold, so the NEXT real create crosses it.
/// Synthetic `resource_created` rows against the anchor — the same shape `region_clocks`' own unit
/// tests use, and exactly what `formation_touched_count_since` counts.
async fn arm_formation_clock(pool: &PgPool, profile: uuid::Uuid, anchor: HomeAnchor, n: i32) {
    let entity: uuid::Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id = $1 LIMIT 1")
            .bind(profile)
            .fetch_one(pool)
            .await
            .expect("emitter entity");
    for _ in 0..n {
        sqlx::query(
            "INSERT INTO kb_events (event_type_id, emitter_entity_id, producing_anchor_table, producing_anchor_id) \
             VALUES ((SELECT id FROM kb_event_types WHERE name = 'resource_created'), $1, $2, $3)",
        )
        .bind(entity)
        .bind(anchor.table())
        .bind(anchor.uuid())
        .execute(pool)
        .await
        .expect("seed formation event");
    }
}

async fn materialized_count(pool: &PgPool, anchor: HomeAnchor) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
          WHERE et.name = 'region_materialized' \
            AND e.producing_anchor_table = $1 AND e.producing_anchor_id = $2",
    )
    .bind(anchor.table())
    .bind(anchor.uuid())
    .fetch_one(pool)
    .await
    .expect("count region_materialized")
}

async fn queued_jobs(pool: &PgPool, anchor: HomeAnchor) -> i64 {
    let (col, id) = match anchor {
        HomeAnchor::Cogmap(m) => ("cogmap_id", m.uuid()),
        HomeAnchor::Context(c) => ("context_id", c.uuid()),
    };
    sqlx::query_scalar(&format!(
        "SELECT count(*) FROM kb_workflow_jobs \
          WHERE {col} = $1 AND persona = 'region' AND dispatch_type = 'materialize' \
            AND status IN ('pending','in_progress','waiting_for_retry')"
    ))
    .bind(id)
    .fetch_one(pool)
    .await
    .expect("count queued region jobs")
}

/// **THE WITNESS.** A create that crosses the formation threshold must not settle during the write.
///
/// Fails against the pre-change code: the inline `tick_region_clocks` runs `incremental_materialize`
/// before `create_resource` returns, so `region_materialized` is already on the ledger here.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_create_that_crosses_the_threshold_does_not_settle_during_the_write(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "witness@example.com").await;
    let anchor = HomeAnchor::Context(ContextId::from(context));
    // 4 seeded + the create's own `resource_created` = 5 = DEFAULT_MATERIALIZE_THRESHOLD.
    arm_formation_clock(&pool, profile, anchor, 4).await;

    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    backend
        .create_resource(create_cmd(anchor, "crosses-threshold", "a1"))
        .await
        .expect("create");

    assert_eq!(
        materialized_count(&pool, anchor).await,
        0,
        "the write must not have settled the anchor — a `region_materialized` event on the ledger \
         when `create_resource` returned means the projection ran inside the request"
    );
    assert_eq!(
        queued_jobs(&pool, anchor).await,
        1,
        "the settling must instead be queued: exactly one region job for this anchor"
    );
}

/// The positive half — asserted separately, because a write that stops settling while NOTHING picks
/// the work up would pass the witness above and still be a regression.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_drain_settles_what_the_write_queued(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "drain@example.com").await;
    let anchor = HomeAnchor::Context(ContextId::from(context));
    arm_formation_clock(&pool, profile, anchor, 4).await;

    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    backend
        .create_resource(create_cmd(anchor, "drained", "b1"))
        .await
        .expect("create");
    assert_eq!(materialized_count(&pool, anchor).await, 0, "precondition");

    let summary = region_service::dispatch_tick(&pool, None)
        .await
        .expect("drain");

    assert_eq!(summary.claimed, 1, "the drain must claim the queued job");
    assert_eq!(summary.completed, 1, "and complete it");
    assert_eq!(
        summary.materialized, 1,
        "the formation clock was over threshold, so this pass must have materialized"
    );
    assert_eq!(
        materialized_count(&pool, anchor).await,
        1,
        "the settling the write declined to do must now be on the ledger"
    );
    assert_eq!(
        queued_jobs(&pool, anchor).await,
        0,
        "and the job must be completed, not left in flight"
    );
}

/// **Both anchor kinds.** Two of the four anchors carrying production materialization load are
/// CONTEXTS, and `kb_workflow_jobs` could not key one before this change — so a detachment that
/// silently covered only cogmaps would have left half the load on the request path.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_cogmap_homed_write_queues_the_same_way_a_context_homed_one_does(pool: PgPool) {
    let profile = seed_profile(&pool, "cogmap@example.com").await;
    let cogmap = birth_cogmap(&pool, profile, "region-drain-map").await;
    let anchor = HomeAnchor::Cogmap(CogmapId::from(cogmap));
    arm_formation_clock(&pool, profile, anchor, 4).await;

    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    backend
        .create_resource(create_cmd(anchor, "cogmap-homed", "c1"))
        .await
        .expect("create");

    assert_eq!(
        materialized_count(&pool, anchor).await,
        0,
        "a cogmap-homed write must not settle inline either"
    );
    assert_eq!(
        queued_jobs(&pool, anchor).await,
        1,
        "and must queue against the cogmap scope"
    );
}

/// Single-flight: N settling-worthy arrivals on one anchor collapse to ONE job. This is the
/// mechanism behind the goal's `concurrent-arrivals-do-not-multiply-work` clause — asserted here as
/// an observed property of the incumbent queue, NOT claimed as that clause being covered (the clause
/// is about cost proportionality under real concurrency, which this does not measure).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn concurrent_arrivals_on_one_anchor_collapse_to_a_single_job(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "coalesce@example.com").await;
    let anchor = HomeAnchor::Context(ContextId::from(context));
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    for (i, seed) in ["d1", "d2", "d3"].iter().enumerate() {
        backend
            .create_resource(create_cmd(anchor, &format!("arrival-{i}"), seed))
            .await
            .expect("create");
    }

    assert_eq!(
        queued_jobs(&pool, anchor).await,
        1,
        "three arrivals on one anchor must leave ONE queued job, not three"
    );
}
