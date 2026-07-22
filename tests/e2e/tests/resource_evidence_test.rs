#![cfg(feature = "test-db")]
//! Set 3, Task 9: `temper resource evidence <ref>` drives the CLI ↔ API ↔ DB stack and
//! returns a `StandingShape` — the evidential-standing shape vector plus the lossy read-time
//! `band` chip carried WITH it, never in place of it (spec §1.1).
//!
//! The load-bearing assertion is that the whole struct rides through: all eight fields are
//! present on the wire, `band` sits alongside the numeric components (not instead of them),
//! and `finding_id` anchors the shape to the resource we asked about. A provenance-bearing
//! create (a body plus a remote `--sources` record) exercises the reinforcement component
//! `r_parent` end-to-end rather than reading a shape that is all zeros.

mod common;

use serde_json::Value;

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
    // is carried WITH the shape, never in place of it.
    for key in [
        "finding_id",
        "indep_breadth",
        "adversarial_survival",
        "challenge_count",
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
        shape["indep_breadth"].is_number(),
        "indep_breadth is numeric: {shape}"
    );
    assert!(
        shape["challenge_count"].is_number(),
        "challenge_count is numeric: {shape}"
    );
}
