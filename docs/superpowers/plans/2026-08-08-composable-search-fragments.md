# Composable Search Fragments Implementation Plan

> ## `[ALL TEN TASKS EXECUTED — 2026-08-08. This plan is history, not instruction.]`
>
> Tasks 0–10 landed on `jct/composable-search-fragments` (local, unpushed, no PR). Do not execute
> from this plan again; read it to understand what the branch contains.
>
> **Two things a later reader needs, neither of which is in the task text below.**
>
> 1. **The wire contract is now [`docs/api/query.openapi.yaml`](../../api/query.openapi.yaml).**
>    Task 4 bound the compiler to the twins against the *merged* type shapes, and the contract amends
>    six of them. Task 5 made `find-exact`'s declaration true against those same shapes.
>
> 2. **Task 4 has a defect the contract found.** `narrowing_for` routes every upstream set to
>    `p_bound_ids` and reads `bounds_mode` nowhere, so a `seed` compiles as a `bound`. Harmless
>    while `follow-from` emits `__temper_unbound_act`; wrong the moment it does not.
>
> **What this plan did NOT resolve, and what the branch still carries** — recorded here because the
> session that ran Tasks 7–9 stored no note:
>
> - The decision gate (Task 6) cleared the array path on ONE query shape and returned PROCEED. A
>   falsifier then fired that its own evidence document did not list — a call whose body returns zero
>   rows — costing `/api/search` ~1.2 ms → ~4.1 ms on text-only queries. It was patched with a `CASE`
>   in `query_find_wide` rather than re-opening the gate.
> - **That patch is asymmetric and the exact arm still has the hole.** `query_find_exact` builds the
>   visible-set array unconditionally while its core opens `WHERE p_query IS NOT NULL AND p_query <> ''`
>   — and `SearchParams.query` is optional, with `search_params_deserializes_embedding_only` proving a
>   query-less search is expressible.
> - `p_anchor_reader` appears nowhere in the spec or this plan. The "ungated" core takes a principal
>   after all, so anchor readability is still checked per call. `[adjudicated — 2026-08-10, P4/ADJ-1]`
>   The parameter is now authorized with a stated charter: it exists for anchor readability only,
>   covering BOTH anchor kinds (cogmap and context), and must never gain another use.
> - `p_visible_ids` NULL means *admit nothing* while `p_bound_ids` NULL means *unbounded* — two
>   same-typed parameters on one function with opposite NULL semantics.
> - Spec §9's trigger-maintained `profile_reachable_teams` closure table was **measured on
>   2026-08-09** `[amended — 2026-08-10]` — spec §9 now carries the result: the closure's share of
>   gate cost is second-order at current scale (1.0–1.9% here, ~40% at a large synthetic tenant),
>   and §9's measured conclusion is that Tasks 7–9 do **not** become unnecessary. The table remains
>   unbuilt, deferred on current need.

**Goal:** Give `/api/query` find-act fragments that can be bounded by an upstream id set, by extracting the deployed arms' shared interiority and building composable twins beside them — without changing either deployed signature.

**Architecture:** Three migrations in sequence. The first extracts repeated predicates into views and refactors `search_exact`/`search_wide` onto them (behaviour-preserving, provable against the existing suite). The second moves those now-clean bodies into `query_find_exact`/`query_find_wide`, adds `p_bound_ids uuid[]`, and re-points the incumbents as delegating wrappers — one body per arm. The third splits each arm into a gated wrapper and an ungated core so a composition computes visibility once, and is **conditional on a measured plan comparison** (Task 6).

**Tech Stack:** PostgreSQL 17/18 + pgvector, sqlx migrations, Rust (temper-core validator, temper-substrate compiler), bash CI tripwire.

**Spec:** `docs/superpowers/specs/2026-08-08-composable-search-fragments-design.md`. Read it. This plan is an index and sequence over it, not a replacement — each task cites the section its implementer must read.

## Global Constraints

Verbatim from the spec and `CLAUDE.md`. Every task's requirements implicitly include these.

