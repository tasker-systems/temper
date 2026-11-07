#![cfg(feature = "test-db")]
//! `public_within`'s doors through the live server: open, close, the forbid, discovery and the team
//! detail, with the status and error code each answers. The rules themselves are pinned at the
//! service layer (`temper-services/tests/public_within_test.rs`); this file pins the wire. A
//! machine principal's refusal on every one of these routes is `machine_refusal_table_test.rs`'s,
//! which enumerates every authenticated route from the OpenAPI document.

mod common;

use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::team::TeamCreateRequest;
use temper_services::services::team_service;

struct World {
    app: common::TestApp,
    /// A direct owner of `org`, `eng` and `eng-sec`: both sides of `(eng-sec, eng)`.
    both: String,
    /// A member of `eng-web`, under the anchor, managing nothing.
    reader: String,
    admin: String,
    org: Uuid,
    eng: Uuid,
    eng_sec: Uuid,
    eng_web: Uuid,
}

fn req(slug: &str, parent: Option<&str>) -> TeamCreateRequest {
    TeamCreateRequest {
        slug: slug.to_owned(),
        name: None,
        parent: parent.map(str::to_owned),
        parents: None,
        auto_join_role: None,
    }
}

async fn profile(pool: &PgPool, tag: &str) -> (Uuid, String) {
    let email = format!("{tag}-{}@example.com", Uuid::new_v4());
    let id = common::fixtures::create_test_profile(pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{id}"), &email);
    (id, token)
}

async fn world(pool: PgPool) -> World {
    let app = common::setup_test_app(pool).await;
    let (owner, both) = profile(&app.pool, "both").await;
    let p = temper_services::test_support::human_principal_for(&app.pool, owner).await;
    let mk = |slug: &'static str, parent: Option<&'static str>| {
        let pool = app.pool.clone();
        let p = p.clone();
        async move {
            team_service::create_team(&pool, &p, &req(slug, parent))
                .await
                .expect("team")
                .id
        }
    };
    let org = mk("pw-org", None).await;
    let eng = mk("pw-eng", Some("pw-org")).await;
    let eng_sec = mk("pw-eng-sec", Some("pw-eng")).await;
    let eng_web = mk("pw-eng-web", Some("pw-eng")).await;
    let (reader_id, reader) = profile(&app.pool, "reader").await;
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(eng_web)
    .bind(reader_id)
    .execute(&app.pool)
    .await
    .expect("reader joins eng-web");
    let (admin_id, admin) = profile(&app.pool, "admin").await;
    common::fixtures::make_test_admin(&app.pool, admin_id).await;
    World {
        app,
        both,
        reader,
        admin,
        org,
        eng,
        eng_sec,
        eng_web,
    }
}

async fn send(
    w: &World,
    method: reqwest::Method,
    path: &str,
    token: &str,
    body: Option<Value>,
) -> (u16, Value) {
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
    let text = resp.text().await.expect("body");
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

fn code(body: &Value) -> &str {
    body["error"]["code"]
        .as_str()
        .or_else(|| body["code"].as_str())
        .unwrap_or("")
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn http_open_answers_the_outcome_and_each_refusal_its_status(pool: PgPool) {
    let w = world(pool).await;
    let path = format!("/api/teams/{}/public-within/{}", w.eng_sec, w.eng);

    // A caller on no side: the bare 403.
    let (status, body) = send(&w, reqwest::Method::PUT, &path, &w.reader, None).await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(code(&body), "FORBIDDEN", "{body}");

    // Not an ancestor: 400.
    let sideways = format!("/api/teams/{}/public-within/{}", w.eng_sec, w.eng_web);
    let (status, body) = send(&w, reqwest::Method::PUT, &sideways, &w.both, None).await;
    assert_eq!(status, 400, "{body}");

    // A forbid above: 403 with the detail code, told to a manager of the team.
    sqlx::query("UPDATE kb_teams SET public_within_forbidden = true WHERE id = $1")
        .bind(w.org)
        .execute(&w.app.pool)
        .await
        .unwrap();
    let (status, body) = send(&w, reqwest::Method::PUT, &path, &w.both, None).await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(
        code(&body),
        temper_core::error::FORBIDDEN_DETAIL_CODE,
        "{body}"
    );
    assert!(
        body.to_string().contains("public_within_forbidden_here"),
        "{body}"
    );
    sqlx::query("UPDATE kb_teams SET public_within_forbidden = false WHERE id = $1")
        .bind(w.org)
        .execute(&w.app.pool)
        .await
        .unwrap();

    // Both sides: opened, then exists.
    for expected in ["opened", "exists"] {
        let (status, body) = send(&w, reqwest::Method::PUT, &path, &w.both, None).await;
        assert_eq!(status, 200, "{body}");
        assert_eq!(body["outcome"], expected);
        assert_eq!(body["instance_wide"], false);
        assert!(body["audience"]["teams"].as_i64().unwrap() >= 3, "{body}");
        assert!(
            body["audience"]["principals"].as_i64().unwrap() >= 2,
            "{body}"
        );
    }

    // The reader discovers it.
    let (status, body) = send(
        &w,
        reqwest::Method::GET,
        "/api/teams/public-within",
        &w.reader,
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body.as_array().unwrap().len(), 1, "{body}");
    assert_eq!(body[0]["team"]["id"], json!(w.eng_sec));
    assert_eq!(body[0]["anchor"]["id"], json!(w.eng));
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn http_close_is_204_then_404_and_403_to_a_non_manager(pool: PgPool) {
    let w = world(pool).await;
    let path = format!("/api/teams/{}/public-within/{}", w.eng_sec, w.eng);
    let (status, _) = send(&w, reqwest::Method::PUT, &path, &w.both, None).await;
    assert_eq!(status, 200);

    let (status, body) = send(&w, reqwest::Method::DELETE, &path, &w.reader, None).await;
    assert_eq!(status, 403, "{body}");
    let (status, body) = send(&w, reqwest::Method::DELETE, &path, &w.both, None).await;
    assert_eq!(status, 204, "{body}");
    let (status, body) = send(&w, reqwest::Method::DELETE, &path, &w.both, None).await;
    assert_eq!(status, 404, "{body}");
    // The non-manager's answer does not change with the row gone.
    let (status, body) = send(&w, reqwest::Method::DELETE, &path, &w.reader, None).await;
    assert_eq!(status, 403, "{body}");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn http_policy_answers_the_rows_it_flips_and_the_root_is_an_admins(pool: PgPool) {
    let w = world(pool).await;
    let open = format!("/api/teams/{}/public-within/{}", w.eng_sec, w.eng);
    let (status, _) = send(&w, reqwest::Method::PUT, &open, &w.both, None).await;
    assert_eq!(status, 200);

    let policy = format!("/api/teams/{}/public-within-policy", w.org);
    let (status, body) = send(
        &w,
        reqwest::Method::PUT,
        &policy,
        &w.reader,
        Some(json!({"forbidden": true})),
    )
    .await;
    assert_eq!(status, 403, "{body}");
    let (status, body) = send(
        &w,
        reqwest::Method::PUT,
        &policy,
        &w.both,
        Some(json!({"forbidden": true})),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["changed"], true);
    assert_eq!(body["dormant"].as_array().unwrap().len(), 1, "{body}");
    assert_eq!(body["dormant"][0]["team"]["id"], json!(w.eng_sec));

    // The detail shows the row to T's members as inert, and to A's managers.
    let (status, body) = send(
        &w,
        reqwest::Method::GET,
        &format!("/api/teams/{}", w.eng_sec),
        &w.both,
        None,
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["public_within"][0]["anchor"]["id"], json!(w.eng));
    assert_eq!(body["public_within"][0]["effective"], false);
    let (_, body) = send(
        &w,
        reqwest::Method::GET,
        &format!("/api/teams/{}", w.eng),
        &w.both,
        None,
    )
    .await;
    assert_eq!(body["public_within_in"][0]["team"]["id"], json!(w.eng_sec));

    // The root: the owner of every other team here is refused; an admin is not.
    let root: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
        .fetch_one(&w.app.pool)
        .await
        .unwrap();
    let root_policy = format!("/api/teams/{root}/public-within-policy");
    let (status, body) = send(
        &w,
        reqwest::Method::PUT,
        &root_policy,
        &w.both,
        Some(json!({"forbidden": true})),
    )
    .await;
    assert_eq!(status, 403, "{body}");
    let (status, body) = send(
        &w,
        reqwest::Method::PUT,
        &root_policy,
        &w.admin,
        Some(json!({"forbidden": true})),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["forbidden"], true);
}
