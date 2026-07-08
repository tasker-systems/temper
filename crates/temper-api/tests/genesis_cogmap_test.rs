#![cfg(feature = "test-db")]
//! `DbBackend::create_cognitive_map` — cognitive-map genesis (org-provisioning Chunk 4). Drives the
//! backend command DIRECTLY. Create itself is open to any authenticated profile (the surface admin
//! gate is gone), but the backend enforces reserved-id hardening: a caller-supplied
//! `cogmap_id`/`telos_resource_id` is honored ONLY for a system-admin; a non-admin is server-minted.
//! So the explicit-id cases here make the caller an admin (`make_admin`), and `non_admin_...` covers
//! the server-mint path. Telos charter blocks are pre-embedded with synthetic (recognizable) chunks,
//! so every case runs in the plain `test-db` tier — NO ONNX (the genesis path is a pure
//! event+projection write).
//!
//! Headline invariants:
//! - genesis creates a cogmap + telos resource (`created: true`); the new map is then RECONCILABLE.
//! - re-genesis at the same id is an idempotent no-op (`created: false`) — ZERO new `cogmap_seeded`
//!   events.
//! - genesis with no telos is born with an empty charter (deliverable later via reconcile).

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::{CogmapId, ProfileId};
use temper_core::types::ingest::{pack_chunks, PackedChunk};
use temper_core::types::reconcile::{
    CreateCogmapRequest, ReconcileCogmapRequest, ReconcileEntry, ReconcileTelos,
    ReconcileTelosBlock,
};
use temper_services::backend::DbBackend;
use temper_workflow::operations::{Backend, CreateCognitiveMap, ReconcileCognitiveMap, Surface};

// ── builders ────────────────────────────────────────────────────────────────────

/// Build a synthetic pre-embedded telos block. `hash_seed` is zero-padded to 64 chars so each block has
/// a stable, distinct `content_hash`. Mirrors `reconcile_charter_test.rs` — NO ONNX.
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

/// A genesis request with caller-supplied ids (so the test can assert + re-deliver the same id).
fn genesis_request(
    cogmap_id: Uuid,
    telos_resource_id: Uuid,
    telos: Option<ReconcileTelos>,
) -> CreateCogmapRequest {
    CreateCogmapRequest {
        cogmap_id: Some(cogmap_id),
        telos_resource_id: Some(telos_resource_id),
        name: "Org provisioning map".to_string(),
        telos_title: "Org telos".to_string(),
        telos,
    }
}

/// Build a pre-embedded reconcile entry with a single synthetic chunk (for the reconcilable follow-up).
fn entry(id: Uuid, origin_uri: &str, title: &str, body: &str, hash_seed: &str) -> ReconcileEntry {
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: body.to_string(),
        content_hash: format!("{hash_seed:0>64}"),
        embedding: vec![0.1; 768],
    };
    let content_hash = temper_substrate::content::body_hash_from_chunk_hashes(
        std::slice::from_ref(&chunk.content_hash),
    );
    let chunks_packed = pack_chunks(std::slice::from_ref(&chunk)).expect("pack");
    ReconcileEntry {
        id,
        origin_uri: origin_uri.to_string(),
        title: title.to_string(),
        doc_type: "kernel_landmark".to_string(),
        content_hash,
        chunks_packed,
        facets: serde_json::json!({ "layer": "concept" }),
        edges: vec![],
    }
}

async fn system_profile(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle = 'system'")
        .fetch_one(pool)
        .await
        .expect("system profile must exist")
}

/// Mint a fresh profile (handle-unique). Returns the new profile id.
async fn mint_profile(pool: &PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("mint profile")
}

/// Make `profile` a system-admin: ensure the `temper-system` gating team exists, make `profile` its
/// owner, and point `gating_team_slug` at it. Mirrors the e2e `enable_invite_only`. Required for the
/// explicit-id genesis path — a caller-supplied cogmap/telos id is honored only for a system-admin
/// (a non-admin is server-minted: the reserved-id guard).
async fn make_admin(pool: &PgPool, profile: Uuid) {
    let team_id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system', 'Temper System')
         ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("ensure gating team");
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner')
         ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(team_id)
    .bind(profile)
    .execute(pool)
    .await
    .expect("make owner of gating team");
    sqlx::query(
        "UPDATE kb_system_settings SET gating_team_slug = 'temper-system', updated = now()",
    )
    .execute(pool)
    .await
    .expect("point gating_team_slug at temper-system");
}

/// An ADMIN backend for the explicit-id genesis/reconcile cases: the system profile promoted to
/// system-admin so its caller-supplied ids are honored (see `make_admin`).
async fn backend(pool: &PgPool) -> DbBackend {
    let sys = system_profile(pool).await;
    make_admin(pool, sys).await;
    DbBackend::new(pool.clone(), ProfileId::from(sys))
}

