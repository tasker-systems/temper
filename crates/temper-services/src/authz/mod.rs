//! Scoped authorization — "may *this* caller do *this* to *this* scoped thing?"
//!
//! The sibling of [`crate::auth`], and deliberately a different question. `auth` answers
//! **system authority**: a boolean about the principal's own role, with no scope
//! (`SystemAdmin` is nullary). This module answers **scoped capability**: a disjunction over
//! roles scoped to a particular object — system admin as one branch, team owner or resource
//! grantee as others. Forcing scoped gates behind `SystemAdmin` would deny the very actors they
//! exist to admit; that distinction is load-bearing and is why these are two modules, not one.
//!
//! # What this is, and what it is not
//!
//! This layer is a **router over SQL predicates, not a replacement for them**. Temper's
//! authorization lives in the database (`resources_visible_to`, `can_modify_resource`, `can`,
//! `is_system_admin`), and it stays there. `ScopedAuthority::resolve` *calls* those predicates
//! in sequence; it never restates them. A resolve body that inlines a policy the database already
//! names is a second copy that will drift from the one it was meant to mirror.
//!
//! # Why it exists
//!
//! Three domains had independently grown the same shape — resolve the caller's authority over a
//! subject into a typed enum, then act under it (`GrantAuthority`, `MachineAuthority`, and
//! temper-principal's `ActorAuthority`) — and one of them, `AuthorizedReach`, had also grown the
//! sealed proof-about-a-value that makes the unchecked path unrepresentable. This module names
//! that shape once so the remaining gates inherit it instead of re-deriving it.
//!
//! Design: `docs/superpowers/specs/2026-07-22-scoped-authority-policy-layer-design.md`.

mod audit_gate;
mod connection;
mod context_admin;
mod grant;
mod machine;
mod read_gates;
mod two_sided;

// Wired by Task 7: `DbBackend::record_citation_audit` derives the subject with `finding_of_block`
// and gates on `AuditAuthority`. That backend command is the single chokepoint every surface funnels
// through, so it is the only caller of `authorize::<AuditAuthority>` — a second one on a surface
// would be a double gate that can drift from this one.
//
// `finding_of_block` itself gained a second caller in Task 8: `citation_audit_service` reuses it
// (never `authorize`) to refuse a path/block mismatch before dispatching to the backend above —
// the one spelling of the lookup, used at both of `audit_gate.rs`'s named call sites.
//
// The fix wave added the other two Set 5 authorization decisions to the same module, so all three
// share one `kb_machine_clients` probe: `AuditorJobAuthority` (job completion — spec §6.5, formerly
// a hand-rolled `sqlx::query_scalar!` inside the backend command and the only Set 5 gate outside
// this layer) and `require_machine_principal` (the dispatch tick, which has no subject and so is
// deliberately NOT a `ScopedAuthority`).
pub(crate) use audit_gate::{
    citation_subject, finding_of_block, require_machine_principal, AuditAuthority,
    AuditorJobAuthority, FINDING_REFUSAL,
};
pub(crate) use connection::{ConnectionAuthority, ConnectionControlAuthority, ConnectionScope};
// The first authority to declare two refusal dialects, and so the first (and only) consumer of
// `ScopedAuthority::denial_for`. Wired by Task 6: `context_service::rename` is its single gate.
pub(crate) use context_admin::ContextAdminAuthority;
pub(crate) use grant::{wire_subject, BornSubject, GrantWarrant, RevokeWarrant};
pub(crate) use read_gates::{ActorHistoryAuthority, TeamReadAuthority, ACTOR_HISTORY_REFUSAL};
pub(crate) use two_sided::{TwoSidedAuthority, TwoSidedScope};

use async_trait::async_trait;
use sqlx::PgPool;
use std::fmt::Debug;

use temper_core::types::ids::ProfileId;

use crate::error::{ApiError, ApiResult};

/// A domain's answer to "what authority does this caller hold over this subject?"
///
/// Implemented by each domain's own authority enum. The arms stay domain-specific on purpose:
/// `GrantAuthority::Delegated` carries an attenuation obligation that `MachineAuthority::TeamOwner`
/// does not, and collapsing them into one shared enum would erase a distinction the compiler is
/// currently keeping for us. Same shape, different intent, separate types.
#[async_trait]
pub(crate) trait ScopedAuthority: Sized + Copy + Debug {
    /// What this authority is *about* — the scope the answer is bound to.
    ///
    /// `Copy` so `Authorized::subject` can hand it back without cloning, which matters because
    /// the whole point is that acts read the subject from the proof rather than from their own
    /// arguments. A `Subject` that had to be cloned would invite callers to keep their own copy.
    type Subject: Copy + Debug;

