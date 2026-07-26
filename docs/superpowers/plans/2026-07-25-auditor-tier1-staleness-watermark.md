# Implementation plan — auditor tier-1 staleness watermark

**Spec:** [`docs/superpowers/specs/2026-07-25-auditor-tier1-staleness-watermark-design.md`](../specs/2026-07-25-auditor-tier1-staleness-watermark-design.md)
**Branch:** `jct/auditor-tier1-staleness-watermark` (off `76d0de12`)
**Rev.** 2026-07-26, after three adversarial review passes. Session note `019f9ebc-6959-7230-8bdf-bbdec1cbbdf6`.

> **Read the spec's revision note first.** The first version of this plan told the implementer *"body
> is given in spec §3; do not re-derive it"* — and that body contained `max(uuid)`, which does not
> exist, so the migration would have failed at `sqlx migrate run`. The body in spec §3.1 has since
> been **executed against the live database**. Everything else in this plan is grep- or
> psql-verified. Steps carry **CONFORM / EXTEND / AMEND** tags per `implementation-grounding.md` GD-3.

---

## Step 1 — Migration `20260726000010_auditor_tier1_staleness.sql`

Numbered above `20260724000220` with a gap for the sibling F-2 branch (PR #546 carries no migrations
today, so this is insurance).

### 1a. New index — do this first, it is what makes 1b viable

**EXTEND**, additive, so the additive-only-on-`main` invariant holds.

```sql
CREATE INDEX idx_kb_content_blocks_resource_cursor
    ON kb_content_blocks (resource_id, last_event_id);
```

Both existing `resource_id` indexes are **partial on `NOT is_folded`**, so the source-side clause can
use neither — verified with `enable_seqscan = off`, which still produced a `Seq Scan`. With this
index the same probe becomes an `Index Only Scan`. **CONFORM** to spec §3.3, which also explains why
adding `NOT sb.is_folded` is a correctness regression rather than the fix.

### 1b. New function `resource_has_stale_citation(uuid, uuid) → boolean`

**EXTEND** — authorized by spec §3 and D7. **Body is in spec §3.1 and has been executed;** copy it,
do not re-derive, but *do* re-run it before trusting this sentence.

Four constraints, each with the citation to honour:

- **CONFORM** — `resource_live_citations(p_finding)`
  (`20260724000120_standing_citation_components.sql:103-111`), whose COMMENT calls it *"the one
  definition the three standing axes share."* Any other spelling here is a fourth definition.
- **CONFORM** — `(array_agg(… ORDER BY … DESC))[1]`, **not** `max()` (does not exist for `uuid`) and
  **not** `ORDER BY … LIMIT 1` (becomes a scalar subquery the planner evaluates three times per row —
  the multiplication `20260724000130:23-28` forbids).
- **CONFORM** — `a.audited_by_profile_id = p_principal`. Spec §3.2(a): a global max picks the
  *coverage* grain while staleness protects the *quality* grain, and `20260724000210` separated them
  deliberately.
- **CONFORM** — the `(finding, source)` grain via `ab.resource_id = p_finding`. Spec §3.2(b):
  `resource_audit_coverage` is `count(DISTINCT source_id)`, so a `(block, source)` key has a blind
  spot on multi-block findings.

`COMMENT ON FUNCTION` must record **why no material-event allow-list exists**, in the corrected form:
*"nothing on the audit path writes `kb_content_blocks`; the only three writers are `_project_blocks`,
`_project_block_mutated`, `_project_charter_set` (`pg_proc`, 2026-07-26), and there are no triggers on
`kb_content_blocks`, `kb_citation_audits`, or `kb_block_provenance`."* **Do not reproduce the first
draft's "exactly four writers" claim — it named a function that does not exist.**

### 1c. DROP + CREATE `audit_drift_sweep`

**AMEND** — shipped function, selection semantics change.

- **CONFORM** — DROP+CREATE in a *new* migration. `20260724000130` sets the precedent (it
  DROP+CREATEs `workflow_job_claim`); applied migrations are immutable.
