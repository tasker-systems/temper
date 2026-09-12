//! Integration test — corpus adoption through the real `DbBackend` (spec 2026-09-11, chunk 1).
//! One behavior per witness: a full pass over a conformed context is ledger-silent at batch
//! grain (w3's shape, scaled); the cursor resumes without overlap or gaps; an out-of-grant row
//! declines `denied` while the rest of the batch completes and fires nothing; every event the
//! batch fires carries the batch correlation id; and the dry run routes candidates to the
//! survey arm — the same classes, zero events.
//!
//! Resources are seeded through substrate `fire` directly (real chunk rows + verbatim bytes,
//! ONNX-free — caller-supplied embeddings are never read by the op), because the write path
//! partitions bodies at create: the would-change shape (one block, two sections) is only
//! reachable the way the adoption tooling faces it.
#![cfg(feature = "test-db")]

use sqlx::PgPool;

use temper_core::types::adoption::{AdoptDeclined, AdoptOutcome, AdoptScope};
use temper_core::types::ids::ProfileId;
use temper_services::backend::DbBackend;
use temper_substrate::content::{prepare_block_from_chunks, IncomingChunk};
use temper_substrate::events::{fire, SeedAction};
use temper_substrate::ids::{EntityId, ProfileId as SubstrateProfileId};
use temper_substrate::payloads::AnchorRef;
use temper_workflow::operations::{AdoptResources, Backend, Surface};
use uuid::Uuid;

const SECTION_A: &str = "# Alpha\n\nAlpha body paragraph.\n";
const BODY_A_B: &str = "# Alpha\n\nAlpha body paragraph.\n## Beta\n\nBeta body paragraph.\n";

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

/// One block with stored verbatim bytes and real chunk rows, no policy application (the
/// create-path hook partitions bodies, so this shape is reachable only by direct invocation —
/// exactly as the adoption tooling faces it). A one-section body is already policy-conformed;
/// `BODY_A_B` (two sections in one block) would-change.
async fn fire_block_resource(
    pool: &PgPool,
    owner: Uuid,
    emitter: Uuid,
    context: Uuid,
    title: &str,
    body: &str,
) -> Uuid {
    let mut block = prepare_block_from_chunks(0, None, incoming_of(body));
    block.raw_text = Some(body.to_string());
    let blocks = [block];
    let mut conn = pool.acquire().await.unwrap();
    fire(
        &mut conn,
        SeedAction::ResourceCreate {
            title,
            origin_uri: &format!("temper://adoption/{title}"),
            resource_id: None,
            home: AnchorRef::context(temper_substrate::ids::ContextId::from(context)),
            owner: SubstrateProfileId::from(owner),
            originator: Some(SubstrateProfileId::from(owner)),
            blocks: &blocks,
            doc_type: Some("concept"),
            emitter: EntityId::from(emitter),
            segmented: false,
        },
    )
    .await
    .unwrap()
    .resource()
    .unwrap()
    .uuid()
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

fn adopt_cmd(
    scope: AdoptScope,
    dry_run: bool,
    limit: i64,
    after_id: Option<Uuid>,
) -> AdoptResources {
    AdoptResources {
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
        .adopt_resources(adopt_cmd(AdoptScope::Context(context), false, 10, None))
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
            .adopt_resources(adopt_cmd(AdoptScope::Context(context), false, 2, after))
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
        .adopt_resources(adopt_cmd(AdoptScope::Context(context), false, 2, after))
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
        .adopt_resources(adopt_cmd(AdoptScope::Context(context), false, 10, None))
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
                matches!(&row.outcome, AdoptOutcome::Declined(AdoptDeclined::Denied)),
                "the out-of-grant row declines denied, got {:?}",
                row.outcome
            );
        } else if row.resource == modifiable {
            assert!(
                matches!(&row.outcome, AdoptOutcome::Reblocked { .. }),
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
        .adopt_resources(adopt_cmd(AdoptScope::Context(context), false, 10, None))
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
        .adopt_resources(adopt_cmd(AdoptScope::Context(context), true, 10, None))
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
                matches!(row.outcome, AdoptOutcome::WouldChange),
                "got {:?}",
                row.outcome
            );
        } else if row.resource == conformed {
            assert!(
                matches!(row.outcome, AdoptOutcome::NoOp),
                "got {:?}",
                row.outcome
            );
        }
    }
    assert_eq!(survey.summary.would_change, 1);
    assert_eq!(survey.summary.no_op, 1);

    // The act then does what the survey said, per row.
    let acted = backend
        .adopt_resources(adopt_cmd(AdoptScope::Context(context), false, 10, None))
        .await
        .unwrap()
        .value;
    for row in &acted.outcomes {
        if row.resource == changing {
            assert!(
                matches!(row.outcome, AdoptOutcome::Reblocked { .. }),
                "got {:?}",
                row.outcome
            );
        } else if row.resource == conformed {
            assert!(
                matches!(row.outcome, AdoptOutcome::NoOp),
                "got {:?}",
                row.outcome
            );
        }
    }
}
