# Context Regions and Wayfinding — Design

**Date:** 2026-07-11
**Status:** Design approved, not yet implemented
**Scope:** Extend the region / salience / coherence layer and the wayfind scope resolver from cognitive maps to contexts.

---

## 1. The problem

Today the region layer is exclusively a cognitive-map facility. `kb_cogmap_regions`,
`kb_cogmap_components`, `kb_cogmap_lenses`, and `kb_cogmap_region_members` are all keyed on
`cogmap_id NOT NULL`. `wayfind_scope_ids` pools over `cogmap_visible_maps(principal)` and nothing
else. The `WAYFIND_UNREACHABLE` hint in `substrate_read.rs` states the consequence plainly to
agents: *"wayfind only reaches cogmap-distilled content — if what you want is context-homed (in no
cogmap), it is unreachable here regardless of phrasing."*

Contexts get a real search stack — FTS over a stored tsvector, pgvector, and graph traversal
(`unified_search`, `search_graph_expand`) — but no region, salience, coherence, telos-alignment, or
centrality signal at all. There is no clustering over a context, no centroid, no `α·salience` prior,
and no orientation survey. `classify_scope` knows four scopes (`Global | Context | Cogmap |
Wayfind`) and treats `context_ref` and `wayfind` as **mutually exclusive** — passing both is a
`BadRequest`.

This is backwards relative to where the work actually lives.

### 1.1 Ground truth (prod, 2026-07-11)

| | cogmaps | contexts |
|---|---|---|
| resources homed | 305 | **1,643** |
| facet property rows | 159 (on 152 resources) | **0** |
| non-folded edges | ~30, richly typed | ~900, but 860 are two kinds |

The two dominant context edge kinds are `near`/`relates_to` (451) and `leads_to`/`advances` (409).
`@me/temper` alone holds 1,012 resources and 547 edges. Doc-type mix across all contexts: 786 task,
489 session, 211 research, 80 goal, 68 concept, 9 decision.

So five-sixths of the corpus — and effectively all of the *work* — is unreachable from the
orientation and wayfinding surfaces the system was built to provide.

---

## 2. The load-bearing premise

> **In a cognitive map, the declared graph is primary and the embedding is a second-order readout.
> In a context, the embedding is primary and the declared graph is weak supervision.**

This is a regime difference, not a tuning difference, and it is *caused* by the shape of the work.

A cogmap is **curated**: a steward distills nodes and asserts facets and typed edges as its job. Its
declared graph *is* the meaning, which is why `affinity()` (`crates/temper-substrate/src/affinity.rs:113`)
computes region formation purely from edge weights and facet overlap, with embeddings entering only
afterward as readouts (centroid, cohesion, telos-alignment).

A context is **accreted**: goal → task → session, bookended by research and decisions. That is a
lifecycle, not an act of curation. Faceting and edge-assertion are things a human or agent does when
it has spare attention, so they are sparse and unreliable midstream. The prod numbers above are the
proof — zero facets, in 1,643 resources, across a year of use.

Therefore, in a context, cosine similarity is not a proxy for regionality. **It is the only primary
signal of regionality there is.**

The consequence for the implementation is that generalizing `cogmap_id` to a polymorphic anchor is
*necessary but useless on its own*. With zero facets and a `relates_to`/`advances`-only graph,
pairwise affinity is ~0 almost everywhere, nothing merges below `resolution = 0.5`, and every region
comes out a singleton. **The substantive work is a second affinity kernel, not a wider key.**

---

## 3. Design

### 3.1 One producer, two regimes — the lens is the switch

`materialize` remains a single code path. The `Lens` row (`kb_cogmap_lenses`, mirrored by
`affinity.rs:58 struct Lens`) is what encodes the regime.

The affinity kernel gains one term:

```
affinity(a,b) =  Σ_edges  w_kind · weight        (declared: sparse, weak supervision)
              +  w_prop · facet_overlap(a,b)     (declared: sparse, weak supervision)
              +  w_cos  · knn_sim(a,b)           (inferred: sparse by construction)   ← NEW
```

`w_cos = 0.0` reproduces today's cogmap behavior bit-for-bit. Existing scenario fixtures and region
membership fingerprints must stay green under the new kernel; that is the regression floor for the
entire arc.

