#![cfg(feature = "test-db")]

//! The S3 security-review residual routes, after Pete's rulings of 2026-11-01 (task
//! `01a19826-4e34-735c-a8c5-13ec6fb27644`, decision `01a19843-b86b-766f-b06e-65560b0f6f6f`). Each
//! route by which a non-admin could put a grant right or reach onto a machine they control is
//! either refused here, with a live control beside the refusal, or pinned as allowed by design.
//!
//! 1. Team laundering: a grant to a shared team the delegate manages. **By design** (pinned).
//! 2. Connection reach onto the caller's own personal team. **Closed.**
//! 3. A grant to a machine the caller controls but did not register (owns its team, or rotated it). **Closed.**
//! 4. A machine a person controls keeps a team after the person leaves it. **Closed.**
//! 5. A grant right on the caller's own personal team. **Relaxed**: it counts as in person.
//!
//! ```text
//!   max:    maintainer on umbrella; umbrella holds read+write+grant on the resource
//!           (max's right there is inherited through a team)
//!   owner:  owns the resource's home
//! ```

use sqlx::PgPool;
use temper_core::types::cognitive_maps::GrantCapabilityRequest;
use temper_core::types::connection::ProvisionConnectionRequest;
use temper_core::types::machine::{GrantSpec, IssueMachineRequest, TeamSpec};
use temper_services::auth_config::AuthMode;
use temper_services::error::ApiError;
use temper_services::services::{
    access_service, connection_service, machine_client_service, machine_registration_service,
    team_service,
};
use temper_services::test_support;
use temper_workflow::operations::Surface;
use uuid::Uuid;

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

async fn team(pool: &PgPool, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_teams (id, slug, name) VALUES (uuid_generate_v7(), $1, $1) \
         ON CONFLICT (slug) DO UPDATE SET slug = EXCLUDED.slug RETURNING id",
    )
    .bind(slug)
    .fetch_one(pool)
    .await
    .expect("seed team")
}

async fn join(pool: &PgPool, team: Uuid, profile: Uuid, role: &str) {
    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, $3::text::team_role) \
         ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(team)
    .bind(profile)
    .bind(role)
    .execute(pool)
    .await
    .expect("seed membership");
}

async fn drop_member(pool: &PgPool, team: Uuid, profile: Uuid) {
    sqlx::query("DELETE FROM kb_team_members WHERE team_id = $1 AND profile_id = $2")
        .bind(team)
        .bind(profile)
        .execute(pool)
        .await
        .expect("drop membership");
}

async fn personal(pool: &PgPool, profile: Uuid) -> Uuid {
    sqlx::query_scalar("SELECT id FROM kb_teams WHERE personal_of = $1")
        .bind(profile)
        .fetch_one(pool)
        .await
        .expect("personal team")
}

async fn can(pool: &PgPool, profile: Uuid, verb: &str, table: &str, id: Uuid) -> bool {
    sqlx::query_scalar("SELECT can('kb_profiles', $1, $2, $3, $4)")
        .bind(profile)
        .bind(verb)
        .bind(table)
        .bind(id)
        .fetch_one(pool)
        .await
        .expect("can")
}

async fn is_member(pool: &PgPool, team: Uuid, profile: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM kb_team_members WHERE team_id = $1 AND profile_id = $2)",
    )
    .bind(team)
    .bind(profile)
    .fetch_one(pool)
    .await
    .expect("membership")
}

async fn rows(pool: &PgPool, subject: Uuid, principal: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM kb_access_grants WHERE subject_id = $1 AND principal_id = $2",
    )
    .bind(subject)
    .bind(principal)
    .fetch_one(pool)
    .await
    .expect("count")
}

fn grant(
    subject_table: &str,
    subject: Uuid,
    principal_table: &str,
    principal: Uuid,
) -> GrantCapabilityRequest {
    GrantCapabilityRequest {
        subject_table: subject_table.into(),
        subject_id: subject,
        principal_table: principal_table.into(),
        principal_id: principal,
        can_read: true,
        can_write: true,
        can_delete: false,
        can_grant: false,
    }
}

