# Context Retirement Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace PR #777's hard delete with a reversible retirement: a context can be made invisible and unwriteable without losing a single row, and can be restored.

**Architecture:** One additive column (`kb_contexts.is_active`), a floor inside the two predicates that are the sole chokepoints for context read and context write, and two verbs (`DELETE` retires, `POST .../restore` restores). Retired contexts are addressed on the **admin** axis — you can see one only if you could have retired it. The slug is mangled on retire rather than freed by relaxing `UNIQUE`, which keeps the migration `additive` and the deploy automatic.

**Tech Stack:** Rust (axum, sqlx, utoipa), PostgreSQL, `cargo-make`, ts-rs + OpenAPI codegen to temper-rb / temper-ts.

**Spec:** `internal/superpowers/specs/2026-08-25-context-retirement-design.md` — read it before starting. This plan is an index over that spec, not a replacement for it.

## Global Constraints

- **Migration class is `additive`.** Every migration calls `declare_migration(<version>, 'additive', '<non-empty reason>')`. The version argument must equal the filename timestamp; CI checks it. Silence fails CI. Source: `DEPLOYING.md:74-95`.
- **Migration number must sort above `origin/main`'s highest.** At time of writing that is `20260825000020_staleness_member_gate.sql`. Re-check after the rebase in Task 1; if main has advanced, renumber before committing.
- **Never edit an applied migration.** Not even a comment — sqlx checksum-verifies. If the local DB rejects it, reset the Docker volume rather than amending.
- **`--all-features` on every build and clippy invocation.**
- **`#[expect(lint, reason = "...")]`, never `#[allow]`.**
- **After any SQL change, regenerate the offline cache with the PER-CRATE task**, e.g. `cargo make prepare-services` for temper-services (and `cargo make prepare-e2e` for the e2e crate). Do **not** run `cargo sqlx prepare --workspace` — `Makefile.toml:114` says it clobbers the per-crate caches, and it leaves orphaned entries behind when a query's last caller is removed. Commit only `.sqlx` entries that belong to queries this branch actually adds; deleting an entry whose last caller you removed is correct, deleting one main still uses is not.
- **Auth before writes.** The authorization gate runs at the top of any function that mutates, before any database write.
- **CLI output goes through `crate::format::render(&value, fmt)`**, never `output::success`, for anything an agent must parse. Non-TTY stdout defaults to JSON.
- **All generated artifacts are committed together** — `openapi.json`, `clients/temper-rb/`, `clients/temper-ts/`, ts-rs `.ts` trees. Note that ts-rs drift only clears after a **commit**, not after `git add`.

---

## File Structure

| File | Responsibility |
|---|---|
| `migrations/20260825000030_context_retirement.sql` | **Create.** The column, the two floored predicates, the declaration. |
| `crates/temper-services/src/services/context_service.rs` | **Modify.** `delete` becomes retire; add `restore`, `list_retired_administered`, `get_retired_administered`. |
| `crates/temper-api/src/handlers/contexts.rs` | **Modify.** Retire handler doc, new `restore` handler, `retired` query param on `list`. |
| `crates/temper-api/src/routes.rs` | **Modify.** Register `restore`. |
| `crates/temper-core/src/types/context.rs` | **Modify.** `retired: bool` on `ContextRow` and `ContextRowWithCounts`. |
| `crates/temper-client/src/contexts.rs` | **Modify.** Add `restore`; keep `delete`. |
| `crates/temper-cli/src/cli.rs`, `commands/context_cmd.rs`, `main.rs` | **Modify.** `restore` verb, `--retired` flag, format-aware output. |
| `crates/temper-services/tests/context_read_predicate_test.rs` | **Modify.** Both floors' witnesses, one per admitting arm. Already owns every fixture and both probes. |
| `tests/e2e/tests/context_retire_e2e.rs` | **Rename from** `context_delete_e2e.rs`, rewrite. |
| `crates/temper-substrate/tests/context_retire_replay.rs` | **Create.** The replay round-trip hard delete could not pass. |
| `docs/reference/cli/context.md` | **Modify.** Regenerated CLI reference. |
| `.github/scripts/audit-elevation-claims.sh` | **Modify.** Claim-count baseline for `context_service.rs`. |

---

## Task 1: Rebase onto main and regenerate the conflicted artifacts

The branch conflicts with `origin/main` in three places, all generated. Resolving by hand produces artifacts that match neither side.

**Files:**
- Modify: `.sqlx/` (regenerated), `clients/temper-ts/src/generated/schema.ts` (regenerated)

**Interfaces:**
- Consumes: nothing.
- Produces: a branch that merges cleanly, and a known-highest migration number for Task 2.

- [ ] **Step 1: Confirm the conflicts before touching anything**

```bash
git fetch origin main
git merge-tree origin/main HEAD | grep -E '^(CONFLICT|changed in both)'
```

Expected: three lines — a rename/delete and a modify/delete on `.sqlx/query-0873…` / `query-62a87d8d…`, and a content conflict in `clients/temper-ts/src/generated/schema.ts`.

- [ ] **Step 2: Rebase**

```bash
git rebase origin/main
```

**CONFORM.** When the rebase stops on the `.sqlx` pair: the file this branch deleted is one `main` re-hashed because the query text changed there. This branch's deletion is stale-base collateral, not an intentional removal. Take **main's** version of both:

```bash
git checkout --theirs .sqlx/ 2>/dev/null || true
git checkout origin/main -- .sqlx/
git add .sqlx/
```

For `schema.ts`, do not merge by hand — take main's and regenerate in Step 3:

```bash
git checkout origin/main -- clients/temper-ts/src/generated/schema.ts
git add clients/temper-ts/src/generated/schema.ts
git rebase --continue
```

