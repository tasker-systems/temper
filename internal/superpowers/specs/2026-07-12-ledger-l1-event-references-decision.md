# Ledger L1 — The Fate of `kb_events.references` — Decision

**Date:** 2026-07-12
**Status:** Decided — **DELETE**. Rust scaffolding removed in this change; the schema drop is a
cutover-governed follow-up (see §5).
**Scope:** Answer the gating question for the "ledger as a readable surface" goal (L1–L5): does any
lineage need to live *on the event*, or does the recorded lineage already live on `derived_from`
edges? This settles which graph L2's reader and L3's cascade traverse.

---

## 1. The question

`kb_events."references"` was designed to carry an event-level lineage graph — a per-write list of
`{rel, target}` statements (`supersedes` / `derived_from` / `touches` pointing at an event,
resource, or block). The gating question for the whole goal:

> **Is there lineage that must live on the event and cannot be an edge?**

If yes, L2/L3 traverse the reference graph and the write path that populates it must be built. If
no, the column and its typed scaffolding are speculative dead weight, and L2/L3 traverse
`derived_from` **edges**.

## 2. Evidence

Two provenances, both first-hand:

- **Source-verified (2026-07-12, this change)** — checked against the tree at `main` (`84002d2a`).
- **Prod-verified (2026-07-12, this change)** — re-queried live against the temperkb.io prod Neon
  (`crimson-fog-23541670`, read-only) this session, superseding the counts the L1 task recorded on
  2026-07-11. The ledger has grown since; every figure below is the fresh reading.

**The column exists and is fully wired — as scaffolding.**

| Artifact | Location | State |
|---|---|---|
| Column `kb_events."references"` (JSONB, default `'[]'`) | `migrations/20260624000001_canonical_schema.sql:477` | defined |
| GIN index `idx_kb_events_references` (`jsonb_path_ops`) | `migrations/20260624000001_canonical_schema.sql:493` | defined |
| `_event_append` param `p_references jsonb DEFAULT '[]'` | `migrations/20260624000002_canonical_functions.sql:768` | defined |
| Rust payload `RefRel` / `RefTarget` / `EventReference` | `crates/temper-substrate/src/payloads.rs:233–258` | defined |

**Nothing writes it.** `_event_append` is the one event writer; every mutation function appends
through it. Across all of its call sites (in `20260624000002_canonical_functions.sql` and every
later `CREATE OR REPLACE` rewrite in the correlation-passthrough / provenance migrations), **not one
passes `p_references`** — each jumps from the positional `p_payload` straight to named args
(`p_metadata =>`, `p_invocation =>`, `p_correlation =>`), leaving `p_references` at its `'[]'`
default. There is no `p_references =>` anywhere in `migrations/`. Prod bears it out: **10,148 events,
every one with `references = []`** (0 references-bearing rows, live query 2026-07-12). The column and
its GIN index physically exist in prod and are entirely empty of signal.

**Nothing reads it, and nothing in Rust even constructs it.** The full footprint of `RefRel` /
`RefTarget` / `EventReference` in the repository is the three type definitions plus **one
serialization roundtrip test** (`payloads.rs:756-765`, `references_serialize_tagged`). They are not
re-exported from any `lib.rs`, not referenced cross-crate, not in the TypeScript bindings, and not
reachable from the `scenario-schema` snapshot or the `payload_schema` test. They are inert.

**The lineage this column was meant to carry already exists as edges.** `derived_from` is a
first-class `EdgeType` (`crates/temper-workflow/src/types/graph.rs:36`, projected as `(LeadsTo,
Inverse, "derived_from")`), asserted by the CLI's `--sources-as-edges` — one `derived_from` edge per
resource-valued source (`crates/temper-cli/src/commands/resource.rs:494`). Prod holds **460
`derived_from`-labelled edges** (live query 2026-07-12), in **two shapes** under the one label:
`(edge_kind=express, forward)` — 310, and `(edge_kind=leads_to, inverse)` — 150 (the
`--sources-as-edges` output). The lineage graph is real, populated, and growing.

> **For L2:** the reader must key on the **label** `derived_from`, not on `edge_kind` — the same
> semantic relation is projected under two `edge_kind`s. Keying on `edge_kind='leads_to'` alone
> silently drops the 310 `express` edges.

