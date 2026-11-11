#![cfg(feature = "test-db")]
//! Team management over MCP (temper-artifacts `specs/2026-11-06-team-dag-and-lifecycle-doors-design.md`,
//! ruling R7 reversed): `team_manage` carries a person's team acts and `team_read` the reads, both
//! relaying on the caller's own bearer.
//!
//! What is owed here is the route boundary, end to end: a person's act lands, and a machine
//! principal's is refused by the API — `AuthUser(HumanPrincipal)` at the route — with the API's
//! sentence reaching the agent verbatim. The MCP crate holds no machine check of its own, so this is
//! the test that fails if a team route ever admits a machine.

mod common;

use rmcp::model::{CallToolResult, ErrorCode};
use serde::Deserialize;
use serde_json::{json, Value};
use temper_client::error::ClientError;
use uuid::Uuid;

use temper_mcp::tools::teams::{team_manage, team_read};

fn input<T: for<'de> Deserialize<'de>>(value: Value) -> T {
    serde_json::from_value(value).expect("input deserializes from its wire shape")
}

fn one_json(res: &CallToolResult) -> Value {
    let text = res.content[0].as_text().expect("a text part").text.clone();
    serde_json::from_str(&text).expect("the part is the tool's JSON response")
}

/// A person creates two teams, attaches one under the other, and reads the child's parents — all
/// over MCP.
///
/// FAILS IF a `team_manage` action stops relaying, or `team_read` detail loses `parents`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_person_builds_a_tree_over_mcp(pool: sqlx::PgPool) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service().await;
    let parts = app.relay_parts_for(&app.token);

    let mut ids = Vec::new();
    for slug in ["mcp-tree-parent", "mcp-tree-child"] {
        let made = one_json(
            &team_manage(
                &svc,
                &parts,
                input(json!({ "action": "create", "slug": slug })),
            )
            .await
            .expect("a person creates a team"),
        );
        ids.push(made["id"].as_str().expect("id").parse::<Uuid>().unwrap());
    }
    let (parent, child) = (ids[0], ids[1]);

    let linked = one_json(
        &team_manage(
            &svc,
            &parts,
            input(json!({ "action": "attach", "team": parent, "child": "+mcp-tree-child" })),
        )
        .await
        .expect("a manager of both attaches"),
    );
    assert_eq!(linked["outcome"], "linked");

    let detail = one_json(
        &team_read(
            &svc,
            &parts,
            input(json!({ "view": "detail", "team": child })),
        )
        .await
        .expect("detail"),
    );
    assert_eq!(detail["parents"][0]["id"], parent.to_string());

    let mine = one_json(
        &team_read(&svc, &parts, input(json!({ "view": "list" })))
            .await
            .expect("list"),
    );
    assert!(mine
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["slug"] == "mcp-tree-child"));
}

