//! A write carrying `If-Match`, `If-None-Match` or `If-Unmodified-Since` is refused with `400`
//! under `PRECONDITION_NOT_HONOURED`, never applied unconditionally, and only a caller the route
//! already admits ever sees that refusal. See `temper_api::middleware::preconditions`.
//!
//! What is witnessed:
//! - **anonymous equivalence**: every caller a route's gate refuses (no token, a garbage one, an
//!   expired one, a deactivated account, an unsigned internal call, an unattested webhook), and
//!   every caller of an exempt or unmatched path, gets exactly the answer it gets without the
//!   header, on every non-safe method: status, body and `Allow`;
//! - **gate order**: an approved person is refused `400` on every `Refused` tier, an unapproved one
//!   meets `403 SYSTEM_ACCESS_REQUIRED` first on the gated tiers, a machine meets the machine
//!   refusal first on the human tiers, a signed internal call (reconcile, Slack mint) is refused
//!   `400`, an unsigned one gets its `401`;
//! - **intake**: an unattested webhook gets its `401`; an attested one with a header is refused
//!   `400` and nothing is written;
//! - every documented write refuses each header for an approved person and writes nothing, the same
//!   writes without the header reach their handlers, and reads ignore the headers;
//! - an authenticated unmounted write keeps the refusal with the fallback's `Allow`, a path shared
//!   by two groups answers the same, and the refusal carries CORS and the security headers.
#![cfg(feature = "test-db")]

mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use reqwest::Method;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::internal_sig::{v2_headers, DOOR_SAML_RECONCILE, DOOR_SLACK_MINT};
use temper_core::types::ingest::IngestPayload;
use temper_services::broker::FakeBroker;
use temper_services::services::subscription_test_support::{
    attach_credential_with_connector, github_pr_payload, grant_reach, seed_admin, seed_connection,
    seed_team, GITHUB_REPO,
};

/// The header values sent. Distinctive, so a refusal that echoed one would be caught.
const IF_MATCH_VALUE: &str = "\"precondition-sentinel-etag\"";
const IF_NONE_MATCH_VALUE: &str = "\"precondition-sentinel-absent\"";
const IF_UNMODIFIED_SINCE_VALUE: &str = "Wed, 21 Oct 2015 07:28:00 GMT";

/// Each policed header, with the value sent.
const POLICED: [(&str, &str); 3] = [
    ("if-match", IF_MATCH_VALUE),
    ("if-none-match", IF_NONE_MATCH_VALUE),
    ("if-unmodified-since", IF_UNMODIFIED_SINCE_VALUE),
];

/// The non-safe methods. Every path below is sent each of them, so each is probed both where the
/// method is mounted and where the method router's fallback answers.
const WRITES: [&str; 4] = ["POST", "PUT", "PATCH", "DELETE"];

const RECONCILE_SECRET: &str = "precondition-reconcile-secret";
const MINT_SECRET: &str = "precondition-mint-secret";
const DISPATCH_SECRET: &str = "precondition-dispatch-secret";
const CONNECTOR_UID: &str = "github/acme-temper";

/// One route per tier and door, `{id}` filled per request. The `Refused` tiers: human self-service
/// (`PATCH /api/profile`, a path the machine-admitting status group shares), gated, the admin
/// surface, the three gated doors with their own body limits, and the three HMAC tiers. Then the
/// doors that are not `Refused`: the intake (`InHandler`), the crons, the Slack callback and health
/// (`Exempt`), and a path nothing mounts.
const PROBED: &[&str] = &[
    "/api/profile",
    "/api/resources/{id}",
    "/api/ingest",
    "/api/access/admin/requests/{id}",
    "/api/blobs",
    "/api/blobs/uploads/{id}/segments",
    "/api/contexts/{id}/shapes",
    "/internal/saml/reconcile",
    "/internal/principal/resolve",
    "/internal/slack/link-state",
    "/internal/slack/mint",
    "/api/intake/webhook",
    "/api/embed/dispatch",
    "/api/embed/warm",
    "/api/auth/slack/callback",
    "/api/health",
    "/api/no-such-route-anywhere",
];

/// A path template with every parameter filled by a fresh UUID, which every path extractor
/// accepts and no row carries.
fn concrete(template: &str) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let close = rest[open..].find('}').expect("closed parameter") + open;
        out.push_str(&rest[..open]);
        out.push_str(&Uuid::now_v7().to_string());
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

fn is_write(method: &str) -> bool {
    !matches!(method, "GET" | "HEAD" | "OPTIONS")
}

