-- A team is never its own ancestor, and temper-system, the root, takes no parent.
--
-- kb_teams_parents has refused only the one-step cycle (`CHECK (child_id <> parent_id)`). A longer
-- one (A under B under A) was refused by nothing. team_ancestors still terminates over a cycle (it
-- recurses with UNION), but every team on the cycle becomes every other's ancestor, so each reads
-- everything attached to the others and inherits their grants. No API writes a cycle today; the
-- refusal lives here so that every writer meets it, including a future re-parent endpoint and
-- operator SQL.
--
-- THE CHECK. Before a link (child C, parent P) is written: if C is temper-system it is refused (the
-- root takes no parent). If P is temper-system the link is allowed without a walk: closing a cycle
-- would need the root to descend from C, which the first rule forbids. That covers every sign-up's
-- personal-team link (sync_personal_team), so sign-ups never queue on the lock below. Otherwise,
-- if C is already an ancestor of P the link is refused. The walk ignores kb_teams.is_active: a cycle
-- through a soft-deleted team is still a cycle, and would come back whole on a restore.
--
-- CONCURRENCY. Two transactions linking A under B and B under A each miss the other's uncommitted
-- row, so the walk runs after one transaction-scoped advisory lock that serializes such links. The
-- walk is a new statement after the lock, so under READ COMMITTED it sees a link committed while
-- this transaction waited. Under REPEATABLE READ it would not; no link writer runs at that level.
--
-- EXISTING ROWS. Under a table lock, the migration refuses to apply over a cycle or a parented root.

LOCK TABLE kb_teams_parents IN SHARE ROW EXCLUSIVE MODE;

DO $$
DECLARE
    v_team uuid;
BEGIN
    IF EXISTS (SELECT 1 FROM kb_teams_parents tp JOIN kb_teams t ON t.id = tp.child_id
                WHERE t.slug = 'temper-system') THEN
        RAISE EXCEPTION 'temper-system has a parent in kb_teams_parents: remove that link before this migration';
    END IF;
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
    IF EXISTS (SELECT 1 FROM kb_teams WHERE id = NEW.child_id AND slug = 'temper-system') THEN
        RAISE EXCEPTION 'temper-system is the root of the team tree and takes no parent'
            USING ERRCODE = 'check_violation';
    END IF;
    -- A link under the root cannot close a cycle while the root has no parent (refused above).
    IF EXISTS (SELECT 1 FROM kb_teams WHERE id = NEW.parent_id AND slug = 'temper-system') THEN
        RETURN NEW;
    END IF;
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
  'Refuses a kb_teams_parents link that gives temper-system a parent, or whose child is already an '
  'ancestor of its parent (a cycle). Links under temper-system skip the walk: they cannot close one. '
  'Otherwise walks every link, live or soft-deleted, under a transaction-scoped advisory lock.';

CREATE TRIGGER trg_team_parent_link_refuses_cycle
    BEFORE INSERT OR UPDATE OF child_id, parent_id ON kb_teams_parents
    FOR EACH ROW EXECUTE FUNCTION team_parent_link_refuses_cycle();

SELECT declare_migration(
    20261112000010,
    'additive',
    'New trigger function team_parent_link_refuses_cycle() and BEFORE INSERT OR UPDATE trigger on '
    'kb_teams_parents: a link that gives temper-system a parent, or makes a team its own ancestor, '
    'is refused. No API writes one today; refuses to apply over an existing cycle or parented root. '
    'No row is rewritten.'
);
