#![cfg(feature = "test-db")]
//! Issue #330's headline acceptance criterion: **an end-to-end cognitive-map authoring pass is a
//! single self-contained CLI script.**
//!
//! The issue was filed because the pass could not be scripted — `facet_set` and `cogmap materialize`
//! were believed to be MCP-only, `--format json` emitted shapes a naive parser choked on, and the
//! create/open/facet responses each used a different key for their id.
//!
//! This test walks the corrected worked example from `crates/temper-cli/skill-content/
//! cognitive-maps.md` end to end, entirely through the `temper` binary. Every response is parsed
//! with **one** `serde_json::from_slice` over the whole of stdout, so any command that emits a
//! second JSON document (the pre-#330 `--edges` behavior) fails here rather than silently passing.
//! Every id is read from the generic `id` key, so a command that only exposes a bespoke alias fails
//! here too.
//!
//! If this test ever needs an MCP call to complete, the issue has regressed.

mod common;

use serde_json::Value;
use uuid::Uuid;

/// The L0 kernel cognitive map reserved id (birth migration `20260625000001`).
const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

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

/// Read the generic `id` key. Every create-style response carries it (PR (a)), so one helper
/// serves `invocation open`, `resource create`, and `resource facet` alike.
fn id_of(v: &Value) -> String {
    v["id"]
        .as_str()
        .unwrap_or_else(|| panic!("response carries no generic `id` key: {v}"))
        .to_string()
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_whole_authoring_pass_runs_through_the_cli_alone(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let profile = app
        .client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    common::grant_cogmap_write(&pool, L0_COGMAP, profile.id).await;

    let map = L0_COGMAP.to_string();

    // 1. Open the invocation envelope. `invocation open` returns `invocation_id` AND `id`.
    let inv = cli_json(
        &app,
        &[
            "invocation",
            "open",
            "--cogmap",
            &map,
            "--trigger-kind",
            "manual",
            "--format",
            "json",
        ],
    )
    .await;
    let inv_id = id_of(&inv);
    assert_eq!(
        inv["invocation_id"], inv["id"],
        "the generic id must alias invocation_id: {inv}"
    );

    // 2. Create the first node under the invocation.
    let node_a = cli_json(
        &app,
        &[
            "resource",
            "create",
            "--type",
            "concept",
            "--title",
            "Authoring Pass Node A",
            "--cogmap",
            &map,
            "--model",
            "claude-opus-4-8",
            "--confidence",
            "confident",
            "--invocation",
            &inv_id,
            "--format",
            "json",
        ],
    )
    .await;
    let a_id = id_of(&node_a);

    // `create` carries the decorated `ref` too — it was once the only resource-returning
    // command whose output had none, so an agent had to round-trip to address what it made.
    let a_ref = node_a["ref"]
        .as_str()
        .unwrap_or_else(|| panic!("create response carries a `ref`: {node_a}"));
    assert!(
        a_ref.ends_with(&a_id),
        "ref is the decorated `sluggify(title)-<uuid>` form: {node_a}"
    );

    // 3. Create a second node that CITES the first, and let `--sources-as-edges` assert the
    //    `derived_from` edge for us — the boilerplate the issue complained about.
    //
    //    `--sources` requires a body, and `--body` takes `-` (stdin) or `@<path>` — never a
    //    literal — so the body goes through a temp file.
    let body_file = tempfile::NamedTempFile::new().expect("temp body file");
    std::fs::write(body_file.path(), "Distilled from node A.\n").expect("write temp body");
    let body_arg = format!("@{}", body_file.path().display());

    let node_b = cli_json(
        &app,
        &[
            "resource",
            "create",
            "--type",
            "concept",
            "--title",
            "Authoring Pass Node B",
            "--cogmap",
            &map,
            "--body",
            &body_arg,
            "--sources",
            &a_id,
            "--sources-as-edges",
            "--model",
            "claude-opus-4-8",
            "--confidence",
            "probable",
            "--invocation",
            &inv_id,
            "--format",
            "json",
        ],
    )
    .await;
    let b_id = id_of(&node_b);

    let asserted = node_b["edges_asserted"]
        .as_array()
        .unwrap_or_else(|| panic!("create response carries no edges_asserted: {node_b}"));
    assert_eq!(
        asserted.len(),
        1,
        "one derived_from edge per resource-valued source: {node_b}"
    );
    assert_eq!(asserted[0], a_id, "the edge points at node A: {node_b}");
    assert!(
        node_b["edges_failed"].is_null() || node_b["edges_failed"].as_array().unwrap().is_empty(),
        "no edge should have failed: {node_b}"
    );

    // 4. Facet node A — `temper resource facet`, the verb the skill content once denied existed.
    let facet = cli_json(
        &app,
        &[
            "resource",
            "facet",
            &a_id,
            "--values",
            r#"{"stance":"accepted"}"#,
            "--confidence",
            "confident",
            "--invocation",
            &inv_id,
            "--format",
            "json",
        ],
    )
    .await;
    assert_eq!(
        facet["property_id"], facet["id"],
        "the generic id must alias property_id: {facet}"
    );

    // 4b. Assert an explicit `near`/`forward` edge from node A to node B — `temper edge assert`,
    //     the other authored verb the issue found silently ignoring `--format json`.
    let edge = cli_json(
        &app,
        &[
            "edge",
            "assert",
            &a_id,
            &b_id,
            "--kind",
            "near",
            "--polarity",
            "forward",
            "--label",
            "relates_to",
            "--weight",
            "1.0",
            "--confidence",
            "confident",
            "--invocation",
            &inv_id,
            "--format",
            "json",
        ],
    )
    .await;
    assert!(
        edge["edge_handle"].as_str().is_some(),
        "edge assert response carries edge_handle: {edge}"
    );

    // 5. Materialize — `temper cogmap materialize`, the one genuinely missing verb.
    let ack = cli_json(&app, &["cogmap", "materialize", &map, "--format", "json"]).await;
    assert!(
        ack.get("materialized").is_some(),
        "materialize ack shape: {ack}"
    );

    // 6. Close the envelope.
    let _closed = cli_json(
        &app,
        &[
            "invocation",
            "close",
            &inv_id,
            "--disposition",
            "completed",
            "--format",
            "json",
        ],
    )
    .await;

    // 7. Verify the close took — `disposition` now round-trips on `show`, derived from `status`.
    let view = cli_json(&app, &["invocation", "show", &inv_id, "--format", "json"]).await;
    assert_eq!(view["status"], "completed", "status: {view}");
    assert_eq!(
        view["disposition"], "completed",
        "the derived disposition must round-trip on show: {view}"
    );

    // 8. The provenance stamp and the derived_from edge are both visible on one `show`, in ONE
    //    JSON document — `--edges` used to emit a second one.
    let shown = cli_json(
        &app,
        &["resource", "show", &b_id, "--edges", "--format", "json"],
    )
    .await;
    assert_eq!(
        shown["managed_meta"]["temper-provenance"], "llm-discovered",
        "provenance stamped on the authored node: {shown}"
    );
    assert_eq!(
        shown["managed_meta"]["temper-llm-run"], inv_id,
        "the node joins back to the run that authored it: {shown}"
    );

    let outgoing = shown["edges"]["outgoing"]
        .as_array()
        .unwrap_or_else(|| panic!("edges folded into the resource object: {shown}"));
    assert!(
        outgoing.iter().any(|e| e["label"] == "derived_from"),
        "--sources-as-edges asserted a derived_from edge: {shown}"
    );
}
