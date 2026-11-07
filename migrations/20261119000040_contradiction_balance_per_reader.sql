-- The evidential-standing shape scores contradiction balance only over edges the reader can see.
--
-- WHAT CHANGES. resource_standing_shape computed contradiction_balance with
-- resource_contradiction_balance(p_finding): the signed weight sum of every live supports /
-- corroborates / contradicts edge on the finding, whoever could read it. On a finding with few
-- such edges that sum is close to a pointer: a single hidden `supports` edge reads as +1.0. The
-- shape now calls _resource_contradiction_balance(p_finding, p_principal_kind, p_principal_id),
-- which counts only edges the reader can see. `band` follows, since standing_band takes the balance.
--
-- WHAT DOES NOT. The other five components stay reader-independent. They are computed from the
-- finding's own provenance and its citation-audit trail, both of which every reader of the finding
-- already reads (resource_block_provenance, resource_citation_audit_trail). The 1-arg
-- resource_contradiction_balance stays as the producer behind refresh_resource_standing and the
-- kb_resource_standing memo, which no read path serves. Recorded in
-- internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md (amendment of
-- 2026-11-07).

CREATE FUNCTION _resource_contradiction_balance(
    p_finding        uuid,
    p_principal_kind text,
    p_principal_id   uuid)
RETURNS double precision
LANGUAGE sql
STABLE
AS $$
    SELECT coalesce(sum(CASE WHEN e.label = 'contradicts' THEN -e.weight ELSE e.weight END), 0)::double precision
    FROM kb_edges e
    WHERE e.edge_kind = 'express' AND NOT e.is_folded
      AND e.label = ANY (ARRAY['supports', 'corroborates', 'contradicts'])
      AND ((e.source_table = 'kb_resources' AND e.source_id = p_finding)
        OR (e.target_table = 'kb_resources' AND e.target_id = p_finding))
      AND (
            (p_principal_kind = 'profile'
               AND e.id IN (SELECT ev.edge_id FROM edges_visible_to(p_principal_id) ev))
         OR (p_principal_kind = 'cogmap'
               AND e.home_anchor_table = 'kb_cogmaps'
               AND e.home_anchor_id = p_principal_id
               AND e.source_table = 'kb_resources' AND e.target_table = 'kb_resources'
               AND e.source_id IN (SELECT a.resource_id FROM resources_accessible_to_cogmap(p_principal_id) a)
               AND e.target_id IN (SELECT a.resource_id FROM resources_accessible_to_cogmap(p_principal_id) a))
      );
$$;

COMMENT ON FUNCTION _resource_contradiction_balance(uuid, text, uuid) IS
$c$Supports minus contradicts over the finding's live express edges, counted only over edges the
principal can see (20261119000040). A vector sum of edge weights over the support/oppose label set
(supports, corroborates, contradicts), not a headcount. For a 'profile' an edge counts only if
edges_visible_to admits it: live, home readable, both endpoints readable. For 'cogmap' (a map
reading for itself) an edge counts only if it is homed in that map and both endpoints are in
resources_accessible_to_cogmap: there is no profile-shaped edge definition for a map, so the gate is
what the map's own principal indisputably reads. Fail-closed for any other kind. Read surface:
resource_standing_shape.$c$;

COMMENT ON FUNCTION resource_contradiction_balance(uuid) IS
$c$Supports minus contradicts over ALL of the finding's live express edges, whoever can read them --
a vector sum of edge weights over the support/oppose label set (supports, corroborates,
contradicts), not a headcount. PRODUCER ONLY since 20261119000040: it feeds refresh_resource_standing
and the kb_resource_standing memo, which no read path serves. A caller's read goes through
_resource_contradiction_balance, which counts only edges the reader can see.$c$;

CREATE OR REPLACE FUNCTION resource_standing_shape(p_finding uuid, p_principal_kind text, p_principal_id uuid)
RETURNS TABLE(finding_id uuid, citation_magnitude integer, audit_coverage integer,
              citation_quality double precision, contradiction_balance double precision,
              freshness double precision, r_parent double precision, band text)
LANGUAGE sql
STABLE
AS $$
    WITH gated AS (
        SELECT p_finding AS fid
        WHERE p_finding IN (SELECT resource_id FROM resources_readable_by(p_principal_kind, p_principal_id))
    ),
    components AS (
        SELECT g.fid,
               resource_citation_magnitude(g.fid)    AS magnitude,
               resource_audit_coverage(g.fid)        AS coverage,
               resource_citation_quality(g.fid)      AS quality,
               _resource_contradiction_balance(g.fid, p_principal_kind, p_principal_id) AS balance,
               resource_freshness(g.fid)             AS fresh,
               resource_r_parent(g.fid)              AS parent
        FROM gated g
    )
    SELECT c.fid, c.magnitude, c.coverage, c.quality, c.balance, c.fresh, c.parent,
           standing_band(c.magnitude, c.coverage, c.quality, c.balance, c.fresh)
    FROM components c;
$$;

COMMENT ON FUNCTION resource_standing_shape(uuid, text, uuid) IS
$c$The finding's evidential-standing shape: six components plus the lossy band, recomputed live at
read -- never the kb_resource_standing memo. Zero rows unless the caller reads the finding: the gate
(resources_readable_by) is inside this function, so an unreadable finding is indistinguishable from
an absent one. contradiction_balance (and so band) is computed per reader, over the edges the
caller can see (_resource_contradiction_balance, 20261119000040). The other five components are
reader-independent by decision: one value for every caller who can read the finding, computed from
the finding's own provenance and citation-audit trail, which every reader of the finding already
reads (internal/decisions/2026-08-26-stored-region-aggregates-are-region-truth.md).$c$;

COMMENT ON TABLE kb_resource_standing IS
$c$Memoized component scalars of a resource's evidential standing. NOT A READ SURFACE: no read path
serves these columns. resource_standing_shape recomputes every component live, and computes
contradiction_balance per reader (20261119000040), so the memo's contradiction_balance (over every
edge) must never be returned to a caller. The band is computed at read (standing_band); never store
one. A projection table: recomputable, refreshed by refresh_resource_standing, not
append-only-guarded.$c$;

SELECT declare_migration(
    20261119000040,
    'additive',
    'resource_standing_shape (GET /api/resources/{id}/evidence) scored contradiction_balance over '
    'every live supports/corroborates/contradicts edge on the finding, including edges the reader '
    'cannot see, so a single hidden edge moved the value. Adds _resource_contradiction_balance '
    '(finding, principal kind, principal), which counts only edges the reader can see, and points '
    'the shape at it; band follows. The 1-arg resource_contradiction_balance stays for the memo '
    'producer. Additive: one new function, one CREATE OR REPLACE at an unchanged signature, '
    'comments. Values only narrow, so an older binary sees the same columns.'
);
