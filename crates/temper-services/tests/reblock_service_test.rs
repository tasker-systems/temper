//! Integration test — corpus adoption through the real `DbBackend` (spec 2026-09-11, chunk 1).
//! One behavior per witness: a full pass over a conformed context is ledger-silent at batch
//! grain (w3's shape, scaled); the cursor resumes without overlap or gaps; an out-of-grant row
//! declines `denied` while the rest of the batch completes and fires nothing; every event the
//! batch fires carries the batch correlation id; the dry run routes candidates to the
//! survey arm — the same classes, zero events; and the receipt counts the scope's
//! still-arriving (`in_progress`) population that the complete-only enumeration skips.
//!
//! Resources are seeded through substrate `fire` directly (real chunk rows + verbatim bytes,
//! ONNX-free — caller-supplied embeddings are never read by the op), because the write path
//! partitions bodies at create: the would-change shape (one block, two sections) is only
//! reachable the way the adoption tooling faces it.
#![cfg(feature = "test-db")]

use sqlx::PgPool;

use temper_core::types::ids::ProfileId;
use temper_core::types::reblock::ReblockOutcome;
use temper_core::types::reblock::ReblockScope;
use temper_services::backend::DbBackend;
use temper_substrate::content::{prepare_block_from_chunks, IncomingChunk};
use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::{EntityId, ProfileId as SubstrateProfileId};
use temper_substrate::payloads::AnchorRef;
use temper_workflow::operations::{Backend, ReblockResources, Surface};
use uuid::Uuid;

const SECTION_A: &str = "# Alpha\n\nAlpha body paragraph.\n";
const BODY_A_B: &str = "# Alpha\n\nAlpha body paragraph.\n## Beta\n\nBeta body paragraph.\n";
const FOREIGN_BODY: &str = "# Foreign\n\nTotally different prose that chunks elsewhere.\n";

// ── fixtures (duplicated per file, per this suite's convention) ─────────────────────────────

/// Seed a substrate profile + per-surface emitter entities + a profile-owned context — the
/// minimum `resolve_emitter` + the visibility gate require. Returns (profile, context, @web
/// entity id) — the entity doubles as the fixtures' emitter.
async fn seed_profile_with_context(pool: &PgPool, email: &str) -> (Uuid, Uuid, Uuid) {
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
    let mut web_entity = Uuid::now_v7();
    for surface in ["web", "cli", "mcp"] {
        let id = Uuid::now_v7();
        if surface == "web" {
            web_entity = id;
        }
        sqlx::query(
            "INSERT INTO kb_entities (id, profile_id, name, metadata) VALUES ($1,$2,$3,'{}'::jsonb)",
        )
        .bind(id)
        .bind(profile_id)
        .bind(format!("{handle}@{surface}"))
        .execute(pool)
        .await
        .expect("seed emitter entity");
    }
    let context_id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,'temper','temper')",
    )
    .bind(context_id)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("seed context");
    (profile_id, context_id, web_entity)
}

/// Real chunk hashes from the real chunker; the embedding is a placeholder the op never reads.
fn incoming_of(text: &str) -> Vec<IncomingChunk> {
    temper_ingest::chunk::chunk_markdown(text)
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

/// The shape knobs of a directly-fired block resource (see `fire_shaped_block_resource`) — a
/// params struct so the helper stays within the argument-count budget.
struct ShapedBlockResource<'a> {
    owner: Uuid,
    emitter: Uuid,
    context: Uuid,
    title: &'a str,
    /// The block's stored verbatim bytes (`None` = a derived shape storing no bytes).
    raw_text: Option<&'a str>,
    /// The chunk rows the block carries.
    chunks: Vec<IncomingChunk>,
    /// Birth the resource `in_progress` (a segmented begin whose finalize never came).
    segmented: bool,
}

