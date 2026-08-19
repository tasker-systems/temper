# Cogmap Telos-Differentiation Experiment — Run Log

Live capture of produced ids, per-phase observations, and verification outputs.
Plan: `docs/superpowers/plans/2026-07-06-cogmap-telos-differentiation-experiment.md`
Spec: `docs/superpowers/specs/2026-07-06-cogmap-telos-differentiation-experiment-design.md`

## Baseline (Task 0)

- **Branch:** `jct/cogmap-telos-differentiation-sop`, tree clean.
- **`cogmap-experiment` team:** absent (good) before Task 1.
- **Steward drift-sweep candidate baseline** — `steward_candidate_cogmaps(agent-y23aq…)`:
  | cogmap_id | name |
  |---|---|
  | `00000000-0000-0000-0005-000000000001` | system-default (L0) |
  | `019f2391-e001-7933-b88a-28fb92e56ac1` | Temper — self-cognition (steward map) |

  The steward can read/sweep **exactly these two** maps. **Invariant for Task 4:** after binding Map 2 + Map 3, this set must remain exactly these two — neither new map may appear.
- **Source slice (8 storyteller research resources):** all 8 readable ✓
  `narrative_graph` · `character_modeling` · `scene-model` · `world_design` · `emotional-model` · `data_driven_narrative_elicitation` · `system_architecture` · `design_philosophy`

## Genesis IDs (Tasks 1–4)

**Task 1 — experiment team**
- `EXPERIMENT_TEAM_ID` = `019f38b4-aad5-7aa2-9b58-c9d831d35c18` (slug `cogmap-experiment`)
- Roster: `j-cole-taylor` (owner) only. Steward `agent-y23aq…` absent ✓

**Task 3 — genesis**
- **Map 2 — Storyteller System Design:** `MAP2_ID` = `019f38b4-de02-7bd1-b96f-075e8ca95b0a`, `MAP2_TELOS_ID` = `019f38b4-de02-7bd1-b96f-076d34898c90`. Charter read-back: 1 statement + 5 questions + 2 framing ✓
- **Map 3 — Cognitive Maps for Storyteller:** `MAP3_ID` = `019f38b5-2048-7902-8159-c208f55ec3f9`, `MAP3_TELOS_ID` = `019f38b5-2048-7902-8159-c21cc85045bb`. Charter read-back: 1 statement + 5 questions + 2 framing ✓

**Task 4 — bind + grant + isolation gate**
- `cogmap_bind` Map 2 → experiment team: `bound: true`; Map 3 → experiment team: `bound: true`. Bound to the experiment team ONLY.
- `cogmap_grant` write→`@j-cole-taylor`: `granted: false` (idempotent no-op). **Definitive check:** `cogmap_authorable_by_profile(019d4add, MAP2)` = `t`, `(…, MAP3)` = `t` — the author is write-capable via admin.
- **ISOLATION GATE ✓** — `steward_candidate_cogmaps(agent-y23aq…)` after binding = exactly `{00000000-…0005-…0001 (L0), 019f2391 (steward map)}`, identical to the Task-0 baseline. Explicit exclusion: `map2_in_stewards_reach = f`, `map3_in_stewards_reach = f`. The steward cannot see or sweep the new maps.

## Phase 1 authoring (Task 5)

Two fresh **Sonnet-5** subagents, parallel, from empty, hypothesis withheld. Identical mechanics block; only charter + map id differed.

**Map 2 — Storyteller System Design** — invocation `019f38bf-0125-74d1-92ea-707a1c8c9857`
- 37 nodes (mostly `concept`, some `principle`/`decision`/`concern`), 67 edges (42 `derived_from` + 15 `supports` + 8 `relates_to` + 2 `part_of`), 3 facets.
- Node character: narrative-system **design concepts** — Narrative Gravity, Character as Tensor, Relational Web, Geological Character Time, Scene Anatomy, Genre as Dimensional Region, Theater-Company agents, Play-as-Creative-Freedom (Q5 tension held).