fn register(
    label: &str,
    owner_team: Uuid,
    teams: Vec<TeamSpec>,
    grants: Vec<GrantSpec>,
) -> IssueMachineRequest {
    IssueMachineRequest {
        label: label.into(),
        owner_team_id: Some(owner_team),
        teams,
        grants,
    }
}

fn member_of(team_id: Uuid) -> TeamSpec {
    TeamSpec {
        team_id,
        role: "member".into(),
    }
}

struct Fixture {
    owner: Uuid,
    max: Uuid,
    umbrella: Uuid,
    resource: Uuid,
}

async fn fixture(pool: &PgPool) -> Fixture {
    // The root exists before profiles, so each personal team is linked under it.
    team(pool, "temper-system").await;
    let owner = person(pool, "mr-owner").await;
    let max = person(pool, "mr-max").await;
    let umbrella = team(pool, "mr-umbrella").await;
    join(pool, umbrella, max, "maintainer").await;
    let ctx: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name) \
         VALUES (uuid_generate_v7(), 'kb_profiles', $1, 'mr-ctx', 'mr-ctx') RETURNING id",
    )
    .bind(owner)
    .fetch_one(pool)
    .await
    .unwrap();
    let resource: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (id, title, origin_uri) VALUES (uuid_generate_v7(), 'r', '') \
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_resource_homes \
           (id, resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES (uuid_generate_v7(), $1, 'kb_contexts', $2, $3, $3)",
    )
    .bind(resource)
    .bind(ctx)
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO kb_access_grants (id, subject_table, subject_id, principal_table, principal_id, \
           can_read, can_write, can_grant, granted_by_profile_id) \
         VALUES (uuid_generate_v7(), 'kb_resources', $1, 'kb_teams', $2, true, true, true, $3)",
    )
    .bind(resource)
    .bind(umbrella)
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
    Fixture {
        owner,
        max,
        umbrella,
        resource,
    }
}

/// Route 1, **by design** (Pete, 2026-11-01): a delegate may grant a right they hold through one
/// team to a shared team they created and manage, and that team's members keep it after the
/// delegate leaves the team the right came from. That is delegation; see `recipient_bar`'s doc.
/// The two spellings of the caller stay refused beside it: their own personal team and their own
/// machine.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_delegates_grant_to_a_shared_team_they_manage_is_delegation_by_design(pool: PgPool) {
    let f = fixture(&pool).await;
    let side = team(&pool, "mr-max-side").await;
    join(&pool, side, f.max, "owner").await;
    let colleague = person(&pool, "mr-colleague").await;
    join(&pool, side, colleague, "member").await;
    let m = test_support::human_principal_for(&pool, f.max).await;

    access_service::grant_capability(
        &pool,
        &m,
        &grant("kb_resources", f.resource, "kb_teams", side),
    )
    .await
    .expect("a delegate shares with a shared team they manage");
    drop_member(&pool, f.umbrella, f.max).await;
    assert!(
        can(&pool, colleague, "write", "kb_resources", f.resource).await,
        "the shared team's members keep the delegated grant: delegation's ordinary residue"
    );

    // Beside it, the two spellings of the caller are refused (with the right held again).
    join(&pool, f.umbrella, f.max, "maintainer").await;
    let own_personal = personal(&pool, f.max).await;
    let refusal = access_service::grant_capability(
        &pool,
        &m,
        &grant("kb_resources", f.resource, "kb_teams", own_personal),
    )
    .await
    .expect_err("the caller's own personal team is the caller");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    let bot = machine_registration_service::issue(
        &pool,
        &m,
        &register("mr-bot", own_personal, vec![], vec![]),
        AuthMode::TemperAs,
    )
    .await
    .expect("register")
    .client
    .profile_id;
    let refusal = access_service::grant_capability(
        &pool,
        &m,
        &grant("kb_resources", f.resource, "kb_profiles", bot),
    )
    .await
    .expect_err("the caller's own machine is the caller");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
}

