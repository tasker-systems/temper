# External systems as subscribed emitters

**Status:** design · **Date:** 2026-07-13 · **Goal:** third-party systems become entities that emit
unjudged facts into the event ledger; teams subscribe to the aspects they own; stewards glean under
their own telos.

---

## The claim

A GitHub PR merging, a Linear ticket moving, a Notion page changing — these are **events in the
world** that temper currently cannot see. This goal makes them first-class: a remote system becomes
an **entity that emits into `kb_events`**, its payload is preserved verbatim, and the set of teams
and cognitive maps it touches is computed at intake from **declared subscriptions**.

Nothing is fetched at intake, and nothing is auto-created. A webhook does not become a document, a
node, or an edge. It becomes **one unjudged fact plus a list of who should look at it**. What each
subscriber makes of it is judgment, it is governed by that subscriber's telos charter, and it
happens on the steward's next tick.

## What already exists

This design is mostly **activation of latent affordances**, not net-new architecture. Grounded
against live production (2026-07-13) and the migration set, not against issue text:

| Affordance | State |
|---|---|
| `kb_events.emitter_entity_id → kb_entities` | **An emitter is an entity, not a profile.** Entities are `<handle>@<surface>`, each backed by a profile. "A third-party system is an entity that emits events" is already the schema's shape. Prod: 10 entities (2 profiles × 4 surfaces + `system`, `migration`). |
| `kb_event_types.payload_schema` | Nullable, and the column comment says why: *"NULL = unregistered/permissive — **foreign/webhook types may stay NULL**."* The permissive path for a foreign body is reserved and unused. |
| `kb_events.references` | `[{rel, target:{kind,id}}]`, rel vocabulary `supersedes \| derived_from \| **touches**`. GIN-indexed (`jsonb_path_ops`). **Written by nothing: 0 of 11,952 prod events.** The Rust types (`RefRel`, `EventReference`) were since deleted as dead scaffolding; the column and index survive. |
| `kb_remote_sources` | `(uri, uri_normalized UNIQUE, first_seen)` — hands a remote URL a UUID. `provenance_source_kind` already carries a `'remote'` variant. A remote PR can be **cited without being fetched**. |
| `kb_machine_clients` | The machine-principal allowlist. Prod: **1 row** (the steward). |
| `kb_access_grants` | `(subject, principal, can_read/write/delete/grant)`. `subject_table CHECK IN ('kb_resources','kb_contexts','kb_cogmaps')`. |
| `team_role` | `ENUM ('owner','maintainer','member','watcher')`. `is_system_admin(profile_id)` exists. |
| Steward loop | `kb_cogmaps.steward_watermark_event_id` + `steward_ingest_delta` + `steward_drift_sweep` + `kb_workflow_jobs` (claim/lease/reap, single-flight). A **pull-on-a-cron** watcher. |
| `internal_sig.rs` | HMAC-SHA256 discipline, first-party (the SAML reconcile channel). Its own comment notes it copies webhook signature practice. |

**What does not exist, anywhere:** no inbound third-party webhook receiver, no connection, no
subscription, no blast radius, no remote tool brokering. Verified by sweep, not assumed.

### The `references` column is the load-bearing repurposing

A blast radius names **teams and contexts**. `kb_edges` endpoints are
`CHECK (… IN ('kb_resources','kb_cogmaps'))` — **an edge structurally cannot point at a team or a
context.** `references.target` is `{kind, id}`, polymorphic, and can.

The second leg is the membrane: the curated edge graph feeds the region clusterer and salience.
Writing a dozen mechanical edges per PR would **drown the curated corpus** — the exact failure the
SCIP goal's membrane invariant exists to prevent. Blast radius is unjudged mechanical fact. It
belongs on the event.

**Verified empirically against prod:** a `touches`-containment lookup
(`"references" @> '[{"rel":"touches","target":{"kind":"kb_cogmaps","id":…}}]'`) plans as a
`Bitmap Index Scan on idx_kb_events_references`. The index that nothing has ever written to is
exactly the index the steward-wake query needs.

**This answers an open decision in another goal.** *The ledger as a readable surface* (`019f51e3`)
has as an acceptance criterion: *"`kb_events.references` is either populated and consumed, or gone.
It is not left as an indexed column that nothing writes."* It could not be settled because nobody
could name a lineage that must live on the event and cannot be an edge. **A webhook's blast radius
is that thing.** This goal resolves that criterion in the affirmative: populated, and consumed.

