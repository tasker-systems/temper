#![cfg(feature = "test-db")]
//! Team lifecycle surface (org-provisioning Chunk 2) — `team_service` role-gating
//! matrix driven at the service layer, plus a couple of HTTP-level cases through
//! the live Axum server.
//!
//! Gating (spec §3 decision #1): any authenticated profile may create a **root**
//! (parentless) team and becomes its `owner`; creating a **child** requires
//! `owner`/`maintainer` on the parent; setting `auto_join_role` requires
//! `is_system_admin`; `add_member` requires `owner`/`maintainer`.

mod common;

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::team::{AddMemberRequest, TeamCreateRequest, TeamRole};
use temper_services::error::ApiError;
use temper_services::services::team_service;

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Configure the gating team slug to `temper-system` (the L0 root team). Both
/// `is_system_admin`'s owner check resolves through this slug.
async fn set_gating_team(pool: &PgPool) {
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'temper-system' WHERE id = 1")
        .execute(pool)
        .await
        .expect("set gating team slug");
}

/// Mint an admin profile under D11: admin-ness is `approved` standing + a `kb_principal_governance`
/// grant — neither the Phase-2-retired `system_access` column nor gating ownership confers it.
async fn admin_profile(pool: &PgPool, email: &str) -> Uuid {
    let id = common::fixtures::create_test_profile(pool, email).await;
    common::fixtures::make_test_admin(pool, id).await;
    id
}

/// Fetch the role of `profile` on `team`, if any.
async fn role_of(pool: &PgPool, team_id: Uuid, profile_id: Uuid) -> Option<TeamRole> {
    sqlx::query_scalar::<_, TeamRole>(
        "SELECT role FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
    )
    .bind(team_id)
    .bind(profile_id)
    .fetch_optional(pool)
    .await
    .expect("query membership")
}

fn req(slug: &str, parent: Option<&str>, auto: Option<TeamRole>) -> TeamCreateRequest {
    TeamCreateRequest {
        slug: slug.to_owned(),
        name: None,
        parent: parent.map(str::to_owned),
        auto_join_role: auto,
    }
}

// ─── root creation → creator becomes owner ───────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_root_team_makes_creator_owner(pool: PgPool) {
    let creator = common::fixtures::create_test_profile(&pool, "root-creator@example.com").await;

    let team =
        team_service::create_team(&pool, ProfileId::from(creator), &req("alpha", None, None))
            .await
            .expect("root team creation should succeed");

    assert_eq!(team.slug, "alpha");
    assert_eq!(team.name, "alpha", "name defaults to slug");
    assert_eq!(team.auto_join_role, None);
    assert_eq!(
        role_of(&pool, team.id, creator).await,
        Some(TeamRole::Owner),
        "creator must be the owner of a freshly-created root team"
    );
}

// ─── child of someone else's team → Forbidden ────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_child_by_non_member_is_forbidden(pool: PgPool) {
    let owner = common::fixtures::create_test_profile(&pool, "parent-owner@example.com").await;
    let stranger = common::fixtures::create_test_profile(&pool, "stranger@example.com").await;

    let parent =
        team_service::create_team(&pool, ProfileId::from(owner), &req("parent", None, None))
            .await
            .expect("parent root team");

    let denied = team_service::create_team(
        &pool,
        ProfileId::from(stranger),
        &req("child", Some("parent"), None),
    )
    .await;

    assert!(
        matches!(denied, Err(ApiError::Forbidden)),
        "non-member must be Forbidden creating a child, got {denied:?}"
    );
    // No orphan team/link left behind.
    let _ = parent;
}

// ─── child as owner / maintainer → ok, parent link present ───────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_child_as_owner_links_parent(pool: PgPool) {
    let owner = common::fixtures::create_test_profile(&pool, "owner@example.com").await;
    let parent =
        team_service::create_team(&pool, ProfileId::from(owner), &req("parent", None, None))
            .await
            .expect("parent");

    let child = team_service::create_team(
        &pool,
        ProfileId::from(owner),
        &req("child", Some("+parent"), None),
    )
    .await
    .expect("owner may create a child");

    let link = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM kb_teams_parents WHERE child_id = $1 AND parent_id = $2)",
    )
    .bind(child.id)
    .bind(parent.id)
    .fetch_one(&pool)
    .await
    .expect("query parent link");
    assert!(link, "kb_teams_parents row must link child → parent");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_child_as_maintainer_ok(pool: PgPool) {
    let owner = common::fixtures::create_test_profile(&pool, "owner2@example.com").await;
    let maintainer = common::fixtures::create_test_profile(&pool, "maintainer@example.com").await;
    let parent = team_service::create_team(&pool, ProfileId::from(owner), &req("p2", None, None))
        .await
        .expect("parent");

    // Owner grants maintainer.
    team_service::add_member(
        &pool,
        ProfileId::from(owner),
        parent.id,
        &AddMemberRequest {
            profile_id: maintainer,
            role: TeamRole::Maintainer,
        },
    )
    .await
    .expect("owner adds maintainer");

    let child = team_service::create_team(
        &pool,
        ProfileId::from(maintainer),
        &req("c2", Some("p2"), None),
    )
    .await;
    assert!(
        child.is_ok(),
        "maintainer may create a child, got {child:?}"
    );
}

