# Implementation plan — auditor tier-1 staleness watermark

**Spec:** [`docs/superpowers/specs/2026-07-25-auditor-tier1-staleness-watermark-design.md`](../specs/2026-07-25-auditor-tier1-staleness-watermark-design.md)
**Branch:** `jct/auditor-tier1-staleness-watermark` (off `76d0de12`)
**Size:** one migration, two tests, one cache regen. Deliberately small — read §"Why this document
exists" in the spec before proposing to widen it.

Every symbol named below was grep- or psql-verified on 2026-07-25 against `main @ 76d0de12`. Steps
carry **CONFORM / EXTEND / AMEND** tags per `implementation-grounding.md` GD-3.

---

## Step 1 — Migration `20260726000010_auditor_tier1_staleness.sql`

Numbered above `20260724000220` (the current tail) with a gap left for the sibling F-2 branch. PR
#546 carries no migrations today, so the gap is insurance, not a known collision.

### 1a. New function `resource_has_stale_citation(uuid) → boolean`

**EXTEND** — new affordance, authorized by spec §3 and by D7 (*"Selection is `uncovered OR stale`"*).
Body is given in spec §3; do not re-derive it.

Three grounded constraints, each with the citation the implementer must honor:

- **CONFORM** — must call `resource_live_citations(p_finding)`
  (`migrations/20260724000120_standing_citation_components.sql:103-111`). Its COMMENT calls it *"the
  one definition the three standing axes share."* Any other spelling of "live citation" here is a
  fourth definition and a drift site.
- **CONFORM** — must compare against `max(a.audited_by_event_id)`, not any audit.
  `20260724000110_citation_audits.sql:12-17` states there is *"deliberately NO `is_superseded`
  column"*, so a citation routinely carries several audits and only the newest bounds staleness.
- **CONFORM** — the `w.watermark IS NOT NULL` guard. A never-audited citation is *uncovered*, not
  *stale*; D7's two disjuncts must stay disjoint. Without it the function returns false for
  unaudited citations anyway (`max` over empty is NULL, and NULL comparisons are not true), but the
  guard states the intent and stops a later reader "simplifying" it away.

Add a `COMMENT ON FUNCTION` recording **why no material-event allow-list exists** — spec §2.2: the
`citation_audited` cycle is structurally impossible because `_project_citation_audited`
(`20260724000110:84-99`) writes only `kb_citation_audits` and bumps no cursor this function reads.
That comment is the whole defence against someone re-introducing D3's allow-list later.

### 1b. DROP + CREATE `audit_drift_sweep`

**AMEND** — shipped function, selection semantics change. Authorized by spec §3.

- **CONFORM** — DROP+CREATE in a *new* migration, never an edit to `20260724000130`. That file sets
  the precedent itself (it DROP+CREATEs `workflow_job_claim`), and the repo rule is that applied
  migrations are immutable.
- **CONFORM** — signature unchanged (`p_principal uuid, p_limit int`), so no deploy-skew concern.
  Nothing is appended, so the positional call site in
  `crates/temper-services/src/services/auditor_service.rs:82` keeps resolving.
- **CONFORM** — compute staleness **once per candidate, inside the existing `scored` CTE**, beside
  `resource_citation_magnitude` / `resource_audit_coverage`. `20260724000130`'s own header:
  *"EACH PRODUCER RUNS ONCE PER CANDIDATE ROW ... repeating them across the WHERE clause and the
  SELECT list would run each producer up to four times per row."* Adding a fourth producer that
  violates its own file's rule would be a self-inflicted regression.
- **CONFORM** — carry the entire existing body forward unchanged: every filter, the cogmap-home
  join, `steward_candidate_cogmaps`, `resources_visible_to`, and the `ORDER BY uncovered DESC,
  s.finding_id` tie-breaker (that file argues at length why `finding_id` is load-bearing for
  determinism). **The only edit is the `WHERE` disjunct.**
- **AMEND** — `WHERE s.magnitude > 0 AND (s.coverage < s.magnitude OR s.stale)`.

**Open sub-decision for the implementer to settle and record:** `uncovered` is currently
`magnitude - coverage`, which is `0` for a fully-covered-but-stale finding. Ordering is
`uncovered DESC`, so stale findings sort to the **tail** and a small `p_limit` may starve them
entirely. Either (a) accept and document it, or (b) order stale findings by a separate key. Do not
silently ship (a) — the sweep's own comments say a cap that changes *which* rows are enqueued, not
just their order, is a correctness issue, not a cosmetic one.

### 1c. Preserve the KNOWN-FIRST-CUT-LIMITATION comment, amended

