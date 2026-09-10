//! Witnesses for the audit gate's folded-citation clause: `finding_of_block` filters
//! `AND NOT is_folded`, so auditing a FOLDED citation is a defined NotFound refusal — never a
//! silent success on a gone citation — while a LIVE citation still audits.
//!
//! `finding_of_block` is `pub(crate)` to temper-services, so the clause is witnessed through
//! the public door that calls it: `services::citation_audit_service::record_citation_audit`
//! (the transposition guard + backend dispatch that renders the refusal). The folded block is
//! seeded through the real whole-body replace write path — the filler-append geometry that
//! folds the rewritten-away incumbent while its citation survives verbatim on the folded row
//! (`citation_is_live` still answers true for it, which is exactly why the filter must live in
//! the gate: without `AND NOT is_folded` this audit silently succeeded).
//!
//! Fixtures duplicate each source suite's convention per this tier's rule, cribbed from
//! `temper-substrate/tests/whole_body_replace.rs` (system_actor / make_home / create_two_block /
//! update_body / blocks_of), `authz/audit_gate.rs`'s inline seed (the read-only access grant,
//! the source resource row), and `standing_clock_test.rs` (the auditor profile + its surface
//! emitter entities).
//!
//! ONNX-dependent (the create/update fixtures server-embed). Isolated ephemeral DB via
//! `temper_substrate::MIGRATOR`.
#![cfg(feature = "test-db")]

use sqlx::PgPool;
use temper_core::types::authorship::ActContext;
use temper_core::types::ids::{BlockId, EntityId, ProfileId, ResourceId};
use temper_core::types::provenance::ProvenanceSource;
use temper_services::error::ApiError;
use temper_services::services::citation_audit_service;
use temper_substrate::payloads::{AnchorRef, Incorporation};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{
    self, AppendParams, CreateMode, CreateParams, FinalizeParams, UpdateParams,
};
use temper_workflow::operations::{RecordCitationAudit, Surface};
use uuid::Uuid;

const SECTION_A: &str = "# Alpha\n\nAlpha body paragraph.\n";
const SECTION_B: &str = "## Beta\n\nBeta body paragraph.\n";

// ── fixture helpers (duplicated per file, per this tier's convention) ───────────────────────

async fn system_actor(pool: &PgPool) -> (ProfileId, EntityId) {
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

async fn make_home(pool: &PgPool, owner: ProfileId, slug: &str) -> AnchorRef {
    let ctx: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, $2, $2) RETURNING id",
    )
    .bind(owner.uuid())
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap();
    AnchorRef::context(temper_core::types::ids::ContextId::from(ctx))
}

/// A two-block resource through the segmented trio (the whole_body_replace.rs fixture shape),
/// with `block0_sources` landing on the first block — a resource-kind source here makes the
/// block a CITER, so its citation is auditable at all (only resource-kind citations are).
async fn create_two_block(
    pool: &PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: &AnchorRef,
    first: &str,
    rest: &str,
    block0_sources: Vec<Incorporation>,
) -> ResourceId {
    use temper_substrate::events::EventContext;
    let resource = writes::create_resource_with_mode(
        pool,
        CreateParams {
            title: "audit-gate fixture",
            origin_uri: "temper://audit-gate/fixture",
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

async fn update_body(pool: &PgPool, emitter: EntityId, resource: ResourceId, body: &str) {
    writes::update_resource(
        pool,
        UpdateParams {
            resource,
            body: Some(body),
            title: None,
            origin_uri: None,
            properties: &[],
            chunks: None,
            sources: vec![],
            content_block: None,
            rehome_to: None,
            emitter,
        },
    )
    .await
    .unwrap();
}

/// (id, seq) of the LIVE blocks, seq order.
async fn blocks_of(pool: &PgPool, resource: ResourceId) -> Vec<(Uuid, i32)> {
    sqlx::query_as(
        "SELECT id, seq FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded ORDER BY seq, id",
    )
    .bind(resource.uuid())
    .fetch_all(pool)
    .await
    .unwrap()
}

/// The cited source — a real resource row, because only resource-kind citations are auditable.
async fn seed_source_resource(pool: &PgPool) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('a source', '') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

/// The auditor principal: a profile with the three surface emitter entities the write path's
/// `resolve_emitter` resolves (`<handle>@<surface>`). Cribbed from `standing_clock_test.rs`'s
/// `seed_profile`.
async fn seed_auditor(pool: &PgPool) -> ProfileId {
    let profile_id = Uuid::now_v7();
    let handle = format!("auditor-{}", &profile_id.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name) VALUES ($1, $2, $2)")
        .bind(profile_id)
        .bind(&handle)
        .execute(pool)
        .await
        .unwrap();
    for surface in ["web", "cli", "mcp"] {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1, $2, '{}'::jsonb)",
        )
        .bind(profile_id)
        .bind(format!("{handle}@{surface}"))
        .execute(pool)
        .await
        .unwrap();
    }
    ProfileId::from(profile_id)
}

/// Read WITHOUT write on the finding — the shape that admits the caller as `Auditor` (it can
/// read, contributed nothing, cannot modify). Cribbed from `authz/audit_gate.rs`'s inline seed.
async fn grant_read(pool: &PgPool, finding: Uuid, reader: ProfileId, author: ProfileId) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (subject_table, subject_id, principal_table, principal_id, can_read, can_write, \
            granted_by_profile_id) \
         VALUES ('kb_resources', $1, 'kb_profiles', $2, true, false, $3)",
    )
    .bind(finding)
    .bind(reader.uuid())
    .bind(author.uuid())
    .execute(pool)
    .await
    .unwrap();
}

