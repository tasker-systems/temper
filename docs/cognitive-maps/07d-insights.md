---
tier: cognitive-maps
order: 7.4
parent: 07-operating-temper
label: /cognitive-maps/operating-temper/insights
title: Insights
description: A PR merged on one team; minutes later a map had new regulation — and the causal chain between them is recorded and queryable, across system boundaries. Analytics is the how; the insight is the why-it-matters — a provenance of how understanding formed. What you'd ask of it is organization-shaped; the forward-exciting close to the set.
register: look what becomes possible
genre: invite
---

# Insights

> A PR merged on team-a. Minutes later, the onboarding map had a new piece of regulation. Those
> two facts are connected — and the connection is *recorded*. You can follow it: the merge woke a
> triage agent, the agent reasoned about it, the reasoning changed a concept, the change
> reinforced a charter question. The whole causal chain, across system boundaries, is queryable.
> This is where running Temper stops being a cost and starts being a payoff.

## Start with a question you usually can't ask

In most systems, *"why did our shared understanding of onboarding shift this week?"* has no answer
you can compute. The change is somewhere in a chat log, a person's memory, a commit nobody
connected to it. Temper's exhaust makes the question answerable.

Here's the chain, concretely. A webhook event arrives — *PR #123 merged* — carrying a correlation
id. A triage agent watching that topic wakes. It reads the onboarding charter, decides the merge
bears on *"where are the sharp edges that scar newcomers,"* and emits a mutation event — **with its
reasoning in the payload** — that writes a new regulation and reinforces that question. Every one
of those events shares the correlation thread back to the original merge.

So the trace exists end to end: **PR merged → triage agent woke → concept mutated, with this
reasoning → charter question reinforced** — one correlated causal chain, crossing from a remote
system into the cognitive substrate. Not a log you assemble afterward. A graph the system already
holds.

## Analytics, and the insight beneath it

Two layers sit here, and they aren't the same kind of thing. **Analytics** is the *how* — the
metrics any running system can produce. **Insight** is the *why it matters* — what those traces
let you understand about your own thinking.

The analytics are what you'd assume: resource and event lifecycle metrics, how maps grow, which
concepts churn, where attention concentrates. Useful, ordinary, and good to have — but not the
reason this page closes the set.

The insight is the chain above — the **provenance graph of how understanding formed.** Because
agents leave their reasoning in the events they emit, and because correlation
ties those events back across integration boundaries to the remote acts that triggered them, you
can query not only *what* the system believes but *how it came to believe it*, step by reasoned
step, all the way out to a merge in someone's repository. The provenance of a thought, made
queryable.

One thing this graph is *not* about: governance. The reasoning-provenance is a **cognitive**
trail — how a shared understanding formed. The record of who was granted what access lives on the
same ledger but in a separate, firewalled stream (that separation is drawn in
[governance & administration](07b-governance-and-administration.md)). The insight here is about
thought, not administration; the two don't bleed into each other.

> **▣ VISUALIZATION PLACEHOLDER — `HERO` · the correlated reasoning-provenance chain**
> **Shows —** a single causal chain drawn left-to-right across a **system boundary**. On the far
> left, *outside* Temper: a remote act — **PR #123 merged** on GitHub. It crosses the boundary as
> a webhook **event carrying a correlation id**. Inside: the triage **agent wakes**, reads the
> charter, and emits a **mutation event with its reasoning in the payload**, which **writes a
> regulation** and **reinforces a charter question**. Every node shares the same correlation
> thread, drawn as a connecting spine. The reader should see one unbroken, queryable line from a
> merge in a repo to a shift in a map's understanding.
> **Honest basis —** the threading is real: `kb_events.correlation_id`, `emitter_entity_id` (the
> agent and the integration are both entities), and the open `metadata jsonb` that can carry an
> agent's reasoning, in [`01_schema.sql`](../../schema-artifact/01_schema.sql); the reinforcement
> effect is the provenance accretion behind `cogmap_questions`' `reinforce_count`
> ([`02_functions.sql`](../../schema-artifact/02_functions.sql)). The **cross-system query and the
> assembled provenance graph are proposed** — the columns exist to support them; the analytics
> layer that reads them does not yet, and *what* a given organization queries from it is its own
> choice. Draw the chain as real, the dashboard around it as proposed.
> **Fidelity —** conceptual / illustrative.

## What's yours to ask

The capability is the architecture's; the questions are yours. What to trace, which provenance
chains repay the attention, what a dashboard over this should even show — those depend
on what your organization runs and what it's trying to learn about itself. temperkb.io doesn't
build this layer today; a deployment that cares about how its understanding evolves would. The
substrate holds the threads either way; pulling them is a choice each organization makes for
itself.

## Why this is the closing note

This is where the whole design pays off in a single direction. Event-primary made every change
answerable. Homed boundaries kept the answers honest. Agents-as-entities let outside systems write
the same ledger. Correlation threaded it all together. Not one of those choices was made *for*
insight — and yet together they produce something most systems can't: a queryable, reasoned
account of how a shared understanding came to be what it is.

That's the invitation, in the end. Not only to help build a system that grows understanding — but
to help build one that can *show its work*, and then to run it where that work is yours to read.

---

*This is where the set ends. Back up to [operating Temper](07-operating-temper.md), or return to
[what a cognitive map is](01-what-a-cognitive-map-is.md).*
