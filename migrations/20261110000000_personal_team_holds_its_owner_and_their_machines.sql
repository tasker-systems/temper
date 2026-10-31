-- A personal team holds one human and that human's machines, and nothing hangs beneath it.
--
-- Task 01a19006-7299-7632-bd74-33780a2fd814, ruled 2026-10-31: a personal team's members are its
-- `personal_of` profile and the machines that profile registered, and no one else, by any door. A
-- system admin may list a personal team for audit, and may not act on it except by erasure.
--
-- Read reach walks UP from a principal's own teams (`team_ancestors`), so whatever is attached to a
-- personal team reaches every member of it and every member of a team beneath it. Both are therefore
-- the same leak, and both are closed here:
--
--   1. personal_team_admits(team, profile): the one statement of who may hold a row on a team. True
--      for every team without `personal_of`; on a personal team, true for its `personal_of` and for a
--      machine whose client that profile registered (`kb_machine_clients.registered_by_profile_id`).
--      The service reads it under its locks to refuse by name; the trigger below holds it for every
--      other writer — operator SQL, the SAML reconcile, a binary that predates the service check.
--   2. kb_team_members: a DEFERRED constraint trigger refuses a row the predicate does not admit.
--      Deferred because a machine registration writes the machine's reach before its client row,
--      in one transaction (machine_registration_service::provision/issue): at the insert the machine
--      is not yet anyone's, and at commit it is the registrar's. A row deleted later in the same
--      transaction is not judged.
--   3. kb_teams_parents: a personal team is never a parent, and its one parent is temper-system.
--   4. kb_saml_group_mappings and kb_team_invitations: never target a personal team. A personal
--      team admits no other human, so there is nothing for either to offer.
--   5. kb_teams: a team that acquires `personal_of` must already satisfy all of the above. Only
--      genesis writes it today (on insert, with no members yet); this holds the rule if that changes.
--
-- Erasure is untouched: it deletes rows, renames the team and nulls identifiers, none of which these
-- triggers judge.
--
-- Pending invitations to a personal team are withdrawn, as 20261101300000 withdrew gating-team ones.
--
-- A deployment already holding a row these rules refuse fails this migration, naming each one.
-- Remove the member (the personal team's owner can, with `temper team remove-member`), detach the
-- child, or delete the mapping, then deploy again. Nothing here rewrites membership on its own.

CREATE OR REPLACE FUNCTION personal_team_admits(p_team uuid, p_profile uuid)
RETURNS boolean LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT NOT EXISTS (
        SELECT 1 FROM kb_teams t
         WHERE t.id = p_team
           AND t.personal_of IS NOT NULL
           AND t.personal_of <> p_profile
           AND NOT EXISTS (SELECT 1 FROM kb_machine_clients mc
                            WHERE mc.profile_id = p_profile
                              AND mc.registered_by_profile_id = t.personal_of)
    );
$$;
COMMENT ON FUNCTION personal_team_admits(uuid, uuid) IS
  'Whether a profile may hold a row on a team: always on a shared team; on a personal team only its '
  'personal_of profile and the machines that profile registered.';

DO $$
DECLARE
    v_found text;
BEGIN
    SELECT string_agg(format('%s holds %s', t.slug, p.handle), ', ')
      INTO v_found
      FROM kb_team_members tm
      JOIN kb_teams t ON t.id = tm.team_id
      JOIN kb_profiles p ON p.id = tm.profile_id
     WHERE NOT personal_team_admits(tm.team_id, tm.profile_id);
    IF v_found IS NOT NULL THEN
        RAISE EXCEPTION 'personal teams hold members that are neither their owner nor the owner''s machines (%): remove them before this migration', v_found
            USING ERRCODE = 'check_violation';
    END IF;

    SELECT string_agg(format('%s under %s', c.slug, p.slug), ', ')
      INTO v_found
      FROM kb_teams_parents tp
      JOIN kb_teams c ON c.id = tp.child_id
      JOIN kb_teams p ON p.id = tp.parent_id
     WHERE p.personal_of IS NOT NULL
        OR (c.personal_of IS NOT NULL AND p.slug <> 'temper-system');
    IF v_found IS NOT NULL THEN
        RAISE EXCEPTION 'team links put a personal team under another team or a team under a personal team (%): detach them before this migration', v_found
            USING ERRCODE = 'check_violation';
    END IF;

    SELECT string_agg(format('%s/%s -> %s', m.idp_key, m.group_value, t.slug), ', ')
      INTO v_found
      FROM kb_saml_group_mappings m
      JOIN kb_teams t ON t.id = m.team_id
     WHERE t.personal_of IS NOT NULL;
    IF v_found IS NOT NULL THEN
        RAISE EXCEPTION 'SAML group mappings target personal teams (%): delete them before this migration', v_found
            USING ERRCODE = 'check_violation';
    END IF;
END;
$$;

DO $$
DECLARE
    v_withdrawn integer;
BEGIN
    UPDATE kb_team_invitations i
       SET revoked_at = now()
      FROM kb_teams t
     WHERE i.team_id = t.id
       AND t.personal_of IS NOT NULL
       AND i.status = 'pending'
       AND i.revoked_at IS NULL;
    GET DIAGNOSTICS v_withdrawn = ROW_COUNT;
    IF v_withdrawn > 0 THEN
        RAISE WARNING 'withdrew % pending invitation(s) to a personal team', v_withdrawn;
    END IF;
END;
$$;

CREATE OR REPLACE FUNCTION personal_team_member_admitted()
RETURNS trigger LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    -- Judged at commit, against the row as it then stands.
    IF NOT EXISTS (SELECT 1 FROM kb_team_members
                    WHERE team_id = NEW.team_id AND profile_id = NEW.profile_id) THEN
        RETURN NULL;
    END IF;
    IF NOT personal_team_admits(NEW.team_id, NEW.profile_id) THEN
        RAISE EXCEPTION 'a personal team holds only its owner and the owner''s machines'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NULL;
END;
$$;
COMMENT ON FUNCTION personal_team_member_admitted() IS
  'Refuses, at commit, a kb_team_members row that personal_team_admits does not admit.';

CREATE CONSTRAINT TRIGGER trg_personal_team_member_admitted
    AFTER INSERT OR UPDATE OF team_id, profile_id ON kb_team_members
    DEFERRABLE INITIALLY DEFERRED
    FOR EACH ROW EXECUTE FUNCTION personal_team_member_admitted();

CREATE OR REPLACE FUNCTION personal_team_link_refused()
RETURNS trigger LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM kb_teams WHERE id = NEW.parent_id AND personal_of IS NOT NULL) THEN
        RAISE EXCEPTION 'a personal team cannot have child teams'
            USING ERRCODE = 'check_violation';
    END IF;
    IF EXISTS (SELECT 1 FROM kb_teams WHERE id = NEW.child_id AND personal_of IS NOT NULL)
       AND NOT EXISTS (SELECT 1 FROM kb_teams WHERE id = NEW.parent_id AND slug = 'temper-system') THEN
        RAISE EXCEPTION 'a personal team''s only parent is temper-system'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;
