#!/usr/bin/env python3
"""What does `public_within`'s read arm cost?

    python3 scripts/visibility-cost/public_within_cost.py [--samples N] [--profiles K]
                                                          [--head-first VERSION] [--server DSN]

Builds TWO scratch databases on the local dev server and the same synthetic world in each:

  * `temper_pw_cost_base`: every migration before `--head-first` (by default this work's first,
    so the read path without the public_within arm; pass a later version to compare against the
    tree as it stood before that migration);
  * `temper_pw_cost_head`: every migration in the tree (the public_within arm, the wrappers, the
    audit split).

Then it times the read doors, for a handful of profiles, in three cases:

  base        the read path without the public_within arm
  head-0      head with no public_within rows (every instance until a row is opened)
  head-51     head with 50 rows plus one admin instance-wide row on the root

Each head case is timed against base sample by sample: a third scratch database,
`temper_pw_cost_driver`, holds one warm dblink connection to each and alternates the call, so drift
and contention on the host land on both sides of every pair. The merge bar: `head-0` is within
noise of `base`.

Each number is the median wall time of one call, over all samples (cycling through the measured
profiles), on connections that have already run every door twice per profile: a server's pooled
connections have already planned every function body, so a fresh session per sample would charge
each sample a one-off planning cost a server does not pay. The time includes the dblink round
trip, which is the same on both sides. Absolute milliseconds are this host's; read the ratios.

The world: a root-linked `org`, 10 divisions, 5 squads each; 300 people, each on two squads; one
context per squad (40 resources) and per division (20 resources); 4000 edges, each homed in its
source's context; a cognitive map per division joined to it, with 20 findings citing two squad
resources each, for the audit sweep. A row opens squad j of division i to division i (50 rows),
and the root row opens one squad to `temper-system`, which every person's closure holds.

A door is one SQL text run on both sides, or a pair: the text the base's code ran and the text the
head's code runs. `list_visible` is timed both ways: as `main`'s text on both sides (the cost
before the set-based rewrite) and as the pair (`list_visible (set)`: base runs `main`'s text, head
runs the text `context_service::list_visible` runs now, which evaluates the reader's context set
and resource set once per call rather than once per context).

`search_wide` is driven with a constant vector over a world with no chunk rows, so its number is
the visibility hand-off and the empty k-NN, not a vector search; the hand-off is what the public_within
arm touches.

It WRITES only to the three scratch databases, and drops and recreates them on every run.
"""

import argparse
import os
import shutil
import statistics
import subprocess
import sys
import tempfile

SERVER = "postgresql://temper:temper@localhost:5437"
BASE_DB = "temper_pw_cost_base"
HEAD_DB = "temper_pw_cost_head"
DRIVER_DB = "temper_pw_cost_driver"
# This work's first migration; the base is every migration before it.
HEAD_FIRST = 20261120000000

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def run(cmd, **kw):
    p = subprocess.run(cmd, capture_output=True, text=True, **kw)
    if p.returncode != 0:
        sys.exit(f"{' '.join(cmd[:4])}… failed:\n{p.stderr.strip()[:2000]}")
    return p.stdout


def psql(dsn, sql):
    return run(["psql", dsn, "-X", "-q", "-t", "-A", "-v", "ON_ERROR_STOP=1", "-c", sql])


def fresh(server, db, source):
    dsn = f"{server}/{db}"
    psql(f"{server}/postgres", f"DROP DATABASE IF EXISTS {db} WITH (FORCE)")
    psql(f"{server}/postgres", f"CREATE DATABASE {db} TEMPLATE template0")
    run(["sqlx", "migrate", "run", "--source", source, "--database-url", dsn])
    return dsn


def base_migrations(head_first):
    tmp = tempfile.mkdtemp(prefix="pw-cost-")
    src = os.path.join(REPO, "migrations")
    for f in os.listdir(src):
        if f.endswith(".sql") and int(f.split("_", 1)[0]) < head_first:
            shutil.copy(os.path.join(src, f), tmp)
    return tmp