**`knn_sim` is a sparse exact-kNN graph, not a raw cosine.** This is not optional. Cosine is *dense*
— every pair of resources has a nonzero similarity — so dropping a raw cosine term into `affinity()`
makes the affinity graph complete. `connected_components` would return one giant blob, the pre-pass
that makes agglomerative clustering tractable would stop doing anything, and the cost would be
Θ(n²) on 1,012 nodes and growing. Instead:

```
knn_sim(a,b) = cos(a,b)  if b ∈ topK(a, knn_k) and cos(a,b) ≥ cos_floor
                         (or symmetrically a ∈ topK(b, …))
             = 0         otherwise
```

Computed **exactly**, not via HNSW — the same reasoning as the #358 scoped-search fix (a scoped
corpus is small enough for an exact scan) and, critically, because an approximate index would break
the determinism that `membership_fingerprint` depends on. Per-resource pooled embeddings are the
same `avg(chunk.embedding) WHERE is_current AND NOT is_folded` pooling that `populate_readouts`
already uses for centroids (`write.rs:516`).

New lens columns: `w_cos`, `knn_k`, `cos_floor`.

### 3.2 Lens weights express meaning-when-present, not frequency

A lens weight is a **rate of exchange** — what a signal is worth when it appears — *not* a prior on
how often it appears. Sparsity is already self-handling: a pair with no `express` edge contributes
zero from that term regardless of `w_express`. Discounting a weight to reflect rarity double-counts
the rarity and accomplishes nothing else.

Once those are separated, the calibration inverts. An `express` edge asserted inside a *context* is
**more** evidential than one inside a cogmap, not less. A steward asserting edges into a map is doing
its job. A human or agent who breaks flow mid-session to say "this concept expresses that one" is
spending attention it did not have to spend — the rarity is exactly what makes it informative.

There is also a feedback-loop argument, and it is decisive on its own:

> **A weight of 0.0 makes the discipline provably unrewarded, and an information system that returns
> no signal for signal provided gets routed around. Humans, agents, and information flows self-adapt
> to functional gaps without anyone deciding to.**

If asserting a facet in a context visibly tightens a region, the skill-discipline pays for itself and
you get more of it. That is the *only* mechanism by which contexts ever become better-structured.
Zeroing those terms forecloses it permanently.

So the axis is not cogmap-vs-context. It is **deliberate vs. cheap**:

| lens term | signal | cogmap (`telos-default`) | context (`workflow-default`) | rationale |
|---|---|---|---|---|
| `w_express` / `w_contains` | deliberate assertion, rare | 1.0 / 1.0 | **1.0 / 1.0** | Means the same thing wherever it appears. Rarity ≠ discount. |
| `w_prop` (facet overlap) | deliberate assertion, rare | 0.4 | **0.4** | Same. Zero facets today is a fact about the corpus, not the kernel. |
| `w_leads_to` | `advances` — auto-projected from `--goal` | 0.6 | **0.9** | Cheap to create, but goal-membership is genuinely structural. This is the hub topology (§3.3). |
| `w_near` | `relates_to` — casually asserted, 451 of them | 0.3 | **0.35** | Cheapest signal, most abundant. Real but weak. |
| `w_cos` | inferred | **0.0** | **1.0** | The regime switch. |
| `knn_k` / `cos_floor` | — | (unused) | 12 / 0.55 | Sparsifies the dense cosine. |
| `s_telos` / `s_ref` / `s_central` | salience blend | 0.5 / 0.3 / 0.2 | 0.6 / 0.15 / 0.25 | Contexts have weaker provenance depth; lean on telos. |

The two lenses now differ in exactly one *conceptual* place — whether inferred similarity counts —
plus a calibration of the two cheap edge kinds. **Everything deliberate is weighted identically in
both.**

This makes the lens a **readout of corpus maturity**. As skill-discipline improves and contexts
accumulate real facets and typed edges, a context lens can be tuned toward the cogmap lens; in the
limit the regimes converge and `w_cos` comes down. That convergence becomes measurable rather than
assumed. Nothing in the producer is hardcoded to "contexts have no facets" — that is merely what the
data says today.

### 3.3 Goals are already the hub topology

`advances` edges (409) run task → goal. Two tasks advancing the same goal have **no direct edge**,
and `affinity()` is strictly pairwise-direct, so it cannot see them as related.

