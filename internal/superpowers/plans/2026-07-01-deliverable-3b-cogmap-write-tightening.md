# Deliverable 3b — Cogmap-Write Tightening (Q-A) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move cogmap authorship from flat team-cogmap membership to an explicit `can_write` grant (Q-A), with creator seeding, a full-parity grant/revoke surface, and a one-time member snapshot so existing authoring survives.

**Architecture:** One forward migration flips `cogmap_authorable_by_profile` to `profile_explicit_grant(…,'write',…)` and co-commits a per-profile backfill (excluding auto-join teams). `DbBackend::create_cognitive_map` seeds the invoking admin a bootstrap grant. A subject-polymorphic `access_service::grant_capability`/`revoke_capability` primitive (mirroring the `bind_team` admin-event pattern — called directly from surfaces, not the cognitive backend) is exposed as cogmap-scoped verbs on CLI + MCP + HTTP.

**Tech Stack:** Rust (sqlx, axum, rmcp, clap), PostgreSQL 18 + pgvector, cargo-make / cargo-nextest.

**Spec:** [docs/superpowers/specs/2026-07-01-deliverable-3b-cogmap-write-tightening-design.md](../specs/2026-07-01-deliverable-3b-cogmap-write-tightening-design.md)

## Global Constraints

- **Additive-only-on-`main`:** every DB change is a new forward migration; never edit an applied migration (sqlx checksums the whole file). New migration: `migrations/20260701000001_cogmap_write_tightening.sql`.
- **Persistence layering:** SQL lives in `temper-services` (services) / `temper-substrate` (writes/readback), never inlined in a surface. Grants are **admin events** (firewalled from cognition) → they call `access_service` **directly** from surfaces, like `bind_team` — NOT through the `DbBackend`/operations trait.
- **Auth before writes:** the authorization check precedes any mutation.
- **Shared types at boundaries:** the grant request/response wire types live in `temper-core` (ts-rs), shared by all three surfaces — no per-surface duplicate.
- **Typed structs over inline JSON:** no `serde_json::json!()` for structured data.
- **SQL macros:** production queries use `sqlx::query!()`/`query_scalar!()`/`query_as!()`; regenerate caches after SQL changes — `cargo sqlx prepare --workspace -- --all-features`, then `cargo make prepare-services`, `cargo make prepare-api`, `cargo make prepare-e2e`.
- **`SQLX_OFFLINE=true`** is forced by `cargo make` — `cargo make check` is the honest offline probe.
- **Gates before "done":** `cargo make check` clean; green under `test-artifacts` (`cargo make test-artifacts`) **and** e2e (`cargo make test-e2e`); flip touches `test-db` targets so run `cargo make test-db` too.
- **Local e2e stale-bin gotcha:** `cargo make test-e2e` does NOT rebuild the CLI binary — after a CLI change run `cargo build -p temper-cli --bin temper` first, or e2e sees the old binary.
- **`test-db` feature gate:** every file with `#[sqlx::test]` must start with `#![cfg(feature = "test-db")]`.
- **The system profile** (backfill `granted_by`, genesis actor) = `SELECT id FROM kb_profiles WHERE handle = 'system'`.

---

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `migrations/20260701000001_cogmap_write_tightening.sql` (new) | Q-A flip + per-profile backfill, atomic | 3 |
| `crates/temper-services/src/backend/db_backend.rs` (`create_cognitive_map`) | Creator bootstrap-grant insert in the create txn | 1 |
| `crates/temper-core/src/types/access_gate.rs` (or a new `access_grants.rs`) | `GrantCapabilityRequest` / `RevokeCapabilityRequest` / `GrantOutcome` wire types (ts-rs) | 2 |
| `crates/temper-services/src/services/access_service.rs` | `grant_capability` / `revoke_capability` + the `can(...,'grant',...) OR is_system_admin` gate | 2 |
| `crates/temper-cli/src/commands/cogmap.rs`, `actions/cogmap.rs`, `cli.rs`, `main.rs` | `temper cogmap grant` / `revoke` | 4 |
| `crates/temper-mcp/src/service.rs`, `tools/cognitive_maps.rs` | `cogmap_grant` / `cogmap_revoke` tools | 5 |
| `crates/temper-api/src/handlers/cognitive_maps.rs`, `routes.rs`, `openapi.rs` | `POST` / `DELETE /api/cognitive-maps/{id}/grants` | 6 |
| `crates/temper-api/tests/cogmap_home_test.rs` (`:181`, `:405`) | Flip membership⇒authorable assertions | 3 |
| `crates/temper-api/tests/access_grants_test.rs` (new) | grant/revoke service unit + backfill-query test | 2, 3 |
| `tests/e2e/tests/cogmap_write_grants_test.rs` (new) | §5 end-to-end scenarios | 7 |

---

## Task 1: Creator seeding in `create_cognitive_map` (additive)

Seeds the invoking admin a `can_read+can_write+can_grant` grant on the map they create, inside the existing create transaction. Purely additive while the stub still ignores write grants — so this task's test asserts the **grant row**, not authoring (authoring materializes after the Task 3 flip).