/// What the matrix compares: the status, the `Allow` header, and the body's exact bytes.
#[derive(Debug, PartialEq, Eq)]
struct Answer {
    status: u16,
    allow: Option<String>,
    body: String,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

async fn answer_of(response: reqwest::Response) -> Answer {
    let status = response.status().as_u16();
    let allow = response
        .headers()
        .get("allow")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    Answer {
        status,
        allow,
        body: response.text().await.unwrap_or_default(),
    }
}

/// One request: `method path`, an optional `Authorization` value, extra headers, a JSON body for a
/// write.
async fn send(
    app: &common::TestApp,
    method: &str,
    path: &str,
    authorization: Option<&str>,
    headers: &[(&str, &str)],
    body: &Value,
) -> Answer {
    let mut request = app.client.request(
        Method::from_bytes(method.as_bytes()).expect("method"),
        app.url(path),
    );
    if let Some(value) = authorization {
        request = request.header("Authorization", value);
    }
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    if is_write(method) {
        request = request.json(body);
    }
    answer_of(request.send().await.expect("request sent")).await
}

fn bearer(token: &str) -> String {
    format!("Bearer {token}")
}

/// True when `answer` is this layer's refusal, naming `header`, without echoing its value.
fn is_precondition_refusal(answer: &Answer, header: (&str, &str)) -> bool {
    let named = match header.0 {
        "if-match" => "the If-Match header is not honoured on this route",
        "if-none-match" => "the If-None-Match header is not honoured on this route",
        _ => "the If-Unmodified-Since header is not honoured on this route",
    };
    let body = answer.json();
    answer.status == 400
        && body["error"]["code"] == json!(temper_core::error::PRECONDITION_NOT_HONOURED_CODE)
        && body["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains(named))
        && !answer.body.contains(header.1)
}

fn assert_refused(what: &str, header: (&str, &str), answer: &Answer) {
    assert!(
        is_precondition_refusal(answer, header),
        "{what} with {} must be the precondition refusal: {answer:?}",
        header.0
    );
}

/// A digest of every row in every user table, in every schema. Any insert, update or delete
/// changes it.
async fn database_fingerprint(pool: &PgPool) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT md5(string_agg(t.schemaname || '.' || t.tablename || ':' || (xpath('/row/d/text()', \
             query_to_xml(format('SELECT md5(coalesce(string_agg(x::text, %L ORDER BY x::text), %L)) \
                                  AS d FROM %I.%I x', '|', '', t.schemaname, t.tablename), \
                          false, true, '')))[1]::text, ',' ORDER BY t.schemaname, t.tablename)) \
           FROM pg_tables t WHERE t.schemaname NOT IN ('pg_catalog', 'information_schema')",
    )
    .fetch_one(pool)
    .await
    .expect("database fingerprint")
}

/// A person with a home context, approved or not; returns `(token, context_id)`. The fixture
/// approves the profile it creates; an unapproved person is that profile with its standing row
/// removed, which is how a fresh profile is born.
async fn person(pool: &PgPool, approved: bool) -> (String, Uuid) {
    let email = format!("precondition-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(pool, &email).await;
    if !approved {
        sqlx::query("DELETE FROM kb_principal_standing WHERE profile_id = $1")
            .bind(profile_id)
            .execute(pool)
            .await
            .expect("remove standing");
    }
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);
    (token, context_id)
}

/// A validly signed token whose profile's standing is `deactivated`.
async fn deactivated_person(pool: &PgPool) -> String {
    let email = format!("precondition-deactivated-{}@example.com", Uuid::new_v4());
    let (profile_id, _) = common::fixtures::create_test_profile_with_context(pool, &email).await;
    sqlx::query(
        "INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1, 'deactivated') \
         ON CONFLICT (profile_id) DO UPDATE SET state = 'deactivated'",
    )
    .bind(profile_id)
    .execute(pool)
    .await
    .expect("deactivate");
    common::generate_test_jwt(&format!("test|{profile_id}"), &email)
}

/// A registered machine with approved standing; returns its bearer.
async fn approved_machine(pool: &PgPool) -> String {
    let client_id = format!("precondition-machine-{}", Uuid::new_v4().simple());
    let profile = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_profiles (id, handle, display_name, email, preferences) \
         VALUES ($1, $2, $2, NULL, '{}')",
    )
    .bind(profile)
    .bind(&client_id)
    .execute(pool)
    .await
    .expect("seed machine profile");
    sqlx::query(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, registered_by_profile_id) \
         VALUES ($1, 'test', $2, $2)",
    )
    .bind(&client_id)
    .bind(profile)
    .execute(pool)
    .await
    .expect("seed machine registration");
    sqlx::query("INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1, 'approved')")
        .bind(profile)
        .execute(pool)
        .await
        .expect("approve machine");
    common::generate_machine_jwt(&client_id)
}

/// The app every matrix test runs against: the reconcile, Slack mint and cron secrets set, so
/// those gates answer their real refusal rather than a "disabled" one. Slack link is left
/// unconfigured (it needs a vault key), so its signature gate answers "slack link disabled"; the
/// anonymous matrix still compares it, and its signed path is the same `mount_group` code the
/// reconcile and mint probes exercise, held per row by the route-table tests.
async fn configured_app(pool: &PgPool, cron_host: Option<&str>) -> common::TestApp {
    let cron_host = cron_host.map(str::to_string);
    common::setup_test_app_with_config(pool.clone(), move |config| {
        config.internal_reconcile_secret = Some(RECONCILE_SECRET.to_string());
        config.slack_mint_secret = Some(MINT_SECRET.to_string());
        config.embed_dispatch_secret = Some(DISPATCH_SECRET.to_string());
        config.cron_host = cron_host;
    })
    .await
}

