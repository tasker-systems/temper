-- The gating team is never someone's personal team.
--
-- Person erasure erases every resource in the subject's personal team and retires its contexts
-- (20261021100000, 20261022100000). That is right for a personal team, and wrong for the gating team,
-- which holds org-wide content. Pointing the gating slug at a personal team made the two one team,
-- so erasing that person would take the org's content with them. Each step was legitimate on its
-- own; this migration refuses the combination.
--
-- The rule lives in the database because the gating slug has writers outside the settings door: the
-- operator's root step (migrations/templates/system_initialization.sql) and hand-written SQL. The
-- door also pre-checks, so an admin gets a 400 that says why rather than a check violation.
--
-- The combination has two sides, and each table gets a trigger:
--   - kb_system_settings: a gating slug naming a team that carries personal_of is refused. A write
--     that leaves the slug as it was is not judged, so an unrelated settings update never fails on it.
--   - kb_teams: a team carrying personal_of never holds the gating slug, whether it is created
--     under it or acquires personal_of or the slug later. Nothing but genesis, the 20261013100000
--     backfill and replay's verbatim restore writes personal_of today, and nothing renames a shared
--     team onto another slug; this side holds the rule if either changes.
--
-- Both triggers take one transaction advisory lock, so two concurrent writes, one on each table,
-- cannot each miss the other's uncommitted row and commit the combination together. The settings side
-- takes it exclusive and the team side shared: two team-side writes cannot make the combination
-- between them, so sign-ups and erasures (which rename a personal team) never wait on one another,
-- only on a gating slug change.
--
-- Genesis (sync_personal_team) skips the gating slug the way it skips a taken one. The gating slug can
-- name no team — hand-written SQL, or a former personal team that erasure renamed away — and can have
-- the `personal-<handle>` shape. Without the skip, the kb_teams trigger would refuse that handle's
-- sign-up; with it, the new profile's personal team takes the next free `personal-<handle>-N`.
--
-- A deployment already in the refused state fails this migration, naming the slug. Repoint the gating
-- team at a shared team, then deploy again.

DO $$
DECLARE
    v_slug text;
BEGIN
    SELECT s.gating_team_slug INTO v_slug
      FROM kb_system_settings s
      JOIN kb_teams t ON t.slug = s.gating_team_slug
     WHERE t.personal_of IS NOT NULL;
    IF v_slug IS NOT NULL THEN
        RAISE EXCEPTION 'the gating team % is a personal team: repoint kb_system_settings.gating_team_slug at a shared team before this migration', v_slug
            USING ERRCODE = 'check_violation';
    END IF;
END;
$$;

-- One key for the whole rule: the gating slug is a singleton, so there is nothing finer to lock by.
-- The `gating_team:` prefix keeps it out of the key space of the repository's other
-- `hashtextextended` advisory locks.
CREATE OR REPLACE FUNCTION _gating_team_lock(p_exclusive boolean)
RETURNS void LANGUAGE plpgsql VOLATILE
SET search_path = public, pg_temp
AS $$
BEGIN
    IF p_exclusive THEN
        PERFORM pg_advisory_xact_lock(hashtextextended('gating_team:', 0));
    ELSE
        PERFORM pg_advisory_xact_lock_shared(hashtextextended('gating_team:', 0));
    END IF;
END;
$$;
COMMENT ON FUNCTION _gating_team_lock(boolean) IS
  'Until the transaction ends: exclusive for a gating slug change, shared for a personal-team write. '
  'A gating slug change and a personal-team write cannot each miss the other; personal-team writes '
  'never wait on one another.';

CREATE OR REPLACE FUNCTION gating_team_is_never_personal()
RETURNS trigger LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    IF TG_OP = 'UPDATE' AND NEW.gating_team_slug IS NOT DISTINCT FROM OLD.gating_team_slug THEN
        RETURN NEW;
    END IF;
    PERFORM _gating_team_lock(true);
    IF EXISTS (SELECT 1 FROM kb_teams
                WHERE slug = NEW.gating_team_slug
                  AND personal_of IS NOT NULL) THEN
        RAISE EXCEPTION 'the gating team cannot be % because it is a personal team', NEW.gating_team_slug
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;
COMMENT ON FUNCTION gating_team_is_never_personal() IS
  'Refuses a kb_system_settings.gating_team_slug naming a team that carries personal_of.';

