#![cfg(feature = "test-db")]
//! Witnesses for the resource-erasure act's DERIVED VECTORS (0.6.0 line item 5; task
//! 01a10161-961f-7062-96a7-935df2c625a4, `## Rulings (2026-10-04)`, ruling 1 amended the same day).
//!
//! The act empties R's own content, but three things DERIVED from R's embeddings outlive it on
//! main: region centroids (folded rows, and the live row the act leaves frozen), a context's telos
//! snapshot (`kb_contexts.telos_centroid`), and the ledger copies of that snapshot
//! (`region_materialized` / `salience_refreshed` payloads). These witnesses plant R where each of
//! those derives from it FOR REAL, then run the real act.
//!
//! **The trap these honour.** A witness that erases R as a region's SOLE member, or erases it
//! without materializing first, passes vacuously: there is nothing derived to leave behind. So
//! R is planted as one of TWO goals in a context's telos, into a live region WITH survivors, and
//! the context is materialized through the real producer before the act. Every assertion reads a
//! STORED VECTOR (a centroid, the telos snapshot) — never the watermark, which main already nulls.
//! Each vector assertion carries a vacuity guard: the expected value must differ from the
//! pre-act value, or the assertion proves nothing.
//!
//! Expected values are computed independently, in SQL, the way the producer defines them
//! (`populate_readouts`' centroid statement: avg per member over current chunks of unfolded
//! blocks, then avg of members) — never by calling the code under test.

use std::collections::BTreeSet;

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::home::HomeAnchor;
use temper_core::types::materialize::default_lens_for;
use temper_services::services::region_service;
use temper_services::services::resource_erasure_service::{
    execute_resource_erasure, ResourceErasureCompletion, ResourceErasureOutcome,
    ResourceErasureRequest,
};
use temper_services::test_support;
use temper_substrate::affinity::EdgeKind;
use temper_substrate::content::IncomingChunk;
use temper_substrate::events::{fire, EdgeHome, SeedAction};
use temper_substrate::ids::{ContextId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{AnchorRef, EdgePolarity};
use temper_substrate::writes::{self, CreateParams};
use temper_substrate::{substrate, write};
use temper_workflow::operations::Surface;

/// Distances below this are "the same vector" (pgvector stores float4).
const SAME: f64 = 1e-5;
/// A vacuity guard: the expected value must be at least this far from the pre-act value.
const MOVED: f64 = 1e-3;

// ── fixture helpers (duplicated per file, per this suite's convention) ──────────────────────

/// A profile with its `<handle>@web` emitter entity. The handle is the FULL id: two UUIDv7s
/// minted in one millisecond share leading bytes, so a truncated handle collides.
async fn principal(pool: &PgPool) -> (ProfileId, EntityId) {
    let id = Uuid::now_v7();
    let handle = format!("user-{id}");
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $2)")
        .bind(id)
        .bind(&handle)
        .execute(pool)
        .await
        .expect("seed profile");
    let emitter: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2) RETURNING id",
    )
    .bind(id)
    .bind(format!("{handle}@web"))
    .fetch_one(pool)
    .await
    .expect("seed emitter entity");
    (ProfileId::from(id), EntityId::from(emitter))
}

async fn context(pool: &PgPool, owner: ProfileId, slug: &str) -> ContextId {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, $2, $2) RETURNING id",
    )
    .bind(owner.uuid())
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("seed context");
    ContextId::from(id)
}

/// A unit 768-dim vector along axis `d`.
fn axis(d: usize) -> Vec<f32> {
    let mut e = vec![0.0_f32; 768];
    e[d] = 1.0;
    e
}

/// A unit vector at cosine 0.95 to axis 0, leaning toward axis `d` — close enough to axis 0 that
/// every planted member lands in one embedding-clustered region, far enough that removing an
/// axis-0 member measurably moves a mean.
fn near_axis0(d: usize) -> Vec<f32> {
    let mut v = axis(0);
    v[0] = 0.95;
    v[d] = (1.0_f32 - 0.95 * 0.95).sqrt();
    v
}

/// The `[...]` text literal a `::vector` bind takes.
fn vec_text(v: &[f32]) -> String {
    let parts: Vec<String> = v.iter().map(|f| format!("{f}")).collect();
    format!("[{}]", parts.join(","))
}

