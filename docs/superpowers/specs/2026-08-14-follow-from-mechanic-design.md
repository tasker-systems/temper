# `follow-from`'s mechanic — the provenance-carrying walk, design

**Status:** design complete. Ships nothing by itself. **One section is deliberately OPEN** (§7) and
is named as a hole rather than filled — see §9.

**`[ruled — 2026-08-14, Pete]` at the start of the build session, before any code.** Four questions
the design left to the builder, each answered where it was raised rather than in a list here:
**depth is definitional and fixed at 2** — no `BoundTerm::Depth`, `accepts_bound_terms` stays
`[Limit]` (§2.1, §5) · **three functions**, the incumbent re-pointed so there is one body (§10) ·
**`p_bound_ids` ships, constraining the whole walk including intermediates**, closing the one
genuine foreclosure (§9). Two conclusions this document previously reached are **retracted** at
their sites rather than quietly edited: §2.1's *"fits `BoundTerm`'s existing model"* and §5's
*"default of 2 with a ceiling of 3"*.

**Task:** [follow-from's mechanic — the walk already carries provenance, so build the sibling that
projects it](./01a00163-c0bb-7651-909f-73e3f33d8a46), under
[Phase 5 — the acts the door cannot reach](./01a0003f-f1a4-7e70-9005-6f8a9c131ee2).
Rides with it: [EdgeFilter grows property predicates — and properties(subject: edge) gets one
home](./01a000c2-033c-7451-8b13-b7aa7469d217).

**Inherited and NOT re-derived:** §3 of
[the compositional design](./2026-08-05-query-builder-compositional-design.md) carries the Task 11
spike's answer, `[verified — 2026-08-14]` against the deployed body on prod. This document begins
where that ends. The one thing it takes from §3 and *changes* is §3's `via`-shape OPEN paragraph,
which §3 said belonged here — settled in §3 below.

## Provenance discipline

- **`[verified — 2026-08-14]`** — read first-hand this session against the working tree at
  `5493c953`, quoted with `file:line`.
- **`[measured on prod — 2026-08-14]`** — a read-only query run against `temper-cloud` (Neon, PG17)
  this session, quoted with its numbers.
- **`[carried]`** — taken from a cited document and not re-opened.
- **`[decided — 2026-08-14, Pete]`** — ruled in session.

---

## 1. The frame this was designed under, because it changed the design

`[decided — 2026-08-14, Pete]` **There is no urgency here and none may be imported.** This is fresh
work with no dependents: nothing outside temper-substrate's tests calls `search_graph_expand`
`[carried — 20260806000010:102]`, and the act is `DoorReach::Absent` at all three doors
`[verified — registry.rs:368-372]`.

An earlier draft of this analysis argued that *"the sibling's first signature is its final one —
every slot it will ever need has to be there on day one, or the next widening is another blocked
deploy."* **The second half is false and it was distorting every scoping decision.** A blocked
deploy comes from a migration that REPLACES or DROPS a shape. Adding a differently-shaped function
under a new name is additive, always. The worked precedent is `20260808000030`, which answered "the
deployed arms cannot express what we now need" by **adding** `__temper_ungated_*` and `query_find_*`
beside `search_exact`/`search_wide` and re-pointing the incumbents via `CREATE OR REPLACE` at
byte-identical signatures.

So getting a signature wrong costs **one more `CREATE FUNCTION` in a later additive migration**, not
a cutover. The one real cost of iterating is drift — *"two bodies drift, and the drift is silent
because both keep returning plausible rows"* `[carried — 20260808000030 header]` — and the same
three-level chain absorbs that too: the newer function holds the body, the older delegates.

**Recorded because the invented constraint produced worse options than the real one.** Every scoping
question below is answered on its merits, never on schedule pressure.

---

## 2. The act: three knobs, three unrelated natures

`follow-from` declares `asker_holds: "a found thing; I want its neighbours"`
`[verified — registry.rs:325]`. Pulling its parameters apart is what dissolves the "how does the
caller configure a walk" question:

| knob | decides | nature |
|---|---|---|
| `depth` | **membership** — a node three hops out is not in the answer at depth 2 | **the definition of the neighbourhood** `[ruled — 2026-08-14, Pete]` — see §2.1 |
| `gamma` | **rank only** — monotone in hop count, so it reorders without changing the set | the definition of the quantity |
| `limit` | how much of the ranked set returns | already declared (`BoundTerm::Limit`, ceiling 50) `[verified — registry.rs:332-334]` |

