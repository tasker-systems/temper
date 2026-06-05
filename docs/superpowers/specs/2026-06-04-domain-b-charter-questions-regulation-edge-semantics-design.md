# The Cognitive-Map Domain Model (Spine #2): Telos-Charter, Questions-as-Blocks, Regulation-as-Edges, and the Edge-Kind Semantics

**Date:** 2026-06-04
**Status:** Design — developed in brainstorming 2026-06-04, pending plan. **Draft, ready for review.**
**Goal:** `substrate-kernel-to-cognitive-map`, Arc 1 (shared-kernel completion) — the **Domain-B successor**
the data-model spine deferred as "spine #2."
**Discharges:** the carve-out in
[`2026-06-01-data-model-reconciliation-design.md`](2026-06-01-data-model-reconciliation-design.md)
§ "Out of Scope" — *"Domain-B table design (telos-as-`kb_properties`-facet, questions-as-resources,
regulation-as-resource, `express`/`near` edge semantics) — spine #2, successor spec."*
**Builds on (does not re-spec):**
[`map-regions`](2026-06-02-map-regions-self-materialized-shape-surface-design.md) §0 (the telos **is a
resource**; `kb_cogmaps.telos_resource_id` FK; porosity dropped; map-as-telos-incubation-home),
[`content-block-primitive`](2026-06-03-content-block-primitive-design.md) (the addressable, versionable,
attributable block; the `block_*` event family; `{kind:block}` reference vocabulary),
[`access-capability-model`](2026-06-02-access-capability-model-design.md) (teams:RBAC, the polymorphic
**edge-home** gated by `edges_visible_to`, `resources_accessible_to_cogmap` = the DAG-expanded
least-privilege team-intersection),
[`map-to-map-delegation`](2026-06-02-map-to-map-delegation-dissolution-design.md) (`cogmaps_share_a_team`),
and [`data-model-reconciliation`](2026-06-01-data-model-reconciliation-design.md) (`kb_properties`,
polymorphic `kb_edges`, doctype demoted to a property).

> **Headline.** Domain B introduces **no new Domain-B kernel tables.** It is the **semantics-and-conventions
> layer** over the already-landed kernel: the **telos-charter resource** (whose interior is
> **questions-as-content-blocks**), **regulation-as-`express`-edged concept-resources**, the **four-way
> edge-kind carve**, the **`cogmap_genesis` seeding composition**, and an **entity-actored, event-driven
> persona model** (its agents emit as entities in the existing ledger, exactly like any external-system
> source). The single kernel touch it presumes
> (`kb_cogmaps.telos_resource_id`, porosity-drop) was **already landed by map-regions §0**. This spec
> determines *what the conventions mean* so the system is coherent in practice — it does **not** force a new
> shape onto the data layer.

---

## Context

The data-model spine drew the `temper-substrate` kernel beneath two domains and explicitly deferred
Domain-B table design to a successor. In the interval, three sibling specs landed pieces that turn out to
*be* most of the Domain-B substrate:

- **map-regions §0** established that a cognitive map's **telos is a resource** (`kb_cogmaps.telos_resource_id`
  FK, the seed-statement materialized as an ordinary `kb_resources` row that homes on its own map, carries
  chunks/embeddings/revisions/edges/properties, and **participates in the map's own shape as an embedded
  node**), and dropped `porosity` — the entity is a **telos-seeded incubation home**, not a permeable membrane.
- **content-block-primitive** gave every resource an addressable, discretely-mutable, attributable
  **interior** (`resource ⊃ blocks ⊃ chunks`), each block a correlation-keyed projected entity over a
  `block_created → block_mutated → block_folded` family, with accreting per-block provenance and a `{kind:block}`
  reference vocabulary that is **never a graph-edge target**.
- **access-capability-model / map-to-map-delegation** carried visibility entirely into **teams:RBAC** with
  **edges as RBAC-bound homed objects** (`edges_visible_to`), the producer-side
  `resources_accessible_to_cogmap` team-intersection, and the `cogmaps_share_a_team` bridge.

What remains — and what this spec owns — is the **Domain-B reading of the kernel**: how a telos, its
guiding questions, its learned regulation, and the cognitive-map edge-kinds are *expressed* over those
primitives without inventing a single new table.

### The organizing test: atomicity-as-self-completeness

The whole model is carved by one test, applied repeatedly:

