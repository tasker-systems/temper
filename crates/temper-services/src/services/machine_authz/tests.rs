use super::*;
use sqlx::PgPool;

use crate::test_support::{profile, team};

async fn join(pool: &PgPool, team_id: Uuid, profile: Uuid, role: &str) {
    team::set_member(pool, team_id, profile, role).await;
}

/// Configure the gating team the way the operator template does. Under D11 this no longer
/// makes anyone an admin — `is_system_admin` reads `kb_principal_governance`, so the admin
/// tests grant governance explicitly (see the `grant_governance` calls after each gating-owner
/// join). This helper is retained because `authorize` still consults the gating team's roster
/// for the ordinary team-owner arm.
///
/// The `temper-system` root team ALREADY EXISTS in a migrated database (the L0 kernel
/// migration creates it, because the canonical functions reference it by slug), so this
/// is an upsert, not an insert.
async fn configure_gating_team(pool: &PgPool) -> Uuid {
    let team: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ('temper-system', 'Temper System')
             ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name
             RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();

    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'temper-system'")
        .execute(pool)
        .await
        .unwrap();
    team
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn team_owner_is_authorized_for_their_own_team(pool: PgPool) {
    let alice = profile::insert(&pool, "authz-alice").await;
    let team = team::insert(&pool, "authz-t").await;
    join(&pool, team, alice, "owner").await;

    let authority = authorize(
        &pool,
        crate::authz::Principal::Bare(ProfileId::from(alice)),
        Some(team),
    )
    .await
    .expect("a team owner may register for their own team");
    assert_eq!(authority, MachineRegistrationAuthority::TeamOwner);
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn maintainer_and_member_are_not_authorized(pool: PgPool) {
    let team = team::insert(&pool, "authz-t2").await;
    for role in ["maintainer", "member", "watcher"] {
        let p = profile::insert(&pool, &format!("authz-{role}")).await;
        join(&pool, team, p, role).await;
        let err = authorize(
            &pool,
            crate::authz::Principal::Bare(ProfileId::from(p)),
            Some(team),
        )
        .await
        .expect_err("only an OWNER may register");
        assert!(matches!(err, ApiError::Forbidden), "{role} got {err:?}");
    }
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn non_member_is_not_authorized(pool: PgPool) {
    let stranger = profile::insert(&pool, "authz-stranger").await;
    let team = team::insert(&pool, "authz-t3").await;
    let err = authorize(
        &pool,
        crate::authz::Principal::Bare(ProfileId::from(stranger)),
        Some(team),
    )
    .await
    .expect_err("a non-member may not register");
    assert!(matches!(err, ApiError::Forbidden));
}

/// Spec D2 — the NULL owning team denies for non-admins. It must NOT fall open.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn none_team_is_admin_only(pool: PgPool) {
    let alice = profile::insert(&pool, "authz-alice2").await;
    let team = team::insert(&pool, "authz-t4").await;
    join(&pool, team, alice, "owner").await;

    let err = authorize(
        &pool,
        crate::authz::Principal::Bare(ProfileId::from(alice)),
        None,
    )
    .await
    .expect_err("a teamless registration is admin-only");
    assert!(
        matches!(err, ApiError::Forbidden),
        "NULL must deny, not fall open"
    );

    let gating = configure_gating_team(&pool).await;
    let admin = profile::insert(&pool, "authz-admin").await;
    join(&pool, gating, admin, "owner").await;
    // D11: admin-ness IS a governance grant, not gating-team ownership (`is_system_admin`).
    crate::test_support::grant_governance(&pool, admin).await;
    let authority = authorize(
        &pool,
        crate::authz::Principal::Bare(ProfileId::from(admin)),
        None,
    )
    .await
    .expect("an admin may register a teamless machine");
    assert_eq!(authority, MachineRegistrationAuthority::SystemAdmin);
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn reach_into_a_managed_team_is_allowed(pool: PgPool) {
    let alice = profile::insert(&pool, "reach-alice").await;
    let owned = team::insert(&pool, "reach-owned").await;
    let managed = team::insert(&pool, "reach-managed").await;
    join(&pool, owned, alice, "owner").await;
    join(&pool, managed, alice, "maintainer").await;

    let teams = vec![TeamSpec {
        team_id: managed,
        role: "member".to_string(),
    }];
    let principal = crate::test_support::human_principal_for(&pool, alice).await;
    let reach = authorize_registration(&pool, &principal, Some(owned), &teams, &[])
        .await
        .expect("can_manage on the target team permits reach into it");
    assert_eq!(reach.teams().len(), 1);
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn reach_into_an_unmanaged_team_is_denied(pool: PgPool) {
    let alice = profile::insert(&pool, "reach-alice2").await;
    let owned = team::insert(&pool, "reach-owned2").await;
    let foreign = team::insert(&pool, "reach-foreign").await;
    join(&pool, owned, alice, "owner").await;
    join(&pool, foreign, alice, "member").await; // member != can_manage

    let teams = vec![TeamSpec {
        team_id: foreign,
        role: "member".to_string(),
    }];
    let err = authorize_registration(
        &pool,
        &crate::test_support::human_principal_for(&pool, alice).await,
        Some(owned),
        &teams,
        &[],
    )
    .await
    .expect_err("a mere member may not grant a machine reach into that team");
    assert!(matches!(err, ApiError::Forbidden));
}

/// Every role above the ceiling. Named here rather than written out per test so the two
/// arms below can never end up testing different sets, and so a role added above `member`
/// fails these tests until it is added here too. This is the same bar the rank rule's
/// check 4 holds at every membership act (`team_membership::decide`, C5): a machine target
/// above `member` is refused for every authority, admin included.
const BARRED_ROLES: [&str; 2] = ["owner", "maintainer"];

/// Spec D4a — the escalation. A gating-team MAINTAINER clears `can_manage` on the
/// gating team but is NOT a system admin. Without the ceiling they could mint a
/// machine at role=owner on the gating team — a governing role that under
/// the pre-D11 model would have been an `is_system_admin` principal.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn cannot_mint_a_governing_role_on_the_gating_team(pool: PgPool) {
    let gating = configure_gating_team(&pool).await;
    let alice = profile::insert(&pool, "escalate-alice").await;
    let owned = team::insert(&pool, "escalate-owned").await;
    join(&pool, owned, alice, "owner").await;
    join(&pool, gating, alice, "maintainer").await;

    assert!(
        !crate::services::access_service::is_system_admin(&pool, ProfileId::from(alice))
            .await
            .unwrap(),
        "precondition: a gating-team maintainer is NOT a system admin"
    );

    for role in BARRED_ROLES {
        let teams = vec![TeamSpec {
            team_id: gating,
            role: role.to_string(),
        }];
        let err = authorize_registration(
            &pool,
            &crate::test_support::human_principal_for(&pool, alice).await,
            Some(owned),
            &teams,
            &[],
        )
        .await
        .expect_err("minting a governing machine on the gating team is an escalation");
        assert!(matches!(err, ApiError::Forbidden), "{role} got {err:?}");
    }
}

/// Plan P15, bite: machine reach's membership bar is `add_member`'s, so a gating-team owner
/// who is not a system admin cannot put a machine on the gating team even at `member` — any
/// more than they could add a person to it (R11). The same caller still reaches a team they
/// manage, so the refusal is the gating arm and not containment failing for another reason.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn non_admin_gating_owner_cannot_reach_the_gating_team_with_a_machine(pool: PgPool) {
    let gating = configure_gating_team(&pool).await;
    let alice = profile::insert(&pool, "gating-owner").await;
    let owned = team::insert(&pool, "gating-owner-team").await;
    join(&pool, owned, alice, "owner").await;
    join(&pool, gating, alice, "owner").await;
    let caller = crate::test_support::human_principal_for(&pool, alice).await;

    for role in ["member", "watcher"] {
        let teams = vec![TeamSpec {
            team_id: gating,
            role: role.to_string(),
        }];
        let err = authorize_registration(&pool, &caller, Some(owned), &teams, &[])
            .await
            .expect_err("the gating roster is admin-only, machine reach included");
        assert!(matches!(err, ApiError::Forbidden), "{role} got {err:?}");
    }

    let teams = vec![TeamSpec {
        team_id: owned,
        role: "member".to_string(),
    }];
    authorize_registration(&pool, &caller, Some(owned), &teams, &[])
        .await
        .expect("reach into a team the caller manages is still allowed");
}

/// The ceiling is not gating-team-specific — a governing role is refused on any team, and
/// on a team the caller genuinely owns, so the denial comes from the ceiling and not from
/// containment failing for an unrelated reason.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn cannot_mint_a_governing_role_on_any_team(pool: PgPool) {
    let alice = profile::insert(&pool, "escalate-alice2").await;
    let owned = team::insert(&pool, "escalate-owned2").await;
    join(&pool, owned, alice, "owner").await;

    for role in BARRED_ROLES {
        let teams = vec![TeamSpec {
            team_id: owned,
            role: role.to_string(),
        }];
        let err = authorize_registration(
            &pool,
            &crate::test_support::human_principal_for(&pool, alice).await,
            Some(owned),
            &teams,
            &[],
        )
        .await
        .expect_err("a non-admin may never mint a machine at a governing role");
        assert!(matches!(err, ApiError::Forbidden), "{role} got {err:?}");
    }
}

/// Spec D4b — the admin arm. A system admin skips containment entirely (D3), but the role
/// ceiling is not containment: no caller may mint a machine that governs a team, and on the
/// gating team, one with a governing role that under the pre-D11 model would
/// have been an `is_system_admin` principal registering further machines
/// unattended.
///
/// The mirror of `cannot_mint_a_governing_role_on_the_gating_team`, one arm over. The admin
/// arm is the one that was previously exempt for `maintainer`, so it is the load-bearing
/// half of this pair.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn admin_cannot_mint_a_governing_role(pool: PgPool) {
    let gating = configure_gating_team(&pool).await;
    let admin = profile::insert(&pool, "escalate-admin").await;
    join(&pool, gating, admin, "owner").await;
    // D11: admin-ness IS a governance grant, not gating-team ownership (`is_system_admin`).
    crate::test_support::grant_governance(&pool, admin).await;
    let foreign = team::insert(&pool, "escalate-foreign").await;

    assert!(
        crate::services::access_service::is_system_admin(&pool, ProfileId::from(admin))
            .await
            .unwrap(),
        "precondition: this caller IS a system admin, so it takes the D3 bypass arm"
    );

    for target in [gating, foreign] {
        for role in BARRED_ROLES {
            let teams = vec![TeamSpec {
                team_id: target,
                role: role.to_string(),
            }];
            let err = authorize_registration(
                &pool,
                &crate::test_support::human_principal_for(&pool, admin).await,
                None,
                &teams,
                &[],
            )
            .await
            .expect_err("the ceiling has no admin exemption");
            assert!(matches!(err, ApiError::Forbidden), "{role} got {err:?}");
        }
    }
}

