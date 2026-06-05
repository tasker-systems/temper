---
tier: cognitive-maps
order: 7.3
parent: 07-operating-temper
label: /cognitive-maps/operating-temper/observability-and-audit
title: Observability & audit
description: Is the system healthy, and what did it know and why did it change — two questions, two homes. Operational audit lives in external tooling and is scoped to an organization's needs; epistemic audit is the ledger itself. Known mechanism, organization-shaped scope, one responsibility boundary.
register: settled mechanism, open scoping
genre: invite
---

# Observability & audit

> The onboarding agent woke, read a charter, and wrote a regulation. Two fair operator questions
> follow: is the system that did this *healthy* — and, separately, can we reconstruct *what it
> knew and why it changed its mind*? Those are two different kinds of audit, in two different
> homes. The mechanism for each is known; *what* to capture is the part your organization scopes.

## Two questions that sound alike

An operator watching Temper run has two kinds of question, easy to file together even though they
want different tools.

The first is **operational**: is it up, is it fast, is it erroring? Calls, latencies, failures —
the ordinary health of a running service. The path is known. Temper already carries tracing, the
patterns are well-worn OpenTelemetry tooling for observability, and the same approach extends to the
CLI, which would give an organization cross-usage visibility into how Temper is actually used.

The scope, though, is yours. *Which* metrics matter — what an organization watches, alerts on, and
retains — is a decision that varies and evolves; Prometheus metrics are undefined here not because
the mechanism is missing but because the choice is genuinely organizational. temperkb.io captures
little of this today; a larger deployment captures far more. It's a low-risk place to contribute,
and a dial each operator sets for themselves.

The second question is **epistemic**: what did the system know, when, and why did a concept
change? That one isn't a tooling question at all.

## The audit you get for free

The epistemic audit is the ledger itself. Because every change is an event — every assertion,
fold, reinforcement, and scar, each with its emitter and its place in a correlation thread — the
question *"why does this concept look the way it does, and what shaped it"* is answered by reading
the substrate, not by bolting on an audit log. The system that grows understanding and the system
that records *how* understanding formed are the same system.

There's a second audit on that same ledger, kept deliberately apart: the **governance** trail —
who was granted what, and when. Administrative acts are events too, but they're compliance records,
firewalled by design from the cognitive stream (the reasoning behind that separation is in
[governance & administration](07b-governance-and-administration.md)). So the inside-the-substrate
audit is really two streams that don't mix: *how understanding formed*, and *who was allowed to do
what*. Keeping them apart is what lets each answer its own question cleanly.

So the audits live in distinct homes, and that division is the design: operational audit in
external tooling, outside; epistemic and governance audit in the event ledger, inside. Running
them together is the mistake; keeping them apart is what lets each be good at its job.

> **▣ VISUALIZATION PLACEHOLDER — `INLINE` · the audits and their homes**
> **Shows —** three lanes around the system boundary. **Operational** (outside): a running Temper
> emitting traces and metrics — calls, latency, errors — into external tooling
> (OpenTelemetry / Prometheus), with a note that *which* metrics is an organization-scoped dial.
> **Epistemic** (inside): the event ledger as the trail of *how understanding formed* —
> assertions, folds, reinforcements, scars. **Governance** (inside, firewalled): administrative
> events — *who was granted what, when* — on the same ledger but in a separate compliance channel
> that does not feed the cognitive maps. A dashed line beneath everything marks the **Postgres
> responsibility boundary**, below which direct database commands fall outside the ledger
> entirely.
> **Honest basis —** the epistemic and governance streams are real on `kb_events`
> (`emitter_entity_id`, `correlation_id`, producing anchor) with the fold / provenance trail
> (`kb_block_provenance`, `is_folded`) in [`01_schema.sql`](../../schema-artifact/01_schema.sql).
> The operational side (OpenTelemetry / Prometheus) is **external tooling, not in the artifact**,
> and its metric scope is organization-specific. Draw operational as outside / proposed.
> **Fidelity —** conceptual.

## One non-goal we'd rather name

There's a limit here and it's the same line governance draws from its side.
Audit at the **Postgres boundary**, protecting against someone with direct database access,
is out of scope on purpose. Anyone with admin access to the database can read
everything, and at that point you've left what Temper's RBAC and ledger can speak to. Compliance
for *that* threat is an **extra-system** concern — database-level controls, infrastructure policy —
and how it's handled depends entirely on how your organization runs its data layer. Better to be
clear about where the system's guarantees stop than to imply they reach further than they do.

---

*Next: [insights](07d-insights.md) — what becomes possible once agents leave correlated,
reasoned traces.*
