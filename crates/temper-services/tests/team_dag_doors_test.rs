#![cfg(feature = "test-db")]

//! The team DAG and lifecycle doors (temper-artifacts
//! `specs/2026-11-06-team-dag-and-lifecycle-doors-design.md`): attach, child-side detach, re-parent,
//! create with several parents, the auto-join door, restore and slug rename.
//!
//! The shared fixture:
//!
//! ```text
//!   temper-system (root)          dg-gate (the configured gating team)
//!
//!   dg-org ─▸ dg-eng ─▸ dg-eng-web          dg-side (a second, unrelated tree)
//!
//!   dg-watch (auto-join watcher pool, no parent)
//!
//!   on each of org, eng, eng-web, side:  <team>-owner, <team>-maint, <team>-member, <team>-watcher
//!   admin: a system admin (governance) on no team but the gate it needs
//!   loner: a person on no shared team, with only their personal team
//! ```
//!
//! Profiles are named `dg-<team>-<role>`; every approved person is also a watcher of `dg-watch`,
//! because the pool enrols every approved profile.

use std::collections::HashMap;

use sqlx::PgPool;
use temper_core::types::team::{TeamCreateRequest, TeamRole};
use temper_services::auth::HumanPrincipal;
use temper_services::error::ApiError;
use temper_services::services::{admin_ledger_service, team_service};
use temper_services::test_support;
use temper_substrate::payloads::{AnchorTable, RefTarget};
use temper_workflow::operations::Surface;
use uuid::Uuid;

/// The four roles every shared fixture team carries one person at.
const ROLES: [&str; 4] = ["owner", "maintainer", "member", "watcher"];

/// The shared fixture teams that carry the four-role roster.
const ROSTERED: [&str; 4] = ["org", "eng", "eng-web", "side"];

async fn person(pool: &PgPool, handle: &str) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (id, handle, display_name) \
         VALUES (uuid_generate_v7(), $1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .expect("seed profile");
    for surface in Surface::ALL {
        sqlx::query(
            "INSERT INTO kb_entities (profile_id, name, metadata) VALUES ($1, $2, '{}'::jsonb)",
        )
        .bind(id)
        .bind(format!("{handle}@{}", surface.marker()))
        .execute(pool)
        .await
        .expect("seed emitter");
    }
    test_support::approve(pool, id).await;
    id
}

/// Put `who` on `team` at `role`, overwriting any row (the creator's `owner` row included).
async fn join(pool: &PgPool, team: Uuid, who: Uuid, role: &str) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, $3::team_role) \
         ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(team)
    .bind(who)
    .bind(role)
    .execute(pool)
    .await
    .expect("seed member");
}

async fn create(
    pool: &PgPool,
    by: &HumanPrincipal,
    slug: &str,
    parent: Option<&str>,
    auto_join_role: Option<TeamRole>,
) -> Uuid {
    team_service::create_team(
        pool,
        by,
        &TeamCreateRequest {
            slug: slug.into(),
            name: None,
            parent: parent.map(|p| format!("+{p}")),
            auto_join_role,
        },
    )
    .await
    .unwrap_or_else(|e| panic!("create {slug}: {e:?}"))
    .id
}

#[allow(dead_code)] // fields are read by the phases that use them
struct Fixture {
    root: Uuid,
    gate: Uuid,
    org: Uuid,
    eng: Uuid,
    eng_web: Uuid,
    side: Uuid,
    watch: Uuid,
    admin: Uuid,
    loner: Uuid,
    loner_personal: Uuid,
    /// `(team key, role)` → profile, for the four roles on each of [`ROSTERED`].
    people: HashMap<(&'static str, &'static str), Uuid>,
}

#[allow(dead_code)]
impl Fixture {
    /// The profile at `role` on the fixture team `team` (one of [`ROSTERED`]).
    fn on(&self, team: &str, role: &str) -> Uuid {
        *self
            .people
            .iter()
            .find(|((t, r), _)| *t == team && *r == role)
            .unwrap_or_else(|| panic!("no fixture profile at {role} on {team}"))
            .1
    }

