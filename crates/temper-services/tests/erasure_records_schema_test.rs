#![cfg(feature = "test-db")]
//! The erasure record's schema (migration 20261201000010): one probe per CHECK and per foreign key
//! the record tables carry, the status moves `kb_erasure_members_status_moves` admits and refuses,
//! the five fence views, the D1.1 map checksum, `kb_event_types.write_retired_at` and the
//! `WRITE_RETIRED` list held equal to it, and the allowlist's re-issued COMMENT.
//!
//! Every row is written by hand, so a schema probe does not depend on the erasure projectors. A
//! refused member row is written as a projector writes one, inserted pending and then moved to
//! refused ([`write_member`]), since the status trigger refuses a row born refused. Each probe row is a valid base row with exactly one
//! field changed, so the constraint it names is the only one it can violate.
//!
//! The catalog fences that the new columns meet run unchanged: `sensitivity_scan_surface_test.rs`
//! (the text-column partition and its count), `personal_data_surface_test.rs` (the derived
//! candidates), `resource_erasure_surface_test.rs` (the discriminator CHECKs and the resource
//! reach), `erasure_policy_catalog_test.rs` and `sensitivity_store_test.rs`.

use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::collections::BTreeSet;
use uuid::Uuid;

const CHECK_VIOLATION: &str = "23514";
const FOREIGN_KEY_VIOLATION: &str = "23503";
const UNIQUE_VIOLATION: &str = "23505";
const A_DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const AT: &str = "2026-11-19T12:00:00Z";

fn code_of(e: &sqlx::Error) -> Option<String> {
    e.as_database_error()
        .and_then(|db| db.code())
        .map(|c| c.into_owned())
}

fn constraint_of(e: &sqlx::Error) -> Option<String> {
    e.as_database_error()
        .and_then(|db| db.constraint())
        .map(str::to_string)
}

fn assert_refused(result: Result<(), sqlx::Error>, code: &str, constraint: &str, what: &str) {
    let Err(e) = result else {
        panic!("{what}: the write must be refused");
    };
    assert_eq!(code_of(&e).as_deref(), Some(code), "{what}: {e}");
    assert_eq!(
        constraint_of(&e).as_deref(),
        Some(constraint),
        "{what}: {e}"
    );
}

/// A refusal raised by `kb_erasure_members_status_moves`: a check_violation naming no constraint,
/// whose message contains `says`.
fn assert_trigger_refused(result: Result<(), sqlx::Error>, says: &str, what: &str) {
    let Err(e) = result else {
        panic!("{what}: the write must be refused");
    };
    assert_eq!(code_of(&e).as_deref(), Some(CHECK_VIOLATION), "{what}: {e}");
    assert_eq!(constraint_of(&e), None, "{what}: {e}");
    let message = e
        .as_database_error()
        .map(|db| db.message().to_string())
        .unwrap_or_default();
    assert!(
        message.contains(says),
        "{what}: expected `{says}` in `{message}`"
    );
}

/// `base` with `changes` applied over it.
fn with(base: &Value, changes: Value) -> Value {
    let mut row: Map<String, Value> = base.as_object().expect("a base row is an object").clone();
    for (k, v) in changes.as_object().expect("changes are an object") {
        assert!(row.contains_key(k), "{k} is not a column of the base row");
        row.insert(k.clone(), v.clone());
    }
    Value::Object(row)
}

/// Inserts `row`, which names every column of `table`.
async fn insert(pool: &PgPool, table: &str, row: &Value) -> Result<(), sqlx::Error> {
    sqlx::query(&format!(
        "INSERT INTO {table} SELECT * FROM jsonb_populate_record(NULL::{table}, $1)"
    ))
    .bind(row)
    .execute(pool)
    .await
    .map(|_| ())
}

async fn system_profile(pool: &PgPool) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_profiles WHERE handle = 'system'")
        .fetch_one(pool)
        .await
        .expect("the canonical system profile is seeded")
}

/// Appends one event of a probe kind registered here, under the canonical system actor.
async fn event(pool: &PgPool) -> Uuid {
    sqlx::query(
        "INSERT INTO kb_event_types (name, category) VALUES ('erasure_schema_probe', 'admin') \
         ON CONFLICT (name) DO NOTHING",
    )
    .execute(pool)
    .await
    .expect("register the probe kind");
    sqlx::query_scalar(
        "SELECT _event_append('erasure_schema_probe', \
                (SELECT e.id FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id \
                  WHERE p.handle = 'system' AND e.name = 'system'), \
                NULL, NULL, '{}'::jsonb)",
    )
    .fetch_one(pool)
    .await
    .expect("append a probe event")
}

/// A valid non-legacy resource erasure.
fn erasure_row(declared_by: Uuid) -> Value {
    json!({
        "id": Uuid::now_v7(),
        "kind": "resource",
        "subject_table": "kb_resources",
        "subject_id": Uuid::now_v7(),
        "request_ref": Uuid::now_v7(),
        "manifest_version": "0.0.0-test",
        "manifest_digest": A_DIGEST,
        "legacy": false,
        "declared_event_id": declared_by,
        "scope_sealed_event_id": null,
        "completed_event_id": null,
    })
}

fn legacy_changes() -> Value {
    json!({"legacy": true, "request_ref": null, "manifest_version": null, "manifest_digest": null})
}

/// Inserts a valid erasure from `changes` over [`erasure_row`] and returns its id.
async fn erasure(pool: &PgPool, declared_by: Uuid, changes: Value) -> Uuid {
    let row = with(&erasure_row(declared_by), changes);
    insert(pool, "kb_erasures", &row)
        .await
        .expect("insert a valid erasure");
    serde_json::from_value(row["id"].clone()).unwrap()
}

