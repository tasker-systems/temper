# G3 Phase B2 — Team-Owner Registration + Reach Containment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Widen machine-client registration from `is_system_admin`-only to `is_system_admin OR is_team_owner(owning team)`, while containing the new machine's reach to a provable subset of the registering caller's own authority.

**Architecture:** Authorization moves out of the handler's `require_admin` and into the services (the pattern `team_service` and `access_service` already follow: auth-before-writes inside the service). A new `machine_authz` module resolves the caller's authority once and returns an `AuthorizedReach` value; `apply_reach` takes that value instead of raw specs, so reach cannot be applied without having been authorized — the check is required to *construct the argument*, not merely expected to precede the call.

**Tech Stack:** Rust, Axum, sqlx (compile-time-checked macros), PostgreSQL, cargo-nextest, reqwest (e2e).

**Spec:** [`docs/superpowers/specs/2026-07-11-machine-principal-phase-b2-team-owner-registration-design.md`](../specs/2026-07-11-machine-principal-phase-b2-team-owner-registration-design.md)

**Branch:** `jct/g3-phase-b2-team-owner-registration` (already created; the spec is committed on it)

## Global Constraints

- **No migration.** `team_id` landed in Phase A; every predicate needed already exists (spec D8). Do not write one.
- **Auth before writes.** Authorization resolves *before* the transaction opens. A rejected request must leave the database completely unchanged — no orphaned agent profile, no partial enrollment.
- **Fail closed on NULL.** `owner_team_id: Option<Uuid>` — `None` is admin-only. Never treat "no team to check" as "nothing to deny" (spec D2).
- **Reuse predicates, never restate them.** Teams check `team_service::can_manage`; grants check `access_service::profile_can_grant`. Calling them (not reimplementing their rules) is what keeps the machine surface tightening automatically whenever the human surface does (spec D4).
- **All new/changed queries use the `sqlx::query!` / `query_as!` / `query_scalar!` macros.** Never inline SQL in a handler.
- `#[expect(lint, reason = "...")]`, never `#[allow]`. All public types derive `Debug`.
- **Pre-commit hook forces `SQLX_OFFLINE=true`.** New macro queries have no cache entry mid-plan and will fail the hook with E0282. Commit SQL-touching tasks with `git commit --no-verify` (DB tests prove them live); Task 7 regenerates every cache and the final commit passes the full offline hook.

## File Structure

| File | Responsibility |
|---|---|
| `crates/temper-services/src/services/machine_authz.rs` | **New.** `MachineAuthority`, `authorize`, `AuthorizedReach`, `authorize_registration`. The only place that can construct an `AuthorizedReach`. |
| `crates/temper-services/src/services/mod.rs` | Register the new module. |
| `crates/temper-services/src/services/machine_registration_service.rs` | `apply_reach` takes `AuthorizedReach`; `provision`/`issue`/`rebind` authorize before their transaction. |
| `crates/temper-services/src/services/machine_client_service.rs` | `list` gains caller-scoped SQL; `get_for_caller`, `revoke`, `rotate_secret` gain authorization. |
| `crates/temper-services/src/services/team_service.rs` | `add_member` gains `change_role`'s owner-guard (D7). |
| `crates/temper-api/src/handlers/machine_clients.rs` | `require_admin` deleted; handlers become thin pass-throughs of `caller`. |
| `crates/temper-api/src/handlers/teams.rs` | `add_member`'s utoipa `responses(...)` documents the new 400. |
| `tests/e2e/tests/common/mod.rs` | New harness helpers: `make_system_admin`, `add_to_gating_team`, `grant_cogmap_grant`. |
| `tests/e2e/tests/machine_registration_authz_e2e.rs` | **New.** The B2 authorization + containment matrix, including the D4a escalation bite test. |

---

### Task 1: `add_member` owner-guard (spec D7)

`change_role` refuses to grant `owner` ("use ownership transfer"); `add_member` does not, and its `ON CONFLICT DO UPDATE SET role = EXCLUDED.role` will upgrade an existing member straight to `owner`. B2 measures containment against `add_member`'s bar, so the bar must not leak. This task stands alone and is the reviewer's smallest gate.

**Files:**
- Modify: `crates/temper-services/src/services/team_service.rs` (`add_member`, ~line 173)
- Modify: `crates/temper-api/src/handlers/teams.rs` (`add_member` utoipa `responses`, ~line 58)
- Test: `crates/temper-services/src/services/team_service.rs` (`mod lifecycle_tests`, ~line 488)

**Interfaces:**
- Consumes: `team_service::role_on_team`, `team_service::can_manage` (both existing, `pub(crate)`).
- Produces: `add_member` now returns `ApiError::BadRequest` when `req.role == TeamRole::Owner`. Task 2 relies on this being the human bar it mirrors.

- [ ] **Step 1: Write the failing tests**

Append to `mod lifecycle_tests` in `crates/temper-services/src/services/team_service.rs`:

```rust
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn add_member_refuses_to_grant_owner(pool: PgPool) {
    let owner = mk_profile(&pool, "b2-owner").await;
    let newcomer = mk_profile(&pool, "b2-newcomer").await;
    let team = mk_team(&pool, "b2-guard-team").await;

    sqlx::query("INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner')")
        .bind(team)
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();

    let err = add_member(
        &pool,
        ProfileId::from(owner),
        team,
        &AddMemberRequest { profile_id: newcomer, role: TeamRole::Owner },
    )
    .await
    .expect_err("add_member must refuse to grant owner");

    assert!(
        matches!(err, ApiError::BadRequest(_)),
        "granting owner via add_member is a 400, not {err:?}"
    );
}

/// The `ON CONFLICT DO UPDATE SET role` path is the sneaky one: it upgrades an
/// EXISTING member to owner, bypassing `change_role`'s guard entirely.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn add_member_cannot_upgrade_an_existing_member_to_owner(pool: PgPool) {
    let owner = mk_profile(&pool, "b2-owner2").await;
    let member = mk_profile(&pool, "b2-member2").await;
    let team = mk_team(&pool, "b2-guard-team2").await;

    sqlx::query("INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner')")
        .bind(team)
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'member')")
        .bind(team)
        .bind(member)
        .execute(&pool)
        .await
        .unwrap();

    let err = add_member(
        &pool,
        ProfileId::from(owner),
        team,
        &AddMemberRequest { profile_id: member, role: TeamRole::Owner },
    )
    .await
    .expect_err("re-adding an existing member as owner must be refused");
    assert!(matches!(err, ApiError::BadRequest(_)), "got {err:?}");

    let role: TeamRole = sqlx::query_scalar(
        r#"SELECT role FROM kb_team_members WHERE team_id = $1 AND profile_id = $2"#,
    )
    .bind(team)
    .bind(member)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(matches!(role, TeamRole::Member), "role must be untouched, got {role:?}");
}

/// The guard must not break the ordinary path.
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn add_member_still_adds_a_maintainer(pool: PgPool) {
    let owner = mk_profile(&pool, "b2-owner3").await;
    let newcomer = mk_profile(&pool, "b2-newcomer3").await;
    let team = mk_team(&pool, "b2-guard-team3").await;

    sqlx::query("INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner')")
        .bind(team)
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();

    let row = add_member(
        &pool,
        ProfileId::from(owner),
        team,
        &AddMemberRequest { profile_id: newcomer, role: TeamRole::Maintainer },
    )
    .await
    .expect("adding a maintainer still works");
    assert!(matches!(row.role, TeamRole::Maintainer));
}
```