`20260724000130:70-82` currently documents the "re-heads forever" limitation. Spec §5 reclassifies it
as a **persona obligation**, not a schema gap. Update the comment rather than deleting it; note that
R10 was withdrawn and point at the separately-filed source-visibility defect (`019f9bfb`).

---

## Step 2 — Tests in `crates/temper-substrate/tests/citation_audits.rs`

**CONFORM** — extend the existing Task-5 sweep family beginning at `:1174`. Do **not** start a
parallel suite; that header explains the file's fixture conventions and the reason assertions check
presence/absence of a specific `(cogmap_id, finding_id)` pair rather than row counts (the L0 kernel
cogmap is seeded into every test DB).

File is `#![cfg(feature = "artifact-tests")]` and uses
`#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]`. Run with **`cargo make test-artifacts`** —
`cargo make check`, `test-db`, and `test-e2e` all skip this tier.

### Verified fixture primitives (do not invent new ones)

| helper | location | note |
|---|---|---|
| `sweep(pool, principal, limit)` | `:1321` | returns raw `(cogmap_id, finding_id, uncovered)` |
| `make_cogmap_finding(...)` | Task-5 section | cogmap-homed finding |
| `seed_cogmap_finding_with_n_citations(...)` | Task-5 section | multi-citation fixture |
| `join_principal_to_cogmap(...)` | Task-5 section | reach |
| `fire_audit(pool, emitter, block, source, value)` | `:100` | `SELECT citation_audit($1,$2)` |
| `first_block(pool, resource)` | `:88` | |
| `common::genesis_cogmap`, `common::create_profile` | `tests/common` | |

⚠️ **`cite()` (`:130`) does NOT bump `last_event_id`.** It is a raw
`INSERT INTO kb_block_provenance` reusing `b.genesis_event_id`. That is *why* the stay-green test
below works, and it is also why `cite()` **cannot** be used as the material change in tests 1–2.

### The material change — use the typed production path

Verified working shape at `crates/temper-substrate/tests/readout_tier.rs:70-90`:

```rust
let prepared = content::prepare_block(0, None, "…new text…").unwrap();
let mut tx = pool.begin().await.unwrap();
fire(&mut tx, SeedAction::BlockMutate {
    incorporated: &[],
    block: BlockId::from(block_id),
    chunks: &prepared.chunks,
    raw: None,
    emitter: EntityId::from(emitter),
}).await.unwrap();
tx.commit().await.unwrap();
```

Do **not** call `block_mutate` via raw SQL with an empty `chunks` array — `block_mutate` raises on an
empty chunk set by design (`20260624000002_canonical_functions.sql:970`), and
`content_mutation.rs:72` exists to pin that.

### The three tests

1. **`sweep_reoffers_a_covered_finding_after_its_block_is_mutated`** — the primary witness.
   Fully covered (`magnitude == coverage`, so absent from today's sweep), then `BlockMutate` on the
   citing block, then sweep. **Must fail before the change** — today's predicate has no notion of a
   cursor, so the finding never returns. This failure *is* the witness working.
2. **`sweep_reoffers_a_covered_finding_after_its_cited_source_changes`** — `BlockMutate` on a block
   of the **source** resource, not the finding. Exercises the second disjunct, which would pass
   vacuously if only clause one were implemented.
3. **`sweep_omits_a_covered_finding_with_no_material_change`** — must **stay green**.
   `sweep_omits_a_fully_covered_finding` (`:1374`) already covers this; add an explicit
   audit-then-sweep-twice variant only if it does not. A staleness predicate that fires on a quiet
   finding is worse than the defect it replaces.

**Run tests 1 and 2 against the pre-change migration first and record that they fail.** A witness
never observed failing discriminates nothing — this is the exact bar W1 was cancelled for missing.

---

## Step 3 — sqlx cache

`audit_drift_sweep` is called through `sqlx::query!`-family macros in
`crates/temper-services/src/services/auditor_service.rs:82`. Changing the SQL function does not
change that call's *shape* (same signature, same return columns), so the workspace cache may not
move — but **verify rather than assume**. Read the `sqlx-query-cache` skill; regenerate with
`cargo sqlx prepare --workspace -- --all-features` and commit only genuinely-changed `.sqlx` entries.

---

## Step 4 — Gates

```bash
cargo make test-artifacts     # the tier these tests live in; NOT covered by check/test-db/e2e
cargo make check              # fmt + clippy + machete + generated-artifact drift
```

Pre-commit runs incremental clippy, which has repeatedly gone green where a clean CI build goes red —
do not treat a green hook as a green CI.

---

## Explicitly out of scope

Per spec §4 and §7, and not to be widened without a decision: the material-event allow-list (D3),
the citation-list payload (D6), tier 2 and C-7 (D2), `resource_updated` metadata staleness, the
corroboration axis, and the source-visibility defect (`019f9bfb`, its own task).
