#![cfg(feature = "artifact-tests")]
//! Binary blobs — the substrate commit path (`migrations/20260901000010_kb_blobs.sql` +
//! `20260901000020_blob_endpoint_reads.sql`).
//!
//! Spec: temper-artifacts/specs/2026-09-01-binary-blobs-design.md (D1-D4, D8-D10; vault copy
//! 01a05d01-648e-74d1-a6b4-345c9bde744b). Read it before changing anything here.
//!
//! What is actually unknown here, and therefore what these pin:
//!
//! 1. **Dedup is get-or-create PER-HOME, not a second row in the same scope** (D2 as amended
//!    2026-09-02): same bytes, same home, twice is one row; the same bytes in another home are
//!    that home's own row with its own identity. Reaching another audience is a relation (D3).
//! 2. **The ledger carries the hash, never the bytes** (D4) — there is not even a sidecar
//!    argument to split; a smuggled payload key is refused outright, and the pathname is the
//!    hash's address (D1), enforced rather than assumed.
//! 3. **The refusal teaches its vocabulary** (D9) — the cap and the allowlist, named from the
//!    values that enforce.
//! 4. **Blob visibility is the blob's own home** (D2) and a blob-related EDGE renders to a reader
//!    of the edge's chain — while **graph walks never materialize a blob as a node** (D3's
//!    deliberate exclusion; the trap named in the handoff).
//! 5. **Replay reproduces the blob projections byte-identically** — there is no sidecar to
//!    re-supply, so this must hold trivially; trivially claims need evidence too.
//!
//! Harness + seeding helpers follow the per-file convention of this suite (duplicated, not
//! shared — see `data_artifacts.rs`'s header).

mod common;

use temper_substrate::blob_store::{blob_pathname, InMemoryBlobStore};
use temper_substrate::events::EventContext;
use temper_substrate::ids::{BlobId, ContextId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, CommitBlobParams};
use uuid::Uuid;

// ── fixtures ──────────────────────────────────────────────────────────────────────────────────

fn sha(bytes: &[u8]) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(bytes))
}

async fn system_actor(pool: &sqlx::PgPool) -> (ProfileId, EntityId) {
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

async fn blob_world(pool: &sqlx::PgPool, slug: &str) -> (ProfileId, EntityId, ContextId) {
    bootseed::seed_system(pool).await.unwrap();
    let (owner, emitter) = system_actor(pool).await;
    let home = ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    );
    (owner, emitter, home)
}

/// The allowlist in force for these tests — a subset of the D9 seeded vocabulary.
const ALLOWLIST: [&str; 3] = ["image/png", "image/svg+xml", "application/pdf"];
const CAP: i64 = 10 * 1024 * 1024;

/// The same vocabulary as `&'static [String]` — the type `CommitBlobParams.allowlist` carries.
fn allowlist() -> &'static [String] {
    static ALLOW: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    ALLOW.get_or_init(|| ALLOWLIST.iter().map(|s| s.to_string()).collect())
}

/// The real commit helper: the caller pre-populates the store (that IS the upload in a fake
/// world), and the hash is computed over the same bytes the pathname names.
fn params(
    home: ContextId,
    owner: ProfileId,
    bytes: &[u8],
    content_type: &str,
    emitter: EntityId,
) -> (CommitBlobParams<'static>, String, String) {
    let hash = sha(bytes);
    let pathname = blob_pathname(&hash);
    let p = CommitBlobParams {
        id: BlobId::from(Uuid::now_v7()),
        home: AnchorRef::context(home),
        owner,
        originator: None,
        content_hash: hash.clone(),
        content_type: content_type.to_string(),
        content_bytes: bytes.len() as i64,
        max_bytes: CAP,
        allowlist: allowlist(),
        emitter,
    };
    (p, hash, pathname)
}

// ── the clauses ───────────────────────────────────────────────────────────────────────────────

