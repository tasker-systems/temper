#![cfg(feature = "test-db")]
//! The search + query family's parity suite, authored against the DIRECT binding
//! first, then carried green across the network door (beat G3b — the second proof of
//! the G3a-prime pattern).
//!
//! [Beat G3a](temper task `01a0cb1b-42e6-7922-a9dc-b30c402d8a3f`) established the
//! discipline this file follows: a parity suite that never saw the old binding cannot
//! prove parity, and one that never crosses the real listener cannot prove the door.
//! The tools are driven the way the MCP function drives them: a real
//! `TemperMcpService`, per-request parts carrying the harness principal's REAL
//! bearer, against THIS process's listener.
//!
//! # The refusal faces, named before they were witnessed (learning 4)
//!
//! **search**
//! - *Degenerate embedding* — `reject_degenerate_embedding`
//!   (`substrate_read.rs`) refuses a zero-norm/non-finite vector as
//!   `ApiError::BadRequest`. The DIRECT binding renders every error as
//!   `internal_error`; through the door it arrives as a 400 and the relayed call
//!   site renders it `invalid_params` with the server's sentence (the
//!   `contexts.rs::map_api_error` BadRequest arm's precedent).
//!   **DECLARED PARITY DELTA**: this one face changes rendered kind
//!   (`internal_error` → `invalid_params`) at the swap — the delta is named here and
//!   the assertion flips in the swap commit, nowhere else.
//! - *Post-edge 401 arms* — machine-gate / registration-gate / deactivation, mapped
//!   by the shared `map_post_edge_refusal` through `AcrossAuth` (expired-in-flight
//!   is wire-only; see the query notes). The arm wording is a shared constant both
//!   bindings speak; the full arm matrix lives in `resources_wire_arms_test.rs`,
//!   which already pins the shared mapping.
//! - *System-access 403* — both routes sit on the gated stack
//!   (`routes.rs::gated_routes`), so a denied-standing caller 403s at
//!   `require_system_access` and the five-field terminal sentence speaks.
//! - *Relay-unconfigured* — the `relay_client` constructor's own typed refusal
//!   (shared across every family; witnessed by `temper-mcp`'s transport-layer tests,
//!   not duplicated here).
//!
//! **query**
//! - *PLAN_REFUSED 400* — the API refuses with every static refusal at once
//!   (`error.details.refusals` under the code `PLAN_REFUSED`); temper-client
//!   reconstructs them into `ClientError::PlanRefused` and the tool renders each with
//!   its stage, reason, and detail — the arm-for-arm parity face this family is known
//!   for. A composition-level refusal (no stage) must not grow an empty `stage ''`
//!   prefix.
//! - *Post-edge 401 arms* and *system-access 403* — same vehicles as search, with one
//!   harness note: expired-in-flight is a face ONLY the wire produces (the direct
//!   gate does not re-check `exp` on planted claims — `auth_seam_parity_e2e`), so its
//!   witness joins at the swap; the other arms' full matrix lives in
//!   `resources_wire_arms_test.rs`, which already pins the shared mapping.
//! - *Body ceiling* — the wire face at `/api/query` is witnessed at the same route by
//!   `resources_wire_arms_test.rs`; not duplicated here.
//! - *Non-refusal errors stay opaque* — pinned at the rendering function by the
//!   unit tests in `tools/query.rs`; a suite cannot honestly force a server fault on
//!   a healthy listener.
//!
//! # How the tools are driven
//!
//! Through the tool functions with the harness-seeded profile — the cache
//! `mcp_relay_service` plants is the DIRECT binding's caller path
//! (`require_profile`); the swap replaces it with the bearer the relayed tool
//! carries. The system-access arm is the one exception: pre-swap its gate runs
//! in-process (`ensure_profile_from_parts`), and the test drives that gate
//! directly.

mod common;

use serde_json::json;
use sqlx::PgPool;
use temper_core::types::ingest::{pack_chunks, IngestPayload};
use temper_mcp::service::TemperMcpService;

mod parity {
    use serde::Deserialize;
    use sqlx::PgPool;
    use uuid::Uuid;

