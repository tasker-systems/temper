#![cfg(feature = "test-db")]
//! The sensitivity sweep's guarded store, its seeded detectors and its validators.
//!
//! Under goal *"Personal data that lands in the corpus by accident is found"*, sensitivity-sweep spec
//! D1, D3-D7 and rulings Q15-Q21. Nothing scans yet; these witnesses hold the shape the scan will
//! write into, and the detectors it will run.
//!
//! Spec witnesses 1 (the column-set half), 18 (`ssn_valid()`) and 19 (the delimiter requirement),
//! plus Q19 (the surface registry covers both manifests, and the enqueue refuses what it does not
//! enable), Q20 (the cursor's tuple watermark), Q21 and Q25 (append-only dispositions about one
//! finding), Q26 (a document finding's structural path) and Q27 (a detector changes only with a
//! version, and every version is kept). Witness 12's grep gate needs no database and lives in
//! `sensitivity_schema_unreachable_test.rs`.

use std::collections::BTreeSet;

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::workflow_job::{DispatchType, Persona, SensitivityJobPayload};
use temper_services::services::workflow_job_service::enqueue_system;

const SCAN_MANIFEST: &str = include_str!("../../../scripts/sensitivity-scan-surface.txt");
const PERSONAL_DATA_MANIFEST: &str = include_str!("../../../scripts/personal-data-surface.txt");

const A_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn code_of(e: &sqlx::Error) -> Option<String> {
    match e {
        sqlx::Error::Database(db) => db.code().map(|c| c.into_owned()),
        _ => None,
    }
}

fn constraint_of(e: &sqlx::Error) -> Option<String> {
    match e {
        sqlx::Error::Database(db) => db.constraint().map(str::to_string),
        _ => None,
    }
}

fn assert_check_violation(e: &sqlx::Error, constraint: &str, what: &str) {
    assert_eq!(code_of(e).as_deref(), Some("23514"), "{what}: {e}");
    assert_eq!(constraint_of(e).as_deref(), Some(constraint), "{what}: {e}");
}

async fn matches(pool: &PgPool, detector: &str, text: &str) -> i32 {
    sqlx::query_scalar::<_, Option<i32>>("SELECT sensitivity.detector_match_count($1, $2)")
        .bind(detector)
        .bind(text)
        .fetch_one(pool)
        .await
        .unwrap()
        .unwrap_or_else(|| panic!("detector {detector} is not seeded"))
}

async fn validator(pool: &PgPool, function: &str, text: &str) -> bool {
    sqlx::query_scalar(&format!("SELECT sensitivity.{function}($1)"))
        .bind(text)
        .fetch_one(pool)
        .await
        .unwrap()
}

// ── Witness 1, the column-set half: the store cannot hold content ─────────────────────────────

/// Spec D1, plus Q26's `path`, with Q28 and Q33's `fingerprint_state` where `fingerprint` was. A later PR adding `sample_text`, an offset or a window fails here, not in
/// review.
const FINDINGS_COLUMNS: &[(&str, &str)] = &[
    ("id", "uuid"),
    ("surface", "text"),
    ("target_table", "text"),
    ("target_id", "uuid"),
    ("path", "text"),
    ("resource_id", "uuid"),
    ("content_hash", "text"),
    ("detector_id", "text"),
    ("detector_version", "integer"),
    ("category", "text"),
    ("severity", "smallint"),
    ("match_count", "integer"),
    ("fingerprint_state", "text"),
    ("first_seen", "timestamp with time zone"),
    ("last_seen", "timestamp with time zone"),
];

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_findings_columns_are_exactly_the_d1_allowlist(pool: PgPool) {
    let live: BTreeSet<(String, String)> = sqlx::query_as(
        "SELECT column_name::text, data_type::text FROM information_schema.columns \
          WHERE table_schema = 'sensitivity' AND table_name = 'findings'",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .collect();
    let allowed: BTreeSet<(String, String)> = FINDINGS_COLUMNS
        .iter()
        .map(|(c, t)| (c.to_string(), t.to_string()))
        .collect();
    assert_eq!(
        live, allowed,
        "sensitivity.findings must carry D1's columns and no others"
    );
}

/// Insert one finding built from a JSON row, so each case changes exactly one column.
async fn insert_finding(pool: &PgPool, row: &serde_json::Value) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO sensitivity.findings (surface, target_table, target_id, path, content_hash, \
           detector_id, detector_version, category, severity, match_count) \
         SELECT r->>'surface', r->>'target_table', (r->>'target_id')::uuid, r->>'path', \
                r->>'content_hash', r->>'detector_id', \
                coalesce((r->>'detector_version')::int, 1), r->>'category', 4, \
                (r->>'match_count')::int \
           FROM (SELECT $1::jsonb r) x",
    )
    .bind(row)
    .execute(pool)
    .await
    .map(|_| ())
}