**Files:**
- Modify: `crates/temper-services/src/backend/db_backend.rs` (`create_cognitive_map`, insert between the `close_invocation_in_tx` call and `tx.commit()`, ~`:1466`)
- Test: `crates/temper-api/tests/cognitive_map_handler_test.rs` (add a case) OR the existing services/backend test that drives `create_cognitive_map` — locate with `grep -rn "create_cognitive_map" crates/*/tests`

**Interfaces:**
- Consumes: `self.pool`, `self.profile_id` (`ProfileId`), `born_cogmap` (`CogmapId`), the open `tx`.
- Produces: a `kb_access_grants` row `(subject_table='kb_cogmaps', subject_id=<map>, principal_table='kb_profiles', principal_id=<caller>, can_read=can_write=can_grant=true, can_delete=false, granted_by_profile_id=<caller>)`.

- [ ] **Step 1: Write the failing test.** In the backend/handler test that creates a cognitive map, after the create call, assert the creator grant row exists:

```rust
// After creating a map as `caller_profile_id`, the creator holds a can_write + can_grant grant.
let row = sqlx::query!(
    r#"SELECT can_read, can_write, can_grant, can_delete
         FROM kb_access_grants
        WHERE subject_table = 'kb_cogmaps' AND subject_id = $1
          AND principal_table = 'kb_profiles' AND principal_id = $2"#,
    created_cogmap_id,
    caller_profile_id,
)
.fetch_one(&pool)
.await
.expect("creator grant row must exist");
assert!(row.can_read && row.can_write && row.can_grant);
assert!(!row.can_delete);
```

- [ ] **Step 2: Run it to verify it fails.** Run: `cargo nextest run -p temper-api --features test-db create_cognitive_map -E 'test(creator_grant)'` (adjust to the test name). Expected: FAIL — no grant row.

- [ ] **Step 3: Implement the seed.** In `create_cognitive_map`, after `close_invocation_in_tx(…)` and before `tx.commit()`:

```rust
// Creator bootstrap grant (spec §3.B): the INVOKING admin (self.profile_id, not the system actor
// genesis fires under) gets read+write+grant on the map they just created — a self-grant, the
// bootstrap admin event. Cogmaps have no ownership floor, so without this the creator could never
// author or add a co-author to their own (still-unbound) map. Only the create path reaches here
// (the re-genesis no-op returned earlier), and ON CONFLICT DO NOTHING guards a retried create.
let creator = uuid::Uuid::from(self.profile_id);
sqlx::query!(
    r#"INSERT INTO kb_access_grants
           (subject_table, subject_id, principal_table, principal_id,
            can_read, can_write, can_grant, granted_by_profile_id)
       VALUES ('kb_cogmaps', $1, 'kb_profiles', $2, true, true, true, $2)
       ON CONFLICT (subject_table, subject_id, principal_table, principal_id) DO NOTHING"#,
    uuid::Uuid::from(born_cogmap),
    creator,
)
.execute(&mut *tx)
.await
.map_err(api_err)?;
```

- [ ] **Step 4: Run test to verify it passes.** Same command as Step 2. Expected: PASS.

- [ ] **Step 5: Regenerate the services sqlx cache.** Run: `cargo sqlx prepare --workspace -- --all-features` then `cargo make prepare-services`. Verify `git status` shows updated `.sqlx`.

- [ ] **Step 6: `cargo make check`.** Expected: clean.

- [ ] **Step 7: Commit.**

```bash
git add crates/temper-services crates/temper-api/tests .sqlx crates/temper-services/.sqlx
git commit -m "feat(access): seed cogmap creator a write+grant bootstrap grant

D3b §3.B: create_cognitive_map grants the invoking admin can_read+write+grant
on the new map in the create txn. Additive (the stub still ignores write
grants); authoring materializes after the Q-A flip."
```

---

## Task 2: Grant/revoke service primitive + shared wire types (additive)

The single surface-facing writer of `kb_access_grants`. Subject-polymorphic service fn; gate = `is_system_admin(caller) OR can('kb_profiles', caller, 'grant', subject_table, subject_id)`. Mirrors the `bind_team` admin-event pattern (`cogmap_service.rs:26-55`).

**Files:**
- Create/modify: `crates/temper-core/src/types/access_gate.rs` — add wire types (ts-rs `#[derive(TS)]` gated by the `typescript` feature, like the sibling types).
- Modify: `crates/temper-services/src/services/access_service.rs` — add `grant_capability` / `revoke_capability`.
- Test: `crates/temper-api/tests/access_grants_test.rs` (new; `#![cfg(feature = "test-db")]`).