/// The four credential states an anonymous caller can present: none, a garbage bearer, an expired
/// signed token, and a validly signed token for a deactivated account.
async fn anonymous_credentials(pool: &PgPool) -> Vec<(&'static str, Option<String>)> {
    let expired = common::generate_expired_jwt("test|precondition-expired", "expired@example.com");
    vec![
        ("none", None),
        ("garbage", Some(bearer("not-a-token"))),
        ("expired", Some(bearer(&expired))),
        ("deactivated", Some(bearer(&deactivated_person(pool).await))),
    ]
}

/// Send every probed path, with every write method and every credential, once without a header and
/// once with each policed header; the answers must be identical. Returns how many pairs were
/// compared.
async fn assert_anonymous_equivalence(
    app: &common::TestApp,
    paths: &[&str],
    credentials: &[(&'static str, Option<String>)],
) -> usize {
    let body = json!({});
    let mut compared = 0;
    for template in paths {
        let path = concrete(template);
        for method in WRITES {
            for (state, authorization) in credentials {
                let plain = send(app, method, &path, authorization.as_deref(), &[], &body).await;
                for header in POLICED {
                    let guarded = send(
                        app,
                        method,
                        &path,
                        authorization.as_deref(),
                        &[header],
                        &body,
                    )
                    .await;
                    assert_eq!(
                        guarded, plain,
                        "{method} {template} as {state}: with {} the answer differs from the same \
                         request without it",
                        header.0
                    );
                    compared += 1;
                }
            }
        }
    }
    compared
}

// FAILS IF: an unauthenticated caller (no token, a garbage or expired one, a deactivated account),
// or any caller of an exempt or unmatched path, gets a different status, body or `Allow` because it
// sent a policed header: the layer moved back before authentication, or a self-gated or public row
// was marked `Refused`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_anonymous_caller_gets_exactly_its_answer_without_the_header(pool: PgPool) {
    let app = configured_app(&pool, None).await;
    let credentials = anonymous_credentials(&pool).await;
    let compared = assert_anonymous_equivalence(&app, PROBED, &credentials).await;
    assert_eq!(
        compared,
        PROBED.len() * WRITES.len() * credentials.len() * 3
    );

    // The comparison is only worth something if the gates answered: every Refused tier refused
    // this caller, rather than (say) every request failing the same way for an unrelated reason.
    let path = concrete("/api/resources/{id}");
    for (state, authorization) in &credentials {
        let answer = send(
            &app,
            "PATCH",
            &path,
            authorization.as_deref(),
            &[],
            &json!({}),
        )
        .await;
        assert_eq!(answer.status, 401, "{state}: {answer:?}");
    }
}

// FAILS IF: on the public host, the cron host pin's masked 404 stops being the answer to a
// header-bearing request (the pre-#1360 answer), on any method.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_cron_host_pin_answers_first_with_or_without_the_header(pool: PgPool) {
    let app = configured_app(&pool, Some("cron.internal.test")).await;
    let credentials = anonymous_credentials(&pool).await;
    assert_anonymous_equivalence(
        &app,
        &["/api/embed/dispatch", "/api/embed/warm", "/api/as/reap"],
        &credentials,
    )
    .await;
    let masked = send(
        &app,
        "POST",
        "/api/embed/dispatch",
        None,
        &[POLICED[0]],
        &json!({}),
    )
    .await;
    assert_eq!(masked.status, 404, "the host pin answers: {masked:?}");
}

/// The `{id}` route each `Refused` tier is probed on, with the write method it mounts there.
const REFUSED_TIER_WRITES: &[(&str, &str, &str)] = &[
    ("HumanAuthOnly", "PATCH", "/api/profile"),
    ("Gated", "PATCH", "/api/resources/{id}"),
    ("Gated", "POST", "/api/ingest"),
    ("HumanGated", "PATCH", "/api/access/admin/requests/{id}"),
    ("Gated (blob commit door)", "POST", "/api/blobs"),
    (
        "Gated (blob segment door)",
        "POST",
        "/api/blobs/uploads/{id}/segments",
    ),
    ("Gated (shape declare)", "POST", "/api/contexts/{id}/shapes"),
];

fn is_gated(tier: &str) -> bool {
    tier.starts_with("Gated") || tier == "HumanGated"
}

fn is_human_tier(tier: &str) -> bool {
    tier.starts_with("Human")
}

