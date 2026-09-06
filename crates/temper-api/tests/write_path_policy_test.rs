#![cfg(feature = "test-db")]
//! Surface-grain witnesses for the write-path blocking policy (goal: Anchor addressability,
//! task `01a0721e-0590-7392-a94a-cdaae6292c74`).
//!
//! The substrate re-block op shipped in PR #860 with zero production callers; the write-path
//! wiring (this task) makes every body write land policy-partitioned through the EXISTING gate
//! train. These witnesses drive the real HTTP surface (`POST /api/ingest`, `PATCH
//! /api/resources/{id}`) and the DbBackend update command — the same doors production uses —
//! never the substrate op directly.
//!
//! Every body write here carries precomputed `chunks_packed` built with the substrate's REAL
//! chunker (`prepare_block`), packed with inert constant vectors. That is the CLI's exact
//! production shape (bring-your-own chunks, ONNX-free), and it is what lets a client-chunked
//! write satisfy the op's chunker-drift refusal: the packed hash sequence IS a fresh chunking
//! of the body, by construction.
#![allow(clippy::too_many_arguments)]

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::ingest::{pack_chunks, PackedChunk};
use temper_services::backend::DbBackend;
use temper_workflow::operations::{Backend, BodyUpdate, Surface, UpdateResource};

const TWO_SECTION: &str = "# Alpha\n\nAlpha prose.\n\n# Beta\n\nBeta prose.\n";
const ONE_SECTION: &str = "Just some plain prose with no headings at all.\n";

async fn auth(pool: &PgPool) -> (String, Uuid, Uuid) {
    let email = format!("wpp-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);
    (token, profile_id, context_id)
}

/// The server's own chunking of `body`, packed with inert constant vectors — no ONNX.
fn pack_body_chunks(body: &str) -> String {
    let prepared =
        temper_substrate::content::prepare_block(0, None, body).expect("server chunking");
    let packed: Vec<PackedChunk> = prepared
        .chunks
        .iter()
        .map(|c| PackedChunk {
            chunk_index: c.chunk_index as u32,
            header_path: c.header_path.clone().unwrap_or_default(),
            heading_depth: c.heading_depth.unwrap_or(0) as u8,
            content: c.content.clone(),
            content_hash: c.content_hash.clone(),
            embedding: vec![0.1; 768],
            embedded_with: None,
        })
        .collect();
    pack_chunks(&packed).expect("pack")
}

async fn post_ingest(app: &common::TestApp, token: &str, body: Value) -> Value {
    let resp = app
        .client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&body)
        .send()
        .await
        .expect("ingest request failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "ingest must succeed; body: {}",
        resp.text().await.unwrap_or_default()
    );
    resp.json().await.expect("ingest JSON")
}

async fn create_resource(pool: &PgPool, content: &str) -> (common::TestApp, String, String, Uuid) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, profile_id, context_id) = auth(pool).await;
    let created = post_ingest(
        &app,
        &token,
        json!({
            "title": "Write-path policy witness",
            "origin_uri": format!("test://wpp-{}", Uuid::new_v4()),
            "context_ref": context_id.to_string(),
            "doc_type_name": "research",
            "content": content,
            "chunks_packed": pack_body_chunks(content),
        }),
    )
    .await;
    let resource = created["id"].as_str().expect("id missing").to_string();
    (app, token, resource, profile_id)
}

/// `(seq, verbatim bytes)` of the resource's live blocks, in seq order.
async fn live_blocks(pool: &PgPool, resource: &str) -> Vec<(i32, String)> {
    sqlx::query_as(
        "SELECT b.seq, bc.content FROM kb_content_blocks b \
         JOIN kb_block_content bc ON bc.block_revision_id = b.current_revision_id \
         WHERE b.resource_id = $1 AND NOT b.is_folded ORDER BY b.seq",
    )
    .bind(Uuid::parse_str(resource).unwrap())
    .fetch_all(pool)
    .await
    .expect("live blocks")
}

/// `(event type name, emitter entity id)` of every `resource_reblocked` event for the resource.
async fn reblocked_events(pool: &PgPool, resource: &str) -> Vec<(String, Uuid)> {
    sqlx::query_as(
        "SELECT t.name, e.emitter_entity_id FROM kb_events e \
         JOIN kb_event_types t ON t.id = e.event_type_id \
         WHERE t.name = 'resource_reblocked' AND e.payload->>'resource_id' = $1 \
         ORDER BY e.occurred_at",
    )
    .bind(resource)
    .fetch_all(pool)
    .await
    .expect("reblocked events")
}

