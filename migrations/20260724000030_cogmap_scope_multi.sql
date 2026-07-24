-- Multi-map search scope: the UNION of several maps' single-map scopes.
--
-- The search sink (`unified_search.p_scope_ids uuid[]`) was always a set; only the resolver and the
-- wire field were singular. This function restores the plural without touching search: it unions
-- per-map `cogmap_scope_ids`, each call independently gated by `cogmap_readable_by_profile` (an
-- unreadable map in the set contributes zero rows, never an error). A resource is homed in exactly
-- one map, so the sets are disjoint; DISTINCT is defensive, not load-bearing.
--
-- Additive-only (new CREATE FUNCTION), safe on main.
CREATE FUNCTION cogmap_scope_ids_multi(p_principal uuid, p_cogmaps uuid[])
RETURNS SETOF uuid LANGUAGE sql STABLE AS $$
    SELECT DISTINCT s.id
    FROM unnest(p_cogmaps) AS m(cogmap_id)
    CROSS JOIN LATERAL cogmap_scope_ids(p_principal, m.cogmap_id) AS s(id);
$$;
