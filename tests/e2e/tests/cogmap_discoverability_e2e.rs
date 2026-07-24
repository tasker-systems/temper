#![cfg(feature = "test-db")]
//! Cognitive-map discoverability end-to-end: `GET /api/cognitive-maps` (list), `GET
//! /api/cognitive-maps/{id}` (show), and multi-map scoping on `resource list`. Drives the REAL Axum
//! server in-process, real Postgres, real JWT auth, through the production temper-client methods
//! (`cognitive_maps().list()/show()`, `resources().list()`).
//!
//! Charter blocks are PRE-EMBEDDED with synthetic chunks (mirroring genesis_cogmap_e2e.rs), so these
//! run on plain `cargo make test-e2e` — NO ONNX. The maps are born via the genesis path; each map's
//! telos/charter resource is its one auto-created foundational resource.
//!
//! The reach/visibility spine under test: a genesis creator holds an explicit read grant on their
//! map (so it lists + shows for them), while a different principal holds neither membership nor grant
//! (so the map is absent from their list and 404s on show) — the "map-read = resource-read agree by
//! construction" invariant, exercised from the surface.

mod common;

use reqwest::StatusCode;
use uuid::Uuid;

use temper_core::types::ingest::{pack_chunks, PackedChunk};
use temper_core::types::reconcile::{CreateCogmapRequest, ReconcileTelos, ReconcileTelosBlock};
use temper_workflow::types::resource::ResourceListParams;

/// A synthetic pre-embedded telos block (mirrors genesis_cogmap_e2e.rs).
fn telos_block(role: &str, content: &str, hash_seed: &str) -> ReconcileTelosBlock {
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: content.to_string(),
        content_hash: format!("{hash_seed:0>64}"),
        embedding: vec![0.1f32; 768],
        embedded_with: None,
    };
    let chunks_packed = pack_chunks(std::slice::from_ref(&chunk)).expect("pack telos chunk");
    ReconcileTelosBlock {
        role: role.to_string(),
        chunks_packed,
    }
}

/// A genesis request for a map with a distinct statement of purpose. Fixed ids keep the test
/// deterministic; the statement content is what `charter_statement` / the charter read must surface.
fn genesis_request(
    cogmap_id: u128,
    telos_id: u128,
    name: &str,
    statement: &str,
    seed: &str,
) -> CreateCogmapRequest {
    CreateCogmapRequest {
        cogmap_id: Some(Uuid::from_u128(cogmap_id)),
        telos_resource_id: Some(Uuid::from_u128(telos_id)),
        name: name.to_string(),
        telos_title: format!("{name} telos"),
        telos: Some(ReconcileTelos {
            blocks: vec![
                telos_block("statement", statement, &format!("{seed}s")),
                telos_block("question", "How does it work?", &format!("{seed}q")),
                telos_block("framing", "Why does it matter?", &format!("{seed}f")),
            ],
        }),
    }
}

/// Preflight a token (auto-provision the profile) and approve it, returning its UUID.
async fn provision_profile(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("preflight request failed");
    assert_eq!(resp.status(), StatusCode::OK, "preflight should succeed");
    let body: serde_json::Value = resp.json().await.expect("preflight json parse");
    let pid: Uuid = body["id"]
        .as_str()
        .expect("profile id missing")
        .parse()
        .expect("profile id parse");
    common::approve(&app.pool, pid).await;
    pid
}

/// GET the caller's visible cognitive maps as a raw JSON array, under an arbitrary token (used for
/// the second principal, whose client is not `app.client`).
async fn list_maps_as(app: &common::E2eTestApp, token: &str) -> Vec<serde_json::Value> {
    let resp = app
        .reqwest_client
        .get(app.url("/api/cognitive-maps"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("list request failed");
    assert_eq!(resp.status(), StatusCode::OK, "list should be 200");
    resp.json().await.expect("list json parse")
}

const MAP_A: u128 = 0x019f0aaa_1111_76cb_b1fc_260239dd0001;
const TELOS_A: u128 = 0x019f0aaa_1111_7c45_bd12_a2a715260001;
const MAP_B: u128 = 0x019f0aaa_2222_76cb_b1fc_260239dd0002;
const TELOS_B: u128 = 0x019f0aaa_2222_7c45_bd12_a2a715260002;

/// Genesis two maps as the admin, returning `(app, admin_id, second_token)`. The second principal is
/// a `watcher` of the root team (so it can see the L0 kernel) but holds NO grant on maps A/B.
async fn setup_two_maps(pool: sqlx::PgPool) -> (common::E2eTestApp, Uuid, String, Uuid) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, admin_id).await;

    for (id, telos, name, stmt, seed) in [
        (
            MAP_A,
            TELOS_A,
            "Architecture Map",
            "What holds the platform together.",
            "a",
        ),
        (
            MAP_B,
            TELOS_B,
            "Roadmap Map",
            "Where the product is going.",
            "b",
        ),
    ] {
        app.client
            .cognitive_maps()
            .create_cognitive_map(&genesis_request(id, telos, name, stmt, seed))
            .await
            .expect("admin genesis should succeed");
    }

    // A second principal with root-team read (sees L0) but no reach into A/B.
    let second_token = common::generate_second_user_jwt();
    let second_id = provision_profile(&app, &second_token).await;
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role)
         SELECT id, $1, 'watcher' FROM kb_teams WHERE slug = 'temper-system'
         ON CONFLICT (team_id, profile_id) DO NOTHING",
    )
    .bind(second_id)
    .execute(&pool)
    .await
    .expect("add second user as watcher");

    (app, admin_id, second_token, second_id)
}

