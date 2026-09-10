#![cfg(feature = "artifact-tests")]
//! Witnesses for the block-addressed read surface — `readback::block_read`, the three-state
//! resolution every read surface shares (the defined-dangling-state design, D-D1/D-D2). A block
//! address resolves `live` (identity, chunk identities, provenance) / `folded` (the envelope:
//! the fold event's disposition map plus the already-persisted attribution history) / `absent`,
//! and a not-visible home denies existence. Folded-state fixtures are seeded through the real
//! whole-body replace write path so folded rows carry real `resource_reblocked` events and real
//! disposition maps; the map-less fold is seeded through the real `charter_set` write path.
//!
//! Fixtures duplicate this suite's convention, cribbed from `whole_body_replace.rs`
//! (system_actor / make_home / create_two_block / update_body / source / blocks_of) and
//! `charter_set_writes.rs` (the charter fold).
//!
//! ONNX-dependent. Isolated ephemeral DB via `temper_substrate::MIGRATOR`.

mod common;

use temper_core::types::provenance::{BlockFoldDisposition, BlockRead};
use temper_substrate::content;
use temper_substrate::events::{fire, EventContext, SeedAction};
use temper_substrate::ids::{BlockId, EntityId, ProfileId};
use temper_substrate::payloads::{AnchorRef, Incorporation, ProvenanceSource};
use temper_substrate::readback::{self, ReadbackError};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{
    self, AppendParams, CreateMode, CreateParams, FinalizeParams, UpdateParams,
};
use uuid::Uuid;

const SECTION_A: &str = "# Alpha\n\nAlpha body paragraph.\n";
const SECTION_B: &str = "## Beta\n\nBeta body paragraph.\n";

// ── fixture helpers (duplicated per file, per this suite's convention) ──────────────────────

async fn system_actor(
    pool: &sqlx::PgPool,
) -> (
    temper_substrate::ids::ProfileId,
    temper_substrate::ids::EntityId,
) {
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
    (
        temper_substrate::ids::ProfileId::from(profile),
        temper_substrate::ids::EntityId::from(entity),
    )
}

/// A second principal with its own emitter — the not-visible-home fixture's owner. Mirrors
/// `charter_set_writes.rs::seed_actor`, parameterized by handle.
async fn insert_actor(pool: &sqlx::PgPool, handle: &str) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .unwrap();
    let entity: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name, metadata) \
         VALUES ($1, $2, '{}'::jsonb) RETURNING id",
    )
    .bind(profile)
    .bind(format!("{handle}@cli"))
    .fetch_one(pool)
    .await
    .unwrap();
    (ProfileId::from(profile), EntityId::from(entity))
}

async fn make_home(
    pool: &sqlx::PgPool,
    owner: temper_substrate::ids::ProfileId,
    slug: &str,
) -> AnchorRef {
    let ctx = common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
        .await
        .unwrap();
    AnchorRef::context(temper_substrate::ids::ContextId::from(ctx))
}

/// A two-block resource through the segmented trio (block boundaries chosen by the fixture, not
/// the policy — the one honest way a resource gets blocks that are not section-aligned, or
/// multi-section blocks).
async fn create_two_block(
    pool: &sqlx::PgPool,
    owner: temper_substrate::ids::ProfileId,
    emitter: temper_substrate::ids::EntityId,
    home: &AnchorRef,
    first: &str,
    rest: &str,
    block0_sources: Vec<Incorporation>,
) -> temper_substrate::ids::ResourceId {
    use temper_substrate::events::EventContext;
    let resource = writes::create_resource_with_mode(
        pool,
        CreateParams {
            title: "block-read fixture",
            origin_uri: "temper://block-read/fixture",
            body: first,
            doc_type: "concept",
            home: *home,
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
            sources: block0_sources,
            idempotency_key: None,
        },
        EventContext::default(),
        CreateMode {
            defer: false,
            segmented: true,
        },
    )
    .await
    .unwrap();
    let breadcrumb: Vec<String> = temper_ingest::chunk::chunk_markdown(first)
        .last()
        .filter(|c| !c.header_path.is_empty())
        .map(|c| c.header_path.split(" > ").map(str::to_owned).collect())
        .unwrap_or_default();
    let mut block1 =
        temper_substrate::content::prepare_block_with_prefix(1, None, rest, &breadcrumb).unwrap();
    block1.raw_text = Some(rest.to_owned());
    writes::append_block(
        pool,
        AppendParams {
            resource,
            block: &block1,
            sources: vec![],
            emitter,
        },
    )
    .await
    .unwrap();
    let h0: Vec<String> = temper_ingest::chunk::chunk_markdown(first)
        .iter()
        .map(|c| c.content_hash.clone())
        .collect();
    let h1: Vec<String> = temper_ingest::chunk::chunk_markdown(rest)
        .iter()
        .map(|c| c.content_hash.clone())
        .collect();
    writes::finalize_ingest(
        pool,
        FinalizeParams {
            resource,
            expected_blocks: 2,
            expected_body_hash: temper_substrate::content::body_hash_from_block_chunk_hashes(&[
                h0, h1,
            ]),
            expected_content_hash: None,
            emitter,
        },
    )
    .await
    .unwrap();
    resource
}

