//! The body token on the wire: `GET /api/resources/{id}/content`'s `body_token` and `ETag`, the
//! `?sections=body` token, and `If-Match` on `PATCH /api/resources/{id}`.
//!
//! The guard's sequence (lock, read standing, judgment, the retried-save no-op) is witnessed
//! against the backend in `temper-services/tests/body_precondition_test.rs`. These pin what only
//! HTTP can show: which answers carry an `ETag`, that no `304` is ever synthesized, the header
//! rules' `400`s and `412`s, the `422` for `expect.body` on `PUT /meta`, and the exact bytes
//! `/content` serves for each storage shape.
#![cfg(feature = "test-db")]

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

struct Caller {
    token: String,
    resource: String,
}

/// What a request answered: the status, the `ETag` header, and the body.
#[derive(Debug)]
struct Answer {
    status: u16,
    etag: Option<String>,
    body: Value,
}

async fn send(
    app: &common::TestApp,
    token: Option<&str>,
    method: reqwest::Method,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> Answer {
    let mut request = app.client.request(method, app.url(path));
    if let Some(token) = token {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    if let Some(body) = body {
        request = request.json(&body);
    }
    let resp = request.send().await.expect("request");
    let status = resp.status().as_u16();
    let etag = resp
        .headers()
        .get("etag")
        .map(|v| v.to_str().expect("an ASCII ETag").to_string());
    let text = resp.text().await.expect("body");
    Answer {
        status,
        etag,
        body: serde_json::from_str(&text).unwrap_or(Value::String(text)),
    }
}

async fn person(app: &common::TestApp) -> (String, Uuid) {
    let email = format!("token-{}@example.com", Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    (
        common::generate_test_jwt(&format!("test|{profile_id}"), &email),
        context_id,
    )
}

/// A resource whose body is `body`, written by an unconditional `PATCH`.
async fn a_resource(app: &common::TestApp, body: &str) -> Caller {
    let (token, context_id) = person(app).await;
    let created = send(
        app,
        Some(&token),
        reqwest::Method::POST,
        "/api/resources",
        &[],
        Some(json!({
            "title": "Tokened",
            "origin_uri": format!("test://token-{}", Uuid::new_v4()),
            "kb_context_id": context_id,
            "doc_type": "research",
        })),
    )
    .await;
    assert_eq!(created.status, 200, "{created:?}");
    let who = Caller {
        token,
        resource: created.body["id"].as_str().expect("id").to_string(),
    };
    let saved = patch(app, &who, &[], json!({"content": body})).await;
    assert_eq!(saved.status, 200, "{saved:?}");
    who
}

async fn content(app: &common::TestApp, who: &Caller, headers: &[(&str, &str)]) -> Answer {
    send(
        app,
        Some(&who.token),
        reqwest::Method::GET,
        &format!("/api/resources/{}/content", who.resource),
        headers,
        None,
    )
    .await
}

async fn patch(
    app: &common::TestApp,
    who: &Caller,
    headers: &[(&str, &str)],
    body: Value,
) -> Answer {
    send(
        app,
        Some(&who.token),
        reqwest::Method::PATCH,
        &format!("/api/resources/{}", who.resource),
        headers,
        Some(body),
    )
    .await
}

fn code(answer: &Answer) -> &str {
    answer.body["error"]["code"].as_str().unwrap_or_default()
}

/// **The `ETag` is on `/content` only, and is the quoted `body_token`.** It is strong, it equals
/// the field, it is answered whatever `If-None-Match` says, and that header never turns the answer
/// into a `304`. The view read answers no `ETag`, with or without the body section; the body
/// section carries the same token as a field. Neither a refusal nor a write's answer carries one.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_etag_is_on_content_only_and_never_304s(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let who = a_resource(&app, "# Doc\n\nthe body\n").await;

    let read = content(&app, &who, &[]).await;
    assert_eq!(read.status, 200, "{read:?}");
    let token = read.body["body_token"]
        .as_str()
        .expect("body_token")
        .to_string();
    assert_eq!(token, temper_core::body_token("# Doc\n\nthe body\n"));
    assert_eq!(read.etag.as_deref(), Some(format!("\"{token}\"").as_str()));
    let members: Vec<&str> = read
        .body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        {
            let mut m = members.clone();
            m.sort();
            m
        },
        vec!["body_token", "markdown", "resource_id"],
        "every field of /content is a function of markdown and the URL"
    );

    // The caller's own read: never a shared cache's to keep.
    let raw = app
        .client
        .get(app.url(&format!("/api/resources/{}/content", who.resource)))
        .header("Authorization", format!("Bearer {}", who.token))
        .send()
        .await
        .expect("request");
    assert_eq!(
        raw.headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("private")
    );
    for inm in [
        format!("\"{token}\""),
        "*".to_string(),
        format!("W/\"{token}\""),
    ] {
        let again = content(&app, &who, &[("if-none-match", &inm)]).await;
        assert_eq!(again.status, 200, "If-None-Match {inm}: {again:?}");
        assert_eq!(again.etag, read.etag);
        assert_eq!(again.body, read.body);
    }

    let view = |sections: &'static str| {
        let path = format!("/api/resources/{}{sections}", who.resource);
        let token = who.token.clone();
        let app = &app;
        async move { send(app, Some(&token), reqwest::Method::GET, &path, &[], None).await }
    };
    let plain = view("").await;
    assert_eq!((plain.status, plain.etag.as_deref()), (200, None));
    assert!(plain.body.get("body_token").is_none(), "{:?}", plain.body);
    let with_body = view("?sections=body").await;
    assert_eq!((with_body.status, with_body.etag.as_deref()), (200, None));
    assert_eq!(with_body.body["body_token"], json!(token));

    let saved = patch(
        &app,
        &who,
        &[("if-match", &format!("\"{token}\""))],
        json!({"content": "# Doc\n\nnext\n"}),
    )
    .await;
    assert_eq!(saved.status, 200, "{saved:?}");
    assert_eq!(saved.etag, None, "a write's answer carries no ETag");
    let next = temper_core::body_token("# Doc\n\nnext\n");
    assert_eq!(saved.body["body_token"], json!(next));

    let refused = patch(
        &app,
        &who,
        &[("if-match", &format!("\"{token}\""))],
        json!({"content": "# Doc\n\nstale\n"}),
    )
    .await;
    assert_eq!(
        (refused.status, code(&refused)),
        (412, "PRECONDITION_FAILED")
    );
    assert_eq!(refused.etag, None, "a refusal carries no ETag");
    assert_eq!(
        refused.body["error"]["precondition"],
        json!({"failed": [{"tier": "body", "expected": token, "current": next}]})
    );
    assert_eq!(
        refused.body["error"]["message"],
        json!("1 precondition failed: body")
    );

    // A reader who cannot see the resource gets the 404 and no ETag.
    let (stranger, _) = person(&app).await;
    let hidden = send(
        &app,
        Some(&stranger),
        reqwest::Method::GET,
        &format!("/api/resources/{}/content", who.resource),
        &[],
        None,
    )
    .await;
    assert_eq!((hidden.status, hidden.etag.as_deref()), (404, None));
}

