use super::*;

async fn a_resource(pool: &PgPool) -> uuid::Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('embed-target', '') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn status_of(pool: &PgPool, id: uuid::Uuid) -> String {
    sqlx::query_scalar("SELECT status FROM kb_workflow_jobs WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

// The drain orchestration: an enqueued embed job is claimed, its (no-op, chunkless) embed
// completes cleanly, the job flips to done, and the summary tallies it. A resource with no
// deferred chunks embeds zero chunks — exercising reap→claim→embed→complete→summary without the
// full chunk graph (the real ONNX round-trip is covered by the Phase 4 e2e create→drain→search
// test). ONNX-free: embed_resource_chunks runs no inference when there are no NULL-vector chunks.
#[sqlx::test(migrations = "../../migrations")]
async fn dispatch_tick_claims_embeds_and_completes(pool: PgPool) {
    let r = a_resource(&pool).await;
    let job = workflow_job_service::enqueue_resource(&pool, r, "embed", "embed")
        .await
        .unwrap()
        .expect("enqueue creates a job");

    let summary = dispatch_tick(&pool, Some(5), false).await.unwrap();
    assert_eq!(summary.claimed, 1);
    assert_eq!(summary.completed, 1);
    assert_eq!(summary.failed, 0);
    assert_eq!(summary.chunks_embedded, 0, "no deferred chunks to embed");
    assert_eq!(
        status_of(&pool, job).await,
        "done",
        "clean embed → job done"
    );

    // Idempotent: a second tick finds nothing to claim.
    let again = dispatch_tick(&pool, Some(5), false).await.unwrap();
    assert_eq!(again.claimed, 0);
}

// An empty queue is a clean no-op pass.
#[sqlx::test(migrations = "../../migrations")]
async fn dispatch_tick_empty_queue_is_noop(pool: PgPool) {
    let summary = dispatch_tick(&pool, None, false).await.unwrap();
    assert_eq!(summary.claimed, 0);
    assert_eq!(summary.completed, 0);
    assert_eq!(summary.chunks_embedded, 0);
}

/// **The fan-out safety invariant.** Two drainers running concurrently against the same queue
/// must claim DISJOINT resources — `FOR UPDATE SKIP LOCKED` guarantees no resource is claimed by
/// both. With cap=1 the two passes interleave claim-by-claim; together they complete each
/// resource exactly once (sum of completed == the enqueued count), never twice, never missing one.
#[sqlx::test(migrations = "../../migrations")]
async fn concurrent_dispatch_ticks_claim_disjoint_resources(pool: PgPool) {
    for i in 0..10 {
        let r = a_named_resource(&pool, &format!("r{i}")).await;
        workflow_job_service::enqueue_resource(&pool, r, "embed", "embed")
            .await
            .unwrap()
            .expect("enqueue");
    }

    let (s1, s2) = tokio::join!(
        dispatch_tick(&pool, Some(1), false),
        dispatch_tick(&pool, Some(1), false),
    );
    let (s1, s2) = (s1.unwrap(), s2.unwrap());

    assert_eq!(
        s1.completed + s2.completed,
        10,
        "two concurrent drainers drain every resource exactly once — no double-claim, none missed"
    );

    // Stronger than the sum: every embed job reached a terminal `done` — none left behind
    // pending/in_progress by a lost race.
    let done: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_workflow_jobs \
         WHERE persona = 'embed' AND dispatch_type = 'embed' AND status = 'done'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        done, 10,
        "all 10 embed jobs reached done — no resource left behind"
    );
}

// The wall-clock guard: a pass that is already past its deadline embeds nothing and re-enqueues the
// claimed job untouched (tallied `partial`, not `failed`), so the next tick resumes it. A
// `Duration::ZERO` deadline forces the defer branch on the first check without sleeping. ONNX-free:
// the resource is chunkless, and the defer path never calls the embedder anyway.
#[sqlx::test(migrations = "../../migrations")]
async fn dispatch_tick_defers_jobs_past_the_wall_clock_deadline(pool: PgPool) {
    let r = a_resource(&pool).await;
    workflow_job_service::enqueue_resource(&pool, r, "embed", "embed")
        .await
        .unwrap()
        .expect("enqueue");

    let summary = dispatch_tick_inner(&pool, Some(5), false, std::time::Duration::ZERO)
        .await
        .unwrap();
    assert_eq!(summary.claimed, 1);
    assert_eq!(
        summary.partial, 1,
        "a deadline-deferred job is partial (re-enqueued), not failed"
    );
    assert_eq!(summary.completed, 0);
    assert_eq!(summary.failed, 0);
    assert_eq!(
        summary.chunks_embedded, 0,
        "nothing is embedded past the deadline"
    );

    // The deferred job was completed + re-enqueued, so the resource holds one fresh pending job.
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_workflow_jobs WHERE resource_id = $1 AND status = 'pending'",
    )
    .bind(r)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pending, 1, "deferred job re-enqueued as pending");

    // A normal-deadline tick then drains the re-enqueued job cleanly — progress is monotonic.
    let drained = dispatch_tick(&pool, Some(5), false).await.unwrap();
    assert_eq!(drained.claimed, 1);
    assert_eq!(
        drained.completed, 1,
        "the re-enqueued job drains on the next tick"
    );
}

