//! The two-sided gate: may this caller attach *this object* to *that team*?
//!
//! One policy where `cogmap_service::can_bind` and `context_service::can_share` were the same
//! policy written twice. They differed in exactly one probe — how the caller's authority over the
//! *object* is established — and agreed on everything else: the admin short-circuit, the
//! gating-team exclusion, and the `can_manage` bar on the target team.
//!
//! **In Rust, this is now the only copy. It is not the only copy.** `context_reassign`
//! (`migrations/20260715000010_context_reassign_fns.sql:77-93`) re-implements the same policy in
//! plpgsql — admin bypass, non-gating target, owner/maintainer on it, administers-the-context —
//! because it is the *atomic* enforcement behind [`crate::services::context_service::reassign`]'s
//! pre-check (`context_service.rs:629` explains the pairing). That copy is deliberate and out of
//! this module's reach. Anyone tempted to describe this as "the one place the policy lives" should
//! read spec §6.2 first.

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;

use super::ScopedAuthority;
use crate::error::{ApiError, ApiResult};
use crate::services::{access_service, context_service, team_service};

/// The object being attached to a team, and which "do you administer it?" question that implies.
///
/// A closed two-arm enum rather than the more general `RefTarget`: only cogmaps and contexts have a
/// two-sided gate, and spelling the subject with a four-variant type would force this module to
/// answer for resources and connections — arms that cannot occur, whose only honest handling is a
/// silent denial. Two arms means the match is exhaustive over things that are actually real, and
/// adding a third is a deliberate edit rather than a fall-through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TwoSidedObject {
    /// A cognitive map being bound to, or unbound from, a team. Administered via `can_grant` on
    /// the map.
    Cogmap(Uuid),
    /// A context being shared with, unshared from, or reassigned to a team. Administered by owning
    /// it, or managing its owning team.
    Context(Uuid),
}

/// **Both** sides of the gate, travelling together as one subject.
///
/// This pairing is the point. A gate that resolved authority from the object alone would leave the
/// act free to name its own team (and vice versa), so authorizing `(map, teamA)` and then writing
/// `(map, teamB)` would be a transposition no compiler could see. The proof carries the pair; the
/// act reads both halves out of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TwoSidedScope {
    pub(crate) object: TwoSidedObject,
    pub(crate) team_id: Uuid,
}

impl TwoSidedScope {
    /// A cognitive map ↔ team binding.
    pub(crate) fn cogmap(cogmap_id: Uuid, team_id: Uuid) -> Self {
        Self {
            object: TwoSidedObject::Cogmap(cogmap_id),
            team_id,
        }
    }

    /// A context ↔ team share, unshare, or reassign.
    pub(crate) fn context(context_id: Uuid, team_id: Uuid) -> Self {
        Self {
            object: TwoSidedObject::Context(context_id),
            team_id,
        }
    }
}

/// Who may attach an object to a team.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TwoSidedAuthority {
    /// A system admin. Bypasses both sides — including the gating-team exclusion, which exists to
    /// keep these acts admin-only in the first place.
    SystemAdmin,
    /// The caller administers the object AND manages a target team that is not the gating team.
    Delegated,
    /// Neither.
    None,
}

#[async_trait]
impl ScopedAuthority for TwoSidedAuthority {
    type Subject = TwoSidedScope;

    /// The probe order is both gates' order, unchanged: admin, then the gating-team exclusion, then
    /// the target-team bar, then the object side. It is preserved rather than tidied because it is
    /// what the two gates already cost — an admin resolves in one query and never touches the
    /// object probe, and the exclusion sits ahead of the team bar so a gating-team target is
    /// refused without a role lookup.
    ///
    /// ## The gating-team exclusion is one line and two different reasons
    ///
    /// It is a single probe here, but the two gates it came from hold it for reasons that are not
    /// the same, and a future reader deciding whether it may be relaxed needs both.
    ///
    /// **Cogmaps — structural, and the load-bearing direction is UNBIND.** A `kb_team_cogmaps` row
    /// joining a map to the gating team is precisely what `access_service::cogmap_write_requires_admin`
    /// reads, so the binding does not merely *relate* the map to a team: it *is* the switch that
    /// puts the map in the admin-write regime. Since one gate serves bind and unbind alike, dropping
    /// the exclusion would let a non-admin holding `can_grant` on the map who manages the gating
    /// team **unbind a protected map** out of that regime. Binding into the gating team is a
    /// restriction the caller inflicts on themselves; unbinding is an escalation.
    ///
    /// **Contexts — the reassign path's plpgsql twin.** The reason once recorded here ("sharing into
    /// the root team is an instance-level escalation") was true while gating-team membership *was*
    /// instance access, and D11 ended that: `has_system_access` reads `kb_principal_standing` and
    /// `is_system_admin` reads `kb_principal_governance`, neither consulting the gating team. What
    /// keeps the exclusion is narrower — this gate also fronts `context_service::reassign`, a
    /// transfer of *ownership* into the root team that `context_reassign` independently forbids in
    /// plpgsql, so relaxing the Rust half would split one act across two error paths.
    ///
    /// Its third sibling, `machine_authz::contain_target_team`, deliberately has no such exclusion.
    /// All three reasons: spec §6.1 in
    /// `docs/superpowers/specs/2026-07-22-scoped-authority-policy-layer-design.md`.
    async fn resolve(pool: &PgPool, caller: ProfileId, scope: TwoSidedScope) -> ApiResult<Self> {
        if access_service::is_system_admin(pool, caller).await? {
            return Ok(TwoSidedAuthority::SystemAdmin);
        }
        if access_service::is_gating_team(pool, scope.team_id).await? {
            return Ok(TwoSidedAuthority::None);
        }

        // The target-team bar: `can_manage` (Owner|Maintainer) by DIRECT membership. Both gates
        // spelled this identically; it is `can_manage` and not `owner` on purpose.
        let manages_team = matches!(
            team_service::role_on_team(pool, scope.team_id, caller).await?,
            Some(role) if team_service::can_manage(role)
        );
        if !manages_team {
            return Ok(TwoSidedAuthority::None);
        }

        // The object side — the one probe the two gates genuinely disagreed on. Each arm CALLS the
        // predicate its gate always called; neither restates it.
        let administers_object = match scope.object {
            TwoSidedObject::Cogmap(cogmap_id) => {
                access_service::profile_can_grant(pool, caller, "kb_cogmaps", cogmap_id).await?
            }
            TwoSidedObject::Context(context_id) => {
                context_service::caller_administers_context(pool, caller, context_id).await?
            }
        };

        Ok(if administers_object {
            TwoSidedAuthority::Delegated
        } else {
            TwoSidedAuthority::None
        })
    }

    fn is_denial(&self) -> bool {
        matches!(self, TwoSidedAuthority::None)
    }

    /// `Forbidden`, as all five call sites returned before the collapse. Unlike the read gates in
    /// `read_gates.rs`, existence is not the secret here: every one of these acts is reached with an
    /// object the caller already named, and an `ensure_*_exist` check runs immediately after the
    /// gate to turn a bad id into its own `NotFound`.
    fn denial() -> ApiError {
        ApiError::Forbidden
    }
}