    /// The harness principal's email — the one the harness provisions and
    /// standing-approves, and the profile behind every `app.relay_parts()` bearer.
    pub const EMAIL: &str = "e2e@test.example.com";

    pub async fn default_context_id(pool: &PgPool) -> Uuid {
        sqlx::query_scalar(
            "SELECT c.id \
             FROM kb_contexts c \
             JOIN kb_profiles p ON p.id = c.owner_id \
             WHERE p.email = $1 AND c.name = 'default'",
        )
        .bind(EMAIL)
        .fetch_one(pool)
        .await
        .expect("the auto-provisioned default context")
    }

    /// Build a tool input from its WIRE shape, so the deserializer — not a struct
    /// literal — pins the field names an MCP caller actually sends.
    pub fn input<T: for<'de> Deserialize<'de>>(value: serde_json::Value) -> T {
        serde_json::from_value(value).expect("input deserializes from its wire shape")
    }

    /// The single text part a one-part tool result carries.
    pub fn one_text(res: &rmcp::model::CallToolResult) -> serde_json::Value {
        let parts = &res.content;
        assert_eq!(parts.len(), 1, "one content part, got {}", parts.len());
        serde_json::from_str(parts[0].as_text().expect("a text part").text.as_str())
            .expect("the part is the tool's JSON response")
    }

    /// `rmcp::ErrorData` codes: -32600 request error (terminal arms),
    /// -32602 invalid_params, -32603 internal_error.
    pub fn code_of(err: &rmcp::ErrorData) -> i32 {
        err.code.0
    }
}

use common::E2eTestApp;
use parity::{code_of, one_text};

/// The parity harness, once per test: the relay-ready service over this app's real
/// listener, and parts carrying the harness principal's REAL bearer.
async fn harness(pool: PgPool) -> (E2eTestApp, TemperMcpService, axum::http::request::Parts) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service(app.pool.clone()).await;
    let parts = app.relay_parts();
    (app, svc, parts)
}

/// Ingest a searchable resource through the harness's own client: content packed
/// into chunks with synthetic 0.1-filled 768-dim vectors, so BOTH arms have
/// something to match without an embed model (`common::chunked`'s shape).
async fn ingest_searchable(app: &E2eTestApp, title: &str, slug: &str, content: &str) {
    let chunks = common::chunked(content, 0.1);
    let payload = IngestPayload {
        idempotency_key: None,
        segmented: None,
        goal: None,
        title: title.to_string(),
        origin_uri: format!("test://search-query-parity/{slug}"),
        context_ref: "@me/default".to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: Some(temper_core::hash::sha256_hex(content.as_bytes())),
        content: content.to_string(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(pack_chunks(&chunks).expect("pack chunks")),
        act: Default::default(),
        sources: Vec::new(),
    };
    app.client
        .ingest()
        .create(&payload)
        .await
        .expect("corpus ingest lands");
}

/// Drive the `search` tool: the tool function over the harness-seeded profile —
/// the DIRECT binding's caller path. Byte-stable across the swap, where the tool
/// carries the bearer to the API instead.
async fn run_search(
    svc: &TemperMcpService,
    _parts: &axum::http::request::Parts,
    params: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::search::search(
        svc,
        parity::input::<temper_core::types::api::SearchParams>(params),
    )
    .await
}

/// Drive the `query` tool — same reasoning as `run_search`.
async fn run_query(
    svc: &TemperMcpService,
    _parts: &axum::http::request::Parts,
    body: serde_json::Value,
) -> Result<rmcp::model::CallToolResult, rmcp::ErrorData> {
    temper_mcp::tools::query::run_query(
        svc,
        parity::input::<temper_mcp::tools::query::QueryInput>(body),
    )
    .await
}

/// Request parts carrying hand-built claims beside the bearer — the shape the
/// DIRECT binding's in-process gate reads (`authed_request` needs BOTH
/// extensions; `relay_parts` alone is the relay's bearer-only shape).
fn claimed_parts(
    token: &str,
    email: Option<&str>,
    exp_offset_secs: i64,
) -> axum::http::request::Parts {
    use chrono::Utc;
    let now = Utc::now().timestamp();
    axum::http::Request::builder()
        .extension(temper_mcp::middleware::BearerToken(token.to_string()))
        .extension(temper_services::auth::RawJwtClaims {
            sub: "e2e-test-user".to_string(),
            email: email.map(str::to_string),
            email_verified: Some(true),
            azp: None,
            gty: None,
            exp: now + exp_offset_secs,
            iat: 0,
        })
        .body(())
        .expect("claimed parts build")
        .into_parts()
        .0
}

// ── search: hit shapes ──────────────────────────────────────────────

/// The exact arm answers with hits carrying the resource and its own quantity, the
/// arm's own disposition, and the scope both arms share — the two-arms-never-combined
/// shape, unmerged.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn exact_arm_hits_carry_resource_quantity_reason_and_scope(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    ingest_searchable(
        &app,
        "Kubernetes Deployment Strategy",
        "k8s",
        "This document covers rolling updates and canary releases for Kubernetes clusters.",
    )
    .await;

    let res = run_search(&svc, &parts, json!({ "query": "Kubernetes" }))
        .await
        .expect("search answers");
    let v = one_text(&res);

    let hits = v["exact"]["hits"].as_array().expect("exact hits array");
    assert!(!hits.is_empty(), "the term matches: {v}");
    let hit = &hits[0];
    assert_eq!(
        hit["resource"]["title"], "Kubernetes Deployment Strategy",
        "{v}"
    );
    assert!(
        hit["resource"]["id"].as_str().is_some(),
        "the hit carries the resource: {v}"
    );
    assert!(
        hit["fts_norm"]
            .as_f64()
            .is_some_and(|f| (0.0..1.0).contains(&f)),
        "the arm's own quantity rides the hit: {v}"
    );
    assert_eq!(v["exact"]["reason"], "ok", "{v}");
    assert_eq!(v["scope"]["kind"], "global", "{v}");
    // The wide arm answers the same question with ITS OWN disposition — never merged.
    assert!(v.get("wide").is_some(), "the wide arm is present: {v}");
}

