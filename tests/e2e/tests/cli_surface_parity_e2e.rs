#![cfg(feature = "test-db")]

//! E2E: the CLI↔client surface-parity verbs, probed against the wire — task
//! 01a0e2ed-a39d-74c0-925c-e5357eeb8254's observable evidence.
//!
//! The task's scan named the client methods with no CLI caller; this file drives the
//! commands that closed them through the REAL binary, and pins the refusal faces the
//! direct route code names (the task's grounding rule: observe the door's face, never
//! inherit it from a sibling's docs):
//!
//! - `context materialize-delta` / `cogmap materialize-delta` — the formation-drift
//!   reads; deny is the route's uniform 404.
//! - `resource meta get/set` — the metadata-only door; the PUT MERGES per key (named
//!   keys overwrite, omitted keys are preserved — observed at the backend; the route
//!   comment's "stated in full" claim does not hold), and never touches the body.
//! - `resource audit-citation` — the block-addressed audit door; an out-of-range
//!   `--value` is refused client-side with the verdict sentence, an absent block is the
//!   door's 404.
//! - `data-artifact get` — the flat artifact read; `schema list/declare --cogmap` — the
//!   cogmap-home shape arms.
//! - `blob progress` — the segmented-upload resume read; an absent session is the
//!   owner-private 404.

mod common;

use serde_json::Value;
use uuid::Uuid;

/// The L0 kernel cognitive map reserved id (birth migration `20260625000001`) — the
/// always-present cogmap home.
const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

