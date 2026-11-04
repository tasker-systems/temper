#![cfg(feature = "test-db")]
//! An edge the caller does not see is refused by every edge-mutation door exactly as an edge that
//! does not exist or is folded: `404`, the same code, the same message. `check_edge_mutable`'s
//! clause 0. "Sees" is `edges_visible_to`'s definition: the home anchor and both endpoints
//! readable.
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
//! The caller is a WRITER in the shared context: the principal for whom a hidden edge and a live
//! one differ only in one part's readability. A second test pins the `403` that remains, to a
//! caller who sees the edge but may not modify its source. Every state is made by a real door,
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

/// `owner` grants `grantee` read on `resource` through the real grant door.
async fn grant_read(app: &common::TestApp, owner: &Caller, resource: Uuid, grantee: Uuid) {
    let (status, body) = call(
        app,
        owner,
        Method::POST,
        &format!("/api/resources/{resource}/grants"),
        Some(json!({
            "principal_table": "kb_profiles",
            "principal_id": grantee,
            "can_read": true,
            "can_write": false,
            "can_delete": false,
            "can_grant": false,
        })),
    )
    .await;
    assert_eq!(status, 200, "the owner grants read; body: {body}");
}

/// `owner` folds `edge` through `POST /api/relationships/{h}/fold`.
async fn fold(app: &common::TestApp, owner: &Caller, edge: Uuid) {
    let (status, body) = call(
        app,
        owner,
        Method::POST,
        &format!("/api/relationships/{edge}/fold"),
        Some(json!({ "reason": "folded before the probe" })),
    )
    .await;
    assert_eq!(status, 200, "the owner folds the edge; body: {body}");
}

/// Does `profile` see `edge` as the read surfaces define it?
async fn edge_visible(pool: &PgPool, profile: Uuid, edge: Uuid) -> bool {
    sqlx::query_scalar("SELECT $2 IN (SELECT edge_id FROM edges_visible_to($1))")
        .bind(profile)
        .bind(edge)
        .fetch_one(pool)
        .await
        .expect("edges_visible_to")
}

/// `can_modify_resource(profile, resource)` — the source floor's own admission, asked directly.
async fn can_modify(pool: &PgPool, profile: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
        .bind(profile)
        .bind(resource)
        .fetch_one(pool)
        .await
        .expect("can_modify_resource")
}

async fn context_authorable(pool: &PgPool, profile: Uuid, context: Uuid) -> bool {
    sqlx::query_scalar("SELECT context_authorable_by_profile($1, $2)")
        .bind(profile)
        .bind(context)
        .fetch_one(pool)
        .await
        .expect("context_authorable_by_profile")
}

/// Open a transaction holding `FOR UPDATE` on each resource row — the erasure act's lock. A
/// request that takes `FOR KEY SHARE` on any of them (the source floor, clause 3's target lock)
/// queues behind it.
async fn hold_rows(
    pool: &PgPool,
    resources: &[Uuid],
) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut held = pool.begin().await.expect("begin the holder");
    for resource in resources {
        sqlx::query("SELECT 1 FROM kb_resources WHERE id = $1 FOR UPDATE")
            .bind(resource)
            .execute(&mut *held)
            .await
            .expect("hold the row");
    }
    held
}

/// Await `request`, failing if it has not answered within three seconds — under the write lock
/// bound (`WRITE_LOCK_TIMEOUT_MS`, five), so a request queued on a held row cannot answer first.
async fn answers_without_waiting<T>(
    request: impl std::future::Future<Output = T>,
    what: &str,
) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(3), request)
        .await
        .unwrap_or_else(|_| panic!("{what}: a refused caller waited on a held row lock"))
}

async fn context_readable(pool: &PgPool, profile: Uuid, context: Uuid) -> bool {
    sqlx::query_scalar("SELECT anchor_readable_by_profile($1, 'kb_contexts', $2)")
        .bind(profile)
        .bind(context)
        .fetch_one(pool)
        .await
        .expect("anchor predicate")
}