/// The wide arm answers an embedding-only question with its own hits, quantity, and
/// disposition, while the exact arm — asked nothing — says so per-arm rather than at
/// response level.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn wide_arm_hits_carry_the_embedding_match_and_the_exact_arm_stays_silent(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    ingest_searchable(
        &app,
        "Vector space probe",
        "vector-probe",
        "Embedding proximity across the corpus, never merged with term matching.",
    )
    .await;

    let embedding = vec![0.1f32; 768];
    let res = run_search(&svc, &parts, json!({ "embedding": embedding }))
        .await
        .expect("search answers");
    let v = one_text(&res);

    let hits = v["wide"]["hits"].as_array().expect("wide hits array");
    assert!(!hits.is_empty(), "the identical vector matches: {v}");
    let hit = &hits[0];
    assert_eq!(hit["resource"]["title"], "Vector space probe", "{v}");
    assert!(
        hit["vec_norm"]
            .as_f64()
            .is_some_and(|f| (0.0..=1.0).contains(&f)),
        "the wide arm's quantity rides the hit: {v}"
    );
    assert_eq!(v["wide"]["reason"], "ok", "{v}");
    assert_eq!(v["wide"]["degraded"], false, "{v}");
    // No query text ⇒ the exact arm matched nothing, and says so ITSELF.
    assert_eq!(v["exact"]["reason"], "no_match", "{v}");
    assert!(v["exact"]["hits"].as_array().expect("empty").is_empty());
}

/// Empty-result shapes: a global miss and a context-scoped miss both answer with
/// the arm's own `no_match` and its hint — the reason, not a bare empty list, is
/// the answer an agent can act on.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn empty_results_carry_the_arm_disposition_not_a_bare_list(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;

    let res = run_search(&svc, &parts, json!({ "query": "zzz-no-such-term" }))
        .await
        .expect("search answers");
    let v = one_text(&res);
    assert!(
        v["exact"]["hits"].as_array().expect("empty").is_empty(),
        "{v}"
    );
    assert_eq!(v["exact"]["reason"], "no_match", "{v}");
    assert!(
        v["exact"].get("hint").is_some(),
        "a non-Ok reason carries its hint: {v}"
    );

    let ctx = parity::default_context_id(&app.pool).await;
    let res = run_search(
        &svc,
        &parts,
        json!({ "query": "anything", "context_ref": ctx.to_string() }),
    )
    .await
    .expect("search answers");
    let v = one_text(&res);
    assert_eq!(v["scope"]["kind"], "context", "{v}");
    assert_eq!(v["exact"]["reason"], "no_match", "{v}");
}

