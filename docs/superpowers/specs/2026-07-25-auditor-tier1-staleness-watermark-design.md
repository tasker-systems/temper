# Auditor tier-1 staleness — the watermark the substrate already keeps

**Date:** 2026-07-25 (rev. 2026-07-26 after adversarial review) · **Status:** design, ready to implement
**Supersedes in part:** `2026-07-24-auditor-event-driven-trigger-model-design.md` (D1–D8)
**Branch:** `jct/auditor-tier1-staleness-watermark`

> **Revision note.** The first draft of this document was reviewed by three adversarial passes
> (attack-the-narrowing · attack-the-predicate · verify-every-citation). It **failed**: the predicate
> did not compile, its watermark was at the wrong grain on two axes, and §2.1's grounding table —
> the load-bearing justification for one of its decisions — was derived from `grep` rather than
> `pg_proc` and was wrong on two of four members. Everything below is the repaired version. Findings
> that survived review unchanged are marked **[held]**; findings that changed are marked
> **[revised]** with what was wrong. Session note: `019f9ebc-6959-7230-8bdf-bbdec1cbbdf6`.

---

## Why this document exists, and why it is short **[held]**

The trigger model has been specified twice and built zero times. The 2026-07-24 spec (D1–D8) and the
2026-07-25 outcome register (37KB, nine witness tasks) both describe a subsystem that does not exist:
`auditor_watermark_event_id` and `auditor_context_delta` appear nowhere in `migrations/` or
`crates/`, and the register says so about D1 in its own words — *"nothing currently asserts
non-reselection, because nothing implements D1."*

The diagnosis (Pete, 2026-07-25) is not complexity. It is that **the ambitious route had no ground to
build on, so its shape kept evolving in unbuilt futures.** This document covers the smallest slice
that can be built against live code today and fixes a real, permanent defect.

**The correction review added:** narrowing must be cut along **dependency** lines, not **document**
lines. The first draft cut D7 mid-decision and dropped the half that keeps the retained half from
starving. §3.4 and §4 are now organised by what the kept part depends on.

---

## 1. The defect **[held]**

`audit_drift_sweep` selects on `coverage < magnitude`
(`migrations/20260724000130_audit_drift_sweep.sql:116-121`):

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

Once `coverage == magnitude`, a finding **never re-enters the queue**, so a block mutation changes an
assertion's text while every verdict about the previous text stands, unrevisited, forever. This is
C-7's shape — **staleness misreported as stability** — against shipped code rather than a subsystem
that does not exist.

---

## 2. Grounding — the cursors already exist

**GD-2 (executable).** Every claim in this section is from `psql` against the live dev database
(PG 18.3, all migrations through `20260724000220`) or from `pg_proc`. **Not from `grep` — that is
what produced the errors in the first draft.**

`kb_events` has **no `resource_id` column** (`20260624000001_canonical_schema.sql:465-488`). Two
routes from an event to the thing it touched:

**Route A — `"references"`. Unusable for domain events. [revised]** GIN-indexed at `:493`, and it is
the join the schema was designed for. `_event_append`'s `p_references` defaults to `'[]'`
(`20260624000002_canonical_functions.sql:765-789`) and no **domain** emitter passes it;
`RefRel::Touches` is constructed only in `crates/temper-api/tests/admin_ledger_wire_parity_test.rs`.

> The first draft called this *"dead."* It is not — five live **admin** functions populate
> `references` with `subject` / `principal` rels (`_admin_grant_created`, `_admin_grant_revoked`,
> `_admin_slack_disconnected`, `principal_governance_set`, `principal_standing_apply`), and
> `admin_ledger_service.rs` reads them. The domain-event conclusion stands; the word was wrong.

**Route B — the reverse join. Live, indexed, sufficient for the content axes. [revised]**

