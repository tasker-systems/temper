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
- **After any SQL change, regenerate the offline cache:** `cargo sqlx prepare --workspace -- --all-features`. Commit only `.sqlx` entries that belong to queries this branch actually adds; deleting an entry whose last caller you removed is correct, deleting one main still uses is not.
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
| `crates/temper-services/tests/context_read_predicate_test.rs` | **Modify.** Read-floor witnesses, one per admitting arm. |
| `crates/temper-api/tests/context_write_authority_test.rs` | **Modify.** Write-floor witness. |
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
- Modify: `crates/temper-services/tests/context_read_predicate_test.rs`
- Modify: `crates/temper-api/tests/context_write_authority_test.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `kb_contexts.is_active BOOLEAN NOT NULL DEFAULT true`; floored `contexts_readable_by_teams(uuid, uuid[])` and `context_authorable_by_profile(uuid, uuid)`. Both keep their existing signatures and return types exactly — no caller changes.

- [ ] **Step 1: Write the failing read-floor witnesses**

**EXTEND** — spec §2.2 authorizes the floor; §3 requires one isolated witness per admitting arm, because a caller with several reaches cannot tell you which arm closed.

Add to `crates/temper-services/tests/context_read_predicate_test.rs`, using the file's existing `Org` fixture and its `sqlx::query_scalar(...).bind(...)` runtime style (this file is `#![cfg(feature = "test-db")]` and deliberately uses runtime queries, so it needs no `.sqlx` entries):

```rust
/// Retiring a context closes EVERY admitting arm of `contexts_readable_by_teams`, and the
/// four arms are proved one at a time: a caller who reaches a context by two routes cannot
/// witness which one closed.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_retired_context_closes_each_read_arm_independently(pool: PgPool) {
    let org = Org::seed(&pool).await.expect("seed org");

    // ARM 1 — personal context, owner is the only reader.
    let personal = personal_context(&pool, org.dana, "dana-personal").await;
    assert!(readable(&pool, org.dana, personal).await, "owner reads it before retirement");
    retire(&pool, personal).await;
    assert!(!readable(&pool, org.dana, personal).await, "arm 1 closes: the owner loses it too");

    // ARM 2 — team-owned, read inherited UP the enclosure chain.
    let team_owned = team_context(&pool, org.engineering, "eng-owned").await;
    assert!(readable(&pool, org.dana, team_owned).await, "dana reaches it via enclosure");
    retire(&pool, team_owned).await;
    assert!(!readable(&pool, org.dana, team_owned).await, "arm 2 closes");

    // ARM 3 — shared into a team via kb_team_contexts. This arm never joins kb_contexts,
    // so it is the one an EXISTS-less floor would silently leave open.
    let shared = personal_context(&pool, org.outsider, "outsider-shared").await;
    share_to_team(&pool, shared, org.squad_two).await;
    assert!(readable(&pool, org.dana, shared).await, "dana reaches it via the share");
    retire(&pool, shared).await;
    assert!(!readable(&pool, org.dana, shared).await, "arm 3 closes");

    // ARM 4 — explicit read-grant. Also never joins kb_contexts.
    let granted = personal_context(&pool, org.outsider, "outsider-granted").await;
    grant_read(&pool, granted, org.dana).await;
    assert!(readable(&pool, org.dana, granted).await, "dana reaches it via the grant");
    retire(&pool, granted).await;
    assert!(!readable(&pool, org.dana, granted).await, "arm 4 closes");
}

/// The floor propagates to resource visibility — but only for reach the CONTAINER conferred.
/// A resource whose home row names you as owner stays visible, which is what keeps
/// retirement from being a data jail (spec §1.4).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn retirement_removes_container_conferred_reach_but_not_ownership(pool: PgPool) {
    let org = Org::seed(&pool).await.expect("seed org");

    let ctx = personal_context(&pool, org.outsider, "outsider-notes").await;
    let theirs = resource_homed_in(&pool, ctx, org.outsider).await;
    grant_read(&pool, ctx, org.dana).await;

    assert!(resource_visible(&pool, org.dana, theirs).await, "dana reads it through the context");
    assert!(resource_visible(&pool, org.outsider, theirs).await, "the owner reads their own");

    retire(&pool, ctx).await;

    assert!(!resource_visible(&pool, org.dana, theirs).await, "container-conferred reach is gone");
    assert!(
        resource_visible(&pool, org.outsider, theirs).await,
        "the owner arm of resources_visible_to is untouched — this is the anti-trap property"
    );
}
```

Add these helpers to the same file, matching its existing runtime-query style:

