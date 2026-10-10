#![cfg(feature = "test-db")]
//! A machine over HTTP takes the content and workflow acts the refusal table leaves it.
//!
//! `temper-api`'s `machine_refusal_table_test` holds the line from the refused side: every refused
//! act answers a machine with the refusal, and no admitted act does. Neither says an admitted act
//! *works* for a machine — a `404` on a nil id is not a refusal and is not a success either. These
//! witnesses are the success side, through the production typed client on the machine's own
//! bearer, for the families no other suite drives as a machine: contexts, cognitive-map authoring
//! and reconcile, the auditor, and the team reads. Resources, edges, properties, blobs, data
//! artifacts, schemas, search, query, steward and invocations are driven as a machine by
//! `mcp_machine_reach_e2e` (every MCP tool relays over HTTP with the caller's bearer) and
//! `resource_write_machine_e2e_test`; the profile read by `temper-api`'s `human_tier_routing_test`.
//!
//! The one admin rung a machine meets on an admitted door is witnessed here too: reconcile on a map
//! whose writes require system-admin standing (the L0 kernel map) refuses a machine with plain
//! `FORBIDDEN`, because `Principal::system_admin` answers `None` for a machine without a query. An
//! admin person reconciling the same map is `reconcile_cogmap_e2e::admin_reconcile_l0_is_idempotent`.

mod common;

use reqwest::StatusCode;
use temper_core::types::auditor::AuditorDispatchTickRequest;
use temper_core::types::context::RenameContextRequest;
use temper_core::types::ingest::{pack_chunks, PackedChunk};
use temper_core::types::query_params::SweepQuery;
use temper_core::types::reconcile::{CreateCogmapRequest, ReconcileCogmapRequest, ReconcileEntry};
use temper_core::types::resource::ResourceCreateRequest;
use uuid::Uuid;

const MACHINE_CLIENT: &str = "allowed-acts-agent";

/// The L0 kernel map: its writes require system-admin standing (`cogmap_write_requires_admin`).
const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

/// One reconcile entry: a single-chunk node whose `content_hash` matches its chunk, as the reconcile
/// write path verifies.
fn entry(title: &str) -> ReconcileEntry {
    use sha2::{Digest, Sha256};
    let hex = |s: &str| format!("{:x}", Sha256::digest(s.as_bytes()));
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: format!("{title}: a node the machine authored."),
        content_hash: hex(title),
        embedding: vec![0.1; 768],
        embedded_with: None,
    };
    let content_hash = hex(&hex(&chunk.content_hash));
    ReconcileEntry {
        id: Uuid::now_v7(),
        origin_uri: format!("test://allowed-acts/{}", Uuid::now_v7()),
        title: title.to_string(),
        doc_type: "kernel_landmark".to_string(),
        content_hash,
        chunks_packed: pack_chunks(std::slice::from_ref(&chunk)).expect("pack"),
        facets: serde_json::json!({}),
        edges: vec![],
    }
}

fn one_entry(title: &str) -> ReconcileCogmapRequest {
    ReconcileCogmapRequest {
        entries: vec![entry(title)],
        fold_resources: vec![],
        fold_edges: vec![],
        telos: None,
    }
}

/// Contexts: a machine creates a context, renames it, and writes a resource into it.
///
/// The context is the machine's own. A team-owned context is created only by the team's `owner` or
/// `maintainer` (`context_service::resolve_create_owner`), a person `member` is refused it the same
/// way, and a machine never holds a governing seat — so its own namespace is where a machine
/// creates.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_machine_creates_renames_and_writes_a_context(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    let m = common::register_team_machine(&app, MACHINE_CLIENT, "member").await;
    let client = app.client_for(&m.token);

    let created = client
        .contexts()
        .create("machine-made", None)
        .await
        .expect("a machine creates a context of its own");
    let context = *created.id;

    let renamed = client
        .contexts()
        .rename(
            context,
            &RenameContextRequest {
                name: "machine-renamed".to_string(),
            },
        )
        .await
        .expect("a machine renames the context");
    assert_eq!(renamed.name, "machine-renamed");

    let resource = client
        .resources()
        .create(&ResourceCreateRequest {
            kb_context_id: context,
            doc_type: "research".to_string(),
            origin_uri: format!("test://allowed-acts/{}", Uuid::now_v7()),
            title: "written by a machine".to_string(),
            idempotency_key: None,
            act: Default::default(),
        })
        .await
        .expect("a machine writes into the context");
    let homed: Uuid = sqlx::query_scalar(
        "SELECT anchor_id FROM kb_resource_homes WHERE resource_id = $1 AND anchor_table = 'kb_contexts'",
    )
    .bind(resource.id)
    .fetch_one(&app.pool)
    .await
    .expect("the resource's home");
    assert_eq!(homed, context, "the write landed in the machine's context");
}