> A **resource** is *discrete and self-complete* — atomic, semantically standalone, able to guide
> human-or-agent meaning on its own, while sitting in a graph of relationships (extend / supersede /
> modify) and carrying history-from-projection. A **content block** is *discrete but incomplete-to-itself* —
> addressable and lifecycle-bearing, but it does not mean on its own; it needs its resource to be whole.

This test — not "does it have a lifecycle?" (both do) — decides resource-vs-block throughout. It is the
substrate expression of the conceptual lineage's *concept-as-tool / structure-is-in-the-edges-not-the-node*
result (`2026-05-31-definitional-fallacy-concept-as-basin-telos-resolves-threshold-primitive`).

---

## 1. The telos-charter: the cogmap's constitutive seed resource

map-regions §0 already lands the spine of this: **`kb_cogmaps.telos_resource_id`** is a constitutive FK to
an ordinary `kb_resources` row — the map's seed-statement — that homes on its own map
(`anchor_table='kb_cogmaps'`) and participates in the graph like any concept. This spec **enriches the
resource's interior** and confirms the constitutive contract.

**The telos-resource is the cogmap's *charter*** (prose term; the column stays `telos_resource_id` —
*telos* foregrounds the constitutive, priming core). Under the content-block model "a resource is nothing
but its sequenced blocks," so the charter decomposes as:

- **block-0 — the telos statement:** the purpose-sentence + thin who-references (the seed note's
  *purpose-without-a-who-is-incoherent* result: who is structurally part of the purpose). This is the
  **priming frame**.
- **blocks 1..n — the guiding question-set:** the *questions-this-cogmap-exists-to-help-answer* (§2).
- **(later) framing-statement blocks:** other priming statements may accrete as the cogmap matures.

No `block_kind` is introduced (it stays YAGNI-deferred per content-block §"Scope boundary"). "block-0 is
the telos; blocks 1..n are questions" is a **positional convention** the acting agent never needs to consult
at storage — the agent always knows which block it is touching, and seq-0 recovers the telos by convention
if any read ever needs to distinguish them.

**Constitutive.** A cogmap is *born by authoring a telos* (seed note; telos/basin note §6). So
`telos_resource_id` is **`NOT NULL`** — a cogmap without a telos is not a cogmap, and the schema says so.
The genesis ordering that makes a `NOT NULL` FK work without a deferred constraint is §5.

**Still just a resource.** The charter earns its constitutive column for being singular and definitional,
but it remains a full `kb_resources` participant: other resources edge to it, it edges out to its regulation
(§3), it is findable, it carries its own provenance. The column is a **distinguished-pointer** (which of the
cogmap's homed resources is the charter), not a fence — it coexists with the charter's ordinary
`kb_resource_homes` row (`anchor='kb_cogmaps'`), which owns and access-gates it. **Architecturally the
charter is the cogmap's single columned hub; everything else — questions (its blocks), regulation (its
labeled edges), derived concepts — is reached by block-interiority or graph traversal from there.**

> **Revision of record.** The data-model spine's out-of-scope line named "telos-as-`kb_properties`-facet."
> That is **superseded**: telos is the **framing content of the telos-resource** (block-0), addressed by the
> `telos_resource_id` FK that map-regions §0 already landed — not a `kb_properties` row. (A denormalized
> purpose/who property on `kb_cogmaps` for hot-path triage was considered and **dropped** as redundant: the
> charter is one O(1) FK hop away.)

---

## 2. Questions-as-content-blocks

The seed note (`2026-06-01-seed-skill-scope-portable-vs-bound-awareness-access-bounded` §3) resolved the
**question-set as the irreducible load-bearing primitive of a seed** (persona is optional scaffold above it).
It modeled each question as a *resource* — but the **only** justification it gave was trajectory: "a question
accrues its own trajectory: reinforced, decayed, superseded-with-scar." The content-block primitive supplies
trajectory at a *finer* grain, which lets the **atomicity test** re-decide the question correctly.

**A question is incomplete-without-the-telos-priming-it → a content block, not a resource.** A seed
question — *"does this change a core-table schema assumption?"* — is inert on its own; it means something
only once the telos frames it. That is *precisely* the block→resource relationship: incomplete-without-its-containing-resource.
So **questions are blocks of the telos-resource**, and "incomplete without the telos priming them" becomes
**structural** — load the charter, you get the priming frame and its questions as one whole — rather than a
convention an agent must remember to assemble.

