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
    ];
    for args in acts {
        let err = team_manage(&svc, &parts, input(args.clone()))
            .await
            .expect_err("a machine is refused");
        assert_eq!(err.code, ErrorCode::INVALID_REQUEST, "{args}: {err:?}");
        assert!(err.message.ends_with(refusal), "{args}: {err:?}");
    }

    let made: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_teams WHERE slug LIKE 'mcp-machine-%'")
            .fetch_one(&app.pool)
            .await
            .unwrap();
    assert_eq!(made, 0, "nothing was written");
}
