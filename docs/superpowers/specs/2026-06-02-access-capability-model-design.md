# Access & Capability Model: Decomposed Capabilities, Edge-as-Homed-Object, and the Telos-Scope Read-Grant

**Date:** 2026-06-02
**Status:** Design — in brainstorming, pending review
**Goal:** `substrate-kernel-to-cognitive-map`, Arc 1 (shared-kernel completion)
**Gates:** un-gates the PROVISIONAL `kb_resource_access` / `access_level` DDL in
[`2026-06-01-data-model-reconciliation-design.md`](2026-06-01-data-model-reconciliation-design.md) §2.

> **Grounding note.** This spec is written against the **actual built schema** — the base
> `20260330000001_consolidated_schema.sql` plus later migrations (notably
> `20260407000001_system_access_gate.sql`, `20260522000001_event_ledger_unification.sql`,
> `20260522100002_edges_as_projection.sql`) — **not** the 2026-05-27 access-wrapper design doc,
> which proposed the polymorphic-projection *substance* but carried the old `access_level` enum
> forward unreconciled. Where this spec says "built" vs "designed," it has been checked against
> migrations. The producer/consumer backbone (Bedrock #4), the homes-vs-access split, and the
> polymorphic anchors are preserved from 2026-05-27; the capability vocabulary, edge homing, and
> system-gate are redesigned here.

---

## Context

The data-model reconciliation spec slimmed `kb_resources` to identity and moved access into a
two-table wrapper (`kb_resource_homes` for navigation, `kb_resource_access` for grants). It marked
the grant **capability vocabulary** PROVISIONAL: the built `access_level` enum
(`vault | mutable | immutable`) is a vault-era artifact that double-encodes a single write boolean
with `team_role`, is undefined across the polymorphic anchor set, and ignores a second axis the
research surfaced (resolution-permeability). The instruction from the confidence inventory was
explicit: *"Don't defend the enum; defend the recognition that there are (at least) two orthogonal
axes."*

Brainstorming this model surfaced that the question is larger than "rename the enum," and that three
pieces are mutually load-bearing — you cannot define the capability vocabulary without knowing that
**edges are themselves access-gated objects** and that **scopes grant read across teams**. They are
one access model seen from three angles; splitting them would force forward-references in every
direction. Hence one cohesive spec.

A second realization reshaped the second axis. The resolution gradient (data / shape-projection /
interior-think-with) was conceived when a scope was imagined as a **selectively-permeable membrane
around an interior** — resolution measured how far past the wall an outsider reached. The model has
since moved to **scope-as-telos-incubation-space**: concepts are *born of a telos-toward-utility*
(ready-to-hand, not concept-as-ideal), they *home* in a scope but *participate elsewhere through
edges*. There is no membrane left to be permeable. The gradient does not die, but it **relocates
out of the access layer** — into a materialized retrieval surface and a scope-to-scope delegation
relationship.

### What the built schema actually looks like at design time

- **Team RBAC is built and real:** `kb_teams`, `kb_team_members.role` (`team_role` =
  `owner | maintainer | member | watcher`), `kb_team_resources.access_level`
  (`vault | mutable | immutable`), `resources_visible_to`, `can_modify_resource`.
- **The system-access gate is built (R11):** `kb_system_settings` singleton
  (`access_mode` = `open | invite_only`, `gating_team_slug`, terms fields), `kb_join_requests`
  (request→review workflow), `has_system_access(profile)` (open ⇒ true; invite_only ⇒ membership in
  the team whose `slug = gating_team_slug`, **any role**), `is_system_admin(profile)` (membership in
  that team with `role = 'owner'`). Bootstrap: the `general` context is **owned by** the
  `temper-system` team, so membership confers its read.
- **`kb_scopes` exists but is bare:** `(id, name, porosity)` — `porosity` is the built
  `access | attention` enum. Nothing leans on it for access yet.
- **Edges are an event-projection with no home of their own:** `kb_resource_edges` =
  `(id, source_resource_id, target_resource_id, weight, edge_kind, polarity, label,
  asserted_by_event_id, last_event_id, is_folded, created)`. The producing scope rides on the
  asserting `kb_events` row (`scope_id`), which the migration stamped **always-public** for legacy
  edges. The graph functions (`graph_traverse`, `graph_neighbors`, `graph_resource_edges`) gate
  visibility **purely on endpoint visibility** — they `JOIN resources_visible_to ON` the peer
  resource. There is no edge-level access gate anywhere.
- **Designed, not built:** `kb_resource_homes`, `kb_resource_access`, `kb_team_scopes`,
  `kb_teams_parents`, `resources_accessible_to_scope`, an edge home. **The entire
  scope-shape / cognitive-map access story is greenfield** — we are defining the second access world
  for the first time, against a first one that is real.

---

## 1. One access model, two home-flavors

The "team-shape" (human-authored docs: research, decisions, sessions, tasks/goals) and the
"scope-shape" (agent-managed concepts and cognitive maps) are **not two access models**. They are two
**home-anchor types** of the single polymorphic `homes + access` wrapper:

- **Home anchor:** `anchor_table = 'kb_contexts'` (human docs) vs `'kb_scopes'` (concepts).
- **Write-authority:** team-role-held (humans edit team docs) vs **scope-telos-held** — concepts are
  *wholly managed by agents working in the scope's telos*; a human in a working session participates
  but does not author the map. That is the **producer side** of the capability model with a non-human
  grantee: write-authority over a scope-homed concept is **scope-bounded**, held by the scope's
  working-agent, bounded by the scope's telos and its visible set.

Committing to the wrapper does **not** prematurely shape-lock the cognitive map — the wrapper is the
substrate both shapes need.

---

## 2. The capability descriptor (replaces `vault | mutable | immutable`)

Reading `can_modify_resource` and the R4 matrix, the old enum + role were jointly encoding **four
distinct verbs**, with the enum a lossy named-bundle and `team_role` redundantly re-gating the
write/delete bits (`watcher` masks them off). Effective permission was `f(access_level, role)`
computed by special-case SQL (`access_level IN ('vault','mutable') AND role != 'watcher'`). **The
verbs are the model.**

Replace the enum with an **explicit capability set** — the *same descriptor for every gated object*
(resources and edges) and *every anchor type*:

| field                   | meaning                                                            |
|-------------------------|--------------------------------------------------------------------|
| `read`                  | reachability — may see the object at all (binary; depth is **not** here) |
| `write`                 | edit content                                                       |
| `delete`                | destroy / retract                                                  |
| `grant`                 | re-share to other anchors / transfer ownership (the orthogonal, security-sensitive verb) |

**Coherence:** a CHECK enforces `write | delete | grant ⇒ read` (you cannot mutate or re-share what
you cannot read).

**`read_resolution` is collapsed.** Reading is binary reachability. The
data / shape / think-with gradient is **not** a grant field — it falls out *downstream of access*
(§5). This is a deliberate revision of an earlier draft that carried a 4-valued `read_resolution`
ceiling on the descriptor: once scopes stopped being enclosures, a per-grant depth-ceiling lost its
referent.

Why an explicit set over an ordered lattice: a lattice (`read < write < delete < administer`)
re-conflates — it couples `grant` (re-share) to `delete`, exactly the forced bundling we are escaping.
The explicit set keeps each verb independent, makes the role-mask composition (§6) a clean
intersection, and generalizes uniformly to edges and all four anchors. The cost — representable
incoherent sets — is handled by the CHECK and by grants being constructed through typed code.

---

## 3. Edge-as-homed-object

Edges become **first-class access-gated objects, homed in the same resource-terms as everything
else** — the *same* polymorphic `(anchor_table, anchor_id)` home that resources use, **not** an
edge-special `scope_id` column. An edge between two human task-docs homes to a **context**; a
cognitive-map edge homes to a **scope**. One homing vocabulary, applied uniformly, which means
`edges_visible_to` is literally the same *shape* of function as `resources_visible_to` and reads the
same home-gate. (The earlier "nullable `scope_id` on the edge" instinct was the right direction but
too narrow — it special-cased concept-edges and assumed scopes.)

**An edge is an authored assertion, not a transparent function of its endpoints.** Two *public*
concepts can be connected by a *private* edge — e.g. a directors' circle asserts
`sprint-rituals --leads_to--> formalization-mandate`, where both endpoints are pre-existing public
concepts but the *connection, its intent-arrow, and its weight* are the directors' strategic insight.
The current model leaks this (both endpoints visible ⇒ edge visible). Homing the edge fixes it.

**Two distinct protections, kept separate:**

- **edge-home visibility** — gates *seeing the assertion exists*. The directors' edge is homed in
  the directors' scope and is invisible to public-only viewers even though both endpoints are public.
- **endpoint integrity** — you cannot dereference / traverse to a node you cannot see (independent of
  the edge-home check).

This is **Bedrock #4 applied to edges**: the edge is **produced** in the directors' scope
(scope-bounded — the agent may only wire edges among nodes *the scope* can read, via §4) and
**consumed** person-bounded (visible only to those who can see its home). "Scopes home but do not
enclose" becomes exact: a concept's *inbound* edges are homed in their **producers'** scopes and
gated there — the concept's own home never encloses the references pointing into it.

**Storage:** edge-home is derived from the asserting event's producing anchor. Today the scope rides
on `kb_events` (always-public for legacy edges); this promotes it to a **gating home on the edge
projection**. The `graph_*` functions gain the edge-home gate alongside their existing endpoint
joins.

---

## 4. Team↔scope read-grant (`kb_team_scopes`)

The producer-bound that lets an agent working in scope Y **reference (read)** concepts homed in
broader shared scopes — and **bounds what it may wire into Y's map to what Y itself can see, never who
is at the keyboard.** Scope-bounded, **not DAG-expanded** to the individual team memberships of
whoever triggered the work (Bedrock #4: producer-side access is scope-bounded; consumer-side
visibility is person-bounded).

This is the single mechanism behind both halves of cross-scope concept reference:

- **Producer side (concept reuse):** a same-named concept arriving in a specific team's cognitive map
  as a separate UUID-resource can carry a `derives_from` / `extends` edge to the foundational concept
  in a broader scope — letting facets act as **taxonomy without forcing a full ontology**, avoiding
  rebuilding every concept from the ground up. This requires Y's read-grant to the broader scope.
- **Consumer side (connection privacy):** the resulting edge is homed in Y and gated there (§3), so
  the *fact* of the reference does not leak to viewers who can see the foundational concept but not Y.

`kb_team_scopes` associates scopes with one-or-more teams (scopes do **not** "belong to" teams). The
producer predicate `resources_accessible_to_scope` is **not** DAG-expanded across team-memberships on
the producer side.

---

## 5. Scope-as-telos-incubation-space and `kb_scope_proximity`

A scope is **not an enclosure**. It is a concept-incubation space through a telos: all concepts are
*born of a telos-toward-utility* (a tool-using, ready-to-hand phenomenological framing — no a-priori
concept-as-ideal). The read gradient abandoned in §2 reappears here, **downstream of access**, as two
things:

- **`kb_scope_proximity`** — the scope's **self-materialized shape surface**: clusters computed via
  cosine-nearness + relational-edge-weight-and-density + property-nearness, at configurable or
  subjective-agent thresholds of salience. Crucially it is materialized **by the scope, under its own
  telos, by its own agents**, and *read* by others. Self-materialization resolves the
  flattened-average epistemic worry from the resolution-contract: an outsider reads *Y's shape as Y
  computed it*, not a crawl through the outsider's own perspective-vector. Reading the scope's
  telos-framing artifact plus this surface is the **workhorse** — it gives an agent "what it needs to
  make more informed decisions" even when it was not launched *in* the scope (assuming traversal
  access and acting-on-behalf-of visibility), commonly by launching subagents.