If `AddMemberRequest` / `ApiError` are not already in `lifecycle_tests`'s scope via `use super::*;`, add `use crate::error::ApiError;` and `use temper_core::types::team::AddMemberRequest;` to the module's imports.

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo make docker-up
cargo nextest run -p temper-services --features test-db -E 'test(add_member)'
```

Expected: `add_member_refuses_to_grant_owner` and `add_member_cannot_upgrade_an_existing_member_to_owner` FAIL (the calls return `Ok`, so `expect_err` panics). `add_member_still_adds_a_maintainer` PASSES.

- [ ] **Step 3: Add the guard**

In `crates/temper-services/src/services/team_service.rs`, in `add_member`, immediately after the existing auth-before-writes `match` block and **before** the `INSERT`:

```rust
    // Same rule as `change_role`: `owner` is conferred by ownership transfer, never by a
    // role grant. Without this, `ON CONFLICT DO UPDATE SET role` makes `add_member` a
    // silent bypass of `change_role`'s guard — and B2 measures machine reach against
    // this bar, so a leak here is a leak there (spec D7/D4a).
    if matches!(req.role, TeamRole::Owner) {
        return Err(ApiError::BadRequest(
            "cannot grant owner via add_member; use ownership transfer".to_string(),
        ));
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo nextest run -p temper-services --features test-db -E 'test(add_member)'
```

Expected: all three PASS.

- [ ] **Step 5: Document the 400 in the OpenAPI contract**

Teams **are** in the OpenAPI contract (mounted via utoipa `routes!`), so the new status must be declared. In `crates/temper-api/src/handlers/teams.rs`, in `add_member`'s `#[utoipa::path(...)]` `responses(...)`, add below the 403 line:

```rust
        (status = 400, description = "Cannot grant owner via add_member; use ownership transfer"),
```

- [ ] **Step 6: Regenerate the OpenAPI spec and the Ruby gem**

The router is the source of truth for both `openapi.json` and the generated `temper-rb` gem; `cargo make check` gates both and will fail on drift. Gem regeneration needs Docker running.

```bash
cargo make openapi
git status --short   # expect openapi.json and clients/temper-rb/lib/temper/generated/** to be modified
```

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/temper-services/src/services/team_service.rs \
        crates/temper-api/src/handlers/teams.rs \
        openapi.json clients/temper-rb
git commit -m "fix(teams): add_member must not grant owner (Phase B2 D7)"
```

---

### Task 2: The `machine_authz` module — authority + `AuthorizedReach`

The heart of the phase. Resolves the caller's authority over a registration, and produces the typed `AuthorizedReach` that Task 3 makes `apply_reach` require.

**Files:**
- Create: `crates/temper-services/src/services/machine_authz.rs`
- Modify: `crates/temper-services/src/services/mod.rs`
- Test: `crates/temper-services/src/services/machine_authz.rs` (inline `#[cfg(all(test, feature = "test-db"))] mod tests`)

**Interfaces:**
- Consumes: `access_service::is_system_admin(pool, ProfileId) -> ApiResult<bool>`; `access_service::profile_can_grant(pool, ProfileId, &str, Uuid) -> ApiResult<bool>`; `team_service::role_on_team(pool, Uuid, ProfileId) -> ApiResult<Option<TeamRole>>`; `team_service::can_manage(TeamRole) -> bool`; `temper_core::types::machine::{TeamSpec, GrantSpec}`.
- Produces, for Tasks 3–5:
  - `pub(crate) enum MachineAuthority { SystemAdmin, TeamOwner }` (derives `Debug, Clone, Copy, PartialEq, Eq`)
  - `pub(crate) async fn authorize(pool: &PgPool, caller: ProfileId, team: Option<Uuid>) -> ApiResult<MachineAuthority>`
  - `pub(crate) struct AuthorizedReach<'a>` with `pub(crate) fn teams(&self) -> &'a [TeamSpec]` and `pub(crate) fn grants(&self) -> &'a [GrantSpec]` (fields **private to this module** — that privacy is the whole mechanism)
  - `pub(crate) async fn authorize_registration<'a>(pool: &PgPool, caller: ProfileId, team: Option<Uuid>, teams: &'a [TeamSpec], grants: &'a [GrantSpec]) -> ApiResult<AuthorizedReach<'a>>`

- [ ] **Step 1: Register the module**

In `crates/temper-services/src/services/mod.rs`, add in alphabetical position (after `invitation_service`):

```rust
pub(crate) mod machine_authz;
```

- [ ] **Step 2: Write the failing tests**

Create `crates/temper-services/src/services/machine_authz.rs` containing **only** this test module for now (the implementation lands in Step 4):

```rust
#[cfg(all(test, feature = "test-db"))]
mod tests {
    use super::*;
    use sqlx::PgPool;

    async fn mk_profile(pool: &PgPool, handle: &str) -> Uuid {
        sqlx::query_scalar("INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id")
            .bind(handle)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn mk_team(pool: &PgPool, slug: &str) -> Uuid {
        sqlx::query_scalar("INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id")
            .bind(slug)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    async fn join(pool: &PgPool, team: Uuid, profile: Uuid, role: &str) {
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role)
             VALUES ($1, $2, $3::text::team_role)
             ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
        )
        .bind(team)
        .bind(profile)
        .bind(role)
        .execute(pool)
        .await
        .unwrap();
    }

    /// A fresh DB seeds `access_mode='open'` with `gating_team_slug` NULL, so nobody is a
    /// system admin until a gating team is configured. Configure it the way the operator
    /// template does — WITHOUT flipping access_mode (prod runs 'open'; the admin check is
    /// load-bearing precisely because the router gate admits everyone there).
    async fn configure_gating_team(pool: &PgPool) -> Uuid {
        let team = mk_team(pool, "temper-system").await;
        sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'temper-system'")
            .execute(pool)
            .await
            .unwrap();
        team
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn team_owner_is_authorized_for_their_own_team(pool: PgPool) {
        let alice = mk_profile(&pool, "authz-alice").await;
        let team = mk_team(&pool, "authz-t").await;
        join(&pool, team, alice, "owner").await;

        let authority = authorize(&pool, ProfileId::from(alice), Some(team))
            .await
            .expect("a team owner may register for their own team");
        assert_eq!(authority, MachineAuthority::TeamOwner);
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn maintainer_and_member_are_not_authorized(pool: PgPool) {
        let team = mk_team(&pool, "authz-t2").await;
        for role in ["maintainer", "member", "watcher"] {
            let p = mk_profile(&pool, &format!("authz-{role}")).await;
            join(&pool, team, p, role).await;
            let err = authorize(&pool, ProfileId::from(p), Some(team))
                .await
                .expect_err("only an OWNER may register");
            assert!(matches!(err, ApiError::Forbidden), "{role} got {err:?}");
        }
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn non_member_is_not_authorized(pool: PgPool) {
        let stranger = mk_profile(&pool, "authz-stranger").await;
        let team = mk_team(&pool, "authz-t3").await;
        let err = authorize(&pool, ProfileId::from(stranger), Some(team))
            .await
            .expect_err("a non-member may not register");
        assert!(matches!(err, ApiError::Forbidden));
    }

    /// Spec D2 — the NULL owning team denies for non-admins. It must NOT fall open.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn none_team_is_admin_only(pool: PgPool) {
        let alice = mk_profile(&pool, "authz-alice2").await;
        let team = mk_team(&pool, "authz-t4").await;
        join(&pool, team, alice, "owner").await;

        let err = authorize(&pool, ProfileId::from(alice), None)
            .await
            .expect_err("a teamless registration is admin-only");
        assert!(matches!(err, ApiError::Forbidden), "NULL must deny, not fall open");

        let gating = configure_gating_team(&pool).await;
        let admin = mk_profile(&pool, "authz-admin").await;
        join(&pool, gating, admin, "owner").await;
        let authority = authorize(&pool, ProfileId::from(admin), None)
            .await
            .expect("an admin may register a teamless machine");
        assert_eq!(authority, MachineAuthority::SystemAdmin);
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn reach_into_a_managed_team_is_allowed(pool: PgPool) {
        let alice = mk_profile(&pool, "reach-alice").await;
        let owned = mk_team(&pool, "reach-owned").await;
        let managed = mk_team(&pool, "reach-managed").await;
        join(&pool, owned, alice, "owner").await;
        join(&pool, managed, alice, "maintainer").await;

        let teams = vec![TeamSpec { team_id: managed, role: "member".to_string() }];
        let reach = authorize_registration(&pool, ProfileId::from(alice), Some(owned), &teams, &[])
            .await
            .expect("can_manage on the target team permits reach into it");
        assert_eq!(reach.teams().len(), 1);
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn reach_into_an_unmanaged_team_is_denied(pool: PgPool) {
        let alice = mk_profile(&pool, "reach-alice2").await;
        let owned = mk_team(&pool, "reach-owned2").await;
        let foreign = mk_team(&pool, "reach-foreign").await;
        join(&pool, owned, alice, "owner").await;
        join(&pool, foreign, alice, "member").await; // member != can_manage

        let teams = vec![TeamSpec { team_id: foreign, role: "member".to_string() }];
        let err = authorize_registration(&pool, ProfileId::from(alice), Some(owned), &teams, &[])
            .await
            .expect_err("a mere member may not grant a machine reach into that team");
        assert!(matches!(err, ApiError::Forbidden));
    }

    /// Spec D4a — the escalation. A gating-team MAINTAINER clears `can_manage` on the
    /// gating team but is NOT a system admin. Without the role bar they could mint a
    /// machine at role=owner on the gating team — an `is_system_admin` principal.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn cannot_mint_owner_role_on_the_gating_team(pool: PgPool) {
        let gating = configure_gating_team(&pool).await;
        let alice = mk_profile(&pool, "escalate-alice").await;
        let owned = mk_team(&pool, "escalate-owned").await;
        join(&pool, owned, alice, "owner").await;
        join(&pool, gating, alice, "maintainer").await;

        assert!(
            !crate::services::access_service::is_system_admin(&pool, ProfileId::from(alice))
                .await
                .unwrap(),
            "precondition: a gating-team maintainer is NOT a system admin"
        );

        let teams = vec![TeamSpec { team_id: gating, role: "owner".to_string() }];
        let err = authorize_registration(&pool, ProfileId::from(alice), Some(owned), &teams, &[])
            .await
            .expect_err("minting a machine as gating-team OWNER is an escalation to system admin");
        assert!(matches!(err, ApiError::Forbidden), "got {err:?}");
    }

    /// The role bar is not gating-team-specific — `owner` is refused on any team.
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn cannot_mint_owner_role_on_any_team(pool: PgPool) {
        let alice = mk_profile(&pool, "escalate-alice2").await;
        let owned = mk_team(&pool, "escalate-owned2").await;
        join(&pool, owned, alice, "owner").await;

        let teams = vec![TeamSpec { team_id: owned, role: "owner".to_string() }];
        let err = authorize_registration(&pool, ProfileId::from(alice), Some(owned), &teams, &[])
            .await
            .expect_err("a non-admin may never mint a machine at role=owner");
        assert!(matches!(err, ApiError::Forbidden));
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn grant_without_can_grant_is_denied(pool: PgPool) {
        let alice = mk_profile(&pool, "grant-alice").await;
        let owned = mk_team(&pool, "grant-owned").await;
        join(&pool, owned, alice, "owner").await;

        // The L0 kernel cogmap — Alice certainly holds no `can_grant` on it.
        let l0: Uuid = "00000000-0000-0000-0005-000000000001".parse().unwrap();
        let grants = vec![GrantSpec { cogmap_id: l0, can_write: true }];

        let err = authorize_registration(&pool, ProfileId::from(alice), Some(owned), &[], &grants)
            .await
            .expect_err("cannot grant a machine write on a cogmap you cannot administer");
        assert!(matches!(err, ApiError::Forbidden));
    }

    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn grant_with_can_grant_is_allowed(pool: PgPool) {
        let alice = mk_profile(&pool, "grant-alice2").await;
        let owned = mk_team(&pool, "grant-owned2").await;
        join(&pool, owned, alice, "owner").await;

        let l0: Uuid = "00000000-0000-0000-0005-000000000001".parse().unwrap();
        sqlx::query(
            "INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id,
                                           can_read, can_write, can_grant, granted_by_profile_id)
             VALUES ('kb_cogmaps', $1, 'kb_profiles', $2, true, true, true, $2)
             ON CONFLICT (subject_table, subject_id, principal_table, principal_id)
             DO UPDATE SET can_grant = true",
        )
        .bind(l0)
        .bind(alice)
        .execute(&pool)
        .await
        .unwrap();

        let grants = vec![GrantSpec { cogmap_id: l0, can_write: true }];
        let reach = authorize_registration(&pool, ProfileId::from(alice), Some(owned), &[], &grants)
            .await
            .expect("a can_grant holder may delegate to a machine");
        assert_eq!(reach.grants().len(), 1);
    }

    /// Spec D5 — the admin bypass survives, unchecked (Phase A D5).
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn system_admin_reach_is_unchecked(pool: PgPool) {
        let gating = configure_gating_team(&pool).await;
        let admin = mk_profile(&pool, "admin-unchecked").await;
        join(&pool, gating, admin, "owner").await;

        let foreign = mk_team(&pool, "admin-foreign").await;
        let l0: Uuid = "00000000-0000-0000-0005-000000000001".parse().unwrap();

        let teams = vec![TeamSpec { team_id: foreign, role: "owner".to_string() }];
        let grants = vec![GrantSpec { cogmap_id: l0, can_write: true }];

        let reach = authorize_registration(&pool, ProfileId::from(admin), None, &teams, &grants)
            .await
            .expect("a system admin may grant any reach (Phase A D5)");
        assert_eq!(reach.teams().len(), 1);
        assert_eq!(reach.grants().len(), 1);
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

```bash
cargo nextest run -p temper-services --features test-db -E 'binary(machine_authz)' 2>&1 | tail -20
```

Expected: FAIL to compile — `authorize`, `authorize_registration`, `MachineAuthority`, `AuthorizedReach` are not defined.

- [ ] **Step 4: Write the implementation**

Prepend to `crates/temper-services/src/services/machine_authz.rs`, **above** the test module:

```rust
//! Authorization for machine-client registration (G3 Phase B2).
//!
//! Two things live here, and the separation is the point:
//!
//! 1. **Who may register** — [`authorize`]: a system admin, or the OWNER of the team that
//!    will own the machine. `is_system_admin` already *is* ownership of the gating team, so
//!    this is one concept keyed on two teams, not two concepts.
//! 2. **What reach they may confer** — [`AuthorizedReach`]: a value that only this module can
//!    construct. `apply_reach` takes it instead of raw specs, so reach cannot be applied
//!    without having been authorized. The invariant is enforced by the type, not by a comment
//!    — which is what the Phase A comment on `apply_reach` asked for and could not get.
//!
//! The containment bar is the *human* bar, reached by CALLING the human predicates rather
//! than restating them: teams need `can_manage` (what `add_member` requires) and a non-`Owner`
//! role (what `add_member` refuses, per D7); grants need `can_grant` (what `grant_capability`
//! requires of a non-admin). Tighten the human surface and the machine surface tightens with
//! it — there is no second copy of the policy to drift.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::types::ids::ProfileId;
use temper_core::types::machine::{GrantSpec, TeamSpec};
use temper_core::types::team::TeamRole;

use crate::error::{ApiError, ApiResult};
use crate::services::{access_service, team_service};

/// The caller's authority over a machine registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MachineAuthority {
    /// Owner of the gating team. Full, unchecked reach (Phase A D5).
    SystemAdmin,
    /// Owner of the team that owns (or will own) the machine. Reach is contained.
    TeamOwner,
}

