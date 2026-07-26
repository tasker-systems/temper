# Auditor tier-1 staleness — the watermark the substrate already keeps

**Date:** 2026-07-25 (rev. 2 — 2026-07-26) · **Status:** design, ready to implement
**Supersedes in part:** `2026-07-24-auditor-event-driven-trigger-model-design.md` (D1–D8)
**Branch:** `jct/auditor-tier1-staleness-watermark`

> **Revision note.** The first draft was reviewed by three adversarial passes
> (attack-the-narrowing · attack-the-predicate · verify-every-citation). It **failed**: the predicate
> did not compile, its watermark was at the wrong grain on two axes, and §2.1's grounding table —
> the load-bearing justification for one of its decisions — was derived from `grep` rather than
> `pg_proc` and was wrong on two of four members. Findings that survived unchanged are marked
> **[held]**; findings that changed are marked **[revised]**.
>
> **Rev. 2 repairs the repair.** Rev. 1's grain fix (§3.2(b)) was itself defective: it re-keyed the
> watermark to `(finding, source)`, which recreates rev. 1's own §3.2(a) defect one grain over — an
> audit of one block masks a sibling block's mutation permanently. Both grains leak, in opposite
> directions; §3.2(b) below carries the executed four-scenario probe and the predicate that passes
> all four. Rev. 2 also **settles the ordering key** (§3.4), which rev. 1 declared "not an open
> sub-decision" and then left to the implementer, and **ratifies D7's conjunct as separable** — for
> a reason rev. 1 did not give. Sections changed in rev. 2 are marked **[rev2]**.
>
> **Rev. 3 changes the comparand and deletes F9.** Pete asked why the predicate was comparing UUIDs
> at all when record-level timestamps exist. It should not have been: `occurred_at` is the
> substrate's own name for when an event happened, and comparing ids made a correctness-critical
> comparison depend on an id generator that **differs between PG17/Neon and PG18/local**. Switching
> to timestamps removes that dependency (F9 dissolves — §4), and removes the `array_agg(…)[1]`
> workaround with it, since `max()` exists for `timestamptz` and not for `uuid`. Sections changed in
> rev. 3 are marked **[rev3]**.
> Session notes: `019f9ebc-6959-7230-8bdf-bbdec1cbbdf6`, and this session's.

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
(*"Selection is `uncovered OR stale`"*). **[rev2] D7's union ships; D7's conjunct is a ratified
fast-follow** — see §3.5 for why it cannot be what bounds *k*, and therefore why it is separable.

One migration, DROP+CREATE (`20260724000130` sets the precedent by DROP+CREATE-ing
`workflow_job_claim`).

### 3.1 The predicate — executed before being written here **[rev3]**

```sql
CREATE FUNCTION resource_has_stale_citation(p_finding uuid, p_principal uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1
        FROM resource_live_citations(p_finding) lc
        JOIN kb_content_blocks b  ON b.id  = lc.block_id
        JOIN kb_events         be ON be.id = b.last_event_id
        CROSS JOIN LATERAL (
            SELECT
                -- This principal's freshest audit anywhere on this (finding, source)...
                max(a.created)                                         AS finding_wm,
                -- ...and its freshest audit of THIS citation specifically.
                max(a.created) FILTER (WHERE a.block_id = lc.block_id) AS block_wm
              FROM kb_citation_audits a
              JOIN kb_content_blocks ab
                ON ab.id = a.block_id AND ab.resource_id = p_finding
             WHERE a.source_kind = 'resource'
               AND a.source_id   = lc.source_id
               AND a.audited_by_profile_id = p_principal
        ) w
        WHERE w.finding_wm IS NOT NULL      -- this principal has engaged this (finding, source)
          AND ( w.block_wm IS NULL          -- ...but never at THIS block
             OR be.occurred_at > w.block_wm
             OR EXISTS (SELECT 1 FROM kb_content_blocks sb
                          JOIN kb_events se ON se.id = sb.last_event_id
                         WHERE sb.resource_id = lc.source_id
                           AND se.occurred_at > w.block_wm) )
    );
$$;
```

**The comparand is a timestamp, not an event id. [rev3 — the change that dissolved F9.]**

Revs. 1 and 2 compared `kb_content_blocks.last_event_id` against
`kb_citation_audits.audited_by_event_id` directly. A UUIDv7 is timestamp-prefixed, so that *looks*
like comparing times — and it genuinely helps btree locality — but it makes a correctness-critical
comparison depend on the **id generator**, which is not the same in both environments:
`20260624000001:48-70` branches on `pg_available_extensions`, giving PG17/Neon `pg_uuidv7` and
PG18/local an alias for the native `uuidv7()`. That is F9, and it was a standing prod gate on this
predicate *and on every future change to it*.

