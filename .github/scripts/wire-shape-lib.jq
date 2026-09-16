# wire-shape-lib.jq — the openapi shape comparator's one definition home.
#
# Since 2026-09-16 the comparator is a defs-only jq library so that more than one
# program can `include "wire-shape-lib"` it: wire-shape.jq (the verdict, called by
# wire-class-crosscheck.sh and check-openapi-pin.sh) and wire-pin-movements.jq (the
# per-movement naming the pin gate fails with). jq's `include` refuses a file that
# carries a main expression, so the main expressions live in their callers and the
# definitions live here — the #874 lesson generalized: the derivation is where shape
# honesty is computed, so it must have exactly one home.
#
# Inputs (invocation bindings, from --slurpfile): $base and $head, each the caller's
# `jq -S 'del(.info.version)'`-stripped contract. Outputs: verdict → "moved" or "grew".
#
# The class table's split (shared-semver-policy spec §4): additive = "shapes only
# grow; tolerant in both skew directions"; shape-breaking = "a rename, a removal, a
# type change, an envelope change". A surviving node may GAIN optional properties and
# may edit PROSE (description/title/summary/example/examples) — anything else on a
# surviving node is breaking. Fail closed: anything undecidable computes as moved.
#
# The array arm (added 2026-09-10, the #874 lesson): utoipa emits flattened-doc
# schemas as `{allOf: [ … ], description}`, so a prose edit can sit INSIDE an array —
# and a comparator that leaf-compared arrays whole read that whitelisted prose as a
# break. Equal-length object arrays now recurse element-wise (the whitelist applies
# inside); a length change or a non-object element diff stays moved, so compositor
# growth (an added allOf member is an intersection constraint — a tightening) and
# enum edits keep their fail-closed verdict.
#
# The required arm (CORRECTED 2026-09-16, the #906 lesson): this arm originally
# treated required GROWTH as breaking and required SHRINKAGE as "the tolerant
# direction". That was wrong for the omit-class. A required field that leaves
# `required` while staying in `properties` is exactly a field the server may now
# OMIT (Option + skip_serializing_if) — and a client built against the old contract
# types it as required, so its parse breaks on the first absent one. PR #906's
# `Scoring.score` is the founding case; the old reading computed that diff "grew"
# and would have let a pinned-contract gate certify it green. Under §4's own
# definition ("tolerant in both skew directions") ANY change to `required` is
# breaking: growth strands new-required-reading clients against old servers,
# shrinkage strands old-required-reading clients against new servers. The arm is
# therefore symmetric: required arrays that differ ⇒ moved. The arm also covers the
# gain-from-absent face (a base node with NO `required` key that gains one in head —
# the independent review of the pin-gate commit caught the base-keys-only loop
# missing it): required-growth is checked before the key loop, on the head side.
# Note `required` is compared as an array, so a pure REORDER computes moved —
# deliberate fail-closed (the spec: anything undecidable computes as moved); utoipa's
# deterministic ordering makes that a non-event in practice.
def broke_node($b; $h):
  ($b | type) != ($h | type)
  or (
    ($b | type) == "array"
    and (
      (($b | length) != ($h | length))
      or ([ range(0; ($b | length)) as $i | broke_node($b[$i]; $h[$i]) ] | any)
    )
  )
  or (
    ($b | type) == "object"
    and (
      ([ $b | keys[] ] - [ $h | keys[] ] | length > 0)
      or (
        # Required-gain-from-absent: the per-key loop below iterates BASE keys, so a
        # `required` key that exists only in HEAD would never be visited — a node
        # growing its first required members would compute grew. Caught here instead:
        # a non-empty head-side `required` on a node that had none is the
        # new-required-reading break, not growth.
        (($h | has("required")) and (($b | has("required")) | not)
          and (($h.required | length) > 0))
      )
      or (
        [ $b | keys[] as $k
          | if (($b[$k] | type) == "object") and (($h[$k] | type) == "object")
            then broke_node($b[$k]; $h[$k])
            elif $k == "required" and (($b[$k] | type) == "array")
            then ($h[$k] != $b[$k])
            elif $k == "description" or $k == "title" or $k == "summary"
              or $k == "example" or $k == "examples"
            then false
            elif (($b[$k] | type) == "array") and (($h[$k] | type) == "array")
            then broke_node($b[$k]; $h[$k])
            else $b[$k] != $h[$k]
            end ]
        | any
      )
      or (
        (($b | has("properties")) and ($h | has("properties")))
        and (
          [ ([ $h.properties | keys[] ] - [ $b.properties | keys[] ])[]
              as $p
              | select((($h.required // []) | index($p)) != null) ]
          | length > 0
        )
      )
    )
  )
  or (($b | type) != "object" and ($b | type) != "array" and $b != $h);

def verdict:
  [ ($base[0].paths | keys[]) as $p
    | if ($head[0].paths | has($p)) | not then "moved"
      elif broke_node($base[0].paths[$p]; $head[0].paths[$p]) then "moved"
      else empty end ]
  + [ (($base[0].components // {}) | keys[]) as $c
      | if (($head[0].components // {}) | has($c) | not) then "moved" else empty end ]
  + [ (($base[0].components.schemas // {}) | keys[]) as $s
      | if (($head[0].components.schemas // {}) | has($s)) | not then "moved"
        elif broke_node($base[0].components.schemas[$s]; $head[0].components.schemas[$s]) then "moved"
        else empty end ]
  | if length > 0 then "moved" else "grew" end;
