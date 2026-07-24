# Set 5 — the adversary persona as citation auditor

*Design spec. Operationalizes Set 5 of the evidential-standing work-breakdown
([spec `019f81e8`](2026-07-20-evidential-standing-substrate-breakdown-and-lead-seams-design.md)),
whose §"What this spec deliberately does NOT design" deferred to Set 5: "the adversary
persona's jobs-to-be-done, its spatial reliability profile, and the concrete
adversarial-challenge / survived event vocabulary the projection reads."*

*Task `019f81ea-f627-78f2-a6cd-151eea16b238`. Goal `019f81e9-25f3-7fe1-b563-4acca2e391eb`.*

*Grounded against `migrations/`, `crates/`, and `packages/agent-workflows/steward/` on
2026-07-23, after Set 3 merged (PR #519). Every claim about existing substrate carries a
quoted `file:line`; every decision is tagged CONFORM / EXTEND / AMEND against that
grounding.*

---

## Bedrock — inherited, unchanged

> **Standing is not truth, and the system cannot close the gap between them — only make
> its shape visible.**

Everything below is answerable to spec `019f81e8`'s preamble. The auditor designed here
assesses **how defensible a citation is for the connection it makes** — a fact about the
structure of emitted evidence. It never assesses whether a claim is true, and it never
assesses what a source *says*. Where a decision would let an audit quietly stand in for a
truth claim, that decision is wrong. §7 records the one place this line was deliberately
drawn and what was given up to hold it.

---

## 1. What Set 5 inherited, and what is actually on disk

Set 3 shipped the standing projection and left four named debts. All four were re-verified
on 2026-07-23:

| Inherited debt | Ground truth |
|---|---|
| The challenge/survived label vocabulary | `resource_adversarial_survival` counts `express` edges labelled `'challenged'` and sums weight on `'survived-challenge'` (`migrations/20260721000010_evidential_standing_memo.sql:169-177`). Both labels appear **nowhere else in the repo**. It returns `(0, 0)` for every finding. |
| The independence-edge writer | `refresh_independence_pairs` builds `kb_independence_pairs` from `express` / `label='independent-of'` edges between two `kb_resources` (`…memo.sql:116-134`). `'independent-of'` has **no production writer**, so the table is empty in any real database. It does have exactly **one test writer** — `crates/temper-substrate/tests/evidential_standing.rs:237` — which is what `affirmed_independence_raises_breadth` (`:227`) exercises, and which §3.4's retirement therefore breaks. Retiring the pairwise model means deleting those tests, not just the functions. |
| The scar writer | `kb_block_provenance.is_corrected` exists with `DEFAULT false` and is read-filtered everywhere (`migrations/20260624000001_canonical_schema.sql:610`), with **no writer anywhere**. |
| Edge-incident refresh | `TODO(Set 5)` at `crates/temper-services/src/backend/db_backend.rs:1165-1172`: the pairs memo is rebuilt only on resource create/update, never on edge writes, so `indep_breadth` would read stale after an independence-edge assert/fold. |

Two further facts constrain everything below.

**Spec §2.2's meta-edge is not persistable — confirmed still true.** `kb_edges` restricts
endpoints to `('kb_resources','kb_cogmaps')` (`canonical_schema.sql:630,632`), and the
command layer types both ends as `ResourceId`
(`crates/temper-workflow/src/operations/commands.rs:207-219`). Set 3 recorded this
falsification and deferred the representational choice to this spec.

**`resource_independence_breadth` is a constant.** With no writer for `'independent-of'`,
it returns `1.0` for every finding that has any base and `0.0` otherwise
(`…memo.sql:141-146`). There is **no data to migrate and no behavior to preserve** in the
pairwise independence model — a fact that makes §3's AMEND cheap.

---

## 2. The reframe — independence is a property of the citation set, not of observers

Spec `019f81e8` §2 relocated corroboration from actor-count to "evidential breadth weighted
by assessed independence," represented as **pairwise `independent-of` claims between
evidentiary bases** (§2.2, §2.3), left conservative-by-default (§2.4).

That model has a defect this spec corrects: **it requires an asserter, and there is no
honest candidate.** Nothing moves until somebody makes a positive independence claim, and
both available pens are compromised — the steward is the monoculture being assessed, and an
adversary that affirms its own independence claims and then attacks them is grading its own
homework. The spec named this recursion honestly (§"Independence relocated, not dissolved")
but resolved it into a representation that still needs a judge.

**The correction: independence is not about *who* observed, but about *what was cited*.**
The system's agents are one M2M-minted agent set with the same guidance, the same
information access, and the same model-and-fallback configuration. Their multiplicity
confers nothing — as re-walking a familiar room day after day confers no fresh observation,
because habituation, not attention, is what returns. So the count that matters is **distinct
evidentiary sources cited**, and the discount that matters is applied to **how well each
citation carries the connection it is cited for**.

Under this reframe the asserter problem dissolves rather than being answered: **the citation
IS the assertion**. The steward asserts by citing. The auditor never affirms an independence
claim into existence; it assesses citations that already exist. Separation of powers falls
out of the data model instead of being imposed on the personas.

- **AMEND** — spec §2.2 (pairwise `independent-of` edges) and §2.3 (`independence_pairs`
  memo). Authorized by the spec's own §2 framing ("corroboration is a property of the
  evidence, not the assertion") — this spec holds that thesis and changes the representation
  that failed to carry it.
- **CONFORM** — §2.4's silence default survives verbatim, relocated to citation grain:
  an unaudited citation is *not* presumed defensible (§3.2).

---

## 3. The projection — magnitude and quality, never one number

### 3.1 Three components, and the unaudited posture lives in the band — **AMEND (of `indep_breadth`), CONFORM (to §1.1)**

`kb_resource_standing.indep_breadth` (`…memo.sql:32`) becomes three columns, and the
skepticism toward unevaluated evidence is expressed by **where the band gates**, not by a
number baked into a mean.

- **`citation_magnitude`** — the count of distinct live cited sources for the finding.
  **Monotone**: citing more evidence never lowers it. This is the *findability* axis — a
  finding with many sources is well-connected in the graph regardless of whether anyone has
  audited it.
- **`audit_coverage`** — the count of those distinct sources that carry **at least one**
  audit event. Integral, and **monotone under append-only** (§4.1): once a source is
  audited it stays covered. The *evaluated-ness* axis.
- **`citation_quality`** — the mean, over the **audited subset only**, of each audited
  source's decay-weighted audit value (§4.1), in `[-1.0, 1.0]`. Undefined (read as a
  neutral `0.0`) when `audit_coverage = 0` — an unaudited finding makes no quality claim,
  and its low standing comes from the band gate below, not from a poisoned mean.
  **The aggregation is per distinct source, in two stages, and the order matters:** a source
  cited by several of the finding's blocks may carry several audit events, so first collapse
  *within* a source (the decay-weighted aggregate over all that source's audits, across all
  its citing blocks — one value per source), then take the mean *across* distinct sources. A
  naive `LEFT JOIN` of audits onto provenance yields one row per `(block, source)` audit and
  gives a five-block source five votes in the outer mean — the echo/actor-count fallacy
  re-entering through block multiplicity, in the very function written to exclude it (§3.1's
  `r_parent`-vs-magnitude distinction).