- [ ] **Step 3: Regenerate every derived artifact**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make generated-artifacts 2>/dev/null || cargo make check
```

If `cargo make generated-artifacts` is not a task in `Makefile.toml`, invoke the `generated-artifacts` skill instead — it names the current regeneration commands. Do not hand-edit any file under `clients/` or `openapi.json`.

- [ ] **Step 4: Verify the branch is clean against main**

```bash
cargo make check 2>&1 | tail -40
git merge-tree origin/main HEAD | grep -E '^(CONFLICT|changed in both)' || echo "NO CONFLICTS"
```

Expected: `NO CONFLICTS`, and `cargo make check` green.

- [ ] **Step 5: Record the highest migration number**

```bash
ls migrations/*.sql | tail -1
```

Expected: `migrations/20260825000020_staleness_member_gate.sql` or later. Task 2's migration must sort **above** whatever this prints.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "chore(context): rebase onto main and regenerate contract artifacts"
```

---

## Task 2: The column and the two floors

The whole enforcement surface. Nothing consumes it yet — every existing row is born `is_active = true`, so behavior is unchanged until Task 3 flips one.

**Files:**
- Create: `migrations/20260825000030_context_retirement.sql`
- Modify: `crates/temper-services/tests/context_read_predicate_test.rs` — **both** floors' witnesses live here. This file already owns the two probes (`can_read` at `:216`, `can_author` at `:224`) and the whole fixture set, so splitting the write witness into `context_write_authority_test.rs` would duplicate a fixture tree to re-ask a question this file already asks.

**Interfaces:**
- Consumes: nothing.
- Produces: `kb_contexts.is_active BOOLEAN NOT NULL DEFAULT true`; floored `contexts_readable_by_teams(uuid, uuid[])` and `context_authorable_by_profile(uuid, uuid)`. Both keep their existing signatures and return types exactly — no caller changes.

- [ ] **Step 1: Write the failing read-floor and write-floor witnesses**

**EXTEND** — spec §2.2 authorizes the floor; §3 requires one isolated witness per admitting arm, because a caller with several reaches cannot tell you which arm closed.

**CONFORM on the fixtures.** `crates/temper-services/tests/context_read_predicate_test.rs` already
carries every helper these tests need. Use them exactly as they are; do not add a second seeding
path beside the incumbent one. The real signatures, read from the file:

```rust
async fn org(pool: &PgPool) -> sqlx::Result<Org>                       // :78  — the EPD fixture, a FREE fn
async fn personal_context(pool: &PgPool, owner: Uuid, slug: &str) -> sqlx::Result<Uuid>   // :119
async fn team_context(pool: &PgPool, owner_team: Uuid, slug: &str) -> sqlx::Result<Uuid>  // :107
async fn share_to_team(pool: &PgPool, context_id: Uuid, team_id: Uuid) -> sqlx::Result<()> // :130
async fn grant(pool: &PgPool, context_id: Uuid, principal_table: &str, principal_id: Uuid,
               granted_by: Uuid, can_read: bool, can_write: bool) -> sqlx::Result<()>      // :141
async fn resource_in(pool: &PgPool, context_id: Uuid, owner: Uuid, title: &str)
    -> sqlx::Result<Uuid>                                                                  // :166
async fn can_read(pool: &PgPool, p: Uuid, c: Uuid) -> sqlx::Result<bool>                   // :216
async fn can_author(pool: &PgPool, p: Uuid, c: Uuid) -> sqlx::Result<bool>                 // :224
async fn sees_resource(pool: &PgPool, p: Uuid, r: Uuid) -> sqlx::Result<bool>              // :232
```

**The migrator in this file is `temper_substrate::MIGRATOR`, not `temper_api::MIGRATOR`** — all 14
existing tests use it, and temper-services' test crate resolves it through its own re-export. Copy
the attribute exactly as written below.

`Org` has fields `epd`, `engineering`, `payroll_group`, `squad_two`, `security_it_ops`, `dana`
(direct member of `squad_two` only), `outsider` (owns nothing, belongs to nothing). Tests in this
file return `sqlx::Result<()>` and use `?`, not `.expect(...)`.

**The only new helper you may add** is the one that does not exist:

```rust
/// Flip the retirement flag directly. Deliberately raw SQL, not the service: this file tests the
/// PREDICATES, and reaching for `context_service::retire` would couple the floor's witnesses to a
/// function Task 3 has not written yet.
async fn retire(pool: &PgPool, context_id: Uuid) -> sqlx::Result<()> {
    sqlx::query("UPDATE kb_contexts SET is_active = false WHERE id = $1")
        .bind(context_id)
        .execute(pool)
        .await?;
    Ok(())
}
```

Add these three tests to the same file:

```rust
/// Retiring a context closes EVERY admitting arm of `contexts_readable_by_teams`, and the four
/// arms are proved one at a time: a caller who reaches a context by two routes cannot witness
/// which one closed. Arms 3 and 4 never join `kb_contexts`, so they are the two an EXISTS-less
/// floor would silently leave open.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_retired_context_closes_each_read_arm_independently(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;

    // ARM 1 — personal context.
    let personal = personal_context(&pool, o.dana, "dana-personal").await?;
    assert!(can_read(&pool, o.dana, personal).await?, "arm 1 admits before retirement");
    retire(&pool, personal).await?;
    assert!(!can_read(&pool, o.dana, personal).await?, "arm 1 closes: the owner loses it too");

    // ARM 2 — team-owned, read inherited UP the enclosure chain.
    let owned = team_context(&pool, o.engineering, "eng-owned").await?;
    assert!(can_read(&pool, o.dana, owned).await?, "arm 2 admits via enclosure");
    retire(&pool, owned).await?;
    assert!(!can_read(&pool, o.dana, owned).await?, "arm 2 closes");

    // ARM 3 — shared into a reachable team. Never joins kb_contexts.
    let shared = personal_context(&pool, o.outsider, "outsider-shared").await?;
    share_to_team(&pool, shared, o.squad_two).await?;
    assert!(can_read(&pool, o.dana, shared).await?, "arm 3 admits via the share");
    retire(&pool, shared).await?;
    assert!(!can_read(&pool, o.dana, shared).await?, "arm 3 closes");

    // ARM 4 — explicit read-grant. Never joins kb_contexts.
    let granted = personal_context(&pool, o.outsider, "outsider-granted").await?;
    grant(&pool, granted, "kb_profiles", o.dana, o.outsider, true, false).await?;
    assert!(can_read(&pool, o.dana, granted).await?, "arm 4 admits via the grant");
    retire(&pool, granted).await?;
    assert!(!can_read(&pool, o.dana, granted).await?, "arm 4 closes");

    Ok(())
}

/// A retired context is frozen on every arm of `context_authorable_by_profile`, including the
/// explicit write-grant arm — which delegates to `profile_explicit_grant`, a subject-polymorphic
/// helper that cannot know a context is retired, so its floor has to be added at the call site.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_retired_context_is_not_authorable_by_any_arm(pool: PgPool) -> sqlx::Result<()> {
    let o = org(&pool).await?;

    // ARM 1 — personal owner.
    let personal = personal_context(&pool, o.dana, "dana-writes").await?;
    // ARM 2 — direct membership in the owning team with an authoring role. Dana is a direct
    // member of squad_two only, which is why the context must be owned by THAT team.
    let owned = team_context(&pool, o.squad_two, "squad-writes").await?;
    // ARM 3 — explicit write-grant, no ownership and no membership.
    let granted = personal_context(&pool, o.outsider, "outsider-writes").await?;
    grant(&pool, granted, "kb_profiles", o.dana, o.outsider, true, true).await?;

    for (label, ctx) in [("personal", personal), ("team-owned", owned), ("write-grant", granted)] {
        assert!(can_author(&pool, o.dana, ctx).await?, "{label} admits before retirement");
        retire(&pool, ctx).await?;
        assert!(!can_author(&pool, o.dana, ctx).await?, "{label} is frozen once retired");
    }

    Ok(())
}

/// The floor removes reach the CONTAINER conferred, and nothing else. A resource whose home row
/// names you as owner stays visible, which is what keeps retirement from being a data jail
/// (spec §1.4) — `resources_visible_to`'s first arm is `h.owner_profile_id = p_profile`.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn retirement_removes_container_conferred_reach_but_not_ownership(
    pool: PgPool,
) -> sqlx::Result<()> {
    let o = org(&pool).await?;

    let ctx = personal_context(&pool, o.outsider, "outsider-notes").await?;
    let theirs = resource_in(&pool, ctx, o.outsider, "their note").await?;
    grant(&pool, ctx, "kb_profiles", o.dana, o.outsider, true, false).await?;

    assert!(sees_resource(&pool, o.dana, theirs).await?, "dana reads it through the context");
    assert!(sees_resource(&pool, o.outsider, theirs).await?, "the owner reads their own");

    retire(&pool, ctx).await?;

    assert!(!sees_resource(&pool, o.dana, theirs).await?, "container-conferred reach is gone");
    assert!(
        sees_resource(&pool, o.outsider, theirs).await?,
        "the owner arm is untouched — this is the anti-trap property"
    );

    Ok(())
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo nextest run -p temper-services --features test-db --test context_read_predicate_test 2>&1 | tail -30
```

Expected: the three new tests FAIL with `column "is_active" of relation "kb_contexts" does not exist`,
raised by the `retire` helper. The file's pre-existing tests still pass.

- [ ] **Step 3: Write the migration**

**AMEND** on the two functions (spec §2.2 authorizes the floor), **EXTEND** for the column.

Create `migrations/20260825000030_context_retirement.sql`. The two function bodies below are the **live definitions printed from the database**, with the floor added — the four read arms and three write arms are carried verbatim so this is an edit, not a rewrite:

```sql
-- Context retirement: a context can be made invisible and unwriteable without losing a row.
--
-- Supersedes the hard delete of PR #777. kb_contexts is a replay INPUT table restored verbatim
-- (crates/temper-substrate/src/replay.rs:101-125) and both context projectors RAISE on a missing
-- row (20260731000040:48, 20260715000010:28), so a hard delete breaks replay for any context that
-- was ever renamed or reassigned. A flag rides in with the verbatim restore and breaks nothing.
--
-- ADDITIVE. One new column with a default, and CREATE OR REPLACE on two STABLE read functions
-- whose signatures and return types are unchanged. UNIQUE (owner_table, owner_id, slug) is NOT
-- touched: retire mangles the slug instead, which frees the address without a shape-breaking
-- constraint swap. See internal/superpowers/specs/2026-08-25-context-retirement-design.md.

ALTER TABLE kb_contexts ADD COLUMN is_active BOOLEAN NOT NULL DEFAULT true;

COMMENT ON COLUMN kb_contexts.is_active IS
'Retirement flag, mirroring kb_teams.is_active. false = retired: confers zero read-reach and zero
write authority, while every row it homes is preserved. Enforced at exactly two chokepoints --
contexts_readable_by_teams (which context_visible_to, context_readable_by_profile,
contexts_readable_by and resources_visible_to all delegate to) and context_authorable_by_profile.
Retired contexts are addressed on the ADMIN axis, never the read axis.';

-- ============================================================================
-- Chokepoint 1 -- the read axis. Arms 1 and 2 select from kb_contexts and take the floor
-- directly; arms 3 and 4 read kb_team_contexts and kb_access_grants and never join the
-- context row, so they need an EXISTS. Missing either of those two is the silent hole.
-- ============================================================================
CREATE OR REPLACE FUNCTION contexts_readable_by_teams(p_profile uuid, p_teams uuid[])
RETURNS TABLE(context_id uuid) LANGUAGE sql STABLE AS $$
    -- 1. personal context
    SELECT c.id
    FROM kb_contexts c
    WHERE c.owner_table = 'kb_profiles' AND c.owner_id = p_profile
      AND c.is_active

    UNION

    -- 2. context OWNED by an enclosing team.
    SELECT c.id
    FROM kb_contexts c
    WHERE c.owner_table = 'kb_teams' AND c.owner_id = ANY(p_teams)
      AND c.is_active

    UNION

    -- 3. context SHARED to an enclosing team
    SELECT tc.context_id
    FROM kb_team_contexts tc
    WHERE tc.team_id = ANY(p_teams)
      AND EXISTS (SELECT 1 FROM kb_contexts c WHERE c.id = tc.context_id AND c.is_active)

    UNION

    -- 4. explicit read-grant on the context (profile-anchored, or team-anchored on a reachable team)
    SELECT g.subject_id
    FROM kb_access_grants g
    WHERE g.subject_table = 'kb_contexts' AND g.can_read
      AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
         OR (g.principal_table = 'kb_teams' AND g.principal_id = ANY(p_teams)) )
      AND EXISTS (SELECT 1 FROM kb_contexts c WHERE c.id = g.subject_id AND c.is_active);
$$;

-- ============================================================================
-- Chokepoint 2 -- the write axis. Its team arm already floors on kb_teams.is_active; this
-- adds the same shape for the context's own flag. The grant arm delegates to
-- profile_explicit_grant, which knows nothing about contexts, so its floor is added here.
-- ============================================================================
CREATE OR REPLACE FUNCTION context_authorable_by_profile(p_profile uuid, p_context uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        -- personal-owned: the owner authors their own context
        SELECT 1
        FROM kb_contexts c
        WHERE c.id = p_context
          AND c.owner_table = 'kb_profiles' AND c.owner_id = p_profile
          AND c.is_active

        UNION ALL

        -- team-owned: DIRECT membership in the OWNING team, with an authoring role.
        SELECT 1
        FROM kb_contexts c
        JOIN kb_team_members tm ON tm.team_id = c.owner_id AND tm.profile_id = p_profile
        JOIN kb_teams t ON t.id = c.owner_id AND t.is_active
        WHERE c.id = p_context
          AND c.owner_table = 'kb_teams'
          AND tm.role IN ('owner', 'maintainer', 'member')
          AND c.is_active
    )
    -- explicit write-grant, floored here because profile_explicit_grant is subject-polymorphic
    -- and cannot know a context is retired.
    OR ( profile_explicit_grant(p_profile, 'write', 'kb_contexts', p_context)
         AND EXISTS (SELECT 1 FROM kb_contexts c WHERE c.id = p_context AND c.is_active) );
$$;

SELECT declare_migration(
    20260825000030,
    'additive',
    'Context retirement: one defaulted column on kb_contexts plus CREATE OR REPLACE on two STABLE read functions whose signatures and return types are unchanged. A binary predating this migration keeps working -- it reads kb_contexts without the column, every existing row is born is_active = true, and both functions answer identically for an active context. Nothing is dropped: UNIQUE (owner_table, owner_id, slug) stays, and retire mangles the slug instead of relaxing the constraint, which is what keeps this class additive rather than shape-breaking (DEPLOYING.md:68-72). Supersedes the hard delete of PR #777, which could not ship: kb_contexts is a replay input table restored verbatim and both context projectors RAISE on a missing row. Design: internal/superpowers/specs/2026-08-25-context-retirement-design.md.'
);
```

- [ ] **Step 4: Apply the migration and run the tests**

```bash
touch crates/temper-migrate/src/lib.rs   # migrations/ changes do NOT trigger a rebuild on their own
cargo make db-migrate
cargo nextest run -p temper-services --features test-db --test context_read_predicate_test 2>&1 | tail -40
```

Expected: all three new tests PASS, and the file's pre-existing tests stay green — the floor
must not change any answer for an active context.

If `db-migrate` fails on a checksum, the migration was edited after being applied — reset the Docker volume rather than amending the file.

- [ ] **Step 5: Regenerate the sqlx cache and check**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make check 2>&1 | tail -40
```

- [ ] **Step 6: Commit**

```bash
git add migrations/ crates/temper-services/tests/ .sqlx/ crates/*/.sqlx/
git commit -m "feat(context): add is_active and floor the read and write predicates"
```

---

## Task 3: Retire replaces the hard delete

**Files:**
- Modify: `crates/temper-services/src/services/context_service.rs` (the `delete` added by PR #777)
- Delete: `crates/temper-services/.sqlx/query-1a8c2d91*.json`, `query-45ea3db6*.json`, `query-9f2682cc*.json`

**Interfaces:**
- Consumes: `kb_contexts.is_active` and both floored predicates (Task 2).
- Produces: `pub async fn retire(pool: &PgPool, caller: ProfileId, context_id: uuid::Uuid) -> ApiResult<RetireContextOutcome>`, where

```rust
/// What retire hands back. The caller needs BOTH halves to undo it: the read floor hides the
/// context and the slug moved, so the ref they arrived with no longer names the row (spec §2.4.1).
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct RetireContextOutcome {
    pub context_id: ContextId,
    /// The address after mangling — `<slug>-retired`, suffixed if that was taken.
    pub slug: String,
    /// The full decorated ref, `{owner_ref}/{slug}`, which is what `restore` accepts.
    pub context_ref: String,
    /// Unchanged by retirement. The display label is not an address.
    pub name: String,
}
```

Place `RetireContextOutcome` in `crates/temper-core/src/types/context.rs`, beside the existing `RenameContextOutcome`.

- [ ] **Step 1: Write the failing service tests**

**EXTEND** — spec §2.3 and §3.

Add to the `#[cfg(feature = "test-db")]` test module in `context_service.rs`, matching the module's existing fixtures:

