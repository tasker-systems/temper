#!/usr/bin/env python3
"""How long does an act hold the public_within effects lock?

    python3 scripts/visibility-cost/public_within_effects_cost.py [--rows N ...] [--samples S]
                                                                  [--server DSN]

Every act that can change whether a public_within row confers takes one instance-wide exclusive
advisory lock (`_public_within_effects_lock`) early in its transaction and holds it to the end, while
it snapshots the rows it can flip, writes, and records each flip
(`_public_within_effects_before`, `_public_within_transitions`). So an act's wall time is, to within
its own lock waits, how long it holds every other such act off. The cost grows with the rows an act
can flip; the per-row cost is `public_within_row_effective`, evaluated once before and once after.

For each `--rows` N it builds a scratch database `temper_pw_effects_cost` from every migration in the
tree, and a world: the root, `org` under it, N/100 divisions under `org`, 100 squads under each
division, and one row (squad, its division) per squad, so N rows. Then it times, on one warm
connection, each act's SQL function inside a subtransaction that is rolled back, so every sample
starts from the same world:

  link          `_admin_team_parent_link_act`: a new team linked under one division (the snapshot
                reads that division's 100 rows; nothing flips)
  delete        `_admin_team_lifecycle_act` deleted: one division soft-deleted (its 100 rows go
                dormant through liveness)
  root forbid   `_admin_public_within_policy_write` on the root: the instance-wide off (every row
                flips)
  open          `_public_within_open_locks` and `_admin_public_within_write`: one more row opened
                (no snapshot; the lock only)

and, per row, `public_within_row_effective` and its parts over every row, so a change to the per-row
cost can be read directly. Each number is the median of `--samples` calls after two warm-up calls.
Absolute milliseconds are this host's.

It WRITES only to the scratch database, and drops and recreates it on every run.
"""

import argparse
import os
import statistics
import subprocess
import sys

SERVER = "postgresql://temper:temper@localhost:5437"
DB = "temper_pw_effects_cost"
REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def run(cmd):
    p = subprocess.run(cmd, capture_output=True, text=True)
    if p.returncode != 0:
        sys.exit(f"{' '.join(cmd[:4])}… failed:\n{p.stderr.strip()[:2000]}")
    return p.stdout


def psql(dsn, sql):
    return run(["psql", dsn, "-X", "-q", "-t", "-A", "-v", "ON_ERROR_STOP=1", "-c", sql])


WORLD = r"""
DO $w$
DECLARE
    v_root uuid; v_org uuid; v_div uuid; v_sys uuid; i int;
BEGIN
    SELECT id INTO v_root FROM kb_teams WHERE slug = 'temper-system';
    SELECT id INTO v_sys FROM kb_profiles WHERE handle = 'system';
    INSERT INTO kb_teams (slug, name) VALUES ('org', 'org') RETURNING id INTO v_org;
    INSERT INTO kb_teams_parents (parent_id, child_id) VALUES (v_root, v_org);
    FOR i IN 1..%(divisions)d LOOP
        INSERT INTO kb_teams (slug, name) VALUES ('d' || i, 'd' || i) RETURNING id INTO v_div;
        INSERT INTO kb_teams_parents (parent_id, child_id) VALUES (v_org, v_div);
        WITH sq AS (
            INSERT INTO kb_teams (slug, name)
            SELECT 'd' || i || 's' || j, 'd' || i || 's' || j FROM generate_series(1, 100) j
            RETURNING id
        ), linked AS (
            INSERT INTO kb_teams_parents (parent_id, child_id) SELECT v_div, id FROM sq
            RETURNING child_id
        )
        INSERT INTO kb_team_public_within (team_id, anchor_id, opened_by)
        SELECT child_id, v_div, v_sys FROM linked;
    END LOOP;
    INSERT INTO kb_teams (slug, name) VALUES ('newcomer', 'newcomer');
    INSERT INTO kb_teams (slug, name) VALUES ('extra', 'extra');
    INSERT INTO kb_teams_parents (parent_id, child_id)
    SELECT id, (SELECT id FROM kb_teams WHERE slug = 'extra') FROM kb_teams WHERE slug = 'd1';
END
$w$;
ANALYZE;
"""