---

## Invariants

These are load-bearing. A change that breaks one is a change to the goal, not an implementation
detail.

**1. The membrane.** Inherited from the SCIP goal, generalized by one word: *the **external ledger**
is the unjudged record. All judgment lives in the curated graph and cites into it.* Intake writes no
edges, no resources, no nodes.

**2. The ledger records receipt, never elaboration.** One webhook is **one event**. What is *inside*
the payload — every symbol a commit moved, every field a ticket changed — is unfolded into
purpose-built tables as an **outcome** of processing that event. It is never a stream of events.
This is what keeps the ledger from being littered, and it is how SCIP fits: the *event* is the
trunk-merge webhook; the `kb_code_*` projection is its outcome.

**3. Intake never fetches.** Receipt is: verify → attribute → match declared subscriptions → append
one event → ack. No network egress to the remote system. Anything requiring a fetch is deferred
enrichment.

**4. Radius is declared, never inferred.** Blast radius is the set of **subscriptions a payload
matched** — an indexed lookup, not a guess about who might care. A payload matching zero
subscriptions has **radius = ∅**: it is still stored (append-only, cheap) and routes nowhere. There
is no separate noise-filter to build; **the empty radius *is* the filter.**

**5. Ingest once, glean many.** One event, N subscribers, N different distillations. An EPD-wide map
and a single team's map may subscribe to the same hook and legitimately take very different facts
from it — the EPD map learns that a feature area is moving; the team map distils a specific
architectural consequence. **Distillation is judgment, and judgment is telos-governed.** The event is
shared; the gleaning is not.

**6. Silence must never encode absence of capability.** The hardest-won lesson of the SCIP goal, and
it recurs here in **four** independent places:
   - a **provider** whose payload cannot answer the question (GitHub's `pull_request` payload carries
     no changed-file list);
   - an **enrichment** that failed (token expired, rate-limited, repo access revoked);
   - a **subscription** with no credential (`needs_credential`);
   - a **connection** with no reach (ledger-capable but not reach-capable).

   In every one, a consumer must be able to distinguish *"nothing you subscribe to was touched"* from
   *"I could not see whether anything you subscribe to was touched."* Capability is **declared** —
   per provider, per event type, per radius grain — and its absence is **loud**.

   **The dual holds too: over-broad reach must never be silent.** Because brokering keeps temper out
   of the call path, an agent's *remote* reach is bounded by the **provider's** scoping granularity,
   not by temper's finer authz. Where an agent's remote reach is **coarser** than its temper reach,
   the connection **declares it**. A coarse app-level connector is acceptable; an *undeclared* one is
   not.

**7. Nothing is auto-created.** No document appears in a context, no node appears in a cognitive map,
because a webhook arrived. Ever.

---

## Architecture

### The connection (admin-provisioned)

A **connection** is temper's authed link to a remote system — a GitHub App installation, a Linear
workspace, a Notion integration. It carries three things, and it is born incomplete:

```
connection
  provider        github | linear | notion | …
  credential      → the token broker (see Reach)      [may be absent: needs_credential]
  webhook         registered event types              [ledger-capable]
  tool manifest   declared, read-only remote tools    [reach-capable]
  entity          the emitter: <connection-handle>@webhook, backed by a machine principal
  home            a context — authz inherits for free
```

**Two capability tiers, separately provisioned, both explicit:**

- **Ledger-capable** — the webhook is registered. Events land. Facts accrue. Useful on its own.
- **Reach-capable** — agents can read the remote system back. **Judgment becomes possible.**

A subscription against a ledger-only connection is legal and useful — you still get the durable
record — but it is **inert for judgment**, and that state is visible rather than inferred from an
agent mysteriously producing nothing. This is invariant 6 applied to the connection itself.

A connection is born in `needs_credential` and becomes ledger-capable and/or reach-capable as those
are provisioned. It never silently pretends to be more than it is.

### The subscription (team owner/maintainer-authored)

```
subscription
  subscriber   (kb_contexts | kb_cogmaps | kb_teams)
  connection   → an admin-provisioned, team-granted connection
  selector     per-provider; the thing the subscriber actually cares about
```

The selector is where the CODEOWNERS decision lives, and it is deliberately **thin**:

> *"We are `@cool-team-name`. We subscribe to repo Y. Tell us when a PR hits **our** CODEOWNERS
> paths."*

