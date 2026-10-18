#![cfg(feature = "test-db")]
//! The intake transport, end to end over HTTP (S3 of "external systems as subscribed emitters").
//!
//! These go through the **router**, not through `intake_service`. Chunk B and chunk C already
//! witness the matching and the projection at the service level; what was never witnessed — and
//! what S3 exists to supply — is that a request arriving on a socket authenticates, resolves to a
//! connection, and lands. A test that calls the service proves none of that.
//!
//! `cargo nextest run -p temper-api --features test-db --test webhook_intake_test`

mod common;

use axum::http::StatusCode;
use jsonwebtoken::{encode, Algorithm, DecodingKey, EncodingKey, Header};
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use temper_core::types::subscription::SubscriptionSelector;
use temper_services::broker::{FakeBroker, VercelConnectBroker, VercelConnectConfig};
use temper_services::services::subscription_test_support::{
    attach_credential_with_connector, create_subscription, github_pr_payload, grant_reach,
    seed_admin, seed_connection, seed_team, GITHUB_REPO,
};
use temper_services::state::JwksKeyStore;

const INTAKE_PATH: &str = "/api/intake/webhook";
const TEAM_SLUG: &str = "acme";
const CONNECTOR_UID: &str = "github/acme-temper";
/// The receiving project — the one every genuine attestation below names.
const PROJECT_ID: &str = "prj_test";

/// An Ed25519 keypair for signing test attestations. Same pair `broker::vercel_connect`'s own
/// tests use — the algorithm differs from production RS256, but `JwksKeyStore::with_static_key`
/// carries the algorithm with the key, so the verifier under test is the deployed one.
#[rustfmt::skip]
const ED_PRIV: &str = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIMBUy9dWl8ECx1v9KN+aoEl/fI80u7Qcv9F8OTVxWW0G\n-----END PRIVATE KEY-----\n"; // gitleaks:allow — inline test keypair, no production trust relationship
const ED_PUB: &str = "-----BEGIN PUBLIC KEY-----\n\
    MCowBQYDK2VwAyEAcCE6sWGL6rcfOATmlUSiuWLQAl+hpPAPp/aTR1yxqdc=\n\
    -----END PUBLIC KEY-----\n";

/// The **real** Vercel Connect adapter with a static key — so every request in this suite runs the
/// production RS256 + claim gate (issuer, audience, the anti-decoy `client_id`, the signed
/// `trigger` claim), not a test-only relaxation of it.
fn real_broker() -> Arc<VercelConnectBroker> {
    broker_with_binding(false)
}

/// [`real_broker`] with the attestation binding enforced: a repeated attestation, or one issued to
/// another project, is refused rather than reported.
fn enforcing_broker() -> Arc<VercelConnectBroker> {
    broker_with_binding(true)
}

fn broker_with_binding(enforce_attestation_binding: bool) -> Arc<VercelConnectBroker> {
    let dec = DecodingKey::from_ed_pem(ED_PUB.as_bytes()).expect("ed pub");
    Arc::new(VercelConnectBroker::with_jwks(
        JwksKeyStore::with_static_key(dec, Algorithm::EdDSA),
        VercelConnectConfig {
            access_token: "unused".into(),
            project_id: PROJECT_ID.into(),
            team_id: "team_test".into(),
            team_slug: TEAM_SLUG.into(),
            enforce_attestation_binding,
        },
    ))
}

/// A genuine Connect attestation for `CONNECTOR_UID`: the claim set captured from five real
/// forwards (research `019f62e6`), signed.
fn attestation(connector_uid: &str) -> String {
    attestation_issued(connector_uid, PROJECT_ID, 1_750_000_000)
}

/// An attestation for `connector_uid` issued to `project_id` at `iat` — a second forward is a second
/// attestation, which differs from the first in `iat` if in nothing else.
fn attestation_issued(connector_uid: &str, project_id: &str, iat: i64) -> String {
    attestation_full(connector_uid, project_id, iat, 9_999_999_999)
}

