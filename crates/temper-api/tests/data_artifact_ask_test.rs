#![cfg(feature = "test-db")]
//! **`POST /api/data-artifacts/ask`, the batch read of one artifact ask, at its HTTP door** (spec
//! `2026-10-19-asking-artifacts-by-what-they-are` §7.1, §7.2, §9).
//!
//! What only this door can show: a body it cannot read is answered as a typed refusal and never in
//! the framework's plain text; a `GET` to the path meets the POST-only route, not the flat read
//! beside it; a content-less artifact is served without a `content` field; and asking adds nothing
//! the search corpus can find. The doors' parity (unreadable equals absent, refusals with their
//! `teaches`) is witnessed across all three doors in `tests/e2e/tests/data_artifact_ask_doors_e2e.rs`.

mod common;

use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::PgPool;
use temper_substrate::events::EventContext;
use temper_substrate::ids::{ContextId, EntityId, ProfileId, ResourceId};
use temper_substrate::payloads::{AnchorRef, ArtifactIntent, EnforcementMode, KindOwner};
use temper_substrate::writes::{self, CommitDataArtifactParams, CreateParams, DeclareShapeParams};
use uuid::Uuid;

const ASK: &str = "/api/data-artifacts/ask";

/// A string that appears only in artifact content, never in a resource.
const SENTINEL: &str = "zqxartifactsentinelqzx";

/// The owning resource's title and body. Neither carries [`SENTINEL`].
const TITLE: &str = "Kestrel latency";
const BODY: &str = "kestrel latency measurements";

/// The owner's world: a context holding resource R, a `measurement` shape declared there, one
/// artifact on R whose content carries [`SENTINEL`], and one committed with no content.
struct World {
    app: common::TestApp,
    token: String,
    shape: Uuid,
    with_sentinel: Uuid,
    contentless: Uuid,
}

async fn emitter_of(pool: &PgPool, profile: Uuid) -> EntityId {
    let id: Uuid = sqlx::query_scalar(
        "SELECT id FROM kb_entities WHERE profile_id = $1 AND name LIKE '%@web'",
    )
    .bind(profile)
    .fetch_one(pool)
    .await
    .expect("the fixture mints a web emitter");
    EntityId::from(id)
}

async fn world(pool: PgPool) -> World {
    let app = common::setup_test_app(pool).await;
    let email = format!("batch-ask-{}@example.com", Uuid::new_v4());
    let (owner, context) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{owner}"), &email);

    let emitter = emitter_of(&app.pool, owner).await;
    let home = AnchorRef::context(ContextId::from(context));
    let profile = ProfileId::from(owner);
    let resource = writes::create_resource_deferred_with(
        &app.pool,
        CreateParams {
            idempotency_key: None,
            title: TITLE,
            origin_uri: "test://data-artifact-ask/kestrel",
            body: BODY,
            doc_type: "research",
            home,
            owner: profile,
            originator: profile,
            emitter,
            properties: &[],
            chunks: None,
            sources: vec![],
        },
        EventContext::default(),
    )
    .await
    .expect("create")
    .uuid();
    let shape = writes::declare_shape(
        &app.pool,
        DeclareShapeParams {
            home,
            kind: "measurement",
            kind_owner: Some(KindOwner::Profile(owner)),
            schema: &json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": { "unit": { "type": "string" }, "note": { "type": "string" } }
            }),
            enforcement: EnforcementMode::Advisory,
            emitter,
        },
    )
    .await
    .expect("declare")
    .uuid();
    let commit = |content: Value| {
        let pool = app.pool.clone();
        async move {
            writes::commit_data_artifact(
                &pool,
                CommitDataArtifactParams {
                    resource: ResourceId::from(resource),
                    kind: "measurement",
                    kind_owner: Some(KindOwner::Profile(owner)),
                    intent: ArtifactIntent::Member,
                    precedence: 0.0,
                    content: &content,
                    supersedes: &[],
                    emitter,
                },
            )
            .await
            .expect("commit")
            .uuid()
        }
    };
    let with_sentinel = commit(json!({ "unit": "ms", "note": SENTINEL })).await;
    let contentless = commit(Value::Null).await;

    World {
        app,
        token,
        shape,
        with_sentinel,
        contentless,
    }
}

/// POST `body` as raw bytes with a JSON content type, so a malformed one reaches the door as sent.
async fn post_raw(w: &World, body: String) -> (StatusCode, String) {
    let resp = w
        .app
        .client
        .post(w.app.url(ASK))
        .header("Authorization", format!("Bearer {}", w.token))
        .header("Content-Type", "application/json")
        .body(body)
        .send()
        .await
        .expect("POST the ask");
    let status = resp.status();
    (status, resp.text().await.expect("a body"))
}

async fn ask(w: &World, body: &Value) -> (StatusCode, Value) {
    let (status, text) = post_raw(w, body.to_string()).await;
    let parsed = serde_json::from_str(&text).unwrap_or_else(|e| panic!("not JSON ({e}): {text}"));
    (status, parsed)
}