WORLD = r"""
DO $w$
DECLARE
    v_root uuid; v_org uuid; v_div uuid; v_sq uuid; v_ctx uuid; v_ev uuid; v_map uuid;
    v_telos uuid; v_res uuid; v_block uuid; i int; j int; k int;
BEGIN
    SELECT id INTO v_root FROM kb_teams WHERE slug = 'temper-system';
    SELECT id INTO v_ev FROM kb_events ORDER BY id LIMIT 1;
    INSERT INTO kb_teams (slug, name) VALUES ('org', 'org') RETURNING id INTO v_org;
    INSERT INTO kb_teams_parents (parent_id, child_id) VALUES (v_root, v_org);

    FOR k IN 1..300 LOOP
        INSERT INTO kb_profiles (handle, display_name) VALUES ('p' || k, 'p' || k);
        INSERT INTO kb_profile_auth_links (id, profile_id, auth_provider, auth_provider_user_id)
        SELECT uuid_generate_v7(), id, 'cost-idp', handle FROM kb_profiles WHERE handle = 'p' || k;
    END LOOP;

    FOR i IN 1..10 LOOP
        INSERT INTO kb_teams (slug, name) VALUES ('d' || i, 'd' || i) RETURNING id INTO v_div;
        INSERT INTO kb_teams_parents (parent_id, child_id) VALUES (v_org, v_div);
        INSERT INTO kb_contexts (owner_table, owner_id, slug, name)
        VALUES ('kb_teams', v_div, 'ctx-d' || i, 'ctx-d' || i) RETURNING id INTO v_ctx;
        FOR k IN 1..20 LOOP
            INSERT INTO kb_resources (title, origin_uri, ingest_state)
            VALUES ('d' || i || '-r' || k, '', 'complete') RETURNING id INTO v_res;
            INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id,
                                           originator_profile_id, owner_profile_id)
            SELECT v_res, 'kb_contexts', v_ctx, id, id FROM kb_profiles WHERE handle = 'p' || (i * 7 % 300 + 1);
        END LOOP;
        FOR j IN 1..5 LOOP
            INSERT INTO kb_teams (slug, name) VALUES ('d' || i || 's' || j, 'd' || i || 's' || j)
            RETURNING id INTO v_sq;
            INSERT INTO kb_teams_parents (parent_id, child_id) VALUES (v_div, v_sq);
            INSERT INTO kb_contexts (owner_table, owner_id, slug, name)
            VALUES ('kb_teams', v_sq, 'ctx-d' || i || 's' || j, 'ctx-d' || i || 's' || j)
            RETURNING id INTO v_ctx;
            FOR k IN 1..40 LOOP
                INSERT INTO kb_resources (title, origin_uri, ingest_state)
                VALUES ('d' || i || 's' || j || '-r' || k, '', 'complete') RETURNING id INTO v_res;
                INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id,
                                               originator_profile_id, owner_profile_id)
                SELECT v_res, 'kb_contexts', v_ctx, id, id
                  FROM kb_profiles WHERE handle = 'p' || ((i * 5 + j) * 11 % 300 + 1);
                INSERT INTO kb_resource_search_index (resource_id, search_vector)
                VALUES (v_res, to_tsvector('english', 'zenith squad ' || i || ' ' || j));
            END LOOP;
        END LOOP;
    END LOOP;

    -- Two squads per person.
    INSERT INTO kb_team_members (team_id, profile_id, role)
    SELECT t.id, p.id, 'member'::team_role
      FROM kb_profiles p
      JOIN LATERAL (SELECT (substr(p.handle, 2)::int % 50) AS a,
                           (substr(p.handle, 2)::int * 7 % 50) AS b) x ON true
      JOIN kb_teams t ON t.slug IN ('d' || (x.a / 5 + 1) || 's' || (x.a % 5 + 1),
                                    'd' || (x.b / 5 + 1) || 's' || (x.b % 5 + 1))
     WHERE p.handle ~ '^p[0-9]+$'
    ON CONFLICT DO NOTHING;

    -- 4000 edges, each homed in its source's context.
    INSERT INTO kb_edges (id, source_table, source_id, target_table, target_id, edge_kind, label,
                          home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id)
    SELECT uuid_generate_v7(), 'kb_resources', s.resource_id, 'kb_resources', t.resource_id,
           'leads_to', 'relates_to', 'kb_contexts', s.anchor_id, v_ev, v_ev
      FROM (SELECT row_number() OVER (ORDER BY resource_id) AS n, resource_id, anchor_id
              FROM kb_resource_homes) s
      JOIN (SELECT row_number() OVER (ORDER BY resource_id DESC) AS n, resource_id
              FROM kb_resource_homes) t ON t.n = (s.n * 37 % 2200) + 1
     WHERE s.resource_id <> t.resource_id
     LIMIT 4000;

    -- A map per division, with findings citing squad resources (the audit sweep's work).
    FOR i IN 1..10 LOOP
        SELECT id INTO v_div FROM kb_teams WHERE slug = 'd' || i;
        INSERT INTO kb_resources (title, origin_uri) VALUES ('telos-' || i, '') RETURNING id INTO v_telos;
        INSERT INTO kb_cogmaps (name, telos_resource_id) VALUES ('map-' || i, v_telos) RETURNING id INTO v_map;
        INSERT INTO kb_team_cogmaps (cogmap_id, team_id) VALUES (v_map, v_div);
        FOR k IN 1..20 LOOP
            INSERT INTO kb_resources (title, origin_uri, ingest_state)
            VALUES ('finding-' || i || '-' || k, '', 'complete') RETURNING id INTO v_res;
            INSERT INTO kb_resource_homes (resource_id, anchor_table, anchor_id,
                                           originator_profile_id, owner_profile_id)
            SELECT v_res, 'kb_cogmaps', v_map, id, id FROM kb_profiles WHERE handle = 'p1';
            INSERT INTO kb_content_blocks (resource_id, seq, genesis_event_id, last_event_id)
            VALUES (v_res, 0, v_ev, v_ev) RETURNING id INTO v_block;
            INSERT INTO kb_block_provenance (block_id, source_kind, source_id,
                                             contributed_by_event_id, accretion_seq)
            SELECT v_block, 'resource', h.resource_id, v_ev, 0
              FROM kb_resource_homes h JOIN kb_contexts c ON c.id = h.anchor_id
             WHERE c.slug IN ('ctx-d' || i || 's' || (k % 5 + 1), 'ctx-d' || (i % 10 + 1) || 's1')
             ORDER BY h.resource_id LIMIT 2;
        END LOOP;
    END LOOP;
END
$w$;
ANALYZE;
"""

