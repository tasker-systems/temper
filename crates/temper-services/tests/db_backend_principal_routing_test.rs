#![cfg(feature = "test-db")]
//! The DbBackend caller-principal routing pin (single-ingress follow-on, 2026-09-29).
//!
//! The seam's Principal-consuming gates dispatch through `DbBackend::principal`, which routes
//! `with_proof`-constructed callers to the classified arm (`Principal::Human` /
//! `Principal::Machine`) their `Caller` carries, and test-harness `new`-constructed ones to
//! `Principal::Bare`. This file pins the ROUTING, not the SQL predicates (the predicates are the
//! gates' own unit suites' subject — `authz/audit_gate.rs`, `services/machine_authz.rs`). Two
//! properties matter:
//!
//! 1. **A classified caller reaches the same gate decision the bare id does.** The witness
//!    drives the real audit write through BOTH spellings for the same caller and demands the
//!    same admission — routing must change which arm the gate receives, never the gate's
//!    answer on these predicates.
//! 2. **The test-harness spelling keeps its posture.** `new` mints, fakes and implies no proof;
//!    the second call below IS that spelling proving it still admits on the same predicates.
//!    It exists only under `test-harness` — no production path builds one.
//!
//! Fixture convention cribbed per this tier's rule from `audit_gate_folded_test.rs` (the
//! bootseed + write-path fixture geometry; `seed_auditor`'s surface-emitter entities — the
//! write path's `resolve_emitter` needs them).
//!
//! ONNX-dependent (the create fixture server-embeds). Isolated ephemeral DB via
//! `temper_substrate::MIGRATOR`.

use sqlx::PgPool;
use temper_core::types::authorship::ActContext;
use temper_core::types::ids::{BlockId, CogmapId, EntityId, ProfileId, ResourceId};
use temper_core::types::provenance::ProvenanceSource;
use temper_services::auth::Caller;
use temper_services::backend::DbBackend;
use temper_substrate::payloads::{AnchorRef, Incorporation};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, AppendParams, CreateMode, CreateParams, FinalizeParams};
use temper_workflow::operations::{Backend, RecordCitationAudit, Surface};
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
            title: "principal-routing fixture",
            origin_uri: "temper://principal-routing/fixture",
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
    writes::finalize_ingest(
        pool,
        FinalizeParams {
            resource,
            expected_blocks: 2,
            expected_body_hash: temper_substrate::content::body_hash_from_block_chunk_hashes(&[
                temper_ingest::chunk::chunk_markdown(first)
                    .iter()
                    .map(|c| c.content_hash.clone())
                    .collect::<Vec<_>>(),
                temper_ingest::chunk::chunk_markdown(rest)
                    .iter()
                    .map(|c| c.content_hash.clone())
                    .collect::<Vec<_>>(),
            ]),
            expected_content_hash: None,
            emitter,
        },
    )
    .await
    .unwrap();
    resource
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

async fn seed_source_resource(pool: &PgPool) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('a source', '') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

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

fn audit_cmd(block: Uuid, source: Uuid) -> RecordCitationAudit {
    RecordCitationAudit {
        block: BlockId::from(block),
        source: ProvenanceSource::Resource(source),
        value: 0.5,
        reason: None,
        act: ActContext::default(),
        origin: Surface::ApiHttp,
    }
}

