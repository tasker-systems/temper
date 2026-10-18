-- The audit sweep offers a principal only the citations whose cited SOURCE it can read.
--
-- THE RULE. A citation is auditor work for a principal when that principal can read the finding AND
-- the source. The three per-principal functions the auditor's queue is built from now ask both:
-- `resource_stale_citations_multi` (the staleness set; `resource_stale_citations` and
-- `resource_has_stale_citation` inherit it), `resource_auditable_citations` (the job payload), and
-- `audit_drift_sweep`'s `uncovered` arm. One predicate, `resources_visible_to(p_principal)`, gates
-- both ends, so the finding gate and the source gate cannot drift apart.
--
-- THE AGGREGATES ARE NOT TOUCHED, AND MUST NOT BE. `resource_live_citations`,
-- `resource_citation_magnitude` and `resource_audit_coverage` are reader-independent by decision
-- (`20260903000010`). So the sweep computes its own READABLE magnitude and coverage inline, rather
-- than giving the shared producers a reader parameter. Readable coverage is `resource_audit_coverage`
-- restricted to readable sources, with the same block conjunct that function documents as
-- load-bearing; for a principal who can read every source the two agree.
--
-- THE HOIST IS LOAD-BEARING. `20260828000020` measured a per-finding `resources_visible_to` as 97% of
-- the sweep. Each function below derives the visible set once per call, in a MATERIALIZED CTE that
-- both gates read. Do not move the source gate into a per-finding helper.
--
-- `candidates`, the ordering key and the `stale` short-circuit are otherwise unchanged from
-- `20260828000020`; read that file and `20260727000010` for why each is shaped as it is. The
-- short-circuit's input is now the negation of the READABLE uncovered arm, so it still admits exactly
-- the findings the cheap arm did not.

-- ── 1. The staleness set ───────────────────────────────────────────────────────────────────────────
CREATE OR REPLACE FUNCTION resource_stale_citations_multi(p_findings uuid[], p_principal uuid)
RETURNS TABLE(finding_id uuid, block_id uuid, source_id uuid) LANGUAGE sql STABLE AS $$
    WITH visible AS MATERIALIZED (
        SELECT resource_id FROM resources_visible_to(p_principal)
    ),
    gated AS (
        SELECT f.fid
          FROM unnest(p_findings) AS f(fid)
         WHERE f.fid IN (SELECT resource_id FROM visible)
    )
    SELECT g.fid, lc.block_id, lc.source_id
      FROM gated g
      -- THE SOURCE GATE, applied to the citations before any arm below reads them: a citation is
      -- stale for a principal only when it can read the cited source.
      CROSS JOIN LATERAL (
          SELECT l.block_id, l.source_id
            FROM resource_live_citations(g.fid) l
           WHERE l.source_id IN (SELECT resource_id FROM visible)
      ) lc
      JOIN kb_content_blocks b  ON b.id  = lc.block_id
      JOIN kb_events         be ON be.id = b.last_event_id
      -- `ab.resource_id = g.fid` keeps each finding's watermark its own; see `20260828000020`.
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

`resource_stale_citations` is a one-element call into this, and `resource_has_stale_citation` is
EXISTS over that, so all three arities share this definition. Body rationale: `20260726000010`;
set grain: `20260828000020`.

Gated at both ends by `resources_visible_to(p_principal)`, derived once per call (`20261101400000`):
a finding the principal cannot read contributes zero rows rather than raising, and a citation whose
cited source it cannot read is never stale for it. Staleness still requires a prior audit by this
principal (`finding_wm IS NOT NULL`); a citation nobody weighed is unweighed, not stale.$c$;

-- The two delegating arities keep their bodies; their comments now say what they inherit.
COMMENT ON FUNCTION resource_stale_citations(uuid, uuid) IS
$c$Which of this finding's citations went stale for THIS principal.

A one-element call into `resource_stale_citations_multi`, which holds the body (`20260828000020`).
Since `20261101400000` that body gates the cited source as well as the finding.$c$;

COMMENT ON FUNCTION resource_has_stale_citation(uuid, uuid) IS
$c$Has anything material happened to this finding's citations since THIS principal weighed them?

EXISTS over `resource_stale_citations` (`20260727000050`), so it inherits that set's gates: since
`20261101400000`, the cited source as well as the finding.$c$;

-- ── 2. The auditor's unit of work ──────────────────────────────────────────────────────────────────
CREATE OR REPLACE FUNCTION resource_auditable_citations(p_finding uuid, p_principal uuid)
RETURNS TABLE(block_id uuid, source_id uuid) LANGUAGE sql STABLE AS $$
    WITH visible AS MATERIALIZED (
        SELECT resource_id FROM resources_visible_to(p_principal)
    ),
    gated AS (
        SELECT p_finding AS fid
        WHERE p_finding IN (SELECT resource_id FROM visible)
    )
    SELECT lc.block_id, lc.source_id
      FROM gated g
      CROSS JOIN LATERAL resource_live_citations(g.fid) lc
     WHERE lc.source_id IN (SELECT resource_id FROM visible)
       AND NOT EXISTS (
           SELECT 1
             FROM kb_citation_audits a
            WHERE a.block_id              = lc.block_id
              AND a.source_kind           = 'resource'
              AND a.source_id             = lc.source_id
              AND a.audited_by_profile_id = p_principal)
    UNION
    -- Source-gated inside the staleness set itself.
    SELECT s.block_id, s.source_id
      FROM resource_stale_citations(p_finding, p_principal) s;
