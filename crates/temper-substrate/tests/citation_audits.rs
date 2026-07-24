#![cfg(feature = "artifact-tests")]
//! Set 5, Task 1 — the append-only citation-audit table, event, and write path (SQL) — and
//! Task 2, the Rust write path on top of it (`writes::record_citation_audit[_with]`).
//!
//! Task 1's tests exercise `citation_audit` / `_project_citation_audited` in
//! `migrations/20260724000110_citation_audits.sql` directly against an ephemeral DB — the same
//! idiom `content_mutation.rs` uses for `block_mutate` (`content_mutation.rs:91-92`:
//! `serde_json::json!` payload, positional `$1`/`$2` binds) — since there was no Rust wrapper
//! yet. Task 2's tests (bottom of file) drive the same SQL through the typed
//! `writes::record_citation_audit[_with]` wrapper instead.
//!
//! Harness + seeding helpers are local copies of `tests/evidential_standing.rs`'s
//! `system_actor`/`make_home`/`make_resource` (duplicated per file across this test suite by
//! established convention — `block_content.rs`, `invocation_envelope.rs`, `search_index.rs`,
//! `search_surface_a.rs`, and `write_path_mutations.rs` each carry their own copy).

mod common;

use temper_substrate::events::EventContext;
use temper_substrate::ids::{
    BlockId, CogmapId, ContextId, EntityId, InvocationId, ProfileId, ResourceId,
};
use temper_substrate::payloads::{
    AgentAuthorship, AnchorRef, ConfidenceBand, Incorporation, ProvenanceSource,
};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, CitationAuditParams, CreateParams};
use uuid::Uuid;

// ── fixtures ──────────────────────────────────────────────────────────────────────────────────

/// The canonical `system` actor as typed newtypes (pattern from `evidential_standing.rs:25-37`).
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