**Map 3 — Cognitive Maps for Storyteller** — invocation `019f38bf-4672-7452-999f-7309462b2eac`
- 13 nodes, 36 edges (22 `derived_from` + `feeds`/`supports`/`relates_to`/`informs`/`tension_with`), 0 facets.
- Node character: **fit-or-friction** between temper's cogmap model and narrative features — "relational web edges exceed temper's single-weight edge," "geological layers vs supersession," "scene warmed-data ≈ lens-scoped wayfind" (clean fit), "turn-cadence mutation outpaces materialize-on-threshold" (keystone gap), three-map-purposes, the gap register.

### (a) differentiation verdict — CONFIRMED (strong)
- **Shared-source proof (`character-modeling`):** Map 2 → *what the character model is* (Character-as-Tensor, Relational Web, Geological Time); Map 3 → *where temper strains against it* (edges-exceed-single-weight, layers-vs-supersession). Same features, telos-driven reframing: Map 3's nodes are meta-commentary on Map 2's nodes' subjects.
- **Blind-read test:** passes both directions. Edge vocabulary also diverged on-telos (Map 3 coined `tension_with`/`feeds`/`informs`).
- **Node-count asymmetry (37 vs 13)** tracks telos breadth (articulate-the-whole vs the-intersection) — differentiation, not noise.
- **Confound:** two agents; mitigated — same model + instructions-modulo-charter, and the meta-commentary structure is telos-attributable, not taste.

### Teachability signals (feed 019f373f)
1. **No confidence-band rubric** — both agents inferred one (Map 2 explicitly: "no rubric given"). Guidance should specify: `confident` = explicit/dated decisions & direct claims; `probable` = synthesized; `tentative` = thin/uncertain.
2. **Multi-source node dedup/merge** uninstructed — Map 2 merged concepts two docs independently assert into one multi-sourced node; sensible, but the guidance is silent on it.
3. **Cross-map linking is NOT spontaneous** — Map 3 read its scope narrowly ("this map only"), did not `search` neighboring maps, and turned the cross-map-reference *pattern* into a node instead of an actual link. Confirms (b): cross-map linking needs explicit direction. (Phase 2 tests it explicitly.)

### Bug candidates (surfaced by authoring)
- **B1 — `temper-llm-model` lands in `open_meta`, not `managed_meta`.** Verified on node `019f38c2-d205…`: managed_meta has `temper-provenance` + `temper-llm-run`; `temper-llm-model: claude-sonnet-5` sits in open_meta. Contract (map-stewardship + `create_resource` schema) puts the whole provenance trio in managed_meta.
- **B2 — `temper-slug` in `managed_meta` is inert; slug auto-derives from title; a non-ASCII title char broke slug generation** and failed a `create_resource` until the agent retitled to ASCII.
- **Open:** each map shows **2 invocations** (expected 1 per authoring agent) — verify in Task 7 whether genesis opens one too, or a retry occurred.

## Phase 2 cross-map (Task 6)

**Linking pass** (fresh Sonnet-5, Map 2 now visible) — invocation `019f38f3-6b27-7b21-aac2-2cec2ebcc126`
- **16 cross-map edges** Map 3 → Map 2, graded 0.6–0.95, meaningful labels (`analyzes`/`maps_onto`/`validates`/`mirrors`/`parallels`/`concerns`).
- **5 candidates considered and rejected** with sound reasoning (followed body content over title matches) — discriminating, not padding.
- **Linking-instinct finding:** high-quality when *directed*; but Phase 1 already showed it is **not spontaneous** (the Phase-1 agent read scope narrowly and never searched neighbors). So: cross-map linking is a *capability*, not an *instinct* — it needs explicit direction.