/// **`If-Match` on `PATCH`, the header rules.** A list matches on any member; a weak tag naming the
/// current body is a `400`; a weak tag naming another is a `412`; `*` is a `400`; disagreeing with
/// `expect.body` is a `400`; `expect.body` itself works the same as the header.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn if_match_follows_the_header_rules(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let who = a_resource(&app, "# Doc\n\nv0\n").await;
    let t0 = content(&app, &who, &[]).await.body["body_token"]
        .as_str()
        .unwrap()
        .to_string();
    let edit = json!({"content": "# Doc\n\nedited\n"});

    let weak = patch(
        &app,
        &who,
        &[("if-match", &format!("W/\"{t0}\""))],
        edit.clone(),
    )
    .await;
    assert_eq!((weak.status, code(&weak)), (400, "BAD_REQUEST"), "{weak:?}");
    let weak_other = patch(&app, &who, &[("if-match", "W/\"bt1.other\"")], edit.clone()).await;
    assert_eq!(weak_other.status, 412, "{weak_other:?}");
    let star = patch(&app, &who, &[("if-match", "*")], edit.clone()).await;
    assert_eq!((star.status, code(&star)), (400, "BAD_REQUEST"), "{star:?}");
    let disagree = patch(
        &app,
        &who,
        &[("if-match", "\"bt1.other\"")],
        json!({"content": "# Doc\n\nedited\n", "expect": {"body": t0}}),
    )
    .await;
    assert_eq!((disagree.status, code(&disagree)), (400, "BAD_REQUEST"));
    assert_eq!(
        content(&app, &who, &[]).await.body["markdown"],
        json!("# Doc\n\nv0\n"),
        "no refusal wrote"
    );

    let list = patch(
        &app,
        &who,
        &[("if-match", &format!("\"bt1.a\", \"{t0}\""))],
        edit.clone(),
    )
    .await;
    assert_eq!(list.status, 200, "a list matches on any member: {list:?}");
    let t1 = list.body["body_token"].as_str().unwrap().to_string();
    let by_field = patch(
        &app,
        &who,
        &[],
        json!({"content": "# Doc\n\nby field\n", "expect": {"body": t1}}),
    )
    .await;
    assert_eq!(by_field.status, 200, "{by_field:?}");
}

