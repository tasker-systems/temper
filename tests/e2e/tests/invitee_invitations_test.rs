#![cfg(feature = "test-db")]
//! Invitee-side invitation resolution e2e: an invited email, once its profile is
//! provisioned, sees the pending invitation via `GET /api/invitations/mine`.
//!
//! Owner-authed writes go through `TemperClient` (`app.client`, bound to the owner
//! token); the invitee's own-invitation read goes through raw `reqwest` with the
//! invitee's Bearer (a different principal than `app.client`). Modeled on
//! `team_invitations_test.rs` — same `provision` helper and `#[sqlx::test(migrator
//! = "temper_api::MIGRATOR")]` harness.
//!
//! The invitee JWT (`generate_second_user_jwt`) carries email
//! `second@test.example.com`; provisioning deposits that on the invitee's auth
//! link, which is what the resolver matches `invited_email` against.

mod common;

use reqwest::StatusCode;
use uuid::Uuid;

use temper_core::types::invitation::{CreateInvitationRequest, InviteeInvitation};
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
    let body: serde_json::Value = resp.json().await.expect("json");
    body["id"].as_str().expect("id").parse().expect("uuid")
}

/// `GET /api/invitations/mine` as `token`, deserialized into the typed rows.
async fn list_mine(app: &common::E2eTestApp, token: &str) -> Vec<InviteeInvitation> {
    let resp = app
        .reqwest_client
        .get(app.url("/api/invitations/mine"))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("list mine request failed");
    assert_eq!(resp.status(), StatusCode::OK, "list mine is 200");
    resp.json().await.expect("invitee invitation json")
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn invitee_sees_own_pending_invitation(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Owner (the app's built-in token/client) + a distinct invitee.
    let owner_token = app.token.clone();
    let _owner_id = provision(&app, &owner_token).await;
    let invitee_token = common::generate_second_user_jwt(); // email second@test.example.com
    let _invitee_id = provision(&app, &invitee_token).await;

    // Owner creates a team and invites the invitee's email.
    let team = app
        .client
        .teams()
        .create(&TeamCreateRequest {
            slug: "invitee-list-team".to_owned(),
            name: None,
            parent: None,
            auto_join_role: None,
        })
        .await
        .expect("owner creates team");
    app.client
        .teams()
        .invite(
            team.id,
            &CreateInvitationRequest {
                invited_email: "second@test.example.com".to_owned(),
                role: TeamRole::Member,
            },
        )
        .await
        .expect("owner invites the invitee email");

    // Invitee lists their OWN pending invitations -> sees exactly this one,
    // token included (the listing is the self-serve delivery).
    let mine = list_mine(&app, &invitee_token).await;
    assert_eq!(mine.len(), 1, "invitee sees their pending invitation");
    assert_eq!(mine[0].team_slug, "invitee-list-team");
    assert_eq!(mine[0].invited_email, "second@test.example.com");
    assert!(
        !mine[0].token.is_empty(),
        "token is self-served for redemption"
    );

    // The inviter (owner) has no invitations addressed to them. Exercises the
    // typed client method end-to-end (plumbing + deserialization).
    let owner_mine = app
        .client
        .teams()
        .list_my_invitations()
        .await
        .expect("owner lists own invitations");
    assert!(
        owner_mine.is_empty(),
        "inviter has no invitations of their own"
    );

    // CLI wiring: `temper invitations` exits 0 (runs as owner -> empty list).
    let out = common::run_temper_cli(&app, &["invitations"])
        .await
        .expect("cli");
    assert!(
        out.status.success(),
        "temper invitations exits 0: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