fn genesis_cmd(req: CreateCogmapRequest) -> CreateCognitiveMap {
    CreateCognitiveMap {
        request: req,
        origin: Surface::ApiHttp,
    }
}

async fn cogmap_seeded_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events ev \
           JOIN kb_event_types et ON et.id = ev.event_type_id \
          WHERE et.name = 'cogmap_seeded'",
    )
    .fetch_one(pool)
    .await
    .expect("count cogmap_seeded events")
}

async fn cogmap_exists(pool: &PgPool, cogmap_id: Uuid) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM kb_cogmaps WHERE id = $1)")
        .bind(cogmap_id)
        .fetch_one(pool)
        .await
        .expect("exists query")
}

async fn resource_exists(pool: &PgPool, resource_id: Uuid) -> bool {
    sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM kb_resources WHERE id = $1)")
        .bind(resource_id)
        .fetch_one(pool)
        .await
        .expect("exists query")
}

// ── (a) genesis creates a new cogmap + telos, then the map is reconcilable ────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn genesis_creates_cogmap_and_telos_then_reconcilable(pool: PgPool) {
    let be = backend(&pool).await;
    let cogmap_id = Uuid::now_v7();
    let telos_id = Uuid::now_v7();

    let out = be
        .create_cognitive_map(genesis_cmd(genesis_request(
            cogmap_id,
            telos_id,
            Some(three_block_telos()),
        )))
        .await
        .expect("genesis")
        .value;

    assert!(out.created, "first genesis creates the map");
    assert_eq!(out.cogmap_id, cogmap_id, "echoes the supplied cogmap id");
    assert_eq!(
        out.telos_resource_id, telos_id,
        "echoes the supplied telos id"
    );
    assert!(cogmap_exists(&pool, cogmap_id).await, "cogmap row written");
    assert!(
        resource_exists(&pool, telos_id).await,
        "telos resource row written"
    );

    // The new map is RECONCILABLE: a kernel-slice reconcile applies against it.
    let recon = ReconcileCognitiveMap {
        cogmap_id: CogmapId::from(cogmap_id),
        request: ReconcileCogmapRequest {
            entries: vec![entry(
                Uuid::now_v7(),
                "temper://kernel/concept/landmark",
                "landmark",
                "A landmark in the freshly-born map.",
                "aa",
            )],
            fold_resources: vec![],
            fold_edges: vec![],
            telos: None,
        },
        act: Default::default(),
        origin: Surface::ApiHttp,
    };
    let recon_out = be
        .reconcile_cognitive_map(recon)
        .await
        .expect("reconcile the new map")
        .value;
    assert_eq!(
        (recon_out.created, recon_out.updated, recon_out.folded),
        (1, 0, 0),
        "the freshly-born map accepts a reconcile (one landmark created)"
    );
}

// ── (b) re-genesis at the same id → idempotent no-op (zero new cogmap_seeded events) ──

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn re_genesis_is_idempotent_no_op(pool: PgPool) {
    let be = backend(&pool).await;
    let cogmap_id = Uuid::now_v7();
    let telos_id = Uuid::now_v7();

    let first = be
        .create_cognitive_map(genesis_cmd(genesis_request(
            cogmap_id,
            telos_id,
            Some(three_block_telos()),
        )))
        .await
        .expect("first genesis")
        .value;
    assert!(first.created);

    let seeded_before = cogmap_seeded_count(&pool).await;

    // Re-genesis at the SAME id → no-op. Even a DIFFERENT telos id in the request must be ignored: the
    // stored telos id is returned, and nothing is written.
    let second = be
        .create_cognitive_map(genesis_cmd(genesis_request(
            cogmap_id,
            Uuid::now_v7(),
            Some(three_block_telos()),
        )))
        .await
        .expect("second genesis")
        .value;

    assert!(!second.created, "re-genesis is a no-op");
    assert_eq!(second.cogmap_id, cogmap_id);
    assert_eq!(
        second.telos_resource_id, telos_id,
        "returns the STORED telos id, not the request's"
    );

    let seeded_after = cogmap_seeded_count(&pool).await;
    assert_eq!(
        seeded_after, seeded_before,
        "re-genesis fires ZERO new cogmap_seeded events; before={seeded_before} after={seeded_after}"
    );
}

