-- Migration: centralize the soft-delete READ floor — is_active gate inside the
-- visibility functions.
--
-- SQL function audit 2026-07-08 (docs/code-reviews/2026-07-08-sql-function-audit.md,
-- finding SQLA-3 / soft-delete-content-leak): `_project_resource_deleted` only flips
-- `kb_resources.is_active`, leaving homes/blocks/grants intact — so `resources_visible_to`
-- (and `resources_readable_by`, which composes it) kept returning soft-deleted ids, and the
-- content surfaces that trust it (`cogmap_regulation`, `resource_blocks`,
-- `resource_block_provenance`) kept serving a deleted resource's title/body/provenance while
-- the graph/search surfaces each re-filtered `r.is_active` themselves.
--
-- Fix: gate BOTH principal axes at the source — `resources_visible_to` (profile) and
-- `resources_accessible_to_cogmap` (cogmap) — so every current and future consumer inherits
-- the floor instead of remembering its own filter. The existing `r.is_active` filters in the
-- graph/search functions become redundant (harmless) rather than load-bearing.
--
-- Caller audit (all consumers of resources_visible_to / resources_readable_by, SQL + Rust):
-- none needs inactive rows. Deliberate behavior changes riding this gate:
--   * element_trail_node: a soft-deleted resource's trail is no longer served — consistent
--     with show/list 404ing on deleted resources (deny-by-invisibility).
--   * edges_visible_to / endpoint_readable_by_profile: edges to a deleted endpoint go
--     invisible — matches graph_atlas_nodes already dropping the deleted node.
--   * edge assert / reassign visibility probes now deny deleted targets.
-- Known non-goal: `can()`'s `profile_explicit_grant` branch stays is_active-blind (an explicit
-- grant row on a deleted resource still answers true). Its only production caller uses the
-- 'grant' action, and the derived-read branch inherits this gate; revisit if can(read) ever
-- becomes a production read gate.
--
-- Bodies below are the live definitions with the union wrapped in an is_active semi-join;
-- branch comments carried verbatim.

CREATE OR REPLACE FUNCTION resources_visible_to(p_profile uuid)
RETURNS TABLE(resource_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT v.resource_id
    FROM (
        WITH reachable_teams AS (
            SELECT DISTINCT a.team_id
            FROM profile_effective_teams(p_profile) e
            CROSS JOIN LATERAL team_ancestors(e.team_id) a
        )
        -- owned / originated (the home confers access to its principals)
        SELECT h.resource_id FROM kb_resource_homes h
         WHERE h.owner_profile_id = p_profile OR h.originator_profile_id = p_profile
        UNION
        -- direct profile-anchored grant (consumer-axis ONLY — never enters a vis(T)). kb_access_grants store.
        SELECT g.subject_id FROM kb_access_grants g
         WHERE g.subject_table = 'kb_resources' AND g.principal_table = 'kb_profiles'
           AND g.principal_id = p_profile AND g.can_read
        UNION
        -- team-anchored grant on a reachable (self-or-ancestor) team. kb_access_grants store.
        SELECT g.subject_id FROM kb_access_grants g
         JOIN reachable_teams rt ON g.principal_id = rt.team_id
         WHERE g.subject_table = 'kb_resources' AND g.principal_table = 'kb_teams' AND g.can_read
        UNION
        -- context-share: resources homed in a context shared to a reachable team (WS6 §2)
        SELECT h.resource_id
        FROM kb_team_contexts tc
        JOIN reachable_teams rt ON tc.team_id = rt.team_id
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_contexts' AND h.anchor_id = tc.context_id
        UNION
        -- team-owned context: resources homed in a context OWNED by a team the principal is a member of.
        -- FLAT by design (§3 item C: context-ownership is home-like + paired with flat addressability) —
        -- do NOT ancestor-expand this branch. Membership routed through profile_effective_teams so a
        -- soft-deleted owning team confers nothing.
        SELECT h.resource_id
        FROM kb_contexts c
        JOIN profile_effective_teams(p_profile) pet
          ON pet.team_id = c.owner_id
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_contexts' AND h.anchor_id = c.id
        WHERE c.owner_table = 'kb_teams'
        UNION
        -- cogmap membership: resources homed in a cognitive map joined to a REACHABLE (self-or-ancestor)
        -- team — UP+union (D4) to match the team-grant/context-share reach above, so map-read and resource-read
        -- agree by construction at the up-expanded level.
        SELECT h.resource_id
        FROM kb_team_cogmaps tc
        JOIN reachable_teams rt ON rt.team_id = tc.team_id
        JOIN kb_resource_homes h
          ON h.anchor_table = 'kb_cogmaps' AND h.anchor_id = tc.cogmap_id
        UNION
        -- explicit subject-grant (D3a): resources homed in a cogmap or context the profile holds an explicit
        -- read-grant on (direct profile grant OR team grant on a reachable team). kb_access_grants only.
        SELECT h.resource_id
        FROM kb_resource_homes h
        JOIN kb_access_grants g
          ON g.subject_table = h.anchor_table AND g.subject_id = h.anchor_id
        WHERE h.anchor_table IN ('kb_cogmaps','kb_contexts') AND g.can_read
          AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
             OR (g.principal_table = 'kb_teams'    AND g.principal_id IN (SELECT team_id FROM reachable_teams)) )
    ) v
    -- soft-delete READ floor: a deleted resource is invisible on every axis (see header).
    JOIN kb_resources r ON r.id = v.resource_id AND r.is_active;
$$;

CREATE OR REPLACE FUNCTION resources_accessible_to_cogmap(p_cogmap uuid)
RETURNS TABLE(resource_id uuid)
LANGUAGE sql STABLE AS $$
    SELECT a.resource_id
    FROM (
        -- own interior (map-home-confers, access §1) — always readable by the map's agent
        SELECT h.resource_id FROM kb_resource_homes h
         WHERE h.anchor_table = 'kb_cogmaps' AND h.anchor_id = p_cogmap
        UNION
        -- shared reach: a resource in EVERY joined team's vis(T); empty join ⇒ none
        SELECT v.resource_id
        FROM (
            SELECT tc.team_id, vt.resource_id
            FROM kb_team_cogmaps tc
            CROSS JOIN LATERAL vis_team(tc.team_id) vt
            WHERE tc.cogmap_id = p_cogmap
        ) v
        GROUP BY v.resource_id
        HAVING count(DISTINCT v.team_id) = (
            SELECT count(*) FROM kb_team_cogmaps tc WHERE tc.cogmap_id = p_cogmap
        )
        AND (SELECT count(*) FROM kb_team_cogmaps tc WHERE tc.cogmap_id = p_cogmap) > 0
    ) a
    -- soft-delete READ floor: a deleted resource is invisible on every axis (see header).
    JOIN kb_resources r ON r.id = a.resource_id AND r.is_active;
$$;
