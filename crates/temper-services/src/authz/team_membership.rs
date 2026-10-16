//! Team membership as a rank: who may add, change or remove whom, and the one way a recorded
//! membership change reaches `kb_team_members`.
//!
//! Design: `temper-artifacts:specs/2026-10-08-team-role-rank-design.md` (R1–R13, C1–C16) and its
//! build plan `plans/2026-10-14-team-role-rank.md` (P1–P16).
//!
//! Three pieces, in the order an act uses them:
//!
//! 1. [`MembershipAuthority`] — who the caller is on THIS team: a system admin, a manager (owner or
//!    maintainer), a member who may act only on themselves, or nobody.
//! 2. [`authorize`] — the rank rule, applied to one act on one target. It returns the sealed
//!    [`AuthorizedMembershipChange`], which has private fields and no constructor outside this
//!    module, so a write nobody authorized has nothing to pass to step 3.
//! 3. [`membership_write`] — the only Rust caller of the SQL writer `_admin_team_membership_write`
//!    (migration `20261101200000_team_membership_changed.sql`), which writes the row and appends
//!    its `team_membership_changed` admin event in one statement.
//!
//! # Locks and order (plan P1)
//!
//! A member act runs: resolve on the pool (a non-manager is refused before any lock, so no
//! unauthorized caller can take a team-row lock) → open a transaction → [`locked`] (the team row,
//! then the target's row) → resolve again on the transaction → [`authorize`] → [`membership_write`]
//! → commit. Because the authority is re-resolved under the team lock, and every routed writer
//! takes that lock, the decision reads current state: two co-owners leaving at once are serialized,
//! and the second sees one owner left (C6, finding 8).
//!
//! # Why `MembershipAuthority` is not a `ScopedAuthority` (plan P2)
//!
//! `ScopedAuthority::resolve` takes a `&PgPool`. This authority must also be resolved on the write
//! transaction, after its locks, so its resolve takes a connection. Its arm order is still
//! load-bearing (`internal/agents/key-patterns.md`): admin, then the gating-team arm, then the
//! caller's role on this team alone.

use sqlx::PgConnection;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::team::{MembershipVia, TeamMemberSource, TeamRole};
use temper_substrate::ids::EntityId;

use crate::auth::{HumanPrincipal, SystemAdmin};
use crate::error::{ApiError, ApiResult};
use crate::services::machine_authz::exceeds_machine_ceiling;

/// A role that manages a team. There is no `Member` or `Watcher` variant, so "a member changes
/// someone" has no value to construct (C3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ManagingRole {
    Owner,
    Maintainer,
}

impl ManagingRole {
    /// The team role this is.
    pub(crate) fn role(self) -> TeamRole {
        match self {
            ManagingRole::Owner => TeamRole::Owner,
            ManagingRole::Maintainer => TeamRole::Maintainer,
        }
    }
}

/// Who is acting on a team's membership, resolved by [`MembershipAuthority::resolve`].
#[derive(Debug)]
pub(crate) enum MembershipAuthority {
    /// A system admin: unrestricted on rank (R3, C9), and the only authority on the gating team's
    /// roster (R11, C14). Carries the sealed proof, so this arm cannot be named without one (P16).
    SystemAdmin(SystemAdmin),
    /// A person holding a managing role on THIS team (C13). Never resolved on the gating team.
    Managing(ManagingRole),
    /// A person who may act only on themselves: a member or watcher, or anyone with a row on the
    /// gating team who is not a system admin (C3, C14).
    SelfOnly(TeamRole),
    /// No row on this team and not a system admin.
    None,
}

/// A resolved [`MembershipAuthority`], bound to the team and the caller it was resolved for, with
/// whether the team is the gating team — the one other fact the rank rule reads about the team.
/// Private fields: only [`MembershipAuthority::resolve`] builds one, so [`authorize`] never decides
/// on an authority someone wrote down by hand, nor on one resolved for another team or caller.
#[derive(Debug)]
pub(crate) struct ResolvedMembership {
    authority: MembershipAuthority,
    is_gating: bool,
    team_id: Uuid,
    caller: ProfileId,
}

impl ResolvedMembership {
    /// Whether the caller manages this team: a system admin, or an owner or maintainer of it (never
    /// on the gating team, R11). "May confer something ON this team" — the admin ledger's C12 read
    /// arm and `machine_authz`'s reach bar call this rather than restating it.
    pub(crate) fn manages(&self) -> bool {
        matches!(
            self.authority,
            MembershipAuthority::SystemAdmin(_) | MembershipAuthority::Managing(_)
        )
    }

    /// Checks 1 and 2 of [`decide`] — the half that needs no target state — for the pool pre-check
    /// that runs before any lock (plan P1 step 1). A caller this refuses never takes the team-row
    /// lock, and learns nothing about the target (S6). The same checks run again, in full, under
    /// the locks.
    pub(crate) fn precheck(&self, act: MembershipAct, target: ProfileId) -> ApiResult<()> {
        admits_caller(
            self.authority.kind(),
            self.caller,
            self.is_gating,
            act,
            target,
        )
        .map(|_| ())
    }

    /// The invitation gate (C16, R10): an invitation may offer a role ranked at or below the
    /// inviter's own (C1), and only a manager of the team — or a system admin — may invite. R11
    /// makes that admin-only on the gating team, which [`MembershipAuthority::resolve`]'s arm
    /// order already decides: a non-admin row on the gating team resolves to the self-only arm
    /// or to none, and a non-manager inviting is refused before anything about the invitation is
    /// learned (S6).
    ///
    /// Deliberately not [`authorize`] over `Add`: an invitation is email-keyed, so there is no
    /// profile to [`locked`] or decide on, and inviting someone already on the team is allowed —
    /// their acceptance is a no-op reporting their actual role (P12). What carries over is
    /// exactly checks 1, 2 and 6 of [`decide`], reached by calling the same functions, so the
    /// refusals and their order cannot drift from the rank rule.
    pub(crate) fn authorize_invitation(&self, role: TeamRole) -> ApiResult<()> {
        // Checks 1–2 against an inert stand-in target: `nil` is not a real profile id (they are
        // UUIDv7s), so it can never equal the caller, and an `Add` reads nothing else about its
        // target — existence, machine and source are invitation-specific absences.
        let own = admits_caller(
            self.authority.kind(),
            self.caller,
            self.is_gating,
            MembershipAct::Add { role },
            ProfileId::from(Uuid::nil()),
        )?;
        // Check 6 (conferral) — the half of `decide` that needs only the caller's own rank. A
        // `BadRequest`, like the owner refusal this gate replaces (C16): the offer names a role
        // this caller cannot confer here.
        if let Some(own) = own {
            if role.rank() > own.rank() {
                return Err(ApiError::BadRequest(OFFER_ABOVE_YOUR_OWN_ROLE.to_string()));
            }
        }
        Ok(())
    }
}

