# wire-shape-lib.jq — the openapi shape comparator's one definition home.
#
# Since 2026-09-16 the comparator is a defs-only jq library so that more than one
# program can `include "wire-shape-lib"` it: wire-shape.jq (the verdict, called by
# wire-class-crosscheck.sh and check-openapi-pin.sh) and wire-pin-movements.jq (the pin
# gate's per-movement naming). jq's `include` refuses a file that carries a main
# expression, so the main expressions live in their callers and the definitions live
# here.
#
# Inputs (invocation bindings, from --slurpfile): $base and $head, each the caller's
# `jq -S 'del(.info.version)'`-stripped contract. Outputs: verdict → "moved" or "grew".
# Callers MUST pass -L pointing at this directory: jq does not resolve `include`
# relative to the program file's own directory (measured, jq 1.7.1).
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
#
# The side-aware tolerance arm (added 2026-09-23, beat G3a's B1 — pulled forward from
# the compat amendment's named comparator follow-on by Pete's ruling, scoped to the
# two provably-tolerant classes; the constraint-vocabulary residue itself stays open):
# the array arm above fails closed on ANY length change, which classified two
# genuinely tolerant movements as breaks:
#
# 1. A BORN OPTIONAL PARAMETER on a surviving operation. Old clients never send it;
#    a new client that sends it against an old server is answered by that server
#    ignoring the unknown query parameter and serving the incumbent shape. Tolerant
#    in both skew directions ⇒ additive by §4's own definition. A born REQUIRED
#    parameter stays moved: old clients never send it, and the new server refuses
#    requests without it.
# 2. STRING-ENUM MEMBER GROWTH in INPUT position — and "growth" means a strict
#    SET-superset: exact membership (`unique` + `index`), never jq `contains`,
#    which on string arrays is SUBSTRING-containment and would certify a rename
#    (`pending` → `pending_recheck`) as growth (found in review, the B1 beat's
#    reviewer, D1). The two transport faces differ and both are honest: at a query
#    parameter, an old server silently ignores the unknown value; in a request
#    body, an old server refuses it with a clean 400 naming the vocabulary — a
#    visible capability boundary, never silent corruption — and old clients (which
#    never send the new member) are untouched either way. What is NOT tolerated is
#    the OUTPUT direction: a server that may now EMIT a new member can strand an
#    old client's exhaustive parse — exactly the "stricter than an older server's
#    emissions" skew the compat amendment forbids. "Input position" is proven, not
#    assumed: an operation's `parameters`/`requestBody` subtrees are input by
#    construction, and a component schema qualifies only while it is input-only in
#    BOTH contracts (see `tolerant_input_names` — reachability computed against the
#    base alone would let a schema that gains a response ref in the same diff keep
#    its tolerance while its enum values gain an output channel; found in review,
#    D2). Anything ambiguous — orphaned schemas, dual reachability on either end —
#    stays "mixed" and fails closed.
#
# THIS SLIDE DOES NOT TOUCH the amendment's named residue: a head-GAINED constraint
# key (`enum`/`minLength`/`maximum`/`pattern`/`format`/`default`/`nullable` on an
# existing property) still passes green today exactly as the amendment records, and
# classifying THAT remains the named follow-on task's work. No overlap: the residue
# is about permissiveness the comparator cannot see; this arm is about a tolerance
# the comparator could not express.