temper does **not** store "team X owns paths Z in repo Y." That would fork ownership into a second
place that drifts the moment someone edits `CODEOWNERS`. **Ownership stays in the repo, where it is
already maintained and already reviewed.** The price — and it is the right price — is that evaluating
the selector *requires an authed fetch*, which is what forces the enrichment phase below into
existence. It is not an optional nicety; it is structural.

### Intake

```
POST /webhooks/<provider>
  → verify signature
  → resolve emitter          the connection's entity
  → match subscriptions      declared, indexed; coarse (payload-only)
  → append ONE event:
        event_type   "github.pull_request.merged"   (payload_schema NULL — the reserved path)
        payload      the raw body, verbatim
        anchor       the connection's home context
        references   [{rel:"touches", target:{kind:"kb_cogmaps", id:…}}, …]   ← matched subs only
  → project delivery rows    one per matched subscription
  → ack
```

An event has exactly **one** `producing_anchor`, so a payload matching three subscribers cannot be
anchored three ways. The split is natural: the **anchor** is where the *connection* is homed (the
receipt fact, one place); the **references** are the fan.

### The delivery row — an unread queue *with* a disposition

A watermark is a single cursor: it carries one bit, and we need three outcomes — *not yet seen*,
*seen and acted on*, *seen and deliberately declined*. Without the third, this is an unread queue
with no DLQ marker, and a steward that advances past a subscribed CODEOWNERS hit is
indistinguishable from one that never saw it.

So intake **projects** delivery rows. This is idiomatic, not novel: `region_materialized` is already
one event projecting into `kb_cogmap_regions` *and* N `_region_members` rows. One event, many
projection rows. The ledger stays clean (invariant 2) and the read surface gains durable
per-subscriber state.

```
kb_subscription_deliveries
  subscription_id, event_id, status, disposition,
  decided_by_invocation_id, decided_at, rationale, confidence
```

**Lifecycle:**

```
                 ┌──────────────── intake (payload-only, coarse) ────────────────┐
                 │                                                               │
            pending_scope                                                        │
                 │                                                               │
                 ├── enrichment (authed fetch: changed files + CODEOWNERS @ sha) │
                 │                                                               │
       ┌─────────┼─────────┬──────────────┐                                      │
       ▼         ▼         ▼              ▼                                      │
   in_scope  out_of_scope  undetermined  (enrichment failed — VISIBLE, never      │
       │                    │             silently out_of_scope)  ── DLQ ─────────┘
       │                    │
       └── steward tick ────┴── (undetermined is surfaced too — invariant 6)
                 │
       ┌─────────┴─────────┐
       ▼                   ▼
     acted             declined
   (authored,        (judged immaterial,
    cites the         with reasoning +
    event)            confidence)
```

**The disposition is a judgment act**, and therefore an authored event under an invocation envelope
carrying reasoning, confidence, and rationale — exactly what the membrane requires of judgment. A
steward declining a PR is not a silent cursor bump; it is accountable and citable: *"the platform
team's steward saw PR #412, judged it immaterial at confidence 0.7, because it touched only test
fixtures."*

This makes the delivery table a **research corpus** as well as a queue: churn × judgment, which is
precisely the stability signal the SCIP goal wants and currently has no denominator for.

### Enrichment (deferred, authed, append-only)

The fine radius cannot be computed at intake — GitHub's `pull_request` webhook payload
[does not carry the changed-file list](https://docs.github.com/en/rest/pulls), so answering "did this
hit our CODEOWNERS paths" means fetching. And `kb_events` is append-only, so a refined radius cannot
overwrite the coarse one.

It therefore arrives as a **second event** that `derived_from` the first and `touches` the finer set
— which activates all three of the reserved `references` rels (`touches`, `derived_from`,
`supersedes`) that the column was designed for and never used.

Enrichment is temper's **own** narrow need — two endpoints (list PR files, read `CODEOWNERS` at the
merge sha), not a general remote-API surface.

### Consumption — pull, not push

**Nothing is enqueued at intake.** The steward's existing hourly sweep already asks "what changed
since my watermark." It gains one branch: *events whose `references` contain a `touches` at me.*

```sql
WHERE (e.producing_anchor_id IN (my team's contexts)          -- internal drift, today
    OR e."references" @> '[{"rel":"touches","target":{"kind":"kb_cogmaps","id":<me>}}]')
  AND e.id > my_watermark
```

