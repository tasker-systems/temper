# Cognitive-Map Telos-Differentiation Experiment — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to run this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. This is an **operational runbook**, not a code build — each task ends in a concrete verification gate (the operational analog of a passing test), not a unit test.

**Goal:** Stand up two new non-foundational cognitive maps over the storyteller corpus, authored by fresh charter-only agents, to test whether a distinct telos drives distinct concept-capture (prize `(a)`) and to produce a gap-spec + live demonstration for cross-map value (`(b)`).

**Architecture:** Approach 2 from the spec — author held constant ("a fresh agent following the guidance"), telos varied. You (admin) perform genesis against production temperkb.io; I (orchestrator) dispatch two controlled subagents that author via the interactive authored-4 (`create_resource`/`assert_relationship`/`facet_set`/`fold_relationship` + `invocation_open`/`close`); then I observe (materialize, region-metrics, wayfind) and write findings. The whole run also probes whether an agent can author a map from guidance alone — a live input to the parent teaching goal.

**Tech Stack:** `temper` CLI (team + cogmap subcommands), temperkb.io MCP tools, `neonctl` + `psql` for production reads, ONNX client-side embedding (already linked in the CLI).

**Spec:** `docs/superpowers/specs/2026-07-06-cogmap-telos-differentiation-experiment-design.md` (read §2 grounding findings and §11 resolved IDs before starting).

## Global Constraints

- **One home per resource → nodes are distilled, never re-homed.** Every cogmap node is a NEW resource created *into the cogmap* via `create_resource`, carrying a `derived_from` edge + `sources` back to its source context resource. Never move a storyteller context resource into a map.
- **Act envelope on EVERY authored-4 call** — `invocation_id` (from `invocation_open`) + `confidence` (`tentative`/`probable`/`confident`) + one-line `reasoning`. An act missing it is orphaned. (map-stewardship §Authorship.)
- **Provenance trio in `managed_meta` on every `create_resource`** — `temper-provenance: "llm-discovered"`, `temper-llm-model: "<model>"`, `temper-llm-run: "<invocation_id>"`.
- **Edges carry the rich layer** — `label` + `polarity` + graded `weight` (never constant 1.0), not just a bare `edge_kind`.
- **Isolation is load-bearing** — the new maps bind to the `cogmap-experiment` team ONLY; the steward M2M agent (`agent-y23aq…`) is never added to it. Task 4 gates on the steward's candidate set excluding both maps.
- **Withhold the hypothesis** — fresh-agent prompts contain the charter + mechanics + source slice and NOTHING about differentiation, the other map, or what we hope to see.
- **Resolved IDs** (spec §11): project `crimson-fog-23541670`, admin/author `@j-cole-taylor` = `019d4add-f49d-7c43-a87d-dda470e5dd9c`, steward principal `agent-y23aqxuvzjysb5n8laueuigixoftcwyu`, steward map `019f2391-e001-7933-b88a-28fb92e56ac1`, L0 `00000000-0000-0000-0005-000000000001`.

## File Structure

Experiment artifacts (committed for reproducibility) live under one directory:

- `docs/experiments/2026-07-06-cogmap-telos-differentiation/map2-storyteller-design.manifest.yaml` — Map 2 genesis manifest (charter).
- `docs/experiments/2026-07-06-cogmap-telos-differentiation/map3-cogmaps-for-storyteller.manifest.yaml` — Map 3 genesis manifest (charter).
- `docs/experiments/2026-07-06-cogmap-telos-differentiation/RUN-LOG.md` — the live capture: produced ids, per-phase observations, verification outputs. This is the running record the tasks append to.

Production state (maps, nodes, edges, teams) lives in temperkb.io, not the repo.

---

### Task 0: Pre-flight & baseline capture

**Files:** Create `docs/experiments/2026-07-06-cogmap-telos-differentiation/RUN-LOG.md`
**Executor:** Orchestrator (me)
**Interfaces:**
- Produces: `CS` (psql connection string), a captured `steward_candidate_baseline` set that Task 4 diffs against.

- [ ] **Step 1: Confirm branch + clean tree**

