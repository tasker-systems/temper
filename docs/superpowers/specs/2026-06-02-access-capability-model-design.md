# Access & Capability Model: Decomposed Capabilities, Edge-as-Homed-Object, and the Telos-Scope Read-Grant

**Date:** 2026-06-02
**Status:** Design — **reviewed** (2026-06-03). Sound; **all review opens resolved.** **CS-1** (§4: read-API
`principal` sum type; `resources_accessible_to_cogmap` = DAG-expanded least-privilege team-intersection,
closed on empty-join), **CS-2** (§5 amendment banner), **A2-1** (§6/§4: `temper-system` as teams-DAG root
content-home + virtual root-membership from `system_access` + symmetric public cogmap), **A2-2** (§1/§2:
principal-as-write-actor; map-home confers concept read/write/delete; `grant` is teams-RBAC-only — not
present on concept/edge), **A2-3** (§4: DAG down-only), **A2-4** (§3: edge-gate AND-composition), **A4-1**
(delegation §2/§4: priming-vs-material gate). **Ready for `approved/pending-plan`.**
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
**edges are themselves access-gated objects** and that **cogmaps grant read across teams**. They are
one access model seen from three angles; splitting them would force forward-references in every
direction. Hence one cohesive spec.

A second realization reshaped the second axis. The resolution gradient (data / shape-projection /
interior-think-with) was conceived when a scope was imagined as a **selectively-permeable membrane
around an interior** — resolution measured how far past the wall an outsider reached. The model has
since moved to **map-as-telos-incubation-home**: concepts are *born of a telos-toward-utility*
(ready-to-hand, not concept-as-ideal), they *home* on a map but *participate elsewhere through
edges*. There is no membrane left to be permeable. The gradient does not die, but it **relocates
out of the access layer** — into a materialized retrieval surface and a map-to-map delegation
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
- **`kb_cogmaps` exists but is bare:** `(id, name, porosity)` — `porosity` is the built
  `access | attention` enum. Nothing leans on it for access yet.
- **Edges are an event-projection with no home of their own:** `kb_resource_edges` =
  `(id, source_resource_id, target_resource_id, weight, edge_kind, polarity, label,
  asserted_by_event_id, last_event_id, is_folded, created)`. The producing cogmap rides on the
  asserting `kb_events` row (`scope_id`, the built producing-anchor column), which the migration stamped **always-public** for legacy
  edges. The graph functions (`graph_traverse`, `graph_neighbors`, `graph_resource_edges`) gate
  visibility **purely on endpoint visibility** — they `JOIN resources_visible_to ON` the peer
  resource. There is no edge-level access gate anywhere.
- **Designed, not built:** `kb_resource_homes`, `kb_resource_access`, `kb_team_cogmaps`,
  `kb_teams_parents`, `resources_accessible_to_cogmap`, an edge home. **The entire
  map-shape / cognitive-map access story is greenfield** — we are defining the second access world
  for the first time, against a first one that is real.

---

## 1. One access model, two home-flavors

The "team-shape" (human-authored docs: research, decisions, sessions, tasks/goals) and the
"map-shape" (agent-managed concepts and cognitive maps) are **not two access models**. They are two
**home-anchor types** of the single polymorphic `homes + access` wrapper:

- **Home anchor:** `anchor_table = 'kb_contexts'` (human docs) vs `'kb_cogmaps'` (concepts).
- **Write-authority:** team-role-held (humans edit team docs) vs **map-telos-held** — concepts are
  *wholly managed by agents working in the map's telos*; a human in a working session participates
  but does not author the map. That is the **producer side** of the capability model with a non-human
  grantee: write-authority over a cogmap-homed concept is **map-bounded**, held by the map's
  working-agent, bounded by the map's telos and its visible set.

Committing to the wrapper does **not** prematurely shape-lock the cognitive map — the wrapper is the
substrate both shapes need.

**How the agent *holds* that authority (resolves A2-2).** The `principal` (§4, CS-1) is the actor on
**writes** as well as reads. The authority decomposes into two levels, neither a per-concept grant row:

- **Map-as-object** capabilities derive from `kb_team_cogmaps` + the §6 role-ceiling — no bespoke per-map
  grant: *read M's shape* = any joined-team member; *write / contribute to M* (may launch an authoring
  agent in M's telos) = member+ ceiling; *manage M* (its team-joins) = owner/maintainer.