/// A valid pending leaf member of `erasure`.
fn member_row(erasure: Uuid) -> Value {
    json!({
        "erasure_id": erasure,
        "member_table": "kb_resources",
        "member_id": Uuid::now_v7(),
        "parent_table": null,
        "parent_id": null,
        "status": "pending",
        "prior_erasure_id": null,
        "late": false,
        "refusal_reason": null,
        "refusal_sqlstate": null,
        "refusal_routine": null,
        "first_refused_at": null,
        "erased_by_event_id": null,
        "attempts": 0,
        "next_attempt_at": null,
        "lease_expires_at": null,
    })
}

fn refused_changes() -> Value {
    json!({"status": "refused", "refusal_reason": "unclassified", "refusal_sqlstate": "P0001",
           "refusal_routine": "resource_erasure_execute", "first_refused_at": AT})
}

/// Writes the member `row`, which names every column of `kb_erasure_members`. A row whose status
/// is `refused` cannot be inserted so (`kb_erasure_members_status_moves`): it is inserted pending,
/// with no refusal detail, and then moved to refused by an UPDATE that sets the status, the
/// refusal's reason, SQLSTATE and routine and first_refused_at from `row`, as the projector moves
/// one. The result is the UPDATE's, so a constraint the refusal breaks is reported by it.
async fn write_member(pool: &PgPool, row: &Value) -> Result<(), sqlx::Error> {
    if row["status"] != "refused" {
        return insert(pool, "kb_erasure_members", row).await;
    }
    let pending = with(
        row,
        json!({"status": "pending", "refusal_reason": null, "refusal_sqlstate": null,
               "refusal_routine": null, "first_refused_at": null}),
    );
    insert(pool, "kb_erasure_members", &pending)
        .await
        .expect("precondition: the row, pending, is valid");
    sqlx::query(
        "UPDATE kb_erasure_members m \
            SET status = r.status, refusal_reason = r.refusal_reason, \
                refusal_sqlstate = r.refusal_sqlstate, refusal_routine = r.refusal_routine, \
                first_refused_at = r.first_refused_at \
           FROM jsonb_populate_record(NULL::kb_erasure_members, $1) r \
          WHERE m.erasure_id = r.erasure_id AND m.member_table = r.member_table \
            AND m.member_id = r.member_id",
    )
    .bind(row)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Writes a valid member from `changes` over [`member_row`] and returns its id.
async fn member(pool: &PgPool, erasure: Uuid, changes: Value) -> Uuid {
    let row = with(&member_row(erasure), changes);
    write_member(pool, &row)
        .await
        .expect("write a valid member");
    serde_json::from_value(row["member_id"].clone()).unwrap()
}

/// Runs `set` as an UPDATE of one member row.
async fn update_member(pool: &PgPool, member: Uuid, set: &str) -> Result<(), sqlx::Error> {
    sqlx::query(&format!(
        "UPDATE kb_erasure_members SET {set} WHERE member_id = $1"
    ))
    .bind(member)
    .execute(pool)
    .await
    .map(|_| ())
}

/// (status, refusal_reason, refusal_sqlstate, refusal_routine, first_refused_at as text, attempts)
type MemberState = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    i32,
);

async fn member_state(pool: &PgPool, member: Uuid) -> MemberState {
    sqlx::query_as(
        "SELECT status, refusal_reason, refusal_sqlstate, refusal_routine, \
                first_refused_at::text, attempts \
           FROM kb_erasure_members WHERE member_id = $1",
    )
    .bind(member)
    .fetch_one(pool)
    .await
    .expect("read the member")
}

// ── the declared act ───────────────────────────────────────────────────────────────────────────

