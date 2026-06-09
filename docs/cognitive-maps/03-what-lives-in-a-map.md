---
tier: cognitive-maps
order: 3
label: /cognitive-maps/what-lives-in-a-map
title: What lives in a map
description: A map's charter, questions, and regulation are built from two kernel primitives chosen for the capabilities each exposes — resources are atomic and graph-participating; content blocks are addressable and attributable. That difference is what makes provenance and freshness answerable.
register: concrete
genre: show
---

# What lives in a map

> The steward has just concluded something — *pair newcomers with a maintainer on the first
> PR* — and now has to put it somewhere. To be useful, the lesson has to become two things at
> once: a concept the map can point at and wire into its graph, and recorded text that
> carries where it came from. Those are two different capabilities, and they're the whole
> shape of what a map is built from.

## When the steward learns something

A map earns a lesson and has to find it a home. Watch what the onboarding steward needs the
moment it concludes *pair on the first PR*, and the data model nearly designs itself.

It needs the lesson to be a **concept it can point at and relate** — wired to the telos,
referenced by other ideas, present in the graph as a thing in its own right. And it needs the
lesson's **text to be addressable and attributable** — so the question it answers (*"where
are the sharp edges that scar newcomers?"*) can record which engineer's week, which events,
which systems shaped it, and so someone later can check whether it still holds.

Two needs, two kinds of thing. The clearer way into the model isn't a catalogue of primitive
types; it's to follow each need to the primitive that serves it. Two kernel primitives carry
all of it — the **resource** and the **content block** — and they split along exactly those
two needs: what stands on its own and anchors relationships, versus what is addressable and
attributable inside something else.

## The charter is "just" a resource

Structurally, a telos-charter is nothing special: a `kb_resources` row, the same kind of
resource as any concept in the graph. That plainness *is* the capability. Because the
charter is an ordinary resource, it **fully participates** — it has its own identity, it can
be the source or target of edges, it can be a region member, and its history is a projection
off the event ledger like everything else. A map's purpose isn't sealed in a special charter
table; it's a first-class citizen of the same graph the map reasons over.

This is what **atomicity** is for. A resource is an atomic, self-sufficient unit of
reference: it can be pointed at from anywhere, found on its own, and it participates in the
graph as a cohesive whole. You can build relationships *to* the charter precisely because
the charter is a thing that stands by itself.

## The questions are content blocks

The charter's guiding questions live *inside* that resource, as **content blocks** — and
the change of primitive is deliberate, because a question needs capabilities the charter-as-
a-whole doesn't. A question has to be **uniquely addressable** (you can point at *this* one),
**mutable** (it gets re-stated, reinforced, folded as the map learns), and above all
**attributable**: a block records which events, in what order, and from which systems shaped
it into its current form.

Attribution is the capability that earns the block as its own primitive. It's what lets a
map answer *"where did this come from?"* and *"where do I look to check it's still fresh
against the remote system that fed it?"* — a block's provenance is a chain back through the
ledger to the integrations and acts that formed it. (Body text lives one level deeper still,
in chunks under a block — the grain at which content is embedded and deduplicated.)

The trade that makes this coherent: a content block is **not** atomic and **not**
self-sufficient. It has a lifecycle of its own, but it does **not** participate in the graph
independently. Nobody draws an edge to a single question-block from across the map; relations
attach to the charter resource that contains it. Addressable interiority, not a free-standing
node — and that's the whole line between the two primitives. Stands alone and anchors
relationships → resource. Addressable and attributable *within* a host, its meaning part of
that host → content block.

## Regulation: the tools a map makes for itself

Regulation is the third thing a map holds, and it isn't a third primitive — it's resources
again, used a particular way. A map's regulation is an open, **agent-maintained** set of
concept-resources that exist to *express and effect* the grounding telos, written as the map
learns what its purpose demands in practice. In the seed, *"pair on the first PR"* is a
resource, homed in the map, marked `doc_type = cogmap_regulation`, reached from the telos by
an `express` edge labelled `operationalized_by`.

