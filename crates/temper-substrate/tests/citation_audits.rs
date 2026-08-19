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
//! `search_graph_expand.rs`, and `write_path_mutations.rs` each carry their own copy).

mod common;

use temper_substrate::content;
use temper_substrate::events::{fire, EventContext, SeedAction};
use temper_substrate::ids::{
    BlockId, CogmapId, ContextId, EntityId, InvocationId, ProfileId, ResourceId,
};
use temper_substrate::payloads::{
    AgentAuthorship, AnchorRef, ConfidenceBand, Incorporation, ProvenanceSource,
};
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, AppendParams, CitationAuditParams, CreateParams};
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
            idempotency_key: None,
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

// ── Beat 1 — audited_by_profile_id, the per-auditor collapse's grouping key ─────────────────────

/// A SECOND, foreign auditor: its own `kb_profiles` row and its own `kb_entities` row.
///
/// Entity spelling copied from the established fixture idiom in this suite
/// (`charter_set_writes.rs:29`, `content_multichunk.rs:26`, `cogmap_genesis_charter.rs:29`:
/// `INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1, …, '{}'::jsonb)`).
///
/// `_event_append` applies NO gate to `p_emitter` beyond the FK
/// (`20260624000002_canonical_functions.sql:765-787`), and the self-audit denial arm lives in
/// `temper-services`' `AuditAuthority`, not in SQL — so at this layer any real entity can emit an
/// audit. That is what lets this test put two different principals on one citation, which is the
/// only way to falsify the attribution.
async fn make_auditor(pool: &sqlx::PgPool, handle: &str) -> (ProfileId, EntityId) {
    let profile = common::insert_profile(pool, handle).await;
    let entity: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_entities (profile_id, name, metadata) \
         VALUES ($1, 'auditor', '{}'::jsonb) RETURNING id",
    )
    .bind(profile)
    .fetch_one(pool)
    .await
    .unwrap();
    (ProfileId::from(profile), EntityId::from(entity))
}

/// The profile a given audit row is attributed to.
async fn attributed_profile(pool: &sqlx::PgPool, audit: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT audited_by_profile_id FROM kb_citation_audits WHERE id=$1")
        .bind(audit)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// LOAD-BEARING: falsifies "the projector attributes every audit to the same profile."
///
/// `audited_by_profile_id` exists so the next migration can collapse a citation's audits *within an
/// auditor* before averaging across sources — the echo/actor-count fallacy
/// (`20260724000120_standing_citation_components.sql:154-224`) re-entering on the auditor axis. That
/// collapse is only as good as the grouping key, and a key that is constant per finding silently
/// merges every auditor into one, which is the exact opposite of what it is for.
///
/// TWO AUDITS OF ONE CITATION FROM TWO DIFFERENT PRINCIPALS is the minimum shape that bites. A
/// single-auditor test cannot: in the fixtures of this file the resource owner, the originator and
/// the emitter are ALL the `system` profile, so a projector that read `audited_by_profile_id` off
/// the finding's owner, off the source's owner, or off any other single-profile path would pass it.
/// The foreign auditor breaks that degeneracy — for its row the emitter's profile is the ONLY
/// profile in the fixture that is not `system`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_projector_attributes_each_audit_to_its_own_emitters_profile(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, system_emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "ca-attrib").await;
    let finding = make_resource(
        &pool,
        owner,
        system_emitter,
        home,
        "finding",
        "temper://ca/attrib-finding",
    )
    .await;
    let source = make_resource(
        &pool,
        owner,
        system_emitter,
        home,
        "source",
        "temper://ca/attrib-source",
    )
    .await;
    let block = first_block(&pool, finding).await;
    cite(&pool, block, source.uuid()).await;

    let (auditor_profile, auditor_emitter) = make_auditor(&pool, "adversary").await;

    // The SAME citation, audited by two different principals — the shape the per-auditor collapse
    // has to be able to tell apart. Opposite-signed, mirroring
    // `two_audits_of_one_citation_both_persist`, so neither row can be mistaken for the other by
    // value either.
    let audit_system = fire_audit(&pool, system_emitter, block, source.uuid(), 0.6)
        .await
        .unwrap();
    let audit_foreign = fire_audit(&pool, auditor_emitter, block, source.uuid(), -0.4)
        .await
        .unwrap();

    let by_system = attributed_profile(&pool, audit_system).await;
    let by_foreign = attributed_profile(&pool, audit_foreign).await;

    assert_eq!(
        by_system,
        owner.uuid(),
        "the system-emitted audit must be attributed to the system profile"
    );
    // THE HALF THAT BITES. `auditor_profile` is the only non-`system` profile in this fixture, so
    // this equality cannot be satisfied by the finding's owner, the source's owner, the originator,
    // or any constant — only by the emitting event's own entity.
    assert_eq!(
        by_foreign,
        auditor_profile.uuid(),
        "the foreign auditor's audit must be attributed to THAT auditor, not to the finding's owner"
    );
    assert_ne!(
        by_system, by_foreign,
        "two auditors of one citation must not collapse to a single profile — that collapse is \
         precisely what the column exists to prevent"
    );

    // Stated once more as the TOTAL invariant rather than as two point checks, so a projector that
    // got these two rows right by coincidence and any future row wrong still fails here.
    let mismatched: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_citation_audits a \
           JOIN kb_events   e  ON e.id  = a.audited_by_event_id \
           JOIN kb_entities en ON en.id = e.emitter_entity_id \
          WHERE a.audited_by_profile_id <> en.profile_id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        mismatched, 0,
        "every audit's audited_by_profile_id must equal its owning event's emitter's profile"
    );
}

// ── Beat 2 — the per-auditor collapse in resource_citation_quality ──────────────────────────────
//
// `migrations/20260724000210_citation_quality_per_auditor.sql` replaces the shipped TWO-stage
// aggregate (`20260724000120_standing_citation_components.sql:201-224`) with THREE stages: collapse
// within an AUDITOR, then across auditors weighted by each auditor's freshest-audit weight, then the
// plain mean across distinct audited sources.
//
// These tests live here rather than in `evidential_standing.rs` because the axis under test is the
// AUDITOR, and this file is where a second principal exists: every fixture in `evidential_standing.rs`
// emits as the one `system` entity, so a per-auditor collapse is invisible there by construction. The
// `make_auditor` fixture above (Beat 1) is what makes these shapes expressible at all.