async fn make_home(pool: &sqlx::PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

async fn make_resource(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: ContextId,
    title: &str,
    uri: &str,
) -> ResourceId {
    writes::create_resource_with(
        pool,
        CreateParams {
            title,
            origin_uri: uri,
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

/// The first live (non-folded) content block of a resource — the citation grain a
/// `citation_audit` targets. Plain `Uuid` (not the typed `BlockId`) because these tests drive
/// the raw SQL entry function directly, mirroring `content_mutation.rs`'s `block_id: uuid::Uuid`
/// (e.g. `content_mutation.rs:79-83`), not a typed Rust write-path wrapper.
async fn first_block(pool: &sqlx::PgPool, resource: ResourceId) -> Uuid {
    sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded ORDER BY seq LIMIT 1",
    )
    .bind(resource.uuid())
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Fire `citation_audit(payload, emitter)` with a `resource`-kind citation, returning
/// the audit id it reports (or the DB error, for the rejection tests).
async fn fire_audit(
    pool: &sqlx::PgPool,
    emitter: EntityId,
    block: Uuid,
    source: Uuid,
    value: f64,
) -> Result<Uuid, sqlx::Error> {
    let payload = serde_json::json!({
        "block_id": block,
        "source": { "kind": "resource", "value": source },
        "value": value,
        "reason": "test audit",
    });
    sqlx::query_scalar::<_, Uuid>("SELECT citation_audit($1, $2)")
        .bind(&payload)
        .bind(emitter.uuid())
        .fetch_one(pool)
        .await
}

/// Make `(block, source)` a LIVE citation.
///
/// `citation_audit` refuses to audit a pair that is not one (`citation_is_live`,
/// `20260724000110_citation_audits.sql`) — an audit of a non-citation is inert for standing, so
/// accepting it would be a silently successful no-op the auditor could never detect. `make_resource`
/// above creates with no sources, so every valid-audit fixture in this half of the file needs this
/// row; the Task 5 fixtures below get theirs from `create_resource_with`'s `sources` instead.
///
/// Borrows the block's own genesis event as the contributing act — nothing under test reads that
/// event's content, the same shortcut `first_block`'s callers already take.
async fn cite(pool: &sqlx::PgPool, block: Uuid, source: Uuid) {
    sqlx::query(
        "INSERT INTO kb_block_provenance \
           (block_id, source_kind, source_id, contributed_by_event_id, accretion_seq) \
         SELECT $1, 'resource', $2, b.genesis_event_id, 0 FROM kb_content_blocks b WHERE b.id = $1",
    )
    .bind(block)
    .bind(source)
    .execute(pool)
    .await
    .unwrap();
}

// ── Task 1 — citation_audit / _project_citation_audited ─────────────────────────────────────────

/// LOAD-BEARING: falsifies "the projector returns the wrong id." The `block_annotate` sibling
/// returns the block id; a copy-paste of that shape would make this function useless to any
/// caller that needs to name the audit row (Task 3's decay aggregation, a future retraction-style
/// read). Asserts the returned uuid addresses a REAL `kb_citation_audits` row for the same block.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn citation_audit_inserts_a_row_and_returns_its_audit_id(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "ca-insert").await;
    let finding = make_resource(
        &pool,
        owner,
        emitter,
        home,
        "finding",
        "temper://ca/finding1",
    )
    .await;
    let source = make_resource(&pool, owner, emitter, home, "source", "temper://ca/source1").await;
    let block = first_block(&pool, finding).await;
    cite(&pool, block, source.uuid()).await;

    let audit_id = fire_audit(&pool, emitter, block, source.uuid(), 0.6)
        .await
        .unwrap();

    assert_ne!(
        audit_id, block,
        "citation_audit must return the audit row's own id, not the block id"
    );
    let row_block: Uuid = sqlx::query_scalar("SELECT block_id FROM kb_citation_audits WHERE id=$1")
        .bind(audit_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        row_block, block,
        "the returned id must address a real kb_citation_audits row for THIS block"
    );
}

/// LOAD-BEARING: falsifies "audits supersede." Fires two audits of the exact same citation with
/// different (indeed opposite-signed) values and asserts BOTH rows persist — the direct proof
/// there is no `is_superseded` bit and no unique-on-citation-key collapsing them, per spec §4.1's
/// "a later +1.0 never erases an earlier -1.0."
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn two_audits_of_one_citation_both_persist(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "ca-dup").await;
    let finding = make_resource(
        &pool,
        owner,
        emitter,
        home,
        "finding",
        "temper://ca/finding2",
    )
    .await;
    let source = make_resource(&pool, owner, emitter, home, "source", "temper://ca/source2").await;
    let block = first_block(&pool, finding).await;
    cite(&pool, block, source.uuid()).await;

    fire_audit(&pool, emitter, block, source.uuid(), 0.9)
        .await
        .unwrap();
    fire_audit(&pool, emitter, block, source.uuid(), -0.4)
        .await
        .unwrap();

    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_citation_audits \
         WHERE block_id=$1 AND source_kind='resource' AND source_id=$2",
    )
    .bind(block)
    .bind(source.uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        count, 2,
        "append-only: two audits of one citation both persist, no supersession"
    );
}

/// Falsifies "the signed-range CHECK is missing or app-level only." `1.5` is out of `[-1,1]`;
/// the table's CHECK constraint (not a plpgsql guard) must reject the insert, so the whole
/// `citation_audit` call fails atomically (no event, no row) rather than clamping or silently
/// accepting an out-of-range verdict.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn citation_audit_rejects_a_value_outside_the_signed_range(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "ca-range").await;
    let finding = make_resource(
        &pool,
        owner,
        emitter,
        home,
        "finding",
        "temper://ca/finding3",
    )
    .await;
    let source = make_resource(&pool, owner, emitter, home, "source", "temper://ca/source3").await;
    let block = first_block(&pool, finding).await;
    // A real citation, so the CHECK on `value` is unambiguously what rejects this — not the
    // citation-existence guard that runs before it.
    cite(&pool, block, source.uuid()).await;

    let err = fire_audit(&pool, emitter, block, source.uuid(), 1.5)
        .await
        .expect_err("value outside [-1,1] must raise, not silently clamp or insert");
    assert!(
        err.to_string().to_lowercase().contains("check constraint"),
        "expected a CHECK-constraint violation on `value`, got: {err}"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_citation_audits")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        count, 0,
        "a rejected audit must leave no row — the failed INSERT rolls back the whole call, \
         including the event it would have appended"
    );
}

/// LOAD-BEARING: falsifies "any provenance_source_kind is auditable." `remote` is a genuinely
/// valid enum member (`20260704000006_remote_source_kind_enum.sql:8`), so this is not testing an
/// impossible input — it is testing that the entry function's explicit guard (spec §6.2: standing
/// only reads resource-kind bases) fires BEFORE an insert that would otherwise silently succeed
/// and become an audit the auditor can never see reflected in standing.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn citation_audit_rejects_a_remote_source_kind(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "ca-remote").await;
    let finding = make_resource(
        &pool,
        owner,
        emitter,
        home,
        "finding",
        "temper://ca/finding4",
    )
    .await;
    let block = first_block(&pool, finding).await;

    let payload = serde_json::json!({
        "block_id": block,
        "source": { "kind": "remote", "value": "https://example.invalid/doc" },
        "value": 0.5,
        "reason": "test audit",
    });
    let err = sqlx::query_scalar::<_, Uuid>("SELECT citation_audit($1, $2)")
        .bind(&payload)
        .bind(emitter.uuid())
        .fetch_one(&pool)
        .await
        .expect_err(
            "a remote-kind citation must raise: standing only reads resource-kind bases (spec §6.2)",
        );
    assert!(
        err.to_string().to_lowercase().contains("resource-kind"),
        "expected the entry function's named guard, got: {err}"
    );
}