/// An attestation with every varying claim chosen, including when it expires.
fn attestation_full(connector_uid: &str, project_id: &str, iat: i64, exp: i64) -> String {
    let claims = serde_json::json!({
        "iss": format!("https://oidc.vercel.com/{TEAM_SLUG}"),
        "aud": format!("https://vercel.com/{TEAM_SLUG}"),
        "sub": format!("owner:{TEAM_SLUG}:project:temper-api:environment:production"),
        "project_id": project_id,
        "client_id": "api-connex",
        "iat": iat,
        "exp": exp,
        "trigger": {
            "id": "scl_test",
            "uid": connector_uid,
            "type": "github",
            "service": "github",
        }
    });
    let enc = EncodingKey::from_ed_pem(ED_PRIV.as_bytes()).expect("ed priv");
    let token = encode(&Header::new(Algorithm::EdDSA), &claims, &enc).expect("sign");
    format!("Bearer {token}")
}

/// The world every test needs: an admin, a team, a live credentialed connection whose connector is
/// the one the attestation names, and — when `selector` is given — one subscription against it.
async fn seed_world(pool: &PgPool, selector: Option<SubscriptionSelector>) -> (Uuid, Uuid) {
    let admin = seed_admin(pool).await;
    let team = seed_team(pool, admin).await;
    let conn = seed_connection(pool, Some(team), admin).await;
    grant_reach(pool, admin, conn, team).await;
    attach_credential_with_connector(pool, conn, CONNECTOR_UID).await;
    if let Some(selector) = selector {
        create_subscription(pool, admin, "kb_teams", team, team, conn, selector).await;
    }
    (team, conn)
}

fn pr_selector() -> SubscriptionSelector {
    SubscriptionSelector::GitHubRepository {
        repo: GITHUB_REPO.into(),
        event_types: vec!["pull_request".into()],
    }
}

async fn webhook_event_count(pool: &PgPool, connection: Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_events e
           JOIN kb_connections c ON c.emitter_entity_id = e.emitter_entity_id
          WHERE c.id = $1",
    )
    .bind(connection)
    .fetch_one(pool)
    .await
    .expect("count events")
}

// ── the acceptance path ─────────────────────────────────────────────────────

/// The whole of S3 in one assertion: a real GitHub `pull_request` payload, posted over HTTP with a
/// valid attestation, lands as exactly one `kb_events` row carrying its `touches` fan, with its
/// delivery row projected — through the router, not through a service call.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_real_webhook_lands_as_one_event_with_its_fan_and_deliveries(pool: PgPool) {
    let (_team, conn) = seed_world(&pool, Some(pr_selector())).await;
    let app = common::setup_test_app_with_state(pool.clone(), |s| s.broker = real_broker()).await;

    let res = app
        .client
        .post(app.url(INTAKE_PATH))
        .header("authorization", attestation(CONNECTOR_UID))
        .header("x-github-event", "pull_request")
        .header("content-type", "application/json")
        .body(github_pr_payload(GITHUB_REPO).to_string())
        .send()
        .await
        .expect("post webhook");

    assert_eq!(
        res.status(),
        StatusCode::ACCEPTED,
        "a verified receipt acks"
    );
    let event_id: Uuid = res
        .json::<serde_json::Value>()
        .await
        .expect("json")
        .get("event_id")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse().ok())
        .expect("event_id in the ack");

    // C2: one webhook receipt is one ledger event, always.
    assert_eq!(webhook_event_count(&pool, conn).await, 1);

    // C4/C10: the matched subscriber rides `references` as a `touches` entry, written on INSERT.
    // `references` is a reserved word — quoted in the schema (`canonical_schema.sql:477`) and here.
    let refs: serde_json::Value = sqlx::query_scalar::<_, serde_json::Value>(
        r#"SELECT "references" FROM kb_events WHERE id = $1"#,
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .expect("read references");
    let arr = refs.as_array().expect("references is an array");
    assert_eq!(
        arr.len(),
        1,
        "one matched subscription => one touches entry"
    );
    assert_eq!(arr[0].get("rel").and_then(|v| v.as_str()), Some("touches"));

    // Chunk C's projection ran inside the same transaction: the routing is readable (C11).
    let deliveries: i64 = sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_subscription_deliveries WHERE event_id=$1",
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .expect("count deliveries");
    assert_eq!(deliveries, 1, "the matched declaration got a delivery row");

    // The provenance of the steering value is on the row, not inferable from it.
    let meta: serde_json::Value =
        sqlx::query_scalar::<_, serde_json::Value>("SELECT metadata FROM kb_events WHERE id=$1")
            .bind(event_id)
            .fetch_one(&pool)
            .await
            .expect("read metadata");
    assert_eq!(
        meta.get("provider_event_type").and_then(|v| v.as_str()),
        Some("pull_request")
    );
    assert_eq!(
        meta.get("provider_event_type_source")
            .and_then(|v| v.as_str()),
        Some("header"),
        "the ledger row must say where its own routing input came from"
    );
}

