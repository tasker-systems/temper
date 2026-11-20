#![cfg(feature = "test-db")]
//! `resource_erased`'s legacy erasure and the legacy backfill (erasure estate-membership spec D9,
//! R21; migration 20261201000030_erasure_record_projectors.sql: `_project_resource_erased_membership`,
//! the re-issued `_project_resource_erased_redactions`, and `_erasure_legacy_backfill`).
//!
//! A cut-1 husk is made here as cut 1's act made one ([`simulate_cut1_husk`], a local copy of the
//! helper in `resource_erasure_act.rs` without its backfill call): a first record carrying
//! `targets`, appended without the wrapper, then the projection-side body. The ledger keeps its
//! text, so the husk still owes a completion pass. A cut-2 erasure runs the act itself.
//!
//! Replay walks the ledger in id order, and `uuid_generate_v7()` has no order within a millisecond
//! (20261116000010_kb_events_record_order.sql). [`pause`] waits two milliseconds before each step
//! whose order a test relies on, and the replay witness asserts that order before it compares.
//!
//! Witnesses:
//!   * a cut-1 husk is backfilled as an `erased` and a `pending` legacy erasure, the second
//!     declared by one legacy `erasure_declared` under the canonical system actor;
//!   * an old-door completion pass then closes the `pending` row, and replay of the whole sequence
//!     is byte-identical;
//!   * a second backfill appends nothing and leaves the `pending` row open, including around an
//!     earlier completion record (the backfill never re-projects one);
//!   * a cut-2 erasure is one `erased` legacy erasure and owes no backfill event;
//!   * a `resource_erased` of neither shape raises, and an erased resource with no record raises;
//!   * a completion record never touches a non-legacy erasure, and clears the drain columns of the
//!     legacy row it closes;
//!   * the migration is a no-op on an empty ledger, and run over a ledger made on the schema
//!     before it, it records both husks as their derivation implies.

mod common;

use std::borrow::Cow;

use sqlx::migrate::Migrator;
use sqlx::PgPool;
use temper_core::types::ids::EntityId;
use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, ProfileId};
use temper_substrate::payloads::{
    AnchorRef, ErasureDeclared, ErasureKind, ErasureMapEntry, ErasureMemberTable,
};
use temper_substrate::replay;
use temper_substrate::writes::{self, CreateParams};
use uuid::Uuid;

/// The first migration of slice 1b: the erasure record's schema.
const FIRST_1B_MIGRATION: i64 = 20261201000010;

const DIGEST: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

// ── the world ────────────────────────────────────────────────────────────────────────────────