### 2.1 Depth cannot be composed from outside the walk — and it is not a caller input either

> **`[ruled — 2026-08-14, Pete]` Depth is DEFINITIONAL, fixed at 2. `BoundTerm` does not grow a
> `Depth` variant and `accepts_bound_terms` stays `[Limit]`.** *"Depth 3 seems too large for a
> neighborhood traversal of this kind"* — which is a claim about what `follow-from` **means**, not
> about what a caller may ask for, and that puts depth in gamma's category (§2.2) rather than
> `limit`'s. The structural argument below is unchanged and still load-bearing; what it establishes
> is that depth cannot be **composed**, not that it must be **exposed**. An earlier draft of this
> section concluded the second from the first, and that conclusion is retracted here.
>
> The SQL still carries `p_depth` — the incumbent's signature has that slot (§10's three-level
> ruling re-points it), so the parameter exists at every level and the compiler passes the constant.
> **Fixing it in the definition rather than in the fragment is the point**: one place says 2, and
> `orders_by.means` is where a reader finds out.
>
> A caller-settable depth returns additively if it is ever wanted (§1) — a new `BoundTerm` variant is
> additive on the wire, and clamp-and-disclose is already the mechanism.

`[decided — 2026-08-14, Pete]` The tempting alternative is that depth needs no slot, because a
composition can chain `follow-from` → `follow-from`, one hop each. It is *more* expressive in one
direction (a different `edge_filter` per hop).

**It cannot work, and the blocker is the contract's own rule rather than a limitation to route
around.** Only ids cross a stage boundary — a downstream stage reads
`ARRAY(SELECT id FROM "<stage>")` `[verified — query_plan.rs:703]` and a combinator projects
`SELECT id, kind` `[verified — query_plan.rs:996]`. No quantity ever crosses, which is what keeps
`no-cross-act-ranking` structural rather than policed. A chained walk therefore yields the two-hop
**set** with stage 2's scores recomputed from its own seeds, all at hop 1. **The decayed-path
quantity the act declares it orders by is destroyed at the seam.**

**So a chained walk is not a deeper walk, and that is why the constant lives inside the fragment
rather than being assembled from outside it.** It is the same structural fact as §8.2: things that
must happen *inside* the walk cannot be composed from outside it.

What this argument does **not** establish is that the caller should choose the number. That step was
taken in an earlier draft and is retracted above.

### 2.2 Gamma is not a caller input, and the blocker is concrete before it is philosophical

`[decided — 2026-08-14, Pete]` `pub terms: BTreeMap<BoundTerm, i64>` `[verified — envelope.rs:53]`,
`bound_ceilings: BTreeMap<BoundTerm, i64>` `[verified — act.rs:338]`, and `paging_for` binds
`QueryBind::Int` `[verified — query_plan.rs:797-801]`. **The bound-term mechanism is integer-valued
end to end.** Gamma is `double precision`. It cannot be a bound term without re-typing the whole
term model, which nobody has asked for.

The structural fact points at the right answer rather than merely blocking. `orders_by.means` is a
**fixed sentence** describing what `graph_score` is — *"the best decayed path from any seed to this
node — MAX(gamma^hop \* product of edge weights) over walks of at least one hop"*
`[verified — registry.rs:375-378]`. A caller-set gamma makes that sentence *"decayed at whatever
rate you asked for"*: still true, no longer interpretable, and a caller has no calibration to prefer
0.5 to 0.7. Nothing would stop `gamma > 1`, which inverts the meaning — distant nodes outscoring
near ones under a declaration that says "best path."

**Gamma is part of the answer's definition, not a parameter of the question.**

### 2.3 `asker_holds` is rewritten

`[decided — 2026-08-14, Pete]` The sentence says **a** found thing; the act declares
`accepts_seeds: vec![IdKind::Resource]` — an id *set* — and its own `means` already says *"from any
seed."* A found thing is always a set, even a set of one. The plural is not a detail: **it is the
entire reason `via` exists** (§3). The sentence is rewritten to say so.

---

## 3. `via` is part of the answer, not a disclosure bolted onto it

`[decided — 2026-08-14, Pete]` **This settles the fork §3 of the compositional design left OPEN:
one row per node with a `via` array. Flat `(node, parent, …)` rows are refused.**