// ── search: refusal faces ───────────────────────────────────────────

/// The degenerate-embedding caller error. DIRECT binding today: the tool wraps every
/// error as `internal_error` (-32603) — pinned here so the swap's kind change is a
/// DECLARED delta, not a silent one (see the header).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_degenerate_embedding_refuses_as_the_direct_binding_renders_it(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;

    let err = run_search(&svc, &parts, json!({ "embedding": vec![0.0f32; 768] }))
        .await
        .expect_err("a zero-magnitude vector is refused");
    assert_eq!(
        code_of(&err),
        -32603,
        "the DIRECT binding renders every error internal: {err}"
    );
    assert!(
        err.message.contains("Search failed:"),
        "the direct wrapper's own voice: {err}"
    );
}

/// System access (Level 2), pre-swap: the gate runs IN-PROCESS
/// (`ensure_profile_from_parts`), and a denied-standing caller meets the gate's
/// terminal sentence, naming the identity. At the swap this gate leaves the
/// method and the same sentence comes back from the API's 403 through the
/// preserved-body mapping.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_denied_standing_speaks_the_system_access_arm(pool: PgPool) {
    let (app, svc, _parts) = harness(pool).await;

    sqlx::query(
        "INSERT INTO kb_principal_standing (profile_id, state)
         SELECT id, 'denied' FROM kb_profiles WHERE email = $1
         ON CONFLICT (profile_id) DO UPDATE SET state = 'denied'",
    )
    .bind(parity::EMAIL)
    .execute(&app.pool)
    .await
    .expect("deny the principal's standing");

    let parts = claimed_parts(&app.token, Some(parity::EMAIL), 3600);
    let err = svc
        .ensure_profile_from_parts(&parts)
        .await
        .expect_err("a denied principal does not pass the gate");
    assert_eq!(code_of(&err), -32600, "terminal, never retryable: {err}");
    assert!(
        err.message.starts_with(
            "Access to this temper instance requires approval for e2e@test.example.com — "
        ),
        "the sentence names the identity: {err}"
    );
    assert!(
        err.message
            .ends_with("This error is terminal and should not be retried."),
        "framed terminal: {err}"
    );
}

// ── query: the composition contract ─────────────────────────────────

/// A stage with a question the CALLER already vectorized: a caller-supplied
/// 768-dim embedding means `prepare`'s embed step has nothing to do, so the plan
/// runs without an embed model. The corpus chunks it matches carry the same
/// synthetic vector (`common::chunked`).
fn answerable_plan() -> serde_json::Value {
    let mut plan = json!({
        "stages": [
            {
                "name": "about",
                "act": "find-about-anywhere",
                "intention": { "query": "vector space" }
            }
        ],
        "outcome": { "returns": [ { "stage": "about", "with": [] } ] }
    });
    plan["stages"][0]["intention"]["embedding"] = json!(vec![0.1f32; 768]);
    plan
}

/// The valid composition answers with `returned` keyed by stage name and a trace
/// covering the stage — arms keyed separately, never merged into one ordered list.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_valid_composition_answers_arms_keyed_by_stage_with_a_trace(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    ingest_searchable(
        &app,
        "Vector space probe",
        "vector-probe",
        "Embedding proximity across the corpus.",
    )
    .await;

    let res = run_query(&svc, &parts, json!({ "plan": answerable_plan() }))
        .await
        .expect("the composition runs");
    let v = one_text(&res);

    let returned = v["returned"].as_object().expect("returned is a map");
    assert_eq!(returned.len(), 1, "one entry per outcome.returns: {v}");
    assert!(
        returned.contains_key("about"),
        "arms are keyed by stage name: {v}"
    );
    let trace_stages = v["trace"]["stages"].as_array().expect("trace stages");
    assert!(
        trace_stages.iter().any(|s| s["stage"] == "about"),
        "the trace covers every declared stage: {v}"
    );
}

