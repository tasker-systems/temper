//! Authorization for machine-client registration (G3 Phase B2).
//!
//! Two things live here, and the separation is the point:
//!
//! 1. **Who may register** — [`authorize`]: a system admin (under D11 a `kb_principal_governance`
//!    grant), or the OWNER of the team that will own the machine. Two distinct authorities — a
//!    governance grant and team ownership — either of which admits, except that on someone
//!    else's personal team only its owner does (task `01a191b4`; `authz/machine.rs`).
//! 2. **What reach they may confer** — [`AuthorizedReach`]: a value that only this module can
//!    construct. `apply_reach` takes it instead of raw specs, so reach cannot be applied
//!    without having been authorized. The invariant is enforced by the type, not by a comment
//!    — which is what the Phase A comment on `apply_reach` asked for and could not get.
//!
//! The containment bar is the *human* bar, reached by CALLING the human predicates rather
//! than restating them: teams need `can_manage` (what `add_member` requires); grants need
//! `can_grant` (what `grant_capability` requires of a non-admin). Tighten the human surface
//! and the machine surface tightens with it — there is no second copy of the policy to drift.
//!
//! The **role bar** is deliberately NOT part of containment (D4b). Containment asks whether the
//! reach is a subset of the caller's own, and D3 answers "unchecked" for an admin (outside
//! someone else's personal cognitive map, which the grant door stops it at); whether a
//! machine may hold a given role at all is a different question, answered `no` for every caller
//! above [`MAX_MACHINE_TEAM_ROLE`]. So it runs in [`authorize_registration`], on both arms.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::machine::{GrantSpec, TeamSpec};
use temper_core::types::team::TeamRole;

use crate::auth::HumanPrincipal;
use crate::error::{ApiError, ApiResult};
use crate::services::{access_service, team_service};

/// The highest team role a machine principal may ever hold (D4b, tightened 2026-07-20).
///
/// `member` confers RW in team contexts, which is the whole of what an M2M credential needs.
/// Everything above it is *governance*: `team_service::can_manage` is `Owner | Maintainer`, so a
/// machine one notch higher could manage team membership, bind cogmaps, and reassign contexts —
/// and on the gating team, `owner`/`maintainer` clears `can_manage` over the very team that gates
/// membership. (Under D11 that no longer *manufactures* an `is_system_admin` principal — admin-ness
/// is a `kb_principal_governance` grant, a separate axis — but managing the gating team's roster is
/// still governance no unattended credential should hold.) There is no machine workload that wants
/// any of that, and an unattended credential is the worst possible holder of it.
///
/// Compared by [`TeamRole::rank`] rather than matched as a set, so a role introduced above
/// `member` is barred by construction instead of by remembering to edit a `matches!` arm.
///
/// **It bounds every `kb_team_members` row a machine holds, however the row came about** —
/// registration reach, `team_service::add_member` / `change_role`, `access_service::promote_admin`,
/// and its own personal team, which it holds as `member` like any other (ruled 2026-10-09). The
/// member acts refuse through the rank rule's target-kind check (`authz::team_membership`, check 4,
/// for every authority); registration refuses in [`authorize_registration`]; the database refuses
/// the same rows underneath them (`20261019100000_machines_never_govern.sql`), and caps a
/// profile's existing roles the moment it becomes a machine — which is how registration's
/// personal-team `owner` row, written before the client row exists, comes out as `member`.
const MAX_MACHINE_TEAM_ROLE: TeamRole = TeamRole::Member;

/// Whether `role` is above what any machine may hold.
pub(crate) fn exceeds_machine_ceiling(role: TeamRole) -> bool {
    role.rank() > MAX_MACHINE_TEAM_ROLE.rank()
}

/// The caller's authority over a machine registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MachineRegistrationAuthority {
    /// A system admin (a `kb_principal_governance` grant). Full, unchecked reach (Phase A D5),
    /// except a cogmap grant on someone else's personal map (task `01a1933d`).
    SystemAdmin,
    /// Owner of the team that owns (or will own) the machine. Reach is contained.
    TeamOwner,
    /// No authority over this machine. Denial is an ARM rather than an `Err` returned from inside
    /// the resolve, so it flows through `ScopedAuthority::denial` like every other domain's
    /// refusal instead of short-circuiting past it.
    None,
}