/// FAILS IF: a `kb_erasures` CHECK admits the one violation it exists to refuse. D1's constraints,
/// plus the single-column discriminator CHECK the resource-erasure fence requires, the subject
/// pair, and the version-iff-not-legacy CHECK beside the digest's.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_erasure_check_refuses_its_one_violation(pool: PgPool) {
    let ev = event(&pool).await;
    let base = erasure_row(ev);
    insert(&pool, "kb_erasures", &base)
        .await
        .expect("precondition: the base row is valid");
    let legacy = with(&erasure_row(ev), legacy_changes());
    insert(&pool, "kb_erasures", &legacy)
        .await
        .expect("precondition: a legacy resource erasure is valid");

    let fresh = || with(&erasure_row(ev), json!({}));
    for (what, changes, constraint) in [
        (
            "an unknown kind",
            json!({"kind": "person"}),
            "kb_erasures_kind_check",
        ),
        (
            "a subject table outside the list",
            json!({"subject_table": "kb_contexts"}),
            "kb_erasures_subject_table_check",
        ),
        (
            "an estate whose subject is a resource",
            json!({"kind": "estate"}),
            "kb_erasures_estate_subject_is_a_profile",
        ),
        (
            "a resource erasure whose subject is a profile",
            json!({"subject_table": "kb_profiles"}),
            "kb_erasures_estate_subject_is_a_profile",
        ),
        (
            "a selection with a subject",
            json!({"kind": "selection"}),
            "kb_erasures_selection_has_no_subject",
        ),
        (
            "a resource erasure with no subject",
            json!({"subject_table": null, "subject_id": null}),
            "kb_erasures_selection_has_no_subject",
        ),
        (
            "a selection naming a subject table without an id",
            json!({"kind": "selection", "subject_id": null}),
            "kb_erasures_subject_pair",
        ),
        (
            "a declared erasure with no manifest digest",
            json!({"manifest_digest": null}),
            "kb_erasures_legacy_has_no_manifest",
        ),
        (
            "a declared erasure with no manifest version",
            json!({"manifest_version": null}),
            "kb_erasures_legacy_has_no_version",
        ),
        (
            "a declared erasure with no request_ref",
            json!({"request_ref": null}),
            "kb_erasures_legacy_has_no_request",
        ),
        (
            "a malformed manifest digest",
            json!({"manifest_digest": "A".repeat(64)}),
            "kb_erasures_manifest_digest_check",
        ),
        (
            "a manifest version that is not a semver",
            json!({"manifest_version": "the march release"}),
            "kb_erasures_manifest_version_check",
        ),
    ] {
        assert_refused(
            insert(&pool, "kb_erasures", &with(&fresh(), changes)).await,
            CHECK_VIOLATION,
            constraint,
            what,
        );
    }

    let legacy_fresh = || with(&erasure_row(ev), legacy_changes());
    for (what, changes, constraint) in [
        (
            "a legacy erasure with a digest",
            json!({"manifest_digest": A_DIGEST}),
            "kb_erasures_legacy_has_no_manifest",
        ),
        (
            "a legacy erasure with a version",
            json!({"manifest_version": "0.0.0-test"}),
            "kb_erasures_legacy_has_no_version",
        ),
        (
            "a legacy erasure with a request_ref",
            json!({"request_ref": Uuid::now_v7()}),
            "kb_erasures_legacy_has_no_request",
        ),
        (
            "a legacy block erasure",
            json!({"kind": "block"}),
            "kb_erasures_legacy_is_a_resource",
        ),
    ] {
        assert_refused(
            insert(&pool, "kb_erasures", &with(&legacy_fresh(), changes)).await,
            CHECK_VIOLATION,
            constraint,
            what,
        );
    }

    assert_refused(
        insert(
            &pool,
            "kb_erasures",
            &with(&fresh(), json!({"declared_event_id": Uuid::now_v7()})),
        )
        .await,
        FOREIGN_KEY_VIOLATION,
        "kb_erasures_declared_event_id_fkey",
        "a declaring event that is not in the ledger",
    );
}

/// FAILS IF: two erasures of one kind share a request_ref, or one profile has two estate
/// erasures.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_erasure_uniques_refuse_a_second_row(pool: PgPool) {
    let ev = event(&pool).await;
    let first = erasure_row(ev);
    insert(&pool, "kb_erasures", &first).await.unwrap();
    assert_refused(
        insert(
            &pool,
            "kb_erasures",
            &with(
                &erasure_row(ev),
                json!({"request_ref": first["request_ref"]}),
            ),
        )
        .await,
        UNIQUE_VIOLATION,
        "kb_erasures_request_ref_key",
        "a second resource erasure under one request_ref",
    );
    insert(
        &pool,
        "kb_erasures",
        &with(
            &erasure_row(ev),
            json!({"kind": "block", "request_ref": first["request_ref"]}),
        ),
    )
    .await
    .expect("a request_ref is unique per kind, not across kinds");

    let subject = system_profile(&pool).await;
    let estate = json!({"kind": "estate", "subject_table": "kb_profiles", "subject_id": subject});
    insert(
        &pool,
        "kb_erasures",
        &with(&erasure_row(ev), estate.clone()),
    )
    .await
    .expect("a profile's first estate erasure");
    assert_refused(
        insert(&pool, "kb_erasures", &with(&erasure_row(ev), estate)).await,
        UNIQUE_VIOLATION,
        "kb_erasures_one_estate_per_subject",
        "a second estate erasure of one profile",
    );
}

// ── the map ────────────────────────────────────────────────────────────────────────────────────

