#![cfg(feature = "test-db")]
//! The coordination convention, witnessed over MCP exactly as the shipped skill teaches it.
//!
//! `coordinating-through-records.md` (a shared skill template) teaches agents to coordinate through
//! records with existing tools only: a setup act over the shape tools, guarded claims on a work
//! item's `open_meta`, and claim and outcome records committed against the shapes it carries. This
//! suite runs those steps through the deployed MCP relay against this process's listener, and reads
//! **the shapes out of the skill file itself**, so a schema edited into something the server
//! refuses fails here rather than in an agent's session.
//!
//! The mechanism under the claim (the guard, its lock, its refusal) is witnessed in
//! `meta_precondition_doors_test.rs`. This suite witnesses the convention built on it.

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

type Parts = axum::http::request::Parts;
type Answer = Result<rmcp::model::CallToolResult, rmcp::ErrorData>;

// ── the skill's shapes ───────────────────────────────────────────────────────────────────────────

/// One family as the skill ships it: `{kind, enforcement, schema}`.
#[derive(Debug, Clone)]
struct Family {
    kind: String,
    enforcement: String,
    schema: Value,
}

/// Every `coordination-*` family in the skill template's JSON fences, in file order.
fn skill_families() -> Vec<Family> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/temper-cli/templates/shared/coordinating-through-records.md"
    );
    let text = std::fs::read_to_string(path).expect("the skill template is readable");
    let mut families = Vec::new();
    let mut rest = text.as_str();
    while let Some(start) = rest.find("```json\n") {
        let body = &rest[start + "```json\n".len()..];
        let end = body.find("```").expect("a closed fence");
        if let Ok(value) = serde_json::from_str::<Value>(&body[..end]) {
            if let Some(kind) = value["kind"]
                .as_str()
                .filter(|k| k.starts_with("coordination-"))
            {
                families.push(Family {
                    kind: kind.to_string(),
                    enforcement: value["enforcement"].as_str().expect("enforcement").into(),
                    schema: value["schema"].clone(),
                });
            }
        }
        rest = &body[end + 3..];
    }
    let kinds: Vec<&str> = families.iter().map(|f| f.kind.as_str()).collect();
    assert_eq!(
        kinds,
        [
            "coordination-claim",
            "coordination-outcome",
            "coordination-agent"
        ],
        "the skill ships the three families"
    );
    families
}

// ── harness ──────────────────────────────────────────────────────────────────────────────────────

async fn harness(pool: PgPool) -> (E2eTestApp, TemperMcpService, Parts) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service().await;
    let parts = app.relay_parts();
    (app, svc, parts)
}

fn input<T: for<'de> serde::Deserialize<'de>>(value: Value) -> T {
    serde_json::from_value(value).expect("input deserializes from its wire shape")
}