fn a_finding_row(target: Uuid) -> serde_json::Value {
    json!({
        "surface": "kb_resources.title",
        "target_table": "kb_resources",
        "target_id": target,
        "content_hash": A_HASH,
        "detector_id": "us_ssn_delimited",
        "category": "national_id",
        "match_count": 1,
    })
}

/// The column names are not the whole guarantee: a `text` column can carry anything. Each text
/// column of `findings` is held to a non-prose shape, and each count to 100000, so a planted
/// value is refused by the constraint that names it. The unmodified row is admitted first, so no
/// refusal below can be about a row that could never be written.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn content_under_a_findings_column_is_refused(pool: PgPool) {
    insert_finding(&pool, &a_finding_row(Uuid::now_v7()))
        .await
        .expect("the base row is admitted");
    for (column, value, constraint) in [
        (
            "content_hash",
            json!("SSN 219-45-6789"),
            "findings_content_hash_check",
        ),
        (
            "target_table",
            json!("kb_resources Jane Doe"),
            "findings_target_table_check",
        ),
        // Fits the table-name shape, and is a dash-stripped SSN: the surface decides the table.
        (
            "target_table",
            json!("kb_078_05_1120"),
            "findings_target_table_is_the_surfaces",
        ),
        (
            "category",
            json!("national_id 219-45-6789"),
            "findings_category_check",
        ),
        (
            "surface",
            json!("kb_resources.title 219-45-6789"),
            "findings_surface_fkey",
        ),
        (
            "detector_id",
            json!("us_ssn_delimited 219-45-6789"),
            "findings_detector_id_detector_version_fkey",
        ),
        // A version the detector never had (Q27).
        (
            "detector_version",
            json!(99),
            "findings_detector_id_detector_version_fkey",
        ),
        // A text surface has no document to point into (Q26).
        ("path", json!("/title"), "findings_path_fits_shape"),
        // An SSN with its dashes stripped fits an int; a count of matches never needs to.
        (
            "match_count",
            json!(219_456_789),
            "findings_match_count_check",
        ),
    ] {
        let mut row = a_finding_row(Uuid::now_v7());
        row[column] = value;
        let err = insert_finding(&pool, &row).await.unwrap_err();
        assert_eq!(
            constraint_of(&err).as_deref(),
            Some(constraint),
            "{column}: {err}"
        );
    }
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM sensitivity.findings")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 1, "only the base row");
}

/// Q22: a finding is one value, by one detector version, in one place. The same value in two
/// places is two findings, so a decision about one never silences the other; the same value seen
/// twice in one place is one.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_finding_is_one_value_in_one_place(pool: PgPool) {
    let here = Uuid::now_v7();
    insert_finding(&pool, &a_finding_row(here)).await.unwrap();
    insert_finding(&pool, &a_finding_row(Uuid::now_v7()))
        .await
        .expect("the same value elsewhere is its own finding");
    let err = insert_finding(&pool, &a_finding_row(here))
        .await
        .unwrap_err();
    assert_eq!(code_of(&err).as_deref(), Some("23505"), "{err}");

    let mut no_place = a_finding_row(here);
    no_place["target_id"] = json!(null);
    let err = insert_finding(&pool, &no_place).await.unwrap_err();
    assert_eq!(
        code_of(&err).as_deref(),
        Some("23502"),
        "a finding names its place: {err}"
    );
}

fn a_payload_finding_row(target: Uuid, path: Option<&str>) -> serde_json::Value {
    json!({
        "surface": "kb_events.payload",
        "target_table": "kb_events",
        "target_id": target,
        "path": path,
        "content_hash": A_HASH,
        "detector_id": "us_ssn_delimited",
        "category": "national_id",
        "match_count": 1,
    })
}

/// Q26: in a document the place is the path as well as the row, so a ledger finding can be read
/// for its per-path remediability (D3). The same value at two paths of one event is two findings.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_document_finding_is_placed_by_its_path(pool: PgPool) {
    let event = Uuid::now_v7();
    insert_finding(&pool, &a_payload_finding_row(event, Some("/title")))
        .await
        .expect("a path into a jsonb surface");
    insert_finding(&pool, &a_payload_finding_row(event, Some("/origin_uri")))
        .await
        .expect("another path in the same event is another place");
    let err = insert_finding(&pool, &a_payload_finding_row(event, Some("/title")))
        .await
        .unwrap_err();
    assert_eq!(code_of(&err).as_deref(), Some("23505"), "{err}");

    let err = insert_finding(&pool, &a_payload_finding_row(Uuid::now_v7(), None))
        .await
        .unwrap_err();
    assert_check_violation(
        &err,
        "findings_path_fits_shape",
        "a document finding with no path",
    );
}

