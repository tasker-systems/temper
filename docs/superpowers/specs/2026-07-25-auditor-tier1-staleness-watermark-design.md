# Auditor tier-1 staleness — the watermark the substrate already keeps

**Date:** 2026-07-25 · **Status:** design, ready to implement
**Supersedes in part:** `2026-07-24-auditor-event-driven-trigger-model-design.md` (D1–D8)
**Branch:** `jct/auditor-tier1-staleness-watermark`

---

## Why this document exists, and why it is short

The trigger model has been specified twice and built zero times. The 2026-07-24 spec (D1–D8) and the
2026-07-25 outcome register (37KB, nine witness tasks) both describe a subsystem that does not exist:
`auditor_watermark_event_id` and `auditor_context_delta` appear nowhere in `migrations/` or
`crates/`, and the register says so about D1 in its own words — *"nothing currently asserts
non-reselection, because nothing implements D1."*

The diagnosis (Pete, 2026-07-25) is not complexity. It is that **the ambitious route had no ground to
build on, so its shape kept evolving in unbuilt futures.** This document deliberately covers the
smallest slice that (a) can be built against live code today and (b) fixes a real, permanent defect.
Everything else is named as deferred rather than designed.

---

## 1. The defect

`audit_drift_sweep` selects on `coverage < magnitude`
(`migrations/20260724000130_audit_drift_sweep.sql`):

```sql
    SELECT s.cogmap_id, s.finding_id, (s.magnitude - s.coverage) AS uncovered
      FROM scored s
     WHERE s.magnitude > 0
       AND s.coverage < s.magnitude
     ORDER BY uncovered DESC, s.finding_id
     LIMIT p_limit;
```

That asks *"is this finding fully covered?"* It should ask *"has anything material happened to it
since it was audited?"*

The consequence is permanent and silent. Once `coverage == magnitude`, a finding **never re-enters
the queue**, so:

> A block is mutated on a fully-covered finding. The assertion's text changes. Every audit of it was
> a verdict about the *previous* text, and those verdicts stand, unrevisited, forever.

This is the corruption shape C-7 named — **staleness misreported as stability** — and it is worse
than C-7's, because C-7 under-triggers a subsystem that does not exist while this one under-triggers
shipped code.

The inverse failure (a partially-covered finding re-heading the queue every tick) is **not** in scope
and is not a defect: it resolves the moment the auditor verdicts the remaining citations, which is a
persona obligation, not a schema gap. See §5.

---

## 2. Grounding — the cursors already exist

**GD-2 (executable).** Established by querying the live dev database and reading the maintaining
projectors, not from the specs.

`kb_events` has **no `resource_id` column** (`20260624000001_canonical_schema.sql:465-488`). The two
routes from an event to the thing it touched:

**Route A — `"references"` (`[{rel, target}]`, GIN-indexed at `:493`). Dead.** This is the join the
schema was designed for. `_event_append`'s `p_references` defaults to `'[]'`
(`20260624000002_canonical_functions.sql:765-789`) and no domain emitter passes it:

```
$ rg -n "RefRel::Touches" crates/
crates/temper-api/tests/admin_ledger_wire_parity_test.rs:46
crates/temper-api/tests/admin_ledger_wire_parity_test.rs:70
```

Two hits, both in one test. Populating it for domain events is the right long-term answer and is a
separate project.

**Route B — the reverse join. Live, indexed, and sufficient.** The projected rows carry the cursor,
so the event→resource direction is never needed:

