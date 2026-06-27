-- Context visibility predicate (I2 fix): a single source of truth.
--
-- Before this migration, the context-resolution paths (UUID resolve, list, get,
-- @handle visibility gate) used a predicate that admitted a context only when it
-- was profile-owned OR explicitly shared to one of the principal's teams via a
-- `kb_team_contexts` row. A TEAM-OWNED context (`owner_table='kb_teams'`) with NO
-- self-share row therefore failed those paths for a member of the owning team —
-- even though the `+team-slug/ctx-slug` resolution path admits it by team
-- membership. The result was a false-negative asymmetry: the same context
-- resolved by `+team/slug` but `NotFound` by UUID and absent from `list`.
--
-- `context_visible_to` makes membership in the owning team a first-class
-- visibility clause, so every site agrees. False-negatives only were affected
-- (this is NOT a leak fix); non-members still see nothing.
--
-- A principal may see context `c` when:
--   1. it is their own personal context (owner_table='kb_profiles', owner_id=principal), OR
--   2. it is owned by a team they are a member of (owner_table='kb_teams'), OR
--   3. it is explicitly shared (kb_team_contexts) to a team they are a member of.
--
-- STABLE + LANGUAGE sql so `sqlx::query!` callers remain compile-checked.
CREATE FUNCTION context_visible_to(p_principal uuid, p_context_id uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM kb_contexts c
        WHERE c.id = p_context_id
          AND (
              (c.owner_table = 'kb_profiles' AND c.owner_id = p_principal)
              OR (c.owner_table = 'kb_teams'
                    AND EXISTS (
                        SELECT 1 FROM kb_team_members tm
                        WHERE tm.team_id = c.owner_id AND tm.profile_id = p_principal))
              OR EXISTS (
                    SELECT 1 FROM kb_team_contexts tc
                    JOIN kb_team_members tm ON tm.team_id = tc.team_id
                    WHERE tc.context_id = c.id AND tm.profile_id = p_principal)
          )
    );
$$;