    /// Resolve the caller's authority over `subject`.
    ///
    /// Sequenced probes, short-circuiting: return the strongest arm as soon as it is established,
    /// so the common path does not pay for the branches below it. SQL predicates are authoritative
    /// here — call them, do not restate them.
    async fn resolve(pool: &PgPool, caller: ProfileId, subject: Self::Subject) -> ApiResult<Self>;

    /// Is this arm a denial?
    ///
    /// Denial is an **arm every domain must name**, never an absence and never an `Err` returned
    /// from inside `resolve`. An error short-circuits `authorize` before `denial` runs,
    /// which would silently bypass the domain's chosen refusal dialect below.
    fn is_denial(&self) -> bool;

    /// How this domain renders a refusal.
    ///
    /// **Not boilerplate, and not always `Forbidden`.** Some gates refuse with `NotFound` on
    /// purpose, because the existence of the subject is itself the secret: `team_detail` returns
    /// `NotFound` to non-members since *"team slugs are globally unique and used in share flows"*
    /// (`team_service.rs:277`), and the admin ledger's actor axis does the same. Hardcoding
    /// `Forbidden` in `authorize` would convert those deliberate information-hiding decisions
    /// into existence leaks. If you are tempted to "simplify" this method away, that is what you
    /// would be removing.
    fn denial() -> ApiError;

    /// How this domain renders a refusal **for the arm that refused** — one gate, two dialects.
    ///
    /// [`denial`](Self::denial) gives a domain exactly one refusal, which is all most of them want.
    /// Some gates need two: a context rename answers `403` to a principal who can *read* the context
    /// but not administer it, and `404` to one who cannot see it at all. That is not an oracle — the
    /// `403` goes only to callers who already read the subject, so it discloses nothing a `GET` would
    /// not already have told them — but it cannot be spelled by a method that does not know which arm
    /// it is refusing from. This is a **dispatch widening, not a replacement**: `denial` stays, the
    /// default here delegates to it, and every authority that wants one dialect keeps saying so once.
    ///
    /// **Overriding this on an existing authority changes that authority's refusal voice.** That is a
    /// behavioral change to a shipped gate, not a refactor, and it is guarded: the boundary suite in
    /// this module asserts that for every authority, every denial arm's `denial_for` renders
    /// identically to its `denial` — same `ApiError` discriminant *and* same message — for all but the
    /// authorities that deliberately declare two dialects. If that suite reds, you changed a refusal
    /// someone else's tests are downstream of.
    ///
    /// The property the static signature bought us survives intact:
    ///
    /// > `&self` exposes the **arm enum**, which carries no subject data. A refusal still
    /// > structurally cannot name the subject it refused.
    ///
    /// Which is why this takes `&self` and not `Self::Subject`. Widening it to the subject would
    /// reopen the question `read_gates.rs:68-70` records as already settled — whether the ambiguity
    /// in "team not found or not readable" is preserved by the signature, or by everyone remembering
    /// to preserve it.
    fn denial_for(&self) -> ApiError {
        Self::denial()
    }
}

/// Proof that `authority` was resolved for `subject`, and that it is not a denial.
///
/// SEALED: both fields are private and `authorize` is this module's only constructor, so a
/// struct-literal forgery elsewhere in this crate is a compile error. It is the same kind of thing
/// as `crate::auth::SystemAdmin` — a proof obtainable only from the gate that establishes it —
/// and it generalizes the one `machine_authz::AuthorizedReach` already proved out.
///
/// The subject travels *inside* the proof. That is the difference that matters: a gate which
/// merely returned an authority value would leave the act free to name its own subject, so
/// authorizing `S` and then acting on `S′` would be a transposition no compiler could see.
#[derive(Debug)]
pub(crate) struct Authorized<A: ScopedAuthority> {
    authority: A,
    subject: A::Subject,
}

impl<A: ScopedAuthority> Authorized<A> {
    /// Which arm admitted the caller — for acts whose behavior differs by arm (attenuation binds a
    /// delegated administrator but not a system admin).
    pub(crate) fn authority(&self) -> A {
        self.authority
    }

    /// **The only subject this act may touch.** Read the scope from here, never from a parameter
    /// carried alongside the proof — a second spelling is a transposition waiting to happen, and
    /// removing it is the reason the subject is sealed in at all.
    pub(crate) fn subject(&self) -> A::Subject {
        self.subject
    }
}

