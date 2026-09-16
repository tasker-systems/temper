# wire-pin-movements.jq — the pin gate's per-movement naming (pinned wire contracts, 2026-09-16).
#
# The pin gate's verdict comes from the shared comparator (wire-shape-lib.jq via
# wire-shape.jq — ONE definition); this program only NAMES what moved, reusing the
# same broke_node so the naming can never disagree with the verdict. A "changed"
# line here is exactly a node whose broke_node is true.
#
# Inputs: --slurpfile base and --slurpfile head, each the caller's
# `jq -S 'del(.info.version)'`-stripped contract. Output: one JSON array of strings.
#
# Line grammar: "<kind> <relation>: <name>" where kind ∈ path|schema and
# relation ∈ removed|changed|born. removed/changed are the breaking set;
# born is tolerated growth, reported so a fail shows the whole movement, not just
# the red half.
include "wire-shape-lib";

[ ($base[0].paths | keys[]) as $p
  | if ($head[0].paths | has($p)) | not then "path removed: \($p)"
    elif broke_node($base[0].paths[$p]; $head[0].paths[$p]) then "path changed: \($p)"
    else empty end ]
+ [ ($head[0].paths | keys[]) as $p
    | if ($base[0].paths | has($p)) | not then "path born: \($p)" else empty end ]
+ [ (($base[0].components.schemas // {}) | keys[]) as $s
    | if (($head[0].components.schemas // {}) | has($s)) | not then "schema removed: \($s)"
      elif broke_node($base[0].components.schemas[$s]; $head[0].components.schemas[$s]) then "schema changed: \($s)"
      else empty end ]
+ [ (($head[0].components.schemas // {}) | keys[]) as $s
    | if (($base[0].components.schemas // {}) | has($s)) | not then "schema born: \($s)" else empty end ]