- **Concept-level** `read` / `write` / `delete` are conferred to the agent-principal `Cogmap(M)` by the
  **map-home itself** — the exact symmetry of "a context-home confers to the owning team," now "a map-home
  confers to the map's working agent." Producer-bounded: the agent may only reference (wire edges to)
  nodes `M` can read (§4). There is **no `grant`** at the concept level (§2). Humans hold map-as-object
  `write` and *launch* the agent; per the paragraph above they never directly author concepts.

---

## 2. The capability descriptor (replaces `vault | mutable | immutable`)

Reading `can_modify_resource` and the R4 matrix, the old enum + role were jointly encoding **four
distinct verbs**, with the enum a lossy named-bundle and `team_role` redundantly re-gating the
write/delete bits (`watcher` masks them off). Effective permission was `f(access_level, role)`
computed by special-case SQL (`access_level IN ('vault','mutable') AND role != 'watcher'`). **The
verbs are the model.**

Replace the enum with an **explicit capability set**. `read` / `write` / `delete` apply uniformly to
*every* gated object and *every* anchor type; `grant` is restricted to objects with **object-local
reachability** (see the note after the table):

| field                   | meaning                                                            |
|-------------------------|--------------------------------------------------------------------|
| `read`                  | reachability — may see the object at all (binary; depth is **not** here) |
| `write`                 | edit content                                                       |
| `delete`                | destroy / retract                                                  |
| `grant`                 | manage the object's **reachability** — add/remove sharing anchors, transfer ownership (the orthogonal, security-sensitive verb). **Not universal** — present only where reachability is object-local; see below |

**Coherence:** a CHECK enforces `write | delete | grant ⇒ read` (you cannot mutate or re-share what
you cannot read).

**`grant` is not a universal verb (resolves A2-2).** It manages an object's *reachability*, which is
object-local only for some kinds — so `read`/`write`/`delete` are universal but `grant` is not:

- **Human docs** (resources homed in a `kb_context`): reachability = the object's `kb_resource_access`
  anchors. `grant` = the existing **teams-level-RBAC** resource sharing (individual→team, team→team) /
  ownership transfer. **Present.**
- **Maps** (`kb_cogmaps` as objects): reachability = `kb_team_cogmaps` joins. `grant` = manage the map's
  team-joins / exposure (the §6 management tier). **Present** (reinterpreted as join-management).