COMMENT ON FUNCTION personal_team_link_refused() IS
  'Refuses a kb_teams_parents link with a personal team as parent, or with a personal team as child '
  'of anything but temper-system.';

CREATE TRIGGER trg_personal_team_link_refused
    BEFORE INSERT OR UPDATE ON kb_teams_parents
    FOR EACH ROW EXECUTE FUNCTION personal_team_link_refused();

CREATE OR REPLACE FUNCTION personal_team_is_never_offered()
RETURNS trigger LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    IF EXISTS (SELECT 1 FROM kb_teams WHERE id = NEW.team_id AND personal_of IS NOT NULL) THEN
        RAISE EXCEPTION 'a personal team takes no other member, so % cannot target one', TG_TABLE_NAME
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;
COMMENT ON FUNCTION personal_team_is_never_offered() IS
  'Refuses a SAML group mapping or an invitation that targets a personal team.';

CREATE TRIGGER trg_personal_team_is_never_mapped
    BEFORE INSERT OR UPDATE OF team_id ON kb_saml_group_mappings
    FOR EACH ROW EXECUTE FUNCTION personal_team_is_never_offered();

CREATE TRIGGER trg_personal_team_is_never_invited
    BEFORE INSERT OR UPDATE OF team_id ON kb_team_invitations
    FOR EACH ROW EXECUTE FUNCTION personal_team_is_never_offered();

CREATE OR REPLACE FUNCTION personal_team_identity_fits()
RETURNS trigger LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    IF NEW.personal_of IS NULL OR NEW.personal_of IS NOT DISTINCT FROM OLD.personal_of THEN
        RETURN NEW;
    END IF;
    IF EXISTS (SELECT 1 FROM kb_team_members tm
                WHERE tm.team_id = NEW.id
                  AND tm.profile_id <> NEW.personal_of
                  AND NOT EXISTS (SELECT 1 FROM kb_machine_clients mc
                                   WHERE mc.profile_id = tm.profile_id
                                     AND mc.registered_by_profile_id = NEW.personal_of))
       OR EXISTS (SELECT 1 FROM kb_teams_parents WHERE parent_id = NEW.id)
       OR EXISTS (SELECT 1 FROM kb_teams_parents tp JOIN kb_teams p ON p.id = tp.parent_id
                   WHERE tp.child_id = NEW.id AND p.slug <> 'temper-system')
       OR EXISTS (SELECT 1 FROM kb_saml_group_mappings WHERE team_id = NEW.id) THEN
        RAISE EXCEPTION 'team % cannot become a personal team: it holds other members, child teams, '
                        'another parent or a SAML mapping', NEW.slug
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;
COMMENT ON FUNCTION personal_team_identity_fits() IS
  'Refuses setting kb_teams.personal_of on a team that already breaks a personal team''s rules.';

CREATE TRIGGER trg_personal_team_identity_fits
    BEFORE UPDATE OF personal_of ON kb_teams
    FOR EACH ROW EXECUTE FUNCTION personal_team_identity_fits();

SELECT declare_migration(
    20261110000000,
    'additive',
    'Adds personal_team_admits and triggers refusing: a kb_team_members row on a personal team for '
    'anyone but its owner and the owner''s machines (deferred to commit), a personal team as a parent '
    'or under anything but temper-system, a SAML mapping or invitation targeting a personal team, and '
    'personal_of set on a team that breaks these. Withdraws pending invitations to personal teams. '
    'Refuses to apply if an existing row breaks them. A binary without this change keeps working for '
    'every act these rules allow; one it refuses (an admin adding a member to someone''s personal '
    'team) fails as a check violation instead of succeeding.'
);
