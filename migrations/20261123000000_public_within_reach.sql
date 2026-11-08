-- public_within: how a reader reaches one object, for the single-object reads.
--
-- context_reached_via(p, c) and resource_reached_via(p, r) answer 'public_within' when p reads the
-- object and does not read it by membership, and 'membership' otherwise. They are called from the
-- SELECT list of the query that already authorizes the read (context get, resource get), so they
-- run only for a row the reader passed, in the same statement and snapshot as the gate.
--
-- What a later edit must not break:
--   * One object, one probe. Each first asks whether the object's owning team has any
--     public_within row: an index probe on kb_team_public_within's primary key (team_id leads).
--     With no such row the answer is 'membership' and nothing else runs, so a read of an object
--     whose team was never opened costs one probe. Only then is the membership set asked about
--     that one id. Never call these per row of a list: a list computes the reader's sets once and
--     joins against them.
--   * PL/pgSQL, not SQL, so that a caller does not inline (and plan, on every call) the membership
--     set it almost never reaches.
--   * The precondition is the caller's gate. Under it, reading = membership UNION the
--     public_within arm, so "reads and not by membership" is "reads through public_within".
--   * The answer is a route, never a source: no team, row or person is named.

CREATE FUNCTION context_reached_via(p_profile uuid, p_context uuid)
RETURNS text
LANGUAGE plpgsql
STABLE
AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM kb_contexts c
          JOIN kb_team_public_within pw ON pw.team_id = c.owner_id
         WHERE c.id = p_context AND c.owner_table = 'kb_teams'
    ) THEN
        RETURN 'membership';
    END IF;
    IF EXISTS (
        SELECT 1 FROM contexts_readable_by_membership(p_profile) m WHERE m.context_id = p_context
    ) THEN
        RETURN 'membership';
    END IF;
    RETURN 'public_within';
END;
$$;

COMMENT ON FUNCTION context_reached_via(uuid, uuid) IS
$c$How p_profile reaches p_context: 'public_within' when it reads the context and not by membership
(contexts_readable_by_membership), else 'membership'. PRECONDITION: the caller has already
established, in the same statement, that p_profile reads p_context (context_visible_to); under it
the reading set is membership plus the public_within arm, so the two answers are exhaustive. Called
from the SELECT list of the context get. With no public_within row on the context's owning team it
returns after one index probe and never computes the membership set. Names no team and no person.$c$;

CREATE FUNCTION resource_reached_via(p_profile uuid, p_resource uuid)
RETURNS text
LANGUAGE plpgsql
STABLE
AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1
          FROM kb_resource_homes h
          JOIN kb_contexts c
            ON h.anchor_table = 'kb_contexts' AND c.id = h.anchor_id AND c.owner_table = 'kb_teams'
          JOIN kb_team_public_within pw ON pw.team_id = c.owner_id
         WHERE h.resource_id = p_resource
    ) THEN
        RETURN 'membership';
    END IF;
    IF EXISTS (
        SELECT 1 FROM resources_visible_by_membership(p_profile) m WHERE m.resource_id = p_resource
    ) THEN
        RETURN 'membership';
    END IF;
    RETURN 'public_within';
END;
$$;

COMMENT ON FUNCTION resource_reached_via(uuid, uuid) IS
$c$How p_profile reaches p_resource: 'public_within' when it reads the resource and not by
membership (resources_visible_by_membership), else 'membership'. PRECONDITION: the caller has
already established, in the same statement, that p_profile reads p_resource (resources_visible_to);
under it the reading set is membership plus the resources the public_within arm opens, which are
only those homed in a context owned by a team with a public_within row, so the two answers are
exhaustive. Called from the SELECT list of the resource get. With no public_within row on the
owning team of the resource's home it returns after one index probe and never computes the
membership set. Names no team and no person.$c$;

SELECT declare_migration(
    20261123000000,
    'additive',
    'public_within reach: adds context_reached_via(uuid,uuid) and resource_reached_via(uuid,uuid), '
    'new functions only; nothing is altered or dropped. An older binary never calls them.'
);
