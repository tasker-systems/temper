# Composable search fragments: one body per arm, a bound set is a scope, and the gate is computed once

> ## `[STILL VALID, WITH ONE GAP NAMED — 2026-08-08 evening]`
>
> This is a SQL design and the wire contract does not supersede it. Ground the surface in
> [`docs/api/query.openapi.yaml`](../../api/query.openapi.yaml); ground the fragments here.
>
> **One gap the contract surfaced that this design did not see.** §4 gives the twins `p_bound_ids`
> and treats a bound set as a scope — correct — but `search_graph_expand` takes `p_seed_ids`, and a
> seed is not a scope: its output *escapes* its input. The compiler never distinguishes them:
> `narrowing_for` (`query_plan.rs`) routes every upstream set to the narrowing slot and reads
> `bounds_mode` nowhere. A `follow-from` stage compiled that way returns only what was already in
> the seed set — a stage that looks like it worked and can never produce a neighbour. Latent today
> because `follow-from` still emits `__temper_unbound_act`; it fires the moment anyone binds it.
>
> **And a disclosure consequence with no SQL to serve it.** `search_graph_expand` collapses paths
> (`SELECT node, MAX(score) … GROUP BY node`), discarding which seed reached which node, and its
> `WHERE hop > 0` means a seed never appears in its own output. So the contract's
> `input_contributed` is null for `follow-from` — honestly, rather than reporting `0` on a stage
> that returned forty neighbours. Making it computable is the same body change as the edge-provenance
> spike.

`[design — 2026-08-08, Pete + session]`

Under frame register
[Every question to Temper is answered by a situated act](./019fbdb9-f287-79c0-aab6-efa0b1de12c8),
task [Phase 4 — build /api/query](./019fd25f-c0ea-70d2-a442-c46d697c4598), spec
[TemperQueryBuilder — a composition is a named DAG](./019fd46d-8da6-7722-8982-896f1a96c967).
This is beat C task 10's design, and it is larger than "bind the deployed fragment" — the reason is
§1.

## 1. The finding this closes

`/api/query` cannot bound a `find` stage by an upstream id set, because the deployed fragments have
no parameter for one. Verified in the live catalog, not inferred from `migrations/`:

```
        proname        | pronargs |                          args
-----------------------+----------+--------------------------------------------------------
 search_exact          |        7 | p_principal uuid, p_query text, p_anchor_table varchar,
                       |          | p_anchor_id uuid, p_doc_type text, p_limit int, p_offset int
 search_wide           |        8 | p_principal uuid, p_emb vector, p_k int, p_anchor_table varchar,
                       |          | p_anchor_id uuid, p_doc_type text, p_limit int, p_offset int
 search_graph_expand   |        5 | p_principal uuid, p_seed_ids uuid[], p_depth int,
                       |          | p_edge_types text[], p_gamma double precision
 wayfind_region_scores |        6 | p_principal uuid, p_lens uuid, p_emb vector, p_regions_n int,
                       |          | p_anchor_table varchar, p_anchor_id uuid
```

`search_graph_expand`'s `p_seed_ids uuid[]` is the **only** fragment input matching the
composition's currency. The other three take an *anchor pair* — one `(table, id)`. So the canonical
pipe (`follow-from` → `find-about-within`) is inexpressible, and `find-exact`'s declaration
(`registry.rs:118`) claims `accepts_bounds: [Resource]` — precisely the kind its fragment cannot
take — while omitting the two kinds it can.

**Post-filtering is not the escape.** Migration `20260806000020` put
`ORDER BY score DESC, resource_id LIMIT p_limit OFFSET p_offset` **inside** both find arms, so a
bound applied to their output filters after truncation. That is the wide-then-filter defect this arc
retired — a correctness rule, not a tuning choice.

**And the incumbent signatures may not change.** `DROP`/`CREATE` is shape-breaking and needs an
operator cutover; `CREATE OR REPLACE` of a body under an unchanged signature is additive and
auto-deploys. The design is therefore *new composable functions plus extracted interiority*, never a
signature change.

## 2. Evidence gathered for this design

Every claim below was executed, not read. Run against local PG 18 with `migrations/` fully applied.

### 2.1 A scalar SQL predicate does NOT inline (decides §3's mechanism)

Incumbent inline form vs. the same predicate behind `LANGUAGE sql STABLE`:

