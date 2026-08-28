-- Staleness moves to set grain, because the sweep's cost was never where anyone looked.
--
-- =================================================================================================
-- THE MEASUREMENT
-- =================================================================================================
--
-- Production, 2026-08-28, `EXPLAIN (ANALYZE, BUFFERS)` on `audit_drift_sweep(principal, 50)` — the
-- first such plan taken of this function:
--
--   Limit  (actual time=3123.803..3123.831 rows=15)   Buffers: shared hit=256972
--     CTE scored
--       ->  Hash Join  (actual time=14.446..83.635 rows=754)   Buffers: shared hit=20456
--     ->  CTE Scan on scored s  (actual time=609.266..3088.078 rows=15)
--           Filter: ((s.magnitude > 0) AND ((s.coverage < s.magnitude)
--                    OR resource_has_stale_citation(s.finding_id, '019fa583-…'::uuid)))
--           Rows Removed by Filter: 739
--   Execution Time: 3090.423 ms
--
-- `scored` -- the CTE `20260724000130` and `20260727000010` both treat as the cost, and which
-- `20260727000010` MATERIALIZED specifically to bound -- is **83.6 ms of 3,090**. The staleness
-- disjunct is 3,004 ms and 236,516 buffers: 97% of the time and 92% of the buffers, at 363 buffers
-- per evaluation.
--
-- =================================================================================================
-- WHY: THE SHORT-CIRCUIT `20260727000010` WAS BUILT ON NEVER FIRES
-- =================================================================================================
--
-- That migration moved `stale` out of `scored` and into the disjunct so it would be "reached only
-- when `s.coverage < s.magnitude` is false" and would "keep paying only for fully-covered findings."
-- The reasoning is right. The premise is not. Measured on the same day, for the sole auditor
-- principal:
--
--   candidates | magnitude > 0 | coverage < magnitude | fully covered | Σ magnitude | Σ coverage
--          754 |           651 |                    0 |           651 |       1,079 |      1,079
--
-- The cheap arm admits NOTHING, so the expensive arm runs for every candidate, every tick. And this
-- is not a transient state to wait out: `kb_citation_audits` has no supersession (`20260724000110`),
-- so coverage is MONOTONE, and full coverage is therefore the steady state of any corpus that has
-- been audited once -- which is the state an hourly cron spends essentially all of its life in.
-- `20260727000010`'s own 221-candidate fixture was measured on a corpus whose uncovered arm was not
-- empty, which is why the shape looked cheap there and is not cheap here.
--
-- The 363 buffers are named by the incumbent itself: `resource_stale_citations` (`20260727000050`)
-- opens with `WHERE p_finding IN (SELECT resource_id FROM resources_visible_to(p_principal))`,
-- recomputing the principal's whole visible-resource set from base tables -- once per finding.
--
-- =================================================================================================
-- THE FIX, AND WHAT IT DELIBERATELY IS NOT
-- =================================================================================================
--
-- ONE LINE OF THE PREDICATE CHANGES: the gate filters an ARRAY instead of a scalar, so
-- `resources_visible_to` is derived once per sweep instead of once per finding. Everything else is
-- `20260727000050`'s body verbatim, with `g.fid` where `p_finding` stood.
--
-- NOT a threshold, NOT a cursor, NOT a watermark, and NOT the terminal-verdict gap. Each was
-- considered against the measurement and rejected; the reasoning is in
-- `temper-artifacts:specs/2026-08-28-audit-drift-sweep-set-orientation-design.md` §6. The one worth
-- repeating here, because a future reader will reach for it exactly as this arc's task did:
-- `steward_drift_sweep` does NOT "cull before the expensive per-map work". Its
-- `WHERE d.new_resources >= p_threshold` filters the OUTPUT of the `CROSS JOIN LATERAL
-- steward_ingest_delta(...)` that IS the work -- structurally the same position as `p_limit`. A
-- threshold here would have bought nothing, and on the live corpus (every candidate at
-- `magnitude - coverage = 0`) would have returned zero rows at unchanged cost.
--
-- INVERTED, NOT FORKED. The body moves INTO the set form and the per-finding form becomes a
-- one-element call into it. `resource_has_stale_citation` is untouched and stays `EXISTS` over the
-- per-finding form. So all three arities keep answering identically BY CONSTRUCTION -- which is the
-- property `20260727000050` established when it moved the body the other way ("so the two cannot
-- answer differently"), preserved one arity further out. An inlined copy in the sweep would have
-- been faster to write and would have forked the predicate into two definitions nothing links.
--
-- VERIFIED AGAINST PRODUCTION BEFORE BEING WRITTEN. The set body below was executed over all 754
-- candidate findings and compared to `resource_stale_citations` called per finding, at
-- (finding, block, source) grain:
--
--   incumbent_rows | setform_rows | only_incumbent | only_setform
--               15 |           15 |              0 |            0
--
--   at 60 ms / 27,762 buffers, against the incumbent sweep's 3,090 ms / 256,972.
--
-- ADDITIVE. One new function; two `CREATE OR REPLACE`s whose signatures and return columns are
-- unchanged, so no deployed binary can disagree with this schema across the apply, and no dependency
-- is dropped. `auditor_service::drift_sweep` calls `audit_drift_sweep($1, $2)` positionally and keeps
-- resolving; no `.sqlx` entry changes because no query text does.

-- ── 1. The staleness set, at set grain ──────────────────────────────────────────────────────────
-- Body from `20260727000050`. Its four load-bearing decisions are documented at `20260726000010` and
-- are unchanged here: staleness requires a prior audit BY THIS PRINCIPAL (`finding_wm IS NOT NULL`);
-- the finding-side clause; the source-side correlated EXISTS; and the visibility gate.
CREATE FUNCTION resource_stale_citations_multi(p_findings uuid[], p_principal uuid)
RETURNS TABLE(finding_id uuid, block_id uuid, source_id uuid) LANGUAGE sql STABLE AS $$
    -- THE ONLY CHANGE FROM `20260727000050`. Filtering `unnest(p_findings)` rather than a scalar is
    -- what makes `resources_visible_to` a once-per-call derivation instead of a once-per-finding
    -- one -- and that single relocation is the entire 3,004 ms. It is a HOIST, not a relaxation:
    -- every finding still passes the same predicate, and `stale_predicate_refuses_a_finding_the_
    -- principal_cannot_read` asserts it at both arities.
    WITH gated AS (
        SELECT f.fid
          FROM unnest(p_findings) AS f(fid)
         WHERE f.fid IN (SELECT resource_id FROM resources_visible_to(p_principal))
    )
    SELECT g.fid, lc.block_id, lc.source_id
      FROM gated g
      CROSS JOIN LATERAL resource_live_citations(g.fid) lc
      JOIN kb_content_blocks b  ON b.id  = lc.block_id
      JOIN kb_events         be ON be.id = b.last_event_id
      -- `ab.resource_id = g.fid` is the per-row correlation that was a constant at scalar arity.
      -- Losing it would pool every array member's audits into one watermark and make a quiet finding
      -- read stale off its neighbour's mutation --
      -- `the_set_form_attributes_each_stale_citation_to_its_own_finding` is the witness.
      CROSS JOIN LATERAL (
          SELECT
              max(a.created)                                         AS finding_wm,
              max(a.created) FILTER (WHERE a.block_id = lc.block_id) AS block_wm
            FROM kb_citation_audits a
            JOIN kb_content_blocks ab
              ON ab.id = a.block_id AND ab.resource_id = g.fid
           WHERE a.source_kind = 'resource'
             AND a.source_id   = lc.source_id
             AND a.audited_by_profile_id = p_principal
      ) w
     WHERE w.finding_wm IS NOT NULL
       AND ( w.block_wm IS NULL
          OR be.occurred_at > w.block_wm
          OR EXISTS (SELECT 1 FROM kb_content_blocks sb
                       JOIN kb_events se ON se.id = sb.last_event_id
                      WHERE sb.resource_id = lc.source_id
                        AND se.occurred_at > w.block_wm) );
$$;

COMMENT ON FUNCTION resource_stale_citations_multi(uuid[], uuid) IS
$c$Which citations went stale for THIS principal, across a SET of findings — the staleness body.

Holds the body `20260727000050` gave `resource_stale_citations`, which is now a one-element call into
this. The only change is that the visibility gate filters `unnest(p_findings)` rather than a scalar,
so `resources_visible_to` is derived once per call instead of once per finding — measured on
production 2026-08-28 as 97% of `audit_drift_sweep`'s runtime and 92% of its buffers.

Body rationale: `20260726000010`. Staleness still requires a prior audit by this principal
(`finding_wm IS NOT NULL`); a citation nobody weighed is unweighed, not stale.

An unreadable finding in the array contributes zero rows rather than raising — the array arity is not
a way around the gate the scalar arity enforces.$c$;

-- ── 2. The per-finding form, inverted ───────────────────────────────────────────────────────────
-- `CREATE OR REPLACE`, not DROP+CREATE: `resource_has_stale_citation` and
-- `resource_auditable_citations` both depend on this, and the signature and return columns are
-- unchanged. `resource_has_stale_citation` is NOT touched -- it stays `EXISTS` over this, so the
-- boolean, the per-finding set and the set form remain one definition.
CREATE OR REPLACE FUNCTION resource_stale_citations(p_finding uuid, p_principal uuid)
RETURNS TABLE(block_id uuid, source_id uuid) LANGUAGE sql STABLE AS $$
    SELECT m.block_id, m.source_id
      FROM resource_stale_citations_multi(ARRAY[p_finding], p_principal) m;
$$;

COMMENT ON FUNCTION resource_stale_citations(uuid, uuid) IS
$c$Which of this finding's citations went stale for THIS principal.

Re-defined by `20260828000020` as a one-element call into `resource_stale_citations_multi`, which now
holds the body — the same inversion `20260727000050` performed when it made
`resource_has_stale_citation` an EXISTS over this. Behavior unchanged; the three arities agree by
construction rather than by review, and `the_staleness_boolean_agrees_with_the_staleness_set` asserts
all three across an audit and a mutation.$c$;

-- ── 3. The sweep ────────────────────────────────────────────────────────────────────────────────
-- `candidates` and `scored` are VERBATIM from `20260727000010`, comments included. Read that file
-- for why `scored` is MATERIALIZED and why each of `candidates`' filters is there -- both remain
-- exactly as load-bearing as they were.
CREATE OR REPLACE FUNCTION audit_drift_sweep(p_principal uuid, p_limit int)
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
    -- MATERIALIZED IS LOAD-BEARING, NOT STYLE. Without it this CTE is inlined and both producers are
    -- re-evaluated at every downstream reference -- see `20260727000010`'s header for the
    -- measurement. It matters MORE now, not less: `stale` below adds a third reference to `scored`.
    scored AS MATERIALIZED (
        SELECT c.cogmap_id, c.finding_id,
               resource_citation_magnitude(c.finding_id) AS magnitude,
               resource_audit_coverage(c.finding_id)     AS coverage
          FROM candidates c
    ),
    -- THE SHORT-CIRCUIT, PRESERVED SET-WISE RATHER THAN ABANDONED. `20260727000010` put `stale` in
    -- the disjunct so it was reached only for findings the cheap arm had NOT already admitted. The
    -- array below carries exactly that set -- `coverage >= magnitude` is the negation of the cheap
    -- arm -- so the intent survives verbatim while the once-per-row scalar call does not. What
    -- changes is that the saving no longer depends on the planner's OR-arm ordering, which
    -- `20260727000010` correctly noted is a behaviour and not a guarantee.
    stale AS (
        SELECT DISTINCT m.finding_id
          FROM resource_stale_citations_multi(
                   ARRAY(SELECT s.finding_id FROM scored s
                          WHERE s.magnitude > 0 AND s.coverage >= s.magnitude),
                   p_principal) m
    ),
    -- THE ORDERING KEY is unchanged from `20260726000010`; read its rationale there. In short: a
    -- stale finding is fully covered by construction, so its `uncovered` is 0 -- the minimum of the
    -- old `uncovered DESC` -- and would sort behind every stuck finding forever. Ranking within each
    -- class and interleaving by rank bounds that starvation structurally.
    ranked AS (
        SELECT s.cogmap_id, s.finding_id, s.magnitude, s.coverage,
               (s.coverage < s.magnitude) AS is_uncovered,
               row_number() OVER (PARTITION BY (s.coverage < s.magnitude)
                                  ORDER BY (s.magnitude - s.coverage) DESC, s.finding_id) AS rn
          FROM scored s
         WHERE s.magnitude > 0
           AND ( s.coverage < s.magnitude
              OR s.finding_id IN (SELECT finding_id FROM stale) )
    )
    SELECT k.cogmap_id, k.finding_id, (k.magnitude - k.coverage) AS uncovered
      FROM ranked k
     ORDER BY k.rn, k.is_uncovered DESC, (k.magnitude - k.coverage) DESC, k.finding_id
     LIMIT p_limit;
$$;

COMMENT ON FUNCTION audit_drift_sweep(uuid, int) IS
$c$Findings this principal should audit: incompletely covered, OR materially changed since it looked.

Replaced by `20260828000020`, which changed COST ONLY: the per-row
`resource_has_stale_citation(s.finding_id, p_principal)` in the disjunct became a semi-join against
`resource_stale_citations_multi` over exactly the set the old short-circuit admitted. Signature,
return columns, row set and row ORDER are unchanged; measured on production as 3,090 ms / 256,972
buffers -> 60 ms / 27,762, identical rows at (finding, block, source) grain across all 754 candidates.

CORRECTION TO THIS FUNCTION'S OWN PRIOR COMMENTS, and it cuts the other way from `20260727000010`'s.
`20260724000130` and `20260727000010` both name a missing terminal "cannot assess" verdict, or a
per-finding backoff, as "the reason the stuck population is unbounded". That reasoning still stands on
its own terms and the gap is still unbuilt. But it is NOT what the production queue evidences. Measured
2026-08-28: 143 dead auditor jobs, zero verdicts written since 2026-08-09, and every model call
returning HTTP 402 from the AI Gateway for want of budget. The auditor never reached a judgment, so it
never declined to record one — those dead jobs are an optional subsystem asked to run more often than
it is funded for, and must not be cited as evidence for the verdict gap. Nor would closing that gap
have removed one buffer of the cost above: the 3,004 ms was staleness over the 651 FULLY COVERED
findings, while the stuck population was 15.

STILL NOT FIXED HERE, and each is still its own task:
  * `resource_live_citations` has no `ingest_state` gate on the FINDING side (the source side gained
    one in `20260727000020`).
  * The source-side clause fires on ANY block of the cited source, including a telos.
  * An edit landing DURING an audit run still under-triggers by one tick.$c$;

SELECT declare_migration(
    20260828000020,
    'additive',
    'Move the staleness predicate to set grain, because audit_drift_sweep spent 97% of its runtime and 92% of its buffers calling resource_has_stale_citation once per candidate. New resource_stale_citations_multi(uuid[], uuid) holds the body from 20260727000050 with ONE change -- the resources_visible_to gate filters unnest(p_findings) rather than a scalar, so it is derived once per sweep instead of once per finding. resource_stale_citations becomes a one-element call into it, and resource_has_stale_citation is untouched, so the boolean, the per-finding set and the set form remain one definition. audit_drift_sweep swaps its per-row scalar call for a semi-join over exactly the set the old short-circuit admitted (coverage >= magnitude), preserving that intent without depending on planner OR-arm ordering. The short-circuit never fired in production: coverage is monotone under an append-only trail, so full coverage is the steady state, and 0 of 651 candidates took the cheap arm. Measured on prod 2026-08-28: 3,090ms/256,972 buffers -> 60ms/27,762, identical rows at (finding, block, source) grain across all 754 candidates. Additive: one new function and two CREATE OR REPLACEs whose signatures and return columns are unchanged, so no deployed binary can disagree with this schema across the apply and no dependency is dropped; no query text changes, so no .sqlx entry moves.'
);
