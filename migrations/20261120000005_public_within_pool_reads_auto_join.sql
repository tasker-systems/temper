-- anchor_subtree_holds_pool counts an auto-join team as well as the root and the gating team. Each
-- brings the whole instance into the anchor's subtree, so a public_within row over it would read
-- instance-wide.
--
-- Split from 20261120000000 only because this body reads kb_teams.auto_join_role, which
-- temper-substrate's pre-auto-join test baseline (tests/common/mod.rs) omits, so that harness skips
-- this file as it skips the other auto-join migrations. Every other database applies both, so the
-- function below is the one that answers.
--
-- Body-only CREATE OR REPLACE: signature, return type and volatility unchanged.

CREATE OR REPLACE FUNCTION anchor_subtree_holds_pool(p_anchor uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1
          FROM kb_teams t
         WHERE (t.id = p_anchor OR t.id IN (SELECT d.team_id FROM team_live_descendants(p_anchor) d))
           AND ( t.auto_join_role IS NOT NULL
              OR t.slug = 'temper-system'
              OR t.slug = (SELECT s.gating_team_slug FROM kb_system_settings s WHERE s.id = 1)));
$$;

COMMENT ON FUNCTION anchor_subtree_holds_pool(uuid) IS
$c$True when the anchor is the root (temper-system), or the anchor itself or a live team beneath it
is an auto-join team or the gating team. Each brings the whole instance into the anchor's subtree
(every personal team sits under the root), so a public_within row over it would
read instance-wide; only an admin's instance_wide row survives it (public_within_row_effective). A
pool team below a dead team is not counted, because it confers nothing upward (team_live_descendants
does not walk through a dead team).$c$;

SELECT declare_migration(
    20261120000005,
    'additive',
    'Body-only CREATE OR REPLACE, signature and return type unchanged, of anchor_subtree_holds_pool: '
    'an auto-join team under the anchor counts as a pool, as the root and the gating team already do. No row is '
    'rewritten, and the function runs only for a candidate public_within row.'
);