Two things give it its character. It's **open** — regulation accrues; it isn't a fixed field
on the map. And it's **the agent's instrument** — regulation is how a map's steward turns
*what we've learned* into *what we now do*, each piece a tool the map made for itself.
[How a map grows](04-how-a-map-grows.md) is partly the story of how a new piece of regulation
gets written — sometimes the hard way.

> **▣ VISUALIZATION PLACEHOLDER — `INLINE` · introductory ERD**
> **Shows —** the handful of tables that hold a map, populated with *the onboarding map's
> own rows*: the telos `kb_resources` row; its `kb_resource_homes` row anchoring it in the
> cogmap; its charter `kb_content_blocks` (the telos statement block + three question blocks)
> and their `kb_chunks`; the `kb_cogmaps` row pointing at the telos; the `express` `kb_edges`
> row to the regulation resource; the `doc_type` `kb_properties` rows. Boxes and the
> relationships between them, labelled with real values from the seed — enough to see *what
> data is a map*, not every column.
> **Honest basis —** the tables named above in
> [`01_schema.sql`](../../schema-artifact/01_schema.sql); the exact rows in
> [`03_seed.sql`](../../schema-artifact/03_seed.sql); the reads — `resource_body_text`
> for the charter body and `resource_blocks` for its question blocks, both resolved through
> `cogmap_telos`, plus `cogmap_regulation` — in
> [`02_functions.sql`](../../schema-artifact/02_functions.sql), demonstrated by scenario
> **S4**.
> **Fidelity —** illustrative. Real table names and real row values, but *not*
> column-precise — omit types, indexes, and the columns that don't earn their place in a
> first look. A column-precise ERD is a later child page.

## How this map came to be

The charter, the blocks, the cogmap row, the home, the `doc_type` — all of it is created in
**one transaction**, by `cogmap_genesis`. The ordering carries the idea: **resource-first**.
The telos resource and its blocks are written *before* the cogmap row, so the map's not-null
pointer to its telos is satisfiable the moment the map row is inserted — no deferred
constraint, no half-built map. The charter exists before the map that's organized around it,
which is the right order for a thing whose whole identity is its purpose.

> **▣ VISUALIZATION PLACEHOLDER — `INLINE` · genesis step diagram (dynamic companion)**
> **Shows —** the single-transaction birth of the onboarding map as an ordered sequence, so
> the static ERD gets a *"how it came to be"* companion:
> `cogmap_seeded` event → telos **resource** → **telos-statement block** + **question
> blocks** → **cogmap** row → **home** (telos anchored in the cogmap) → **`doc_type`**
> property → backfill the event's producing anchor. Emphasis on **resource-first**: the
> resource exists before the cogmap that points at it.
> **Honest basis —** the `cogmap_genesis` function in
> [`02_functions.sql`](../../schema-artifact/02_functions.sql), step for step; the call in
> [`03_seed.sql`](../../schema-artifact/03_seed.sql) that seeds the onboarding map.
> **Fidelity —** illustrative. The *order* and the *resource-first* shape are what this
> shows; exact arguments and hashes are not.
> **For successor —** this pairs naturally with the ERD as a small animation or a
> step-through — the ERD is the noun, this is the verb that produced it.

## An open seam

The resource-or-block call is clean at the extremes and has a soft middle: a thing that's
*mostly* interior but occasionally referenced on its own sits right on the line. The artifact
takes a position — findable-and-graph-participating ⇒ resource — and the seed stays clear of
the ambiguous cases on purpose. Where exactly the line wants to fall under real content is
something the empirical load is meant to pressure-test, not something we've frozen.

---

*Next: [how a map grows](04-how-a-map-grows.md) — the five things that can happen to an
idea, and the agent that does them.*