// ── Phase 4: derived embedding_status + re-drive (all ONNX-free) ───────────────────────────
//
// These fixtures build the chunk graph by hand (a real migration DB seeds a few kb_events we can
// borrow for the block's genesis/last event FKs), so the derivation and re-drive logic are
// exercised without any ONNX inference. `embedding IS NULL` vs a filled 768-vector is all the
// derivation reads; the vector's contents are irrelevant, so a zero-fill stands in.

async fn a_named_resource(pool: &PgPool, title: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_resources (title, origin_uri) VALUES ($1, '') RETURNING id")
        .bind(title)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// A content block on `resource` at `seq`, reusing a migration-seeded event for the genesis/last
/// FKs. `folded` blocks are excluded by the status derivation.
async fn a_block(pool: &PgPool, resource: Uuid, seq: i32, folded: bool) -> Uuid {
    let ev: Uuid = sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(pool)
        .await
        .unwrap();
    sqlx::query_scalar(
        "INSERT INTO kb_content_blocks (resource_id, seq, is_folded, genesis_event_id, last_event_id) \
         VALUES ($1, $2, $3, $4, $4) RETURNING id",
    )
    .bind(resource)
    .bind(seq)
    .bind(folded)
    .bind(ev)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A chunk on `block`. `embedded` fills a zero 768-vector (any non-NULL vector reads as embedded);
/// otherwise the embedding stays NULL (the deferred state).
async fn a_chunk(
    pool: &PgPool,
    block: Uuid,
    resource: Uuid,
    idx: i32,
    embedded: bool,
    current: bool,
) {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash, is_current) \
         VALUES ($1, $2, $3, 'h', $4) RETURNING id",
    )
    .bind(block)
    .bind(resource)
    .bind(idx)
    .bind(current)
    .fetch_one(pool)
    .await
    .unwrap();
    if embedded {
        sqlx::query("UPDATE kb_chunks SET embedding = array_fill(0::real, ARRAY[768])::vector WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
    }
}

async fn stamp(pool: &PgPool, resource: Uuid, sha: &str) {
    sqlx::query("UPDATE kb_chunks SET embedded_with = $2 WHERE resource_id = $1")
        .bind(resource)
        .bind(sha)
        .execute(pool)
        .await
        .unwrap();
}

/// **The regression test for the model split.** A chunk that HAS a vector but no provenance is
/// exactly the shape of every row written before `embedded_with` existed — i.e. ~95% of the real
/// index, all of it fp32. Under the old predicate (`embedding IS NULL`) such a chunk was considered
/// perfectly fine and would never have been re-embedded. It must now read as stale.
#[sqlx::test(migrations = "../../migrations")]
async fn an_embedded_chunk_with_unknown_provenance_is_stale(pool: PgPool) {
    let r = a_named_resource(&pool, "legacy").await;
    let b = a_block(&pool, r, 0, false).await;
    a_chunk(&pool, b, r, 0, true, true).await; // has a vector, no embedded_with

    let (resources, chunks) = stale_summary(&pool, ReembedScope::All).await.unwrap();
    assert_eq!(
        (resources, chunks),
        (1, 1),
        "an embedded-but-unstamped chunk must be STALE — it is the fp32 case"
    );
}

/// The other half: a chunk stamped with the model this build actually embeds with is NOT stale, so
/// the drain converges instead of re-embedding the same rows forever.
#[sqlx::test(migrations = "../../migrations")]
async fn a_chunk_stamped_with_the_current_model_is_not_stale(pool: PgPool) {
    let r = a_named_resource(&pool, "fresh").await;
    let b = a_block(&pool, r, 0, false).await;
    a_chunk(&pool, b, r, 0, true, true).await;
    stamp(&pool, r, temper_ingest::embed::EXPECTED_MODEL_SHA256).await;

    let (resources, chunks) = stale_summary(&pool, ReembedScope::All).await.unwrap();
    assert_eq!(
        (resources, chunks),
        (0, 0),
        "current-model vectors are fresh"
    );
}

/// A vector stamped by SOME OTHER model is stale — this is what makes a future model change
/// a future model change need no migration: the constant changes, the whole index re-stales itself,
/// and an operator sweep drains it.
#[sqlx::test(migrations = "../../migrations")]
async fn a_chunk_stamped_with_a_different_model_is_stale(pool: PgPool) {
    let r = a_named_resource(&pool, "other-model").await;
    let b = a_block(&pool, r, 0, false).await;
    a_chunk(&pool, b, r, 0, true, true).await;
    stamp(
        &pool,
        r,
        "0000000000000000000000000000000000000000000000000000000000000000",
    )
    .await;

    let (resources, _) = stale_summary(&pool, ReembedScope::All).await.unwrap();
    assert_eq!(resources, 1, "a foreign model's vectors are stale");
}

/// Scoping is the whole point of the trigger: try ONE resource before you try 31,000.
#[sqlx::test(migrations = "../../migrations")]
async fn enqueue_stale_scopes_to_a_single_resource(pool: PgPool) {
    let admin = crate::test_support::system_admin_proof(&pool).await;
    let a = a_named_resource(&pool, "a").await;
    let ba = a_block(&pool, a, 0, false).await;
    a_chunk(&pool, ba, a, 0, true, true).await;

    let b = a_named_resource(&pool, "b").await;
    let bb = a_block(&pool, b, 0, false).await;
    a_chunk(&pool, bb, b, 0, true, true).await;

    let (all_stale, _) = stale_summary(&pool, ReembedScope::All).await.unwrap();
    assert_eq!(all_stale, 2, "both resources are stale");

    let enqueued = enqueue_stale(&pool, &admin, ReembedScope::Resource(a), 100)
        .await
        .unwrap();
    assert_eq!(enqueued, vec![a], "only the scoped resource is enqueued");

    // Idempotent: the resource now has a live job, so a second call adds nothing.
    let again = enqueue_stale(&pool, &admin, ReembedScope::Resource(a), 100)
        .await
        .unwrap();
    assert!(
        again.is_empty(),
        "a resource with a live job is not enqueued twice"
    );
}

/// Context scoping goes through `kb_resource_homes` (a resource has no context column), and the
/// `anchor_table = 'kb_contexts'` predicate is load-bearing: the SAME table homes cogmap nodes, so
/// without it a context-scoped re-embed would silently drag in every cogmap-homed resource too.
#[sqlx::test(migrations = "../../migrations")]
async fn enqueue_stale_scopes_to_a_context_and_excludes_cogmap_homes(pool: PgPool) {
    let admin = crate::test_support::system_admin_proof(&pool).await;
    let owner: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ('scope-owner', 'Scope Owner') \
         RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let ctx: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, name, slug) \
         VALUES ('kb_profiles', $1, 'scope-ctx', 'scope-ctx') RETURNING id",
    )
    .bind(owner)
    .fetch_one(&pool)
    .await
    .unwrap();

    // In the context.
    let inside = a_named_resource(&pool, "inside").await;
    let bi = a_block(&pool, inside, 0, false).await;
    a_chunk(&pool, bi, inside, 0, true, true).await;
    sqlx::query(
        "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(inside)
    .bind(ctx)
    .bind(owner)
    .execute(&pool)
    .await
    .unwrap();

    // Homed in a COGMAP whose id happens to equal the context id — the exact collision the
    // anchor_table predicate exists to reject. Without it this resource would be swept in.
    let decoy = a_named_resource(&pool, "cogmap-homed").await;
    let bd = a_block(&pool, decoy, 0, false).await;
    a_chunk(&pool, bd, decoy, 0, true, true).await;
    sqlx::query(
        "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_cogmaps', $2, $3, $3)",
    )
    .bind(decoy)
    .bind(ctx)
    .bind(owner)
    .execute(&pool)
    .await
    .unwrap();

    let (stale, _) = stale_summary(&pool, ReembedScope::Context(ctx))
        .await
        .unwrap();
    assert_eq!(stale, 1, "only the context-homed resource is in scope");

    let enqueued = enqueue_stale(&pool, &admin, ReembedScope::Context(ctx), 100)
        .await
        .unwrap();
    assert_eq!(
        enqueued,
        vec![inside],
        "a cogmap-homed resource must NOT be swept into a context re-embed"
    );
}