/// One resource created through the REAL create path, homed in `ctx`, whose chunks carry the
/// CALLER-CHOSEN embedding (real chunker hashes: the write path applies the blocking policy and
/// refuses a hash no fresh chunking produces).
async fn create(
    pool: &PgPool,
    who: (ProfileId, EntityId),
    ctx: ContextId,
    title: &str,
    doc_type: &str,
    properties: &[(String, serde_json::Value)],
    embedding: &[f32],
) -> ResourceId {
    let body = format!("body of {title}");
    let chunks = temper_ingest::chunk::chunk_markdown(&body)
        .into_iter()
        .map(|c| IncomingChunk {
            chunk_index: c.chunk_index as i32,
            content_hash: c.content_hash,
            content: c.content,
            embedding: embedding.to_vec(),
            embedded_with: None,
            header_path: c.header_path,
            heading_depth: c.heading_depth as i16,
        })
        .collect();
    let origin = format!("test://{title}-{}", Uuid::now_v7());
    writes::create_resource(
        pool,
        CreateParams {
            idempotency_key: None,
            title,
            origin_uri: &origin,
            body: &body,
            doc_type,
            home: AnchorRef::context(ctx),
            owner: who.0,
            originator: who.0,
            emitter: who.1,
            properties,
            chunks: Some(chunks),
            sources: vec![],
        },
    )
    .await
    .expect("create through the real path")
}

/// A goal, plus one in-progress task that `advances` it — without a live task a goal's liveness
/// is zero and it contributes nothing to the telos (`context_goal_liveness`).
async fn live_goal(
    pool: &PgPool,
    who: (ProfileId, EntityId),
    ctx: ContextId,
    title: &str,
    embedding: &[f32],
) -> ResourceId {
    let goal = create(pool, who, ctx, title, "goal", &[], embedding).await;
    let stage = [(
        "temper-stage".to_owned(),
        serde_json::Value::from("in-progress"),
    )];
    let task = create(
        pool,
        who,
        ctx,
        &format!("{title} task"),
        "task",
        &stage,
        embedding,
    )
    .await;
    let mut conn = pool.acquire().await.expect("acquire");
    fire(
        &mut conn,
        SeedAction::RelationshipAssert {
            src: AnchorRef::resource(task),
            tgt: AnchorRef::resource(goal),
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("advances"),
            weight: 1.0,
            home: EdgeHome::Context(ctx),
            emitter: who.1,
        },
    )
    .await
    .expect("the advances edge asserts through the real path");
    goal
}

fn anchor_of(ctx: ContextId) -> HomeAnchor {
    HomeAnchor::Context(ctx)
}

/// Materialize through the real (full) producer.
async fn materialize(pool: &PgPool, ctx: ContextId, emitter: EntityId) {
    let anchor = anchor_of(ctx);
    write::materialize(pool, anchor, default_lens_for(anchor), emitter)
        .await
        .expect("materialize");
}

async fn lens_of(pool: &PgPool, ctx: ContextId) -> Uuid {
    let anchor = anchor_of(ctx);
    substrate::load_lens(pool, anchor, default_lens_for(anchor))
        .await
        .expect("lens")
        .1
        .uuid()
}

async fn genesis_of(pool: &PgPool, r: ResourceId) -> Uuid {
    sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'resource_created' AND (e.payload->>'resource_id')::uuid = $1",
    )
    .bind(r.uuid())
    .fetch_one(pool)
    .await
    .expect("R's genesis event")
}

/// A FOLDED region on `ctx` holding `members`, with a planted non-zero centroid — the shape a
/// past materialize leaves behind (the husk suite seeds region rows the same way).
async fn seed_folded_region(
    pool: &PgPool,
    ctx: ContextId,
    members: &[ResourceId],
    centroid: &[f32],
    event: Uuid,
) -> Uuid {
    let lens = lens_of(pool, ctx).await;
    let region: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmap_regions \
           (home_anchor_table, home_anchor_id, lens_id, centroid, salience, member_count, \
            asserted_by_event_id, last_event_id, is_folded) \
         VALUES ('kb_contexts', $1, $2, $3::vector, 1.0, $4, $5, $5, true) RETURNING id",
    )
    .bind(ctx.uuid())
    .bind(lens)
    .bind(vec_text(centroid))
    .bind(members.len() as i32)
    .bind(event)
    .fetch_one(pool)
    .await
    .expect("seed folded region");
    for m in members {
        sqlx::query(
            "INSERT INTO kb_cogmap_region_members (region_id, member_table, member_id, affinity) \
             VALUES ($1, 'kb_resources', $2, 1.0)",
        )
        .bind(region)
        .bind(m.uuid())
        .execute(pool)
        .await
        .expect("seed folded region member");
    }
    region
}

