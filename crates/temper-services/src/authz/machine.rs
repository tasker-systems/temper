//! Machine-registration authority — may this caller act on a machine owned by this team?
//!
//! **Fails closed on `None`** (spec D2): a teamless machine (`team_id IS NULL`) is admin-only to
//! create, read, or operate. *"No team to check" must never mean "nothing to deny"* — so the
//! absent-team branch is an explicit denial arm here, not a fallthrough.

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::team::TeamRole;

use super::ScopedAuthority;
use crate::error::{ApiError, ApiResult};
use crate::services::{access_service, machine_authz::MachineAuthority, team_service};

#[async_trait]
impl ScopedAuthority for MachineAuthority {
    /// The team that owns (or will own) the machine. `None` is a real, denied case — see the
    /// module doc — not an "unknown" to be skipped over.
    type Subject = Option<Uuid>;

    async fn resolve(pool: &PgPool, caller: ProfileId, team: Option<Uuid>) -> ApiResult<Self> {
        if access_service::is_system_admin(pool, caller).await? {
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
