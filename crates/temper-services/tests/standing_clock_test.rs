//! Integration test — the standing clock fires on the resource write path (Set 3, Phase B, Task 6).
//!
//! `DbBackend::tick_resource_standing` is wired immediately after `tick_region_clocks` at the resource
//! create and update sites. This test proves that wiring end-to-end: it drives a resource create
//! through the real backend command path (the same path that reaches the create-site tick) and asserts
//! the `kb_resource_standing` memo row appeared as a side effect — *without* any explicit refresh call.
//! That single side-effect assertion is what proves the clock is on the write path; the memo's
//! component parity (r_parent etc.) is already covered at the substrate level by Task 5's wrapper test
//! (`temper-substrate/tests/evidential_standing.rs`).
#![cfg(feature = "test-db")]

use sqlx::PgPool;

use temper_core::types::authorship::ActContext;
use temper_core::types::home::HomeAnchor;
use temper_core::types::ids::{BlockId, ContextId, ProfileId};
use temper_core::types::ingest::{pack_chunks, PackedChunk};
use temper_core::types::provenance::ProvenanceSource;
use temper_services::backend::DbBackend;
use temper_workflow::operations::{
    Backend, BodyUpdate, CreateResource, RecordCitationAudit, Surface,
};
use temper_workflow::types::managed_meta::ManagedMeta;

/// Seed a substrate profile plus its three surface emitter entities — the minimum any principal
/// that authors an act needs, since the write path resolves `<handle>@<surface>` through
/// `writes::resolve_profile` + `resolve_emitter`. Split out of
/// [`seed_profile_with_context`] because the auditor below needs a principal and no context of its
/// own: its access to the finding is a grant, not a home.
async fn seed_profile(pool: &PgPool, email: &str) -> uuid::Uuid {
    let profile_id = uuid::Uuid::now_v7();
    let local = email.split('@').next().unwrap_or("test-user");
    let handle = format!("{local}-{}", &profile_id.simple().to_string()[..8]);
    sqlx::query("INSERT INTO kb_profiles (id, handle, display_name, email) VALUES ($1,$2,$3,$4)")
        .bind(profile_id)
        .bind(&handle)
        .bind(email)
        .bind(email)
        .execute(pool)
        .await
        .expect("seed profile");
    for surface in ["web", "cli", "mcp"] {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1,$2,'{}'::jsonb)",
        )
        .bind(profile_id)
        .bind(format!("{handle}@{surface}"))
        .execute(pool)
        .await
        .expect("seed emitter entity");
    }
    profile_id
}

/// Seed a substrate profile + a profile-owned `temper` context (the minimum the write path's
/// `resolve_emitter` + visibility gate require). Mirrors `segmented_backend_test.rs`'s inlined
/// fixture — each test-target crate keeps its own copy so it has no cross-target test-harness
/// dependency.
async fn seed_profile_with_context(pool: &PgPool, email: &str) -> (uuid::Uuid, uuid::Uuid) {
    let profile_id = seed_profile(pool, email).await;
    let context_id = uuid::Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1,'kb_profiles',$2,'temper','temper')",
    )
    .bind(context_id)
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("seed context");
    (profile_id, context_id)
}

/// A single pre-chunked, pre-embedded segment (bring-your-own-vectors path) — ONNX-free, so the
/// create lands without touching the server-side embedder. Mirrors `segmented_backend_test.rs`.
fn one_chunk_packed(text: &str, hash_seed: &str) -> String {
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: text.to_owned(),
        content_hash: format!("{hash_seed:0>64}"),
        embedding: vec![0.1_f32; 768],
        embedded_with: None,
    };
    pack_chunks(&[chunk]).expect("pack chunk")
}

/// One create command, so the cited source and the citing finding differ only in the fields this
/// test is about. `sources` is what makes a finding a *citer*: position becomes the accretion `seq`
/// and the create itself lands the `kb_block_provenance` row (`create_sources`), so no separate
/// annotate call is needed to build a citation.
fn create_cmd(
    context: uuid::Uuid,
    slug: &str,
    title: &str,
    hash_seed: &str,
    sources: Vec<ProvenanceSource>,
) -> CreateResource {
    let content = format!("body of {title}");
    CreateResource {
        idempotency_key: None,
        slug: slug.to_string(),
        doctype: "research".to_string(),
        home: HomeAnchor::Context(ContextId::from(context)),
        title: title.to_string(),
        body: Some(BodyUpdate {
            content: content.clone(),
            content_hash: None,
            chunks_packed: None,
            sources,
            content_block: None,
        }),
        managed_meta: ManagedMeta::default(),
        open_meta: None,
        goal: None,
        origin_uri: Some(format!("test://{slug}")),
        chunks_packed: Some(one_chunk_packed(&content, hash_seed)),
        content_hash: None,
        act: ActContext::default(),
        origin: Surface::ApiHttp,
    }
}

