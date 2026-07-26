-- Auditor tier 1: re-offer a finding when something MATERIAL happened since this principal audited
-- it, not only when the finding is incompletely covered.
--
-- Design: docs/superpowers/specs/2026-07-25-auditor-tier1-staleness-watermark-design.md (rev. 2)
--
-- THE DEFECT (spec §1). `audit_drift_sweep` selected on `coverage < magnitude`, which asks "is this
-- finding fully covered?" Once `coverage == magnitude` a finding NEVER re-entered the queue, so a
-- `block_mutated` could change an assertion's text while every verdict about the previous text stood
-- unrevisited forever. Staleness misreported as stability, against shipped code.
--
-- WHY THERE IS NO MATERIAL-EVENT ALLOW-LIST (spec §2.2). `kb_events` has no `resource_id`, so the
-- obvious design enumerates "material" event types and joins through each payload. That work is not
-- needed: the substrate already keeps the cursor. `kb_content_blocks.last_event_id` is bumped by
-- exactly three functions -- `_project_blocks`, `_project_block_mutated`, `_project_charter_set`
-- (from `pg_proc`, 2026-07-26; an earlier draft derived this from `grep` and named a fourth,
-- `_project_block_folded`, which does not exist). Nothing on the audit path writes
-- `kb_content_blocks`: `_project_citation_audited` writes only `kb_citation_audits`, and there are
-- no triggers on `kb_content_blocks`, `kb_citation_audits`, or `kb_block_provenance` (the database
-- has exactly three non-internal triggers, on `kb_events`, `kb_profiles`, and
-- `kb_principal_standing_events`). So `citation_audited` cannot make itself material and drive its
-- own re-audit loop, and the allow-list D3 introduced to prevent that cycle is unnecessary.
--
-- ADDITIVE except for the DROP+CREATE of `audit_drift_sweep`, whose signature and return columns are
-- unchanged so the positional call site keeps resolving. `20260724000130` sets that precedent (it
-- DROP+CREATEs `workflow_job_claim`); applied migrations are immutable, so an amendment is a new
-- file, never an edit.

-- ── 1. The index the source-side clause needs ────────────────────────────────────────────────────
-- Both existing `resource_id` indexes on `kb_content_blocks` are PARTIAL on `NOT is_folded`
-- (`idx_kb_content_blocks_resource`, `kb_content_blocks_resource_seq_live`), so an unfiltered probe
-- can use neither: with `enable_seqscan = off` the planner still reports `Seq Scan ... Disabled:
-- true`. That would be a correlated sequential scan of the largest table in the schema, once per
-- citation row, for every candidate. With this index the same probe is an `Index Only Scan`.
--
-- DO NOT "fix" that by adding `NOT sb.is_folded` to the predicate instead. Folding a source block IS
-- content removal -- the source deleted the passage you cited -- and `_project_charter_set` folds and
-- bumps the cursor in one statement. Filtering folded blocks makes exactly the change the auditor
-- most needs to see invisible. The predicate's omission of `is_folded` is load-bearing, and this
-- paragraph is the documentation it previously lacked.
CREATE INDEX idx_kb_content_blocks_resource_cursor
    ON kb_content_blocks (resource_id, last_event_id);

COMMENT ON INDEX idx_kb_content_blocks_resource_cursor IS
    'Non-partial (resource_id, last_event_id) — deliberately covers FOLDED blocks too, because '
    'folding a cited block is content removal and is precisely what `resource_has_stale_citation` '
    'must be able to see. The two partial `NOT is_folded` indexes cannot serve that probe.';

-- ── 2. The staleness predicate ───────────────────────────────────────────────────────────────────
CREATE FUNCTION resource_has_stale_citation(p_finding uuid, p_principal uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1
        FROM resource_live_citations(p_finding) lc
        JOIN kb_content_blocks b ON b.id = lc.block_id
        CROSS JOIN LATERAL (
            SELECT
                -- This principal's freshest audit anywhere on this (finding, source)...
                (array_agg(a.audited_by_event_id ORDER BY a.audited_by_event_id DESC))[1]
                    AS finding_wm,
                -- ...and its freshest audit of THIS citation specifically.
                (array_agg(a.audited_by_event_id ORDER BY a.audited_by_event_id DESC)
                     FILTER (WHERE a.block_id = lc.block_id))[1]
                    AS block_wm
              FROM kb_citation_audits a
              JOIN kb_content_blocks ab
                ON ab.id = a.block_id AND ab.resource_id = p_finding
             WHERE a.source_kind = 'resource'
               AND a.source_id   = lc.source_id
               AND a.audited_by_profile_id = p_principal
        ) w
        WHERE w.finding_wm IS NOT NULL
          AND ( w.block_wm IS NULL
             OR b.last_event_id > w.block_wm
             OR EXISTS (SELECT 1 FROM kb_content_blocks sb
                         WHERE sb.resource_id   = lc.source_id
                           AND sb.last_event_id > w.block_wm) )
    );