// FAILS IF: the layer is dropped from a tier, or runs before `require_system_access` or
// `refuse_machine`: an approved person must be refused 400 on every tier; an unapproved one must
// meet the system-access refusal first where the tier has one; a machine must meet the machine
// refusal first on the human tiers.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn every_refused_tier_refuses_only_after_its_gates(pool: PgPool) {
    let app = configured_app(&pool, None).await;
    let (approved, _) = person(&pool, true).await;
    let (unapproved, _) = person(&pool, false).await;
    let machine = approved_machine(&pool).await;
    let body = json!({});

    for (tier, method, template) in REFUSED_TIER_WRITES {
        let path = concrete(template);
        let route = format!("{tier}: {method} {template}");
        for header in POLICED {
            let answer = send(
                &app,
                method,
                &path,
                Some(&bearer(&approved)),
                &[header],
                &body,
            )
            .await;
            assert_refused(&format!("{route} as an approved person"), header, &answer);

            // The unapproved person and the machine: the header changes nothing about the answer
            // wherever a gate refuses them, and that answer is the gate's own.
            for (who, token) in [
                ("an unapproved person", &unapproved),
                ("a machine", &machine),
            ] {
                let plain = send(&app, method, &path, Some(&bearer(token)), &[], &body).await;
                let guarded =
                    send(&app, method, &path, Some(&bearer(token)), &[header], &body).await;
                let refused_by_gate = match who {
                    "a machine" => is_human_tier(tier),
                    _ => is_gated(tier),
                };
                if !refused_by_gate {
                    // Admitted by every gate this tier has, so refused for the header.
                    assert_refused(&format!("{route} as {who}"), header, &guarded);
                    continue;
                }
                assert_eq!(guarded, plain, "{route} as {who} with {}", header.0);
                let code = plain.json()["error"]["code"].clone();
                match who {
                    "a machine" => assert_eq!(
                        (plain.status, code),
                        (403, json!(temper_core::error::FORBIDDEN_DETAIL_CODE)),
                        "{route}: the machine refusal: {plain:?}"
                    ),
                    _ => assert_eq!(
                        (plain.status, code),
                        (403, json!("SYSTEM_ACCESS_REQUIRED")),
                        "{route}: the system-access refusal: {plain:?}"
                    ),
                }
            }
        }
    }
}

/// A request to `door` carrying `body`, signed under `secret` with the v2 scheme, with `extra`
/// headers.
fn signed_internal(
    app: &common::TestApp,
    secret: &str,
    door: &str,
    body: &'static str,
    extra: &[(&str, &str)],
) -> reqwest::RequestBuilder {
    let now = chrono::Utc::now().timestamp();
    let nonce = Uuid::new_v4().simple().to_string();
    let mut request = app
        .client
        .post(app.url(door))
        .header("content-type", "application/json")
        .body(body);
    for (name, value) in v2_headers(
        secret.as_bytes(),
        "POST",
        door,
        now,
        &nonce,
        body.as_bytes(),
    ) {
        request = request.header(name, value);
    }
    for (name, value) in extra {
        request = request.header(*name, *value);
    }
    request
}

// FAILS IF: an HMAC tier refuses before its signature gate (an unsigned call would get 400 instead
// of its 401), or not at all (a signed call carrying a header would reach its handler).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_internal_call_is_refused_only_once_signed(pool: PgPool) {
    let app = configured_app(&pool, None).await;
    // Each body is one its gate reads as well formed: the mint gate parses the body as part of
    // verifying it, and answers a malformed one 400 BAD_REQUEST before anything inner runs.
    for (secret, door, body) in [
        (RECONCILE_SECRET, DOOR_SAML_RECONCILE, "{}"),
        (
            MINT_SECRET,
            DOOR_SLACK_MINT,
            r#"{"slack_principal_id":"slack:T0PRECOND:U0PRECOND"}"#,
        ),
    ] {
        for header in POLICED {
            let signed = answer_of(
                signed_internal(&app, secret, door, body, &[header])
                    .send()
                    .await
                    .expect("sent"),
            )
            .await;
            assert_refused(&format!("signed POST {door}"), header, &signed);

            let unsigned = send(&app, "POST", door, None, &[header], &json!({})).await;
            let plain = send(&app, "POST", door, None, &[], &json!({})).await;
            assert_eq!(unsigned, plain, "unsigned POST {door} with {}", header.0);
            assert_eq!(plain.status, 401, "unsigned POST {door}: {plain:?}");
        }
    }
}

