//! Machine-registration authority — may this caller act on a machine owned by this team? — and its
//! per-row twin, [`MachineClientControlAuthority`], for acts on an existing machine client.
//!
//! **Fails closed on `None`** (spec D2): a teamless machine (`team_id IS NULL`) is admin-only to
//! create, read, or operate. *"No team to check" must never mean "nothing to deny"* — so the
//! absent-team branch is an explicit denial arm here, not a fallthrough.

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::team::TeamRole;

use super::{Principal, ScopedAuthority};
use crate::error::{ApiError, ApiResult};
use crate::services::machine_client_service::{self, MACHINE_CLIENT_REFUSAL};
use crate::services::{machine_authz::MachineRegistrationAuthority, team_service};

#[async_trait]
impl ScopedAuthority for MachineRegistrationAuthority {
    /// The team that owns (or will own) the machine. `None` is a real, denied case — see the
    /// module doc — not an "unknown" to be skipped over.
    type Subject = Option<Uuid>;

    async fn resolve(
        pool: &PgPool,
        principal: Principal<'_>,
        team: Option<Uuid>,
    ) -> ApiResult<Self> {
        let caller = principal.profile_id();
        // The admin arm stops at a personal team (task `01a191b4`): a machine or connection owned by
        // one is its owner's alone. A teamless machine stays admin-only (spec D2).
        let personal = match team {
            Some(team_id) => team_service::personal_of(pool, team_id)
                .await?
                .is_some_and(|owner| owner != caller),
            None => false,
        };
        if !personal && principal.system_admin(pool).await?.is_some() {
            return Ok(MachineRegistrationAuthority::SystemAdmin);
        }

        let Some(team_id) = team else {
            return Ok(MachineRegistrationAuthority::None);
        };

        Ok(
            match team_service::role_on_team(pool, team_id, caller).await? {
                Some(TeamRole::Owner) => MachineRegistrationAuthority::TeamOwner,
                _ => MachineRegistrationAuthority::None,
            },
        )
    }

    fn is_denial(&self) -> bool {
        matches!(self, MachineRegistrationAuthority::None)
    }

    fn denial() -> ApiError {
        ApiError::Forbidden
    }
}

/// May this caller act on **this machine client** — the per-row gate behind get, revoke and
/// secret rotation?
///
/// [`MachineRegistrationAuthority`] is keyed on a team, which is right for registration (the caller
/// names the team that will own the machine, and there is no row yet). A per-row act is keyed on
/// the row: the owning team is read from it, never supplied, and the policy is then
/// `MachineRegistrationAuthority`'s, **called, not restated**.
///
/// **Its refusal is `NotFound`, not `Forbidden`, and that is the reason it exists.** Keyed on the
/// team, a refusal arrived only after the row was loaded, so a missing id answered 404 and a
/// present-but-forbidden one 403 — an existence oracle any approved bearer could probe. Here a
/// denial renders `machine_client_service::MACHINE_CLIENT_REFUSAL`, the same sentence the row
/// lookup renders for a missing id, so the two are indistinguishable. Nothing is withheld from a
/// caller who controls the row: they can `GET` it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MachineClientControlAuthority {
    /// A system admin.
    SystemAdmin,
    /// Owner of the team that owns this machine client.
    OwnerOfOwningTeam,
    /// Neither — including a teamless machine, which fails closed (spec D2).
    None,
}

#[async_trait]
impl ScopedAuthority for MachineClientControlAuthority {
    /// The machine client's own id. Its owning team is derived from the row.
    type Subject = Uuid;

    async fn resolve(
        pool: &PgPool,
        caller: Principal<'_>,
        machine_client: Uuid,
    ) -> ApiResult<Self> {
        // A missing row is `NotFound(MACHINE_CLIENT_REFUSAL)` from the lookup itself — the same
        // refusal `denial` renders below, which is what closes the oracle.
        let client = machine_client_service::get(pool, machine_client).await?;

        Ok(
            match <MachineRegistrationAuthority as ScopedAuthority>::resolve(
                pool,
                caller,
                client.team_id,
            )
            .await?
            {
                MachineRegistrationAuthority::SystemAdmin => {
                    MachineClientControlAuthority::SystemAdmin
                }
                MachineRegistrationAuthority::TeamOwner => {
                    MachineClientControlAuthority::OwnerOfOwningTeam
                }
                MachineRegistrationAuthority::None => {
                    super::log_concealed_refusal(caller, "machine_client");
                    MachineClientControlAuthority::None
                }
            },
        )
    }

    fn is_denial(&self) -> bool {
        matches!(self, MachineClientControlAuthority::None)
    }

    /// `NotFound`, byte-identical to the missing-row refusal — see the type's doc.
    fn denial() -> ApiError {
        ApiError::NotFound(MACHINE_CLIENT_REFUSAL.to_string())
    }
}

#[cfg(all(test, feature = "test-db"))]
mod personal_space_tests {
    //! Task 01a191b4: a system admin's arm stops at someone else's personal team, for machines and
    //! connections (this authority) and for subscriptions (`SubscriptionAuthority`), and keeps the
    //! admin's own personal team and every shared team.

    use super::*;
    use crate::authz::SubscriptionAuthority;
    use temper_core::types::ids::ProfileId;

    async fn person(pool: &PgPool, handle: &str) -> (ProfileId, Uuid) {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
        )
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap();
        let personal: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE personal_of = $1")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap();
        (ProfileId::from(id), personal)
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn the_admin_arm_stops_at_someone_elses_personal_team(pool: PgPool) {
        let (admin, admins_own) = person(&pool, "admin").await;
        let (_owner, theirs) = person(&pool, "owner").await;
        crate::test_support::grant_governance(&pool, *admin).await;
        let shared: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_teams (slug, name) VALUES ('acme', 'acme') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let p = || Principal::Bare(admin);

        assert_eq!(
            MachineRegistrationAuthority::resolve(&pool, p(), Some(theirs))
                .await
                .unwrap(),
            MachineRegistrationAuthority::None
        );
        assert_eq!(
            SubscriptionAuthority::resolve(&pool, p(), theirs)
                .await
                .unwrap(),
            SubscriptionAuthority::None
        );
        for team in [Some(shared), Some(admins_own), None] {
            assert_eq!(
                MachineRegistrationAuthority::resolve(&pool, p(), team)
                    .await
                    .unwrap(),
                MachineRegistrationAuthority::SystemAdmin,
                "{team:?}"
            );
        }
        assert_eq!(
            SubscriptionAuthority::resolve(&pool, p(), shared)
                .await
                .unwrap(),
            SubscriptionAuthority::SystemAdmin
        );
    }
}
