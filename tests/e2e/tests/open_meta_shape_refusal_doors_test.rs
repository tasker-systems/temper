#![cfg(feature = "test-db")]
//! Open-tier shape is refused by the server, and the CLI relays that refusal intact.
//!
//! Ruling R8 makes the server the only enforcing copy of request shaping. `DbBackend`'s
//! `validate_open_meta_shape` refuses a recognized open_meta key carrying the wrong shape on
//! create, on update's SET half and on the `open_meta_add` union. The CLI no longer runs a copy
//! of that check before sending, so its refusal for a mis-shaped key is the server's refusal:
//! the request reaches the server, and the CLI renders the answer as it renders any other server
//! refusal, with the same exit code and the same JSON payload the HTTP door's error carries.
//!
//! The reference refusal is one the CLI never had a copy of: a scalar on the add channel
//! (`open_meta_add_properties`). A shape refusal that renders differently from it is a door
//! translating a refusal the server already named.

mod common;

use serde_json::{json, Value};

async fn cli(app: &common::E2eTestApp, args: &[&str]) -> std::process::Output {
    common::run_temper_cli(app, args)
        .await
        .expect("spawn the temper binary")
}

fn stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not one JSON document ({e}): stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// A resource to update, created through the CLI, with a tag and an unrelated key to watch.
async fn seeded(app: &common::E2eTestApp, ctx: &str) -> String {
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create(ctx, None)
        .await
        .expect("ctx create");
    let created = cli(
        app,
        &[
            "--format",
            "json",
            "resource",
            "create",
            "--type",
            "concept",
            "--title",
            "Open Meta Shape Probe",
            "--context",
            &format!("@me/{ctx}"),
            "--open-meta",
            r#"{"tags":["alpha"],"marker":"keep"}"#,
        ],
    )
    .await;
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    stdout_json(&created)["id"]
        .as_str()
        .expect("id")
        .to_string()
}

/// The HTTP door's answer to the same PATCH, as the CLI's JSON payload would carry it.
async fn http_refusal(app: &common::E2eTestApp, id: &str, body: Value) -> (u16, Value) {
    let res = reqwest::Client::new()
        .patch(app.url(&format!("/api/resources/{id}")))
        .bearer_auth(&app.token)
        .json(&body)
        .send()
        .await
        .expect("PATCH");
    let status = res.status().as_u16();
    let body: Value = res.json().await.expect("error body is JSON");
    (status, body["error"].clone())
}

async fn stored_open_meta(app: &common::E2eTestApp, id: &str) -> Value {
    let shown = cli(app, &["--format", "json", "resource", "show", id]).await;
    assert!(shown.status.success());
    stdout_json(&shown)["open_meta"].clone()
}

/// The CLI's rendering of a refusal: its exit code and the payload's `code`, `message` and
/// `details`, the fields the HTTP door's error also carries.
fn rendered(output: &std::process::Output) -> (Option<i32>, Value) {
    let payload = stdout_json(output);
    (
        output.status.code(),
        json!({
            "code": payload["code"],
            "message": payload["message"],
            "details": payload.get("details").cloned().unwrap_or(Value::Null),
        }),
    )
}

