# Map-to-Map Delegation: A Dissolution (`maps_share_a_team`)

**Date:** 2026-06-02
**Status:** Design — **reviewed** (2026-06-03). The dissolution is sound; **all review opens resolved.**
**CS-1** (read-API `principal` sum type — access §4; §3 never-escalate binds `principal = Map(originating)`)
and **A4-1** (§2/§4: `maps_share_a_team` gates *priming* of the blurred frame; the team-intersection gates
*material* — two gates, not an equivalence) both resolved. Ready for `approved/pending-plan`.
**Goal:** `substrate-kernel-to-cognitive-map`, Arc 1 (shared-kernel completion)
**Spun out of:** [`2026-06-02-access-capability-model-design.md`](2026-06-02-access-capability-model-design.md) §5
(the "scope-to-scope delegation mechanism" deferred to "a separate spec").
**Depends on:** [`2026-06-02-map-regions-self-materialized-shape-surface-design.md`](2026-06-02-map-regions-self-materialized-shape-surface-design.md)
(priming reads the `kb_map_regions` surface) and that spec's `scope → map` reframe.

> **Grounding note.** Written against the built schema (the `kb_scopes`→`kb_maps` entity, the
> `kb_team_*` RBAC, the `kb_events` ledger) and the two sibling specs it composes with (access/capability
> and map-regions). It adds **one substrate function and nothing else** — no tables, no enum, no columns.
> The bulk of this spec is a *negative result*: the stored delegation object §5 reached for does not
> survive the move from scope-as-membrane to map-as-telos-home, and recording why is its main value.

---

## Context

The access/capability model (§5) recognized the *shape* of map-to-map delegation — an agent
**primed with a target map's telos** but **bounded by the delegating map's visibility** — and deferred
"the priming runtime, the delegation-edge lifecycle/semantics, and runtime visibility-enforcement" to a
separate spec, tagging them "the staged **Domain-B** think-with mechanism." This is that spec, at the
**access-layer altitude** decided in brainstorming: it owns the delegation *relationship* and the
*never-escalate enforcement contract*, and treats the think-with agent runtime (telos-frame injection,
the think-with loop, subagent orchestration) as an **opaque Domain-B consumer** — the `temper-llm` /
scope-bound-triage-agent track. Symmetric with the map-regions spec's "black-box agent" altitude, which
keeps Arc-1 work in the kernel.

§5's own framing called the relationship "a homed edge (§3)." Pressure-testing that against the
already-decided model dissolved it. The result is smaller and more correct than a stored grant.

---

## 1. The dissolution: there is no stored delegation object

**§5's "homed edge" is rejected.** A stored delegation grant — a `kb_map_delegations` row, or a
`delegates_to` edge — is a **scope-as-membrane vestige**. When a scope was a selectively-permeable
*interior*, you needed a grant to "cross in" to another scope. The model discarded that: a **map homes
concepts but has no interior** (map-regions spec §0/§3; access spec §3). With no interior, **there is
nothing to grant**.

Two further reasons the stored object actively misleads:

- **Legitimacy is a live predicate, not a frozen pointer.** A stored `authorizing_team_id` would name
  *one* bridging team; but legitimacy is "*some* team bridges the two maps," and *which* team is
  irrelevant — if the named team leaves while another bridge survives, delegation is still legitimate,
  yet the stored pointer is now stale and wrong. The fact to be checked is the **non-emptiness of
  `teams(M1) ∩ teams(M2)`**, evaluated at the moment of use.
- **Emergent and self-revoking is safer than curated.** With no stored grant, delegation legitimacy
  tracks team structure automatically: drop the team bridge and the next launch's predicate is simply
  false. There is no separate delegation-permission to forget to revoke — *"the safe thing is the easy
  thing"* (access spec §8).

Provenance does not need the object either (§6).

---

## 2. Authorization — the live predicate `maps_share_a_team`

The one thing this spec adds to the substrate:

```
maps_share_a_team(map_a uuid, map_b uuid) -> bool      -- (teams(a) ∩ teams(b)) ≠ ∅, evaluated live
```

It is:

- **symmetric** — the bridge exists or it does not; delegation may be primed in either direction.
- **team-identity-agnostic** — *any* surviving bridge suffices. There is no "the authorizing team," so
  the "which team authorizes this session?" question simply does not arise (§5 of the access model
  worried about it; the answer is that the question was malformed).
