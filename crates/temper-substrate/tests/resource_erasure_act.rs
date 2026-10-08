#![cfg(feature = "test-db")]
//! Build order 2b witnesses (resource erasure spec 2026-09-28, the act's cut-1 forms; task
//! 01a0e9e7-491d-7700-8f58-99d0b068e059). The SQL act (`resource_erasure_execute`, migration
//! 20260929040730) against a world built through the REAL write paths, replay through the same
//! snapshot/reset/replay harness the substrate artifact tests use.
//!
//! Spec witnesses covered (numbers per the spec's list):
//!   * **1 (cut-1 form) + 14** — replay byte-identity with a `resource_erased` in the ledger,
//!     provenance + remote-source tables diffed (D4's build check: both added to
//!     PROJECTION_DUMPS); the record names every unreached ledger path by event (R's
//!     `resource_created` by `title`, `origin_uri` and its sources, each `property_set` by
//!     `property_key` and `value`); every typed payload on the ledger roundtrips, the act's
//!     folds included. The redaction steps a byte-identical replay cannot prove (replay runs the
//!     same body) are asserted directly: embeddings, search vector, audit reasons, the home
//!     context's formation watermark, the husk's sentinels and `erased_at`.
//!   * **2** — custody-never-bytes: a sibling in another home with byte-identical content keeps
//!     it, its embedding and its search vector; `kb_erased_content` gains no row.
//!   * **3** — history reached: three revisions of one block, a block folded by a
//!     `replaces_body` mutate and every superseded chunk end with empty content and NULL
//!     embeddings.
//!   * **6** — the edge folds through `relationship_folded` under the act's correlation id; an
//!     edge another principal authored is folded by an event on that edge's own trail; a fold
//!     uncommitted when the act reaches the edge makes the act wait and raise, never fold twice
//!     (20260930000070).
//!   * **8** — soft delete is not YET erasure (`erased_at IS NULL`, content intact) — and a
//!     tombstone IS erasable: the act completes over one (compliance erasure of a
//!     soft-deleted resource is the flow's main shape).
//!   * **9** — the property surface (Q3): every row R owns, live and folded, ends with an
//!     `erased-key-<n>` key and the `"erased"` value; a set→unset→set key maps to ONE
//!     `erased-key-<n>`; two keys tied on occurred_at number by event id, never key text (D4);
//!     replay reproduces it.
//!   * **11 (SQL half)** — the closed refusals RAISE (already-erased, charter, a NULL request
//!     reference at both the act and the refusal); the two former refusals complete — an
//!     in-flight ingest (its `targets` name the ended ingest) and a tombstone made through the
//!     real delete path; the recorded/typed refusal face is the service's, PR 2's witness.
//!   * **10** — artifacts gone: current, member, pinned and superseded artifacts all end `{}`.
//!   * **12** — the joint-read columns: `header_path` NULL, audit `reason` NULL, artifact content
//!     `{}`::jsonb.
//!   * **D11 scope** — the one redaction body takes a block set: it runs steps (1)–(3) narrowed to
//!     the named blocks and stops before the whole-resource steps, and the two-argument call the
//!     act and the replay arm make still resolves. The scrub's own witnesses are
//!     `block_history_scrub.rs`.
//!   * **20** — no writer lands on the husk (D13): a block mutate or a property set holding its
//!     transaction makes the act wait and is erased; a property set or a block mutate arriving
//!     while the act holds R's row refuses; a citation audit, a finalize, a retype, a reweight
//!     and a verdict upsert after the act refuse; the embed write-back after the act writes
//!     nothing and the drain finds nothing stale on the husk; replay byte-identical after each
//!     race.
//!   * **21** — replay does not depend on intra-transaction order (D14): with the act's
//!     `resource_erased` sorted BEFORE its folds over two live R→T edges that differ only by
//!     label, replay completes byte-identical. The deferral is bounded to the act's own
//!     transaction: a later event reusing the request reference does not pull the body past a
//!     lawful write that followed the act. A lawful write that sorts after the act in walk order projects under
//!     the replay walk's guard bypass instead of aborting it.
//!   * **22** — the remote-source re-point holds (D4): two sources at one seq in one event get
//!     distinct `erased:<block_id>:<n>` sentinels; a pre-minted look-alike sentinel does not stop
//!     the re-point; a block citing its own sentinel literal still erases; a citer holding its
//!     transaction keeps its remote source, and the record says the source was kept as shared;
//!     a citer arriving while the act holds the source waits and cites a fresh row; a shared
//!     source is named in the remainder by id, never by URL.
//!   * **23** — edge-owned content is gone (goal §8): another principal's edges into R, live and
//!     already folded, end folded with their structure intact, label NULL, and every edge-owned
//!     key and value sentineled, numbered per edge; the other principal's resource is untouched.
//!   * **24** — the record attests and never repeats (D8): the ingestion record's `source_uri`,
//!     `done`/`dead` job excerpts and a verdict's `detail` are reached; `targets` names every
//!     reached table and none it did not reach; the payload carries no planted string.
//!   * **25** — the facet regrain cannot re-inflate a husk (goal §8): `_facet_regrain_from_events`
//!     over the erased resource, over an edge into it, or over every owner raises on the write
//!     guard and changes nothing, although the ledger still carries the original facet values.
//!   * **15** — the completion pass (cut 2 PR 3, 20261010100000): a cut-1 husk completes under
//!     one more `resource_erased` carrying only `redacted_fields`; its remote sources are numbered
//!     from the ledger as step (9e) numbered them, a URL listed twice in one event included; the
//!     projection moves only by the body's two stamps; replay is byte-identical; a third run is
//!     refused. Also the shape arm's cut-1 branch, and a JSON null kept on the ledger.
//!
//! The doors (Rust) land in PR 2; this file pins the SQL behavior the doors consume.

mod common;

use sha2::Digest;
use sqlx::PgPool;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::EntityId;
use temper_core::types::property_owner::PropertyOwner;
use temper_substrate::affinity::EdgeKind;
use temper_substrate::blob_store::InMemoryBlobStore;
use temper_substrate::content::IncomingChunk;
use temper_substrate::events::{fire, EdgeHome, EventContext, SeedAction};
use temper_substrate::ids::{
    BlobId, BlockId, ContextId, DataArtifactId, EdgeId, EventId, LensId, ProfileId, ResourceId,
};
use temper_substrate::payloads::EdgePolarity;
use temper_substrate::payloads::{
    self, AgentAuthorship, AnchorRef, ArtifactIntent, ConfidenceBand, Incorporation, KindOwner,
    ProvenanceSource,
};
use temper_substrate::replay;
use temper_substrate::writes::CommitBlobParams;
use temper_substrate::writes::{self, CommitDataArtifactParams, CreateParams, UpdateParams};
use temper_substrate::writes::{AssertParams, CreateMode};
use uuid::Uuid;

/// The leaked prose and the clean replacement, one pair per file. `chunk_hash` is the chunker's
/// own sha256-of-trim (content.rs:21), so hash joins the tests assert on are the REAL ones.
const SECRET: &str = "the plan and SSN 123-45-6789";
const CLEAN: &str = "clean replacement prose";
const URL: &str = "https://leak.example/internal/jane-smith";

fn chunk_hash(prose: &str) -> String {
    format!("{:x}", sha2::Sha256::digest(prose.trim()))
}

fn chunk(prose: &str, header: &str) -> IncomingChunk {
    IncomingChunk {
        chunk_index: 0,
        content_hash: chunk_hash(prose),
        content: prose.to_string(),
        embedding: vec![0.1; 768],
        embedded_with: Some("model-sha-1".to_string()),
        header_path: header.to_string(),
        heading_depth: if header.is_empty() { 0 } else { 1 },
    }
}

async fn system_actor(pool: &PgPool) -> (ProfileId, EntityId) {
    let profile: Uuid = sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle='system'")
        .fetch_one(pool)
        .await
        .unwrap();
    let entity: EntityId =
        sqlx::query_scalar("SELECT id FROM kb_entities WHERE profile_id=$1 AND name='system'")
            .bind(profile)
            .fetch_one(pool)
            .await
            .unwrap();
    (ProfileId::from(profile), entity)
}

async fn make_home(pool: &PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

/// The full leaking shape, built through the REAL write paths.
struct Leak {
    resource: ResourceId,
    /// The sibling twin: byte-identical chunk + revision bytes, another home (Witness 2).
    twin: ResourceId,
    /// The now-superseded chunk still carrying the secret (Witness 3).
    old_chunk: Uuid,
    /// The edge the act folds (Witness 6).
    edge: EdgeId,
    /// The resource's data artifact (Witness 12, artifact half).
    artifact: Uuid,
}

async fn seed_leak(
    pool: &PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: ContextId,
    twin_home: ContextId,
) -> Leak {
    let resource = writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title: "M&A notes (leaked)",
            origin_uri: "test://seed-leak",
            body: SECRET,
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: Some(vec![chunk(SECRET, "merger notes")]),
            sources: vec![Incorporation {
                source: ProvenanceSource::Remote(URL.to_owned()),
                seq: 1,
            }],
        },
        EventContext::default(),
    )
    .await
    .expect("seed resource through the create path");

    let twin = writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title: "harmless twin",
            origin_uri: "test://seed-twin",
            body: SECRET,
            doc_type: "research",
            home: AnchorRef::context(twin_home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: Some(vec![chunk(SECRET, "")]),
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("seed the byte-identical twin");

    // History: revise the body to CLEAN prose — the superseded chunk keeps the secret.
    let leaky_chunk_hash = chunk_hash(SECRET);
    writes::update_resource(
        pool,
        UpdateParams {
            resource,
            body: Some(CLEAN),
            title: None,
            origin_uri: None,
            properties: &[],
            unset_keys: &[],
            chunks: Some(vec![chunk(CLEAN, "clean")]),
            sources: vec![],
            content_block: None,
            rehome_to: None,
            emitter,
        },
    )
    .await
    .expect("revise the body out of the leak");

    let old_chunk: Uuid = sqlx::query_scalar(
        "SELECT c.id FROM kb_chunks c \
           JOIN kb_content_blocks b ON b.id = c.block_id \
          WHERE b.resource_id = $1 AND NOT c.is_current LIMIT 1",
    )
    .bind(resource.uuid())
    .fetch_one(pool)
    .await
    .unwrap();

    // An edge touching the resource (its own trail must show a deliberate end — Witness 6).
    let mut tx = pool.begin().await.unwrap();
    let edge = fire(
        &mut tx,
        SeedAction::RelationshipAssert {
            src: AnchorRef::resource(resource),
            tgt: AnchorRef::resource(twin),
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("jane smith spoke to us"),
            weight: 1.0,
            home: EdgeHome::Context(home),
            emitter,
        },
    )
    .await
    .unwrap()
    .relationship()
    .unwrap();
    tx.commit().await.unwrap();

    // The set→set→set key the act maps to ONE erased-key-<n> (Witness 9).
    writes::set_property(
        pool,
        resource,
        "transient",
        &serde_json::json!("one"),
        emitter,
    )
    .await
    .unwrap();

    // A data artifact on the resource (Witness 12).
    let artifact = writes::commit_data_artifact(
        pool,
        CommitDataArtifactParams {
            resource,
            kind: "notes",
            kind_owner: Some(KindOwner::Profile(owner.uuid())),
            intent: ArtifactIntent::Current,
            precedence: 0.0,
            content: &serde_json::json!({"jane": "was here"}),
            supersedes: &[],
            emitter,
        },
    )
    .await
    .unwrap();

    let _ = leaky_chunk_hash;
    Leak {
        resource,
        twin,
        old_chunk,
        edge,
        artifact: Uuid::from(artifact),
    }
}

/// Re-register `block_provenance_annotated`: migration 20260710000001 inserts it; `reset_schema` truncates it.
async fn register_block_provenance_annotated(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO kb_event_types (name, payload_schema, schema_version, category) \
         VALUES ('block_provenance_annotated', NULL, 1, 'domain') \
         ON CONFLICT (name) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("re-register block_provenance_annotated");
}

/// Re-register `citation_audited`: migration 20260724000110 inserts it; `reset_schema` truncates it.
async fn register_citation_audited(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO kb_event_types (name, payload_schema, schema_version, category) \
         VALUES ('citation_audited', NULL, 1, 'domain') \
         ON CONFLICT (name) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("re-register citation_audited");
}

/// Re-register `resource_finalized`: migration 20260708000012 inserts it; `reset_schema` truncates it.
async fn register_resource_finalized(pool: &PgPool) {
    sqlx::query(
        "INSERT INTO kb_event_types (name, payload_schema, schema_version, category) \
         VALUES ('resource_finalized', NULL, 1, 'domain') \
         ON CONFLICT (name) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("re-register resource_finalized");
}

/// The one act invocation every witness uses — the boot-seeded system actor is the operator
/// (the service gate is PR 2's concern; here SQL executes as the operator), a fresh request
/// reference per act. Returns the `resource_erased` event id.
async fn execute_act(pool: &PgPool, resource: Uuid) -> Uuid {
    execute_act_with_ref(pool, resource, Uuid::now_v7()).await
}

/// [`execute_act`] under a request reference the caller names.
async fn execute_act_with_ref(pool: &PgPool, resource: Uuid, request_ref: Uuid) -> Uuid {
    let (_, operator_entity) = system_actor(pool).await;
    let raw: String =
        sqlx::query_scalar("SELECT (resource_erasure_execute($1,$2,$3,$4)->>'event_id')::text")
            .bind(resource)
            .bind(operator_entity)
            .bind(operator_entity)
            .bind(request_ref)
            .fetch_one(pool)
            .await
            .expect("the act completes");
    Uuid::parse_str(&raw).expect("the event id parses")
}
/// Read the live (non-folded) property rows the resource owns, as (key, value) in key order.
async fn resource_props(pool: &PgPool, resource: ResourceId) -> Vec<(String, serde_json::Value)> {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL search_path TO public")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query_as(
        "SELECT property_key, property_value FROM kb_properties \
          WHERE owner_table='kb_resources' AND owner_id=$1 AND NOT is_folded \
          ORDER BY property_key",
    )
    .bind(resource.uuid())
    .fetch_all(&mut *tx)
    .await
    .unwrap()
}