/// A person opens a child team's owned contexts to its parent, finds the opening in their discovery
/// list and on the child's detail, and closes it, all over MCP. The anchor is named by slug.
///
/// FAILS IF a public_within action or the `public_within` view stops relaying, or the detail drops
/// the row for the team's own member.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_person_opens_and_closes_public_within_over_mcp(pool: sqlx::PgPool) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service().await;
    let parts = app.relay_parts_for(&app.token);

    let mut ids = Vec::new();
    for (slug, parents) in [
        ("mcp-pw-anchor", json!([])),
        ("mcp-pw-team", json!(["+mcp-pw-anchor"])),
    ] {
        let made = one_json(
            &team_manage(
                &svc,
                &parts,
                input(json!({ "action": "create", "slug": slug, "parents": parents })),
            )
            .await
            .expect("a person creates a team"),
        );
        ids.push(made["id"].as_str().expect("id").parse::<Uuid>().unwrap());
    }
    let (anchor, team) = (ids[0], ids[1]);

    let opened = one_json(
        &team_manage(
            &svc,
            &parts,
            input(json!({
                "action": "public_within_open", "team": "+mcp-pw-team", "anchor": "mcp-pw-anchor",
            })),
        )
        .await
        .expect("a manager of both sides opens"),
    );
    assert_eq!(opened["outcome"], "opened", "{opened}");
    assert_eq!(opened["instance_wide"], false, "{opened}");
    assert!(
        opened["audience"]["principals"].as_i64().unwrap() >= 1,
        "{opened}"
    );

    let mine = one_json(
        &team_read(&svc, &parts, input(json!({ "view": "public_within" })))
            .await
            .expect("the discovery view"),
    );
    assert!(
        mine.as_array().unwrap().iter().any(|r| r["team"]["id"]
            == team.to_string()
            && r["anchor"]["id"] == anchor.to_string()),
        "the opening confers on a person under the anchor: {mine}"
    );

    let detail = one_json(
        &team_read(
            &svc,
            &parts,
            input(json!({ "view": "detail", "team": team })),
        )
        .await
        .expect("detail"),
    );
    let rows = detail["public_within"].as_array().expect("public_within");
    assert_eq!(rows.len(), 1, "{detail}");
    assert_eq!(rows[0]["anchor"]["id"], anchor.to_string());
    assert_eq!(rows[0]["effective"], true);

    let closed = one_json(
        &team_manage(
            &svc,
            &parts,
            input(json!({ "action": "public_within_close", "team": team, "anchor": anchor })),
        )
        .await
        .expect("either side closes"),
    );
    assert_eq!(closed["closed"], true);

    let mine = one_json(
        &team_read(&svc, &parts, input(json!({ "view": "public_within" })))
            .await
            .expect("the discovery view"),
    );
    assert!(
        !mine
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["team"]["id"] == team.to_string()),
        "a closed row confers on no one: {mine}"
    );
}

/// A machine principal's team acts are refused by the API, and its sentence reaches the agent as the
/// caller's own refusal (`INVALID_REQUEST`), never as an internal fault. Nothing is written.
///
/// FAILS IF a team route admits a machine principal, or `map_team_err` loses its detailed-403 arm.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_machine_is_refused_every_team_act(pool: sqlx::PgPool) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service().await;
    let m = common::register_team_machine(&app, "mcp-team-manage-agent", "member").await;
    let parts = app.relay_parts_for(&m.token);
    let refusal = temper_services::auth::MACHINE_PRINCIPAL_REFUSAL;

    let acts = [
        json!({ "action": "create", "slug": "mcp-machine-made" }),
        json!({ "action": "update", "team": m.team, "name": "renamed" }),
        json!({ "action": "attach", "team": m.team, "child": m.team }),
        json!({ "action": "rename_slug", "team": m.team, "slug": "mcp-machine-slug" }),
        json!({ "action": "restore", "team": m.team }),
        json!({ "action": "set_auto_join", "team": m.team, "auto_join_role": "watcher" }),
        json!({ "action": "reparent", "team": m.team, "from": "+a", "to": "+b" }),
        json!({ "action": "public_within_open", "team": m.team, "anchor": m.team }),
        json!({ "action": "public_within_close", "team": m.team, "anchor": m.team }),
        json!({ "action": "public_within_policy", "team": m.team, "forbidden": true }),
        json!({ "action": "public_within_policy", "team": m.team, "forbidden": false }),
    ];
    for args in acts {
        let err = team_manage(&svc, &parts, input(args.clone()))
            .await
            .expect_err("a machine is refused");
        assert_eq!(err.code, ErrorCode::INVALID_REQUEST, "{args}: {err:?}");
        assert!(err.message.ends_with(refusal), "{args}: {err:?}");
    }

    // A machine never reads through public_within, so its discovery read is refused too.
    let err = team_read(&svc, &parts, input(json!({ "view": "public_within" })))
        .await
        .expect_err("a machine's discovery read is refused");
    assert_eq!(err.code, ErrorCode::INVALID_REQUEST, "{err:?}");
    assert!(err.message.ends_with(refusal), "{err:?}");

    let made: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_teams WHERE slug LIKE 'mcp-machine-%'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(made, 0, "nothing was written");
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM kb_team_public_within")
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert_eq!(rows, 0, "no opening was written");
    let forbidden: bool =
        sqlx::query_scalar("SELECT public_within_forbidden FROM kb_teams WHERE id = $1")
            .bind(m.team)
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert!(!forbidden, "no forbid was written");
}