- **CONFORM** — signature `(p_principal uuid, p_limit int)` unchanged, so the positional call site
  keeps resolving. That call is `crates/temper-services/src/services/auditor_service.rs` —
  `pub async fn drift_sweep` at `:49`, the `sqlx::query!` at `:55`, `FROM audit_drift_sweep($1, $2)`
  at `:60`. *(The first version of this plan cited `:82` twice; that line is prose in
  `group_by_cogmap`'s doc comment.)*
- **CONFORM** — compute `stale` **once per candidate inside the existing `scored` CTE**, beside the
  two existing producers (`:23-28`).
- **CONFORM** — carry the entire existing body forward unchanged: every filter, the cogmap-home join,
  `steward_candidate_cogmaps`, `resources_visible_to`. Only the `WHERE` and `ORDER BY` change.
- **AMEND** — `WHERE s.magnitude > 0 AND (s.coverage < s.magnitude OR s.stale)`.
- **AMEND, REQUIRED** — the ordering key. A stale finding has `uncovered = 0`, the minimum of
  `ORDER BY uncovered DESC`, so as-is every stale finding sorts behind every uncovered one and, at
  `k >= 50` permanently-stuck findings, the disjunct returns nothing forever. **This is not an open
  sub-decision** (the first version of this plan called it one). Choose a key that can rank a class
  whose `uncovered` is always 0, and **record the choice and its reasoning in the migration**.

### 1d. Amend the KNOWN-FIRST-CUT-LIMITATION comment

`20260724000130:70-82`. Spec §5 splits R10: the evidential half is withdrawn (it was always a verdict,
not a refusal); the structural half — self-authored and unreadable-source citations — is real, is what
starves the stale disjunct, and is D7's conjunct. Update the comment to say that; do not delete it.

---

## Step 2 — Tests in `crates/temper-substrate/tests/citation_audits.rs`

**CONFORM** — extend the existing Task-5 family at `:1174`. Do **not** start a parallel suite; that
header explains why assertions check presence/absence of a `(cogmap_id, finding_id)` pair rather than
row counts (the L0 kernel cogmap is seeded into every test DB).

`#![cfg(feature = "artifact-tests")]`, `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]`. Run
with **`cargo make test-artifacts`** — `check`, `test-db`, and `test-e2e` all skip this tier.

### Verified fixture primitives (do not invent new ones)

| helper | line | note |
|---|---|---|
| `sweep(pool, principal, limit)` | `:1323` | raw `(cogmap_id, finding_id, uncovered)` |
| `join_principal_to_cogmap(...)` | `:1210` | reach |
| `make_cogmap_finding(...)` | `:1223` | cogmap-homed finding |
| `seed_cogmap_finding_with_n_citations(...)` | `:1279` | multi-citation fixture |
| `fire_audit(pool, emitter, block, source, value)` | `:100` | `SELECT citation_audit($1,$2)` at `:113` |
| `first_block(pool, resource)` | `:88` | |
| `common::genesis_cogmap`, `common::create_profile` | `tests/common` | |

⚠️ **`cite()` (`:130-141`) does NOT bump `last_event_id`** — raw `INSERT INTO kb_block_provenance`
reusing `b.genesis_event_id`. That is why the stay-green test works, and why `cite()` cannot be the
material change.

### The material change — typed production path

Working shape verified at `crates/temper-substrate/tests/readout_tier.rs:71-90`; the variant's fields
match `events.rs:320-337` exactly:

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

Do **not** call `block_mutate` via raw SQL with an empty `chunks` array — it raises by design
(`20260624000002_canonical_functions.sql:970`), pinned by `content_mutation.rs:72`.

### The four tests

Numbered per spec §6. **Run 1–4 against the pre-change migration first and record that they fail** —
a witness never observed failing discriminates nothing, which is the exact bar W1 was cancelled for
missing.

1. `sweep_reoffers_a_covered_finding_after_its_block_is_mutated` — primary.
2. `sweep_reoffers_a_covered_finding_after_its_cited_source_changes` — source-side arm.
3. `sweep_reoffers_a_two_block_finding_when_the_unaudited_pair_s_block_changes` — **required.**
   Witnesses spec §3.2(b). Tests 1–2 use single-block fixtures and cannot observe that blind spot.
4. `sweep_stale_is_scoped_to_the_sweeping_principal` — witnesses spec §3.2(a). A audits, block
   mutates, B re-audits; stale for A, not for B. Needs a second profile —
   `common::create_profile` is already used this way at `:1339-1340`.

Stay-green: `sweep_omits_a_fully_covered_finding` (`:1374`).

---

## Step 3 — sqlx cache

`audit_drift_sweep`'s signature and return columns are unchanged, so the workspace cache may not
move — **verify, do not assume.** Read the `sqlx-query-cache` skill; regenerate with
`cargo sqlx prepare --workspace -- --all-features` and commit only genuinely-changed `.sqlx` entries.
Note `prepare` can materialise untracked entries; never blanket `git add .sqlx`.

---

## Step 4 — Gates

```bash
cargo make test-artifacts     # the tier these tests live in; NOT covered by check/test-db/e2e
cargo make check
```

Pre-commit runs incremental clippy, which has gone green where a clean CI build goes red.

---

## Out of scope — each its own task, none to be widened into this PR

| item | why separate |
|---|---|
| **F7** — `ingest_state` gate on `resource_live_citations` | All three standing axes read that function. Far wider blast radius than this change earns. |
| **F6** — content-hash dedup before emitting `block_mutated` | Write-path change; `update_resource_in_tx` gates on `p.body.is_some()`, not "changed". |
| **F9** — verify same-ms UUIDv7 ordering under `pg_uuidv7` on Neon PG17 | Different generator from local PG18. Verify before shipping, but it is an investigation, not a code change. |
| **F8** — source-side clause fires on any block of the source | Unquantified; needs a measurement of how often findings cite their own telos. |
| D3 allow-list, D6 payload, tier 2 / C-7 | Spec §7. |

**Open and unratified** — spec §3.4: whether D7's conjunct is a blocker for this PR or a fast-follow.
I judged it separable; a reviewer judged it inseparable. **Pete decides before Step 1c is written.**