/// (1 + 14) Replay byte-identity: seed the leak, erase, snapshot, reset, replay — the projection
/// comes back byte-identical INCLUDING the re-pointed provenance rows and the sentinel
/// remote-source rows (the tables D4's build check added). Before replay: the record names each
/// of R's ledger paths (14), the ledger's payloads roundtrip over the act's folds, all four
/// artifact intents are emptied (10), and each redaction step replay cannot prove is asserted
/// directly (steps 3, 4, 6, 7, 9a). A repeat erasure on the replayed
/// namespace REFUSES (ruled 2026-09-29: a recorded refusal, no second event, no-op projection),
/// and replay after it is still identical.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_of_a_resource_erasure_is_byte_identical(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    register_block_provenance_annotated(&pool).await;
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "leak-home").await;
    let twin_home = make_home(&pool, owner, "twin-home").await;
    let leak = seed_leak(&pool, owner, emitter, home, twin_home).await;
    // The twin also cites URL, so URL's remote-source row is shared: it survives the act and the
    // remainder names it (D4, D8).
    writes::annotate_block_sources(
        &pool,
        writes::AnnotateParams {
            resource: leak.twin,
            sources: vec![Incorporation {
                source: ProvenanceSource::Remote(URL.to_owned()),
                seq: 0,
            }],
            content_block: None,
            emitter,
        },
    )
    .await
    .expect("the twin cites URL too");
    let url_id: Uuid = sqlx::query_scalar("SELECT id FROM kb_remote_sources WHERE uri = $1")
        .bind(URL)
        .fetch_one(&pool)
        .await
        .unwrap();

    // Step (6)'s precondition: R's live block cites the twin (resource-kind, the only auditable
    // kind — `citation_audit`), and an auditor's verdict on that citation carries a reason.
    register_citation_audited(&pool).await;
    let r_block: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded \
          ORDER BY seq LIMIT 1",
    )
    .bind(leak.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let r_block = writes::annotate_block_sources(
        &pool,
        writes::AnnotateParams {
            resource: leak.resource,
            sources: vec![Incorporation {
                source: ProvenanceSource::Resource(leak.twin.uuid()),
                seq: 0,
            }],
            content_block: Some(r_block),
            emitter,
        },
    )
    .await
    .expect("R's block cites the twin");
    writes::record_citation_audit(
        &pool,
        writes::CitationAuditParams {
            block: r_block,
            source: ProvenanceSource::Resource(leak.twin.uuid()),
            value: 0.5,
            reason: Some("jane smith confirmed the figures"),
            emitter,
        },
    )
    .await
    .expect("an audit of R's citation, with a reason");

    // Step (7)'s precondition: a materialize over R's home context stamps its formation
    // watermark, through the real `region_materialize` door (a boot-seeded global lens; no
    // region rows are needed for the stamp).
    let lens: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_cogmap_lenses WHERE cogmap_id IS NULL AND name = 'telos-default'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let watermark: Uuid = sqlx::query_scalar("SELECT id FROM kb_events ORDER BY id DESC LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let mut tx = pool.begin().await.unwrap();
    let materialized = fire(
        &mut tx,
        SeedAction::Materialize {
            anchor: HomeAnchor::Context(home),
            lens: LensId::from(lens),
            watermark: EventId::from(watermark),
            membership_fingerprint: "erasure-witness",
            region_ids: &[],
            telos: None,
            emitter,
        },
    )
    .await
    .unwrap()
    .materialize_event()
    .unwrap();
    tx.commit().await.unwrap();
    let home_watermark: Option<Uuid> =
        sqlx::query_scalar("SELECT shape_materialized_event_id FROM kb_contexts WHERE id = $1")
            .bind(home.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        home_watermark,
        Some(materialized.uuid()),
        "the witness needs the home context's watermark stamped before the act"
    );

    // Witness 10's seed: beside seed_leak's `Current` artifact, a `Member` and a `Pinned` one,
    // then a second `Current` commit that supersedes the first — four artifacts, one folded.
    let mut all_artifacts: Vec<Uuid> = vec![leak.artifact];
    for (intent, content) in [
        (
            ArtifactIntent::Member,
            serde_json::json!({"run": "jane smith, member"}),
        ),
        (
            ArtifactIntent::Pinned,
            serde_json::json!({"run": "jane smith, pinned"}),
        ),
    ] {
        let peer = writes::commit_data_artifact(
            &pool,
            CommitDataArtifactParams {
                resource: leak.resource,
                kind: "notes",
                kind_owner: Some(KindOwner::Profile(owner.uuid())),
                intent,
                precedence: 0.0,
                content: &content,
                supersedes: &[],
                emitter,
            },
        )
        .await
        .unwrap();
        all_artifacts.push(Uuid::from(peer));
    }
    let successor = writes::commit_data_artifact(
        &pool,
        CommitDataArtifactParams {
            resource: leak.resource,
            kind: "notes",
            kind_owner: Some(KindOwner::Profile(owner.uuid())),
            intent: ArtifactIntent::Current,
            precedence: 0.0,
            content: &serde_json::json!({"jane": "was here again"}),
            supersedes: &[DataArtifactId::from(leak.artifact)],
            emitter,
        },
    )
    .await
    .unwrap();
    let superseded: bool =
        sqlx::query_scalar("SELECT is_folded FROM kb_data_artifacts WHERE id = $1")
            .bind(leak.artifact)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        superseded,
        "the first Current artifact is superseded before the act"
    );
    all_artifacts.push(Uuid::from(successor));

    // Steps (3) and (4)'s preconditions: R's chunks carry embeddings and R has a non-empty
    // search vector, so the post-act NULLs and '' are the act's.
    let embedded_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks WHERE resource_id = $1 AND embedding IS NOT NULL",
    )
    .bind(leak.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        embedded_before > 0,
        "the witness needs embedded chunks on R"
    );
    let searchable_before: bool = sqlx::query_scalar(
        "SELECT search_vector <> ''::tsvector FROM kb_resource_search_index WHERE resource_id = $1",
    )
    .bind(leak.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        searchable_before,
        "the witness needs a non-empty search vector on R"
    );
    // Witness 4's authorship half: an agent's prose and persona on an edit of R, and another
    // author's value on R's edge.
    writes::set_property_with(
        &pool,
        leak.resource,
        "status",
        &serde_json::json!("under review"),
        emitter,
        EventContext {
            authorship: Some(AgentAuthorship {
                reasoning: Some("jane smith asked for the review".into()),
                confidence: ConfidenceBand::Probable,
                rationale: Some("the merger memo names her".into()),
                persona: Some("jane's assistant".into()),
                model: Some("model-under-test".into()),
            }),
            ..EventContext::default()
        },
    )
    .await
    .unwrap();
    writes::assert_keyed_property_with(
        &pool,
        PropertyOwner::edge(leak.edge),
        "evidence",
        &serde_json::json!("jane's diary, page 4"),
        1.0,
        emitter,
        EventContext::default(),
    )
    .await
    .unwrap();

    // Witness 9's denominator: every property row R owns, live and folded, before the act.
    let family_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties WHERE owner_table = 'kb_resources' AND owner_id = $1",
    )
    .bind(leak.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        family_before >= 2,
        "the witness needs R's property family (doc_type and `transient`); got {family_before}"
    );

    let event_id = execute_act(&pool, leak.resource.uuid()).await;

    // The record names what the act rewrote on the ledger (cut 2, D3): `redacted_fields` lists
    // every free-text path of every trail-scope event that held text, by class (the F3 catalog
    // through the allowlist), and `ledger_remainder` keeps only what the exception cannot reach,
    // the telos copies, of which R (not a goal) has none.
    let (redacted, ledger_remainder, remainder, folded_edges): (
        serde_json::Value,
        serde_json::Value,
        serde_json::Value,
        serde_json::Value,
    ) = sqlx::query_as(
        "SELECT payload->'redacted_fields', payload->'ledger_remainder', payload->'remainder', \
                payload->'folded_edges' \
           FROM kb_events WHERE id = $1",
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        ledger_remainder,
        serde_json::json!([]),
        "a resource that was never a goal leaves nothing on the ledger unreached"
    );
    // Witness 4: R's own trail no longer says what the erased text was, in any payload or any
    // authorship metadata.
    for planted in [
        "M&A notes (leaked)",
        "test://seed-leak",
        URL,
        "\"research\"",
        "jane smith spoke to us",
        "transient",
        "\"notes\"",
        "jane smith confirmed the figures",
        "jane smith asked for the review",
        "the merger memo names her",
        "jane's assistant",
        "model-under-test",
        "under review",
        "jane's diary",
    ] {
        let still: Vec<Uuid> = sqlx::query_scalar(
            "SELECT ev.id FROM _resource_erasure_trail_scope($1) s \
               JOIN kb_events ev ON ev.id = s.event_id \
              WHERE strpos(ev.payload::text || ev.metadata::text, $2) > 0",
        )
        .bind(leak.resource.uuid())
        .bind(planted)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert!(
            still.is_empty(),
            "the trail still carries {planted:?} in {still:?}"
        );
    }
    let paths_named_for = |event: Uuid| -> Vec<String> {
        redacted
            .as_array()
            .expect("redacted_fields is an array")
            .iter()
            .find(|entry| entry["event"] == serde_json::json!(event))
            .unwrap_or_else(|| panic!("redacted_fields names event {event}; got {redacted}"))
            ["paths"]
            .as_array()
            .expect("an entry's paths is an array")
            .iter()
            .map(|p| p.as_str().expect("a path is a string").to_owned())
            .collect()
    };
    let created_event: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'resource_created' AND (e.payload->>'resource_id')::uuid = $1",
    )
    .bind(leak.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let created_paths = paths_named_for(created_event);
    for path in [
        "title",
        "origin_uri",
        "doc_type",
        "blocks[*].incorporated[*].source.value",
    ] {
        assert!(
            created_paths.iter().any(|p| p == path),
            "R's resource_created is named with {path}; got {created_paths:?}"
        );
    }
    // The path F3's correction (2026-10-03) added, an artifact's family; and a citation audit's
    // source is NOT named: citation_audit admits only resource-kind sources, so it is the cited
    // resource's id, which the projector casts to uuid and a sentinel would break.
    let named_by_type = |ty: &'static str| {
        let pool = pool.clone();
        let resource = leak.resource.uuid();
        async move {
            sqlx::query_scalar::<_, Uuid>(
                "SELECT s.event_id FROM _resource_erasure_trail_scope($1) s WHERE s.event_type = $2",
            )
            .bind(resource)
            .bind(ty)
            .fetch_all(&pool)
            .await
            .unwrap()
        }
    };
    for (ty, want, never) in [
        ("data_artifact_committed", &["artifact_kind"][..], &[][..]),
        ("citation_audited", &["reason"][..], &["source.value"][..]),
    ] {
        let events = named_by_type(ty).await;
        assert!(!events.is_empty(), "the witness needs R's {ty} events");
        for event in events {
            let paths = paths_named_for(event);
            for path in want {
                assert!(
                    paths.iter().any(|p| p == path),
                    "{ty} {event} is named with {path}; got {paths:?}"
                );
            }
            for path in never {
                assert!(
                    !paths.iter().any(|p| p == path),
                    "{ty} {event} is never named with {path}; got {paths:?}"
                );
            }
        }
    }
    // Step (9f), Witness 10 amended: every artifact of R, whatever its intent or fold state, has
    // the per-event family sentinel. Replay reproduces it below: its arm runs the same body.
    let families: Vec<(String, Uuid)> = sqlx::query_as(
        "SELECT artifact_kind, asserted_by_event_id FROM kb_data_artifacts WHERE resource_id = $1",
    )
    .bind(leak.resource.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        families.len() >= 4,
        "the witness needs every intent's artifact; got {families:?}"
    );
    for (family, asserted_by) in &families {
        assert_eq!(
            family,
            &format!("erased:{asserted_by}"),
            "an erased resource's artifact keeps no family"
        );
    }
    let property_set_events: Vec<Uuid> = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'property_set' \
            AND e.payload #>> '{owner,table}' = 'kb_resources' \
            AND (e.payload #>> '{owner,id}')::uuid = $1",
    )
    .bind(leak.resource.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        !property_set_events.is_empty(),
        "the witness needs R's property_set events on the ledger"
    );
    for event in property_set_events {
        let paths = paths_named_for(event);
        for path in ["property_key", "value"] {
            assert!(
                paths.iter().any(|p| p == path),
                "property_set {event} is named with {path}; got {paths:?}"
            );
        }
    }

    // The ledger carries the act's folds (`folded_edges` is non-empty: seed_leak's edge), and
    // every typed payload on it — the act's `resource_erased` and `relationship_folded`
    // included — deserializes into its struct.
    assert!(
        !folded_edges
            .as_array()
            .expect("folded_edges is an array")
            .is_empty(),
        "the roundtrip must run over a ledger with folds; got {folded_edges}"
    );
    payloads::verify_ledger_roundtrip(&pool)
        .await
        .expect("every typed payload on an erasure ledger roundtrips");
    // The twin is NOT in any remainder entry naming the resource's own content — the twin is a
    // separate resource whose identical bytes are lawful, not a remainder of this act.
    let remainder_text = remainder.to_string();
    assert!(
        !remainder_text.contains("harmless twin"),
        "the twin's identity never rides the record; got {remainder_text}"
    );
    // The shared remote source is named by its kb_remote_sources id, never by its URL: the
    // record is an admin event outside the trail scope, so nothing could redact a URL in it.
    for fragment in [URL, "leak.example", "jane-smith"] {
        assert!(
            !remainder_text.contains(fragment),
            "the record carries no part of the URL ({fragment}); got {remainder_text}"
        );
    }
    assert!(
        remainder_text.contains(&format!(
            "shared remote source {url_id}; another resource's block still cites it; named, kept"
        )),
        "the remainder names the shared remote source by id; got {remainder_text}"
    );
    let url_survives: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_remote_sources WHERE id = $1")
            .bind(url_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        url_survives, 1,
        "a remote source the twin still cites stays"
    );

    // 2 (custody-never-bytes) BEFORE replay: the twin's content, embedding and search vector
    // are untouched, and kb_erased_content gains no row from this act.
    let twin_prose: Vec<String> = sqlx::query_as::<_, (String,)>(
        "SELECT cc.content FROM kb_chunks c \
           JOIN kb_content_blocks b ON b.id = c.block_id \
           JOIN kb_chunk_content cc ON cc.chunk_id = c.id \
          WHERE b.resource_id = $1 AND cc.content <> ''",
    )
    .bind(leak.twin.uuid())
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .map(|(c,)| c)
    .collect();
    assert!(
        twin_prose.iter().any(|c| c == SECRET),
        "the twin's byte-identical content is untouched; got {twin_prose:?}"
    );
    let erased_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_erased_content")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        erased_rows, 0,
        "NO hash enters kb_erased_content from a resource act"
    );

    // 3 (history reached) + 12: the old chunk, every revision, header_path, artifact content.
    let old_prose: Option<String> = sqlx::query_scalar::<_, String>(
        "SELECT cc.content FROM kb_chunk_content cc WHERE cc.chunk_id = $1",
    )
    .bind(leak.old_chunk)
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(
        old_prose,
        Some("".to_string()),
        "the superseded chunk's prose is emptied"
    );
    let block_bytes: Vec<(Option<String>,)> = sqlx::query_as(
        "SELECT bc.content FROM kb_block_content bc \
           JOIN kb_block_revisions br ON br.id = bc.block_revision_id \
          WHERE br.block_id IN (SELECT id FROM kb_content_blocks WHERE resource_id = $1)",
    )
    .bind(leak.resource.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        block_bytes.iter().all(|(b,)| b == &Some("".to_string())),
        "every revision's verbatim bytes are emptied (live revision included); got {block_bytes:?}"
    );
    let null_headers: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks c \
           JOIN kb_content_blocks b ON b.id = c.block_id \
          WHERE b.resource_id = $1 AND c.header_path IS NOT NULL",
    )
    .bind(leak.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        null_headers, 0,
        "every chunk's header_path is NULL (the joint-read fix)"
    );
    // 10 + 12 (artifact half): current, member, pinned and superseded — all four emptied.
    let artifact_contents: Vec<(Uuid, serde_json::Value)> = sqlx::query_as(
        "SELECT artifact_id, content FROM kb_data_artifact_content WHERE artifact_id = ANY($1)",
    )
    .bind(&all_artifacts)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        artifact_contents.len(),
        4,
        "every artifact keeps its content row; got {artifact_contents:?}"
    );
    assert!(
        artifact_contents
            .iter()
            .all(|(_, c)| c == &serde_json::json!({})),
        "every artifact's content is empty jsonb — every intent, superseded included; got {artifact_contents:?}"
    );

    // Step (3): embeddings and their provenance nulled together on every chunk of R.
    let still_embedded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks WHERE resource_id = $1 \
            AND (embedding IS NOT NULL OR embedded_with IS NOT NULL)",
    )
    .bind(leak.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        still_embedded, 0,
        "every chunk of R has embedding IS NULL AND embedded_with IS NULL"
    );

    // Step (4): the search vector is empty.
    let search_emptied: bool = sqlx::query_scalar(
        "SELECT search_vector = ''::tsvector FROM kb_resource_search_index WHERE resource_id = $1",
    )
    .bind(leak.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(search_emptied, "R's search_vector is ''");

    // Step (6) + 12 (audit half): every audit on R's blocks has its reason nulled.
    let audit_reasons: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT ca.reason FROM kb_citation_audits ca \
           JOIN kb_content_blocks b ON b.id = ca.block_id \
          WHERE b.resource_id = $1",
    )
    .bind(leak.resource.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        !audit_reasons.is_empty() && audit_reasons.iter().all(Option::is_none),
        "the audit on R's block has reason IS NULL; got {audit_reasons:?}"
    );

    // Step (7): the home context's formation watermark is nulled.
    let home_watermark: Option<Uuid> =
        sqlx::query_scalar("SELECT shape_materialized_event_id FROM kb_contexts WHERE id = $1")
            .bind(home.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        home_watermark, None,
        "the home context's shape_materialized_event_id IS NULL after the act"
    );

    // Step (9a): the husk — sentinel title and origin_uri, inactive, erased_at the act's
    // occurred_at.
    let (husk_title, husk_uri, husk_active, erased_at_is_the_acts): (String, String, bool, bool) =
        sqlx::query_as(
            "SELECT r.title, r.origin_uri, r.is_active, \
                    coalesce(r.erased_at = e.occurred_at, false) \
               FROM kb_resources r, kb_events e \
              WHERE r.id = $1 AND e.id = $2",
        )
        .bind(leak.resource.uuid())
        .bind(event_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(husk_title, format!("erased-{}", leak.resource.uuid()));
    assert_eq!(husk_uri, format!("erased:{}", leak.resource.uuid()));
    assert!(!husk_active, "the husk is inactive");
    assert!(
        erased_at_is_the_acts,
        "erased_at equals the resource_erased event's occurred_at"
    );

    // 9 (property surface): EVERY row R owns — the act folds the whole family, so the live
    // surface is empty and the check runs over the folded rows — has an erased-key-<n> key and
    // the sentinel value, and the act neither dropped nor added a row.
    let family: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT property_key, property_value FROM kb_properties \
          WHERE owner_table = 'kb_resources' AND owner_id = $1 ORDER BY property_key",
    )
    .bind(leak.resource.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        family.len() as i64,
        family_before,
        "the act keeps every row of R's property family; got {family:?}"
    );
    assert!(
        family.iter().all(|(k, _)| k.starts_with("erased-key-")),
        "no original metadata key survives, live or folded (Q3); got {family:?}"
    );
    assert!(
        family
            .iter()
            .all(|(_, v)| v == &serde_json::json!("erased")),
        "every value, live or folded, is the sentinel; got {family:?}"
    );

    // The old title is GONE from the ledger-free-text reach this act has: the husk carries the
    // sentinel, and no kb_properties row (live or folded) carries an original key or value.
    let stale_meta: Option<(String,)> = sqlx::query_as::<_, (String,)>(
        "SELECT property_key::text FROM kb_properties \
          WHERE owner_table = 'kb_resources' AND owner_id = $1 \
            AND (property_key = ANY(ARRAY['tags','transient']) \
              OR property_value IN ('\"one\"'::jsonb, '\"jane smith\"'::jsonb)) LIMIT 1",
    )
    .bind(leak.resource.uuid())
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert!(
        stale_meta.is_none(),
        "no original key or value survives in the surface; got {stale_meta:?}"
    );

    // 6 (edges): the edge is folded, label NULL, through a relationship_folded under the act's
    // correlation id; the fold's reason is the fixed literal.
    let (folded, label): (bool, Option<String>) =
        sqlx::query_as("SELECT is_folded, label FROM kb_edges WHERE id = $1")
            .bind(leak.edge.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(folded, "the act folds the edge");
    assert!(label.is_none(), "the edge's label is sentineled (D4)");
    let fold: (Option<String>, Uuid) = sqlx::query_as(
        "SELECT e.payload->>'reason', e.correlation_id \
           FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'relationship_folded' AND (e.payload->>'edge_id')::uuid = $1",
    )
    .bind(leak.edge.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        fold.0.as_deref(),
        Some("resource_erased"),
        "the fold's reason is the fixed literal"
    );
    let erased_corr: Uuid =
        sqlx::query_scalar("SELECT correlation_id FROM kb_events WHERE id = $1")
            .bind(event_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        fold.1, erased_corr,
        "the fold rides the act's correlation id"
    );

    // ── replay #1: snapshot, reset, walk, diff ──
    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();
    let after = replay::dump_projections(&pool).await.unwrap();
    for ((ta, a), (tb, b)) in before.iter().zip(after.iter()) {
        assert_eq!(ta, tb);
        assert_eq!(
            a, b,
            "projection table {ta} diverged under replay of an erasure"
        );
    }

    // A repeat erasure on the replayed namespace: the recorded-refusal posture's SQL half —
    // the act RAISES (the service turns that into a recorded `resource_erasure_refused`,
    // PR 2's witness), the projection does not change, and no second `resource_erased` mints.
    let before2 = replay::dump_projections(&pool).await.unwrap();
    let refused = sqlx::query("SELECT resource_erasure_execute($1,$2,$3,$4)")
        .bind(leak.resource.uuid())
        .bind(owner.uuid())
        .bind(emitter)
        .bind(Uuid::now_v7())
        .execute(&pool)
        .await;
    assert!(
        refused.is_err(),
        "a repeat erasure RAISES at SQL grain (the service records it)"
    );
    let after2 = replay::dump_projections(&pool).await.unwrap();
    for ((ta, a), (_tb, b)) in before2.iter().zip(after2.iter()) {
        assert_eq!(a, b, "a refused repeat mutates nothing ({ta})");
    }
    let second_erasure: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
         WHERE t.name = 'resource_erased'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        second_erasure, 1,
        "no second resource_erased is minted by the refused repeat"
    );
}

/// Every `kb_block_content` and `kb_chunk_content` row of the resource, and how many of them are
/// not the empty string: (block rows, non-empty block rows, chunk rows, non-empty chunk rows).
async fn content_rows(pool: &PgPool, resource: ResourceId) -> (i64, i64, i64, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM kb_block_content bc \
                   JOIN kb_block_revisions br ON br.id = bc.block_revision_id \
                   JOIN kb_content_blocks b ON b.id = br.block_id \
                  WHERE b.resource_id = $1), \
                (SELECT count(*) FROM kb_block_content bc \
                   JOIN kb_block_revisions br ON br.id = bc.block_revision_id \
                   JOIN kb_content_blocks b ON b.id = br.block_id \
                  WHERE b.resource_id = $1 AND bc.content IS DISTINCT FROM ''), \
                (SELECT count(*) FROM kb_chunk_content cc \
                   JOIN kb_chunks c ON c.id = cc.chunk_id \
                  WHERE c.resource_id = $1), \
                (SELECT count(*) FROM kb_chunk_content cc \
                   JOIN kb_chunks c ON c.id = cc.chunk_id \
                  WHERE c.resource_id = $1 AND cc.content IS DISTINCT FROM '')",
    )
    .bind(resource.uuid())
    .fetch_one(pool)
    .await
    .unwrap()
}

/// (3) History reached: one block with three revisions (its create, then two per-block
/// `update_resource` calls, each superseding the prior chunk), and a second block folded by a
/// `replaces_body` mutate. After the act every `kb_block_content` and `kb_chunk_content` row of
/// R is `''` and every chunk's embedding is NULL; replay is byte-identical.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn history_is_reached_across_revisions_folds_and_supersession(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "history-home").await;
    let resource = writes::create_resource_with(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: "history",
            origin_uri: "test://history",
            body: SECRET,
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: Some(vec![chunk(SECRET, "first")]),
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("seed R through the create path");
    let block: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded \
          ORDER BY seq LIMIT 1",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();

    // Revisions two and three of the one block, each superseding the prior chunk.
    for prose in [
        "revision two: SSN 987-65-4321",
        "revision three: SSN 555-44-3333",
    ] {
        writes::update_resource(
            &pool,
            UpdateParams {
                resource,
                body: Some(prose),
                title: None,
                origin_uri: None,
                properties: &[],
                unset_keys: &[],
                chunks: Some(vec![chunk(prose, "")]),
                sources: vec![],
                content_block: Some(block),
                rehome_to: None,
                emitter,
            },
        )
        .await
        .expect("a per-block revise");
    }
    let revisions: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_block_revisions WHERE block_id = $1")
            .bind(block)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        revisions >= 3,
        "the witness needs three revisions of one block; got {revisions}"
    );

    // A second block, then a `replaces_body` mutate of the first that folds it.
    const SIDE: &str = "a side block naming jane smith";
    let mut side =
        temper_substrate::content::prepare_block_from_chunks(1, None, vec![chunk(SIDE, "side")]);
    side.raw_text = Some(SIDE.to_owned());
    let sibling = writes::append_block(
        &pool,
        writes::AppendParams {
            resource,
            block: &side,
            sources: vec![],
            emitter,
        },
    )
    .await
    .expect("append a second block");
    const WHOLE: &str = "the whole body, replaced";
    let replacement =
        temper_substrate::content::prepare_block_from_chunks(0, None, vec![chunk(WHOLE, "")]);
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::BlockMutate {
            block: BlockId::from(block),
            chunks: &replacement.chunks,
            raw: Some(WHOLE),
            incorporated: &[],
            replaces_body: true,
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let sibling_folded: bool =
        sqlx::query_scalar("SELECT is_folded FROM kb_content_blocks WHERE id = $1")
            .bind(sibling.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        sibling_folded,
        "the replaces_body mutate folded the sibling"
    );
    let superseded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks WHERE resource_id = $1 AND NOT is_current",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        superseded >= 3,
        "the witness needs superseded chunks (two revises + the folded sibling's); got {superseded}"
    );
    let (blocks_before, blocks_filled, chunks_before, chunks_filled) =
        content_rows(&pool, resource).await;
    assert!(
        blocks_filled > 0 && chunks_filled > 0,
        "the witness needs content to erase; got {blocks_filled} block and {chunks_filled} chunk rows"
    );

    execute_act(&pool, resource.uuid()).await;

    let (blocks_after, blocks_left, chunks_after, chunks_left) =
        content_rows(&pool, resource).await;
    assert_eq!(
        (blocks_after, chunks_after),
        (blocks_before, chunks_before),
        "emptied rows stay rows"
    );
    assert_eq!(
        (blocks_left, chunks_left),
        (0, 0),
        "every kb_block_content and kb_chunk_content row of R is '' — every revision, the \
         folded block, every superseded chunk"
    );
    let still_embedded: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks WHERE resource_id = $1 \
            AND (embedding IS NOT NULL OR embedded_with IS NOT NULL)",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(still_embedded, 0, "every embedding of R is NULL");

    assert_replay_byte_identical(
        &pool,
        "of an erasure over revisions, a fold and supersession",
    )
    .await;
}

/// (8) Soft delete is not erasure: the act's survey says not-erased, `erased_at` stays NULL, and
/// the content is intact — the negative face's cheapest witness.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_soft_deleted_resource_is_not_an_erased_one(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "soft-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "soft-twin").await,
    )
    .await;

    // soft-delete through the REAL path
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::ResourceDelete {
            resource: leak.resource,
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    let (is_active, erased_at): (bool, Option<chrono::DateTime<chrono::Utc>>) =
        sqlx::query_as("SELECT is_active, erased_at FROM kb_resources WHERE id = $1")
            .bind(leak.resource.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!is_active, "the soft delete flipped is_active");
    assert!(
        erased_at.is_none(),
        "erased_at IS NULL — a tombstone is not a husk"
    );
    let prose: String = sqlx::query_scalar(
        "SELECT cc.content FROM kb_chunks c \
           JOIN kb_content_blocks b ON b.id = c.block_id \
           JOIN kb_chunk_content cc ON cc.chunk_id = c.id \
          WHERE b.resource_id = $1 AND c.is_current AND cc.content <> '' LIMIT 1",
    )
    .bind(leak.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        prose, CLEAN,
        "the current content is intact — soft delete hides, never erases"
    );
    let already: Option<bool> = sqlx::query_scalar::<_, bool>(
        "SELECT (resource_erasure_survey_plan($1)->>'already_erased')::boolean",
    )
    .bind(leak.resource.uuid())
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(already, Some(false), "the survey says NOT erased");
}

/// (11, SQL half) The closed refusals RAISE at the SQL surface: already-erased and a charter
/// resource; a NULL request reference raises at both the act and the refusal, and the act it
/// refused leaves the resource un-erased. The two former refusals complete (D5): a resource whose
/// ingest is in flight is erased and the record's `targets` names the ingest the act ended, and a
/// tombstone is erased. The recorded face is the service's.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_sql_refusals_raise_and_the_former_refusals_complete(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "refuse-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "refuse-twin").await,
    )
    .await;

    // A NULL request reference: without it every event would correlate to itself and the act's
    // span would be lost, so both SQL faces refuse it.
    let no_ref_act = sqlx::query_scalar::<sqlx::Postgres, serde_json::Value>(
        "SELECT resource_erasure_execute($1,$2,$3,$4)",
    )
    .bind(leak.resource.uuid())
    .bind(owner.uuid())
    .bind(emitter)
    .bind(Option::<Uuid>::None)
    .fetch_one(&pool)
    .await;
    let msg = no_ref_act.unwrap_err().to_string();
    assert!(
        msg.contains("resource_erasure_execute: p_request_ref is required"),
        "the act refuses a NULL request reference; got {msg}"
    );
    let untouched: Option<chrono::DateTime<chrono::Utc>> =
        sqlx::query_scalar("SELECT erased_at FROM kb_resources WHERE id = $1")
            .bind(leak.resource.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(untouched.is_none(), "the refused act erased nothing");
    let no_ref_refusal = sqlx::query_scalar::<sqlx::Postgres, Uuid>(
        "SELECT resource_erasure_refuse($1,$2,$3,$4,'unauthorized')",
    )
    .bind(leak.resource.uuid())
    .bind(owner.uuid())
    .bind(emitter)
    .bind(Option::<Uuid>::None)
    .fetch_one(&pool)
    .await;
    let msg = no_ref_refusal.unwrap_err().to_string();
    assert!(
        msg.contains("resource_erasure_refuse: p_request_ref is required"),
        "the refusal refuses a NULL request reference; got {msg}"
    );

    execute_act(&pool, leak.resource.uuid()).await;

    // already-erased
    let again =
        sqlx::query_scalar::<sqlx::Postgres, Uuid>("SELECT resource_erasure_execute($1,$2,$3,$4)")
            .bind(leak.resource.uuid())
            .bind(owner.uuid())
            .bind(emitter)
            .bind(Uuid::now_v7())
            .fetch_one(&pool)
            .await;
    let msg = again.unwrap_err().to_string();
    assert!(
        msg.contains("already erased"),
        "the repeat refusal says why; got {msg}"
    );

    // a charter resource (a cogmap's telos) refuses — the map-grain act is another task.
    // The map is made through the REAL cogmap-genesis path, so its ledger event is there for
    // replay. Genesis MINTS the telos resource itself — a pre-created one at the same id
    // collides (`kb_resources_pkey`), because the projector owns both inserts.
    let refusal_map = {
        let mut conn = pool.acquire().await.unwrap();
        fire(
            &mut conn,
            SeedAction::CogmapGenesis {
                name: "refusal-map",
                telos_title: "telos-for-refusal",
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
        .unwrap()
        .1
    };
    let telos = refusal_map;
    let _ = refusal_map;
    let charter =
        sqlx::query_scalar::<sqlx::Postgres, Uuid>("SELECT resource_erasure_execute($1,$2,$3,$4)")
            .bind(telos.uuid())
            .bind(owner.uuid())
            .bind(emitter)
            .bind(Uuid::now_v7())
            .fetch_one(&pool)
            .await;
    assert!(
        charter
            .unwrap_err()
            .to_string()
            .contains("charter resource"),
        "the charter refusal names itself"
    );

    // An ingest in flight is NOT a refusal (D5): a segmented-ingest resource, not yet
    // finalized, is erased; the ingest ends with the erasure, and the record names it.
    let in_flight = writes::create_resource_with_mode(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: "mid-ingest",
            origin_uri: "test://mid-ingest",
            body: "block zero",
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
        CreateMode {
            defer: false,
            segmented: true,
        },
    )
    .await
    .unwrap();
    let ingest_before: String =
        sqlx::query_scalar("SELECT ingest_state FROM kb_resources WHERE id = $1")
            .bind(in_flight.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        ingest_before, "in_progress",
        "the witness needs an ingest actually in flight"
    );
    let flight_event = execute_act(&pool, in_flight.uuid()).await;
    let (f_erased, f_ingest): (Option<chrono::DateTime<chrono::Utc>>, String) =
        sqlx::query_as("SELECT erased_at, ingest_state FROM kb_resources WHERE id = $1")
            .bind(in_flight.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        f_erased.is_some(),
        "the in-flight resource is erased; erased_at is authoritative"
    );
    assert_eq!(
        f_ingest, "in_progress",
        "the husk keeps its ingest_state (D5)"
    );
    let flight_prose: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM kb_chunk_content cc \
                   JOIN kb_chunks c ON c.id = cc.chunk_id \
                  WHERE c.resource_id = $1 AND cc.content <> '') \
              + (SELECT count(*) FROM kb_block_content bc \
                   JOIN kb_block_revisions br ON br.id = bc.block_revision_id \
                   JOIN kb_content_blocks b ON b.id = br.block_id \
                  WHERE b.resource_id = $1 AND bc.content <> '')",
    )
    .bind(in_flight.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(flight_prose, 0, "the in-flight resource's content is empty");
    let flight_targets: serde_json::Value =
        sqlx::query_scalar("SELECT payload->'targets' FROM kb_events WHERE id = $1")
            .bind(flight_event)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        flight_targets
            .as_array()
            .expect("targets is an array")
            .contains(&serde_json::json!({
                "target": "kb_resources.ingest_state",
                "outcome": "ingest in_progress; ended by erasure; erased_at is authoritative",
            })),
        "the record's targets name the ingest the erasure ended; got {flight_targets}"
    );

    // a nil resource cannot execute (not-found)
    let pending =
        sqlx::query_scalar::<sqlx::Postgres, Uuid>("SELECT resource_erasure_execute($1,$2,$3,$4)")
            .bind(Uuid::nil())
            .bind(owner.uuid())
            .bind(emitter)
            .bind(Uuid::now_v7())
            .fetch_one(&pool)
            .await;
    assert!(pending.is_err(), "a nil resource cannot execute");

    // A TOMBSTONE IS ERASABLE — the compliance flow's main shape: the content was
    // soft-deleted because it should never have been persisted, then the compliance need
    // arrives demanding it not exist at all. The act completes over one (D5).
    let tombstone = writes::create_resource_with(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: "tombstoned",
            origin_uri: "test://tombstone",
            body: "gone from the estate",
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
    .unwrap();
    // Tombstone through the REAL delete path, so the ledger carries its `resource_deleted`.
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::ResourceDelete {
            resource: tombstone,
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let tomb_deleted: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'resource_deleted' AND (e.payload->>'resource_id')::uuid = $1",
    )
    .bind(tombstone.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        tomb_deleted, 1,
        "the tombstone's resource_deleted is on the ledger"
    );
    // The act COMPLETES: a tombstone is not a refusal state.
    let _tomb_event = execute_act(&pool, tombstone.uuid()).await;
    let (t_active, t_erased): (bool, Option<chrono::DateTime<chrono::Utc>>) =
        sqlx::query_as("SELECT is_active, erased_at FROM kb_resources WHERE id = $1")
            .bind(tombstone.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        !t_active && t_erased.is_some(),
        "the tombstone became a husk; got is_active={t_active} erased_at={t_erased:?}"
    );
    let tomb_prose: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks c \
           JOIN kb_content_blocks b ON b.id = c.block_id \
           JOIN kb_chunk_content cc ON cc.chunk_id = c.id \
          WHERE b.resource_id = $1 AND cc.content <> ''",
    )
    .bind(tombstone.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        tomb_prose, 0,
        "the tombstone's content is gone — soft delete hid it, the act ended it"
    );

    // The tombstone-then-erased ledger REPLAYS byte-identically: the walk projects the
    // `resource_deleted` (is_active cleared) at its position between create and erase, and
    // the erasure arm then applies the body at its own position.
    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();
    let after = replay::dump_projections(&pool).await.unwrap();
    for ((ta, a), (_tb, b)) in before.iter().zip(after.iter()) {
        assert_eq!(
            a, b,
            "projection table {ta} diverged under replay of a tombstone-then-erased erasure"
        );
    }
}

