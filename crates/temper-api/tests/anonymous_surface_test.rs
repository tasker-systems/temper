//! What an unauthenticated caller can learn, asserted against pinned bytes.
//!
//! The rule: an unauthenticated caller learns nothing except from the designated public routes.
//! Every assertion here compares an answer against a pinned constant (absolute bytes, not "every
//! path answered alike"), so a revert that moves every path together still fails.
//!
//! What is witnessed so far, through the real `create_app` and its outer service:
//! - **preflight**: a CORS preflight from an allowlisted origin is the same bytes on every path,
//!   matched or not, public or not, carrying the security headers and never `Allow`; every other
//!   `OPTIONS` is routed like any other method;
//! - **`Allow` rides only a 405**: never on a 401, never on the 415 an undecodable
//!   `Content-Encoding` gets before any gate;
//! - **the uniform refusal**: every pre-identity refusal, whatever the gate, the door, the
//!   credential shape or the configuration, is exactly [`u401`]; while the bearer key store is
//!   failing, every one of them is exactly [`unavailable`] instead;
//! - **the request id** is minted per request, 32 lowercase hex digits, and never adopted from the
//!   request; an allowlisted origin can read it (the root span's copy is witnessed in
//!   `telemetry_request_id_test`).
#![cfg(feature = "test-db")]

mod common;

use std::collections::BTreeMap;

use reqwest::Method;
use sqlx::PgPool;
use uuid::Uuid;

const ORIGIN: &str = "https://allowed.example";
const FOREIGN: &str = "https://foreign.example";

/// One answer, with its headers lowercased and `date` dropped (the comparison never includes it).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Answer {
    status: u16,
    headers: BTreeMap<String, String>,
    body: String,
}

impl Answer {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    /// The answer without `x-request-id`, which is asserted by format, never by value.
    fn without_request_id(&self) -> Answer {
        let mut a = self.clone();
        a.headers.remove("x-request-id");
        a
    }
}

async fn send(
    app: &common::TestApp,
    method: Method,
    path: &str,
    headers: &[(&str, &str)],
) -> Answer {
    let mut request = app.client.request(method, app.url(path));
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = request.send().await.expect("request sent");
    answer_of(response).await
}

async fn answer_of(response: reqwest::Response) -> Answer {
    let status = response.status().as_u16();
    let mut map = BTreeMap::new();
    for (name, value) in response.headers() {
        if name.as_str() == "date" {
            continue;
        }
        let value = value.to_str().expect("ascii header").to_string();
        map.entry(name.as_str().to_string())
            .and_modify(|v: &mut String| {
                v.push_str(", ");
                v.push_str(&value);
            })
            .or_insert(value);
    }
    let body = response.text().await.expect("body");
    Answer {
        status,
        headers: map,
        body,
    }
}

/// The CORS actual-response headers and `vary`, every value (`send` joins repeated headers).
fn cors_headers(answer: &Answer) -> BTreeMap<String, String> {
    answer
        .headers
        .iter()
        .filter(|(n, _)| n.starts_with("access-control-") || n.as_str() == "vary")
        .map(|(n, v)| (n.clone(), v.clone()))
        .collect()
}

fn assert_request_id_format(what: &str, answer: &Answer) {
    let id = answer
        .header("x-request-id")
        .unwrap_or_else(|| panic!("{what}: no x-request-id: {answer:?}"));
    assert!(
        id.len() == 32
            && id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "{what}: x-request-id is not 32 lowercase hex digits: {id:?}"
    );
}

/// The five security headers, exactly as the per-endpoint layers set them.
const SECURITY_HEADERS: [(&str, &str); 5] = [
    (
        "content-security-policy",
        "default-src 'none'; frame-ancestors 'none'; base-uri 'none'",
    ),
    ("referrer-policy", "no-referrer"),
    (
        "strict-transport-security",
        "max-age=63072000; includeSubDomains",
    ),
    ("x-content-type-options", "nosniff"),
    ("x-frame-options", "DENY"),
];

const U401_BODY: &str =
    r#"{"error":{"code":"UNAUTHORIZED","message":"Unauthorized: authentication required"}}"#;
const UNAVAILABLE_BODY: &str = r#"{"error":{"code":"UNAUTHORIZED","message":"Unauthorized: Authentication service unavailable"}}"#;

/// A 401 with `body`, without the CORS headers (`access-control-*` and `vary`), which depend only
/// on `Origin` and the CORS configuration and are compared across paths instead
/// ([`assert_refusals`]). `x-request-id` is asserted by format.
fn refusal_constant(body: &str) -> Answer {
    let mut headers: BTreeMap<String, String> = SECURITY_HEADERS
        .iter()
        .map(|(n, v)| (n.to_string(), v.to_string()))
        .collect();
    for (n, v) in [
        ("content-type", "application/json"),
        ("content-length", &body.len().to_string()),
    ] {
        headers.insert(n.to_string(), v.to_string());
    }
    Answer {
        status: 401,
        headers,
        body: body.to_string(),
    }
}

/// `U401`: every pre-identity refusal.
fn u401() -> Answer {
    refusal_constant(U401_BODY)
}

/// `UNAVAILABLE`: every pre-identity refusal while the bearer key store is failing.
fn unavailable() -> Answer {
    refusal_constant(UNAVAILABLE_BODY)
}

/// The answer without its CORS headers or request id: the bytes a refusal pins absolutely.
fn without_cors(answer: &Answer) -> Answer {
    let mut a = answer.without_request_id();
    a.headers
        .retain(|n, _| !n.starts_with("access-control-") && n.as_str() != "vary");
    a
}

/// Every answer is exactly `expected` apart from its CORS headers, and those are one set across
/// all of them: they may depend on the configuration, never on the path.
fn assert_refusals(answers: &[(String, Answer)], expected: &Answer) {
    let cors = cors_headers(&answers[0].1);
    for (what, answer) in answers {
        assert_request_id_format(what, answer);
        assert_eq!(&without_cors(answer), expected, "{what}");
        assert_eq!(
            cors_headers(answer),
            cors,
            "{what}: CORS headers differ by path"
        );
    }
}

/// `PREFLIGHT`: the answer to a preflight from [`ORIGIN`] asking for `POST` with `authorization`,
/// on any path (`x-request-id` asserted by format).
fn preflight_constant() -> Answer {
    let mut headers: BTreeMap<String, String> = SECURITY_HEADERS
        .iter()
        .map(|(n, v)| (n.to_string(), v.to_string()))
        .collect();
    for (n, v) in [
        ("access-control-allow-origin", ORIGIN),
        ("access-control-allow-methods", "*"),
        ("access-control-allow-headers", "authorization"),
        // tower-http's own `vary` for an allowlist that mirrors the request headers: the origin
        // and the header list are the two inputs the answer varies on (the methods are `*`).
        ("vary", "origin, access-control-request-headers"),
        ("content-length", "0"),
    ] {
        headers.insert(n.to_string(), v.to_string());
    }
    Answer {
        status: 200,
        headers,
        body: String::new(),
    }
}