/// LOAD-BEARING: falsifies the exact trap the brief calls out — that
/// `INSERT ... ON CONFLICT DO NOTHING RETURNING id` yields NO row on the replay path, so a
/// projector without the fallback SELECT would return NULL on replay instead of the existing
/// audit id. Re-projects the SAME event id directly (the shape a real replay takes: the event
/// already exists in `kb_events`, only `_project_citation_audited` runs again) and asserts both
/// the returned id and the row count are unchanged.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn citation_audit_is_idempotent_under_replay(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "ca-replay").await;
    let finding = make_resource(
        &pool,
        owner,
        emitter,
        home,
        "finding",
        "temper://ca/finding5",
    )
    .await;
    let source = make_resource(&pool, owner, emitter, home, "source", "temper://ca/source5").await;
    let block = first_block(&pool, finding).await;
    cite(&pool, block, source.uuid()).await;

    let audit_id = fire_audit(&pool, emitter, block, source.uuid(), 0.3)
        .await
        .unwrap();
    let event_id: Uuid =
        sqlx::query_scalar("SELECT audited_by_event_id FROM kb_citation_audits WHERE id=$1")
            .bind(audit_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    let payload = serde_json::json!({
        "block_id": block,
        "source": { "kind": "resource", "value": source.uuid() },
        "value": 0.3,
        "reason": "test audit",
    });
    let replayed: Uuid = sqlx::query_scalar("SELECT _project_citation_audited($1, $2)")
        .bind(event_id)
        .bind(&payload)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(
        replayed, audit_id,
        "replaying the same event must return the SAME audit id, not NULL"
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_citation_audits WHERE audited_by_event_id=$1")
            .bind(event_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        count, 1,
        "replay must not create a second row for the same event"
    );
}

/// Falsifies "an unknown block is a silent no-op." Mirrors `block_annotate`'s own guard
/// (`20260710000001_block_provenance_annotate.sql:52-54`) — an audit against a block that does
/// not exist must raise, not write an orphaned or nonsensical row.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn citation_audit_raises_for_an_unknown_block(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (_owner, emitter) = system_actor(&pool).await;

    let unknown_block = Uuid::now_v7();
    let err = fire_audit(&pool, emitter, unknown_block, Uuid::now_v7(), 0.5)
        .await
        .expect_err("an unknown block must raise, not silently no-op");
    assert!(
        err.to_string().to_lowercase().contains("not found"),
        "expected the entry function's named guard, got: {err}"
    );
}

/// LOAD-BEARING: falsifies "an audit of a NON-citation is accepted." The block exists, the source
/// exists and is resource-kind, and the only thing wrong is that the block never cited it — the
/// shape a one-row transposition while iterating `get_block_provenance` produces. Before the guard
/// this returned an audit id with HTTP 200 while moving nothing: both `resource_audit_coverage` and
/// `resource_citation_quality` join through `resource_live_citations` on the full citation key, so
/// the row is permanently inert, the finding re-heads `audit_drift_sweep` with the identical
/// `uncovered` count on every subsequent tick, and the auditor is never told because it succeeded.
///
/// Deliberately paired with a source that IS cited on a DIFFERENT block, so "the source does not
/// exist" cannot be the reason.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn citation_audit_rejects_a_source_that_is_not_a_citation_of_the_block(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "ca-noncite").await;
    let cited = make_resource(&pool, owner, emitter, home, "cited", "temper://ca/cited").await;
    let elsewhere = make_resource(
        &pool,
        owner,
        emitter,
        home,
        "elsewhere",
        "temper://ca/elsewhere",
    )
    .await;
    let source = make_resource(&pool, owner, emitter, home, "source", "temper://ca/nc-src").await;

    // The source is a real, live citation — of the OTHER finding's block.
    cite(&pool, first_block(&pool, elsewhere).await, source.uuid()).await;
    let block = first_block(&pool, cited).await;

    let err = fire_audit(&pool, emitter, block, source.uuid(), 0.5)
        .await
        .expect_err("auditing a (block, source) pair that is not a citation must raise");
    assert!(
        err.to_string()
            .to_lowercase()
            .contains("not a live citation"),
        "expected the entry function's named guard, got: {err}"
    );

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_citation_audits")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        count, 0,
        "the rejected audit must leave no row — a silently inert verdict is worse than an error"
    );
}

/// The corrected-provenance scar is not a live citation either. `is_corrected` is the "this source
/// was wrong" marker every incumbent citation reader excludes
/// (`20260721000010_evidential_standing_memo.sql:55,68,108`), so an audit of one would be inert in
/// exactly the same way — the guard must exclude it for the same reason the producers do.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn citation_audit_rejects_a_corrected_citation(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "ca-corrected").await;
    let finding = make_resource(&pool, owner, emitter, home, "finding", "temper://ca/corr").await;
    let source = make_resource(
        &pool,
        owner,
        emitter,
        home,
        "source",
        "temper://ca/corr-src",
    )
    .await;
    let block = first_block(&pool, finding).await;
    cite(&pool, block, source.uuid()).await;
    sqlx::query("UPDATE kb_block_provenance SET is_corrected = true WHERE block_id=$1")
        .bind(block)
        .execute(&pool)
        .await
        .unwrap();

    let err = fire_audit(&pool, emitter, block, source.uuid(), 0.5)
        .await
        .expect_err("a corrected citation is not live and must not be auditable");
    assert!(
        err.to_string()
            .to_lowercase()
            .contains("not a live citation"),
        "expected the entry function's named guard, got: {err}"
    );
}