/// Resolve who the caller is with respect to a registration owned by `team`.
///
/// **Fails closed on `None`** (spec D2): a teamless machine (`team_id IS NULL`) is admin-only
/// to create, read, or operate. "No team to check" must never mean "nothing to deny".
pub(crate) async fn authorize(
    pool: &PgPool,
    caller: crate::authz::Principal<'_>,
    team: Option<Uuid>,
) -> ApiResult<MachineRegistrationAuthority> {
    // The policy itself is this enum's `ScopedAuthority` impl (`authz/machine.rs`). This wrapper
    // keeps the non-denial arms as a bare value for the two callers that branch on them, while the
    // gate renders refusal — so a denial still surfaces as `Forbidden`, from one place.
    Ok(
        crate::authz::authorize::<MachineRegistrationAuthority>(pool, caller, team)
            .await?
            .authority(),
    )
}

/// ONE authorized cogmap grant out of a reach.
///
/// Sealed the same way [`AuthorizedReach`] is — private field, no constructor outside this module —
/// but at the **item** grain, and that grain is the point. `apply_reach` loops over the grants and
/// writes one `kb_access_grants` row per item, so the subject of each row comes from the item, not
/// from the reach. Handing the write primitive a `&AuthorizedGrant` therefore makes the per-row
/// subject *structural*: there is no runtime "is this grant really a member of that reach?" check to
/// write, because a caller cannot hold one that isn't.
///
/// A whole-reach proof would not do this. `&AuthorizedReach` plus a separately-named cogmap id is
/// exactly the two-spellings shape `Authorized<A>` exists to remove, one level down.
/// Holds the two authorized values directly rather than wrapping a `GrantSpec`: the spec is a
/// deserialized wire type, and a proof should not be one field-addition away from carrying something
/// nobody authorized.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AuthorizedGrant {
    cogmap_id: Uuid,
    can_write: bool,
}

impl AuthorizedGrant {
    /// The map this grant is over — the subject of the row it authorizes, read from the proof.
    pub(crate) fn cogmap_id(&self) -> Uuid {
        self.cogmap_id
    }

    /// Whether the authorized reach included write. Read is implied (the DB coherence CHECK
    /// enforces `write ⇒ read` anyway).
    pub(crate) fn can_write(&self) -> bool {
        self.can_write
    }
}

/// Reach that has been authorized against a caller's authority (spec D3).
///
/// The fields are private to this module and there is no public constructor, so an
/// `AuthorizedReach` can only come from [`authorize_registration`]. `apply_reach` takes this
/// type, which makes the unchecked path *unrepresentable* rather than merely discouraged.
///
/// `grants` is **owned** where `teams` is borrowed: each grant is re-wrapped as a sealed
/// [`AuthorizedGrant`] at authorization time, which cannot be done through a borrow of the caller's
/// `GrantSpec` slice. A reach carries a handful of grants on a path that runs once per machine
/// registration, so the allocation is not worth contorting the type to avoid.
///
/// `registrar` is the person the reach was authorized FOR. It rides in the proof, not beside it, so
/// `apply_reach` re-checks the membership bar under each team lock for exactly that person
/// (task `01a14592-3f83`) and records exactly them as actor. The membership write cannot be handed
/// a different registrar from the one the reach was authorized for.
#[derive(Debug)]
pub(crate) struct AuthorizedReach<'a> {
    registrar: &'a HumanPrincipal,
    teams: &'a [TeamSpec],
    grants: Vec<AuthorizedGrant>,
    /// Whether the reach was authorized on the system-admin arm, read on the pool. The
    /// registration's transaction asks it again under the governance lock
    /// ([`Self::recheck_admin_arm`]) before its first write.
    on_admin_arm: bool,
    /// The team that will own the machine, as authorized. Carried so the owner arm's re-check
    /// ([`Self::recheck_owner_arm`]) asks about exactly the team the pool check admitted.
    owning_team: Option<Uuid>,
}