- **Concepts** (resources homed in a `kb_map`) **and edges**: reachability is *not* object-local — a
  concept's reach is inherited from its home-map; an edge is the producer's assertion gated by its own
  home (§3). There is nothing for an owner to grant, so the **`grant` bit is simply not present** on
  concept-grants or edge-grants. (DDL realization — omit the column for those anchor/object kinds vs. a
  CHECK forbidding `grant=true` — is plan-level.) Cross-map concept availability is achieved by
  **forward-translation** (a derived concept homed in a broader map + a `derives_from`/`extends` edge, §4)
  or **consumer-pulled reference** (an edge homed in the consumer's cogmap, gated by read-reach) —
  both are `write`s, never grants. This is why cogmaps are cheap and safely ephemeral: spinning one up
  is pure write+reference, with no reachability-grants to set up or forget to tear down (§8).

> **⚠ Reconciliation item A2 — restate the `kb_resource_access` grantee-anchor set (added 2026-06-04,
> coherence pass).** This spec un-gates the PROVISIONAL `kb_resource_access` DDL in the data-model spec §2
> but only replaced the *capability vocabulary* (the descriptor, §2 below); it never restated the
> **grantee-anchor set**. The model resolved here implies grants are teams-RBAC (team/profile anchors) and
> that **maps read via the team-intersection, not per-resource grants** — so the data-model §2 check that
> admits `kb_cogmaps` as a `kb_resource_access` anchor is stale. Reconcile the `anchor_table`
> CHECK to the teams-RBAC anchor set when the DDL is written. (Reciprocal note lives in data-model §2.)

**`read_resolution` is collapsed.** Reading is binary reachability. The
data / shape / think-with gradient is **not** a grant field — it falls out *downstream of access*
(§5). This is a deliberate revision of an earlier draft that carried a 4-valued `read_resolution`
ceiling on the descriptor: once cogmaps stopped being enclosures, a per-grant depth-ceiling lost its
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
cognitive-map edge homes to a **cogmap**. One homing vocabulary, applied uniformly, which means
`edges_visible_to` is literally the same *shape* of function as `resources_visible_to` and reads the
same home-gate. (The earlier "nullable `scope_id` on the edge" instinct was the right direction but
too narrow — it special-cased concept-edges and assumed cogmaps.)

**An edge is an authored assertion, not a transparent function of its endpoints.** Two *public*
concepts can be connected by a *private* edge — e.g. a directors' circle asserts
`sprint-rituals --leads_to--> formalization-mandate`, where both endpoints are pre-existing public
concepts but the *connection, its intent-arrow, and its weight* are the directors' strategic insight.
The current model leaks this (both endpoints visible ⇒ edge visible). Homing the edge fixes it.

**Two distinct protections, kept separate:**

- **edge-home visibility** — gates *seeing the assertion exists*. The directors' edge is homed in
  the directors' cogmap and is invisible to public-only viewers even though both endpoints are public.
- **endpoint integrity** — you cannot dereference / traverse to a node you cannot see (independent of
  the edge-home check).

**How they compose (A2-4): AND, for traversal.** Both gates are independent and *both* must pass to use an
edge in a walk. Invert the directors' example to see the second gate alone: a **public** edge
`onboarding-guide --leads_to--> q3-restructure-memo`, where the memo is homed in a private exec context. A
public viewer **sees the edge exists** (edge-home public) but the target is **opaque and non-traversable** —
they cannot dereference the memo or step to it (endpoint integrity). They learn "the guide leads
*somewhere*," never what. So a graph walk follows an edge only when its home is visible **and** the
destination endpoint is independently readable; an edge into an unreadable endpoint is a visible-but-dead
arrow, never a path.

This is **Bedrock #4 applied to edges**: the edge is **produced** in the directors' cogmap
(map-bounded — the agent may only wire edges among nodes *the cogmap* can read, via §4) and
**consumed** person-bounded (visible only to those who can see its home). "Cogmaps home but do not
enclose" becomes exact: a concept's *inbound* edges are homed in their **producers'** cogmaps and
gated there — the concept's own home never encloses the references pointing into it.

**Storage:** edge-home is derived from the asserting event's producing anchor. Today the producing cogmap rides
on `kb_events` (always-public for legacy edges); this promotes it to a **gating home on the edge
projection**. The `graph_*` functions gain the edge-home gate alongside their existing endpoint
joins.

---

## 4. Team↔cogmap read-grant (`kb_team_cogmaps`) and the producer read-reach

`kb_team_cogmaps` associates a cogmap with **one-or-more teams** (cogmaps do **not** "belong to" a
team). It is the producer-bound that lets an agent working in cogmap Y **reference (read)** concepts
homed elsewhere — and **bounds what it may wire into Y to what Y itself can see, never who is at the
keyboard** (Bedrock #4: producer-side access is map-bounded; consumer-side visibility is
person-bounded).

### The two principals (resolves CS-1)

A substrate read carries **one principal**, a sum type:

- **`Profile(uuid)`** — a *person* reading. Gated by `resources_visible_to` (consumer axis,
  person-bounded). This is the built path.
- **`Cogmap(uuid)`** — an *agent producing in a cogmap*. Gated by `resources_accessible_to_cogmap`
  (producer axis, map-bounded). The agent **cannot** pass a profile, so "never who is at the keyboard"
  is **structural**, not a discipline. Any clamp of an agent's *output* to the launching person's
  visibility is a Domain-B runtime concern, **not** a kernel read-gate.

Both arms gate the **same** read surfaces — raw search (FTS+vector) and graph traversal alike — so an
agent's search and edge-walks are bounded identically to its direct reads.

### `resources_accessible_to_cogmap` — the least-privilege team-intersection

```
resources_accessible_to_cogmap(resource, M)  ≡  resource ∈   ⋂   vis(T)
                                                          T ∈ teams(M)
```

where `vis(T)` is team T's visibility **DAG-expanded down `kb_teams_parents`** (see "down-only" below)
and `teams(M)` is M's `kb_team_cogmaps` set.

- **Intersection, not union — and it is *forced*, not a preference.** M's shape (regions, homed edges)
  is readable by **every** team joined to M (§3, map-regions §4). So if M's agent read a resource only
  *one* joined team could see and wired it into M, the *other* joined teams would learn it by reading
  M's shape — a cross-team leak. Intersection is the only bound that closes it: M may incorporate only
  the **common ground** of its joined teams. Dual invariant: under intersection, **M's interior is
  exactly the common ground of its joined teams**, so any joined-team member can fully dereference M's
  whole shape with no dangling unreadable references.
- **Empty-join ⇒ closed (hard invariant).** `⋂` over the empty set is the *universe* — exactly
  backwards. `teams(M) = ∅ ⇒ resources_accessible_to_cogmap(M) = ∅`. The emergent-default cogmap
  (§8) thus reads nothing shared until joined to ≥1 team — default-closed, by construction.
- **Single-team join** degenerates correctly to `vis(T)`.
- **"More teams = narrower reach" is deliberate.** Joining a map to more teams *shrinks* its read-reach
  to the overlap — the point of a bridge is to operate over common ground. Need more reach → join fewer
  teams, or spin a sub-map. Do **not** "fix" this into a union; union reopens the leak.
- **Not the keyboard-person's memberships.** The intersection is over M's *joined teams*, never unioned
  with the launching person's own team memberships. (This is the original "not DAG-expanded across
  team-memberships" line, now precise: the **`kb_teams_parents`** DAG expands `vis(T)`; the *person's*
  membership set does not enter at all.)

