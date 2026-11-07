-- cogmap_regulation admits a telos edge only under the edge conjunction.
--
-- cogmap_regulation (behind cogmap_analytics) returns the map telos's live `express` edges with
-- readable targets. Same signature, CREATE OR REPLACE. Each edge must pass a per-principal edge gate:
--   * 'profile': the edge is in edges_visible_to(p_principal_id) -- live, home readable, both
--     endpoints readable. The one definition, not a restatement.
--   * 'cogmap': the map reading itself (cogmap_analytics admits that kind only when the principal
--     IS the map). No profile-shaped edge definition exists for a map principal, so the gate is
--     what the map indisputably reads: an edge homed in the map itself, whose source (the telos)
--     is in resources_accessible_to_cogmap. The target is gated by resources_readable_by.
--   * any other kind admits no edge. Fail closed.
-- Rationale sits in the COMMENT below, not in the body.

CREATE OR REPLACE FUNCTION cogmap_regulation(p_cogmap uuid, p_principal_kind text, p_principal_id uuid)
RETURNS TABLE(resource_id uuid, title text, body_text text, edge_label text)
LANGUAGE sql
STABLE
AS $$
    SELECT r.id, r.title, resource_body_text(r.id), e.label
    FROM kb_cogmaps c
    JOIN kb_edges e ON e.source_table = 'kb_resources'
                   AND e.source_id = c.telos_resource_id
                   AND e.edge_kind = 'express'
                   AND NOT e.is_folded
    JOIN kb_resources r ON r.id = e.target_id AND e.target_table = 'kb_resources'
    WHERE c.id = p_cogmap
      AND r.id IN (SELECT resource_id FROM resources_readable_by(p_principal_kind, p_principal_id))
      AND (
            (p_principal_kind = 'profile'
               AND e.id IN (SELECT ev.edge_id FROM edges_visible_to(p_principal_id) ev))
         OR (p_principal_kind = 'cogmap'
               AND p_principal_id = p_cogmap
               AND e.home_anchor_table = 'kb_cogmaps'
               AND e.home_anchor_id = p_cogmap
               AND c.telos_resource_id IN
                   (SELECT a.resource_id FROM resources_accessible_to_cogmap(p_cogmap) a))
      );
$$;

COMMENT ON FUNCTION cogmap_regulation(uuid, text, uuid) IS
$c$The map telos's live `express` edges and their readable targets (cogmap_analytics'
`regulation`). An edge is admitted only under the edge conjunction for the principal
(20261119000030). For a 'profile' that is edges_visible_to: live, home readable, both endpoints
readable. For 'cogmap' (the map reading itself) there is no profile-shaped edge definition, so it
is gated on what the map's own principal reads: an edge homed in the map itself, whose source is
in resources_accessible_to_cogmap, with the target gated by resources_readable_by. An edge homed
in a context therefore never surfaces to the map principal, however the map reaches that
context's resources. Any other kind admits nothing.$c$;

SELECT declare_migration(
    20261119000030,
    'additive',
    'cogmap_regulation (behind cogmap_analytics) admits a telos express edge only under the edge '
    'conjunction: for a profile, the edge is in edges_visible_to; for the map principal, the edge is '
    'homed in the map itself with an accessible source; any other principal kind admits no edge. '
    'CREATE OR REPLACE at the unchanged signature. Rows only disappear, so an older binary sees at '
    'most fewer regulation rows. Columns and shape are unchanged.'
);