/// Resolve who the caller is with respect to a registration owned by `team`.
///
/// **Fails closed on `None`** (spec D2): a teamless machine (`team_id IS NULL`) is admin-only
/// to create, read, or operate. "No team to check" must never mean "nothing to deny".
pub(crate) async fn authorize(
    pool: &PgPool,
    caller: ProfileId,
    team: Option<Uuid>,
) -> ApiResult<MachineAuthority> {
    if access_service::is_system_admin(pool, caller).await? {
        return Ok(MachineAuthority::SystemAdmin);
    }

    let Some(team_id) = team else {
        return Err(ApiError::Forbidden);
    };

    match team_service::role_on_team(pool, team_id, caller).await? {
        Some(TeamRole::Owner) => Ok(MachineAuthority::TeamOwner),
        _ => Err(ApiError::Forbidden),
    }
}

/// Reach that has been authorized against a caller's authority (spec D3).
///
/// The fields are private to this module and there is no public constructor, so an
/// `AuthorizedReach` can only come from [`authorize_registration`]. `apply_reach` takes this
/// type, which makes the unchecked path *unrepresentable* rather than merely discouraged.
#[derive(Debug)]
pub(crate) struct AuthorizedReach<'a> {
    teams: &'a [TeamSpec],
    grants: &'a [GrantSpec],
}

