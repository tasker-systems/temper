-- I3a: Add polymorphic ownership to kb_contexts.
-- Contexts are now scoped to a profile or team via (kb_owner_table, kb_owner_id).

-- Step 1: Add columns as nullable
ALTER TABLE kb_contexts
  ADD COLUMN kb_owner_table VARCHAR(64),
  ADD COLUMN kb_owner_id UUID;

-- Step 2: Backfill existing seed contexts to system profile
UPDATE kb_contexts
  SET kb_owner_table = 'kb_profiles',
      kb_owner_id = '00000000-0000-0000-0004-000000000001';

-- Step 3: Set NOT NULL after backfill
ALTER TABLE kb_contexts
  ALTER COLUMN kb_owner_table SET NOT NULL,
  ALTER COLUMN kb_owner_table SET DEFAULT 'kb_profiles',
  ALTER COLUMN kb_owner_id SET NOT NULL;

-- Step 4: Replace global unique with per-owner unique
ALTER TABLE kb_contexts DROP CONSTRAINT kb_contexts_name_key;
ALTER TABLE kb_contexts
  ADD CONSTRAINT kb_contexts_owner_name_unique
  UNIQUE (kb_owner_table, kb_owner_id, name);

-- Step 5: Constrain owner table values
ALTER TABLE kb_contexts
  ADD CONSTRAINT kb_contexts_owner_table_check
  CHECK (kb_owner_table IN ('kb_profiles', 'kb_teams'));

-- Step 6: Index for ownership lookups
CREATE INDEX idx_contexts_owner ON kb_contexts(kb_owner_table, kb_owner_id);

-- Step 7: Function to return contexts visible to a profile
CREATE FUNCTION contexts_visible_to(
    p_profile_id UUID,
    p_team_id UUID DEFAULT NULL
) RETURNS TABLE(id UUID, name VARCHAR(128), kb_owner_table VARCHAR(64), kb_owner_id UUID)
LANGUAGE SQL STABLE AS $$
    -- Contexts I own
    SELECT c.id, c.name, c.kb_owner_table, c.kb_owner_id
    FROM kb_contexts c
    WHERE c.kb_owner_table = 'kb_profiles'
      AND c.kb_owner_id = p_profile_id

    UNION

    -- Contexts owned by teams I belong to
    SELECT c.id, c.name, c.kb_owner_table, c.kb_owner_id
    FROM kb_contexts c
    JOIN kb_team_members tm ON tm.team_id = c.kb_owner_id
    WHERE c.kb_owner_table = 'kb_teams'
      AND tm.profile_id = p_profile_id
      AND (p_team_id IS NULL OR c.kb_owner_id = p_team_id)
$$;