fn one_text(res: &rmcp::model::CallToolResult) -> Value {
    assert_ne!(res.is_error, Some(true), "a successful result: {res:?}");
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

async fn default_context_id(pool: &PgPool) -> Uuid {
    sqlx::query_scalar(
        "SELECT c.id FROM kb_contexts c JOIN kb_profiles p ON p.id = c.owner_id \
          WHERE p.email = $1 AND c.name = 'default'",
    )
    .bind(EMAIL)
    .fetch_one(pool)
    .await
    .expect("the auto-provisioned default context")
}

/// A fresh claim value: a lowercase UUIDv7, minted per attempt as the skill says.
fn mint() -> String {
    Uuid::now_v7().to_string()
}

/// A work item in `ctx`: any resource will do, so it is not a task.
async fn work_item(svc: &TemperMcpService, parts: &Parts, ctx: Uuid) -> String {
    let created = one_text(
        &temper_mcp::tools::resources::create_resource(
            svc,
            parts,
            input(json!({
                "context_ref": ctx.to_string(),
                "doc_type_name": "research",
                "title": "A work item",
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

/// A second approved identity with read and write on the harness principal's default context.
async fn second_agent(app: &E2eTestApp) -> Parts {
    let unique = Uuid::new_v4();
    let email = format!("agent-{unique}@example.com");
    let token = common::generate_test_jwt(&format!("agent-sub-{unique}"), &email);
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
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, \
                                       can_read, can_write, granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, true, $3)",
    )
    .bind(default_context_id(&app.pool).await)
    .bind(profile_id(&app.pool, &email).await)
    .bind(profile_id(&app.pool, EMAIL).await)
    .execute(&app.pool)
    .await
    .expect("grant context write");
    app.relay_parts_for(&token)
}

// ── the skill's steps ────────────────────────────────────────────────────────────────────────────

async fn live_shapes(svc: &TemperMcpService, parts: &Parts, ctx: Uuid) -> Vec<Value> {
    let listed = one_text(
        &temper_mcp::tools::data_artifact_shapes::list_shapes(
            svc,
            parts,
            input(json!({"home_type": "context", "home_id": ctx.to_string()})),
        )
        .await
        .expect("list lands"),
    );
    listed.as_array().expect("a list").clone()
}

/// The setup act, step for step: list; per family compare the root `$id` and the enforcement;
/// declare only what is missing or differs, naming the context's owner as `kind_owner`; list
/// again. Answers the kinds it declared.
async fn setup_act(
    svc: &TemperMcpService,
    parts: &Parts,
    ctx: Uuid,
    families: &[Family],
) -> Vec<String> {
    let context = one_text(
        &temper_mcp::tools::contexts::context_read(
            svc,
            parts,
            input(json!({"view": "get", "id": ctx.to_string()})),
        )
        .await
        .expect("read the context"),
    );
    let owner = json!({
        context["kb_owner_table"].as_str().expect("owner table"):
            context["kb_owner_id"].as_str().expect("owner id")
    });

    let live = live_shapes(svc, parts, ctx).await;
    let mut declared = Vec::new();
    for family in families {
        let current = live
            .iter()
            .find(|s| s["artifact_kind"] == family.kind.as_str());
        let matches = current.is_some_and(|s| {
            s["schema"]["$id"] == family.schema["$id"] && s["enforcement"] == family.enforcement
        });
        if matches {
            continue;
        }
        one_text(
            &temper_mcp::tools::data_artifact_shapes::declare_shape(
                svc,
                parts,
                input(json!({
                    "home_type": "context",
                    "home_id": ctx.to_string(),
                    "kind": family.kind,
                    "kind_owner": owner,
                    "schema": family.schema,
                    "enforcement": family.enforcement,
                })),
            )
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "the server accepts the skill's {} shape: {e:?}",
                    family.kind
                )
            }),
        );
        declared.push(family.kind.clone());
    }

    let after = live_shapes(svc, parts, ctx).await;
    for family in families {
        let shape = after
            .iter()
            .find(|s| s["artifact_kind"] == family.kind.as_str())
            .unwrap_or_else(|| panic!("{} is live after setup", family.kind));
        assert_eq!(shape["schema"]["$id"], family.schema["$id"], "{shape}");
        assert_eq!(shape["enforcement"], family.enforcement.as_str(), "{shape}");
    }
    declared
}

/// The claim: the convention's keys, expecting `claim_id` absent, under correlation `c`.
async fn claim(svc: &TemperMcpService, parts: &Parts, item: &str, c: &str) -> Answer {
    temper_mcp::tools::resources::update_resource_meta(
        svc,
        parts,
        input(json!({
            "id": item,
            "managed_meta": {},
            "open_meta": {"claim_id": c, "claimed_at": "2026-11-14T12:00:00Z"},
            "expect": {"open": {"claim_id": null}},
            "correlation_id": c,
            "model": "witness-model",
            "persona": "worker",
            "confidence": "probable",
        })),
    )
    .await
}

/// A heartbeat: a liveness key the workflow chose, guarded on the holder's claim value.
async fn heartbeat(svc: &TemperMcpService, parts: &Parts, item: &str, c: &str, at: &str) -> Answer {
    temper_mcp::tools::resources::update_resource_meta(
        svc,
        parts,
        input(json!({
            "id": item,
            "managed_meta": {},
            "open_meta": {"lease_renewed_at": at},
            "expect": {"open": {"claim_id": c}},
            "correlation_id": c,
        })),
    )
    .await
}

/// A reclaim: clear the claim keys, expecting `claim_id` to equal the stale value read.
async fn reclaim(svc: &TemperMcpService, parts: &Parts, item: &str, stale: &str) -> Answer {
    temper_mcp::tools::resources::update_resource_meta(
        svc,
        parts,
        input(json!({
            "id": item,
            "managed_meta": {},
            "open_meta": {"claim_id": null, "claimed_at": null},
            "expect": {"open": {"claim_id": stale}},
            "correlation_id": mint(),
            "persona": "operator",
            "confidence": "confident",
        })),
    )
    .await
}

/// What the skill tells a claimant its answer means.
#[derive(Debug, PartialEq)]
enum Verdict {
    Held,
    Won,
    Lost(Value),
}

/// Read an answer the way the skill teaches: a landed write holds; a refusal is read from
/// `structuredContent.precondition`, and a `current` equal to your own value means you won.
fn verdict(answer: &Answer, mine: &str) -> Verdict {
    let result = match answer {
        Ok(r) => r,
        Err(e) => panic!("a refusal is a tool result, never a protocol error: {e:?}"),
    };
    if result.is_error != Some(true) {
        return Verdict::Held;
    }
    let data = result
        .structured_content
        .as_ref()
        .expect("the refusal rides structuredContent");
    assert_eq!(data["code"], "PRECONDITION_FAILED", "{data}");
    let failed = data["precondition"]["failed"].as_array().expect("entries");
    let entry = failed
        .iter()
        .find(|f| f["tier"] == "open" && f["key"] == "claim_id")
        .unwrap_or_else(|| panic!("the refusal names claim_id: {data}"));
    if entry["current"] == mine {
        Verdict::Won
    } else {
        Verdict::Lost(entry["current"].clone())
    }
}

/// `key`'s newest live value on the item: `null` when it has none, or when that value is a stored
/// top-level `null`, which the guard also reads as absent.
async fn stored(pool: &PgPool, item: &str, key: &str) -> Value {
    sqlx::query_scalar(
        "SELECT property_value FROM kb_properties \
          WHERE owner_table = 'kb_resources' AND owner_id = $1::uuid \
            AND property_key = $2 AND NOT is_folded \
          ORDER BY created DESC, id DESC LIMIT 1",
    )
    .bind(item)
    .bind(key)
    .fetch_optional(pool)
    .await
    .expect("read the key")
    .unwrap_or(Value::Null)
}

async fn commit(
    svc: &TemperMcpService,
    parts: &Parts,
    item: &str,
    kind: &str,
    content: Value,
    c: &str,
) -> Answer {
    temper_mcp::tools::data_artifacts::commit_artifact(
        svc,
        parts,
        input(json!({
            "resource_id": item,
            "kind": kind,
            "intent": "member",
            "content": content,
            "correlation_id": c,
            "model": "witness-model",
            "persona": "worker",
            "confidence": "probable",
        })),
    )
    .await
}

// ── the setup act ────────────────────────────────────────────────────────────────────────────────

/// **The setup act declares the three families in a fresh context, and re-running it declares
/// nothing.** In a context with no resource (so `kind_owner` must be named), the act declares
/// claim and outcome enforcing and agent advisory, from the skill's own schemas. A second run
/// compares `$id` and enforcement, declares nothing, and leaves every shape id where it was. A
/// changed enforcement re-declares that family alone.
///
/// FAILS IF the server refuses a schema the skill ships, the comparison misses a live shape (a
/// re-run mints new shape ids), or a listing does not carry `$id` and enforcement back.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_setup_act_declares_three_families_once_and_a_rerun_declares_nothing(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let families = skill_families();
    let created = one_text(
        &temper_mcp::tools::contexts::context_manage(
            &svc,
            &parts,
            input(json!({"action": "create", "name": "coordination witness"})),
        )
        .await
        .expect("create the context"),
    );
    let ctx: Uuid = created["id"].as_str().expect("an id").parse().unwrap();
    assert!(
        live_shapes(&svc, &parts, ctx).await.is_empty(),
        "a fresh context"
    );

    let declared = setup_act(&svc, &parts, ctx, &families).await;
    assert_eq!(
        declared,
        [
            "coordination-claim",
            "coordination-outcome",
            "coordination-agent"
        ]
    );
    let first = live_shapes(&svc, &parts, ctx).await;
    let enforcement: Vec<(String, String)> = families
        .iter()
        .map(|f| {
            let s = first
                .iter()
                .find(|s| s["artifact_kind"] == f.kind.as_str())
                .unwrap();
            (
                f.kind.clone(),
                s["enforcement"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        enforcement,
        [
            ("coordination-claim".into(), "enforcing".into()),
            ("coordination-outcome".into(), "enforcing".into()),
            ("coordination-agent".into(), "advisory".into()),
        ]
    );
    let ids = |shapes: &[Value]| {
        let mut ids: Vec<String> = shapes
            .iter()
            .map(|s| s["shape_id"].as_str().unwrap().to_string())
            .collect();
        ids.sort();
        ids
    };
    let versions_before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_data_artifact_shapes WHERE home_anchor_id = $1",
    )
    .bind(ctx)
    .fetch_one(&app.pool)
    .await
    .unwrap();

    let rerun = setup_act(&svc, &parts, ctx, &families).await;
    assert!(rerun.is_empty(), "a re-run declares nothing: {rerun:?}");
    let second = live_shapes(&svc, &parts, ctx).await;
    assert_eq!(ids(&first), ids(&second), "the shape ids are unchanged");
    let versions_after: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_data_artifact_shapes WHERE home_anchor_id = $1",
    )
    .bind(ctx)
    .fetch_one(&app.pool)
    .await
    .unwrap();
    assert_eq!(versions_after, versions_before, "no version was minted");

    // Enforcement is the workflow's parameter: choosing advisory for claims re-declares that
    // family and no other.
    let mut advisory = families.clone();
    advisory[0].enforcement = "advisory".into();
    let changed = setup_act(&svc, &parts, ctx, &advisory).await;
    assert_eq!(changed, ["coordination-claim"]);
}

// ── claims ───────────────────────────────────────────────────────────────────────────────────────

/// Hold R's writer lock on a side transaction, so both claimants are seen contending for it.
async fn hold_writer_lock(pool: &PgPool, id: &str) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut held = pool.begin().await.expect("begin");
    sqlx::query("SELECT 1 FROM kb_resources WHERE id = $1::uuid FOR NO KEY UPDATE")
        .bind(id)
        .execute(&mut *held)
        .await
        .expect("take the writer lock");
    held
}

async fn writers_wait_on_the_row_lock(pool: &PgPool, n: i64) {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
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

/// **Two agents claim one work item; exactly one wins, and the loser's refusal shows the winner's
/// value.** Both claimants are held at the work item's writer lock and released together. The
/// winner's verdict is `Held`; the loser's is `Lost` with `current` equal to the winner's claim
/// value, and the stored claim is the winner's.
///
/// FAILS IF both claims land (the later overwrites the earlier), or the refusal does not carry the
/// value there now.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn two_agents_claim_one_item_and_exactly_one_wins(pool: PgPool) {
    let (app, svc, owner) = harness(pool).await;
    let ctx = default_context_id(&app.pool).await;
    let item = work_item(&svc, &owner, ctx).await;
    let other = second_agent(&app).await;
    let svc = Arc::new(svc);

    let held = hold_writer_lock(&app.pool, &item).await;
    let spawn = |parts: Parts, c: String| {
        let svc = svc.clone();
        let item = item.clone();
        tokio::spawn(async move {
            let answer = claim(&svc, &parts, &item, &c).await;
            (c, answer)
        })
    };
    let a = spawn(owner, mint());
    let b = spawn(other, mint());
    writers_wait_on_the_row_lock(&app.pool, 2).await;
    held.commit().await.expect("release the writer lock");
    let answers = [a.await.unwrap(), b.await.unwrap()];

    let verdicts: Vec<Verdict> = answers.iter().map(|(c, r)| verdict(r, c)).collect();
    let winners: Vec<&String> = answers
        .iter()
        .zip(&verdicts)
        .filter(|(_, v)| **v == Verdict::Held)
        .map(|((c, _), _)| c)
        .collect();
    assert_eq!(winners.len(), 1, "exactly one claim lands: {verdicts:?}");
    let winner = winners[0].clone();
    let loser = verdicts
        .iter()
        .find(|v| **v != Verdict::Held)
        .expect("a loser");
    assert_eq!(
        *loser,
        Verdict::Lost(json!(winner)),
        "the loser sees the winner's value"
    );
    assert_eq!(stored(&app.pool, &item, "claim_id").await, json!(winner));
}

/// A forwarding proxy that loses the answer to the first `PUT /meta` after the API committed it,
/// and answers the relay `502`, so the relay replays the write once.
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

/// **A refusal whose `current` equals your own claim value means you won.** The claim's first
/// attempt commits and its answer is lost; the relay replays it and the replay is refused with the
/// claimant's own `C` as `current`. Read as the skill teaches, that is `Won`, and the claimant
/// continues as the holder: its heartbeat, guarded on `C`, lands.
///
/// FAILS IF the replay is not refused with the caller's own value (the claimant cannot tell it
/// won), or the claimant, having won, cannot act as the holder.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_refusal_showing_your_own_claim_value_means_you_won(pool: PgPool) {
    let (app, direct, parts) = harness(pool).await;
    let ctx = default_context_id(&app.pool).await;
    let item = work_item(&direct, &parts, ctx).await;

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

    let c = mint();
    let answer = claim(&lossy, &parts, &item, &c).await;
    serve.abort();
    assert_eq!(
        proxy.meta_puts.load(Ordering::SeqCst),
        2,
        "the relay replayed once"
    );
    assert_eq!(verdict(&answer, &c), Verdict::Won, "{answer:?}");
    assert_eq!(stored(&app.pool, &item, "claim_id").await, json!(c));

    let beat = heartbeat(&direct, &parts, &item, &c, "2026-11-14T12:01:00Z").await;
    assert_eq!(
        verdict(&beat, &c),
        Verdict::Held,
        "the winner holds the claim"
    );
}