/// (1, the twin half) A sibling with identical bytes in ANOTHER home keeps its content AND its
/// embedding — replayed proof — while the act empties R's rows row-anchored. `kb_erased_content`
/// gains no row.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn custody_is_never_decided_by_bytes(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        make_home(&pool, owner, "custody-home").await,
        make_home(&pool, owner, "custody-twin").await,
    )
    .await;
    let twin_vector_sql =
        "SELECT search_vector::text FROM kb_resource_search_index WHERE resource_id = $1";
    let twin_vector_before: String = sqlx::query_scalar(twin_vector_sql)
        .bind(leak.twin.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        !twin_vector_before.is_empty(),
        "setup: the twin has a non-empty search vector"
    );

    execute_act(&pool, leak.resource.uuid()).await;

    // The twin: content, embedding and search vector SURVIVE, byte for byte.
    let twin_prose: Option<String> = sqlx::query_scalar::<_, String>(
        "SELECT cc.content FROM kb_chunks c \
           JOIN kb_content_blocks b ON b.id = c.block_id \
           JOIN kb_chunk_content cc ON cc.chunk_id = c.id \
          WHERE b.resource_id = $1 AND c.is_current LIMIT 1",
    )
    .bind(leak.twin.uuid())
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(
        twin_prose,
        Some(SECRET.to_string()),
        "the twin keeps its prose"
    );
    let twin_vec: Option<bool> = sqlx::query_scalar::<_, bool>(
        "SELECT (c.embedding IS NOT NULL) FROM kb_chunks c \
           JOIN kb_content_blocks b ON b.id = c.block_id \
          WHERE b.resource_id = $1 AND c.is_current LIMIT 1",
    )
    .bind(leak.twin.uuid())
    .fetch_optional(&pool)
    .await
    .unwrap();
    assert_eq!(
        twin_vec,
        Some(true),
        "the twin's embedding is untouched (the drain never re-embeds it)"
    );
    let twin_vector_after: String = sqlx::query_scalar(twin_vector_sql)
        .bind(leak.twin.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        twin_vector_after, twin_vector_before,
        "the twin's search vector is untouched"
    );
    let twin_alive: bool = sqlx::query_scalar("SELECT is_active FROM kb_resources WHERE id = $1")
        .bind(leak.twin.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(twin_alive, "the twin is a live resource in every dimension");

    // No hash enters the principal act's erased-content set (the fence of fences).
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_erased_content")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        n, 0,
        "NO hash enters kb_erased_content from a resource erasure"
    );
}

/// Unset via the REAL path: update_resource's unset_keys (the key-grain delete event).
async fn unset_via_update(
    pool: &PgPool,
    resource: ResourceId,
    emitter: EntityId,
) -> Result<(), anyhow::Error> {
    writes::update_resource(
        pool,
        UpdateParams {
            resource,
            body: None,
            title: None,
            origin_uri: None,
            properties: &[],
            unset_keys: &["alpha-key".to_owned()],
            chunks: None,
            sources: vec![],
            content_block: None,
            rehome_to: None,
            emitter,
        },
    )
    .await
}

/// (9, the mapping half) A key set → unset → re-set over the resource's events maps to ONE
/// erased-key-<n>; two keys tied on occurred_at number by event id; after the act every row R
/// owns carries a sentinel key and the `"erased"` value; replay is byte-identical (the mapping is
/// a pure function of ledger order).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_reset_key_maps_to_one_sentinel_key(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "keymap-home").await;
    let resource = writes::create_resource_with(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: "keyed",
            origin_uri: "test://keyed",
            body: "plainer",
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[("alpha-key".into(), serde_json::json!("beta"))],
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .unwrap();

    // THE SAME original key, set → unset → re-set: three property events, ONE family
    // position.
    unset_via_update(&pool, resource, emitter).await.unwrap();
    writes::set_property(
        &pool,
        resource,
        "alpha-key",
        &serde_json::json!("gamma"),
        emitter,
    )
    .await
    .unwrap();

    // The tie-break (D4): two keys asserted in ONE transaction, their text sorting opposite to
    // their event order — `zeta_diagnosis` first, then `alpha_name`. One transaction stamps
    // both events with the same occurred_at (`_event_append` leaves it to the column default,
    // now() = the transaction's start), so this IS the tie case: the event id must decide, and
    // the key text must not. Numbering by key text would put alpha_name ahead.
    let mut tx = pool.begin().await.unwrap();
    for (key, value) in [("zeta_diagnosis", "stage 3"), ("alpha_name", "jane smith")] {
        writes::set_property_in_tx(
            &mut tx,
            resource,
            key,
            &serde_json::json!(value),
            emitter,
            EventContext::default(),
        )
        .await
        .unwrap();
    }
    tx.commit().await.unwrap();
    let tie_row = |key: &'static str| {
        let pool = pool.clone();
        let resource = resource.uuid();
        async move {
            sqlx::query_as::<_, (Uuid, Uuid, chrono::DateTime<chrono::Utc>)>(
                "SELECT p.id, ev.id, ev.occurred_at FROM kb_properties p \
                   JOIN kb_events ev ON ev.id = p.asserted_by_event_id \
                  WHERE p.owner_table='kb_resources' AND p.owner_id=$1 AND p.property_key=$2",
            )
            .bind(resource)
            .bind(key)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let (zeta_row, zeta_event, zeta_at) = tie_row("zeta_diagnosis").await;
    let (alpha_row, alpha_event, alpha_at) = tie_row("alpha_name").await;
    assert_eq!(
        zeta_at, alpha_at,
        "precondition: one transaction ties the two keys on occurred_at"
    );
    assert!(
        zeta_event < alpha_event,
        "precondition: zeta_diagnosis's event is first in ledger identity"
    );

    execute_act(&pool, resource.uuid()).await;

    // Every property event that named the key now names ONE sentineled form: the
    // whole family collapses to exactly one `erased-key-<n>`, asserted exactly —
    // a `.all(starts_with)` would pass even if a regression split the family in two.
    let mut keys: Vec<String> = sqlx::query_scalar(
        "SELECT distinct property_key FROM kb_properties \
          WHERE owner_table='kb_resources' AND owner_id=$1",
    )
    .bind(resource.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "erased-key-1".to_owned(),
            "erased-key-2".to_owned(),
            "erased-key-3".to_owned(),
            "erased-key-4".to_owned(),
        ],
        // key-1 is the resource's birth `doc_type` (the earliest first-asserting event);
        // key-2 is the alpha-key family; key-3 and key-4 are the tied pair. FOUR keys,
        // each ONE — the reset family did not split.
        "the reset key's family maps ONE erased-key-<n> per key; distinct keys now: {keys:?}"
    );

    // The tied pair numbers by ledger identity: zeta_diagnosis (first event) before
    // alpha_name, whatever their text.
    let key_of = |row: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT property_key FROM kb_properties WHERE id=$1")
                .bind(row)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(
        key_of(zeta_row).await,
        "erased-key-3",
        "zeta_diagnosis, asserted first in the tied transaction, takes the lower number"
    );
    assert_eq!(
        key_of(alpha_row).await,
        "erased-key-4",
        "alpha_name, asserted second, takes the next number despite sorting first as text"
    );

    // Values: every row R owns, live and folded, carries the sentinel — "beta", "gamma",
    // "stage 3" and "jane smith" are gone.
    let values: Vec<serde_json::Value> = sqlx::query_scalar(
        "SELECT property_value FROM kb_properties \
          WHERE owner_table='kb_resources' AND owner_id=$1",
    )
    .bind(resource.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        !values.is_empty() && values.iter().all(|v| v == &serde_json::json!("erased")),
        "every value R owns is the sentinel; got {values:?}"
    );

    // The typed roundtrip contract is what catches a payload-shape drift like a
    // wrapped `{"edge_id": …}` item instead of a bare uuid — add it to the walk.
    payloads::verify_ledger_roundtrip(&pool).await.unwrap();

    assert_replay_byte_identical(&pool, "of a reset key and a tied pair").await;
}

/// (9, the facet half) Two live rows of ONE key — the facet shape `facet_set` exists for — must
/// sentinel without colliding on `uq_kb_properties_active`. The act folds the whole family (one
/// live row per key is exactly what a two-value facet breaks), so the husk's surface is empty and
/// the collision cannot raise. Replay reproduces the fold + sentinels.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn two_live_rows_of_one_key_sentinel_without_colliding(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "facet-home").await;
    let resource = writes::create_resource_with(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: "faceted",
            origin_uri: "test://faceted",
            body: "facet body",
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
    .unwrap();

    // TWO live rows of one key via facet_set: the `facet` key APPENDS rather than folds, and
    // one object value with two inner keys projects TWO live rows — exactly the shape the
    // sentinel pass would collide on if it left rows live.
    let vals: Vec<temper_substrate::ids::PropertyId> = writes::set_facet(
        &pool,
        PropertyOwner::resource(resource),
        &serde_json::json!({"status": "open", "owner": "jane smith"}),
        1.0,
        emitter,
    )
    .await
    .unwrap();
    assert_eq!(
        vals.len(),
        2,
        "one fire, two marks — TWO live rows; got {vals:?}"
    );

    let live_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties \
          WHERE owner_table='kb_resources' AND owner_id=$1 AND property_key='facet' AND NOT is_folded",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        live_before, 2,
        "TWO live rows of one key (the facet shape); got {live_before}"
    );

    // The planted values, present before the act — the absence check below needs them to bite.
    let planted_sql = "SELECT count(*) FROM kb_properties \
          WHERE owner_table='kb_resources' AND owner_id=$1 \
            AND property_value::text LIKE ANY (ARRAY['%\"open\"%', '%\"jane smith\"%'])";
    let planted_before: i64 = sqlx::query_scalar(planted_sql)
        .bind(resource.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        planted_before, 2,
        "setup: both planted values are on R's rows before the act"
    );

    execute_act(&pool, resource.uuid()).await;

    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties \
          WHERE owner_table='kb_resources' AND owner_id=$1 AND NOT is_folded AND property_key <> 'doc_type'",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(live, 0, "the husk keeps NO live metadata (Q3)");
    let planted_after: i64 = sqlx::query_scalar(planted_sql)
        .bind(resource.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        planted_after, 0,
        "neither planted value (\"open\", \"jane smith\") survives, folded or live"
    );
    let stale_keys: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties \
          WHERE owner_table='kb_resources' AND owner_id=$1 \
            AND property_key NOT LIKE 'erased-key-%'",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stale_keys, 0, "no original key survives folded either");

    // Replay reproduces the same folded+sentinelled family.
    let before = replay::dump_projections(&pool).await.unwrap();
    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;
    replay::replay(&pool, &snap).await.unwrap();
    let after = replay::dump_projections(&pool).await.unwrap();
    for ((ta, a), (_tb, b)) in before.iter().zip(after.iter()) {
        assert_eq!(
            a, b,
            "projection table {ta} diverged under replay of a facet-sentineled erasure"
        );
    }
}

/// (Pre-existing fold) A resource whose history carries an ALREADY-FOLDED edge is a lawful
/// state — the act completes on the LIVE edges and records only what it folds; a pre-existing
/// fold does not abort execute.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_pre_existing_folded_edge_does_not_abort_the_act(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "prefold-home").await;
    let other = make_home(&pool, owner, "prefold-other").await;

    let a = writes::create_resource_with(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: "with-history",
            origin_uri: "test://with-history",
            body: "history carries an ended edge",
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
    .unwrap();
    let b = writes::create_resource_with(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: "other party",
            origin_uri: "test://other-party",
            body: "b",
            doc_type: "research",
            home: AnchorRef::context(other),
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
    .unwrap();

    let edge = writes::assert_relationship(
        &pool,
        AssertParams {
            src: a,
            tgt: b,
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("superseded-by"),
            weight: 1.0,
            home,
            emitter,
        },
    )
    .await
    .unwrap();

    // History folds it lawfully, for its own reason — before the erasure is ever asked.
    writes::fold_relationship(&pool, edge, Some("superseded"), emitter)
        .await
        .unwrap();

    execute_act(&pool, a.uuid()).await;

    // The folded edge was NOT re-listed as this act's work: exactly one fold of it
    // exists in the record (the history's own), and the act completed. The act's fold
    // events carry the payload FLAT (edge_id at the top level) — a regression that
    // re-folded a pre-folded edge would show up here.
    let act_folds: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'relationship_folded' AND (e.payload->>'edge_id')::uuid = $1 \
            AND e.payload->>'reason' = 'resource_erased'",
    )
    .bind(edge)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        act_folds, 0,
        "the act folds only LIVE edges; the pre-existing fold keeps its own history"
    );
}

/// (23) Edge-owned content is gone (goal §8: an edge touching R is R's surface, whoever authored
/// it). A second principal's resource S asserts an edge S→R carrying a label, an edge facet and a
/// keyed edge property, plus a second S→R edge it folded itself before the act. After the act
/// both edges are folded with their structure intact (kind, polarity, asserting event,
/// endpoints) and their label NULL; every edge-owned row carries an `erased-key-<n>` key numbered
/// within its edge by ledger identity and the `"erased"` value; S itself is untouched; replay is
/// byte-identical.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_edge_from_another_principal_keeps_its_structure_and_loses_its_text(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "edge-text-home").await;
    let r = writes::create_resource_with(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: "the erased one",
            origin_uri: "test://edge-text-r",
            body: "r body",
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
    .unwrap();

    // B is a REAL second principal — own profile, own emitter, own context (the
    // `a_cross_principal_recommit_is_the_committers_own_row` setup in blobs.rs).
    let owner_b = ProfileId::from(common::insert_profile(&pool, "edge-text-b").await);
    let emitter_b = EntityId::from(
        sqlx::query_scalar::<sqlx::Postgres, Uuid>(
            "INSERT INTO kb_entities (profile_id, name, metadata) \
             VALUES ($1, 'edge-text-b@web', '{}'::jsonb) RETURNING id",
        )
        .bind(owner_b.uuid())
        .fetch_one(&pool)
        .await
        .unwrap(),
    );
    let home_b = make_home(&pool, owner_b, "edge-text-b-home").await;
    let s = writes::create_resource_with(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: "B's own notes",
            origin_uri: "test://edge-text-s",
            body: "s body",
            doc_type: "research",
            home: AnchorRef::context(home_b),
            owner: owner_b,
            originator: owner_b,
            emitter: emitter_b,
            properties: &[],
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .unwrap();

    // The live edge S→R, authored by B: a label, an edge facet, a keyed edge property.
    let edge = writes::assert_relationship(
        &pool,
        AssertParams {
            src: s,
            tgt: r,
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("source for jane smith"),
            weight: 1.0,
            home: home_b,
            emitter: emitter_b,
        },
    )
    .await
    .unwrap();
    let facet_rows = writes::set_facet(
        &pool,
        PropertyOwner::edge(edge),
        &serde_json::json!({"diagnosis": "stage 3"}),
        1.0,
        emitter_b,
    )
    .await
    .unwrap();
    assert_eq!(facet_rows.len(), 1, "one mark, one row; got {facet_rows:?}");
    let keyed_row = writes::assert_keyed_property_with(
        &pool,
        PropertyOwner::edge(edge),
        "patient_ssn",
        &serde_json::json!("123-45-6789"),
        1.0,
        emitter_b,
        EventContext::default(),
    )
    .await
    .unwrap();

    // An S→R edge B folded itself before the act: its facet was folded with it by the fold
    // projector, key and value intact — the act must still reach it.
    let folded_edge = writes::assert_relationship(
        &pool,
        AssertParams {
            src: s,
            tgt: r,
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("earlier link to jane"),
            weight: 1.0,
            home: home_b,
            emitter: emitter_b,
        },
    )
    .await
    .unwrap();
    writes::set_facet(
        &pool,
        PropertyOwner::edge(folded_edge),
        &serde_json::json!({"alias": "jd-alias"}),
        1.0,
        emitter_b,
    )
    .await
    .unwrap();
    writes::fold_relationship(&pool, folded_edge, Some("superseded"), emitter_b)
        .await
        .unwrap();

    type Structure = (String, Uuid, String, Uuid, String, String, Uuid);
    let structure = |e: EdgeId| {
        let pool = pool.clone();
        async move {
            sqlx::query_as::<_, Structure>(
                "SELECT source_table::text, source_id, target_table::text, target_id, \
                        edge_kind::text, polarity::text, asserted_by_event_id \
                   FROM kb_edges WHERE id=$1",
            )
            .bind(e)
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let s_state = |res: ResourceId| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, serde_json::Value>(
                "SELECT jsonb_build_object( \
                    'resource', (SELECT to_jsonb(x) FROM kb_resources x WHERE x.id=$1), \
                    'properties', (SELECT coalesce(jsonb_agg(to_jsonb(p) ORDER BY p.id), '[]') \
                                     FROM kb_properties p \
                                    WHERE p.owner_table='kb_resources' AND p.owner_id=$1), \
                    'chunks', (SELECT coalesce(jsonb_agg(cc.content ORDER BY c.id), '[]') \
                                 FROM kb_chunks c \
                                 JOIN kb_chunk_content cc ON cc.chunk_id = c.id \
                                WHERE c.resource_id=$1))",
            )
            .bind(res.uuid())
            .fetch_one(&pool)
            .await
            .unwrap()
        }
    };
    let edge_before = structure(edge).await;
    let folded_before = structure(folded_edge).await;
    let s_before = s_state(s).await;

    let request_ref = Uuid::now_v7();
    execute_act_with_ref(&pool, r.uuid(), request_ref).await;

    // 6: B's live edge is ended by a `relationship_folded` on ITS OWN trail — keyed to the edge,
    // anchored in the edge's home (B's context), under the act's request reference. The
    // already-folded edge gets no second fold.
    let folds_on_trail = |e: EdgeId| {
        let pool = pool.clone();
        async move {
            sqlx::query_as::<_, (String, Option<String>, Option<Uuid>)>(
                "SELECT e.payload->>'reason', e.producing_anchor_table, e.producing_anchor_id \
                   FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
                  WHERE t.name = 'relationship_folded' \
                    AND (e.payload->>'edge_id')::uuid = $1 AND e.correlation_id = $2",
            )
            .bind(e)
            .bind(request_ref)
            .fetch_all(&pool)
            .await
            .unwrap()
        }
    };
    assert_eq!(
        folds_on_trail(edge).await,
        vec![(
            "resource_erased".to_owned(),
            Some("kb_contexts".to_owned()),
            Some(home_b.uuid())
        )],
        "the act folds B's edge through one event on that edge's trail, in B's home"
    );
    assert!(
        folds_on_trail(folded_edge).await.is_empty(),
        "an edge B already folded gets no second fold from the act"
    );

    // Structure stays: both edges folded, kind/polarity/asserting event/endpoints unchanged,
    // label NULL.
    for (e, before) in [(edge, &edge_before), (folded_edge, &folded_before)] {
        assert_eq!(&structure(e).await, before, "edge {e} keeps its structure");
        let (folded, label): (bool, Option<String>) =
            sqlx::query_as("SELECT is_folded, label FROM kb_edges WHERE id=$1")
                .bind(e)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(folded, "edge {e} ends folded");
        assert_eq!(label, None, "edge {e} loses its label");
    }

    // Every edge-owned row is sentineled and folded; nothing of the original text survives.
    let rows: Vec<(Uuid, Uuid, String, serde_json::Value, bool)> = sqlx::query_as(
        "SELECT id, owner_id, property_key, property_value, is_folded FROM kb_properties \
          WHERE owner_table='kb_edges' AND owner_id = ANY($1) ORDER BY owner_id, id",
    )
    .bind(vec![edge.uuid(), folded_edge.uuid()])
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        rows.len(),
        3,
        "two rows on the live edge, one on the folded; got {rows:?}"
    );
    for (id, owner_id, key, value, folded) in &rows {
        assert!(
            key.starts_with("erased-key-"),
            "row {id} of edge {owner_id} keeps a sentinel key; got {key}"
        );
        assert_eq!(
            value,
            &serde_json::json!("erased"),
            "row {id} value sentineled"
        );
        assert!(folded, "row {id} ends folded");
    }
    let leaked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties \
          WHERE (owner_table='kb_edges' AND property_key IN ('facet', 'patient_ssn')) \
             OR property_value::text LIKE ANY (ARRAY['%diagnosis%', '%stage 3%', \
                                                     '%123-45-6789%', '%jd-alias%'])",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(leaked, 0, "no row carries an original edge key or value");

    // Numbering is WITHIN each edge by ledger identity: on the live edge the facet (asserted
    // first) is erased-key-1 and patient_ssn erased-key-2; the folded edge's one key is its own
    // erased-key-1.
    let key_of = |row: Uuid| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>("SELECT property_key FROM kb_properties WHERE id=$1")
                .bind(row)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    assert_eq!(key_of(facet_rows[0].uuid()).await, "erased-key-1");
    assert_eq!(key_of(keyed_row.uuid()).await, "erased-key-2");
    let folded_keys: Vec<String> = rows
        .iter()
        .filter(|(_, o, ..)| *o == folded_edge.uuid())
        .map(|(_, _, k, ..)| k.clone())
        .collect();
    assert_eq!(folded_keys, vec!["erased-key-1".to_owned()]);

    // S itself — B's resource, its properties, its chunks — is untouched.
    assert_eq!(
        s_state(s).await,
        s_before,
        "the other principal's resource is untouched"
    );

    assert_replay_byte_identical(&pool, "of an erasure reaching another principal's edges").await;
}