const PREFLIGHT_REQUEST: [(&str, &str); 3] = [
    ("origin", ORIGIN),
    ("access-control-request-method", "POST"),
    ("access-control-request-headers", "authorization"),
];

/// Representative paths: one per kind of door, plus the unmatched and public ones.
fn representative_paths() -> Vec<String> {
    let id = Uuid::new_v4();
    vec![
        "/api/health".to_string(),
        "/api/auth/slack/callback".to_string(),
        "/api/profile".to_string(),
        "/api/resources".to_string(),
        format!("/api/resources/{id}"),
        "/api/admin/ledger".to_string(),
        "/internal/saml/reconcile".to_string(),
        "/internal/slack/mint".to_string(),
        "/api/embed/dispatch".to_string(),
        "/api/intake/webhook".to_string(),
        "/no-such".to_string(),
        "/api/no-such".to_string(),
        "/internal/no-such".to_string(),
    ]
}

async fn allowlisted_app(pool: &PgPool) -> common::TestApp {
    common::setup_test_app_with_config(pool.clone(), |config| {
        config.cors_origins = vec![ORIGIN.to_string()];
        config.internal_reconcile_secret = Some("reconcile-secret".to_string());
        config.slack_mint_secret = Some("mint-secret".to_string());
        config.embed_dispatch_secret = Some("dispatch-secret".to_string());
    })
    .await
}

// FAILS IF: an allowlisted preflight's answer depends on the path, lacks the security headers,
// carries `Allow`, or its bytes move.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn preflight_bytes_are_pinned_and_path_independent(pool: PgPool) {
    let app = allowlisted_app(&pool).await;
    let expected = preflight_constant();
    for path in representative_paths() {
        let answer = send(&app, Method::OPTIONS, &path, &PREFLIGHT_REQUEST).await;
        assert_request_id_format(&format!("OPTIONS {path}"), &answer);
        assert_eq!(answer.without_request_id(), expected, "OPTIONS {path}");
    }
}

// FAILS IF: an `OPTIONS` that is not a preflight this instance answers (no `Origin`, a foreign one,
// an allowlisted one without `Access-Control-Request-Method`, or any origin when none is
// configured) gets the CORS layer's 200, or a gated path's answer carries `Allow`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn options_without_a_preflight_is_routed(pool: PgPool) {
    let app = allowlisted_app(&pool).await;
    let not_preflights: [&[(&str, &str)]; 3] = [
        &[],
        &[
            ("origin", FOREIGN),
            ("access-control-request-method", "POST"),
        ],
        &[("origin", ORIGIN)],
    ];
    for headers in not_preflights {
        for path in [
            "/api/admin/ledger",
            "/api/profile",
            "/api/resources",
            "/internal/slack/mint",
        ] {
            let answer = send(&app, Method::OPTIONS, path, headers).await;
            assert_eq!(answer.status, 401, "OPTIONS {path} {headers:?}: {answer:?}");
            assert_eq!(answer.header("allow"), None, "OPTIONS {path} {headers:?}");
            assert_request_id_format(&format!("OPTIONS {path}"), &answer);
        }
        // The CORS actual-response headers depend only on `Origin`, routed `OPTIONS` included.
        let routed = send(&app, Method::OPTIONS, "/api/profile", headers).await;
        let get = send(&app, Method::GET, "/api/profile", headers).await;
        assert_eq!(
            cors_headers(&routed),
            cors_headers(&get),
            "OPTIONS /api/profile {headers:?} against GET: {routed:?}"
        );
        if headers.first() == Some(&("origin", ORIGIN)) {
            assert_eq!(routed.header("access-control-allow-origin"), Some(ORIGIN));
        }
        // A designated public route keeps its own answer: 405 with `Allow`.
        let health = send(&app, Method::OPTIONS, "/api/health", headers).await;
        assert_eq!(
            health.status, 405,
            "OPTIONS /api/health {headers:?}: {health:?}"
        );
        assert!(
            health.header("allow").is_some(),
            "the public route's 405 names Allow"
        );
    }

    // No origins configured: nothing is a preflight this instance answers.
    let closed = common::setup_test_app_with_config(pool.clone(), |config| {
        config.cors_origins = vec![];
    })
    .await;
    let answer = send(
        &closed,
        Method::OPTIONS,
        "/api/admin/ledger",
        &PREFLIGHT_REQUEST,
    )
    .await;
    assert_eq!(answer.status, 401, "{answer:?}");
}

// FAILS IF: in permissive mode (`CORS_ORIGINS=*`) a preflight from any origin is not answered, or
// its answer depends on the path.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_permissive_preflight_is_path_independent(pool: PgPool) {
    let app = common::setup_test_app_with_config(pool.clone(), |config| {
        config.cors_origins = vec!["*".to_string()];
    })
    .await;
    let request = [
        ("origin", FOREIGN),
        ("access-control-request-method", "POST"),
    ];
    let first = send(&app, Method::OPTIONS, "/api/health", &request).await;
    assert_eq!(first.status, 200, "{first:?}");
    assert_eq!(first.header("access-control-allow-origin"), Some("*"));
    assert_eq!(first.header("allow"), None);
    for path in representative_paths() {
        let answer = send(&app, Method::OPTIONS, &path, &request).await;
        assert_eq!(
            answer.without_request_id(),
            first.without_request_id(),
            "OPTIONS {path}"
        );
    }
}

// FAILS IF: wrapping the router in the outer one changes a `HEAD` answer: a different
// `content-length` from the `GET`'s, or a body.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn head_keeps_the_get_headers(pool: PgPool) {
    let app = allowlisted_app(&pool).await;
    // Not an unmatched path: its 404 body echoes the method, so `HEAD` and `GET` differ by a byte.
    for path in ["/api/health", "/api/profile"] {
        let get = send(&app, Method::GET, path, &[]).await;
        let head = send(&app, Method::HEAD, path, &[]).await;
        assert_eq!(head.status, get.status, "HEAD {path}");
        assert_eq!(head.body, "", "HEAD {path} has no body");
        assert_eq!(
            head.header("content-length"),
            get.header("content-length"),
            "HEAD {path}: {head:?} against {get:?}"
        );
    }
}

