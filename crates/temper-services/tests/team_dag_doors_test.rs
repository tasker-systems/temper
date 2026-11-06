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
use temper_services::services::team_service;
use temper_services::test_support;
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