/// The finding's live `citation_quality`. Read straight off the SQL producer rather than through
/// `resource_standing_shape`: the access gate is not what these tests are about and
/// `evidential_standing.rs::standing_shape_returns_none_for_an_unreadable_finding` already covers it.
async fn citation_quality(pool: &sqlx::PgPool, finding: ResourceId) -> f64 {
    sqlx::query_scalar("SELECT resource_citation_quality($1)")
        .bind(finding.uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn audit_coverage(pool: &sqlx::PgPool, finding: ResourceId) -> i32 {
    sqlx::query_scalar("SELECT resource_audit_coverage($1)")
        .bind(finding.uuid())
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Back-date one audit's decay clock — the only way to exercise decay without sleeping.
///
/// It is the AUDIT's `created` that moves, never the owning event's `occurred_at`:
/// `kb_citation_audits` carries no append-only trigger (the trigger `kb_events_append_only` is on
/// `kb_events`, `20260624000001_canonical_schema.sql:504-506`, and rejects every UPDATE), so ageing
/// the event would simply fail. Same idiom as `evidential_standing.rs:899`.
async fn age_audit(pool: &sqlx::PgPool, audit: Uuid, age: &str) {
    sqlx::query("UPDATE kb_citation_audits SET created = now() - $2::interval WHERE id = $1")
        .bind(audit)
        .bind(age)
        .execute(pool)
        .await
        .unwrap();
}

/// A finding citing one source, ready to audit: returns `(finding, source, block)`.
async fn one_citation(
    pool: &sqlx::PgPool,
    owner: ProfileId,
    emitter: EntityId,
    home: ContextId,
    slug: &str,
) -> (ResourceId, ResourceId, Uuid) {
    let finding = make_resource(
        pool,
        owner,
        emitter,
        home,
        &format!("{slug}-finding"),
        &format!("temper://pa/{slug}-finding"),
    )
    .await;
    let source = make_resource(
        pool,
        owner,
        emitter,
        home,
        &format!("{slug}-source"),
        &format!("temper://pa/{slug}-source"),
    )
    .await;
    let block = first_block(pool, finding).await;
    cite(pool, block, source.uuid()).await;
    (finding, source, block)
}

/// Every near-zero / equality assertion below uses `1e-4`, not an exact compare, and the slack is
/// principled rather than nervous: the decay clock is CONTINUOUS, so two audits fired microseconds
/// apart do not carry byte-identical weights. Ten audits landing over even a full second differ by
/// ~3e-7 in weight (30-day half-life), so the residual is ~1e-7. `1e-4` is four orders of magnitude
/// clear of that and four orders clear of the `-0.818182` this beat exists to eliminate — it can only
/// be exceeded by an aggregation change, never by test-runner latency.
const NEAR_ZERO: f64 = 1e-4;

/// LOAD-BEARING. **Volume no longer wins.** One griefer emits TEN `-1.0` audits of a citation; one
/// honest auditor emits a single `+1.0`. The aggregate must read what a plain 1-vs-1 disagreement
/// reads — because that is what it IS: two principals disagreed.
///
/// THIS FALSIFIES THE SHIPPED FUNCTION, not merely a hypothetical one. The two-stage aggregate groups
/// by `source_id` alone (`20260724000120_standing_citation_components.sql:218-222`), so all eleven
/// audits land in one weighted mean and it reads **-0.818182** — measured, not estimated, by running
/// both bodies side by side against this exact fixture. `-0.818182` is `disputed` under the band gate
/// (`:312-314`). The 1-vs-1 baseline reads `0.0`. So the equality assertion below is a difference of
/// **0.818** under the old body: it cannot pass by accident.
///
/// Both shapes are built IN THIS TEST rather than the baseline being hard-coded, so the assertion is
/// "volume changed nothing" — a claim about the function — instead of "the number is 0.0", a claim
/// about a constant that a future re-tune would falsify for the wrong reason.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn one_auditors_volume_no_longer_outweighs_a_single_dissenter(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, honest) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "pa-volume").await;
    let (_griefer_profile, griefer) = make_auditor(&pool, "pa-volume-griefer").await;

    // Shape 1 — the attack: ten `-1.0` from ONE principal, one `+1.0` from another.
    let (loud, loud_src, loud_block) = one_citation(&pool, owner, honest, home, "loud").await;
    for _ in 0..10 {
        fire_audit(&pool, griefer, loud_block, loud_src.uuid(), -1.0)
            .await
            .unwrap();
    }
    fire_audit(&pool, honest, loud_block, loud_src.uuid(), 1.0)
        .await
        .unwrap();

    // Shape 2 — the honest baseline, identical in every respect but the griefer's persistence.
    let (quiet, quiet_src, quiet_block) = one_citation(&pool, owner, honest, home, "quiet").await;
    fire_audit(&pool, griefer, quiet_block, quiet_src.uuid(), -1.0)
        .await
        .unwrap();
    fire_audit(&pool, honest, quiet_block, quiet_src.uuid(), 1.0)
        .await
        .unwrap();

    let loud_q = citation_quality(&pool, loud).await;
    let quiet_q = citation_quality(&pool, quiet).await;

    assert!(
        quiet_q.abs() < NEAR_ZERO,
        "precondition: one +1.0 against one -1.0 cancels to ~0.0 (got {quiet_q})"
    );
    assert!(
        (loud_q - quiet_q).abs() < NEAR_ZERO,
        "ten audits from one principal must weigh exactly what one does: 1-vs-1 read {quiet_q}, \
         10-vs-1 read {loud_q}. Under the shipped two-stage function 10-vs-1 read -0.818182 — \
         nine extra POSTs bought most of the way to the floor of the scale."
    );
    assert!(
        loud_q > -0.5,
        "stated once more as the direct falsification of the two-stage body, which produced \
         -0.818182 (band 'disputed') for this exact fixture: got {loud_q}"
    );

    // The LEDGER is untouched by any of this — the collapse changes influence, never history
    // (spec §4.1; `two_audits_of_one_citation_both_persist` above pins the two-row case).
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_citation_audits WHERE block_id=$1")
        .bind(loud_block)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        rows, 11,
        "all eleven audits persist: the per-auditor collapse is a READ-time aggregation, not a \
         write-time deduplication"
    );
}

/// LOAD-BEARING. **Repeated agreement, spread across the decay window, still counts once.** One
/// auditor posts `+1.0` on the SAME citation five times — today and at 15, 30, 45 and 60 days — and a
/// second principal posts a single `-1.0` today. One principal, one vote: the two verdicts cancel.
///
/// THIS IS THE LARGEST BEHAVIORAL DELTA THE BEAT SHIPS, and it is measured rather than estimated.
/// Under the shipped two-stage aggregate all six audits land in ONE weighted mean grouped by
/// `source_id` alone (`20260724000120_standing_citation_components.sql:218-222`), so the five `+1.0`s
/// carry `1 + 0.7071 + 0.5 + 0.3536 + 0.25 = 2.8107` of weight against the dissenter's `1.0` and it
/// reads **0.475157** — `reinforced` under the band gate (`:280-284`: magnitude 1, coverage 1,
/// quality > 0.0). Three-staged it reads **0.000000**, which is `disputed` (`:312-314`). The
/// assertion below is a difference of 0.475 AND a band flip: it cannot pass against the old body.
///
/// Distinct from `one_auditors_volume_no_longer_outweighs_a_single_dissenter` above, whose repeats
/// all land at once and therefore carry near-identical weights — a collapse that merely deduplicated
/// by count would satisfy that one. STAGGERING the repeats over two half-lives is what makes this a
/// test of the WEIGHTED within-auditor collapse: the five weights differ 4x end to end, and stage 1
/// still has to arrive at exactly `+1.0`, because every one of them says `+1.0`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn repeated_agreement_across_the_decay_window_still_counts_once(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, system_emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "pa-repeat").await;
    let (_pr, repeater) = make_auditor(&pool, "pa-repeat-repeater").await;
    let (_pd, dissenter) = make_auditor(&pool, "pa-repeat-dissenter").await;

    let (finding, source, block) = one_citation(&pool, owner, system_emitter, home, "repeat").await;

    // ONE principal, five `+1.0`s, staggered across the decay window. Ageing moves the AUDIT's
    // `created` (never the owning event's `occurred_at` — `kb_events` is append-only); see
    // `age_audit`.
    for days in [0, 15, 30, 45, 60] {
        let audit = fire_audit(&pool, repeater, block, source.uuid(), 1.0)
            .await
            .unwrap();
        if days > 0 {
            age_audit(&pool, audit, &format!("{days} days")).await;
        }
    }
    // A DIFFERENT principal, one `-1.0`, today.
    fire_audit(&pool, dissenter, block, source.uuid(), -1.0)
        .await
        .unwrap();

    let q = citation_quality(&pool, finding).await;
    assert!(
        q.abs() < NEAR_ZERO,
        "five `+1.0`s from one principal weigh exactly what one weighs, so the lone fresh `-1.0` \
         cancels them: got {q}. The shipped two-stage body read 0.475157 for this exact fixture — \
         `reinforced` instead of `disputed`, bought by re-posting the same verdict four more times."
    );

    // Coverage is untouched: it asks *"did anyone look?"*, not *"how many times?"* — one distinct
    // source, evaluated, however many audits it carries.
    assert_eq!(
        audit_coverage(&pool, finding).await,
        1,
        "one covered source, before and after the collapse"
    );

    // And the ledger keeps every row: the collapse is a READ-time aggregation, never a write-time
    // deduplication (spec §4.1 — no supersession, nothing erased).
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_citation_audits WHERE block_id=$1")
        .bind(block)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 6, "all six audits persist");
}