/// **The trap.** A payload matching zero subscriptions is ACKED. The empty radius is the noise
/// filter (C4); a non-2xx here would make GitHub retry a payload temper deliberately routed
/// nowhere, forever, to the same conclusion.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_payload_matching_nothing_is_acked_and_still_stored(pool: PgPool) {
    // A live connection with NO subscriptions at all.
    let (_team, conn) = seed_world(&pool, None).await;
    let app = common::setup_test_app_with_state(pool.clone(), |s| s.broker = real_broker()).await;

    let res = app
        .client
        .post(app.url(INTAKE_PATH))
        .header("authorization", attestation(CONNECTOR_UID))
        .header("x-github-event", "pull_request")
        .body(github_pr_payload(GITHUB_REPO).to_string())
        .send()
        .await
        .expect("post webhook");

    assert_eq!(
        res.status(),
        StatusCode::ACCEPTED,
        "routed nowhere is a well-formed act the system said no to, not an error"
    );
    assert_eq!(
        webhook_event_count(&pool, conn).await,
        1,
        "stored, routes nowhere — the ledger keeps the refusal"
    );
}

// ── the refusal face ────────────────────────────────────────────────────────

/// An unverifiable attestation and a verified attestation naming an unknown connector must be
/// **indistinguishable** to the sender. Asserted on the bytes, not merely the status: a differing
/// message is an existence oracle over which connectors this instance has provisioned.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_refusal_leaks_nothing_about_whether_the_connector_exists(pool: PgPool) {
    seed_world(&pool, Some(pr_selector())).await;
    let app = common::setup_test_app_with_state(pool.clone(), |s| s.broker = real_broker()).await;

    let post = |authorization: String| {
        let app = &app;
        async move {
            app.client
                .post(app.url(INTAKE_PATH))
                .header("authorization", authorization)
                .header("x-github-event", "pull_request")
                .body(github_pr_payload(GITHUB_REPO).to_string())
                .send()
                .await
                .expect("post webhook")
        }
    };

    // (a) A token signed by nobody — the attestation does not verify.
    let bad = post("Bearer not-a-jwt".to_string()).await;
    let bad_status = bad.status();
    let bad_body = bad.text().await.expect("body");

    // (b) A genuine, correctly-signed attestation for a connector no connection carries.
    let unknown = post(attestation("github/not-provisioned-here")).await;
    let unknown_status = unknown.status();
    let unknown_body = unknown.text().await.expect("body");

    assert_eq!(bad_status, StatusCode::UNAUTHORIZED);
    assert_eq!(unknown_status, StatusCode::UNAUTHORIZED);
    assert_eq!(
        bad_body, unknown_body,
        "'this did not verify' and 'this connector is unknown' must be one answer"
    );
}

/// A missing `Authorization` header is refused identically. The ambient `x-vercel-oidc-token` is
/// present on every real inbound request and must never be mistaken for the attestation.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unattested_request_is_refused(pool: PgPool) {
    let (_team, conn) = seed_world(&pool, Some(pr_selector())).await;
    let app = common::setup_test_app_with_state(pool.clone(), |s| s.broker = real_broker()).await;

    let res = app
        .client
        .post(app.url(INTAKE_PATH))
        .header("x-github-event", "pull_request")
        .body(github_pr_payload(GITHUB_REPO).to_string())
        .send()
        .await
        .expect("post webhook");

    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        webhook_event_count(&pool, conn).await,
        0,
        "nothing is appended for a request that never authenticated"
    );
}

// ── C3: no egress at receipt, witnessed at the transport ────────────────────