/// The ceiling must not become a ban: `member` (the ceiling itself, and what `--team <ref>`
/// defaults to) and `watcher` stay permitted on BOTH arms. Without this, a "fix" that
/// refused every team spec would pass every test above.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn roles_at_or_below_the_ceiling_are_permitted_on_both_arms(pool: PgPool) {
    let gating = configure_gating_team(&pool).await;
    let admin = profile::insert(&pool, "ceiling-admin").await;
    join(&pool, gating, admin, "owner").await;
    // D11: admin-ness IS a governance grant, not gating-team ownership (`is_system_admin`).
    crate::test_support::grant_governance(&pool, admin).await;

    let alice = profile::insert(&pool, "ceiling-alice").await;
    let owned = team::insert(&pool, "ceiling-owned").await;
    join(&pool, owned, alice, "owner").await;

    for role in ["member", "watcher"] {
        let teams = vec![TeamSpec {
            team_id: owned,
            role: role.to_string(),
        }];

        let principal = crate::test_support::human_principal_for(&pool, alice).await;
        let reach = authorize_registration(&pool, &principal, Some(owned), &teams, &[])
            .await
            .unwrap_or_else(|e| panic!("team owner must still confer {role}: {e:?}"));
        assert_eq!(reach.teams().len(), 1);

        let principal = crate::test_support::human_principal_for(&pool, admin).await;
        let reach = authorize_registration(&pool, &principal, None, &teams, &[])
            .await
            .unwrap_or_else(|e| panic!("admin must still confer {role}: {e:?}"));
        assert_eq!(reach.teams().len(), 1);
    }
}

