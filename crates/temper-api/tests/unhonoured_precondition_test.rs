//! A write carrying `If-Match`, `If-None-Match` or `If-Unmodified-Since` is refused with `400`
//! under `PRECONDITION_NOT_HONOURED`, never applied unconditionally. See
//! `temper_api::middleware::preconditions`.
//!
//! The documented write operations come from [`temper_api::routes::openapi_spec`], so a write route
//! added to a documented group is swept without an edit. The undocumented groups never enter the
//! spec, so their write routes are listed in [`UNDOCUMENTED_WRITES`], and
//! `the_undocumented_list_is_what_the_router_mounts` holds that list to the route files' own
//! source and to the router's own answers.
//!
//! What is witnessed:
//! - every write, in every route family, refuses each header with `400` and leaves the database
//!   exactly as it was, for an authenticated caller and an anonymous one;
//! - the same requests without the header reach a mounted route (never the fallback's `404` or a
//!   `405`) and are not refused, and a real create, update, meta write and delete land as before;
//! - reads (`GET`, `HEAD`, `OPTIONS`) ignore every policed header;
//! - the refusal carries CORS and the security headers, and an unmatched path still answers `404`.
#![cfg(feature = "test-db")]

mod common;

use std::collections::BTreeSet;

use reqwest::Method;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ingest::IngestPayload;

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

/// The paths the undocumented groups mount for `GET` only. They are not write routes, and the
/// router must answer a `POST` to them with `405` (or the refusal, when a header is carried).
const UNDOCUMENTED_GET_ONLY: &[&str] = &["/api/embed/warm", "/api/auth/slack/callback"];

/// The route files that hold the undocumented groups, and the group builders the route table
/// names as `Undocumented`. A new undocumented group fails the check until it is listed here.
const UNDOCUMENTED_SOURCES: &[&str] = &[
    include_str!("../src/routes/internal.rs"),
    include_str!("../src/routes/embed_internal.rs"),
    include_str!("../src/routes/webhook_intake.rs"),
    include_str!("../src/routes/slack_link_public.rs"),
];
const UNDOCUMENTED_GROUPS: &[&str] = &[
    "internal_routes",
    "slack_link_internal_routes",
    "slack_mint_internal_routes",
    "slack_link_public_routes",
    "embed_internal_routes",
    "webhook_intake_routes",
];

/// The write routes of the groups that never enter the spec: the internal HMAC groups, the
/// Vercel crons (which mount the same handler on `GET` and `POST`; the `GET` is a read by method
/// and is not policed), and the webhook intake.
const UNDOCUMENTED_WRITES: &[(&str, &str)] = &[
    ("POST", "/internal/saml/reconcile"),
    ("POST", "/internal/principal/resolve"),
    ("POST", "/internal/slack/link-state"),
    ("POST", "/internal/slack/mint"),
    ("POST", "/api/embed/dispatch"),
    ("POST", "/api/slack/intents/reap"),
    ("POST", "/api/as/reap"),
    ("POST", "/api/internal-calls/health"),
    ("POST", "/api/region/dispatch"),
    ("POST", "/api/erasure/drain"),
    ("POST", "/api/sensitivity/sweep"),
    ("POST", "/api/shape/reconcile"),
    ("POST", "/api/intake/webhook"),
];

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

fn is_write(method: &str) -> bool {
    !matches!(method, "GET" | "HEAD" | "OPTIONS")
}

/// Every write route the app mounts: the documented writes and [`UNDOCUMENTED_WRITES`].
fn write_routes() -> Vec<(String, String)> {
    let mut routes: Vec<(String, String)> = documented_operations()
        .into_iter()
        .filter(|(m, _)| is_write(m))
        .collect();
    routes.extend(
        UNDOCUMENTED_WRITES
            .iter()
            .map(|(m, p)| (m.to_string(), p.to_string())),
    );
    routes
}

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

/// A digest of every row in every user table, in every schema (`public` and `sensitivity` both).
/// Any insert, update or delete changes it.
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