/// The live region on `ctx` whose members include every one of `members`.
async fn live_region_holding(pool: &PgPool, ctx: ContextId, members: &[Uuid]) -> Option<Uuid> {
    sqlx::query_scalar(
        "SELECT r.id FROM kb_cogmap_regions r \
          WHERE r.home_anchor_table = 'kb_contexts' AND r.home_anchor_id = $1 AND NOT r.is_folded \
            AND (SELECT count(*) FROM kb_cogmap_region_members m \
                  WHERE m.region_id = r.id AND m.member_table = 'kb_resources' \
                    AND m.member_id = ANY($2)) = cardinality($2)",
    )
    .bind(ctx.uuid())
    .bind(members)
    .fetch_optional(pool)
    .await
    .expect("live region lookup")
}

async fn region_centroid(pool: &PgPool, region: Uuid) -> String {
    sqlx::query_scalar("SELECT centroid::text FROM kb_cogmap_regions WHERE id = $1")
        .bind(region)
        .fetch_one(pool)
        .await
        .expect("region centroid")
}

async fn region_is_folded(pool: &PgPool, region: Uuid) -> bool {
    sqlx::query_scalar("SELECT is_folded FROM kb_cogmap_regions WHERE id = $1")
        .bind(region)
        .fetch_one(pool)
        .await
        .expect("region row")
}

/// THE ORACLE for a region's centroid: the mean, over the region's member rows, of each member's
/// mean embedding across its current chunks of unfolded blocks — `populate_readouts`' definition,
/// computed here independently. A member whose embeddings are NULL (an erased R) contributes
/// nothing, because `avg` skips NULL: so over a region still listing R, this IS the survivors'
/// mean.
async fn member_mean(pool: &PgPool, region: Uuid) -> String {
    sqlx::query_scalar::<_, Option<String>>(
        "SELECT avg(mv)::text FROM ( \
           SELECT avg(ch.embedding) AS mv FROM kb_cogmap_region_members mm \
             JOIN kb_chunks ch ON ch.resource_id = mm.member_id AND ch.is_current \
             JOIN kb_content_blocks b ON b.id = ch.block_id AND NOT b.is_folded \
            WHERE mm.region_id = $1 GROUP BY mm.member_id) per_member",
    )
    .bind(region)
    .fetch_one(pool)
    .await
    .expect("member mean")
    .expect("the region has embedded survivors")
}

async fn distance(pool: &PgPool, a: &str, b: &str) -> f64 {
    sqlx::query_scalar("SELECT ($1::vector <-> $2::vector)::float8")
        .bind(a)
        .bind(b)
        .fetch_one(pool)
        .await
        .expect("vector distance")
}

async fn stored_telos(pool: &PgPool, ctx: ContextId) -> Option<String> {
    sqlx::query_scalar("SELECT telos_centroid::text FROM kb_contexts WHERE id = $1")
        .bind(ctx.uuid())
        .fetch_one(pool)
        .await
        .expect("telos_centroid")
}

/// The live telos — computed on read, already blind to an inactive goal.
async fn live_telos(pool: &PgPool, ctx: ContextId) -> Option<String> {
    let lens = lens_of(pool, ctx).await;
    sqlx::query_scalar("SELECT anchor_telos_embedding('kb_contexts', $1, $2)::text")
        .bind(ctx.uuid())
        .bind(lens)
        .fetch_one(pool)
        .await
        .expect("anchor_telos_embedding")
}

/// Every `region_materialized` / `salience_refreshed` event on `ctx` minted after `after` whose
/// payload carries a non-null `telos_centroid` — ruling 4's predicate, evaluated independently.
async fn telos_carrying_events_after(pool: &PgPool, ctx: ContextId, after: Uuid) -> BTreeSet<Uuid> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name IN ('region_materialized', 'salience_refreshed') \
            AND e.producing_anchor_table = 'kb_contexts' AND e.producing_anchor_id = $1 \
            AND e.id > $2 AND e.payload->>'telos_centroid' IS NOT NULL",
    )
    .bind(ctx.uuid())
    .bind(after)
    .fetch_all(pool)
    .await
    .expect("telos-carrying events")
    .into_iter()
    .collect()
}