// FAILS IF: `Allow` rides any status but 405: the anonymous 401 on an unmounted method, or the 415
// an undecodable `Content-Encoding` gets before any gate, on a mounted, an unmounted-method or an
// unmatched path. And the admitted control: an authenticated unmounted method keeps its 405 with
// `Allow`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn allow_rides_only_a_405(pool: PgPool) {
    let app = allowlisted_app(&pool).await;

    for (method, path) in [
        (Method::POST, "/api/profile"),
        (Method::PUT, "/api/resources"),
        (Method::PUT, "/internal/slack/mint"),
    ] {
        let answer = send(&app, method.clone(), path, &[]).await;
        assert_eq!(answer.status, 401, "{method} {path}: {answer:?}");
        assert_eq!(answer.header("allow"), None, "{method} {path}");
    }

    for (method, path) in [
        (Method::POST, "/api/resources"),
        (Method::PUT, "/api/resources"),
        (Method::POST, "/api/no-such"),
        (Method::PUT, "/internal/slack/mint"),
    ] {
        let answer = send(&app, method.clone(), path, &[("content-encoding", "foo")]).await;
        assert_eq!(
            answer.status, 415,
            "{method} {path} Content-Encoding: foo: {answer:?}"
        );
        assert_eq!(
            answer.header("allow"),
            None,
            "{method} {path} Content-Encoding: foo"
        );
    }

    let email = format!("anon-surface-{}@example.com", Uuid::new_v4());
    let (profile_id, _) = common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);
    let bearer = format!("Bearer {token}");
    let admitted = send(
        &app,
        Method::PUT,
        "/api/resources",
        &[("authorization", &bearer)],
    )
    .await;
    assert_eq!(admitted.status, 405, "{admitted:?}");
    assert_eq!(
        admitted.header("allow"),
        Some("GET,HEAD,POST"),
        "{admitted:?}"
    );
}

// FAILS IF: a response lacks `x-request-id`, two requests share one, or the id is taken from the
// request's own `x-request-id` or `traceparent`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn request_id_is_minted_and_never_adopted(pool: PgPool) {
    let app = allowlisted_app(&pool).await;
    const INBOUND_ID: &str = "0123456789abcdef0123456789abcdef";
    const TRACEPARENT: &str = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
    let mut seen = std::collections::BTreeSet::new();
    for path in representative_paths() {
        for headers in [
            &[][..],
            &[("x-request-id", INBOUND_ID), ("traceparent", TRACEPARENT)][..],
        ] {
            let answer = send(&app, Method::GET, &path, headers).await;
            assert_request_id_format(&format!("GET {path}"), &answer);
            let id = answer.header("x-request-id").expect("id").to_string();
            assert_ne!(id, INBOUND_ID, "GET {path}: the inbound id was adopted");
            assert!(
                !TRACEPARENT.contains(&id),
                "GET {path}: the trace id was adopted"
            );
            assert!(
                !answer.body.contains(INBOUND_ID),
                "GET {path}: the inbound id was echoed"
            );
            assert!(seen.insert(id), "GET {path}: a request id repeated");
        }
    }
}

// FAILS IF: a cross-origin browser client on an allowlisted origin cannot read the request id (it
// is not exposed), or the actual response after a preflight loses its CORS headers.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn expose_headers_name_the_request_id(pool: PgPool) {
    let app = allowlisted_app(&pool).await;
    let email = format!("anon-surface-{}@example.com", Uuid::new_v4());
    let (profile_id, _) = common::fixtures::create_test_profile_with_context(&pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);
    let bearer = format!("Bearer {token}");

    let preflight = send(&app, Method::OPTIONS, "/api/profile", &PREFLIGHT_REQUEST).await;
    assert_eq!(
        preflight.header("access-control-allow-origin"),
        Some(ORIGIN)
    );

    for headers in [
        vec![("origin", ORIGIN)],
        vec![("origin", ORIGIN), ("authorization", bearer.as_str())],
    ] {
        let answer = send(&app, Method::GET, "/api/profile", &headers).await;
        assert_eq!(
            answer.header("access-control-allow-origin"),
            Some(ORIGIN),
            "{answer:?}"
        );
        let exposed = answer
            .header("access-control-expose-headers")
            .unwrap_or_default()
            .to_ascii_lowercase();
        assert!(
            exposed
                .split(',')
                .map(str::trim)
                .any(|h| h == "x-request-id"),
            "x-request-id is not exposed: {answer:?}"
        );
        assert!(
            exposed.split(',').map(str::trim).any(|h| h == "link"),
            "link stays exposed"
        );
    }

    // A foreign origin gets no ACAO on the actual response, whatever the path.
    let foreign = send(&app, Method::GET, "/api/profile", &[("origin", FOREIGN)]).await;
    assert_eq!(foreign.header("access-control-allow-origin"), None);
}

/// Every credential shape that does not verify, for the bearer routes.
fn unverified_bearers() -> Vec<Vec<(&'static str, String)>> {
    vec![
        vec![],
        vec![("authorization", "Bearer garbage".to_string())],
        vec![("authorization", "Basic eDp5".to_string())],
        vec![(
            "authorization",
            format!(
                "Bearer {}",
                common::generate_expired_jwt("test|expired", "e@example.com")
            ),
        )],
        vec![(
            "authorization",
            format!(
                "Bearer {}",
                common::generate_test_jwt_with_audience("test|aud", "a@example.com", "not-ours")
            ),
        )],
    ]
}

/// A signature header set that parses and does not verify.
fn bad_signature() -> Vec<(&'static str, String)> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    vec![
        (temper_core::internal_sig::TIMESTAMP_HEADER, now.to_string()),
        (
            temper_core::internal_sig::NONCE_HEADER,
            Uuid::new_v4().simple().to_string(),
        ),
        (
            temper_core::internal_sig::SIGNATURE_V2_HEADER,
            "00".repeat(32),
        ),
    ]
}

