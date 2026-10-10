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
use crate::services::{machine_authz::MachineAuthority, team_service};

#[async_trait]
impl ScopedAuthority for MachineAuthority {
    /// The team that owns (or will own) the machine. `None` is a real, denied case — see the
    /// module doc — not an "unknown" to be skipped over.
    type Subject = Option<Uuid>;

    async fn resolve(pool: &PgPool, caller: Principal<'_>, team: Option<Uuid>) -> ApiResult<Self> {
        let principal = caller;
        let caller = principal.profile_id();
        if principal.system_admin(pool).await?.is_some() {
            return Ok(MachineAuthority::SystemAdmin);
        }

        let Some(team_id) = team else {
            return Ok(MachineAuthority::None);
        };

        Ok(
            match team_service::role_on_team(pool, team_id, caller).await? {
                Some(TeamRole::Owner) => MachineAuthority::TeamOwner,
                _ => MachineAuthority::None,
            },
        )
    }

    fn is_denial(&self) -> bool {
        matches!(self, MachineAuthority::None)
    }

    fn denial() -> ApiError {
        ApiError::Forbidden
    }
}

/// May this caller act on **this machine client** — the per-row gate behind get, revoke and
/// secret rotation?
///
/// [`MachineAuthority`] is keyed on a team, which is right for registration (the caller names the
/// team that will own the machine, and there is no row yet). A per-row act is keyed on the row:
/// the owning team is read from it, never supplied, and the policy is then `MachineAuthority`'s,
/// **called, not restated**.
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
            match <MachineAuthority as ScopedAuthority>::resolve(pool, caller, client.team_id)
                .await?
            {
                MachineAuthority::SystemAdmin => MachineClientControlAuthority::SystemAdmin,
                MachineAuthority::TeamOwner => MachineClientControlAuthority::OwnerOfOwningTeam,
                MachineAuthority::None => {
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