/// (Blob strike) THE BLOB STRIKE ARM + its list-verification fence: a
/// listed live blob is struck through `blob_delete('blob_erased', …)` (released verdict;
/// the row's outcome prose is the fence template byte-parsed downstream), an operator
/// widening the plan mid-act is REFUSED, and the struck blob is named in `targets`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_operator_listed_blob_strike_verifies_and_strikes(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "strike-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "strike-twin").await,
    )
    .await;

    // The blob + its relation edge, through the REAL write paths, so each projection has its
    // event behind it: commit_blob stores + emits `blob_committed`, and the relation is an
    // ordinary edge — `AnchorRef::blob` is a lawful source (D3).
    let bytes = b"blob bytes under erasure".to_vec();
    let hash = {
        use sha2::Digest as _;
        format!("{:x}", sha2::Sha256::digest(&bytes))
    };
    let pathname = temper_substrate::blob_store::blob_pathname(&hash);
    let store = InMemoryBlobStore::default().with_object(pathname.clone());
    let blob = writes::commit_blob(
        &pool,
        &store,
        CommitBlobParams {
            id: BlobId::from(Uuid::now_v7()),
            home: AnchorRef::context(home),
            owner,
            originator: None,
            content_hash: hash.clone(),
            content_type: "image/png".to_owned(),
            content_bytes: bytes.len() as i64,
            max_bytes: 10 * 1024 * 1024,
            allowlist: &["image/png".to_owned()][..],
            emitter,
        },
    )
    .await
    .expect("the blob commits through the real path");
    {
        let mut conn = pool.acquire().await.unwrap();
        fire(
            &mut conn,
            SeedAction::RelationshipAssert {
                src: payloads::AnchorRef::blob(blob),
                tgt: payloads::AnchorRef::resource(leak.resource),
                kind: EdgeKind::Contains,
                polarity: EdgePolarity::Forward,
                label: Some("derived-from"),
                weight: 1.0,
                home: EdgeHome::Context(home),
                emitter,
            },
        )
        .await
        .expect("the blob-resource relation asserts through the real path");
    }

    // SURVEY: the plan names the blob in the related-blob remainder.
    let survey: serde_json::Value = sqlx::query_scalar("SELECT resource_erasure_survey($1)")
        .bind(leak.resource.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    let remainder_blob_named = survey
        .get("remainder")
        .and_then(|r| r.as_array())
        .map(|a| {
            a.iter().any(|e| {
                e.get("target").and_then(|t| t.as_str()) == Some("kb_blobs")
                    && e.get("outcome")
                        .and_then(|o| o.as_str())
                        .map(|o| o.contains(&format!("blob {blob};")))
                        .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    assert!(
        remainder_blob_named,
        "the survey names the related blob: {survey}"
    );

    // EXECUTE with the operator's list: the strike runs, released=true, and the
    // byte-delete fence prose is recorded for the fence to parse.
    let (_, operator_entity) = system_actor(&pool).await;
    let outcome: serde_json::Value =
        sqlx::query_scalar("SELECT resource_erasure_execute($1,$2,$3,$4,$5)")
            .bind(leak.resource.uuid())
            .bind(operator_entity)
            .bind(operator_entity)
            .bind(Uuid::now_v7())
            .bind(vec![blob])
            .fetch_one(&pool)
            .await
            .expect("the act completes with the listed strike");
    let struck = outcome
        .get("targets")
        .and_then(|t| t.as_array())
        .map(|a| {
            a.iter()
                .any(|e| e.get("target").and_then(|t| t.as_str()) == Some("kb_blobs"))
        })
        .unwrap_or(false);
    assert!(struck, "the record names the blob strike; got {outcome}");

    // Widening the plan mid-act: a SECOND resource with its own related blob; the
    // operator offers a blob in NO relation to it, and is refused rather than
    // silently struck.
    let resource2 = writes::create_resource_with(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: "strike-target-two",
            origin_uri: "test://strike-two",
            body: "second erasure target",
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
    .unwrap();
    let widened = sqlx::query_scalar::<sqlx::Postgres, Uuid>(
        "SELECT (resource_erasure_execute($1,$2,$3,$4,$5)->>'event_id')::text",
    )
    .bind(resource2.uuid())
    .bind(operator_entity)
    .bind(operator_entity)
    .bind(Uuid::now_v7())
    .bind(vec![blob])
    .fetch_one(&pool)
    .await;
    assert!(
        widened
            .unwrap_err()
            .to_string()
            .contains("related-blob remainder; strike refused"),
        "a listed blob the plan did NOT name is refused, not silently struck"
    );
}

async fn chunk_prose(pool: &PgPool, chunk: Uuid) -> String {
    sqlx::query_scalar("SELECT content FROM kb_chunk_content WHERE chunk_id = $1")
        .bind(chunk)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// What only the whole-resource steps touch: the search vector (step 4), the title and
/// `erased_at` (step 9a).
async fn whole_resource_state(
    pool: &PgPool,
    resource: ResourceId,
) -> (String, String, Option<chrono::DateTime<chrono::Utc>>) {
    sqlx::query_as(
        "SELECT si.search_vector::text, r.title, r.erased_at
           FROM kb_resources r JOIN kb_resource_search_index si ON si.resource_id = r.id
          WHERE r.id = $1",
    )
    .bind(resource.uuid())
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The redaction body's scope parameter (D11): a block set runs steps (1)–(3) narrowed to the
/// named block and stops there, and the two-argument whole-resource call the act and the replay
/// arm make still resolves (one function, no overload).
///
/// FAILS IF: the block-set call raises, leaves the named block's superseded chunk its prose, or
/// reaches a whole-resource step (the search vector, the title, `erased_at`); or the
/// two-argument call stops resolving or stops emptying the resource.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_redaction_body_accepts_a_block_scope_and_stops_after_step_three(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "scope-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "scope-twin").await,
    )
    .await;
    let event: Uuid = sqlx::query_scalar("SELECT id FROM kb_events ORDER BY id LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();
    let block: Uuid = sqlx::query_scalar("SELECT block_id FROM kb_chunks WHERE id = $1")
        .bind(leak.old_chunk)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        chunk_prose(&pool, leak.old_chunk).await,
        SECRET,
        "setup: the superseded chunk carries the leak"
    );
    let before = whole_resource_state(&pool, leak.resource).await;
    assert!(
        !before.0.is_empty(),
        "setup: the search vector is not empty"
    );

    sqlx::query("SELECT _resource_erasure_apply_redaction($1, $2, $3)")
        .bind(leak.resource.uuid())
        .bind(event)
        .bind(vec![block])
        .execute(&pool)
        .await
        .expect("a block-set scope runs");
    assert_eq!(
        chunk_prose(&pool, leak.old_chunk).await,
        "",
        "the named block's superseded chunk is emptied (step (1))"
    );
    assert_eq!(
        whole_resource_state(&pool, leak.resource).await,
        before,
        "the search vector, the title and erased_at are unchanged: steps (4)–(9) did not run"
    );

    sqlx::query("SELECT _resource_erasure_apply_redaction($1, $2)")
        .bind(leak.resource.uuid())
        .bind(event)
        .execute(&pool)
        .await
        .expect("the two-argument whole-resource call resolves and succeeds");
    let leaked: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks c JOIN kb_chunk_content cc ON cc.chunk_id = c.id \
          WHERE c.resource_id = $1 AND cc.content <> ''",
    )
    .bind(leak.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(leaked, 0, "the whole-resource form emptied the chunk prose");
}

// ── Witness 20: no writer lands on the husk (D13) ───────────────────────────────────────────
//
// The `blob_byte_window_test` choreography: a second connection races the act under explicit
// transactions, and a 2-second `tokio::time::timeout` that EXPIRES is the serialization signal —
// the racing side cannot complete while the other side holds R's row.

/// The prose the racing writer lands; it must end nowhere in R's content.
const RACED: &str = "late prose written while the act was waiting";

/// Snapshot, reset, replay, and diff every projection table — the byte-identity check the
/// witnesses above run inline.
async fn assert_replay_byte_identical(pool: &PgPool, after_what: &str) {
    let before = replay::dump_projections(pool).await.unwrap();
    let snap = replay::snapshot(pool).await.unwrap();
    common::reset_schema(pool).await;
    replay::replay(pool, &snap).await.unwrap();
    let after = replay::dump_projections(pool).await.unwrap();
    for ((ta, a), (tb, b)) in before.iter().zip(after.iter()) {
        assert_eq!(ta, tb);
        assert_eq!(
            a, b,
            "projection table {ta} diverged under replay {after_what}"
        );
    }
}

/// (20, content half, writer first) A block mutate holding its transaction makes the act wait
/// on R's row lock; once the writer commits, the act erases what it wrote. The mutate lands
/// BEFORE the act — its event precedes `resource_erased` — and the husk ends with every chunk's
/// prose and every revision's bytes empty.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_writer_holding_its_transaction_makes_the_act_wait_then_is_erased(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "race-writer-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "race-writer-twin").await,
    )
    .await;
    let block: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded \
          ORDER BY seq LIMIT 1",
    )
    .bind(leak.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();

    // The writer: a per-block mutate through the in-transaction write path, NOT committed.
    let mut writer = pool.begin().await.unwrap();
    writes::update_resource_in_tx(
        &mut writer,
        UpdateParams {
            resource: leak.resource,
            body: Some(RACED),
            title: None,
            origin_uri: None,
            properties: &[],
            unset_keys: &[],
            chunks: Some(vec![chunk(RACED, "")]),
            sources: vec![],
            content_block: Some(block),
            rehome_to: None,
            emitter,
        },
        EventContext::default(),
        false,
    )
    .await
    .expect("the racing block mutate writes inside its open transaction");

    let pool_for_act = pool.clone();
    let resource = leak.resource.uuid();
    let mut act = tokio::spawn(async move { execute_act(&pool_for_act, resource).await });
    let finished_within_window =
        tokio::time::timeout(std::time::Duration::from_secs(2), &mut act).await;
    assert!(
        finished_within_window.is_err(),
        "the act completed while a writer held R's row — its FOR UPDATE did not wait"
    );

    writer.commit().await.unwrap();
    let event_id = act.await.expect("the act task must not panic");

    // The mutate landed before the act: its event precedes resource_erased in the ledger.
    let mutated: Vec<Uuid> = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'block_mutated' AND (e.payload->>'block_id')::uuid = $1",
    )
    .bind(block)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        !mutated.is_empty() && mutated.iter().all(|id| *id < event_id),
        "the racing mutate committed before the act; got {mutated:?} vs {event_id}"
    );

    let leaked_chunks: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_chunks c JOIN kb_chunk_content cc ON cc.chunk_id = c.id \
          WHERE c.resource_id = $1 AND cc.content <> ''",
    )
    .bind(resource)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        leaked_chunks, 0,
        "every chunk on the husk is empty — the raced prose included"
    );
    let leaked_revisions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_block_content bc \
           JOIN kb_block_revisions br ON br.id = bc.block_revision_id \
           JOIN kb_content_blocks b ON b.id = br.block_id \
          WHERE b.resource_id = $1 AND bc.content <> ''",
    )
    .bind(resource)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        leaked_revisions, 0,
        "every block revision on the husk is empty — the raced revision included"
    );

    assert_replay_byte_identical(&pool, "of a writer that committed ahead of the act").await;
}

/// (6, the fold race) A principal's fold of an edge touching R, uncommitted when the act reaches
/// that edge, makes the act's fold loop wait on the edge row (`FOR UPDATE`, 20260930000070).
/// Once the fold commits, the loop's `NOT is_folded` is re-checked and the act raises
/// `edge … missing or already folded`; the SQL alone does not retry (the service does). The edge
/// carries exactly ONE `relationship_folded`, the principal's, and a re-run of the act completes.
/// FAILS IF the fold loop reads the edge without a lock: the act then waits on the projector's
/// UPDATE instead, appends its own fold of the already-folded edge and completes — two
/// `relationship_folded` events for one edge, and `expect_err` fails.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_fold_holding_its_transaction_makes_the_act_raise_not_fold_twice(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "fold-race-home").await;
    let other = make_home(&pool, owner, "fold-race-other").await;
    let r = writes::create_resource_with(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: "fold-race-r",
            origin_uri: "test://fold-race-r",
            body: "an edge out of this resource is folded under the act",
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
    .unwrap();
    let t = writes::create_resource_with(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: "fold-race-t",
            origin_uri: "test://fold-race-t",
            body: "the edge's other end",
            doc_type: "research",
            home: AnchorRef::context(other),
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
    .unwrap();
    let edge = writes::assert_relationship(
        &pool,
        AssertParams {
            src: r,
            tgt: t,
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("raced"),
            weight: 1.0,
            home,
            emitter,
        },
    )
    .await
    .unwrap();

    // The principal's fold, through the real fold path, NOT committed.
    let mut folder = pool.begin().await.unwrap();
    writes::fold_relationship_in_tx(
        &mut folder,
        edge,
        Some("the principal's own reason"),
        emitter,
        EventContext::default(),
    )
    .await
    .expect("the racing fold writes inside its open transaction");

    let pool_for_act = pool.clone();
    let resource = r.uuid();
    let mut act = tokio::spawn(async move {
        sqlx::query_scalar::<_, serde_json::Value>("SELECT resource_erasure_execute($1,$2,$3,$4)")
            .bind(resource)
            .bind(emitter)
            .bind(emitter)
            .bind(Uuid::now_v7())
            .fetch_one(&pool_for_act)
            .await
    });
    let finished_within_window =
        tokio::time::timeout(std::time::Duration::from_secs(2), &mut act).await;
    assert!(
        finished_within_window.is_err(),
        "the act finished while a fold of R's edge was uncommitted — it did not wait on the edge"
    );

    folder.commit().await.unwrap();
    let err = act
        .await
        .expect("the act task must not panic")
        .expect_err("the act must raise on an edge folded under it, not fold it again");
    let message = err
        .as_database_error()
        .map(|d| d.message().to_string())
        .unwrap_or_default();
    assert_eq!(
        message,
        format!(
            "resource_erasure_execute: edge {} missing or already folded",
            edge.uuid()
        )
    );

    assert_eq!(
        relationship_folds_of(&pool, edge).await,
        1,
        "one fold of the edge: the principal's"
    );
    let erased: bool =
        sqlx::query_scalar("SELECT erased_at IS NOT NULL FROM kb_resources WHERE id = $1")
            .bind(resource)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!erased, "the raised act committed nothing");

    // The re-plan: the edge is folded now, so the plan leaves it out and the act completes.
    execute_act(&pool, resource).await;
    assert_eq!(
        relationship_folds_of(&pool, edge).await,
        1,
        "the completed act did not fold the edge again"
    );
}

/// (14, the replay arm's validation) A `resource_erased` whose `subject_table` is not
/// `kb_resources` is refused by the replay walk with context, as one missing its `subject_id`
/// is. The event is appended raw (the act only ever writes `kb_resources`), naming a context.
/// FAILS IF the arm reads `subject_id` without checking `subject_table`: replay then runs the
/// redaction body over a context id, and either succeeds or fails with some other message.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_refuses_a_resource_erased_naming_another_table(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "wrong-subject-home").await;
    sqlx::query(
        "SELECT _event_append('resource_erased', $1, NULL, NULL, \
                jsonb_build_object('subject_table', 'kb_contexts', 'subject_id', $2::uuid), \
                p_correlation => $3)",
    )
    .bind(emitter)
    .bind(home.uuid())
    .bind(Uuid::now_v7())
    .execute(&pool)
    .await
    .expect("the raw append lands");

    let snap = replay::snapshot(&pool).await.unwrap();
    common::reset_schema(&pool).await;
    let err = replay::replay(&pool, &snap)
        .await
        .expect_err("replay must refuse a resource_erased naming another table");
    let chain = format!("{err:#}");
    assert!(
        chain.contains("names subject_table \"kb_contexts\", not kb_resources"),
        "{chain}"
    );
}

/// How many `relationship_folded` events name `edge`.
async fn relationship_folds_of(pool: &PgPool, edge: EdgeId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'relationship_folded' AND (e.payload->>'edge_id')::uuid = $1",
    )
    .bind(edge.uuid())
    .fetch_one(pool)
    .await
    .unwrap()
}

/// (20, non-content half, act first) A property set arriving while the act holds R's row waits
/// on the write guard's FOR KEY SHARE; once the act commits, the guard re-reads the row, sees
/// `erased_at`, and refuses. Nothing of the write survives: no row carries its key and no event
/// records it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_writer_after_the_act_is_refused(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "race-act-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "race-act-twin").await,
    )
    .await;

    // The act, run inside a transaction that is NOT committed yet.
    let mut act = pool.begin().await.unwrap();
    sqlx::query("SELECT resource_erasure_execute($1,$2,$3,$4)")
        .bind(leak.resource.uuid())
        .bind(emitter)
        .bind(emitter)
        .bind(Uuid::now_v7())
        .execute(&mut *act)
        .await
        .expect("the act runs inside its open transaction");

    let pool_for_writer = pool.clone();
    let resource = leak.resource;
    let mut writer = tokio::spawn(async move {
        writes::set_property(
            &pool_for_writer,
            resource,
            "raced",
            &serde_json::json!(RACED),
            emitter,
        )
        .await
    });
    let finished_within_window =
        tokio::time::timeout(std::time::Duration::from_secs(2), &mut writer).await;
    assert!(
        finished_within_window.is_err(),
        "the property set completed while the act held R's row — the write guard did not wait"
    );

    act.commit().await.unwrap();
    let refused = writer
        .await
        .expect("the writer task must not panic")
        .expect_err("a write that arrives after the act refuses");
    let expected = format!("resource {} is erased; writes are refused", resource.uuid());
    assert!(
        format!("{refused:#}").contains(&expected),
        "the refusal is the write guard's; got {refused:#}"
    );

    let raced_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties \
          WHERE owner_table = 'kb_resources' AND owner_id = $1 \
            AND (property_key = 'raced' OR property_value = to_jsonb($2::text))",
    )
    .bind(resource.uuid())
    .bind(RACED)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(raced_rows, 0, "no property row carries the refused write");
    let props = resource_props(&pool, resource).await;
    assert!(
        props
            .iter()
            .all(|(k, v)| k.starts_with("erased-key-") && v == &serde_json::json!("erased")),
        "R has no live property but the act's sentinels; got {props:?}"
    );
    let raced_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'property_set' AND e.payload->>'property_key' = 'raced'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        raced_events, 0,
        "the refused write rolled back its event with it"
    );

    assert_replay_byte_identical(&pool, "of a writer refused after the act").await;
}

/// (20, non-content half, writer first) A property set holding its transaction makes the act wait
/// on R's row lock; once the writer commits, the act sentinels what it wrote. The set lands BEFORE
/// the act — its event precedes `resource_erased` — and no row R owns, live or folded, keeps its
/// key or its value.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_property_set_holding_its_transaction_makes_the_act_wait_then_is_sentineled(
    pool: sqlx::PgPool,
) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "race-prop-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "race-prop-twin").await,
    )
    .await;

    // The writer: a property set through the in-transaction write path, NOT committed.
    let mut writer = pool.begin().await.unwrap();
    writes::set_property_in_tx(
        &mut writer,
        leak.resource,
        "raced",
        &serde_json::json!(RACED),
        emitter,
        EventContext::default(),
    )
    .await
    .expect("the racing property set writes inside its open transaction");
    // Its event id, read inside the writer's transaction: after the act the payload's key is a
    // sentinel, so the key text no longer finds it.
    let raced_event: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'property_set' AND e.payload->>'property_key' = 'raced'",
    )
    .fetch_one(&mut *writer)
    .await
    .unwrap();

    let pool_for_act = pool.clone();
    let resource = leak.resource.uuid();
    let mut act = tokio::spawn(async move { execute_act(&pool_for_act, resource).await });
    let finished_within_window =
        tokio::time::timeout(std::time::Duration::from_secs(2), &mut act).await;
    assert!(
        finished_within_window.is_err(),
        "the act completed while a property set held R's row — its FOR UPDATE did not wait"
    );

    writer.commit().await.unwrap();
    let event_id = act.await.expect("the act task must not panic");

    assert!(
        raced_event < event_id,
        "the racing set committed before the act; got {raced_event} vs {event_id}"
    );
    let raced_payload: serde_json::Value =
        sqlx::query_scalar("SELECT payload FROM kb_events WHERE id = $1")
            .bind(raced_event)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        raced_payload["property_key"]
            .as_str()
            .is_some_and(|k| k.starts_with("erased-key-"))
            && raced_payload["value"] == serde_json::json!(format!("erased:{raced_event}")),
        "the raced event's key and value are redacted on the ledger too: {raced_payload}"
    );
    let raced_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties \
          WHERE owner_table = 'kb_resources' AND owner_id = $1 \
            AND (property_key = 'raced' OR property_value = to_jsonb($2::text))",
    )
    .bind(resource)
    .bind(RACED)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        raced_rows, 0,
        "the raced key and value are sentineled like the rest of R's family"
    );
    let unsentineled: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties \
          WHERE owner_table = 'kb_resources' AND owner_id = $1 \
            AND (property_key NOT LIKE 'erased-key-%' OR property_value <> '\"erased\"'::jsonb)",
    )
    .bind(resource)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        unsentineled, 0,
        "every row R owns ends sentineled, the raced one included"
    );

    assert_replay_byte_identical(&pool, "of a property set that committed ahead of the act").await;
}

/// (20, content half, act first) A block mutate arriving while the act holds R's row waits on it;
/// once the act commits, the mutate's write guard re-reads the row, sees `erased_at`, and refuses.
/// Nothing of the write survives: no event records it and no chunk or revision of R carries its
/// prose.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_block_mutate_after_the_act_is_refused(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "race-mutate-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "race-mutate-twin").await,
    )
    .await;
    let block: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded \
          ORDER BY seq LIMIT 1",
    )
    .bind(leak.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let mutates_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'block_mutated' AND (e.payload->>'block_id')::uuid = $1",
    )
    .bind(block)
    .fetch_one(&pool)
    .await
    .unwrap();

    // The act, run inside a transaction that is NOT committed yet.
    let mut act = pool.begin().await.unwrap();
    sqlx::query("SELECT resource_erasure_execute($1,$2,$3,$4)")
        .bind(leak.resource.uuid())
        .bind(emitter)
        .bind(emitter)
        .bind(Uuid::now_v7())
        .execute(&mut *act)
        .await
        .expect("the act runs inside its open transaction");

    let pool_for_writer = pool.clone();
    let resource = leak.resource;
    let mut writer = tokio::spawn(async move {
        writes::update_resource(
            &pool_for_writer,
            UpdateParams {
                resource,
                body: Some(RACED),
                title: None,
                origin_uri: None,
                properties: &[],
                unset_keys: &[],
                chunks: Some(vec![chunk(RACED, "")]),
                sources: vec![],
                content_block: Some(block),
                rehome_to: None,
                emitter,
            },
        )
        .await
    });
    let finished_within_window =
        tokio::time::timeout(std::time::Duration::from_secs(2), &mut writer).await;
    assert!(
        finished_within_window.is_err(),
        "the block mutate completed while the act held R's row — the write guard did not wait"
    );

    act.commit().await.unwrap();
    let refused = writer
        .await
        .expect("the writer task must not panic")
        .expect_err("a block mutate that arrives after the act refuses");
    let expected = format!("resource {} is erased; writes are refused", resource.uuid());
    assert!(
        format!("{refused:#}").contains(&expected),
        "the refusal is the write guard's; got {refused:#}"
    );

    let mutates_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'block_mutated' AND (e.payload->>'block_id')::uuid = $1",
    )
    .bind(block)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        mutates_after, mutates_before,
        "the refused mutate rolled back its event with it"
    );
    let (_, blocks_filled, _, chunks_filled) = content_rows(&pool, resource).await;
    assert_eq!(
        (blocks_filled, chunks_filled),
        (0, 0),
        "no revision or chunk of R carries the refused prose"
    );

    assert_replay_byte_identical(&pool, "of a block mutate refused after the act").await;
}

/// (20, the remaining writers) After the act, every other guarded write path refuses on R: a
/// citation audit of R's block (`_project_citation_audited`), a retype and a reweight of R's
/// folded edge (`_project_relationship_retyped` / `_reweighted`), a verdict on R's artifact
/// (`data_artifact_verdict_upsert`, the writer shape reconcile calls), and the finalize of an
/// erased segmented ingest (`_project_resource_finalized`) — which leaves the husk `in_progress`.
/// Each refusal is the write guard's, and none leaves a row behind.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_other_guarded_writers_refuse_after_the_act(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    register_block_provenance_annotated(&pool).await;
    register_citation_audited(&pool).await;
    register_resource_finalized(&pool).await;
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "guarded-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "guarded-twin").await,
    )
    .await;
    // R's block cites the twin (resource-kind, the auditable kind), so an audit of that citation
    // is a lawful write before the act.
    let r_block: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded \
          ORDER BY seq LIMIT 1",
    )
    .bind(leak.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let r_block = writes::annotate_block_sources(
        &pool,
        writes::AnnotateParams {
            resource: leak.resource,
            sources: vec![Incorporation {
                source: ProvenanceSource::Resource(leak.twin.uuid()),
                seq: 0,
            }],
            content_block: Some(r_block),
            emitter,
        },
    )
    .await
    .expect("R's block cites the twin");

    // A segmented ingest, not yet finalized: its finalize is the last write the act must refuse.
    let in_flight = writes::create_resource_with_mode(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: "mid-ingest",
            origin_uri: "test://guarded-mid-ingest",
            body: "block zero",
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
        CreateMode {
            defer: false,
            segmented: true,
        },
    )
    .await
    .unwrap();

    execute_act(&pool, leak.resource.uuid()).await;
    execute_act(&pool, in_flight.uuid()).await;
    let refusal = |r: ResourceId| format!("resource {} is erased; writes are refused", r.uuid());

    // The citation audit.
    let audit = writes::record_citation_audit(
        &pool,
        writes::CitationAuditParams {
            block: r_block,
            source: ProvenanceSource::Resource(leak.twin.uuid()),
            value: -1.0,
            reason: Some("jane smith's figures, quoted"),
            emitter,
        },
    )
    .await
    .expect_err("an audit of the husk's citation refuses");
    assert!(
        format!("{audit:#}").contains(&refusal(leak.resource)),
        "the audit refusal is the write guard's; got {audit:#}"
    );
    let audits: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_citation_audits WHERE block_id = $1")
            .bind(r_block.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(audits, 0, "no audit row lands on the husk's block");

    // The retype and the reweight of R's folded edge.
    let retype = writes::retype_relationship(
        &pool,
        leak.edge,
        EdgeKind::Contains,
        EdgePolarity::Forward,
        emitter,
    )
    .await
    .expect_err("a retype of the husk's edge refuses");
    assert!(
        format!("{retype:#}").contains(&refusal(leak.resource)),
        "the retype refusal is the write guard's; got {retype:#}"
    );
    let reweight = writes::reweight_relationship(&pool, leak.edge, 0.25, emitter)
        .await
        .expect_err("a reweight of the husk's edge refuses");
    assert!(
        format!("{reweight:#}").contains(&refusal(leak.resource)),
        "the reweight refusal is the write guard's; got {reweight:#}"
    );
    let (kind, weight): (String, f64) =
        sqlx::query_as("SELECT edge_kind::text, weight FROM kb_edges WHERE id = $1")
            .bind(leak.edge)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        (kind.as_str(), weight),
        ("leads_to", 1.0),
        "the husk's edge keeps its kind and weight"
    );

    // The verdict writer shape reconcile calls.
    let verdict = sqlx::query("SELECT data_artifact_verdict_upsert($1, $2, 1, 'h', false, $3)")
        .bind(leak.artifact)
        .bind(Uuid::now_v7())
        .bind(serde_json::json!({"message": "jane was here"}))
        .execute(&pool)
        .await
        .expect_err("a verdict on the husk's artifact refuses");
    assert!(
        verdict.to_string().contains(&refusal(leak.resource)),
        "the verdict refusal is the write guard's; got {verdict}"
    );
    let verdicts: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_data_artifact_verdicts WHERE artifact_id = $1")
            .bind(leak.artifact)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(verdicts, 0, "no verdict row lands on the husk's artifact");

    // The finalize of the erased segmented ingest, with the counts and hash it would otherwise
    // accept, so the refusal is the guard's and not a mismatch.
    let (blocks, body_hash): (i64, Option<String>) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded), \
                (SELECT body_hash FROM kb_resources WHERE id = $1)",
    )
    .bind(in_flight.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let finalize = writes::finalize_ingest(
        &pool,
        writes::FinalizeParams {
            resource: in_flight,
            expected_blocks: u32::try_from(blocks).expect("a block count fits u32"),
            expected_body_hash: body_hash.expect("setup: the husk has a body hash"),
            expected_content_hash: None,
            emitter,
        },
    )
    .await
    .expect_err("the finalize of an erased ingest refuses");
    assert!(
        format!("{finalize:#}").contains(&refusal(in_flight)),
        "the finalize refusal is the write guard's; got {finalize:#}"
    );
    let ingest_state: String =
        sqlx::query_scalar("SELECT ingest_state FROM kb_resources WHERE id = $1")
            .bind(in_flight.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        ingest_state, "in_progress",
        "the husk keeps in_progress; the finalize did not land"
    );
}

