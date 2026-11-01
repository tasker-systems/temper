#![cfg(feature = "test-db")]
//! The shape declare routes take a body no larger than a schema needs.
//!
//! `POST /api/contexts/{id}/shapes` and `POST /api/cognitive-maps/{id}/shapes` mount in their own
//! route group with `SHAPE_DECLARE_MAX_BODY_BYTES`, inner to the gated tier's 25 MiB. The list
//! GETs on the same two paths stay in the gated group, so these witnesses also prove the two
//! routers merge by method.
//!
//! `cargo nextest run -p temper-api --features test-db --test shape_declare_body_limit_test`

mod common;

use axum::http::StatusCode;
use serde_json::{json, Map, Value};
use sqlx::PgPool;
use uuid::Uuid;

use temper_api::routes::SHAPE_DECLARE_MAX_BODY_BYTES;
use temper_core::types::artifact_ask::walk::MAX_SCHEMA_BYTES;
use temper_substrate::shape_schema::{count_up_to, MAX_SHAPE_NODES, MAX_SHAPE_OBJECTS};

/// The L0 cognitive map: seeded by the migrations, readable by any approved profile.
const L0_COGMAP: Uuid = Uuid::from_u128(0x00000000_0000_0000_0005_000000000001);

/// A profile that clears the gated front door, with its own `temper` context, and its bearer.
/// Returns `(profile, context, token)`.
async fn caller(app: &common::TestApp) -> (Uuid, Uuid, String) {
    let email = format!("shape-declare-{}@test.com", Uuid::now_v7());
    let (profile, context) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);
    (profile, context, token)
}

/// A declare body of exactly `len` bytes, the padding in the schema's `description`.
fn declare_body_of_len(len: usize) -> Vec<u8> {
    let body_with = |padding: &str| {
        serde_json::to_vec(&json!({
            "kind": "bodylimit",
            "schema": { "type": "object", "description": padding },
            "enforcement": "advisory",
        }))
        .expect("serializes")
    };
    let unpadded = body_with("").len();
    let body = body_with(&"x".repeat(len - unpadded));
    assert_eq!(body.len(), len, "the padding is ASCII, one byte a char");
    body
}

/// A field tree of 800 described fields, each with seven small `examples`, built so that:
///
/// - compact it is about 248 KiB, within `MAX_SCHEMA_BYTES` (256 KiB);
/// - sent pretty-printed, inside a pretty-printed declare envelope, it is about 425 KiB, past
///   `MAX_SCHEMA_BYTES` and within `SHAPE_DECLARE_MAX_BODY_BYTES`;
/// - it holds 3 + 800 x 14 = 11,203 JSON values (cap 12,288) and 2 + 800 x 3 = 2,402 objects
///   (cap 3,072). Per field: the field object, its `properties` object, the `v` object, the
///   `description`, the `examples` array, seven integers, and two `type` strings. The root adds
///   itself, its `properties` object and its `type`.
///
/// Long `description`s reach the byte size with few values; the `examples` array, which
/// pretty-printing puts one element a line, is what widens the pretty form.
fn pretty_heavy_schema() -> Value {
    let mut fields = Map::new();
    for i in 0..800 {
        fields.insert(
            format!("f{i:04}"),
            json!({
                "type": "object",
                "properties": {
                    "v": {
                        "type": "string",
                        "description": "d".repeat(210),
                        "examples": [0, 0, 0, 0, 0, 0, 0],
                    }
                }
            }),
        );
    }
    json!({ "type": "object", "properties": Value::Object(fields) })
}

/// A body one byte past the limit is refused by the limit, on both declare paths.
///
/// Bites: the limit itself. Before the change both routes sat in `gated_routes` under the 25 MiB
/// tier limit, so this body was parsed and refused as a shape (a 4xx that is not 413: it is not a
/// field tree, or the caller lacks authority), never by the transport. The 413 is only reachable
/// when the group's own 512 KiB sits inner to the tier's.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_declare_body_past_the_limit_is_refused_by_the_limit(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (_profile, context, token) = caller(&app).await;
    let body = declare_body_of_len(SHAPE_DECLARE_MAX_BODY_BYTES + 1);

    for path in [
        format!("/api/contexts/{context}/shapes"),
        format!("/api/cognitive-maps/{L0_COGMAP}/shapes"),
    ] {
        let res = app
            .client
            .post(app.url(&path))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(body.clone())
            .send()
            .await
            .expect("post declare");

        assert_eq!(
            res.status(),
            StatusCode::PAYLOAD_TOO_LARGE,
            "{path}: over the limit must be refused BY THE LIMIT — asserting merely 'not 200' \
             would pass for any unrelated failure, including one that never reached the limit"
        );
    }
}