// FAILS IF: the internal app (the separate Vercel function) answers an unsigned caller
// differently for carrying a header, polices the crons it serves, or lets a signed write through.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_internal_app_places_the_refusal_identically(pool: PgPool) {
    use tower::ServiceExt;

    let state = common::test_state_with_config(pool.clone(), |config| {
        config.internal_reconcile_secret = Some(RECONCILE_SECRET.to_string());
        config.slack_mint_secret = Some(MINT_SECRET.to_string());
        config.embed_dispatch_secret = Some(DISPATCH_SECRET.to_string());
    })
    .await;
    let app = temper_api::create_internal_app(state);

    async fn call(
        app: &axum::Router,
        path: &str,
        headers: &[(&str, String)],
    ) -> (u16, Option<String>, Vec<u8>) {
        let mut request = axum::http::Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/json");
        for (name, value) in headers {
            request = request.header(*name, value);
        }
        let response = app
            .clone()
            .oneshot(request.body(axum::body::Body::from("{}")).expect("request"))
            .await
            .expect("response");
        let status = response.status().as_u16();
        let allow = response
            .headers()
            .get("allow")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body")
            .to_vec();
        (status, allow, body)
    }

    for path in [
        "/internal/saml/reconcile",
        "/internal/principal/resolve",
        "/internal/slack/link-state",
        "/internal/slack/mint",
        "/api/embed/dispatch",
        "/api/embed/warm",
        "/api/region/dispatch",
    ] {
        let plain = call(&app, path, &[]).await;
        for (name, value) in POLICED {
            let guarded = call(&app, path, &[(name, value.to_string())]).await;
            assert_eq!(guarded, plain, "internal POST {path} with {name}");
        }
    }

    let now = chrono::Utc::now().timestamp();
    for (name, value) in POLICED {
        let nonce = Uuid::new_v4().simple().to_string();
        let mut headers: Vec<(&str, String)> = v2_headers(
            RECONCILE_SECRET.as_bytes(),
            "POST",
            DOOR_SAML_RECONCILE,
            now,
            &nonce,
            b"{}",
        )
        .into_iter()
        .collect();
        headers.push((name, value.to_string()));
        let (status, _, body) = call(&app, DOOR_SAML_RECONCILE, &headers).await;
        let answer = Answer {
            status,
            allow: None,
            body: String::from_utf8_lossy(&body).to_string(),
        };
        assert_refused(
            "internal signed POST /internal/saml/reconcile",
            (name, value),
            &answer,
        );
    }
}

async fn webhook_event_count(pool: &PgPool, connection: Uuid) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM kb_events e \
           JOIN kb_connections c ON c.emitter_entity_id = e.emitter_entity_id \
          WHERE c.id = $1",
    )
    .bind(connection)
    .fetch_one(pool)
    .await
    .expect("count events")
}

// FAILS IF: the webhook intake refuses before its attestation (an unattested caller would get 400
// instead of its 401), or not at all (an attested receipt carrying a header would be recorded).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_intake_refuses_only_once_attested_and_writes_nothing(pool: PgPool) {
    // Unattested: the default broker verifies nothing.
    let refusing = configured_app(&pool, None).await;
    for header in POLICED {
        let plain = send(
            &refusing,
            "POST",
            "/api/intake/webhook",
            Some("Bearer x"),
            &[],
            &json!({}),
        )
        .await;
        let guarded = send(
            &refusing,
            "POST",
            "/api/intake/webhook",
            Some("Bearer x"),
            &[header],
            &json!({}),
        )
        .await;
        assert_eq!(guarded, plain, "unattested intake with {}", header.0);
        assert_eq!(plain.status, 401, "unattested intake: {plain:?}");
    }

    // Attested, but naming a connector no connection carries: the handler answers it with the same
    // 401 as an unverified attestation, so the header must not tell the two apart.
    let unprovisioned = FakeBroker::accepting_inbound("github", "github/nobody-here", "scl_none");
    let unprovisioned = common::setup_test_app_with_state(pool.clone(), |state| {
        state.broker = Arc::new(unprovisioned)
    })
    .await;
    let pr = github_pr_payload(GITHUB_REPO);
    for header in POLICED {
        let extra = [("x-github-event", "pull_request")];
        let plain = send(
            &unprovisioned,
            "POST",
            "/api/intake/webhook",
            Some("Bearer x"),
            &extra,
            &pr,
        )
        .await;
        let with: Vec<(&str, &str)> = extra.iter().copied().chain([header]).collect();
        let guarded = send(
            &unprovisioned,
            "POST",
            "/api/intake/webhook",
            Some("Bearer x"),
            &with,
            &pr,
        )
        .await;
        assert_eq!(
            guarded, plain,
            "verified but unprovisioned intake with {}",
            header.0
        );
        assert_eq!(
            plain.status, 401,
            "verified but unprovisioned intake: {plain:?}"
        );
    }

    // Attested, against a provisioned connection that a receipt would land on.
    let admin = seed_admin(&pool).await;
    let team = seed_team(&pool, admin).await;
    let connection = seed_connection(&pool, Some(team), admin).await;
    grant_reach(&pool, admin, connection, team).await;
    attach_credential_with_connector(&pool, connection, CONNECTOR_UID).await;
    let broker = FakeBroker::accepting_inbound("github", CONNECTOR_UID, "scl_test");
    let attested =
        common::setup_test_app_with_state(pool.clone(), |state| state.broker = Arc::new(broker))
            .await;
    let payload = github_pr_payload(GITHUB_REPO).to_string();
    let post = |extra: Option<(&'static str, &'static str)>| {
        let mut request = attested
            .client
            .post(attested.url("/api/intake/webhook"))
            .header("authorization", "Bearer anything-the-fake-accepts")
            .header("x-github-event", "pull_request")
            .body(payload.clone());
        if let Some((name, value)) = extra {
            request = request.header(name, value);
        }
        request
    };

    for header in POLICED {
        let before = database_fingerprint(&pool).await;
        let answer = answer_of(post(Some(header)).send().await.expect("sent")).await;
        assert_refused("attested POST /api/intake/webhook", header, &answer);
        assert_eq!(
            database_fingerprint(&pool).await,
            before,
            "a refused receipt wrote"
        );
    }
    assert_eq!(webhook_event_count(&pool, connection).await, 0);

    // The same receipt without the header lands, so the silence above is about the header.
    let answer = answer_of(post(None).send().await.expect("sent")).await;
    assert_eq!(
        answer.status, 202,
        "a receipt without the header lands: {answer:?}"
    );
    assert_eq!(webhook_event_count(&pool, connection).await, 1);
}