async fn fresh(pool: &PgPool) {
    common::reset_schema(pool).await;
    temper_substrate::scenario::bootseed::seed_system(pool)
        .await
        .unwrap();
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

async fn make_context(pool: &PgPool, owner: ProfileId, slug: &str) -> ContextId {
    ContextId::from(
        common::insert_context(pool, "kb_profiles", owner.uuid(), slug, slug)
            .await
            .unwrap(),
    )
}

/// A resource whose title the ledger carries, so a cut-1 husk of it owes a completion pass.
async fn make_resource(pool: &PgPool, slug: &str) -> Uuid {
    let (owner, emitter) = system_actor(pool).await;
    let home = make_context(pool, owner, &format!("{slug}-home")).await;
    let origin = format!("test://erasure-legacy-backfill/{slug}");
    writes::create_resource_with(
        pool,
        CreateParams {
            idempotency_key: None,
            title: slug,
            origin_uri: &origin,
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
    .unwrap()
    .uuid()
}

/// Two milliseconds, so the next append lands in a later millisecond than every earlier one.
async fn pause(pool: &PgPool) {
    sqlx::query("SELECT pg_sleep(0.002)")
        .execute(pool)
        .await
        .unwrap();
}

/// A husk as cut 1 left it, in the cut-1 act's own transaction: the plan, a `resource_erased`
/// first record that carries `targets` and names no `redacted_fields`, appended without the
/// wrapper as cut 1 appended it, then the projection-side body. The ledger's text is untouched.
/// A local copy of `resource_erasure_act.rs`'s helper, without its backfill call; it uses only
/// functions that exist before 1b, so the migration-run test builds one on the older schema.
/// Returns the first record's id.
async fn simulate_cut1_husk(pool: &PgPool, resource: Uuid) -> Uuid {
    let (operator, emitter) = system_actor(pool).await;
    let request_ref = Uuid::now_v7();
    let mut tx = pool.begin().await.unwrap();
    let plan: serde_json::Value = sqlx::query_scalar("SELECT resource_erasure_survey_plan($1)")
        .bind(resource)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    assert!(
        plan["edges"].as_array().unwrap().is_empty(),
        "setup: no edge to fold"
    );
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

/// The resource act, cut 2's: a first record live, or a completion pass on a husk. Returns its
/// `resource_erased` event.
async fn execute_act(pool: &PgPool, resource: Uuid) -> Uuid {
    let (_, operator) = system_actor(pool).await;
    let raw: String =
        sqlx::query_scalar("SELECT (resource_erasure_execute($1,$2,$3,$4)->>'event_id')::text")
            .bind(resource)
            .bind(operator)
            .bind(operator)
            .bind(Uuid::now_v7())
            .fetch_one(pool)
            .await
            .expect("the act completes");
    Uuid::parse_str(&raw).expect("the event id parses")
}

async fn try_backfill(pool: &PgPool) -> Result<i32, sqlx::Error> {
    sqlx::query_scalar("SELECT _erasure_legacy_backfill()")
        .fetch_one(pool)
        .await
}

/// The backfill, after a pause so its appends sort after everything before them.
async fn backfill(pool: &PgPool) -> i32 {
    pause(pool).await;
    try_backfill(pool).await.expect("the backfill runs")
}

/// One legacy erasure of a resource: its id, its declaring event, and its one member's status and
/// erasing event.
type LegacyRow = (Uuid, Uuid, String, Option<Uuid>);

/// The resource's legacy erasures, a first record's (its id is its declaring event's) before any
/// the backfill declared.
async fn legacy_of(pool: &PgPool, resource: Uuid) -> Vec<LegacyRow> {
    sqlx::query_as(
        "SELECT e.id, e.declared_event_id, m.status, m.erased_by_event_id \
           FROM kb_erasures e \
           JOIN kb_erasure_members m ON m.erasure_id = e.id \
          WHERE e.legacy AND m.member_table = 'kb_resources' AND m.member_id = $1 \
          ORDER BY e.id <> e.declared_event_id, e.declared_event_id",
    )
    .bind(resource)
    .fetch_all(pool)
    .await
    .unwrap()
}

/// The `erasure_declared` events: (id, emitter, payload), in id order.
async fn declarations(pool: &PgPool) -> Vec<(Uuid, Uuid, serde_json::Value)> {
    sqlx::query_as(
        "SELECT e.id, e.emitter_entity_id, e.payload \
           FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'erasure_declared' ORDER BY e.id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

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

/// Append a `resource_erased` record by hand and project it through the wrapper in the same
/// transaction, as the act does. Returns the event id.
async fn record_resource_erased(pool: &PgPool, payload: &serde_json::Value) -> Uuid {
    pause(pool).await;
    let (_, emitter) = system_actor(pool).await;
    let mut tx = pool.begin().await.unwrap();
    let event: Uuid = sqlx::query_scalar(
        "SELECT _event_append('resource_erased', $1, NULL, NULL, $2, p_correlation => $3)",
    )
    .bind(emitter)
    .bind(payload)
    .bind(Uuid::now_v7())
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query("SELECT _project_resource_erased_redactions($1, $2)")
        .bind(event)
        .bind(payload)
        .execute(&mut *tx)
        .await
        .expect("the wrapper projects the record");
    tx.commit().await.unwrap();
    event
}

/// A completion-shaped `resource_erased` payload that rewrites nothing.
fn completion_payload(resource: Uuid) -> serde_json::Value {
    serde_json::json!({
        "subject_table": "kb_resources",
        "subject_id": resource,
        "redacted_fields": [],
    })
}

/// Append an `erasure_declared` and project it in the same transaction. Returns the event id.
async fn declare(pool: &PgPool, d: &ErasureDeclared) -> Uuid {
    pause(pool).await;
    let payload = serde_json::to_value(d).unwrap();
    let (_, emitter) = system_actor(pool).await;
    let mut tx = pool.begin().await.unwrap();
    let event: Uuid = sqlx::query_scalar(
        "SELECT _event_append('erasure_declared', $1, NULL, NULL, $2, p_correlation => $3)",
    )
    .bind(emitter)
    .bind(&payload)
    .bind(d.erasure_id)
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    sqlx::query("SELECT _project_erasure_declared($1, $2)")
        .bind(event)
        .bind(&payload)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    event
}

/// A resource erasure of one pending member, legacy or not.
fn resource_declaration(erasure: Uuid, resource: Uuid, legacy: bool) -> ErasureDeclared {
    ErasureDeclared {
        erasure_id: erasure,
        kind: ErasureKind::Resource,
        subject_table: Some("kb_resources".to_owned()),
        subject_id: Some(resource),
        request_ref: (!legacy).then(Uuid::now_v7),
        manifest_version: (!legacy).then(|| "1.0.0".to_owned()),
        manifest_digest: (!legacy).then(|| DIGEST.to_owned()),
        map: vec![ErasureMapEntry {
            table: ErasureMemberTable::KbResources,
            id: resource,
            parent: None,
            prior_erasure: None,
            prior_event: None,
        }],
        counts: None,
        checksum: None,
        holders: vec![],
        legacy,
    }
}

async fn member_status(pool: &PgPool, erasure: Uuid, resource: Uuid) -> (String, Option<Uuid>) {
    sqlx::query_as(
        "SELECT status, erased_by_event_id FROM kb_erasure_members \
          WHERE erasure_id = $1 AND member_table = 'kb_resources' AND member_id = $2",
    )
    .bind(erasure)
    .bind(resource)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// A fresh world holding one backfilled cut-1 husk: (resource, first record, the backfill's
/// `erasure_declared`, the backfill's erasure).
async fn backfilled_husk(pool: &PgPool) -> (Uuid, Uuid, Uuid, Uuid) {
    fresh(pool).await;
    let r = make_resource(pool, "cut1-husk").await;
    let first = simulate_cut1_husk(pool, r).await;
    assert_eq!(backfill(pool).await, 1, "one husk owes a completion pass");
    let decls = declarations(pool).await;
    assert_eq!(decls.len(), 1);
    let erasure: Uuid = decls[0].2["erasure_id"].as_str().unwrap().parse().unwrap();
    (r, first, decls[0].0, erasure)
}

// ── the backfill ─────────────────────────────────────────────────────────────────────────────

/// A fresh cut-1 husk is backfilled as two legacy erasures: the first record's, `erased` by it,
/// then one the backfill declared, `pending`, under one legacy `erasure_declared` the canonical
/// system actor emitted.
///
/// FAILS IF: the first record's erasure is missing or not erased by it; no `pending` row is
/// declared for the owed pass; the declaration is not legacy, names another subject or map, or is
/// emitted by anyone but the system actor.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_cut1_husk_is_backfilled_as_an_erased_and_a_pending_legacy_erasure(pool: PgPool) {
    let (r, first, declared, erasure) = backfilled_husk(&pool).await;
    assert_eq!(
        legacy_of(&pool, r).await,
        vec![
            (first, first, "erased".to_owned(), Some(first)),
            (erasure, declared, "pending".to_owned(), None),
        ]
    );

    let (_, system) = system_actor(&pool).await;
    let decls = declarations(&pool).await;
    assert_eq!(
        decls[0].1,
        system.uuid(),
        "emitted by the canonical system actor"
    );
    let payload: ErasureDeclared = serde_json::from_value(decls[0].2.clone()).unwrap();
    assert!(payload.legacy);
    assert_eq!(payload.kind, ErasureKind::Resource);
    assert_eq!(payload.subject_table.as_deref(), Some("kb_resources"));
    assert_eq!(payload.subject_id, Some(r));
    assert_eq!(
        payload.map,
        vec![ErasureMapEntry {
            table: ErasureMemberTable::KbResources,
            id: r,
            parent: None,
            prior_erasure: None,
            prior_event: None,
        }]
    );
    assert!(payload.request_ref.is_none() && payload.manifest_digest.is_none());
    assert!(payload.counts.is_none() && payload.checksum.is_none());
    assert!(payload.holders.is_empty());
}

/// An old-door completion pass marks the backfill's `pending` row `erased` by its record, and
/// leaves the first record's row as it was.
///
/// FAILS IF: the completion record leaves the backfill's row open, or names another event.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_old_door_completion_pass_closes_the_backfilled_pending_row(pool: PgPool) {
    let (r, first, declared, erasure) = backfilled_husk(&pool).await;
    pause(&pool).await;
    let completion = execute_act(&pool, r).await;
    assert_eq!(
        legacy_of(&pool, r).await,
        vec![
            (first, first, "erased".to_owned(), Some(first)),
            (erasure, declared, "erased".to_owned(), Some(completion)),
        ]
    );
}

/// Replay of the sequence (cut-1 husk, backfill, completion pass) rebuilds every projection byte
/// for byte, the legacy erasures among them.
///
/// The precondition first: the backfill's `erasure_declared` sorts below the completion record,
/// as replay walks them. A same-millisecond inversion fails here, loudly, and not as a diff below.
///
/// FAILS IF: replay of the first record, the backfill's declaration, or the completion record
/// writes any row the live sequence did not.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn replay_of_a_backfilled_and_completed_husk_is_byte_identical(pool: PgPool) {
    let (r, first, declared, _) = backfilled_husk(&pool).await;
    pause(&pool).await;
    let completion = execute_act(&pool, r).await;
    assert!(
        first < declared && declared < completion,
        "precondition: the first record, the backfill's erasure_declared and the completion record \
         sort in append order ({first}, {declared}, {completion})"
    );
    assert_replay_byte_identical(&pool, "after a backfilled husk's completion pass").await;
}

/// A second backfill appends nothing and leaves the `pending` row open: the owed pass is already
/// recorded, and the first record's re-projection is a no-op.
///
/// FAILS IF: the second call appends a declaration, re-projects anything over the open row, or
/// raises on the first record's existing rows.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_second_backfill_appends_nothing_and_leaves_the_pending_row_open(pool: PgPool) {
    let (r, first, declared, erasure) = backfilled_husk(&pool).await;
    assert_eq!(backfill(&pool).await, 0, "nothing is appended twice");
    assert_eq!(declarations(&pool).await.len(), 1);
    assert_eq!(
        legacy_of(&pool, r).await,
        vec![
            (first, first, "erased".to_owned(), Some(first)),
            (erasure, declared, "pending".to_owned(), None),
        ]
    );
}

/// The backfill never re-projects a completion record (P11). A husk carries an earlier completion
/// record that rewrote nothing, so it still owes a pass: the backfill declares it `pending`, and a
/// second call leaves that row open. Re-projecting the earlier completion record would close it.
///
/// FAILS IF: the backfill re-projects completion records, or skips the husk because a completion
/// record exists.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_backfill_around_an_earlier_completion_record_leaves_the_pending_row_open(
    pool: PgPool,
) {
    fresh(&pool).await;
    let r = make_resource(&pool, "earlier-completion").await;
    let first = simulate_cut1_husk(&pool, r).await;
    let earlier = record_resource_erased(&pool, &completion_payload(r)).await;
    let owed: serde_json::Value =
        sqlx::query_scalar("SELECT resource_erasure_completion_fields($1)")
            .bind(r)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        owed.as_array().is_some_and(|a| !a.is_empty()),
        "precondition: the earlier completion record rewrote nothing, so a pass is still owed"
    );

    assert_eq!(backfill(&pool).await, 1);
    let rows = legacy_of(&pool, r).await;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0], (first, first, "erased".to_owned(), Some(first)));
    assert_eq!(
        (rows[1].2.as_str(), rows[1].3),
        ("pending", None),
        "the backfill's row is born pending after the earlier completion record ({earlier})"
    );

    assert_eq!(backfill(&pool).await, 0);
    assert_eq!(
        legacy_of(&pool, r).await,
        rows,
        "a second call leaves the backfill's pending row open"
    );
}