impl MembershipAuthority {
    /// Resolve the caller's authority over `team_id`'s membership, in this order:
    ///
    /// 1. a system admin → [`Self::SystemAdmin`];
    /// 2. the gating team → [`Self::SelfOnly`] with the caller's role there, or [`Self::None`]
    ///    (C14: a role on the gating team confers no membership authority there);
    /// 3. the caller's role on this team alone, with no walk up or down the team DAG (C13):
    ///    owner or maintainer → [`Self::Managing`], member or watcher → [`Self::SelfOnly`], no row →
    ///    [`Self::None`].
    ///
    /// The caller is a [`HumanPrincipal`] by type, so a machine cannot act on membership (C4).
    pub(crate) async fn resolve(
        conn: &mut PgConnection,
        caller: &HumanPrincipal,
        team_id: Uuid,
    ) -> ApiResult<ResolvedMembership> {
        let admin = match crate::auth::require_system_admin_on(conn, caller).await {
            Ok(admin) => Some(admin),
            Err(ApiError::Forbidden) => None,
            Err(other) => return Err(other),
        };
        let is_gating = crate::services::access_service::is_gating_team_on(conn, team_id).await?;
        let bound = |authority| ResolvedMembership {
            authority,
            is_gating,
            team_id,
            caller: caller.profile_id(),
        };
        if let Some(admin) = admin {
            return Ok(bound(MembershipAuthority::SystemAdmin(admin)));
        }
        let role =
            crate::services::team_service::role_on_team_on(conn, team_id, caller.profile_id())
                .await?;
        let authority = if is_gating {
            role.map_or(MembershipAuthority::None, MembershipAuthority::SelfOnly)
        } else {
            match role {
                Some(TeamRole::Owner) => MembershipAuthority::Managing(ManagingRole::Owner),
                Some(TeamRole::Maintainer) => {
                    MembershipAuthority::Managing(ManagingRole::Maintainer)
                }
                Some(role @ (TeamRole::Member | TeamRole::Watcher)) => {
                    MembershipAuthority::SelfOnly(role)
                }
                None => MembershipAuthority::None,
            }
        };
        Ok(bound(authority))
    }

    /// The proof-free shape of this authority, which [`decide`] reads.
    fn kind(&self) -> AuthorityKind {
        match self {
            MembershipAuthority::SystemAdmin(_) => AuthorityKind::Admin,
            MembershipAuthority::Managing(role) => AuthorityKind::Managing(*role),
            MembershipAuthority::SelfOnly(role) => AuthorityKind::SelfOnly(*role),
            MembershipAuthority::None => AuthorityKind::None,
        }
    }
}

/// [`MembershipAuthority`] without the proof. It exists so the rank rule ([`decide`]) is a pure
/// function a unit test can drive over the whole table without minting — or forging — a
/// `SystemAdmin`. Nothing outside this module can build one, and nothing turns one back into an
/// authority: [`authorize`] takes the real authority and only reads this from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthorityKind {
    Admin,
    Managing(ManagingRole),
    SelfOnly(TeamRole),
    None,
}

/// One membership act.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MembershipAct {
    /// Add a profile at `role`. Insert-only: an existing row is a 409 naming `change_role` (#1070).
    Add { role: TeamRole },
    /// Change an existing member's role to `role`.
    Change { role: TeamRole },
    /// Remove an existing member (a leave, when the target is the caller).
    Remove,
}

/// The member an act is aimed at, as read under the locks. Private fields: only [`locked`] builds
/// one, so [`authorize`] decides on a target read under the team lock, on the team it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MembershipTarget {
    team_id: Uuid,
    profile: ProfileId,
    /// The target's row on this team, if any.
    current: Option<(TeamRole, TeamMemberSource)>,
    is_machine: bool,
}

impl MembershipTarget {
    /// Whether the target is a machine principal, as read under the locks. The approval flow's
    /// machine skip reads this (plan C3): a machine joins no team nobody chose for it.
    pub(crate) fn is_machine(&self) -> bool {
        self.is_machine
    }
}

/// The refusal for an add over an existing row (#1070), naming the operation that exists.
pub(crate) const ALREADY_A_MEMBER: &str = "already a member of this team; change their role with \
     `temper team set-role` (PATCH /api/teams/{id}/members/{profile_id})";
/// The refusal for an act on an identity-provider row (C7).
pub(crate) const PROVISIONED_BY_SAML: &str =
    "this membership is provisioned by SAML; change it via the identity provider";
/// The refusal for a machine target above the ceiling (C5), every authority included.
pub(crate) const MACHINE_ROLE_CEILING: &str =
    "a machine principal cannot hold a team role above 'member'";
/// The refusal for granting a role above your own (C1).
pub(crate) const ABOVE_YOUR_OWN_ROLE: &str =
    "you can grant a role up to your own on this team; only a system admin can grant more";
/// The refusal for offering a role above the inviter's own (C16, R10). A `BadRequest`, like the
/// owner refusal it replaces: the requested role is not grantable by this caller on this team.
pub(crate) const OFFER_ABOVE_YOUR_OWN_ROLE: &str =
    "an invitation can offer a role up to your own on this team; only a system admin can offer more";
/// The refusal for acting on an owner other than yourself (C2, R6).
pub(crate) const ANOTHER_OWNER: &str = "only a system admin can change or remove an owner other \
     than yourself; an owner hands a team over by granting `owner`, then lowering their own role";
/// The refusal for acting on a member ranked above you (C2).
pub(crate) const ABOVE_YOUR_RANK: &str =
    "you can change or remove members up to your own role on this team";
/// The refusal for raising your own role, or any self-change by a member or watcher (C3, C3a).
pub(crate) const RAISE_YOURSELF: &str =
    "you can lower your own role or leave, but not raise it; a member or watcher can only leave";