/// `upload-record-own-provenance` rests on get-or-create being PER-HOME (D2 as amended
/// 2026-09-02): same bytes, same HOME, twice is ONE row and the second commit returns the SAME
/// id; the same bytes in a different home are that home's own row, never the first home's
/// identity. Reaching another audience is a relation (D3), never a second identity.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn dedup_gets_or_creates_within_a_home(pool: sqlx::PgPool) {
    let (owner, emitter, home) = blob_world(&pool, "dedup").await;
    let bytes = b"\x89PNG-representative-bytes".to_vec();
    let (p1, _hash, pathname) = params(home, owner, &bytes, "image/png", emitter);
    let store = InMemoryBlobStore::default().with_object(pathname.clone());

    let first = writes::commit_blob(&pool, &store, p1).await.unwrap();

    // The SAME home again: get-or-create returns the SAME row id.
    let (p2, _h, _) = params(home, owner, &bytes, "image/png", emitter);
    let second = writes::commit_blob(&pool, &store, p2).await.unwrap();

    assert_eq!(
        first, second,
        "get-or-create within a home returns the EXISTING row id (D2, scoped)"
    );

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_blobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        rows, 1,
        "same home + same bytes is one row — dedup is a constraint, not a habit"
    );

    // The home rides the row now (the homes table is folded into it).
    let (h_table, h_id, h_owner): (String, Uuid, Uuid) =
        sqlx::query_as("SELECT home_table, home_id, owner_profile_id FROM kb_blobs")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(h_table, "kb_contexts");
    assert_eq!(h_id, home.uuid());
    assert_eq!(h_owner, owner.uuid());

    // A DIFFERENT home — even the same principal — holds its own identity: the row-per-home
    // shape, not the old global first-home-stands.
    let second_ctx = ContextId::from(
        common::insert_context(&pool, "kb_profiles", owner.uuid(), "dedup-2", "dedup-2")
            .await
            .unwrap(),
    );
    let (p3, _h2, _) = params(second_ctx, owner, &bytes, "image/png", emitter);
    let third = writes::commit_blob(&pool, &store, p3).await.unwrap();

    assert_ne!(
        first, third,
        "a different home commits its own row — never the first home's identity"
    );
}

/// FAILS IF: a principal committing byte-identical bytes that another principal committed
/// first receives the OTHER principal's row — the pre-amendment D2's dead end (review F6's
/// merged-provenance defect). Under per-home identity B's commit is B's OWN readable row,
/// asserted by B's event, carrying B's identity; A's row is untouched and stays invisible to
/// B. The storage layer still dedups (one pathname), which is D1's job, not the ledger's.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_cross_principal_recommit_is_the_committers_own_row(pool: sqlx::PgPool) {
    let (owner_a, emitter_a, home_a) = blob_world(&pool, "dedup-a").await;
    // B is a REAL second principal — own profile, own emitter, own context. (blob_world's
    // owner is the bootseed's system actor; reusing it for "B" would make B the same
    // principal and prove nothing about cross-principal visibility.)
    let owner_b = ProfileId::from(common::insert_profile(&pool, "dedup-b").await);
    let emitter_b = EntityId::from(
        sqlx::query_scalar::<sqlx::Postgres, Uuid>(
            "INSERT INTO kb_entities (profile_id, name, metadata) \
             VALUES ($1, 'dedup-b@web', '{}'::jsonb) RETURNING id",
        )
        .bind(owner_b.uuid())
        .fetch_one(&pool)
        .await
        .unwrap(),
    );
    let home_b = ContextId::from(
        common::insert_context(&pool, "kb_profiles", owner_b.uuid(), "dedup-b", "dedup-b")
            .await
            .unwrap(),
    );
    let bytes = b"cross-principal-bytes".to_vec();

    let (pa, _hash, pathname) = params(home_a, owner_a, &bytes, "image/png", emitter_a);
    let store = InMemoryBlobStore::default().with_object(pathname.clone());
    let a_id = writes::commit_blob(&pool, &store, pa).await.unwrap();

    let (pb, _h, _p) = params(home_b, owner_b, &bytes, "image/png", emitter_b);
    let b_id = writes::commit_blob(&pool, &store, pb).await.unwrap();

    assert_ne!(
        a_id, b_id,
        "B's commit returns B's own identity, never A's row id"
    );

    // B reads their own row whole; A's row does not exist for B (the SAME 404-shape None).
    let b_row = temper_substrate::readback::blob_by_id(&pool, owner_b, b_id)
        .await
        .unwrap()
        .expect("B reads their own committed row");
    assert_eq!(b_row.content_hash, sha(&bytes));
    assert!(
        temper_substrate::readback::blob_by_id(&pool, owner_b, a_id)
            .await
            .unwrap()
            .is_none(),
        "A's row stays invisible to B — a probe over ids learns nothing"
    );

    // A's row is untouched: A's home, A's owner, asserted by A's event.
    let (a_owner, a_home): (Uuid, Uuid) =
        sqlx::query_as("SELECT owner_profile_id, home_id FROM kb_blobs WHERE id = $1")
            .bind(a_id.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(a_owner, owner_a.uuid(), "A's row keeps A's provenance");
    assert_eq!(a_home, home_a.uuid());

    // B's row asserts B's provenance: the event that created it is B's commit.
    let (b_asserted_owner,): (Uuid,) =
        sqlx::query_as("SELECT owner_profile_id FROM kb_blobs WHERE id = $1")
            .bind(b_id.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(b_asserted_owner, owner_b.uuid());
}

/// `ledger-carries-hash-not-bytes` (D4) and D1's enforced addressing: there is no bytes argument
/// at all, a smuggled payload key is refused by name, and a pathname that is not the hash's
/// address is refused by name.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_ledger_carries_the_hash_never_the_bytes(pool: sqlx::PgPool) {
    let (owner, emitter, home) = blob_world(&pool, "hash-not-bytes").await;
    let hash = sha(b"payload-smuggling-attempt");
    let base = serde_json::json!({
        "blob_id": Uuid::now_v7(),
        "home": {"table": "kb_contexts", "id": home.uuid()},
        "owner_profile_id": owner.uuid(),
        "content_hash": hash,
        "blob_pathname": blob_pathname(&hash),
        "content_type": "image/png",
        "content_bytes": 42i64,
    });

    for smuggled in ["bytes", "__bytes", "content", "__content"] {
        let mut p = base.clone();
        p.as_object_mut()
            .unwrap()
            .insert(smuggled.into(), "pretend-bytes".into());
        let err = sqlx::query("SELECT blob_commit($1, $2, $3, $4)")
            .bind(p)
            .bind(emitter.uuid())
            .bind(CAP)
            .bind(&ALLOWLIST[..])
            .execute(&pool)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("never the bytes"),
            "smuggled key {smuggled} must be refused by name, got: {err}"
        );
    }

    // D1: the pathname IS the hash's address — anything else is refused, naming both shapes.
    let mut p = base.clone();
    p.as_object_mut()
        .unwrap()
        .insert("blob_pathname".into(), "uploads/photo.png".into());
    let err = sqlx::query("SELECT blob_commit($1, $2, $3, $4)")
        .bind(p)
        .bind(emitter.uuid())
        .bind(CAP)
        .bind(&ALLOWLIST[..])
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("content-addressed"),
        "a non-addressed pathname must be refused as such, got: {err}"
    );

    // A clean commit: the STORED event payload is the proof the refusal guards — no bytes key
    // survives into kb_events, and the row's pathname matches its hash.
    let store = InMemoryBlobStore::default().with_object(blob_pathname(&sha(b"clean")));
    let (p, hash, pathname) = params(home, owner, b"clean", "image/png", emitter);
    let id = writes::commit_blob(&pool, &store, p).await.unwrap();
    let (row_path, row_hash): (String, String) =
        sqlx::query_as("SELECT blob_pathname, content_hash FROM kb_blobs WHERE id=$1")
            .bind(id.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        row_path, pathname,
        "the row's pathname is the hash's address"
    );
    assert_eq!(row_hash, hash);
    let payload: serde_json::Value =
        sqlx::query_scalar("SELECT e.payload FROM kb_events e JOIN kb_event_types t ON t.id=e.event_type_id WHERE t.name='blob_committed'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(payload.get("bytes").is_none() && payload.get("content").is_none());
}

