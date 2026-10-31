//! The two-sided gate: may this caller attach *this object* to *that team*?
//!
//! One policy where `cogmap_service::can_bind` and `context_service::can_share` were the same
//! policy written twice. They differed in exactly one probe — how the caller's authority over the
//! *object* is established — and agreed on everything else: the admin short-circuit, the
//! instance-team exclusion (the root and the gating team), and the `can_manage` bar on the target
//! team.
//!
//! **In Rust, this is now the only copy. It is not the only copy.** `context_reassign`
//! (`migrations/20260715000010_context_reassign_fns.sql:77-93`) re-implements the same policy in
//! plpgsql — admin bypass, a target that is neither the root nor the gating team, owner/maintainer on it, administers-the-context —
//! because it is the *atomic* enforcement behind [`crate::services::context_service::reassign`]'s
//! pre-check (`context_service.rs:629` explains the pairing). That copy is deliberate and out of
//! this module's reach. Anyone tempted to describe this as "the one place the policy lives" should
//! read spec §6.2 first.

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use super::{Principal, ScopedAuthority};
use crate::error::{ApiError, ApiResult};
use crate::services::{access_service, cogmap_service, context_service, team_service};

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
    /// keep these acts admin-only in the first place — on any team but a personal one.
    SystemAdmin,
    /// The caller administers the object AND manages a target team that is neither the root nor
    /// the gating team.
    Delegated,
    /// Neither.
    None,
    /// Neither, for a system admin whose object is someone else's personal context, or names no
    /// context at all. Refused as a
    /// missing context is, so the refusal does not confirm that the id is personal space (task
    /// `01a1933d`).
    ConcealedContext,
    /// The same for someone else's personal cognitive map, refused as a missing map is.
    ConcealedCogmap,
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
    /// Its third sibling, `machine_authz::contain_target_team`, deliberately has no gating-team
    /// exclusion. All three reasons: spec §6.1 in
    /// `temper-artifacts:specs/2026-07-22-scoped-authority-policy-layer-design.md`.
    ///
    /// ## The root is excluded beside the gating team, for a reason that is still true
    ///
    /// `temper-system` is every personal team's parent, and reading inherits down the team tree, so
    /// an object attached to the root reaches **every profile on the instance**. That is the
    /// "instance-level escalation" the context reason above once claimed for the gating team, and
    /// for the root it never stopped being true. The two are the same team only when an instance
    /// points `gating_team_slug` at `temper-system`; it is NULL by default and may name any shared
    /// team, and then a non-admin manager of the root would pass this gate into everyone's reach
    /// (team DAG review G14). Only a system admin attaches to the root, as only a system admin
    /// grants to it (`access_service::recipient_bar`). `context_reassign`'s plpgsql twin refuses it
    /// too (`20261112000000`).
    ///
    /// ## A personal team target is its owner's alone
    ///
    /// The admin short-circuit does not apply when the target is a personal team (task
    /// `01a19006`, ruled 2026-10-31): attaching an object to someone's personal team, or detaching
    /// one from it, is acting on that team, which only its owner and erasure do. Nor when the
    /// object is a personal context (task `01a191b4`): sharing, unsharing or reassigning someone's
    /// personal context is acting on their personal space. Nor when the object is a personal
    /// cognitive map (task `01a1933d`, `cogmap_personal_owner`): binding someone's private map to a
    /// team the admin manages would let them read it. The owner passes the ordinary arm below —
    /// they manage the team and administer the object. An admin refused on someone else's
    /// personal object is refused as a missing one would be (the concealed arms).
    async fn resolve(
        pool: &PgPool,
        principal: Principal<'_>,
        scope: TwoSidedScope,
    ) -> ApiResult<Self> {
        let caller = principal.profile_id();
        let object_personal = match scope.object {
            TwoSidedObject::Context(context_id) => {
                context_service::is_others_personal_context(pool, context_id, caller).await?
            }
            TwoSidedObject::Cogmap(cogmap_id) => {
                cogmap_service::is_others_personal_cogmap(pool, cogmap_id, caller).await?
            }
        };
        let personal = object_personal
            || team_service::personal_of(pool, scope.team_id)
                .await?
                .is_some();
        if !personal && principal.system_admin(pool).await?.is_some() {
            return Ok(TwoSidedAuthority::SystemAdmin);
        }

        let authority = delegated(pool, caller, scope).await?;

        // A system admin refused on someone else's personal object is told what a missing id would
        // tell them (task `01a1933d`): an admin passes this gate on an id that names nothing, and
        // the door's existence check then answers `NotFound`, so a `Forbidden` here would mark the
        // id as personal space. A missing object is concealed the same way when the admin is
        // refused for another reason (a personal team as the target), or the two would differ
        // there instead. Only the admin: anyone else is refused `Forbidden` on a missing id too,
        // so concealing it for them would open the oracle it closes for the admin.
        if authority == TwoSidedAuthority::None
            && principal.system_admin(pool).await?.is_some()
            && (object_personal || !object_exists(pool, scope.object).await?)
        {
            return Ok(match scope.object {
                TwoSidedObject::Context(_) => TwoSidedAuthority::ConcealedContext,
                TwoSidedObject::Cogmap(_) => TwoSidedAuthority::ConcealedCogmap,
            });
        }
        Ok(authority)
    }

    fn is_denial(&self) -> bool {
        matches!(
            self,
            TwoSidedAuthority::None
                | TwoSidedAuthority::ConcealedContext
                | TwoSidedAuthority::ConcealedCogmap
        )
    }

    /// `Forbidden`, as all five call sites returned before the collapse. Unlike the read gates in
    /// `read_gates.rs`, existence is not the secret here: every one of these acts is reached with an
    /// object the caller already named, and an `ensure_*_exist` check runs immediately after the
    /// gate to turn a bad id into its own `NotFound`. That check is why a system admin refused on
    /// someone else's personal object is NOT answered here but by [`Self::denial_for`].
    fn denial() -> ApiError {
        ApiError::Forbidden
    }

    /// Two dialects: the concealed arms answer as the door answers a missing object, the
    /// `NotFound` of `context_service::ensure_context_and_team_exist` and
    /// `cogmap_service::ensure_cogmap_and_team_exist`. The arm names the object's kind and nothing
    /// else, so the refusal still cannot name the id.
    fn denial_for(&self) -> ApiError {
        match self {
            TwoSidedAuthority::ConcealedContext => {
                ApiError::NotFound(context_service::CONTEXT_MISSING.to_string())
            }
            TwoSidedAuthority::ConcealedCogmap => {
                ApiError::NotFound(cogmap_service::COGMAP_MISSING.to_string())
            }
            _ => Self::denial(),
        }
    }
}

/// Does the object's row exist at all, whatever its state?
async fn object_exists(pool: &PgPool, object: TwoSidedObject) -> ApiResult<bool> {
    Ok(match object {
        TwoSidedObject::Context(id) => {
            sqlx::query_scalar!(
                r#"SELECT EXISTS(SELECT 1 FROM kb_contexts WHERE id = $1) AS "e!: bool""#,
                id
            )
            .fetch_one(pool)
            .await?
        }
        TwoSidedObject::Cogmap(id) => {
            sqlx::query_scalar!(
                r#"SELECT EXISTS(SELECT 1 FROM kb_cogmaps WHERE id = $1) AS "e!: bool""#,
                id
            )
            .fetch_one(pool)
            .await?
        }
    })
}

/// The ordinary arm, for every caller the admin short-circuit did not admit: neither instance team,
/// `can_manage` on the target team, and authority over the object.
async fn delegated(
    pool: &PgPool,
    caller: temper_core::types::ids::ProfileId,
    scope: TwoSidedScope,
) -> ApiResult<TwoSidedAuthority> {
    if access_service::is_root_or_gating_team(pool, scope.team_id).await? {
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