Same cursor, same sweep, same `kb_workflow_jobs` — **which stays cogmap-shaped and untouched.** An
earlier reading of this design assumed a push fan-out and concluded `kb_workflow_jobs.cogmap_id
NOT NULL` had to be generalized. It does not. Pull-on-tick deletes that migration from the plan.

(The delivery table supersedes the watermark's role *for external events* — the watermark cannot
express "declined," the delivery row can. Whether external events should also move the watermark, or
whether the two cursors coexist, is an open question below.)

### Reach — brokering is an admission criterion, not a preference

An agent handed a Linear-ticket event **can do nothing with it** without a Linear read tool. Reach is
therefore part of the goal, not an adjacent concern.

**Two axes are easy to collapse into one, and they are independent.** Naming them separately, because
a reader who conflates them will reach for the wrong fallback under pressure:

| | |
|---|---|
| **Axis 1 — who holds and mints the remote credential** | *Buy*: managed infra (Vercel Connect) holds the GitHub App installation, handles rotation and multi-installation tenancy, mints scoped tokens. *Build*: temper stores the App private key and mints installation tokens itself. |
| **Axis 2 — where the agent's tool call goes** | *Proxy*: the agent calls `temper-mcp`, which grows a `linear_get_issue` tool, which calls Linear. *Broker*: temper declares reach, mints the token proving it, and the agent calls **Linear's own MCP server** directly. |

They are orthogonal. One could buy the credential from Connect **and still proxy** — `temper-mcp`
wrapping Linear with a Connect-minted token. That is the wrong call, and "use managed infra" does not
protect against it. "Broker" is a claim about **axis 2**.

#### The rule

> **Proxy is out of scope, by default and by design.** If a remote system cannot be reached through an
> API, an MCP server, or a CLI tool that we can make available, with credentials handed to us
> correctly — **we do not integrate with that system.**

This is a **provider admission criterion**, not an implementation lean. temper's job is knowing *that*
`@cool-team-name` may read repo Y read-only, and minting the token that proves it — never re-exposing
someone else's API surface. `temper-mcp` does not grow `github_*` / `linear_*` / `notion_*` tools.

**The rule stands as the target.** Revisiting it requires a **named system or business need** and an
explicit decision. What it forbids is an implementer *quietly* reaching for proxying as a fallback when
brokering gets hard. The door is closed by decision, not welded shut — but it is not a hatch anyone
opens on their own.

**What it buys:** temper's MCP surface stays **fixed**. Proxying makes it grow without bound — one tool
family per provider, forever, chasing their API changes. This is what forecloses temper becoming an
integration hub.

**What it costs:** the fallback is gone. An earlier draft said that if per-subscription dynamic reach
proved unworkable, "the design changes shape" — meaning we would proxy. **S1 does not get to *prefer*
dynamic brokering; it must *prove* it.** If it cannot be proven, the agent-reach half of this goal is
**blocked and must be solved**, not routed around.

#### The rule *causes* a privilege asymmetry, and that is the thing to review

This is entailed by brokering, not incidental to it, and it is the sharpest open risk in the design:

**If temper does not mediate remote calls, temper cannot enforce remote scope.** Enforcement must live
in the **token** — so the fidelity of an agent's remote reach is bounded by the **provider's own
scoping granularity**, and temper's authz model is far finer than most providers'.

```
temper says:   this steward may read team A's cogmaps and contexts.     (precise)
the token says: this app may read the org.                              (coarse)
```

**The danger runs inward, not outward.** It is not a temper data leak. It is that an agent can **fetch
remote content its team never subscribed to and author it into that team's cogmap**, where it arrives
wearing legitimate provenance and looks clean on the way in.

Fidelity is **per-provider**: GitHub can scope to a repo set (an App installation, or a fine-grained
PAT); Linear is likely workspace-level. So "how closely can remote reach track temper reach" is a
capability that differs by provider — which means, for the fifth time in this design, **it must be
declared, never assumed.** Hence the dual of invariant 6:

> **Silence must never encode absence of capability — and over-broad reach must never be silent
> either.** If an agent's remote reach is **coarser** than its temper reach, the connection **says so,
> out loud.**

That does not fix the asymmetry. It makes it a **declared, reviewable property of a connection**
instead of a latent surprise — which is the most an honest brokering design can offer.

**Two things materially shrink the blast radius:**