/// N4 (2026-09-03 review): the projector's owner/originator mapping follows the
/// ResourceCreated precedent — owner ← the payload's owner, originator ←
/// COALESCE(originator, owner). FAILS IF: the columns come back swapped (owner ← COALESCE,
/// originator ← owner) — the shape was inert while every commit passed `originator: None`
/// (both mappings degenerate to owner=caller), but it would mint swapped provenance the
/// moment on-behalf-of is threaded, and the erasure joins key these columns.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_projector_maps_owner_and_originator_like_resource_created(pool: sqlx::PgPool) {
    let (owner, emitter, home) = blob_world(&pool, "n4-provenance").await;
    let originator = ProfileId::from(common::insert_profile(&pool, "n4-origin").await);

    let bytes = b"provenance-bytes".to_vec();
    let (mut p, _hash, pathname) = params(home, owner, &bytes, "image/png", emitter);
    p.originator = Some(originator);
    let store = InMemoryBlobStore::default().with_object(pathname);
    writes::commit_blob(&pool, &store, p).await.unwrap();

    let (row_owner, row_originator): (Uuid, Uuid) =
        sqlx::query_as("SELECT owner_profile_id, originator_profile_id FROM kb_blobs")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        row_owner,
        owner.uuid(),
        "owner is the payload's owner — never the originator"
    );
    assert_eq!(
        row_originator,
        originator.uuid(),
        "originator is the payload's originator when present; COALESCE falls to the owner \
         only when the payload carries none"
    );
}

/// N6 (2026-09-03 review): the wrapper's home-vocabulary arm is LIVE — a present-but-wrong
/// home table (identity-as-input: any event writer can send one) refuses in the wrapper's
/// own voice. FAILS IF: the RAISE arm is vacuous again (`IS NOT DISTINCT FROM` on both
/// sides of the pair) and the bad home reaches the projector's DDL CHECK — a scrubbed 5xx
/// on the service faces instead of the `blob_commit:` vocabulary refusal.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_wrong_vocabulary_home_refuses_in_the_wrappers_own_voice(pool: sqlx::PgPool) {
    let (owner, emitter, home) = blob_world(&pool, "n6-vocabulary").await;
    let hash = sha(b"wrong-home-vocabulary");
    let payload = serde_json::json!({
        "blob_id": Uuid::now_v7(),
        // Present, non-null, and simply the WRONG kind of anchor.
        "home": {"table": "kb_resources", "id": home.uuid()},
        "owner_profile_id": owner.uuid(),
        "content_hash": hash,
        "blob_pathname": blob_pathname(&hash),
        "content_type": "image/png",
        "content_bytes": 42i64,
    });

    let err = sqlx::query("SELECT blob_commit($1, $2, $3, $4)")
        .bind(payload)
        .bind(emitter.uuid())
        .bind(CAP)
        .bind(&ALLOWLIST[..])
        .execute(&pool)
        .await
        .expect_err("a wrong-vocabulary home must be refused");
    assert!(
        err.to_string().contains("a blob needs a home"),
        "the refusal speaks the wrapper's vocabulary, not a constraint failure: {err}"
    );
}