    /// The fixture team's id by key.
    fn team(&self, key: &str) -> Uuid {
        match key {
            "org" => self.org,
            "eng" => self.eng,
            "eng-web" => self.eng_web,
            "side" => self.side,
            "watch" => self.watch,
            "gate" => self.gate,
            "root" => self.root,
            other => panic!("no fixture team {other}"),
        }
    }
}

async fn fixture(pool: &PgPool) -> Fixture {
    let root: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
        .fetch_one(pool)
        .await
        .expect("the root");

    let admin = person(pool, "dg-admin").await;
    test_support::grant_governance(pool, admin).await;
    let admin_p = test_support::human_principal_for(pool, admin).await;

    // The gating team: a plain team the settings row names.
    let gate = create(pool, &admin_p, "dg-gate", None, None).await;
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'dg-gate'")
        .execute(pool)
        .await
        .expect("configure the gating team");

    let mut people = HashMap::new();
    for team in ROSTERED {
        for role in ROLES {
            let id = person(pool, &format!("dg-{team}-{role}")).await;
            people.insert((team, role), id);
        }
    }
    let owner = |team: &'static str| people[&(team, "owner")];

    // Each team is created by its own owner; a child by its owner after they are made a
    // maintainer of the parent (the create door's bar), then put back to their fixture role.
    let org_owner = test_support::human_principal_for(pool, owner("org")).await;
    let org = create(pool, &org_owner, "dg-org", None, None).await;

    join(pool, org, owner("eng"), "maintainer").await;
    let eng_owner = test_support::human_principal_for(pool, owner("eng")).await;
    let eng = create(pool, &eng_owner, "dg-eng", Some("dg-org"), None).await;
    sqlx::query("DELETE FROM kb_team_members WHERE team_id = $1 AND profile_id = $2")
        .bind(org)
        .bind(owner("eng"))
        .execute(pool)
        .await
        .unwrap();

    join(pool, eng, owner("eng-web"), "maintainer").await;
    let web_owner = test_support::human_principal_for(pool, owner("eng-web")).await;
    let eng_web = create(pool, &web_owner, "dg-eng-web", Some("dg-eng"), None).await;
    sqlx::query("DELETE FROM kb_team_members WHERE team_id = $1 AND profile_id = $2")
        .bind(eng)
        .bind(owner("eng-web"))
        .execute(pool)
        .await
        .unwrap();

    let side_owner = test_support::human_principal_for(pool, owner("side")).await;
    let side = create(pool, &side_owner, "dg-side", None, None).await;

    for (key, id) in [
        ("org", org),
        ("eng", eng),
        ("eng-web", eng_web),
        ("side", side),
    ] {
        for role in ROLES {
            join(pool, id, people[&(key, role)], role).await;
        }
    }

    let loner = person(pool, "dg-loner").await;
    let loner_personal: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE personal_of = $1")
        .bind(loner)
        .fetch_one(pool)
        .await
        .expect("the loner's personal team");

    // Last, so its backfill enrols every person above.
    let watch = create(pool, &admin_p, "dg-watch", None, Some(TeamRole::Watcher)).await;

    Fixture {
        root,
        gate,
        org,
        eng,
        eng_web,
        side,
        watch,
        admin,
        loner,
        loner_personal,
        people,
    }
}

/// Is `ancestor` among the teams `who` reaches (`profile_reachable_teams`)?
async fn reaches(pool: &PgPool, who: Uuid, ancestor: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM profile_reachable_teams($1) WHERE team_id = $2)",
    )
    .bind(who)
    .bind(ancestor)
    .fetch_one(pool)
    .await
    .expect("reach probe")
}

