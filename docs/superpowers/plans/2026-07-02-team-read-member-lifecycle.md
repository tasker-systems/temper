# Team Read + Member Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give teams a usable read + membership-management surface: view a team and its members, remove a member (incl. self-leave), and change a member's role — through proper endpoints and CLI commands.

**Architecture:** Mirror the existing `team_service` pattern exactly — thin `handlers/teams.rs` handlers dispatch one service-direct `team_service` function each (no Backend-trait command, no event emission), returning typed rows. Authz reuses the existing `pub(crate)` helpers `role_on_team` + `can_manage`; auth precedes every write. No migration — `source`, roles, and ownership all live on existing `kb_team_members`/`kb_teams` columns.

**Tech Stack:** Rust, Axum (temper-api), sqlx macros against Postgres, temper-client (reqwest wrapper), clap (temper-cli), ts-rs (wire type generation), cargo-nextest, `#[sqlx::test]`.

**Spec:** `docs/superpowers/specs/2026-07-02-team-read-member-lifecycle-design.md`
**Task:** `019f25d9-c112-7042-bf0c-62a0f6a1d981` (goal `teams-in-temper`)

## Global Constraints

- **`--all-features`** required for all builds/clippy/check.
- **Typed structs over inline JSON**; **params structs** if a fn exceeds 5 domain params.
- **Auth before writes** — every mutator checks authz before any DB write.
- **Service owns SQL** — no `sqlx::query!()` in handlers or CLI; production queries use the compile-time macros (`query_as!`/`query_scalar!`/`query!`).
- **Test-fixture writes** use runtime `sqlx::query(...)` (not the macro), per project convention.
- **All public types implement `Debug`.**
- Wire types shared Rust↔TS live in `temper-core` with `ts-rs` derives; regenerate with `cargo make generate-ts-types`.
- **sqlx offline cache:** after adding/changing production macro queries, regenerate: `cargo sqlx prepare --workspace -- --all-features`, then `cargo make prepare-services`, then `cargo make prepare-api` (per-crate last). `cargo make check` runs `SQLX_OFFLINE=true` — it is the honest local probe of the committed cache.
- **Membership-semantics tests need the e2e tier** — `test-db` green alone is a false signal (the e2e harness mints admins via direct `kb_team_members` owner-writes). Run `cargo make test-e2e`.
- **DATABASE_URL** for local macro checks / `#[sqlx::test]` under bare cargo: `postgresql://temper:temper@localhost:5437/temper_development` (the `cargo make` tasks export it).
- Commit per task. Branch is `jct/teams-member-lifecycle` (already created off `main`). Do **not** push or open a PR until the whole plan is green and the user approves.

## File Structure

| File | Responsibility | Change |
|---|---|---|
| `crates/temper-core/src/types/team.rs` | wire types | add `TeamMemberSource`, `TeamMemberDetail`, `TeamDetail`, `ChangeRoleRequest` |
| `crates/temper-services/src/services/team_service.rs` | team business logic + SQL | add `team_detail`, `remove_member`, `change_role`, `count_owners`, `load_member` helper; unit tests |
| `crates/temper-api/src/handlers/teams.rs` | thin HTTP handlers | add `detail`, `remove_member`, `change_role` |
| `crates/temper-api/src/routes.rs` | route registration | add `GET /api/teams/{id}` and `DELETE`+`PATCH /api/teams/{id}/members/{profile_id}` |
| `crates/temper-client/src/teams.rs` | typed sub-client | add `get`, `remove_member`, `change_role` |
| `crates/temper-cli/src/cli.rs` | clap arg model | rename `TeamAction::Leave`→`WithdrawRequest`; add `Show`, `Leave`, `RemoveMember`, `SetRole` |
| `crates/temper-cli/src/main.rs` | command dispatch | update `TeamAction` match arms |
| `crates/temper-cli/src/commands/team.rs` | CLI handlers | rename `leave`→`withdraw_request`; add `show_remote`, `leave_remote`, `remove_member_remote`, `set_role_remote` |
| `tests/e2e/tests/team_member_lifecycle_test.rs` | end-to-end auth/membership matrix | new file |

---

### Task 1: Wire types in temper-core

**Files:**
- Modify: `crates/temper-core/src/types/team.rs` (append after the existing `*Row`/`*Request` wire types)
- Verify: `crates/temper-core/src/types/mod.rs` (re-export block for `team` — confirm the new types are covered by the existing `pub use team::*` or add them)

**Interfaces:**
- Produces: `TeamMemberSource` (enum `Native|Idp`), `TeamMemberDetail { profile_id: Uuid, handle: String, role: TeamRole, source: TeamMemberSource }`, `TeamDetail { id, slug, name, created, auto_join_role: Option<TeamRole>, members: Vec<TeamMemberDetail> }`, `ChangeRoleRequest { role: TeamRole }`. Consumed by Tasks 2–7.

- [ ] **Step 1: Add the types**

Append to `crates/temper-core/src/types/team.rs` (match the derive style of the existing `TeamRow`):