async fn queued_region_jobs(pool: &PgPool, ctx: ContextId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_workflow_jobs \
          WHERE context_id = $1 AND persona = 'region' AND dispatch_type = 'materialize' \
            AND status IN ('pending', 'in_progress', 'waiting_for_retry')",
    )
    .bind(ctx.uuid())
    .fetch_one(pool)
    .await
    .expect("count queued region jobs")
}

/// Empty the region queue, so a job seen after the act is the ACT'S enqueue, not a leftover.
async fn clear_region_queue(pool: &PgPool) {
    sqlx::query("DELETE FROM kb_workflow_jobs WHERE persona = 'region'")
        .execute(pool)
        .await
        .expect("clear region queue");
}

/// An operator's sealed proof, minted through the real gate after `grant_governance`. The
/// operator has a `@web` emitter: the act resolves one before it runs.
async fn operator(pool: &PgPool) -> temper_services::auth::SystemAdmin {
    let (op, _) = principal(pool).await;
    test_support::grant_governance(pool, op.uuid()).await;
    test_support::system_admin_proof_for(pool, op.uuid()).await
}

/// Run the real act and require it to complete.
async fn erase(pool: &PgPool, r: ResourceId) -> ResourceErasureCompletion {
    let admin = operator(pool).await;
    let outcome = execute_resource_erasure(
        pool,
        None,
        &admin,
        ResourceErasureRequest {
            resource: r,
            also_strike_blobs: &[],
            surface: Surface::ApiHttp,
        },
    )
    .await
    .expect("the act answers");
    match outcome {
        ResourceErasureOutcome::Completed(c) => c,
        other => panic!("the act must complete, got {other:?}"),
    }
}

async fn drain(pool: &PgPool) {
    region_service::dispatch_tick(pool, None)
        .await
        .expect("the region drain runs");
}

/// R planted everywhere its embedding can be derived into, materialized for real.
struct Planted {
    ctx: ContextId,
    /// The goal under erasure: axis 0.
    r: ResourceId,
    /// The surviving non-goal member that shares R's live region.
    s: ResourceId,
    r_genesis: Uuid,
    /// The live region holding both R and S when the act runs.
    live_region: Uuid,
    /// Its stored centroid before the act (R's share inside it).
    pre_act_centroid: String,
    /// The context's stored telos before the act (R and G2 blended).
    pre_act_telos: String,
}

/// Context C with two live goals (R on axis 0, G2 near it) and a surviving research member S:
///
/// 1. G2 (+ its task) and S are created, and C materialized ONCE BEFORE R exists — a
///    telos-carrying `region_materialized` minted before R's genesis, which ruling 4 must NOT name.
/// 2. R (+ its task) is created and C materialized: R and S share a live region; R is in the telos.
/// 3. X (axis 0, like R) is created and C materialized again: the region of step 2 changes member
///    set, so it FOLDS with R's share inside its centroid — a real folded region holding R.
/// 4. A salience refresh mints a telos-carrying `salience_refreshed` event.
/// 5. Two more folded regions are seeded: R alone (centroid = R's embedding exactly) and a stable
///    R+S pair (`2·centroid − S` recovers R) — ruling 2's two cases.
/// 6. The region queue is emptied.
async fn planted(pool: &PgPool) -> Planted {
    let who = principal(pool).await;
    let ctx = context(pool, who.0, "planted").await;
    let r_vec = axis(0);
    let s_vec = near_axis0(2);

    live_goal(pool, who, ctx, "goal-g2", &near_axis0(1)).await;
    let s = create(pool, who, ctx, "survivor-s", "research", &[], &s_vec).await;
    materialize(pool, ctx, who.1).await;

    let r = live_goal(pool, who, ctx, "goal-r", &r_vec).await;
    let r_genesis = genesis_of(pool, r).await;
    materialize(pool, ctx, who.1).await;

    create(pool, who, ctx, "survivor-x", "research", &[], &axis(0)).await;
    materialize(pool, ctx, who.1).await;

    let anchor = anchor_of(ctx);
    write::refresh_salience(pool, anchor, default_lens_for(anchor), who.1)
        .await
        .expect("refresh salience");

    seed_folded_region(pool, ctx, &[r], &r_vec, r_genesis).await;
    let pair: Vec<f32> = r_vec
        .iter()
        .zip(&s_vec)
        .map(|(a, b)| (a + b) / 2.0)
        .collect();
    seed_folded_region(pool, ctx, &[r, s], &pair, r_genesis).await;

    clear_region_queue(pool).await;

    let live_region = live_region_holding(pool, ctx, &[r.uuid(), s.uuid()])
        .await
        .expect("fixture precondition: R and S share a live region — R is NOT a sole member");
    let pre_act_centroid = region_centroid(pool, live_region).await;
    let pre_act_telos = stored_telos(pool, ctx)
        .await
        .expect("fixture precondition: C carries a telos snapshot with R inside it");

    Planted {
        ctx,
        r,
        s,
        r_genesis,
        live_region,
        pre_act_centroid,
        pre_act_telos,
    }
}