§3 gave two arguments (the contract stated in two unlinked places; an arm's output IS the next
stage's input). Three more come from the stage contract and are stronger for being structural:

1. **Only `id` crosses a stage boundary.** Flat rows put duplicate ids into
   `ARRAY(SELECT id FROM "<stage>")` `[verified — query_plan.rs:703]`, and in the **seed** slot
   duplicates multiply the next walk. §3's cardinality-inflation argument is that one line.
2. **`produced` would lie.** The tally is `(SELECT count(*) FROM "<stage>")`
   `[verified — query_plan.rs:1039]` and it is the number a reader uses to judge whether a stage
   earned its place. Flat rows make it a *path* count, over-reporting by the branching factor with
   nothing saying so. **A disclosure that misleads is a harder objection than a cost.**
3. **`quantity` is per row, and the assembler reads it as the hit's score**
   `[verified — query_read.rs:485]`. With flat rows there is no row at which `MAX(score)` is true,
   so the spike's *"`MAX(score)` per node must survive unchanged"* becomes unstatable at row grain.
4. **`ResourceHit` holds one `resource: ResourceView`** `[verified — hits.rs:158-159]`. Flat rows
   hydrate the same resource N times or need assembler-side dedup — the second unlinked place again.

### 3.1 It is always on, with no knob

The declaration says the score is the best path **from any seed**, and the act takes a seed *set*.
So for any multi-seed walk **a bare `(node, score)` row is not a complete answer to the question the
act's own sentence poses** — the caller asked for the neighbours of the things they found and got
back a list that cannot say which. Provenance here is not optional colour; it is what makes the
plural case intelligible.

Cost is the only argument for a knob, and it is measured away in §5.

### 3.2 Every parent, not only the winning path's

`[decided — 2026-08-14, Pete]` `graph_score` is `MAX` over paths; the parent set is a union over
**all** paths. They disagree by construction: a node reached both by a strong 1-hop edge and a weak
3-hop chain reports both parents and a score from only one.

**Naming every parent is the more useful and the clearer answer**, with the non-correspondence
**declared on the field**. The precedent is `RegionHit`, which carries `salience` beside
`region_score` and says outright that the two *"are not rivals"* `[carried — hits.rs:205-208]`.

**This rules out the tempting repair.** A per-parent score on each `via` entry is nearly free to
compute (same `GROUP BY` grain) and is wrong: §3 admits flat provenance precisely because
*"provenance is structure rather than quantity. It introduces no ranking"*, and guards
`member_count` as *"metadata on a row, not a second ranking axis."* A scored `via` entry is that
second axis, inside the row. **`via` carries no numbers.**

### 3.3 `Disclosure` gets its member back

The enum already predicted this: *"`InputContribution` used to lead this enum. Removed with its
`input_contributed` field … **returns when a walk carries origin**"* `[verified — act.rs:297-298]`.
A member returns, and `follow-from`'s `discloses` stops being empty.

---

## 4. What a `via` entry names — the edge **as asserted**

`[decided — 2026-08-14, Pete]` `seed_id` · the edge's own `source_id` / `target_id` ·
`edge_kind` · `label` · `polarity`.

**Reporting source/target rather than a `from_id` plus a direction flag is the load-bearing choice.**
Two fields that must agree are two fields that can drift; the edge as stored cannot contradict
itself, and the parent is derivable from it. `seed_id` is §3's explicit ask — *"which seed each is a
neighbour of, and via which edge"* — and is what makes `means`' "from any seed" answerable.

### 4.1 The walk is undirected over a directed graph, and §3's sketch had no arrow

`adj` unions both orientations (`adj.a = w.node` ∪ `adj.b = w.node`)
`[verified — 20260711000030:47-51]`, so a traversal runs with or against the stored
`source → target`. §3 sketched three columns — `from_id`, `edge_kind`, `label` — none of which says
which way the edge points. `Near` is symmetric and unaffected. `Contains` and `LeadsTo` are not:
*"p contains n"* and *"n contains p"* are different facts.

### 4.2 Polarity is live at 24.7%, which is why it rides

`kb_edges.polarity` exists because the structural arrow and the semantic one can disagree — the
type's own example is *"a `depends_on` edge is asserted source=dependant/target=dependency, but the
causal arrow runs dependency→dependant, so it is `Inverse` `LeadsTo`"* `[verified — graph.rs:43-47]`.

`[measured on prod — 2026-08-14]` over `kb_edges WHERE NOT is_folded` and both endpoints
`kb_resources`:

| kind | edges | inverse |
|---|---|---|
| `leads_to` | 2,417 | **929 (38%)** |
| `contains` | 190 | **166 (87%)** |
| `near` | 1,220 | 4 |
| `express` | 627 | 0 |

**1,099 of 4,454 edges (24.7%) are `inverse`, and `contains` — the most direction-sensitive kind —
is inverse in 87% of cases.** A `via` entry omitting polarity would report the majority of
containment relationships backwards.

**Recorded because the argument was nearly made the other way.** The draft position was that
polarity rides *even if* the count were zero, on the registry's own precedent about `weight`'s
scale: *"today's corpus stays under it because nothing writes such a weight, which is a property of
the DATA, not of the quantity"* `[carried — registry.rs:378-384]`. That reasoning is sound and it
was preparing for a null result. The measurement made it unnecessary — and a null result would have
been an argument for **shipping the bug now and finding it later**.

### 4.3 `label` is nullable in the DDL and 100% populated in prod

`label TEXT` with no `NOT NULL` `[verified — 20260624000001:636]`, and the repo's key convention for
it is `COALESCE(label, '')` in the uniqueness index `[verified — 20260624000001:646]`.
`[measured on prod — 2026-08-14]` every one of the 4,454 edges carries a label, across 68 distinct
values on forward `leads_to` alone. **The nullable handling is still required** — the DDL admits it
and the model may not claim otherwise — but no caller sees a null today. A `labels` filter silently
excludes unlabelled edges, which is correct and should be stated rather than discovered.

---

## 5. Measured: the payload is bounded by the score and the limit

`[measured on prod — 2026-08-14]` Deliberate worst case — the **25 highest-degree nodes** as seeds,
undirected walk, no visibility gate (an upper bound), `via` counted as
`count(DISTINCT (seed, source, target, kind, label, polarity))` per node.

Corpus: 2,570 nodes carry edges; mean degree **3.47**, p50 **2**, p95 **11**, p99 **20**, max **87**.

| depth | nodes reached | path rows | via tuples (all) | max via on one node | **via in the top-50 page** | **max via in page** |
|---|---|---|---|---|---|---|
| 2 | 1,142 | 4,134 | 3,512 | 55 | — | — |
| 3 | 1,619 | 33,684 | 9,434 | 124 | **125** | **9** |

**No cap is needed, and the reason is structural rather than "small corpus."** The nodes carrying
enormous `via` sets are reached by many long weak paths, so `MAX(gamma^hop · weights)` ranks them
near the bottom and the limit sheds them before they are ever described. A high-scoring node is one
hop from a seed, and its `via` set is bounded by how many seeds it is adjacent to.

> **The score and the limit jointly bound the payload, and they bound it in the right direction: the
> rows that would be expensive to describe are exactly the rows that do not come back.**

That retires the cap question permanently rather than until the corpus grows. It also matters
because **capping would silently truncate provenance**, which is the one thing this field exists to
be trusted about.

**And it confirms §2's split.** Path rows went 4,134 → 33,684 for one extra hop (×8, the `b^d`
term); 33,684 path rows collapse to 9,434 distinct tuples, aggregated over rows the walk already
holds. **Depth is the cost parameter; `via` is not.**