```rust
async fn personal_context(pool: &PgPool, owner: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name)
         VALUES (uuid_generate_v7(), 'kb_profiles', $1, $2, $2) RETURNING id",
    )
    .bind(owner).bind(slug).fetch_one(pool).await.expect("insert personal context")
}

async fn team_context(pool: &PgPool, team: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (id, owner_table, owner_id, slug, name)
         VALUES (uuid_generate_v7(), 'kb_teams', $1, $2, $2) RETURNING id",
    )
    .bind(team).bind(slug).fetch_one(pool).await.expect("insert team context")
}

async fn share_to_team(pool: &PgPool, context: Uuid, team: Uuid) {
    sqlx::query("INSERT INTO kb_team_contexts (team_id, context_id) VALUES ($1, $2)")
        .bind(team).bind(context).execute(pool).await.expect("share context");
}

async fn grant_read(pool: &PgPool, context: Uuid, profile: Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants
             (id, principal_table, principal_id, subject_table, subject_id, can_read)
         VALUES (uuid_generate_v7(), 'kb_profiles', $1, 'kb_contexts', $2, true)",
    )
    .bind(profile).bind(context).execute(pool).await.expect("grant read");
}

async fn retire(pool: &PgPool, context: Uuid) {
    sqlx::query("UPDATE kb_contexts SET is_active = false WHERE id = $1")
        .bind(context).execute(pool).await.expect("retire");
}

async fn readable(pool: &PgPool, profile: Uuid, context: Uuid) -> bool {
    sqlx::query_scalar("SELECT context_readable_by_profile($1, $2)")
        .bind(profile).bind(context).fetch_one(pool).await.expect("readable probe")
}

async fn resource_visible(pool: &PgPool, profile: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM resources_visible_to($1) v WHERE v.resource_id = $2)",
    )
    .bind(profile).bind(resource).fetch_one(pool).await.expect("resource visibility probe")
}
```

> ⚠️ **Plan/reality gap to close before writing these.** This file's fixture is a
> `struct Org` with a constructor the plan has not read in full, and `resource_homed_in`
> does not exist yet. Open `crates/temper-services/tests/context_read_predicate_test.rs`
> and match the real constructor name and the real way it seeds a resource + home row. If
> the fixture builds `Org` by a free function rather than `Org::seed`, use that. Do not
> invent a second seeding path beside the incumbent one.

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo nextest run -p temper-services --features test-db a_retired_context_closes_each_read_arm 2>&1 | tail -30
```

Expected: FAIL — `column "is_active" of relation "kb_contexts" does not exist`, from the `retire` helper.

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
cargo nextest run -p temper-services --features test-db a_retired_context_closes_each_read_arm 2>&1 | tail -30
cargo nextest run -p temper-services --features test-db retirement_removes_container 2>&1 | tail -30
```

Expected: both PASS.

If `db-migrate` fails on a checksum, the migration was edited after being applied — reset the Docker volume rather than amending the file.

- [ ] **Step 5: Write and run the write-floor witness**

**EXTEND** — spec §2.2.

Add to `crates/temper-api/tests/context_write_authority_test.rs`, matching that file's existing harness:

```rust
/// A retired context is frozen: no arm of `context_authorable_by_profile` admits, including
/// the explicit write-grant arm, which delegates to a subject-polymorphic helper that cannot
/// know a context is retired.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_retired_context_is_not_authorable_by_any_arm(pool: PgPool) {
    // Build one context per admitting arm: personal-owned, team-owned with an authoring
    // role, and one reached only by an explicit write-grant. Assert all three authorable,
    // retire all three, assert none authorable.
    // Probe: SELECT context_authorable_by_profile($1, $2)
}
```

> ⚠️ Replace the comment body with the real fixture calls from that file. The plan does not
> name them because it has not read the file; open it and follow its incumbent setup rather
> than seeding a fourth way.

```bash
cargo nextest run -p temper-api --features test-db a_retired_context_is_not_authorable 2>&1 | tail -30
```

Expected: PASS.

- [ ] **Step 6: Regenerate the sqlx cache and check**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make check 2>&1 | tail -40
```

- [ ] **Step 7: Commit**

```bash
git add migrations/ crates/temper-services/tests/ crates/temper-api/tests/ .sqlx/ crates/*/.sqlx/
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

Delete from `context_service.rs`: the `HomedResourceCount` struct, the two dependents queries, both `ApiError::Conflict` returns, and `map_context_delete_err`. Keep the `authorize::<ContextAdminAuthority>` call and the `fetch_one`-on-`EXISTS` existence check exactly as they are — the reasoning in that comment (the `SystemAdmin` arm admits without consulting the subject's existence) is correct and still applies.

The new body, after the gate and the existence check:

```rust
    // The mangled address, computed through the incumbent rather than a second uniqueness
    // rule. `next_unique_context_slug` is deliberately `is_active`-BLIND: retired rows keep
    // their slugs in the same UNIQUE space, so a floor added there would hand out an address
    // that collides with a retired row and fail at the INSERT. Do not "fix" it.
    let retired_slug =
        next_unique_context_slug(pool, &cur.owner_table, cur.owner_id, &format!("{}-retired", cur.slug))
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
```

Then build and return `RetireContextOutcome` from `cur` plus `retired_slug`, composing `context_ref` as `format!("{}/{retired_slug}", cur.owner_ref)` — the same composition `rename` uses, and for the same reason its comment gives: never through `decorated_context_ref`, whose parameter is the bare handle and would yield `@@handle/slug`.

> **Two things to verify on disk before writing this.** (a) `cur` is whatever the existing
> `delete` already fetched — confirm it carries `owner_table`, `owner_id`, `slug`, `name` and
> `owner_ref`; if it does not, widen that fetch rather than adding a second one. (b)
> `map_context_write_err` takes `anyhow::Error` (`:1104`), not `sqlx::Error` — the wrap above is
> required. Confirm the `23505` arm still renders `CONTEXT_SLUG_TAKEN`.

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

Wire both into the handlers: `list` gains an optional `retired: Option<bool>` query parameter and routes to `list_retired_administered` when true; `get` falls back to `get_retired_administered` when the read-axis lookup returns `NotFound`.

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

**CONFORM** — mirror the existing `share_team` POST method in the same file, not the `delete` method (which returns `()`; `restore` returns a body).

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