/// The refusal for a self-change on the gating team (R11, P14 b).
pub(crate) const GATING_SELF_CHANGE: &str =
    "on the gating team only a system admin changes roles; you may leave it";
/// The refusal when the last-owner guard fires (C6).
pub(crate) const LAST_OWNER: &str =
    "cannot remove or demote the team's last owner; grant `owner` to another member first";
/// The refusal when the target's row changed between authorization and write.
pub(crate) const MEMBERSHIP_CHANGED: &str = "membership changed; retry";

/// What [`decide`] admits, before it is sealed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Decision {
    via: MembershipVia,
}

/// Checks 1 and 2 of [`decide`]: the authority is not `None`, and a non-admin's act is either on
/// themselves and admitted by the self rule, or on another member by a manager. Needs no target
/// state, so it also serves as the pool pre-check ([`ResolvedMembership::precheck`]). Returns the
/// caller's own role — `None` for a system admin, who has no rank limit.
fn admits_caller(
    authority: AuthorityKind,
    caller: ProfileId,
    is_gating: bool,
    act: MembershipAct,
    target: ProfileId,
) -> Result<Option<TeamRole>, ApiError> {
    // 1. Authority.
    let caller_role = match authority {
        AuthorityKind::None => return Err(ApiError::Forbidden),
        AuthorityKind::Admin => return Ok(None),
        AuthorityKind::Managing(role) => role.role(),
        AuthorityKind::SelfOnly(role) => role,
    };

    // 2. Self or other, for a non-admin.
    if target == caller {
        match act {
            MembershipAct::Remove => {}
            MembershipAct::Change { role } => {
                if is_gating {
                    return Err(ApiError::ForbiddenDetail(GATING_SELF_CHANGE.to_string()));
                }
                let manages = matches!(authority, AuthorityKind::Managing(_));
                if !manages || role.rank() > caller_role.rank() {
                    return Err(ApiError::ForbiddenDetail(RAISE_YOURSELF.to_string()));
                }
            }
            MembershipAct::Add { .. } => return Err(ApiError::Forbidden),
        }
    } else if !matches!(authority, AuthorityKind::Managing(_)) {
        return Err(ApiError::Forbidden);
    }
    Ok(Some(caller_role))
}

/// The rank rule (spec "The authority type", plan B3 and P13/P14), applied to one act. Pure: no
/// database, no proof. The checks run in this order, and the first refusal wins:
///
/// 1. **Authority.** `None` → 403. Always first, so a caller with no authority on the team learns
///    nothing about whether a member, an identity-provider row or a personal team exists (S6).
/// 2. **Self or other, for a non-admin.** `SelfOnly` admits only acts on the caller. A non-admin's
///    act on themselves is decided HERE and never reaches checks 6–7:
///    - a leave (`Remove`) is admitted for any role (C3, Q4 on the gating team);
///    - a `Change` is admitted only from a managing role, off the gating team, to a rank ≤ the
///      caller's own (C3a; P14 b, c — equal is a no-op the writer records as nothing);
///    - anything else → 403.
///
///    Deciding self-acts here is what keeps check 7's "not an owner" from refusing an owner who
///    lowers themselves — the hand-over (the R6 trap). A system admin's self-acts are not special:
///    they go through the admin arm like any other act (P14 a).
/// 3. **Existence (P13).** `Add` over an existing row → 409 naming `change_role` (#1070). `Change`
///    or `Remove` of an absent row → 404.
/// 4. **Target kind.** A machine target above `member` → 400, for EVERY authority, admin included
///    (C5).
/// 5. **Target source.** An identity-provider row → 409 (C7).
/// 6. **Conferral, non-admin.** The new role ranks at or below the caller's (C1) → else 403.
/// 7. **Reach, non-admin, on another member.** The target ranks at or below the caller **and** is
///    not an `owner` (C2, R6) → else 403. Deliberately not "strictly below": a maintainer acts on a
///    maintainer.
///
/// ## The decision table
///
/// `A` admin, `O` / `Mt` managing owner / maintainer, `S(r)` self-only at role `r`, `N` none.
/// "gating" is the gating team. Rows marked † are the R6 trap; rows marked ‡ are P14.
///
/// | Authority | Target | Act | Target now | Outcome |
/// |---|---|---|---|---|
/// | N | any | any | any | 403 |
/// | S(member/watcher) | other | any | any | 403 |
/// | S(any) | self | remove | own row | allowed (leave; still C6, C7) |
/// | S(member/watcher) | self | change | own row | 403 |
/// | S(owner/maintainer), gating | self | change | own row | 403 ‡ (b) |
/// | S(any), gating | other | any | any | 403 (C14) |
/// | O / Mt | self | add | own row | 403 |
/// | O / Mt | self | change to rank ≤ own | own row | allowed; equal is a 200 no-op ‡ (c) † |
/// | O / Mt | self | change to rank > own | own row | 403 |
/// | O / Mt | self | remove | own row | allowed (leave; still C6, C7) † |
/// | O / Mt / A | other | add | row exists | 409 naming `change_role` |
/// | O / Mt / A | other | change / remove | no row | 404 |
/// | any but N | any | add / change above `member` | machine | 400 (A included) |
/// | O / Mt / A | any | change / remove | idp row | 409 |
/// | Mt | other | add / change to `owner` | — | 403 (C1) |
/// | O | other | add / change to `owner` | non-owner or absent | allowed (C1) |
/// | Mt | other | change / remove | owner | 403 (C2) |
/// | O | other | change / remove | owner | 403 (R6) |
/// | Mt | other | change / remove | maintainer | allowed † |
/// | O / Mt | other | change / remove | member / watcher | allowed |
/// | A | any, self included | any | any | allowed, subject to checks 3–5 ‡ (a) |
fn decide(
    authority: AuthorityKind,
    caller: ProfileId,
    is_gating: bool,
    act: MembershipAct,
    target: &MembershipTarget,
) -> Result<Decision, ApiError> {
    // 1–2. Authority; self or other.
    let caller_role = admits_caller(authority, caller, is_gating, act, target.profile)?;
    let is_self = target.profile == caller;

    // 3. Existence.
    match (act, target.current) {
        (MembershipAct::Add { .. }, Some(_)) => {
            return Err(ApiError::Conflict(ALREADY_A_MEMBER.to_string()))
        }
        (MembershipAct::Change { .. } | MembershipAct::Remove, None) => {
            return Err(ApiError::NotFound("team member not found".to_string()))
        }
        _ => {}
    }

    // 4. Target kind — every authority.
    if let MembershipAct::Add { role } | MembershipAct::Change { role } = act {
        if target.is_machine && exceeds_machine_ceiling(role) {
            return Err(ApiError::BadRequest(MACHINE_ROLE_CEILING.to_string()));
        }
    }

    // 5. Target source.
    if let Some((_, TeamMemberSource::Idp)) = target.current {
        return Err(ApiError::Conflict(PROVISIONED_BY_SAML.to_string()));
    }

    if let (Some(own), false) = (caller_role, is_self) {
        // 6. Conferral.
        if let MembershipAct::Add { role } | MembershipAct::Change { role } = act {
            if role.rank() > own.rank() {
                return Err(ApiError::ForbiddenDetail(ABOVE_YOUR_OWN_ROLE.to_string()));
            }
        }
        // 7. Reach.
        if let (MembershipAct::Change { .. } | MembershipAct::Remove, Some((current, _))) =
            (act, target.current)
        {
            if current == TeamRole::Owner {
                return Err(ApiError::ForbiddenDetail(ANOTHER_OWNER.to_string()));
            }
            // With today's four roles this cannot fire: the managing roles are owner and maintainer,
            // and an owner target was refused just above. It states the rule (C2) rather than
            // leaning on the role list, so a role added between maintainer and owner is covered.
            if current.rank() > own.rank() {
                return Err(ApiError::ForbiddenDetail(ABOVE_YOUR_RANK.to_string()));
            }
        }
    }

    // `self` is leaving or lowering your own role (the `MembershipVia::SelfAct` contract). A system
    // admin who adds themselves or raises their own role acted through the admin arm (P14 a), so
    // the record says `member_act`.
    let leaves_or_lowers = match (act, target.current) {
        (MembershipAct::Remove, _) => true,
        (MembershipAct::Change { role }, Some((current, _))) => role.rank() <= current.rank(),
        _ => false,
    };
    Ok(Decision {
        via: if is_self && leaves_or_lowers {
            MembershipVia::SelfAct
        } else {
            MembershipVia::MemberAct
        },
    })
}

