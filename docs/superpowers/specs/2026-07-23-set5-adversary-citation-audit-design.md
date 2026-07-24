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

### 3.1 Two components, not one — **AMEND (of `indep_breadth`), CONFORM (to §1.1)**

`kb_resource_standing.indep_breadth` (`…memo.sql:32`) becomes two columns:

- **`citation_magnitude`** — the count of distinct live cited sources for the finding.
  **Monotone**: citing more evidence never lowers standing.
- **`citation_quality`** — the **mean** signed audit value over those distinct sources, in
  `[-1.0, 1.0]`.

The full component mapping against the shipped memo (`…memo.sql:30-40`), so no column's
fate is left to inference:

| Shipped column | Fate |
|---|---|
| `indep_breadth` | **Replaced** by `citation_magnitude` + `citation_quality`. |
| `adversarial_survival` | **Subsumed** into `citation_quality`. Survival stops being a separate scalar because the gradient *is* the survival signal — a citation that withstood audit carries a positive value, one that did not carries a negative one. This is the binary-to-gradient move (§3.3) applied to the component it was invented for. |
| `challenge_count` | **Kept, and kept integral** — redefined as **audit coverage**: how many of the finding's distinct cited sources have been audited at all. Its job is unchanged and is the reason it must not become a float: spec §1 requires 0-challenges to be distinguishable from N-withstood, and that is a question about whether anyone *tried*, not about magnitude. |
| `contradiction_balance`, `freshness` | **Unchanged.** |
| `r_parent` | **Unchanged, and deliberately not the same thing as `citation_magnitude`.** `resource_r_parent` counts *all* uncorrected provenance rows over live blocks (`…memo.sql:51-57`) — total accretion, duplicates included — while magnitude counts *distinct sources*. Ten citations of one source is `r_parent = 10, citation_magnitude = 1`, and that difference is load-bearing: it is the echo case. An implementer who collapses them reintroduces the actor-count fallacy. |

`standing_band` (`…memo.sql:186-200`) is re-thresholded over the new component set. Its
`near-canonical` arm must require **both** magnitude and positive quality; a high-magnitude,
negative-quality finding is the monoculture and must not reach it.

They must not be collapsed into a product or a sum. A signed per-citation value summed into
one breadth number has a perverse gradient — a finding with ten unaudited sources would
score worse than one with a single unaudited source — and multiplying by a magnitude term
makes it worse, not better, because magnitude × negative digs the hole faster.

The deeper reason is that the collapse is the same error spec §1.1 already forbids:
**standing IS the vector; the band is a lossy read-time chip over it**. Kept separate, the
two components are exactly what distinguishes diverse-high from echo-high — the Landmesser
boundary made computable. A saluting crowd reads **high magnitude, negative quality**; a
small well-audited evidence set reads **low magnitude, positive quality**; and no single
blended float can tell those apart.

- CONFORM — spec §1.1 (shape-primary, band as lossy chip) and §1.3 (memoized components,
  band computed at read). `standing_band` (`…memo.sql:186-200`) stays a read-time function
  over components and is re-thresholded to take both.

### 3.2 The unaudited prior is negative, not neutral — **EXTEND**

An unaudited citation contributes **`-0.5`** to the quality mean. Not zero.

Zero would make the auditor's absence indistinguishable from its approval — the "absence of
challenge is not survival" failure (spec §1) reappearing at citation grain. A negative prior
states the design's actual posture: evidence that no second party has weighed is *suspect*,
not merely uncounted. Because quality is a **mean**, accumulating unaudited citations cannot
drive it below the prior — the floor is the prior — so the conservative posture costs
nothing in monotonicity.

The exact value is a tunable default owned by this set, in the same sense Set 3 owned the
band thresholds and the 30-day freshness half-life (`…memo.sql:59-75`).

- EXTEND — authorized by spec §2.4 (silence is not evidence of independence) and §1
  (0-challenges must be distinguishable from N-withstood), relocated to citation grain.

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

**The `TODO(Set 5)` at `db_backend.rs:1170` dissolves rather than being fixed.** It exists
only because breadth read a memo built from independence *edges*, so edge writes had to
re-drive it. Once breadth reads citations and audits, the refresh trigger is the
citation/audit write path. No edge-incident refresh is ever added, and the staleness it
warned about becomes unreachable by construction rather than by circumstance.

---

## 4. What a citation audit is

### 4.1 The grain forces the representation — **CONFORM**

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
| **A new event-sourced `kb_citation_audits` projection** | **Chosen.** |