fn audit_cmd(block: Uuid, source: Uuid, value: f64) -> RecordCitationAudit {
    RecordCitationAudit {
        block: BlockId::from(block),
        source: ProvenanceSource::Resource(source),
        value,
        reason: Some("witness verdict".to_string()),
        act: ActContext::default(),
        origin: Surface::ApiHttp,
    }
}

/// CLAUSE: auditing a FOLDED citation is refused — NotFound-class, the same zero-rows arm as an
/// unknown block — even though everything else about the audit would pass: the citation is
/// still live on the ledger, and the caller can read the finding and contributed nothing. The
/// refusal is the `AND NOT is_folded` filter, not a readability or authorization denial.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_audit_of_a_folded_citation_is_refused_as_not_found(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (author, emitter) = system_actor(&pool).await;
    let source = seed_source_resource(&pool).await;
    let finding = create_two_block(
        &pool,
        author,
        emitter,
        &make_home(&pool, author, "ag-folded").await,
        SECTION_A,
        SECTION_B,
        vec![Incorporation {
            source: ProvenanceSource::Resource(source),
            seq: 0,
        }],
    )
    .await;
    let (cited_block, _) = blocks_of(&pool, finding).await[0];
    let auditor = seed_auditor(&pool).await;
    grant_read(&pool, finding.uuid(), auditor, author).await;

    // Fold the cited block (filler-append geometry: A's section re-creates, the incumbent
    // folds, its citation row survives verbatim ON THE FOLDED ROW).
    let filler = "Brand new prose. ".repeat(120);
    let new_body =
        format!("# Alpha\n\nAlpha body paragraph.\n\n{filler}\n## Beta\n\nBeta body paragraph.\n");
    update_body(&pool, emitter, finding, &new_body).await;
    let live_now: Vec<Uuid> = blocks_of(&pool, finding)
        .await
        .iter()
        .map(|(id, _)| *id)
        .collect();
    assert!(
        !live_now.contains(&cited_block),
        "fixture: the cited block folded"
    );
    let still_a_live_citation: bool =
        sqlx::query_scalar("SELECT citation_is_live($1, 'resource'::provenance_source_kind, $2)")
            .bind(cited_block)
            .bind(source)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        still_a_live_citation,
        "fixture: the citation row survives on the folded block — the gate filter is the only \
         thing standing between the auditor and a gone citation"
    );
    assert!(
        temper_substrate::readback::is_resource_visible(&pool, auditor, finding)
            .await
            .unwrap(),
        "precondition: the caller CAN read the finding, so the refusal below is the folded \
         filter, not a readability denial"
    );

    let err = citation_audit_service::record_citation_audit(
        &pool,
        auditor,
        finding,
        audit_cmd(cited_block, source, -1.0),
    )
    .await
    .expect_err("a folded citation must not audit");
    assert!(
        matches!(err, ApiError::NotFound(_)),
        "the folded refusal renders in the gate's NotFound dialect, got {err:?}"
    );
}

/// CLAUSE: the control — the same caller, same grant shape, auditing a LIVE citation on the
/// same-shaped finding, proceeds. Without this arm the refusal above could be the gate refusing
/// everything, not the folded filter refusing folded blocks.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_audit_of_a_live_citation_proceeds(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (author, emitter) = system_actor(&pool).await;
    let source = seed_source_resource(&pool).await;
    let finding = create_two_block(
        &pool,
        author,
        emitter,
        &make_home(&pool, author, "ag-live").await,
        SECTION_A,
        SECTION_B,
        vec![Incorporation {
            source: ProvenanceSource::Resource(source),
            seq: 0,
        }],
    )
    .await;
    let (cited_block, _) = blocks_of(&pool, finding).await[0];
    let auditor = seed_auditor(&pool).await;
    grant_read(&pool, finding.uuid(), auditor, author).await;

    let audit_id = citation_audit_service::record_citation_audit(
        &pool,
        auditor,
        finding,
        audit_cmd(cited_block, source, 0.5),
    )
    .await
    .expect("a live citation audits for a reader who did not contribute it");

    let value: f64 = sqlx::query_scalar("SELECT value FROM kb_citation_audits WHERE id = $1")
        .bind(audit_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(value, 0.5, "the verdict landed, append-only, on the ledger");
}
