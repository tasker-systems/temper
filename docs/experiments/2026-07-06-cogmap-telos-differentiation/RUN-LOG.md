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

_pending._

## Phase 3 findings (Task 7)

_pending._