// ── A person's path for every other act ─────────────────────────────────────────────────────────
//
// Each test below relays an act on the caller's own bearer, then reads the effect back through
// `team_read` (or the database where no read names it). Where a refusal is cheap, the same act by
// someone without the authority is refused over MCP with the refusal HTTP gives that bearer.

/// The harness principal's profile id.
async fn me(app: &common::E2eTestApp) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_profiles WHERE email = $1")
        .bind("e2e@test.example.com")
        .fetch_one(&app.pool)
        .await
        .expect("harness principal")
}

/// Create a team over MCP as `parts`' bearer and return its id.
async fn create(
    svc: &temper_mcp::service::TemperMcpService,
    parts: &http::request::Parts,
    args: Value,
) -> Uuid {
    let made = one_json(
        &team_manage(svc, parts, input(args))
            .await
            .expect("a person creates a team"),
    );
    made["id"].as_str().expect("id").parse().unwrap()
}

async fn detail(
    svc: &temper_mcp::service::TemperMcpService,
    parts: &http::request::Parts,
    team: Uuid,
) -> Value {
    one_json(
        &team_read(svc, parts, input(json!({ "view": "detail", "team": team })))
            .await
            .expect("detail"),
    )
}

async fn my_slugs(
    svc: &temper_mcp::service::TemperMcpService,
    parts: &http::request::Parts,
) -> Vec<String> {
    one_json(
        &team_read(svc, parts, input(json!({ "view": "list" })))
            .await
            .expect("list"),
    )
    .as_array()
    .unwrap()
    .iter()
    .map(|t| t["slug"].as_str().unwrap().to_string())
    .collect()
}

/// The role `profile` holds on a team's roster, if any.
fn role_of(detail: &Value, profile: Uuid) -> Option<String> {
    detail["members"]
        .as_array()
        .expect("members")
        .iter()
        .find(|m| m["profile_id"] == profile.to_string())
        .map(|m| m["role"].as_str().unwrap().to_string())
}

/// The MCP refusal is the one HTTP gave the same bearer for the same act: a bare 403 becomes the
/// tool's own "was refused" sentence, and every other refusal carries the API's sentence.
fn assert_same_refusal(act: &str, mcp: &rmcp::ErrorData, http: ClientError) {
    match http {
        ClientError::Forbidden => {
            assert_eq!(mcp.code, ErrorCode::INVALID_PARAMS, "{act}: {mcp:?}");
            assert!(
                mcp.message.starts_with(&format!("{act} was refused")),
                "{act}: {mcp:?}"
            );
        }
        ClientError::ForbiddenDetail { message } => {
            assert_eq!(mcp.code, ErrorCode::INVALID_REQUEST, "{act}: {mcp:?}");
            assert!(
                mcp.message.ends_with(&message),
                "{act}: {mcp:?} vs {message}"
            );
        }
        ClientError::NotFound { message } => {
            assert_eq!(mcp.code, ErrorCode::INVALID_PARAMS, "{act}: {mcp:?}");
            assert_eq!(mcp.message, message, "{act}");
        }
        ClientError::Conflict { message }
        | ClientError::Server {
            status: 400,
            message,
        } => {
            // The tool passes the API's cause through without its status label.
            let cause = message
                .split_once(": ")
                .map_or(message.as_str(), |(_, cause)| cause);
            assert_eq!(mcp.code, ErrorCode::INVALID_PARAMS, "{act}: {mcp:?}");
            assert_eq!(mcp.message, cause, "{act}: HTTP said {message:?}");
        }
        other => panic!("{act}: HTTP gave an unexpected refusal {other:?}; MCP gave {mcp:?}"),
    }
}

