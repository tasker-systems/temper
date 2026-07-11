//! Authorization for machine-client registration (G3 Phase B2).
//!
//! Two things live here, and the separation is the point:
//!
//! 1. **Who may register** — [`authorize`]: a system admin, or the OWNER of the team that
//!    will own the machine. `is_system_admin` already *is* ownership of the gating team, so
//!    this is one concept keyed on two teams, not two concepts.
//! 2. **What reach they may confer** — [`AuthorizedReach`]: a value that only this module can
//!    construct. `apply_reach` takes it instead of raw specs, so reach cannot be applied
//!    without having been authorized. The invariant is enforced by the type, not by a comment
//!    — which is what the Phase A comment on `apply_reach` asked for and could not get.
//!
//! The containment bar is the *human* bar, reached by CALLING the human predicates rather
//! than restating them: teams need `can_manage` (what `add_member` requires) and a non-`Owner`
//! role (what `add_member` refuses, per D7); grants need `can_grant` (what `grant_capability`
//! requires of a non-admin). Tighten the human surface and the machine surface tightens with
//! it — there is no second copy of the policy to drift.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::machine::{GrantSpec, TeamSpec};
use temper_core::types::team::TeamRole;

use crate::error::{ApiError, ApiResult};
use crate::services::{access_service, team_service};

/// The caller's authority over a machine registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MachineAuthority {
    /// Owner of the gating team. Full, unchecked reach (Phase A D5).
    SystemAdmin,
    /// Owner of the team that owns (or will own) the machine. Reach is contained.
    TeamOwner,
}

/// Resolve who the caller is with respect to a registration owned by `team`.
///
/// **Fails closed on `None`** (spec D2): a teamless machine (`team_id IS NULL`) is admin-only
/// to create, read, or operate. "No team to check" must never mean "nothing to deny".
pub(crate) async fn authorize(
    pool: &PgPool,
    caller: ProfileId,
    team: Option<Uuid>,
) -> ApiResult<MachineAuthority> {
    if access_service::is_system_admin(pool, caller).await? {
        return Ok(MachineAuthority::SystemAdmin);
    }

    let Some(team_id) = team else {
        return Err(ApiError::Forbidden);
    };

    match team_service::role_on_team(pool, team_id, caller).await? {
        Some(TeamRole::Owner) => Ok(MachineAuthority::TeamOwner),
        _ => Err(ApiError::Forbidden),
    }
}

/// Reach that has been authorized against a caller's authority (spec D3).
///
/// The fields are private to this module and there is no public constructor, so an
/// `AuthorizedReach` can only come from [`authorize_registration`]. `apply_reach` takes this
/// type, which makes the unchecked path *unrepresentable* rather than merely discouraged.
#[derive(Debug)]
pub(crate) struct AuthorizedReach<'a> {
    teams: &'a [TeamSpec],
    grants: &'a [GrantSpec],
}

impl<'a> AuthorizedReach<'a> {
    pub(crate) fn teams(&self) -> &'a [TeamSpec] {
        self.teams
    }

    pub(crate) fn grants(&self) -> &'a [GrantSpec] {
        self.grants
    }
}

/// Authorize a registration and the reach it asks for, in that order.
///
/// A system admin gets the Phase A D5 bypass — named here, so the bypass is visible at this
/// call site instead of being implicit in the absence of a check.
pub(crate) async fn authorize_registration<'a>(
    pool: &PgPool,
    caller: ProfileId,
    team: Option<Uuid>,
    teams: &'a [TeamSpec],
    grants: &'a [GrantSpec],
) -> ApiResult<AuthorizedReach<'a>> {
    let authority = authorize(pool, caller, team).await?;

    // The caller is authorized; now the payload must be well-formed. An unknown role would
    // otherwise fail the `::team_role` enum cast deep inside `apply_reach`'s transaction and
    // surface as a 500 — a malformed request is a 400. Runs on both paths (an admin's bad role
    // is a 400 too). Validating AFTER authorize means an unauthorized caller still gets 403,
    // not a hint about their payload.
    for spec in teams {
        parse_team_role(&spec.role)?;
    }

    match authority {
        // Phase A D5: a system admin may confer any reach on a machine.
        MachineAuthority::SystemAdmin => Ok(AuthorizedReach { teams, grants }),
        MachineAuthority::TeamOwner => {
            contain_reach(pool, caller, teams, grants).await?;
            Ok(AuthorizedReach { teams, grants })
        }
    }
}