async fn send_owned(
    app: &common::TestApp,
    method: Method,
    path: &str,
    headers: &[(&'static str, String)],
) -> Answer {
    let borrowed: Vec<(&str, &str)> = headers.iter().map(|(n, v)| (*n, v.as_str())).collect();
    send(app, method, path, &borrowed).await
}

/// Request headers owned by the request list.
type Headers = Vec<(&'static str, String)>;

/// The anonymous requests every gate kind refuses, with the credential each might present.
fn refused_requests() -> Vec<(Method, String, Headers)> {
    let id = Uuid::new_v4();
    let mut out = Vec::new();
    for bearer in unverified_bearers() {
        for (method, path) in [
            (Method::GET, "/api/profile".to_string()),
            (Method::POST, "/api/profile".to_string()),
            (Method::GET, format!("/api/resources/{id}")),
            (Method::GET, "/api/admin/ledger".to_string()),
            (Method::OPTIONS, "/api/admin/ledger".to_string()),
            (Method::GET, "/api/embed/dispatch".to_string()),
            (Method::GET, "/api/embed/warm".to_string()),
            (Method::POST, "/api/intake/webhook".to_string()),
        ] {
            out.push((method, path, bearer.clone()));
        }
    }
    for headers in [vec![], bad_signature()] {
        for path in [
            "/internal/saml/reconcile",
            "/internal/slack/mint",
            "/internal/slack/link-state",
        ] {
            out.push((Method::POST, path.to_string(), headers.clone()));
        }
    }
    // The tier layers answer before routing detail: an unmounted method on a cron door or the
    // intake, and a query the cron door's extractor would reject.
    for (method, path) in [
        (Method::PUT, "/api/embed/dispatch"),
        (Method::DELETE, "/api/embed/warm"),
        (Method::GET, "/api/embed/dispatch?shard=abc"),
        (Method::GET, "/api/region/dispatch?cap=x"),
        (Method::PUT, "/api/intake/webhook"),
        (Method::GET, "/api/intake/webhook"),
    ] {
        out.push((method, path.to_string(), vec![]));
    }
    // A stale signature, and a malformed nonce, are the same refusal as a bad one.
    let mut stale = bad_signature();
    stale[0].1 = "1".to_string();
    out.push((Method::POST, "/internal/slack/mint".to_string(), stale));
    let mut nonce = bad_signature();
    nonce[1].1 = "not-a-nonce".to_string();
    out.push((Method::POST, "/internal/saml/reconcile".to_string(), nonce));
    out
}

/// The refusals a header map cannot carry as `&str`: a non-UTF-8 `Authorization`, and a signed
/// request whose body is over the signature gates' 64 KiB read.
async fn raw_refusals(app: &common::TestApp) -> Vec<(String, Answer)> {
    let mut out = Vec::new();
    let bytes = reqwest::header::HeaderValue::from_bytes(b"Bearer \xff\xfe").expect("obs-text");
    let response = app
        .client
        .get(app.url("/api/profile"))
        .header("authorization", bytes)
        .send()
        .await
        .expect("sent");
    out.push((
        "non-UTF-8 Authorization".to_string(),
        answer_of(response).await,
    ));
    for path in ["/internal/slack/mint", "/internal/saml/reconcile"] {
        let mut request = app.client.post(app.url(path)).body(vec![b'x'; 70 * 1024]);
        for (n, v) in bad_signature() {
            request = request.header(n, v);
        }
        let response = request.send().await.expect("sent");
        out.push((
            format!("70 KiB signed body to {path}"),
            answer_of(response).await,
        ));
    }
    // A body the intake's extractor could not decompress: the gate answers before the body is read.
    let response = app
        .client
        .post(app.url("/api/intake/webhook"))
        .header("content-encoding", "gzip")
        .body("notgzip")
        .send()
        .await
        .expect("sent");
    out.push((
        "corrupt gzip to the intake".to_string(),
        answer_of(response).await,
    ));
    out
}

// FAILS IF: any pre-identity refusal (bearer, signature, cron, intake; any credential shape; secrets
// set or unset) is not exactly `U401`: a gate's own message, a door's label or a configuration
// state reaches the body.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn pre_identity_refusals_are_u401(pool: PgPool) {
    let configured = allowlisted_app(&pool).await;
    let bare = common::setup_test_app_with_config(pool.clone(), |_| {}).await;
    for app in [&configured, &bare] {
        let mut answers = Vec::new();
        for (method, path, headers) in refused_requests() {
            let answer = send_owned(app, method.clone(), &path, &headers).await;
            answers.push((format!("{method} {path} {headers:?}"), answer));
        }
        answers.extend(raw_refusals(app).await);
        assert_refusals(&answers, &u401());
    }
}

// FAILS IF: while the bearer key store is failing, a pre-identity refusal on some path still answers
// `U401` (the outage would tell a prober which paths verify a bearer), or a public route stops
// answering for itself.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_failing_key_store_makes_every_refusal_unavailable(pool: PgPool) {
    let app = common::setup_test_app_with_state(pool.clone(), |state| {
        // Nothing listens on port 1, so every refresh fails.
        state.jwks_store = std::sync::Arc::new(temper_services::state::JwksKeyStore::new(
            "http://127.0.0.1:1/.well-known/jwks.json".to_string(),
        ));
        let mut config = (*state.config).clone();
        config.cors_origins = vec![ORIGIN.to_string()];
        state.config = std::sync::Arc::new(config);
    })
    .await;
    // Any bearer reaches the empty cache and fails its refresh; from then on the store is failing.
    let primed = send(
        &app,
        Method::GET,
        "/api/profile",
        &[("authorization", "Bearer garbage")],
    )
    .await;
    let mut answers = vec![("the priming request".to_string(), primed)];
    for (method, path, headers) in refused_requests() {
        let answer = send_owned(&app, method.clone(), &path, &headers).await;
        answers.push((format!("{method} {path} {headers:?}"), answer));
    }
    assert_refusals(&answers, &unavailable());

    // The swap replaces the body, never the headers that depend on the request: an allowlisted
    // origin still reads the unavailable answer, and the same CORS headers ride every path.
    let mut answers = Vec::new();
    for (method, path, mut headers) in refused_requests() {
        headers.push(("origin", ORIGIN.to_string()));
        let answer = send_owned(&app, method.clone(), &path, &headers).await;
        assert_eq!(
            answer.header("access-control-allow-origin"),
            Some(ORIGIN),
            "{method} {path}"
        );
        answers.push((format!("{method} {path} {headers:?}"), answer));
    }
    assert_refusals(&answers, &unavailable());
    let health = send(&app, Method::GET, "/api/health", &[]).await;
    assert_eq!(health.status, 200, "{health:?}");

    // The raw shapes, and HEAD: the same unavailable answer, with no body. A refusal here waits
    // on the store's refresh, and hyper closes a connection whose request body was still unread
    // when the answer went out. That framing is held equal across the large bodies, on a
    // signature door and a bearer door alike, and then left out of the comparison with the
    // small-body constant.
    let mut answers = raw_refusals(&app).await;
    let large = app
        .client
        .post(app.url("/api/profile"))
        .body(vec![b'x'; 70 * 1024])
        .send()
        .await
        .expect("sent");
    answers.push((
        "70 KiB body to a bearer door".to_string(),
        answer_of(large).await,
    ));
    let framing: Vec<_> = answers
        .iter()
        .filter(|(what, _)| what.starts_with("70 KiB"))
        .map(|(what, answer)| (what.clone(), answer.header("connection").map(str::to_owned)))
        .collect();
    assert!(
        framing.windows(2).all(|w| w[0].1 == w[1].1),
        "the large bodies are framed alike on every door: {framing:?}"
    );
    for (what, answer) in &mut answers {
        if what.starts_with("70 KiB") {
            answer.headers.remove("connection");
        }
    }
    let head = send(&app, Method::HEAD, "/api/profile", &[]).await;
    assert_eq!(head.body, "", "HEAD carries no body");
    assert_eq!(
        head.header("content-length"),
        Some(UNAVAILABLE_BODY.len().to_string().as_str()),
        "HEAD advertises the unavailable body's length: {head:?}"
    );
    answers.push(("the priming request".to_string(), primed_again(&app).await));
    assert_refusals(&answers, &unavailable());

    // Only a pre-identity refusal is swapped: a 415 (answered before any gate) and a public
    // route's 405 keep their own answers.
    let encoding = send(
        &app,
        Method::POST,
        "/api/resources",
        &[("content-encoding", "foo")],
    )
    .await;
    assert_eq!(encoding.status, 415, "{encoding:?}");
    let not_allowed = send(&app, Method::POST, "/api/health", &[]).await;
    assert_eq!(not_allowed.status, 405, "{not_allowed:?}");
    assert!(!not_allowed.body.contains("unavailable"), "{not_allowed:?}");
}

async fn primed_again(app: &common::TestApp) -> Answer {
    send(
        app,
        Method::GET,
        "/api/profile",
        &[("authorization", "Bearer garbage")],
    )
    .await
}

// FAILS IF: on a host other than the pinned cron host, a cron door answers anything but `U401`: the
// pin's old `404 "not found"`, a `405` with `Allow` for an unmounted method, or an extractor's
// `400`, holding the secret or not.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_cron_host_pin_is_the_uniform_refusal(pool: PgPool) {
    let app = common::setup_test_app_with_config(pool.clone(), |config| {
        config.cron_host = Some("cron.internal.test".to_string());
        config.embed_dispatch_secret = Some("dispatch-secret".to_string());
    })
    .await;
    let mut answers = Vec::new();
    for (method, path) in [
        (Method::GET, "/api/embed/dispatch"),
        (Method::POST, "/api/embed/dispatch"),
        (Method::PUT, "/api/embed/dispatch"),
        (Method::GET, "/api/embed/dispatch?shard=abc"),
        (Method::GET, "/api/embed/warm"),
        (Method::POST, "/api/as/reap"),
    ] {
        for bearer in [None, Some("Bearer dispatch-secret")] {
            let mut headers = vec![("host", "public.example".to_string())];
            if let Some(b) = bearer {
                headers.push(("authorization", b.to_string()));
            }
            let answer = send_owned(&app, method.clone(), path, &headers).await;
            answers.push((format!("{method} {path} {bearer:?}"), answer));
        }
    }
    assert_refusals(&answers, &u401());
}

