# Emergent Region Projection: The Deterministic Telos-Lens Producer

**Date:** 2026-06-06
**Status:** Design — drafted from a brainstorming session 2026-06-06. Ready for spec review → implementation plan.
**Goal:** `substrate-kernel-to-cognitive-map`, Arc 1 (shared-kernel completion)
**Spun out of:** [`2026-06-02-map-regions-self-materialized-shape-surface-design.md`](2026-06-02-map-regions-self-materialized-shape-surface-design.md) — that spec defined the region *surface + read* and deferred "the clustering algorithm, the salience-threshold judgment" to "an opaque Domain-B producer." **This is that producer — and the central move of this spec is that it is *not* opaque, *not* an agent, and *not* an LLM: it is deterministic compute.**

> **Grounding note.** Written against the **schema-artifact** (`schema-artifact/{01_schema,02_functions,03_seed,04_scenarios}.sql`, the `temper_next` destination schema) and the five Arc-1 specs it composes with (map-regions, access-capability, data-model-reconciliation, domain-b-charter-questions, content-block-primitive). The conceptual frame is the **projection-classes-as-functions** research (`2026-05-23-*`). Where this spec says "built" vs "designed" it has been checked: `kb_cogmap_regions` / `kb_cogmap_region_members` / `cogmap_shape` / `cogmap_staleness` exist in the artifact with a **hand-seeded** region (S6); `kb_properties` exists with `owner_table ∈ ('kb_resources','kb_cogmaps')`; `kb_chunks.embedding` is `vector(768)` but **the seed carries no embeddings**; `temper-ingest` embeds with **`BAAI/bge-base-en-v1.5`** (768-dim, `crates/temper-ingest/src/embed.rs`).

---

## 0. The reframe: the producer is deterministic compute

The map-regions spec treated the thing that produces a region's shape as an **opaque Domain-B agent** that *judges* clustering, salience, and weighting. This spec replaces that framing:

- A cognitive-map **region is computable** — deterministically and reproducibly — from the **resources, edges, and properties homed in the cogmap**, at a point in time. It is not judged; it is *derived*.
- The judgment that the map-regions spec located *in the producer* is relocated to where the rest of the model already puts it: the **input grain**. Agents and humans hydrate the substrate — edge `kind`/`direction`/`weight`/`label`, and `kb_properties` facets/keywords — as explicit, discrete, weighted, event-attributable declarations. **Region formation is then a pure function of those declared inputs.** This is the projection-classes seam verbatim: *"judgment hydrates inputs; computation is deterministic."*
- Therefore **`temper-llm` is *not* this producer.** `temper-llm` (the lightweight agent harness) is a *consumer* of this surface — the deployment-gated "steward" that triages untriaged events and *hydrates inputs* — and is out of scope here (§Out of scope). Region computation owes nothing to it.

The payoff is empirical: the schema-artifact exists to evaluate the model before any migration. A deterministic producer turns "regions are computable" from an assertion into a **demonstrated, falsifiable** fact (§5).

---

## 1. The function and its first instance

The whole of this spec is **one pure function**, surfaced as a memoized projection:

```
materialize_regions(
    selection : SubstrateSelection,   // which homed objects are in view
    weighting : Lens,                 // the perspective: per-edge_kind + per-property weights
    at        : EventWatermark        // the point-in-time snapshot
) -> RegionSet                        // deterministic · reproducible
```

- **`selection`** — the homed objects under view. The telos-lens default is *everything homed in cogmap C* (the access-model homing predicate). This is the **only** selection built in the first cut; the team / sub-selection variants are this *same parameter*, varied later.
- **`weighting` (the Lens)** — a small, legible vector: a weight per `edge_kind ∈ {express, contains, leads_to, near}`, per-property/facet weights, the salience-blend weights, and a clustering resolution. The telos-lens default is a **named, stored, immutable row** (§3). This *is* the "intentional weights-as-floats," at its true size — a handful of floats. **Plurality = vary this vector.**
- **`at` (watermark)** — `max(event)` over the selection's homed objects (the on-read staleness aggregate already in the artifact, A3-3). **Reproducibility contract:** same `(selection, weighting, at)` ⇒ byte-identical `RegionSet`.

**The unifying claim (why plurality is a seam, not new machinery):** other lenses are this same function with different arguments — a team's cross-map sub-selection (*which clusters emerge from that slice*), or mutated edge weights (*a different point of view on relatedness*). The operation is invariant; only the inputs vary. A stored `kb_cogmap_regions` row is therefore the **memoization** of this function: `centroid` and `salience` are *computed*, not authored; the table is a cache of a pure projection, and the watermark is its freshness.

