---
tier: cognitive-maps
order: 2
label: /cognitive-maps/the-substrate-beneath-it
title: The substrate beneath it
description: Pull any thread in a map and it ends at an event. What the substrate does — make every part of a map answerable to how it came to be — and how the present is materialized from an append-only ledger, with the kernel kept convention-agnostic.
register: foundational
genre: show
---

# The substrate beneath it

> A week later, another engineer hits the same wall — and *"pair on the first PR"* is already
> there to meet them. Lean on a rule like that and a fair question follows: where did it come
> from, and is it still what we think? Pull the thread and you never reach a typed-in row.
> You reach an event. That's what the substrate does: it makes every part of a map
> answerable to *how it came to be.*

## Where a rule comes from

A week into the onboarding map's life, a second engineer hits the same sharp edge the first
one did, and the steward's rule — *pair on the first PR* — is already there to meet them.
Anyone leaning on a rule like that is owed two answers: *where did this come from,* and *can
I trust it's still current?*

Pull the thread in Temper and you don't land on a row someone typed and could quietly have
edited. You land on an **event**: the moment the lesson was recorded, by which actor, under
which map. Pull *any* thread in a cognitive map — a region, an edge, a charter question — and
it ends the same way, at an event on an append-only ledger. That answerability is the whole
job of the substrate.

## Events are the truth; everything else is derived

So the single decision everything else in Temper leans on: **events are the source of truth,
and everything else is derived.** A resource, an edge, a region, a charter block — each is a
*projection*: a materialized current-state view the system maintains, and could always
rebuild, from the events that produced it. The projection is what you query; the ledger is
what's true.

Everything downstream — supersession that leaves a mark, folding that preserves rather than
deletes, provenance you can follow — falls out of that one choice.

## What an event carries

The ledger is `kb_events`. Every change lands there as an event, and an event carries four
things that matter:

- an **emitter** — *who* acted, modelled as an *entity*, never a bare person (the reason
  surfaces in [how a map grows](04-how-a-map-grows.md));
- a **type** — what kind of act it was (`cogmap_seeded`, `relationship_asserted`,
  `block_mutated`, and so on);
- an optional **producing anchor** — the cogmap or context the act happened in, kept as
  *provenance*, not as the access gate;
- a **correlation id** — the thread that ties a multi-event act together.

It only ever grows. The present is a fold over the past, not a row someone overwrote — which
is what let the second engineer's question have a real answer.

## The present is materialized, not replayed

The things you actually read — resources, content blocks, edges, properties, regions — are
projection tables. Each carries its lineage back to the ledger: an `asserted_by_event_id`
(or `genesis_event_id`) for where it came from, and a `last_event_id` for the event that
last changed it. They're maintained, not recomputed on every read — kept materialized so a
query doesn't replay history each time.

That materialization is where the *current-state machinery* lives, and it earns its place.
The full-text index, the vector embeddings used for similarity, the weights on edges, the
salience on regions — all of it is computed onto the projections, not the ledger. Regions in
particular are recomputed on a **cadence** rather than continuously: a region's shape is a
snapshot taken at a moment, which is exactly why the system can tell you, on read, when that
snapshot has gone stale. The ledger holds what happened; the materialized present holds
what's currently searchable, rankable, and salient.

This is also where forgetting becomes mechanical. When something is folded, it gets an
`is_folded` flag — the projection stops surfacing it, the indexes stop ranking it, and the
event that folded it stays on the ledger. Nothing was destroyed; what changed is what the
current state carries forward. Decay, fold, and scar — the moves a map makes as it grows —
all happen on the materialized present, against a ledger that forgets nothing.

## The onboarding map, traced

Take the map the engineers are leaning on and follow it down. It didn't spring into being —
it was *emitted*:

- a single `cogmap_seeded` event is the genesis correlation root;
- the **telos resource** and its **charter blocks** (the statement, then the three questions)
  were written under that event;
- the **regulation** — *"pair on the first PR"* — hangs off the telos by an `express` edge,
  itself asserted by an event;
