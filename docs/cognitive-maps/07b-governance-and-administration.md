---
tier: cognitive-maps
order: 7.2
parent: 07-operating-temper
label: /cognitive-maps/operating-temper/governance-and-administration
title: Governance & administration
description: Authoring a map and reshaping the access graph beneath every map are different powers. Authoring is built; the administrative surface is org-shaped — how guarded it must be varies by organization. Administration is event-sourced (auditable by construction), with two deliberate boundaries.
register: authoring vs. administration
genre: invite
---

# Governance & administration

> Dave is a maintainer of org-common. Carol owns directors. Someone made those things true —
> added a person to a team, created the team, joined it into the right place. That isn't the same
> act as *authoring a map*, and how guarded it needs to be is one of the most organization-shaped
> decisions here. What *is* settled: every one of those administrative acts is an event, on the
> ledger, auditable by construction.

## Two different powers

Look closely at what it took to set the cast up, and two distinct powers come apart.

One is **authoring** — bringing a telos and its map into being. That's built and invariant:
`cogmap_genesis`, reachable over MCP, is the act that created the onboarding map. Authoring is
creative and relatively safe; the worst a bad map does is exist until it's folded.

The other is **administration** — adding dave to org-common, creating the directors team, joining
a team to a map, disabling a profile. These reshape *who can see what* across the whole system.
They shouldn't share a surface with authoring: the power to create a map and the power to rewrite
the access graph beneath every map differ in kind, and the second wants a different, more
deliberate door.

## How guarded is your call

Authoring writes *inside* the boundaries that already exist. Administration *moves the
boundaries*. A mistaken map is local and recoverable; a mistaken grant — a team joined to a map it
shouldn't reach, a profile enabled that shouldn't be — changes what everyone in its shadow can
read. So the administrative surface wants to be guarded — and *how* guarded is where the
organization decides.

A small, trusted team running its own deployment may be fine with a thin admin surface where the
operators are the administrators. A regulated enterprise wants the opposite: a separated,
heavily-audited plane, with approvals, with enterprise identity behind it. temperkb.io sits at the
minimal end — single-tenant, no separate administrative plane to speak of, the operators *are* the
admins. Your deployment chooses where on that spectrum it needs to be, and moves as it grows.

That choice rides an **authentication fork**. Temper already integrates OAuth (Auth0 / Okta) for
who-you-are. Administration raises whether your organization needs **SAML over and above** that —
enterprise identity, group mapping, the assurances a security team asks for before it will put
real org structure into a system. Some deployments need it on day one; others never do.

What the administrative surface must *do* is steady across all of that: provision profiles (human
and agent alike), create and disable teams, place teams in the DAG, join and remove teams from
maps. What it looks like — how separated, how audited, how authenticated — is yours.

## What administration is, on the ledger

Here's the part that's settled rather than open. Administrative acts are **events** — creating a
team, granting a team to a map, each with an emitter and a producing anchor, exactly like every
other change in the system. So governance is auditable *by construction*: every "who granted whom
access to what, and when" is already on the ledger, no separate audit log to bolt on.

Two boundaries make this precise, and they're deliberate:

- **Governance is traceable, but it isn't knowledge.** Administrative events are privacy- and
  auth-bound records, kept for **compliance**. By design they do **not** participate in cognitive
  maps, subscriptions, or resource relationships — a grant is not a concept, and the agents
  growing maps never see the governance stream as material to reason over. The two live on the
  same ledger, firewalled by intent.
- **The ledger stops at the persistence layer.** A command issued straight to Postgres can bypass
  the event stream entirely. That's not a hole in the audit — it's a **system-responsibility
  boundary**: below the application, you're in the domain of database controls and infrastructure
  policy, not Temper's ledger. (The same line is drawn from the other side in
  [observability & audit](07c-observability-and-audit.md).)

> **▣ VISUALIZATION PLACEHOLDER — `INLINE` · authoring, administration, and the audit stream**
> **Shows —** two surfaces over one ledger. **Authoring** (left): an MCP call into
> `cogmap_genesis` producing a new map *inside* existing boundaries — built / solid. **The
> administrative surface** (right): operations on the access graph — add profile to team,
> create / disable team, place team in the DAG, join / remove team↔map — drawn as
> organization-shaped (a dial from a thin operator surface to a separated, audited, SAML-backed
> plane). Both write **events** to the ledger, but the administrative events flow into a
> **firewalled compliance-audit stream** (drawn as a separate channel that does *not* feed the
> cognitive maps / subscriptions). A dashed line at the bottom marks the **Postgres
> responsibility boundary**, below which commands can bypass the ledger.
> **Honest basis —** authoring is real (`cogmap_genesis`,
> [`02_functions.sql`](../../schema-artifact/02_functions.sql)); the graph it administers is real
> (`kb_profiles`, `kb_teams`, `kb_teams_parents`, `kb_team_members`, `kb_team_cogmaps` in
> [`01_schema.sql`](../../schema-artifact/01_schema.sql)); the event ledger and producing-anchor
> shape that admin events would use are real (`kb_events`). The **administrative surface itself is
> unbuilt** (no admin functions in the artifact) and its *shape* is organization-specific — draw
> it as a proposed dial. The compliance-stream firewall and the Postgres boundary are stated
> commitments, not yet code.
> **Fidelity —** conceptual.

---

*Next: [observability & audit](07c-observability-and-audit.md) — how an operator sees the system
is healthy, and the audit it gets for free.*