```
-- INCUMBENT (inline EXISTS)          -- VIA SCALAR FUNCTION
 Seq Scan on kb_resources r            Seq Scan on kb_resources r
   Filter: (ANY (r.id = (hashed          Filter: spike_has_doc_type(r.id, 'note'::text)
           SubPlan 2).col1))
   SubPlan 2
     -> Index Only Scan using
        uq_kb_properties_active
```

The index path disappears; a hashed set becomes a per-row function call. **Extraction by scalar
function is a performance regression, not a cleanup.** (The blocker is the sublink in the body;
a function whose body contains no sub-select is not covered by this measurement.)

### 2.2 A view is plan-identical (the mechanism that works)

Same predicate sourced from a view:

```
 Seq Scan on kb_resources r
   Filter: (ANY (r.id = (hashed SubPlan 2).col1))
   SubPlan 2
     -> Index Only Scan using uq_kb_properties_active on kb_properties p
          Index Cond: ((p.owner_table = 'kb_resources') AND (p.property_key = 'doc_type'))
          Filter: ((p.property_value #>> '{}') = 'note')
```

Byte-identical to the incumbent. And with `p_doc_type` NULL the `OR` collapses and no SubPlan is
planned at all.

### 2.3 The planner does NOT dedupe gate calls (decides §5)

Two call sites, same **literal** argument, in one statement:

```
 Result
   InitPlan 5
     -> ... CTE reachable_teams -> Recursive Union ...
   InitPlan 10
     -> ... CTE reachable_teams -> Recursive Union ...
```

Two full evaluations, each with its own recursive team closure. Via `WITH vis AS MATERIALIZED`:
one `CTE vis`, one `CTE reachable_teams`, then two `CTE Scan on vis`.

`resources_visible_to` is `provolatile = 's'`. **STABLE permits caching within a scan, not
common-subexpression elimination across call sites.** This is a planner property, so it holds on any
corpus — it is not something [the visibility-cost probe](./019fddc6-aace-7db0-a14d-5c610bc6506b)
needs to gather, and should be handed to that task as settled.

**Consequence:** an N-stage composition whose fragments each gate internally pays N full gates,
including N recursive team closures. All four fragments do gate internally — `search_exact`/
`search_wide` join `resources_visible_to` in their bodies; `search_graph_expand` at
`20260711000030:28,:109`; `wayfind_region_scores` via `visible_region_anchors` at
`20260731000050:93,:197,:205,:222`.

### 2.4 `kb_resource_homes` carries no filter columns

`\d kb_resource_homes` shows no `is_folded` / `is_current` / `is_active`. The anchor `EXISTS` is
complete as written, so it needs no view. (`resource_id` is UNIQUE — a resource has exactly one
home.)

## 3. Part 1 — extract the shared interiority (behaviour-preserving)

Three repeated things, each on the mechanism that fits it. Counted from the bodies, not restated
from memory:

| Extract | As | Copies | Why this mechanism |
|---|---|---|---|
| `doc_type` property lookup | view `kb_resource_doc_type` | 3 → 1 | §2.2 plan-identical. Homes `owner_table` / `property_key` / `is_folded` / `#>> '{}'` once — the polymorphic-key knowledge the two-arm split already **dropped once** and `20260806000020` had to restore |
| live-row predicate | view `kb_resources_live` | 3 → 1 | Makes *"`ingest_state = 'complete'` goes exactly where `r.is_active` goes"* structural rather than remembered |
| shrunk best-of-N formula | IMMUTABLE fn `shrunk_best_of_n(min_d, avg_d, n)` | 2 → 1 | Called on **already-aggregated** values, once per group — §2.1's per-row hazard does not apply. A scoring formula written twice will drift, and the two `search_wide` branches would then silently score differently |

**Not extracted, deliberately.** The anchor `EXISTS` (§2.4 — no hidden knowledge to protect). The
anchor readability gate: it already *calls* `cogmap_readable_by_profile` rather than restating it,
and its two forms — a `WHERE` conjunct in `search_exact` vs. a guard clause with early `RETURN` in
`search_wide` — are load-bearing per-arm differences, not drift. `search_wide`'s guard returns
**without scanning**; a conjunct could not.

Then `CREATE OR REPLACE` both incumbents onto these. Signatures unchanged ⇒ additive; behaviour
unchanged ⇒ **provable against the existing `search_exact_and_wide.rs` suite with no test edits**.
That provability is the whole reason this is its own migration.

