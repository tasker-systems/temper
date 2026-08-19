# Set 5 follow-up — per-auditor collapse and audit attribution

- **Date:** 2026-07-24
- **Status:** Design — accepted, implementing
- **Goal:** Evidential standing as substrate (`019f81e9-25f3-7fe1-b563-4acca2e391eb`)
- **Task:** `019f96b4-ddf9-7260-ab18-53f64a916e51`
- **Amends:** [2026-07-23-set5-adversary-citation-audit-design.md](2026-07-23-set5-adversary-citation-audit-design.md)
- **Sits under:** Set 5, shipped as PR #531. This closes the vector that PR knowingly left open.

## Why this exists

Set 5's §4 decided, at the keyboard-holder's direction, that **humans may audit** — the machine
conjunct came off the citation-audit write, leaving it on dispatch and complete. A human audit is a
distinct signal and a promotion mechanic, and that is still the right call.

The session that shipped it recorded the cost honestly and did not pay it:

> ⚠️ I undersold the cost when asking: the arm blocked a griefing vector (any reader can pin a
> finding to `disputed`, no attribution, no retraction). Implemented as decided; mitigations
> (per-audit attribution + rate-limiting) NOT built = the price of the mechanic.

That framing named the right problem and the wrong fix. This spec corrects it.

## The defect, precisely

`resource_citation_quality` (`migrations/20260724000120_standing_citation_components.sql:201-224`)
aggregates in two stages:

```sql
per_source AS (
    SELECT w1.source_id, sum(w1.w * w1.value) / nullif(sum(w1.w), 0) AS source_value
    FROM weighted w1
    GROUP BY w1.source_id          -- ← groups by SOURCE only
)
SELECT coalesce(avg(ps.source_value), 0.0)::double precision FROM per_source ps;
```

Stage 1 is a decay-weighted mean over **every audit that source carries, from every principal**.
Ten `-1.0` audits from one griefer against one honest `+1.0` yields ≈ `-0.82`, not `0.0`; the band
gate (`:313`) turns `citation_quality <= 0.0` into `disputed`. **Volume wins.**

This is the **identical actor-count fallacy the same function already fights one level down.** Its
own `COMMENT ON FUNCTION` (`:226-229`) states:

> 'Two-stage decay-weighted audit mean (spec §3.1): collapse within a source first (one value per
> distinct source, across all its citing blocks), then mean across distinct AUDITED sources. The
> stage order is what keeps a multi-block source from voting more than once.'

A multi-block source cannot vote more than once. **An auditor can.** The principle was generalized
to the source axis and stopped there.

## D1 — Per-principal collapse, not rate-limiting

**Decision.** Add a third collapse stage rather than a write-rate cap.

| | Rate limiting | Per-principal collapse |
|---|---|---|
| Effect on the fallacy | throttles its exploitation | makes it structurally impossible |
| Tuning | a constant somebody must pick and revisit | none |
| Repeat auditing | refused outright | permitted and collapsed to one voice |
| Retraction | unaddressed | **also unaddressed** — see the correction below |

The aggregation becomes three stages:

1. `per_auditor` — `GROUP BY source_id, audited_by_profile_id`, decay-weighted as stage 1 is today.
2. `per_source` — collapse across auditors, weighted by each auditor's freshest-audit weight.
3. the outer mean across distinct audited sources, unchanged.

### Correction — this change does NOT deliver retraction

An earlier draft of this spec claimed retraction "falls out for free" because within a principal's
bucket the decay weighting makes their newest audit dominate their older ones. **Measurement
refutes it.** Old-vs-new on identical fixtures (one auditor posts `-1.0`, then `+1.0` after a gap)
is byte-identical:

| gap | old (2-stage) | new (3-stage) | band |
|---|---|---|---|
| same instant | 0.000000 | 0.000000 | disputed |
| 7 days | 0.080691 | 0.080691 | reinforced |
| 30 days | 0.333333 | 0.333333 | near-canonical |

Whatever retraction exists was already in the shipped two-stage body and is not improved here.

It is also close to inert on any cadence a machine runs at. The half-life is 30 days, so audits an
hour apart weigh `0.9990` and `1.0000` — indistinguishable. An auditor that has emitted 24 verdicts
in a day and then changes its mind moves its own bucket by roughly **1/24**. Within-bucket recency
is real monthly and meaningless hourly.

The honest scope of this change is therefore: **it makes volume weightless. It does not address
retraction.** Making retraction mean something requires the auditor to re-audit on *change* rather
than on a clock — that is
[the trigger-model spec](2026-07-24-auditor-event-driven-trigger-model-design.md), not this one.

Set 5 refused supersession on purpose (`20260724000110_citation_audits.sql:12-17`) and nothing here
reverses that.

### Stage 2 is weighted, not a plain mean — the load-bearing sub-decision

The shipped function documents an invariant at `:177`: *"decay only arbitrates BETWEEN competing
audits."* A **plain** mean across auditors silently destroys it — a two-year-old verdict would
count equally with today's.

Weighting stage 2 by each auditor's freshest-audit weight preserves that invariant **and** still
gives exactly one vote per principal. Both properties are required; neither is negotiable. This is
called out because "just average the auditors" is the obvious simplification and it is wrong.

### What does not change

`resource_audit_coverage` (`:142-152`) counts distinct sources carrying ≥1 audit. Per-auditor
collapse does not change coverage — coverage asks *was this evaluated*, not *by how many*. The
30-day half-life, the `pow()` overflow clamp, the decay drop-out, `citation_magnitude`, the band
gate, and `resource_standing_shape` are all untouched.