/// One block with real chunk rows, fired directly (no policy application — the create-path hook
/// partitions bodies at create, so these shapes are reachable only by direct invocation, exactly
/// as the adoption tooling faces them).
async fn fire_shaped_block_resource(pool: &PgPool, shape: ShapedBlockResource<'_>) -> Uuid {
    let mut block = prepare_block_from_chunks(0, None, shape.chunks);
    block.raw_text = shape.raw_text.map(str::to_string);
    let blocks = [block];
    let mut conn = pool.acquire().await.unwrap();
    fire(
        &mut conn,
        SeedAction::ResourceCreate {
            title: shape.title,
            origin_uri: &format!("temper://reblock/{}", shape.title),
            resource_id: None,
            home: AnchorRef::context(temper_substrate::ids::ContextId::from(shape.context)),
            owner: SubstrateProfileId::from(shape.owner),
            originator: Some(SubstrateProfileId::from(shape.owner)),
            blocks: &blocks,
            doc_type: Some("concept"),
            emitter: EntityId::from(shape.emitter),
            segmented: shape.segmented,
        },
    )
    .await
    .unwrap()
    .resource()
    .unwrap()
    .uuid()
}

/// One block with stored verbatim bytes and real chunk rows, no policy application. A
/// one-section body is already policy-conformed; `BODY_A_B` (two sections in one block)
/// would-change.
async fn fire_block_resource(
    pool: &PgPool,
    owner: Uuid,
    emitter: Uuid,
    context: Uuid,
    title: &str,
    body: &str,
) -> Uuid {
    fire_shaped_block_resource(
        pool,
        ShapedBlockResource {
            owner,
            emitter,
            context,
            title,
            raw_text: Some(body),
            chunks: incoming_of(body),
            segmented: false,
        },
    )
    .await
}

async fn grant_resource(
    pool: &PgPool,
    resource: Uuid,
    profile: Uuid,
    granted_by: Uuid,
    can_read: bool,
    can_write: bool,
) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (subject_table, subject_id, principal_table, principal_id, can_read, can_write, granted_by_profile_id) \
         VALUES ('kb_resources', $1, 'kb_profiles', $2, $3, $4, $5)",
    )
    .bind(resource)
    .bind(profile)
    .bind(can_read)
    .bind(can_write)
    .bind(granted_by)
    .execute(pool)
    .await
    .unwrap();
}

async fn event_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn reblocked_event_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id \
          WHERE t.name='resource_reblocked'",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

/// (id, seq, is_folded, current_revision_id) of the live blocks, seq order — the projection a
/// declined row must not move.
async fn blocks_of(pool: &PgPool, resource: Uuid) -> Vec<(Uuid, i32, bool, Uuid)> {
    sqlx::query_as(
        "SELECT id, seq, is_folded, current_revision_id \
           FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded ORDER BY seq, id",
    )
    .bind(resource)
    .fetch_all(pool)
    .await
    .unwrap()
}

fn reblock_cmd(
    scope: ReblockScope,
    dry_run: bool,
    limit: i64,
    after_id: Option<Uuid>,
) -> ReblockResources {
    ReblockResources {
        scope,
        dry_run,
        limit,
        after_id,
        origin: Surface::ApiHttp,
    }
}

// ── the witnesses ────────────────────────────────────────────────────────────────────────────

/// (b) Batch-grain no-op silence: a full pass over an already-conformed context fires ZERO
/// events of any kind — the batch is ledger-indistinguishable from never having run. The
/// receipt's no-op count is a survey-time fact, never a ledger one.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_full_pass_over_a_conformed_context_fires_nothing(pool: PgPool) {
    let (owner, context, entity) = seed_profile_with_context(&pool, "owner@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(owner));
    for i in 0..3 {
        fire_block_resource(
            &pool,
            owner,
            entity,
            context,
            &format!("conformed-{i}"),
            SECTION_A,
        )
        .await;
    }

    let events_before = event_count(&pool).await;
    let receipt = backend
        .reblock_resources(reblock_cmd(ReblockScope::Context(context), false, 10, None))
        .await
        .unwrap()
        .value;

    assert_eq!(receipt.outcomes.len(), 3, "every candidate produced a row");
    assert_eq!(receipt.summary.no_op, 3, "all conformed rows no-op");
    assert_eq!(receipt.summary.reblocked, 0);
    assert_eq!(
        reblocked_event_count(&pool).await,
        0,
        "a conformed pass fires zero resource_reblocked events"
    );
    assert_eq!(
        event_count(&pool).await,
        events_before,
        "a conformed pass fires nothing AT ALL — no batch-level event exists"
    );
}