ROWS = r"""
INSERT INTO kb_team_public_within (team_id, anchor_id, opened_by, instance_wide, admin_act)
SELECT s.id, d.id, (SELECT id FROM kb_profiles WHERE handle = 'p1'), false, false
  FROM kb_teams d JOIN kb_teams_parents l ON l.parent_id = d.id JOIN kb_teams s ON s.id = l.child_id
 WHERE d.slug ~ '^d[0-9]+$';
INSERT INTO kb_team_public_within (team_id, anchor_id, opened_by, instance_wide, admin_act)
SELECT (SELECT id FROM kb_teams WHERE slug = 'd1'),
       (SELECT id FROM kb_teams WHERE slug = 'temper-system'),
       (SELECT id FROM kb_profiles WHERE handle = 'p1'), true, true;
ANALYZE kb_team_public_within;
"""

LIST_VISIBLE = """SELECT c.id,
       context_authorable_by_profile($1, c.id),
       (SELECT count(*) FROM kb_resource_homes rh
          JOIN resources_visible_to($1) v ON v.resource_id = rh.resource_id
          JOIN kb_resources r ON r.id = rh.resource_id AND r.is_active
         WHERE rh.anchor_table = 'kb_contexts' AND rh.anchor_id = c.id)
  FROM kb_contexts c WHERE context_visible_to($1, c.id) ORDER BY c.name"""

