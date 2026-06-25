# L0 Orientation-Kernel Charter — Design

**Date:** 2026-06-25
**Status:** Design / spec
**Context:** Workstream 7 (Agent surface) under goal `substrate-kernel-to-cognitive-map`.
**Companions:**
- Architecture: [2026-06-25-cognitive-map-agent-invocation-architecture-design.md](2026-06-25-cognitive-map-agent-invocation-architecture-design.md) (defines L0 as the deterministic kernel tier)
- Plan (birth mechanism, done): [docs/superpowers/plans/2026-06-25-l0-kernel-cognitive-map.md](../plans/2026-06-25-l0-kernel-cognitive-map.md) — L0 born empty (SQL-native, `system-default` cogmap)
- Postures source: research `2026-05-23-cognitive-maps-and-the-projection-class-insight` (names the insight) + `projection-class-plurality--research-area-decomposition` (`019e54bb`, the posture list + the agent-arrival sharpening)
- Charter form: `2026-06-10-charter-bootstrapping-procedure-design.md`; exemplar `crates/temper-next/tests/fixtures/seeds/temper-foundational.yaml`

---

## Why this design

The architecture spec birthed L0 — the public, root-team-joined `system-default` cogmap — but
**content-light** (an empty telos), with its actual charter content explicitly deferred. This design
fills that gap: it specifies **what L0 *is for* and what it *contains*** as the **orientation kernel for
arriving agents**, and it does so in the `temper-next` scenario *workbench* so we can watch it
materialize before committing to a production-delivery mechanism.

The driving frame (Pete, this session): take the telos-charter concept *seriously* for L0. An agent —
possibly a **less-powerful open-weight model** — "arrives" in temper with a context-window cost for every
byte. What do we tell it? What would it ask? What does it need to know about how temper works — held
"like a set of skill files and references"? The existing `temper-foundational` map was deliberately
*under-theorized* (a shape-testing scenario); a **kernel-for-agents** is a higher bar.

### The postures (grounding)

The projection-class research cuts a family of **information postures** by *phenomenology of attention* —
each wants a different kind of attention from the engaging perspective:

> **orientation · wayfinding · recall · recognition · composition · boundary-sensing · translation · trust-calibration**

- **orientation** wants *survey-attention* ("where am I") and is *role-universal* — every perspective asks it.
- **wayfinding** is *graph-traversal* ("how do I get from here to there").
- The research flags the **agent-arrival case as the hardest**: an agent landing into a session pays
  context cost per byte; the attention-manifesto's commitment *extends to agents*.

**Postures are lenses (projection-classes), not content.** A map is *read through* a posture; L0 is the
map *built to serve the orientation posture well*, while remaining readable under others.

### Decision: L0 serves orientation **+** wayfinding as a paired kernel

(Chosen over orientation-alone and over a fully posture-lensed content model.) Rationale: orientation
alone risks weaker models **stalling** — the gap in such models is not only reasoning but **agency**: the
willingness to notice a tool, reach for it, and compose small tools into larger ones. Wayfinding, made
active, is an **activation-energy reducer for tool-use**: "need X → the tool is Y → reach for it → you may
compose Y with Z." L0 both *situates* and *routes*, and it *gives permission to act*. This is the
"skill files **and** references" framing made literal.

### Decision: L0 is born **populated** (a reference layer), not charter-only

`temper-foundational` is charter-only-at-birth ("seeded content masks the charter's own signal"; landmarks
accrete from real work). L0 is the **opposite kind of object: its content *is* its function.** An empty
orientation kernel orients no one; a set of skill files and references must ship *with* the skills and
references. L0 is therefore born populated and grows by **curation** (release/operator-governed, per the
architecture spec's governance boundary), not by organic accretion. L0 is a *distinct* map from
`temper-foundational` (which is system-owned `system`, public; `temper-foundational` is `owner: pete`, an
L1-flavored exemplar) — `temper-foundational` is **not modified** by this work.

---

## Section 1 — Telos statement

> **Orient an arriving agent so it can act correctly under temper's substrate at minimal attention cost** —
> by holding the landmarks that say *what temper is and how it works*, the settled invariants it must not
> break, and the wayfinding that routes it to the right tool, skill, or more-specific map. This is the
> bottom referent every agent and every other cognitive map is situated by: it says "this is the system
> you are in," and it actively lowers the activation energy to reach for — and compose — the capabilities
> temper offers, so a less-powerful model acts where it would otherwise stall. **In service of** any agent,
> on any model, becoming competent-to-act in temper without rediscovering the system.

---

## Section 2 — Questions-with-context

The charter's load-bearing structure: simultaneously *what L0 helps an arriving agent answer* and *the
test for what belongs in it*. Six questions, mapping to **orientation ×3, wayfinding ×2, boundary ×1**.

1. **Situation** (orientation / survey-attention)
   - *Q:* "Is this a landmark an agent needs **the moment it arrives** to know what temper is and where it
     stands — the substrate, this map's bottom-referent role, the telos it's currently thinking under?"
   - *Context:* "The first thing any agent asks is *where am I*. Hold the few situating landmarks, not their depth."
2. **Shared vocabulary** (orientation / translation bedrock)
   - *Q:* "Is this a core term an agent must share to **read** temper at all — cogmap, telos, resource, edge,
     facet, region, lens, event, invocation — versus jargon a specific map can own?"
   - *Context:* "An agent that can't read the system's words can't act in it. L0 is the kernel-vocabulary
     bedrock; deeper or domain terms live where they're used."
3. **Invariants / how to act correctly** (orientation, invariant-forward)
   - *Q:* "Is this a **settled invariant an agent must not break** — event-as-primary, the access floor (it
     operates as a scoped principal), agents tend declared structure and never cluster, acts carry
     attribution, cross-map promotion is human-gated?"
   - *Context:* "A weaker model won't infer these and will violate them by default. State the always/nevers
     plainly as landmarks — this is where L0 earns its keep."