/// The three Set 5 axes as the `kb_resource_standing` memo holds them. Read as one row so the
/// before and after snapshots are literally the same query — three separate scalar reads could
/// drift apart and hide which axis moved.
#[derive(Debug, sqlx::FromRow)]
struct MemoAxes {
    citation_magnitude: i32,
    audit_coverage: i32,
    citation_quality: f64,
}

/// Read the memo COLUMNS, deliberately — not `resource_standing_shape`, which recomputes every
/// component live and would therefore report the right answer even if the clock never fired. These
/// columns are written by nothing except `refresh_resource_standing`.
async fn read_memo_axes(pool: &PgPool, finding: uuid::Uuid) -> MemoAxes {
    sqlx::query_as::<_, MemoAxes>(
        "SELECT citation_magnitude, audit_coverage, citation_quality \
           FROM kb_resource_standing WHERE finding_id = $1",
    )
    .bind(finding)
    .fetch_one(pool)
    .await
    .expect("query kb_resource_standing axes")
}

/// **The acceptance criterion: the standing clock fires on a resource CREATE.**
///
/// The create commits, then `tick_resource_standing` refreshes the finding's standing memo as a side
/// effect — with no explicit refresh call anywhere in this test. If the wiring were missing, no
/// `kb_resource_standing` row would exist for the new resource.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_resource_create_ticks_the_standing_memo(pool: PgPool) {
    let (profile, context) = seed_profile_with_context(&pool, "standing-clock@example.com").await;
    let backend = DbBackend::new(pool.clone(), ProfileId::from(profile));

    let created = backend
        .create_resource(CreateResource {
            idempotency_key: None,
            slug: "zz-standing-probe".to_string(),
            doctype: "research".to_string(),
            home: HomeAnchor::Context(ContextId::from(context)),
            title: "ZZ standing probe".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            goal: None,
            origin_uri: Some("test://standing-probe".to_string()),
            chunks_packed: Some(one_chunk_packed("first segment", "aa")),
            content_hash: None,
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("create resource")
        .value;

    let memo_rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_resource_standing WHERE finding_id = $1")
            .bind(created.id.uuid())
            .fetch_one(&pool)
            .await
            .expect("query kb_resource_standing");

    assert_eq!(
        memo_rows, 1,
        "the create-site standing clock must have UPSERTed exactly one memo row for the new finding"
    );

    // A fresh create carries no reinforcement provenance, so r_parent (the breadth term, a count of
    // block_provenance rows) is expected to be 0 — the memo exists, it simply has nothing to count
    // yet. This asserts the row is materialized with the expected zero baseline, not that provenance
    // was seeded (that is Task 5's substrate-level concern).
    let r_parent: f64 =
        sqlx::query_scalar("SELECT r_parent FROM kb_resource_standing WHERE finding_id = $1")
            .bind(created.id.uuid())
            .fetch_one(&pool)
            .await
            .expect("query r_parent");
    assert_eq!(
        r_parent, 0.0,
        "a create with no reinforcement provenance leaves r_parent at its zero baseline"
    );
}

/// **The Set 5 acceptance criterion: recording a citation audit ticks the standing clock.**
///
/// The fixture is built so the assertions cannot pass vacuously. The finding is created WITH a
/// resource-kind citation of a real, live source resource, so the create-site tick already memoizes
/// `citation_magnitude = 1`: the `citation_quality == 0.0` asserted BEFORE the audit therefore means
/// *"one citation, nobody has evaluated it"*, not *"empty fixture, every axis sits at its column
/// default"* — a distinction a `0.0`-only assertion could not make. And the AFTER assertion is
/// unreachable for a broken implementation: `citation_quality` is written into
/// `kb_resource_standing` by nothing except `refresh_resource_standing`, and
/// `resource_citation_quality` can only return non-zero if an audit row actually exists. Drop the
/// write and it stays 0.0; drop the tick and it stays 0.0.
///
/// The auditor is a SECOND profile holding a read-only grant AND a `kb_machine_clients`
/// registration, because `AuditAuthority` refuses the finding's author (spec §7's self-audit
/// denial) and refuses any principal that is not a registered machine (spec §7's other conjunct).
/// A single-profile version of this test, or one whose auditor is an unregistered human, would 404
/// instead of auditing — which is the gate working, not the clock.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn recording_an_audit_moves_quality_off_zero(pool: PgPool) {
    let (citer, context) = seed_profile_with_context(&pool, "citer@example.com").await;
    let auditor = seed_profile(&pool, "auditor@example.com").await;
    let citer_backend = DbBackend::new(pool.clone(), ProfileId::from(citer));

    // The cited source must be a live `kb_resources` row: `resource_live_citations` joins
    // `kb_resources src ON src.id = p.source_id AND src.is_active`
    // (`20260724000120_standing_citation_components.sql:100-106`), so a synthetic uuid would be
    // counted by no axis and the "after" assertion could never move.
    let source = citer_backend
        .create_resource(create_cmd(
            context,
            "zz-cited-source",
            "ZZ cited source",
            "cc",
            Vec::new(),
        ))
        .await
        .expect("create the cited source")
        .value;

    let finding = citer_backend
        .create_resource(create_cmd(
            context,
            "zz-citing-finding",
            "ZZ citing finding",
            "ff",
            vec![ProvenanceSource::Resource(source.id.uuid())],
        ))
        .await
        .expect("create the citing finding")
        .value;

    // Read WITHOUT write — the shape the whole feature depends on existing. `AuditAuthority` admits
    // a principal who can read the finding and cannot modify it, and refuses one who can
    // (`authz/audit_gate.rs`); the grants table's coherence CHECK permits read-only.
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (subject_table, subject_id, principal_table, principal_id, can_read, can_write, \
            granted_by_profile_id) \
         VALUES ('kb_resources', $1, 'kb_profiles', $2, true, false, $3)",
    )
    .bind(finding.id.uuid())
    .bind(auditor)
    .bind(citer)
    .execute(&pool)
    .await
    .expect("grant the auditor read on the finding");

    // An audit is an AGENT act (spec §7, §5.2): the write gate admits only a registered, unrevoked
    // machine principal. One allowlist row — no provisioning side effects, since the reach this
    // test needs is the grant above.
    sqlx::query(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ($1, $1, $2, $2)",
    )
    .bind(format!("auditor-{auditor}@clients"))
    .bind(auditor)
    .execute(&pool)
    .await
    .expect("register the auditor as a machine principal");

    let block: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1 AND NOT is_folded",
    )
    .bind(finding.id.uuid())
    .fetch_one(&pool)
    .await
    .expect("the finding's body block");

    let before = read_memo_axes(&pool, finding.id.uuid()).await;
    assert_eq!(
        before.citation_magnitude, 1,
        "the create landed a live resource-kind citation and the create-site clock counted it — \
         this is what makes the 0.0 below meaningful rather than vacuous"
    );
    assert_eq!(
        before.audit_coverage, 0,
        "nobody has evaluated that citation yet"
    );
    assert_eq!(
        before.citation_quality, 0.0,
        "an unaudited citation makes no quality claim (spec §3.2), so quality reads neutral 0.0"
    );

    let auditor_backend = DbBackend::new(pool.clone(), ProfileId::from(auditor));
    let audit = auditor_backend
        .record_citation_audit(RecordCitationAudit {
            block: BlockId::from(block),
            source: ProvenanceSource::Resource(source.id.uuid()),
            value: 0.8,
            reason: Some("the source states the claim directly".to_string()),
            act: ActContext::default(),
            origin: Surface::ApiHttp,
        })
        .await
        .expect("a reader who did not author the finding may audit its citation")
        .value;
    assert!(!audit.is_nil(), "the ledger row id is returned");

    let after = read_memo_axes(&pool, finding.id.uuid()).await;
    assert_eq!(
        after.citation_magnitude, 1,
        "auditing a citation does not create one — magnitude is unmoved"
    );
    assert_eq!(
        after.audit_coverage, 1,
        "the single cited source now carries an audit"
    );
    assert!(
        after.citation_quality > 0.0,
        "the audit write must have ticked the clock: quality moved off the 0.0 asserted above, and \
         only refresh_resource_standing writes this column"
    );
    assert!(
        (after.citation_quality - 0.8).abs() < 1e-6,
        "a lone audit reads at its own value — its decay weight cancels in its own per-source mean \
         (spec §3.1); got {}",
        after.citation_quality
    );
}
