# Evidence: the interiority extraction is plan-identical, and the pin needed re-applying

`[measured — 2026-08-08]` Task 2 of
`docs/superpowers/plans/2026-08-08-composable-search-fragments.md`, over migration
`20260808000020_search_arm_shared_interiority.sql`.

## Verdict

**Plan-identical on all three arm shapes.** No node type, join order, index name or index condition
moved. Reproduce with `scripts/measure/capture-search-arm-plans.sh` before and after applying the
migration.

```
### 7. diff
PLAN-IDENTICAL: no node type, join order, index name or index condition moved.
```

## What was compared, and why it is not vacuous

An empty diff between two thin captures proves nothing, so the denominator is recorded:

| | before | after |
|---|---|---|
| captured lines | 3214 | 3214 |
| plan nodes (`"Node Type"`) | 218 | 218 |
| `uq_kb_properties_active` present | 1 | 1 |
| `idx_kb_chunks_embedding` (HNSW) present | 1 | 1 |

`uq_kb_properties_active` is the specific index spec §2.1 measured being **lost** when the same
predicate is extracted behind a `LANGUAGE sql STABLE` scalar function. Its survival on both sides is
the positive result; had the extraction been done by function rather than by view, that row would
read `1 → 0` and a per-row `Filter:` naming the function would have appeared. No such filter appears
in the after-capture.

The corpus is `cargo make seed-corpus 20` — 4802 resources, 4800 embedded chunks, visibility uneven
(ana 66.7%, ben/dev/cara 33.3%, nomad ~0) with 1601/801/801 rows arriving via the team-grant arm. On
a near-empty corpus every arm seq-scans and the comparison would agree because no index decision was
ever made.

Both captures ran against a **byte-identical** corpus: the generator is a seeded SplitMix64 and the
corpus is a pure function of its declaration, so the before-side database was destroyed and rebuilt
rather than mutated, and the two seeds produce the same rows.

## Method

`scripts/measure/capture-search-arm-plans.sh` + `.sql`, committed. Three shapes:

1. `search_exact` with a `p_doc_type` filter
2. `search_wide` **unscoped** — the approximate global top-k branch
3. `search_wide` **scoped** — the exhaustive branch

Two mechanics matter and neither is obvious:

- **`auto_explain`, not `EXPLAIN`.** `search_wide` is `LANGUAGE plpgsql`, so `EXPLAIN SELECT * FROM
  search_wide(...)` reports one line — `Function Scan on search_wide` — and says nothing about the
  plan inside, which is the only thing being measured. `auto_explain.log_nested_statements` reaches
  the statements the function actually executes. (`search_exact` is `LANGUAGE sql` and inlines, but
  routing both arms through the same mechanism captures them on equal terms rather than one by luck.)
- **Plan structure only.** `auto_explain` emits JSON; the wrapper extracts `.Plan` and drops
  `Query Text`. A refactor changes the arms' SQL *by definition*, so a diff including the source says
  only "you edited the thing you edited". Costs, row estimates and widths are stripped too — they
  move with statistics. Generated uuids and the 768-float query vector are normalized, being inputs
  rather than structure.

  Recorded because the first attempt got this wrong: with `log_format = 'text'` the diff was 45
  lines, *all* of them inside `Query Text:` blocks. That is a true result presented in a form where
  the reader has to take the author's word that none of the hunks were plan nodes. Extracting
  `.Plan` structurally makes the claim checkable.

## The finding the plan did not anticipate: `CREATE OR REPLACE` drops `proconfig`

Task 1 step 4 reads *"`CREATE OR REPLACE FUNCTION search_exact(...)` and `search_wide(...)` at
byte-identical signatures"* and says nothing about the `hnsw.ef_search` pin. Following it literally
ships an **unpinned** `search_wide`.

Measured on a scratch function rather than reasoned about:

```
ALTER FUNCTION __probe() SET hnsw.ef_search = 200;
SELECT proconfig FROM pg_proc WHERE proname='__probe';   -- {hnsw.ef_search=200}
CREATE OR REPLACE FUNCTION __probe() ...;
SELECT proconfig FROM pg_proc WHERE proname='__probe';   -- NULL
```

`CREATE OR REPLACE FUNCTION` redefines a function's properties wholesale, discarding every `SET`
clause a previous `ALTER FUNCTION` applied. An unpinned `search_wide` draws at the server default of
40 — below any `k` a caller passes — so `LIMIT p_k` becomes unreachable and the ANN draw truncates
with no error and no symptom. `20260806000020`'s own header calls the equivalent line *"the single
most dangerous line in this migration to omit, because nothing fails: results merely get quietly
worse."*

The migration therefore re-applies the pin, preceded by the `_PG_init` vector warmup (without which
an ordinary role gets `permission denied to set parameter "hnsw.ef_search"` — how `20260804000030`
failed its first Vercel deploy; a superuser, which every local and CI role is, never sees it).

**The existing guard bites.** Verified by removing only the re-pin — exactly what following the plan
literally would produce — and re-running:

```
### running the pin test WITHOUT the re-pin (expect FAIL)
    search_wide must pin hnsw.ef_search on the function itself; the pin on
    search_vector_candidates binds to that signature and does not inherit
     Summary: 1 test run: 0 passed, 1 failed

### restored; re-running (expect PASS)
        PASS search_wide_pins_ef_search_at_or_above_the_k_it_is_asked_for
```

## Task 1's own deliverable

`crates/temper-substrate/tests/search_exact_and_wide.rs` — **25 tests, 25 passed, zero file edits**.
That the suite needed no edit is the evidence that behaviour is preserved; it is why the extraction
is a separate migration from the twins that follow.

## What this does NOT establish

- **Nothing about performance.** Plan identity is a structural claim. No timing was taken and none is
  implied.
- **Nothing about the composable twins.** This covers only the extraction and the two incumbents.
- **Only three shapes.** An arm shape not in the capture — `p_limit` NULL, a cogmap anchor, an empty
  `p_query` — is uncompared. The three chosen are the ones whose predicates the migration touched.
- **One corpus, one machine, one PostgreSQL version** (local PG 18). Neon runs PG 17. The planner
  choices recorded here are not asserted to hold there.
