#!/usr/bin/env python3
"""What does `public_within`'s read arm cost? (temper-artifacts plans/2026-11-06-public-within.md 2.6)

    python3 scripts/visibility-cost/public_within_cost.py [--samples N] [--profiles K]

Builds TWO scratch databases on the local dev server and the same synthetic world in each:

  * `temper_pw_cost_base`: every migration up to Phase 0's last (the read path without arm 5);
  * `temper_pw_cost_head`: every migration in the tree (arm 5, the wrappers, the audit split).

Then it times the five doors the plan names, for a handful of profiles, on base and on head in
four cases:

  base        no switch and no arm exist
  off         head, switch off, 50 rows plus a root row present (the born-denied instance)
  on-0        head, switch on, no rows
  on-51       head, switch on, 50 rows plus one admin root row

The merge bar (plan 2.6): `off` is within noise of `base`.

Each number is the median `Execution Time` of `EXPLAIN (ANALYZE, FORMAT JSON)` over all samples
for all measured profiles. Absolute milliseconds are this host's; read the ratios.

The world: a root-linked `org`, 10 divisions, 5 squads each; 300 people, each on two squads; one
context per squad (40 resources) and per division (20 resources); 4000 edges, each homed in its
source's context; a cognitive map per division joined to it, with 20 findings citing two squad
resources each, for the audit sweep. A row opens squad j of division i to division i (50 rows),
and the root row opens one squad to `temper-system`, which every person's closure holds.

`search_wide` is driven with a constant vector over a world with no chunk rows, so its number is
the visibility hand-off and the empty k-NN, not a vector search; the hand-off is what arm 5 touches.

It WRITES only to the two scratch databases, and drops and recreates them on every run.
"""

import argparse
import json
import os
import shutil
import statistics
import subprocess
import sys
import tempfile

SERVER = "postgresql://temper:temper@localhost:5437"
BASE_DB = "temper_pw_cost_base"
HEAD_DB = "temper_pw_cost_head"
# Phase 0's last migration; everything after it is this work.
BASE_LAST = 20261119000020

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def run(cmd, **kw):
    p = subprocess.run(cmd, capture_output=True, text=True, **kw)
    if p.returncode != 0:
        sys.exit(f"{' '.join(cmd[:4])}… failed:\n{p.stderr.strip()[:2000]}")
    return p.stdout


def psql(dsn, sql):
    return run(["psql", dsn, "-X", "-q", "-t", "-A", "-v", "ON_ERROR_STOP=1", "-c", sql])


def fresh(db, source):
    dsn = f"{SERVER}/{db}"
    psql(f"{SERVER}/postgres", f"DROP DATABASE IF EXISTS {db} WITH (FORCE)")
    psql(f"{SERVER}/postgres", f"CREATE DATABASE {db} TEMPLATE template0")
    run(["sqlx", "migrate", "run", "--source", source, "--database-url", dsn])
    return dsn


def base_migrations():
    tmp = tempfile.mkdtemp(prefix="pw-cost-")
    src = os.path.join(REPO, "migrations")
    for f in os.listdir(src):
        if f.endswith(".sql") and int(f.split("_", 1)[0]) <= BASE_LAST:
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

DOORS = {
    "list_visible": LIST_VISIBLE,
    "resources_visible_to": "SELECT count(*) FROM resources_visible_to($1)",
    "edges_visible_to": "SELECT count(*) FROM edges_visible_to($1)",
    "search_wide": "SELECT count(*) FROM search_wide($1, array_fill(0.01::real, ARRAY[768])::vector, "
    "50, NULL, NULL, NULL, 20, 0)",
    "audit_drift_sweep": "SELECT count(*) FROM audit_drift_sweep($1, 50)",
    "graph_atlas_nodes_visible": "SELECT count(*) FROM graph_atlas_nodes_visible($1, "
    "ARRAY(SELECT resource_id FROM kb_resource_homes ORDER BY resource_id LIMIT 200))",
}


def time_once(dsn, sql, profile):
    stmt = sql.replace("$1", f"'{profile}'::uuid")
    out = psql(dsn, f"EXPLAIN (ANALYZE, FORMAT JSON) {stmt}")
    return json.loads(out[out.index("[") : out.rindex("]") + 1])[0]["Execution Time"]


def measure(dsn, profiles, samples):
    out = {}
    for name, sql in DOORS.items():
        times = []
        for p in profiles:
            time_once(dsn, sql, p)  # warm
            times += [time_once(dsn, sql, p) for _ in range(samples)]
        out[name] = statistics.median(times)
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--samples", type=int, default=9)
    ap.add_argument("--profiles", type=int, default=5)
    args = ap.parse_args()

    base_src = base_migrations()
    base = fresh(BASE_DB, base_src)
    head = fresh(HEAD_DB, os.path.join(REPO, "migrations"))
    for dsn in (base, head):
        psql(dsn, WORLD)
    handles = ", ".join(f"'p{k}'" for k in range(1, 300, 300 // args.profiles))
    profiles = psql(head, f"SELECT id FROM kb_profiles WHERE handle IN ({handles})").split()
    base_profiles = psql(base, f"SELECT id FROM kb_profiles WHERE handle IN ({handles})").split()

    results = {"base": measure(base, base_profiles, args.samples)}
    psql(head, ROWS)
    psql(head, "UPDATE kb_system_settings SET public_within_enabled = false WHERE id = 1")
    results["off"] = measure(head, profiles, args.samples)
    psql(head, "DELETE FROM kb_team_public_within")
    psql(head, "UPDATE kb_system_settings SET public_within_enabled = true WHERE id = 1")
    results["on-0"] = measure(head, profiles, args.samples)
    psql(head, ROWS)
    results["on-51"] = measure(head, profiles, args.samples)

    reads = psql(
        head,
        "SELECT count(*) FROM resources_visible_to((SELECT id FROM kb_profiles WHERE handle='p1'))",
    ).strip()
    member = psql(
        head,
        "SELECT count(*) FROM resources_visible_by_membership("
        "(SELECT id FROM kb_profiles WHERE handle='p1'))",
    ).strip()

    cases = ["base", "off", "on-0", "on-51"]
    print(f"median ms, {args.profiles} profiles x {args.samples} samples (p1 reads {reads}, "
          f"{member} by membership, in on-51)\n")
    print(f"| door | {' | '.join(cases)} | off/base | on-51/base |")
    print(f"|---|{'---|' * len(cases)}---|---|")
    for door in DOORS:
        row = [results[c][door] for c in cases]
        print(f"| `{door}` | " + " | ".join(f"{v:.2f}" for v in row)
              + f" | {row[1] / row[0]:.2f} | {row[3] / row[0]:.2f} |")
    shutil.rmtree(base_src, ignore_errors=True)


if __name__ == "__main__":
    main()