/// Goal C3: receipt produces no egress to the remote. The broker's `mint` is the only path by
/// which intake could reach one, so a mint counter over a real request is the witness — reading
/// the service and observing that it calls nothing is not.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_receipt_performs_no_egress(pool: PgPool) {
    let (_team, conn) = seed_world(&pool, Some(pr_selector())).await;
    let broker = FakeBroker::accepting_inbound("github", CONNECTOR_UID, "scl_test");
    let probe = broker.clone();
    let app =
        common::setup_test_app_with_state(pool.clone(), |s| s.broker = Arc::new(broker)).await;

    let res = app
        .client
        .post(app.url(INTAKE_PATH))
        .header("authorization", "Bearer anything-the-fake-accepts")
        .header("x-github-event", "pull_request")
        .body(github_pr_payload(GITHUB_REPO).to_string())
        .send()
        .await
        .expect("post webhook");

    assert_eq!(res.status(), StatusCode::ACCEPTED);
    assert_eq!(
        webhook_event_count(&pool, conn).await,
        1,
        "the receipt did land — so zero mints is a fact about a real receipt, not a no-op"
    );
    assert_eq!(
        probe.mint_calls(),
        0,
        "receipt must reach the remote zero times (goal C3)"
    );
}

// ── the decided semantics ───────────────────────────────────────────────────

/// Decided 2026-08-19: a redelivery is a **second receipt**, recorded. GitHub redelivers on
/// non-2xx and offers manual redelivery; suppressing the repeat would mean trusting an unsigned,
/// unwitnessed `X-GitHub-Delivery` header to decide whether an event exists at all.
///
/// This sends one attestation twice, under the default broker, which reports a repeat rather than
/// refusing it. Enforced, a repeat inside its burst window still lands while one after it is
/// refused (`an_enforced_binding_lands_a_burst_and_refuses_a_later_replay`), and a fresh
/// attestation is always a second receipt
/// (`an_enforced_binding_still_records_a_freshly_attested_redelivery`).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_redelivery_is_a_second_receipt(pool: PgPool) {
    let (_team, conn) = seed_world(&pool, Some(pr_selector())).await;
    let app = common::setup_test_app_with_state(pool.clone(), |s| s.broker = real_broker()).await;

    for _ in 0..2 {
        let res = app
            .client
            .post(app.url(INTAKE_PATH))
            .header("authorization", attestation(CONNECTOR_UID))
            .header("x-github-event", "pull_request")
            .body(github_pr_payload(GITHUB_REPO).to_string())
            .send()
            .await
            .expect("post webhook");
        assert_eq!(res.status(), StatusCode::ACCEPTED);
    }

    assert_eq!(
        webhook_event_count(&pool, conn).await,
        2,
        "two receipts are two acts; the ledger records acts"
    );
}

/// The provider stated an event-name convention and the request carried none. temper cannot
/// compute the radius from an input it does not have, so it FAILS rather than landing an event
/// whose empty fan is indistinguishable from a correct one. This is the branch that makes the
/// unwitnessed "does Connect forward `X-GitHub-Event`?" assumption fail loudly on first contact.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_missing_event_name_fails_rather_than_routing_on_a_guess(pool: PgPool) {
    let (_team, conn) = seed_world(&pool, Some(pr_selector())).await;
    let app = common::setup_test_app_with_state(pool.clone(), |s| s.broker = real_broker()).await;

    let res = app
        .client
        .post(app.url(INTAKE_PATH))
        .header("authorization", attestation(CONNECTOR_UID))
        // no x-github-event
        .body(github_pr_payload(GITHUB_REPO).to_string())
        .send()
        .await
        .expect("post webhook");

    assert_eq!(
        res.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "unable to act is a failure, not a refusal — and a retry is the right response to it"
    );
    assert_eq!(
        webhook_event_count(&pool, conn).await,
        0,
        "nothing may land whose radius temper knows it could not compute"
    );
}

/// A body between the shared 2 MiB ceiling and GitHub's 25 MB ceiling is accepted. That window was
/// the gap: GitHub believed it had delivered while temper refused, and nothing in temper-api set
/// `DefaultBodyLimit` at all.
///
/// **It now witnesses a second thing.** `apply_base_layers` sets an explicit
/// `MAX_REQUEST_BODY_BYTES` of 2 MiB across both public surfaces, so this route's own limit is no
/// longer merely larger than a framework default — it has to *win* against a limit temper applies
/// deliberately. It does because a limit layered onto a single route runs inside the router-wide
/// one. If that ever stopped holding, this is the test that says so.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_payload_above_axums_default_limit_is_accepted(pool: PgPool) {
    let (_team, conn) = seed_world(&pool, Some(pr_selector())).await;
    let app = common::setup_test_app_with_state(pool.clone(), |s| s.broker = real_broker()).await;

    // A real-shaped payload padded past 2 MB — 3 MB is above the old ceiling and well under 25.
    let mut payload = github_pr_payload(GITHUB_REPO);
    payload["_padding"] = serde_json::Value::String("x".repeat(3 * 1024 * 1024));

    let res = app
        .client
        .post(app.url(INTAKE_PATH))
        .header("authorization", attestation(CONNECTOR_UID))
        .header("x-github-event", "pull_request")
        .body(payload.to_string())
        .send()
        .await
        .expect("post webhook");

    assert_eq!(
        res.status(),
        StatusCode::ACCEPTED,
        "a 3 MB delivery is one GitHub sends and temper used to drop"
    );
    assert_eq!(webhook_event_count(&pool, conn).await, 1);
}