This is strictly sharper than questions-as-resources, on two counts:

1. **Blocks subsume the only thing that forced resource-hood.** A block is a correlation-keyed projected
   entity over `block_created → block_mutated → block_folded`, independently versionable
   (`kb_block_revisions`), foldable (`is_folded`), and attributable (accreting `kb_block_provenance`).
   Trajectory — covered.
2. **"Addressable ≠ findable" is exactly right for a question.** content-block is emphatic that blocks are
   addressable but **not** findable — "they cannot leak into traversal or search... `block` is a
   reference/provenance kind only, never a graph-edge target." A question is interior to a cogmap's
   self-knowledge; it is never a named thing other resources edge to, and never independently searched.
   Modeling each question as a resource would have made every question traversable/searchable and then
   demanded suppression guards. The role-play confirms questions never need edge-endpoint status: doc-3 and
   the sweeper both **route a question as an *event/observation*, never as an edge** ("emit a question-event…
   NO edges, never translation; the sweeper is just another event source"). A block can still be the
   *subject of a reference* (`{kind:block}`), which is all routing needs.

### Question lifecycle = block lifecycle

The five learning-acts map onto the generic block family with **zero new Domain-B event types** — the
ontology does the work, the mechanism stays minimal:

| question act | mechanism | notes |
|---|---|---|
| **reinforce** ("kept being right") | *derived* from the block's reference/incorporation stream — events that reference `{kind:block}` for this question | no native event; **derivable-not-denormalized** (blocks carry no weight column, and a block cannot own a `kb_properties` row — `kb_properties.owner_table ∈ ('kb_resources','kb_cogmaps')`). Reinforcement strength is read from event count/recency, not stored. |
| **decay** ("stopped mattering") | `block_folded` | content-block's fold is **"preserved, not wrong"** — exactly a decayed-but-not-mistaken question. |
| **supersede-with-scar** ("was the *wrong* question") | `block_folded` **+** the lesson written to **regulation** | the seed note's discipline: *write the lesson to regulation, not the domain.* The scar **becomes a regulation concept-resource** (§3) whose provenance references the folded question-block via `{kind:block}`. The wrongness lives where future cultivation will read it, not stamped on the retired question. |

Charter-level supersession (a whole charter replaced) is a **resource-level** `kb_edges` relationship
(`supersede`/`modify`), since the resource is the atomic unit that participates in the situated graph.

---

## 3. Regulation-as-`express`-edged concept-resources

A cogmap's **learned regulation** is its accumulated *way-of-growing-its-map-here* — "how to read relevance
in this cogmap," "how to elicit purpose here" (seed note §5–6). The atomicity test lands it **parallel but
not identical** to questions:

- **Each regulation lesson is self-complete** — a heuristic like *"status-report-shaped docs from adjacent
  teams match strongly on similarity but rarely move assumptions; weight assumption-movement over
  reference-density"* stands on its own and can guide. → a **resource** (a concept), not a block.
- **The set is open and growing** — "may not be just one concept or resource over time." That alone rules
  out blocks-of-one-host *and* a single columned resource.

So **regulation is an open set of ordinary cogmap-homed concept-resources, clustered to the telos-charter by
`express` edges** (§4; label `operationalized_by`). It has **no column, no distinguished pointer, no single-resource
assumption** — it is reached by traversing `express` edges out from the charter hub. This **dissolves the
seed note's flagged concern** (§158: *"cultivation_notes may be too coarse — regulation has ≥2 kinds"*):
kinds and lessons are concept-resources in a graph cluster, never a typology.

**Inheritance.** At genesis a child cogmap copies the parent's regulation as a warm start and asserts a
`near` / `regulation_inherited_from` edge (§4, §5) — cogmap→cogmap, recording *lineage*, not location. `near`
is chosen *because* it asserts no hierarchy, so the child overrides freely; the edge-kind choice **is** the
override-semantics. (Promotion of a lesson back into the **portable** seed-skill is a separate, guarded,
human-gated channel — out of scope, see below.)

---

## 4. The edge-kind semantics carve

The kernel `kb_edges` carries an `edge_kind`, `polarity` (`forward|inverse`), `label`, `weight`, and the
polymorphic edge-home — all RBAC-bound (`edges_visible_to`). Domain B reads these as: **`edge_kind` = the
structural class that carries the ownership/override semantics; `label` = the specific Domain-B
relationship.** A small fixed set of kinds × open-ended labels. *Structure is in the edges, not the node.*

| kind | structural meaning | Domain-B uses (label) | endpoints |
|---|---|---|---|
| **`express`** | abstract prior → its **local operationalization** | skill → cogmap (`cultivated_by`); telos → regulation (`operationalized_by`) | `(kb_resources → kb_cogmaps)`; `(kb_resources → kb_resources)` |
| **`near`** | **non-hierarchical** association, no ownership → **free override** | parent → child regulation inheritance (`regulation_inherited_from`); tentative cross-frame coupling (doc-3's low-weight "unconfirmed interaction") | `(kb_cogmaps → kb_cogmaps)`; `(kb_resources → kb_resources)` |
| **`contains`** | **hierarchical ownership** (the one with real has-a structure) | concept ⊃ subconcept | `(kb_resources → kb_resources)` |
| **`leads_to`** | **causal / sequential** dependency | concept → concept reasoning | `(kb_resources → kb_resources)` |

**`express` is one relation at two scales.** Source = the prior, target = its expresser: a **global** skill
expressed by a cogmap (`cultivated_by`), and a cogmap's **own** telos expressed by its regulation
(`operationalized_by`). Prior-to-expression, recursing one level inward — the primitive-economy the model
keeps rewarding: one edge-kind earning its keep at two altitudes rather than two bespoke relations.

**`near` carries override by construction.** It is the channel for relationships that must *not* confer
ownership: regulation inheritance (so the child can override), and the tentative cross-frame couplings the
triage role-play asserts at low weight when impact is unresolved-from-my-frame. Contrast `contains`, which
*does* confer ownership/hierarchy — which is exactly why inheritance is `near` and not `contains`.

No new shape: all four ride the kernel `kb_edges` projection unchanged. Cogmap-layer edges home in the
cogmap (`anchor_table='kb_cogmaps'`) and are gated by `edges_visible_to` — so asserting *any* relationship is
safe (§6).

---

## 5. `cogmap_genesis` — the seeding composition

A cogmap is born in one transaction (expressible as a single SQL function in the `schema.sql` artifact for
atomicity). It composes neutral kernel writes; by the carve-out test it is **Domain-B orchestration** (it
encodes the charter + questions-as-blocks + seed conventions), even though every primitive it calls is
neutral. Steps:

1. **Emit genesis events** — `scope_seeded`, plus a `block_created` per charter block (telos statement +
   each question), since `kb_content_blocks.genesis_event_id`/`last_event_id` FK into `kb_events` (events are
   the source of truth; the rows are the projection).
2. **Create the charter resource + its blocks** (telos-statement block-0, question blocks 1..n).
3. **Create the cogmap** with `telos_resource_id` → that resource (FK satisfied; `NOT NULL` holds).
4. **Create the charter's `kb_resource_homes` row** (`anchor_table='kb_cogmaps'`) — owns and access-gates it.
5. **If a parent exists** — copy the parent's regulation concept-resources as the child's warm start and
   assert the `near` / `regulation_inherited_from` edge (child starts warm, not bare).

**FK soundness — no deferred constraint needed.** The only hard FK between the two tables runs one way:
`kb_cogmaps.telos_resource_id → kb_resources(id)`. The resource has no hard FK back — its link to the cogmap
is the homes row, and `kb_resource_homes.anchor_id` is polymorphic (no FK). So **resource-first ordering
breaks the cycle outright**: create the resource (step 2) before the cogmap (step 3), and `NOT NULL` holds
with zero deferral.

> This **tightens** map-regions §0, which flagged the map↔telos mutual reference as "a creation-ordering
> detail (deferred FK / single creation event batch), noted for the plan." Resource-first ordering resolves
> it without a deferred FK. The crate-packaging of `cogmap_genesis` (one SQL function vs. a `temper-cogmap`
> orchestration over substrate commands) is a **spine-#3** sequencing concern; colocating it as a function in
> the artifact is fine for empirical evaluation now.

---

## 6. The personas are behavior; their actor is an entity

The cognitive map's agents — triage, steward, sweeper — are **telos-bearing personas**: *behavior*, not a
rich standing model (the design law — complexity in the fresh act of judgment, not the ossifying model). But
the **actor that emits their events is a distinct layer with a hard data requirement.** The event ledger
attributes every event to an entity:
`event_substrate.events.emitter_entity_id NOT NULL REFERENCES event_substrate.entities(id)` — *"every event
must attribute to a known entity."* So a launched agent-instance **is an entity** — a `uuidv7` row in the
entities table — exactly as external systems (GitHub, Linear, Notion webhook sources) are entities, agentic
or deterministic alike. **The persona is the behavior; the entity is the actor; the two must not be
collapsed** (an earlier draft of this spec did).

**The agent-instance entity is a runtime creation** carrying **launch-metadata**:

- **what it is** — e.g. *claude-sonnet-4.6 on Anthropic's managed-agent platform*;
- **its reach** — the cogmap it is bound into; it reads/writes under the access spec's **`Cogmap(M)`
  principal**, gated by `resources_accessible_to_cogmap(M)` (the DAG-expanded least-privilege team-intersection);
- **its persona / type-of-work** — triage, batching, gap-sweep;
- **its priming frame** — the telos it thinks-with: its own cogmap's telos, *or*, under `cogmaps_share_a_team`
  delegation, a **related-and-visible** cogmap's telos (access §292: an agent launched into `map-c` reads as
  `Cogmap(c)` — the priming frame may differ from the reach).

Capturing **both reach and priming-frame** on the entity is what makes **forward-provenance** answerable —
*which actor, reading from which frame, with which reach, asserted this edge* — the
tool-selection-is-entity-selection commitment (`forward-provenance`, Confident #9).

> **Entity-layer seam (a cross-spec dependency, not a Domain-B table).** The `entities` table already exists
> (event-substrate). Where the *richer* launch-metadata lives — the entity's legible `name` + the launch
> event's `metadata`, vs. an extension of the entity row — touches the event-substrate's **deliberately-deferred
> entity-typology** (`agent | human | integration`, flagged there as a known v1 flattening:
> `2026-05-18-event-substrate-foundations-design` §"deferred"). Domain-B agent-instances are the concrete
> driver that may un-defer (part of) that typology. **It is a kernel/event-substrate concern; Domain B adds no
> entity table of its own.**

- **Triage ("does this matter") loop.** Per inbound event: load the **charter** (O(1) via
  `telos_resource_id`) — its telos-frame (block-0) and question-blocks — plus the cogmap's **regulation**
  (traverse `express` edges) and instrument readings (pgvector, tsvector, `kb_properties` salience self-join,
  homed edge-density — all already-gated). Make the relevance call *against this cogmap's telos/questions*,
  not a universal threshold; act via one of the five learning-acts, emitting provenance-with-stance; if it
  learned about *cultivation* (not the domain), write a regulation concept-resource (§3).

- **The sweeper (recursion-persona), and why awareness-is-access-bounded is now automatic.** The sweeper
  launches **within** a cogmap, primed by that cogmap's telos plus secondary type-of-work guidance (find
  gaps/lacunae; find cross-initiative impact within shared visibility bounds). It fires **content-free
  signal-events** — *"look for couplings of kind K"*, never *"review resource X"* — that cogmap-bound agents
  subscribe to. The seed note's awareness-leak (firing a referent into a cogmap that can't see it betrays
  existence-plus-relevance) **dissolves on two legs** under the landed access model:
  1. **Edges are RBAC-bound homed objects.** Any edge an agent creates is only visible to those who can
     already see it (`edges_visible_to`); an edge to something you can't access simply is not in your view.
     Edge *creation* is therefore always safe — the guard *is* the edge's own visibility. There is no
     existence-leak to pre-check.
  2. **The signal carries no referent.** Each subscribing agent acts entirely within its own
     `resources_accessible_to_cogmap` least-privilege view; second-order concept-creation / scar-deformation
     stays inside its bounds. The signal propagates; the referent never crosses.

  So the seed note's special "pre-check the scope's access before emission" rule is **subsumed** — awareness
  is access-bounded *automatically*, because the substrate (RBAC-bound edges + content-free signals) makes it
  so. The sweeper really is "just another event source" — another **emitter entity** in the ledger (§6),
  indistinguishable at the substrate from a Notion webhook save for what its events carry.