#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn grant_without_can_grant_is_denied(pool: PgPool) {
    let alice = profile::insert(&pool, "grant-alice").await;
    let owned = team::insert(&pool, "grant-owned").await;
    join(&pool, owned, alice, "owner").await;

    // The L0 kernel cogmap — Alice certainly holds no `can_grant` on it.
    let l0: Uuid = "00000000-0000-0000-0005-000000000001".parse().unwrap();
    let grants = vec![GrantSpec {
        cogmap_id: l0,
        can_write: true,
    }];

    let err = authorize_registration(
        &pool,
        &crate::test_support::human_principal_for(&pool, alice).await,
        Some(owned),
        &[],
        &grants,
    )
    .await
    .expect_err("cannot grant a machine write on a cogmap you cannot administer");
    assert!(matches!(err, ApiError::Forbidden));
}

/// A bare cogmap row — enough for the authorization predicates, which is all these tests
/// exercise. (Genesis machinery would drag in an emitter and a telos body for no gain here.)
async fn mk_cogmap(pool: &PgPool, name: &str) -> Uuid {
    let telos: Uuid = sqlx::query_scalar(
        "INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $1) RETURNING id",
    )
    .bind(format!("{name}-telos"))
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query_scalar(
        "INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(name)
    .bind(telos)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn grant_on_cogmap(
    pool: &PgPool,
    cogmap: Uuid,
    profile: Uuid,
    can_write: bool,
    can_grant: bool,
) {
    sqlx::query(
        "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id,
                                           can_read, can_write, can_delete, can_grant,
                                           granted_by_profile_id)
             VALUES ('kb_cogmaps', $1, 'kb_profiles', $2, true, $3, false, $4, $2)
             ON CONFLICT (subject_table, subject_id, principal_table, principal_id)
             DO UPDATE SET can_write = EXCLUDED.can_write, can_grant = EXCLUDED.can_grant",
    )
    .bind(cogmap)
    .bind(profile)
    .bind(can_write)
    .bind(can_grant)
    .execute(pool)
    .await
    .unwrap();
}

/// Attenuation parity with the human path (5b.3). The machine path must not **launder** a
/// capability the caller does not hold: a `read+grant`-without-write holder who could provision
/// a machine with `can_write` would end up commanding a principal holding write they can never
/// hold themselves — and the ledger would record the machine, not them, as the grantee.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn cannot_launder_cogmap_write_to_a_machine_without_holding_write(pool: PgPool) {
    let alice = profile::insert(&pool, "launder-alice").await;
    let owned = team::insert(&pool, "launder-owned").await;
    join(&pool, owned, alice, "owner").await;
    let cogmap = mk_cogmap(&pool, "launder-map").await;
    grant_on_cogmap(&pool, cogmap, alice, false, true).await; // read + grant, NOT write

    // Non-vacuity: Alice really can administer grants on this map, and really lacks write —
    // so a denial below is attenuation, not her failing the grant bar for an unrelated reason.
    assert!(
        access_service::profile_can_grant(&pool, ProfileId::from(alice), "kb_cogmaps", cogmap)
            .await
            .unwrap(),
        "fixture must confer can_grant"
    );

    let grants = vec![GrantSpec {
        cogmap_id: cogmap,
        can_write: true,
    }];
    let err = authorize_registration(
        &pool,
        &crate::test_support::human_principal_for(&pool, alice).await,
        Some(owned),
        &[],
        &grants,
    )
    .await
    .expect_err("a caller without write must not confer write to a machine");
    assert!(matches!(err, ApiError::Forbidden), "got {err:?}");
}