/// The fixture is the shape the doc comment draws: reads flow down the tree and never up, the
/// gating team is configured, and the watcher pool holds every approved person at `watcher`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn the_fixture_is_the_shape_it_claims(pool: PgPool) {
    let f = fixture(&pool).await;

    // Reads flow down: a watcher of eng-web reaches eng and org; nobody on org reaches eng-web.
    let web_watcher = f.on("eng-web", "watcher");
    assert!(reaches(&pool, web_watcher, f.eng).await);
    assert!(reaches(&pool, web_watcher, f.org).await);
    assert!(!reaches(&pool, f.on("org", "owner"), f.eng_web).await);
    assert!(!reaches(&pool, f.on("org", "owner"), f.eng).await);
    assert!(!reaches(&pool, web_watcher, f.side).await);

    let gating: bool =
        sqlx::query_scalar("SELECT gating_team_slug = 'dg-gate' FROM kb_system_settings")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(gating, "the gating team is configured");

    for team in ROSTERED {
        for role in ROLES {
            let got: String = sqlx::query_scalar(
                "SELECT role::text FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
            )
            .bind(f.team(team))
            .bind(f.on(team, role))
            .fetch_one(&pool)
            .await
            .unwrap();
            assert_eq!(got, role, "{team}/{role}");
        }
    }

    let pool_role: String = sqlx::query_scalar(
        "SELECT role::text FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
    )
    .bind(f.watch)
    .bind(f.loner)
    .fetch_one(&pool)
    .await
    .expect("the pool enrolled the loner");
    assert_eq!(pool_role, "watcher");

    let personal: bool = sqlx::query_scalar("SELECT personal_of = $2 FROM kb_teams WHERE id = $1")
        .bind(f.loner_personal)
        .bind(f.loner)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(personal);
    let _ = (f.root, f.admin);
}

/// `team_lifecycle_changed` records about `team` with `act`, optionally by `actor` (D8). Every one
/// names the team on its `subject` reference, the axis its managers read.
async fn lifecycle_events(pool: &PgPool, team: Uuid, act: &str, actor: Option<Uuid>) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'team_lifecycle_changed' \
            AND e.payload @> jsonb_build_object('team_id', $1::uuid, 'act', $2::text) \
            AND ($3::uuid IS NULL OR e.payload @> jsonb_build_object('actor', $3::uuid)) \
            AND e.\"references\" @> jsonb_build_array(jsonb_build_object( \
                  'rel', 'subject', 'target', jsonb_build_object('kind', 'kb_teams', 'id', $1::uuid)))",
    )
    .bind(team)
    .bind(act)
    .bind(actor)
    .fetch_one(pool)
    .await
    .expect("lifecycle event count")
}

/// Every `team_lifecycle_changed` record on the ledger.
async fn all_lifecycle_events(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'team_lifecycle_changed'",
    )
    .fetch_one(pool)
    .await
    .expect("lifecycle event count")
}

// ---------------------------------------------------------------------------------------------
// Phase 1 — D8: lifecycle acts are recorded
// ---------------------------------------------------------------------------------------------

/// Creating a team records one `created` event by its creator, in the insert's transaction.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn creating_a_team_records_one_created_event(pool: PgPool) {
    let f = fixture(&pool).await;
    for (team, creator) in [
        (f.org, f.on("org", "owner")),
        (f.eng, f.on("eng", "owner")),
        (f.watch, f.admin),
    ] {
        assert_eq!(
            lifecycle_events(&pool, team, "created", Some(creator)).await,
            1,
            "one created event by the creator"
        );
    }

    // It reads back through the admin ledger on the team's subject axis, for its managers — the
    // door that decodes every row on the page, so an undecodable payload fails it.
    let owner = test_support::human_principal_for(&pool, f.on("org", "owner")).await;
    let page = admin_ledger_service::list_by_subject(
        &pool,
        &owner,
        RefTarget {
            kind: AnchorTable::Teams,
            id: f.org,
        },
        50,
        0,
    )
    .await
    .expect("org's owner reads its ledger page");
    assert!(
        page.iter()
            .any(|e| e.event_type == "team_lifecycle_changed"),
        "{page:?}"
    );
}

/// A refused create records nothing: not a forbidden child, not a slug collision.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_refused_create_records_nothing(pool: PgPool) {
    let f = fixture(&pool).await;
    let before = all_lifecycle_events(&pool).await;

    let member = test_support::human_principal_for(&pool, f.on("org", "member")).await;
    let refusal = team_service::create_team(
        &pool,
        &member,
        &TeamCreateRequest {
            slug: "dg-nope".into(),
            name: None,
            parent: Some("+dg-org".into()),
            auto_join_role: None,
        },
    )
    .await
    .expect_err("a member of the parent creates no child");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");

    let refusal = team_service::create_team(
        &pool,
        &member,
        &TeamCreateRequest {
            slug: "dg-org".into(),
            name: None,
            parent: None,
            auto_join_role: None,
        },
    )
    .await
    .expect_err("a taken slug");
    assert!(matches!(refusal, ApiError::Conflict(_)), "{refusal:?}");

    assert_eq!(
        all_lifecycle_events(&pool).await,
        before,
        "nothing recorded"
    );
}

