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

/// An Ed25519 keypair for signing test attestations. Same pair `broker::vercel_connect`'s own
/// tests use — the algorithm differs from production RS256, but `JwksKeyStore::with_static_key`
/// carries the algorithm with the key, so the verifier under test is the deployed one.
const ED_PRIV: &str = "-----BEGIN PRIVATE KEY-----\n\
    MC4CAQAwBQYDK2VwBCIEIMBUy9dWl8ECx1v9KN+aoEl/fI80u7Qcv9F8OTVxWW0G\n\
    -----END PRIVATE KEY-----\n";
const ED_PUB: &str = "-----BEGIN PUBLIC KEY-----\n\
    MCowBQYDK2VwAyEAcCE6sWGL6rcfOATmlUSiuWLQAl+hpPAPp/aTR1yxqdc=\n\
    -----END PUBLIC KEY-----\n";

/// The **real** Vercel Connect adapter with a static key — so every request in this suite runs the
/// production RS256 + claim gate (issuer, audience, the anti-decoy `client_id`, the signed
/// `trigger` claim), not a test-only relaxation of it.
fn real_broker() -> Arc<VercelConnectBroker> {
    let dec = DecodingKey::from_ed_pem(ED_PUB.as_bytes()).expect("ed pub");
    Arc::new(VercelConnectBroker::with_jwks(
        JwksKeyStore::with_static_key(dec, Algorithm::EdDSA),
        VercelConnectConfig {
            access_token: "unused".into(),
            project_id: "prj_test".into(),
            team_id: "team_test".into(),
            team_slug: TEAM_SLUG.into(),
        },
    ))
}

/// A genuine Connect attestation for `CONNECTOR_UID`: the claim set captured from five real
/// forwards (research `019f62e6`), signed.
fn attestation(connector_uid: &str) -> String {
    let claims = serde_json::json!({
        "iss": format!("https://oidc.vercel.com/{TEAM_SLUG}"),
        "aud": format!("https://vercel.com/{TEAM_SLUG}"),
        "sub": format!("owner:{TEAM_SLUG}:project:temper-api:environment:production"),
        "client_id": "api-connex",
        "exp": 9_999_999_999_i64,
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

/// A body between axum's 2 MB default and GitHub's 25 MB ceiling is accepted. That window was the
/// gap: GitHub believed it had delivered while temper refused, and nothing in temper-api set
/// `DefaultBodyLimit` at all.
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
