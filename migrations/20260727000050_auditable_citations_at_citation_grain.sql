-- The auditor's work, at the grain the auditor works in.
--
-- `audit_drift_sweep` selects findings; the auditor emits one verdict per citation. Nothing bridged
-- those grains, so a finding selected on its last uncovered citation was worked in full and every
-- already-weighed citation re-weighed. Measured on prod 2026-07-27: finding
-- 019f5cdd-ba0b-7fa2-90b1-e78ca7979254, 1 of 8 weighed, still swept at `uncovered: 7`.
--
-- Task 019fa5eb-2819-7e71-bd27-0d2875f60960 · goal 019f81e9-25f3-7fe1-b563-4acca2e391eb.

-- 1. The staleness SET — extracted verbatim from `resource_has_stale_citation`
--    (`20260726000010:51-98`), EXISTS removed, citation columns projected. That function's four
--    load-bearing decisions are documented there and unchanged here.
CREATE FUNCTION resource_stale_citations(p_finding uuid, p_principal uuid)
RETURNS TABLE(block_id uuid, source_id uuid) LANGUAGE sql STABLE AS $$
    WITH gated AS (
        SELECT p_finding AS fid
        WHERE p_finding IN (SELECT resource_id FROM resources_visible_to(p_principal))
    )
    SELECT lc.block_id, lc.source_id
      FROM gated g
      CROSS JOIN LATERAL resource_live_citations(g.fid) lc
      JOIN kb_content_blocks b  ON b.id  = lc.block_id
      JOIN kb_events         be ON be.id = b.last_event_id
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

COMMENT ON FUNCTION resource_stale_citations(uuid, uuid) IS
$c$Which of this finding's citations went stale for THIS principal — the set
`resource_has_stale_citation` always computed and discarded. Staleness requires a prior audit by
this principal (`finding_wm IS NOT NULL`); a citation nobody weighed is unweighed, not stale.
Body rationale: `20260726000010`.$c$;

-- 2. The boolean, now EXISTS over the set, so the two cannot answer differently. Same signature and
--    rows; `audit_drift_sweep` is untouched. REPLACE, not DROP — the sweep depends on it.
CREATE OR REPLACE FUNCTION resource_has_stale_citation(p_finding uuid, p_principal uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (SELECT 1 FROM resource_stale_citations(p_finding, p_principal));
$$;

COMMENT ON FUNCTION resource_has_stale_citation(uuid, uuid) IS
$c$Has anything material happened to this finding's citations since THIS principal weighed them?
Re-defined by `20260727000050` as EXISTS over `resource_stale_citations`, which now holds the body.
Behavior unchanged.$c$;

-- 3. What this principal should weigh: never weighed by it, or weighed and since stale.
--    UNION not UNION ALL — a citation can qualify on both arms, and offering it twice is the bug.
CREATE FUNCTION resource_auditable_citations(p_finding uuid, p_principal uuid)
RETURNS TABLE(block_id uuid, source_id uuid) LANGUAGE sql STABLE AS $$
    WITH gated AS (
        SELECT p_finding AS fid
        WHERE p_finding IN (SELECT resource_id FROM resources_visible_to(p_principal))
    )
    SELECT lc.block_id, lc.source_id
      FROM gated g
      CROSS JOIN LATERAL resource_live_citations(g.fid) lc
     WHERE NOT EXISTS (
           SELECT 1
             FROM kb_citation_audits a
            WHERE a.block_id              = lc.block_id
              AND a.source_kind           = 'resource'
              AND a.source_id             = lc.source_id
              AND a.audited_by_profile_id = p_principal)
    UNION
    SELECT s.block_id, s.source_id
      FROM resource_stale_citations(p_finding, p_principal) s;
$$;

COMMENT ON FUNCTION resource_auditable_citations(uuid, uuid) IS
$c$The citations of this finding THIS principal should weigh — the auditor's unit of work.

PER PRINCIPAL, which is NOT the coverage grain: `resource_audit_coverage` asks whether anyone
weighed a source. A citation another principal weighed is still returned here (cross-principal
audit is the premise); a citation this principal weighed is withheld until it goes stale.

A finding is never skipped because some of its citations are weighed — it is dispatched with the
remainder. Unreadable finding returns zero rows, matching the write gate's zero-rows-to-404
dialect, so this cannot become an existence oracle.$c$;
