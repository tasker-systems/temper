-- R3 territory slice: components + visibility-scoped region members.

CREATE FUNCTION graph_region_components(
    p_profile uuid, p_region uuid
) RETURNS TABLE(component_id uuid, member_count int) LANGUAGE sql STABLE AS $$
    SELECT comp.id, cardinality(comp.member_ids)::int
    FROM kb_cogmap_regions reg
    JOIN kb_cogmap_components comp
      ON comp.cogmap_id = reg.cogmap_id AND comp.lens_id = reg.lens_id AND NOT comp.is_folded
    WHERE reg.id = p_region AND NOT reg.is_folded
      AND cogmap_readable_by_profile(p_profile, reg.cogmap_id);
$$;

CREATE FUNCTION graph_region_members(
    p_profile uuid, p_region uuid
) RETURNS TABLE(id uuid, title text, doc_type text, affinity double precision)
LANGUAGE sql STABLE AS $$
    WITH doc AS (
        SELECT p.owner_id AS rid, (p.property_value #>> '{}') AS dt
        FROM kb_properties p
        WHERE p.owner_table='kb_resources' AND p.property_key='doc_type' AND NOT p.is_folded
    ),
    visible AS (SELECT resource_id FROM resources_visible_to(p_profile))
    SELECT r.id, r.title, d.dt AS doc_type, m.affinity
    FROM kb_cogmap_regions reg
    JOIN kb_cogmap_region_members m ON m.region_id = reg.id AND m.member_table = 'kb_resources'
    JOIN visible v ON v.resource_id = m.member_id
    JOIN kb_resources r ON r.id = m.member_id AND r.is_active
    LEFT JOIN doc d ON d.rid = r.id
    WHERE reg.id = p_region AND NOT reg.is_folded
      AND cogmap_readable_by_profile(p_profile, reg.cogmap_id)
    ORDER BY m.affinity DESC NULLS LAST;
$$;