### `vis(T)` is DAG-expanded **down-only** (resolves A2-3)

`kb_teams_parents` inherits **down**: a descendant (more specific) team sees its **ancestor**
(more general / umbrella) team's grants; an ancestor gains **no** visibility into a descendant's
private material. *"More general should never have expected visibility into the more specific."* A
shared ancestor's grants therefore survive the intersection and become the bridge's common ground.

> **Worked example — producer-vs-consumer, and intersection-with-DAG.** Teams `epd-team-a` and
> `epd-team-b` are both children of sibling parents `epd-department` and `org-common`. `map-c` is
> joined to `{a, b, c}`. Separately `a` is joined to `map-Y`, `b` to `map-Zed`, and the hinge team `c`
> to **both** `Y` and `Zed`.
> - A **person** in `c` can read the shapes of `c`, `Y`, and `Zed` — `c` is joined to all three
>   (consumer / person axis).
> - An **agent** launched into `map-c` reads as `Cogmap(c)` = `vis(a) ∩ vis(b) ∩ vis(c)`. `Y`'s concepts
>   are in `vis(a)` and `vis(c)` but **not** `vis(b)` (b isn't joined to `Y`, and inherits no join-to-Y
>   from its ancestors), so `Y` falls out of the intersection; symmetrically `Zed` falls out via `a`.
>   **The agent in `map-c` sees neither `Y` nor `Zed`** — the hinge is a hinge for *people*,
>   deliberately not for the map's agent.
> - DAG-expansion is what lets `epd-department` / `org-common` grants (present in *both* `vis(a)` and
>   `vis(b)`) survive the intersection: the bridge reasons over org/department commons while every
>   initiative- and team-private concept stays out.

### The two halves of cross-map reference

- **Producer side (concept reuse):** a same-named concept arriving in a team's map as a separate
  UUID-resource can carry a `derives_from` / `extends` edge to the foundational concept in a broader
  cogmap — facets as **taxonomy without a forced ontology**. Requires M's read-reach (the
  intersection above) to that broader material.
- **Consumer side (connection privacy):** the resulting edge is homed in M and gated there (§3), so the
  *fact* of the reference does not leak to viewers who can see the foundational concept but not M.

### Boundary of responsibility — what the RBAC model is **not** for

The down-only rule means the access model intentionally has **no upward audit / leadership roll-up.**
That is a deliberate non-goal, not a gap:

- **Postgres is the persistence layer, and Postgres RBAC is out of our domain.** Admin-level Postgres
  access *intrinsically* confers system-level read-all of every cogmap. Compliance / legal / audit
  ("who said what, where, and what did we write down") and leadership hard-questions-time review are
  therefore **extra-system-access** questions answered at the database / operations boundary — **not**
  internal RBAC modeling questions. Building "umbrella team sees all sub-team work" into the teams-DAG
  would re-implement, badly, an audit capability that already exists one layer down.
