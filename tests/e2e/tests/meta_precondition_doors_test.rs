#![cfg(feature = "test-db")]
//! Conditional meta writes through the MCP and CLI doors.
//!
//! The mechanism and its HTTP door are witnessed in `temper-services`' `meta_precondition_test.rs`
//! and `temper-api`'s `meta_precondition_http_test.rs`. This suite witnesses that each door above
//! them carries the guard end to end and renders the refusal for its reader:
//!
//! - **MCP** (`update_resource_meta`, `update_resource`): driven through the deployed relay against
//!   this process's real listener, exactly as production dispatches a tool call. The refusal is a
//!   tool error result (`isError: true`) whose `structuredContent` is the HTTP door's `error`
//!   object.
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

/// True when the tool call landed: a result that is not an error result.
fn landed(answer: &Result<rmcp::model::CallToolResult, rmcp::ErrorData>) -> bool {
    matches!(answer, Ok(r) if r.is_error != Some(true))
}

/// The precondition a refused MCP meta write carries, after asserting the refusal's shape: a
/// tool-execution error result (`isError: true`, never a JSON-RPC error, which rmcp would log with
/// its data), whose one text part names keys and no value, and whose `structuredContent` is the
/// HTTP door's `error` object and nothing more. Every entry is `{tier, key, expected, current}`:
/// no actor, time or id is ever added.
fn refusal(
    answer: &Result<rmcp::model::CallToolResult, rmcp::ErrorData>,
    never_in_text: &[&str],
) -> Value {
    let result = match answer {
        Ok(r) => r,
        Err(e) => panic!("a refusal is a tool result, never a JSON-RPC error: {e:?}"),
    };
    assert_eq!(result.is_error, Some(true), "an error result: {result:?}");
    assert_eq!(result.content.len(), 1, "one text part");
    let text = &result.content[0].as_text().expect("a text part").text;
    for value in never_in_text {
        assert!(
            !text.contains(value),
            "the text carries a value ({value}): {text}"
        );
    }
    let data = result
        .structured_content
        .as_ref()
        .expect("the refusal rides structuredContent");
    let mut members: Vec<&str> = data
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    members.sort_unstable();
    assert_eq!(members, ["code", "message", "precondition"], "{data}");
    assert_eq!(data["code"], "PRECONDITION_FAILED", "{data}");
    for value in never_in_text {
        assert!(
            !data["message"].as_str().unwrap_or_default().contains(value),
            "the message carries a value ({value}): {data}"
        );
    }
    let precondition = data["precondition"].clone();
    let mut outer: Vec<&str> = precondition
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    outer.sort_unstable();
    assert_eq!(outer, ["failed"], "{precondition}");
    for entry in precondition["failed"].as_array().expect("entries") {
        let mut keys: Vec<&str> = entry
            .as_object()
            .expect("an entry")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["current", "expected", "key", "tier"], "{entry}");
    }
    precondition
}

async fn stored_claim(pool: &PgPool, id: &str) -> Value {
    stored(pool, id, "claim_id").await
}

