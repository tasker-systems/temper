#!/usr/bin/env python3
"""What share of `resources_visible_to` cost is the recursive team closure?

Run this when the question "is the team closure worth removing yet?" comes back. The
answer moves with team topology, so it is a measurement with a shelf life, not a fact.

    python3 scripts/visibility-cost/closure_share.py corpus
    python3 scripts/visibility-cost/closure_share.py sweep
    python3 scripts/visibility-cost/closure_share.py shapes
    python3 scripts/visibility-cost/closure_share.py plausible

`corpus` is READ-ONLY and safe against any database you can reach. The other three BUILD
SYNTHETIC TEAM TOPOLOGY and must only ever be pointed at a throwaway database — they take
`--dsn`, and they refuse to run against a DSN whose database name is not clearly scratch.

For a real deployment there is a different instrument with different constraints (read-only,
no content leaves, bounded runtime): task 019fddc6. This is not that; do not point it at
production.

## Why it decomposes the way it does

`resources_visible_to(p)` opens with

    WITH reachable_teams AS MATERIALIZED (SELECT team_id FROM profile_reachable_teams(p))

evaluated once, feeding four of its six arms. That CTE **is** the closure. The SQL function
INLINES into the calling query, so the CTE appears as its own plan node with its own actual
time — which means the closure and the whole gate can be read from ONE execution rather than
compared across two statements. That matters: the first version of this measurement compared
a CTE node against a standalone statement's total execution time, and the comparison was
meaningless because the two carry different overhead.

## The cost model this found

`profile_reachable_teams` is

    SELECT DISTINCT a.team_id
    FROM profile_effective_teams(p) e
    CROSS JOIN LATERAL team_ancestors(e.team_id) a

so the recursive walk runs once **per membership** and the DISTINCT collapses only the
OUTPUT. Cost therefore tracks **memberships x depth** (walk-steps) and the size of the team
tables — NOT reachable-team cardinality, which is off by up to 7.5x as a predictor.

## Caveats that belong to every number this prints

- **Custom plans.** `EXPLAIN` on an ad-hoc statement plans a custom plan; production prepares
  its statements and runs generic ones. `--generic` forces the generic plan for `corpus`.
- **Absolute milliseconds are not portable** across hosts or Neon compute sizes. The ratios
  within one run are.
- **Synthetic topology is not a tenant.** `sweep`/`shapes`/`plausible` bracket the question.
  They do not report anyone's deployment.
"""
import argparse
import json
import statistics
import subprocess
import sys

DEV = "postgresql://temper:temper@localhost:5437/temper_development"
SCRATCH = "postgresql://temper:temper@localhost:5437/temper_closure_probe"
PROBE = "019fe250-0000-7000-8000-00000000ffff"
CORPUS_ROWS = 3000


def psql(sql, dsn):
    p = subprocess.run(["psql", dsn, "-X", "-q", "-t", "-A", "-c", sql],
                       capture_output=True, text=True)
    if p.returncode != 0:
        raise RuntimeError(f"{p.stderr.strip()}\n--- sql ---\n{sql[:400]}")
    return p.stdout


def require_scratch(dsn):
    """A synthetic-topology run must not touch a database anyone cares about."""
    name = dsn.rsplit("/", 1)[-1].split("?")[0]
    if "probe" not in name and "scratch" not in name and "tmp" not in name:
        sys.exit(
            f"refusing to build synthetic teams in {name!r}: this subcommand WRITES.\n"
            "Point it at a throwaway database, e.g.\n"
            "  createdb -T temper_development temper_closure_probe"
        )


def find_cte(node, name):
    if node.get("Subplan Name") == f"CTE {name}":
        return node
    for c in node.get("Plans", []):
        got = find_cte(c, name)
        if got:
            return got
    return None


def sample(profile, dsn, generic=False):
    """One (closure_ms, gate_ms) pair, decomposed within a single execution."""
    stmt = f"SELECT count(*) FROM resources_visible_to('{profile}')"
    if generic:
        out = psql(
            "BEGIN; SET LOCAL plan_cache_mode = force_generic_plan; "
            f"PREPARE g(uuid) AS SELECT count(*) FROM resources_visible_to($1); "
            f"EXPLAIN (ANALYZE, TIMING ON, FORMAT JSON) EXECUTE g('{profile}'); ROLLBACK;",
            dsn)
    else:
        out = psql(f"EXPLAIN (ANALYZE, TIMING ON, FORMAT JSON) {stmt};", dsn)
    plan = json.loads(out[out.index("["): out.rindex("]") + 1])[0]
    cte = find_cte(plan["Plan"], "reachable_teams")
    if cte is None:
        raise RuntimeError(
            "no `CTE reachable_teams` node. Either the gate no longer materializes the "
            "closure, or the function stopped inlining -- either way this instrument's "
            "decomposition is invalid and must be rewritten, not reinterpreted.")
    return cte["Actual Total Time"], plan["Execution Time"]


