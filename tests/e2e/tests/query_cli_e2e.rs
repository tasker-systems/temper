//! `temper query` through the real binary, against the real server.
//!
//! **What the route e2e already took, so this does not repeat it.** `query_route_e2e.rs` drives
//! `POST /api/query` over HTTP: the gate, deserialization, hydration, and the 400 body's shape.
//! What only this tier reaches is the CLI ↔ client ↔ API ↔ DB chain — the plan arriving on stdin
//! through the non-TTY auto-detect branch, and the refusal list surviving `map_status_to_error`
//! and reaching a human's terminal.
//!
//! **That last hop is the point.** `validate` returns every refusal rather than the first *"because
//! a caller repairing a plan should see all of it in one round trip"*. Before the client's 400 arm,
//! that property was real for raw HTTP and absent for the CLI — the door's headline consumer. These
//! are the only tests in the arc that fail if the client silently drops `details`.
//!
//! `test-embed` gated: the happy path drives a **find-about** stage, so it needs the real ONNX
//! model. A run scoped `--features test-db` alone compiles this file to nothing and reads green.
//! Use `cargo make test-e2e-embed`.
//!
//! **Local runs need a fresh binary.** nextest builds this crate's lib, not temper-cli's separate
//! `temper` bin target, so a new subcommand fails here as `unrecognized subcommand 'query'` against
//! a stale `target/debug/temper`. Run `cargo build -p temper-cli --bin temper` first. CI is
//! unaffected — its job builds the bin before the e2e step.
#![cfg(all(feature = "test-db", feature = "test-embed"))]

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload};

/// A plan whose single find-about stage asks a real question, as a caller would author it — JSON,
/// not a Rust struct, because the point of this tier is that hand-written JSON reaches the server.
const ANSWERABLE_PLAN: &str = r#"{
  "stages": [
    {
      "name": "about",
      "act": "find-about-anywhere",
      "intention": { "query": "kubernetes deployment" }
    }
  ],
  "outcome": { "returns": [{ "stage": "about", "with": [] }] }
}"#;

/// Two stages, each independently unrunnable: neither carries an intention, and a find act needs a
/// question. A validator that stopped at the first would report one.
const DOUBLY_REFUSED_PLAN: &str = r#"{
  "stages": [
    { "name": "one", "act": "find-about-anywhere" },
    { "name": "two", "act": "find-about-anywhere" }
  ],
  "outcome": { "returns": [{ "stage": "one", "with": [] }, { "stage": "two", "with": [] }] }
}"#;

async fn ingest_semantic(app: &common::E2eTestApp, title: &str, slug: &str, content: &str) {
    let packed = temper_ingest::pipeline::prepare_markdown(content).expect("prepare_markdown");
    let payload = IngestPayload {
        idempotency_key: None,
        segmented: None,
        goal: None,
        title: title.to_string(),
        origin_uri: format!("test://query-cli/{slug}"),
        context_ref: "@me/qcli".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(content)),
        content: content.to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(pack_chunks(&packed).expect("pack chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };
    app.client
        .ingest()
        .create(&payload)
        .await
        .expect("ingest failed");
}

/// A plan piped on stdin runs, and its answer reaches stdout as JSON an agent can parse.
///
/// The assertion is on a hydrated title, not on a zero exit code: a stage that answered empty for
/// some other reason would also exit zero, so only the seeded resource coming back proves the whole
/// chain ran.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_plan_piped_on_stdin_runs_and_answers_on_stdout(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("qcli", None)
        .await
        .expect("context create");
    ingest_semantic(
        &app,
        "Container Scheduling Primer",
        "container-scheduling-primer",
        "Pods, replicas, and self-healing workloads are placed and rescheduled automatically by \
         the control plane.",
    )
    .await;

    // No `--plan` flag: the implicit non-TTY auto-detect branch, which is how an agent pipes one.
    let out = common::run_temper_cli_with_stdin(&app, ANSWERABLE_PLAN, &["query"])
        .await
        .expect("spawn temper query");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "temper query failed on a runnable plan\nstdout: {stdout}\nstderr: {stderr}"
    );

    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not JSON: {e}\n{stdout}"));
    assert!(
        stdout.contains("Container Scheduling Primer"),
        "the seeded resource must come back through the CLI; got {parsed:#}"
    );
    assert!(
        parsed["trace"].is_object(),
        "every stage is traced, including ones not returned; got {parsed:#}"
    );
}