/// `key`'s newest live value on R, `null` when it has none.
async fn stored(pool: &PgPool, id: &str, key: &str) -> Value {
    sqlx::query_scalar(
        "SELECT property_value FROM kb_properties \
          WHERE owner_table = 'kb_resources' AND owner_id = $1::uuid \
            AND property_key = $2 AND NOT is_folded \
          ORDER BY created DESC, id DESC LIMIT 1",
    )
    .bind(id)
    .bind(key)
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
/// tool error result whose `structuredContent` lists the key with the value there now, and whose
/// text carries no value. FAILS IF the tool drops `expect` (the second claim lands), renders the
/// refusal as a JSON-RPC error or without `structuredContent`, or the view is not the write's own
/// answer.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn update_resource_meta_carries_the_guard_and_renders_the_refusal(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let id = create_task(&svc, &parts, &app.pool).await;

    let first = claim(&svc, &parts, &id, "claim-a").await;
    assert!(landed(&first), "{first:?}");
    let ack = one_text(first.as_ref().unwrap());
    assert_eq!(ack["updated"], json!(true));
    assert_eq!(ack["id"], json!(id));
    assert_eq!(ack["view"]["open_meta"]["claim_id"], "claim-a", "{ack}");

    let refused = claim(&svc, &parts, &id, "claim-b").await;
    let precondition = refusal(&refused, &["claim-a", "claim-b"]);
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
    assert!(landed(&claim(&svc, &parts, &id, "claim-a").await));

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
    .await;
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
        .filter(|(_, r)| landed(r))
        .map(|(v, _)| *v)
        .collect();
    assert_eq!(landed.len(), 1, "exactly one claim lands: {answers:?}");
    let winner = landed[0];
    let (loser, refused) = answers.iter().find(|(v, _)| *v != winner).unwrap();
    let precondition = refusal(refused, &[winner, loser]);
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
    // The replay meets the claim its first attempt landed.
    let err = claim(&lossy, &parts, &id, "claim-mine").await;
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

/// A write of `open` to R through `update_resource_meta`, guarded by `expect` when given.
async fn meta_write(
    svc: &TemperMcpService,
    parts: &axum::http::request::Parts,
    id: &str,
    open: Value,
    expect: Option<Value>,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    let mut args = json!({"id": id, "managed_meta": {}, "open_meta": open});
    if let Some(expect) = expect {
        args["expect"] = expect;
    }
    temper_mcp::tools::resources::update_resource_meta(svc, parts, input(args)).await
}

/// **All-or-nothing through MCP.** Three keys written, two guarded, one guard failing: nothing
/// lands, not even the keys whose guard held or that were not guarded, and the refusal names only
/// the failed key. FAILS IF the door applies part of the write.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn all_or_nothing_through_mcp(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let id = create_task(&svc, &parts, &app.pool).await;
    assert!(landed(
        &meta_write(&svc, &parts, &id, json!({"a": "1", "b": "2"}), None).await
    ));

    let refused = meta_write(
        &svc,
        &parts,
        &id,
        json!({"a": "10", "b": "20", "c": "30"}),
        Some(json!({"open": {"a": "1", "b": "wrong"}})),
    )
    .await;
    let precondition = refusal(&refused, &["10", "20", "30", "wrong"]);
    assert_eq!(
        precondition["failed"],
        json!([{"tier": "open", "key": "b", "expected": "wrong", "current": "2"}])
    );
    assert_eq!(stored(&app.pool, &id, "a").await, json!("1"));
    assert_eq!(stored(&app.pool, &id, "b").await, json!("2"));
    assert_eq!(stored(&app.pool, &id, "c").await, Value::Null);
}

/// Every event type recorded after the first `after` events, in order.
async fn event_types_since(pool: &PgPool, after: i64) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT t.name FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          ORDER BY e.id OFFSET $1",
    )
    .bind(after)
    .fetch_all(pool)
    .await
    .expect("event types")
}

async fn event_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_events")
        .fetch_one(pool)
        .await
        .expect("count events")
}

/// **A landed conditional write is ordinary through MCP**: the same events, and the same meta in
/// the answer's view, as the same write without a guard on a twin resource. FAILS IF the guarded
/// path records anything more or less, or answers differently.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_landed_conditional_write_is_ordinary_through_mcp(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let guarded = create_task(&svc, &parts, &app.pool).await;
    let plain = create_task(&svc, &parts, &app.pool).await;
    for id in [&guarded, &plain] {
        assert!(landed(
            &meta_write(&svc, &parts, id, json!({"lease": "mine"}), None).await
        ));
    }

    let n = event_count(&app.pool).await;
    let a = meta_write(
        &svc,
        &parts,
        &guarded,
        json!({"lease": "next", "gone": null}),
        Some(json!({"open": {"lease": "mine"}})),
    )
    .await;
    assert!(landed(&a), "{a:?}");
    let guarded_events = event_types_since(&app.pool, n).await;
    let n = event_count(&app.pool).await;
    let b = meta_write(
        &svc,
        &parts,
        &plain,
        json!({"lease": "next", "gone": null}),
        None,
    )
    .await;
    assert!(landed(&b), "{b:?}");
    assert!(!guarded_events.is_empty());
    assert_eq!(
        event_types_since(&app.pool, n).await,
        guarded_events,
        "the same events"
    );
    let (a, b) = (one_text(&a.unwrap()), one_text(&b.unwrap()));
    assert_eq!(
        a["view"]["open_meta"], b["view"]["open_meta"],
        "the same answer"
    );
    assert_eq!(a["view"]["managed_meta"], b["view"]["managed_meta"]);
    assert_eq!(
        a.as_object().map(|o| o.len()),
        b.as_object().map(|o| o.len())
    );
}