/// FAILS IF: a `kb_erasure_members` CHECK admits the one violation it exists to refuse. A
/// biconditional is probed once per conjunct.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_member_check_refuses_its_one_violation(pool: PgPool) {
    let ev = event(&pool).await;
    let e = erasure(&pool, ev, json!({})).await;
    let prior = erasure(&pool, ev, json!({})).await;
    member(&pool, e, json!({})).await;
    member(&pool, e, refused_changes()).await;
    member(
        &pool,
        e,
        json!({"status": "erased", "erased_by_event_id": ev, "prior_erasure_id": prior}),
    )
    .await;

    let erased = json!({"status": "erased", "erased_by_event_id": ev});
    let cases: Vec<(&str, Value, Value, &str)> = vec![
        (
            "a member table outside the per-item list",
            json!({}),
            json!({"member_table": "kb_teams"}),
            "kb_erasure_members_member_table_check",
        ),
        (
            "a parent that is not a container",
            json!({}),
            json!({"parent_table": "kb_resources", "parent_id": Uuid::now_v7()}),
            "kb_erasure_members_parent_table_check",
        ),
        (
            "a parent table without a parent id",
            json!({}),
            json!({"parent_table": "kb_contexts"}),
            "kb_erasure_members_parent_pair",
        ),
        (
            "a parent id without a parent table",
            json!({}),
            json!({"parent_id": Uuid::now_v7()}),
            "kb_erasure_members_parent_pair",
        ),
        (
            "an unknown status",
            json!({}),
            json!({"status": "skipped"}),
            "kb_erasure_members_status_check",
        ),
        (
            "a refusal with no reason",
            refused_changes(),
            json!({"refusal_reason": null}),
            "kb_erasure_members_refused_has_reason",
        ),
        (
            "a refusal with no first_refused_at",
            refused_changes(),
            json!({"first_refused_at": null}),
            "kb_erasure_members_refused_has_reason",
        ),
        (
            "a reason on a pending row",
            json!({}),
            json!({"refusal_reason": "defect"}),
            "kb_erasure_members_refusal_detail_only_when_refused",
        ),
        (
            "a SQLSTATE on a pending row",
            json!({}),
            json!({"refusal_sqlstate": "P0001"}),
            "kb_erasure_members_refusal_detail_only_when_refused",
        ),
        (
            "a routine on a pending row",
            json!({}),
            json!({"refusal_routine": "resource_erasure_execute"}),
            "kb_erasure_members_refusal_detail_only_when_refused",
        ),
        (
            "a malformed SQLSTATE",
            refused_changes(),
            json!({"refusal_sqlstate": "oops"}),
            "kb_erasure_members_refusal_sqlstate_check",
        ),
        (
            "a refusal routine that is a message, not a routine name",
            refused_changes(),
            json!({"refusal_routine": "ERROR: could not erase the resource"}),
            "kb_erasure_members_refusal_routine_check",
        ),
        (
            "a refusal routine in capitals",
            refused_changes(),
            json!({"refusal_routine": "Resource_Erasure_Execute"}),
            "kb_erasure_members_refusal_routine_check",
        ),
        (
            "a pending row with a first refusal",
            json!({}),
            json!({"first_refused_at": AT}),
            "kb_erasure_members_pending_never_refused",
        ),
        (
            "an erased row with no authorising event",
            erased.clone(),
            json!({"erased_by_event_id": null}),
            "kb_erasure_members_erased_has_event",
        ),
        (
            "an authorising event on a pending row",
            json!({}),
            json!({"erased_by_event_id": ev}),
            "kb_erasure_members_erased_has_event",
        ),
        (
            "a prior erasure on a pending row",
            json!({}),
            json!({"prior_erasure_id": prior}),
            "kb_erasure_members_prior_only_when_erased",
        ),
        (
            "a row whose prior erasure is its own erasure",
            erased.clone(),
            json!({"prior_erasure_id": e}),
            "kb_erasure_members_prior_is_another",
        ),
        (
            "a late row with a prior erasure",
            erased.clone(),
            json!({"prior_erasure_id": prior, "late": true}),
            "kb_erasure_members_late_has_no_prior",
        ),
        (
            "an erased row still leased",
            erased.clone(),
            json!({"lease_expires_at": AT}),
            "kb_erasure_members_erased_is_off_the_drain",
        ),
        (
            "an erased row still scheduled",
            erased.clone(),
            json!({"next_attempt_at": AT}),
            "kb_erasure_members_erased_is_off_the_drain",
        ),
        (
            "negative attempts",
            json!({}),
            json!({"attempts": -1}),
            "kb_erasure_members_attempts_check",
        ),
    ];
    for (what, shape, violation, constraint) in cases {
        let valid = with(&member_row(e), shape);
        let row = with(&valid, violation);
        assert_refused(
            write_member(&pool, &row).await,
            CHECK_VIOLATION,
            constraint,
            what,
        );
    }
}

/// FAILS IF: a member names a parent that is not a container member of the same erasure, a
/// reason outside the vocabulary, an event outside the ledger, or an erasure that does not exist.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_member_references_its_parent_its_reason_and_its_events(pool: PgPool) {
    let ev = event(&pool).await;
    let e = erasure(&pool, ev, json!({})).await;
    let other = erasure(&pool, ev, json!({})).await;
    let context = member(&pool, e, json!({"member_table": "kb_contexts"})).await;
    member(
        &pool,
        e,
        json!({"parent_table": "kb_contexts", "parent_id": context}),
    )
    .await;

    for (what, erasure_id, changes, constraint) in [
        (
            "a parent that is no member",
            e,
            json!({"parent_table": "kb_contexts", "parent_id": Uuid::now_v7()}),
            "kb_erasure_members_parent_fkey",
        ),
        (
            "a parent that is another erasure's member",
            other,
            json!({"parent_table": "kb_contexts", "parent_id": context}),
            "kb_erasure_members_parent_fkey",
        ),
        (
            "a parent named under the wrong table",
            e,
            json!({"parent_table": "kb_cogmaps", "parent_id": context}),
            "kb_erasure_members_parent_fkey",
        ),
        (
            "a reason outside the vocabulary",
            e,
            with(
                &refused_changes(),
                json!({"refusal_reason": "already_erased"}),
            ),
            "kb_erasure_members_refusal_reason_fkey",
        ),
        (
            "an authorising event outside the ledger",
            e,
            json!({"status": "erased", "erased_by_event_id": Uuid::now_v7()}),
            "kb_erasure_members_erased_by_event_id_fkey",
        ),
        (
            "a prior erasure that does not exist",
            e,
            json!({"status": "erased", "erased_by_event_id": ev, "prior_erasure_id": Uuid::now_v7()}),
            "kb_erasure_members_prior_erasure_id_fkey",
        ),
        (
            "an erasure that does not exist",
            Uuid::now_v7(),
            json!({}),
            "kb_erasure_members_erasure_id_fkey",
        ),
    ] {
        let row = with(&member_row(erasure_id), changes);
        assert_refused(
            write_member(&pool, &row).await,
            FOREIGN_KEY_VIOLATION,
            constraint,
            what,
        );
    }
}

// ── the status moves ───────────────────────────────────────────────────────────────────────────