/// A path is structure, never a value: letters and underscores only, so it cannot hold a digit,
/// and an array index or a user-authored key is written as `*` or `?`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_path_holds_no_value(pool: PgPool) {
    insert_finding(
        &pool,
        &a_payload_finding_row(Uuid::now_v7(), Some("/properties/?/*")),
    )
    .await
    .expect("the base path is admitted");
    for path in [
        "/ssn_219456789",
        "/items/0/title",
        "/Jane",
        "/jane doe",
        "title",
        "/",
        "/a/b/c/d/e/f/g/h/i/j/k/l/m/n/o/p/q",
    ] {
        let err = insert_finding(&pool, &a_payload_finding_row(Uuid::now_v7(), Some(path)))
            .await
            .unwrap_err();
        assert_check_violation(&err, "findings_path_check", path);
    }
}

// ── Q19 and Q26: the surface registry covers both manifests ───────────────────────────────────

/// The `table.column` of every line in `manifest` whose second field is `class`.
fn manifest_lines(manifest: &str, class: &str) -> BTreeSet<String> {
    manifest
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let cols: Vec<&str> = l.split('|').map(str::trim).collect();
            (cols.get(1) == Some(&class)).then(|| cols[0].to_string())
        })
        .collect()
}

/// Every `scan` line is a `text` surface and every `incidental` line of the personal-data manifest
/// a `jsonb` one (Q26). A registry row neither manifest names any longer must be disabled: findings
/// and cursors pin a surface's row, so retiring a line cannot delete it (Q19, amended).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_surface_registry_covers_both_manifests(pool: PgPool) {
    let rows: Vec<(String, String, bool)> =
        sqlx::query_as("SELECT surface, shape, enabled FROM sensitivity.surfaces")
            .fetch_all(&pool)
            .await
            .unwrap();
    let declared = [
        (manifest_lines(SCAN_MANIFEST, "scan"), "text"),
        (
            manifest_lines(PERSONAL_DATA_MANIFEST, "incidental"),
            "jsonb",
        ),
    ];
    for (lines, shape) in &declared {
        assert!(
            !lines.is_empty(),
            "the manifest parse found no {shape} lines"
        );
        let registered: BTreeSet<String> = rows
            .iter()
            .filter(|(_, s, _)| s == shape)
            .map(|(surface, _, _)| surface.clone())
            .collect();
        assert_eq!(
            lines.difference(&registered).collect::<Vec<_>>(),
            Vec::<&String>::new(),
            "manifest lines with no {shape} sensitivity.surfaces row"
        );
    }
    let retired_but_enabled: Vec<&String> = rows
        .iter()
        .filter(|(surface, _, enabled)| {
            *enabled && !declared.iter().any(|(lines, _)| lines.contains(surface))
        })
        .map(|(surface, _, _)| surface)
        .collect();
    assert_eq!(
        retired_but_enabled,
        Vec::<&String>::new(),
        "enabled surfaces that neither manifest declares"
    );
}

/// Cut 1 cursors D3's first cut (plan P3), less `kb_blobs.blob_pathname`, which the manifest
/// declares structural, and `kb_workflow_jobs.last_error`, which `workflow_job_reap` rewrites on
/// old ids, plus the three documents D3 reads through the personal-data manifest (Q26). Every other
/// surface is seeded disabled with no cursor kind, which is what the run summary reads to name it
/// as not yet cursored.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn cut_one_enables_the_first_cut_surfaces_and_no_others(pool: PgPool) {
    let enabled: Vec<(String, String)> = sqlx::query_as(
        "SELECT surface, cursor_kind FROM sensitivity.surfaces WHERE enabled ORDER BY surface",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let expect = [
        ("kb_block_content.content", "append_only_v7"),
        ("kb_chunk_content.content", "append_only_v7"),
        ("kb_chunks.header_path", "append_only_v7"),
        ("kb_citation_audits.reason", "append_only_v7"),
        ("kb_edges.label", "append_only_v7"),
        ("kb_events.metadata", "append_only_v7"),
        ("kb_events.payload", "append_only_v7"),
        ("kb_properties.property_key", "append_only_v7"),
        ("kb_properties.property_value", "append_only_v7"),
        ("kb_remote_sources.uri", "append_only_v7"),
        ("kb_resources.origin_uri", "mutable_timestamp"),
        ("kb_resources.title", "mutable_timestamp"),
    ];
    let expect: Vec<(String, String)> = expect
        .iter()
        .map(|(s, k)| (s.to_string(), k.to_string()))
        .collect();
    assert_eq!(enabled, expect);

    let stray: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM sensitivity.surfaces WHERE NOT enabled AND cursor_kind IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stray, 0, "a disabled surface carries no cursor kind");

    let err = sqlx::query(
        "UPDATE sensitivity.surfaces SET enabled = true WHERE surface = 'kb_teams.description'",
    )
    .execute(&pool)
    .await
    .unwrap_err();
    assert_check_violation(
        &err,
        "surfaces_enabled_needs_cursor",
        "enable without a cursor",
    );
}