$$;

COMMENT ON FUNCTION resource_auditable_citations(uuid, uuid) IS
$c$The citations of this finding THIS principal should weigh — the auditor's unit of work.

PER PRINCIPAL, which is NOT the coverage grain: `resource_audit_coverage` asks whether anyone
weighed a source. A citation another principal weighed is still returned here (cross-principal
audit is the premise); a citation this principal weighed is withheld until it goes stale.
A finding is never skipped because some of its citations are weighed — it is dispatched with the
remainder.

Only citations whose cited source this principal can read are returned (`20261101400000`), on both
arms. An unreadable finding returns zero rows, matching the write gate's zero-rows-to-404 dialect,
so this cannot become an existence oracle.$c$;

-- ── 3. The sweep ───────────────────────────────────────────────────────────────────────────────────
CREATE OR REPLACE FUNCTION audit_drift_sweep(p_principal uuid, p_limit int)
RETURNS TABLE(cogmap_id uuid, finding_id uuid, uncovered int)
LANGUAGE sql STABLE AS $$
    WITH visible AS MATERIALIZED (
        SELECT resource_id FROM resources_visible_to(p_principal)
    ),
    candidates AS (
        SELECT h.anchor_id AS cogmap_id, r.id AS finding_id
          FROM steward_candidate_cogmaps(p_principal) m
          JOIN kb_resource_homes h
            ON h.anchor_table = 'kb_cogmaps' AND h.anchor_id = m.cogmap_id
          JOIN kb_resources r ON r.id = h.resource_id
         WHERE r.is_active
           AND r.ingest_state = 'complete'
           -- The same predicate Task 6's gate runs, so the queue cannot offer what the gate refuses.
           AND r.id IN (SELECT resource_id FROM visible)
    ),
    -- MATERIALIZED IS LOAD-BEARING, NOT STYLE (`20260727000010`). `magnitude` and `coverage` here
    -- are READABLE: distinct sources this principal can read, and those of them anyone has audited at
    -- this citation. One pass over `resource_live_citations` yields both.
    scored AS MATERIALIZED (
        SELECT c.cogmap_id, c.finding_id, rc.magnitude, rc.coverage
          FROM candidates c
          CROSS JOIN LATERAL (
              SELECT count(DISTINCT lc.source_id)::int AS magnitude,
                     count(DISTINCT lc.source_id) FILTER (WHERE EXISTS (
                         SELECT 1 FROM kb_citation_audits a
                          WHERE a.block_id    = lc.block_id
                            AND a.source_kind = 'resource'
                            AND a.source_id   = lc.source_id))::int AS coverage
                FROM resource_live_citations(c.finding_id) lc
               WHERE lc.source_id IN (SELECT resource_id FROM visible)
          ) rc
    ),
    -- THE SHORT-CIRCUIT, set-wise (`20260828000020`): only findings the uncovered arm did not admit.
    stale AS (
        SELECT DISTINCT m.finding_id
          FROM resource_stale_citations_multi(
                   ARRAY(SELECT s.finding_id FROM scored s
                          WHERE s.magnitude > 0 AND s.coverage >= s.magnitude),
                   p_principal) m
    ),
    -- THE ORDERING KEY is unchanged from `20260726000010`; read its rationale there.
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

Both arms count only citations whose cited source this principal can read (`20261101400000`):
`uncovered` is readable magnitude minus readable coverage, computed inline because
`resource_citation_magnitude` and `resource_audit_coverage` are reader-independent by decision
(`20260903000010`). A finding all of whose sources are unreadable to this principal is not offered.
For a principal who can read every source, the row set and order are those of `20260828000020`.

Set-grain staleness and its cost: `20260828000020`. A stuck queue is not by itself evidence of a
missing terminal verdict: a dispatched finding whose session never completes re-heads the queue on
the next tick exactly as one the auditor weighed and declined does. `20260727000010`'s three
recorded caveats are carried forward unchanged.$c$;

SELECT declare_migration(
    20261101400000,
    'additive',
    'Gate the cited source, not only the finding, in the per-principal audit sweep layer: resource_stale_citations_multi (and so resource_stale_citations and resource_has_stale_citation), resource_auditable_citations, and audit_drift_sweep''s uncovered arm, each on resources_visible_to(p_principal) derived once per call. The reader-independent aggregates (resource_live_citations, resource_citation_magnitude, resource_audit_coverage) are untouched; the sweep computes its readable magnitude and coverage inline. Additive: three CREATE OR REPLACEs whose signatures and return columns are unchanged, plus two COMMENTs, so no deployed binary can disagree with this schema across the apply and no dependency is dropped; no query text changes, so no .sqlx entry moves.'
);
