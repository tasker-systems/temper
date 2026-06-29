#![cfg(feature = "artifact-tests")]
//! Salience payoff: delivering the telos charter gives the telos resource embeddings, so
//! `cogmap_region_telos_alignment` (the function the materialize path uses to populate the stored
//! `kb_cogmap_regions.telos_alignment` column) goes from NULL (empty telos) to a finite value.
//!
//! ONNX-free: the charter chunks carry synthetic embeddings verbatim via `prepare_block_from_chunks`.
//! Isolated ephemeral DB via `temper_substrate::MIGRATOR`.
mod common;

use temper_substrate::content;
use temper_substrate::events::{fire, EventContext, SeedAction};
use temper_substrate::ids::{CogmapId, EntityId, ProfileId};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes;
use uuid::Uuid;

/// sha256 hex of `s` — the content_hash a real chunker would assign.
fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Boot a fresh profile + entity inline (mirrors `charter_set_writes.rs`).
async fn seed_actor(pool: &sqlx::PgPool) -> (Uuid, Uuid) {
    let profile: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name, system_access) \
         VALUES ('owner', 'Owner', 'approved'::system_access) RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let entity: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1, 'agent#1', '{}'::jsonb) RETURNING id",
    )
    .bind(profile)
    .fetch_one(pool)
    .await
    .unwrap();
    (profile, entity)
}

/// Build 3 charter blocks with synthetic non-zero embeddings (ONNX-free).
fn build_charter_blocks() -> Vec<content::PreparedBlock> {
    let specs: [(&str, &str); 3] = [
        (
            "statement",
            "Orient an arriving agent to what this map is for.",
        ),
        (
            "question",
            "Where am I, and what is this cognitive map about?",
        ),
        (
            "framing",
            "This map is self-referential: it describes temper itself.",
        ),
    ];
    specs
        .into_iter()
        .enumerate()
        .map(|(i, (role, prose))| {
            let chunk = content::IncomingChunk {
                chunk_index: 0,
                content_hash: sha256_hex(prose),
                content: prose.to_string(),
                embedding: vec![0.2f32; 768],
                header_path: String::new(),
                heading_depth: 0,
            };
            content::prepare_block_from_chunks(i as i32, Some(role), vec![chunk])
        })
        .collect()
}

/// Call `cogmap_region_telos_alignment(region, cogmap)` and return `Option<f64>` (NULL → None).
async fn query_telos_alignment(pool: &sqlx::PgPool, region: Uuid, cogmap: Uuid) -> Option<f64> {
    sqlx::query_scalar("SELECT cogmap_region_telos_alignment($1, $2)")
        .bind(region)
        .bind(cogmap)
        .fetch_one(pool)
        .await
        .expect("cogmap_region_telos_alignment query")
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn charter_delivery_makes_telos_alignment_computable(pool: sqlx::PgPool) {
    // ── 1. boot seed + empty-charter cogmap ──────────────────────────────────────────────────────
    bootseed::seed_system(&pool).await.unwrap();
    let (_, emitter_uuid) = seed_actor(&pool).await;
    let emitter = EntityId::from(emitter_uuid);

    // The owner profile is needed by CogmapGenesis; fetch by the handle we just inserted.
    let owner_uuid: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='owner'")
        .fetch_one(&pool)
        .await
        .unwrap();
    let owner = ProfileId::from(owner_uuid);

    // Genesis a cogmap with an EMPTY telos (charter:[] → zero blocks, zero chunks, no embed call).
    let mut conn = pool.acquire().await.unwrap();
    let (cogmap_id, telos_id) = fire(
        &mut conn,
        SeedAction::CogmapGenesis {
            name: "telos-alignment-cogmap",
            telos_title: "Alignment Test Telos",
            charter: &[],
            cogmap_id: None,
            telos_resource_id: None,
            owner,
            emitter,
        },
    )
    .await
    .unwrap()
    .cogmap_genesis()
    .unwrap();
    drop(conn);

    let cogmap = cogmap_id.uuid();
    let telos = telos_id.uuid();

    // Verify: telos starts with ZERO chunks (truly empty — no embed call ever touched it).
    let telos_chunk_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_chunks WHERE resource_id = $1")
            .bind(telos)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        telos_chunk_count, 0,
        "empty-charter genesis ⇒ zero chunks on the telos"
    );

    // ── 2. insert one region with a non-zero centroid ─────────────────────────────────────────────
    // Fetch the global `telos-default` lens (seeded by bootseed; cogmap_id IS NULL = global).
    let lens: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_cogmap_lenses WHERE name='telos-default' AND cogmap_id IS NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("global telos-default lens");

    // Any event will do for the FK columns; genesis fired at least one.
    let event: Uuid = sqlx::query_scalar("SELECT id FROM kb_events LIMIT 1")
        .fetch_one(&pool)
        .await
        .expect("any event for FK");

    // Non-zero centroid: all-0.1 vector.  All-zero would make <=> undefined (NaN/NULL).
    let region: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmap_regions
           (cogmap_id, lens_id, centroid, salience, centrality, content_cohesion,
            internal_tension, reference_standing, telos_alignment, label, member_count,
            asserted_by_event_id, last_event_id, is_folded)
         VALUES ($1, $2,
            array_fill(0.1::double precision, ARRAY[768])::vector,
            0.5, 4.0, 0.25, 1.5, 7.0, NULL, 'r', 2, $3, $3, false)
         RETURNING id",
    )
    .bind(cogmap)
    .bind(lens)
    .bind(event)
    .fetch_one(&pool)
    .await
    .expect("insert region with non-zero centroid");

    // ── 3. BEFORE delivery: function returns NULL (telos has no chunks) ───────────────────────────
    let before = query_telos_alignment(&pool, region, cogmap).await;
    assert!(
        before.is_none(),
        "before charter delivery: telos_alignment must be NULL (telos has no chunks), got {before:?}"
    );

    // ── 4. deliver the 3-block charter ────────────────────────────────────────────────────────────
    let blocks = build_charter_blocks();
    let mut tx = pool.begin().await.unwrap();
    let returned_telos = writes::set_charter_in_tx(
        &mut tx,
        CogmapId::from(cogmap),
        &blocks,
        emitter,
        EventContext::default(),
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        returned_telos.uuid(),
        telos,
        "set_charter_in_tx returns the cogmap's telos id"
    );

    // ── 5. AFTER delivery: function returns a finite value ────────────────────────────────────────
    // The telos now has 3 embedded chunks (0.2f32 each); the centroid is all-0.1.
    // Both vectors are uniform → cosine distance ≈ 0 → telos_alignment ≈ 1.0.
    let after = query_telos_alignment(&pool, region, cogmap).await;
    assert!(
        after.is_some(),
        "after charter delivery: telos_alignment must be Some (telos has chunks now)"
    );
    let v = after.unwrap();
    assert!(v.is_finite(), "telos_alignment must be finite, got {v}");
    assert!(
        (-1.0..=1.0).contains(&v),
        "telos_alignment must be a valid cosine similarity in [-1, 1], got {v}"
    );
}