---

## The convention registry (a documented contract, not DDL)

Domain B is expressed entirely through the kernel's primitives plus these conventions:

| convention | kernel form | value |
|---|---|---|
| **the charter** | `kb_cogmaps.telos_resource_id` (NOT NULL, landed map-regions §0) | the cogmap's seed resource |
| **telos statement** | charter block-0 | purpose + thin-who |
| **guiding questions** | charter blocks 1..n | the question-set |
| **regulation** | cogmap-homed concept-resources, `express`-edged from the charter | open/growing set |
| **`express` labels** | `kb_edges.label` on `edge_kind='express'` | `cultivated_by` (skill→cogmap), `operationalized_by` (telos→regulation) |
| **`near` labels** | `kb_edges.label` on `edge_kind='near'` | `regulation_inherited_from` (parent→child) |
| **doc_type (demoted)** | `kb_properties` `key='doc_type'` | charter, regulation, concept tags (exact values — plan-level) |
| **genesis event** | `kb_events` | `scope_seeded` (exact naming — plan-level) |
| **question routing / sweeper signal** | `kb_events` (content-free) | a signal of what-to-look-for, never a referent |
| **agent-instance (the actor)** | `event_substrate.entities` row (`emitter_entity_id`); reads as `Cogmap(M)` | runtime launch-metadata: model/platform, bound-cogmap reach, persona, priming-telos |