impl<'a> AuthorizedReach<'a> {
    /// The person this reach was authorized for.
    pub(crate) fn registrar(&self) -> &'a HumanPrincipal {
        self.registrar
    }

    pub(crate) fn teams(&self) -> &'a [TeamSpec] {
        self.teams
    }

    pub(crate) fn grants(&self) -> &[AuthorizedGrant] {
        &self.grants
    }

    /// The admin re-check of a registration, on its transaction, before its first write. A reach
    /// authorized on the system-admin arm was decided by a plain pool read of governance; the
    /// registration holds only while the registrar is still an admin, so it is asked again with
    /// the locked read ([`crate::auth::is_system_admin_locked`]): a demotion committed since
    /// refuses with the arm's own denial (`403`, what `authorize` gives a caller with no
    /// authority), and one in flight makes this wait, then refuses. A demoted admin who also owns
    /// the team is refused too; a retry decides on the owner arm, with its containment.
    ///
    /// A reach on the owner arm has nothing to re-check here: its team memberships are decided
    /// again under each team lock (`apply_reach`), and nothing about it reads governance.
    pub(crate) async fn recheck_admin_arm(&self, tx: &mut sqlx::PgConnection) -> ApiResult<()> {
        if self.on_admin_arm
            && !crate::auth::is_system_admin_locked(tx, self.registrar.profile_id()).await?
        {
            return Err(ApiError::Forbidden);
        }
        Ok(())
    }

    /// The owner arm's re-check of a registration, on its transaction, before its first write. A
    /// reach authorized on the team-owner arm was decided by pool reads: ownership of the owning
    /// team, and the grant half of `contain_reach`. Both leave something the registrar keeps, a
    /// credential owned by that team and grants that reach a machine they hold the secret of, so
    /// both are asked again here:
    ///
    /// - ownership of the owning team, under its row lock (with the reach's teams, in one ascending
    ///   pass, so `apply_reach` takes none of them out of that order afterwards). An owner removed
    ///   or demoted since the pool check, or while this runs, is refused with the arm's own denial
    ///   (`403`, what `authorize` gives a caller who owns nothing there);
    /// - each grant's bar, with the registrar's own grant rights on the map share-locked
    ///   ([`access_service::callers_machine_grant_holds_on`]): a `can_grant` withdrawn in flight
    ///   makes this wait, then refuses.
    ///
    /// A reach on the admin arm has nothing to re-check here; [`Self::recheck_admin_arm`] asks it.
    pub(crate) async fn recheck_owner_arm(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    ) -> ApiResult<()> {
        if self.on_admin_arm {
            return Ok(());
        }
        let Some(team) = self.owning_team else {
            return Err(ApiError::Forbidden);
        };
        let reach_teams: Vec<Uuid> = self.teams.iter().map(|spec| spec.team_id).collect();
        let role = crate::authz::team_membership::role_under_lock(
            tx,
            team,
            self.registrar.profile_id(),
            &reach_teams,
        )
        .await?;
        if role != Some(TeamRole::Owner) {
            return Err(ApiError::Forbidden);
        }
        for grant in &self.grants {
            if !access_service::callers_machine_grant_holds_on(
                tx,
                self.registrar.profile_id(),
                grant.cogmap_id,
                grant.can_write,
            )
            .await?
            {
                return Err(ApiError::Forbidden);
            }
        }
        Ok(())
    }

    /// Seal a slice of specs. Private, and the ONLY way an `AuthorizedGrant` comes into existence —
    /// co-located with the two `authorize_registration` arms that have earned the right to call it.
    fn seal(grants: &[GrantSpec]) -> Vec<AuthorizedGrant> {
        grants
            .iter()
            .map(|spec| AuthorizedGrant {
                cogmap_id: spec.cogmap_id,
                can_write: spec.can_write,
            })
            .collect()
    }
}

