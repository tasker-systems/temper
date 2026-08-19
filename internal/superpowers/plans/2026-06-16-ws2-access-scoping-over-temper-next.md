# WS2 — Access-Scoping Over `temper_next` (Consumer Axis) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `temper_next` reads/writes access-correct by wiring the already-proven `resources_visible_to(profile)` into the `NextBackend` read path and adding+wiring a `can_modify_resource` write gate, so flipping `flag=next` cannot leak or accept unauthorized access (flip-prerequisite #1).

**Architecture:** The access *model* and `resources_visible_to(profile)` already exist, are proven (PR #129 access-scenarios), and are deployed (install migration `20260613000001`). This plan is **wiring, not model design**: (1) synthesis preserves production profile ids so the auth'd principal resolves directly; (2) the `readback` read functions take a `principal` and JOIN-filter through `resources_visible_to`, conforming to production's JOIN pattern and its 404-read/403-write deny split; (3) a new `can_modify_resource` function (capability-model, write-bit) is added to the artifact + a forward migration, and `NextBackend` write methods gate on it before mutating. Proof = the rich PR #129 scenario topology (wiring correctness) + a thin principal-aware extension to the chunk-3 parity harness (no regression on real data).

**Tech Stack:** Rust (temper-next, temper-api), PostgreSQL 18 + pgvector, sqlx (runtime schema-qualified `query` for `temper_next` — never the `query!` macros, per the module discipline), cargo-nextest. Tests run under the `artifact-tests` feature (the `temper-next-write` nextest group, which owns + resets the `temper_next` namespace) and the `next-backend` feature for temper-api.

**Spec:** `docs/superpowers/specs/2026-06-16-ws2-access-scoping-over-temper-next-design.md`. Read it before starting. Load-bearing invariants carried verbatim below.

**Build/test notes (CONFORM — these bite):**
- `next-backend` builds need `SQLX_OFFLINE=true` (CLAUDE.md temper-next note). All `cargo make` tasks set it.
- After changing temper-next SQL or the artifact functions a macro references: `cargo make prepare-next` (per-crate `.sqlx`, never `--workspace`). The readback/gate queries in this plan are **runtime** `sqlx::query`/`query_scalar` (schema-qualified), so they do **not** need macro re-prepare — but the existing `check_can_modify` in temper-api uses the `query_scalar!` *macro* against `public`, which is unaffected.
- temper-next artifact tests: `cargo nextest run -p temper-next --features artifact-tests` (write-path; resets namespace to `01_schema`+`02_functions` then seeds). The drift guard test also lives here.
- temper-api next-backend tests: `cargo nextest run -p temper-api --features "test-db next-backend" <name>`.

---

## File Structure

- **Modify** `crates/temper-next/src/synthesis/bootstrap.rs` (`insert_profile`, ~`:202`) — insert explicit `id = old_id` to preserve production profile ids.
- **Modify** `crates/temper-next/src/readback/mod.rs` — add a `principal: Uuid` parameter to `list`, `meta`, `resource_row`, `body`, `fts_search`, `vector_search`, `neighbors`; JOIN-filter each through `temper_next.resources_visible_to(principal)`.
- **Modify** `schema-artifact/02_functions.sql` — add `can_modify_resource(p_profile, p_resource)` (capability model, write bit). Artifact stays design-master.
- **Create** `migrations/20260616NNNNNN_temper_next_can_modify.sql` — idempotent forward migration adding the same function to the deployed `temper_next` (append-only lineage; install migration stays frozen).
- **Modify** `crates/temper-api/src/backend/read_selector.rs` — Next arms pass `profile_id` into `next_impl`; `next_impl` passes it to `readback`.
- **Modify** `crates/temper-api/src/backend/next_backend.rs` — `update_resource` / `delete_resource` / `assert_relationship` / `retype_relationship` / `reweight_relationship` / `fold_relationship` call a `check_can_modify_next` gate before mutating.
- **Create** `crates/temper-next/tests/access_scoping.rs` — P1 wiring-correctness tests over the PR #129 access-scenario topology.
- **Modify** `crates/temper-next/tests/parity_reads.rs` — P2 principal-aware parity extension.

**Decomposition note:** Tasks 1–4 are independent of each other except Task 2 depends on Task 1's preserved ids being available to assert against, and Task 4 depends on Task 3's function existing. Tasks 5–6 (proofs) depend on 1–4. Commit after every task.

---

## Task 1: Preserve production profile ids in synthesis

**Why:** `resources_visible_to(p_profile)` expects a `temper_next` profile id; the auth'd principal is a *production* profile id. Preserving the id in synthesis removes any read-time mapping layer. (Spec D2, principal-mapping.)

**Invariant (verbatim, spec D2):** *"synthesis preserves production profile ids verbatim … then `resources_visible_to(prod_profile_id)` resolves directly, no mapping … it tightens parity and never widens the §9 floor (profile ids are not asserted invariants there)."*

**Files:**
- Modify: `crates/temper-next/src/synthesis/bootstrap.rs` (`insert_profile`, `:202-230`)
- Test: `crates/temper-next/tests/access_scoping.rs` (created here; reused by Task 5)

**Tag:** AMEND — changes synthesis id behavior; authorized by spec D2.

- [ ] **Step 1: Write the failing test**

Create `crates/temper-next/tests/access_scoping.rs`:

```rust
#![cfg(feature = "artifact-tests")]
//! WS2 — access-scoping wiring proofs over temper_next. Owns + resets the
//! temper_next namespace (temper-next-write nextest group), same as parity_reads.

mod common; // if a shared harness module exists alongside parity_reads; otherwise inline setup

use sqlx::Row;
use temper_next::test_support::{reset_and_seed_prod_shape}; // CONFORM to parity_reads' setup entrypoint

#[sqlx::test(migrations = false)]
async fn synthesis_preserves_production_profile_ids(pool: sqlx::PgPool) {
    // Arrange: a prod-shape fixture with a known owner profile, synthesized into temper_next.
    let ctx = reset_and_seed_prod_shape(&pool).await;

    // Act: read the synthesized owner profile id back.
    let synth_id: uuid::Uuid = sqlx::query_scalar(
        "SELECT id FROM temper_next.kb_profiles WHERE handle = $1",
    )
    .bind(&ctx.owner_handle)
    .fetch_one(&pool)
    .await
    .unwrap();

    // Assert: it equals the PRODUCTION profile id (preserved, not re-minted).
    assert_eq!(
        synth_id, ctx.prod_owner_profile_id,
        "synthesis must preserve the production profile id verbatim"
    );
}
```

> **Engineer note:** `parity_reads.rs` already has the prod-shape fixture + synthesis-run setup. Before writing, open `crates/temper-next/tests/parity_reads.rs` and reuse its exact setup helper and the struct it returns (its fields expose the production owner profile id and handle). Mirror its `#[sqlx::test]` attributes and feature gate. Do **not** invent a `test_support` module if the existing harness lives in a test-local `common` module — use whatever `parity_reads.rs` uses. The assertion (`synth_id == prod_owner_profile_id`) is the contract regardless of helper names.

- [ ] **Step 2: Run the test to verify it fails**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests synthesis_preserves_production_profile_ids`
Expected: FAIL — synthesized id differs from the production id (ids are re-minted today).

- [ ] **Step 3: Implement — preserve the id in `insert_profile`**

In `crates/temper-next/src/synthesis/bootstrap.rs`, change both INSERTs in `insert_profile` to set the explicit `id` column bound to `old_id`:

```rust
async fn insert_profile(
    conn: &mut sqlx::PgConnection,
    old_id: Uuid,
    handle: &str,
    display_name: &str,
) -> Result<ProfileId> {
    // Preserve the production profile id verbatim (WS2 D2): resources_visible_to(prod_profile)
    // then resolves directly with no read-time mapping. uuid_generate_v7() is NOT used.
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO temper_next.kb_profiles (id, handle, display_name) VALUES ($1, $2, $3) \
         ON CONFLICT (handle) DO NOTHING RETURNING id",
    )
    .bind(old_id)
    .bind(handle)
    .bind(display_name)
    .fetch_optional(&mut *conn)
    .await?;
    let id = match inserted {
        Some(id) => id,
        None => {
            // handle collision (two profiles sluggify to the same handle): disambiguate the
            // handle but STILL preserve the id (old_id is unique, so no PK collision here).
            let disambiguated = format!("{handle}-{}", &old_id.simple().to_string()[..8]);
            sqlx::query_scalar(
                "INSERT INTO temper_next.kb_profiles (id, handle, display_name) VALUES ($1, $2, $3) \
                 RETURNING id",
            )
            .bind(old_id)
            .bind(disambiguated)
            .bind(display_name)
            .fetch_one(&mut *conn)
            .await?
        }
    };
    Ok(ProfileId::from(id))
}
```

> **Engineer note:** `temper_next.kb_profiles.id` has a `DEFAULT uuid_generate_v7()` (see `schema-artifact/01_schema.sql:70-75`); supplying `id` explicitly overrides the default. No schema change is needed.

- [ ] **Step 4: Run the test to verify it passes**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests synthesis_preserves_production_profile_ids`
Expected: PASS.

