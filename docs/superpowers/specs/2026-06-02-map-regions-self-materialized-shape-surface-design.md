# Map-Regions: The Self-Materialized Shape Surface (`kb_map_regions`)

**Date:** 2026-06-02
**Status:** Design — in brainstorming, pending review
**Goal:** `substrate-kernel-to-cognitive-map`, Arc 1 (shared-kernel completion)
**Spun out of:** [`2026-06-02-access-capability-model-design.md`](2026-06-02-access-capability-model-design.md) §5
(the "separate retrieval spec" the access model defers to).

> **Grounding note.** Written against the **actual built schema** — the base
> `20260330000001_consolidated_schema.sql` (which carries `kb_chunks.embedding vector(768)` with an
> HNSW `vector_cosine_ops` index) plus `20260522000001_event_ledger_unification.sql` (the `kb_scopes`
> table `(id, name, porosity)` and the unified `kb_events` ledger) — and against the two sibling design
> specs it composes with (the access/capability model and the data-model reconciliation). Where this
> spec says "real" vs "designed," it has been checked against migrations. The three proximity *inputs*
> (chunk embeddings, homed edge-weights, `kb_properties`) are guaranteed by those siblings; this spec
> owns only the materialized surface, its access-gated read, and its freshness contract.

---

## Context