/// A cogmap the auditor can read, with its own citation-audit job in flight, claimed by the
/// auditor. Fixture-shaped (raw INSERTs, the reach the machinery needs, no provisioning side
/// effects): team membership is what `anchor_readable_by_profile` reads (so
/// `AuditorJobAuthority` admits the caller as `Auditor`), and the claim's reach scope
/// (`steward_candidate_cogmaps` → readable cogmaps) is what let the auditor enqueue+claim the
/// row it completes. The machine-conjunct row itself is seeded by the test — one allowlist row,
/// per `standing_clock_test.rs`'s fixture spelling.
async fn seed_auditor_job(pool: &PgPool, auditor_profile: ProfileId) -> CogmapId {
    use temper_core::types::workflow_job::{DispatchType, Persona};
    use temper_services::services::workflow_job_service;

    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('telos', '') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let cogmap: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('routing-pin-map', $1) RETURNING id",
    )
    .bind(telos)
    .fetch_one(pool)
    .await
    .unwrap();
    let team: Uuid =
        sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
            .bind(format!("job-team-{}", &cogmap.simple().to_string()[..8]))
            .fetch_one(pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(auditor_profile.uuid())
    .execute(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .unwrap();

    // One job for the cogmap, claimed by the auditor — `complete_claimed` transitions only a row
    // this principal claimed, so the completion below is that row's own owner completing it.
    workflow_job_service::enqueue(
        pool,
        cogmap,
        Persona::Auditor.as_str(),
        DispatchType::CitationAudit.as_str(),
    )
    .await
    .unwrap();
    let claimed = workflow_job_service::claim(
        pool,
        Persona::Auditor.as_str(),
        DispatchType::CitationAudit.as_str(),
        10,
        600,
        None,
        auditor_profile,
    )
    .await
    .unwrap();
    assert_eq!(claimed.len(), 1, "the auditor claims its own enqueued job");
    CogmapId::from(cogmap)
}

/// THE SECOND ROUTING PIN, on the machine-principal job door (`complete_auditor_job`): the same
/// caller, both constructor spellings — the classified arm its middleware minted and the
/// test-harness `Bare` arm. The completion must land under both, and the ledger-row count forbids
/// either spelling from widening or demoting the gate's answer for this caller.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn proof_holding_and_bare_spellings_complete_the_same_auditor_job(pool: sqlx::PgPool) {
    use temper_workflow::operations::CompleteAuditorJob;

    bootseed::seed_system(&pool).await.unwrap();
    let auditor = seeded_authed(&pool, "routing-job-auditor").await;
    // The machine-conjunct registration: without it both spellings refuse as NotMachine, which
    // would pin the refusal, not the routing.
    sqlx::query(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ($1, $1, $2, $2)",
    )
    .bind(format!("routing-pin-{}", Uuid::now_v7()))
    .bind(auditor.profile_id().uuid())
    .execute(&pool)
    .await
    .unwrap();
    let cogmap = seed_auditor_job(&pool, auditor.profile_id()).await;
    let complete_cmd = CompleteAuditorJob {
        cogmap,
        origin: Surface::ApiHttp,
    };

    // Arm 1 — the proof-holding spelling (what every HTTP handler and MCP tool builds).
    let proof_backend = DbBackend::with_proof(pool.clone(), &auditor);
    proof_backend
        .complete_auditor_job(complete_cmd.clone())
        .await
        .expect("the proof-holding caller's gate dispatch completes its job");

    // Arm 2 — the SAME caller through the test-harness spelling (`new`): the `Bare` arm keeps
    // the gate's identical admission. The first call already finished the caller's only job
    // (`workflow_job_complete_claimed` is single-flight), so this exercises the gate's ADMIT
    // and the no-op `None` return — both spellings must pass the gate identically rather than
    // one being refused where the other was admitted.
    let bare_backend = DbBackend::new(pool.clone(), auditor.profile_id());
    let second = bare_backend
        .complete_auditor_job(complete_cmd)
        .await
        .expect("the same caller through the bare spelling is admitted by the same gate");
    assert!(
        second.value.is_none(),
        "the bare spelling is admissible but owns nothing left in flight — the gate, not the queue, is under test"
    );
}

/// Mint a real classified caller for a seeded profile — the exact thing `with_proof` accepts and
/// the HTTP middleware produces. Same shape as `system_admin_proof_test.rs`'s helper.
async fn seeded_authed(pool: &PgPool, handle: &str) -> Caller {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1,$1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .unwrap();
    // The surface emitter entities the audit write's resolve_emitter resolves.
    for surface in ["web", "cli", "mcp"] {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1, $2, '{}'::jsonb)",
        )
        .bind(id)
        .bind(format!("{handle}@{surface}"))
        .execute(pool)
        .await
        .unwrap();
    }
    temper_services::test_support::caller_for(pool, id).await
}

/// THE ROUTING PIN, on the real audit door: the same caller, the same gate, both constructor
/// spellings. `with_proof` dispatches the classified arm its middleware minted; `new` dispatches
/// `Bare` — and for this caller both must land the identical admission, because the spelling
/// changes the arm the gate receives, never the gate's answer. The second call is also the one
/// place the `Bare` spelling is exercised end-to-end at the seam beside its unit suites.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn proof_holding_and_bare_spellings_reach_the_same_gate_decision(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (author, emitter) = system_actor(&pool).await;
    let source = seed_source_resource(&pool).await;
    let finding = create_two_block(
        &pool,
        author,
        emitter,
        &make_home(&pool, author, "principal-routing").await,
        SECTION_A,
        SECTION_B,
        vec![Incorporation {
            source: ProvenanceSource::Resource(source),
            seq: 0,
        }],
    )
    .await;
    let (cited_block, _) = blocks_of(&pool, finding).await[0];
    let auditor = seeded_authed(&pool, "routing-auditor").await;
    grant_read(&pool, finding.uuid(), auditor.profile_id(), author).await;

    // Arm 1 — the proof-holding spelling (what every HTTP handler and MCP tool builds).
    let proof_backend = DbBackend::with_proof(pool.clone(), &auditor);
    proof_backend
        .record_citation_audit(audit_cmd(cited_block, source))
        .await
        .expect("the proof-holding caller's gate dispatch lands the audit");

    // Arm 2 — the SAME caller through the test-harness spelling (`new`): the `Bare` arm keeps
    // the gate's identical admission, unchanged from before the refactor.
    let bare_backend = DbBackend::new(pool.clone(), auditor.profile_id());
    bare_backend
        .record_citation_audit(audit_cmd(cited_block, source))
        .await
        .expect("the same caller through the bare spelling keeps the gate's admission");

    let rows: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_citation_audits WHERE block_id = $1")
            .bind(cited_block)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        rows, 2,
        "both spellings landed exactly one audit each — the routing never widened the gate"
    );
}