def measure(profile, dsn, n=11, generic=False):
    """Median of n after 3 warm-ups. A single sample is not a measurement."""
    for _ in range(3):
        sample(profile, dsn, generic)
    pairs = [sample(profile, dsn, generic) for _ in range(n)]
    return (statistics.median(c for c, _ in pairs),
            statistics.median(g for _, g in pairs),
            statistics.median(100.0 * c / g for c, g in pairs))


# ── read-only: this deployment as it stands ─────────────────────────────────────────────

def cmd_corpus(args):
    dsn = args.dsn or DEV
    profiles = [l for l in psql("SELECT id FROM kb_profiles ORDER BY id;", dsn).split() if l]
    meta = {}
    for line in psql("""SELECT p.id,
            (SELECT count(*) FROM kb_team_members m WHERE m.profile_id=p.id),
            (SELECT count(*) FROM profile_reachable_teams(p.id)),
            (SELECT count(*) FROM resources_visible_to(p.id))
        FROM kb_profiles p ORDER BY p.id;""", dsn).strip().splitlines():
        pid, mem, reach, vis = line.split("|")
        meta[pid] = (int(mem), int(reach), int(vis))

    print(f"plan: {'GENERIC (as production runs)' if args.generic else 'CUSTOM (ad-hoc EXPLAIN)'}")
    print(f"{'ord':>3} {'memb':>5} {'reach':>6} {'visible':>8} "
          f"{'closure_ms':>11} {'gate_ms':>9} {'share%':>7}")
    for i, p in enumerate(profiles, 1):
        c, g, s = measure(p, dsn, generic=args.generic)
        mem, reach, vis = meta[p]
        print(f"{i:>3} {mem:>5} {reach:>6} {vis:>8} {c:>11.3f} {g:>9.3f} {s:>6.1f}%")


# ── synthetic topology ──────────────────────────────────────────────────────────────────

def reset(dsn):
    psql(f"""
        DELETE FROM kb_team_members WHERE profile_id = '{PROBE}';
        DELETE FROM kb_access_grants WHERE principal_id IN
            (SELECT id FROM kb_teams WHERE slug LIKE 'sweep-%');
        DELETE FROM kb_teams_parents WHERE child_id IN
            (SELECT id FROM kb_teams WHERE slug LIKE 'sweep-%')
           OR parent_id IN (SELECT id FROM kb_teams WHERE slug LIKE 'sweep-%');
        DELETE FROM kb_teams WHERE slug LIKE 'sweep-%';
    """, dsn)


def ensure_probe(dsn):
    psql(f"""INSERT INTO kb_profiles (id, handle, display_name)
             VALUES ('{PROBE}', 'sweep-probe', 'sweep probe')
             ON CONFLICT (id) DO NOTHING;""", dsn)


def grant_corpus(dsn, leaf_slug, rows):
    if not rows:
        return
    psql(f"""
        INSERT INTO kb_access_grants
            (subject_table, subject_id, principal_table, principal_id, can_read,
             granted_by_profile_id)
        SELECT 'kb_resources', r.id, 'kb_teams',
               (SELECT id FROM kb_teams WHERE slug = '{leaf_slug}'), true,
               (SELECT id FROM kb_profiles WHERE handle = 'system')
        FROM (SELECT id FROM kb_resources WHERE is_active ORDER BY id LIMIT {rows}) r
        ON CONFLICT DO NOTHING;
    """, dsn)


def analyze(dsn):
    psql("ANALYZE kb_teams; ANALYZE kb_team_members; ANALYZE kb_teams_parents; "
         "ANALYZE kb_access_grants;", dsn)


def build_disjoint(dsn, memb, walk, corpus=0):
    """`memb` independent chains, each `walk` tall. Reach = memb * walk. Worst case: the
    LATERAL shares nothing between memberships."""
    reset(dsn); ensure_probe(dsn)
    psql(f"""
        WITH spec AS (
            SELECT c, l FROM generate_series(1,{memb}) c, generate_series(1,{walk}) l
        ), made AS (
            INSERT INTO kb_teams (slug, name)
            SELECT format('sweep-%s-%s', c, l), format('t %s %s', c, l) FROM spec
            RETURNING id, slug
        ), parsed AS (
            SELECT id, split_part(slug,'-',2)::int c, split_part(slug,'-',3)::int l FROM made
        ), e AS (
            INSERT INTO kb_teams_parents (child_id, parent_id)
            SELECT ch.id, pa.id FROM parsed ch JOIN parsed pa ON pa.c=ch.c AND pa.l=ch.l-1
            RETURNING 1
        )
        INSERT INTO kb_team_members (team_id, profile_id, role)
        SELECT id, '{PROBE}', 'member' FROM parsed WHERE l = {walk};
    """, dsn)
    grant_corpus(dsn, f"sweep-1-{walk}", corpus)
    analyze(dsn)