/// **`expect.body` exists only on `PATCH`; the other headers stay refused.** `PUT /meta` with a
/// `body` expectation is a `422` (never dropped); `PUT /meta` with `If-Match` is still the layer's
/// `400`; `PATCH` with `If-None-Match` or `If-Unmodified-Since` is still the layer's `400`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn body_guards_are_never_silently_dropped(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let who = a_resource(&app, "# Doc\n\nv0\n").await;
    let meta_path = format!("/api/resources/{}/meta", who.resource);
    let meta = |expect: Value| {
        json!({
            "resource_id": who.resource,
            "managed_meta": {},
            "open_meta": {"k": "v"},
            "managed_hash": "",
            "open_hash": "",
            "expect": expect,
        })
    };
    let put = send(
        &app,
        Some(&who.token),
        reqwest::Method::PUT,
        &meta_path,
        &[],
        Some(meta(json!({"body": "bt1.x"}))),
    )
    .await;
    assert_eq!(put.status, 422, "{put:?}");
    let put = send(
        &app,
        Some(&who.token),
        reqwest::Method::PUT,
        &meta_path,
        &[("if-match", "\"bt1.x\"")],
        Some(meta(json!({"open": {"k": null}}))),
    )
    .await;
    assert_eq!((put.status, code(&put)), (400, "PRECONDITION_NOT_HONOURED"));
    for header in [
        ("if-none-match", "*"),
        ("if-unmodified-since", "Wed, 21 Oct 2015 07:28:00 GMT"),
    ] {
        let answer = patch(&app, &who, &[header], json!({"content": "# Doc\n\nx\n"})).await;
        assert_eq!(
            (answer.status, code(&answer)),
            (400, "PRECONDITION_NOT_HONOURED"),
            "{header:?}"
        );
    }
    // A misspelled member inside `expect` is refused, never dropped.
    let answer = patch(
        &app,
        &who,
        &[],
        json!({"content": "# Doc\n\nx\n", "expect": {"bodyy": "bt1.x"}}),
    )
    .await;
    assert_eq!(answer.status, 422, "{answer:?}");
    assert_eq!(
        content(&app, &who, &[]).await.body["markdown"],
        json!("# Doc\n\nv0\n")
    );
}

/// **A caller the route refuses gets exactly its answer without the header.** No token at all, or
/// a stranger who may neither read nor modify the resource: `If-Match`, matching or not, changes
/// nothing about the answer, and so never a `412`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_refused_caller_learns_nothing_from_if_match(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let who = a_resource(&app, "# Doc\n\nv0\n").await;
    let t0 = content(&app, &who, &[]).await.body["body_token"]
        .as_str()
        .unwrap()
        .to_string();
    let (stranger, _) = person(&app).await;
    let path = format!("/api/resources/{}", who.resource);
    let body = json!({"content": "# Doc\n\nx\n"});
    for caller in [None, Some(stranger.as_str())] {
        let plain = send(
            &app,
            caller,
            reqwest::Method::PATCH,
            &path,
            &[],
            Some(body.clone()),
        )
        .await;
        assert!(matches!(plain.status, 401 | 403 | 404), "{plain:?}");
        for tag in [
            format!("\"{t0}\""),
            "\"bt1.wrong\"".to_string(),
            format!("W/\"{t0}\""),
        ] {
            let guarded = send(
                &app,
                caller,
                reqwest::Method::PATCH,
                &path,
                &[("if-match", &tag)],
                Some(body.clone()),
            )
            .await;
            assert_eq!(
                (guarded.status, &guarded.body, &guarded.etag),
                (plain.status, &plain.body, &plain.etag),
                "{tag} as {caller:?}"
            );
        }
    }
}