/// The limit is real. Above GitHub's own ceiling GitHub does not deliver at all, so accepting more
/// would buy nothing.
///
/// **What this asserts and what it does not.** Bracketed with
/// `a_payload_above_axums_default_limit_is_accepted`, the pair pins the ceiling to the window
/// `(3 MB, 26 MB)` — the accept proves the limit is above axum's 2 MB default, this proves it is
/// below 26 MB. Neither pins 25 MB exactly, and this one alone witnesses nothing new: axum's 2 MB
/// default also refuses 26 MB with a 413, so it passes unchanged against the pre-fix code. It is
/// here to stop the ceiling being removed, not to prove it was added.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_payload_above_githubs_ceiling_is_refused(pool: PgPool) {
    let (_team, conn) = seed_world(&pool, Some(pr_selector())).await;
    let app = common::setup_test_app_with_state(pool.clone(), |s| s.broker = real_broker()).await;

    let mut payload = github_pr_payload(GITHUB_REPO);
    payload["_padding"] = serde_json::Value::String("x".repeat(26 * 1024 * 1024));

    let res = app
        .client
        .post(app.url(INTAKE_PATH))
        .header("authorization", attestation(CONNECTOR_UID))
        .header("x-github-event", "pull_request")
        .body(payload.to_string())
        .send()
        .await
        .expect("post webhook");

    assert_eq!(
        res.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "over the ceiling must be refused BY THE LIMIT — asserting merely 'not 202' would pass \
         for any unrelated failure, including one that never reached the limit at all"
    );
    assert_eq!(webhook_event_count(&pool, conn).await, 0);
}

/// Goal C7: no document, no node, no edge is created because a webhook arrived. **Ever.**
///
/// The register recorded this as "vacuously satisfied by absence of the feature" — there was no
/// intake path to test against, so anything at all satisfied it. The path exists now, so the
/// vacuous pass has to become a real one: this counts the curated-graph tables across a receipt
/// that genuinely landed, and a receipt that landed is what makes the zeroes mean something.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_webhook_creates_no_resource_no_node_and_no_edge(pool: PgPool) {
    let (_team, conn) = seed_world(&pool, Some(pr_selector())).await;

    // Seed real curated material FIRST, so the baseline is non-zero. Compared at (0, 0, 0) the
    // equality below would hold for a counter pointed at tables nothing ever writes — it would
    // pass whether or not it could fail, which is the failure mode this whole test exists to
    // leave behind.
    common::seed_context_with_goal_and_tasks(&pool, 2).await;
    let before = curated_graph_counts(&pool).await;
    assert!(
        before.0 > 0 && before.1 > 0,
        "the baseline must be non-zero or the comparison proves nothing: {before:?}"
    );

    let app = common::setup_test_app_with_state(pool.clone(), |s| s.broker = real_broker()).await;
    let res = app
        .client
        .post(app.url(INTAKE_PATH))
        .header("authorization", attestation(CONNECTOR_UID))
        .header("x-github-event", "pull_request")
        .body(github_pr_payload(GITHUB_REPO).to_string())
        .send()
        .await
        .expect("post webhook");
    assert_eq!(res.status(), StatusCode::ACCEPTED);

    // The receipt LANDED. Without this the zeroes below would hold for a request that did
    // nothing at all, and the clause would still be vacuous — just more expensively.
    assert_eq!(webhook_event_count(&pool, conn).await, 1);

    assert_eq!(
        curated_graph_counts(&pool).await,
        before,
        "a webhook becomes one unjudged fact plus a list of who should look at it — never a \
         document, a node, or an edge (goal C7)"
    );
}