- This does **not** remove the need to **intentionally forward-and-translate** concepts/resources into
  higher-order / higher-access contexts. That is fully supported — via the producer-side concept-reuse
  edge above and the shallow-clone north star (§8) — but it is an **explicit authored act** governed by
  organizational topology, transparency norms, and honesty expectations, **not** an automatic
  visibility a more-general team is owed by structure.

### The root team and the public cogmap (the universal read-floor)

`temper-system` is the **teams-DAG root** — every team descends from it (§6 repurposes it from access-gate
to content-home). Two symmetric homes give "system-public" content one uniform mechanism across both axes,
no new primitive:

- **Resources / contexts** granted to / homed in the root sit in `vis(root)`, hence (down-only
  inheritance) in *every* `vis(T)`, hence in *every* map's intersection and every person's reach.
  `general` lives here.
- **Cognitive-map content** gets the mirror: a **system-default cogmap joined to *only* the root team**
  — a perfect overlap with "everyone." Its concepts/edges sit in `vis(root)`, so they're universally
  readable by every agent (via the DAG) and every person (via membership). It homes the truly-public,
  foundational concepts any team's map may `derives_from`/`extends` (the concept-reuse pattern above, at
  the public tier). Its own producer-reach is `vis(root)` = exactly the public floor, so its agent can
  never pull private material into public; authoring it requires root management-tier (`system_access =
  admin`).

The empty-join rule still holds: a map joined to ≥1 team inherits the root floor via the DAG; a genuinely
zero-join map reads ∅ (closed) — including no public content.

---

## 5. Scope-as-telos-incubation-space and `kb_cogmap_regions`

> **⚠ AMENDED by children specs (post-review, 2026-06-03).** Two load-bearing claims below are
> superseded by specs spun out of this section — read them as historical:
> - **Porosity is *dropped*, not "reframed."** `2026-06-02-map-regions-self-materialized-shape-surface-design`
>   §4 retires `kb_cogmaps.porosity` and the `porosity` enum entirely (visibility = teams:RBAC;
>   contribution = a `write` check). The "`kb_cogmaps.porosity` reframes accordingly" line at the end of
>   this section no longer holds.
> - **Delegation is *not* "a homed edge."** `2026-06-02-map-to-map-delegation-dissolution-design` §1
>   rejects any stored delegation object; the mechanism is the live, identity-agnostic predicate
>   `cogmaps_share_a_team`. The "a homed edge, §3" framing below is superseded.
> - Throughout, `kb_scope_proximity` → `kb_cogmap_regions` and `scope` → `cogmap` per the map-regions §0 rename (swept 2026-06-04).

A cogmap is **not an enclosure**. It is a concept-incubation space through a telos: all concepts are
*born of a telos-toward-utility* (a tool-using, ready-to-hand phenomenological framing — no a-priori
concept-as-ideal). The read gradient abandoned in §2 reappears here, **downstream of access**, as two
things:

- **`kb_cogmap_regions`** — the cogmap's **self-materialized shape surface**: clusters computed via
  cosine-nearness + relational-edge-weight-and-density + property-nearness, at configurable or
  subjective-agent thresholds of salience. Crucially it is materialized **by the cogmap, under its own
  telos, by its own agents**, and *read* by others. Self-materialization resolves the
  flattened-average epistemic worry from the resolution-contract: an outsider reads *Y's shape as Y
  computed it*, not a crawl through the outsider's own perspective-vector. Reading the cogmap's
  telos-framing artifact plus this surface is the **workhorse** — it gives an agent "what it needs to
  make more informed decisions" even when it was not launched *in* the cogmap (assuming traversal
  access and acting-on-behalf-of visibility), commonly by launching subagents.
- **Map-to-map delegation** *(shape recognized here; mechanism deferred)* — to *think-with* a
  cogmap's interior, an agent is **primed with the target cogmap's telos** but operates within the
  **delegating** cogmap's visibility boundary. This is **not a per-resource permission** — it is a
  **map↔map relationship** (a homed edge, §3), valid only where a **mutual team bridges the two
  cogmaps** (the §4 team↔cogmap join supplies the RBAC constraint). Priming is possible precisely
  because the delegating cogmap can already *read* the target's telos-framing + materialized shape
  (the §4/§5 read), so the delegated agent borrows the target's *frame* applied to the delegating
  cogmap's *visible material* — it never exposes the target's private interior. **Safety invariant,
  stated now because it is an access property: delegation must never escalate visibility.** The
  priming runtime, the delegation-edge lifecycle/semantics, and runtime visibility-enforcement are the
  staged **Domain-B** think-with mechanism — deferred to a separate spec.