/// A writer in an edge's home context gets the same refusal from every edge-mutation door for
/// every edge they do not see — one whose SOURCE they cannot read, one whose TARGET they cannot
/// read, one whose HOME they cannot read though they read both endpoints — and for a FOLDED edge
/// and an edge id that does not exist: status, code and message alike.
///
/// FAILS IF `check_edge_mutable`'s row lookup stops filtering on visibility (`.filter(|row|
/// row.visible)` removed): the hidden-source and hidden-home edges then answer the source floor's
/// `403`. Dropping only the `anchor_readable_by_profile` conjunct fails through the hidden-home
/// edge alone. The hidden-target edge does NOT bite on either: clause 3's refusal is re-rendered
/// as the edge's own `NotFound`, so that arm's BODY is uniform without clause 0 — but its cost is
/// not: dropping only the target conjunct lets the hidden-target edge reach the source floor,
/// which locks the (admitted) source row, and the lock probe fails on it (the request queues
/// behind the held row). FAILS IF the lookup stops carrying `NOT is_folded`, through the folded
/// edge.
///
/// The positive control (an edge between two endpoints the writer reads, in the same home) is
/// what proves the writer has the standing that makes the refusals worth comparing: the doors
/// admit them on a visible edge.
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
    // Both endpoints of the home-hidden edge live in the private context, and the writer is
    // granted read on each resource directly — never on their home.
    let granted_x = create(&app, &owner, private, "granted x").await;
    let granted_y = create(&app, &owner, private, "granted y").await;
    grant_read(&app, &owner, granted_x, writer.profile).await;
    grant_read(&app, &owner, granted_y, writer.profile).await;

    let hidden_target_edge = assert_edge(&app, &owner, open_a, private_target).await;
    let hidden_source_edge = assert_edge(&app, &owner, private_source, open_b).await;
    move_to(&app, &owner, private_source, private).await;
    let hidden_home_edge = assert_edge(&app, &owner, granted_x, granted_y).await;
    let folded_edge = assert_edge(&app, &owner, open_b, open_a).await;
    fold(&app, &owner, folded_edge).await;
    let missing_edge = Uuid::now_v7();

    // Non-vacuity, per edge: exactly the one part named is what the writer cannot read.
    for edge in [hidden_target_edge, hidden_source_edge, folded_edge] {
        assert_eq!(
            edge_home(&app.pool, edge).await,
            shared,
            "homed in the shared context"
        );
    }
    assert_eq!(edge_home(&app.pool, hidden_home_edge).await, private);
    assert!(context_readable(&app.pool, writer.profile, shared).await);
    assert!(!context_readable(&app.pool, writer.profile, private).await);
    for readable_endpoint in [open_a, open_b, granted_x, granted_y] {
        assert!(readable(&app.pool, writer.profile, readable_endpoint).await);
    }
    assert!(!readable(&app.pool, writer.profile, private_target).await);
    assert!(!readable(&app.pool, writer.profile, private_source).await);
    for edge in [
        hidden_target_edge,
        hidden_source_edge,
        hidden_home_edge,
        folded_edge,
    ] {
        assert!(
            !edge_visible(&app.pool, writer.profile, edge).await,
            "the writer does not see {edge}"
        );
    }

    let compared = [
        ("hidden SOURCE", hidden_source_edge),
        ("hidden TARGET", hidden_target_edge),
        ("hidden HOME", hidden_home_edge),
        ("FOLDED", folded_edge),
    ];
    let property = Uuid::now_v7();
    for (index, (name, ..)) in edge_mutate_doors(missing_edge, property).iter().enumerate() {
        let probe = |edge: Uuid| {
            let (_, method, path, body) = edge_mutate_doors(edge, property).swap_remove(index);
            let app = &app;
            let writer = &writer;
            async move {
                let (status, response) = call(app, writer, method, &path, body).await;
                masked(status, &response, edge)
            }
        };
        let missing = probe(missing_edge).await;
        assert_eq!(
            missing.0, 404,
            "{name}: a missing edge answers 404: {missing:?}"
        );
        for (label, edge) in compared {
            assert_eq!(
                probe(edge).await,
                missing,
                "{name}: an edge with a {label} answers as a missing one"
            );
        }
    }

    // The refusal costs what a missing edge's does: the row lookup, and no lock. With every
    // endpoint row of the hidden edges held `FOR UPDATE`, each door still answers each hidden edge
    // the missing edge's refusal, promptly. A gate that let a hidden edge past its lookup would
    // reach the source floor (which locks an admitted source) or clause 3 (which locks the
    // target) and queue — the body can be uniform while the cost is not.
    let held = hold_rows(
        &app.pool,
        &[
            open_a,
            open_b,
            private_source,
            private_target,
            granted_x,
            granted_y,
        ],
    )
    .await;
    for (index, (name, ..)) in edge_mutate_doors(missing_edge, property).iter().enumerate() {
        for (label, edge) in [
            ("hidden SOURCE", hidden_source_edge),
            ("hidden TARGET", hidden_target_edge),
            ("hidden HOME", hidden_home_edge),
        ] {
            let (_, method, path, body) = edge_mutate_doors(edge, property).swap_remove(index);
            let (status, response) = answers_without_waiting(
                call(&app, &writer, method, &path, body),
                &format!("{name} on the {label} edge"),
            )
            .await;
            assert_eq!(
                (status, response["error"]["code"].as_str()),
                (404, Some("NOT_FOUND")),
                "{name}: the {label} edge, its rows held; body: {response}"
            );
        }
    }
    held.rollback().await.expect("release the rows");

    // Nothing was written through the refused doors.
    for (edge, folded) in [
        (hidden_source_edge, false),
        (hidden_target_edge, false),
        (hidden_home_edge, false),
        (folded_edge, true),
    ] {
        let (kind, weight, is_folded): (String, f64, bool) = sqlx::query_as(
            "SELECT edge_kind::text, weight::float8, is_folded FROM kb_edges WHERE id = $1",
        )
        .bind(edge)
        .fetch_one(&app.pool)
        .await
        .expect("edge row");
        assert_eq!(
            (kind.as_str(), weight, is_folded),
            ("leads_to", 1.0, folded)
        );
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

/// The `403` that remains, from the SOURCE FLOOR: a caller who sees the edge as `edges_visible_to`
/// defines it, and may author into its home, but may not modify its source gets the floor's `403`
/// from every edge-mutation door — never the `404` a hidden edge gets.
///
/// The caller is a writer in the shared context the edge is homed in, so container-write admits
/// them. The source was asserted there and then moved to the owner's private context (an edge
/// keeps its home), and the caller reads it through a resource read grant only. So the floor
/// (`can_modify_resource` on the source) is the one clause that refuses.
///
/// FAILS IF clause 0 refuses a caller who sees the edge (the doors then answer `404`), or the
/// source floor admits a caller who may only read the source — the bite: replace the
/// `"kb_resources"` arm's `write_floor::modify_floor_in_tx(..)` with `{}`, and retype, reweight,
/// fold and facet set answer `200`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_caller_who_sees_the_edge_but_may_not_modify_its_source_gets_403(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (owner, shared) = caller(&app.pool, "owner").await;
    let private = private_context(&app.pool, owner.profile).await;
    let (writer, _) = caller(&app.pool, "writer").await;
    grant_context_write(&app.pool, shared, writer.profile, owner.profile).await;

    let source = create(&app, &owner, shared, "read-only source").await;
    let target = create(&app, &owner, shared, "seen target").await;
    let edge = assert_edge(&app, &owner, source, target).await;
    move_to(&app, &owner, source, private).await;
    grant_read(&app, &owner, source, writer.profile).await;

    // Non-vacuity: the writer sees the edge and authors its home; only the source floor refuses.
    assert_eq!(
        edge_home(&app.pool, edge).await,
        shared,
        "homed in the shared context"
    );
    assert!(
        edge_visible(&app.pool, writer.profile, edge).await,
        "the writer sees the edge"
    );
    assert!(
        context_authorable(&app.pool, writer.profile, shared).await,
        "container-write admits the writer"
    );
    assert!(
        !can_modify(&app.pool, writer.profile, source).await,
        "the writer may not modify the source"
    );

    for (name, method, path, body) in edge_mutate_doors(edge, Uuid::now_v7()) {
        let (status, response) = call(&app, &writer, method, &path, body).await;
        assert_eq!(
            (status, response["error"]["code"].as_str()),
            (403, Some("FORBIDDEN")),
            "{name}: the source floor refuses a caller who sees the edge; body: {response}"
        );
    }
}