/// `trace: false` strips the trace client-side: the returned arms stay, the trace
/// key is gone.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn trace_false_strips_the_trace_key(pool: PgPool) {
    let (app, svc, parts) = harness(pool).await;
    ingest_searchable(
        &app,
        "Vector space probe",
        "vector-probe",
        "Embedding proximity across the corpus.",
    )
    .await;

    let res = run_query(
        &svc,
        &parts,
        json!({ "plan": answerable_plan(), "trace": false }),
    )
    .await
    .expect("the composition runs");
    let v = one_text(&res);

    assert!(
        v["returned"]
            .as_object()
            .expect("returned is a map")
            .contains_key("about"),
        "the arms stay: {v}"
    );
    assert!(
        v.get("trace").is_none(),
        "the trace key is stripped, not emptied: {v}"
    );
}

/// Two independently unrunnable stages: the refusal carries EVERY fault at once —
/// each naming its stage and its reason — so the plan is repaired in one round trip.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_refused_plan_carries_every_refusal_verbatim(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;

    let plan = json!({
        "stages": [
            { "name": "one", "act": "find-about-anywhere" },
            { "name": "two", "act": "find-about-anywhere" }
        ],
        "outcome": { "returns": [
            { "stage": "one", "with": [] },
            { "stage": "two", "with": [] }
        ] }
    });
    let err = run_query(&svc, &parts, json!({ "plan": plan }))
        .await
        .expect_err("a plan without questions is refused");
    assert_eq!(
        code_of(&err),
        -32602,
        "a refused plan is a caller error, not a fault: {err}"
    );
    let msg = err.message.as_ref();
    assert!(msg.contains("stage 'one'"), "names the first stage: {msg}");
    assert!(msg.contains("stage 'two'"), "names the second stage: {msg}");
    assert!(
        msg.matches("MissingIntention").count() >= 2,
        "every refusal's reason rides, not just the first: {msg}"
    );
    assert!(
        msg.lines().count() >= 2,
        "one rendered refusal per fault: {msg}"
    );
}

/// A composition-level refusal (no stage) still renders, and never grows an empty
/// `stage ''` prefix.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_composition_level_refusal_omits_the_empty_stage_prefix(pool: PgPool) {
    let (_app, svc, parts) = harness(pool).await;

    let plan = json!({
        "stages": [],
        "outcome": { "returns": [] }
    });
    let err = run_query(&svc, &parts, json!({ "plan": plan }))
        .await
        .expect_err("an empty outcome is refused");
    assert_eq!(code_of(&err), -32602, "{err}");
    let msg = err.message.as_ref();
    assert!(
        !msg.contains("stage ''"),
        "no empty stage prefix on a composition-level refusal: {msg}"
    );
    assert!(msg.contains("NoReturns"), "the reason is named: {msg}");
}

// ── query: the shared refusal arms ──────────────────────────────────

/// System access on the query door, pre-swap: the same in-process gate, the same
/// five-field terminal sentence.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn query_denied_standing_speaks_the_system_access_arm(pool: PgPool) {
    let (app, svc, _parts) = harness(pool).await;

    sqlx::query(
        "INSERT INTO kb_principal_standing (profile_id, state)
         SELECT id, 'denied' FROM kb_profiles WHERE email = $1
         ON CONFLICT (profile_id) DO UPDATE SET state = 'denied'",
    )
    .bind(parity::EMAIL)
    .execute(&app.pool)
    .await
    .expect("deny the principal's standing");

    let parts = claimed_parts(&app.token, Some(parity::EMAIL), 3600);
    let err = svc
        .ensure_profile_from_parts(&parts)
        .await
        .expect_err("a denied principal does not pass the gate");
    assert_eq!(code_of(&err), -32600, "terminal, never retryable: {err}");
    assert!(
        err.message.starts_with(
            "Access to this temper instance requires approval for e2e@test.example.com — "
        ),
        "the sentence names the identity: {err}"
    );
}
