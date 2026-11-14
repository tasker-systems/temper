#![cfg(feature = "test-db")]
//! Conditional meta writes through the MCP and CLI doors.
//!
//! The mechanism and its HTTP door are witnessed in `temper-services`' `meta_precondition_test.rs`
//! and `temper-api`'s `meta_precondition_http_test.rs`. This suite witnesses that each door above
//! them carries the guard end to end and renders the refusal for its reader:
//!
//! - **MCP** (`update_resource_meta`, `update_resource`): driven through the deployed relay against
//!   this process's real listener, exactly as production dispatches a tool call. The refusal is a
//!   tool error whose `data` is the HTTP door's `error` object.
//! - **The CLI** (`resource meta set`, `resource update`): the real `temper` binary. Build it first
//!   (`cargo build -p temper-cli --bin temper`), or the run tests whatever binary is on disk.
//!
//! The race is witnessed here once more, through MCP, and so is the relay's retry: a write whose
//! answer is lost after it committed is replayed by the relay and meets its own value as `current`.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use sqlx::PgPool;
use temper_mcp::service::TemperMcpService;
use uuid::Uuid;

use common::E2eTestApp;

const EMAIL: &str = "e2e@test.example.com";

// ── harness ──────────────────────────────────────────────────────────────────────────────────────

async fn harness(pool: PgPool) -> (E2eTestApp, TemperMcpService, axum::http::request::Parts) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service().await;
    let parts = app.relay_parts();
    (app, svc, parts)
}

fn input<T: for<'de> serde::Deserialize<'de>>(value: Value) -> T {
    serde_json::from_value(value).expect("input deserializes from its wire shape")
}

fn one_text(res: &rmcp::model::CallToolResult) -> Value {
    assert_eq!(res.content.len(), 1, "one content part");
    serde_json::from_str(res.content[0].as_text().expect("a text part").text.as_str())
        .expect("the part is the tool's JSON response")
}

async fn profile_id(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_profiles WHERE email = $1")
        .bind(email)
        .fetch_one(pool)
        .await
        .expect("the profile")
}

async fn default_context_id(pool: &PgPool, email: &str) -> Uuid {
    sqlx::query_scalar(
        "SELECT c.id FROM kb_contexts c JOIN kb_profiles p ON p.id = c.owner_id \
          WHERE p.email = $1 AND c.name = 'default'",
    )
    .bind(email)
    .fetch_one(pool)
    .await
    .expect("the auto-provisioned default context")
}

/// A task in the harness principal's default context, created through the MCP door.
async fn create_task(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    pool: &PgPool,
) -> String {
    let ctx = default_context_id(pool, EMAIL).await;
    let created = one_text(
        &temper_mcp::tools::resources::create_resource(
            svc,
            parts,
            input(json!({
                "context_ref": ctx.to_string(),
                "doc_type_name": "task",
                "title": "Claimable",
            })),
        )
        .await
        .expect("create lands"),
    );
    created["resource"]["id"]
        .as_str()
        .expect("an id")
        .to_string()
}

/// A second approved identity, warmed through the listener, and granted read and write on the
/// harness principal's default context: the second owner of a claim race.
async fn second_writer(app: &E2eTestApp) -> (axum::http::request::Parts, Uuid) {
    let unique = Uuid::new_v4();
    let email = format!("claimant-{unique}@example.com");
    let token = common::generate_test_jwt(&format!("claimant-sub-{unique}"), &email);
    let _ = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .bearer_auth(&token)
        .send()
        .await;
    sqlx::query(
        "INSERT INTO kb_principal_standing (profile_id, state)
         SELECT id, 'approved' FROM kb_profiles WHERE email = $1
         ON CONFLICT (profile_id) DO UPDATE SET state = 'approved', updated = now()",
    )
    .bind(&email)
    .execute(&app.pool)
    .await
    .expect("approve the second identity");
    let grantee = profile_id(&app.pool, &email).await;
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, \
                                       can_read, can_write, granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, true, $3)",
    )
    .bind(default_context_id(&app.pool, EMAIL).await)
    .bind(grantee)
    .bind(profile_id(&app.pool, EMAIL).await)
    .execute(&app.pool)
    .await
    .expect("grant context write");
    (app.relay_parts_for(&token), grantee)
}