// ── Task 2 — the Rust write path (writes::record_citation_audit[_with]) ─────────────────────────

/// LOAD-BEARING: falsifies "the Rust write path never reaches `_project_citation_audited`" — the
/// exact `resource_finalized`-shaped failure mode a missing `EventKind`/`SeedAction` wire-up
/// produces (compiles clean, hard-fails or silently no-ops at the SQL boundary). Fires through
/// `writes::record_citation_audit` (the typed Rust wrapper, not the raw SQL entry function Task
/// 1's tests call directly) and asserts a real `kb_citation_audits` row exists for the exact
/// block/source/value the caller supplied, addressed by the returned audit id.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn record_citation_audit_writes_a_row_and_returns_its_audit_id(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "ca-rust-insert").await;
    let finding = make_resource(
        &pool,
        owner,
        emitter,
        home,
        "finding",
        "temper://ca/rust-finding1",
    )
    .await;
    let source = make_resource(
        &pool,
        owner,
        emitter,
        home,
        "source",
        "temper://ca/rust-source1",
    )
    .await;
    let block = BlockId::from(first_block(&pool, finding).await);
    cite(&pool, block.uuid(), source.uuid()).await;

    let audit_id = writes::record_citation_audit(
        &pool,
        CitationAuditParams {
            block,
            source: ProvenanceSource::Resource(source.uuid()),
            value: 0.6,
            reason: Some("test audit via rust write path"),
            emitter,
        },
    )
    .await
    .unwrap();

    let (row_block, row_source_kind, row_source_id, row_value): (Uuid, String, Uuid, f64) =
        sqlx::query_as(
            "SELECT block_id, source_kind::text, source_id, value \
               FROM kb_citation_audits WHERE id=$1",
        )
        .bind(audit_id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        row_block,
        block.uuid(),
        "the row's block_id must match the audited block"
    );
    assert_eq!(
        row_source_kind, "resource",
        "the Rust write path must serialize ProvenanceSource::Resource as source_kind='resource'"
    );
    assert_eq!(
        row_source_id,
        source.uuid(),
        "the row's source_id must match the cited resource"
    );
    assert_eq!(
        row_value, 0.6,
        "the row's value must match the caller's verdict"
    );
}

