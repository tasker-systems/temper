#!/usr/bin/env bash
# Capture both search arms' plans as PLAN STRUCTURE ONLY, so two captures can be compared.
#
# Committed rather than throwaway because its whole purpose is to be run TWICE — once before a
# refactor of the arms' interiority and once after — and a comparison is only as good as the
# guarantee that both sides ran the identical statement against the identical corpus. A pair of
# hand-typed psql sessions cannot give that guarantee; this can.
#
#   scripts/measure/capture-search-arm-plans.sh > before.txt
#   ... apply the migration ...
#   scripts/measure/capture-search-arm-plans.sh > after.txt
#   diff before.txt after.txt
#
# THE QUERY TEXT IS DROPPED, AND THAT IS THE POINT. A refactor of the arms' bodies changes their
# SQL by definition, so a diff that includes the source says only "you edited the thing you edited".
# The question this tool exists to answer is whether the PLAN moved — node types, join orders,
# index names, index conditions. Extracting `.Plan` from auto_explain's JSON output is what makes
# that separation exact rather than a regex approximation over indented text.
#
# An EMPTY diff is the deliverable. What a non-empty diff means is not uniform, and the distinction
# is the whole exercise:
#   * a lost `Index Only Scan`, or a new per-row `Filter:` naming a function, is a REGRESSION —
#     spec §2.1 measured exactly that failure and it is why the extraction is by view and not by
#     scalar function;
#   * a re-ordered but equivalent join on a small relation is noise.
# Read the diff; do not merely check that it is empty.
#
# Expects the measurement corpus (`cargo make seed-corpus 20`). On a near-empty corpus every arm
# seq-scans and the capture proves nothing — the plans agree because no index decision was made.
set -euo pipefail

DB="${DATABASE_URL:-postgresql://temper:temper@localhost:5437/temper_development}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

psql "$DB" -X -f "$HERE/capture-search-arm-plans.sql" 2>&1 | python3 -c '
import json, re, sys

# auto_explain emits, per statement: a "LOG:  ... plan:" line, then a JSON object over N lines.
# Everything else on stdout is psql chatter or one of the \echo shape banners, which we keep as
# section markers so a diff hunk lands somewhere nameable.
raw = sys.stdin.read().splitlines()

def strip_costs(node):
    """Cost/row/width are ESTIMATES that move with statistics, not plan structure. auto_explain has
    no log_costs knob (that is an EXPLAIN option), so they are removed here instead."""
    if isinstance(node, dict):
        return {k: strip_costs(v) for k, v in node.items()
                if k not in ("Startup Cost", "Total Cost", "Plan Rows", "Plan Width")}
    if isinstance(node, list):
        return [strip_costs(v) for v in node]
    # Generated uuids and the 768-float query vector are INPUTS that differ per seed, not structure.
    if isinstance(node, str):
        node = re.sub(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", "<uuid>", node)
        node = re.sub(r"\[[-0-9.,e ]{40,}\]", "<vector>", node)
    return node

buf, depth, inside = [], 0, False
for line in raw:
    if line.startswith("####"):
        print(line); continue
    if not inside:
        if line.strip() in ("{", "{}"):
            inside, buf, depth = True, [line], line.count("{") - line.count("}")
        continue
    buf.append(line)
    depth += line.count("{") - line.count("}")
    if depth <= 0:
        inside = False
        try:
            doc = json.loads("\n".join(buf))
        except json.JSONDecodeError:
            continue
        entries = doc if isinstance(doc, list) else [doc]
        for e in entries:
            if "Plan" in e:
                print(json.dumps(strip_costs(e["Plan"]), indent=2, sort_keys=True))
'
