# `TemperQueryBuilder` — the compositional surface, v0 design

> ## `[SUPERSEDED IN PART — 2026-08-08]` The wire contract now lives in `internal/api/query.openapi.yaml`
>
> **Ground in [`internal/api/query.openapi.yaml`](../../api/query.openapi.yaml) for anything about the
> request or response SHAPE.** It is hand-written, reviewed field by field, and it is the committed
> target. This document remains authoritative for the *reasoning* — why a DAG and not an expression
> language, why the executor does not transfer, why GraphQL is deferred — and for everything below
> the wire.
>
> Six shapes this document (and the types it produced) got wrong, each amended there with its
> argument:
>
> | this document says | the contract says |
> |---|---|
> | `OutcomeDeclaration.description` is required | removed — register discipline in a wire contract |
> | `ReturnSpec.fields: [String]` subselects | `with: [open-meta]`; `body` refused, `edges` dropped |
> | `bounds_mode` sits on the invocation | `StageInput.as`, so "input without a relation" cannot be expressed |
> | `on_stage_refusal` is required | removed — all but one `RefusalReason` are static; `embedding_unavailable` is the single runtime refusal, and the field's two settings were observationally indistinguishable for that one case |
> | a stage with no intention refuses | the server embeds, and a FAILED embed is the one runtime refusal |
> | the trace keys stages by ordinal | keyed by `StageName` — the caller's own vocabulary |
>
> **And one defect this document's types have that is not a shape question:** `bounds_mode` is read
> by the validator and by the compiler *nowhere*. `narrowing_for` routes every upstream set to the
> narrowing slot, so a `seed` compiles as a `bound`. Latent only because `follow-from` is unbound.

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

**Contract authority:** `internal/superpowers/specs/2026-08-03-query-envelope-contract-v0-design.md`.
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

> `[superseded — 2026-08-10]` The whole section below — `survey` producing `IdKind::Region`, the
> region row and `region_score`, `survey` as `Indeterminate`, and the region-row projection — is
> superseded by ratification ⟨3⟩ (2026-08-09): `survey` produces the member **resources** of its
> salience-matched regions; regions are trace disclosure, not a returnable currency. See the
> RATIFICATION block in [`internal/api/query.openapi.yaml`](../../api/query.openapi.yaml).

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

   > **`[corrected — 2026-08-14]` Those three columns are not enough, and the shortfall is
   > directional.** The walk is undirected over a directed graph (`adj` unions both orientations),
   > so `from_id, edge_kind, label` carries no arrow — and `contains` / `leads_to` are asymmetric.
   > `[measured on prod — 2026-08-14]` **24.7% of resource-resource edges are `polarity = inverse`,
   > and `contains` is inverse in 87% of cases**, so the sketch would report the majority of
   > containment relationships backwards. An entry names the edge **as asserted** — `seed_id`,
   > `source_id`, `target_id`, `edge_kind`, `label`, `polarity`. See the mechanic's design §4.
3. **Nested projection — REFUSED.** A response shaped as a tree, where the caller declares a
   selection set that recursively follows edges into the returned shape, is where a GraphQL rebuild
   actually begins, and no caller has asked for it.

> **The rule that separates 2 from 3: a row may NAME its parents; it may not EMBED them.** A `via`
> entry carries ids and edge metadata, never a hydrated resource.

