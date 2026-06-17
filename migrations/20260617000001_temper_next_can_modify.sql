-- WS2 — add the temper_next WRITE-AXIS gate (can_modify_resource).
--
-- Append-only to the frozen temper_next lineage: the install migration
-- (20260613000001) stays untouched and the 4c forward migration (20260616000001)
-- precedes this. The artifact (schema-artifact/02_functions.sql) is the design-master;
-- this is its faithful append. The semantic drift guard (crates/temper-next/tests/
-- schema_drift.rs) proves the lineage reconstructs the artifact, so the function BODY
-- here is byte-identical to the artifact (unqualified names that resolve against the
-- `SET search_path` below — exactly as 20260616000001 does); the body is what
-- pg_get_functiondef fingerprints, so it must not schema-qualify.
--
-- Idempotent: CREATE OR REPLACE so a re-run or a fresh persistent DB both converge here.
SET search_path TO temper_next, public;

CREATE OR REPLACE FUNCTION temper_next.can_modify_resource(p_profile uuid, p_resource uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    WITH reachable_teams AS (
        SELECT DISTINCT a.team_id
        FROM profile_effective_teams(p_profile) e
        CROSS JOIN LATERAL team_ancestors(e.team_id) a
    )
    SELECT EXISTS (
        -- owned / originated (the home confers modify to its principals)
        SELECT 1 FROM kb_resource_homes h
         WHERE h.resource_id = p_resource
           AND (h.owner_profile_id = p_profile OR h.originator_profile_id = p_profile)
        UNION ALL
        -- direct profile-anchored WRITE grant
        SELECT 1 FROM kb_resource_access ra
         WHERE ra.resource_id = p_resource
           AND ra.anchor_table = 'kb_profiles' AND ra.anchor_id = p_profile AND ra.can_write
        UNION ALL
        -- team-anchored WRITE grant on a reachable (self-or-ancestor) team
        SELECT 1 FROM kb_resource_access ra
         JOIN reachable_teams rt ON ra.anchor_id = rt.team_id
         WHERE ra.resource_id = p_resource
           AND ra.anchor_table = 'kb_teams' AND ra.can_write
    );
$$;