# `context_service::list_visible` since the set-based rewrite: the same predicates, each evaluated
# once per call.
LIST_VISIBLE_SET = """WITH readable AS MATERIALIZED (SELECT r.context_id FROM contexts_readable_by($1) r),
     visible AS MATERIALIZED (SELECT v.resource_id FROM resources_visible_to($1) v)
SELECT c.id,
       context_authorable_by_profile($1, c.id),
       (SELECT count(*) FROM kb_resource_homes rh
          JOIN visible v ON v.resource_id = rh.resource_id
          JOIN kb_resources r ON r.id = rh.resource_id AND r.is_active
         WHERE rh.anchor_table = 'kb_contexts' AND rh.anchor_id = c.id)
  FROM kb_contexts c WHERE c.id IN (SELECT rc.context_id FROM readable rc) ORDER BY c.name"""

# The single-object gets, as `main` ran them and as they run with `reach`. Two targets, each a
# context and the first resource homed in it; the target lookup is the same text on both sides:
#   * own: a context of one of the reader's own squads. Admitted in every case; with rows every
#     squad is a T, so the reach function runs its membership probe, which hits.
#   * public: a context of a sibling squad in the reader's division that the reader is not on.
#     Unreadable without rows (the gate refuses, reach never runs, as on `main`); with rows the
#     reader reads it only through public_within, so the membership probe runs and misses.
OWN_TARGET = """WITH target AS MATERIALIZED (
    SELECT c.id AS context_id,
           (SELECT h.resource_id FROM kb_resource_homes h
             WHERE h.anchor_table = 'kb_contexts' AND h.anchor_id = c.id
             ORDER BY h.resource_id LIMIT 1) AS resource_id
      FROM kb_team_members tm
      JOIN kb_contexts c ON c.owner_table = 'kb_teams' AND c.owner_id = tm.team_id
     WHERE tm.profile_id = $1
     ORDER BY c.id LIMIT 1)"""

PUBLIC_TARGET = """WITH target AS MATERIALIZED (
    SELECT c.id AS context_id,
           (SELECT h.resource_id FROM kb_resource_homes h
             WHERE h.anchor_table = 'kb_contexts' AND h.anchor_id = c.id
             ORDER BY h.resource_id LIMIT 1) AS resource_id
      FROM kb_team_members tm
      JOIN kb_teams_parents mine ON mine.child_id = tm.team_id
      JOIN kb_teams_parents sib ON sib.parent_id = mine.parent_id
      JOIN kb_contexts c ON c.owner_table = 'kb_teams' AND c.owner_id = sib.child_id
     WHERE tm.profile_id = $1
       AND NOT EXISTS (SELECT 1 FROM kb_team_members m
                        WHERE m.team_id = sib.child_id AND m.profile_id = $1)
     ORDER BY c.id LIMIT 1)"""

CONTEXT_GET = """
SELECT c.id, c.name, c.slug, context_authorable_by_profile($1, c.id), NOT c.is_active{reach}
  FROM kb_contexts c
 WHERE c.id = (SELECT context_id FROM target)
   AND context_visible_to($1, c.id)"""

RESOURCE_GET = """
SELECT r.id, r.title, h.owner_profile_id, c.slug{reach}
  FROM kb_resources r
  JOIN kb_resource_homes h ON h.resource_id = r.id
  JOIN resources_visible_to($1) v
    ON v.resource_id = r.id AND v.resource_id = ANY(ARRAY[(SELECT resource_id FROM target)])
  LEFT JOIN kb_contexts c ON c.id = h.anchor_id AND h.anchor_table = 'kb_contexts'
 WHERE r.id = ANY(ARRAY[(SELECT resource_id FROM target)])"""

CONTEXT_REACH = ", context_reached_via($1, c.id)"
RESOURCE_REACH = ", CASE WHEN true THEN resource_reached_via($1, r.id) END"