> **`[ruled — 2026-08-14, Pete]` Depth is fixed at 2 and is not a caller input at all** (§2.1). This
> section's numbers are what set the constant: the ×8 step to depth 3 is the cost, and *"depth 3
> seems too large for a neighborhood traversal of this kind"* is the meaning. An earlier draft read
> this table as recommending *"default 2 with a ceiling of 3"* — the measurement supports the 2 and
> never argued for the ceiling, which was the draft's own step.

---

## 6. The edge filter is the same two axes, pointed inward

`EdgeFilter { edge_kinds: Vec<EdgeKind>, labels: Vec<String> }` `[verified — filter.rs:26-31]`
narrows *which edges the walk traverses*; `via` reports *which it traversed*. Same pair — a closed
DDL enum beside open free text, which §8 of the compositional design says cannot be merged on live
data.

**They ship together, and not merely because the task bundles them.** A caller explores unfiltered,
reads the labels off `via`, and narrows on what they actually saw. The 68-value label vocabulary
(§4.3) is what makes that loop worth having.

The incumbent carries **one** of the two: `p_edge_types text[]`, compared as
`e.edge_kind::text = ANY(...)` `[verified — 20260711000030:35-36]`. **The label axis has never
existed in the walk.**

---

## 7. OPEN — `PropertyPredicate`'s container, and whether `PropertySubject` survives

