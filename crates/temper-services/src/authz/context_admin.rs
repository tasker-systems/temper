//! Who may administer a context — the gate a rename passes through.
//!
//! The closest sibling of [`super::read_gates`]: a gate over a subject whose *existence* is
//! withheld from principals who cannot see it, refusing with `NotFound` rather than `Forbidden` for
//! the reason that module's doc records. The difference is that this one answers in **two**
//! dialects — `403` to a principal who can read the context but does not administer it, `404` to
//! one who cannot see it at all — which is what [`ScopedAuthority::denial_for`] exists for. This is
//! its first and only consumer.
//!
//! **This is not an oracle.** The `403` goes only to principals who already read the context — they
//! learn nothing a `GET` would not already have told them. Refusal detail stays bounded by what the
//! caller already has standing to know. That is the argument a later "let's make the denials
//! consistent" pass has to answer before collapsing the two arms into one refusal.
//!
//! Every arm **calls** its incumbent predicate; none restates one, per `super`'s module doc
//! (`authz/mod.rs:12-16`). Design:
//! `docs/superpowers/specs/2026-07-30-context-rename-design.md` §"`ContextAdminAuthority`".

use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;

use super::ScopedAuthority;
use crate::error::{ApiError, ApiResult};
use crate::services::context_service::CONTEXT_REFUSAL;
use crate::services::{access_service, context_service};

/// Who may administer a context (rename it, and whatever else joins that class later).
///
/// Two admitting arms and two denying ones, and the denials are **not** interchangeable: they carry
/// different disclosure, so they render differently. See [`ContextAdminAuthority::denial_for`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ContextAdminAuthority {
    /// The profile owner of a personal context, or a caller who `can_manage` the owning team.
    Administers,
    /// A system admin. Admits; confers no ownership.
    SystemAdmin,
    /// Visible to the caller, but not administered by them → `403`.
    ReadOnly,
    /// Not visible to the caller at all → `404`.
    Invisible,
}

#[async_trait]
impl ScopedAuthority for ContextAdminAuthority {
    /// The bare context id, matching the spelling this seam already uses:
    /// `TwoSidedObject::Context(Uuid)` and `TeamReadAuthority::Subject = Uuid`.
    type Subject = Uuid;