/// **The wedge test.** A blank chunk has nothing to embed. It must still CONVERGE — i.e. stop
/// being stale — or `remaining` never reaches zero, the drain re-enqueues that resource every
/// minute forever embedding nothing, and `stale_summary` never converges (destroying the
/// operator's only progress signal).
///
/// This is why STALE_CHUNK_PREDICATE keys on `embedded_with` ALONE. With the redundant
/// `embedding IS NULL OR` disjunct it originally carried, stamping a blank chunk could never clear
/// it and this test would hang the drain in perpetuity.
#[sqlx::test(migrations = "../../migrations")]
async fn a_blank_chunk_converges_instead_of_wedging_the_drain(pool: PgPool) {
    let r = a_named_resource(&pool, "blank").await;
    let b = a_block(&pool, r, 0, false).await;

    let chunk: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash, is_current) \
         VALUES ($1, $2, 0, 'h', true) RETURNING id",
    )
    .bind(b)
    .bind(r)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO kb_chunk_content (chunk_id, content) VALUES ($1, '   ')")
        .bind(chunk)
        .execute(&pool)
        .await
        .unwrap();

    let progress = temper_substrate::embed::embed_resource_chunks(
        &pool,
        r,
        temper_substrate::embed::EMBED_CHUNK_BUDGET,
    )
    .await
    .unwrap();
    assert_eq!(progress.embedded, 0, "a blank chunk embeds nothing");
    assert!(
        progress.is_complete(),
        "...but it MUST converge: remaining={} — a non-zero remaining here means the drain \
         re-enqueues this resource every minute forever",
        progress.remaining
    );

    let (stale, _) = stale_summary(&pool, ReembedScope::All).await.unwrap();
    assert_eq!(
        stale, 0,
        "and the operator's convergence signal reaches zero"
    );
}

