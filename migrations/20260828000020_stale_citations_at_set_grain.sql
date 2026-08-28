-- Staleness moves to set grain: the sweep called a scalar predicate once per candidate row.
--
-- THE COST. `EXPLAIN (ANALYZE, BUFFERS)`, production, 2026-08-28: the `scored` CTE that
-- `20260724000130` and `20260727000010` both treat as the cost is 83.6 ms of 3,090. The staleness
-- disjunct is 3,004 ms and 236,516 buffers -- 97% of the runtime, 92% of the buffers. Its 363
-- buffers per evaluation are named by the incumbent itself: `resource_stale_citations`
-- (`20260727000050`) re-derives `resources_visible_to` from base tables once per finding.
--
-- WHY IT WENT UNSEEN, AND WHAT NOT TO UNDO. `20260727000010` moved `stale` into the disjunct so it
-- would be reached only for findings the cheap `coverage < magnitude` arm had not already admitted.
-- That short-circuit does not fire: `kb_citation_audits` has no supersession (`20260724000110`), so
-- coverage is MONOTONE and full coverage is the steady state of any corpus audited once -- which is
-- where an hourly cron lives. Measured the same day: 0 of 651 candidates took the cheap arm. The
-- `stale` CTE below carries that short-circuit SET-WISE; do not "restore" the scalar form.
--
-- NOT A BOUND. A threshold, a cursor, a watermark and the terminal-verdict gap were each considered
-- against the measurement and rejected -- argued in `temper-artifacts:specs/2026-08-28-audit-drift-
-- sweep-set-orientation-design.md`. One correction belongs here, because the next reader will reach
-- for it: `steward_drift_sweep` does NOT cull before its expensive per-map work. Its
-- `WHERE d.new_resources >= p_threshold` filters the OUTPUT of the `CROSS JOIN LATERAL
-- steward_ingest_delta(...)` that IS the work -- structurally the same position as `p_limit`.
--
-- INVERTED, NOT FORKED. The body moves into the set form; the per-finding form becomes a
-- one-element call into it and `resource_has_stale_citation` is untouched, so all three arities
-- agree by construction -- the property `20260727000050` established, one arity further out.
-- Verified against production over all 754 candidates before being written: identical rows at
-- (finding, block, source) grain, at 3,090 ms / 256,972 buffers -> 60 ms / 27,762.

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
its own terms and the gap is still unbuilt. But a stuck queue is NOT evidence for it. When the auditor's
sessions cannot complete for a reason upstream of any judgment — the agent runtime being unavailable,
whatever the cause — every finding it was dispatched re-heads the queue on the next tick, and that
looks identical to a finding the auditor examined and declined. Only the second is what those comments
describe. Before citing a stuck population as evidence for the verdict gap, establish that the auditor
actually reached a judgment.

Nor would closing that gap have removed one buffer of the cost above: the 3,004 ms was staleness over
the 651 FULLY COVERED findings, and a stuck population is a rounding error beside that.

`20260727000010`'s three recorded caveats are carried forward unchanged; read them there rather than
here, since nothing in this migration touches any of them.$c$;

SELECT declare_migration(
    20260828000020,
    'additive',
    'Move the staleness predicate to set grain, because audit_drift_sweep spent 97% of its runtime and 92% of its buffers calling resource_has_stale_citation once per candidate. New resource_stale_citations_multi(uuid[], uuid) holds the body from 20260727000050 with ONE change -- the resources_visible_to gate filters unnest(p_findings) rather than a scalar, so it is derived once per sweep instead of once per finding. resource_stale_citations becomes a one-element call into it, and resource_has_stale_citation is untouched, so the boolean, the per-finding set and the set form remain one definition. audit_drift_sweep swaps its per-row scalar call for a semi-join over exactly the set the old short-circuit admitted (coverage >= magnitude), preserving that intent without depending on planner OR-arm ordering. The short-circuit never fired in production: coverage is monotone under an append-only trail, so full coverage is the steady state, and 0 of 651 candidates took the cheap arm. Measured on prod 2026-08-28: 3,090ms/256,972 buffers -> 60ms/27,762, identical rows at (finding, block, source) grain across all 754 candidates. Additive: one new function and two CREATE OR REPLACEs whose signatures and return columns are unchanged, so no deployed binary can disagree with this schema across the apply and no dependency is dropped; no query text changes, so no .sqlx entry moves.'
);