/// `refusal-names-its-vocabulary` (D9): the cap refusal names the cap, the allowlist refusal
/// lists the allowlist.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_refusal_teaches_its_vocabulary(pool: sqlx::PgPool) {
    let (owner, emitter, home) = blob_world(&pool, "refusals").await;

    // Over the cap: the error carries the cap value that was in force. The provider gate must
    // PASS here (object registered) — the refusal under test is the SQL cap, not D4's gate.
    let bytes = vec![0u8; 64];
    let hash = sha(&bytes);
    let store = InMemoryBlobStore::default().with_object(blob_pathname(&hash));
    let over: CommitBlobParams = CommitBlobParams {
        id: BlobId::from(Uuid::now_v7()),
        home: AnchorRef::context(home),
        owner,
        originator: None,
        content_hash: hash,
        content_type: "image/png".into(),
        content_bytes: CAP + 1, // declared size breaches the cap; bytes are irrelevant to SQL
        max_bytes: CAP,
        allowlist: allowlist(),
        emitter,
    };
    let err = writes::commit_blob(&pool, &store, over).await.unwrap_err();
    let msg = format!("{err:#}");
    assert!(msg.contains("cap"), "cap refusal must name the cap: {msg}");
    assert!(
        msg.contains(&CAP.to_string()),
        "cap refusal must name the value in force: {msg}"
    );

    // Off the allowlist: the error enumerates what IS admitted.
    let bytes = b"svg-claiming-to-be-something-else";
    let hash = sha(bytes);
    let pathname = blob_pathname(&hash);
    let store = InMemoryBlobStore::default().with_object(pathname);
    let off: CommitBlobParams = CommitBlobParams {
        id: BlobId::from(Uuid::now_v7()),
        home: AnchorRef::context(home),
        owner,
        originator: None,
        content_hash: hash,
        content_type: "application/x-msdownload".into(),
        content_bytes: bytes.len() as i64,
        max_bytes: CAP,
        allowlist: allowlist(),
        emitter,
    };
    let err = writes::commit_blob(&pool, &store, off).await.unwrap_err();
    let msg = format!("{err:#}");
    assert!(
        msg.contains("allowlist") && msg.contains("image/png") && msg.contains("application/pdf"),
        "allowlist refusal must enumerate the vocabulary in force: {msg}"
    );
}

/// D4's gate is Rust-side and REAL: no provider object at the content-addressed pathname, no
/// event — the ledger verifies presence, it does not take it on faith.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_commit_without_provider_bytes_is_refused(pool: sqlx::PgPool) {
    let (owner, emitter, home) = blob_world(&pool, "provider-gate").await;
    let (p, _hash, _path) = params(home, owner, b"never-uploaded", "image/png", emitter);
    let err = writes::commit_blob(&pool, &InMemoryBlobStore::default(), p)
        .await
        .unwrap_err();
    assert!(
        format!("{err:#}").contains("no object at"),
        "the provider gate must name the missing pathname: {err}"
    );
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_blobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "a refused commit leaves no row");
}

/// Assert an edge whose TARGET is a blob, homed in the given context — driven through the
/// real Rust write path (`writes::assert_anchored_edge_with`, whose SeedAction endpoints
/// widened to polymorphic `AnchorRef`s with the S4 relate surface; the SQL wrapper and the
/// projector were polymorphic from the start).
async fn assert_edge_to_blob(
    pool: &sqlx::PgPool,
    emitter: EntityId,
    src: ResourceId,
    blob: BlobId,
    home: ContextId,
) -> Uuid {
    let edge = writes::assert_anchored_edge_with(
        pool,
        writes::AssertAnchoredEdgeParams {
            source: AnchorRef::resource(src),
            target: AnchorRef::blob(blob),
            kind: temper_substrate::affinity::EdgeKind::Express,
            polarity: temper_substrate::payloads::EdgePolarity::Forward,
            label: Some("evidence_for"),
            weight: 1.0,
            home: temper_substrate::events::EdgeHome::Context(home),
            emitter,
        },
        EventContext::default(),
    )
    .await
    .unwrap();
    edge.uuid()
}

/// Grant `profile` READ on `context` — the access_grants_context_edge pattern: one grant row
/// wires context-read AND homed-resource read (20260630000002 + 20260701000004), which is what
/// `edges_visible_to`'s home leg and resource-endpoint leg both consult.
async fn grant_context_read(pool: &sqlx::PgPool, context: Uuid, profile: Uuid, granter: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
         (subject_table, subject_id, principal_table, principal_id, can_read, granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, $3)",
    )
    .bind(context)
    .bind(profile)
    .bind(granter)
    .execute(pool)
    .await
    .unwrap();
}