**The deciding argument is not the generator, though — it is that the substrate already has a name
for this.** `occurred_at` is the event's own "when this happened", replay-stable by construction
(`replay.rs`: *"projected timestamps come from the event's `occurred_at`, never `now()`"*) and
already read exactly this way by `20260626000001_fts_search_index.sql:48`. Comparing ids
reimplemented an ordering the system already exposes under a name — the drift-site test in
`plan-verification.md`, which this design failed for two revisions and which no amount of
*verifying the claim* would have caught, because the claim was true.

Three things follow, in descending order of importance:

1. **F9 is deleted, not deferred.** No generator dependency, so nothing to verify against Neon.
2. **`max()` just works**, and a trap goes with it. `max(uuid)` **does not exist** in PostgreSQL and
   a `LANGUAGE sql` body parses at CREATE time, so rev. 0's body would have failed at
   `sqlx migrate run`. Its workaround was `(array_agg(… ORDER BY … DESC))[1]` — because
   `ORDER BY … LIMIT 1` becomes a scalar subquery the planner evaluates **three times per citation
   row** (measured: `SubPlan 1` + `SubPlan 2` + `InitPlan 3`), the producer multiplication
   `20260724000130:23-28` forbids. That entire hazard existed only to serve the uuid comparand.
3. **Both watermarks still come from ONE aggregate**, now via `FILTER` on `max()`. Two `CROSS JOIN
   LATERAL` blocks over the same audit set would be two `Aggregate` nodes for one question — the
   same multiplication `:23-28` forbids, from the other direction.

**The cost, stated honestly.** Two primary-key joins to `kb_events`, and it **could not be measured
here**: `kb_content_blocks` has **0 rows** on the dev database, so local plans are meaningless. The
structural claim is only that PK joins are the cheapest join available and that
`resource_live_citations` already performs several per call. **If profiling objects, denormalize a
`last_event_at` onto `kb_content_blocks` (additive column, three writers, total backfill from
`kb_events`) — do not go back to comparing ids.**

**Neither comparand is commit order.** The uuid was minted mid-transaction; `occurred_at` is
transaction start. Both are approximations, and the gap between them sits below the resolution of
what this predicate means. `occurred_at` wins because it is *named and replay-stable*, not because
it is more precise.

`kb_citation_audits.created` needs no join of its own: `_project_citation_audited` sets it
explicitly from `e.occurred_at`, so it is the audit event's own time and is replay-stable for the
same reason.

### 3.2 Two grain corrections, both load-bearing **[(a) revised · (b) rev2]**

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

**(b) Resolve the watermark at `(block, source)`, and add a guarded arm for citations this
principal has engaged elsewhere on the finding but not here.** **[rev2 — rev. 1 got this wrong.]**

The underlying asymmetry: **an audit is keyed `(block_id, source_kind, source_id)`** — block grain,
per `kb_citation_audits` — **but `resource_audit_coverage` collapses it to
`count(DISTINCT source_id)`**, so auditing a source on one block marks it covered across all blocks
of the finding.

Rev. 1 responded by coarsening the watermark to `(finding, source)` to match coverage. That closes
the gap coverage opens and **opens a new one inside the block set**: the coarse key takes the
*latest* audit across every block, so an audit of one block sits above another block's mutation and
hides it permanently — §3.2(a)'s defect, one grain over. Rev. 1 carried the block grain into the
*join* (`b.last_event_id`, the source-side `EXISTS`) but not into the *watermark resolution*, so the
comparand stayed resource-scoped while both things it is compared against became block-scoped.

Executed, dev PG 18.3, rolled-back transaction. `uncovered` is false in **every** row, so no scenario
is rescued by the other disjunct:

| scenario | rev. 1 `(finding, source)` | pure `(block, source)` | §3.1 as written |
|---|---|---|---|
| 1 · an audit of a sibling block masks a mutated block | **false** ✗ | true | true |
| 2 · block appended after the audit (the F2b case) | true | **false** ✗ | true |
| 3 · quiet fully-covered finding — *must stay green* | false | false | false |
| 4 · principal who has never audited this finding | false | false | false |