**Interfaces:**
- Produces:
```rust
// temper-core
pub struct GrantCapabilityRequest {
    pub subject_table: String,     // 'kb_cogmaps' (3b); validated by the service
    pub subject_id: Uuid,
    pub principal_table: String,   // 'kb_profiles' | 'kb_teams'
    pub principal_id: Uuid,
    pub can_read: bool,
    pub can_write: bool,
    pub can_delete: bool,
    pub can_grant: bool,
}
pub struct RevokeCapabilityRequest {
    pub subject_table: String,
    pub subject_id: Uuid,
    pub principal_table: String,
    pub principal_id: Uuid,
}
pub struct GrantOutcome { pub granted: bool }   // false when the row already existed (upsert no-op)

// temper-services access_service
pub async fn grant_capability(pool: &PgPool, caller: ProfileId, req: &GrantCapabilityRequest) -> ApiResult<GrantOutcome>;
pub async fn revoke_capability(pool: &PgPool, caller: ProfileId, req: &RevokeCapabilityRequest) -> ApiResult<()>;
```
- Consumes: `access_service::is_system_admin` (`:40`), the SQL `can(...)` function (`20260630000001:102`).

- [ ] **Step 1: Add the wire types.** In `crates/temper-core/src/types/access_gate.rs`, add the four structs above, following the existing derive stack in that file (`#[derive(Debug, Clone, Serialize, Deserialize)]`, `#[cfg_attr(feature = "typescript", derive(TS), ts(export))]`, and `#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]` — copy the attribute set from `BindTeamRequest`).

- [ ] **Step 2: Write the failing test.** In `crates/temper-api/tests/access_grants_test.rs`:

```rust
#![cfg(feature = "test-db")]
use sqlx::PgPool;
use temper_core::types::ids::ProfileId;
use temper_core::types::access_gate::{GrantCapabilityRequest, RevokeCapabilityRequest};
use temper_services::services::access_service;
// ... test harness helpers to mint a profile + a cogmap + set system_access ...

#[sqlx::test(migrator = "temper_api::MIGRATOR")]   // use the crate's real migrator
async fn admin_can_grant_and_revoke_cogmap_write(pool: PgPool) {
    let admin = mint_admin(&pool).await;          // system_access='admin'
    let grantee = mint_profile(&pool).await;      // no membership, no grant
    let cogmap = mint_unbound_cogmap(&pool).await;

    let req = GrantCapabilityRequest {
        subject_table: "kb_cogmaps".into(), subject_id: cogmap,
        principal_table: "kb_profiles".into(), principal_id: grantee,
        can_read: true, can_write: true, can_delete: false, can_grant: false,
    };
    let out = access_service::grant_capability(&pool, admin, &req).await.unwrap();
    assert!(out.granted);

    // The grant confers write through the general seam.
    let can_write: bool = sqlx::query_scalar!(
        "SELECT can('kb_profiles', $1, 'write', 'kb_cogmaps', $2)", grantee.uuid(), cogmap)
        .fetch_one(&pool).await.unwrap().unwrap();
    assert!(can_write);

    access_service::revoke_capability(&pool, admin,
        &RevokeCapabilityRequest {
            subject_table: "kb_cogmaps".into(), subject_id: cogmap,
            principal_table: "kb_profiles".into(), principal_id: grantee,
        }).await.unwrap();
    let can_write_after: bool = sqlx::query_scalar!(
        "SELECT can('kb_profiles', $1, 'write', 'kb_cogmaps', $2)", grantee.uuid(), cogmap)
        .fetch_one(&pool).await.unwrap().unwrap();
    assert!(!can_write_after);
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn non_granter_is_forbidden(pool: PgPool) {
    let stranger = mint_profile(&pool).await;     // not admin, no can_grant
    let grantee = mint_profile(&pool).await;
    let cogmap = mint_unbound_cogmap(&pool).await;
    let err = access_service::grant_capability(&pool, stranger, &GrantCapabilityRequest {
        subject_table: "kb_cogmaps".into(), subject_id: cogmap,
        principal_table: "kb_profiles".into(), principal_id: grantee,
        can_read: true, can_write: true, can_delete: false, can_grant: false,
    }).await.unwrap_err();
    assert!(matches!(err, temper_services::error::ApiError::Forbidden));
}
```

- [ ] **Step 3: Run it to verify it fails.** Run: `cargo nextest run -p temper-api --features test-db --test access_grants_test`. Expected: FAIL — `grant_capability` not defined.

- [ ] **Step 4: Implement `grant_capability` / `revoke_capability`.** In `access_service.rs`:

```rust
use temper_core::types::access_gate::{GrantCapabilityRequest, GrantOutcome, RevokeCapabilityRequest};

/// Mint/update an access grant (spec §3.C). Admin event — firewalled from cognition, called
/// directly from surfaces (bind_team precedent), NOT via the DbBackend trait.
///
/// Auth before write: the caller must be a system admin OR hold `can_grant` on the subject (the
/// general `can(...,'grant',...)` seam — grant-administration, a DIFFERENT axis from authoring:
/// authoring stays wholly explicit, §3.E). The coherence CHECK (write|delete|grant ⇒ read) is the
/// DB integrity backstop; callers should pass a coherent request (a write grant implies read).
pub async fn grant_capability(
    pool: &PgPool,
    caller: ProfileId,
    req: &GrantCapabilityRequest,
) -> ApiResult<GrantOutcome> {
    // Auth before write.
    if !can_administer_grant(pool, caller, &req.subject_table, req.subject_id).await? {
        return Err(ApiError::Forbidden);
    }
    let inserted = sqlx::query_scalar!(
        r#"INSERT INTO kb_access_grants
               (subject_table, subject_id, principal_table, principal_id,
                can_read, can_write, can_delete, can_grant, granted_by_profile_id)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           ON CONFLICT (subject_table, subject_id, principal_table, principal_id)
           DO UPDATE SET can_read = EXCLUDED.can_read, can_write = EXCLUDED.can_write,
                         can_delete = EXCLUDED.can_delete, can_grant = EXCLUDED.can_grant,
                         granted_by_profile_id = EXCLUDED.granted_by_profile_id,
                         granted_at = now()
           RETURNING (xmax = 0) AS "inserted!""#,   // xmax=0 ⇒ a fresh INSERT, not an UPDATE
        req.subject_table, req.subject_id, req.principal_table, req.principal_id,
        req.can_read, req.can_write, req.can_delete, req.can_grant, caller.uuid(),
    )
    .fetch_one(pool)
    .await?;
    Ok(GrantOutcome { granted: inserted })
}

pub async fn revoke_capability(
    pool: &PgPool,
    caller: ProfileId,
    req: &RevokeCapabilityRequest,
) -> ApiResult<()> {
    if !can_administer_grant(pool, caller, &req.subject_table, req.subject_id).await? {
        return Err(ApiError::Forbidden);
    }
    sqlx::query!(
        r#"DELETE FROM kb_access_grants
            WHERE subject_table = $1 AND subject_id = $2
              AND principal_table = $3 AND principal_id = $4"#,
        req.subject_table, req.subject_id, req.principal_table, req.principal_id,
    )
    .execute(pool)
    .await?;
    Ok(())   // absent row ⇒ no-op success (idempotent, mirrors bind_team)
}

/// Grant-administration gate: system admin OR `can_grant` on the subject.
async fn can_administer_grant(
    pool: &PgPool, caller: ProfileId, subject_table: &str, subject_id: Uuid,
) -> ApiResult<bool> {
    if is_system_admin(pool, caller).await? {
        return Ok(true);
    }
    let ok = sqlx::query_scalar!(
        "SELECT can('kb_profiles', $1, 'grant', $2, $3)",
        caller.uuid(), subject_table, subject_id,
    )
    .fetch_one(pool)
    .await?
    .unwrap_or(false);
    Ok(ok)
}
```

- [ ] **Step 5: Run tests to verify they pass.** Same command as Step 3. Expected: PASS (both cases).

- [ ] **Step 6: Regenerate caches + generate TS types.** Run: `cargo sqlx prepare --workspace -- --all-features`, `cargo make prepare-services`, `cargo make prepare-api`, and `cargo make generate-ts-types`.

- [ ] **Step 7: `cargo make check`.** Expected: clean.

- [ ] **Step 8: Commit.**

```bash
git add crates/temper-core crates/temper-services crates/temper-api/tests .sqlx crates/*/.sqlx packages/temper-ui
git commit -m "feat(access): grant_capability/revoke_capability service primitive

D3b §3.C: subject-polymorphic grant/revoke on kb_access_grants, gated by
is_system_admin OR can(...,'grant',...). Admin-event pattern (bind_team
precedent); shared wire types in temper-core. Surface verbs follow."
```

---

## Task 3: The Q-A flip + backfill migration (behavior-changing)

The core tightening. Flip and backfill co-commit atomically in one migration (backfill-first, then flip) so no author ever lacks their grant.

**Files:**
- Create: `migrations/20260701000001_cogmap_write_tightening.sql`
- Modify: `crates/temper-api/tests/cogmap_home_test.rs:181`, `:405`
- Test (backfill query + L0 exclusion): `crates/temper-api/tests/access_grants_test.rs` (extend)

**Interfaces:**
- Consumes: `profile_explicit_grant` (`20260630000001:50`), `kb_access_grants`, `kb_team_cogmaps`, `kb_teams.auto_join_role`, `kb_team_members`.
- Produces: `cogmap_authorable_by_profile(p, c) = profile_explicit_grant(p,'write','kb_cogmaps',c)`.

- [ ] **Step 1: Write the migration.**