async fn update_body(
    pool: &sqlx::PgPool,
    emitter: temper_substrate::ids::EntityId,
    resource: temper_substrate::ids::ResourceId,
    body: &str,
    sources: Vec<Incorporation>,
) {
    writes::update_resource(
        pool,
        UpdateParams {
            resource,
            body: Some(body),
            title: None,
            origin_uri: None,
            properties: &[],
            chunks: None,
            sources,
            content_block: None,
            rehome_to: None,
            emitter,
        },
    )
    .await
    .unwrap();
}

/// (id, seq) of the LIVE blocks, seq order. Folded rows stay history — they are the subject of
/// the folded witnesses, read through `block_read` itself.
async fn blocks_of(
    pool: &sqlx::PgPool,
    resource: temper_substrate::ids::ResourceId,
) -> Vec<(Uuid, i32)> {
    sqlx::query_as(
        "SELECT id, seq FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded ORDER BY seq, id",
    )
    .bind(resource.uuid())
    .fetch_all(pool)
    .await
    .unwrap()
}

fn source(url: &str, seq: i32) -> Incorporation {
    Incorporation {
        source: ProvenanceSource::Remote(url.to_owned()),
        seq,
    }
}

/// The event-type name of an event id — what the folded envelope's `folded_by_event_id`
/// pointer actually walked to.
async fn event_type_of(pool: &sqlx::PgPool, event_id: Uuid) -> String {
    sqlx::query_scalar(
        "SELECT t.name FROM kb_events e \
         JOIN kb_event_types t ON t.id = e.event_type_id WHERE e.id = $1",
    )
    .bind(event_id)
    .fetch_one(pool)
    .await
    .unwrap()
}

// ── the charter fold fixture (cribbed from charter_set_writes.rs) ───────────────────────────

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

/// sha256 hex of `s` — the content_hash a real chunker would assign (charter blocks are built
/// from synthetic, already-embedded chunks, ONNX-free).
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

/// Build a fresh `PreparedBlock` set for the charter — fresh block/chunk ids each call (so a
/// re-delivery does not PK-conflict) but IDENTICAL `content_hash`es.
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

// ── the witnesses ───────────────────────────────────────────────────────────

/// CLAUSE: a live block resolves `live` carrying its born assembly — its current chunks'
/// identity (matching the stored current chunk rows, in order) and its provenance rows, so an
/// addressable block is itemizable without any second read.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_live_block_resolves_with_identity_chunks_and_provenance(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "br-live").await;
    let resource = create_two_block(
        &pool,
        owner,
        emitter,
        &home,
        SECTION_A,
        SECTION_B,
        vec![source("https://ex.com/alpha-src", 0)],
    )
    .await;
    let (block0, seq0) = blocks_of(&pool, resource).await[0];

    let read = readback::block_read(&pool, owner, resource, BlockId::from(block0))
        .await
        .unwrap();
    let BlockRead::Live {
        block_id,
        seq,
        chunks,
        provenance,
        ..
    } = read
    else {
        panic!("a live block must resolve Live, got {read:?}")
    };
    assert_eq!(
        block_id, block0,
        "the envelope addresses the block asked for"
    );
    assert_eq!(
        seq, seq0,
        "seq is the block's position in the live partition"
    );
    assert!(
        !chunks.is_empty(),
        "a live block carries its chunks' identity"
    );
    let stored_chunks: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_chunks WHERE block_id=$1 AND is_current")
            .bind(block0)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        chunks.len() as i64,
        stored_chunks,
        "the chunk listing is the block's CURRENT chunk rows"
    );
    assert!(
        provenance.iter().any(|r| r.source_kind == "remote"
            && r.source_uri.as_deref() == Some("https://ex.com/alpha-src")),
        "the annotation rides the block's provenance rows, got {provenance:?}"
    );
}