> **`[RESOLVED — 2026-08-14, later the same session]` Taken in
> [Property conventions and the predicate container](./2026-08-14-property-conventions-and-predicate-container-design.md),
> which is the argument for the "both halves" branch of §7.3.** The blocker was not knowing where a
> key's convention lives; the answer is that the write path had already decided one axis of it —
> shape conventions are **owner-agnostic** (an edge-owned facet already gets the inner-key grain,
> `20260730000010:195`), projection conventions are **owner-scoped** (`:216`). A shape convention
> therefore lives in an owner-agnostic view that the predicate reads instead of `kb_properties`, and
> `PropertySubject` disappears. **The edge half is the easy one and is independently shippable** —
> no conventions to preserve, zero live rows. The section below is kept as the material that was
> inherited.

**Not settled at the time of writing `[2026-08-14]`.** Stated with its material so the next session
inherits rather than re-derives, per the same rule §3 followed for the `via` fork.

### 7.1 The rule that places edge predicates here

`[carried — task 01a000c2]`, `[decided — 2026-08-14, Pete]`:

> **A narrowing that can be expressed as a set must be an act. A narrowing that cannot be a set
> belongs to the act whose semantics it constrains.**

Resource narrowings became `find-resources-with` because a set expresses them. Edge narrowings
cannot be a set — **they constrain hops** — so they belong to the act whose semantics they
constrain, and **the only act that traverses an edge is this one.** When a question is asked *about
edges* and answered *in resources*, the walk is the sole place edge-level semantics can be spelled.

`PropertySubject` is already `resource | edge | Other(String)`, open by decision
`[verified — filter.rs:106-115]`, and §12 of the compositional design ruled edge-owned properties
**in scope despite this deployment having none** — *"edge properties are used extensively in
deployments beyond this one… A zero count on one dataset is evidence about that dataset, never about
the schema's affordances."* `[measured on prod — 2026-08-14]` still zero: `kb_properties` carries
16,629 rows on `kb_resources` over **70 distinct keys**, 37 on `kb_content_blocks`, **0 on
`kb_edges`**.

### 7.2 The correction that opens the fork

Task `01a000c2` says *"the `resource` half folded into `find-resources-with` when that act landed."*
**On disk that is imprecise, and the imprecision is load-bearing.** `ResourceFilter` carries seven
**named** fields `[verified — filter.rs:60-79]`; `__temper_ungated_find_resources_with` has
`p_doc_types`, `p_tags` and `p_facets`, each hardcoded to one `property_key`
`[verified — 20260814000010:106-118, :189, :212]` — **no open-key slot at all**. And
`capability.rs:353` still refuses `inv.properties` **unconditionally, on every act, for both
subjects**.

So the *named* resource narrowings folded in; **the open-key resource half did not, is served by no
fragment, and has no filed task** — "Task 10b" is named in `capability.rs`'s header and appears
nowhere in the backlog. **67 of the 70 live property keys are unreachable by any narrowing on any
act.**

### 7.3 The two coherent end states

- **Edge half only.** `EdgeFilter` grows predicates; `ActInvocation.properties` survives carrying a
  subject enum whose `edge` variant has moved out and whose `resource` variant nothing serves. A
  field in a strange half-life.
- **Both halves.** Edge predicates into `EdgeFilter`, open-key resource predicates into
  `ResourceFilter` beside its seven named fields — and then **`PropertySubject` disappears
  entirely**, taking `Other(String)` and its `UnknownFilterValue` arm with it.

The second is the completion of §7.1's rule: **a subject tag exists only because the predicate
floats free of a container.** Give it a container and the tag has no job. That it also makes 67
property keys queryable for the first time is a second, independent argument. Its cost is pulling in
a different act's unfiled mechanic.

**Deliberately not ruled here.** Per §1 there is no schedule forcing it, and the fork deserves its
own thinking rather than a decision taken to close a section.

---

## 8. Two traps, and they are different traps

### 8.1 Do not `DROP` and re-`CREATE` the incumbent

