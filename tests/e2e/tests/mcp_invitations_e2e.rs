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
use temper_mcp::tools::invitations::{invitation_manage, invitation_read, InvitationManageInput};

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

async fn has_system_access(pool: &sqlx::PgPool, profile: Uuid) -> bool {
    sqlx::query_scalar("SELECT has_system_access($1)")
        .bind(profile)
        .fetch_one(pool)
        .await
        .expect("has_system_access")
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
/// accepts — and is a `member` of the team afterwards, with nothing left waiting, and still
/// without system access: joining a team is not the access decision.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_invitation_is_created_found_and_accepted_over_mcp(pool: sqlx::PgPool) {
    let w = world(pool).await;
    assert!(
        !has_system_access(&w.app.pool, w.invitee_id).await,
        "the invitee walks the un-gated loop unapproved"
    );

    let created = invite(&w).await;
    assert_eq!(created["invited_email"], INVITEE_EMAIL);
    assert_eq!(created["team_id"], json!(w.team));
    assert!(
        created.get("token").is_none(),
        "create hands the steward no token: {created}"
    );

    let pending = read(&w, &w.owner, json!({ "view": "team", "team": TEAM_SLUG })).await;
    assert_eq!(pending.as_array().map(Vec::len), Some(1), "{pending}");
    assert_eq!(pending[0]["id"], created["id"]);
    assert!(
        pending[0].get("token").is_none(),
        "the team view carries no token: {pending}"
    );

    let counts = read(
        &w,
        &w.invitee,
        json!({ "view": "count", "team": TEAM_SLUG }),
    )
    .await;
    assert_eq!(counts, json!({ "count": 1, "matching": 1 }));

    let mine = read(&w, &w.invitee, json!({ "view": "mine" })).await;
    assert_eq!(mine.as_array().map(Vec::len), Some(1), "{mine}");
    assert_eq!(mine[0]["team_slug"], TEAM_SLUG);
    let token = mine[0]["token"]
        .as_str()
        .expect("mine carries the token")
        .to_owned();
    assert!(
        !counts.to_string().contains(&token),
        "count carries no token"
    );

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
    assert!(
        !has_system_access(&w.app.pool, w.invitee_id).await,
        "accepting an invitation does not grant system access"
    );
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
    assert_eq!(one_json(&declined), json!({ "pending": false }));
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
    assert_eq!(
        one_json(&revoked),
        json!({ "revoked": true, "team_id": w.team, "invitation_id": second["id"] })
    );
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

/// A machine principal neither answers nor issues invitations: it holds no verified email, so it is
/// never an invitee, and it does not steward a team even where it holds a seat. Every view and
/// action that reaches an invitation route is refused with the API's machine sentence, carried as
/// the caller's own refusal (`INVALID_REQUEST`), never as an internal fault.
///
/// FAILS IF `map_invitation_err` loses its detailed-403 arm, or an invitation route admits a machine.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_machine_is_refused_every_invitation_act(pool: sqlx::PgPool) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service().await;
    let m = common::register_team_machine(&app, "mcp-invitations-agent", "member").await;
    let parts = app.relay_parts_for(&m.token);
    let refusal = temper_services::auth::MACHINE_PRINCIPAL_REFUSAL;

    let reads = [
        json!({ "view": "mine" }),
        json!({ "view": "count" }),
        json!({ "view": "team", "team": m.team }),
    ];
    for args in reads {
        let err = invitation_read(&svc, &parts, input(args.clone()))
            .await
            .expect_err("a machine is refused");
        assert_eq!(err.code, ErrorCode::INVALID_REQUEST, "{args}: {err:?}");
        assert!(err.message.ends_with(refusal), "{args}: {err:?}");
    }

    let writes = [
        json!({ "action": "accept", "token": "0".repeat(32) }),
        json!({ "action": "decline", "token": "0".repeat(32) }),
        json!({
            "action": "create",
            "team": m.team,
            "email": "someone@test.example.com",
            "role": "member",
        }),
        json!({ "action": "revoke", "team": m.team, "invitation_id": Uuid::now_v7() }),
    ];
    for args in writes {
        let err = invitation_manage(&svc, &parts, input(args.clone()))
            .await
            .expect_err("a machine is refused");
        assert_eq!(err.code, ErrorCode::INVALID_REQUEST, "{args}: {err:?}");
        assert!(err.message.ends_with(refusal), "{args}: {err:?}");
    }

    let invitations: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_team_invitations")
        .fetch_one(&app.pool)
        .await
        .expect("count");
    assert_eq!(invitations, 0, "no invitation was created");
}