/// A person renames a team's display text, soft-deletes it, and restores it, all over MCP; a
/// stranger's update is refused as HTTP refuses it.
///
/// FAILS IF `update`, `delete` or `restore` stops relaying to its route, or `update` drops the
/// description.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_person_updates_deletes_and_restores_a_team_over_mcp(pool: sqlx::PgPool) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service().await;
    let parts = app.relay_parts_for(&app.token);
    let team = create(
        &svc,
        &parts,
        json!({ "action": "create", "slug": "mcp-life" }),
    )
    .await;

    team_manage(
        &svc,
        &parts,
        input(json!({
            "action": "update", "team": "+mcp-life", "name": "Life", "description": "A team's life",
        })),
    )
    .await
    .expect("its owner updates it");
    let read = detail(&svc, &parts, team).await;
    assert_eq!(read["name"], "Life", "{read}");
    assert_eq!(read["description"], "A team's life", "{read}");

    // A stranger's update: refused over MCP exactly as over HTTP, and nothing changes.
    common::provision_and_approve_second(&app).await;
    let second = common::generate_second_user_jwt();
    let mcp = team_manage(
        &svc,
        &app.relay_parts_for(&second),
        input(json!({ "action": "update", "team": team, "name": "Hijacked" })),
    )
    .await
    .expect_err("a stranger is refused");
    let http = app
        .client_for(&second)
        .teams()
        .update(
            team,
            &temper_core::types::team::TeamUpdateRequest {
                name: Some("Hijacked".into()),
                description: None,
            },
        )
        .await
        .expect_err("HTTP refuses the stranger");
    assert_same_refusal("team_manage update", &mcp, http);
    assert_eq!(detail(&svc, &parts, team).await["name"], "Life");

    let deleted = one_json(
        &team_manage(
            &svc,
            &parts,
            input(json!({ "action": "delete", "team": team })),
        )
        .await
        .expect("its owner deletes it"),
    );
    assert_eq!(deleted["deleted"], true, "{deleted}");
    assert!(!my_slugs(&svc, &parts)
        .await
        .contains(&"mcp-life".to_string()));
    let gone: bool = sqlx::query_scalar("SELECT NOT is_active FROM kb_teams WHERE id = $1")
        .bind(team)
        .fetch_one(&app.pool)
        .await
        .unwrap();
    assert!(gone, "the team is soft-deleted");

    team_manage(
        &svc,
        &parts,
        input(json!({ "action": "restore", "team": team })),
    )
    .await
    .expect("its owner restores it by UUID");
    assert!(my_slugs(&svc, &parts)
        .await
        .contains(&"mcp-life".to_string()));
    assert_eq!(detail(&svc, &parts, team).await["name"], "Life");
}