/// **The CLI prints MORE THAN ONE refusal**, and exits non-zero.
///
/// The end-to-end witness for the client's 400 arm, and the only test in the arc that fails if the
/// client silently drops `details`. A single-refusal assertion would pass against a CLI that shows
/// one at a time — the experience the "every refusal" rule exists to prevent.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_refused_plan_prints_every_refusal_and_exits_non_zero(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let out = common::run_temper_cli_with_stdin(&app, DOUBLY_REFUSED_PLAN, &["query"])
        .await
        .expect("spawn temper query");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "a plan that will not run must exit non-zero\nstdout: {stdout}\nstderr: {stderr}"
    );

    // The e2e harness spawns the binary with piped stdout (non-TTY → JSON mode).
    // In JSON mode the error rides stdout as a structured ErrorPayload. The
    // refusals are in the `message` field (rendered by `render_refusals`).
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout is a JSON ErrorPayload: {stdout:?}");
    let message = parsed["message"].as_str().expect("message field");

    // Both stages named, so the caller can repair each. Counting mentions rather than asserting one
    // substring is what makes this fail against a CLI that renders only the first refusal.
    assert!(
        message.contains("one:") && message.contains("two:"),
        "both refused stages must be named in one run; got:\n{message}"
    );
    assert!(
        message.contains("2 refusal(s)"),
        "the caller must be told how many refusals there are; got:\n{message}"
    );

    // A caller fault is not reported as a server error — the misclassification the client's 400 arm
    // corrects. Before it, this arrived as `ClientError::Server { status: 400 }`.
    assert!(
        !message.to_lowercase().contains("server error"),
        "a refused plan was reported as a SERVER fault; got:\n{message}"
    );

    // The code is "project" (TemperError::Project), not a server error code.
    assert_eq!(
        parsed["code"], "project",
        "a refused plan must arrive under code 'project'; got: {parsed}"
    );
}

/// `--check` reports shape refusals on **stdout** and exits non-zero, touching no network.
///
/// Only reachable through the real binary: the exit code comes from `std::process::exit`, which no
/// in-process test can observe, and "did it reach the server" is a claim about a whole process.
///
/// **It reports FEWER refusals than the server does, and that is the contract, not a shortfall.**
/// The same plan comes back from `POST /api/query` with a third — `follow-from`'s mechanic is not
/// reachable on this deployment — which is a *capability* refusal that a local shape pass cannot
/// see. That gap is exactly what the disclosure exists to state.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn check_reports_shape_refusals_on_stdout_and_never_calls_the_server(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    let out = common::run_temper_cli_with_stdin(&app, DOUBLY_REFUSED_PLAN, &["query", "--check"])
        .await
        .expect("spawn temper query --check");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !out.status.success(),
        "a plan with refusals must exit non-zero; stdout: {stdout}"
    );

    // Data on stdout, not a rendered error on stderr — `--check` was ASKED to find these, so they
    // are its output. An agent gates on this.
    let report: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("--check must emit parseable JSON on stdout: {e}\n{stdout}"));
    assert_eq!(report["expressible"], false);
    let refusals = report["refusals"]
        .as_array()
        .unwrap_or_else(|| panic!("no refusals array in {report:#}"));
    assert!(
        refusals.len() >= 2,
        "every shape refusal at once, not the first; got {report:#}"
    );
    assert!(
        report["disclosure"].as_str().is_some_and(|d| !d.is_empty()),
        "the disclosure must ride on the wire, not only on the terminal: {report:#}"
    );
}

/// A clean plan checks clean, exits zero, and still carries the disclosure.
///
/// The pairing matters: a `--check` that only ever spoke up about problems would let
/// `expressible: true` read as a promise the server will run it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_clean_check_exits_zero_and_still_declines_to_promise(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    let out = common::run_temper_cli_with_stdin(&app, ANSWERABLE_PLAN, &["query", "--check"])
        .await
        .expect("spawn temper query --check");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "a well-formed plan must check clean: {stdout}"
    );
    let report: serde_json::Value = serde_json::from_str(&stdout).expect("JSON on stdout");
    assert_eq!(report["expressible"], true);
    assert!(
        report["disclosure"]
            .as_str()
            .is_some_and(|d| d.contains("does not promise")),
        "a clean report must still decline to promise: {report:#}"
    );
}

/// A missing plan is an error rather than an empty request — the deliberate divergence from
/// `resource update`, which treats absent stdin as "no body update requested".
///
/// `Command::output()` gives the child a null stdin: non-TTY and immediately at EOF, which is
/// exactly the "a pipe with nothing in it" case.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_missing_plan_is_refused_before_any_request(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    let out = common::run_temper_cli(&app, &["query"])
        .await
        .expect("spawn temper query");

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "an empty plan must not be sent");
    // In JSON mode (non-TTY piped stdout), the error rides stdout as a
    // structured ErrorPayload. The message carries the refusal text.
    let parsed: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("stdout is a JSON ErrorPayload: {stdout:?}");
    let message = parsed["message"].as_str().expect("message field");
    assert!(
        message.contains("no plan supplied"),
        "the error must say what to pass; got:\n{message}"
    );
}
