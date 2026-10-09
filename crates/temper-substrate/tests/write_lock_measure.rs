#![cfg(feature = "test-db")]
//! The measurement behind the write-side lock bound (task 01a0fd12-f4b7-7bd2-81d0-13c0814650d5).
//! It asserts nothing and is `#[ignore]`d, so no job runs it. Run it by hand:
//!
//! ```text
//! MEASURE_SECTIONS=200 MEASURE_REVISIONS=10 MEASURE_PROPS=50 MEASURE_EDGES=200 \
//! MEASURE_ESTATE=500 cargo nextest run -p temper-substrate --features test-db \
//!   --test write_lock_measure --run-ignored only --no-capture
//! ```
//!
//! It builds one resource R through the real write paths: `MEASURE_SECTIONS` heading sections,
//! rewritten whole `MEASURE_REVISIONS` times, plus `MEASURE_PROPS` open keys and `MEASURE_EDGES`
//! edges into R from other resources. Around it sit `MEASURE_ESTATE` unrelated resources, because
//! parts of the act read the ledger and scale with the estate, not only with R. It then reports:
//!
//! * **act**: the wall time of `resource_erasure_execute` on R, the statement that holds R's
//!   `FOR UPDATE`;
//! * **writer wait**: how long a floored writer that arrives just after the act starts waits on
//!   R's row before its `FOR KEY SHARE` is granted (or refused, once R is erased). This is the
//!   wait a lock bound would cut off.

mod common;

use std::time::{Duration, Instant};

use sha2::Digest;
use sqlx::PgPool;
use temper_substrate::affinity::EdgeKind;
use temper_substrate::content::IncomingChunk;
use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{AnchorRef, EdgePolarity};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, AssertParams, CreateParams, UpdateParams};
use uuid::Uuid;

