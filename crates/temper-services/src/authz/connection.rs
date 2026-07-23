//! Connection reach — may this caller hand read-reach on *this connection* to *that team*?
//!
//! The third two-sided gate, and the one that does **not** share
//! [`super::TwoSidedAuthority`]'s resolver. It looks similar and is not: the object side here is
//! not "do you administer this thing?" but the whole of `MachineAuthority`, and there is no
//! gating-team exclusion (a reach grant flips no regime and transfers no ownership — spec §6.1).
//! Folding it into the shared resolver would mean parameterizing away everything that is actually
//! shared.
//!
//! It also must not route through `GrantAuthority`, despite writing a `kb_access_grants` row. That
//! is stated at the call site and is load-bearing: *"the `can_grant` seam has no bootstrap holder
//! for a connection subject"* (`connection_service.rs`, `grant_reach`'s doc). A connection has no
//! principal who can grant on it, so a `can_grant` gate would deny everyone.

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;

use super::ScopedAuthority;
use crate::error::{ApiError, ApiResult};
use crate::services::connection_service;
use crate::services::machine_authz::{self, MachineAuthority};

/// The connection whose reach is being conferred, and the team receiving it.
///
/// **Named fields, not a `(Uuid, Uuid)` tuple.** Two same-typed ids in positional order is the
/// transposition hazard this whole layer exists to remove — `(connection, team)` and
/// `(team, connection)` type-check identically, and the compiler would watch a swap go by. The
/// connection's *owning* team is deliberately absent: it is derived inside `resolve` from the
/// connection row, so a caller cannot name the team its authority will be checked against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConnectionScope {
    /// The connection whose received data the grant exposes.
    pub(crate) connection_id: Uuid,
    /// The team gaining READ. Not the connection's owner.
    pub(crate) target_team_id: Uuid,
}

impl ConnectionScope {
    pub(crate) fn new(connection_id: Uuid, target_team_id: Uuid) -> Self {
        Self {
            connection_id,
            target_team_id,
        }
    }
}

/// May this caller act on this connection at all?
///
/// The **first** of `ConnectionAuthority`'s two questions, under its own name and its own subject —
/// the connection, not a (connection, team) pair. It exists because revocation legitimately asks
/// only this one: `revoke_reach` withdraws reach from a team the caller may no longer manage, and
/// must still succeed (spec §2.5).
///
/// `ConnectionAuthority` **composes** this rather than re-asking, so there is exactly one place that
/// knows the owning team is read from the row and handed to `MachineAuthority`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectionControlAuthority {
    /// A system admin.
    SystemAdmin,
    /// Owner of the team that owns this connection.
    OwnerOfOwningTeam,
    /// Neither — including a teamless connection, which fails closed (spec D2).
    None,
}

#[async_trait]
impl ScopedAuthority for ConnectionControlAuthority {
    /// The connection itself. Its owning team is derived, never supplied.
    type Subject = Uuid;

    async fn resolve(pool: &PgPool, caller: ProfileId, connection_id: Uuid) -> ApiResult<Self> {
        // Read from the row, never from the caller — see `ConnectionScope`'s doc for why this
        // derivation is what gives the proof its meaning.
        let connection = connection_service::get(pool, connection_id).await?;

        Ok(
            match <MachineAuthority as ScopedAuthority>::resolve(
                pool,
                caller,
                connection.owner_team_id,
            )
            .await?
            {
                MachineAuthority::SystemAdmin => ConnectionControlAuthority::SystemAdmin,
                MachineAuthority::TeamOwner => ConnectionControlAuthority::OwnerOfOwningTeam,
                MachineAuthority::None => ConnectionControlAuthority::None,
            },
        )
    }

    fn is_denial(&self) -> bool {
        matches!(self, ConnectionControlAuthority::None)
    }

    fn denial() -> ApiError {
        ApiError::Forbidden
    }
}