**Neither pure grain is sufficient**, and the reason is structural: a compensation applied at a
different grain than the defect leaks in the other direction. §3.1 therefore resolves the watermark
at the grain an audit actually has (`block_wm`) and adds one arm for the gap coverage leaves —
`block_wm IS NULL`, *this principal has never audited this citation* — guarded by
`finding_wm IS NOT NULL` so it fires only for a principal already engaged with this
`(finding, source)`. Row 4 is what that guard buys: without it, every finding would read stale for
every principal who has not yet audited it, converting staleness into per-principal **coverage** — a
far larger behaviour change than tier 1 claims.

**Scenario 1 needs no skipped citation, and this is the part rev. 1's scope table gets wrong.** Read
its three events as one audit run: the auditor audits block b1, someone edits b1 while the run is
still going — runs are LLM-paced minutes — the auditor audits b2 and finishes. Ordinary behaviour on
both sides. See §4's *block mutated mid-session* row, whose "benign race" assessment rev. 1
invalidated without revisiting it.

**What bounds this, and what does not.** The queue payload is
`AuditJobPayload { findings: Vec<Uuid> }` — finding ids, no citation list — and the auditor's step 8
is *"emit one verdict per auditable citation"*
(`packages/agent-workflows/steward/agent/subagents/auditor/instructions.md:36-73`). So a visit that
**completes** refreshes every citation and the coarse watermark re-converges with reality;
dispatching at the finding grain is a real mitigation. What fails is *"the next tick will catch
it"*: after the masking event `stale = false` **and** `uncovered = 0`, so the sweep does not return
the finding at all. It returns only on a new material event touching the finding or one of its
sources — and for a settled finding, the state §1 exists to condemn, that may be never.

### 3.3 One additive index, and why the obvious filter is wrong **[revised · still required in rev. 3]**

Both existing `resource_id` indexes are **partial on `NOT is_folded`**, so the source-side clause can
use neither — confirmed with `enable_seqscan = off`, which still yields a `Seq Scan`. That is a
correlated sequential scan of the largest table in the schema, once per citation row, for every
candidate.

**[rev3] The move to timestamps does not retire this index — both columns still earn their place.**
`resource_id` selects the cited source's blocks, and carrying `last_event_id` keeps that an
**index-only** scan, so the `kb_events` step is a plain primary-key probe with no heap fetch on
`kb_content_blocks`. The comparison moved to the joined row; the lookup did not.

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

### 3.4 Selection and ordering **[rev2]**

```sql
     WHERE s.magnitude > 0
       AND (s.coverage < s.magnitude OR s.stale)
```

with `stale` computed **once per candidate inside the existing `scored` CTE** — CONFORM to that
file's *"EACH PRODUCER RUNS ONCE PER CANDIDATE ROW"* rule (`:23-28`).

**The ordering key must change.** A stale finding is by construction fully covered, so
`uncovered = magnitude - coverage = 0` — the **minimum** of the carried-forward
`ORDER BY uncovered DESC`. Every stale finding sorts behind every uncovered one. With
`DEFAULT_AUDITOR_DISPATCH_CAP = 50` (`crates/temper-core/src/types/workflow_job.rs:39`) and *k*
permanently-stuck findings sitting at `uncovered >= 1` forever, stale rows get `50 − k` slots,
allocated by `finding_id` ASC — UUIDv7, so **oldest-first deterministically**, meaning newer stale
findings are *structurally excluded* rather than delayed. At `k >= 50` the disjunct returns nothing,
silently, forever: the exact sentence §1 uses to condemn the incumbent.

**The key: rank within class, interleave by rank. [rev2]** Rev. 1 left this to the implementer while
asserting it was not an open sub-decision. It is settled here.

```sql
), ranked AS (
    SELECT s.*, (s.coverage < s.magnitude) AS is_uncovered,
           row_number() OVER (PARTITION BY (s.coverage < s.magnitude)
                              ORDER BY (s.magnitude - s.coverage) DESC, s.finding_id) AS rn
      FROM scored s
     WHERE s.magnitude > 0 AND (s.coverage < s.magnitude OR s.stale)
)
SELECT cogmap_id, finding_id, (magnitude - coverage) AS uncovered
  FROM ranked
 ORDER BY rn, is_uncovered DESC, (magnitude - coverage) DESC, finding_id
 LIMIT p_limit;
```

Executed — 6 stuck findings against 4 stale ones at a cap of 5, the starvation shape scaled down:
stale takes **2 of 5 slots**, where today it takes 0. It is **self-balancing rather than
reserving** — only 3 findings stale means 3 stale and 47 uncovered, no slots wasted — and with
nothing stale it is **byte-identical to today's ordering** (verified by string comparison of the two
orderings, not by inspection), so the change is invisible until the new disjunct has something to
contribute.