impl<'a> AuthorizedReach<'a> {
    pub(crate) fn teams(&self) -> &'a [TeamSpec] {
        self.teams
    }

    pub(crate) fn grants(&self) -> &'a [GrantSpec] {
        self.grants
    }
}

/// Authorize a registration and the reach it asks for, in that order.
///
/// A system admin gets the Phase A D5 bypass — named here, so the bypass is visible at this
/// call site instead of being implicit in the absence of a check.
pub(crate) async fn authorize_registration<'a>(
    pool: &PgPool,
    caller: ProfileId,
    team: Option<Uuid>,
    teams: &'a [TeamSpec],
    grants: &'a [GrantSpec],
) -> ApiResult<AuthorizedReach<'a>> {
    match authorize(pool, caller, team).await? {
        // Phase A D5: a system admin may confer any reach on a machine.
        MachineAuthority::SystemAdmin => Ok(AuthorizedReach { teams, grants }),
        MachineAuthority::TeamOwner => {
            contain_reach(pool, caller, teams, grants).await?;
            Ok(AuthorizedReach { teams, grants })
        }
    }
}

/// The non-admin containment bar. Every check calls an existing human-surface predicate.
async fn contain_reach(
    pool: &PgPool,
    caller: ProfileId,
    teams: &[TeamSpec],
    grants: &[GrantSpec],
) -> ApiResult<()> {
    for spec in teams {
        // D4a — the ROLE bar. `can_manage` admits maintainers, and a gating-team maintainer
        // is not a system admin; without this, they could mint a machine at role=owner on the
        // gating team and thereby manufacture an `is_system_admin` principal. `apply_reach`'s
        // raw `ON CONFLICT DO UPDATE SET role` never passes through `add_member`, so D7's
        // guard does not cover this write site. Any gate on granting a role must check the
        // role being granted, not merely the grantor's access to the team.
        if spec.role.eq_ignore_ascii_case("owner") {
            return Err(ApiError::Forbidden);
        }

        // D4 — the membership bar: exactly what `add_member` requires of a human.
        match team_service::role_on_team(pool, spec.team_id, caller).await? {
            Some(role) if team_service::can_manage(role) => {}
            _ => return Err(ApiError::Forbidden),
        }
    }

    for grant in grants {
        // D4 — exactly what `grant_capability` requires of a non-admin: `can_grant` on the
        // subject. (`can_administer_grant` is this OR `is_system_admin`; the admin case has
        // already returned above.)
        if !access_service::profile_can_grant(pool, caller, "kb_cogmaps", grant.cogmap_id).await? {
            return Err(ApiError::Forbidden);
        }
    }

    Ok(())
}
```

If `access_service::profile_can_grant` or `team_service::{role_on_team, can_manage}` are not visible from this module, widen them from `pub(crate)` — they already are `pub(crate)`, so no change should be needed. Do **not** make them `pub`.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo nextest run -p temper-services --features test-db -E 'binary(machine_authz)'
```