/// LOAD-BEARING: falsifies "the auditor's own confidence in its verdict leaks into the payload
/// the projector reads" — spec §4.2's self-grading prohibition ("the projection never reads that
/// self-assessment... structural rather than procedural"). Fires with BOTH authorship and an
/// invocation attached and asserts: (1) `kb_events.invocation_id` is stamped, (2) authorship lives
/// in `kb_events.metadata`, and (3) neither `confidence` nor `reasoning` appears ANYWHERE in
/// `kb_events.payload` — the exact column the SQL projector (`_project_citation_audited`) reads.
/// Mirrors `act_authorship_projection_invisibility.rs`'s pattern, applied to this write path.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn record_citation_audit_with_stamps_the_invocation_and_authorship(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "ca-rust-authorship").await;
    let finding = make_resource(
        &pool,
        owner,
        emitter,
        home,
        "finding",
        "temper://ca/rust-finding2",
    )
    .await;
    let source = make_resource(
        &pool,
        owner,
        emitter,
        home,
        "source",
        "temper://ca/rust-source2",
    )
    .await;
    let block = BlockId::from(first_block(&pool, finding).await);
    cite(&pool, block.uuid(), source.uuid()).await;
    let invocation = InvocationId::from(Uuid::now_v7());

    let audit_id = writes::record_citation_audit_with(
        &pool,
        CitationAuditParams {
            block,
            source: ProvenanceSource::Resource(source.uuid()),
            value: -0.3,
            reason: Some("adversarial re-check"),
            emitter,
        },
        EventContext {
            authorship: Some(AgentAuthorship {
                reasoning: Some("citation looks fabricated".to_string()),
                confidence: ConfidenceBand::Confident,
                rationale: None,
                persona: Some("adversary".to_string()),
                model: None,
            }),
            invocation: Some(invocation),
            correlation: None,
        },
    )
    .await
    .unwrap();

    let event_id: Uuid =
        sqlx::query_scalar("SELECT audited_by_event_id FROM kb_citation_audits WHERE id=$1")
            .bind(audit_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let (metadata, got_invocation, payload): (serde_json::Value, Option<Uuid>, serde_json::Value) =
        sqlx::query_as("SELECT metadata, invocation_id, payload FROM kb_events WHERE id=$1")
            .bind(event_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(
        got_invocation,
        Some(invocation.uuid()),
        "invocation_id must be stamped on kb_events, not dropped"
    );
    assert_eq!(
        metadata["confidence"], "confident",
        "authorship confidence rides kb_events.metadata: {metadata}"
    );
    assert_eq!(
        metadata["reasoning"], "citation looks fabricated",
        "authorship reasoning rides kb_events.metadata: {metadata}"
    );
    assert!(
        payload.get("confidence").is_none() && payload.get("reasoning").is_none(),
        "the auditor's own confidence must NEVER leak into the payload the projector reads \
         (spec §4.2 self-grading prohibition): payload was {payload}"
    );
}

// ── Task 5 — audit_drift_sweep (SQL) ─────────────────────────────────────────────────────────────
//
// Spec `docs/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md` §6.2-6.3,
// migration `20260724000130_audit_drift_sweep.sql`. Every fixture below builds through the SAME
// production writes as the rest of this file (`create_resource_with` via `make_home`/`make_resource`,
// `citation_audit` via `fire_audit`, `writes::delete_resource`) plus the cogmap/team-membership
// fixtures already established in this test suite for exactly this purpose
// (`common::genesis_cogmap`, `common::create_team`, `common::create_profile`,
// `common::add_team_member`, and the raw `kb_team_cogmaps` insert precedented at
// `cogmap_shape_readback.rs:103-106`).
//
// Every assertion below checks PRESENCE/ABSENCE of a specific `(cogmap_id, finding_id)` pair in the
// sweep's rows, never the total row count — the L0 kernel cogmap
// (`20260625000001_l0_kernel_cogmap.sql`) and its root-team join are seeded into every test database
// by `MIGRATOR` regardless of `bootseed::seed_system`, so an assertion on total row count would be
// coupled to unrelated background seed data instead of the behavior under test.

/// Join a team to a cogmap (`kb_team_cogmaps`) — the row `cogmap_readable_by_profile`
/// (`20260624000002_canonical_functions.sql:259-267`) reads to decide reachability, and so what
/// `steward_candidate_cogmaps` (and therefore `audit_drift_sweep`) admits. Raw insert; there is no
/// typed Rust wrapper for this one row — mirrors the identical fixture insert already used for this
/// purpose elsewhere in this test suite (`cogmap_shape_readback.rs:103-106`).
async fn join_team_to_cogmap(pool: &sqlx::PgPool, team: Uuid, cogmap: Uuid) {
    sqlx::query("INSERT INTO kb_team_cogmaps (team_id, cogmap_id) VALUES ($1, $2)")
        .bind(team)
        .bind(cogmap)
        .execute(pool)
        .await
        .unwrap();
}

/// Make `member` a principal who can reach `cogmap` through `steward_candidate_cogmaps`: a fresh
/// team, joined to the cogmap, with `member` added to it as a `'member'`. This is the ONLY path
/// `cogmap_readable_by_profile` recognizes — direct membership in a team that is ITSELF joined to the
/// cogmap (`profile_effective_teams`, not team-hierarchy inheritance), so this fixture is deliberately
/// a fresh team rather than reusing any ambient membership.
async fn join_principal_to_cogmap(
    pool: &sqlx::PgPool,
    member: Uuid,
    cogmap: Uuid,
    team_slug: &str,
) {
    let team = common::create_team(pool, team_slug).await;
    common::add_team_member(pool, team, member).await;
    join_team_to_cogmap(pool, team, cogmap).await;
}

/// A cogmap-homed finding (spec §6.2's boundary) optionally citing one `source` at create time.
/// Mirrors `make_resource` above but homes `AnchorRef::cogmap(cogmap)` instead of a context.
async fn make_cogmap_finding(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    emitter: EntityId,
    cogmap: CogmapId,
    title: &str,
    uri: &str,
    source: Option<ResourceId>,
) -> ResourceId {
    let sources = match source {
        Some(s) => vec![Incorporation {
            source: ProvenanceSource::Resource(s.uuid()),
            seq: 0,
        }],
        None => vec![],
    };
    writes::create_resource_with(
        pool,
        CreateParams {
            title,
            origin_uri: uri,
            body: "seed body",
            doc_type: "research",
            home: AnchorRef::cogmap(cogmap),
            owner,
            originator: owner,
            emitter,
            properties: &[],
            chunks: None,
            sources,
        },
        EventContext::default(),
    )
    .await
    .unwrap()
}

/// A cogmap-homed finding citing `n` distinct, unaudited sources all at once — `magnitude = n`,
/// `coverage = 0`. Multiple `Incorporation`s in a single create is a supported shape (mirrors the
/// two-source create in `evidential_standing.rs`'s
/// `a_source_cited_by_two_blocks_counts_once_in_quality`), used here only by the ordering test, which
/// needs one finding with more than one uncovered source.
/// The eight things that finding needs. A params struct rather than eight positional arguments:
/// the repo treats `#[expect(clippy::too_many_arguments)]` as a smell to fix, not to suppress, and
/// four of these are `&str`/id pairs that positional calls would happily transpose in silence.
struct CogmapFindingSeed<'a> {
    owner: ProfileId,
    emitter: EntityId,
    cogmap: CogmapId,
    home_slug: &'a str,
    title: &'a str,
    uri: &'a str,
    /// How many distinct sources the finding cites.
    n: usize,
}

