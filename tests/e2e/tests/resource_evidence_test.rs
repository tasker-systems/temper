#![cfg(feature = "test-db")]
//! Set 3, Task 9 (renamed fields under Set 5, Task 11): `temper resource evidence <ref>` drives
//! the CLI ↔ API ↔ DB stack and returns a `StandingShape` — the evidential-standing shape vector
//! plus the lossy read-time `band` chip carried WITH it, never in place of it (spec §1.1).
//!
//! The load-bearing assertion is that the whole struct rides through: all eight fields are
//! present on the wire, `band` sits alongside the numeric components (not instead of them),
//! and `finding_id` anchors the shape to the resource we asked about. A provenance-bearing
//! create (a body plus a remote `--sources` record) exercises the reinforcement component
//! `r_parent` end-to-end rather than reading a shape that is all zeros.
//!
//! Set 5 replaced `indep_breadth` / `adversarial_survival` / `challenge_count` with the three
//! citation axes `citation_magnitude` / `audit_coverage` / `citation_quality`
//! (`crates/temper-core/src/types/standing.rs:36-45`). The second test below drives the new
//! write surface end to end: a resource-kind citation (never the `https://…` remote source used
//! above, which `resource_citation_magnitude` does not count —
//! `migrations/20260723000020_standing_citation_components.sql:100-124`) plus a citation audit
//! recorded by a distinct, non-author auditor (`temper_client::resources::record_citation_audit`,
//! Task 7/8), asserting the axes move off their pre-audit values rather than merely being numeric.

mod common;

use std::sync::Arc;

use serde_json::Value;
use temper_client::TemperClient;
use temper_core::types::citation_audit::CitationAuditRequest;
use temper_core::types::provenance::ProvenanceSource;
use temper_core::types::resource_grant::ResourceGrantBody;

