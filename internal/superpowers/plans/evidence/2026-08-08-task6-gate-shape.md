# Evidence: the ungated core's array path does not regress `/api/search`

`[amended — 2026-08-10]` A fifth falsifier, not in the list below, fired after this gate returned
PROCEED: a call whose body is guaranteed to return zero rows still paid the full visibility-gate
expansion (~1.2 ms → ~4.1 ms on text-only queries, measured on the 4802-resource corpus). It was
patched with a `CASE` guard in `query_find_wide` (`migrations/20260808000030_composable_find_family.sql`,
the block explaining the split), and adjudication ADJ-6 (2026-08-10) extended the same guard
symmetrically to `query_find_exact`. Two scope corrections this document owes its readers: the
PROCEED verdict was established against a query that returns rows and stands as re-scoped, not as
originally stated; and the deciding comparison (b) below is not reproducible from the committed
harness — `scripts/measure/gate-shape-comparison.sql` contains shapes A and B only, so comparison
(b)'s figures are attested by this document alone.

`[measured — 2026-08-08]` Task 6 of
`internal/superpowers/plans/2026-08-08-composable-search-fragments.md` — the decision gate that can
cancel Tasks 7–9.

## Decision

**PROCEED.** Tasks 7–9 (the gated-wrapper / ungated-core split, the single emitter, the tripwire) are
not cancelled. Spec §7's fallback does not apply.

Taken under **ship-as-measurement** `[decided — 2026-08-08, Pete]`: select the highest-confidence
option, state what would falsify it, ship, and iterate if the falsifier fires — rather than blocking
on a representative corpus we do not have. The falsifier is named below and is specific enough to act
on.

## The question

Spec §5 splits each arm into a gated wrapper and an `__temper_ungated_` core taking the visible set,
so a composition computes visibility once rather than once per stage. **A CTE cannot be passed to a
function**, so the core must receive that set as `uuid[]`. The chain becomes `search_exact` →
`query_find_exact` → `__temper_ungated_find_exact`, which puts `/api/search` on an ARRAY path where
it has a JOIN today. PR #659 established that `= ANY(...)` forms no equivalence class and so does not
propagate — this is that hazard run in reverse, and if it regresses, the split does not land.

## Method

`scripts/measure/gate-shape-comparison.sql`, committed. Corpus: `cargo make seed-corpus 20` — 4802
resources, principal `ana`, whose visible set is **3201 of 4802 (66.7%)**.

`unnest`, never `= ANY` — spec §5 is explicit, because `= ANY` is precisely the form that forms no
equivalence class.

**Row-set equality was confirmed first**, on every run: 3201 both ways. A timing comparison between
two queries returning different rows would measure nothing.

## Result

Two comparisons, and the second is the one that decides.

**(a) The join alone, given the set already computed** — the multi-stage case, where the gated
wrapper computes `vis` once and hands it to every stage:

| | incumbent `JOIN resources_visible_to(p)` | core `JOIN unnest(uuid[])` |
|---|---|---|
| 5 runs (ms) | 7.729 / 5.334 / 4.989 / 4.503 / 4.490 | 1.848 / 1.362 / 1.263 / 1.147 / 1.103 |

The array path is roughly **4× faster** here — but this number flatters it and must not be quoted on
its own: it excludes building the array, which is done outside the timed statement.

**(b) Total cost including array construction** — what `/api/search`, a SINGLE-stage caller, would
actually pay:

| | A: gate fused into the join (incumbent) | C: array materialized inline, then unnest-joined |
|---|---|---|
| cold (ms) | 13.537 | 11.953 |
| warm (ms) | 7.862 / 7.167 / 6.552 / 6.238 | 7.861 / 7.110 / 6.877 / 6.691 |
| **warm mean** | **6.95** | **7.13** |

**≈2.6% slower, which is inside the run-to-run spread of either column.** No regression.

The two comparisons say different things and both matter: (b) says `/api/search` does not pay for the
split, and (a) says a composition is the case that gains from it.

## What would falsify this — check these before trusting the decision elsewhere

1. **A principal whose visible set is very large in absolute terms.** The array path's cost scales
   with `|visible set|`, which must be materialized, shipped and unnested. Here that is 3201 uuids.
   At 500k the materialization could dominate and the sign could flip. **This is the primary
   falsifier**; the measurement says nothing about that regime.
2. **A principal who sees a small fraction of a large corpus.** Expected to favour the array path
   *more* (a smaller array against the same index), so this is a direction of increased confidence,
   not risk — but it is unmeasured, and "expected" is not "measured".
3. **PostgreSQL 17.** This ran on local PG 18. Neon runs 17.
4. **Real embeddings and real text.** The corpus is synthetic. That is deliberate and does not affect
   a join-shape comparison, which turns on cardinality rather than content — but a corpus whose
   `kb_resource_search_index` term distribution is very different could change how many rows the FTS
   predicate admits before the gate join, which is the input to both shapes.

None of these is a reason to withhold the decision; each is a reason a later measurement could
overturn it, which is what ship-as-measurement means.

## What this does NOT establish

- **Nothing about the split's correctness**, only its cost. The ungated core applies no visibility
  gate, and everything keeping that safe is Tasks 8 (a single emitter with the id source not a
  parameter) and 9 (the tripwire) — plus spec §6's stated, accepted residue: the tripwire is source
  discipline and not a database permission, since the application connects as the owning role.
- **Nothing about the hoist rule.** *Materialize `vis` iff at least one stage is unbounded* remains
  unadopted and owned by `019fddc6`. This measured one plan comparison, as the plan specified, and
  nothing more.
- **One corpus, one machine, one principal, one query.** A single FTS term (`postgres`) admitting a
  substantial fraction of the corpus. A highly selective term, or a wide arm draw, is uncompared.