- **`search_exact` and `search_wide` signatures MUST NOT change.** `DROP`/`CREATE` is shape-breaking and needs an operator cutover; `CREATE OR REPLACE` at an unchanged signature is additive and auto-deploys. (spec §1)
- **Every migration ends with `SELECT declare_migration(<version>, '<class>', '<description>');`** — see `migrations/20260807000010_single_team_closure_per_gate_call.sql` for the shape. Silence fails CI.
- **After adding or editing a migration: `cargo clean -p temper-migrate`.** `sqlx::migrate!` embeds `migrations/` at compile time and cargo's tracking of that directory is unreliable.
- **No part of this design may assume a gate cache.** `[decided — 2026-08-07, Pete]` (spec §8)
- **The visibility-hoist rule stays unadopted**, owned by `019fddc6`. This plan does not choose materialize-vs-inline; Task 6 measures one specific plan comparison and nothing more. (spec §9)
- **Never scope a test with `--workspace`** — it is both slow and a known nextest hang. Always `-p <crate>`, and prefer `--test <target>`.
- **CI owns the broad suites.** Run the tests you wrote, their neighbours, and anything regenerating a committed artifact. Do not run `cargo make test-all` / `test-e2e` locally.
- **GD-3 tagging is required.** Every task below carries CONFORM / EXTEND / AMEND. Any code block is either quoted from disk with a `file:line` citation or tagged EXTEND against a spec section — per the temper skill's GD-4, an *invented* body in a plan wins over the correct prose beside it, so there are none here.

## File Structure

**Created:**
- `migrations/20260808000020_search_arm_shared_interiority.sql` — the two views, `shrunk_best_of_n`, and both incumbents refactored onto them.
- `migrations/20260808000030_composable_find_family.sql` — `query_find_exact` / `query_find_wide` with incumbents as delegating wrappers, plus the gated-wrapper / ungated-core split (**conditional on Task 6**). (A separate `20260808000030_composable_find_fragments.sql` was planned but never shipped — it was folded into this family migration.)
- `.github/scripts/audit-ungated-fragments.sh` — **conditional on Task 6.** Derived-set tripwire.

**Modified:**
- `crates/temper-substrate/tests/search_exact_and_wide.rs` — new witnesses; the `ef_search` pin test rederived.
- `crates/temper-substrate/src/readback/query_plan.rs` — real act fragments; the single ungated-core emitter.
- `crates/temper-core/src/types/query/validate.rs:37` — `CALLABLE_FRAGMENTS` set → map.
- `crates/temper-core/src/types/query/registry.rs:115-130` — `find-exact` bound kinds and its stale rationale.

---

### Task 0: A corpus a measurement can be taken on `[added — 2026-08-08]`

**GD-3: EXTEND** — new affordance on the existing access-scenario fixture model. Not in the
original plan, and the plan was wrong to omit it: Tasks 2 and 6 are both *measurements*, and the
session that wrote this plan recorded that "a near-empty local corpus cannot answer this" without
filing the work that would fix it. **A measurement task with no corpus is a declared hole, not a
step.**

**Files:**
- Modify: `crates/temper-substrate/src/scenario/access/model.rs` — `AccessWorld.populations`
- Create: `crates/temper-substrate/src/scenario/access/population.rs`
- Create: `crates/temper-substrate/tests/fixtures/access-scenarios/measurement-corpus.yaml`
- Create: `crates/temper-substrate/tests/measurement_corpus.rs`
- Modify: `crates/temper-substrate/src/main.rs` — `seed-corpus` subcommand; `Makefile.toml`

**What it is.** The `AccessWorld` fixture already declares the whole visibility topology — teams +
DAG, memberships, contexts, shares, homes, grants — and is schema-backed and loader-validated. What
it cannot do is *scale* (every resource is hand-keyed) or produce search-path rows (grep found zero
references to `kb_chunks` / `kb_resource_search_index` under `scenario/`). So the topology stays
hand-authored and the bulk is generated from a declared distribution, through the **product write
path** (`SeedAction::ResourceCreate`, whose projector writes blocks, chunks, the `doc_type` property
and `_rebuild_resource_search_vector`). One declaration, loaded at declared size by a
`#[sqlx::test]` and at `--scale`× by the seeder binary, so the test-sized and measurement-sized
corpora cannot drift.

**Three properties, each asserted, because a degenerate corpus is SILENT** — every downstream
measurement reads green against a corpus that cannot answer the question:

1. **Clustered vectors.** Uniform 768-dim draws concentrate: all pairwise distances collapse into a
   narrow band and ANN ranking becomes arbitrary. Measured during the build, before the fix:
   within-topic mean cosine distance **0.9961** against cross-topic **1.0002** — a separation of
   0.004, i.e. none. The cause is that a unit vector's components are ~N(0, 1/768) ≈ 0.036, so
   per-dimension noise at 0.55 is fifteen times the signal. `topic_spread` is therefore a *magnitude
   ratio*, not a per-dimension sigma.