Run: `cd /Users/petetaylor/projects/tasker-systems/temper && git branch --show-current && git status --porcelain`
Expected: `jct/cogmap-telos-differentiation-sop`, clean (or only untracked experiment files).

- [ ] **Step 2: Confirm no `cogmap-experiment` team exists yet**

Run: `temper team list --format json | grep -i cogmap-experiment || echo "absent (good)"`
Expected: `absent (good)`.

- [ ] **Step 3: Capture the steward's current drift-sweep candidate set (baseline)**

```bash
CS=$(neonctl connection-string main --project-id crimson-fog-23541670 --database-name neondb --role-name neondb_owner)
psql "$CS" -c "SELECT cogmap_id FROM steward_candidate_cogmaps((SELECT id FROM kb_profiles WHERE handle LIKE 'agent-y23aq%')) ORDER BY 1;"
```
Expected: a small set NOT containing the (not-yet-created) new maps. Record it verbatim in RUN-LOG.md — Task 4 asserts the new maps never join it.

- [ ] **Step 4: Confirm the 8 source-slice resources are readable**

Run: `for r in narrative-graph-019d5042-5531-7a90-b939-10cbcb717913 character-modeling-019d5042-f8e0-72a2-a390-c687cbfd30b4 scene-model-019d5044-012f-7513-a940-abc93c778035 world-design-019d5042-2acc-7ea3-adcf-edaf06565550 emotional-model-019d5042-2044-70f1-9c4b-f175e7a318dc data-driven-narrative-elicitation-019d5042-0e74-7262-8bdb-78a96d33452d system-architecture-019d5042-ea0d-7b62-b332-ea63768b7c0b design-philosophy-019d5042-eeea-7661-b846-73754c216696; do temper resource show "$r" --meta-only --fields id >/dev/null 2>&1 && echo "ok $r" || echo "FAIL $r"; done`
Expected: 8 × `ok`.

- [ ] **Step 5: Create RUN-LOG.md and commit the experiment dir scaffold**

Write `docs/experiments/2026-07-06-cogmap-telos-differentiation/RUN-LOG.md` with headings: `## Baseline (Task 0)`, `## Genesis IDs (Tasks 1–4)`, `## Phase 1 authoring (Task 5)`, `## Phase 2 cross-map (Task 6)`, `## Phase 3 findings (Task 7)`. Paste the Step-3 baseline under Baseline.

```bash
git add docs/experiments/2026-07-06-cogmap-telos-differentiation/RUN-LOG.md
git commit -m "chore(experiment): run-log scaffold + steward candidate baseline"
```

---

### Task 1: Create the isolated experiment team

**Files:** none (production)
**Executor:** You (admin/owner)
**Interfaces:**
- Produces: `EXPERIMENT_TEAM_ID` (uuid) — consumed by Task 4 binding.

- [ ] **Step 1: Create the team (you become owner)**

Run: `temper team create --slug cogmap-experiment --name "Cognitive-Map Experiment"`
Expected: JSON with the new team `id` and you as `owner`. (If `--slug`/`--name` differ, check `temper team create --help`.)

- [ ] **Step 2: Verify roster is exactly you, steward absent**

Run: `temper team show cogmap-experiment`
Expected: one member — `j-cole-taylor` (owner). **`agent-y23aq…` MUST NOT appear.** Record `EXPERIMENT_TEAM_ID` in RUN-LOG.md.

- [ ] **Step 3: Append the team id to RUN-LOG.md and commit**

```bash
git add docs/experiments/2026-07-06-cogmap-telos-differentiation/RUN-LOG.md
git commit -m "chore(experiment): record cogmap-experiment team id"
```

---

### Task 2: Author the two genesis manifests

**Files:** Create both `*.manifest.yaml` under the experiment dir.
**Executor:** Orchestrator (me)
**Interfaces:**
- Produces: two manifest files consumed by Task 3 `temper cogmap create --manifest`.
- Schema (from `genesis.rs`): `name`, `telos_title`, optional `cogmap_id`/`telos_resource_id`, and `telos: { statement, questions: [{question, context}], framing: [str] }`.