/// A membership change someone was authorized to make. **Sealed**: private fields, no `Default`,
/// `Clone` or public constructor. The constructors are [`authorize`] and the one-per-writer token
/// constructors of plan PR C (genesis, invitation, approval, machine reach, SAML reconcile), each
/// taking a token only its one caller can mint (plan P16).
#[derive(Debug)]
pub(crate) struct AuthorizedMembershipChange {
    team_id: Uuid,
    profile: ProfileId,
    act: MembershipAct,
    via: MembershipVia,
    /// The row the change was authorized against — `None` for an add. The writer re-checks it under
    /// its lock and answers `stale` if it moved.
    expected: Option<(TeamRole, TeamMemberSource)>,
    /// The source an add writes.
    new_source: TeamMemberSource,
    /// `None` only for a system act (SAML reconcile, plan PR C).
    actor: Option<ProfileId>,
    invited_by: Option<ProfileId>,
    /// `false` only for SAML reconcile (R8), which has its own constructor in plan PR C. Carried in the
    /// sealed value rather than passed beside it, so no call site can switch the guard off.
    guard_last_owner: bool,
}

/// Apply the rank rule ([`decide`]) for a resolved authority and seal the result. The resolution is
/// consumed: it was made for one decision, under one set of locks. The team and the caller come from
/// the resolution itself, and a target locked on any other team is refused, so an authority
/// resolved on one team cannot authorize a write on another.
pub(crate) fn authorize(
    resolved: ResolvedMembership,
    act: MembershipAct,
    target: &MembershipTarget,
) -> Result<AuthorizedMembershipChange, ApiError> {
    if target.team_id != resolved.team_id {
        return Err(ApiError::Internal(
            "membership authority resolved for one team, target locked on another".to_string(),
        ));
    }
    let decision = decide(
        resolved.authority.kind(),
        resolved.caller,
        resolved.is_gating,
        act,
        target,
    )?;
    // The recorded actor. On the admin arm it is read from the sealed proof — the profile the
    // governance check actually passed for — rather than from the caller beside it.
    let actor = match &resolved.authority {
        MembershipAuthority::SystemAdmin(admin) => admin.actor(),
        _ => resolved.caller,
    };
    Ok(AuthorizedMembershipChange {
        team_id: resolved.team_id,
        profile: target.profile,
        act,
        via: decision.via,
        expected: target.current,
        new_source: TeamMemberSource::Native,
        actor: Some(actor),
        invited_by: None,
        guard_last_owner: true,
    })
}

impl AuthorizedMembershipChange {
    /// `create_team`'s creator row (plan P7): the creator is its `owner` by construction, recorded
    /// as `team_genesis` with the creator as both actor and member. `token` is a struct with a
    /// private field defined in `team_service`, constructed only inside `create_team` immediately
    /// after its team insert, so this cannot run for a team the caller did not just create.
    ///
    /// The caller does not run [`locked`]: the team was born in the caller's own transaction, so no
    /// concurrent writer can see it, and the SQL writer re-takes the team and member locks itself.
    pub(crate) fn genesis(
        _token: crate::services::team_service::GenesisToken,
        team_id: Uuid,
        creator: ProfileId,
    ) -> Self {
        Self {
            team_id,
            profile: creator,
            act: MembershipAct::Add {
                role: TeamRole::Owner,
            },
            via: MembershipVia::TeamGenesis,
            expected: None,
            new_source: TeamMemberSource::Native,
            actor: Some(creator),
            invited_by: None,
            guard_last_owner: true,
        }
    }

    /// An accepted invitation's member row (plan P12): the invitee joins at the invited role, the
    /// record says `invitation` and names the inviter. `claimed` is minted only by the claim
    /// `UPDATE` in `invitation_service::accept_invitation` — a struct with private fields, so
    /// this cannot run for an invitation nobody claimed — and the invitation's authority was
    /// checked when it was created (C16), which is why no rank decision runs here. The caller
    /// holds [`locked`]'s locks on the team, as P12 orders it.
    pub(crate) fn from_claimed_invitation(
        claimed: crate::services::invitation_service::ClaimedInvitation,
        invitee: ProfileId,
    ) -> Self {
        Self {
            team_id: claimed.team_id(),
            profile: invitee,
            act: MembershipAct::Add {
                role: claimed.role(),
            },
            via: MembershipVia::Invitation,
            expected: None,
            new_source: TeamMemberSource::Native,
            actor: Some(invitee),
            invited_by: Some(claimed.invited_by()),
            guard_last_owner: true,
        }
    }