/// CLAUSE: a folded incumbent resolves `folded` — the envelope walks the folded row's own
/// `last_event_id` to the `resource_reblocked` event and renders its disposition map, the
/// dominant created-absorber case — and carries the folded row's own attribution history
/// (history-rides-the-folded-row). Before the disposition map existed this read had no folded
/// face at all; before the history read stopped going through `resource_block_provenance` the
/// envelope's history was empty by construction — the folded row's availability filter
/// excluded exactly the row whose history the envelope carries.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_folded_incumbent_resolves_folded_by_its_reblocked_event_with_a_created_absorber(
    pool: sqlx::PgPool,
) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "br-folded").await;
    let resource = create_two_block(
        &pool,
        owner,
        emitter,
        &home,
        SECTION_A,
        SECTION_B,
        vec![source("https://ex.com/alpha-src", 0)],
    )
    .await;
    let before = blocks_of(&pool, resource).await;
    let folded_a = before[0].0;

    // The absorbed geometry (whole_body_replace.rs:310-313): append a full extra page of new
    // prose to A's section — A's bytes differ so A folds, but the alpha paragraph survives
    // verbatim inside the NEW section, which is the created absorber.
    let filler = "Brand new prose. ".repeat(120);
    let new_body =
        format!("# Alpha\n\nAlpha body paragraph.\n\n{filler}\n## Beta\n\nBeta body paragraph.\n");
    update_body(&pool, emitter, resource, &new_body, vec![]).await;
    let live_now: Vec<Uuid> = blocks_of(&pool, resource)
        .await
        .iter()
        .map(|(id, _)| *id)
        .collect();
    assert!(
        !live_now.contains(&folded_a),
        "fixture: the alpha incumbent folded"
    );

    let read = readback::block_read(&pool, owner, resource, BlockId::from(folded_a))
        .await
        .unwrap();
    let BlockRead::Folded {
        block_id,
        folded_by_event_id,
        disposition,
        attribution_history,
    } = read
    else {
        panic!("a folded incumbent must resolve Folded, got {read:?}")
    };
    assert_eq!(block_id, folded_a);
    assert_eq!(
        event_type_of(&pool, folded_by_event_id).await,
        "resource_reblocked",
        "the envelope's fold pointer walks to the reblocked event that folded the row"
    );
    match disposition {
        BlockFoldDisposition::Located { absorbers, carried } => {
            assert_eq!(
                absorbers.len(),
                1,
                "the alpha chunk multiset lives whole in exactly one section"
            );
            let absorber = absorbers[0].block_id;
            assert!(
                !before.iter().any(|(id, _)| *id == absorber),
                "the named absorber is a CREATED block, not a kept incumbent"
            );
            assert!(
                live_now.contains(&absorber),
                "the named absorber is live now"
            );
            assert!(
                carried.is_empty(),
                "the whole multiset is in the absorber — no partial copies"
            );
        }
        other => panic!("an absorbed fold resolves Located, got {other:?}"),
    }
    // History rides the folded row: the alpha source was asserted on A pre-fold, and the
    // envelope carries that persisted row — the same trail the provenance read shows for the
    // absorbed union on the live absorber, but attributed to the FOLDED address.
    assert_eq!(
        attribution_history.len(),
        1,
        "the folded row's own provenance history rides the envelope"
    );
    assert_eq!(
        attribution_history[0].source_uri.as_deref(),
        Some("https://ex.com/alpha-src"),
        "the history is the alpha source's row, never a reconstruction"
    );
}