But the goal is itself a context-homed resource — a *node*. Average-link agglomeration
(`cluster.rs:139`) will chain `task₁ — G — task₂` through the goal automatically. Goals therefore do
double duty: they are the **telos** (§3.4) *and* they are the structural hubs that give an otherwise
near-topology-free declared graph its only real shape. `w_leads_to = 0.9` in the context lens is
tuned to lean on this deliberately.

### 3.4 A context's telos is its goals — weighted by the task census beneath them

A cogmap orients by its charter: `cogmap_region_telos_alignment` (`canonical_functions.sql:461`)
cosines a region centroid against `kb_cogmaps.telos_resource_id`'s pooled chunk embeddings. A
context has no charter and should not be made to author one — its purpose is legible from the goals
it already holds.

**The naive version does not work.** Defining the telos as "the centroid of goals whose
`temper-status` says `active`" produces a centroid of ~everything and a uniformly high, completely
uninformative `telos_alignment`. Prod, `@me/temper` (80 goals repo-wide, 30 in this context):

| goal | declared | in-prog | backlog | done | cancelled | truth |
|---|---|---|---|---|---|---|
| Context regions and wayfinding | active | 1 | 4 | 5 | 0 | live ✓ |
| Substrate kernel → cognitive map | active | 1 | 0 | 34 | 2 | live ✓ |
| Graph Atlas | active | 1 | 0 | 0 | 0 | live ✓ (brand new) |
| Maintenance | active | 0 | 3 | 71 | 13 | faintly live ✓ (a container) |
| **Temper Cloud** | **active** | **0** | **0** | 19 | 6 | **dead — declaration is wrong** |
| **path-to-alpha** | **active** | **0** | **0** | 17 | 1 | **dead — declaration is wrong** |
| **Teams in Temper** | **completed** | 0 | **1** | 0 | 0 | **live — declaration is wrong** |

Six goals declared `active` have zero open tasks and are unambiguously finished. Four declared
`completed` still carry an open task. The declared field is stale **in both directions**.

> **The census above is a snapshot (re-read from prod 2026-07-12 during T5), and that is the point.**
> The first draft of this table cited `temper-rb` as the headline "declared `completed`, one task in
> progress" case. By the time T5 implemented it, `temper-rb` had been flipped back to `active` and had
> *two* tasks in progress — while four other goals had drifted into the `completed`-with-open-work
> state. The specific rows rot; the phenomenon does not. Anything downstream of this table must treat
> it as a **frozen fixture**, never a live query — prod moved twice inside a single working session.

Nor does recency rescue it. "Days since the last touch on any advancing task" fails because
**marking a task done *touches* it** — a burst of recent `updated` timestamps under a closing goal
is the sound of the goal *finishing*, not the sound of it being alive. That signal measures closure
and reads it as liveness.

**Why the goal layer is unreliable, structurally.** A goal's boundary is a modeling choice. Program,
initiative, milestone, epic, project, triage-event — all the same "enveloping descriptive layer over
ever-more-discrete units of work," all called a goal, spanning multi-year efforts and two-day
cleanups alike. There is genuinely no formula for when to close one, which is why backlog grooming is
universally miserable in every PM tool ever built. **A task, by contrast, is the unit of work
precisely because it terminates.** It has a demonstrable stop-point, and people and agents reliably
open, work, and close them.

So read liveness off the layer that reliably terminates, and let the layer that does not inherit it:

```
liveness(g) = status_damper(g) × sqrt( Σ  stage_weight(stage(t)) · exp(−idle_days(t)/halflife) )
                                     t advances g

  stage_weight    in-progress 1.0 · backlog 0.35 · done 0.0 · cancelled 0.0
  idle_days(t)    days since t was last touched
  halflife        lens column, ~30d initial
  status_damper   active/absent 1.0 · paused 0.3 · completed 0.4      ← damps, never gates

telos_embedding(ctx) = Σ liveness(g) · emb(g) / Σ liveness(g)   over goals homed in ctx
```