/// A person adds a member, changes their role and removes them; the member then rejoins and leaves
/// on their own bearer. A watcher's add is refused, and the last owner's leave is refused, each as
/// HTTP refuses it.
///
/// FAILS IF `add_member`, `change_role`, `remove_member` or `leave` stops relaying, sends the wrong
/// role, or `leave` removes someone other than the caller.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_person_manages_a_roster_over_mcp(pool: sqlx::PgPool) {
    use temper_core::types::team::{AddMemberRequest, TeamRole};

    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service().await;
    let parts = app.relay_parts_for(&app.token);
    let owner = me(&app).await;
    let other = common::provision_and_approve_second(&app).await;
    let second = common::generate_second_user_jwt();
    let second_parts = app.relay_parts_for(&second);
    let team = create(
        &svc,
        &parts,
        json!({ "action": "create", "slug": "mcp-roster" }),
    )
    .await;

    team_manage(
        &svc,
        &parts,
        input(json!({
            "action": "add_member", "team": "+mcp-roster", "profile_id": other, "role": "maintainer",
        })),
    )
    .await
    .expect("its owner adds a member");
    assert_eq!(
        role_of(&detail(&svc, &parts, team).await, other).as_deref(),
        Some("maintainer")
    );

    team_manage(
        &svc,
        &parts,
        input(json!({
            "action": "change_role", "team": team, "profile_id": other, "role": "watcher",
        })),
    )
    .await
    .expect("its owner changes the role");
    assert_eq!(
        role_of(&detail(&svc, &parts, team).await, other).as_deref(),
        Some("watcher")
    );

    // A watcher adding a member (here, re-adding the owner at a lesser role): refused as HTTP
    // refuses it, and the roster is unchanged.
    let mcp = team_manage(
        &svc,
        &second_parts,
        input(json!({
            "action": "add_member", "team": team, "profile_id": owner, "role": "watcher",
        })),
    )
    .await
    .expect_err("a watcher does not manage the roster");
    let http = app
        .client_for(&second)
        .teams()
        .add_member(
            team,
            &AddMemberRequest {
                profile_id: owner,
                role: TeamRole::Watcher,
            },
        )
        .await
        .expect_err("HTTP refuses the watcher");
    assert_same_refusal("team_manage add_member", &mcp, http);
    assert_eq!(
        role_of(&detail(&svc, &parts, team).await, owner).as_deref(),
        Some("owner")
    );

    team_manage(
        &svc,
        &parts,
        input(json!({ "action": "remove_member", "team": team, "profile_id": other })),
    )
    .await
    .expect("its owner removes the member");
    assert_eq!(role_of(&detail(&svc, &parts, team).await, other), None);

    // The member rejoins and leaves on their own bearer, naming the team by slug.
    team_manage(
        &svc,
        &parts,
        input(json!({
            "action": "add_member", "team": team, "profile_id": other, "role": "member",
        })),
    )
    .await
    .expect("its owner re-adds the member");
    assert_eq!(
        role_of(&detail(&svc, &parts, team).await, other).as_deref(),
        Some("member")
    );
    let left = one_json(
        &team_manage(
            &svc,
            &second_parts,
            input(json!({ "action": "leave", "team": "+mcp-roster" })),
        )
        .await
        .expect("a member leaves"),
    );
    assert_eq!(left["left"], true, "{left}");
    let read = detail(&svc, &parts, team).await;
    assert_eq!(role_of(&read, other), None, "the leaver is off: {read}");
    assert_eq!(role_of(&read, owner).as_deref(), Some("owner"), "{read}");

    // The last owner's leave: refused as HTTP refuses it, and they stay.
    let mcp = team_manage(
        &svc,
        &parts,
        input(json!({ "action": "leave", "team": team })),
    )
    .await
    .expect_err("the last owner cannot leave");
    let http = app
        .client
        .teams()
        .remove_member(team, owner)
        .await
        .expect_err("HTTP refuses the last owner");
    assert_same_refusal("team_manage leave", &mcp, http);
    assert_eq!(
        role_of(&detail(&svc, &parts, team).await, owner).as_deref(),
        Some("owner")
    );
}

