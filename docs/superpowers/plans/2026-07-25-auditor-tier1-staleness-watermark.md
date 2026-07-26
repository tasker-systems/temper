# Implementation plan — auditor tier-1 staleness watermark

**Spec:** [`docs/superpowers/specs/2026-07-25-auditor-tier1-staleness-watermark-design.md`](../specs/2026-07-25-auditor-tier1-staleness-watermark-design.md)
**Branch:** `jct/auditor-tier1-staleness-watermark` (off `76d0de12`)
**Rev. 3** — 2026-07-26. Session notes `019f9ebc-6959-7230-8bdf-bbdec1cbbdf6` + this session's.

> **Read the spec's revision note first.** Rev. 0 of this plan told the implementer *"body is given
> in spec §3; do not re-derive it"* — and that body contained `max(uuid)`, which does not exist.
> Rev. 1 fixed that and shipped a watermark at the wrong grain. **Rev. 2 changes two things rev. 1
> got wrong**: the watermark resolves at `(block, source)` with a guarded arm (spec §3.2(b), with an
> executed four-scenario probe), and **the ordering key is now specified** rather than delegated
> (spec §3.4). D7's conjunct is **ratified separable** (spec §3.5) and is not in this PR.
>
> **Rev. 3 changes the comparand**: `occurred_at`, not the event id. That deletes F9 outright (no id
> generator in the correctness path) and removes the `array_agg(…)[1]` workaround with it. See spec
> §3.1.
>
> Everything here is grep- or psql-verified. Steps carry **CONFORM / EXTEND / AMEND** tags per
> `implementation-grounding.md` GD-3.

---

## Step 1 — Migration `20260726000010_auditor_tier1_staleness.sql`

Numbered above `20260724000220`, the highest on `origin/main`, leaving a gap for concurrent sibling
sessions. *(Rev. 1 justified the gap by PR #546; that has since merged and carried no migrations. The
gap stays as insurance — the reason is now generic, not that branch.)* Re-check the highest migration
on `origin/main` before writing the file; falling behind main means renumbering yours above it, and a
fresh-DB test will not catch the collision.

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
- **CONFORM** — the comparand is **`occurred_at`, not the event id** (spec §3.1). `occurred_at` is
  the substrate's own name for when an event happened, replay-stable, and already read this way by
  `20260626000001_fts_search_index.sql:48`. Comparing `last_event_id` against `audited_by_event_id`
  makes correctness depend on the id generator, which **differs between PG17/Neon (`pg_uuidv7`) and
  PG18/local (native `uuidv7()`)**. Because the comparand is a timestamp, plain `max()` works —
  `max(uuid)` does not exist, and the `(array_agg(… ORDER BY … DESC))[1]` workaround it forced is
  gone with it.
- **CONFORM** — `a.audited_by_profile_id = p_principal`. Spec §3.2(a): a global max picks the
  *coverage* grain while staleness protects the *quality* grain, and `20260724000210` separated them
  deliberately.
- **CONFORM** — **two watermarks, one aggregate.** `block_wm` (via `FILTER (WHERE a.block_id =
  lc.block_id)`) is what the cursors are compared against; `finding_wm` only *guards* the
  `block_wm IS NULL` arm. Spec §3.2(b) carries the executed probe showing that
  `(finding, source)` alone and `(block, source)` alone each miss a case the other catches.
  **Do not "simplify" this to a single LATERAL per watermark** — that is two `Aggregate` nodes for
  one question, the multiplication `20260724000130:23-28` forbids, reached from the other direction.
  **Do not drop `finding_wm IS NOT NULL`** — it reads as a tidy-up and silently converts staleness
  into per-principal coverage (spec §3.2(b) scenario 4).

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
- **AMEND** — `WHERE s.magnitude > 0 AND (s.coverage < s.magnitude OR s.stale)`, moved into the
  `ranked` CTE below.
- **AMEND** — **the ordering key, specified in spec §3.4.** Rev. 0 called it an open sub-decision;
  rev. 1 said it was not one and still left the choice to the implementer. It is settled: a
  `row_number()` ranked within each class (`is_uncovered`), then `ORDER BY rn, is_uncovered DESC, …`
  to interleave. **Copy the block from spec §3.4** — it is executed, it is self-balancing rather than
  slot-reserving, and with nothing stale it is byte-identical to today's ordering.
  **Record in the migration** why a composite `uncovered + stale_count` axis was rejected (spec §3.4:
  stuck findings are older by construction, so they win every tie and `k >= 50` starves it anyway).

### 1d. Correct the KNOWN-FIRST-CUT-LIMITATION prose — via `COMMENT ON FUNCTION`, NOT by editing `20260724000130`

⚠️ **Plan/reality gap, found at implementation.** Earlier revisions of this step said to "amend the
comment at `20260724000130:70-82`." **That is impossible.** `_sqlx_migrations` carries a `checksum`
column, so editing an applied migration file breaks it for every environment that already ran it —
the file is frozen. `audit_drift_sweep` also carries **no** `COMMENT` today (verified via
`obj_description`), so the correction goes on the function object in the *new* migration.

