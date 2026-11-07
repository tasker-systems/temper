-- public_within: the citation audit is split. The FINDING is judged on membership; the cited
-- SOURCE on reading. Public reach can be read and referenced, never acted on.
--
-- THIS DELIBERATELY REVERSES `20261101400000:7-8`, which gated both ends on one shared `visible` CTE
-- "so the finding gate and the source gate cannot drift apart". Grading a finding is a write about
-- it (a kb_events row in its home and a standing memo, 20260909000020:36-43), so a public reader
-- may not grade T's findings: the finding is gated on MEMBERSHIP. The cited source is a reference:
-- an auditor under A may judge its own team's finding that cites T, so the source is gated on
-- READING.
--
-- What keeps them from drifting is a shared rule, applied identically in the gate
-- (`authz/audit_gate.rs`, via `readback::is_resource_visible_by_membership`) and in the queue below,
-- and the test that the sweep never offers what the gate refuses (`the_audit_queue_matches_the_gate`).
-- The queue must use the gate's sets or it livelocks offering refused work; humans see the queue.
--
-- Each function derives both sets once per call, MATERIALIZED, as the single set was (the hoist is
-- load-bearing: `20260828000020` measured the per-finding form as 97% of the sweep). A cited source
-- is readable when it is in `member` or in `delta` (`resources_public_within`), never a second full
-- scan and never a copy of `member`: reading is exactly membership UNION that delta (the visibility
-- harness holds the equality). With no public_within row the delta is one table probe; for a
-- machine the fence inside contexts_public_within makes it empty.
--
-- Everything else in the three bodies is unchanged from `20261101400000`; read it for why each is
-- shaped as it is.

CREATE OR REPLACE FUNCTION resource_stale_citations_multi(p_findings uuid[], p_principal uuid)
RETURNS TABLE(finding_id uuid, block_id uuid, source_id uuid) LANGUAGE sql STABLE AS $$
    WITH member AS MATERIALIZED (
        SELECT resource_id FROM resources_visible_by_membership(p_principal)
    ),
    delta AS MATERIALIZED (
        SELECT resource_id FROM resources_public_within(p_principal)
    ),
    gated AS (
        SELECT f.fid
          FROM unnest(p_findings) AS f(fid)
         WHERE f.fid IN (SELECT resource_id FROM member)
    )
    SELECT g.fid, lc.block_id, lc.source_id
      FROM gated g
      -- THE SOURCE GATE, applied to the citations before any arm below reads them: a citation is
      -- stale for a principal only when it can read the cited source.
      CROSS JOIN LATERAL (
          SELECT l.block_id, l.source_id
            FROM resource_live_citations(g.fid) l
           WHERE (l.source_id IN (SELECT resource_id FROM member)
               OR l.source_id IN (SELECT resource_id FROM delta))
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

CREATE OR REPLACE FUNCTION resource_auditable_citations(p_finding uuid, p_principal uuid)
RETURNS TABLE(block_id uuid, source_id uuid) LANGUAGE sql STABLE AS $$
    WITH member AS MATERIALIZED (
        SELECT resource_id FROM resources_visible_by_membership(p_principal)
    ),
    delta AS MATERIALIZED (
        SELECT resource_id FROM resources_public_within(p_principal)
    ),
    gated AS (
        SELECT p_finding AS fid
        WHERE p_finding IN (SELECT resource_id FROM member)
    )
    SELECT lc.block_id, lc.source_id
      FROM gated g
      CROSS JOIN LATERAL resource_live_citations(g.fid) lc
     WHERE (lc.source_id IN (SELECT resource_id FROM member)
         OR lc.source_id IN (SELECT resource_id FROM delta))
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

CREATE OR REPLACE FUNCTION audit_drift_sweep(p_principal uuid, p_limit int)
RETURNS TABLE(cogmap_id uuid, finding_id uuid, uncovered int)
LANGUAGE sql STABLE AS $$
    WITH member AS MATERIALIZED (
        SELECT resource_id FROM resources_visible_by_membership(p_principal)
    ),
    delta AS MATERIALIZED (
        SELECT resource_id FROM resources_public_within(p_principal)
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
           AND r.id IN (SELECT resource_id FROM member)
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
               WHERE (lc.source_id IN (SELECT resource_id FROM member)
                   OR lc.source_id IN (SELECT resource_id FROM delta))
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

COMMENT ON FUNCTION resource_stale_citations_multi(uuid[], uuid) IS
$c$Which citations went stale for THIS principal, across a SET of findings — the staleness body.

`resource_stale_citations` is a one-element call into this, and `resource_has_stale_citation` is
EXISTS over that, so all three arities share this definition. Body rationale: `20260726000010`;
set grain: `20260828000020`.

Gated at both ends, each set derived once per call (`20261120000020`): the FINDING on membership
visibility (resources_visible_by_membership), the cited SOURCE on reading visibility (public_within
included for a person). A finding the principal cannot read by membership contributes zero rows
rather than raising, and a citation whose cited source it cannot read is never stale for it.
Staleness still requires a prior audit by this principal (`finding_wm IS NOT NULL`).$c$;

COMMENT ON FUNCTION resource_auditable_citations(uuid, uuid) IS
$c$The citations of this finding THIS principal should weigh — the auditor's unit of work.

PER PRINCIPAL, which is NOT the coverage grain: `resource_audit_coverage` asks whether anyone
weighed a source. A citation another principal weighed is still returned here (cross-principal
audit is the premise); a citation this principal weighed is withheld until it goes stale.
A finding is never skipped because some of its citations are weighed — it is dispatched with the
remainder.

The finding must be readable by MEMBERSHIP, and only citations whose cited source this principal
can READ are returned (`20261120000020`): the same two sets the write gate asks. An
unreadable finding returns zero rows, matching the write gate's zero-rows-to-404 dialect, so this
cannot become an existence oracle.$c$;

COMMENT ON FUNCTION audit_drift_sweep(uuid, int) IS
$c$Findings this principal should audit: incompletely covered, OR materially changed since it looked.

A finding is offered only when the principal reads it by MEMBERSHIP, the set the audit gate's
finding probe asks, so the queue never offers what the gate refuses (`20261120000020`). Both
arms count only citations whose cited source the principal can READ (public_within included for a
person): `uncovered` is readable magnitude minus readable coverage, computed inline because
`resource_citation_magnitude` and `resource_audit_coverage` are reader-independent by decision
(`20260903000010`). A finding all of whose sources are unreadable to this principal is not offered.$c$;

SELECT declare_migration(
    20261120000020,
    'additive',
    'public_within audit split: body-only CREATE OR REPLACE, signatures and return types unchanged, of '
    'resource_stale_citations_multi, resource_auditable_citations and audit_drift_sweep. Each '
    'gates the finding on resources_visible_by_membership and the cited source on reading '
    'visibility, deliberately reversing the single shared set of 20261101400000. For '
    'every principal not under the anchor of an effective public_within row the two sets are '
    'equal, and a row needs both sides, so every answer is unchanged until a row exists.'
);