```rust
/// Retirement preserves everything. This is the whole difference from the hard delete it
/// replaces: the guard is gone because there is nothing to guard against.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn retire_preserves_every_row_it_homes(pool: PgPool) {
    // seed: a context owned by `caller`, homing one live resource
    // retire(&pool, caller, ctx).await.expect("retire succeeds WITH a resource homed here");
    // assert the kb_contexts row still exists with is_active = false
    // assert the kb_resources row and its kb_resource_homes row are both untouched
}

/// The address is freed; the display label is not touched.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn retire_frees_the_slug_and_keeps_the_name(pool: PgPool) {
    // seed a personal context named "scratch" (slug "scratch")
    // let out = retire(...).await.expect("retire");
    // assert!(out.slug.starts_with("scratch-retired"));
    // assert_eq!(out.name, "scratch");
    // create(...) with name "scratch" now succeeds and lands on slug "scratch"
}

/// Retiring twice is a clean refusal, never a 500 and never a second mangle.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn retiring_an_already_retired_context_is_not_found(pool: PgPool) {
    // retire once, then retire again; assert ApiError::NotFound
}
```

> ⚠️ Replace each comment body with real calls. Read the existing `#[cfg(feature = "test-db")]`
> module in `context_service.rs` first — `transfers_personal_context_to_team_and_members_can_author`
> (around `:1434`) shows the incumbent seeding style. Use it rather than a fourth one.