- the **materialized region** the steward reads cites the event it was materialized under.

Every part of the map traces to the ledger. The map is what those events currently add up to.

> **▣ VISUALIZATION PLACEHOLDER — `HERO` · system-architecture**
> **Shows —** the event ledger (`kb_events`) drawn as a single **append-only spine** down
> the page, with **projections** branching off it: resources, content blocks, edges,
> properties, and cogmap-regions, each connected back to the spine by its
> `asserted_by` / `last_event` lineage. Above the spine, the kernel shown as
> **convention-agnostic**: the same ledger + entities + resources underlie *both* Temper's
> workflow / knowledge-base patterns *and* the cognitive map, with neither baked into the
> schema. The reader should come away with: one source of truth at the bottom, everything
> else hanging off it.
> **Honest basis —** `kb_events` (`emitter_entity_id`, `event_type_id`,
> `producing_anchor_*`, `correlation_id`, `metadata`) in
> [`01_schema.sql`](../../schema-artifact/01_schema.sql); the lineage columns
> (`asserted_by_event_id` / `last_event_id` / `genesis_event_id`) on `kb_resources`,
> `kb_edges`, `kb_properties`, `kb_cogmap_regions`, `kb_content_blocks`. The
> one-kernel-many-conventions framing traces to the decision *2026-06-01
> the-shared-kernel-boundary*.
> **Fidelity —** conceptual. Kernel + projections only. The crate-level topology
> (temper-core / -api / -cli / -mcp) is a *later* child page, not this one.
> **For successor —** the spine wants to read top-to-bottom or left-to-right as clearly
> *append-only* (an arrow of time). Resist drawing it as a hub-and-spoke; it's a timeline
> with growths off it.

## The kernel doesn't pick sides

The kernel underneath is deliberately **convention-agnostic**. It knows about events,
entities, resources, edges, properties, and blocks — and nothing about what they're *for*.
The workflow and knowledge-base mechanics Temper ships — documents, contexts, sync — are
**conventions** expressed over that kernel, not structure carved into it. The cognitive map
is another such convention: a charter is a resource carrying a particular `doc_type`
property; a regulation is a resource reached by a particular edge. Neither is new machinery
beneath the kernel — both are *agreements about how to read it*.

That separation is what keeps the model open. Because the schema commits to the kernel and
not to today's use cases, it can express patterns we haven't reached for yet, while
convention steers the common ones toward shared shapes — intention expressed, not enforced by
the table layout. The cognitive map gets to be event-sourced without paying for it twice: it
isn't bolted onto the substrate, it's a reading of it.

## What this buys

An append-only ledger costs something to maintain, and what it buys is everything that comes
later. Because the trace of *how* a thing came to be is never thrown away, supersession can
leave a [scar](04-how-a-map-grows.md), a fold can preserve instead of delete, and the
question the second engineer asked — *why does the map say this?* — always has an answer the
system can compute, across system boundaries even, which the [insights](07d-insights.md)
story returns to. The honesty about its own history that `/theory` asked for isn't a feature
added on top; it's what's left when nothing is overwritten.

## An open seam

One modelling call we made here and are watching: the event's producing anchor is kept as
**provenance, and left nullable**. Two reasons sit behind that. First, every homed object
(an edge, a region) already carries its own access anchor, so the event doesn't re-decide
access — it records *where the act happened* and lets the object's own home gate the read.
Second, not every event is born inside a map. An integration like a webhook emits a **pure
data event** with no cogmap to anchor to yet — raw signal, until a triage agent picks it up
and works it for salience within some map's telos. A nullable anchor is what lets unowned,
external events exist on the ledger before any map has claimed them, which is exactly the
shape the integration story (later in this set) leans on. Whether the same modelling holds
for every event family is the kind of thing the empirical load of the artifact exists to
surface.

---

*Next: [what lives in a map](03-what-lives-in-a-map.md) — the two kinds of thing a map is
built from, and the capability that tells them apart.*