```sql
-- Deliverable 3b — cogmap-write tightening (Q-A). Design:
-- docs/superpowers/specs/2026-07-01-deliverable-3b-cogmap-write-tightening-design.md
--
-- BEHAVIOR-CHANGING but not big-bang: a single forward CREATE OR REPLACE + a bounded one-time
-- snapshot. Ordered BACKFILL-FIRST then FLIP, committing atomically, so there is never a committed
-- state where a current author lacks their grant. Namespace-free (no SET search_path).

-- (1) BACKFILL FIRST — snapshot today's DELIBERATE flat authors as PER-PROFILE can_write grants so
-- #221's multi-author authoring survives the flip. Per-profile (a true snapshot; NO ongoing
-- membership-inheritance, which Q-A forbids). auto_join_role teams (temper-system → the L0 kernel)
-- are EXCLUDED: that membership is the universal "everyone" pool, so snapshotting it would grant the
-- whole userbase write to the operator-governed kernel. granted_by = the system profile.
INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id,
                              can_read, can_write, granted_by_profile_id)
SELECT DISTINCT 'kb_cogmaps', tc.cogmap_id, 'kb_profiles', tm.profile_id, true, true,
       (SELECT id FROM kb_profiles WHERE handle = 'system')
FROM kb_team_cogmaps tc
JOIN kb_teams t         ON t.id = tc.team_id
JOIN kb_team_members tm ON tm.team_id = tc.team_id
WHERE t.auto_join_role IS NULL
ON CONFLICT (subject_table, subject_id, principal_table, principal_id) DO NOTHING;

-- (2) FLIP — Q-A: cogmap authorship = explicit write grant only (no membership-implies-write).
-- Cogmaps have no owner column, so there is no ownership floor; authority is wholly explicit. Reads
-- stay membership-broad (cogmap_readable_by_profile, unchanged). derived_access_profile's cogmap/write
-- arm delegates here by name, so can(...,'write','kb_cogmaps',…) follows automatically.
CREATE OR REPLACE FUNCTION cogmap_authorable_by_profile(p_profile uuid, p_cogmap uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT profile_explicit_grant(p_profile, 'write', 'kb_cogmaps', p_cogmap);
$$;
```

- [ ] **Step 2: Clean the migrate macro cache.** Run: `cargo clean -p temper-api` (new migration ⇒ avoid the phantom "function does not exist" stale-cache failure).

- [ ] **Step 3: Flip the existing membership⇒authorable assertions.** In `cogmap_home_test.rs` at `:181` and `:405`, change the assertions from "a team member is authorable" to "a team member alone is NOT authorable; an explicit can_write grant IS." Read the surrounding test first; the transformation:

```rust
// BEFORE (membership conferred authoring):
// assert!(cogmap_authorable_by_profile(member, cogmap));

// AFTER: membership alone no longer authorizes (Q-A).
let member_authorable: bool = sqlx::query_scalar!(
    "SELECT cogmap_authorable_by_profile($1, $2)", member.uuid(), cogmap)
    .fetch_one(&pool).await.unwrap().unwrap();
assert!(!member_authorable, "membership alone must NOT confer authoring after Q-A");

// An explicit can_write grant DOES authorize.
sqlx::query!(
    r#"INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id,
                                     can_read, can_write, granted_by_profile_id)
       VALUES ('kb_cogmaps', $1, 'kb_profiles', $2, true, true, $2)"#,
    cogmap, member.uuid()).execute(&pool).await.unwrap();
let granted_authorable: bool = sqlx::query_scalar!(
    "SELECT cogmap_authorable_by_profile($1, $2)", member.uuid(), cogmap)
    .fetch_one(&pool).await.unwrap().unwrap();
assert!(granted_authorable, "an explicit can_write grant confers authoring");
```

- [ ] **Step 4: Add the backfill-query + L0-exclusion tests** to `access_grants_test.rs`:

```rust
// Backfill query logic (the migration reuses this exact SELECT): a member of a NON-auto-join team
// joined to a map is snapshotted; a member of an auto-join team is NOT.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn backfill_snapshots_real_members_not_auto_join(pool: PgPool) {
    let real_team = mint_team(&pool, /*auto_join_role*/ None).await;
    let member = mint_profile(&pool).await;
    add_member(&pool, real_team, member).await;
    let cogmap = mint_unbound_cogmap(&pool).await;
    bind_cogmap(&pool, cogmap, real_team).await;

    // Run the migration's backfill SELECT verbatim (mirrors 20260701000001 step 1).
    sqlx::query!(
        r#"INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id,
                                         can_read, can_write, granted_by_profile_id)
           SELECT DISTINCT 'kb_cogmaps', tc.cogmap_id, 'kb_profiles', tm.profile_id, true, true,
                  (SELECT id FROM kb_profiles WHERE handle = 'system')
           FROM kb_team_cogmaps tc
           JOIN kb_teams t ON t.id = tc.team_id
           JOIN kb_team_members tm ON tm.team_id = tc.team_id
           WHERE t.auto_join_role IS NULL
           ON CONFLICT (subject_table, subject_id, principal_table, principal_id) DO NOTHING"#,
    ).execute(&pool).await.unwrap();

    let authorable: bool = sqlx::query_scalar!(
        "SELECT cogmap_authorable_by_profile($1, $2)", member.uuid(), cogmap)
        .fetch_one(&pool).await.unwrap().unwrap();
    assert!(authorable, "a backfilled real-team member still authors");
}

// The L0 kernel (joined only to auto-join temper-system) has NO backfilled human write grant.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn l0_kernel_has_no_backfilled_write_grant(pool: PgPool) {
    let l0 = uuid::uuid!("00000000-0000-0000-0005-000000000001");
    let n: i64 = sqlx::query_scalar!(
        r#"SELECT count(*) AS "n!" FROM kb_access_grants
            WHERE subject_table = 'kb_cogmaps' AND subject_id = $1 AND can_write"#, l0)
        .fetch_one(&pool).await.unwrap();
    assert_eq!(n, 0, "no human gets write to the operator-governed kernel via backfill");
}
```