/// Run the CLI and parse stdout as exactly one JSON document.
async fn cli_json(app: &common::E2eTestApp, args: &[&str]) -> Value {
    let output = common::run_temper_cli(app, args).await.expect("cli run");
    assert!(
        output.status.success(),
        "cli {args:?} failed: stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "cli {args:?} did not emit exactly one JSON document ({e}): {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

/// Run the CLI expecting failure; return (exit success, both streams) for the refusal face.
///
/// The face rides EITHER stream: the agent-first default emits the JSON error envelope
/// on stdout (`{"code":...,"message":...}`), while client-side refusals print to
/// stderr. The guard asserts over both so a door that moves its face between streams
/// stays pinned.
async fn cli_refusal(app: &common::E2eTestApp, args: &[&str]) -> (bool, String) {
    let output = common::run_temper_cli(app, args).await.expect("cli run");
    (
        output.status.success(),
        format!(
            "stderr={} stdout={}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        ),
    )
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_materialize_delta_reads_answer_through_the_cli(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("delta-ctx", None)
        .await
        .expect("ctx create");

    let delta = cli_json(
        &app,
        &[
            "context",
            "materialize-delta",
            "@me/delta-ctx",
            "--format",
            "json",
        ],
    )
    .await;
    assert_eq!(delta["anchor_table"], "kb_contexts");
    assert!(
        delta["formation_events"].is_i64(),
        "delta carries the formation-event count: {delta}"
    );

    let cogmap_delta = cli_json(
        &app,
        &[
            "cogmap",
            "materialize-delta",
            &L0_COGMAP.to_string(),
            "--format",
            "json",
        ],
    )
    .await;
    assert_eq!(cogmap_delta["anchor_table"], "kb_cogmaps");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_materialize_delta_deny_is_the_uniform_404(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let absent = Uuid::now_v7();
    let (ok, faces) =
        cli_refusal(&app, &["context", "materialize-delta", &absent.to_string()]).await;
    assert!(!ok, "an absent context's delta read must refuse");
    assert!(
        faces.contains("not found") || faces.contains("404"),
        "deny renders as the route's uniform 404 posture, got: {faces}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_meta_get_set_is_the_metadata_only_door(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("meta-ctx", None)
        .await
        .expect("ctx create");

    let created = cli_json(
        &app,
        &[
            "resource",
            "create",
            "--type",
            "note",
            "--title",
            "meta door probe",
            "--context",
            "@me/meta-ctx",
            "--format",
            "json",
        ],
    )
    .await;
    let id = created["id"]
        .as_str()
        .unwrap_or_else(|| panic!("create carries the generic id key: {created}"))
        .to_string();

    // The read answers both tiers; the body travels with it (ResourceView).
    let got = cli_json(&app, &["resource", "meta", "get", &id, "--format", "json"]).await;
    assert_eq!(
        got["id"], created["id"],
        "the meta read addresses the resource"
    );

    // The set states BOTH tiers in full — and never re-chunks (the body is untouched).
    let set = cli_json(
        &app,
        &[
            "resource",
            "meta",
            "set",
            &id,
            "--managed",
            r#"{"temper-status":"paused"}"#,
            "--open",
            r#"{"marker":"x","tags":["a"]}"#,
            "--format",
            "json",
        ],
    )
    .await;
    assert_eq!(
        set["open_meta"]["marker"], "x",
        "the open tier lands: {set}"
    );

    // Merge semantics, OBSERVED at the wire (the route comment's "states the tiers in
    // full" does not hold at the backend): named keys overwrite, omitted keys survive —
    // `tags` survives a set that doesn't name it, `marker` survives a set that names
    // only `tags`. Clearing goes through `resource update`'s null-deletes channel.
    let replaced = cli_json(
        &app,
        &[
            "resource",
            "meta",
            "set",
            &id,
            "--managed",
            r#"{"temper-status":"paused"}"#,
            "--open",
            r#"{"tags":["b"]}"#,
            "--format",
            "json",
        ],
    )
    .await;
    assert_eq!(
        replaced["open_meta"]["tags"],
        Value::Array(vec![Value::String("b".to_string())]),
        "a named key overwrites: {replaced}"
    );
    assert_eq!(
        replaced["open_meta"]["marker"], "x",
        "an omitted key is preserved by the per-key merge: {replaced}"
    );
    // The body survived both writes untouched — pinned by comparing the reconstituted
    // content and body_hash across the writes, not by expecting an empty body.
    let before = cli_json(&app, &["resource", "show", &id, "--format", "json"]).await;
    let set2 = cli_json(
        &app,
        &[
            "resource",
            "meta",
            "set",
            &id,
            "--managed",
            r#"{"temper-status":"active"}"#,
            "--open",
            r#"{"tags":["b"]}"#,
            "--format",
            "json",
        ],
    )
    .await;
    let after = cli_json(&app, &["resource", "show", &id, "--format", "json"]).await;
    assert_eq!(
        before["content"], after["content"],
        "the metadata-only door must not touch the body"
    );
    assert_eq!(
        before["body_hash"], after["body_hash"],
        "the metadata-only door must not restale the body hash: {set2}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_out_of_range_verdict_is_refused_client_side_with_the_sentence(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    let (ok, faces) = cli_refusal(
        &app,
        &[
            "resource",
            "audit-citation",
            &Uuid::now_v7().to_string(),
            "--source",
            &Uuid::now_v7().to_string(),
            "--value",
            "1.5",
        ],
    )
    .await;
    assert!(!ok, "a verdict outside [-1, 1] must refuse");
    assert!(
        faces.contains("[-1.0, 1.0]"),
        "the refusal carries the verdict sentence, got: {faces}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_absent_address_refusals_carry_their_door_faces(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    // audit-citation on an absent block: the door's 404 — unreadable finding, self-audit,
    // and absent block are ONE sentence by design.
    let (ok, faces) = cli_refusal(
        &app,
        &[
            "resource",
            "audit-citation",
            &Uuid::now_v7().to_string(),
            "--source",
            &Uuid::now_v7().to_string(),
            "--value",
            "0.5",
        ],
    )
    .await;
    assert!(!ok, "an audit on an absent block must refuse");
    assert!(
        faces.contains("not found") || faces.contains("404"),
        "the block-addressed audit's absent face is its 404, got: {faces}"
    );

    // blob progress on an absent upload: owner-private staging — 404 either way.
    let (ok, faces) = cli_refusal(&app, &["blob", "progress", &Uuid::now_v7().to_string()]).await;
    assert!(!ok, "an absent upload session must refuse");
    assert!(
        faces.contains("not found") || faces.contains("404"),
        "the progress read's absent face is its 404, got: {faces}"
    );

    // data-artifact get on an absent id: the flat read's 404.
    let (ok, faces) =
        cli_refusal(&app, &["data-artifact", "get", &Uuid::now_v7().to_string()]).await;
    assert!(!ok, "an absent artifact must refuse");
    assert!(
        faces.contains("not found") || faces.contains("404"),
        "the flat read's absent face is its 404, got: {faces}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_schema_cogmap_home_arms_declare_and_list_through_the_cli(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");

    // A fresh cogmap home — genesis through the client, then the CLI drives both arms.
    let outcome = app
        .client
        .cognitive_maps()
        .create_cognitive_map(&temper_core::types::reconcile::CreateCogmapRequest {
            cogmap_id: None,
            telos_resource_id: None,
            name: "parity guard probe map".to_string(),
            telos_title: "probe telos".to_string(),
            telos: None,
        })
        .await
        .expect("cogmap genesis");
    let cogmap = outcome.cogmap_id.to_string();

    // The schema content rides stdin (`--content -`), the body-source convention.
    let schema = r#"{"$schema":"https://json-schema.org/draft/2020-12/schema","type":"object"}"#;
    let output = common::run_temper_cli_with_stdin(
        &app,
        schema,
        &[
            "data-artifact",
            "schema",
            "declare",
            "--cogmap",
            &cogmap,
            "--kind",
            "measurement",
            "--enforcement",
            "advisory",
            "--content",
            "-",
            "--format",
            "json",
        ],
    )
    .await
    .expect("cli run");
    assert!(
        output.status.success(),
        "cogmap-home schema declare failed: stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let declared: Value = serde_json::from_slice(&output.stdout).expect("one JSON document");
    assert_eq!(
        declared["artifact_kind"], "measurement",
        "the cogmap-home declare lands: {declared}"
    );
    assert_eq!(
        declared["home_anchor_table"], "kb_cogmaps",
        "the shape's home anchor is the cogmap: {declared}"
    );

    let listed = cli_json(
        &app,
        &[
            "data-artifact",
            "schema",
            "list",
            "--cogmap",
            &cogmap,
            "--format",
            "json",
        ],
    )
    .await;
    assert_eq!(
        listed.as_array().map(|a| a.len()),
        Some(1),
        "the cogmap-home list carries the declared shape: {listed}"
    );

    // Exactly-one home: both flags is a client-side refusal, not a silent pick.
    let (ok, faces) = cli_refusal(
        &app,
        &[
            "data-artifact",
            "schema",
            "list",
            "--cogmap",
            &cogmap,
            "--context",
            "@me/nope",
        ],
    )
    .await;
    assert!(
        !ok,
        "naming both homes must refuse — the face assert below must hold on a refusal"
    );
    // A missing context ref would 404 at the resolver; BOTH given refuses EARLIER,
    // client-side, with the exactly-one-home sentence — assert the FACE, not just the
    // refusal, or the run would stay green if the validation were deleted.
    assert!(
        faces.contains("exactly one home"),
        "naming both homes must refuse with the exactly-one-home sentence: {faces}"
    );
}