2. **Uneven, partial visibility.** Nothing is granted to the root team (down-only inheritance would
   hand everyone everything). Measured at scale 1: ana 66.5%, ben/dev/cara 33.5%, nomad ~0, with
   81/41/41 rows arriving via the **team-grant arm** — the arm that is empty on the deployment whose
   97.6% figure this corpus exists to replace.
3. **Both arms reachable.** Every generated resource carries an embedded chunk and an FTS vector.

**Determinism is load-bearing, not a nicety:** Task 2 compares plans captured before and after a
refactor, and a corpus that differed between captures would make the diff measure the corpus.
Seeded SplitMix64, no `rand` dependency.

**Two things it changed outside itself, both deliberate:**
- `loader::insert_teams` now reconciles `ON CONFLICT (slug)`. Every access fixture must declare
  `temper-system` (the DAG parents reference it) but migration `20260625000001` already creates that
  team — so the loader could only ever run against a schema that was reset first, i.e. never against
  a real migrated database.
- The grant insert was **extracted** to `loader::insert_resource_grants` rather than copied into the
  generator. `audit-grant-sinks.sh` counts write-sites per file; a copy would have needed a baseline
  bump, and that script's own header records why that is corrosive — absorbing a movement into the
  baseline "teaches the next reader that the number moves for cosmetic reasons, which is how a
  tripwire stops being read."

**Sequencing consequence — Task 2 no longer needs volume cycling.** The plan's Task 2 steps 1–2 call
for `docker-down-volumes` twice, which was only necessary because reseeding was not repeatable. With
a committed seeder the order is: seed → capture "before" plans → apply Task 1's migration →
re-capture. Nothing is destroyed and both captures run on a byte-identical corpus, which is a
stronger comparison than the original procedure could give.

```bash
cargo make docker-down-volumes && cargo make docker-up
cargo make seed-corpus 20        # ~4800 resources; scale 1 is the test-asserted size
cargo nextest run -p temper-substrate --features artifact-tests --test measurement_corpus
```

---

### Task 1: The shared interiority — two views and one scoring function

**GD-3: EXTEND** (new objects) with bodies CONFORM to existing predicates. Cites `migrations/20260806000020_search_arms_paging_and_doctype.sql:108-112,:157-158,:177-180`. Spec §3.

**Files:**
- Create: `migrations/20260808000020_search_arm_shared_interiority.sql`
- Test: `crates/temper-substrate/tests/search_exact_and_wide.rs`

**Interfaces:**
- Produces: view `kb_resource_doc_type(resource_id uuid, doc_type text)`; view `kb_resources_live(id uuid)`; function `shrunk_best_of_n(p_min double precision, p_avg double precision, p_n bigint) RETURNS real`.

- [ ] **Step 1: Read the spec section and the source predicates**

Read spec §2.1, §2.2 and §3 — they contain the measurement that decided view-not-function, and you must not re-derive it. Then read `migrations/20260806000020_search_arms_paging_and_doctype.sql:84-241` in full. The three predicates you are extracting are at `:108-112` (doc_type, ×3 across the file), `:103-104`/`:157-158`/`:203-204` (live-row, ×3), `:177-180` and `:229-231` (shrunk score, ×2).

- [ ] **Step 2: Create the migration with the two views**

The `doc_type` view body is quoted from `20260806000020:108-112` with the correlated `p.owner_id = r.id` lifted to a projected column:

```sql
CREATE VIEW kb_resource_doc_type AS
    SELECT p.owner_id AS resource_id,
           p.property_value #>> '{}' AS doc_type
      FROM kb_properties p
     WHERE p.owner_table = 'kb_resources'
       AND p.property_key = 'doc_type'
       AND NOT p.is_folded;
```

The live-row view is **id-only, deliberately**. Do NOT write `SELECT r.*`: PostgreSQL expands `*` at view-creation time, so a column added to `kb_resources` later would silently not appear — a view that looks like it tracks the table and does not.

```sql
CREATE VIEW kb_resources_live AS
    SELECT r.id
      FROM kb_resources r
     WHERE r.is_active
       AND r.ingest_state = 'complete';
```

- [ ] **Step 3: Add the scoring function**

Body quoted from `20260806000020:177-180`. IMMUTABLE and called on already-aggregated values — once per group, not once per row — so spec §2.1's non-inlining hazard does not apply here.

```sql
CREATE FUNCTION shrunk_best_of_n(
    p_min double precision,
    p_avg double precision,
    p_n   bigint)
RETURNS real
LANGUAGE sql IMMUTABLE AS $$
    SELECT (1.0 - (p_min + (p_avg - p_min) * (1.0 - 1.0 / sqrt(p_n::float8))) / 2.0)::real
$$;
```