---

## 2. The derivations and the determinism contract

Three pure stages.

### 2a · The declared affinity (stage 1)

For two selected concept-resources `i, j`:

```
affinity(i,j) =  Σ_e [ lens.w_kind[e.edge_kind] · e.weight · lens.label_factor(e.label) ]   // declared edges
              +  lens.w_prop · facet_overlap(i, j)                                            // declared properties
```

- The sum is over **declared edges** between `i` and `j`. **Cosine is absent.** No declared path ⇒ no affinity ⇒ no co-membership.
- **`lens.label_factor` defaults to `1` for every label.** There are **no reserved label literals** — formation keys off nothing semantic by default, so no labeler must learn a reserved vocabulary. A lens *may* override specific labels/properties explicitly (including negatively) as part of *its own* declared parameterization, but the telos-default treats every label as ordinary positive relatedness.
- **`facet_overlap`** is shared `kb_properties` facets/keywords between the two resources. It is *declared* (someone tagged both), so by the rule **only declared signals form**, it is admissible — unlike cosine. Lens-weighted (default modest); it gives a young, lightly-edged map some shape from tagging alone. Metric: `min`-weighted overlap over shared `(path, value)` pairs (§4).
- **Contradiction binds, it does not separate.** *You can only contradict within a shared frame* — to assert "A contradicts B" you already hold A and B against the same question. A contradiction is therefore **evidence of co-regionality**, often heightened salience (the charter's "where are the sharp edges?"). It is recorded on the edge and surfaced as `internal_tension` (§2c), **never** used to fracture a region. **There is no cannot-link, no signed-network clustering, and no declared "push-apart": separation is the *absence* of affinity, full stop.**
- `polarity` (forward/inverse) is preserved on edges for region interpretation; formation affinity is symmetric magnitude — directionality describes a region, it does not split it.

### 2b · Deterministic clustering (stage 2)

Over the positive-affinity graph. The **hard spec commitment is the contract, not the algorithm**: *order-stable, no random initialization, reproducible* (re-run @ same watermark → identical membership). Recommended first pass: **average-link hierarchical agglomerative** with a stable UUID tie-break, cut at a **lens-parameterized `resolution`** — deterministic with no seed gymnastics, yields nested structure (sub-regions for free, for the later zoom/plurality story), and the cut is a natural lens knob. Leiden-with-fixed-seed is the plan-level alternative. **The exact algorithm is plan-level; determinism and reproducibility are spec-level.**

### 2c · Derived readouts (stage 3)

Per formed region `R`, every one a pure aggregate; **cosine appears here only — downstream of a formed region, never in formation:**

- **`centroid`** = pool-per-concept-then-mean of member chunk embeddings (map-regions OQ-1, resolved).
- **`content_cohesion`** = mean member-to-centroid cosine — the surface-vs-relational character readout. Low cohesion + tight declared structure = a **relational-surplus** region (the map's distinctive knowledge, which a vector index would scatter).
- **`salience`** = lens-weighted blend of three derived terms, stored **decomposed** for legibility:
  - **`telos_alignment`** = `cosine(centroid, telos_resource.embedding)` — "importance under the map's telos," literal because the telos *is* a resource with an embedding;
  - **`reference_standing`** = aggregate of members' `reinforce_count` (a `count()` over `kb_block_provenance`, page-04 precedent — derived, never stored);
  - **`centrality`** = the region's internal declared-affinity density × size.
  `salience = lens.s_telos·telos_alignment + lens.s_ref·reference_standing + lens.s_central·centrality`.
- **`internal_tension`** = a measure over oppositional-labeled declared edges among members — a *feature* of the region (feeding salience/insight and the steward's attention), never a fracture.
- **member `affinity`** = each member's nearness to centroid (core vs peripheral).

### The contract (the section's spine)

`materialize_regions` is **pure over (selected rows @ watermark, lens vector)** → identical `RegionSet` (membership *and* every derived float) on re-run. It holds because formation is declared-only (no embedding-coincidence drift), clustering is order-stable, and readouts are pure aggregates. It is **directly tested** (§5): run twice → byte-identical; emit one new edge event → watermark advances → regions update *predictably* (the functorial-with-narrowing flavor — an event yields a coherent projection-update, not a reshuffle).

---

## 3. The schema delta on `kb_cogmap_regions`

### A · Salience: computed, memoized, decomposed

`salience` **stays** (no type change); its meaning flips from "agent-assigned" to "computed, memoized." The decomposition is stored beside the blend — the artifact's job is to *measure*, and this keeps salience legible (page-04's "never a baked-in opaque number"):

```sql
ALTER TABLE kb_cogmap_regions
  -- salience comment: computed blend, memoized (was agent-assigned)
  ADD COLUMN telos_alignment    DOUBLE PRECISION,   -- cosine(centroid, telos_resource.embedding)
  ADD COLUMN reference_standing  DOUBLE PRECISION,   -- aggregate reinforce_count over members
  ADD COLUMN centrality          DOUBLE PRECISION,   -- internal declared-affinity density × size
  ADD COLUMN content_cohesion    DOUBLE PRECISION,   -- mean member-to-centroid cosine (surface↔relational)
  ADD COLUMN internal_tension    DOUBLE PRECISION;   -- over oppositional-labeled declared edges among members
```

All five are surface-safe aggregates (no member-identity leak). *(Decision 3A: store the decomposition — production may later collapse to the scalar and recompute.)*

### B · The lens: declared, stored, immutable data (the plurality seam)

```sql
CREATE TABLE kb_cogmap_lenses (
    id                   UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    cogmap_id            UUID REFERENCES kb_cogmaps(id),  -- NULL = global default; non-null = map-specific
    name                 TEXT NOT NULL,                   -- 'telos-default', …
    selection_kind       TEXT NOT NULL,                   -- 'homed' (first cut); 'team_visible' later
    w_express, w_contains, w_leads_to, w_near  DOUBLE PRECISION NOT NULL,  -- per-edge_kind affinity weights
    w_prop               DOUBLE PRECISION NOT NULL,       -- facet-overlap weight
    s_telos, s_ref, s_central                  DOUBLE PRECISION NOT NULL,  -- salience blend weights
    resolution           DOUBLE PRECISION NOT NULL,       -- agglomerative cut
    asserted_by_event_id UUID NOT NULL REFERENCES kb_events(id),
    created              TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE kb_cogmap_regions
  ADD COLUMN lens_id UUID NOT NULL REFERENCES kb_cogmap_lenses(id);
```

Lenses are **immutable/asserted** — editing a lens means asserting a *new* row; a region's `lens_id` pins the exact weight-vector it was computed under, which is what **anchors reproducibility**. First build seeds **exactly one** row (telos-default); every region points at it. Plurality = insert another row; the function is unchanged. *(Decision 3C: named per-kind columns — the 4-way `edge_kind` carve is deliberately stable; a normalized `kb_cogmap_lens_kind_weights` child is the kind-agnostic alternative.)*

### C · Emergent is the only kind — `region_boundary_kind` is **dropped**

The earlier map-regions plan modeled an emergent-vs-declared boundary so an authority could *declare* a region. **Removed.** A declared region is, by construction, **not a projection** — it has no "prove it from point-in-time Y," only "we said so" — which breaks the one invariant the design rests on and is the surface most prone to going stale. It is also redundant: the honest expression of "these belong together" is to **assert the declared relations** (edges, weights, facets) until the region *emerges*, grounded and re-projectable. The intentional act lives at the input grain, where it is evidence, not assertion. The invariant strengthens to its unqualified form: **every region, no exceptions, is a pure projection provable from substrate @ watermark.** (If the emergent↔authority-driven *spectrum* is ever wanted, it is a **derived readout** over member-edge provenance — beside `internal_tension` — never a stored declaration. Re-adding an enum is a cheap pure-addition if a genuine need appears.)

The "region-ify this" ergonomic need is met without dishonesty by **bulk-facet/keyword helpers** (API/MCP tooling, still event-emitting and attributable) — hydration ergonomics, out of scope here (§Out of scope).

### D · The surface read gains the new signals + a lens selector

```sql
cogmap_shape(p_cogmap, p_principal_kind, p_principal_id, p_lens DEFAULT <telos-default>)
  RETURNS TABLE(region_id, lens_id, salience, content_cohesion, label, member_count)
```

(No `boundary_kind` — every region is emergent, per C.) The member-level interior stays gated exactly as before; the cross-map shape-projection (how-maps-relate) reads the telos-default lens's shape.

### E · The amend-and-scar of map-regions §5

Surgical, not a banishment of cosine:

- **Scar** §5's closing note — *"the same three signals are what the agent clusters over to produce regions."* That line made cosine a **formation** signal. Retired: **formation is declared-only**, because cosine-in-formation conflates *computed* similarity with *declared* relatedness and muddies the emergent provenance the whole model rests on. (The conversation's transient "formation metric = locate metric" idea dies by the same lesson.)
- **Preserve** §5's *locate* function: a free-text/embedding **query→region** proximity legitimately uses cosine-to-centroid — it is the only signal an embedding query *has*. Cosine's role now **differs by operation**: out of formation, alive in locate.

---

## 4. `kb_properties`: facets/keywords

### 4a · `owner_table += 'kb_edges'`

```sql
-- kb_properties.owner_table CHECK
CHECK (owner_table IN ('kb_resources', 'kb_cogmaps', 'kb_edges'))
```

`facet_overlap` rides on **resource** facets (already supported); the extension gives **edge** facets a typed home (e.g. `{stance:opposed}` feeding `internal_tension`, richer lens weighting) instead of overloading the single `label` string. Matches the declared model where an edge carries "kind, direction, weight, labels, facets, keywords."

### 4b · The facet/keyword model

Facets/keywords are ordinary `kb_properties` rows — `(owner, property_key, property_value jsonb, weight)`, foldable and event-asserted. Convention: `property_key = 'facet'`, `property_value` a `{dimension: value}` object (`{"topic":"deployment"}`, `{"phase":"first-week"}`, `{"stance":"opposed"}`), `weight` = asserter confidence. `facet_overlap(i,j)` = **`min`-weighted overlap over shared `(path, value)` pairs** (a facet is only as strong as its weaker assertion). Reads *only declared facets*, never cosine.

**jsonb-nesting is the graded-overlap seam.** `property_value` is jsonb and free to nest *without* a prescribed taxonomy. The first cut computes **flat exact `(path, value)` match**; **hierarchical/graded overlap** (`topic:deployment:rollback` ⋂ `topic:deployment:flags` earning partial credit on the shared `topic:deployment` ancestor) is a later **function-body** refinement — *no schema change*, turned on where patterns of use prove it wanted.

### 4c · Facet *vocabulary* governance is out of scope

The facet vocabulary should be **living declared data, telos-seeded and regulation-grown** — a charter-offered facet-contract (`kb_properties` on the cogmap/charter), extended by regulation-resources, conformed-to by consent where use coheres, never by decree (the phenomenological commitment: facets earn their place by what they let concepts *do*, not what they *are*). **But region computation does not — and must not — read the contract.** It consumes whatever declared facets exist, however they came to be. Facet-contract governance (and the bulk-facet helpers of §3C) are **hydration ergonomics**, a separate unit, explicitly out of scope here.

---

## 5. The test frame (a falsification design)

The artifact's load order gains a step: **schema → functions → seed → harness → scenarios.** The seed carries authored content + facets + the lens row; the §6 harness embeds and computes; `04_scenarios.sql` asserts over what was materialized. (The *reproducibility* assertion straddles into the harness — the SQL frame asserts the result's shape/relationships; the harness asserts determinism of the computation.)