/// Who may confer a team's read-reach on a connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectionAuthority {
    /// A system admin. Exempt from the receiving team's manage bar (Phase A D5) — but its
    /// *existence* was still checked, because the D5 bypass is about authority, not about writing
    /// a `principal_id` that points at nothing.
    SystemAdmin,
    /// Owner of the connection's owning team, who also manages the receiving team.
    OwnerAndTargetManager,
    /// Neither — failing closed on a teamless connection, on a caller who does not own the owning
    /// team, and on a receiving team the caller does not manage.
    None,
}

#[async_trait]
impl ScopedAuthority for ConnectionAuthority {
    type Subject = ConnectionScope;

    /// Two teams are in play and they are two different questions, asked in this order — the order
    /// `grant_reach` has always asked them in.
    ///
    /// 1. **May you act on this connection?** — `MachineAuthority` over the connection's OWNING
    ///    team, the same policy every other connection mutator uses. A teamless connection fails
    ///    closed there (spec D2), so it needs no special case here.
    /// 2. **May you hand read-reach to THAT team?** — `machine_authz::contain_target_team` on the
    ///    receiving team. Without it, the owner of one team could bind their connection's reach to
    ///    any team UUID in the instance.
    ///
    /// Both are **called, not restated** — question 2 in particular routes through the shared
    /// `require_manage_on_team` seam expressly so it cannot drift from `contain_reach`'s team loop.
    async fn resolve(pool: &PgPool, caller: ProfileId, scope: ConnectionScope) -> ApiResult<Self> {
        // Question 1, asked through `ConnectionControlAuthority` rather than restated — that type
        // exists precisely because revocation asks this one alone, and two copies of "resolve the
        // owning team, then MachineAuthority" is exactly the drift this layer removes.
        let control = <ConnectionControlAuthority as ScopedAuthority>::resolve(
            pool,
            caller,
            scope.connection_id,
        )
        .await?;
        if control.is_denial() {
            return Ok(ConnectionAuthority::None);
        }

        // `contain_target_team` still takes a `MachineAuthority`, so map back for the one call. The
        // mapping is total and lossless — the two enums are the same three cases under two names,
        // which is the cost of `contain_target_team` being shared with the machine path.
        let machine = match control {
            ConnectionControlAuthority::SystemAdmin => MachineAuthority::SystemAdmin,
            ConnectionControlAuthority::OwnerOfOwningTeam => MachineAuthority::TeamOwner,
            // Unreachable — the denial arm returned above. Enumerated rather than `_ =>` so a
            // future arm cannot land here and be silently authorized.
            ConnectionControlAuthority::None => return Ok(ConnectionAuthority::None),
        };

        // `contain_target_team` answers with `Ok`, a `Forbidden` refusal, or a `NotFound` when the
        // receiving team does not exist. Only the refusal becomes an arm: `NotFound` is a
        // precondition failure about the target, not this domain's denial, and collapsing it into
        // one would turn "that team does not exist" into "you may not" — a real behavior change
        // pinned by `granting_reach_to_a_nonexistent_team_writes_no_dangling_row`.
        match machine_authz::contain_target_team(pool, machine, caller, scope.target_team_id).await
        {
            Ok(()) => Ok(match control {
                ConnectionControlAuthority::SystemAdmin => ConnectionAuthority::SystemAdmin,
                ConnectionControlAuthority::OwnerOfOwningTeam => {
                    ConnectionAuthority::OwnerAndTargetManager
                }
                ConnectionControlAuthority::None => ConnectionAuthority::None,
            }),
            Err(ApiError::Forbidden) => Ok(ConnectionAuthority::None),
            Err(other) => Err(other),
        }
    }

    fn is_denial(&self) -> bool {
        matches!(self, ConnectionAuthority::None)
    }

    /// `Forbidden`, as both underlying questions have always refused with.
    fn denial() -> ApiError {
        ApiError::Forbidden
    }
}