/// (20, embed half) The embed drain's write-back arriving after the act writes nothing: the
/// drain's own statement (`CHUNK_EMBEDDING_WRITE_BACK`) against one of the husk's chunks
/// leaves the vector NULL. The same statement against the live twin writes, so the refusal is
/// the guard's, not a statement that never writes.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_embed_write_back_after_the_act_writes_nothing(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "embed-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "embed-twin").await,
    )
    .await;
    execute_act(&pool, leak.resource.uuid()).await;

    let vector = format!("[{}]", vec!["0.1"; 768].join(","));
    // A current chunk of a live block: the write-back also refuses a superseded chunk or a folded
    // block's (the drain's write-time currency check), so only such a chunk isolates the erasure
    // guard as the thing that refuses the husk.
    let first_chunk_sql = "SELECT c.id FROM kb_chunks c \
                           JOIN kb_content_blocks b ON b.id = c.block_id \
                           WHERE c.resource_id = $1 AND c.is_current AND NOT b.is_folded \
                           ORDER BY c.id LIMIT 1";
    let husk_chunk: Uuid = sqlx::query_scalar(first_chunk_sql)
        .bind(leak.resource.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    let twin_chunk: Uuid = sqlx::query_scalar(first_chunk_sql)
        .bind(leak.twin.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();

    let husk_write = sqlx::query(temper_substrate::embed::CHUNK_EMBEDDING_WRITE_BACK)
        .bind(vector.as_str())
        .bind("model-sha-1")
        .bind(husk_chunk)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        husk_write.rows_affected(),
        0,
        "the write-back refuses the husk"
    );
    let (has_embedding, embedded_with): (bool, Option<String>) =
        sqlx::query_as("SELECT embedding IS NOT NULL, embedded_with FROM kb_chunks WHERE id = $1")
            .bind(husk_chunk)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(!has_embedding, "the husk's chunk keeps a NULL vector");
    assert!(
        embedded_with.is_none(),
        "and no provenance stamp rides in with a vector that did not land"
    );

    let twin_write = sqlx::query(temper_substrate::embed::CHUNK_EMBEDDING_WRITE_BACK)
        .bind(vector.as_str())
        .bind("model-sha-1")
        .bind(twin_chunk)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        twin_write.rows_affected(),
        1,
        "the same statement writes a live resource's chunk"
    );
}

/// (20, embed half, the drain converges) After the act the husk's chunks are not embed work:
/// the stale predicate excludes an erased resource, so a drain job for R — one in flight across
/// the act included — reports nothing remaining and completes instead of re-enqueueing forever
/// on chunks its guarded write-backs may never stamp. Before the act the same chunks ARE stale
/// (the seed's `embedded_with` is not this build's model), so the zero is the exclusion's.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn after_the_act_the_embed_drain_finds_nothing_stale(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "drain-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "drain-twin").await,
    )
    .await;
    let resource = leak.resource.uuid();

    let stale_before = temper_substrate::embed::count_stale_chunks(&pool, resource)
        .await
        .unwrap();
    assert!(
        stale_before > 0,
        "setup: R's chunks are stale before the act (seeded under another model)"
    );

    execute_act(&pool, resource).await;

    assert_eq!(
        temper_substrate::embed::count_stale_chunks(&pool, resource)
            .await
            .unwrap(),
        0,
        "an erased resource's chunks are not embed work"
    );
    let progress = temper_substrate::embed::embed_resource_chunks(&pool, resource, 64)
        .await
        .unwrap();
    assert_eq!(progress.embedded, 0, "the drain embeds nothing on the husk");
    assert!(
        progress.is_complete(),
        "the drain reports nothing remaining, so its job completes; got {progress:?}"
    );
}

// ── Witness 22: the remote-source re-point holds (D4) ──────────────────────────────────────
//
// Every remote provenance row of R's blocks re-points to `erased:<block_id>:<n>`, n numbered
// per block in ledger order of first appearance; the re-point goes by the id the upsert
// returns; the delete considers only R's captured originals, each locked before the check.

/// A second resource in its own home, carrying no sources — the "another resource" the D4
/// witnesses cite from.
async fn other_resource(
    pool: &PgPool,
    owner: ProfileId,
    emitter: EntityId,
    slug: &str,
) -> ResourceId {
    let home = make_home(pool, owner, slug).await;
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title: "another resource",
            origin_uri: "test://another",
            body: CLEAN,
            doc_type: "research",
            home: AnchorRef::context(home),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: Some(vec![chunk(CLEAN, "")]),
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("seed another resource through the create path")
}

/// (22, collision half) Two distinct remote sources cited in ONE annotate event at ONE seq get
/// distinct sentinels: a sentinel keyed by seq would give both the same row, and the second
/// re-point would violate the provenance unique key (block_id, source_kind, source_id,
/// contributed_by_event_id).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn two_remote_sources_at_one_seq_erase_without_collision(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    register_block_provenance_annotated(&pool).await;
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "two-at-one-seq-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "two-at-one-seq-twin").await,
    )
    .await;
    let block: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded \
          ORDER BY seq LIMIT 1",
    )
    .bind(leak.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();

    const FIRST: &str = "https://first.example/cited-at-seq-1";
    const SECOND: &str = "https://second.example/cited-at-seq-1";
    writes::annotate_block_sources(
        &pool,
        writes::AnnotateParams {
            resource: leak.resource,
            sources: vec![
                Incorporation {
                    source: ProvenanceSource::Remote(FIRST.to_owned()),
                    seq: 1,
                },
                Incorporation {
                    source: ProvenanceSource::Remote(SECOND.to_owned()),
                    seq: 1,
                },
            ],
            content_block: Some(block),
            emitter,
        },
    )
    .await
    .expect("one annotate event cites both sources at seq 1");

    // The two provenance rows the annotate wrote, by row id — the act re-points them in place.
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT bp.id FROM kb_block_provenance bp \
           JOIN kb_remote_sources rs ON rs.id = bp.source_id \
          WHERE bp.block_id = $1 AND bp.source_kind = 'remote' AND rs.uri = ANY($2) \
          ORDER BY bp.id",
    )
    .bind(block)
    .bind(vec![FIRST.to_owned(), SECOND.to_owned()])
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        rows.len(),
        2,
        "setup: both sources projected a provenance row"
    );
    let row_ids: Vec<Uuid> = rows.into_iter().map(|(id,)| id).collect();

    execute_act(&pool, leak.resource.uuid()).await;

    let after: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT bp.source_id, rs.uri FROM kb_block_provenance bp \
           JOIN kb_remote_sources rs ON rs.id = bp.source_id \
          WHERE bp.id = ANY($1) ORDER BY bp.id",
    )
    .bind(&row_ids)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        after.len(),
        2,
        "both rows still resolve to a remote-source row"
    );
    assert_ne!(
        after[0].0, after[1].0,
        "the two sources point at DISTINCT sentinel rows; got {after:?}"
    );
    let prefix = format!("erased:{block}:");
    assert!(
        after.iter().all(|(_, uri)| uri.starts_with(&prefix)),
        "each row resolves to an erased:<block>:<n> sentinel; got {after:?}"
    );
    let originals_left: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_remote_sources WHERE uri = ANY($1)")
            .bind(vec![FIRST.to_owned(), SECOND.to_owned()])
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        originals_left, 0,
        "R cited both exclusively, so neither original row survives"
    );

    assert_replay_byte_identical(&pool, "of two remote sources at one seq").await;
}

/// (22, look-alike half) A row another resource minted in advance whose NORMALIZED uri equals
/// R's sentinel (`' erased:<block>:1'`, leading space; `normalize_remote_uri` trims it) is the
/// row `_upsert_remote_source` returns for the sentinel. Re-pointing by that returned id lands
/// R's provenance on it; matching on `uri` text would miss it and leave R on the original URL.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_pre_minted_look_alike_sentinel_does_not_stop_the_re_point(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    register_block_provenance_annotated(&pool).await;
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "look-alike-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "look-alike-twin").await,
    )
    .await;
    let other = other_resource(&pool, owner, emitter, "look-alike-other").await;

    // Every block of R that cites URL. Each cites URL as its only remote source, so URL is n = 1
    // on each and its sentinel is erased:<block>:1.
    let blocks: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT bp.block_id FROM kb_block_provenance bp \
           JOIN kb_content_blocks b ON b.id = bp.block_id \
           JOIN kb_remote_sources rs ON rs.id = bp.source_id \
          WHERE b.resource_id = $1 AND bp.source_kind = 'remote' AND rs.uri = $2 \
          ORDER BY bp.block_id",
    )
    .bind(leak.resource.uuid())
    .bind(URL)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(!blocks.is_empty(), "setup: R's blocks cite URL");
    let one_source_each: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT (bp.block_id, bp.source_id)) FROM kb_block_provenance bp \
          WHERE bp.block_id = ANY($1) AND bp.source_kind = 'remote'",
    )
    .bind(&blocks)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        one_source_each,
        blocks.len() as i64,
        "setup: each of those blocks cites exactly one remote source, so URL is its n = 1"
    );

    // The other resource mints a look-alike of each sentinel first.
    writes::annotate_block_sources(
        &pool,
        writes::AnnotateParams {
            resource: other,
            sources: blocks
                .iter()
                .enumerate()
                .map(|(i, b)| Incorporation {
                    source: ProvenanceSource::Remote(format!(" erased:{b}:1")),
                    seq: i as i32,
                })
                .collect(),
            content_block: None,
            emitter,
        },
    )
    .await
    .expect("another resource cites the look-alikes");
    let look_alikes: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT id, uri FROM kb_remote_sources WHERE uri LIKE ' erased:%' ORDER BY uri",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        look_alikes.len(),
        blocks.len(),
        "setup: one look-alike row per block"
    );

    execute_act(&pool, leak.resource.uuid()).await;

    let on_url: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_block_provenance bp \
           JOIN kb_content_blocks b ON b.id = bp.block_id \
           JOIN kb_remote_sources rs ON rs.id = bp.source_id \
          WHERE b.resource_id = $1 AND bp.source_kind = 'remote' \
            AND rs.uri_normalized = normalize_remote_uri($2)",
    )
    .bind(leak.resource.uuid())
    .bind(URL)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        on_url, 0,
        "none of R's provenance rows resolves to R's original URL"
    );
    for block in &blocks {
        let expected = format!(" erased:{block}:1");
        let look_alike_id = look_alikes
            .iter()
            .find(|(_, uri)| *uri == expected)
            .map(|(id, _)| *id)
            .expect("the look-alike for this block exists");
        let pointed: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT source_id FROM kb_block_provenance \
              WHERE block_id = $1 AND source_kind = 'remote'",
        )
        .bind(block)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            pointed,
            vec![look_alike_id],
            "block {block}'s rows point at the id the sentinel upsert returned — the look-alike"
        );
    }
    let url_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_remote_sources WHERE uri = $1")
        .bind(URL)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        url_rows, 0,
        "R cited URL exclusively, so its row is deleted"
    );

    // Replay matches in every table but one column of these rows (ruled 2026-10-08, a declared
    // replay limit under D14). The redacted ledger cites `erased:<block>:1` at R's original
    // citation, before the other resource minted its look-alike, so replay keeps the canonical
    // spelling where live kept the look-alike's. The row is the other resource's, so the act
    // cannot respell it (Q1).
    assert_replay_identical_but_first_writer_spelling(&pool, blocks.len(), |live, _| {
        live.starts_with(' ')
    })
    .await;
}

/// (22, concurrent-citer half, citer first) R exclusively cites URL. Another resource's annotate
/// of URL holds its transaction — its `_upsert_remote_source` holds URL's row lock — while the act
/// runs; the act locks R's captured originals before it computes the plan, so it waits there, and
/// the plan, the record and the delete decision all see the committed citer: the row is kept,
/// and the record's `targets` says it was kept as shared, not deleted. The other resource's
/// provenance stays readable: its whole-body update reads attributions (`read_attributions`),
/// which fails on a remote row with no `kb_remote_sources` uri.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_concurrent_citer_keeps_its_remote_source(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    register_block_provenance_annotated(&pool).await;
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "citer-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "citer-twin").await,
    )
    .await;
    let other = other_resource(&pool, owner, emitter, "citer-other").await;
    let url_id: Uuid = sqlx::query_scalar("SELECT id FROM kb_remote_sources WHERE uri = $1")
        .bind(URL)
        .fetch_one(&pool)
        .await
        .unwrap();

    // The citer: annotate the other resource's block with URL, NOT committed.
    let mut citer = pool.begin().await.unwrap();
    writes::annotate_block_sources_in_tx(
        &mut citer,
        writes::AnnotateParams {
            resource: other,
            sources: vec![Incorporation {
                source: ProvenanceSource::Remote(URL.to_owned()),
                seq: 0,
            }],
            content_block: None,
            emitter,
        },
        EventContext::default(),
    )
    .await
    .expect("the concurrent annotate writes inside its open transaction");

    let pool_for_act = pool.clone();
    let resource = leak.resource.uuid();
    let mut act = tokio::spawn(async move { execute_act(&pool_for_act, resource).await });
    let finished_within_window =
        tokio::time::timeout(std::time::Duration::from_secs(2), &mut act).await;
    assert!(
        finished_within_window.is_err(),
        "the act completed while a citer held URL's row — its FOR UPDATE did not wait"
    );

    citer.commit().await.unwrap();
    let event_id = act.await.expect("the act task must not panic");

    // The record matches what the act did: the source it kept is counted shared, not deleted,
    // and the remainder names it by id.
    let (targets, remainder): (serde_json::Value, serde_json::Value) = sqlx::query_as(
        "SELECT payload->'targets', payload->'remainder' FROM kb_events WHERE id = $1",
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    let source_outcomes: Vec<&str> = targets
        .as_array()
        .expect("targets is an array")
        .iter()
        .filter(|t| t["target"] == "kb_remote_sources")
        .map(|t| t["outcome"].as_str().expect("an outcome is a string"))
        .collect();
    assert_eq!(
        source_outcomes,
        vec!["0 exclusive remote sources deleted; 1 shared remote sources kept, named in the remainder"],
        "the record says the source was kept as shared, as it was; got {targets}"
    );
    assert!(
        remainder.to_string().contains(&format!(
            "shared remote source {url_id}; another resource's block still cites it; named, kept"
        )),
        "the remainder names the kept source by id; got {remainder}"
    );

    let survivor: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM kb_remote_sources WHERE id = $1")
            .bind(url_id)
            .fetch_optional(&pool)
            .await
            .unwrap();
    assert_eq!(
        survivor,
        Some(url_id),
        "URL's row survives: a citer committed while the act waited on it"
    );
    let other_uris: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT rs.uri FROM kb_block_provenance bp \
           JOIN kb_content_blocks b ON b.id = bp.block_id \
           LEFT JOIN kb_remote_sources rs ON rs.id = bp.source_id \
          WHERE b.resource_id = $1 AND bp.source_kind = 'remote'",
    )
    .bind(other.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        other_uris,
        vec![Some(URL.to_owned())],
        "the other resource's provenance resolves to URL's row"
    );
    let r_on_url: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_block_provenance bp \
           JOIN kb_content_blocks b ON b.id = bp.block_id \
          WHERE b.resource_id = $1 AND bp.source_kind = 'remote' AND bp.source_id = $2",
    )
    .bind(resource)
    .bind(url_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(r_on_url, 0, "R's own provenance was re-pointed off URL");

    writes::update_resource(
        &pool,
        UpdateParams {
            resource: other,
            body: Some(RACED),
            title: None,
            origin_uri: None,
            properties: &[],
            unset_keys: &[],
            chunks: Some(vec![chunk(RACED, "")]),
            sources: vec![],
            content_block: None,
            rehome_to: None,
            emitter,
        },
    )
    .await
    .expect("the other resource's whole-body update reads its attributions and succeeds");

    assert_replay_byte_identical(&pool, "of a citer that committed while the act waited").await;
}

/// (22, concurrent-citer half, act first) R exclusively cites URL. The act runs inside a
/// transaction that has not committed, so it holds URL's row, which it has deleted. Another
/// resource's annotate of URL arrives: its `_upsert_remote_source` waits on that row. Once the act
/// commits, the upsert finds no row and inserts URL fresh, so the citer's provenance resolves to a
/// live row with a new id — nothing dangles — and its whole-body update succeeds.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_citer_arriving_during_the_act_waits_and_cites_a_fresh_row(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    register_block_provenance_annotated(&pool).await;
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "late-citer-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "late-citer-twin").await,
    )
    .await;
    let other = other_resource(&pool, owner, emitter, "late-citer-other").await;
    let url_id: Uuid = sqlx::query_scalar("SELECT id FROM kb_remote_sources WHERE uri = $1")
        .bind(URL)
        .fetch_one(&pool)
        .await
        .unwrap();

    // The act, run inside a transaction that is NOT committed yet.
    let mut act = pool.begin().await.unwrap();
    sqlx::query("SELECT resource_erasure_execute($1,$2,$3,$4)")
        .bind(leak.resource.uuid())
        .bind(emitter)
        .bind(emitter)
        .bind(Uuid::now_v7())
        .execute(&mut *act)
        .await
        .expect("the act runs inside its open transaction");

    let pool_for_citer = pool.clone();
    let mut citer = tokio::spawn(async move {
        writes::annotate_block_sources(
            &pool_for_citer,
            writes::AnnotateParams {
                resource: other,
                sources: vec![Incorporation {
                    source: ProvenanceSource::Remote(URL.to_owned()),
                    seq: 0,
                }],
                content_block: None,
                emitter,
            },
        )
        .await
    });
    let finished_within_window =
        tokio::time::timeout(std::time::Duration::from_secs(2), &mut citer).await;
    assert!(
        finished_within_window.is_err(),
        "the citer completed while the act held URL's row — its upsert did not wait"
    );

    act.commit().await.unwrap();
    citer
        .await
        .expect("the citer task must not panic")
        .expect("the citer completes once the act commits");

    let fresh: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM kb_remote_sources WHERE uri = $1")
        .bind(URL)
        .fetch_all(&pool)
        .await
        .unwrap();
    assert_eq!(fresh.len(), 1, "URL has exactly one row after the act");
    assert_ne!(
        fresh[0], url_id,
        "the act deleted R's exclusive original; the citer's upsert inserted URL fresh"
    );
    let other_uris: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT rs.uri FROM kb_block_provenance bp \
           JOIN kb_content_blocks b ON b.id = bp.block_id \
           LEFT JOIN kb_remote_sources rs ON rs.id = bp.source_id \
          WHERE b.resource_id = $1 AND bp.source_kind = 'remote'",
    )
    .bind(other.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        other_uris,
        vec![Some(URL.to_owned())],
        "the citer's provenance resolves to the fresh URL row"
    );

    writes::update_resource(
        &pool,
        UpdateParams {
            resource: other,
            body: Some(RACED),
            title: None,
            origin_uri: None,
            properties: &[],
            unset_keys: &[],
            chunks: Some(vec![chunk(RACED, "")]),
            sources: vec![],
            content_block: None,
            rehome_to: None,
            emitter,
        },
    )
    .await
    .expect("the citer's whole-body update reads its attributions and succeeds");

    assert_replay_byte_identical(&pool, "of a citer that waited on the act").await;
}

/// (22, self-sentinel half) An author cannot make their own resource un-erasable. One annotate
/// event on R's block cites a real URL (numbered 1) and the literal `erased:<that block>:1`
/// (numbered 2). The URL's sentinel upsert returns the literal's own row, while the literal
/// moves on to `erased:<block>:2`. The act parks every captured row before placing any, so it
/// completes, and the event keeps one row per original, each on its own sentinel — the state
/// replay of the redacted payloads lands on. The literal's row stays: R still cites it.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_block_citing_its_own_sentinel_literal_still_erases(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    register_block_provenance_annotated(&pool).await;
    let (owner, emitter) = system_actor(&pool).await;
    let resource = other_resource(&pool, owner, emitter, "self-sentinel-home").await;
    let block: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded \
          ORDER BY seq LIMIT 1",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();

    const REAL: &str = "https://real.example/cited-beside-its-sentinel";
    let literal_1 = format!("erased:{block}:1");
    let literal_2 = format!("erased:{block}:2");
    writes::annotate_block_sources(
        &pool,
        writes::AnnotateParams {
            resource,
            sources: vec![
                Incorporation {
                    source: ProvenanceSource::Remote(REAL.to_owned()),
                    seq: 1,
                },
                Incorporation {
                    source: ProvenanceSource::Remote(literal_1.clone()),
                    seq: 1,
                },
            ],
            content_block: Some(block),
            emitter,
        },
    )
    .await
    .expect("one annotate event cites the URL and the block's own sentinel literal");

    let event: Uuid = sqlx::query_scalar(
        "SELECT DISTINCT bp.contributed_by_event_id FROM kb_block_provenance bp \
           JOIN kb_remote_sources rs ON rs.id = bp.source_id \
          WHERE bp.block_id = $1 AND bp.source_kind = 'remote' AND rs.uri = $2",
    )
    .bind(block)
    .bind(REAL)
    .fetch_one(&pool)
    .await
    .unwrap();
    let literal_row: Uuid = sqlx::query_scalar("SELECT id FROM kb_remote_sources WHERE uri = $1")
        .bind(&literal_1)
        .fetch_one(&pool)
        .await
        .unwrap();

    execute_act(&pool, resource.uuid()).await;

    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT bp.source_id, rs.uri FROM kb_block_provenance bp \
           JOIN kb_remote_sources rs ON rs.id = bp.source_id \
          WHERE bp.block_id = $1 AND bp.contributed_by_event_id = $2 \
            AND bp.source_kind = 'remote' \
          ORDER BY rs.uri",
    )
    .bind(block)
    .bind(event)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 2, "the event keeps two rows; got {rows:?}");
    assert_eq!(
        rows,
        vec![
            (literal_row, literal_1.clone()),
            (rows[1].0, literal_2.clone())
        ],
        "the event keeps one row per original: the URL's on the literal's own row \
         (erased:<block>:1), the literal's on erased:<block>:2"
    );
    assert_ne!(
        rows[0].0, rows[1].0,
        "the two rows sit on distinct sentinel rows"
    );
    let real_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_remote_sources WHERE uri = $1")
            .bind(REAL)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        real_rows, 0,
        "R cited the URL exclusively, so its row is deleted"
    );

    assert_replay_byte_identical(&pool, "of a block citing its own sentinel literal").await;
}