- [ ] **Step 4: Refactor both incumbents onto them**

`CREATE OR REPLACE FUNCTION search_exact(...)` and `search_wide(...)` at **byte-identical signatures**, with each of the three predicate families replaced by a reference to the new objects. Take the full bodies from `20260806000020:82-241` and change only those references. Do NOT extract the anchor `EXISTS` or the anchor readability gate — spec §3 states why both are deliberately left in place, and `search_wide`'s guard-clause form returns without scanning where a conjunct could not.

- [ ] **Step 5: Declare the migration**

```sql
SELECT declare_migration(
    20260808000020,
    'additive',
    'Extracts the three predicate families search_exact and search_wide each carried multiple copies of — the doc_type property lookup (3 copies), the is_active/ingest_state live-row pair (3), and the shrunk best-of-N score (2) — into two views and one IMMUTABLE function, and rewrites both arms onto them at unchanged signatures. Extraction is by VIEW and not by scalar function because a LANGUAGE sql STABLE predicate whose body contains a sublink does not inline: measured, the doc_type EXISTS loses its Index Only Scan on uq_kb_properties_active and becomes a per-row call, while the view form is plan-identical to the incumbent. Additive: two CREATE VIEW, one CREATE FUNCTION, and CREATE OR REPLACE on two existing functions at unchanged signatures, no DROP. Row sets and scores are unchanged, which the existing search_exact_and_wide suite asserts without edit.'
);
```

- [ ] **Step 6: Rebuild the migration crate and apply**

```bash
cargo clean -p temper-migrate
cargo make docker-up
```
Expected: `migrations applied; outcomes recorded in kb_migration_ledger`.

- [ ] **Step 7: Run the existing suite UNEDITED — this is the deliverable**

```bash
cargo nextest run -p temper-substrate --features artifact-tests --test search_exact_and_wide
```
Expected: PASS, with **no test file changes**. That the suite needed no edit is the evidence that behaviour is preserved; if you find yourself editing a test here, stop — you have changed behaviour, and the whole reason this is a separate migration has been lost.

- [ ] **Step 8: Add witnesses for what the views filter**

Two tests asserting the views' own predicates, so the invariants have a home rather than being held by the arms that use them: that `kb_resources_live` excludes both a soft-deleted resource and an `in_progress` one, and that `kb_resource_doc_type` excludes a folded property row. Follow the file's existing `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]` harness and its `bootseed` setup helpers.

- [ ] **Step 9: Run the new tests**

```bash
cargo nextest run -p temper-substrate --features artifact-tests --test search_exact_and_wide
```
Expected: PASS, including the two new tests.

- [ ] **Step 10: Commit**

```bash
git add migrations/20260808000020_search_arm_shared_interiority.sql crates/temper-substrate/tests/search_exact_and_wide.rs
git commit -m "Extract the search arms' shared interiority into views, not functions"
```

---

### Task 2: Prove plan-identity, don't assume it

**GD-3: CONFORM** — the load-bearing constraint is that Task 1 changed no plan. Spec §2.2.

**Files:**
- Create: `docs/superpowers/plans/evidence/2026-08-08-task1-plan-identity.md` (recorded output)

- [ ] **Step 1: Capture the incumbent plans BEFORE the refactor**

```bash
git stash list  # ensure clean; then, on the commit BEFORE Task 1:
git checkout HEAD~1 -- migrations/
cargo clean -p temper-migrate && cargo make docker-down-volumes && cargo make docker-up
```
Then `EXPLAIN (VERBOSE, COSTS OFF)` a call to each arm — one `search_exact` with a `p_doc_type`, one `search_wide` unscoped, one `search_wide` scoped — and save the three plans.

- [ ] **Step 2: Restore Task 1 and capture the same three plans**

```bash
git checkout HEAD -- migrations/
cargo clean -p temper-migrate && cargo make docker-down-volumes && cargo make docker-up
```
Re-run the identical three `EXPLAIN`s.

- [ ] **Step 3: Diff and record**

Expected: identical node types, join orders, index conditions. A cost-estimate difference on an empty corpus is not a finding; a **missing Index Only Scan or a new per-row Filter is**. Save both sets to the evidence file with the commands that produced them.

- [ ] **Step 4: Escalate if they differ**

If any plan lost an index path, STOP and report BLOCKED. Do not proceed to Task 3 — spec §2.1 is the measurement saying this is the failure mode to watch for, and carrying it forward would bury a regression under three more migrations.

- [ ] **Step 5: Commit the evidence**

