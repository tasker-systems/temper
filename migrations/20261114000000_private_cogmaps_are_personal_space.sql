-- Private cognitive maps are personal space, and personal to at most one person.
--
-- Task 01a1933d-2944-77ee-9323-c22c034c2e69, ruled 2026-10-31, the follow-up to 20261113000000
-- (personal space is its owner's alone). A team bound to a map reads the map and every resource
-- homed in it (20260629000006), so a private map is as personal as a private context, and a system
-- admin's arm stops at it the same way.
--
-- A map belongs to at most one person:
--
--   cogmap_personal_owner(map)   NULL when the map is bound to a live shared team (it has been
--                                shared deliberately); otherwise its creator, when a person; otherwise
--                                the one person whose personal team it is bound to; otherwise NULL.
--
-- Two people's claims on one map are refused rather than reconciled. The only binding to a personal
-- team anyone needs is a person binding their own map to their own personal team, so their machine
-- clients read it; sharing between two people is what a shared team is for. So a binding to a
-- person's personal team is refused, whoever asks, unless that person is the map's creator, or the
-- map has no person as creator and is bound to no other person's personal team (TC001).
--
-- Nothing stored a map's creator. Genesis fires as the system actor; the creator appears only as the
-- grantee of the read+write+grant seed (db_backend.rs, cogmap_genesis), and a system admin granting
-- themselves on a map writes a row of the same shape. What tells them apart is time: the seed is
-- written in the genesis transaction, so its row and its grant_created event carry the map's own
-- `created`. kb_cogmaps.created_by_profile_id records it, from that seed: backfilled from the event,
-- and set going forward by a trigger on the seed's insert, so no binary has to write it.
--
-- resource_personal_owner now also answers for a resource homed in a personal map, so every gate
-- 20261113000000 built on it (the grant door, re-embed, the reblock sweep, the admin ledger) stops at
-- a personal map's resources too. Its context arm is carried verbatim from 20261113000000.

ALTER TABLE kb_cogmaps
    ADD COLUMN created_by_profile_id uuid REFERENCES kb_profiles(id) ON DELETE SET NULL;
COMMENT ON COLUMN kb_cogmaps.created_by_profile_id IS
  'The profile that created the map: the grantee of the genesis read+write+grant seed. NULL for '
  'maps created before the grant chokepoint (20260718000010) or by the system.';

-- The backfill: the creator seed's grant_created event, recognised by being a self-grant written in
-- the genesis transaction (occurred_at equals the map's created; both are that transaction's now()).
UPDATE kb_cogmaps c
   SET created_by_profile_id = s.granted_by
  FROM (SELECT DISTINCT ON ((e.payload->>'subject_id')::uuid)
               (e.payload->>'subject_id')::uuid AS cogmap_id,
               (e.payload->>'granted_by')::uuid AS granted_by,
               e.occurred_at
          FROM kb_events e
          JOIN kb_event_types et ON et.id = e.event_type_id
         WHERE et.name = 'grant_created'
           AND e.payload->>'subject_table' = 'kb_cogmaps'
           AND e.payload->>'principal_table' = 'kb_profiles'
           AND e.payload->>'principal_id' = e.payload->>'granted_by'
         ORDER BY (e.payload->>'subject_id')::uuid, e.occurred_at, e.id) s
 WHERE s.cogmap_id = c.id
   AND s.occurred_at = c.created
   AND c.created_by_profile_id IS NULL
   AND EXISTS (SELECT 1 FROM kb_profiles p WHERE p.id = s.granted_by);

-- Going forward: the seed's own insert records the creator. Only genesis creates a map, so a
-- self-grant whose transaction is the map's (created = now()) is the seed; any later self-grant, an
-- admin's included, runs in a later transaction and changes nothing. Set once.
CREATE OR REPLACE FUNCTION _cogmap_record_creator()
RETURNS trigger LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    IF NEW.subject_table = 'kb_cogmaps'
       AND NEW.principal_table = 'kb_profiles'
       AND NEW.principal_id = NEW.granted_by_profile_id THEN
        UPDATE kb_cogmaps
           SET created_by_profile_id = NEW.principal_id
         WHERE id = NEW.subject_id
           AND created_by_profile_id IS NULL
           AND created = now();
    END IF;
    RETURN NULL;
END;
$$;
COMMENT ON FUNCTION _cogmap_record_creator() IS
  'Records kb_cogmaps.created_by_profile_id from the genesis creator seed: a profile self-grant on '
  'the map inserted in the transaction that created it.';

CREATE TRIGGER kb_access_grants_record_cogmap_creator
    AFTER INSERT ON kb_access_grants
    FOR EACH ROW EXECUTE FUNCTION _cogmap_record_creator();

CREATE OR REPLACE FUNCTION cogmap_personal_owner(p_cogmap uuid)
RETURNS uuid LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT CASE
             -- Bound to a live shared team: shared on purpose, nobody's personal space. A
             -- soft-deleted team confers nothing (20261110000100), so it shares nothing either.
             WHEN EXISTS (SELECT 1 FROM kb_team_cogmaps tc
                            JOIN kb_teams t ON t.id = tc.team_id
                           WHERE tc.cogmap_id = c.id
                             AND t.personal_of IS NULL AND t.is_active) THEN NULL
             WHEN profile_is_person(c.created_by_profile_id) THEN c.created_by_profile_id
             -- No person created it: the person whose personal team it is bound to, when there is
             -- exactly one. TC001 keeps there from being two; a map that predates it with two is
             -- left to its bindings' owners to resolve.
             ELSE (SELECT (array_agg(DISTINCT t.personal_of))[1]
                     FROM kb_team_cogmaps tc
                     JOIN kb_teams t ON t.id = tc.team_id
                    WHERE tc.cogmap_id = c.id
                      AND profile_is_person(t.personal_of)
                   HAVING count(DISTINCT t.personal_of) = 1)
           END
      FROM kb_cogmaps c
     WHERE c.id = p_cogmap;
$$;
COMMENT ON FUNCTION cogmap_personal_owner(uuid) IS
  'The person whose personal space a cognitive map is, or NULL: NULL when bound to a live shared '
  'team; else its creator when a person; else the one person whose personal team it is bound to.';

CREATE OR REPLACE FUNCTION cogmap_is_personal(p_cogmap uuid)
RETURNS boolean LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT cogmap_personal_owner(p_cogmap) IS NOT NULL;
$$;
COMMENT ON FUNCTION cogmap_is_personal(uuid) IS
  'Whether a cognitive map is personal space (cogmap_personal_owner is not NULL).';

-- A resource is personal space when its home context is, or (new here) its home map is.
CREATE OR REPLACE FUNCTION resource_personal_owner(p_resource uuid)
RETURNS uuid LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT COALESCE(
        (SELECT context_personal_owner(h.anchor_id)
           FROM kb_resource_homes h
          WHERE h.resource_id = p_resource AND h.anchor_table = 'kb_contexts'
          LIMIT 1),
        (SELECT cogmap_personal_owner(h.anchor_id)
           FROM kb_resource_homes h
          WHERE h.resource_id = p_resource AND h.anchor_table = 'kb_cogmaps'
          LIMIT 1));
$$;
COMMENT ON FUNCTION resource_personal_owner(uuid) IS
  'The profile whose personal space a resource is homed in (context_personal_owner of its home '
  'context, else cogmap_personal_owner of its home map), or NULL.';

-- The bind refusal. Whoever asks, a system admin included: a map is bound to a person's personal
-- team only for that person. The map row is locked first so two concurrent binds of a map with no
-- person creator, into two people's personal teams, cannot both pass the check.
CREATE OR REPLACE FUNCTION _cogmap_binding_stays_one_persons()
RETURNS trigger LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE
    v_person  uuid;
    v_creator uuid;
BEGIN
    SELECT personal_of INTO v_person FROM kb_teams WHERE id = NEW.team_id;
    IF NOT profile_is_person(v_person) THEN
        RETURN NEW;  -- a shared team, or an agent's personal team: not a person's personal space
    END IF;

    SELECT created_by_profile_id INTO v_creator
      FROM kb_cogmaps WHERE id = NEW.cogmap_id
       FOR NO KEY UPDATE;

    IF profile_is_person(v_creator) THEN
        IF v_creator <> v_person THEN
            RAISE EXCEPTION 'cognitive map % is bound to a personal team only by its creator',
                  NEW.cogmap_id USING ERRCODE = 'TC001';
        END IF;
    ELSIF EXISTS (SELECT 1 FROM kb_team_cogmaps tc
                    JOIN kb_teams t ON t.id = tc.team_id
                   WHERE tc.cogmap_id = NEW.cogmap_id
                     AND t.personal_of <> v_person
                     AND profile_is_person(t.personal_of)
                     -- On UPDATE, the row being replaced is not another binding.
                     AND NOT (TG_OP = 'UPDATE' AND tc.cogmap_id = OLD.cogmap_id
                              AND tc.team_id = OLD.team_id)) THEN
        RAISE EXCEPTION 'cognitive map % is already bound to another person''s personal team',
              NEW.cogmap_id USING ERRCODE = 'TC001';
    END IF;
    RETURN NEW;
END;
$$;
COMMENT ON FUNCTION _cogmap_binding_stays_one_persons() IS
  'Refuses (TC001) binding a cognitive map to a person''s personal team unless that person is its '
  'creator, or it has no person creator and no other person''s personal-team binding.';

CREATE TRIGGER kb_team_cogmaps_stay_one_persons
    BEFORE INSERT OR UPDATE ON kb_team_cogmaps
    FOR EACH ROW EXECUTE FUNCTION _cogmap_binding_stays_one_persons();

SELECT declare_migration(
    20261114000000,
    'additive',
    'Adds the nullable kb_cogmaps.created_by_profile_id (backfilled from the genesis creator seed, and '
    'set by an AFTER INSERT trigger on kb_access_grants), cogmap_personal_owner and cogmap_is_personal, '
    'and a BEFORE INSERT OR UPDATE trigger on kb_team_cogmaps refusing (TC001) a binding to a person''s personal '
    'team other than the map''s creator''s. Replaces resource_personal_owner (signature unchanged) so '
    'it also answers for a resource homed in a personal map. A binary without this change keeps '
    'working; a bind the trigger refuses surfaces to it as a database error.'
);
