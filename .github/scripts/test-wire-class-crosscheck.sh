#!/usr/bin/env bash
# .github/scripts/test-wire-class-crosscheck.sh
#
# Test harness for wire-class-crosscheck.sh — the declared-class gate for the openapi/register
# side of the wire (shared-semver-policy spec §4, D-S4), sibling to the SQL gate's harness
# test-sqlx-schema-crosscheck.sh.
#
# TWO PROBES EXIST BECAUSE OF WHAT A GREEN CHECK MAY NOT CLAIM
# ------------------------------------------------------------
# The gate's structural bar (spec §4, verbatim): "CI is structurally barred from certifying
# the behavioral class green — behavior is not in the diff." So the harness pins the boundary
# from both sides: the shape-underdeclaration probe proves the gate CAN fail when the diff
# contradicts the declaration, and the over-declaration probe proves it stays green (NOTED)
# when the declaration exceeds the diff — failing an honest over-declaration would train
# under-declaration, the behaviour that produced the 2026-07-30 outage on the SQL side.
# Presence is probed against the ROW, not the file: a diff that edits the register without
# adding its own row must still fail.
#
#   bash .github/scripts/test-wire-class-crosscheck.sh

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CHECK="${SCRIPT_DIR}/wire-class-crosscheck.sh"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; shift; printf '    %s\n' "$@"; FAIL=$((FAIL + 1)); }

WIRE="${WORK}/wire.txt"
ADDED="${WORK}/added.txt"
REG="${WORK}/register.md"

# A minimal well-formed register, as the header documents it.
reset_register() {
  cat > "$REG" <<'MD'
# Release-verdict register

## Since v0.4.0 — unreleased

- **Citation 1 — some prior PR's behavioral row**
  Prose.
pr: 867
classes: behavioral
surfaces: http
status: open
MD
}

reset_fixtures() {
  reset_register
  : > "$WIRE"
  : > "$ADDED"
}

add_own_row() { # $1 = pr value, $2 = classes value
  cat >> "$ADDED" <<MD
- **Citation N — this PR's row**
  Prose.
pr: $1
classes: $2
surfaces: http
status: signal-only
MD
}

# Every probe runs in full fixture mode: all three evidence seams supplied, so the harness
# never touches git and stays pure bash.
run_check() { # $1 = shape verdict, remaining args passed through (e.g. --pr 501)
  local sv="$1"; shift
  bash "$CHECK" --wire-touched "$WIRE" --added-register "$ADDED" \
      --shape-verdict "$sv" --register "$REG" "$@" 2>&1
}

echo "test-wire-class-crosscheck"
echo

# ── 1. No wire paths changed: nothing to check, even with the register in hand ──────────────────
reset_fixtures
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'no wire paths changed'; then
  ok "no wire paths: passes, saying why"
else bad "no wire paths: passes, saying why" "exit=$rc" "$out"; fi

# ── 2. PRESENCE — wire paths changed, no row naming this PR: FAIL, naming the files ─────────────
reset_fixtures
printf '%s\n' "openapi.json" "clients/temper-rb/lib/temper/generated.rb" > "$WIRE"
add_own_row 867 behavioral   # a row lands in this diff — but it names ANOTHER PR
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -ne 0 ] \
    && printf '%s' "$out" | grep -q 'adds no register row for this PR' \
    && printf '%s' "$out" | grep -q 'openapi.json'; then
  ok "presence: wire-touched with no own row fails, naming the wire files"
else bad "presence: wire-touched with no own row fails, naming the wire files" "exit=$rc" "$out"; fi

# ── 3. PRESENCE — pr: self satisfies it (the number is unknowable before push) ──────────────────
reset_fixtures
printf '%s\n' "openapi.json" > "$WIRE"
add_own_row self additive
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -qi 'D-S3 baseline'; then
  ok "pr: self + additive + info.version-only diff: passes as the D-S3 baseline"
else bad "pr: self + additive + info.version-only diff: passes as the D-S3 baseline" "exit=$rc" "$out"; fi