/// Authorize a registration and the reach it asks for, in that order.
///
/// A system admin gets the Phase A D5 bypass — named here, so the bypass is visible at this
/// call site instead of being implicit in the absence of a check.
pub(crate) async fn authorize_registration<'a>(
    pool: &PgPool,
    authed: &'a HumanPrincipal,
    team: Option<Uuid>,
    teams: &'a [TeamSpec],
    grants: &'a [GrantSpec],
) -> ApiResult<AuthorizedReach<'a>> {
    let authority = authorize(pool, crate::authz::Principal::Human(authed), team).await?;

    // The caller is authorized; now the payload must be well-formed. An unknown role would
    // otherwise fail the `::team_role` enum cast deep inside `apply_reach`'s transaction and
    // surface as a 500 — a malformed request is a 400. Runs on both paths (an admin's bad role
    // is a 400 too). Validating AFTER authorize means an unauthorized caller still gets 403,
    // not a hint about their payload.
    for spec in teams {
        let role = parse_team_role(&spec.role)?;

        // D4b — the ROLE CEILING, on BOTH arms including the admin's. It sits here rather than
        // in `contain_reach` because it is not a containment question: containment asks *is this
        // reach a subset of the caller's own?*, and D3 deliberately answers "unchecked" for an
        // admin. This asks something else — *may a machine hold this role at all?* — and the
        // answer is no above `MAX_MACHINE_TEAM_ROLE` for every caller, unconditionally.
        //
        // The ceiling is `member` because that is where *governance* starts: `can_manage` is
        // `Owner | Maintainer`, so anything higher lets an unattended credential manage team
        // membership, bind cogmaps, and reassign contexts. Under D11 gating-team
        // `owner` no longer mints an `is_system_admin` principal (governance grant
        // is separate), but the role ceiling is still enforced because a governing
        // role on any team is too much authority for an unattended credential. This check is
        // what keeps a registration from conferring such a role at MINT; `apply_reach` routes
        // the write through the membership writer (plan C3, `from_reach`), and the database
        // trigger refuses it too.
        //
        // Reach stays unchecked for admins (D3 preserved) — an admin may still put a machine on
        // any team it likes, at any role up to the ceiling.
        if exceeds_machine_ceiling(role) {
            return Err(ApiError::Forbidden);
        }
    }

    match authority {
        // Phase A D5: a system admin may confer any reach on a machine — but not on someone
        // else's personal cognitive map (task `01a1933d`). A reach grant is a grant on the map,
        // so it asks the grant door's own decision, which stops the admin arm there and tells the
        // admin what they hit. Everywhere else that decision resolves `SystemAdmin`, unattenuated,
        // so D3's "reach unchecked for admins" holds outside personal space.
        MachineRegistrationAuthority::SystemAdmin => {
            for grant in grants {
                access_service::authorize_capability_grant(
                    pool,
                    crate::authz::Principal::Human(authed),
                    temper_substrate::payloads::RefTarget {
                        kind: temper_substrate::payloads::AnchorTable::Cogmaps,
                        id: grant.cogmap_id,
                    },
                    access_service::RequestedCapabilities {
                        read: true,
                        write: grant.can_write,
                        delete: false,
                        grant: false,
                    },
                )
                .await?;
            }
            Ok(AuthorizedReach {
                registrar: authed,
                teams,
                grants: AuthorizedReach::seal(grants),
                on_admin_arm: true,
                owning_team: team,
            })
        }
        MachineRegistrationAuthority::TeamOwner => {
            contain_reach(pool, authed, teams, grants).await?;
            Ok(AuthorizedReach {
                registrar: authed,
                teams,
                grants: AuthorizedReach::seal(grants),
                on_admin_arm: false,
                owning_team: team,
            })
        }
        // Unreachable — `authorize` refuses a denial before returning. Enumerated rather than
        // `_ =>` so that a future arm cannot land here and fall into an authorizing branch.
        MachineRegistrationAuthority::None => Err(ApiError::Forbidden),
    }
}

/// Parse a wire role string into a `TeamRole`, the enum's own serde as the single source of
/// truth. Unknown roles are a 400 here rather than a 500 from the downstream enum cast.
pub(crate) fn parse_team_role(role: &str) -> ApiResult<TeamRole> {
    serde_json::from_value::<TeamRole>(serde_json::Value::String(role.to_string()))
        .map_err(|_| ApiError::BadRequest(format!("unknown team role '{role}'")))
}