/// Send `head` (a request without its body) on a raw socket and return the first status line the
/// server answers within `wait`, or `None` if it answers nothing in time.
async fn first_status_line(
    app: &common::TestApp,
    head: &str,
    wait: std::time::Duration,
) -> Option<String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(app.addr)
        .await
        .expect("connect");
    stream.write_all(head.as_bytes()).await.expect("write");
    let mut buf = vec![0u8; 256];
    match tokio::time::timeout(wait, stream.read(&mut buf)).await {
        Ok(Ok(n)) if n > 0 => {
            let text = String::from_utf8_lossy(&buf[..n]).into_owned();
            text.lines().next().map(str::to_string)
        }
        _ => None,
    }
}

// FAILS IF: the intake reads (or asks for) the body before its gate refuses an unattested caller:
// `Expect: 100-continue` would draw `100 Continue`, and a declared body that never arrives would
// stall, where every other refused path answers the 401 at once.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn intake_refuses_before_the_body(pool: PgPool) {
    let app = allowlisted_app(&pool).await;
    let wait = std::time::Duration::from_millis(1500);
    for path in ["/api/intake/webhook", "/api/profile"] {
        let expect = format!(
            "POST {path} HTTP/1.1\r\nhost: x\r\ncontent-length: 10\r\nexpect: 100-continue\r\n\r\n"
        );
        let line = first_status_line(&app, &expect, wait).await;
        assert_eq!(
            line.as_deref(),
            Some("HTTP/1.1 401 Unauthorized"),
            "{path} with Expect: 100-continue"
        );
        let stalled = format!("POST {path} HTTP/1.1\r\nhost: x\r\ncontent-length: 1000\r\n\r\n");
        let line = first_status_line(&app, &stalled, wait).await;
        assert_eq!(
            line.as_deref(),
            Some("HTTP/1.1 401 Unauthorized"),
            "{path} with a declared body that never arrives"
        );
    }
}

// ---- The key store's outage state is server-wide, bounded and self-healing ----------------------

/// What the test's JWKS endpoint does with a fetch.
#[derive(Clone, Copy, Debug)]
enum KeyEndpoint {
    /// Answer 500.
    Refuse,
    /// Serve a one-key document.
    Serve,
    /// Accept the connection and never answer: the fetch runs into the client's timeout.
    Hang,
}

/// A JWKS endpoint the test switches between behaviours. Returns its URL.
async fn key_endpoint(mode: std::sync::Arc<std::sync::Mutex<KeyEndpoint>>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // RFC 8037's example Ed25519 public key: any well-formed key will do, nothing verifies with it.
    const DOCUMENT: &str = r#"{"keys":[{"kty":"OKP","crv":"Ed25519","kid":"k","x":"11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo"}]}"#;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind the key endpoint");
    let addr = listener.local_addr().expect("its address");
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mode = *mode.lock().expect("mode lock");
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;
                let reply = match mode {
                    KeyEndpoint::Refuse => {
                        "HTTP/1.1 500 Internal Server Error\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                            .to_string()
                    }
                    KeyEndpoint::Serve => format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{DOCUMENT}",
                        DOCUMENT.len()
                    ),
                    KeyEndpoint::Hang => {
                        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                        return;
                    }
                };
                let _ = socket.write_all(reply.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}/.well-known/jwks.json")
}

/// Serve an already-built router (the internal surface) the way the harness serves `create_app`.
async fn serve(router: axum::Router, pool: PgPool) -> common::TestApp {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("address");
    tokio::spawn(async move {
        axum::serve(listener, router).await.expect("serve");
    });
    common::TestApp {
        addr,
        pool,
        client: reqwest::Client::new(),
    }
}

/// The public app with its bearer key store pointed at `url`.
async fn app_with_key_endpoint(pool: &PgPool, url: String) -> common::TestApp {
    common::setup_test_app_with_state(pool.clone(), |state| {
        state.jwks_store = std::sync::Arc::new(temper_services::state::JwksKeyStore::new(url));
    })
    .await
}

