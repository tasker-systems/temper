//! Subscription authority — may this caller declare, read, or revoke a subscription authored by
//! this team?
//!
//! **Why this type exists at all**, since the predicate it calls was already being called inline:
//! `subscription_service` used `team_service::require_manage_on_team` directly at each of its three
//! gate sites, so there was no artifact naming the gate — and its module doc drifted into
//! describing one composed "with the system-admin path at the authority layer" that did not exist,
//! with nothing in the codebase able to contradict it. Two of this crate's three
//! deployment-subject surfaces (machine registration, connection provisioning) already resolve
//! through a named authority; this is the third, and naming it is what makes the doc checkable.
//!
//! **The admin leg is deliberate, not incidental.** `list` has always admitted a system admin
//! (`subscription_service::list` passes `is_system_admin` into its predicate), while `create`,
//! `get` and `revoke` did not — an admin could see every subscription in the instance and revoke
//! none of them. The asymmetry was the accident; the composed gate is what the module doc always
//! claimed.

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;

use super::ScopedAuthority;
use crate::error::{ApiError, ApiResult};
use crate::services::{access_service, team_service};

/// The caller's authority over subscriptions authored by a given team.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubscriptionAuthority {
    /// A system admin (a `kb_principal_governance` grant), who reaches every authoring team.
    SystemAdmin,
    /// Owner or maintainer of the authoring team — `team_service::can_manage`, the same bar the
    /// human team-management surface uses. **Called, not restated**, so tightening the human
    /// surface tightens this one with it.
    TeamManager,
    /// Neither. Denial is an ARM rather than an `Err` returned from inside `resolve`, so it flows
    /// through `ScopedAuthority::denial` like every other domain's refusal instead of
    /// short-circuiting past it — which is also why this resolve calls `role_on_team` and not
    /// `require_manage_on_team`, whose `Err(Forbidden)` would bypass the dialect below.
    None,
}

#[async_trait]
impl ScopedAuthority for SubscriptionAuthority {
    /// The **authoring** team — the team the caller names on create, and the one carried on the
    /// row for read and revoke. Not the subscriber, and not the connection's owning team: owner ≠
    /// reach (`migrations/20260714000010_connections.sql:60-61`), and leg 2 asks the reach
    /// question separately.
    type Subject = Uuid;

    async fn resolve(pool: &PgPool, caller: ProfileId, authoring_team: Uuid) -> ApiResult<Self> {
        // Role first, matching `TeamReadAuthority`'s ordering and for the same reason: the common
        // caller here is a team manager, and probing `is_system_admin` first would add a query to
        // every one of them.
        //
        // A nonexistent team resolves to `None` here (`role_on_team` returns `None` for a team_id
        // no row carries), so a bogus UUID is denied rather than reaching the act — the property
        // `require_manage_on_team`'s call site used to note inline.
        if let Some(role) = team_service::role_on_team(pool, authoring_team, caller).await? {
            if team_service::can_manage(role) {
                return Ok(SubscriptionAuthority::TeamManager);
            }
        }
        Ok(if access_service::is_system_admin(pool, caller).await? {
            SubscriptionAuthority::SystemAdmin
        } else {
            SubscriptionAuthority::None
        })
    }

    fn is_denial(&self) -> bool {
        matches!(self, SubscriptionAuthority::None)
    }

    /// `Forbidden` — the dialect these three acts have always refused in, preserved verbatim
    /// through the move behind this type. A subscription's authoring team is not a secret the way
    /// a team slug is (`read_gates.rs`), so there is nothing here for a `NotFound` to withhold.
    fn denial() -> ApiError {
        ApiError::Forbidden
    }
}
