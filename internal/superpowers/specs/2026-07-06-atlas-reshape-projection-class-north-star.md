# Atlas Reshape — a projection-class-grounded north star

**Status:** north-star / decision record (not an implementation spec). Decomposes into
beats A–E, each specced separately. Beat A is specced first.

**Origin:** emerged mid-flight during the C3.1 territory-label work (task
`019f38b3`). Fixing "region labels are illegible stubs" surfaced that the label
problem was a *symptom* of an unclear panorama: what is it a survey *of*, what
should its circles *encode*, and why does an inert context sphere sit among
enterable regions. Stepping back to first principles — using this repo's own
**projection-class / informational-act** framework — reshaped the whole Atlas IA.

---

## 1. The lens: projection-classes as the design rubric

We already have a strong theory of *what a knowledge surface is for*, sitting in the
vault and under-used. We are now promoting it from background theory to the **rubric we
design against**.

A **projection-class** is an informational act modeled as a **function** over the
event-sourced substrate: `f(substrate, perspective, time, …) → projection`. Its
**signature is an attention-contract** — what it consumes and the *shape/size* of what
it returns encodes the attention it means to preserve. Naming the act ("orientation",
"wayfinding") makes its attention-concern available to design deliberately instead of
leaving it to the designer's vigilance (names-as-tools).

The acts we design the Atlas for (signatures paraphrased from the framework docs):

| Act | Signature (attention-contract) | Attention shape |
|-----|--------------------------------|-----------------|
| **Orientation** | `(substrate, perspective, time) → small structured survey` | survey — *the smallness IS the preservation* |
| **Wayfinding** | `(substrate, perspective, from) → path/traversal to a target` | route — from here to there |
| **Recall** | `(substrate, perspective, time, key) → resource-with-context` | pinpoint |
| **Boundary-sensing** | `(substrate, perspective, time, region) → known-edges + unknowns` | edge |
| **Composition** | `(substrate, perspective, scope) → workable assembly` | depth |

**The rubric:** every Atlas surface serves one or more projection-classes; each surface
must **honor its act's attention-contract**; and when a design question is contested
(how many labels? what does size mean? what belongs in this view?), **the attention-
contract adjudicates it** — not taste. Several load-bearing decisions below are
*derived* this way, and are tagged with the act that forces them.

> Framework sources (vault): "Cognitive Maps and the Projection-Class Insight"
> (`019e54b9`), "Projection-Class Plurality: Research-Area Decomposition" (`019e54bb`),
> "Projection-Classes as Functions: First Specifications" (`019e5530`), "…Specification,
> Composition, and the Translation Family" (`019e552c`).

---

## 2. Two personas = two lenses, no role gate

The Atlas serves two ways of engaging, which are **lenses, not roles** — the same
person flips between them freely, and there is no permission boundary:

- **Builder / workflow lens** — lives *in* temper. Works through **contexts**
  (`general`, `tasker`, `temper`, `writing`…) full of tasks/goals/sessions/research,
  and also reads cogmaps. Traverses and utilizes. Reaches: **{contexts, cogmaps}**.
- **Consumer / knowledge lens** — *relies on* temper without working in it. Never
  creates a task; learns the org/team/work through **cognitive maps** and their
  **telos-charters**, often arriving via integrations (GitHub / Linear / Notion),
  possibly deep-linked straight to a region, never touching home. Reaches: **cogmaps**.

**Cogmaps are universal; contexts are the builder's *additional* axis.** This is the
correction that unmuddies the panorama: the two axes were being co-displayed in one
view, so a context (builder axis) sat inert among regions (knowledge axis).

---

## 3. Surface → projection-class map

| Atlas surface | Primary act(s) | Contract consequence |
|---------------|----------------|----------------------|
| **Home** (teams + cogmaps) | Orientation (of your whole footprint) | few sized containers, linked; not a wall |
| **Cogmap panorama** (regions) | Orientation (of a knowledge domain) | **gate** labels, **encode magnitude**; label-all is an anti-goal |
| **Region hover** | Boundary-sensing / recall-preview | surface resource-count · salience · coherence |
| **Search** | Wayfinding + recall | spans both axes; the "from-anywhere → target" path |
| **Region → resources** | Composition / wayfinding | force-graph depth; link the context-resources |
| **Team panorama** (contexts) | Orientation (of your workspace) | one coherent axis; recency = "what's alive" |
| **Context view** | Composition (builder work) | depth; Part D re-imagining |

---

## 4. Decisions that fall out (each tagged with the act that forces it)