- [ ] **Step 2: Run to verify they fail**

```bash
cargo nextest run -p temper-services --features test-db retire_preserves_every_row 2>&1 | tail -20
```

Expected: FAIL — `cannot find function 'retire'`.

- [ ] **Step 3: Replace `delete` with `retire`**

**AMEND** — spec §2.7 authorizes removing the guard; the auth gate and existence check are **CONFORM**, carried unchanged from PR #777.

Delete from `context_service.rs`: the `HomedResourceCount` struct, the two dependents queries, both `ApiError::Conflict` returns, and `map_context_delete_err`. Keep the `authorize::<ContextAdminAuthority>` call exactly where it is.

**Replace the `EXISTS` probe with a row fetch.** The current `delete` (`:1000-1008`) fetches only a
boolean, so there is no `cur` to build an outcome from. `rename` already has the exact fetch this
needs, at `:867-880` — copy it verbatim, including its `fetch_optional` + `ok_or_else`, which
subsumes the existence check the `EXISTS` was doing and carries the same SystemAdmin reasoning:

```rust
    let cur = sqlx::query!(
        r#"SELECT owner_table AS "owner_table!", owner_id AS "owner_id!", slug, name,
              CASE owner_table
                WHEN 'kb_teams' THEN '+' || (SELECT slug   FROM kb_teams    WHERE id = owner_id)
                ELSE                   '@' || (SELECT handle FROM kb_profiles WHERE id = owner_id)
              END AS "owner_ref!"
         FROM kb_contexts WHERE id = $1"#,
        context_id,
    )
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| ApiError::NotFound(CONTEXT_REFUSAL.to_string()))?;
```