/// A person moves a team from one parent to another, detaches it, and renames its slug, all over
/// MCP. A stranger's rename is refused as HTTP refuses it.
///
/// FAILS IF `reparent`, `detach` or `rename_slug` stops relaying, or `reparent` swaps `from` and
/// `to`.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_person_reshapes_a_tree_over_mcp(pool: sqlx::PgPool) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service().await;
    let parts = app.relay_parts_for(&app.token);
    let old = create(
        &svc,
        &parts,
        json!({ "action": "create", "slug": "mcp-shape-old" }),
    )
    .await;
    let new = create(
        &svc,
        &parts,
        json!({ "action": "create", "slug": "mcp-shape-new" }),
    )
    .await;
    let team = create(
        &svc,
        &parts,
        json!({ "action": "create", "slug": "mcp-shape-team", "parents": ["+mcp-shape-old"] }),
    )
    .await;
    let parent_ids = |d: &Value| -> Vec<String> {
        d["parents"]
            .as_array()
            .expect("parents")
            .iter()
            .map(|p| p["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(
        parent_ids(&detail(&svc, &parts, team).await),
        [old.to_string()]
    );

    team_manage(
        &svc,
        &parts,
        input(json!({
            "action": "reparent", "team": "+mcp-shape-team",
            "from": "+mcp-shape-old", "to": "+mcp-shape-new",
        })),
    )
    .await
    .expect("a manager of all three reparents");
    assert_eq!(
        parent_ids(&detail(&svc, &parts, team).await),
        [new.to_string()]
    );
    let children = detail(&svc, &parts, new).await;
    assert!(
        children["children"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["id"] == team.to_string()),
        "{children}"
    );

    let detached = one_json(
        &team_manage(
            &svc,
            &parts,
            input(json!({ "action": "detach", "team": new, "child": "mcp-shape-team" })),
        )
        .await
        .expect("a manager of the parent detaches"),
    );
    assert_eq!(detached["detached"], true, "{detached}");
    assert!(parent_ids(&detail(&svc, &parts, team).await).is_empty());

    // A stranger's rename: refused as HTTP refuses it, and the slug stands.
    common::provision_and_approve_second(&app).await;
    let second = common::generate_second_user_jwt();
    let mcp = team_manage(
        &svc,
        &app.relay_parts_for(&second),
        input(json!({ "action": "rename_slug", "team": team, "slug": "mcp-shape-stolen" })),
    )
    .await
    .expect_err("a stranger is refused");
    let http = app
        .client_for(&second)
        .teams()
        .rename_slug(
            team,
            &temper_core::types::team::RenameTeamSlugRequest {
                slug: "mcp-shape-stolen".into(),
            },
        )
        .await
        .expect_err("HTTP refuses the stranger");
    assert_same_refusal("team_manage rename_slug", &mcp, http);
    assert_eq!(detail(&svc, &parts, team).await["slug"], "mcp-shape-team");

    team_manage(
        &svc,
        &parts,
        input(json!({ "action": "rename_slug", "team": "+mcp-shape-team", "slug": "mcp-shape-renamed" })),
    )
    .await
    .expect("its owner renames the slug");
    assert_eq!(
        detail(&svc, &parts, team).await["slug"],
        "mcp-shape-renamed"
    );
    let slugs = my_slugs(&svc, &parts).await;
    assert!(
        slugs.contains(&"mcp-shape-renamed".to_string()),
        "{slugs:?}"
    );
    assert!(!slugs.contains(&"mcp-shape-team".to_string()), "{slugs:?}");
}

/// A system admin sets a team's auto-join role and clears it over MCP, on their own bearer; a
/// person who owns the team but is no admin is refused as HTTP refuses them.
///
/// FAILS IF `set_auto_join` stops relaying, sends the wrong role, or cannot clear.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_admin_sets_auto_join_over_mcp(pool: sqlx::PgPool) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service().await;
    common::make_system_admin(&app.pool, me(&app).await).await;
    let admin = app.relay_parts_for(&app.token);

    // The second person owns the team, so only the admin bar stands in their way.
    common::provision_and_approve_second(&app).await;
    let second = common::generate_second_user_jwt();
    let owner = app.relay_parts_for(&second);
    let team = create(
        &svc,
        &owner,
        json!({ "action": "create", "slug": "mcp-pool" }),
    )
    .await;

    let mcp = team_manage(
        &svc,
        &owner,
        input(json!({ "action": "set_auto_join", "team": team, "auto_join_role": "watcher" })),
    )
    .await
    .expect_err("an owner who is no admin is refused");
    let http = app
        .client_for(&second)
        .teams()
        .set_auto_join(
            team,
            &temper_core::types::team::SetAutoJoinRequest {
                role: Some(temper_core::types::team::TeamRole::Watcher),
            },
        )
        .await
        .expect_err("HTTP refuses the owner");
    assert_same_refusal("team_manage set_auto_join", &mcp, http);
    assert!(detail(&svc, &owner, team).await["auto_join_role"].is_null());

    team_manage(
        &svc,
        &admin,
        input(json!({ "action": "set_auto_join", "team": team, "auto_join_role": "watcher" })),
    )
    .await
    .expect("an admin sets auto-join");
    assert_eq!(
        detail(&svc, &owner, team).await["auto_join_role"],
        "watcher"
    );

    team_manage(
        &svc,
        &admin,
        input(json!({ "action": "set_auto_join", "team": team, "auto_join_role": "member" })),
    )
    .await
    .expect("an admin changes auto-join");
    assert_eq!(detail(&svc, &owner, team).await["auto_join_role"], "member");

    team_manage(
        &svc,
        &admin,
        input(json!({ "action": "set_auto_join", "team": team })),
    )
    .await
    .expect("an admin clears auto-join");
    assert!(detail(&svc, &owner, team).await["auto_join_role"].is_null());
}