    /// The approval flow's watcher row (plan C3): `access_service::review_request` acting as a
    /// system admin writes the requester onto the request's team as `watcher`, recorded as
    /// `access_approval` with the admin as actor. `admin` is the sealed proof, so nothing else
    /// can call this. No rank decision runs — the admin arm is unrestricted (R3) — and
    /// idempotency is the writer's: an add over an existing row via `access_approval` is a
    /// silent `noop`, never the 409 a rank decision would raise.
    pub(crate) fn from_system_admin(admin: &SystemAdmin, team_id: Uuid, member: ProfileId) -> Self {
        Self {
            team_id,
            profile: member,
            act: MembershipAct::Add {
                role: TeamRole::Watcher,
            },
            via: MembershipVia::AccessApproval,
            expected: None,
            new_source: TeamMemberSource::Native,
            actor: Some(admin.actor()),
            invited_by: None,
            guard_last_owner: true,
        }
    }

    /// A machine registration's team reach (plan C3): the registrar's authorized reach lands the
    /// machine on `target`'s team, recorded as `machine_reach` with the registrar as actor. The
    /// proof is the `AuthorizedReach` itself — only `machine_authz` can construct one — and it
    /// carries the D4b ceiling already enforced at mint, which is why no rank decision runs
    /// here. `target` is a [`locked`] read, so the act is `add` or `change` per the row as it
    /// stood under the team lock; a change to the current role is the writer's `noop`.
    pub(crate) fn from_reach(
        _reach: &crate::services::machine_authz::AuthorizedReach,
        registrar: ProfileId,
        target: &MembershipTarget,
        role: TeamRole,
    ) -> Self {
        let (act, expected) = match target.current {
            None => (MembershipAct::Add { role }, None),
            Some((current, source)) => (MembershipAct::Change { role }, Some((current, source))),
        };
        Self {
            team_id: target.team_id,
            profile: target.profile,
            act,
            via: MembershipVia::MachineReach,
            expected,
            new_source: TeamMemberSource::Native,
            actor: Some(registrar),
            invited_by: None,
            guard_last_owner: true,
        }
    }

    /// An identity-provider reconcile write (plan C4, R8): the IdP wins over the last-owner
    /// guard, the system is the actor, and the row's source stays (or becomes) `idp`. `token` is
    /// minted only inside `saml_provisioning_service::reconcile_idp_memberships`. `target` is a
    /// [`locked`] read under the team lock; an add or a change per the row as it stood.
    ///
    /// A native row is REFUSED: reconcile skips native rows on its locked re-read (S1, C7), so a
    /// native `current` here is a caller bug, and the internal error says so rather than
    /// silently writing a native row's role.
    pub(crate) fn idp(
        _token: crate::services::saml_provisioning_service::IdpReconcileToken,
        target: &MembershipTarget,
        role: TeamRole,
    ) -> ApiResult<Self> {
        let (act, expected) = match target.current {
            None => (MembershipAct::Add { role }, None),
            Some((current, TeamMemberSource::Idp)) => (
                MembershipAct::Change { role },
                Some((current, TeamMemberSource::Idp)),
            ),
            Some((_, TeamMemberSource::Native)) => {
                return Err(ApiError::Internal(
                    "idp reconcile was handed a native membership row".to_string(),
                ))
            }
        };
        Ok(Self {
            team_id: target.team_id,
            profile: target.profile,
            act,
            via: MembershipVia::IdpReconcile,
            expected,
            new_source: TeamMemberSource::Idp,
            actor: None,
            invited_by: None,
            guard_last_owner: false,
        })
    }

    /// An identity-provider reconcile revocation (plan C4, R8): the row leaves. The same proof
    /// and the same refusals as `idp` above; the target must hold an `idp` row.
    pub(crate) fn idp_removal(
        _token: crate::services::saml_provisioning_service::IdpReconcileToken,
        target: &MembershipTarget,
    ) -> ApiResult<Self> {
        match target.current {
            Some((_, TeamMemberSource::Idp)) => Ok(Self {
                team_id: target.team_id,
                profile: target.profile,
                act: MembershipAct::Remove,
                via: MembershipVia::IdpReconcile,
                expected: target.current,
                new_source: TeamMemberSource::Idp,
                actor: None,
                invited_by: None,
                guard_last_owner: false,
            }),
            Some((_, TeamMemberSource::Native)) => Err(ApiError::Internal(
                "idp reconcile was handed a native membership row".to_string(),
            )),
            None => Err(ApiError::Internal(
                "idp reconcile was handed an absent membership row".to_string(),
            )),
        }
    }
}

/// Take plan P1's two locks — the team row (`FOR NO KEY UPDATE`), then the target's membership row
/// (`FOR UPDATE`) — and read the target as it stands under them.
///
/// **Lock order: team, then member row,** and the machine-ceiling trigger's per-profile advisory
/// lock after both (its order since `20261024100000`). Every routed writer takes the team lock first,
/// so all of them serialize on one team and none can deadlock another over a member row.
///
/// A team with no row is a 404. A caller reaches this only after the pool pre-check admitted them,
/// so only a system admin can be told a team does not exist.
pub(crate) async fn locked(
    conn: &mut PgConnection,
    team_id: Uuid,
    profile: ProfileId,
) -> ApiResult<MembershipTarget> {
    let team = sqlx::query_scalar!(
        "SELECT id FROM kb_teams WHERE id = $1 FOR NO KEY UPDATE",
        team_id
    )
    .fetch_optional(&mut *conn)
    .await?;
    if team.is_none() {
        return Err(ApiError::NotFound(
            "team not found or not readable".to_string(),
        ));
    }
    let current = sqlx::query!(
        r#"SELECT role AS "role: TeamRole", source AS "source: TeamMemberSource"
             FROM kb_team_members
            WHERE team_id = $1 AND profile_id = $2
              FOR UPDATE"#,
        team_id,
        *profile,
    )
    .fetch_optional(&mut *conn)
    .await?
    .map(|r| (r.role, r.source));
    let is_machine =
        sqlx::query_scalar!(r#"SELECT is_machine_profile($1) AS "m!: bool""#, *profile)
            .fetch_one(&mut *conn)
            .await?;
    Ok(MembershipTarget {
        team_id,
        profile,
        current,
        is_machine,
    })
}

/// What a recorded membership write did. Every other outcome of the SQL writer is an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MembershipOutcome {
    Added,
    Changed,
    Removed,
    /// Nothing changed and nothing was recorded (C8): a change to the current role.
    Noop,
}