/// FAILS IF: the trigger refuses a legal move: a pending row's drain columns moving, pending to
/// erased, pending to refused, refused to refused with the same reason (a retry that moves only
/// the drain columns) or a changed one, each keeping first_refused_at.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_legal_status_moves_are_admitted(pool: PgPool) {
    let ev = event(&pool).await;
    let e = erasure(&pool, ev, json!({})).await;

    let m = member(&pool, e, json!({})).await;
    update_member(
        &pool,
        m,
        "attempts = 1, next_attempt_at = now(), lease_expires_at = now()",
    )
    .await
    .expect("pending -> pending moves only the drain columns");
    update_member(
        &pool,
        m,
        &format!(
            "status = 'erased', erased_by_event_id = '{ev}', next_attempt_at = NULL, \
             lease_expires_at = NULL"
        ),
    )
    .await
    .expect("pending -> erased");

    let m = member(&pool, e, json!({})).await;
    update_member(
        &pool,
        m,
        &format!(
            "status = 'refused', refusal_reason = 'defect', refusal_sqlstate = 'P0001', \
             first_refused_at = '{AT}'"
        ),
    )
    .await
    .expect("pending -> refused");
    let first = member_state(&pool, m).await.4;
    update_member(&pool, m, "attempts = attempts + 1, next_attempt_at = now()")
        .await
        .expect("refused -> refused, the same reason: a retry moving only the drain columns");
    update_member(
        &pool,
        m,
        "refusal_reason = 'unclassified', refusal_sqlstate = '23514'",
    )
    .await
    .expect("refused -> refused, a changed reason");
    let state = member_state(&pool, m).await;
    assert_eq!(state.0, "refused");
    assert_eq!(state.1.as_deref(), Some("unclassified"));
    assert_eq!(state.4, first, "first_refused_at is kept across retries");
}

/// FAILS IF: refused -> erased keeps the refusal's reason, SQLSTATE or routine, or loses
/// first_refused_at. The projector writes only the status and the authorising event.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn refused_to_erased_clears_the_reason_and_keeps_first_refused_at(pool: PgPool) {
    let ev = event(&pool).await;
    let e = erasure(&pool, ev, json!({})).await;
    let m = member(&pool, e, refused_changes()).await;
    let before = member_state(&pool, m).await;
    assert!(before.4.is_some(), "precondition: the row was refused");

    update_member(
        &pool,
        m,
        &format!("status = 'erased', erased_by_event_id = '{ev}'"),
    )
    .await
    .expect("refused -> erased");

    let after = member_state(&pool, m).await;
    assert_eq!(after.0, "erased");
    assert_eq!(
        (after.1, after.2, after.3),
        (None, None, None),
        "the refusal's reason, SQLSTATE and routine are cleared"
    );
    assert_eq!(after.4, before.4, "first_refused_at is kept");
}

/// FAILS IF: the trigger admits a status move outside D1's four: anything out of erased (an
/// erased -> erased re-projection included), or refused back to pending.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_other_status_move_is_refused(pool: PgPool) {
    let ev = event(&pool).await;
    let e = erasure(&pool, ev, json!({})).await;
    let erased = member(
        &pool,
        e,
        json!({"status": "erased", "erased_by_event_id": ev}),
    )
    .await;
    let refused = member(&pool, e, refused_changes()).await;

    for (what, m, set, says) in [
        (
            "erased -> pending",
            erased,
            "status = 'pending', erased_by_event_id = NULL".to_string(),
            "erased -> pending",
        ),
        (
            "erased -> refused",
            erased,
            format!(
                "status = 'refused', erased_by_event_id = NULL, refusal_reason = 'defect', \
                 first_refused_at = '{AT}'"
            ),
            "erased -> refused",
        ),
        (
            "erased -> erased with nothing changed",
            erased,
            "status = status".to_string(),
            "erased -> erased",
        ),
        (
            "erased -> erased under another event",
            erased,
            format!("erased_by_event_id = '{}'", event(&pool).await),
            "erased -> erased",
        ),
        (
            "refused -> pending",
            refused,
            "status = 'pending', refusal_reason = NULL, refusal_sqlstate = NULL, \
             refusal_routine = NULL"
                .to_string(),
            "refused -> pending",
        ),
    ] {
        assert_trigger_refused(update_member(&pool, m, &set).await, says, what);
    }
}

/// FAILS IF: first_refused_at can be set other than by a move into refused: on a row born erased,
/// or by a move from pending straight to erased. The controls: a row born erased without it, a
/// move into refused that sets it, and a refused row's move to erased that keeps it, are admitted.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_refusal_time_is_set_only_by_a_move_into_refused(pool: PgPool) {
    let ev = event(&pool).await;
    let e = erasure(&pool, ev, json!({})).await;
    let says = "first_refused_at is set only by a move into refused";

    let born = with(
        &member_row(e),
        json!({"status": "erased", "erased_by_event_id": ev, "first_refused_at": AT}),
    );
    assert_trigger_refused(
        insert(&pool, "kb_erasure_members", &born).await,
        says,
        "a row born erased with a refusal time",
    );
    member(
        &pool,
        e,
        json!({"status": "erased", "erased_by_event_id": ev}),
    )
    .await;

    let m = member(&pool, e, json!({})).await;
    assert_trigger_refused(
        update_member(
            &pool,
            m,
            &format!("status = 'erased', erased_by_event_id = '{ev}', first_refused_at = '{AT}'"),
        )
        .await,
        says,
        "pending -> erased setting a refusal time",
    );
    update_member(
        &pool,
        m,
        &format!(
            "status = 'refused', refusal_reason = 'defect', first_refused_at = '{AT}', \
             refusal_routine = 'public._resource_erasure_payload_redaction'"
        ),
    )
    .await
    .expect("control: pending -> refused sets the refusal time and a qualified routine name");
    update_member(
        &pool,
        m,
        &format!("status = 'erased', erased_by_event_id = '{ev}'"),
    )
    .await
    .expect("control: refused -> erased keeps the refusal time");
    assert!(
        member_state(&pool, m).await.4.is_some(),
        "the refusal time survives the move to erased"
    );
}