/// `blob-visibility-self-contained` (D2) + D3's read face: a blob endpoint is readable iff the
/// blob's OWN home is readable — never widened by the edge, never narrowed by it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn edge_visibility_asks_the_blobs_home(pool: sqlx::PgPool) {
    let (owner, emitter, home) = blob_world(&pool, "vis-self-contained").await;
    grant_context_read(&pool, home.uuid(), owner.uuid(), owner.uuid()).await;
    let resource = temper_substrate_test_resource(&pool, owner, emitter, home).await;

    let bytes = b"evidence.png".to_vec();
    let (p, _hash, pathname) = params(home, owner, &bytes, "image/png", emitter);
    let store = InMemoryBlobStore::default().with_object(pathname);
    let blob = writes::commit_blob(&pool, &store, p).await.unwrap();
    let edge = assert_edge_to_blob(&pool, emitter, resource, blob, home).await;

    // A reader of the home sees the endpoint and the edge.
    let readable: bool =
        sqlx::query_scalar("SELECT endpoint_readable_by_profile($1, 'kb_blobs', $2)")
            .bind(owner.uuid())
            .bind(blob.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(readable, "the home's reader can read the blob endpoint");
    let visible: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM edges_visible_to($1) WHERE edge_id=$2)")
            .bind(owner.uuid())
            .bind(edge)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(visible, "the home's reader sees the blob-related edge");

    // A principal with NO reach into the home reads neither — the edge's home would not have
    // shown them the edge anyway, but the ENDPOINT gate is the blob's own (D2), asserted here
    // directly so it cannot silently ride the edge gate.
    let outsider = ProfileId::from(common::insert_profile(&pool, "outsider").await);
    let outsider_readable: bool =
        sqlx::query_scalar("SELECT endpoint_readable_by_profile($1, 'kb_blobs', $2)")
            .bind(outsider.uuid())
            .bind(blob.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        !outsider_readable,
        "a non-reader of the home cannot read the blob endpoint"
    );
    let outsider_edge: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM edges_visible_to($1) WHERE edge_id=$2)")
            .bind(outsider.uuid())
            .bind(edge)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        !outsider_edge,
        "a non-reader of the home sees no blob-related edge"
    );
}