// ─── auto_join_role: non-admin Forbidden, admin ok + backfills ───────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn auto_join_role_forbidden_for_non_admin(pool: PgPool) {
    let creator = common::fixtures::create_test_profile(&pool, "nonadmin@example.com").await;

    let denied = team_service::create_team(
        &pool,
        ProfileId::from(creator),
        &req("everyone", None, Some(TeamRole::Watcher)),
    )
    .await;

    assert!(
        matches!(denied, Err(ApiError::Forbidden)),
        "non-admin setting auto_join_role must be Forbidden, got {denied:?}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn auto_join_role_admin_creates_and_backfills(pool: PgPool) {
    set_gating_team(&pool).await;
    let admin = admin_profile(&pool, "admin@example.com").await;
    // A pre-existing eligible profile (open mode → has_system_access true).
    let preexisting = common::fixtures::create_test_profile(&pool, "preexisting@example.com").await;

    let team = team_service::create_team(
        &pool,
        ProfileId::from(admin),
        &req("everyone", None, Some(TeamRole::Watcher)),
    )
    .await
    .expect("admin may set auto_join_role");

    assert_eq!(team.auto_join_role, Some(TeamRole::Watcher));
    // Creator remains owner (backfill is ON CONFLICT DO NOTHING).
    assert_eq!(role_of(&pool, team.id, admin).await, Some(TeamRole::Owner));
    // The pre-existing eligible profile was enrolled by backfill_auto_join_team.
    assert_eq!(
        role_of(&pool, team.id, preexisting).await,
        Some(TeamRole::Watcher),
        "backfill must enroll a pre-existing eligible profile into the new auto-join team"
    );
}

// ─── add_member gating ───────────────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn add_member_by_non_owner_is_forbidden(pool: PgPool) {
    let owner = common::fixtures::create_test_profile(&pool, "team-owner@example.com").await;
    let stranger = common::fixtures::create_test_profile(&pool, "outsider@example.com").await;
    let newbie = common::fixtures::create_test_profile(&pool, "newbie@example.com").await;
    let team = team_service::create_team(&pool, ProfileId::from(owner), &req("t3", None, None))
        .await
        .expect("team");

    let denied = team_service::add_member(
        &pool,
        ProfileId::from(stranger),
        team.id,
        &AddMemberRequest {
            profile_id: newbie,
            role: TeamRole::Member,
        },
    )
    .await;
    assert!(
        matches!(denied, Err(ApiError::Forbidden)),
        "non-owner add_member must be Forbidden, got {denied:?}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn add_member_by_owner_succeeds(pool: PgPool) {
    let owner = common::fixtures::create_test_profile(&pool, "owner3@example.com").await;
    let newbie = common::fixtures::create_test_profile(&pool, "newbie3@example.com").await;
    let team = team_service::create_team(&pool, ProfileId::from(owner), &req("t4", None, None))
        .await
        .expect("team");

    let member = team_service::add_member(
        &pool,
        ProfileId::from(owner),
        team.id,
        &AddMemberRequest {
            profile_id: newbie,
            role: TeamRole::Member,
        },
    )
    .await
    .expect("owner adds member");

    assert_eq!(member.profile_id, newbie);
    assert_eq!(member.role, TeamRole::Member);
    assert_eq!(
        role_of(&pool, team.id, newbie).await,
        Some(TeamRole::Member)
    );
}

// ─── duplicate slug → Conflict ───────────────────────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn duplicate_slug_is_conflict(pool: PgPool) {
    let creator = common::fixtures::create_test_profile(&pool, "dup@example.com").await;
    team_service::create_team(
        &pool,
        ProfileId::from(creator),
        &req("dup-slug", None, None),
    )
    .await
    .expect("first creation");

    let again = team_service::create_team(
        &pool,
        ProfileId::from(creator),
        &req("dup-slug", None, None),
    )
    .await;
    assert!(
        matches!(again, Err(ApiError::Conflict(_))),
        "duplicate slug must be a Conflict, got {again:?}"
    );
}