- **the priming authz-prior — *not* a producer-read equivalence** — it gates **priming**, not material
  reads, and the two are different conditions. `maps_share_a_team(a,b)` (∃ *one* bridge) is strictly
  *weaker* than "an agent in `a` may producer-read `b`'s concepts" (= `resources_accessible_to_map(a)`,
  the access §4 team-**intersection**, visible to *every* team of `a`). The bridge legitimizes the launch
  tooling **injecting `b`'s *frame*** — its `telos_resource` + *blurred* region surface (§4) — because
  those are shareable frame artifacts the bridge team can already read; it does **not** authorize reading
  `b`'s *material*. The agent's every substrate read stays bound to
  `resources_accessible_to_map(originating)` (§3). So it introduces **no new access primitive**: it is a
  named, live, identity-agnostic surfacing of the team-bridge existence check the launch tooling invokes
  as its **authz-prior** — with material access bounded *separately* by the intersection.

(The thin speculative-surface cost of naming a derivable predicate is paid deliberately: the launch gate
should be one legible call, not an inferred consequence, and this is the natural place to assert the
live-not-stored property.)

---

## 3. `never-escalate` — an immutable, root-bound visibility binding

The single real enforcement contract. At **root launch** the agent is bound to its **originating map**
— the map in whose telos it was launched (one always exists: the access spec §8 emergent-default map if
nothing more specific). That binding is **immutable for the entire delegation tree**:

- Every descendant subagent, however deeply primed, resolves **all** substrate reads through
  `resources_accessible_to_map(originating)`. The priming *frame* varies per subagent; the *visibility
  root* never moves.
- **Transitively:** a subagent primed by `M2` may spawn one primed by `M3` iff
  `maps_share_a_team(originating, M3)` — the **originating** map is the subject of *every* authz check
  and *every* read, never the immediate parent. Binding-to-root, not binding-to-parent, is what makes the
  guarantee hold down an arbitrary chain: visibility can only ever stay equal or shrink-to-implicit,
  **never widen**.

This is the access-property the access spec stated as an invariant ("delegation must never escalate
visibility") and §5 deferred here as "runtime visibility-enforcement." Its kernel-side expression is
just: the substrate read API already takes the producer-map as a parameter; the launch binds it to the
originating map and the tooling never re-binds it. Hard-enforceable as an **authz-prior at agent launch**.

---

## 4. Priming material is already-gated surface

A delegated subagent is primed with the target map's:

- **`telos_resource`** — read like any resource (map-regions spec §0: the telos *is* a resource).
- **`kb_map_regions` surface** — via the proximity spec's `map_shape_visible_to` (centroid / salience /
  label / count).

Both priming reads are authorized by **`maps_share_a_team(originating, target)`** — the bridge, checked by
the launch tooling, which injects the frame (telos + *blurred* surface). This is **not** the originating
map's full producer-read (the §4 intersection): the bridge is deliberately *weaker* and gates only the
shareable frame, while the agent's actual *material* reads stay bound to
`resources_accessible_to_map(originating)`. The subagent therefore borrows the target's *frame* and reasons
through it over **originating's** visible material — never the target's private interior.

The interior stays protected **automatically**, with no work in this spec: when the primed agent
dereferences a region *member* of the target's shape, that deref resolves through
`resources_accessible_to_map(originating)` (map-regions §3: members are dereferenced through ordinary
resource access, never returned wholesale in the surface). So it sees a member only if the originating
map already could (e.g. a genuinely shared concept), and **never** a target-private one. The
surface/interior line in the map-regions spec is what lets never-escalate hold on the interior for free;
this spec only has to bind the root (§3).

---

## 5. The frame-of-reference confusion, resolved

Three things were being conflated when delegation was imagined as a stored, team-authorized edge:

| concept | what it is | role |
|---|---|---|
| **telos** | a map's cognitive frame ("whose way of seeing") | the priming material |
| **team-bridge** | `maps_share_a_team` (RBAC predicate) | the launch authz-prior |
| **originating map** | the map the root agent launched in | the immutable visibility root |

The agent **never identifies *as a team***; it works in a *map's telos* with *that map's visibility*.
Telos is cognitive, the team-bridge is RBAC, the originating map is the visibility root — three distinct
things meeting only as a launch precondition. The metaphors stop mixing, which is what made the stored
`authorizing_team_id` feel both necessary and unanswerable. It was neither.

---

## 6. Provenance via the event ledger

A delegated launch is an **event** — originating map, target map(s), and the bridge(s) observed at
launch — not a standing row. This is consistent with the data-model spec's "provenance is answered by the
event ledger, not a tier column," and it means the audit trail of who-thought-with-what exists without
re-introducing the stored object §1 rejected.