> **`done` is weighted 0.0, and an earlier draft of this spec got that wrong.** It specified 0.15,
> reasoning that a small weight plus decay would keep a graveyard from masquerading as a heartbeat.
> Run against the census above — the fixture this spec nominates for exactly this purpose — 0.15
> produces very nearly the *inverse* of the ranking demanded below: `Maintenance` ranks **1st of 32**,
> while `Temper Cloud` and `path-to-alpha` (which must fall out entirely) outrank `Graph Atlas` (which
> must rank at the top).
>
> The mechanism is arithmetic, and no decay rate fixes it. A weight of 0.15 is small; seventy-one of
> them is not. And because *marking a task done touches it*, `exp(−idle/halflife)` is ≈1.0 for
> precisely the tasks that just closed — so a goal that is **finishing** looks maximally alive. A
> shorter halflife does not help, because `Maintenance` closes tasks *continuously*: no decay rate
> distinguishes a steady drip of completed chores from live work. **The count is the problem, not the
> age.** This was measured, not argued: `context_telos_liveness.rs` pins it as a differential test, so
> restoring 0.15 fails loudly.
>
> This is not a retreat from the argument below — it is the argument below, which the number
> contradicted: *old completed work is history, not purpose*. Closing a task is still rewarded, by
> removing it from the open set. The lens column survives and stays tunable; only its calibration
> changes.
>
> **The `grace` floor is dropped.** An earlier draft floored a goal created <14d ago with no tasks at
> 0.5. The cold-start it protects lasts minutes in practice (tasks are created with `--goal` moments
> after the goal), while a goal sitting two weeks with zero work is *precisely* the aspirational-not-
> real case this section exists to exclude. It bought a knob and cost the principle.

Three properties follow, none of which require anyone to groom anything:

1. **Only OPEN work counts, so a graveyard cannot masquerade as a heartbeat.** Maintenance's 71
   finished tasks contribute exactly nothing; its 3 fresh backlog items are the whole of what keeps it
   faintly warm — a container, not a driver. Temper Cloud's 19 finished tasks likewise contribute
   nothing, and it drops out of the telos entirely despite saying `active`. Old completed work is
   *history*, not *purpose*.

2. **Goal scale infers itself.** A long-lived program with continuous task flow stays in the telos
   for years; a two-day triage goal spikes and decays out within a week. Nobody declares which kind
   it is — the system reads a goal's scale off the work beneath it, so the
   "is-this-an-epic-or-a-milestone" question never has to be asked. `sqrt` compresses the 84-task
   container against the 4-task new goal so size tilts without swamping.

3. **A goal is as real as the work beneath it.** Zero live tasks means zero telos contribution,
   whatever the status field claims. This is the pragmatic answer to "is this aspirational or a hard
   target?" — we never ask; we look.

The declared status survives only as a **damper**. It cannot resurrect a goal with no work, and it
cannot kill one with a task in progress. But marking a goal `completed` *does* immediately damp it to
0.4, so the hygiene pays off the instant someone does it — the same contract as §3.2: rewarded,
never required.

A context with zero live goals degrades gracefully: `telos_alignment` is NULL, `coalesce(…, 0)`
applies, and salience falls back to reference-standing plus centrality.

`anchor_telos_embedding(anchor_table, anchor_id, lens)` is one function with two branches — the
charter's pooled chunks for `kb_cogmaps`, the liveness-weighted goal centroid for `kb_contexts`. (It
takes the **lens**, which an earlier draft's signature omitted: the constants below are lens-resident,
so a function without the lens cannot read them.) The region-level readout
`anchor_region_telos_alignment(region, anchor_table, anchor_id, lens)` dispatches on the same pair and
delegates the cogmap branch to the untouched `cogmap_region_telos_alignment`, which keeps the cogmap
regime byte-identical — the regression floor of §5.

**Constants are lens-resident and calibrated, not guessed.** `halflife`, the stage weights, and the
dampers live on the lens row beside `knn_k` / `cos_floor` / `resolution`, tunable by additive
migration (consistent with the α/β wayfind constants staying SQL-resident). The table above is the
calibration fixture — see §5.

> Lens-resident is not the same as lens-*reachable*. T2 added these columns; `_project_lens_created`
> was never widened to project them, so until T5 a lens minted through the ledger silently took the
> column defaults and the payload's values were dropped on the floor — tunable in the DDL, untunable
> in practice. This was the **third** group of T2 columns to be missed by that same function (T4 fixed
> the first two and stated in its own migration that there were only two). A column that exists is not
> a column that is wired.

### 3.5 Two clocks: formation and salience refresh independently

