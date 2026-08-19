# PR A1 — `/api/search` accepts a resource bound

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a caller narrow `/api/search` to a set of resource ids, closing the `bounds_unreachable: [Resource]` shortfall at all three doors at once.

**Architecture:** The gated bound-accepting twin already exists for the exact arm (`query_find_exact`, shipped `20260810000010`, called by nothing). The wide arm needs the same eight-line wrapper. Then `SearchParams` gains a field, the CLI gains a repeatable flag, and two declarations empty.

**Tech Stack:** PostgreSQL 18 + pgvector, Rust (sqlx, clap 4, axum), cargo-nextest.

**Spec:** [`internal/superpowers/specs/2026-08-12-api-query-door-design.md`](../specs/2026-08-12-api-query-door-design.md) ⟨6⟩ — read it first; it carries the measurement this plan rests on.

## Global Constraints

- **Never scope a test with `--workspace`** — it hangs on bin-target enumeration. Always `-p <crate>`, prefer `--test <target>`.
- **Do not pipe test output through `tee`** — it reports tee's exit code, so a red gate looks green.
- **`cargo make check` must pass** before any task is claimed complete. **It is not sufficient on its own** — see the next bullet.
- **`cargo make check` does not run the schema-snapshot tests.** `crates/temper-core/tests/query_schema.rs` is `#![cfg(feature = "mcp")]` and feature-pinned. Any change to a **doc comment on a schemars-derived type** changes a serialized `description`. If you touch one, run `UPDATE_SCHEMA=1 cargo nextest run -p temper-core --features mcp --test query_schema` and commit the fixtures in the same commit. This exact gap reddened CI on the sibling PR.
- **`search_exact`'s call site is a compile-time `sqlx::query!`**, so changing its SQL requires regenerating the query cache: `cargo sqlx prepare --workspace -- --all-features`. Read the `sqlx-query-cache` skill before doing so. `search_wide`'s is a runtime `query_as` (it binds `$2::vector`, which forbids the macro) and needs no cache entry.
- **Docker Postgres must be up**: `cargo make docker-up`. `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`.
- **Migrations are additive-only on `main`** — a new `CREATE OR REPLACE FUNCTION` qualifies. Declare it `additive` in the migration header, matching the incumbent style in `migrations/20260810000010_anchor_readability_both_kinds.sql`.
- **After adding a migration, `cargo clean -p temper-migrate`** — `sqlx::migrate!` embeds `migrations/` at compile time and cargo's tracking of that directory is unreliable.

---

### Task 1: `query_find_wide`, and both arms accept a bound

**Files:**
- Create: `migrations/<timestamp>_query_find_wide.sql`
- Modify: `crates/temper-substrate/src/readback/mod.rs:1553-1575` (`search_exact`), `:1599-1620` (`search_wide`), and `ArmQuery`
- Modify: `.sqlx/` (regenerated)

**Interfaces:**
- Produces: `ArmQuery { …, pub bound_ids: Option<&'a [Uuid]> }` — consumed by Task 2.
- Produces SQL: `query_find_wide(p_principal uuid, p_emb vector, p_k int, p_bound_ids uuid[] DEFAULT NULL, p_anchor_table varchar DEFAULT NULL, p_anchor_id uuid DEFAULT NULL, p_doc_type text DEFAULT NULL, p_limit int DEFAULT NULL, p_offset int DEFAULT 0) RETURNS TABLE (resource_id uuid, vec_norm real)`.

**Grounding you can rely on** (verified at `80d5d67c`):

```sql
-- migrations/20260810000010: the pattern to mirror, in full
CREATE OR REPLACE FUNCTION query_find_exact(
    p_principal uuid, p_query text, p_bound_ids uuid[] DEFAULT NULL,
    p_anchor_table varchar DEFAULT NULL, p_anchor_id uuid DEFAULT NULL,
    p_doc_type text DEFAULT NULL, p_limit int DEFAULT NULL, p_offset int DEFAULT 0)
RETURNS TABLE (resource_id uuid, fts_norm real)
LANGUAGE sql STABLE AS $$
    SELECT c.resource_id, c.fts_norm
      FROM __temper_ungated_find_exact(
             CASE WHEN p_query IS NULL OR p_query = '' THEN NULL::uuid[]
                  ELSE ARRAY(SELECT v.resource_id FROM resources_visible_to(p_principal) v) END,
             p_query, p_bound_ids, p_anchor_table, p_anchor_id, p_principal,
             p_doc_type, p_limit, p_offset) c;
$$;

-- and the wide core it must call
__temper_ungated_find_wide(p_visible_ids uuid[], p_emb vector, p_k int,
    p_bound_ids uuid[], p_anchor_table varchar, p_anchor_id uuid,
    p_anchor_reader uuid, p_doc_type text, p_limit int, p_offset int)
```