`[carried — task 01a00163]` It is the obvious move, **every local test passes** (a fresh test
database applies the whole chain and never evaluates additivity), and it fails at deploy on `main`:
`vercel.json`'s `buildCommand` runs `temper-migrate --additive-only`, which halts at the first
shape-breaking migration and **fails the build**. The consequence is not a red check — it is that
every subsequent deploy on that target is blocked until an operator takes a cutover.

The sibling avoids this, which is why it is **forced rather than chosen** — and why §1's correction
does not touch it. §1 says a *wrong new function* is cheap to replace with *another new function*;
it never says the incumbent may be reshaped.

### 8.2 A node admitted by one edge and walked through another

`[carried — task 01a000c2]` **This one looks identical to success.** Binding `follow-from` by
*"nodes that participate in an edge matching P"* is **not** edge-constrained traversal: a node
admitted because it has a matching edge *somewhere* is then walked through a *different*,
non-matching one. It answers a different question and returns plausible rows.

Its acceptance criterion is carried verbatim: *"witnessed as refused or impossible, not merely
described in a comment."*

**It is the same structural fact as §2.1.** Things that must happen *inside* the walk cannot be
composed from outside it — which is why depth is a parameter and why edge predicates are not an act.

---

## 9. Declared holes and stated silences

- **§7 is OPEN**, not deferred-with-a-plan. Under the outcome-register rule, a clause whose
  mechanism is unbuilt carries **a declared hole, not a filed task**.
- **The open-key resource property half has no filed task**, and this document does not file one —
  it belongs to whoever takes §7's fork.
- **Bounded `follow-from` is CLOSED, and the semantic half was answered rather than assumed.**
  `[ruled — 2026-08-14, Pete]` The sibling **carries `p_bound_ids`**, and a bound **constrains the
  whole walk — every node on it, intermediates included** — not merely the returned set.

  Three things decide it, and the first is that the declaration had already said so: the incumbent
  reads *"walk from these seeds but **stay inside this set**"* `[verified — registry.rs:328-330]`.
  The second is that the walk already has one set-shaped constraint behaving exactly this way —
  `adj` admits an edge only when **both** endpoints are visible `[verified — 20260711000030:37-38]`,
  so visibility constrains intermediates and a bound that did not would be the odd one out. The
  third rules the alternative out rather than merely preferring against it: an output-only bound
  **is `CombineOp::Intersect`**, and a slot for it would be a second spelling of a combinator — the
  precedent being `find-resources-with`, which was given no `p_bound_ids` for that exact reason
  `[verified — 20260814000010 declaration]`. Only the interior reading cannot be composed from
  outside the walk, and that is what earns it a parameter (§2.1, §8.2).

  **The observable difference is worth stating once**: seed → B ∉ bound → C ∈ bound returns C under
  the output-only reading and does not return it under this one.

  > **`[found at build time — 2026-08-14]` The fragment can express this and THE WIRE CANNOT, so
  > `accepts_bounds` does not close yet.** A bounded walk needs two sets at once — seeds to start
  > from, a bound to stay inside — and a stage carries exactly one:
  > `ActInvocation.input` is a single `Option<StageInput>` `[verified — envelope.rs:48]` holding one
  > relation; `capability.rs:196-220` branches exclusively on it, checking against `accepts_seeds`
  > **or** `accepts_bounds`; and `StageNarrowing` is an enum of `Seed(_)` **or** `Bound(_)`
  > `[verified — query_plan.rs:590-608]`. Handed only a bound, the act has no seeds and walks
  > nowhere.
  >
  > Declaring `accepts_bounds: [Resource]` against that wire would name a capability no caller can
  > express — the same falsehood `DoorReach::Absent` was introduced to stop telling, one field over.
  >
  > **`[ruled — 2026-08-14, Pete]` The wire is widened: `input: Option<StageInput>` becomes
  > `inputs: Vec<StageInput>`, at most one per relation.** The relation already distinguishes them,
  > so a bound gets no second spelling — which is why this was taken over the additive
  > `bound: Option<IdSet>` field beside `input`, that being the incumbent literal `bounds` shape
  > this contract deliberately replaced. It reaches the **response** too: the trace's
  > `input_source` and its relation are singular `[verified — trace.rs:74]`.
  >
  > Refused with it: **seeds doubling as the bound**. It needs no wire change and it is a different
  > act chosen silently — every seeded walk would stop returning neighbours outside its own seeds.
