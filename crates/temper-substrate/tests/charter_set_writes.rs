#![cfg(feature = "artifact-tests")]
//! `cogmap_charter_set` replaces an empty genesis telos's blocks with a role-tagged charter, and is
//! idempotent on body merkle: re-delivering the SAME charter content yields the SAME `body_hash` (the
//! reconciler's diff key, Task 5) even though the lower-level fire always emits a `charter_set` event.
//!
//! ONNX-FREE by construction: the telos is genesis'd EMPTY (`charter: &[]`, no embed call), and the
//! charter blocks are built from synthetic, already-embedded `IncomingChunk`s via
//! `prepare_block_from_chunks` (which carries the embedding verbatim, no ort dylib). Isolated ephemeral
//! DB via `temper_substrate::MIGRATOR`.
mod common;

use temper_substrate::content;
use temper_substrate::events::{fire, EventContext, SeedAction};
use temper_substrate::ids::{EntityId, ProfileId};
use temper_substrate::scenario::bootseed;
use temper_substrate::{readback, writes};
use uuid::Uuid;

/// Boot a canonical owner profile + emitter entity inline (mirrors `cogmap_genesis_charter.rs`).
async fn seed_actor(pool: &sqlx::PgPool) -> (Uuid, Uuid) {
    let profile: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) \
         VALUES ('owner', 'Owner') RETURNING id",
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

/// sha256 hex of `s` — the content_hash a real chunker would assign. Computed independently here so the
/// test can recompute the expected merkle from the same hashes (sha2 is a substrate dependency; no `hex`).
fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(s.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// The 3-block charter: (role, prose) at seq 0..2.
fn charter_specs() -> [(&'static str, &'static str); 3] {
    [
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
    ]
}

/// Build a fresh `PreparedBlock` set for the charter — fresh block/chunk ids each call (so a re-delivery
/// does not PK-conflict) but IDENTICAL `content_hash`es (sha256 of the fixed prose), so the body merkle is
/// stable across deliveries. Each block carries one synthetic, already-embedded chunk (ONNX-free).
fn build_charter() -> Vec<content::PreparedBlock> {
    charter_specs()
        .into_iter()
        .enumerate()
        .map(|(i, (role, prose))| {
            let chunk = content::IncomingChunk {
                chunk_index: 0,
                content_hash: sha256_hex(prose),
                content: prose.to_string(),
                embedding: vec![0.1f32; 768],
                embedded_with: None,
                header_path: String::new(),
                heading_depth: 0,
            };
            content::prepare_block_from_chunks(i as i32, Some(role), vec![chunk])
        })
        .collect()
}

/// The expected resource body merkle for a charter block set (each block's chunk hashes, in order).
fn expected_body_hash(blocks: &[content::PreparedBlock]) -> String {
    let per_block: Vec<Vec<String>> = blocks
        .iter()
        .map(|b| b.chunks.iter().map(|c| c.content_hash.clone()).collect())
        .collect();
    content::body_hash_from_block_chunk_hashes(&per_block)
}

/// The live block roles of a telos in seq order (folded blocks excluded).
async fn live_roles(pool: &sqlx::PgPool, telos: Uuid) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT p.property_value #>> '{}' \
           FROM kb_content_blocks b \
           JOIN kb_properties p \
             ON p.owner_table = 'kb_content_blocks' AND p.owner_id = b.id \
            AND p.property_key = 'block_role' AND NOT p.is_folded \
          WHERE b.resource_id = $1 AND NOT b.is_folded \
          ORDER BY b.seq",
    )
    .bind(telos)
    .fetch_all(pool)
    .await
    .unwrap()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn charter_set_populates_empty_telos_then_idempotent(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = seed_actor(&pool).await;
    let emitter = EntityId::from(emitter);

    // 1. genesis a cogmap with an EMPTY telos (charter:[] ⇒ zero blocks, NO embed call).
    let mut conn = pool.acquire().await.unwrap();
    let (cogmap, genesis_telos) = fire(
        &mut conn,
        SeedAction::CogmapGenesis {
            name: "l0-charter-cogmap",
            telos_title: "L0 telos",
            charter: &[],
            cogmap_id: None,
            telos_resource_id: None,
            owner: ProfileId::from(owner),
            emitter,
        },
    )
    .await
    .unwrap()
    .cogmap_genesis()
    .unwrap();
    drop(conn);

    // a fresh genesis telos has NO live blocks and an empty-body merkle (sha256(""), not NULL).
    assert!(live_roles(&pool, genesis_telos.uuid()).await.is_empty());
    let empty = readback::telos_charter_state(&mut pool.acquire().await.unwrap(), cogmap)
        .await
        .unwrap();
    assert_eq!(empty.telos_resource_id, genesis_telos);
    assert_eq!(
        empty.body_hash.as_deref(),
        Some(content::body_hash_from_block_chunk_hashes(&[]).as_str())
    );

    // 2. deliver the 3-block charter through the substrate write-path wrapper.
    let blocks = build_charter();
    let mut tx = pool.begin().await.unwrap();
    let telos =
        writes::set_charter_in_tx(&mut tx, cogmap, &blocks, emitter, EventContext::default())
            .await
            .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        telos, genesis_telos,
        "charter_set returns the cogmap's telos id"
    );

    // 3. the telos now has exactly the 3 role-tagged blocks in seq order.
    assert_eq!(
        live_roles(&pool, telos.uuid()).await,
        vec!["statement", "question", "framing"]
    );

    // 4. the telos body_hash equals the multi-block merkle of the delivered charter.
    let expect = expected_body_hash(&blocks);
    let state = readback::telos_charter_state(&mut pool.acquire().await.unwrap(), cogmap)
        .await
        .unwrap();
    assert_eq!(state.body_hash.as_deref(), Some(expect.as_str()));
    assert_ne!(
        state.body_hash, empty.body_hash,
        "a non-empty charter never matches the empty-telos hash (first delivery is a real change)"
    );

    // 5. IDEMPOTENCY: re-deliver the SAME charter content (fresh ids, identical content_hashes). The
    //    fold-then-reproject re-projects at seq 0..2 over the now-folded prior blocks (this is why the
    //    seq-uniqueness index is partial on NOT is_folded).
    let blocks2 = build_charter();
    let mut tx2 = pool.begin().await.unwrap();
    writes::set_charter_in_tx(&mut tx2, cogmap, &blocks2, emitter, EventContext::default())
        .await
        .unwrap();
    tx2.commit().await.unwrap();

    // same content ⇒ same merkle (the reconciler's diff key) ⇒ the caller would skip; here we re-fire
    // unconditionally to prove the LOWER-level guarantee.
    let state2 = readback::telos_charter_state(&mut pool.acquire().await.unwrap(), cogmap)
        .await
        .unwrap();
    assert_eq!(
        state2.body_hash, state.body_hash,
        "same content ⇒ same body merkle"
    );
    assert_eq!(
        live_roles(&pool, telos.uuid()).await,
        vec!["statement", "question", "framing"],
        "re-delivery leaves exactly the 3 live role-tagged blocks (prior set folded, not duplicated)"
    );

    // the substrate fired charter_set on EACH raw delivery (it always fires; the caller's
    // skip-on-unchanged is Task 5's concern), so exactly 2 events after two raw deliveries.
    let charter_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'charter_set'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        charter_events, 2,
        "two raw deliveries ⇒ two charter_set events"
    );
}