### 5a · The enriched cast (homed on `onboarding-cogmap`)

Telos: *safe, confident first-week contribution.*

| group | concepts | bound by | proves |
|---|---|---|---|
| **α — first-week confidence** | `pair-on-first-PR`, `smallest-real-change`, `early-confidence-signal` | `express`/`near` edges + `{phase:first-week}`; content **similar** | a *surface* region (high `content_cohesion`) |
| **β — deployment mechanics** | `staging-rollout`, `feature-flags`, `rollback-runbook`, `oncall-handoff` | `leads_to` flow + `{topic:deployment}`; content **divergent** | the *relational-surplus* region (low `content_cohesion`) — **headline** |
| **bridge** | `deploy-confidence-checklist` | `{topic:deployment}` facet, **no edge** | `facet_overlap` forming co-membership |
| **tension** | `blue-green`, `big-bang-cutover` | `contradicts`-labeled edge + `{stance:opposed}` edge-facet | tension **binds** + lifts `internal_tension` |
| **isolate** | `solo-retro-note` | content **similar to α**, **no declared link** | separation = absence (cosine does *not* form) |

### 5b · Authored content is the independent variable (the falsification hinge)

The frame is a **controlled experiment**: content is engineered so declared-structure and cosine-structure deliberately **disagree**, because only in the disagreement cells can you tell which one clustered.