- **Human-driven sessions are naturally 1:1.** A human working through Claude Desktop (or equivalent)
  carries **their own** GitHub/Linear credentials, so their remote reach already matches what they may
  see. **The asymmetry bites only unattended machine agents** — a much smaller and more reviewable
  surface than "all agent tooling."
- **Team-minted M2M credentials make the coarse connector a floor, not a ceiling.** Where a provider
  supports team-scoped credentials, a team stands up its own connection and the gap narrows or closes.

**A single app-level MCP connector is therefore acceptable** — it is the same shape as
"infra is provisioned by admin," which this design already accepts for webhooks. What is *not*
acceptable is it being **undeclared**.

#### Structural unbrokerability rejects a provider; temporary absence of reach does not

The two capability tiers survive this rule and are sharpened by it:

- A connection may legitimately be **ledger-only** — credential not yet provisioned, or reach not yet
  granted to a given team. That is a **visible, legal state**. Events still land; judgment is simply
  not yet possible, and says so (invariant 6).
- A provider that is **structurally** unreachable — no API, no MCP server, no CLI, or a credential
  model we cannot hold — is **rejected**. We do not integrate with it.

*Temporary absence of reach is a state. Structural unbrokerability is a rejection.* The connection's
**tool manifest** is therefore not decorative metadata: it is the **evidence that the provider is
admissible at all.**

#### The open risk this rule concentrates

Brokering assumes an agent can acquire an MCP connection **per subscription, at runtime**. But Eve
declares connections **statically, in code** — the steward's `agent/connections/temper.ts` is a source
file with a fixed 24-tool allowlist. If a new subscription to a new Linear workspace demands an agent
**redeploy** to reach it, brokering is operationally worse than the thing we have forbidden.