/// An approved person with a home context; returns `(token, context_id)`.
async fn approved_person(pool: &PgPool) -> (String, Uuid) {
    let email = format!("precondition-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(pool, &email).await;
    common::fixtures::approve_standing_by_email(pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);
    (token, context_id)
}

async fn send(
    app: &common::TestApp,
    method: &str,
    path: &str,
    token: Option<&str>,
    header: Option<(&str, &str)>,
    body: &Value,
) -> (u16, String) {
    let method = Method::from_bytes(method.as_bytes()).expect("method");
    let mut request = app.client.request(method.clone(), app.url(path));
    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    if let Some((name, value)) = header {
        request = request.header(name, value);
    }
    if is_write(method.as_str()) {
        request = request.json(body);
    }
    let response = request.send().await.expect("request sent");
    let status = response.status().as_u16();
    (status, response.text().await.unwrap_or_default())
}

/// True when `body` is this layer's refusal, naming `header`.
fn is_precondition_refusal(body: &str, header: &str) -> bool {
    let Ok(parsed) = serde_json::from_str::<Value>(body) else {
        return false;
    };
    let named = match header {
        "if-match" => "the If-Match header is not honoured on this route",
        "if-none-match" => "the If-None-Match header is not honoured on this route",
        _ => "the If-Unmodified-Since header is not honoured on this route",
    };
    parsed["error"]["code"] == json!(temper_core::error::PRECONDITION_NOT_HONOURED_CODE)
        && parsed["error"]["message"]
            .as_str()
            .is_some_and(|m| m.contains(named))
}

/// Assert a refusal: `400`, the API's envelope, the header named, its value not echoed.
fn assert_refused(route: &str, header: (&str, &str), status: u16, body: &str) {
    assert_eq!(
        status, 400,
        "{route} with {} must be 400; body: {body}",
        header.0
    );
    assert!(
        is_precondition_refusal(body, header.0),
        "{route} with {} must answer the precondition refusal; body: {body}",
        header.0
    );
    assert!(
        !body.contains(header.1),
        "{route}: the refusal must not echo the header's value; body: {body}"
    );
}

// FAILS IF: any write route, in any family, applies or even reaches its handler with a policed
// header present — authenticated or not — or the refusal changes a row.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn every_write_route_refuses_each_policed_header_and_writes_nothing(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, context_id) = approved_person(&pool).await;
    let routes = write_routes();
    assert!(
        routes.len() > 50,
        "the sweep found only {} write routes; the spec read is broken",
        routes.len()
    );
    for family in ["/api/resources/{id}", "/api/ingest", "/api/admin/"] {
        assert!(
            routes.iter().any(|(_, p)| p.starts_with(family)),
            "no write route under {family}: the sweep lost a family"
        );
    }

    // A body the routes would act on where they can: it names a real, writable context.
    let body = json!({ "context_ref": context_id.to_string(), "title": "precondition sweep" });
    let before = database_fingerprint(&pool).await;
    for (method, template) in &routes {
        let path = concrete(template);
        let route = format!("{method} {template}");
        for header in POLICED {
            for caller in [Some(token.as_str()), None] {
                let (status, text) = send(&app, method, &path, caller, Some(header), &body).await;
                assert_refused(&route, header, status, &text);
            }
        }
    }
    assert_eq!(
        database_fingerprint(&pool).await,
        before,
        "a refused write changed the database"
    );
}

/// True when `body` is the app fallback's answer: no route matched the path at all.
fn is_unmatched(status: u16, body: &str) -> bool {
    status == 404 && body.contains("No route matches")
}

// FAILS IF: the layer refuses a write that carries no policed header, or a swept route is not
// mounted with that method — a hand-listed route whose write method was removed answers the
// fallback's `404` or a `405`, so the sweep's refusals would prove nothing about it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn without_the_header_no_write_route_meets_the_refusal(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, _) = approved_person(&pool).await;
    for (method, template) in write_routes() {
        let path = concrete(&template);
        let (status, text) = send(&app, &method, &path, Some(&token), None, &json!({})).await;
        assert!(
            status != 405 && !is_unmatched(status, &text),
            "{method} {template} is not mounted ({status}): {text}"
        );
        for (header, _) in POLICED {
            assert!(
                !is_precondition_refusal(&text, header),
                "{method} {template} without a precondition header was refused ({status}): {text}"
            );
        }
    }
}

// FAILS IF: the internal app — the separate Vercel function — serves a write with a policed
// header unrefused.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_internal_app_refuses_too(pool: PgPool) {
    use tower::ServiceExt;

    let state = common::test_state_with_config(pool.clone(), |_| {}).await;
    let app = temper_api::create_internal_app(state);
    for (method, path) in UNDOCUMENTED_WRITES
        .iter()
        .filter(|(_, p)| *p != "/api/intake/webhook")
    {
        for (name, value) in POLICED {
            let request = axum::http::Request::builder()
                .method(*method)
                .uri(*path)
                .header(name, value)
                .header("content-type", "application/json")
                .body(axum::body::Body::from("{}"))
                .expect("request");
            let response = app.clone().oneshot(request).await.expect("response");
            let status = response.status().as_u16();
            let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
                .await
                .expect("body");
            let text = String::from_utf8_lossy(&bytes);
            assert_refused(
                &format!("internal {method} {path}"),
                (name, value),
                status,
                &text,
            );
        }
    }
}

