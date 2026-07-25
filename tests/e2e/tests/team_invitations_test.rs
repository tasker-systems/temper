#![cfg(feature = "test-db")]
//! Team invitations e2e: drives the real Axum server + Postgres through the full
//! invite → accept flow across two distinct profiles.
//!
//! Owner-authed writes go through `TemperClient` (`app.client`, bound to the
//! owner token). The invitee is a different principal than `app.client`, and is
//! covered at two levels deliberately: `invitation_invite_accept_flow` redeems
//! with raw `reqwest`, pinning the **wire** shape independently of the client,
//! while `client_accept_and_decline_drive_the_body_carrying_routes` goes through
//! a second `TemperClient` — the path `temper team join` actually takes. Neither
//! subsumes the other: the raw form would keep passing if the client serialized
//! the token into the wrong place, and the client form would keep passing if both
//! ends drifted together. Modeled on `team_member_lifecycle_test.rs` — same
//! `provision` helper, same `#[sqlx::test(migrator = "temper_api::MIGRATOR")]`
//! harness.
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

/// `POST /api/invitations/accept` as `token`, with the invite token in the body.
async fn accept(app: &common::E2eTestApp, token: &str, invite_token: &str) -> reqwest::Response {
    app.reqwest_client
        .post(app.url("/api/invitations/accept"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&serde_json::json!({ "token": invite_token }))
        .send()
        .await
        .expect("accept request failed")
}

/// A `TemperClient` bound to an arbitrary principal's token.
///
/// `app.client` is the owner's. The invitee is a different principal, and the CLI's real path into
/// accept/decline is `TeamsClient`, not raw `reqwest` — so covering the invitee's side at the
/// production caller's level needs a second client.
fn client_for(app: &common::E2eTestApp, token: &str) -> temper_client::TemperClient {
    use temper_client::auth::{MemoryTokenStore, Provider, StoredAuth};

    let stored_auth = StoredAuth {
        provider: Provider::Auth0 {
            domain: "test".to_string(),
        },
        access_token: token.to_string().into(),
        refresh_token: None,
        expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        profile_id: None,
        device_id: Some("e2e-test-device".to_string()),
    };
    let store: std::sync::Arc<dyn temper_client::auth::TokenStore> =
        std::sync::Arc::new(MemoryTokenStore::with_auth(stored_auth));
    temper_client::config::build_client_from(
        &app.config,
        store,
        temper_workflow::operations::Surface::CliCloud,
    )
    .expect("build invitee client")
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

/// The invitee's accept and decline, driven through `TeamsClient` — the path the CLI actually takes.
///
/// The flow test above redeems with raw `reqwest`, which asserts the *wire* shape. That leaves the
/// client methods themselves uncovered, and they are what `temper team join` / `temper team decline`
/// call: a wrong body field name there compiles, passes every other test, and breaks only in a user's
/// hands. `decline_invitation` had no e2e coverage of any kind before this.
///
/// Both routes now carry the token in the request body rather than the path — see
/// `InvitationTokenRequest`. Serializing it into the wrong place is exactly the regression this
/// catches, because the server reads only the body.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn client_accept_and_decline_drive_the_body_carrying_routes(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let owner_token = app.token.clone();
    let _owner_id = provision(&app, &owner_token).await;
    let invitee_token = common::generate_second_user_jwt();
    let invitee_id = provision(&app, &invitee_token).await;
    let invitee = client_for(&app, &invitee_token);

    let team = app
        .client
        .teams()
        .create(&TeamCreateRequest {
            slug: "client-invite-team".to_owned(),
            name: None,
            parent: None,
            auto_join_role: None,
        })
        .await
        .expect("owner creates team");

    // Decline first, so the invitee is still a non-member when it runs.
    let to_decline = app
        .client
        .teams()
        .invite(
            team.id,
            &CreateInvitationRequest {
                invited_email: "declines@example.com".to_owned(),
                role: TeamRole::Member,
            },
        )
        .await
        .expect("owner invites the decliner");

    invitee
        .teams()
        .decline_invitation(&to_decline.token)
        .await
        .expect("client declines through the body-carrying route");

    let resp = get_team(&app, &owner_token, team.id).await;
    let body: Value = resp.json().await.expect("team detail json");
    assert!(
        !body["members"]
            .as_array()
            .expect("members array")
            .iter()
            .any(|m| m["profile_id"].as_str() == Some(&invitee_id.to_string())),
        "declining must not join the team"
    );

    // Now accept, and assert the response the client deserializes.
    let to_accept = app
        .client
        .teams()
        .invite(
            team.id,
            &CreateInvitationRequest {
                invited_email: "accepts@example.com".to_owned(),
                role: TeamRole::Member,
            },
        )
        .await
        .expect("owner invites the accepter");

    let accepted = invitee
        .teams()
        .accept_invitation(&to_accept.token)
        .await
        .expect("client accepts through the body-carrying route");
    assert_eq!(accepted.team_id, team.id);
    assert_eq!(accepted.team_slug, "client-invite-team");
    assert_eq!(accepted.role, TeamRole::Member);

    let resp = get_team(&app, &owner_token, team.id).await;
    let body: Value = resp.json().await.expect("team detail json");
    assert!(
        body["members"]
            .as_array()
            .expect("members array")
            .iter()
            .any(
                |m| m["profile_id"].as_str() == Some(&invitee_id.to_string())
                    && m["role"].as_str() == Some("member")
            ),
        "accepting through the client joins the team"
    );

    // An unknown token still errors, so a silently-dropped body cannot read as success.
    let err = invitee
        .teams()
        .accept_invitation("deadbeefdeadbeefdeadbeefdeadbeef")
        .await;
    assert!(err.is_err(), "unknown token must not succeed");
}