/// The steward's edges, each answered in the caller's own voice: `owner` is not an invitable role
/// (the schema does not offer it), a missing field and a duplicate invite are invalid params, a
/// UUID names the team in the `team` view as a slug does, a slug the caller is not a member of is
/// "not a team", an unknown invitation id does not revoke, and `count` takes only a bare slug and
/// answers `matching: 0` for a team holding nothing for the caller.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn steward_and_count_edges_answer_in_the_callers_voice(pool: sqlx::PgPool) {
    let w = world(pool).await;

    let owner_role = serde_json::from_value::<InvitationManageInput>(json!({
        "action": "create", "team": TEAM_SLUG, "email": INVITEE_EMAIL, "role": "owner",
    }));
    assert!(owner_role.is_err(), "owner is not an invitable role");

    let missing = invitation_manage(
        &w.svc,
        &w.owner,
        input(json!({ "action": "create", "team": TEAM_SLUG, "role": "member" })),
    )
    .await
    .expect_err("create needs an email");
    assert_eq!(missing.code, ErrorCode::INVALID_PARAMS, "{missing:?}");
    assert_eq!(missing.message, "invitation_manage create requires `email`");

    invite(&w).await;
    let duplicate = invitation_manage(
        &w.svc,
        &w.owner,
        input(json!({
            "action": "create", "team": TEAM_SLUG, "email": INVITEE_EMAIL, "role": "member",
        })),
    )
    .await
    .expect_err("a second pending invite for the same address conflicts");
    assert_eq!(duplicate.code, ErrorCode::INVALID_PARAMS, "{duplicate:?}");
    assert_eq!(
        duplicate.message,
        "a pending invitation already exists for this email"
    );

    let by_uuid = read(&w, &w.owner, json!({ "view": "team", "team": w.team })).await;
    assert_eq!(by_uuid.as_array().map(Vec::len), Some(1), "{by_uuid}");

    let stranger = invitation_read(
        &w.svc,
        &w.owner,
        input(json!({ "view": "team", "team": "no-such-team" })),
    )
    .await
    .expect_err("a slug the caller is not a member of names no team");
    assert_eq!(stranger.code, ErrorCode::INVALID_PARAMS, "{stranger:?}");
    assert!(stranger.message.starts_with("not a team"), "{stranger:?}");

    let unknown = invitation_manage(
        &w.svc,
        &w.owner,
        input(json!({ "action": "revoke", "team": w.team, "invitation_id": Uuid::now_v7() })),
    )
    .await
    .expect_err("an unknown invitation does not revoke");
    assert_eq!(unknown.code, ErrorCode::INVALID_PARAMS, "{unknown:?}");

    let elsewhere = read(
        &w,
        &w.invitee,
        json!({ "view": "count", "team": "some-other-team" }),
    )
    .await;
    assert_eq!(elsewhere, json!({ "count": 1, "matching": 0 }));

    let decorated = invitation_read(
        &w.svc,
        &w.invitee,
        input(json!({ "view": "count", "team": format!("{TEAM_SLUG}-{}", w.team) })),
    )
    .await
    .expect_err("count takes only a bare slug");
    assert_eq!(decorated.code, ErrorCode::INVALID_PARAMS, "{decorated:?}");
}