[Vercel Connect](https://vercel.com/docs/connect)'s connector types include **"MCP servers — any MCP
server (`mcp.<host>/<path>`)"**, so Connect is plausibly the brokering *mechanism* and not merely the
credential store — which is where axis 1 and axis 2 converge in practice. **This is unverified.** It is
a **hard gate on S1**, not an assumption, and — because the proxy fallback is closed — it is the single
highest-risk unknown in the goal.

#### Axis 1, separately

Connect is a strong candidate on the credential axis too: it verifies inbound GitHub/Linear/Slack
webhooks and forwards them as signed OIDC requests (no per-webhook HMAC secrets stored), *and* mints
app-scoped tokens with rotation and tenancy handled server-side. The counter-argument is Vercel
coupling (self-hosted instances) and that we already own HMAC discipline in `internal_sig.rs`. **This
one *is* a spike decision** — and the verification seam should stay abstract enough to swap either
way. Note the asymmetry: **axis 2 is settled by rule; axis 1 is open.**

---

## Authorization

Both tiers map onto structures that already exist. **No new authz vocabulary** — and, as of PR #418,
no new authz *predicate* either.

A connection is a machine principal wearing an integration's clothes, so it inherits the
machine-registration gate verbatim:

| Action | Gate |
|---|---|
| Provision a connection (credential, webhook, tool manifest) | `is_system_admin` **OR** owner of the team that will own the connection |
| Grant a connection's **reach** to teams/cogmaps | explicit, plural, **never inferred from the owner** |
| Author a subscription against a reachable connection | `team_role IN ('owner','maintainer')` on the subscribing team |
| Read a delivery / an external event | the ordinary read gates (`resources_visible_to`, `anchor_readable_by_profile`) |

**This is the machine-principal rule, unchanged.** The shipped model (`docs/guides/machine-credentials.md`)
already establishes every piece:

- **Registration is authorized by team ownership**, not admin-only. A team owner may stand up a
  machine owned by their own team, with no operator in the loop — "the point of the model: a team
  runs its own agents." The same should hold for a connection: a team that owns its own Linear
  workspace provisions its own connection. A connection to the **company's** GitHub org is owned by
  the gating team, and is therefore admin in practice — which is where the "provisioning is admin"
  intuition is correct, and *why*.
- **Owner ≠ reach.** `--owner-team` "records who owns the machine — **not what the machine can
  reach**." Reach is plural and explicitly granted. A connection follows suit: owning it does not
  confer the right to subscribe to it, and subscribing to it does not confer ownership.
- **Teamless fails closed.** A teamless machine is admin-only to create, read, or operate — "the
  empty owning team fails closed, never open." A teamless connection inherits that.

**Admin actions are events, firewalled from cognition** — consistent with the existing
admin-event-sourcing shape.

**The credential half is already built.** PR #418 shipped `ClientCredentials` in `temper-ts`
alongside `Temper::Credentials` in the Ruby gem, both pinned to one cross-language wire contract
(`tests/contracts/m2m-token-request.json`), with two mint paths — `provision` (an external IdP holds
the secret) and `issue` (Temper *is* the Authorization Server and mints a `tmpr_…` id + a
write-once secret, stored only as a SHA-256 hash). A connection's temper-side credential needs no new
machinery; it needs a **second kind of credential** — the *remote* system's token — which is the
build-vs-buy question S1 settles.

**One real migration, and it deserves review as such:** `kb_access_grants.subject_table` is
`CHECK (… IN ('kb_resources','kb_contexts','kb_cogmaps'))`. Granting reach on a *connection* means
widening that CHECK. Relaxing a CHECK is safe under the additive-only-on-`main` invariant (no existing
row is invalidated), but this is the most load-bearing authz table in the system and the change
should not be waved through.

---

## What this is not

- **Not a sync engine.** A Linear ticket does not become a temper task. A PR does not become a
  document. Remote resources are **cited, not copied** (`kb_remote_sources` gives them a UUID
  handle).
- **Not an integration hub — and this is enforced by rule, not by restraint.** temper brokers scoped
  tokens; it does not re-expose remote APIs. A provider that cannot be brokered is **not integrated**.
  `temper-mcp` never grows `github_*` / `linear_*` / `notion_*` tools.
- **Not a notification firehose.** Radius is declared. A payload nobody subscribed to routes nowhere.
- **Not eager indexing.** Intake never fetches. Enrichment fetches narrowly and only when a selector
  requires it.
- **Not a push system.** Consumption is the existing pull-on-tick steward sweep.

---

## Open questions the spikes must settle

1. **Does the watermark survive for external events?** The delivery table can express "declined"; the
   watermark cannot. Do they coexist, or does the delivery table become the sole cursor for
   subscribed events? Getting this wrong reintroduces the exact ambiguity the delivery row exists to
   remove.
2. **Can an agent acquire an MCP connection per subscription, at runtime?** *(highest risk in the
   goal)* Eve declares connections statically in code. If a new subscription demands an agent
   redeploy, brokering is operationally worse than proxying — **and proxying is forbidden by rule, so
   there is no fallback.** This must be *proven* in S1, not assumed. Connect's "any MCP server"
   connector type is the likely mechanism and is **unverified**.
3. **How closely can remote reach track temper reach, per provider?** *(the review this design most
   needs)* Brokering keeps temper out of the call path, so scope must be enforced in the token — and
   the provider's scoping granularity, not temper's authz, sets the ceiling. An agent could fetch
   remote content its team never subscribed to and author it into that team's cogmap with clean
   provenance. Mitigations already identified: human-driven sessions are naturally 1:1 (the asymmetry
   bites **only unattended machine agents**); team-minted M2M credentials make a coarse connector a
   floor rather than a ceiling. **A coarse app-level connector is acceptable; an undeclared one is
   not.** What is the minimum declaration that makes this reviewable rather than latent?
4. **Vercel Connect vs. own the credential + HMAC** *(axis 1 only)*. Strong lean to Connect (it does
   both halves), but the self-hosting coupling could overturn it. Note the asymmetry: the *brokering*
   axis is settled by rule; only the *credential* axis is open.
5. **What does a remote binding look like, if we want one?** An edge cannot point at a
   `kb_remote_sources` row (`kb_edges` endpoints are resources/cogmaps only). Binding a temper task to
   a Linear ticket would need a stub resource, a property/facet, or a widened edge. **Not decided.**
6. **Selector language.** Per-provider and per-grain, and it must declare its own capability. What is
   the minimum expressive selector that covers "CODEOWNERS paths in repo Y" and "Linear project P"
   without becoming a query language?
7. **What distillation actually means in practice.** Named by the goal's author as undefined. What a
   steward *does* with a subscribed event under its telos — and specifically what edges, if any, get
   authored into a cogmap, and by whom.

---

## Decomposition — the goal and its spikes

**S1 — The connection & capability model.** *(first; everything hangs off it)*
What a connection is: credential + webhook + tool manifest, the ledger-capable / reach-capable tiers,
`needs_credential` birth, provisioning under the machine-registration gate, explicit plural reach
(and the `kb_access_grants` CHECK widening). Settles the Vercel-Connect build-vs-buy.

**The sharpest question in S1 is that a connection needs *two* credentials, and only one exists.**
temper's own machine credential (a machine authenticating *to* temper) shipped in PR #418 and is
done. What a connection also needs is the **remote** credential — the token temper presents *to*
GitHub/Linear — plus a way to hand a *scoped, read-only* version of it to an agent. Those are
different objects with different lifecycles, and conflating them is the most likely way to get this
wrong.

**S1 carries a hard gate, and it can fail.** Because proxying is forbidden by rule, S1 must **prove**
that an agent can acquire an MCP connection **per subscription, at runtime** — not merely prefer it.
Eve declares connections statically in code. If a new subscription demands an agent redeploy to reach
its remote system, **there is no fallback**, and the agent-reach half of this goal is blocked until
that is solved. This is the goal's highest-risk unknown and S1 exists partly to retire it early,
while it is still cheap to be wrong.

**Dogfoods the M2M work** — prod has exactly one machine client today (the steward). This is what
makes the second one exist.

**S2 — Subscriptions, radius matching, and the delivery lifecycle.**
The subscription table, the selector language, coarse radius at intake, the `references` `touches`
write path (the first-ever writer of that column), and the
`pending_scope → in_scope/out_of_scope/undetermined → acted/declined` state machine with its DLQ.
The conceptual core. Prototypable against a stubbed connection, so it can run alongside S1.

**S3 — Intake: land a real webhook.**
The thinnest end-to-end slice — a real GitHub PR webhook, verified, attributed to a connection
entity, written as one `kb_events` row with a foreign `event_type` and a raw payload. Proves the
ledger accepts a foreign event and that emitter/anchor/references hold up under a real payload
(including the 25 MB cap, above which
[GitHub silently drops the delivery](https://docs.github.com/en/webhooks/webhook-events-and-payloads)).

**S4 — Deferred enrichment and the fine radius.**
The authed fetch (changed files + `CODEOWNERS` at the merge sha), CODEOWNERS evaluation, the
append-only refinement event, and the `undetermined`/DLQ path. Where invariant 6 is actually tested.

**S5 — The consumption seam.**
Extending `steward_ingest_delta` with the `references`-GIN branch; the watermark-vs-delivery-cursor
question; surfacing subscribed events on the steward tick.

**S6 — Distillation under telos.**
What agents *do*. The EPD-wide-coarse vs team-specific-nuance differentiation. What gets authored,
what gets cited, what gets declined. Most conceptually open; hardest to spike before the machinery
beneath it exists.

**Two providers, not one, in the first real build.** GitHub and Linear differ *precisely* on the axis
that matters — GitHub's PR payload cannot answer "which files," Linear's issue webhook carries the
issue inline. One provider would let us build an abstraction that fits it perfectly and is wrong.
Two is the minimum that validates the capability-declaration invariant.

---

## Relationship to other goals

**SCIP (`019f56e1`) is an *instance* of this, not a sibling.** A SCIP indexer is a third-party system
emitting mechanical facts about a repo. The *event* is the trunk-merge webhook — **one** event. The
unfolding into `kb_code_*` is that event's **outcome**, not a stream of events (invariant 2). SCIP is
an unusually rich instance — it earns a whole projection family — but the intake, attribution, and
membrane discipline are identical. **SCIP should land on this substrate rather than beside it.**

**The ledger as a readable surface (`019f51e3`) gets an answer.** Its blocking decision —
*"`references` is either populated and consumed, or gone"* — is resolved here in the affirmative. A
webhook's blast radius is the lineage that must live on the event and cannot be an edge.

**The M2M credential work (PR #418, landed 2026-07-13) stops being adjacent and becomes
load-bearing.** A *provisioned webhook* is really a **provisioned connection**: credential + webhook
registration + machine principal + entity + the subscriptions hanging off it. This goal is the first
real consumer of that machinery — and it consumes it in three distinct ways:

- **The authz predicate, verbatim.** `is_system_admin OR owner of the owning team`, owner-≠-reach,
  teamless-fails-closed. No new gate is invented here.
- **The principal.** A connection's emitter entity is backed by a machine profile — the thing
  `provision`/`issue` already create, along with emitter entities and gating-team enrollment.
- **The gap it exposes.** M2M solves *machines authenticating to temper*. A connection also needs
  *temper authenticating to a remote system*, and *temper brokering a scoped read-only token to an
  agent*. That second axis does not exist yet and is the core of S1.