/// Claim `claim_id` for `value`, expecting it absent.
async fn claim(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    id: &str,
    value: &str,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::resources::update_resource_meta(
        svc,
        parts,
        input(json!({
            "id": id,
            "managed_meta": {},
            "open_meta": {"claim_id": value},
            "expect": {"open": {"claim_id": null}},
        })),
    )
    .await
}

/// The precondition a refused MCP meta write carries, after asserting it is the typed caller error
/// (never `internal_error`) whose message names keys and no value.
fn refusal(err: &rmcp::ErrorData, never_in_message: &[&str]) -> Value {
    assert_eq!(
        err.code.0, -32602,
        "a caller error, never internal: {err:?}"
    );
    for value in never_in_message {
        assert!(
            !err.message.contains(value),
            "the message carries a value ({value}): {}",
            err.message
        );
    }
    let data = err.data.as_ref().expect("the refusal rides data");
    assert_eq!(data["code"], "PRECONDITION_FAILED", "{data}");
    data["precondition"].clone()
}

async fn stored_claim(pool: &PgPool, id: &str) -> Value {
    sqlx::query_scalar(
        "SELECT property_value FROM kb_properties \
          WHERE owner_table = 'kb_resources' AND owner_id = $1::uuid \
            AND property_key = 'claim_id' AND NOT is_folded \
          ORDER BY created DESC, id DESC LIMIT 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .expect("read the claim")
    .unwrap_or(Value::Null)
}

/// Hold R's row `FOR NO KEY UPDATE`, the lock every writer of R takes, on a side transaction.
async fn hold_writer_lock(pool: &PgPool, id: &str) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut held = pool.begin().await.expect("begin");
    sqlx::query("SELECT 1 FROM kb_resources WHERE id = $1::uuid FOR NO KEY UPDATE")
        .bind(id)
        .execute(&mut *held)
        .await
        .expect("take the writer lock");
    held
}

/// Poll until `n` backends wait on R's writer lock, a row-lock wait while running
/// `lock_resource_for_write`'s statement; fail the moment one waits on an advisory lock instead.
/// Ported from `temper-services`' witness of the same race.
async fn writers_wait_on_the_row_lock(pool: &PgPool, n: i64) {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let advisory: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
              WHERE datname = current_database() AND wait_event_type = 'Lock' \
                AND wait_event = 'advisory'",
        )
        .fetch_one(pool)
        .await
        .expect("poll advisory waits");
        assert_eq!(advisory, 0, "a writer waited on an advisory lock");
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
              WHERE datname = current_database() AND wait_event_type = 'Lock' \
                AND wait_event IN ('tuple', 'transactionid') \
                AND query ILIKE '%FROM kb_resources WHERE id = $1 FOR NO KEY UPDATE%'",
        )
        .fetch_one(pool)
        .await
        .expect("poll row-lock waits");
        if waiting >= n {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{waiting} of {n} writers waited on the writer lock within 4 seconds"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

// ── MCP ──────────────────────────────────────────────────────────────────────────────────────────

/// **`update_resource_meta` carries `expect` end to end.** A guard that holds lands and answers
/// `{updated, id, view}` with the claim in the view; a guard that fails writes nothing and is a
/// caller error whose `data` lists the key with the value there now, and whose message carries no
/// value. FAILS IF the tool drops `expect` (the second claim lands), renders the refusal as
/// `internal_error` or without `data`, or the view is not the write's own answer.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_resource_meta_carries_the_guard_and_renders_the_refusal(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let id = create_task(&svc, &parts, &app.pool).await;

    let ack = one_text(&claim(&svc, &parts, &id, "claim-a").await.expect("lands"));
    assert_eq!(ack["updated"], json!(true));
    assert_eq!(ack["id"], json!(id));
    assert_eq!(ack["view"]["open_meta"]["claim_id"], "claim-a", "{ack}");

    let err = claim(&svc, &parts, &id, "claim-b")
        .await
        .expect_err("a stale guard is refused");
    let precondition = refusal(&err, &["claim-a", "claim-b"]);
    assert_eq!(
        precondition,
        json!({"failed": [
            {"tier": "open", "key": "claim_id", "expected": null, "current": "claim-a"}
        ]})
    );
    assert_eq!(stored_claim(&app.pool, &id).await, json!("claim-a"));
}

