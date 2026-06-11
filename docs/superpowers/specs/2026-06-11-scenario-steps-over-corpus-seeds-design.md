# Scenario steps over the corpus seeds: growth runbooks + the `relationship_fold` mutation

**Date:** 2026-06-11
**Status:** Design — drafted, pending review
**Goal:** `substrate-kernel-to-cognitive-map`, workstream 1 (scenario-corpus diversification), unblocking workstream 5 (drift detection)
**Scope:** artifact-side + `temper-next` crate only. No production code, no incremental-clustering algorithm, no migration.

---

## Context

The scenario YAML DSL is real and working. A `Scenario` is a seed reference (or inline embed) plus
an ordered `steps` runbook; the runner (`crates/temper-next/src/scenario/runner.rs`) resolves the
seed, loads its substrate through `loader::load_seed`, then executes each step —
`materialize` / `emit_event` / `assert` — with eight expectation kinds (`region_count`, `co_region`,
`cohesion_order`, `region_size`, `internal_tension`, `reproducible`, `fingerprint_differs`,
`stale`). Exactly one scenario exists: `onboarding-cogmap.yaml`, the S6a–h falsification runbook.

The corpus is four **charter seeds** — `temper-convergence`, `temper-foundational`, `storyteller`,
`learning-maths` — authored through the charter-bootstrapping elaboration sessions (PR #127). They
are **charter-only at birth by design**: a telos (statement, questions-with-context, framing), a
one-profile world, and *no domain resources or edges*. The guidance reasoning is explicit in each
seed's header: seeded content would mask the signal of whether the charter itself recruits the right
material as real work runs.

The goal's own framing of the remaining gap (workstream 1 status, 2026-06-11):

> the corpus milestone is reached … what the corpus still lacks for [drift detection] is scenario
> *steps* over these seeds (the charters are charter-only at birth by design).

And workstream 5's acceptance criterion is *"incremental ≡ full across the scenario corpus."* So this
work supplies the **substrate drift detection will later chew on**: runbooks that grow and mutate a
seeded map over time, so that — once WS5 builds the incremental-clustering path — there is a diverse
corpus of real region-evolution to test equivalence against.

A charter map can only become drift-worthy if material can both **arrive and retire**. Today the step
vocabulary can only *add* an edge (`emit_event` → `relationship_assert`). It cannot introduce a new
concept resource, re-facet one, or fold a relationship. This spec closes that gap.

---

## Load-bearing decisions

1. **This thread provides the substrate; WS5 owns the drift machinery.** Runbooks always
   *full*-materialize at each step (`write::materialize_cogmap`, which recomputes from scratch).
   The incremental-clustering path and the `incremental ≡ full` equivalence check are **not built
   here** — they are workstream 5. Drift is *exhibited* by these runbooks, not *detected* by new code.

2. **Drift needs no new expectation kinds.** The existing eight checks express every drift assertion:
   a member moving regions is a `co_region` flip across two materializes; a region splitting is a
   `region_count` delta; a cohesion shift is `cohesion_order`; freshness is `stale`. We add **steps**
   (mutations), not **expectations**.

3. **Folding is in scope, at the edge level only.** Region membership responds to the edge layer, so
   `relationship_fold` is load-bearing for drift; the corpus's natural fold stories are edge-shaped (a
   walked-back correspondence, a superseded decision, a retired commitment). **`block_fold` is
   explicitly out of scope** — it is a *shape-of-visible-resource* concern, not an access concern, and
   the read path already gates correctly on `NOT is_folded`. It needs an event later, not here.

4. **No premature back-compat.** The one existing `emit_event` step (`onboarding-cogmap.yaml`) is
   rewritten to the new `assert_edge` variant. The project is young; we restructure cleanly rather
   than special-case.

5. **Real work is not test material.** The growth pair is `storyteller` + `learning-maths`, *not*
   `temper-convergence`. Convergence is live engineering work (goal workstream 6); using it as a test
   fixture would blur assertion material with real planning. Dogfooding stays in reasonable bounds.