Spec §5 splits R10: the evidential half is withdrawn (it was always a verdict, not a refusal); the
structural half — self-authored and unreadable-source citations — is real and is D7's conjunct.

**Do not write that the conjunct fixes the starvation** — rev. 1 implied it and spec §3.5 refutes it.
This comment's own text already names the dominant stuck population (*"a citation that is readable,
live, and simply never gets audited"*), which the conjunct does not reach and which it defers to a
reaper pass. The starvation is handled by the ordering key in 1c, standing alone. The comment should
now also say that the `uncovered`-monotonicity it describes no longer implies permanent
non-selection, because the `stale` disjunct offers another route in.

---

## Step 2 — The persona line the ordering change falsifies

**AMEND** — `packages/agent-workflows/steward/agent/subagents/auditor/instructions.md:37`:

> The list is ordered by how much of each finding's evidence is still unweighed; work it in that
> order.

Interleaving (Step 1c) makes that false, and a stale row now arrives carrying `uncovered = 0` — a
zero at the top of a list the persona has been told is ranked by unweighed evidence. One-line edit:
*unweighed **or out of date***. Authorized by spec §3.4.

**This is prose describing a gate, not prose standing in for one** — the direction spec §5 rejects is
the other one (moving a gate *into* prose). It rides in this PR because the sentence becomes wrong
the moment the migration lands.

---

## Step 3 — Tests in `crates/temper-substrate/tests/citation_audits.rs`

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

### The six tests

Numbered per spec §6. **Run 1–6 against the pre-change migration first and record that they fail** —
a witness never observed failing discriminates nothing, which is the exact bar W1 was cancelled for
missing.

1. `sweep_reoffers_a_covered_finding_after_its_block_is_mutated` — primary.
2. `sweep_reoffers_a_covered_finding_after_its_cited_source_changes` — source-side arm.
3. `sweep_reoffers_a_two_block_finding_when_the_unaudited_pair_s_block_changes` — witnesses the
   `block_wm IS NULL` arm. Tests 1–2 use single-block fixtures and cannot observe it.
4. `sweep_stale_is_scoped_to_the_sweeping_principal` — witnesses spec §3.2(a). A audits, block
   mutates, B re-audits; stale for A, not for B. Needs a second profile —
   `common::create_profile` is already used this way at `:1339-1340`.
5. `sweep_reoffers_when_a_sibling_audit_lands_after_another_blocks_mutation` — **the one that
   falsifies rev. 1.** Two blocks citing a **shared** source: audit b1, mutate b1, audit b2; must
   read stale. **Additionally run it against rev. 1's `(finding, source)` body** — this witness has a
   second wrong implementation available to bite, and a bite against "the feature is absent" would
   discriminate nothing.
6. `sweep_ordering_gives_stale_findings_slots_under_a_stuck_backlog` — witnesses spec §3.4. Seed
   **more permanently-stuck findings (`uncovered >= 1`) than the cap**, plus stale ones, call `sweep`
   with that cap, assert stale findings appear. Every other fixture in this family is smaller than
   the cap, so nothing else in the suite can observe F4 starvation.

Stay-green:

- `sweep_omits_a_fully_covered_finding` (`:1374`).
- **A principal who has never audited a finding must not see it as stale** — spec §3.2(b) scenario 4.
  This is the witness that stops a later "simplification" from dropping `finding_wm IS NOT NULL`.

---

## Step 4 — sqlx cache

`audit_drift_sweep`'s signature and return columns are unchanged, so the workspace cache may not
move — **verify, do not assume.** Read the `sqlx-query-cache` skill; regenerate with
`cargo sqlx prepare --workspace -- --all-features` and commit only genuinely-changed `.sqlx` entries.
Note `prepare` can materialise untracked entries; never blanket `git add .sqlx`.

---

## Step 5 — Gates

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
| ~~**F9**~~ — same-ms UUIDv7 ordering under `pg_uuidv7` on Neon | **Dissolved, not deferred.** The predicate no longer compares ids, so there is no generator to verify (spec §3.1, §4). |
| **F8** — source-side clause fires on any block of the source | Unquantified; needs a measurement of how often findings cite their own telos. |
| **D7's conjunct** — scope `uncovered` to citations this principal could actually audit (`019f9bfb`) | **Ratified separable** (spec §3.5). It reaches only one of *k*'s three members; the dominant one is deferred to a reaper pass by `20260724000130`'s own comment. So it never was what bounded starvation, and the ordering key had to stand alone regardless. Worth doing on its own merits. |
| **The residual mid-run race** — an edit landing inside an audit run still under-triggers by one tick | Spec §4. Fixing the grain removes the *permanence*, not the one-tick gap. If it shows up in practice: order write-action timestamps, or define a suspect "within" window that emits a bump-for-re-audit event. Judged not worth solving today (Pete, 2026-07-26). |
| D3 allow-list, D6 payload, tier 2 / C-7 | Spec §7. |

**Nothing in this plan is unratified.** Rev. 1 ended with an open question — whether D7's conjunct
blocked this PR — and spec §3.5 settles it against both prior positions.