/// **`update_resource` carries `expect` end to end**, on either tier. A wrong expected value is
/// refused with nothing written (the title included); the right one lands with the rest of the
/// update. FAILS IF the tool drops `expect`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_resource_carries_the_guard(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let id = create_task(&svc, &parts, &app.pool).await;
    claim(&svc, &parts, &id, "claim-a").await.expect("lands");

    let err = temper_mcp::tools::resources::update_resource(
        &svc,
        &parts,
        input(json!({
            "id": id,
            "title": "Renamed",
            "managed_meta": {"temper-stage": "in-progress"},
            "expect": {"open": {"claim_id": "claim-z"}, "managed": {"temper-stage": "done"}},
        })),
    )
    .await
    .expect_err("a stale guard is refused");
    let precondition = refusal(&err, &["claim-a", "claim-z"]);
    let failed = precondition["failed"].as_array().expect("entries");
    assert_eq!(failed.len(), 2, "{precondition}");
    assert!(failed.contains(&json!(
        {"tier": "open", "key": "claim_id", "expected": "claim-z", "current": "claim-a"}
    )));
    let managed = failed
        .iter()
        .find(|f| f["tier"] == "managed")
        .expect("the managed entry");
    assert_eq!(managed["key"], "temper-stage", "{precondition}");
    assert_eq!(managed["expected"], "done", "{precondition}");
    assert_ne!(managed["current"], "done", "{precondition}");
    let after = one_text(
        &temper_mcp::tools::resources::get_resource(&svc, &parts, input(json!({ "id": id })))
            .await
            .expect("get lands"),
    );
    assert_eq!(after["title"], "Claimable", "nothing was written: {after}");

    let landed = one_text(
        &temper_mcp::tools::resources::update_resource(
            &svc,
            &parts,
            input(json!({
                "id": id,
                "title": "Renamed",
                "expect": {"open": {"claim_id": "claim-a"}},
            })),
        )
        .await
        .expect("a holding guard lands"),
    );
    assert_eq!(landed["title"], "Renamed", "{landed}");
}

/// **Two concurrent claimants through MCP, two owners: exactly one lands.** A side transaction
/// holds R's writer lock; both claimants' tool calls are seen waiting on it before release. One
/// lands; the other's tool error carries `current` equal to the winner's claim.
///
/// FAILS IF a door drops the guard: both claims then land, and the later overwrites the earlier.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn two_claimants_through_mcp_exactly_one_lands(pool: PgPool) {
    let (app, svc, owner) = harness(pool).await;
    let id = create_task(&svc, &owner, &app.pool).await;
    let (other, _) = second_writer(&app).await;
    let svc = Arc::new(svc);

    let held = hold_writer_lock(&app.pool, &id).await;
    let spawn_claim = |parts: axum::http::request::Parts, value: &'static str| {
        let svc = svc.clone();
        let id = id.clone();
        tokio::spawn(async move { (value, claim(&svc, &parts, &id, value).await) })
    };
    let a = spawn_claim(owner, "claim-owner");
    let b = spawn_claim(other, "claim-other");
    writers_wait_on_the_row_lock(&app.pool, 2).await;
    held.commit().await.expect("release the writer lock");
    let answers = [a.await.unwrap(), b.await.unwrap()];

    let landed: Vec<&str> = answers
        .iter()
        .filter(|(_, r)| r.is_ok())
        .map(|(v, _)| *v)
        .collect();
    assert_eq!(landed.len(), 1, "exactly one claim lands: {answers:?}");
    let winner = landed[0];
    let (loser, refused) = answers.iter().find(|(v, _)| *v != winner).unwrap();
    let err = refused.as_ref().expect_err("the other is refused");
    let precondition = refusal(err, &[winner, loser]);
    assert_eq!(
        precondition["failed"],
        json!([{"tier": "open", "key": "claim_id", "expected": null, "current": winner}])
    );
    assert_eq!(stored_claim(&app.pool, &id).await, json!(winner));
}