/// Attenuation BOUNDS delegation, it does not forbid it — the companion to the test above, so a
/// fix cannot pass by simply refusing every machine grant.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn a_write_holder_may_still_delegate_write_to_a_machine(pool: PgPool) {
    let alice = profile::insert(&pool, "delegator-alice").await;
    let owned = team::insert(&pool, "delegator-owned").await;
    join(&pool, owned, alice, "owner").await;
    let cogmap = mk_cogmap(&pool, "delegator-map").await;
    grant_on_cogmap(&pool, cogmap, alice, true, true).await; // read + write + grant

    let grants = vec![GrantSpec {
        cogmap_id: cogmap,
        can_write: true,
    }];
    let principal = crate::test_support::human_principal_for(&pool, alice).await;
    let reach = authorize_registration(&pool, &principal, Some(owned), &[], &grants)
        .await
        .expect("a holder of write may delegate write to a machine");
    assert_eq!(reach.grants().len(), 1);
}

/// L0/gating parity with the human path (5b.4). `grant_authority` denies a non-admin `can_grant`
/// holder on the kernel outright; the machine path must too, or the same person confers kernel
/// write to a proxy they control.
///
/// This test previously asserted the OPPOSITE, as `grant_with_can_grant_is_allowed` — that a
/// non-admin `can_grant` holder MAY delegate kernel write to a machine. It encoded the pre-5b
/// policy, exactly as `can_grant_holder_can_delegate` did on the human path. The assertion
/// inverts deliberately.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn cannot_confer_kernel_write_to_a_machine_without_being_admin(pool: PgPool) {
    let alice = profile::insert(&pool, "grant-alice2").await;
    let owned = team::insert(&pool, "grant-owned2").await;
    join(&pool, owned, alice, "owner").await;

    let l0: Uuid = "00000000-0000-0000-0005-000000000001".parse().unwrap();
    grant_on_cogmap(&pool, l0, alice, true, true).await;

    // Non-vacuity: she holds BOTH can_grant and can_write on L0, so attenuation alone would
    // pass her. Only the L0/gating admin guard can produce the denial below.
    assert!(
        access_service::profile_can_grant(&pool, ProfileId::from(alice), "kb_cogmaps", l0)
            .await
            .unwrap(),
        "fixture must confer can_grant on the kernel"
    );

    let grants = vec![GrantSpec {
        cogmap_id: l0,
        can_write: true,
    }];
    let err = authorize_registration(
        &pool,
        &crate::test_support::human_principal_for(&pool, alice).await,
        Some(owned),
        &[],
        &grants,
    )
    .await
    .expect_err("the L0 kernel stays admin-only on the grant axis, machines included");
    assert!(matches!(err, ApiError::Forbidden), "got {err:?}");
}

