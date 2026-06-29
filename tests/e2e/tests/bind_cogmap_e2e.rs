#![cfg(feature = "test-db")]
//! `POST /api/cognitive-maps/{id}/teams` + `DELETE …/{team_id}` end-to-end: drives the REAL Axum server
//! (in-process), real Postgres, real JWT auth. Cogmap↔team binding (org-provisioning Chunk 5). Binding
//! writes a `kb_team_cogmaps` row, which widens the map's producer-intersection reach
//! (`resources_accessible_to_cogmap`). The surface is SERVICE-DIRECT and admin-gated (mirrors team
//! membership). The optional telos charter is PRE-EMBEDDED with synthetic chunks → NO ONNX, so this runs
//! on plain `cargo make test-e2e` (mirroring `genesis_cogmap_e2e.rs`).
//!
//! `common::enable_invite_only` configures the gating team AND makes the given profile its owner/admin,
//! so `is_system_admin` is true for the admin and false for a second (watcher) user — exercising both the
//! allow and deny paths of the bind gate.

mod common;

use reqwest::StatusCode;
use uuid::Uuid;

use temper_core::types::ingest::{pack_chunks, PackedChunk};
use temper_core::types::reconcile::{CreateCogmapRequest, ReconcileTelos, ReconcileTelosBlock};

// ── chunk fabrication (mirrors genesis_cogmap_e2e.rs; the e2e crate cannot depend on temper-substrate). ──

/// A synthetic pre-embedded telos block. Genesis does not diff the charter, so any well-formed chunk works.
fn telos_block(role: &str, content: &str, hash_seed: &str) -> ReconcileTelosBlock {
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: content.to_string(),
        content_hash: format!("{hash_seed:0>64}"),
        embedding: vec![0.1f32; 768],
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

/// A genesis request with fixed ids (so the cogmap/telos ids are predictable for assertions).
fn genesis_request() -> CreateCogmapRequest {
    CreateCogmapRequest {
        cogmap_id: Some(Uuid::from_u128(0x019f0bbb_2ace_76cb_b1fc_260239dd16a5)),
        telos_resource_id: Some(Uuid::from_u128(0x019f0bbb_2acf_7c45_bd12_a2a7152644a1)),
        name: "Bindable map".to_string(),
        telos_title: "Bind telos".to_string(),
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
    body["id"]
        .as_str()
        .expect("profile id missing")
        .parse()
        .expect("profile id parse")
}

/// Create a team with a single resource visible to it (`kb_resource_access` team grant, the first branch
/// of `vis_team`). Returns `(team_id, resource_id)`. `granted_by` must be a real profile.
async fn team_with_visible_resource(pool: &sqlx::PgPool, granted_by: Uuid) -> (Uuid, Uuid) {
    let team_id = Uuid::now_v7();
    let slug = format!("bind-team-{}", &Uuid::new_v4().simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_teams (id, slug, name) VALUES ($1, $2, $2)")
        .bind(team_id)
        .bind(&slug)
        .execute(pool)
        .await
        .expect("insert team");

    let resource_id = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_resources (id, title, origin_uri) VALUES ($1, 'Team note', $2)")
        .bind(resource_id)
        .bind(format!("test://{resource_id}"))
        .execute(pool)
        .await
        .expect("insert resource");

    sqlx::query(
        "INSERT INTO kb_resource_access \
            (resource_id, anchor_table, anchor_id, can_read, granted_by_profile_id) \
         VALUES ($1, 'kb_teams', $2, true, $3)",
    )
    .bind(resource_id)
    .bind(team_id)
    .bind(granted_by)
    .execute(pool)
    .await
    .expect("grant team read");

    (team_id, resource_id)
}

/// `true` when `resources_accessible_to_cogmap(cogmap)` includes `resource_id`.
async fn resource_accessible(pool: &sqlx::PgPool, cogmap_id: Uuid, resource_id: Uuid) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM resources_accessible_to_cogmap($1) r WHERE r.resource_id = $2)",
    )
    .bind(cogmap_id)
    .bind(resource_id)
    .fetch_one(pool)
    .await
    .expect("resources_accessible_to_cogmap query")
}

/// `true` when a `kb_team_cogmaps(cogmap_id, team_id)` row exists.
async fn binding_exists(pool: &sqlx::PgPool, cogmap_id: Uuid, team_id: Uuid) -> bool {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM kb_team_cogmaps WHERE cogmap_id = $1 AND team_id = $2)",
    )
    .bind(cogmap_id)
    .bind(team_id)
    .fetch_one(pool)
    .await
    .expect("kb_team_cogmaps exists query")
}