/// A forwarding proxy in front of the API that loses the answer to the first `PUT /meta` after
/// the API answered it (the write committed), and answers the relay `502` instead.
struct LossyProxy {
    upstream: String,
    http: reqwest::Client,
    meta_puts: AtomicUsize,
}

async fn forward(
    axum::extract::State(proxy): axum::extract::State<Arc<LossyProxy>>,
    req: axum::extract::Request,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let (head, body) = req.into_parts();
    let body = axum::body::to_bytes(body, usize::MAX)
        .await
        .expect("read the body");
    let path = head
        .uri
        .path_and_query()
        .map(|p| p.as_str().to_string())
        .unwrap_or_default();
    let mut upstream = proxy
        .http
        .request(head.method.clone(), format!("{}{}", proxy.upstream, path));
    for (name, value) in &head.headers {
        if !matches!(name.as_str(), "host" | "content-length" | "connection") {
            upstream = upstream.header(name, value);
        }
    }
    let answer = upstream.body(body).send().await.expect("the API answers");
    let lose_this_one = head.method == axum::http::Method::PUT
        && head.uri.path().ends_with("/meta")
        && proxy.meta_puts.fetch_add(1, Ordering::SeqCst) == 0;
    let status = answer.status();
    let headers = answer.headers().clone();
    let bytes = answer.bytes().await.expect("the API's body");
    if lose_this_one {
        assert!(status.is_success(), "the first attempt committed: {status}");
        return (
            axum::http::StatusCode::BAD_GATEWAY,
            r#"{"error":{"code":"BAD_GATEWAY","message":"the upstream answer was lost"}}"#,
        )
            .into_response();
    }
    let mut response = (status, bytes).into_response();
    for (name, value) in &headers {
        if !matches!(
            name.as_str(),
            "content-length" | "transfer-encoding" | "connection"
        ) {
            response.headers_mut().insert(name.clone(), value.clone());
        }
    }
    response
}

/// **A relay retry after commit meets its own value.** The first attempt of a guarded claim
/// commits, its answer is lost, and the relay replays it once (its one retry on a 5xx). The
/// replay is refused `412`, and its `current` is the caller's own claim: the signal that its
/// earlier attempt landed. Exactly two `PUT /meta` reached the API, and the claim is stored once.
///
/// FAILS IF the relay does not replay (one PUT, and the caller sees an opaque server error), or the
/// replay lands a second write (the guard was dropped on a door).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_relay_retry_after_commit_meets_its_own_value(pool: PgPool) {
    let (app, direct, parts) = harness(pool).await;
    let id = create_task(&direct, &parts, &app.pool).await;

    let proxy = Arc::new(LossyProxy {
        upstream: app.base_url(),
        http: reqwest::Client::new(),
        meta_puts: AtomicUsize::new(0),
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the proxy");
    let proxy_url = format!("http://{}", listener.local_addr().unwrap());
    let router = axum::Router::new()
        .fallback(forward)
        .with_state(proxy.clone());
    let serve = tokio::spawn(async move { axum::serve(listener, router).await.ok() });
    let lossy = temper_mcp_server::tool_service(
        temper_mcp_server::config::blob_door(None, false),
        common::deployed_relay_to(&proxy_url),
    );

    let events_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties WHERE owner_id = $1::uuid AND property_key = 'claim_id'",
    )
    .bind(&id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    let err = claim(&lossy, &parts, &id, "claim-mine")
        .await
        .expect_err("the replay meets the claim its first attempt landed");
    serve.abort();

    assert_eq!(
        proxy.meta_puts.load(Ordering::SeqCst),
        2,
        "the relay replayed the write exactly once"
    );
    let precondition = refusal(&err, &["claim-mine"]);
    assert_eq!(
        precondition["failed"],
        json!([{"tier": "open", "key": "claim_id", "expected": null, "current": "claim-mine"}])
    );
    assert_eq!(stored_claim(&app.pool, &id).await, json!("claim-mine"));
    let rows_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_properties WHERE owner_id = $1::uuid AND property_key = 'claim_id'",
    )
    .bind(&id)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(rows_after, events_before + 1, "the claim was written once");
}

// ── CLI ──────────────────────────────────────────────────────────────────────────────────────────

async fn cli(app: &E2eTestApp, args: &[&str]) -> std::process::Output {
    common::run_temper_cli(app, args).await.expect("cli run")
}