/// Re-send `request` until its answer (CORS aside) is `expected`, within `within`. The store's
/// state changes on a background refresh, so the first answers may still be the old state's.
async fn until_answered<F, Fut>(
    expected: &Answer,
    within: std::time::Duration,
    request: F,
) -> Answer
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Answer>,
{
    let deadline = std::time::Instant::now() + within;
    loop {
        let answer = request().await;
        if &without_cors(&answer) == expected {
            return answer;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "never answered {expected:?}; last {answer:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// A signature-door refusal, which never reaches the bearer key store.
async fn signed_refusal(app: &common::TestApp, path: &str) -> Answer {
    let mut request = app.client.post(app.url(path)).body("{}");
    for (n, v) in bad_signature() {
        request = request.header(n, v);
    }
    answer_of(request.send().await.expect("sent")).await
}

// FAILS IF: an instance whose routes never verify a bearer (the internal surface, which keeps its
// own key store in a deployment that splits it out) answers the uniform refusal through an outage
// the bearer surface reports "unavailable" for: the outage would tell a prober which function
// served the path.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_internal_app_no_bearer_reaches_still_reports_the_outage(pool: PgPool) {
    let mut state = common::test_state_with_config(pool.clone(), |_| {}).await;
    // Nothing listens on port 1, and nothing in this app ever verifies a bearer.
    state.jwks_store = std::sync::Arc::new(temper_services::state::JwksKeyStore::new(
        "http://127.0.0.1:1/.well-known/jwks.json".to_string(),
    ));
    let app = serve(temper_api::create_internal_app(state), pool).await;

    // Refusals alone poke the store, and the failed refresh flips what every refusal answers.
    until_answered(&unavailable(), std::time::Duration::from_secs(5), || {
        signed_refusal(&app, "/internal/slack/mint")
    })
    .await;
    let mut answers = Vec::new();
    for path in ["/internal/slack/mint", "/internal/saml/reconcile"] {
        answers.push((
            format!("bad signature to {path}"),
            signed_refusal(&app, path).await,
        ));
        answers.push((
            format!("no signature to {path}"),
            send(&app, Method::POST, path, &[]).await,
        ));
    }
    assert_refusals(&answers, &unavailable());
}

// FAILS IF: a JWKS endpoint that accepts and never answers holds a bearer refusal and one with
// no bearer differently (a prober could tell which paths verify a bearer), answers either before
// the fetch it joined has ended, or answers either as anything but "unavailable": before the store
// has failed, inside the backoff, and once a fresh fetch past the backoff hangs again.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_hanging_key_endpoint_holds_every_refusal_alike(pool: PgPool) {
    let mode = std::sync::Arc::new(std::sync::Mutex::new(KeyEndpoint::Hang));
    let app = app_with_key_endpoint(&pool, key_endpoint(mode).await).await;
    let wait = temper_services::state::JwksKeyStore::REFRESH_WAIT;

    async fn timed(
        app: &common::TestApp,
        headers: &[(&str, &str)],
    ) -> (std::time::Duration, Answer) {
        let started = std::time::Instant::now();
        let answer = send(app, Method::GET, "/api/profile", headers).await;
        (started.elapsed(), answer)
    }
    // A bearer refusal and one without, sent together so both meet the same store state.
    async fn pair(app: &common::TestApp) -> [(std::time::Duration, Answer); 2] {
        let (bearer, bare) = tokio::join!(
            timed(app, &[("authorization", "Bearer garbage")]),
            timed(app, &[])
        );
        [bearer, bare]
    }
    let check = |phase: &str, answers: &[(std::time::Duration, Answer); 2], at_least| {
        let [(bearer, b), (bare, n)] = answers;
        for (door, held, answer) in [("bearer", bearer, b), ("no bearer", bare, n)] {
            assert_eq!(&without_cors(answer), &unavailable(), "{phase}: {door}");
            assert!(
                *held < wait + std::time::Duration::from_secs(1),
                "{phase}: {door} held {held:?}"
            );
            assert!(
                *held >= at_least,
                "{phase}: {door} answered after {held:?}, before its fetch ended"
            );
        }
        let apart = if bearer > bare {
            *bearer - *bare
        } else {
            *bare - *bearer
        };
        assert!(
            apart < std::time::Duration::from_millis(750),
            "{phase}: bearer {bearer:?} against no bearer {bare:?}"
        );
    };
    // The fetch's own timeout: a refusal that joined it waits until it gives up.
    let fetch = std::time::Duration::from_millis(4500);

    // Cold: both join the one hanging fetch and answer when it gives up, failed.
    check("cold, fetch hanging", &pair(&app).await, fetch);
    // Inside the backoff the failed refresh answers for everyone, at once.
    check(
        "failing, inside the backoff",
        &pair(&app).await,
        std::time::Duration::ZERO,
    );
    // Past it, both join a fresh fetch, which hangs too.
    tokio::time::sleep(std::time::Duration::from_millis(5500)).await;
    check("failing, fetch hanging again", &pair(&app).await, fetch);
}

/// An app whose key endpoint refuses until the store is failing, then serves again, with the
/// failed-refresh backoff waited out. No bearer is sent on the way.
async fn failed_then_recovered(pool: &PgPool) -> common::TestApp {
    let mode = std::sync::Arc::new(std::sync::Mutex::new(KeyEndpoint::Refuse));
    let app = app_with_key_endpoint(pool, key_endpoint(mode.clone()).await).await;
    until_answered(&unavailable(), std::time::Duration::from_secs(5), || {
        signed_refusal(&app, "/internal/slack/mint")
    })
    .await;
    *mode.lock().expect("mode lock") = KeyEndpoint::Serve;
    tokio::time::sleep(std::time::Duration::from_millis(5500)).await;
    app
}

// FAILS IF: a failing store recovers only when a bearer drives a refresh: with no bearer traffic
// (a signature door's callers, a prober) the instance would answer "unavailable" for as long as the
// outage's last bearer left it, after the endpoint is back. No bearer is sent at all.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_failing_store_recovers_with_no_bearer(pool: PgPool) {
    let app = failed_then_recovered(&pool).await;
    // The request that runs the recovering refresh arrived while the store was failing.
    let first = signed_refusal(&app, "/internal/slack/mint").await;
    assert_eq!(
        without_cors(&first),
        unavailable(),
        "the recovering request"
    );
    let answers = vec![
        (
            "signature door".to_string(),
            signed_refusal(&app, "/internal/slack/mint").await,
        ),
        (
            "no bearer".to_string(),
            send(&app, Method::GET, "/api/profile", &[]).await,
        ),
    ];
    assert_refusals(&answers, &u401());
}