fn order(surface: &str) -> SensitivityJobPayload {
    SensitivityJobPayload {
        surface: surface.into(),
        budget: 500,
    }
}

/// Q19's enqueue half. PR A's CHECK admits any `kb_<table>.<column>`; membership is the real
/// constraint. A surface that is a scan line but not enabled, and one shaped right but naming
/// nothing, are both refused, and the refusal leaves no row.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_enqueue_refuses_a_surface_that_is_not_enabled(pool: PgPool) {
    let persona = Persona::Sensitivity.as_str();
    let dispatch = DispatchType::SensitivitySweep.as_str();
    for surface in [
        "kb_teams.description",
        "kb_jane.doe",
        "kb_workflow_jobs.last_error",
    ] {
        // The typed wrapper refuses too, but renders an opaque internal error by design: the
        // database message never reaches a log. The raw call shows which refusal fired.
        assert!(enqueue_system(&pool, persona, dispatch, &order(surface))
            .await
            .is_err());
        let err = sqlx::query("SELECT workflow_job_enqueue_system($1, $2, $3)")
            .bind(persona)
            .bind(dispatch)
            .bind(serde_json::to_value(order(surface)).unwrap())
            .execute(&pool)
            .await
            .unwrap_err();
        assert_eq!(code_of(&err).as_deref(), Some("23514"), "{surface}: {err}");
        assert!(
            err.to_string()
                .contains("sensitivity surface is not enabled"),
            "{surface}: {err}"
        );
    }
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_workflow_jobs WHERE persona = $1")
        .bind(persona)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "a refused enqueue leaves no job");

    enqueue_system(&pool, persona, dispatch, &order("kb_resources.title"))
        .await
        .unwrap()
        .expect("an enabled surface is admitted");
}

/// The membership check runs after the insert, so PR A's work-order CHECK still answers first for
/// a malformed payload rather than being shadowed by the newer refusal.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_malformed_work_order_still_meets_the_check_first(pool: PgPool) {
    let err = sqlx::query("SELECT workflow_job_enqueue_system($1, $2, $3)")
        .bind(Persona::Sensitivity.as_str())
        .bind(DispatchType::SensitivitySweep.as_str())
        .bind(json!({"surface": "jane.doe", "budget": 10}))
        .execute(&pool)
        .await
        .unwrap_err();
    assert_check_violation(
        &err,
        "ck_workflow_jobs_sensitivity_work_order",
        "malformed order",
    );
}

// ── Q18 and Q16: the seeded corpus ────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_seeded_detectors_are_cut_ones_nine(pool: PgPool) {
    let seeded: Vec<(String, String, i16, Option<String>, bool, i32)> = sqlx::query_as(
        "SELECT id, category, severity, validator, enabled, version \
           FROM sensitivity.detectors ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    let expect: Vec<(String, String, i16, Option<String>, bool, i32)> = [
        ("aba_routing", "financial", 3, Some("aba_routing_valid")),
        ("cloud_saas_key", "credential", 4, None),
        ("connection_string_password", "credential", 4, None),
        ("jwt", "credential", 3, None),
        ("local_path_username", "identifier", 1, None),
        ("payment_card", "payment_card", 4, Some("luhn_valid")),
        ("private_key_block", "secret_material", 4, None),
        ("us_ssn_contextual", "national_id", 4, Some("ssn_valid")),
        ("us_ssn_delimited", "national_id", 4, Some("ssn_valid")),
    ]
    .iter()
    .map(|(id, cat, sev, v)| {
        (
            id.to_string(),
            cat.to_string(),
            *sev,
            v.map(str::to_string),
            true,
            // payment_card v2 stops reading cards out of hex runs (Q42, 20261003150000).
            if *id == "payment_card" { 2 } else { 1 },
        )
    })
    .collect();
    assert_eq!(seeded, expect);
}

/// Q16: email is deferred, so nothing in cut 1 detects `contact` data, and an address yields
/// nothing. The run summary must say so rather than read as "no contact data found".
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn cut_one_detects_no_contact_data(pool: PgPool) {
    let contact: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sensitivity.detectors WHERE category = 'contact'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(contact, 0);
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM sensitivity.detectors")
        .fetch_all(&pool)
        .await
        .unwrap();
    for id in ids {
        assert_eq!(
            matches(&pool, &id, "write to jane.doe@example.org").await,
            0,
            "{id}"
        );
    }
}