/// Write one authorized membership change and its `team_membership_changed` record, atomically.
///
/// **The only Rust call site of `_admin_team_membership_write`** (plan PR C's tripwire pins it). The
/// caller holds [`locked`]'s locks on the same transaction. `emitter` is the acting person's entity
/// when the caller already resolved it; otherwise it is resolved here from the change's actor
/// (`<handle>@web`, the `access_service::grant_capability` precedent). A system act passes no
/// emitter and the SQL writer emits as the `system` entity — never as the member (plan P4).
///
/// The writer's refusals map to: `last_owner` → 409, `exists` → 409 naming `change_role`,
/// `absent` → 404, `stale` → 409 "membership changed; retry".
pub(crate) async fn membership_write(
    conn: &mut PgConnection,
    emitter: Option<EntityId>,
    change: AuthorizedMembershipChange,
) -> ApiResult<MembershipOutcome> {
    let emitter = match (emitter, change.actor) {
        (Some(emitter), _) => Some(emitter),
        (None, Some(actor)) => Some(
            temper_substrate::writes::resolve_emitter(&mut *conn, actor, "web")
                .await
                .map_err(|e| ApiError::Internal(e.to_string()))?,
        ),
        (None, None) => None,
    };
    let (act, role) = match change.act {
        MembershipAct::Add { role } => ("add", Some(role)),
        MembershipAct::Change { role } => ("change", Some(role)),
        MembershipAct::Remove => ("remove", None),
    };
    let outcome = sqlx::query_scalar!(
        r#"SELECT _admin_team_membership_write(
               $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, NULL) AS "outcome!""#,
        emitter.map(|e| e.uuid()),
        change.actor.map(|a| a.uuid()),
        change.via.as_str(),
        act,
        change.team_id,
        *change.profile,
        role as Option<TeamRole>,
        change.expected.map(|(r, _)| r) as Option<TeamRole>,
        change.expected.map(|(_, s)| s) as Option<TeamMemberSource>,
        Some(change.new_source) as Option<TeamMemberSource>,
        change.guard_last_owner,
        change.invited_by.map(|p| p.uuid()),
    )
    .fetch_one(&mut *conn)
    .await?;
    match outcome.as_str() {
        "added" => Ok(MembershipOutcome::Added),
        "changed" => Ok(MembershipOutcome::Changed),
        "removed" => Ok(MembershipOutcome::Removed),
        "noop" => Ok(MembershipOutcome::Noop),
        "last_owner" => Err(ApiError::Conflict(LAST_OWNER.to_string())),
        "exists" => Err(ApiError::Conflict(ALREADY_A_MEMBER.to_string())),
        "absent" => Err(ApiError::NotFound("team member not found".to_string())),
        "stale" => Err(ApiError::Conflict(MEMBERSHIP_CHANGED.to_string())),
        other => Err(ApiError::Internal(format!(
            "unexpected team membership write outcome {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    //! The rank rule over its whole table, against a HAND-WRITTEN expected outcome per row — never
    //! one recomputed from the same rule (plan B3).

    use super::*;

    const CALLER: Uuid = Uuid::from_u128(1);
    const OTHER: Uuid = Uuid::from_u128(2);
    const TEAM: Uuid = Uuid::from_u128(10);

    /// What a row of the table expects, compared by shape rather than by message.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Want {
        Allow(MembershipVia),
        Forbidden,
        ForbiddenDetail,
        NotFound,
        Conflict,
        BadRequest,
    }

    fn got(r: Result<Decision, ApiError>) -> Want {
        match r {
            Ok(d) => Want::Allow(d.via),
            Err(ApiError::Forbidden) => Want::Forbidden,
            Err(ApiError::ForbiddenDetail(_)) => Want::ForbiddenDetail,
            Err(ApiError::NotFound(_)) => Want::NotFound,
            Err(ApiError::Conflict(_)) => Want::Conflict,
            Err(ApiError::BadRequest(_)) => Want::BadRequest,
            Err(other) => panic!("unexpected refusal {other:?}"),
        }
    }

    fn target(who: Uuid, current: Option<TeamRole>) -> MembershipTarget {
        MembershipTarget {
            team_id: TEAM,
            profile: ProfileId::from(who),
            current: current.map(|r| (r, TeamMemberSource::Native)),
            is_machine: false,
        }
    }

    fn run(
        authority: AuthorityKind,
        gating: bool,
        act: MembershipAct,
        target: MembershipTarget,
    ) -> Want {
        got(decide(
            authority,
            ProfileId::from(CALLER),
            gating,
            act,
            &target,
        ))
    }

    use AuthorityKind as K;
    use MembershipAct as Act;
    use MembershipVia::{MemberAct, SelfAct};
    use TeamRole::{Maintainer, Member, Owner, Watcher};
    use Want::*;

    const O: AuthorityKind = K::Managing(ManagingRole::Owner);
    const MT: AuthorityKind = K::Managing(ManagingRole::Maintainer);

    /// The full table for acts on ANOTHER member of an ordinary team: every authority × act × the
    /// target's current role (or absent). Each expected cell is written out by hand.
    #[test]
    fn acts_on_another_member_of_an_ordinary_team() {
        let roles = [
            Some(Owner),
            Some(Maintainer),
            Some(Member),
            Some(Watcher),
            None,
        ];
        // Columns follow `roles`: owner, maintainer, member, watcher, absent.
        #[rustfmt::skip]
        let table: &[(AuthorityKind, Act, [Want; 5])] = &[
            // ── system admin: unrestricted on rank; existence still checked ──
            (K::Admin, Act::Add { role: Owner },      [Conflict, Conflict, Conflict, Conflict, Allow(MemberAct)]),
            (K::Admin, Act::Change { role: Owner },   [Allow(MemberAct), Allow(MemberAct), Allow(MemberAct), Allow(MemberAct), NotFound]),
            (K::Admin, Act::Change { role: Watcher }, [Allow(MemberAct), Allow(MemberAct), Allow(MemberAct), Allow(MemberAct), NotFound]),
            (K::Admin, Act::Remove,                   [Allow(MemberAct), Allow(MemberAct), Allow(MemberAct), Allow(MemberAct), NotFound]),
            // ── owner: grants owner (C1); never acts on another owner (R6) ──
            (O, Act::Add { role: Owner },             [Conflict, Conflict, Conflict, Conflict, Allow(MemberAct)]),
            (O, Act::Add { role: Watcher },           [Conflict, Conflict, Conflict, Conflict, Allow(MemberAct)]),
            (O, Act::Change { role: Owner },          [ForbiddenDetail, Allow(MemberAct), Allow(MemberAct), Allow(MemberAct), NotFound]),
            (O, Act::Change { role: Maintainer },     [ForbiddenDetail, Allow(MemberAct), Allow(MemberAct), Allow(MemberAct), NotFound]),
            (O, Act::Change { role: Watcher },        [ForbiddenDetail, Allow(MemberAct), Allow(MemberAct), Allow(MemberAct), NotFound]),
            (O, Act::Remove,                          [ForbiddenDetail, Allow(MemberAct), Allow(MemberAct), Allow(MemberAct), NotFound]),
            // ── maintainer: grants up to maintainer (C1); acts on a maintainer (the R6 trap) ──
            (MT, Act::Add { role: Owner },            [Conflict, Conflict, Conflict, Conflict, ForbiddenDetail]),
            (MT, Act::Add { role: Maintainer },       [Conflict, Conflict, Conflict, Conflict, Allow(MemberAct)]),
            (MT, Act::Change { role: Owner },         [ForbiddenDetail, ForbiddenDetail, ForbiddenDetail, ForbiddenDetail, NotFound]),
            (MT, Act::Change { role: Maintainer },    [ForbiddenDetail, Allow(MemberAct), Allow(MemberAct), Allow(MemberAct), NotFound]),
            (MT, Act::Change { role: Watcher },       [ForbiddenDetail, Allow(MemberAct), Allow(MemberAct), Allow(MemberAct), NotFound]),
            (MT, Act::Remove,                         [ForbiddenDetail, Allow(MemberAct), Allow(MemberAct), Allow(MemberAct), NotFound]),
            // ── members and watchers change no one (C3); no authority, nothing at all ──
            (K::SelfOnly(Member), Act::Add { role: Watcher },     [Forbidden; 5]),
            (K::SelfOnly(Member), Act::Change { role: Watcher },  [Forbidden; 5]),
            (K::SelfOnly(Member), Act::Remove,                    [Forbidden; 5]),
            (K::SelfOnly(Watcher), Act::Add { role: Watcher },    [Forbidden; 5]),
            (K::SelfOnly(Watcher), Act::Change { role: Watcher }, [Forbidden; 5]),
            (K::SelfOnly(Watcher), Act::Remove,                   [Forbidden; 5]),
            (K::None, Act::Add { role: Watcher },                 [Forbidden; 5]),
            (K::None, Act::Change { role: Watcher },              [Forbidden; 5]),
            (K::None, Act::Remove,                                [Forbidden; 5]),
        ];
        for (authority, act, wants) in table {
            for (role, want) in roles.iter().zip(wants) {
                assert_eq!(
                    run(*authority, false, *act, target(OTHER, *role)),
                    *want,
                    "{authority:?} {act:?} on another member at {role:?}"
                );
            }
        }
    }

    /// The full table for acts on THEMSELVES, on an ordinary team. The caller's own row is at the
    /// authority's role, as `resolve` would have found it.
    #[test]
    fn acts_on_yourself_on_an_ordinary_team() {
        let to = [Owner, Maintainer, Member, Watcher];
        // Columns follow `to`: change to owner, maintainer, member, watcher; then remove; then add.
        #[rustfmt::skip]
        let table: &[(AuthorityKind, TeamRole, [Want; 6])] = &[
            (O,                    Owner,      [Allow(SelfAct), Allow(SelfAct), Allow(SelfAct), Allow(SelfAct), Allow(SelfAct), Forbidden]),
            (MT,                   Maintainer, [ForbiddenDetail, Allow(SelfAct), Allow(SelfAct), Allow(SelfAct), Allow(SelfAct), Forbidden]),
            (K::SelfOnly(Member),  Member,     [ForbiddenDetail, ForbiddenDetail, ForbiddenDetail, ForbiddenDetail, Allow(SelfAct), Forbidden]),
            (K::SelfOnly(Watcher), Watcher,    [ForbiddenDetail, ForbiddenDetail, ForbiddenDetail, ForbiddenDetail, Allow(SelfAct), Forbidden]),
            // A system admin is not special on themselves (P14 a): the admin arm, checks 3–5 only.
            // Raising their own role is recorded as `member_act`; lowering or leaving as `self`.
            (K::Admin,             Member,     [Allow(MemberAct), Allow(MemberAct), Allow(SelfAct), Allow(SelfAct), Allow(SelfAct), Conflict]),
        ];
        for (authority, own, wants) in table {
            let me = target(CALLER, Some(*own));
            for (role, want) in to.iter().zip(&wants[..4]) {
                assert_eq!(
                    run(*authority, false, Act::Change { role: *role }, me),
                    *want,
                    "{authority:?} at {own:?} changing themselves to {role:?}"
                );
            }
            assert_eq!(
                run(*authority, false, Act::Remove, me),
                wants[4],
                "{authority:?} leaving"
            );
            assert_eq!(
                run(*authority, false, Act::Add { role: *own }, me),
                wants[5],
                "{authority:?} adding themselves"
            );
        }
    }

    /// The gating team (R11, C14, Q4, P14 b): no one but a system admin changes its roster; a
    /// member of any role may still leave it, and may not lower themselves.
    #[test]
    fn the_gating_team_roster_is_admin_only_but_a_member_may_leave() {
        for own in [Owner, Maintainer, Member, Watcher] {
            let s = K::SelfOnly(own);
            let me = target(CALLER, Some(own));
            assert_eq!(
                run(s, true, Act::Remove, me),
                Allow(SelfAct),
                "{own:?} leaves the gating team"
            );
            for to in [Owner, Maintainer, Member, Watcher] {
                assert_eq!(
                    run(s, true, Act::Change { role: to }, me),
                    ForbiddenDetail,
                    "{own:?} → {to:?} on the gating team"
                );
            }
            for current in [Some(Owner), Some(Member), None] {
                for act in [
                    Act::Add { role: Watcher },
                    Act::Change { role: Watcher },
                    Act::Remove,
                ] {
                    assert_eq!(
                        run(s, true, act, target(OTHER, current)),
                        Forbidden,
                        "gating-team {own:?} who is not an admin acting on another ({act:?}, {current:?})"
                    );
                }
            }
        }
        // A system admin changes the gating roster like any other.
        assert_eq!(
            run(
                K::Admin,
                true,
                Act::Add { role: Owner },
                target(OTHER, None)
            ),
            Allow(MemberAct)
        );
        assert_eq!(
            run(K::Admin, true, Act::Remove, target(OTHER, Some(Owner))),
            Allow(MemberAct)
        );
    }

    /// The R6 trap, named: "strictly below" applies ONLY to an owner target.
    #[test]
    fn r6_trap_a_maintainer_acts_on_a_maintainer_and_an_owner_lowers_themselves() {
        assert_eq!(
            run(
                MT,
                false,
                Act::Change { role: Member },
                target(OTHER, Some(Maintainer))
            ),
            Allow(MemberAct)
        );
        assert_eq!(
            run(MT, false, Act::Remove, target(OTHER, Some(Maintainer))),
            Allow(MemberAct)
        );
        // The hand-over: an owner lowers themselves. Decided at check 2, never reaching check 7.
        assert_eq!(
            run(
                O,
                false,
                Act::Change { role: Maintainer },
                target(CALLER, Some(Owner))
            ),
            Allow(SelfAct)
        );
        assert_eq!(
            run(O, false, Act::Remove, target(CALLER, Some(Owner))),
            Allow(SelfAct)
        );
        // ...and an owner may not touch a co-owner.
        assert_eq!(
            run(
                O,
                false,
                Act::Change { role: Maintainer },
                target(OTHER, Some(Owner))
            ),
            ForbiddenDetail
        );
    }

    /// P14 (c): a manager changing themselves to their current role is admitted; the writer turns
    /// it into a recorded-nothing no-op.
    #[test]
    fn p14_a_self_change_to_your_current_role_is_admitted() {
        assert_eq!(
            run(
                O,
                false,
                Act::Change { role: Owner },
                target(CALLER, Some(Owner))
            ),
            Allow(SelfAct)
        );
        assert_eq!(
            run(
                MT,
                false,
                Act::Change { role: Maintainer },
                target(CALLER, Some(Maintainer))
            ),
            Allow(SelfAct)
        );
    }

    /// C5: a machine target above `member` is a 400 for every authority, the admin's included; at
    /// or below `member` it is the ordinary rule. A removal is never refused for being a machine.
    #[test]
    fn a_machine_target_is_capped_for_every_authority() {
        let machine = |current| MembershipTarget {
            is_machine: true,
            ..target(OTHER, current)
        };
        for authority in [K::Admin, O, MT] {
            for role in [Owner, Maintainer] {
                assert_eq!(
                    run(authority, false, Act::Add { role }, machine(None)),
                    BadRequest,
                    "{authority:?} add {role:?}"
                );
                assert_eq!(
                    run(
                        authority,
                        false,
                        Act::Change { role },
                        machine(Some(Member))
                    ),
                    BadRequest,
                    "{authority:?} change {role:?}"
                );
            }
            assert_eq!(
                run(authority, false, Act::Add { role: Member }, machine(None)),
                Allow(MemberAct)
            );
            assert_eq!(
                run(authority, false, Act::Remove, machine(Some(Member))),
                Allow(MemberAct)
            );
        }
        // Authority still comes first: a non-manager learns nothing about the target.
        assert_eq!(
            run(K::None, false, Act::Add { role: Owner }, machine(None)),
            Forbidden
        );
    }

    /// C7: an identity-provider row is refused to every authority, the caller's own row included.
    #[test]
    fn an_idp_row_is_refused_to_every_authority() {
        let idp = |who, role| MembershipTarget {
            current: Some((role, TeamMemberSource::Idp)),
            ..target(who, Some(role))
        };
        for authority in [K::Admin, O, MT] {
            assert_eq!(
                run(
                    authority,
                    false,
                    Act::Change { role: Watcher },
                    idp(OTHER, Member)
                ),
                Conflict
            );
            assert_eq!(
                run(authority, false, Act::Remove, idp(OTHER, Member)),
                Conflict
            );
        }
        // Leaving your own idp row (`idp_member_cannot_leave_their_idp_row`).
        assert_eq!(
            run(K::SelfOnly(Member), false, Act::Remove, idp(CALLER, Member)),
            Conflict
        );
        // Before C7, existence; before both, authority.
        assert_eq!(
            run(K::SelfOnly(Member), false, Act::Remove, idp(OTHER, Member)),
            Forbidden
        );
    }

    /// P14 (a): a system admin with no row may add themselves, at any role, on the ledger as
    /// `member_act`; a manager or member may not add themselves.
    #[test]
    fn a_system_admin_may_add_themselves() {
        for role in [Owner, Maintainer, Member, Watcher] {
            assert_eq!(
                run(K::Admin, false, Act::Add { role }, target(CALLER, None)),
                Allow(MemberAct),
                "admin adds themselves at {role:?}"
            );
            assert_eq!(
                run(K::Admin, true, Act::Add { role }, target(CALLER, None)),
                Allow(MemberAct),
                "admin adds themselves to the gating team at {role:?}"
            );
        }
    }

    /// An authority resolved for one team cannot authorize a write on a target locked on another.
    #[test]
    fn an_authority_resolved_on_one_team_cannot_authorize_on_another() {
        let resolved = ResolvedMembership {
            authority: MembershipAuthority::Managing(ManagingRole::Owner),
            is_gating: false,
            team_id: Uuid::from_u128(11),
            caller: ProfileId::from(CALLER),
        };
        let err = authorize(resolved, Act::Remove, &target(OTHER, Some(Member))).unwrap_err();
        assert!(matches!(err, ApiError::Internal(_)), "{err:?}");
    }
}