/// (c) Cursor continuation: bounded batches threaded through the returned `after_id` cover the
/// scope exactly once — no overlaps, no gaps, and an exhausted scope returns an empty page.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_cursor_resumes_without_overlap_or_gaps(pool: PgPool) {
    let (owner, context, entity) = seed_profile_with_context(&pool, "owner@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(owner));
    let mut seeded = Vec::new();
    for i in 0..5 {
        seeded.push(
            fire_block_resource(
                &pool,
                owner,
                entity,
                context,
                &format!("cursor-{i}"),
                SECTION_A,
            )
            .await,
        );
    }
    seeded.sort();

    let mut seen: Vec<Uuid> = Vec::new();
    let mut after: Option<Uuid> = None;
    for page in 0..3 {
        let receipt = backend
            .reblock_resources(reblock_cmd(ReblockScope::Context(context), false, 2, after))
            .await
            .unwrap()
            .value;
        assert!(
            receipt.outcomes.len() <= 2,
            "page {page} exceeded its bound"
        );
        let page_ids: Vec<Uuid> = receipt.outcomes.iter().map(|o| o.resource).collect();
        assert_eq!(
            receipt.after_id,
            page_ids.last().copied(),
            "the cursor is the last candidate id considered"
        );
        seen.extend(page_ids);
        after = receipt.after_id;
    }
    let tail = backend
        .reblock_resources(reblock_cmd(ReblockScope::Context(context), false, 2, after))
        .await
        .unwrap()
        .value;
    assert!(
        tail.outcomes.is_empty(),
        "an exhausted scope returns an empty page, got {:?}",
        tail.outcomes
    );

    seen.sort();
    assert_eq!(seen, seeded, "batches tile the scope: no overlaps, no gaps");
    let mut deduped = seen.clone();
    deduped.dedup();
    assert_eq!(deduped, seen, "no candidate was considered twice");
}

/// (d) Gate denial arm: an out-of-grant row declines `denied` per-row while the rest of the
/// batch completes, and the denied act leaves zero events — the grant boundary doing its work,
/// auditable in the receipt, never a batch abort.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_out_of_grant_row_declines_denied_while_the_batch_completes(pool: PgPool) {
    let (owner, context, entity) = seed_profile_with_context(&pool, "owner@example.com").await;
    let (reader, _rctx, _re) = seed_profile_with_context(&pool, "reader@example.com").await;
    let changing =
        fire_block_resource(&pool, owner, entity, context, "out-of-grant", BODY_A_B).await;
    let modifiable = fire_block_resource(&pool, owner, entity, context, "in-grant", BODY_A_B).await;
    // The reader sees both (direct resource read grants) but may modify only one.
    grant_resource(&pool, changing, reader, owner, true, false).await;
    grant_resource(&pool, modifiable, reader, owner, true, true).await;

    let blocks_before = blocks_of(&pool, changing).await;
    let reader_backend = DbBackend::new(pool.clone(), ProfileId::from(reader));
    let receipt = reader_backend
        .reblock_resources(reblock_cmd(ReblockScope::Context(context), false, 10, None))
        .await
        .unwrap()
        .value;

    assert_eq!(
        receipt.outcomes.len(),
        2,
        "both visible candidates considered"
    );
    for row in &receipt.outcomes {
        if row.resource == changing {
            assert!(
                matches!(&row.outcome, ReblockOutcome::Denied),
                "the out-of-grant row declines denied, got {:?}",
                row.outcome
            );
        } else if row.resource == modifiable {
            assert!(
                matches!(&row.outcome, ReblockOutcome::Reblocked { .. }),
                "the in-grant row completes, got {:?}",
                row.outcome
            );
        }
    }
    assert_eq!(receipt.summary.declined, 1);
    assert_eq!(receipt.summary.reblocked, 1);
    assert_eq!(
        reblocked_event_count(&pool).await,
        1,
        "only the granted act reached the ledger"
    );
    assert_eq!(
        blocks_of(&pool, changing).await,
        blocks_before,
        "the denied act left the out-of-grant row's partition untouched"
    );
}