/// The enqueued job must be claimable by the drain — i.e. enqueued under the SAME
/// (persona, dispatch_type) tuple `dispatch_tick` claims on. Get this wrong and the job sits
/// pending forever, looking enqueued and doing nothing.
#[sqlx::test(migrations = "../../migrations")]
async fn a_stale_enqueued_resource_is_claimed_by_the_drain(pool: PgPool) {
    let admin = crate::test_support::system_admin_proof(&pool).await;
    let r = a_named_resource(&pool, "drainable").await;
    let b = a_block(&pool, r, 0, false).await;
    a_chunk(&pool, b, r, 0, true, true).await;

    let enqueued = enqueue_stale(&pool, &admin, ReembedScope::All, 100)
        .await
        .unwrap();
    assert_eq!(enqueued, vec![r]);

    let summary = dispatch_tick(&pool, Some(5), false).await.unwrap();
    assert_eq!(
        summary.claimed, 1,
        "the drain must claim what enqueue_stale queued"
    );
}

async fn set_job_status(pool: &PgPool, resource: Uuid, status: &str) {
    sqlx::query("UPDATE kb_workflow_jobs SET status = $2 WHERE resource_id = $1")
        .bind(resource)
        .bind(status)
        .execute(pool)
        .await
        .unwrap();
}