/// Parse a wire role string into a `TeamRole`, the enum's own serde as the single source of
/// truth. Unknown roles are a 400 here rather than a 500 from the downstream enum cast.
fn parse_team_role(role: &str) -> ApiResult<TeamRole> {
    serde_json::from_value::<TeamRole>(serde_json::Value::String(role.to_string()))
        .map_err(|_| ApiError::BadRequest(format!("unknown team role '{role}'")))
}

/// The non-admin containment bar. Every check calls an existing human-surface predicate.
async fn contain_reach(
    pool: &PgPool,
    caller: ProfileId,
    teams: &[TeamSpec],
    grants: &[GrantSpec],
) -> ApiResult<()> {
    for spec in teams {
        // D4a — the ROLE bar. `can_manage` admits maintainers, and a gating-team maintainer
        // is not a system admin; without this, they could mint a machine at role=owner on the
        // gating team and thereby manufacture an `is_system_admin` principal. `apply_reach`'s
        // raw `ON CONFLICT DO UPDATE SET role` never passes through `add_member`, so D7's
        // guard does not cover this write site. Any gate on granting a role must check the
        // role being granted, not merely the grantor's access to the team.
        if spec.role.eq_ignore_ascii_case("owner") {
            return Err(ApiError::Forbidden);
        }

        // D4 — the membership bar: exactly what `add_member` requires of a human.
        match team_service::role_on_team(pool, spec.team_id, caller).await? {
            Some(role) if team_service::can_manage(role) => {}
            _ => return Err(ApiError::Forbidden),
        }
    }

    for grant in grants {
        // D4 — exactly what `grant_capability` requires of a non-admin: `can_grant` on the
        // subject. (`can_administer_grant` is this OR `is_system_admin`; the admin case has
        // already returned above.)
        if !access_service::profile_can_grant(pool, caller, "kb_cogmaps", grant.cogmap_id).await? {
            return Err(ApiError::Forbidden);
        }
    }

    Ok(())
}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use sqlx::PgPool;

    async fn mk_profile(pool: &PgPool, handle: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
        )
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn mk_team(pool: &PgPool, slug: &str) -> Uuid {
        sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
            .bind(slug)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn join(pool: &PgPool, team: Uuid, profile: Uuid, role: &str) {
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role)
             VALUES ($1, $2, $3::text::team_role)
             ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
        )
        .bind(team)
        .bind(profile)
        .bind(role)
        .execute(pool)
        .await
        .unwrap();
    }

    /// A fresh DB seeds `access_mode='open'` with `gating_team_slug` NULL, so nobody is a
    /// system admin until a gating team is configured. Configure it the way the operator
    /// template does — WITHOUT flipping access_mode (prod runs 'open'; the admin check is
    /// load-bearing precisely because the router gate admits everyone there).
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
        let alice = mk_profile(&pool, "authz-alice").await;
        let team = mk_team(&pool, "authz-t").await;
        join(&pool, team, alice, "owner").await;

        let authority = authorize(&pool, ProfileId::from(alice), Some(team))
            .await
            .expect("a team owner may register for their own team");
        assert_eq!(authority, MachineAuthority::TeamOwner);
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn maintainer_and_member_are_not_authorized(pool: PgPool) {
        let team = mk_team(&pool, "authz-t2").await;
        for role in ["maintainer", "member", "watcher"] {
            let p = mk_profile(&pool, &format!("authz-{role}")).await;
            join(&pool, team, p, role).await;
            let err = authorize(&pool, ProfileId::from(p), Some(team))
                .await
                .expect_err("only an OWNER may register");
            assert!(matches!(err, ApiError::Forbidden), "{role} got {err:?}");
        }
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn non_member_is_not_authorized(pool: PgPool) {
        let stranger = mk_profile(&pool, "authz-stranger").await;
        let team = mk_team(&pool, "authz-t3").await;
        let err = authorize(&pool, ProfileId::from(stranger), Some(team))
            .await
            .expect_err("a non-member may not register");
        assert!(matches!(err, ApiError::Forbidden));
    }

    /// Spec D2 — the NULL owning team denies for non-admins. It must NOT fall open.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn none_team_is_admin_only(pool: PgPool) {
        let alice = mk_profile(&pool, "authz-alice2").await;
        let team = mk_team(&pool, "authz-t4").await;
        join(&pool, team, alice, "owner").await;

        let err = authorize(&pool, ProfileId::from(alice), None)
            .await
            .expect_err("a teamless registration is admin-only");
        assert!(
            matches!(err, ApiError::Forbidden),
            "NULL must deny, not fall open"
        );

        let gating = configure_gating_team(&pool).await;
        let admin = mk_profile(&pool, "authz-admin").await;
        join(&pool, gating, admin, "owner").await;
        let authority = authorize(&pool, ProfileId::from(admin), None)
            .await
            .expect("an admin may register a teamless machine");
        assert_eq!(authority, MachineAuthority::SystemAdmin);
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn reach_into_a_managed_team_is_allowed(pool: PgPool) {
        let alice = mk_profile(&pool, "reach-alice").await;
        let owned = mk_team(&pool, "reach-owned").await;
        let managed = mk_team(&pool, "reach-managed").await;
        join(&pool, owned, alice, "owner").await;
        join(&pool, managed, alice, "maintainer").await;

        let teams = vec![TeamSpec {
            team_id: managed,
            role: "member".to_string(),
        }];
        let reach = authorize_registration(&pool, ProfileId::from(alice), Some(owned), &teams, &[])
            .await
            .expect("can_manage on the target team permits reach into it");
        assert_eq!(reach.teams().len(), 1);
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn reach_into_an_unmanaged_team_is_denied(pool: PgPool) {
        let alice = mk_profile(&pool, "reach-alice2").await;
        let owned = mk_team(&pool, "reach-owned2").await;
        let foreign = mk_team(&pool, "reach-foreign").await;
        join(&pool, owned, alice, "owner").await;
        join(&pool, foreign, alice, "member").await; // member != can_manage

        let teams = vec![TeamSpec {
            team_id: foreign,
            role: "member".to_string(),
        }];
        let err = authorize_registration(&pool, ProfileId::from(alice), Some(owned), &teams, &[])
            .await
            .expect_err("a mere member may not grant a machine reach into that team");
        assert!(matches!(err, ApiError::Forbidden));
    }

    /// Spec D4a — the escalation. A gating-team MAINTAINER clears `can_manage` on the
    /// gating team but is NOT a system admin. Without the role bar they could mint a
    /// machine at role=owner on the gating team — an `is_system_admin` principal.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn cannot_mint_owner_role_on_the_gating_team(pool: PgPool) {
        let gating = configure_gating_team(&pool).await;
        let alice = mk_profile(&pool, "escalate-alice").await;
        let owned = mk_team(&pool, "escalate-owned").await;
        join(&pool, owned, alice, "owner").await;
        join(&pool, gating, alice, "maintainer").await;

        assert!(
            !crate::services::access_service::is_system_admin(&pool, ProfileId::from(alice))
                .await
                .unwrap(),
            "precondition: a gating-team maintainer is NOT a system admin"
        );

        let teams = vec![TeamSpec {
            team_id: gating,
            role: "owner".to_string(),
        }];
        let err = authorize_registration(&pool, ProfileId::from(alice), Some(owned), &teams, &[])
            .await
            .expect_err("minting a machine as gating-team OWNER is an escalation to system admin");
        assert!(matches!(err, ApiError::Forbidden), "got {err:?}");
    }

    /// The role bar is not gating-team-specific — `owner` is refused on any team.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn cannot_mint_owner_role_on_any_team(pool: PgPool) {
        let alice = mk_profile(&pool, "escalate-alice2").await;
        let owned = mk_team(&pool, "escalate-owned2").await;
        join(&pool, owned, alice, "owner").await;

        let teams = vec![TeamSpec {
            team_id: owned,
            role: "owner".to_string(),
        }];
        let err = authorize_registration(&pool, ProfileId::from(alice), Some(owned), &teams, &[])
            .await
            .expect_err("a non-admin may never mint a machine at role=owner");
        assert!(matches!(err, ApiError::Forbidden));
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn grant_without_can_grant_is_denied(pool: PgPool) {
        let alice = mk_profile(&pool, "grant-alice").await;
        let owned = mk_team(&pool, "grant-owned").await;
        join(&pool, owned, alice, "owner").await;

        // The L0 kernel cogmap — Alice certainly holds no `can_grant` on it.
        let l0: Uuid = "00000000-0000-0000-0005-000000000001".parse().unwrap();
        let grants = vec![GrantSpec {
            cogmap_id: l0,
            can_write: true,
        }];

        let err = authorize_registration(&pool, ProfileId::from(alice), Some(owned), &[], &grants)
            .await
            .expect_err("cannot grant a machine write on a cogmap you cannot administer");
        assert!(matches!(err, ApiError::Forbidden));
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn grant_with_can_grant_is_allowed(pool: PgPool) {
        let alice = mk_profile(&pool, "grant-alice2").await;
        let owned = mk_team(&pool, "grant-owned2").await;
        join(&pool, owned, alice, "owner").await;

        let l0: Uuid = "00000000-0000-0000-0005-000000000001".parse().unwrap();
        sqlx::query(
            "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id,
                                           can_read, can_write, can_grant, granted_by_profile_id)
             VALUES ('kb_cogmaps', $1, 'kb_profiles', $2, true, true, true, $2)
             ON CONFLICT (subject_table, subject_id, principal_table, principal_id)
             DO UPDATE SET can_grant = true",
        )
        .bind(l0)
        .bind(alice)
        .execute(&pool)
        .await
        .unwrap();

        let grants = vec![GrantSpec {
            cogmap_id: l0,
            can_write: true,
        }];
        let reach =
            authorize_registration(&pool, ProfileId::from(alice), Some(owned), &[], &grants)
                .await
                .expect("a can_grant holder may delegate to a machine");
        assert_eq!(reach.grants().len(), 1);
    }

    /// Spec D5 — the admin bypass survives, unchecked (Phase A D5).
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn system_admin_reach_is_unchecked(pool: PgPool) {
        let gating = configure_gating_team(&pool).await;
        let admin = mk_profile(&pool, "admin-unchecked").await;
        join(&pool, gating, admin, "owner").await;

        let foreign = mk_team(&pool, "admin-foreign").await;
        let l0: Uuid = "00000000-0000-0000-0005-000000000001".parse().unwrap();

        let teams = vec![TeamSpec {
            team_id: foreign,
            role: "owner".to_string(),
        }];
        let grants = vec![GrantSpec {
            cogmap_id: l0,
            can_write: true,
        }];

        let reach = authorize_registration(&pool, ProfileId::from(admin), None, &teams, &grants)
            .await
            .expect("a system admin may grant any reach (Phase A D5)");
        assert_eq!(reach.teams().len(), 1);
        assert_eq!(reach.grants().len(), 1);
    }

    /// An unknown role is a 400 (BadRequest), not a 500 from the downstream enum cast — and it
    /// is rejected on the TEAM-OWNER path.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn unknown_role_is_a_bad_request_for_a_team_owner(pool: PgPool) {
        let alice = mk_profile(&pool, "role-alice").await;
        let owned = mk_team(&pool, "role-owned").await;
        join(&pool, owned, alice, "owner").await;

        let teams = vec![TeamSpec {
            team_id: owned,
            role: "membr".to_string(), // typo — not a real role
        }];
        let err = authorize_registration(&pool, ProfileId::from(alice), Some(owned), &teams, &[])
            .await
            .expect_err("an unknown role must not reach the enum cast");
        assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");
    }

    /// The same 400 holds on the ADMIN path — a malformed payload is malformed regardless of
    /// who sends it (the D5 bypass is about authority, not input validation).
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn unknown_role_is_a_bad_request_for_an_admin(pool: PgPool) {
        let gating = configure_gating_team(&pool).await;
        let admin = mk_profile(&pool, "role-admin").await;
        join(&pool, gating, admin, "owner").await;
        let target = mk_team(&pool, "role-target").await;

        let teams = vec![TeamSpec {
            team_id: target,
            role: "MEMBER".to_string(), // wrong case — the enum is snake_case
        }];
        let err = authorize_registration(&pool, ProfileId::from(admin), None, &teams, &[])
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
        let admin = mk_profile(&pool, "list-admin").await;
        join(&pool, gating, admin, "owner").await;

        let alice = mk_profile(&pool, "list-alice").await;
        let alice_team = mk_team(&pool, "list-alice-team").await;
        join(&pool, alice_team, alice, "owner").await;

        let other_team = mk_team(&pool, "list-other-team").await;

        // Three rows: one owned by Alice's team, one by another team, one teamless.
        for (client_id, team) in [
            ("list-mine", Some(alice_team)),
            ("list-theirs", Some(other_team)),
            ("list-teamless", None),
        ] {
            let agent = mk_profile(&pool, client_id).await;
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

        let mine = machine_client_service::list(&pool, ProfileId::from(alice), false)
            .await
            .unwrap();
        let ids: Vec<&str> = mine.iter().map(|c| c.client_id.as_str()).collect();
        assert_eq!(
            ids,
            ["list-mine"],
            "a team owner sees only their team's machines"
        );

        let all = machine_client_service::list(&pool, ProfileId::from(admin), false)
            .await
            .unwrap();
        assert_eq!(all.len(), 3, "an admin sees every row, including teamless");
    }
}