No new **Domain-B** tables — agents emit as entities in the existing ledger (like any external-system
source); questions/regulation ride the `block_*` and edge projections; no new event family; no forced shape.

---

## Out of Scope

- **The clustering algorithm / region materialization** — owned by
  [`map-regions`](2026-06-02-map-regions-self-materialized-shape-surface-design.md) (`kb_cogmap_regions`).
- **The access/capability model** — teams:RBAC, the edge-home, `resources_accessible_to_cogmap`,
  `cogmaps_share_a_team` are owned by the access + delegation specs; this spec *consumes* them.
- **The triage/sweeper/steward *judgment*** — the relevance call, the clustering threshold, the sweeper's
  **orthogonal-signal detection** (the seed note's load-bearing open question: how to detect a salience-miss
  without the salience-metric that missed it; instinct = structural/provenance coupling). These are the
  black-box Domain-B producers' work, staged with the synthesis agents.
- **Tier-two genome promotion** — promoting a regulation lesson back into the *portable* seed-skill, gated by
  falsification-against-maximally-different-cogmap-shape; **human-gated** until "cogmap-shape" is formally
  computable (seed note carried-forward).
- **#15 chained-routing** (bridge *carries* vs. *records* intent across multi-hop question-routing) — a
  behavior/protocol question, not table design; flagged load-bearing-open in the lineage.
