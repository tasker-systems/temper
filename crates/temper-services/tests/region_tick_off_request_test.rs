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
use temper_workflow::operations::{Backend, BodyUpdate, CreateResource, DeleteResource, Surface};
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

// ── the delete window ────────────────────────────────────────────────────────────────────────
//
// A soft delete invalidates a STORED AGGREGATE, not just the ledger: the deleted member's chunk
// vectors stay inside every region centroid it contributed to until a materialize re-derives them.
// The count threshold cannot be relied on to carry that — an anchor that goes quiet after one
// delete accumulates no further events, so under a count-only gate the wrong centroid persists
// indefinitely (the same stability that made the prod ghost regions a finding). So the tick's gate
// treats a delete as its own pressure, and the delete joins the enqueue posture of every other
// write: settled by the next drain, not inline in the request.

/// A unit 768-dim vector along axis `d`.
fn axis(d: usize) -> Vec<f32> {
    let mut e = vec![0.0_f32; 768];
    e[d] = 1.0;
    e
}

/// The `[...]` text literal a `::vector` bind takes.
fn vec_text(v: &[f32]) -> String {
    let parts: Vec<String> = v.iter().map(|f| format!("{f}")).collect();
    format!("[{}]", parts.join(","))
}

/// One-chunk create whose chunk carries a CALLER-CHOSEN embedding — the region this fixture forms
/// is embedding-clustered (`workflow-default` holds `w_cos = 1.0`), so the embeddings are the
/// membership and the centroid is computable in the test, exactly.
fn create_cmd_embedded(home: HomeAnchor, slug: &str, embedding: Vec<f32>) -> CreateResource {
    let content = format!("body of {slug}");
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: content.clone(),
        content_hash: {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            slug.hash(&mut h);
            format!("{:0>64}", h.finish())
        },
        embedding,
        embedded_with: None,
    };
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
        chunks_packed: Some(pack_chunks(&[chunk]).expect("pack chunk")),
        content_hash: None,
        act: ActContext::default(),
        origin: Surface::ApiHttp,
    }
}

/// Cosine similarity in f64 — what `<=>` computes 1-minus.
fn cos(a: &[f64], b: &[f64]) -> f64 {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f64 = a.iter().map(|x| x.powi(2)).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| x.powi(2)).sum::<f64>().sqrt();
    dot / (na * nb)
}

fn f64ify(v: &[f32]) -> Vec<f64> {
    v.iter().map(|x| *x as f64).collect()
}

async fn live_region_centroid(pool: &PgPool, context: uuid::Uuid) -> Vec<f64> {
    let raw: String = sqlx::query_scalar(
        "SELECT r.centroid::text FROM kb_cogmap_regions r \
         WHERE r.home_anchor_table = 'kb_contexts' AND r.home_anchor_id = $1 \
           AND NOT r.is_folded",
    )
    .bind(context)
    .fetch_one(pool)
    .await
    .expect("one live region");
    // pgvector text: `[0.1,0.2,...]`
    raw.trim_matches(|c| c == '[' || c == ']')
        .split(',')
        .map(|p| p.trim().parse::<f64>().expect("centroid component"))
        .collect()
}