- [ ] **Step 5: Regression — full temper-next artifact suite**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests`
Expected: PASS (preserving ids tightens parity; nothing asserts re-minted profile ids). If a synthesis test asserted a profile id was *different* from production, that assertion was encoding the re-mint accident — update it to expect equality (cite this task).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-next/src/synthesis/bootstrap.rs crates/temper-next/tests/access_scoping.rs
git commit -m "WS2: preserve production profile ids in synthesis

insert_profile now inserts the explicit id=old_id (PR#124 identity-as-input)
so resources_visible_to(prod_profile) resolves directly — no read-time
profile bimap. Tightens parity (synthesized owner id == prod id); profile
ids are not §9-asserted invariants so the floor is unchanged.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Thread the principal through `readback` + JOIN-filter

**Why:** Today every `readback` read returns the unscoped active set (`readback/mod.rs:7-14`). WS2 scopes each through `resources_visible_to(principal)`, conforming to production's JOIN pattern (`resource_service.rs:249,348,370`).

**Invariant (verbatim, spec D1):** *"a not-visible resource returns 404 `NotFound` on reads (never 403 — denying existence prevents an existence-leak oracle); writes return 403 `Forbidden`."* And: set reads JOIN-filter to the visible set; single-resource reads gate ⇒ `None` ⇒ surface maps to 404.

**Files:**
- Modify: `crates/temper-next/src/readback/mod.rs` (`list:148`, `meta:221`, `resource_row:316`, `body:410`, `fts_search:444`, `vector_search:504`, `neighbors:560`)
- Modify: `crates/temper-api/src/backend/read_selector.rs` (Next arms `:37,52,65,78`; `next_impl` `:134,154,168,186`)
- Test: `crates/temper-next/tests/access_scoping.rs`

**Tag:** EXTEND — adds scoping to the §9-unscoped read floor; authorized by spec D1/D2.

- [ ] **Step 1: Write the failing test (visible-set exactness + deny)**

Append to `crates/temper-next/tests/access_scoping.rs`:

```rust
#[sqlx::test(migrations = false)]
async fn list_returns_only_resources_visible_to_principal(pool: sqlx::PgPool) {
    // Load the rich access topology (alice/bob/carol/nomad) into temper_next, the same
    // topology PR #129 proves resources_visible_to correct over.
    let topo = load_access_scenario_topology(&pool, "context-share-access").await;

    // alice can see rdoc (team-a share) but not sdoc (unshared context).
    let visible = temper_next::readback::list(&pool, topo.profile("alice")).await.unwrap();
    let uris: std::collections::HashSet<_> =
        visible.iter().map(|r| r.origin_uri.clone()).collect();
    assert!(uris.contains(&topo.origin_uri("rdoc")), "alice must see rdoc");
    assert!(!uris.contains(&topo.origin_uri("sdoc")), "alice must NOT see sdoc");

    // The set must equal resources_visible_to(alice) computed by the SQL function directly.
    let expected: std::collections::HashSet<String> = sqlx::query(
        "SELECT r.origin_uri FROM temper_next.kb_resources r \
         JOIN temper_next.resources_visible_to($1) v ON v.resource_id = r.id",
    )
    .bind(topo.profile("alice"))
    .fetch_all(&pool).await.unwrap()
    .iter().map(|row| row.get::<String,_>("origin_uri")).collect();
    assert_eq!(uris, expected, "list must equal resources_visible_to(alice) exactly");
}