/// Route 2, **closed**: a connection's team owner may not bind its read-reach to their own personal
/// team, as the grant door already refused. Still landing: a binding to the owning team, a
/// connection the personal team itself owns bound to that team, and a system admin's binding to
/// their own personal team.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn connection_reach_onto_the_callers_own_personal_team_is_refused(pool: PgPool) {
    let f = fixture(&pool).await;
    let owning = team(&pool, "mr-conn-team").await;
    join(&pool, owning, f.max, "owner").await;
    let m = test_support::human_principal_for(&pool, f.max).await;
    let provision = |name: &str, team| ProvisionConnectionRequest {
        provider: "github".into(),
        name: name.into(),
        owner_team_id: Some(team),
        reach_granularity: None,
        reach_covers: None,
    };
    let c = connection_service::provision(&pool, &m, &provision("MR Conn", owning))
        .await
        .expect("provision");
    let own_personal = personal(&pool, f.max).await;

    let refusal = connection_service::grant_reach(&pool, &m, c.id, own_personal, None)
        .await
        .expect_err("reach onto the caller's own personal team is a grant to the caller");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    assert_eq!(rows(&pool, c.id, own_personal).await, 0, "nothing written");
    assert!(!can(&pool, f.max, "read", "kb_connections", c.id).await);

    // Live control: the owning team itself.
    connection_service::grant_reach(&pool, &m, c.id, owning, None)
        .await
        .expect("reach onto a shared team the caller manages");
    assert_eq!(rows(&pool, c.id, owning).await, 1);

    // Live control: a connection max's personal team itself owns may take reach onto that team.
    // That hold cannot be lost, and without the reach max could not read his own connection.
    let own_conn =
        connection_service::provision(&pool, &m, &provision("MR Own Conn", own_personal))
            .await
            .expect("max provisions a connection on his personal team");
    connection_service::grant_reach(&pool, &m, own_conn.id, own_personal, None)
        .await
        .expect("a personal connection's reach onto its own personal team");
    assert!(can(&pool, f.max, "read", "kb_connections", own_conn.id).await);

    // A system admin onto their own personal team is unchanged.
    let admin = person(&pool, "mr-admin").await;
    test_support::approved_admin(&pool, admin).await;
    let a = test_support::human_principal_for(&pool, admin).await;
    let admins_conn = connection_service::provision(&pool, &a, &provision("MR Admin Conn", owning))
        .await
        .expect("an admin provisions");
    let admins_personal = personal(&pool, admin).await;
    connection_service::grant_reach(&pool, &a, admins_conn.id, admins_personal, None)
        .await
        .expect("a system admin binds reach to their own personal team");
}

