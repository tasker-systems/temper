#![cfg(feature = "test-db")]
//! Cogmap-read direction lockstep flip (D4) end-to-end: drives the REAL Axum server, real Postgres,
//! real JWT auth, and the REAL resource-show read path (`resources_visible_to`). The cogmap-membership
//! read branch is now UP+union, so a member of a CHILD team reads a resource homed in a map joined to
//! an ANCESTOR (parent) team.
//!
//! This is the full-stack proof the visibility EXPANSION demands (#219's lesson: isolated-DB predicate
//! tests are not enough — the deny code + auth + handler must agree):
//!   • a child-team member GETs 200 on a resource homed in an ancestor-joined map;
//!   • a non-member GETs 404 on the same resource (deny-as-absence, never a leak).
//!
//! The resource is authored through the production `POST /api/ingest` path (homed in the map, so it
//! carries real doc_type frontmatter and renders on read), by the admin holding an explicit `can_write`
//! grant on the map (post-D3b authorship). Neither reader owns the resource, so a 200 is genuinely the
//! flipped cogmap-membership branch.

mod common;

use reqwest::StatusCode;
use uuid::Uuid;

/// Pre-flight a token (auto-provisions the profile), returning its UUID.
async fn provision_profile(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("preflight request failed");
    assert_eq!(resp.status(), StatusCode::OK, "preflight should succeed");
    let body: serde_json::Value = resp.json().await.expect("preflight json parse");
    // D11: a fresh principal is born Denied. Approve so this actor clears the front door
    // and the ENDPOINT authz (ownership, admin-only, grants) is what the test exercises.
    let __pid: Uuid = body["id"]
        .as_str()
        .expect("profile id missing")
        .parse()
        .expect("profile id parse");
    common::approve(&app.pool, __pid).await;
    __pid
}

async fn create_team(pool: &sqlx::PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .expect("create team")
}

/// Make `child` a child of `parent` — `team_ancestors(child) = {child, parent}`.
async fn link_parent(pool: &sqlx::PgPool, parent: Uuid, child: Uuid) {
    sqlx::query("INSERT INTO kb_teams_parents (parent_id, child_id) VALUES ($1, $2)")
        .bind(parent)
        .bind(child)
        .execute(pool)
        .await
        .expect("link parent/child team");
}

async fn add_member(pool: &sqlx::PgPool, team: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')",
    )
    .bind(team)
    .bind(profile)
    .execute(pool)
    .await
    .expect("add team member");
}

/// A real cognitive map (with its NOT-NULL telos resource) joined to `team`. Returns the cogmap id.
async fn create_map_joined_to(pool: &sqlx::PgPool, name: &str, team: Uuid) -> Uuid {
    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $2) RETURNING id",
    )
    .bind(format!("{name}-telos"))
    .bind(format!("temper://d4-e2e/{name}/telos"))
    .fetch_one(pool)
    .await
    .expect("insert telos");
    let cogmap: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(name)
    .bind(telos)
    .fetch_one(pool)
    .await
    .expect("insert cogmap");
    sqlx::query("INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES ($1, $2)")
        .bind(cogmap)
        .bind(team)
        .execute(pool)
        .await
        .expect("join cogmap to team");
    cogmap
}

/// Grant `profile` explicit `can_write` (+read) on `cogmap` — post-D3b authorship. `granted_by` is the
/// grantee (a fixture bootstrap standing in for the creator-seed / delegated grant).
async fn grant_cogmap_write(pool: &sqlx::PgPool, cogmap: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id, \
                                       can_read, can_write, granted_by_profile_id) \
         VALUES ('kb_cogmaps', $1, 'kb_profiles', $2, true, true, $2) \
         ON CONFLICT (subject_table, subject_id, principal_table, principal_id) DO NOTHING",
    )
    .bind(cogmap)
    .bind(profile)
    .execute(pool)
    .await
    .expect("grant cogmap write");
}

/// POST /api/ingest homed in `cogmap`, as `token`. Returns the created resource id.
async fn ingest_into_cogmap(
    app: &common::E2eTestApp,
    token: &str,
    cogmap: Uuid,
    title: &str,
    slug: &str,
) -> Uuid {
    let resp = app
        .reqwest_client
        .post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({
            "title": title,
            "origin_uri": format!("test://d4-read-flip/{}", Uuid::new_v4()),
            "context_ref": "",
            "home_cogmap_id": cogmap.to_string(),
            "doc_type_name": "research",
            "slug": slug,
            "content": "A resource homed in an ancestor-joined cognitive map.",
        }))
        .send()
        .await
        .expect("ingest request failed");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "admin with can_write authors the map's resource"
    );
    let body: serde_json::Value = resp.json().await.expect("ingest json parse");
    body["id"]
        .as_str()
        .expect("ingested resource id missing")
        .parse()
        .expect("resource id parse")
}

async fn show_status(app: &common::E2eTestApp, token: &str, resource: Uuid) -> StatusCode {
    app.reqwest_client
        .get(app.url(&format!("/api/resources/{resource}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("show request failed")
        .status()
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn child_member_reads_ancestor_joined_maps_resource(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    let admin_id = provision_profile(&app, &app.token).await;

    // Two more users: a child-team member, and a non-member (member of no relevant team).
    let child_token = common::generate_second_user_jwt();
    let child_id = provision_profile(&app, &child_token).await;
    let outsider_token =
        common::generate_test_jwt("e2e-d4-outsider", "d4-outsider@test.example.com");
    let outsider_id = provision_profile(&app, &outsider_token).await;

    // Team hierarchy: child ⟶ parent. The map + its resource live on the PARENT.
    let parent = create_team(&pool, "d4-e2e-parent").await;
    let child = create_team(&pool, "d4-e2e-child").await;
    link_parent(&pool, parent, child).await;
    add_member(&pool, child, child_id).await;
    // outsider is deliberately added to NO team joined to the map.
    let _ = outsider_id;

    let map = create_map_joined_to(&pool, "d4-e2e-parent-map", parent).await;

    // The admin authors a resource into the map via the production ingest path (needs can_write, D3b).
    grant_cogmap_write(&pool, map, admin_id).await;
    let resource =
        ingest_into_cogmap(&app, &app.token, map, "d4 ancestor doc", "d4-ancestor-doc").await;

    // ── EXPANSION: the child-team member reads the parent-joined map's resource (200), full stack.
    assert_eq!(
        show_status(&app, &child_token, resource).await,
        StatusCode::OK,
        "child-team member reads a resource homed in the ancestor-joined map"
    );

    // ── NON-LEAK: a non-member gets 404 (deny-as-absence), never the resource.
    assert_eq!(
        show_status(&app, &outsider_token, resource).await,
        StatusCode::NOT_FOUND,
        "a non-member cannot read the ancestor-joined map's resource"
    );
}