/// A fresh approved system admin, warmed through the listener: `(parts, profile id, token)`.
async fn new_admin(app: &E2eTestApp, tag: &str) -> (axum::http::request::Parts, Uuid, String) {
    let unique = Uuid::new_v4();
    let email = format!("{tag}-{unique}@example.com");
    let token = common::generate_test_jwt(&format!("{tag}-sub-{unique}"), &email);
    let _ = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .bearer_auth(&token)
        .send()
        .await;
    let id = profile_id(&app.pool, &email).await;
    common::approved_admin(&app.pool, id).await;
    (app.relay_parts_for(&token), id, token)
}

/// A map in the admin-only regime with a telos owned by the harness principal, and a system admin
/// who may therefore modify the telos but is no member of the gating team and cannot read it.
/// Ported from `temper-services`' fixture of the same standing, which it asserts.
async fn modify_but_not_read(app: &E2eTestApp) -> (axum::http::request::Parts, Uuid, Uuid) {
    let (admin_parts, admin, _) = new_admin(app, "regime-admin").await;
    let owner = profile_id(&app.pool, EMAIL).await;
    let cogmap = Uuid::now_v7();
    let telos = Uuid::now_v7();
    sqlx::query(
        "SELECT cogmap_genesis($1, '{}'::jsonb, \
             (SELECT e.id FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id \
               WHERE p.handle = 'system' AND e.name = 'system'))",
    )
    .bind(json!({
        "cogmap_id": cogmap,
        "name": format!("regime-{cogmap}"),
        "owner_profile_id": owner,
        "telos": {"resource_id": telos, "title": "telos",
                  "origin_uri": format!("temper://test/{cogmap}/telos"), "blocks": []},
    }))
    .execute(&app.pool)
    .await
    .expect("birth the map");
    let team = Uuid::now_v7();
    sqlx::query("INSERT INTO kb_teams (id, slug, name) VALUES ($1, 'regime-gate', 'regime-gate')")
        .bind(team)
        .execute(&app.pool)
        .await
        .expect("seed the gating team");
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(&app.pool)
        .await
        .expect("bind the map");
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'regime-gate' WHERE id = 1")
        .execute(&app.pool)
        .await
        .expect("point the gate at it");
    let (modify, read): (bool, bool) = sqlx::query_as(
        "SELECT can_modify_resource($1, $2), \
                EXISTS (SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2)",
    )
    .bind(admin)
    .bind(telos)
    .fetch_one(&app.pool)
    .await
    .expect("standing");
    assert!(
        modify && !read,
        "the fixture is modify-but-not-read: modify {modify}, read {read}"
    );
    (admin_parts, admin, telos)
}

/// **Read standing first, through MCP.** A caller who may modify a resource it cannot read gets
/// the not-found error `get_resource` gives it, from both meta tools, for a guard that would hold
/// and for one that would fail: never a precondition refusal, never a value, nothing written.
/// FAILS IF a door answers the non-reader with the refusal's `structuredContent`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn read_standing_first_through_mcp(pool: PgPool) {
    let (app, svc, _owner) = harness(pool).await;
    let (admin, _, telos) = modify_but_not_read(&app).await;
    let telos = telos.to_string();

    let get = temper_mcp::tools::resources::get_resource(&svc, &admin, input(json!({"id": telos})))
        .await
        .expect_err("the admin cannot read the telos");
    assert_eq!(get.code.0, -32602, "{get:?}");

    for guard in [
        json!({"open": {"claim": null}}),
        json!({"open": {"claim": "x"}}),
    ] {
        let meta = meta_write(
            &svc,
            &admin,
            &telos,
            json!({"claim": "x"}),
            Some(guard.clone()),
        )
        .await
        .expect_err("update_resource_meta refuses the non-reader");
        let update = temper_mcp::tools::resources::update_resource(
            &svc,
            &admin,
            input(json!({"id": telos, "open_meta": {"claim": "x"}, "expect": guard})),
        )
        .await
        .expect_err("update_resource refuses the non-reader");
        for err in [&meta, &update] {
            assert_eq!(err.code.0, -32602, "{err:?}");
            assert!(
                err.message.to_lowercase().contains("not found"),
                "the not-found face: {err:?}"
            );
            assert!(err.data.is_none(), "no data for a non-reader: {err:?}");
            assert!(!err.message.contains("PRECONDITION"), "{err:?}");
        }
    }
    assert_eq!(stored(&app.pool, &telos, "claim").await, Value::Null);
}