// ── records ──────────────────────────────────────────────────────────────────────────────────────

/// **A claim record and an outcome record commit against the enforcing shapes.** After the setup
/// act, the claim's record and the work's outcome, both under correlation `C`, land
/// `declared_satisfied`. An outcome missing the required `status` is refused, because the shape
/// enforces.
///
/// FAILS IF the skill's record examples do not satisfy the skill's own shapes, or the shapes the
/// setup act declares do not enforce.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn claim_and_outcome_records_commit_against_the_enforcing_shapes(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let ctx = default_context_id(&app.pool).await;
    let item = work_item(&svc, &parts, ctx).await;
    setup_act(&svc, &parts, ctx, &skill_families()).await;

    let c = mint();
    assert_eq!(
        verdict(&claim(&svc, &parts, &item, &c).await, &c),
        Verdict::Held
    );

    let record = commit(
        &svc,
        &parts,
        &item,
        "coordination-claim",
        json!({"claim_id": c, "subject": item, "model": "witness-model"}),
        &c,
    )
    .await;
    let record = one_text(&record.expect("the claim record commits"));
    assert_eq!(
        record["artifact"]["shape_state"], "declared_satisfied",
        "{record}"
    );
    assert_eq!(
        record["artifact"]["artifact_kind"], "coordination-claim",
        "{record}"
    );

    let outcome = commit(
        &svc,
        &parts,
        &item,
        "coordination-outcome",
        json!({
            "claim_id": c,
            "subject": item,
            "status": "succeeded",
            "summary": "done",
            "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15},
        }),
        &c,
    )
    .await;
    let outcome = one_text(&outcome.expect("the outcome record commits"));
    assert_eq!(
        outcome["artifact"]["shape_state"], "declared_satisfied",
        "{outcome}"
    );

    let refused = commit(
        &svc,
        &parts,
        &item,
        "coordination-outcome",
        json!({"claim_id": c, "subject": item}),
        &c,
    )
    .await;
    let refused_landed = matches!(&refused, Ok(r) if r.is_error != Some(true));
    assert!(
        !refused_landed,
        "an outcome without `status` is refused: {refused:?}"
    );
    assert!(
        format!("{refused:?}").contains("status"),
        "the refusal names the field that failed: {refused:?}"
    );
}