| | cosine coherent | cosine incoherent |
|---|---|---|
| **declared coherent** | α — both agree (*confirms nothing*) | **β — must form** → declared forms |
| **declared incoherent** | **`solo-retro-note` — must NOT merge** → cosine doesn't form | background (trivially absent) |

β and `solo-retro-note` are the **discriminating cells where the hypothesis can fail and we would see it**. Uniform-similar prose collapses both into the non-discriminating corner — so authoring genuine cross-axis disagreement (α similar; β divergent; solo near-α but standalone) is a **correctness requirement of the frame**, not a nicety. Nothing is rigged: the embeddings must independently carry the structure the scenarios then assert.

### 5c · The lens row

One seeded `kb_cogmap_lenses` row, telos-default: `w_express`/`w_contains` high, `w_leads_to` mid, `w_near` low, `w_prop` modest, the `s_*` weights, a `resolution`.

### 5d · The region scenario suite (supersedes hand-seeded S6)

- **S6a · Computed shape** — `materialize_regions(onboarding, telos-default)` yields α and β (bridge folded into β, tension pair co-clustered). The hand-seed is gone; the shape is derived.
- **S6b · Reproducibility** *(harness-side)* — run twice @ same watermark → byte-identical regions, every derived float.
- **S6c · Surface vs relational** *(headline)* — `content_cohesion(α) > content_cohesion(β)`: β coherent yet content-divergent, the region a vector index would scatter, demonstrably held by declared structure.
- **S6d · Separation = absence** — `solo-retro-note` forms its **own** region; not absorbed into α. The clean proof cosine doesn't form.
- **S6e · `facet_overlap` forms** — the bridge concept joins β on the shared facet alone, no edge.
- **S6f · Plurality by varied input** — a *second* lens (e.g. `w_prop` high, `w_leads_to` low) over the *same* substrate yields a *different* region-set: same function, different arguments.
- **S6g · Tension binds** — `blue-green` and `big-bang-cutover` land in the **same** region *and* `internal_tension > 0`.
- **S6h · Functorial update + staleness** — emit one new edge event → `cogmap_staleness` flips stale → re-run → regions update predictably, and the surface read still never exposes member identities (the old S6 invariant, preserved).