async fn seed_cogmap_finding_with_n_citations(
    pool: &sqlx::PgPool,
    p: CogmapFindingSeed<'_>,
) -> ResourceId {
    let src_home = make_home(pool, p.owner, p.home_slug).await;
    let mut sources = Vec::with_capacity(p.n);
    for i in 0..p.n {
        let src = make_resource(
            pool,
            p.owner,
            p.emitter,
            src_home,
            &format!("{}-src{i}", p.title),
            &format!("{}-src{i}", p.uri),
        )
        .await;
        sources.push(Incorporation {
            source: ProvenanceSource::Resource(src.uuid()),
            seq: i as i32,
        });
    }
    writes::create_resource_with(
        pool,
        CreateParams {
            title: p.title,
            origin_uri: p.uri,
            body: "seed body",
            doc_type: "research",
            home: AnchorRef::cogmap(p.cogmap),
            owner: p.owner,
            originator: p.owner,
            emitter: p.emitter,
            properties: &[],
            chunks: None,
            sources,
        },
        EventContext::default(),
    )
    .await
    .unwrap()
}

/// Run `audit_drift_sweep(principal, limit)`, returning the raw `(cogmap_id, finding_id, uncovered)`
/// rows at the shape the migration declares (`20260724000130_audit_drift_sweep.sql`).
async fn sweep(pool: &sqlx::PgPool, principal: Uuid, limit: i32) -> Vec<(Uuid, Uuid, i32)> {
    sqlx::query_as("SELECT cogmap_id, finding_id, uncovered FROM audit_drift_sweep($1, $2)")
        .bind(principal)
        .bind(limit)
        .fetch_all(pool)
        .await
        .unwrap()
}

/// LOAD-BEARING. Falsifies "the sweep returns nothing" and "the sweep miscomputes `uncovered`". A
/// cogmap-homed finding citing one live, unaudited source has `magnitude=1, coverage=0`, so it must
/// appear with `uncovered=1` for a principal whose team is joined to the cogmap.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn sweep_returns_a_finding_with_uncovered_citations(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let (cogmap, _telos) = common::genesis_cogmap(&pool, "ads-basic", "ADS Basic Telos").await;
    let principal = common::create_profile(&pool, "ads-basic-principal@example.com").await;
    join_principal_to_cogmap(&pool, principal, cogmap, "ads-basic-team").await;

    let src_home = make_home(&pool, owner, "ads-basic-src-home").await;
    let source = make_resource(
        &pool,
        owner,
        emitter,
        src_home,
        "source",
        "temper://ads/basic-source",
    )
    .await;
    let finding = make_cogmap_finding(
        &pool,
        owner,
        emitter,
        CogmapId::from(cogmap),
        "finding",
        "temper://ads/basic-finding",
        Some(source),
    )
    .await;

    let rows = sweep(&pool, principal, 10).await;
    let hit = rows
        .iter()
        .find(|(c, f, _)| *c == cogmap && *f == finding.uuid())
        .expect(
            "the uncovered finding must appear in the sweep for a principal who can read its cogmap",
        );
    assert_eq!(hit.2, 1, "magnitude 1, coverage 0 => uncovered 1");
}

/// LOAD-BEARING. Falsifies "the coverage predicate is missing or inverted." Once the finding's only
/// source is audited, `coverage == magnitude` and the finding must drop out of the queue — spec
/// §6.3's actual selection predicate is incomplete coverage, not mere presence of a citation.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn sweep_omits_a_fully_covered_finding(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let (cogmap, _telos) = common::genesis_cogmap(&pool, "ads-covered", "ADS Covered Telos").await;
    let principal = common::create_profile(&pool, "ads-covered-principal@example.com").await;
    join_principal_to_cogmap(&pool, principal, cogmap, "ads-covered-team").await;

    let src_home = make_home(&pool, owner, "ads-covered-src-home").await;
    let source = make_resource(
        &pool,
        owner,
        emitter,
        src_home,
        "source",
        "temper://ads/covered-source",
    )
    .await;
    let finding = make_cogmap_finding(
        &pool,
        owner,
        emitter,
        CogmapId::from(cogmap),
        "finding",
        "temper://ads/covered-finding",
        Some(source),
    )
    .await;
    let block = first_block(&pool, finding).await;
    fire_audit(&pool, emitter, block, source.uuid(), 0.5)
        .await
        .unwrap();

    let rows = sweep(&pool, principal, 10).await;
    assert!(
        !rows
            .iter()
            .any(|(c, f, _)| *c == cogmap && *f == finding.uuid()),
        "a fully covered finding (coverage == magnitude) must not appear: {rows:?}"
    );
}