- **`survey` is out of scope** — a separate Phase 5 row, and the `sal_norm` re-allocation ruling
  must not be settled by accident here `[carried — task 01a00163]`.
- **The MCP query tool is out of scope** — ⟨2⟩, deferred with the consolidation view.
- **The visibility-hoist strategy is untouched**, still behind its seam.

## 10. What is not measured

- ~~**The with-`via` versus without-`via` execution delta.**~~ **`[measured at build time —
  2026-08-14]` `via` costs +20.0% execution.** Depth 2 (the shipped constant), 25 highest-degree
  seeds, limit 50, ungated: median **315.4 ms with `via`** against **262.9 ms without**, five
  alternating runs per arm with ~1 ms run-to-run spread inside each — so the delta is far outside
  noise. At depth 3 the ratio holds (4,786 ms against 3,954 ms, +21%).

  **What was measured and what was not.** The real function body, not an approximation — that was
  the objection this bullet originally recorded, and building it removed it. The *without* arm is
  the same body with only the `via` subquery deleted, because an arm that merely projects two of the
  three columns measures column pruning rather than the work. **Not** `pg_stat_statements` and
  **not** prod: the extension is preloaded on Neon and absent from the local Docker image, and the
  function is not deployed anywhere with a real corpus. The corpus is synthetic, matched to §5's
  measured shape on edge count (4,454), p50 (2), p95 (11) and max degree (87), with a **heavier p99
  tail (33 against prod's 20)** — so the *absolute* milliseconds are this corpus's, while the
  *ratio* is robust because both arms run the identical corpus and seed set. A first attempt used an
  uncapped skew, produced a single node of degree **604**, and was discarded rather than reported:
  a hub 7× prod's heaviest dominates every timing, and being wrong in the direction that makes the
  number look worse is not conservatism.

- **`[found while measuring — 2026-08-14]` The re-pointed incumbent pays for `via` and throws it
  away.** `search_graph_expand` projects two of the core's three columns, and the planner does
  **not** prune the discarded one through the recursive CTE: 314.8 ms projecting two columns against
  321.5 ms projecting three — i.e. it pays the full +20%, not a fraction of it.

  **This is a consequence of §10's three-function ruling that the ruling did not name**, and it is
  accepted rather than fixed: the incumbent's only callers are temper-substrate's own tests, so
  nobody pays it. It is recorded because *the reason it is free is a fact about today's callers* —
  the moment `search_graph_expand` acquires a real one, that caller inherits a 20% surcharge for a
  column it cannot see. The alternative was letting the incumbent keep its own body, which is the
  drift the ruling exists to prevent; paying 20% on a function nothing calls is the cheaper side of
  that trade, and saying so is what stops a later reader mistaking it for an oversight.
- **Anything under a visibility gate.** §5's numbers are ungated upper bounds; a real walk sees a
  subset.
- ~~**Whether the sibling should be two functions or three.**~~ **`[ruled — 2026-08-14, Pete]`
  THREE**, and the third level's job here is not the find family's.

  There, the top level exists because `/api/search` must keep its shape
  `[carried — 20260808000030 header]`. Here `search_graph_expand` has no door and no production
  caller at all — *"nothing outside temper-substrate's tests calls it"*
  `[verified — registry.rs:356, 20260806000010:104]`. Its top level earns its place for the **other**
  reason that migration gives: **ONE BODY PER ARM.** Left alone, the incumbent is a second walk that
  must agree with the new one and is linked to it by nothing — *"two bodies drift, and the drift is
  silent because both keep returning plausible rows"*. Re-pointed by `CREATE OR REPLACE` at its
  byte-identical signature, it delegates, and its existing tests start exercising the real body.

  So: `search_graph_expand` → `query_follow_from` (gated) → `__temper_ungated_follow_from` (core).

  **The consequence to design for, not around: the core must carry `p_gamma`,** because the
  incumbent's signature has that slot and delegation means passing it through. That does not
  re-open §2.2 — gamma stays fixed in the act's definition and the compiler passes the constant. A
  parameter the fragment accepts and the act never exposes is exactly what §2.1 now also says about
  `p_depth`.

  What was **not** open, and still is not: the core takes `p_visible_ids uuid[]` and never
  `p_principal`, or an N-stage composition pays N `Recursive Union` team closures, which the hoist
  exists to prevent.
