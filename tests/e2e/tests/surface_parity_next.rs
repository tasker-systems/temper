#![cfg(all(feature = "test-db", feature = "next-backend"))]

//! WS6 migration-endgame — the **surface-parity gate**: a durable acceptance test for the schema
//! collapse. Prod runs a two-schema split (`temper_next.*` is the live substrate; `public.*` is stale
//! legacy). Reads are *supposed* to route through `AppState.backend_selection`, but four read surfaces
//! BYPASS the flag and hit `public.*` directly (raw `sqlx::query!` against the default search_path):
//!   - `GET /api/resources?meta_only=true` (list --meta-only) → `resource_service::list_visible_meta`
//!   - `GET /api/resources/{id}/edges` (show --edges)         → `edge_service::list_resource_edges`
//!   - `GET /api/graph/subgraph` (graph aggregator)           → `graph_service::aggregator_subgraph`
//!   - `GET /api/events` (events feed)                        → `event_service::list_visible`
//!
//! The other five reads are flag-aware (route through `read_selector::*` / `select_backend`):
//! list (default), show, show --meta, content, search.
//!
//! This test creates a resource that exists ONLY in `temper_next` (schema-only-resident, via
//! `NextBackend` writes), then drives EVERY read surface over the REAL HTTP stack under
//! `BackendSelection::Next` and asserts each one resolves it. The assertions encode the DESIRED
//! post-collapse end-state — every surface MUST see the resource. Today that state is RED on exactly
//! the four leaking surfaces (they look in `public`, which lacks the resource) and GREEN on the five
//! flag-aware ones. The test does NOT codify the bug (it never asserts "must NOT see"); it documents the
//! leaks by asserting the floor the collapse must reach. When the endgame collapse dissolves the split —
//! one schema, every surface resolves to it — this test goes fully GREEN.
//!
//! Ships `#[ignore]`d as the collapse acceptance gate. Local-only: no CI job enables `next-backend`. Run:
//! `cargo nextest run -p temper-e2e --features test-db,next-backend --run-ignored all -E 'test(surface_parity_next)'`.

mod common;

use reqwest::StatusCode;
use temper_api::backend::{BackendSelection, NextBackend};
use temper_core::operations::{AssertRelationship, Backend, CreateResource, Surface};
use temper_core::types::graph::{EdgeKind, Polarity};
use temper_core::types::ids::ProfileId;
use temper_core::types::managed_meta::ManagedMeta;

const SEED_RESOURCE_ID: &str = "00000000-0000-0000-0099-000000000001";

/// origin_uris for the two schema-only-resident resources (the concept under test + its edge peer).
const CONCEPT_URI: &str = "test://surface-parity-concept";
const PEER_URI: &str = "test://surface-parity-peer";

/// A single FTS token carried in the concept's title so the search surface has a deterministic match
/// (the concept has no body — an empty body keeps the create ONNX-free, so FTS rides the title weight-A).
const SENTINEL_TOKEN: &str = "surfaceparitysentinel";

