# Context Transfer Safety + Residual-Access Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the deferred safety/completeness items around context ownership transfer — demote `originator_profile_id` from access, and surface (without sweeping) the read-reach a transfer inherits.

**Architecture:** Two independent PRs. **PR1** narrows two SQL access predicates so `owner_profile_id` alone confers access (`originator_profile_id` becomes pure provenance) — additive `CREATE OR REPLACE`, behavior-preserving on all current data. **PR2** extends the transfer outcome DTO with the inherited shares + context read-grants the new owner just acquired, gathered by a new service read; both CLI and MCP already serialize the whole outcome, so no rendering code changes.

**Tech Stack:** Rust workspace (temper-substrate SQL + tests, temper-core DTOs, temper-services), sqlx (Postgres 18/pgvector, port 5437), cargo-make, cargo-nextest, ts-rs + utoipa + schemars codegen.

**Spec:** [2026-07-15-context-transfer-safety-residual-access-design.md](../specs/2026-07-15-context-transfer-safety-residual-access-design.md)

## Global Constraints

- **Migrations are additive-only on `main`** — `CREATE OR REPLACE FUNCTION` only; no table/column changes. Never edit an applied migration (sqlx checksum-locked).
- **Migration version** must sort after `20260715000010`; use `20260715000020`.
- **DTO field additions are additive + skew-safe** — new fields carry `#[serde(default)]` so an older client omitting them deserializes cleanly.
- **A new/changed response DTO restales three artifacts** — `openapi.json`, the temper-rb gem, and temper-ts `schema.ts`. Regen via `cargo make openapi` (+ `cargo make generate-ts-types` for ts-rs). The drift gates compare against **git**, so **stage the regenerated files** before `cargo make check`.
- **DATABASE_URL** for bare `cargo`/`#[sqlx::test]`: `postgresql://temper:temper@localhost:5437/temper_development` (the `cargo make` tasks set it for you).
- **Run `cargo make check` before every commit.** SQL-macro query changes need `cargo sqlx prepare --workspace -- --all-features`.

---

## PR1 — D1: Demote `originator_profile_id` from access

### Task 1: Narrow `resources_visible_to` + `can_modify_resource` to owner-only

Remove the `originator_profile_id` arm from both access predicates. Access becomes
`owner_profile_id` + explicit grants + container cascade; `originator_profile_id` stays a
recorded provenance fact. Verified safe: `owner_profile_id` is `NOT NULL` (never orphans) and
owner↔originator diverge in **0 rows** today (behavior-preserving on current data). The
behavioral change is only visible after a `resource_reassign` splits the two.

**Files:**
- Create: `migrations/20260715000020_demote_originator_from_access.sql`
- Create: `crates/temper-substrate/tests/originator_demotion.rs`

**Interfaces:**
- Consumes: existing SQL functions `resources_visible_to(uuid) RETURNS TABLE(resource_id uuid)`, `can_modify_resource(uuid, uuid) RETURNS boolean`; test helpers copied from `crates/temper-substrate/tests/container_write_cascade.rs` (`insert_profile`, `insert_resource`, `insert_context`, `home_resource`, `can_modify`).
- Produces: no Rust API surface — this task changes SQL behavior only.

- [ ] **Step 1: Guard check — find any test asserting originator-grants-access**

Run:
```bash
cd /Users/petetaylor/projects/tasker-systems/temper
grep -rniE 'originator.*(visible|can_modify|can_read|access|read|write)' crates/*/tests/ tests/ --include='*.rs'
```
Expected: no test that asserts an originator (who is *not* the owner) can see or modify a resource. If one exists, it encodes the old behavior — update it to assert the new behavior as part of this task and note it in the commit. (Divergence is 0 in real data, so none is expected.)

- [ ] **Step 2: Write the failing predicate test**

