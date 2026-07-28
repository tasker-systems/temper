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

/// The refusal both actor-history sites render — this gate and `admin_ledger_service::list_by_actor`,
/// which answers the same question one layer up.
///
/// Two literals is the `FINDING_REFUSAL` argument at n=2: not one defence written twice, but two
/// that drift the first time either is "improved". Lower stakes than the finding class, since both
/// arms ask only about the **caller** (`ActorHistoryAuthority::resolve` never touches whether the
/// actor exists), so a drift would disclose the caller's own standing back to the caller. Shared
/// anyway — the reason to share is that they must agree, not how bad it is when they don't.
pub(crate) const ACTOR_HISTORY_REFUSAL: &str = "actor history not found or not readable";

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
    ///
    /// The message says "or not readable" for the same reason, and cannot say more: `denial` is
    /// static and argument-free, so it has no access to the slug it is refusing. The ambiguity is
    /// a property of the signature, not of anyone remembering to preserve it.
    fn denial() -> ApiError {
        ApiError::NotFound("team not found or not readable".to_string())
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
    /// The message withholds the same thing — and, `denial` being static, structurally cannot
    /// name the profile even by accident.
    fn denial() -> ApiError {
        ApiError::NotFound(ACTOR_HISTORY_REFUSAL.to_string())
    }
}
