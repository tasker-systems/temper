---
tier: cognitive-maps
order: 5
label: /cognitive-maps/how-maps-relate
title: How maps relate
description: Two teams circling the same problem, unable to read each other's maps — and how a shared understanding still forms. Maps relate three ways without opening an interior: seeing that another cares (shape), borrowing its frame (delegation), and promoting a concept across scopes (a gated, curated send-forward).
register: concrete
genre: show
---

# How maps relate

> Two teams, working apart, are circling the same thing: how their sprints actually run.
> Neither can read the other's map — different scopes, different access. And yet a
> department-wide understanding of *good sprint process* obviously wants to exist, built from
> both. Maps relate three ways that make that possible without ever opening an interior:
> seeing that another cares, borrowing its frame to think with, and — when someone standing
> in both scopes chooses to — lifting a concept across into shared ground.

## Two teams, one problem

`epd-team-a`'s side-map and a map over on `epd-team-b` are, without coordinating, working the
same question: how their sprints actually go — which rituals help, where they stall, what a
healthy cadence feels like. Each is learning it under its own telos, inside its own scope.
Neither can read the other's interior; the access simply isn't there.

And yet the thing that *should* exist — a shared, department-level understanding of good
sprint process, drawing on both teams' hard-won specifics — has nowhere to live. That gap is
what this page is really about: **how do two maps that can't read each other still come to
inform, and eventually grow, one another?**

## Why there's no shortcut

The tempting answer — merge them into one canonical view — doesn't survive contact with what
a map is. Every map is telos-seeded, and what matters under one telos isn't what matters under
another. The two teams weight the same ritual differently and *both are right*, because weight
is relative to purpose. There's no view from nowhere that reconciles them.

So translation between maps is irreducible — not a gap waiting for a better merge algorithm.
The real question isn't "how do we unify the maps," it's "how do they stay legible to each
other, and pass understanding across, *without* a unified view." There are three answers, and
none of them opens an interior.

## Seeing that another map cares

The first is **shape**. A map materializes its own regions, and anyone who can read the map
can read that shape: the regions, each with a salience and a `member_count` blur — the shape
surface this set opened on. Across a boundary, that shape is **salience made visible**. One
map can see *that* another cares about something, and roughly *how much*, without seeing
*what* the caring is made of.

So side-map(team-a) and the team-b map, which share a team and sit in the same world, can each
see the other weighting a "sprint rituals" region heavily — while the material each map can
actually *read* stays its own. Legible partiality, in one line: you get the outline and the
weight, not the contents. Shape is a projection a map offers, not a peephole another map
drills.