/// The curated-graph tables a webhook must never grow.
///
/// There is deliberately no `kb_cogmap_nodes` table to count: a cogmap **node is a
/// `kb_resources` row** with a home in a cogmap, so `kb_resources` + `kb_resource_homes` is what
/// "no document and no node" actually means here. Counting a table that does not exist would
/// have been a zero that proved nothing.
async fn curated_graph_counts(pool: &PgPool) -> (i64, i64, i64) {
    let one = |sql: &'static str| async move {
        sqlx::query_scalar::<_, i64>(sql)
            .fetch_one(pool)
            .await
            .expect("count")
    };
    (
        one("SELECT count(*) FROM kb_resources").await,
        one("SELECT count(*) FROM kb_resource_homes").await,
        one("SELECT count(*) FROM kb_edges").await,
    )
}

// ── single use and the project binding (enforced) ───────────────────────────

/// Post one PR webhook under `authorization`, with or without the event-name header.
async fn post_pr(app: &common::TestApp, authorization: &str, event_header: bool) -> StatusCode {
    let mut req = app
        .client
        .post(app.url(INTAKE_PATH))
        .header("authorization", authorization);
    if event_header {
        req = req.header("x-github-event", "pull_request");
    }
    req.body(github_pr_payload(GITHUB_REPO).to_string())
        .send()
        .await
        .expect("post webhook")
        .status()
}

/// Move every recorded attestation's first acceptance a minute into the past, past the burst
/// window, as if the receipts that recorded them had arrived then.
async fn age_recorded_attestations(pool: &PgPool) {
    sqlx::query(
        "UPDATE kb_inbound_attestations_seen SET first_seen_at = first_seen_at - interval '1 minute'",
    )
    .execute(pool)
    .await
    .expect("age attestations");
}

/// Enforced, the same attestation again straight away is one of Connect's bursts (a queue merge's
/// `closed` and `dequeued`, distinct events under one attestation) and lands. Presented after the
/// burst window, it is a capture replayed with any body: refused with the uniform 401, landing
/// nothing.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_enforced_binding_lands_a_burst_and_refuses_a_later_replay(pool: PgPool) {
    let (_team, conn) = seed_world(&pool, Some(pr_selector())).await;
    let app =
        common::setup_test_app_with_state(pool.clone(), |s| s.broker = enforcing_broker()).await;
    let authz = attestation(CONNECTOR_UID);

    assert_eq!(post_pr(&app, &authz, true).await, StatusCode::ACCEPTED);
    assert_eq!(
        post_pr(&app, &authz, true).await,
        StatusCode::ACCEPTED,
        "a repeat inside the burst window lands"
    );
    age_recorded_attestations(&pool).await;
    assert_eq!(
        post_pr(&app, &authz, true).await,
        StatusCode::UNAUTHORIZED,
        "the same attestation after the window must be refused"
    );
    assert_eq!(webhook_event_count(&pool, conn).await, 2);
}

/// Enforced, a redelivery that arrives under a fresh attestation is still a second receipt — the
/// 2026-08-19 decision holds for every forward Connect actually attests.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_enforced_binding_still_records_a_freshly_attested_redelivery(pool: PgPool) {
    let (_team, conn) = seed_world(&pool, Some(pr_selector())).await;
    let app =
        common::setup_test_app_with_state(pool.clone(), |s| s.broker = enforcing_broker()).await;

    for iat in [1_750_000_000, 1_750_000_001] {
        let authz = attestation_issued(CONNECTOR_UID, PROJECT_ID, iat);
        assert_eq!(post_pr(&app, &authz, true).await, StatusCode::ACCEPTED);
    }
    assert_eq!(webhook_event_count(&pool, conn).await, 2);
}

/// A receipt temper failed to record does not spend its attestation: the redelivery GitHub makes
/// after the 500 is accepted, not refused as a replay.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_failed_receipt_does_not_spend_its_attestation(pool: PgPool) {
    let (_team, conn) = seed_world(&pool, Some(pr_selector())).await;
    let app =
        common::setup_test_app_with_state(pool.clone(), |s| s.broker = enforcing_broker()).await;
    let authz = attestation(CONNECTOR_UID);

    assert_eq!(
        post_pr(&app, &authz, false).await,
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(
        post_pr(&app, &authz, true).await,
        StatusCode::ACCEPTED,
        "the retry of an unrecorded receipt is not a replay"
    );
    assert_eq!(webhook_event_count(&pool, conn).await, 1);
}