#[sqlx::test(migrations = false)]
async fn single_resource_read_denies_when_not_visible(pool: sqlx::PgPool) {
    let topo = load_access_scenario_topology(&pool, "context-share-access").await;
    // bob cannot see rdoc (no share to team-b). resource_row must return "not visible".
    let rdoc = topo.new_id("rdoc");
    let res = temper_next::readback::resource_row(&pool, topo.profile("bob"), rdoc).await;
    assert!(
        matches!(res, Err(ref e) if e.to_string().contains("not visible")),
        "resource_row for a not-visible resource must signal absence, got {res:?}"
    );
}
```

> **Engineer note:** `load_access_scenario_topology` loads `schema-artifact/access-scenarios/context-share-access.yaml` into a freshly-reset `temper_next` via the existing access-scenario loader used by the `temper-next-write` group (find it under `crates/temper-next/src/scenario/` / the access-scenario test harness; `context-share-access.yaml:32-83` is the topology + checks). Reuse it — do not hand-build the topology. It must expose `profile(name)->Uuid`, `origin_uri(name)->String`, `new_id(name)->Uuid`. If the loader returns raw maps, write a thin local adapter in the test file. The `resource_row` "not visible" signal is whatever the implementation in Step 3 chooses (an `Err` whose message contains `not visible`, or a dedicated `NotVisible` error) — keep test and impl consistent.

- [ ] **Step 2: Run to verify failure**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests list_returns_only single_resource_read_denies`
Expected: FAIL — `list`/`resource_row` don't take a principal yet (compile error), then logic failure.

- [ ] **Step 3: Implement — add `principal` + JOIN to each readback fn**

For the **set reads**, add `principal: Uuid` and JOIN `resources_visible_to`. Example for `list` (`readback/mod.rs:148`):

```rust
pub async fn list(pool: &PgPool, principal: Uuid) -> Result<Vec<ListRow>> {
    let rows = sqlx::query(
        "SELECT r.origin_uri, r.title,
                dt.property_value #>> '{}' AS doc_type,
                st.property_value #>> '{}' AS stage,
                md.property_value #>> '{}' AS mode,
                ef.property_value #>> '{}' AS effort
           FROM temper_next.kb_resources r
           JOIN temper_next.resources_visible_to($1) v ON v.resource_id = r.id
           JOIN temper_next.kb_properties dt
             ON dt.owner_table = 'kb_resources' AND dt.owner_id = r.id
            AND dt.property_key = 'doc_type' AND NOT dt.is_folded
           LEFT JOIN temper_next.kb_properties st
             ON st.owner_table = 'kb_resources' AND st.owner_id = r.id
            AND st.property_key = 'temper-stage' AND NOT st.is_folded
           LEFT JOIN temper_next.kb_properties md
             ON md.owner_table = 'kb_resources' AND md.owner_id = r.id
            AND md.property_key = 'temper-mode' AND NOT md.is_folded
           LEFT JOIN temper_next.kb_properties ef
             ON ef.owner_table = 'kb_resources' AND ef.owner_id = r.id
            AND ef.property_key = 'temper-effort' AND NOT ef.is_folded
          ORDER BY r.origin_uri",
    )
    .bind(principal)
    .fetch_all(pool)
    .await?;
    // ... unchanged row mapping ...
}
```

> **Engineer note — bind numbering:** `list`/`vector_search` currently bind nothing / one positional. Adding `$1 = principal` shifts existing binds. For `fts_search` (binds the query twice as `$1`) and `vector_search` (binds the embedding as `$1`), make `principal` `$1` and renumber the query/embedding to `$2` (and `$3` where the FTS query appears in both WHERE and ORDER BY → it stays one bind reused; only renumber the placeholder). Re-read each function body and renumber carefully — these are runtime queries so a wrong number fails at runtime, not compile time. Add a test exercising each (the P1 tests in Task 5 cover fts/vector/neighbors).