// ── (a) admin bind → resource accessible; re-bind is an idempotent no-op ──────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn admin_bind_makes_resource_accessible_and_is_idempotent(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let admin_id = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, admin_id).await;

    let (team_id, resource_id) = team_with_visible_resource(&pool, admin_id).await;

    let req = genesis_request();
    let genesis = app
        .client
        .cognitive_maps()
        .create_cognitive_map(&req)
        .await
        .expect("admin genesis should succeed");
    let cogmap_id = genesis.cogmap_id;

    // Before binding, the team's resource is NOT reachable by the map (empty join ⇒ no shared reach).
    assert!(
        !resource_accessible(&pool, cogmap_id, resource_id).await,
        "an unbound map must not reach the team's resource"
    );

    // First bind: writes the row.
    let out1 = app
        .client
        .cognitive_maps()
        .bind_team(
            cogmap_id,
            &temper_core::types::cognitive_maps::BindTeamRequest { team_id },
        )
        .await
        .expect("admin bind should succeed");
    assert!(out1.bound, "first bind inserts the binding");
    assert_eq!(out1.cogmap_id, cogmap_id);
    assert_eq!(out1.team_id, team_id);

    // The resource is now reachable through the binding.
    assert!(
        resource_accessible(&pool, cogmap_id, resource_id).await,
        "after binding, the map reaches the team's resource"
    );

    // Re-bind: idempotent no-op.
    let out2 = app
        .client
        .cognitive_maps()
        .bind_team(
            cogmap_id,
            &temper_core::types::cognitive_maps::BindTeamRequest { team_id },
        )
        .await
        .expect("second admin bind should succeed");
    assert!(!out2.bound, "re-bind is an idempotent no-op (bound: false)");
}

// ── (b) unbind reverts accessibility; unbinding a non-existent binding is a no-op ─────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn unbind_reverts_accessibility_and_is_safe(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let admin_id = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, admin_id).await;

    let (team_id, resource_id) = team_with_visible_resource(&pool, admin_id).await;

    let cogmap_id = app
        .client
        .cognitive_maps()
        .create_cognitive_map(&genesis_request())
        .await
        .expect("admin genesis should succeed")
        .cogmap_id;

    app.client
        .cognitive_maps()
        .bind_team(
            cogmap_id,
            &temper_core::types::cognitive_maps::BindTeamRequest { team_id },
        )
        .await
        .expect("admin bind should succeed");
    assert!(resource_accessible(&pool, cogmap_id, resource_id).await);

    // Unbind: deletes the row and reverts reach.
    let out = app
        .client
        .cognitive_maps()
        .unbind_team(cogmap_id, team_id)
        .await
        .expect("admin unbind should succeed");
    assert!(out.unbound, "unbind deletes the binding (unbound: true)");
    assert!(
        !resource_accessible(&pool, cogmap_id, resource_id).await,
        "after unbinding, the map no longer reaches the team's resource"
    );

    // Unbind again: no-op safe.
    let out2 = app
        .client
        .cognitive_maps()
        .unbind_team(cogmap_id, team_id)
        .await
        .expect("second unbind should succeed");
    assert!(
        !out2.unbound,
        "unbinding a non-existent binding is a no-op (unbound: false)"
    );
}

// ── (c) non-admin bind is denied (403) and writes nothing ────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_admin_bind_is_denied_and_writes_nothing(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let admin_id = provision_profile(&app, &app.token).await;
    common::enable_invite_only(&pool, admin_id).await;

    let (team_id, _resource_id) = team_with_visible_resource(&pool, admin_id).await;

    let cogmap_id = app
        .client
        .cognitive_maps()
        .create_cognitive_map(&genesis_request())
        .await
        .expect("admin genesis should succeed")
        .cogmap_id;

    // A SECOND user with system access (a `watcher` of temper-system) but NOT admin: passes the
    // system-access middleware, reaches the handler, and is denied by the service `is_system_admin` gate.
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

    let resp = app
        .reqwest_client
        .post(app.url(&format!("/api/cognitive-maps/{cogmap_id}/teams")))
        .header("Authorization", format!("Bearer {second_token}"))
        .json(&serde_json::json!({ "team_id": team_id }))
        .send()
        .await
        .expect("request failed");

    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "a non-admin bind must be denied by the service is_system_admin gate"
    );
    let body: serde_json::Value = resp.json().await.expect("json parse");
    assert_eq!(body["error"]["code"], "FORBIDDEN");

    // No binding row was written.
    assert!(
        !binding_exists(&pool, cogmap_id, team_id).await,
        "a denied bind must write nothing"
    );
}