# ── 4. PRESENCE — the real PR number satisfies it when the caller knows it; a wrong one does not ─
reset_fixtures
printf '%s\n' "packages/temper-ui/package.json" > "$WIRE"
add_own_row 501 additive
out="$(run_check unchanged --pr 501)"; rc=$?
if [ "$rc" -eq 0 ]; then
  ok "pr: <real number> satisfies presence when --pr matches"
else bad "pr: <real number> satisfies presence when --pr matches" "exit=$rc" "$out"; fi
out="$(run_check unchanged --pr 502)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'adds no register row for this PR'; then
  ok "--pr naming a different number than the row: still fails presence"
else bad "--pr naming a different number than the row: still fails presence" "exit=$rc" "$out"; fi

# ── 5. PARSE — an invalid classes token fails, naming the line ──────────────────────────────────
reset_fixtures
printf '%s\n' "crates/temper-mcp/src/lib.rs" > "$WIRE"
sed 's/^classes: behavioral$/classes: breaking/' "$REG" > "${REG}.tmp" && mv "${REG}.tmp" "$REG"
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -ne 0 ] \
    && printf '%s' "$out" | grep -q "classes: 'breaking' is not one of" \
    && printf '%s' "$out" | grep -q 'register.md:'; then
  ok "parse: 'breaking' fails, naming the file and line"
else bad "parse: 'breaking' fails, naming the file and line" "exit=$rc" "$out"; fi

# ── 6. PARSE — a nameless blocker (empty after blocked:) fails ──────────────────────────────────
reset_fixtures
printf '%s\n' "crates/temper-mcp/src/lib.rs" > "$WIRE"
sed 's/^status: open$/status: blocked:/' "$REG" > "${REG}.tmp" && mv "${REG}.tmp" "$REG"
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q "names no release class"; then
  ok "parse: 'blocked:' with no class name fails"
else bad "parse: 'blocked:' with no class name fails" "exit=$rc" "$out"; fi

# ── 7. PARSE — a non-numeric, non-sentinel pr: fails ────────────────────────────────────────────
reset_fixtures
printf '%s\n' "crates/temper-mcp/src/lib.rs" > "$WIRE"
sed 's/^pr: 867$/pr: 867a/' "$REG" > "${REG}.tmp" && mv "${REG}.tmp" "$REG"
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q "pr: '867a'"; then
  ok "parse: 'pr: 867a' fails the pr vocabulary"
else bad "parse: 'pr: 867a' fails the pr vocabulary" "exit=$rc" "$out"; fi

# ── 8. SHAPE — moved beyond .info.version with only additive declared: FAIL ─────────────────────
# The #858 class: additive is a lie about a rename.
reset_fixtures
printf '%s\n' "openapi.json" > "$WIRE"
add_own_row self additive
out="$(run_check moved)"; rc=$?
if [ "$rc" -ne 0 ] \
    && printf '%s' "$out" | grep -q 'beyond .info.version' \
    && printf '%s' "$out" | grep -q 'shape-breaking'; then
  ok "shape moved + only additive declared: fails (the #858 class)"
else bad "shape moved + only additive declared: fails (the #858 class)" "exit=$rc" "$out"; fi

# ── 9. SHAPE — moved with a behavioral-only row: FAIL ───────────────────────────────────────────
# 'behavioral' does not answer the shape question the openapi diff poses.
reset_fixtures
printf '%s\n' "openapi.json" > "$WIRE"
add_own_row self behavioral
out="$(run_check moved)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'beyond .info.version'; then
  ok "shape moved + behavioral-only row: fails"
else bad "shape moved + behavioral-only row: fails" "exit=$rc" "$out"; fi

# ── 10. SHAPE — moved and shape-breaking declared: pass ─────────────────────────────────────────
reset_fixtures
printf '%s\n' "openapi.json" > "$WIRE"
add_own_row self shape-breaking
out="$(run_check moved)"; rc=$?
if [ "$rc" -eq 0 ]; then
  ok "shape moved + shape-breaking declared: passes"
else bad "shape moved + shape-breaking declared: passes" "exit=$rc" "$out"; fi

