# `TemperQueryBuilder` — the compositional surface, v0 design

**Status:** design, approved in session 2026-08-05. Ships nothing by itself; it is the settled shape
that phase 4 of the frame register's sequenced plan is built from.

**Frame register:** [Every question to Temper is answered by a situated act — acts compose by piping,
no constant decides between them, and no composition
flatters](./019fbdb9-f287-79c0-aab6-efa0b1de12c8).

**Keystone decisions carried in, not re-argued:**
[Search splits in two](./019fd25a-ef4c-7473-b72e-265a7d36dd65) ·
[A composition compiles to one SQL statement](./019fcd13-4e65-7213-ac6f-20c3c8ccfce1) ·
[`visibility_profile` declares the act's ordering fragment](./019fd2ea-daa6-72b3-a92a-9b75485c586e) ·
[A declaration is a description](./019fd377-ee04-7223-861b-3e0bebabaceb).

**Contract authority:** `docs/superpowers/specs/2026-08-03-query-envelope-contract-v0-design.md`.
This document does not restate that contract; it amends it where noted below and builds the executor
the contract was written for. Where the two disagree, the amendments here are the later word.

## Provenance discipline

- **`[verified — 2026-08-05]`** — read first-hand this session against the working tree at
  `56761a10`, or executed against the production database.
- **`[measured on prod — 2026-08-05]`** — a query run against `temper-cloud` (Neon, PG17) this
  session, quoted with its numbers.
- **`[carried]`** — taken from a cited decision or research document and not re-opened here.

---

## 1. The gap that blocks everything else

**A composition cannot express a pipe.** `ActInvocation` carries `bounds: Option<IdSet>` — a literal
set of uuids the caller supplies — and no field naming an upstream stage
`[verified — 2026-08-05, crates/temper-core/src/types/query/envelope.rs:21-39]`. `Composition.stages`
is a `Vec<ActInvocation>` `[verified — composition.rs:68]` and `act_sequence()` returns act names in
order with nothing between them `[verified — composition.rs:73]`.

The trace side can already *report* the relationship it has no way to *declare*:
`BoundsSource::Upstream { stage: u32 }` `[verified — trace.rs:19]`.

So the shipped contract describes chaining and admits only a list of independent acts over
caller-supplied ids. Everything else in this document follows from closing that.

**This reshape is cheap, which is why it goes first.** The query contract types have **no consumer
outside `temper-core`** — no route, no CLI command, no MCP tool, no service
`[verified — 2026-08-05, repo-wide search]`. The only dependents are a schema-snapshot test
(`crates/temper-core/tests/query_schema.rs` and its two fixtures) and a generated
`packages/temper-ui/src/lib/types/generated/query.ts` that nothing imports. A breaking reshape of a
type that shipped nine days ago costs two regenerated snapshots and a ts-rs run.

---

## 2. A composition is a named DAG

`[decided — 2026-08-05, Pete]` `Composition.stages` becomes a set of **named nodes**, each declaring
its inputs as **references** rather than literal ids. The incumbent `bounds: Option<IdSet>` survives
as one input variant — the caller-supplied case — joined by an upstream-stage reference.

**Set combinators are their own node kind.** `union` and `intersect` take two inputs; no act does, so
they cannot be act invocations without lying about what an act is.

Three properties are what the type buys, and each is what a test pins:

- **Chainability is derived, never encoded.** A node's inputs are legal iff each upstream's
  `produces` kind appears in this act's `accepts_bounds` / `accepts_seeds`. That relation already
  exists in `search_family()` `[verified — registry.rs:76]`, and the registry's own header states the
  rule this obeys: *"The chainability matrix is not a separate structure… Encoding it twice would be
  the `ADMIN_EVENT_TYPES` failure — a second copy that drifts from the first."* The DAG **consumes**
  that relation and adds no second copy.
- **Topology is validated statically** — cycles, dangling references, and kind mismatches are
  properties of the plan, decided before any SQL exists (§5).
- **`ActResult.produced` stops being a required `IdSet`.** It becomes a variant. This is the same
  change that gives `substantiate` somewhere to land: the phase-3 record states that *"`ActResult.produced`
  is a required `IdSet` … so `substantiate` — which annotates rather than selects — can be declared
  and cannot return,"* leaving `claims-carry-standing` with nowhere to land `[carried — decision
  019fd377]`. The reshape closes that shape gap without building the act.

### Why a DAG rather than a linear list

`[decided — 2026-08-05, Pete]` The rule this contract runs on is *"where return shapes are
commensurate they can drive a sub-selection downstream."* By that same rule, two acts producing the
same `IdKind` are combinable — so a linear list makes the one operation the rule most obviously
licenses the one it cannot express.

The shape is not novel here: tasker-core models workflow steps as a DAG, and Temper's own team
structure is a recursive ancestor walk (`team_ancestors`). A query over a graph must be directed and
acyclic even though what it operates on is neither.

---

## 3. The response is per-arm, and hydration is declared

`[decided — 2026-08-05, Pete]`

### Why per-arm follows from the frame rather than from taste

A composition ending in a union has a well-defined **set** and an ill-defined **ranking** — ordering
it across arms is exactly the cross-act comparison `no-cross-act-ranking` forbids. Three shapes were
available: return the merged set unordered; return per-arm arrays; or let the composition nominate
one stage's quantity to order everything, which produces a partially-ordered result reading as a bug.

**Per-arm arrays are taken**, and the deciding reason is that it makes `/api/query`'s answer the same
shape `/api/search` is becoming under phase 1 (`{ exact: [...], wide: [...] }`), so the two surfaces
teach one lesson rather than two.

### v1 already needs two kinds, so this is not future-proofing

`survey` declares `produces: Some(IdKind::Region)`, `accepts_bounds: [Cogmap, Context]`,
`accepts_bound_terms: [Regions]`, and orders by `region_score`
`[verified — 2026-08-05, registry.rs]`. Phase 1 orphans its mechanic from every door, so
**`/api/query` becomes the only way to reach it.**

The first real composition therefore demonstrates the whole thesis:

| arm | kind | ordered by | scale |
|---|---|---|---|
| a `find-about-*` stage | `resource` | `vec_norm` | `[0, 1]` |
| a `survey` stage | `region` | `region_score` | **`[-0.6, 1.0]`** |

Two arms, declared and **different** ranges, no number ranking one against the other. It is also the
first time `region_score`'s negative range is visible to a caller — something decision
[019fd377](./019fd377-ee04-7223-861b-3e0bebabaceb) records as a value *"the whole arc has been
reading as a `[0,1]` score."*

### The response shape

A group is a **returned stage**, carrying its `kind`, its rows, its `ordered_by` (the declared field
name and scale from `ActQuantity` `[verified — act.rs:177-184]`), and its own `extent` — per-group by
necessity, since `survey` is `Indeterminate` while a `find` arm is `Complete` or `Partial`.

**Projection is per-kind.** A resource row hydrates title / doc_type / home display; a region row
hydrates label / member_count / salience / home anchor. Neither carries the other's columns. Carrying
`member_count` on a region row is **metadata on a row, not a second ranking axis**, precisely because
rows from different arms never share a list to be ordered in.

**This retires the enrichment N+1 rather than inheriting it.** `search_select` today loops over hits
and runs `native_resource_row` per result `[verified — 2026-08-05,
crates/temper-services/src/backend/substrate_read.rs:927]` — 50 results, 51 queries. Per-kind
projection is one join per returned kind, inside the same statement.

### Returned stages are declared, not inferred

`[decided — 2026-08-05, Pete]` `OutcomeDeclaration` gains `returns` — the stages whose rows are
hydrated — with an optional per-stage field list defaulting to the kind's projection.

Inferring from out-degree zero was rejected for a specific failure, not for taste: it makes returning
an intermediate impossible without a dummy consumer, and it means **adding a downstream stage
silently stops returning what you used to get back**. Declaring also keeps `OutcomeDeclaration` doing
what its own doc comment says — *"the pocket outcome register: a saved plan states its served-by"* —
which is a statement about what comes back, not about graph shape.

**One shipped field breaks.** `OutcomeDeclaration.produces: Option<IdKind>`
`[verified — composition.rs:45]` assumes one kind per composition. A resource arm beside a region arm
has no single answer, so `produces` becomes **derived** from the returned stages rather than declared.

### Three axes that must not be conflated

| axis | decides | applies to |
|---|---|---|
| `OutcomeDeclaration.returns` | what is **hydrated** | declared stages only |
| `CompositionTrace` | what **ran** and how it resolved | **every** stage, always, no knob |
| `MetaDetail` | how much **per-id participation** is retained | ids, not rows |

An interstitial stage is never invisible: it always carries a `StageTrace` with its disposition,
`bounds_in` / `bounds_honored` / `bounds_dropped` and `narrowed_by`. What it does not carry is
hydrated rows. That satisfies `composition-is-legible` through the mandatory tier — contract §4.4:
*"mandatory, never truncated, no knob turns it off"* — while the expensive part stays opt-in.

`MetaDetail::Full` retains **id-level participation records** ("this id was in stage 1 and dropped at
stage 2"); it does **not** hydrate them. Full is a diagnostic axis, `returns` is a payload axis, and
they are orthogonal.

### A row may name its parents; it may not embed them

`[decided — 2026-08-05, Pete]` Three things are easy to collapse into one and must not be, because
two of them are the point of a knowledge-graph surface and the third is a rebuild of GraphQL.

1. **Edge-filtered traversal as a stage — IN, and it is the primary case.** `follow-from` admitting
   `EdgeFilter { edge_kinds, labels }` `[verified — filter.rs:26-31]` narrows *the walk*, inside the
   act, narrow-first. `filter.rs`'s two-axis design — the closed DDL `EdgeKind` re-used rather than
   restated, with open `labels` beside it — is one of the affordances this builder exists to make
   reachable at all. §8 shows why the two axes cannot be merged on live data.
2. **Flat arms carrying edge provenance — IN, subject to the check below.** An arm returning 40
   neighbours should say *which seed each is a neighbour of, and via which edge*. That is real
   information and it needs no tree: `from_id`, `edge_kind`, `label` are columns on a flat row. It
   introduces no ranking, because provenance is structure rather than quantity.
3. **Nested projection — REFUSED.** A response shaped as a tree, where the caller declares a
   selection set that recursively follows edges into the returned shape, is where a GraphQL rebuild
   actually begins, and no caller has asked for it.

> **The rule that separates 2 from 3: a row may NAME its parents; it may not EMBED them.** A `via`
> entry carries ids and edge metadata, never a hydrated resource.

**The open check, stated as unverified rather than assumed.** A node reachable from two seeds via two
edges has more than one parent, and `follow-from`'s mechanic is a `MAX(score) GROUP BY node` over the
walk `[carried — decision 019fd2ea]` — it **collapses paths by construction**. Whether
`search_graph_expand` can emit path provenance without a change to its body is **not verified here**,
and beat C owns finding out. If it can, `via` is an array; first-wins would be a silent lossy pick and
is the wrong answer. `search_graph_expand` is untouched by phase 1, so any change to it belongs to
this phase and collides with nothing.

### `union` narrows to an intermediate

Per-arm returns remove the reason to merge at the end — you already get both arms. A `union`
**terminal** would honestly have to report `ordered_by: null`. As an **intermediate** feeding a
downstream act that bounds on it, order is irrelevant and the combinator is genuinely useful. It
stays for that case and is not the shape callers should reach for at the end of a plan.

---

## 4. The stage contract is kind-parametric

`[decided — 2026-08-05, Pete]` **Every stage fragment emits `(id, kind, quantity)`**, where
`quantity` is a nullable `double precision` — nullable because `orders_by` is `Option<ActQuantity>`
`[verified — act.rs:262]` and an act that orders nothing has none to emit.

**This is the internal CTE shape, never a neutral score on the wire.** Contract §4.1.1 rule 4 states
*"Field names carry their act. No bare `score`."* The identity travels beside the column as declared
metadata — `ActQuantity.field` is *"the DEPLOYED column name the serving function emits"*, plus
`means` and `scale` `[verified — act.rs:177-184]` — so the builder aliases the positional column back
to `fts_norm` / `vec_norm` / `graph_score` / `region_score` in the final select. **Positional for
compilation, named for the reader.**

### The rule that keeps `no-cross-act-ranking` structural

> **A quantity never crosses a stage boundary.** A downstream stage references its upstream as
> `SELECT id FROM <stage>` — ids only. A quantity exists for its own stage's ordering, its
> `terms_effective`, and the trace. It is never in scope for another act's expression.

Without this rule, a uniform quantity column would make cross-act arithmetic mechanically easy in a
way the shipped design deliberately made impossible — contract §3.1: *"a stage receives a set, so no
ordering is available to blend."* The code generator enforces it; a test pins that no generated stage
reference selects a quantity column.

### Why kind-parametric is the decision that would otherwise foreclose the region phase

The v1 chains never change kind — `find-about-anywhere` → `follow-from` is resource to resource. So a
`(resource_id, score)` contract would **pass every v1 test** and quietly make region-mediated
composition unbuildable. Three further requirements follow, and exist so that "we did not foreclose
it" is a checked property rather than an intention:

1. **Projection is per-kind**, never defined solely over resource-row fields (§3).
2. **`IdSet.provenance` is carried and checked in v1's chaining** even though only `survey` produces
   regions. Contract §3.1 records why: context regions and cogmap regions are both `RegionId` and are
   not interchangeable, and `graph_region_composition` gates on `cogmap_readable_by_profile`, so a
   context-region id 404s at the sole consumer of region ids. Checked, that is a declined plan;
   unchecked, it is a rediscovered 404. **Live data confirms both populations exist** (§8).
3. **At least one test proves a kind-changing hop is expressible**, so kind-typed chainability is not
   accidentally resource-to-resource only.

---

## 5. Validation is two layers, and only one of them can refuse mid-plan

The split is already committed by contract §4.1.1: *"That refusal is static. It is decided by
evaluating the plan against the generated schemas at the correctness layer, before execution."* This
extends it from bound terms to topology.

**Static — decided against `search_family()` alone, no database:** unknown act; a cycle; a reference
to an undeclared stage name; kind mismatch (`UnsupportedBoundKind` / `UnsupportedSeedKind`);
`MissingProvenance`; `BoundTermNotApplicable`; `FilterNotApplicable`; `UnknownFilterValue` on a closed
vocabulary; `MissingIntention` for a `find-about-*` stage; `NotImplemented` for an act whose
`build_state` is `unbuilt`; a `returns` naming a stage that does not exist.

**A validate-only path is published.** A caller who can check a plan without running it is the
difference between this surface being authorable and being trial-and-error. It is also the honest
answer to the ergonomics question §9 records: introspection plus validation is what a schema explorer
gives you, at a fraction of the machinery.

**Runtime — per stage, only what needs data:** the disposition (`Answered` / `Empty` / `Refused`),
`terms_effective`, `bounds_dropped`, `Extent`.

**`RefusalDisposition` applies only to the runtime layer.** `Halt` and `DegradeAndDisclose` govern
what a composition does when a *stage* refuses; a static refusal fails the whole plan before
execution and has nothing to degrade. Worth stating, because *"the composition declares its
disposition toward a stage refusal"* reads as though it covers both, and it cannot.

---

## 6. Compilation, and the one repo invariant it breaks

The builder composes **calls to the per-act SQL functions**, never hand-written predicates:
`search_exact` / `search_wide` (phase 1), `search_graph_expand`, `wayfind_region_scores`. Each becomes
a CTE. The visibility relation is materialized once and joined per stage — decision
[019fcd13](./019fcd13-4e65-7213-ac6f-20c3c8ccfce1) records that a single `unified_search` call makes
**4** `resources_visible_to` computations at runtime `[carried]`, and one query time is what collapses
them.

Its prerequisite is **already landed**: `profile_reachable_teams` ships as one authoritative relation
beneath the whole visibility spine `[verified — 2026-08-05,
migrations/20260804000010_profile_reachable_teams_read_path.sql,
migrations/20260804000020_profile_reachable_teams_write_gates.sql]`.

### The trade, stated rather than discovered

**This is dynamic SQL, so `sqlx::query!()` compile-time checking does not apply**, and the `.sqlx`
cache discipline assumes static statements.
`[decided — 2026-08-05, Pete: this trade is inherent to any builder-shaped approach, and is paid for
with tests rather than avoided.]`

**It is not unprecedented, and the precedent comes with an instruction the plan must obey.**
`readback/mod.rs` already carries three runtime `sqlx::query_as` reads — `vector_search`,
`unified_search`, `wayfind_scope_ids` — and its module note is explicit about their status:
*"all for one reason — a `$n::vector` bind the macros cannot type. That is the allow-listed exception
class and the whole of it here; it is deliberately not a house style."* The note also records what
happened when the exemption spread: *"Until 2026-07-30 sixteen further reads in this module were
runtime 'for consistency' with those three, which left them absent from the cache — and the cache is
the record a schema/binary change detector reads, so an exemption taken for tidiness was subtracting
from the coverage of a safety check."* `[verified — 2026-08-05,
crates/temper-substrate/src/readback/mod.rs:1-34]`

So the builder is a **fourth runtime read with a second, different reason** — dynamic composition
rather than a `::vector` bind. **Amending that module note to name the second class is a required
step of the build, not documentation hygiene.** Left unamended, the note instructs the next cleanup
sweep to claw the builder back exactly as it clawed back the sixteen, and it would be right to.

*(Note for whoever lands phase 1: `wayfind_scope_ids` is one of the three and phase 1 retires it, so
the `::vector` class shrinks to two as this one arrives.)*

Three obligations make it tolerable, and each is a plan step rather than a hope:

1. **The predicates are not dynamic — only the skeleton is.** Every filter, score and visibility join
   lives inside a function whose body its own migration checks. The builder assembles
   `WITH … AS (SELECT * FROM search_wide($1, $2, …))`, not SQL logic.
2. **Every value is bound; no value is interpolated.** Stage names are the only identifiers the
   builder emits and they come from a validated plan — but they are still held to an identifier
   allowlist rather than trusted, with its own test.
3. **The substitute for compile-time checking is a generative harness**: enumerate the DAG shapes the
   declarations permit, compile each, and `EXPLAIN` it against a real database. This catches what
   `query!()` would have **plus** plan-shape regressions it never could — which matters because
   decision `019fcd13` warns that *"a single compiled statement concentrates all risk in the query
   plan."*

**The interface obligation, stated as the acceptance bar:** the builder must be unable to emit a
composition the system cannot process, and unable to emit one carrying caller text into an identifier
position. Those are the two properties the harness exists to hold.

---

## 7. Sequencing

`[decided — 2026-08-05, Pete]` **The builder is demonstrable end-to-end before phase 1 lands.** Phase
1 rewrites the FTS and vector arms and retires `unified_search`, but `search_graph_expand` and
`wayfind_region_scores` survive untouched — they are the two mechanics it orphans from every door.
So the executor can compile and run real compositions over `survey` and `follow-from` with **no
dependency on the sibling's work**, against exactly the acts most in need of a door.

| beat | needs phase 1 | lands |
|---|---|---|
| **A** | no | The type reshape: named DAG, stage references, combinator node, `returns`, `ActResult.produced` as a variant, `produces` derived. Closes phase 3's `claims-carry-standing` shape gap |
| **B** | no | Static validation and the validate-only path. Pure, no database |
| **C** | no | The compiler: kind-parametric fragment contract, CTE assembly, `vis` hoist, identifier allowlist, generative `EXPLAIN` harness — **executing compositions over `survey` and `follow-from`**. Owns the edge-provenance check (§3): can `search_graph_expand` emit path provenance without a body change, given its `MAX(score) GROUP BY node` collapse |
| **D** | **yes** | Bind `find-exact` / `find-about-*` to `search_exact` / `search_wide` |
| **E** | no | The doors: `POST /api/query`, an MCP tool, `temper query`, and a `door_coverage` entry per act |
| **F** | **yes** | Reconcile act declarations — `Fused { unified_search }` → `Served` where `/api/query` serves an act alone |

**Coordination points with phase 1, and only these two.** D consumes the sibling's two new functions.
F is the same declaration reconciliation phase 1's own step 5 owns — one reconciliation, not two.

### The state between C and D, and the `BuildState` gap it lands on

Between beats C and D the three `find` acts are declared `Fused { host: "unified_search" }` and the
builder **cannot call them**: their mechanics live inside a composite the builder has no way to reach
into, and the standalone functions do not exist yet. So a plan naming `find-exact` must refuse — and
`RefusalReason` has no honest variant for it. `NotImplemented` is documented as *"the act is declared
but not built (`build_state` is not `served` or `fused`)"* `[verified — disposition.rs:64-65]`, which
is false of a fused act.

**This is the same `BuildState` gap phase 1 already flagged, reached from the other side.** That task
records that after its step 4 `survey`'s mechanic exists and no door reaches it — *"neither `Fused`
(no host has a door either) nor `Unbuilt` (the function is right there). The type cannot currently
say it."* Here the shape is *fused into a host this caller cannot invoke*. Both are the same missing
distinction: **`BuildState` conflates whether a mechanic exists with whether the asking surface can
reach it.**

Named, not solved here. Two things follow for the plan: `RefusalReason` is **open**
`[verified — disposition.rs, and decision 019fcd13]`, so a variant for *"declared, built, not
reachable from this surface"* is additive and can be added when beat C needs it; and whichever of the
two phases reaches the gap first should settle it once, since fixing it twice differently is the
outcome to avoid.

---

## 8. Measured grounding

`[measured on prod — 2026-08-05, temper-cloud]` Run this session so the next reader does not re-run
them.

**The edge corpus supports `EdgeFilter`'s two-field shape on live data**, not only on the
`--edge-type advances` bug contract §4.1.2 cites:

| label | appears under |
|---|---|
| `derived_from` | `leads_to` (873) **and** `express` (312) |
| `relates_to` | `near` (1039) **and** `leads_to` (77) |
| `part_of` | `contains` (161), `leads_to` (20), `near` (12) |

Filtering by label alone merges kinds; filtering by kind alone merges labels. 4665 edges total across
4 kinds and **at least 30 distinct kind×label pairs** — the query was capped at 30 rows, so that is a
floor, not a count.

**The region substrate is real, thin, and lopsided:**

| | |
|---|---|
| region rows | 6134 — of which **5201 folded** |
| live regions | **933** (447 cogmap-anchored, 486 context-anchored) |
| anchors bearing live regions | 3 cogmaps, 3 contexts |
| live membership | a **partition**: 1.00 regions per member, both anchor kinds |
| active resources | 3367 |
| coverage | **1956 (58%)** in a live context region; 772 (23%) in a live cogmap region |

Three things follow. **Context regions exist as first-class rows** with
`home_anchor_table = 'kb_contexts'` in a table named `kb_cogmap_regions` — corroborating contract
§3.1's `provenance` argument on live data rather than by construction. **Resource→region is a
function, not a fan-out**, so a region chain is bounded. And **coverage is partial**, which the region
phase inherits as an obligation, below.

---

## 9. GraphQL — a deferred second door, and why the ordering is the argument

`[decided — 2026-08-05, Pete]` Raised in design as a serious alternative — *"what if we did GraphQL
but with more steps"* — and **deferred rather than refused**. Recorded in full because the same
instinct will recur, and it deserves an answer rather than a rediscovery.

**Where it holds.** For a browser client GraphQL is better ergonomics: mature client libraries,
codegen, introspection instead of reading a spec. And agents genuinely do know the syntax.

**Where it deflates.** At the MCP door — the dominant agent surface — the caller invokes a tool and
reads its parameter schema, never touching the wire format, so the familiarity dividend is collected
only at the raw-HTTP door, which already has generated clients on both sides. And the familiarity is
for *syntax*, not for *acts*: an agent fluent in GraphQL still must learn that `find-about-anywhere`
and `find-about-within` are two acts and why — while expecting nesting to work and field selection to
be free, both of which are wrong here. **Familiar syntax over unfamiliar semantics suppresses the
reflex to read anything.**

**Three structural facts, one of them decisive.**

1. **GraphQL nests; it does not re-root.** Taking a result set from one root field and using it as the
   *scope argument* of another is not expressible in standard GraphQL — Apollo's `@export` is the
   non-standard workaround and it works by additional client-side round trips. Re-rooting is the
   operation this whole surface exists for.
2. **Resolver-per-field is N snapshots**, which is what decision `019fcd13` exists to prevent.
   Reaching one statement means compiling the GraphQL AST to SQL — which is this builder with GraphQL
   as front-end *syntax*, not instead of it.
3. **Decisive: GraphQL cannot refuse a cross-act sum.** A client selects two acts' quantities in one
   document and the type system has no way to decline it. The clause this arc exists to make
   *untypeable* would revert to merely documented.

**The ordering is one-way safe and that is the whole argument.** The engine consumes a `Composition`
— a value, not a call — so a GraphQL layer is an **adapter that translates a document into a
`Composition`**, inheriting validation, refusal semantics and the trace unchanged. Engine-first leaves
that door available. GraphQL-first commits to a type system that cannot express
refused-vs-empty-vs-withheld and cannot decline a cross-act sum, and those semantics cannot be
extracted back out afterwards.

**On introspection specifically** — the thing a schema explorer gives that OpenAPI does not:
`search_family()` is already a machine-readable declaration list `[verified — registry.rs:76]`.
Exposing it as a read endpoint is introspection with a fraction of the machinery, and it is the first
artifact goal [Surface parity](./019fa618-ce41-7762-97dd-179132503ea2) could be witnessed against.

## 9.1 What transfers from tasker-grammar, and what does not

`[verified — 2026-08-05, ~/projects/tasker-systems/tasker-core/crates/tasker-grammar]`

**Transfers:** the envelope *shape*, the declaration and schema-compat validation layer, the `explain`
analyzer, and a proven dependency set (`jaq-core 2.1`, `jaq-std 2.1`, `jaq-json 1.1`,
`jsonschema 0.28`).

**Does not transfer: the executor.** It is a chained per-invocation evaluator whose
`CompositionEnvelope::resolve_target()` is literally *"use `.prev` when present and non-null,
otherwise `.context`"* `[verified — tasker-grammar/src/types/envelope.rs]` — the prev-else-context
fallback contract §4.3 names as the flattering-degradation vector Temper must not inherit. Temper
compiles; it does not iterate.

### There is no expression language, and that is a strengthening

`[decided — 2026-08-05, Pete]` Not "v1 may not need jaq" — **the design has no place for one, and
adding one would be contrary to it.**

Every narrowing axis already has a typed slot: bounds are stage references or caller id sets; terms
are the closed `BoundTerm` vocabulary; predicates are `ResourceFilter` / `EdgeFilter` /
`FacetPredicate` `[verified — filter.rs]`; combination is a node kind; projection is a field list.
There is no residue for an expression to handle. And `filter.rs`'s own header states why adding one
would be a regression rather than a feature: *"deliberately NOT a generic `{field, op, value}`
grammar: a general predicate language would be more expressive and would immediately re-open every
conflation this contract exists to close."*

So the grammar is **declarative data, not a language** — a `Composition` is a JSON body validated
against generated schemas. That makes it publishable as part of `/api/query`'s OpenAPI surface and
handed to a human or an agent as a delivered artifact, which is strictly stronger than a surface whose
plans must be authored in an embedded expression language. It is also the honest answer to the *"am I
inventing a query language for fun"* question this design opened with: the artifact is a schema, not
a syntax.

**Two shipped types are left with no producer, and they are treated differently on a real
distinction:**

- **`RefusalReason::ExpressionNotPushdownable` is removed.** `RefusalReason` is **open**
  `[verified — disposition.rs:44-50, and decision 019fcd13]`, so re-adding it later is additive and
  costs nothing. Keeping a reason nothing can raise is a claim about the system with no referent.
- **`BoundsSource::Expression` is kept and documented as reserved-and-unreachable**, with a test
  asserting **no compiled plan ever emits it**. `BoundsSource` is a *closed* tagged enum
  `[verified — trace.rs:16-32]`, so removing it now would make re-adding it a breaking change. The
  test is what converts "reserved" from an unfalsifiable claim into a checked one — the same
  discipline that caught `nothing_in_the_search_family_is_served` sitting green beside a comment that
  was false about the deployed system.

---

## 10. Declared holes and stated silences

**A second currency beyond ids — OUT, with its reason.** Deriving keywords from a result set is
plausible inside one statement; *embedding* them is not — embedding happens in Rust before the
statement is built `[verified — substrate_read.rs:641, embed_query_if_missing]`. So a
derived-intention stage does not extend the builder, it breaks decision `019fcd13`'s single-statement
rule. Reopening this is a decision **about that decision**, not a feature request. The stage output
type is nonetheless designed as a **tagged union with exactly one variant**, so admitting a second
currency later is additive rather than breaking.

**Region-mediated chains — a later phase, deliberately not foreclosed.** Three acts are undeclared:
resource→region (occupancy), region→region (adjacency), region→resource (membership; the nearest
mechanic, `graph_region_composition`, is cogmap-only per contract §3.1). §8 records the substrate so
it is not re-measured, and §4 records the four requirements that keep the phase buildable.

> **The obligation that phase inherits: region coverage must be disclosable.** 42% of active
> resources occupy no live region `[measured on prod — 2026-08-05]`, so a region-routed chain drops
> them — material excluded *because nobody clustered it*, arriving as material that *did not match*.
> That is not `visibility-is-never-presented-as-relevance` (it is not visibility-shaped) but it is the
> same species, and it is undisclosed today.

**Nested projection — REFUSED, not deferred.** A response shaped as a tree, where a selection set
recursively follows edges into the returned shape. **This does not touch edge-filtered traversal or
edge provenance, both of which are in** — see §3's three-way split and the rule that a row may name
its parents but not embed them. The refusal is about the response being a tree, never about whether
edges can be walked.

**Rate-shaped axes — open, inherited.** Nothing here closes over composition depth, plan complexity
or query volume. Decision `019fcd13`'s warning that one statement concentrates all risk in the query
plan is precisely a rate-shaped exposure, and the generative harness measures **shapes, not load**.

**`ActRefusal` composing with `temper_principal::Refusal`** — carried open from contract §5.1, and
still not answered here.

**The act-vocabulary growth risk, named because it is the live version of the GraphQL worry.** If the
vocabulary grows to twenty acts each with their own params, this *is* a hand-rolled query language and
the "more steps" critique lands. Nothing structural prevents that drift — only the discipline of
keeping acts asker-shaped and few. Stated so a future reader can check the count against this
sentence.

---

## 11. What this design does not claim

- **No answer-quality witness.** The standing caution in the frame register is now five-for-five:
  every intervention in this area has moved a proxy and not the outcome. This phase moves a surface.
  A builder over five known acts is the first thing in the arc that *could* be judged by whether a
  question gets answered — which is worth noting without claiming it has been.
- **No clause of the frame register is witnessed by this document.** Designing an executor is not
  exercising one.
- **The act inventory is not widened.** v1 composes the five acts that exist and declares no new ones.
- **`unified_search`'s retirement is phase 1's**, not this phase's, and this design takes no
  dependency on its internals — consistent with the standing note that it *"must not be treated as a
  stable substrate to build against."*