---

## Deliverables

Three phases, each independently reviewable. Multi-session is expected and fine.

### D1 — Step vocabulary + the `relationship_fold` mutation (the machinery)

**Step enum restructure** (`scenario/model.rs`). Each mutation becomes its own `do:` variant,
mirroring the `SeedAction` surface 1:1:

| `do:` | Fields | Backed by |
|---|---|---|
| `create_resource` | `key`, `title?`, `origin_uri`, `doc_type?`, `body`, `facets?` | `SeedAction::ResourceCreate` + `content::prepare_blocks` |
| `set_facet` | `resource` (key), `values`, `weight?` | `SeedAction::FacetSet` |
| `assert_edge` | `from`, `to`, `kind`, `label?`, `weight?` | `SeedAction::RelationshipAssert` (replaces `emit_event`) |
| `fold_edge` | `from`, `to`, `kind`, `reason?` | **new** `SeedAction::RelationshipFold` |
| `materialize` | `lens` | unchanged |
| `assert` | `checks` | unchanged |

- `create_resource` registers its new `key` in the runner's key map, which must become **mutable**
  (today `Loaded.keys` is built once in the loader and returned immutable). New keys are usable by
  subsequent `set_facet` / `assert_edge` / `fold_edge` / `assert` steps.
- `set_facet`, `assert_edge`, `fold_edge` resolve their `resource`/`from`/`to` against the key map and
  fail with a descriptive error on an unknown key (matching the existing `emit_event` pattern).
- `fold_edge` resolves the **live, non-folded** edge at coordinates `{source_id, target_id,
  edge_kind, home}` to its `edge_id`, then fires `RelationshipFold` (identity-as-input: Rust supplies
  the resolved `edge_id`, so the YAML stays coordinate-based). Ambiguity (>1 live edge at those
  coordinates) or no-match is a descriptive error.

**The `relationship_fold` mutation** (`02_functions.sql`), molded exactly on `relationship_assert`:

- `_project_relationship_folded(p_event uuid, p_payload jsonb)` → sets `is_folded = true,
  last_event_id = p_event` on `kb_edges WHERE id = (p_payload->>'edge_id')::uuid`; returns the edge id.
- `relationship_fold(p_payload jsonb, p_emitter uuid)` = `_event_append('relationship_folded',
  p_emitter, <edge home_anchor>, p_payload)` → `_project_relationship_folded`. The home anchor is
  read from the target edge's `home_anchor_table`/`home_anchor_id` (an envelope concern, not payload
  data — same discipline as `facet_set`).
- Payload conforms to the existing `payloads/relationship_folded.v1.schema.json` (`edge_id` required,
  optional `reason`).

**Event-type registration & Rust plumbing:**

- Local `EventKind::RelationshipFolded` → `as_canonical_name() == "relationship_folded"`
  (`events.rs`); parity-shaped with the eventual `kb_events` taxonomy (deliverable-6 merge stays
  rename-free).
- `system.yaml` bootseed `event_types` gains `relationship_folded`; the `kb_event_types` registry row
  is seeded by the bootseed path.
- New `SeedAction::RelationshipFold { edge_id, reason, home, emitter }` arm in `fire()`; typed
  `payloads::RelationshipFolded { edge_id, reason }` struct (no `serde_json::json!()`).
- `cargo make prepare-next` regenerates `crates/temper-next/.sqlx` (temper_next namespace) after the
  new SQL.

### D2 — Smoke runbooks, all four charters

One scenario per charter (`schema-artifact/scenarios/{charter}-smoke.yaml`), referencing the existing
seed. Each: `materialize` under the charter's default lens, assert a sensible region shape
(`region_count >= N`), assert `reproducible` (second materialize, identical fingerprint), and where a
second lens exists assert `fingerprint_differs` (lens sensitivity). These prove the model holds across
the *diverse* corpus — the confidence-gate purpose — cheaply, using the unchanged
`materialize`/`assert` vocabulary. Because the charters are charter-only, smoke runbooks operate over
the telos blocks alone; `region_count` thresholds are calibrated against an initial run, not guessed.