/// (f) Correlation grouping: every event the batch fires carries the batch correlation id, and
/// the fired emitter is the invoking operator's own per-surface entity — the batch mints no
/// system actor.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_fired_event_carries_the_batch_correlation(pool: PgPool) {
    let (owner, context, web_entity) = seed_profile_with_context(&pool, "owner@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(owner));
    let changing =
        fire_block_resource(&pool, owner, web_entity, context, "correlated", BODY_A_B).await;
    fire_block_resource(&pool, owner, web_entity, context, "silent", SECTION_A).await;

    let receipt = backend
        .reblock_resources(reblock_cmd(ReblockScope::Context(context), false, 10, None))
        .await
        .unwrap()
        .value;

    assert_eq!(receipt.summary.reblocked, 1);
    let (count, emitters): (i64, Vec<Uuid>) = sqlx::query_as(
        "SELECT count(*), coalesce(array_agg(e.emitter_entity_id), '{}') \
           FROM kb_events e \
           JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE e.correlation_id = $1 AND t.name = 'resource_reblocked'",
    )
    .bind(receipt.correlation_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        count as u64, receipt.summary.reblocked,
        "the ledger holds exactly the acts the receipt claims, under the batch id"
    );
    assert_eq!(
        emitters,
        vec![web_entity],
        "the act rides the invoking operator's emitter, not a system actor"
    );
    let anchored: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e \
          JOIN kb_event_types t ON t.id = e.event_type_id \
         WHERE e.correlation_id = $1 AND t.name = 'resource_reblocked' \
           AND e.payload->>'resource_id' = $2",
    )
    .bind(receipt.correlation_id)
    .bind(changing.to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        anchored, 1,
        "the grouped act is the re-block of the changed row"
    );
}

/// Dry-run routing: the same candidates classify through the survey arm — would-change and
/// no-op rows match what the subsequent act then does — and the dry pass fires nothing.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_dry_run_surveys_without_touching(pool: PgPool) {
    let (owner, context, entity) = seed_profile_with_context(&pool, "owner@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(owner));
    let changing =
        fire_block_resource(&pool, owner, entity, context, "dry-changing", BODY_A_B).await;
    let conformed =
        fire_block_resource(&pool, owner, entity, context, "dry-conformed", SECTION_A).await;

    let events_before = event_count(&pool).await;
    let survey = backend
        .reblock_resources(reblock_cmd(ReblockScope::Context(context), true, 10, None))
        .await
        .unwrap()
        .value;
    assert_eq!(
        event_count(&pool).await,
        events_before,
        "the dry run is read-only"
    );
    assert!(survey.dry_run);
    for row in &survey.outcomes {
        if row.resource == changing {
            assert!(
                matches!(row.outcome, ReblockOutcome::WouldChange),
                "got {:?}",
                row.outcome
            );
        } else if row.resource == conformed {
            assert!(
                matches!(row.outcome, ReblockOutcome::NoOp),
                "got {:?}",
                row.outcome
            );
        }
    }
    assert_eq!(survey.summary.would_change, 1);
    assert_eq!(survey.summary.no_op, 1);

    // The act then does what the survey said, per row.
    let acted = backend
        .reblock_resources(reblock_cmd(ReblockScope::Context(context), false, 10, None))
        .await
        .unwrap()
        .value;
    for row in &acted.outcomes {
        if row.resource == changing {
            assert!(
                matches!(row.outcome, ReblockOutcome::Reblocked { .. }),
                "got {:?}",
                row.outcome
            );
        } else if row.resource == conformed {
            assert!(
                matches!(row.outcome, ReblockOutcome::NoOp),
                "got {:?}",
                row.outcome
            );
        }
    }
}