Create `crates/temper-substrate/tests/originator_demotion.rs`. The helpers mirror `container_write_cascade.rs` (same crate's test dir):

```rust
#![cfg(feature = "test-db")]

//! D1 — `originator_profile_id` confers NO access; `owner_profile_id` is the access-bearing
//! profile key. Provenance ≠ access. See the context-transfer-safety spec.

use uuid::Uuid;

async fn insert_profile(pool: &sqlx::PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_profiles (handle, display_name, system_access) \
         VALUES ($1, $1, false) RETURNING id",
    )
    .bind(handle)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn insert_resource(pool: &sqlx::PgPool, title: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_resources (title, origin_uri) VALUES ($1, $1) RETURNING id")
        .bind(title)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn insert_context(pool: &sqlx::PgPool, owner: Uuid, slug: &str) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_contexts (owner_table, owner_id, slug, name) \
         VALUES ('kb_profiles', $1, $2, $2) RETURNING id",
    )
    .bind(owner)
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap()
}

async fn home_resource(pool: &sqlx::PgPool, resource: Uuid, context: Uuid, originator: Uuid, owner: Uuid) {
    sqlx::query(
        "INSERT INTO kb_resource_homes \
             (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id) \
         VALUES ($1, 'kb_contexts', $2, $3, $4)",
    )
    .bind(resource)
    .bind(context)
    .bind(originator)
    .bind(owner)
    .execute(pool)
    .await
    .unwrap();
}

async fn can_read(pool: &sqlx::PgPool, profile: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM resources_visible_to($1) WHERE resource_id = $2)")
        .bind(profile)
        .bind(resource)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn can_modify(pool: &sqlx::PgPool, profile: Uuid, resource: Uuid) -> bool {
    sqlx::query_scalar("SELECT can_modify_resource($1, $2)")
        .bind(profile)
        .bind(resource)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// Behavior-preserving: a creator (owner == originator) still reads + modifies their resource.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn owner_that_is_also_originator_keeps_access(pool: sqlx::PgPool) {
    let alice = insert_profile(&pool, "alice").await;
    let ctx = insert_context(&pool, alice, "alice-ctx").await;
    let res = insert_resource(&pool, "doc").await;
    home_resource(&pool, res, ctx, alice, alice).await; // originator == owner == alice

    assert!(can_read(&pool, alice, res).await, "creator reads their own resource");
    assert!(can_modify(&pool, alice, res).await, "creator modifies their own resource");
}

/// The D1 change: when owner and originator DIVERGE, only the OWNER has access — the
/// originator (former creator, now handed off) is cut off on both axes.
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn originator_without_ownership_has_no_access(pool: sqlx::PgPool) {
    let alice = insert_profile(&pool, "alice").await; // originator (former owner)
    let bob = insert_profile(&pool, "bob").await; // current owner (post-handoff)
    let ctx = insert_context(&pool, bob, "bob-ctx").await;
    let res = insert_resource(&pool, "doc").await;
    home_resource(&pool, res, ctx, alice, bob).await; // originator=alice, owner=bob

    assert!(can_read(&pool, bob, res).await, "owner reads");
    assert!(can_modify(&pool, bob, res).await, "owner modifies");
    assert!(!can_read(&pool, alice, res).await, "bare originator does NOT read");
    assert!(!can_modify(&pool, alice, res).await, "bare originator does NOT modify");
}
```

- [ ] **Step 3: Run the test to confirm the divergence case fails**

Run:
```bash
cargo nextest run -p temper-substrate --features test-db --test originator_demotion
```
Expected: `owner_that_is_also_originator_keeps_access` PASSES; `originator_without_ownership_has_no_access` **FAILS** on the first `assert!(!can_read(... alice ...))` — today the originator arm grants access.

- [ ] **Step 4: Write the migration**

Create `migrations/20260715000020_demote_originator_from_access.sql`. Reproduces both function bodies verbatim from the current live definitions with the single `originator_profile_id` arm removed from each (comments updated):

```sql
-- D1 — demote originator_profile_id from access. owner_profile_id is the single access-bearing
-- profile key; originator_profile_id becomes pure recorded provenance. Additive: CREATE OR
-- REPLACE only. Behavior-preserving on current data (owner is NOT NULL; owner<>originator
-- diverge in 0 rows). See docs/superpowers/specs/2026-07-15-context-transfer-safety-residual-access-design.md.

CREATE OR REPLACE FUNCTION public.resources_visible_to(p_profile uuid)
 RETURNS TABLE(resource_id uuid)
 LANGUAGE sql
 STABLE
AS $function$
    SELECT v.resource_id
    FROM (
        WITH reachable_teams AS (
            SELECT DISTINCT a.team_id
            FROM profile_effective_teams(p_profile) e
            CROSS JOIN LATERAL team_ancestors(e.team_id) a
        )
        -- owned (the home confers access to its OWNER; originator is provenance only, not access)
        SELECT h.resource_id FROM kb_resource_homes h
         WHERE h.owner_profile_id = p_profile
        UNION
        -- direct profile-anchored grant (consumer-axis ONLY — never enters a vis(T))
        SELECT g.subject_id FROM kb_access_grants g
         WHERE g.subject_table = 'kb_resources' AND g.principal_table = 'kb_profiles'
           AND g.principal_id = p_profile AND g.can_read
        UNION
        -- team-anchored grant on a reachable (self-or-ancestor) team
        SELECT g.subject_id FROM kb_access_grants g
         JOIN reachable_teams rt ON g.principal_id = rt.team_id
         WHERE g.subject_table = 'kb_resources' AND g.principal_table = 'kb_teams' AND g.can_read
        UNION
        -- resources homed in a context the profile can READ
        SELECT h.resource_id
        FROM contexts_readable_by(p_profile) rc
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_contexts' AND h.anchor_id = rc.context_id
        UNION
        -- cogmap membership: resources homed in a cognitive map joined to a REACHABLE team
        SELECT h.resource_id
        FROM kb_team_cogmaps tc
        JOIN reachable_teams rt ON rt.team_id = tc.team_id
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_cogmaps' AND h.anchor_id = tc.cogmap_id
        UNION
        -- explicit read-grant on a COGMAP home
        SELECT h.resource_id
        FROM kb_resource_homes h
        JOIN kb_access_grants g
          ON g.subject_table = h.anchor_table AND g.subject_id = h.anchor_id
        WHERE h.anchor_table = 'kb_cogmaps' AND g.can_read
          AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
             OR (g.principal_table = 'kb_teams'    AND g.principal_id IN (SELECT team_id FROM reachable_teams)) )
    ) v
    -- soft-delete READ floor: a deleted resource is invisible on every axis.
    JOIN kb_resources r ON r.id = v.resource_id AND r.is_active;
$function$;

CREATE OR REPLACE FUNCTION public.can_modify_resource(p_profile uuid, p_resource uuid)
 RETURNS boolean
 LANGUAGE sql
 STABLE
AS $function$
    -- Soft-delete WRITE floor: a tombstone is unmodifiable on every axis.
    SELECT EXISTS (SELECT 1 FROM kb_resources r WHERE r.id = p_resource AND r.is_active)
       AND EXISTS (
        WITH reachable_teams AS (
            SELECT DISTINCT a.team_id
            FROM profile_effective_teams(p_profile) e
            CROSS JOIN LATERAL team_ancestors(e.team_id) a
        )
        -- owned (the home confers modify to its OWNER; originator is provenance only, not access)
        SELECT 1 FROM kb_resource_homes h
         WHERE h.resource_id = p_resource
           AND h.owner_profile_id = p_profile
        UNION ALL
        -- direct profile-anchored WRITE grant.
        SELECT 1 FROM kb_access_grants g
         WHERE g.subject_table = 'kb_resources' AND g.subject_id = p_resource
           AND g.principal_table = 'kb_profiles' AND g.principal_id = p_profile AND g.can_write
        UNION ALL
        -- team-anchored WRITE grant on a reachable (self-or-ancestor) team.
        SELECT 1 FROM kb_access_grants g
         JOIN reachable_teams rt ON g.principal_id = rt.team_id
         WHERE g.subject_table = 'kb_resources' AND g.subject_id = p_resource
           AND g.principal_table = 'kb_teams' AND g.can_write
        UNION ALL
        -- container-write cascade: whoever may author the home container may modify its nodes.
        SELECT 1 FROM kb_resource_homes h
         WHERE h.resource_id = p_resource
           AND CASE h.anchor_table
                 WHEN 'kb_cogmaps'  THEN cogmap_authorable_by_profile(p_profile, h.anchor_id)
                 WHEN 'kb_contexts' THEN context_authorable_by_profile(p_profile, h.anchor_id)
                 ELSE false
               END
    );
$function$;
```

- [ ] **Step 5: Rebuild so the migrator macro sees the new migration, run the test green**

The `#[sqlx::test]` migrator embeds `migrations/` at compile time; a freshly-added migration can be missed by a stale build cache.

Run:
```bash
cargo clean -p temper-substrate
cargo nextest run -p temper-substrate --features test-db --test originator_demotion
```
Expected: BOTH tests PASS.

- [ ] **Step 6: Run the neighboring predicate suites to confirm no regression**

Run:
```bash
cargo nextest run -p temper-substrate --features test-db --test container_write_cascade
cargo nextest run -p temper-services --features test-db --test context_read_predicate_test
cargo nextest run -p temper-api --features test-db --test context_team_owned_resource_visibility_test --test soft_delete_read_floor_test
```
Expected: all PASS (these use owner == originator creators, so the narrowed predicate is behavior-identical for them).

- [ ] **Step 7: `cargo make check` and commit**

Run:
```bash
cargo make check
git add migrations/20260715000020_demote_originator_from_access.sql crates/temper-substrate/tests/originator_demotion.rs
git commit -m "feat(access): demote originator_profile_id from access predicates (D1)

owner_profile_id is now the single access-bearing profile key; originator is
pure provenance. CREATE OR REPLACE resources_visible_to + can_modify_resource,
dropping the originator arm. Behavior-preserving on current data (owner NOT NULL,
0 divergence); makes resource_reassign a true handoff. Task 019f6399-3c96.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```
Expected: check green, commit succeeds.

---

## PR2 — D3: Surface inherited reach on transfer

### Task 2: Extend the transfer outcome with inherited shares + read-grants

Add the residual read-reach (shares + context read-grants) to `ReassignContextOutcome`,
gathered by a new `context_service` read and populated on both the success and idempotent-no-op
return paths. No sweeping — surface only.

**Files:**
- Modify: `crates/temper-core/src/types/context.rs` (add two structs + two fields near `:124`)
- Modify: `crates/temper-services/src/services/context_service.rs` (new `inherited_reach` read; populate in `reassign`, near `:498`)
- Modify: `tests/e2e/tests/context_transfer_e2e.rs` (new test)

**Interfaces:**
- Consumes: `ReassignContextOutcome` (existing: `context_id: Uuid`, `owner_ref: String`, `reassigned: bool`), `context_service::reassign(pool, caller: ProfileId, context_id: Uuid, to_team_id: Uuid) -> ApiResult<ReassignContextOutcome>`.
- Produces:
  - `pub struct InheritedShare { pub team_id: Uuid, pub team_ref: String }`
  - `pub struct InheritedReadGrant { pub principal_table: String, pub principal_id: Uuid, pub principal_ref: String }`
  - `ReassignContextOutcome.inherited_shares: Vec<InheritedShare>`, `.inherited_read_grants: Vec<InheritedReadGrant>`
  - `async fn inherited_reach(pool, context_id) -> ApiResult<(Vec<InheritedShare>, Vec<InheritedReadGrant>)>` (private to context_service).

- [ ] **Step 1: Add the two DTO structs and the two outcome fields**

In `crates/temper-core/src/types/context.rs`, add above `ReassignContextOutcome`:

```rust
/// A team the context is shared to (read-reach) at transfer time — reach the new owner inherits.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InheritedShare {
    pub team_id: Uuid,
    /// Decorated `+team-slug` ref.
    pub team_ref: String,
}

/// An explicit context read-grant that survives the ownership flip — inherited residual reach.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "context.ts"))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InheritedReadGrant {
    /// `kb_profiles` or `kb_teams`.
    pub principal_table: String,
    pub principal_id: Uuid,
    /// Decorated ref: `@handle` for a profile, `+slug` for a team.
    pub principal_ref: String,
}
```

Then add these two fields to the existing `ReassignContextOutcome` struct (after `reassigned`):

```rust
    /// Read-reach the new owner inherits: teams this context was shared to (kb_team_contexts).
    /// Surfaced, not swept — the new owner prunes deliberately.
    #[serde(default)]
    pub inherited_shares: Vec<InheritedShare>,
    /// Read-reach the new owner inherits: explicit context read-grants (kb_access_grants).
    #[serde(default)]
    pub inherited_read_grants: Vec<InheritedReadGrant>,
```

- [ ] **Step 2: Add the `inherited_reach` read to context_service**

In `crates/temper-services/src/services/context_service.rs`, add (near `team_owner_ref`, importing `InheritedShare`/`InheritedReadGrant` at the top `use temper_core::types::context::{...}` group):

```rust
/// Gather the read-reach a transfer leaves in place: `kb_team_contexts` shares plus explicit
/// `kb_access_grants` context read-grants. Surfaced in the transfer outcome; never swept.
async fn inherited_reach(
    pool: &PgPool,
    context_id: uuid::Uuid,
) -> ApiResult<(Vec<InheritedShare>, Vec<InheritedReadGrant>)> {
    let shares = sqlx::query!(
        r#"SELECT tc.team_id AS "team_id!", t.slug AS "slug!"
             FROM kb_team_contexts tc
             JOIN kb_teams t ON t.id = tc.team_id
            WHERE tc.context_id = $1
            ORDER BY t.slug"#,
        context_id,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| InheritedShare { team_id: r.team_id, team_ref: format!("+{}", r.slug) })
    .collect();

    let grants = sqlx::query!(
        r#"SELECT g.principal_table AS "principal_table!",
                  g.principal_id    AS "principal_id!",
                  COALESCE(p.handle, t.slug) AS "principal_name!"
             FROM kb_access_grants g
             LEFT JOIN kb_profiles p ON g.principal_table = 'kb_profiles' AND p.id = g.principal_id
             LEFT JOIN kb_teams    t ON g.principal_table = 'kb_teams'    AND t.id = g.principal_id
            WHERE g.subject_table = 'kb_contexts' AND g.subject_id = $1 AND g.can_read
            ORDER BY g.principal_table, g.principal_id"#,
        context_id,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|r| {
        let principal_ref = if r.principal_table == "kb_profiles" {
            format!("@{}", r.principal_name)
        } else {
            format!("+{}", r.principal_name)
        };
        InheritedReadGrant { principal_table: r.principal_table, principal_id: r.principal_id, principal_ref }
    })
    .collect();

    Ok((shares, grants))
}
```

- [ ] **Step 3: Populate the fields on both return paths of `reassign`**

In `reassign`, after the auth + existence checks resolve `context_id`, gather reach once and
spread into both returns. Replace the idempotent-no-op return:

```rust
    let (inherited_shares, inherited_read_grants) = inherited_reach(pool, context_id).await?;
    if cur.owner_table == "kb_teams" && cur.owner_id == to_team_id {
        return Ok(ReassignContextOutcome {
            context_id,
            owner_ref: team_owner_ref(pool, to_team_id).await?,
            reassigned: false,
            inherited_shares,
            inherited_read_grants,
        });
    }
```

and the success return at the end of the function:

```rust
    Ok(ReassignContextOutcome {
        context_id,
        owner_ref: team_owner_ref(pool, to_team_id).await?,
        reassigned: true,
        inherited_shares,
        inherited_read_grants,
    })
```

(The `inherited_reach` call goes after the `cur` lookup so `context_id` is confirmed to exist.)

- [ ] **Step 4: Regenerate the sqlx cache for the new macro queries**

Run:
```bash
cargo sqlx prepare --workspace -- --all-features
```
Expected: `.sqlx/` gains entries for the two new `inherited_reach` queries. Stage them later with the code.

- [ ] **Step 5: Write the e2e test**

In `tests/e2e/tests/context_transfer_e2e.rs`, add a test that seeds a share + a context
read-grant via direct SQL, transfers via the API, and asserts the outcome carries both. Follow
the file's existing `provision` / `transfer_status` / `context_owner` helpers and its
`#[sqlx::test(migrator = "temper_api::MIGRATOR")]` style. Assert the deserialized
`ReassignContextOutcome` JSON:

```rust
/// A transfer surfaces (does not sweep) the read-reach it inherits.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn transfer_surfaces_inherited_shares_and_read_grants(pool: sqlx::PgPool) {
    // Reuse the file's setup: an admin who owns a personal context, a target team T the admin
    // owns/maintains, plus a second team `other` and a second profile `viewer`.
    // (Build with the same helpers the sibling tests use — provision(), team + membership inserts.)

    // Seed residual reach on the context BEFORE transfer:
    //   1) a share to `other`  → kb_team_contexts
    //   2) a context read-grant to `viewer` → kb_access_grants(subject='kb_contexts', can_read)
    sqlx::query("INSERT INTO kb_team_contexts (context_id, team_id) VALUES ($1, $2)")
        .bind(ctx).bind(other_team).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO kb_access_grants \
             (subject_table, subject_id, principal_table, principal_id, can_read, can_write, granted_by_profile_id) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, false, $3)",
    ).bind(ctx).bind(viewer).bind(admin).execute(&pool).await.unwrap();

    // Transfer ctx → T via the API, deserialize ReassignContextOutcome from the 200 body.
    let outcome: temper_core::types::context::ReassignContextOutcome = /* POST /api/contexts/{ctx}/reassign */;

    assert!(outcome.reassigned);
    assert_eq!(outcome.inherited_shares.len(), 1, "the share to `other` is surfaced");
    assert_eq!(outcome.inherited_shares[0].team_id, other_team);
    assert_eq!(outcome.inherited_read_grants.len(), 1, "the viewer read-grant is surfaced");
    assert_eq!(outcome.inherited_read_grants[0].principal_id, viewer);
    assert!(outcome.inherited_read_grants[0].principal_ref.starts_with('@'));
}
```

Then a companion asserting a context with no share/grant returns empty vectors (extend the
existing `transfer_makes_context_team_owned_and_members_can_author` with
`assert!(outcome.inherited_shares.is_empty()); assert!(outcome.inherited_read_grants.is_empty());`).

- [ ] **Step 6: Prepare the e2e per-crate cache, run the e2e tests**

The e2e crate keeps its own `.sqlx` for test-target queries — but this test uses runtime
`sqlx::query(...)` for its raw inserts (no macro), so no e2e cache regen is needed. Run:
```bash
cargo make test-e2e -E 'binary(context_transfer_e2e)'
```
Expected: the new test and the extended existing test PASS.

- [ ] **Step 7: `cargo make check` and commit (code only, before wire regen)**

Run:
```bash
cargo make check
git add crates/temper-core/src/types/context.rs \
        crates/temper-services/src/services/context_service.rs \
        crates/temper-services/.sqlx tests/e2e/tests/context_transfer_e2e.rs
git commit -m "feat(teams): surface inherited read-reach on context transfer (D3)

ReassignContextOutcome now reports the shares + context read-grants the new owner
inherits (surfaced, not swept). New context_service::inherited_reach read; both
CLI and MCP already serialize the whole outcome. Task 019f6399-3c96.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```
Expected: check green (the wire-drift gates may still be RED here — that is Task 3's job; if `cargo make check` fails only on `openapi-check`/`openapi-ts-drift`, proceed to Task 3 and commit together instead).

> **Note:** if `cargo make check` reds on the drift gates at Step 7, do **not** split the commit — fold Steps 1-7 and Task 3 into one commit so no commit is left with a stale wire artifact.

### Task 3: Regenerate the wire artifacts (openapi + rb + ts + ts-rs)

The new DTO restales `openapi.json`, the temper-rb gem, temper-ts `schema.ts`, and the ts-rs
`context.ts`. Regenerate and stage so the drift gates (which diff against git) pass.

**Files:**
- Modify (generated): `openapi.json`, `clients/temper-rb/lib/temper/generated/**`, `clients/temper-ts/src/generated/schema.ts`, `packages/temper-ui/src/lib/types/generated/context.ts` (ts-rs export target)

**Interfaces:**
- Consumes: the `ReassignContextOutcome` / `InheritedShare` / `InheritedReadGrant` types from Task 2.
- Produces: regenerated committed artifacts, no new API surface.

- [ ] **Step 1: Regenerate ts-rs types**

Run:
```bash
cargo make generate-ts-types
```
Expected: `context.ts` (the ts-rs export target) gains `InheritedShare`, `InheritedReadGrant`, and the two new `ReassignContextOutcome` fields.

- [ ] **Step 2: Regenerate openapi + gem + temper-ts schema**

Run (gem regen needs Docker running):
```bash
cargo make openapi
```
Expected: `openapi.json` gains the two schemas + fields; the gem and `schema.ts` regenerate to match.

- [ ] **Step 3: Stage everything and confirm the drift gates pass**

Run:
```bash
git add openapi.json clients/temper-rb clients/temper-ts packages/temper-ui/src/lib/types/generated
cargo make check
```
Expected: `openapi-check`, `openapi-ts-drift`, and (Docker present) `openapi-rb-drift` all green now that the regenerated files are staged.

- [ ] **Step 4: Commit the regenerated artifacts**

Run:
```bash
git commit -m "chore(openapi): regenerate wire artifacts for inherited-reach fields

Regen of openapi.json + temper-rb + temper-ts + ts-rs for the D3
ReassignContextOutcome additions. Task 019f6399-3c96.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```
Expected: commit succeeds.

---

## Task 4: Create the offboarding spin-out backlog task

The genuine residual-access case transfer does not cover: after D1, a departed member retains
access to resources they still *own* in a team context until ownership is handed off (Lever B on
offboarding). Capture it so it is not lost.

- [ ] **Step 1: Create the backlog task**

Run:
```bash
cat <<'EOF' | temper resource create --type task --title "Offboarding: departed-member owned-resource handoff (Lever B on team removal)" --context @me/temper --mode plan --effort medium --goal 019f25d6-e1a9-7360-8a35-6bdf8ef53940
# Offboarding: departed-member owned-resource handoff

Fast-follow to context-transfer-safety (T-D). After D1 demoted originator from access,
a resource's access is `owner_profile_id` + grants + container cascade. When a member
LEAVES a team, they retain access to resources they still OWN in that team's contexts
(owner_profile_id still points at them) until ownership is handed off — this is Lever B
(`resource_reassign`) applied on offboarding, the residual-access case a container
transfer legitimately does not address.

## Questions
- On team-member removal, what happens to resources they own in the team's contexts?
  Bulk-reassign owner to a designated team owner/maintainer? Leave + surface?
- Reuse `resource_reassign` per-resource, or a new bulk op?
- Interaction with the two-axis access model and container-write cascade.

## Refs
- Spec: docs/superpowers/specs/2026-07-15-context-transfer-safety-residual-access-design.md
- Parent task 019f6399-3c96-7273-97a7-53397682c881
EOF
```
Expected: task created in `@me/temper`, linked to the Teams goal.

---

## Self-Review

**Spec coverage:**
- D1 (demote originator) → PR1 Task 1. ✓
- D2 (transfer performs no owner reassign) → no code; documented boundary in spec, nothing to build. ✓ (correctly absent)
- D3 (surface residual reach) → PR2 Tasks 2-3. ✓
- D4 (write-grant guardrail) → no code (verified no mint path); spec-documented. ✓ (correctly absent)
- D5 (cogmap boundary), D6 (ingest race) → documented boundaries, no code. ✓ (correctly absent)
- Spin-out (offboarding) → Task 4. ✓
- Testing: differential/behavior-preserving + divergence (Task 1 Steps 2-6); adversarial predicate probes are the SQL-level `can_read`/`can_modify` calls; e2e surfacing (Task 2 Step 5). ✓

**Placeholder scan:** The e2e test (Task 2 Step 5) leaves the setup helpers and the POST call as prose comments rather than full code, because they must mirror the sibling tests' existing private helpers in the same file (`provision`, `transfer_status`, team/membership inserts) which the implementer reads in-place — spelling them out here would duplicate and risk drift. Every other step carries complete code. The seeded-reach inserts and all assertions ARE fully specified.

**Type consistency:** `InheritedShare { team_id, team_ref }` and `InheritedReadGrant { principal_table, principal_id, principal_ref }` are defined once (Task 2 Step 1) and consumed identically in `inherited_reach` (Step 2), the `reassign` returns (Step 3), and the e2e assertions (Step 5). `ReassignContextOutcome` field names (`inherited_shares`, `inherited_read_grants`) match across all three. `inherited_reach` signature matches its call site. ✓