/// **No admin-act disclosure, through MCP.** After an admin clears and scrubs an open key, a guard
/// is judged on exactly what a read shows: guessing the scrubbed value fails with the read's
/// current value, and the removed value appears nowhere in the tool's answer. FAILS IF the door
/// (or what it relays) surfaces anything the act removed.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_scrubbed_value_never_surfaces_through_mcp(pool: PgPool) {
    use temper_core::types::erasure::ScrubFieldKind;

    let (app, svc, parts) = harness(pool).await;
    let id = create_task(&svc, &parts, &app.pool).await;
    assert!(landed(
        &meta_write(&svc, &parts, &id, json!({"note": "hush-hush"}), None).await
    ));

    // The operator scrubs through its own admin door, as production does.
    let (_, _, operator) = new_admin(&app, "operator").await;
    let property = serde_json::to_value(ScrubFieldKind::Property).unwrap();
    let families: Value = app
        .reqwest_client
        .post(app.url("/api/admin/resources/field-scrub/families"))
        .bearer_auth(&operator)
        .json(&json!({"resource": id}))
        .send()
        .await
        .expect("families")
        .json()
        .await
        .expect("families json");
    let mut family = None;
    for f in families["families"]
        .as_array()
        .expect("families: {families}")
    {
        if f["field"] != property {
            continue;
        }
        let event = Uuid::parse_str(
            f["family"]
                .as_str()
                .expect("a property family has a handle"),
        )
        .unwrap();
        let payload: String =
            sqlx::query_scalar("SELECT payload::text FROM kb_events WHERE id = $1")
                .bind(event)
                .fetch_one(&app.pool)
                .await
                .expect("family event");
        if payload.contains("\"note\"") {
            family = Some(event);
        }
    }
    let scrubbed = app
        .reqwest_client
        .post(app.url("/api/admin/resources/field-scrub"))
        .bearer_auth(&operator)
        .json(&json!({
            "resource": id,
            "field": property,
            "family": family.expect("the note family"),
            "clear": true,
        }))
        .send()
        .await
        .expect("scrub");
    assert!(
        scrubbed.status().is_success(),
        "{}: {}",
        scrubbed.status(),
        scrubbed.text().await.unwrap_or_default()
    );

    let read = one_text(
        &temper_mcp::tools::resources::get_resource(&svc, &parts, input(json!({"id": id})))
            .await
            .expect("the owner reads"),
    );
    let shown = read["open_meta"]
        .get("note")
        .cloned()
        .unwrap_or(Value::Null);
    for guess in ["hush-hush", "the-old-value"] {
        let answer = meta_write(
            &svc,
            &parts,
            &id,
            json!({"x": 1}),
            Some(json!({"open": {"note": guess}})),
        )
        .await;
        let precondition = refusal(&answer, &[]);
        assert_eq!(
            precondition["failed"][0]["current"], shown,
            "current is what the read shows"
        );
        if guess != "hush-hush" {
            let whole = serde_json::to_string(answer.as_ref().unwrap()).unwrap();
            assert!(
                !whole.contains("hush-hush"),
                "nothing the act removed appears anywhere in the answer: {whole}"
            );
        }
    }
    assert!(landed(
        &meta_write(
            &svc,
            &parts,
            &id,
            json!({"x": 1}),
            Some(json!({"open": {"note": shown}})),
        )
        .await
    ));
}

/// The deployed MCP router relaying to `api_base_url`, on a loopback listener.
async fn spawn_mcp_router(api_base_url: &str) -> std::net::SocketAddr {
    let api_base_url = api_base_url.to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mcp listener");
    let addr = listener.local_addr().expect("mcp addr");
    tokio::spawn(async move {
        axum::serve(
            listener,
            temper_mcp_server::build_router(
                common::mcp_server_config_relaying(
                    temper_mcp_server::config::blob_door(None, false),
                    &api_base_url,
                ),
                common::mcp_test_jwks(),
                common::mcp_discovery_config("http://mcp.test"),
            ),
        )
        .await
        .expect("mcp server");
    });
    addr
}

