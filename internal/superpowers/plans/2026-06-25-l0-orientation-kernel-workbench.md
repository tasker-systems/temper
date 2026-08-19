# L0 Orientation-Kernel Workbench Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Develop the L0 "what is temper" orientation kernel as a `temper-next` seed + scenario, and demonstrate — under two new global posture-lenses (`orientation`, `wayfinding`) — that the same kernel reads as a different region shape depending on the posture.

**Architecture:** Two new global lenses added to the bootseed (`system.yaml`); an L0 seed YAML carrying the charter (telos + six questions + framing) **and** the seeded landmark content (concept / invariant / reference / boundary resources + edges) — "born populated"; a scenario YAML that materializes the seed under both lenses and asserts non-degeneracy, reproducibility, and lens-sensitivity. Runs on the proven `corpus_smoke.rs` harness in the serialized `temper-next-write` nextest group. Workbench-only (W1) — no production delivery.

**Tech Stack:** YAML (serde_yaml) seed/scenario DSL; `temper-next` scenario runner (`run_scenario`); Postgres + pgvector in the `temper_next` test namespace; ONNX bge-768 embeddings (auto-on under `artifact-tests`); cargo-nextest.

## Global Constraints

- **Workbench-only (W1).** All work lands in `crates/temper-next/tests/fixtures/` + the bootseed `system.yaml` + `corpus_smoke.rs`. **No production migration, no change to the live `system-default` cogmap.** Production delivery + L0 lifecycle are deferred (see spec §6).
- **Do NOT modify `temper-foundational.yaml`** (seed or scenario) — L0 is a distinct map.
- **Lens YAML shape is FLAT** (per `system.yaml` line 38–39 + `LensDef` in `model.rs:232`): `{ name, w_express, w_contains, w_leads_to, w_near, w_prop, s_telos, s_ref, s_central, resolution }`. (The spec's nested `{weights, salience}` sketch is the *production* SQL shape, not the workbench YAML shape.)
- **EdgeKind values:** `express | contains | leads_to | near` (only these four).
- **CmpOp values:** `">=" | ">" | "=="`.
- **Facet shape in YAML:** explicit form `facets: { values: { layer: concept } }` (used throughout for clarity).
- **Seed owner/emitter:** `owner` is a profile **handle** declared in `world.profiles`; `emitter` is an entity **name** declared in `world.entities`. Use a fresh handle `temper` (NOT `system` — `system` is already seeded by the bootseed and would collide on the loader's profile insert).
- **`system_access` allowed values:** `none | approved | admin`.
- **Test command (single test):**
  `DATABASE_URL='postgresql://temper:temper@localhost:5437/temper_development?options=-csearch_path%3Dtemper_next,public' SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests <name>`
  Group run: `cargo make test-next`. Requires Docker Postgres on 5437 (`cargo make docker-up`) and builds ONNX.
- **No new `sqlx::query!` macros** are added (all work is YAML data + an existing-pattern test), so **no `cargo make prepare-next` is needed.**
- Run `cargo make check` before every commit.

---

## File Structure

- **Modify:** `crates/temper-next/tests/fixtures/seeds/system.yaml` — add `orientation` + `wayfinding` to the `lenses:` list (the only bootseed change).
- **Create:** `crates/temper-next/tests/fixtures/seeds/l0-kernel.yaml` — the L0 charter + seeded landmark resources + edges.
- **Create:** `crates/temper-next/tests/fixtures/scenarios/l0-kernel-orientation.yaml` — `seed:` ref + `materialize` under both lenses + `assert` checks.
- **Modify:** `crates/temper-next/tests/corpus_smoke.rs` — add the `l0_kernel_orientation` test fn (rides the existing `run_smoke` harness; already in the `temper-next-write` group).
- **Modify (only if needed):** `crates/temper-next/tests/bootseed.rs` — if it asserts an exact global-lens count, bump it for the two new lenses.

---

### Task 1: Add the `orientation` + `wayfinding` global posture-lenses to the bootseed

**Files:**
- Modify: `crates/temper-next/tests/fixtures/seeds/system.yaml`
- Test: `crates/temper-next/tests/bootseed.rs`

**Interfaces:**
- Consumes (existing): `bootseed::seed_system(pool)` loads `system.yaml`'s `lenses:` as global lenses (`cogmap_id` NULL); `kb_cogmap_lenses(name, cogmap_id)`.
- Produces: two global lenses named `orientation` and `wayfinding`, available to any scenario's `materialize`/`uses_lenses`.

- [ ] **Step 1: Write the failing test**

Add to `crates/temper-next/tests/bootseed.rs` (match the file's existing `#[tokio::test]` + `common::reset_artifact()` + `seed_system` pattern — read the top of the file first to mirror imports/setup):

```rust
#[tokio::test]
async fn bootseed_creates_orientation_and_wayfinding_lenses() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    temper_next::scenario::bootseed::seed_system(&pool)
        .await
        .unwrap();

    let names: Vec<String> = sqlx::query_scalar(
        "SELECT name FROM kb_cogmap_lenses WHERE cogmap_id IS NULL ORDER BY name",
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    assert!(
        names.contains(&"orientation".to_string()),
        "expected a global `orientation` posture-lens, got {names:?}"
    );
    assert!(
        names.contains(&"wayfinding".to_string()),
        "expected a global `wayfinding` posture-lens, got {names:?}"
    );
}
```

- [ ] **Step 2: Run test to verify it fails**

Run:
```bash
DATABASE_URL='postgresql://temper:temper@localhost:5437/temper_development?options=-csearch_path%3Dtemper_next,public' SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests bootseed_creates_orientation_and_wayfinding_lenses
```
Expected: FAIL — the assertion prints a `names` list containing only `telos-default` and `telos-default-propheavy`.

- [ ] **Step 3: Add the two lenses to `system.yaml`**

In `crates/temper-next/tests/fixtures/seeds/system.yaml`, append two entries to the `lenses:` list (after `telos-default-propheavy`), matching the existing flat shape exactly:

```yaml
  - { name: orientation, w_express: 1.0, w_contains: 1.0, w_leads_to: 0.1, w_near: 0.5, w_prop: 1.0, s_telos: 0.6, s_ref: 0.3, s_central: 0.1, resolution: 0.5 }
  - { name: wayfinding,  w_express: 0.4, w_contains: 0.4, w_leads_to: 1.5, w_near: 0.2, w_prop: 0.2, s_telos: 0.3, s_ref: 0.5, s_central: 0.2, resolution: 0.5 }
```

(`orientation` = survey-attention: high telos-salience + `prop`, low `leads_to`. `wayfinding` = graph-traversal: heavy `leads_to`, references salient. These are the spec §5 starting cut; Task 3 may tune them.)

- [ ] **Step 4: Run the test to verify it passes**

Run the same command as Step 2. Expected: PASS.

- [ ] **Step 5: Guard the existing bootseed tests**

Run the whole bootseed binary — if any test asserts an exact global-lens count (e.g. `== 2`), update it to account for the two new lenses (do NOT delete the assertion):
```bash
DATABASE_URL='postgresql://temper:temper@localhost:5437/temper_development?options=-csearch_path%3Dtemper_next,public' SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests --test bootseed
```
Expected: all PASS (after any count bump).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-next/tests/fixtures/seeds/system.yaml crates/temper-next/tests/bootseed.rs
git commit -m "feat(l0): add orientation + wayfinding global posture-lenses to bootseed

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: Author the L0 kernel seed + a load/materialize scenario (it forms structure)

**Files:**
- Create: `crates/temper-next/tests/fixtures/seeds/l0-kernel.yaml`
- Create: `crates/temper-next/tests/fixtures/scenarios/l0-kernel-orientation.yaml`
- Modify: `crates/temper-next/tests/corpus_smoke.rs`

**Interfaces:**
- Consumes: the `orientation`/`wayfinding` lenses (Task 1); `runner::run_scenario`; the `run_smoke` helper in `corpus_smoke.rs`.
- Produces: a loadable L0 seed (charter + landmark resources keyed for edges/asserts) and a scenario that materializes it; resource keys later assertions use (`event`, `invocation`, `resource`, `search_ref`, …).

- [ ] **Step 1: Author the seed** `crates/temper-next/tests/fixtures/seeds/l0-kernel.yaml`

```yaml
# L0 — the kernel "what is temper" orientation map (spec: 2026-06-25-l0-orientation-kernel-charter).
# Unlike temper-foundational (charter-only at birth), L0 is BORN POPULATED: its content IS its function
# (a set of skill files + references for an arriving agent). Workbench fixture; not the production L0.
name: l0-kernel

cogmap:
  telos:
    title: "What Temper Is"
    statement: "Orient an arriving agent so it can act correctly under temper's substrate at minimal attention cost — by holding the landmarks that say what temper is and how it works, the settled invariants it must not break, and the wayfinding that routes it to the right tool, skill, or more-specific map. This is the bottom referent every agent and every other cognitive map is situated by: it says 'this is the system you are in,' and it actively lowers the activation energy to reach for — and compose — the capabilities temper offers, so a less-powerful model acts where it would otherwise stall. In service of any agent, on any model, becoming competent-to-act in temper without rediscovering the system."
    questions:
      - question: "Is this a landmark an agent needs the moment it arrives to know what temper is and where it stands — the substrate, this map's bottom-referent role, the telos it's currently thinking under?"
        context: "The first thing any agent asks is where am I. Hold the few situating landmarks, not their depth."
      - question: "Is this a core term an agent must share to read temper at all — cogmap, telos, resource, edge, facet, region, lens, event, invocation — versus jargon a specific map can own?"
        context: "An agent that can't read the system's words can't act in it. L0 is the kernel-vocabulary bedrock; deeper or domain terms live where they're used."
      - question: "Is this a settled invariant an agent must not break — event-as-primary, the access floor (it operates as a scoped principal), agents tend declared structure and never cluster, acts carry attribution, cross-map promotion is human-gated?"
        context: "A weaker model won't infer these and will violate them by default. State the always/nevers plainly as landmarks — this is where L0 earns its keep."
      - question: "When an agent needs to do something, does L0 name the tool, skill, or map to reach for — and make reaching the obvious next move?"
        context: "Weaker models stall not for lack of reasoning but for lack of willingness to reach for and compose tools. L0 routes — need X, the tool is Y, use it; compose Y with Z — it gives permission to act."
      - question: "Is this depth that belongs in a more-specific map, with L0 holding only the landmark and the path to it?"
        context: "L0 holds landmarks-and-the-way-to-reach, never contents. What falls through to be elaborated here is the saturation pole — it bloats the kernel an arriving model must read."
      - question: "Does the agent need this to know the edge of what it may do here — what's out of bounds, what needs a human, what it must not assume?"
        context: "An oriented agent also needs to know where its competence and authority stop — the HITL gates, the leak-safety floor it can't cross, the acts that aren't its to make. This is also where a steward learns to read a telos-charter as an instrument: acting-under-a-telos is the steward's job."
    framing:
      - "This map is self-referential: temper mapped in temper's own substrate; the canonical worked-example of a bootstrapped map."
      - "It is a reference layer — skill files and references — born populated and curated, not accreted from work. That is what distinguishes it from every other map."
      - "Every other map (organizational-foundational, domain) is situated by L0 and routes through it; L0 holds kernel landmarks and the paths, the specific maps hold depth."
      - "Authored for the arriving agent, possibly a weaker model — invariant-forward, scannable, landmark-shaped. Every byte costs context; the attention-manifesto extends to agents."
      - "L0 models how a telos-charter is read to make judgment calls, so a steward learns to find the edge of its mandate from the charter itself."
  owner: temper
  emitter: "kernel-curator#1"

world:
  profiles:
    - { handle: temper, display_name: "Temper System", system_access: admin }
  entities:
    - { name: "kernel-curator#1", profile: temper }

# Born populated: the four landmark categories (concept / invariant / reference / boundary).
resources:
  # --- concept-landmarks (Q2 vocabulary) ---
  - { key: cogmap,     origin_uri: "temper://kernel/concept/cogmap",     facets: { values: { layer: concept } },  body: "A cognitive map: a bounded, telos-governed view of resources and their relationships. An agent works inside one map's frame at a time." }
  - { key: telos,      origin_uri: "temper://kernel/concept/telos",      facets: { values: { layer: concept } },  body: "A map's telos: its declared purpose, held as a charter (statement + questions-with-context + framing). The telos is the perspective under which salience is judged — salience is never universal." }
  - { key: resource,   origin_uri: "temper://kernel/concept/resource",   facets: { values: { layer: concept } },  body: "A resource: the named, findable unit of content in a map. Addressed by ref; its body is content blocks." }
  - { key: edge,       origin_uri: "temper://kernel/concept/edge",       facets: { values: { layer: concept } },  body: "An edge: a declared, typed relationship between resources (express, contains, leads_to, near). Edges are authored, never inferred." }
  - { key: facet,      origin_uri: "temper://kernel/concept/facet",      facets: { values: { layer: concept } },  body: "A facet: a key/value property on a resource (e.g. layer: concept). Facet overlap binds resources into families and is an affinity input." }
  - { key: region,     origin_uri: "temper://kernel/concept/region",     facets: { values: { layer: concept } },  body: "A region: a materialized cluster of resources under a lens. Regions are the substrate's pure function over edges + facets — agents never assign them." }
  - { key: lens,       origin_uri: "temper://kernel/concept/lens",       facets: { values: { layer: concept } },  body: "A lens: a weighting over edge-kinds and salience that shapes how a map materializes into regions. The same map yields different regions under different lenses." }
  - { key: event,      origin_uri: "temper://kernel/concept/event",      facets: { values: { layer: concept } },  body: "An event: the append-only record of a mutation, projected to state. The ledger, not the row, is the source of truth." }
  - { key: invocation, origin_uri: "temper://kernel/concept/invocation", facets: { values: { layer: concept } },  body: "An invocation: one accountable agent run — its trigger, its scope (a cogmap's telos), the mutation events it produced, and a terminal outcome." }
  - { key: steward,    origin_uri: "temper://kernel/concept/steward",    facets: { values: { layer: concept } },  body: "A steward: an agent that tends a map's declared structure under its telos — creating resources, asserting edges, setting facets — but never clustering." }
  # --- invariant-landmarks (Q3) ---
  - { key: inv_event_primary, origin_uri: "temper://kernel/invariant/event-as-primary", facets: { values: { layer: invariant } }, body: "Always: every mutation is an event appended to the ledger and projected to state. Never edit state directly; the ledger is authoritative and replayable." }
  - { key: inv_access_floor,  origin_uri: "temper://kernel/invariant/access-floor",     facets: { values: { layer: invariant } }, body: "Always: you operate as a scoped principal. You can only read and write within your map's visibility; the substrate enforces this — you cannot reach beyond your bounds even by mistake." }
  - { key: inv_tend,          origin_uri: "temper://kernel/invariant/tend-not-cluster",  facets: { values: { layer: invariant } }, body: "Always tend declared structure (resources, edges, facets). Never compute regions or assign salience — region formation is the substrate's pure function on materialize." }
  - { key: inv_attribution,   origin_uri: "temper://kernel/invariant/attribution",       facets: { values: { layer: invariant } }, body: "Always: your structural acts carry attribution — a reason and a confidence band — so every act is reviewable and reversible." }
  - { key: inv_promotion,     origin_uri: "temper://kernel/invariant/promotion-gated",   facets: { values: { layer: invariant } }, body: "Lifting a concept across into a different map (promotion-translation) is human-gated: it means something different under the target telos. Never promote autonomously." }
  # --- wayfinding references (Q4/Q5) ---
  - { key: ref_search,     origin_uri: "temper://kernel/reference/search",     facets: { values: { layer: reference } }, body: "To find what already exists by meaning or graph-nearness, reach for the search tool. Start here before creating anything." }
  - { key: ref_create,     origin_uri: "temper://kernel/reference/create",     facets: { values: { layer: reference } }, body: "To add a resource reach for resource_create; to relate two, relationship_assert; to tag one, facet_set. Compose them: create, then relate, then facet." }
  - { key: ref_materialize, origin_uri: "temper://kernel/reference/materialize", facets: { values: { layer: reference } }, body: "To see the map's current regions, reach for request_materialize under a lens. Read the regions to orient before acting." }
  - { key: ref_charter,    origin_uri: "temper://kernel/reference/charter",    facets: { values: { layer: reference } }, body: "To understand a map's purpose and the edge of your mandate, read its telos-charter. The questions-with-context tell you what belongs and what doesn't." }
  # --- boundary-landmarks (Q6) ---
  - { key: bnd_hitl,    origin_uri: "temper://kernel/boundary/hitl",    facets: { values: { layer: boundary } }, body: "Out of bounds without a human: cross-map promotion, founding a new map's identity, and changing a settled commitment. Pause and ask." }
  - { key: bnd_leak,    origin_uri: "temper://kernel/boundary/leak",    facets: { values: { layer: boundary } }, body: "You cannot read another team's material into a lower map — the access floor forbids it structurally. If a read returns nothing, it may be out of scope, not absent." }
  - { key: bnd_mandate, origin_uri: "temper://kernel/boundary/mandate", facets: { values: { layer: boundary } }, body: "Read the charter's questions to find the edge of your mandate: if material is depth for a more-specific map, route it there rather than elaborating it here." }

edges:
  # concept vocabulary clusters (near / express) — orientation topology
  - { from: event,      to: invocation, kind: near,    label: ledger-grain }
  - { from: invocation, to: steward,    kind: express, label: run-by }
  - { from: cogmap,     to: region,     kind: contains, label: holds }
  - { from: region,     to: lens,       kind: express, label: shaped-by }
  - { from: telos,      to: cogmap,     kind: express, label: governs }
  - { from: resource,   to: edge,       kind: near,    label: graph-grain }
  - { from: facet,      to: region,     kind: express, label: binds-into }
  # invariants govern concepts (express)
  - { from: inv_event_primary, to: event,      kind: express, label: governs }
  - { from: inv_access_floor,  to: resource,   kind: express, label: governs }
  - { from: inv_tend,          to: region,     kind: express, label: governs }
  - { from: inv_attribution,   to: invocation, kind: express, label: governs }
  # wayfinding routes (leads_to) — reference -> capability
  - { from: ref_search,      to: resource,   kind: leads_to, label: to-find }
  - { from: ref_create,      to: resource,   kind: leads_to, label: to-add }
  - { from: ref_materialize, to: region,     kind: leads_to, label: to-see }
  - { from: ref_charter,     to: telos,      kind: leads_to, label: to-understand }

uses_lenses: [orientation, wayfinding]
```

- [ ] **Step 2: Author a minimal load/materialize scenario** `crates/temper-next/tests/fixtures/scenarios/l0-kernel-orientation.yaml`

(Start with non-degeneracy + reproducibility only — Task 3 adds the differentiation asserts.)

```yaml
# Materialize the L0 kernel under both posture-lenses; prove it forms a non-degenerate, reproducible
# shape under each. Task 3 adds the lens-sensitivity + co-region assertions.
name: l0-kernel-orientation
seed: ../seeds/l0-kernel.yaml
steps:
  - { do: materialize, lens: orientation }
  - { do: materialize, lens: wayfinding }
  - do: assert
    checks:
      - { check: region_count, lens: orientation, op: ">=", value: 2 }
      - { check: region_count, lens: wayfinding,  op: ">=", value: 2 }
  - { do: materialize, lens: orientation }
  - { do: assert, checks: [ { check: reproducible, lens: orientation } ] }
  - { do: materialize, lens: wayfinding }
  - { do: assert, checks: [ { check: reproducible, lens: wayfinding } ] }
```

- [ ] **Step 3: Add the test fn** to `crates/temper-next/tests/corpus_smoke.rs`

Mirror the existing `temper_foundational_smoke` fn exactly (it calls the file's `run_smoke` helper):

```rust
#[tokio::test]
async fn l0_kernel_orientation_smoke() {
    run_smoke("l0-kernel-orientation.yaml").await
}
```

- [ ] **Step 4: Run it**

```bash
DATABASE_URL='postgresql://temper:temper@localhost:5437/temper_development?options=-csearch_path%3Dtemper_next,public' SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests l0_kernel_orientation_smoke
```
Expected: PASS — the seed loads, embeds, and materializes into ≥2 regions under each lens, reproducibly. If `region_count` fails (one blob), the lens `resolution` is too low or edges too dense; this is a real shape finding — adjust the seed's edges or the lens `resolution` (do not weaken the assertion below 2). If load fails on profile collision, confirm `owner: temper` (not `system`).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-next/tests/fixtures/seeds/l0-kernel.yaml crates/temper-next/tests/fixtures/scenarios/l0-kernel-orientation.yaml crates/temper-next/tests/corpus_smoke.rs
git commit -m "feat(l0): author the L0 kernel seed (born populated) + load/materialize scenario

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Demonstrate posture-sensitivity (the headline) + tune

**Files:**
- Modify: `crates/temper-next/tests/fixtures/scenarios/l0-kernel-orientation.yaml`
- Modify (only if tuning needs it): `crates/temper-next/tests/fixtures/seeds/system.yaml`, `crates/temper-next/tests/fixtures/seeds/l0-kernel.yaml`

**Interfaces:**
- Consumes: the materialized fingerprints per lens (runner caches them); resource keys from Task 2.
- Produces: the asserted evidence that orientation and wayfinding produce different shapes, and that the intended pairs co-region under the intended posture.

- [ ] **Step 1: Add the differentiation + co-region asserts**

Replace the first `assert` block in `l0-kernel-orientation.yaml` with the headline checks (keep the reproducible steps after):

```yaml
  - do: assert
    checks:
      - { check: region_count, lens: orientation, op: ">=", value: 2 }
      - { check: region_count, lens: wayfinding,  op: ">=", value: 2 }
      # Headline: the two postures produce different shapes over the same kernel.
      - { check: fingerprint_differs, lens_a: orientation, lens_b: wayfinding }
      # Orientation binds concept-landmarks that share a facet + a near edge.
      - { check: co_region, lens: orientation, members: [event, invocation], expect: true }
      # Wayfinding binds a reference to the capability it routes to (leads_to chain).
      - { check: co_region, lens: wayfinding, members: [ref_search, resource], expect: true }
```

- [ ] **Step 2: Run and observe**

```bash
DATABASE_URL='postgresql://temper:temper@localhost:5437/temper_development?options=-csearch_path%3Dtemper_next,public' SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests l0_kernel_orientation_smoke
```
Expected on first run: `fingerprint_differs` and the `reproducible` checks PASS (the two lenses weight `leads_to`/`prop` very differently). The two `co_region` checks may or may not hold on the first try.

- [ ] **Step 3: Tune the MAP (not the assertions) until the co-region checks hold**

This is authoring, not softening. If `co_region [event, invocation] @ orientation` is false: strengthen their binding — raise the `near` edge `weight` (e.g. `weight: 0.8`) or confirm both carry `layer: concept` (facet overlap + `prop`-heavy orientation lens should bind them). If `co_region [ref_search, resource] @ wayfinding` is false: raise the `leads_to` edge `weight` on `ref_search → resource`, or nudge the `wayfinding` lens `w_leads_to` up / `resolution` down. Re-run after each change.

**Escalation rule:** if a co-region claim cannot be made true by tuning edges/facets/lens-weights into a *sensible* shape (e.g. it would require an absurd weight or contradict another check), STOP — do not weaken or delete the assertion to pass. Report which pair won't bind and why; that is a real finding about the lens design, not a thing to paper over. A legitimate alternative is to choose a *different* representative pair that the substrate genuinely binds and document the swap.

- [ ] **Step 4: Confirm reproducibility survived tuning**

Re-run the full test (Step 2 command). Expected: all checks PASS — `region_count ≥ 2` both lenses, `fingerprint_differs`, both `co_region`, both `reproducible`.

- [ ] **Step 5: Crate regression guard**

Run the whole `temper-next` write-path group to confirm no fixture/lens change broke a sibling scenario:
```bash
cargo make test-next
```
Expected: all PASS (existing corpus/scenario tests + the two new ones).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-next/tests/fixtures/scenarios/l0-kernel-orientation.yaml crates/temper-next/tests/fixtures/seeds/l0-kernel.yaml crates/temper-next/tests/fixtures/seeds/system.yaml
git commit -m "feat(l0): demonstrate orientation vs wayfinding posture-sensitivity over the kernel

Same L0 kernel materializes to different region shapes per posture-lens
(fingerprint_differs); concept-landmarks co-region under orientation, a
reference->capability route co-regions under wayfinding.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**1. Spec coverage:**
- Telos statement (§1) → seed `cogmap.telos.statement` (Task 2). ✓
- Six questions-with-context (§2) → seed `telos.questions` verbatim (Task 2). ✓
- Framing (§3) → seed `telos.framing` (Task 2). ✓
- Four seeded landmark categories + edges (§4) → seed `resources` (concept/invariant/reference/boundary) + `edges` (Task 2). ✓
- Two global posture-lenses (§5) → `system.yaml` (Task 1). ✓
- "See it in practice" / lens-sensitivity (§5, §7) → Task 3 (`fingerprint_differs` + co-region under each posture). ✓
- Non-degenerate + reproducible (§7) → Task 2/3 (`region_count >= 2`, `reproducible`). ✓
- W1 workbench-only, no production delivery (§6) → Global Constraints + no migration touched. ✓
- "Born populated, not charter-only" → landmarks in the seed's `resources` (not deferred). ✓
- Don't modify `temper-foundational` → Global Constraints. ✓

**2. Placeholder scan:** No TBD/TODO. Every step has runnable YAML/Rust + exact commands. The tuning in Task 3 Step 3 is bounded by an explicit escalation rule (tune the map into a sensible shape OR report BLOCKED / swap to a pair the substrate genuinely binds — never weaken the assertion). The "only if needed" edits (bootseed count, tuning) are conditioned on observed test output, not vague.

**3. Type/name consistency:** Lens names `orientation`/`wayfinding` identical across Task 1 (system.yaml), Task 2 (`uses_lenses` + `materialize`), Task 3 (`fingerprint_differs`/`co_region` lens args). Resource keys (`event`, `invocation`, `resource`, `ref_search`) defined in Task 2's seed and referenced in Task 3's asserts. Flat `LensDef` field names (`w_express`…`resolution`) match `system.yaml`/`model.rs`. EdgeKinds used (`near`/`express`/`contains`/`leads_to`) are all in the allowed set. Facet `values: { layer: … }` form consistent. `owner: temper` / `emitter: kernel-curator#1` consistent between `cogmap` and `world`.

**Implementer pre-flight (real grounding, not placeholder):** read the top of `corpus_smoke.rs` to copy its exact `use`/`mod common` lines and the `run_smoke` signature, and the top of `bootseed.rs` for its setup pattern, before adding the new test fns. Confirm `cargo make docker-up` is running.
