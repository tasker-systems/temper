#![cfg(feature = "test-db")]

//! Who reads a team's membership record (team-role rank spec C12, R5; plan Task B5), and what is
//! never recorded at all (C8's derived writers).
//!
//! The record is readable by exactly those who could make it: the team's owners and maintainers
//! (on the gating team, a system admin only) and system admins. Not a member, not the member acted
//! on, not a manager of some other team. A member's own self-acts stay readable to them as actor.

use sqlx::PgPool;
use temper_core::types::ids::ProfileId;
use temper_core::types::team::{AddMemberRequest, TeamRole};
use temper_services::error::ApiError;
use temper_services::services::{admin_ledger_service, team_service};
use temper_services::test_support;
use temper_substrate::payloads::{AnchorTable, RefTarget};
use uuid::Uuid;

/// An approved profile with its `<handle>@web` emitter — it can act, and it can read its own acts.
async fn person(pool: &PgPool, handle: &str) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO kb_entities (profile_id, name) VALUES ($1, $2 || '@web')")
        .bind(id)
        .bind(handle)
        .execute(pool)
        .await
        .unwrap();
    test_support::approve(pool, id).await;
    id
}

async fn team(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn seat(pool: &PgPool, team: Uuid, profile: Uuid, role: &str) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, $3::team_role)",
    )
    .bind(team)
    .bind(profile)
    .bind(role)
    .execute(pool)
    .await
    .unwrap();
}

fn team_subject(id: Uuid) -> RefTarget {
    RefTarget {
        kind: AnchorTable::Teams,
        id,
    }
}

/// The membership records `reader` may read about `team` — `None` when the door answers 404.
async fn read_by_subject(pool: &PgPool, reader: Uuid, team: Uuid) -> Option<usize> {
    let caller = test_support::human_principal_for(pool, reader).await;
    match admin_ledger_service::list_by_subject(pool, &caller, team_subject(team), 50, 0).await {
        Ok(entries) => Some(
            entries
                .iter()
                .filter(|e| e.event_type == "team_membership_changed")
                .count(),
        ),
        Err(ApiError::NotFound(_)) => None,
        Err(other) => panic!("unexpected ledger refusal {other:?}"),
    }
}

struct Scene {
    team: Uuid,
    owner: Uuid,
    maintainer: Uuid,
    member: Uuid,
    removed: Uuid,
}

/// A team on which the owner added `removed`, and then the maintainer removed them: two records.
async fn scene(pool: &PgPool) -> Scene {
    let (owner, maintainer, member, removed) = (
        person(pool, "owner").await,
        person(pool, "maint").await,
        person(pool, "member").await,
        person(pool, "removed").await,
    );
    let t = team(pool, "acme").await;
    seat(pool, t, owner, "owner").await;
    seat(pool, t, maintainer, "maintainer").await;
    seat(pool, t, member, "member").await;
    team_service::add_member(
        pool,
        &test_support::human_principal_for(pool, owner).await,
        t,
        &AddMemberRequest {
            profile_id: removed,
            role: TeamRole::Member,
        },
    )
    .await
    .unwrap();
    team_service::remove_member(
        pool,
        &test_support::human_principal_for(pool, maintainer).await,
        t,
        removed,
    )
    .await
    .unwrap();
    Scene {
        team: t,
        owner,
        maintainer,
        member,
        removed,
    }
}

/// C12, bite: the team's owner and maintainer — the people who could make these changes — read
/// them by subject, and so does a system admin who is not a member.
#[sqlx::test(migrations = "../../migrations")]
async fn managers_of_the_team_and_admins_read_its_membership_record(pool: PgPool) {
    let s = scene(&pool).await;
    let admin = person(&pool, "admin").await;
    test_support::grant_governance(&pool, admin).await;
    for reader in [s.owner, s.maintainer, admin] {
        assert_eq!(
            read_by_subject(&pool, reader, s.team).await,
            Some(2),
            "reader {reader}"
        );
    }
}

/// C12, bite: a plain member, the removed member, and a maintainer of a DIFFERENT team read none.
#[sqlx::test(migrations = "../../migrations")]
async fn no_one_else_reads_it(pool: PgPool) {
    let s = scene(&pool).await;
    let elsewhere = person(&pool, "elsewhere").await;
    let other = team(&pool, "other").await;
    seat(&pool, other, elsewhere, "maintainer").await;
    for reader in [s.member, s.removed, elsewhere] {
        assert_eq!(
            read_by_subject(&pool, reader, s.team).await,
            None,
            "reader {reader}"
        );
    }
}

