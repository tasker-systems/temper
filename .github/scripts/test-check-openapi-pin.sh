#!/usr/bin/env bash
# .github/scripts/test-check-openapi-pin.sh
#
# Test harness for check-openapi-pin.sh — the pinned-contract gate (the committed
# openapi.json stays inside the current pinned minor; additive-only within a pin).
# Sibling to test-wire-class-crosscheck.sh, sharing its conventions and its base
# fixture.
#
# THE BITE THIS HARNESS MUST PROVE
# --------------------------------
# The gate's whole reason to exist is the movement class the declared-class gate
# could only signal. So the probes run the REAL comparator (no verdict seam): the
# required-shrink probe is the #906 class itself — a field leaving `required` while
# staying in `properties` is a field the server may now omit, and the comparator
# computed exactly that diff "grew" until its required arm was corrected the same
# day this gate was built. The probe pins the correction from the pin-gate side:
# that diff FAILS here, naming the schema. Selection and fail-closed faces are
# probed with the same seams the script documents.
#
#   bash .github/scripts/test-check-openapi-pin.sh

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CHECK="${SCRIPT_DIR}/check-openapi-pin.sh"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; shift; printf '    %s\n' "$@"; FAIL=$((FAIL + 1)); }

VERFILE="${WORK}/VERSION"
PINDIR="${WORK}/schemas/versions"
EMIT="${WORK}/openapi.json"

write_version() { printf '%s\n' "${1:-0.5.1}" > "$VERFILE"; }