fn stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is one JSON document ({e}): stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// **`resource meta set` carries `--expect-absent` and `--expect`, and a refusal exits 1 with the
/// payload a JSON caller branches on**: code `PRECONDITION_FAILED`, a key-only message, and each
/// failed key's current value in `details.failed`. With `--format toon` the stderr prose names
/// the key with what was expected and what is there now. FAILS IF a flag is dropped (the second
/// claim lands), the lifter flattens the refusal (code `api`, no `details`), or the exit is 0.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cli_meta_set_carries_the_guard_and_exits_non_zero_on_refusal(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let id = create_task(&svc, &parts, &app.pool).await;

    let first = cli(
        &app,
        &[
            "--format",
            "json",
            "resource",
            "meta",
            "set",
            &id,
            "--managed",
            "{}",
            "--open",
            r#"{"claim_id":"claim-a"}"#,
            "--expect-absent",
            "open:claim_id",
        ],
    )
    .await;
    assert!(
        first.status.success(),
        "a holding guard lands: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert_eq!(stdout_json(&first)["open_meta"]["claim_id"], "claim-a");

    let refused = cli(
        &app,
        &[
            "--format",
            "json",
            "resource",
            "meta",
            "set",
            &id,
            "--managed",
            "{}",
            "--open",
            r#"{"claim_id":"claim-b"}"#,
            "--expect",
            r#"{"open":{"claim_id":null}}"#,
        ],
    )
    .await;
    assert_eq!(refused.status.code(), Some(1), "a refusal exits 1");
    let payload = stdout_json(&refused);
    assert_eq!(payload["code"], "PRECONDITION_FAILED", "{payload}");
    assert_eq!(payload["message"], "1 precondition failed: open.claim_id");
    assert_eq!(
        payload["details"],
        json!({"failed": [
            {"tier": "open", "key": "claim_id", "expected": null, "current": "claim-a"}
        ]})
    );
    assert_eq!(stored_claim(&app.pool, &id).await, json!("claim-a"));

    // A piped stdout defaults to JSON, so the human rendering is asked for by name.
    let prose = cli(
        &app,
        &[
            "--format",
            "toon",
            "resource",
            "meta",
            "set",
            &id,
            "--managed",
            "{}",
            "--open",
            r#"{"claim_id":"claim-b"}"#,
            "--expect-absent",
            "open:claim_id",
        ],
    )
    .await;
    assert_eq!(prose.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&prose.stderr);
    assert!(
        stderr.contains("open.claim_id: expected absent, now \"claim-a\""),
        "{stderr}"
    );
}

/// **`resource update` carries `--expect`**: a wrong expected value is refused (exit 1, code
/// `PRECONDITION_FAILED`, nothing written); the right one lands. FAILS IF the flag is dropped
/// between the command and the wire.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cli_update_carries_the_guard(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let id = create_task(&svc, &parts, &app.pool).await;
    claim(&svc, &parts, &id, "claim-a").await.expect("lands");

    let refused = cli(
        &app,
        &[
            "--format",
            "json",
            "resource",
            "update",
            &id,
            "--open-meta",
            r#"{"claim_id":"claim-b"}"#,
            "--expect",
            r#"{"open":{"claim_id":"claim-z"}}"#,
        ],
    )
    .await;
    assert_eq!(refused.status.code(), Some(1));
    let payload = stdout_json(&refused);
    assert_eq!(payload["code"], "PRECONDITION_FAILED", "{payload}");
    assert_eq!(payload["details"]["failed"][0]["current"], "claim-a");
    assert_eq!(stored_claim(&app.pool, &id).await, json!("claim-a"));

    let landed = cli(
        &app,
        &[
            "--format",
            "json",
            "resource",
            "update",
            &id,
            "--open-meta",
            r#"{"claim_id":"claim-b"}"#,
            "--expect",
            r#"{"open":{"claim_id":"claim-a"}}"#,
        ],
    )
    .await;
    assert!(
        landed.status.success(),
        "a holding guard lands: {}",
        String::from_utf8_lossy(&landed.stderr)
    );
    assert_eq!(stored_claim(&app.pool, &id).await, json!("claim-b"));
}