- **Scope-to-scope delegation** *(shape recognized here; mechanism deferred)* — to *think-with* a
  scope's interior, an agent is **primed with the target scope's telos** but operates within the
  **delegating** scope's visibility boundary. This is **not a per-resource permission** — it is a
  **scope↔scope relationship** (a homed edge, §3), valid only where a **mutual team bridges the two
  scopes** (the §4 team↔scope join supplies the RBAC constraint). Priming is possible precisely
  because the delegating scope can already *read* the target's telos-framing + materialized shape
  (the §4/§5 read), so the delegated agent borrows the target's *frame* applied to the delegating
  scope's *visible material* — it never exposes the target's private interior. **Safety invariant,
  stated now because it is an access property: delegation must never escalate visibility.** The
  priming runtime, the delegation-edge lifecycle/semantics, and runtime visibility-enforcement are the
  staged **Domain-B** think-with mechanism — deferred to a separate spec.

**This spec's obligation to proximity is bounded.** It must guarantee the access-resolved *inputs*
exist and are coherently gated — chunk embeddings (pgvector) for cosine, **homed** edge-weights for
relational density, and `kb_properties` for property-nearness — so that clustering is *"just compute"*
over already-gated data, needing **no new access primitive**. The clustering algorithm, the
materialization table shape, and the retrieval/query surface are a **separate retrieval spec**, not
this one.