- **Migration phase-ordering and crate packaging** (`cogmap_genesis` home; build Limb 1c → extract
  `temper-substrate` → birth `temper-cogmap`) — **spine #3**, a plan-level decision.

## Plan-level questions (resolve during implementation planning)

1. **Reinforce derivation** — precisely which event(s) constitute "reinforcement" of a question-block (any
   `{kind:block}` reference? only confirming triage acts?), and the read that turns the stream into a
   salience signal. — **RESOLVED (2026-06-04, schema.sql prep):** for the artifact's scenarios,
   reinforcement = **any `{kind:block}` reference into the question-block**; the read aggregates count +
   recency over that stream into the salience signal. Narrowing to only-confirming-triage-acts is a
   tuning question left to the black-box producers — the substrate just exposes the reference stream.
2. **Scar linkage** — does the scar-lesson regulation-resource link back to the folded question-block via
   `kb_block_provenance` (`{kind:block}`) or via an edge? (Lean: provenance — the lesson *came from* scarring
   that question.) — **RESOLVED (2026-06-04, schema.sql prep):** **provenance** (`kb_block_provenance`,
   `{kind:block}`). The lesson *came from* scarring that question — a provenance relation, not a
   first-class graph edge. Keeps the regulation graph (express/near) clean of lineage bookkeeping.