4. **Reach for the capability** (wayfinding / activation-energy)
   - *Q:* "When an agent needs to **do** something, does L0 name the tool, skill, or map to reach for — and
     make reaching the obvious next move?"
   - *Context:* "Weaker models stall not for lack of reasoning but for lack of willingness to reach for and
     compose tools. L0 routes — 'need X → the tool is Y, use it; compose Y with Z' — it gives permission to act."
5. **Route to depth, don't hold it** (wayfinding / saturation guard)
   - *Q:* "Is this **depth that belongs in a more-specific map**, with L0 holding only the landmark and the
     *path* to it?"
   - *Context:* "L0 holds landmarks-and-the-way-to-reach, never contents. What falls through to be elaborated
     here is the saturation pole — it bloats the kernel an arriving model must read."
6. **Edge of autonomy** (boundary-sensing / edge-attention)
   - *Q:* "Does the agent need this to know the **edge of what it may do here** — what's out of bounds, what
     needs a human, what it must not assume?"
   - *Context:* "An oriented agent also needs to know where its competence and authority stop — the HITL
     gates, the leak-safety floor it can't cross, the acts that aren't its to make. This is also where a
     steward learns to read a telos-charter *as an instrument*: acting-under-a-telos is the steward's job,
     so the kernel models how a charter is read to find the edge of a mandate."

Recall / recognition / composition / translation / trust-calibration are deliberately **not** standalone
charter questions — they are postures an agent applies *later, in specific maps*, not things L0's founding
context must answer (keeping the kernel small per its own telos).

---

## Section 3 — Framing

- **Self-referential:** temper mapped in temper's own substrate; the canonical worked-example of a
  bootstrapped map.
- **A reference layer — "skill files and references":** born populated and *curated*
  (release/operator-governed), not accreted from work. This is what distinguishes L0 from every other map.
- **Tier relationship:** every other map (L1 organizational-foundational, L2+ domain) is *situated by* L0
  and *routes through* it; L0 holds kernel landmarks + the paths, the specific maps hold depth.
- **Attention-economy:** authored for the arriving agent, possibly a weaker model — invariant-forward,
  scannable, landmark-shaped. The attention-manifesto extends to agents.
- **Charters as instruments:** L0 models *how a telos-charter is read to make judgment calls*, so a steward
  learns to find the edge of its mandate from the charter itself.

---

## Section 4 — Seeded content (organized by the questions)

L0 ships with four landmark categories, each a set of crisp, scannable definition-resources (a few
sentences each — landmark, not depth), tagged with a `layer` facet so a posture-lens can weight them.

| Category | Serves | Facet | Contents (initial cut — tuned in the workbench) |
|---|---|---|---|
| **Concept-landmarks** | Q2 vocabulary | `layer: concept` | cogmap · telos · resource · edge · facet · region · lens · event · invocation · steward · map-tier |
| **Invariant-landmarks** | Q3 invariants | `layer: invariant` | event-as-primary · the access floor (scoped principal) · agents-tend-declared-structure-never-cluster · acts-carry-attribution · cross-map-promotion-is-human-gated · cloud-only · agent-first |
| **Wayfinding references** | Q4/Q5 routing | `layer: reference` | "to do X → reach for Y": the MCP tools (`search`, `resource_create`, `relationship_assert`, `facet_set`, `request_materialize`, `read_charter`) · the deeper skills (charter-bootstrapping, map-stewardship) · how to find the right more-specific map |
| **Boundary-landmarks** | Q6 edges | `layer: boundary` | HITL gates · the leak-safety floor · acts-not-yours-to-make |

**Edges (the topology a posture-lens reads):**
- `near` / `express` among concept-landmarks → vocabulary clusters (what-binds-with-what).
- `express` invariant → concept (an invariant *governs* a concept).
- `leads_to` reference → capability ("need this → reach that"). **These `leads_to` chains are what
  wayfinding surfaces.**

---

## Section 5 — Posture-lenses (the dogfood payoff)