/// **A body this door cannot read is a typed refusal, never axum's text:** malformed JSON, a value
/// of the wrong type, a field the request does not declare, and an `after` missing half its
/// position each answer a JSON error body under `UNREADABLE_PLAN`, with no `details` and a bounded
/// message, at the reader's own status. A readable request, the control, is answered. Bite: the
/// handler reading `Json<AskRequest>` in place of `AskBody` — the rejections come back as plain
/// text, which does not parse.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_unreadable_body_is_a_typed_refusal(pool: PgPool) {
    let w = world(pool).await;
    let ask_of = json!({ "shape": w.shape });

    let (status, _) = ask(&w, &json!({ "ask": ask_of })).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the control: a readable ask is answered"
    );

    let unknown = json!({ "ask": ask_of, "order": "desc" });
    let wrong_type = json!({ "ask": ask_of, "limit": "two" });
    let half_after = json!({ "ask": ask_of, "after": { "created": "2026-10-24T09:15:30Z" } });
    for (what, body, expected) in [
        (
            "malformed JSON",
            "{not json".to_string(),
            StatusCode::BAD_REQUEST,
        ),
        (
            "a wrong type",
            wrong_type.to_string(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "an unknown field",
            unknown.to_string(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
        (
            "half a position",
            half_after.to_string(),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        let (status, text) = post_raw(&w, body).await;
        let parsed: Value = serde_json::from_str(&text)
            .unwrap_or_else(|e| panic!("{what}: a typed body, not plain text ({e}): {text}"));
        assert_eq!(status, expected, "{what}: {parsed}");
        assert_eq!(
            parsed["error"]["code"],
            temper_core::error::UNREADABLE_PLAN_CODE,
            "{what}: {parsed}"
        );
        assert!(
            parsed["error"].get("details").is_none_or(Value::is_null),
            "{what}: an unreadable body has no refusal list: {parsed}"
        );
        let message = parsed["error"]["message"].as_str().expect("a message");
        assert!(
            message.len() <= 1024 + 64,
            "{what}: {} bytes",
            message.len()
        );
    }
}

/// **An `after` naming an instant the store cannot hold is a body this door cannot read, never a
/// server fault.** Postgres's `timestamptz` starts at 4714-11-24 00:00:00 BC (UTC), and a caller's
/// position is read as a `DateTime<Utc>`, which reaches back to year -262,143: the instant one
/// microsecond before the start is refused `422` under `UNREADABLE_PLAN` with a bounded message,
/// and the start itself, the control, is answered. At the high end the latest instant a
/// `DateTime<Utc>` holds is answered — it is inside `timestamptz`'s range — and one past it is not
/// a position at all, refused the same way by the reader. Bite: the `is_storable` check removed
/// from `AskBody::from_request` — the low case reaches the database and answers `500`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_after_the_store_cannot_hold_is_unreadable_and_never_a_fault(pool: PgPool) {
    let w = world(pool).await;
    let after = |created: &str| {
        json!({
            "ask": { "shape": w.shape },
            "after": { "created": created, "artifact_id": Uuid::nil() },
        })
    };

    for (what, created) in [
        ("the first instant the store holds", "-4713-11-24T00:00:00Z"),
        (
            "the latest instant a position can be read at",
            "+262142-12-31T23:59:59.999999Z",
        ),
    ] {
        let (status, body) = ask(&w, &after(created)).await;
        assert_eq!(status, StatusCode::OK, "{what}, the control: {body}");
        assert_eq!(body["next"], Value::Null, "{what}: {body}");
    }

    for (what, created) in [
        (
            "one microsecond before the store's first instant",
            "-4713-11-23T23:59:59.999999Z",
        ),
        (
            "a year past what a position can be read at",
            "+262143-01-01T00:00:00Z",
        ),
    ] {
        let (status, body) = ask(&w, &after(created)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{what}: {body}");
        assert_eq!(
            body["error"]["code"],
            temper_core::error::UNREADABLE_PLAN_CODE,
            "{what}: {body}"
        );
        assert!(
            body["error"].get("details").is_none_or(Value::is_null),
            "{what}: {body}"
        );
        let message = body["error"]["message"].as_str().expect("a message");
        assert!(
            message.len() <= 1024 + 64,
            "{what}: {} bytes",
            message.len()
        );
    }
}

/// **A `GET` to the batch read's path is answered `405` by its POST-only route, to a caller who
/// passes the route's checks** — the static segment is matched ahead of the flat read's
/// `{artifact_id}`, so it is not read as a malformed artifact id. Pinned whatever axum does,
/// because the spec's §7.1 parenthesis says otherwise. **The same `GET` without credentials is
/// answered `401`**: the gated tier's checks wrap the route, its method fallback included, so they
/// answer first. The control: the flat read beside it answers a `GET` for an artifact by id.
/// Bites: a `GET` route registered on the path (no `405`), or the gated tier's layers applied
/// with `route_layer` (the unauthenticated `GET` meets the fallback first: `405`, not `401`).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_get_to_the_ask_path_meets_the_post_only_route(pool: PgPool) {
    let w = world(pool).await;
    let get = |path: String| {
        w.app
            .client
            .get(w.app.url(&path))
            .header("Authorization", format!("Bearer {}", w.token))
            .send()
    };

    let resp = get(ASK.to_string()).await.expect("GET the ask path");
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);

    let anonymous = w
        .app
        .client
        .get(w.app.url(ASK))
        .send()
        .await
        .expect("GET the ask path without credentials");
    assert_eq!(
        anonymous.status(),
        StatusCode::UNAUTHORIZED,
        "without credentials the caller check answers before the route"
    );

    let flat = get(format!("/api/data-artifacts/{}", w.with_sentinel))
        .await
        .expect("GET the flat read");
    assert_eq!(flat.status(), StatusCode::OK, "the control");
}

/// **An artifact committed with no content is served without a `content` field** — absent, not
/// `null`: `ArtifactView`'s own rule, which `list` and `show` share — and one with content carries
/// it whole. An ask with no predicates returns both. Bite: the batch read
/// serializing `content` as `null` (`skip_serializing_if` dropped from `ArtifactView.content`).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_contentless_artifact_is_served_without_content(pool: PgPool) {
    let w = world(pool).await;
    let (status, body) = ask(&w, &json!({ "ask": { "shape": w.shape } })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let rows = body["artifacts"].as_array().expect("artifacts");
    let row = |id: Uuid| {
        rows.iter()
            .find(|r| r["artifact_id"] == id.to_string())
            .unwrap_or_else(|| panic!("{id} is served: {body}"))
    };
    assert!(
        row(w.contentless).get("content").is_none(),
        "absent, not null: {}",
        row(w.contentless)
    );
    assert_eq!(
        row(w.with_sentinel)["content"],
        json!({ "unit": "ms", "note": SENTINEL })
    );
}

/// One comparable digest of what search reads: `kb_resource_search_index`'s rows and an md5 of
/// their full-text vectors (`tsvector`), `kb_chunks`' rows, how many carry an embedding and an md5
/// of the embedding values themselves, and `kb_chunk_content`'s rows. It does not cover the
/// context and cognitive-map tables (`kb_contexts`, `kb_cogmaps.telos_centroid`), which no
/// artifact read reaches either.
async fn corpus_digest(pool: &PgPool) -> (i64, String, i64, i64, String, i64) {
    sqlx::query_as(
        "SELECT (SELECT count(*) FROM kb_resource_search_index),
                (SELECT md5(COALESCE(string_agg(resource_id::text || ':' || search_vector::text,
                                                ',' ORDER BY resource_id), ''))
                   FROM kb_resource_search_index),
                (SELECT count(*) FROM kb_chunks),
                (SELECT count(*) FROM kb_chunks WHERE embedding IS NOT NULL),
                (SELECT md5(COALESCE(string_agg(id::text || ':' || embedding::text,
                                                ',' ORDER BY id), ''))
                   FROM kb_chunks WHERE embedding IS NOT NULL),
                (SELECT count(*) FROM kb_chunk_content)",
    )
    .fetch_one(pool)
    .await
    .expect("the corpus digest reads")
}

/// **Asking adds no findability** (`asking-is-never-resemblance`, the corpus face). A batch ask
/// that serves the artifact carrying [`SENTINEL`] leaves the search corpus and the embedding tables
/// exactly as they were, and a search for the sentinel finds nothing: `POST /api/search` with a
/// text query, whose exact arm reads `kb_resource_search_index` through `search_exact` with no
/// embedding model — the cheapest entry point onto the real corpus — and the index itself, asked
/// directly. Bite: [`TITLE`] carrying the sentinel (`"Kestrel latency"` →
/// `"Kestrel latency zqxartifactsentinelqzx"`), which puts it in a resource and so in the corpus,
/// and both search assertions go red.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn asking_adds_nothing_search_can_find(pool: PgPool) {
    let w = world(pool).await;
    let before = corpus_digest(&w.app.pool).await;

    let (status, body) = ask(
        &w,
        &json!({ "ask": { "shape": w.shape, "predicates": [
            { "path": ["note"], "op": { "op": "contains", "values": [SENTINEL] } }
        ] } }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["artifacts"][0]["artifact_id"],
        w.with_sentinel.to_string(),
        "the precondition: the ask serves the sentinel's artifact: {body}"
    );

    assert_eq!(
        corpus_digest(&w.app.pool).await,
        before,
        "the corpus and the embeddings are unchanged"
    );

    let resp = w
        .app
        .client
        .post(w.app.url("/api/search"))
        .header("Authorization", format!("Bearer {}", w.token))
        .json(&json!({ "query": SENTINEL, "limit": 50 }))
        .send()
        .await
        .expect("search");
    assert_eq!(resp.status(), StatusCode::OK);
    let found: Value = resp.json().await.expect("search JSON");
    assert_eq!(
        found["exact"]["hits"].as_array().map(Vec::len),
        Some(0),
        "search finds nothing for a string only artifact content carries: {found}"
    );
    let indexed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM kb_resource_search_index
          WHERE search_vector @@ plainto_tsquery('english', $1)",
    )
    .bind(SENTINEL)
    .fetch_one(&w.app.pool)
    .await
    .expect("the index reads");
    assert_eq!(indexed, 0, "the corpus holds no row the sentinel matches");
}