`kb_citation_audits` carries one row per `(citation, auditing act)`: the citation key, the
signed value, and the emitting event id. The standing read takes the **latest live audit per
citation**; superseded audits remain as history. An audit is thereby itself scarrable and
re-auditable — the auditor's verdict is an emitted, fallible claim like any other, never an
authority. This is the house pattern (a `citation_audited` domain event, a `_project_*`
half, a derived projection), inheriting replay-stability, the append-only guard, and the
ledger firewall with no new mechanism.

- CONFORM — events-as-primary, projections derived; the `block_annotate` precedent
  (`migrations/20260710000001_block_provenance_annotate.sql:44`) for a payload-only,
  chunk-independent write path.

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

### 6.2 Scope boundary — cogmap-homed findings only

`kb_workflow_jobs.cogmap_id` is `NOT NULL REFERENCES kb_cogmaps(id)`, but Set 3 made the
subject of standing **any** `kb_resource` (`…memo.sql:22`), and resources also home in
contexts. The first cut therefore audits **cogmap-homed findings only**. Widening the queue
is additive but is a separate concern; the boundary is named rather than designed around.

### 6.3 Selection — unaudited citation coverage

The sweep selects findings with **incomplete audit coverage** — `citation_magnitude > 0`
and `challenge_count < citation_magnitude` (§3.1) — ordered by the size of the uncovered
remainder, so the most-cited and least-audited findings are worked first. This is the
auditor's analogue of the steward's ingest-drift sweep.

Coverage, not quality, is the correct predicate. A quality-based sweep ("still at the
floor") would mis-handle the partially-audited finding: one audited citation lifts the mean
off the floor while the rest of the evidence remains unweighed, and the finding would fall
out of the queue having been only glanced at.

**Re-audit requires no extra mechanism.** Annotating new sources onto a finding pulls the
quality mean back toward the prior, which re-enters the finding into the sweep on its own.

### 6.4 Duty-to-challenge-before-promote is structural, not procedural

Quality cannot leave the negative prior without an audit, so no band above `provisional` is
reachable on unaudited evidence. Set 6 (promotion as translation) inherits the gate for
free, and Set 5 does not need to know Set 6 exists. This is the spec's
"duty to challenge before promote" discharged by the data model rather than by an agent
remembering to do it.

---

## 7. Authorization — `can_audit_resource` is an open grounding obligation

**This section is a required pre-plan grounding item, not a resolved design.**

Every authored write today gates on `can_modify_resource`. An audit must **not**: an auditor
that may only assess findings it owns is not an auditor. The gate wanted is *can read the
finding* — the full canonical visibility predicate — plus being a registered, unrevoked
machine principal with reach. That is a deliberate widening (a write authorized by
readability), and it must be designed explicitly rather than falling out of a copied
handler.

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
3. **The denial arm**, named explicitly. Denial is an arm every domain must name, never an
   absence and never an `Err` from inside `resolve` (`authz/mod.rs:69-74`).
4. **The refusal dialect.** `Forbidden` vs `NotFound` is a deliberate information-hiding
   decision, not boilerplate (`authz/mod.rs:76-85`). The consistency argument favours
   `NotFound`: the evidence **read** shipped by Set 3 is already leak-safe by returning no
   row → 404 (`…memo.sql:234-258`, the `gated` CTE over `resources_readable_by`), so the
   audit **write** over the same subject should refuse in the same dialect rather than
   creating an existence oracle beside a gate built to avoid one.
5. **Machine reach.** How this composes with `machine_authz`'s `AuthorizedReach`, given
   §5.2 requires the auditor to be a registered machine principal.

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
- **Set 4 (steward's three jobs).** Untouched. This spec adds a schedule beside the steward
  and never modifies the steward's own tick.

---

## 10. Faithfulness checks

- **Standing ≠ truth** — an audit assesses the defensibility a citation confers, never what
  a source says or whether a claim is true (§3.4, §9).
- **Events-as-primary / derivable-not-denormalized** — audits are a domain event with a
  derived projection; standing components stay memoized-and-recomputed, with no stored band
  (§4.1, §4.3).
- **Scarrification** — audits supersede rather than mutate; the auditor's verdict is itself
  challengeable and appears in the element trail (§4.1, §4.3).
- **No view from nowhere** — the auditor is a situated skeptic with a negative prior, not an
  authority. Its self-assessment is structurally barred from moving standing (§4.2), and its
  identity is a distinct ledger-visible principal rather than a self-declared label (§5.2).
- **Landmesser** — magnitude and quality are kept separate precisely so that a
  high-magnitude monoculture and a small well-audited evidence set cannot be rendered as the
  same number (§3.1).