// FAILS IF: an admitted caller sending a policed header with a method the path does not mount gets
// the method router's 405 instead of the refusal, or the refusal loses the fallback's `Allow`. And,
// for a path two groups share, the answer depends on which group's fallback axum kept.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_admitted_unmounted_write_keeps_the_refusal_and_its_allow(pool: PgPool) {
    let app = configured_app(&pool, None).await;
    let (token, context_id) = person(&pool, true).await;
    let authorization = bearer(&token);
    let shapes = format!("/api/contexts/{context_id}/shapes");

    // `/api/ingest` mounts POST only; `/api/profile` is shared by the human self-service group
    // (PATCH) and the machine-admitting status group (GET); the shapes path is shared by the
    // gated list (GET) and the shape-declare group (POST).
    for (method, path) in [
        ("PUT", "/api/ingest"),
        ("DELETE", "/api/ingest"),
        ("POST", "/api/profile"),
        ("PUT", "/api/profile"),
        ("DELETE", shapes.as_str()),
    ] {
        let plain = send(&app, method, path, Some(&authorization), &[], &json!({})).await;
        assert_eq!(plain.status, 405, "{method} {path} is unmounted: {plain:?}");
        assert!(
            plain.allow.is_some(),
            "{method} {path}: the 405 names Allow"
        );
        for header in POLICED {
            let guarded = send(
                &app,
                method,
                path,
                Some(&authorization),
                &[header],
                &json!({}),
            )
            .await;
            assert_refused(&format!("{method} {path}"), header, &guarded);
            assert_eq!(
                guarded.allow, plain.allow,
                "{method} {path} with {}: the refusal keeps the fallback's Allow",
                header.0
            );
        }
    }

    // A machine on the shared `/api/profile`: the person-only PATCH refuses it with the machine
    // refusal whether or not it sends a header; an unmounted method answers behind the
    // machine-admitting status group's stack (axum keeps the last-merged group's fallback), which
    // admits the machine, so it gets the refusal where it would get 405, the same as a person.
    let machine = bearer(&approved_machine(&pool).await);
    for header in POLICED {
        let plain = send(
            &app,
            "PATCH",
            "/api/profile",
            Some(&machine),
            &[],
            &json!({}),
        )
        .await;
        let guarded = send(
            &app,
            "PATCH",
            "/api/profile",
            Some(&machine),
            &[header],
            &json!({}),
        )
        .await;
        assert_eq!(
            guarded, plain,
            "PATCH /api/profile as a machine with {}",
            header.0
        );
        assert_eq!(plain.status, 403, "the machine refusal: {plain:?}");
        let plain = send(&app, "PUT", "/api/profile", Some(&machine), &[], &json!({})).await;
        assert_eq!(
            plain.status, 405,
            "PUT /api/profile as a machine: {plain:?}"
        );
        let guarded = send(
            &app,
            "PUT",
            "/api/profile",
            Some(&machine),
            &[header],
            &json!({}),
        )
        .await;
        assert_refused("PUT /api/profile as a machine", header, &guarded);
        assert_eq!(guarded.allow, plain.allow);
    }

    // The shared paths' mounted methods: refused for the write, untouched for the read.
    for header in POLICED {
        for (method, path) in [("PATCH", "/api/profile"), ("POST", shapes.as_str())] {
            let answer = send(
                &app,
                method,
                path,
                Some(&authorization),
                &[header],
                &json!({}),
            )
            .await;
            assert_refused(&format!("{method} {path}"), header, &answer);
        }
        for path in ["/api/profile", shapes.as_str()] {
            let plain = send(&app, "GET", path, Some(&authorization), &[], &json!({})).await;
            let guarded = send(
                &app,
                "GET",
                path,
                Some(&authorization),
                &[header],
                &json!({}),
            )
            .await;
            assert_eq!(plain.status, 200, "GET {path}: {plain:?}");
            assert_eq!(guarded.status, plain.status, "GET {path} with {}", header.0);
        }
    }
}