```rust
/// Provenance of a team membership row. Maps to the `team_member_source`
/// Postgres enum (added by `20260702000001_saml_group_provisioning.sql`).
/// `Idp` rows are owned by SAML reconcile and are not user-mutable.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "team_member_source", rename_all = "snake_case")]
pub enum TeamMemberSource {
    Native,
    Idp,
}

/// A team member enriched with the profile handle and provenance — the row
/// shape returned inside `TeamDetail`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TeamMemberDetail {
    pub profile_id: Uuid,
    pub handle: String,
    pub role: TeamRole,
    pub source: TeamMemberSource,
}

/// Full team detail — the team row plus its member roster. Response body for
/// `GET /api/teams/{id}`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamDetail {
    pub id: Uuid,
    pub slug: String,
    pub name: String,
    pub created: DateTime<Utc>,
    pub auto_join_role: Option<TeamRole>,
    pub members: Vec<TeamMemberDetail>,
}

/// Request body for `PATCH /api/teams/{id}/members/{profile_id}`.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "team.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeRoleRequest {
    pub role: TeamRole,
}
```

- [ ] **Step 2: Verify it compiles (all features)**

Run: `cargo build -p temper-core --all-features`
Expected: success. If `mod.rs` uses explicit re-exports rather than `pub use team::*`, add `TeamMemberSource, TeamMemberDetail, TeamDetail, ChangeRoleRequest` to the `team::{…}` list and rebuild.

- [ ] **Step 3: Regenerate TS types**

Run: `cargo make generate-ts-types`
Expected: `team.ts` (under the ts-rs output dir) now contains `TeamDetail`, `TeamMemberDetail`, `TeamMemberSource`, `ChangeRoleRequest`. Commit whatever regenerated `.ts` files change (ride-along codegen is expected).

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/src/types/team.rs crates/temper-core/src/types/mod.rs
git add -A  # include regenerated *.ts
git commit -m "feat(core): add TeamDetail/TeamMemberDetail/TeamMemberSource/ChangeRoleRequest wire types"
```

---

### Task 2: `team_detail` service function

**Files:**
- Modify: `crates/temper-services/src/services/team_service.rs`
- Test: same file, `#[cfg(test)]` module