/// A pretty-printed schema whose compact form is within `MAX_SCHEMA_BYTES` is not refused at the
/// door: the limit leaves room for pretty-printing and the envelope.
///
/// Bites: a limit set at `MAX_SCHEMA_BYTES` itself, or a body measured compact, which would
/// refuse this body with a 413 although the schema it carries is within every cap. Before the
/// change the route accepted it too (25 MiB), so this passes unchanged against the old code; it
/// is here to stop the limit being tightened to the schema's compact size.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_pretty_printed_schema_within_the_byte_cap_is_accepted_at_the_door(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (profile, context, token) = caller(&app).await;

    let schema = pretty_heavy_schema();
    let compact = serde_json::to_vec(&schema).expect("serializes").len();
    assert!(
        (240 * 1024..=MAX_SCHEMA_BYTES).contains(&compact),
        "the fixture drifted: the compact schema is {compact} bytes, not about 250 KiB within \
         MAX_SCHEMA_BYTES ({MAX_SCHEMA_BYTES})"
    );
    let count = count_up_to(&schema, MAX_SHAPE_NODES, MAX_SHAPE_OBJECTS);
    assert!(
        !count.over,
        "the fixture drifted past the node budget: {} values, {} objects (caps {}, {})",
        count.nodes, count.objects, MAX_SHAPE_NODES, MAX_SHAPE_OBJECTS
    );

    let body = serde_json::to_vec_pretty(&json!({
        "kind": "prettyheavy",
        // A fresh context homes no resource to default the family's owner from, so name it.
        "kind_owner": { "kb_profiles": profile },
        "schema": schema,
        "enforcement": "advisory",
    }))
    .expect("serializes");
    assert!(
        body.len() > 400 * 1024 && body.len() < SHAPE_DECLARE_MAX_BODY_BYTES,
        "the fixture drifted: the pretty body is {} bytes, not about 450 KiB under the limit \
         ({SHAPE_DECLARE_MAX_BODY_BYTES})",
        body.len()
    );

    let res = app
        .client
        .post(app.url(&format!("/api/contexts/{context}/shapes")))
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .expect("post declare");

    // 200 is the declare route's success status (`declare_shape`'s `utoipa::path` responses).
    let status = res.status();
    let text = res.text().await.expect("body");
    assert_eq!(status, StatusCode::OK, "the declare was refused: {text}");
}

/// The list GETs on the two paths the declare POSTs moved off still answer: the two routers
/// merged by method, with neither shadowing the other.
///
/// Bites: a merge that drops or shadows the GET, which would answer 405 or 404. Passes on the
/// code before the change too, where the GETs and the POSTs were one router.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn listing_shapes_still_answers_on_the_shared_path(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (_profile, context, token) = caller(&app).await;

    for path in [
        format!("/api/contexts/{context}/shapes"),
        format!("/api/cognitive-maps/{L0_COGMAP}/shapes"),
    ] {
        let res = app
            .client
            .get(app.url(&path))
            .header("authorization", format!("Bearer {token}"))
            .send()
            .await
            .expect("get shapes");

        assert_eq!(res.status(), StatusCode::OK, "{path}: the list GET");
    }
}

/// A small valid declare body with no credentials is refused on both declare paths with 401
/// Unauthorized.
///
/// Both the tier's `require_auth` and the declare handler's own auth extractor refuse an
/// unauthenticated request, so this does not witness which tier the routes are mounted in; that is
/// `declaring_without_system_access_is_refused_on_both_paths`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn declaring_without_credentials_is_refused_on_both_paths(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (_profile, context, _token) = caller(&app).await;
    let body = declare_body_of_len(1024);

    for path in [
        format!("/api/contexts/{context}/shapes"),
        format!("/api/cognitive-maps/{L0_COGMAP}/shapes"),
    ] {
        let res = app
            .client
            .post(app.url(&path))
            .header("content-type", "application/json")
            .body(body.clone())
            .send()
            .await
            .expect("post declare");

        assert_eq!(
            res.status(),
            StatusCode::UNAUTHORIZED,
            "{path}: a declare without credentials must be refused as unauthenticated"
        );
    }
}

/// An authenticated caller whose standing is `denied` is refused on both declare paths with 403,
/// by the gated tier's `require_system_access` before any handler runs.
///
/// Bites: mounting `shape_declare_routes` at a tier without `require_system_access`
/// (`Tier::AuthOnly` or `Tier::Public`). Only the gated tier checks standing, and the declare
/// handlers do not, so a mount outside it would let this caller through to a declare.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn declaring_without_system_access_is_refused_on_both_paths(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (profile, context, token) = caller(&app).await;
    // The fixture approves its profile; return it to the standing a fresh principal is born in.
    sqlx::query(
        "INSERT INTO kb_principal_standing (profile_id, state) VALUES ($1, 'denied') \
         ON CONFLICT (profile_id) DO UPDATE SET state = 'denied', updated = now()",
    )
    .bind(profile)
    .execute(&app.pool)
    .await
    .expect("set standing to denied");
    let body = declare_body_of_len(1024);

    for path in [
        format!("/api/contexts/{context}/shapes"),
        format!("/api/cognitive-maps/{L0_COGMAP}/shapes"),
    ] {
        let res = app
            .client
            .post(app.url(&path))
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(body.clone())
            .send()
            .await
            .expect("post declare");

        assert_eq!(
            res.status(),
            StatusCode::FORBIDDEN,
            "{path}: a principal without system access must be refused by the gated tier"
        );
    }
}