---

## Substrate / DDL delta

**New**
- `maps_share_a_team(map_a, map_b) -> bool` — the live team-bridge predicate (the launch authz-prior).

**Nothing else.** No tables, no enum, no columns. The never-escalate binding (§3) is a launch-tooling
property over the existing `resources_accessible_to_map`-parameterized read API; provenance (§6) is the
existing ledger.

---

## Open questions (refine during the plan; not blockers)

1. **`maps_share_a_team` implementation** — a standalone SQL function vs. an inlined existence-check
   inside the launch authz path. *Lean:* a named SQL function, so the authz-prior is one legible call and
   the live-not-stored guarantee has a single home.
2. **Delegated-launch event type** — whether it reuses an existing event type with a payload discriminator
   or earns its own `event_type` row. *Lean:* its own event type, for clean audit querying; plan-level.
3. **`maps_share_a_team` — priming gate, not producer-read equivalence (A4-1)** — **RESOLVED (§2, §4):**
   the predicate gates **priming** (the launch tooling injecting target's telos + *blurred* region surface;
   ∃ one bridge suffices — shareable frame artifacts), **not** material reads. Material stays bound to
   `resources_accessible_to_map(originating)` = the access §4 team-intersection (strictly stronger). Two
   gates — bridge-for-frame, intersection-for-material — not one equivalence. No new primitive.
4. **Read-API dual parameterization (CS-1)** — **RESOLVED (access spec §4 / map-regions OQ-7):** the
   substrate read-API takes a `principal` sum type `Profile | Map`. §3's never-escalate binds
   `principal = Map(originating)`, so the producer-map *is* the read parameter and the root-binding has a
   concrete surface. The §3 transitivity (every descendant read resolves through
   `resources_accessible_to_map(originating)`) is exactly "the principal never re-binds off the root map."

---

## Out of scope

**Rejected (load-bearing — resist re-litigation):**
- **A stored delegation object** (`kb_map_delegations` table or a `delegates_to` homed edge) — the
  scope-as-membrane vestige (§1). §5's "a homed edge" framing is explicitly superseded here.
- **A stored `authorizing_team_id`** — legitimacy is a live, identity-agnostic predicate, not a frozen
  pointer (§1, §2).
- **Binding a delegated subagent's visibility to its immediate parent map** — it binds to the *root*
  originating map; parent-binding would let visibility widen down a chain (§3).

**Deferred (in scope elsewhere — the black-box Domain-B think-with runtime):**
- the **launch tooling** itself, **telos-frame injection** into agent context, the **think-with loop**,
  and **subagent orchestration** — the `temper-llm` / scope-bound-triage-agent track. This spec hands that
  runtime its **two contracts**: (1) call `maps_share_a_team(originating, target)` as the authz-prior;
  (2) bind every read to the originating map, immutably, for the whole delegation tree.

---

## Connections

- **Spun out of:** [`2026-06-02-access-capability-model-design.md`](2026-06-02-access-capability-model-design.md) §5 (the deferred delegation mechanism); **supersedes** its "a homed edge" framing for delegation.
- **Depends on:** [`2026-06-02-map-regions-self-materialized-shape-surface-design.md`](2026-06-02-map-regions-self-materialized-shape-surface-design.md) (priming reads the `kb_map_regions` surface; the §3 surface/interior line gives never-escalate-on-the-interior for free), and its `scope → map` reframe.
- **Composes with:** [`2026-06-01-data-model-reconciliation-design.md`](2026-06-01-data-model-reconciliation-design.md) (`resources_accessible_to_map` producer-read, the event ledger, the crate topology — `temper-substrate` owns `maps_share_a_team`; `temper-cogmap` / `temper-llm` consume it).
- **Hands contracts to:** the `temper-llm` redesign task (`2026-05-28-redesign-temper-llm-temper-llm-smoke-for-the-scope-bound-triage-agent-workflow`) — the think-with runtime that enforces the two contracts.
- **Research grounding:** `2026-05-29-resolution-contract-and-the-permeable-scope-surface` (the think-with tier this dissolves), `2026-06-01-seed-skill-scope-portable-vs-bound-awareness-access-bounded` (scope-bound agent launch; awareness-is-access-bounded), `2026-05-31-temper-confidence-inventory` (Bedrock #4 producer/consumer).
- **Goal:** `substrate-kernel-to-cognitive-map`, Arc 1