/// Each seeded detector finds its own planted value, so every negative in this file is measured
/// against a detector that can fire.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_seeded_detector_finds_its_planted_value(pool: PgPool) {
    for (detector, text) in [
        ("private_key_block", "-----BEGIN OPENSSH PRIVATE KEY-----"),
        (
            "cloud_saas_key",
            "key AKIAABCDEFGHIJKLMNOP here", // gitleaks:allow — a planted fake key the detector must find
        ),
        (
            "connection_string_password",
            "postgres://app:hunter2@db:5432/x",
        ),
        (
            "jwt",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.abcdefghijklmnop", // gitleaks:allow — a planted unsigned sample JWT the detector must find
        ),
        ("payment_card", "card 4111 1111 1111 1111 exp"),
        ("aba_routing", "routing number: 021000021"),
        ("us_ssn_delimited", "ssn 219-45-6789"),
        ("us_ssn_contextual", "SSN: 219456789"),
        ("local_path_username", "see /Users/jdoe/notes.md"),
    ] {
        assert_eq!(
            matches(&pool, detector, text).await,
            1,
            "{detector}: {text}"
        );
    }
}

// ── Witness 18: ssn_valid() rejects what it must ──────────────────────────────────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn ssn_valid_applies_each_structural_rule_and_known_fake(pool: PgPool) {
    assert!(
        validator(&pool, "ssn_valid", "219-45-6789").await,
        "control"
    );
    for (rule, ssn) in [
        ("area 000", "000-45-6789"),
        ("area 666", "666-45-6789"),
        ("area 9xx", "900-45-6789"),
        ("area 9xx", "999-45-6789"),
        ("group 00", "219-00-6789"),
        ("serial 0000", "219-45-0000"),
        ("known fake", "078-05-1120"),
        ("known fake", "219-09-9999"),
        ("known fake", "123-45-6789"),
        ("not nine digits", "219-45-678"),
    ] {
        assert!(!validator(&pool, "ssn_valid", ssn).await, "{rule}: {ssn}");
    }
}

/// The case that decides whether an always-on detector is tolerable: both fakes appear in ordinary
/// tutorials and test data.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_tutorial_fakes_yield_no_finding(pool: PgPool) {
    let fixture = "Example: SSN 123-45-6789. The Woolworth card read 078-05-1120. \
                   ssn: 123456789 and social security 078051120.";
    for detector in ["us_ssn_delimited", "us_ssn_contextual"] {
        assert_eq!(matches(&pool, detector, fixture).await, 0, "{detector}");
    }
}

// ── Witness 19: the delimiter requirement holds ───────────────────────────────────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn bare_digits_are_not_a_delimited_ssn(pool: PgPool) {
    for text in [
        "row id 219456789 updated",
        "call +12194567890 today",
        "at 2026-10-02T21:94:56.219456789Z",
        "build 219-45-67890",
    ] {
        assert_eq!(matches(&pool, "us_ssn_delimited", text).await, 0, "{text}");
    }
    for text in ["ssn 219-45-6789", "ssn 219 45 6789"] {
        assert_eq!(matches(&pool, "us_ssn_delimited", text).await, 1, "{text}");
    }
}

/// The contextual detector recovers the pasted-bare case only next to its keyword.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn bare_nine_digits_need_the_keyword(pool: PgPool) {
    assert_eq!(
        matches(&pool, "us_ssn_contextual", "order 219456789").await,
        0
    );
    assert_eq!(
        matches(&pool, "us_ssn_contextual", "ssn 219456789").await,
        1
    );
    assert_eq!(
        matches(&pool, "us_ssn_contextual", "Social Security no. 219456789").await,
        1
    );
}

// ── The other two validators ──────────────────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn luhn_and_aba_adjudicate_what_the_patterns_nominate(pool: PgPool) {
    for (valid, card) in [
        (true, "4111 1111 1111 1111"),
        (true, "5500-0000-0000-0004"),
        (false, "4111 1111 1111 1112"),
        (false, "0000000000000000"),
        (false, "411111111111"),
    ] {
        assert_eq!(validator(&pool, "luhn_valid", card).await, valid, "{card}");
    }
    for (valid, routing) in [
        (true, "021000021"),
        (true, "011000015"),
        (false, "021000022"),
        (false, "000000000"),
        (false, "02100002"),
    ] {
        assert_eq!(
            validator(&pool, "aba_routing_valid", routing).await,
            valid,
            "{routing}"
        );
    }
    // A routing keyword inside a word ("database") is not context.
    assert_eq!(matches(&pool, "aba_routing", "database 021000021").await, 0);
}

/// The commonest pasted forms carry a card's neighbours with it. A loose grouping let the longest
/// match absorb them, Luhn rejected the whole run, and the card went unfound.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_card_beside_its_cvv_expiry_or_other_digits_is_found(pool: PgPool) {
    for text in [
        "card 4111 1111 1111 1111 123",
        "4111 1111 1111 1111 12/27",
        "item 12 4111 1111 1111 1111",
        "4111-1111-1111-1111 cvv 123",
        "4111111111111111 123",
        "amex 3782 822463 10005 exp 01/28",
    ] {
        assert_eq!(matches(&pool, "payment_card", text).await, 1, "{text}");
    }
    // Mixed separators are not a card grouping.
    assert_eq!(
        matches(&pool, "payment_card", "4111 1111-1111 1111").await,
        0
    );
}

