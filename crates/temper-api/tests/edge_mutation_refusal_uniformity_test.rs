#![cfg(feature = "test-db")]
//! An edge the caller cannot see is refused by every edge-mutation door exactly as an edge that
//! does not exist: `404`, the same code, the same message. `check_edge_mutable`'s clause 0.
//!
//! Before it, the gate answered by which endpoint was hidden: the source floor's `403` when the
//! caller could not read the SOURCE (`write_floor::modify_floor_in_tx`), the target clause's `404`
//! naming the TARGET's table and id when they could not read the target, and `404` naming the edge
//! when the edge was missing or folded. The context change feed hands any reader of a context the
//! ids of every edge homed there, including edges whose other endpoint that reader cannot read
//! (task 01a1a1a5-773a-7599-a157-6128705ef71e), so those three answers made the write doors an
//! oracle for which side of an edge is hidden and whether the edge is still live. The read
//! surfaces already answer hidden and missing alike.
//!
//! The caller is a WRITER in the edge's home context: the one principal for whom a hidden edge
//! and a live one differ only in an endpoint's readability. Every state is made by a real door,
//! except the writer's context grant and the second context, which are fixture rows.

mod common;

use reqwest::Method;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

struct Caller {
    token: String,
    profile: Uuid,
}

/// A provisioned profile with its own context, and its JWT.
async fn caller(pool: &PgPool, label: &str) -> (Caller, Uuid) {
    let email = format!("edge-oracle-{label}-{}@example.com", Uuid::new_v4());
    let (profile, own_context) =
        common::fixtures::create_test_profile_with_context(pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile}"), &email);
    (Caller { token, profile }, own_context)
}

/// A second context owned by `owner`, for the owner's private resources.
async fn private_context(pool: &PgPool, owner: Uuid) -> Uuid {
    let context = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES ($1, 'kb_profiles', $2, 'private', 'private')",
    )
    .bind(context)
    .bind(owner)
    .execute(pool)
    .await
    .expect("insert the owner's private context");
    context
}

/// A direct read+write grant on `context` for `profile` — a writer in that context, nothing more.
async fn grant_context_write(pool: &PgPool, context: Uuid, profile: Uuid, granted_by: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (subject_table, subject_id, principal_table, principal_id, \
            can_read, can_write, can_delete, can_grant, granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, true, false, false, $3)",
    )
    .bind(context)
    .bind(profile)
    .bind(granted_by)
    .execute(pool)
    .await
    .expect("seed a context write grant");
}