/// FAILS IF: a member row can be inserted refused. A row is born pending or erased, and reaches
/// refused only by an UPDATE.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_member_row_is_never_born_refused(pool: PgPool) {
    let ev = event(&pool).await;
    let e = erasure(&pool, ev, json!({})).await;
    let row = with(&member_row(e), refused_changes());
    assert_trigger_refused(
        insert(&pool, "kb_erasure_members", &row).await,
        "born pending or erased, never refused",
        "an INSERT whose status is refused",
    );
    write_member(&pool, &row)
        .await
        .expect("control: the same row, inserted pending and then refused, is admitted");
    assert_eq!(
        member_state(
            &pool,
            serde_json::from_value(row["member_id"].clone()).unwrap()
        )
        .await
        .0,
        "refused"
    );
}

/// FAILS IF: a member row can be deleted, or an UPDATE can change its identity, parent,
/// prior_erasure_id or late, or a set first_refused_at.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_member_row_is_never_deleted_and_its_identity_never_changes(pool: PgPool) {
    let ev = event(&pool).await;
    let e = erasure(&pool, ev, json!({})).await;
    let other = erasure(&pool, ev, json!({})).await;
    let pending = member(&pool, e, json!({})).await;
    let refused = member(&pool, e, refused_changes()).await;

    assert_trigger_refused(
        sqlx::query("DELETE FROM kb_erasure_members WHERE member_id = $1")
            .bind(pending)
            .execute(&pool)
            .await
            .map(|_| ()),
        "never deleted",
        "DELETE",
    );

    let fixed = "identity, parent, prior_erasure_id and late are fixed at insert";
    for (what, set) in [
        ("erasure_id", format!("erasure_id = '{other}'")),
        ("member_table", "member_table = 'kb_blobs'".to_string()),
        ("member_id", format!("member_id = '{}'", Uuid::now_v7())),
        (
            "the parent",
            format!(
                "parent_table = 'kb_contexts', parent_id = '{}'",
                Uuid::now_v7()
            ),
        ),
        ("prior_erasure_id", format!("prior_erasure_id = '{other}'")),
        ("late", "late = true".to_string()),
    ] {
        assert_trigger_refused(update_member(&pool, pending, &set).await, fixed, what);
    }

    for (what, set) in [
        (
            "first_refused_at moved",
            "first_refused_at = first_refused_at + interval '1 second'",
        ),
        ("first_refused_at cleared", "first_refused_at = NULL"),
    ] {
        assert_trigger_refused(
            update_member(&pool, refused, set).await,
            "first_refused_at is never changed once set",
            what,
        );
    }
}

/// FAILS IF: the status trigger stands down under `temper.replaying`. Replay's projectors meet the
/// same moves as the live ones.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_status_trigger_holds_under_replay(pool: PgPool) {
    let ev = event(&pool).await;
    let e = erasure(&pool, ev, json!({})).await;
    let erased = member(
        &pool,
        e,
        json!({"status": "erased", "erased_by_event_id": ev}),
    )
    .await;

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SELECT set_config('temper.replaying', 'on', true)")
        .execute(&mut *tx)
        .await
        .unwrap();
    temper_substrate::test_schema::mark_replay_target(&mut *tx).await;
    let moved = sqlx::query(
        "UPDATE kb_erasure_members SET status = 'pending', erased_by_event_id = NULL \
          WHERE member_id = $1",
    )
    .bind(erased)
    .execute(&mut *tx)
    .await
    .map(|_| ());
    assert_trigger_refused(moved, "erased -> pending", "erased -> pending under replay");
}

// ── the snapshot, the holders and the vocabulary ───────────────────────────────────────────────

/// FAILS IF: a snapshot belongs to anything but an estate erasure, or carries a negative count or
/// a malformed checksum.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_estate_snapshot_belongs_to_an_estate(pool: PgPool) {
    let ev = event(&pool).await;
    let subject = system_profile(&pool).await;
    let estate = erasure(
        &pool,
        ev,
        json!({"kind": "estate", "subject_table": "kb_profiles", "subject_id": subject}),
    )
    .await;
    let resource = erasure(&pool, ev, json!({})).await;
    let row = |erasure: Uuid| {
        json!({
            "erasure_id": erasure, "kind": "estate",
            "resource_count": 1, "context_count": 1, "cogmap_count": 0, "blob_count": 0,
            "edge_count": 0, "checksum": A_DIGEST,
        })
    };
    insert(&pool, "kb_erasure_estate_snapshots", &row(estate))
        .await
        .expect("precondition: an estate's snapshot is valid");

    assert_refused(
        insert(&pool, "kb_erasure_estate_snapshots", &row(resource)).await,
        FOREIGN_KEY_VIOLATION,
        "kb_erasure_estate_snapshots_erasure_fkey",
        "a snapshot of a resource erasure",
    );
    for (what, changes, constraint) in [
        (
            "a snapshot of another kind",
            json!({"kind": "resource"}),
            "kb_erasure_estate_snapshots_kind_check",
        ),
        (
            "a negative resource count",
            json!({"resource_count": -1}),
            "kb_erasure_estate_snapshots_resource_count_check",
        ),
        (
            "a negative context count",
            json!({"context_count": -1}),
            "kb_erasure_estate_snapshots_context_count_check",
        ),
        (
            "a negative cogmap count",
            json!({"cogmap_count": -1}),
            "kb_erasure_estate_snapshots_cogmap_count_check",
        ),
        (
            "a negative blob count",
            json!({"blob_count": -1}),
            "kb_erasure_estate_snapshots_blob_count_check",
        ),
        (
            "a negative edge count",
            json!({"edge_count": -1}),
            "kb_erasure_estate_snapshots_edge_count_check",
        ),
        (
            "a malformed checksum",
            json!({"checksum": "not-a-digest"}),
            "kb_erasure_estate_snapshots_checksum_check",
        ),
    ] {
        let fresh = erasure(
            &pool,
            ev,
            json!({"kind": "estate", "subject_table": "kb_profiles", "subject_id": Uuid::now_v7()}),
        )
        .await;
        assert_refused(
            insert(
                &pool,
                "kb_erasure_estate_snapshots",
                &with(&row(fresh), changes),
            )
            .await,
            CHECK_VIOLATION,
            constraint,
            what,
        );
    }
}