# Refs to component schemas found anywhere under $x.
def schema_refs($x):
  [ $x | .. | objects | (."$ref" // empty) ]
  | map(select(startswith("#/components/schemas/")) | ltrimstr("#/components/schemas/"))
  | unique;

# The transitive closure of component-schema reachability from $seeds ($schemas is
# the components.schemas map the names index into).
def schema_closure($seeds; $schemas):
  def step($s): ($s + [ $s[] as $name | ($schemas[$name] | schema_refs(.))[] ] | unique);
  def fixed($s): (step($s)) as $n | if (($n | length) == ($s | length)) then $s else fixed($n) end;
  fixed($seeds);

# Component schemas referenced from INPUT positions: parameter entries (a `name`+`in`
# object carrying a `schema`) and `requestBody` subtrees, anywhere under .paths —
# path-item-level parameters included by the same walk.
def input_refs($doc):
  [ $doc.paths
    | .. | objects
    | select(((has("in") and has("schema")) or has("requestBody")))
    | if has("requestBody") then ."requestBody" else .schema end ]
  | schema_refs(.);

# Component schemas referenced from RESPONSE positions (the `responses` subtree of
# any operation).
def response_refs($doc):
  [ $doc.paths
    | .. | objects
    | select(has("responses"))
    | .responses ]
  | schema_refs(.);

# Input-only schemas: referenced from input positions and from no response position.
# Referenced from both sides → excluded (fail-closed: an enum that grows under a
# schema a response can carry is treated as output). Unreferenced → excluded the
# same way. Computed per contract by both callers (verdict, and the pin gate's
# movement naming) so the naming can never disagree with the verdict.
def input_only_names($doc):
  ($doc.components.schemas // {}) as $schemas
  | schema_closure(input_refs($doc); $schemas) as $in
  | schema_closure(response_refs($doc); $schemas) as $out
  | [ $in[] | select((. | IN($out[])) | not) ];

# The schemas the input-side tolerance may consider: input-only in BOTH contracts.
# Computed against the BASE alone, a schema that BECOMES response-reachable in the
# same diff would keep its input tolerance while its enum values gain an output
# channel — the exact "stricter than an older server's emissions" direction the
# compat amendment forbids, found in review (the B1 beat's reviewer, D2). The
# intersection is the honest proof: the side must be unambiguous on both ends of
# the movement being judged.
def tolerant_input_names:
  input_only_names($base[0]) as $in_b
  | input_only_names($head[0]) as $in_h
  | [ $in_b[] | select(. | IN($in_h[])) ];

# The side a descent into key $k inherits: parameters and request bodies are input;
# responses are output; everything else keeps the side it arrived with.
def child_side($k; $side):
  if $k == "parameters" or $k == "requestBody" then "input"
  elif $k == "responses" then "output"
  else $side
  end;

# A parameter's identity: the same (`in`, `name`) on both sides is the same
# parameter; anything else is a different one.
def param_key: "\(.in)|\(.name)";

def broke_node($b; $h; $side):
  if ($b | type) != ($h | type) then true
  elif ($b | type) == "array" then
    # The side-aware tolerance: input-side string-enum member GROWTH — a strict
    # SET-superset (exact membership, never `contains`' substring semantics) — is
    # the one tolerant length change. Everything else — a shrink, a same-length
    # permutation, a born enum where none was, any non-string element diff — falls
    # through to the fail-closed generic arm. The arm is an EARLY FALSE for the
    # tolerant case only; it must never decide breaking, or it would reclassify
    # identical and equal-length arrays as breaks.
    if (($side == "input")
        and (($b | all(type == "string"))) and (($h | all(type == "string")))
        and ( ($h | unique) as $hu
              | ($b | unique) as $bu
              | (($hu | length) > ($bu | length))
              and ($bu | all(. as $m | ($hu | index($m)) != null)) )) then false
    else
      (($b | length) != ($h | length))
      or ([ range(0; ($b | length)) as $i | broke_node($b[$i]; $h[$i]; $side) ] | any)
    end
  elif ($b | type) == "object" then
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
      # A surviving node that GAINS a `parameters` key whose entries include a
      # required parameter: old clients never send what the new server refuses.
      # (Born-optional parameters on the same gain are the tolerant class.)
      (($h | has("parameters")) and (($b | has("parameters")) | not)
        and ([ $h.parameters[]
               | select((type == "object") and ((.required // false) == true)) ]
             | length > 0))
    )
    or (
      [ $b | keys[] as $k
        | if $k == "parameters"
          then
            # The parameter array, element-wise by identity (`in` + `name`):
            #   removed ⇒ moved (an old client's request loses its parameter),
            #   matched pairs recurse under the input side (schema tightening,
            #   required flips),
            #   born ⇒ moved only when `required` (old clients never send what
            #   the new server refuses), tolerated growth otherwise.
            # jq has no forward references, so this recursion into broke_node
            # lives inline in broke_node's own body rather than in a sibling def.
            ( ($b[$k] | map({key: param_key, value: .}) | from_entries) as $bm
              | ($h[$k] | map({key: param_key, value: .}) | from_entries) as $hm
              | ([ ($bm | keys[]) | select((. | IN($hm | keys[])) | not) ] | length > 0)
              or ([ ($bm | keys[]) as $kp
                    | select($hm | has($kp))
                    | broke_node($bm[$kp]; $hm[$kp]; "input") ]
                  | any)
              or ([ ($hm | keys[]) as $kp
                    | select(($bm | has($kp)) | not)
                    | (($hm[$kp].required // false) == true) ]
                  | any) )
          elif (($b[$k] | type) == "object") and (($h[$k] | type) == "object")
          then broke_node($b[$k]; $h[$k]; child_side($k; $side))
          elif $k == "required" and (($b[$k] | type) == "array")
          then ($h[$k] != $b[$k])
          elif $k == "description" or $k == "title" or $k == "summary"
            or $k == "example" or $k == "examples"
          then false
          elif (($b[$k] | type) == "array") and (($h[$k] | type) == "array")
          then broke_node($b[$k]; $h[$k]; child_side($k; $side))
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
  else $b != $h
  end;

def verdict:
  tolerant_input_names as $in_only
  | [ ($base[0].paths | keys[]) as $p
      | if ($head[0].paths | has($p)) | not then "moved"
        elif broke_node($base[0].paths[$p]; $head[0].paths[$p]; "mixed") then "moved"
        else empty end ]
  + [ (($base[0].components // {}) | keys[]) as $c
      | if (($head[0].components // {}) | has($c) | not) then "moved" else empty end ]
  + [ (($base[0].components.schemas // {}) | keys[]) as $s
      | if (($head[0].components.schemas // {}) | has($s)) | not then "moved"
        elif broke_node($base[0].components.schemas[$s]; $head[0].components.schemas[$s];
                        (if ($in_only | index($s)) != null then "input" else "mixed" end)) then "moved"
        else empty end ]
  | if length > 0 then "moved" else "grew" end;