/// Seed one resource via the write path (the per-file helper shape data_artifacts.rs uses).
async fn temper_substrate_test_resource(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: ContextId,
) -> ResourceId {
    writes::create_resource_with(
        pool,
        writes::CreateParams {
            idempotency_key: None,
            title: "blob witness resource",
            origin_uri: "blob-witness",
            body: "seed body",
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .unwrap()
}

/// D3's deliberate exclusion — the TRAP named in the handoff: the CHECK admits blob endpoints,
/// and the walk surface must NOT inherit them by accident. The walk's node universe is the
/// resource visible-set (its caller passes `resources_visible_to`'s ids), so the witness asks
/// the structural question twice: the blob is not IN the visible set, and a follow-from seeded
/// at the related resource never materializes the blob as a node.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn graph_walks_never_materialize_a_blob_node(pool: sqlx::PgPool) {
    let (owner, emitter, home) = blob_world(&pool, "walk-exclusion").await;
    grant_context_read(&pool, home.uuid(), owner.uuid(), owner.uuid()).await;
    let resource = temper_substrate_test_resource(&pool, owner, emitter, home).await;

    let bytes = b"walk-exclusion.png".to_vec();
    let (p, _hash, pathname) = params(home, owner, &bytes, "image/png", emitter);
    let store = InMemoryBlobStore::default().with_object(pathname);
    let blob = writes::commit_blob(&pool, &store, p).await.unwrap();
    assert_edge_to_blob(&pool, emitter, resource, blob, home).await;

    // POSITIVE CONTROL: a resource→resource edge under identical conditions. The walk's `adj`
    // stage requires BOTH endpoints in the admitted (resource-visible) set, so the blob edge is
    // dropped there — an empty walk cannot witness exclusion. The resource edge proves the walk
    // RUNS and returns resource endpoints; the assertion under test is that the blob endpoint
    // never rides along. (The peer is created BEFORE `visible` is captured: `admitted` is the
    // caller-passed array, not a live re-query.)
    let peer = temper_substrate_test_resource(&pool, owner, emitter, home).await;
    writes::assert_relationship(
        &pool,
        writes::AssertParams {
            src: resource,
            tgt: peer,
            kind: temper_substrate::affinity::EdgeKind::Express,
            polarity: temper_substrate::payloads::EdgePolarity::Forward,
            label: Some("related_to"),
            weight: 1.0,
            home,
            emitter,
        },
    )
    .await
    .unwrap();

    let visible: Vec<Uuid> = sqlx::query_scalar("SELECT resource_id FROM resources_visible_to($1)")
        .bind(owner.uuid())
        .fetch_all(&pool)
        .await
        .unwrap();
    assert!(
        !visible.contains(&blob.uuid()),
        "the blob must never join the resource visible-set — that IS the walk's node universe"
    );

    let walked: Vec<Uuid> = sqlx::query_scalar(
        "SELECT resource_id FROM __temper_ungated_follow_from($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(&visible)
    .bind([resource.uuid()])
    .bind(3i32)
    .bind(0.8f64)
    .bind::<Option<Vec<String>>>(None)
    .bind::<Option<Vec<String>>>(None)
    .bind::<Option<Vec<Uuid>>>(None)
    .bind(50i32)
    .bind::<Option<serde_json::Value>>(None)
    .bind::<Option<i32>>(None)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        walked.contains(&peer.uuid()),
        "the positive control failed: the walk did not traverse the resource→resource edge, so \
         the blob assertion below would be vacuous"
    );
    assert!(
        !walked.contains(&blob.uuid()),
        "follow-from over a resource→blob edge must not return the blob as a node (D3 exclusion)"
    );
}

/// Replay re-runs ONLY the projector halves, and a blob commit has NO sidecar to re-supply — the
/// rows must still come back byte-identical (proof obligation 2, payload spec §7, blobs join it).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_reproduces_blob_projections(pool: sqlx::PgPool) {
    use temper_substrate::replay;

    common::reset_schema(&pool).await;
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter, home) = blob_world(&pool, "blob-replay").await;

    let bytes = b"replay-proven-bytes".to_vec();
    let (p1, _h, path) = params(home, owner, &bytes, "image/png", emitter);
    let store = InMemoryBlobStore::default().with_object(path);
    writes::commit_blob(&pool, &store, p1).await.unwrap();
    // And a dedup hit, so replay re-derives the get-or-create too.
    let (p2, _h, _path2) = params(home, owner, &bytes, "image/png", emitter);
    writes::commit_blob(&pool, &store, p2).await.unwrap();

    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();

    common::reset_schema(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();

    let after = replay::dump_projections(&pool).await.unwrap();
    // Every projection table must agree — kb_blobs/kb_blob_homes in FULL (nothing to mask:
    // identity-as-input), and no sibling desynced by the blob commits.
    // PROJECTION_DUMPS is one constant list, so the zip pairs identical tables by construction;
    // the assertion is over the VALUES — replay must reproduce every row set exactly.
    for ((table_a, a), (_table_b, b)) in before.iter().zip(after.iter()) {
        assert_eq!(a, b, "projection table {table_a} diverged under replay");
    }
}

// ── S3: staged uploads — the pre-ledger transport half (D7) ──────────────────────────────────
// The begin/append/finalize precedent's row mechanics, owned by `temper_substrate::uploads`.
// What is unknown here, and therefore what these pin:
//
// 1. **Append is idempotent, and an occupied seq is NEVER superseded** — the assembled whole
//    must stay unambiguous; a differing segment at an occupied seq is a conflict, not a revision.
// 2. **A staged session is owner-private** — absent and not-yours are the same `None`, the
//    one-face posture; owner-equality is the ONLY gate (never `blob_readable_by_profile` —
//    a staged session is not a blob, it has no hash yet).
// 3. **Staging rides NO events** — the strongest form of the pre-ledger claim: a full
//    stage cycle moves the event ledger by zero rows.
// 4. **The staging pair is outside replay's diff set** — pinned structurally against
//    `dump_projections`' real table list, not by reading a constant.

use temper_substrate::uploads::{self, AppendOutcome};