/// LOAD-BEARING. **Retraction.** With no supersession and no `is_superseded` column
/// (`20260724000110_citation_audits.sql:12-17`), the ONLY way an auditor changes its mind is to
/// append a new verdict — so the within-auditor collapse has to be decay-weighted, or "I was wrong"
/// is inexpressible and an auditor is bound to its first impression forever.
///
/// THE BITE IS AGAINST THE NEW CODE, and it is the obvious wrong simplification of it: write stage 1
/// as a plain `avg(value)` per (source, auditor) — which "collapses one auditor to one vote" just as
/// truthfully — and this fixture reads **0.0**, because `-1.0` and `+1.0` average out however old the
/// first one is. Only `sum(w*value)/sum(w)` gets it right.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_auditors_later_verdict_dominates_its_own_earlier_one(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, system_emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "pa-retract").await;
    let (_p, auditor) = make_auditor(&pool, "pa-retract-auditor").await;

    let (finding, source, block) =
        one_citation(&pool, owner, system_emitter, home, "retract").await;

    // The SAME principal, twice, opposite-signed — a retraction, not a disagreement.
    let first = fire_audit(&pool, auditor, block, source.uuid(), -1.0)
        .await
        .unwrap();
    age_audit(&pool, first, "365 days").await;
    fire_audit(&pool, auditor, block, source.uuid(), 1.0)
        .await
        .unwrap();

    let q = citation_quality(&pool, finding).await;
    assert!(
        q > 0.9,
        "a year-old -1.0 has faded to ~2e-4 of today's +1.0 weight (30-day half-life), so the \
         auditor's current position is what shows: got {q}. A plain avg() at the within-auditor \
         stage reads 0.0 here."
    );

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_citation_audits WHERE block_id=$1")
        .bind(block)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        rows, 2,
        "the retracted verdict is still a permanent row — changing your mind is an append"
    );
}

/// LOAD-BEARING. **Cross-auditor recency still arbitrates**, and this is the test that defends the
/// one genuinely contestable judgment in the migration: stage 2 is a mean WEIGHTED by each auditor's
/// freshest-audit weight, not a plain mean.
///
/// The shipped header states the invariant — *"decay only arbitrates BETWEEN competing audits"*
/// (`20260724000120_standing_citation_components.sql:177`). Two auditors disagreeing about one source
/// are the textbook competing audits. A plain `avg(auditor_value)` at stage 2 reads **0.0** here: a
/// verdict from last year counting equally with a verdict from today.
///
/// And it would be a REGRESSION, which is what makes this test non-negotiable rather than a
/// preference: the shipped TWO-stage function gets this case right (`-0.999565`, measured), because
/// with one audit each the flat weighted mean already arbitrates by recency. A per-auditor collapse
/// that dropped the weighting would fix the volume attack by breaking a guarantee that already
/// worked — trading one defect for another and calling it a repair.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_stale_verdict_does_not_outweigh_a_fresh_one_from_another_auditor(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, system_emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "pa-recency").await;
    let (_px, stale_auditor) = make_auditor(&pool, "pa-recency-stale").await;
    let (_py, fresh_auditor) = make_auditor(&pool, "pa-recency-fresh").await;

    let (finding, source, block) =
        one_citation(&pool, owner, system_emitter, home, "recency").await;

    // ONE audit each — so the per-auditor stage collapses nothing, and the ONLY thing that can
    // separate these two verdicts is their age.
    let stale = fire_audit(&pool, stale_auditor, block, source.uuid(), 1.0)
        .await
        .unwrap();
    age_audit(&pool, stale, "365 days").await;
    fire_audit(&pool, fresh_auditor, block, source.uuid(), -1.0)
        .await
        .unwrap();

    let q = citation_quality(&pool, finding).await;
    assert!(
        q < -0.9,
        "today's -1.0 must dominate last year's +1.0 across auditors as it does within one: got \
         {q}. A PLAIN mean at the across-auditor stage reads exactly 0.0 here, and the shipped \
         two-stage function read -0.999565 — so 0.0 is a regression, not a simplification."
    );
}