/// The norm of every folded region on `ctx` that lists `member`.
async fn folded_norms_holding(pool: &PgPool, ctx: ContextId, member: ResourceId) -> Vec<f64> {
    sqlx::query_scalar(
        "SELECT vector_norm(r.centroid)::float8 FROM kb_cogmap_regions r \
           JOIN kb_cogmap_region_members m ON m.region_id = r.id \
          WHERE r.home_anchor_table = 'kb_contexts' AND r.home_anchor_id = $1 AND r.is_folded \
            AND m.member_table = 'kb_resources' AND m.member_id = $2",
    )
    .bind(ctx.uuid())
    .bind(member.uuid())
    .fetch_all(pool)
    .await
    .expect("folded regions holding the member")
}

// ── the witnesses ────────────────────────────────────────────────────────────────────────────

/// (a) Ruling 2: every FOLDED region holding R gets the zero centroid in the act — the
/// single-member one (whose centroid IS R's embedding), the stable R+S pair (which recovers R
/// from S), and the one a real re-materialize folded with R inside.
///
/// FAILS ON MAIN BECAUSE step 7 touches only live regions' watermarks: every folded centroid
/// keeps R's share.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn every_folded_region_holding_the_erased_resource_gets_the_zero_centroid(pool: PgPool) {
    let p = planted(&pool).await;
    let before = folded_norms_holding(&pool, p.ctx, p.r).await;
    assert!(
        before.len() >= 3,
        "precondition: two seeded folded regions and at least one a real materialize folded, got {}",
        before.len()
    );
    assert!(
        before.iter().all(|n| *n > MOVED),
        "precondition: every folded centroid holding R is non-zero: {before:?}"
    );

    erase(&pool, p.r).await;

    let after = folded_norms_holding(&pool, p.ctx, p.r).await;
    assert_eq!(
        after.len(),
        before.len(),
        "R's member rows stay: pointers, not content"
    );
    assert!(
        after.iter().all(|n| *n == 0.0),
        "every folded region holding R must carry the zero centroid after the act: norms {after:?}"
    );
}

/// (a′) Ruling 2 as amended: the LIVE region holding R has its centroid recomputed IN THE ACT —
/// right after it, before any drain, the centroid is the survivors' mean.
///
/// FAILS ON MAIN BECAUSE the act recomputes nothing: the live centroid still includes R.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn right_after_the_act_the_live_region_centroid_is_the_survivors_mean(pool: PgPool) {
    let p = planted(&pool).await;

    erase(&pool, p.r).await;

    // R's embeddings are nulled by the act, so the oracle over the region's member rows (which
    // still list R) is the survivors' mean.
    let survivors = member_mean(&pool, p.live_region).await;
    assert!(
        distance(&pool, &p.pre_act_centroid, &survivors).await > MOVED,
        "vacuity guard: the survivors' mean must differ from the pre-act centroid"
    );
    let stored = region_centroid(&pool, p.live_region).await;
    let gap = distance(&pool, &stored, &survivors).await;
    assert!(
        gap < SAME,
        "the live region's centroid right after the act must be the survivors' mean \
         (distance {gap}); R's share is still inside it"
    );
}