## 4. Part 2 — the composable twins

`query_find_exact` and `query_find_wide`, created by **moving** part 1's now-clean incumbent bodies
into them and adding `p_bound_ids uuid[]` — the same currency as `search_graph_expand`'s
`p_seed_ids`. The incumbents then delegate with `p_bound_ids => NULL`, so there is **one body per
arm**, not two.

Three semantics carry weight:

- **A bound set is a scope.** In `query_find_wide` the branch predicate becomes
  `IF p_anchor_id IS NULL AND p_bound_ids IS NULL THEN <top-k> ELSE <exhaustive>`. The exhaustive
  branch has no truncation to defeat, so the rank-shaped rule is satisfied **structurally** rather
  than by a rule anyone must remember. The fork `search_wide` already carries *is* the fork the rule
  describes.
- **Empty is not absent.** `p_bound_ids = '{}'` means bounded-to-nothing ⇒ zero rows; only `NULL`
  means unbounded. An upstream stage that returned empty must never silently become an unbounded
  search — delta 3, *"an empty upstream is a disclosed disposition, never a substitution."*
- **`query_find_wide` carries its own `SET hnsw.ef_search`.** `proconfig` binds to a signature and
  does not inherit. A pin on a *wrapper* does reach a nested call, so `/api/search` stays safe — but
  `/api/query` calls the twin directly and would draw at the server default of 40, truncating
  silently. This is exactly the trap `search_exact_and_wide.rs:538-540` documents, arriving through
  the door its test does not watch: that test reads `WHERE proname = 'search_wide'` (`:547`). It must
  be **rederived over ANN-drawing functions** rather than named at one.

## 5. Part 3 — the gate is computed once, and the split that makes it possible

§2.3 forecloses the simple options: fragments that gate internally cannot share a gate, and the
planner will not do it for them.

So each arm splits in two:

- `__temper_ungated_find_exact(p_visible_ids, p_query, p_bound_ids, …)` — the real body. **Applies
  no visibility gate**; it is handed the verdict.
- `query_find_exact(p_principal, p_query, p_bound_ids, …)` — computes the gate and calls the core.

The compiler emits `vis` once and hands it to every stage.

**The full call chain, stated because the alternative reading is the wrong one.**
`search_exact(p_principal, …)` → `query_find_exact(p_principal, …, p_bound_ids => NULL)` →
`__temper_ungated_find_exact(p_visible_ids, …)`. So `/api/search` **also** routes through the core,
and therefore also pays the visible-set materialization. Letting it keep its own gated join instead
would restore two bodies per arm and forfeit the entire point of parts 1 and 2 — so the choice is
one body with `/api/search` on the array path, or the §7 fallback. It is **not** available to have
one body and leave `/api/search` on a join.

**The name states the hazard, not the visibility.** `_private` tells a reader "internal detail" and
invites a caller; `__temper_ungated_` cannot be misread. It follows the `__temper_unbound_act`
convention beat C already established for a deliberately-unsafe identifier.

**Naming is not the guard — §6 is.** And one build-time obligation is non-negotiable: a CTE cannot
be passed to a function, so the core receives the visible set as `uuid[]`. **Converting the
incumbent's gate JOIN into `= ANY(uuid[])` is PR #659's no-equivalence-class hazard run in reverse**
and could regress `/api/search`. The core must therefore join `unnest(p_visible_ids)` rather than use
`= ANY`, and the resulting plan must be compared against the incumbent's before the wrapper is
adopted. If it regresses, the split does not land and §7's fallback applies.

## 6. Part 4 — the tripwire, and what it cannot do

A `.github/scripts/audit-ungated-fragments.sh`, in the established layer (nine `audit-*.sh` scripts
exist; `audit-grant-sinks.sh` is the closest analogue).

**Derive the set; do not pin a list.** The repo's own lesson from
`assert_every_compiled_in_doc_is_vetoed` is that a hand-maintained enumeration rots while a derived
set does not — it greps for `include_str!` rather than trusting a list. Here: `rg` the
`__temper_ungated_` prefix across `migrations/` and `crates/`, assert the resulting **set** against a
reviewed corpus, and fail closed on any addition.

**Two failures it catches:** a new call site, and a new ungated function.