// ── (1) `cogmap list` returns the creator's visible maps with charter statements, and hides maps the
//        caller cannot reach ────────────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cogmap_list_shows_visible_maps_with_charter_and_hides_others(pool: sqlx::PgPool) {
    let (app, _admin_id, second_token, _second_id) = setup_two_maps(pool).await;

    // The creator sees both maps, each with its charter statement + at least its telos foundation.
    let rows = app
        .client
        .cognitive_maps()
        .list()
        .await
        .expect("admin list should succeed");

    let map_a = rows
        .iter()
        .find(|r| r.id == Uuid::from_u128(MAP_A))
        .expect("map A present in the creator's list");
    assert_eq!(map_a.name, "Architecture Map");
    assert_eq!(
        map_a.charter_statement.as_deref(),
        Some("What holds the platform together."),
        "the statement of purpose rides in the list row"
    );
    assert!(
        map_a.resource_count >= 1,
        "the telos is a homed resource (count >= 1), got {}",
        map_a.resource_count
    );
    assert_eq!(
        map_a.region_count, 0,
        "a fresh map has no materialized regions"
    );
    assert!(
        rows.iter().any(|r| r.id == Uuid::from_u128(MAP_B)),
        "map B present in the creator's list"
    );

    // The second principal (root watcher, no grant on A/B) sees neither map.
    let second_rows = list_maps_as(&app, &second_token).await;
    let second_ids: Vec<&str> = second_rows
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    let a = Uuid::from_u128(MAP_A).to_string();
    let b = Uuid::from_u128(MAP_B).to_string();
    assert!(
        !second_ids.contains(&a.as_str()) && !second_ids.contains(&b.as_str()),
        "a principal without reach must not see maps A/B; saw {second_ids:?}"
    );
}

// ── (2) `cogmap show` returns identity + charter + foundations (telos flagged); 404 for an unreadable
//        map ───────────────────────────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cogmap_show_returns_charter_and_foundations_and_404s_on_unreadable(pool: sqlx::PgPool) {
    let (app, _admin_id, second_token, _second_id) = setup_two_maps(pool).await;

    let detail = app
        .client
        .cognitive_maps()
        .show(Uuid::from_u128(MAP_A))
        .await
        .expect("admin show should succeed");

    assert_eq!(detail.cogmap.name, "Architecture Map");
    // Charter: statement / question / framing, in seq order.
    assert_eq!(detail.charter.len(), 3, "three charter blocks");
    assert_eq!(detail.charter[0].role, "statement");
    assert_eq!(detail.charter[0].body, "What holds the platform together.");

    // Foundations: the telos is present and flagged.
    let telos = detail
        .foundations
        .iter()
        .find(|f| f.resource_id == Uuid::from_u128(TELOS_A))
        .expect("the telos is a foundational resource");
    assert!(
        telos.is_telos,
        "the telos/charter resource is flagged is_telos"
    );
    assert!(
        detail.foundations.iter().filter(|f| f.is_telos).count() == 1,
        "exactly one foundation is the telos"
    );

    // The second principal cannot read map A → 404, no partial leak.
    let resp = app
        .reqwest_client
        .get(app.url(&format!("/api/cognitive-maps/{}", Uuid::from_u128(MAP_A))))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("show request failed");
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "an unreadable map shows as 404"
    );
}