// ── golden bytes ─────────────────────────────────────────────────────────────────────────────────

const GOLDEN_SOURCE: &str =
    "# Alpha\n\nFirst paragraph, with  two spaces.\r\n\n## Beta\n\nSecond.\n\n# Gamma\n\nThird.\n";

/// The live blocks of `resource`, in `seq` order.
async fn blocks(pool: &PgPool, resource: &str) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT id FROM kb_content_blocks WHERE resource_id = $1::uuid AND NOT is_folded \
          ORDER BY seq",
    )
    .bind(resource)
    .fetch_all(pool)
    .await
    .expect("blocks")
}

/// Drop the stored bytes of these blocks' current revisions: the shape a legacy (pre-verbatim)
/// block has, which `/content` can only reconstruct.
async fn drop_bytes(pool: &PgPool, blocks: &[Uuid]) {
    sqlx::query(
        "DELETE FROM kb_block_content bc USING kb_content_blocks b \
          WHERE bc.block_revision_id = b.current_revision_id AND b.id = ANY($1)",
    )
    .bind(blocks)
    .execute(pool)
    .await
    .expect("drop bytes");
}

/// **Golden bytes.** `/content` for a verbatim body, a derived one (no block has bytes) and a
/// mixed one (one block lacks them), pinned byte for byte with their tokens. A change to either
/// renderer moves the token of every body it renders differently, under the same `bt1.` prefix,
/// and every held token then fails its guard; this test makes such a change visible, so it is made
/// on purpose and named in the release notes.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn content_bytes_are_pinned_for_each_storage_shape(pool: PgPool) {
    let app = common::setup_test_app(pool.clone()).await;

    let verbatim = a_resource(&app, GOLDEN_SOURCE).await;
    let derived = a_resource(&app, GOLDEN_SOURCE).await;
    let mixed = a_resource(&app, GOLDEN_SOURCE).await;
    let all = blocks(&pool, &derived.resource).await;
    assert!(all.len() > 1, "the fixture is multi-block: {all:?}");
    drop_bytes(&pool, &all).await;
    let first = blocks(&pool, &mixed.resource).await;
    drop_bytes(&pool, &first[..1]).await;

    let cases = [
        (&verbatim, GOLDEN_SOURCE, GOLDEN_VERBATIM_TOKEN),
        (&derived, GOLDEN_DERIVED, GOLDEN_DERIVED_TOKEN),
        (&mixed, GOLDEN_DERIVED, GOLDEN_DERIVED_TOKEN),
    ];
    for (who, bytes, token) in cases {
        let read = content(&app, who, &[]).await;
        assert_eq!(read.status, 200, "{read:?}");
        assert_eq!(read.body["markdown"], json!(bytes), "the pinned bytes");
        assert_eq!(read.body["body_token"], json!(token), "the pinned token");
        assert_eq!(temper_core::body_token(bytes), token);
    }
}

/// What the derived reconstruction renders for [`GOLDEN_SOURCE`]: lossy (CRLF folded, sections
/// re-joined, headings re-synthesized from the chunk breadcrumbs). A mixed-coverage body renders
/// the same way, because one byteless block sends the whole body down the derived path.
const GOLDEN_DERIVED: &str =
    "# Alpha\n\nFirst paragraph, with  two spaces.\n\n## Beta\n\nSecond.\n\n# Gamma\n\nThird.";
const GOLDEN_VERBATIM_TOKEN: &str = "bt1.lwthX4SqKLIZawZTtvSwhbqh3mVXD6RB4-OAcGvZc1g";
const GOLDEN_DERIVED_TOKEN: &str = "bt1.ft-T0TyScl3Rrov8gq9dlAHxoD4zaJl7Vlx2BYgJ4l8";