    /// Probe in the spec's order. **Both orderings are load-bearing.**
    ///
    /// Admin sits at **2, not 1**, following the reasoning `TeamReadAuthority` already records
    /// (`read_gates.rs:45-46`): the common caller is the administrator, and probing
    /// `is_system_admin` first charges every one of them an extra round-trip.
    ///
    /// Admin must sit **above 3**, and this is load-bearing. `context_visible_to` →
    /// `context_readable_by_profile` → `contexts_readable_by`, and printed live from the dev
    /// database (plan §G2, a `\sf contexts_readable_by` dump) there is **no system-admin branch**
    /// in any of the four arms — personal context, context owned by an enclosing team, context
    /// shared to an enclosing team, explicit read-grant. A visibility-first ordering would render
    /// `404` to a system admin renaming a context they do not otherwise read — the exact actor the
    /// feature must admit.
    async fn resolve(pool: &PgPool, caller: ProfileId, context_id: Uuid) -> ApiResult<Self> {
        // 1. The object-side probe, unchanged and not re-derived: profile-owned ⇒ caller *is* the
        //    owner; team-owned ⇒ `can_manage` (Owner|Maintainer) by DIRECT membership. A missing
        //    context answers `false` here, which is why the visibility probe below — not this one —
        //    is what separates `ReadOnly` from `Invisible`.
        if context_service::caller_administers_context(pool, caller, context_id).await? {
            return Ok(ContextAdminAuthority::Administers);
        }
        // 2. System authority admits, and stays its own arm: it never becomes ownership.
        if access_service::is_system_admin(pool, caller).await? {
            return Ok(ContextAdminAuthority::SystemAdmin);
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
            ContextAdminAuthority::ReadOnly | ContextAdminAuthority::Invisible
        )
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
mod tests {
    use super::*;

    // Arm resolution only — the mechanism's own unit hygiene. The four routes that must land on
    // `ReadOnly` (team-tree direction, shares, read-grants) are the rename's own witness and live
    // with it, not here.
    //
    // Fixture inserts use runtime `sqlx::query(...)`, not the compile-time macro, per project
    // convention for test-fixture writes — the idiom in `context_service.rs`'s own test module.

    /// A bare profile. No `<handle>@web` emitter entity, unlike `context_service`'s `mk_profile_ent`:
    /// resolving an authority fires no event, so nothing here reaches `resolve_emitter`.
    async fn mk_profile(pool: &PgPool, handle: &str) -> ProfileId {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
        )
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap();
        ProfileId::from(id)
    }

    async fn mk_team(pool: &PgPool, slug: &str) -> Uuid {
        sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
            .bind(slug)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn add_member(pool: &PgPool, team: Uuid, p: ProfileId, role: &str) {
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role, source) \
             VALUES ($1, $2, $3::team_role, 'native'::team_member_source) \
             ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
        )
        .bind(team)
        .bind(*p)
        .bind(role)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn mk_personal_context(pool: &PgPool, slug: &str, owner: ProfileId) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_contexts (slug, name, owner_table, owner_id) \
             VALUES ($1, $1, 'kb_profiles', $2) RETURNING id",
        )
        .bind(slug)
        .bind(*owner)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn mk_team_context(pool: &PgPool, slug: &str, team: Uuid) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_contexts (slug, name, owner_table, owner_id) \
             VALUES ($1, $1, 'kb_teams', $2) RETURNING id",
        )
        .bind(slug)
        .bind(team)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn profile_owner_administers_their_own_context(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let ctx = mk_personal_context(&pool, "notes", owner).await;

        let arm = ContextAdminAuthority::resolve(&pool, owner, ctx)
            .await
            .unwrap();
        assert_eq!(arm, ContextAdminAuthority::Administers);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn team_owner_and_maintainer_both_administer_a_team_context(pool: PgPool) {
        // `can_manage` is Owner|Maintainer, so both roles resolve to the same arm.
        let team_owner = mk_profile(&pool, "team-owner").await;
        let maintainer = mk_profile(&pool, "maintainer").await;
        let team = mk_team(&pool, "eng").await;
        add_member(&pool, team, team_owner, "owner").await;
        add_member(&pool, team, maintainer, "maintainer").await;
        let ctx = mk_team_context(&pool, "eng-notes", team).await;

        assert_eq!(
            ContextAdminAuthority::resolve(&pool, team_owner, ctx)
                .await
                .unwrap(),
            ContextAdminAuthority::Administers
        );
        assert_eq!(
            ContextAdminAuthority::resolve(&pool, maintainer, ctx)
                .await
                .unwrap(),
            ContextAdminAuthority::Administers
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn system_admin_who_cannot_read_the_context_still_resolves_admin(pool: PgPool) {
        // The §G2 case: `contexts_readable_by` has no system-admin branch, so this caller is
        // invisible to the visibility predicate. Probing visibility first would 404 them.
        let owner = mk_profile(&pool, "owner").await;
        let admin = mk_profile(&pool, "admin").await;
        crate::test_support::grant_governance(&pool, *admin).await;
        let ctx = mk_personal_context(&pool, "private", owner).await;

        assert!(
            !context_service::context_visible(&pool, *admin, ctx)
                .await
                .unwrap(),
            "fixture precondition: the admin must NOT be able to read this context"
        );
        assert_eq!(
            ContextAdminAuthority::resolve(&pool, admin, ctx)
                .await
                .unwrap(),
            ContextAdminAuthority::SystemAdmin
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_stranger_is_invisible(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let stranger = mk_profile(&pool, "stranger").await;
        let ctx = mk_personal_context(&pool, "private", owner).await;

        assert_eq!(
            ContextAdminAuthority::resolve(&pool, stranger, ctx)
                .await
                .unwrap(),
            ContextAdminAuthority::Invisible
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn a_nonexistent_context_is_invisible(pool: PgPool) {
        // `caller_administers_context` answers `false` for a missing row, and so does
        // `context_visible_to` — an absent subject must not fall through to an admitting arm.
        let caller = mk_profile(&pool, "caller").await;

        assert_eq!(
            // `now_v7`, not `new_v4`: this crate's `uuid` carries the `v7` feature only, and every
            // id in the system is a v7 anyway.
            ContextAdminAuthority::resolve(&pool, caller, Uuid::now_v7())
                .await
                .unwrap(),
            ContextAdminAuthority::Invisible
        );
    }
}