Formation is expensive and depends on membership inputs (resources, edges, facets, embeddings).
Salience is a handful of cosines and depends on the telos. In a cogmap these move together, so
`populate_readouts` running only inside `materialize` is fine.

**In a context they come apart.** Goals close and open without any region's membership changing — the
shape is identical, but what matters has moved. Gating the cheap thing behind the expensive thing
would mean a goal closing has no effect until ~20 unrelated writes happen to trip the formation
threshold.

So `materialize_on_threshold` grows a second, cheaper gate, and both live in one call:

```
on write to anchor A:
  1. telos drift    d = 1 − cos(telos_now(A), A.telos_centroid)          -- one cosine
                    if d > ε:  refresh_salience(A, lens)                 -- cheap: no clustering
                               A.telos_centroid := telos_now(A)

  2. formation      n = formation_touched_count_since(A, watermark)      -- existing count(*)
                    if n ≥ threshold:  incremental_materialize(A, lens)  -- expensive
                                       (implies a salience refresh)
```

`kb_contexts.telos_centroid vector(768)` is the snapshot that makes gate 1 possible. It also makes
**telos drift a first-class queryable signal** rather than something merely tolerated:
`anchor_telos_drift(anchor)` reports how far a context's purpose has moved since its shape was last
computed — the context analogue of `cogmap_staleness`.

The trigger stays event-count-threshold-on-write, generalized. No cron, no agent. Events are
*already* anchored to contexts (`steward_ingest_delta` counts them today), so
`formation_touched_count_since` needs only its `producing_anchor_table = 'kb_cogmaps'` filter
widened.

### 3.6 Schema — expand, migrate, contract (contract deferred)

Three phases. `main` stays auto-deployable throughout, per the additive-only-on-`main` invariant.

**M1 — additive.** Safe to auto-deploy.
- The four region tables gain `home_anchor_table VARCHAR(64) CHECK IN ('kb_contexts','kb_cogmaps')`
  and `home_anchor_id UUID`, backfilled from `cogmap_id`. `cogmap_id` stays and is dual-written.
- `kb_cogmap_region_members.member_table` CHECK gains `'kb_contexts'`.
- `kb_contexts` gains `shape_materialized_event_id UUID` and `telos_centroid vector(768)`.
- `kb_cogmap_lenses` gains `w_cos`, `knn_k`, `cos_floor`, `kappa_anchor_prior`, `telos_halflife_days`,
  and the stage-weight / damper columns. **Existing rows default to `w_cos = 0.0`**, preserving
  current cogmap behavior exactly.
- `COMMENT ON TABLE / COLUMN` records that the `kb_cogmap_*` names are transitional and that
  `cogmap_id` is vestigial, since the names will lie until M3.

**M2 — code.** Producer, readbacks, and wayfind read and write only the anchor pair. `cogmap_id`
remains populated but unread.

**M3 — contract. Operator-run, deferred indefinitely.** `DROP cogmap_id`; rename
`kb_cogmap_{lenses,components,regions,region_members}` → `kb_{lenses,components,regions,region_members}`.
Decoupled from the feature; it lands once the functionality has soaked in prod. Naming follows
confidence, not the other way round.

The known cost: between M1 and M3 the table names are wrong — `kb_cogmap_regions` will hold context
regions. Accepted, and carried by the column/table comments.

### 3.7 Reads

**Wayfind goes anchor-agnostic.** `visible_region_anchors(principal)` replaces
`cogmap_visible_maps(principal)`, returning `(anchor_table, anchor_id)` over both kinds. The k-CTE in
`wayfind_scope_ids` (`20260629000007_wayfind_scope.sql:35`) gains an anchor-kind prior:

```
region_score = α · sal_norm  +  β · query_cos  +  κ · anchor_prior
                (0.4)            (0.6)              (NEW)

  anchor_prior:  kb_cogmaps 1.0  ·  kb_contexts 0.6     (lens-resident, tunable)
```

Unscoped `wayfind` pools regions from every visible anchor. The prior is what keeps the 5:1
raw-to-distilled ratio from drowning the distilled signal — a tunable tilt rather than a structural
exclusion. Scoped `wayfind --context X` / `--cogmap Y` restricts to that anchor's regions.

This makes the **composition read** free: a single wayfind can surface a distilled idea *and* the raw
work it came from, which is what `graph_region_composition_edges` reaches for today via a separate
traversal.

