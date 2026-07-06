# Cognitive-Map Telos-Differentiation Experiment — SoP / Design

**Date:** 2026-07-06
**Status:** Draft for review
**Frame-of-reference goal:** `019f373e` — "Teach temper to agents — the deeper model"
**Related tasks:** `019f373f` (teach the cogmap authoring workflow — this experiment feeds it); `019f37fb` (democratize cogmap creation — out of scope, surfaced here); `019f3739` (can_write precedence — settled, DONE)

> This is an **operational runbook** (SoP), not a code change. Most steps are acts performed against production temperkb.io by named roles. The one code-shaped output is a *gap-spec* (deliverable D4), which is written prose, not implementation.

---

## 1. Why now — the unproven properties

Production holds **N=2** cognitive maps and **N=1 non-system-foundational** one (the steward's team-self-cognition map `019f2391`; the other is the L0 kernel). At N=1 two load-bearing, *modeled-but-undemonstrated* properties of the multi-map design are simply **unobservable**:

- **(a) Telos-differentiation** *(the prize)* — does a distinct telos actually **cause** an agent to capture *distinct, purpose-relevant* concepts from shared source material, rather than a generic dump? Needs ≥2 maps to see divergence.
- **(b) Cross-map linking → denser relational graph** *(gap-and-spec only; not to be built)* — given visibility into a neighboring map, does an agent reach to **link existing concepts** rather than re-invent them, and does that yield value?

We move N=1 → N=3 with two new maps whose **teloi overlap in subject but differ in purpose**, chosen so they are *also genuinely useful work* (not lab artifacts): a design of how the storyteller systems and temper cognitive maps should work together.

## 2. Grounding findings (mechanics that shape the design)

Verified against the codebase (2026-07-06):

1. **One home per resource** — `kb_resource_homes.resource_id` is `UNIQUE`. A resource cannot live in two maps. Therefore a cogmap **node is a NEW, distilled resource** homed in the map, carrying a `derived_from` edge to its source context resource — *not* the source row re-homed. (This corrects the goal-doc's loose "same rows" framing; the correction is itself a finding for `019f373f`.)
2. **Cross-map edges are structurally allowed but inert** — an edge homes wherever its *source* lives; the target's map is never validated (`db_backend.rs:1185`, `writes.rs:697-766`). So `Map3-node → Map2-node` is legal and homes in Map 3. But…
3. **Region materialization + the telos-lens are single-map only** — `materialize` pulls only nodes+edges homed in the one map (`substrate.rs:32-53`); `telos_alignment` scores a region's centroid against **its own map's** charter embedding, hard-scoped `WHERE c.id = p_cogmap` (`canonical_functions.sql:461-471`). A cross-map edge's off-map target is not a member of any region, so cross-map edges contribute **nothing** to regions/salience — and produce an untested dangling-target case.
4. **Wayfind is the one real, tested cross-map path** — it takes no cogmap argument; it pools top-N regions across **every map the principal can read** (`cogmap_visible_maps`), ranks them `α·salience + β·query_cos` (α=0.4/β=0.6), unions members, re-gated by `resources_visible_to` (`wayfind_scope.sql:38-95`; `cogmap_wayfind_test.rs`). The cross-map ranking signal is the **query embedding + visibility union**, *not* a cross-map telos.
5. **The steward sweeps ALL team-joined maps it can read** — `steward_candidate_cogmaps(principal) = SELECT DISTINCT tc.cogmap_id FROM kb_team_cogmaps tc WHERE anchor_readable_by_profile(principal, …)` (`migrations/20260705000002_steward_drift_sweep.sql:11-16`). The steward M2M principal (`agent-y23aq…`) is a member of **both** of `@j-cole-taylor`'s teams (`personal-j-cole-taylor` → steward map, `temper-system` → L0). So binding the new maps to either existing team would enroll them in the steward's autonomous drift-sweep and let it author into them mid-experiment — a third, uncontrolled author. **Mitigation (§6 Phase 0): a dedicated experiment team the steward is not a member of.**

**Consequence for (b):** cross-map value **today** is realized at *query time via wayfind*, never at *authoring time via edges or a shared telos*. The gap-spec's whole question is therefore narrow: *should cross-map edges (or cross-map telos projection) ever carry value, or is wayfind-pooling the intended and sufficient mechanism?* We **demonstrate** the wayfind path as evidence; we **do not build** anything.

## 3. Method — Approach 2 (fresh-agent authoring, charter-only)

Chosen over "I author" (proves feasibility, not emergence) and the hybrid (mixed authorship confounds the (a) signal).

**Author held constant, telos varied.** Both new maps are authored by **fresh subagents** given *only* their charter + map access + the `map-stewardship` authoring mechanics + the same source slice — and **nothing about this hypothesis**. This single choice does triple duty:
- rigorous (a) test — divergence over shared source is attributable to telos, not author;
- **live teachability probe** for goal `019f373e` — can an agent author a map from guidance alone, or does it flounder? Either outcome feeds `019f373f`;
- natural (b) exercise — Map 3's intersection telos invites cross-map reach.

> **"Fresh" = controlled-information dispatch, not a separate principal.** The subagents author as the granted profile (`@j-cole-taylor`). A dedicated non-admin authoring identity would also exercise the real write-grant path (not admin-bypass) and is *more* rigorous — but needs a second auth on this machine and is **out of scope**; recorded as a rigor limitation and a natural fit for task `019f37fb`.

### Design decisions made on your behalf — review these

- **D-slice — the shared source corpus.** Eight storyteller research resources both agents distill from (rich enough for both teloi, bounded):
  `narrative_graph` · `character_modeling` · `scene-model` · `world_design` · `emotional-model` · `data_driven_narrative_elicitation` · `system_architecture` · `design_philosophy`.
- **D-isolation — clean differentiation via timing.** Phase-1 authoring runs **in parallel from empty**: while both agents work, neither map has nodes for the other to see, so the Map-2-vs-Map-3 differentiation read over the shared slice is clean. (Map 3 *may* still link to the **pre-existing** temper cogmaps — steward/L0 — in Phase 1; that's legitimate and yields incidental (b) edge evidence.) The Map-2↔Map-3 linking instinct is a **separate Phase-2 read**, once both maps are populated.
- **D-charters — draft teloi** in §4, mirroring the steward charter shape (one `statement`, five `question`s, framing rules).

## 4. The three maps

| Map | Role | Telos (one line) |
|---|---|---|
| `019f2391` steward team-self-cognition | **existing — reference/control** | "Understand how this team works…" |
| **Map 2 — Storyteller System Design** | new | design a system that models rich narrative possibility, tracing relational/thematic/genre/plot-beat lifecycles as information-rich structures, so agentic/generative narrative unfolds while world + characters stay grounded |
| **Map 3 — Cognitive Maps for Storyteller** | new | where + how temper's cognitive maps could manage the information the storyteller agent-personas, characters, and plot beats work with — the intersection of temper cogmap tooling and storyteller needs |

### Map 2 charter (draft)
- **statement:** "Understand how to design a system that models rich narrative possibility — one that traces the relational, thematic, genre, and plot-beat lifecycles of a story as information-rich, evolving structures, so agentic and generative narrative can unfold while the world and its characters stay grounded and coherent. Nodes are distilled from the storyteller project's own resources and judged for their bearing on that design purpose — never universally."
- **question 1:** What must the system represent to hold a narrative world coherent? (entities, relations, canon, state)
- **question 2:** How are narrative lifecycles — thematic, genre, plot-beat, relational — traced as they evolve?
- **question 3:** Where does generative / agentic unfolding get its freedom, and what keeps it grounded?
- **question 4:** What has the project decided about its model, and what is still open?
- **question 5:** What tension sits between rich possibility and coherent grounding?
- **framing:** nodes carry `derived_from` to their source(s); regions emerge only from `materialize`; labels are expressive (concept, fact, theme, decision, question, concern, principle…).

### Map 3 charter (draft)
- **statement:** "Understand where and how temper's cognitive maps could manage the information the storyteller system works with — the knowledge held by its agent-personas (storykeeper, narrator, world agent, and others) and the living state of its characters and plot beats. This map sits at the intersection of temper's cognitive-map tooling and storyteller's narrative needs; nodes name the design concepts of that fit, judged for their bearing on it — never universally."
- **question 1:** What information must each storyteller agent-persona hold, and what shape would a cogmap give it?
- **question 2:** What would the telos of a character map, a world map, or a plot-beat map be — what makes a concept salient there?
- **question 3:** How would temper's authored-4 / provenance / region model map onto narrative state that changes constantly?
- **question 4:** Where do temper's cogmap primitives fit the narrative domain cleanly, and where do they strain?
- **question 5:** What must temper's cogmap model gain for narrative information management to be viable? (the gap register)
- **framing:** nodes carry `derived_from`; **where a source concept is already a node in a visible neighboring map, link to it (cross-map reference) rather than re-distilling**; regions emerge only from `materialize`.

## 5. Roles

| Role | Who | Does |
|---|---|---|
| **Admin / genesis** | You (system admin, holds the privilege, wants to witness) | Phase 0: `temper team create cogmap-experiment` (owner; steward NOT added), `cogmap_create` ×2, deliver charters via `cogmap reconcile`, `cogmap_bind` to the experiment team ×2, `cogmap_grant write` ×2. |
| **Orchestrator** | Me | Draft charters/slice (this doc); dispatch the fresh agents; Phase-2 cross-map assertion + wayfind demo; Phase-3 analysis + write-up. |
| **Fresh authors** | Two dispatched subagents | Distill nodes into their assigned map (Phase 1); Map-3 linking pass (Phase 2). Charter + mechanics only, no hypothesis. |

*Each role observes the others.* (Mechanically the authed session could perform the admin acts too — this split is a deliberate choice so the human witnesses the emergent parts and the honest admin→authoring access path is exercised as themselves.)

## 6. The runbook

### Phase 0 — Genesis (You, admin)
1. `temper team create` the **dedicated experiment team** `cogmap-experiment` (name "Cognitive-Map Experiment"). You become owner. **Do NOT add the steward M2M agent** (`agent-y23aq…`) — this is the isolation that keeps the steward's drift-sweep off the new maps (grounding finding §2.5). Record the new `team_id`. *(Visibility parity still holds: `@j-cole-taylor` sees the steward map via `personal-j-cole-taylor` and L0 via `temper-system`, so Map 3 keeps its cross-map reach into temper's cogmap knowledge.)*
2. `cogmap_create` Map 2 and Map 3 (empty charters). Record `cogmap_id` + `telos_resource_id` for each.
3. Author the charter prose (§4) and deliver each via `temper cogmap reconcile` (client-side embed).
4. `cogmap_bind` both maps to the experiment team **only** (never to `personal-j-cole-taylor` or `temper-system`).
5. `cogmap_grant write` on both maps to the authoring principal (`@j-cole-taylor` profile, `019d4add-f49d-7c43-a87d-dda470e5dd9c`). Read is implied by write.
6. Verify: `cogmap_read_charter` + `cogmap_analytics` return the delivered charters; confirm the steward's candidate set does **not** include the new maps (`steward_candidate_cogmaps(agent-y23aq…)`).

### Phase 1 — Differentiation authoring (Me → two fresh agents, parallel, from empty)
7. Dispatch **Agent-2** and **Agent-3** simultaneously. Each prompt contains: its charter, its `cogmap_id`, the shared source-slice refs (§D-slice), the `map-stewardship` mechanics (authored-4: create/assert/facet/fold; invocation `open`/`close` envelope; distillation + `derived_from`; fold-then-recreate; never edit in place), and the standard subagent-guidance. **Withheld:** the hypothesis, the other map's existence/telos, any differentiation framing.
8. Each agent opens an invocation, distills nodes from the slice into its map, closes the invocation. Map 3 may link to pre-existing temper cogmaps (steward/L0); it cannot yet see Map 2 (empty/concurrent).

### Phase 2 — Linking instinct + cross-map probe (Me + one fresh agent)
9. With both maps populated, dispatch a **Map-3 linking pass** (fresh subagent) that is now told Map 2 exists and is visible: does it reach to link Map-2 concepts it would otherwise have restated? Record whether the reach is spontaneous.
10. Orchestrator deliberately asserts one `Map3-node → Map2-node` cross-map edge, then `materialize` Map 3 → observe the inert/dangling-target behavior (evidence for D4).
11. Run **wayfind** (no cogmap arg) over the principal's visible-map union with a narrative-information query → show regions pooled across Map 2, Map 3, steward, L0. This is the *demonstration* of the real cross-map path.

### Phase 3 — Observe + write up (Me; You review)
12. `materialize` both maps; pull `region-metrics` + `analytics` (per-map telos-salience).
13. **(a) differentiation read** over the shared slice: for each source doc, compare the node(s) each map distilled — are they distinct *and* each telos-coherent? Blind-read test: could a reader infer each telos from its node set alone? Note the two-agents confound explicitly.
14. **(b) gap-spec (D4):** write up "cross-map value = wayfind pooling (query + visibility); cross-map edges + cross-map telos = inert/gap," with the observed dangling behavior, and pose the single design question (should edges/telos ever carry cross-map value, or is wayfind sufficient?).
15. **Feed `019f373f`:** capture what the fresh agents needed / lacked to author from guidance alone (teachability findings), and the "distilled node ≠ same row" correction.

## 7. Success criteria

- **(a) primary:** Both maps materialize with coherent regions; over the shared slice, the two node sets are demonstrably **distinct and each telos-coherent**; the blind-read test passes. A null result (node sets substantially overlap regardless of telos) is a *valid, informative* outcome — it would falsify the differentiation premise and reshape the teaching goal.
- **teachability:** Recorded evidence of whether a charter-only agent can author a map, and where it needed help — a direct input to `019f373f`.
- **(b) secondary:** A written gap-spec + a working wayfind demonstration across all visible maps. No cross-map value code is built.

## 8. Risks & limitations

- **Authorship confound** — differentiation is read over shared source with identical prompts-modulo-charter; still, N is small. Mitigation: qualitative blind-read + honest reporting; optional second pass if the signal is ambiguous.
- **Same principal, not a separate identity** — "fresh" is informational only. A dedicated non-admin author would be more rigorous; deferred to `019f37fb`.
- **Charter quality is a variable** — a weak telos could blunt differentiation. The drafts (§4) are reviewed before genesis for this reason.
- **Dangling cross-map edge (step 10)** touches an untested materialize path — expected; it *is* the evidence. Contained to Map 3.

## 9. Out of scope

- Building any cross-map edge value, cross-map telos projection, or materialize spanning (that's the gap-spec's subject, deferred).
- Democratizing cogmap creation (`019f37fb`).
- Rewriting the temper skill's cogmap guidance (`019f373f`) — this experiment *feeds* it; it does not perform it.

## 10. Deliverables

- **D1** — Map 2 (Storyteller System Design), live + materialized.
- **D2** — Map 3 (Cognitive Maps for Storyteller), live + materialized.
- **D3** — Differentiation + teachability findings (a temper `research` resource in `@me/temper`).
- **D4** — The (b) cross-map gap-spec + wayfind demonstration record.

## 11. Resolved concrete values (for execution)

| Item | Value |
|---|---|
| Neon production project | `crimson-fog-23541670` (temper-cloud), branch `main`, db `neondb` |
| Admin / authoring profile | `@j-cole-taylor` = `019d4add-f49d-7c43-a87d-dda470e5dd9c` (system admin) |
| Steward M2M principal (to EXCLUDE from the experiment team) | `agent-y23aqxuvzjysb5n8laueuigixoftcwyu` |
| Existing steward map (reference/control) | `019f2391-e001-7933-b88a-28fb92e56ac1` — bound to `personal-j-cole-taylor` (`019eea5e-daf4-7eaa-a85a-369ab11539e4`) |
| L0 kernel map | `system-default` `00000000-0000-0000-0005-000000000001` — bound to `temper-system` (`019f04a9-e43e-7985-99a6-ca6a95139e85`) |
| Experiment team (Phase 0 step 1 creates) | slug `cogmap-experiment`, name "Cognitive-Map Experiment", owner `@j-cole-taylor`, **steward excluded** |
| Shared source slice refs (`@me/storyteller` research) | `narrative-graph-019d5042-5531-7a90-b939-10cbcb717913` · `character-modeling-019d5042-f8e0-72a2-a390-c687cbfd30b4` · `scene-model-019d5044-012f-7513-a940-abc93c778035` · `world-design-019d5042-2acc-7ea3-adcf-edaf06565550` · `emotional-model-019d5042-2044-70f1-9c4b-f175e7a318dc` · `data-driven-narrative-elicitation-019d5042-0e74-7262-8bdb-78a96d33452d` · `system-architecture-019d5042-ea0d-7b62-b332-ea63768b7c0b` · `design-philosophy-019d5042-eeea-7661-b846-73754c216696` |