// FAILS IF: a read meets the refusal, or answers differently, because it carries a policed header.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reads_ignore_the_headers(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, _) = approved_person(&pool).await;
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
            let headers = POLICED.iter().copied();
            // The status without any header is the baseline. A HEAD answer has no body, so the
            // status is the only thing that can show a refusal there.
            let (plain, _) = send(&app, verb, &path, Some(&token), None, &json!({})).await;
            for header in headers {
                let (status, text) =
                    send(&app, verb, &path, Some(&token), Some(header), &json!({})).await;
                assert_eq!(
                    status, plain,
                    "{verb} {template} with {} answered {status}, without it {plain}: {text}",
                    header.0
                );
                for (policed, _) in POLICED {
                    assert!(
                        !is_precondition_refusal(&text, policed),
                        "{verb} {template} with {} met the refusal ({status}): {text}",
                        header.0
                    );
                }
            }
        }
    }

    // A real read answers exactly as it does without the headers.
    let plain = send(&app, "GET", "/api/profile", Some(&token), None, &json!({})).await;
    for header in POLICED {
        let guarded = send(
            &app,
            "GET",
            "/api/profile",
            Some(&token),
            Some(header),
            &json!({}),
        )
        .await;
        assert_eq!(
            guarded, plain,
            "GET /api/profile with {} changed its answer",
            header.0
        );
    }
}

/// The path literals a route file mounts: quoted strings starting with `/` on non-comment lines
/// above its test module. The same reading `routes::embed_internal`'s own test makes.
fn mounted_paths(source: &str) -> BTreeSet<String> {
    source
        .split("#[cfg(test)]")
        .next()
        .expect("source precedes tests")
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with("//"))
        .flat_map(|line| line.split('"').skip(1).step_by(2))
        .filter(|literal| literal.starts_with('/'))
        .map(str::to_string)
        .collect()
}

// FAILS IF: an undocumented group gains a route, or loses one, without `UNDOCUMENTED_WRITES` or
// `UNDOCUMENTED_GET_ONLY` following; a GET-only path starts answering a write; or the route table
// gains an undocumented group whose file this test does not read.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_undocumented_list_is_what_the_router_mounts(pool: PgPool) {
    // The table rows are one greppable line each (the audit scripts rely on the same).
    let table: BTreeSet<String> = include_str!("../src/routes/mod.rs")
        .lines()
        .filter_map(|line| line.split("build: Undocumented(").nth(1))
        .filter_map(|rest| rest.split(')').next())
        .map(str::to_string)
        .collect();
    let named: BTreeSet<String> = UNDOCUMENTED_GROUPS.iter().map(|g| g.to_string()).collect();
    assert_eq!(table, named, "the route table's undocumented groups");

    let mounted: BTreeSet<String> = UNDOCUMENTED_SOURCES
        .iter()
        .flat_map(|source| mounted_paths(source))
        .collect();
    let listed: BTreeSet<String> = UNDOCUMENTED_WRITES
        .iter()
        .map(|(_, p)| p.to_string())
        .chain(UNDOCUMENTED_GET_ONLY.iter().map(|p| p.to_string()))
        .collect();
    assert_eq!(mounted, listed, "undocumented paths mounted vs listed");

    // The router's own answer settles which listed paths are GET-only: a write without a header is
    // `405`, and with one it is the refusal, which runs before the method router's fallback.
    let app = common::setup_test_app(pool.clone()).await;
    for path in UNDOCUMENTED_GET_ONLY {
        let (status, text) = send(&app, "POST", path, None, None, &json!({})).await;
        assert_eq!(status, 405, "POST {path} must be unmounted: {text}");
        let header = POLICED[0];
        let (status, text) = send(&app, "POST", path, None, Some(header), &json!({})).await;
        assert_refused(&format!("POST {path}"), header, status, &text);
    }
}