The access/capability model (`2026-06-02`) collapsed the resolution gradient out of the access layer
and relocated it downstream into two things: a **self-materialized shape surface** and a
**map-to-map delegation** relationship. It defined the access-resolved *inputs* to the surface and
explicitly deferred "the clustering algorithm, materialization table, and retrieval/query surface" to a
separate retrieval spec. **This is that spec.** (The delegation mechanism is a third spec; this one is
foundational to it — delegation's priming step *reads* the surface defined here.)

The access model's obligation to this surface was bounded and is already discharged: the three signals
it clusters over are already-gated kernel data — chunk embeddings (pgvector), **homed** edge-weights
(access spec §3), and `kb_properties` (data-model spec §3). Clustering is therefore *"just compute"*
over already-gated data and needs **no new access primitive**. This spec holds that line.

### Altitude (decided in brainstorming)

This spec defines the **materialized surface + its read API**, and treats the agent that *computes and
judges* the shape as an **opaque Domain-B producer**. Concretely it owns: the `kb_map_regions` table
shape, the read-time proximity/combination function over the three inputs, the freshness contract, and
the cross-map read surface. It does **not** own the clustering algorithm, the salience-threshold
judgment, or the trigger scheduling — those are the black-box agent's, staged to Domain B.

---

## 0. Vocabulary reframe: `scope` → `map`

This spec **renames `kb_scopes` to `kb_maps`** and adopts the new vocabulary natively. The rename is a
load-bearing decision, recorded here because this is the first net-new modeling to touch the entity.

**Why.** "Scope" entered the model as a *think-with frame of reference* with an inside and an outside —
a selectively-permeable membrane. The model has since moved decisively away from that: the access spec
established that the entity **homes** concepts but does **not enclose** them (concepts home in one place
and participate elsewhere through edges), and that visibility is carried entirely by **teams:RBAC**, not
by any membrane property of the entity. The entity is now a **telos-seeded home**: a seed-statement-as-
telos onto which concepts arrive and are linked by telos-as-intent, growing branching relational maps
that develop **regions** of clustered cognitive-map elements. It no longer determines an
inside/outside-ness. "Scope" misnames that; **`map`** names it — the entity *is* the cognitive map;
concepts home **on** a map, edges connect them, and a concept may appear on several maps (homed on one,
referenced from others) exactly as a place appears on multiple maps. This makes `temper-cogmap` literal:
it manages `kb_maps`.

**`telos` is the map's seed-statement, and the seed-statement *is a resource*.** Throughout the specs
*telos* denotes the **intent** ("concepts born of a telos-toward-utility," "primed with the target's
telos"). Naming the entity "telos" would make "a telos's telos" incoherent — so the entity is the map,
and its telos is referenced by a **`telos_resource_id` FK to `kb_resources`**, *not* stored as a bare
`text` column. The seed-statement is the materialized latest-version of an ordinary resource: it has
externalized content (chunks + embeddings), revisions (the telos can evolve), and may carry its own edges
and properties — no special-casing outside the everything-is-a-resource model. Two payoffs: **(1)** the
telos participates in the map's *own* shape as an embedded node — the seed is a first-class concept on the
map it seeds; **(2)** the access spec's "read the map's telos-framing artifact plus this surface" and
delegation's "primed with the target's telos" become **literal** — the framing artifact is a resource you
read (gated by ordinary resource access) like any other, beside the §2 region surface. The telos-resource
homes on its own map (`anchor_table='kb_maps'`); the resulting map↔telos mutual reference is a
creation-ordering detail (deferred FK / single creation event batch), noted for the plan.

**Mechanical substitution (this spec uses the right-hand column throughout):**

| was | becomes |
|---|---|
| `kb_scopes` | `kb_maps` (gains a `telos_resource_id` FK — the seed-statement **is a resource**, §0; **loses** `porosity`, §4) |
| `kb_scope_proximity` | `kb_map_regions` |
| home/access anchor `'kb_scopes'` | `'kb_maps'` |
| `kb_team_scopes` | `kb_team_maps` |
| `resources_accessible_to_scope` | `resources_accessible_to_map` |
| `kb_edges.scope_id` (data-model §5) | superseded by the polymorphic edge-home (access §3); see note |
| "scope-telos-held write authority" | "the map's telos-held write authority" |
| "scope-to-scope delegation" | "map-to-map delegation" |
| "scope-as-telos-incubation-space" | "map-as-telos-incubation-home" |

**Blast radius / cross-spec consequence.** The rename ripples through the two already-committed sibling
specs (access + data-model) and the single built `kb_scopes` table (currently unused — `scope_id` rides
on `kb_events` and nothing leans on it for access). Because the only built artifact is that one unused
table, the rename is near-free now and expensive once Limb 1c lands — hence *deciding* it here. A focused
`scope→map` terminology sweep over the access and data-model specs is **deliberately deferred to the
implementation-planning phase**: it resolves down when the hard DDL is written, not as a standalone doc
sweep now. This spec is the canonical record of the rename until then.

> **Edge-home note.** The data-model spec §5 gave `kb_edges` a nullable `scope_id`. The access spec §3
> superseded that with a polymorphic edge-home `(anchor_table, anchor_id)` where `anchor_table ∈
> ('kb_contexts','kb_maps')`. This spec assumes the access-spec edge-home; the `scope_id`/`map_id` column
> does not survive. Recorded for the reconciliation sweep.

---

## 1. What `kb_map_regions` is

A map's **self-materialized shape surface**: a set of **regions**, each one the map's own
telos-chosen cluster of the concepts homed on it. A region is the readable unit. It carries a **centroid**
(an aggregate embedding), a **salience** (its importance under the map's telos), an optional **label**,
and a **member set** (the concepts it groups). The clustering that produced the region — which concepts
group together, at what salience threshold, under what subjective weighting — is the black-box agent's
judgment; this spec stores its **output** and provides the read.

Owned by **`temper-substrate`** as an **access-gated projection**, kin to homed edges and properties:
Domain-B agents *write* regions through the substrate `Backend`; any caller (Domain A or B) *reads* them
through the kernel's access functions. This asserts `kb_map_regions` is a **kernel access surface, not a
spine-#2 Domain-B operational table** — a deliberate carve-out from the data-model spec's "Domain-B
tables → spine #2" deferral, justified because the surface is read cross-map through the kernel's access
layer and introduces no cognitive-map *semantics* into the kernel (a region is a centroid + salience +
members; the kernel never interprets what a region *means*).

---

## 2. Table shape (event-sourced, uniform with edges/properties)

```sql
kb_map_regions (
    id                   uuid pk default uuid_generate_v7(),
    map_id               uuid not null references kb_maps(id),
    centroid             vector(768) not null,   -- mean-pool of member concepts' chunk embeddings
    salience             float not null,         -- region importance under the map's telos (agent-assigned)
    label                text,                   -- optional agent-authored region label
    member_count         int not null,           -- aggregate; exposed in the surface read
    asserted_by_event_id uuid not null references kb_events(id),
    last_event_id        uuid not null references kb_events(id),
    is_folded            boolean not null default false,
    created              timestamptz not null default now()
);
create index idx_kb_map_regions_map on kb_map_regions(map_id) where not is_folded;
-- optional: hnsw on centroid for cross-map "nearest region to a query" search; plan-level.

kb_map_region_members (            -- the interior; never returned wholesale by the surface read
    region_id    uuid not null references kb_map_regions(id),
    member_table varchar(64) not null check (member_table in ('kb_resources','kb_maps')),
    member_id    uuid not null,
    affinity     float,            -- member's nearness to the region centroid (core vs peripheral)
    primary key (region_id, member_table, member_id)
);
create index idx_kb_map_region_members_member on kb_map_region_members(member_table, member_id);
```

`member_table` stays polymorphic (a region may group concept-resources and, in principle, sub-maps) for
consistency with the homes/edges anchor vocabulary. The fold/assert pattern
(`asserted_by_event_id` / `last_event_id` / `is_folded`) is **identical** to the built
`kb_resource_edges` and the designed `kb_properties` — no new freshness primitive.

---

## 3. Surface vs. interior (the access line)

Two tiers, gated differently:

- **Surface** (the region row): `centroid, salience, label, member_count` + the materialization
  watermark. Readable by anyone who can read the map (§4). `member_count` is an aggregate (a count, a
  blur) — it does not leak member identities.
- **Interior** (the member set): individual member identities live in `kb_map_region_members` and are
  **never** returned by the surface read. To resolve a specific member a caller dereferences it through
  ordinary `resources_visible_to` and **may be denied**.

This is "maps home but do not enclose," exact: you may read a map's *shape* without being able to read
every concept that shaped it. It needs **no new access primitive** — the surface is gated by map-read,
the interior by resource-access — and it is the same posture as the access spec's
delegation invariant (**never escalate visibility**).

---

## 4. The read gate (porosity dropped)

Reading a map's shape is gated by **teams:RBAC** — the *same* "can the reader read map Y" resolution
used for reading Y's homed concepts (access spec §3 homing + §4 `kb_team_maps`). There is **no separate
posture** on the map.

**`kb_maps.porosity` is dropped; the `porosity` enum is retired.** Porosity was the last vestige of the
membrane image: a per-map setting for "how readable / how permeable." With visibility carried entirely by
teams:RBAC and the membrane gone, porosity has no referent. Both halves it was reaching for collapse into
mechanisms that already exist:

- **"is my shape readable"** → can the reader read the map (teams:RBAC). Nothing map-local to set.
- **"is my shape contributable-to"** → does the writer hold **`write` on the map** (the map's
  telos-held write authority, access spec §1–2). Contribution is a write; it needs no porosity.

(Confirmed against the confidence inventory, where porosity was already **Drifted #19** — "almost
certainly insufficient … don't defend the enum." This spec discharges that by removing it.)

Three substrate read functions, shaped to mirror `resources_visible_to` / `edges_visible_to`:

```
map_shape_visible_to(map_id, profile)          -> region surface rows, iff profile can read map_id
locate_in_map_shape(map_id, query, profile)    -> regions ranked by proximity to query (see §5)
map_region_members(region_id, profile)         -> members filtered through resources_visible_to
```

`query` in `locate_in_map_shape` is either an **embedding** (the free-text workhorse: an agent embeds its
concern and asks "where does this land in map Y") or a **concept_id** (an existing concept located in Y's
shape, enabling the full three-signal blend of §5).

---

## 5. The combination function (read-time `query → region` proximity)

Read-time proximity of a `query` to a region is a **normalized weighted blend** of up to three signals,
computed against the region:

- **cosine** — `query` embedding ↔ region `centroid` (always available).
- **edge-density** — weight/count of the query-concept's **homed edges** to the region's members
  (available only when `query` is a concept).
- **property-overlap** — shared `kb_properties` between the query-concept and the region's members (the
  symmetric salience self-join from the data-model spec; available only when `query` is a concept).

It **degrades gracefully**: a free-text/embedding query uses cosine-to-centroid alone; a concept query
uses all three. Signals normalize to `[0,1]` and blend by weight; **default equal weights**, optionally
overridden per-map via `kb_properties` on the map (e.g. `proximity:w_cosine`), so a map can express that
its telos weights relational structure over surface similarity. To keep an outsider reading *"Y's shape
as Y computed it"* rather than through their own lens, the locate uses **Y's** weights, not the reader's.

The exact normalization, the edge-density formula, and the default weight vector are **plan-level**
(deliberately not pinned here — per the confidence inventory's Drifted #20, field-level protocol detail
masks the load-bearing decisions, which are settled above).

> The same three signals are what the black-box agent clusters over to *produce* regions. This spec does
> **not** constrain the agent's internal algorithm; it defines the read-time blend (the contract a
> coherent shape is expected to honor) and stores the agent's output.

---

## 6. Freshness (event-sourced + materialization watermark)

A recompute is **one event-correlated batch**: fold the map's prior live regions and assert the new set,
all under a single materialization event (correlation id). The **watermark** is that materialization
event's id, returned on every surface read. A caller compares it to the map's current `last_event_id`
(the latest event touching the map's homed concepts/edges/properties) to see **how stale** the shape is.

**Stale reads are allowed and legible** — the surface never blocks on freshness; it reports it. Regions do
**not** auto-fold when underlying concepts change; they remain the last materialized shape, marked by an
older watermark, until the agent recomputes. This matches the substrate's read-what-was-projected
posture for edges and properties.

---

## 7. The write contract (the black box's only door)

Domain-B agents materialize a map's shape through **one** substrate command:

```rust
struct MaterializeMapShape { map_id: Uuid, regions: Vec<RegionAssertion> }
struct RegionAssertion { centroid: Vector, salience: f64, label: Option<String>, members: Vec<MemberRef> }
```

The command performs the §6 fold-old / assert-new batch atomically. **Auth-before-write:** the caller
must hold **`write` on `map_id`** (the map's telos-held authority, access spec §1–2) — this *is* the
"contributable-to" half of old porosity, now just a write check. Producer-bounded (access spec §4): the
agent may only group members the **map** can read, never the keyboard-holder's wider set.

Everything upstream of this command — the clustering algorithm, the salience-threshold and subjective
weighting choices, *when* to recompute (on-event, scheduled, on-demand), and the agent runtime itself —
is **out of scope**, staged to Domain B. The kernel sees only well-formed region assertions arriving
through an authorized write.

---

## 8. Inputs guarantee (no new access primitive)

This spec stands on already-gated kernel data and adds none:

| signal | source | status |
|---|---|---|
| cosine | `kb_chunks.embedding vector(768)` + HNSW `vector_cosine_ops` | **built** (`20260330000001`) |
| edge-density | homed edge weights | **designed** — access spec §3 (polymorphic edge-home) |
| property-overlap | `kb_properties` symmetric self-join | **designed** — data-model spec §3 |

The centroid is a mean-pool of member concepts' chunk embeddings (exact pooling → plan). Because every
input is already access-resolved, materialization and read are *"just compute"*; the only access surface
this spec adds is the read-gate **shape** (§4), which is the existing teams:RBAC resolution re-expressed,
not a new mechanism.

---

## DDL delta (grounded against the built schema)

**New**
- `kb_map_regions` (the region surface, event-sourced)
- `kb_map_region_members` (the gated interior)
- `map_shape_visible_to` / `locate_in_map_shape` / `map_region_members` read functions

**Changed**
- `kb_scopes` → **`kb_maps`** (rename); **add** `telos_resource_id` FK to `kb_resources` (the
  seed-statement is a resource, §0); **drop** `porosity`
- `MaterializeMapShape` added to the substrate command base (write door)

**Retired**
- `kb_scopes.porosity` column and the `porosity` enum (`20260522000001`)
- (consequent) the access spec §5 "porosity reframes" line — superseded by §4 here

---

## Open questions (refine during the plan; not blockers)

1. **Centroid pooling** — mean-pool of all member chunk embeddings vs. pool-per-concept-then-mean.
   *Lean:* mean over member concepts' pooled embeddings (one vector per concept first).
2. **Cross-map "nearest region" search** — whether `locate_in_map_shape` over *many* visible maps wants a
   global HNSW on `centroid`, or per-map scan suffices (regions per map are few). *Lean:* per-map scan
   first; add the index only if a cross-map locate path materializes.
3. **Default weight vector + normalization** for §5 — plan-level; equal weights as the starting point.
4. **map↔telos creation ordering** — `kb_maps.telos_resource_id` and the telos-resource's
   home-on-its-own-map are mutually referential. *Lean:* deferred FK + a single creation event batch;
   plan-level.

---

## Out of scope

**Rejected (load-bearing — resist re-litigation):**
- **`porosity` as a map-local readability/permeability setting.** Removed; visibility is teams:RBAC,
  contribution is `write`. (Confidence inventory Drifted #19.)
- **Returning member identities in the surface read.** The surface is an aggregate; the interior is
  dereferenced per-member through resource access (§3). Violating this reopens the §3 connection-privacy
  leak the access spec closed.

**Deferred (in scope elsewhere or later):**
- the **clustering algorithm**, salience-threshold judgment, subjective weighting, and **agent runtime /
  trigger scheduling** — the black-box Domain-B producer.
- **map-to-map delegation** — the next sibling spec; it *reads* this surface to prime a delegated agent.
- the **exact §5 formula / weight defaults / normalization** and **centroid pooling** — plan-level.
- the **`scope → map` terminology sweep** over the access and data-model specs and the migration —
  **resolved during implementation planning (hard DDL)**, not now; this spec is the canonical record of
  the rename until then (§0).

---

## Connections

- **Spun out of:** [`2026-06-02-access-capability-model-design.md`](2026-06-02-access-capability-model-design.md) §5 (the "separate retrieval spec")
- **Amends:** that spec's §5 porosity-reframe line (porosity dropped) and its DDL (`kb_scopes`→`kb_maps`, drop `porosity`)
- **Composes with:** [`2026-06-01-data-model-reconciliation-design.md`](2026-06-01-data-model-reconciliation-design.md) — `kb_properties` (§3), the substrate command base, the crate topology (`temper-substrate` owns this; `temper-cogmap` writes through it)
- **Foundational to:** the forthcoming map-to-map delegation spec (priming reads this surface)
- **Research grounding:** `2026-05-31-temper-confidence-inventory` (porosity = Drifted #19; protocol-over-design = Drifted #20), `2026-05-29-resolution-contract-and-the-permeable-scope-surface` (the resolution gradient this surface absorbs), `2026-05-31-definitional-fallacy-concept-as-basin-telos-resolves-threshold-primitive` (concept-as-basin; telos)
- **Goal:** `substrate-kernel-to-cognitive-map`, Arc 1
