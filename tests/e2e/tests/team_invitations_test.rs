#![cfg(feature = "test-db")]
//! Team invitations e2e: drives the real Axum server + Postgres through the full
//! invite → accept flow across two distinct profiles.
//!
//! Owner-authed writes go through `TemperClient` (`app.client`, bound to the
//! owner token); the invitee's accept/decline go through raw `reqwest` with the
//! invitee's Bearer token (the invitee is a different principal than
//! `app.client`). Modeled on `team_member_lifecycle_test.rs` — same `provision`
//! helper, same `#[sqlx::test(migrator = "temper_api::MIGRATOR")]` harness.
//!
//! Acceptance is bearer-authority: the 128-bit token is the authority and
//! `invited_email` need not match the caller's identity, so the test invites an
//! arbitrary email and accepts as the invitee.

mod common;

use reqwest::StatusCode;
use serde_json::Value;
use uuid::Uuid;

use temper_core::types::invitation::CreateInvitationRequest;
use temper_core::types::team::{TeamCreateRequest, TeamRole};

/// Provision a profile by hitting an authed endpoint (auto-provision on first request).
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
    body["id"].as_str().expect("id").parse().expect("uuid")
}

/// `POST /api/invitations/{token}/accept` as `token`.
async fn accept(app: &common::E2eTestApp, token: &str, invite_token: &str) -> reqwest::Response {
    app.reqwest_client
        .post(app.url(&format!("/api/invitations/{invite_token}/accept")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("accept request failed")
}

/// `GET /api/teams/{team_id}` as `token`.
async fn get_team(app: &common::E2eTestApp, token: &str, team_id: Uuid) -> reqwest::Response {
    app.reqwest_client
        .get(app.url(&format!("/api/teams/{team_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("get_team request failed")
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn invitation_invite_accept_flow(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Owner (the app's built-in token/client) + a distinct invitee.
    let owner_token = app.token.clone();
    let _owner_id = provision(&app, &owner_token).await;
    let invitee_token = common::generate_second_user_jwt();
    let invitee_id = provision(&app, &invitee_token).await;

    // Owner creates a team.
    let team = app
        .client
        .teams()
        .create(&TeamCreateRequest {
            slug: "invite-team".to_owned(),
            name: None,
            parent: None,
            auto_join_role: None,
        })
        .await
        .expect("owner creates team");

    // Owner invites an email at role=member.
    let inv = app
        .client
        .teams()
        .invite(
            team.id,
            &CreateInvitationRequest {
                invited_email: "invitee@example.com".to_owned(),
                role: TeamRole::Member,
            },
        )
        .await
        .expect("owner invites");
    assert_eq!(inv.token.len(), 32, "token is 128-bit hex");

    // Owner lists pending invitations -> exactly the one we created.
    let pending = app
        .client
        .teams()
        .list_invitations(team.id)
        .await
        .expect("list invitations");
    assert_eq!(pending.len(), 1, "one pending invitation");
    assert_eq!(pending[0].invited_email, "invitee@example.com");

    // Invitee accepts (bearer) -> 200 and joins as member.
    let resp = accept(&app, &invitee_token, &inv.token).await;
    assert_eq!(resp.status(), StatusCode::OK, "invitee redeems the token");
    let body: Value = resp.json().await.expect("accept json");
    assert_eq!(
        body["team_id"].as_str().expect("team_id"),
        team.id.to_string()
    );
    assert_eq!(
        body["team_slug"].as_str().expect("team_slug"),
        "invite-team"
    );
    assert_eq!(body["role"].as_str().expect("role"), "member");

    // Team detail (as owner) now lists the invitee as a member.
    let resp = get_team(&app, &owner_token, team.id).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("team detail json");
    let members = body["members"].as_array().expect("members array");
    assert!(
        members.iter().any(
            |m| m["profile_id"].as_str() == Some(&invitee_id.to_string())
                && m["role"].as_str() == Some("member")
        ),
        "invitee is now a member with role member"
    );

    // Pending list is now empty (the invite is accepted, not pending).
    let pending = app
        .client
        .teams()
        .list_invitations(team.id)
        .await
        .expect("list invitations after accept");
    assert!(pending.is_empty(), "no pending invitations after accept");

    // Accept is idempotent — re-redeeming by the same profile still succeeds.
    let resp = accept(&app, &invitee_token, &inv.token).await;
    assert_eq!(resp.status(), StatusCode::OK, "accept is idempotent");

    // An unknown token is a 404.
    let resp = accept(&app, &invitee_token, "deadbeefdeadbeefdeadbeefdeadbeef").await;
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "unknown token is not found"
    );

    // CLI wiring: `temper team invitations <team>` exits 0 (owner-authed).
    let out = common::run_temper_cli(&app, &["team", "invitations", &team.id.to_string()])
        .await
        .expect("cli");
    assert!(
        out.status.success(),
        "team invitations exits 0: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