/// The gate: resolve, refuse denials in the domain's own dialect, seal the pair.
///
/// Co-located with `Authorized` because it must be — the private fields mean only this module
/// can construct one, so the gate that mints the proof lives beside the type that carries it.
pub(crate) async fn authorize<A: ScopedAuthority>(
    pool: &PgPool,
    caller: ProfileId,
    subject: A::Subject,
) -> ApiResult<Authorized<A>> {
    let authority = A::resolve(pool, caller, subject).await?;
    if authority.is_denial() {
        return Err(authority.denial_for());
    }
    Ok(Authorized { authority, subject })
}

/// The **voices-unchanged boundary** over this module's refusal dialects.
///
/// `denial_for` is defaulted to `denial`, so the only way an arm's refusal can diverge from its
/// domain's is for someone to *override* `denial_for` on it — a behavioral change to a shipped gate,
/// not a refactor. This suite is what fails when that happens. It needs no database: `denial` and
/// `denial_for` take no pool, so the boundary runs in the cheapest, most-often-run tier rather than
/// only under `test-db`, where most contributors would never trip it.
///
/// **Every comparison checks the `ApiError` variant AND the rendered message.** `ApiError` has no
/// `PartialEq`, so the variant is compared through `std::mem::discriminant`; the message is compared
/// through `Display`. Discriminant alone would let a `NotFound` sentence change silently, and the
/// sentence is exactly what a refusal discloses.
///
/// # This suite is a conjunction of three claims, and it closes only two of them
///
/// * **(a) the nine voices are unchanged** — the nine tests below, one per `ScopedAuthority` impl,
///   each authority named explicitly rather than reached through an enumeration that could silently
///   cover eight. This is the conjunct the suite exists for, and it is the one that is probed: the
///   bite probe temporarily overrides `denial_for` on `TeamReadAuthority` — whose `denial` is
///   `NotFound` — to return `Forbidden`, which must red `team_read_authority_denies_in_one_voice`
///   and must green again once the override is removed. A suite that passes only because a
///   defaulted method cannot differ is not yet evidence of anything.
/// * **(b) a tenth authority cannot appear uncovered** — **NOT guarded here, deliberately.** A
///   `covered.len() == 9` assertion guards the *removal* direction; the failure mode is someone
///   *adding* a tenth impl and never touching this file, which no assertion inside a Rust test can
///   see (the language cannot enumerate trait impls at runtime, and `include_str!` takes no dynamic
///   path, so a source scan's file list decays somewhere less visible than what it replaced). A
///   count assertion here would only *look* like coverage. Declared an uncovered remainder in
///   `docs/superpowers/plans/2026-07-30-context-rename.md` Part 5; if the guard is later wanted it
///   belongs in `.github/scripts/audit-*.sh`, which pins a reviewed *set* and `rg`s the directory,
///   so a new `authz/*.rs` is caught for free.
/// * **(c) `ContextAdminAuthority` is the only divergent authority** — the tenth impl, and the
///   deliberate two-dialect exception, so it is asserted *positively* in the last two tests instead
///   of being asserted voice-unchanged. Its behavioral guarantee is Task 11's, not this suite's; see
///   the comment on those tests.
#[cfg(test)]
mod tests {
    use super::*;

    use crate::services::access_service::GrantAuthority;
    use crate::services::context_service::CONTEXT_REFUSAL;
    use crate::services::machine_authz::MachineAuthority;

    /// One denial arm refuses in exactly the dialect its domain declares.
    ///
    /// Takes the authority's name as a string because the failure message has to say *which* gate
    /// changed voice — a bare `assert_eq!` on two `ApiError`s names neither.
    #[track_caller]
    fn assert_one_voice<A: ScopedAuthority>(authority: &str, arm: A) {
        assert!(
            arm.is_denial(),
            "{authority}: `{arm:?}` is asserted here as a denial arm, but `is_denial` says it is \
             not — this suite is measuring an arm that never reaches a refusal at all"
        );

        let per_arm = arm.denial_for();
        let declared = A::denial();

        assert_eq!(
            std::mem::discriminant(&per_arm),
            std::mem::discriminant(&declared),
            "{authority}: `{arm:?}.denial_for()` renders {per_arm:?}, but `{authority}::denial()` \
             renders {declared:?}. This arm has been given its own refusal voice — a behavioral \
             change to a shipped gate that someone else's tests are downstream of."
        );
        assert_eq!(
            per_arm.to_string(),
            declared.to_string(),
            "{authority}: `{arm:?}.denial_for()` and `{authority}::denial()` are the same \
             `ApiError` variant but render different messages. The message IS the disclosure."
        );
    }