/// (b) Ruling 3: the home context's telos snapshot is NULL right after the act.
///
/// FAILS ON MAIN BECAUSE step 7 nulls only the watermark: `telos_centroid` still holds the
/// snapshot with R's embedding in it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_home_contexts_telos_snapshot_is_null_right_after_the_act(pool: PgPool) {
    let p = planted(&pool).await;

    erase(&pool, p.r).await;

    assert_eq!(
        stored_telos(&pool, p.ctx).await,
        None,
        "the act must null the home context's telos snapshot"
    );
}

/// (c) Ruling 4: R is a goal homed in C, so the record's `ledger_remainder` names every
/// `region_materialized` / `salience_refreshed` event on C minted after R's genesis with a
/// non-null `telos_centroid`, each as `(event, ["telos_centroid"])` — and no other.
///
/// FAILS ON MAIN BECAUSE `ledger_remainder` carries only R's own trail events.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_record_names_every_telos_carrying_region_event_on_a_goals_home_context(pool: PgPool) {
    let p = planted(&pool).await;
    let expected = telos_carrying_events_after(&pool, p.ctx, p.r_genesis).await;
    let all_time = telos_carrying_events_after(&pool, p.ctx, Uuid::nil()).await;
    assert!(
        expected.len() >= 3,
        "precondition: two materializes and one refresh after R's genesis, got {}",
        expected.len()
    );
    assert!(
        all_time.len() > expected.len(),
        "precondition: a telos-carrying event minted BEFORE R's genesis exists, so the \
         after-genesis bound is exercised"
    );

    let record = erase(&pool, p.r).await;

    let telos_path = vec!["telos_centroid".to_owned()];
    let named: BTreeSet<Uuid> = record
        .ledger_remainder
        .iter()
        .filter(|f| f.paths == telos_path)
        .map(|f| f.event.uuid())
        .collect();
    assert_eq!(
        named, expected,
        "the record must name exactly the telos-carrying region events on R's home context \
         minted after R's genesis, each with path [\"telos_centroid\"]"
    );
}

/// (c′) Ruling 4's goal-only predicate: erasing a NON-goal homed in the same context names no
/// telos-carrying event, though such events exist after its genesis.
///
/// Passes on main (main names none); it bites a fix that drops the `doc_type = 'goal'` condition.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_non_goal_erasure_names_no_telos_carrying_event(pool: PgPool) {
    let p = planted(&pool).await;
    let s_genesis = genesis_of(&pool, p.s).await;
    assert!(
        !telos_carrying_events_after(&pool, p.ctx, s_genesis)
            .await
            .is_empty(),
        "precondition: telos-carrying events exist after S's genesis, so a widened predicate \
         would name them"
    );

    let record = erase(&pool, p.s).await;

    let carrying: Vec<Uuid> = record
        .ledger_remainder
        .iter()
        .filter(|f| f.paths.iter().any(|path| path == "telos_centroid"))
        .map(|f| f.event.uuid())
        .collect();
    assert!(
        carrying.is_empty(),
        "a non-goal erasure must name no telos_centroid path: {carrying:?}"
    );
}

/// (d) Ruling 1: the act queues a region settle for R's home context.
///
/// FAILS ON MAIN BECAUSE `execute_resource_erasure` enqueues no region job (the queue was
/// emptied before the act, so any job here is the act's).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_act_queues_a_region_settle_for_the_home_context(pool: PgPool) {
    let p = planted(&pool).await;
    assert_eq!(queued_region_jobs(&pool, p.ctx).await, 0, "precondition");

    erase(&pool, p.r).await;

    assert_eq!(
        queued_region_jobs(&pool, p.ctx).await,
        1,
        "the act must queue exactly one region settle for R's home context"
    );
}

/// (e) Ruling 1, the settle: after one drain R has left formation, and the live region holding
/// S has the centroid of its own members — R's share gone.
///
/// FAILS ON MAIN BECAUSE the drain finds no job: nothing re-materializes, R is still a member of
/// the live region and its centroid still includes R.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn after_one_drain_the_live_region_centroid_is_the_survivors_mean(pool: PgPool) {
    let p = planted(&pool).await;

    erase(&pool, p.r).await;
    drain(&pool).await;

    assert_eq!(
        live_region_holding(&pool, p.ctx, &[p.r.uuid()]).await,
        None,
        "after the drain R must be a member of no live region"
    );
    let region = live_region_holding(&pool, p.ctx, &[p.s.uuid()])
        .await
        .expect("S, a survivor, is in a live region");
    let survivors = member_mean(&pool, region).await;
    assert!(
        distance(&pool, &p.pre_act_centroid, &survivors).await > MOVED,
        "vacuity guard: the survivors' mean must differ from the pre-act centroid"
    );
    let stored = region_centroid(&pool, region).await;
    let gap = distance(&pool, &stored, &survivors).await;
    assert!(
        gap < SAME,
        "the live centroid must be the survivors' mean after the drain (distance {gap})"
    );
}