/// **No WARN-or-louder log line carries a refusal's value**, over the deployed MCP transport.
/// rmcp logs every JSON-RPC error at `warn` with its `data` in full; the refusal is a tool result,
/// which it logs at `debug` only. A control call that does answer a JSON-RPC error is seen at
/// `warn`, so the capture is live on this path.
///
/// FAILS IF the refusal travels as a JSON-RPC error (the value appears in rmcp's `response error`
/// line), or anything else on the request path logs a value at `warn` or `error`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn no_warn_line_carries_a_refusals_value(pool: PgPool) {
    use rmcp::model::{CallToolRequestParams, ClientConfig};
    use rmcp::service::ServiceExt;
    use rmcp::transport::streamable_http_client::{
        StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    };
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let (layer, events) = common::tracing_layer::TestTracingLayer::new();
    let _guard = tracing_subscriber::registry().with(layer).set_default();

    let (app, svc, parts) = harness(pool).await;
    let id = create_task(&svc, &parts, &app.pool).await;
    assert!(landed(
        &meta_write(
            &svc,
            &parts,
            &id,
            json!({"claim_id": "claim-secret-value"}),
            None
        )
        .await
    ));

    let mcp = spawn_mcp_router(&app.base_url()).await;
    let transport = StreamableHttpClientTransport::with_client(
        reqwest13::Client::new(),
        StreamableHttpClientTransportConfig::with_uri(format!("http://{mcp}/mcp"))
            .auth_header(app.token.clone()),
    );
    let service = ClientConfig::default()
        .serve(transport)
        .await
        .expect("the initialize handshake");
    let peer = service.peer().clone();
    let call = |args: Value| {
        CallToolRequestParams::new("update_resource_meta".to_owned())
            .with_arguments(args.as_object().expect("object args").to_owned())
    };

    let refused = peer
        .call_tool(call(json!({
            "id": id,
            "managed_meta": {},
            "open_meta": {"claim_id": "claim-of-the-caller"},
            "expect": {"open": {"claim_id": null}},
        })))
        .await;

    // The control: an unknown resource answers a JSON-RPC error, which rmcp logs at `warn`.
    let ghost = peer
        .call_tool(call(json!({
            "id": Uuid::now_v7(),
            "managed_meta": {},
            "open_meta": {"claim_id": "x"},
        })))
        .await;
    assert!(ghost.is_err(), "the control is a JSON-RPC error: {ghost:?}");
    service.cancel().await.ok();

    let captured = events.lock().unwrap().clone();
    let loud: Vec<_> = captured
        .iter()
        .filter(|e| e.level <= tracing::Level::WARN)
        .collect();
    assert!(
        loud.iter()
            .any(|e| e.fields.values().any(|v| v.contains("response error"))),
        "the control's JSON-RPC error was logged at warn, so the capture sees rmcp: {loud:#?}"
    );
    for event in &loud {
        for value in event.fields.values().chain(event.span_fields.values()) {
            for secret in ["claim-secret-value", "claim-of-the-caller"] {
                assert!(
                    !value.contains(secret),
                    "a {} line carries a value ({secret}): {event:#?}",
                    event.level
                );
            }
        }
    }
    // Asserted after the logs, so a refusal sent as a JSON-RPC error fails on the log line.
    let refused = refused.expect("a refusal is a tool result over the transport");
    assert_eq!(refused.is_error, Some(true));
    assert_eq!(
        refused.structured_content.as_ref().unwrap()["precondition"]["failed"][0]["current"],
        "claim-secret-value",
        "the caller is told"
    );
}

