#![cfg(feature = "test-db")]
//! Team member-lifecycle e2e (Task 8): drives the real Axum server + Postgres to prove the
//! GET `/api/teams/{id}`, DELETE `/api/teams/{id}/members/{profile_id}`, and PATCH
//! `/api/teams/{id}/members/{profile_id}` endpoints (already implemented and committed in
//! `temper-services::team_service`) behave correctly end-to-end: deny-as-absence for
//! non-members, self-leave vs. removing-others authorization, the last-owner guard, the
//! idp-sourced-row guard, and the owner-role-grant guard on the PATCH endpoint.
//!
//! Modeled on `context_share_e2e.rs` — same `provision` helper, same raw
//! `app.reqwest_client` + Bearer-header idiom for endpoints not (yet) wrapped by
//! `TemperClient`, same `#[sqlx::test(migrator = "temper_api::MIGRATOR")]` harness. This test
//! never needs a system admin (the `is_system_admin` branch of `team_detail` is covered by
//! unit tests in `team_service.rs`), so it skips `root_bootstrap_first_admin` entirely.

mod common;

use reqwest::StatusCode;
use serde_json::Value;
use uuid::Uuid;

use temper_core::types::team::{AddMemberRequest, TeamCreateRequest, TeamRole};

/// Provision a profile by hitting an authed endpoint (auto-provision on first request).
/// Copied verbatim from `context_share_e2e.rs`.
async fn provision(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = app
        .reqwest_client
        .get(app.url("/api/profile"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("preflight");
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("json");
    // D11: a fresh principal is born Denied. Approve so this actor clears the front door
    // and the ENDPOINT authz (ownership, admin-only, grants) is what the test exercises.
    let __pid: Uuid = body["id"].as_str().expect("id").parse().expect("uuid");
    common::approve(&app.pool, __pid).await;
    __pid
}

/// `GET /api/teams/{team_id}` as `token`. Returns the raw `reqwest::Response` so callers can
/// assert status and, on success, parse the `TeamDetail` body (in particular `members`).
async fn get_team(app: &common::E2eTestApp, token: &str, team_id: Uuid) -> reqwest::Response {
    app.reqwest_client
        .get(app.url(&format!("/api/teams/{team_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("get_team request failed")
}

/// `DELETE /api/teams/{team_id}/members/{profile_id}` as `token`. Returns just the status —
/// none of these assertions need the (empty) body.
async fn delete_member(
    app: &common::E2eTestApp,
    token: &str,
    team_id: Uuid,
    profile_id: Uuid,
) -> StatusCode {
    app.reqwest_client
        .delete(app.url(&format!("/api/teams/{team_id}/members/{profile_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("delete_member request failed")
        .status()
}

/// `PATCH /api/teams/{team_id}/members/{profile_id}` with `{"role": role}` as `token`.
async fn patch_role(
    app: &common::E2eTestApp,
    token: &str,
    team_id: Uuid,
    profile_id: Uuid,
    role: &str,
) -> StatusCode {
    app.reqwest_client
        .patch(app.url(&format!("/api/teams/{team_id}/members/{profile_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "role": role }))
        .send()
        .await
        .expect("patch_role request failed")
        .status()
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn team_member_lifecycle_matrix(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Four distinct profiles: the owner (the app's built-in token/client), a plain member,
    // a non-member outsider, and a fourth profile we'll graft in as an idp-sourced row.
    let owner_token = app.token.clone();
    let owner_id = provision(&app, &owner_token).await;

    let member_token = common::generate_second_user_jwt();
    let member_id = provision(&app, &member_token).await;

    let outsider_token = common::generate_test_jwt("outsider-sub", "outsider@example.com");
    // Provisioning (not the id itself) is what matters here: the outsider must exist as a
    // profile so the GET-as-non-member assertion below exercises a real "not a member of this
    // team" deny, not an auth failure on an unprovisioned caller.
    let _outsider_id = provision(&app, &outsider_token).await;

    let idp_token = common::generate_test_jwt("idp-sub", "idp@example.com");
    let idp_id = provision(&app, &idp_token).await;

    // 2. Owner creates team T; owner becomes its sole owner+member.
    let team = app
        .client
        .teams()
        .create(&TeamCreateRequest {
            slug: "lifecycle-team".to_owned(),
            name: None,
            parent: None,
            auto_join_role: None,
        })
        .await
        .expect("owner creates team");

    // 3. GET as owner -> 200, members == [owner as "owner"].
    let resp = get_team(&app, &owner_token, team.id).await;
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "owner can see their own team"
    );
    let body: Value = resp.json().await.expect("team detail json");
    let members = body["members"].as_array().expect("members array");
    assert_eq!(members.len(), 1, "team starts with just the owner");
    assert_eq!(
        members[0]["profile_id"].as_str().expect("profile_id"),
        owner_id.to_string(),
        "sole member is the owner"
    );
    assert_eq!(
        members[0]["role"].as_str().expect("role"),
        "owner",
        "owner's role is 'owner'"
    );

    // 4. Owner adds member. GET as owner -> 2 members. GET as outsider -> 404 (deny-as-absence).
    app.client
        .teams()
        .add_member(
            team.id,
            &AddMemberRequest {
                profile_id: member_id,
                role: TeamRole::Member,
            },
        )
        .await
        .expect("owner adds member");

    let resp = get_team(&app, &owner_token, team.id).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("team detail json");
    assert_eq!(
        body["members"].as_array().expect("members array").len(),
        2,
        "team now has owner + member"
    );

    let resp = get_team(&app, &outsider_token, team.id).await;
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "non-member cannot see the team (deny-as-absence)"
    );

    // 5. Owner promotes member to maintainer -> 200; re-GET confirms the new role.
    let status = patch_role(&app, &owner_token, team.id, member_id, "maintainer").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "owner can change a member's role to maintainer"
    );
    let resp = get_team(&app, &owner_token, team.id).await;
    let body: Value = resp.json().await.expect("team detail json");
    let members = body["members"].as_array().expect("members array");
    let member_entry = members
        .iter()
        .find(|m| m["profile_id"].as_str() == Some(&member_id.to_string()))
        .expect("member entry present");
    assert_eq!(
        member_entry["role"].as_str().expect("role"),
        "maintainer",
        "member's role updated to maintainer"
    );

    // 6. Owner cannot grant "owner" via the role-change endpoint -> 400.
    let status = patch_role(&app, &owner_token, team.id, member_id, "owner").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "cannot grant owner via role change"
    );

    // 7. Member self-leaves -> 200 with the residual-owned-reach body. Re-add as a
    //    plain member for the next steps.
    let status = delete_member(&app, &member_token, team.id, member_id).await;
    assert_eq!(status, StatusCode::OK, "member can self-leave");

    app.client
        .teams()
        .add_member(
            team.id,
            &AddMemberRequest {
                profile_id: member_id,
                role: TeamRole::Member,
            },
        )
        .await
        .expect("owner re-adds member");

    // 8. Plain member cannot remove the owner -> 403.
    let status = delete_member(&app, &member_token, team.id, owner_id).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "plain member cannot remove another member (the owner)"
    );

    // 9. Owner cannot self-leave as the last owner -> 409.
    let status = delete_member(&app, &owner_token, team.id, owner_id).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "last owner cannot remove themselves"
    );

    // 10. An idp-sourced member row cannot be removed by the owner -> 409.
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role, source) VALUES ($1, $2, 'member', 'idp')",
    )
    .bind(team.id)
    .bind(idp_id)
    .execute(&pool)
    .await
    .expect("insert idp member");

    let status = delete_member(&app, &owner_token, team.id, idp_id).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "owner cannot remove an idp-sourced member"
    );

    // 11. CLI wiring: `temper team show` exits 0 against the same team.
    let out = common::run_temper_cli(&app, &["team", "show", &team.id.to_string()])
        .await
        .expect("cli");
    assert!(
        out.status.success(),
        "team show exits 0: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
