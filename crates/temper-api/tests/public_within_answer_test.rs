#![cfg(feature = "test-db")]
//! The opt-in `?answer=effects` on `POST /api/teams`, `DELETE /api/teams/{id}` and
//! `DELETE /api/teams/{id}/children/{child_id}`, through the live server.
//!
//! Without the parameter each act answers exactly as it did before the parameter existed: the bare
//! `TeamRow` keys, or an empty `204`. With it, each answers what the act did to `public_within`
//! rows under the same naming rule as every other act answer: a row is named only to a caller who
//! manages its team or its anchor, or is a system admin, and counted otherwise.
//!
//! The world: `pw-ans-a` → `pw-ans-p` → `pw-ans-t`, with the row `(t, a)` open. `all` owns all three
//! teams. `mid` owns only the middle team, so it may detach `t`, delete `p` and create under `p`
//! while managing neither side of the row.

mod common;

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::team::TeamCreateRequest;
use temper_services::services::team_service;

struct World {
    app: common::TestApp,
    all_id: Uuid,
    all: String,
    mid: String,
    admin: String,
    a: Uuid,
    p: Uuid,
    t: Uuid,
}

async fn profile(pool: &PgPool, tag: &str) -> (Uuid, String) {
    let email = format!("{tag}-{}@example.com", Uuid::new_v4());
    let id = common::fixtures::create_test_profile(pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{id}"), &email);
    (id, token)
}

async fn world(pool: PgPool) -> World {
    let app = common::setup_test_app(pool).await;
    let (all_id, all) = profile(&app.pool, "all").await;
    let p_all = temper_services::test_support::human_principal_for(&app.pool, all_id).await;
    let mut ids = Vec::new();
    for (slug, parent) in [
        ("pw-ans-a", None),
        ("pw-ans-p", Some("pw-ans-a")),
        ("pw-ans-t", Some("pw-ans-p")),
    ] {
        let req = TeamCreateRequest {
            slug: slug.to_owned(),
            parent: parent.map(str::to_owned),
            ..Default::default()
        };
        ids.push(
            team_service::create_team(&app.pool, &p_all, &req)
                .await
                .expect("team")
                .id,
        );
    }
    let (a, p, t) = (ids[0], ids[1], ids[2]);
    team_service::open_public_within(&app.pool, &p_all, t, a)
        .await
        .expect("both sides open (t, a)");

    let (mid_id, mid) = profile(&app.pool, "mid").await;
    sqlx::query("INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner')")
        .bind(p)
        .bind(mid_id)
        .execute(&app.pool)
        .await
        .expect("mid owns the middle team only");
    let (admin_id, admin) = profile(&app.pool, "admin").await;
    common::fixtures::make_test_admin(&app.pool, admin_id).await;
    World {
        app,
        all_id,
        all,
        mid,
        admin,
        a,
        p,
        t,
    }
}

/// Send and return the status and the raw body text.
async fn send(
    w: &World,
    method: reqwest::Method,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (u16, String) {
    let mut rb = w
        .app
        .client
        .request(method, w.app.url(path))
        .header("Authorization", format!("Bearer {token}"));
    if let Some(b) = body {
        rb = rb.json(&b);
    }
    let resp = rb.send().await.expect("request");
    let status = resp.status().as_u16();
    (status, resp.text().await.expect("body"))
}

fn json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

fn row(team: Uuid, anchor: Uuid) -> impl Fn(&Value) -> bool {
    move |r: &Value| r["team"]["id"] == team.to_string() && r["anchor"]["id"] == anchor.to_string()
}

/// FAILS IF an answer asked without the parameter carries anything it did not carry before: a key
/// beyond the six `TeamRow` columns, or a body on a `204`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn without_the_parameter_every_answer_is_unchanged(pool: PgPool) {
    let w = world(pool).await;

    let (status, text) = send(
        &w,
        reqwest::Method::POST,
        "/api/teams",
        &w.all,
        Some(serde_json::json!({ "slug": "pw-ans-new", "parent": "pw-ans-a" })),
    )
    .await;
    assert_eq!(status, 201, "{text}");
    let mut keys: Vec<String> = json(&text).as_object().unwrap().keys().cloned().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "auto_join_role",
            "created",
            "description",
            "id",
            "name",
            "slug"
        ],
        "{text}"
    );

    let detach = format!("/api/teams/{}/children/{}", w.p, w.t);
    let (status, text) = send(&w, reqwest::Method::DELETE, &detach, &w.all, None).await;
    assert_eq!((status, text.as_str()), (204, ""), "detach");

    let delete = format!("/api/teams/{}", w.p);
    let (status, text) = send(&w, reqwest::Method::DELETE, &delete, &w.all, None).await;
    assert_eq!((status, text.as_str()), (204, ""), "delete");
}