**The check, answered — `[verified — 2026-08-14]`, Task 11, against the deployed body on prod
(read-only).** The question was whether `search_graph_expand` can emit path provenance, given that
`follow-from`'s mechanic is a `MAX(score) GROUP BY node` over the walk `[carried — decision
019fd2ea]` and therefore **collapses paths by construction**.

**The answer is STATE 2 — available with an additive change.** The collapse is real but it is a
property of the **final projection only**; the recursion retains everything needed:

```sql
walk AS (
  SELECT s.id AS node, 1.0::double precision AS score, 0 AS hop, ARRAY[s.id] AS path
    FROM unnest(p_seed_ids) AS s(id) WHERE s.id IN (SELECT id FROM visible)
  UNION ALL
  SELECT nb.node, w.score * p_gamma * nb.weight, w.hop + 1, w.path || nb.node
    FROM walk w JOIN LATERAL (...) nb ON true
   WHERE w.hop < p_depth AND NOT nb.node = ANY(w.path)
)
SELECT node, MAX(score)::real FROM walk WHERE hop > 0 GROUP BY node;   -- ← the only collapse
```

`path` is carried through the entire walk, so per walk row the **seed** (`path[1]`), the **immediate
parent** (`path[array_length(path,1)-1]`) and the full ancestry are all already present. **State 3 is
ruled out: the walk needs no restructuring**, and `via` therefore does not have to be deferred.

Two things do stand in the way, and both are additive:

1. **Edge metadata is discarded, not absent.** The `adj` CTE projects `(a, b, weight)` only, dropping
   `e.edge_kind` and `e.label` — both of which exist on `kb_edges` (`edge_kind` the enum, `label`
   `text`). Carrying them widens `adj` and the walk row by two columns.
2. **A sibling function is forced, not chosen.** `RETURNS TABLE(resource_id uuid, graph_score real)`
   cannot be widened in place — `CREATE OR REPLACE` raises *"cannot change return type of existing
   function / Row type defined by OUT parameters is different"* `[probed — 2026-08-14]`, and
   `DROP`+`CREATE` is shape-breaking, which the additive-only build gate rejects on `main`. So
   provenance ships beside the incumbent, which is also the safest shape: the deployed walk stays
   byte-identical and no current caller can be disturbed.

**`via` as an array is reachable, as this section requires.** Because every path survives in `walk`,
aggregating the distinct `(parent, edge_kind, label)` triples per node yields the true parent set —
so first-wins, correctly named here as a silent lossy pick, is not forced by the mechanic.

**Two constraints the mechanic must carry, discovered by this spike and belonging to its spec:**

- **`MAX(score)` per node must survive unchanged.** A provenance variant that ranked differently
  from the incumbent would leave two functions disagreeing about ordering — the drift this codebase
  has repeatedly spent migrations removing.
- **Cost scales with PATH count, not parent count.** `walk` holds one row per path; at depth *d* and
  branching *b* that grows ~*b^d*, while the distinct parents it collapses to stay small. Cheap on
  the community corpus (`kb_edges` 4,852 rows over 3,738 resources, average degree ≈2.6) and the
  term that grows on a dense graph. **Now measurable per-statement** — `pg_stat_statements` was
  never installed until migration `20260814000020` (PR #675).

> **`[SETTLED — 2026-08-14]` One row per node with a `via` array; flat rows are refused.** Taken in
> [the mechanic's own design](./2026-08-14-follow-from-mechanic-design.md) §3, which is where the
> paragraph below says it belongs. Three arguments were added to the two here, all from the stage
> contract: only `id` crosses a stage boundary, so flat rows put duplicate ids in a seed slot; the
> `produced` tally is `count(*)` and would report a *path* count; and `quantity` is per row, so
> `MAX(score)` would be true of no row. The paragraph below is kept as the material that was
> inherited.

**OPEN, and deliberately not settled by this spike** — whether the variant emits **one row per node
with a `via` array** or **flat `(node, parent, …)` rows aggregated above it**. Phase 5's rule is that
*"the spike precedes its mechanic"*, so this belongs to the mechanic's own spec. The material it
should inherit: flat rows put the contract *"one row per node, score is the max, `via` is the parent
set"* in two places that nothing links, and — the stronger argument — **in a composition an arm's
output IS the next stage's input**, so path-count rows would inflate every downstream stage's
cardinality and multiply a node's rank by how many ways it was reached. That points at the array,
from the composability premise rather than from cost.

`search_graph_expand` remains untouched by phase 1, so any change to it collides with nothing.

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
`readback/mod.rs` carries runtime `sqlx::query` / `query_as` reads — `vector_search`, `search_wide`,
`wayfind_region_diagnostics` — and its module note is explicit about their status: they are runtime
*"for one allow-listed reason only: a `$n::vector` bind the macros cannot type. **That reason is the
whole of the class.** A runtime read that cannot state it is drift, not an exception, and belongs as
a macro."* The note also records what happened when the exemption spread: *"Until 2026-07-30 sixteen
further reads in this module were runtime 'for consistency' with the vector ones, which left them
absent from the cache — and the cache is the record a schema/binary change detector reads, so an
exemption taken for tidiness was subtracting from the coverage of a safety check."*
`[verified — 2026-08-07, crates/temper-substrate/src/readback/mod.rs module note]`

**That class admits only what can state its ground, and the discipline is enforced rather than
described.** `search_exact` binds a principal, query text and an anchor pair — **no vector, no
cast** — so the class's stated reason never covered it, and it is a `sqlx::query!` macro with
`"resource_id!"` / `"fts_norm!"` overrides and an entry in the workspace cache, not a runtime read
`[verified — 2026-08-07, readback/mod.rs, the search_exact read]`.

So the builder is a **runtime read with a second, different reason** — dynamic composition rather
than a `::vector` bind. **Amending that module note to name the second class is a required step of
the build, not documentation hygiene.** Left unamended, the note instructs the next cleanup sweep to
claw the builder back exactly as it clawed back the sixteen, and it would be right to.

**The module note is the authority, and its members are counted from the file, never from prose
here.** The note says so itself — *"Count the members by reading the file, never by trusting a number
written in prose — here or in any document describing this module"* `[verified — 2026-08-07,
readback/mod.rs module note]`. This section names the incumbent reads to identify the class; it
carries no number out. When Task 10 lands, read the note there.

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
| **F** | **done** | Reconcile act declarations — landed in phase 1's own step 5, before this phase begins |

**Coordination points with phase 1, and only these two.** D consumes the sibling's two new functions.
F is the same declaration reconciliation phase 1's own step 5 owns — one reconciliation, not two —
and it is done `[verified — 59ee9100, "Phase 1 step 5 — the declarations describe the deployed system
again, and offset stops being ignored"]`: `find-exact`, `find-about-anywhere` and `find-about-within`
are `BuildState::Served` with `served_by` `search_exact` / `search_wide`
`[verified — registry.rs: the FindExact, FindAboutAnywhere and FindAboutWithin declarations each
carry build_state: BuildState::Served]`. It did not wait for `/api/query`, because the declarations
became wrong the moment the read path moved — *"The declarations did not have to wait for step 4 to
become wrong — they became wrong when the read path moved"* `[verified — 59ee9100 commit body]`.
What step 5 did **not** reconcile is `follow-from` and `survey`; that remainder is the subsection
below, and it is this phase's.

### The state between C and D, and the `BuildState` gap it lands on

**The live instance of the gap is `follow-from` and `survey`.** Both declare
`provisionally_unexpressed()`, a helper whose only job is to record the imprecision at the point where
it lives rather than hide it: it returns `BuildState::Fused { host: "unified_search" }` as the
least-wrong of the available values, and says so — *"Their mechanics are live — `search_graph_expand`
and `wayfind_region_scores` are both still deployed — but as of phase 1 steps 2-3 **no door reaches
either one**. … So `Fused` is now false in BOTH its clauses — there is no host, and it has no door —
while `Unbuilt` would still be false about a function that is right there, and `Served` would still be
false about a door that does not exist."* `[verified — registry.rs, provisionally_unexpressed and its
two call sites, the FollowFrom and Survey declarations]`. The host string names nothing:
`unified_search` was dropped from the schema
`[verified — migrations/20260806000010_retire_unified_search.sql]`. A fourth variant is deliberately
**not** minted there, on the grounds that *"`/api/query` is the next phase and is being taken up
immediately; it gives both acts doors and expresses this remainder properly, so a variant minted here
would be born obsolete"* — tagged `[provisional — 2026-08-05; resolve in phase 4]`, which is this
phase.

**Two consequences for the builder, and they point in opposite directions.** A validator that keys on
the `BuildState` discriminant — *refuse anything `Fused`, the builder cannot reach into a composite* —
refuses exactly the two acts beat C exists to execute, because the `Fused` they carry is a placeholder
and their `served_by` values name standalone functions the builder calls directly. And a plan naming
`find-exact` between beats C and D must still refuse, because the compiler emits no fragment for
`search_exact` / `search_wide` until beat D — but `NotImplemented`, documented as *"the act is declared
but not built (`build_state` is not `served` or `fused`)"* `[verified — disposition.rs:64-65]`, is
false of it: the act is `Served` `[verified — registry.rs, the FindExact declaration]`.

**This is the same `BuildState` gap phase 1 flagged, and phase 1 states it in the same words** —
*"`follow-from` and `survey` are in a state `BuildState` cannot express: their mechanics … are live and
deployed, and no door reaches either"* `[verified — 59ee9100 commit body]`. The missing distinction is
unchanged: **`BuildState` conflates whether a mechanic exists with whether the asking surface can
reach it.**

Named, not solved here. Two things follow for the plan: `RefusalReason` is **open**
`[verified — disposition.rs, and decision 019fcd13]`, so a variant for *"declared, built, not
reachable from this surface"* is additive and can be added when beat C needs it; and **the settling
falls to this phase**, once, since fixing it twice differently is the outcome to avoid. Phase 1
reached the gap first and **declined on purpose**, leaving `provisionally_unexpressed()` and its
`[provisional — 2026-08-05; resolve in phase 4]` tag in place because widening `BuildState` is
*"semver-breaking on a shipped contract type"* and a variant minted there would be *"born obsolete"*
`[verified — 59ee9100 commit body, and provisionally_unexpressed's doc comment]`. **How** it settles
is deliberately still open — the `RefusalReason` variant just named, or the doors of beat E, which is
what `provisionally_unexpressed()` itself anticipates.

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

**The `ResourceFilter` overlap — named, and deliberately not resolved here** `[2026-08-05, Pete]`.
§12 admits an open property predicate while `ResourceFilter` keeps typed slots for `doc_type`,
`stage` and `status` whose whole purpose is closed-vocabulary refusal. A caller can therefore address
one of those keys through the open predicate and receive a confident empty where the slot would have
raised `UnknownFilterValue`. **This is a real hole, left open on purpose:** `ResourceFilter` is a
stopgap for queryability that §12's semantics supersede, and bolting a guard onto a struct we are
moving beyond would entrench it. The convergence — `temper resource list --properties` and the
composition predicate sharing one semantics — is its own work. Recorded so the gap reads as a
decision rather than an oversight, and so whoever takes that work knows it is the first thing to close.

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

---

## 12. Properties are queryable — open keys, closed operators

`[added — 2026-08-05, Pete]` A scope addition taken during plan authoring. It **grows the surface and
does not change its shape**: a property predicate is one more typed narrowing slot alongside
`ResourceFilter` and `EdgeFilter`, validated statically and compiled into the same statement.

### What the data says

`[measured on prod — 2026-08-05, temper-cloud]` `kb_properties` is polymorphic — `(owner_table,
owner_id, property_key, property_value jsonb, weight)`. Live, unfolded: **15,732 rows over 71 distinct
keys**, values spanning `string`, `array`, `object`, `number` and `null`.

**The indexes already exist, and they are key-agnostic** `[verified — pg_indexes, 2026-08-05]`:

```
idx_kb_properties_key        btree (property_key)          WHERE NOT is_folded
idx_kb_properties_owner      btree (owner_table, owner_id) WHERE NOT is_folded
idx_kb_properties_value_gin  gin   (property_value jsonb_path_ops)
```

**This is the fact that decides the design.** Declaring a first-class list of supported keys and
building GIN indexes for them — the obvious approach — solves a problem the schema already solved
generically: the GIN covers values across *all* keys, and the btree makes key-filtering cheap for any
key. A declared list would buy nothing and would go stale against a vault whose keys are user-defined
by design.

**So: the key space is OPEN and the operator set is CLOSED.**

### The operators

| operator | compiles to | index |
|---|---|---|
| `has_key` | `EXISTS (… WHERE property_key = $k)` | the `property_key` btree |
| `contains` | `property_value @> $v::jsonb` | the `jsonb_path_ops` GIN |
| `weight_at_least` *(phase 2)* | `weight >= $w` | none — see below |

Both v1 operators take **bound parameters only**. No identifier, no operator and no fragment of
Postgres JSON syntax is ever assembled from caller text — which is the whole reason a jsonpath or
jq-shaped string parameter was refused rather than sandboxed.

**`has_key` needs no jsonb operator at all**, and that is worth stating because the obvious reach is
wrong: `jsonb_path_ops` deliberately does **not** support the key-existence operators (`?`, `?|`,
`?&`), so `?` would have meant a second GIN index for a question the `property_key` btree already
answers as a row-existence check.

### The subject is carried by the predicate, and its vocabulary is open

`[decided — 2026-08-05, Pete]` A predicate names what it addresses:

```
PropertyPredicate { subject: PropertySubject, key: String, op: PropertyOp }
```

**Carried, not inferred**, because inference is ambiguous exactly where it matters: a `follow-from`
stage walks **edges** and produces **resources**, so "the properties of this stage's subject" has two
answers.

**`PropertySubject` is OPEN** — `resource`, `edge`, and an unknown value renders
`RefusalReason::UnknownFilterValue` rather than failing to deserialize. This is the **opposite** call
from `EdgeKind` (contract §4.1.2, closed so that `"advances"` cannot be constructed), and the
difference is principled rather than inconsistent: `EdgeKind` mirrors a **DDL enum**, so its
closedness is a fact about the database; `owner_table` is a **varchar** that mirrors nothing, so a
closed set here would be a claim the schema does not make.

**Edge-owned properties are in scope even though this deployment has none.**
`[corrected — 2026-08-05, Pete]` The measurement returned **zero** rows with
`owner_table = 'kb_edges'`, and the first draft of this section concluded an edge-property filter was
a declared-empty affordance. That was wrong: **edge properties are used extensively in deployments
beyond this one**, and the polymorphic owner is the design intent rather than an accident. A zero
count on one dataset is evidence about that dataset, never about the schema's affordances. The shape
is identical for both subjects, so admitting `edge` costs a variant and no mechanism.

### Content-block properties are addressable but not queryable

`[decided — 2026-08-05, Pete]` `kb_content_blocks` is the third owner in the data (`block_role`, 37
rows) and is **deliberately excluded**. Block-level properties exist so that provenance can attach to
*part* of a resource or claim rather than the whole of it — addressability, which is a different
affordance from being a queryable subject. Naming the exclusion so its absence reads as a decision.

### Two measured hazards

**The key space is not type-stable.** Three of the 71 keys carry more than one JSON type:
`derived_from` is an `array` on 112 resources and a `string` on 21; `temper-pr` is a `string` on 57
and `null` on 7 `[measured on prod — 2026-08-05]`.

Containment is honest about this rather than coercing — `@> '"x"'` matches the string rows, `@>
'["x"]'` matches the array rows, and `'["x"]'::jsonb @> '"x"'::jsonb` is **false**. So a caller
asking the natural way against a type-unstable key gets a partial answer with no signal. The
mitigation is that **the predicate's value carries its own shape**: the caller sends the JSON they
mean, so scalar-vs-array is explicit in the request rather than guessed by the server. `contains`
takes a **list** of values, OR-composed within the predicate — matching the established
within-field-OR of `doc_type` and `EdgeFilter.labels` — so one predicate can span both shapes of a
type-unstable key.

> **Unverified, and a build step rather than a claim:** whether `property_value @> ANY($1::jsonb[])`
> uses `idx_kb_properties_value_gin`, or degrades to a scan. If it does not, the OR is emitted as
> repeated containment terms instead. Measure with `EXPLAIN` before choosing the emission.

**`weight` is real and phase-2.** `facet` is the only weighted key — **342 of its 1,233 rows carry
`weight <> 1.0`** `[measured on prod — 2026-08-05]`. Contract §4.1.2 declared a richer facet
predicate "not undertaken… nothing in the search family needs it"; that is now superseded in
direction, since a caller does. `[decided — 2026-08-05, Pete: build it, but it may be a later phase
so long as the design admits it.]` The design admits it by making `PropertyOp` an enum with room for
`weight_at_least`; **no index serves it today**, so whether it needs one is a measurement that
belongs with its build and not before.

### What this does not do

- It does not touch `ResourceFilter`'s existing typed slots. The overlap and the closed-vocabulary
  bypass it creates are recorded in §10 as a deliberate open hole with its convergence direction.
- It admits no operator whose value is a fragment of a query language. `has_key` and `contains` are
  the whole vocabulary, and both bind.
- It does not make properties **orderable**. A property is a narrowing predicate, never a quantity an
  act orders by — admitting one would put a caller-chosen number beside an act's declared quantity,
  which is the cross-act comparison the frame register forbids.