/// Enforced, an attestation Connect issued to a sibling project in the same team is refused with the
/// same 401 and body as every other refusal, so it is no oracle.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_enforced_binding_refuses_another_projects_attestation(pool: PgPool) {
    let (_team, conn) = seed_world(&pool, Some(pr_selector())).await;
    let app =
        common::setup_test_app_with_state(pool.clone(), |s| s.broker = enforcing_broker()).await;
    let sibling = attestation_issued(CONNECTOR_UID, "prj_sibling", 1_750_000_000);

    let res = app
        .client
        .post(app.url(INTAKE_PATH))
        .header("authorization", sibling)
        .header("x-github-event", "pull_request")
        .body(github_pr_payload(GITHUB_REPO).to_string())
        .send()
        .await
        .expect("post webhook");
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let sibling_body = res.text().await.expect("body");
    let forged_body = app
        .client
        .post(app.url(INTAKE_PATH))
        .header("authorization", "Bearer not-a-jwt")
        .header("x-github-event", "pull_request")
        .body(github_pr_payload(GITHUB_REPO).to_string())
        .send()
        .await
        .expect("post forged")
        .text()
        .await
        .expect("body");
    assert_eq!(sibling_body, forged_body, "one answer for every refusal");
    assert_eq!(webhook_event_count(&pool, conn).await, 0);
}

/// An attestation still verifies for the verifier's clock-skew leeway past its `exp`, so it must be
/// remembered at least that long. Here one already past `exp` — accepted on leeway — is presented,
/// another receipt's claim runs the prune, and the first is presented again after its burst
/// window: still refused.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_enforced_binding_remembers_an_attestation_through_its_leeway(pool: PgPool) {
    let (_team, conn) = seed_world(&pool, Some(pr_selector())).await;
    let app =
        common::setup_test_app_with_state(pool.clone(), |s| s.broker = enforcing_broker()).await;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_secs() as i64;
    let expired_on_leeway = attestation_full(CONNECTOR_UID, PROJECT_ID, now - 600, now - 20);

    assert_eq!(
        post_pr(&app, &expired_on_leeway, true).await,
        StatusCode::ACCEPTED,
        "still inside the verifier's leeway"
    );
    let other = attestation_issued(CONNECTOR_UID, PROJECT_ID, now);
    assert_eq!(post_pr(&app, &other, true).await, StatusCode::ACCEPTED);
    age_recorded_attestations(&pool).await;
    assert_eq!(
        post_pr(&app, &expired_on_leeway, true).await,
        StatusCode::UNAUTHORIZED,
        "pruned at exp, the replay would claim the attestation afresh"
    );
    assert_eq!(webhook_event_count(&pool, conn).await, 2);
}

// ── GitHub's own signature, observed ────────────────────────────────────────

/// Collects every line the server logs while a guard is held.
#[derive(Clone, Default)]
struct Capture(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl Capture {
    fn contents(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("poisoned")).into_owned()
    }
}

impl std::io::Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("poisoned").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'w> tracing_subscriber::fmt::MakeWriter<'w> for Capture {
    type Writer = Self;
    fn make_writer(&'w self) -> Self::Writer {
        self.clone()
    }
}

/// The `hub_signature` value of every line in `log`, in order.
fn hub_signatures(log: &str) -> Vec<String> {
    log.lines()
        .filter_map(|line| line.split("hub_signature=").nth(1))
        .map(|rest| {
            rest.split_whitespace()
                .next()
                .unwrap_or_default()
                .trim_matches('"')
                .to_string()
        })
        .collect()
}