/// Route 3, **closed**: a co-owner of a machine's owning team controls it (they may rotate its
/// secret), so a grant from them is held to the in-person bar even though someone else registered
/// it, before a rotation and after one. A co-owner who holds the right in person is admitted, and
/// so is a grant to a machine the caller does not control.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_grant_to_a_machine_the_caller_controls_but_did_not_register_needs_an_in_person_right(
    pool: PgPool,
) {
    let f = fixture(&pool).await;
    let alice = person(&pool, "mr-alice").await;
    let org = team(&pool, "mr-org").await;
    join(&pool, org, alice, "owner").await;
    join(&pool, org, f.max, "owner").await;
    join(&pool, org, f.owner, "owner").await;
    let a = test_support::human_principal_for(&pool, alice).await;
    let issued = machine_registration_service::issue(
        &pool,
        &a,
        &register("mr-org-bot", org, vec![], vec![]),
        AuthMode::TemperAs,
    )
    .await
    .expect("alice registers the org's bot");
    let bot = issued.client.profile_id;
    let m = test_support::human_principal_for(&pool, f.max).await;
    let req = grant("kb_resources", f.resource, "kb_profiles", bot);

    let refusal = access_service::grant_capability(&pool, &m, &req)
        .await
        .expect_err("max controls the bot and holds the right only through umbrella");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");

    machine_client_service::rotate_secret(&pool, &m, issued.client.id, 0, AuthMode::TemperAs)
        .await
        .expect("an owner of the owning team may rotate");
    let refusal = access_service::grant_capability(&pool, &m, &req)
        .await
        .expect_err("still refused once max holds the credential");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    assert_eq!(rows(&pool, f.resource, bot).await, 0, "nothing written");

    // Live control: the resource's home owner, also an owner of the org, holds the right in person.
    let o = test_support::human_principal_for(&pool, f.owner).await;
    access_service::grant_capability(&pool, &o, &req)
        .await
        .expect("an in-person right reaches a machine the caller controls");
    assert_eq!(rows(&pool, f.resource, bot).await, 1);

    // Rotate, then step down: max minted the bot's current secret, so he holds its key after he
    // stops owning the org, and he still controls it.
    sqlx::query("DELETE FROM kb_access_grants WHERE subject_id = $1 AND principal_id = $2")
        .bind(f.resource)
        .bind(bot)
        .execute(&pool)
        .await
        .unwrap();
    join(&pool, org, f.max, "member").await;
    let refusal = access_service::grant_capability(&pool, &m, &req)
        .await
        .expect_err("the rotator still holds the key after stepping down");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    assert_eq!(rows(&pool, f.resource, bot).await, 0, "nothing written");

    // Live control: a second org bot max never registered or rotated, now that he no longer owns
    // the org, is not his. An inherited right reaches it as it reaches anyone else's machine.
    let other_bot = machine_registration_service::issue(
        &pool,
        &a,
        &register("mr-org-bot-2", org, vec![], vec![]),
        AuthMode::TemperAs,
    )
    .await
    .expect("alice registers a second org bot")
    .client
    .profile_id;
    access_service::grant_capability(
        &pool,
        &m,
        &grant("kb_resources", f.resource, "kb_profiles", other_bot),
    )
    .await
    .expect("a machine the caller does not control is an ordinary recipient");
}