/// (24) The record attests and never repeats (D2 steps 7/7a, D8, goal §8). Beside `seed_leak`,
/// the rows the act must also reach: an ingestion record whose `source_uri` is a local path, a
/// `done` and a `dead` workflow job whose payload and `last_error` quote the leak, and an artifact
/// whose verdict `detail` quotes the value a validator rejected. After the act, every one is
/// reached (the jobs keep their status), the `resource_erased` payload's `targets` names each
/// reached table and claims none it did not reach, its top-level keys are exactly the
/// `ResourceErased` shape, and the payload text contains none of the planted strings.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_record_attests_every_target_and_repeats_nothing(pool: sqlx::PgPool) {
    const INGEST_PATH: &str = "/Users/jane/medical/leak.pdf";
    const EXCERPT: &str = "Jane: carcinoma";
    const PROP_KEY: &str = "diagnosis_code";
    const PROP_VALUE: &str = "C50.9-jane";

    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "attest-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "attest-twin").await,
    )
    .await;
    writes::set_property(
        &pool,
        leak.resource,
        PROP_KEY,
        &serde_json::json!(PROP_VALUE),
        emitter,
    )
    .await
    .unwrap();

    // The ingestion record, through its write path.
    writes::upsert_ingestion_record(
        &pool,
        writes::IngestionRecord {
            resource: leak.resource,
            source_uri: INGEST_PATH,
            source_mimetype: Some("application/pdf"),
            conversion_tool: "kreuzberg",
            conversion_version: "1",
            source_hash: Some("5eed5eed"),
        },
    )
    .await
    .unwrap();

    // Finished jobs quoting the leak. kb_workflow_jobs is a work queue, not a projection: raw
    // rows are the substrate precedent (invocation_envelope.rs `claimed_job`).
    for status in ["done", "dead"] {
        sqlx::query(
            "INSERT INTO kb_workflow_jobs (resource_id, persona, dispatch_type, status, payload, last_error) \
             VALUES ($1, 'ingest', 'embed', $2, $3, $4)",
        )
        .bind(leak.resource.uuid())
        .bind(status)
        .bind(serde_json::json!({ "excerpt": EXCERPT }))
        .bind(format!("embed failed on: {EXCERPT}"))
        .execute(&pool)
        .await
        .unwrap();
    }

    // A verdict whose detail quotes the rejected value: an advisory shape in the resource's home
    // (an enforcing one refuses the commit, so no verdict row is written), then a
    // non-conforming commit through the real path, which records the validator's messages.
    writes::declare_shape(
        &pool,
        writes::DeclareShapeParams {
            home: AnchorRef::context(home),
            kind: "diagnosis",
            kind_owner: Some(KindOwner::Profile(owner.uuid())),
            schema: &serde_json::json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": { "dx": { "type": "integer" } },
                "required": ["dx"]
            }),
            enforcement: payloads::EnforcementMode::Advisory,
            emitter,
        },
    )
    .await
    .unwrap();
    let diagnosis = writes::commit_data_artifact(
        &pool,
        CommitDataArtifactParams {
            resource: leak.resource,
            kind: "diagnosis",
            kind_owner: Some(KindOwner::Profile(owner.uuid())),
            intent: ArtifactIntent::Current,
            precedence: 0.0,
            content: &serde_json::json!({ "dx": EXCERPT }),
            supersedes: &[],
            emitter,
        },
    )
    .await
    .unwrap();
    let detail_before: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT detail FROM kb_data_artifact_verdicts WHERE artifact_id = $1")
            .bind(Uuid::from(diagnosis))
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        detail_before
            .as_ref()
            .is_some_and(|d| d.to_string().contains(EXCERPT)),
        "the witness needs a verdict whose detail quotes the value; got {detail_before:?}"
    );

    let event = execute_act(&pool, leak.resource.uuid()).await;

    // D2 step 7a: the ingestion record takes the origin_uri class sentinel, its hash kept.
    let (source_uri, source_hash): (String, Option<String>) = sqlx::query_as(
        "SELECT source_uri, source_hash FROM kb_ingestion_records WHERE resource_id = $1",
    )
    .bind(leak.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(source_uri, format!("erased:{}", leak.resource.uuid()));
    assert_eq!(source_hash.as_deref(), Some("5eed5eed"), "the hash is kept");

    // D2 step 7a: the verdict's detail is gone; the verdict itself stays.
    let detail_after: Option<serde_json::Value> =
        sqlx::query_scalar("SELECT detail FROM kb_data_artifact_verdicts WHERE artifact_id = $1")
            .bind(Uuid::from(diagnosis))
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(detail_after, None, "the verdict detail is nulled");

    // D2 step 7: every job loses its excerpts, in every status; a finished job keeps its status.
    let jobs: Vec<(String, serde_json::Value, Option<String>)> = sqlx::query_as(
        "SELECT status, payload, last_error FROM kb_workflow_jobs \
          WHERE resource_id = $1 ORDER BY status",
    )
    .bind(leak.resource.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        jobs,
        vec![
            ("dead".to_string(), serde_json::json!({}), None),
            ("done".to_string(), serde_json::json!({}), None),
        ],
        "both jobs are emptied and the done job is still done"
    );

    // D8: targets names every reached table, and claims none the act did not reach.
    let payload: serde_json::Value =
        sqlx::query_scalar("SELECT payload FROM kb_events WHERE id = $1")
            .bind(event)
            .fetch_one(&pool)
            .await
            .unwrap();
    // The record's top-level keys are exactly the ResourceErased shape.
    for key in payload
        .as_object()
        .expect("the payload is an object")
        .keys()
    {
        assert!(
            matches!(
                key.as_str(),
                "subject_table"
                    | "subject_id"
                    | "actor"
                    | "redacted_fields"
                    | "folded_edges"
                    | "targets"
                    | "remainder"
                    | "ledger_remainder"
            ),
            "unexpected resource_erased payload key {key:?}"
        );
    }
    let named: std::collections::BTreeSet<&str> = payload["targets"]
        .as_array()
        .expect("targets is an array")
        .iter()
        .map(|t| t["target"].as_str().expect("every target is named"))
        .collect();
    for reached in [
        "kb_chunk_content",
        "kb_block_content",
        "kb_chunks.embedding",
        "kb_resource_search_index",
        "kb_data_artifact_content",
        "kb_data_artifact_verdicts.detail",
        "kb_workflow_jobs",
        "kb_ingestion_records.source_uri",
        "kb_resources",
        "kb_properties",
        "kb_edges",
        "kb_block_provenance",
        "kb_remote_sources",
    ] {
        assert!(
            named.contains(reached),
            "targets names {reached}; got {named:?}"
        );
    }
    for unreached in [
        "kb_citation_audits.reason",
        "formation watermarks",
        "kb_resources.ingest_state",
    ] {
        assert!(
            !named.contains(unreached),
            "targets does not claim {unreached}, which this act reached nothing in; got {named:?}"
        );
    }

    // Goal §8: the record never repeats what it erased.
    let text = payload.to_string();
    for planted in [
        "M&A notes (leaked)",
        URL,
        "transient",
        PROP_KEY,
        PROP_VALUE,
        "jane smith spoke to us",
        INGEST_PATH,
        EXCERPT,
    ] {
        assert!(
            !text.contains(planted),
            "the resource_erased payload repeats {planted:?}: {text}"
        );
    }
}

/// (21) Replay does not depend on intra-transaction order (D14). The production-shaped ledger,
/// built directly: R with two LIVE edges to one target, one kind, one home, different labels (the
/// body's label NULL collides on `uq_kb_edges_assertion` while both are live). The act's events
/// are appended with the erasure FIRST — `resource_erased`, then one `relationship_folded` per
/// edge, all under one correlation id, in one transaction — so on local PG18 (native `uuidv7()`,
/// monotonic per backend) the erasure's id sorts below its folds: exactly the inversion
/// PG17/Neon's unordered-within-a-millisecond ids produce. Each fold projects the way
/// `resource_erasure_execute` projects it, then the body runs with the erasure's id. Replay walks
/// `ORDER BY e.id`, meets the erasure before either fold, and must still complete byte-identical.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_completes_when_the_erasure_sorts_before_its_folds(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "inverted-home").await;
    let mut resources = Vec::new();
    for (title, uri) in [("R", "test://inverted-r"), ("T", "test://inverted-t")] {
        resources.push(
            writes::create_resource_with(
                &pool,
                CreateParams {
                    idempotency_key: None,
                    title,
                    origin_uri: uri,
                    body: "body",
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
            .unwrap(),
        );
    }
    let (r, t) = (resources[0], resources[1]);
    let mut edges = Vec::new();
    for label in ["first label", "second label"] {
        edges.push(
            writes::assert_relationship(
                &pool,
                AssertParams {
                    src: r,
                    tgt: t,
                    kind: EdgeKind::LeadsTo,
                    polarity: EdgePolarity::Forward,
                    label: Some(label),
                    weight: 1.0,
                    home,
                    emitter,
                },
            )
            .await
            .unwrap()
            .uuid(),
        );
    }

    // The act's events, erasure first, one transaction (one backend), one correlation id.
    let correlation = Uuid::now_v7();
    let mut tx = pool.begin().await.unwrap();
    let erasure: Uuid = sqlx::query_scalar(
        "SELECT _event_append('resource_erased', $1, NULL, NULL, $2, \
                              p_references => $3, p_correlation => $4)",
    )
    .bind(emitter)
    .bind(serde_json::json!({
        "subject_table": "kb_resources",
        "subject_id": r.uuid(),
        "actor": emitter,
        "redacted_fields": [],
        "folded_edges": edges,
        "targets": [],
        "remainder": [],
        "ledger_remainder": [],
    }))
    .bind(serde_json::json!([
        {"rel": "subject", "target": {"kind": "kb_resources", "id": r.uuid()}},
        {"rel": "request", "target": {"kind": "kb_events", "id": correlation}},
    ]))
    .bind(correlation)
    .fetch_one(&mut *tx)
    .await
    .expect("append the erasure first");
    for edge in &edges {
        let payload = serde_json::json!({"edge_id": edge, "reason": "resource_erased"});
        let fold: Uuid = sqlx::query_scalar(
            "SELECT _event_append('relationship_folded', $1, 'kb_contexts', $2, $3, \
                                  p_correlation => $4)",
        )
        .bind(emitter)
        .bind(home)
        .bind(&payload)
        .bind(correlation)
        .fetch_one(&mut *tx)
        .await
        .expect("append the fold after the erasure");
        sqlx::query("SELECT _project_relationship_folded($1, $2)")
            .bind(fold)
            .bind(&payload)
            .execute(&mut *tx)
            .await
            .expect("project the fold as the act does");
    }
    sqlx::query("SELECT _resource_erasure_apply_redaction($1, $2)")
        .bind(r.uuid())
        .bind(erasure)
        .execute(&mut *tx)
        .await
        .expect("the body runs with both edges folded");
    tx.commit().await.unwrap();

    // The precondition that makes this witness bite: the erasure sorts BELOW both of its folds,
    // so the walk meets it first.
    let folds_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'relationship_folded' AND e.correlation_id = $1 AND e.id > $2",
    )
    .bind(correlation)
    .bind(erasure)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        folds_after, 2,
        "both folds must sort after the erasure, or this ledger is not the inverted one"
    );
    let ended: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_edges WHERE id = ANY($1) AND is_folded AND label IS NULL",
    )
    .bind(&edges)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(ended, 2, "both edges end folded with their label gone");

    assert_replay_byte_identical(&pool, "with the erasure sorted before its folds").await;
}

/// (21, the span's bound) The replay deferral is bounded to the act's own transaction (D14). R is
/// erased under request reference X; then, in later transactions, the husk is soft-deleted through
/// the real delete path (a lawful write — `_project_resource_deleted` is unguarded, it writes no
/// content — that stamps `updated` with its own `occurred_at`), and a retried request is refused as
/// already erased under the SAME X. The refusal shares X but not the act's transaction, so it is
/// not part of the act's span: replay applies the body at the act, then the delete, and the
/// replayed husk's `updated` is the delete's, as live. A deferral unbounded by the transaction runs
/// the body after the refusal and rewinds `updated` to the act's `occurred_at`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_defers_only_within_the_acts_own_transaction(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "span-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "span-twin").await,
    )
    .await;

    let request_ref = Uuid::now_v7();
    let erasure = execute_act_with_ref(&pool, leak.resource.uuid(), request_ref).await;
    // Separate milliseconds, so each later transaction's event ids sort after the one before.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;

    // A lawful write after the act that the body would undo if it ran again: the soft delete of
    // the husk stamps `updated`.
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::ResourceDelete {
            resource: leak.resource,
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let deleted: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'resource_deleted' AND (e.payload->>'resource_id')::uuid = $1",
    )
    .bind(leak.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;

    // A retried request under the same reference, refused as already erased.
    let refused: Uuid =
        sqlx::query_scalar("SELECT resource_erasure_refuse($1, $2, $3, $4, 'already_erased')")
            .bind(leak.resource.uuid())
            .bind(owner.uuid())
            .bind(emitter)
            .bind(request_ref)
            .fetch_one(&pool)
            .await
            .expect("the refusal records");

    // Preconditions that make this witness bite: the refusal carries X and sorts after the
    // delete, which sorts after every event of the act's own transaction; the live husk's
    // `updated` is the delete's, not the act's.
    let (span_max, refused_corr, act_at, refused_at): (
        Uuid,
        Uuid,
        chrono::DateTime<chrono::Utc>,
        chrono::DateTime<chrono::Utc>,
    ) = sqlx::query_as(
        "SELECT (SELECT max(s.id::text)::uuid FROM kb_events s \
                  WHERE s.correlation_id = $1 AND s.occurred_at = er.occurred_at), \
                rf.correlation_id, er.occurred_at, rf.occurred_at \
           FROM kb_events er, kb_events rf WHERE er.id = $2 AND rf.id = $3",
    )
    .bind(request_ref)
    .bind(erasure)
    .bind(refused)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        refused_corr, request_ref,
        "the refusal shares the act's request reference"
    );
    assert_ne!(
        act_at, refused_at,
        "the refusal is not in the act's transaction"
    );
    assert!(
        span_max < deleted && deleted < refused,
        "walk order is: the act's span, the delete, the refusal"
    );
    let husk_updated_sql = "SELECT r.updated = d.occurred_at FROM kb_resources r, kb_events d \
                             WHERE r.id = $1 AND d.id = $2";
    let live_is_the_delete: bool = sqlx::query_scalar(husk_updated_sql)
        .bind(leak.resource.uuid())
        .bind(deleted)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        live_is_the_delete,
        "setup: the live husk's updated is the post-act delete's occurred_at"
    );

    // kb_resources is in the dumps, so the byte-identity diff compares `updated` too.
    assert_replay_byte_identical(&pool, "with the request reference reused after the act").await;
    let replayed_is_the_delete: bool = sqlx::query_scalar(husk_updated_sql)
        .bind(leak.resource.uuid())
        .bind(deleted)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        replayed_is_the_delete,
        "replay leaves the husk's updated where the lawful delete after the act stamped it"
    );
}

/// (21, the replay bypass) A lawful write can sort after the act in walk order: its transaction
/// committed before the act's, but event ids are uuidv7, unordered across transactions within a
/// millisecond on PG17. The ledger's walk-order shape is built here by appending a `property_set`
/// on R through the real write path AFTER the act commits, inside a transaction that sets
/// `temper.replaying` — the setting the replay walk sets around each event — so the live write
/// guard lets it through as the walk's would. The witness is that replay projects that event
/// where it sorts instead of aborting on the write guard, and comes back byte-identical. The
/// setting is transaction-local: the same connection no longer carries it after the commit.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_projects_a_lawful_write_that_sorts_after_the_act(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "bypass-home").await;
    let resource = writes::create_resource_with(
        &pool,
        CreateParams {
            idempotency_key: None,
            title: "sorted-after",
            origin_uri: "test://sorted-after",
            body: "a body",
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
    .unwrap();
    let erasure = execute_act(&pool, resource.uuid()).await;
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;

    let mut conn = pool.acquire().await.unwrap();
    let mut tx = sqlx::Connection::begin(&mut *conn).await.unwrap();
    sqlx::query("SELECT set_config('temper.replaying', 'on', true)")
        .execute(&mut *tx)
        .await
        .unwrap();
    writes::set_property_in_tx(
        &mut tx,
        resource,
        "late_note",
        &serde_json::json!("written in a transaction that committed first"),
        emitter,
        EventContext::default(),
    )
    .await
    .expect("under the replay setting the guard lets the write through");
    tx.commit().await.unwrap();
    let flag_after: Option<String> =
        sqlx::query_scalar("SELECT current_setting('temper.replaying', true)")
            .fetch_one(&mut *conn)
            .await
            .unwrap();
    assert_ne!(
        flag_after.as_deref(),
        Some("on"),
        "the setting ended with its transaction; the pooled connection does not carry it"
    );
    drop(conn);

    let late_event: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'property_set' AND e.payload->>'property_key' = 'late_note'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        late_event > erasure,
        "precondition: the write sorts after the act in walk order"
    );

    assert_replay_byte_identical(&pool, "with a lawful write sorted after the act").await;
    let late_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties \
          WHERE owner_table = 'kb_resources' AND owner_id = $1 \
            AND property_key = 'late_note' AND NOT is_folded",
    )
    .bind(resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        late_rows, 1,
        "the walk projected the write at its position after the act"
    );
}

/// A husk as cut 1 left it, in the cut-1 act's own order and transaction: the plan, one
/// `relationship_folded` per live edge under the act's correlation id, a `resource_erased` that
/// names no `redacted_fields` and records every ledger path it left in `ledger_remainder`, then the
/// projection-side body. The ledger's text is untouched. Cut 2's act no longer makes one, but every
/// resource erased before it shipped is one until the completion pass reaches it.
async fn simulate_cut1_husk(pool: &PgPool, resource: Uuid) -> Uuid {
    let (operator, emitter) = system_actor(pool).await;
    let request_ref = Uuid::now_v7();
    let mut tx = pool.begin().await.unwrap();
    let plan: serde_json::Value = sqlx::query_scalar("SELECT resource_erasure_survey_plan($1)")
        .bind(resource)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    for edge in plan["edges"].as_array().unwrap() {
        let edge: Uuid = edge.as_str().unwrap().parse().unwrap();
        sqlx::query(
            "SELECT _project_relationship_folded( \
                 _event_append('relationship_folded', $2, e.home_anchor_table, e.home_anchor_id, \
                               jsonb_build_object('edge_id', e.id, 'reason', 'resource_erased'), \
                               p_correlation => $3), \
                 jsonb_build_object('edge_id', e.id, 'reason', 'resource_erased')) \
               FROM kb_edges e WHERE e.id = $1",
        )
        .bind(edge)
        .bind(emitter.uuid())
        .bind(request_ref)
        .execute(&mut *tx)
        .await
        .unwrap();
    }
    let erased: Uuid = sqlx::query_scalar(
        "SELECT _event_append('resource_erased', $2, NULL, NULL, jsonb_build_object( \
             'subject_table', 'kb_resources', 'subject_id', $1, 'actor', $3, \
             'folded_edges', $4->'edges', 'targets', $4->'targets', 'remainder', $4->'remainder', \
             'ledger_remainder', ($4->'redacted_fields') || ($4->'ledger_remainder')), \
             p_correlation => $5)",
    )
    .bind(resource)
    .bind(emitter.uuid())
    .bind(operator.uuid())
    .bind(&plan)
    .bind(request_ref)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query("SELECT _resource_erasure_apply_redaction($1, $2)")
        .bind(resource)
        .bind(erased)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    erased
}