1. **Size-encoding is required.** [Orientation] A survey is survey-attention; seeing
   relative magnitude at a glance *is* the projection's job. Region circles must encode
   salience as size (and see D4's field-effect). Losing size (uniform circles) breaks
   the orientation contract.
2. **Label gating, not label-all.** [Orientation] "The smallness *is* the attention-
   preservation." A wall of 58 labels is not a richer orientation — it is a *failed*
   one (it has become an unindexed wayfinding surface). Name a few landmarks; reveal the
   rest on hover; find the specific via search. **The gate is mandated, not a compromise.**
3. **A panorama surveys ONE subject.** [Orientation-coherence] Mixing regions and
   contexts breaks the survey's coherence. **Regions leave the team panorama** and live
   under the cogmap door; the team panorama becomes purely contexts. The inert-sphere
   defect dissolves.
4. **Regions render as a field, not a graph.** [Orientation] Force-separated + gated
   (adopting the Tier-2 neighborhood visual language for consistency), but salience is
   drawn *heavier* via foreground density — higher opacity + glow where salient — so the
   panorama reads as a **field-sense** (heat), the shape of a knowledge domain, rather
   than discrete nodes.
5. **Wayfinding is served by search + drill + bridges** — not by cramming labels into
   the orientation surface. Keep the two acts' surfaces distinct.
6. **Home drops the `you` node.** [Orientation] Show **teams + cogmaps** directly, self
   implied, team↔cogmap links kept where they fall. Less chrome, same survey.
7. **Region → resources is composition.** Click a region (or **shift-click several** for
   a union) → force-graph of its resources (ideas/concepts/facts) **plus the context-
   resources they link to** — the knowledge axis meeting the builder axis.
8. **Team → contexts encodes size + recency.** [Orientation] Contexts sized by
   resources, weighted by recency ("what's alive"). One coherent axis.
9. **Every field view has an a11y fallback.** A plain **list of links + metadata** is
   the accessible equivalent of the field; small regions stay hoverable/clickable
   entrypoints, never keyboard-dead.

---

## 5. The Atlas, tier by tier (the vision)

- **Home.** Teams and cogmaps, `you` implied, linked where they fall. Two lenses at the
  fork: teams (builder) and cogmaps (universal).
- **Cogmap panorama — the knowledge field.** Force+gate regions; salience field-effect
  (opacity/glow); major regions named; hover reveals resource-count · salience ·
  coherence; small regions remain accessible entrypoints; a11y list fallback. Anchored
  by the cogmap's telos-charter (the consumer's "what is this *for*").
- **Region → resources.** Single- or shift-multi-select regions → force-graph of
  resources + linked context-resources. As today's Tier-2, enriched.
- **Team panorama — the workspace.** The team's contexts, sized + recency-weighted;
  every circle enterable; nothing inert. Lists the team's cogmap doors for the cross-
  lens hop.
- **Context view.** As today, or re-imagined under Part D (repoint / retire the old
  `/[owner]/[context]/graph` cytoscape).

---

## 6. Decomposition — beats A–E

Each beat is independently shippable with its own spec → plan → build.

| Beat | Scope | Subsumes / relates | Grain |
|------|-------|--------------------|-------|
| **A — Cogmap panorama as knowledge field** | force+gate layout · salience field-effect (opacity/glow) · hover metadata (resources · salience · coherence) · a11y list fallback · Problem 2 derived labels on `graph_cogmap_territories` | **task `019f38b3`** (territory labels) | cogmap |
| **B — Home reframe** | drop `you` node; teams+cogmaps linked; self implied | — | home |
| **C — Team panorama = contexts** | regions *leave* the team panorama (axis un-mix); contexts sized + recency; backend read change (team panorama stops serving `graph_region_territories`) | resolves inert-sphere defect | team |
| **D — Region → resources drill** | force-graph + linked context-resources + shift-click multi-region union | enriches today's Tier-2 | region |
| **E — Context view re-imagining** | repoint / retire the old context-specific cytoscape | **Chunk D** (roadmapped) | context |

**Order:** **A first** (heart of the vision; live `/dev/atlas` harness ready; folds in
the original label task), then **C** (the structural axis-split that makes every
panorama coherent), then **B**, **D**, **E**. Order is advisory; A and C are the
load-bearing pair.

---

## 7. Open questions / deferred

- **Team-grain aggregate knowledge view** — *deferred (leaning: don't build).* We chose
  the **cogmap grain** for the knowledge survey (one cogmap's regions). A team-wide
  "all regions across all the team's cogmaps" survey is a possible second surface for
  the consumer who wants "everything this team knows," but risks an unbounded aggregate;
  revisit only if the per-cogmap grain proves insufficient.
- **Field-effect rendering specifics** — exact opacity/glow curve, salience→size scale,
  gate-K and collision tuning: settled in the Beat A spec against the harness.
- **Coherence metric surfacing** — the hover wants a "coherence" figure; confirm the
  read exists (region coherence/affinity) or add it. (Beat A.)
- **Part D scope** — how far to re-imagine vs merely repoint the context cytoscape.
  (Beat E.)

---

## 8. Connections

- `[[project_graph_atlas_visualization_goal]]` — this reshape is the next arc of the
  Atlas goal (`019f28a1`).
- Framework: projection-class docs `019e54b9` / `019e54bb` / `019e5530` / `019e552c` —
  now foregrounded as the standing design rubric for the Atlas, not background theory.
- C3.1 umbrella task `019f2fbe`; territory-label task `019f38b3` (subsumed by Beat A);
  Chunk D (becomes Beat E).
- `[[feedback_local_proddata_render_harness_for_ui]]` — `/dev/atlas` harness drives the
  Beat A field/label iteration (bypasses the loader; fixtures need real labels injected).