Two things must be deleted because they stop being true:
- the `context_ref` × `wayfind` mutual-exclusion `BadRequest` in `resolve_search_scope`
  (`substrate_read.rs`) — passing both now means *"wayfind within this context"*;
- the `WAYFIND_UNREACHABLE` hint string, which tells agents context-homed content is unreachable via
  wayfind.

**Orientation gets a context surface**: `context_shape`, `context_region_metrics`, and
`graph_context_territories`, mirroring the cogmap trio, exposed on MCP and CLI. This is the
"region-level view of everything in a context" this arc exists to deliver. The existing context graph
reads (`20260709000010_graph_context_reads.sql`) supply only containment and residual counts — no
salience.

### 3.8 Authz prerequisites

> **Rewritten 2026-07-11 against the live schema, mid-execution.** This section originally claimed the
> context arm of `anchor_readable_by_profile` "ignores `kb_access_grants` entirely," citing the header
> of `20260630000002_access_grants_read_wiring.sql`. **That gap was already closed** — on 2026-07-01,
> by `20260701000004_anchor_readable_context_grant.sql`, the migration that quotes it. The sweep read
> the stale header rather than the live function. Verifying the claim turned up two *different* and
> real defects, and forced the access model to be written down for the first time. Landed in
> `20260712000010_context_read_predicates.sql`.

#### The model (it was nowhere stated, which is how it rotted)

The team DAG is an org **enclosure hierarchy** — `EPD ▸ engineering ▸ payroll-group ▸ squad-two` —
plus cross-cutting affinity groups on the same mechanism. Membership is **transitive upward**: a
direct member of `squad-two` is thereby a member of every enclosing team. Two axes follow, and they
are **not** the same axis:

- **READ inherits UP the enclosure chain.** A squad-two member reads what is at or above them —
  engineering's contexts, EPD's contexts — and *never sideways*: `squad-one` and `security-it-ops`
  are invisible. This is what `team_ancestors` expresses. It expands upward *from the principal's own
  team*, so a thing attached to an ancestor reaches every member beneath it.
- **WRITE requires DIRECT membership** in the owning team, with an authoring role (`owner` /
  `maintainer` / `member`; `watcher` is read-only). Being transitively in `engineering` lets you read
  engineering's context; it does not let you author into it.

Team-management RBAC (an owner of an enclosing team creating and managing sub-teams) is a **third**
axis and confers nothing on contexts or resources.

#### Defect 1 — read was too NARROW: the team-owned arm was flat, in five places

The context-read rule was written out **five times** — `context_visible_to`, `resources_visible_to`
(branch 5), `edges_visible_to`, `graph_home_contexts`, `resources_in_team_scope` — and every copy
gated the team-**owned** arm on *direct* membership. So a squad-two member could read a context
*shared to* engineering but not the context engineering *owns*. Owning was somehow more private than
sharing.

The copies had already begun drifting from one another, which is the real lesson. `graph_home_contexts`
had gone flat on the **share** arm too, and its `candidates` CTE is documented as *"a proven superset
(same branches)"* of `context_visible_to` — a claim that held only while both were equally wrong.
Widening the predicate alone would have silently turned it into a **subset** and dropped contexts out
of the graph view.

So the fix is not to widen five copies. It is to create **one** — `contexts_readable_by(p_profile)`,
the single context read-set — and route all five through it, with `context_readable_by_profile` as its
boolean grain (the peer of `cogmap_readable_by_profile`). There is nothing left to drift.

#### Defect 2 — write was too WIDE: mutation inherited up, and role gated nothing

`context_authorable_by_profile`'s team-owned arm **ancestor-expanded**. Combined with defect 1 that
produced a **write-wider-than-read inversion on the same object**: a squad-two member could *author
into* engineering's context while being unable to *read* it. And no access predicate anywhere
consulted `kb_team_members.role` — **0 of 15** — so a `watcher` could author.

The write arm is therefore **narrowed** to direct membership in the owning team with an authoring
role. This *revokes* write that exists today: the one non-additive change in the arc, taken
deliberately while the deployment is a handful of alpha testers, because it only gets more expensive
to fix later.

Explicit `kb_access_grants` **write grants are untouched** and still reach through `team_ancestors`. A
grant is a deliberate act of delegation, not an accident of enclosure — granting write to an umbrella
team is a considered decision to let everyone under it author.