Two **global posture-lenses** (`cogmap_id` NULL — peers of the existing `telos-default` /
`telos-default-propheavy`, seeded in the bootseed `system.yaml`), so *any* map can be read through either.
Weight vectors are an **indicative starting cut to be tuned in the workbench**; shape is
`weights{express,contains,leads_to,near,prop}` + `salience{telos,ref,central}` + `resolution`.

- **`orientation`** — survey-attention; landmarks cluster by *what they are*. High telos-salience and `prop`
  (facet overlap binds concept/invariant families), strong `express`/`contains`, low `leads_to` (routes must
  not dominate). Starting cut: `weights{express:1.0, contains:1.0, leads_to:0.1, near:0.5, prop:1.0}`,
  `salience{telos:0.6, ref:0.3, central:0.1}`, `resolution:0.5`.
- **`wayfinding`** — graph-traversal; regions follow *paths-to-capability*. Heavy `leads_to`, low `prop`/`near`,
  references salient. Starting cut: `weights{express:0.4, contains:0.4, leads_to:1.5, near:0.2, prop:0.2}`,
  `salience{telos:0.3, ref:0.5, central:0.2}`, `resolution:0.5`.

**"See it in practice"** = the scenario materializes L0 under **both** lenses and asserts the region shapes
**differ** (`fingerprint_differs`). Same kernel, read as *orientation* (concept/invariant families) or as
*wayfinding* (routes to capability) depending on the posture-lens — projection-class plurality made
concrete and testable, on the proven scenario-builder rails.

---

## Section 6 — Delivery: workbench-first (W1)

This round lands entirely in the **`temper-next` scenario workbench**:

- A **seed** `crates/temper-next/tests/fixtures/seeds/l0-kernel.yaml` — the charter (Section 1 telos +
  Section 2 questions + Section 3 framing), `owner: system`-flavored, world minimal.
- A **scenario** `crates/temper-next/tests/fixtures/scenarios/l0-kernel-orientation.yaml` — `steps` that
  seed the Section 4 landmarks + edges + facets, then `materialize` under `orientation` and `wayfinding`,
  with `assert` checks (below).
- Two new global lenses (`orientation`, `wayfinding`) added to the bootseed `system.yaml` lens set.

**Production delivery is explicitly deferred to a future branch.** Landing this content on the *live*
`system-default` L0 hits the embeddings-in-a-migration problem (content chunks carry bge-768 embeddings; a
migration can't run ONNX) and — separately — the **lifecycle question: "when does L0 update, and what
updates it"** (curation cadence, operator vs shipped-scenario, the L0↔L1 promotion path). Both deserve
their own design rather than being shoehorned here. The architecture spec already records the principle
(L0 evolves via additive shipped scenarios + operator-directed runs; ambient steward wake = never); the
*mechanism* is the deferred thread.

---

## Section 7 — Validation (what the scenario asserts)

On the proven `temper-next` artifact-test rails (`run_scenario`, the `temper-next-write` group, ONNX embed):

1. **Non-degenerate:** `region_count >= 2` under each posture-lens (the kernel forms real structure, not one blob).
2. **Co-region (orientation):** concept-landmarks that share a `layer`/facet and a `near` edge land in one
   region under `orientation` (e.g. event ↔ invocation, or two vocabulary terms that bind).
3. **Reproducible:** a second `materialize` under the same lens yields the same fingerprint.
4. **Lens-sensitive (the headline):** `fingerprint_differs` between `orientation` and `wayfinding` — the two
   postures produce genuinely different shapes over the same kernel.
5. **Routes surface under wayfinding:** a `leads_to` reference→capability chain co-regions under `wayfinding`
   where it would not under `orientation`.

---

## Deferred / open (named, not dropped)

- **Production delivery of L0 content** — embeddings-in-migration vs operator-run ingest; a future branch.
- **L0 lifecycle — "when does it update, and what updates it"** — curation cadence, shipped-scenario vs
  operator-directed steward, the L0↔L1 promotion path. The architecture spec states the principle; the
  mechanism is undesigned.
- **The other postures** (recall / recognition / composition / translation / trust-calibration) as
  first-class lenses — only `orientation` + `wayfinding` are built here.
- **Tuning the lens weight vectors** — the Section 5 vectors are a starting cut; the workbench is where they
  earn their values against the asserted shape.

---

## Self-review notes

- **Scope:** one focused workbench deliverable (a seed + a scenario + two lenses + assertions). Production
  delivery and lifecycle are cleanly excluded.
- **Consistency:** the six questions, four content categories, and two posture-lenses align — every content
  category serves a question and is weighted by a posture-lens; orientation×3 / wayfinding×2 / boundary×1
  matches Section 2.
- **Grounded:** charter form mirrors `temper-foundational.yaml`; scenario steps + checks
  (`create_resource`/`assert_edge`/`materialize`/`assert` with `region_count`/`co_region`/`reproducible`/
  `fingerprint_differs`) mirror `temper-foundational-smoke.yaml`; lens shape mirrors the bootseed
  `system.yaml` lenses. The plan must verify the exact `assert` check vocabulary against `scenario.schema.json`
  before authoring.