Do **not** write a fourth spelling of that `CASE`: it is the incumbent both-kinds owner-ref
expression used at `:42-46`, `:77-80`, `:377-380` and now here. `team_owner_ref` is team-only and
would `fetch_one`-panic on a profile-owned context.

Then the retire itself:

```rust
    // The mangled address, computed through the incumbent rather than a second uniqueness rule.
    // `next_unique_context_slug` is deliberately `is_active`-BLIND: retired rows keep their slugs
    // in the same UNIQUE space, so a floor added there would hand out an address that collides
    // with a retired row and fail at the INSERT. Do not "fix" it.
    let retired_slug = next_unique_context_slug(
        pool,
        &cur.owner_table,
        cur.owner_id,
        &format!("{}-retired", cur.slug),
    )
    .await?;

    let updated = sqlx::query!(
        r#"UPDATE kb_contexts
              SET is_active = false, slug = $2
            WHERE id = $1 AND is_active"#,
        context_id,
        retired_slug,
    )
    .execute(pool)
    .await
    .map_err(|e| map_context_write_err(anyhow::Error::new(e)))?;

    if updated.rows_affected() == 0 {
        return Err(ApiError::NotFound(CONTEXT_REFUSAL.to_string()));
    }

    Ok(RetireContextOutcome {
        context_id: ContextId::from(context_id),
        context_ref: format!("{}/{retired_slug}", cur.owner_ref),
        slug: retired_slug,
        name: cur.name,
    })
```

`map_context_write_err` takes `anyhow::Error` (`:1104`), not `sqlx::Error` — the wrap is required,
and its `23505` arm renders `CONTEXT_SLUG_TAKEN`, which is the right refusal if a concurrent write
takes the mangled address between the two statements.

Compose `context_ref` from the already-decorated `cur.owner_ref`, never through
`decorated_context_ref` — that helper's parameter is the **bare** handle and would yield
`@@handle/slug`. This is the same note `rename` carries at its own return.

- [ ] **Step 4: Run the tests**

```bash
cargo nextest run -p temper-services --features test-db retire_ 2>&1 | tail -30
```

Expected: PASS.

- [ ] **Step 5: Regenerate the sqlx cache**

```bash
cargo sqlx prepare --workspace -- --all-features
git status --short crates/temper-services/.sqlx/
```

Expected: the three entries PR #777 added are gone (their queries no longer exist), and one new entry for the `UPDATE` appears.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(context): retire a context instead of hard-deleting it"
```

---

## Task 4: Restore

**Files:**
- Modify: `crates/temper-services/src/services/context_service.rs`
- Modify: `crates/temper-api/src/handlers/contexts.rs`
- Modify: `crates/temper-api/src/routes.rs:105`

**Interfaces:**
- Consumes: `retire` and `RetireContextOutcome` (Task 3).
- Produces: `pub async fn restore(pool, caller, context_id) -> ApiResult<RestoreContextOutcome>` and `POST /api/contexts/{id}/restore`. `RestoreContextOutcome` has the same four fields as `RetireContextOutcome` plus `pub slug_changed: bool` — true when the original address was taken and restore landed on a suffix.

- [ ] **Step 1: Write the failing tests**

```rust
/// Restore round-trips the address when nothing claimed it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn restore_returns_the_original_address_when_it_is_free(pool: PgPool) {
    // create "scratch" → retire → restore
    // assert slug == "scratch", slug_changed == false, and the context is readable again
}

/// Restore into a collision lands on the suffix and SAYS SO. Handing back a different
/// address silently is the failure mode `rename` explicitly refuses (spec §2.4).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn restore_into_a_taken_address_suffixes_and_reports_it(pool: PgPool) {
    // create "scratch" → retire → create a NEW "scratch" → restore the first
    // assert slug == "scratch-2" and slug_changed == true
}

