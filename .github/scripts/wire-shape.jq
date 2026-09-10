# wire-shape.jq — the openapi shape comparator (shared-semver-policy spec §4, D-S4).
#
# One definition, two callers: wire-class-crosscheck.sh derives its shape verdict with this
# program, and test-wire-class-crosscheck.sh probes it directly against fixture specs — the
# derivation is where shape honesty is computed, so it is the part that must be pinned, not
# only the verdict handling around it.
#
# Inputs: --slurpfile base (the base contract) and --slurpfile head (the working tree's),
# each jq -S 'del(.info.version)'-stripped by the caller. Output: "moved" or "grew".
#
# The class table's split (spec §4): additive = "shapes only grow; tolerant in both skew
# directions"; shape-breaking = "a rename, a removal, a type change, an envelope change".
# A surviving node may therefore GAIN optional properties and may edit PROSE
# (description/title/summary/example/examples) — anything else on a surviving node is
# breaking. Fail closed: anything undecidable computes as moved.
#
# The array arm (added 2026-09-10, the #874 lesson): utoipa emits flattened-doc schemas as
# `{allOf: [ … ], description}`, so a prose edit can sit INSIDE an array — and a comparator
# that leaf-compared arrays whole read that whitelisted prose as a break. Equal-length
# object arrays now recurse element-wise (the whitelist applies inside); a length change or
# a non-object element diff stays moved, so compositor growth (an added allOf member is an
# intersection constraint — a tightening) and enum edits keep their fail-closed verdict.
# `required` keeps its own arm BEFORE the array arm: growth is breaking, shrinkage is the
# tolerant direction — a semantic the generic array comparison must not overwrite.
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
        [ $b | keys[] as $k
          | if (($b[$k] | type) == "object") and (($h[$k] | type) == "object")
            then broke_node($b[$k]; $h[$k])
            elif $k == "required" and (($b[$k] | type) == "array")
            then (($h[$k] - $b[$k]) | length > 0)
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
| if length > 0 then "moved" else "grew" end