/// **An all-decayed source drops OUT of the mean rather than becoming `0.0`** — the shipped
/// null-handling (`20260724000120_standing_citation_components.sql:179-183`: *"it was evaluated, but
/// the verdict no longer carries numeric weight"*), preserved across the extra stage.
///
/// PRESERVATION, NOT FALSIFICATION — and the label matters, because the two siblings above are the
/// other kind. The shipped TWO-stage body reads **1.000000** on this identical fixture (measured):
/// its single `nullif(sum(w), 0)` already drops an all-decayed source, so there is no old-body
/// number here to falsify. Like `an_auditors_later_verdict_dominates_its_own_earlier_one` above, the
/// bite is against a mutation of the NEW code:
///   * stage 1's `nullif` removed — every weight in the group is `0`, so `sum(w*value)/0` is
///     `0.0/0`, which RAISES `division by zero` in a READ path;
///   * `coalesce(auditor_value, 0.0)` anywhere in the collapse — the decayed source votes as a
///     neutral verdict instead of dropping out, and the fixture reads `0.5`.
/// Stage 2's `nullif` is NOT what this fixture pins: a NULL `auditor_value` implies that auditor's
/// `max(w)` is `0` (the header's equivalence), and `NULL / 0` is NULL rather than an error because
/// division is strict — so the source drops out through the NULL numerator whether or not the second
/// guard is there. That guard states the invariant; this test does not falsify its removal.
///
/// Two auditors on the decayed source, not one, so the NULL arrives through the per-auditor grouping
/// and not merely through a single degenerate group.
///
/// The age is 500 years, not the ~88 the shipped header cites. That is measured, not arbitrary:
/// `pow(0.5, …)` here is NUMERIC, so it does not underflow to a float zero — it reaches exactly `0`
/// only around 5000 half-lives (~410 years), and in the band between them the numeric→`double
/// precision` cast RAISES `out of range` instead. That gap is an incumbent property of the shipped
/// `weighted` CTE, which this beat carries over verbatim and does not address.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_source_whose_every_auditor_decayed_drops_out_of_the_mean(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, system_emitter) = system_actor(&pool).await;
    let home = make_home(&pool, owner, "pa-decayed").await;
    let (_pa, auditor_a) = make_auditor(&pool, "pa-decayed-a").await;
    let (_pb, auditor_b) = make_auditor(&pool, "pa-decayed-b").await;

    let (finding, decayed_src, block) =
        one_citation(&pool, owner, system_emitter, home, "decayed").await;
    let fresh_src = make_resource(
        &pool,
        owner,
        system_emitter,
        home,
        "decayed-fresh-source",
        "temper://pa/decayed-fresh-source",
    )
    .await;
    cite(&pool, block, fresh_src.uuid()).await;

    for emitter in [auditor_a, auditor_b] {
        let old = fire_audit(&pool, emitter, block, decayed_src.uuid(), -1.0)
            .await
            .unwrap();
        age_audit(&pool, old, "500 years").await;
    }
    fire_audit(&pool, auditor_a, block, fresh_src.uuid(), 1.0)
        .await
        .unwrap();

    let q = citation_quality(&pool, finding).await;
    assert!(
        (q - 1.0).abs() < 1e-9,
        "the fully-decayed source must drop out of the mean, leaving the fresh +1.0 alone. \
         Counting it as a neutral 0.0 reads 0.5; counting its weight in the denominator reads \
         something between. Got {q}"
    );

    // The other half of the shipped reading, and the reason dropping out is honest rather than
    // lossy: the source is STILL COVERED. Someone did look; the verdict simply no longer carries
    // numeric weight (`20260724000120:181-183`). The mean's denominator is therefore not
    // `audit_coverage`, and that gap is recorded there, not fixed here.
    assert_eq!(
        audit_coverage(&pool, finding).await,
        2,
        "a decayed verdict still counts as coverage — decay changes weight, never whether it \
         happened"
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
// Spec `internal/superpowers/specs/2026-07-23-set5-adversary-citation-audit-design.md` §6.2-6.3,
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
            idempotency_key: None,
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
            idempotency_key: None,
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
            idempotency_key: None,
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

/// LOAD-BEARING (`20260727000020`), and the SOURCE-side twin of the test above.
///
/// `sweep_omits_a_deleted_or_in_progress_finding` proves the FINDING's `ingest_state` is gated —
/// that gate lives in `audit_drift_sweep`'s `candidates`. Nothing proved the same about the SOURCE,
/// and it was not true: `resource_live_citations` gated the source on `is_active` alone, so a
/// finding whose only citation pointed at a half-uploaded source had `magnitude = 1` and headed the
/// queue. The auditor would then be handed a passage whose bytes are still arriving.
///
/// The two findings are identical in every respect except their source's `ingest_state`, so the
/// gate is the only thing that can separate them. Bite probe: drop `src.ingest_state = 'complete'`
/// from `resource_live_citations` and this is the test that fails.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn sweep_omits_a_finding_whose_only_source_is_in_progress(pool: sqlx::PgPool) {
    bootseed::seed_system(&pool).await.unwrap();
    let (owner, emitter) = system_actor(&pool).await;
    let (cogmap, _telos) = common::genesis_cogmap(&pool, "ads-src-ip", "ADS Source Ingest").await;
    let principal = common::create_profile(&pool, "ads-src-ip-principal@example.com").await;
    join_principal_to_cogmap(&pool, principal, cogmap, "ads-src-ip-team").await;
    let cogmap_id = CogmapId::from(cogmap);

    let src_home = make_home(&pool, owner, "ads-src-ip-home").await;

    let in_flight_source = make_resource(
        &pool,
        owner,
        emitter,
        src_home,
        "src-in-flight",
        "temper://ads/src-ip-in-flight",
    )
    .await;
    let finding_on_in_flight = make_cogmap_finding(
        &pool,
        owner,
        emitter,
        cogmap_id,
        "finding-on-in-flight-source",
        "temper://ads/src-ip-finding-in-flight",
        Some(in_flight_source),
    )
    .await;
    // The SOURCE is half-uploaded — the finding itself is complete and live throughout.
    sqlx::query("UPDATE kb_resources SET ingest_state = 'in_progress' WHERE id = $1")
        .bind(in_flight_source.uuid())
        .execute(&pool)
        .await
        .unwrap();

    let complete_source = make_resource(
        &pool,
        owner,
        emitter,
        src_home,
        "src-complete",
        "temper://ads/src-ip-complete",
    )
    .await;
    let finding_on_complete = make_cogmap_finding(
        &pool,
        owner,
        emitter,
        cogmap_id,
        "finding-on-complete-source",
        "temper://ads/src-ip-finding-complete",
        Some(complete_source),
    )
    .await;

    let rows = sweep(&pool, principal, 10).await;
    assert!(
        !rows
            .iter()
            .any(|(_, f, _)| *f == finding_on_in_flight.uuid()),
        "a finding whose only citation is a half-uploaded source has no auditable evidence yet, \
         so it must not head the queue: {rows:?}"
    );
    assert!(
        rows.iter()
            .any(|(c, f, u)| *c == cogmap && *f == finding_on_complete.uuid() && *u == 1),
        "the otherwise-identical finding on a COMPLETE source must still appear at uncovered=1, \
         which is what proves the exclusion above is the ingest gate and not an empty sweep: \
         {rows:?}"
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

// ── Tier-1 staleness — `resource_has_stale_citation` + the `stale` disjunct ──────────────────────
//
// Migration `20260726000010_auditor_tier1_staleness.sql`, design
// `internal/superpowers/specs/2026-07-25-auditor-tier1-staleness-watermark-design.md` (rev. 2).
//
// THE DEFECT THESE WITNESS: `audit_drift_sweep` selected on `coverage < magnitude`, so once a
// finding was fully covered it never re-entered the queue — a `block_mutated` could change an
// assertion's text while every verdict about the previous text stood unrevisited forever.
//
// WHY THE AUDITING ENTITY MUST BELONG TO THE SWEEPING PRINCIPAL. The watermark is scoped
// `audited_by_profile_id = p_principal` (spec §3.2(a)), and `audited_by_profile_id` is resolved from
// the owning event's emitter (`20260724000200_citation_audit_attribution.sql:72-76`). The Task-5
// fixtures above audit as the `system` actor while sweeping as a separate `principal`, which is fine
// for coverage — coverage is global — but would make EVERY staleness assertion below vacuously false.
// So these fixtures audit with the sweeping principal's own entity, via `make_auditor`.

/// The shape every tier-1 staleness witness needs: a cogmap the sweeping principal can reach, that
/// principal's own entity, one source, and one finding citing it.
struct StaleFixture {
    cogmap: Uuid,
    /// Sweeping principal — also the auditor, for the reason in this section's header.
    principal: ProfileId,
    auditor: EntityId,
    /// The `system` actor, which creates the resources (authorship is not under test here).
    owner: ProfileId,
    emitter: EntityId,
    source: ResourceId,
    finding: ResourceId,
}

/// `slug` disambiguates every fixture row this creates; each test passes its own.
async fn stale_fixture(pool: &sqlx::PgPool, slug: &str) -> StaleFixture {
    bootseed::seed_system(pool).await.unwrap();
    let (owner, emitter) = system_actor(pool).await;
    let (cogmap, _telos) = common::genesis_cogmap(pool, slug, &format!("{slug} telos")).await;
    let (principal, auditor) = make_auditor(pool, &format!("{slug}-principal")).await;
    join_principal_to_cogmap(pool, principal.uuid(), cogmap, &format!("{slug}-team")).await;

    let src_home = make_home(pool, owner, &format!("{slug}-src-home")).await;
    let source = make_resource(
        pool,
        owner,
        emitter,
        src_home,
        "source",
        &format!("temper://{slug}/source"),
    )
    .await;
    let finding = make_cogmap_finding(
        pool,
        owner,
        emitter,
        CogmapId::from(cogmap),
        "finding",
        &format!("temper://{slug}/finding"),
        Some(source),
    )
    .await;
    StaleFixture {
        cogmap,
        principal,
        auditor,
        owner,
        emitter,
        source,
        finding,
    }
}

/// Revise a block's prose through the production write path — the `SeedAction::BlockMutate` shape
/// verified at `readout_tier.rs:71-90`.
///
/// This is what bumps `kb_content_blocks.last_event_id`, and it is why `cite()` above CANNOT stand in
/// for a material change: `cite()` reuses the block's `genesis_event_id` and moves no cursor.
/// `incorporated: &[]` is safe — `_project_block_mutated` *accretes* provenance ("empty ⇒ no-op"), so
/// the finding's existing citation survives the revision rather than being replaced by nothing.
async fn mutate_block(pool: &sqlx::PgPool, block: Uuid, emitter: EntityId, prose: &str) {
    let prepared = content::prepare_block(0, None, prose).unwrap();
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::BlockMutate {
            incorporated: &[],
            block: BlockId::from(block),
            chunks: &prepared.chunks,
            raw: None,
            emitter,
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
}

/// Append a second block to `finding` that cites `source` — a live citation on a block that has
/// never been audited.
async fn append_citing_block(
    pool: &sqlx::PgPool,
    finding: ResourceId,
    source: ResourceId,
    emitter: EntityId,
    prose: &str,
) -> BlockId {
    let block = content::prepare_block(1, None, prose).unwrap();
    writes::append_block(
        pool,
        AppendParams {
            resource: finding,
            block: &block,
            sources: vec![Incorporation {
                source: ProvenanceSource::Resource(source.uuid()),
                seq: 0,
            }],
            emitter,
        },
    )
    .await
    .unwrap()
}

/// Does the sweep offer this `(cogmap, finding)` pair to this principal?
async fn sweep_offers(
    pool: &sqlx::PgPool,
    principal: ProfileId,
    cogmap: Uuid,
    finding: ResourceId,
) -> bool {
    sweep(pool, principal.uuid(), 10)
        .await
        .iter()
        .any(|(c, f, _)| *c == cogmap && *f == finding.uuid())
}

/// LOAD-BEARING (the defect itself). A fully covered finding whose citing block is then MUTATED must
/// be re-offered. Before `20260726000010` this returned nothing, forever: `coverage == magnitude` was
/// the only selection predicate, so the verdict about the replaced text stood unrevisited.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn sweep_reoffers_a_covered_finding_after_its_block_is_mutated(pool: sqlx::PgPool) {
    let f = stale_fixture(&pool, "ads-stale-mutate").await;
    let block = first_block(&pool, f.finding).await;
    fire_audit(&pool, f.auditor, block, f.source.uuid(), 0.5)
        .await
        .unwrap();
    assert!(
        !sweep_offers(&pool, f.principal, f.cogmap, f.finding).await,
        "precondition: once audited, the finding is fully covered and must be quiet"
    );

    mutate_block(
        &pool,
        block,
        f.emitter,
        "the assertion now says something else entirely",
    )
    .await;

    assert!(
        sweep_offers(&pool, f.principal, f.cogmap, f.finding).await,
        "a covered finding whose block was mutated after the audit must be re-offered"
    );
}

/// THE NOISE FLOOR (migration `20260726000030`). The mirror of the witness above: staleness must fire
/// on a CHANGED assertion and stay silent on an unchanged one. Before that migration `block_mutate`
/// fired on the PRESENCE of a body rather than a changed one, so a no-op round-trip through
/// CLAUDE.md's own documented show-edit-`cat` idiom bumped `last_event_id` on byte-identical content
/// and re-offered a quiet finding — the exact failure tier 1's design §6 calls "worse than the defect
/// it replaces".
///
/// This bites: with the suppression removed, the second `mutate_block` moves the cursor and the
/// finding is offered again. It is the pair to
/// `sweep_reoffers_a_covered_finding_after_its_block_is_mutated`, which holds the other direction —
/// neither alone discriminates a predicate that is merely always-on or always-off.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn sweep_stays_quiet_after_a_byte_identical_revise(pool: sqlx::PgPool) {
    let f = stale_fixture(&pool, "ads-noop-revise").await;
    let block = first_block(&pool, f.finding).await;
    const PROSE: &str = "the assertion exactly as it currently stands";

    // Establish PROSE as the block's current text, then audit it — covered, and quiet.
    mutate_block(&pool, block, f.emitter, PROSE).await;
    fire_audit(&pool, f.auditor, block, f.source.uuid(), 0.5)
        .await
        .unwrap();
    assert!(
        !sweep_offers(&pool, f.principal, f.cogmap, f.finding).await,
        "precondition: audited at its current text, the finding must be quiet"
    );

    // The no-op round-trip: re-submit byte-identical prose.
    mutate_block(&pool, block, f.emitter, PROSE).await;

    assert!(
        !sweep_offers(&pool, f.principal, f.cogmap, f.finding).await,
        "a byte-identical revise changed nothing, so the finding must stay quiet — \
         re-offering here is the noise floor tier 1 must not have"
    );
}

/// The SOURCE-side arm. Without it this suite would pass with a predicate that only ever looks at the
/// citing block, and the auditor would never learn that a source it relied on had changed underneath.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn sweep_reoffers_a_covered_finding_after_its_cited_source_changes(pool: sqlx::PgPool) {
    let f = stale_fixture(&pool, "ads-stale-source").await;
    let block = first_block(&pool, f.finding).await;
    fire_audit(&pool, f.auditor, block, f.source.uuid(), 0.5)
        .await
        .unwrap();

    let source_block = first_block(&pool, f.source).await;
    mutate_block(
        &pool,
        source_block,
        f.emitter,
        "the source no longer says that",
    )
    .await;

    assert!(
        sweep_offers(&pool, f.principal, f.cogmap, f.finding).await,
        "a covered finding whose CITED SOURCE changed after the audit must be re-offered"
    );
}

