-- B1: an unlabeled region falls back to its top VISIBLE member's title, so the
-- panorama is legible. Prefers a stored/steward label when present (so a future
-- steward-naming arc needs no read change). The representative respects
-- resources_visible_to — a private member's title is never surfaced as a label.
-- Body-only change; RETURNS TABLE shape unchanged → CREATE OR REPLACE.

CREATE OR REPLACE FUNCTION graph_region_territories(
    p_profile uuid, p_team uuid, p_lens uuid
) RETURNS TABLE(region_id uuid, cogmap_id uuid, label text, member_count int, salience double precision)
LANGUAGE sql STABLE AS $$
    SELECT reg.id, reg.cogmap_id,
           COALESCE(reg.label, rep.title) AS label,
           reg.member_count, reg.salience
    FROM kb_cogmap_regions reg
    JOIN kb_team_cogmaps tc ON tc.cogmap_id = reg.cogmap_id
    JOIN team_ancestors(p_team) a ON a.team_id = tc.team_id
    LEFT JOIN LATERAL (
        SELECT r.title
        FROM kb_cogmap_region_members m
        JOIN resources_visible_to(p_profile) v ON v.resource_id = m.member_id
        JOIN kb_resources r ON r.id = m.member_id AND r.is_active
        WHERE m.region_id = reg.id AND m.member_table = 'kb_resources'
        ORDER BY m.affinity DESC NULLS LAST
        LIMIT 1
    ) rep ON true
    WHERE NOT reg.is_folded
      AND reg.lens_id = p_lens
      AND cogmap_readable_by_profile(p_profile, reg.cogmap_id);
$$;
