# Stored region aggregates are region-truth, and the row set is what varies by reader

**Date:** 2026-08-26
**Status:** Decided — recorded, with two gaps named
**Scope:** `kb_cogmap_regions` stored readouts, and `StandingShape`'s components
**Task:** `01a035f2-d37a-7a83-9f6c-b93d58eb5847`

## Decision

A region's stored readouts are computed **once, at materialize time, over every member of the
region**, and returned byte-identically to every reader who clears the gate.

The stored set is **six metrics plus the centroid** — not the four the register row names:

| Column | What it is |
|---|---|
| `salience` | computed blend (telos-alignment + reference-standing + centrality), memoized |
| `centrality` | internal declared-affinity density × size |
| `content_cohesion` | mean member-to-centroid cosine |
| `telos_alignment` | cosine(centroid, telos resource embedding) |
| `reference_standing` | aggregate `reinforce_count` over members |
| `internal_tension` | over oppositional-labeled declared edges among members |
| `centroid` | `vector(768)`, mean over members' pooled chunk vectors |

`StandingShape`'s six components are the same class, computed over a finding's full citation set and
full express-edge neighbourhood.

**What varies per reader is not the number but which rows carry one.** Both read doors refuse to
enumerate a region in which the caller can see nothing:

- `anchor_region_metrics` (`migrations/20260713000050_region_visible_member_count.sql`) — an `EXISTS`
  over `kb_cogmap_region_members` joined to the visibility CTE.
- `anchor_shape` (`migrations/20260823000010_anchor_shape_envelope.sql`) — `seen.visible_members > 0`.

That gate is the bound, and it is the load-bearing half of the design. A record that says "not
re-filtered per reader" without it overstates the exposure.

## `member_count` is not in this exception — it was the original violation, and it was fixed

`member_count` is now computed per reader. `crates/temper-core/src/types/cognitive_maps.rs`:

> Member count (the blur the surface tier exposes; identities stay interior) — over the members
> **this caller can read**, never all of them (spec §D5). Two readers of the same region can
> legitimately see different numbers; that is the point.

Grouping it with the metrics inverts the fix.

## Why

An aggregate is never an identity. A partial-visibility caller learns that a topic area has weight or
is shifting — not who or what is in it — and an invisible member's contribution is not recoverable
from a scalar blended with the visible ones.

The alternative is per-reader metric recomputation: five SQL aggregates and a 768-dimension centroid,
per region, per reader, on a read path that today serves them from a stored column. The spec priced
that and declined. `temper-artifacts:specs/2026-07-13-unified-visibility-semantics-design.md` §D5
states the invariant and, under *Trade-offs accepted*, the carve-out:

> **Invariant: no returned value is computed over members the caller cannot see.**

`migrations/20260713000050`'s header states the boundary of what it changed:

> **This is not per-caller metric recomputation** — the thing the task explicitly rules out, and
> rightly. The stored metrics still ride through exactly as stored (an aggregate over all members,
> reader-independent, accepted as a bounded disclosure — spec §D5, "Trade-offs accepted"). All that
> changes is WHICH regions are enumerated.

`[provisional — 2026-08-26, judgement call]` The register row attributes this decision to a review on
2026-07-21. The citable record found in the tree is spec §D5 and the migration header above, both
predating that date. The decision is real and recorded; the 2026-07-21 attribution is not something
this pass could corroborate, so it is not repeated as provenance.

## Scope condition

Re-review is forced by any of:

1. **A visibility boundary that actually runs through a region.** The 2026-07-13 differential measured
   **0 of 546 live regions over-counted, and 0 fully invisible**, on production. That number is what
   makes the disclosure theoretical today, and it is the first thing to re-measure.
2. **A metric becoming low-cardinality or invertible.** `reference_standing` averaged 0 on 277 of 278
   context regions; `internal_tension` was **0.0000 on 278 of 278**. A nonzero value on a small region
   is therefore close to a pointer.
3. **An agent or surface acquiring a region door while holding partial visibility.** Today only
   `cogmap_read` / `context_read` on MCP and the graph HTTP door reach these values.
4. **`centroid` becoming reader-observable, or steering a caller's search.** `wayfind_region_scores`
   ranks candidates by a salience recomputed from these stored columns, so its containment rests on
   properties of the surfaces downstream of it rather than on the scoring itself. Any change to how a
   wayfind result is dereferenced, and any new surface that exposes its intermediate output, re-opens
   this question and should be reviewed against this record before it ships.
5. A new aggregate of this shape shipping without a statement at the field.

## How it is enforced, and two gaps that are real

**1. The member gate in SQL is real and enforced.** It proves a region with no visible member is never
enumerated. It proves **nothing** about a region with one visible member out of twelve, and **no test
exercises that case in either direction** — `region_metrics_surface_gate_and_lens`
(`crates/temper-substrate/tests/cogmap_analytics_readback.rs`) seeds a 2-member region with both
members visible.

**2. GAP — the statement-at-the-field is incomplete, and the site the register row named is the wrong
one.** The register row's acceptance criterion asked for a comment at the *materialization* site.
There is none: `crates/temper-substrate/src/write.rs` contains zero references to reader visibility.
But the requirement in `internal/agents/key-patterns.md` is that an exception be *"stated at the
field"* — which means the read surfaces and the wire types, not the producer. The concrete gaps are:

- `anchor_shape`'s `COMMENT ON FUNCTION` — returns `salience` and `content_cohesion`, says nothing
  about them being reader-independent.
- `CogmapRegionRow.salience` and `.content_cohesion` — `member_count` beside them states the rule;
  these two state nothing.
- Every `CogmapRegionMetricsRow` metric field.
- Every `StandingShape` field.

Only `COMMENT ON FUNCTION anchor_region_metrics` carries the statement today.

**3. GAP — `key-patterns.md` states an exception clause these metrics do not satisfy.** The
agent-facing canonical statement admits:

> **One exception, and it is the only kind:** a value may report over rows the caller cannot enumerate
> where the value is about the **anchor** rather than about the rows — and that has to be stated at the
> field, never inferred from the fact that it currently ships.

A region metric is an aggregate **over the rows**, not a fact about the anchor. No region metric
appears anywhere in that file, and its enumerated "filed, not fixed" list names seven other columns
and none of these. **An agent auditing against `key-patterns.md` would classify every one of these as
a violation.** That divergence — not the exception itself — is the live risk.

## The agent rule is not stated anywhere, and the mechanism cited for it has rotted

The register row says the agent rule (*partial-visibility agents do not read stored aggregates*) is
already enforced by the steward's connection allowlist, as a per-agent choice awaiting promotion to a
stated rule.

**The exclusion is real.** `STEWARD_TOOLS` and `AUDITOR_TOOLS`
(`packages/agent-workflows/steward/agent/lib/tool-allowlists.ts`) omit every region door;
`cogmap_read` and `context_read` appear nowhere under `packages/agent-workflows/`.

**But the reason given is not visibility.** The list's own docstring:

> The 9 excluded (region reads + genesis/admin/access) are **role-inappropriate for a steward**.

**And the list is stale.** Twelve of its 24 names are not on the router's live `#[tool]` set —
including `cogmap_read_charter`, which is now a `view` arm inside `cogmap_read` (itself not
allowlisted). Nothing in `crates/`, `.github/`, or `scripts/` cross-checks `STEWARD_TOOLS` against the
router.

So the rule the register row attributes to the codebase is an **inference from a configuration
written for a different reason, which has since rotted and which nothing holds in place.** Citing the
allowlist as evidence of a live posture, without that, would credit a mechanism nobody is maintaining.

## `StandingShape` is the weaker case, not the parallel case

`resource_standing_shape` (`migrations/20260724000120_standing_citation_components.sql`) gates the
finding through `resources_readable_by` and the principal **stops there** — every component producer
takes `(p_finding uuid)` alone. `resource_contradiction_balance` sums express edges by endpoint
incidence with no `edges_visible_to` and no `endpoint_readable_by_profile`, so an edge whose other
endpoint the reader cannot see still moves the number. `resource_live_citations` gates on
`is_active AND ingest_state = 'complete'` — liveness, not readability.

So it has **no member-equivalent gate and no documented exception anywhere.** If one record covers
both, it must say that the region metrics carry a bound and a statement that standing does not.

## Amended 2026-09-03 — the exception's scope now names the standing components

Decided in `temper-artifacts:specs/2026-09-03-evidential-standing-phase-c-visibility-posture-design.md`
(task `01a063ee-a367-7790-b2fa-777e601751e1`). The standing components — `StandingShape`'s six:
`citation_magnitude`, `audit_coverage`, `citation_quality`, `contradiction_balance`, `freshness`,
`r_parent` (the Set-5 model; the Set-3 breadth/survival components this record originally named are
retired) — join the region metrics under this exception.

- **The finding gate plays the member gate's role:** `resource_standing_shape` returns zero rows
  unless the caller reads the finding — the anchor is fully visible or nothing is returned. That is
  stronger than the region bound in the anchor's direction, and there is deliberately no
  contributor-side counterpart: the components are evidential metadata of a claim the caller fully
  sees (the bibliographic case), so the exception's *about the anchor* test holds for them
  directly.
- **Attribution stays behind per-reader checks.** The citation-audit trail is the shipped
  sibling-read pattern; any future drill-down rides it and never the shape. No per-contributor
  identity rides the wire without a per-reader gate.
- **The agent rule covers `StandingShape` from day one** — *partial-visibility agents do not read
  stored aggregates*, stated in `internal/agents/key-patterns.md` beside the standing bullet and
  covering the evidence doors (`temper resource evidence`, `GET /api/resources/{id}/evidence`).
- **The statements land at the fields** — the wire type's docs and the SQL `COMMENT`s state the
  posture; they ship with the implementing PR, per this record's own requirement that an exception
  be stated at the field, never inferred from the fact that it ships.
- **The scope condition above extends to the standing components**, with the standing-specific
  triggers named in the design doc: small-finding invertibility (±1 balance on a single edge),
  any ranking consumer of the shape, and any drill-down surface beyond the trail pattern.

The two classes this record covers now stand symmetrically: region metrics bounded by the member
gate, standing components bounded by the finding gate, both reader-independent by decision and
stated at their fields, with the scope condition governing both.
