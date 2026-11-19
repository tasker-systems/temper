#![cfg(feature = "test-db")]
//! A server `BAD_REQUEST` reaches the CLI caller as `bad-request`, with the server's sentence.
//!
//! `temper-client` types a readable `BAD_REQUEST` body as `ClientError::BadRequest`, and the CLI's
//! one lifter (`client_err_to_temper`) lifts it to `TemperError::BadRequest`. Before, every such
//! 400 fell to `ClientError::Server { status: 400 }` and the CLI reported a caller-fixable refusal
//! as the generic `api`.
//!
//! The refusal driven here is one only the server makes: a scalar on the `open_meta_add` channel,
//! refused by `DbBackend`'s `open_meta_add_properties`. The witness compares the CLI's JSON error
//! payload with the HTTP door's answer to the same PATCH.

mod common;

use serde_json::{json, Value};

fn stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not one JSON document ({e}): stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_server_bad_request_reaches_the_cli_as_bad_request_with_the_servers_sentence(
    pool: sqlx::PgPool,
) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight");
    app.client
        .contexts()
        .create("bad-request-kind-ctx", None)
        .await
        .expect("ctx create");

    let created = common::run_temper_cli(
        &app,
        &[
            "--format",
            "json",
            "resource",
            "create",
            "--type",
            "concept",
            "--title",
            "Bad Request Kind Probe",
            "--context",
            "@me/bad-request-kind-ctx",
            "--open-meta",
            r#"{"tags":["alpha"]}"#,
        ],
    )
    .await
    .expect("spawn the temper binary");
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let id = stdout_json(&created)["id"]
        .as_str()
        .expect("id")
        .to_string();

    // The HTTP door's answer to the same PATCH: the kind and sentence the CLI must relay.
    let res = reqwest::Client::new()
        .patch(app.url(&format!("/api/resources/{id}")))
        .bearer_auth(&app.token)
        .json(&json!({"open_meta_add": {"tags": "beta"}}))
        .send()
        .await
        .expect("PATCH");
    assert_eq!(res.status().as_u16(), 400);
    let http: Value = res.json().await.expect("error body is JSON");
    assert_eq!(http["error"]["code"], "BAD_REQUEST", "{http}");
    let http_message = http["error"]["message"].as_str().expect("http message");

    let refused = common::run_temper_cli(
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
    .await
    .expect("spawn the temper binary");

    assert_eq!(refused.status.code(), Some(1), "a refusal exits 1");
    let payload = stdout_json(&refused);
    assert_eq!(
        payload["code"], "bad-request",
        "a caller-fixable refusal keeps its kind: {payload}"
    );
    assert_eq!(
        payload["message"], http_message,
        "the CLI relays the server's sentence unchanged: {payload}"
    );
}
