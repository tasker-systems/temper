#![cfg(feature = "test-db")]
//! Team invitations through MCP: `invitation_read` and `invitation_manage`, end to end.
//!
//! Both tools relay to the existing invitation routes on the caller's own bearer, so what is owed
//! here is the loop as an MCP-only principal walks it: a team's owner invites by email and reviews
//! the team's pending invitations, the invitee counts and lists their own and accepts or declines,
//! and the refusals reach the agent in the caller's own voice rather than as internal faults.
//!
//! Three principals: the harness principal (owner of the team), the second user (the invitee,
//! `second@test.example.com`, never approved — `mine`, `count`, `accept` and `decline` sit on the
//! un-gated router, which is the point), and the third user (an approved plain `member`, who may
//! neither invite nor redeem someone else's invitation).

mod common;

use reqwest::StatusCode;
use rmcp::model::{CallToolResult, ErrorCode};
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use temper_core::types::team::{AddMemberRequest, TeamCreateRequest, TeamRole};
use temper_mcp::service::TemperMcpService;
use temper_mcp::tools::invitations::{invitation_manage, invitation_read};

const TEAM_SLUG: &str = "mcp-invite-team";
const INVITEE_EMAIL: &str = "second@test.example.com";

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

/// Deserialize a tool input from its wire shape.
fn input<T: for<'de> Deserialize<'de>>(value: Value) -> T {
    serde_json::from_value(value).expect("input deserializes from its wire shape")
}

/// The text of a one-part tool result.
fn text(res: &CallToolResult) -> String {
    res.content[0].as_text().expect("a text part").text.clone()
}

/// The JSON of a one-part tool result.
fn one_json(res: &CallToolResult) -> Value {
    serde_json::from_str(&text(res)).expect("the part is the tool's JSON response")
}

struct World {
    app: common::E2eTestApp,
    svc: TemperMcpService,
    owner: axum::http::request::Parts,
    invitee: axum::http::request::Parts,
    member: axum::http::request::Parts,
    invitee_id: Uuid,
    team: Uuid,
}

/// The harness principal owns `TEAM_SLUG`; the third user is an approved `member` of it; the
/// second user is provisioned and nothing more.
async fn world(pool: sqlx::PgPool) -> World {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service().await;

    let owner_token = app.token.clone();
    provision(&app, &owner_token).await;
    let invitee_token = common::generate_second_user_jwt();
    let invitee_id = provision(&app, &invitee_token).await;
    let member_token = common::generate_third_user_jwt();
    let member_id = provision(&app, &member_token).await;
    common::approve(&app.pool, member_id).await;

    let team = app
        .client
        .teams()
        .create(&TeamCreateRequest {
            slug: TEAM_SLUG.to_owned(),
            name: None,
            parent: None,
            auto_join_role: None,
        })
        .await
        .expect("owner creates team")
        .id;
    app.client
        .teams()
        .add_member(
            team,
            &AddMemberRequest {
                profile_id: member_id,
                role: TeamRole::Member,
            },
        )
        .await
        .expect("owner adds the member");

    World {
        owner: app.relay_parts_for(&owner_token),
        invitee: app.relay_parts_for(&invitee_token),
        member: app.relay_parts_for(&member_token),
        app,
        svc,
        invitee_id,
        team,
    }
}

async fn read(w: &World, parts: &axum::http::request::Parts, args: Value) -> Value {
    one_json(
        &invitation_read(&w.svc, parts, input(args))
            .await
            .expect("invitation_read answers"),
    )
}

async fn invite(w: &World) -> Value {
    one_json(
        &invitation_manage(
            &w.svc,
            &w.owner,
            input(json!({
                "action": "create",
                "team": format!("+{TEAM_SLUG}"),
                "email": INVITEE_EMAIL,
                "role": "member",
            })),
        )
        .await
        .expect("the owner invites over MCP"),
    )
}

async fn role_on_team(pool: &sqlx::PgPool, team: Uuid, profile: Uuid) -> Option<String> {
    sqlx::query_scalar(
        "SELECT role::text FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
    )
    .bind(team)
    .bind(profile)
    .fetch_optional(pool)
    .await
    .expect("membership read")
}

