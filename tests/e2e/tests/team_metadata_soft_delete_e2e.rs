#![cfg(feature = "test-db")]
//! Team metadata + soft-delete e2e (teams-in-temper scope task #5): drives the real Axum
//! server + Postgres to prove PATCH `/api/teams/{id}` (metadata update) and DELETE
//! `/api/teams/{id}` (soft-delete) end-to-end — owner/maintainer gating on update, owner-only
//! gating on delete, deny-as-absence for a soft-deleted team, and the CLI `team update` /
//! `team delete` round-trip.
//!
//! Modeled on `team_member_lifecycle_test.rs` — same `provision` helper, raw
//! `app.reqwest_client` + Bearer idiom for the second user, and the
//! `#[sqlx::test(migrator = "temper_api::MIGRATOR")]` harness.

mod common;

use reqwest::StatusCode;
use serde_json::Value;
use uuid::Uuid;

use temper_core::types::team::{AddMemberRequest, TeamCreateRequest, TeamRole, TeamUpdateRequest};

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

/// `GET /api/teams/{team_id}` as `token`.
async fn get_team(app: &common::E2eTestApp, token: &str, team_id: Uuid) -> reqwest::Response {
    app.reqwest_client
        .get(app.url(&format!("/api/teams/{team_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("get_team request failed")
}

/// `DELETE /api/teams/{team_id}` as `token` — returns just the status.
async fn delete_team_raw(app: &common::E2eTestApp, token: &str, team_id: Uuid) -> StatusCode {
    app.reqwest_client
        .delete(app.url(&format!("/api/teams/{team_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("delete_team request failed")
        .status()
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn team_metadata_and_soft_delete(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    let owner_token = app.token.clone();
    let _owner_id = provision(&app, &owner_token).await;

    let maintainer_token = common::generate_second_user_jwt();
    let maintainer_id = provision(&app, &maintainer_token).await;

    // Owner creates the team and promotes the second user to maintainer.
    let team = app
        .client
        .teams()
        .create(&TeamCreateRequest {
            slug: "meta-team".to_owned(),
            name: None,
            parent: None,
            auto_join_role: None,
        })
        .await
        .expect("owner creates team");
    app.client
        .teams()
        .add_member(
            team.id,
            &AddMemberRequest {
                profile_id: maintainer_id,
                role: TeamRole::Maintainer,
            },
        )
        .await
        .expect("owner adds maintainer");

    // Owner updates name + description; the row and a follow-up GET reflect it.
    let updated = app
        .client
        .teams()
        .update(
            team.id,
            &TeamUpdateRequest {
                name: Some("Meta Team".to_owned()),
                description: Some("we hold the metadata".to_owned()),
            },
        )
        .await
        .expect("owner updates metadata");
    assert_eq!(updated.name, "Meta Team");
    assert_eq!(updated.description.as_deref(), Some("we hold the metadata"));

    let resp = get_team(&app, &owner_token, team.id).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.expect("team detail json");
    assert_eq!(
        body["description"].as_str(),
        Some("we hold the metadata"),
        "GET reflects the updated description"
    );

    // A maintainer may manage membership but NOT dissolve the team -> 403.
    let status = delete_team_raw(&app, &maintainer_token, team.id).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "a maintainer cannot soft-delete the team"
    );

    // Owner soft-deletes -> 204; the team then reads as absent (404) even to the owner.
    let status = delete_team_raw(&app, &owner_token, team.id).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "owner soft-deletes the team"
    );
    let resp = get_team(&app, &owner_token, team.id).await;
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "a soft-deleted team reads as absent"
    );

    // CLI wiring: `temper team update` then `temper team delete` on a fresh team both exit 0,
    // and the team is gone afterward.
    let team2 = app
        .client
        .teams()
        .create(&TeamCreateRequest {
            slug: "cli-team".to_owned(),
            name: None,
            parent: None,
            auto_join_role: None,
        })
        .await
        .expect("owner creates cli team");

    let out = common::run_temper_cli(
        &app,
        &[
            "team",
            "update",
            "cli-team",
            "--name",
            "CLI Team",
            "--description",
            "driven by the cli",
        ],
    )
    .await
    .expect("cli update");
    assert!(
        out.status.success(),
        "team update exits 0: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let out = common::run_temper_cli(&app, &["team", "delete", "cli-team"])
        .await
        .expect("cli delete");
    assert!(
        out.status.success(),
        "team delete exits 0: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let resp = get_team(&app, &owner_token, team2.id).await;
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "cli-deleted team reads as absent"
    );
}