/// (e′) Ruling 3, the re-arm: after one drain C's telos snapshot is the live telos of the
/// surviving goal G2 alone.
///
/// FAILS ON MAIN BECAUSE the drain finds no job: the snapshot still blends R with G2.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn after_one_drain_the_telos_snapshot_is_the_surviving_goals_telos(pool: PgPool) {
    let p = planted(&pool).await;

    erase(&pool, p.r).await;
    drain(&pool).await;

    // The live telos is computed on read and already excludes the inactive R — the oracle.
    let live = live_telos(&pool, p.ctx)
        .await
        .expect("G2 is a live goal, so C has a telos");
    assert!(
        distance(&pool, &p.pre_act_telos, &live).await > MOVED,
        "vacuity guard: G2's telos alone must differ from the pre-act R+G2 snapshot"
    );
    let stored = stored_telos(&pool, p.ctx)
        .await
        .expect("the drain's materialize re-arms the snapshot");
    let gap = distance(&pool, &stored, &live).await;
    assert!(
        gap < SAME,
        "the telos snapshot after the drain must be G2's telos alone (distance {gap})"
    );
}

/// (g) Ruling 2 as amended: the region that was LIVE when the act ran is folded by the drain's
/// re-materialize (its member set changed, so it is not reused) — and it folds with the
/// survivors' mean the act wrote, not a centroid with R inside.
///
/// FAILS ON MAIN BECAUSE no job is queued, so the drain never re-materializes and the region is
/// still live; and were it folded, its row would keep R's share (main recomputes no live
/// centroid in the act).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_region_live_at_the_act_folds_without_the_erased_resources_share(pool: PgPool) {
    let p = planted(&pool).await;

    erase(&pool, p.r).await;
    drain(&pool).await;

    assert!(
        region_is_folded(&pool, p.live_region).await,
        "the drain's re-materialize must fold the region R was live in (its member set changed)"
    );
    let survivors = member_mean(&pool, p.live_region).await;
    assert!(
        distance(&pool, &p.pre_act_centroid, &survivors).await > MOVED,
        "vacuity guard: the survivors' mean must differ from the pre-act centroid"
    );
    let stored = region_centroid(&pool, p.live_region).await;
    let gap = distance(&pool, &stored, &survivors).await;
    assert!(
        gap < SAME,
        "the folded row must hold the survivors' mean (distance {gap}), not R's share"
    );
}

/// (f) Ruling 3 when R is a context's ONLY goal: after the act and one drain the telos snapshot
/// is NULL — the live telos is NULL, and the drain's materialize records that.
///
/// FAILS ON MAIN BECAUSE the snapshot is never re-armed: the drift gate declines on a NULL live
/// telos and no job is queued, so `telos_centroid` stays exactly R's embedding.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_context_whose_only_goal_is_erased_ends_with_no_telos_snapshot(pool: PgPool) {
    let who = principal(&pool).await;
    let ctx = context(&pool, who.0, "sole-goal").await;
    let r = live_goal(&pool, who, ctx, "only-goal", &axis(0)).await;
    for i in 1..=3 {
        create(
            &pool,
            who,
            ctx,
            &format!("survivor-{i}"),
            "research",
            &[],
            &near_axis0(i),
        )
        .await;
    }
    materialize(&pool, ctx, who.1).await;
    clear_region_queue(&pool).await;
    assert!(
        stored_telos(&pool, ctx).await.is_some(),
        "precondition: the snapshot is R's embedding before the act"
    );

    erase(&pool, r).await;
    drain(&pool).await;

    assert_eq!(
        stored_telos(&pool, ctx).await,
        None,
        "a context whose only goal was erased must carry no telos snapshot after the drain"
    );
}