DOORS = {
    "context get (own)": (
        OWN_TARGET + CONTEXT_GET.format(reach=""),
        OWN_TARGET + CONTEXT_GET.format(reach=CONTEXT_REACH),
    ),
    "resource get (own)": (
        OWN_TARGET + RESOURCE_GET.format(reach=""),
        OWN_TARGET + RESOURCE_GET.format(reach=RESOURCE_REACH),
    ),
    "context get (public)": (
        PUBLIC_TARGET + CONTEXT_GET.format(reach=""),
        PUBLIC_TARGET + CONTEXT_GET.format(reach=CONTEXT_REACH),
    ),
    "resource get (public)": (
        PUBLIC_TARGET + RESOURCE_GET.format(reach=""),
        PUBLIC_TARGET + RESOURCE_GET.format(reach=RESOURCE_REACH),
    ),
    "list_visible": LIST_VISIBLE,
    "list_visible (set)": (LIST_VISIBLE, LIST_VISIBLE_SET),
    "resources_visible_to": "SELECT count(*) FROM resources_visible_to($1)",
    "edges_visible_to": "SELECT count(*) FROM edges_visible_to($1)",
    "search_wide": "SELECT count(*) FROM search_wide($1, array_fill(0.01::real, ARRAY[768])::vector, "
    "50, NULL, NULL, NULL, 20, 0)",
    "audit_drift_sweep": "SELECT count(*) FROM audit_drift_sweep($1, 50)",
    "graph_atlas_nodes_visible": "SELECT count(*) FROM graph_atlas_nodes_visible($1, "
    "ARRAY(SELECT resource_id FROM kb_resource_homes ORDER BY resource_id LIMIT 200))",
}


# Alternates one call on `a` and one on `b` per sample, flipping which goes first, on two warm
# connections. The doors take the profile as `$1`; it is spliced in as a literal for dblink.
BENCH = """
CREATE FUNCTION pg_temp.bench(qa text, qb text, a text, b text, handles text[], warm int, n int)
RETURNS TABLE(side text, ms float8) LANGUAGE plpgsql AS $b$
DECLARE
    t0 timestamptz;
    d text;
    ids text[];
    pa text[];
    pb text[];
    call text;
BEGIN
    PERFORM dblink_connect('a', a);
    PERFORM dblink_connect('b', b);
    FOREACH d IN ARRAY ARRAY['a', 'b'] LOOP
        ids := ARRAY(SELECT x FROM dblink(d, format(
            'SELECT id::text FROM kb_profiles WHERE handle = ANY(%L) ORDER BY handle', handles))
            AS t(x text));
        IF d = 'a' THEN pa := ids; ELSE pb := ids; END IF;
        FOR k IN 1..array_length(ids, 1) LOOP
            FOR w IN 1..warm LOOP
                PERFORM * FROM dblink(d, replace(CASE WHEN d = 'a' THEN qa ELSE qb END, '$1',
                                                 quote_literal(ids[k]) || '::uuid'))
                    AS t(c text);
            END LOOP;
        END LOOP;
    END LOOP;
    FOR i IN 1..n LOOP
        FOREACH d IN ARRAY CASE WHEN i % 2 = 0 THEN ARRAY['a', 'b'] ELSE ARRAY['b', 'a'] END LOOP
            ids := CASE WHEN d = 'a' THEN pa ELSE pb END;
            call := replace(CASE WHEN d = 'a' THEN qa ELSE qb END, '$1',
                            quote_literal(ids[1 + i % array_length(ids, 1)]) || '::uuid');
            t0 := clock_timestamp();
            PERFORM * FROM dblink(d, call) AS t(c text);
            side := d;
            ms := extract(epoch FROM clock_timestamp() - t0) * 1000;
            RETURN NEXT;
        END LOOP;
    END LOOP;
    PERFORM dblink_disconnect('a');
    PERFORM dblink_disconnect('b');
END $b$;
"""