- [ ] **Step 5: Run the affected suites.** Run:
```
cargo make test-db 2>&1 | tail -20
cargo nextest run -p temper-api --features test-db --test cogmap_home_test --test access_grants_test
```
Expected: PASS. If any OTHER test authored into a cogmap via membership, it now fails — fix it to grant `can_write` first (using the pattern in Step 3). Note such fixes in the commit.

- [ ] **Step 6: Regenerate caches.** Run: `cargo sqlx prepare --workspace -- --all-features`, `cargo make prepare-api`.

- [ ] **Step 7: `cargo make check`.** Expected: clean.

- [ ] **Step 8: Commit.**

```bash
git add migrations crates/temper-api/tests .sqlx crates/temper-api/.sqlx
git commit -m "feat(access): Q-A cogmap-write flip + member backfill (behavior change)

D3b §3.A+§3.D: cogmap_authorable_by_profile -> explicit write grant only.
Co-committed per-profile backfill (excludes auto_join_role teams) preserves
current authors; L0 kernel excluded. Flips cogmap_home_test membership
assertions."
```

---

## Task 4: CLI — `temper cogmap grant` / `revoke`

Mirror the existing `bind`/`unbind` path (`commands/cogmap.rs:63-88`, `actions/cogmap.rs`, `cli.rs`, `main.rs`).

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (add `Grant`/`Revoke` variants to the `Cogmap` subcommand enum, mirroring `Bind`/`Unbind`)
- Modify: `crates/temper-cli/src/commands/cogmap.rs` (add `grant`/`revoke` fns, mirroring `bind`/`unbind` at `:63`)
- Modify: `crates/temper-cli/src/actions/cogmap.rs` (add `grant_api`/`revoke_api`, mirroring `bind_api` at `:86`)
- Modify: `crates/temper-cli/src/main.rs` (dispatch the new variants)
- Test: `tests/e2e` covers the CLI end-to-end (Task 7) — CLI unit surface here is the arg parse.

**Interfaces:**
- Consumes: `GrantCapabilityRequest`/`RevokeCapabilityRequest` (temper-core), the HTTP endpoints from Task 6 (via `temper-client`).
- CLI shape: `temper cogmap grant <cogmap_ref> --to-profile <ref> | --to-team <uuid> [--read] [--write] [--grant]`; `temper cogmap revoke <cogmap_ref> --from-profile <ref> | --from-team <uuid>`. Default when only `--write` given: also set `--read` (coherence). At least one capability required for grant.

- [ ] **Step 1: Write the failing arg-parse test.** In `cli.rs`'s test module (mirror the existing `cogmap_bind` parse test if present, else add one):

```rust
#[test]
fn cogmap_grant_parses_profile_write() {
    let cli = Cli::try_parse_from([
        "temper", "cogmap", "grant", "map-<uuid>", "--to-profile", "@alice", "--write",
    ]).unwrap();
    // assert the parsed Cogmap::Grant variant carries cogmap_ref, to_profile=Some, write=true
}
```

- [ ] **Step 2: Run to verify it fails.** Run: `cargo nextest run -p temper-cli cogmap_grant_parses`. Expected: FAIL — variant absent.

- [ ] **Step 3: Add the enum variants + command fns + action fns + dispatch.** Follow `bind` exactly. `grant`/`revoke` in `commands/cogmap.rs` parse the cogmap ref (`parse_ref`), resolve the principal (`--to-profile` via the profile resolver used elsewhere in actions; `--to-team` is a raw UUID like `bind`), build the request, and call `actions::cogmap::grant_api`/`revoke_api` inside `with_client`. `grant_api`/`revoke_api` POST/DELETE to the Task 6 endpoints via the client (mirror `bind_api`).

- [ ] **Step 4: Run to verify it passes.** Same as Step 2. Expected: PASS.

- [ ] **Step 5: Rebuild the CLI binary** (e2e stale-bin gotcha): `cargo build -p temper-cli --bin temper`.

- [ ] **Step 6: `cargo make check`.** Expected: clean.

- [ ] **Step 7: Commit.**

```bash
git add crates/temper-cli
git commit -m "feat(cli): temper cogmap grant/revoke (mirrors bind)"
```

---

## Task 5: MCP — `cogmap_grant` / `cogmap_revoke` tools

Mirror `cogmap_bind`/`cogmap_unbind` (`service.rs:286-308`, `tools/cognitive_maps.rs:202-283`).

**Files:**
- Modify: `crates/temper-mcp/src/tools/cognitive_maps.rs` (add `cogmap_grant`/`cogmap_revoke` fns + input structs, mirroring `cogmap_bind` at `:227` and its `CogmapBindInput` at `:205`)
- Modify: `crates/temper-mcp/src/service.rs` (register the two `#[tool]` wrappers, mirroring `:286`)
- Test: unit deserialize tests (mirror `cogmap_bind_input_deserializes` at `tools/cognitive_maps.rs:285`)