/// **THE WITNESS.** A soft delete settles at the next drain, and the next drain alone — no
/// threshold of unrelated writes, and no settling inside the request.
///
/// Five embedding-clustered resources form one region; the stored centroid is then computable to
/// the test exactly. Deleting one member and draining must leave the centroid equal to the
/// LIVE-ONLY mean, not the stale all-members mean — and the differential probe (two chosen
/// vectors, difference the region scores) must resolve the live centroid's direction, which is
/// everything the caller can already compute, rather than the stale one's, which is not.
///
/// Fails against the pre-change code in two independent ways: with the delete enqueueing nothing,
/// the drain has no job to claim; with a count-only gate, the drain claims the job and declines to
/// materialize (one event < threshold 5) — the centroid stays stale under both.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_delete_settles_the_anchor_at_the_next_drain_without_further_writes(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "delete-window@example.com").await;
    let anchor = HomeAnchor::Context(ContextId::from(context));

    // Four members on axis 0, one at cosine 0.9 off it — all pairwise above any plausible
    // resolution, so the fixture forms ONE region whose membership is exactly these five.
    let mut b = axis(0);
    let off = (1.0_f32 - 0.9 * 0.9).sqrt();
    b[0] = 0.9;
    b[1] = off;

    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    for i in 0..4 {
        backend
            .create_resource(create_cmd_embedded(
                anchor,
                &format!("live-member-{i}"),
                axis(0),
            ))
            .await
            .expect("create live member");
    }
    let deleted_id = backend
        .create_resource(create_cmd_embedded(anchor, "deleted-member", b.clone()))
        .await
        .expect("create deleted member")
        .value
        .id;

    // First drain: five structural events, watermark NULL — the count gate crosses and the region
    // forms. Precondition, not subject: asserted so a fixture that failed to form fails HERE.
    let summary = region_service::dispatch_tick(&pool, None)
        .await
        .expect("drain 1");
    assert_eq!(summary.claimed, 1, "the creates queued exactly one settle");
    assert_eq!(
        summary.materialized, 1,
        "count 5 >= threshold: it materialized"
    );

    // The region exists with all five members, and the centroid is the all-members mean.
    let member_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_cogmap_region_members m \
         JOIN kb_cogmap_regions r ON r.id = m.region_id \
         WHERE r.home_anchor_table = 'kb_contexts' AND r.home_anchor_id = $1 AND NOT r.is_folded",
    )
    .bind(context)
    .fetch_one(&pool)
    .await
    .expect("member count");
    assert_eq!(
        member_count, 5,
        "fixture precondition: one region, five members"
    );

    let centroid = live_region_centroid(&pool, context).await;
    let e0 = axis(0);
    let all_mean: Vec<f32> = (0..768).map(|d| (4.0 * e0[d] + b[d]) / 5.0).collect();
    let cos_all = cos(&centroid, &f64ify(&all_mean));
    assert!(
        cos_all > 0.999_999,
        "precondition: the stored centroid is the all-members mean (cosine {cos_all})"
    );

    // THE DELETE. Same posture as every write: enqueue, return, settle later.
    backend
        .delete_resource(DeleteResource {
            resource: deleted_id,
            force: true,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("delete");

    assert_eq!(
        queued_jobs(&pool, anchor).await,
        1,
        "the delete must queue the settle — a write returns without waiting on projection"
    );
    assert_eq!(
        materialized_count(&pool, anchor).await,
        1,
        "and must not have settled inline"
    );

    // THE NEXT DRAIN ALONE closes the window: no further resource events exist to cross a count.
    let summary = region_service::dispatch_tick(&pool, None)
        .await
        .expect("drain 2");
    assert_eq!(summary.claimed, 1, "the delete queued a job for this pass");
    assert_eq!(
        summary.materialized, 1,
        "a delete is its own pressure: one event < threshold 5 must still materialize"
    );

    // The stored centroid is now the LIVE-ONLY mean — e0, four identical members — and not the
    // stale all-members mean (cosine 0.9961... — near, but measurably not 1).
    let centroid = live_region_centroid(&pool, context).await;
    let cos_live = cos(&centroid, &f64ify(&e0));
    let cos_stale = cos(&centroid, &f64ify(&all_mean));
    assert!(
        (cos_live - 1.0).abs() < 1e-9,
        "the centroid must be the live-only mean after the drain (cosine {cos_live}); \
         cosine {cos_stale} against the all-members mean means the dead member is still inside it"
    );
    let member_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_cogmap_region_members m \
         JOIN kb_cogmap_regions r ON r.id = m.region_id \
         WHERE r.home_anchor_table = 'kb_contexts' AND r.home_anchor_id = $1 AND NOT r.is_folded",
    )
    .bind(context)
    .fetch_one(&pool)
    .await
    .expect("member count after");
    assert_eq!(
        member_count, 4,
        "the dead member is out of the membership too"
    );

    // THE DIFFERENTIAL PROBE. Two chosen vectors, difference the scores: with sal_norm and prior
    // constant for a fixed candidate pool, the difference is 0.6 × the query-cosine difference, so
    // the probes recover the centroid's projection onto the two vectors' span. Post-settle that
    // projection is the live centroid's — everything the caller can already compute from visible
    // members — and NOT the stale centroid's, whose off-axis weight came from a member the caller
    // can no longer read. Values are computed from the fixture, not hand-baked.
    let qcos_axis0 = {
        let emb_text = vec_text(&axis(0));
        sqlx::query_scalar::<_, f64>(
            "SELECT query_cos::float8 FROM wayfind_region_scores(\
               $1, NULL::uuid, $2::vector, 20, 'kb_contexts', $3)",
        )
        .bind(profile)
        .bind(&emb_text)
        .bind(context)
        .fetch_one(&pool)
        .await
        .expect("probe axis0")
    };
    let qcos_axis1 = {
        let emb_text = vec_text(&axis(1));
        sqlx::query_scalar::<_, f64>(
            "SELECT query_cos::float8 FROM wayfind_region_scores(\
               $1, NULL::uuid, $2::vector, 20, 'kb_contexts', $3)",
        )
        .bind(profile)
        .bind(&emb_text)
        .bind(context)
        .fetch_one(&pool)
        .await
        .expect("probe axis1")
    };
    let delta = qcos_axis0 - qcos_axis1;
    // Live centroid ∝ axis 0 → Δquery_cos = 1 − 0 = 1. Stale centroid → Δ = cos(c,e0) − cos(c,e1)
    // ≈ 0.9075. The probe must resolve the LIVE direction.
    assert!(
        (delta - 1.0).abs() < 1e-3,
        "the differenced probes must resolve the live-only centroid direction (Δ = {delta}); \
         a Δ near {} means the probes still resolve the stale centroid, dead member included",
        cos(&f64ify(&all_mean), &f64ify(&axis(0))) - cos(&f64ify(&all_mean), &f64ify(&axis(1)))
    );
}