def measure(driver, handles, samples):
    """Per door, the median ms on base and on head, timed in alternating pairs."""
    out = {}
    arr = "ARRAY[" + ", ".join(f"'{h}'" for h in handles) + "]"
    a = f"dbname={BASE_DB} user=temper password=temper"
    b = f"dbname={HEAD_DB} user=temper password=temper"
    for name, sql in DOORS.items():
        base_sql, head_sql = sql if isinstance(sql, tuple) else (sql, sql)
        qa, qb = (
            # count(door.*), not count(*): a whole-row reference uses every output column. With
            # count(*) the planner drops each unused, non-volatile column of the door subquery, so
            # a STABLE function in a door's SELECT list (reach, a per-context count) never runs
            # and the door is timed as its WHERE clause alone.
            f"SELECT count(door.*)::text FROM ({s}) door".replace("'", "''")
            for s in (base_sql, head_sql)
        )
        rows = psql(
            driver,
            BENCH + f"SELECT side, ms FROM pg_temp.bench('{qa}', '{qb}', '{a}', '{b}', {arr}, 2, "
            f"{samples});",
        )
        times = {"a": [], "b": []}
        for line in rows.split():
            side, ms = line.split("|")
            times[side].append(float(ms))
        out[name] = (statistics.median(times["a"]), statistics.median(times["b"]))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--samples", type=int, default=200)
    ap.add_argument("--profiles", type=int, default=5)
    ap.add_argument("--head-first", type=int, default=HEAD_FIRST)
    ap.add_argument("--server", default=SERVER)
    ap.add_argument(
        "--rows-on-base",
        action="store_true",
        help="open the same rows on base too (base' is then base with rows): for comparing two "
        "trees that both have the public_within arm, e.g. --head-first at a later migration",
    )
    args = ap.parse_args()
    server = args.server

    base_src = base_migrations(args.head_first)
    fresh(server, BASE_DB, base_src)
    head = fresh(server, HEAD_DB, os.path.join(REPO, "migrations"))
    for db in (BASE_DB, HEAD_DB):
        psql(f"{server}/{db}", WORLD)
    psql(f"{server}/postgres", f"DROP DATABASE IF EXISTS {DRIVER_DB} WITH (FORCE)")
    psql(f"{server}/postgres", f"CREATE DATABASE {DRIVER_DB} TEMPLATE template0")
    driver = f"{server}/{DRIVER_DB}"
    psql(driver, "CREATE EXTENSION dblink")
    handles = [f"p{k}" for k in range(1, 300, 300 // args.profiles)]

    zero = measure(driver, handles, args.samples)
    psql(head, ROWS)
    if args.rows_on_base:
        psql(f"{server}/{BASE_DB}", ROWS)
    rows = measure(driver, handles, args.samples)
    results = {
        "base": {d: zero[d][0] for d in DOORS},
        "head-0": {d: zero[d][1] for d in DOORS},
        "base'": {d: rows[d][0] for d in DOORS},
        "head-51": {d: rows[d][1] for d in DOORS},
    }

    reads = psql(
        head,
        "SELECT count(*) FROM resources_visible_to((SELECT id FROM kb_profiles WHERE handle='p1'))",
    ).strip()
    member = psql(
        head,
        "SELECT count(*) FROM resources_visible_by_membership("
        "(SELECT id FROM kb_profiles WHERE handle='p1'))",
    ).strip()

    cases = ["base", "head-0", "base'", "head-51"]
    print(
        f"median ms, {len(handles)} profiles, {args.samples} alternating pairs per door; base' is "
        f"base timed beside head-51 (p1 reads {reads}, {member} by membership, in head-51)\n"
    )
    print(f"| door | {' | '.join(cases)} | head-0/base | head-51/base' |")
    print(f"|---|{'---|' * len(cases)}---|---|")
    for door in DOORS:
        row = [results[c][door] for c in cases]
        print(
            f"| `{door}` | "
            + " | ".join(f"{v:.2f}" for v in row)
            + f" | {row[1] / row[0]:.2f} | {row[3] / row[2]:.2f} |"
        )
    shutil.rmtree(base_src, ignore_errors=True)


if __name__ == "__main__":
    main()