/// Cognitive maps: a machine creates a map, authors a node into it by reconcile, and re-delivers
/// the same manifest as a no-op — the map is its own, so authorship is the gate.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_machine_creates_and_reconciles_its_own_cogmap(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    let m = common::register_team_machine(&app, MACHINE_CLIENT, "member").await;
    let client = app.client_for(&m.token);

    let map = client
        .cognitive_maps()
        .create_cognitive_map(&CreateCogmapRequest {
            cogmap_id: None,
            telos_resource_id: None,
            name: "machine map".to_string(),
            telos_title: "what the machine is for".to_string(),
            telos: None,
        })
        .await
        .expect("a machine creates a cognitive map");
    assert!(map.created);

    let req = one_entry("machine node");
    let first = client
        .cognitive_maps()
        .reconcile_cognitive_map(map.cogmap_id, &req, &Default::default())
        .await
        .expect("a machine reconciles its own map");
    assert_eq!(first.created, 1, "the node is authored");
    let again = client
        .cognitive_maps()
        .reconcile_cognitive_map(map.cogmap_id, &req, &Default::default())
        .await
        .expect("a re-delivery is accepted");
    assert_eq!((again.created, again.unchanged), (0, 1), "and is a no-op");
}

/// The admin rung on an admitted door: reconcile on the L0 map requires system-admin standing, and
/// a machine never holds it — refused with plain `FORBIDDEN` (not the machine sentence: the door
/// admits machines, the regime refuses this one), and nothing lands on the map.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_machine_is_refused_reconciling_the_l0_map(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    let m = common::register_team_machine(&app, MACHINE_CLIENT, "member").await;
    common::enable_invite_only(&app.pool, m.admin).await;

    let req = one_entry("not on the kernel");
    let title = req.entries[0].title.clone();
    let resp = app
        .reqwest_client
        .put(app.url(&format!("/api/cognitive-maps/{L0_COGMAP}")))
        .bearer_auth(&m.token)
        .json(&req)
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    let body: serde_json::Value = resp.json().await.expect("error body");
    assert_eq!(body["error"]["code"], "FORBIDDEN", "{body}");

    let landed: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_resources WHERE title = $1")
        .bind(&title)
        .fetch_one(&app.pool)
        .await
        .expect("count");
    assert_eq!(landed, 0, "the refused reconcile wrote nothing");
}

/// The auditor: a machine surveys audit coverage and runs a dispatch tick (which requires a
/// registered, unrevoked machine principal). Nothing is due, so the tick claims nothing.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_machine_runs_the_auditor_sweep_and_dispatch(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    let m = common::register_team_machine(&app, MACHINE_CLIENT, "member").await;
    let client = app.client_for(&m.token);

    client
        .auditor()
        .sweep(&SweepQuery { cap: None })
        .await
        .expect("a machine surveys audit coverage");
    let tick = client
        .auditor()
        .dispatch(&AuditorDispatchTickRequest { cap: None }, None)
        .await
        .expect("a machine runs the dispatch tick");
    assert!(
        tick.claimed.is_empty(),
        "nothing is due on a fresh deployment"
    );
}

/// The team reads: a machine lists its teams and reads the one it is seated on.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_machine_lists_and_reads_its_team(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    let m = common::register_team_machine(&app, MACHINE_CLIENT, "member").await;
    let client = app.client_for(&m.token);

    let teams = client
        .teams()
        .list()
        .await
        .expect("a machine lists its teams");
    assert!(
        teams.iter().any(|t| t.id == m.team),
        "its team is listed: {teams:?}"
    );
    let detail = client
        .teams()
        .get(m.team)
        .await
        .expect("a machine reads its team");
    assert_eq!(detail.id, m.team);
}