/// A cut-2 erasure is recorded live by the wrapper as one `erased` legacy erasure, and owes no
/// completion pass, so the backfill appends nothing for it.
///
/// FAILS IF: the act's first record is not recorded, is recorded twice, or the backfill declares a
/// pass the act does not owe.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_cut2_erasure_is_one_erased_legacy_erasure_and_owes_no_backfill(pool: PgPool) {
    fresh(&pool).await;
    let r = make_resource(&pool, "cut2").await;
    let event = execute_act(&pool, r).await;
    let live = legacy_of(&pool, r).await;
    assert_eq!(live, vec![(event, event, "erased".to_owned(), Some(event))]);

    assert_eq!(backfill(&pool).await, 0);
    assert!(declarations(&pool).await.is_empty());
    assert_eq!(legacy_of(&pool, r).await, live);
}

/// An erased resource with no `resource_erased` record raises the backfill's named error, and so
/// would abort the migration.
///
/// FAILS IF: the backfill skips the resource, or raises some other error.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn an_erased_resource_without_a_record_raises(pool: PgPool) {
    fresh(&pool).await;
    let r = make_resource(&pool, "recordless").await;
    sqlx::query("UPDATE kb_resources SET erased_at = now(), is_active = false WHERE id = $1")
        .bind(r)
        .execute(&pool)
        .await
        .unwrap();
    let err = try_backfill(&pool)
        .await
        .expect_err("a husk with no record raises");
    assert!(
        err.to_string().contains(&format!(
            "_erasure_legacy_backfill: erased resource {r} has no resource_erased record"
        )),
        "{err}"
    );
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("P0002")
    );
}