`kb_scopes.porosity` (built `access | attention`) reframes accordingly: it expresses *"is my
materialized shape readable / contributable-to,"* not *"how deep can you crawl."*

---

## 6. Role recast and the `watcher` disentangle

**With the per-resource verbs on the descriptor, `team_role` gets straightforward.** It no longer
encodes a per-resource write boolean; it collapses to two genuinely team-level questions:

1. **Management tier** — may this member add/remove members, create/revoke grants on the team's
   behalf, transfer team ownership? (`owner` / `maintainer` yes; `member` / `watcher` no.)
2. **Member ceiling** — the *maximum* a member may attain of any grant the team holds. `watcher` =
   read-only ceiling (present, observe-only); `member`+ = full ceiling.

Effective per-member capability = `grant-caps ∩ role-ceiling` — a **clean set-intersection**, not the
old special-case SQL. The double-encoding dies because the two relations answer different questions:
*"what is this resource exposed to the team as"* (grant) vs *"what is this member's standing in the
team"* (role). The four role names survive, re-grounded as `(management, ceiling)` pairs; the
`team_role` enum likely stays unchanged.

**The `watcher` overload — three unrelated jobs in the built schema — splits:**

1. **Team read-only tier** — *keep* as `watcher`'s `ceiling = read-only`. The one legitimate meaning.
2. **System-access entitlement** ("approved to use this instance," currently *any* membership in
   `temper-system`) — **extracted from the team model entirely.** It is *prior to* any resource; it
   is not a team-resource-access question. It becomes a **profile-level status**
   (`system_access ∈ none | approved | admin`). The original bug was treating "may use the instance"
   as a kind of resource-membership; the fix **separates the category** even at the cost of the
   "one capability model everywhere" uniformity — the auth gate is genuinely a different layer.