/// **A misspelled guard is refused, never dropped**, over the deployed MCP transport. Before the
/// door's argument rule, serde dropped `"expects"` (for `"expect"`) without a word: the write
/// landed unconditionally and answered success, the claim race the guard exists to stop. Now the
/// call is a tool error result naming the key, the resource is unchanged and no event is
/// recorded, on both meta writes; no WARN-or-louder line carries a value; and the same call
/// spelled right still meets the guard.
///
/// FAILS IF the shared gate in `call_tool` is removed (the misspelled claim lands), or the refusal
/// travels as a JSON-RPC error.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_misspelled_expect_is_refused_and_nothing_is_written(pool: PgPool) {
    use rmcp::model::{CallToolRequestParams, ClientConfig};
    use rmcp::service::ServiceExt;
    use rmcp::transport::streamable_http_client::{
        StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
    };
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let (layer, events) = common::tracing_layer::TestTracingLayer::new();
    let _guard = tracing_subscriber::registry().with(layer).set_default();

    let (app, svc, parts) = harness(pool).await;
    let id = create_task(&svc, &parts, &app.pool).await;
    assert!(landed(&claim(&svc, &parts, &id, "claim-a").await));

    let mcp = spawn_mcp_router(&app.base_url()).await;
    let transport = StreamableHttpClientTransport::with_client(
        reqwest13::Client::new(),
        StreamableHttpClientTransportConfig::with_uri(format!("http://{mcp}/mcp"))
            .auth_header(app.token.clone()),
    );
    let service = ClientConfig::default()
        .serve(transport)
        .await
        .expect("the initialize handshake");
    let peer = service.peer().clone();
    let call = |tool: &str, args: Value| {
        CallToolRequestParams::new(tool.to_owned())
            .with_arguments(args.as_object().expect("object args").to_owned())
    };

    let before = event_count(&app.pool).await;
    let meta = peer
        .call_tool(call(
            "update_resource_meta",
            json!({
                "id": id,
                "managed_meta": {},
                "open_meta": {"claim_id": "claim-of-the-caller"},
                "expects": {"open": {"claim_id": null}},
            }),
        ))
        .await
        .expect("an undeclared argument is a tool result over the transport");
    let update = peer
        .call_tool(call(
            "update_resource",
            json!({
                "id": id,
                "title": "Renamed-by-the-caller",
                "open_meta": {"claim_id": "claim-of-the-caller"},
                "expects": {"open": {"claim_id": null}},
            }),
        ))
        .await
        .expect("an undeclared argument is a tool result over the transport");
    for (tool, refused) in [
        ("update_resource_meta", &meta),
        ("update_resource", &update),
    ] {
        assert_eq!(refused.is_error, Some(true), "{tool}: {refused:?}");
        let text = refused.content[0]
            .as_text()
            .expect("a text part")
            .text
            .as_str();
        assert!(
            text.contains("unknown argument `expects`"),
            "{tool}: {text}"
        );
        assert!(
            !text.contains("claim-of-the-caller"),
            "{tool}: no value: {text}"
        );
        assert!(refused.structured_content.is_none(), "{tool}: {refused:?}");
    }
    assert_eq!(stored_claim(&app.pool, &id).await, json!("claim-a"));
    let title: String = sqlx::query_scalar("SELECT title FROM kb_resources WHERE id = $1::uuid")
        .bind(&id)
        .fetch_one(&app.pool)
        .await
        .expect("the title");
    assert_ne!(title, "Renamed-by-the-caller", "nothing was written");
    assert_eq!(
        event_count(&app.pool).await,
        before,
        "no event was recorded"
    );

    // The same claim spelled right reaches the guard, which refuses it on its own terms.
    let guarded = peer
        .call_tool(call(
            "update_resource_meta",
            json!({
                "id": id,
                "managed_meta": {},
                "open_meta": {"claim_id": "claim-of-the-caller"},
                "expect": {"open": {"claim_id": null}},
            }),
        ))
        .await
        .expect("a guard refusal is a tool result");
    assert_eq!(
        guarded
            .structured_content
            .as_ref()
            .expect("the precondition")["code"],
        "PRECONDITION_FAILED"
    );
    service.cancel().await.ok();

    let captured = events.lock().unwrap().clone();
    for event in captured.iter().filter(|e| e.level <= tracing::Level::WARN) {
        for value in event.fields.values().chain(event.span_fields.values()) {
            assert!(
                !value.contains("claim-of-the-caller"),
                "a {} line carries the refused call: {event:#?}",
                event.level
            );
        }
    }
    assert_eq!(stored_claim(&app.pool, &id).await, json!("claim-a"));
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
    assert!(landed(&claim(&svc, &parts, &id, "claim-a").await));

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
