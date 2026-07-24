#![cfg(feature = "artifact-tests")]
//! Set 5, Task 1 — the append-only citation-audit table, event, and write path (SQL).
//!
//! Exercises `citation_audit` / `_project_citation_audited` in
//! `migrations/20260723000010_citation_audits.sql` against an ephemeral DB. There is no Rust
//! wrapper yet (that is a later task) — these tests call the SQL entry function and projector
//! directly, the same idiom `content_mutation.rs` uses for `block_mutate`
//! (`content_mutation.rs:91-92`: `serde_json::json!` payload, positional `$1`/`$2` binds).
//!
//! Harness + seeding helpers are local copies of `tests/evidential_standing.rs`'s
//! `system_actor`/`make_home`/`make_resource` (duplicated per file across this test suite by
//! established convention — `block_content.rs`, `invocation_envelope.rs`, `search_index.rs`,
//! `search_surface_a.rs`, and `write_path_mutations.rs` each carry their own copy).

mod common;

use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::AnchorRef;
use temper_substrate::scenario::bootseed;
use temper_substrate::writes::{self, CreateParams};
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

/// Fire `citation_audit(payload, emitter)` with a `source_kind: "resource"` citation, returning
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
        "source_kind": "resource",
        "source_id": source,
        "value": value,
        "reason": "test audit",
    });
    sqlx::query_scalar::<_, Uuid>("SELECT citation_audit($1, $2)")
        .bind(&payload)
        .bind(emitter.uuid())
        .fetch_one(pool)
        .await
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
        "source_kind": "remote",
        "source_id": Uuid::now_v7(),
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
        "source_kind": "resource",
        "source_id": source.uuid(),
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
