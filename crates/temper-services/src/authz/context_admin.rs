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

    // Two suites in one module, and the second is the load-bearing one.
    //
    // **1. Arm resolution** — the mechanism's own unit hygiene (owner administers, both manage
    // roles administer, system admin admits without reading, stranger and missing row are
    // invisible).
    //
    // **2. The four-route equivalence class, exercised separately.** The register claims one
    // class — *read-without-administration* — but it is reached through **four genuinely different
    // mechanisms**: three are `UNION` arms of `contexts_readable_by` and one is `kb_access_grants`.
    // The claim holds structurally, since `caller_administers_context` consults neither, but a
    // class claimed across four mechanisms is exactly the shape that hides an unexamined cell, so
    // the build owes each route separately rather than one representative. Four fixtures, four
    // named tests, one shared assertion — a stranger pole beside them, because four `403`s prove
    // nothing about the split on their own: a gate that returned `403` to everyone would pass all
    // four.
    //
    // **These tests bite by construction, and the proof is already on the record — no separate
    // probe is owed.** A witness must fail against the state its clause claims to change, and
    // normally that has to be demonstrated. Here it already was: route 2 was written the wrong way
    // round in the spec, and a test built to that wording asserts `403` against an actor who
    // resolves `Invisible` and **fails**. That is this suite failing against a genuinely wrong
    // world, observed before the suite existed. Its absence here is deliberate, not an oversight.
    //
    // ⚠️ **Reads inherit DOWN the team tree, not up** (plan §G3, and re-confirmed live against the
    // dev database while writing these). `reachable_teams` is the caller's own teams expanded UP to
    // their ancestors, and arm 2 admits contexts owned by a team in that set — so a member of a
    // **descendant** team reads an **ancestor** team's context, and a maintainer of a parent team
    // does not reach a child team's context at all. Route 2's actor is therefore a maintainer of
    // the DESCENDANT team and its context is owned by the ANCESTOR. Built the other way round the
    // actor is `Invisible`, the test asserts `403` and gets `404`, and the obvious "fix" — widening
    // the gate so a parent-maintainer resolves `ReadOnly` — would be a **real authorization
    // widening shipped to turn a test green**.
    //
    // Fixture inserts use runtime `sqlx::query(...)`, not the compile-time macro, per project
    // convention for test-fixture writes — the idiom in `context_service.rs`'s own test module.
    // (Consequently nothing here restales a `.sqlx` cache.)

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

    // ── fixtures for the four routes ─────────────────────────────────────────────────────────

    /// `mk_personal_context` with a name distinct from the slug — needed only by the
    /// attributability test, whose whole point is asserting the `from_*` pair **by value**. With
    /// `name == slug` a payload that carried the slug twice would pass.
    async fn mk_named_personal_context(
        pool: &PgPool,
        slug: &str,
        name: &str,
        owner: ProfileId,
    ) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_contexts (slug, name, owner_table, owner_id) \
             VALUES ($1, $2, 'kb_profiles', $3) RETURNING id",
        )
        .bind(slug)
        .bind(name)
        .bind(*owner)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// `mk_profile`, plus the `<handle>@web` emitter entity `resolve_emitter` requires.
    ///
    /// Only the two tests that actually **rename** need this. Resolving an authority fires no
    /// event, so every arm-resolution test above stays on the bare `mk_profile`.
    async fn mk_actor(pool: &PgPool, handle: &str) -> ProfileId {
        let profile = mk_profile(pool, handle).await;
        sqlx::query("INSERT INTO kb_entities (name, profile_id) VALUES ($1, $2)")
            .bind(format!("{handle}@web"))
            .bind(*profile)
            .execute(pool)
            .await
            .unwrap();
        profile
    }

    /// `child` becomes a DESCENDANT of `parent` — the direction route 2 depends on.
    async fn link_child_team(pool: &PgPool, child: Uuid, parent: Uuid) {
        sqlx::query("INSERT INTO kb_teams_parents (child_id, parent_id) VALUES ($1, $2)")
            .bind(child)
            .bind(parent)
            .execute(pool)
            .await
            .unwrap();
    }

    /// `contexts_readable_by` arm 3's row.
    async fn share_context_to_team(pool: &PgPool, context: Uuid, team: Uuid) {
        sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
            .bind(context)
            .bind(team)
            .execute(pool)
            .await
            .unwrap();
    }

    /// `contexts_readable_by` arm 4's row. `granted_by_profile_id` is **NOT NULL** on
    /// `kb_access_grants` — omitting it is what made the first live probe of this route fail.
    async fn grant_context_read(
        pool: &PgPool,
        context: Uuid,
        principal: ProfileId,
        granted_by: ProfileId,
    ) {
        sqlx::query(
            "INSERT INTO kb_access_grants \
                 (subject_table, subject_id, principal_table, principal_id, can_read, \
                  granted_by_profile_id) \
             VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, $3)",
        )
        .bind(context)
        .bind(*principal)
        .bind(*granted_by)
        .execute(pool)
        .await
        .unwrap();
    }

    /// The pair every one of the four routes hangs off: a team that OWNS a context, and an
    /// `owner`-role member of that team as the control the routes are *not*.
    ///
    /// Returns `(owning_team, context)`.
    async fn owning_team_with_context(pool: &PgPool) -> (Uuid, Uuid) {
        let team_owner = mk_profile(pool, "owning-team-owner").await;
        let team = mk_team(pool, "owning").await;
        add_member(pool, team, team_owner, "owner").await;
        let context = mk_team_context(pool, "owned-notes", team).await;
        (team, context)
    }

    /// The shared assertion, and the whole content of the class: this caller **reads** the context
    /// (so refusing them with the existence-hiding `404` would lie about a row they can `GET`) and
    /// does **not** administer it — `ReadOnly`, rendered as `403`.
    ///
    /// Both halves are asserted, in that order. `ReadOnly` alone would pass if `denial_for` ever
    /// collapsed its two arms, and `Forbidden` alone would pass if the fixture accidentally seeded
    /// an administrator.
    async fn assert_reads_but_does_not_administer(
        pool: &PgPool,
        caller: ProfileId,
        context: Uuid,
        route: &str,
    ) {
        let arm = ContextAdminAuthority::resolve(pool, caller, context)
            .await
            .unwrap();
        assert_eq!(
            arm,
            ContextAdminAuthority::ReadOnly,
            "{route}: the caller reads this context but does not administer it"
        );
        assert!(
            matches!(arm.denial_for(), ApiError::Forbidden),
            "{route}: a reader must be refused with 403, never the existence-hiding 404 — \
             got {:?}",
            arm.denial_for()
        );
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

    // ── the four routes to read-without-administration, one test each ────────────────────────

    /// Route 1 — `contexts_readable_by` **arm 2**, direct membership at a role below `can_manage`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn arm2_lower_role_on_the_owning_team_is_read_only(pool: PgPool) {
        let (team, context) = owning_team_with_context(&pool).await;
        let member = mk_profile(&pool, "lower-role").await;
        // `member` is below `can_manage` (Owner|Maintainer), which is the entire difference
        // between this caller and the `owner` the fixture also seeds.
        add_member(&pool, team, member, "member").await;

        assert_reads_but_does_not_administer(
            &pool,
            member,
            context,
            "arm 2 — member on the owning team",
        )
        .await;
    }

    /// Route 2 — `contexts_readable_by` **arm 2 via `team_ancestors`**.
    ///
    /// ⚠️ The actor is a maintainer of the **descendant** team and the context is owned by the
    /// **ancestor**. See the module comment: the inverted construction resolves `Invisible`, and
    /// widening the gate to admit it would be a real authorization change.
    ///
    /// `maintainer` is deliberately the highest role `can_manage` accepts — the point is that the
    /// role is irrelevant here, because `caller_administers_context` asks `role_on_team(<owning
    /// team>, caller)` by DIRECT membership and this caller has none on the owning team at all.
    #[sqlx::test(migrations = "../../migrations")]
    async fn arm2_via_team_ancestors_descendant_maintainer_is_read_only(pool: PgPool) {
        let (owning_team, context) = owning_team_with_context(&pool).await;
        let descendant = mk_team(&pool, "descendant").await;
        link_child_team(&pool, descendant, owning_team).await;
        let maintainer = mk_profile(&pool, "descendant-maintainer").await;
        add_member(&pool, descendant, maintainer, "maintainer").await;

        assert_reads_but_does_not_administer(
            &pool,
            maintainer,
            context,
            "arm 2 via team_ancestors — maintainer of the descendant team",
        )
        .await;
    }

    /// Route 3 — `contexts_readable_by` **arm 3** (`kb_team_contexts`).
    ///
    /// The caller `owner`s the team the context is *shared to*, which is the most administrative
    /// role there is — on the wrong team. Sharing confers read-reach, never administration.
    #[sqlx::test(migrations = "../../migrations")]
    async fn arm3_kb_team_contexts_share_recipient_is_read_only(pool: PgPool) {
        let (_owning_team, context) = owning_team_with_context(&pool).await;
        let other_team = mk_team(&pool, "other").await;
        let other_owner = mk_profile(&pool, "other-team-owner").await;
        add_member(&pool, other_team, other_owner, "owner").await;
        share_context_to_team(&pool, context, other_team).await;

        assert_reads_but_does_not_administer(
            &pool,
            other_owner,
            context,
            "arm 3 — owner of a team the context is shared to",
        )
        .await;
    }

    /// Route 4 — **`kb_access_grants`**, the one route that is not a team predicate at all.
    #[sqlx::test(migrations = "../../migrations")]
    async fn arm4_kb_access_grants_read_grant_holder_is_read_only(pool: PgPool) {
        let (_owning_team, context) = owning_team_with_context(&pool).await;
        let granter = mk_profile(&pool, "granter").await;
        let grantee = mk_profile(&pool, "grantee").await;
        grant_context_read(&pool, context, grantee, granter).await;

        assert_reads_but_does_not_administer(
            &pool,
            grantee,
            context,
            "arm 4 — explicit profile-anchored read grant",
        )
        .await;
    }

    /// The negative pole for the four above.
    ///
    /// Without it they prove nothing about the *split*: a gate that answered `403` to everyone
    /// would pass all four. `a_stranger_is_invisible` asserts the arm; this asserts the
    /// **rendering**, which is the half the four routes are measured against — and that it is
    /// byte-identical to `CONTEXT_REFUSAL`, so an administration refusal is indistinguishable from
    /// `get_visible`'s and `resolve_context_ref`'s.
    #[sqlx::test(migrations = "../../migrations")]
    async fn the_negative_pole_a_stranger_renders_the_withholding_404(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let stranger = mk_profile(&pool, "stranger").await;
        let context = mk_personal_context(&pool, "private", owner).await;

        let arm = ContextAdminAuthority::resolve(&pool, stranger, context)
            .await
            .unwrap();
        assert!(
            matches!(arm.denial_for(), ApiError::NotFound(msg) if msg == CONTEXT_REFUSAL),
            "a non-reader must get the existence-hiding 404, not the reader's 403 — got {:?}",
            arm.denial_for()
        );
    }

    // ── system authority never becomes ownership ─────────────────────────────────────────────

    /// A system admin renames a context they cannot otherwise read — and acquires nothing by it.
    ///
    /// The clause is about **residue**, so the assertions are about what did *not* appear: no
    /// `kb_team_members` row, no `kb_access_grants` row, no change to `kb_contexts.owner_*`. The
    /// sharpest of them is the last one — after the rename the admin still cannot *read* the
    /// context, which is the same predicate that was false before they acted on it.
    #[sqlx::test(migrations = "../../migrations")]
    async fn system_admin_rename_leaves_no_ownership_residue(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let admin = mk_actor(&pool, "admin").await;
        crate::test_support::grant_governance(&pool, *admin).await;
        let context = mk_named_personal_context(&pool, "private", "Private", owner).await;

        assert!(
            !context_service::context_visible(&pool, *admin, context)
                .await
                .unwrap(),
            "fixture precondition: the admin must NOT be able to read this context"
        );

        // The clause is about RESIDUE, so the measurement has to be a DELTA, not an absolute.
        //
        // This assertion was `assert_eq!(memberships, 0)` and could never pass: `kb_profiles`
        // carries `trg_sync_personal_team`, which on INSERT creates a personal team with the new
        // profile as its `owner`. So `mk_actor` gives the admin one membership before this test
        // does anything, and the absolute-zero form was measuring the fixture rather than the act.
        //
        // The failure value said so: it reported 1, which is exactly the personal-team baseline —
        // had the rename granted anything there would have been 2. The clause was satisfied the
        // whole time; only its instrument was wrong.
        let memberships_before: i64 =
            sqlx::query_scalar("SELECT count(*) FROM kb_team_members WHERE profile_id = $1")
                .bind(*admin)
                .fetch_one(&pool)
                .await
                .unwrap();

        let outcome = context_service::rename(&pool, admin, context, "Renamed By Admin")
            .await
            .expect("a system admin may rename a context they cannot read");
        assert!(outcome.renamed, "the rename actually happened");
        assert_eq!(outcome.slug, "renamed-by-admin");

        let memberships_after: i64 =
            sqlx::query_scalar("SELECT count(*) FROM kb_team_members WHERE profile_id = $1")
                .bind(*admin)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            memberships_after, memberships_before,
            "renaming granted the admin no membership it did not already hold"
        );

        let grants: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kb_access_grants \
             WHERE subject_table = 'kb_contexts' AND subject_id = $1",
        )
        .bind(context)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(grants, 0, "renaming minted no access grant on the context");

        let ownership: (String, Uuid) =
            sqlx::query_as("SELECT owner_table, owner_id FROM kb_contexts WHERE id = $1")
                .bind(context)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            ownership,
            ("kb_profiles".to_string(), *owner),
            "the context's owner is exactly who it was before the rename"
        );

        assert!(
            !context_service::context_visible(&pool, *admin, context)
                .await
                .unwrap(),
            "the admin still cannot READ the context they just renamed — authority to act on a \
             subject never became standing to see it"
        );
    }

    // ── attributability ──────────────────────────────────────────────────────────────────────

    /// Every completed rename is attributable, **including what the context used to be called**.
    ///
    /// `kb_contexts` has no `updated` column and keeps no before-image, so the trail is the only
    /// thing on disk that can answer "what was this called?". The `from_*` pair is therefore
    /// asserted **by value**: a payload carrying only `to_name`/`to_slug` would satisfy every other
    /// assertion in this suite and fail this clause outright.
    #[sqlx::test(migrations = "../../migrations")]
    async fn a_completed_rename_is_attributable_with_its_before_image(pool: PgPool) {
        let owner = mk_actor(&pool, "owner").await;
        let context = mk_named_personal_context(&pool, "old-notes", "Old Notes", owner).await;

        let outcome = context_service::rename(&pool, owner, context, "New Notes")
            .await
            .expect("the owner administers their own context");
        assert!(outcome.renamed);

        // Every field is non-`Option` on purpose: a payload that OMITTED `from_name` would decode
        // as a NULL into a `String` and fail here, which is the same failure the value assertions
        // below are for. An `Option` would have swallowed it into a `None` nobody looks at.
        #[derive(sqlx::FromRow)]
        struct RenamedRow {
            from_name: String,
            from_slug: String,
            to_name: String,
            to_slug: String,
            anchor_table: String,
            anchor_id: Uuid,
            emitter_profile: Uuid,
        }

        // `fetch_all`, not `fetch_one`: sqlx's `fetch_one` returns the FIRST of several rows
        // without complaining, so the row count has to be asserted rather than assumed. One
        // rename emits exactly one event, and a second would mean the write path fired twice.
        let rows: Vec<RenamedRow> = sqlx::query_as(
            "SELECT e.payload->>'from_name' AS from_name, \
                    e.payload->>'from_slug' AS from_slug, \
                    e.payload->>'to_name'   AS to_name, \
                    e.payload->>'to_slug'   AS to_slug, \
                    e.producing_anchor_table AS anchor_table, \
                    e.producing_anchor_id    AS anchor_id, \
                    en.profile_id            AS emitter_profile \
               FROM kb_events e \
               JOIN kb_event_types t ON t.id = e.event_type_id \
               JOIN kb_entities en   ON en.id = e.emitter_entity_id \
              WHERE t.name = 'context_renamed' \
                AND e.producing_anchor_id = $1",
        )
        .bind(context)
        .fetch_all(&pool)
        .await
        .expect("query the context_renamed trail");
        assert_eq!(rows.len(), 1, "one rename, one event");
        let row = &rows[0];

        // The before-image, by value. This is the assertion the clause lives or dies on.
        assert_eq!(row.from_name, "Old Notes");
        assert_eq!(row.from_slug, "old-notes");
        assert_eq!(row.to_name, "New Notes");
        assert_eq!(row.to_slug, "new-notes");

        assert_eq!(row.anchor_table, "kb_contexts");
        assert_eq!(row.anchor_id, context);
        assert_eq!(
            row.emitter_profile, *owner,
            "the emitter resolves to the profile that acted"
        );
    }
}
