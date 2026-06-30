-- Surface B Beat 1: producer write seam + single-map search scope.

-- Authorial RBAC seam. Today team-cogmap membership confers write; the cogmap-arc
-- RBAC tightens this WITHOUT touching call sites. (memory: project_authorial_rbac_undefined_contexts_cogmaps)
CREATE FUNCTION cogmap_authorable_by_profile(p_profile uuid, p_cogmap uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT cogmap_readable_by_profile(p_profile, p_cogmap);
$$;

-- Single-map search scope: resources homed in the cogmap that the principal can see,
-- gated by map readability (deny -> zero rows -> empty corpus, never an error).
CREATE FUNCTION cogmap_scope_ids(p_principal uuid, p_cogmap uuid)
RETURNS SETOF uuid LANGUAGE sql STABLE AS $$
    SELECT h.resource_id
    FROM kb_resource_homes h
    WHERE h.anchor_table = 'kb_cogmaps'
      AND h.anchor_id = p_cogmap
      AND cogmap_readable_by_profile(p_principal, p_cogmap)
      AND h.resource_id IN (SELECT resource_id FROM resources_visible_to(p_principal));
$$;
