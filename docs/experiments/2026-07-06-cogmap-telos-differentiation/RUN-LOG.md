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

_pending._

## Phase 2 cross-map (Task 6)

_pending._

## Phase 3 findings (Task 7)

_pending._