TIMER = r"""
CREATE FUNCTION pg_temp.time_act(p_sql text, p_n int) RETURNS SETOF float8 LANGUAGE plpgsql AS $t$
DECLARE
    t0 timestamptz;
    v  float8;
    i  int;
BEGIN
    FOR i IN 1..p_n + 2 LOOP
        BEGIN
            t0 := clock_timestamp();
            EXECUTE p_sql;
            v := extract(epoch FROM clock_timestamp() - t0) * 1000;
            RAISE EXCEPTION 'undo' USING ERRCODE = 'P0001';
        EXCEPTION WHEN raise_exception THEN
            IF i > 2 THEN
                RETURN NEXT v;
            END IF;
        END;
    END LOOP;
END
$t$;
"""

SYS = (
    "(SELECT e.id FROM kb_entities e JOIN kb_profiles p ON p.id = e.profile_id "
    "WHERE p.handle = 'system' AND e.name = 'system')"
)
ACTOR = "(SELECT id FROM kb_profiles WHERE handle = 'system')"


def team(slug):
    return f"(SELECT id FROM kb_teams WHERE slug = '{slug}')"


ACTS = {
    "link": f"SELECT _admin_team_parent_link_act({SYS}, {ACTOR}, 'link', {team('d1')}, "
    f"{team('newcomer')})",
    "delete": f"SELECT _admin_team_lifecycle_act({SYS}, {ACTOR}, {team('d1')}, 'deleted')",
    "root forbid": f"SELECT _admin_public_within_policy_write({SYS}, {ACTOR}, "
    f"{team('temper-system')}, true)",
    "open": f"SELECT _public_within_open_locks({team('extra')}, {team('org')}), "
    f"_admin_public_within_write({SYS}, {ACTOR}, {team('extra')}, {team('org')}, "
    f"'opened', false)",
}

# Per row, over every row: the whole predicate and each of its parts.
PER_ROW = {
    "public_within_row_effective": "public_within_row_effective(pw.team_id, pw.anchor_id, pw.instance_wide)",
    "  public_within_allowed_over": "public_within_allowed_over(pw.team_id)",
    "  team_is_root_or_gating": "team_is_root_or_gating(pw.team_id)",
    "  live chain (team_ancestors)": "EXISTS (SELECT 1 FROM team_ancestors(pw.team_id) a WHERE a.team_id = pw.anchor_id)",
    "  anchor_subtree_holds_pool": "anchor_subtree_holds_pool(pw.anchor_id)",
}


def timed(dsn, sql, samples):
    q = sql.replace("'", "''")
    out = psql(dsn, TIMER + f"SELECT * FROM pg_temp.time_act('{q}', {samples});")
    return statistics.median(float(x) for x in out.split())


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--rows", type=int, nargs="+", default=[1000, 5000])
    ap.add_argument("--samples", type=int, default=15)
    ap.add_argument("--server", default=SERVER)
    args = ap.parse_args()

    results = {}
    for n in args.rows:
        dsn = f"{args.server}/{DB}"
        psql(f"{args.server}/postgres", f"DROP DATABASE IF EXISTS {DB} WITH (FORCE)")
        psql(f"{args.server}/postgres", f"CREATE DATABASE {DB} TEMPLATE template0")
        run(
            [
                "sqlx",
                "migrate",
                "run",
                "--source",
                os.path.join(REPO, "migrations"),
                "--database-url",
                dsn,
            ]
        )
        psql(dsn, WORLD % {"divisions": max(1, n // 100)})
        rows = int(psql(dsn, "SELECT count(*) FROM kb_team_public_within").strip())
        col = {}
        for name, sql in ACTS.items():
            col[name] = timed(dsn, sql, args.samples)
        for name, expr in PER_ROW.items():
            ms = timed(
                dsn,
                f"SELECT count(*) FILTER (WHERE {expr}) FROM kb_team_public_within pw",
                args.samples,
            )
            col[name + " (per row)"] = ms / rows
        results[rows] = col

    sizes = sorted(results)
    print(
        f"median ms, {args.samples} samples after two warm-ups; per-row lines are one evaluation "
        f"over every row, divided by the row count\n"
    )
    print("| act | " + " | ".join(f"{n} rows" for n in sizes) + " |")
    print("|---|" + "---|" * len(sizes))
    for name in results[sizes[0]]:
        print(f"| {name} | " + " | ".join(f"{results[n][name]:.3f}" for n in sizes) + " |")


if __name__ == "__main__":
    main()