/// (25) The facet regrain cannot re-inflate a husk. `_facet_regrain_from_events` deletes an owner's
/// `facet` rows and re-projects them from the ledger. Cut 2's act redacts the ledger's facet
/// payloads (Witness 28), but a husk erased under cut 1 keeps them in plain text until its
/// completion pass, so this witness runs on one, and the write guard both property projectors call
/// first is what stops the regrain: one that reaches R, an edge into R, or every owner raises and
/// rolls back whole. The control at the end runs the same regrain under the replay walk's guard
/// bypass and watches the planted value come back, so the absence checks above it can see a
/// re-inflation.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_facet_regrain_cannot_reinflate_an_erased_resource(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "regrain-home").await;
    let create = |title: &'static str, origin_uri: &'static str| {
        let pool = pool.clone();
        async move {
            writes::create_resource_with(
                &pool,
                CreateParams {
                    idempotency_key: None,
                    title,
                    origin_uri,
                    body: "regrain body",
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
    };
    let r = create("the erased one", "test://regrain-r").await;
    let s = create("the survivor", "test://regrain-s").await;

    // A facet R owns, and a facet on a live edge S→R.
    writes::set_facet(
        &pool,
        PropertyOwner::resource(r),
        &serde_json::json!({"owner": "jane smith"}),
        1.0,
        emitter,
    )
    .await
    .unwrap();
    let edge = writes::assert_relationship(
        &pool,
        AssertParams {
            src: s,
            tgt: r,
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: None,
            weight: 1.0,
            home,
            emitter,
        },
    )
    .await
    .unwrap();
    writes::set_facet(
        &pool,
        PropertyOwner::edge(edge),
        &serde_json::json!({"diagnosis": "stage 3"}),
        1.0,
        emitter,
    )
    .await
    .unwrap();

    // A planted value on a row, or an original facet key on R's or the edge's rows.
    let leaked_sql = "SELECT count(*) FROM kb_properties \
          WHERE property_value::text LIKE ANY (ARRAY['%jane smith%', '%stage 3%']) \
             OR (property_key = 'facet' AND owner_id = ANY($1))";
    let leaked = |pool: PgPool| async move {
        sqlx::query_scalar::<_, i64>(leaked_sql)
            .bind(vec![r.uuid(), edge.uuid()])
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    assert_eq!(
        leaked(pool.clone()).await,
        2,
        "setup: one facet row on R, one on the edge"
    );

    simulate_cut1_husk(&pool, r.uuid()).await;
    assert_eq!(
        leaked(pool.clone()).await,
        0,
        "the act sentineled both facets"
    );

    // The premise: a cut-1 husk's ledger still holds both original facet payloads.
    let ledger: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events \
          WHERE payload->>'property_key' = 'facet' \
            AND (payload#>>'{owner,id}')::uuid = ANY($1) \
            AND payload::text LIKE ANY (ARRAY['%jane smith%', '%stage 3%'])",
    )
    .bind(vec![r.uuid(), edge.uuid()])
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        ledger, 2,
        "premise: cut 1 leaves both facet payloads in the ledger"
    );

    let props_sql = "SELECT id, owner_id, property_key, property_value, is_folded \
          FROM kb_properties ORDER BY id";
    type PropRow = (Uuid, Uuid, String, serde_json::Value, bool);
    let props_before: Vec<PropRow> = sqlx::query_as(props_sql).fetch_all(&pool).await.unwrap();

    for (scope, owner_arg) in [
        ("the erased resource", Some(r.uuid())),
        ("an edge into it", Some(edge.uuid())),
        ("every owner", None),
    ] {
        let err = sqlx::query("SELECT * FROM _facet_regrain_from_events($1)")
            .bind(owner_arg)
            .execute(&pool)
            .await
            .expect_err(&format!("a regrain over {scope} must refuse"));
        assert!(
            err.to_string().contains(&format!(
                "resource {} is erased; writes are refused",
                r.uuid()
            )),
            "a regrain over {scope} raises on R's write guard; got {err}"
        );
    }
    let props_after: Vec<PropRow> = sqlx::query_as(props_sql).fetch_all(&pool).await.unwrap();
    assert_eq!(
        props_after, props_before,
        "the refused regrains changed no kb_properties row"
    );
    assert_eq!(
        leaked(pool.clone()).await,
        0,
        "no original facet key or value is back"
    );

    // Control: the same regrain past the guard re-projects R's original value from the ledger.
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('temper.replaying', 'on', true)")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SELECT * FROM _facet_regrain_from_events($1)")
        .bind(r.uuid())
        .execute(&mut *tx)
        .await
        .expect("past the guard, the regrain runs");
    let reinflated: i64 = sqlx::query_scalar(leaked_sql)
        .bind(vec![r.uuid(), edge.uuid()])
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert!(
        reinflated > 0,
        "control: without the guard the regrain re-projects R's facet, so the checks can see it"
    );
    tx.rollback().await.unwrap();
}

/// (Task 01a0fedb item 2) Properties owned by R's blocks go with R: every block-owned row ends
/// folded with an `erased-key-<n>` key and the `"erased"` value, numbered per block (two blocks
/// each get `erased-key-1`), the survey's `kb_properties` target counts them, and replay
/// reproduces the result. The roles are written through the real append path, so each row has an
/// event replay can walk.
///
/// FAILS IF: the act leaves a block-owned row its key or value, or a live one; or the survey does
/// not count it; or replay diverges.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_blocks_own_properties_are_erased_with_it(pool: sqlx::PgPool) {
    const ROLE: &str = "jane-smith-intake-notes";

    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        make_home(&pool, owner, "block-props-home").await,
        make_home(&pool, owner, "block-props-twin").await,
    )
    .await;
    let mut block_ids = Vec::new();
    for (seq, prose) in [
        (50, "a second block with a role"),
        (51, "a third block with a role"),
    ] {
        let block = temper_substrate::content::prepare_block_from_chunks(
            seq,
            Some(ROLE),
            vec![chunk(prose, "")],
        );
        block_ids.push(
            writes::append_block(
                &pool,
                writes::AppendParams {
                    resource: leak.resource,
                    block: &block,
                    sources: vec![],
                    emitter,
                },
            )
            .await
            .expect("append a block carrying a role"),
        );
    }
    let block_id = block_ids[0];

    let before: Vec<(String, serde_json::Value)> = sqlx::query_as(
        "SELECT property_key, property_value FROM kb_properties \
          WHERE owner_table = 'kb_content_blocks' AND owner_id = $1 AND NOT is_folded",
    )
    .bind(block_id.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        before,
        vec![("block_role".to_string(), serde_json::json!(ROLE))],
        "the append path writes the block's role as a block-owned property"
    );

    let plan: serde_json::Value = sqlx::query_scalar("SELECT resource_erasure_survey_plan($1)")
        .bind(leak.resource.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    let props_target = plan["targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["target"] == "kb_properties")
        .expect("the survey claims kb_properties");
    assert!(
        props_target["outcome"]
            .as_str()
            .unwrap()
            .contains(" and 2 block-owned rows"),
        "the survey counts the block-owned row: {props_target}"
    );

    execute_act(&pool, leak.resource.uuid()).await;

    let after: Vec<(String, serde_json::Value, bool)> = sqlx::query_as(
        "SELECT p.property_key, p.property_value, p.is_folded FROM kb_properties p \
           JOIN kb_content_blocks b ON b.id = p.owner_id \
          WHERE p.owner_table = 'kb_content_blocks' AND b.resource_id = $1",
    )
    .bind(leak.resource.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        after,
        vec![
            (
                "erased-key-1".to_string(),
                serde_json::json!("erased"),
                true
            ),
            (
                "erased-key-1".to_string(),
                serde_json::json!("erased"),
                true
            ),
        ],
        "every block-owned row of R is sentineled and folded, numbered within its own block"
    );

    assert_replay_byte_identical(
        &pool,
        "after erasing a resource whose block owns a property",
    )
    .await;
}

/// The remainder names a shape in R's home for a family R's artifacts used, by shape id (D4,
/// ruled 2026-10-03, ruling 5), and the trail scope reaches a property event owned by one of R's
/// blocks (20261008100000), so the act redacts its key and value on the ledger (cut 2).
///
/// No write path emits a block-owned property event today (`block_role` rows come from the block
/// projector), so the event here is appended directly, as a future writer would. The arm matches
/// on (home, kind owner, family): a shape for a family R never used, the same family in another
/// home, and the same family under another kind owner are not named. The act's own record names
/// the shape. Since cut 2 the act also redacts the family on the ledger, so a plan recomputed on
/// the husk finds no family left to match: the record is where the shape stays named. (On a husk
/// erased under cut 1 the ledger still carries the family until its completion pass, and the arm
/// reads it there.)
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_remainder_names_the_homes_shapes_and_block_owned_property_events(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "shape-remainder-home").await;
    let twin_home = make_home(&pool, owner, "shape-remainder-twin").await;
    let leak = seed_leak(&pool, owner, emitter, home, twin_home).await;

    let declare = |home: ContextId, kind: &'static str, kind_owner: KindOwner| {
        let pool = pool.clone();
        async move {
            writes::declare_shape(
                &pool,
                writes::DeclareShapeParams {
                    home: AnchorRef::context(home),
                    kind,
                    kind_owner: Some(kind_owner),
                    schema: &serde_json::json!({"type": "object"}),
                    enforcement: payloads::EnforcementMode::Advisory,
                    emitter,
                },
            )
            .await
            .unwrap()
            .uuid()
        }
    };
    // seed_leak's artifact is family "notes" under the owner's profile, homed in `home`.
    let mine = KindOwner::Profile(owner.uuid());
    let notes_shape = declare(home, "notes", mine).await;
    let unrelated_shape = declare(home, "unrelated", mine).await;
    let twin_home_shape = declare(twin_home, "notes", mine).await;
    let other_owner_shape = declare(home, "notes", KindOwner::Team(Uuid::now_v7())).await;
    let not_named = [unrelated_shape, twin_home_shape, other_owner_shape];

    let block: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 ORDER BY seq LIMIT 1",
    )
    .bind(leak.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let block_property_event: Uuid = sqlx::query_scalar(
        "SELECT _event_append('property_set', $1, 'kb_contexts', $3,
                jsonb_build_object('property_id', gen_random_uuid(),
                                   'owner', jsonb_build_object('table', 'kb_content_blocks', 'id', $2),
                                   'property_key', 'intake_note', 'value', 'jane smith', 'weight', 1.0))",
    )
    .bind(emitter.uuid())
    .bind(block)
    .bind(home.uuid())
    .fetch_one(&pool)
    .await
    .expect("append a block-owned property event");

    let plan_now = || {
        let pool = pool.clone();
        let resource = leak.resource.uuid();
        async move {
            sqlx::query_scalar::<_, serde_json::Value>("SELECT resource_erasure_survey_plan($1)")
                .bind(resource)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    let assert_names_only_the_notes_shape = |remainder: &serde_json::Value, when: &str| {
        let shapes: Vec<String> = remainder
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["target"] == "kb_data_artifact_shapes")
            .map(|e| e["outcome"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(
            shapes.len(),
            1,
            "{when}: exactly the shape for a family R used is named; got {shapes:?}"
        );
        assert!(
            shapes[0].contains(&notes_shape.to_string()) && !shapes[0].contains("notes"),
            "{when}: named by shape id, never by family: {shapes:?}"
        );
        for other in not_named {
            assert!(
                !remainder.to_string().contains(&other.to_string()),
                "{when}: shape {other} is not named"
            );
        }
    };
    let plan = plan_now().await;
    assert_names_only_the_notes_shape(&plan["remainder"], "before the act");

    let entry = plan["redacted_fields"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["event"] == serde_json::json!(block_property_event))
        .unwrap_or_else(|| panic!("the block-owned property event is named: {plan}"));
    for path in ["property_key", "value"] {
        assert!(
            entry["paths"].as_array().unwrap().iter().any(|p| p == path),
            "named with {path}: {entry}"
        );
    }

    let erased = execute_act(&pool, leak.resource.uuid()).await;
    let record: serde_json::Value =
        sqlx::query_scalar("SELECT payload FROM kb_events WHERE id = $1")
            .bind(erased)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_names_only_the_notes_shape(&record["remainder"], "the act's record");
    // Step (9f) is claimed among the targets: the record says the families were rewritten.
    assert!(
        record["targets"].as_array().unwrap().iter().any(|t| {
            t["target"] == "kb_data_artifacts.artifact_kind"
                && t["outcome"]
                    .as_str()
                    .unwrap()
                    .ends_with("artifact families set to erased:<asserted_by_event_id>")
        }),
        "the record claims step (9f): {}",
        record["targets"]
    );
    let husk_plan = plan_now().await;
    assert!(
        !husk_plan["remainder"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["target"] == "kb_data_artifact_shapes"),
        "on the husk, the family is gone from the row and the ledger alike: {}",
        husk_plan["remainder"]
    );
}

// ── Cut 2: the ledger exception (spec D3, D4; migration 20261009100000) ─────────────────────────

/// One event's payload.
async fn payload_of(pool: &PgPool, event: Uuid) -> serde_json::Value {
    sqlx::query_scalar("SELECT payload FROM kb_events WHERE id = $1")
        .bind(event)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// The `resource_created` event of `resource`.
async fn created_event(pool: &PgPool, resource: Uuid) -> Uuid {
    sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'resource_created' AND (e.payload->>'resource_id')::uuid = $1",
    )
    .bind(resource)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Run `sql` (one statement or several) in a transaction that is rolled back, and return its
/// error text, or None.
async fn refused(pool: &PgPool, sql: &str) -> Option<String> {
    let mut tx = pool.begin().await.unwrap();
    let out = sqlx::raw_sql(sql)
        .execute(&mut *tx)
        .await
        .err()
        .map(|e| e.to_string());
    tx.rollback().await.unwrap();
    out
}

/// (5) The verifier admits only an erasure's own rewrite. DELETE raises. So does an UPDATE of a
/// column other than payload and metadata, of a path no redaction row names, of a named path to a
/// value that is not its sentinel, and of an event the erasure's own record does not list, even
/// when a row for it was inserted by hand. Another resource's event is refused at the projector
/// and at the trigger. Every refusal says only what the trigger has always said.
///
/// FAILS IF: any of the five conditions is dropped from `kb_events_append_only`, or the projector
/// stops checking the subject's trail.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_ledger_admits_only_an_erasures_own_sentinel_rewrite(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "verifier-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "verifier-twin").await,
    )
    .await;
    let other = other_resource(&pool, owner, emitter, "verifier-other").await;
    let erased = execute_act(&pool, leak.resource.uuid()).await;
    let r_created = created_event(&pool, leak.resource.uuid()).await;
    let s_created = created_event(&pool, other.uuid()).await;
    let fold: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'relationship_folded' AND e.payload->>'edge_id' = $1",
    )
    .bind(leak.edge.uuid().to_string())
    .fetch_one(&pool)
    .await
    .unwrap();

    let append_only = |what: &str, err: Option<String>| {
        let err = err.unwrap_or_else(|| panic!("{what} was admitted"));
        assert!(
            err.contains("event ledger is append-only"),
            "{what}: refused with the trigger's own message; got {err}"
        );
    };
    append_only(
        "a DELETE",
        refused(
            &pool,
            &format!("DELETE FROM kb_events WHERE id = '{r_created}'"),
        )
        .await,
    );
    append_only(
        "an UPDATE of a column other than payload and metadata",
        refused(
            &pool,
            &format!(
                "UPDATE kb_events SET occurred_at = occurred_at + interval '1 second' \
                  WHERE id = '{r_created}'"
            ),
        )
        .await,
    );
    append_only(
        "an UPDATE of a path no redaction row names",
        refused(
            &pool,
            &format!(
                "UPDATE kb_events SET payload = jsonb_set(payload, '{{segmented}}', 'true') \
                  WHERE id = '{r_created}'"
            ),
        )
        .await,
    );
    append_only(
        "an UPDATE of a named path to a value that is not its sentinel",
        refused(
            &pool,
            &format!(
                "UPDATE kb_events SET payload = jsonb_set(payload, '{{title}}', '\"rewritten\"') \
                  WHERE id = '{r_created}'"
            ),
        )
        .await,
    );
    append_only(
        "another resource's event, to its own sentinel, with no row",
        refused(
            &pool,
            &format!(
                "UPDATE kb_events SET payload = jsonb_set(payload, '{{title}}', \
                         to_jsonb('erased-' || (payload->>'resource_id'))) \
                  WHERE id = '{s_created}'"
            ),
        )
        .await,
    );
    append_only(
        "another resource's event, under a row inserted by hand",
        refused(
            &pool,
            &format!(
                "INSERT INTO kb_event_field_redactions VALUES ('{s_created}', 'title', '{erased}'); \
                 UPDATE kb_events SET payload = jsonb_set(payload, '{{title}}', \
                         to_jsonb('erased-' || (payload->>'resource_id'))) \
                  WHERE id = '{s_created}'"
            ),
        )
        .await,
    );
    append_only(
        "R's own event, at a path the erasure's record does not list, under a row inserted by hand",
        refused(
            &pool,
            &format!(
                "INSERT INTO kb_event_field_redactions VALUES ('{fold}', 'reason', '{erased}'); \
                 UPDATE kb_events SET payload = jsonb_set(payload, '{{reason}}', 'null') \
                  WHERE id = '{fold}'"
            ),
        )
        .await,
    );
    // A resource_erased appended by hand, naming R as its subject and S's event in its record,
    // with the row inserted past the projector: the record check passes, the trail check is what
    // refuses (the exception never crosses the resource boundary, Q1).
    append_only(
        "another resource's event, under a forged erasure record that lists it",
        refused(
            &pool,
            &format!(
                "WITH forged AS (SELECT _event_append('resource_erased', '{emitter}', NULL, NULL,                      jsonb_build_object('subject_table', 'kb_resources', 'subject_id', '{r}',                          'redacted_fields', jsonb_build_array(jsonb_build_object(                              'event', '{s_created}', 'paths', jsonb_build_array('title'))))) AS id)                  INSERT INTO kb_event_field_redactions SELECT '{s_created}', 'title', id FROM forged;                  UPDATE kb_events SET payload = jsonb_set(payload, '{{title}}',                          to_jsonb('erased-' || (payload->>'resource_id')))                   WHERE id = '{s_created}'",
                emitter = emitter.uuid(),
                r = leak.resource.uuid(),
            ),
        )
        .await,
    );
    let crossing = refused(
        &pool,
        &format!(
            "SELECT _project_resource_erased_redactions('{erased}', jsonb_build_object( \
                 'subject_id', '{}', \
                 'redacted_fields', jsonb_build_array(jsonb_build_object( \
                     'event', '{s_created}', 'paths', jsonb_build_array('title')))))",
            leak.resource.uuid()
        ),
    )
    .await
    .expect("the projector refuses another resource's event");
    assert!(
        crossing.contains("not in its subject's trail"),
        "refused for the trail; got {crossing}"
    );
    append_only(
        "a DELETE of a redaction row",
        refused(
            &pool,
            &format!("DELETE FROM kb_event_field_redactions WHERE event_id = '{r_created}'"),
        )
        .await
        .map(|e| e.replace("kb_event_field_redactions is", "event ledger is")),
    );

    // After the act's transaction, an authorization is spent: a pattern-valid renumbering of a
    // sentinel the act wrote is refused (found by both reviews, 2026-10-08).
    let r_set: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'property_set' AND (e.payload#>>'{owner,id}')::uuid = $1 \
            AND e.payload->>'property_key' LIKE 'erased-key-%' ORDER BY e.id LIMIT 1",
    )
    .bind(leak.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    append_only(
        "a later renumbering of a property-key sentinel",
        refused(
            &pool,
            &format!(
                "UPDATE kb_events SET payload = jsonb_set(payload, '{{property_key}}', '\"erased-key-99\"') \
                  WHERE id = '{r_set}'"
            ),
        )
        .await,
    );
    append_only(
        "a later renumbering of a remote-source sentinel",
        refused(
            &pool,
            &format!(
                "UPDATE kb_events SET payload = jsonb_set(payload, '{{blocks,0,incorporated,0,source,value}}', \
                         to_jsonb('erased:' || (payload#>>'{{blocks,0,block_id}}') || ':7')) \
                  WHERE id = '{r_created}'"
            ),
        )
        .await,
    );
    append_only(
        "an element added under a wildcard path",
        refused(
            &pool,
            &format!(
                "UPDATE kb_events SET payload = jsonb_set(payload, '{{blocks}}', \
                         (payload->'blocks') || (payload->'blocks'->0)) \
                  WHERE id = '{r_created}'"
            ),
        )
        .await,
    );
    append_only(
        "a metadata key added",
        refused(
            &pool,
            &format!(
                "UPDATE kb_events SET metadata = metadata || '{{\"reasoning\": \"erased\"}}' \
                  WHERE id = '{r_created}'"
            ),
        )
        .await,
    );
    // A forged erasure of a LIVE resource, record, row and rewrite all in one transaction: the
    // subject is not erased, so nothing it authorizes lands.
    append_only(
        "a forged erasure of a live resource, in its own transaction",
        refused(
            &pool,
            &format!(
                "WITH forged AS (SELECT _event_append('resource_erased', '{emitter}', NULL, NULL, \
                     jsonb_build_object('subject_table', 'kb_resources', 'subject_id', '{s}', \
                         'redacted_fields', jsonb_build_array(jsonb_build_object( \
                             'event', '{s_created}', 'paths', jsonb_build_array('title'))))) AS id) \
                 INSERT INTO kb_event_field_redactions SELECT '{s_created}', 'title', id FROM forged; \
                 UPDATE kb_events SET payload = jsonb_set(payload, '{{title}}', \
                         to_jsonb('erased-' || (payload->>'resource_id'))) \
                  WHERE id = '{s_created}'",
                emitter = emitter.uuid(),
                s = other.uuid(),
            ),
        )
        .await,
    );

    // Control: inside an act's own transaction, a pattern-valid value at a path its record lists
    // IS admitted, so the refusals of the same change above are the transaction rule's. The act
    // runs inside a savepoint here: its rows then carry a subtransaction's id, and the rule must
    // still hold (it reads the transaction's start time, not xmin).
    let s2 = other_resource(&pool, owner, emitter, "verifier-control").await;
    writes::set_property(&pool, s2, "colour", &serde_json::json!("blue"), emitter)
        .await
        .unwrap();
    let s2_set: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e WHERE e.payload->>'property_key' = 'colour' \
            AND (e.payload#>>'{owner,id}')::uuid = $1",
    )
    .bind(s2.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    let (_, operator_entity) = system_actor(&pool).await;
    assert_eq!(
        refused(
            &pool,
            &format!(
                "SAVEPOINT inner_act; \
                 SELECT resource_erasure_execute('{s2}', '{operator_entity}', '{operator_entity}', '{}'); \
                 RELEASE SAVEPOINT inner_act; \
                 UPDATE kb_events SET payload = jsonb_set(payload, '{{property_key}}', '\"erased-key-9\"') \
                  WHERE id = '{s2_set}'",
                Uuid::now_v7(),
                s2 = s2.uuid(),
                operator_entity = operator_entity.uuid(),
            ),
        )
        .await,
        None,
        "inside the act's transaction, a pattern-valid key at a listed path is admitted"
    );

    // Control: at a path the record lists, the sentinel already there is admitted, so each
    // refusal above is its condition's, not a blanket one.
    assert_eq!(
        refused(
            &pool,
            &format!(
                "UPDATE kb_events SET payload = jsonb_set(payload, '{{title}}', payload->'title') \
                  WHERE id = '{r_created}'"
            ),
        )
        .await,
        None,
        "a rewrite to the sentinel at a listed path is admitted"
    );
}

/// (25) The ledger agrees with the projection on keys. Every non-literal property event of R and
/// of its edges carries, as `property_key`, the projection key of the row it asserted; an unset
/// carries its sibling assertion's key; a key only ever unset is numbered above the asserted
/// range. Replay cannot see a wrong `n` (step 9 re-ranks), which is why this witness exists.
///
/// FAILS IF: the payload derivation numbers a key differently from `_resource_erasure_key_numbers`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_ledgers_property_keys_are_the_projections(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "keys-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "keys-twin").await,
    )
    .await;
    let r = leak.resource.uuid();
    writes::set_property(
        &pool,
        leak.resource,
        "colour",
        &serde_json::json!("blue"),
        emitter,
    )
    .await
    .unwrap();
    writes::assert_keyed_property_with(
        &pool,
        PropertyOwner::edge(leak.edge),
        "evidence",
        &serde_json::json!("jane said so"),
        1.0,
        emitter,
        EventContext::default(),
    )
    .await
    .unwrap();
    writes::update_resource(
        &pool,
        UpdateParams {
            resource: leak.resource,
            body: None,
            title: None,
            origin_uri: None,
            properties: &[],
            unset_keys: &["transient".to_string(), "never-set".to_string()],
            chunks: None,
            sources: vec![],
            content_block: None,
            rehome_to: None,
            emitter,
        },
    )
    .await
    .unwrap();

    // The original key of each property event, read before the act rewrites it.
    let originals: Vec<(Uuid, String, String, Uuid, String)> = sqlx::query_as(
        "SELECT e.id, t.name, e.payload#>>'{owner,table}', (e.payload#>>'{owner,id}')::uuid, \
                e.payload->>'property_key' \
           FROM _resource_erasure_trail_scope($1) s \
           JOIN kb_events e ON e.id = s.event_id JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name IN ('property_set', 'property_asserted', 'property_unset') \
            AND e.payload->>'property_key' NOT IN ('doc_type', 'facet')",
    )
    .bind(r)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        originals.iter().any(|o| o.4 == "never-set") && originals.iter().any(|o| o.4 == "evidence"),
        "the witness needs an unset-only key and an edge-owned key: {originals:?}"
    );

    execute_act(&pool, r).await;

    let mut by_original: std::collections::BTreeMap<(Uuid, String), String> = Default::default();
    for (event, kind, owner_table, owner_id, original) in &originals {
        let key = payload_of(&pool, *event).await["property_key"]
            .as_str()
            .unwrap()
            .to_owned();
        assert!(
            key.starts_with("erased-key-"),
            "{kind} {event} keeps {key:?}"
        );
        if kind != "property_unset" {
            let row_key: String = sqlx::query_scalar(
                "SELECT property_key FROM kb_properties \
                  WHERE asserted_by_event_id = $1 AND owner_table = $2 AND owner_id = $3",
            )
            .bind(event)
            .bind(owner_table)
            .bind(owner_id)
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(
                key, row_key,
                "{kind} {event}: the payload key is its row's key"
            );
        }
        let seen = by_original
            .entry((*owner_id, original.clone()))
            .or_insert_with(|| key.clone());
        assert_eq!(
            *seen, key,
            "every event naming one original key of one owner carries one sentinel"
        );
    }
    let asserted_max: i64 = sqlx::query_scalar(
        "SELECT max(n)::bigint FROM _resource_erasure_key_numbers('kb_resources', $1)",
    )
    .bind(r)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        by_original[&(r, "never-set".to_string())],
        format!("erased-key-{}", asserted_max + 1),
        "a key only ever unset is numbered above the asserted range"
    );
    assert_replay_byte_identical(&pool, "after redacting property keys").await;
}

/// (26) Two live values of one plain key replay, on R and on an edge touching R: each value
/// redacts to `"erased:<event_id>"`, distinct per event, so replay never meets two live rows of
/// one (owner, key, value).
///
/// FAILS IF: values redact to one constant (replay raises 23505 on `uq_kb_properties_active`).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn two_live_values_of_one_key_replay_after_redaction(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "two-values-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "two-values-twin").await,
    )
    .await;
    for owner_ref in [
        PropertyOwner::resource(leak.resource),
        PropertyOwner::edge(leak.edge),
    ] {
        for value in ["jane", "smith"] {
            writes::assert_keyed_property_with(
                &pool,
                owner_ref,
                "name",
                &serde_json::json!(value),
                1.0,
                emitter,
                EventContext::default(),
            )
            .await
            .unwrap();
        }
    }
    execute_act(&pool, leak.resource.uuid()).await;
    let values: Vec<(Uuid, serde_json::Value)> = sqlx::query_as(
        "SELECT e.id, e.payload->'value' FROM _resource_erasure_trail_scope($1) s \
           JOIN kb_events e ON e.id = s.event_id \
          WHERE s.event_type = 'property_asserted' AND e.payload->>'property_key' LIKE 'erased-key-%'",
    )
    .bind(leak.resource.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(
        values.len() >= 4,
        "the witness needs both owners' asserts: {values:?}"
    );
    for (event, value) in &values {
        assert_eq!(*value, serde_json::json!(format!("erased:{event}")));
    }
    assert_replay_byte_identical(&pool, "with two live values of one key").await;
}

/// (27) A retype keeps the numbering. R is created with a doc type, retyped, and given an open
/// key. The doc type keeps its literal key on the ledger, its values become `"erased:<event_id>"`,
/// and the open key is `erased-key-2` in the payload and the projection alike.
///
/// FAILS IF: the retype's key is redacted (step 9 would then rank it as a second key and the open
/// key would become `erased-key-3` in the projection, against the payload).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_retype_keeps_the_key_numbering(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let r = other_resource(&pool, owner, emitter, "retype-home").await;
    writes::set_property(&pool, r, "doc_type", &serde_json::json!("memo"), emitter)
        .await
        .unwrap();
    writes::set_property(&pool, r, "colour", &serde_json::json!("blue"), emitter)
        .await
        .unwrap();
    let events: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT e.id, e.payload->>'property_key' FROM kb_events e \
           JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'property_set' AND (e.payload#>>'{owner,id}')::uuid = $1 ORDER BY e.id",
    )
    .bind(r.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    let created = created_event(&pool, r.uuid()).await;

    execute_act(&pool, r.uuid()).await;

    assert_eq!(
        payload_of(&pool, created).await["doc_type"],
        serde_json::json!(format!("erased:{created}"))
    );
    for (event, original) in &events {
        let p = payload_of(&pool, *event).await;
        match original.as_str() {
            "doc_type" => {
                assert_eq!(p["property_key"], "doc_type", "the literal key stays");
                assert_eq!(p["value"], serde_json::json!(format!("erased:{event}")));
            }
            "colour" => assert_eq!(p["property_key"], "erased-key-2"),
            other => panic!("unexpected key {other}"),
        }
    }
    let projected: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties WHERE owner_id = $1 AND property_key = 'erased-key-2'",
    )
    .bind(r.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        projected, 1,
        "the projection's open key is erased-key-2 too"
    );
    assert_replay_byte_identical(&pool, "after a retype").await;
}

/// (28) Facet marks survive redaction. A facet value of two inner keys redacts to an object of two
/// `erased-facet-<m>` keys, a later assert of one original inner key carries that key's same `m`,
/// and replay projects the same rows.
///
/// FAILS IF: a facet value redacts to one mark (replay projects one row where live has two).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn facet_marks_survive_redaction(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let r = other_resource(&pool, owner, emitter, "facets-home").await;
    let owner_ref = PropertyOwner::resource(r);
    writes::set_facet(
        &pool,
        owner_ref,
        &serde_json::json!({"a": "jane", "bb": "smith"}),
        1.0,
        emitter,
    )
    .await
    .unwrap();
    writes::set_facet(
        &pool,
        owner_ref,
        &serde_json::json!({"a": "doe"}),
        1.0,
        emitter,
    )
    .await
    .unwrap();
    let facet_events: Vec<Uuid> = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e \
          WHERE e.payload->>'property_key' = 'facet' AND (e.payload#>>'{owner,id}')::uuid = $1 \
          ORDER BY e.id",
    )
    .bind(r.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(facet_events.len(), 2);

    execute_act(&pool, r.uuid()).await;

    assert_eq!(
        payload_of(&pool, facet_events[0]).await["value"],
        serde_json::json!({"erased-facet-1": "erased", "erased-facet-2": "erased"}),
        "two marks stay two, numbered by stored position"
    );
    assert_eq!(
        payload_of(&pool, facet_events[1]).await["value"],
        serde_json::json!({"erased-facet-1": "erased"}),
        "a later assert of inner key `a` carries a's m"
    );
    assert_replay_byte_identical(&pool, "after redacting facet marks").await;
}