/// Build a metadata-only (`body: None`) create command for the Next write path. Empty body ⇒ no chunks
/// ⇒ no embedding call, so the fixture builds under `test-db,next-backend` without ONNX.
fn create_cmd(slug: &str, doctype: &str, title: &str, origin_uri: &str) -> CreateResource {
    CreateResource {
        slug: slug.into(),
        doctype: doctype.into(),
        context: "temper".into(),
        title: title.into(),
        body: None,
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        origin_uri: Some(origin_uri.into()),
        chunks_packed: None,
        content_hash: None,
        origin: Surface::CliCloud,
    }
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
#[ignore = "WS6 surface-parity GATE: RED until the endgame collapse dissolves the raw-pool reads (list --meta-only, show --edges, graph, events); the collapse must turn this fully green"]
async fn all_read_surfaces_resolve_next_only_resource(pool: sqlx::PgPool) {
    // 1. Legacy setup: base profiles/contexts + the seeded resource in `public`.
    let app = common::setup(pool).await;

    // Bind the authenticated principal (`e2e-test-user`) to the SYSTEM profile so the WS2
    // visibility-scoped reads are authorized — resolve_from_claims resolves by (provider, subject)
    // first, so this pre-bound link makes the HTTP caller BE the SYSTEM owner (the same binding the
    // 4b read-path test uses). auth_provider = the test server's configured "test-provider".
    sqlx::query(
        "INSERT INTO kb_profile_auth_links \
            (id, profile_id, auth_provider, auth_provider_user_id, email, is_default, linked_at) \
         VALUES (gen_random_uuid(), $1::uuid, 'test-provider', 'e2e-test-user', \
                 'e2e@test.example.com', true, now()) \
         ON CONFLICT DO NOTHING",
    )
    .bind(common::SYSTEM_PROFILE_ID)
    .execute(&app.pool)
    .await
    .expect("bind principal to SYSTEM profile");

    // 2. Manifest the seed + own the temper context with SYSTEM, so synthesis bootstraps the
    //    `temper_next` substrate (SYSTEM profile, its per-surface emitters, and a "temper" context
    //    owned by the synthesized SYSTEM) that the Next write path resolves against. Mirrors the
    //    write-path round-trip test's bootstrap.
    sqlx::query(
        "INSERT INTO kb_resource_manifests (resource_id, managed_meta, open_meta) \
         VALUES ($1::uuid, '{}'::jsonb, '{}'::jsonb) ON CONFLICT (resource_id) DO NOTHING",
    )
    .bind(SEED_RESOURCE_ID)
    .execute(&app.pool)
    .await
    .expect("seed manifest");
    sqlx::query(
        "UPDATE kb_contexts SET kb_owner_table='kb_profiles', kb_owner_id=$1::uuid WHERE id=$2::uuid",
    )
    .bind(common::SYSTEM_PROFILE_ID)
    .bind(common::TEMPER_CONTEXT_ID)
    .execute(&app.pool)
    .await
    .expect("own temper context");

    temper_next::synthesis::run(&app.pool, temper_next::synthesis::RunOpts::default())
        .await
        .expect("synthesis::run");

    // 3. Create the schema-only-resident fixture via the Next write path (lands ONLY in temper_next):
    //    a CONCEPT (so it is an aggregator seed for the graph subgraph surface) + a peer, then an edge
    //    between them (so the show --edges + graph surfaces have an edge to find). The edge is written
    //    through `NextBackend::assert_relationship` — the live 4c Next edge-write path (proven by
    //    `backend_write_path_next.rs`); it lands the edge directly in `temper_next.kb_edges`, which is
    //    all this READ-surface gate requires. The Next create also asserts the `doc_type` property and
    //    mints a `resource_created` event in temper_next, feeding the events surface.
    let profile = ProfileId::from(uuid::Uuid::parse_str(common::SYSTEM_PROFILE_ID).unwrap());
    let next = NextBackend::new(app.pool.clone(), profile);

    let concept = next
        .create_resource(create_cmd(
            "surface-parity-concept",
            "concept",
            &format!("{SENTINEL_TOKEN} Concept"),
            CONCEPT_URI,
        ))
        .await
        .expect("create next-only concept")
        .value;
    let concept_id = *concept.id;

    let peer = next
        .create_resource(create_cmd(
            "surface-parity-peer",
            "research",
            "Surface Parity Peer",
            PEER_URI,
        ))
        .await
        .expect("create next-only peer")
        .value;
    let peer_id = *peer.id;

    let edge_id = next
        .assert_relationship(AssertRelationship {
            source: concept.id,
            target: peer.id,
            edge_kind: EdgeKind::LeadsTo,
            polarity: Polarity::Forward,
            label: "relates_to".into(),
            weight: 1.0,
            origin: Surface::CliCloud,
        })
        .await
        .expect("assert next-only edge")
        .value;

    // ── negative controls: prove the fixture is genuinely schema-only-resident ──────────────────
    // The concept EXISTS in temper_next and does NOT exist in public, so a leaking-surface miss is
    // attributable to the schema split, not to a bad fixture.
    let in_next: i64 =
        sqlx::query_scalar("SELECT count(*) FROM temper_next.kb_resources WHERE id = $1")
            .bind(concept_id)
            .fetch_one(&app.pool)
            .await
            .expect("count temper_next");
    assert_eq!(in_next, 1, "concept must exist in temper_next.kb_resources");
    let in_public: i64 =
        sqlx::query_scalar("SELECT count(*) FROM public.kb_resources WHERE id = $1")
            .bind(concept_id)
            .fetch_one(&app.pool)
            .await
            .expect("count public");
    assert_eq!(
        in_public, 0,
        "concept must NOT exist in public.kb_resources (genuinely schema-only-resident)"
    );
    let edge_in_next: i64 =
        sqlx::query_scalar("SELECT count(*) FROM temper_next.kb_edges WHERE id = $1")
            .bind(edge_id)
            .fetch_one(&app.pool)
            .await
            .expect("count temper_next edge");
    assert_eq!(edge_in_next, 1, "edge must exist in temper_next.kb_edges");

    // 4. Serve the SAME pool under `Next` and drive every read surface over the real HTTP stack.
    let next_addr = common::spawn_app_server(app.pool.clone(), BackendSelection::Next).await;
    let base = format!("http://{next_addr}");
    let auth = format!("Bearer {}", app.token);
    let cid = concept_id.to_string();

    // ════════════════════════════════════════════════════════════════════════════════════════════
    // Flag-aware surfaces (route through read_selector / select_backend) — expected PASS today.
    // ════════════════════════════════════════════════════════════════════════════════════════════

    // (1) list (default): GET /api/resources → the visible set includes the concept.
    let resp = app
        .reqwest_client
        .get(format!("{base}/api/resources"))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("list default GET");
    assert_eq!(resp.status(), StatusCode::OK, "list (default): 200");
    let body: serde_json::Value = resp.json().await.expect("list default json");
    assert!(
        body["rows"]
            .as_array()
            .expect("list rows array")
            .iter()
            .any(|r| r["id"].as_str() == Some(cid.as_str())),
        "list (default): flag-aware surface resolves the live schema and returns the concept"
    );

    // (2) show: GET /api/resources/{id} → 200 with the concept row.
    let resp = app
        .reqwest_client
        .get(format!("{base}/api/resources/{cid}"))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("show GET");
    assert_eq!(resp.status(), StatusCode::OK, "show: 200");
    let body: serde_json::Value = resp.json().await.expect("show json");
    assert_eq!(
        body["id"].as_str(),
        Some(cid.as_str()),
        "show: flag-aware surface returns the concept"
    );

    // (3) show --meta: GET /api/resources/{id}/meta → 200 for the concept.
    let resp = app
        .reqwest_client
        .get(format!("{base}/api/resources/{cid}/meta"))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("show --meta GET");
    assert_eq!(resp.status(), StatusCode::OK, "show --meta: 200");
    let body: serde_json::Value = resp.json().await.expect("meta json");
    assert_eq!(
        body["resource_id"].as_str(),
        Some(cid.as_str()),
        "show --meta: flag-aware surface returns the concept's meta"
    );

    // (4) content: GET /api/resources/{id}/content → 200 (resource resolves; empty-body markdown is fine).
    let resp = app
        .reqwest_client
        .get(format!("{base}/api/resources/{cid}/content"))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("content GET");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "content: flag-aware surface resolves the concept (200)"
    );
    let body: serde_json::Value = resp.json().await.expect("content json");
    assert_eq!(
        body["resource_id"].as_str(),
        Some(cid.as_str()),
        "content: response is for the concept"
    );

    // (5) search: POST /api/search (FTS on the sentinel title token) → the concept is a hit.
    let resp = app
        .reqwest_client
        .post(format!("{base}/api/search"))
        .header("Authorization", &auth)
        .json(&serde_json::json!({
            "query": SENTINEL_TOKEN,
            "graph_expand": false,
        }))
        .send()
        .await
        .expect("search POST");
    assert_eq!(resp.status(), StatusCode::OK, "search: 200");
    let body: serde_json::Value = resp.json().await.expect("search json");
    assert!(
        body.as_array()
            .expect("search results array")
            .iter()
            .any(|r| r["resource_id"].as_str() == Some(cid.as_str())),
        "search: flag-aware surface (FTS over temper_next) finds the concept"
    );

    // ════════════════════════════════════════════════════════════════════════════════════════════
    // Leaking surfaces (raw-pool reads against public.*) — RED today, GREEN at the collapse.
    // These assert the DESIRED post-collapse end-state (every surface MUST see the concept). They are
    // NOT inverted to "must NOT see" — the test documents the leak, it does not codify the bug.
    // ════════════════════════════════════════════════════════════════════════════════════════════

    // (6) list --meta-only: GET /api/resources?meta_only=true → list_visible_meta(public).
    let resp = app
        .reqwest_client
        .get(format!("{base}/api/resources?meta_only=true"))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("list --meta-only GET");
    assert_eq!(resp.status(), StatusCode::OK, "list --meta-only: 200");
    let body: serde_json::Value = resp.json().await.expect("list --meta-only json");
    assert!(
        body["rows"]
            .as_array()
            .expect("meta rows array")
            .iter()
            .any(|r| r["resource_id"].as_str() == Some(cid.as_str())),
        "list --meta-only: surface must resolve to the live schema (RED until collapse)"
    );

    // (7) show --edges: GET /api/resources/{id}/edges → list_resource_edges(public). The concept is
    //     absent from public, so the visibility gate there returns 404 today; at collapse it resolves
    //     and returns the edge to the peer.
    let resp = app
        .reqwest_client
        .get(format!("{base}/api/resources/{cid}/edges"))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("show --edges GET");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "show --edges: surface must resolve to the live schema (RED until collapse)"
    );
    let body: serde_json::Value = resp.json().await.expect("edges json");
    let peer_str = peer_id.to_string();
    assert!(
        body.as_array()
            .expect("edges array")
            .iter()
            .any(|e| e["peer_resource_id"].as_str() == Some(peer_str.as_str())),
        "show --edges: the concept's edge to the peer must be present (RED until collapse)"
    );

    // (8) graph subgraph: GET /api/graph/subgraph?owner=@me&context=temper → aggregator_subgraph(public).
    //     The concept is an aggregator (concept-typed) seed, so it must appear as a node post-collapse.
    let resp = app
        .reqwest_client
        .get(format!(
            "{base}/api/graph/subgraph?owner=@me&context=temper"
        ))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("graph subgraph GET");
    assert_eq!(resp.status(), StatusCode::OK, "graph subgraph: 200");
    let body: serde_json::Value = resp.json().await.expect("subgraph json");
    assert!(
        body["nodes"]
            .as_array()
            .expect("subgraph nodes array")
            .iter()
            .any(|n| n["id"].as_str() == Some(cid.as_str())),
        "graph subgraph: surface must resolve to the live schema (RED until collapse)"
    );

    // (9) events: GET /api/events?resource_id={id} → list_visible(public). The Next create minted a
    //     resource_created event in temper_next; the public-reading feed misses it today.
    let resp = app
        .reqwest_client
        .get(format!("{base}/api/events?resource_id={cid}"))
        .header("Authorization", &auth)
        .send()
        .await
        .expect("events GET");
    assert_eq!(resp.status(), StatusCode::OK, "events: 200");
    let body: serde_json::Value = resp.json().await.expect("events json");
    assert!(
        !body.as_array().expect("events array").is_empty(),
        "events: the concept's create event must be visible (RED until collapse)"
    );
}