- [ ] **Step 1: Write Map 2 manifest** (`map2-storyteller-design.manifest.yaml`)

```yaml
name: "Storyteller System Design"
telos_title: "Storyteller System Design — telos"
telos:
  statement: >-
    Understand how to design a system that models rich narrative possibility — one that
    traces the relational, thematic, genre, and plot-beat lifecycles of a story as
    information-rich, evolving structures, so that agentic and generative narrative can
    unfold while the world and its characters stay grounded and coherent. Nodes are
    distilled from the storyteller project's own resources and judged for their bearing on
    that design purpose — never universally.
  questions:
    - question: "What must the system represent to hold a narrative world coherent?"
      context: "entities, relations, canon, and state — the substrate a story rests on."
    - question: "How are narrative lifecycles — thematic, genre, plot-beat, relational — traced as they evolve?"
      context: "the moving parts, and how their change over time is captured."
    - question: "Where does generative / agentic unfolding get its freedom, and what keeps it grounded?"
      context: "the tension between open possibility and a coherent, consistent world."
    - question: "What has the project decided about its model, and what is still open?"
      context: "settled design choices vs live questions."
    - question: "What tension sits between rich possibility and coherent grounding?"
      context: "the central design pressure the system must hold."
  framing:
    - "Nodes carry a derived_from edge to their source(s); regions emerge only from materialize."
    - "Labels are expressive: concept, fact, decision, theme, question, concern, principle, commitment, domain."
```

- [ ] **Step 2: Write Map 3 manifest** (`map3-cogmaps-for-storyteller.manifest.yaml`)

```yaml
name: "Cognitive Maps for Storyteller"
telos_title: "Cognitive Maps for Storyteller — telos"
telos:
  statement: >-
    Understand where and how temper's cognitive maps could manage the information the
    storyteller system works with — the knowledge held by its agent-personas (storykeeper,
    narrator, world agent, and others) and the living state of its characters and plot
    beats. This map sits at the intersection of temper's cognitive-map tooling and
    storyteller's narrative needs; nodes name the design concepts of that fit, judged for
    their bearing on it — never universally.
  questions:
    - question: "What information must each storyteller agent-persona hold, and what shape would a cogmap give it?"
      context: "storykeeper, narrator, world agent — each persona's knowledge, mapped."
    - question: "What would the telos of a character map, a world map, or a plot-beat map be?"
      context: "what makes a concept salient in each — the differentiating purpose."
    - question: "How would temper's authored-4 / provenance / region model map onto narrative state that changes constantly?"
      context: "fitting a distillation-and-supersession model to fast-moving story state."
    - question: "Where do temper's cogmap primitives fit the narrative domain cleanly, and where do they strain?"
      context: "the honest fit-and-friction assessment."
    - question: "What must temper's cogmap model gain for narrative information management to be viable?"
      context: "the gap register — what is missing today."
  framing:
    - "Nodes carry a derived_from edge to their source(s); regions emerge only from materialize."
    - "Where a source concept is already a node in a visible neighboring map, link to it (a cross-map reference) rather than re-distilling it."
```

- [ ] **Step 3: Verify both manifests parse as YAML**

Run: `python3 -c "import yaml,sys; [yaml.safe_load(open(f)) for f in sys.argv[1:]]; print('both parse')" docs/experiments/2026-07-06-cogmap-telos-differentiation/map2-storyteller-design.manifest.yaml docs/experiments/2026-07-06-cogmap-telos-differentiation/map3-cogmaps-for-storyteller.manifest.yaml`
Expected: `both parse`.

- [ ] **Step 4: Commit the manifests**

```bash
git add docs/experiments/2026-07-06-cogmap-telos-differentiation/*.manifest.yaml
git commit -m "feat(experiment): genesis manifests for Map 2 + Map 3 charters"
```

---

### Task 3: Genesis both maps from the manifests

**Files:** none (production writes)
**Executor:** You (admin — `cogmap create` is admin-gated)
**Interfaces:**
- Consumes: the two manifest files from Task 2.
- Produces: `MAP2_ID`, `MAP2_TELOS_ID`, `MAP3_ID`, `MAP3_TELOS_ID` — consumed by Tasks 4–7.

