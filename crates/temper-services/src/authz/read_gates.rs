//! Read gates whose refusal is `NotFound` rather than `Forbidden`.
//!
//! These are the reason `ScopedAuthority::denial` exists. For both domains here the *existence* of
//! the subject is itself the secret, so refusing with `Forbidden` would confirm what the refusal is
//! meant to withhold. That is a deliberate decision at each site, carried onto the impls below so a
//! later "let's make the denials consistent" pass has to read the reason before changing it.

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;

use super::ScopedAuthority;
use crate::error::{ApiError, ApiResult};
use crate::services::{access_service, team_service};

/// Who may read a team's detail (row + member roster).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TeamReadAuthority {
    /// A member of the team, at any role.
    Member,
    /// A system admin, who reads every team.
    SystemAdmin,
    /// Not a member and not an admin.
    None,
}

#[async_trait]
impl ScopedAuthority for TeamReadAuthority {
    type Subject = Uuid;

    async fn resolve(pool: &PgPool, caller: ProfileId, team_id: Uuid) -> ApiResult<Self> {
        // Membership first, matching the order this gate has always probed in: the common reader
        // is a member, and asking `is_system_admin` first would add a query to every one of them.
        if team_service::role_on_team(pool, team_id, caller)
            .await?
            .is_some()
        {
            return Ok(TeamReadAuthority::Member);
        }
        Ok(if access_service::is_system_admin(pool, caller).await? {
            TeamReadAuthority::SystemAdmin
        } else {
            TeamReadAuthority::None
        })
    }

    fn is_denial(&self) -> bool {
        matches!(self, TeamReadAuthority::None)
    }

    /// `NotFound`, not `Forbidden` — *"to avoid leaking team existence to non-members: team slugs
    /// are globally unique and used in share flows"* (`team_service.rs:277`). A `Forbidden` here
    /// would confirm that a guessed slug names a real team, which is exactly what the refusal is
    /// withholding.
    fn denial() -> ApiError {
        ApiError::NotFound
    }
}

/// Who may read a principal's authorship history on the admin ledger's **actor axis**.
///
/// Scoped to the actor being read about — NOT to standing. `list_by_actor`'s `has_system_access`
/// check sits above this one and stays there: that asks whether the caller is admitted at all,
/// which is a standing question, and ANDing a provisional fact into an authority decision is the
/// shape `temper_principal::admit` exists to forbid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActorHistoryAuthority {
    /// Reading your own authorship. Not an admin act.
    SelfActor,
    /// Reading someone else's is an audit, and audits are admin-only.
    SystemAdmin,
    /// Neither.
    None,
}

#[async_trait]
impl ScopedAuthority for ActorHistoryAuthority {
    /// The actor whose history is being read.
    type Subject = ProfileId;

    async fn resolve(pool: &PgPool, caller: ProfileId, actor: ProfileId) -> ApiResult<Self> {
        // Self-read first, and it is free: no query at all. Probing `is_system_admin` ahead of it
        // would charge every principal a round-trip to read their own authorship.
        if caller == actor {
            return Ok(ActorHistoryAuthority::SelfActor);
        }
        Ok(if access_service::is_system_admin(pool, caller).await? {
            ActorHistoryAuthority::SystemAdmin
        } else {
            ActorHistoryAuthority::None
        })
    }

    fn is_denial(&self) -> bool {
        matches!(self, ActorHistoryAuthority::None)
    }

    /// `NotFound`, matching what this axis has always returned: a `Forbidden` would confirm that
    /// the queried profile exists and has ledger authorship, which is what the refusal withholds.
    fn denial() -> ApiError {
        ApiError::NotFound
    }
}