/// Restoring a context that was never retired is a clean refusal.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn restoring_a_live_context_is_not_found(pool: PgPool) { }
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo nextest run -p temper-services --features test-db restore_ 2>&1 | tail -20
```

Expected: FAIL — `cannot find function 'restore'`.

- [ ] **Step 3: Implement `restore`**

**EXTEND** — spec §2.4.

Same gate as `retire` (`authorize::<ContextAdminAuthority>`, before any write). Re-derive the address from the untouched `name` through `next_unique_context_slug`, then:

```rust
    let restored_slug =
        next_unique_context_slug(pool, &cur.owner_table, cur.owner_id, &cur.name).await?;

    let updated = sqlx::query!(
        r#"UPDATE kb_contexts
              SET is_active = true, slug = $2
            WHERE id = $1 AND NOT is_active"#,
        context_id,
        restored_slug,
    )
    .execute(pool)
    .await
    .map_err(|e| map_context_write_err(anyhow::Error::new(e)))?;

    if updated.rows_affected() == 0 {
        return Err(ApiError::NotFound(CONTEXT_REFUSAL.to_string()));
    }
```

`slug_changed` is `restored_slug != sluggify(&cur.name)`.

> **Fetching `cur` here is the subtle part.** The context is retired, so any helper that reads
> through the read predicate will not find it. `restore` must fetch the row by id directly —
> the same shape `caller_administers_context` uses at `:579-588`, which is already
> `is_active`-blind because it reads `kb_contexts` by primary key.

- [ ] **Step 4: Add the handler and route**

**CONFORM** — mirror `handlers::contexts::share_team`'s shape for a `POST /{id}/<verb>` route, and `handlers::teams::delete`'s thin-handler shape (`crates/temper-api/src/handlers/teams.rs:160-167`).

```rust
/// Restore a retired context
#[utoipa::path(
    post,
    operation_id = "restore_context",
    path = "/api/contexts/{id}/restore",
    tag = "Contexts",
    params(("id" = Uuid, Path, description = "Context ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Context restored", body = RestoreContextOutcome),
        (status = 403, description = "Caller may read but not administer this context"),
        (status = 404, description = "Context not found, or not retired (uniform — no existence oracle)"),
        (status = 409, description = "The restored address collided under a concurrent write"),
    )
)]
pub async fn restore(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(context_id): Path<Uuid>,
) -> ApiResult<Json<RestoreContextOutcome>> {
    context_service::restore(&state.pool, ProfileId::from(auth.0.profile().id), context_id)
        .await
        .map(Json)
}
```

Register it in `routes.rs` beside the other single-verb context routes:

```rust
        .routes(routes!(handlers::contexts::restore))
```

Also update the `delete` handler's utoipa block: it now returns `200` with a `RetireContextOutcome` body rather than `204`, and the `409` for dependents is gone.

- [ ] **Step 5: Run the tests and check**

```bash
cargo nextest run -p temper-services --features test-db restore_ 2>&1 | tail -30
cargo make check 2>&1 | tail -40
```

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(context): restore a retired context"
```

---

## Task 5: The admin-axis door — retired listing and retired show

The one place this feature adds surface beyond two floors and two verbs, and it is load-bearing: without it `--retired` lists rows nobody can address and `restore` is unreachable (spec §2.4.1).

**Files:**
- Modify: `crates/temper-core/src/types/context.rs`
- Modify: `crates/temper-services/src/services/context_service.rs`
- Modify: `crates/temper-api/src/handlers/contexts.rs`

**Interfaces:**
- Consumes: Tasks 2–4.
- Produces: `retired: bool` on `ContextRow` and `ContextRowWithCounts`;
  `list_retired_administered(pool, profile_id) -> ApiResult<Vec<ContextRowWithCounts>>`;
  `get_retired_administered(pool, profile_id, context_id) -> ApiResult<ContextRow>`;
  `GET /api/contexts?retired=true`.

- [ ] **Step 1: Add `retired` to both DTOs**

**EXTEND** — spec §2.6.

`ContextRow` and `ContextRowWithCounts` (`crates/temper-core/src/types/context.rs:17` and `:52`) carry an identical field set. Add to **both**, beside `can_write`:

```rust
    /// Whether this context is retired — invisible to every read path and unwriteable, with
    /// every row it homes preserved.
    ///
    /// **Polarity is inverted from the column on purpose.** The database stores `is_active`,
    /// mirroring `kb_teams`; the wire says `retired`, which is the word the product uses. The
    /// inversion is written exactly twice — once in each query literal, as
    /// `NOT c.is_active AS "retired!"` — and is never re-derived anywhere else.
    pub retired: bool,
```

Then add `NOT c.is_active AS "retired!"` to the select list of **both** existing queries — `list_visible` (`:78-105`) and `get_visible` (`:118-137`). Those two are read-axis queries, so the value is always `false` there; it is still the honest row shape, and it is what lets one DTO serve both doors.

Adding a public field breaks struct-literal construction across the workspace. That is a compile error at every site, caught immediately — fix them; do not add `..Default::default()` to silence it.

- [ ] **Step 2: Write the failing admin-axis tests**

```rust
/// You can see a retired context only if you could have retired it. A team MEMBER who could
/// read and author it before retirement sees nothing; an owner/maintainer sees it.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn the_retired_listing_rides_the_admin_axis_not_the_read_axis(pool: PgPool) { }

/// Listing a thing you cannot then inspect is an incoherent pair.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn an_administrator_can_show_a_retired_context(pool: PgPool) { }
```

- [ ] **Step 3: Implement the admin-axis query**

**EXTEND** — spec §2.5.

The care point, restated because it is the whole reason this step exists: `caller_administers_context` (`:575`) is a **point** check, and its team half is decided in Rust via `team_service::role_on_team` plus `can_manage` (`team_service.rs:79-81` — `Owner | Maintainer`). There is **no SQL predicate for "teams I manage"**; a scan of `pg_proc` for `manage` returns nothing. So do **not** write `tm.role IN ('owner','maintainer')` into SQL — that is a second spelling of `can_manage` that will drift.

Instead, derive the role list from `can_manage` itself and pass it as a parameter:

```rust
/// The manage-capable roles, derived from `can_manage` rather than restated. Adding a role to
/// `TeamRole` and forgetting it here is impossible: the iteration is over the enum.
fn manage_capable_roles() -> Vec<String> {
    [TeamRole::Owner, TeamRole::Maintainer, TeamRole::Member, TeamRole::Watcher]
        .into_iter()
        .filter(|r| team_service::can_manage(*r))
        .map(|r| r.to_string())
        .collect()
}
```