/// `method path` as `who`: the status and the parsed body (`Null` when not JSON).
async fn call(
    app: &common::TestApp,
    who: &Caller,
    method: Method,
    path: &str,
    body: Option<Value>,
) -> (u16, Value) {
    let mut req = app
        .client
        .request(method, app.url(path))
        .header("Authorization", format!("Bearer {}", who.token));
    if let Some(body) = body {
        req = req.json(&body);
    }
    let resp = req.send().await.expect("request");
    let status = resp.status().as_u16();
    let text = resp.text().await.expect("body");
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

/// A resource created by `owner` in `context` through `POST /api/resources`.
async fn create(app: &common::TestApp, owner: &Caller, context: Uuid, title: &str) -> Uuid {
    let (status, body) = call(
        app,
        owner,
        Method::POST,
        "/api/resources",
        Some(json!({
            "kb_context_id": context,
            "doc_type": "research",
            "origin_uri": format!("test://edge-oracle-{}", Uuid::new_v4()),
            "title": title,
        })),
    )
    .await;
    assert!(
        (200..300).contains(&status),
        "the owner creates; body: {body}"
    );
    Uuid::parse_str(body["id"].as_str().expect("id")).expect("resource id")
}

/// `owner` asserts `source → target` through `POST /api/relationships`; returns the edge handle.
async fn assert_edge(app: &common::TestApp, owner: &Caller, source: Uuid, target: Uuid) -> Uuid {
    let (status, body) = call(
        app,
        owner,
        Method::POST,
        "/api/relationships",
        Some(json!({
            "source": source,
            "target": target,
            "edge_kind": "leads_to",
            "polarity": "forward",
            "label": "oracle-edge",
            "weight": 1.0,
        })),
    )
    .await;
    assert_eq!(status, 200, "the owner asserts the edge; body: {body}");
    Uuid::parse_str(body["edge_handle"].as_str().expect("edge_handle")).expect("edge uuid")
}

/// `owner` moves `resource` to `context` through `PATCH /api/resources/{id}`.
async fn move_to(app: &common::TestApp, owner: &Caller, resource: Uuid, context: Uuid) {
    let (status, body) = call(
        app,
        owner,
        Method::PATCH,
        &format!("/api/resources/{resource}"),
        Some(json!({ "context_to": context.to_string() })),
    )
    .await;
    assert_eq!(status, 200, "the owner moves the resource; body: {body}");
}

async fn edge_home(pool: &PgPool, edge: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT home_anchor_id FROM kb_edges WHERE id = $1")
        .bind(edge)
        .fetch_one(pool)
        .await
        .expect("edge row")
}

async fn readable(pool: &PgPool, profile: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar("SELECT endpoint_readable_by_profile($1, 'kb_resources', $2)")
        .bind(profile)
        .bind(resource)
        .fetch_one(pool)
        .await
        .expect("endpoint predicate")
}

/// The five doors that mutate an existing edge, as `(name, method, path, body)`. The retract's
/// property id is a fixed fresh uuid, so it is the same for every edge.
fn edge_mutate_doors(
    edge: Uuid,
    property: Uuid,
) -> Vec<(&'static str, Method, String, Option<Value>)> {
    vec![
        (
            "retype",
            Method::POST,
            format!("/api/relationships/{edge}/retype"),
            Some(json!({ "edge_kind": "near", "polarity": "forward" })),
        ),
        (
            "reweight",
            Method::POST,
            format!("/api/relationships/{edge}/reweight"),
            Some(json!({ "weight": 0.5 })),
        ),
        (
            "fold",
            Method::POST,
            format!("/api/relationships/{edge}/fold"),
            Some(json!({ "reason": "oracle probe" })),
        ),
        (
            "facet set",
            Method::POST,
            format!("/api/relationships/{edge}/facets"),
            Some(json!({ "values": { "summary": "probe" } })),
        ),
        (
            "facet retract",
            Method::DELETE,
            format!("/api/relationships/{edge}/facets/{property}"),
            None,
        ),
    ]
}

/// The answer a door gives, with the addressed edge id masked: the id is the caller's own input,
/// and everything else must not vary with the edge.
fn masked(status: u16, body: &Value, edge: Uuid) -> (u16, String) {
    (
        status,
        body.to_string().replace(&edge.to_string(), "<edge>"),
    )
}

/// A writer in an edge's home context gets the same refusal from every edge-mutation door for an
/// edge whose SOURCE they cannot read, one whose TARGET they cannot read, and an edge id that
/// does not exist — status, code and message alike.
///
/// FAILS IF clause 0 is removed from `check_edge_mutable`, or moved after the source floor: the
/// private-source edge then answers `403` from the floor, and the private-target edge a `404`
/// naming `kb_resources` and the target's id. Either alone splits the three answers.
///
/// The positive control (an edge between two endpoints the writer reads, in the same home) is
/// what proves the writer has the standing that makes the three refusals worth comparing: the
/// doors admit them on a visible edge.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_hidden_edge_is_refused_exactly_as_a_missing_one(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, shared) = caller(&app.pool, "owner").await;
    let private = private_context(&app.pool, owner.profile).await;
    let (writer, _) = caller(&app.pool, "writer").await;
    grant_context_write(&app.pool, shared, writer.profile, owner.profile).await;

    let open_a = create(&app, &owner, shared, "open a").await;
    let open_b = create(&app, &owner, shared, "open b").await;
    let private_target = create(&app, &owner, private, "private target").await;
    // Created in the shared context so its edge is homed there, then moved out: an edge keeps the
    // home it was asserted in.
    let private_source = create(&app, &owner, shared, "private source").await;

    let hidden_target_edge = assert_edge(&app, &owner, open_a, private_target).await;
    let hidden_source_edge = assert_edge(&app, &owner, private_source, open_b).await;
    move_to(&app, &owner, private_source, private).await;
    let missing_edge = Uuid::now_v7();

    // Non-vacuity: both hidden edges are homed where the writer writes, and each has exactly one
    // endpoint the writer cannot read.
    for edge in [hidden_target_edge, hidden_source_edge] {
        assert_eq!(
            edge_home(&app.pool, edge).await,
            shared,
            "homed in the shared context"
        );
    }
    assert!(readable(&app.pool, writer.profile, open_a).await);
    assert!(readable(&app.pool, writer.profile, open_b).await);
    assert!(!readable(&app.pool, writer.profile, private_target).await);
    assert!(!readable(&app.pool, writer.profile, private_source).await);

    let property = Uuid::now_v7();
    for (index, (name, ..)) in edge_mutate_doors(missing_edge, property).iter().enumerate() {
        let mut answers = Vec::new();
        for edge in [missing_edge, hidden_source_edge, hidden_target_edge] {
            let (_, method, path, body) = edge_mutate_doors(edge, property).swap_remove(index);
            let (status, response) = call(&app, &writer, method, &path, body).await;
            answers.push(masked(status, &response, edge));
        }
        assert_eq!(
            answers[0].0, 404,
            "{name}: a missing edge answers 404: {:?}",
            answers[0]
        );
        assert_eq!(
            answers[1], answers[0],
            "{name}: an edge with a hidden SOURCE answers as a missing one"
        );
        assert_eq!(
            answers[2], answers[0],
            "{name}: an edge with a hidden TARGET answers as a missing one"
        );
    }

    // Nothing was written through the refused doors.
    for edge in [hidden_source_edge, hidden_target_edge] {
        let (kind, weight, folded): (String, f64, bool) = sqlx::query_as(
            "SELECT edge_kind::text, weight::float8, is_folded FROM kb_edges WHERE id = $1",
        )
        .bind(edge)
        .fetch_one(&app.pool)
        .await
        .expect("edge row");
        assert_eq!((kind.as_str(), weight, folded), ("leads_to", 1.0, false));
    }

    // Positive control: the same writer, an edge in the same home whose endpoints they read.
    let visible_edge = assert_edge(&app, &owner, open_a, open_b).await;
    let (status, body) = call(
        &app,
        &writer,
        Method::POST,
        &format!("/api/relationships/{visible_edge}/reweight"),
        Some(json!({ "weight": 0.5 })),
    )
    .await;
    assert_eq!(
        status, 200,
        "the writer reweights a visible edge; body: {body}"
    );
}
