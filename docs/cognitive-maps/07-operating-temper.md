---
tier: cognitive-maps
order: 7
label: /cognitive-maps/operating-temper
title: Operating Temper
description: The architecture is only half of a running system; the other half is the shape an organization gives it, which varies between organizations and evolves over time. What temper-next fixes vs. what a deployment shapes, temperkb.io as one near-minimal point on the range, and the decisions a private deployment comes to own.
register: invitation
genre: invite
---

# Operating Temper

> Everything so far showed an architecture whose shape is proven — help us finish it. Operating
> it is a different kind of question, because the architecture is only half of a running system.
> The other half is the shape a particular organization gives it — its topology, its tenancy,
> its agents, its rules — and that shape varies between organizations and shifts over time. The
> public deployment you may be reading this on is one such shape, and a deliberately small one.

## Someone had to stand it up

We go back to the onboarding map one more time and ask the question the show pages stepped over:
how did it come to exist *at all*?

Something had to be running first. A Postgres instance. Temper itself, serving its CLI, API, and
MCP. Dave provisioned, org-common created, a webhook wired, and a threshold that woke
`onboarding-agent#1` for the first time. None of that appears in the story the show pages told,
and all of it had to happen for the story to be possible.

But "something had to be running" hides a choice: *running how, and where?* On the public
deployment that's Vercel functions over a Neon database; in your organization it might be
containers in a cluster, a database you operate, agents on a platform you pick. The cognitive
map looks the same from inside either way. Underneath, the operating shape is a decision — and
mostly distinct by the needs of where it is being deployed and for whom.

## What's fixed, and what's yours

It helps to pull apart two layers the word "Temper" runs together.

The **architecture** — what these pages have described as temper-next — fixes a specific set of
things, and they don't vary by deployment: events are primary, the kernel is
convention-agnostic, access is teams-RBAC over homed boundaries, actors are entities, and every
writer (agent or integration) meets the same event shape. Adopt Temper and you adopt those.

The **operating shape** is everything else, and it's a *range*, not a point. The architecture
has a minimum viable form — small, single-tenant, serverless — and a much larger one —
multi-tenant, per-tenant integrations, dedicated agent infrastructure, deep observability. Every
real deployment sits somewhere on that range and moves along it as the organization grows. So
the pages here *invite* in two senses: help us, the project, refine the architecture and the
range it admits — and, when you run Temper privately, own these operational decisions yourself,
revisiting them as your needs change. The questions are real and the mechanisms mostly known;
the answers are shaped by how your organization needs to run.

## temperkb.io is one point on the range

A concrete example, since it's probably in front of you. `temperkb.io`, the public deployment, is
one shape — and a near-minimal one. It runs on Vercel serverless functions over a Neon database,
routed edge functions rather than containers in a cluster. It's single-tenant: it isn't set up
today for the multitenant choices, or the per-tenant webhook subscriptions, that a private
organizational deployment would want. Its agents run on the mechanisms Vercel offers, which are
not the same as a dedicated managed-agent platform. (It's also the *current* public version,
while temper-next — the architecture you've been reading — is the destination; even its own shape
is one option among the range temper-next opens.) None of that is specifically a flaw. It's a *choice of
shape*, near the small end of what the architecture allows, and a useful picture of what a
minimum looks like. Your deployment gets to choose differently, and to change its mind later.

## Four dimensions you'll shape

The operating story splits four ways, each a dimension a deployment shapes — and each with its
own texture:

- **[Deployment](07a-deployment.md)** — topology, tenancy, and how new maps, integrations, and
  agents come online. The dimension where one organization's Temper diverges most from another's.
- **[Governance & administration](07b-governance-and-administration.md)** — who may create a map,
  and who may reshape the teams and grants beneath it. How guarded that second power must be
  depends on the organization.
- **[Observability & audit](07c-observability-and-audit.md)** — how an operator sees the system
  is healthy, plus the audit the ledger gives for free. Which metrics matter is an organizational
  call.
- **[Insights](07d-insights.md)** — what becomes *possible* once agents leave correlated,
  reasoned traces. There's a payoff hiding in the operating layer — the exhaust from running
  Temper is one of the more interesting things it produces — and what you'd ask of it varies with
  what you're running. The forward-looking close.

## The decisions, and who owns them

Three decisions cut across those dimensions — two still open, and one we've settled and would
rather state plainly than leave you to guess. Each has a part the architecture fixes and a part a
deployment shapes:

1. **The event-shape data contract** *(open).* *The architecture's part:* every external writer
   is an entity and every event meets a shared shape — fixed. *Your part:* which integrations you
   wire in, what their events carry, how much raw signal you admit before an agent makes sense of
   it. This is the boundary where Temper becomes infrastructure your other systems emit into.
   (Deployment goes deeper.)
2. **Administration is event-sourced** *(settled, with boundaries).* Creating a team, granting a
   team to a map — these are *events*, with an emitter and a producing anchor, so governance is
   auditable by construction. Two deliberate limits, though. They're privacy- and auth-bound
   records kept for **compliance**, and by design they do **not** participate in cognitive maps,
   subscriptions, or resource relationships — governance is traceable, but it isn't knowledge. And
   they stop at the persistence layer: a command issued straight to Postgres can bypass the
   ledger, which is a system-responsibility boundary, not a gap. (Governance and audit carry
   this.)
3. **What wakes an agent** *(open, and mostly yours).* Event volume, a cadence, salience crossing
   a floor — the rhythm of waking and sweeping a map (the *temper-system dreaming* we keep naming)
   depends on your traffic and your tolerances, and you'll re-tune it over time. (Deployment,
   again.)

## One thing we're not pretending

A straight answer for a security-minded reader, so trust isn't lost later: **v1 assumes
good-faith actors.** We name that as a bracket rather than hide it, because it's real — the
features that make a knowledge substrate good at *discoverability* (surfacing the right concept
to the right reader at the right moment) are close to isomorphic with the features that make it
good at *reconnaissance*. The RBAC and homed-boundary work genuinely gates access; what v1 does
not yet model is an actor working *against* the system from inside its good-faith assumptions.
How much that matters is itself partly a deployment question — a trusted internal team is a
different threat surface than an open one — but if your context makes that adversary real, it's a
conversation we want early, not a surprise you find later.

---

*Next: [deployment](07a-deployment.md) — the dimension where one organization's shape diverges
most from another's.*
