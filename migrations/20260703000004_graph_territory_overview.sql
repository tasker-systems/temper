-- R2 territory overview: region + context territories, orphan salient nodes
-- (sparsity fallback = edge-degree), and aggregated cross-territory bridges.

CREATE FUNCTION graph_region_territories(
    p_profile uuid, p_team uuid, p_lens uuid
) RETURNS TABLE(region_id uuid, cogmap_id uuid, label text, member_count int, salience double precision)
LANGUAGE sql STABLE AS $$
    SELECT reg.id, reg.cogmap_id, reg.label, reg.member_count, reg.salience
    FROM kb_cogmap_regions reg
    JOIN kb_team_cogmaps tc ON tc.cogmap_id = reg.cogmap_id
    JOIN team_ancestors(p_team) a ON a.team_id = tc.team_id
    WHERE NOT reg.is_folded
      AND reg.lens_id = p_lens
      AND cogmap_readable_by_profile(p_profile, reg.cogmap_id);
$$;

CREATE FUNCTION graph_context_territories(
    p_profile uuid, p_team uuid
) RETURNS TABLE(context_id uuid, label text, member_count int) LANGUAGE sql STABLE AS $$
    WITH scope AS (SELECT resource_id FROM resources_in_team_scope(p_profile, p_team)),
    homed AS (
        SELECT h.anchor_id AS context_id, h.resource_id
        FROM kb_resource_homes h
        JOIN scope s ON s.resource_id = h.resource_id
        WHERE h.anchor_table = 'kb_contexts'
    )
    SELECT c.id, c.name, count(homed.resource_id)::int
    FROM homed
    JOIN kb_contexts c ON c.id = homed.context_id
    GROUP BY c.id, c.name;
$$;

-- Orphan salient nodes: in-scope resources whose cogmap home has NO live region,
-- ranked by visible edge-degree. doc_type LEFT-joined (nullable). Bounded in Rust.
CREATE FUNCTION graph_orphan_salient_nodes(
    p_profile uuid, p_team uuid
) RETURNS TABLE(id uuid, title text, doc_type text, degree int, anchor_id uuid)
LANGUAGE sql STABLE AS $$
    WITH scope AS (SELECT resource_id FROM resources_in_team_scope(p_profile, p_team)),
    doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
        FROM kb_properties p
        WHERE p.owner_table='kb_resources' AND p.property_key='doc_type' AND NOT p.is_folded
    ),
    cogmap_homed AS (
        SELECT h.resource_id, h.anchor_id AS cogmap_id
        FROM kb_resource_homes h
        JOIN scope s ON s.resource_id = h.resource_id
        WHERE h.anchor_table = 'kb_cogmaps'
    ),
    region_maps AS (
        SELECT DISTINCT cogmap_id FROM kb_cogmap_regions WHERE NOT is_folded
    )
    SELECT r.id, r.title, d.dt AS doc_type,
           deg.degree, ch.cogmap_id
    FROM cogmap_homed ch
    LEFT JOIN region_maps rm ON rm.cogmap_id = ch.cogmap_id
    JOIN kb_resources r ON r.id = ch.resource_id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    LEFT JOIN LATERAL (
        SELECT count(*)::int AS degree
        FROM kb_edges e
        JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
        WHERE (e.source_id = r.id OR e.target_id = r.id)
    ) deg ON true
    WHERE rm.cogmap_id IS NULL  -- home cogmap has no materialized region
    ORDER BY deg.degree DESC;
$$;

-- Aggregated cross-territory bridges: visible edges whose endpoints' cogmap homes differ.
CREATE FUNCTION graph_territory_bridges(
    p_profile uuid, p_team uuid
) RETURNS TABLE(source_territory uuid, target_territory uuid, edge_count int)
LANGUAGE sql STABLE AS $$
    WITH scope AS (SELECT resource_id FROM resources_in_team_scope(p_profile, p_team)),
    homed AS (
        SELECT h.resource_id, h.anchor_id AS territory
        FROM kb_resource_homes h
        JOIN scope s ON s.resource_id = h.resource_id
        WHERE h.anchor_table = 'kb_cogmaps'
    )
    SELECT LEAST(sh.territory, th.territory), GREATEST(sh.territory, th.territory), count(*)::int
    FROM kb_edges e
    JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
    JOIN homed sh ON sh.resource_id = e.source_id
    JOIN homed th ON th.resource_id = e.target_id
    WHERE NOT e.is_folded AND sh.territory <> th.territory
    GROUP BY LEAST(sh.territory, th.territory), GREATEST(sh.territory, th.territory);
$$;
