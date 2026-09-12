#![cfg(feature = "artifact-tests")]
//! Witnesses for the read-only re-block survey (corpus adoption, spec 2026-09-11 chunk 1).
//! The survey is the act's own machinery minus the write arm: it composes the body from stored
//! verbatim bytes, computes the partition, and classifies — touching nothing. Two behaviors:
//! the survey's class is the subsequent act's outcome, per resource, for every classification
//! (the differential — the two arms share ONE computation, so they cannot drift), and a survey
//! pass is ledger- and projection-inert (it reads, it never writes).

mod common;

use temper_ingest::chunk::chunk_markdown;
use temper_substrate::content::{
    prepare_block_from_chunks, prepare_block_with_prefix, IncomingChunk,
};
use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::{EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{
    self, CreateMode, CreateParams, ReblockOutcome, ReblockParams, ReblockSurvey,
};
use uuid::Uuid;

const SECTION_A: &str = "# Alpha\n\nAlpha body paragraph.\n";
const BODY_A_B: &str = "# Alpha\n\nAlpha body paragraph.\n## Beta\n\nBeta body paragraph.\n";

// ── fixture helpers (duplicated per file, per this suite's convention) ──────────────────────

async fn system_actor(pool: &sqlx::PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: Uuid =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    (ProfileId::from(profile), EntityId::from(entity))
}

fn emitter_of(actor: &(ProfileId, EntityId)) -> EntityId {
    actor.1
}

async fn make_home(pool: &sqlx::PgPool, owner: ProfileId, slug: &str) -> AnchorRef {
    let ctx = common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
        .await
        .unwrap();
    AnchorRef::context(temper_substrate::ids::ContextId::from(ctx))
}

/// One block, N sections, stored verbatim bytes and real chunk rows — no policy application
/// (the create-path hook partitions bodies, so this shape is reachable only by direct
/// invocation, exactly as the adoption tooling faces it).
async fn fire_single_block_body_resource(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: &AnchorRef,
    title: &str,
    body: &str,
) -> ResourceId {
    let mut block = prepare_block_with_prefix(0, None, body, &[]).unwrap();
    block.raw_text = Some(body.to_string());
    let blocks = [block];
    let mut conn = pool.acquire().await.unwrap();
    fire(
        &mut conn,
        SeedAction::ResourceCreate {
            title,
            origin_uri: &format!("temper://reblock-survey/{title}"),
            resource_id: None,
            home: *home,
            owner,
            originator: Some(owner),
            blocks: &blocks,
            doc_type: Some("concept"),
            emitter,
            segmented: false,
        },
    )
    .await
    .unwrap()
    .resource()
    .unwrap()
}

fn incoming_of(text: &str) -> Vec<IncomingChunk> {
    chunk_markdown(text)
        .into_iter()
        .map(|c| IncomingChunk {
            chunk_index: c.chunk_index as i32,
            content_hash: c.content_hash,
            content: c.content,
            embedding: vec![0.0; 768],
            embedded_with: None,
            header_path: c.header_path.clone(),
            heading_depth: c.heading_depth as i16,
        })
        .collect()
}

/// One block whose chunks were cut from DIFFERENT text than the stored verbatim bytes: the
/// stored chunking cannot be reproduced from the body — the chunker-drift decline.
async fn fire_drifted_resource(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: &AnchorRef,
    body: &str,
) -> ResourceId {
    let foreign = "# Foreign\n\nTotally different prose that chunks elsewhere.\n";
    let mut block = prepare_block_from_chunks(0, None, incoming_of(foreign));
    block.raw_text = Some(body.to_string());
    let blocks = [block];
    let mut conn = pool.acquire().await.unwrap();
    fire(
        &mut conn,
        SeedAction::ResourceCreate {
            title: "drifted",
            origin_uri: "temper://reblock-survey/drifted",
            resource_id: None,
            home: *home,
            owner,
            originator: Some(owner),
            blocks: &blocks,
            doc_type: Some("concept"),
            emitter,
            segmented: false,
        },
    )
    .await
    .unwrap()
    .resource()
    .unwrap()
}

/// One block with chunk rows but NO stored verbatim bytes — the byteless (derived-shape) decline.
async fn fire_byteless_resource(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: &AnchorRef,
) -> ResourceId {
    let block = prepare_block_from_chunks(0, None, incoming_of(SECTION_A));
    let blocks = [block];
    let mut conn = pool.acquire().await.unwrap();
    fire(
        &mut conn,
        SeedAction::ResourceCreate {
            title: "byteless",
            origin_uri: "temper://reblock-survey/byteless",
            resource_id: None,
            home: *home,
            owner,
            originator: Some(owner),
            blocks: &blocks,
            doc_type: Some("concept"),
            emitter,
            segmented: false,
        },
    )
    .await
    .unwrap()
    .resource()
    .unwrap()
}

/// A segmented begin whose finalize never came — born and staying `in_progress`.
async fn create_in_progress_resource(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: &AnchorRef,
) -> ResourceId {
    writes::create_resource_with_mode(
        pool,
        CreateParams {
            title: "in progress",
            origin_uri: "temper://reblock-survey/in-progress",
            body: SECTION_A,
            doc_type: "concept",
            home: *home,
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
            sources: vec![],
            idempotency_key: None,
        },
        temper_substrate::events::EventContext::default(),
        CreateMode {
            defer: false,
            segmented: true,
        },
    )
    .await
    .unwrap()
}

/// (id, seq, is_folded, genesis_event_id, last_event_id, current_revision_id), seq order —
/// the live-block projection a survey must not disturb.
async fn blocks_of(
    pool: &sqlx::PgPool,
    resource: ResourceId,
) -> Vec<(Uuid, i32, bool, Uuid, Uuid, Uuid)> {
    sqlx::query_as(
        "SELECT id, seq, is_folded, genesis_event_id, last_event_id, current_revision_id \
           FROM kb_content_blocks WHERE resource_id=$1 ORDER BY seq, id",
    )
    .bind(resource.uuid())
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn chunks_of(pool: &sqlx::PgPool, resource: ResourceId) -> Vec<(Uuid, Uuid, String, bool)> {
    sqlx::query_as(
        "SELECT c.id, c.block_id, c.content_hash, c.is_current \
           FROM kb_chunks c JOIN kb_content_blocks b ON b.id=c.block_id \
          WHERE b.resource_id=$1 ORDER BY c.id",
    )
    .bind(resource.uuid())
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn event_count(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(pool)
        .await
        .unwrap()
}

// ── the witnesses ────────────────────────────────────────────────────────────────────────────

/// The differential: the survey's class IS the act's outcome, for every classification. The
/// two run the same computation — the survey may never classify a row differently than the act
/// then does (would-change → Reblocked; no-op → NoOp; declined{reason} → Declined{same reason}).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_survey_class_matches_the_subsequent_act_outcome(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let actor = system_actor(&pool).await;
    let emitter = emitter_of(&actor);

    // would-change: one block, two sections — the partition will move.
    let home_change = make_home(&pool, actor.0, "survey-change").await;
    let changing = fire_single_block_body_resource(
        &pool,
        actor.0,
        emitter,
        &home_change,
        "changing",
        BODY_A_B,
    )
    .await;
    // no-op: one block, one section — already the policy partition.
    let home_noop = make_home(&pool, actor.0, "survey-noop").await;
    let conformed = fire_single_block_body_resource(
        &pool,
        actor.0,
        emitter,
        &home_noop,
        "conformed",
        SECTION_A,
    )
    .await;
    // declined — drift: the stored chunking does not reproduce from the body.
    let home_drift = make_home(&pool, actor.0, "survey-drift").await;
    let drifted = fire_drifted_resource(&pool, actor.0, emitter, &home_drift, BODY_A_B).await;
    // declined — byteless: a block with no stored verbatim bytes (a derived shape).
    let home_byteless = make_home(&pool, actor.0, "survey-byteless").await;
    let byteless = fire_byteless_resource(&pool, actor.0, emitter, &home_byteless).await;
    // declined — in_progress: a still-arriving segmented begin.
    let home_ip = make_home(&pool, actor.0, "survey-in-progress").await;
    let arriving = create_in_progress_resource(&pool, actor.0, emitter, &home_ip).await;

    for (name, resource) in [
        ("would-change", changing),
        ("no-op", conformed),
        ("drift", drifted),
        ("byteless", byteless),
        ("in_progress", arriving),
    ] {
        let surveyed = writes::survey_reblock_resource(&pool, resource)
            .await
            .unwrap();
        let acted = writes::reblock_resource(&pool, ReblockParams { resource, emitter })
            .await
            .unwrap();
        match (&surveyed, &acted) {
            (ReblockSurvey::WouldChange, ReblockOutcome::Reblocked { .. }) => {}
            (ReblockSurvey::NoOp, ReblockOutcome::NoOp) => {}
            (
                ReblockSurvey::Declined {
                    reason: survey_reason,
                },
                ReblockOutcome::Declined { reason: act_reason },
            ) => {
                assert_eq!(
                    survey_reason, act_reason,
                    "{name}: the survey and the act must decline for the SAME reason"
                );
            }
            _ => panic!(
                "{name}: the survey class diverged from the act outcome: \
                 surveyed {surveyed:?}, acted {acted:?}"
            ),
        }
    }
}

/// The survey touches nothing: a pass over a population leaves the ledger and the block/chunk
/// projection byte-identical. It reads, it never writes.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_survey_pass_leaves_no_trace(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let actor = system_actor(&pool).await;
    let emitter = emitter_of(&actor);

    let home_change = make_home(&pool, actor.0, "trace-change").await;
    let changing = fire_single_block_body_resource(
        &pool,
        actor.0,
        emitter,
        &home_change,
        "changing",
        BODY_A_B,
    )
    .await;
    let home_noop = make_home(&pool, actor.0, "trace-noop").await;
    let conformed = fire_single_block_body_resource(
        &pool,
        actor.0,
        emitter,
        &home_noop,
        "conformed",
        SECTION_A,
    )
    .await;
    let home_drift = make_home(&pool, actor.0, "trace-drift").await;
    let drifted = fire_drifted_resource(&pool, actor.0, emitter, &home_drift, BODY_A_B).await;
    let home_byteless = make_home(&pool, actor.0, "trace-byteless").await;
    let byteless = fire_byteless_resource(&pool, actor.0, emitter, &home_byteless).await;
    let home_ip = make_home(&pool, actor.0, "trace-in-progress").await;
    let arriving = create_in_progress_resource(&pool, actor.0, emitter, &home_ip).await;

    let population = [changing, conformed, drifted, byteless, arriving];
    let events_before = event_count(&pool).await;
    type BlockState = Vec<(Uuid, i32, bool, Uuid, Uuid, Uuid)>;
    type ChunkState = Vec<(Uuid, Uuid, String, bool)>;
    let mut state_before: Vec<(ResourceId, BlockState, ChunkState)> = Vec::new();
    for resource in population {
        state_before.push((
            resource,
            blocks_of(&pool, resource).await,
            chunks_of(&pool, resource).await,
        ));
    }

    for resource in population {
        writes::survey_reblock_resource(&pool, resource)
            .await
            .unwrap();
    }

    assert_eq!(
        event_count(&pool).await,
        events_before,
        "a survey pass fires no event of any kind — it reads, it never writes"
    );
    for (resource, blocks_before, chunks_before) in state_before {
        assert_eq!(
            blocks_of(&pool, resource).await,
            blocks_before,
            "survey must not move the block projection of {resource}"
        );
        assert_eq!(
            chunks_of(&pool, resource).await,
            chunks_before,
            "survey must not move the chunk projection of {resource}"
        );
    }
}