/// A person forbids openings over an anchor, sees an opening under it refused, and allows them
/// again, all over MCP. A stranger's forbid is refused as HTTP refuses it.
///
/// FAILS IF `public_within_policy` stops relaying, or sends `forbidden` inverted.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_person_forbids_and_allows_public_within_over_mcp(pool: sqlx::PgPool) {
    let app = common::setup_relay(pool).await;
    let svc = app.mcp_relay_service().await;
    let parts = app.relay_parts_for(&app.token);
    let anchor = create(
        &svc,
        &parts,
        json!({ "action": "create", "slug": "mcp-pol-anchor" }),
    )
    .await;
    let team = create(
        &svc,
        &parts,
        json!({ "action": "create", "slug": "mcp-pol-team", "parents": ["+mcp-pol-anchor"] }),
    )
    .await;
    let forbidden = |pool: sqlx::PgPool| async move {
        sqlx::query_scalar::<_, bool>("SELECT public_within_forbidden FROM kb_teams WHERE id = $1")
            .bind(anchor)
            .fetch_one(&pool)
            .await
            .unwrap()
    };
    let open = json!({
        "action": "public_within_open", "team": "+mcp-pol-team", "anchor": "+mcp-pol-anchor",
    });

    // A stranger's forbid: refused as HTTP refuses it, and nothing is written.
    common::provision_and_approve_second(&app).await;
    let second = common::generate_second_user_jwt();
    let mcp = team_manage(
        &svc,
        &app.relay_parts_for(&second),
        input(json!({ "action": "public_within_policy", "team": anchor, "forbidden": true })),
    )
    .await
    .expect_err("a stranger is refused");
    let http = app
        .client_for(&second)
        .teams()
        .set_public_within_policy(
            anchor,
            &temper_core::types::team::PublicWithinPolicyRequest { forbidden: true },
        )
        .await
        .expect_err("HTTP refuses the stranger");
    assert_same_refusal("team_manage public_within_policy", &mcp, http);
    assert!(!forbidden(app.pool.clone()).await);

    let set = one_json(
        &team_manage(
            &svc,
            &parts,
            input(json!({ "action": "public_within_policy", "team": "+mcp-pol-anchor", "forbidden": true })),
        )
        .await
        .expect("its owner forbids"),
    );
    assert_eq!(set["forbidden"], true, "{set}");
    assert!(forbidden(app.pool.clone()).await);
    let mcp = team_manage(&svc, &parts, input(open.clone()))
        .await
        .expect_err("no opening under a forbid");
    let http = app
        .client
        .teams()
        .open_public_within(team, anchor)
        .await
        .expect_err("HTTP refuses an opening under a forbid");
    assert!(
        matches!(http, ClientError::ForbiddenDetail { .. }),
        "the forbid is named: {http:?}"
    );
    assert_same_refusal("team_manage public_within_open", &mcp, http);

    team_manage(
        &svc,
        &parts,
        input(json!({ "action": "public_within_policy", "team": anchor, "forbidden": false })),
    )
    .await
    .expect("its owner allows again");
    assert!(!forbidden(app.pool.clone()).await);
    let opened = one_json(
        &team_manage(&svc, &parts, input(open))
            .await
            .expect("an opening once allowed"),
    );
    assert_eq!(opened["outcome"], "opened", "{opened}");
}