/// The emitter of the resource's `resource_created` event.
async fn created_event_emitter(pool: &PgPool, resource: &str) -> Uuid {
    sqlx::query_scalar(
        "SELECT e.emitter_entity_id FROM kb_events e \
         JOIN kb_event_types t ON t.id = e.event_type_id \
         WHERE t.name = 'resource_created' AND e.payload->>'resource_id' = $1",
    )
    .bind(resource)
    .fetch_one(pool)
    .await
    .expect("created event")
}

async fn total_events(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(pool)
        .await
        .expect("event count")
}

/// w1 — a multi-section body written through the gated create surface lands
/// policy-partitioned, content-preserving, and authored by the acting principal:
/// `writes-land-at-policy-grain` + the authored-4 threading, at surface grain.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_with_multi_section_body_lands_policy_partitioned_and_authored(pool: PgPool) {
    let (app, _token, resource, _profile) = create_resource(&pool, TWO_SECTION).await;

    let blocks = live_blocks(&pool, &resource).await;
    assert_eq!(
        blocks.len(),
        2,
        "a two-heading body must land as two policy blocks; got {blocks:?}"
    );
    let composed: String = blocks.iter().map(|(_, c)| c.as_str()).collect();
    assert_eq!(
        composed, TWO_SECTION,
        "the partition composes to exactly the written body bytes"
    );

    let reblocked = reblocked_events(&pool, &resource).await;
    assert_eq!(
        reblocked.len(),
        1,
        "exactly one re-block act must ride the create transaction"
    );
    assert_eq!(
        reblocked[0].1,
        created_event_emitter(&pool, &resource).await,
        "the re-block rides the acting principal's emitter, not a system actor"
    );
    let _ = app; // the surface under test
}

/// w2 — a body-restructure update re-partitions the resource to its NEW content in the same
/// committed state: `no-readable-window-of-stale-partition`'s observable half (the hook runs
/// inside the update's transaction, so the committed partition is the new one — there is no
/// post-commit state in which the old partition is still observable).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn body_restructure_update_repartitions_to_the_new_content(pool: PgPool) {
    let (app, token, resource, _profile) = create_resource(&pool, ONE_SECTION).await;
    assert_eq!(reblocked_events(&pool, &resource).await.len(), 0);

    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "content": TWO_SECTION,
            "chunks_packed": pack_body_chunks(TWO_SECTION),
        }))
        .send()
        .await
        .expect("PATCH failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "restructuring PATCH must succeed; body: {}",
        resp.text().await.unwrap_or_default()
    );

    let blocks = live_blocks(&pool, &resource).await;
    assert_eq!(
        blocks.len(),
        2,
        "the committed partition follows the new body"
    );
    let composed: String = blocks.iter().map(|(_, c)| c.as_str()).collect();
    assert_eq!(composed, TWO_SECTION);
    assert_eq!(
        reblocked_events(&pool, &resource).await.len(),
        1,
        "the re-partition is ledger-recorded like any other write"
    );
}

/// w3 — a body write that does not change the effective partition fires NO re-block event:
/// surface re-witness of `noop-partition-writes-leave-no-ledger-trace`, on an
/// already-policy-partitioned resource.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn identical_body_write_fires_no_reblock_event(pool: PgPool) {
    let (app, token, resource, _profile) = create_resource(&pool, TWO_SECTION).await;
    assert_eq!(reblocked_events(&pool, &resource).await.len(), 1);
    let blocks_before = live_blocks(&pool, &resource).await;
    let block_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded ORDER BY seq",
    )
    .bind(Uuid::parse_str(&resource).unwrap())
    .fetch_all(&pool)
    .await
    .expect("block ids");

    // A real write (it re-fires block_mutated on the addressed block) whose content changes no
    // partition boundary: the block's OWN bytes re-written verbatim, addressed at the first
    // block of the multi-block set. (`content` is the addressed block's whole text — writing the
    // whole BODY into one block of a partitioned resource would change the partition, which is
    // w2's witness, not this one.)
    let block0_slice = blocks_before[0].1.clone();
    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "content": block0_slice,
            "chunks_packed": pack_body_chunks(&block0_slice),
            "content_block": block_ids[0],
        }))
        .send()
        .await
        .expect("PATCH failed");
    assert_eq!(
        resp.status().as_u16(),
        200,
        "identical-body PATCH must succeed; body: {}",
        resp.text().await.unwrap_or_default()
    );

    assert_eq!(
        reblocked_events(&pool, &resource).await.len(),
        1,
        "a partition-preserving write must be indistinguishable in the re-block ledger"
    );
    let blocks_after = live_blocks(&pool, &resource).await;
    assert_eq!(
        blocks_before, blocks_after,
        "the partition (ids, seqs, bytes) is untouched"
    );
}

