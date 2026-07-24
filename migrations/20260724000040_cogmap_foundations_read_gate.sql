-- Make `cogmap_foundations` self-gating on map-readability, symmetric with `cogmap_scope_ids`.
--
-- As first written (…000020) `cogmap_foundations` intersected only `resources_visible_to`, relying on
-- its sole caller (`show_visible`) to 404 on an unreadable map BEFORE calling it. That is correct today
-- but ordering-fragile: a future caller, or a query reorder, could surface the (individually-visible)
-- foundational resources of a map the caller cannot read — and it diverged from `cogmap_scope_ids`
-- (the search scope), which gates BOTH `cogmap_readable_by_profile` AND `resources_visible_to`.
--
-- Adding the map-read gate makes the function safe regardless of caller and makes "scope to map X"
-- mean the same thing on the `cogmap show`/foundations path as on search. Deny → empty, never an
-- error. Surfaced by the reach/visibility adversarial review of the discoverability PR.
CREATE OR REPLACE FUNCTION cogmap_foundations(p_profile uuid, p_cogmap uuid)
RETURNS TABLE(resource_id uuid, title text, doc_type text, is_telos boolean)
LANGUAGE sql STABLE AS $$
    SELECT r.id,
           r.title,
           dt.property_value #>> '{}' AS doc_type,
           (r.id = (SELECT telos_resource_id FROM kb_cogmaps WHERE id = p_cogmap)) AS is_telos
    FROM kb_resource_homes h
    JOIN kb_resources r ON r.id = h.resource_id
    JOIN resources_visible_to(p_profile) v ON v.resource_id = r.id
    JOIN kb_properties dt
      ON dt.owner_table = 'kb_resources' AND dt.owner_id = r.id
     AND dt.property_key = 'doc_type' AND NOT dt.is_folded
    WHERE h.anchor_table = 'kb_cogmaps'
      AND h.anchor_id = p_cogmap
      AND cogmap_readable_by_profile(p_profile, p_cogmap)
      AND r.is_active
      AND r.ingest_state = 'complete'
    ORDER BY is_telos DESC, r.title;
$$;