**Liveness is not optional in these counts.** Soft-delete sets `is_active = false` and does
not fold blocks or provenance (`canonical_functions.sql:1056` is the whole projector), so a
component that counts provenance without a liveness join keeps a deleted source conferring
standing forever. Every producer must therefore join `kb_resources` and require the source
`is_active` (the CLAUDE.md rule: *"`ingest_state = 'complete'` goes exactly where
`r.is_active` already goes"*), and `citation_magnitude` is *"distinct **live** cited
sources"* — the word "live" is load-bearing and Set 3's `resource_bases` did not yet carry
it. The sweep (§6.3) likewise excludes findings that are not `is_active AND
ingest_state = 'complete'`, or it will feed the auditor a deleted or half-uploaded resource
at the head of the queue every tick.

The unaudited posture — *"present and reachable, but visibly not-yet-earned"* — is the
**coverage ratio** `audit_coverage / citation_magnitude` gating the band. An unaudited
finding has ratio 0, so it is pinned to the floor band no matter how many sources it cites;
citing more sources *lowers* the ratio and re-enters it into the auditor's queue (§6.3),
without ever destroying a verdict already earned. This is why the perverse gradient the
earlier draft carried is gone: quality is computed over audited sources only, so adding
unaudited evidence cannot pull a positive verdict down — it moves the coverage axis, not the
quality axis.

The full component mapping against the shipped memo (`…memo.sql:30-40`), so no column's
fate is left to inference:

| Shipped column | Fate |
|---|---|
| `indep_breadth` | **Replaced** by `citation_magnitude` + `citation_quality` + `audit_coverage`. |
| `adversarial_survival` | **Subsumed** into `citation_quality`. Survival stops being a separate scalar because the gradient *is* the survival signal — a citation that withstood audit carries a positive value, one that did not carries a negative one (§3.3). |
| `challenge_count` | **Replaced** by `audit_coverage`, which is the clearer name for the same job — distinguishing "nobody tried" (coverage 0) from "N sources evaluated" (spec §1). Drop the old column rather than leave a name that lies about its meaning. |
| `contradiction_balance`, `freshness` | **Unchanged.** |
| `r_parent` | **Unchanged, and deliberately not the same thing as `citation_magnitude`.** `resource_r_parent` counts *all* uncorrected provenance rows over live blocks (`…memo.sql:51-57`) — total accretion, duplicates included — while magnitude counts *distinct sources*. Ten citations of one source is `r_parent = 10, citation_magnitude = 1`, and that difference is load-bearing: it is the echo case. An implementer who collapses them reintroduces the actor-count fallacy. |

**The band** (`standing_band`, `…memo.sql:186-200`) is re-thresholded over the new set. It
gains a **fourth arm** so the lossy chip carries the *sign* of the auditor's work, and so
"never evaluated" and "evaluated and found wanting" are not flattened together (spec §1's
requirement, on its negative side):

- `near-canonical` — `magnitude >= 2` **and** coverage ratio `= 1.0` **and** net-positive
  quality **and** not under live contradiction.
- `reinforced` — `magnitude >= 1` **and** coverage ratio `>= 0.5` **and** `quality > 0.0`
  **and** not under live contradiction.
- `disputed` — `audit_coverage > 0` **and** `citation_quality < 0`: the adversary examined
  it and it did not hold. Distinct from the floor.
- `provisional` — the floor, including every unaudited finding (coverage 0).

**Calibrated for the system's real dynamics — reachable in one thorough pass, not many
rounds.** Two structural choices make the ladder achievable for a resource that is reviewed
*well once* rather than *often*, which is the common case (most concepts/facts see only a
handful of steward/auditor passes in their lifetime, and even the future coherence/salience
reap pass does not change that):

- **The magnitude floor for the top band is `2`, not `3`.** Two is the honest Landmesser line
  — *more than one independent source*; a lone source can never be near-canonical no matter
  how well-audited (that is the whole point), but demanding three would make near-canonical
  structurally unreachable for the large fraction of an atomic KB that rests on two vetted
  sources, even where it is deserved.
- **Full coverage is reached in a single pass, and a lone positive audit holds without
  eroding.** Coverage is per-distinct-source, so one thorough auditor visit audits every
  cited source and reaches ratio `1.0` at once — the top band does **not** require repeated
  rounds. And decay only arbitrates *between competing* audits; a single audit's weight
  cancels in the mean, so an old lone `+0.8` still reads `+0.8` on the quality axis
  indefinitely. Staleness is carried by the separate `freshness` component, which the band
  deliberately does **not** gate on, precisely so infrequent review does not demote earned
  standing.

**The absolute quality threshold is co-calibrated with the auditor prompt, not fixed here.**
`quality > 0.5` versus `> 0.3` is meaningful only *relative to how the auditor uses the
`[-1.0, 1.0]` range* (whether it reserves the top of the range for "fully carries the claim"
or spreads its scores differently), and that distribution does not exist until the auditor
prompt does. So the numeric quality cut is a **provisional low default** finalized against the
auditor's real scoring distribution during the persona work (§5), not guessed against none.
Because the band is a read-time function over stored components (§1.3), re-tuning it later is
a one-migration change with no backfill — the numbers are the cheapest thing in this design
to move, and the structural choices above (floor of 2, full-coverage-in-one-pass) are the
load-bearing part.

- CONFORM — spec §1.1 (shape-primary, band as lossy chip) and §1.3 (memoized components,
  band computed at read). The band stays a read-time function over components.

### 3.2 Unaudited is a band state, not a number in a mean — **EXTEND**

The design's posture — *evidence no second party has weighed is not yet defensible* — is
real and must be legible, but it is **not** a value folded into the quality mean. An earlier
draft gave unaudited citations a `-0.5` contribution to the mean; that produced a perverse
gradient (adding good-faith evidence demoted a finding two bands) and contradicted the
monotonicity the same design claimed. The faithful expression is the split above:
unaudited-ness pins the **band** via the coverage ratio, while **quality** speaks only for
what was actually assessed.

This keeps every property the negative prior was reaching for — an unaudited finding sits at
the floor, findable but not elevated; the auditor's absence is distinguishable from its
approval — with none of the gradient pathology, because the two concerns now live on two
axes instead of being blended into one.

- EXTEND — authorized by spec §2.4 (silence is not evidence of independence) and §1
  (0-challenges must be distinguishable from N-withstood), relocated to citation grain and
  to the band rather than the mean.

### 3.3 An audit is signed, not a discount — **EXTEND**

An audit value spans `[-1.0, 1.0]`: the auditor may **reinforce** a citation as well as
discredit it. This is not self-grading — the auditor did not author the citation; the
steward did. Assessing another party's act in either direction is precisely the adversarial
relation. What remains prohibited is an agent's assessment moving *its own* standing
(§4.2).

The auditor weighs: the citing act's recorded confidence and rationale, the related
resources and evidentiary claims, and the magnitude of the citation set it sits in.

### 3.4 What this absorbs, and what it retires

**Absorbed.** "This source does not say what you claim" — the `is_corrected` provenance
scar — is expressible as a strongly negative audit value without ever asserting what the
source says. The audit claims only that the citation confers little defensibility *for the
connection it makes*. Same information, and it stays inside the standing≠truth boundary.
`is_corrected` is therefore **not** written by Set 5 (§9).

**Retired.** `kb_independence_pairs`, `refresh_independence_pairs`, and
`resource_independence_breadth` (`…memo.sql:86-146`) are superseded. The edge labels
`'independent-of'`, `'challenged'`, and `'survived-challenge'` are never written; the
edge-based `resource_adversarial_survival` reader (`…memo.sql:169-177`) is replaced by the
citation-grain components. Because the pairs table is provably empty (§1), this AMEND
carries no data migration and no behavior change.

**The `TODO(Set 5)` at `db_backend.rs:1170` dissolves rather than being fixed** — for the
component it was about. It existed because breadth read a memo built from independence
*edges*, so edge writes had to re-drive it; once breadth reads citations and audits, no
independence-edge write can make magnitude, coverage, or quality stale, and that specific
refresh is never needed.

A narrower edge-staleness *does* remain, and the replacement note must say so rather than
claim total victory: `contradiction_balance` is **unchanged** (§3.1) and still reads
`kb_edges` (`…memo.sql:159-163`), while the standing clock still fires only on
resource writes. So the `kb_resource_standing.contradiction_balance` **column** is stale
after an edge write until the next resource write over the finding. This is harmless **only**
because `resource_standing_shape` recomputes every component live at read and never trusts
the memo column (§4.3) — so the read is correct and the memo is a write-cost optimization,
exactly as it is for `freshness`. The Task-5 replacement for the TODO must state this
precisely: *any future consumer that reads `kb_resource_standing` directly, rather than
calling `resource_standing_shape`, must first wire an edge-incident refresh for
`contradiction_balance`.* A deleted warning that a memo-column reader needed is worse than a
stale warning.

---

## 4. What a citation audit is

### 4.1 The grain forces the representation, and the trail is append-only — **CONFORM**

A citation is a `(block, source)` pair: `kb_block_provenance` is keyed
`(block_id, source_kind, source_id, contributed_by_event_id)`
(`canonical_schema.sql:603-613`). `kb_edges` cannot address a block — its CHECK admits only
`kb_resources` / `kb_cogmaps` as endpoints (`:630,632`). **An audit at citation grain
therefore cannot be an edge.** Set 3's edge-based challenge readers are not badly built;
they are simply unable to see the grain the assessment happens at.

Three candidate homes were considered:

| Option | Verdict |
|---|---|
| A float column on `kb_block_provenance` | Rejected. Provenance rows are event projections; a citation is audited repeatedly over its life, and a single column keeps only the latest verdict and discards the history. It also puts an assessment inside the record of what happened — the same category error as storing a band (§1.3 of `019f81e8`). |
| Reuse `is_corrected` | Rejected. Binary, and it is exactly the truth-claim baggage §3.4 exists to avoid. |
| **A new append-only `kb_citation_audits` event projection** | **Chosen.** |

**The audit trail is append-only, and no audit is ever mutated or superseded.** An audit is
an event; the ledger is immutable by design; an auditor does not retract a verdict, it emits
a new one. So `kb_citation_audits` carries **one immutable row per `citation_audited`
event** — the citation key, the signed value, and the emitting event id — with **no
`is_superseded` bit, no "latest live audit," and no supersede-on-write.** A later `+1.0`
never erases an earlier adversary's `-1.0`; both are permanent events in the trail. This is
strictly the events-as-primary tenet: the row set is a derived, replayable projection of the
`citation_audited` events, and replay re-appends idempotently on the event id
(`ON CONFLICT (audited_by_event_id) DO NOTHING`) — there is no mutable state for replay to
disagree with.

**The visible standing is a decay-weighted projection recomputed fresh from that trail.** A
citation's audit value is a **recency-weighted aggregate over all its audit events** — newer
verdicts weigh more, older ones fade in influence but never vanish. It may be materialized as
a stored column for read-cost, but that materialization is **always recomputed fresh along
the audit trail, never incrementally from its own prior stored value** — a projection of
history, not a cache of a cache. Because the aggregate is a pure function of
`(trail, as-of-time)`, a citation's standing *as of* any past moment is derivable by
recomputing over the trail truncated at that time: a rolling view of the past, not a
denormalized latest.

This is the exact pattern the codebase already uses for `freshness` (`…memo.sql:63-75`:
memoized as a snapshot by the write-path clock, but **recomputed live at read** because it is
time-decayed and must reflect the current moment) and the disposable-read-model tenet — the
memo is derivable and throwaway; the events are the truth. See
[[reference_standing_memo_disposable_readmodel_not_event_sourced]] and
[[reference_temper_memo_refresh_is_rust_clock_not_trigger]]. The audit decay is that pattern
pointed at the audit trail instead of the provenance trail.

*Consequence, accepted deliberately:* the live view favours recent assessment — a single
recent verdict can outweigh an older opposite one — while both remain immutable in the
ledger and both resurface in an as-of-then view. That recency asymmetry is the intended
productive-disagreement behaviour (it mirrors §1's contradiction-as-vector-sum, not
erasure), not a hole to guard against. The exact decay half-life and aggregation shape are
tunable defaults this set owns.

- CONFORM — events-as-primary, projections derived; the `block_annotate` precedent
  (`migrations/20260710000001_block_provenance_annotate.sql:44`) for a payload-only,
  chunk-independent write path; the freshness precedent for a time-decayed component
  recomputed live at read.

### 4.2 The float rides the payload; self-assessment rides metadata — **CONFORM**

`AgentAuthorship { reasoning, confidence, rationale, persona, model }` rides
`kb_events.metadata`, deliberately **not** the payload, "so it is invisible to projections
(and thus affinity math) by construction"
(`crates/temper-core/src/types/authorship.rs:9-11,56-66`). The two columns are adjacent and
distinct: `payload` at `canonical_schema.sql:475`, whose comment at `:473-474` states that
"the projection halves (`_project_*`) read ONLY this", and `metadata` at `:480`.
`confidence` is non-`Option` — required whenever authorship is supplied at all.

This existing invariant draws the line this design needs, so we adopt it rather than
inventing one:

- The **auditor** reads a citing act's confidence and rationale as *input to its judgment*.
  That is an agent querying the ledger, entirely legitimate.
- The **projection** never reads that self-assessment. An agent's own confidence band must
  not move its own standing; only another party's assessment of it can.

Therefore the audit's signed value is **payload**, and the auditor's own confidence in its
verdict is **metadata**, where the projection cannot see it. The self-grading prohibition
becomes structural rather than procedural.

### 4.3 Refresh — **CONFORM**

The audit write path calls `refresh_resource_standing(finding)` following
`tick_resource_standing` (`db_backend.rs:1173-1185`) exactly, including its established
failure policy: **never fail the write**; log and swallow, and let the memo self-heal on the
next write. That policy is already reasoned on disk and is inherited, not re-decided.

Registering `citation_audited` with `category = 'domain'` puts audits in the element trail
by default — an audit is inspectable and challengeable like any other act. The allowlist
direction is load-bearing: `element_trail_node`/`_edge` filter `et.category = 'domain'`
precisely so a future category is excluded by default (the two functions are declared at
`migrations/20260719000010_admin_cognition_firewall_declarative.sql:105,146`; the
`et.category = 'domain'` filters themselves are at `:139` and `:165`).

**Registering the event type is not a copy-paste.** `kb_event_types.category` is `NOT NULL`
with **no default** — `20260718000020_trail_admin_event_firewall.sql:47` added it with a
default and `20260719000010:98` dropped that default, precisely so a registration using the
pre-firewall idiom **aborts at apply time** rather than landing silently mis-categorized
(`20260719000010:74-81` says so in as many words). Every post-firewall registration spells
`category` explicitly; see `20260720000020_principal_standing_events.sql:26`.

---

## 5. The persona

### 5.1 The auditor is a citation auditor

Its unit of work is a `(finding, citation)` pair. Its question is *"does this source carry
the connection claimed here?"* Its emission is a signed defensibility verdict. This is the
concrete content of the spec's "jobs-to-be-done shaped like the steward's but
challenge-substanced."

### 5.2 Identity must live in the principal, not the persona string — **CONFORM**

The chain is `client_id → kb_machine_clients → profile → kb_entities.profile_id →
kb_events.emitter_entity_id` (`canonical_schema.sql:144-151,468`;
`crates/temper-services/src/services/profile_service.rs:230-241`, lookup-or-401, no JIT
create). **One credential means one emitter entity.** A steward and an auditor sharing a
machine client would emit acts the ledger cannot tell apart, and `AgentAuthorship.persona`
cannot rescue that: it is self-declared *and* deliberately invisible to projections (§4.2).
"Assessed by another party" would degrade to "asserted by the same party wearing a label."

**The auditor therefore requires its own registered machine client**
(`temper admin machine provision --client-id … --label …`), independent of where its code
lives.

### 5.3 Placement — a second schedule in the steward package

The auditor ships as a new schedule in `packages/agent-workflows/steward/`, alongside
`schedules/steward.ts` and `schedules/materialize.ts`, with its own credential env set.

Code colocation does not create epistemic dependence: the two agents have separate
instructions, separate success criteria, and never read one another. The one lever that
genuinely attacks shared trained priors — running the auditor on a **different model** — is
a config choice (`agent/lib/model-config.ts`) available under either layout, and is
recommended. A sibling Eve project was considered and rejected as cost without benefit: its
own package-lock, Vercel project, and CI wiring buy separation that is already real.

### 5.4 The invocation envelope needs nothing new — **CONFORM**

`_project_delegated_launch` resolves `telos_resource_id` from `kb_cogmaps`, not from the
payload (`migrations/20260624000002_canonical_functions.sql:1244-1253`). An auditor
invocation is scoped to the **map's** telos exactly as a steward one is. Persona
differentiation lives in the machine principal (ledger-visible, §5.2) and the agent's own
instructions (not ledger-visible). No envelope change is required.

---

## 6. The tick

### 6.1 The queue is already persona-agnostic — **CONFORM**

`kb_workflow_jobs` opens with *"Persona-agnostic durable job queue for agent dispatch"*,
carries a `persona text` column (`migrations/20260705000001_workflow_jobs.sql:1,22`), and
enforces single-flight on `(cogmap_id, persona, dispatch_type)` (`:43-45`). The auditor takes
a new persona value and inherits SKIP LOCKED claim, lease-expiry reaping, attempts gating,
and in-flight dedup with **no new queue DDL**.

**But the queue's grain is a cogmap and the auditor's unit of work is a finding**, and that
mismatch is not free. The single-flight index means N uncovered findings in one cogmap
enqueue **one** job, with the other N−1 silently discarded by `workflow_job_enqueue`'s
`ON CONFLICT DO NOTHING` (`:59-62`). The resolution: enqueue **one job per cogmap whose
payload carries the uncovered finding list**, and let the session iterate. That keeps the
steward's one-session-per-cogmap shape and avoids an unbounded `dispatch_type` cardinality,
which is what per-finding discrimination would cost.

The schedule mirrors `schedules/steward.ts`: deterministic server-side sweep → enqueue →
claim, a correlation id minted per tick and threaded across the app boundary, then one
isolated model session per claimed job.

### 6.2 Scope boundaries — cogmap-homed findings, resource-kind citations

Two boundaries, both named rather than designed around:

- **Cogmap-homed findings only.** `kb_workflow_jobs.cogmap_id` is
  `NOT NULL REFERENCES kb_cogmaps(id)` (`migrations/20260705000001_workflow_jobs.sql:21`),
  but Set 3 made the subject of standing **any** `kb_resource` (`…memo.sql:22`), and
  resources also home in contexts. The first cut audits cogmap-homed findings only; widening
  the queue is additive and separate.
- **Resource-kind citations only.** `provenance_source_kind` has three values —
  `'event'`, `'resource'`, `'remote'` (`canonical_schema.sql:105`;
  `20260704000006_remote_source_kind_enum.sql:8`) — and Set 3's `resource_bases` filters to
  `'resource'` (`…memo.sql:110`). The standing components inherit that filter, so an audit
  of a `remote` (external-URL) citation would move nothing. The write path must therefore
  **reject** a non-`'resource'` `source_kind` with an error naming the reason, rather than
  silently accepting a no-op the auditor cannot detect. Auditing external-source citations
  is a coherent later extension (external sources are arguably the *most* independent
  evidence there is), but it is out of scope here and must be a deliberate widening of the
  component filters, not an accident of an unrestricted write.

### 6.3 Selection — incomplete audit coverage

The sweep selects findings with **incomplete audit coverage** — `citation_magnitude > 0`
and `audit_coverage < citation_magnitude` (§3.1) — ordered by the size of the uncovered
remainder, so the most-cited and least-audited findings are worked first. This is the
auditor's analogue of the steward's ingest-drift sweep, and — like it — is
**principal-scoped**: the sweep takes the auditor's principal and gates through the same
readability predicate every read uses (the steward's `steward_drift_sweep(p_principal, …)`
routes through `steward_candidate_cogmaps`; ours must route through the equivalent). A sweep
with no principal is a cross-tenant enumeration oracle, which would defeat §7's entire
`NotFound` posture.

Coverage, not quality, is the correct predicate. A quality-based sweep would drop a
partially-audited finding out of the queue after a single citation is weighed.

**Re-audit under append-only.** New *citations* re-queue a finding on their own: magnitude
rises, coverage does not, so the coverage ratio drops and the finding re-enters the sweep.
Coverage is monotone (§3.1), so re-auditing an *already-covered* finding is **not**
coverage-triggered — a deliberate first-cut limitation.

The natural next signal, given the decay model of §4.1, is a finding whose newest audit has
decayed past a threshold — "stale-covered." This is deliberately left for a **future reaper
pass**: a periodic tick (on the steward or auditor persona) that fires on a different
predicate than a moved watermark — regions where internal coherence has disaggregated, or
salience has drifted from the telos, sweeping older resources and older facts against newer
ones so that unrevisited assumptions are structurally re-accounted-for. That pass is where
decay-driven re-audit belongs; scoping and building it will likely evolve the auditor persona
itself, and it is out of scope here. Recorded so the first-cut limitation points at real
planned work rather than a gap. See [[project_evidential_standing_goal_sets]] and the Set 4
steward-expansion task.

### 6.4 Duty-to-challenge-before-promote is structural, not procedural

The coverage-ratio band gate (§3.1) makes it so: no band above `provisional` is reachable
until the auditor has evaluated the finding's evidence, because an unaudited finding has
coverage ratio 0 and is pinned to the floor regardless of magnitude. Set 6 (promotion as
translation) inherits that gate for free, and Set 5 does not need to know Set 6 exists — the
duty to challenge before promote is discharged by the data model, not by an agent remembering
to do it. The self-audit denial arm (§7) is what stops the citer from discharging that duty
against themselves.

---

## 7. Authorization — `can_audit_resource` is an open grounding obligation

**This section is a required pre-plan grounding item, not a resolved design.**

Every authored write today gates on `can_modify_resource`. An audit must **not**: an auditor
that may only assess findings it owns is not an auditor. The gate wanted is *can read the
finding* — the full canonical visibility predicate — plus being a registered, unrevoked
machine principal with reach. That is a deliberate widening (a write authorized by
readability), and it must be designed explicitly rather than falling out of a copied
handler.

**But readability alone is not sufficient, and this is load-bearing.** Readability lets the
citer audit their own citations — and because the append-only decay model favours recency
(§4.1) and the sweep clears a finding once covered (§6.3), a citer who audits their own work
positively both inflates their standing *and* removes the finding from the adversary's
queue. That defeats the entire adversarial premise (the self-grading prohibition §4.2 barred
only the auditor's *confidence* from moving standing; it never barred citer-equals-auditor).
So the gate **must** carry a self-audit **denial** arm: an audit is refused when the auditing
principal is the author of the citation it targets (the emitter of the block's contributing
`block_mutated`/`block_annotated` event, or — the cheaper sufficient proxy — a principal who
`can_modify` the finding). "Assessed by another party" is enforced here or it is not enforced
anywhere.

The subject of authorization is the finding **derived from the target `block_id`**, not a
finding named independently by the caller. The write lands on a block; the gate must resolve
that block's owning resource and authorize *that*, so a caller cannot authorize over a
finding they can read while writing onto a block of one they cannot (the transposition the
sealed proof exists to stop — `authz/mod.rs:95-117`).

It must be expressed in the **`ScopedAuthority` policy layer**
(`crates/temper-services/src/authz/mod.rs:54-133`; design doc
[`2026-07-22-scoped-authority-policy-layer-design.md`](2026-07-22-scoped-authority-policy-layer-design.md)),
not as an ad-hoc predicate. Before the implementation plan is written, the following must be
resolved **against that trait and the existing impls** (`authz/read_gates.rs:30,79`;
`authz/machine.rs`; `authz/grant.rs`):

1. **Subject.** Almost certainly the finding (`ResourceId`), so the sealed `Authorized<A>`
   carries it and the act cannot name a different one — the transposition the proof exists
   to prevent (`authz/mod.rs:95-117`).
2. **Arms.** What authorities admit an audit, in short-circuit order, and — per the trait's
   own doctrine — **which incumbent SQL predicate each arm calls**. *"SQL predicates are
   authoritative here — call them, do not restate them"* (`authz/mod.rs:65-66`). A restated
   visibility predicate is the drift site this whole layer exists to close.
3. **The denial arms**, named explicitly — there are **two**: not-readable, and
   self-audit (the citer auditing their own work, per the load-bearing paragraph above).
   Denial is an arm every domain must name, never an absence and never an `Err` from inside
   `resolve` (`authz/mod.rs:69-74`).
4. **The refusal dialect.** `Forbidden` vs `NotFound` is a deliberate information-hiding
   decision, not boilerplate (`authz/mod.rs:76-85`). The consistency argument favours
   `NotFound`: the evidence **read** shipped by Set 3 is already leak-safe by returning no
   row → 404 (`…memo.sql:234-258`, the `gated` CTE over `resources_readable_by`), so the
   audit **write** over the same subject should refuse in the same dialect rather than
   creating an existence oracle beside a gate built to avoid one.
5. **Machine reach.** How this composes with `machine_authz`'s `AuthorizedReach`, given
   §5.2 requires the auditor to be a registered machine principal. The plan must ground this
   concretely — read `authz/machine.rs`, confirm which grant rows the auditor's
   `provision --team <ref>:member` (§5.2) actually creates, and confirm that
   `resources_visible_to(<auditor machine profile>)` returns the findings it is meant to
   audit. The failure this guards against is silent: the whole pipeline builds and every
   audit 404s in production because the machine principal cannot see the corpus.

---

## 8. Data flow, surfaces, and what restales

**Read path for the auditor.** MCP already exposes `get_block_provenance` (a finding's
per-block citations) and `annotate_resource`. The ledger read is
`GET /api/graph/elements/{node|edge}/{id}/trail` (`crates/temper-api/src/handlers/events.rs:55`),
access-gated (`resources_visible_to` for nodes; anchor **and** both endpoints for edges) and
firewalled to `category = 'domain'`. It is a **discrete per-element call** — the auditor
fetches trails only for the acts it decides to weigh, never as part of a first query pass.

Two gaps to settle in the plan, both additive:

- The SQL returns the whole `metadata jsonb`, but the DTO lifts only `confidence`
  (`crates/temper-core/src/types/element_trail.rs:37-39`). `rationale`, `persona`, and
  `model` — the situational-distance material §3.3 wants — are present in the column and
  dropped by the projection.
- `element_trail` is **REST-only**; there is no MCP tool for it. The auditor reaches it via
  `temperFetch` (as `schedules/steward.ts` hits `/api/steward/dispatch`) unless a tool is
  added.

**What the component split restales**, so the plan sequences it rather than discovering it:
`StandingShape` (temper-core), `openapi.json`, the temper-rb generated gem,
`clients/temper-ts/src/generated/schema.ts`, **both** ts-rs trees, and the
`temper resource evidence` renderer. All are gated by `cargo make check`
(`openapi-check`, `openapi-rb-drift`, `openapi-ts-drift`, `ts-rs-drift`).

---

## 9. Deliberately not designed here

- **`is_corrected` / the provenance scar writer.** Absorbed by §3.4 as a strongly negative
  audit value. Set 5 writes no provenance scar. If a genuine "this source was
  misrepresented" act is ever wanted, it is a separate design with a truth-claim boundary
  problem this spec does not take on.
- **Set 6 (promotion as translation).** Consumes the gate §6.4 establishes; not designed
  here.
- **Context-homed findings.** Out of scope per §6.2 until the queue is widened.
- **Meta-edges / widening the `kb_edges` CHECK.** Not needed under the citation-grain model,
  and not built.
- **Auditing `remote` / `event`-kind citations.** Out of scope per §6.2; the write path
  rejects them rather than silently no-op'ing. A coherent later extension.
- **Time-based re-audit of already-covered findings.** The decay model (§4.1) makes it
  natural, but the first-cut sweep re-queues only on new citations (§6.3). A tuning surface.
- **A `supports`/`corroborates`-edge writer.** The band's `contradiction_balance` conjunct
  is satisfied at the default `0.0` (no contradicts edges), so near-canonical **is** reachable
  through the audit path alone (§3.1's recalibration; the plan pins this with a
  `near_canonical_is_reachable_in_one_pass` test). Set 5 does **not** ship a positive-edge
  writer, so `contradiction_balance` can only ever be `<= 0` today; a finding cannot be pushed
  *above* neutral on that axis until such a writer exists. That is a deliberate non-goal here —
  the vector-sum contradiction axis is Set 6's/a later concern — not a blocked band.
- **Set 4 (steward's three jobs).** Untouched. This spec adds a schedule beside the steward
  and never modifies the steward's own tick.

---

## 10. Faithfulness checks

- **Standing ≠ truth** — an audit assesses the defensibility a citation confers, never what
  a source says or whether a claim is true (§3.4, §9).
- **Events-as-primary / derivable-not-denormalized** — audits are an append-only domain
  event; the visible standing is a decay-weighted projection recomputed fresh from the trail,
  never incrementally from its own prior value, with no stored band (§4.1, §4.3).
- **Scarrification** — nothing is mutated or erased: an adversary's negative verdict is a
  permanent event that fades in influence but stays in the ledger and resurfaces in an
  as-of-then view; the auditor's verdict is itself challengeable and appears in the element
  trail (§4.1, §4.3).
- **No view from nowhere** — the auditor is a situated skeptic, not an authority. Its
  self-assessment is structurally barred from moving standing (§4.2); the citer is barred
  from auditing their own work (§7); and its identity is a distinct ledger-visible principal
  rather than a self-declared label (§5.2).
- **Landmesser** — magnitude, coverage, and quality are kept as separate axes precisely so
  that a high-magnitude monoculture (high magnitude, low coverage ratio or negative quality)
  and a small well-audited evidence set cannot be rendered as the same number (§3.1).