- [ ] **Step 1: Genesis Map 2**

Run: `temper cogmap create --manifest docs/experiments/2026-07-06-cogmap-telos-differentiation/map2-storyteller-design.manifest.yaml`
Expected: JSON returning the minted `cogmap_id` + `telos_resource_id`. Record as `MAP2_ID` / `MAP2_TELOS_ID`.

- [ ] **Step 2: Genesis Map 3**

Run: `temper cogmap create --manifest docs/experiments/2026-07-06-cogmap-telos-differentiation/map3-cogmaps-for-storyteller.manifest.yaml`
Expected: JSON with `MAP3_ID` / `MAP3_TELOS_ID`. Record both.

- [ ] **Step 3: Verify each charter delivered (statement + 5 questions + framing)**

Run: `temper cogmap analytics <MAP2_ID>` then the MCP `cogmap_read_charter` on each map.
Expected: Map 2 charter reads back with 1 statement + 5 questions + 2 framing blocks; Map 3 likewise. Record confirmation in RUN-LOG.md (Genesis IDs section) and commit the log.

---

### Task 4: Bind + grant + **isolation gate**

**Files:** none (production)
**Executor:** You (admin — `cogmap_bind`/`cogmap_grant` are admin-gated)
**Interfaces:**
- Consumes: `EXPERIMENT_TEAM_ID`, `MAP2_ID`, `MAP3_ID`.
- Produces: authoring capability for `@j-cole-taylor`; the isolation guarantee.

- [ ] **Step 1: Bind both maps to the experiment team ONLY**

Use MCP `cogmap_bind` with `cogmap=<MAP2_ID>`, `team_id=<EXPERIMENT_TEAM_ID>`; repeat for Map 3.
Expected: `bound: true` for each. Do NOT bind to `personal-j-cole-taylor` or `temper-system`.

- [ ] **Step 2: Grant write on both maps to the authoring principal**

MCP `cogmap_grant` with `cogmap=<MAP2_ID>`, `to_profile=019d4add-f49d-7c43-a87d-dda470e5dd9c`, `write=true`; repeat for Map 3. (Read is implied by write.)
Expected: grant confirmed for each.

- [ ] **Step 3: ISOLATION GATE — steward candidate set still excludes both maps**

```bash
CS=$(neonctl connection-string main --project-id crimson-fog-23541670 --database-name neondb --role-name neondb_owner)
psql "$CS" -c "SELECT cogmap_id FROM steward_candidate_cogmaps((SELECT id FROM kb_profiles WHERE handle LIKE 'agent-y23aq%')) ORDER BY 1;"
```
Expected: the SAME set as the Task-0 baseline — **neither `MAP2_ID` nor `MAP3_ID` present**. If either appears, STOP: the steward would author into the map. Diagnose (accidental team add / extra binding) before proceeding.

- [ ] **Step 4: Confirm the author can read+write; record and commit**

Run: `temper resource list --context <MAP2_ID as cogmap> ...` sanity (or MCP `get_resource` on the telos) as `@j-cole-taylor`; append the isolation-gate output + grant confirmations to RUN-LOG.md and commit.

---

### Task 5: Phase 1 — parallel differentiation authoring (two fresh agents)

**Files:** none (production writes by subagents)
**Executor:** Orchestrator (me) dispatches; two fresh subagents author.
**Interfaces:**
- Consumes: `MAP2_ID`, `MAP3_ID`, the 8 source-slice refs, the map-stewardship mechanics.
- Produces: distilled nodes + edges + facets in each map, correlated to one closed invocation each.

- [ ] **Step 1: Dispatch Agent-2 and Agent-3 concurrently (single message, two Agent calls)**

Each prompt contains, verbatim: (i) its map id, (ii) its charter (the manifest `telos`), (iii) the 8 source-slice refs, (iv) the adapted authoring loop below, (v) the Global Constraints (act envelope, provenance trio, edge rich-layer, distilled-node rule). **Withheld:** the hypothesis, the other map, any differentiation framing.