/// The migration is a no-op on an empty ledger: the freshly migrated database holds no erasure
/// record, and the backfill run over it again appends nothing.
///
/// FAILS IF: the migration or the backfill writes any record row, or appends any event, with no
/// erased resource to account for.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn the_backfill_is_a_noop_on_an_empty_ledger(pool: PgPool) {
    let counts = || async {
        sqlx::query_as::<_, (i64, i64, i64)>(
            "SELECT (SELECT count(*) FROM kb_erasures), \
                    (SELECT count(*) FROM kb_erasure_members), \
                    (SELECT count(*) FROM kb_events)",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    let (erasures, members, _) = counts().await;
    assert_eq!(
        (erasures, members),
        (0, 0),
        "the migration recorded nothing"
    );
    let before = counts().await;
    assert_eq!(try_backfill(&pool).await.unwrap(), 0);
    assert_eq!(counts().await, before, "the backfill wrote nothing");

    // Again on the replay baseline: every migration applied to a fresh schema, then seeded.
    fresh(&pool).await;
    let before = counts().await;
    assert_eq!((before.0, before.1), (0, 0));
    assert_eq!(try_backfill(&pool).await.unwrap(), 0);
    assert_eq!(counts().await, before);
}

// ── the membership projector ─────────────────────────────────────────────────────────────────

/// A `resource_erased` that is neither a first record (no `targets`) nor a completion record (a
/// key outside the completion set) raises the projector's named error.
///
/// FAILS IF: the payload is read as a completion record and quietly projects nothing.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_resource_erased_of_neither_shape_raises(pool: PgPool) {
    fresh(&pool).await;
    let r = make_resource(&pool, "neither").await;
    let event = Uuid::now_v7();
    let mut payload = completion_payload(r);
    payload["folded_edges"] = serde_json::json!([]);
    let err = sqlx::query("SELECT _project_resource_erased_membership($1, $2)")
        .bind(event)
        .bind(&payload)
        .execute(&pool)
        .await
        .expect_err("a payload of neither shape raises");
    assert!(
        err.to_string().contains(&format!(
            "_project_resource_erased_membership: resource_erased {event} is neither a first \
             record nor a completion record"
        )),
        "{err}"
    );
    assert_eq!(
        err.as_database_error().and_then(|e| e.code()).as_deref(),
        Some("23514")
    );

    // Control: the same payload without the stray key is a completion record, and projects.
    sqlx::query("SELECT _project_resource_erased_membership($1, $2)")
        .bind(event)
        .bind(completion_payload(r))
        .execute(&pool)
        .await
        .expect("a completion-shaped payload projects");
}

