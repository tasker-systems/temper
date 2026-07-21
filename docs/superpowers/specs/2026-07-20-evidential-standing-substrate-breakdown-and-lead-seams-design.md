# Evidential standing as substrate — work-breakdown + lead-seam design

*Design spec. Operationalizes the discovery-session research
[`019f7665`](mcp://agent/3621dc70-f8ae-464f-8948-d9fe893ef5ad) ("Evidential
standing as substrate: not an agent chat forum but a defensibility-of-claims
machine"), which explicitly deferred operationalization to "a write-owning
session." This is that session.*

*Scope of this spec: (a) the six-set work-breakdown with dependency/gate
structure and readiness assessment; (b) the resolved design of the **two lead
seams** — #2 the independence-assessment and #1 the projection shape — which
together gate Sets 3/4/5. The remaining sets are scoped here but each earns its
own spec→plan→implementation cycle downstream.*

*Grounded against `migrations/` and `packages/agent-workflows/steward/` on
2026-07-20. Every "maps onto existing substrate" claim below carries a quoted
`file:line` citation; decisions are tagged CONFORM / EXTEND / AMEND against
that grounding.*

---

## Bedrock preamble — standing is not truth

> **Standing is not truth, and the system cannot close the gap between them —
> only make its shape visible.**

This is the load-bearing commitment the whole design serves, folded in from the
research's seam #4. The maturity axis does **not** measure "is this claim true."
It measures how *defensible on the present evidence* a claim is — a fact about
the structure of emitted evidence and its relations, never about the world.

- A lone agent with one datum may be flatly right, and the system will correctly
  show that claim as **low-standing, with no contradiction**, because low-standing
  was never a claim about truth. Truth and standing are orthogonal; the system
  speaks only to the second.
- The **August Landmesser** proof-case: the saluting crowd is high-standing-and-wrong
  (monocultural echo — one generating cause, zero evidential diversity); the single
  man refusing is low-standing-and-right. A system that reads standing *as* truth
  does not merely miss him — it *actively renders his correctness as low-status*
  with total faithfulness to the evidence. The only defense is never having claimed
  to measure truth.
- This sits in the same tier as "knowledge bases are misnomers" and "the
  translation problem is irreducible." It appears to be the *same* commitment as
  the translation problem, pointed at the evidence axis instead of the perspective
  axis.

Every design decision below is answerable to this boundary. Where a decision
would let standing quietly stand in for truth, that decision is wrong.

---

## The reframe (one paragraph)

We are not building an agent chat forum. We are building the substrate for the
**defensibility of claims** — machinery that computes, faithfully and without
pretending to truth, how well-stood-up any fact/concept/commitment/finding is on
the evidence present. The "chat" was only ever the accumulation surface where raw
observations land; the *product* is the maturity/standing projection over that
surface, plus the jobs-to-be-done and personas that turn a pile of emitted
observations into something a human or a tracker can act on with the evidentiary
case in hand. The load-bearing correction to the original instinct: **corroboration
is a property of the evidence, not the assertion** — breadth-and-depth of the
evidentiary basis a claim subtends, weighted by assessed independence,
adversarially discounted — *not* a headcount of agreeing actors. (A monoculture
agrees with itself; N echoes are not N confirmations.)

---

## The six-set work-breakdown

```
                    ┌─────────────────────────────────────────────┐
                    │  GATE ── Set 1: lead-seam design (THIS SPEC)  │
                    │  #2 independence-assessment (load-bearing)    │
                    │  #1 projection shape                          │
                    │  + "standing ≠ truth" bedrock (seam #4 folded)│
                    └───────────────┬─────────────────────────────┘
                                    │ unblocks
          ┌─────────────────────────┼─────────────────────────┐
          ▼                         ▼                          ▼
   Set 3: Maturity          Set 4: Steward's           Set 5: Adversary persona
   projection               three jobs                 (challenge-agent; emits
   (build; well-posed       (grow exists; tend         adversarial-survival events
   after this spec)         gated on Set 3; reap        the projection consumes)
                            mechanics partly ready)
                                                              │
   ── parallel, seam-independent ──                          ▼
   Set 2: Findings substrate / homing              Set 6: Promotion as translation
   (board anchor + team-join + scope exclusion;    (gated redacting bridge board→durable;
    BUILD-READY, large blast radius)               reads standing incl. adversarial-survival
                                                    as gate; human-in-loop; capstone)
```

| Set | What | Kind | Readiness | Gated by |
|-----|------|------|-----------|----------|
| **1 — Lead-seam design** (#1, #2, +#4 folded) | Resolve the independence-assessment and the projection shape; state the standing≠truth boundary | design → **this spec** | **DONE (this doc)** | — (the gate) |
| **2 — Findings substrate** | Findings/channel-msgs as `kb_resources` homed in a new board anchor; `kb_team_finding_boards` sibling join; scope-exclusion from default human search; topics via `kb_topics` | build | **BUILD-READY**, large blast radius (see readiness note) | seam-independent |
| **3 — Maturity projection** | Materialized standing over provenance + edges; component-memo + read-time band/shape; reuses the salience pattern | build | **WELL-POSED** after this spec | Set 1 ✅ |
| **4 — Steward three jobs** | grow→tend→reap *within one tick*; reinforcement act-type; fade/scar/retire; preserve dispatch/watermark/resume | build | grow exists; tend gated on Set 3; reap mechanics exist | Set 3 (tend) |
| **5 — Adversary persona** | A challenge-agent with its own reliability profile and a duty-to-challenge-before-promote; emits the adversarial-challenge / survived events Set 3 consumes | build (agent) | contract defined by this spec; agent unbuilt | Sets 1 ✅, 3 |
| **6 — Promotion as translation** | Gated redacting bridge board→durable (cogmap node / issue); reads standing (incl. adversarial-survival) as gate; generalizes/redacts on crossing; human-in-loop default | build (capstone) | most-gated | Sets 2, 3, 4, 5 |

**The gate insight:** Set 1 was the true gate. Sets 3, 4-tend, and 5 could not be
honestly tasked until the independence-assessment and projection shape resolved —
which this spec does. Sets 0 (boundary, folded here) and 2 (findings board) are the
two things that never needed the gate; the board is pure accumulation surface and
needs no maturity to exist.

**Adversary vs. promotion — the deliberate split** (per review): the adversary is an
**actor-persona** — its own agent, jobs-to-be-done shaped like the steward's but
challenge-substanced. Promotion-as-translation is a **system operation** that *reads*
the reinforcement- and adversarial-survival-grounding as **evidentiary inputs**, but
is not itself the adversary acting. Promotion consumes the adversary's output; it is
not in the adversary's remit. They are Sets 5 and 6, not one set.

### Readiness note — the findings-board blast radius (grounded correction)

The research called adding a board anchor "a one-line predicate" with "remarkably
little new DDL." Grounding contradicts the *size*, not the direction:

- The anchor enumeration `('kb_cogmaps','kb_contexts')` is copied across **7+ read-gate
  functions** — e.g. `migrations/20260708000007_visibility_is_active_gate.sql:90`,
  `20260630000002_access_grants_read_wiring.sql:118`, `20260701000002_cogmap_read_up_flip.sql:130`,
  `20260703000002_team_graph_scope_reads.sql:98`, `20260703000001_team_metadata_soft_delete.sql:130`,
  `20260709000005_backfill_goal_parent_of_to_advances.sql:59` — **and** a *parallel*
  `home_anchor_table` CHECK on regions: `20260712000030_region_anchor_expand.sql:17,22,29`.
- Adding `kb_finding_boards` therefore means a **DROP+CREATE additive migration touching
  every one of those** (shipped migrations are immutable). Missing one is either a
  **visibility leak** (a board reachable where it shouldn't be) or a **dead board** (a
  board no read-gate admits). This is a careful sweep, not a one-liner.

Direction still holds: RBAC comes for free via a `kb_team_finding_boards` sibling of
the existing `kb_team_cogmaps`/`kb_team_contexts`
(`migrations/20260624000001_canonical_schema.sql:254,267`), and scope-exclusion is a
predicate in the scope-aware search functions
(`20260711000050_search_vector_scope_aware.sql`, `20260629000004_search_scope_ids.sql`).
Set 2's own spec must carry the full read-gate inventory as its checklist.

---

## Seam #2 — the independence-assessment (RESOLVED)

The load-bearing seam. Corroboration was redefined from *actor-count* to *evidential
breadth weighted by assessed independence* — which rescues the M2M steward (one
actor, many ticks, still corroborates) but **relocates** the hard judgment rather
than dissolving it: "these two bases are independent" is itself an emitted, fallible,
scarrable claim, and a monoculture assessor mis-judges independence in a consistent
direction. Four sub-decisions ground that recursion out.

### 2.1 Two-count split — terminating, no recursion — **CONFORM + EXTEND**

There are two distinct reinforcement counts on two distinct claims:

- **R_indep** — reinforcement of the *independence claim itself* ("these two bases are
  independent"): how many times re-affirmed (or scarred).
- **R_parent** — reinforcement of the *finding* whose standing we score: how much
  evidence has accreted. This is the breadth/decay term itself.

The maturity projection reads independence at **face value weighted by R_indep** — a
low-standing independence assertion contributes less — **but the recursion is cut at
depth 1**: R_indep is a *leaf tally*, cheap to read, **not** a recursive standing
computation. This is the disciplined, terminating slice of "one bounded recursive
descent" without opening the descent.

```
independence_weight = f( independence_estimate_value , R_indep )
breadth_term        = g( evidentiary bases , independence_weight )   -- weighted, not counted
maturity            = h( breadth_term , R_parent-decay , adversarial-survival , contradiction )
```

- CONFORM — R_parent/R_indep accretion rides on the existing `kb_block_provenance`
  accretion model: `accretion_seq` monotonic per `(block_id, source_kind, source_id)`,
  `is_corrected` as the scar bit (`migrations/20260624000001_canonical_schema.sql`,
  the `kb_block_provenance` DDL; write path `20260704000003_block_provenance_write_path.sql:25-26`).
- EXTEND — the *distinction* between R_indep and R_parent, and reinforcement-as-a-
  standing-proxy, is new (authorized by research §"maturity as a materialized
  projection" and seam #2).

**Scarrability falls out at no cost:** because maturity is a *materialized projection,
never a stored band*, a later source proving two bases were *not* independent scars the
independence claim → R_indep drops → the parent's breadth silently re-lowers on the
**next materialization pass**. "Maturity must not be stored" and "independence must be
scarrable" are the same constraint.

> **Grounding correction (2026-07-21, Set 3 plan):** the scar bit `is_corrected` on
> `kb_block_provenance` exists (`DEFAULT false`, read-filtered everywhere) but has **no writer
> anywhere in the codebase today** — full-repo grep. So scar *propagation* is real (Set 3's
> projection reads `WHERE NOT is_corrected`, exactly as the incumbent `resource_blocks.reinforce_count`
> / `cogmap_region_reference_standing` already do), but scar *emission* is unbuilt. Set 3 reads the
> filter; the writer is deferred to **Set 5** (the adversary) and a future scar path. The mechanism
> is latent-but-correct, not zero-cost-and-live.

### 2.2 Representation — one pairwise claim is a labeled edge — **CONFORM**

A single pairwise independence claim = an **`express` edge**, `label='independent-of'`
(or `shares-cause-with` with `polarity='inverse'` — the `contradicts` precedent
exactly), `weight` carrying the estimate, between two evidentiary bases.

- CONFORM — `edge_kind` is a **closed** ENUM (`express`/`contains`/`leads_to`/`near`),
  semantic types ride on the free-text `label`; `contradicts` is itself a label on an
  `express` edge (`migrations/20260624000001_canonical_schema.sql:87,95,634-635`;
  `20260624000002_canonical_functions.sql:506` default label set `{'contradicts'}`).
- CONFORM — scarrable for free via fold + edge-level correction (`fold_relationship`,
  edge `is_corrected`).
- ~~CONFORM — the `AnchorTable` enum **includes `kb_edges` itself**
  (`20260624000003_canonical_seed.sql:35`, `AnchorTable` enum) — so an independence
  edge can be an *endpoint* of another edge. That is the **bounded scar-recursion made
  physical**: a challenge to an independence claim is a meta-edge onto it. No new
  machinery.~~

  > **Grounding correction (2026-07-21, Set 3 plan) — this claim is FALSIFIED.** `AnchorTable`
  > includes `kb_edges` only in the **wire/JSON-Schema** layer. The `kb_edges` **table** CHECK
  > restricts `source_table`/`target_table` to `IN ('kb_resources','kb_cogmaps')`
  > (`20260624000001_canonical_schema.sql:630,632`), so an edge-onto-an-edge is **not persistable**
  > today. Meta-edges are wire-representable but not storable without a CHECK-widening DROP+CREATE
  > (its own read-gate blast radius). "No new machinery" is wrong. **Resolution:** Set 3 does **not**
  > build meta-edges. It reads an independence claim's **live edge state** (`weight`, `is_folded`)
  > as the scar signal; representing a *challenge* to an independence claim — meta-edge vs
  > fold-then-recreate (`relationship_fold` + `shares-cause-with` re-assert) — is **Set 5's** spec
  > to settle, not Set 3's. (Note also: edges scar via `is_folded`, not `is_corrected`; and
  > `fold_relationship` is actually `relationship_fold`.)

### 2.3 Arity — pairwise, sparse-by-assertion, memoized — **CONFORM + EXTEND**

Independence is a property of a *set* of N bases (N-choose-2 pairs). Emission is
**pairwise**, but the graph is **sparse-by-assertion**, for a load-bearing reason:

- **Independence is not transitive.** A⊥B and B⊥C does *not* give A⊥C — a shared latent
  cause breaks transitivity (the entire monoculture point). So the closure **cannot** be
  inferred; only pairs the assessor had an actual basis to judge ever get an edge. For a
  real evidence set that is a handful, not N².
- **Read cost is solved by a materialized projection**, not by cheaper emission: a
  flattened **`independence_pairs`** memo (`finding_id, base_a, base_b, weight,
  is_scarred`) refreshed on edge-change drift — structurally the region-salience pattern.
  A finding's breadth read then scans *its* pairs, O(pairs-for-this-finding), no graph
  traversal.
- CONFORM — the memoize-a-projection / refresh-on-drift pattern:
  `kb_cogmap_regions.salience` (`20260712000030_region_anchor_expand.sql:73` drift-refresh;
  `20260629000007_wayfind_scope.sql:20,40` memoized-vs-recompute).

  > **Grounding correction (2026-07-21, Set 3 plan) — mechanism was misstated.** "Refreshed on
  > edge-change drift" is right in *spirit* but the repo has **zero** recompute-on-write DB triggers
  > and **zero** materialized views. The actual house style is: memoized value + its input
  > components stored as **plain columns**, refreshed by an **application-orchestrated Rust "clock"**
  > (`crates/temper-services/src/backend/region_clocks.rs`) fired inline on the write path and gated
  > by a **SQL drift function** (`anchor_telos_drift`, `20260712000070_two_clocks.sql:147`) vs a
  > snapshot column, with the band/shape **recomputed at read** from the stored components
  > (`wayfind_scope.sql:41-49`). Set 3's `independence_pairs` + component-memo therefore follow the
  > **columns + Rust-clock + read-time recompute** template, **not** a Postgres trigger. (The cited
  > `region_anchor_expand.sql:73` is a *comment* describing the future clock, not a trigger.)
- EXTEND — the `independence_pairs` memo table and its refresh trigger are new (its own
  DDL in Set 3).

### 2.4 Silence default — an unasserted pair is correlated — **EXTEND**

An unasserted pair (no independence edge either way) is treated as **NOT independent
(assumed to share a cause) until an explicit `independent-of` edge proves otherwise.**
Breadth rises only when independence is *affirmatively established*. Silence is not
evidence of independence.

- EXTEND — authorized directly by the research thesis: "a monoculture agrees with
  itself"; actor-multiplicity contributes ~0 unless the actors are genuinely
  differently-situated. Conservative silence is the only default consistent with
  standing≠truth — the alternative (silence = independent) *is* the actor-count fallacy
  the whole design rejects.

---

## Seam #1 — the projection shape (RESOLVED)

After seam #2, the research sketch `maturity ≈ f(breadth, independence,
adversarial-survival, contradiction, decay)` tightens:

- **breadth ⊇ independence** — one term, "independence-discounted breadth" = the
  effective independent rank of the evidence set, read from `independence_pairs`
  under the conservative silence default. Not two multiplicative terms.
- **contradiction** — `contradicts` edges (inverse polarity), **vector-sum** framing:
  5 supports + 4 contradicts is a live `concern`, not near-canonical (productive
  disagreement vs. unresolved).
- **decay** — reversible fade, clocked off R_parent recency.
- **adversarial-survival** — must distinguish **0 challenges** from **N withstood**
  (absence of challenge is not survival). Emitted by Set 5 (the adversary), consumed
  here. A survived challenge is physical: a pairwise independence/claim edge that was
  attacked (meta-edge, §2.2) and **not** scarred.

### 1.1 Standing is shape-primary; the band is a lossy chip — **EXTEND**

Standing **IS the vector** — `(independence-discounted-breadth, adversarial-survival,
contradiction-balance, freshness)`. Any band label
(`provisional`/`reinforced`/`near-canonical`) is a **lossy read-time summary** over
that shape, always presented *with* the shape, **never instead of it.**

Rationale (load-bearing): a scalar-default washes out what the design preserves *in
practice* even if it nominally keeps the shape — consumers sort/gate on the number and
the shape goes vestigial. Shape-primary is the **only** version where "diverse-high and
echo-high are different projections" survives contact: the lone-right man and the
monocultural crowd may carry the *same band chip* but visibly *different shapes*
(high-breadth-high-independence vs. high-breadth-zero-independence). This is the
Landmesser boundary made computable.

- EXTEND — authorized by research §"the named bedrock boundary" ("diverse-evidence-high
  vs. echo-high must be *different projections*") and the standing≠truth preamble.
- Consequence to accept: every consumer handles a vector, not a scalar; "is this
  near-canonical" has no single true answer, only a shape plus a chosen summary. This
  cost is accepted deliberately.

### 1.2 Two orthogonal axes — never collapse — **CONFORM**

- **Maturity / corroboration:** provisional → reinforced → near-canonical (accretion of
  independent, adversarially-survived evidence).
- **Validity / health:** live → scarred → retired (is it still true; if it was wrong,
  keep the scar).

These must stay orthogonal — collapsing them breaks on the near-canonical-then-scarred
case (a catastrophic scar the semantic model flags as resisting decay). Settled by the
research as a hard requirement, restated here as CONFORM.

### 1.3 Materialization home — component-memo, read-time band — **CONFORM + AMEND**

- AMEND — do **not** add a `maturity` enum/band to `kb_resources`. (The tempting store;
  the research forbids it because a stored band reintroduces the cache-vs-truth bug the
  events-as-primary bedrock refuses.)
- CONFORM — memoize the **component scalars** in a projection memo and compute the
  **band/shape at read**, exactly the wayfind precedent: `NULL lens → memoized salience;
  non-null lens → recompute` (`20260629000007_wayfind_scope.sql:20`). The band is a
  read-time function over memoized components, never a written column.

---

## What this spec deliberately does NOT design (downstream)

Each is scoped by the breakdown and earns its own spec→plan→implementation:

- **Set 2** — the full read-gate inventory + DROP/CREATE sweep for the `kb_finding_boards`
  anchor; `kb_team_finding_boards`; scope-exclusion predicate; topic wiring via `kb_topics`.
- **Set 3** — the `independence_pairs` and maturity component-memo DDL, refresh clocks (Rust,
  not triggers — see the 2026-07-21 correction under §2.3), the exact band **thresholds** and the
  shape presentation (a tuning + surfacing pass). **Now planned:**
  [`docs/superpowers/plans/2026-07-21-set3-maturity-projection.md`](plans/2026-07-21-set3-maturity-projection.md).
- **Set 5** — the adversary persona's jobs-to-be-done, its spatial reliability profile, and
  the concrete adversarial-challenge / survived event vocabulary the projection reads.
- **Set 6** — the promotion gate (what standing shape licenses promotion), the redaction/
  generalization on crossing, and the human-in-the-loop default.

---

## Faithfulness checks

- **Events-as-primary / derivable-not-denormalized:** maturity is a memoized projection
  over components, never a stored band (§1.3); `independence_pairs` is a refreshable memo
  over edge events (§2.3).
- **Scarrification:** validity axis (§1.2) + independence-claim scars propagating via
  recomputation (§2.1) + edge-level correction (§2.2).
- **No canonical view / no view from nowhere:** standing≠truth (preamble) is the same
  commitment on the evidence axis; shape-primary (§1.1) is what keeps the maverick legible.
- **Attention-as-anchor:** promotion-gating (Set 6) is anti-spam by construction (#460's
  point).
- **Independence relocated, not dissolved:** §2 holds the irreducibility honestly — the
  assessor's own blind spots survive (shared trained priors → shared blind spots), which is
  why the independence judgment is itself scarrable and situated rather than authoritative.