// ── heartbeat and reclaim ────────────────────────────────────────────────────────────────────────

/// **A guarded reclaim makes the old holder's heartbeat fail.** The holder claims with `C1` and
/// heartbeats under it. A reclaimer clears the claim keys expecting `claim_id = C1`. The old
/// holder's next heartbeat is refused with `current` absent, which the skill reads as `Lost`, and a
/// new claimant's claim then lands.
///
/// FAILS IF the reclaim's guard is dropped or misjudged, or a heartbeat guarded on a cleared claim
/// still lands.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_guarded_reclaim_makes_the_old_holders_heartbeat_fail(pool: PgPool) {
    let (app, svc, holder) = harness(pool).await;
    let ctx = default_context_id(&app.pool).await;
    let item = work_item(&svc, &holder, ctx).await;
    let operator = second_agent(&app).await;

    let c1 = mint();
    assert_eq!(
        verdict(&claim(&svc, &holder, &item, &c1).await, &c1),
        Verdict::Held
    );
    let beat = heartbeat(&svc, &holder, &item, &c1, "2026-11-14T12:01:00Z").await;
    assert_eq!(verdict(&beat, &c1), Verdict::Held);

    // A reclaim against a value that is not there is refused: the flow moved.
    let wrong = reclaim(&svc, &operator, &item, &mint()).await;
    assert_eq!(verdict(&wrong, "none"), Verdict::Lost(json!(c1)));

    let freed = reclaim(&svc, &operator, &item, &c1).await;
    assert!(
        matches!(&freed, Ok(r) if r.is_error != Some(true)),
        "{freed:?}"
    );
    assert_eq!(stored(&app.pool, &item, "claim_id").await, Value::Null);

    let late = heartbeat(&svc, &holder, &item, &c1, "2026-11-14T12:02:00Z").await;
    assert_eq!(
        verdict(&late, &c1),
        Verdict::Lost(Value::Null),
        "the old holder lost it"
    );
    assert_eq!(
        stored(&app.pool, &item, "lease_renewed_at").await,
        json!("2026-11-14T12:01:00Z"),
        "the refused heartbeat wrote nothing"
    );

    let c2 = mint();
    assert_eq!(
        verdict(&claim(&svc, &operator, &item, &c2).await, &c2),
        Verdict::Held
    );
}

