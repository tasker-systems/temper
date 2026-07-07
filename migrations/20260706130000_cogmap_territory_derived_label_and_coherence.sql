-- Beat A: the cogmap panorama read gains B1's derived label (unlabeled region →
-- top VISIBLE member title, resources_visible_to discipline) AND returns
-- content_cohesion for the hover. Adding an OUT column changes the return type →
-- DROP + CREATE (CREATE OR REPLACE is illegal). Skew-safe: the sole caller selects
-- columns by name, so pre-deploy code selecting the old 5 keeps working.
DROP FUNCTION IF EXISTS graph_cogmap_territories(uuid, uuid, uuid);
CREATE FUNCTION graph_cogmap_territories(p_profile uuid, p_cogmap uuid, p_lens uuid)
RETURNS TABLE(region_id uuid, cogmap_id uuid, label text,
              member_count int, salience double precision, coherence double precision)
LANGUAGE sql STABLE AS $$
    SELECT reg.id, reg.cogmap_id,
           COALESCE(reg.label, rep.title) AS label,
           reg.member_count, reg.salience, reg.content_cohesion
    FROM kb_cogmap_regions reg
    LEFT JOIN LATERAL (
        SELECT r.title
        FROM kb_cogmap_region_members m
        JOIN resources_visible_to(p_profile) v ON v.resource_id = m.member_id
        JOIN kb_resources r ON r.id = m.member_id AND r.is_active
        WHERE m.region_id = reg.id AND m.member_table = 'kb_resources'
        ORDER BY m.affinity DESC NULLS LAST
        LIMIT 1
    ) rep ON true
    WHERE reg.cogmap_id = p_cogmap AND NOT reg.is_folded AND reg.lens_id = p_lens
      AND cogmap_readable_by_profile(p_profile, p_cogmap);
$$;
