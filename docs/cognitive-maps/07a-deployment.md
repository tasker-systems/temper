---
tier: cognitive-maps
order: 7.1
parent: 07-operating-temper
label: /cognitive-maps/operating-temper/deployment
title: Deployment
description: The 0→1 bootstrap is the seed file itself and doesn't vary; everything after — topology, tenancy, per-tenant integration, where agents run — is where one organization's deployment diverges most from another's. temperkb.io is one near-minimal shape; a private deployment chooses its own.
register: answerable, not answered
genre: invite
---

# Deployment

> The very first thing that exists is the seed file — `temper-system` and a `system-default`
> map, the floor everything stands on. That part is the same everywhere. After it, getting to
> one map and then many is a path every deployment walks — but *how* it walks it, and on what
> infrastructure, is where one organization's Temper diverges most sharply from another's.

## Zero to one

Standing up the first Temper is a short list with one elegant property at the end.

A Postgres instance, with the schema loaded. Temper itself, serving its three surfaces — the
CLI, the API, and the MCP server that agents and integrations speak to. And then the first telos,
seeded.

The elegant property: that first seed isn't a special bootstrap script. It's the same
`temper-system` root team and `system-default` map the seed file already creates — the public
floor every later team descends from and every enabled profile joins. The 0→1 picture is
literally the seed you've been reading; the system's first act is to describe itself in its own
terms. That much is invariant — it looks identical on every deployment.

What's *already* a choice is the substrate it loads into. The public deployment runs "Postgres +
Temper serving its surfaces" as Vercel functions over a Neon database; a private deployment might
run it as containers over a database it operates. The seed is the same; the ground it lands on is
yours to pick.

## One to many

Growth from there has three motions, visible already in the cast:

- **New maps, by authoring.** A telos and its charter come into being through `cogmap_genesis`,
  reachable over MCP — exactly how the onboarding map was born. A solved, callable act.
- **Raw events, by integration.** A GitHub webhook writes events into the ledger as they happen —
  a PR merged, an issue closed — as a pure data stream, no map attached yet. The integration is
  an entity, the same kind of actor as an agent.
- **Attention, by agents.** Agents wake on some signal, read the maps they're bound to, and do
  the growing — triage, regulation, the five learning-acts.

The first motion is invariant — it's a function you call. The second and third are where the
shape of *your* deployment starts to matter.

## Where deployments diverge

This is the dimension that varies most between organizations, so here are the axes concretely —
using the public deployment as the near-minimal reference point:

- **Topology.** Serverless edge functions (temperkb.io: Vercel) versus containers in a cluster.
  The architecture runs on either; the operational properties — scaling, cold starts, where
  long-running agents live — differ, and so does what your organization already knows how to
  operate.
- **Tenancy.** temperkb.io is effectively single-tenant. A private deployment usually asks the
  opposite question: one organization, yes, but often many internal sub-tenants — divisions,
  customers, environments — with the isolation and per-tenant data boundaries that
  single-tenant temperkb.io doesn't draw today.
- **Per-tenant integration.** Webhook subscription *by tenant* — so each tenant's GitHub or
  Notion feeds only its own maps — is something a multi-tenant private deployment needs and the
  public one doesn't currently do. The event-shape contract makes it possible; wiring it per
  tenant is a deployment's own work.
- **Agent infrastructure.** Where the waking agents actually run. Vercel offers agent mechanisms;
  a dedicated managed-agent platform (Anthropic's, for instance) is not identical, and a private
  deployment may choose differently again, or run its own. The agent is an entity either way;
  *where it executes* is yours.

None of these has a single right answer the project could publish for you. They're the shape you
choose on the way in, and re-choose as you grow.

> **▣ VISUALIZATION PLACEHOLDER — `HERO` · the 0→1→N path, and the shape range around it**
> **Shows —** two layers. (1) The **invariant path**: a phased timeline — empty Postgres →
> schema loaded → Temper serving CLI / API / MCP → first seed (labelled *"this is the seed
> file"*) → 1→N (a new map via authoring, a webhook stream arriving, an agent waking). (2)
> Around it, the **deployment-shape range**: the same path drawn at two points — a *near-minimal*
> shape (serverless / Neon / single-tenant / platform agents, tagged "temperkb.io") and a
> *fuller* shape (cluster / operated DB / multi-tenant + per-tenant webhooks / dedicated agent
> infra). The reader should see that the bootstrap is identical everywhere, while the topology,
> tenancy, integration, and agent-execution choices slide along a range.
> **Honest basis —** the 0→1 floor is real: `temper-system` + `system-default` in
> [`03_seed.sql`](../../schema-artifact/03_seed.sql); authoring is `cogmap_genesis`
> ([`02_functions.sql`](../../schema-artifact/02_functions.sql)); integrations-as-entities is
> `kb_entities` + `emitter_entity_id NOT NULL` + `correlation_id` + the nullable producing anchor
> ([`01_schema.sql`](../../schema-artifact/01_schema.sql)); `kb_topics` seeds topic bounds. The
> **topology / tenancy / per-tenant-subscription / agent-platform choices are operational, not in
> the artifact** — draw them as a range, with the temperkb.io point annotated from its actual
> stack (Vercel functions, Neon, single-tenant). The contract and trigger mechanisms are designed,
> not yet built — draw as proposed.
> **Fidelity —** conceptual / illustrative.

## The standing machinery

For the integration and agent motions to run continuously, a few things have to be in place — and
each is where a tenancy or platform choice lands:

- **Integrations as ledger writers.** An outside system is an entity with permission to append
  events, and its events satisfy a shared shape — an emitter, a type, a correlation thread — so a
  triage agent can pick them up later. That shape is the **event-shape data contract** (below);
  *which* systems write, and *for which tenant*, is the per-tenant integration choice above.
- **Triggers for agent sessions.** Something decides when an agent wakes — the
  **trigger-threshold** fork.
- **Topic bounds for subscription.** An agent watching for events needs a formal way to say
  *which* events it cares about — a topic scope it subscribes to — so a busy ledger doesn't wake
  everything for everything. In a multi-tenant deployment, that scope is also how a tenant's
  agents stay bound to a tenant's events.

## The forks here

**The event-shape data contract.** This is the boundary where Temper becomes infrastructure your
other systems emit into. The architecture fixes its floor — every external writer is an entity,
`emitter_entity_id` is never null, so a GitHub source is a row of the same kind as
`onboarding-agent#1`. What a *deployment* settles is the rest: what an event must carry to be
admissible, how raw a webhook may be before a triage agent has to make sense of it, and how the
whole thing is partitioned per tenant. It sets how far Temper reaches into the tools your
organization already runs.

**What wakes an agent.** A triage session is triggered by *something* — event volume past a
threshold, a time cadence, salience accumulating past a floor — and the broader rhythm of
sweeping a map to keep it coherent (the *temper-system dreaming* we keep naming) is undecided on
purpose. Too eager and the system thrashes; too lazy and maps go stale. The right cadence depends
on the map, the traffic, and the organization's tolerances — which is why it's a dial you set and
re-set, not a constant the project ships.

## What's invariant, what's yours

The 0→1 path is invariant and solid — it's the seed, loadable today, identical everywhere.
Authoring new maps is invariant — a function you call. Everything that gives a running Temper its
*operational* character — topology, tenancy, per-tenant integration, where agents execute, the
waking cadence — is yours, varies by organization, and moves over time. Answerable, in other
words. Answered differently by each deployment that asks.

---

*Next: [governance & administration](07b-governance-and-administration.md) — who may create a
map, and who may move the boundaries beneath it.*
