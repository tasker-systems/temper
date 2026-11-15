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
    // A stale signature is the same refusal as a bad one.
    let mut stale = bad_signature();
    stale[0].1 = "1".to_string();
    out.push((Method::POST, "/internal/slack/mint".to_string(), stale));
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
}