// ── (c) genesis with no telos → born with an empty charter ───────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn genesis_without_telos_creates_empty_charter_map(pool: PgPool) {
    let be = backend(&pool).await;
    let cogmap_id = Uuid::now_v7();
    let telos_id = Uuid::now_v7();

    let out = be
        .create_cognitive_map(genesis_cmd(genesis_request(cogmap_id, telos_id, None)))
        .await
        .expect("empty-charter genesis")
        .value;

    assert!(out.created);
    assert!(cogmap_exists(&pool, cogmap_id).await);
    assert!(
        resource_exists(&pool, telos_id).await,
        "telos resource exists"
    );
    // The telos resource has no charter blocks yet.
    let block_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_content_blocks WHERE resource_id = $1")
            .bind(telos_id)
            .fetch_one(&pool)
            .await
            .expect("count blocks");
    assert_eq!(block_count, 0, "empty-charter genesis writes no blocks");
}

// ── (e) creator seeding: the INVOKING profile gets a read+write+grant bootstrap grant ─

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn genesis_seeds_creator_grant(pool: PgPool) {
    // A distinct creator profile — NOT the system actor genesis fires under. The backend's
    // `self.profile_id` is the invoking caller; creator seeding (§3.B) keys on it. Admin so the
    // supplied id is honored (the grant is then asserted at that id).
    let creator = mint_profile(&pool, "creator-admin").await;
    make_admin(&pool, creator).await;
    let be = DbBackend::new(pool.clone(), ProfileId::from(creator));

    let cogmap_id = Uuid::now_v7();
    let telos_id = Uuid::now_v7();
    let out = be
        .create_cognitive_map(genesis_cmd(genesis_request(cogmap_id, telos_id, None)))
        .await
        .expect("genesis")
        .value;
    assert!(out.created);

    // The creator holds an explicit read+write+grant bootstrap grant on the new map (delete NOT set).
    let (can_read, can_write, can_grant, can_delete): (bool, bool, bool, bool) = sqlx::query_as(
        "SELECT can_read, can_write, can_grant, can_delete FROM kb_access_grants \
           WHERE subject_table = 'kb_cogmaps' AND subject_id = $1 \
             AND principal_table = 'kb_profiles' AND principal_id = $2",
    )
    .bind(cogmap_id)
    .bind(creator)
    .fetch_one(&pool)
    .await
    .expect("creator grant row must exist");
    assert!(
        can_read && can_write && can_grant,
        "creator gets read+write+grant"
    );
    assert!(!can_delete, "creator seed does not confer delete");
}

// ── (d) backend-minted id when the request omits ids ─────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn genesis_mints_ids_when_absent(pool: PgPool) {
    let be = backend(&pool).await;
    let req = CreateCogmapRequest {
        cogmap_id: None,
        telos_resource_id: None,
        name: "Minted map".to_string(),
        telos_title: "Minted telos".to_string(),
        telos: None,
    };
    let out = be
        .create_cognitive_map(genesis_cmd(req))
        .await
        .expect("genesis with minted ids")
        .value;
    assert!(out.created);
    assert_ne!(out.cogmap_id, Uuid::nil(), "minted a real cogmap id");
    assert!(cogmap_exists(&pool, out.cogmap_id).await);
    assert!(resource_exists(&pool, out.telos_resource_id).await);
}

// ── (f) reserved-id hardening: a NON-admin's supplied ids are ignored and server-minted ──────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_admin_genesis_server_mints_ids(pool: PgPool) {
    // A plain (non-admin) caller — no `make_admin`.
    let creator = mint_profile(&pool, "non-admin-creator").await;
    let be = DbBackend::new(pool.clone(), ProfileId::from(creator));

    let supplied_cogmap = Uuid::now_v7();
    let supplied_telos = Uuid::now_v7();
    let out = be
        .create_cognitive_map(genesis_cmd(genesis_request(
            supplied_cogmap,
            supplied_telos,
            None,
        )))
        .await
        .expect("non-admin genesis")
        .value;

    assert!(out.created, "the map is created");
    assert_ne!(
        out.cogmap_id, supplied_cogmap,
        "a non-admin's supplied cogmap id is ignored (server-minted)"
    );
    assert_ne!(
        out.telos_resource_id, supplied_telos,
        "a non-admin's supplied telos id is ignored (server-minted)"
    );
    assert!(
        !cogmap_exists(&pool, supplied_cogmap).await,
        "nothing written at the supplied cogmap id"
    );
    assert!(
        cogmap_exists(&pool, out.cogmap_id).await,
        "the map exists at the server-minted id"
    );

    // The creator grant lands on the server-minted id (so the creator can author its new map).
    let has_grant: bool =
        sqlx::query_scalar::<_, bool>("SELECT can('kb_profiles', $1, 'grant', 'kb_cogmaps', $2)")
            .bind(creator)
            .bind(out.cogmap_id)
            .fetch_one(&pool)
            .await
            .expect("can grant query");
    assert!(has_grant, "creator holds grant on the server-minted map");
}
