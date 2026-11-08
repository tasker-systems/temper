#![cfg(feature = "test-db")]
//! `temper team public-within` end to end: the real binary against the real API and Postgres.
//!
//! A person opens a child team's owned contexts to its parent by slug, finds the opening in
//! `list`, and a manager of the anchor alone (not on the opened team) closes it, naming the opened
//! team by a slug only the anchor's detail resolves.

mod common;

use reqwest::StatusCode;
use serde_json::Value;
use uuid::Uuid;

use temper_core::types::team::{AddMemberRequest, TeamCreateRequest, TeamRole};

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

fn json_out(out: &std::process::Output) -> Value {
    assert!(
        out.status.success(),
        "exits 0: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("stdout is the command's JSON")
}

/// FAILS IF `temper team public-within` stops reaching its doors, or a slug that is not one of the
/// caller's teams stops resolving through the other side's detail.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_person_opens_lists_and_the_anchor_side_closes(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    provision(&app, &app.token).await;
    let anchor_manager_token = common::generate_second_user_jwt();
    let anchor_manager = provision(&app, &anchor_manager_token).await;
    common::approve(&pool, anchor_manager).await;

    let teams = app.client.teams();
    let anchor = teams
        .create(&TeamCreateRequest {
            slug: "cli-pw-anchor".to_owned(),
            ..Default::default()
        })
        .await
        .expect("anchor");
    let team = teams
        .create(&TeamCreateRequest {
            slug: "cli-pw-team".to_owned(),
            parent: Some("+cli-pw-anchor".to_owned()),
            ..Default::default()
        })
        .await
        .expect("team");
    teams
        .add_member(
            anchor.id,
            &AddMemberRequest {
                profile_id: anchor_manager,
                role: TeamRole::Maintainer,
            },
        )
        .await
        .expect("a second manager on the anchor only");

    let opened = json_out(
        &common::run_temper_cli(
            &app,
            &[
                "team",
                "public-within",
                "open",
                "+cli-pw-team",
                "cli-pw-anchor",
            ],
        )
        .await
        .expect("open"),
    );
    assert_eq!(opened["outcome"], "opened", "{opened}");

    let mine = json_out(
        &common::run_temper_cli(&app, &["team", "public-within", "list"])
            .await
            .expect("list"),
    );
    assert!(
        mine.as_array()
            .unwrap()
            .iter()
            .any(|r| r["team"]["id"] == team.id.to_string()
                && r["anchor"]["id"] == anchor.id.to_string()),
        "{mine}"
    );

    // The anchor's manager is not on the opened team, so its slug resolves through the anchor's
    // detail (`public_within_in`, shown to the anchor's managers).
    let config_path = app.vault_dir.path().join("cli-pw-config.toml");
    std::fs::write(&config_path, toml::to_string(&app.config).unwrap()).unwrap();
    let out = common::run_temper_cli_with_token(
        &app.base_url(),
        &anchor_manager_token,
        &config_path,
        &[
            "team",
            "public-within",
            "close",
            "cli-pw-team",
            "cli-pw-anchor",
        ],
    )
    .await
    .expect("close");
    assert!(
        out.status.success(),
        "the anchor side closes alone: {:?} {} {}",
        out.status,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_team_public_within")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "the row is gone");
}