// FAILS IF: the request that sees the recovery answers by its door: on a bearer door its own
// lookup runs the recovering refresh, and if "was failing" were read after that, it alone would
// answer the constant refusal while every other door's first request says "unavailable".
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_recovering_request_answers_unavailable_on_a_bearer_door(pool: PgPool) {
    let app = failed_then_recovered(&pool).await;
    let first = send(
        &app,
        Method::GET,
        "/api/profile",
        &[("authorization", "Bearer garbage")],
    )
    .await;
    assert_eq!(
        without_cors(&first),
        unavailable(),
        "the recovering request"
    );
    let next = send(
        &app,
        Method::GET,
        "/api/profile",
        &[("authorization", "Bearer garbage")],
    )
    .await;
    assert_eq!(without_cors(&next), u401(), "after recovery");
}

// ---- The signed-body equaliser: a signed request's body is read alike on every path -------------

/// The representative paths that are not public. A public route answers by its own rules, and an
/// unmatched one is still the 404 fallback until the declared fallback lands, so the unsigned arms
/// below assert timing on every path and the status only where a gate refuses.
fn non_public_paths() -> Vec<String> {
    representative_paths()
        .into_iter()
        .filter(|p| p != "/api/health" && p != "/api/auth/slack/callback")
        .collect()
}

fn is_unmatched(path: &str) -> bool {
    path.ends_with("/no-such")
}

/// The three signature headers as raw request lines, valid in shape and not verifying.
fn signature_lines() -> String {
    bad_signature()
        .into_iter()
        .map(|(n, v)| format!("{n}: {v}\r\n"))
        .collect()
}

/// Two of the three signature headers: not a signed request, so never read early.
fn two_of_three_lines() -> String {
    bad_signature()
        .into_iter()
        .skip(1)
        .map(|(n, v)| format!("{n}: {v}\r\n"))
        .collect()
}

/// How long an answer that is not waiting on a body may take. Generous: every refusal passes the
/// key store's refresh check, and a loaded runner is slow.
const AT_ONCE: std::time::Duration = std::time::Duration::from_secs(2);

/// How long a waiting request is watched before it is called waiting.
const WAITING: std::time::Duration = std::time::Duration::from_millis(500);

/// Send a raw request and read until the server closes, reading concurrently with the write so an
/// early answer is never lost to a reset when the server closes on unread bytes.
async fn raw_exchange(app: &common::TestApp, head: String, body: Vec<u8>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let stream = tokio::net::TcpStream::connect(app.addr)
        .await
        .expect("connect");
    let (mut read, mut write) = stream.into_split();
    let reader = tokio::spawn(async move {
        let mut raw = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(5), read.read(&mut buf)).await
            {
                Ok(Ok(0)) | Ok(Err(_)) | Err(_) => break,
                Ok(Ok(n)) => raw.extend_from_slice(&buf[..n]),
            }
        }
        raw
    });
    let _ = write.write_all(head.as_bytes()).await;
    let _ = write.write_all(&body).await;
    String::from_utf8_lossy(&reader.await.expect("reader")).into_owned()
}

// FAILS IF: a signed request's body is read on some paths and not others. A declared body that
// never arrives would then stall only where a signature gate reads it, which names the doors.
// Unsigned, or with only two of the three headers, nothing reads it before refusing, so every path
// answers at once.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn signed_requests_wait_alike(pool: PgPool) {
    let app = allowlisted_app(&pool).await;
    for path in representative_paths() {
        let signed = format!(
            "POST {path} HTTP/1.1\r\nhost: x\r\ncontent-length: 1000\r\n{}\r\n",
            signature_lines()
        );
        assert_eq!(
            first_status_line(&app, &signed, WAITING).await,
            None,
            "{path}: a signed request with a declared body must wait for it"
        );
        for (what, lines) in [
            ("unsigned", String::new()),
            ("two of three", two_of_three_lines()),
        ] {
            let head =
                format!("POST {path} HTTP/1.1\r\nhost: x\r\ncontent-length: 1000\r\n{lines}\r\n");
            let line = first_status_line(&app, &head, AT_ONCE).await;
            assert!(line.is_some(), "{path} {what}: must answer at once");
            if non_public_paths().contains(&path) && !is_unmatched(&path) {
                assert_eq!(
                    line.as_deref(),
                    Some("HTTP/1.1 401 Unauthorized"),
                    "{path} {what}"
                );
            }
        }
    }
}

// FAILS IF: a signed body that never arrives is waited on forever, or the deadline answers with
// anything but the uniform refusal. Without it, a caller dripping a signed body holds a connection
// on every path for as long as it likes.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_signed_body_that_never_arrives_is_refused_at_the_deadline(pool: PgPool) {
    let app = allowlisted_app(&pool).await;
    for path in ["/api/profile", "/internal/saml/reconcile", "/api/health"] {
        let head = format!(
            "POST {path} HTTP/1.1\r\nhost: x\r\ncontent-length: 1000\r\n{}\r\n",
            signature_lines()
        );
        let line = first_status_line(&app, &head, std::time::Duration::from_secs(15)).await;
        assert_eq!(line.as_deref(), Some("HTTP/1.1 401 Unauthorized"), "{path}");
    }
}

// FAILS IF: `Expect: 100-continue` on a signed request draws `100 Continue` on some paths and the
// refusal on others: the first line would name the paths whose gate reads the body. A declared
// length over the cap is refused before any `100`, on every path.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn signed_expect_continue_is_uniform(pool: PgPool) {
    let app = allowlisted_app(&pool).await;
    let wait = std::time::Duration::from_millis(1500);
    for (length, expected) in [
        (10, "HTTP/1.1 100 Continue"),
        (70 * 1024, "HTTP/1.1 401 Unauthorized"),
    ] {
        for path in representative_paths() {
            let head = format!(
                "POST {path} HTTP/1.1\r\nhost: x\r\ncontent-length: {length}\r\n\
                 expect: 100-continue\r\n{}\r\n",
                signature_lines()
            );
            assert_eq!(
                first_status_line(&app, &head, wait).await.as_deref(),
                Some(expected),
                "{path}, content-length {length}"
            );
        }
    }
}