Note two things in that pattern. The gate is computed inline and **short-circuited to `NULL` when the query is empty**, so an empty search never pays for `resources_visible_to`; the wide equivalent short-circuits on `p_emb IS NULL`. And `p_principal` is passed twice — once as the gate's argument, once as `p_anchor_reader`.

**A semantic that needs no new rule.** `__temper_ungated_find_wide` already applies `WHERE (p_bound_ids IS NULL OR v.resource_id = ANY(p_bound_ids)) AND (p_anchor_id IS NULL OR …)` — bound and anchor **compose conjunctively**. And it already branches `IF p_anchor_id IS NULL AND p_bound_ids IS NULL THEN <top-k> ELSE <exhaustive>`, so supplying a bound routes to the exhaustive path on its own. That is the wide-then-filter defect this arc retired, already served. **Do not add a branch for it.**

- [ ] **Step 1: Write the failing test**

In `crates/temper-substrate/tests/` (find the file that already exercises `search_wide` against a real DB and add beside it; if none exists, create `search_bound_ids.rs`):

```rust
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn a_resource_bound_narrows_the_wide_arm_to_the_named_set(pool: PgPool) {
    // Seed three visible resources whose bodies all match the query concept, then bound to one.
    // The assertion is set membership, not ordering: this test is about the bound reaching the
    // fragment, and `vec_norm` ordering is asserted elsewhere.
    let (principal, ids) = seed_three_matching_resources(&pool).await;

    let unbounded = readback::search_wide(&pool, Some(&embedding()), 50,
        ArmQuery { principal, bound_ids: None, ..arm() }).await.unwrap();
    assert!(unbounded.len() >= 2, "the corpus must be able to return more than the bound set");

    let bounded = readback::search_wide(&pool, Some(&embedding()), 50,
        ArmQuery { principal, bound_ids: Some(&ids[..1]), ..arm() }).await.unwrap();
    assert_eq!(
        bounded.iter().map(|h| h.resource_id).collect::<Vec<_>>(),
        vec![ids[0]],
        "a bound must narrow to exactly the named set"
    );
}
```

Write the exact-arm twin of this test in the same file, against `readback::search_exact`.

**Do not invent the seeding helper** — find how the neighbouring substrate DB tests seed resources and reuse it. If nothing suitable exists, say so in your report rather than writing a bespoke seeder.

- [ ] **Step 2: Run and confirm failure**

```bash
cargo make docker-up
cargo nextest run -p temper-substrate --features test-db --test <target> a_resource_bound_narrows
```

Expected: compile error — `ArmQuery` has no field `bound_ids`.

- [ ] **Step 3: Write the migration**

Create `migrations/<timestamp>_query_find_wide.sql` with a header in the incumbent style (declare it **additive**), and a body mirroring `query_find_exact` exactly — same wrapper shape, gate short-circuited on `p_emb IS NULL`, `p_principal` passed as both gate argument and `p_anchor_reader`.

State in the header **why the function exists and why it is not `search_wide`**: `search_wide` is the incumbent `/api/search` calls and its signature is load-bearing for callers that do not supply a bound; `query_find_wide` is the bound-accepting twin, mirroring `query_find_exact`, which shipped without its sibling.

- [ ] **Step 4: Apply it and confirm the function exists in the catalog**

```bash
cargo make db-migrate
psql "$DATABASE_URL" -c "\df query_find_wide"
```

Expected: one row, 9 arguments. **Check the catalog, not the migration file** — the file is the intent, the catalog is the fact.

- [ ] **Step 5: Add `bound_ids` to `ArmQuery` and repoint both arms**

`readback/mod.rs`: add `pub bound_ids: Option<&'a [Uuid]>` to `ArmQuery`. Repoint `search_exact`'s `query!` at `query_find_exact` (8 placeholders, `bound_ids` third) and `search_wide`'s `query_as` at `query_find_wide` (9 placeholders, `bound_ids` fourth).

Update `search_exact`'s existing comment about being a compile-time macro — it explains a choice that still holds, but it now names a different function.

