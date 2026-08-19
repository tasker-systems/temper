# Steward Act-Model & Cogmap-Resource Vocabulary

**Date:** 2026-06-30
**Status:** Design — approved in brainstorming, pending plan
**Goal:** `team-self-cognition-steward-agent-eve-mvp`
**Task:** T3 (keystone design; T1 tool shapes and T5 agent instructions inherit from it)
**Workstream:** WS7 (Agent surface) under `substrate-kernel-to-cognitive-map`

> This is the keystone spec for the steward agent MVP. It pins the decisions that the
> build tasks depend on: the act-model (what the steward reads and produces), the
> cogmap-resource label vocabulary, the provenance model, the edge model, and the
> recurring-run semantics. T1 (MCP+CLI+API parity) and T5 (Eve agent dir) fall out of it.

## Context

The goal reframes the WS7 "triage / map-stewardship" workflow into something concrete and
dogfooding: not a steward *persona* in the abstract, but a **team self-cognition map**. A team's
own temper resources become the ingest source for a cogmap that is **1:1 with the team**, born from
a templatized team-telos-charter ("understand how this team works — what they work on, their most
active projects, the problems they solve"), and tended by a Vercel/Eve steward on a cron-threshold
cadence. temper-workflows becomes the native dogfooding source for cognitive maps.

This spec builds on settled prior work — the determinism reframe (agents tend declared structure,
never cluster; region formation is the substrate's pure function on `materialize`), the content-block
primitive (`kb_content_blocks` + `kb_block_provenance`), the invocation envelope (live MCP tools),
and the `temper-agents` neutral contract (PR #151). Prior ideation pulled in: `Agentic workflows on
temper via Vercel Eve`, `Vercel Eve & Claude Managed Agents — investigation`, `temper-next
deliverable 3` (questions-with-context topology), `Data-model reconciliation` (doctype-as-opaque-facet),
`Content-block primitive: addressable resource interiority + per-block provenance`.

## Scope

**In (MVP):** one team (the temper team itself); the steward distills the team's resources into a
single team self-cogmap; runs the authored-4 (create / assert / facet / fold); audited via the
invocation envelope; deployed as a real Vercel cron.

**Deferred (named, not built):** block-level provenance accretion (T7 fast-follow); remote ingest
sources (Linear/GitHub); cross-map promotion-translation + its HITL gate; auto-birth-of-self-cogmap-
per-team (MVP uses on-demand templated genesis, T2).

## Decisions

### D1 — Purpose & scope: single-map steward, determinism-respecting

The steward reads a team's resources and tends **one** cogmap (the team's self-cognition map) using
the authored-4. It **never clusters or assigns salience** — region formation stays the substrate's
pure function on `materialize` (driven on its own threshold cadence by T4b). Regions will tend to
share shape with contexts, but *emergently*, from materialize over the declared structure — never
drawn by the agent.

### D2 — Read-scope: access-bounded, bodies, watermark-gated

The steward reads source resources' **full bodies** (it needs content to distill, not just meta),
across all of the team's contexts. Reads are **access-bounded by the substrate floor**
(`resources_visible_to` / producer-intersection) — the steward physically cannot reach beyond the
team intersection; a runtime bug cannot breach leak-safety. Each run is **watermark-gated** to
resources new-or-changed since the last run (the delta capability is T4a).

### D3 — Node vocabulary: extend `DocType`, one registry, open tail

Cogmap-homed resources carry an expressive label, **not** constrained to workflow frames
(task/goal/session). At the kernel the label is just the opaque `doc_type` property
(`kb_properties key='doc_type'`); the *recognized* set is a Domain-A interpretation.

**Mechanism:** exhaustive-match a recognized enum; on no match, **pass the raw label string through
as-is** (closed-set-with-open-tail). CLI + API + MCP all tolerant. `DocType::from_str` already has
graceful no-match fallbacks; this generalizes them.

**Home:** extend the **existing** `temper-workflow` `DocType` enum (single source of truth for the
`doc_type` property's recognized values), rather than forking a parallel cogmap enum — avoids two
competing definitions of `concept`/`decision`. (`EdgeKind`/`Polarity` live in temper-core; the
node-label enum may warrant relocation as the Domain-A/B split matures — noted, not done now.)

**Recognized seed set** (`concept` + `decision` already exist; the rest are additions):

| Label | Granularity | Gloss |
|-------|-------------|-------|
| `fact` | per-source | An observation distilled from a resource ("the team uses pgvector") |
| `memory` | per-source | A regulation / lesson carried forward ("always run test-e2e-embed before context pushes"); often scar-linked |
| `decision` *(exists)* | per-source | A settled choice |
| `concept` *(exists)* | synthesized | A distilled idea spanning sources |
| `question` | synthesized | An open question-with-context ("how should access RBAC work?") |
| `theme` | synthesized | A higher-order cluster — "what they work on" |
| `concern` | synthesized | A live tension / risk the team holds |
| `principle` | synthesized | A guiding tenet the team operates by |
| `commitment` | synthesized | Something the team has committed to / owes |
| `domain` | synthesized | An area of expertise / responsibility the team owns |

`project` is deliberately excluded (overloaded; `theme` carries it). `concern` vs `question` and
`concept` vs `theme` have fuzzy borders — the steward's instructions (T5) must gloss each crisply so
label choice stays consistent. The open tail keeps seeding-rich low-risk.

### D4 — Provenance: resource-level `derived_from` edge (block-level deferred)

A cogmap node is a distinct resource homed in the cogmap, linked to its source resource(s) by a
typed **`derived_from`** edge (node → source). Sources stay in their contexts; nodes live in the
cogmap; edges cross the boundary. Provenance is a first-class graph citizen — traversable,
reweightable, **foldable** when a source is superseded.

**Block-level provenance** (`kb_block_provenance`, which answers "where did each addressable block of
this node come from, in what order") is the **designated next-phase home** (T7) — schema-ready
(table + read joins exist) but its write path is stubbed (`events.rs:720`
`incorporated: Vec::new() // provenance accretion deferred`). MVP nodes are block-bearing resources,
so they slot into accretion later with no redesign. **Do not build redundant attribution tooling.**

### D5 — Granularity: label-determined

Granularity follows the label (D3 table). **Per-source** labels (`fact`/`memory`/`decision`) cite
~1 source — one `derived_from` edge. **Synthesized** labels (`concept`/`question`/`theme`/`concern`/
`principle`/`commitment`/`domain`) span many sources — many `derived_from` edges to one node. This
is the most agent-judgment-heavy axis; the steward's instructions carry the rules.

### D6 — Edge model: no new schema; 4 EdgeKinds + semantic label

Inter-node relationships reuse the existing edge model — **no new enum**:
`AssertRelationship { edge_kind: EdgeKind, polarity: Polarity, label: String, weight: f64 }`.

`EdgeKind` is the small **closed structural taxonomy** the affinity/region math understands —
`Express`, `Contains`, `LeadsTo`, `Near`. The **semantic** relationship name rides the free-text
`label` (with `polarity` Forward/Inverse and `weight`). Steward instructions (T5) define the
kind+label conventions, e.g.:

| Semantic label | EdgeKind | Polarity | Notes |
|----------------|----------|----------|-------|
| `derived_from` | `Express` or `LeadsTo` | per the arrow | the provenance edge (D4) |
| `relates_to` | `Near` | Forward | symmetric affinity |
| `part_of` | `Contains` | Inverse | whole–part |
| `answers` | `LeadsTo` | Forward | a fact/concept answers a question |
| `supports` / `contradicts` | `LeadsTo` | Forward / Inverse | stance |

The exact kind for `derived_from` is a T5 convention detail (likely `Express`: the source *expresses*
the node); the structural kind carries affinity, the label carries human meaning.

### D7 — Re-run model: accretive + fold-on-supersede

The steward runs on a cron over a growing, changing resource set. Each tick:

1. **Watermark delta (T4a):** the set of sources new-or-changed since the last run.
2. **Search before create:** query the cogmap (semantic + `derived_from` traversal) for an existing
   node already covering this source/idea — dedup. Identity-as-input (WS3) lets the steward
   pre-generate stable ids so re-issue across a durable park/resume is byte-exact (idempotent).
3. **Net-new source →** distill new node(s), assert `derived_from` + inter-node edges, set facets.
4. **Materially-changed source →** **fold** the stale derived node (a designed steward act) and create
   a fresh one; supersession via fold, **not** in-place block edit. Folds carry full history.

"Materially changed" is the **steward's judgment** (reading the changed source against the existing
node), framed as a heuristic in T5 — **not** mechanical substrate logic (no hash threshold).

In-place reconciliation (update node blocks via `block_mutated`, stable identity) is the richer
north-star but is **out of MVP scope** — fold-on-supersede keeps the MVP free of in-place-update
complexity and its interaction with the (deferred) block-provenance.

### D8 — Autonomy & audit: fully autonomous + audited, no HITL in MVP

Because the MVP is a **single team self-cogmap with no cross-map promotion**, the steward is **fully
autonomous + audited** — no human-in-the-loop gate. (The cross-map promotion-translation HITL gate
is a later thread, out of scope.)

- **Invocation envelope:** every run wrapped in `invocation_open` → … → `invocation_close` (live MCP
  tools). The envelope correlates the run's mutation events and records a terminal outcome
  (completed N nodes / M edges / K facets / P folds, failed, abandoned).
- **Authorship stamping:** every act carries `AgentAuthorship` — `invocation_id` (correlation),
  `confidence` (graded band: tentative/probable/confident), `reasoning`, plus provenance
  (`persona`/`model`). **Reasoning required on structural acts** (create / edge / fold); optional on
  facets. These ride the existing `create`/`assert` wire (already implemented).

## The steward loop (concrete)

```
on tick:
  delta = threshold_check(team_contexts, since=watermark)        # T4a; skip if under threshold
  inv   = invocation_open(originating_cogmap, telos_scope)       # live MCP tool
  telos = read_telos_blocks(team_cogmap)                         # T1 (new read tool) — orient
  for source in delta.new_or_changed:
    existing = search(team_cogmap, source)                       # dedup (D7.2)
    if source is materially changed and existing:
      fold_relationship(existing.derived_from)                   # supersede (D7.4)
      existing = none
    if not existing:
      node = create_resource(--cogmap team_cogmap, --type <label>,  # D3 label
                             authorship=stamp(inv, confidence, reasoning))   # D8
      assert_relationship(node -> source, label="derived_from", kind=Express)  # D4/D6
      for rel in inter_node_relationships(node):                 # D6
        assert_relationship(node -> other, kind, polarity, label, weight)
      for f in facets(node):
        facet_set(node, f)                                       # T1 (new tool)
  invocation_close(inv, outcome)                                 # D8
# region materialization runs on its own threshold cadence — T4b, NOT the steward
```

## Forward-compatibility & deferred (named so we don't build redundant tooling)

- **Block-level provenance (T7):** un-stub `incorporated` → `kb_block_provenance`; nodes are already
  block-bearing, so no redesign. MVP ships resource-level `derived_from` only.
- **Remote source kinds:** `provenance_source_kind` is `('event','resource')` today; a `'remote'`/
  `'external'` value lands with Linear/GitHub ingest (also deferred).
- **Cross-map promotion-translation + HITL gate:** a later thread; needs the target-telos
  adjudication a human gates.
- **In-place reconciliation (D7):** the north-star update model; revisit post-MVP.
- **Auto-birth self-cogmap per team:** MVP uses on-demand templated genesis (T2); generalization later.

## Downstream cascade (what the build tasks inherit)

- **T1 (MCP+CLI+API parity):** add a `facet_set` MCP tool (substrate/CLI/API have it; MCP doesn't) and
  a **telos/charter-block read** tool (the steward orients on its telos; `cogmap_shape` returns
  regions, not telos prose). Extend `DocType` (D3) + ensure the `--type`/doc_type wire passes the new
  labels through all three surfaces with the open-tail fallback.
- **T5 (Eve agent dir):** the steward persona/instructions encode D3 label glosses, D5 granularity
  rules, D6 edge conventions, D7 re-run + "materially changed" heuristic, D8 authorship discipline.
- **T4a/T4b:** the two threshold cadences (ingest→steward; cogmap-delta→materialize) reuse the
  `formation_touched_since` watermark shape.

## Code anchors (verified 2026-06-30)

- `DocType` enum: `crates/temper-workflow/src/frontmatter/document.rs:14`; `KNOWN_DOC_TYPES`:
  `crates/temper-workflow/src/schema.rs:293`.
- Edge model: `EdgeKind`/`Polarity` `crates/temper-core/src/types/graph.rs:33,56`;
  `AssertRelationship` `crates/temper-workflow/src/operations/commands.rs:149`.
- Provenance: `kb_block_provenance` / `kb_content_blocks`
  `migrations/20260624000001_canonical_schema.sql:603,541`; `provenance_source_kind`
  enum `:105`; deferred write `crates/temper-substrate/src/events.rs:720`;
  `Incorporation` payload `crates/temper-substrate/src/payloads.rs:446`.
- Invocation envelope MCP tools: `crates/temper-mcp/src/tools/invocations.rs`.
- Team↔cogmap: `kb_team_cogmaps` + `cogmaps_share_a_team`
  (`migrations/20260624000002_canonical_functions.sql`); L0 birth pattern
  `migrations/20260625000001_l0_kernel_cogmap.sql`.
- Watermark/drift: `formation_touched_since` `crates/temper-substrate/src/replay.rs`;
  drift tiers `crates/temper-substrate/src/drift.rs`.

## Connections

- Goal `team-self-cognition-steward-agent-eve-mvp`; tasks T1, T2, T4a, T4b, T5, T6, T7.
- Builds on PR #151 (`temper-agents`), the live invocation-envelope MCP tools, the content-block
  primitive, and the determinism reframe.
