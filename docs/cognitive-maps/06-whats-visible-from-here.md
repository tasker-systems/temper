---
tier: cognitive-maps
order: 6
label: /cognitive-maps/whats-visible-from-here
title: What's visible from here
description: Visibility = permission × precedence — two orthogonal answers the page keeps apart. Two kinds of reader (a person vs. a map), down-DAG inheritance, leak-safety, edge-home protection; then salience, where the attention-economy lives.
register: precise
genre: show
---

# What's visible from here

> Alice, Bob, and Nomad each ask the same thing — *what can I see here?* — and get three
> different answers. Same system, same question, three different worlds. The reason isn't one
> rule but two, and they're easy to mistake for each other: what you may see, and — within
> that — what ranks.

## Three people, three worlds

Put the same question to three of the cast: *what can I see?*

**Alice**, on team-a, sees her team's working docs, the org-wide policy every team shares, a
document shared with her by name, and the public concepts on the floor. **Bob**, on team-b,
sees the org policy and that same public floor — but not team-a's private work; he was never
granted it. **Nomad**, enabled for nothing and owning nothing, sees *nothing at all.*

Same system, same query, three different worlds. And once you ask *why* each set looks the way
it does, a second question separates out from the first: not only *what may I see*, but — of
the things I may see — *what comes up first.* Those are different machinery, and most confusion
about "what an agent can see" comes from running them together.

## Permission × precedence

So: two orthogonal answers. **Permission** decides what's in your set at all — the hard,
structural yes/no that makes Alice's world and Bob's differ. **Precedence** decides, within
that set, what surfaces first — a graded, earned ranking. Permission comes first here (it
decides what's even possible); precedence closes (it's where the attention-economy lives).

## Two kinds of reader

Two very different things read from the substrate, for different reasons — and the access
question has a different answer for each.

A **person** reads to *use* what's there: Alice opening docs, Bob checking the policy. This is
the **consumer** side — what you may read follows from who you are, your team memberships and
anything granted directly to you, inherited *down* the teams DAG. It's the function behind the
three worlds above: `resources_visible_to(you)`.