// FAILS IF: a signed body over the cap is refused only where a gate reads it, or with anything but
// the uniform 401, or a public route lets it through; declared, chunked, or with an `Origin`, whose
// CORS headers must be those a routed refusal carries for the same origin.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_70_kib_signed_body_is_u401_everywhere(pool: PgPool) {
    let app = allowlisted_app(&pool).await;
    let body = "x".repeat(70 * 1024);
    for origin in [None, Some(ORIGIN), Some("https://foreign.example")] {
        let mut answers = Vec::new();
        let routed = {
            let mut headers = Vec::new();
            if let Some(origin) = origin {
                headers.push(("origin", origin.to_string()));
            }
            send_owned(&app, Method::POST, "/api/profile", &headers).await
        };
        for path in representative_paths() {
            let mut declared = app.client.post(app.url(&path)).body(body.clone());
            for (n, v) in bad_signature() {
                declared = declared.header(n, v);
            }
            if let Some(origin) = origin {
                declared = declared.header("origin", origin);
            }
            let answer = answer_of(declared.send().await.expect("sent")).await;
            assert_eq!(
                cors_headers(&answer),
                cors_headers(&routed),
                "{path} {origin:?}: the CORS headers of a routed refusal"
            );
            answers.push((format!("{path} {origin:?}"), answer));
        }
        // Refused on its declared length, so its body is never read and the connection closes:
        // framed alike on every path, then left out of the comparison with the constant.
        assert_connection_alike_then_drop(&mut answers);
        assert_refusals(&answers, &u401());
    }

    for path in representative_paths() {
        // Chunked: no declared length, so the cap is met while reading.
        let head = format!(
            "POST {path} HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\
             transfer-encoding: chunked\r\n{}\r\n{:x}\r\n",
            signature_lines(),
            body.len()
        );
        let mut chunk = body.clone().into_bytes();
        chunk.extend_from_slice(b"\r\n0\r\n\r\n");
        let raw = raw_exchange(&app, head, chunk).await;
        assert!(
            raw.starts_with("HTTP/1.1 401 Unauthorized"),
            "{path} chunked: {raw}"
        );
        assert!(raw.ends_with(&u401().body), "{path} chunked: {raw}");
    }
}

// FAILS IF: an oversize signed request, refused before routing, is not swapped to "unavailable"
// while the key store is failing, as every other pre-identity refusal is.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_oversize_signed_body_is_unavailable_while_the_store_fails(pool: PgPool) {
    let app = common::setup_test_app_with_state(pool.clone(), |state| {
        state.jwks_store = std::sync::Arc::new(temper_services::state::JwksKeyStore::new(
            "http://127.0.0.1:1/.well-known/jwks.json".to_string(),
        ));
    })
    .await;
    let primed = send(
        &app,
        Method::GET,
        "/api/profile",
        &[("authorization", "Bearer garbage")],
    )
    .await;
    let mut answers = Vec::new();
    for path in ["/api/profile", "/internal/saml/reconcile", "/api/health"] {
        let mut request = app.client.post(app.url(path)).body("x".repeat(70 * 1024));
        for (n, v) in bad_signature() {
            request = request.header(n, v);
        }
        answers.push((
            path.to_string(),
            answer_of(request.send().await.expect("sent")).await,
        ));
    }
    assert_connection_alike_then_drop(&mut answers);
    answers.push(("the priming request".to_string(), primed));
    assert_refusals(&answers, &unavailable());
}

/// A refusal that leaves a body unread closes its connection. That framing must be the same on
/// every path in `answers`; it is then removed so the rest compares with the small-body constant.
fn assert_connection_alike_then_drop(answers: &mut [(String, Answer)]) {
    let framing: Vec<_> = answers
        .iter()
        .map(|(what, a)| (what.clone(), a.header("connection").map(str::to_owned)))
        .collect();
    assert!(
        framing.windows(2).all(|w| w[0].1 == w[1].1),
        "framed alike on every path: {framing:?}"
    );
    for (_, answer) in answers.iter_mut() {
        answer.headers.remove("connection");
    }
}

/// A resolve request signed with the test app's reconcile secret.
fn signed_resolve() -> (String, Vec<(&'static str, String)>) {
    let body = format!(
        r#"{{"external_user_id":"eq-{}","email":"eq@example.test","email_verified":true}}"#,
        Uuid::new_v4().simple()
    );
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let nonce = Uuid::new_v4().simple().to_string();
    let signature = temper_core::internal_sig::sign_v2(
        b"reconcile-secret",
        "POST",
        temper_core::internal_sig::DOOR_PRINCIPAL_RESOLVE,
        now,
        &nonce,
        body.as_bytes(),
    );
    let headers = vec![
        (temper_core::internal_sig::TIMESTAMP_HEADER, now.to_string()),
        (temper_core::internal_sig::NONCE_HEADER, nonce),
        (temper_core::internal_sig::SIGNATURE_V2_HEADER, signature),
        ("content-type", "application/json".to_string()),
    ];
    (body, headers)
}

// FAILS IF: the equaliser loses or alters a signed body on its way to the gate, declared or
// chunked, or an admitted caller that asks to continue is not continued. Any of these breaks every
// real signer.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_signed_request_reaches_its_handler_with_and_without_expect(pool: PgPool) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let app = allowlisted_app(&pool).await;

    let (body, headers) = signed_resolve();
    let mut request = app
        .client
        .post(app.url("/internal/principal/resolve"))
        .body(body);
    for (n, v) in headers {
        request = request.header(n, v);
    }
    let answer = answer_of(request.send().await.expect("sent")).await;
    assert_eq!(answer.status, 200, "{answer:?}");
    assert!(answer.body.contains("profile_id"), "{answer:?}");

    let (body, headers) = signed_resolve();
    let lines: String = headers
        .iter()
        .map(|(n, v)| format!("{n}: {v}\r\n"))
        .collect();
    let head = format!(
        "POST /internal/principal/resolve HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\
         transfer-encoding: chunked\r\n{lines}\r\n"
    );
    let chunked = format!("{:x}\r\n{body}\r\n0\r\n\r\n", body.len()).into_bytes();
    let raw = raw_exchange(&app, head, chunked).await;
    assert!(raw.contains("HTTP/1.1 200 OK"), "chunked: {raw}");
    assert!(raw.contains("profile_id"), "chunked: {raw}");

    let (body, headers) = signed_resolve();
    let lines: String = headers
        .iter()
        .map(|(n, v)| format!("{n}: {v}\r\n"))
        .collect();
    let head = format!(
        "POST /internal/principal/resolve HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\
         content-length: {}\r\nexpect: 100-continue\r\n{lines}\r\n",
        body.len()
    );
    let mut stream = tokio::net::TcpStream::connect(app.addr)
        .await
        .expect("connect");
    stream.write_all(head.as_bytes()).await.expect("write head");
    let mut buf = vec![0u8; 64];
    let n = tokio::time::timeout(std::time::Duration::from_secs(5), stream.read(&mut buf))
        .await
        .expect("continued in time")
        .expect("read");
    assert!(
        String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 100 Continue"),
        "{:?}",
        String::from_utf8_lossy(&buf[..n])
    );
    stream.write_all(body.as_bytes()).await.expect("write body");
    let mut rest = Vec::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stream.read_to_end(&mut rest),
    )
    .await
    .expect("answered in time")
    .expect("read");
    let rest = String::from_utf8_lossy(&rest);
    assert!(rest.contains("HTTP/1.1 200 OK"), "{rest}");
    assert!(rest.contains("profile_id"), "{rest}");
}