CREATE TRIGGER trg_gating_team_is_never_personal
    BEFORE INSERT OR UPDATE OF gating_team_slug ON kb_system_settings
    FOR EACH ROW EXECUTE FUNCTION gating_team_is_never_personal();

CREATE OR REPLACE FUNCTION personal_team_is_never_gating()
RETURNS trigger LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    IF NEW.personal_of IS NULL THEN
        RETURN NEW;
    END IF;
    PERFORM _gating_team_lock(false);
    IF EXISTS (SELECT 1 FROM kb_system_settings WHERE gating_team_slug = NEW.slug) THEN
        RAISE EXCEPTION 'team % is the gating team, so it cannot be a personal team', NEW.slug
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;
COMMENT ON FUNCTION personal_team_is_never_gating() IS
  'Refuses a kb_teams row carrying personal_of whose slug is the gating slug.';

CREATE TRIGGER trg_personal_team_is_never_gating
    BEFORE INSERT OR UPDATE OF slug, personal_of ON kb_teams
    FOR EACH ROW EXECUTE FUNCTION personal_team_is_never_gating();

-- Unchanged from 20261013100000 except that a candidate slug equal to the gating slug is skipped, under
-- the lock, so the trigger above never refuses a sign-up.
CREATE OR REPLACE FUNCTION sync_personal_team()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    v_base   text := 'personal-' || NEW.handle;
    v_team   uuid;
    v_root   uuid;
    v_gating text;
    v_slug   text;
    v_n      integer := 1;
BEGIN
    SELECT id INTO v_root FROM kb_teams WHERE slug = 'temper-system';

    SELECT id INTO v_team FROM kb_teams WHERE personal_of = NEW.id;

    IF v_team IS NULL THEN
        PERFORM _gating_team_lock(false);
        SELECT gating_team_slug INTO v_gating FROM kb_system_settings WHERE id = 1;
    END IF;

    WHILE v_team IS NULL LOOP
        v_slug := CASE WHEN v_n = 1 THEN v_base ELSE v_base || '-' || v_n END;
        v_n := v_n + 1;
        CONTINUE WHEN v_slug IS NOT DISTINCT FROM v_gating;
        INSERT INTO kb_teams (slug, name, personal_of)
        VALUES (v_slug, NEW.display_name || ' (personal)', NEW.id)
        ON CONFLICT (slug) DO NOTHING
        RETURNING id INTO v_team;
    END LOOP;

    INSERT INTO kb_team_members (team_id, profile_id, role)
    VALUES (v_team, NEW.id, 'owner'::team_role)
    ON CONFLICT (team_id, profile_id) DO NOTHING;
    IF v_root IS NOT NULL THEN
        INSERT INTO kb_teams_parents (child_id, parent_id)
        VALUES (v_team, v_root)
        ON CONFLICT DO NOTHING;
    END IF;
    RETURN NEW;
END;
$$;

SELECT declare_migration(
    20261025100000,
    'additive',
    'Adds two triggers so the gating team is never a personal team, serialized by a new helper, '
    '_gating_team_lock(boolean), that takes one transaction advisory lock: exclusive for a gating slug '
    'change, shared for a personal-team write. On kb_system_settings, a changed '
    'gating_team_slug naming a team with personal_of set is refused. On kb_teams, a team with '
    'personal_of set never holds the gating slug. CREATE OR REPLACE, signature unchanged, of '
    'sync_personal_team: genesis skips the gating slug as it skips a taken one. Refuses to apply if '
    'the deployment is already in the refused state. A binary without this migration keeps working: '
    'the settings door accepted these writes before, and now the database refuses them, which that '
    'binary renders as an error.'
);