/// A completion record marks erased the subject's open legacy row and leaves its pending row in a
/// non-legacy erasure untouched.
///
/// FAILS IF: the completion record closes a non-legacy erasure's row, or (the control) closes no
/// legacy row at all.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_completion_record_leaves_a_non_legacy_pending_row_untouched(pool: PgPool) {
    fresh(&pool).await;
    let r = make_resource(&pool, "non-legacy").await;
    let legacy = Uuid::now_v7();
    declare(&pool, &resource_declaration(legacy, r, true)).await;
    let declared = Uuid::now_v7();
    declare(&pool, &resource_declaration(declared, r, false)).await;

    let completion = record_resource_erased(&pool, &completion_payload(r)).await;
    assert_eq!(
        member_status(&pool, legacy, r).await,
        ("erased".to_owned(), Some(completion)),
        "control: the legacy row is closed by the record"
    );
    assert_eq!(
        member_status(&pool, declared, r).await,
        ("pending".to_owned(), None),
        "the non-legacy row is the drain's to finish"
    );
}

/// A completion record that marks a pending legacy row `erased` also clears its drain columns.
/// The row's CHECK refuses an erased row with either set, so a projector that did not clear them
/// raises.
///
/// FAILS IF: the completion record leaves `next_attempt_at` or `lease_expires_at` set, or fails to
/// close the row.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_completion_record_clears_the_drain_columns_of_the_row_it_closes(pool: PgPool) {
    fresh(&pool).await;
    let r = make_resource(&pool, "drained").await;
    let legacy = Uuid::now_v7();
    declare(&pool, &resource_declaration(legacy, r, true)).await;
    sqlx::query(
        "UPDATE kb_erasure_members \
            SET next_attempt_at = now(), lease_expires_at = now() + interval '1 minute' \
          WHERE erasure_id = $1",
    )
    .bind(legacy)
    .execute(&pool)
    .await
    .unwrap();

    let completion = record_resource_erased(&pool, &completion_payload(r)).await;
    let (status, by, next, lease): (
        String,
        Option<Uuid>,
        Option<chrono::DateTime<chrono::Utc>>,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as(
        "SELECT status, erased_by_event_id, next_attempt_at, lease_expires_at \
           FROM kb_erasure_members WHERE erasure_id = $1",
    )
    .bind(legacy)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((status.as_str(), by), ("erased", Some(completion)));
    assert_eq!((next, lease), (None, None), "the closed row left the drain");
}