/// Deleting a team records one `deleted` event by the actor; a delete of a team that is already
/// gone (404) records nothing.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn deleting_a_team_records_one_event_and_a_404_records_none(pool: PgPool) {
    let f = fixture(&pool).await;
    let owner = f.on("side", "owner");
    let p = test_support::human_principal_for(&pool, owner).await;
    team_service::delete_team(&pool, &p, f.side)
        .await
        .expect("the owner deletes side");
    assert_eq!(
        lifecycle_events(&pool, f.side, "deleted", Some(owner)).await,
        1
    );

    let before = all_lifecycle_events(&pool).await;
    let refusal = team_service::delete_team(&pool, &p, f.side)
        .await
        .expect_err("already deleted");
    assert!(matches!(refusal, ApiError::NotFound(_)), "{refusal:?}");
    let refusal = team_service::delete_team(&pool, &p, Uuid::now_v7())
        .await
        .expect_err("no such team");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    assert_eq!(
        all_lifecycle_events(&pool).await,
        before,
        "nothing recorded"
    );
}

// ---------------------------------------------------------------------------------------------
// Phase 2 — D1 attach, D9 parents on detail
// ---------------------------------------------------------------------------------------------

/// `team_parent_link_changed` records with `parent` as subject and `child` as principal.
async fn link_events(pool: &PgPool, parent: Uuid, child: Uuid, act: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'team_parent_link_changed' \
            AND e.payload @> jsonb_build_object('parent_team_id', $1::uuid, \
                                                'child_team_id', $2::uuid, 'act', $3::text)",
    )
    .bind(parent)
    .bind(child)
    .bind(act)
    .fetch_one(pool)
    .await
    .expect("link event count")
}

async fn linked(pool: &PgPool, parent: Uuid, child: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM kb_teams_parents WHERE parent_id = $1 AND child_id = $2)",
    )
    .bind(parent)
    .bind(child)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn attach(
    pool: &PgPool,
    who: Uuid,
    parent: Uuid,
    child: Uuid,
) -> Result<temper_core::types::team::AttachOutcome, ApiError> {
    let p = test_support::human_principal_for(pool, who).await;
    team_service::attach_child(pool, &p, parent, child)
        .await
        .map(|o| o.outcome)
}

/// A manager of both teams attaches: the link is written and recorded once, the child's roster
/// reaches the new parent (a second parent: both ancestors stay reachable), and a repeat answers
/// `exists` and records nothing.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_manager_of_both_attaches_a_second_parent(pool: PgPool) {
    use temper_core::types::team::AttachOutcome;
    let f = fixture(&pool).await;
    // side's owner also maintains eng-web.
    let both = f.on("side", "owner");
    join(&pool, f.eng_web, both, "maintainer").await;
    let web_watcher = f.on("eng-web", "watcher");
    assert!(!reaches(&pool, web_watcher, f.side).await, "fixture");

    assert_eq!(
        attach(&pool, both, f.side, f.eng_web).await.unwrap(),
        AttachOutcome::Linked
    );
    assert_eq!(link_events(&pool, f.side, f.eng_web, "linked").await, 1);
    assert!(
        reaches(&pool, web_watcher, f.side).await,
        "the new parent is reached"
    );
    assert!(
        reaches(&pool, web_watcher, f.org).await,
        "and the old ancestors still are"
    );

    assert_eq!(
        attach(&pool, both, f.side, f.eng_web).await.unwrap(),
        AttachOutcome::Exists
    );
    assert_eq!(
        link_events(&pool, f.side, f.eng_web, "linked").await,
        1,
        "a repeat records nothing"
    );
}

