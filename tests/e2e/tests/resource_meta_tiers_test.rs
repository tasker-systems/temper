#![cfg(feature = "test-db")]
//! Issue #330: full `resource show` carries both metadata tiers, `--meta-only` is a
//! literal strict subset of it, and the provenance trio is stamped server-side.
//!
//! Differential by construction: the load-bearing assertion compares the two read paths
//! against each other rather than against a typed-out expected shape. A hand-written
//! expectation would just reproduce the author's understanding of the wire format; making
//! the paths check each other cannot.

mod common;

use serde_json::Value;

/// Run the CLI and parse stdout as exactly one JSON document.
///
/// `from_slice` consumes the whole buffer, so trailing data (the pre-#330 `--edges`
/// behavior) fails here rather than being silently ignored.
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

/// The acceptance criterion for #330's items 3 and 4, in one pass:
/// the trio is stamped on create, the full `show` carries both tiers, and every key
/// `--meta-only` returns is present with an equal value in the full `show`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn meta_only_is_a_strict_subset_of_full_show_and_carries_the_stamp(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("subset-ctx", None)
        .await
        .expect("ctx create");

    // Create through the real CLI code path, with an LLM authorship envelope.
    let created = cli_json(
        &app,
        &[
            "resource",
            "create",
            "--type",
            "concept",
            "--title",
            "Subset Probe",
            "--context",
            "@me/subset-ctx",
            "--model",
            "claude-opus-4-8",
            "--confidence",
            "confident",
            "--open-meta",
            r#"{"custom":"value"}"#,
            "--format",
            "json",
        ],
    )
    .await;

    let id = created["id"]
        .as_str()
        .expect("create response carries an `id`")
        .to_string();

    // ---- full show ----
    let full = cli_json(&app, &["resource", "show", &id, "--format", "json"]).await;

    // The server-side stamp landed in the managed tier. Before #330 the CLI's
    // --model/--invocation flags populated only the act event, never managed_meta.
    assert_eq!(
        full["managed_meta"]["temper-provenance"], "llm-discovered",
        "provenance stamped: {full}"
    );
    assert_eq!(
        full["managed_meta"]["temper-llm-model"], "claude-opus-4-8",
        "model stamped: {full}"
    );

    // open_meta is present on the FULL view — it used to be absent entirely, so a script
    // reading it off `show` silently got null.
    assert_eq!(
        full["open_meta"]["custom"], "value",
        "open_meta on full show: {full}"
    );

    // ---- meta-only ----
    let meta = cli_json(
        &app,
        &["resource", "show", &id, "--meta-only", "--format", "json"],
    )
    .await;

    // THE differential assertion: every key --meta-only returns is present, with an equal
    // value, in the full show object. No expected shape is typed out anywhere.
    let meta_obj = meta.as_object().expect("meta-only is an object");
    assert!(!meta_obj.is_empty(), "meta-only returned nothing: {meta}");
    for (key, meta_value) in meta_obj {
        let full_value = full
            .get(key)
            .unwrap_or_else(|| panic!("full show is missing `{key}` that --meta-only returned"));
        assert_eq!(
            full_value, meta_value,
            "`{key}` disagrees between full show and --meta-only"
        );
    }

    // The anchor is `id` on both — that rename is what makes the subset relation possible.
    assert!(
        meta_obj.contains_key("id"),
        "meta-only anchors on id: {meta}"
    );

    // And the §7-dissolved hashes are gone from the wire entirely, not emitted empty.
    assert!(!meta_obj.contains_key("managed_hash"), "{meta}");
    assert!(!meta_obj.contains_key("open_hash"), "{meta}");
}

/// A create with no `--model` is a human act: `user-created`, and no LLM fields.
/// `base.schema.json` has always declared this value; nothing produced it until #330.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_llm_create_is_stamped_user_created(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("user-created-ctx", None)
        .await
        .expect("ctx create");

    let created = cli_json(
        &app,
        &[
            "resource",
            "create",
            "--type",
            "concept",
            "--title",
            "Human Node",
            "--context",
            "@me/user-created-ctx",
            "--format",
            "json",
        ],
    )
    .await;
    let id = created["id"].as_str().expect("id").to_string();

    let full = cli_json(&app, &["resource", "show", &id, "--format", "json"]).await;

    assert_eq!(
        full["managed_meta"]["temper-provenance"], "user-created",
        "no --model means a human act: {full}"
    );
    assert!(
        full["managed_meta"].get("temper-llm-model").is_none(),
        "no model to record: {full}"
    );
    assert!(
        full["managed_meta"].get("temper-llm-run").is_none(),
        "no invocation to record: {full}"
    );
}