### D3 — Growth/drift runbooks: storyteller + learning-maths

Two scenarios (`schema-artifact/scenarios/{storyteller,learning-maths}-growth.yaml`) telling a
true-to-life "the map grows" story, drawing on the **inbound events each charter names in its own
framing**:

- **learning-maths** — the charter enumerates its inbound events: *"a section engaged, a concept that
  stabilized, a correspondence proposed or walked back."* Growth: concept resources arrive
  (`create_resource`) for engaged sections and stabilized concepts; correspondences are asserted as
  edges (`assert_edge`) toward the telos questions; facets carry the section/concept phase
  (`set_facet`). The **fold** is constitutive here — *"borrowed concepts are walked back where they
  stop landing with specificity"* — so a correspondence edge is `fold_edge`'d, and the affected
  concept regroups on re-materialize.
- **storyteller** — the corpus's non-engineering shape. Growth: persona concepts (narrator,
  storykeeper, character agents) and constitutive commitments accrete as resources, faceted and
  linked so they cluster along the tension axis the charter guards. A superseded commitment's edge is
  `fold_edge`'d as the design matures.

Each runbook follows the **drift-assertion pattern**: baseline `materialize` → snapshot membership via
`co_region`/`region_count` → mutate (`create_resource` + `set_facet` + `assert_edge`/`fold_edge`) →
assert `stale: true` → `materialize` → assert the observable change (a `co_region` flip, a
`region_count` delta, a member moving regions, a `cohesion_order` shift). The concrete resource
bodies, facets, and edges are authored to be domain-credible, not filler.

---

## Out of scope (named, so the boundary is explicit)

- **Incremental deterministic clustering** and the `incremental ≡ full` equivalence check — workstream 5.
- **`block_fold`** (retiring a charter question / concept block) — a later event on the
  shape-of-visible-resource path; the read gate already exists.
- **Production team-mechanics / migration** — workstream 6.
- **`temper-convergence` growth runbook** — deliberately excluded (decision 5).
- New expectation kinds — deliberately none (decision 2).

---

## Testing & verification

- **D1:** model unit tests deserializing each new step variant; an inline-seed roundtrip test
  exercising `create_resource` → `set_facet` → `assert_edge` → `materialize` → `assert` →
  `fold_edge` → `materialize` → `assert` (membership demonstrably changes across the fold); the
  JSON-Schema snapshot test regenerated (`tests/scenario_schema.rs`, `scenario-schema` feature).
- **D2 / D3:** each runbook is a `#[sqlx::test]` in the `temper-next-write` nextest group (owns the
  `temper_next` namespace: resets to clean `01_schema` + `02_functions`, then loads — ONNX-gated via
  `artifact-tests`).
- **Full gate:** `cargo nextest run -p temper-next --features artifact-tests` and `cargo make check`
  (with `cargo make prepare-next` run after the D1 SQL change).

---

## Connections

- Goal: `substrate-kernel-to-cognitive-map` (workstream 1 supplies WS5's substrate).
- Drift decision (the consumer of this substrate):
  `2026-06-07-cogmap-region-drift-detection-lens-relative-incremental-deterministic-clustering`.
- Scenario DSL origin: `2026-06-07-scenario-yaml-seed-dsl-design.md`; seed/scenario split: PR #126.
- Corpus: the four charter seeds in `schema-artifact/seeds/` (PR #127).
- The `emit_event`-only precedent being generalized: `runner.rs::emit_event` +
  `onboarding-cogmap.yaml`.
- Fold payload contract: `schema-artifact/payloads/relationship_folded.v1.schema.json`.