/// Both sides consent: managing only one side, or being a member or watcher of both, is `403`,
/// and nothing is written.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn attach_needs_manage_on_both_sides(pool: PgPool) {
    let f = fixture(&pool).await;
    let parent_only = f.on("side", "maintainer");
    let child_only = f.on("eng-web", "owner");
    let member = f.on("side", "member");
    join(&pool, f.eng_web, member, "member").await;
    let watcher = f.on("side", "watcher");
    join(&pool, f.eng_web, watcher, "watcher").await;
    for who in [parent_only, child_only, member, watcher, f.loner] {
        let refusal = attach(&pool, who, f.side, f.eng_web)
            .await
            .expect_err("not a manager of both");
        assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    }
    assert!(!linked(&pool, f.side, f.eng_web).await);
    assert_eq!(link_events(&pool, f.side, f.eng_web, "linked").await, 0);
}

/// The root and the gating team take a child only from a system admin, and an auto-join team is
/// attached only by one (R6), whatever rows the caller holds.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn instance_parents_and_auto_join_children_are_an_admins_act(pool: PgPool) {
    use temper_core::types::team::AttachOutcome;
    let f = fixture(&pool).await;
    let owner = f.on("side", "owner");
    join(&pool, f.root, owner, "owner").await;
    join(&pool, f.gate, owner, "owner").await;
    join(&pool, f.watch, owner, "owner").await;
    for (parent, child) in [(f.root, f.side), (f.gate, f.side), (f.side, f.watch)] {
        let refusal = attach(&pool, owner, parent, child)
            .await
            .expect_err("a non-admin");
        assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    }
    for (parent, child) in [(f.gate, f.side), (f.side, f.watch)] {
        assert_eq!(
            attach(&pool, f.admin, parent, child).await.unwrap(),
            AttachOutcome::Linked
        );
    }
    // The pool's whole roster now reaches side.
    assert!(reaches(&pool, f.loner, f.side).await);
}

/// Structural refusals, for a caller past the authority checks: a personal team on either side and
/// the root as a child are `400`, a cycle is `409`, a soft-deleted side is `404`.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn attach_refuses_what_the_dag_cannot_hold(pool: PgPool) {
    let f = fixture(&pool).await;
    // The loner owns their personal team; make them a maintainer of side too.
    join(&pool, f.side, f.loner, "maintainer").await;
    for (parent, child) in [(f.loner_personal, f.side), (f.side, f.loner_personal)] {
        let refusal = attach(&pool, f.loner, parent, child)
            .await
            .expect_err("a personal team");
        assert!(matches!(refusal, ApiError::BadRequest(_)), "{refusal:?}");
    }
    let refusal = attach(&pool, f.admin, f.side, f.root)
        .await
        .expect_err("the root takes no parent");
    assert!(matches!(refusal, ApiError::BadRequest(_)), "{refusal:?}");

    // eng-web above org: org's owner also maintains eng-web.
    let org_owner = f.on("org", "owner");
    join(&pool, f.eng_web, org_owner, "maintainer").await;
    let refusal = attach(&pool, org_owner, f.eng_web, f.org)
        .await
        .expect_err("a cycle");
    assert!(matches!(refusal, ApiError::Conflict(_)), "{refusal:?}");

    // Soft-deleted either side.
    let side_owner = f.on("side", "owner");
    join(&pool, f.eng_web, side_owner, "maintainer").await;
    let p = test_support::human_principal_for(&pool, side_owner).await;
    team_service::delete_team(&pool, &p, f.side).await.unwrap();
    for (parent, child) in [(f.side, f.eng_web), (f.eng_web, f.side)] {
        let refusal = attach(&pool, side_owner, parent, child)
            .await
            .expect_err("a dead side");
        assert!(matches!(refusal, ApiError::NotFound(_)), "{refusal:?}");
    }
    assert_eq!(link_events(&pool, f.eng_web, f.org, "linked").await, 0);
}