/// The receipt names the scope's still-arriving population: enumeration is complete-only (the
/// op refuses `in_progress` rows, so the window is never spent on guaranteed declines), so a
/// still-arriving sibling homed in the same context produces no outcome row — the summary's
/// `in_progress` count keeps it visible to the operator, and the cursor stays on the last
/// candidate actually considered.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_context_receipt_counts_its_still_arriving_uploads(pool: PgPool) {
    let (owner, context, entity) = seed_profile_with_context(&pool, "owner@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(owner));
    let complete = fire_block_resource(&pool, owner, entity, context, "complete", BODY_A_B).await;
    let arriving = fire_shaped_block_resource(
        &pool,
        ShapedBlockResource {
            owner,
            emitter: entity,
            context,
            title: "arriving",
            raw_text: Some(SECTION_A),
            chunks: incoming_of(SECTION_A),
            segmented: true,
        },
    )
    .await;

    let receipt = backend
        .reblock_resources(reblock_cmd(ReblockScope::Context(context), false, 10, None))
        .await
        .unwrap()
        .value;

    assert_eq!(
        receipt.outcomes.len(),
        1,
        "only the complete row is a candidate"
    );
    assert_eq!(receipt.outcomes[0].resource, complete);
    assert!(
        !receipt.outcomes.iter().any(|row| row.resource == arriving),
        "the still-arriving sibling appears in no outcome row"
    );
    assert_eq!(
        receipt.summary.in_progress, 1,
        "the receipt names the still-arriving sibling the outcomes omit"
    );
    assert_eq!(
        receipt.after_id,
        Some(complete),
        "the cursor is the last candidate id considered"
    );
}

// ── error-row witnesses (receipt grain — the bounded-sentence disclosure invariant) ──────────

/// The one bounded row-error sentence the receipt ships (`DbBackend::reblock_resources`'s row
/// helper). Asserted verbatim: the receipt is client-facing contract text, so the oracle here is
/// the sentence itself, never a substring of whatever the internal error displayed.
const ROW_ERROR_MESSAGE: &str = "internal error while processing this candidate; retry it alone";

/// Corrupt one live block's provenance: extend the closed `provenance_source_kind` enum with a
/// value the Rust reader does not map, and attach a provenance row carrying it. The reblock
/// op's shared classification half (`read_attributions`) bails on the unmapped kind — the
/// canonical row-error the receipt arms must survive without leaking. Runs in this test's own
/// database (`sqlx::test`), so the type extension cannot leak past the witness.
async fn poison_attributions(pool: &PgPool, resource: Uuid) {
    sqlx::query("ALTER TYPE provenance_source_kind ADD VALUE IF NOT EXISTS 'probe_unknown'")
        .execute(pool)
        .await
        .expect("extend the enum with a value the reader does not map");
    let (block, event): (Uuid, Uuid) = sqlx::query_as(
        "SELECT b.id, \
            (SELECT e.id FROM kb_events e ORDER BY e.id DESC LIMIT 1) \
           FROM kb_content_blocks b \
          WHERE b.resource_id = $1 AND NOT b.is_folded \
          ORDER BY b.seq, b.id LIMIT 1",
    )
    .bind(resource)
    .fetch_one(pool)
    .await
    .expect("a live block to poison");
    sqlx::query(
        "INSERT INTO kb_block_provenance \
           (block_id, source_kind, source_id, contributed_by_event_id, accretion_seq, is_corrected) \
         VALUES ($1, $2::text::provenance_source_kind, $3, $4, 99, false)",
    )
    .bind(block)
    .bind("probe_unknown")
    .bind(Uuid::now_v7())
    .bind(event)
    .execute(pool)
    .await
    .expect("seed the unmapped provenance row");
}