**Interfaces:**
- Consumes: `access_service::grant_capability`/`revoke_capability`, `GrantCapabilityRequest`, `parse_ref`, `svc.require_profile()`.
- Input structs (`#[derive(Debug, Deserialize, JsonSchema)]`): `CogmapGrantInput { cogmap: String, to_profile: Option<Uuid>, to_team: Option<Uuid>, read: bool, write: bool, grant: bool }`; `CogmapRevokeInput { cogmap: String, from_profile: Option<Uuid>, from_team: Option<Uuid> }`.

- [ ] **Step 1: Write the failing deserialize test.** In `tools/cognitive_maps.rs` test module:

```rust
#[test]
fn cogmap_grant_input_deserializes() {
    let id = Uuid::now_v7();
    let raw = serde_json::json!({ "cogmap": "m-<uuid>", "to_profile": id.to_string(), "write": true });
    let input: CogmapGrantInput = serde_json::from_value(raw).unwrap();
    assert_eq!(input.to_profile, Some(id));
    assert!(input.write);
}
```

- [ ] **Step 2: Run to verify it fails.** Run: `cargo nextest run -p temper-mcp cogmap_grant_input_deserializes`. Expected: FAIL.

- [ ] **Step 3: Implement the tools.** In `tools/cognitive_maps.rs`, `cogmap_grant`: `require_profile` → parse cogmap ref → resolve exactly one of `to_profile`/`to_team` (error if both/neither, like the create home check at `:328`) → build `GrantCapabilityRequest` (subject_table `"kb_cogmaps"`; `read` defaulted true when `write||grant`) → call `access_service::grant_capability(pool, profile_id, &req)` → map `ApiError::Forbidden` to an invalid-params "not authorized to administer grants on this map". `cogmap_revoke` mirrors with `revoke_capability`. Register both in `service.rs` via `#[tool]` wrappers delegating to these fns (copy the `cogmap_bind` wrapper at `:286`).

- [ ] **Step 4: Run to verify it passes.** Same as Step 2. Expected: PASS.

- [ ] **Step 5: `cargo make check`.** Expected: clean.

- [ ] **Step 6: Commit.**

```bash
git add crates/temper-mcp
git commit -m "mcp: cogmap_grant/cogmap_revoke tools (mirrors cogmap_bind)"
```

---

## Task 6: HTTP — `POST` / `DELETE /api/cognitive-maps/{id}/grants`

Mirror the cognitive-maps bind handler + route.

**Files:**
- Modify: `crates/temper-api/src/handlers/cognitive_maps.rs` (add `grant`/`revoke` handlers, mirroring the bind handler)
- Modify: `crates/temper-api/src/routes.rs` (add `POST`/`DELETE .../grants`)
- Modify: `crates/temper-api/src/openapi.rs` (register the two paths + the request schemas)
- Test: `crates/temper-api/tests/cognitive_map_handler_test.rs` (add grant/revoke handler cases; `#![cfg(feature = "test-db")]`)