/// Falsifies "the sweep surfaces uncited findings." A finding with zero live resource-kind citations
/// (`magnitude=0`) has nothing for the auditor to weigh, so `magnitude > 0` must exclude it — without
/// the guard it would appear at `uncovered = 0` as pure noise.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn sweep_omits_a_finding_with_no_resource_citations(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let (cogmap, _telos) = common::genesis_cogmap(&pool, "ads-uncited", "ADS Uncited Telos").await;
    let principal = common::create_profile(&pool, "ads-uncited-principal@example.com").await;
    join_principal_to_cogmap(&pool, principal, cogmap, "ads-uncited-team").await;

    let finding = make_cogmap_finding(
        &pool,
        owner,
        emitter,
        CogmapId::from(cogmap),
        "finding",
        "temper://ads/uncited-finding",
        None,
    )
    .await;

    let rows = sweep(&pool, principal, 10).await;
    assert!(
        !rows
            .iter()
            .any(|(c, f, _)| *c == cogmap && *f == finding.uuid()),
        "a finding with zero live resource-kind citations has nothing to audit: {rows:?}"
    );
}

/// LOAD-BEARING (the §6.2 boundary). The principal here OWNS a context-homed finding — readable via
/// `resources_visible_to`'s ownership arm (`20260624000002_canonical_functions.sql:132-134`) with NO
/// cogmap-team join anywhere in this test. That makes the assertion a genuine falsification of the
/// join shape: a sweep built on the principal's full readable-resource set
/// (`resources_visible_to`/`resources_readable_by`) and filtered by `anchor_table` AFTERWARD would
/// ALSO see this finding through plain ownership and leak it in. Only starting the query from the
/// cogmap-home join (`kb_resource_homes` restricted to `anchor_table='kb_cogmaps'`) keeps a
/// context-homed finding structurally unreachable, however readable it otherwise is.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn sweep_omits_a_context_homed_finding(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (_owner, emitter) = system_actor(&pool).await;
    let principal = ProfileId::from(common::insert_profile(&pool, "ads-context-principal").await);

    let home = make_home(&pool, principal, "ads-context-home").await;
    let source = make_resource(
        &pool,
        principal,
        emitter,
        home,
        "source",
        "temper://ads/context-source",
    )
    .await;
    let finding = writes::create_resource_with(
        &pool,
        CreateParams {
            title: "finding",
            origin_uri: "temper://ads/context-finding",
            body: "seed body",
            doc_type: "research",
            home: AnchorRef::context(home),
            owner: principal,
            originator: principal,
            emitter,
            properties: &[],
            chunks: None,
            sources: vec![Incorporation {
                source: ProvenanceSource::Resource(source.uuid()),
                seq: 0,
            }],
        },
        EventContext::default(),
    )
    .await
    .unwrap();

    let rows = sweep(&pool, principal.uuid(), 10).await;
    assert!(
        !rows.iter().any(|(_, f, _)| *f == finding.uuid()),
        "a context-homed finding must never surface, however readable it is by ownership: {rows:?}"
    );
}

/// LOAD-BEARING. Falsifies "the queue never clears a deleted finding" and "the queue audits
/// half-uploaded resources." Two otherwise-identical uncovered findings in a readable cogmap: one
/// soft-deleted (spec §3.1's liveness rule applied to the queue itself — soft-delete flips
/// `is_active` but does not fold blocks or provenance, so an unfiltered sweep would re-surface it
/// forever), one flipped to `ingest_state='in_progress'` (its citation set is still forming, by
/// construction, so it is not yet a finding to audit). Both must be excluded; a third, ordinary
/// uncovered finding in the SAME cogmap must still appear, which is what proves the exclusion is
/// per-row and not an accident of the whole cogmap being unreachable.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn sweep_omits_a_deleted_or_in_progress_finding(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let (cogmap, _telos) =
        common::genesis_cogmap(&pool, "ads-liveness", "ADS Liveness Telos").await;
    let principal = common::create_profile(&pool, "ads-liveness-principal@example.com").await;
    join_principal_to_cogmap(&pool, principal, cogmap, "ads-liveness-team").await;
    let cogmap_id = CogmapId::from(cogmap);

    let src_home = make_home(&pool, owner, "ads-liveness-src-home").await;

    let deleted_source = make_resource(
        &pool,
        owner,
        emitter,
        src_home,
        "src-del",
        "temper://ads/liveness-src-del",
    )
    .await;
    let deleted_finding = make_cogmap_finding(
        &pool,
        owner,
        emitter,
        cogmap_id,
        "deleted-finding",
        "temper://ads/liveness-deleted",
        Some(deleted_source),
    )
    .await;
    writes::delete_resource(&pool, deleted_finding, emitter)
        .await
        .unwrap();

    let in_progress_source = make_resource(
        &pool,
        owner,
        emitter,
        src_home,
        "src-ip",
        "temper://ads/liveness-src-ip",
    )
    .await;
    let in_progress_finding = make_cogmap_finding(
        &pool,
        owner,
        emitter,
        cogmap_id,
        "in-progress-finding",
        "temper://ads/liveness-in-progress",
        Some(in_progress_source),
    )
    .await;
    sqlx::query("UPDATE kb_resources SET ingest_state = 'in_progress' WHERE id = $1")
        .bind(in_progress_finding.uuid())
        .execute(&pool)
        .await
        .unwrap();

    let live_source = make_resource(
        &pool,
        owner,
        emitter,
        src_home,
        "src-live",
        "temper://ads/liveness-src-live",
    )
    .await;
    let live_finding = make_cogmap_finding(
        &pool,
        owner,
        emitter,
        cogmap_id,
        "live-finding",
        "temper://ads/liveness-live",
        Some(live_source),
    )
    .await;

    let rows = sweep(&pool, principal, 10).await;
    assert!(
        !rows.iter().any(|(_, f, _)| *f == deleted_finding.uuid()),
        "a soft-deleted finding must not head the queue: {rows:?}"
    );
    assert!(
        !rows
            .iter()
            .any(|(_, f, _)| *f == in_progress_finding.uuid()),
        "an in-progress (half-uploaded) finding must not be audited yet: {rows:?}"
    );
    assert!(
        rows.iter()
            .any(|(c, f, _)| *c == cogmap && *f == live_finding.uuid()),
        "an ordinary live, complete finding in the SAME cogmap must still appear: {rows:?}"
    );
}

