-- D3: carry the home cogmap's name on orphan salient nodes so the sparse
-- territory label shows a real name. RETURNS-TABLE shape change ⇒ DROP+CREATE.

DROP FUNCTION IF EXISTS graph_orphan_salient_nodes(uuid, uuid);
CREATE FUNCTION graph_orphan_salient_nodes(
    p_profile uuid, p_team uuid
) RETURNS TABLE(id uuid, title text, doc_type text, degree int, anchor_id uuid, anchor_label text)
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
           deg.degree, ch.cogmap_id, cm.name AS anchor_label
    FROM cogmap_homed ch
    LEFT JOIN region_maps rm ON rm.cogmap_id = ch.cogmap_id
    JOIN kb_resources r ON r.id = ch.resource_id AND r.is_active
    JOIN kb_cogmaps cm ON cm.id = ch.cogmap_id
    LEFT JOIN doc d ON d.rid = r.id
    LEFT JOIN LATERAL (
        SELECT count(*)::int AS degree
        FROM kb_edges e
        JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
        WHERE (e.source_id = r.id OR e.target_id = r.id)
    ) deg ON true
    WHERE rm.cogmap_id IS NULL
    ORDER BY deg.degree DESC;
$$;

DROP FUNCTION IF EXISTS graph_cogmap_orphan_nodes(uuid, uuid);
CREATE FUNCTION graph_cogmap_orphan_nodes(p_profile uuid, p_cogmap uuid)
RETURNS TABLE(id uuid, title text, doc_type text, degree int, anchor_id uuid, anchor_label text)
LANGUAGE sql STABLE AS $$
    WITH doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
        FROM kb_properties p
        WHERE p.owner_table='kb_resources' AND p.property_key='doc_type' AND NOT p.is_folded
    ),
    homed AS (
        SELECT h.resource_id
        FROM kb_resource_homes h
        JOIN resources_visible_to(p_profile) v ON v.resource_id = h.resource_id
        WHERE h.anchor_table = 'kb_cogmaps' AND h.anchor_id = p_cogmap
    ),
    region_members AS (
        SELECT DISTINCT rm.member_id AS resource_id
        FROM kb_cogmap_region_members rm
        JOIN kb_cogmap_regions reg ON reg.id = rm.region_id
        WHERE reg.cogmap_id = p_cogmap AND NOT reg.is_folded
          AND rm.member_table = 'kb_resources'
    )
    SELECT r.id, r.title, d.dt AS doc_type, deg.degree, p_cogmap AS anchor_id,
           (SELECT name FROM kb_cogmaps WHERE id = p_cogmap) AS anchor_label
    FROM homed
    JOIN kb_resources r ON r.id = homed.resource_id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    LEFT JOIN LATERAL (
        SELECT count(*)::int AS degree
        FROM kb_edges e
        JOIN edges_visible_to(p_profile) ev ON ev.edge_id = e.id
        WHERE (e.source_id = r.id OR e.target_id = r.id)
    ) deg ON true
    WHERE r.id NOT IN (SELECT resource_id FROM region_members)
    ORDER BY deg.degree DESC;
$$;