/// GitHub's `X-Hub-Signature-256` value for `body` under `secret`.
fn hub_signature_for(secret: &str, body: &[u8]) -> String {
    use hmac::{Hmac, KeyInit, Mac};
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(secret.as_bytes()).expect("hmac key");
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

const WEBHOOK_SECRET: &str = "the-github-apps-webhook-secret";

/// Post one PR webhook under a fresh attestation, with `signature` as `X-Hub-Signature-256`.
async fn post_signed(
    app: &common::TestApp,
    iat: i64,
    body: &str,
    signature: Option<&str>,
) -> StatusCode {
    let mut req = app
        .client
        .post(app.url(INTAKE_PATH))
        .header(
            "authorization",
            attestation_issued(CONNECTOR_UID, PROJECT_ID, iat),
        )
        .header("x-github-event", "pull_request");
    if let Some(signature) = signature {
        req = req.header("x-hub-signature-256", signature);
    }
    req.body(body.to_string())
        .send()
        .await
        .expect("post webhook")
        .status()
}

/// Each verified GitHub receipt logs what its `X-Hub-Signature-256` says, checked against the
/// configured `GITHUB_WEBHOOK_SECRET` over the exact bytes received, and handling is unchanged:
/// valid, invalid and absent all land. The body is pretty-printed JSON, which no serializer
/// reproduces from the parsed value, so a check over re-serialized JSON would log `invalid`. The
/// capture reaches the server's lines for the reason `admin_field_scrub_surface_test` gives: the
/// server polls on this test's thread.
///
/// FAILS IF a receipt is not classified, is classified over anything but the bytes received, or is
/// refused on its signature.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn each_github_receipt_logs_its_signature_and_none_is_refused(pool: PgPool) {
    let (_team, conn) = seed_world(&pool, Some(pr_selector())).await;
    let app = common::setup_test_app_with_state(pool.clone(), |s| {
        s.broker = real_broker();
        std::sync::Arc::make_mut(&mut s.config).github_webhook_secret =
            Some(WEBHOOK_SECRET.to_string());
    })
    .await;
    let body = serde_json::to_string_pretty(&github_pr_payload(GITHUB_REPO)).expect("pretty json");

    let captured = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);

    let valid = hub_signature_for(WEBHOOK_SECRET, body.as_bytes());
    let other_key = hub_signature_for("not-the-apps-secret", body.as_bytes());
    let statuses = [
        post_signed(&app, 1_750_000_001, &body, Some(&valid)).await,
        post_signed(&app, 1_750_000_002, &body, Some(&other_key)).await,
        post_signed(&app, 1_750_000_003, &body, None).await,
    ];
    drop(guard);

    assert_eq!(
        statuses,
        [StatusCode::ACCEPTED; 3],
        "the signature is observed, never enforced"
    );
    assert_eq!(webhook_event_count(&pool, conn).await, 3);
    let log = captured.contents();
    assert_eq!(hub_signatures(&log), ["valid", "invalid", "absent"]);
    assert!(
        !log.contains(WEBHOOK_SECRET),
        "the webhook secret is in a log line"
    );
    assert!(
        !log.contains(&valid["sha256=".len()..]),
        "a signature value is in a log line"
    );
}

/// The line is written before any later step can refuse the receipt, over the raw bytes: a validly
/// signed body that is not JSON logs `valid` and is still refused exactly as before (`400`).
///
/// FAILS IF the line moves after a refusing step, or the MAC is taken over parsed content.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_signed_receipt_refused_downstream_still_logs_its_signature(pool: PgPool) {
    let (_team, conn) = seed_world(&pool, Some(pr_selector())).await;
    let app = common::setup_test_app_with_state(pool.clone(), |s| {
        s.broker = real_broker();
        std::sync::Arc::make_mut(&mut s.config).github_webhook_secret =
            Some(WEBHOOK_SECRET.to_string());
    })
    .await;
    let body = "not json, but signed";

    let captured = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    let status = post_signed(
        &app,
        1_750_000_001,
        body,
        Some(&hub_signature_for(WEBHOOK_SECRET, body.as_bytes())),
    )
    .await;
    drop(guard);

    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a non-JSON body is refused as before"
    );
    assert_eq!(webhook_event_count(&pool, conn).await, 0);
    assert_eq!(hub_signatures(&captured.contents()), ["valid"]);
}

/// Without `GITHUB_WEBHOOK_SECRET`, a signed receipt logs only that the header arrived.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn without_a_secret_a_signed_receipt_logs_present(pool: PgPool) {
    seed_world(&pool, Some(pr_selector())).await;
    let app = common::setup_test_app_with_state(pool.clone(), |s| s.broker = real_broker()).await;
    let body = github_pr_payload(GITHUB_REPO).to_string();

    let captured = Capture::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_max_level(tracing::Level::INFO)
        .with_ansi(false)
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    let status = post_signed(
        &app,
        1_750_000_001,
        &body,
        Some(&hub_signature_for(WEBHOOK_SECRET, body.as_bytes())),
    )
    .await;
    drop(guard);

    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(hub_signatures(&captured.contents()), ["present"]);
}