```bash
git add docs/superpowers/plans/evidence/2026-08-08-task1-plan-identity.md
git commit -m "Evidence: the interiority extraction is plan-identical on all three arm shapes"
```

---

### Task 3: The composable twins

**GD-3: EXTEND** — new affordance authorized by spec §4. Bodies are **moved** from Task 1's output, not authored.

**Files:**
- Create: `migrations/20260808000030_composable_find_fragments.sql`
- Test: `crates/temper-substrate/tests/search_exact_and_wide.rs`

**Interfaces:**
- Consumes: Task 1's `kb_resource_doc_type`, `kb_resources_live`, `shrunk_best_of_n`.
- Produces: `query_find_exact(p_principal uuid, p_query text, p_bound_ids uuid[], p_anchor_table varchar, p_anchor_id uuid, p_doc_type text, p_limit int, p_offset int) RETURNS TABLE(resource_id uuid, fts_norm real)`; `query_find_wide(p_principal uuid, p_emb vector, p_k int, p_bound_ids uuid[], p_anchor_table varchar, p_anchor_id uuid, p_doc_type text, p_limit int, p_offset int) RETURNS TABLE(resource_id uuid, vec_norm real)`.

- [ ] **Step 1: Read spec §4 in full**

Three semantics carry weight and none is inferable from the signature: a bound set is a scope, empty is not absent, and the `ef_search` pin does not inherit to a directly-called twin.

- [ ] **Step 2: Create `query_find_exact` by moving the body**

Move Task 1's `search_exact` body verbatim into the new function, adding one conjunct in the inner subquery's `WHERE` — **beneath** the `ORDER BY`/`LIMIT`, which is the whole point:

```sql
AND (p_bound_ids IS NULL OR r.id = ANY(p_bound_ids))
```

- [ ] **Step 3: Create `query_find_wide`, and make the bound set select the exhaustive branch**

Move Task 1's `search_wide` body. Change the branch predicate from `IF p_anchor_id IS NULL THEN` to:

```sql
IF p_anchor_id IS NULL AND p_bound_ids IS NULL THEN
```

so a bounded call takes the **exhaustive** branch, which has no top-k to defeat. Add the same `p_bound_ids` conjunct to `scoped_res`. Spec §4: the fork `search_wide` already carries *is* the fork the rank-shaped rule describes.

- [ ] **Step 4: Give `query_find_wide` its own `ef_search` pin**

`proconfig` binds to a signature and does not inherit. Copy the pin block from `20260806000020:243-266` **including the `_PG_init` vector warmup that precedes it** — the header there records that without the warmup the `SET` fails with `permission denied to set parameter "hnsw.ef_search"`, which is how `20260804000030` failed.

- [ ] **Step 5: Re-point the incumbents as delegating wrappers**

`CREATE OR REPLACE` both at unchanged signatures, each delegating with `p_bound_ids => NULL`. After this there is **one body per arm**.

- [ ] **Step 6: Declare and apply**

Write the `declare_migration(20260808000030, 'additive', …)` call, then:
```bash
cargo clean -p temper-migrate && cargo make docker-up
```

- [ ] **Step 7: Write the four new witnesses**

Each asserts one thing spec §4 states, and each must fail against a twin that ignores `p_bound_ids`:
1. A bounded call returns only ids in the bound set.
2. `p_bound_ids = '{}'` returns **zero rows**; `p_bound_ids => NULL` returns the unbounded set. These must be separate assertions — collapsing them is the substitution delta 3 forbids.
3. A bounded `query_find_wide` returns a resource that a top-k draw would have crowded out — the witness that the exhaustive branch was taken. Seed enough chunks that `p_k` is genuinely binding, or the test is vacuous.
4. `search_exact`/`search_wide` return exactly what they returned before this migration.

- [ ] **Step 8: Rederive the `ef_search` pin test**

`crates/temper-substrate/tests/search_exact_and_wide.rs:545-570` currently reads `WHERE proname = 'search_wide'`. Change it to derive the set — every function that carries an ANN draw must pin `hnsw.ef_search` at or above the `k` it is asked for — and assert over **all** of them. Keep the existing `pin >= k` form; the file's own comment at `:542-543` explains why a value assertion would be wrong.

- [ ] **Step 9: Run the suite**

```bash
cargo nextest run -p temper-substrate --features artifact-tests --test search_exact_and_wide
```
Expected: PASS, including all new tests. Confirm the rederived pin test **fails** if you temporarily drop `query_find_wide`'s pin — a pin test that cannot fail is the trap this task exists to avoid.

- [ ] **Step 10: Commit**