3. **Bootstrap-content recipient** (membership → reads `general`) — becomes a **default read-grant to
   approved profiles** (the `general` context is readable when `system_access ≥ approved`), not a
   side effect of `temper-system` membership.

`is_system_admin` → `system_access = admin`, not team-owner. `has_system_access` /
`is_system_admin` read profile status — which **also drops their `gating_team_slug` dependency, an
item slug-retirement would otherwise have broken.** `kb_join_requests` and `kb_system_settings`
(`access_mode`, terms) stay; the approval workflow's terminal action becomes *set profile status*
rather than *insert a `watcher` membership row*. `temper-system` retires as an access mechanism (it
may survive only as a content home).

---

## 7. The resulting three-layer stack

Three different questions, three mechanisms, no cross-contamination:

1. **System-access gate** — *may you exist here at all* → profile `system_access` status
   (`require_auth` precondition).
2. **Resource / edge / scope access** — *what may you do with this object* → the capability descriptor
   on `kb_resource_access` grants, intersected with `role-ceiling` for team anchors.
3. **Resolution / delegation** — *how deep* → reachability + the `kb_scope_proximity` retrieval surface
   + scope-to-scope delegation (shape in §5; mechanism deferred).

---

## 8. Default-safety and scope lifecycle

*"The safe thing must be the easy thing."* The **emergent default scope** a person receives before
they have reasoned about scopes at all is a **first-class design object**, and it **defaults closed**
(produces nothing into shared scopes until explicitly granted; consumes only what the person can see).
A naive default here turns the emergent-scope story into an incident report.

**North star (forward-guidance, not a requirement of this spec):** scopes are as durable or as
ephemeral as they *earn*. Shallow-cloning a scope to expand, refine, or generalize its telos should be
cheap — the concept-home stays with the **original** scope; the clone carries edges **back** to those
homed concepts. This needs no special affordance: a clone is a new scope, and edges-back are exactly
the §3 (edge-as-homed-object) + §4 (team↔scope read-grant) pattern already designed. Recorded so the
access model stays forward-compatible with it, not to build it now.

---

## DDL delta (grounded against the built schema)

