#![cfg(feature = "test-db")]
//! `POST /api/cognitive-maps` end-to-end: drives the REAL Axum server (in-process), real Postgres, real
//! JWT auth. Cognitive-map genesis (org-provisioning Chunk 4). The optional telos charter is PRE-EMBEDDED
//! with synthetic (recognizable) chunks, so the handler stays a pure event+projection path — NO ONNX.
//! This runs on plain `cargo make test-e2e` (NOT `test-e2e-embed`), mirroring `reconcile_cogmap_e2e.rs`.
//!
//! The canonical seed leaves `kb_system_settings.gating_team_slug` NULL → `is_system_admin` is false for
//! everyone. `common::enable_invite_only` configures the gating team AND makes the given profile its
//! owner/admin, exercising BOTH the allow (admin) and deny (non-admin) paths of the genesis gate.

mod common;

use reqwest::StatusCode;
use uuid::Uuid;

use temper_core::types::ingest::{pack_chunks, PackedChunk};
use temper_core::types::reconcile::{CreateCogmapRequest, ReconcileTelos, ReconcileTelosBlock};

// ── chunk fabrication (mirrors reconcile_cogmap_e2e.rs; the e2e crate cannot depend on temper-substrate,
//    which pulls ort/ONNX — so telos blocks are built with sha2-free synthetic content hashes). ──

/// A synthetic pre-embedded telos block. `hash_seed` is zero-padded to 64 chars for a stable, distinct
/// `content_hash`. Genesis does not diff the charter (it is a fresh create), so any well-formed chunk works.
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

fn three_block_telos() -> ReconcileTelos {
    ReconcileTelos {
        blocks: vec![
            telos_block("statement", "What is this map?", "s1"),
            telos_block("question", "How does it work?", "q1"),
            telos_block("framing", "Why does it matter?", "f1"),
        ],
    }
}

/// A genesis request with fixed ids (so a re-POST delivers the SAME id → idempotent no-op).
fn genesis_request() -> CreateCogmapRequest {
    CreateCogmapRequest {
        cogmap_id: Some(Uuid::from_u128(0x019f0aaa_2ace_76cb_b1fc_260239dd16a5)),
        telos_resource_id: Some(Uuid::from_u128(0x019f0aaa_2acf_7c45_bd12_a2a7152644a1)),
        name: "Org provisioning map".to_string(),
        telos_title: "Org telos".to_string(),
        telos: Some(three_block_telos()),
    }
}

/// Pre-flight a token by hitting GET /api/profile (auto-provisions the profile), returning its UUID.
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
    // D11: a fresh principal is born Denied. Approve so this actor clears the front door
    // and the ENDPOINT authz (ownership, admin-only, grants) is what the test exercises.
    let __pid: Uuid = body["id"]
        .as_str()
        .expect("profile id missing")
        .parse()
        .expect("profile id parse");
    common::approve(&app.pool, __pid).await;
    __pid
}

// ── (a) admin genesis creates a map, then re-genesis at the same id is an idempotent no-op ───────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn admin_genesis_creates_then_is_idempotent(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let admin_id = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, admin_id).await;

    let req = genesis_request();

    // First genesis: the map is created. Drive the production CLIENT method.
    let out1 = app
        .client
        .cognitive_maps()
        .create_cognitive_map(&req)
        .await
        .expect("admin genesis should succeed");
    assert!(out1.created, "first genesis creates the map");
    assert_eq!(out1.cogmap_id, req.cogmap_id.unwrap());
    assert_eq!(out1.telos_resource_id, req.telos_resource_id.unwrap());

    // Re-POST the identical request → idempotent no-op (the map exists at this id).
    let out2 = app
        .client
        .cognitive_maps()
        .create_cognitive_map(&req)
        .await
        .expect("second admin genesis should succeed");
    assert!(!out2.created, "re-genesis at the same id is a no-op");
    assert_eq!(out2.cogmap_id, out1.cogmap_id);
    assert_eq!(out2.telos_resource_id, out1.telos_resource_id);
}

// ── (b) non-admin genesis now SUCCEEDS, but its caller-supplied id is IGNORED (server-minted) ─────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_admin_genesis_succeeds_with_server_minted_id(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let admin_id = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, admin_id).await;

    // A SECOND user with system access (a `watcher` of temper-system) but NOT admin.
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

    let req = genesis_request(); // supplies fixed cogmap_id / telos_resource_id
    let resp = app
        .reqwest_client
        .post(app.url("/api/cognitive-maps"))
        .header("Authorization", format!("Bearer {second_token}"))
        .json(&req)
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "a non-admin may now genesis a non-reserved map"
    );
    let body: serde_json::Value = resp.json().await.expect("json parse");
    assert_eq!(body["created"], true, "the map was created");

    // Reserved-id hardening: the caller-supplied id was IGNORED; the server minted a fresh one.
    let returned_id: Uuid = body["cogmap_id"].as_str().unwrap().parse().unwrap();
    assert_ne!(
        returned_id,
        req.cogmap_id.unwrap(),
        "a non-admin's supplied id must be ignored and server-minted"
    );
    let requested_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM kb_cogmaps WHERE id = $1)")
            .bind(req.cogmap_id.unwrap())
            .fetch_one(&pool)
            .await
            .expect("exists query");
    assert!(
        !requested_exists,
        "nothing was written at the caller-supplied id"
    );

    // Creator-grant: the non-admin creator holds `grant` on and can AUTHOR the new map.
    let can_grant: bool =
        sqlx::query_scalar("SELECT can('kb_profiles', $1, 'grant', 'kb_cogmaps', $2)")
            .bind(second_id)
            .bind(returned_id)
            .fetch_one(&pool)
            .await
            .expect("can grant query");
    assert!(can_grant, "the creator holds can_grant on its new map");

    let authorable: bool = sqlx::query_scalar("SELECT cogmap_authorable_by_profile($1, $2)")
        .bind(second_id)
        .bind(returned_id)
        .fetch_one(&pool)
        .await
        .expect("authorable query");
    assert!(authorable, "the creator can author its new map immediately");
}