/// Witnesses the `block_wm IS NULL` arm (spec §3.2(b)).
///
/// `resource_audit_coverage` is `count(DISTINCT source_id)`, so auditing a source on ONE block marks
/// it covered across ALL blocks of the finding. A new block citing that already-covered source is
/// therefore selected by NEITHER `coverage < magnitude` nor a watermark comparison that requires an
/// existing audit on that block — it needs the explicit "this principal never audited this citation"
/// arm. Tests 1-2 use single-block fixtures and cannot observe this.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn sweep_reoffers_a_two_block_finding_when_the_unaudited_pair_s_block_changes(
    pool: sqlx::PgPool,
) {
    let f = stale_fixture(&pool, "ads-stale-append").await;
    let block = first_block(&pool, f.finding).await;
    fire_audit(&pool, f.auditor, block, f.source.uuid(), 0.5)
        .await
        .unwrap();

    append_citing_block(
        &pool,
        f.finding,
        f.source,
        f.emitter,
        "a second assertion drawn from the same source",
    )
    .await;

    // The new block cites an ALREADY-COVERED source, so the coverage disjunct stays silent.
    let coverage: i32 = sqlx::query_scalar("SELECT resource_audit_coverage($1)")
        .bind(f.finding.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    let magnitude: i32 = sqlx::query_scalar("SELECT resource_citation_magnitude($1)")
        .bind(f.finding.uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        (magnitude, coverage),
        (1, 1),
        "the appended block cites the same source, so coverage must still equal magnitude — \
         otherwise this test would pass through the `uncovered` disjunct and witness nothing"
    );

    assert!(
        sweep_offers(&pool, f.principal, f.cogmap, f.finding).await,
        "a new block citing an already-covered source carries unweighed text and must be re-offered"
    );
}

/// Witnesses the per-principal scoping (spec §3.2(a)).
///
/// A global watermark picks the COVERAGE grain ("did anyone look?") while staleness protects the
/// QUALITY grain ("is each principal's vote about the current text?"), and `20260724000210` separated
/// those deliberately. With a global max, B's fresh verdict would extinguish A's staleness and leave
/// the finding terminal while A's verdict about deleted text still carried half the aggregate.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn sweep_stale_is_scoped_to_the_sweeping_principal(pool: sqlx::PgPool) {
    let f = stale_fixture(&pool, "ads-stale-principal").await;
    let (b_principal, b_auditor) = make_auditor(&pool, "ads-stale-principal-b").await;
    join_principal_to_cogmap(
        &pool,
        b_principal.uuid(),
        f.cogmap,
        "ads-stale-principal-b-team",
    )
    .await;

    let block = first_block(&pool, f.finding).await;
    fire_audit(&pool, f.auditor, block, f.source.uuid(), 0.5)
        .await
        .unwrap();
    mutate_block(
        &pool,
        block,
        f.emitter,
        "revised after A looked, before B looked",
    )
    .await;
    fire_audit(&pool, b_auditor, block, f.source.uuid(), 0.5)
        .await
        .unwrap();

    assert!(
        sweep_offers(&pool, f.principal, f.cogmap, f.finding).await,
        "A's verdict predates the mutation, so the finding is stale FOR A"
    );
    assert!(
        !sweep_offers(&pool, b_principal, f.cogmap, f.finding).await,
        "B read the current text, so the finding is NOT stale for B — a global watermark would \
         have extinguished A's staleness here too"
    );
}

