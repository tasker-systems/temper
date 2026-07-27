-- `audit_drift_sweep`: make the once-per-candidate-row rule true, instead of only claimed.
--
-- THE DEFECT. `20260726000010` asserts, in capitals, that each producer runs once per candidate row.
-- It does not. `scored` is referenced exactly once, so PostgreSQL inlines it (PG12+ inlines a
-- single-reference, non-recursive, side-effect-free CTE unless it is declared MATERIALIZED), and
-- `s.magnitude` / `s.coverage` are then substituted at every downstream site: the `ranked` target
-- list, `is_uncovered`, the window ORDER BY, and two WHERE conjuncts. Measured on a 221-candidate
-- fixture (2026-07-27), counting real calls with a `nextval`-instrumented copy of each producer:
--
--   shape                                    magnitude  coverage  stale   median
--   previous sweep (`20260724000130`)             2.36      1.36      -    17.8ms
--   merged sweep   (`20260726000010`)             4.62      3.62   0.45    62.4ms
--   merged + `scored AS MATERIALIZED` only        1.00      1.00   1.00    57.6ms
--   this migration                                1.00      1.00   0.45    31.7ms
--
-- Not a correctness bug -- all three producers are STABLE and every shape returns the same rows in
-- the same order (verified positionally, at the full limit and at `LIMIT 50`). It is a cost bug and
-- a comment-accuracy bug, and `20260724000130`, cited as the authority for the rule, breaks it too.
--
-- THE TRAP, and why `scored AS MATERIALIZED` ALONE IS NOT THE FIX. Materializing `scored` with
-- `stale` still inside it fixes the multiplication and buys almost nothing (62.4ms -> 57.6ms),
-- because it also destroys a short-circuit: `resource_has_stale_citation` is far the most expensive
-- of the three -- it runs `resources_visible_to`, then joins blocks to events per citation, then a
-- correlated EXISTS over the source's blocks -- and inside `scored` it must be computed for EVERY
-- candidate. Left in the WHERE it is reached only when `s.coverage < s.magnitude` is false. So this
-- migration does BOTH halves: materialize `scored` to bound magnitude/coverage at one call per row,
-- and move `stale` to the disjunct so it keeps paying only for fully-covered findings.
--
-- A SINGLE WHERE REFERENCE DOES NOT REPEAT A PRODUCER. `20260726000010`'s comment put `stale` in
-- `scored` "never repeated in the WHERE" -- correct while `scored` was inlined, since a WHERE
-- reference would have been one more substitution site. Under MATERIALIZED that reasoning inverts:
-- `scored` is computed once, so a producer named once in the WHERE runs AT MOST once per candidate
-- row, which is what the rule asks. Materializing and leaving `stale` in `scored` is the strictly
-- worse of the two ways to satisfy it.
--
-- The short-circuit is a planner behaviour, not a guarantee -- PostgreSQL orders OR arms by cost and
-- is free to choose otherwise. That is why `MATERIALIZED` is here and not just the WHERE move: it
-- makes one-call-per-row the floor. If the short-circuit is ever lost this degrades to the 57.6ms
-- row above, never to the 62.4ms one.
--
-- ADDITIVE. `CREATE OR REPLACE`, not the DROP+CREATE `20260726000010` used: the signature and return
-- columns are unchanged, so replacing in place leaves no window in which the function is absent and
-- drops no dependencies. The positional call site at
-- `crates/temper-services/src/services/auditor_service.rs:60` keeps resolving either way.

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
    -- re-evaluated at every downstream reference -- see this migration's header for the measurement.
    scored AS MATERIALIZED (
        SELECT c.cogmap_id, c.finding_id,
               resource_citation_magnitude(c.finding_id) AS magnitude,
               resource_audit_coverage(c.finding_id)     AS coverage
          FROM candidates c
    ),
    -- THE ORDERING KEY is unchanged from `20260726000010`; read its rationale there. In short: a
    -- stale finding is fully covered by construction, so its `uncovered` is 0 -- the minimum of the
    -- old `uncovered DESC` -- and would sort behind every stuck finding forever. Ranking within each
    -- class and interleaving by rank bounds that starvation structurally, and is byte-identical to
    -- the old ordering when nothing is stale.
    --
    -- `resource_has_stale_citation` is called HERE rather than in `scored` so the `OR` can
    -- short-circuit past it whenever the cheap arm already admitted the row. One reference, so at
    -- most one call per candidate row.
    ranked AS (
        SELECT s.cogmap_id, s.finding_id, s.magnitude, s.coverage,
               (s.coverage < s.magnitude) AS is_uncovered,
               row_number() OVER (PARTITION BY (s.coverage < s.magnitude)
                                  ORDER BY (s.magnitude - s.coverage) DESC, s.finding_id) AS rn
          FROM scored s
         WHERE s.magnitude > 0
           AND ( s.coverage < s.magnitude
              OR resource_has_stale_citation(s.finding_id, p_principal) )
    )
    SELECT k.cogmap_id, k.finding_id, (k.magnitude - k.coverage) AS uncovered
      FROM ranked k
     ORDER BY k.rn, k.is_uncovered DESC, (k.magnitude - k.coverage) DESC, k.finding_id
     LIMIT p_limit;
$$;

COMMENT ON FUNCTION audit_drift_sweep(uuid, int) IS
$c$Findings this principal should audit: incompletely covered, OR materially changed since it looked.

Re-created by `20260726000010` to add the `stale` disjunct and an ordering key that can rank a class
whose `uncovered` is always 0. Replaced by `20260727000010`, which changed cost only: `scored` is now
MATERIALIZED so `resource_citation_magnitude`/`resource_audit_coverage` run once per candidate row
rather than 4.6/3.6 times, and `resource_has_stale_citation` moved into the disjunct so it is reached
only for findings the cheap arm did not already admit. Signature, return columns, row set and row
ORDER are unchanged.

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
  * The source-side clause fires on ANY block of the cited source, including a telos, which
    `_project_charter_set` folds and reinserts wholesale on every `charter_set`. Unquantified.
  * An edit landing DURING an audit run still under-triggers by one tick -- the verdict is written
    after the mutation, so it sits above that block's cursor. Bounded to one tick, and it clears on
    the next material event. Write-action timestamps exist for every agent and document if it ever
    needs solving.$c$;