# The base contract: one path (whose 200 carries a scored body, so the #906-class
# probe below MODIFIES a survivor rather than growing one), one allOf-shaped schema
# (the utoipa flattened-doc shape) — the same fixture test-wire-class-crosscheck.sh's
# derivation probes use, plus the scored response.
base_contract() {
  cat <<'JSON'
{"paths":{"/api/blobs/{id}":{"get":{"responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"type":"object","properties":{"score":{"type":"number","description":"the ordering quantity"}},"required":["score"]}}}}}}}},"components":{"schemas":{"Example":{"allOf":[{"type":"object","properties":{"peer_table":{"type":"string","description":"`kb_resources` | `kb_cogmaps` | `kb_blobs` — the peer endpoint's table."}},"required":["peer_table"]}],"description":"Assert a relation."}}}}
JSON
}

# A pin at <M.m>: the contract plus the provenance README the gate demands.
write_pin() { # $1 = M.m, $2 = contract JSON
  mkdir -p "${PINDIR}/${1}"
  printf '%s\n' "$2" > "${PINDIR}/${1}/openapi.json"
  printf '# Pin %s\n\n**Source:** fixture.\n' "$1" > "${PINDIR}/${1}/README.md"
}

reset_fixtures() {
  rm -rf "$PINDIR"
  write_version
  write_pin 0.5 "$(base_contract)"
  base_contract > "$EMIT"
  rm -rf "$REGISTER"
}

REGISTER="${WORK}/release-register"
REPO="$(cd "$(dirname "$0")/../.." && pwd)"
run_gate() { bash "$CHECK" --emitted "$EMIT" --pin-dir "$PINDIR" --version-file "$VERFILE" --register "$REGISTER" 2>&1; }

echo "test-check-openapi-pin"
echo

# ── 1. Identical: the founding state (main == the 0.5 pin, version stripped) ────────────────────
reset_fixtures
out="$(run_gate)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'identical to the current pin (0.5)'; then
  ok "identical contract: passes, naming the pin"
else bad "identical contract: passes, naming the pin" "exit=$rc" "$out"; fi

# ── 2. Additive growth: born path + born schema alongside an untouched survivor ─────────────────
reset_fixtures
jq -S '.paths["/api/blobs/{id}"].delete = {"responses":{"200":{"description":"struck"}}} | .components.schemas.BornAck = {"type":"object","properties":{"blob_id":{"type":"string"}}}' "$EMIT" > "${EMIT}.tmp" && mv "${EMIT}.tmp" "$EMIT"
out="$(run_gate)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'GREW' && printf '%s' "$out" | grep -q 'P floor'; then
  ok "additive growth: passes as grew, the P floor on the record"
else bad "additive growth: passes as grew, the P floor on the record" "exit=$rc" "$out"; fi

# ── 3. Schema removed: FAIL naming it ───────────────────────────────────────────────────────────
reset_fixtures
jq -S 'del(.components.schemas.Example)' "$EMIT" > "${EMIT}.tmp" && mv "${EMIT}.tmp" "$EMIT"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'schema removed: Example'; then
  ok "schema removed: fails naming the movement"
else bad "schema removed: fails naming the movement" "exit=$rc" "$out"; fi

# ── 4. The #906 omit-class: required shrunk, property retained — FAIL naming the schema ──────────
# This is the probe the comparator's corrected required arm exists for: the diff is
# exactly a field going absent-able against clients that typed it required.
reset_fixtures
jq -S '.components.schemas.Example.allOf[0].required = []' "$EMIT" > "${EMIT}.tmp" && mv "${EMIT}.tmp" "$EMIT"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'schema changed: Example'; then
  ok "required shrunk (the #906 omit-class): fails naming the schema"
else bad "required shrunk (the #906 omit-class): fails naming the schema" "exit=$rc" "$out"; fi

# ── 5. Required grown: FAIL naming the schema ───────────────────────────────────────────────────
reset_fixtures
jq -S '.components.schemas.Example.allOf[0].required += ["newly_required"] | .components.schemas.Example.allOf[0].properties.newly_required = {"type":"string"}' "$EMIT" > "${EMIT}.tmp" && mv "${EMIT}.tmp" "$EMIT"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'schema changed: Example'; then
  ok "required grown: fails naming the schema"
else bad "required grown: fails naming the schema" "exit=$rc" "$out"; fi

# ── 5b. Required GAINED from absent: FAIL naming the schema ─────────────────────────────────────
# The pin's schema has NO `required` key; the committed contract grows one. The
# comparator's per-key loop iterates base keys only, so without the gain-from-absent
# clause this computed grew — the independent review of the pin-gate commit caught it.
reset_fixtures
jq -S 'del(.components.schemas.Example.allOf[0].required)' "${PINDIR}/0.5/openapi.json" > "${PINDIR}/0.5/openapi.json.tmp" && mv "${PINDIR}/0.5/openapi.json.tmp" "${PINDIR}/0.5/openapi.json"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'schema changed: Example'; then
  ok "required gained from absent (the review find): fails naming the schema"
else bad "required gained from absent (the review find): fails naming the schema" "exit=$rc" "$out"; fi

# ── 6. Movement plus tolerated growth: the fail names both halves ───────────────────────────────
reset_fixtures
jq -S 'del(.components.schemas.Example) | .components.schemas.BornAck = {"type":"object"}' "$EMIT" > "${EMIT}.tmp" && mv "${EMIT}.tmp" "$EMIT"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] \
    && printf '%s' "$out" | grep -q 'schema removed: Example' \
    && printf '%s' "$out" | grep -q 'schema born: BornAck' \
    && printf '%s' "$out" | grep -q 'tolerated growth'; then
  ok "movement alongside growth: the fail names both, only the movement is red"
else bad "movement alongside growth: the fail names both, only the movement is red" "exit=$rc" "$out"; fi

# ── 7. Era routing: the actual pin vs a shape-breaking head — the gate names the two routes ────
# The founding movement (score leaving `required` on a survivor, the field going
# absent-able) must read as pin-breaking, and the failure must carry the era
# instruction (D-C2): convert to the deprecation path, or wait for the retirement
# release train.
reset_fixtures
jq -S '.paths["/api/blobs/{id}"].get.responses["200"].content["application/json"].schema.required = []' "$EMIT" > "${EMIT}.tmp" && mv "${EMIT}.tmp" "$EMIT"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] \
    && printf '%s' "$out" | grep -q 'path changed: /api/blobs/{id}' \
    && printf '%s' "$out" | grep -q 'shape break' \
    && printf '%s' "$out" | grep -q 'deprecation path' \
    && printf '%s' "$out" | grep -q 'retirement release train'; then
  ok "an absence-introducing movement fails, naming the path, with the convert-or-wait instruction"
else bad "an absence-introducing movement fails, naming the path, with the convert-or-wait instruction" "exit=$rc" "$out"; fi

# ── 8. SELECTION — the newest pin ≤ VERSION's minor is current ──────────────────────────────────
reset_fixtures
write_pin 0.6 "$(jq -S '.components.schemas = {"SixOnly":{"type":"object"}}' "$EMIT")"
jq -S '.components.schemas = {"SixOnly":{"type":"object"}}' "$EMIT" > "${EMIT}.tmp" && mv "${EMIT}.tmp" "$EMIT"
write_version 0.6.0
out="$(run_gate)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'current pin (0.6)'; then
  ok "VERSION 0.6.0 selects the 0.6 pin, not 0.5"
else bad "VERSION 0.6.0 selects the 0.6 pin, not 0.5" "exit=$rc" "$out"; fi

# ── 9. SELECTION — VERSION above the newest pin keeps judging against it (interim state) ────────
reset_fixtures
write_version 0.7.0
out="$(run_gate)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'current pin (0.5)'; then
  ok "VERSION 0.7.0 with only a 0.5 pin: judges against 0.5 (the pre-pin-cut interim)"
else bad "VERSION 0.7.0 with only a 0.5 pin: judges against 0.5 (the pre-pin-cut interim)" "exit=$rc" "$out"; fi

# ── 10. FAIL CLOSED — no pin for VERSION's major ────────────────────────────────────────────────
reset_fixtures
write_version 1.0.0
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q "no pin for major 1"; then
  ok "VERSION 1.0.0 with only 0.x pins: fails closed, naming the pin to cut"
else bad "VERSION 1.0.0 with only 0.x pins: fails closed, naming the pin to cut" "exit=$rc" "$out"; fi

# ── 11. FAIL CLOSED — a pin without its provenance README ───────────────────────────────────────
reset_fixtures
rm "${PINDIR}/0.5/README.md"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'provenance README.md'; then
  ok "pin without provenance README: fails closed"
else bad "pin without provenance README: fails closed" "exit=$rc" "$out"; fi

# ── 12. FAIL CLOSED — unparseable committed contract ────────────────────────────────────────────
reset_fixtures
printf '{"paths": broken' > "$EMIT"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'not readable JSON'; then
  ok "unparseable committed contract: fails closed, never renders as a pass"
else bad "unparseable committed contract: fails closed, never renders as a pass" "exit=$rc" "$out"; fi

# ── 13. CORRECTIONS — the recorded corrected shape is the baseline, not a waiver ────────────────
# The fixture's correction: the pinned GET answer declared `score` required, and the corrected
# shape drops that. On its own that is the #906 omit-class (case 4), so without the record it fails.
SURVIVOR='/api/blobs/{id}'
ROW='blob-reads-document-what-the-server-returned'
corrected_head() { jq -S '.paths["/api/blobs/{id}"].get.responses["200"].content["application/json"].schema.required = []' "$EMIT"; }
entry_toml() { # $1 = title, $2 = classes ("a, b")
  printf 'title = "%s"\npr = "self"\nclasses = [%s]\nsurfaces = ["http"]\nstatus = "signal-only"\nbody = "Prose."\n' \
    "$1" "$(printf '%s' "$2" | awk -F', *' '{ for (i = 1; i <= NF; i++) printf "%s\"%s\"", (i > 1 ? ", " : ""), $i }')"
}
write_register() { # $1 = classes for the entry under probe
  rm -rf "$REGISTER"
  mkdir -p "${REGISTER}/unreleased"
  cp "${REPO}/release-register/vocabulary.toml" "${REGISTER}/vocabulary.toml"
  entry_toml "Blob reads document what the server always returned" "$1" > "${REGISTER}/unreleased/${ROW}.toml"
  entry_toml "Another entry" "spec-correction" > "${REGISTER}/unreleased/another-entry.toml"
}
write_corrections() { # $1 = register entry id, $2 = path, $3 = the head to record from
  jq -n --arg row "$1" --arg p "$2" --slurpfile h "$3" \
    '{corrections: [{path: $p, method: "get", register_entry: $row, reason: "fixture", operation: $h[0].paths["/api/blobs/{id}"].get}]}' \
    > "${PINDIR}/0.5/corrections.json"
}
corrected_setup() { # $1 = classes for the row
  reset_fixtures
  corrected_head > "${EMIT}.tmp" && mv "${EMIT}.tmp" "$EMIT"
  write_register "$1"
  write_corrections "$ROW" "$SURVIVOR" "$EMIT"
}

corrected_setup "behavioral, spec-correction"
out="$(run_gate)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q '1 recorded correction(s) applied: GET /api/blobs/{id}'; then
  ok "corrections: a recorded correction declared spec-correction passes, and the pass names it"
else bad "corrections: a recorded correction declared spec-correction passes, and the pass names it" "exit=$rc" "$out"; fi

corrected_setup "behavioral, spec-correction"
rm "${PINDIR}/0.5/corrections.json"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'MOVED'; then
  ok "corrections: the same movement with no record still fails (the record is what passes it)"
else bad "corrections: the same movement with no record still fails (the record is what passes it)" "exit=$rc" "$out"; fi

corrected_setup "behavioral, spec-correction"
jq -S '.paths["/api/blobs/{id}"].get.responses["200"].content["application/json"].schema.properties.score.type = "string"' "$EMIT" > "${EMIT}.tmp" && mv "${EMIT}.tmp" "$EMIT"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'MOVED'; then
  ok "corrections: a corrected operation that moves again fails (not a waiver for its path)"
else bad "corrections: a corrected operation that moves again fails (not a waiver for its path)" "exit=$rc" "$out"; fi

corrected_setup "behavioral, spec-correction"
jq -S 'del(.components.schemas.Example)' "$EMIT" > "${EMIT}.tmp" && mv "${EMIT}.tmp" "$EMIT"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'removed'; then
  ok "corrections: movement elsewhere still fails beside an applied correction"
else bad "corrections: movement elsewhere still fails beside an applied correction" "exit=$rc" "$out"; fi

corrected_setup "behavioral"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'not spec-correction'; then
  ok "corrections: an entry not declared spec-correction fails"
else bad "corrections: an entry not declared spec-correction fails" "exit=$rc" "$out"; fi

corrected_setup "behavioral, spec-correction"
write_corrections "an-entry-nobody-wrote" "$SURVIVOR" "$EMIT"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'does not carry'; then
  ok "corrections: a register entry that does not exist fails"
else bad "corrections: a register entry that does not exist fails" "exit=$rc" "$out"; fi

# W8 — a correction outlives its entry's roll: the entry moved to a rolled window still resolves.
corrected_setup "behavioral, spec-correction"
mkdir -p "${REGISTER}/v0.5.0"
mv "${REGISTER}/unreleased/${ROW}.toml" "${REGISTER}/v0.5.0/${ROW}.toml"
out="$(run_gate)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q '1 recorded correction(s) applied'; then
  ok "W8: a correction whose entry was rolled into v0.5.0/ still resolves"
else bad "W8: a correction whose entry was rolled still resolves" "exit=$rc" "$out"; fi

corrected_setup "behavioral, spec-correction"
write_corrections "$ROW" "/api/nowhere" "$EMIT"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'does not have'; then
  ok "corrections: correcting an operation the pin lacks fails"
else bad "corrections: correcting an operation the pin lacks fails" "exit=$rc" "$out"; fi

corrected_setup "behavioral, spec-correction"
jq '.corrections += .corrections' "${PINDIR}/0.5/corrections.json" > "${WORK}/c.tmp" && mv "${WORK}/c.tmp" "${PINDIR}/0.5/corrections.json"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'more than once'; then
  ok "corrections: correcting one operation twice fails"
else bad "corrections: correcting one operation twice fails" "exit=$rc" "$out"; fi

# ── 14. COMPONENT CORRECTIONS — a `$ref`'d component's recorded shape is the baseline ───────────
# The fixture mirrors the first real instance: an open string enum pinned as a `oneOf` whose bare
# string arm overlaps its named arm (so no value validates against exactly one), corrected to
# `anyOf`. The pinned operation reaches it only through a `$ref`, so an OPERATION correction cannot
# record it without restating the operation; the component correction records it once.
CROW='open-enums-publish-any-of'
OPEN_ENUM_ONE_OF='{"oneOf":[{"type":"string","enum":["known"]},{"type":"string","enum":["also_known"]},{"type":"string"}],"description":"An open reason."}'
with_open_enum() { # $1 = file: add the open enum (as pinned) and a $ref to it from the GET answer
  jq -S --argjson e "$OPEN_ENUM_ONE_OF" \
    '.components.schemas.OpenReason = $e
     | .paths["/api/blobs/{id}"].get.responses["200"].content["application/json"].schema.properties.reason = {"$ref":"#/components/schemas/OpenReason"}' \
    "$1" > "${1}.tmp" && mv "${1}.tmp" "$1"
}
any_of_head() { # the corrected publication: the same arms under anyOf
  jq -S '.components.schemas.OpenReason |= ({anyOf: .oneOf} + del(.oneOf))' "$EMIT" > "${EMIT}.tmp" && mv "${EMIT}.tmp" "$EMIT"
}
write_component_register() { # $1 = classes for the component entry
  rm -rf "$REGISTER"
  mkdir -p "${REGISTER}/unreleased"
  cp "${REPO}/release-register/vocabulary.toml" "${REGISTER}/vocabulary.toml"
  entry_toml "Open enums publish anyOf" "$1" > "${REGISTER}/unreleased/${CROW}.toml"
  entry_toml "Blob reads document what the server always returned" "behavioral, spec-correction" > "${REGISTER}/unreleased/${ROW}.toml"
}
write_component_corrections() { # $1 = register entry id, $2 = component name
  jq -n --arg row "$1" --arg c "$2" --slurpfile h "$EMIT" \
    '{component_corrections: [{component: $c, register_entry: $row, reason: "fixture", schema: $h[0].components.schemas.OpenReason}]}' \
    > "${PINDIR}/0.5/corrections.json"
}
component_setup() { # $1 = classes for the component entry
  reset_fixtures
  with_open_enum "${PINDIR}/0.5/openapi.json"
  with_open_enum "$EMIT"
  any_of_head
  write_component_register "$1"
  write_component_corrections "$CROW" "OpenReason"
}

# (a) the anyOf change passes with the component correction recorded, and the pass names it.
component_setup "spec-correction"
out="$(run_gate)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q '1 recorded correction(s) applied: component OpenReason'; then
  ok "component corrections: oneOf→anyOf on a \$ref'd component passes with the record, and the pass names it"
else bad "component corrections: oneOf→anyOf on a \$ref'd component passes with the record, and the pass names it" "exit=$rc" "$out"; fi

# (b) the same change with the record removed fails, naming the component.
component_setup "spec-correction"
rm "${PINDIR}/0.5/corrections.json"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'schema changed: OpenReason'; then
  ok "component corrections: the same change with no record fails, naming the component"
else bad "component corrections: the same change with no record fails, naming the component" "exit=$rc" "$out"; fi

# (c) a component correction whose register entry is not spec-correction, or is missing, fails.
component_setup "behavioral"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'not spec-correction'; then
  ok "component corrections: an entry not declared spec-correction fails"
else bad "component corrections: an entry not declared spec-correction fails" "exit=$rc" "$out"; fi

component_setup "spec-correction"
write_component_corrections "an-entry-nobody-wrote" "OpenReason"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'does not carry'; then
  ok "component corrections: a register entry that does not exist fails"
else bad "component corrections: a register entry that does not exist fails" "exit=$rc" "$out"; fi

# The record is DERIVED: exactly the pinned component with `oneOf` renamed to `anyOf`. The published
# component is then judged against it by the comparator, like any other schema.
MISMATCH='is not the 0.5 pin.s component with `oneOf` renamed to `anyOf`: OpenReason'
published() { jq -S "$1" "$EMIT" > "${EMIT}.tmp" && mv "${EMIT}.tmp" "$EMIT"; }
record() { jq "$1" "${PINDIR}/0.5/corrections.json" > "${WORK}/c.tmp" && mv "${WORK}/c.tmp" "${PINDIR}/0.5/corrections.json"; }

# Growth since the pin needs NO edit to corrections.json: a new named arm before the catch-all in
# the PUBLISHED schema passes against the unchanged derived record.
component_setup "spec-correction"
published '.components.schemas.OpenReason.anyOf |= (.[0:1] + [{"type":"string","enum":["newer"]}] + .[1:])'
out="$(run_gate)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'GREW'; then
  ok "component corrections: a new named arm in the published schema passes with no corrections.json edit"
else bad "component corrections: a new named arm in the published schema passes with no corrections.json edit" "exit=$rc" "$out"; fi

# Movement in the published schema is judged against the record and refuses: a renamed, dropped or
# reordered named arm, a type change, a new `required`, an arm after the catch-all.
for m in '.components.schemas.OpenReason.anyOf[0].enum = ["renamed"]::renamed named arm' \
         '.components.schemas.OpenReason.anyOf |= del(.[0])::dropped named arm' \
         '.components.schemas.OpenReason.anyOf |= ([.[1], .[0]] + [{"type":"string","enum":["newer"]}] + .[2:])::growth beside a reorder' \
         '.components.schemas.OpenReason = {"type":"integer","enum":[1]}::type change' \
         '.components.schemas.OpenReason.required = ["x"]::new required' \
         '.components.schemas.OpenReason.anyOf += [{"type":"string","enum":["late"]}]::arm after the catch-all'; do
  component_setup "spec-correction"
  published "${m%::*}"
  out="$(run_gate)"; rc=$?
  if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'schema changed: OpenReason'; then
    ok "component corrections: a published ${m##*::} is judged against the record and fails"
  else bad "component corrections: a published ${m##*::} is judged against the record and fails" "exit=$rc" "$out"; fi
done

# A record that differs from pin-with-rename in ANY way refuses, naming the component — however it
# is arranged against the published shape. Review repro (a), the `{}` record, in three framings;
# repro (b), a dropped arm recorded to match; and records that re-record growth, a rename, a type
# change, a new `required`, or only a prose edit.
for r in '.component_corrections[0].schema = {}::a {} record beside the real shape::' \
         '.component_corrections[0].schema = {}::a {} record beside an integer-enum publication::.components.schemas.OpenReason = {"type":"integer","enum":[1]}' \
         '.component_corrections[0].schema = {}::a {} record matching a {} publication::.components.schemas.OpenReason = {}' \
         '.component_corrections[0].schema.anyOf |= del(.[0])::a dropped arm recorded to match::.components.schemas.OpenReason.anyOf |= del(.[0])' \
         '.component_corrections[0].schema.anyOf |= (.[0:1] + [{"type":"string","enum":["newer"]}] + .[1:])::re-recorded growth::.components.schemas.OpenReason.anyOf |= (.[0:1] + [{"type":"string","enum":["newer"]}] + .[1:])' \
         '.component_corrections[0].schema.anyOf[0].enum = ["renamed"]::a re-recorded rename::.components.schemas.OpenReason.anyOf[0].enum = ["renamed"]' \
         '.component_corrections[0].schema = {"type":"integer","enum":[1]}::a re-recorded type change::.components.schemas.OpenReason = {"type":"integer","enum":[1]}' \
         '.component_corrections[0].schema.required = ["x"]::a re-recorded new required::.components.schemas.OpenReason.required = ["x"]' \
         '.component_corrections[0].schema.description = "reworded"::a record differing only in prose::'; do
  rec="${r%%::*}"; rest="${r#*::}"; label="${rest%%::*}"; pub="${rest#*::}"
  component_setup "spec-correction"
  [ -n "$pub" ] && published "$pub"
  record "$rec"
  out="$(run_gate)"; rc=$?
  if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q "$MISMATCH"; then
    ok "component corrections: ${label} is not the derived record and fails"
  else bad "component corrections: ${label} is not the derived record and fails" "exit=$rc" "$out"; fi
done

# Only an OPEN VOCABULARY can be corrected (final review, points 2 and 3). A tagged union published
# as anyOf under a recorded correction, a closed oneOf of enums, and a component with no top-level
# oneOf at all (a do-nothing "correction" equal to the pin) are each refused, naming the component.
NOTOPEN='does not publish as an open vocabulary .*: '
union_setup() { # $1 = the pinned shape of component Union
  component_setup "spec-correction"
  jq -S --argjson u "$1" '.components.schemas.Union = $u' "${PINDIR}/0.5/openapi.json" > "${PINDIR}/0.5/openapi.json.tmp" && mv "${PINDIR}/0.5/openapi.json.tmp" "${PINDIR}/0.5/openapi.json"
  jq -S --argjson u "$1" '.components.schemas.Union = (if ($u | has("oneOf")) then {anyOf: $u.oneOf} + ($u | del(.oneOf)) else $u end)' "$EMIT" > "${EMIT}.tmp" && mv "${EMIT}.tmp" "$EMIT"
  jq --arg row "$CROW" --slurpfile h "$EMIT" '.component_corrections += [{component: "Union", register_entry: $row, reason: "fixture", schema: $h[0].components.schemas.Union}]' \
    "${PINDIR}/0.5/corrections.json" > "${WORK}/c.tmp" && mv "${WORK}/c.tmp" "${PINDIR}/0.5/corrections.json"
}
for u in '{"oneOf":[{"type":"object","properties":{"kind":{"type":"string","enum":["a"]}},"required":["kind"]},{"type":"object","properties":{"kind":{"type":"string","enum":["b"]}},"required":["kind"]}]}::a tagged union' \
         '{"oneOf":[{"type":"string","enum":["a"]},{"type":"string","enum":["b"]}]}::a closed oneOf of enums' \
         '{"oneOf":[{"type":"string"},{"type":"string","enum":["a"]}]}::an open vocabulary whose catch-all is not last' \
         '{"type":"object","properties":{"id":{"type":"string"}}}::a component with no oneOf (a do-nothing correction)'; do
  union_setup "${u%%::*}"
  out="$(run_gate)"; rc=$?
  if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q "${NOTOPEN}Union"; then
    ok "component corrections: correcting ${u##*::} is refused, naming it"
  else bad "component corrections: correcting ${u##*::} is refused, naming it" "exit=$rc" "$out"; fi
done

# Nit 6: an id carrying a newline would resolve as two entries; it is refused as one bad token.
component_setup "spec-correction"
jq --arg row "${CROW}"$'\n'"${ROW}" '.component_corrections[0].register_entry = $row' "${PINDIR}/0.5/corrections.json" > "${WORK}/c.tmp" && mv "${WORK}/c.tmp" "${PINDIR}/0.5/corrections.json"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'not one lowercase kebab-case token'; then
  ok "corrections: a register_entry carrying a newline fails"
else bad "corrections: a register_entry carrying a newline fails" "exit=$rc" "$out"; fi

# Not a waiver for anything else: other movement beside the correction still fails.
component_setup "spec-correction"
jq -S 'del(.components.schemas.Example)' "$EMIT" > "${EMIT}.tmp" && mv "${EMIT}.tmp" "$EMIT"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'schema removed: Example'; then
  ok "component corrections: movement elsewhere still fails beside an applied component correction"
else bad "component corrections: movement elsewhere still fails beside an applied component correction" "exit=$rc" "$out"; fi

component_setup "spec-correction"
write_component_corrections "$CROW" "NoSuchComponent"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'corrects a component the 0.5 pin does not have: NoSuchComponent'; then
  ok "component corrections: correcting a component the pin lacks fails"
else bad "component corrections: correcting a component the pin lacks fails" "exit=$rc" "$out"; fi

component_setup "spec-correction"
jq '.component_corrections += .component_corrections' "${PINDIR}/0.5/corrections.json" > "${WORK}/c.tmp" && mv "${WORK}/c.tmp" "${PINDIR}/0.5/corrections.json"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'same component more than once'; then
  ok "component corrections: correcting one component twice fails"
else bad "component corrections: correcting one component twice fails" "exit=$rc" "$out"; fi

# Removal is not a correction: the recorded schema must be an object.
component_setup "spec-correction"
jq '.component_corrections[0].schema = null' "${PINDIR}/0.5/corrections.json" > "${WORK}/c.tmp" && mv "${WORK}/c.tmp" "${PINDIR}/0.5/corrections.json"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'an object schema'; then
  ok "component corrections: a null schema (a removal) is refused"
else bad "component corrections: a null schema (a removal) is refused" "exit=$rc" "$out"; fi

# A misspelled array name refuses rather than silently applying nothing.
component_setup "spec-correction"
jq '{component_correction: .component_corrections}' "${PINDIR}/0.5/corrections.json" > "${WORK}/c.tmp" && mv "${WORK}/c.tmp" "${PINDIR}/0.5/corrections.json"
out="$(run_gate)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'does not read: component_correction'; then
  ok "component corrections: an unknown top-level key fails, naming it"
else bad "component corrections: an unknown top-level key fails, naming it" "exit=$rc" "$out"; fi

# (d) operation and component corrections together: both apply, and the pass names both.
component_setup "spec-correction"
corrected_head > "${EMIT}.tmp" && mv "${EMIT}.tmp" "$EMIT"
jq --arg row "$ROW" --slurpfile h "$EMIT" \
  '.corrections = [{path: "/api/blobs/{id}", method: "get", register_entry: $row, reason: "fixture", operation: $h[0].paths["/api/blobs/{id}"].get}]' \
  "${PINDIR}/0.5/corrections.json" > "${WORK}/c.tmp" && mv "${WORK}/c.tmp" "${PINDIR}/0.5/corrections.json"
out="$(run_gate)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q '2 recorded correction(s) applied: GET /api/blobs/{id}, component OpenReason'; then
  ok "corrections: operation and component corrections apply together, and the pass names both"
else bad "corrections: operation and component corrections apply together, and the pass names both" "exit=$rc" "$out"; fi

# ── 15. THE REAL TREE — the four open string enums against the current pin ──────────────────────
# The fixtures above prove the rule; this proves the record. It runs the gate on the repository's
# own contract, pin tree, VERSION and register, then again with the component corrections removed.
# Kept to those four names on purpose: a fifth component correction landing later is its own
# record and its own witness, not something this case should absorb.
#
# Self-retiring on one exact condition only: the selected pin ITSELF publishes all four as `anyOf`
# (a pin cut after the fix), which needs no record. Then it prints a SKIP naming that. While the
# pin still publishes them as `oneOf`, a missing record for any of the four FAILS.
REAL="${WORK}/real"
mkdir -p "$REAL"
cp -R "${REPO}/schemas/versions" "${REAL}/versions"
REAL_VERSION="$(tr -d '[:space:]' < "${REPO}/VERSION")"
REAL_MM="${REAL_VERSION%.*}"
REAL_PIN="${REAL}/versions/${REAL_MM}/corrections.json"
real_gate() { bash "$CHECK" --emitted "${REPO}/openapi.json" --pin-dir "${REAL}/versions" --version-file "${REPO}/VERSION" --register "${REPO}/release-register" 2>&1; }
OPEN_ENUMS='["ActName","IdKind","RefusalReason","ScoreKind"]'
REAL_PIN_JSON="${REAL}/versions/${REAL_MM}/openapi.json"
if jq -e --argjson want "$OPEN_ENUMS" '. as $p | all($want[]; . as $n | ($p.components.schemas[$n] | has("anyOf") and (has("oneOf") | not)))' "$REAL_PIN_JSON" >/dev/null 2>&1; then
  # The one condition that retires this case: the pin itself publishes all four as anyOf.
  echo "  SKIP: real tree: the ${REAL_MM} pin already publishes the four open enums as anyOf, so it needs no component correction for them. Retire case 15."
else
  MISSING="$(jq -r --argjson want "$OPEN_ENUMS" '[(.component_corrections // [])[].component] as $have | $want[] | select(. as $w | $have | index($w) | not)' "$REAL_PIN" 2>/dev/null)"
  if [ ! -f "$REAL_PIN" ] || [ -n "$MISSING" ]; then
    bad "real tree: the ${REAL_MM} pin publishes the open enums as oneOf, so its corrections.json must record all four" "missing: ${MISSING:-the whole corrections.json}"
  else
    out="$(real_gate)"; rc=$?
    if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'component ActName, component IdKind, component RefusalReason, component ScoreKind'; then
      ok "real tree: the open enums' anyOf publication passes with its component corrections recorded"
    else bad "real tree: the open enums' anyOf publication passes with its component corrections recorded" "exit=$rc" "$out"; fi

    # Final review repro: a do-nothing correction for a real component that is no open vocabulary.
    cp "$REAL_PIN" "${REAL_PIN}.keep"
    jq --slurpfile pin "$REAL_PIN_JSON" '.component_corrections += [{component: "AcceptInvitationResponse", register_entry: "open-enums-publish-any-of", reason: "probe", schema: $pin[0].components.schemas.AcceptInvitationResponse}]' \
      "$REAL_PIN" > "${REAL_PIN}.tmp" && mv "${REAL_PIN}.tmp" "$REAL_PIN"
    out="$(real_gate)"; rc=$?
    if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'does not publish as an open vocabulary .*: AcceptInvitationResponse'; then
      ok "real tree: a do-nothing correction for AcceptInvitationResponse is refused, naming it"
    else bad "real tree: a do-nothing correction for AcceptInvitationResponse is refused, naming it" "exit=$rc" "$out"; fi
    mv "${REAL_PIN}.keep" "$REAL_PIN"

    jq 'del(.component_corrections)' "$REAL_PIN" > "${REAL_PIN}.tmp" && mv "${REAL_PIN}.tmp" "$REAL_PIN"
    out="$(real_gate)"; rc=$?
    named=0
    for c in ActName IdKind RefusalReason ScoreKind; do
      printf '%s' "$out" | grep -q "schema changed: ${c}$" && named=$((named + 1))
    done
    if [ "$rc" -ne 0 ] && [ "$named" -eq 4 ]; then
      ok "real tree: the same publication with the component corrections removed fails, naming all four"
    else bad "real tree: the same publication with the component corrections removed fails, naming all four" "exit=$rc" "named=$named" "$out"; fi
  fi
fi

echo
echo "  ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