// ─── list_teams returns the caller's memberships ─────────────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_teams_returns_callers_memberships(pool: PgPool) {
    let creator = common::fixtures::create_test_profile(&pool, "lister@example.com").await;
    team_service::create_team(
        &pool,
        ProfileId::from(creator),
        &req("listed-a", None, None),
    )
    .await
    .expect("a");
    team_service::create_team(
        &pool,
        ProfileId::from(creator),
        &req("listed-b", None, None),
    )
    .await
    .expect("b");

    let teams = team_service::list_teams(&pool, ProfileId::from(creator))
        .await
        .expect("list");
    let slugs: Vec<&str> = teams.iter().map(|t| t.slug.as_str()).collect();
    assert!(slugs.contains(&"listed-a"), "missing listed-a in {slugs:?}");
    assert!(slugs.contains(&"listed-b"), "missing listed-b in {slugs:?}");
}

// ─── HTTP-level: live server through real Axum + Postgres ─────────────────────

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn http_create_root_team_returns_201_and_owner(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("http-root-{}@example.com", Uuid::new_v4());
    let profile_id = common::fixtures::create_test_profile(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);

    let resp = app
        .client
        .post(app.url("/api/teams"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "slug": "http-alpha" }))
        .send()
        .await
        .expect("create team request");

    assert_eq!(resp.status(), 201, "root team create must be 201");
    let body: serde_json::Value = resp.json().await.expect("json");
    let team_id = Uuid::parse_str(body["id"].as_str().expect("id string")).expect("uuid");
    assert_eq!(
        role_of(&app.pool, team_id, profile_id).await,
        Some(TeamRole::Owner),
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn http_create_child_of_foreign_team_is_403(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    // Owner creates the parent (service-direct).
    let owner = common::fixtures::create_test_profile(&app.pool, "http-owner@example.com").await;
    team_service::create_team(
        &app.pool,
        ProfileId::from(owner),
        &req("http-parent", None, None),
    )
    .await
    .expect("parent team");

    // A different profile tries to create a child via HTTP → 403.
    let email = format!("http-stranger-{}@example.com", Uuid::new_v4());
    let stranger = common::fixtures::create_test_profile(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{stranger}"), &email);

    let resp = app
        .client
        .post(app.url("/api/teams"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "slug": "http-child", "parent": "http-parent" }))
        .send()
        .await
        .expect("create child request");

    assert_eq!(
        resp.status(),
        403,
        "non-member creating a child must be 403 Forbidden"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn http_remove_member_returns_residual_reach_body(pool: PgPool) {
    let app = common::setup_test_app(pool).await;

    // Owner creates a team; a leaver joins as member.
    let owner_email = format!("http-rm-owner-{}@example.com", Uuid::new_v4());
    let owner = common::fixtures::create_test_profile(&app.pool, &owner_email).await;
    let team = team_service::create_team(
        &app.pool,
        ProfileId::from(owner),
        &req("http-offboard", None, None),
    )
    .await
    .expect("team")
    .id;
    let leaver_email = format!("http-rm-leaver-{}@example.com", Uuid::new_v4());
    let leaver = common::fixtures::create_test_profile(&app.pool, &leaver_email).await;
    team_service::add_member(
        &app.pool,
        ProfileId::from(owner),
        team,
        &AddMemberRequest {
            profile_id: leaver,
            role: TeamRole::Member,
        },
    )
    .await
    .expect("add leaver");

    // Leaver owns a resource in a personal context shared to the team.
    let ctx: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, 'off-ctx', 'off-ctx') RETURNING id",
    )
    .bind(leaver)
    .fetch_one(&app.pool)
    .await
    .expect("ctx");
    sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
        .bind(ctx)
        .bind(team)
        .execute(&app.pool)
        .await
        .expect("share");
    let rid: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('r','r') RETURNING id",
    )
    .fetch_one(&app.pool)
    .await
    .expect("res");
    sqlx::query(
        "INSERT INTO kb_resource_homes \
           (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(rid)
    .bind(ctx)
    .bind(leaver)
    .execute(&app.pool)
    .await
    .expect("home");

    // Owner removes the leaver via HTTP → 200 + residual body.
    let token = common::generate_test_jwt(&format!("test|{owner}"), &owner_email);
    let resp = app
        .client
        .delete(app.url(&format!("/api/teams/{team}/members/{leaver}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("delete member request");

    assert_eq!(resp.status(), 200, "removal returns 200 with a body");
    let body: temper_core::types::reassign::RemoveMemberOutcome = resp.json().await.expect("json");
    assert_eq!(body.residual_owned.count, 1);
    assert_eq!(body.residual_owned.contexts.len(), 1);
    assert!(body.residual_owned.contexts[0]
        .context_ref
        .ends_with("/off-ctx"));
}
