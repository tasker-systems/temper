# Cognitive-Map Agent-Invocation Architecture

**Date:** 2026-06-25
**Status:** Design / spec
**Context:** Workstream 7 (Agent surface) under goal `substrate-kernel-to-cognitive-map`.
**Companions:**
- Research: *Vercel Eve & Claude Managed Agents — investigation & comparative analysis* (`019edd1e`)
- Research: *Agentic workflows on temper via Vercel Eve* (`019edc4a`)
- Built foundation: invocation envelope + authorship metadata (PR #148 → carried into canonical schema in #156)
- Access model: `2026-06-02-access-capability-model-design.md` (§8 system-default cogmap)
- Delegation: `2026-06-02-map-to-map-delegation-dissolution-design.md` (originating-map pin, `cogmaps_share_a_team`, reach vs priming-frame)
- Bootstrapping: `2026-06-10-charter-bootstrapping-procedure-design.md`
- Genesis/regulation: `2026-06-04-domain-b-charter-questions-regulation-edge-semantics-design.md` (`cogmap_genesis`)
- Seed/scenario DSL: `2026-06-07-scenario-yaml-seed-dsl-design.md`, `2026-06-11-scenario-steps-over-corpus-seeds-design.md`

---

## Why this design

The agent surface had its *portable* layer (persona, markdown skills, MCP tools, HITL policy) and its
*thin* `DeploymentProfile` (runtime / residency / token-budget) settled. What stayed open was the
**fact-of-invocation**: the concrete mechanism that takes a trigger condition and produces a running
agent session bound to (persona, deployment profile, **cognitive-map frame of reference**), and records
it as a `kb_invocations` row opened by a `kb_event`.

Prior analysis had named the *record* (the envelope, shipped in #156) and the *shape* of triggering
(push via an HTTP channel + a schedule backstop), but left two seams unspecified:

1. **The steward "threshold."** The specs were explicit that there is *no* threshold today — the
   relevance call is the agent's judgment "against this cogmap's telos/questions, **not a universal
   threshold**," made *per inbound event*. Whether to add a real gate, and where it lives, was open.
2. **The bootstrapping frame-of-reference paradox.** The bootstrapper *creates* the cogmap
   (`cogmap_genesis`), so at invocation time there is no map to scope to — yet the delegation model
   assumes an originating map always exists.

This design resolves both by introducing a **deterministic kernel cognitive map** at the bottom and a
**stewardship queue + wake policy** at the top, unifying the two cases under one model: *who holds the
trigger decision, and against what frame.*

---

## The spine: three invocation modes, each the priming frame for the next

The path to invocation is three modes stacked by **who holds the trigger decision**. Each lower tier is
the frame of reference the tier above thinks *with*.

| Tier | Map | Who invokes | Trigger | Frame at invocation |
|---|---|---|---|---|
| **L0** | "what is temper" (the system itself) | **Rust / boot-seed** (no agent) | deterministic, at boot | *is* the bottom frame |
| **L1** | "who does temper exist for" (org-foundational) | **human + MCP-bound agent** | explicit, interactive | primed by L0 |
| **L2+** | domain / concern maps | **steward agent** (autonomous) | stewardship queue + wake policy | primed by L1 (→ L0) |

Two consequences:

- **The org bootstrap becomes a thing-in-kind with every other bootstrap.** "Organizational-level
  foundational bootstrap" is no longer sui-generis; it is one agent-mediated bootstrap among many, all
  sharing the L0 referent at the bottom.
- **The frame-of-reference paradox dissolves.** Because L0 always exists, no bootstrapper is ever truly
  unscoped — there is always a "what is this system" frame to think *with* and refer to, even while a new
  map's own frame is being born.

---

## Tier L0 — the kernel "what is temper" map

### Identity (decision)

L0 **is** the access model's already-reserved **system-default cogmap** (joined only to the root team;
public; the emergent-default originating frame a principal receives before anything more specific — access
spec §8), **now given content**: a telos-charter, intentionally-minted facets-as-properties, and edges
among temper's own concepts.

We fill the reserved root rather than inventing a parallel one. The universal originating-default frame
is therefore *opinionated* — it says what temper is — which is intended: there is a necessary
"temper-as-a-system" foundational map, and every other map is situated above it.

### Birth — deterministic (SQL-native seed migration)

L0 is born at boot **through the same genesis SQL functions every map uses** —
`cogmap_genesis(payload, content, emitter)` → charter-resource → blocks, then
`facet_set` / `relationship_assert` — invoked from a **canonical-seed migration**, authored by the
**system actor** (a peer of the existing seed in `20260624000003_canonical_seed.sql`).

**Mechanism note (grounded 2026-06-25).** The seed-DSL Rust loader (`load_seed`/`run_scenario` in
`temper-next`) is *test-only* — no production path calls it; production boot runs only the
`sqlx::migrate!` files. `cogmap_genesis`/`resource_create`/`facet_set`/`relationship_assert` are SQL
functions callable directly from a migration, so L0 is seeded **migration-native** (no `temper-next`
runtime dependency on `temper-api`). This satisfies the real invariant — *same genesis functions,
deterministic, replay-safe* — rather than the literal "seed DSL" phrasing. The birth is reproducible
from the migration and a genuine worked example of the genesis mechanism (the SQL calls are the same
ones every map's genesis makes). Authoring L0 as a first-class seed/scenario *YAML* loadable in
production is a deferred thread (it would make `temper-next` a runtime dep) — see Deferred.

**Latent gap this closes.** The root team `temper-system` is *referenced* by canonical triggers/functions
but is created only in test fixtures, never in production migrations. L0 needs it, so the seed migration
creates the root team + joins the system-default cogmap to it — fixing a pre-existing production gap.

### Evolution — living, but release/operator-governed

L0 is a *living* map (M1: there is no reason to pretend temper does not grow), but its life is
**release/operator-governed, not operationally-stewarded**:

1. **Shipped map-update scenarios.** When temper-the-software changes, we ship a versioned **scenario**
   (a scenario already = "a seed reference + a `steps` runbook" of
   `resource_create` / `relationship_assert` / `facet_set` / `lens_create`) whose steps accrete/supersede
   L0 blocks. These are *migrations for the self-map* — additive, replay-safe, versioned alongside the
   release. Seed births L0; scenarios grow it.
2. **Operator-directed steward runs (opt-in).** An operator *may* point a steward at L0 for a larger
   restructure, but it is never auto-attached.

**Governance boundary (decision).** L0's charter declares **ambient wake = never (operator-only)**. No
day-to-day steward reacts to operational events (Linear updates, ingest, etc.) on L0. L0's events are
admin/release-shaped and **firewalled from operational cognition** — consistent with the
admin-events-firewalled-from-cognition principle. Day-to-day stewardship belongs to L1/L2+.

---

## Tier L1 — the organizational-foundational bootstrap (interactive)

### What it is

The **first agent-mediated bootstrap**: an MCP-bound session (Claude-Code over the `temper` MCP surface,
or an Eve web channel) running the **charter-bootstrapping skill**, primed by L0. It develops the guided
telos-charter and ingests early documents as `kb_resources`. It answers "who does this temper instance
exist for."

### Genesis shape — shell-first (decision)

1. A human provides a one-line **seed telos** (the org's intent).
2. That deterministically mints the L1 cogmap **shell** via `cogmap_genesis`, primed by L0.
3. The bootstrapper is invoked **scoped to the fresh shell** (`reach` = L1; `priming-frame` = L0 + the
   seed telos) and accretes the charter in place.

The shell exists before the agent acts, so the agent is scoped from its first tool call — the paradox is
gone without special-casing the bootstrapper.

### Resumability as a substrate property

Resumption = re-read L1's current telos blocks + regulation memories and continue. This is a **substrate
property**, not an Eve checkpoint feature, so it works across Eve, Claude managed agents, and runtime
restarts. (This answers the carried-open "resumability" goal-question via the genesis-first model: the
draft *is* the map's accreting blocks, not a YAML file on disk.)

### HITL by nature

The conversation *is* the gate; no schedule.

---

## Tier L2+ — the steward (autonomous): the stewardship queue + wake policy

The steward's "threshold" is not a single number. It is a **wake policy over a stewardship queue**,
realized in two stages. This keeps the determinism reframe intact: stage 1 is pure substrate; the steward
still only *tends declared structure* and **never clusters** (region formation stays the substrate's pure
function on `materialize`).

### Stage 1 — substrate (deterministic, pure)

**The `stewardship_requested` event (new, first-class).** Any *authorized* actor — including a human or a
**non-steward** MCP working session — emits it to enqueue a **named material set** against a **target
cogmap**, with an **urgency**. Cross-map requests are gated by the existing
`cogmaps_share_a_team(source, target)`. This is the declarative affordance: e.g. you and a non-steward
MCP agent review a Notion doc together, create a `kb_resources` research document, and tag it (plus the
reference to the original) *for stewardship immediately*, so a steward picks it up and acts under the
persona + telos guidance.

**The stewardship queue (projection).** `stewardship_requested` events project into a queue table
(pattern: as `kb_invocations` projects from events). Queue items are also written by:
- **Derived/inferred signals** — un-triaged material count, or change-magnitude since the last steward run
  (mirroring how WS5 region re-materialization fires on "changes exceed a bound").
- **External webhooks** — Linear ticket created / status changed, etc., *mapped to queue items*, **not**
  per-hook invocations.

Each item carries: target cogmap (the frame), the material refs, an urgency/priority, and the
source/`trigger_kind`.

**The wake policy** decides when to invoke the steward:
- **Directed** items (explicit `stewardship_requested`, high urgency) → **push / prompt wake**. They
  bypass debounce: the worth-acting call has already been made; the steward's job is to *execute under the
  telos*, not to decide whether.
- **Ambient** items (derived + webhooks) → **debounced pull**: wake when accumulation crosses a bound, *or*
  a cadence elapses, *or* a priority item lands. **This is the answer to the per-hook question: a webhook
  enqueues a cheap item and returns; it never invokes the steward directly.** Per-hook invocation is the
  agent-overwhelm failure mode; debouncing into the queue bounds staleness without thrashing a run per
  ticket-status-change.

Freshness-vs-overwhelm is therefore *tunable*, not a hardcoded posture: directed work is fresh-by-design;
ambient noise is batched.

### Stage 2 — agent (judgment)

The steward is invoked scoped to the target cogmap, drains its queue slice, reads the telos, and judges
*what is worth acting on*. It tends declared structure (concepts / edges / facets / fold-scar) within the
map's visibility scope — fully autonomous + audited — and escalates cross-map promotion-translation to the
HITL gate (deferred; see below). It never clusters or assigns salience.

---

## Config: a three-layer story

Each layer owns what it is uniquely positioned to know.

| Layer | Home | Owns | Rationale |
|---|---|---|---|
| **Preference** | per-cogmap **charter regulation** blocks | *how fresh* this map wants stewardship (eagerness / accumulation bound / cadence preference) | The freshness posture is a property of *what the map is for* — the telos's domain. The map is the only thing that knows whether it is fast-moving operational or slow reference. Born at bootstrap, evolvable by steward/scenarios like any regulation. |
| **Ceiling** | `DeploymentProfile` | operator *cap* — max wake frequency / token budget per period | Neither reference runtime gives a managed spend/overwhelm ceiling for free. A per-cogmap preference must not be able to breach an operator's bound — especially since a bootstrapping agent effectively *authors its own map's eagerness*. The profile bounds; the charter expresses preference within it. |
| **Wiring** | `kb_system_settings` (singleton, `id = 1`) | *where* the ambient drain is driven: `steward_scheduler ∈ {eve, temper}`, default `eve` | This is how the *deployment* is wired, not a per-map property nor a spend ceiling. On Vercel we default to Eve scheduling (Vercel Cron). The singleton instance-config table is the established home for operating-shape settings (`access_mode`, `instance_name`, …). |

**L0 is the trivial case** of the Preference layer: its charter declares ambient wake = never (operator-only).

A self-authored charter cannot grant itself an unbounded budget: the operator ceiling is what makes
"the charter owns its freshness" safe.

---

## How it rides what already exists (mostly additive)

- **Invocation envelope (#156).** Every steward run opens a `kb_invocations` row with
  `trigger_kind ∈ {directed, swept, scheduled}`, `originating_cogmap_id` = target,
  `telos_resource_id` = its charter; it drains the queue slice it was woken for and marks those items
  handled on close (`closed_by_event_id` / `outcome`). `trigger_kind` and `kb_events.invocation_id` already
  exist. Agent-authorship metadata (reasoning + graded-band confidence) rides each authored act, as built.
- **Delegation binding.** Originating-map pin (immutable for the delegation tree),
  `resources_accessible_to_cogmap(originating)`, and `cogmaps_share_a_team(a, b)` reused unchanged. The
  `reach` vs `priming-frame` split is the mechanism by which L0-as-frame works: bootstrappers/stewards are
  *primed with* L0's telos while *bound to* their own map's reach.
- **Seed/scenario DSL.** Seeds birth maps (L0; L1 shells); scenarios evolve them (L0 updates; future
  map-updates). No new declarative format is invented.
- **`DeploymentProfile`.** The thin 3-field object (`runtime`, `residency`, `token_budget`) gains a small
  **ambient-wake ceiling** — the only field addition.

### Genuinely new substrate primitives

1. The L0 **canonical seed** + its content (the "what is temper" charter, facets, edges).
2. The **`stewardship_requested`** event + **stewardship-queue** projection + the deterministic
   wake/debounce evaluator (stage 1).
3. Per-cogmap **freshness regulation** blocks (charter regulation layer).
4. The thin **ambient-wake ceiling** on `DeploymentProfile`, and the `steward_scheduler` setting on
   `kb_system_settings`.

---

## Implementation staging (the design captures all three tiers; the plan stages them)

1. **L0 first.** Smallest, purely deterministic, and the prerequisite for everything: the canonical seed,
   its load at boot through `cogmap_genesis`, and the governance declaration (ambient wake = never). Plus
   the map-update-scenario mechanism (so L0 can evolve from day one). No agent runtime needed to land this.
2. **Steward queue + wake policy.** The `stewardship_requested` event, the queue projection, the
   deterministic wake/debounce evaluator, the three config layers, and the envelope wiring
   (`trigger_kind`). The steward agent itself (Eve/Claude binding) consumes this surface.
3. **L1 interactive bootstrap.** Shell-first genesis, charter-bootstrapping skill over the MCP surface,
   substrate resumability.

Each stage is independently shippable and independently valuable.

---

## Deferred (named, not dropped)

- **Cross-cogmap promotion-translation** — the HITL gate's actual shape; under-specified; a distinct
  future thread. Subsumes the closed cascade-propagation case.
- **The sweeper** — the drift/salience-miss detector; where the parked second confidence axis (`import` /
  salience-claim) pays off; home of the "orthogonal third signal" open question. Undesigned.
- **L0's actual charter content** — *which* temper concepts / facets / edges populate the kernel map — is
  its own authoring task and a dogfood of the charter-bootstrapping skill. (The mechanism is in scope here;
  the content is not.)
- **Eve-vs-temper wake-evaluator mechanics** — the concrete cron/dispatch wiring behind the
  `kb_system_settings.steward_scheduler` switch (Vercel Cron → temper endpoint → POST Eve, vs Eve's own
  schedule as the backstop). An infra detail for the plan, gated by the setting.
- **L0 authored as a first-class seed/scenario YAML loadable in production** — would make `temper-next`'s
  loader a runtime dependency of `temper-api`; pursue only if the worked-example-in-DSL value proves worth
  the runtime-dep cost. L0 ships SQL-native first (see Birth).

---

## Open questions for review

1. **L1 shell-first** — human seeds a one-line telos → shell minted → agent accretes. Chosen to dissolve
   the paradox cleanly. Confirm vs "map crystallizes purely from the conversation."
2. **Staging** — L0 → steward-queue → L1. Confirm this is the right order (L0 is the prerequisite; the
   steward surface is the highest-leverage agent-facing piece; L1 is the richest but depends on both).
3. **`stewardship_requested` granularity** — does it name a *material set* (list of resource refs) per
   event, or one resource per event with the set assembled by the queue? (Leaning: a set per event, so the
   "tag this analysis + the original doc together" gesture is one atomic enqueue.)