/// A detach that lapses `(t, a)` is counted, never named, to a manager of the middle team alone.
///
/// FAILS IF the detach answer names a row to a caller who manages neither of its teams.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_detach_answers_its_lapse_named_only_to_either_side(pool: PgPool) {
    let w = world(pool).await;
    let path = format!("/api/teams/{}/children/{}?answer=effects", w.p, w.t);
    let (status, text) = send(&w, reqwest::Method::DELETE, &path, &w.mid, None).await;
    assert_eq!(status, 200, "{text}");
    let body = json(&text);
    assert_eq!(body["lapsed"], serde_json::json!([]), "{body}");
    assert_eq!(body["lapsed_unnamed"], 1, "{body}");
    let leaked = text.contains(&w.t.to_string()) || text.contains(&w.a.to_string());
    assert!(!leaked, "neither team is named to mid: {text}");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_detach_names_the_lapse_to_a_manager_of_both(pool: PgPool) {
    let w = world(pool).await;
    let path = format!("/api/teams/{}/children/{}?answer=effects", w.p, w.t);
    let (status, text) = send(&w, reqwest::Method::DELETE, &path, &w.all, None).await;
    assert_eq!(status, 200, "{text}");
    let body = json(&text);
    assert!(
        body["lapsed"].as_array().unwrap().iter().any(row(w.t, w.a)),
        "{body}"
    );
    assert_eq!(body["lapsed_unnamed"], 0, "{body}");
}

/// Deleting the middle team makes `(t, a)` stop conferring: counted to the middle team's owner,
/// named to a system admin.
///
/// FAILS IF the delete answer names a row to a caller who manages neither of its teams, or withholds
/// it from an admin.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_delete_answers_its_dormant_rows_by_the_naming_rule(pool: PgPool) {
    let w = world(pool).await;
    let path = format!("/api/teams/{}?answer=effects", w.p);
    let (status, text) = send(&w, reqwest::Method::DELETE, &path, &w.mid, None).await;
    assert_eq!(status, 200, "{text}");
    let body = json(&text);
    assert_eq!(body["dormant"], serde_json::json!([]), "{body}");
    assert_eq!(body["dormant_unnamed"], 1, "{body}");

    // Restored, then deleted again by an admin, who is told the row by name.
    let p_all = temper_services::test_support::human_principal_for(&w.app.pool, w.all_id).await;
    team_service::restore_team(&w.app.pool, &p_all, w.p)
        .await
        .expect("restore");
    let (status, text) = send(&w, reqwest::Method::DELETE, &path, &w.admin, None).await;
    assert_eq!(status, 200, "{text}");
    let body = json(&text);
    assert!(
        body["dormant"]
            .as_array()
            .unwrap()
            .iter()
            .any(row(w.t, w.a)),
        "{body}"
    );
}

/// A team created under the middle team reads `(t, a)` through its parents: named to a manager of
/// either side, counted to the middle team's owner. The answer is still `201 TeamRow`.
///
/// FAILS IF the create answer names a row its caller manages neither side of, or drops the row.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_create_answers_the_rows_its_roster_reads_by_the_naming_rule(pool: PgPool) {
    let w = world(pool).await;
    for (token, slug, named) in [(&w.mid, "pw-ans-c1", false), (&w.all, "pw-ans-c2", true)] {
        let (status, text) = send(
            &w,
            reqwest::Method::POST,
            "/api/teams?answer=effects",
            token,
            Some(serde_json::json!({ "slug": slug, "parent": w.p.to_string() })),
        )
        .await;
        assert_eq!(status, 201, "{text}");
        let body = json(&text);
        assert_eq!(body["slug"], slug, "the row is still the answer: {body}");
        let reach = &body["public_within_in_reach"];
        assert!(body["public_within"].is_object(), "{body}");
        if named {
            assert!(
                reach["rows"].as_array().unwrap().iter().any(row(w.t, w.a)),
                "{body}"
            );
            assert_eq!(reach["unnamed"], 0, "{body}");
        } else {
            assert_eq!(reach["rows"], serde_json::json!([]), "{body}");
            assert_eq!(reach["unnamed"], 1, "{body}");
            assert!(
                !text.contains(&w.t.to_string()) && !text.contains(&w.a.to_string()),
                "{text}"
            );
        }
    }
}