/// FAILS IF: a former holder is not a profile. The foreign key is what puts the column in the
/// personal-data derivation.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_former_holder_is_a_profile(pool: PgPool) {
    let ev = event(&pool).await;
    let e = erasure(&pool, ev, json!({})).await;
    let holder = system_profile(&pool).await;
    insert(
        &pool,
        "kb_erasure_former_holders",
        &json!({"erasure_id": e, "profile_id": holder}),
    )
    .await
    .expect("a profile holds the erasure's item");
    assert_refused(
        insert(
            &pool,
            "kb_erasure_former_holders",
            &json!({"erasure_id": e, "profile_id": Uuid::now_v7()}),
        )
        .await,
        FOREIGN_KEY_VIOLATION,
        "kb_erasure_former_holders_profile_id_fkey",
        "a holder that is no profile",
    );
}

/// FAILS IF: the vocabulary admits a malformed code or a blank meaning.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_refusal_vocabulary_refuses_a_malformed_code_or_a_blank_meaning(pool: PgPool) {
    for (what, code, meaning, constraint) in [
        (
            "a code with a capital",
            "Defect",
            "A meaning.",
            "kb_erasure_refusal_reasons_code_check",
        ),
        (
            "a blank meaning",
            "probe_code",
            "   ",
            "kb_erasure_refusal_reasons_meaning_check",
        ),
    ] {
        assert_refused(
            sqlx::query("INSERT INTO kb_erasure_refusal_reasons (code, meaning) VALUES ($1, $2)")
                .bind(code)
                .bind(meaning)
                .execute(&pool)
                .await
                .map(|_| ()),
            CHECK_VIOLATION,
            constraint,
            what,
        );
    }
}

// ── the fence views ────────────────────────────────────────────────────────────────────────────

/// FAILS IF: a fence view drops a member row of its type because of its status, or lists a row of
/// another type.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn each_fence_view_lists_every_member_of_its_type_whatever_its_status(pool: PgPool) {
    let ev = event(&pool).await;
    let e = erasure(&pool, ev, json!({})).await;
    let statuses = [
        json!({}),
        json!({"status": "erased", "erased_by_event_id": ev}),
        refused_changes(),
    ];
    for (view, table) in [
        ("kb_erasure_fenced_resources", "kb_resources"),
        ("kb_erasure_fenced_contexts", "kb_contexts"),
        ("kb_erasure_fenced_cogmaps", "kb_cogmaps"),
        ("kb_erasure_fenced_blobs", "kb_blobs"),
        ("kb_erasure_fenced_edges", "kb_edges"),
    ] {
        let mut expected = Vec::new();
        for status in &statuses {
            let changes = with(
                &with(&member_row(e), status.clone()),
                json!({"member_table": table}),
            );
            let id: Uuid = serde_json::from_value(changes["member_id"].clone()).unwrap();
            write_member(&pool, &changes).await.unwrap();
            expected.push((id, e));
        }
        expected.sort();
        let listed: Vec<(Uuid, Uuid)> = sqlx::query_as(&format!(
            "SELECT id, erasure_id FROM {view} WHERE id = ANY($1) ORDER BY id, erasure_id"
        ))
        .bind(expected.iter().map(|(id, _)| *id).collect::<Vec<_>>())
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(listed, expected, "{view} lists every {table} member");

        let foreign: i64 = sqlx::query_scalar(&format!(
            "SELECT count(*) FROM {view} v JOIN kb_erasure_members m \
                 ON m.member_id = v.id AND m.erasure_id = v.erasure_id \
              WHERE m.member_table <> $1"
        ))
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(foreign, 0, "{view} lists only {table} members");
    }
}

// ── the map checksum ───────────────────────────────────────────────────────────────────────────

/// D1.1, computed here from its definition: the header, then one line per declared member in
/// (member_table, member_id) byte order, tab-separated and newline-terminated.
fn d1_1(erasure: Uuid, digest: &str, members: &[(&str, Uuid, &str, String, String)]) -> String {
    let mut lines: Vec<(&str, Uuid, String)> = members
        .iter()
        .map(|(table, id, parent_table, parent_id, prior)| {
            (
                *table,
                *id,
                format!("{table}\t{id}\t{parent_table}\t{parent_id}\t{prior}\n"),
            )
        })
        .collect();
    lines.sort_by(|a, b| (a.0.as_bytes(), a.1).cmp(&(b.0.as_bytes(), b.1)));
    let mut text = format!("temper-erasure-map/v1\t{erasure}\t{digest}\n");
    for (_, _, line) in lines {
        text.push_str(&line);
    }
    hex::encode(Sha256::digest(text.as_bytes()))
}

async fn checksum(pool: &PgPool, erasure: Uuid) -> Option<String> {
    sqlx::query_scalar("SELECT erasure_map_checksum($1)")
        .bind(erasure)
        .fetch_one(pool)
        .await
        .expect("compute the checksum")
}