```bash
git add migrations/20260808000030_composable_find_fragments.sql crates/temper-substrate/tests/search_exact_and_wide.rs
git commit -m "Composable find twins: a bound set is a scope, and empty is not absent"
```

---

### Task 4: Teach the compiler the real fragments

**GD-3: AMEND** — replaces Task 9's placeholder bodies. Cites `crates/temper-substrate/src/readback/query_plan.rs:82-103` and spec §7.

**Files:**
- Modify: `crates/temper-substrate/src/readback/query_plan.rs:46-48,:82-103`
- Modify: `crates/temper-core/src/types/query/validate.rs:37`

**Interfaces:**
- Consumes: Task 3's `query_find_exact` / `query_find_wide`.
- Produces: `CompiledQuery` whose find-act CTEs target real functions.

> **⚠️ Plan/reality gap — `compile` cannot reach an embedding today.** `Composition.intention` is
> `Intention { query: String, embedded: bool }` (`composition.rs:26-31`) — it carries the *fact* that
> an embedding was computed, **not the vector**. But `query_find_wide` requires `p_emb vector`, and
> `compile(v: &ValidatedComposition, principal: ProfileId)` (`query_plan.rs:52`) has no third
> argument. `QueryBind::Embedding(Vec<f32>)` already exists (`:42`) and is currently unconstructed.
> So this task must widen `compile` to accept the caller-computed embedding as `Option<&[f32]>`, and
> a `find-about-*` stage with `None` must refuse rather than bind NULL — spec: *"a `find-about-*`
> stage with no intention refuses, rather than the server embedding on the caller's behalf. That is
> what makes 'I chose not to embed' and 'I cannot embed' different states instead of one ambiguous
> one."* Do not add the vector to `Intention`: it is a wire type, and a 768-float array in the
> envelope is a contract change nobody asked for.

- [ ] **Step 1: Turn `CALLABLE_FRAGMENTS` into a map**

It is currently `const CALLABLE_FRAGMENTS: &[&str] = &["search_graph_expand", "wayfind_region_scores"];` (`validate.rs:37`). It must become a mapping from **declared mechanic** (`search_exact`, what `/api/search` calls and what `served_by` names) to **emitted fragment** (`query_find_exact`). Spec §7: `served_by` must keep naming the deployed door's function. Preserve the existing doc comment's argument that the set is keyed on served-by names and never on `build_state`.

- [ ] **Step 2: Write the failing compiler test**

A test asserting a `find-about-within` stage compiles to a CTE referencing `query_find_wide`, that every caller value is a positional bind, and that no literal uuid appears in the SQL. Follow the existing emission tests in `query_plan.rs`'s test module.

- [ ] **Step 3: Run it and confirm it fails**

```bash
cargo nextest run -p temper-substrate query_plan
```
Expected: FAIL — the emitted body still targets `__temper_unbound_act`.

- [ ] **Step 4: Emit the real fragments**

Replace `emit_act_body`'s placeholder for the three find acts. Bind the intention's query text and embedding, the bound ids, and the act's `terms`. **Leave `__temper_unbound_act` in place for `follow-from` and `survey`** — those are a later task, and a placeholder that fails loudly is correct until then.

- [ ] **Step 5: Run the tests**

```bash
cargo nextest run -p temper-substrate query_plan
cargo nextest run -p temper-core --lib query
```
Expected: PASS. The 99 existing query unit tests must stay green.

- [ ] **Step 6: Flip the reachability test to green**

`validate.rs`'s `a_served_act_this_builder_has_no_fragment_for_refuses_honestly` was written expecting the find acts to be statically refused. With fragments bound they are reachable; update it to assert refusal for an act that genuinely still has none (`substantiate`, whose `resource_standing_shape` this builder does not emit).

- [ ] **Step 7: Commit**

```bash
git add crates/temper-substrate/src/readback/query_plan.rs crates/temper-core/src/types/query/validate.rs
git commit -m "Bind the find acts to their composable fragments"
```

---

### Task 5: Make the declarations true

**GD-3: AMEND** — cites `crates/temper-core/src/types/query/registry.rs:115-130` and spec §7.

**Files:**
- Modify: `crates/temper-core/src/types/query/registry.rs:115-130,:166`

- [ ] **Step 1: Correct `find-exact`'s bound kinds**

It declares `accepts_bounds: vec![IdKind::Resource]` (`:118`) — the one kind its fragment could not take — and omits `Context`/`Cogmap`, which the anchor pair does take. After Task 3 all three are true. Add the two missing kinds.

