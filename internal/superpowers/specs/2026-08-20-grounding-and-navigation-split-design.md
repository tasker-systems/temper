# Grounding and navigation are different acts — the graph surface split

`[2026-08-20]` Design spec. Supersedes nothing; extends the successor surface built in Beats 0–D
(`2026-08-20-graph-successor-surface-design.md`).

Sits under [The graph surface shows the reader's own material](./019fbaac-96e2-7620-ace2-667a0f8ff000).
Filed from [The unaddressed entry draws 244 unconnected marks of 250](./01a0215d-75aa-7db2-8df0-088832b46485),
which is where the investigation started and which this spec outgrew.

---

## 1. The thesis

> **A composition grounds you. It does not navigate you.**
> `[ruled — 2026-08-20, Pete]`

Asking a question sets a space. Moving around inside that space afterwards is a different act with
different needs, and today the surface has only the first one. Every movement re-runs a composition,
so the reader is locked inside one answer's frame and every hop pays for a full query-and-render
cycle.

This spec splits the two, names what each door owes the reader, and disposes of the endpoints Beat D
orphaned.

## 2. What started it, measured

`/graph/@me`, question box empty, production, `[measured — 2026-08-20]`:

```
Showing 200 of 3561 across your places · 50 followed on · more exist ·
groupings not applicable · 12 of 12 places
```

**250 nodes drawn, 244 unconnected.** The reader's word for it was *"a disaster"*.

### 2.1 The mechanism, which is not a rendering bug

`follow-from` returns edges connecting a **walked node to the seed it was reached from**. The
unaddressed entry seeds that walk from `find-resources-with` over *every visible resource* (3561),
while drawing only 200 rows fetched separately from `GET /api/resources?limit=200`
(`graph-query.ts:36`, `readSeedRows` with `addressed: false`). That list is ordered
`r.updated DESC, r.id ASC` (`substrate_read.rs:354`) — pure recency.

So **the drawn set and the walked set are chosen by unrelated criteria**, and nearly every edge has
one endpoint off-canvas and is dropped. The 244 are not unconnected in the corpus; they are
unconnected *in this drawing*.

Recency is close to anti-correlated with structure: what a person touched this week is spread across
contexts by construction, while things that link to each other were mostly written at different
times.

### 2.2 The answered state does not have the problem

Same canvas, same marks, same band component, `?q=graph surface legibility`:

| | unaddressed | question asked |
|---|---|---|
| drawn | 250 | 130 |
| unconnected | **244 — 97.6%** | **45 — 35%** |
| groupings | `not applicable` | 15, twelve named, 3 declared unlisted |

`survey` runs a semantic funnel (`regions: 3` per anchor), so the selection is regionally coherent
and the edges land. **Any fix applied at the canvas would apply to both states, and the answered
state does not want one.**

### 2.3 The rejected fix, and why it was rejected

A goal-seeded walk was measured and looked excellent — 65 goals → 50 walked, 56 drawn, **1
unconnected**. It was rejected `[ruled — 2026-08-20, Pete]`:

> *"even I … have contexts that are not workflow-oriented contexts, they are massive document
> corpuses — so default assuming goals is not the right choice."*

`doc_type: goal` is a **workflow-shaped assumption**. A document corpus has no goals, and seeding on
them would fail `entry-does-not-presume-organization` for exactly the actor the register singles out
as the one the predecessor failed. Recorded because the measurement is flattering and someone will
propose it again.

## 3. Three corrections — the composition door is not the graph door

Each of these was asserted during design and is wrong. They are wrong in one direction: true of
`/api/query`'s vocabulary, false of the `/api/graph/*` reads. **That asymmetry is the thesis, found
the expensive way.**

| Claimed | Actually |
|---|---|
| Region membership is walled off (§D5 *"identities stay interior"*) | `GET /api/graph/regions/composition` — *"Read the resources composing a region"* → `AtlasSubgraph` |
| Degree needs new Rust and new SQL | `AtlasNode.degree` — *"the node's total visible edge count"*, `COALESCE(deg.degree, 0)`, returned today |
| Edges have no id or durable address | `AtlasEdge { id, source, target, edge_kind, polarity, label, weight }` |

The third is the sharpest. In the composition vocabulary an edge is `ViaEntry` — one entry per
*(seed, edge)* **pair**, measured at 1,973 entries collapsing to 102 distinct edges, a 19.3×
inflation, with no id anywhere. Edges there are walk *provenance*, not objects. **A surface whose
subject is edges cannot be built on that**, and the orientation view's subject is edges.

## 4. Disposition of the nine orphaned endpoints

Beat D left nine `/api/graph/*` endpoints with zero callers. They are not one group.

**Keep — frame-neutral navigation reads.** None presumes goals, tiers or territories:

- `GET /api/cogmaps/{id}/graph/slice` — seeds + BFS depth + edge-kind filter → induced subgraph
- `GET /api/graph/cogmaps/{id}/panorama` — a map's interior
- `GET /api/graph/regions/composition` — region ids → subgraph
- `GET /api/graph/home` — the reader's teams and maps

**Delete — the retired frame.** `GET /api/graph/contexts/panorama` and
`GET /api/graph/contexts/composition`: *"goal-container territories and residuals"*, containers
defaulting to `["goal"]`. This is the tier model Beat D deleted **and** the workflow assumption
rejected in §2.3. Two reasons, independently sufficient.

This amends the disposition recorded in
[Nine /api/graph/* endpoints now have zero callers — the Rust half of Beat D's deletion](./01a02131-9c5f-7a70-88f0-9c98df4a8a88)
from *delete nine* to **delete the territory pair, keep and caller the rest**.

### 4.1 The canvas was already built for this shape

`model.ts` on the successor's own node type: the flattened fields are *"named exactly as `AtlasNode`
names them so the surviving marks need no adapter."* Beat B built the mark vocabulary against
`AtlasNode`. This is not a coincidence to exploit quietly — it is why the migration is cheap, and it
should be stated in the PR rather than discovered.

## 5. The two reads that do not exist

Everything else is present. Two gaps, both in the same SQL family as the four kept reads.

### 5.1 The entry read — orientation

> **A place, and no question at all — show me what its work is built around.**

Returns the **K most-connected resources the reader can see, plus every edge among them**, as an
`AtlasSubgraph`. No seeds required; this is the door for a reader who has supplied nothing.

Three constraints on the degree computation, each with a reason:

1. **`WHERE NOT is_folded`** — folded edges are retracted assertions. Omitting this counts things
   somebody took back, and it would never surface as a bug, only as subtly wrong ordering.
2. **`[corrected — 2026-08-20]` The induced subgraph is what protects §2.1 — not the ranking
   metric.** This constraint first read *"degree must be measured over the set that will be drawn"*,
   which is **circular**: the drawn set is chosen *by* the ranking, so the ranking cannot be scoped
   to it. The rule that actually holds:

   > **Rank by corpus degree; return the induced subgraph over the top-K.**

   Every drawn edge then has both endpoints drawn **by construction**, which is what §2.1 was
   protecting. Whether the top-K by corpus degree actually cohere is an **empirical** question, not
   something to assert here — see the reporting requirement below.
3. **Both endpoints `kb_resources`** — `kb_edges` may target `kb_cogmaps`, which are not drawable
   resource marks.

**`[ruled — 2026-08-20, Pete]` A reuses the incumbent node shape.** `graph_atlas_nodes_visible(p_profile, p_ids)`
already returns the whole payload — `id, title, doc_type, home, degree, first_chunk, stage`. A is
therefore a **ranking function returning ids**, then the incumbent for hydration. One node shape
across every graph read, and no second place for it to drift.

**A must report the degree distribution, not just the top K** — including, for the returned set,
both the corpus degree and the induced-subgraph edge count. K and rung 2's threshold (§6, §10.1) are
fixed in C from those numbers, and the two figures side by side are what say whether ranking by
corpus degree produces a set that actually coheres.

Existing indexes serve this: `idx_kb_edges_source`, `idx_kb_edges_target`, `idx_kb_edges_home`, all
`WHERE NOT is_folded`.

**Degree counts what this caller can see.** Precedent is set and should be followed rather than
re-argued: `member_count` is *"over the members **this caller can read**, never all of them… Two
readers of the same region can legitimately see different numbers; that is the point."*

**K is chosen against a measurement, not picked.** Follow `ANCHOR_CEILING`'s precedent — that number
was set from the heaviest real reader's 12 anchors, and a ceiling that fires routinely would make an
ordinary act silently change what the door asks.

### 5.2 The traversal read — `[corrected — 2026-08-20]` the SQL already exists

An earlier draft of this section said a visibility-scoped slice was missing. **It is not.**
`cogmap_neighborhood_slice` is bound to one cogmap, but the function underneath its family is not:

```sql
-- canonical_functions.sql:1308
CREATE OR REPLACE FUNCTION graph_traverse(p_profile uuid, p_seed_ids uuid[], p_depth int)
RETURNS TABLE (resource_id uuid, source_id uuid, target_id uuid,
               edge_kind edge_kind, polarity edge_polarity, label text, depth int)
```

It is already visibility-scoped (`resources_visible_to(p_profile)`, both endpoints checked), already
excludes folded edges, already restricts to `kb_resources` on both ends, and **is bound to no anchor
at all.** B is therefore a service function and a handler over an existing fragment, not new SQL.

**`[ruled — 2026-08-20, Pete]` The directed chain is retired; one walk body survives.**

`graph_traverse` walks **forward only** — base arm matches `e.source_id = ANY(p_seed_ids)`, recursive
arm joins `e.source_id = w.target_id` — while the composition's `__temper_ungated_follow_from` is
*"Undirected adjacency over admitted, unfolded, `kb_resources` edges"*, joining `admitted` on **both**
endpoints. Two walks that disagree, and the repo already ruled on that shape:

> *"two walks that must agree and are linked by nothing will drift silently, both still returning
> plausible rows"* — `20260814000030`

**The blast radius turned out to be nil.** `graph_traverse`'s only consumer is `graph_subgraph_nodes`
(`canonical_functions.sql:1348`), and **`graph_subgraph_nodes` has no Rust caller at all** — grep
across `crates/` returns nothing outside `.sqlx`. So the directed chain is already dead.

`[corrected — 2026-08-21]` **It is deader than that.** `graph_subgraph_nodes` is not in the database
at all — `pg_proc` returns zero rows for it, so a later migration already dropped it. Only
`graph_traverse` itself remains for E to retire.

It is **retired in chunk E** alongside the territory pair. B gets undirected-and-induced, which is
what a neighbourhood read wants, and nothing regresses because nothing consumed it.

**One constraint on how B gets there.** B must **not** call `__temper_ungated_follow_from` directly:
that prefix is *"source discipline enforced by `audit-ungated-fragments.sh` and a single compiler
emitter, NOT a database permission"*, and the graph door is not that emitter. Either extract the
shared adjacency body, or link B's `graph_*` wrapper to it by a test that fails on drift. The
precedent is re-pointing at a byte-identical signature — **one body, two names**.

**Neither walk yields `AtlasEdge`.** `graph_traverse` returns no edge `id` or `weight`;
`__temper_ungated_follow_from` returns `via` naming edges *as asserted*, deliberately carrying no id
and no numbers. Both gaps are closed by the induced-edge read in §5.4.

### 5.4 The induced-edge read — `[corrected — 2026-08-21]` it already existed too

This section called for "a sibling taking an arbitrary node-id array". **That sibling was already in
the database.** `graph_context_composition_edges(p_profile, p_seed_ids uuid[], p_depth int)` takes an
arbitrary node-id array, returns exactly the `AtlasEdge` shape, and is visibility-scoped,
unfolded-only and `kb_resources` on both ends. At `p_depth = 0` its recursive arm is dead
(`r.depth < LEAST(0, 3)`), so it returns **precisely the induced subgraph** over the array.

Proved by execution rather than reading: on production, `graph_context_composition_edges(pete,
top130, 0)` returns **275** edges, matching the independently computed K=130 induced count. At
depth 1 it returns 2672.

**This is the fourth time a `/api/graph/*` affordance was asserted missing and was not** — after the
three in §3 and `graph_traverse` in §5.2. The pattern is now established well enough to be a rule:
*before specifying a new graph read, print the live function list.*

`[ruled — 2026-08-21, Pete]` The body moves to **`graph_induced_edges`** — a frame-neutral name, since
it was never context-specific and chunk E deletes the context door that is currently its only caller.
The old name survives as a **delegating wrapper**, which is what keeps the migration `additive`
rather than a shape-breaking rename, and is deleted with the endpoint in E. A test pins the two
across every depth — one body, two names, the same precedent §5.2 names for B.

`graph_region_composition_edges` is the region-parameterised member of the same family and is
untouched.

### 5.3 `[found — 2026-08-20]` There are two degrees, and they must not share a name

The incumbent degree is **corpus-scoped**:

```sql
-- graph_region_composition.sql:77-83
SELECT count(*)::int AS degree FROM kb_edges e
  JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
 WHERE e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
   AND (e.source_id = r.id OR e.target_id = r.id)
```

Nothing requires the *other* endpoint to be in the set being drawn. So `AtlasNode.degree` answers
*"how connected is this in your corpus"*, while §5.1 constraint 2 requires *"how connected is this
in what you are looking at"*. **Both are legitimate; they are different quantities.**

This matters because it is **the same confusion this whole spec is about, a third time**: a quantity
measured over one set and displayed over another. A node can carry `degree: 12` and show zero edges,
and a reader has no way to reconcile that but to doubt themselves.

Today the successor does not hit this — `model.ts` recomputes degree client-side over the drawn edge
set. **The moment A and B feed `AtlasNode` through, two degrees coexist under one name.**

**`[ruled — 2026-08-20, Pete]` Only one degree ever reaches the screen, and it is the derived one.**

The corpus degree is a **ranking input for A** and does not need to be sent to the client at all.
What the reader sees stays what they see today: a count derived from the edges actually returned,
which `model.ts` already computes. **This kills the hazard at its source rather than managing it
with two field names** — there is never a second quantity on screen to disagree with the first.

The corpus figure still appears in **A's distribution report** (§5.1), which is a build-time
measurement feeding the choice of K, not a rendered number.

## 6. The fallback ladder

`[ruled — 2026-08-20, Pete]` — *"a kind of lower-density-down-to-empty-state fallback"*. Three rungs.
Fewer than the five an earlier draft carried, because degree degrades on what the corpus *contains*
rather than on what the reader was supposed to have done.

1. **Most-connected, then traversable.** The entry read seeds the canvas.
2. **Too little structure to be a graph** — no edges at all, or below a threshold. The
   document-corpus case from §2.3. `[ruled — 2026-08-20, Pete]` **The surface says so and sends the
   reader somewhere better**: it names what cannot be rendered as a graph and points at the vault's
   list view, which is the right instrument for a corpus with no relationships.
3. **Nothing readable** — the existing refusal, unchanged.

**Rung 2 is a declaration, not a degraded drawing.** An earlier draft had it fall back to the
recency page — draw 200 dots and hope. That was rejected: dots the reader cannot use are not more
honest than a sentence saying the graph is the wrong instrument here, and the sentence is the one
that respects `the-unstructured-reader-is-never-worse-off`. Their need is greater, so they get a
working door rather than an empty canvas.

**The threshold is a number nobody has yet.** It comes from the same distribution A must report
(§10.1), and until then it is a parameter rather than a constant.

**The rung must be visible.** A reader on rung 2 is looking at a different claim than one on rung 1.
Swapping silently is `legibility-is-never-bought-with-silent-omission`, and arguably
`surface-declares-its-kind`, since the screen would be changing what kind of answer it is without
saying so.

**Choosing the rung is the client's job.** The entry read returns its rows with their degrees,
zeros included; a read must not make presentation decisions.

## 7. What the surface owes the reader after the handoff

**This section is the one that regresses covered clauses if it is skipped.** The bound line and the
readout are covered today *because the composition trace hands them their numbers for free*. The
moment navigation takes over, that source is gone.

### 7.1 The bound line

It must not keep displaying the grounding query's counts — on hop three those describe a screen the
reader is no longer looking at. It must not disappear either: it is deliberately **chrome, not a
warning**, present whether or not the view is partial, *"so complete is something the reader is TOLD
rather than something they infer from silence."*

So **the navigation reads carry their own bound declaration** — how many of how many, and whether
more exist — and `renderBoundLine` gains a shape for a traversed view. It is pure and tested; this
is an addition to its vocabulary, not a rewrite.

### 7.2 *Why these*

Once the reader traverses away, the panel describes an answer they have left. Three options, and
silently persisting is the worst of them:

- **Update** — wrong; there is no readout for a traversal, and inventing one fabricates reasoning.
- **Disappear** — honest, and loses the reader's route back to how they got here.
- **Declare itself as the grounding** that the current view descends from — *recommended*. It stops
  claiming to explain the current screen and becomes provenance for it.

### 7.3 Traversal needs an address

`q` pushes and `sel` replaces, deliberately — *"a selection is ephemeral panel state and does not
belong in the history the Back button walks."* Real hops are not ephemeral. Without an address, Back
either leaves the site or silently discards the reader's path.

`AtlasEdge` carries an `id`, so both endpoints of a hop are addressable — which the composition
vocabulary could not offer. The grammar is an open design question (§10).

## 8. The guardrail — this must not rebuild the predecessor

**The predecessor was traversal-first, and it is the thing this goal exists to replace.** Moving
toward free traversal walks back toward it, and the distinction has to be written down rather than
remembered:

> The predecessor's **organizing structure was derived** — tiers, territories — and posed as
> navigation. Here the reader traverses **their own edges**. Derived structure organizes; it is
> never the thing being walked.

One concrete exposure: `AtlasNode.salience` is region-derived and rides along on every one of these
reads. **It must not drive any visual channel.** `degree` may — it is a count of the reader's own
edges, intrinsic to their material, and it is already documented as a sizing hint. Salience is not.
A mark sized or coloured by salience is `no-derived-thing-poses-as-authored`, which is the clause
that got the tier model deleted.

## 9. Clause impact

Assessed against the register as it stands after the `[2026-08-20]` amendment.

| Clause | Impact |
|---|---|
| `entry-does-not-presume-organization` | **Strengthened.** Degree presumes no doc types, no goals, no charter, no naming convention — it works on a document corpus and a workflow corpus alike. This is why §2.3 was rejected |
| `legible-at-the-sizes-the-corpus-actually-reaches` | **The target.** 244-of-250 is the case; the reader established that at that density *no* visual encoding can be read |
| `legibility-is-never-bought-with-silent-omission` | **At risk — §7.1 is what protects it.** Covered today via the composition trace; regresses the moment a navigation view ships without its own bound declaration |
| `no-derived-thing-poses-as-authored` | **At risk — §8 is what protects it.** `salience` arrives on every navigation read |
| `surface-declares-its-kind` | **Touched.** The rung must be visible (§6) and the grounding must not masquerade as an explanation of a traversed view (§7.2) |
| `cross-kind-relationship-is-reachable` | **Preserved.** `AtlasNode.home` spans both anchor kinds; the kept reads cover cogmaps and the new ones cover contexts |
| `the-unstructured-reader-is-never-worse-off` | **Improved, still plan-level only.** §2.3's rejection is precisely this actor. Nothing here converts the clause — it has still never met a server |

## 10. Open questions — rulings needed before build

1. ~~**K, and how it interacts with twelve anchors.**~~ **DIRECTION RULED, VALUE STILL OPEN**
   `[2026-08-20, Pete]` — *"agreed with the direction but also agreed that we will have to
   investigate to be sure."* Visibility-scoping dissolves the union-arity problem, but not how many
   marks a first screen carries. **K is a parameter in A, not a constant**; A must report the degree
   distribution, and K (with rung 2's threshold) is fixed in C from real numbers, following
   `ANCHOR_CEILING`'s precedent of measuring the heaviest real reader.

   The distribution may also change K's *kind*. ~~Degree-ordered selection means every node above the
   cut has at least one edge to another drawn node **by construction**, so the unconnected band does
   not exist until the connected material runs out.~~ That argues K may want to be a **threshold**
   (`degree ≥ n`, capped) rather than a count. **Deciding that without the measurement is exactly
   the error this spec was written to correct**, so it stays open by design.

   `[refuted by measurement — 2026-08-21]` **The struck sentence is false, and it is the reason the
   measurement was worth insisting on.** Degree ordering guarantees *nothing* about induced
   connectivity: a hub's neighbours are typically **leaves**, which is precisely why they are
   low-degree and miss the cut. Measured on production (3574 visible resources, 4385 edges) — see
   §10.1.1 — **at K=50, 18 of 50 nodes with corpus degree ≥ 16 have no edge inside the drawing.**

   It is the spec's own defect a sixth time: a quantity measured over one set (corpus degree) used
   to predict a property of another (induced connectivity).
### 10.1.1 `[measured — 2026-08-21]` The distribution, and what it licenses

Production, profile `j-cole-taylor`: **3574 visible resources**, **4385 visible edges**, all
resource↔resource. Degree via the incumbent `edges_visible_to`, which already excludes folded edges.

| degree | 0 | 1 | 2 | 3–4 | 5–9 | 10–19 | 20–49 | ≥50 |
|---|---|---|---|---|---|---|---|---|
| nodes | 1077 | 1063 | 439 | 446 | 360 | 161 | 24 | 4 |

`max 87 · mean 2.45 · 2497 at degree ≥ 1 · 549 at ≥ 5 · 189 at ≥ 10`

Rank by corpus degree, draw the top K, keep every edge with both endpoints in the set:

| K | cut degree | induced edges | connected | unconnected | % |
|---|---|---|---|---|---|
| 50 | ≥16 | 56 | 32 | 18 | **36.0%** |
| 75 | ≥13 | 121 | 58 | 17 | 22.7% |
| 100 | ≥12 | 193 | 75 | 25 | 25.0% |
| 130 | ≥11 | 275 | 104 | 26 | **20.0%** |
| 150 | ≥10 | 312 | 113 | 37 | 24.7% |
| 200 | ≥9 | 447 | 154 | 46 | 23.0% |
| 250 | ≥8 | 594 | 199 | 51 | 20.4% |
| 400 | ≥6 | 1029 | 341 | 59 | 14.8% |
| 549 | ≥5 | 1441 | 488 | 61 | 11.1% |

**Two readings C must carry.**

The band **gets worse as K shrinks** — 36% at K=50 against 11% at K=549. Every instinct that says
"draw fewer marks to get a cleaner graph" is backwards here, and the fallback ladder was drafted on
that instinct.

But **every K measured beats the state the reader accepted.** Today's unaddressed entry is
244/250 = **97.6%** unconnected; the *answered* state the reader did not complain about is
45/130 = **35%**. The design works at any of these; what remains is a density-vs-band trade, and it
is C's to rule against §2's finding that no visual encoding survives 250 marks.

2. ~~**The traversal URL grammar.**~~ **RULED** `[2026-08-20, Pete]`:

   ```
   /graph/@me?q=<grounding question>&from=<node-ids>&depth=<n>
   ```

   Mirrors `SliceRequest` (`seeds`, `depth`, `edge_kinds`), so the address says exactly what read
   produced the screen. Each hop **pushes**, so Back walks the reader's path instead of leaving the
   site — a change from `sel`'s deliberate `replaceState`, because a hop is not ephemeral panel
   state.

   **`q` survives the handoff as provenance**, and that is what makes §7.2 work: *Why these* stops
   claiming to explain the current screen and says what the view descends from, because the question
   is still in the address. *"It gives traversal a home."*

   **No edge id ever appears in a URL.** An edge is not a place; you navigate to nodes.
   `AtlasEdge.id` earns its keep in selection and trails, not in addressing a view.
3. ~~**Does `?q=` still redraw on every change, or ground once and hand off?**~~ **RULED — hand off**
   `[2026-08-20, Pete]`: *"asking a question and our query composition frame helps set the space, but
   then you traverse the graph as normal without a question locking you in."* Not open; recorded
   here so it is not re-argued. The consequence is that the answered state stops re-running a
   composition per interaction, which is most of the latency the reader ranked second-most-jarring.
4. ~~**Whether the recency page survives at all**~~ **RULED — it does not** `[2026-08-20, Pete]`:
   *"a corpus with no edges or under some reasonable threshold should probably just say so — say
   what is not really renderable and indicate that the vault list view is better than the graph one
   for this."* Rung 2 became a declaration plus a door, not a degraded drawing. See §6.

   **Consequence worth naming:** `readSeedRows` and the recency page have no remaining caller once C
   lands. They are deleted in C rather than kept as a fallback — the fallback is now a sentence.

## 11. What this spec does not do

- **It does not fix the four defects filed from the reader session.** They are independent and
  separately filed. `REACHED`'s false claim (`01a0215d-65d5-7373-92c4-c1559dd911d4`) overlaps this
  work — its arm vocabulary changes here — and should be sequenced with it rather than merged into
  it.
- **It does not close `surface-declares-its-kind`.** Nothing built closes a judged clause; only a
  reader does.
- **It does not address latency directly**, though §10.3 would remove most of it as a side effect.
  That axis is filed with no goal link (`01a0215d-7dc5-77e1-b055-76c52d3c013b`) and still has no
  clause anywhere.

## 12. Build order, and the true new-SQL surface

`[settled — 2026-08-20, Pete]` Grounding shrank this considerably. **Two new SQL functions, not a
new act framework.**

| New SQL | Serves |
|---|---|
| A degree ranking over the visible corpus → ids **+ the distribution** (§5.1) | A |
| ~~Induced edges in `AtlasEdge` shape over an arbitrary node-id array (§5.4)~~ | ~~**A and B both**~~ |

`[corrected — 2026-08-21]` **The true new-SQL surface is ONE function, not two.** The induced-edge
read already existed (§5.4); chunk A gave its body a frame-neutral name and left a delegating
wrapper behind. So the only genuinely new SQL in this whole spec is the degree ranking.

Everything else is a service function and a handler over fragments that already exist:
`graph_atlas_nodes_visible` hydrates nodes for both chunks, and the surviving undirected walk body
serves B.

| Chunk | | Depends on |
|---|---|---|
| **A** ✅ | Entry read — ranking fn, induced-edge fn, service, handler, tests. Rust only, nothing on screen. **Landed `[2026-08-21]`** as `GET /api/graph/entry`; distribution reported in §10.1.1 | — |
| **B** | Traversal read — service + handler over the surviving walk and the induced-edge fn. Rust only | A (shares §5.4) |
| **C** | Unaddressed entry switches to A; `readSeedRows` and the recency page **deleted**; rung ladder and its declaration; **K and the threshold fixed from A's distribution** | A |
| **D** | The handoff — ground once, then navigate. Bound-line vocabulary, *Why these* becomes provenance, the §10.2 URL grammar | A, B, C |
| **E** | Delete `contexts/panorama`, `contexts/composition`, **and the dead directed chain** (`graph_traverse`, `graph_subgraph_nodes`) | C |

**A and B are no longer fully independent** — B consumes §5.4, which A introduces. A first, then B;
or §5.4 lands as its own step ahead of both.

**C cannot be planned in detail until A has run**, because K and rung 2's threshold come out of A's
distribution report. That is deliberate: choosing them earlier is the exact error — asserting a
number instead of measuring it — that this spec exists to correct.