$$;

COMMENT ON FUNCTION resource_has_stale_citation(uuid, uuid) IS
$c$Has anything material happened to this finding's citations since THIS principal weighed them?

Reads the cursors the substrate already keeps -- `kb_content_blocks.last_event_id` for content and
`kb_citation_audits.audited_by_event_id` for the audit -- and compares them as UUIDv7, which is
time-sortable. No material-event allow-list is needed; see this migration's header for why the
`citation_audited` self-cycle is structurally impossible.

FOUR THINGS HERE ARE LOAD-BEARING. Each was arrived at by breaking the alternative.

1. `array_agg(... ORDER BY ... DESC)[1]`, NOT `max()` and NOT `ORDER BY ... LIMIT 1`.
   `max(uuid)` DOES NOT EXIST in PostgreSQL, and a `LANGUAGE sql` body parses at CREATE time, so
   that spelling fails at `sqlx migrate run`. The obvious repair is also wrong: a scalar subquery is
   pulled up and evaluated three times per citation row (`SubPlan 1` + `SubPlan 2` + `InitPlan 3`),
   the producer multiplication `20260724000130` forbids. `array_agg` yields one `Aggregate` node.

2. `a.audited_by_profile_id = p_principal` -- the watermark is PER PRINCIPAL.
   A global max picks the COVERAGE grain ("did anyone look?") while staleness protects the QUALITY
   grain ("is each principal's vote about the current text?"), and `20260724000210` separated those
   deliberately. Without this: A verdicts -1.0, the block is mutated, B reads the new text and
   verdicts +1.0; the global max takes B's event so nothing is stale, `coverage == magnitude` so
   nothing is uncovered, and the finding is terminal while A's verdict about deleted text still
   carries half of a one-vote-per-principal aggregate.

3. `block_wm` is the comparand; `finding_wm` only GUARDS.
   An audit is keyed `(block_id, source_kind, source_id)` -- block grain -- but
   `resource_audit_coverage` collapses that to `count(DISTINCT source_id)`, so auditing a source on
   one block marks it covered on all blocks. Coarsening the watermark to `(finding, source)` to
   match coverage recreates defect (2) one grain over: the latest audit across all blocks sits above
   another block's mutation and hides it permanently. Both pure grains leak, in opposite directions
   -- a compensation applied at a different grain than the defect leaks the other way. So the
   comparison uses `block_wm`, and the `block_wm IS NULL` arm covers the gap coverage leaves.

4. `w.finding_wm IS NOT NULL` -- do NOT remove this as a tidy-up.
   It restricts the `block_wm IS NULL` arm to a principal already engaged with this
   (finding, source). Without it every finding reads stale for every principal who has not yet
   audited it, silently converting staleness into per-principal COVERAGE -- a far larger behaviour
   change, and one the band's coverage-ratio gate already handles.