// ── discovery ────────────────────────────────────────────────────────────────────────────────────

/// **A workflow-maintained tag finds claimable work, and each row shows its claim.** The workflow
/// adds a `claimable` marker with `open_meta_add`; `list_resources` by context and tag returns the
/// marked item and not an unmarked one, and the row's `open_meta` carries `claim_id` once claimed.
/// A claimed item can still be listed (a stale hit): the guarded claim decides, so a second
/// claimant loses.
///
/// FAILS IF the tag filter or the row's `open_meta` stops serving the pattern the skill teaches.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_workflow_tag_lists_claimable_work_with_its_claim(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    let ctx = default_context_id(&app.pool).await;
    let marked = work_item(&svc, &parts, ctx).await;
    let unmarked = work_item(&svc, &parts, ctx).await;
    one_text(
        &temper_mcp::tools::resources::update_resource(
            &svc,
            &parts,
            input(json!({"id": marked, "open_meta_add": {"tags": ["claimable"]}})),
        )
        .await
        .expect("mark the item"),
    );
    let c = mint();
    assert_eq!(
        verdict(&claim(&svc, &parts, &marked, &c).await, &c),
        Verdict::Held
    );

    let listed = one_text(
        &temper_mcp::tools::resources::list_resources(
            &svc,
            &parts,
            input(json!({"context_ref": ctx.to_string(), "tags": ["claimable"]})),
        )
        .await
        .expect("list by tag"),
    );
    let rows = listed["rows"].as_array().expect("rows");
    let ids: Vec<&str> = rows.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert_eq!(
        ids,
        [marked.as_str()],
        "only the marked item, never {unmarked}: {listed}"
    );
    assert_eq!(rows[0]["open_meta"]["claim_id"], json!(c), "{listed}");

    let late = mint();
    assert_eq!(
        verdict(&claim(&svc, &parts, &marked, &late).await, &late),
        Verdict::Lost(json!(c)),
        "a stale hit loses to the claim already there"
    );
}