---

## 6. The `temper-next` harness

Exists because the seed has **no embeddings** and deterministic community detection is not natural in SQL.

### 6a · Two jobs, a thin Rust surface

- **Job A — embed.** Chunk the authored content blocks, embed, write `kb_chunks.embedding`. **Reuse** `temper-ingest` (chunking + `BAAI/bge-base-en-v1.5`, 768-dim, already matching the schema) — no model decision, no dim mismatch.
- **Job B — cluster.** Read declared edges + facets via `sqlx`; build the affinity; run the deterministic agglomerative clustering (**the only genuinely new algorithm**); write `kb_cogmap_regions` + members (membership + `lens_id` + watermark) through the `MaterializeCogmapShape` pattern.

**Readouts stay in SQL.** `centroid`, `content_cohesion`, `telos_alignment`, `reference_standing`, `centrality`, `internal_tension` are computable from *membership + embeddings + edges*, so they live as **functions in `02_functions.sql`** — inspectable, trivially deterministic. Rust is confined to *embed* (reused) + *cluster-membership* (new), shrinking the determinism-risk surface to one small module.

### 6b · Size and disposition

Small — the heavy lift (embedding) is borrowed; the new surface is the clustering core + read/write plumbing. **Disposition (decided): option (b)** — a **standalone `temper-next` crate written to production quality** (unit-tested clustering core, reviewable), precisely so the affinity→cluster→membership core **lifts wholesale into `temper-cogmap`** later. Kept *out* of `temper-cogmap` for now to avoid the entangled-crate commitments that move would entail. The harness *plumbing* is throwaway; the clustering *core* is not.

### 6c · Doc fix (follow-up, separate)

CLAUDE.md and prose docs still cite all-MiniLM-L6-v2 (384) — stale since the ~3-day local-only era. The real model is `bge-base-en-v1.5` (768). A grep-sweep doc fix is a separate tiny follow-up, not part of this work.

---

## DDL delta (grounded against the artifact)

**New**
- `kb_cogmap_lenses` (immutable lens rows; seeded with one telos-default)
- `materialize_regions` cluster-membership (Rust, `temper-next`) + SQL readout functions (`02_functions.sql`)