/// CLAUSE: the two no-such-readable-block arms. (a) A well-formed address that names no row
/// under its home resource resolves `absent` BY NAME — not an error. (b) A real block whose
/// home resource the caller cannot see refuses with `NotVisible` — the leak-safe deny the
/// surface renders as 404, denying existence rather than admitting the address resolves.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_bogus_address_resolves_absent_and_a_not_visible_home_denies_existence(
    pool: sqlx::PgPool,
) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "br-absent").await;
    let resource =
        create_two_block(&pool, owner, emitter, &home, SECTION_A, SECTION_B, vec![]).await;

    // (a) absent, by name.
    let missing = Uuid::now_v7();
    let read = readback::block_read(&pool, owner, resource, BlockId::from(missing))
        .await
        .unwrap();
    match read {
        BlockRead::Absent { block_id } => assert_eq!(block_id, missing),
        other => panic!("a bogus address must resolve Absent, got {other:?}"),
    }

    // (b) another profile's resource: the address is real, the home is not readable.
    let (other, other_emitter) = insert_actor(&pool, "other").await;
    let other_home = make_home(&pool, other, "br-absent-other").await;
    let other_resource = create_two_block(
        &pool,
        other,
        other_emitter,
        &other_home,
        SECTION_A,
        SECTION_B,
        vec![],
    )
    .await;
    let (other_block, _) = blocks_of(&pool, other_resource).await[0];
    let err = readback::block_read(&pool, owner, other_resource, BlockId::from(other_block))
        .await
        .unwrap_err();
    assert!(
        matches!(err, ReadbackError::NotVisible { .. }),
        "a real block behind an unreadable home must deny with NotVisible, got {err:?}"
    );
}

/// CLAUSE: a fold the ledger does not map resolves to the DEFINED `unrecorded` arm — never a
/// guessed successor. The canonical map-less producer is `charter_set`: it folds the telos's
/// prior blocks with no disposition map, so the envelope states "the ledger does not carry
/// where this content went" rather than approximating an answer.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_map_less_fold_resolves_unrecorded(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter_uuid) = seed_actor(&pool).await;
    let owner = ProfileId::from(owner);
    let emitter = EntityId::from(emitter_uuid);

    // Genesis a cogmap with an EMPTY telos, then deliver the charter twice: the SECOND
    // delivery folds the first delivery's blocks (fold-then-reproject, `_project_charter_set`).
    let mut conn = pool.acquire().await.unwrap();
    let (cogmap, genesis_telos) = fire(
        &mut conn,
        SeedAction::CogmapGenesis {
            name: "br-map-less-cogmap",
            telos_title: "block-read telos",
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

    let blocks = build_charter();
    let mut tx = pool.begin().await.unwrap();
    let telos =
        writes::set_charter_in_tx(&mut tx, cogmap, &blocks, emitter, EventContext::default())
            .await
            .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(
        telos, genesis_telos,
        "fixture: the telos is the genesis telos"
    );
    let first_delivery: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded ORDER BY seq",
    )
    .bind(telos.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        first_delivery.len(),
        3,
        "fixture: the first delivery landed three charter blocks"
    );

    let blocks2 = build_charter();
    let mut tx2 = pool.begin().await.unwrap();
    writes::set_charter_in_tx(&mut tx2, cogmap, &blocks2, emitter, EventContext::default())
        .await
        .unwrap();
    tx2.commit().await.unwrap();

    let read = readback::block_read(&pool, owner, telos, BlockId::from(first_delivery[0]))
        .await
        .unwrap();
    let BlockRead::Folded {
        folded_by_event_id,
        disposition,
        ..
    } = read
    else {
        panic!("a charter-folded block must resolve Folded, got {read:?}")
    };
    assert_eq!(
        event_type_of(&pool, folded_by_event_id).await,
        "charter_set",
        "fixture: the fold walked to the charter_set event, which carries no map"
    );
    assert!(
        matches!(disposition, BlockFoldDisposition::Unrecorded),
        "a charter_set fold carries no disposition map — the defined unrecorded arm, got {disposition:?}"
    );
}