#### The `'context'` principal kind

`resources_readable_by(p_principal_kind, p_principal_id)` supported only `'profile'` and `'cogmap'`. A
`'context'` kind is needed for the self-read pattern `cogmap_shape` / `cogmap_region_metrics` use.

Note its real shape: it is `LANGUAGE sql`, a `UNION` whose arms are guarded by
`WHERE p_principal_kind = …` — **not** a plpgsql `IF/ELSIF`. An unhandled kind therefore returns
**zero rows rather than raising**, so any test for the new kind must assert that a homed resource comes
*back*, not that the result is empty. (The originally-planned test asserted `count == 0` and would
have passed against the unmigrated schema.) The fail-closed behavior is pre-existing and left alone.

### 3.9 Bugs surfaced by this work

Both are pre-existing and both are squarely in this arc's narrative, so they bundle into its PRs
rather than being extracted.

1. **`kb_cogmap_region_members.affinity` is never written.** `write.rs:487` inserts only
   `(region_id, member_table, member_id)`. Four readers — `graph_region_members`,
   `graph_region_territories`, `graph_cogmap_territories`, `atlas_search` — all
   `ORDER BY m.affinity DESC NULLS LAST`. **Every "top member" and derived region label in the
   product today is therefore arbitrary.** The column needs a definition and a writer; the natural
   one, available from the clustering pass, is the member's average-link affinity to the rest of its
   component.
2. **`cogmap_region_centrality` (`canonical_functions.sql:488`) sums `kb_edges.weight` with no
   `home_anchor` filter**, so it already counts edges asserted outside the map. Under a polymorphic
   anchor this would silently mix context and cogmap edges into one region's centrality.
3. **`can_modify_resource` had no soft-delete WRITE floor** (found by the adversarial security review
   of the T1 write axis, and confirmed live). The read side floors on `is_active` everywhere; the
   write gate never did, so `can_modify_resource(author, tombstone)` returned true while
   `resources_visible_to` excluded it — read said deny, write said permit on the identical pair. And
   the write *committed*: `update_resource` (`db_backend.rs`) gates only on `check_can_modify_next`,
   and a **body-only or open_meta-only PATCH** skips the visibility-gated readback prefetch (it sits
   behind an `if managed_meta.is_some() || title.is_some()` guard), so the mutation landed on the
   tombstone. A leak (over-permissive write, I6-violating), not the false-negatives everything else in
   this arc turned out to be. Fixed additively in `20260712000020_can_modify_active_floor.sql` by
   wrapping the existing four-arm body in a leading `EXISTS(… is_active)` conjunct — one floor, every
   present and future arm inherits it. A NARROWING with negligible blast radius (it only ever denies
   writes to already-invisible rows that have no undelete path), bundled here because it is the same
   write-authz surface T1 reworks.

---

## 4. Non-goals

- **No steward for contexts.** The whole point of a context is real-time capture by humans and
  agents. Region production is a threshold-gated write-path side effect, not an agent's job.
- **No context charters.** A context's purpose is inferred from its goals (§3.4). Authoring a charter
  per context is a burden that would go unpaid, exactly like faceting.
- **No ANN in formation.** Exact kNN only, to preserve fingerprint determinism.
- **M3 is not in scope.** The rename waits for confidence.
- **No UI.** Data → API → CLI/MCP. UI last, once the shape has stabilized.

---

## 5. Testing

- **Regression floor.** Every existing scenario fixture (`crates/temper-substrate/tests/fixtures/`)
  must produce identical region membership and identical fingerprints under the new kernel with
  `w_cos = 0.0`. This is the single most important test in the arc: it proves the cogmap regime is
  byte-for-byte unchanged.
- **Scenario DSL generalization.** `BootSeed` is rooted at a `CogmapDef` with a mandatory `TelosDef`
  (`scenario/model.rs:82`), so a context-region scenario is currently inexpressible. The DSL needs a
  `ContextDef` peer, and `Step::Materialize` needs an anchor.