/// LOAD-BEARING (the §6.3 principal gate). A finding homed in a cogmap the principal's team is NOT
/// joined to must never surface — without this gate the sweep is a cross-tenant enumeration oracle
/// (spec §6.3: "a sweep with no principal is a cross-tenant enumeration oracle"). The principal IS
/// joined to a different cogmap (so the sweep is genuinely exercised, not fed an empty candidate set
/// that would trivially return nothing), and the excluded finding is otherwise identical in shape to
/// `sweep_returns_a_finding_with_uncovered_citations` (magnitude 1, coverage 0) — the only thing
/// standing between it and a hit is the principal-scoping.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn sweep_omits_a_finding_the_principal_cannot_read(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let (unreachable_cogmap, _telos) =
        common::genesis_cogmap(&pool, "ads-unreachable", "ADS Unreachable Telos").await;
    let (reachable_cogmap, _telos2) =
        common::genesis_cogmap(&pool, "ads-reachable", "ADS Reachable Telos").await;
    let principal = common::create_profile(&pool, "ads-unreachable-principal@example.com").await;
    join_principal_to_cogmap(&pool, principal, reachable_cogmap, "ads-unreachable-team").await;

    let src_home = make_home(&pool, owner, "ads-unreachable-src-home").await;
    let source = make_resource(
        &pool,
        owner,
        emitter,
        src_home,
        "source",
        "temper://ads/unreachable-source",
    )
    .await;
    let finding = make_cogmap_finding(
        &pool,
        owner,
        emitter,
        CogmapId::from(unreachable_cogmap),
        "finding",
        "temper://ads/unreachable-finding",
        Some(source),
    )
    .await;

    let rows = sweep(&pool, principal, 10).await;
    assert!(
        !rows.iter().any(|(_, f, _)| *f == finding.uuid()),
        "a finding in a cogmap the principal cannot reach must never surface: {rows:?}"
    );
}

/// LOAD-BEARING. Falsifies "the sweep is unordered" and "the sweep orders ascending." Two findings in
/// the SAME readable cogmap: one with three uncovered sources, one with one. The three-uncovered
/// finding must sort strictly before the one-uncovered finding (`ORDER BY uncovered DESC`).
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn sweep_orders_by_uncovered_descending(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let (cogmap, _telos) = common::genesis_cogmap(&pool, "ads-order", "ADS Order Telos").await;
    let principal = common::create_profile(&pool, "ads-order-principal@example.com").await;
    join_principal_to_cogmap(&pool, principal, cogmap, "ads-order-team").await;
    let cogmap_id = CogmapId::from(cogmap);

    let low_home = make_home(&pool, owner, "ads-order-low-src-home").await;
    let low_source = make_resource(
        &pool,
        owner,
        emitter,
        low_home,
        "low-src",
        "temper://ads/order-low-src",
    )
    .await;
    let low_finding = make_cogmap_finding(
        &pool,
        owner,
        emitter,
        cogmap_id,
        "low-finding",
        "temper://ads/order-low",
        Some(low_source),
    )
    .await; // magnitude 1, coverage 0 => uncovered 1

    let high_finding = seed_cogmap_finding_with_n_citations(
        &pool,
        CogmapFindingSeed {
            owner,
            emitter,
            cogmap: cogmap_id,
            home_slug: "ads-order-high-src-home",
            title: "high-finding",
            uri: "temper://ads/order-high",
            n: 3,
        },
    )
    .await; // magnitude 3, coverage 0 => uncovered 3

    let rows = sweep(&pool, principal, 10).await;
    let low_pos = rows
        .iter()
        .position(|(_, f, _)| *f == low_finding.uuid())
        .expect("the low-uncovered finding must appear");
    let high_pos = rows
        .iter()
        .position(|(_, f, _)| *f == high_finding.uuid())
        .expect("the high-uncovered finding must appear");
    assert!(
        high_pos < low_pos,
        "uncovered DESC: the finding with 3 uncovered sources must sort before the one with 1: {rows:?}"
    );
}