/// LOAD-BEARING, and the one that falsifies the FIRST repair of this design.
///
/// Rev. 1 keyed the watermark on `(finding, source)` to match `resource_audit_coverage`'s grain. That
/// takes the LATEST audit across every block of the finding, so an audit of one block sits above
/// another block's mutation and hides it permanently — the same defect
/// `sweep_stale_is_scoped_to_the_sweeping_principal` guards, one grain over.
///
/// This needs no skipped citation: read the three writes below as ONE audit run, with an edit landing
/// mid-run. Runs are LLM-paced minutes; ordinary behaviour on both sides.
///
/// A witness that only bites against "the feature is absent" discriminates nothing — the bar W1 was
/// cancelled for missing. This one also bites against a specific WRONG implementation: swap the
/// `FILTER (WHERE a.block_id = lc.block_id)` watermark for a plain finding-scoped `array_agg` and
/// this test, alone in the suite, goes red.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn sweep_reoffers_when_a_sibling_audit_lands_after_another_blocks_mutation(
    pool: sqlx::PgPool,
) {
    let f = stale_fixture(&pool, "ads-stale-sibling").await;
    let block0 = first_block(&pool, f.finding).await;
    let block1 = append_citing_block(
        &pool,
        f.finding,
        f.source,
        f.emitter,
        "a second assertion drawn from the same source",
    )
    .await;

    fire_audit(&pool, f.auditor, block0, f.source.uuid(), 0.5)
        .await
        .unwrap();
    mutate_block(
        &pool,
        block0,
        f.emitter,
        "block 0's assertion now reads differently",
    )
    .await;
    fire_audit(&pool, f.auditor, block1.uuid(), f.source.uuid(), 0.5)
        .await
        .unwrap();

    assert!(
        sweep_offers(&pool, f.principal, f.cogmap, f.finding).await,
        "block 0 was mutated after its own audit; a later audit of its SIBLING must not bury that"
    );
}

