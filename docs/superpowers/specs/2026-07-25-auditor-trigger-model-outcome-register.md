# Outcome Register: the auditor's event-driven trigger model

**Witnesses**: goal `019f9a34-3306-70d1-b07a-f23c99943751` → **C1** (fully), **C1a**/**C1b**, **C3** (partially — this document and its corrections are the evidence C3 judges).
**Subject**: task `019f975e-7be9-7ff3-a5bd-ef7ea72ff4a5`; spec `docs/superpowers/specs/2026-07-24-auditor-event-driven-trigger-model-design.md` (D1–D8, marked *accepted*).
**Subject reassigned to C1 by** decision `019f9a64-b738-75d2-8d0a-b832872b6a64`.
**Register schema**: research `019f9a32-e1b2-7f43-b4cf-ac9b58447cb9`; refusal face per `019f9a33-90e5-7882-bf63-61898a33e78d`.
**Findings summary (visual)**: https://claude.ai/code/artifact/e55cd4cb-5796-43ef-82cf-8359b80f8e68 — the corrections, the refusal ladder, and the two-axis exercise status, laid out for review. Keep this link with the register wherever the register is cited.

> **Read §10 and §11 first if you are short of time.** The spec this register covers is marked
> "accepted, all eight decisions settled." One grounding pass falsified parts of D1, D3, and D5, and
> refuted the premise all eight rest on. The register's value here was not tidier expression — it was
> that its required instruments *fired*.

---

## 1. Why-anchor

The auditor exists so a finding's cited evidence is assessed by someone other than its author. Its
trigger model protects **Pete's attention, and every future reader's, from a standing number that
looks earned and is not** — a verdict re-emitted on a clock is a verdict about nothing, and a queue
that re-heads a declined citation forever spends an agent's budget re-asking a question already
answered.

When this anchor stops describing anything anyone cares about — if standing is never read, or if
findings stop carrying citations — the trigger model is due for supersession, visibly, not quiet
maintenance.

---

## 2. Situated actors

Never "a user." The partition below is over the **audit write** and the **queue endpoints**, which
turn out to have different answers — see EC2.

| # | Actor class | Registered machine? | Reads finding? | Contributed citation? |
|---|---|---|---|---|
| A1 | Auditor machine principal | yes | yes | no |
| A2 | Human reader, non-author | no | yes | no |
| A3 | Citation's contributor (human or machine) | either | yes | **yes** |
| A4 | Principal without read | either | **no** | — |
| A5 | **Revoked** machine principal | registered-but-revoked | yes | no |
| A6 | Admin/system emitter | n/a | n/a | n/a |

### Equivalence claims — stated so they can be attacked

**EC1 — A1 and A2 are interchangeable for the audit write.** **TRUE, and deliberate.**
`AuditAuthority::resolve` (`crates/temper-services/src/authz/audit_gate.rs:269-313`) runs exactly
three probes — `is_resource_visible`, `citation_contributed_by_profile`, `can_modify_resource` — and
**no** registration probe. The enum carries no `NotMachine` arm (`:224-244`). Pinned by
`a_human_reader_who_is_not_the_author_may_audit` (`:799-821`). This was a product decision, not an
oversight: a human audit is a distinct and arguably stronger signal, wanted as a promotion mechanic.

*Falsified when*: `AuditAuthority` gains a registration arm, or `is_registered_principal` appears in
`resolve`.

**EC2 — A1 and A2 are NOT interchangeable for the queue, and the queue is not internally consistent.**
**TRUE, and this is a defect the register surfaces rather than inherits.** One feature, four
endpoints, three different registration answers:

| Endpoint | Registration required? | Denial rendering |
|---|---|---|
| `GET /api/auditor/sweep` | **none** | — |
| `POST /api/auditor/dispatch` | yes | **403** (`audit_gate.rs:149-154`) |
| job-complete | yes | **404** (`audit_gate.rs:407-419`) |
| `POST /api/resources/{id}/citation-audits` | **none** | 404 |

Dispatch and complete guard the *same* resource with the *same* probe and disclose differently. The
403 is defended at `audit_gate.rs:146-148` — *"there is no subject whose existence a refusal could
confirm, and a 404 on a fixed route would just be a lie"* — which is correct for a fixed route, and
job-complete is also a fixed route. **One of the two is wrong; the register does not resolve which,
it declares the inconsistency examined.**

**EC3 — all principals who cannot audit a citation are interchangeable.** **FALSE, and D7 depends on
the falsity.** A3's denial is **permanent and self-inflicted** (`kb_block_provenance` is immutable, so
`citation_contributed_by_profile` never changes answer). A4's denial is **contingent** — one grant
flips it. D7 correctly scopes `uncovered` to *readable AND non-self-authored*; the register records
**why both conjuncts are needed and why they are not the same kind of thing**: one is a fact about
the past that will never move, the other is a fact about now that may move at any time. A queue that
treated them alike would either permanently drop citations a grant would have made auditable, or
permanently re-offer citations that will 404 forever.

**EC4 — a revoked machine (A5) is denied everywhere.** **FALSE for the audit write.**
`a_revoked_machine_is_still_admitted_by_this_gate_because_authn_stops_it` (`audit_gate.rs:829-847`)
pins that `AuditAuthority` returns `Auditor` for a revoked machine; the defence is that authentication
stops it upstream. That defence is sound **only while every path to the write authenticates through
the machine-client lookup.** It is recorded here because it is a live dependency between two layers
that no test spans, and because A5 is precisely the actor an attacker controls after a revocation.

**A6 (admin/system) is deliberately excluded**, not unexamined: `admin` and `system` are
CHECK-constrained categories (`kb_event_types_category_check`), 7 members total, and the trigger
model neither reads nor writes through them.

---

## 3. Priors vs. provided

| | The auditor holds as prior | Must be supplied per act |
|---|---|---|
| **Reach** | its cogmap set, via `steward_candidate_cogmaps(principal)` | — |
| **Work** | — | the citation list in the dispatch payload (D6) |
| **Judgement inputs** | the four in `instructions.md:77-89` — connection, recorded confidence, corroboration, citation-set size | — |
| **Prior audits** | **nothing** — see §7 | — |

That last row is the defect D6 exists to fix, and it is a *priors* failure, not an instruction
failure: the agent is told *"you do not need to re-check whether they need auditing"*
(`schedules/auditor.ts:161-165`) at **finding** grain while its unit of work is **citation** grain
(`instructions.md:72-75`). A finding with 4-of-5 citations audited is selected on the fifth, and the
agent is instructed not to re-check and then to emit one verdict per auditable citation — all five.
The prompt does not merely omit a skip step; it **actively forbids the check** that would prevent the
duplicate.

---

## 4. The act

**Topic**: `citation_audited`, `domain` category, registered, **has a production emitter**
(`_event_append('citation_audited'`, via `citation_audit`).
**Anchor**: the finding's home, *preferring* `kb_cogmaps` (`citation_audit`'s
`ORDER BY (anchor_table = 'kb_cogmaps') DESC LIMIT 1`).
**Character**: mutation, append-only. No supersession, no `is_superseded` column
(`20260724000110:12-17`).
**On-behalf-of**: `audited_by_profile_id`, filled by the projector from the **owning event**, never
from an ambient principal — so replay cannot re-attribute history to whoever ran it.

---

## 5. The three-way Then (successful re-audit on material change)

**Synchronous postconditions.** A row in `kb_citation_audits` with `audited_by_event_id` unique
(the only UNIQUE on the table). `resource_standing_shape` recomputes live — `citation_magnitude`
unchanged (it counts distinct *live sources*, not audits), `audit_coverage` unchanged if the citation
was already covered, `citation_quality` re-collapsed through all three stages.

**Ledger trace.** One `citation_audited` event anchored to the cogmap, carrying the emitter entity;
the projector writes `audited_by_profile_id` from it. `GET /api/resources/{id}/citation-audits`
surfaces the attributed trail. **`GET /evidence` does not** — attribution shipped as a deliberate
opt-in sibling (`20260724000220:12-18`) so the cheap read does not pay for the expensive one.

**Eventual stable-state, after run-to-quiescence.** The citation's watermark advances (the trail *is*
the watermark — `audited_by_event_id` + UUIDv7 ordering, D2 Tier 1, no new column). Under D1 the
citation is **not** re-selected until a material event exceeds that cursor. **This is the clause with
no witness today and it is the one that matters**: nothing currently asserts non-reselection, because
nothing implements D1.

---

## 6. The refusal face

| # | Refusal | Ground | Rung | Recourse | Status |
|---|---|---|---|---|---|
| R1 | Self-audit (`Author`) | **standing** | **0** (404, indistinguishable from unreadable) | none disclosed | shipped, tested |
| R2 | Unreadable finding | **standing** | **0** (404) | none disclosed | shipped, tested |
| R3 | Value outside `[-1,1]` / NaN | **commitment** | 2 (400, ground stated) | fix the value | shipped, tested |
| R4 | Non-resource-kind source | **commitment** | 2 (400) | audit a resource citation | shipped, tested |
| R5 | `(block, source)` not a live citation | **commitment** | 2 (400) | — | shipped, tested |
| R6 | `cap < 1` | **commitment** | 2 (400, echoes the value) | supply ≥ 1 | shipped |
| R7 | `cap > 500` | — | **0 — silently clamped, 200** | **none; caller cannot tell** | shipped |
| R8 | Dispatch by non-machine | **standing** | 1 (403) | register a machine principal | shipped |
| R9 | Job-complete by non-machine | **standing** | **0** (404) | — | shipped |
| R10 | **"I cannot assess this citation"** | **evidential?** | **inexpressible** | — | **does not exist** |

### R1 is a deliberate rung-0, and it is the most interesting row

`audit_gate.rs:330-333`: *"a `Forbidden` distinguishable from the unreadable case would tell a prober
'you may see this finding but you wrote it'."* Correct, and it means **the refusal carrying the most
actionable information for a legitimate caller is the one most deliberately hidden.** A human who
authored a citation and tries to audit it gets a 404 that is a lie by design — the generating
invariant holds (their visible manifold does not entitle them to learn the gate's reasoning), but the
attention cost lands on a legitimate actor.

### R7 is an unexamined rung-0 that nobody chose

`cap < 1` refuses with the offending value echoed; `cap > 500` is silently truncated with a 200. Same
parameter, opposite disclosure, and the asymmetry is defended for the *upper* bound only as *"as much
as you can is a coherent request"* (`auditor.rs:78-79`). It is a rung-0 disclosure decision made as
an ergonomics decision. Recorded as **examined-and-questioned**, not endorsed.

### R10 — the refusal the system cannot make, and the evidential-ground verdict

`20260724000130:70-82`, verbatim:

> *"a citation that is readable, live, and simply never gets audited will re-head this queue every
> tick with the SAME `uncovered` count forever."*

The auditor can decline a citation, and the system has **no way to record that it did**. Coverage is
monotone; there is no terminal verdict, no backoff, no aging. The refusal exists in the agent's
reasoning and nowhere in the ledger — and an unrecorded refusal is indistinguishable from a network
error, which is exactly what `019f9a33` says a refusal must never be.

**Verdict on the open taxonomy question, and it is a negative result.** R10 is the only candidate
here for an *evidential* ground, and **it does not qualify**. The auditor declining "I cannot assess
this" is a statement about the auditor's own competence against a specific citation, not about
accumulated corroboration crossing a threshold. Every other row resolves cleanly to standing or
commitment. **This subject cannot settle whether evidential is a third ground** — consistent with the
goal's named remainder, and now confirmed from the subject rather than predicted from outside it.

### Inertness

Covered by the canonical system-level test (goal C2), not restated per-feature. One subject-specific
note: R1/R2 are inert **only if** the 404 path writes nothing, including no audit-attempt record.
Confirmed — `authorize` returns `Err` before `db_backend.rs:2072`'s write.

---

## 7. Exercise status — **and a schema amendment**

The research doc's element 7 offers two values: *specified/merged* versus *ever-executed*. **That is
insufficient, and this subject is the counterexample.**

Production runtime logs, `steward-agent`, 2026-07-25:

```
17:30:12 GET /eve/v1/cron/_RIx5Ny…  200
    [auditor-dispatch] tick a05f3b2a-5adb-4146-8e5d-ba9a9808f31d starting (temper-ts 0.0.0)
    [auditor-dispatch] tick a05f3b2a… failed: Error: TEMPER_AUDITOR_TOKEN is required
        at requireEnv (file:///var/task/index.mjs:703:20)
16:30:18 GET /eve/v1/cron/psCxld_…  200
    [auditor-dispatch] tick 11aa2bf5-0ac6-46ea-ba9e-2fc741d5e716 starting …failed: … TEMPER_AUDITOR_TOKEN is required
```

Deployed since **2026-07-24T23:16:27Z** (deployment `5596549062`, ref `dad69939`, status `success`);
firing hourly at `:30`; ~18 wakes; **every one dies at `requireEnv` before the outbound fetch.**

**Amendment — exercise status needs two axes, not one value:**

| | Trigger fires? | Work executes? |
|---|---|---|
| auditor cron | **YES** (~18 ticks) | **NO** (dies at `requireEnv`) |

A one-axis check reads this wrong in *both* directions. "Does any `citation_audited` event exist?" →
zero → "never ran." "Has the schedule fired?" → 18 → "exercised." Neither is true. The spec's own
premise — *"nothing here is deployed"* — is the first reading, and it is **false**: the thing is
deployed and running, it is merely inert.

This matters beyond bookkeeping. The original lesson (session `019f978d`) was *don't treat a
never-run artifact as a constraint*. The spec then over-applied it and asserted non-deployment of
something that had been live for a day. **The instrument that catches one direction must be built to
catch the other, or it just moves the error.**

### Per-artifact status

| Artifact | Landed | Status |
|---|---|---|
| `POST /api/resources/{id}/citation-audits` | 2026-07-24 | **exercised in test** (HTTP + CLI e2e + SQL) |
| `GET …/citation-audits` (trail) | 2026-07-25 | **exercised in test** (5 cases) |
| `audit_drift_sweep` | 2026-07-24, **1 commit ever** | **exercised in test** (SQL fixtures only); its one production caller untested |
| `POST /api/auditor/dispatch` | 2026-07-24 | **merged, zero tests** |
| `schedules/auditor.ts` | 2026-07-24, 2 commits | **deployed + firing + inert**; run handler untested |
| `subagents/auditor/instructions.md` | 2026-07-24, 2 commits | **specified only** — no test executes the prompt |

**Nothing here is more than two days old or carries more than three commits.** There is no age to
mistake for ratification; the live risk is the opposite — treating yesterday's first draft as settled
because it merged.

---

## 8. Closure

**Axes closed over**: situated actor (§2), event type (§8.1), refusal ground (§6).
**Axes explicitly OPEN** — per the goal's requirement that rate-shaped axes be named open unless
enumerated:

- **Cadence** — `30 * * * *` was never chosen; it trails the steward's `0 * * * *`. Under D1 the
  clock becomes only the *wake*, so cadence stops being load-bearing — but it is not thereby closed.
- **Volume** — nothing bounds repeat audits by one principal on one citation. The per-auditor
  collapse makes them **weightless, not free**; the write is still unrated, and mitigation (b) of
  `audit_gate.rs:30-31` remains genuinely unbuilt (the only textual hit for "rate-limit" in
  services + api + migrations is the comment asserting its own absence).
- **Payload size** — D6's citation list is "bounded by the same finding budget," which bounds
  *findings*, not citations-per-finding.

### 8.1 The event-type axis — closed, and it caught things

38 registered types; `category` CHECK-constrained to `domain` | `admin` | `system`. All 23 names the
spec classifies exist, exactly spelled. Two findings:

**(a) Four of thirteen "material" events cannot be emitted.** `block_folded`,
`block_provenance_corrected`, `relationship_retracted`, `relationship_corrected` have no production
write path. Three are named in a constant that exists to say so —
`crates/temper-substrate/src/events.rs:1341`:

```rust
const NO_WRITE_PATH_YET: [&str; 4] = [
    "relationship_decayed", "relationship_corrected",
    "block_folded", "block_provenance_corrected",
];
```

**The spec's tripwire flags `relationship_decayed` — the one member it excluded — and admits the
other three without noticing.** Sharpest: `block_provenance_corrected` is the spec's *sole* staleness
signal for "the size of the citation set" alongside `block_provenance_annotated`. That axis would
silently never fire.

**(b) Eight `domain` types are classified neither material nor excluded; seven are live.**
`block_created`, `resource_deleted`, `resource_finalized`, `resource_reassigned`,
`context_reassigned`, `cogmap_seeded`, `charter_set`, plus `relationship_decayed` (inert). At least
two bear directly on the subject: **`resource_deleted`** tombstones a *cited source*, and
**`resource_finalized`** is the moment a segmented source *becomes* citable. Named here as a
remainder; not silently dropped.

#### Discharged 2026-07-25 — the partition is now exhaustive over `domain`

Classified against D1's four judgement inputs and D4's boundary (*"if it cannot enter the
judgement, it cannot invalidate the verdict"*). The material set goes **13 → 17** (still 4 inert).

| event | verdict | ground |
|---|---|---|
| `block_created` | **material** | Content touch. `replay.rs`'s own `CONTENT_EVENTS` already groups it with `block_mutated` and `block_folded`, both material. Live via `block_append` (`20260708000012`). Input 1. |
| `resource_deleted` | **material** | Two inputs at once: the cited source stops being live (1), and `resource_live_citations` filters `src.is_active`, so `citation_magnitude` moves mechanically (4). |
| `resource_finalized` | **material** | The moment an `in_progress` resource enters list/search — `20260714000001`: *"`ingest_state = 'complete'` goes exactly where `r.is_active` already goes."* Input 3. |
| `context_reassigned` | **material** | Changes `kb_contexts.owner_id`, i.e. the OWNS half of `steward_team_contexts`. Input 3 — but see C-7: admitting it is necessary and **not sufficient**. |
| `resource_reassigned` | excluded | Owner on the home row. D4 exactly: metadata-*about*, not content-*of*. |
| `cogmap_seeded` | excluded | Map genesis — no citations and no audits exist yet, so there is no verdict to stale. Structural, not judgemental. |
| `charter_set` | excluded | Changes what the map is *for*, not what a source says. Barred by the auditor's own boundary; also fires on ordinary map operations (every charter, incl. L0), so admitting it would add standing noise. |
| `relationship_decayed` | excluded, pending phase 4 | Inert; D3's tripwire already governs it. Now *classified* rather than unclassified. |

**C-4 is resolved by this table.** Left in §10 as a visible scar, per B-5's precedent.

### 8.2 Schema amendment — the closure section needs an **emittability** axis

Finding (a) is neither actor-shaped nor rate-shaped, so neither the original closure discipline nor
the scar the goal added to it would catch it. The generalization:

> **For any allow-list drawn over a registered vocabulary, "is registered" ≠ "can occur."** A closure
> declaration over such a vocabulary must state, per member, whether a production write path exists —
> otherwise the declaration is complete against the *registry* and full of holes against *reality*.

This is the closure analogue of exercise status: §7 asks whether a *behavior* ever ran, §8.2 asks
whether a *vocabulary term* ever can. Both are "the corpus describes more than the system does."

#### 8.2a — the converse, found while discharging §8.1(b)

The emittability axis has a mirror, and the codebase carries a live instance of it.
`crates/temper-substrate/src/replay.rs:615-619` declares:

```rust
/// `block_created`/`block_folded` are listed forward-compatibly (no mutation fires them yet)
const CONTENT_EVENTS: &[&str] = &["block_mutated", "block_created", "block_folded"];
```

`block_created` has fired since **`20260708000012_streaming_ingest.sql`** — whose own header reads
*"activate the dormant `block_created` event"* — via `SeedAction::BlockAppend → EventKind::BlockCreated`
and `_event_append('block_created', …)`. The comment is stale by seventeen days.

Where §8.2 says *"is registered ≠ can occur,"* this says:

> **"is commented inert ≠ is inert."** A prose claim about emittability decays exactly as a prose
> claim about behavior does (§7), and in the same direction — toward *understating* what the system
> now does, because a comment is written once and the write path is added later.

Both directions matter and they fail differently: §8.2's error admits a term that can never fire
(a silently dead axis); §8.2a's error **excludes a term that fires constantly** (a silently missed
one). Neither is visible to a reader who trusts the corpus. This was caught only because the
classification checked the write path instead of either comment — the two comments disagreed, so
neither could be trusted, which is the tell.

---

## 9. Stated silence

Not specified here, deliberately: the reaper / terminal "cannot assess" verdict (R10 — D1 reduces its
urgency but does not replace it); the aggregation (correct under any trigger model); any new
authorization surface; supersession or retraction-as-a-verb.

**Named remainder** (examined-and-deferred, not unexamined): the evidential refusal ground — see §6,
where this subject returns a *negative* result and cannot settle it.

---

## 10. Corrections to the spec

The spec is marked *"accepted; all eight decisions settled."* It is not.

**C-1 — D5's proposed extraction already shipped, under another name, and the spec quotes a dead
body.** D5 says *"extract the steward's inline 'team contexts, owned ∪ shared' CTE as
`cogmap_observable_contexts(p_cogmap)` — a new function, additive."* That extraction landed
2026-07-16 as **`steward_team_contexts(p_cogmap)`** (migration `20260716000010`, which DROP+CREATEs
`steward_ingest_delta`). The live `steward_ingest_delta` already calls it. **Worse than redundant:**
the live function traverses `team_ancestors(tc.team_id)` so shares inherit down the team DAG; the
dead 2026-07-01 inline CTE joined `kb_team_contexts` directly. *Extracting the inline CTE would ship
a narrower reach predicate than production runs.*

**C-2 — D5's second justification does not hold.** D5 claims reuse would re-arm the auditor because
`new_events` *"counts every event in scope, including `citation_audited`."* But that window filters
`producing_anchor_table = 'kb_contexts'`; `citation_audit` anchors preferring `kb_cogmaps`; and
`audit_drift_sweep` only enqueues cogmap-homed findings. The back door is already shut by anchoring.
D5's *conclusion* (the auditor needs its own delta) still stands on its **first** reason — the
steward's delta counts `resource_created` only and is blind to relationship churn — but one of its
two legs is gone.

**C-3 — D1/D3's material set has four inert members.** §8.1(a).

**C-4 — the material/excluded partition is not exhaustive.** §8.1(b): eight unclassified `domain`
types, seven live.

**C-5 — the premise is false.** §7. *"Nothing here is deployed"* → deployed, firing hourly, inert on
`TEMPER_AUDITOR_TOKEN`.

**C-6 — `clamp_auditor_cap` is Rust, not SQL** (`crates/temper-core/src/types/workflow_job.rs:39-64`).
Minor, but the spec cites it as a SQL precedent.

None of these were caught by three adversarial review lenses on the prior PR. All six fell out of one
grounding pass whose *shape* the register dictates.

**C-7 — D2 tier 2 counts events *inside* a boundary that itself moves uncounted.** Found 2026-07-25
while discharging §8.1(b); this one changes a decision rather than a citation, and it is not fixable
by any choice of allow-list membership.

The observable space is `steward_team_contexts(cogmap)` =

```
OWNS    — kb_contexts.owner_table/owner_id = the joined team
  ∪
SHARED  — kb_team_contexts, inherited down the team DAG via team_ancestors()
```

Both halves move, and neither is countable:

- **OWNS** moves via `context_reassigned` — **one** event standing for **N** resources. Admitting it
  to the material set (§8.1(b)) is necessary but not sufficient: a threshold over a count
  under-weights it by an unbounded factor.
- **SHARED** moves via `share_context` / unshare — **zero** events. `context_service.rs` states the
  reason outright: *"Context creation is a plain INSERT (no event emission — product decision 5:
  contexts are infrastructure)."* The write is `INSERT … ON CONFLICT DO NOTHING` on
  `kb_team_contexts`, with no `_event_append` anywhere on the path.

**Counting events inside a boundary cannot detect the boundary moving.** So this is a category error
in the measurement, not a tuning parameter — which is why no weighting rule resolves it, and why
"just event the share path" is the wrong reflex: it argues against a standing product decision, needs
a projector + replay arm + backfill, and *still* leaves the one-event-N-resources problem on the OWNS
half.

**Blast radius — inference, not data.** Nothing mutates. `kb_citation_audits` is append-only,
attribution is projector-filled from the owning event, recorded `confidence` is immutable by D1, and
the three standing axes are **not** visibility- or boundary-scoped (`resource_live_citations` gates
only on `is_corrected` / `is_active` / `is_folded` / `source_kind`). The defect is confined to
*selection*, and it strictly **under**-triggers — the direction `steward_team_contexts`'s own comment
names as the dangerous one (*"would UNDER-trigger … and silently stale the map"*).

But it is worth fixing rather than tolerating, because of what D1 makes absence *mean*: under
change-triggering, *"an unrefreshed verdict is a verdict about **something that has not changed**"* —
that is the whole reason a 30-day half-life is defensible rather than arbitrary. When the boundary
moves invisibly, the absent re-audit is still read as stability. The staleness is not merely missed,
it is **misreported as stability**, in precisely the model that converts that absence into a signal.

**Proposed remedy — split the question.** Content *churn* stays a counted threshold. Boundary
*change* is detected directly: a fingerprint of `steward_team_contexts(cogmap)`
(`md5(array_agg(context_id ORDER BY context_id))` — the function is already `STABLE`) snapshotted
beside `auditor_watermark_event_id`. Tier 2 fires on `count >= threshold` **OR**
`fingerprint <> stored`.

This is D2's own move for tier 1 — *observe the thing, do not reconstruct it from a proxy* — with one
asymmetry that is the actual design content: **tier 1 could derive because the trail has history; the
boundary has none.** `kb_team_contexts` is current-state only, so the boundary cannot be
reconstructed as-of a past watermark and must be *snapshotted*. Costs, stated: a second piece of tier-2
state; a fingerprint reports *that* it moved, never *how much* (a 1-resource context fires as loudly
as a 10,000-resource one — deliberate over-approximation); and the snapshot must advance in the same
act as the watermark, or the two diverge into a permanent re-fire.

---

## 11. Break report — where the register could not hold shape

Per C1, this section is a success condition, not an apology.

**B-1 — Element 7 (exercise status) is under-specified as a single value.** §7. Two axes needed.
**Schema amendment proposed.**

**B-2 — The closure discipline has no emittability axis.** §8.2. **Schema amendment proposed.**

**B-3 — The refusal face has no slot for a refusal the system cannot express.** R10 is real, is
documented in a migration header, and has no row shape: no ground fits cleanly, the rung column is
meaningless (nothing is disclosed because nothing is recorded), and recourse is undefined. The
schema assumes refusals *happen*; it has no way to record that one **should** happen and **cannot**.
Proposed: an `inexpressible` status, distinct from a refusal at rung 0 — rung 0 means *the actor
cannot tell*; inexpressible means *the ledger cannot say*.

**B-4 — "Situated actors" assumes one gate per act.** §2/EC2: one feature, four endpoints, three
registration answers, two contradictory disclosure choices for the same probe. The actor table wants
to be actor × act, and the act column is not enumerable from the spec — it took reading four
handlers. The schema should require the **act partition** to be enumerated before the actor
partition, or EC2-shaped inconsistencies stay invisible.

**B-5 — Cost note for C3 (⟲ corrected 2026-07-25; the first draft of this clause was wrong).**

*The original text read: "the register did not make the corrections in §10; the grounding pass did.
What the register contributed was merely requiring that pass." Left visible as a scar because the
error is instructive — it is an **understatement** defect in a register whose job is calibrated
claims, and an understated finding miscalibrates exactly as badly as an inflated one.*

The correction, from Pete: *"if the discipline-as-workflow forced a grounding pass as non-optional,
and not doing that is what has routinely led to drift, then it did what it was supposed to. That's
like saying all the hand did was wield the hammer."* Correct. The original conflated **where the
cost lives** with **what caused the finding**, and answered the second question with the first.

The causal decomposition, stated exactly, because the credit is not uniform:

| Finding | Cause |
|---|---|
| C-5 — the premise is false (deployed + firing + inert) | **Element 7 (exercise status)**. No generic grounding pass asks *"has this ever run?"* — that is register content, and the newest element. |
| C-4 — eight unclassified `domain` event types | **The closure discipline** (named remainder, never silent), which forced enumeration of all 38 registry members. |
| C-3 — four inert "material" events | **Exceeded the register.** An agent asked a question no register section specified — which is why it is filed as schema break B-2, the emittability axis closure lacks. |
| C-1/C-2 — D5's stale extraction and void justification | **Adjacent existing discipline** — the temper skill's `plan-verification.md` (*"does this codebase already have a name for this concept?"*), not the register. |

Two findings caused by register elements, one that exceeded the register and became an amendment,
two from discipline already in the skill.

**The control condition is in the evidence and was underweighted**: three adversarial review lenses
ran over the prior PR and caught none of these. Not proof, but the closest available counterfactual,
and it points *toward* the convention rather than away.

**What survives of the original, and it is worth keeping:** the expensive, load-bearing part of
adopting this convention is the **grounding pass**, not the prose. Budget four read-only subagents
and a production log read, not an afternoon of document authoring. That is a cost statement, and it
is compatible with the register deserving the credit.

---

## 12. Witness decomposition (C1a)

Every Then-clause either has a witness or is a visible hole. Tasks to file against this register:

| # | Clause witnessed | Mode | Bites against |
|---|---|---|---|
| W1 | §5 stable-state: a citation is **not** re-selected absent a material event | executable | today's `audit_drift_sweep`, which re-selects forever |
| W2 | §8.1(a): every member of the material set has a production write path | executable | the current D3 list (fails on 4) |
| W3 | §6 R10: a declined citation leaves a durable trace | executable | nothing records it today |
| W4 | EC3: `uncovered` excludes self-authored citations | executable | today's sweep (readability only) |
| W5 | EC2: the four endpoints agree on registration + disclosure | executable | today (3 answers) |
| W6 | §7: exercise status is reported on both axes | judged | one-axis reporting |
| W7 | C-1: no new `cogmap_observable_contexts`; `steward_team_contexts` is reused | judged | D5 as written |

**Uncovered by construction** (declared, not hidden): §8.1(b)'s eight unclassified event types have
no witness until they are classified — that classification is prerequisite work, not a test.
