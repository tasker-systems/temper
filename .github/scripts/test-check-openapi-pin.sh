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
}

run_gate() { bash "$CHECK" --emitted "$EMIT" --pin-dir "$PINDIR" --version-file "$VERFILE" 2>&1; }

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

echo
echo "  ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