/// Spec D5 — the admin bypass survives, unchecked (Phase A D5).
///
/// **Amended 2026-07-18 for D4b, again 2026-07-20 for the ceiling:** this asserted reach
/// breadth using `role = "owner"`, then `"maintainer"`, both now barred on every arm. The role
/// was always incidental to what the test is *for* — the claim is that an admin may reach a
/// team it is not a member of and a cogmap it holds nothing on, neither of which a non-admin
/// could do. Both are still asserted here, at a role the ceiling permits;
/// `admin_cannot_mint_a_governing_role` covers the carve-out.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn system_admin_reach_is_unchecked(pool: PgPool) {
    let gating = configure_gating_team(&pool).await;
    let admin = profile::insert(&pool, "admin-unchecked").await;
    join(&pool, gating, admin, "owner").await;
    // D11: admin-ness IS a governance grant, not gating-team ownership (`is_system_admin`).
    crate::test_support::grant_governance(&pool, admin).await;

    let foreign = team::insert(&pool, "admin-foreign").await;
    let l0: Uuid = "00000000-0000-0000-0005-000000000001".parse().unwrap();

    let teams = vec![TeamSpec {
        team_id: foreign,
        role: "member".to_string(),
    }];
    let grants = vec![GrantSpec {
        cogmap_id: l0,
        can_write: true,
    }];

    let principal = crate::test_support::human_principal_for(&pool, admin).await;
    let reach = authorize_registration(&pool, &principal, None, &teams, &grants)
        .await
        .expect("a system admin may grant any reach (Phase A D5)");
    assert_eq!(reach.teams().len(), 1);
    assert_eq!(reach.grants().len(), 1);
}