/// G12: a caller demoted on the parent after the pool pre-check, while the attach waits on the
/// team-row lock, is refused under the lock and nothing is written.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_caller_demoted_under_the_lock_attaches_nothing(pool: PgPool) {
    let f = fixture(&pool).await;
    let both = f.on("side", "owner");
    join(&pool, f.eng_web, both, "maintainer").await;
    let first = f.side.min(f.eng_web);

    let mut holder = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM kb_teams WHERE id = $1 FOR NO KEY UPDATE")
        .bind(first)
        .execute(&mut *holder)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE kb_team_members SET role = 'member' WHERE team_id = $1 AND profile_id = $2",
    )
    .bind(f.eng_web)
    .bind(both)
    .execute(&mut *holder)
    .await
    .unwrap();

    let pool2 = pool.clone();
    let (side, web) = (f.side, f.eng_web);
    let contender = tokio::spawn(async move { attach(&pool2, both, side, web).await });
    test_support::await_team_lock_waiter(&pool).await;
    holder.commit().await.unwrap();

    let refusal = contender
        .await
        .unwrap()
        .expect_err("demoted under the lock");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    assert!(!linked(&pool, f.side, f.eng_web).await);
}

/// D9: every member of a team, at any role, sees its live parents; `children` stays a manager's.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_member_sees_parents_and_only_managers_see_children(pool: PgPool) {
    let f = fixture(&pool).await;
    let watcher = test_support::caller_for(&pool, f.on("eng-web", "watcher")).await;
    let web = team_service::team_detail(&pool, &watcher, f.eng_web)
        .await
        .expect("a watcher reads its team");
    let parents: Vec<Uuid> = web.parents.iter().map(|t| t.id).collect();
    assert_eq!(parents, vec![f.eng]);

    // The watcher of eng-web is also a watcher of eng in this case.
    join(&pool, f.eng, f.on("eng-web", "watcher"), "watcher").await;
    let eng = team_service::team_detail(&pool, &watcher, f.eng)
        .await
        .unwrap();
    assert!(eng.children.is_empty(), "a watcher sees no children");
    assert_eq!(
        eng.parents.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![f.org]
    );

    let manager = test_support::caller_for(&pool, f.on("eng", "maintainer")).await;
    let eng = team_service::team_detail(&pool, &manager, f.eng)
        .await
        .unwrap();
    assert_eq!(
        eng.children.iter().map(|t| t.id).collect::<Vec<_>>(),
        vec![f.eng_web]
    );

    // A dead parent is not listed.
    let p = test_support::human_principal_for(&pool, f.on("eng", "owner")).await;
    team_service::delete_team(&pool, &p, f.eng).await.unwrap();
    let web = team_service::team_detail(&pool, &watcher, f.eng_web)
        .await
        .unwrap();
    assert!(web.parents.is_empty(), "{:?}", web.parents);
}

// ---------------------------------------------------------------------------------------------
// Phase 3 — D2 child-side detach, D3 re-parent
// ---------------------------------------------------------------------------------------------

async fn detach(pool: &PgPool, who: Uuid, parent: Uuid, child: Uuid) -> Result<(), ApiError> {
    let p = test_support::human_principal_for(pool, who).await;
    team_service::detach_child(pool, &p, parent, child).await
}

async fn reparent(
    pool: &PgPool,
    who: Uuid,
    child: Uuid,
    from: &str,
    to: &str,
) -> Result<temper_core::types::team::ReparentOutcome, ApiError> {
    let p = test_support::human_principal_for(pool, who).await;
    team_service::reparent(
        pool,
        &p,
        child,
        &temper_core::types::team::ReparentRequest {
            from: from.into(),
            to: to.into(),
        },
    )
    .await
}

/// R1: a manager of the child detaches it from a parent it does not manage — even the gating team,
/// which a parent-side manager could not — and the reach ends. A plain member of the child cannot.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_childs_manager_detaches_it_from_any_parent(pool: PgPool) {
    let f = fixture(&pool).await;
    let refusal = detach(&pool, f.on("eng-web", "member"), f.eng, f.eng_web)
        .await
        .expect_err("a member of the child");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");

    let web_maint = f.on("eng-web", "maintainer");
    detach(&pool, web_maint, f.eng, f.eng_web)
        .await
        .expect("the child's maintainer leaves eng");
    assert!(!reaches(&pool, f.on("eng-web", "watcher"), f.eng).await);
    assert_eq!(link_events(&pool, f.eng, f.eng_web, "unlinked").await, 1);

    // Under the gating team (attached by an admin), the child's owner may still leave it.
    attach(&pool, f.admin, f.gate, f.side).await.unwrap();
    detach(&pool, f.on("side", "owner"), f.gate, f.side)
        .await
        .expect("leaving the gating team's subtree is a reduction");
    assert!(!linked(&pool, f.gate, f.side).await);
}