Three constraints, each checked rather than assumed:

- **CONFORM `:23-28`** — window functions over `scored`; no additional producer calls.
- **CONFORM the determinism claim** — `rn`'s window ordering terminates in `finding_id`, so `rn` is
  total. `auditor_service::drift_sweep`'s *"deterministic, principal-scoped sweep"* and
  `group_by_cogmap`'s *"the cogmaps come out in the order their first finding appeared"* both hold.
- **Signature unchanged** — `RETURNS TABLE(cogmap_id, finding_id, uncovered)`, so the `sqlx::query!`
  at `auditor_service.rs:55-60` and `AuditSweepRow` do not move, and D6 stays deferred.

**A rejected alternative, recorded because it is the tempting one.** A single composite axis —
`attention = uncovered + stale_citation_count` — is semantically cleaner and **does not work**: a
stuck finding sits at `attention >= 1` and a stale finding sits at `attention >= 1`, they tie, and
the tie-break is `finding_id` ASC. Stuck findings are older *by construction* (they have been stuck),
so they win every tie and `k >= 50` starves the disjunct exactly as before.

**Persona ripple — must ride in the same PR.** `instructions.md:37` tells the auditor *"The list is
ordered by how much of each finding's evidence is still unweighed; work it in that order."*
Interleaving makes that false, and a stale row arrives carrying `uncovered = 0`. One-line edit:
*unweighed **or out of date***. This is prose *describing* a gate, not prose *standing in for* one —
the direction §5 warns against is the other one — but it is exactly the kind of thing that gets
dropped.

### 3.5 D7's conjunct — separable. Ratified. **[rev2]**

D7's second half: *"'uncovered' is permanent for a citation this principal cannot audit … the loop
surviving its own fix."* Rev. 1 judged it separable from the ordering fix; a reviewer judged them
inseparable because the conjunct is what bounds *k*. **Both were wrong about the mechanism, and the
conclusion is separable.**

`20260724000130:70-82`, the shipped KNOWN FIRST-CUT LIMITATION comment, already characterises *k*:

> The filters above remove the common causes of permanent unauditability (a remote/deleted source, a
> half-uploaded resource, an unreachable cogmap), but **a citation that is readable, live, and simply
> never gets audited will re-head this queue every tick with the SAME `uncovered` count forever.**
> The real fix — a terminal "cannot assess" verdict, or a per-finding backoff — is deferred to the
> future reaper pass…

*k* has at least three members and the conjunct reaches one:

| member of *k* | reached by D7's conjunct? |
|---|---|
| source the principal cannot read (`019f9bfb`) | **yes** — the conjunct's entire content |
| readable, live, auditable, never verdicted | **no** — deferred to a reaper pass that does not exist |
| self-authored citations | ~empty for the auditor, which authors nothing (`instructions.md:162`) |

**So the conjunct does not bound *k*** — the dominant member is the one the shipped comment defers.
The ordering must therefore bound starvation structurally and independently of *k*, which is what
§3.4 does. The conjunct remains worth doing on its own merits (it stops offering work the gate will
404), but it is a **fast-follow, not a blocker**, and `019f9bfb` carries it.

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
| ~~**F9 — same-millisecond ordering unverified on prod**~~ | **DISSOLVED in rev. 3 — not verified, removed.** The predicate no longer compares event ids, so there is no generator to verify: see §3.1. Kept here rather than deleted because the *reasoning* is the reusable part. F9 was framed as "verify `pg_uuidv7` against Neon before shipping", and the framing was the error — it accepted a dependency the design never needed and promoted it to a permanent gate on every future edit to this predicate. Pete, 2026-07-26: *"why are we looking for `max(uuid)` anyway — we should already have record-level timestamps that do not rely on the ids."* The local measurement stands as a fact about PG18 and is now trivia: **49,951 same-millisecond pairs across 49 distinct milliseconds, 0 out of order.** |
| block mutated mid-session **[rev2]** | **Rev. 1 marked this "benign race; under-triggers by one tick" and then invalidated it in §3.2(b) without revisiting the row.** At the block grain the assessment is correct — a verdict written after the mutation sits above that block's own watermark by one tick and the next material event clears it. At rev. 1's `(finding, source)` grain it was *permanent*, because a later sibling audit put the watermark above the mutation forever. §3.1 restores "one tick." **Still deferred:** the residual one-tick under-trigger is not solved here. If it shows up in practice, write-action timestamps exist for every agent and document, so the options are ordering them outright or defining a suspect "within" window that emits a bump-for-re-audit event to refresh the watermark. Judged not worth solving today (Pete, 2026-07-26). |

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
2. **Structural inability → D7's conjunct** (§3.5). Self-authored citations, and citations whose
   source the principal cannot read (`019f9bfb`). These genuinely never gain coverage.
   **[rev2] They are not, however, "the population that starves the stale disjunct"** — rev. 1 said
   so and §3.5 refutes it from the shipped code's own comment: the dominant stuck population is the
   readable, live, auditable citation that simply never gets verdicted, which this conjunct does not
   reach. Self-authorship in particular should be ~empty for the auditor, which authors nothing
   (`instructions.md:162`). The conjunct is worth doing; it is not load-bearing for §3.4.