/// The whole loop over MCP: the owner invites by slug and sees it in the team's pending list; the
/// invitee, never approved for system access, counts it without a token, lists it with one, and
/// accepts — and is a `member` of the team afterwards, with nothing left waiting.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_invitation_is_created_found_and_accepted_over_mcp(pool: sqlx::PgPool) {
    let w = world(pool).await;

    let created = invite(&w).await;
    assert_eq!(created["invited_email"], INVITEE_EMAIL);
    assert_eq!(created["team_id"], json!(w.team));

    let pending = read(&w, &w.owner, json!({ "view": "team", "team": TEAM_SLUG })).await;
    assert_eq!(pending.as_array().map(Vec::len), Some(1), "{pending}");
    assert_eq!(pending[0]["id"], created["id"]);

    let counts = read(
        &w,
        &w.invitee,
        json!({ "view": "count", "team": TEAM_SLUG }),
    )
    .await;
    assert_eq!(counts, json!({ "count": 1, "matching": 1 }));
    assert!(
        !counts
            .to_string()
            .contains(created["token"].as_str().unwrap()),
        "count carries no token"
    );

    let mine = read(&w, &w.invitee, json!({ "view": "mine" })).await;
    assert_eq!(mine.as_array().map(Vec::len), Some(1), "{mine}");
    assert_eq!(mine[0]["team_slug"], TEAM_SLUG);
    let token = mine[0]["token"]
        .as_str()
        .expect("mine carries the token")
        .to_owned();

    let joined = one_json(
        &invitation_manage(
            &w.svc,
            &w.invitee,
            input(json!({ "action": "accept", "token": token })),
        )
        .await
        .expect("the invitee accepts over MCP"),
    );
    assert_eq!(joined["team_slug"], TEAM_SLUG);
    assert_eq!(joined["role"], "member");
    assert_eq!(
        role_on_team(&w.app.pool, w.team, w.invitee_id)
            .await
            .as_deref(),
        Some("member")
    );

    let after = read(&w, &w.invitee, json!({ "view": "count" })).await;
    assert_eq!(after, json!({ "count": 0, "matching": null }));
}

/// Decline and revoke each take an invitation out of the invitee's list, and neither joins them.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn decline_and_revoke_each_withdraw_an_invitation(pool: sqlx::PgPool) {
    let w = world(pool).await;

    invite(&w).await;
    let mine = read(&w, &w.invitee, json!({ "view": "mine" })).await;
    let token = mine[0]["token"].as_str().unwrap().to_owned();
    let declined = invitation_manage(
        &w.svc,
        &w.invitee,
        input(json!({ "action": "decline", "token": token })),
    )
    .await
    .expect("the invitee declines over MCP");
    assert_eq!(text(&declined), "Invitation declined.");
    assert_eq!(
        read(&w, &w.invitee, json!({ "view": "mine" })).await,
        json!([])
    );

    let second = invite(&w).await;
    let revoked = invitation_manage(
        &w.svc,
        &w.owner,
        input(json!({
            "action": "revoke",
            "team": w.team,
            "invitation_id": second["id"],
        })),
    )
    .await
    .expect("the owner revokes over MCP");
    assert_eq!(text(&revoked), "Invitation revoked.");
    assert_eq!(
        read(&w, &w.invitee, json!({ "view": "mine" })).await,
        json!([])
    );
    assert_eq!(role_on_team(&w.app.pool, w.team, w.invitee_id).await, None);
}

/// The refusals an agent meets, each in the caller's own voice and none an internal fault:
/// a plain member may neither invite nor list the team's invitations (the steward gate, named),
/// may not redeem an invitation addressed to someone else (the server's sentence), and an unknown
/// token is the server's not-found sentence.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_member_is_refused_stewardship_and_someone_elses_invitation(pool: sqlx::PgPool) {
    let w = world(pool).await;
    invite(&w).await;

    let create = invitation_manage(
        &w.svc,
        &w.member,
        input(json!({
            "action": "create",
            "team": TEAM_SLUG,
            "email": "someone@test.example.com",
            "role": "member",
        })),
    )
    .await
    .expect_err("a member may not invite");
    assert_eq!(create.code, ErrorCode::INVALID_PARAMS, "{create:?}");
    assert_eq!(
        create.message,
        "invitation_manage create requires that you manage the team (owner or maintainer)"
    );

    let list = invitation_read(
        &w.svc,
        &w.member,
        input(json!({ "view": "team", "team": TEAM_SLUG })),
    )
    .await
    .expect_err("a member may not list the team's invitations");
    assert_eq!(list.code, ErrorCode::INVALID_PARAMS, "{list:?}");
    assert!(list.message.contains("manage the team"), "{list:?}");

    let mine = read(&w, &w.invitee, json!({ "view": "mine" })).await;
    let token = mine[0]["token"].as_str().unwrap().to_owned();
    let stolen = invitation_manage(
        &w.svc,
        &w.member,
        input(json!({ "action": "accept", "token": token })),
    )
    .await
    .expect_err("someone else's invitation does not redeem");
    assert_eq!(stolen.code, ErrorCode::INVALID_REQUEST, "{stolen:?}");
    assert!(
        stolen.message.contains("addressed to someone else"),
        "{stolen:?}"
    );

    let unknown = invitation_manage(
        &w.svc,
        &w.invitee,
        input(json!({ "action": "accept", "token": "0".repeat(32) })),
    )
    .await
    .expect_err("an unknown token does not redeem");
    assert_eq!(unknown.code, ErrorCode::INVALID_PARAMS, "{unknown:?}");
    assert!(unknown.message.contains("not found"), "{unknown:?}");
}