/// A detector is operator data (D5), so a bad regex must fail when it is written, not mid-scan,
/// where the error would land in a tick's error column.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_detector_pattern_must_compile_and_never_match_empty(pool: PgPool) {
    let insert =
        "INSERT INTO sensitivity.detectors (id, category, severity, prefilter, pattern, note) \
                  VALUES ('probe', 'credential', 2, $1, $2, 'probe')";
    for (prefilter, pattern) in [("x", ".*"), ("[0-9]*", "x[0-9]+")] {
        let err = sqlx::query(insert)
            .bind(prefilter)
            .bind(pattern)
            .execute(&pool)
            .await
            .unwrap_err();
        assert_check_violation(&err, "detectors_patterns_usable", pattern);
    }
    let err = sqlx::query(insert)
        .bind("x")
        .bind("x(")
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(
        code_of(&err).as_deref(),
        Some("2201B"),
        "invalid regex: {err}"
    );
    // The scan runs the pattern wrapped in a group. An embedded option is legal only at the start
    // of a regex, so it compiles bare and fails wrapped: refused here, not mid-scan.
    let err = sqlx::query(insert)
        .bind("x")
        .bind("(?i)secret")
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(
        code_of(&err).as_deref(),
        Some("2201B"),
        "an option that only compiles bare: {err}"
    );
    // The wrapping group renumbers a backreference, so `\1` would silently mean another group.
    let err = sqlx::query(insert)
        .bind("x")
        .bind(r"(x)\1")
        .execute(&pool)
        .await
        .unwrap_err();
    assert_check_violation(&err, "detectors_patterns_usable", "a backreference");
    sqlx::query(insert)
        .bind("x")
        .bind("x[0-9]+")
        .execute(&pool)
        .await
        .expect("a usable pattern is admitted");
}

/// A pattern that matches only between characters never matches the empty string, so the write
/// gate admits it; the count is what refuses to treat a position as a value.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn an_empty_match_is_not_a_match(pool: PgPool) {
    sqlx::query(
        "INSERT INTO sensitivity.detectors (id, category, severity, prefilter, pattern, note) \
         VALUES ('probe_boundary', 'credential', 2, 'x', '\\y', 'probe')",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(
        matches(&pool, "probe_boundary", "x marks the spot").await,
        0
    );
}

/// The store caps a count at 100000. One huge row must yield a capped count, not a value that
/// fails the insert and wedges the tick on that row for good.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_match_count_stops_at_the_stores_ceiling(pool: PgPool) {
    let text = "/home/ab ".repeat(100_001);
    assert_eq!(matches(&pool, "local_path_username", &text).await, 100_000);
}

// ── Q27: a detector changes only with a version, and every version is kept ────────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_detector_changes_only_with_a_version_bump(pool: PgPool) {
    for (column, value) in [
        ("pattern", "'(?<![0-9])[0-9]{3}-[0-9]{2}-[0-9]{4}(?![0-9])'"),
        ("prefilter", "'[0-9]{3}-'"),
        ("validator", "NULL"),
        ("category", "'identifier'"),
    ] {
        let err = sqlx::query(&format!(
            "UPDATE sensitivity.detectors SET {column} = {value} WHERE id = 'us_ssn_delimited'"
        ))
        .execute(&pool)
        .await
        .unwrap_err();
        assert_eq!(code_of(&err).as_deref(), Some("23514"), "{column}: {err}");
    }
    let err = sqlx::query("UPDATE sensitivity.detectors SET version = 0 WHERE id = 'jwt'")
        .execute(&pool)
        .await
        .unwrap_err();
    assert_eq!(
        code_of(&err).as_deref(),
        Some("23514"),
        "a version goes down: {err}"
    );

    // Severity is operational policy (Q3), tunable without a version.
    sqlx::query("UPDATE sensitivity.detectors SET severity = 3 WHERE id = 'us_ssn_delimited'")
        .execute(&pool)
        .await
        .expect("a severity retune needs no bump");
    sqlx::query(
        "UPDATE sensitivity.detectors SET pattern = 'eyJ[A-Za-z0-9_-]{8,}\\.[A-Za-z0-9_-]{8,}\\.[A-Za-z0-9_-]{8,}', \
                version = 2 WHERE id = 'jwt'",
    )
    .execute(&pool)
    .await
    .expect("a change with a bump is admitted");

    let versions: Vec<(String, i32)> = sqlx::query_as(
        "SELECT detector_id, version FROM sensitivity.detector_versions \
          WHERE detector_id IN ('jwt', 'us_ssn_delimited') ORDER BY 1, 2",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        versions,
        vec![
            ("jwt".to_string(), 1),
            ("jwt".to_string(), 2),
            ("us_ssn_delimited".to_string(), 1),
        ],
        "every version is kept, and a severity retune is not one"
    );
    let v1_kept: bool = sqlx::query_scalar(
        "SELECT pattern LIKE '%{10,}%' FROM sensitivity.detector_versions \
          WHERE detector_id = 'jwt' AND version = 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        v1_kept,
        "version 1 keeps the pattern that produced its findings"
    );
    let err = sqlx::query("DELETE FROM sensitivity.detector_versions")
        .execute(&pool)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("append-only"), "{err}");
}