/// The trail scope's UNION form (ruled 2026-10-08) is the set its OR form was, over every arm:
/// R's own events, owner-keyed property events of R, its blocks and its edges, block-keyed and
/// edge-keyed events, and relationship_asserted events by their payload endpoints, which reach a
/// re-assertion whose edge_id never became a row. Checked for R and for its twin, so an arm that
/// leaks across resources shows.
///
/// FAILS IF: an arm is dropped from, or added to, the UNION.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_trail_scope_is_the_or_predicates_set(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "scope-home").await;
    let leak = seed_leak(
        &pool,
        owner,
        emitter,
        home,
        make_home(&pool, owner, "scope-twin").await,
    )
    .await;
    writes::assert_keyed_property_with(
        &pool,
        PropertyOwner::edge(leak.edge),
        "evidence",
        &serde_json::json!("x"),
        1.0,
        emitter,
        EventContext::default(),
    )
    .await
    .unwrap();
    let block: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 ORDER BY seq LIMIT 1",
    )
    .bind(leak.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "SELECT _event_append('property_set', $1, 'kb_contexts', $3,
                jsonb_build_object('property_id', gen_random_uuid(),
                                   'owner', jsonb_build_object('table', 'kb_content_blocks', 'id', $2),
                                   'property_key', 'note', 'value', 'x', 'weight', 1.0))",
    )
    .bind(emitter.uuid())
    .bind(block)
    .bind(home.uuid())
    .execute(&pool)
    .await
    .unwrap();

    // A re-assertion of seed_leak's edge: its payload edge_id never becomes a row.
    let edge_row: (Uuid, Uuid, String) =
        sqlx::query_as("SELECT target_id, home_anchor_id, label FROM kb_edges WHERE id = $1")
            .bind(leak.edge.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    writes::assert_relationship(
        &pool,
        AssertParams {
            src: leak.resource,
            tgt: ResourceId::from(edge_row.0),
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some(&edge_row.2),
            weight: 0.5,
            home: ContextId::from(edge_row.1),
            emitter,
        },
    )
    .await
    .unwrap();
    let reassertions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'relationship_asserted' \
            AND NOT EXISTS (SELECT 1 FROM kb_edges g WHERE g.id = (e.payload->>'edge_id')::uuid)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        reassertions, 1,
        "setup: one re-assertion whose edge_id is no row"
    );

    // 20261008100000's predicate verbatim, plus 20261009100000's two endpoint arms.
    let or_form = "SELECT ev.id FROM kb_events ev JOIN kb_event_types et ON et.id = ev.event_type_id \
         WHERE et.category = 'domain' AND et.name <> 'webhook_received' AND ( \
              (ev.payload ->> 'resource_id')::uuid = $1 \
           OR ((ev.payload #>> '{owner,table}') = 'kb_resources' AND (ev.payload #>> '{owner,id}')::uuid = $1) \
           OR EXISTS (SELECT 1 FROM kb_content_blocks b WHERE b.id = (ev.payload ->> 'block_id')::uuid AND b.resource_id = $1) \
           OR EXISTS (SELECT 1 FROM kb_edges ee WHERE ee.id = (ev.payload ->> 'edge_id')::uuid \
                       AND ((ee.source_table = 'kb_resources' AND ee.source_id = $1) \
                         OR (ee.target_table = 'kb_resources' AND ee.target_id = $1))) \
           OR ((ev.payload #>> '{owner,table}') = 'kb_content_blocks' \
               AND EXISTS (SELECT 1 FROM kb_content_blocks b2 WHERE b2.id = (ev.payload #>> '{owner,id}')::uuid AND b2.resource_id = $1)) \
           OR ((ev.payload #>> '{owner,table}') = 'kb_edges' \
               AND (ev.payload #>> '{owner,id}')::uuid IN ( \
                   SELECT ee2.id FROM kb_edges ee2 \
                    WHERE (ee2.source_table = 'kb_resources' AND ee2.source_id = $1) \
                       OR (ee2.target_table = 'kb_resources' AND ee2.target_id = $1))) \
           OR (ev.payload #>> '{source,table}' = 'kb_resources' AND ev.payload #>> '{source,id}' = $1::text) \
           OR (ev.payload #>> '{target,table}' = 'kb_resources' AND ev.payload #>> '{target,id}' = $1::text)) \
         ORDER BY 1";
    for resource in [leak.resource.uuid(), leak.twin.uuid()] {
        let expected: Vec<Uuid> = sqlx::query_scalar(or_form)
            .bind(resource)
            .fetch_all(&pool)
            .await
            .unwrap();
        let actual: Vec<Uuid> =
            sqlx::query_scalar("SELECT event_id FROM _resource_erasure_trail_scope($1) ORDER BY 1")
                .bind(resource)
                .fetch_all(&pool)
                .await
                .unwrap();
        assert!(!expected.is_empty());
        assert_eq!(
            actual, expected,
            "the UNION form is the OR form's set for {resource}"
        );
    }
}

/// Edge labels redact per endpoint pair, every non-empty label counting as text (ruled 2026-10-08,
/// with the review fixes). A re-assertion, whose payload edge_id never became a row, is in the
/// trail and redacts to its first assertion's sentinel; a label someone typed in a sentinel's shape
/// is numbered like any other label, so it never lands on another label's text. Replay builds the
/// same edges.
///
/// FAILS IF: the trail misses a re-assertion (replay then mints an edge live never had), or a
/// sentinel-shaped label is kept as already redacted (replay merges two edges and aborts).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn reassertions_and_sentinel_shaped_labels_redact_and_replay(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "labels-home").await;
    let r = other_resource(&pool, owner, emitter, "labels-r").await;
    let t = other_resource(&pool, owner, emitter, "labels-t").await;
    for label in ["foo", "erased-label-1", "foo"] {
        writes::assert_relationship(
            &pool,
            AssertParams {
                src: r,
                tgt: t,
                kind: EdgeKind::LeadsTo,
                polarity: EdgePolarity::Forward,
                label: Some(label),
                weight: 1.0,
                home,
                emitter,
            },
        )
        .await
        .unwrap();
    }
    let asserts: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT e.id, e.payload->>'label' FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'relationship_asserted' AND e.payload#>>'{source,id}' = $1::text ORDER BY e.id",
    )
    .bind(r.uuid())
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        asserts.len(),
        3,
        "setup: foo, a sentinel-shaped label, and foo again"
    );

    execute_act(&pool, r.uuid()).await;

    let after: Vec<String> = {
        let mut out = Vec::new();
        for (event, _) in &asserts {
            out.push(
                payload_of(&pool, *event).await["label"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            );
        }
        out
    };
    assert_eq!(
        after,
        vec!["erased-label-1", "erased-label-2", "erased-label-1"],
        "foo is 1 in both its assertions; the typed label is text, numbered 2"
    );
    assert_replay_byte_identical(&pool, "with a re-assertion and a sentinel-shaped label").await;
}

/// An empty facet value names no mark, so it has nothing to redact, and it does not stop the act
/// (found by the code review, 2026-10-08: the derivation raised, so anyone able to set an empty
/// facet on an edge into R could make R unerasable).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_empty_facet_does_not_block_the_act(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let r = other_resource(&pool, owner, emitter, "empty-facet").await;
    writes::set_facet(
        &pool,
        PropertyOwner::resource(r),
        &serde_json::json!({}),
        1.0,
        emitter,
    )
    .await
    .unwrap();
    execute_act(&pool, r.uuid()).await;
    assert_replay_byte_identical(&pool, "after an empty facet").await;
}

/// An erasure act's own events are its record. When the other end of an edge an earlier act folded
/// is erased later, that act's `relationship_folded` keeps its fixed reason.
///
/// FAILS IF: the derivation rewrites another erasure act's events.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_later_erasure_leaves_an_earlier_acts_record(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "two-acts-home").await;
    let r = other_resource(&pool, owner, emitter, "two-acts-r").await;
    let s = other_resource(&pool, owner, emitter, "two-acts-s").await;
    let edge = writes::assert_relationship(
        &pool,
        AssertParams {
            src: r,
            tgt: s,
            kind: EdgeKind::LeadsTo,
            polarity: EdgePolarity::Forward,
            label: Some("jane's link"),
            weight: 1.0,
            home,
            emitter,
        },
    )
    .await
    .unwrap();
    execute_act(&pool, r.uuid()).await;
    let fold: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'relationship_folded' AND e.payload->>'edge_id' = $1",
    )
    .bind(edge.uuid().to_string())
    .fetch_one(&pool)
    .await
    .unwrap();
    execute_act(&pool, s.uuid()).await;
    assert_eq!(
        payload_of(&pool, fold).await["reason"],
        "resource_erased",
        "the first act's fold keeps its reason"
    );
    assert_replay_byte_identical(&pool, "after erasing both ends of an edge").await;
}

/// Snapshot, reset, replay and diff, as [`assert_replay_byte_identical`] does, except that
/// `kb_remote_sources.uri` may differ on exactly `differing` rows, each of which `is_respelled`
/// accepts as (live spelling, replayed spelling) and whose `uri_normalized` is unchanged. This is
/// the declared replay limit of ruling 10 (D14): `uri` keeps its first writer's spelling, and cut 2
/// moves the first write of a URL R cited to R's original citation.
async fn assert_replay_identical_but_first_writer_spelling(
    pool: &PgPool,
    differing: usize,
    is_respelled: impl Fn(&str, &str) -> bool,
) {
    let before = replay::dump_projections(pool).await.unwrap();
    let snap = replay::snapshot(pool).await.unwrap();
    common::reset_schema(pool).await;
    replay::replay(pool, &snap).await.unwrap();
    let after = replay::dump_projections(pool).await.unwrap();
    for ((table, live), (_, replayed)) in before.iter().zip(after.iter()) {
        if table != "kb_remote_sources" {
            assert_eq!(
                live, replayed,
                "projection table {table} diverged under replay"
            );
            continue;
        }
        let (live, replayed) = (live.as_array().unwrap(), replayed.as_array().unwrap());
        assert_eq!(
            live.len(),
            replayed.len(),
            "kb_remote_sources row count under replay"
        );
        let mut seen = 0;
        for (l, r) in live.iter().zip(replayed) {
            let (mut lm, mut rm) = (l.clone(), r.clone());
            lm["uri"] = serde_json::Value::Null;
            rm["uri"] = serde_json::Value::Null;
            assert_eq!(
                lm, rm,
                "kb_remote_sources differs under replay beyond `uri`"
            );
            if l != r {
                seen += 1;
                let (lu, ru) = (l["uri"].as_str().unwrap(), r["uri"].as_str().unwrap());
                assert!(
                    is_respelled(lu, ru),
                    "an unexpected respelling: {lu:?} live, {ru:?} replayed"
                );
            }
        }
        assert_eq!(
            seen, differing,
            "exactly the expected rows keep a different spelling"
        );
    }
}

/// (Ruling 10, widened 2026-10-08) A remote source R cited first and another resource also cites
/// keeps R's spelling live (the row is kept, Q1), while replay of the redacted ledger mints it from
/// the other citer's. The difference is that row's `uri` spelling and nothing else.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_shared_remote_sources_spelling_is_its_first_remaining_writers_on_replay(
    pool: sqlx::PgPool,
) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let cite = |slug: &'static str, url: &'static str| {
        let pool = pool.clone();
        async move {
            let home = make_home(&pool, owner, slug).await;
            writes::create_resource_with(
                &pool,
                CreateParams {
                    idempotency_key: None,
                    title: slug,
                    origin_uri: "test://shared-url",
                    body: CLEAN,
                    doc_type: "research",
                    home: AnchorRef::context(home),
                    owner,
                    originator: owner,
                    emitter,
                    properties: &[],
                    chunks: Some(vec![chunk(CLEAN, "")]),
                    sources: vec![Incorporation {
                        source: ProvenanceSource::Remote(url.to_owned()),
                        seq: 1,
                    }],
                },
                EventContext::default(),
            )
            .await
            .unwrap()
        }
    };
    let r = cite("shared-url-r", "HTTPS://Leak.Example/a").await;
    cite("shared-url-q", "https://leak.example/a").await;
    execute_act(&pool, r.uuid()).await;
    let kept: String = sqlx::query_scalar(
        "SELECT uri FROM kb_remote_sources WHERE uri_normalized = normalize_remote_uri('https://leak.example/a')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        kept, "HTTPS://Leak.Example/a",
        "setup: live keeps R's spelling"
    );
    assert_replay_identical_but_first_writer_spelling(&pool, 1, |live, replayed| {
        live == "HTTPS://Leak.Example/a" && replayed == "https://leak.example/a"
    })
    .await;
}

/// The planted strings a completion pass must take off R's trail, by the trail's own scope.
async fn trail_carrying(pool: &PgPool, resource: Uuid, planted: &str) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT ev.id FROM _resource_erasure_trail_scope($1) s \
           JOIN kb_events ev ON ev.id = s.event_id \
          WHERE strpos(ev.payload::text || ev.metadata::text, $2) > 0",
    )
    .bind(resource)
    .bind(planted)
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn erasure_records_of(pool: &PgPool, resource: Uuid) -> Vec<serde_json::Value> {
    sqlx::query_scalar(
        "SELECT e.payload FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'resource_erased' AND e.payload->>'subject_id' = $1::text ORDER BY e.id",
    )
    .bind(resource)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// (15) The completion pass (spec D12, ruling 14 of cut 2 PR 2). A resource erased under cut 1
/// keeps its text on the ledger; running the act on it again mints ONE more `resource_erased`
/// carrying only `redacted_fields`, the trail no longer carries any planted string, the rest of
/// the projection does not move, and replay is byte-identical. A third run is refused `already
/// erased` and changes nothing.
///
/// R's block cites remote sources the husk's provenance has already re-pointed (step 9e), one of
/// them shared with the twin, and one URL listed twice in one event at two seqs, the later
/// element at the LOWER seq. Provenance keeps the first-listed element's seq (its unique key is
/// per contributing event, ON CONFLICT DO NOTHING), so (9e) numbers the other URL of that event
/// first; a ledger numbering by the lowest seq would disagree, and the pass would raise on the
/// husk forever. The agreement half compares the two numberings on the live resource first.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_cut1_husk_completes_once_and_replays_byte_identical(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    register_block_provenance_annotated(&pool).await;
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "completion-home").await;
    let twin_home = make_home(&pool, owner, "completion-twin").await;
    let leak = seed_leak(&pool, owner, emitter, home, twin_home).await;
    let r = leak.resource.uuid();
    writes::annotate_block_sources(
        &pool,
        writes::AnnotateParams {
            resource: leak.twin,
            sources: vec![Incorporation {
                source: ProvenanceSource::Remote(URL.to_owned()),
                seq: 0,
            }],
            content_block: None,
            emitter,
        },
    )
    .await
    .expect("the twin cites URL too, so its row is shared");
    let r_block: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded \
          ORDER BY seq LIMIT 1",
    )
    .bind(r)
    .fetch_one(&pool)
    .await
    .unwrap();
    const TWICE: &str = "https://leak.example/twice-cited";
    const BETWEEN: &str = "https://leak.example/cited-between";
    writes::annotate_block_sources(
        &pool,
        writes::AnnotateParams {
            resource: leak.resource,
            sources: vec![
                Incorporation {
                    source: ProvenanceSource::Remote(TWICE.to_owned()),
                    seq: 5,
                },
                Incorporation {
                    source: ProvenanceSource::Remote(BETWEEN.to_owned()),
                    seq: 2,
                },
                Incorporation {
                    source: ProvenanceSource::Remote(TWICE.to_owned()),
                    seq: 1,
                },
            ],
            content_block: Some(r_block),
            emitter,
        },
    )
    .await
    .expect("one URL listed twice in one event");
    writes::set_property_with(
        &pool,
        leak.resource,
        "status",
        &serde_json::json!("under review"),
        emitter,
        EventContext {
            authorship: Some(AgentAuthorship {
                reasoning: Some("jane smith asked for the review".into()),
                confidence: ConfidenceBand::Probable,
                rationale: None,
                persona: Some("jane's assistant".into()),
                model: None,
            }),
            ..EventContext::default()
        },
    )
    .await
    .unwrap();
    writes::assert_keyed_property_with(
        &pool,
        PropertyOwner::edge(leak.edge),
        "evidence",
        &serde_json::json!("jane's diary, page 4"),
        1.0,
        emitter,
        EventContext::default(),
    )
    .await
    .unwrap();

    // The agreement half: on the live resource, the ledger numbering is (9e)'s.
    let numbering = |sql: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_as::<_, (Uuid, String, i32)>(sql)
                .bind(r)
                .fetch_all(&pool)
                .await
                .unwrap()
        }
    };
    let from_ledger = numbering(
        "SELECT block_id, uri_normalized, n FROM _resource_erasure_ledger_remote_numbers($1) \
          ORDER BY block_id, n",
    )
    .await;
    let from_provenance = numbering(
        "SELECT o.block_id, rs.uri_normalized, o.n FROM _resource_erasure_remote_originals($1) o \
           JOIN kb_remote_sources rs ON rs.id = o.source_id ORDER BY o.block_id, o.n",
    )
    .await;
    assert_eq!(
        from_ledger, from_provenance,
        "the ledger numbers R's remote sources as step (9e) does"
    );
    let n_of = |url: &str| {
        from_provenance
            .iter()
            .find(|(b, u, _)| *b == r_block && u.contains(&url[8..]))
            .map(|(_, _, n)| *n)
            .unwrap_or_else(|| panic!("{url} is numbered on R's block: {from_provenance:?}"))
    };
    assert!(
        n_of(BETWEEN) < n_of(TWICE),
        "precondition: (9e) orders the twice-listed URL by its first-listed seq, so the case bites"
    );

    let first = simulate_cut1_husk(&pool, r).await;
    for planted in [
        "M&A notes (leaked)",
        URL,
        TWICE,
        "jane smith spoke to us",
        "jane's diary",
    ] {
        assert!(
            !trail_carrying(&pool, r, planted).await.is_empty(),
            "precondition: the cut-1 husk's trail still carries {planted:?}"
        );
    }
    let first_record = &erasure_records_of(&pool, r).await[0];
    assert!(
        first_record.get("redacted_fields").is_none()
            && first_record["ledger_remainder"]
                .as_array()
                .is_some_and(|a| !a.is_empty()),
        "precondition: the husk is cut 1's, naming its ledger remainder: {first_record}"
    );

    // The survey door's list (D10) is what the pass will name.
    let pending: serde_json::Value =
        sqlx::query_scalar("SELECT resource_erasure_completion_fields($1)")
            .bind(r)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(pending.as_array().is_some_and(|a| !a.is_empty()));

    let projection_before = replay::dump_projections(&pool).await.unwrap();
    let completion = execute_act(&pool, r).await;
    assert_ne!(completion, first);

    let records = erasure_records_of(&pool, r).await;
    assert_eq!(records.len(), 2, "the pass mints exactly one more record");
    let keys: std::collections::BTreeSet<&str> = records[1]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        ["actor", "redacted_fields", "subject_id", "subject_table"]
            .into_iter()
            .collect(),
        "the completion record carries only redacted_fields"
    );
    assert_eq!(records[1]["redacted_fields"], pending);
    for planted in [
        "M&A notes (leaked)",
        "test://seed-leak",
        URL,
        TWICE,
        BETWEEN,
        "jane smith spoke to us",
        "transient",
        "jane smith asked for the review",
        "jane's assistant",
        "under review",
        "jane's diary",
    ] {
        let still = trail_carrying(&pool, r, planted).await;
        assert!(
            still.is_empty(),
            "the trail still carries {planted:?} in {still:?}"
        );
    }

    // The body changed nothing but its two stamps: only the redaction rows (and the ledger,
    // which is not a projection) moved, plus the two columns the body stamps from the erasing
    // event, as replay's arm does at the record's position: `kb_resources.updated` (its
    // occurred_at, asserted below) and `kb_properties.last_event_id` (its id).
    let projection_after = replay::dump_projections(&pool).await.unwrap();
    for ((table, a), (_, b)) in projection_before.iter().zip(projection_after.iter()) {
        if table != "kb_event_field_redactions" && a != b {
            let (ra, rb) = (a.as_array().unwrap(), b.as_array().unwrap());
            let moved: Vec<String> = ra
                .iter()
                .zip(rb.iter())
                .flat_map(|(x, y)| {
                    x.as_object()
                        .unwrap()
                        .iter()
                        .filter(|(k, v)| y.get(k.as_str()) != Some(*v))
                        .filter(|(k, v)| {
                            !(table == "kb_resources" && k.as_str() == "updated")
                                && !(table == "kb_properties"
                                    && k.as_str() == "last_event_id"
                                    && y[k.as_str()] == serde_json::json!(completion)
                                    && **v != y[k.as_str()])
                        })
                        .map(|(k, v)| format!("{k}: {v} -> {}", y[k.as_str()]))
                        .collect::<Vec<_>>()
                })
                .collect();
            assert!(
                moved.is_empty() && ra.len() == rb.len(),
                "the completion pass moved projection table {table} ({} rows -> {}): {moved:?}",
                ra.len(),
                rb.len()
            );
        }
    }

    let stamped: bool = sqlx::query_scalar(
        "SELECT r.updated = e.occurred_at FROM kb_resources r, kb_events e \
          WHERE r.id = $1 AND e.id = $2",
    )
    .bind(r)
    .bind(completion)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        stamped,
        "the husk's updated is the completion record's occurred_at"
    );

    // A third run has nothing left: refused, nothing changes.
    let ledger_before: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    let (_, operator_entity) = system_actor(&pool).await;
    let third = sqlx::query("SELECT resource_erasure_execute($1, $2, $2, $3)")
        .bind(r)
        .bind(operator_entity)
        .bind(Uuid::now_v7())
        .execute(&pool)
        .await
        .expect_err("a completed husk is refused");
    assert!(
        third.to_string().contains("already erased"),
        "refused as already erased: {third}"
    );
    let ledger_after: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(ledger_before, ledger_after);
    let empty: serde_json::Value =
        sqlx::query_scalar("SELECT resource_erasure_completion_fields($1)")
            .bind(r)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(empty, serde_json::json!([]));

    assert_replay_byte_identical(&pool, "after a cut-1 husk's completion pass").await;
}

/// The shape arm's cut-1 branch (ruling 8 of cut 2 PR 2). On a husk erased under cut 1, step (9f)
/// has rewritten the artifact rows' family but the ledger's `data_artifact_committed` still
/// carries it, so a plan on that husk still names the home's shape for the family, through the
/// ledger. The completion pass redacts that family, and the plan names the shape no more. This
/// branch needed the survey to run on a cut-1 husk, which raised on remote sources until the
/// completion pass numbered them from the ledger.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_cut1_husks_plan_names_its_shape_through_the_ledger_until_completed(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "cut1-shape-home").await;
    let twin_home = make_home(&pool, owner, "cut1-shape-twin").await;
    let leak = seed_leak(&pool, owner, emitter, home, twin_home).await;
    let mut shapes = Vec::new();
    for kind in ["notes", "unrelated"] {
        shapes.push(
            writes::declare_shape(
                &pool,
                writes::DeclareShapeParams {
                    home: AnchorRef::context(home),
                    kind,
                    kind_owner: Some(KindOwner::Profile(owner.uuid())),
                    schema: &serde_json::json!({"type": "object"}),
                    enforcement: payloads::EnforcementMode::Advisory,
                    emitter,
                },
            )
            .await
            .unwrap()
            .uuid(),
        );
    }
    let (notes_shape, unrelated_shape) = (shapes[0], shapes[1]);
    let named_shapes = || {
        let pool = pool.clone();
        let resource = leak.resource.uuid();
        async move {
            let plan: serde_json::Value =
                sqlx::query_scalar("SELECT resource_erasure_survey_plan($1)")
                    .bind(resource)
                    .fetch_one(&pool)
                    .await
                    .expect("the plan runs on the husk");
            plan["remainder"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e["target"] == "kb_data_artifact_shapes")
                .map(|e| e["outcome"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        }
    };

    simulate_cut1_husk(&pool, leak.resource.uuid()).await;
    let family_on_rows: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_data_artifacts WHERE resource_id = $1 AND artifact_kind = 'notes'",
    )
    .bind(leak.resource.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        family_on_rows, 0,
        "precondition: step (9f) rewrote the rows' family"
    );
    let on_husk = named_shapes().await;
    assert_eq!(
        on_husk.len(),
        1,
        "the ledger family names one shape: {on_husk:?}"
    );
    assert!(
        on_husk[0].contains(&notes_shape.to_string())
            && !on_husk[0].contains(&unrelated_shape.to_string()),
        "the shape for the family R used, by id: {on_husk:?}"
    );

    execute_act(&pool, leak.resource.uuid()).await;
    assert_eq!(
        named_shapes().await,
        Vec::<String>::new(),
        "after the completion pass the ledger no longer carries the family"
    );
}

/// A JSON `null` at an allowlisted path carries no text, and the redaction keeps it (ruling 13 of
/// cut 2 PR 2, which had no witness). Rewritten to a sentinel, replay of the redacted ledger would
/// project a value live never held, and every key number after it would move.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_null_on_the_ledger_is_kept_and_replays(pool: sqlx::PgPool) {
    common::reset_schema(&pool).await;
    temper_substrate::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "null-keep-home").await;
    let twin_home = make_home(&pool, owner, "null-keep-twin").await;
    let leak = seed_leak(&pool, owner, emitter, home, twin_home).await;
    writes::set_property(
        &pool,
        leak.resource,
        "cleared",
        &serde_json::Value::Null,
        emitter,
    )
    .await
    .expect("a property set to null through the real path");
    let null_event: Uuid = sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name IN ('property_set', 'property_asserted') \
            AND e.payload->>'property_key' = 'cleared'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let before: serde_json::Value = payload_of(&pool, null_event).await;
    assert_eq!(
        before["value"],
        serde_json::Value::Null,
        "precondition: {before}"
    );

    execute_act(&pool, leak.resource.uuid()).await;
    let after = payload_of(&pool, null_event).await;
    assert_eq!(
        after["value"],
        serde_json::Value::Null,
        "the null is kept: {after}"
    );
    assert_ne!(
        after["property_key"], "cleared",
        "the key is still redacted: {after}"
    );
    assert_replay_byte_identical(&pool, "with a null value on R's trail").await;
}