/// An unknown role is a 400 (BadRequest), not a 500 from the downstream enum cast — and it
/// is rejected on the TEAM-OWNER path.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn unknown_role_is_a_bad_request_for_a_team_owner(pool: PgPool) {
    let alice = profile::insert(&pool, "role-alice").await;
    let owned = team::insert(&pool, "role-owned").await;
    join(&pool, owned, alice, "owner").await;

    let teams = vec![TeamSpec {
        team_id: owned,
        role: "membr".to_string(), // typo — not a real role
    }];
    let err = authorize_registration(
        &pool,
        &crate::test_support::human_principal_for(&pool, alice).await,
        Some(owned),
        &teams,
        &[],
    )
    .await
    .expect_err("an unknown role must not reach the enum cast");
    assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
}

/// The same 400 holds on the ADMIN path — a malformed payload is malformed regardless of
/// who sends it (the D5 bypass is about authority, not input validation).
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn unknown_role_is_a_bad_request_for_an_admin(pool: PgPool) {
    let gating = configure_gating_team(&pool).await;
    let admin = profile::insert(&pool, "role-admin").await;
    join(&pool, gating, admin, "owner").await;
    // D11: admin-ness IS a governance grant, not gating-team ownership (`is_system_admin`).
    crate::test_support::grant_governance(&pool, admin).await;
    let target = team::insert(&pool, "role-target").await;

    let teams = vec![TeamSpec {
        team_id: target,
        role: "MEMBER".to_string(), // wrong case — the enum is snake_case
    }];
    let err = authorize_registration(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin).await,
        None,
        &teams,
        &[],
    )
    .await
    .expect_err("an admin's unknown role is still a 400");
    assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
}

/// Spec D5 — `list` is scoped in SQL by the same authority `authorize` resolves. The teamless
/// row is the one that matters: it must be invisible to a team owner, not fall open.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn list_is_scoped_to_owned_teams(pool: PgPool) {
    use crate::services::machine_client_service;

    let gating = configure_gating_team(&pool).await;
    let admin = profile::insert(&pool, "list-admin").await;
    join(&pool, gating, admin, "owner").await;
    // D11: admin-ness IS a governance grant, not gating-team ownership (`is_system_admin`).
    crate::test_support::grant_governance(&pool, admin).await;

    let alice = profile::insert(&pool, "list-alice").await;
    let alice_team = team::insert(&pool, "list-alice-team").await;
    join(&pool, alice_team, alice, "owner").await;

    let other_team = team::insert(&pool, "list-other-team").await;

    // Three rows: one owned by Alice's team, one by another team, one teamless.
    for (client_id, team) in [
        ("list-mine", Some(alice_team)),
        ("list-theirs", Some(other_team)),
        ("list-teamless", None),
    ] {
        let agent = profile::insert(&pool, client_id).await;
        sqlx::query(
            "INSERT INTO kb_machine_clients
                     (client_id, issuer, label, profile_id, team_id, registered_by_profile_id)
                 VALUES ($1, 'temper', $1, $2, $3, $4)",
        )
        .bind(client_id)
        .bind(agent)
        .bind(team)
        .bind(admin)
        .execute(&pool)
        .await
        .unwrap();
    }

    let mine = machine_client_service::list(
        &pool,
        &crate::test_support::human_principal_for(&pool, alice).await,
        false,
    )
    .await
    .unwrap();
    let ids: Vec<&str> = mine.iter().map(|c| c.client_id.as_str()).collect();
    assert_eq!(
        ids,
        ["list-mine"],
        "a team owner sees only their team's machines"
    );

    let all = machine_client_service::list(
        &pool,
        &crate::test_support::human_principal_for(&pool, admin).await,
        false,
    )
    .await
    .unwrap();
    assert_eq!(all.len(), 3, "an admin sees every row, including teamless");
}