**Interfaces:**
- Consumes: `access_service::grant_capability`/`revoke_capability`, the authenticated `ProfileId` from the JWT middleware (as the other handlers extract it), the path `{id}` (cogmap UUID), a JSON body deserialized into a **cogmap-scoped** request (subject fixed to the path's cogmap):
```rust
// Body types (temper-core, ts-rs): subject is the path {id}, so the body carries principal + caps only.
pub struct CogmapGrantBody { pub principal_table: String, pub principal_id: Uuid,
    pub can_read: bool, pub can_write: bool, pub can_delete: bool, pub can_grant: bool }
pub struct CogmapRevokeBody { pub principal_table: String, pub principal_id: Uuid }
```
- Produces: `POST` → `200 {granted: bool}`; `DELETE` → `204`. `ApiError::Forbidden` → `403`.

- [ ] **Step 1: Write the failing handler test.** In `cognitive_map_handler_test.rs`: admin grants a stranger `can_write` via `POST /api/cognitive-maps/{id}/grants`, assert `200` + the grant row exists + `can(...,'write',...)` true; a non-admin non-granter gets `403`.

- [ ] **Step 2: Run to verify it fails.** Run: `cargo nextest run -p temper-api --features test-db --test cognitive_map_handler_test grant`. Expected: FAIL — route absent (404).

- [ ] **Step 3: Implement handlers + routes + openapi.** Add `CogmapGrantBody`/`CogmapRevokeBody` to temper-core. The `grant` handler builds a `GrantCapabilityRequest { subject_table: "kb_cogmaps".into(), subject_id: path_id, ..body }` and calls `access_service::grant_capability(&state.pool, caller, &req)`; map the outcome to `Json(GrantOutcome)`; `Forbidden → 403`. `revoke` mirrors → `204`. Register routes in `routes.rs` and paths in `openapi.rs` next to the bind entries.

- [ ] **Step 4: Run to verify it passes.** Same as Step 2. Expected: PASS.

- [ ] **Step 5: Regenerate caches + TS types.** Run: `cargo sqlx prepare --workspace -- --all-features`, `cargo make prepare-api`, `cargo make generate-ts-types`.

- [ ] **Step 6: `cargo make check`.** Expected: clean.

- [ ] **Step 7: Commit.**

```bash
git add crates/temper-api crates/temper-core .sqlx crates/temper-api/.sqlx packages/temper-ui
git commit -m "feat(api): POST/DELETE /api/cognitive-maps/{id}/grants (mirrors bind)"
```

---

## Task 7: End-to-end scenarios (§5)

The access-semantics tier (#219's lesson: e2e catches hazards isolated-DB tests miss). Drives real CLI ↔ API ↔ DB with JWT auth.

**Files:**
- Create: `tests/e2e/tests/cogmap_write_grants_test.rs` (follow an existing e2e test's harness usage — `tests/e2e/tests/common/`)

**Interfaces:**
- Consumes: the e2e harness (spawns Axum + Postgres, mints JWTs, runs the built `temper` binary), the Task 4/5/6 surfaces.

- [ ] **Step 1: Write the e2e scenarios** (one test fn each, or table-driven):
  1. **Non-member gains write only via explicit grant:** a fresh non-member profile is denied authoring (403 from ingest into the map); an admin grants `can_write` through the **production caller** (`temper cogmap grant` or `POST .../grants`); the same profile now authors successfully; `temper cogmap revoke` removes it and authoring is denied again.
  2. **Creator authors their unbound map:** an admin creates a map (no bind), then authors a resource homed in it — succeeds via the creator seed (Task 1).
  3. **Backfilled member authors:** (if the harness seeds a non-auto-join team+cogmap+member before migrations — else assert via a grant that mirrors the backfill result) a real-team member authors after the migration.
  4. **Arbitrary user cannot author L0:** a non-admin, non-granted profile is denied authoring the `system-default` kernel (`00000000-0000-0000-0005-000000000001`).
  5. **Grant-admin axis:** the creator (`can_grant`) grants a co-author write; a profile holding only `can_write` (no `can_grant`) is `403` when it tries to grant further.

- [ ] **Step 2: Rebuild the CLI binary.** Run: `cargo build -p temper-cli --bin temper` (stale-bin gotcha).

- [ ] **Step 3: Run the e2e suite.** Run: `cargo make test-e2e 2>&1 | tail -30`. Expected: PASS. (Scenarios that exercise ingest embedding need `cargo make test-e2e-embed`; keep these grant/authz scenarios embed-free where possible.)

- [ ] **Step 4: Regenerate the e2e sqlx cache** if any macro query was added: `cargo make prepare-e2e`.

- [ ] **Step 5: `cargo make check`.** Expected: clean.

- [ ] **Step 6: Commit.**

```bash
git add tests/e2e
git commit -m "test(e2e): cogmap-write grant/revoke + creator + L0-exclusion scenarios"
```

---

## Final verification (branch-level, before PR)

- [ ] `cargo make check` — clean.
- [ ] `cargo make test-db` — green (unit + integration incl. the flipped `cogmap_home_test`).
- [ ] `cargo make test-artifacts` — green (substrate write-path).
- [ ] `cargo build -p temper-cli --bin temper` then `cargo make test-e2e` — green.
- [ ] `cargo make test-e2e-embed` — green (catches feature-unification surprises).
- [ ] Confirm no orphaned `.sqlx` files: `git status` after the full prepare ritual (`--workspace --all-features` → `prepare-services` → `prepare-api` → `prepare-e2e`).
- [ ] Open PR; regular merge (per-chunk history), not squash.

## Self-review notes (author)

- **Spec coverage:** §3.A→T3, §3.B→T1, §3.C→T2 (primitive) + T4/T5/T6 (surfaces), §3.D→T3, §3.E→T2 (`can_administer_grant`) + T7 scenario 5, §3.F→T3 (co-commit) + task ordering (additive T1/T2 before behavior-changing T3), §2 (no new capability fn)→respected (T3 is a one-line flip; grant authority uses `can()`).
- **Type consistency:** `GrantCapabilityRequest`/`RevokeCapabilityRequest`/`GrantOutcome` defined T2, consumed T4/T5/T6; `grant_capability`/`revoke_capability` signatures stable across tasks; HTTP uses cogmap-scoped `CogmapGrantBody` (subject from path) that the handler widens into `GrantCapabilityRequest`.
- **Ordering safety:** T1/T2 additive (green throughout); T3 is the only behavior change and flips its own breaking tests in-task; surfaces T4–T6 additive; T7 end-to-end last.
- **Open confirm:** the `xmax = 0` "was-inserted" trick in `grant_capability` (T2 Step 4) — if the reviewer prefers, replace with a pre-`SELECT`/`ON CONFLICT DO NOTHING` + `rows_affected()` check; the outcome semantics (`granted: fresh insert`) are what matters.