/// FAILS IF: `erasure_map_checksum` departs from D1.1: a separator that is not the tab or newline
/// byte, an order other than (member_table, member_id), a late row counted, an absent field not
/// the empty string; or a legacy erasure given a checksum.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_map_checksum_follows_d1_1(pool: PgPool) {
    let ev = event(&pool).await;
    let subject = system_profile(&pool).await;
    let digest = "0123456789abcdef".repeat(4);
    let estate = erasure(
        &pool,
        ev,
        json!({"kind": "estate", "subject_table": "kb_profiles", "subject_id": subject,
               "manifest_digest": digest}),
    )
    .await;
    let prior = erasure(&pool, ev, json!({})).await;

    assert_eq!(
        checksum(&pool, prior).await,
        Some(d1_1(prior, A_DIGEST, &[])),
        "an empty map checksums its header alone"
    );

    let context = member(&pool, estate, json!({"member_table": "kb_contexts"})).await;
    let leaf = member(
        &pool,
        estate,
        json!({"parent_table": "kb_contexts", "parent_id": context}),
    )
    .await;
    let blob = member(
        &pool,
        estate,
        json!({"member_table": "kb_blobs", "status": "erased", "erased_by_event_id": ev,
               "prior_erasure_id": prior}),
    )
    .await;
    member(&pool, estate, json!({"late": true})).await;

    assert_eq!(
        checksum(&pool, estate).await,
        Some(d1_1(
            estate,
            &digest,
            &[
                ("kb_contexts", context, "", String::new(), String::new()),
                (
                    "kb_resources",
                    leaf,
                    "kb_contexts",
                    context.to_string(),
                    String::new(),
                ),
                ("kb_blobs", blob, "", String::new(), prior.to_string()),
            ],
        )),
        "the declared rows, late excluded, in D1.1's form"
    );

    let legacy = erasure(&pool, ev, legacy_changes()).await;
    member(
        &pool,
        legacy,
        json!({"status": "erased", "erased_by_event_id": ev}),
    )
    .await;
    assert_eq!(
        checksum(&pool, legacy).await,
        None,
        "a legacy erasure has no checksum"
    );
    assert_eq!(
        checksum(&pool, Uuid::now_v7()).await,
        None,
        "an id that names no erasure has no checksum"
    );
}

// ── write retirement and the allowlist's COMMENT ───────────────────────────────────────────────

/// FAILS IF: `kb_event_types.write_retired_at` is not a nullable timestamptz, or the migration
/// retired any kind.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn write_retired_at_is_a_nullable_timestamp_set_on_no_kind(pool: PgPool) {
    let (data_type, nullable): (String, String) = sqlx::query_as(
        "SELECT data_type::text, is_nullable::text FROM information_schema.columns \
          WHERE table_schema = 'public' AND table_name = 'kb_event_types' \
            AND column_name = 'write_retired_at'",
    )
    .fetch_one(&pool)
    .await
    .expect("kb_event_types.write_retired_at exists");
    assert_eq!(data_type, "timestamp with time zone");
    assert_eq!(nullable, "YES");
    let retired: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_event_types WHERE write_retired_at IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(retired, 0, "no kind is retired for writing yet");
}

/// The kinds whose `kb_event_types.write_retired_at` is set, by name.
async fn retired_kinds(pool: &PgPool) -> BTreeSet<String> {
    sqlx::query_scalar("SELECT name FROM kb_event_types WHERE write_retired_at IS NOT NULL")
        .fetch_all(pool)
        .await
        .expect("read the retired kinds")
        .into_iter()
        .collect()
}

/// FAILS IF: `temper_substrate::payloads::WRITE_RETIRED` and the `kb_event_types` rows whose
/// `write_retired_at` is set name different kinds, in either direction. The probe sets the column
/// on one kind the list does not name and asserts the comparison then fails, so the equality is not
/// two empty reads agreeing by accident.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn write_retired_is_the_rows_whose_column_is_set(pool: PgPool) {
    let listed: BTreeSet<String> = temper_substrate::payloads::WRITE_RETIRED
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    assert_eq!(
        retired_kinds(&pool).await,
        listed,
        "WRITE_RETIRED names exactly the kinds whose write_retired_at is set"
    );

    let probe = temper_substrate::payloads::TYPED_EVENT_NAMES
        .iter()
        .find(|name| !listed.contains(**name))
        .expect("a typed kind that is not retired");
    sqlx::query("UPDATE kb_event_types SET write_retired_at = now() WHERE name = $1")
        .bind(*probe)
        .execute(&pool)
        .await
        .expect("retire the probe kind");
    let retired = retired_kinds(&pool).await;
    assert!(
        retired.contains(*probe),
        "the probe set {probe}'s write_retired_at"
    );
    assert_ne!(
        retired, listed,
        "a kind retired in the registry but missing from WRITE_RETIRED must break the equality"
    );
}

/// FAILS IF: the allowlist's COMMENT names a path under `scripts/` (the manifest it was generated
/// from moved to `policy/erasure/`), or stops naming its present manifest.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_allowlist_comment_names_its_present_manifest(pool: PgPool) {
    let comment: Option<String> =
        sqlx::query_scalar("SELECT obj_description('_erasure_redact_paths'::regproc, 'pg_proc')")
            .fetch_one(&pool)
            .await
            .unwrap();
    let comment = comment.expect("_erasure_redact_paths carries a COMMENT");
    assert!(
        !comment.contains("scripts/"),
        "the COMMENT names a path under scripts/: {comment}"
    );
    assert!(
        comment.contains("policy/erasure/resource-erasure.toml"),
        "the COMMENT names the manifest the allowlist is held to: {comment}"
    );
}