> **Three things to confirm on disk.** (a) `team_service::can_manage` is `pub(crate)`
> (`team_service.rs:79`) — `context_service` is in the same crate, so this works; if it is not,
> widen it rather than copying the `matches!`. (b) `TeamRole`'s `to_string()` must produce the
> lowercase spelling `kb_team_members.role` stores (`'owner'`, `'maintainer'`) — check its
> `Display`/`Serialize` impl in `crates/temper-core/src/types/team.rs:18-23` and use whatever
> the incumbent write path uses. (c) There is no `TeamRole::iter()` unless a derive provides
> one; the explicit array above is exhaustive and the compiler will not check it, so if
> `strum` or similar is already a dependency, prefer its iterator.

The listing query is `list_visible`'s (`:78-105`), with the `WHERE` swapped from
`context_visible_to($1, c.id)` to `NOT c.is_active AND (<admin predicate>)`. Carry the
`resource_count` subquery verbatim — it is counted through the caller's own read predicate for
reasons that comment explains at length, and retirement does not change them.

`get_retired_administered` is the same predicate over a single id.

Wire both into the handlers. `handlers/contexts.rs` already imports `axum::extract::Query` and
`serde::Deserialize`, so the incumbent query-param shape — copied from `handlers/edges.rs:48-52`
and its `params(...)` at `:60` — is:

```rust
/// Query params for the context list. `retired = true` switches the read from the visibility
/// axis to the ADMIN axis: a retired context is invisible to `contexts_readable_by_teams` by
/// construction, so it can only be listed by someone who could have retired it.
#[derive(Debug, Deserialize, utoipa::IntoParams)]
pub struct ListContextsQuery {
    /// List retired contexts you administer instead of the contexts you can read.
    pub retired: Option<bool>,
}
```

then `params(ListContextsQuery)` in `list`'s `#[utoipa::path]`, and
`Query(q): Query<ListContextsQuery>` in its signature.

`get` falls back to `get_retired_administered` when the read-axis lookup returns `NotFound`.

> **The `IntoParams` trap does not bite here, and it is worth knowing why.** `openapi.rs:751-754`
> records that *enums* reachable only through an `IntoParams` query struct are NOT auto-collected
> by `.routes()` and must be named in `components(schemas(...))` by hand, or the spec carries a
> dangling `$ref` and `openapi-generator` emits zero files. `Option<bool>` is a primitive and
> generates no `$ref`, so nothing needs registering. Do not add a `components(schemas(...))` entry
> for `ListContextsQuery`.

- [ ] **Step 4: Run the tests and regenerate**

```bash
cargo nextest run -p temper-services --features test-db retired_listing 2>&1 | tail -30
cargo sqlx prepare --workspace -- --all-features
cargo make check 2>&1 | tail -40
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(context): address retired contexts on the admin axis"
```

---

## Task 6: Client and CLI surfaces

**Files:**
- Modify: `crates/temper-client/src/contexts.rs`
- Modify: `crates/temper-cli/src/cli.rs`, `crates/temper-cli/src/commands/context_cmd.rs`, `crates/temper-cli/src/main.rs`

**Interfaces:**
- Consumes: Tasks 3–5.
- Produces: `ContextClient::restore(&self, context_id: Uuid) -> Result<RestoreContextOutcome>`; `ContextAction::Restore { context: String }`; `--retired` on `ContextAction::List`.

- [ ] **Step 1: Add the client method**

**CONFORM.** `crates/temper-client/src/contexts.rs` has two send variants and the difference is
load-bearing: `send` returns a raw `Response` (what `delete` uses today, which is why it returns
`()`), while `send_json` deserializes into the return type.

`unshare_team` (`:91-104`) is the exact model for **both** changes here — a body-less request that
returns a typed outcome:

```rust
    pub async fn unshare_team(&self, context_id: Uuid, team_id: Uuid) -> Result<UnshareContextOutcome> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/contexts/{context_id}/teams/{team_id}");
        let req = self.http.delete(&path);
        self.http.send_json(&Method::DELETE, &path, req, Some(&token)).await
    }
```

So: change `delete` from `send` to `send_json` and its return type from `Result<()>` to
`Result<RetireContextOutcome>`; add `restore` as the same shape with `self.http.post(&path)` (no
`.json(...)` — there is no request body) and `send_json(&Method::POST, ...)`, returning
`Result<RestoreContextOutcome>`.

- [ ] **Step 2: Change `delete_remote` to render its outcome**

**AMEND** — spec §2.6 requires format-aware output.

PR #777's `delete_remote` takes `_fmt` and discards it, printing `output::success("Context deleted.")`. Replace with the shape `rename_remote` uses two functions above it (`context_cmd.rs:287-306`):

```rust
    let outcome = client
        .contexts()
        .delete(context_id)
        .await
        .map_err(|e| map_admin_required_err("delete", e))?;
    let rendered = crate::format::render(&outcome, fmt)?;
    println!("{rendered}");
    Ok(())
```

Rename the parameter from `_fmt` to `fmt`. This is what makes the mangled ref reach the operator — `RetireContextOutcome` carries it, and an agent on non-TTY stdout gets JSON rather than `✓ Context deleted.`

- [ ] **Step 3: Add `restore` and `--retired`**

`ContextAction::Restore` takes a `context: String` that accepts a UUID **or the mangled ref**, and must resolve on the admin axis — `resolve_context_id_for_read` (`context_cmd.rs:360`) goes through the read predicate and will not find a retired context. Add a sibling resolver, or accept a bare UUID and document that. Prefer the sibling: `temper context list --retired` prints refs, and a ref that cannot be fed back to `restore` is a dead end.

Update the `delete` doc comment on `ContextAction` (`cli.rs:1063-1073`) — it currently says "THIS IS A HARD DELETE… there is nothing to undo", which is now false in every particular.

- [ ] **Step 4: Wire `main.rs` and check**