**One failure it cannot catch, and this is the likelier one.** It pins *where* the core is called,
never *what is passed*. The realistic bug is not a rogue call site — it is an approved site passing
`stage_2` where it should pass `vis`. CI green, RBAC bypassed. That is closed **structurally in the
compiler instead**: one emitter for ungated-core calls, with the id source **not a parameter** —
`vis` fixed inside it. Then there is no wrong set to pass. Asserted by a Rust unit test over
`CompiledQuery.sql`, no database, beside Task 9's existing identifier-safety tests.

**What the tripwire is not: a database permission.** Anyone holding a psql connection, a Neon
console, or the app credentials can call `__temper_ungated_find_exact` with an arbitrary `uuid[]` and
receive ungated rows. `REVOKE` buys nothing, because the application connects as the owning role.
**This is a real, accepted residue of the split, stated so it is not a surprise later.** The
invariant "every mechanic acts only on resources visible to the principal" is held, inside the
repository, by structure and by CI — and outside it, not at all.

## 7. What changes in Rust, and the fallback

`query_plan.rs::emit_act_body` gains real fragments for the three `find` acts. `CALLABLE_FRAGMENTS`
(`validate.rs:37`) becomes a **map** from declared mechanic (`search_exact`) to emitted fragment,
rather than a set — `served_by` must keep naming what `/api/search` calls.

The payoff is that §1's declaration gap **closes by making the declaration true rather than by
weakening it**. After part 2, `search_exact` *is* `query_find_exact` with NULL bounds — one body, one
`scoring_revision`, no fork — so `find-exact`'s `accepts_bounds: [Resource]` becomes accurate for the
first time and can honestly gain `Context`/`Cogmap`. Its rationale comment (`registry.rs:115-117`,
*"the exact arm carries no top-k"*) is **false since `20260806000020` added `LIMIT p_limit`** and is
corrected in the same pass.

**Fallback if §5's plan comparison regresses:** land parts 1 and 2 only. The twins gate internally,
`/api/search` keeps its join untouched, no ungated function is created — so **part 4 is not needed
either**, since the tripwire exists solely to guard functions this fallback never mints. `vis` is not
emitted for act stages, and an N-stage composition pays N gates. That is today's cost, not a new one,
and it leaves the compute-once decision to
[019fddc6](./019fddc6-aace-7db0-a14d-5c610bc6506b) with §2.3 in hand.

## 8. Refused, and recorded so it is not rediscovered

**A wide pre-joined view that gathers doc_type *and* the rest of the resource shape.** Already
refused by [Rebuild the pre-joined resource view](./019fd8f3-4955-7c20-be57-bc90b60d5122), on
grounds still live: *"The search arms are NOT candidates for it… Routing an arm through a pre-joined
view would put the join back inside the ranked query, which is what the two-arm split took out."*
Also: a view unifying consumers that want different things acquires an optional column per consumer,
*"which is the shape that drifts"*; and the `LATERAL` such a view would remove measured **0.300 ms of
7.718 ms — 3.9%**, Memoized.

`kb_resource_doc_type` is not that view and the refusal does not reach it: it is a narrow **predicate
source** (one table, three constant filters, two columns) living in an `EXISTS` under the `WHERE`,
contributing **no output columns**, carrying no visibility claim, whose consumers all want the same
thing, and measured plan-identical (§2.2). The distinction is *predicate source* vs. *hydration
spine*.

**A per-request temp table for the visible set.** Refused upstream (session affinity, which pooled
Neon behind Vercel does not guarantee); unchanged here.

**Caching the gate.** Deferred with its precondition named `[decided — 2026-08-07, Pete]`. No part
of this design may assume a cache.

**A cron-refreshed materialized view of the whole visibility relation** — one point-in-time
`(profile, resource)` relation per principal covering every reach route (ownership, team and team
DAG with role, cogmap-bound intersections, explicit grants), so compositions run against it and
visibility hoists out entirely. **Considered 2026-08-08 and not taken**, on three grounds — and note
that it attacks the right root: it does not solve compute-once, it *dissolves* it, since an indexed
lookup makes N gate calls cheap and would remove §5's core, §6's tripwire, the `uuid[]` currency and
`/api/search`'s array path together.

1. **Sizing.** The relation is principals × visible-resources — a tenant with 500 profiles each
   seeing 20k of 50k resources is ~10M rows, rebuilt every tick whether or not anyone queries;
   `REFRESH … CONCURRENTLY` needs a unique index and costs more.
