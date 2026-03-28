-- Audit fix: can_modify_resource() should check is_active on the resource.
-- Previously, a soft-deleted resource could pass the authorization check
-- even though callers filter is_active = true in their queries.

CREATE OR REPLACE FUNCTION can_modify_resource(
    p_profile_id UUID,
    p_resource_id UUID
) RETURNS BOOLEAN
LANGUAGE SQL STABLE AS $$
    SELECT EXISTS (
        -- I own it and it's active
        SELECT 1 FROM resources
        WHERE id = p_resource_id
          AND owner_profile_id = p_profile_id
          AND is_active = true
    ) OR EXISTS (
        -- It's vault or mutable in a team I belong to, and I'm not a watcher,
        -- and the resource is active
        SELECT 1
        FROM kb_team_resources tr
        JOIN kb_team_members tm ON tm.team_id = tr.team_id
        JOIN resources r ON r.id = tr.resource_id
        WHERE tr.resource_id = p_resource_id
          AND tm.profile_id = p_profile_id
          AND tr.access_level IN ('vault', 'mutable')
          AND tm.role != 'watcher'
          AND r.is_active = true
    )
$$;