**Changed**
- `kb_cogmap_regions`: `salience` recommented (computed/memoized); **add** `telos_alignment`, `reference_standing`, `centrality`, `content_cohesion`, `internal_tension`, `lens_id`
- `kb_properties.owner_table` CHECK: **add** `'kb_edges'`
- `cogmap_shape`: extend return (`lens_id`, `content_cohesion`) + optional `p_lens`
- `kb_chunks.embedding`: populated by the harness (was empty in seed)
- `03_seed.sql`: enriched cast + authored content + facets + lens row
- `04_scenarios.sql`: S6 → the S6a–h region suite

**Dropped**
- `region_boundary_kind` enum + `boundary_kind` column (never built; not introduced)

**Amended (spec doc)**
- map-regions §5 closing note — formation is declared-only; locate's cosine-to-centroid preserved

---

## Open questions (plan-level; not blockers)

1. **Clustering algorithm** — average-link agglomerative (lean) vs Leiden-fixed-seed. Determinism is the hard constraint either way.
2. **`resolution` default** and the `s_*` / `w_*` default vectors for the telos-default lens.
3. **`facet_overlap` exact formula** beyond `min`-weighted flat pairs; the graded/hierarchical (nesting) refinement.
4. **Per-kind weight storage** — named columns (lean) vs normalized child.
5. **Centroid HNSW** — deferred (map-regions OQ-2); per-map scan suffices until a cross-map locate path materializes.
6. **Where the SQL readout functions ultimately home** in the production crate topology (`temper-substrate` vs `temper-cogmap`).

---

## Out of scope

**Rejected (load-bearing — resist re-litigation):**
- **Cosine in region *formation*.** Formation is declared-only; cosine is salience + cohesion + discovery, downstream only. (Amends map-regions §5.)
- **`declared`/intentional regions** (a `boundary_kind`). Every region is emergent and projectable; "declaring" a region is an ungrounded, stale-prone non-projection. The intentional act lives at the input grain.
- **Cannot-link / signed-weight clustering / reserved label literals.** Contradiction *binds* (shared frame); separation is absence of affinity; no label is reserved.

**Deferred (in scope elsewhere or later):**
- **The steward / triage agent (②)** — `temper-llm` as the deployment-gated host that *consumes* this surface and hydrates inputs. A separate unit; this producer owes it nothing.
- **Deployment commitment (③)** — temperkb.io's answers to the operating questions, the "what wakes the steward" cadence, and the migration path.
- **Other lenses** — team / cross-map sub-selection and biased-relatedness weight-mutations. The schema seam (`kb_cogmap_lenses`, `lens_id`) supports them; the first cut builds only the telos-default.
- **Facet-contract governance + bulk-facet/keyword helpers** — telos-seeded, regulation-grown consent-contracts and the region-ify ergonomics. Hydration tooling; the computation never reads it.
- **The broader projection-class families** — the other 7 families / open research areas (`2026-05-23-*`). This spec is the region/clustering family, telos-lens instance.

---

## Connections

- **Spun out of / amends:** [`2026-06-02-map-regions-self-materialized-shape-surface-design.md`](2026-06-02-map-regions-self-materialized-shape-surface-design.md) — supplies the producer it deferred; amends §5 (cosine out of formation), §1/§7 (producer is deterministic compute, not an opaque agent), and the salience semantics (computed, not agent-assigned).
- **Composes with:** [`2026-06-02-access-capability-model-design.md`](2026-06-02-access-capability-model-design.md) (homing = the selection predicate; `MaterializeCogmapShape` auth), [`2026-06-01-data-model-reconciliation-design.md`](2026-06-01-data-model-reconciliation-design.md) (`kb_properties`, crate topology), [`2026-06-04-domain-b-charter-questions-regulation-edge-semantics-design.md`](2026-06-04-domain-b-charter-questions-regulation-edge-semantics-design.md) (the 4-way `edge_kind` carve; regulation as the facet-contract extender), [`2026-06-03-content-block-primitive-design.md`](2026-06-03-content-block-primitive-design.md) (the embedding grain Job A targets).
- **Research grounding:** `2026-05-23-projection-classes-as-functions-*` (the function/perspective/determinism seam; plurality as a family of projections; emergent-vs-intentional provenance), `2026-05-31-temper-confidence-inventory` (porosity / protocol-over-design drift).
- **Artifact:** `schema-artifact/` (the `temper_next` destination this evaluates against).
- **Goal:** `substrate-kernel-to-cognitive-map`, Arc 1.