/// w4 — an unauthorized principal reaches neither the op nor any surface that calls it:
/// the PATCH is denied and NO event of any kind fires — the act never becomes a write, so the
/// re-block it would have ridden never exists either.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unauthorized_principal_reaches_neither_surface_nor_op(pool: PgPool) {
    let (_app, _token, resource, _profile) = create_resource(&pool, ONE_SECTION).await;

    // A different principal with no standing on this resource's home.
    let app = common::setup_test_app(pool.clone()).await;
    let (stranger_token, _stranger_profile, _stranger_context) = auth(&pool).await;
    let before = total_events(&pool).await;

    let resp = app
        .client
        .patch(app.url(&format!("/api/resources/{resource}")))
        .header("Authorization", format!("Bearer {stranger_token}"))
        .json(&json!({
            "content": "# Hijacked\n\nNo.\n",
            "chunks_packed": pack_body_chunks("# Hijacked\n\nNo.\n"),
        }))
        .send()
        .await
        .expect("PATCH request failed");
    assert_eq!(
        resp.status().as_u16(),
        403,
        "a principal without modify standing is denied at the gate train — never routed onward"
    );

    assert_eq!(
        total_events(&pool).await,
        before,
        "a denied write fires no event at all — neither block_mutated nor resource_reblocked"
    );
    assert_eq!(reblocked_events(&pool, &resource).await.len(), 0);
}

/// w5 — the reconcile sentinel (`Some("")`, a derived rebuild from chunks) must SKIP the
/// policy: it has no prose to partition and stores no verbatim bytes the op could compose.
/// Driven at the DbBackend, the same door `apply_resource_phase` uses. The chunks carry a
/// synthetic content_hash so the rebuild is a REAL revision — hash-identical chunks would be
/// suppressed by block_mutate's own no-op dedup and the write would never happen at all.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn derived_rebuild_sentinel_skips_the_policy(pool: PgPool) {
    let (_app, _token, resource, profile) = create_resource(&pool, ONE_SECTION).await;

    let rebuilt: Vec<PackedChunk> = vec![PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: ONE_SECTION.to_string(),
        content_hash: format!("{:0>64}", "0deadbeef"),
        embedding: vec![0.1; 768],
        embedded_with: None,
    }];
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));
    backend
        .update_resource(UpdateResource {
            open_meta_add: None,
            resource: temper_core::types::ids::ResourceId::from(
                Uuid::parse_str(&resource).unwrap(),
            ),
            title: None,
            slug: None,
            body: Some(BodyUpdate {
                content: String::new(),
                content_hash: None,
                chunks_packed: Some(pack_chunks(&rebuilt).expect("pack")),
                sources: Vec::new(),
                content_block: None,
            }),
            managed_meta: None,
            open_meta: None,
            move_to: None,
            context_ref: None,
            goal: None,
            act: Default::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("the sentinel rebuild must succeed — the policy must not refuse it");

    assert_eq!(
        reblocked_events(&pool, &resource).await.len(),
        0,
        "a derived rebuild fires no re-block"
    );
    let verbatim_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_content_blocks b \
         JOIN kb_block_content bc ON bc.block_revision_id = b.current_revision_id \
         WHERE b.resource_id = $1 AND NOT b.is_folded",
    )
    .bind(Uuid::parse_str(&resource).unwrap())
    .fetch_one(&pool)
    .await
    .expect("verbatim rows");
    assert_eq!(
        verbatim_rows, 0,
        "the rebuilt block is honestly derived — no verbatim bytes stored"
    );
}