/// C12/C14, bite: on the gating team, an owner who is not a system admin reads nothing.
#[sqlx::test(migrations = "../../migrations")]
async fn a_gating_team_owner_who_is_not_an_admin_reads_none(pool: PgPool) {
    let admin = person(&pool, "admin").await;
    test_support::grant_governance(&pool, admin).await;
    let (gate_owner, newcomer) = (person(&pool, "gateowner").await, person(&pool, "new").await);
    let gate = team(&pool, "gate").await;
    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'gate'")
        .execute(&pool)
        .await
        .unwrap();
    seat(&pool, gate, gate_owner, "owner").await;
    team_service::add_member(
        &pool,
        &test_support::human_principal_for(&pool, admin).await,
        gate,
        &AddMemberRequest {
            profile_id: newcomer,
            role: TeamRole::Watcher,
        },
    )
    .await
    .unwrap();

    assert_eq!(read_by_subject(&pool, gate_owner, gate).await, None);
    assert_eq!(read_by_subject(&pool, admin, gate).await, Some(1));
}

/// C12, bite: a member reads their own leave on the actor axis — it is emitted by them — while the
/// removal a manager performed on someone else never reaches that someone.
#[sqlx::test(migrations = "../../migrations")]
async fn a_member_reads_their_own_leave_as_actor(pool: PgPool) {
    let s = scene(&pool).await;
    let leaver = test_support::human_principal_for(&pool, s.member).await;
    team_service::remove_member(&pool, &leaver, s.team, s.member)
        .await
        .unwrap();

    let mine =
        admin_ledger_service::list_by_actor(&pool, &leaver, ProfileId::from(s.member), 50, 0)
            .await
            .unwrap();
    let leaves: Vec<_> = mine
        .iter()
        .filter(|e| e.event_type == "team_membership_changed")
        .collect();
    assert_eq!(leaves.len(), 1, "their own leave");
    assert_eq!(leaves[0].payload["via"], "self");

    let removed = test_support::human_principal_for(&pool, s.removed).await;
    let theirs =
        admin_ledger_service::list_by_actor(&pool, &removed, ProfileId::from(s.removed), 50, 0)
            .await
            .unwrap();
    assert!(
        theirs
            .iter()
            .all(|e| e.event_type != "team_membership_changed"),
        "a removal by a manager is not the removed member's act"
    );
}

/// C12, pin: the member acted on gets no subject arm — the record names them only as its
/// `principal`, never as a `subject`. Asked as a system admin, who can read every admin event type,
/// so the answer comes from the reference match itself rather than from a refusal before any query.
#[sqlx::test(migrations = "../../migrations")]
async fn the_removed_member_finds_nothing_about_themselves_by_subject(pool: PgPool) {
    let s = scene(&pool).await;
    let admin = person(&pool, "admin").await;
    test_support::grant_governance(&pool, admin).await;
    // The records exist and this reader reads them — by the team, their subject.
    assert_eq!(read_by_subject(&pool, admin, s.team).await, Some(2));
    let caller = test_support::human_principal_for(&pool, admin).await;
    let about_removed = admin_ledger_service::list_by_subject(
        &pool,
        &caller,
        RefTarget {
            kind: AnchorTable::Profiles,
            id: s.removed,
        },
        50,
        0,
    )
    .await
    .expect("a system admin reads by any subject");
    assert!(
        about_removed
            .iter()
            .all(|e| e.event_type != "team_membership_changed"),
        "the member acted on is a principal of the record, not its subject"
    );
}

async fn membership_events(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'team_membership_changed'",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

/// C8 pin (R13): the derived writers stay un-evented — personal-team genesis on a profile insert,
/// and an auto-join enrollment.
#[sqlx::test(migrations = "../../migrations")]
async fn derived_writers_record_nothing(pool: PgPool) {
    let p = person(&pool, "someone").await;
    let personal: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_team_members tm JOIN kb_teams t ON t.id = tm.team_id WHERE t.personal_of = $1")
            .bind(p)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(personal, 1, "the personal team was written");

    let everyone: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name, auto_join_role) VALUES ('everyone', 'everyone', 'watcher') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("SELECT backfill_auto_join_team($1)")
        .bind(everyone)
        .execute(&pool)
        .await
        .unwrap();
    let enrolled: i64 =
        sqlx::query_scalar("SELECT count(*) FROM kb_team_members WHERE team_id = $1")
            .bind(everyone)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(enrolled >= 1, "auto-join enrolled the approved profile");

    assert_eq!(membership_events(&pool).await, 0);
}