**Interfaces:**
- Consumes: `role_on_team`, `is_system_admin` (`access_service`), `TeamDetail`/`TeamMemberDetail` (Task 1).
- Produces: `pub async fn team_detail(pool: &PgPool, caller: ProfileId, team_id: Uuid) -> ApiResult<TeamDetail>`.

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` in `team_service.rs` (create the module if absent, mirroring `context_service`'s). Reuse this fixture idiom (runtime `sqlx::query`, per convention):

```rust
#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use sqlx::PgPool;
    use temper_core::types::team::{TeamMemberSource, TeamRole};

    /// Insert a profile with the given handle, return its id.
    async fn mk_profile(pool: &PgPool, handle: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id",
        )
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    /// Insert a root team with the given slug, return its id.
    async fn mk_team(pool: &PgPool, slug: &str) -> Uuid {
        sqlx::query_scalar(
            "INSERT INTO kb_teams (id, slug, name) VALUES (gen_random_uuid(), $1, $1) RETURNING id",
        )
        .bind(slug)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    async fn add(pool: &PgPool, team: Uuid, profile: Uuid, role: &str, source: &str) {
        sqlx::query(
            "INSERT INTO kb_team_members (team_id, profile_id, role, source) \
             VALUES ($1, $2, $3::team_role, $4::team_member_source)",
        )
        .bind(team)
        .bind(profile)
        .bind(role)
        .bind(source)
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn team_detail_lists_members_for_a_member(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let member = mk_profile(&pool, "member").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;
        add(&pool, team, member, "member", "native").await;

        let detail = team_detail(&pool, ProfileId::from(owner), team).await.unwrap();
        assert_eq!(detail.slug, "acme");
        assert_eq!(detail.members.len(), 2);
        assert!(detail
            .members
            .iter()
            .any(|m| m.handle == "member" && matches!(m.role, TeamRole::Member)
                && matches!(m.source, TeamMemberSource::Native)));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn team_detail_hides_from_non_member(pool: PgPool) {
        let owner = mk_profile(&pool, "owner").await;
        let outsider = mk_profile(&pool, "outsider").await;
        let team = mk_team(&pool, "acme").await;
        add(&pool, team, owner, "owner", "native").await;

        let denied = team_detail(&pool, ProfileId::from(outsider), team).await;
        assert!(matches!(denied, Err(ApiError::NotFound)));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p temper-services --features test-db -E 'test(team_detail_)'`
Expected: FAIL — `team_detail` not found.
(If a bare `cargo nextest -p temper-services` is used it may hang at list enumeration; always scope with `-E 'test(...)'`.)

- [ ] **Step 3: Implement `team_detail`**

Add to `team_service.rs` (after `list_teams`). Uses `is_system_admin` from `access_service` (already imported for `create_team`):

```rust
/// Full team detail (row + member roster with handles + provenance).
///
/// Visible to any member of the team, or to a system admin. Non-visible teams
/// return `NotFound` (not `Forbidden`) to avoid leaking team existence to
/// non-members — team slugs are globally unique and used in share flows.
pub async fn team_detail(
    pool: &PgPool,
    caller: ProfileId,
    team_id: Uuid,
) -> ApiResult<TeamDetail> {
    // Auth (read gate): member (any role) or system admin.
    let is_member = role_on_team(pool, team_id, caller).await?.is_some();
    if !is_member && !access_service::is_system_admin(pool, caller).await? {
        return Err(ApiError::NotFound);
    }

    let team = sqlx::query_as!(
        TeamRow,
        r#"SELECT id, slug, name, created,
                  auto_join_role AS "auto_join_role: TeamRole"
             FROM kb_teams WHERE id = $1"#,
        team_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or(ApiError::NotFound)?;

    let members = sqlx::query_as!(
        TeamMemberDetail,
        r#"SELECT tm.profile_id,
                  p.handle,
                  tm.role AS "role: TeamRole",
                  tm.source AS "source: TeamMemberSource"
             FROM kb_team_members tm
             JOIN kb_profiles p ON p.id = tm.profile_id
            WHERE tm.team_id = $1
            ORDER BY tm.role, p.handle"#,
        team_id,
    )
    .fetch_all(pool)
    .await?;

    Ok(TeamDetail {
        id: team.id,
        slug: team.slug,
        name: team.name,
        created: team.created,
        auto_join_role: team.auto_join_role,
        members,
    })
}
```

Add the imports at the top of the file: extend the `temper_core::types::team::{…}` use with `ChangeRoleRequest, TeamDetail, TeamMemberDetail, TeamMemberSource` (ChangeRoleRequest is used in Task 4 — add it now to avoid churn).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p temper-services --features test-db -E 'test(team_detail_)'`
Expected: PASS (2 tests).

- [ ] **Step 5: Regenerate sqlx cache + commit**

```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
git add crates/temper-services/src/services/team_service.rs .sqlx crates/temper-services/.sqlx
git commit -m "feat(services): add team_detail (member/admin read gate, member roster)"
```

---

### Task 3: `remove_member` service function

**Files:**
- Modify: `crates/temper-services/src/services/team_service.rs`
- Test: same file, `lifecycle_tests` module

**Interfaces:**
- Consumes: `role_on_team`, `can_manage`, fixtures from Task 2.
- Produces: `pub async fn remove_member(pool: &PgPool, caller: ProfileId, team_id: Uuid, target: Uuid) -> ApiResult<()>`; helpers `async fn count_owners(pool, team_id) -> ApiResult<i64>` and `async fn load_member(pool, team_id, profile) -> ApiResult<Option<(TeamRole, TeamMemberSource)>>`.

- [ ] **Step 1: Write the failing tests**

Add to `lifecycle_tests`:

```rust
#[sqlx::test(migrations = "../../migrations")]
async fn owner_removes_member(pool: PgPool) {
    let owner = mk_profile(&pool, "owner").await;
    let member = mk_profile(&pool, "member").await;
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;
    add(&pool, team, member, "member", "native").await;

    remove_member(&pool, ProfileId::from(owner), team, member)
        .await
        .unwrap();
    let detail = team_detail(&pool, ProfileId::from(owner), team).await.unwrap();
    assert_eq!(detail.members.len(), 1);
}

#[sqlx::test(migrations = "../../migrations")]
async fn member_can_self_leave_but_not_remove_others(pool: PgPool) {
    let owner = mk_profile(&pool, "owner").await;
    let a = mk_profile(&pool, "a").await;
    let b = mk_profile(&pool, "b").await;
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;
    add(&pool, team, a, "member", "native").await;
    add(&pool, team, b, "member", "native").await;

    // a removing b → Forbidden.
    let denied = remove_member(&pool, ProfileId::from(a), team, b).await;
    assert!(matches!(denied, Err(ApiError::Forbidden)));
    // a removing a (self-leave) → ok.
    remove_member(&pool, ProfileId::from(a), team, a).await.unwrap();
}

#[sqlx::test(migrations = "../../migrations")]
async fn cannot_remove_last_owner(pool: PgPool) {
    let owner = mk_profile(&pool, "owner").await;
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;

    let denied = remove_member(&pool, ProfileId::from(owner), team, owner).await;
    assert!(matches!(denied, Err(ApiError::Conflict(_))));
}

#[sqlx::test(migrations = "../../migrations")]
async fn cannot_remove_idp_row(pool: PgPool) {
    let owner = mk_profile(&pool, "owner").await;
    let idp = mk_profile(&pool, "idp").await;
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;
    add(&pool, team, idp, "member", "idp").await;

    let denied = remove_member(&pool, ProfileId::from(owner), team, idp).await;
    assert!(matches!(denied, Err(ApiError::Conflict(_))));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p temper-services --features test-db -E 'test(remove) or test(self_leave) or test(last_owner) or test(idp_row)'`
Expected: FAIL — `remove_member` not found.

- [ ] **Step 3: Implement `remove_member` + helpers**

```rust
/// Count the `owner`-role members of a team.
async fn count_owners(pool: &PgPool, team_id: Uuid) -> ApiResult<i64> {
    let n = sqlx::query_scalar!(
        "SELECT COUNT(*) FROM kb_team_members WHERE team_id = $1 AND role = 'owner'",
        team_id,
    )
    .fetch_one(pool)
    .await?;
    Ok(n.unwrap_or(0))
}

/// Load a member's role + provenance, if the row exists.
async fn load_member(
    pool: &PgPool,
    team_id: Uuid,
    profile: Uuid,
) -> ApiResult<Option<(TeamRole, TeamMemberSource)>> {
    let row = sqlx::query!(
        r#"SELECT role AS "role: TeamRole", source AS "source: TeamMemberSource"
             FROM kb_team_members WHERE team_id = $1 AND profile_id = $2"#,
        team_id,
        profile,
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| (r.role, r.source)))
}

/// Remove a member from a team. Owner/maintainer may remove others; any member
/// may remove themselves (self-leave). Refuses SAML-provisioned rows and refuses
/// to remove the last owner.
pub async fn remove_member(
    pool: &PgPool,
    caller: ProfileId,
    team_id: Uuid,
    target: Uuid,
) -> ApiResult<()> {
    // Auth before writes: manager, or self-leave.
    let is_self = *caller == target;
    if !is_self {
        match role_on_team(pool, team_id, caller).await? {
            Some(role) if can_manage(role) => {}
            _ => return Err(ApiError::Forbidden),
        }
    }

    let (target_role, source) = load_member(pool, team_id, target)
        .await?
        .ok_or(ApiError::NotFound)?;

    if matches!(source, TeamMemberSource::Idp) {
        return Err(ApiError::Conflict(
            "this membership is provisioned by SAML; change it via the identity provider"
                .to_string(),
        ));
    }
    if matches!(target_role, TeamRole::Owner) && count_owners(pool, team_id).await? == 1 {
        return Err(ApiError::Conflict(
            "cannot remove the last owner; transfer ownership or promote another member first"
                .to_string(),
        ));
    }

    sqlx::query!(
        "DELETE FROM kb_team_members WHERE team_id = $1 AND profile_id = $2",
        team_id,
        target,
    )
    .execute(pool)
    .await?;
    Ok(())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p temper-services --features test-db -E 'test(remove) or test(self_leave) or test(last_owner) or test(idp_row)'`
Expected: PASS.

- [ ] **Step 5: Regenerate sqlx cache + commit**

```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
git add crates/temper-services/src/services/team_service.rs .sqlx crates/temper-services/.sqlx
git commit -m "feat(services): add remove_member (self-leave, idp + last-owner guards)"
```

---

### Task 4: `change_role` service function

**Files:**
- Modify: `crates/temper-services/src/services/team_service.rs`
- Test: same file, `lifecycle_tests` module

**Interfaces:**
- Consumes: `role_on_team`, `can_manage`, `count_owners`, `load_member` (Task 3), `ChangeRoleRequest` (Task 1).
- Produces: `pub async fn change_role(pool: &PgPool, caller: ProfileId, team_id: Uuid, target: Uuid, new_role: TeamRole) -> ApiResult<TeamMemberRow>`.

- [ ] **Step 1: Write the failing tests**

```rust
#[sqlx::test(migrations = "../../migrations")]
async fn owner_changes_member_role(pool: PgPool) {
    let owner = mk_profile(&pool, "owner").await;
    let member = mk_profile(&pool, "member").await;
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;
    add(&pool, team, member, "member", "native").await;

    let row = change_role(&pool, ProfileId::from(owner), team, member, TeamRole::Maintainer)
        .await
        .unwrap();
    assert!(matches!(row.role, TeamRole::Maintainer));
}

#[sqlx::test(migrations = "../../migrations")]
async fn cannot_grant_owner_via_role_change(pool: PgPool) {
    let owner = mk_profile(&pool, "owner").await;
    let member = mk_profile(&pool, "member").await;
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;
    add(&pool, team, member, "member", "native").await;

    let denied = change_role(&pool, ProfileId::from(owner), team, member, TeamRole::Owner).await;
    assert!(matches!(denied, Err(ApiError::BadRequest(_))));
}

#[sqlx::test(migrations = "../../migrations")]
async fn cannot_demote_last_owner(pool: PgPool) {
    let owner = mk_profile(&pool, "owner").await;
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;

    let denied =
        change_role(&pool, ProfileId::from(owner), team, owner, TeamRole::Maintainer).await;
    assert!(matches!(denied, Err(ApiError::Conflict(_))));
}

#[sqlx::test(migrations = "../../migrations")]
async fn change_role_on_nonmember_is_not_found(pool: PgPool) {
    let owner = mk_profile(&pool, "owner").await;
    let ghost = mk_profile(&pool, "ghost").await;
    let team = mk_team(&pool, "acme").await;
    add(&pool, team, owner, "owner", "native").await;

    let denied = change_role(&pool, ProfileId::from(owner), team, ghost, TeamRole::Member).await;
    assert!(matches!(denied, Err(ApiError::NotFound)));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo nextest run -p temper-services --features test-db -E 'test(change_role) or test(grant_owner) or test(demote_last_owner)'`
Expected: FAIL — `change_role` not found.

- [ ] **Step 3: Implement `change_role`**

```rust
/// Change an existing member's role. Owner/maintainer only. Cannot create a
/// member (404 if absent), cannot grant `owner` (ownership is transferred, not
/// granted), refuses SAML rows, and refuses to demote the last owner.
pub async fn change_role(
    pool: &PgPool,
    caller: ProfileId,
    team_id: Uuid,
    target: Uuid,
    new_role: TeamRole,
) -> ApiResult<TeamMemberRow> {
    // Auth before writes.
    match role_on_team(pool, team_id, caller).await? {
        Some(role) if can_manage(role) => {}
        _ => return Err(ApiError::Forbidden),
    }

    if matches!(new_role, TeamRole::Owner) {
        return Err(ApiError::BadRequest(
            "cannot grant owner via role change; use ownership transfer".to_string(),
        ));
    }

    let (current_role, source) = load_member(pool, team_id, target)
        .await?
        .ok_or(ApiError::NotFound)?;

    if matches!(source, TeamMemberSource::Idp) {
        return Err(ApiError::Conflict(
            "this membership is provisioned by SAML; change it via the identity provider"
                .to_string(),
        ));
    }
    if matches!(current_role, TeamRole::Owner) && count_owners(pool, team_id).await? == 1 {
        return Err(ApiError::Conflict(
            "cannot remove the last owner; transfer ownership or promote another member first"
                .to_string(),
        ));
    }

    let row = sqlx::query_as!(
        TeamMemberRow,
        r#"UPDATE kb_team_members SET role = $3
            WHERE team_id = $1 AND profile_id = $2
        RETURNING team_id, profile_id, role AS "role: TeamRole", created"#,
        team_id,
        target,
        new_role as TeamRole,
    )
    .fetch_one(pool)
    .await?;
    Ok(row)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo nextest run -p temper-services --features test-db -E 'test(change_role) or test(grant_owner) or test(demote_last_owner)'`
Expected: PASS.

- [ ] **Step 5: Regenerate sqlx cache + commit**

```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
git add crates/temper-services/src/services/team_service.rs .sqlx crates/temper-services/.sqlx
git commit -m "feat(services): add change_role (owner-grant + idp + last-owner guards)"
```

---

### Task 5: API handlers + routes

**Files:**
- Modify: `crates/temper-api/src/handlers/teams.rs`
- Modify: `crates/temper-api/src/routes.rs`

**Interfaces:**
- Consumes: `team_service::team_detail/remove_member/change_role` (Tasks 2–4); `TeamDetail`, `ChangeRoleRequest`, `TeamMemberRow`.
- Produces: handlers `detail`, `remove_member`, `change_role`; the routes `GET /api/teams/{id}`, `DELETE`+`PATCH /api/teams/{id}/members/{profile_id}`.

- [ ] **Step 1: Add the handlers**

Append to `crates/temper-api/src/handlers/teams.rs` (extend the `use temper_core::types::team::{…}` line with `ChangeRoleRequest, TeamDetail`; add `use axum::http::StatusCode;` is already present):

```rust
#[utoipa::path(
    get,
    path = "/api/teams/{id}",
    tag = "Teams",
    params(("id" = Uuid, Path, description = "Team ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Team detail + members", body = TeamDetail),
        (status = 404, description = "Team not found or not visible to caller"),
    )
)]
pub async fn detail(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(team_id): Path<Uuid>,
) -> ApiResult<Json<TeamDetail>> {
    team_service::team_detail(&state.pool, ProfileId::from(auth.0.profile.id), team_id)
        .await
        .map(Json)
}

#[utoipa::path(
    delete,
    path = "/api/teams/{id}/members/{profile_id}",
    tag = "Teams",
    params(
        ("id" = Uuid, Path, description = "Team ID"),
        ("profile_id" = Uuid, Path, description = "Member profile ID"),
    ),
    security(("bearer_auth" = [])),
    responses(
        (status = 204, description = "Member removed"),
        (status = 403, description = "Forbidden (not owner/maintainer and not self)"),
        (status = 404, description = "Member not found"),
        (status = 409, description = "Cannot remove last owner or SAML-provisioned row"),
    )
)]
pub async fn remove_member(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((team_id, profile_id)): Path<(Uuid, Uuid)>,
) -> ApiResult<StatusCode> {
    team_service::remove_member(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        team_id,
        profile_id,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    patch,
    path = "/api/teams/{id}/members/{profile_id}",
    tag = "Teams",
    params(
        ("id" = Uuid, Path, description = "Team ID"),
        ("profile_id" = Uuid, Path, description = "Member profile ID"),
    ),
    security(("bearer_auth" = [])),
    request_body = ChangeRoleRequest,
    responses(
        (status = 200, description = "Role changed", body = TeamMemberRow),
        (status = 400, description = "Cannot grant owner via role change"),
        (status = 403, description = "Forbidden (not owner/maintainer)"),
        (status = 404, description = "Member not found"),
        (status = 409, description = "Cannot demote last owner or SAML-provisioned row"),
    )
)]
pub async fn change_role(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((team_id, profile_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<ChangeRoleRequest>,
) -> ApiResult<Json<TeamMemberRow>> {
    team_service::change_role(
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        team_id,
        profile_id,
        body.role,
    )
    .await
    .map(Json)
}
```

- [ ] **Step 2: Register the routes**

In `crates/temper-api/src/routes.rs`, immediately after the existing `.route("/api/teams/{id}/members", post(handlers::teams::add_member))` line, add:

```rust
        .route("/api/teams/{id}", get(handlers::teams::detail))
        .route(
            "/api/teams/{id}/members/{profile_id}",
            delete(handlers::teams::remove_member).patch(handlers::teams::change_role),
        )
```

(`get`, `delete`, `post` are already imported in this file; add `patch` to the `axum::routing::{…}` import if it is not already there.)

- [ ] **Step 3: Register handlers in the OpenAPI doc (if the crate enumerates paths)**

Check whether `crates/temper-api/src` has a utoipa `#[openapi(paths(...))]` list (grep `paths(`). If it does, add `handlers::teams::detail`, `handlers::teams::remove_member`, `handlers::teams::change_role` to it. If paths are auto-collected, skip.

Run: `grep -rn "paths(" crates/temper-api/src | head`

- [ ] **Step 4: Verify it compiles**

Run: `cargo build -p temper-api --all-features`
Expected: success.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/handlers/teams.rs crates/temper-api/src/routes.rs
git commit -m "feat(api): wire team detail + member remove/change-role endpoints"
```

---

### Task 6: temper-client methods

**Files:**
- Modify: `crates/temper-client/src/teams.rs`

**Interfaces:**
- Consumes: `TeamDetail`, `ChangeRoleRequest`, `TeamMemberRow`.
- Produces: `TeamsClient::get(id) -> Result<TeamDetail>`, `remove_member(id, profile) -> Result<()>`, `change_role(id, profile, &ChangeRoleRequest) -> Result<TeamMemberRow>`.

- [ ] **Step 1: Add the methods**

Follow the existing `list`/`create`/`add_member` shape in `teams.rs`. Extend the `use temper_core::types::team::{…}` with `ChangeRoleRequest, TeamDetail`. For the DELETE (no JSON body / no response body), match how other sub-clients issue a no-content request (grep `send_no_content` / `DELETE` in `crates/temper-client/src/contexts.rs:79-90` `unshare_team` for the exact helper name and use it verbatim):

```rust
    /// GET /api/teams/{id} — team detail + members.
    pub async fn get(&self, team_id: Uuid) -> Result<TeamDetail> {
        let token = self.http.token().await?;
        let path = format!("/api/teams/{team_id}");
        let req = self.http.get(&path);
        self.http
            .send_json(&Method::GET, &path, req, Some(&token))
            .await
    }

    /// PATCH /api/teams/{id}/members/{profile_id} — change a member's role.
    pub async fn change_role(
        &self,
        team_id: Uuid,
        profile_id: Uuid,
        request: &ChangeRoleRequest,
    ) -> Result<TeamMemberRow> {
        let token = self.http.token().await?;
        let path = format!("/api/teams/{team_id}/members/{profile_id}");
        let req = self.http.patch(&path).json(request);
        self.http
            .send_json(&Method::PATCH, &path, req, Some(&token))
            .await
    }

    /// DELETE /api/teams/{id}/members/{profile_id} — remove a member (or self-leave).
    pub async fn remove_member(&self, team_id: Uuid, profile_id: Uuid) -> Result<()> {
        let token = self.http.token().await?;
        let path = format!("/api/teams/{team_id}/members/{profile_id}");
        // Mirror `ContextClient::unshare_team`'s no-content DELETE idiom exactly.
        let req = self.http.delete(&path);
        self.http
            .send_no_content(&Method::DELETE, &path, req, Some(&token))
            .await
    }
```

> Note: the exact token-fetch (`self.http.token()` vs a `TokenStore`), method-builder (`self.http.get/patch/delete`), and no-content sender names must be copied from the sibling methods already in `teams.rs`/`contexts.rs` — the snippet above shows structure, not necessarily the exact helper names in this codebase. Match the file.

- [ ] **Step 2: Verify it compiles**

Run: `cargo build -p temper-client --all-features`
Expected: success.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-client/src/teams.rs
git commit -m "feat(client): TeamsClient get / remove_member / change_role"
```

---

### Task 7: CLI commands (rename leave, add show/leave/remove-member/set-role)

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (`TeamAction` enum, ~lines 568-617)
- Modify: `crates/temper-cli/src/main.rs` (`TeamAction` dispatch, ~lines 339-384)
- Modify: `crates/temper-cli/src/commands/team.rs`

**Interfaces:**
- Consumes: `TeamsClient::get/remove_member/change_role`, `ProfileClient::get` (`client.profile().get().await?.id`), `resolve_team_id` (`crates/temper-cli/src/actions/cogmap.rs`), `parse_role` (`commands/team.rs:10`).
- Produces: CLI verbs `team show`, `team leave`, `team remove-member`, `team set-role`, `team withdraw-request`.

- [ ] **Step 1: Update the `TeamAction` enum in `cli.rs`**

Rename the existing `Leave` variant to `WithdrawRequest` (keep its current fields: optional `--team`, `--message`? — check the current variant; it currently withdraws a request). Then add:

```rust
    /// Show a team's detail and member roster.
    Show {
        /// Team slug (optionally `+`-prefixed) or UUID.
        team: String,
    },
    /// Leave a team you are a member of (removes your membership).
    Leave {
        /// Team slug (optionally `+`-prefixed) or UUID.
        team: String,
    },
    /// Remove a member from a team (owner/maintainer).
    RemoveMember {
        /// Team slug (optionally `+`-prefixed) or UUID.
        team: String,
        /// Member profile UUID.
        profile: String,
    },
    /// Change a member's role (owner/maintainer).
    SetRole {
        /// Team slug (optionally `+`-prefixed) or UUID.
        team: String,
        /// Member profile UUID.
        profile: String,
        /// New role: maintainer | member | watcher (owner is via transfer).
        #[arg(long)]
        role: String,
    },
```

Keep the renamed variant:

```rust
    /// Withdraw your pending join request to the system gating team.
    WithdrawRequest,
```

(The old `Leave` variant's body — a bare join-request withdrawal — moves under this name. If the old `Leave` carried `--team`/`--message`, drop `--message` only if it was unused; preserve fields that were actually read in dispatch.)

- [ ] **Step 2: Update dispatch in `main.rs`**

Replace the old `TeamAction::Leave { .. } => …` arm and add the new arms. The `withdraw-request` arm calls the renamed handler; the new arms call the new remote handlers. Example shape (match the surrounding async dispatch idiom — these commands need a client, so they follow the `create_remote`/`add_member_remote` pattern that builds a client and passes `fmt`):

```rust
        TeamAction::WithdrawRequest => commands::team::withdraw_request(),
        TeamAction::Show { team } => {
            with_client_and_fmt(|client, fmt| commands::team::show_remote(client, &team, fmt)).await
        }
        TeamAction::Leave { team } => {
            with_client_and_fmt(|client, fmt| commands::team::leave_remote(client, &team, fmt)).await
        }
        TeamAction::RemoveMember { team, profile } => {
            with_client_and_fmt(|client, fmt| {
                commands::team::remove_member_remote(client, &team, &profile, fmt)
            })
            .await
        }
        TeamAction::SetRole { team, profile, role } => {
            with_client_and_fmt(|client, fmt| {
                commands::team::set_role_remote(client, &team, &profile, &role, fmt)
            })
            .await
        }
```

> `with_client_and_fmt` is a placeholder for whatever the existing arms use to obtain `(client, fmt)` — copy the exact idiom used by the current `TeamAction::Create`/`List`/`AddMember` arms in `main.rs:352-381` (they already build a client and pass `fmt`). Do not invent a new helper.

- [ ] **Step 3: Rename `leave` → `withdraw_request` in `commands/team.rs`**

Rename the existing `pub fn leave()` to `pub fn withdraw_request()` — body unchanged (it already does the join-request withdrawal). Update its doc-comment to "Withdraw a pending join request."

- [ ] **Step 4: Add the new remote handlers in `commands/team.rs`**

```rust
use crate::actions::cogmap::resolve_team_id;
use temper_core::types::team::ChangeRoleRequest;

/// Show a team's detail + members.
pub async fn show_remote(
    client: &temper_client::TemperClient,
    team: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = resolve_team_id(client, team).await?;
    let detail = client
        .teams()
        .get(team_id)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&detail, fmt)?);
    Ok(())
}

/// Leave a team you are a member of (self-removal).
pub async fn leave_remote(
    client: &temper_client::TemperClient,
    team: &str,
    _fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = resolve_team_id(client, team).await?;
    let me = client
        .profile()
        .get()
        .await
        .map_err(crate::commands::client_err)?;
    client
        .teams()
        .remove_member(team_id, me.id)
        .await
        .map_err(crate::commands::client_err)?;
    output::success("You have left the team.");
    Ok(())
}

/// Remove a member from a team (owner/maintainer).
pub async fn remove_member_remote(
    client: &temper_client::TemperClient,
    team: &str,
    profile: &str,
    _fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = resolve_team_id(client, team).await?;
    let profile_id = uuid::Uuid::parse_str(profile)
        .map_err(|e| TemperError::Api(format!("invalid profile id '{profile}': {e}")))?;
    client
        .teams()
        .remove_member(team_id, profile_id)
        .await
        .map_err(crate::commands::client_err)?;
    output::success("Member removed.");
    Ok(())
}

/// Change a member's role (owner/maintainer).
pub async fn set_role_remote(
    client: &temper_client::TemperClient,
    team: &str,
    profile: &str,
    role: &str,
    fmt: crate::format::OutputFormat,
) -> Result<()> {
    let team_id = resolve_team_id(client, team).await?;
    let profile_id = uuid::Uuid::parse_str(profile)
        .map_err(|e| TemperError::Api(format!("invalid profile id '{profile}': {e}")))?;
    let req = ChangeRoleRequest {
        role: parse_role(role)?,
    };
    let member = client
        .teams()
        .change_role(team_id, profile_id, &req)
        .await
        .map_err(crate::commands::client_err)?;
    println!("{}", crate::format::render(&member, fmt)?);
    Ok(())
}
```

- [ ] **Step 5: Verify it compiles + build the binary**

Run: `cargo build -p temper-cli --all-features --bin temper`
Expected: success. (Building the bin explicitly matters — nextest rebuilds the lib, not the bin, and the e2e suite in Task 8 spawns the `temper` binary.)

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs crates/temper-cli/src/commands/team.rs
git commit -m "feat(cli): team show/leave/remove-member/set-role; rename leave->withdraw-request"
```

---

### Task 8: End-to-end membership matrix

**Files:**
- Create: `tests/e2e/tests/team_member_lifecycle_test.rs`

**Interfaces:**
- Consumes: the e2e harness in `tests/e2e/tests/common/` (spawns a real Axum server + Postgres, mints authed users). Read `tests/e2e/tests/common/mod.rs` and an existing team/access e2e test first to copy the harness idiom (server spawn, JWT minting, admin/owner minting via direct `kb_team_members` owner-writes).

- [ ] **Step 1: Study the harness**

Run: `ls tests/e2e/tests && sed -n '1,120p' tests/e2e/tests/common/mod.rs`
Then read one existing membership/access test (e.g. `grep -rln "kb_team_members\|create_team\|/api/teams" tests/e2e/tests | head`).

- [ ] **Step 2: Write the e2e test**

Cover the full matrix through the API (and at least one path through the spawned `temper` CLI binary to prove the wiring):
1. Owner creates a team, `GET /api/teams/{id}` returns the owner in the roster.
2. Owner adds a member; `GET` now lists 2; a **non-member** `GET` returns 404.
3. Owner `PATCH`es the member to `maintainer` → 200; `PATCH` to `owner` → 400.
4. Member self-`DELETE` → 204; re-add; a plain member `DELETE`-ing another → 403.
5. Owner self-`DELETE` as last owner → 409.
6. Insert an `idp`-sourced member row directly; owner `DELETE` on it → 409.

Model the structure on the existing e2e tests exactly — do not invent harness helpers. Each assertion checks both the HTTP status and (where relevant) the follow-up `GET` roster.

- [ ] **Step 3: Build the CLI binary the e2e suite spawns**

Run: `cargo build -p temper-cli --bin temper`
Expected: success (fresh binary; avoids running a stale `temper`).

- [ ] **Step 4: Run the e2e test**

Run: `cargo make test-e2e` (or scope: `cargo nextest run -p temper-e2e --features test-db -E 'test(team_member_lifecycle)'`)
Expected: PASS. If a fresh e2e binary hangs at nextest `--list` on macOS, run the one target via `cargo test -p temper-e2e --test team_member_lifecycle_test` to bypass list-all.

- [ ] **Step 5: Regenerate e2e sqlx cache (if the test uses macro queries) + commit**

```bash
cargo make prepare-e2e   # only if the test file uses sqlx::query! macros
git add tests/e2e/tests/team_member_lifecycle_test.rs tests/e2e/.sqlx
git commit -m "test(e2e): team member lifecycle auth/guard matrix"
```

---

### Task 9: Full verification gate

**Files:** none (verification + cache consolidation only)

- [ ] **Step 1: Consolidate sqlx caches**

```bash
DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development \
  cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo make prepare-api
```
Prune any orphaned `.sqlx` files if code moved (none expected here). Commit if anything changed:
```bash
git add .sqlx crates/temper-services/.sqlx crates/temper-api/.sqlx
git commit -m "chore: regenerate sqlx offline caches" || true
```

- [ ] **Step 2: Full check (offline — the honest CI probe)**

Run: `cargo make check`
Expected: PASS (fmt + clippy `-D warnings` + docs + machete, and offline sqlx). Fix any `cargo fmt` drift (`cargo make fix`) and re-run — the pre-commit hook gates on `cargo fmt --check`.

- [ ] **Step 3: Full test suite**

Run: `cargo make test && cargo make test-db && cargo make test-e2e`
Expected: PASS. `test-db` alone is a false signal for membership semantics — the e2e run is the load-bearing one here.

- [ ] **Step 4: TypeScript typecheck (regenerated types ride along)**

Run: `cargo make generate-ts-types && (cd packages/temper-ui && bun run check)`
Expected: no type errors from the new `team.ts` exports. Commit any regenerated `.ts`.

- [ ] **Step 5: Final commit + hand back for review**

```bash
git add -A && git commit -m "chore: finalize team read + member lifecycle" || true
```
Do **not** push or open a PR. Report the branch state and the task/goal refs; the user decides on push/PR (per standing preference: always push+PR when finishing, but only after the user reviews).

---

## Self-Review

**Spec coverage:**
- `GET /api/teams/:id` → Tasks 2 (service), 5 (handler+route), 6 (client), 7 (`team show`), 8 (e2e). ✓
- `DELETE …/members/:pid` (remove + self-leave) → Tasks 3, 5, 6, 7 (`team leave`, `remove-member`), 8. ✓
- `PATCH …/members/:pid` (role change) → Tasks 4, 5, 6, 7 (`set-role`), 8. ✓
- `team leave` overload split → Task 7 (rename to `withdraw-request`; new `leave`). ✓
- Last-owner guard (remove + demote) → Tasks 3, 4; e2e 8. ✓
- idp-row refusal (DELETE + PATCH) → Tasks 3, 4; e2e 8. ✓
- 404-not-403 for non-visible team → Task 2 test `team_detail_hides_from_non_member`. ✓
- `TeamMemberSource`/`TeamDetail`/`TeamMemberDetail`/`ChangeRoleRequest` → Task 1. ✓
- No migration → confirmed; nothing in the plan adds one. ✓
- sqlx cache regen + e2e tier + `--all-features` → Global Constraints + per-task steps + Task 9. ✓

**Placeholder scan:** The three "match the existing idiom" notes (client no-content sender in Task 6, `with_client_and_fmt` in Task 7, e2e harness in Task 8) are deliberate — they point the implementer at the exact sibling code to copy because the precise helper names live in files not fully quoted here. Each names the sibling file+lines to copy from. No `TBD`/`implement later`/"add error handling" placeholders remain.

**Type consistency:** `team_detail`/`remove_member`/`change_role` signatures, `ChangeRoleRequest { role }`, `TeamMemberSource { Native, Idp }`, and `TeamMemberDetail` fields are identical across Tasks 1→2→3→4→5→6→7. `count_owners`/`load_member` defined in Task 3, reused in Task 4. Error variants (`NotFound`/`Forbidden`/`BadRequest`/`Conflict`) match the spec's mapping and `ApiError`'s actual variants.