3. **doc_type values** — the demoted `kb_properties` `key='doc_type'` values for the charter and for
   regulation concepts (`cogmap_charter` / `cogmap_regulation` / plain `concept`?), and whether render/UI ever
   needs to distinguish a framing-statement block from a question block (if so, that un-defers
   content-block's `block_kind`; YAGNI until forced). — **RESOLVED (2026-06-04, schema.sql prep):**
   `doc_type` values `cogmap_charter` / `cogmap_regulation` / `concept`. `block_kind` stays **deferred**
   (YAGNI) — nothing in the artifact's scenarios needs to distinguish a framing-statement block from a
   question block at the storage layer; block-0-is-telos is positional convention.
4. **Genesis event naming** — `scope_seeded` vs a `cogmap_*`-prefixed name, consistent with the CS-3 sweep.
   — **RESOLVED (2026-06-04, schema.sql prep):** **`cogmap_seeded`**, consistent with the CS-3
   scope→cogmap sweep. Seeded as its own `event_type` row in the artifact.
5. **`cogmap_genesis` packaging** — one SQL function in the artifact vs. a `temper-cogmap` orchestration over
   neutral substrate commands (the carve-out seam; spine #3). — **RESOLVED (2026-06-04, schema.sql
   prep):** **one SQL function in the artifact** (`cogmap_genesis(...)`). The eventual crate seam
   (temper-cogmap orchestration over neutral substrate commands) is spine #3's call — not the artifact's
   problem; the artifact only needs the single-txn seeding to be runnable.
6. **Artifact read-projections** — which Domain-B reads the `schema.sql` artifact emits to run scenarios:
   `cogmap_charter(cogmap)`, `cogmap_questions(cogmap)` (charter blocks), `cogmap_regulation(cogmap)`
   (`express`-edge traversal). These are *"just compute"* over already-gated data (map-regions' line), needing
   no new access primitive. — **RESOLVED (2026-06-04, schema.sql prep):** emit all three —
   `cogmap_charter(cogmap)`, `cogmap_questions(cogmap)`, `cogmap_regulation(cogmap)` — as read
   projections over already-gated data. No new access primitive.
7. **Agent-instance entity launch-metadata home** (§6) — does the launch-metadata (model/platform,
   bound-cogmap reach, persona, priming-telos) ride the entity `name` + launch-event `metadata`, or does it
   un-defer the event-substrate **entity-typology** (`agent | human | integration`)? A cross-spec dependency on
   `2026-05-18-event-substrate-foundations-design`, not a Domain-B table — but Domain-B agents are its first
   concrete driver, so the cut should be made when this lands. — **RESOLVED (2026-06-04, schema.sql
   prep) — neither; a third path:** add an **open `metadata jsonb`** column to
   `event_substrate.entities` (default `'{}'`), **not** a hard `entity_kind` enum/typology. An agent
   instance populates it with its launch-metadata (model/platform, bound-cogmap reach, persona,
   priming-telos); an integration-shaped entity populates differently; neither is forced into the
   other's shape. Rationale: we don't yet know the full capture shape, entities span ephemeral
   (agent-instance) to long-lived (integration), and consistently-shaped ephemeral metadata may later
   *promote* to a profile property — a jsonb keeps that flexibility open where a frozen enum would not.
   The typed-typology cut, if ever earned, is deferred to the event-substrate spec on real evidence.

## Connections

- **Discharges:** [`data-model-reconciliation`](2026-06-01-data-model-reconciliation-design.md) §"Out of
  Scope" (the spine-#2 carve-out) — and revises its "telos-as-`kb_properties`-facet" line (§1 here).
- **Builds on:** [`map-regions`](2026-06-02-map-regions-self-materialized-shape-surface-design.md) §0
  (`telos_resource_id`; incubation-home; tightens its mutual-reference plan note),
  [`content-block-primitive`](2026-06-03-content-block-primitive-design.md) (blocks; `block_*` family;
  `{kind:block}`), [`access-capability-model`](2026-06-02-access-capability-model-design.md) +
  [`map-to-map-delegation`](2026-06-02-map-to-map-delegation-dissolution-design.md) (teams:RBAC; edge-home;
  `resources_accessible_to_cogmap`; `cogmaps_share_a_team`).
- **Conceptual lineage:**
  `2026-05-31-definitional-fallacy-concept-as-basin-telos-resolves-threshold-primitive` (concept-as-tool;
  telos resolves the threshold-primitive; structure-in-edges-not-nodes),
  `2026-06-01-seed-skill-scope-portable-vs-bound-awareness-access-bounded` (seed = telos + thin-who +
  question-set; questions-as-trajectory; `express`/`near` channels; regulation; the sweeper and
  awareness-is-access-bounded), `2026-05-31-temper-confidence-inventory` (#14 threshold; #17/#18 granularity;
  porosity = Drifted #19).
- **Feeds:** the fresh one-shot `schema.sql` artifact (separate PG namespace) — seed scripts + scenario
  queries to evaluate the cognitive-map model empirically before the phased migration (project strategy).