Apply the same `JOIN temper_next.resources_visible_to($1) v ON v.resource_id = r.id` to `fts_search` (inside the `doc` CTE's `FROM temper_next.kb_resources r`), `vector_search`, and `neighbors` (gate BOTH endpoints: the seed must be visible AND each neighbor endpoint must be visible — add the join on the seed in each UNION arm's `kb_resources` and on the neighbor `t`/`s`).

For the **single-resource reads** (`resource_row`, `meta`, `body`), gate the seed and return a not-visible signal. Example for `resource_row` (`:316`): add `principal: Uuid`, change `WHERE r.id = $1` to bind principal as `$2` and add `JOIN temper_next.resources_visible_to($2) v ON v.resource_id = r.id`, and switch `.fetch_one` → `.fetch_optional`, mapping `None`:

```rust
pub async fn resource_row(pool: &PgPool, principal: Uuid, new_id: Uuid) -> Result<ResourceRowParity> {
    let row = sqlx::query(
        "SELECT r.id AS re_minted_id, /* ...unchanged columns... */
           FROM temper_next.kb_resources r
           JOIN temper_next.resources_visible_to($2) v ON v.resource_id = r.id
           JOIN temper_next.kb_resource_homes h ON h.resource_id = r.id
           /* ...unchanged joins... */
          WHERE r.id = $1",
    )
    .bind(new_id)
    .bind(principal)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow::anyhow!("resource {new_id} not visible to {principal}"))?;
    // ... unchanged field extraction ...
}
```

Mirror for `meta` (`:221`, add principal + visibility predicate on the implicit resource; since `meta` reads `kb_properties` keyed by `owner_id = $1`, add a guard `AND EXISTS (SELECT 1 FROM temper_next.resources_visible_to($2) v WHERE v.resource_id = $1)` and treat empty result as not-visible) and `body` (`:410`, gate the `SELECT origin_uri ... WHERE id = $1` with the visibility join → `fetch_optional` → not-visible error).

- [ ] **Step 4: Implement — pass the principal through `read_selector` Next arms**

In `crates/temper-api/src/backend/read_selector.rs`, the `*_select` functions already receive `profile_id` (`:32,45,60,73`). Pass it into the Next arms and `next_impl` (profile ids are now preserved, so the prod `profile_id` is the `temper_next` principal directly):

```rust
// list_select Next arm (:37)
BackendSelection::Next => next_impl::list(pool, profile_id).await,
// get_content_select (:52)
BackendSelection::Next => next_impl::get_content(pool, profile_id, resource_id).await,
// get_meta_select (:65) — profile_id is ProfileId here; pass Uuid::from(profile_id)
BackendSelection::Next => next_impl::get_meta(pool, Uuid::from(profile_id), Uuid::from(resource_id)).await,
// search_select (:78)
BackendSelection::Next => next_impl::search(pool, profile_id, params).await,
```

Update the `#[cfg(feature = "next-backend")] mod next_impl` functions to take `principal: Uuid` and pass it to `readback`. `get_content`/`get_meta` resolve `prod_id → new_id` via `resolve_new_id` then call the principal-aware readback. Because the body/meta readback now gate on visibility, map their not-visible error to `ApiError::from(TemperError::NotFound(...))` (CONFORM: 404, not 403). Also update the `#[cfg(not(feature = "next-backend"))]` stub `next_impl` signatures to match (add the `_principal: Uuid` arg) so the crate compiles without the feature.

- [ ] **Step 5: Run the P1 read tests**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests list_returns_only single_resource_read_denies`
Expected: PASS.

- [ ] **Step 6: Build temper-api under the feature (signature propagation)**

Run: `cargo make check` then `SQLX_OFFLINE=true cargo build -p temper-api --features "test-db next-backend"`
Expected: compiles — confirms the Next arms + both `next_impl` variants + callers line up.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-next/src/readback/mod.rs crates/temper-api/src/backend/read_selector.rs crates/temper-next/tests/access_scoping.rs
git commit -m "WS2: scope temper_next reads through resources_visible_to(principal)

Thread the auth'd principal through every readback read; JOIN-filter set
reads to the visible set and gate single-resource reads (not-visible ->
None -> 404). read_selector Next arms pass the (preserved) prod profile id
down. CONFORM to production's resources_visible_to JOIN pattern + 404 deny.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Add `can_modify_resource` to the artifact + a forward migration

**Why:** The artifact has the two read axes but **no write-axis function** (`02_functions.sql` — only `resources_visible_to`/`vis_team`/`resources_accessible_to_cogmap`). Production's `can_modify_resource` (`migrations/20260330000001`) is built on the **retired** tri-state model (`access_level`, `team_role`), so it is the wrong template — model the new one on the artifact's `resources_visible_to` (capability `can_write`, no context-share union: writes need an explicit grant or ownership).

**Invariant (verbatim, spec D3/D4):** *"the new write-axis gate lands as a **new forward migration** … idempotent … `can_modify_resource` is **also added to `schema-artifact/02_functions.sql`** … keeping the artifact the design-master and the forward migration its faithful append."* The semantic drift guard (committed migrations reconstruct the artifact schema) must keep passing.

**Files:**
- Modify: `schema-artifact/02_functions.sql` (append after the consumer-axis block, ~`:147`)
- Create: `migrations/20260616NNNNNN_temper_next_can_modify.sql`
- Test: `crates/temper-next/tests/access_scoping.rs`

**Tag:** EXTEND — new function the artifact lacks; authorized by spec D3. CONFORM to `resources_visible_to`'s reachability CTE shape.

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-next/tests/access_scoping.rs`:

```rust
#[sqlx::test(migrations = false)]
async fn can_modify_resource_honors_ownership_and_write_grants(pool: sqlx::PgPool) {
    let topo = load_access_scenario_topology(&pool, "context-share-access").await;
    let rdoc = topo.new_id("rdoc"); // homed/owned by carol per the fixture

    async fn can_modify(pool: &sqlx::PgPool, profile: uuid::Uuid, resource: uuid::Uuid) -> bool {
        sqlx::query_scalar::<_, Option<bool>>(
            "SELECT temper_next.can_modify_resource($1, $2)",
        )
        .bind(profile).bind(resource)
        .fetch_one(pool).await.unwrap().unwrap_or(false)
    }

    // Owner can modify; a reader-via-context-share canNOT (context-share is read-only reach).
    assert!(can_modify(&pool, topo.profile("carol"), rdoc).await, "owner carol can modify rdoc");
    assert!(!can_modify(&pool, topo.profile("alice"), rdoc).await,
        "alice reaches rdoc for READ via the share but has no write grant -> cannot modify");
    assert!(!can_modify(&pool, topo.profile("bob"), rdoc).await, "bob cannot even see rdoc");
}
```

> **Engineer note:** confirm against `context-share-access.yaml` which profile *owns* `rdoc` (the fixture comment at `:42` says rdoc is owned by carol so alice's reach rides the share). If the loader names differ, adjust — the contract is: owner→true, read-only-sharee→false, no-access→false.

- [ ] **Step 2: Run to verify failure**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests can_modify_resource_honors`
Expected: FAIL — `temper_next.can_modify_resource` does not exist.

- [ ] **Step 3: Add the function to the artifact (design-master)**

Append to `schema-artifact/02_functions.sql` after the consumer-axis block (after `:147`):

```sql
-- ============================================================================
-- WRITE AXIS — can_modify_resource(profile, resource)   (WS2)
-- ============================================================================
-- A person may MODIFY a resource if they own/originated it, hold a direct
-- profile-anchored WRITE grant, or hold a team-anchored WRITE grant on a
-- reachable team. Context-share is deliberately NOT a write path: it is a
-- read-reach mechanism (it enters vis(T)); writing requires an explicit
-- can_write grant or ownership (capability model — access-capability-model
-- design). Modeled on resources_visible_to's reachability CTE, write bit.
CREATE FUNCTION can_modify_resource(p_profile uuid, p_resource uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    WITH reachable_teams AS (
        SELECT DISTINCT a.team_id
        FROM profile_effective_teams(p_profile) e
        CROSS JOIN LATERAL team_ancestors(e.team_id) a
    )
    SELECT EXISTS (
        SELECT 1 FROM kb_resource_homes h
         WHERE h.resource_id = p_resource
           AND (h.owner_profile_id = p_profile OR h.originator_profile_id = p_profile)
        UNION ALL
        SELECT 1 FROM kb_resource_access ra
         WHERE ra.resource_id = p_resource
           AND ra.anchor_table = 'kb_profiles' AND ra.anchor_id = p_profile AND ra.can_write
        UNION ALL
        SELECT 1 FROM kb_resource_access ra
         JOIN reachable_teams rt ON ra.anchor_id = rt.team_id
         WHERE ra.resource_id = p_resource
           AND ra.anchor_table = 'kb_teams' AND ra.can_write
    );
$$;
```

> **Engineer note:** verify column names against `schema-artifact/01_schema.sql` `kb_resource_access` (the `can_write` boolean and `anchor_table`/`anchor_id`) and `kb_resource_homes` (`owner_profile_id`/`originator_profile_id`/`resource_id`) — `resources_visible_to` (`02_functions.sql:121-147`) uses exactly these, so copy its column references.

- [ ] **Step 4: Create the idempotent forward migration**

Create `migrations/20260616NNNNNN_temper_next_can_modify.sql` (use a timestamp strictly greater than `20260616000001`; check `ls migrations/ | tail` for the latest and increment):

```sql
-- WS2 — add the temper_next write-axis gate. Append-only to the frozen
-- temper_next lineage (install migration 20260613000001 stays untouched).
-- Idempotent: CREATE OR REPLACE so a re-run / a fresh persistent DB both land here.
CREATE OR REPLACE FUNCTION temper_next.can_modify_resource(p_profile uuid, p_resource uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    WITH reachable_teams AS (
        SELECT DISTINCT a.team_id
        FROM temper_next.profile_effective_teams(p_profile) e
        CROSS JOIN LATERAL temper_next.team_ancestors(e.team_id) a
    )
    SELECT EXISTS (
        SELECT 1 FROM temper_next.kb_resource_homes h
         WHERE h.resource_id = p_resource
           AND (h.owner_profile_id = p_profile OR h.originator_profile_id = p_profile)
        UNION ALL
        SELECT 1 FROM temper_next.kb_resource_access ra
         WHERE ra.resource_id = p_resource
           AND ra.anchor_table = 'kb_profiles' AND ra.anchor_id = p_profile AND ra.can_write
        UNION ALL
        SELECT 1 FROM temper_next.kb_resource_access ra
         JOIN reachable_teams rt ON ra.anchor_id = rt.team_id
         WHERE ra.resource_id = p_resource
           AND ra.anchor_table = 'kb_teams' AND ra.can_write
    );
$$;
```

> **Engineer note:** the artifact body uses unqualified names (it runs with `search_path=temper_next`); the migration runs against `public` search_path so it **schema-qualifies** every reference, exactly as `20260613000001` and `20260616000001` do. Confirm by reading the tail of `20260616000001_*.sql`.

- [ ] **Step 5: Run the test + the drift guard**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests can_modify_resource_honors`
Expected: PASS (the namespace reset loads `01`+`02`, which now includes the function).

Run the semantic drift guard (the test that committed migrations reconstruct the artifact schema):
Run: `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests -E 'test(drift)'`
Expected: PASS — both artifact and migration gained the function, so the `pg_catalog` fingerprints still match. If it FAILS, the artifact body and the migration body differ semantically (e.g. a column name typo in one) — diff them.

- [ ] **Step 6: Commit**

```bash
git add schema-artifact/02_functions.sql migrations/20260616*_temper_next_can_modify.sql crates/temper-next/tests/access_scoping.rs
git commit -m "WS2: add can_modify_resource write-axis gate (artifact + forward migration)

temper_next had only the two read axes. Add can_modify_resource modeled on
resources_visible_to (capability can_write, ownership; NO context-share union
— context-share is read-reach, writes are explicit). Lands in the artifact
(design-master) AND an idempotent forward migration (frozen-lineage append);
semantic drift guard stays green.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Gate NextBackend writes on `can_modify_resource`

**Why:** NextBackend mutations are ungated. Production gates via `check_can_modify` → `SELECT can_modify_resource($1,$2)` → `ApiError::Forbidden` (`resource_service.rs:518-532`). CONFORM: gate update/delete/relationship (existing-resource mutations) before any write. **Create is NOT gated** — production create sets owner=originator=caller (`next_backend.rs:182-228`; ingest has no separate context-write gate), so a fresh create is owner-implicit.

**Invariant (verbatim, spec D3):** *"NextBackend … call this gate **before any mutation** (CONFORM to the auth-before-writes rule and production's `assert_can_modify` placement), returning `Forbidden` on failure."* (Code Quality Rule: *Auth before writes — authorization checks go before any mutations.*)

**Files:**
- Modify: `crates/temper-api/src/backend/next_backend.rs` (`update_resource:240`, `delete_resource:303`, `assert_relationship:367`, `retype_relationship:446`, `reweight_relationship:469`, `fold_relationship:490`; add a private `check_can_modify_next` helper)
- Test: `crates/temper-api/tests/next_backend_access_test.rs` (create)

**Tag:** EXTEND — new gate on the write path; authorized by spec D3. CONFORM to production's `check_can_modify`.

- [ ] **Step 1: Write the failing test**

Create `crates/temper-api/tests/next_backend_access_test.rs`:

```rust
#![cfg(all(feature = "test-db", feature = "next-backend"))]
//! WS2 — NextBackend write gate. A non-owner/non-granted writer is Forbidden.

// Reuse the existing next-backend e2e harness setup used by the 4c round-trip tests
// (find it under crates/temper-api/src/backend/tests.rs or tests/). It must:
//  - synthesize a prod-shape fixture into temper_next,
//  - build a NextBackend bound to a NON-owner profile id,
//  - attempt an update_resource on a resource that profile cannot modify.

#[sqlx::test]
async fn update_resource_forbidden_for_non_owner(pool: sqlx::PgPool) {
    let fx = setup_next_backend_fixture(&pool).await; // CONFORM to 4c test setup
    let backend = fx.next_backend_for(fx.other_profile_id); // a profile with no write grant
    let cmd = fx.update_cmd_for(fx.someone_elses_resource_ref());

    let err = backend.update_resource(cmd).await.expect_err("must be Forbidden");
    assert!(
        matches!(err, temper_core::error::TemperError::Forbidden(_))
            || err.to_string().to_lowercase().contains("forbidden"),
        "non-owner update must be Forbidden, got {err:?}"
    );
}
```

> **Engineer note:** open `crates/temper-api/src/backend/tests.rs` and the 4c round-trip e2e to reuse the exact NextBackend construction (it carries `profile_id`) and the synthesized fixture. The contract: a NextBackend bound to a profile that neither owns nor has a `can_write` grant on the target resource must return `Forbidden` from `update_resource`. Map the error variant to whatever `TemperError` carries Forbidden (grep `Forbidden` in `temper-core/src/error.rs`).

- [ ] **Step 2: Run to verify failure**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-api --features "test-db next-backend" update_resource_forbidden_for_non_owner`
Expected: FAIL — the update currently succeeds (no gate).

- [ ] **Step 3: Implement the gate helper + calls**

In `crates/temper-api/src/backend/next_backend.rs`, add a private helper on `impl NextBackend` (near `resolve_new_id:163`):

```rust
/// Auth-before-writes gate (WS2): the caller (self.profile_id, a preserved prod id) must be able
/// to modify the target temper_next resource. Returns Forbidden otherwise. Runtime, schema-
/// qualified query (the temper_next macro discipline — never the query! macros here).
async fn check_can_modify_next(&self, new_id: uuid::Uuid) -> Result<(), TemperError> {
    let can: Option<bool> = sqlx::query_scalar(
        "SELECT temper_next.can_modify_resource($1, $2)",
    )
    .bind(*self.profile_id)
    .bind(new_id)
    .fetch_one(&self.pool)
    .await
    .map_err(api_err)?;
    if can.unwrap_or(false) {
        Ok(())
    } else {
        Err(TemperError::Forbidden(format!(
            "profile {} cannot modify resource {new_id}", *self.profile_id
        )))
    }
}
```

Then, in each existing-resource mutation, call it **before any write**, right after the target id is resolved. For `update_resource` (`:240`), `delete_resource` (`:303`): after the `resolve_new_id(&cmd.resource)`/equivalent that yields the temper_next id, insert `self.check_can_modify_next(new_id).await?;` before the `writes::*` mutation. For the relationship methods (`assert_relationship:367`, `retype_relationship:446`, `reweight_relationship:469`, `fold_relationship:490`), gate on the resource the edge mutation is anchored to (the source/anchor resource the command resolves) — resolve it, then `check_can_modify_next` before the edge write.

> **Engineer note:** verify `TemperError::Forbidden` exists and its shape (grep `Forbidden` in `crates/temper-core/src/error.rs`); production maps it to HTTP 403 in `ApiError`. If the variant is unit-style (`Forbidden`), drop the format arg. Each method already resolves its target id for the mutation — place the gate immediately after that resolution and before the first `writes::` call (auth-before-writes; no write-then-check).

- [ ] **Step 4: Run the test to verify it passes**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-api --features "test-db next-backend" update_resource_forbidden_for_non_owner`
Expected: PASS.

- [ ] **Step 5: Regression — 4c round-trip e2es still pass (owner writes still succeed)**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-api --features "test-db next-backend"`
Expected: PASS — the 4c round-trip tests use the owner as caller, so the gate admits them. If any fail because the test caller isn't the resource owner, that test was relying on the missing gate; give it an owning caller (cite this task).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/backend/next_backend.rs crates/temper-api/tests/next_backend_access_test.rs
git commit -m "WS2: gate NextBackend writes on can_modify_resource (auth-before-writes)

update/delete/relationship mutations check temper_next.can_modify_resource
(caller = preserved prod profile id) before any write; Forbidden otherwise.
CONFORM to production check_can_modify. Create stays owner-implicit (caller
is owner/originator), matching production ingest.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: P1 — scenario-topology wiring correctness (edge-home + completeness)

**Why:** Tasks 2/4 added the per-test assertions inline; P1 completes the wiring proof over the rich PR #129 topology — edge-home gating (the private-edge-between-public-endpoints crux) and search/neighbor scoping — so the *whole* wired read/write surface is proven, not just list + one deny.

**Invariant (verbatim, spec P1):** *"visible-set exactness … deny … edge-home gating — `neighbors` honors edge-home visibility (the private-edge-between-public-endpoints crux) … write gate denies a non-owner/non-granted writer (403)."*

**Files:**
- Modify: `crates/temper-next/tests/access_scoping.rs`

**Tag:** CONFORM — asserts the wired surface against the proven topology; no new production behavior.

- [ ] **Step 1: Write the edge-home + search scoping tests**

Append to `crates/temper-next/tests/access_scoping.rs`:

```rust
#[sqlx::test(migrations = false)]
async fn neighbors_honor_edge_home_visibility(pool: sqlx::PgPool) {
    let topo = load_access_scenario_topology(&pool, "context-share-access").await;
    // The fixture's edge "pub1~pub2(research)" has PUBLIC endpoints but a HOME that is unshared to
    // bob (yaml :81-82): alice sees the edge, bob does not, even though both endpoints are public.
    let pub1 = topo.new_id("pub1");
    let alice_neighbors = temper_next::readback::neighbors(&pool, topo.profile("alice"), pub1).await.unwrap();
    let bob_neighbors   = temper_next::readback::neighbors(&pool, topo.profile("bob"),   pub1).await.unwrap();
    let has_research = |ns: &[temper_next::readback::Neighbor]| ns.iter().any(|n| n.edge_kind == "research");
    assert!(has_research(&alice_neighbors), "alice sees the research edge (home reachable)");
    assert!(!has_research(&bob_neighbors), "bob must NOT see the research edge (home unshared)");
}

#[sqlx::test(migrations = false)]
async fn fts_and_vector_search_scope_to_principal(pool: sqlx::PgPool) {
    let topo = load_access_scenario_topology(&pool, "context-share-access").await;
    // A query term present in rdoc only: alice (share) finds it, bob (no share) does not.
    // fts_search now takes the principal after the pool: fts_search(pool, principal, query).
    let hits_alice = temper_next::readback::fts_search(&pool, topo.profile("alice"), "rdoc_unique_term").await.unwrap();
    let hits_bob   = temper_next::readback::fts_search(&pool, topo.profile("bob"),   "rdoc_unique_term").await.unwrap();
    assert!(hits_alice.contains(&topo.origin_uri("rdoc")), "alice's FTS finds rdoc");
    assert!(!hits_bob.contains(&topo.origin_uri("rdoc")), "bob's FTS must not find rdoc");
}
```

> **Engineer note:** the second test needs a query term that actually lives in `rdoc`'s title/body in the fixture. Inspect `context-share-access.yaml` for `rdoc`'s content (or the seed it embeds) and pick a real term; if the fixture's docs have no distinctive body text, add a deterministic term to the fixture doc (small, cite this task) OR assert on a title token. The neighbor edge-home test depends on `pub1`/`pub2`/the `research` edge existing in the fixture (yaml `:81-82`); confirm the loader exposes `pub1`.

- [ ] **Step 2: Run to verify (these should pass if Tasks 2–4 are correct)**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests neighbors_honor_edge_home fts_and_vector_search_scope`
Expected: PASS. If `neighbors` over-returns for bob, the neighbor query is gating only the seed, not the *edge home / neighbor endpoint* — revisit Task 2 Step 3's `neighbors` join (edge-home visibility may require gating the edge's home, mirroring production's `edge_visible_to`; if the §9 neighbor read doesn't carry edge-home, escalate per GD-5 rather than faking the assertion).

> **GD-5 escalation note:** the chunk-3 `neighbors` read is a *direct symmetric edge read* (`readback/mod.rs:560-580`) with no edge-home column projected. If edge-home gating cannot be expressed by a `resources_visible_to` join on the endpoints alone (because the leak is the *edge's home*, not the endpoints), STOP and report BLOCKED: the spec's edge-home P1 claim needs an `edge_visible_to`-equivalent over `temper_next.kb_edges` homes, which may be a Task-2 addition. Do not weaken the assertion to pass.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-next/tests/access_scoping.rs
git commit -m "WS2: P1 wiring-correctness — edge-home + search scoping over PR#129 topology

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: P2 — principal-aware production-parity extension

**Why:** Confirm no regression on *real* data: a principal-scoped `temper_next` read returns the same set as production's scoped read for the same principal. Production's topology is trivial (owner+public), so this is thin — its job is to catch a wiring regression, with P1 carrying the differential-access weight.

**Invariant (verbatim, spec P2):** *"for the actual synthesized production topology, a principal-scoped `temper_next` read returns the same row/result *set* as the production scoped read for the same principal."*

**Files:**
- Modify: `crates/temper-next/tests/parity_reads.rs`

**Tag:** EXTEND — adds a principal dimension to the existing §9 parity harness; the §9 floor's set-parity invariant is preserved.

- [ ] **Step 1: Write the principal-aware parity test**

Append to `crates/temper-next/tests/parity_reads.rs` (reuse its existing prod-shape fixture + `ResolvedIds`):

```rust
#[sqlx::test(migrations = false)]
async fn list_parity_is_principal_scoped(pool: sqlx::PgPool) {
    let ctx = reset_and_seed_prod_shape(&pool).await; // existing harness entrypoint
    let owner = ctx.prod_owner_profile_id; // preserved => same id in both schemas

    // Production scoped list (owner principal) — origin_uri set.
    let prod: std::collections::HashSet<String> = sqlx::query(
        "SELECT vb.origin_uri FROM vault_resources_browse vb \
         JOIN resources_visible_to($1) rv ON rv.resource_id = vb.id",
    )
    .bind(owner).fetch_all(&pool).await.unwrap()
    .iter().map(|r| r.get::<String,_>("origin_uri")).collect();

    // temper_next scoped list (same principal) via the wired readback.
    let next: std::collections::HashSet<String> =
        temper_next::readback::list(&pool, owner).await.unwrap()
        .into_iter().map(|r| r.origin_uri).collect();

    assert_eq!(next, prod, "principal-scoped list set must match production for the owner");
}
```

> **Engineer note:** match the existing `parity_reads.rs` setup helper name and the field exposing the production owner profile id. Production's scoped list joins `vault_resources_browse` (the view `resource_service::list_visible` uses, `resource_service.rs:249`); confirm the view + column names there. If the existing parity test already lists unscoped, this adds the scoped-by-owner variant alongside it (do not delete the unscoped one — it still proves data/projection parity).

- [ ] **Step 2: Run the test**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests list_parity_is_principal_scoped`
Expected: PASS — the prod fixture makes every active resource owned-by/visible-to the owner, so both sets coincide (and profile-id preservation makes `owner` valid in both schemas).

- [ ] **Step 3: Full regression + check**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests`
Run: `cargo make check`
Expected: both PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-next/tests/parity_reads.rs
git commit -m "WS2: P2 principal-aware parity — scoped temper_next list == production scoped list

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Final Verification (before PR)

- [ ] `cargo make check` — clean (fmt + clippy + docs + machete + TS).
- [ ] `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests` — all green (incl. drift guard).
- [ ] `SQLX_OFFLINE=true cargo nextest run -p temper-api --features "test-db next-backend"` — all green.
- [ ] `cargo make test-e2e` (and, if push-body/ingest touched — not expected here — `test-e2e-embed`).
- [ ] Confirm `cargo make prepare-next` produces **no** `.sqlx` diff (the new queries are runtime, not macros) — if it does, a macro query slipped in; convert to runtime schema-qualified per the module discipline.

## Acceptance criterion (spec)

P1 green over the scenario topology + P2 green over the synthesized production topology ⇒ the consumer-axis read/write surface of `temper_next` is access-correct ⇒ **flip-prerequisite #1 closed.**

## Out of scope (other flip-prereqs — see strategy doc)

Producer axis (`resources_accessible_to_cogmap`, WS7); `by_uri` re-addressing + MCP enrichment reads + native-id write addressing / `ResourceRef::Scoped` collapse (strategy step 2); deployed-adapter `next-backend` enable (step 3); §5-narrow hygiene; re-mint-vs-preserve *resource* ids (this plan preserves *profile* ids only).