# ── 11. SHAPE — declared shape-breaking, diff shows no movement: pass, NOTED ────────────────────
# The asymmetry probe: a break can hide from the only shape record CI has, and failing an
# honest over-declaration would train under-declaration. It must pass AND say something.
reset_fixtures
printf '%s\n' "openapi.json" > "$WIRE"
add_own_row self shape-breaking
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -qi 'noted'; then
  ok "no shape movement + shape-breaking declared: passes AND is noted"
else bad "no shape movement + shape-breaking declared: passes AND is noted" "exit=$rc" "$out"; fi

# ── 12. SHAPE — undeterminable while openapi.json changed: never renders as pass ────────────────
# The canary's lesson: 'I could not look' is not 'nothing moved'.
reset_fixtures
printf '%s\n' "openapi.json" > "$WIRE"
add_own_row self additive
out="$(run_check undeterminable)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -qi 'could not'; then
  ok "undeterminable shape + openapi.json touched: fails loudly"
else bad "undeterminable shape + openapi.json touched: fails loudly" "exit=$rc" "$out"; fi

# ── 13. The behavioral bar — a behavioral-only row on a non-openapi wire path passes shape ──────
# MCP/CLI-stdout/clients changes carry no committed shape record, so shape honesty has nothing
# to check; presence is the gate there. This pins the field of view: the gate must NOT invent
# an openapi ruling for a PR that never touched openapi.json.
reset_fixtures
printf '%s\n' "crates/temper-mcp/src/lib.rs" > "$WIRE"
add_own_row self behavioral
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -eq 0 ] && ! printf '%s' "$out" | grep -qi 'shape (openapi.json'; then
  ok "non-openapi wire path: presence-only, no shape ruling invented"
else bad "non-openapi wire path: presence-only, no shape ruling invented" "exit=$rc" "$out"; fi

# ── 14. FIELD OF VIEW — the crates/temper-api/ prefix family carries the same gate ──────────────
# PR #871's CI failure was exactly this shape: routes + handlers under crates/temper-api/
# (born doors, openapi.json untouched) with no register row. The failure output names the
# prefix's files, and the row that names the PR turns the same diff green.
reset_fixtures
printf '%s\n' "crates/temper-api/src/routes.rs" "crates/temper-api/src/handlers/erasure.rs" > "$WIRE"
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -ne 0 ] \
    && printf '%s' "$out" | grep -q 'adds no register row for this PR' \
    && printf '%s' "$out" | grep -q 'crates/temper-api/'; then
  ok "temper-api wire paths with no own row: fails, naming them (the #871 class)"
else bad "temper-api wire paths with no own row: fails, naming them (the #871 class)" "exit=$rc" "$out"; fi

reset_fixtures
printf '%s\n' "crates/temper-api/src/routes.rs" > "$WIRE"
add_own_row self additive
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -eq 0 ] && ! printf '%s' "$out" | grep -qi 'shape (openapi.json'; then
  ok "temper-api wire paths + own row: passes, presence-only, no shape ruling invented"
else bad "temper-api wire paths + own row: passes, presence-only, no shape ruling invented" "exit=$rc" "$out"; fi

# ── 15. DERIVATION — the shape comparator itself (wire-shape.jq) ────────────────────────────────
# The verdict-handling probes above feed --shape-verdict, so they never exercise the jq that
# COMPUTES the verdict. The #874 class lived exactly there: a schema emitted as
# `{allOf: [...], description}` leaf-compared its array whole, so a PROSE edit the whitelist
# exists to allow read as a break and failed an honest additive declaration. These probes pin
# the comparator directly, both directions.
SHAPE_JQ="${SCRIPT_DIR}/wire-shape.jq"
shape_fixture() { # $1 = file, payload on stdin
  jq -S 'del(.info.version)' > "$1"
}
derive_with() { # $1 = base fixture, $2 = head fixture
  jq -n -r -f "$SHAPE_JQ" --slurpfile base "$1" --slurpfile head "$2" 2>&1
}