- [ ] **Step 2: Correct the stale rationale**

The comment at `:115-117` reads *"The exact arm carries no top-k, so nothing can be crowded out of it and where the bound is applied cannot change WHICH resources come back."* That was true of `20260805000020` and is **false** since `20260806000020` added `LIMIT p_limit`. Replace it with what is now true — the bound is applied beneath the `ORDER BY`, so position is again immaterial, but for a different reason.

- [ ] **Step 3: Update the affected registry tests**

`find_about_anywhere_accepts_no_bounds_by_definition` and any test asserting `find-exact`'s exact bound set. Do NOT weaken the exact-set assertions to `contains` — `exactly_survey_and_follow_from_are_relative_in_domain` records why (`:608-609`): a `contains` would admit the very drift the test exists to catch.

- [ ] **Step 4: Run and regenerate**

```bash
cargo nextest run -p temper-core --lib query
UPDATE_SCHEMA=1 cargo make test-schema
cargo make generate-ts-types
```
Read the `generated-artifacts` skill first — the schema regen must be package-scoped (`-p temper-substrate`), never `--workspace`.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/query/registry.rs
git add -A crates/temper-substrate/tests packages/temper-ui/src/lib/types/generated
git commit -m "A declaration describes its mechanic: find-exact's bound kinds are now true"
```

---

### Task 6: The decision gate — measure before splitting the gate

**GD-3: CONFORM** — the constraint is PR #659's finding that `= ANY` forms no equivalence class. Spec §5.

**This task can abort the remaining tasks. That is its purpose.**

> `[noted — 2026-08-10]` The gate ran and returned PROCEED (see
> `evidence/2026-08-08-task6-gate-shape.md`). A falsifier not on its list later fired and was
> patched — see the header block at the top of this plan and the evidence doc's amendment. The
> step checkboxes below were never ticked and are left as-is.

**Files:**
- Create: `docs/superpowers/plans/evidence/2026-08-08-task6-gate-shape.md`

- [ ] **Step 1: Read spec §5's final paragraph and §7's fallback**

The core must receive the visible set as `uuid[]` because a CTE cannot be passed to a function, which puts `/api/search` on an array path where it currently has a join.

- [ ] **Step 2: Write both candidate shapes by hand**

Two hand-written queries over the same corpus: the incumbent's `JOIN resources_visible_to(p) v ON v.resource_id = r.id`, and the core's form `JOIN unnest($1::uuid[]) AS v(id) ON v.id = r.id`. **Use `unnest`, not `= ANY`** — spec §5 states why.

- [ ] **Step 3: `EXPLAIN (ANALYZE, BUFFERS)` both, repeatedly**

Single samples are not measurements. Repeat and report a distribution. Note that a near-empty local corpus cannot answer this — seed a corpus of at least a few thousand resources, or run against a restored snapshot.

- [ ] **Step 4: Decide, and record the decision with its numbers**

If the `unnest` form is within noise of the join, proceed to Task 7. If it regresses, **stop here**: Tasks 7–9 are cancelled, spec §7's fallback applies (parts 1 and 2 land, the twins gate internally, no ungated function is minted, and Task 9's tripwire is unnecessary because it guards functions that were never created). Record which happened and why.

- [ ] **Step 5: Commit the evidence either way**

```bash
git add docs/superpowers/plans/evidence/2026-08-08-task6-gate-shape.md
git commit -m "Evidence: whether the ungated core's array path regresses /api/search"
```

---

### Task 7: The ungated cores — CONDITIONAL on Task 6

**GD-3: EXTEND** — authorized by spec §5.

**Files:**
- Create: `migrations/20260808000030_composable_find_family.sql`

**Interfaces:**
- Produces: `__temper_ungated_find_exact(p_visible_ids uuid[], …)`, `__temper_ungated_find_wide(p_visible_ids uuid[], …)`.

- [x] **Step 1: Move each twin's body into an ungated core**

The core applies **no** visibility gate — it joins `unnest(p_visible_ids)` where the twin joined `resources_visible_to(p_principal)`. Everything else is unchanged.

- [x] **Step 2: Name for the hazard**

`__temper_ungated_`, following the `__temper_unbound_act` convention (`query_plan.rs:48`). Spec §5: `_private` reads as "internal detail" and invites a caller; `__temper_ungated_` cannot be misread.

- [x] **Step 3: Make the twins gated wrappers**

`query_find_exact(p_principal, …)` computes `ARRAY(SELECT resource_id FROM resources_visible_to(p_principal))` and calls the core. The full chain is then `search_exact` → `query_find_exact` → `__temper_ungated_find_exact`, so `/api/search` also routes through the core — spec §5 states this explicitly and states that the alternative forfeits one-body-per-arm.

- [x] **Step 4: Comment each core with what it does not do**

A `COMMENT ON FUNCTION` stating plainly that it applies no visibility gate, that its caller must supply an RBAC verdict, and that the CI tripwire is source discipline and not a database permission.

- [x] **Step 5: Declare, apply, and run the suite**

```bash
cargo clean -p temper-migrate && cargo make docker-up
cargo nextest run -p temper-substrate --features artifact-tests --test search_exact_and_wide
```
Expected: PASS with no test edits — the public behaviour of both arms is unchanged.

- [x] **Step 6: Commit**

---

### Task 8: One emitter, so there is no wrong set to pass — CONDITIONAL on Task 6

**GD-3: EXTEND** — authorized by spec §6.

**Files:**
- Modify: `crates/temper-substrate/src/readback/query_plan.rs`

- [x] **Step 1: Write the failing test**

Assert that every ungated-core call in a compiled statement takes its ids from `vis` and from nothing else — over a composition with several stages, including one whose upstream is another stage.

- [x] **Step 2: Run it and confirm it fails**

```bash
cargo nextest run -p temper-substrate query_plan
```

- [x] **Step 3: Implement the single emitter**

One function emits ungated-core calls, and **the id source is not a parameter of it** — `vis` is fixed inside. Spec §6: this is the failure the tripwire cannot see (right place, wrong argument), closed structurally so there is no wrong set to pass.

- [x] **Step 4: Run the tests, then commit**

---

### Task 9: The tripwire — CONDITIONAL on Task 6

**GD-3: EXTEND** — authorized by spec §6.

**Files:**
- Create: `.github/scripts/audit-ungated-fragments.sh`
- Modify: `.github/workflows/code-quality.yml`

- [x] **Step 1: Read two existing tripwires**

`.github/scripts/audit-grant-sinks.sh` (closest analogue) and `audit-sqlx-macro-exceptions.sh`. Follow their structure and failure output; do not invent a new format.

- [x] **Step 2: Derive the set, do not pin a list**

`rg` the `__temper_ungated_` prefix across `migrations/` and `crates/`, and assert the resulting **set** against a reviewed corpus. Spec §6 cites the repo's own lesson from `assert_every_compiled_in_doc_is_vetoed`: a hand-maintained enumeration rots, a derived set does not.

- [x] **Step 3: Verify it fails closed**

Add a call site in a scratch file and confirm the script exits non-zero naming it. A tripwire not observed failing is not a tripwire.

- [x] **Step 4: Wire it into CI and commit**

Add it to `code-quality.yml` beside the other audits. **Do not add an `-E` filter to any test job.**

---

### Task 10: Regenerate, check, and close out

- [x] **Step 1: Regenerate the sqlx caches**

Read the `sqlx-query-cache` skill first — the workspace ritual does **not** cover test-target queries, and this plan added queries in both places.

```bash
cargo sqlx prepare --workspace -- --all-features
```

- [x] **Step 2: Full check**

```bash
cargo make check
```
Expected: green — fmt, clippy, docs, machete, openapi, ts-rs drift, skills drift.

- [x] **Step 3: Verify the artifact gates specifically**

```bash
cargo make test-schema
cd packages/temper-ui && bun run check
```
`cargo make check` does **not** cover temper-ui; a generated-type change that breaks a UI fixture passes it and fails only in CI.

- [ ] **Step 4: Commit and open the PR**

Title: `Composable find fragments: one body per arm, and a bound set is a scope`. The PR body must state which branch Task 6 took and why.

---

## What this plan does NOT do

Named so the next reader does not look for it:

- **`follow-from` and `survey` are not bound.** They keep `__temper_unbound_act`. Their fragments have unmodeled arguments (`p_lens`, `p_depth`, `p_gamma` — spec §9) and binding them needs those answered first.
- **No answer-quality witness is taken.** The arc's standing caution holds; every clause of the frame register remains declared-uncovered.
- **The hoist rule is not adopted.** Task 6 measures one plan comparison. Materialize-vs-inline remains `019fddc6`'s.
- **The `profile_reachable_teams` materialization is not built** (spec §9). `[amended — 2026-08-10]` It was measured on 2026-08-09, and §9's measured conclusion is that Tasks 7–9 do **not** become unnecessary if it lands: the closure's share of gate cost is second-order at current scale (1.0–1.9% here, ~40% at a large synthetic tenant). It remains unbuilt, deferred on current need.