- [ ] **Step 6: Regenerate the query cache**

```bash
cargo sqlx prepare --workspace -- --all-features
git status --short .sqlx | head
```

Read the `sqlx-query-cache` skill first. Expect one entry to change (the exact arm's). If more change, read them before staging.

- [ ] **Step 7: Run both tests and their neighbours**

```bash
cargo nextest run -p temper-substrate --features test-db --test <target>
```

Expected: PASS, including the pre-existing tests in that file unchanged.

- [ ] **Step 8: `cargo make check` and commit**

```bash
cargo clean -p temper-migrate
cargo make check
git add migrations crates/temper-substrate .sqlx
git commit -m "The wide arm gets the bound-accepting twin its sibling shipped without"
```

### Task 2: `SearchParams.bound_ids`, threaded to the arms

**Files:**
- Modify: `crates/temper-core/src/types/api.rs` (`SearchParams`)
- Modify: `crates/temper-services/src/backend/substrate_read.rs:1000-1020`
- Modify: `openapi.json`, `packages/temper-ui/src/lib/types/generated/*` (regenerated)

**Interfaces:**
- Consumes: `ArmQuery.bound_ids` (Task 1).
- Produces: `SearchParams.bound_ids: Option<Vec<Uuid>>` — consumed by Task 3.

- [ ] **Step 1: Write the failing test**

Add to the API-level search tests (find the existing `#[sqlx::test]` search handler tests and add beside them):

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn a_bounded_search_returns_only_the_named_resources(pool: PgPool) {
    // The wire field must reach the fragment. Asserted through the handler rather than the
    // service, because the field's whole purpose is to be reachable by a caller.
    let (principal, ids) = seed_three_matching_resources(&pool).await;
    let body = SearchParams { query: Some("...".into()), bound_ids: Some(vec![ids[0]]), ..Default::default() };
    let res = search_handler(&pool, principal, body).await.unwrap();
    let returned: Vec<Uuid> = res.exact.hits.iter().map(|h| h.resource.id).collect();
    assert_eq!(returned, vec![ids[0]]);
}
```

- [ ] **Step 2: Run and confirm failure** — compile error, no field `bound_ids`.

- [ ] **Step 3: Add the field**

```rust
    /// Narrow to a set of resource ids. Composes with `context_ref` / `cogmap_id` rather than
    /// replacing them — the fragments apply bound and anchor conjunctively.
    ///
    /// Reachable from every door: the MCP `search` tool takes this whole struct as its
    /// `Parameters`, so the field arrives there without a tool change.
    #[serde(default)]
    pub bound_ids: Option<Vec<Uuid>>,
```

Add it to the `Default` impl beside the other `None` fields.

- [ ] **Step 4: Thread it** — `substrate_read.rs` builds `ArmQuery`; pass `params.bound_ids.as_deref()`.

- [ ] **Step 5: Run the test** — expect PASS.

- [ ] **Step 6: Regenerate the router-derived artifacts**

```bash
cargo make openapi
cargo make generate-ts-types
git diff --stat openapi.json packages/temper-ui/src/lib/types/generated
```

Read the `generated-artifacts` skill. `SearchParams` carries `ts-rs` and `utoipa` derives, so both move. **Read the TS diff before staging** — ts-rs writes a dependency's file with only the types reachable from the graph being exported, and this repo has a recorded incident where a full regen silently dropped a type.

- [ ] **Step 7: `cargo make check` and commit** (artifacts in the same commit).

### Task 3: `temper search --within`

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (the `Search` variant), `main.rs` (`Commands::Search`), `commands/search_cmd.rs`, `actions/search.rs`

- [ ] **Step 1: Write the failing test**

In `actions/search.rs`'s test module, beside `the_cli_offset_reaches_search_params`:

```rust
#[test]
fn the_cli_within_refs_reach_search_params_as_ids() {
    // Refs are trailing-UUID-only, like every other ref this CLI takes — the slug half is
    // presentation and a stale one is harmless.
    let id = uuid::Uuid::now_v7();
    let params = build_search_params(CliSearchArgs {
        query: "anything",
        within: &[format!("some-stale-slug-{id}")],
        ..args()
    })
    .expect("a decorated ref resolves to its trailing uuid");
    assert_eq!(params.bound_ids, Some(vec![id]));
}
```

- [ ] **Step 2: Run and confirm failure.**

- [ ] **Step 3: Add the flag and resolve refs**

`cli.rs`, in `Search` after `--offset`:

```rust
        /// Narrow to specific resources, by ref (UUID or decorated `slug-<uuid>`). Repeatable.
        /// Composes with --context / --cogmap rather than replacing them.
        #[arg(long = "within")]
        within: Vec<String>,
```

`CliSearchArgs` gains `pub within: &'a [String]`, and `build_search_params` resolves each through the one resolver — `temper_workflow::operations::parse_ref` — mapping a parse failure to `TemperError::BadRequest` naming the offending ref. An empty vec means `None`, not `Some(vec![])`: **bounded-to-nothing and unbounded are different questions**, and the fragments distinguish them (`'{}'` returns zero rows, `NULL` is unbounded).

- [ ] **Step 4: Thread through `main.rs` and `search_cmd.rs`** — both destructure and re-bundle.

- [ ] **Step 5: Run the test and its neighbours**

```bash
cargo nextest run -p temper-cli --lib actions::search
```

- [ ] **Step 6: `cargo make check` and commit.**

### Task 4: the declarations empty, at all three doors

**Files:**
- Modify: `crates/temper-core/src/types/query/registry.rs` — two `unified_doors` calls and `no_door_can_supply_the_resource_bound_the_find_acts_accept`

- [ ] **Step 1: Write the failing test** — rename the incumbent and invert it:

```rust
    #[test]
    fn every_door_can_now_supply_the_resource_bound_the_find_acts_accept() {
        // The shortfall this axis was created to record. `/api/search` gained `bound_ids`, the MCP
        // tool takes the whole `SearchParams` so it arrived there too, and `temper search` gained
        // `--within` — so the axis empties at all three doors at once rather than door by door.
        //
        // `find-about-anywhere` still declares an empty list for the OTHER reason: it accepts no
        // bounds at all. An empty list here is "nothing to fall short on", and that distinction is
        // why the two cases are asserted separately below.
        for name in [ActName::FindExact, ActName::FindAboutWithin] {
            let a = declaration(&name).unwrap();
            assert!(a.accepts_bounds.contains(&IdKind::Resource));
            for door in Door::ALL {
                let Some(DoorReach::Serves { bounds_unreachable, .. }) = a.door_coverage.get(&door)
                else {
                    panic!("{name:?} must serve {door:?}");
                };
                assert!(
                    bounds_unreachable.is_empty(),
                    "{name:?} at {door:?} still declares {bounds_unreachable:?} unreachable"
                );
            }
        }
    }
```

Keep the `find-about-anywhere` half of the incumbent test exactly as it is — its subject is unchanged.

- [ ] **Step 2: Run and confirm failure** — the declarations still say `[IdKind::Resource]`.

- [ ] **Step 3: Empty both declarations** — `unified_doors(vec![], vec![IdKind::Resource])` becomes `unified_doors(vec![], vec![])` at both find-act sites.

- [ ] **Step 4: Correct the prose the change falsifies.** `unified_doors`' doc comment says the bounds shortfall *"genuinely is door-independent: the shipped arms hard-bind `NULL` for bound-ids, so no caller anywhere can supply a resource bound."* No longer true. Rewrite it to say the axis is now empty everywhere and what the parameter is still for. **Check the surrounding comments on both edited declarations for the same claim** — this repo treats a stale comment as a real defect, and the sibling PR spent three review rounds on exactly this.

- [ ] **Step 5: Run the registry suite and the door-coverage gates**

```bash
cargo nextest run -p temper-core --lib types::query::registry
cargo nextest run -p temper-cli --test act_door_coverage_cli_terms
cargo nextest run -p temper-core --test act_door_coverage_reachability
```

- [ ] **Step 6: `cargo make check` and commit.**

---

## What this plan does NOT do

- **No door.** `POST /api/query` is PR B.
- **No composition.** A resource bound is a filter on one act. Multi-anchor, piping and combinators stay `/api/query`'s.
- **No change to `unified_doors`' signature.** ⟨6⟩'s whole point is that the per-door argument never becomes necessary.
- **No `follow-from` / `survey` work** — they remain `DoorReach::Absent`, refused statically.

## Declared risk

**The exact arm's ordering is not asserted by Task 1's tests**, which check set membership. `search_exact` orders by `fts_norm` and a bound does not change the ordering expression — but the tests would not catch it if it did. Ordering is asserted elsewhere in the substrate suite; this plan does not extend that coverage, and says so rather than implying the bound is fully witnessed.