// The three derived states (design §8), plus the no-chunks and no-job edges, in one batch call.
#[sqlx::test(migrations = "../../migrations")]
async fn embedding_status_derives_ready_pending_failed(pool: PgPool) {
    // ready — a fully-embedded chunk, no job.
    let ready = a_named_resource(&pool, "ready").await;
    let b = a_block(&pool, ready, 0, false).await;
    a_chunk(&pool, b, ready, 0, true, true).await;

    // ready — a resource with no chunks at all (empty body is trivially embedded).
    let empty = a_named_resource(&pool, "empty").await;

    // pending — a NULL-embedding chunk with a live (pending) embed job.
    let pending = a_named_resource(&pool, "pending").await;
    let pb = a_block(&pool, pending, 0, false).await;
    a_chunk(&pool, pb, pending, 0, false, true).await;
    workflow_job_service::enqueue_resource(&pool, pending, "embed", "embed")
        .await
        .unwrap()
        .expect("enqueue");

    // failed — a NULL-embedding chunk whose only embed job is dead.
    let failed = a_named_resource(&pool, "failed").await;
    let fb = a_block(&pool, failed, 0, false).await;
    a_chunk(&pool, fb, failed, 0, false, true).await;
    workflow_job_service::enqueue_resource(&pool, failed, "embed", "embed")
        .await
        .unwrap()
        .expect("enqueue");
    set_job_status(&pool, failed, "dead").await;

    // failed — a NULL-embedding chunk with no job at all (supersede-race shape).
    let orphan = a_named_resource(&pool, "orphan").await;
    let ob = a_block(&pool, orphan, 0, false).await;
    a_chunk(&pool, ob, orphan, 0, false, true).await;

    let ids = [ready, empty, pending, failed, orphan];
    let got = embedding_status_batch(&pool, &ids).await.unwrap();
    assert_eq!(got[&ready], EmbeddingStatus::Ready);
    assert_eq!(got[&empty], EmbeddingStatus::Ready, "no chunks ⇒ ready");
    assert_eq!(got[&pending], EmbeddingStatus::Pending);
    assert_eq!(got[&failed], EmbeddingStatus::Failed, "dead job ⇒ failed");
    assert_eq!(
        got[&orphan],
        EmbeddingStatus::Failed,
        "no live job ⇒ failed"
    );
}

// The derivation ignores non-current and folded-block chunks: a resource whose ONLY NULL-embedding
// chunk is superseded (not current) or on a folded block reads as `ready`.
#[sqlx::test(migrations = "../../migrations")]
async fn embedding_status_ignores_noncurrent_and_folded(pool: PgPool) {
    // Only NULL chunk is non-current (an old generation) — the current one is embedded.
    let superseded = a_named_resource(&pool, "superseded").await;
    let sb = a_block(&pool, superseded, 0, false).await;
    a_chunk(&pool, sb, superseded, 0, false, false).await; // old, NULL, not current
    a_chunk(&pool, sb, superseded, 1, true, true).await; // current, embedded

    // Only NULL chunk lives on a folded block.
    let folded = a_named_resource(&pool, "folded").await;
    let foldb = a_block(&pool, folded, 0, true).await;
    a_chunk(&pool, foldb, folded, 0, false, true).await;

    let got = embedding_status_batch(&pool, &[superseded, folded])
        .await
        .unwrap();
    assert_eq!(
        got[&superseded],
        EmbeddingStatus::Ready,
        "non-current NULL chunk is ignored"
    );
    assert_eq!(
        got[&folded],
        EmbeddingStatus::Ready,
        "folded-block NULL chunk is ignored"
    );
}

