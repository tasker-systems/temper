---
tier: cognitive-maps
order: 1
label: /cognitive-maps/what-a-cognitive-map-is
title: What a cognitive map is
description: A new engineer's first week, and the tended region of understanding that exists to get her there. What a cognitive map does — hold a purpose, keep asking, accrete what's learned, show its shape without dumping its contents — and the name we give to that.
register: evocative, concrete
genre: show
---

# What a cognitive map is

> A new engineer starts Monday; by Friday the aim is a merge she trusts. Something has to
> hold the question of *how she gets there* and keep working at it as it learns. That tended
> understanding-toward-a-purpose is what we'll come to call a cognitive map — and what it
> *does* is the better way in than what it *is*.

## A week-one problem

A new engineer joins epd-team-a. The goal that matters first is small and concrete: a pull
request she actually trusts, inside the first week. Getting her there isn't a document
anyone can hand over — it's a live question, with a few more underneath it. What does she
already know that carries over? What's the smallest real change that builds confidence?
Where are the sharp edges that scar newcomers?

Those aren't rhetorical. In the running scenario they're written down — the standing
questions of a part of the system that exists for exactly this purpose, seeded with the
goal *"help a new EPD engineer reach first-merge confidence in week one."*

## The thing doing the work

What makes it more than a folder of onboarding notes is that something *tends* it.
`onboarding-agent#1` — an agent working as a steward — holds those questions, watches what
actually happens as engineers come through, and records what it learns. When pairing a
newcomer with a maintainer turns out to head off the worst scars, that doesn't stay an
observation; it becomes something the map now *does*: a small standing rule, *pair on the
first PR.*

A purpose, the questions it keeps asking, the understanding gathering around it, and an
agent keeping the whole thing current — that working apparatus is what we call a **cognitive
map**. The name is a handle for the thing we've just watched do its job, not a category it
belongs to.

## What it's for

This is the turn `/theory` sets up. `/theory` ends on a hard claim — *the system does not
store knowledge*; it stores data and the traces of acts, and computes projections so that a
perspective engaging them can *produce* knowledge. A cognitive map is where that production
is given a home and a direction. Compressed to a sentence:

> Temper is an event-sourced coordination substrate whose organizing purpose is to be
> economical with attention. A cognitive map is a telos-seeded region of that substrate
> where humans and agents grow a shared, situated understanding together — and everything
> else is a projection over it.

The rest of this set is that sentence, working.

## What you reach for it to do

The thing you actually use a map for is to *see where understanding has gotten* without
reading everything inside it. A map offers a **shape**: from wherever you stand, you can see
the regions it has formed, how much each one matters under its purpose, and roughly how
populated each is — while the material those regions are made of stays something you reach
for piece by piece, and can be refused.

So it isn't a container with an interior you're admitted to. It's a shape it shows openly
and a set of materials you ask for individually — no membrane, no act of *entering*, nothing
bored through. That split — **shape you can see, materials you reach for** — is what later
makes maps legible to [one another](05-how-maps-relate.md) and gives
[visibility](06-whats-visible-from-here.md) something to work on.

> **▣ VISUALIZATION PLACEHOLDER — `HERO` · cluster / region field**
> **Shows —** an emergent field of concept-points that has settled into one or two
> **density regions**. Each region carries a **label** and a **salience weight** (how much
> it matters under the map's telos), and a **`member_count` blur** — you can see *that*
> roughly N things cluster here, but the individual members are deliberately not drawn.
> The honest picture of a cognitive map: a shape with weight and population, **not** a
> walled garden with a gate. For the onboarding map, one region labelled *"first-week
> confidence,"* salience high, a small member count, members fogged.
> **Honest basis —** `kb_cogmap_regions` (`centroid`, `salience`, `label`, `member_count`)
> in [`01_schema.sql`](../../schema-artifact/01_schema.sql); the surface read
> `cogmap_shape()` in [`02_functions.sql`](../../schema-artifact/02_functions.sql), which
> returns exactly *salience / label / member_count* and **never** member identities;
> members live in `kb_cogmap_region_members`, dereferenced per-member elsewhere. Scenario
> **S6** in [`04_scenarios.sql`](../../schema-artifact/04_scenarios.sql) demonstrates the
> surface returning the blur with identities withheld.
> **Fidelity —** conceptual. This is the page's emotional anchor; evoke a *field with
> weather*, not a diagram with rows.
> **For successor —** this doubles as the teaching image for "shape without interior" in
> [*how maps relate*](05-how-maps-relate.md); consider rendering it once and re-using it at
> smaller scale there.

## Why a shape and not a dump

Why reach for the shape and not the contents? Because handing over everything a map holds
would spend the exact attention the system exists to conserve — so instead it shows where
the weight is and lets you spend attention deliberately. For the new engineer's steward,
that means meeting *"pair on the first PR"* and the sharp-edges question first, not wading
through every note the map has ever absorbed. The map is economical with attention by
construction — the thesis already visible in the smallest unit.

## An open seam

One thing this leaves unsettled: **when** a map's shape gets re-drawn. The regions are
*materialized* — computed at a moment and then held — so a shape can sit slightly behind the
events that have since touched the map. The system treats that as normal and reports it
honestly rather than blocking on freshness (the staleness signal is real and on-read). *What
should wake an agent to re-materialize a shape* — event volume? a cadence? salience crossing
a floor? — is genuinely open. We mark it as a seam and return to it in
[how a map grows](04-how-a-map-grows.md) and the [deployment](07a-deployment.md) forks.
Naming a seam *as* a seam, instead of papering over it, is the discipline the system applies
to superseded thinking — and the one these pages try to keep.

---

*Next: [the substrate beneath it](02-the-substrate-beneath-it.md) — why events are primary,
and how the present is materialized from them.*