/// The containment bar for a **single target team** that is about to receive something — the
/// non-machine sibling of [`contain_reach`]'s team loop, for callers that confer team reach one
/// team at a time (`connection_service::grant_reach`).
///
/// Two teams are in play and they are different questions. `authorize` asks *may you act on this
/// connection?* — keyed on the connection's OWNING team. This asks *may you hand read-reach to
/// THAT team?* — keyed on the target. Without it, an owner of one team could bind their
/// connection's reach to any team UUID in the instance.
///
/// The bar is `can_manage`, called through the shared `require_manage_on_team` so it cannot drift
/// from [`contain_reach`]. A system admin is exempt (Phase A D5) but still has the target team's
/// existence checked — the D5 bypass is about authority, not about writing a `principal_id` that
/// points at nothing. The exemption stops at a personal team: there the bar is its owner's, for
/// every caller (task `01a19006`). A personal team is refused outright to a non-admin, their own
/// included. A team owner's own personal team is them under another name, as at the grant door, so
/// read-reach bound to it would outlive their hold on the connection's owning team (S3 residual
/// route 2, closed 2026-11-01, decision `01a19843-b86b-766f-b06e-65560b0f6f6f`). Two cases stay
/// open. A system admin may still bind to their own personal team. A team owner may bind a
/// connection their personal team itself owns (`owning_team`) to that same team, because that hold
/// cannot be lost.
///
/// **There is deliberately NO gating-team exclusion here**, unlike the two other two-sided gates
/// (`cogmap_service::can_bind`, `context_service::can_share`), which both refuse the gating team as a
/// target. The asymmetry is correct: those two guard acts that change what a *shared* object is
/// subject to — a cogmap binding IS the admin-write-regime switch, and a context reassign transfers
/// ownership into the root team. A reach grant does neither. It writes one `kb_access_grants` row
/// (`subject_table = 'kb_connections'`) conferring READ on what the caller's own connection receives;
/// it flips no regime, transfers no ownership, and exposes only the granter's data. Post-D11 the
/// gating team confers neither standing nor admin-ness, so it is, for this act, an ordinary team.
///
/// Recorded as a decision rather than left to read as an oversight, and pinned by
/// `connection_service::tests::reach_to_the_gating_team_is_allowed_for_a_non_admin`. Full
/// reasoning for all three gates:
/// `temper-artifacts:specs/2026-07-22-scoped-authority-policy-layer-design.md` §6.1.
///
/// **The root is different, and is refused to a non-admin** (team DAG review G14). `temper-system`
/// is every personal team's parent, so a reach grant onto it hands the connection's data to every
/// profile on the instance. That is publication to the instance, which the grant door already
/// refuses a delegate for any subject (`access_service::recipient_bar`) — and a reach grant is a
/// grant row by another door. The gating team keeps its exemption above; only the root is excluded.
pub(crate) async fn contain_target_team(
    pool: &PgPool,
    authority: MachineRegistrationAuthority,
    caller: ProfileId,
    team_id: Uuid,
    owning_team: Option<Uuid>,
) -> ApiResult<()> {
    // A personal team is acted on by its owner alone (task `01a19006`): binding read-reach to one
    // is held to the owner's bar whatever the caller's authority on the connection. And only a
    // system admin's authority on the connection reaches even their own (S3 residual route 2,
    // closed by Pete's ruling of 2026-11-01, decision `01a19843-b86b-766f-b06e-65560b0f6f6f`): for a
    // team owner, their personal team is a second spelling of themselves, as at the grant door
    // (`access_service::recipient_bar`), so the reach would outlive their hold on the owning team.
    if team_service::personal_of(pool, team_id).await?.is_some() {
        team_service::require_manage_on_team(pool, team_id, caller).await?;
        return match authority {
            MachineRegistrationAuthority::SystemAdmin => Ok(()),
            // A connection the personal team itself owns is the one case left open: its owner can
            // never lose that hold, so nothing outlives anything, and without reach they could not
            // read their own connection at all.
            MachineRegistrationAuthority::TeamOwner if owning_team == Some(team_id) => Ok(()),
            MachineRegistrationAuthority::TeamOwner | MachineRegistrationAuthority::None => {
                Err(ApiError::Forbidden)
            }
        };
    }
    match authority {
        MachineRegistrationAuthority::SystemAdmin => {
            team_service::require_team_exists(pool, team_id).await
        }
        MachineRegistrationAuthority::TeamOwner => {
            team_service::require_manage_on_team(pool, team_id, caller).await?;
            // After the manage bar, so a caller who manages nothing there learns nothing more.
            if access_service::is_root_team(pool, team_id).await? {
                return Err(ApiError::Forbidden);
            }
            Ok(())
        }
        // Unreachable for the same reason as `authorize_registration`'s arm, and enumerated for
        // the same reason.
        MachineRegistrationAuthority::None => Err(ApiError::Forbidden),
    }
}