---

## 6. Witnesses **[rev2]**

Failing-before / passing-after, extending `citation_audits.rs`'s Task-5 family (`:1174+`) —
**CONFORM**, not a parallel suite.

1. **`sweep_reoffers_a_covered_finding_after_its_block_is_mutated`** — fully covered, then
   `block_mutated`. Today: never re-offered.
2. **`sweep_reoffers_a_covered_finding_after_its_cited_source_changes`** — the source-side arm;
   would pass vacuously if only clause one existed.
3. **`sweep_reoffers_a_two_block_finding_when_the_unaudited_pair_s_block_changes`** — witnesses the
   `block_wm IS NULL` arm of §3.2(b). Rev. 1's witness set used single-block fixtures throughout and
   **could not observe the blind spot it was written for**.
4. **`sweep_stale_is_scoped_to_the_sweeping_principal`** — witnesses §3.2(a): auditor A audits, block
   mutates, auditor B re-audits; the finding must still read stale *for A* and not *for B*.
5. **`sweep_reoffers_when_a_sibling_audit_lands_after_another_blocks_mutation`** — **new in rev. 2,
   and the one that falsifies rev. 1.** Two blocks citing a **shared** source: audit b1, mutate b1,
   audit b2. Must read stale. **Run it against rev. 1's `(finding, source)` body as well as the
   pre-change migration** — a witness that only fails against "the feature is absent" discriminates
   nothing, which is the bar W1 was cancelled for missing, and this one has a *second* wrong
   implementation available to bite.
6. **`sweep_ordering_gives_stale_findings_slots_under_a_stuck_backlog`** — **new in rev. 2.**
   Witnesses §3.4: seed more permanently-stuck findings (`uncovered >= 1`) than the cap, plus stale
   ones, and assert stale findings appear. Under today's `ORDER BY uncovered DESC` this returns none
   of them — the F4 starvation, which no other witness observes because every other fixture is
   smaller than the cap.

Must **stay green**:

- `sweep_omits_a_fully_covered_finding` (`:1374`) — verified unaffected (the fixture creates the
  source before the finding and audits last, so both cursors precede the watermark). A predicate that
  fires on a quiet finding is worse than the defect it replaces.
- **A principal who has never audited a finding must not see it as stale** — scenario 4 of §3.2(b),
  the guard on the `block_wm IS NULL` arm. Without a witness, a later simplification that drops
  `finding_wm IS NOT NULL` reads as a tidy-up and silently converts staleness into per-principal
  coverage.

---

## 7. Superseded

- **D3** (material-event allow-list) — not built; §2.2. Unanimously upheld on review.
- **D6** (citation-list payload), **D2 tier 2**, **C-7** — deferred; §4.
- **R10** — half withdrawn, half promoted to D7's conjunct; §5.
- **D7's conjunct** — **[rev2]** ratified *separable*; a fast-follow, not part of this PR. §3.5.
- Witness tasks **W1** (`019f9bce-9a7f-79b3-8b2b-77e9ee064730`) and
  **W3** (`019f9bcf-30c0-7772-8833-340b90f22f6b`) — cancelled 2026-07-25, reasons on the tasks.
- Task `019f9bd0-c9e9-7aa3-95ad-bc8b11325428` (material-event set) — closed by §2.2, not implemented.
- `019f9bfb-62e2-7c62-85b2-e309ac1b18c1` (source-visibility) — carries D7's conjunct. **[rev2]** Not
  in this PR: §3.5 shows the conjunct does not bound *k*, so the ordering key had to stand alone
  anyway. Worth doing on its own merits — it stops the queue offering work the gate will 404.