/// Witnesses the ordering key (spec §3.4).
///
/// A stale finding is by construction fully covered, so its `uncovered` is 0 — the MINIMUM of the
/// `ORDER BY uncovered DESC` this migration inherited. Carried forward unchanged, every stale finding
/// sorts behind every uncovered one, and with k permanently-stuck findings ahead of it the stale
/// disjunct silently returns nothing.
///
/// Every other fixture in this family is smaller than the cap, so nothing else in the suite can
/// observe this: the assertion is not "stale findings sort first" but "stale findings get slots AT
/// ALL when the uncovered class alone would fill the cap."
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn sweep_ordering_gives_stale_findings_slots_under_a_stuck_backlog(pool: sqlx::PgPool) {
    let f = stale_fixture(&pool, "ads-stale-order").await;

    // The stale finding: covered, then mutated.
    let block = first_block(&pool, f.finding).await;
    fire_audit(&pool, f.auditor, block, f.source.uuid(), 0.5)
        .await
        .unwrap();
    mutate_block(&pool, block, f.emitter, "revised after the audit").await;

    // Four never-audited findings, each `uncovered = 1` — the permanently-stuck class, and more of
    // them than the cap below.
    for i in 0..4 {
        let home = make_home(&pool, f.owner, &format!("ads-stale-order-stuck-home{i}")).await;
        let src = make_resource(
            &pool,
            f.owner,
            f.emitter,
            home,
            &format!("stuck-src{i}"),
            &format!("temper://ads-stale-order/stuck-src{i}"),
        )
        .await;
        make_cogmap_finding(
            &pool,
            f.owner,
            f.emitter,
            CogmapId::from(f.cogmap),
            &format!("stuck{i}"),
            &format!("temper://ads-stale-order/stuck{i}"),
            Some(src),
        )
        .await;
    }

    // Cap of 3 against 4 uncovered findings: under `ORDER BY uncovered DESC` the uncovered class
    // takes every slot and the stale finding is structurally excluded, not merely deprioritized.
    let rows = sweep(&pool, f.principal.uuid(), 3).await;
    assert_eq!(
        rows.len(),
        3,
        "the cap must be the binding constraint: {rows:?}"
    );
    assert!(
        rows.iter().any(|(_, r, _)| *r == f.finding.uuid()),
        "the stale finding must get a slot even though four uncovered findings could fill the cap: \
         {rows:?}"
    );
}

/// STAY-GREEN, and the guard on the `block_wm IS NULL` arm.
///
/// `finding_wm IS NOT NULL` restricts that arm to a principal already engaged with this
/// `(finding, source)`. Drop it as a tidy-up and every finding reads stale for every principal who
/// has not yet audited it — silently converting staleness into per-principal COVERAGE, which is a far
/// larger behaviour change and one the band's coverage-ratio gate already handles.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn sweep_omits_a_finding_for_a_principal_who_never_audited_it(pool: sqlx::PgPool) {
    let f = stale_fixture(&pool, "ads-stale-unengaged").await;
    let (b_principal, _b_auditor) = make_auditor(&pool, "ads-stale-unengaged-b").await;
    join_principal_to_cogmap(
        &pool,
        b_principal.uuid(),
        f.cogmap,
        "ads-stale-unengaged-b-team",
    )
    .await;

    let block = first_block(&pool, f.finding).await;
    fire_audit(&pool, f.auditor, block, f.source.uuid(), 0.5)
        .await
        .unwrap();

    assert!(
        !sweep_offers(&pool, b_principal, f.cogmap, f.finding).await,
        "B has never audited this finding, and it is globally covered — staleness must stay silent \
         rather than offering every finding to every principal who has not looked yet"
    );
}

/// LOAD-BEARING, and the witness for the `gated` CTE itself.
///
/// The gate is **redundant for today's only caller** — `audit_drift_sweep` filters `candidates`
/// through `steward_candidate_cogmaps` and `resources_visible_to` before scoring — so every other
/// test in this family passes with or without it. That makes it decoration unless something
/// exercises it directly, which is what this does: it calls `resource_has_stale_citation` itself,
/// the way a future second caller would.
///
/// The bite needs a principal that has a WATERMARK but no VISIBILITY. A principal who simply never
/// audited returns false through `finding_wm IS NULL` and would pass ungated too — proving nothing.
/// So A audits while joined to the cogmap, the finding goes stale, and only then is A's team
/// detached: the audits remain, the visibility does not.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn stale_predicate_refuses_a_finding_the_principal_cannot_read(pool: sqlx::PgPool) {
    let f = stale_fixture(&pool, "ads-stale-gate").await;
    let block = first_block(&pool, f.finding).await;
    fire_audit(&pool, f.auditor, block, f.source.uuid(), 0.5)
        .await
        .unwrap();
    mutate_block(&pool, block, f.emitter, "revised after the audit").await;

    let stale_while_visible: bool =
        sqlx::query_scalar("SELECT resource_has_stale_citation($1, $2)")
            .bind(f.finding.uuid())
            .bind(f.principal.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        stale_while_visible,
        "precondition: with reach, this principal's watermark predates the mutation, so it IS stale"
    );

    // Detach the team from the cogmap — `cogmap_readable_by_profile` reads exactly this row, and it
    // is the principal's ONLY path to a cogmap-homed finding. The audits it already wrote survive.
    sqlx::query("DELETE FROM kb_team_cogmaps WHERE cogmap_id = $1")
        .bind(f.cogmap)
        .execute(&pool)
        .await
        .unwrap();

    let stale_without_reach: bool =
        sqlx::query_scalar("SELECT resource_has_stale_citation($1, $2)")
            .bind(f.finding.uuid())
            .bind(f.principal.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        !stale_without_reach,
        "the watermark still exists but the finding is no longer readable — the `gated` CTE must \
         refuse it rather than answer a question about a resource this principal cannot see"
    );
}

// ── Citation-grain dispatch — `resource_auditable_citations` ────────────────────────────────────
//
// Migration `20260727000050_auditable_citations_at_citation_grain.sql`, task
// `019fa5eb-2819-7e71-bd27-0d2875f60960`.
//
// THE DEFECT THESE WITNESS: the dispatch payload named findings while the auditor's unit of work is
// a citation, so a finding selected on its LAST uncovered citation was worked in full and every
// already-weighed citation of it was weighed again. Reproduced on production 2026-07-27 — finding
// `019f5cdd-ba0b-7fa2-90b1-e78ca7979254`, 1 of 8 weighed, still swept at `uncovered: 7`.
//
// These reuse `stale_fixture` and audit with the sweeping principal's own entity, for the reason in
// the tier-1 header above: `audited_by_profile_id` resolves from the owning event's emitter, so
// auditing as `system` while asking about `principal` would make every assertion vacuously true.