/// The non-admin containment bar. Every check calls an existing human-surface predicate.
async fn contain_reach(
    pool: &PgPool,
    authed: &HumanPrincipal,
    teams: &[TeamSpec],
    grants: &[GrantSpec],
) -> ApiResult<()> {
    // The D4b role ceiling is NOT here: it asks a question about the role itself, not about
    // containment, so it runs for BOTH arms in `authorize_registration`. Do not restore a copy —
    // a second copy is what would let the two arms drift apart again.
    // D4 — the membership bar: exactly what `add_member` requires of a human. That is the
    // membership authority's managing arm (team-role rank plan P15), which also refuses a
    // non-admin on the gating team or temper-system (R11): an owner there who is not a system admin can no more
    // put a machine on that roster than add a person to it. (The admin arm returned before this
    // function runs; the rank bound is moot, since D4b caps a machine at `member`.)
    // `contain_target_team` keeps `require_manage_on_team`: a reach grant writes no membership.
    // This is the POOL pre-check (plan P1 step 1), so a non-manager never takes a team lock. The
    // same bar runs again under each team lock in `from_reach`, which is the one that decides: a
    // registrar demoted or removed between here and the write is refused there.
    if !teams.is_empty() {
        let mut conn = pool.acquire().await?;
        for spec in teams {
            let resolved = crate::authz::team_membership::MembershipAuthority::resolve_precheck(
                &mut conn,
                authed,
                spec.team_id,
            )
            .await?;
            if !resolved.manages() {
                return Err(ApiError::Forbidden);
            }
        }
    }

    for grant in grants {
        // D4 — exactly what `grant_capability` requires of a non-admin, by CALLING that decision
        // rather than restating it. This previously checked `profile_can_grant` alone, which WAS
        // parity until 5b.3/5b.4 hardened the human path and left this one behind. The divergence
        // was exploitable in the same class this arc exists to close: a `read+grant`-without-write
        // holder could provision a machine carrying `can_write` and then command a principal
        // holding write they can never hold themselves (laundering by proxy), and a non-admin
        // `can_grant` holder on the L0 kernel could do the same there, walking around 5b.4's
        // admin-only guard. Sharing the decision is what stops it recurring.
        //
        // `apply_reach` confers exactly `read + (write iff spec)`, never delete or grant — so that
        // is what is attenuated here. The admin arm asks the same decision separately, so this
        // resolves to Delegated or None in practice.
        let proof = access_service::authorize_capability_grant(
            pool,
            crate::authz::Principal::Human(authed),
            temper_substrate::payloads::RefTarget {
                kind: temper_substrate::payloads::AnchorTable::Cogmaps,
                id: grant.cogmap_id,
            },
            access_service::RequestedCapabilities {
                read: true,
                write: grant.can_write,
                delete: false,
                grant: false,
            },
        )
        .await?;
        // v0.6.3 review S3: the machine being registered is the caller's own (it records them as
        // `registered_by_profile_id`), so a delegated reach grant is held to the bar the grant door
        // applies to a grant to the caller's machine. One rule, called from both doors. A right
        // held only through a team is refused, because this row would outlive the caller's
        // membership. A right held in the caller's own name, such as a map's creator's, is admitted.
        if matches!(proof.authority(), access_service::GrantAuthority::Delegated) {
            access_service::callers_machine_grant_bar(pool, authed.profile_id(), proof.subject())
                .await?;
        }
    }

    Ok(())
}

#[cfg(all(test, feature = "test-db"))]
mod tests;