// Re-drive resurrects a dead embed job as a fresh pending one; a resource with a live job is left
// alone. ONNX-free: the resources are chunkless, so nothing is inferred here.
#[sqlx::test(migrations = "../../migrations")]
async fn redrive_reenqueues_dead_only(pool: PgPool) {
    // A dead-jobbed resource.
    let dead = a_resource(&pool).await;
    workflow_job_service::enqueue_resource(&pool, dead, "embed", "embed")
        .await
        .unwrap()
        .expect("enqueue");
    set_job_status(&pool, dead, "dead").await;

    // A resource whose job is still live — re-drive must not touch it.
    let live = a_named_resource(&pool, "live").await;
    workflow_job_service::enqueue_resource(&pool, live, "embed", "embed")
        .await
        .unwrap()
        .expect("enqueue");

    let n = redrive(&pool, None).await.unwrap();
    assert_eq!(n, 1, "only the dead-jobbed resource is re-driven");

    // The dead resource now has a fresh pending job (the dead row stays as history).
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_workflow_jobs WHERE resource_id = $1 AND status = 'pending'",
    )
    .bind(dead)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(pending, 1, "dead job re-enqueued as pending");

    // Re-drive is idempotent while a live job exists: a second pass re-drives nothing.
    assert_eq!(redrive(&pool, None).await.unwrap(), 0);
}

// The `?redrive=true` composition: a single dispatch pass revives a dead job and drains it in the
// same tick. Chunkless resource ⇒ the embed is a clean no-op, so this stays ONNX-free.
#[sqlx::test(migrations = "../../migrations")]
async fn dispatch_tick_with_redrive_revives_and_drains(pool: PgPool) {
    let r = a_resource(&pool).await;
    workflow_job_service::enqueue_resource(&pool, r, "embed", "embed")
        .await
        .unwrap()
        .expect("enqueue");
    set_job_status(&pool, r, "dead").await;

    // Without re-drive, a plain tick finds nothing claimable (the job is dead, not pending).
    let plain = dispatch_tick(&pool, Some(5), false).await.unwrap();
    assert_eq!(plain.redriven, 0);
    assert_eq!(plain.claimed, 0, "a dead job is not claimable");

    // With re-drive, the same pass re-enqueues the dead job and then claims + completes it.
    let summary = dispatch_tick(&pool, Some(5), true).await.unwrap();
    assert_eq!(summary.redriven, 1);
    assert_eq!(
        summary.claimed, 1,
        "the re-driven job is drained in the same pass"
    );
    assert_eq!(summary.completed, 1);
    assert_eq!(summary.failed, 0);
}

/// **The loop-drain gate.** Three enqueued resources with cap=1 → one resource per claim. A
/// single-claim pass (the old behavior) would drain exactly ONE and return; loop-drain must drain
/// all THREE inside one invocation by re-claiming until the queue is empty. Chunkless resources
/// keep this ONNX-free (embed is a clean no-op).
#[sqlx::test(migrations = "../../migrations")]
async fn dispatch_tick_loop_drains_multiple_claims_in_one_pass(pool: PgPool) {
    for name in ["a", "b", "c"] {
        let r = a_named_resource(&pool, name).await;
        workflow_job_service::enqueue_resource(&pool, r, "embed", "embed")
            .await
            .unwrap()
            .expect("enqueue");
    }

    let summary = dispatch_tick(&pool, Some(1), false).await.unwrap();
    assert_eq!(
        summary.claimed, 3,
        "loop-drain re-claims until the queue is empty — not just one claim"
    );
    assert_eq!(summary.completed, 3);
    assert_eq!(summary.failed, 0);

    // Idempotent: a second pass finds nothing.
    let again = dispatch_tick(&pool, Some(1), false).await.unwrap();
    assert_eq!(again.claimed, 0);
}