// ── the migration, run ───────────────────────────────────────────────────────────────────────

/// A database no migration has touched, beside the test's own, and its name. As in
/// `temper-services`' `sensitivity_default_off_test.rs`: CI copies every `#[sqlx::test]` database
/// from a `template1` that already holds the whole chain, and `migrations = false` stops sqlx from
/// migrating, not Postgres from copying. `template0` is never migrated. The name derives from the
/// test's database, so a copy a failed run leaked is dropped by the next run.
async fn unmigrated(pool: &PgPool) -> (PgPool, String) {
    // Hashed, not suffixed: sqlx's test database names already fill Postgres's 63-byte identifier
    // limit.
    let name: String = sqlx::query_scalar("SELECT 'unmigrated_' || md5(current_database())")
        .fetch_one(pool)
        .await
        .unwrap();
    for ddl in [
        format!(r#"DROP DATABASE IF EXISTS "{name}" WITH (FORCE)"#),
        format!(r#"CREATE DATABASE "{name}" TEMPLATE template0"#),
    ] {
        sqlx::query(&ddl).execute(pool).await.unwrap();
    }
    let options = pool.connect_options().as_ref().clone().database(&name);
    (PgPool::connect_with(options).await.unwrap(), name)
}

/// The migration, run over a ledger made on the schema before it: the declared-limitation witness
/// for the husks a deployment holds. The chain before 1b is applied, a cut-1 husk and a cut-2
/// erasure are made on it, and then the rest of the chain runs, 20261201000030's backfill among it.
/// The husk is an `erased` and a `pending` legacy erasure, the cut-2 erasure one `erased`, and
/// exactly one legacy `erasure_declared` was appended, under the canonical system actor.
///
/// FAILS IF: the migration does not run the backfill, raises over these husks, records either
/// erasure otherwise, or appends any other declaration.
#[sqlx::test(migrations = false)]
async fn the_migration_records_the_husks_of_a_ledger_made_before_it(own: PgPool) {
    let (pool, name) = unmigrated(&own).await;
    let all = &temper_substrate::MIGRATOR;
    let before_1b = Migrator {
        migrations: Cow::Owned(
            all.migrations
                .iter()
                .filter(|m| m.version < FIRST_1B_MIGRATION)
                .cloned()
                .collect(),
        ),
        ignore_missing: all.ignore_missing,
        locking: all.locking,
        no_tx: all.no_tx,
    };
    before_1b
        .run(&pool)
        .await
        .expect("the chain before 1b applies");
    let tables: i64 =
        sqlx::query_scalar("SELECT count(*) FROM pg_tables WHERE tablename = 'kb_erasures'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(tables, 0, "setup: the schema is the one before 1b");

    let husk = make_resource(&pool, "pre-1b-husk").await;
    let first = simulate_cut1_husk(&pool, husk).await;
    let cut2 = make_resource(&pool, "pre-1b-cut2").await;
    let cut2_event = execute_act(&pool, cut2).await;
    pause(&pool).await;

    all.run(&pool).await.expect("the rest of the chain applies");

    let rows = legacy_of(&pool, husk).await;
    assert_eq!(rows.len(), 2, "the husk: {rows:?}");
    assert_eq!(rows[0], (first, first, "erased".to_owned(), Some(first)));
    assert_eq!((rows[1].2.as_str(), rows[1].3), ("pending", None));
    assert_eq!(
        legacy_of(&pool, cut2).await,
        vec![(
            cut2_event,
            cut2_event,
            "erased".to_owned(),
            Some(cut2_event)
        )]
    );

    let decls = declarations(&pool).await;
    assert_eq!(
        decls.len(),
        1,
        "exactly one legacy erasure_declared: {decls:?}"
    );
    let (_, system) = system_actor(&pool).await;
    assert_eq!(
        decls[0].0, rows[1].1,
        "it declared the husk's pending erasure"
    );
    assert_eq!(
        decls[0].1,
        system.uuid(),
        "under the canonical system actor"
    );
    assert_eq!(decls[0].2["legacy"], serde_json::json!(true));
    assert_eq!(decls[0].2["subject_id"], serde_json::json!(husk));

    pool.close().await;
    // FORCE: a pool's close() returns before Postgres has reaped its backends.
    sqlx::query(&format!(r#"DROP DATABASE "{name}" WITH (FORCE)"#))
        .execute(&own)
        .await
        .unwrap();
}
