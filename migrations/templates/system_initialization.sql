-- migrations/templates/system_initialization.sql
-- Template for self-hosted operators to enable invite-only mode.
-- Copy this file, fill in your values, and run against your database.
-- Also used verbatim by integration tests.

-- Step 1: Add yourself as owner of the gating team.
-- Replace the profile_id with your own (from kb_profiles).
INSERT INTO kb_team_members (id, team_id, profile_id, role, joined_at)
VALUES (
    gen_random_uuid(),
    '00000000-0000-0000-0000-000000000002',  -- temper-system team
    '00000000-0000-0000-0004-000000000001',  -- REPLACE with your profile ID
    'owner',
    now()
);

-- Step 2: Enable invite-only mode.
UPDATE kb_system_settings
   SET access_mode = 'invite_only',
       gating_team_slug = 'temper-system',
       instance_name = 'temper',              -- REPLACE with your instance name
       updated = now();

-- Optional: Set terms version and URI.
-- UPDATE kb_system_settings
--    SET terms_version = '1.0',
--        terms_resource_uri = 'kb://+temper-system/general/concept/terms',
--        updated = now();