fn knob(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn chunk(i: usize, prose: &str, header: &str) -> IncomingChunk {
    IncomingChunk {
        chunk_index: i as i32,
        content_hash: format!("{:x}", sha2::Sha256::digest(prose.trim())),
        content: prose.to_string(),
        embedding: vec![0.1; 768],
        embedded_with: Some("model-sha-1".to_string()),
        header_path: header.to_string(),
        heading_depth: 1,
    }
}

/// A body of `sections` heading sections, every prose line stamped with `rev`, and its chunks.
fn body(sections: usize, rev: usize) -> (String, Vec<IncomingChunk>) {
    let mut text = String::new();
    let mut chunks = Vec::with_capacity(sections);
    for s in 0..sections {
        let header = format!("Section {s}");
        let prose = format!(
            "Revision {rev} of section {s}: a paragraph long enough to look like prose, with \
             a name, an address and a few numbers 4111 1111 1111 {s:04} in it."
        );
        text.push_str(&format!("# {header}\n\n{prose}\n\n"));
        chunks.push(chunk(s, &prose, &header));
    }
    (text, chunks)
}

async fn system_actor(pool: &PgPool) -> (ProfileId, EntityId) {
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

async fn create(
    pool: &PgPool,
    who: (ProfileId, EntityId),
    home: ContextId,
    n: usize,
    sections: usize,
) -> ResourceId {
    let (text, chunks) = body(sections, 0);
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title: &format!("measure {n}"),
            origin_uri: &format!("test://measure/{n}"),
            body: &text,
            doc_type: "research",
            home: AnchorRef::context(home),
            owner: who.0,
            originator: who.0,
            emitter: who.1,
            properties: &[],
            chunks: Some(chunks),
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .unwrap()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
#[ignore = "a measurement, run by hand; see the module note"]
async fn measure_the_act_and_a_writer_waiting_behind_it(pool: PgPool) {
    let sections = knob("MEASURE_SECTIONS", 50);
    let revisions = knob("MEASURE_REVISIONS", 5);
    let props = knob("MEASURE_PROPS", 20);
    let edges = knob("MEASURE_EDGES", 50);
    let estate = knob("MEASURE_ESTATE", 100);

    bootseed::seed_system(&pool).await.unwrap();
    let who = system_actor(&pool).await;
    let home = ContextId::from(
        common::insert_context(&pool, "kb_profiles", who.0.uuid(), "measure", "measure")
            .await
            .unwrap(),
    );

    let built = Instant::now();
    let r = create(&pool, who, home, 0, sections).await;
    for rev in 1..=revisions {
        let (text, chunks) = body(sections, rev);
        writes::update_resource(
            &pool,
            UpdateParams {
                resource: r,
                body: Some(&text),
                title: None,
                origin_uri: None,
                properties: &[],
                unset_keys: &[],
                chunks: Some(chunks),
                sources: vec![],
                content_block: None,
                rehome_to: None,
                emitter: who.1,
            },
        )
        .await
        .unwrap();
    }
    let properties: Vec<(String, serde_json::Value)> = (0..props)
        .map(|k| (format!("key-{k}"), serde_json::json!(format!("value {k}"))))
        .collect();
    writes::update_resource(
        &pool,
        UpdateParams {
            resource: r,
            body: None,
            title: None,
            origin_uri: None,
            properties: &properties,
            unset_keys: &[],
            chunks: None,
            sources: vec![],
            content_block: None,
            rehome_to: None,
            emitter: who.1,
        },
    )
    .await
    .unwrap();
    for n in 1..=estate {
        let other = create(&pool, who, home, n, 3).await;
        if n <= edges {
            writes::assert_relationship(
                &pool,
                AssertParams {
                    src: other,
                    tgt: r,
                    kind: EdgeKind::LeadsTo,
                    polarity: EdgePolarity::Forward,
                    label: Some("cites"),
                    weight: 1.0,
                    home,
                    emitter: who.1,
                },
            )
            .await
            .unwrap();
        }
    }
    let blocks: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded",
    )
    .bind(r.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let revs: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_block_revisions rv JOIN kb_content_blocks b \
           ON b.id = rv.block_id WHERE b.resource_id = $1",
    )
    .bind(r.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    sqlx::query("ANALYZE").execute(&pool).await.unwrap();
    eprintln!(
        "built in {:?}: R has {blocks} live blocks, {revs} revisions, {props} keys, {edges} \
         edges in; estate {estate} resources, {events} events",
        built.elapsed()
    );

    // A whole-body rewrite of R, and a property-only update arriving just after it: since updates
    // of one resource serialize on R's row, the second waits for the first to commit.
    let (text, chunks) = body(sections, revisions + 1);
    let rewrite_pool = pool.clone();
    let rewriter = who.1;
    let rewrite = tokio::spawn(async move {
        let started = Instant::now();
        writes::update_resource(
            &rewrite_pool,
            UpdateParams {
                resource: r,
                body: Some(&text),
                title: None,
                origin_uri: None,
                properties: &[],
                unset_keys: &[],
                chunks: Some(chunks),
                sources: vec![],
                content_block: None,
                rehome_to: None,
                emitter: rewriter,
            },
        )
        .await
        .unwrap();
        started.elapsed()
    });
    tokio::time::sleep(Duration::from_millis(5)).await;
    let asked = Instant::now();
    let note = [("note".to_owned(), serde_json::json!("behind the rewrite"))];
    writes::update_resource(
        &pool,
        UpdateParams {
            resource: r,
            body: None,
            title: None,
            origin_uri: None,
            properties: &note,
            unset_keys: &[],
            chunks: None,
            sources: vec![],
            content_block: None,
            rehome_to: None,
            emitter: who.1,
        },
    )
    .await
    .unwrap();
    let behind_rewrite = asked.elapsed();
    let rewrite_took = rewrite.await.unwrap();
    eprintln!("MEASURE rewrite={rewrite_took:?} | update_behind_rewrite={behind_rewrite:?}");

    // The act, on its own connection.
    let act_pool = pool.clone();
    let operator = who.1;
    let act = tokio::spawn(async move {
        let started = Instant::now();
        sqlx::query("SELECT resource_erasure_execute($1,$2,$3,$4)")
            .bind(r.uuid())
            .bind(operator)
            .bind(operator)
            .bind(Uuid::now_v7())
            .execute(&act_pool)
            .await
            .expect("the act completes");
        started.elapsed()
    });

    // A floored writer arriving once the act holds R: the floor's own lock statement.
    tokio::time::sleep(Duration::from_millis(5)).await;
    let mut writer = pool.begin().await.unwrap();
    let asked = Instant::now();
    let floor: Option<Option<chrono::DateTime<chrono::Utc>>> =
        sqlx::query_scalar("SELECT erased_at FROM kb_resources WHERE id = $1 FOR KEY SHARE")
            .bind(r.uuid())
            .fetch_optional(&mut *writer)
            .await
            .unwrap();
    let waited = asked.elapsed();
    writer.rollback().await.unwrap();
    let act_took = act.await.unwrap();

    eprintln!(
        "MEASURE sections={sections} revisions={revisions} props={props} edges={edges} \
         estate={estate} | act={act_took:?} | writer_wait={waited:?} (saw erased: {})",
        floor.flatten().is_some()
    );
}