> **An incumbent bug, confirmed and deliberately left.** The shipped header at
> `20260724000120:179-180` describes the decay tail as underflowing to zero around 88 years. It does
> not: `pow(0.5, …)` resolves to the **numeric** overload, which does not underflow, and the
> numeric→`double precision` cast **raises `out of range`** between roughly 89 and ~250 years,
> reaching exact `0` only past ~300. So a sufficiently old audit makes this read path error rather
> than fade. Measured, not inferred. It is unreachable in practice — `kb_events.occurred_at` is
> `DEFAULT now()` and `_event_append` takes no parameter for it, so no write path can produce a
> century-old audit — and the gate short-circuits before the aggregation, so it can never reach a
> caller who has not already passed authorization. Fixing it means changing the decay expression,
> which is a different beat with a different argument.

## D2 — Denormalize `audited_by_profile_id`

**Decision.** `ALTER TABLE kb_citation_audits ADD COLUMN audited_by_profile_id UUID NOT NULL
REFERENCES kb_profiles(id)`, nullable → backfilled → `SET NOT NULL` in one migration.

The SQL has no auditor identity to group by. Today it is reachable only as
`audited_by_event_id → kb_events.emitter_entity_id → kb_entities.profile_id`
(`canonical_schema.sql:146,468`). Both links are `NOT NULL` FKs to a PK, so the join is 1:1 and a
backfill cannot strand a row.

Denormalizing over joining, because: `ADD COLUMN` is additive and therefore safe under the
**additive-only-on-`main`** invariant; the standing read is recomputed **live on every call** (never
from the memo, so `freshness` reflects the current moment) and does not want two extra joins on that
path; and the attribution read of D3 needs the same identity anyway.

**The projector fills it from the owning event, never from an ambient current principal.** This is
the replay-stability rule the same projector already follows for `created`
(`20260724000110_citation_audits.sql:68-78`): a replay must reproduce the identical row. Sourcing
the profile from the current principal would re-attribute every historical audit to whoever ran the
replay.

## D3 — Attribution as a sibling read, not a fatter `StandingShape`

**Decision.** `GET /api/resources/{id}/citation-audits` returns the finding's per-audit trail with
the auditor's identity. `StandingShape` is unchanged.

`POST` already lives at that exact path (`handlers/citation_audits.rs:30-31`), so `GET` is the
natural sibling. The shape read stays cheap and aggregate-only by design; attribution is a
variable-length trail and must be opt-in.

**The gate is inside the SQL**, mirroring `resource_standing_shape`'s `gated` CTE over
`resources_readable_by` — one spelling, no drift. Readability is gated as `/evidence` gates it, so
the pair cannot be diffed into an existence oracle: an empty array means *readable finding, no
audits*, never *finding you may not see*.

## D4 — Attribution is disclosure, and that is the intent

**Decision.** The trail names every auditor — `auditor_profile_id`, `handle`, `display_name` — to
any principal who can already read the finding. Recorded as a rule rather than left implicit:

> For any audit on a resource a profile may already see, that profile may also see **who** applied
> it — agent or human. Who has access to the system at all is not private information. This is
> transparency and accountability, and it is the point of the read.

A review probe framed this as a cross-tenant leak, showing one profile learning another's handle
with no team in common. **That framing does not apply: temper has no tenancy.** Tenancy is a
self-hosting deployment concern, not a system concept — two teams sharing a cogmap are not two
tenants, they are two teams that were deliberately given the same map. The disclosed set is bounded
by construction anyway: `AuditAuthority` admits only principals who could read the finding, so every
auditor named is already a co-reader. `kb_profiles.email` is never selected.

The established precedent is `resource_row`, which already projects `owner_handle` to any reader
(`readback/mod.rs:419`). This extends that from the owner to the auditors, on purpose.

## Residual — stated, not hidden

- **One griefer plus one honest auditor averages toward `disputed`.** Under the collapse that is a
  *legitimate* dispute — two principals disagreeing, one vote each — and it is attributable. It is
  no longer volume. (An earlier draft wrote "*still* averages toward `disputed`"; that was wrong.
  Before this change a repeat-auditor could out-weigh a griefer by volume, and now cannot. That is
  the intended effect, not an unchanged one.)
- **A principal who audits one citation many times now carries one vote.** This is the collapse
  working, and it applies to diligent and malicious repeaters alike — weight cannot be earned by
  repetition. The auditor persona is structurally the biggest repeater in the system (same
  credential, same model family, same instructions every run), which is precisely why N of its
  verdicts must count as one observation rather than N.
- **Write-volume and storage abuse are unbounded by this change.** The collapse makes extra audits
  *weightless*, not *free*. A per-principal cap is deliberately not built: the real answer is to
  stop generating redundant audits at the source, which is the trigger-model spec's job.

## Acceptance

- N audits from one principal count once, decay-weighted, in `citation_quality`.
- A principal's newest audit of a citation dominates their own earlier ones *within their bucket*
  (subject to the half-life — see the retraction correction above).
- A stale verdict does not outweigh a fresh one across auditors.
- A source whose every audit has decayed still drops out rather than reading as `0.0`.
- The evidence trail attributes each audit to its auditor, gated as the shape read is.
- `cargo make check`, `test-db`, and `test-artifacts` green.

> **On "additive":** both migrations `DROP FUNCTION` and re-`CREATE` with an identical name and
> signature inside one transaction, and `…000200` also does `ALTER COLUMN … SET NOT NULL`. There is
> no window outside the transaction and no signature change, so migrate-ahead-of-deploy stays safe —
> but calling this "additive-only" would be imprecise, and each migration's header flags it.

## Non-goals

- Rate limiting or any write-volume cap.
- Supersession, retraction-as-a-verb, or any `is_superseded` column.
- Changes to `resource_audit_coverage`, `citation_magnitude`, the band gate, or the half-life.
- The reaper pass / terminal "cannot assess" verdict — that is Set 4's growth.
