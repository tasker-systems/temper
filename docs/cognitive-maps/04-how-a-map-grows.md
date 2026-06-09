---
tier: cognitive-maps
order: 4
label: /cognitive-maps/how-a-map-grows
title: How a map grows
description: Five learning-acts — form, modify, decay, fold, scar — each mapped to a real mechanism, not a metaphor. The agent that performs them is a persona; the actor that emits is an entity.
register: concrete
genre: show
---

# How a map grows

> Watch the onboarding map over a few weeks. A question keeps getting leaned on and grows
> sturdier. An old assumption quietly stops being used — then one week it backfires, and the
> steward folds it *and* writes down what it cost. Each of those is one of five things that
> can happen to an idea in a map, and not one is a metaphor.

## A few weeks in the life of a map

Give the onboarding map a little time, and watch what happens to its ideas.

One of its questions — *"what's the smallest real change that builds confidence?"* — keeps
getting leaned on. Every engineer who comes through references it, and the map notices: the
question grows sturdier, not because anyone bumped a number, but because the traffic past it
is real. That's **reinforcement**.

An assumption it once held — *new folks should start by reading the whole architecture doc* —
quietly stops getting used. Nobody argues it down; it just stops drawing references, and its
standing falls on its own. That's **decay**.

Then one week the assumption backfires: a newcomer sinks three days into that doc and comes
out *less* confident. The steward doesn't simply drop it. It **folds** the assumption — sets
it aside as superseded, without erasing that it was ever held — *and* writes the lesson that
replaces it into the map's regulation: *pair on the first PR.* Fold, plus a lesson written
forward: that's a **scar**.

Reinforcement, decay, fold, scar — plus the plain **forming** of something new — are the five
things that can happen to an idea in a map. Each is a real mechanism in the kernel, not a
mood, and the rest of this page is what each one actually is.

## The five, named

- **form** — a new concept or relation appears;
- **modify** — something already there is re-stated or re-weighted (reinforcement is the
  quiet case);
- **decay** — something fades because nothing keeps reinforcing it (restraint, not deletion);
- **fold** — something is deliberately set aside as superseded — *preserved, not wrong*;
- **scar** — the hardest one: a thing is folded *and* the lesson from folding it is written
  into regulation.

Each maps to a mechanism already in the kernel. None is a special "learning" table.

## Reinforcement is derived, not stored

The act that surprised us most is **modify**, specifically reinforcement — because there's
nothing to bump. There is no weight column on a question that an agent increments when the
question proves useful. A question's standing is *read from the reference stream*: the
count and recency of provenance accretions into its block. `resource_blocks` returns a
`reinforce_count`, and that number is a `count(...)` over `kb_block_provenance`, not a
stored field.

The reason this is the right shape and not just a clever one: the substrate exposes the
raw reference stream honestly, and any narrowing of "referenced" down to "confirmed" is a
tuning decision an agent makes out in the open, not a number baked into storage where
nobody can see how it was set.

## Decay and fold

**Fold** is a visibility act, and it's orthogonal to whether something is current. Setting
`is_folded` on a block (or an edge) stops the projection from surfacing it; the event that
folded it stays on the ledger; the content is *preserved, not deleted*. "Wrong" is not the
claim — "superseded" is. Anyone re-engaging the history can still see what was set aside and
when.

**Decay** is the softer cousin of fold: nothing is actively set aside, a thing simply stops
being reinforced and its standing falls on its own. Restraint rather than action. The map
gets lighter without anyone deciding to delete.

## Scar

**Scar** is fold with a memory. A question (or concept) is folded, *and* a lesson is
written into the map's regulation, linked back to the folded block through
`kb_block_provenance` — a provenance row that says, in effect, *"this lesson came from
something scarring."* It's the act that turns a painful supersession into durable guidance
instead of a silently-dropped row.

That's the scar the page opened with, now in mechanism: the architecture-doc assumption
folded, *"pair on the first PR"* written into regulation, and a `kb_block_provenance` row
tying the new lesson back to what it replaced. The charter's third question — *"where are the
sharp edges that scar newcomers?"* — is the map asking, in advance, for exactly this.

> **▣ VISUALIZATION PLACEHOLDER — `HERO` · triage / learning workflow**
> **Shows —** what an agent does when an event arrives, as a flow: **inbound event** →
> **load the charter** (telos + questions) **and regulation** → **relevance call against
> *this map's* telos** → **one of the five learning-acts** → **emit provenance-with-stance**
> back to the ledger. The **scar path** drawn explicitly as its own branch: *fold the
> question* **and** *write a regulation resource*, with the provenance link back to the
> folded block. The reader should see that growth is a loop, and that scar is the branch
> that feeds regulation.
> **Honest basis —** `kb_block_provenance` (`accretion_seq`, `is_corrected`, `source_kind`)
> and the `is_folded` gates on `kb_content_blocks` / `kb_edges` in
> [`01_schema.sql`](../../schema-artifact/01_schema.sql); the reads the agent loads —
> `resource_body_text` and `resource_blocks` (via `cogmap_telos`) for the charter and its
> questions, plus `cogmap_regulation` — in
> [`02_functions.sql`](../../schema-artifact/02_functions.sql); the five learning-acts are
> canonical from the research doc *2026-05-29 resolution-contract* (its permeable-surface
> tiers are superseded — do not draw them).
> **Fidelity —** conceptual. A workflow, not a state machine with guards. The scar branch
> is the one detail that must be unmistakable.

## The actor is an entity

A word on *who* does all this, because the modelling is deliberate. An agent is a
**persona** — a telos-bearing behaviour, a way of attending to a map. But the thing that
actually emits an event is an **entity**, and `emitter_entity_id` is `NOT NULL` on every
event. Persona is behaviour; entity is the actor of record.

`onboarding-agent#1` is an entity. Its launch details — `model: claude-opus-4-8`,
`persona: steward`, `bound_cogmap: onboarding-cogmap` — live in an **open `metadata jsonb`**,
not in a frozen `entity_kind` enum. That openness is the hinge into
[operating Temper](07-operating-temper.md): a GitHub webhook or a Notion integration is
*the same kind of entity*, writing *the same ledger*,
with its own launch metadata in the same open field. The system doesn't privilege "an
agent" over "an integration" — both are entities that emit, and that's what lets external
systems become first-class writers later.

## An open seam

What we've described is the agent's loop *once it's awake*. What **wakes it** is genuinely
unsettled — event volume, a time cadence, salience crossing a floor. It's the same seam
the opening raised about re-materializing shape, and it matters enough to be one of the
explicit "help us decide" forks in [deployment](07a-deployment.md) (the *"temper-system
dreaming"* question). We mark it here rather than pretend the cadence is solved.

---

*Next: [how maps relate](05-how-maps-relate.md) — translation without a canonical view,
and without an interior to make porous.*