// ── the embed-repair gate (D4 arm 2; re-keyed to the ROW by the offboarding ruling,
// 20260911000000) ────────────────────────────────────────────────────────────────────
//
// A WIPED chunk — its own content empty AND its hash in `kb_erased_content` — is dead on
// the embed side: the erasure nulled its vector and provenance, and the drain must never
// re-embed it. The gate is an exclusion in the STALE predicate
// (temper_substrate::embed::STALE_CHUNK_PREDICATE and this file's two copies of the same
// query), NOT a skip in the loop — a skip would leave the chunk stale forever, and
// `remaining` would never reach zero (the wedge the predicate's doc warns about). Excluded
// chunks are not work; the resource converges with the vector NULL.
//
// The row anchor is the ruling: the 2026-08-31 gate keyed on hash membership alone, which
// excluded another principal's LIVE same-hash chunk from embed work forever. Only the
// chunk's own wiped state excludes now.

/// A current chunk carrying exactly the content given (so the drain would embed it when
/// the content is non-empty), optionally registered in the erased-content set.
async fn a_prose_chunk(
    pool: &PgPool,
    block: Uuid,
    resource: Uuid,
    idx: i32,
    hash: &str,
    content: &str,
) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_chunks (block_id, resource_id, chunk_index, content_hash, is_current) \
         VALUES ($1, $2, $3, $4, true) RETURNING id",
    )
    .bind(block)
    .bind(resource)
    .bind(idx)
    .bind(hash)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO kb_chunk_content (chunk_id, content) VALUES ($1, $2)")
        .bind(id)
        .bind(content)
        .execute(pool)
        .await
        .unwrap();
    id
}

async fn erase_hash(pool: &PgPool, hash: &str) {
    sqlx::query(
        "INSERT INTO kb_erased_content (content_hash, erased_by_event_id) \
         VALUES ($1, (SELECT id FROM kb_events ORDER BY id LIMIT 1))",
    )
    .bind(hash)
    .execute(pool)
    .await
    .unwrap();
}

/// FAILS IF the drain re-embeds (or even re-stamps provenance on) a WIPED chunk, or if
/// hash membership alone still excludes: the resource carries BOTH shapes under the SAME
/// erased hash — the wiped chunk (content '', vector + provenance NULL, hash in the set)
/// and a live chunk (real prose, SAME hash). The row-anchored gate excludes only the
/// wiped one; the live one embeds. Under the retired hash-only gate BOTH were excluded
/// and `embedded` would be 0 — this differential is the retirement's bite. The resource
/// must CONVERGE (`remaining` reaches zero, the drain's own wedge invariant) with the
/// wiped chunk's vector still NULL.
#[sqlx::test(migrations = "../../migrations")]
async fn a_wiped_chunk_is_never_re_embedded_and_a_same_hash_live_chunk_embeds(pool: PgPool) {
    let r = a_named_resource(&pool, "tombstoned").await;
    let b = a_block(&pool, r, 0, false).await;
    let hash = "aa0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let wiped = a_prose_chunk(&pool, b, r, 0, hash, "").await;
    erase_hash(&pool, hash).await;
    let live = a_prose_chunk(
        &pool,
        b,
        r,
        1,
        hash,
        "live prose under a hash some erasure recorded — THIS row is not that erasure's",
    )
    .await;

    let progress = temper_substrate::embed::embed_resource_chunks(
        &pool,
        r,
        temper_substrate::embed::EMBED_CHUNK_BUDGET,
    )
    .await
    .unwrap();
    assert_eq!(
        progress.embedded, 1,
        "the live same-hash chunk embeds — under the retired hash-only gate this would be \
         0: membership alone excluded every chunk carrying the hash"
    );
    assert_eq!(
        progress.remaining, 0,
        "and the resource CONVERGES: the wiped chunk is not work, so the drain does not \
         re-enqueue it every minute forever (the wedge)"
    );
    let (emb, prov): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT embedding::text, embedded_with FROM kb_chunks WHERE id = $1")
            .bind(wiped)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (emb, prov),
        (None, None),
        "the wiped chunk keeps the erasure's emptied shape — no inference, no vector, not \
         even the blank-path provenance stamp may land on a redacted row"
    );
    let (live_emb, _): (Option<String>, Option<String>) =
        sqlx::query_as("SELECT embedding::text, embedded_with FROM kb_chunks WHERE id = $1")
            .bind(live)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        live_emb.is_some(),
        "the live same-hash chunk got its vector"
    );
}