/// The `(block, source)` pairs this principal should weigh, ordered so assertions are positional.
async fn auditable_citations(
    pool: &sqlx::PgPool,
    finding: ResourceId,
    principal: ProfileId,
) -> Vec<(Uuid, Uuid)> {
    sqlx::query_as(
        "SELECT block_id, source_id FROM resource_auditable_citations($1, $2) \
          ORDER BY block_id, source_id",
    )
    .bind(finding.uuid())
    .bind(principal.uuid())
    .fetch_all(pool)
    .await
    .unwrap()
}

/// Add a second live cited source to `block`, so a finding can be PARTIALLY weighed.
async fn second_source(
    pool: &sqlx::PgPool,
    f: &StaleFixture,
    block: Uuid,
    slug: &str,
) -> ResourceId {
    let home = make_home(pool, f.owner, &format!("{slug}-src2-home")).await;
    let source = make_resource(
        pool,
        f.owner,
        f.emitter,
        home,
        "source",
        &format!("temper://{slug}/source2"),
    )
    .await;
    cite(pool, block, source.uuid()).await;
    source
}

/// LOAD-BEARING — this is the defect itself. A citation this principal has weighed must not be
/// offered back to it, and the finding must still be dispatched with the REMAINDER rather than
/// dropped. Both halves in one test on purpose: a fix that withheld the whole finding once any
/// citation was weighed would satisfy the first half and silently starve the rest, and would look
/// from the queue's side exactly like draining correctly.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_weighed_citation_is_withheld_and_the_remainder_still_offered(pool: sqlx::PgPool) {
    let f = stale_fixture(&pool, "auditable-remainder").await;
    let block = first_block(&pool, f.finding).await;
    let source2 = second_source(&pool, &f, block, "auditable-remainder").await;

    let before = auditable_citations(&pool, f.finding, f.principal).await;
    assert_eq!(
        before.len(),
        2,
        "both live citations are auditable before anything is weighed: got {before:?}"
    );

    fire_audit(&pool, f.auditor, block, f.source.uuid(), 0.5)
        .await
        .unwrap();

    let after = auditable_citations(&pool, f.finding, f.principal).await;
    assert_eq!(
        after,
        vec![(block, source2.uuid())],
        "exactly the unweighed citation remains — the weighed one is withheld and the finding is \
         NOT skipped wholesale"
    );
}

/// The no-repeater invariant is PER PRINCIPAL, and this is the half that keeps it from becoming a
/// global suppression. Cross-principal audit is the entire premise of the substrate; a second
/// auditor must still be offered a citation the first one weighed.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn another_principal_is_still_offered_a_citation_this_one_weighed(pool: sqlx::PgPool) {
    let f = stale_fixture(&pool, "auditable-cross").await;
    let block = first_block(&pool, f.finding).await;

    fire_audit(&pool, f.auditor, block, f.source.uuid(), 0.5)
        .await
        .unwrap();
    assert!(
        auditable_citations(&pool, f.finding, f.principal)
            .await
            .is_empty(),
        "the auditing principal has nothing left to weigh on this finding"
    );

    let (other, _other_entity) = make_auditor(&pool, "auditable-cross-other").await;
    join_principal_to_cogmap(&pool, other.uuid(), f.cogmap, "auditable-cross-other-team").await;

    assert_eq!(
        auditable_citations(&pool, f.finding, other).await,
        vec![(block, f.source.uuid())],
        "a DIFFERENT principal is still offered it — suppressing this would collapse \
         'assessed by another party' exactly the way a shared credential would"
    );
}

/// Withholding is not permanent. Once the cited block changes, the citation re-enters this
/// principal's work — otherwise the fix for re-auditing would have created the frozen-verdict defect
/// tier-1 staleness exists to prevent, one grain down.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_weighed_citation_re_enters_once_it_goes_stale(pool: sqlx::PgPool) {
    let f = stale_fixture(&pool, "auditable-stale").await;
    let block = first_block(&pool, f.finding).await;

    fire_audit(&pool, f.auditor, block, f.source.uuid(), 0.5)
        .await
        .unwrap();
    assert!(
        auditable_citations(&pool, f.finding, f.principal)
            .await
            .is_empty(),
        "withheld immediately after being weighed"
    );

    mutate_block(
        &pool,
        block,
        f.emitter,
        "the assertion now says something else",
    )
    .await;

    assert_eq!(
        auditable_citations(&pool, f.finding, f.principal).await,
        vec![(block, f.source.uuid())],
        "the block changed under the verdict, so the citation is auditable again"
    );
}

/// The RBAC gate is inherited, not re-implemented. An unreadable finding yields zero rows rather
/// than raising or leaking, matching the zero-rows-to-404 dialect the audit write gate answers in —
/// so this producer can never become an existence oracle.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_unreadable_finding_offers_no_auditable_citations(pool: sqlx::PgPool) {
    let f = stale_fixture(&pool, "auditable-gated").await;
    let (stranger, _entity) = make_auditor(&pool, "auditable-gated-stranger").await;

    assert!(
        !auditable_citations(&pool, f.finding, f.principal)
            .await
            .is_empty(),
        "control: the reaching principal does see work, so the next assertion is about \
         READABILITY and not about an empty fixture"
    );
    assert!(
        auditable_citations(&pool, f.finding, stranger)
            .await
            .is_empty(),
        "a principal with no path to the cogmap is told nothing about this finding"
    );
}

/// The extraction's own invariant: the boolean and the set cannot disagree, because the boolean IS
/// an EXISTS over the set. Asserted rather than assumed — the whole point of extracting the body was
/// that two copies of this predicate would drift, and nothing else in the suite compares them.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_staleness_boolean_agrees_with_the_staleness_set(pool: sqlx::PgPool) {
    let f = stale_fixture(&pool, "auditable-agree").await;
    let block = first_block(&pool, f.finding).await;

    for stage in [
        "before any audit",
        "after auditing",
        "after a block mutation",
    ] {
        let boolean: bool = sqlx::query_scalar("SELECT resource_has_stale_citation($1, $2)")
            .bind(f.finding.uuid())
            .bind(f.principal.uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
        let set_nonempty: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM resource_stale_citations($1, $2))")
                .bind(f.finding.uuid())
                .bind(f.principal.uuid())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            boolean, set_nonempty,
            "boolean and set must agree ({stage}) — they are one definition since 20260727000050"
        );

        match stage {
            "before any audit" => {
                fire_audit(&pool, f.auditor, block, f.source.uuid(), 0.5)
                    .await
                    .unwrap();
            }
            "after auditing" => {
                mutate_block(&pool, block, f.emitter, "revised prose").await;
            }
            _ => {}
        }
    }
}