> **▣ VISUALIZATION PLACEHOLDER — `HERO` · two maps, legible-partiality boundary**
> **Shows —** two cognitive maps (bridge-map and side-map) with a boundary between them.
> One map projects its **shape** across the boundary — regions with salience and a
> member-count blur — rendered as something the other map can *see*. The **material
> interior** (the actual member resources) stays drawn on the far side, behind the
> boundary, unreadable across it. The line the reader should feel: *"you can see that it
> cares, and roughly how, without seeing what."*
> **Honest basis —** the shape surface `cogmap_shape` (returns salience / label /
> member_count, never member identities) and `kb_cogmap_regions` /
> `kb_cogmap_region_members` in
> [`01_schema.sql`](../../schema-artifact/01_schema.sql) and
> [`02_functions.sql`](../../schema-artifact/02_functions.sql); the shared-team relation
> `cogmaps_share_a_team`, demonstrated by scenario **S5** (bridge ∩ side = true; bridge ∩
> directors = false); the materialized-shape read in **S6**.
> **Fidelity —** conceptual. The boundary and the asymmetry (shape crosses, interior
> doesn't) are what this turns on. Re-uses the cluster image from
> [*what a cognitive map is*](01-what-a-cognitive-map-is.md) at smaller scale for the
> projecting map.

## Borrowing a frame

The second is **delegation**, and it's deliberately narrow. `cogmaps_share_a_team` is true
when two maps share at least one joined team. When they do, an agent working in one map may
**borrow the other's frame** — its telos and its blurred shape — across a single bridge.
That's frame-injection: enough to think *with* another map's purpose in view.

What it is **not** is a material read. The resources an agent can actually open stay bound to
`resources_accessible_to_cogmap` for the map it's producing in — strictly stronger than "we
share a team." Sharing a team lets you borrow how another map *frames* a problem; it doesn't
let you read what's inside it. Frame and contents come apart on purpose.

## Promoting a concept across scopes

Shape and delegation let maps *see* and *think with* each other. Neither moves a concept from
one map into another — and the two teams' sprint work has matured to where that's exactly
what's needed. When a shared department concept *should* exist, something has to carry
it across. That act is **promotion**.

Promotion is deliberately not automatic, and it is not a leak. Because every resource and edge
is **homed**, a concept in side-map(team-a) stays bound to its scope; nothing about team-b or
the department gets to read it just because it would be useful. What happens instead is a
**send-forward**: an intentional, curated event that seeds a concept into a broader-scoped map
and records the relationship back to where it came from. Two things make it accountable rather
than a back door:

- **It's gated to an actor who stands in both scopes.** Only someone who is a member of teams
  participating in *both* the send-from and the send-to map can promote — they already have
  legitimate standing on each side, so nothing crosses a boundary its mover couldn't already
  cross themselves. For now that actor is a human, by choice: promotion is where
  accountability matters most, so it stays on a person's judgment rather than automated.
- **The mover curates the send-event.** Promotion isn't a copy. The actor shapes what actually
  goes forward — the concept-content seeded into the broader map, and the graph relation drawn
  back to the origin — so the shared concept is *authored*, and the private interior it draws
  on stays put, **referenced, not exposed.**

So a department-scoped concept *good sprint process* can come to exist — homed in the
department, readable there — carrying relationships to the team-a and team-b concepts that
informed it, without either team's interior being pulled across. The teams keep their
boundaries; the shared understanding finally gets a home; and from there it can be grown by
anyone in the department scope, mutually, where before it had nowhere to be.

**The signal that points the way.** The neat part is that maps already tell you *where*
promotion will pay off. Shape — the salience a map makes visible across a boundary — is a
signal of what a map cares about and how much. When a department-scoped agent sees
side-map(team-a) and the team-b map *both* weighting "sprint rituals" heavily, that mutual
high salience is a flag: there's a generalizable concept here worth lifting. Shape-projection
doesn't only describe relation; it points at where richer sharing is likely to repay someone's
attention. Taken further, the same signals could propose *new* maps and teloi over time — when
several maps keep circling the same ground, that's the system noticing an org-level purpose
that doesn't yet have a map of its own.

> **▣ VISUALIZATION PLACEHOLDER — `INLINE` · promotion / send-forward**
> **Shows —** three maps: side-map(team-a) and a team-b map below, an `epd-department` map
> above. Each lower map has a high-salience "sprint rituals" region; those salience markers,
> *visible across the boundary*, are drawn as the **signal** the department map reads. An
> actor who stands in both scopes (a person, badged as a member on each side) authors a
> **send-forward event** that seeds a new *good sprint process* concept in the department map
> and draws **relationship edges** back down to the two source concepts. Emphasis: the source
> interiors stay behind their boundaries (referenced, not copied); only the curated concept
> and its relations cross.
> **Honest basis —** homed resources/edges (`kb_resource_homes`, `kb_edges.home_anchor_*`)
> and the team/cogmap join (`kb_team_cogmaps`, `cogmaps_share_a_team`) in
> [`01_schema.sql`](../../schema-artifact/01_schema.sql) /
> [`02_functions.sql`](../../schema-artifact/02_functions.sql); the cross-boundary salience
> signal is `cogmap_shape` (**S6**). The promotion *act* — the both-scopes-membership gate and
> the curated send-event — is a **designed capability, not yet in the artifact**; draw it as
> proposed, and see the seam below.
> **Fidelity —** conceptual.

Promotion is real in the data model — homed boundaries, an actor with standing on both sides,
an event that seeds and relates — but the *protocol* around it is still open, deliberately.
Who flags a concept as a candidate, and when? How far does an agent go on a salience signal —
propose and leave the move to a person, or more? What exactly does "curate the send-event" put
in the mover's hands? These are genuine forks, not settled mechanism, and they want real use
before they're frozen. The capability is grounded; the choreography isn't drawn yet.

## The move we didn't make

There's one move we deliberately *didn't* make, and saying which one clears up a lot — because
an earlier version of this design did make it, and it was wrong. There's no permeability here:
no surface bored with tiers of access, no membrane between an inside and an outside. The reason
is upstream — **a map has no interior to make porous.** There's nothing to drill through.
Relation between maps is shape, a shared-team predicate, and a deliberately-moved concept —
all of them acts *over* the same RBAC that governs everything else, never a breach of it.
Promotion is the cleanest case of it: concepts cross between scopes **by intention, not by
osmosis.** The thing that made the porous-surface model tempting (maps need to relate)
turned out to be fully satisfiable without inventing a new kind of boundary.

## Handing off to access

This leaves the running example somewhere specific. The onboarding map is joined to
org-common — so *its* reach is itself one of these team intersections, the same machinery
side-map and bridge-map are made of. Which raises the question this thread kept leaning on and
never quite answered: *what, exactly, decides who can read what.* That's what
[what's visible from here](06-whats-visible-from-here.md) takes on.

---

*Next: [what's visible from here](06-whats-visible-from-here.md) — permission and precedence,
kept carefully apart.*