- **Liveness calibration is a labeled fixture, not an invented expectation.** The `@me/temper` goal
  census in §3.4 is the fixture. The test asserts the ranking, against real data: Temper Cloud and
  path-to-alpha fall out of the telos entirely; Substrate-kernel and Graph-Atlas rank at the top; a
  `completed`-declared goal with an open task is present but damped; Maintenance is faintly warm.
  Constants get fitted to that, rather than the fixture being fitted to the constants — which is how
  `sw_done` came out at 0.0 rather than the 0.15 this spec first guessed.
  **The fixture is FROZEN, not queried live** (`context_telos_liveness.rs`). It is still real,
  labeled prod data — but taken as a snapshot, because prod is not stable enough to assert against:
  during the single session that implemented T5, the census changed *twice*, and a live-query test
  would have failed for reasons unrelated to the code under test.
- **Determinism.** Same corpus + same lens → same `membership_fingerprint`, across repeated runs and
  a rebuilt vector index.
- **Two-clock separation.** Closing a goal must move salience *without* changing region membership or
  the membership fingerprint.
- **e2e at the production caller's level.** A context wayfind driven through `temper search`, not
  only through a direct `wayfind_scope_ids` call.

---

## 6. Work breakdown

PR-sized, ordered by dependency.

| # | Task | Depends on |
|---|---|---|
| 1 | **Authz prerequisites** — collapse the five copies of the context-read rule into one `contexts_readable_by` read-set (fixing the flat team-owned arm: read now inherits up the enclosure chain); **narrow** `context_authorable_by_profile` to direct membership + authoring role (closing the write-wider-than-read inversion); add the `'context'` principal kind to `resources_readable_by`. The `kb_access_grants` wiring this row originally called for was **already done** in `20260701000004` — see §3.8. | — |
| 2 | **M1 schema (additive)** — anchor pair on the four region tables + backfill; `kb_contexts.shape_materialized_event_id` + `telos_centroid`; the new lens columns defaulting to today's behavior; transitional `COMMENT`s. | — |
| 3 | **Anchor-generalize the producer** — `load()`, `materialize`, `incremental_materialize`, `fold_live_*`, `create_component`, `assert_region`, `formation_touched_count_since`, `region_materialize` + its event schema. Cogmap behavior byte-identical. Fixes bug §3.9.1 (persist `affinity`) and §3.9.2 (home-filter centrality) on the way. | 2 |
| 4 | **The `w_cos` kernel** — sparse exact-kNN affinity term, `knn_k` / `cos_floor`, the `workflow-default` context lens. Regression floor: all cogmap fixtures identical. | 3 |
| 5 | **Context telos + liveness** — `anchor_telos_embedding` with its two branches; the task-census liveness function; the calibration fixture against the `@me/temper` census. | 2 |
| 6 | **Two clocks** — decouple `refresh_salience` from formation; the telos-drift gate; `anchor_telos_drift`; generalize `materialize_on_threshold` to fire on context writes. | 4, 5 |
| 7 | **Anchor-agnostic wayfind** — `visible_region_anchors`; `κ · anchor_prior` in the k-CTE; delete the `context_ref` × `wayfind` mutual-exclusion and the `WAYFIND_UNREACHABLE` hint. | 6 |
| 8 | **Context orientation reads** — `context_shape`, `context_region_metrics`, `graph_context_territories`; MCP tools; CLI surface. | 6, 1 |
| 9 | **Scenario DSL** — `ContextDef` peer to `CogmapDef`; anchor on `Step::Materialize`; context-region scenario fixtures. | 4 |

**Decision doc (separate, and it outlives this arc):** *"A goal is as real as the work beneath it"* —
the liveness insight from §3.4, captured as a temper `decision` resource. It is a claim about how
work is legible in any knowledge system, not a detail of this implementation.

---

## 7. Open questions

- **Scale ceiling.** Exact pairwise kNN is comfortable at 1,012 nodes and fine at a few thousand. It
  is not fine at 50,000. When a context crosses that threshold the options are blocked/tiled exact
  computation or accepting an approximate index and giving up on fingerprint determinism. Not a
  problem today; worth a note in the code where the assumption lives.
- **`κ` initial value.** 0.6 for contexts is a starting guess. It is lens-resident, so it evolves by
  additive migration once there is a real corpus of context regions to look at.
- **Cross-anchor regions.** Nothing here lets a single region span a context *and* a cogmap. That
  seems right — the composition edge already crosses that boundary, and a region that straddles both
  would have no coherent telos. Recorded as deliberately out of scope.