/// (g) A candidate whose data poisons the op itself (a provenance row carrying a source kind the
/// reader does not map) errors as a RECEIPT ROW carrying exactly the bounded sentence — never
/// the raw internal text (SQL/PG/provenance internals) — while the batch declines-and-continues:
/// the healthy sibling still reblocks and reaches the ledger. Per-row diagnostics are log-only,
/// joinable by the correlation id the receipt already carries.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_poisoned_candidate_errors_as_a_bounded_row_while_the_batch_completes(pool: PgPool) {
    let (owner, context, entity) = seed_profile_with_context(&pool, "owner@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(owner));
    let poisoned = fire_block_resource(&pool, owner, entity, context, "poisoned", SECTION_A).await;
    let healthy = fire_block_resource(&pool, owner, entity, context, "healthy", BODY_A_B).await;
    poison_attributions(&pool, poisoned).await;

    let receipt = backend
        .reblock_resources(reblock_cmd(ReblockScope::Context(context), false, 10, None))
        .await
        .expect("an errored row never aborts the batch")
        .value;

    assert_eq!(receipt.outcomes.len(), 2, "both candidates produced a row");
    for row in &receipt.outcomes {
        if row.resource == poisoned {
            assert_eq!(
                row.outcome,
                ReblockOutcome::Error {
                    message: ROW_ERROR_MESSAGE.to_owned()
                },
                "the poisoned row errors with the bounded sentence, got {:?}",
                row.outcome
            );
        } else if row.resource == healthy {
            assert!(
                matches!(row.outcome, ReblockOutcome::Reblocked { .. }),
                "the healthy row completes, got {:?}",
                row.outcome
            );
        }
    }
    assert_eq!(receipt.summary.error, 1);
    assert_eq!(receipt.summary.reblocked, 1);
    assert_eq!(
        reblocked_event_count(&pool).await,
        1,
        "the batch declined-and-continued: the healthy act reached the ledger"
    );
}