**Inert-edge probe** — after `cogmap_materialize` (Map 3: 12 regions/66 events; Map 2: 32 regions/108 events; both succeeded, no error on the cross-map edges):
- 16 cross-map edges (source ∈ Map 3, target ∈ Map 2, homed in Map 3).
- Map-2 targets that became **Map-3 region members: 0** → the edges are structurally **inert** at Map 3's region/salience layer.
- Those same targets that are **Map-2 region members: 12** → the value lives in Map 2, unreachable from Map 3 via the edge.
- **Conclusion:** an agent can author high-quality cross-map edges; materialize accepts them; they contribute nothing to regions/salience on either side. This is the D4 gap, measured.

**Wayfind demonstration** (`temper search --wayfind`, client-side query embed) — the one real cross-map path:
- Region salience is healthy: Map 3 regions `telos_alignment` 0.84–0.90, `salience` up to 2.29 (strong charter alignment).
- Default `--regions 3` returned `[]` (too narrow across 4 visible maps — the top-3 pooled regions were dominated by other maps / didn't clear for the query). **Tuning note / minor finding.**
- `--regions 20`, narrative query → pooled **Map 2 + Map 3 together, ranked by relevance**: Map 2 "Narrative Gravity: Scenes as Gravitational Bodies" adjacent to Map 3 "Narrative gravity as a runtime-recomputed field" (same concept, two teloi), plus Map 3 gap-register/turn-cadence interleaved with Map 2 design nodes.
- `--regions 20`, temper query → Map 3 telos adjacent to L0 "What Temper Is". Cross-map, confirmed.
- **Conclusion:** cross-map value is realized at *query time via wayfind* (query relevance + each region's own-telos salience), never via cross-map edges or a cross-map telos. The `lens` is single-map by construction; wayfind pools lens-produced regions across the visible-map union.

## Phase 3 findings (Task 7)

- **Invocation open-item resolved:** the "extra" invocation per map is `admin_genesis` (genesis auto-opens one). Map 2 = genesis + 1 authoring; Map 3 = genesis + authoring + linking. Accountability chain intact, no anomaly.
- **D3 findings resource:** `019f38ff-024a-7002-8a16-95cff1d65184` (research, `@me/temper`).
- **019f373f fed:** experiment-input section appended (correction + teachability rubric + cross-map guidance + bug pointers + worked-example note).

### Deliverables
- **D1** Map 2 live + materialized (37 nodes, 32 regions).
- **D2** Map 3 live + materialized (13 nodes, 12 regions).
- **D3** findings research `019f38ff-024a`.
- **D4** cross-map gap-spec (below).
- Bug tasks **B1** `019f38f4-3dec`, **B2** `019f38f4-506f`; follow-on capability task `019f37fb` (democratize cogmap creation).

### D4 — cross-map gap-spec (stated, not built)

**Finding.** Cross-map value in temper today is realized **only at query time via wayfind** (visibility-union pooling of each map's lens-produced regions, ranked by query relevance + own-telos salience). It is **never** realized at authoring time:
- **Cross-map edges are inert** — asserted, materialize-accepted, but contribute nothing to regions/salience/wayfind (measured: 16 edges → 0 region contribution).
- **No cross-map telos projection** — `telos_alignment` is hard-scoped to a region's own map (`canonical_functions.sql:461-471`); the `lens` is single-map by construction.

**The open design question:** should cross-map *edges* (or a cross-map *telos projection*) ever carry value — e.g. a Map-3 fit-node's `analyzes`→Map-2 link surfacing Map 2 context when reading Map 3 — or is **wayfind-pooling the intended and sufficient** cross-map mechanism, with cross-map edges kept as pure graph-level annotation? Deferred; no implementation. If pursued, the first sub-question is what a cross-map edge *should do* at materialize time (today its off-map target simply isn't a region member).

**Minor tuning note.** Wayfind default `--regions 3` is too narrow to surface across several visible maps (returned empty until `--regions 20`). Worth revisiting the default / making cross-map recall more robust.

### (a) verdict — CONFIRMED. The prize is banked: a distinct telos demonstrably causes an agent to capture distinct, purpose-relevant concepts from identical sources.