/// The differential half, on both copies of the stale predicate: a live same-hash chunk is
/// still drain work (counted, enqueued), and a resource whose ONLY chunk is wiped drops out
/// of the enqueue sweep entirely. Both mixed-resource chunks carry the SAME erased hash —
/// the bite: under the retired hash-only gate the stale summary would count (0, 0) and
/// enqueue nothing.
#[sqlx::test(migrations = "../../migrations")]
async fn a_wiped_chunk_is_not_work_and_a_live_same_hash_chunk_is(pool: PgPool) {
    // r_mixed: one WIPED chunk (emptied shape) + one LIVE chunk — SAME hash, both shapes.
    let r_mixed = a_named_resource(&pool, "mixed").await;
    let b_mixed = a_block(&pool, r_mixed, 0, false).await;
    let erased = "bb0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    a_prose_chunk(&pool, b_mixed, r_mixed, 0, erased, "").await;
    a_prose_chunk(
        &pool,
        b_mixed,
        r_mixed,
        1,
        erased,
        "live prose under the erased hash",
    )
    .await;
    erase_hash(&pool, erased).await;

    // r_wiped_only: a wiped chunk is the ONLY content.
    let r_only = a_named_resource(&pool, "wiped-only").await;
    let b_only = a_block(&pool, r_only, 0, false).await;
    a_prose_chunk(&pool, b_only, r_only, 0, erased, "").await;

    let (mixed_stale, chunk_stale) = stale_summary(&pool, ReembedScope::All).await.unwrap();
    assert_eq!(
        (mixed_stale, chunk_stale),
        (1, 1),
        "the mixed resource is still work — and ONLY on its live same-hash chunk: the wiped \
         chunk is out of the stale predicate, so count and embed stay in lockstep (under \
         the retired hash-only gate this would be (0, 0))"
    );

    let enqueued = enqueue_stale(
        &pool,
        &crate::test_support::system_admin_proof(&pool).await,
        ReembedScope::All,
        100,
    )
    .await
    .unwrap();
    assert_eq!(
        enqueued,
        vec![r_mixed],
        "the mixed resource is enqueued for its live chunk; the wiped-only resource is not \
         enqueued at all — there is nothing the drain may lawfully do for it"
    );

    // And the drain of the mixed resource lands the live chunk's vector while the wiped
    // chunk stays exactly as erasure left it.
    let progress = temper_substrate::embed::embed_resource_chunks(
        &pool,
        r_mixed,
        temper_substrate::embed::EMBED_CHUNK_BUDGET,
    )
    .await
    .unwrap();
    assert_eq!(
        progress.remaining, 0,
        "the mixed resource converges on its live chunk's completion"
    );
    let (wiped_stamped, live_stamped): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE cc.content = ''), \
                count(*) FILTER (WHERE cc.content <> '') \
           FROM kb_chunks ch \
           JOIN kb_chunk_content cc ON cc.chunk_id = ch.id \
          WHERE ch.content_hash = $1 \
            AND (ch.embedding IS NOT NULL OR ch.embedded_with IS NOT NULL)",
    )
    .bind(erased)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        wiped_stamped, 0,
        "the wiped chunk stays vectorless and unstamped"
    );
    assert_eq!(
        live_stamped, 1,
        "the live same-hash chunk is stamped — membership alone never blocked it"
    );
}