/// Route 4, **closed**: when a person leaves a team, every machine on it whose key they hold or can
/// still mint leaves with them. That covers a machine they registered (owned by their personal team,
/// or by a throwaway team), one they only rotated, and one owned by a team they still own, even when
/// a teammate co-owns it. A machine the team owns stays once another owner has rotated it. A machine
/// whose only row in the person's name is revoked stays too. Removal by an owner and the person's own
/// leave are both covered. Each removal is a recorded `team_membership_changed` (`machine_reach`),
/// correlated with the person's own event.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn every_machine_whose_key_a_person_holds_leaves_a_team_with_them(pool: PgPool) {
    let f = fixture(&pool).await;
    join(&pool, f.umbrella, f.owner, "owner").await;
    let m = test_support::human_principal_for(&pool, f.max).await;
    let o = test_support::human_principal_for(&pool, f.owner).await;
    let own_personal = personal(&pool, f.max).await;
    // A throwaway shared team only max owns.
    let throwaway = team(&pool, "mr-throwaway").await;
    join(&pool, throwaway, f.max, "owner").await;
    // A team max owns with a teammate who is also on umbrella.
    let ops = team(&pool, "mr-ops").await;
    join(&pool, ops, f.max, "owner").await;
    let ops_peer = person(&pool, "mr-ops-peer").await;
    join(&pool, ops, ops_peer, "owner").await;
    join(&pool, f.umbrella, ops_peer, "maintainer").await;
    let p = test_support::human_principal_for(&pool, ops_peer).await;
    async fn issue(
        pool: &PgPool,
        who: &temper_services::auth::HumanPrincipal,
        label: &str,
        owner_team: Uuid,
        teams: Vec<TeamSpec>,
    ) -> (Uuid, Uuid) {
        let c = machine_registration_service::issue(
            pool,
            who,
            &register(label, owner_team, teams, vec![]),
            AuthMode::TemperAs,
        )
        .await
        .unwrap_or_else(|e| panic!("{label}: {e:?}"))
        .client;
        (c.id, c.profile_id)
    }

    let (_, personal_bot) = issue(
        &pool,
        &m,
        "mr-p-bot",
        own_personal,
        vec![member_of(f.umbrella)],
    )
    .await;
    let (_, throwaway_bot) = issue(
        &pool,
        &m,
        "mr-t-bot",
        throwaway,
        vec![member_of(f.umbrella)],
    )
    .await;
    // Registered by the teammate, on a team max still owns: max can mint its key after he leaves.
    let (_, co_owned_bot) = issue(&pool, &p, "mr-co-bot", ops, vec![member_of(f.umbrella)]).await;
    // Registered by the teammate, rotated by max: max holds its current key.
    let (rotated_client, rotated_bot) =
        issue(&pool, &p, "mr-r-bot", ops, vec![member_of(f.umbrella)]).await;
    machine_client_service::rotate_secret(&pool, &m, rotated_client, 0, AuthMode::TemperAs)
        .await
        .expect("max rotates the teammate's bot");
    // Owned by umbrella and registered by its owner: max holds nothing of it.
    let (_, umbrellas_bot) = issue(
        &pool,
        &o,
        "mr-u-bot",
        f.umbrella,
        vec![member_of(f.umbrella)],
    )
    .await;
    // A machine whose only row in max's name is revoked; its live row is the owner's.
    let (old_client, revoked_bot) = issue(&pool, &m, "mr-old-bot", own_personal, vec![]).await;
    sqlx::query("UPDATE kb_machine_clients SET revoked_at = now(), revoked_by_profile_id = $1 WHERE id = $2")
        .bind(f.max)
        .bind(old_client)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO kb_machine_clients (client_id, label, profile_id, team_id, registered_by_profile_id) \
         VALUES ('mr-old-bot-live', 'test', $1, $2, $3)",
    )
    .bind(revoked_bot)
    .bind(f.umbrella)
    .bind(f.owner)
    .execute(&pool)
    .await
    .unwrap();
    join(&pool, f.umbrella, revoked_bot, "member").await;
    assert!(can(&pool, throwaway_bot, "write", "kb_resources", f.resource).await);

    team_service::remove_member(&pool, &o, f.umbrella, f.max)
        .await
        .expect("the owner removes max");
    for (name, bot) in [
        ("personal", personal_bot),
        ("throwaway", throwaway_bot),
        ("co-owned", co_owned_bot),
        ("rotated", rotated_bot),
    ] {
        assert!(
            !is_member(&pool, f.umbrella, bot).await,
            "{name}: max holds or can mint its key, so it leaves with him"
        );
    }
    assert!(!can(&pool, throwaway_bot, "write", "kb_resources", f.resource).await);
    for (name, bot) in [
        ("umbrella's own", umbrellas_bot),
        ("revoked row", revoked_bot),
    ] {
        assert!(
            is_member(&pool, f.umbrella, bot).await,
            "{name}: max holds no key to it"
        );
    }

    let events: Vec<(Uuid, Option<Uuid>, String)> = sqlx::query_as(
        "SELECT (e.payload->>'profile_id')::uuid, e.correlation_id, e.payload->>'via' \
           FROM kb_events e JOIN kb_event_types t ON t.id = e.event_type_id \
          WHERE t.name = 'team_membership_changed' AND e.payload->>'team_id' = $1::text \
            AND NOT (e.payload ? 'role')",
    )
    .bind(f.umbrella)
    .fetch_all(&pool)
    .await
    .unwrap();
    let corr = |p: Uuid| events.iter().find(|e| e.0 == p).map(|e| e.1);
    let maxs = corr(f.max)
        .flatten()
        .expect("max's removal carries a correlation id");
    for bot in [personal_bot, throwaway_bot, co_owned_bot, rotated_bot] {
        assert_eq!(corr(bot), Some(Some(maxs)), "correlated with max's removal");
        assert!(events.iter().any(|e| e.0 == bot && e.2 == "machine_reach"));
    }

    // The person's own leave. A machine ops owns, which max registered, leaves with him. One whose
    // key the teammate took over by rotating it before max left stays.
    let (_, ops_unrotated) = issue(&pool, &m, "mr-ops-own", ops, vec![member_of(ops)]).await;
    let (handed_client, ops_handed) =
        issue(&pool, &m, "mr-ops-kept", ops, vec![member_of(ops)]).await;
    machine_client_service::rotate_secret(&pool, &p, handed_client, 0, AuthMode::TemperAs)
        .await
        .expect("the teammate rotates it first");
    team_service::remove_member(&pool, &m, ops, f.max)
        .await
        .expect("max leaves ops");
    assert!(
        !is_member(&pool, ops, ops_unrotated).await,
        "max minted its key, so it leaves with him"
    );
    assert!(
        is_member(&pool, ops, ops_handed).await,
        "the teammate holds its key now, so it stays on ops"
    );
}