// ── Q20: a cursor's watermark is typed by its surface's kind ──────────────────────────────────

async fn insert_cursor(
    pool: &PgPool,
    surface: &str,
    kind: &str,
    lane: &str,
    at: Option<&str>,
    id: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO sensitivity.cursors (surface, cursor_kind, detector_id, detector_version, \
           lane, watermark_at, watermark_id) \
         VALUES ($1, $2, 'us_ssn_delimited', 1, $3, $4::timestamptz, $5)",
    )
    .bind(surface)
    .bind(kind)
    .bind(lane)
    .bind(at)
    .bind(id)
    .execute(pool)
    .await
    .map(|_| ())
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_timestamp_watermark_is_a_tuple_and_an_id_watermark_is_an_id(pool: PgPool) {
    let now = Some("2026-10-02T12:00:00Z");
    let id = Some(Uuid::now_v7());

    insert_cursor(
        &pool,
        "kb_resources.title",
        "mutable_timestamp",
        "head",
        now,
        id,
    )
    .await
    .unwrap();
    insert_cursor(&pool, "kb_edges.label", "append_only_v7", "head", None, id)
        .await
        .unwrap();
    insert_cursor(
        &pool,
        "kb_resources.title",
        "mutable_timestamp",
        "backfill",
        None,
        None,
    )
    .await
    .expect("a cursor that has not advanced yet has no watermark");

    for (surface, kind, at, wid, why) in [
        (
            "kb_resources.origin_uri",
            "mutable_timestamp",
            now,
            None,
            "timestamp without id",
        ),
        (
            "kb_resources.origin_uri",
            "mutable_timestamp",
            None,
            id,
            "id without timestamp",
        ),
        (
            "kb_chunks.header_path",
            "append_only_v7",
            now,
            id,
            "timestamp on an id cursor",
        ),
    ] {
        let err = insert_cursor(&pool, surface, kind, "head", at, wid)
            .await
            .unwrap_err();
        assert_check_violation(&err, "cursors_watermark_shape", why);
    }
}

/// The kind is the surface's, not the writer's: the composite foreign key refuses a cursor that
/// claims a kind its surface does not have, and one on a surface nobody enabled a cursor for.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_cursor_cannot_claim_a_kind_its_surface_lacks(pool: PgPool) {
    for (surface, kind) in [
        ("kb_resources.title", "append_only_v7"),
        ("kb_teams.description", "append_only_v7"),
    ] {
        let err = insert_cursor(&pool, surface, kind, "head", None, Some(Uuid::now_v7()))
            .await
            .unwrap_err();
        assert_eq!(code_of(&err).as_deref(), Some("23503"), "{surface}: {err}");
    }
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn backfill_bookkeeping_lives_on_the_backfill_lane_only(pool: PgPool) {
    let err = sqlx::query(
        "INSERT INTO sensitivity.cursors (surface, cursor_kind, detector_id, detector_version, \
           lane, backfill_completed_at) \
         VALUES ('kb_edges.label', 'append_only_v7', 'jwt', 1, 'head', now())",
    )
    .execute(&pool)
    .await
    .unwrap_err();
    assert_check_violation(
        &err,
        "cursors_backfill_lane_only",
        "completion on the head lane",
    );
}

// ── Q21 and Q25: dispositions ─────────────────────────────────────────────────────────────────────────

async fn a_finding(pool: &PgPool) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO sensitivity.findings (surface, target_table, target_id, content_hash, \
           detector_id, detector_version, category, severity, match_count) \
         VALUES ('kb_resources.title', 'kb_resources', gen_random_uuid(), $1, 'jwt', 1, \
                 'credential', 3, 1) RETURNING id",
    )
    .bind(A_HASH)
    .fetch_one(pool)
    .await
    .unwrap()
}

/// One disposition row. `Default` leaves every optional column NULL.
#[derive(Default, Clone)]
struct Disposition {
    state: &'static str,
    finding: Option<Uuid>,
    expires_in_days: Option<i32>,
}

