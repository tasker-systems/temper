//! `temper resource create` sends one idempotency key per invocation, the caller can choose it, and
//! a network error names it.
//!
//! The server replays a create by `(owner, idempotency_key)` (issue #581), so a retry that sends
//! the first attempt's key returns the first write. A retry is a new invocation, so the key has to
//! outlive the process: `--idempotency-key` supplies it, and the network-error hint prints the key
//! that was sent. These tests drive the real binary against a mock API, so they see the bytes on
//! the wire and the process's own output. That a replayed key returns one resource is the server's
//! half, witnessed end to end in `tests/e2e/tests/create_idempotency_key_e2e.rs`.

use std::process::{Command, Output, Stdio};

use tempfile::TempDir;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

/// A JWT-shaped bearer (`{"alg":"HS256"}.{"sub":"idempotency-probe","exp":4102444800}.sig`): the
/// CLI checks a token's shape and expiry before sending it, and the mock API never verifies it.
const TEST_TOKEN: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.\
eyJzdWIiOiJpZGVtcG90ZW5jeS1wcm9iZSIsImV4cCI6NDEwMjQ0NDgwMH0.c2ln";

/// A global config pointing the CLI at `api_url`, with its vault inside `dir`.
fn write_config(dir: &TempDir, api_url: &str) -> std::path::PathBuf {
    let config_path = dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        format!(
            "[vault]\npath = \"{vault}\"\n\n[cloud]\napi_url = \"{api_url}\"\n",
            vault = dir.path().join("vault").display(),
        ),
    )
    .expect("write config");
    config_path
}

/// Run `temper --format json resource create …` against `api_url`, plus `extra` args.
fn run_create(dir: &TempDir, api_url: &str, extra: &[&str]) -> Output {
    let config_path = write_config(dir, api_url);
    let mut args = vec![
        "--format",
        "json",
        "resource",
        "create",
        "--type",
        "research",
        "--title",
        "Idempotency Key Probe",
        "--context",
        "@me/probe",
    ];
    args.extend_from_slice(extra);
    Command::new(env!("CARGO_BIN_EXE_temper"))
        .env("TEMPER_GLOBAL_CONFIG", &config_path)
        .env("TEMPER_API_URL", api_url)
        .env("TEMPER_TOKEN", TEST_TOKEN)
        .env("HOME", dir.path())
        .env_remove("TEMPER_AUTH")
        .env_remove("TEMPER_FORMAT")
        // No body: an empty stdin is no body, so nothing is embedded before the send.
        .stdin(Stdio::null())
        .args(&args)
        .output()
        .expect("run the temper binary")
}

/// The key each `POST /api/ingest` carried, in its body and in its `Idempotency-Key` header.
fn sent_keys(requests: &[Request]) -> Vec<(Option<String>, Option<String>)> {
    requests
        .iter()
        .filter(|r| r.method.as_str() == "POST" && r.url.path() == "/api/ingest")
        .map(|r| {
            let body: serde_json::Value =
                serde_json::from_slice(&r.body).expect("ingest body is JSON");
            let in_body = body["idempotency_key"].as_str().map(str::to_owned);
            let in_header = r
                .headers
                .get("idempotency-key")
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            (in_body, in_header)
        })
        .collect()
}

fn stdout_json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not one JSON document ({e}): stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// A mock API whose create always answers `503`: the client retries it, so every attempt the
/// invocation makes reaches the recorder.
fn mock_failing_ingest(rt: &tokio::runtime::Runtime) -> MockServer {
    rt.block_on(async {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/ingest"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        server
    })
}

/// FAILS IF `--idempotency-key` is not what the create sends, in the body the server dedups on
/// and the header the proxy reads, or if an in-process retry of the same invocation sends any
/// other key.
#[test]
fn the_given_key_is_sent_on_every_attempt_of_one_invocation() {
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let server = mock_failing_ingest(&rt);
    let dir = TempDir::new().expect("tempdir");
    let key = Uuid::now_v7().to_string();

    let output = run_create(&dir, &server.uri(), &["--idempotency-key", &key]);
    assert!(!output.status.success(), "a 503 create fails");

    let requests = rt
        .block_on(server.received_requests())
        .expect("recording is on");
    let sent = sent_keys(&requests);
    assert!(
        sent.len() > 1,
        "the keyed create is retried in process: {sent:?}"
    );
    for (in_body, in_header) in &sent {
        assert_eq!(in_body.as_deref(), Some(key.as_str()), "{sent:?}");
        assert_eq!(in_header.as_deref(), Some(key.as_str()), "{sent:?}");
    }
}

/// FAILS IF a create given no key sends none, or sends a different minted key on each in-process
/// retry, or if two invocations share a key (which would make two creates one).
#[test]
fn an_unkeyed_invocation_mints_one_key_and_keeps_it_across_its_retries() {
    let rt = tokio::runtime::Runtime::new().expect("test runtime");
    let server = mock_failing_ingest(&rt);
    let dir = TempDir::new().expect("tempdir");

    let mut per_invocation = Vec::new();
    for _ in 0..2 {
        rt.block_on(server.reset());
        rt.block_on(
            Mock::given(method("POST"))
                .and(path("/api/ingest"))
                .respond_with(ResponseTemplate::new(503))
                .mount(&server),
        );
        let _ = run_create(&dir, &server.uri(), &[]);
        let requests = rt
            .block_on(server.received_requests())
            .expect("recording is on");
        let sent = sent_keys(&requests);
        assert!(sent.len() > 1, "{sent:?}");
        let first = sent[0].0.clone().expect("a minted key in the body");
        Uuid::parse_str(&first).expect("the minted key is a UUID");
        for (in_body, in_header) in &sent {
            assert_eq!(in_body.as_deref(), Some(first.as_str()), "{sent:?}");
            assert_eq!(in_header.as_deref(), Some(first.as_str()), "{sent:?}");
        }
        per_invocation.push(first);
    }
    assert_ne!(
        per_invocation[0], per_invocation[1],
        "each invocation mints its own key"
    );
}

/// The one port nothing listens on: bind an ephemeral port, then release it.
fn closed_port_url() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    drop(listener);
    format!("http://127.0.0.1:{port}")
}

/// FAILS IF a create that fails with a network error does not name, in its JSON error payload,
/// the key it sent — the given one when the caller gave one, else the one it minted — in the
/// flag a retry passes back.
#[test]
fn a_network_error_names_the_key_the_create_sent() {
    let dir = TempDir::new().expect("tempdir");
    let api = closed_port_url();

    let key = Uuid::now_v7().to_string();
    let given = stdout_json(&run_create(&dir, &api, &["--idempotency-key", &key]));
    assert_eq!(given["code"], "network", "{given}");
    let hint = given["hint"].as_str().expect("a hint");
    assert!(hint.contains(&format!("--idempotency-key {key}")), "{hint}");

    let minted = stdout_json(&run_create(&dir, &api, &[]));
    assert_eq!(minted["code"], "network", "{minted}");
    let hint = minted["hint"].as_str().expect("a hint");
    let named = hint
        .split("--idempotency-key ")
        .nth(1)
        .and_then(|rest| rest.get(..36))
        .expect("the hint names a key after the flag");
    Uuid::parse_str(named).expect("the named key is a UUID");
    assert_ne!(named, key, "an unkeyed create names its own minted key");
}