/// Route 5, **relaxed**: a grant right on the caller's own live personal team counts as held in
/// person, at the grant door and at registration. Once that personal team is soft-deleted it
/// confers nothing and the bar refuses again.
#[sqlx::test(migrator = "temper_services::MIGRATOR")]
async fn a_grant_right_on_the_callers_own_personal_team_counts_as_in_person(pool: PgPool) {
    let f = fixture(&pool).await;
    let admin = person(&pool, "mr-admin").await;
    test_support::approved_admin(&pool, admin).await;
    let ad = test_support::human_principal_for(&pool, admin).await;
    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ('t', 't') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let map: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('mr-map', $1) RETURNING id",
    )
    .bind(telos)
    .fetch_one(&pool)
    .await
    .unwrap();
    let own_personal = personal(&pool, f.max).await;
    let mut to_personal = grant("kb_cogmaps", map, "kb_teams", own_personal);
    to_personal.can_grant = true;
    access_service::grant_capability(&pool, &ad, &to_personal)
        .await
        .expect("an admin grants read+write+grant to max's personal team");

    let m = test_support::human_principal_for(&pool, f.max).await;
    let bot = machine_registration_service::issue(
        &pool,
        &m,
        &register("mr-bot", own_personal, vec![], vec![]),
        AuthMode::TemperAs,
    )
    .await
    .expect("register")
    .client
    .profile_id;
    access_service::grant_capability(&pool, &m, &grant("kb_cogmaps", map, "kb_profiles", bot))
        .await
        .expect("the grant door counts a right on the caller's own personal team as in person");
    let reach = machine_registration_service::issue(
        &pool,
        &m,
        &register(
            "mr-bot-2",
            own_personal,
            vec![],
            vec![GrantSpec {
                cogmap_id: map,
                can_write: true,
            }],
        ),
        AuthMode::TemperAs,
    )
    .await
    .expect("so does the registration door");
    assert_eq!(rows(&pool, map, reach.client.profile_id).await, 1);

    // A soft-deleted personal team confers nothing (#1274), so its row is no in-person right. Max
    // keeps a grant right on the map through umbrella, so the grant door's authority still admits
    // him and only the in-person bar decides.
    sqlx::query(
        "INSERT INTO kb_access_grants (id, subject_table, subject_id, principal_table, principal_id, \
           can_read, can_write, can_grant, granted_by_profile_id) \
         VALUES (uuid_generate_v7(), 'kb_cogmaps', $1, 'kb_teams', $2, true, true, true, $3)",
    )
    .bind(map)
    .bind(f.umbrella)
    .bind(admin)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE kb_teams SET is_active = false WHERE id = $1")
        .bind(own_personal)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM kb_access_grants WHERE subject_id = $1 AND principal_id = $2")
        .bind(map)
        .bind(bot)
        .execute(&pool)
        .await
        .unwrap();
    let refusal =
        access_service::grant_capability(&pool, &m, &grant("kb_cogmaps", map, "kb_profiles", bot))
            .await
            .expect_err("a soft-deleted personal team's right is not in person");
    assert!(matches!(refusal, ApiError::Forbidden), "{refusal:?}");
    assert_eq!(rows(&pool, map, bot).await, 0, "nothing written");

    // Live control: the umbrella-held right still admits a recipient max does not control, so the
    // refusal above is the in-person bar's.
    let stranger = person(&pool, "mr-stranger").await;
    access_service::grant_capability(
        &pool,
        &m,
        &grant("kb_cogmaps", map, "kb_profiles", stranger),
    )
    .await
    .expect("the delegated right still reaches an ordinary recipient");
}