Expected: all 10 tests PASS. (`-E 'binary(...)'` selects the file; `test(...)` matches test *names* — see the nextest filter gotcha.)

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/temper-services/src/services/machine_authz.rs crates/temper-services/src/services/mod.rs
git commit --no-verify -m "feat(machine): AuthorizedReach + team-owner registration authority (B2 D1-D4a)"
```

`--no-verify` because these new macro queries have no `.sqlx` cache entry yet and the pre-commit hook runs offline. Task 7 regenerates the caches; the final commit passes the full hook.

---

### Task 3: `apply_reach` requires `AuthorizedReach`; `provision`/`issue` authorize

Makes the bypass unrepresentable at the write site, and moves authorization off the handler.

**Files:**
- Modify: `crates/temper-services/src/services/machine_registration_service.rs` (`apply_reach` ~line 57, `provision` ~line 143, `issue` ~line 201)

**Interfaces:**
- Consumes: `machine_authz::{authorize_registration, AuthorizedReach}` from Task 2.
- Produces: `apply_reach(conn, caller, profile_id, reach: AuthorizedReach<'_>)`. `provision` and `issue` keep their existing public signatures — the handler still calls `provision(&state.pool, caller, &body)` — but now authorize internally and return `ApiError::Forbidden` themselves.

- [ ] **Step 1: Change `apply_reach` to take an `AuthorizedReach`**

Replace the signature and the two loop headers in `apply_reach`:

```rust
/// Apply a machine's reach. Takes an [`AuthorizedReach`] — which only `machine_authz` can
/// construct — so reach can never be applied without having been authorized against the
/// caller's own authority (spec D3).
///
/// The raw `insert_grant` / raw team INSERT below remain deliberately unchecked: for a system
/// admin that is Phase A's D5 bypass, and for a team owner `machine_authz::contain_reach` has
/// already proven the reach is a subset of what the caller could confer on a human. The
/// authorization is in the TYPE now, not in a comment asking you not to widen this.
async fn apply_reach(
    conn: &mut sqlx::PgConnection,
    caller: ProfileId,
    profile_id: Uuid,
    reach: AuthorizedReach<'_>,
) -> ApiResult<()> {
    for team in reach.teams() {
        // ... existing INSERT INTO kb_team_members, unchanged ...
    }

    for grant in reach.grants() {
        // ... existing insert_grant call, unchanged ...
    }

    Ok(())
}
```

Delete the old `Do not "tighten" this to grant_capability without revisiting D5` comment block — the type now says it, and the reason it existed (an invariant with no enforcement) is gone.

Add to the file's imports:

```rust
use crate::services::machine_authz::{self, AuthorizedReach};
```

- [ ] **Step 2: Authorize in `provision`, before the transaction**

In `provision`, insert as the **first** statement of the function body (before `pool.begin()`):

```rust
    // Auth before writes: a rejected registration must leave the DB completely unchanged —
    // no orphaned agent profile, no partial enrollment. Resolving before the transaction is
    // what makes that assertable.
    let reach = machine_authz::authorize_registration(
        pool,
        caller,
        req.owner_team_id,
        &req.teams,
        &req.grants,
    )
    .await?;
```

Then change the existing call site from `apply_reach(&mut tx, caller, profile_id, &req.teams, &req.grants).await?;` to:

```rust
    apply_reach(&mut tx, caller, profile_id, reach).await?;
```

- [ ] **Step 3: Authorize in `issue`, before the transaction**

Identical treatment in `issue`. Insert before `let client_id = ...` / `pool.begin()`:

```rust
    let reach = machine_authz::authorize_registration(
        pool,
        caller,
        req.owner_team_id,
        &req.teams,
        &req.grants,
    )
    .await?;
```

and change its `apply_reach(...)` call to `apply_reach(&mut tx, caller, profile_id, reach).await?;`.

- [ ] **Step 4: Verify it compiles and existing tests still pass**

```bash
cargo clippy -p temper-services --all-features -- -D warnings
cargo nextest run -p temper-services --features test-db -E 'binary(machine_authz)'
```

Expected: clean compile; Task 2's tests still pass. A compile error of the form "expected `AuthorizedReach`, found `&[TeamSpec]`" anywhere else in the crate means a caller is trying to apply unauthorized reach — that is the type doing its job; route that caller through `authorize_registration`.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/temper-services/src/services/machine_registration_service.rs
git commit --no-verify -m "feat(machine): apply_reach requires AuthorizedReach; provision/issue authorize (B2 D3)"
```

---

### Task 4: Row-addressed lifecycle + scoped `list` + thin handlers

Keyed on the **existing row's** `team_id` (spec D5). `rebind` copies `old.team_id` onto the new row, so gating it on the old row is coherent — a rebind never moves a machine between teams.

The scoped `list` lands **here**, not in its own task: `list`'s service signature and its handler call site must change in the same commit or `temper-api` will not compile. Splitting them would strand the tree in a red state that tells a reviewer nothing.

**Files:**
- Modify: `crates/temper-services/src/services/machine_registration_service.rs` (`rebind` ~line 255)
- Modify: `crates/temper-services/src/services/machine_client_service.rs` (`get`, `list`, `revoke`, `rotate_secret`)
- Modify: `crates/temper-api/src/handlers/machine_clients.rs`
- Test: `crates/temper-services/src/services/machine_authz.rs` (`mod tests`)

**Interfaces:**
- Consumes: `machine_authz::authorize` from Task 2.
- Produces:
  - `machine_client_service::get_for_caller(pool: &PgPool, caller: ProfileId, id: Uuid) -> ApiResult<MachineClient>` (**new**; `get` stays as the unauthorized internal primitive — the auth path and `issue`/`rebind` call it after insert, and must not be gated)
  - `machine_client_service::list(pool: &PgPool, caller: ProfileId, include_revoked: bool) -> ApiResult<Vec<MachineClient>>` — **caller added**
  - `machine_client_service::revoke(pool, id, revoker)` — unchanged signature, now authorizes internally
  - `machine_client_service::rotate_secret(pool, caller: ProfileId, id, grace_seconds)` — **caller added**
  - `machine_registration_service::rebind(pool, caller, req)` — unchanged signature, now authorizes internally

- [ ] **Step 1: Add `get_for_caller` and authorize `revoke` / `rotate_secret`**

In `crates/temper-services/src/services/machine_client_service.rs`, add the import:

```rust
use crate::services::machine_authz;
```

Add, below the existing `get`:

```rust
/// `get`, authorized for a surface caller (spec D5). `get` itself stays unauthorized: it is the
/// internal primitive the auth path and post-insert readbacks use, and gating it would break them.
pub async fn get_for_caller(pool: &PgPool, caller: ProfileId, id: Uuid) -> ApiResult<MachineClient> {
    let client = get(pool, id).await?;
    machine_authz::authorize(pool, caller, client.team_id).await?;
    Ok(client)
}
```

In `revoke`, insert **before** the `UPDATE` (auth before writes):

```rust
    let existing = get(pool, id).await?;
    machine_authz::authorize(pool, revoker, existing.team_id).await?;
```

In `rotate_secret`, add `caller: ProfileId` as the second parameter and insert **before** the existing grace-window validation and the transaction:

```rust
    let existing = get(pool, id).await?;
    machine_authz::authorize(pool, caller, existing.team_id).await?;
```

- [ ] **Step 2: Replace `list` with the caller-scoped query**

In the same file, replace `list` entirely:

```rust
/// List machine clients visible to `caller` (spec D5). A system admin sees every row,
/// including teamless ones; a team owner sees only machines owned by a team they own.
///
/// `EXISTS`, not `array_agg` — an empty scope must DENY, and an aggregate over an empty scope
/// yields NULL, which falls open.
pub async fn list(
    pool: &PgPool,
    caller: ProfileId,
    include_revoked: bool,
) -> ApiResult<Vec<MachineClient>> {
    let is_admin = crate::services::access_service::is_system_admin(pool, caller).await?;

    let rows = sqlx::query_as!(
        MachineClient,
        r#"SELECT id, client_id, issuer, label, profile_id, team_id,
                  registered_by_profile_id, created, last_seen_at,
                  revoked_at, revoked_by_profile_id
             FROM kb_machine_clients mc
            WHERE ($1 OR mc.revoked_at IS NULL)
              AND ( $2
                    OR ( mc.team_id IS NOT NULL
                         AND EXISTS (
                             SELECT 1
                               FROM kb_team_members tm
                              WHERE tm.team_id = mc.team_id
                                AND tm.profile_id = $3
                                AND tm.role = 'owner'
                         ) ) )
            ORDER BY created DESC"#,
        include_revoked,
        is_admin,
        *caller,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
```

- [ ] **Step 3: Authorize `rebind`**

In `crates/temper-services/src/services/machine_registration_service.rs`, `rebind` already loads `let old = machine_client_service::get(pool, req.from_machine_client_id).await?;` on its first line. Immediately after it, and **before** `pool.begin()`:

```rust
    // Keyed on the row being rebound. `rebind` copies `old.team_id` onto the new row, so the
    // machine never changes hands — authorizing against the old row is authorizing against the
    // new one.
    machine_authz::authorize(pool, caller, old.team_id).await?;
```

- [ ] **Step 4: Delete `require_admin` and thin the handlers**

In `crates/temper-api/src/handlers/machine_clients.rs`:

Replace the module doc comment with:

```rust
//! Machine-client registration (G3 Phase A/B1/B2). Out of the OpenAPI contract (plain
//! `.route()` mounting), like `/api/access/admin/*`.
//!
//! **Authorization lives in the services, not here** (Phase B2) — the same shape
//! `team_service` and `access_service` already use. It is `is_system_admin OR owner of the
//! machine's owning team`, and it is load-bearing rather than defense-in-depth: production
//! runs `access_mode = 'open'`, under which `has_system_access` is true for every profile, so
//! `require_system_access` on the gated router admits everyone. The service-side check is the
//! only thing protecting these endpoints (Phase A D12).
```

Delete the `require_admin` function entirely, and the now-unused `use temper_services::services::access_service;` and `use temper_core::types::AuthenticatedProfile;` imports if nothing else in the file needs them.

Rewrite each handler to pass `caller` straight through:

```rust
pub async fn provision(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<ProvisionMachineRequest>,
) -> ApiResult<Json<MachineClient>> {
    let caller = ProfileId::from(auth.0.profile.id);
    let client = machine_registration_service::provision(&state.pool, caller, &body).await?;
    Ok(Json(client))
}

pub async fn rebind(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(mut body): Json<RebindMachineRequest>,
) -> ApiResult<Json<MachineClient>> {
    let caller = ProfileId::from(auth.0.profile.id);
    // The path segment is authoritative for which client is being rotated away from.
    body.from_machine_client_id = id;
    let client = machine_registration_service::rebind(&state.pool, caller, &body).await?;
    Ok(Json(client))
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<ListQuery>,
) -> ApiResult<Json<Vec<MachineClient>>> {
    let caller = ProfileId::from(auth.0.profile.id);
    Ok(Json(
        machine_client_service::list(&state.pool, caller, q.include_revoked).await?,
    ))
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<MachineClient>> {
    let caller = ProfileId::from(auth.0.profile.id);
    Ok(Json(
        machine_client_service::get_for_caller(&state.pool, caller, id).await?,
    ))
}

pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<MachineClient>> {
    let caller = ProfileId::from(auth.0.profile.id);
    Ok(Json(
        machine_client_service::revoke(&state.pool, id, caller).await?,
    ))
}

pub async fn issue(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<IssueMachineRequest>,
) -> ApiResult<Json<IssuedMachineCredential>> {
    let caller = ProfileId::from(auth.0.profile.id);
    let cred = machine_registration_service::issue(&state.pool, caller, &body).await?;
    Ok(Json(cred))
}

pub async fn rotate_secret(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Json(body): Json<RotateSecretRequest>,
) -> ApiResult<Json<IssuedMachineCredential>> {
    let caller = ProfileId::from(auth.0.profile.id);
    let cred =
        machine_client_service::rotate_secret(&state.pool, caller, id, body.grace_seconds).await?;
    Ok(Json(cred))
}
```

- [ ] **Step 5: Write the `list`-scoping test**

Append to `machine_authz.rs`'s `mod tests` (it already has the profile/team/join/gating helpers):

```rust
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn list_is_scoped_to_owned_teams(pool: PgPool) {
        use crate::services::machine_client_service;

        let gating = configure_gating_team(&pool).await;
        let admin = mk_profile(&pool, "list-admin").await;
        join(&pool, gating, admin, "owner").await;

        let alice = mk_profile(&pool, "list-alice").await;
        let alice_team = mk_team(&pool, "list-alice-team").await;
        join(&pool, alice_team, alice, "owner").await;

        let other_team = mk_team(&pool, "list-other-team").await;

        // Three rows: one owned by Alice's team, one by another team, one teamless.
        for (client_id, team) in [
            ("list-mine", Some(alice_team)),
            ("list-theirs", Some(other_team)),
            ("list-teamless", None),
        ] {
            let agent = mk_profile(&pool, client_id).await;
            sqlx::query(
                "INSERT INTO kb_machine_clients
                     (client_id, issuer, label, profile_id, team_id, registered_by_profile_id)
                 VALUES ($1, 'temper', $1, $2, $3, $4)",
            )
            .bind(client_id)
            .bind(agent)
            .bind(team)
            .bind(admin)
            .execute(&pool)
            .await
            .unwrap();
        }

        let mine = machine_client_service::list(&pool, ProfileId::from(alice), false)
            .await
            .unwrap();
        let ids: Vec<&str> = mine.iter().map(|c| c.client_id.as_str()).collect();
        assert_eq!(ids, ["list-mine"], "a team owner sees only their team's machines");

        let all = machine_client_service::list(&pool, ProfileId::from(admin), false)
            .await
            .unwrap();
        assert_eq!(all.len(), 3, "an admin sees every row, including teamless");
    }
```

- [ ] **Step 6: Verify — the whole workspace compiles and every service test passes**

```bash
cargo clippy --workspace --all-features -- -D warnings
cargo nextest run -p temper-services --features test-db -E 'binary(machine_authz)'
```

Expected: clean clippy across the workspace (service signatures and handler call sites moved together, so there is no red state), and all `machine_authz` tests pass including `list_is_scoped_to_owned_teams`.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/temper-services/src/services/machine_client_service.rs \
        crates/temper-services/src/services/machine_registration_service.rs \
        crates/temper-services/src/services/machine_authz.rs \
        crates/temper-api/src/handlers/machine_clients.rs
git commit --no-verify -m "feat(machine): authorize lifecycle + scope list by owning team (B2 D5)"
```

---

### Task 5: The e2e matrix

Access-semantics changes are exactly where a green `test-db` run is a false signal, so the real gate is e2e — over HTTP, through the real router, with real JWTs.

**Files:**
- Modify: `tests/e2e/tests/common/mod.rs`
- Create: `tests/e2e/tests/machine_registration_authz_e2e.rs`

**Interfaces:**
- Consumes: `common::{setup, generate_test_jwt, E2eTestApp}` (existing).
- Produces (new `common` helpers, used by this test file):
  - `pub async fn make_system_admin(pool: &PgPool, profile_id: Uuid)`
  - `pub async fn add_to_gating_team(pool: &PgPool, profile_id: Uuid, role: &str)`

- [ ] **Step 1: Add the harness helpers**

A fresh test DB seeds `kb_system_settings(access_mode='open')` with `gating_team_slug` **NULL** (`system_initialization.sql` is an operator template, not a migration), so `is_system_admin` is false for everybody until a gating team is configured. This is what blocked B1's happy-path e2e. Add to `tests/e2e/tests/common/mod.rs`:

```rust
/// Configure the gating team and make `profile_id` its OWNER — i.e. a system admin.
///
/// Deliberately does NOT flip `access_mode`: production runs `'open'`, and the machine-client
/// authorization check is load-bearing precisely because the router's `require_system_access`
/// gate admits everyone under `'open'`. Testing under `'open'` is testing what prod does.
/// (Contrast `enable_invite_only`, which also flips the mode.)
pub async fn make_system_admin(pool: &PgPool, profile_id: uuid::Uuid) {
    add_to_gating_team(pool, profile_id, "owner").await;
}

/// Ensure the `temper-system` gating team exists, is configured as the gating team, and holds
/// `profile_id` at `role`. Roles other than `owner` do NOT confer system-adminhood —
/// `is_system_admin` requires `owner` — which is what the D4a escalation test turns on.
pub async fn add_to_gating_team(pool: &PgPool, profile_id: uuid::Uuid, role: &str) {
    let team_id: uuid::Uuid = sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name)
         VALUES ('temper-system', 'Temper System')
         ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("ensure temper-system gating team");

    sqlx::query("UPDATE kb_system_settings SET gating_team_slug = 'temper-system', updated = now()")
        .execute(pool)
        .await
        .expect("configure gating team slug");

    sqlx::query(
        "INSERT INTO kb_team_members (team_id, profile_id, role)
         VALUES ($1, $2, $3::text::team_role)
         ON CONFLICT (team_id, profile_id) DO UPDATE SET role = EXCLUDED.role",
    )
    .bind(team_id)
    .bind(profile_id)
    .bind(role)
    .execute(pool)
    .await
    .expect("add profile to gating team");
}
```

- [ ] **Step 2: Write the e2e matrix**

Create `tests/e2e/tests/machine_registration_authz_e2e.rs`:

```rust
//! G3 Phase B2 — machine-client registration authorization and reach containment, over HTTP.
//!
//! `test-db` green is a false signal for access semantics: these assertions have to run through
//! the real router, the real auth middleware, and real JWTs. The bite test here
//! (`gating_team_maintainer_cannot_mint_a_system_admin`) is the one that matters — it asserts a
//! privilege-escalation path is closed, not merely that a predicate returns false.

mod common;

use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// Provision a profile by hitting an authed endpoint (auto-provision on first request).
async fn provision_profile(app: &common::E2eTestApp, token: &str) -> Uuid {
    let resp = reqwest::Client::new()
        .get(app.url("/api/profile"))
        .bearer_auth(token)
        .send()
        .await
        .expect("GET /api/profile");
    assert_eq!(resp.status(), 200, "profile auto-provision");
    let body: serde_json::Value = resp.json().await.expect("profile json");
    body["id"].as_str().expect("profile id").parse().expect("uuid")
}

/// Create a team via the API; the caller becomes its sole owner.
async fn create_team(app: &common::E2eTestApp, token: &str, slug: &str) -> Uuid {
    let resp = reqwest::Client::new()
        .post(app.url("/api/teams"))
        .bearer_auth(token)
        .json(&json!({ "slug": slug, "name": slug }))
        .send()
        .await
        .expect("POST /api/teams");
    assert_eq!(resp.status(), 201, "team create: {:?}", resp.text().await);
    let body: serde_json::Value = resp.json().await.expect("team json");
    body["id"].as_str().expect("team id").parse().expect("uuid")
}

/// `POST /api/machine-clients/issue`. Returns (status, body).
async fn issue(
    app: &common::E2eTestApp,
    token: &str,
    body: serde_json::Value,
) -> (reqwest::StatusCode, serde_json::Value) {
    let resp = reqwest::Client::new()
        .post(app.url("/api/machine-clients/issue"))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .expect("POST /issue");
    let status = resp.status();
    let json = resp.json().await.unwrap_or(serde_json::Value::Null);
    (status, json)
}

async fn machine_count(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM kb_machine_clients")
        .fetch_one(pool)
        .await
        .expect("count machines")
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn team_owner_can_issue_for_their_own_team(pool: PgPool) {
    let app = common::setup(pool.clone()).await;
    let token = common::generate_test_jwt("b2-owner", "b2-owner@example.com");
    provision_profile(&app, &token).await;
    let team = create_team(&app, &token, "b2-owner-team").await;

    let (status, body) = issue(
        &app,
        &token,
        json!({ "label": "team agent", "owner_team_id": team, "teams": [], "grants": [] }),
    )
    .await;

    assert_eq!(status, 200, "a team owner may issue for their own team: {body:?}");
    assert!(
        body["client_secret"].as_str().is_some_and(|s| !s.is_empty()),
        "the plaintext secret is returned once"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_owner_cannot_issue_for_a_team(pool: PgPool) {
    let app = common::setup(pool.clone()).await;

    let owner_token = common::generate_test_jwt("b2-o", "b2-o@example.com");
    let owner_id = provision_profile(&app, &owner_token).await;
    let team = create_team(&app, &owner_token, "b2-nonowner-team").await;

    // A maintainer of the very same team is still not an owner.
    let maint_token = common::generate_test_jwt("b2-m", "b2-m@example.com");
    let maint_id = provision_profile(&app, &maint_token).await;
    let resp = reqwest::Client::new()
        .post(app.url(&format!("/api/teams/{team}/members")))
        .bearer_auth(&owner_token)
        .json(&json!({ "profile_id": maint_id, "role": "maintainer" }))
        .send()
        .await
        .expect("add maintainer");
    assert_eq!(resp.status(), 201);
    let _ = owner_id;

    let (status, _) = issue(
        &app,
        &maint_token,
        json!({ "label": "nope", "owner_team_id": team, "teams": [], "grants": [] }),
    )
    .await;
    assert_eq!(status, 403, "a maintainer is not an owner; registration needs OWNER");

    // And a total stranger.
    let stranger = common::generate_test_jwt("b2-s", "b2-s@example.com");
    provision_profile(&app, &stranger).await;
    let (status, _) = issue(
        &app,
        &stranger,
        json!({ "label": "nope", "owner_team_id": team, "teams": [], "grants": [] }),
    )
    .await;
    assert_eq!(status, 403, "a non-member cannot register for someone else's team");
}

/// Spec D2 — a teamless registration is admin-only. NULL must deny, not fall open.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_admin_cannot_issue_a_teamless_machine(pool: PgPool) {
    let app = common::setup(pool.clone()).await;
    let token = common::generate_test_jwt("b2-null", "b2-null@example.com");
    provision_profile(&app, &token).await;
    create_team(&app, &token, "b2-null-team").await; // owns a team, but doesn't name it

    let (status, _) = issue(
        &app,
        &token,
        json!({ "label": "teamless", "owner_team_id": null, "teams": [], "grants": [] }),
    )
    .await;
    assert_eq!(status, 403, "owner_team_id: null is admin-only (D2)");
}

/// Spec D4 + auth-before-writes: reach into an unmanaged team is refused AND writes nothing.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reach_into_an_unmanaged_team_is_denied_and_writes_nothing(pool: PgPool) {
    let app = common::setup(pool.clone()).await;

    let alice = common::generate_test_jwt("b2-alice", "b2-alice@example.com");
    provision_profile(&app, &alice).await;
    let alice_team = create_team(&app, &alice, "b2-alice-team").await;

    let bob = common::generate_test_jwt("b2-bob", "b2-bob@example.com");
    provision_profile(&app, &bob).await;
    let bob_team = create_team(&app, &bob, "b2-bob-team").await;

    let before = machine_count(&pool).await;

    let (status, _) = issue(
        &app,
        &alice,
        json!({
            "label": "reaching too far",
            "owner_team_id": alice_team,
            "teams": [{ "team_id": bob_team, "role": "member" }],
            "grants": []
        }),
    )
    .await;

    assert_eq!(status, 403, "Alice cannot walk a machine into Bob's team");
    assert_eq!(
        machine_count(&pool).await,
        before,
        "auth before writes: a rejected registration leaves NO row behind"
    );
}

/// **The bite test (spec D4a).** A gating-team MAINTAINER is not a system admin, but clears
/// `can_manage` on the gating team. Without the role bar they could mint a machine at
/// `role = owner` on the gating team — and that machine WOULD be `is_system_admin`.
/// This asserts the escalation is closed, not merely that a predicate said no.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn gating_team_maintainer_cannot_mint_a_system_admin(pool: PgPool) {
    let app = common::setup(pool.clone()).await;

    let alice = common::generate_test_jwt("b2-esc", "b2-esc@example.com");
    let alice_id = provision_profile(&app, &alice).await;
    let alice_team = create_team(&app, &alice, "b2-esc-team").await;

    // Alice is a MAINTAINER of the gating team — emphatically not an owner, so not an admin.
    common::add_to_gating_team(&pool, alice_id, "maintainer").await;

    let is_admin: bool = sqlx::query_scalar("SELECT is_system_admin($1)")
        .bind(alice_id)
        .fetch_one(&pool)
        .await
        .expect("is_system_admin")
        .unwrap_or(false);
    assert!(!is_admin, "precondition: a gating-team maintainer is NOT a system admin");

    let gating_id: Uuid = sqlx::query_scalar("SELECT id FROM kb_teams WHERE slug = 'temper-system'")
        .fetch_one(&pool)
        .await
        .expect("gating team id");

    let (status, _) = issue(
        &app,
        &alice,
        json!({
            "label": "trojan",
            "owner_team_id": alice_team,
            "teams": [{ "team_id": gating_id, "role": "owner" }],
            "grants": []
        }),
    )
    .await;

    assert_eq!(status, 403, "minting a gating-team OWNER is an escalation to system admin");

    let admin_machines: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM kb_machine_clients mc
           JOIN kb_team_members tm ON tm.profile_id = mc.profile_id
          WHERE tm.team_id = $1 AND tm.role = 'owner'",
    )
    .bind(gating_id)
    .fetch_one(&pool)
    .await
    .expect("count admin machines");
    assert_eq!(admin_machines, 0, "no machine may hold owner on the gating team");
}

/// Spec D5 — a system admin retains full, unchecked reach.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn system_admin_retains_full_reach(pool: PgPool) {
    let app = common::setup(pool.clone()).await;

    let admin = common::generate_test_jwt("b2-admin", "b2-admin@example.com");
    let admin_id = provision_profile(&app, &admin).await;
    common::make_system_admin(&pool, admin_id).await;

    let bob = common::generate_test_jwt("b2-bob2", "b2-bob2@example.com");
    provision_profile(&app, &bob).await;
    let bob_team = create_team(&app, &bob, "b2-admin-foreign").await;

    let (status, body) = issue(
        &app,
        &admin,
        json!({
            "label": "operator agent",
            "owner_team_id": null,
            "teams": [{ "team_id": bob_team, "role": "member" }],
            "grants": []
        }),
    )
    .await;

    assert_eq!(status, 200, "an admin may confer any reach (Phase A D5): {body:?}");
}

/// Spec D5 — reads and per-row lifecycle are scoped to the owning team.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn reads_and_lifecycle_are_scoped_to_the_owning_team(pool: PgPool) {
    let app = common::setup(pool.clone()).await;
    let client = reqwest::Client::new();

    let alice = common::generate_test_jwt("b2-r-alice", "b2-r-alice@example.com");
    provision_profile(&app, &alice).await;
    let alice_team = create_team(&app, &alice, "b2-r-alice-team").await;

    let bob = common::generate_test_jwt("b2-r-bob", "b2-r-bob@example.com");
    provision_profile(&app, &bob).await;
    create_team(&app, &bob, "b2-r-bob-team").await;

    let (status, body) = issue(
        &app,
        &alice,
        json!({ "label": "alice agent", "owner_team_id": alice_team, "teams": [], "grants": [] }),
    )
    .await;
    assert_eq!(status, 200);
    let machine_id = body["client"]["id"].as_str().expect("machine id").to_string();

    // Alice sees her machine; Bob sees none.
    let mine: serde_json::Value = client
        .get(app.url("/api/machine-clients"))
        .bearer_auth(&alice)
        .send()
        .await
        .expect("list as alice")
        .json()
        .await
        .expect("json");
    assert_eq!(mine.as_array().expect("array").len(), 1, "Alice sees her own machine");

    let theirs: serde_json::Value = client
        .get(app.url("/api/machine-clients"))
        .bearer_auth(&bob)
        .send()
        .await
        .expect("list as bob")
        .json()
        .await
        .expect("json");
    assert!(
        theirs.as_array().expect("array").is_empty(),
        "Bob owns a team, but none of Alice's machines"
    );

    // Bob cannot GET, revoke, or rotate Alice's machine.
    let resp = client
        .get(app.url(&format!("/api/machine-clients/{machine_id}")))
        .bearer_auth(&bob)
        .send()
        .await
        .expect("get as bob");
    assert_eq!(resp.status(), 403, "Bob cannot read Alice's machine");

    let resp = client
        .post(app.url(&format!("/api/machine-clients/{machine_id}/rotate-secret")))
        .bearer_auth(&bob)
        .json(&json!({ "grace_seconds": 0 }))
        .send()
        .await
        .expect("rotate as bob");
    assert_eq!(resp.status(), 403, "Bob cannot rotate Alice's machine's secret");

    let resp = client
        .delete(app.url(&format!("/api/machine-clients/{machine_id}")))
        .bearer_auth(&bob)
        .send()
        .await
        .expect("revoke as bob");
    assert_eq!(resp.status(), 403, "Bob cannot revoke Alice's machine");

    // Alice can do all three — this is the point of the phase: no operator in the loop.
    let resp = client
        .post(app.url(&format!("/api/machine-clients/{machine_id}/rotate-secret")))
        .bearer_auth(&alice)
        .json(&json!({ "grace_seconds": 0 }))
        .send()
        .await
        .expect("rotate as alice");
    assert_eq!(resp.status(), 200, "Alice rotates her own machine's secret");

    let resp = client
        .delete(app.url(&format!("/api/machine-clients/{machine_id}")))
        .bearer_auth(&alice)
        .send()
        .await
        .expect("revoke as alice");
    assert_eq!(resp.status(), 200, "Alice revokes her own machine");
}
```

- [ ] **Step 3: Run the e2e suite**

On macOS, freshly-built e2e binaries hang at nextest's `--list` enumeration, so drive this file with plain `cargo test --test`:

```bash
cargo test -p e2e-tests --features test-db --test machine_registration_authz_e2e -- --nocapture
```

Expected: all 7 tests PASS.

- [ ] **Step 4: Confirm the Phase A / B1 gate tests still pass**

The verifier and the Phase A gate are untouched by B2; prove it.

```bash
cargo test -p e2e-tests --features test-db --test machine_gate_e2e
cargo test -p e2e-tests --features test-db --test auth_seam_e2e
```

Expected: all pass. One known interaction: `machine_gate_e2e::non_admin_cannot_issue_a_machine_credential` asserts a 403 for an unregistered non-admin issuing with `owner_team_id: null` — under B2 that is *still* 403 (D2: NULL is admin-only), so it must remain green. If it now returns 200, D2 has been implemented wrong.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add tests/e2e/tests/common/mod.rs tests/e2e/tests/machine_registration_authz_e2e.rs
git commit --no-verify -m "test(e2e): B2 registration authz + reach containment matrix"
```

---

### Task 6: Regenerate sqlx caches, full check, push

**Files:**
- Modify: `.sqlx/`, `crates/temper-services/.sqlx/`, `crates/temper-api/.sqlx/`, `tests/e2e/.sqlx/`

- [ ] **Step 1: Regenerate every cache, in order**

Order matters: the workspace ritual first, then the per-crate ones (which capture **test-target** queries the workspace pass skips).

```bash
export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo make prepare-api
cargo make prepare-e2e
```

- [ ] **Step 2: Stage caches by path — never `git add .sqlx`**

`prepare-api` / `prepare-services` materialize ~200 untracked `.sqlx` files; blanket-adding them pollutes the diff. Stage only what git already tracks as modified, plus genuinely new entries for queries this plan added.

```bash
git add -u .sqlx crates/temper-services/.sqlx crates/temper-api/.sqlx tests/e2e/.sqlx
git status --short | head -30
```

Then review the diff and `git add` by explicit path any *new* cache entry corresponding to a query this plan introduced (the scoped `list`, the `machine_authz` test queries). Deleting a query's last caller orphans its cache entry — `git rm` those by path.

- [ ] **Step 3: The honest offline probe**

`cargo make check` forces `SQLX_OFFLINE=true`, so it is the only local command that proves the committed caches are complete. It also runs `openapi-check` and the temper-rb gem drift check (Docker needed).

```bash
cargo make check
```

Expected: green. A `query!` E0282 here means a cache entry is missing — re-run the matching `prepare` task.

- [ ] **Step 4: Full test sweep**

```bash
cargo make test-db
cargo test -p e2e-tests --features test-db --test machine_registration_authz_e2e
cargo test -p e2e-tests --features test-db --test machine_gate_e2e
cargo test -p e2e-tests --features test-db --test auth_seam_e2e
```

Expected: all green.

- [ ] **Step 5: Commit and push**

```bash
git add -u
git commit -m "chore(sqlx): regenerate caches for Phase B2 queries"
git merge origin/main    # CI runs pull/<n>/merge; reconcile first
git push -u origin jct/g3-phase-b2-team-owner-registration
```

Then open the PR. **Never merge locally.**

---

## Self-Review

**Spec coverage:**

| Spec | Task |
|---|---|
| D1 — team-ownership predicate | 2 |
| D2 — NULL owning team denies | 2 (`none_team_is_admin_only`), 5 (`non_admin_cannot_issue_a_teamless_machine`) |
| D3 — `AuthorizedReach`, bypass unrepresentable | 2, 3 |
| D4 — containment bar = human bar, by call | 2 (`contain_reach`) |
| D4a — role bar / gating-team escalation | 2 (`cannot_mint_owner_role_on_*`), 5 (bite test) |
| D5 — full lifecycle keyed by owning team; scoped `list` | 4, 5 |
| D6 — machines never re-delegate (`can_grant = false`) | unchanged from Phase A; no task needed (Task 3 leaves `insert_grant`'s `can_grant: false` intact) |
| D7 — `add_member` owner-guard | 1 |
| D8 — no migration | enforced by Global Constraints |
| §5 test matrix | 5 |
| §6 machine-RBAC consequence | narrative; no code |

**Type consistency:** `authorize` / `authorize_registration` / `AuthorizedReach::{teams,grants}` / `get_for_caller` / `list(pool, caller, include_revoked)` / `rotate_secret(pool, caller, id, grace_seconds)` are used identically in every task that references them. `MachineAuthority` derives `PartialEq` because Task 2's tests `assert_eq!` on it.

**No red states:** every task ends with a compiling tree. `list`'s service signature and its handler call site move together inside Task 4 for exactly this reason.
