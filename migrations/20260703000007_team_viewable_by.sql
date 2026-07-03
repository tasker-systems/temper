-- Extract the repeated deny-as-absence "can this profile view this team?" gate
-- (member of the team or one of its descendants) into a single SQL function.
-- Was copy-pasted inline in graph_service::neighborhood_slice,
-- graph_service::territory_overview, and team_service::graph_scope.
CREATE FUNCTION team_viewable_by(p_profile uuid, p_team uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS(
        SELECT 1 FROM team_descendants(p_team) d
        JOIN kb_team_members tm ON tm.team_id = d.team_id AND tm.profile_id = p_profile
    );
$$;
