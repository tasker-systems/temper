//! Who may administer a context — the gate a rename passes through.
//!
//! The closest sibling of [`super::read_gates`]: a gate over a subject whose *existence* is
//! withheld from principals who cannot see it, refusing with `NotFound` rather than `Forbidden` for
//! the reason that module's doc records. The difference is that this one answers in **two**
//! dialects — `403` to a principal who can read the context but does not administer it, `404` to
//! one who cannot see it at all — which is what [`ScopedAuthority::denial_for`] exists for. This is
//! its first consumer; `ConnectionAuthority` (`super::connection`) is the second, on the same
//! argument.
//!
//! **This is not an oracle.** The `403` goes only to principals who already read the context — they
//! learn nothing a `GET` would not already have told them. Refusal detail stays bounded by what the
//! caller already has standing to know. That is the argument a later "let's make the denials
//! consistent" pass has to answer before collapsing the two arms into one refusal.
//!
//! Every arm **calls** its incumbent predicate; none restates one, per `super`'s module doc
//! (*What this is, and what it is not*). Design:
//! `temper-artifacts:specs/2026-07-30-context-rename-design.md` §"`ContextAdminAuthority`".

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use temper_substrate::payloads::{AnchorTable, RefTarget};

use super::{personal_space, Principal, ScopedAuthority};
use crate::error::{ApiError, ApiResult};
use crate::services::context_service;
use crate::services::context_service::CONTEXT_REFUSAL;

/// Who may administer a context (rename it, and whatever else joins that class later).
///
/// Two admitting arms and two denying ones, and the denials are **not** interchangeable: they carry
/// different disclosure, so they render differently. See [`ContextAdminAuthority::denial_for`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContextAdminAuthority {
    /// The profile owner of a personal context, or a caller who `can_manage` the owning team.
    Administers,
    /// A system admin, on any context but a personal one (task `01a191b4`). Admits; confers no
    /// ownership.
    SystemAdmin,
    /// Visible to the caller, but not administered by them → `403`.
    ReadOnly,
    /// Not visible to the caller at all → `404`.
    Invisible,
    /// A system admin who does not administer someone else's personal context → `403` that says so
    /// (task `01a1933d`): the admin arm does not reach it, and the admin is told what they hit
    /// rather than sent hunting for a missing row (`super::personal_space`).
    AdminStoppedAtPersonalSpace(personal_space::PersonalSpaceStop),
}

#[async_trait]
impl ScopedAuthority for ContextAdminAuthority {
    /// The bare context id, matching the spelling this seam already uses:
    /// `TwoSidedObject::Context(Uuid)` and `TeamReadAuthority::Subject = Uuid`.
    type Subject = Uuid;

    /// Probe in the spec's order. **Both orderings are load-bearing.**
    ///
    /// Admin sits at **2, not 1**, following the reasoning `TeamReadAuthority` already records
    /// (in `TeamReadAuthority::resolve`): the common caller is the administrator, and probing
    /// `is_system_admin` first charges every one of them an extra round-trip.
    ///
    /// Admin must sit **above 3**, and this is load-bearing. `context_visible_to` →
    /// `context_readable_by_profile` → `contexts_readable_by`, and printed live from the dev
    /// database (plan §G2, a `\sf contexts_readable_by` dump) there is **no system-admin branch**
    /// in any of the four arms — personal context, context owned by an enclosing team, context
    /// shared to an enclosing team, explicit read-grant. A visibility-first ordering would render
    /// `404` to a system admin renaming a context they do not otherwise read — the exact actor the
    /// feature must admit.
    async fn resolve(pool: &PgPool, principal: Principal<'_>, context_id: Uuid) -> ApiResult<Self> {
        let caller = principal.profile_id();
        // 1. The object-side probe, unchanged and not re-derived: profile-owned ⇒ caller *is* the
        //    owner; team-owned ⇒ `can_manage` (Owner|Maintainer) by DIRECT membership. A missing
        //    context answers `false` here, which is why the visibility probe below — not this one —
        //    is what separates `ReadOnly` from `Invisible`.
        if context_service::caller_administers_context(pool, caller, context_id).await? {
            return Ok(ContextAdminAuthority::Administers);
        }
        // 2. System authority admits, and stays its own arm: it never becomes ownership. It stops at
        //    personal space, which only its owner (arm 1) and erasure act on (task `01a191b4`), and
        //    an admin stopped there is told what they hit (task `01a1933d`).
        if principal.system_admin(pool).await?.is_some() {
            let subject = RefTarget {
                kind: AnchorTable::Contexts,
                id: context_id,
            };
            return Ok(
                match personal_space::others_personal_space(pool, subject, caller).await? {
                    Some(stop) => ContextAdminAuthority::AdminStoppedAtPersonalSpace(stop),
                    None => ContextAdminAuthority::SystemAdmin,
                },
            );
        }
        // 3/4. The one visibility predicate, called through the one Rust spelling of it.
        Ok(
            if context_service::context_visible(pool, *caller, context_id).await? {
                ContextAdminAuthority::ReadOnly
            } else {
                ContextAdminAuthority::Invisible
            },
        )
    }

    /// Two arms, not one — and that is the whole reason `denial_for` was widened.
    fn is_denial(&self) -> bool {
        matches!(
            self,
            ContextAdminAuthority::ReadOnly
                | ContextAdminAuthority::Invisible
                | ContextAdminAuthority::AdminStoppedAtPersonalSpace(_)
        )
    }

    fn personal_space_stop(&self) -> Option<personal_space::PersonalSpaceStop> {
        match self {
            ContextAdminAuthority::AdminStoppedAtPersonalSpace(stop) => Some(*stop),
            _ => None,
        }
    }

    /// The **safe fallback**: the more withholding of this domain's two refusals.
    ///
    /// `denial` is static and so cannot know which arm refused; rendering the `403` here would hand
    /// the disclosive answer to any caller that reaches the static path. `NotFound` with
    /// `CONTEXT_REFUSAL` — the incumbent constant from `context_service`, never a second literal —
    /// makes an administration refusal byte-identical to `get_visible`'s and
    /// `resolve_context_ref`'s, which is what
    /// `the_three_handle_slug_refusals_are_indistinguishable` asserts.
    fn denial() -> ApiError {
        ApiError::NotFound(CONTEXT_REFUSAL.to_string())
    }

    /// One gate, two dialects — dispatched on the arm that actually refused.
    ///
    /// `ReadOnly` → `Forbidden`: the caller demonstrably reads this context, so refusing with a
    /// `404` would lie to them about a row they can `GET`. `Invisible` → the incumbent `404`, whose
    /// existence-hiding is the whole point.
    ///
    /// The admitting arms are unreachable — `authorize` calls this only when `is_denial` — and
    /// delegate to [`Self::denial`] rather than panicking: an `unreachable!()` would convert a
    /// future mis-wiring into a crash, where the safe fallback converts it into the *more*
    /// withholding refusal.
    fn denial_for(&self) -> ApiError {
        match self {
            ContextAdminAuthority::ReadOnly => ApiError::Forbidden,
            _ => Self::denial(),
        }
    }
}

#[cfg(all(test, feature = "test-db"))]
mod tests;