async fn dispose(pool: &PgPool, d: Disposition) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO sensitivity.dispositions (state, finding_id, expires_at) \
         VALUES ($1, $2, now() + make_interval(days => $3))",
    )
    .bind(d.state)
    .bind(d.finding)
    .bind(d.expires_in_days)
    .execute(pool)
    .await
    .map(|_| ())
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn accepted_risk_carries_a_clock_and_nothing_else_does(pool: PgPool) {
    let f = Some(a_finding(&pool).await);
    for (d, why) in [
        (
            Disposition {
                state: "accepted_risk",
                finding: f,
                ..Default::default()
            },
            "accepted risk with no expiry",
        ),
        (
            Disposition {
                state: "acknowledged",
                finding: f,
                expires_in_days: Some(30),
            },
            "an expiry on another state",
        ),
    ] {
        let err = dispose(&pool, d).await.unwrap_err();
        assert_check_violation(&err, "dispositions_expiry_iff_accepted_risk", why);
    }
    dispose(
        &pool,
        Disposition {
            state: "accepted_risk",
            finding: f,
            expires_in_days: Some(30),
        },
    )
    .await
    .unwrap();
}

/// Q25: every state, `false_positive` included, is about one finding. The table has no column that
/// could key a ruling by value, so a test number ruled benign in one team's content cannot hide the
/// same digits where they are real. A value benign everywhere is a versioned detector change.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_disposition_names_one_finding_and_never_a_value(pool: PgPool) {
    let columns: BTreeSet<String> = sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns \
          WHERE table_schema = 'sensitivity' AND table_name = 'dispositions'",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .into_iter()
    .collect();
    let expect: BTreeSet<String> = ["id", "finding_id", "state", "expires_at", "decided_at"]
        .iter()
        .map(|c| c.to_string())
        .collect();
    assert_eq!(columns, expect);

    for state in ["acknowledged", "actioned", "false_positive"] {
        let err = dispose(
            &pool,
            Disposition {
                state,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
        assert_eq!(code_of(&err).as_deref(), Some("23502"), "{state}: {err}");
    }
}

/// The same value in two places: ruling one a false positive leaves the other with no disposition,
/// which is what "open" means (Q21).
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_false_positive_in_one_place_leaves_the_other_open(pool: PgPool) {
    let (here, there) = (a_finding(&pool).await, a_finding(&pool).await);
    dispose(
        &pool,
        Disposition {
            state: "false_positive",
            finding: Some(here),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let open: Vec<Uuid> = sqlx::query_scalar(
        "SELECT f.id FROM sensitivity.findings f \
          WHERE NOT EXISTS (SELECT 1 FROM sensitivity.dispositions d WHERE d.finding_id = f.id)",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(open, vec![there]);
}

/// Under the coverage rule (Q22) a disposition covers what was seen up to its `decided_at`, so a
/// writer's future date would silence the finding for good. The database supplies the clock, and an
/// accepted risk must expire after it was accepted.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_disposition_is_decided_now(pool: PgPool) {
    let f = a_finding(&pool).await;
    let in_the_future: bool = sqlx::query_scalar(
        "INSERT INTO sensitivity.dispositions (state, finding_id, decided_at) \
         VALUES ('actioned', $1, '2999-01-01') RETURNING decided_at > now() + interval '1 day'",
    )
    .bind(f)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!in_the_future, "decided_at is the database's now()");

    let err = dispose(
        &pool,
        Disposition {
            state: "accepted_risk",
            finding: Some(f),
            expires_in_days: Some(-1),
        },
    )
    .await
    .unwrap_err();
    assert_check_violation(
        &err,
        "dispositions_expiry_after_decision",
        "expired before decided",
    );
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn dispositions_are_append_only_in_enforcement(pool: PgPool) {
    let f = Some(a_finding(&pool).await);
    dispose(
        &pool,
        Disposition {
            state: "acknowledged",
            finding: f,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    for sql in [
        "UPDATE sensitivity.dispositions SET state = 'actioned'",
        "DELETE FROM sensitivity.dispositions",
    ] {
        let err = sqlx::query(sql).execute(&pool).await.unwrap_err();
        assert!(err.to_string().contains("append-only"), "{sql}: {err}");
    }
}

// ── D9: a run's tally is categories and counts ────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_runs_tally_holds_only_categories_and_counts(pool: PgPool) {
    let insert = "INSERT INTO sensitivity.runs (surface, by_category) \
                  VALUES ('kb_resources.title', $1)";
    sqlx::query(insert)
        .bind(json!({"national_id": 100_000, "credential": 0}))
        .execute(&pool)
        .await
        .unwrap();
    for tally in [
        json!({"219-45-6789": 1}),
        json!({"national_id": "219-45-6789"}),
        json!({"national_id": 123_456_789_012_i64}),
        json!({"national_id": 219_456_789}),
        // Six digits, but past every other count's ceiling: two keys could split an SSN.
        json!({"national_id": 219_456, "payment_card": 789}),
        json!(["national_id"]),
    ] {
        let err = sqlx::query(insert)
            .bind(&tally)
            .execute(&pool)
            .await
            .unwrap_err();
        assert_check_violation(&err, "runs_by_category_check", &tally.to_string());
    }
}