// FAILS IF: the layer moves outside the base layers or CORS — the refusal would then lose
// `Access-Control-Allow-Origin` and the security headers — or before the fallback, so an
// unmatched path carrying a policed header would answer `400` instead of `404`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_refusal_carries_cors_and_the_security_headers_and_misses_no_fallback(pool: PgPool) {
    const ORIGIN: &str = "https://allowed.example";
    let app = common::setup_test_app_with_config(pool.clone(), |config| {
        config.cors_origins = vec![ORIGIN.to_string()];
    })
    .await;

    for header in POLICED {
        let response = app
            .client
            .post(app.url("/api/ingest"))
            .header("Origin", ORIGIN)
            .header(header.0, header.1)
            .json(&json!({}))
            .send()
            .await
            .expect("request sent");
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let text = response.text().await.unwrap_or_default();
        assert_refused(
            "POST /api/ingest from an allowed origin",
            header,
            status,
            &text,
        );
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

        let (status, text) = send(
            &app,
            "POST",
            "/api/no-such-route-anywhere",
            None,
            Some(header),
            &json!({}),
        )
        .await;
        assert!(
            is_unmatched(status, &text),
            "an unmatched path with {} must answer the fallback's 404 ({status}): {text}",
            header.0
        );
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

async fn send_json(
    app: &common::TestApp,
    method: Method,
    path: &str,
    token: &str,
    header: Option<(&str, &str)>,
    body: Value,
) -> (u16, Value) {
    let mut request = app
        .client
        .request(method, app.url(path))
        .header("Authorization", format!("Bearer {token}"));
    if !body.is_null() {
        request = request.json(&body);
    }
    if let Some((name, value)) = header {
        request = request.header(name, value);
    }
    let response = request.send().await.expect("request sent");
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

async fn resource_row(app: &common::TestApp, token: &str, id: &str) -> (u16, Value) {
    send_json(
        app,
        Method::GET,
        &format!("/api/resources/{id}"),
        token,
        None,
        Value::Null,
    )
    .await
}

// FAILS IF: a create, a PATCH, a meta write or a delete carrying a policed header lands, or one
// without it stops landing. The sweep above proves the refusal is uniform; this proves on real
// rows that it is the header, and only the header, that decides.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn real_writes_land_without_the_header_and_not_with_it(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;
    let (token, context_id) = approved_person(&pool).await;

    for header in POLICED {
        // Create.
        let payload = serde_json::to_value(ingest_payload(context_id, "Refused create")).unwrap();
        let before = database_fingerprint(&pool).await;
        let (status, body) = send_json(
            &app,
            Method::POST,
            "/api/ingest",
            &token,
            Some(header),
            payload,
        )
        .await;
        assert_refused("POST /api/ingest", header, status, &body.to_string());
        assert_eq!(
            database_fingerprint(&pool).await,
            before,
            "refused create wrote"
        );
    }

    let payload = serde_json::to_value(ingest_payload(context_id, "Original")).unwrap();
    let before = database_fingerprint(&pool).await;
    let (status, created) =
        send_json(&app, Method::POST, "/api/ingest", &token, None, payload).await;
    assert_eq!(status, 200, "create without a header lands: {created}");
    assert_ne!(
        database_fingerprint(&pool).await,
        before,
        "the fingerprint must see a real write, or its silence above proves nothing"
    );
    let id = created["id"].as_str().expect("id").to_string();
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
        let (status, body) = send_json(
            &app,
            Method::PATCH,
            &resource,
            &token,
            Some(header),
            json!({"title": "Renamed"}),
        )
        .await;
        assert_refused(
            "PATCH /api/resources/{id}",
            header,
            status,
            &body.to_string(),
        );
        let (status, body) = send_json(
            &app,
            Method::PUT,
            &meta,
            &token,
            Some(header),
            meta_body.clone(),
        )
        .await;
        assert_refused(
            "PUT /api/resources/{id}/meta",
            header,
            status,
            &body.to_string(),
        );
        let (status, body) = send_json(
            &app,
            Method::DELETE,
            &resource,
            &token,
            Some(header),
            Value::Null,
        )
        .await;
        assert_refused(
            "DELETE /api/resources/{id}",
            header,
            status,
            &body.to_string(),
        );
        assert_eq!(
            database_fingerprint(&pool).await,
            before,
            "a refused write wrote"
        );
    }
    let (_, row) = resource_row(&app, &token, &id).await;
    assert_eq!(
        row["title"],
        json!("Original"),
        "refused PATCH left the title: {row}"
    );

    // The same writes without the header land exactly as before.
    let (status, body) = send_json(
        &app,
        Method::PATCH,
        &resource,
        &token,
        None,
        json!({"title": "Renamed"}),
    )
    .await;
    assert_eq!(status, 200, "PATCH without a header lands: {body}");
    let (_, row) = resource_row(&app, &token, &id).await;
    assert_eq!(row["title"], json!("Renamed"));
    let (status, body) = send_json(&app, Method::PUT, &meta, &token, None, meta_body).await;
    assert_eq!(status, 200, "PUT meta without a header lands: {body}");
    let (status, body) =
        send_json(&app, Method::DELETE, &resource, &token, None, Value::Null).await;
    assert!(
        (200..300).contains(&status),
        "DELETE without a header lands ({status}): {body}"
    );
}