// ── (3) multi-map scope on `resource list`: the corpus is the union of the chosen maps' visible homed
//        resources; a caller without reach gets nothing ──────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_list_scopes_to_selected_cogmaps_union(pool: sqlx::PgPool) {
    let (app, _admin_id, second_token, _second_id) = setup_two_maps(pool.clone()).await;

    let telos_a = Uuid::from_u128(TELOS_A);
    let telos_b = Uuid::from_u128(TELOS_B);

    // Single map: only A's homed resource (its telos).
    let only_a = app
        .client
        .resources()
        .list(&ResourceListParams {
            cogmap_ids: Some(Uuid::from_u128(MAP_A).to_string()),
            ..Default::default()
        })
        .await
        .expect("list --cogmap A should succeed");
    let a_ids: Vec<Uuid> = only_a.rows.iter().map(|r| Uuid::from(r.id)).collect();
    assert!(a_ids.contains(&telos_a), "A's telos is in scope");
    assert!(
        !a_ids.contains(&telos_b),
        "B's telos is NOT in a single-map A scope"
    );

    // Union: both maps' homed resources.
    let both = app
        .client
        .resources()
        .list(&ResourceListParams {
            cogmap_ids: Some(format!(
                "{},{}",
                Uuid::from_u128(MAP_A),
                Uuid::from_u128(MAP_B)
            )),
            ..Default::default()
        })
        .await
        .expect("list --cogmap A,B should succeed");
    let both_ids: Vec<Uuid> = both.rows.iter().map(|r| Uuid::from(r.id)).collect();
    assert!(
        both_ids.contains(&telos_a) && both_ids.contains(&telos_b),
        "the union spans both maps' homed resources; got {both_ids:?}"
    );

    // The second principal cannot see A/B's resources → an empty scope, not an error.
    let resp = app
        .reqwest_client
        .get(app.url(&format!(
            "/api/resources?cogmap_ids={},{}",
            Uuid::from_u128(MAP_A),
            Uuid::from_u128(MAP_B)
        )))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("list request failed");
    assert_eq!(resp.status(), StatusCode::OK, "list is 200 even when empty");
    let body: serde_json::Value = resp.json().await.expect("list json");
    let rows = body["rows"].as_array().expect("rows array");
    let leaked: Vec<&str> = rows
        .iter()
        .filter_map(|r| r["id"].as_str())
        .filter(|id| *id == telos_a.to_string() || *id == telos_b.to_string())
        .collect();
    assert!(
        leaked.is_empty(),
        "a principal without reach sees none of A/B's resources; leaked {leaked:?}"
    );
}

// ── (4) coherence: `--cogmap X` means "maps you can READ", not "your visible resources homed in X".
//        A resource individually visible via a direct grant, but homed in a map the caller cannot read,
//        must NOT appear under `list --cogmap X` — matching `search --cogmap X`. (Reach/visibility
//        adversarial review, Finding 1: cogmap_foundations + the list filter now gate map-read.) ───────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_cogmap_gates_map_read_even_for_individually_visible_resources(pool: sqlx::PgPool) {
    let (app, admin_id, second_token, second_id) = setup_two_maps(pool.clone()).await;
    let telos_a = Uuid::from_u128(TELOS_A);

    // Grant the second principal a DIRECT profile read on A's telos resource — it becomes
    // individually visible to them, WITHOUT any reach into map A itself.
    sqlx::query(
        "INSERT INTO kb_access_grants
           (subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id)
         VALUES ('kb_resources', $1, 'kb_profiles', $2, true, $3)
         ON CONFLICT (subject_table, subject_id, principal_table, principal_id)
           DO UPDATE SET can_read = true",
    )
    .bind(telos_a)
    .bind(second_id)
    .bind(admin_id)
    .execute(&pool)
    .await
    .expect("direct read grant on the telos");

    // Precondition: the resource IS individually visible to the second principal (200 on show).
    let show = app
        .reqwest_client
        .get(app.url(&format!("/api/resources/{telos_a}")))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("resource show request failed");
    assert_eq!(
        show.status(),
        StatusCode::OK,
        "the granted resource is individually visible to the second principal"
    );

    // But map A is NOT readable to them, so scoping to it yields nothing — the resource is not
    // disclosed as a member of a map they cannot read.
    let scoped = app
        .reqwest_client
        .get(app.url(&format!(
            "/api/resources?cogmap_ids={}",
            Uuid::from_u128(MAP_A)
        )))
        .header("Authorization", format!("Bearer {second_token}"))
        .send()
        .await
        .expect("scoped list request failed");
    assert_eq!(scoped.status(), StatusCode::OK, "list is 200");
    let body: serde_json::Value = scoped.json().await.expect("list json");
    let ids: Vec<&str> = body["rows"]
        .as_array()
        .expect("rows array")
        .iter()
        .filter_map(|r| r["id"].as_str())
        .collect();
    assert!(
        !ids.contains(&telos_a.to_string().as_str()),
        "list --cogmap X must gate map-read: an unreadable map's resource stays out of scope even \
         when individually visible; got {ids:?}"
    );
}