/// (h) The same poison under `dry_run`: the survey arm takes the SAME bounded row-error path —
/// the invocation answers with a 200-shaped receipt carrying the error row, never an aborted
/// invocation, and the dry pass stays read-only. One poisoned candidate must not make every
/// survey of its context fail wholesale while the act on the same context proceeds.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_poisoned_candidate_errors_in_the_survey_instead_of_aborting_the_invocation(
    pool: PgPool,
) {
    let (owner, context, entity) = seed_profile_with_context(&pool, "owner@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(owner));
    let poisoned = fire_block_resource(&pool, owner, entity, context, "poisoned", SECTION_A).await;
    let healthy = fire_block_resource(&pool, owner, entity, context, "healthy", BODY_A_B).await;
    poison_attributions(&pool, poisoned).await;

    let events_before = event_count(&pool).await;
    let receipt = backend
        .reblock_resources(reblock_cmd(ReblockScope::Context(context), true, 10, None))
        .await
        .expect("an errored survey row is a receipt row, not an aborted invocation")
        .value;

    assert!(receipt.dry_run);
    assert_eq!(receipt.outcomes.len(), 2, "both candidates produced a row");
    for row in &receipt.outcomes {
        if row.resource == poisoned {
            assert_eq!(
                row.outcome,
                ReblockOutcome::Error {
                    message: ROW_ERROR_MESSAGE.to_owned()
                },
                "the poisoned row errors with the bounded sentence, got {:?}",
                row.outcome
            );
        } else if row.resource == healthy {
            assert!(
                matches!(row.outcome, ReblockOutcome::WouldChange),
                "the healthy row still surveys, got {:?}",
                row.outcome
            );
        }
    }
    assert_eq!(receipt.summary.error, 1);
    assert_eq!(receipt.summary.would_change, 1);
    assert_eq!(
        event_count(&pool).await,
        events_before,
        "the dry run stays read-only"
    );
}

// ── decline witnesses (per class, receipt grain) ─────────────────────────────────────────────

/// A still-arriving candidate (`in_progress` — a segmented begin whose finalize never came)
/// declines `InProgress` with its human remediation riding along. The addressed-scope arm has no
/// ingest-state filter, so the op's own state-column refusal reaches the receipt.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_in_progress_candidate_declines_in_progress(pool: PgPool) {
    let (owner, context, entity) = seed_profile_with_context(&pool, "owner@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(owner));
    let arriving = fire_shaped_block_resource(
        &pool,
        ShapedBlockResource {
            owner,
            emitter: entity,
            context,
            title: "arriving",
            raw_text: Some(SECTION_A),
            chunks: incoming_of(SECTION_A),
            segmented: true,
        },
    )
    .await;

    let receipt = backend
        .reblock_resources(reblock_cmd(
            ReblockScope::Resource(arriving),
            false,
            1,
            None,
        ))
        .await
        .unwrap()
        .value;

    assert_eq!(receipt.outcomes.len(), 1, "the candidate produced a row");
    assert!(
        matches!(
            &receipt.outcomes[0].outcome,
            ReblockOutcome::InProgress { detail }
                if detail.contains("mid-ingest")
        ),
        "the still-arriving row declines in_progress, got {:?}",
        receipt.outcomes[0].outcome
    );
    assert_eq!(receipt.summary.declined, 1);
}

/// A derived-shape candidate (chunk rows but no stored verbatim bytes) declines `Byteless` —
/// there are no stored bytes to compose a body from.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_derived_shape_candidate_declines_byteless(pool: PgPool) {
    let (owner, context, entity) = seed_profile_with_context(&pool, "owner@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(owner));
    let byteless = fire_shaped_block_resource(
        &pool,
        ShapedBlockResource {
            owner,
            emitter: entity,
            context,
            title: "byteless",
            raw_text: None,
            chunks: incoming_of(SECTION_A),
            segmented: false,
        },
    )
    .await;

    let receipt = backend
        .reblock_resources(reblock_cmd(
            ReblockScope::Resource(byteless),
            false,
            1,
            None,
        ))
        .await
        .unwrap()
        .value;

    assert_eq!(receipt.outcomes.len(), 1, "the candidate produced a row");
    assert!(
        matches!(
            &receipt.outcomes[0].outcome,
            ReblockOutcome::Byteless { detail }
                if detail.contains("verbatim bytes")
        ),
        "the derived-shape row declines byteless, got {:?}",
        receipt.outcomes[0].outcome
    );
    assert_eq!(receipt.summary.declined, 1);
}

/// A drifted candidate (chunks cut from different text than the stored verbatim bytes) declines
/// `Drift` — a fresh chunking of the body does not reproduce the stored chunking.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_drifted_candidate_declines_drift(pool: PgPool) {
    let (owner, context, entity) = seed_profile_with_context(&pool, "owner@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(owner));
    let drifted = fire_shaped_block_resource(
        &pool,
        ShapedBlockResource {
            owner,
            emitter: entity,
            context,
            title: "drifted",
            raw_text: Some(BODY_A_B),
            chunks: incoming_of(FOREIGN_BODY),
            segmented: false,
        },
    )
    .await;

    let receipt = backend
        .reblock_resources(reblock_cmd(ReblockScope::Resource(drifted), false, 1, None))
        .await
        .unwrap()
        .value;

    assert_eq!(receipt.outcomes.len(), 1, "the candidate produced a row");
    assert!(
        matches!(
            &receipt.outcomes[0].outcome,
            ReblockOutcome::Drift { detail }
                if detail.contains("does not reproduce")
        ),
        "the drifted row declines drift, got {:?}",
        receipt.outcomes[0].outcome
    );
    assert_eq!(receipt.summary.declined, 1);
}