KNOWN AND ACCEPTED: an edit landing DURING an audit run still under-triggers by one tick (the
verdict is written after the mutation, so it sits above that block's cursor). The block grain makes
that a one-tick gap rather than the permanent blind spot the `(finding, source)` grain produced. If
it matters in practice, write-action timestamps exist for every agent and document: order them, or
define a suspect "within" window that emits a bump-for-re-audit event.$c$;

-- ── 3. Selection: `uncovered OR stale`, and an ordering key that can rank a class whose
--       `uncovered` is always 0 ─────────────────────────────────────────────────────────────────
DROP FUNCTION audit_drift_sweep(uuid, int);

CREATE FUNCTION audit_drift_sweep(p_principal uuid, p_limit int)
RETURNS TABLE(cogmap_id uuid, finding_id uuid, uncovered int)
LANGUAGE sql STABLE AS $$
    WITH candidates AS (
        SELECT h.anchor_id AS cogmap_id, r.id AS finding_id
          FROM steward_candidate_cogmaps(p_principal) m
          JOIN kb_resource_homes h
            ON h.anchor_table = 'kb_cogmaps' AND h.anchor_id = m.cogmap_id
          JOIN kb_resources r ON r.id = h.resource_id
         WHERE r.is_active
           AND r.ingest_state = 'complete'
           -- The same predicate Task 6's gate runs, so the queue cannot offer what the gate refuses.
           AND r.id IN (SELECT resource_id FROM resources_visible_to(p_principal))
    ),
    -- EACH PRODUCER RUNS ONCE PER CANDIDATE ROW (`20260724000130`). `stale` is the third producer
    -- and is computed here, beside the other two, never repeated in the WHERE or the ORDER BY.
    scored AS (
        SELECT c.cogmap_id, c.finding_id,
               resource_citation_magnitude(c.finding_id)              AS magnitude,
               resource_audit_coverage(c.finding_id)                  AS coverage,
               resource_has_stale_citation(c.finding_id, p_principal) AS stale
          FROM candidates c
    ),
    -- THE ORDERING KEY, and why it is not simply the old `ORDER BY uncovered DESC`.
    --
    -- A stale finding is by construction FULLY COVERED, so its `uncovered` is 0 -- the MINIMUM of
    -- `uncovered DESC`. Carried forward unchanged, every stale finding sorts behind every uncovered
    -- one. With `DEFAULT_AUDITOR_DISPATCH_CAP = 50` and k permanently-stuck findings sitting at
    -- `uncovered >= 1` forever, stale rows get 50 - k slots allocated by `finding_id` ASC -- UUIDv7,
    -- so oldest-first DETERMINISTICALLY, meaning newer stale findings are structurally excluded
    -- rather than delayed. At k >= 50 the new disjunct returns nothing, silently, forever: exactly
    -- the failure this migration exists to fix, reintroduced by its own ordering.
    --
    -- And k is NOT bounded by scoping `uncovered` to auditable citations, the fix that suggests
    -- itself. `20260724000130`'s own KNOWN-FIRST-CUT-LIMITATION names the dominant stuck population
    -- -- "a citation that is readable, live, and simply never gets audited" -- and defers it to a
    -- future reaper pass. So the ordering has to bound starvation structurally, on its own.
    --
    -- REJECTED ALTERNATIVE: a single composite axis, `uncovered + stale_citation_count`. Cleaner to
    -- read and it does not work -- a stuck finding and a stale finding both sit at >= 1, they tie,
    -- and the tie-break is `finding_id` ASC. Stuck findings are older BY CONSTRUCTION (they have
    -- been stuck), so they win every tie and k >= 50 starves the disjunct exactly as before.
    --
    -- What works: rank within each class, then interleave by rank. Self-balancing rather than
    -- slot-reserving -- if only three findings are stale you get three stale and the rest uncovered,
    -- no slots wasted -- and with NOTHING stale it is byte-identical to the ordering this replaces.
    -- Determinism survives because `rn`'s window ordering terminates in `finding_id`, which is what
    -- `auditor_service::drift_sweep` ("deterministic, principal-scoped sweep") and `group_by_cogmap`
    -- ("the cogmaps come out in the order their first finding appeared") both depend on.
    ranked AS (
        SELECT s.cogmap_id, s.finding_id, s.magnitude, s.coverage,
               (s.coverage < s.magnitude) AS is_uncovered,
               row_number() OVER (PARTITION BY (s.coverage < s.magnitude)
                                  ORDER BY (s.magnitude - s.coverage) DESC, s.finding_id) AS rn
          FROM scored s
         WHERE s.magnitude > 0
           AND (s.coverage < s.magnitude OR s.stale)
    )
    SELECT k.cogmap_id, k.finding_id, (k.magnitude - k.coverage) AS uncovered
      FROM ranked k
     ORDER BY k.rn, k.is_uncovered DESC, (k.magnitude - k.coverage) DESC, k.finding_id
     LIMIT p_limit;
$$;

COMMENT ON FUNCTION audit_drift_sweep(uuid, int) IS
$c$Findings this principal should audit: incompletely covered, OR materially changed since it looked.

Re-created by `20260726000010` to add the `stale` disjunct and an ordering key that can rank a class
whose `uncovered` is always 0. Signature and return columns are unchanged.

CORRECTION TO `20260724000130`'s KNOWN FIRST-CUT LIMITATION. That comment is accurate about coverage
being monotone -- `kb_citation_audits` has no supersession, so coverage never falls -- but its
conclusion, that such a citation "will re-head this queue every tick with the SAME `uncovered` count
forever", was drawn when `uncovered` was the ONLY route in. It no longer is. A finding that is fully
covered can now re-enter via `stale`, and the ordering above guarantees that class a share of the
cap regardless of how many permanently-stuck findings sit ahead of it. What that comment describes
as needing a reaper pass -- a terminal "cannot assess" verdict, or a per-finding backoff -- is still
unbuilt, and is still the reason the stuck population is unbounded.

NOT FIXED HERE, and each is its own task:
  * `resource_live_citations` has no `ingest_state` gate, though its own header quotes the rule. All
    three standing axes read it, so the blast radius is far wider than this change earns.
  * `block_mutated` is emitted without a content-hash comparison (`update_resource_in_tx` gates on
    `p.body.is_some()`, not "the body changed"), so a no-op round-trip through the documented
    show-edit-`cat` idiom fires staleness on byte-identical content.
  * The source-side clause fires on ANY block of the cited source, including a telos, which
    `_project_charter_set` folds and reinserts wholesale on every `charter_set`. Unquantified.
  * Same-millisecond UUIDv7 ordering is verified on PG18 (native `uuidv7()`) but NOT on PG17/Neon,
    which uses the `pg_uuidv7` extension -- a different generator. If it fills `rand_a` randomly,
    sub-millisecond comparisons there are arbitrary.$c$;