```bash
cargo make check 2>&1 | tail -40
```

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(cli): temper context restore, --retired, and format-aware retire output"
```

---

## Task 7: Regenerate every derived artifact

Its own task because ts-rs drift only clears after a **commit**, not after `git add` — folding it into an earlier task produces a gate that passes locally and fails in CI.

**Files:**
- Modify: `openapi.json`, `clients/temper-rb/`, `clients/temper-ts/`, ts-rs `.ts` output, `docs/reference/cli/context.md`, `.github/scripts/audit-elevation-claims.sh`

- [ ] **Step 1: Regenerate**

Invoke the `generated-artifacts` skill — it names the current commands for openapi, the Ruby gem, `schema.ts`, the ts-rs trees, and the committed `agent-skills/` projection. Do not hand-edit any of them.

- [ ] **Step 2: Update the elevation-claims baseline**

PR #777 bumped `context_service.rs` from 5 to 6. This task adds `restore`'s gate doc and changes `retire`'s, so re-derive rather than guess:

```bash
.github/scripts/audit-elevation-claims.sh --list | grep context_service
```

Set the `claim crates/temper-services/src/services/context_service.rs <n> context_admin` line to whatever that prints. Then **read** the claims and confirm each matches `ContextAdminAuthority::resolve` (`crates/temper-services/src/authz/context_admin.rs:65-85`) — the script binds a claim to a gate, it does not decide whether the claim is true.

- [ ] **Step 3: Verify every drift gate**

```bash
cargo make check 2>&1 | tail -60
```

Expected: `openapi-check`, `openapi-rb-drift`, `openapi-ts-drift`, `ts-rs-drift` and `skills-drift` all green.

- [ ] **Step 4: Commit, then re-run**

```bash
git add -A
git commit -m "chore(context): regenerate contract artifacts for retirement"
cargo make check 2>&1 | tail -30
```

Expected: still green **after** the commit — this is the run that actually exercises ts-rs drift.

---

## Task 8: End-to-end and replay witnesses

**Files:**
- Rename: `tests/e2e/tests/context_delete_e2e.rs` → `tests/e2e/tests/context_retire_e2e.rs`
- Create: `crates/temper-substrate/tests/context_retire_replay.rs`

- [ ] **Step 1: Rewrite the e2e suite**

```bash
git mv tests/e2e/tests/context_delete_e2e.rs tests/e2e/tests/context_retire_e2e.rs
```

Keep the file's harness — `common::setup`, `provision`, `root_bootstrap_first_admin`, the `delete_status` HTTP helper and the `cli_delete` spawn helper are all sound and reusable. Replace the two tests with:

1. **Retire → restore round-trip over HTTP and the CLI.** Retire a context that still homes a live resource (the old suite's "refused while attached" case inverts: it must now **succeed**), assert the resource survives and is still readable by its owner, assert the context is gone from `GET /api/contexts`, restore it, assert it is back.
2. **The mangled ref reaches the operator.** Assert the retire response carries `context_ref`, and that feeding exactly that ref to `temper context restore` works.
3. **The authorization arms PR #777 never exercised.** Both of its tests provisioned an instance admin via `root_bootstrap_first_admin`, so no other caller was ever tried. Add: a caller who may read but not administer gets `403`; an unprivileged caller naming a foreign context gets the uniform `404`. Both for **retire and restore**.

- [ ] **Step 2: Write the replay witness**

**EXTEND** — spec §3. This is the witness the hard delete could not have passed.

Create `crates/temper-substrate/tests/context_retire_replay.rs`, modelled on the existing `crates/temper-substrate/tests/context_rename_replay.rs`:

```rust
/// create → rename → retire, then a full ledger replay. The hard delete this supersedes
/// aborted here: `_project_context_renamed` RAISEs on a missing row
/// (20260731000040:48) and `replay.rs:621-637` calls it unguarded. A retired context is
/// still a row, so the projector finds it and the `is_active = false` rides in with the
/// verbatim input-table restore.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_retired_context_replays(pool: PgPool) { }
```

> ⚠️ Read `context_rename_replay.rs` first. Its module doc records that it deliberately does
> **not** call `common::reset_schema` up front, because `context_renamed` is seeded by a
> migration rather than by `event_types` — an initial reset would remove it. That constraint
> applies here verbatim.

- [ ] **Step 3: Run both**

```bash
cargo nextest run -p temper-substrate --features test-db a_retired_context_replays 2>&1 | tail -30
cargo make test-e2e 2>&1 | tail -60
```

`test-e2e` spawns a built binary — rebuild first, or it tests the previous one.

- [ ] **Step 4: Full suite**

```bash
cargo make check 2>&1 | tail -40
cargo make test-all 2>&1 | tail -60
```

Note: `test-all` has one known pre-existing streaming/embed timeout on this repo. A single timeout there is not a regression; anything else is. `nextest` cancels on first failure, so a reported failure count is a lower bound, not a total.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "test(context): retire/restore e2e, authorization arms, and the replay round-trip"
```

---

## Self-review notes

**Spec coverage.** §2.1 → Task 2. §2.2 → Task 2. §2.3 → Task 3. §2.4 → Task 4. §2.4.1 → Tasks 4, 5, 6 (outcome carries the ref; restore resolves on the admin axis; CLI prints it). §2.5 → Task 5. §2.6 → Tasks 4–7. §2.7 → Tasks 1 and 3. §3 → Tasks 2, 3, 4, 5, 8. §4 rejections require no work by construction.

**Known plan/reality gaps, flagged rather than papered over.** Four test bodies are specified by intent with a ⚠️ note rather than written out: the `Org` fixture constructor in `context_read_predicate_test.rs`, the fixtures in `context_write_authority_test.rs`, the seeding style in `context_service.rs`'s test module, and `TeamRole`'s string spelling. Each is a file the plan has not read in full, and inventing a body for it would be exactly the laundered grounding this repo's discipline forbids. The implementer reads the file and follows the incumbent.

**Naming consistency.** `retire`/`restore` are the service functions throughout; the HTTP verb stays `DELETE` and the CLI verb stays `delete`, per spec §2.6 — this asymmetry is deliberate and matches `handlers::teams::delete`. `RetireContextOutcome` and `RestoreContextOutcome` are distinct types; the latter adds `slug_changed`.