So GitHub #380's framing — "a complete, queryable lineage graph… written faithfully, read by
nothing" — was wrong twice over: it is **not written** (every event is `[]`), and the lineage that
*is* recorded lives **on edges**, not on the event.

## 3. The case for keeping, tested

The task named three candidate arguments for keeping event-level references. Each was to be tested,
not assumed. All three collapse.

**(a) `touches` has no natural edge — a claim the write *brushed* a row, not that two resources are
related.** True as a distinction, but there is no consumer that needs it. `RefRel::Touches` is never
constructed and never read; no reader, projector, or surface asks "what did this write touch." A
capability with no claimant is not a reason to keep an indexed column and a typed payload alive — it
is the definition of speculative scaffolding. If a real `touches` need appears later, it is a small
additive migration then, against a clean slate.

**(b) An edge is foldable/re-assertable; a reference on an immutable event is a permanent record of
what the write *believed* at the time.** The immutable record already exists — through a different
mechanism. Edge assertions are themselves events (`relationship_asserted`, `relationship_folded`),
so "what was believed at write time, permanently" is already captured as the edge's own
append-only assertion event in the ledger. The distinction is real but **not load-bearing**: nothing
needs the belief recorded *on the mutating event* specifically, and the append-only guarantee it
would provide is already provided by the edge-assertion event.

**(c) Cross-home lineage: an event reference has no home anchor, so it could escape the home-gating
edges are subject to.** The task flagged the trap itself — *"if yes, that is a leak, not a
feature."* It is. Edges are access-gated by their home anchor; a reference with no home would let
lineage cross the gate. That is an argument **against** the reference graph as a lineage mechanism,
not for it. L2's reader is explicitly access-gated; an un-homed reference graph would be a hole in
exactly that gate.

## 4. Decision — DELETE

Retire event-level references as speculative scaffolding. There is no lineage that must live on the
event: the one candidate with no edge form (`touches`) has no claimant, the immutability argument is
already satisfied by edge-assertion events, and the un-homed reach is a leak rather than a feature.

**L2 and L3 traverse `derived_from` edges.** L2's lineage reader walks the edge graph
(forward/reverse, access-gated by home anchor). L3's fold/supersede cascade walks direct
`derived_from` dependents. Neither depends on `kb_events.references`.

## 5. What "delete" means — split by governance

The cleanup crosses the additive-only-on-`main` boundary (DEPLOYING.md §"The invariant that keeps
auto-deploy safe"), so it lands in two parts with different governance. There is **zero** precedent
for `DROP COLUMN` / `DROP INDEX` in `migrations/` — the steady state is strictly additive — so the
destructive half is not an ordinary merge.

**Part A — Rust scaffolding (additive-safe, landed in this change).** Remove `RefRel`, `RefTarget`,
`EventReference`, and the `references_serialize_tagged` test from `payloads.rs`. Pure code with zero
consumers; removing it cannot affect any running instance (the DB column keeps defaulting to `[]`
regardless of whether the Rust types exist). Safe to merge to `main` immediately.

**Part B — schema drop (destructive; operator-gated cutover, NOT a `main` auto-deploy).** Dropping
the column `kb_events."references"`, the index `idx_kb_events_references`, and the `p_references`
parameter from `_event_append` (a function-signature change — a `DROP FUNCTION` + recreate, not a
`CREATE OR REPLACE`) is a big-bang change per DEPLOYING.md §"Applying schema changes per target." It
follows **back up → migrate → verify → deploy**, operator-run against each target. It is safe to
sequence any time after Part A — the column is write-dead, so no data is lost and no reader breaks —
but it must **not** land as a silent additive migration. It is best folded into the earliest
cutover this goal already requires (L3's cascade / the L4 as-of work), rather than paying a
standalone cutover for a dead column. Until then, an inert indexed column survives; that is
acceptable and explicitly bounded — the goal's "no dangling indexed column" acceptance is satisfied
when Part B lands in that cutover, not before.

## 6. Acceptance

- [x] The question is answered in writing, with the evidence, not deferred — **delete** (§4).
- [x] L2/L3 know which graph they traverse — **`derived_from` edges** (§4).
- [x] Part A (Rust scaffolding) removed in this change.
- [ ] Part B (column + index + `p_references` param) — carried as a cutover-governed follow-up
      (§5), to fold into the earliest schema cutover this goal requires. Tracked so the "no
      dangling indexed column that nothing writes survives this goal" criterion lands there.