| what the auditor weighed (D1's axes) | cursor | verified |
|---|---|---|
| the citing block's content | `kb_content_blocks.last_event_id` | live `\d`: NOT NULL, FK → `kb_events` |
| the cited source's content | the same column on the *source's* blocks | needs a new index — §3.3 |
| the citing act's confidence | — | immutable once written (D1); no cursor needed |
| the audit itself (the comparand) | `kb_citation_audits.audited_by_event_id` | NOT NULL, the table's only UNIQUE (`20260724000110:23-35`) |

> **[revised] The first draft claimed a third row — "the size of the citation set | the same column |
> `_insert_block_provenance` bumps it".** That is **false**. `_insert_block_provenance` writes only
> `kb_block_provenance` (live body: zero occurrences of `last_event_id`), and so does
> `_project_block_annotated`, the production annotate path. The cited line `20260704000003:123` is
> inside `_project_block_mutated` — a filename was mistaken for a function identity.
>
> **The citation-set-size axis is carried by the `uncovered` disjunct, not by the cursor:** adding a
> citation raises `magnitude` while `coverage` stays put, so `coverage < magnitude` fires. The
> shrink direction (`block_provenance_corrected`) has no write path to fire from.
>
> This reframes the design usefully. **The two disjuncts are complementary, not redundant** —
> `uncovered` handles the citation set changing *size*, the watermark handles content changing under
> a *stable* set. Neither is sufficient alone, which is why §3.4 keeps both and why D7's union is the
> point rather than an incidental.

### 2.1 What bumps `last_event_id` — the complete set, from `pg_proc` **[revised]**

```
$ psql … -c "SELECT proname FROM pg_proc WHERE prosrc ~ 'last_event_id' AND prosrc ~ 'kb_content_blocks';"
 _project_block_mutated
 _project_blocks           -- INSERT; genesis == last
 _project_charter_set      -- folds a telos's blocks, bumps the cursor in the same statement
```

**Three writers, not four.** `_project_block_folded` **does not exist** (0 rows in `pg_proc`); the
first draft named a projector for `block_folded`, an event with no write path at all. The line it
cited (`20260629000001:44`) is inside `_project_charter_set`, which the first draft never named.

There are **no triggers** on `kb_content_blocks`, `kb_citation_audits`, or `kb_block_provenance` —
the database has exactly three non-internal triggers, on `kb_events`, `kb_profiles`, and
`kb_principal_standing_events`.

**Two of the three writers are content-material. `_project_charter_set` is not** — `charter_set` is
classified *excluded* by the register's own partition (*"Changes what the map is for, not what a
source says"*), yet it bumps the cursor on every live block of a cogmap's telos. Any finding citing
its own map's telos therefore goes stale on every charter edit. See §4 (F8) — accepted, bounded, and
named rather than discovered later.

### 2.2 The cycle D3's allow-list exists to prevent is structurally impossible **[held]**

D3 introduces a named allow-list so `citation_audited` cannot make itself material and drive its own
re-audit loop. It cannot: `_project_citation_audited` (`20260724000110:79-104`) does exactly one
thing — `INSERT INTO kb_citation_audits … ON CONFLICT (audited_by_event_id) DO NOTHING` — and
touches no cursor this predicate reads. `_event_append` only inserts into `kb_events`. No trigger can
carry it (see §2.1).

**Verified independently by all three reviewers. Do not re-litigate it.**

**Therefore the material-event set is not built.** It was 17 members, four unemittable, and had
already drifted. The migration's `COMMENT` must record the **true** reason — *"nothing on the audit
path writes `kb_content_blocks`; the only three writers are `_project_blocks`,
`_project_block_mutated`, `_project_charter_set`"* — not the first draft's false enumeration.

---

## 3. The change

**AMEND** — `audit_drift_sweep` is shipped and this changes its selection. Authorized by D7
(*"Selection is `uncovered OR stale`"*). **[revised] D7 is honoured whole, not halved** — see §3.4.

One migration, DROP+CREATE (`20260724000130` sets the precedent by DROP+CREATE-ing
`workflow_job_claim`).

### 3.1 The predicate — executed before being written here **[revised]**

The first draft's body used `max(uuid)`, **which does not exist in PostgreSQL**, so the migration
would have failed at `sqlx migrate run` (`LANGUAGE sql` bodies parse at CREATE time). It was written
into a spec and the plan told the implementer not to re-derive it. The body below was **executed
against the live database and creates cleanly**:

```sql
CREATE FUNCTION resource_has_stale_citation(p_finding uuid, p_principal uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1
        FROM resource_live_citations(p_finding) lc
        JOIN kb_content_blocks b ON b.id = lc.block_id
        CROSS JOIN LATERAL (
            SELECT (array_agg(a.audited_by_event_id ORDER BY a.audited_by_event_id DESC))[1]
                   AS watermark
              FROM kb_citation_audits a
              JOIN kb_content_blocks ab
                ON ab.id = a.block_id AND ab.resource_id = p_finding
             WHERE a.source_kind = 'resource'
               AND a.source_id   = lc.source_id
               AND a.audited_by_profile_id = p_principal
        ) w
        WHERE w.watermark IS NOT NULL
          AND ( b.last_event_id > w.watermark
             OR EXISTS (SELECT 1 FROM kb_content_blocks sb
                         WHERE sb.resource_id   = lc.source_id
                           AND sb.last_event_id > w.watermark) )
    );
$$;
```

**`array_agg(…)[1]`, not `ORDER BY … LIMIT 1`.** The obvious repair is also wrong: a scalar subquery
gets pulled up and evaluated **three times per citation row** (measured: `SubPlan 1` + `SubPlan 2` +
`InitPlan 3`), which is exactly what `20260724000130:23-28` forbids. `array_agg` produces one
`Aggregate` node.

### 3.2 Two grain corrections, both load-bearing **[revised]**

**(a) Scope the watermark to the sweeping principal** — `a.audited_by_profile_id = p_principal`.

Without it, `max()` over *all* auditors re-creates §1's own defect one principal over. Demonstrated:
auditor A verdicts `-1.0`; the block is mutated; auditor B reads the new text and verdicts `+1.0`.
The global max takes B's event id, so `stale = false`, and `coverage == magnitude`, so the finding is
**terminal** — unreachable by any route — while A's verdict about deleted text still carries 50% of a
per-auditor aggregate (`20260724000210` is one-vote-per-principal), rendering the finding `disputed`
though the only auditor who read the current text endorsed it.

The diagnosis: a global max picks the **coverage** grain (*did anyone look?*) while staleness
protects the **quality** grain (*is each principal's vote about the current text?*). Those grains
were deliberately separated by `20260724000210`. `audit_drift_sweep(p_principal, …)` already asks a
principal-scoped question; the staleness half must ask the same one.

**(b) Key the watermark on `(finding, source)`, not `(block, source)`** — hence the join
`ab.resource_id = p_finding`.

`resource_audit_coverage` is `count(DISTINCT source_id)` with a per-row `EXISTS` on
`(block_id, source_id)`, so auditing a source on **one** block marks it covered across **all** blocks
of the finding. A `(block, source)`-keyed watermark therefore has a blind spot: a two-block finding,
or a `block_append` adding new text citing an already-audited source, is selected by **neither**
disjunct, permanently. Matching coverage's grain closes it.

### 3.3 One additive index, and why the obvious filter is wrong **[revised]**

Both existing `resource_id` indexes are **partial on `NOT is_folded`**, so the source-side clause can
use neither — confirmed with `enable_seqscan = off`, which still yields a `Seq Scan`. That is a
correlated sequential scan of the largest table in the schema, once per citation row, for every
candidate.

**Do NOT add `NOT sb.is_folded` to fix it.** Folding a source block **is content removal** — *the
source deleted the passage you cited* — and `_project_charter_set` folds and bumps in one statement.
Filtering folded blocks makes that invisible. The omission is load-bearing; this paragraph is the
documentation it lacked.

The fix is a new **non-partial** index, additive so it does not disturb the additive-only-on-`main`
invariant. Verified:

```
-- without
 Seq Scan on kb_content_blocks sb
   Disabled: true
-- with CREATE INDEX ON kb_content_blocks (resource_id, last_event_id)
 Index Only Scan using idx_kb_content_blocks_resource_cursor on kb_content_blocks sb
   Index Cond: ((resource_id = …) AND (last_event_id > …))
```

### 3.4 Selection and ordering — D7 whole **[revised]**

```sql
     WHERE s.magnitude > 0
       AND (s.coverage < s.magnitude OR s.stale)
```

with `stale` computed **once per candidate inside the existing `scored` CTE** — CONFORM to that
file's *"EACH PRODUCER RUNS ONCE PER CANDIDATE ROW"* rule (`:23-28`).

**The ordering key must change, and this is not an implementer's sub-decision.** A stale finding is
by construction fully covered, so `uncovered = magnitude - coverage = 0` — the **minimum** of the
carried-forward `ORDER BY uncovered DESC`. Every stale finding sorts behind every uncovered one.
With `DEFAULT_AUDITOR_DISPATCH_CAP = 50` and *k* permanently-stuck findings sitting at
`uncovered >= 1` forever, stale rows get `50 − k` slots, allocated by `finding_id` ASC — UUIDv7, so
**oldest-first deterministically**, meaning newer stale findings are *structurally excluded* rather
than delayed. At `k >= 50` the disjunct returns nothing, silently, forever: the exact sentence §1
uses to condemn the incumbent.

The ordering must rank a class whose `uncovered` is always 0. The shape is the implementer's to
choose and **must be recorded in the migration**, not settled in their head.

**D7's second half comes back.** *"'uncovered' is permanent for a citation this principal cannot
audit … the loop surviving its own fix."* Scoping `uncovered` to citations this principal could
actually audit is what bounds *k*. `citation_contributed_by_profile` already exists and is the exact
question the gate asks, so this is a `NOT EXISTS` join, not a new predicate.

> **Open — Pete to ratify.** I judged this *separable* from the ordering fix (with an honest ordering
> key, stale findings get slots regardless of *k*, and the stuck population is a **pre-existing**
> pathology this change does not introduce), against a reviewer who called them inseparable. If that
> judgement is wrong, the conjunct is a blocker rather than a fast-follow. **Not settled.**

---

## 4. Scope — declared holes, honestly described **[revised]**

The first draft described two of these as *handled*. They are not; they are *absorbed*. A declared
hole requires an honest description of the hole.

| not covered | status |
|---|---|
| source soft-deleted | **Not "handled."** The citation leaves `magnitude`, so the finding's evidential breadth silently halves and **no re-audit is triggered** — the surviving verdict was a verdict about a larger citation set. C-7's shape inside a row the first draft marked acceptable. Deferred, not benign. |
| `resource_updated` metadata (title, `origin_uri`) | Correct to miss — D4 classifies metadata-*about* as immaterial. |
| tier 2 / corroboration / C-7 | Deferred. **Accepted consequence:** re-audit attention correlates with *editing activity*, not with corroboration change. Still strictly better than the status quo, where nothing is re-offered. |
| D6's citation-list payload | Deferred; ships against the existing finding-list payload. The auditor will re-audit non-stale citations of a re-offered finding — bounded cost, and it partially mitigates any residual grain gap. |
| **F6 — no content-hash dedup** | `update_resource_in_tx` gates on `p.body.is_some()`, not "the body changed", and neither `block_mutate` nor its projector guards. CLAUDE.md's own show-edit-`cat` idiom therefore fires staleness on **byte-identical** content. `_project_block_mutated` already computes `v_block_hash`, so the fact is available. **Own task.** |
| **F7 — `resource_live_citations` has no `ingest_state` gate** | Its own header quotes the rule it violates. A half-uploaded source confers standing *and* re-queues its citers once per arriving block. **Must NOT ride along** — all three standing axes read this function; far wider blast radius than this change earns. **Own task.** |
| **F8 — source-side clause fires on ANY block of the source** | Including self-citations and high-churn sources (a telos: `_project_charter_set` folds and reinserts every block per `charter_set`). In a self-cognition KB, findings-citing-findings is the normal case. **Unquantified — nobody has measured how often findings cite their own telos.** |
| **F9 — same-millisecond ordering unverified on prod** | `20260624000001:48-70` branches: **PG17/Neon ⇒ `pg_uuidv7` extension; PG18/local ⇒ native `uuidv7()`.** Different generators. Local is monotone within a millisecond (0 out-of-order pairs in 50k). If `pg_uuidv7` fills `rand_a` randomly, same-ms comparisons are arbitrary. **Verify against Neon before shipping.** |
| block mutated mid-session | Benign race; under-triggers by one tick. |

---

## 5. R10 — withdrawn for the evidential case, NOT the structural one **[revised]**

**What holds.** Default-closed means an assertion is creditable only on demonstrated derivation, so
failure to demonstrate is a **verdict** of `<= 0`, not a refusal — the signed value already expresses
it. The standing model already separates the two states R10 conflated: quality carries
*evaluated-but-weak*, the band's coverage-ratio gate carries *unevaluated*. There is no third state.

**What the first draft got wrong.** It extended this to *"the auditor always verdicts, so the
re-heads-forever loop is a persona obligation, testable in instructions."* The shipped instructions
refute it (`packages/agent-workflows/steward/agent/subagents/auditor/instructions.md:144-146`):

> If a write comes back "not found, unreadable, or self-authored", it is not retryable and it is not
> a bug ... **Note it and move on to the next citation.**

Some citations the auditor is **structurally forbidden** to verdict — the gate 404s them, and the
persona is already correctly instructed to skip. No instruction can change that. Worse, moving a gate
into prose is the move **D6's own words** reject: *"a gate is only as strong as the narrowest path
around it. Prose in an instructions file is a wider path than a queue payload."*

**So R10 decomposes into two, and only the first is withdrawn:**

1. **Evidential inability → a verdict.** Withdrawn. No mechanism needed.
2. **Structural inability → D7's conjunct** (§3.4). Self-authored citations, and citations whose
   source the principal cannot read (`019f9bfb`). These genuinely never gain coverage, and they are
   the population that starves the stale disjunct.

---

## 6. Witnesses **[revised]**

Failing-before / passing-after, extending `citation_audits.rs`'s Task-5 family (`:1174+`) —
**CONFORM**, not a parallel suite.

1. **`sweep_reoffers_a_covered_finding_after_its_block_is_mutated`** — fully covered, then
   `block_mutated`. Today: never re-offered.
2. **`sweep_reoffers_a_covered_finding_after_its_cited_source_changes`** — the source-side arm;
   would pass vacuously if only clause one existed.
3. **`sweep_reoffers_a_two_block_finding_when_the_unaudited_pair_s_block_changes`** — **new, and
   required.** Witnesses §3.2(b). The first draft's witness set used single-block fixtures
   throughout and **could not observe the primary blind spot**.
4. **`sweep_stale_is_scoped_to_the_sweeping_principal`** — **new.** Witnesses §3.2(a): auditor A
   audits, block mutates, auditor B re-audits; the finding must still read stale *for A* and not
   *for B*.

Must **stay green** — `sweep_omits_a_fully_covered_finding` (`:1374`), verified unaffected (the
fixture creates the source before the finding and audits last, so both cursors precede the
watermark). A predicate that fires on a quiet finding is worse than the defect it replaces.

---

## 7. Superseded

- **D3** (material-event allow-list) — not built; §2.2. Unanimously upheld on review.
- **D6** (citation-list payload), **D2 tier 2**, **C-7** — deferred; §4.
- **R10** — half withdrawn, half promoted to D7's conjunct; §5.
- Witness tasks **W1** (`019f9bce-9a7f-79b3-8b2b-77e9ee064730`) and
  **W3** (`019f9bcf-30c0-7772-8833-340b90f22f6b`) — cancelled 2026-07-25, reasons on the tasks.
- Task `019f9bd0-c9e9-7aa3-95ad-bc8b11325428` (material-event set) — closed by §2.2, not implemented.
- `019f9bfb-62e2-7c62-85b2-e309ac1b18c1` (source-visibility) — **promoted**: part of D7's conjunct.