    // ── (a) the nine voices, each authority named explicitly ──────────────────

    #[test]
    fn grant_authority_denies_in_one_voice() {
        assert_one_voice("GrantAuthority", GrantAuthority::None);
    }

    #[test]
    fn connection_control_authority_denies_in_one_voice() {
        assert_one_voice(
            "ConnectionControlAuthority",
            ConnectionControlAuthority::None,
        );
    }

    #[test]
    fn connection_authority_denies_in_one_voice() {
        assert_one_voice("ConnectionAuthority", ConnectionAuthority::None);
    }

    #[test]
    fn machine_authority_denies_in_one_voice() {
        assert_one_voice("MachineAuthority", MachineAuthority::None);
    }

    #[test]
    fn two_sided_authority_denies_in_one_voice() {
        assert_one_voice("TwoSidedAuthority", TwoSidedAuthority::None);
    }

    /// Two denial arms, and both are asserted: `Author` is as much a refusal as `Unreadable`, and a
    /// `Forbidden` distinguishable from the unreadable case would leak the authorship relation.
    #[test]
    fn audit_authority_denies_in_one_voice() {
        assert_one_voice("AuditAuthority", AuditAuthority::Author);
        assert_one_voice("AuditAuthority", AuditAuthority::Unreadable);
    }

    /// Two denial arms again — "not a registered machine" and "no such readable cogmap" must be
    /// indistinguishable, or a guessed cogmap id becomes an existence oracle.
    #[test]
    fn auditor_job_authority_denies_in_one_voice() {
        assert_one_voice("AuditorJobAuthority", AuditorJobAuthority::NotMachine);
        assert_one_voice("AuditorJobAuthority", AuditorJobAuthority::Unreadable);
    }

    #[test]
    fn team_read_authority_denies_in_one_voice() {
        assert_one_voice("TeamReadAuthority", TeamReadAuthority::None);
    }

    #[test]
    fn actor_history_authority_denies_in_one_voice() {
        assert_one_voice("ActorHistoryAuthority", ActorHistoryAuthority::None);
    }

    // ── (c) the tenth authority: the deliberate two-dialect exception ─────────
    //
    // `ContextAdminAuthority` is NOT asserted voice-unchanged, because diverging is the point: one
    // gate, two dialects. The two tests below assert each dialect positively, which is a claim about
    // the MECHANISM only.
    //
    // **The behavioral guarantee is Task 11's, not this suite's.** Task 11's four route tests drive
    // the four ways a principal lands on `ReadOnly` — the team-tree read direction, context shares,
    // and explicit read-grants — through the live rename endpoint and assert `403` on every one. If
    // someone "simplifies" `ReadOnly` back to the static `NotFound`, the test below and all four of
    // those red together. Recorded here so a reader can see where (c)'s coverage lives rather than
    // having to infer that it exists.

    /// The `403` half: a caller who demonstrably reads the context is refused with `Forbidden`, not
    /// with the domain's static, existence-hiding `NotFound`. Not an oracle — the caller already
    /// read the subject, so the `403` discloses nothing a `GET` would not.
    #[test]
    fn context_admin_read_only_refuses_with_forbidden() {
        assert!(ContextAdminAuthority::ReadOnly.is_denial());

        let refusal = ContextAdminAuthority::ReadOnly.denial_for();
        assert!(
            matches!(refusal, ApiError::Forbidden),
            "ReadOnly must render Forbidden, got {refusal:?}"
        );
        assert_eq!(refusal.to_string(), ApiError::Forbidden.to_string());
    }

    /// The `404` half: byte-identical to the incumbent read refusal, because it reads the same
    /// `CONTEXT_REFUSAL` constant `resolve_context_ref` and `ensure_context_visible` render. The
    /// constant is read here rather than the sentence re-typed — a second literal would pass this
    /// test while drifting from the reads it is supposed to be indistinguishable from.
    #[test]
    fn context_admin_invisible_refuses_with_the_incumbent_context_refusal() {
        assert!(ContextAdminAuthority::Invisible.is_denial());

        match ContextAdminAuthority::Invisible.denial_for() {
            ApiError::NotFound(message) => assert_eq!(message, CONTEXT_REFUSAL),
            other => panic!("Invisible must render NotFound(CONTEXT_REFUSAL), got {other:?}"),
        }
    }
}