Adapted authoring loop (fresh-map variant of map-stewardship — no steward watermark):
```
inv = invocation_open(cogmap=<MAP_ID>, trigger="scheduled")
charter already known (given in prompt); read it back with cogmap_read_charter to orient
for source in the 8 slice refs:
  read the source (resource show)
  distill the concepts that are SALIENT UNDER THIS CHARTER (not everything)
  for each node:
    create_resource(cogmap=<MAP_ID>, type=<label>, sources=[source id(s)],
        managed_meta={temper-provenance:"llm-discovered", temper-llm-model:"<model>", temper-llm-run:inv.id},
        act={invocation_id:inv.id, confidence:<band>, reasoning:"<why this node under this telos>"})
    assert_relationship(node -> source, edge_kind="express", polarity="forward", label="derived_from", weight~1.0, act)
  assert inter-node edges (relates_to/answers/supports/contradicts/part_of) with label+polarity+graded weight + act
  facet_set semantic properties where warranted (+ act)
invocation_close(inv, outcome="<N nodes / E edges / F facets>")
```
Map 3 only: its charter permits linking to a node already present in a *visible neighboring map*; in Phase 1 that means the pre-existing steward/L0 maps (Map 2 is being authored concurrently and is not a target here).

- [ ] **Step 2: Verify each agent authored + closed cleanly**

For each map: MCP `invocation_list(cogmap)` shows one closed invocation; `invocation_show` lists its acts; `list_resources`/`search` on the map shows the new nodes. Spot-check 3 nodes per map: each has the provenance trio in `managed_meta`, a `derived_from` edge, and its acts carry `invocation_id`.
Expected: both maps populated; no orphaned acts.

- [ ] **Step 3: Capture the raw authoring transcripts + node inventory into RUN-LOG.md**