// FAILS IF: the refusal loses `Access-Control-Allow-Origin` or the security headers (the layer
// moved outside the base layers or CORS), or an unmatched path carrying a header stops answering
// the fallback's `404`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_refusal_carries_cors_and_the_security_headers(pool: PgPool) {
    const ORIGIN: &str = "https://allowed.example";
    let app = common::setup_test_app_with_config(pool.clone(), |config| {
        config.cors_origins = vec![ORIGIN.to_string()];
    })
    .await;
    let (token, _) = person(&pool, true).await;

    for header in POLICED {
        let response = app
            .client
            .post(app.url("/api/ingest"))
            .header("Authorization", bearer(&token))
            .header("Origin", ORIGIN)
            .header(header.0, header.1)
            .json(&json!({}))
            .send()
            .await
            .expect("request sent");
        let headers = response.headers().clone();
        let answer = answer_of(response).await;
        assert_refused("POST /api/ingest from an allowed origin", header, &answer);
        assert_eq!(
            headers
                .get("access-control-allow-origin")
                .and_then(|v| v.to_str().ok()),
            Some(ORIGIN),
            "the refusal must carry CORS; headers: {headers:?}"
        );
        assert_eq!(
            headers
                .get("x-content-type-options")
                .and_then(|v| v.to_str().ok()),
            Some("nosniff"),
            "the refusal must carry the security headers; headers: {headers:?}"
        );

        let unmatched = send(
            &app,
            "POST",
            "/api/no-such-route-anywhere",
            Some(&bearer(&token)),
            &[header],
            &json!({}),
        )
        .await;
        assert!(
            unmatched.status == 404 && unmatched.body.contains("No route matches"),
            "an unmatched path with {} must answer the fallback's 404: {unmatched:?}",
            header.0
        );
    }
}

/// Every documented operation, as `(METHOD, path template)`.
fn documented_operations() -> BTreeSet<(String, String)> {
    let spec = serde_json::to_value(temper_api::routes::openapi_spec()).expect("spec serializes");
    let mut out = BTreeSet::new();
    for (path, item) in spec["paths"].as_object().expect("paths") {
        for (method, _) in item.as_object().expect("path item") {
            if matches!(
                method.as_str(),
                "get" | "post" | "put" | "patch" | "delete" | "head" | "options"
            ) {
                out.insert((method.to_uppercase(), path.clone()));
            }
        }
    }
    out
}

/// Every documented write. All of them are served by `Refused` groups: the one documented group
/// that is not (`public_routes`) mounts no write, which
/// `preconditions_no_unpoliced_documented_group_mounts_a_write` in `routes::tests` holds.
fn documented_writes() -> Vec<(String, String)> {
    documented_operations()
        .into_iter()
        .filter(|(m, _)| is_write(m))
        .collect()
}

// FAILS IF: any documented write applies, or even reaches its handler, for an approved person
// carrying a policed header, or the refusal changes a row.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn every_documented_write_refuses_each_header_for_an_admitted_caller(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, context_id) = person(&pool, true).await;
    let authorization = bearer(&token);
    let routes = documented_writes();
    assert!(
        routes.len() > 50,
        "the sweep found only {} write routes; the spec read is broken",
        routes.len()
    );
    for family in [
        "/api/resources/{id}",
        "/api/ingest",
        "/api/admin/",
        "/api/blobs",
    ] {
        assert!(
            routes.iter().any(|(_, p)| p.starts_with(family)),
            "no write route under {family}: the sweep lost a family"
        );
    }

    // A body the routes would act on where they can: it names a real, writable context.
    let body = json!({ "context_ref": context_id.to_string(), "title": "precondition sweep" });
    // The admitted caller's first request may record its own sign-in; take the fingerprint after.
    send(
        &app,
        "GET",
        "/api/profile",
        Some(&authorization),
        &[],
        &json!({}),
    )
    .await;
    let before = database_fingerprint(&pool).await;
    for (method, template) in &routes {
        let path = concrete(template);
        for header in POLICED {
            let answer = send(&app, method, &path, Some(&authorization), &[header], &body).await;
            assert_refused(&format!("{method} {template}"), header, &answer);
        }
    }
    assert_eq!(
        database_fingerprint(&pool).await,
        before,
        "a refused write changed the database"
    );
}

// FAILS IF: the layer refuses a write that carries no policed header, or a swept route is not
// mounted with that method, so the sweep's refusals would prove nothing about it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn without_the_header_no_write_route_meets_the_refusal(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, _) = person(&pool, true).await;
    for (method, template) in documented_writes() {
        let path = concrete(&template);
        let answer = send(&app, &method, &path, Some(&bearer(&token)), &[], &json!({})).await;
        assert!(
            answer.status != 405
                && !(answer.status == 404 && answer.body.contains("No route matches")),
            "{method} {template} is not mounted: {answer:?}"
        );
        assert_ne!(
            answer.json()["error"]["code"],
            json!(temper_core::error::PRECONDITION_NOT_HONOURED_CODE),
            "{method} {template} without a precondition header was refused: {answer:?}"
        );
    }
}