**New**
- capability-descriptor fields on `kb_resource_access` (replaces `access_level`)
- `kb_resource_homes`, `kb_resource_access` (the wrapper, from the data-model spec, now with the
  descriptor)
- edge-home columns on `kb_resource_edges` (`anchor_table`, `anchor_id`), projected from the asserting
  event
- `kb_team_scopes` (scope↔team association); `kb_teams_parents` (teams DAG)
- `edges_visible_to(...)` function (same shape as `resources_visible_to`)
- `resources_accessible_to_scope(...)` producer predicate (scope-bounded, not DAG-expanded)
- `kb_profiles.system_access` (`none | approved | admin`)

**Changed**
- `kb_team_resources` → subsumed into `kb_resource_access`
- `has_system_access` / `is_system_admin` → read profile status; drop `gating_team_slug` dependency
- `graph_traverse` / `graph_neighbors` / `graph_resource_edges` → add the edge-home gate alongside the
  existing endpoint joins
- `team_role` semantics re-grounded as `(management, ceiling)` (enum likely unchanged)

**Retired**
- `access_level` enum (→ descriptor)
- `watcher`-as-system-access; `temper-system`-as-access-mechanism
- the `gating_team_slug` dependency in the system-gate functions

---

## Open questions (refine during the plan; not blockers)

1. **Descriptor's literal DDL shape** — discrete boolean columns vs a composite type vs a small
   bitmask. *Lean:* explicit boolean columns on `kb_resource_access`, readable directly in SQL.
2. **Edge-home storage** — denormalized home columns on `kb_resource_edges` (query-cheap) vs
   join-through-asserting-event (normalized, no drift). *Lean:* denormalized, projected from the event
   at assertion time.

---

## Out of scope

**Rejected (load-bearing decisions — resist re-litigation):**
- **read-resolution as a graded grant ceiling.** Collapsed into binary reachability + the
  `kb_scope_proximity` retrieval surface + scope-to-scope delegation. The membrane-permeability image
  it rested on was superseded by scope-as-telos-incubation-space.
- **system-access as resource-membership.** The original R11 bug; system-access is a separate layer
  (profile status), deliberately *not* unified into the capability model.
- **`access_level` as an ordered lattice.** Re-conflates `grant` with `delete`; the explicit
  capability set is the decomposition.

**Deferred (in scope elsewhere or later):**
- the `kb_scope_proximity` **clustering algorithm**, materialization table, and retrieval/query
  surface — a separate retrieval spec.
- the **scope-to-scope delegation mechanism** — the priming runtime, the delegation-edge
  lifecycle/semantics, and runtime visibility-enforcement (the staged Domain-B think-with mechanism).
  Its *shape* and the never-escalate-visibility safety invariant are stated in §5; the mechanism is a
  separate spec.
- **shallow-clone / scope-lifecycle** affordances — north star only (§8).
- **ResourceRef / CLI-UX** rework from slug-retirement — already a tracked follow-up in the data-model
  spec.
- **migration sequencing** (spine #3) — the data-model and access specs share a plan that owes the
  build order (Limb 1c → extract `temper-substrate` → birth `temper-cogmap`).

---

## Connections

- **Gates / un-gates:** [`2026-06-01-data-model-reconciliation-design.md`](2026-06-01-data-model-reconciliation-design.md) §2 (the PROVISIONAL access tables)
- **Charter:** decision `2026-06-01-the-shared-kernel-boundary-temper-substrate-beneath-two-domains-workflow-kb-and-cognitive-map`
- **Preserves substance of:** decision `2026-05-27-access-wrapper-extraction-and-polymorphic-projection-substrate`
- **Research grounding:** `2026-05-31-temper-confidence-inventory` (Bedrock #4; porosity Drifted #19 — "two orthogonal axes"), `2026-05-29-resolution-contract-and-the-permeable-scope-surface` (the resolution tiers; perspective-paired; permeable surface), `r4-crate-architecture-auth-access-control` (`access_level`/`team_role` origin), `2026-04-07-r11-system-access-gate-and-owner-scoped-uris` (the `watcher` overload)
- **Tool-use reflection from this session:** research `2026-06-02-temper-as-research-substrate-what-the-access-capability-deep-research-run-proved-and-what-it-didn-t` (the doctype-coupling friction is live evidence for slug-retirement)
- **Goal:** `substrate-kernel-to-cognitive-map`, Arc 1