Record, per map: node count, labels used, and the node(s) distilled from EACH of the 8 shared sources (this is the raw material for Task 7's differentiation read). Note qualitatively whether each agent needed help / floundered (teachability signal). Commit the log.

---

### Task 6: Phase 2 — linking instinct + cross-map probe + wayfind demo

**Files:** none (production)
**Executor:** Orchestrator (me) + one fresh subagent
**Interfaces:**
- Consumes: both populated maps.
- Produces: a linking-instinct observation, the inert/dangling-edge evidence, and a wayfind cross-map demonstration (feeds D4).

- [ ] **Step 1: Dispatch a Map-3 linking pass (Map 2 now visible)**

Fresh subagent, told only: Map 2 (`Storyteller System Design`) now exists and is visible; re-read your own Map 3 nodes and Map 2's nodes; where a Map-3 node would restate a concept that is *already a Map-2 node*, assert a cross-map reference edge to it (`relates_to`/`answers`/`supports` as fits) with the act envelope, rather than duplicating. Withhold the hypothesis.
Expected: record whether the agent spontaneously reaches for Map-2 links, and how many.

- [ ] **Step 2: Orchestrator asserts one deliberate cross-map edge + observe dangling behavior**

Assert one `Map3-node → Map2-node` edge (homes in Map 3). Then MCP `cogmap_materialize` Map 3.
```bash
CS=$(neonctl connection-string main --project-id crimson-fog-23541670 --database-name neondb --role-name neondb_owner)
# after materialize, inspect whether the cross-map edge's off-map target dangles
psql "$CS" -c "SELECT e.source_id, e.target_id, e.home_anchor_id FROM kb_edges e WHERE e.home_anchor_id = '<MAP3_ID>' AND e.target_id NOT IN (SELECT resource_id FROM kb_cogmap_region_members m JOIN kb_cogmap_regions r ON r.id=m.region_id WHERE r.cogmap_id='<MAP3_ID>');"
```
Expected: the cross-map edge is present but its target contributes to no Map-3 region — the inert/dangling case. Record verbatim (D4 evidence).

- [ ] **Step 3: Wayfind demonstration across the visible-map union**

As `@j-cole-taylor`, run a wayfind search (MCP `search` with `wayfind:true`, no cogmap arg) with a narrative-information query (e.g. "how should character and plot-beat state be represented and managed").
Expected: results pool regions from Map 2, Map 3, the steward map, and L0 — the real cross-map path. Capture which maps contributed. Record in RUN-LOG.md.

---

### Task 7: Phase 3 — observe, analyze, write findings

**Files:** Create D3 (a temper `research` resource) + D4 (gap-spec section, appended to RUN-LOG.md or its own doc).
**Executor:** Orchestrator (me); you review at the checkpoint.
**Interfaces:**
- Consumes: everything from Tasks 5–6.
- Produces: **D3** differentiation + teachability findings, **D4** cross-map gap-spec, and a feedback note into task `019f373f`.

- [ ] **Step 1: Materialize both maps and pull region metrics**

MCP `cogmap_materialize` Map 2 and Map 3; then `cogmap_region_metrics` + `cogmap_analytics` on each. Record region counts, per-region `telos_alignment`/salience.
Expected: both maps produce coherent regions; each region's telos-salience is against its OWN charter.

- [ ] **Step 2: Differentiation read over the shared slice (the (a) result)**

For each of the 8 shared sources, place Map-2's node(s) beside Map-3's node(s). Assess: are they distinct AND each telos-coherent? Run the **blind-read test**: from each map's node set alone, could a reader infer its telos? Tabulate. State the verdict honestly — including a null result (substantial overlap regardless of telos) if that is what the data shows. Note the two-agents confound.

- [ ] **Step 3: Write D3 — findings resource**

```bash
cat <<'EOF' | temper resource create --type research --title "Cogmap telos-differentiation experiment — findings" --context @me/temper
# Cogmap telos-differentiation experiment — findings
## (a) Telos-differentiation result
<blind-read table + verdict + confound note>
## Teachability signal (feeds 019f373f)
<did charter-only agents author successfully; where they needed help; the "distilled node ≠ same row" correction>
## Method + links
<link spec + RUN-LOG; maps MAP2_ID / MAP3_ID>
EOF
```
Expected: research resource created in `@me/temper`; record its ref.

- [ ] **Step 4: Write D4 — cross-map gap-spec**

Append a `## D4 — cross-map gap-spec` section to RUN-LOG.md (or a sibling doc): "cross-map value today = wayfind pooling (query + visibility); cross-map edges + cross-map telos = inert/gap," the observed dangling-edge evidence (Task 6.2), the wayfind demonstration (Task 6.3), and the single open design question (should edges/telos ever carry cross-map value, or is wayfind sufficient?). No implementation.

- [ ] **Step 5: Feed the parent teaching task**

Append the teachability findings + the "nodes are distilled, not re-homed" correction as a note on task `019f373f` (via `temper resource update <ref>` body append or a linked session). Commit RUN-LOG.md + D4.

- [ ] **Step 6: Review checkpoint**

Present to you: the differentiation verdict, the teachability signal, the D4 gap-spec, and the two live maps. Decide together whether `(a)` is answered, whether a second authoring pass is warranted (if the signal was ambiguous), and what lands back in the temper skill (`019f373f`).

---

## Self-Review

**Spec coverage:** §1 hypothesis → Tasks 5–7. §2 grounding (incl. §2.5 steward) → Task 4 gate. §3 method/D-slice/D-isolation → Tasks 2/5. §4 charters → Task 2. §5 roles → per-task Executor. §6 runbook Phases 0–3 → Tasks 0–7. §7 success criteria → Task 7.2/7.6. §8 risks (confound, same-principal) → Task 7.2 note. §9 out-of-scope → honored (no cross-map build; no democratize; no skill rewrite). §10 deliverables D1/D2 → Tasks 3/5; D3 → 7.3; D4 → 7.4. §11 IDs → Global Constraints. **Covered.**

**Placeholder scan:** `<MAP2_ID>` etc. are runtime-produced ids recorded in RUN-LOG, not plan placeholders. Charters, manifests, commands, and the authoring loop are concrete.

**Consistency:** Map/telos id names (`MAP2_ID`/`MAP2_TELOS_ID`/`MAP3_ID`/`MAP3_TELOS_ID`), the experiment team (`cogmap-experiment`), and the source-slice refs are used identically across tasks.