// FAILS IF: a read meets the refusal, or answers differently, because it carries a policed header.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reads_ignore_the_headers(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, _) = person(&pool, true).await;
    let authorization = bearer(&token);
    let reads: Vec<(String, String)> = documented_operations()
        .into_iter()
        .filter(|(m, _)| !is_write(m))
        .collect();
    assert!(
        reads.len() > 20,
        "the sweep found only {} reads",
        reads.len()
    );

    for (method, template) in reads {
        let path = concrete(&template);
        for verb in [method.as_str(), "HEAD", "OPTIONS"] {
            // A HEAD answer has no body, so the status is the only thing that can show a refusal.
            let plain = send(&app, verb, &path, Some(&authorization), &[], &json!({})).await;
            for header in POLICED {
                let guarded = send(
                    &app,
                    verb,
                    &path,
                    Some(&authorization),
                    &[header],
                    &json!({}),
                )
                .await;
                assert_eq!(
                    guarded.status, plain.status,
                    "{verb} {template} with {}: {guarded:?}",
                    header.0
                );
                assert!(
                    !is_precondition_refusal(&guarded, header),
                    "{verb} {template} with {} met the refusal: {guarded:?}",
                    header.0
                );
            }
        }
    }
}

fn ingest_payload(context_id: Uuid, title: &str) -> IngestPayload {
    IngestPayload {
        idempotency_key: None,
        segmented: None,
        title: title.to_string(),
        origin_uri: format!("test://precondition-{}", Uuid::new_v4()),
        context_ref: context_id.to_string(),
        home_cogmap_id: None,
        doc_type_name: "research".to_string(),
        content_hash: None,
        content: String::new(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: None,
        goal: None,
        act: Default::default(),
        sources: Vec::new(),
    }
}

// FAILS IF: a create, a PATCH, a meta write or a delete carrying a policed header lands, or one
// without it stops landing. The sweep above proves the refusal is uniform; this proves on real
// rows that it is the header, and only the header, that decides.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn real_writes_land_without_the_header_and_not_with_it(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, context_id) = person(&pool, true).await;
    let authorization = bearer(&token);
    let auth = Some(authorization.as_str());
    // The caller's first authenticated request may record its own sign-in; make it before any
    // fingerprint is taken, so the fingerprints below see only what the writes do.
    send(&app, "GET", "/api/profile", auth, &[], &json!({})).await;

    for header in POLICED {
        let payload = serde_json::to_value(ingest_payload(context_id, "Refused create")).unwrap();
        let before = database_fingerprint(&pool).await;
        let answer = send(&app, "POST", "/api/ingest", auth, &[header], &payload).await;
        assert_refused("POST /api/ingest", header, &answer);
        assert_eq!(
            database_fingerprint(&pool).await,
            before,
            "refused create wrote"
        );
    }

    let payload = serde_json::to_value(ingest_payload(context_id, "Original")).unwrap();
    let before = database_fingerprint(&pool).await;
    let created = send(&app, "POST", "/api/ingest", auth, &[], &payload).await;
    assert_eq!(
        created.status, 200,
        "create without a header lands: {created:?}"
    );
    assert_ne!(
        database_fingerprint(&pool).await,
        before,
        "the fingerprint must see a real write, or its silence above proves nothing"
    );
    let id = created.json()["id"].as_str().expect("id").to_string();
    let resource = format!("/api/resources/{id}");
    let meta = format!("/api/resources/{id}/meta");
    let meta_body = json!({
        "resource_id": id,
        "managed_meta": {"temper-provenance": "llm-discovered"},
        "open_meta": {},
        "managed_hash": "",
        "open_hash": "",
    });

    for header in POLICED {
        let before = database_fingerprint(&pool).await;
        let rename = json!({"title": "Renamed"});
        let answer = send(&app, "PATCH", &resource, auth, &[header], &rename).await;
        assert_refused("PATCH /api/resources/{id}", header, &answer);
        let answer = send(&app, "PUT", &meta, auth, &[header], &meta_body).await;
        assert_refused("PUT /api/resources/{id}/meta", header, &answer);
        let answer = send(&app, "DELETE", &resource, auth, &[header], &json!({})).await;
        assert_refused("DELETE /api/resources/{id}", header, &answer);
        assert_eq!(
            database_fingerprint(&pool).await,
            before,
            "a refused write wrote"
        );
    }
    let row = send(&app, "GET", &resource, auth, &[], &json!({})).await;
    assert_eq!(row.json()["title"], json!("Original"), "{row:?}");

    let answer = send(
        &app,
        "PATCH",
        &resource,
        auth,
        &[],
        &json!({"title": "Renamed"}),
    )
    .await;
    assert_eq!(
        answer.status, 200,
        "PATCH without a header lands: {answer:?}"
    );
    let row = send(&app, "GET", &resource, auth, &[], &json!({})).await;
    assert_eq!(row.json()["title"], json!("Renamed"));
    let answer = send(&app, "PUT", &meta, auth, &[], &meta_body).await;
    assert_eq!(
        answer.status, 200,
        "PUT meta without a header lands: {answer:?}"
    );
    let answer = send(&app, "DELETE", &resource, auth, &[], &json!({})).await;
    assert!(
        (200..300).contains(&answer.status),
        "DELETE without a header lands: {answer:?}"
    );
}