2. **It is not local to `/api/query`.** `resources_visible_to` serves `list`, `show`, `get_meta` and
   both search arms. Either the entire read surface inherits the staleness, or the schema carries
   **two visibility predicates** — the drift hazard this design exists to remove.
3. **The revocation window is a disclosure window.** A stale ranking is wrong; a stale
   *authorization* predicate is a leak. A cron refresh is precisely the *eventual* invalidation the
   2026-08-07 deferral rules out (*"synchronous with membership/grant writes, never eventual"*).

Recorded rather than dismissed: (1) and (3) are properties of materializing the **whole** relation,
not of materializing at all, which is what §9's narrowing turns on.

## 9. Open, and who owns it

- **The hoist rule** — *materialize `vis` iff at least one stage is unbounded* — remains unadopted
  and owned by [019fddc6](./019fddc6-aace-7db0-a14d-5c610bc6506b). §2.3 narrows it: compute-once and
  push-bounds-into-the-gate are **complementary, not competing**. Compute-once dominates for
  unbounded stages; a bounds-accepting gate dominates for bounded ones (PR #659's measured 13× on
  `show`). How many stages are unbounded is what decides, and that is corpus-dependent.
- **Precomputing `profile_reachable_teams` alone — OPEN, and the promising narrowing of §8's
  refusal.** It targets exactly what §2.3 measured: the `Recursive Union` planned once per call site.
  It inverts both of §8's structural objections — size becomes profiles × **teams** (thousands, not
  millions), and its invalidation surface is small and **fully enumerable** (team create/delete,
  membership add/remove, parent-edge change). The grant and homes arms stay live, so ownership and
  explicit-sharing revocation remain instant; only team-derived reach is memoized, and team
  membership is administrative and infrequent. PR #660 (`20260807000010`) removed the double
  expansion; this would remove the remainder.

  **A MATERIALIZED VIEW is the wrong mechanism for it, and the reason is a hard PostgreSQL
  constraint** `[corrected — 2026-08-08]`. What makes this narrowing satisfy the 2026-08-07
  precondition rather than defer it is **synchronous** invalidation, and a matview cannot provide it:
  `REFRESH MATERIALIZED VIEW CONCURRENTLY` **cannot run inside a transaction block**, so it is
  unreachable from a trigger; and plain `REFRESH` takes an **ACCESS EXCLUSIVE** lock, which — since
  every gate call would read the matview — blocks every read in the system for its duration. A
  cron-refreshed matview is therefore just §8's eventual invalidation at a smaller size, carrying the
  same leak.

  **The mechanism that does work is an ordinary table maintained incrementally by triggers** — a
  closure table on `(profile_id, team_id)`. A membership write updates only the affected rows: no
  rebuild, no exclusive lock, fully transactional, so the gate is never stale even momentarily.

  **It is additive and changes no wire shape.** `profile_reachable_teams(uuid)` keeps its signature
  and becomes a delegating read, so every caller is untouched and the correctness surface is one
  function body — provable by asserting the table equals the recursive walk over a nontrivial team
  DAG. Precedent for the exact shape: `20260807000010`, declared `additive` for *"one new function
  plus CREATE OR REPLACE on two existing ones at unchanged signatures, no DROP."*

  ~~**The one figure that decides it is unmeasured: what share of gate cost the closure actually
  is. If 10%, this buys little; if 70%, it changes this design.**~~ **MEASURED `[2026-08-09]`.**
  Method and numbers: [Measuring the visibility gate's closure
  share](./019fe7f9-7d40-7c30-aff5-7eaadb3ab6dd). The call:
  [At current scale a team-closure table is not borne out by
  need](./019fe80d-152c-72d1-b612-212d77dff47d).

  **First, the framing above is withdrawn.** *"If 10% … if 70%"* was a **prior agent session's
  note, not the frame owner's valuation** `[corrected — 2026-08-09, Pete]`. It should not be read
  as a threshold anyone set, and an argument of the form *"it never reaches 70%, therefore this is
  minor"* reasons from a number nobody chose.

  The closure is **1.0–1.9%** of gate cost at this deployment's topology (depth 2, ≤2 memberships —
  and the team-anchored grant arm that returns zero rows in community has **4,800 rows** here, so
  this is not the corpus this section feared). Across synthetic plausible tenants it rises to
  **~40%**, crossing 10% at roughly 8 memberships / depth 5 / 100 teams. **At ~40%, on a large
  corpus, that is a felt difference** — the decision defers on current need, and explicitly does not
  rank the work as minor.

  What 70% turned out to describe: ~32–64 memberships over **disjoint** ancestor chains. Not an org
  chart. The value of having measured it is the **map** — we now know what topology gets there and
  what removing the closure buys at each point (~1.02× here, ~1.13× at the 10% marker, ~1.65× at
  16 memberships / 500 teams).

  **§5 and §6 do NOT become unnecessary**, and that conclusion is independent of the withdrawn
  framing: it required the closure to be the dominant cost at *our* scale, and at 1–2% it is not.
  The remaining 98% is the six-arm `UNION` over the visible corpus, which is precisely what §5 and
  §6 address.

  **Three findings that change what a probe must gather:**

  - **Reachable-team cardinality does not predict cost.** Shared reach 39 costs 4.14 ms; disjoint
    reach 32 costs 0.55 ms — 7.5× apart at nearly equal reach. The `CROSS JOIN LATERAL` walks once
    per MEMBERSHIP and the `DISTINCT` collapses only the output, so cost tracks **memberships ×
    depth**, modulated by team-table size. [019fddc6](./019fddc6-aace-7db0-a14d-5c610bc6506b)
    question 3 asks for the cardinality; it should ask for the walk-steps.
  - **The tenant's total team count is a cost input**, independent of any principal's position:
    16 memberships at depth 4 in a 500-team org costs more than 16 at depth 6 in a 200-team org.
  - **Cost is superlinear** — a 4× topology is a 12× cost, so a mean understates the tail.

  Still true and unchanged: **nothing in this design may assume the narrowing lands**, and arm
  contribution remains [019fddc6](./019fddc6-aace-7db0-a14d-5c610bc6506b)'s *"biggest single
  unknown"* — this figure does not answer it and does not touch the hoist rule. `[amended —
  2026-08-10]` The custom/generic-plan agreement (within 0.2pp) holds for the **corpus timing
  only**: the committed harness (`scripts/visibility-cost/closure_share.py`) can force a generic
  plan for its `corpus` subcommand alone, as its own `--generic` help states. The ~40% band, the
  10% marker, and the superlinearity finding come from `sweep`/`shapes`/`plausible`, which cannot
  force a generic plan — for those figures the custom-plan caveat still applies and is carried,
  not closed.

  The figures in this section are attested by session note
  [019fe7f9-7d40-7c30-aff5-7eaadb3ab6dd](./019fe7f9-7d40-7c30-aff5-7eaadb3ab6dd) — the measurement
  session. The committed harness is the instrument, not the record; no committed output file
  exists.

- **`survey` has two fragment arguments with no declared slot** — `p_lens uuid` and `p_emb vector`.
  Query and embedding reach a stage via `Composition.intention`; nothing carries a lens. Beat C task
  10's binding of `wayfind_region_scores` must answer this; this design does not.
- **`follow-from`'s `p_depth` / `p_gamma` are unmodeled.** `EdgeFilter` covers `edge_types` only.
- **An `IdSet` holds N ids; an anchor slot holds one.** A cardinality gap for the kinds that *do*
  map. `p_bound_ids` sidesteps it for resources; a multi-anchor bound is still unexpressible.
- **Rate-shaped axes** — inherited open, unchanged.

## 10. Acceptance criteria

Stated as what must be true, naming no mechanism — the lesson
[019fd8f3](./019fd8f3-4955-7c20-be57-bc90b60d5122) ends on, whose criteria said *"the view"* and
became unevaluable when the view was disproven.

- A composition can narrow a `find` stage to a set produced by an upstream stage, and the result is
  what a caller entitled to those rows would receive — never a page thinned by a filter applied after
  truncation.
- An upstream stage that produced nothing yields nothing downstream, and never silently widens.
- No mechanic returns a row the principal could not see, and no path to one exists that CI does not
  fail on.
- The `doc_type`, live-row and shrunk-score rules each have one definition; changing one changes
  every reader.
- `/api/search`'s observable behaviour is unchanged, and the evidence for that is a suite that did
  not have to be edited.
- An ANN draw cannot be reached by any door at a candidate list smaller than the `k` asked of it.
- Every declaration in `search_family()` describes what its mechanic in fact accepts.
