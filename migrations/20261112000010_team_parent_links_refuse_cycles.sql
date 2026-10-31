-- A team is never its own ancestor (team DAG review L9).
--
-- kb_teams_parents has refused only the one-step cycle (`CHECK (child_id <> parent_id)`). A longer
-- one — A under B under A — was refused by nothing. team_ancestors still terminates over a cycle
-- (it recurses with UNION), but every team on the cycle becomes every other's ancestor, so each
-- reads everything attached to the others and inherits their grants: mutual full reach that no one
-- granted.
--
-- No API writes a cycle today. The only link writer is create_team, which links a team born in the
-- same transaction (it has no descendants), and the personal-team trigger's structural link under
-- temper-system. A re-parent, attach-parent or second-parent endpoint (review L1) would be the first
-- door able to close one, and operator SQL can already. So the refusal lives here, where every
-- writer meets it, rather than in the endpoint that does not exist yet.
--
-- THE CHECK. Before a link (child C, parent P) is written: if C is P, or C is already an ancestor of
-- P, the link would close a cycle, and it is refused with check_violation. The walk follows every
-- link regardless of kb_teams.is_active: a cycle through a soft-deleted team is still a cycle, and
-- would come back whole if that team were restored.
--
-- CONCURRENCY. Two transactions linking A under B and B under A each see no cycle in the other's
-- uncommitted row. So the check takes one transaction-scoped advisory lock first, which serializes
-- link writes against each other only. Links are rare (team creation and, later, re-parenting), and
-- the lock is released at commit or rollback.
--
-- EXISTING ROWS. The migration refuses to apply over a cycle already present, and names one team on
-- it, so an operator can untangle it by hand rather than ship a trigger over a broken graph.

DO $$
DECLARE
    v_team uuid;
BEGIN
    WITH RECURSIVE walk(origin, ancestor) AS (
        SELECT child_id, parent_id FROM kb_teams_parents
        UNION
        SELECT w.origin, tp.parent_id
          FROM walk w
          JOIN kb_teams_parents tp ON tp.child_id = w.ancestor
    )
    SELECT origin INTO v_team FROM walk WHERE origin = ancestor LIMIT 1;
    IF v_team IS NOT NULL THEN
        RAISE EXCEPTION 'kb_teams_parents already holds a cycle through team %: remove a link on it before this migration', v_team;
    END IF;
END $$;

CREATE OR REPLACE FUNCTION team_parent_link_refuses_cycle()
RETURNS trigger LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
BEGIN
    PERFORM pg_advisory_xact_lock(hashtext('kb_teams_parents:cycle_check'));
    IF EXISTS (
        WITH RECURSIVE up(team_id) AS (
            SELECT NEW.parent_id
            UNION
            SELECT tp.parent_id
              FROM up
              JOIN kb_teams_parents tp ON tp.child_id = up.team_id
        )
        SELECT 1 FROM up WHERE team_id = NEW.child_id
    ) THEN
        RAISE EXCEPTION 'a team cannot be linked under one of its own descendants'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;
COMMENT ON FUNCTION team_parent_link_refuses_cycle() IS
  'Refuses a kb_teams_parents link whose child is already an ancestor of its parent (a cycle). '
  'Walks every link, live or soft-deleted. Serialized by a transaction-scoped advisory lock.';

CREATE TRIGGER trg_team_parent_link_refuses_cycle
    BEFORE INSERT OR UPDATE OF child_id, parent_id ON kb_teams_parents
    FOR EACH ROW EXECUTE FUNCTION team_parent_link_refuses_cycle();

SELECT declare_migration(
    20261112000010,
    'additive',
    'New trigger function team_parent_link_refuses_cycle() and BEFORE INSERT OR UPDATE trigger on '
    'kb_teams_parents: a link that would make a team its own ancestor is refused. No API writes one '
    'today; refuses to apply over an existing cycle. No row is rewritten.'
);
