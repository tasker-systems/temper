-- A private cognitive map is personal space: one person per map, and a system admin's arm stops there.
--
-- Task 01a1933d-2944-77ee-9323-c22c034c2e69, ruled 2026-10-31, the cogmap half of 20261113000000
-- (personal space is its owner's alone). A binding runs both ways: the bound team's members read the
-- map, and the map's agent computes over what every bound team can read. A personal-team binding
-- therefore makes the map one person's, and points it at that person's personal content. Sharing
-- between people is what a shared team is for, so a map is bound to one person's personal team at
-- most, and only to its creator's when it has a person as creator.
--
--   kb_cogmaps.created_by_profile_id   who invoked the genesis, recorded from the genesis self-grant;
--   cogmap_genesis_creator(map)        the same fact read off the ledger, for the backfill and replay;
--   cogmap_personal_owner(map)         the one statement: whose personal space a map is, or NULL;
--   cogmap_is_personal(map)            the boolean shorthand;
--   cogmap_personal_bind_refusal(map, team)   why a binding would break the one-person rule, or NULL;
--   kb_team_cogmaps_one_person         the trigger that applies it to every writer of a binding.
--
-- The admin arm itself is closed in Rust, where the cogmap grant and bind gates live; no plpgsql
-- function carries a system-admin bypass over a cogmap. Nothing existing is redefined here.
--
-- The migration REFUSES TO RUN while a map already breaks the rule (ruled R1): bound to more than
-- one person's personal team, or to a personal team whose person is not the map's creator. The
-- classification would keep such a binding live, so another person, and their machines, would keep
-- reading the map and the map's agent would keep computing over their personal content. Unbind, then
-- re-run. The operator pre-flight that lists these maps, with what each will be classified as, is
-- temper-artifacts runbooks/2026-10-31-private-cogmaps-one-person-preflight.sql.

-- ---------------------------------------------------------------------------
-- Section 1. The creator.
-- ---------------------------------------------------------------------------
-- Genesis fires as the system actor, so nothing on the map recorded who asked for it. The trace it
-- leaves is the creator's self-grant, written in the same transaction as the map: its grant_created
-- event's occurred_at equals kb_cogmaps.created to the microsecond, because both are now() of one
-- transaction. A later self-grant, an admin's included, cannot match. Maps from before the grant
-- chokepoint (20260718000010) have no such event and stay NULL.
ALTER TABLE kb_cogmaps ADD COLUMN created_by_profile_id uuid;
COMMENT ON COLUMN kb_cogmaps.created_by_profile_id IS
    'The profile that invoked the map''s genesis (a person or a machine), or NULL when unknown. Set from '
    'the genesis self-grant by a trigger on kb_access_grants; backfilled, and re-derived by replay, '
    'through cogmap_genesis_creator. No foreign '
    'key: replay projects kb_cogmaps before it could check one. Read whose personal space a map is '
    'through cogmap_personal_owner, never from this column alone.';

CREATE FUNCTION cogmap_genesis_creator(p_cogmap uuid)
RETURNS uuid LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    -- Narrowed by event type and by the map's own creation instant before any payload text is read.
    SELECT (e.payload->>'granted_by')::uuid
      FROM kb_cogmaps c
      JOIN kb_events e
        ON e.event_type_id = (SELECT et.id FROM kb_event_types et WHERE et.name = 'grant_created')
       AND e.occurred_at = c.created
     WHERE c.id = p_cogmap
       AND e.payload->>'subject_table' = 'kb_cogmaps'
       AND e.payload->>'subject_id' = p_cogmap::text
       AND e.payload->>'principal_table' = 'kb_profiles'
       AND e.payload->>'principal_id' = e.payload->>'granted_by'
     ORDER BY e.id
     LIMIT 1;
$$;
COMMENT ON FUNCTION cogmap_genesis_creator(uuid) IS
  'The grantee of a map''s genesis self-grant (a grant_created event on the map whose principal is its '
  'granter and whose occurred_at equals kb_cogmaps.created), or NULL. The ledger''s statement of '
  'kb_cogmaps.created_by_profile_id.';

UPDATE kb_cogmaps SET created_by_profile_id = cogmap_genesis_creator(id);

-- From now on the column is set where the fact is made: the genesis self-grant's own row. A grant on
-- a map to its granter, written in the transaction that created the map (kb_cogmaps.created is that
-- transaction's now()), names the creator, which is exactly cogmap_genesis_creator's rule read at the
-- moment it becomes true. In SQL, not in the genesis binary, so a binary that predates this migration
-- still records the creator, and every genesis path does. Replay restores kb_access_grants before any
-- map exists, so this does nothing there; replay re-derives the column through
-- cogmap_genesis_creator once the maps are projected.
CREATE FUNCTION kb_access_grants_records_cogmap_creator()
RETURNS trigger LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    UPDATE kb_cogmaps
       SET created_by_profile_id = NEW.principal_id
     WHERE id = NEW.subject_id
       AND created_by_profile_id IS NULL
       AND created = now();
    RETURN NULL;
END;
$$;
COMMENT ON FUNCTION kb_access_grants_records_cogmap_creator() IS
  'Records a map''s creator from its genesis self-grant (a grant on the map to its granter, in the '
  'transaction that created the map) into kb_cogmaps.created_by_profile_id.';

CREATE TRIGGER kb_access_grants_records_cogmap_creator
    AFTER INSERT ON kb_access_grants
    FOR EACH ROW
    WHEN (NEW.subject_table = 'kb_cogmaps' AND NEW.principal_table = 'kb_profiles'
          AND NEW.principal_id = NEW.granted_by_profile_id)
    EXECUTE FUNCTION kb_access_grants_records_cogmap_creator();

-- ---------------------------------------------------------------------------
-- Section 2. Whose map.
-- ---------------------------------------------------------------------------
-- NULL when the map has a binding to a live shared team: a soft-deleted team confers nothing
-- (20261110000100), so it does not make the map shared. Otherwise its creator when a person, else
-- the person of its one personal-team binding. Every candidate is gated by profile_is_person: a
-- machine's or a connection's agent profile is not personal space (20261113000000).
CREATE FUNCTION cogmap_personal_owner(p_cogmap uuid)
RETURNS uuid LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT CASE
             WHEN EXISTS (SELECT 1 FROM kb_team_cogmaps tc
                            JOIN kb_teams t ON t.id = tc.team_id
                           WHERE tc.cogmap_id = c.id
                             AND t.personal_of IS NULL AND t.is_active) THEN NULL
             WHEN profile_is_person(c.created_by_profile_id) THEN c.created_by_profile_id
             ELSE (SELECT CASE WHEN count(DISTINCT t.personal_of) = 1
                               THEN (array_agg(t.personal_of))[1] END
                     FROM kb_team_cogmaps tc
                     JOIN kb_teams t ON t.id = tc.team_id
                    WHERE tc.cogmap_id = c.id
                      AND profile_is_person(t.personal_of))
           END
      FROM kb_cogmaps c
     WHERE c.id = p_cogmap;
$$;
COMMENT ON FUNCTION cogmap_personal_owner(uuid) IS
  'The person whose personal space a cognitive map is, or NULL: NULL when it is bound to a live shared '
  'team; else its creator when a person; else the person of its single personal-team binding.';

CREATE FUNCTION cogmap_is_personal(p_cogmap uuid)
RETURNS boolean LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT cogmap_personal_owner(p_cogmap) IS NOT NULL;
$$;
COMMENT ON FUNCTION cogmap_is_personal(uuid) IS
  'Whether a cognitive map is personal space (cogmap_personal_owner is not NULL).';

-- ---------------------------------------------------------------------------
-- Section 3. One person per map, for every writer of a binding.
-- ---------------------------------------------------------------------------
-- A binding to a shared team, or to a machine's personal team, is not this rule's business. A
-- binding to a person's personal team is allowed when that person created the map; on a map with no
-- person as creator, when no other person's personal team is bound to it.
CREATE FUNCTION cogmap_personal_bind_refusal(p_cogmap uuid, p_team uuid)
RETURNS text LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT CASE
             WHEN NOT profile_is_person(t.personal_of) THEN NULL
             WHEN profile_is_person(c.created_by_profile_id) THEN
                  CASE WHEN t.personal_of = c.created_by_profile_id THEN NULL
                       ELSE 'a cognitive map can be bound to a personal team only when that team''s '
                            'person created the map; share it through a shared team instead' END
             WHEN EXISTS (SELECT 1 FROM kb_team_cogmaps tc
                            JOIN kb_teams o ON o.id = tc.team_id
                           WHERE tc.cogmap_id = p_cogmap
                             AND o.personal_of IS DISTINCT FROM t.personal_of
                             AND profile_is_person(o.personal_of))
                  THEN 'a cognitive map with no person as its creator can be bound to one person''s '
                       'personal team at most; share it through a shared team instead'
           END
      FROM kb_teams t, kb_cogmaps c
     WHERE t.id = p_team AND c.id = p_cogmap;
$$;
COMMENT ON FUNCTION cogmap_personal_bind_refusal(uuid, uuid) IS
  'Why binding a cognitive map to a team would break the one-person rule, or NULL when it would not. '
  'Read by the bind service for its refusal and by the kb_team_cogmaps trigger for every writer.';

CREATE FUNCTION kb_team_cogmaps_one_person()
RETURNS trigger LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_refusal text;
BEGIN
    -- Two binds of one map, each to a different person's personal team, must not both pass the
    -- check below: hold the map row so they decide one after the other.
    PERFORM 1 FROM kb_cogmaps WHERE id = NEW.cogmap_id FOR NO KEY UPDATE;
    v_refusal := cogmap_personal_bind_refusal(NEW.cogmap_id, NEW.team_id);
    IF v_refusal IS NOT NULL THEN
        RAISE EXCEPTION 'kb_team_cogmaps: %', v_refusal USING ERRCODE = '42501';
    END IF;
    RETURN NEW;
END;
$$;
COMMENT ON FUNCTION kb_team_cogmaps_one_person() IS
  'Refuses a kb_team_cogmaps row that cogmap_personal_bind_refusal refuses (42501).';

-- ---------------------------------------------------------------------------
-- Section 4. Existing conflicts stop the migration (R1).
-- ---------------------------------------------------------------------------
-- Before the trigger exists, so the check reads the bindings as they stand. Ids and counts only.
DO $$
DECLARE
    v_found text;
BEGIN
    SELECT string_agg(format('%s (%s)', b.cogmap_id, b.why), ', ' ORDER BY b.cogmap_id)
      INTO v_found
      FROM (SELECT tc.cogmap_id,
                   CASE WHEN count(DISTINCT t.personal_of) > 1
                        THEN 'bound to more than one person''s personal team'
                        ELSE 'bound to a personal team that is not its creator''s' END AS why
              FROM kb_team_cogmaps tc
              JOIN kb_teams t ON t.id = tc.team_id
              JOIN kb_cogmaps c ON c.id = tc.cogmap_id
             WHERE profile_is_person(t.personal_of)
             GROUP BY tc.cogmap_id, c.created_by_profile_id
            HAVING count(DISTINCT t.personal_of) > 1
                OR (profile_is_person(c.created_by_profile_id)
                    AND bool_or(t.personal_of <> c.created_by_profile_id))) b;
    IF v_found IS NOT NULL THEN
        RAISE EXCEPTION 'cognitive maps break the one-person rule (%): unbind them, then re-run this migration. The pre-flight is temper-artifacts runbooks/2026-10-31-private-cogmaps-one-person-preflight.sql', v_found
            USING ERRCODE = 'check_violation';
    END IF;
END;
$$;

CREATE TRIGGER kb_team_cogmaps_one_person
    BEFORE INSERT OR UPDATE ON kb_team_cogmaps
    FOR EACH ROW EXECUTE FUNCTION kb_team_cogmaps_one_person();

SELECT declare_migration(
    20261115000000,
    'additive',
    'Adds the nullable kb_cogmaps.created_by_profile_id (backfilled from the ledger), the functions '
    'cogmap_genesis_creator, cogmap_personal_owner, cogmap_is_personal and cogmap_personal_bind_refusal, '
    'a BEFORE INSERT OR UPDATE trigger on kb_team_cogmaps, and an AFTER INSERT trigger on kb_access_grants '
    'that records a new map''s creator from its genesis self-grant. No existing function, column or '
    'constraint is changed, and no existing query''s result shape moves. A binary without this change '
    'keeps working for every bind the rule allows, and the maps it creates get their creator; a bind into '
    'a personal team the rule refuses fails (42501). Aborts, changing nothing, while a map already breaks '
    'the rule.'
);