**This spec's obligation to proximity is bounded.** It must guarantee the access-resolved *inputs*
exist and are coherently gated — chunk embeddings (pgvector) for cosine, **homed** edge-weights for
relational density, and `kb_properties` for property-nearness — so that clustering is *"just compute"*
over already-gated data, needing **no new access primitive**. The clustering algorithm, the
materialization table shape, and the retrieval/query surface are a **separate retrieval spec**, not
this one.

`kb_cogmaps.porosity` (built `access | attention`) reframes accordingly: it expresses *"is my
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
3. **Bootstrap-content recipient** (membership → reads `general`) — becomes **read-reachability through a
   content-root team** (resolves A2-1). `temper-system` retires as the *gate* but is repurposed as the
   **teams-DAG root content-home**: `general` (and any system-public resource/context) is granted to /
   homed in it, and—because every team descends from the root (down-only DAG, §4)—its grants are the
   universal read-floor for **both** people (membership) and agents (present in every map's intersection).
   An approved profile is a **virtual** member of the root, *derived from* `system_access` with **no stored
   row** (preserving "set status, not insert membership"): `approved` ⇒ **read-only ceiling** on root
   content; `admin` ⇒ **management tier** (may author system-public content). The system-access *gate*
   stays the profile status; the root team is only the content-reachability face of the same status — the
   two concerns share a source but answer different questions, so this does **not** re-fuse the R11
   conflation.

`is_system_admin` → `system_access = admin`, not team-owner. `has_system_access` /
`is_system_admin` read profile status — which **also drops their `gating_team_slug` dependency, an
item slug-retirement would otherwise have broken.** `kb_join_requests` and `kb_system_settings`
(`access_mode`, terms) stay; the approval workflow's terminal action becomes *set profile status*
(virtual root-membership follows from it), not *insert a `watcher` membership row*. `temper-system`
retires as the **access gate** and is repurposed as the **teams-DAG root content-home** (above).

---

## 7. The resulting three-layer stack

Three different questions, three mechanisms, no cross-contamination:

1. **System-access gate** — *may you exist here at all* → profile `system_access` status
   (`require_auth` precondition).
2. **Resource / edge / cogmap access** — *what may you do with this object* → the capability descriptor
   on `kb_resource_access` grants, intersected with `role-ceiling` for team anchors.
3. **Resolution / delegation** — *how deep* → reachability + the `kb_cogmap_regions` retrieval surface
   + map-to-map delegation (shape in §5; mechanism deferred).

---

## 8. Default-safety and cogmap lifecycle

*"The safe thing must be the easy thing."* The **emergent default cogmap** a person receives before
they have reasoned about cogmaps at all is a **first-class design object**, and it **defaults closed**
(produces nothing into shared cogmaps until explicitly granted; consumes only what the person can see).
A naive default here turns the emergent-cogmap story into an incident report.

**North star (forward-guidance, not a requirement of this spec):** cogmaps are as durable or as
ephemeral as they *earn*. Shallow-cloning a cogmap to expand, refine, or generalize its telos should be
cheap — the concept-home stays with the **original** cogmap; the clone carries edges **back** to those
homed concepts. This needs no special affordance: a clone is a new cogmap, and edges-back are exactly
the §3 (edge-as-homed-object) + §4 (team↔cogmap read-grant) pattern already designed. Recorded so the
access model stays forward-compatible with it, not to build it now.

---

## DDL delta (grounded against the built schema)

**New**
- capability-descriptor fields on `kb_resource_access` (replaces `access_level`)
- `kb_resource_homes`, `kb_resource_access` (the wrapper, from the data-model spec, now with the
  descriptor)
- edge-home columns on `kb_resource_edges` (`anchor_table`, `anchor_id`), projected from the asserting
  event
- `kb_team_cogmaps` (cogmap↔team association); `kb_teams_parents` (teams DAG)
- `edges_visible_to(...)` function (same shape as `resources_visible_to`)
- `resources_accessible_to_cogmap(...)` producer predicate (the DAG-expanded least-privilege
  team-**intersection**, closed on empty-join; *not* unioned with the launching person's memberships)
- `kb_profiles.system_access` (`none | approved | admin`) — gate *and* virtual root-team ceiling
- a seeded **system-default cogmap** joined only to the root team (the public cognitive-map home)

**Changed**
- `kb_team_resources` → subsumed into `kb_resource_access`
- `has_system_access` / `is_system_admin` → read profile status; drop `gating_team_slug` dependency
- `resources_visible_to` → resolves virtual root-membership from `system_access` (the universal read-floor)
- `temper-system` → repurposed from access-gate to **teams-DAG root content-home** (homes `general` +
  system-public content; every team descends from it)
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
3. **General-context default-read mechanism (A2-1)** — **RESOLVED (§6, §4):** no per-profile grant, no
   `system_default_read` column. `temper-system` becomes the **teams-DAG root content-home**; `general`
   (and system-public content) is granted to / homed in it; **virtual** root-membership is *derived from*
   `system_access` (no stored row) — `approved` = read-only ceiling, `admin` = manage. Universal
   readability falls out of teams-RBAC + down-only DAG for **both** people and agents. A symmetric
   **system-default cogmap** (joined only to the root) homes truly-public cognitive-map content. The
   gate stays `profile.system_access`, kept distinct from root-membership (no R11 re-fusion).
4. **Producer-grantee mechanics (A2-2)** — **RESOLVED (§1, §2):** the `principal` is the actor on *writes*
   too (completing CS-1). **Map-as-object** caps come from `kb_team_cogmaps` + role-ceiling (read = any
   joined-team member; write/contribute = member+ ceiling; manage = owner/maintainer). **Concept-level**
   read/write/delete are conferred to the agent-principal `Cogmap(M)` by the **map-home** (symmetric with
   context-home-confers), producer-bounded by the §4 intersection. **`grant` is not present** on concept-
   or edge-grants (§2): cross-map availability is forward-translation or consumer-pulled reference, both
   `write`s. Humans never directly author concepts (§1); they hold map-as-object `write` and launch the
   agent.
5. **`kb_teams_parents` role (A2-3)** — **RESOLVED (§4):** `vis(T)` is DAG-expanded **down-only**
   (descendant inherits ancestor/umbrella grants; an ancestor gains no visibility into descendant
   privates). The producer reach is the **intersection** over M's *joined teams*, never unioned with the
   launching person's memberships.
6. **`cogmaps_share_a_team` equivalence — sharpened (A4-1)** — §4 now *defines*
   `resources_accessible_to_cogmap` as the DAG-expanded team-**intersection**, which reveals the delegation
   spec's "`cogmaps_share_a_team` = exactly the producer-read condition" claim is **not exact**:
   `cogmaps_share_a_team` (∃ *one* shared team) is strictly *weaker* than the intersection (visible to
   *every* joined team). Resolution direction (settle in the A4-1 dig): `cogmaps_share_a_team` gates
   **priming** (borrowing the target's telos + *blurred* shape surface — legitimate on a single bridge);
   the intersection gates **material** (what the primed agent may actually deref — never-escalate). Two
   gates, not one equivalence.

---

## Out of scope

**Rejected (load-bearing decisions — resist re-litigation):**
- **read-resolution as a graded grant ceiling.** Collapsed into binary reachability + the
  `kb_cogmap_regions` retrieval surface + map-to-map delegation. The membrane-permeability image
  it rested on was superseded by map-as-telos-incubation-home.
- **system-access as resource-membership.** The original R11 bug; system-access is a separate layer
  (profile status), deliberately *not* unified into the capability model.
- **`access_level` as an ordered lattice.** Re-conflates `grant` with `delete`; the explicit
  capability set is the decomposition.

**Deferred (in scope elsewhere or later):**
- the `kb_cogmap_regions` **clustering algorithm**, materialization table, and retrieval/query
  surface — a separate retrieval spec.
- the **map-to-map delegation mechanism** — the priming runtime, the delegation-edge
  lifecycle/semantics, and runtime visibility-enforcement (the staged Domain-B think-with mechanism).
  Its *shape* and the never-escalate-visibility safety invariant are stated in §5; the mechanism is a
  separate spec.
- **shallow-clone / cogmap-lifecycle** affordances — north star only (§8).
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