fn http_shape(error: &Value) -> Value {
    json!({
        "code": error["code"],
        "message": error["message"],
        "details": error.get("details").cloned().unwrap_or(Value::Null),
    })
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_misshaped_key_on_update_is_refused_by_the_server_and_relayed_intact(pool: sqlx::PgPool) {
    let (app, log) = common::setup_recording(pool).await;
    let id = seeded(&app, "shape-update-ctx").await;
    let patch_path = format!("/api/resources/{id}");
    let patches_before = log
        .lock()
        .unwrap()
        .iter()
        .filter(|p| **p == patch_path)
        .count();

    let refused = cli(
        &app,
        &[
            "--format",
            "json",
            "resource",
            "update",
            &id,
            "--open-meta",
            r#"{"descriptor":42}"#,
        ],
    )
    .await;

    // The request reached the server: the CLI judged nothing before sending.
    let patches_after = log
        .lock()
        .unwrap()
        .iter()
        .filter(|p| **p == patch_path)
        .count();
    assert!(
        patches_after > patches_before,
        "the update must reach the server, which owns the shape refusal; stdout={}",
        String::from_utf8_lossy(&refused.stdout)
    );

    let (status, http_error) =
        http_refusal(&app, &id, json!({"open_meta": {"descriptor": 42}})).await;
    assert_eq!(status, 400, "{http_error}");
    let (exit, cli_error) = rendered(&refused);
    assert_eq!(exit, Some(1), "a refusal exits 1");
    assert_eq!(
        cli_error,
        http_shape(&http_error),
        "the CLI relays the server's refusal, not its own"
    );
    let message = cli_error["message"].as_str().expect("message");
    assert!(
        message.starts_with("invalid open_meta shape:") && message.contains("descriptor"),
        "the server's refusal, naming the key: {cli_error}"
    );

    // The reference: a server-only refusal on the same verb renders with the same kind and exit.
    let reference = cli(
        &app,
        &[
            "--format",
            "json",
            "resource",
            "update",
            &id,
            "--open-meta-add",
            r#"{"tags":"beta"}"#,
        ],
    )
    .await;
    let (ref_exit, ref_error) = rendered(&reference);
    assert_eq!(
        ref_exit, exit,
        "the same exit code as any other server refusal"
    );
    assert_eq!(
        ref_error["code"], cli_error["code"],
        "the same kind as any other server refusal: {ref_error} vs {cli_error}"
    );

    let stored = stored_open_meta(&app, &id).await;
    assert!(
        stored.get("descriptor").is_none(),
        "nothing was written: {stored}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_misshaped_key_on_the_add_channel_is_refused_by_the_server_and_relayed_intact(
    pool: sqlx::PgPool,
) {
    let (app, log) = common::setup_recording(pool).await;
    let id = seeded(&app, "shape-add-ctx").await;
    let patch_path = format!("/api/resources/{id}");
    let patches_before = log
        .lock()
        .unwrap()
        .iter()
        .filter(|p| **p == patch_path)
        .count();

    let refused = cli(
        &app,
        &[
            "--format",
            "json",
            "resource",
            "update",
            &id,
            "--open-meta-add",
            r#"{"tags":[1,2]}"#,
        ],
    )
    .await;

    let patches_after = log
        .lock()
        .unwrap()
        .iter()
        .filter(|p| **p == patch_path)
        .count();
    assert!(
        patches_after > patches_before,
        "the update must reach the server; stdout={}",
        String::from_utf8_lossy(&refused.stdout)
    );

    let (status, http_error) =
        http_refusal(&app, &id, json!({"open_meta_add": {"tags": [1, 2]}})).await;
    assert_eq!(status, 400, "{http_error}");
    let (exit, cli_error) = rendered(&refused);
    assert_eq!(exit, Some(1), "a refusal exits 1");
    assert_eq!(
        cli_error,
        http_shape(&http_error),
        "the CLI relays the server's refusal, not its own"
    );

    let stored = stored_open_meta(&app, &id).await;
    assert_eq!(
        stored["tags"],
        json!(["alpha"]),
        "nothing was written: {stored}"
    );
}

/// `{"tags":null}` is the delete verb, not a mis-shaped value: with the CLI's copy gone, the
/// server alone tells the two apart, and it still deletes.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_null_recognized_key_on_update_still_deletes(pool: sqlx::PgPool) {
    let (app, log) = common::setup_recording(pool).await;
    let id = seeded(&app, "shape-null-ctx").await;
    let patch_path = format!("/api/resources/{id}");
    let patches_before = log
        .lock()
        .unwrap()
        .iter()
        .filter(|p| **p == patch_path)
        .count();

    let deleted = cli(
        &app,
        &[
            "--format",
            "json",
            "resource",
            "update",
            &id,
            "--open-meta",
            r#"{"tags":null}"#,
        ],
    )
    .await;
    assert!(
        deleted.status.success(),
        "the delete verb is not refused: stdout={} stderr={}",
        String::from_utf8_lossy(&deleted.stdout),
        String::from_utf8_lossy(&deleted.stderr)
    );
    let patches_after = log
        .lock()
        .unwrap()
        .iter()
        .filter(|p| **p == patch_path)
        .count();
    assert!(
        patches_after > patches_before,
        "the update reached the server"
    );

    let stored = stored_open_meta(&app, &id).await;
    assert!(stored.get("tags").is_none(), "the key is deleted: {stored}");
    assert_eq!(stored["marker"], "keep", "unnamed keys are untouched");
}

/// Create already leaves shape to the server; this pins it beside update, so the two verbs
/// cannot drift back apart.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_misshaped_key_on_create_is_refused_by_the_server_and_relayed_intact(pool: sqlx::PgPool) {
    let (app, log) = common::setup_recording(pool).await;
    let _ = seeded(&app, "shape-create-ctx").await;
    let creates_before = log
        .lock()
        .unwrap()
        .iter()
        .filter(|p| p.starts_with("/api/resources") || p.starts_with("/api/ingest"))
        .count();

    let refused = cli(
        &app,
        &[
            "--format",
            "json",
            "resource",
            "create",
            "--type",
            "concept",
            "--title",
            "Mis-shaped Create",
            "--context",
            "@me/shape-create-ctx",
            "--open-meta",
            r#"{"date":"July 11"}"#,
        ],
    )
    .await;

    let creates_after = log
        .lock()
        .unwrap()
        .iter()
        .filter(|p| p.starts_with("/api/resources") || p.starts_with("/api/ingest"))
        .count();
    assert!(
        creates_after > creates_before,
        "the create must reach the server; stdout={}",
        String::from_utf8_lossy(&refused.stdout)
    );
    let (exit, cli_error) = rendered(&refused);
    assert_eq!(exit, Some(1), "a refusal exits 1");
    let message = cli_error["message"].as_str().expect("message");
    assert!(
        message.starts_with("invalid open_meta shape:") && message.contains("date"),
        "the server's refusal, naming the key: {cli_error}"
    );
}