| what the auditor weighed (D1's axes) | cursor | verified |
|---|---|---|
| the citing block's content | `kb_content_blocks.last_event_id` | `\d kb_content_blocks`: NOT NULL, FK → `kb_events` |
| the size of the citation set | **the same column** | `_insert_block_provenance` path bumps it (`20260704000003_block_provenance_write_path.sql:123`) |
| the cited source's content | the same column on the *source's* blocks | index `idx_kb_content_blocks_resource` covers the join |
| the citing act's confidence | — | immutable once written (D1); no cursor needed |
| the audit itself (the comparand) | `kb_citation_audits.audited_by_event_id` | NOT NULL, the table's only UNIQUE (`20260724000110:23-35`) |

Both sides are `kb_events.id`, i.e. UUIDv7, so `>` is a chronological comparison — the same property
D2 tier 1 already relies on (*"the watermark **is** the trail"*).

### 2.1 What bumps `last_event_id` — the complete set

```
$ rg -n "last_event_id" migrations/*.sql | grep -i "set\|insert"
```

On `kb_content_blocks`, exactly four writers: block genesis (INSERT, `genesis == last`),
`_project_block_mutated` (`20260714000002:183`, `20260713000040:183`), `_project_block_folded`
(`20260629000001:44`), and the block-provenance insert path (`20260704000003:123`).

**All four are material. Nothing immaterial bumps it.** This is why no allow-list is needed: the
column is already, in effect, the material-event filter for the content axes.

### 2.2 The cycle D3's allow-list exists to prevent is structurally impossible here

D3 introduces a named allow-list so `citation_audited` cannot make itself material and drive its own
re-audit loop. On this route it cannot: `_project_citation_audited`
(`20260724000110_citation_audits.sql:84-99`) inserts into `kb_citation_audits` and **touches no
cursor the predicate reads**.

**Therefore the material-event set is not built.** It was 17 members, four of them unemittable, and
it had already drifted twice. Building it to prevent something that cannot happen is the ceremony
this narrowing exists to stop paying. If tier 2 later needs an event-type filter, it can have one
then.

---

## 3. The change

**AMEND** — `audit_drift_sweep` is shipped, and this changes its selection. Authorized by D7
(*"Selection is `uncovered OR stale`"*); this document narrows D7 to the tier-1 half.

One migration, DROP+CREATE (shipped SQL function ⇒ new migration, never an edit
— `20260724000130` itself sets the precedent). The `WHERE` gains a disjunct:

```sql
     WHERE s.magnitude > 0
       AND (s.coverage < s.magnitude OR s.stale)
```

with `stale` computed once per candidate in the existing `scored` CTE — **CONFORM** to that file's
stated rule: *"EACH PRODUCER RUNS ONCE PER CANDIDATE ROW ... repeating them across the WHERE clause
and the SELECT list would run each producer up to four times per row."*

The predicate itself, as a new SQL function so the sweep stays readable and the rule is testable in
isolation:

```sql
CREATE FUNCTION resource_has_stale_citation(p_finding uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1
        FROM resource_live_citations(p_finding) lc
        JOIN kb_content_blocks b ON b.id = lc.block_id
        CROSS JOIN LATERAL (
            SELECT max(a.audited_by_event_id) AS watermark
            FROM kb_citation_audits a
            WHERE a.block_id     = lc.block_id
              AND a.source_kind  = 'resource'
              AND a.source_id    = lc.source_id
        ) w
        WHERE w.watermark IS NOT NULL
          AND (
                b.last_event_id > w.watermark
             OR EXISTS (
                    SELECT 1 FROM kb_content_blocks sb
                     WHERE sb.resource_id   = lc.source_id
                       AND sb.last_event_id > w.watermark
                )
          )
    );
$$;
```

Three clauses, each named by what it is for:

- **`resource_live_citations`** — **CONFORM.** The one definition the three standing axes share
  (`20260724000120:113-116`). Using anything else here would be a fourth spelling of "live citation".
- **`w.watermark IS NOT NULL`** — a never-audited citation is **uncovered, not stale**. Change
  cannot select it (there is nothing to compare against), which is D7's own reasoning and what keeps
  the two disjuncts disjoint rather than overlapping.
- **`max(audited_by_event_id)`** — the *newest* audit is the watermark. Deliberately not "any audit":
  `kb_citation_audits` has no supersession by design (`20260724000110:12-17`), so a citation
  routinely carries several, and only the latest bounds staleness.

---

## 4. Scope — what this does NOT cover

Stated here so it is a **declared hole**, not a gap discovered later.

| not covered | why it is acceptable |
|---|---|
| source soft-deleted | Does not bump blocks, but `resource_live_citations` filters `src.is_active`, so the citation leaves `magnitude`. The incumbent already handles it. |
| `resource_updated` metadata (title, `origin_uri`) | Does not bump blocks. **Correct to miss** — D4 classifies metadata-*about* as immaterial. |
| the corroboration axis (relationship events, new map resources) | Tier 2. Deferred entire. `kb_edges.last_event_id` exists when it is wanted. |
| the observable boundary moving (C-7) | Tier 2. Deferred, and known-broken as designed. |
| D6's citation-list payload | Deferred. This ships against the existing finding-list payload. |
| block mutated mid-session | Benign race: the audit event may postdate content the auditor did not read. Under-triggers by one tick. |

---

## 5. One behavioral contract, not a schema change

Register §6's R10 — *"the auditor has no way to say 'I cannot assess this'"* — **is withdrawn as a
defect** (Pete, 2026-07-25). Default-closed means an assertion is creditable only on demonstrated
derivation, so failure to demonstrate is a **verdict** of `≤ 0`, not a refusal, and the signed value
already expresses it. The two states R10 treated as conflated are already distinct: quality carries
*evaluated-but-weak*, and the band's coverage-ratio gate carries *unevaluated*.

So `20260724000130:70-82`'s *"will re-head this queue every tick with the SAME `uncovered` count
forever"* is a **persona obligation** — the auditor returns a verdict for every citation it is
handed — testable in the auditor's instructions, costing no migration.

The one genuine defect R10 concealed is filed separately (`019f9bfb`): where the auditor **cannot
read the cited source**, "cannot assess" is true for a reason unrelated to the evidence, and grading
it would inject the auditor's reach into the finding's standing. That is a standing refusal — *do not
offer it* — and it must be fixed at the queue, never in `resource_live_citations`, which is correctly
reach-independent.

---

## 6. Witnesses

Two tests, both **failing before and passing after**, extending `citation_audits.rs`'s existing
Task-5 family (`:1174+`) — **CONFORM**, not a parallel suite.

1. **`sweep_reoffers_a_covered_finding_after_its_block_is_mutated`** — the primary. Fully covered
   (`magnitude == coverage`), then `block_mutated`. Today: never re-offered, permanently. After:
   re-offered.
2. **`sweep_reoffers_a_covered_finding_after_its_cited_source_changes`** — the source-side arm,
   which exercises the second disjunct and would pass vacuously if only clause one were implemented.

And one that must **stay green**, guarding against over-triggering:

3. **`sweep_omits_a_covered_finding_with_no_material_change`** — the existing
   `sweep_omits_a_fully_covered_finding` (`:1374`) already asserts this. It must not regress; a
   staleness predicate that fires on a quiet finding is worse than the defect it replaces.

---

## 7. Superseded

- **D3** (material-event allow-list) — not built; §2.2.
- **D6** (citation-list payload) — deferred; §4.
- **D2 tier 2** and **C-7** — deferred; §4.
- **R10** — withdrawn; §5.
- Witness tasks **W1** (`019f9bce`) and **W3** (`019f9bcf`) — cancelled 2026-07-25, reasons on the
  tasks.
- Task `019f9bd0-c9e9` (material-event set) — closed by §2.2 rather than implemented.