BASE_OAS="${WORK}/base.json"
HEAD_OAS="${WORK}/head.json"

# The base contract: one path, one allOf-shaped schema (the utoipa flattened-doc shape).
cat > "$BASE_OAS" <<'JSON'
{"paths":{"/api/blobs/{id}":{"get":{"responses":{"200":{"description":"ok"}}}}},
 "components":{"schemas":{"Example":{"allOf":[{"type":"object","properties":{"peer_table":{"type":"string","description":"`kb_resources` | `kb_cogmaps` | `kb_blobs` — the peer endpoint's table."}},"required":["peer_table"]}],"description":"Assert a relation."}}}}
JSON
cp "$BASE_OAS" "$HEAD_OAS"

# 15a — prose edit INSIDE the allOf array: grew, never moved.
jq -S '.components.schemas.Example.allOf[0].properties.peer_table.description = "`kb_resources` — the peer endpoint'"'"'s table (blob-relation peers narrow)."' "$BASE_OAS" > "$HEAD_OAS"
v="$(derive_with "$BASE_OAS" "$HEAD_OAS")"
if [ "$v" = "grew" ]; then
  ok "derivation: a prose edit inside allOf computes as grew (the #874 class)"
else bad "derivation: a prose edit inside allOf computes as grew (the #874 class)" "verdict=$v"; fi

# 15b — a rename INSIDE the allOf member: moved. The whitelist must not leak through arrays.
jq -S '.components.schemas.Example.allOf[0].properties.peer_type = .components.schemas.Example.allOf[0].properties.peer_table | del(.components.schemas.Example.allOf[0].properties.peer_table)' "$BASE_OAS" > "$HEAD_OAS"
v="$(derive_with "$BASE_OAS" "$HEAD_OAS")"
if [ "$v" = "moved" ]; then
  ok "derivation: a rename inside allOf still computes as moved"
else bad "derivation: a rename inside allOf still computes as moved" "verdict=$v"; fi

# 15c — an ADDED allOf member (compositor growth = an intersection tightening): moved.
jq -S '.components.schemas.Example.allOf += [{"type":"object","required":["extra"]}]' "$BASE_OAS" > "$HEAD_OAS"
v="$(derive_with "$BASE_OAS" "$HEAD_OAS")"
if [ "$v" = "moved" ]; then
  ok "derivation: an added allOf member (length change) computes as moved"
else bad "derivation: an added allOf member (length change) computes as moved" "verdict=$v"; fi

# 15d — born path and born schema alongside the untouched survivor: grew (the canonical additive).
jq -S '.paths["/api/blobs/{id}"].delete = {"responses":{"200":{"description":"struck"}}} | .components.schemas.BornAck = {"type":"object","properties":{"blob_id":{"type":"string"}}}' "$BASE_OAS" > "$HEAD_OAS"
v="$(derive_with "$BASE_OAS" "$HEAD_OAS")"
if [ "$v" = "grew" ]; then
  ok "derivation: a born operation and a born schema alongside an untouched survivor compute as grew"
else bad "derivation: a born operation and a born schema alongside an untouched survivor compute as grew" "verdict=$v"; fi

# 15e — required GROWN inside allOf: moved; required SHRUNK: the tolerant direction, grew.
jq -S '.components.schemas.Example.allOf[0].required = ["peer_table","extra"]' "$BASE_OAS" > "$HEAD_OAS"
v="$(derive_with "$BASE_OAS" "$HEAD_OAS")"
if [ "$v" = "moved" ]; then
  ok "derivation: required grown inside allOf computes as moved"
else bad "derivation: required grown inside allOf computes as moved" "verdict=$v"; fi
jq -S '.components.schemas.Example.allOf[0].required = []' "$BASE_OAS" > "$HEAD_OAS"
v="$(derive_with "$BASE_OAS" "$HEAD_OAS")"
if [ "$v" = "grew" ]; then
  ok "derivation: required shrunk (the tolerant direction) computes as grew"
else bad "derivation: required shrunk (the tolerant direction) computes as grew" "verdict=$v"; fi

echo
echo "  ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