/// The personal team's link under the root is still nobody's to cut, its owner's included.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn nobody_detaches_a_personal_team(pool: PgPool) {
    let f = fixture(&pool).await;
    for who in [f.loner, f.admin] {
        let refusal = detach(&pool, who, f.root, f.loner_personal)
            .await
            .expect_err("a personal team");
        assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    }
    assert!(linked(&pool, f.root, f.loner_personal).await);
}

/// D3: a manager of the child who also manages the new parent moves it in one act — the old
/// ancestor is no longer reached, the new one is, and the two link records share a correlation id.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_move_is_one_act_with_one_correlation(pool: PgPool) {
    let f = fixture(&pool).await;
    let mover = f.on("eng-web", "owner");
    join(&pool, f.side, mover, "maintainer").await;
    let watcher = f.on("eng-web", "watcher");

    let moved = reparent(&pool, mover, f.eng_web, "+dg-eng", &f.side.to_string())
        .await
        .expect("move eng-web from eng to side");
    assert_eq!(
        (moved.from.id, moved.to.id, moved.team.id),
        (f.eng, f.side, f.eng_web)
    );
    assert!(
        !reaches(&pool, watcher, f.eng).await,
        "the old parent is gone"
    );
    assert!(
        !reaches(&pool, watcher, f.org).await,
        "and everything above it"
    );
    assert!(
        reaches(&pool, watcher, f.side).await,
        "the new parent is reached"
    );

    let correlations: Vec<Option<Uuid>> = sqlx::query_scalar(
        "SELECT e.correlation_id FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'team_parent_link_changed' \
            AND e.payload @> jsonb_build_object('child_team_id', $1::uuid) \
            AND e.correlation_id = $2",
    )
    .bind(f.eng_web)
    .bind(moved.correlation_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        correlations.len(),
        2,
        "link and unlink carry one correlation id"
    );
}

/// A move that would close a cycle is `409` and changes nothing; `from` that is not a parent is
/// `404`; a caller who does not manage `to` is `403`; `from == to` is `400`. In every case the old
/// link is still there.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_refused_move_changes_nothing(pool: PgPool) {
    let f = fixture(&pool).await;
    // eng's owner also maintains eng-web, eng's own child.
    let eng_owner = f.on("eng", "owner");
    join(&pool, f.eng_web, eng_owner, "maintainer").await;
    let refusal = reparent(&pool, eng_owner, f.eng, "+dg-org", "+dg-eng-web")
        .await
        .expect_err("under its own child");
    assert!(matches!(refusal, ApiError::Conflict(_)), "{refusal:?}");
    assert!(linked(&pool, f.org, f.eng).await, "the old link survives");
    assert!(!linked(&pool, f.eng_web, f.eng).await);

    join(&pool, f.side, eng_owner, "maintainer").await;
    let refusal = reparent(&pool, eng_owner, f.eng, "+dg-eng-web", "+dg-side")
        .await
        .expect_err("eng-web is not eng's parent");
    assert!(matches!(refusal, ApiError::NotFound(_)), "{refusal:?}");
    assert!(
        !linked(&pool, f.side, f.eng).await,
        "the new link rolled back"
    );

    let refusal = reparent(&pool, eng_owner, f.eng, "+dg-org", "+dg-org")
        .await
        .expect_err("from == to");
    assert!(matches!(refusal, ApiError::BadRequest(_)), "{refusal:?}");

    let eng_maint = f.on("eng", "maintainer");
    let refusal = reparent(&pool, eng_maint, f.eng, "+dg-org", "+dg-side")
        .await
        .expect_err("does not manage side");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    let refusal = reparent(&pool, f.on("side", "owner"), f.eng, "+dg-org", "+dg-side")
        .await
        .expect_err("does not manage eng");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");

    assert!(linked(&pool, f.org, f.eng).await);
    assert!(!linked(&pool, f.side, f.eng).await);
}