/// Run the CLI and parse stdout as exactly one JSON document.
async fn cli_json(app: &common::E2eTestApp, args: &[&str]) -> Value {
    let output = common::run_temper_cli(app, args).await.expect("cli run");
    assert!(
        output.status.success(),
        "cli {args:?} failed: stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "cli {args:?} did not emit exactly one JSON document ({e}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn evidence_returns_the_full_standing_shape_with_band(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("evidence-ctx", None)
        .await
        .expect("ctx create");

    // A provenance-bearing create: `--sources` requires a body, and `--body` takes `@<path>`
    // (never a literal), so the body goes through a temp file. A remote http source stamps a
    // block-provenance record on the body block — the reinforcement that `r_parent` counts.
    let body_file = tempfile::NamedTempFile::new().expect("temp body file");
    std::fs::write(
        body_file.path(),
        "A finding distilled from an external source.\n",
    )
    .expect("write temp body");
    let body_arg = format!("@{}", body_file.path().display());

    let created = cli_json(
        &app,
        &[
            "resource",
            "create",
            "--type",
            "concept",
            "--title",
            "Evidence Probe",
            "--context",
            "@me/evidence-ctx",
            "--body",
            &body_arg,
            "--sources",
            "https://example.com/source-a",
            "--model",
            "claude-opus-4-8",
            "--confidence",
            "confident",
            "--format",
            "json",
        ],
    )
    .await;

    let id = created["id"]
        .as_str()
        .expect("create response carries an `id`")
        .to_string();

    // The command under test.
    let shape = cli_json(&app, &["resource", "evidence", &id, "--format", "json"]).await;
    let obj = shape.as_object().expect("evidence emits a JSON object");

    // All eight StandingShape fields ride through — the shape is shape-primary, and `band`
    // is carried WITH the shape, never in place of it. Set 5 retired `indep_breadth` /
    // `adversarial_survival` / `challenge_count` in favor of the three citation axes below
    // (`crates/temper-core/src/types/standing.rs:36-45`); this is an EXACT set, not a subset —
    // it must fail if a field is added or removed, so the retired names are replaced, not kept
    // alongside the new ones.
    for key in [
        "finding_id",
        "citation_magnitude",
        "audit_coverage",
        "citation_quality",
        "contradiction_balance",
        "freshness",
        "r_parent",
        "band",
    ] {
        assert!(
            obj.contains_key(key),
            "StandingShape is missing `{key}`: {shape}"
        );
    }

    // The shape anchors to the resource we asked about.
    assert_eq!(
        shape["finding_id"], id,
        "finding_id anchors the shape to the resource: {shape}"
    );

    // The band is a non-empty summary string sitting ALONGSIDE the numeric components.
    let band = shape["band"]
        .as_str()
        .unwrap_or_else(|| panic!("band is a string: {shape}"));
    assert!(!band.is_empty(), "band is non-empty: {shape}");

    // The numeric components are numbers, not strings or nulls — the vector is intact.
    // r_parent counts uncorrected provenance over the finding's live blocks
    // (`resource_r_parent`), so the provenance-bearing create above (a body block with a
    // remote `--sources` record) must read back NON-zero: this is what proves the shape
    // was computed end-to-end from real reinforcement, not returned all-zeros.
    let r_parent = shape["r_parent"]
        .as_f64()
        .unwrap_or_else(|| panic!("r_parent is numeric: {shape}"));
    assert!(
        r_parent > 0.0,
        "r_parent reflects the seeded provenance (> 0), not an all-zeros shape: {shape}"
    );
    assert!(
        shape["citation_magnitude"].is_number(),
        "citation_magnitude is numeric: {shape}"
    );
    assert!(
        shape["audit_coverage"].is_number(),
        "audit_coverage is numeric: {shape}"
    );
}

// ─── Set 5, Task 11: citation audit moves audit_coverage + citation_quality ─────────────────

/// The resource-kind citation + citation-audit write surface, driven end to end through the
/// real CLI ↔ API ↔ DB stack (create + evidence via the CLI binary) plus a second, distinct
/// auditor profile driving `temper_client::resources::record_citation_audit` directly (there is
/// no CLI subcommand for it — `crates/temper-cli/src/commands/resource.rs` has none).
///
/// Deliberately asserts the BEFORE shape too, not just the after: an `is_number()`-only
/// assertion would pass against an all-zeros shape regardless of whether the write surface does
/// anything at all. The load-bearing proof here is that `audit_coverage` and `citation_quality`
/// are pinned at their pre-audit values (0 and 0.0) and then MOVE once the audit lands.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn citation_audit_moves_audit_coverage_and_citation_quality(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("citation-audit-ctx", None)
        .await
        .expect("ctx create");

    // The cited source: a plain resource with no provenance of its own. `citation_magnitude`
    // only counts DISTINCT LIVE resource-kind citations
    // (`migrations/20260723000020_standing_citation_components.sql:100-124`), so the finding
    // below must cite this resource's id directly (never an `https://…` string, which resolves
    // to `ProvenanceSource::Remote` and is invisible to that count).
    let cited = cli_json(
        &app,
        &[
            "resource",
            "create",
            "--type",
            "concept",
            "--title",
            "Cited Source",
            "--context",
            "@me/citation-audit-ctx",
            "--format",
            "json",
        ],
    )
    .await;
    let cited_id = cited["id"]
        .as_str()
        .expect("cited source create response carries an `id`")
        .to_string();

    // The finding: `--sources` requires a body (`resource.rs:447`), so the body goes through a
    // temp file exactly as the first test does. The source is the cited resource's own id — a
    // bare UUID resolves via `parse_ref` to `ProvenanceSource::Resource`
    // (`temper-workflow/src/operations/refs.rs:120-127`), not `Remote`.
    let body_file = tempfile::NamedTempFile::new().expect("temp body file");
    std::fs::write(
        body_file.path(),
        "A finding distilled from a citable resource.\n",
    )
    .expect("write temp body");
    let body_arg = format!("@{}", body_file.path().display());

    let created = cli_json(
        &app,
        &[
            "resource",
            "create",
            "--type",
            "concept",
            "--title",
            "Audited Finding",
            "--context",
            "@me/citation-audit-ctx",
            "--body",
            &body_arg,
            "--sources",
            &cited_id,
            "--model",
            "claude-opus-4-8",
            "--confidence",
            "confident",
            "--format",
            "json",
        ],
    )
    .await;
    let finding_id = created["id"]
        .as_str()
        .expect("finding create response carries an `id`")
        .to_string();

    // ── Before: the resource-kind citation is live but unaudited ──
    let before = cli_json(
        &app,
        &["resource", "evidence", &finding_id, "--format", "json"],
    )
    .await;
    let citation_magnitude_before = before["citation_magnitude"]
        .as_i64()
        .unwrap_or_else(|| panic!("citation_magnitude is numeric: {before}"));
    assert_eq!(
        citation_magnitude_before, 1,
        "the finding cites exactly one distinct resource-kind source: {before}"
    );
    let audit_coverage_before = before["audit_coverage"]
        .as_i64()
        .unwrap_or_else(|| panic!("audit_coverage is numeric: {before}"));
    assert_eq!(
        audit_coverage_before, 0,
        "no audit has been recorded yet: {before}"
    );
    let citation_quality_before = before["citation_quality"]
        .as_f64()
        .unwrap_or_else(|| panic!("citation_quality is numeric: {before}"));
    assert_eq!(
        citation_quality_before, 0.0,
        "citation_quality reads neutral 0.0 when audit_coverage = 0 (spec §3.2): {before}"
    );

    // Find the block-provenance row for the resource-kind citation, to address the audit at
    // its (block, source) grain (spec §4.1) — the finding id in the request path is routing
    // only; the server derives authorization from `block_id`.
    let provenance = app
        .client
        .resources()
        .provenance(finding_id.parse().expect("finding id is a uuid"))
        .await
        .expect("fetch provenance");
    let cited_uuid: uuid::Uuid = cited_id.parse().expect("cited id is a uuid");
    let citation_row = provenance
        .iter()
        .find(|row| row.source_kind == "resource" && row.source_id == cited_uuid)
        .unwrap_or_else(|| {
            panic!("no resource-kind provenance row for {cited_id}: {provenance:?}")
        });
    let block_id = citation_row.block_id;

    // A second, distinct profile audits the citation. It must NOT be the finding's author: the
    // audit gate refuses the author with 404 (`AuditAuthority::Author`,
    // `crates/temper-api/tests/citation_audit_handler_test.rs:211-239`), and it must be able to
    // READ the finding, which — unlike the author's own writes — is not ambient for a second
    // profile until explicitly granted (`resource_grant_e2e.rs:174-179`).
    let auditor_profile_id = common::provision_and_approve_second(&app).await;
    app.client
        .resources()
        .grant(
            finding_id.parse().expect("finding id is a uuid"),
            &ResourceGrantBody {
                principal_table: "kb_profiles".to_string(),
                principal_id: auditor_profile_id,
                can_read: true,
                can_write: false,
                can_delete: false,
                can_grant: false,
            },
        )
        .await
        .expect("grant read access to the auditor");

    // …and it must be a REGISTERED MACHINE PRINCIPAL. Spec §7 gates the audit write on readability
    // AND `kb_machine_clients` membership: an audit is an agent act, and the trail it appends to is
    // permanent, unattributed at the read surface, and unbounded. Without this row the call below
    // 404s (`AuditAuthority::NotMachine`). A raw allowlist insert rather than
    // `temper admin machine provision`, because the reach this test needs is the grant above —
    // all this row carries is "is a registered agent".
    sqlx::query(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ($1, $1, $2, $2)",
    )
    .bind(format!("e2e-auditor-{auditor_profile_id}@clients"))
    .bind(auditor_profile_id)
    .execute(&app.pool)
    .await
    .expect("register the auditor as a machine principal");

    let auditor_token = common::generate_second_user_jwt();
    let auditor_client = TemperClient::with_token(
        &app.base_url(),
        Some("e2e-auditor-device".to_string()),
        temper_workflow::operations::Surface::CliCloud,
        auditor_token,
        Arc::new(temper_client::auth::MemoryTokenStore::empty()),
    );

    let audit_request = CitationAuditRequest {
        block_id,
        source: ProvenanceSource::Resource(cited_uuid),
        value: 0.8,
        reason: Some("independently verified".to_string()),
    };
    auditor_client
        .resources()
        .record_citation_audit(
            finding_id.parse().expect("finding id is a uuid"),
            &audit_request,
        )
        .await
        .expect("record citation audit as the non-author auditor");

    // ── After: the audit lands, and the axes move off their pre-audit values ──
    let after = cli_json(
        &app,
        &["resource", "evidence", &finding_id, "--format", "json"],
    )
    .await;
    let audit_coverage_after = after["audit_coverage"]
        .as_i64()
        .unwrap_or_else(|| panic!("audit_coverage is numeric: {after}"));
    assert_eq!(
        audit_coverage_after,
        audit_coverage_before + 1,
        "audit_coverage increments by exactly one covered source: before={before} after={after}"
    );
    let citation_quality_after = after["citation_quality"]
        .as_f64()
        .unwrap_or_else(|| panic!("citation_quality is numeric: {after}"));
    assert!(
        citation_quality_after > citation_quality_before,
        "citation_quality must move strictly off its neutral 0.0 baseline once a positive-value \
         audit lands, or the write surface is a no-op that only LOOKS wired up: before={before} \
         after={after}"
    );
}