An **agent** reads to *do a map's work*: the onboarding steward growing its map, materializing
its shape, deciding what to write into regulation. This is the **producer** side, and the move
that matters is that a producing agent reads as the **map**, not as whoever launched it — with
reach `resources_accessible_to_cogmap(map)`, the least-privilege *intersection* of the map's
joined teams (why it's an intersection is the next thing).

Tying the agent's reach to the map rather than to its launcher closes a real risk. If a
producing agent inherited the access of the person who started it, you could launch one inside
a narrowly-scoped map from a broadly-privileged account and quietly let that map pull in — and
be shaped by — material it has no business touching. So every substrate read carries exactly
**one principal**: one identity behind the read, a person *or* a map, never both. And it holds
structurally, not as a rule someone has to remember — the read function has no parameter for
*the human behind the agent*, so the map's agent sees what the map may see, and who happens to
be at the keyboard can't enter the question at all.

## More teams, narrower reach

The producer axis is an **intersection**, and that inverts the usual intuition. A map
joined to `epd-team-a` *and* `epd-team-b` can read only their **common ground** — a
resource has to be visible to *every* joined team to be in reach. So `team-a-private` is in
side-map's reach (joined to team-a alone) but **falls out** of bridge-map's reach (a∩b),
even though bridge-map "has" team-a. More teams means *narrower*, not wider.

The empty case follows the same logic to its end: a map joined to **no** team has an empty
intersection and reads **nothing** shared — default-**CLOSED**, not default-open. (Its own
homed resources stay readable; that's map-home, not a grant.) Scenario **S2** runs exactly
this.

## Down-only inheritance, and the root floor

On the consumer side, grants inherit **down** the DAG: a grant on a team is readable by
that team and everything descending from it, never upward into a parent's privates. Every
team descends from the `temper-system` root, so a grant on root is a **universal floor** —
the public baseline everyone with access shares.

"With access" is itself a real membership, not a special case: enabling a profile
auto-joins it to the root team via a maintained `kb_team_members` row, so the floor falls
out of the same mechanism as every other grant. Scenario **S1** is the proof — alice (team-a)
and bob (team-b) see overlapping-but-different sets; **nomad**, enabled for nothing, owning
nothing, sees *nothing at all*.

## Leak-safety

One invariant makes the whole thing safe to extend to per-person grants: a
**profile-anchored grant is consumer-axis only**. It can let a *person* read a specific
resource, and it **never** enters a team's visibility set or a producer intersection. So
`shared-with-alice` is readable by alice and is in **no map's** reach — a personal grant
can't be laundered into a cogmap or leaked across a team boundary. That's what makes
admitting profiles as grantees safe rather than a hole. **S2** checks this too.

## Edge-home protection

An edge is access-gated like everything else: it's *homed*, and its home gates whether you
can even see *that the assertion exists*. This produces a result that's easy to miss. The
directors' `leads_to` edge runs between two **public** concepts — both endpoints readable by
anyone — but the edge is homed in directors-map, so it's invisible to anyone who can't read
that map. Both nodes visible, the link between them not. Scenario **S3**: carol (∈ directors)
sees it; alice and nomad don't, *despite* alice being able to read both endpoints.

> **▣ VISUALIZATION PLACEHOLDER — `HERO` · seed DAG + the two axes**
> **Shows —** the full seed world: six teams as a DAG (`temper-system` root →
> {org-common, epd-department, directors}; team-a and team-b each descending from
> epd-department *and* org-common), and the five cogmaps joined to their teams. Overlaid,
> the **two axes** as two distinct reading-motions: the **consumer axis** as a *person*
> reading **down** the DAG (grants inheriting downward), and the **producer axis** as a
> *map* reading the **intersection** of its joined teams (highlight bridge-map = team-a ∩
> team-b shrinking the reachable set). The reader should see one structure read two
> different ways.
> **Honest basis —** `resources_visible_to`, `vis_team`,
> `resources_accessible_to_cogmap` in
> [`02_functions.sql`](../../schema-artifact/02_functions.sql); the teams DAG
> (`kb_teams_parents`) and the five cogmaps in
> [`03_seed.sql`](../../schema-artifact/03_seed.sql); verdicts **S1** (consumer) and **S2**
> (producer / leak-safety) in [`04_scenarios.sql`](../../schema-artifact/04_scenarios.sql).
> **Fidelity —** illustrative. Real teams, real cogmaps, real verdicts; the two
> reading-motions are the teaching point.

> **▣ VISUALIZATION PLACEHOLDER — `INLINE` · two-gates panel**
> **Shows —** the two gates that are *easiest to conflate*, side by side, as two
> differently-coloured arrows into the same map: (1) **membership-based shape-readability** —
> `cogmap_readable_by_profile`, "are you a member of a team joined to this map" — which
> gates seeing the map's *shape*; and (2) **down-DAG grant-inheritance** —
> `resources_visible_to`, "has a resource been granted to a team you reach" — which gates
> reading *resources*. Different questions, different functions, different answers; the
> panel's whole job is to keep them from blurring.
> **Honest basis —** `cogmap_readable_by_profile` vs. `resources_visible_to` in
> [`02_functions.sql`](../../schema-artifact/02_functions.sql); the edge-home traversal
> gate `edges_visible_to` and scenario **S3** for why the distinction bites.
> **Fidelity —** conceptual. Two arrows, two colours, one map.

## Salience: the other answer

Everything above is permission — binary, structural, the same for everyone who shares your
position. **Salience** is the orthogonal answer: within what you *may* see, what *ranks*.
It's `weight` on edges and properties, `salience` on regions, reinforce-count on questions —
all graded, all earned, all changing as the map learns.

This is the attention-economy in operation. *What surfaces first is an attention decision,
not a permission one.* Permission says you're allowed to see ten thousand things; salience
is why you meet the three that matter under this telos first. Keeping the two apart matters
because they fail differently: a permission bug leaks data, a salience bug wastes attention
— and the system takes the second problem as seriously as the first.

## Runnable proof

None of the verdicts on this page are assertions to take on trust. S1, S2, and S3 are
labelled queries in [`04_scenarios.sql`](../../schema-artifact/04_scenarios.sql) that run
against the seed and print exactly the behaviour described — alice's set, bob's set,
nomad's nothing, the intersection that drops team-a-private, the private edge carol sees and
alice doesn't. The model is meant to be *checked*, which is exactly why the artifact gets
built before the migration.

---

*Next: [operating Temper](07-operating-temper.md) — the map had to be stood up somewhere,
and that's a good problem. The invitation begins.*