async fn staged_session(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    home: ContextId,
    content_type: &str,
) -> Uuid {
    uploads::create_session(pool, owner, &AnchorRef::context(home), content_type)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn staging_appends_are_idempotent_and_occupied_seqs_never_supersede(pool: sqlx::PgPool) {
    let (owner, _emitter, home) = blob_world(&pool, "staged-append").await;
    let id = staged_session(&pool, owner, home, "image/png").await;
    let a: &[u8] = b"AAAA-segment";
    let b: &[u8] = b"BBBB-segment";

    assert_eq!(
        uploads::append_segment(&pool, owner, id, 0, a, &sha(a), i64::MAX)
            .await
            .unwrap(),
        Some(AppendOutcome::Landed),
        "a fresh seq lands"
    );
    assert_eq!(
        uploads::append_segment(&pool, owner, id, 0, a, &sha(a), i64::MAX)
            .await
            .unwrap(),
        Some(AppendOutcome::AlreadyLanded {
            segment_hash: sha(a)
        }),
        "the SAME segment re-sent is the idempotent no-op"
    );
    assert_eq!(
        uploads::append_segment(&pool, owner, id, 0, b, &sha(b), i64::MAX)
            .await
            .unwrap(),
        Some(AppendOutcome::Conflict {
            existing_hash: sha(a)
        }),
        "a DIFFERENT segment at an occupied seq is a conflict — never a supersede"
    );
    assert_eq!(
        uploads::append_segment(&pool, owner, id, 1, b, &sha(b), i64::MAX)
            .await
            .unwrap(),
        Some(AppendOutcome::Landed)
    );

    let landed = uploads::landed_segments(&pool, owner, id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        landed.len(),
        2,
        "the conflict and the no-op changed nothing"
    );
    assert_eq!(landed[0].seq, 0);
    assert_eq!(landed[1].seq, 1, "seq order is the assembly order");
    assert_eq!(
        uploads::assemble_body(&pool, id).await.unwrap(),
        [a, b].concat(),
        "assembly is the seq-ordered concatenation"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_staged_session_is_owner_private(pool: sqlx::PgPool) {
    let (owner, _emitter, home) = blob_world(&pool, "staged-private").await;
    let id = staged_session(&pool, owner, home, "image/png").await;
    let outsider = ProfileId::from(common::insert_profile(&pool, "staging-outsider").await);

    assert!(
        uploads::load_session(&pool, outsider, id)
            .await
            .unwrap()
            .is_none(),
        "another profile's session does not exist for them"
    );
    assert!(
        uploads::landed_segments(&pool, outsider, id)
            .await
            .unwrap()
            .is_none(),
        "another profile's landed set does not exist for them"
    );
    assert_eq!(
        uploads::append_segment(&pool, outsider, id, 0, b"x", &sha(b"x"), i64::MAX)
            .await
            .unwrap(),
        None,
        "another profile cannot append"
    );
    assert!(
        uploads::load_session(&pool, owner, Uuid::now_v7())
            .await
            .unwrap()
            .is_none(),
        "an unknown id renders the same None — absent == not-yours, one face"
    );
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn staging_rides_no_events_and_dies_on_delete(pool: sqlx::PgPool) {
    let (owner, _emitter, home) = blob_world(&pool, "staged-no-ledger").await;
    let id = staged_session(&pool, owner, home, "image/png").await;

    let events_before: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    uploads::append_segment(&pool, owner, id, 0, b"zero", &sha(b"zero"), i64::MAX)
        .await
        .unwrap();
    uploads::append_segment(&pool, owner, id, 1, b"one", &sha(b"one"), i64::MAX)
        .await
        .unwrap();
    let events_after: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        events_before, events_after,
        "a full stage cycle moves the ledger by zero rows — the pre-ledger contract, witnessed"
    );

    uploads::delete_session(&pool, id).await.unwrap();
    let uploads_left: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_blob_uploads")
        .fetch_one(&pool)
        .await
        .unwrap();
    let segments_left: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_blob_upload_segments")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(uploads_left, 0, "delete removes the session");
    assert_eq!(segments_left, 0, "the segments row cascades");
}

/// FAILS IF: the staging ceiling is a read-then-insert check (the review's F4 TOCTOU) —
/// N concurrent appends from the ONE owner each read the same staged total and ALL land,
/// staging an over-cap whole that finalize then assembles in RAM and puts to the
/// provider. The ceiling is decided inside the append's transaction under the session
/// row's lock, so eight concurrent 512-byte appends against a 1024-byte ceiling land
/// exactly two and refuse the rest — the staged total can never exceed the ceiling.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn concurrent_appends_cannot_stage_past_the_ceiling(pool: sqlx::PgPool) {
    let (owner, _emitter, home) = blob_world(&pool, "staged-ceiling-race").await;
    let id = staged_session(&pool, owner, home, "image/png").await;

    let ceiling: i64 = 1024;
    let segment: Vec<u8> = vec![9u8; 512];
    let hash = sha(&segment);

    let futures =
        (0..8).map(|seq| uploads::append_segment(&pool, owner, id, seq, &segment, &hash, ceiling));
    let outcomes = futures_util::future::join_all(futures).await;

    let mut landed = 0;
    let mut refused = 0;
    for outcome in outcomes {
        match outcome.unwrap() {
            Some(AppendOutcome::Landed) => landed += 1,
            Some(AppendOutcome::OverCeiling { staged, ceiling }) => {
                refused += 1;
                assert!(
                    staged > ceiling,
                    "a refused append's would-be total is over the ceiling: {staged} vs {ceiling}"
                );
            }
            other => panic!("concurrent distinct seqs: no conflict/no-op expected, got {other:?}"),
        }
    }
    assert_eq!(landed, 2, "exactly two 512-byte segments fit under 1024");
    assert_eq!(refused, 6, "the rest are refused at the ceiling");

    let staged_set = uploads::landed_segments(&pool, owner, id)
        .await
        .unwrap()
        .unwrap();
    let staged_total: i64 = staged_set.iter().map(|s| s.segment_bytes).sum();
    assert!(
        staged_total <= ceiling,
        "the staged byte total can never exceed the staging ceiling: {staged_total} vs {ceiling}"
    );
    assert_eq!(staged_total, 1024, "the ceiling is used, not merely obeyed");
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_staging_pair_is_outside_replays_diff_set(pool: sqlx::PgPool) {
    let dumps = temper_substrate::replay::dump_projections(&pool)
        .await
        .unwrap();
    assert!(
        !dumps
            .iter()
            .any(|(table, _)| table.contains("kb_blob_uploads")
                || table.contains("kb_blob_upload_segments")),
        "the staging pair must not join replay's diff set — its exclusion is the contract, \
         pinned against the real table list: {:?}",
        dumps.iter().map(|(t, _)| t).collect::<Vec<_>>()
    );
}

// ── S4: the anchored (blob-endpoint) edge write — the relate surface's substrate half ────────
// What is unknown here, and therefore what this pins:
//
// 1. **The widened SeedAction really projects a blob endpoint** — the payload's
//    source/target tables ARE what `_project_relationship_asserted` writes; no SQL changed,
//    and the row proves it (source_table = 'kb_blobs').
// 2. **Idempotent re-assert is the active-edge invariant** — same (source, target, kind,
//    label, home) returns the SAME edge id with the weight updated (`one-blob-many-relations`:
//    a re-assert neither duplicates nor removes any other relation).
// 3. **Replay reproduces blob-endpoint edges** — the projection is payload-driven, so replay
//    must rebuild the kb_blobs-endpoint row exactly; a fire-path widening that broke replay
//    would be a ledger divergence, the worst kind.

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_anchored_edge_write_carries_blob_endpoints_and_replays(pool: sqlx::PgPool) {
    use temper_substrate::replay;

    let (owner, emitter, home) = blob_world(&pool, "anchored-blob-edge").await;
    grant_context_read(&pool, home.uuid(), owner.uuid(), owner.uuid()).await;
    let resource = temper_substrate_test_resource(&pool, owner, emitter, home).await;
    let bytes = b"relation-provenance.png".to_vec();
    let (p, _hash, pathname) = params(home, owner, &bytes, "image/png", emitter);
    let store = InMemoryBlobStore::default().with_object(pathname);
    let blob = writes::commit_blob(&pool, &store, p).await.unwrap();

    // Fire blob → resource through the real write path (the relate surface's shape,
    // blob-as-source: the figure points at what it figures).
    let mk = || writes::AssertAnchoredEdgeParams {
        source: AnchorRef::blob(blob),
        target: AnchorRef::resource(resource),
        kind: temper_substrate::affinity::EdgeKind::Express,
        polarity: temper_substrate::payloads::EdgePolarity::Forward,
        label: Some("figure_of"),
        weight: 1.0,
        home: temper_substrate::events::EdgeHome::Context(home),
        emitter,
    };
    let edge = writes::assert_anchored_edge(&pool, mk()).await.unwrap();

    let (src_table, tgt_table, tgt_id): (String, String, Uuid) =
        sqlx::query_as("SELECT source_table, target_table, target_id FROM kb_edges WHERE id = $1")
            .bind(edge.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        src_table, "kb_blobs",
        "the payload's table IS the endpoint table"
    );
    assert_eq!(tgt_table, "kb_resources");
    assert_eq!(tgt_id, resource.uuid());

    // Idempotent re-assert: same identity, new weight → the SAME edge, the weight updated.
    let again = writes::assert_anchored_edge(
        &pool,
        writes::AssertAnchoredEdgeParams {
            weight: 0.5,
            ..mk()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        again.uuid(),
        edge.uuid(),
        "re-assert upserts, never duplicates"
    );
    let weight: f64 = sqlx::query_scalar("SELECT weight FROM kb_edges WHERE id = $1")
        .bind(edge.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(weight, 0.5, "the re-assert's weight is the edge's weight");

    // The other relation is untouched by all of the above: a second blob-related edge to a
    // different peer survives the re-assert of the first.
    let resource2 = temper_substrate_test_resource(&pool, owner, emitter, home).await;
    let _other = writes::assert_anchored_edge(
        &pool,
        writes::AssertAnchoredEdgeParams {
            target: AnchorRef::resource(resource2),
            ..mk()
        },
    )
    .await
    .unwrap();
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_edges \
         WHERE source_table = 'kb_blobs' AND source_id = $1 AND NOT is_folded",
    )
    .bind(blob.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(live, 2, "two peers, two live relations");

    // Replay: snapshot, reset, replay — every projection table must come back identical,
    // blob-endpoint edges included (they are ordinary payload-driven projections).
    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();
    let after = replay::dump_projections(&pool).await.unwrap();
    for ((table_a, a), (_table_b, b)) in before.iter().zip(after.iter()) {
        assert_eq!(a, b, "projection table {table_a} diverged under replay");
    }
    let replayed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_edges \
         WHERE source_table = 'kb_blobs' AND source_id = $1 AND NOT is_folded",
    )
    .bind(blob.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(replayed, 2, "both blob-endpoint edges survive replay");
}