def build_shared(dsn, memb, walk, org=None, corpus=0):
    """A spine `walk-1` tall with `org` leaves on its bottom; the probe joins `memb` of them.

    Each membership still walks `walk` levels, so walk-steps match the disjoint build at the
    same (memb, walk) -- only the SHARING of ancestors differs. That is what isolates
    cardinality from cost.
    """
    org = org or memb
    reset(dsn); ensure_probe(dsn)
    spine = walk - 1
    psql(f"""
        WITH spine AS (
            INSERT INTO kb_teams (slug, name)
            SELECT format('sweep-spine-%s', l), format('s %s', l)
            FROM generate_series(1,{spine}) l
            RETURNING id, split_part(slug,'-',3)::int AS l
        ), se AS (
            INSERT INTO kb_teams_parents (child_id, parent_id)
            SELECT ch.id, pa.id FROM spine ch JOIN spine pa ON pa.l = ch.l-1
            RETURNING 1
        ), leaves AS (
            INSERT INTO kb_teams (slug, name)
            SELECT format('sweep-leaf-%s', c), format('l %s', c)
            FROM generate_series(1,{org}) c
            RETURNING id, split_part(slug,'-',3)::int AS c
        ), le AS (
            INSERT INTO kb_teams_parents (child_id, parent_id)
            SELECT l.id, (SELECT id FROM spine WHERE l={spine}) FROM leaves l
            RETURNING 1
        )
        INSERT INTO kb_team_members (team_id, profile_id, role)
        SELECT id, '{PROBE}', 'member' FROM leaves WHERE c <= {memb};
    """, dsn)
    grant_corpus(dsn, "sweep-leaf-1", corpus)
    analyze(dsn)


def reach_and_visible(dsn):
    return (int(psql(f"SELECT count(*) FROM profile_reachable_teams('{PROBE}');", dsn)),
            int(psql(f"SELECT count(*) FROM resources_visible_to('{PROBE}');", dsn)))


def cmd_sweep(args):
    """How the share scales, at two corpus sizes. Brackets the question."""
    dsn = args.dsn or SCRATCH
    require_scratch(dsn)
    print(f"{'memb':>5} {'depth':>6} {'reach':>6} {'visible':>8} "
          f"{'closure_ms':>11} {'gate_ms':>9} {'share%':>7}")
    for corpus in (0, CORPUS_ROWS):
        for memb, depth in [(1, 1), (1, 4), (2, 2), (4, 4), (8, 4),
                            (16, 4), (16, 8), (32, 8), (64, 16)]:
            build_disjoint(dsn, memb, depth, corpus)
            reach, vis = reach_and_visible(dsn)
            c, g, s = measure(PROBE, dsn)
            print(f"{memb:>5} {depth:>6} {reach:>6} {vis:>8} "
                  f"{c:>11.3f} {g:>9.3f} {s:>6.1f}%")
        print()
    reset(dsn)


def cmd_shapes(args):
    """Disjoint vs shared at equal walk depth: isolates cardinality from cost."""
    dsn = args.dsn or SCRATCH
    require_scratch(dsn)
    print(f"{'memb':>5} {'walk':>5} | {'disjoint reach':>14} {'ms':>8} "
          f"| {'shared reach':>12} {'ms':>8} | {'ratio':>7}")
    for memb, walk in [(8, 4), (16, 4), (32, 8), (64, 16)]:
        build_disjoint(dsn, memb, walk)
        dr, _ = reach_and_visible(dsn)
        dms, _, _ = measure(PROBE, dsn)
        build_shared(dsn, memb, walk)
        sr, _ = reach_and_visible(dsn)
        sms, _, _ = measure(PROBE, dsn)
        print(f"{memb:>5} {walk:>5} | {dr:>14} {dms:>8.3f} "
              f"| {sr:>12} {sms:>8.3f} | {sms/dms:>6.2f}x")
    reset(dsn)


def cmd_plausible(args):
    """The decision band: shapes a real tenant might actually have."""
    dsn = args.dsn or SCRATCH
    require_scratch(dsn)
    print(f"shared ancestry, {CORPUS_ROWS}-row visible corpus\n")
    print(f"{'memb':>5} {'depth':>6} {'org':>5} {'reach':>6} {'visible':>8} "
          f"{'closure_ms':>11} {'gate_ms':>9} {'share%':>7}")
    for memb, depth, org in [(2, 3, 20), (4, 3, 20), (4, 4, 50), (8, 4, 50),
                             (8, 5, 100), (12, 5, 100), (16, 6, 200), (16, 4, 500)]:
        build_shared(dsn, memb, depth, org, CORPUS_ROWS)
        reach, vis = reach_and_visible(dsn)
        c, g, s = measure(PROBE, dsn)
        print(f"{memb:>5} {depth:>6} {org:>5} {reach:>6} {vis:>8} "
              f"{c:>11.3f} {g:>9.3f} {s:>6.1f}%")
    reset(dsn)


def main():
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("command", choices=["corpus", "sweep", "shapes", "plausible"])
    ap.add_argument("--dsn")
    ap.add_argument("--generic", action="store_true",
                    help="force the generic plan (what production runs). `corpus` only.")
    args = ap.parse_args()
    {"corpus": cmd_corpus, "sweep": cmd_sweep,
     "shapes": cmd_shapes, "plausible": cmd_plausible}[args.command](args)


if __name__ == "__main__":
    main()
