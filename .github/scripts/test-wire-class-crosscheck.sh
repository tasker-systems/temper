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
# In-place sed without `-i`: GNU and BSD (macOS) sed disagree on its argument, and BSD reads the
# script as a backup suffix — so a local run fails every probe whose fixture is edited this way.
sed_inplace() { # $1 = sed script, $2 = file
  sed "$1" "$2" > "$2.tmp" && mv "$2.tmp" "$2"
}

WIRE="${WORK}/wire.txt"
ADDED="${WORK}/added"
REG="${WORK}/release-register"
LEGACY="${WORK}/RELEASE_REGISTER.md"
REPO="$(cd "${SCRIPT_DIR}/../.." && pwd)"

# A minimal well-formed register: the repo's own vocabulary and one prior PR's entry.
reset_register() {
  rm -rf "$REG"
  mkdir -p "${REG}/unreleased"
  cp "${REPO}/release-register/vocabulary.toml" "${REG}/vocabulary.toml"
  cat > "${REG}/unreleased/prior-row.toml" <<'TOML'
title    = "Citation 1 — some prior PR's behavioral entry"
pr       = 867
classes  = ["behavioral"]
surfaces = ["http"]
status   = "open"
body = """
Prose.
"""
TOML
}

reset_fixtures() {
  reset_register
  : > "$WIRE"
  rm -rf "$ADDED" "$LEGACY"
  mkdir -p "$ADDED"
  OWN=0
}

# "a, b" -> "\"a\", \"b\"" (a TOML array body); a bare number stays a number.
toml_list() { printf '%s' "$1" | awk -F', *' '{ for (i = 1; i <= NF; i++) printf "%s\"%s\"", (i > 1 ? ", " : ""), $i }'; }
toml_pr() { case "$1" in *[!0-9]*) printf '"%s"' "$1" ;; *) printf '%s' "$1" ;; esac; }

add_own_row() { # $1 = pr value, $2 = classes value ("a, b"), $3 = surfaces (default "http"); one entry file the diff adds
  OWN=$((OWN + 1))
  cat > "${ADDED}/this-pr-${OWN}.toml" <<TOML
title    = "Citation N — this PR's entry"
pr       = $(toml_pr "$1")
classes  = [$(toml_list "$2")]
surfaces = [$(toml_list "${3:-http}")]
status   = "signal-only"
body = """
Prose.
"""
TOML
}

# Every probe runs in full fixture mode: all evidence seams supplied, so the harness never
# touches git and stays pure bash (plus the python3 reader the gate itself calls).
run_check() { # $1 = shape verdict, remaining args passed through (e.g. --pr 501)
  local sv="$1"; shift
  bash "$CHECK" --wire-touched "$WIRE" --added-entries "$ADDED" \
      --shape-verdict "$sv" --register-dir "$REG" --legacy-register "$LEGACY" "$@" 2>&1
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
    && printf '%s' "$out" | grep -q 'adds no register entry for this PR' \
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
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'adds no register entry for this PR'; then
  ok "--pr naming a different number than the row: still fails presence"
else bad "--pr naming a different number than the row: still fails presence" "exit=$rc" "$out"; fi

# ── 5. PARSE — an invalid classes token fails, naming the line ──────────────────────────────────
reset_fixtures
printf '%s\n' "crates/temper-mcp/src/lib.rs" > "$WIRE"
sed_inplace 's/^classes  = \["behavioral"\]$/classes  = ["breaking"]/' "${REG}/unreleased/prior-row.toml"
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -ne 0 ] \
    && printf '%s' "$out" | grep -q "classes: 'breaking' is not one of" \
    && printf '%s' "$out" | grep -q 'prior-row.toml: classes:'; then
  ok "parse: 'breaking' fails, naming the file and key"
else bad "parse: 'breaking' fails, naming the file and key" "exit=$rc" "$out"; fi

# ── 5b. PARSE — retirement-train without shape-breaking fails (a routing is never a class) ──────
reset_fixtures
printf '%s\n' "crates/temper-mcp/src/lib.rs" > "$WIRE"
sed_inplace 's/^classes  = \["behavioral"\]$/classes  = ["additive", "retirement-train"]/' "${REG}/unreleased/prior-row.toml"
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -ne 0 ] \
    && printf '%s' "$out" | grep -q "a routing, never a class on its own"; then
  ok "parse: 'retirement-train' without 'shape-breaking' fails the co-occurrence rule"
else bad "parse: 'retirement-train' without 'shape-breaking' fails the co-occurrence rule" "exit=$rc" "$out"; fi

# ── 5c. PARSE — spec-correction with shape-breaking contradicts ──────────────────────────────────
reset_fixtures
printf '%s\n' "crates/temper-mcp/src/lib.rs" > "$WIRE"
sed_inplace 's/^classes  = \["behavioral"\]$/classes  = ["shape-breaking", "spec-correction"]/' "${REG}/unreleased/prior-row.toml"
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -ne 0 ] \
    && printf '%s' "$out" | grep -q "'spec-correction' and 'shape-breaking' contradict"; then
  ok "parse: 'spec-correction' with 'shape-breaking' fails the contradiction rule"
else bad "parse: 'spec-correction' with 'shape-breaking' fails the contradiction rule" "exit=$rc" "$out"; fi

# ── 6. PARSE — a nameless blocker (empty after blocked:) fails ──────────────────────────────────
reset_fixtures
printf '%s\n' "crates/temper-mcp/src/lib.rs" > "$WIRE"
sed_inplace 's/^status   = "open"$/status   = "blocked:"/' "${REG}/unreleased/prior-row.toml"
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q "names no release class"; then
  ok "parse: 'blocked:' with no class name fails"
else bad "parse: 'blocked:' with no class name fails" "exit=$rc" "$out"; fi

# ── 7. PARSE — a non-numeric, non-sentinel pr: fails ────────────────────────────────────────────
reset_fixtures
printf '%s\n' "crates/temper-mcp/src/lib.rs" > "$WIRE"
sed_inplace 's/^pr       = 867$/pr       = "867a"/' "${REG}/unreleased/prior-row.toml"
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

# ── 10. SHAPE — moved and shape-breaking declared without routing: FAIL (D-C2) ──────────────────
# A shape-breaking declaration below the era level is a refusal, not a schedule: the
# row must name its routing — convert to the deprecation path, or wait for the
# retirement train.
reset_fixtures
printf '%s\n' "openapi.json" > "$WIRE"
add_own_row self shape-breaking
out="$(run_check moved)"; rc=$?
if [ "$rc" -ne 0 ] \
    && printf '%s' "$out" | grep -q 'refusal, not a schedule' \
    && printf '%s' "$out" | grep -q 'deprecation path'; then
  ok "shape moved + unrouted shape-breaking declared: fails (D-C2's refusal)"
else bad "shape moved + unrouted shape-breaking declared: fails (D-C2's refusal)" "exit=$rc" "$out"; fi

# ── 10b. SHAPE — moved and shape-breaking routed to the retirement train: pass, NOTED ───────────
# The era release's own movement (D-C4): the PR moves M and cuts the next pin.
reset_fixtures
printf '%s\n' "openapi.json" > "$WIRE"
add_own_row self "shape-breaking, retirement-train"
out="$(run_check moved)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'retirement train'; then
  ok "shape moved + shape-breaking routed to the retirement train: passes (the era release's own movement)"
else bad "shape moved + shape-breaking routed to the retirement train: passes (the era release's own movement)" "exit=$rc" "$out"; fi

# ── 10c. SHAPE — moved and spec-correction declared: pass, NOTED ─────────────────────────────────
# The contract described what the server never did; the movement corrects it. Review-owned:
# CI passes it only with a note saying the claim is the row's to carry.
reset_fixtures
printf '%s\n' "openapi.json" > "$WIRE"
add_own_row self "behavioral, spec-correction"
out="$(run_check moved)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'declares spec-correction: PASS, NOTED'; then
  ok "shape moved + spec-correction declared: passes, noted as review-owned"
else bad "shape moved + spec-correction declared: passes, noted as review-owned" "exit=$rc" "$out"; fi

# ── 10d. SHAPE — spec-correction answers only a move; on growth the additive question stands ─────
reset_fixtures
printf '%s\n' "openapi.json" > "$WIRE"
add_own_row self "behavioral, spec-correction"
out="$(run_check grew)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'declare neither additive nor'; then
  ok "shape grew + spec-correction without additive: fails (growth is still additive's question)"
else bad "shape grew + spec-correction without additive: fails (growth is still additive's question)" "exit=$rc" "$out"; fi

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
    && printf '%s' "$out" | grep -q 'adds no register entry for this PR' \
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
  # -L is required since the comparator split (2026-09-16): wire-shape.jq includes
  # wire-shape-lib.jq, and jq does not resolve `include` relative to the program
  # file's own directory (measured, jq 1.7.1).
  jq -n -r -f "$SHAPE_JQ" -L "$SCRIPT_DIR" --slurpfile base "$1" --slurpfile head "$2" 2>&1
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

# 15e — required GROWN inside allOf: moved. Required SHRUNK with the property retained:
# ALSO moved — CORRECTED 2026-09-16 (the #906 lesson). The old probe pinned shrinkage as
# "the tolerant direction"; that reading is wrong for the omit-class: a field leaving
# `required` while staying in `properties` is a field the server may now OMIT (Option +
# skip_serializing_if), and a client built against the old contract types it required —
# Scoring.score in PR #906 is the founding case. Under §4 ("tolerant in both skew
# directions") any required change is breaking; the comparator's arm is now symmetric.
jq -S '.components.schemas.Example.allOf[0].required = ["peer_table","extra"]' "$BASE_OAS" > "$HEAD_OAS"
v="$(derive_with "$BASE_OAS" "$HEAD_OAS")"
if [ "$v" = "moved" ]; then
  ok "derivation: required grown inside allOf computes as moved"
else bad "derivation: required grown inside allOf computes as moved" "verdict=$v"; fi
jq -S '.components.schemas.Example.allOf[0].required = []' "$BASE_OAS" > "$HEAD_OAS"
v="$(derive_with "$BASE_OAS" "$HEAD_OAS")"
if [ "$v" = "moved" ]; then
  ok "derivation: required shrunk (the omit-class, the #906 lesson) computes as moved"
else bad "derivation: required shrunk (the omit-class, the #906 lesson) computes as moved" "verdict=$v"; fi

# 15f — required GAINED from absent (base node has no `required` at all): moved.
# The pin-gate commit's independent review caught the base-keys-only loop missing
# this face; the comparator now checks head-side required-growth explicitly.
jq -S 'del(.components.schemas.Example.allOf[0].required)' "$BASE_OAS" > "${WORK}/base_noreq.json"
jq -S '.components.schemas.Example.allOf[0].required = ["peer_table"]' "$BASE_OAS" > "${WORK}/head_gain.json"
v="$(derive_with "${WORK}/base_noreq.json" "${WORK}/head_gain.json")"
if [ "$v" = "moved" ]; then
  ok "derivation: required gained from absent computes as moved"
else bad "derivation: required gained from absent computes as moved" "verdict=$v"; fi

# 15g–15m — the SIDE-AWARE tolerance arm (2026-09-23, beat G3a's B1): two length-change
# classes are provably tolerant in both skew directions (§4's own definition) and were
# fail-closed before: a born OPTIONAL parameter on a surviving operation (old clients
# never send it; a new client's param against an old server is answered by param
# ignorance), and input-side string-enum member growth. Each tolerance probe has its
# boundary probe — the same shape one nudge past the tolerance must stay moved.
# The fixture: a surviving operation with one required path param; one component schema
# referenced ONLY from the request body; one referenced from a response; one referenced
# from both (the ambiguous side fails closed).
cat > "${WORK}/side_base.json" <<'JSON'
{"paths":{"/api/things/{id}":{"get":{"parameters":[{"in":"path","name":"id","required":true,"schema":{"type":"string"}}],"responses":{"200":{"description":"ok"}}},
           "post":{"requestBody":{"content":{"application/json":{"schema":{"$ref":"#/components/schemas/InOnly"}}}},"responses":{"200":{"description":"ok"}}},
           "delete":{"responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"$ref":"#/components/schemas/OutSide"}}}}}}},
 "/api/things":{"get":{"responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"$ref":"#/components/schemas/BothSides"}}}}}}}},
 "components":{"schemas":{
   "InOnly":{"type":"object","properties":{"kind":{"type":"string","enum":["alpha","beta"]}}},
   "OutSide":{"type":"object","properties":{"kind":{"type":"string","enum":["alpha","beta"]}}},
   "BothSides":{"type":"object","properties":{"reply":{"$ref":"#/components/schemas/OutSide"}}}}}}
JSON

# 15g — born OPTIONAL query parameter on a surviving operation: grew.
jq -S '.paths["/api/things/{id}"].get.parameters += [{"in":"query","name":"sections","required":false,"schema":{"type":"string"}}]' "${WORK}/side_base.json" > "${WORK}/side_head.json"
v="$(derive_with "${WORK}/side_base.json" "${WORK}/side_head.json")"
if [ "$v" = "grew" ]; then
  ok "derivation: a born optional parameter on a surviving operation computes as grew"
else bad "derivation: a born optional parameter on a surviving operation computes as grew" "verdict=$v"; fi

# 15h — born REQUIRED parameter on a surviving operation: moved (old clients never send
# what the new server refuses).
jq -S '.paths["/api/things/{id}"].get.parameters += [{"in":"query","name":"mode","required":true,"schema":{"type":"string"}}]' "${WORK}/side_base.json" > "${WORK}/side_head.json"
v="$(derive_with "${WORK}/side_base.json" "${WORK}/side_head.json")"
if [ "$v" = "moved" ]; then
  ok "derivation: a born required parameter on a surviving operation computes as moved"
else bad "derivation: a born required parameter on a surviving operation computes as moved" "verdict=$v"; fi

# 15i — a REMOVED parameter: moved.
jq -S '.paths["/api/things/{id}"].get.parameters = [.paths["/api/things/{id}"].get.parameters[0]] | .paths["/api/things/{id}"].get.parameters += [{"in":"query","name":"sections","required":false,"schema":{"type":"string"}}]' "${WORK}/side_base.json" \
  | jq -S 'del(.paths["/api/things/{id}"].get.parameters[0])' > "${WORK}/side_head.json"
v="$(derive_with "${WORK}/side_base.json" "${WORK}/side_head.json")"
if [ "$v" = "moved" ]; then
  ok "derivation: a removed parameter computes as moved"
else bad "derivation: a removed parameter computes as moved" "verdict=$v"; fi

# 15j — input-only component schema: string-enum member GROWTH (strict superset): grew.
jq -S '.components.schemas.InOnly.properties.kind.enum += ["gamma"]' "${WORK}/side_base.json" > "${WORK}/side_head.json"
v="$(derive_with "${WORK}/side_base.json" "${WORK}/side_head.json")"
if [ "$v" = "grew" ]; then
  ok "derivation: input-only enum member growth computes as grew"
else bad "derivation: input-only enum member growth computes as grew" "verdict=$v"; fi

# 15k — output-reachable component schema: the same growth stays moved (a server that
# may now EMIT a member can strand an old client's exhaustive parse — the compat
# amendment's own worry).
jq -S '.components.schemas.OutSide.properties.kind.enum += ["gamma"]' "${WORK}/side_base.json" > "${WORK}/side_head.json"
v="$(derive_with "${WORK}/side_base.json" "${WORK}/side_head.json")"
if [ "$v" = "moved" ]; then
  ok "derivation: output-side enum member growth computes as moved"
else bad "derivation: output-side enum member growth computes as moved" "verdict=$v"; fi

# 15l — BOTH-sides-reachable component schema (a response echoes a request shape): the
# ambiguity fails closed.
jq -S '.components.schemas.BothSides.properties = {"echo":{"$ref":"#/components/schemas/InOnly"},"reply":{"$ref":"#/components/schemas/OutSide"}}' "${WORK}/side_base.json" > "${WORK}/side_head.json"
v="$(derive_with "${WORK}/side_base.json" "${WORK}/side_head.json")"
if [ "$v" = "grew" ]; then
  ok "derivation: both-sides fixture baseline stays grew (the 15k–15m probe substrate)"
else bad "derivation: both-sides fixture baseline stays grew (the 15k–15m probe substrate)" "verdict=$v"; fi
jq -S '.components.schemas.InOnly.properties.kind.enum += ["gamma"]' "${WORK}/side_head.json" > "${WORK}/side_head2.json"
v="$(derive_with "${WORK}/side_head.json" "${WORK}/side_head2.json")"
if [ "$v" = "moved" ]; then
  ok "derivation: enum growth under a both-sides-reachable schema computes as moved"
else bad "derivation: enum growth under a both-sides-reachable schema computes as moved" "verdict=$v"; fi

# 15m — input-only enum SHRINK: moved (a member only the old contract accepted leaving
# is a removal, not growth).
jq -S '.components.schemas.InOnly.properties.kind.enum = ["alpha"]' "${WORK}/side_base.json" > "${WORK}/side_head.json"
v="$(derive_with "${WORK}/side_base.json" "${WORK}/side_head.json")"
if [ "$v" = "moved" ]; then
  ok "derivation: input-only enum member shrink computes as moved"
else bad "derivation: input-only enum member shrink computes as moved" "verdict=$v"; fi

# 15n — the D1 shape (found in review): a RENAME whose old member is a SUBSTRING of a
# new one, the rest a true superset. jq `contains` is substring-containment on string
# arrays, so `pending` → `pending_recheck` reads as superset growth and certifies a
# change that sends every old client's still-valid `pending` request to a 4xx. The
# tolerance is exact-set membership: this must be moved. BITE-PROVEN: under the
# unfixed `contains` arm this exact fixture computes `grew` — the probe reddens the
# regression it exists for.
jq -S '.components.schemas.InOnly.properties.kind.enum = ["alpha","pending_recheck","failed"]' "${WORK}/side_base.json" > "${WORK}/side_head.json"
# side_base's kind enum must carry the substring member for the shape to exist; the
# mutation writes a sibling file — redirecting onto the fixture itself would truncate
# it before jq reads it.
jq -S '.components.schemas.InOnly.properties.kind.enum = ["alpha","pending"]' "${WORK}/side_base.json" > "${WORK}/side_base_15n.json"
v="$(derive_with "${WORK}/side_base_15n.json" "${WORK}/side_head.json")"
if [ "$v" = "moved" ]; then
  ok "derivation: an enum rename whose old member is a substring of the new computes as moved (D1)"
else bad "derivation: an enum rename whose old member is a substring of the new computes as moved (D1)" "verdict=$v"; fi

# 15o — the D2 shape (found in review): the same diff makes a schema response-reachable
# AND grows its input enum. Tolerance is input-only in BOTH contracts, so the growth
# rides a diff that opens an output channel: moved, fail-closed.
jq -S '.components.schemas.BothSides.properties = {"echo":{"$ref":"#/components/schemas/InOnly"},"reply":{"$ref":"#/components/schemas/OutSide"}} | .components.schemas.InOnly.properties.kind.enum += ["gamma"]' "${WORK}/side_base.json" > "${WORK}/side_head.json"
v="$(derive_with "${WORK}/side_base.json" "${WORK}/side_head.json")"
if [ "$v" = "moved" ]; then
  ok "derivation: enum growth in the same diff that opens a response ref computes as moved (D2)"
else bad "derivation: enum growth in the same diff that opens a response ref computes as moved (D2)" "verdict=$v"; fi

# 15p–15u — the OPEN-VOCABULARY arm (2026-10-06): a `oneOf` of string arms with exactly
# one unconstrained catch-all (utoipa's rendering of a serde enum with an untagged
# `Other(String)`, e.g. `RefusalReason`) accepts every string in both contracts, so a
# new `enum` arm inserted before the catch-all is growth even in OUTPUT position. The
# fixture is response-reachable, so the input-side tolerance cannot be what passes it.
cat > "${WORK}/vocab_base.json" <<'JSON'
{"paths":{"/api/query":{"post":{"responses":{"400":{"description":"refused","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Reason"}}}}}}}},
 "components":{"schemas":{
   "Reason":{"oneOf":[{"type":"string","description":"a","enum":["alpha"]},{"type":"string","description":"b","enum":["beta"]},{"type":"string","description":"anything newer"}]},
   "Closed":{"oneOf":[{"type":"string","enum":["alpha"]},{"type":"string","enum":["beta"]}]}}}}
JSON
vocab_probe() { # $1 = jq mutation, $2 = expected verdict, $3 = label
  jq -S "$1" "${WORK}/vocab_base.json" > "${WORK}/vocab_head.json"
  v="$(derive_with "${WORK}/vocab_base.json" "${WORK}/vocab_head.json")"
  if [ "$v" = "$2" ]; then ok "derivation: $3"; else bad "derivation: $3" "verdict=$v (want $2)"; fi
}
# 15p — an enum arm inserted before the catch-all: grew. BITE-PROVEN: without the arm
# in wire-shape-lib.jq this computes `moved` (the catch-all's index shifts).
vocab_probe '.components.schemas.Reason.oneOf |= .[0:2] + [{"type":"string","description":"new","enum":["gamma"]}] + .[2:]' \
  grew "an open vocabulary gaining an enum arm in output position computes as grew"
# 15q — the same insertion with a prose edit on a surviving arm: still grew.
vocab_probe '.components.schemas.Reason.oneOf |= ([.[0] | .description = "a, reworded"] + .[1:2] + [{"type":"string","enum":["gamma"]}] + .[2:])' \
  grew "open-vocabulary growth beside a prose edit computes as grew"
# 15r — a CLOSED oneOf (no catch-all) gaining an arm: moved — an old exhaustive parse breaks.
vocab_probe '.components.schemas.Closed.oneOf += [{"type":"string","enum":["gamma"]}]' \
  moved "a closed oneOf gaining an enum arm computes as moved"
# 15s — an open vocabulary LOSING an arm: moved.
vocab_probe '.components.schemas.Reason.oneOf |= ([.[0]] + .[2:])' \
  moved "an open vocabulary losing an enum arm computes as moved"
# 15t — growth while the catch-all gains a constraint: moved — the accepted set narrows.
vocab_probe '.components.schemas.Reason.oneOf |= (.[0:2] + [{"type":"string","enum":["gamma"]}] + [.[2] | .pattern = "^[a-z_]+$"])' \
  moved "open-vocabulary growth with a constrained catch-all computes as moved"
# 15u — growth while a surviving arm's enum is edited (a rename): moved.
vocab_probe '.components.schemas.Reason.oneOf |= ([.[0] | .enum = ["alpha_renamed"]] + .[1:2] + [{"type":"string","enum":["gamma"]}] + .[2:])' \
  moved "open-vocabulary growth beside a renamed member computes as moved"

# 15v–15z — the UNORDERED-oneOf arm (2026-10-17): a `oneOf` is a set, so its arm order is not
# shape. utoipa 6 emits a nullable reference as `[$ref, {type: null}]` where 5 emitted
# `[{type: null}, $ref]`. Response-reachable, so the input-side tolerance cannot pass it.
cat > "${WORK}/set_base.json" <<'JSON'
{"paths":{"/api/thing":{"get":{"responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Holder"}}}}}}}},
 "components":{"schemas":{
   "Holder":{"type":"object","properties":{
     "one":{"oneOf":[{"type":"null"},{"$ref":"#/components/schemas/Inner","description":"the inner one"}]},
     "any":{"anyOf":[{"type":"null"},{"$ref":"#/components/schemas/Inner"}]}}},
   "Inner":{"type":"object","properties":{"id":{"type":"string"}},"required":["id"]},
   "Other":{"type":"object","properties":{"id":{"type":"string"}},"required":["id"]}}}}
JSON
set_probe() { # $1 = jq mutation, $2 = expected verdict, $3 = label
  jq -S "$1" "${WORK}/set_base.json" > "${WORK}/set_head.json"
  v="$(derive_with "${WORK}/set_base.json" "${WORK}/set_head.json")"
  if [ "$v" = "$2" ]; then ok "derivation: $3"; else bad "derivation: $3" "verdict=$v (want $2)"; fi
}
# 15v — the utoipa 6 reorder itself: grew. BITE-PROVEN: without the arm in wire-shape-lib.jq this
# computes `moved` (index-pairing compares `null` against `$ref`).
set_probe '.components.schemas.Holder.properties.one.oneOf |= reverse' \
  grew "a oneOf whose arms only reorder computes as grew"
# 15w — reorder plus a prose edit on an arm: grew — the pairing ignores prose.
set_probe '.components.schemas.Holder.properties.one.oneOf |= (reverse | .[0].description = "reworded")' \
  grew "a oneOf reorder beside an arm prose edit computes as grew"
# 15x — reorder while an arm retargets its $ref: moved — the set changed, not just its order.
set_probe '.components.schemas.Holder.properties.one.oneOf |= (reverse | .[0]["$ref"] = "#/components/schemas/Other")' \
  moved "a oneOf reorder that also retargets an arm computes as moved"
# 15y — a oneOf losing its null arm: moved — the count changed, so the set arm never runs.
set_probe '.components.schemas.Holder.properties.one.oneOf |= [.[1]]' \
  moved "a oneOf losing an arm computes as moved"
# 15z — an anyOf reorder: moved — anyOf stays index-paired (first-match deserializers).
set_probe '.components.schemas.Holder.properties.any.anyOf |= reverse' \
  moved "an anyOf whose arms only reorder still computes as moved"

# ── 16. DEPENDENCY-ONLY — manifest/lockfile bumps under wire trees owe no row ───────────────────
# The fixture seam: paths named by --dependency-only are set aside; a row is owed only for the
# rest. A mixed PR (a bump plus a code edit) still fails presence, naming only the code file.
DEPS="${WORK}/deps.txt"
reset_fixtures
printf '%s\n' "clients/temper-ts/package.json" "clients/temper-ts/package-lock.json" > "$WIRE"
cp "$WIRE" "$DEPS"
out="$(run_check unchanged --dependency-only "$DEPS")"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'every wire-tree change is dependency-only' \
    && printf '%s' "$out" | grep -q 'behavioral class'; then
  ok "dependency-only seam: all wire paths set aside passes, naming the behavioral bar"
else bad "dependency-only seam: all wire paths set aside passes, naming the behavioral bar" "exit=$rc" "$out"; fi

reset_fixtures
printf '%s\n' "clients/temper-ts/package.json" "clients/temper-ts/src/index.ts" > "$WIRE"
printf '%s\n' "clients/temper-ts/package.json" > "$DEPS"
out="$(run_check unchanged --dependency-only "$DEPS")"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'adds no register entry for this PR' \
    && printf '%s' "$out" | grep -q 'clients/ (1 file(s))'; then
  ok "dependency-only seam: a bump beside a code edit still owes the row for the code"
else bad "dependency-only seam: a bump beside a code edit still owes the row for the code" "exit=$rc" "$out"; fi

# The classifier itself, in git mode against a throwaway repository: no seams, a real base.
# Each case commits a base manifest, edits it, and asks the real script.
classify_probe() { # $1 = path, $2 = base content, $3 = head content ("" = file new in head), $4 = expect pass|fail, $5 = label
  local repo="${WORK}/repo-$RANDOM$RANDOM"
  mkdir -p "$repo" && git -C "$repo" init -q && git -C "$repo" config user.email t@t && git -C "$repo" config user.name t
  mkdir -p "$repo/$(dirname "$1")"
  if [ -n "$2" ]; then printf '%s\n' "$2" > "$repo/$1"; else : > "$repo/.keep"; fi
  git -C "$repo" add -A && git -C "$repo" commit -qm base
  printf '%s\n' "$3" > "$repo/$1"
  git -C "$repo" add -A
  local out rc
  out="$(cd "$repo" && bash "$CHECK" --base HEAD 2>&1)"; rc=$?
  if { [ "$4" = pass ] && [ "$rc" -eq 0 ]; } || { [ "$4" = fail ] && [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'adds no register entry'; }; then
    ok "$5"
  else bad "$5" "exit=$rc" "$out"; fi
}

PJ_BASE='{"name":"@temperkb/client","version":"0.4.0","dependencies":{"a":"^1.0.0"},"devDependencies":{"vitest":"^3.2.7"},"peerDependencies":{"p":"^1"}}'
classify_probe clients/temper-ts/package.json "$PJ_BASE" \
  '{"name":"@temperkb/client","version":"0.4.0","dependencies":{"a":"^1.2.0"},"devDependencies":{"vitest":"^5.0.3"},"peerDependencies":{"p":"^1"}}' \
  pass "classifier: package.json moving only dependencies/devDependencies is dependency-only"
classify_probe clients/temper-ts/package.json "$PJ_BASE" \
  '{"name":"@temperkb/client","version":"0.5.0","dependencies":{"a":"^1.0.0"},"devDependencies":{"vitest":"^3.2.7"},"peerDependencies":{"p":"^1"}}' \
  fail "classifier: a package.json version change stays on the wire"
classify_probe clients/temper-ts/package.json "$PJ_BASE" \
  '{"name":"@temperkb/client","version":"0.4.0","dependencies":{"a":"^1.0.0"},"devDependencies":{"vitest":"^3.2.7"},"peerDependencies":{"p":"^2"}}' \
  fail "classifier: a peerDependencies change stays on the wire (consumer-facing)"
classify_probe clients/temper-ts/package.json "" "$PJ_BASE" \
  fail "classifier: a package.json new in this diff stays on the wire (fail closed)"
classify_probe clients/temper-ts/package.json "$PJ_BASE" '{not json' \
  fail "classifier: an unparseable package.json stays on the wire (fail closed)"
classify_probe clients/temper-ts/package-lock.json '{"lockfileVersion":3}' '{"lockfileVersion":3,"x":1}' \
  pass "classifier: a lockfile is dependency-only"
classify_probe clients/temper-rb/mise.lock "" 'lockfile_version = 3' \
  pass "classifier: a mise.lock is dependency-only, even new in this diff (dev tooling, never published)"

CT_BASE='[package]
name = "temper-api"
version = "0.4.0"

[features]
default = []

[dependencies]
jsonwebtoken = { version = "9.3", features = [
    "rust_crypto",
] }
serde = "1"

[target.'"'"'cfg(unix)'"'"'.dependencies]
libc = "0.2"

[dev-dependencies]
rand = "0.8"'
CT_DEPS="$(printf '%s' "$CT_BASE" | sed 's/"9.3"/"11.1"/; s/rust_crypto/aws_lc_rs/; s/"0.2"/"0.3"/; s/"0.8"/"0.10"/')"
classify_probe crates/temper-api/Cargo.toml "$CT_BASE" "$CT_DEPS" \
  pass "classifier: Cargo.toml moving only [*dependencies*] tables (incl. target-cfg, multi-line) is dependency-only"
classify_probe crates/temper-api/Cargo.toml "$CT_BASE" "$(printf '%s' "$CT_BASE" | sed 's/^default = \[\]/default = ["x"]/')" \
  fail "classifier: a Cargo.toml [features] change stays on the wire"
classify_probe crates/temper-api/Cargo.toml "$CT_BASE" "$(printf '%s\n\n[dependencies.extra]\nversion = "1"\n\n[lib]\npath = "x.rs"' "$CT_BASE")" \
  fail "classifier: a non-dependency table after a [dependencies.<name>] table stays on the wire"
classify_probe crates/temper-api/src/lib.rs 'fn a() {}' 'fn b() {}' \
  fail "classifier: source files are never dependency-only"

# ── 18. W9 — the old single-file register never quietly comes back ──────────────────────────────
reset_fixtures
printf '%s\n' "openapi.json" > "$WIRE"
add_own_row self additive
printf '# Release-verdict register\n' > "$LEGACY"
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -ne 0 ] \
    && printf '%s' "$out" | grep -q 'RELEASE_REGISTER.md exists' \
    && printf '%s' "$out" | grep -q 'release-register/unreleased/<id>.toml'; then
  ok "W9: RELEASE_REGISTER.md existing fails, naming the new location — even beside a valid entry"
else bad "W9: RELEASE_REGISTER.md existing fails, naming the new location" "exit=$rc" "$out"; fi

# ── 18a. ERASURE POLICY — policy/erasure/ is declared on its own surface, and is not wire ───────
# Check 4. The policy is not wire shape: no wire path is touched, no class is demanded, and the
# only thing owed is an added entry that lists `erasure-policy` in surfaces.
reset_fixtures
printf '%s\n' "policy/erasure/estate.toml" > "$WIRE"
add_own_row self additive "internal"   # an own row, but it does not list erasure-policy
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -ne 0 ] \
    && printf '%s' "$out" | grep -q 'lists' \
    && printf '%s' "$out" | grep -q '"erasure-policy" in surfaces' \
    && printf '%s' "$out" | grep -q 'policy/erasure/estate.toml'; then
  ok "erasure policy: a policy edit without the erasure-policy surface fails, naming the files"
else bad "erasure policy: a policy edit without the erasure-policy surface fails, naming the files" "exit=$rc" "$out"; fi

reset_fixtures
printf '%s\n' "policy/erasure/estate.toml" > "$WIRE"
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q '"erasure-policy" in surfaces'; then
  ok "erasure policy: a policy edit with no added entry at all fails"
else bad "erasure policy: a policy edit with no added entry at all fails" "exit=$rc" "$out"; fi

reset_fixtures
printf '%s\n' "policy/erasure/estate.toml" "policy/erasure/manifest.toml" > "$WIRE"
add_own_row self additive "erasure-policy, internal"
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'wire-class-crosscheck: PASS' \
    && ! printf '%s' "$out" | grep -q 'wire paths ──'; then
  ok "erasure policy: an added entry listing erasure-policy passes, and the policy is not a wire path"
else bad "erasure policy: an added entry listing erasure-policy passes, and the policy is not a wire path" "exit=$rc" "$out"; fi

# ── 19. W2 — in git mode, only an ADDED entry is a declaration ──────────────────────────────────
# The fixture seams above bypass git; this arm runs the gate's own derivation. A sandbox repo
# carries the gate, the reader and a register whose base holds another PR's entry; each branch
# touches a wire path and does one thing to the register.
GITBOX="${WORK}/gitbox"
mkdir -p "${GITBOX}/.github/scripts" "${GITBOX}/tools" "${GITBOX}/clients"
cp "${REPO}/.github/scripts/wire-class-crosscheck.sh" "${REPO}/.github/scripts/"wire-shape*.jq "${REPO}/.github/scripts/wire-resolve.py" "${GITBOX}/.github/scripts/"
cp -R "${REPO}/tools/manifest" "${GITBOX}/tools/manifest"
reset_register
cp -R "$REG" "${GITBOX}/release-register"
echo "v1" > "${GITBOX}/clients/thing.txt"
gb() { git -C "$GITBOX" -c user.name=t -c user.email=t@t -c commit.gpgsign=false "$@"; }
gb init -q -b main
gb add -A
gb commit -q -m base
BASE_SHA="$(gb rev-parse HEAD)"
git_mode_check() { (cd "$GITBOX" && bash .github/scripts/wire-class-crosscheck.sh --base "$BASE_SHA" --shape-verdict unchanged 2>&1); }

gb checkout -q -b edits-another main
echo "v2" > "${GITBOX}/clients/thing.txt"
sed_inplace 's/^status   = "open"$/status   = "satisfied"/' "${GITBOX}/release-register/unreleased/prior-row.toml"
sed_inplace 's/^pr       = 867$/pr       = "self"/' "${GITBOX}/release-register/unreleased/prior-row.toml"
gb commit -q -am "edit someone else's entry into pr = self"
out="$(git_mode_check)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'adds no register entry for this PR'; then
  ok "W2: editing another PR's entry (even into pr = self) is not a declaration"
else bad "W2: editing another PR's entry is not a declaration" "exit=$rc" "$out"; fi

gb checkout -q -b renames-another main
echo "v2" > "${GITBOX}/clients/thing.txt"
# Renamed AND re-pointed at this PR: the masquerade `-M` exists to stop. Without rename
# detection the move reads as an added file whose pr is "self", and it would count.
gb mv release-register/unreleased/prior-row.toml release-register/unreleased/mine-now.toml
sed_inplace 's/^pr       = 867$/pr       = "self"/' "${GITBOX}/release-register/unreleased/mine-now.toml"
gb commit -q -am "rename someone else's entry and re-point it at this PR"
out="$(git_mode_check)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'adds no register entry for this PR'; then
  ok "W2: renaming another PR's entry (and re-pointing it at this PR) is not a declaration"
else bad "W2: renaming another PR's entry is not a declaration" "exit=$rc" "$out"; fi

gb checkout -q -b adds-own main
echo "v2" > "${GITBOX}/clients/thing.txt"
ADDED_SAVE="$ADDED"; ADDED="${GITBOX}/release-register/unreleased"; OWN=0
add_own_row self behavioral
ADDED="$ADDED_SAVE"
gb add -A
gb commit -q -m "add this PR's own entry"
out="$(git_mode_check)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'found — declared classes: behavioral'; then
  ok "W2: adding this PR's own entry declares it"
else bad "W2: adding this PR's own entry declares it" "exit=$rc" "$out"; fi

gb checkout -q -b adds-malformed main
echo "v2" > "${GITBOX}/clients/thing.txt"
printf 'title = "x"\n' > "${GITBOX}/release-register/unreleased/half-written.toml"
gb add -A
gb commit -q -m "add a malformed entry"
out="$(git_mode_check)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'half-written.toml: pr: missing'; then
  ok "W2: a malformed added entry fails as itself, naming the file and key"
else bad "W2: a malformed added entry fails as itself" "exit=$rc" "$out"; fi

# ── 20. COMPAT-1 — the verdict is release-relative, judged per resolved operation ───────────────
# The conventions doctrine (compat.md) and its bite list. A client of the last release can only
# observe shape the release had: shape born in the window moves freely, released shape is judged
# against its released form wherever the change is made. broke_node returns a verdict per node,
# so the gate runs the doctrine's per-entry-point fallback: an operation is charged when
# release → head breaks it AND this PR changes its resolved form. Each probe feeds three
# contracts through the --release / --base-contract / --head-contract seams, with a row
# declaring `additive` — so a FAIL is the gate charging a break.
CDIR="${WORK}/compat"
mkdir -p "$CDIR"
# The release: GET/DELETE /items/{id}, POST /items, a cycle, a security scheme, an orphan.
cat > "${CDIR}/release.json" <<'JSON'
{"openapi":"3.1.0","info":{"title":"t","version":"1.0.0"},
 "security":[{"bearer":[]}],
 "paths":{
  "/items/{id}":{
   "get":{"operationId":"get_item",
     "parameters":[{"in":"path","name":"id","required":true,"schema":{"type":"string"}}],
     "responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Item"}}}}}},
   "delete":{"operationId":"delete_item",
     "parameters":[{"in":"path","name":"id","required":true,"schema":{"type":"string"}}],
     "responses":{"204":{"description":"gone"}}}},
  "/items":{
   "post":{"operationId":"create_item",
     "requestBody":{"content":{"application/json":{"schema":{"$ref":"#/components/schemas/ItemCreate"}}}},
     "responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Item"}}}}}}}},
 "components":{
  "securitySchemes":{"bearer":{"type":"http","scheme":"bearer"}},
  "schemas":{
   "Item":{"type":"object","required":["id"],"properties":{
     "id":{"type":"string"},"name":{"type":"string"},
     "parent":{"$ref":"#/components/schemas/Node"}}},
   "Node":{"type":"object","properties":{"child":{"$ref":"#/components/schemas/Node"}}},
   "ItemCreate":{"type":"object","properties":{"name":{"type":"string","maxLength":10}}},
   "Orphan":{"type":"object","properties":{"a":{"type":"string"}}}}}}
JSON

# A derived contract: $1 = from, $2 = out, $3 = jq edit.
derive_contract() { jq "$3" "${CDIR}/$1.json" > "${CDIR}/$2.json" || echo "fixture edit failed: $2"; }
compat_check() { # $1 = release (a fixture name, or `none`), $2 = base, $3 = head, $4 = classes
  reset_fixtures
  printf '%s\n' "openapi.json" > "$WIRE"
  add_own_row self "${4:-additive}"
  local rel="$1"; [ "$rel" = "none" ] || rel="${CDIR}/$1.json"
  bash "$CHECK" --wire-touched "$WIRE" --added-entries "$ADDED" --register-dir "$REG" \
      --legacy-register "$LEGACY" --release "$rel" \
      --base-contract "${CDIR}/$2.json" --head-contract "${CDIR}/$3.json" 2>&1
}
expect_pass() { # $1 = label, $2 = output, $3 = rc
  if [ "$3" -eq 0 ] && printf '%s' "$2" | grep -q 'release-relative verdict'; then ok "$1"
  else bad "$1" "exit=$3" "$2"; fi
}
expect_charged() { # $1 = label, $2 = output, $3 = rc, $4 = the key the gate must name
  if [ "$3" -ne 0 ] && printf '%s' "$2" | grep -q 'charged — release → head breaks it' \
      && printf '%s' "$2" | grep -qF "    $4"; then ok "$1"
  else bad "$1" "exit=$3" "$2"; fi
}

# Bite 1 (temper#1388's shape): a schema born in the window, reached only through a new optional
# property of a released operation, gains a required member. Base → head reads that as a break
# (required growth); against the release the whole subtree is growth.
derive_contract release born '.components.schemas.Item.properties.extra = {"$ref":"#/components/schemas/Extra"}
  | .components.schemas.Extra = {"type":"object","properties":{"a":{"type":"string"}}}'
derive_contract born born-moved '.components.schemas.Extra.required = ["a"]'
out="$(compat_check release born born-moved)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'release-relative verdict' \
    && printf '%s' "$out" | grep -q 'reclassified as growth' \
    && printf '%s' "$out" | grep -qF '    op GET /items/{}'; then
  ok "COMPAT-1 bite 1: a moved window-born node passes as additive, its operations named as reclassified"
else bad "COMPAT-1 bite 1: a moved window-born node passes as additive" "exit=$rc" "$out"; fi

# Bite 3: the release reference removed (the fix reverted) — bite 1 fails, base-only.
out="$(compat_check none born born-moved)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'base-only verdict'; then
  ok "COMPAT-1 bite 3: with no release, the same diff fails base-only"
else bad "COMPAT-1 bite 3: with no release, the same diff fails base-only" "exit=$rc" "$out"; fi

# Bite 2: a released node moved fails without shape-breaking — and passes, noted, with the
# retirement-train routing (the declared-break path is unchanged).
derive_contract release released-moved '.components.schemas.Item.properties.name.type = "integer"'
out="$(compat_check release release released-moved)"; rc=$?
expect_charged "COMPAT-1 bite 2: a moved released node fails without shape-breaking" "$out" "$rc" "op GET /items/{}"
out="$(compat_check release release released-moved "shape-breaking, retirement-train")"; rc=$?
if [ "$rc" -eq 0 ]; then ok "COMPAT-1 bite 2: the same break declared and routed passes"
else bad "COMPAT-1 bite 2: the same break declared and routed passes" "exit=$rc" "$out"; fi

# Bite 4: a re-birth judged against its released form. The window removed Item.name; this PR
# restores it as an integer. Base → head is a new optional property (growth); release → head is
# a type change.
derive_contract release name-gone 'del(.components.schemas.Item.properties.name)'
derive_contract name-gone name-reborn-differs '.components.schemas.Item.properties.name = {"type":"integer"}'
derive_contract name-gone name-reborn-same '.components.schemas.Item.properties.name = {"type":"string"}'
out="$(compat_check release name-gone name-reborn-differs)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'reclassified as a break' \
    && printf '%s' "$out" | grep -qF '    op GET /items/{}'; then
  ok "COMPAT-1 bite 4: a differing re-birth of a property fails, named as reclassified"
else bad "COMPAT-1 bite 4: a differing re-birth of a property fails" "exit=$rc" "$out"; fi
out="$(compat_check release name-gone name-reborn-same)"; rc=$?
expect_pass "COMPAT-1 bite 4: a property re-born unchanged passes" "$out" "$rc"
# ... and once as an operation: DELETE removed in the window, restored with a required query.
derive_contract release delete-gone 'del(.paths."/items/{id}".delete)'
derive_contract release delete-reborn '.paths."/items/{id}".delete.parameters += [{"in":"query","name":"force","required":true,"schema":{"type":"boolean"}}]'
out="$(compat_check release delete-gone delete-reborn)"; rc=$?
expect_charged "COMPAT-1 bite 4: a differing re-birth of an operation fails" "$out" "$rc" "op DELETE /items/{}"
out="$(compat_check release delete-gone release)"; rc=$?
expect_pass "COMPAT-1 bite 4: an operation re-born unchanged passes" "$out" "$rc"

# Bite 5 / the non-schema referrer: a window-born PARAMETER component on a released operation
# becomes required. The path still holds the same `$ref` string, and the old comparator never
# compared components.parameters at all — the change is visible only once the operation is
# resolved.
derive_contract release param-born '.paths."/items/{id}".get.parameters += [{"$ref":"#/components/parameters/Verbose"}]
  | .components.parameters = {"Verbose":{"in":"query","name":"verbose","required":false,"schema":{"type":"boolean"}}}'
derive_contract param-born param-required '.components.parameters.Verbose.required = true'
out="$(compat_check release param-born param-required)"; rc=$?
expect_charged "COMPAT-1 bite 5: a window-born parameter component that becomes required fails (a non-schema referrer)" "$out" "$rc" "op GET /items/{}"
# ... and a released RESPONSE component, changed behind an unchanged $ref string.
derive_contract release resp-ref '.paths."/items/{id}".delete.responses."404" = {"$ref":"#/components/responses/Missing"}
  | .components.responses = {"Missing":{"description":"none","content":{"application/json":{"schema":{"type":"string"}}}}}'
derive_contract resp-ref resp-ref-moved '.components.responses.Missing.content."application/json".schema.type = "integer"'
out="$(compat_check resp-ref resp-ref resp-ref-moved)"; rc=$?
expect_charged "COMPAT-1: a released response component changed behind its \$ref fails" "$out" "$rc" "op DELETE /items/{}"

# A gained $ref at a released location: Item.name becomes a reference to a schema born in this
# PR. What the client sees is the inlined target, judged against the released property.
derive_contract release ref-swap-break '.components.schemas.Item.properties.name = {"$ref":"#/components/schemas/Name"}
  | .components.schemas.Name = {"type":"integer"}'
derive_contract release ref-swap-same '.components.schemas.Item.properties.name = {"$ref":"#/components/schemas/Name"}
  | .components.schemas.Name = {"type":"string"}'
out="$(compat_check release release ref-swap-break)"; rc=$?
expect_charged "COMPAT-1: a gained \$ref at a released location, to a differing born schema, fails" "$out" "$rc" "op POST /items"
out="$(compat_check release release ref-swap-same)"; rc=$?
expect_pass "COMPAT-1: a gained \$ref whose target equals the released form passes" "$out" "$rc"

# Bite 7: a renamed path-template parameter is the released operation — no removal, no birth.
derive_contract release renamed '.paths."/items/{item_id}" = (.paths."/items/{id}"
    | (.get.parameters, .delete.parameters) |= map(.name = "item_id"))
  | del(.paths."/items/{id}")'
out="$(compat_check release release renamed)"; rc=$?
if [ "$rc" -eq 0 ] && ! printf '%s' "$out" | grep -q 'charged'; then
  ok "COMPAT-1 bite 7: a renamed path template is the released operation (rename alone passes)"
else bad "COMPAT-1 bite 7: a renamed path template is the released operation" "exit=$rc" "$out"; fi
derive_contract renamed renamed-moved '.paths."/items/{item_id}".get.parameters[0].schema.type = "integer"'
out="$(compat_check release release renamed-moved)"; rc=$?
expect_charged "COMPAT-1 bite 7: a renamed template whose parameter also moved is judged against the released form" "$out" "$rc" "op GET /items/{}"

# Bite 10: a released operation this PR removes fails; one an earlier PR removed does not.
out="$(compat_check release release delete-gone)"; rc=$?
expect_charged "COMPAT-1 bite 10: removing a released operation fails" "$out" "$rc" "op DELETE /items/{}"
derive_contract delete-gone delete-gone-grown '.components.schemas.Orphan.properties.b = {"type":"string"}'
out="$(compat_check release delete-gone delete-gone-grown)"; rc=$?
expect_pass "COMPAT-1 bite 10: an operation an earlier PR removed is not charged again" "$out" "$rc"

# Bite 11: a component no entry point reaches is judged as its own entry point.
derive_contract release orphan-moved '.components.schemas.Orphan.properties.a.type = "integer"'
out="$(compat_check release release orphan-moved)"; rc=$?
expect_charged "COMPAT-1 bite 11: an unreachable component this PR breaks fails" "$out" "$rc" "component schemas/Orphan"
derive_contract release orphan-reached '.components.schemas.Item.properties.orphan = {"$ref":"#/components/schemas/Orphan"}'
out="$(compat_check release release orphan-reached)"; rc=$?
expect_pass "COMPAT-1 bite 11: a component this PR makes reachable is not read as removed" "$out" "$rc"

# Bite 12: a rename inside a reference cycle passes; servers on a released operation fail.
derive_contract release cycle-renamed '.components.schemas.Tree = (.components.schemas.Node
    | .properties.child."$ref" = "#/components/schemas/Tree")
  | del(.components.schemas.Node)
  | .components.schemas.Item.properties.parent."$ref" = "#/components/schemas/Tree"'
out="$(compat_check release release cycle-renamed)"; rc=$?
expect_pass "COMPAT-1 bite 12: a rename inside a reference cycle passes" "$out" "$rc"
derive_contract release servers-moved '.paths."/items".post.servers = [{"url":"https://elsewhere.example"}]'
out="$(compat_check release release servers-moved)"; rc=$?
expect_charged "COMPAT-1 bite 12: a change to a released operation's servers fails" "$out" "$rc" "op POST /items"

# Bite 9: a changed security scheme on a released operation fails.
derive_contract release scheme-moved '.components.securitySchemes.bearer.scheme = "basic"'
out="$(compat_check release release scheme-moved)"; rc=$?
expect_charged "COMPAT-1 bite 9: a changed security scheme on a released operation fails" "$out" "$rc" "op DELETE /items/{}"

# Bite 6 under the fallback, and the residue it records. An earlier PR declared maxLength
# 10 → 5; this PR relaxes it to 8. Against the release 10 → 8 is still a tightening, and this
# PR changes the operation, so the verdict-only fallback charges it again (the residue
# status.toml records for COMPAT-1). A deeper break at the same place (5 → 3) fails, as it must.
derive_contract release max5 '.components.schemas.ItemCreate.properties.name.maxLength = 5'
derive_contract release max8 '.components.schemas.ItemCreate.properties.name.maxLength = 8'
derive_contract max5 max5-enum '.components.schemas.ItemCreate.properties.name.maxLength = 3'
out="$(compat_check release max5 max8)"; rc=$?
expect_charged "COMPAT-1 bite 6 (residue): a relaxed earlier declared break is re-charged on the touched operation" "$out" "$rc" "op POST /items"
out="$(compat_check release max5 max5-enum)"; rc=$?
expect_charged "COMPAT-1 bite 6: a further break at an earlier declared break's place fails" "$out" "$rc" "op POST /items"
derive_contract max5 max5-elsewhere '.components.schemas.Orphan.properties.b = {"type":"string"}'
out="$(compat_check release max5 max5-elsewhere)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'release-relative verdict' \
    && ! printf '%s' "$out" | grep -q 'charged —'; then
  ok "COMPAT-1 bite 6: an earlier declared break on an operation this PR leaves alone is not charged"
else bad "COMPAT-1 bite 6: an earlier declared break this PR leaves alone is not charged" "exit=$rc" "$out"; fi

# A cycle marker names the enclosing node by its place in the operation, not by ref-hops: two
# contracts that differ (p.q was an Item-like node, it is now a node with only q) never resolve
# to the same form.
derive_contract release cyc-a '.paths."/a" = {"get":{"responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"$ref":"#/components/schemas/A"}}}}}}}
  | .components.schemas.A = {"type":"object","properties":{"v":{"type":"string"},"p":{"type":"object","properties":{"q":{"$ref":"#/components/schemas/A"}}}}}'
derive_contract cyc-a cyc-p '.components.schemas.A.properties.p = {"$ref":"#/components/schemas/P"}
  | .components.schemas.P = {"type":"object","properties":{"q":{"$ref":"#/components/schemas/P"}}}'
out="$(compat_check cyc-a cyc-a cyc-p)"; rc=$?
expect_charged "COMPAT-1: a cycle re-pointed at a different enclosing node is not aliased to the released one" "$out" "$rc" "op GET /a"

# A webhook's payload is server-sent: enum growth there is output growth, a break — never the
# input tolerance a request body gets. Both readings refuse it.
derive_contract release hook '.webhooks = {"evt":{"post":{"requestBody":{"content":{"application/json":{"schema":{"$ref":"#/components/schemas/Evt"}}}},"responses":{"200":{"description":"ok"}}}}}
  | .components.schemas.Evt = {"type":"object","properties":{"kind":{"type":"string","enum":["a","b"]}}}'
derive_contract hook hook-grown '.components.schemas.Evt.properties.kind.enum += ["c"]'
out="$(compat_check hook hook hook-grown)"; rc=$?
expect_charged "COMPAT-1 bite 9: enum growth in a released webhook payload fails" "$out" "$rc" "webhook evt POST"
out="$(compat_check none hook hook-grown)"; rc=$?
if [ "$rc" -ne 0 ]; then ok "COMPAT-1: the same webhook payload growth fails base-only"
else bad "COMPAT-1: webhook payload growth fails base-only" "exit=$rc" "$out"; fi
# ... and the same in a callback.
derive_contract release cb '.paths."/items".post.callbacks = {"done":{"{$request.body#/url}":{"post":{"requestBody":{"content":{"application/json":{"schema":{"type":"string","enum":["ok"]}}}},"responses":{"200":{"description":"ok"}}}}}}'
derive_contract cb cb-grown '.paths."/items".post.callbacks.done."{$request.body#/url}".post.requestBody.content."application/json".schema.enum += ["late"]'
out="$(compat_check cb cb cb-grown)"; rc=$?
expect_charged "COMPAT-1: enum growth in a released callback payload fails" "$out" "$rc" "op POST /items"

# Base-only is never looser than the per-entry-point reading: a released parameter component
# made required, with no release to measure against, still fails.
derive_contract param-born param-born-req '.components.parameters.Verbose.required = true'
out="$(compat_check none param-born param-born-req)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'base-only verdict'; then
  ok "COMPAT-2: base-only also judges resolved operations (a parameter component made required fails)"
else bad "COMPAT-2: base-only also judges resolved operations" "exit=$rc" "$out"; fi

# Two $ref'd parameters carrying non-prose siblings keep their own identities: one of them
# turning required fails, and is never collapsed away.
derive_contract release two-params '.paths."/items/{id}".get.parameters += [{"$ref":"#/components/parameters/A","x-order":1},{"$ref":"#/components/parameters/B","x-order":2}]
  | .components.parameters = {"A":{"in":"query","name":"a","schema":{"type":"string"}},"B":{"in":"query","name":"b","schema":{"type":"string"}}}'
derive_contract two-params two-params-a-req '.components.parameters.A.required = true'
out="$(compat_check two-params two-params two-params-a-req)"; rc=$?
expect_charged "COMPAT-1: a \$ref'd parameter with siblings is judged as itself" "$out" "$rc" "op GET /items/{}"

# A $ref into the middle of a component does not reach the rest of it: the rest is still judged.
derive_contract release partial '.components.schemas.Foo = {"type":"object","properties":{"x":{"type":"string"},"y":{"type":"string"}}}
  | .components.schemas.Item.properties.fx = {"$ref":"#/components/schemas/Foo/properties/x"}'
derive_contract partial partial-moved '.components.schemas.Foo.properties.y.type = "integer"'
out="$(compat_check partial partial partial-moved)"; rc=$?
expect_charged "COMPAT-1: a ref into the middle of a component leaves the rest judged as unreached" "$out" "$rc" "component schemas/Foo"

# A $ref'd parameter's non-prose siblings are ignored (OAS Reference Object), as a generator
# ignores them: a sibling restating the released schema cannot hide a narrowed target.
derive_contract release q-param '.paths."/items/{id}".get.parameters += [{"$ref":"#/components/parameters/Q"}]
  | .components.parameters = {"Q":{"in":"query","name":"q","schema":{"type":"string","enum":["a","b"]}}}'
derive_contract q-param q-masked '.components.parameters.Q.schema.enum = ["a"]
  | .paths."/items/{id}".get.parameters[-1].schema = {"type":"string","enum":["a","b"]}'
out="$(compat_check q-param q-param q-masked)"; rc=$?
expect_charged "COMPAT-1: a sibling beside a \$ref'd parameter cannot mask its narrowed target" "$out" "$rc" "op GET /items/{}"

# A chain of refs landing on one location, where one carries non-prose siblings, gives each
# enclosing node its own cycle marker: re-pointing B.next from A to B is a different contract.
derive_contract release chain '.paths."/chain" = {"get":{"responses":{"200":{"description":"ok","content":{"application/json":{"schema":{"$ref":"#/components/schemas/CA"}}}}}}}
  | .components.schemas.CA = {"$ref":"#/components/schemas/CB","properties":{"tag":{"type":"string","enum":["a"]}}}
  | .components.schemas.CB = {"type":"object","properties":{"next":{"$ref":"#/components/schemas/CA"}}}'
derive_contract chain chain-moved '.components.schemas.CB.properties.next."$ref" = "#/components/schemas/CB"'
out="$(compat_check chain chain chain-moved)"; rc=$?
expect_charged "COMPAT-1: a cycle through a sibling-carrying \$ref chain keeps distinct markers" "$out" "$rc" "op GET /chain"

# `required` written out wherever it is a boolean: a callback parameter, a request body and an
# unreached parameter component that gain it are judged, never a comparator crash.
derive_contract cb cb-param '.paths."/items".post.callbacks.done."{$request.body#/url}".post.parameters = [{"in":"header","name":"X-Sig","schema":{"type":"string"}}]'
derive_contract cb-param cb-param-req '.paths."/items".post.callbacks.done."{$request.body#/url}".post.parameters[0].required = true'
out="$(compat_check cb-param cb-param cb-param-req)"; rc=$?
expect_charged "COMPAT-1: a callback parameter gaining required is judged, not a crash" "$out" "$rc" "op POST /items"
derive_contract release rb-req '.paths."/items".post.requestBody.required = true'
out="$(compat_check release release rb-req)"; rc=$?
expect_charged "COMPAT-1: a request body gaining required is judged, not a crash" "$out" "$rc" "op POST /items"
derive_contract release lone-param '.components.parameters = {"L":{"in":"query","name":"l","schema":{"type":"string"}}}'
derive_contract lone-param lone-param-req '.components.parameters.L.required = true'
out="$(compat_check lone-param lone-param lone-param-req)"; rc=$?
expect_charged "COMPAT-1: an unreached parameter component gaining required is judged, not a crash" "$out" "$rc" "component parameters/L"

# example/examples beside a $ref are prose, as broke_node reads them.
derive_contract release ex '.paths."/items/{id}".get.responses."200".content."application/json".schema.examples = [{"id":"z"}]'
out="$(compat_check release release ex)"; rc=$?
expect_pass "COMPAT-1: examples beside a \$ref are prose, not a break" "$out" "$rc"

# A resolver that cannot resolve is "could not look": a dangling $ref fails, never passes.
derive_contract release dangling '.components.schemas.Item.properties.name = {"$ref":"#/components/schemas/Nowhere"}'
out="$(compat_check release release dangling)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'verdict: undeterminable'; then
  ok "COMPAT-1: an unresolvable \$ref is undeterminable and fails"
else bad "COMPAT-1: an unresolvable \$ref is undeterminable and fails" "exit=$rc" "$out"; fi

# ── 21. COMPAT-2 — the last release is the release tooling's, and a fallback is never one ────────
# A sandbox upstream with release tags, cloned the way CI and a developer would clone it. The
# gate runs in git mode (no --release): it calls last_release from the release library, the same
# function detect-changes.sh calls. The contracts are the bite-1 pair, so a release-relative
# verdict passes and a base-only verdict fails: the verdict line says which one ran.
UP="${WORK}/upstream"
mkdir -p "${UP}/.github/scripts" "${UP}/tools/scripts/release/lib"
cp "${REPO}/.github/scripts/wire-class-crosscheck.sh" "${REPO}/.github/scripts/"wire-shape*.jq "${REPO}/.github/scripts/wire-resolve.py" "${UP}/.github/scripts/"
cp -R "${REPO}/tools/manifest" "${UP}/tools/manifest"
cp "${REPO}/tools/scripts/release/lib/common.sh" "${UP}/tools/scripts/release/lib/common.sh"
cp "${REPO}/tools/scripts/release/detect-changes.sh" "${UP}/tools/scripts/release/detect-changes.sh"
reset_register
cp -R "$REG" "${UP}/release-register"
up() { git -C "$UP" -c user.name=t -c user.email=t@t -c commit.gpgsign=false "$@"; }
cp "${CDIR}/release.json" "${UP}/openapi.json"
up init -q -b main
up add -A
up commit -q -m "release"
up tag v1.0.0
cp "${CDIR}/born.json" "${UP}/openapi.json"
up commit -q -am "window: a born schema"
UP_BASE="$(up rev-parse HEAD)"
up checkout -q -b pr
cp "${CDIR}/born-moved.json" "${UP}/openapi.json"
up commit -q -am "the PR: the born schema moves"
up checkout -q main

clone_of() { # $1 = dir, remaining = git clone args
  local d="$1"; shift
  rm -rf "$d"
  git clone -q "$@" "file://${UP}" "$d" 2>/dev/null
  git -C "$d" fetch -q origin pr 2>/dev/null
  git -C "$d" -c advice.detachedHead=false checkout -q FETCH_HEAD 2>/dev/null
}
derived_check() { # $1 = clone; the gate in git mode against the window base
  reset_fixtures
  printf '%s\n' "openapi.json" > "$WIRE"
  add_own_row self additive
  (cd "$1" && bash .github/scripts/wire-class-crosscheck.sh --wire-touched "$WIRE" \
      --added-entries "$ADDED" --register-dir "$REG" --legacy-register "$LEGACY" --base "$UP_BASE" 2>&1)
}

FULL="${WORK}/clone-full"
clone_of "$FULL"
out="$(derived_check "$FULL")"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'release: v1.0.0 — release-relative verdict'; then
  ok "COMPAT-2: a full clone with tags measures against the derived release v1.0.0"
else bad "COMPAT-2: a full clone with tags measures against the derived release" "exit=$rc" "$out"; fi

# Bite 15: the CI shape before the fix — depth 2, no tags. The derivation's fallback would be a
# root inside the window (here: the PR's own parent); it must give base-only, never a verdict
# against that commit.
SHALLOW="${WORK}/clone-shallow"
clone_of "$SHALLOW" --depth 2 --no-tags
git -C "$SHALLOW" fetch -q --depth=1 origin "$UP_BASE" 2>/dev/null
out="$(derived_check "$SHALLOW")"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'base-only verdict (COMPAT-2): the clone is shallow'; then
  ok "COMPAT-2 bite 15: a shallow clone with no tags gives the base-only verdict"
else bad "COMPAT-2 bite 15: a shallow clone with no tags gives the base-only verdict" "exit=$rc" "$out"; fi
DETECT_OUT="$(cd "$SHALLOW" && bash tools/scripts/release/detect-changes.sh 2>/dev/null)"
if printf '%s' "$DETECT_OUT" | grep -qx 'CHANGES_BASE_IS_FALLBACK=true'; then
  ok "COMPAT-2: detect-changes reports the shallow clone's base as a fallback, machine-readably"
else bad "COMPAT-2: detect-changes reports the fallback machine-readably" "$DETECT_OUT"; fi
# Unshallowed with tags, the same clone becomes a provable one.
git -C "$SHALLOW" fetch -q --unshallow --tags origin 2>/dev/null
out="$(derived_check "$SHALLOW")"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'release: v1.0.0 — release-relative verdict'; then
  ok "COMPAT-2: after an unshallow-and-tags fetch, the same clone measures against v1.0.0"
else bad "COMPAT-2: after an unshallow-and-tags fetch, the clone measures against v1.0.0" "exit=$rc" "$out"; fi
# The guard-tests checkout: full history, tags, blobs on demand (fetch-depth 0, filter blob:none).
BLOBLESS="${WORK}/clone-blobless"
clone_of "$BLOBLESS" --filter=blob:none
out="$(derived_check "$BLOBLESS")"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'release: v1.0.0 — release-relative verdict'; then
  ok "COMPAT-2: the guard-tests checkout shape (blobless, full history, tags) measures against v1.0.0"
else bad "COMPAT-2: the guard-tests checkout shape measures against v1.0.0" "exit=$rc" "$out"; fi
DETECT_OUT="$(cd "$SHALLOW" && bash tools/scripts/release/detect-changes.sh 2>/dev/null)"
if printf '%s' "$DETECT_OUT" | grep -qx 'CHANGES_BASE_REF=v1.0.0' \
    && printf '%s' "$DETECT_OUT" | grep -qx 'CHANGES_BASE_IS_FALLBACK=false'; then
  ok "COMPAT-2: detect-changes and the gate derive the same release"
else bad "COMPAT-2: detect-changes and the gate derive the same release" "$DETECT_OUT"; fi

# Bite 16: a tag in the release form on a PR-only commit is not the release.
up tag v9.9.9 pr
clone_of "$FULL"
out="$(derived_check "$FULL")"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'release: v1.0.0 — release-relative verdict'; then
  ok "COMPAT-2 bite 16: a release-form tag on a PR-only commit is not taken as the release"
else bad "COMPAT-2 bite 16: a release-form tag on a PR-only commit is not the release" "exit=$rc" "$out"; fi
up tag -d v9.9.9 >/dev/null

# A nearer tag that is not in the release form (a pre-release) neither hides the release nor
# forces a fallback: only v<MAJOR>.<MINOR>.<PATCH> tags are releases.
up tag v1.1.0-rc1 "$UP_BASE"
clone_of "$FULL"
out="$(derived_check "$FULL")"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -q 'release: v1.0.0 — release-relative verdict'; then
  ok "COMPAT-2: a nearer pre-release tag is skipped, not taken and not a fallback"
else bad "COMPAT-2: a nearer pre-release tag is skipped" "exit=$rc" "$out"; fi
up tag -d v1.1.0-rc1 >/dev/null

# The newest release by version, not the nearest tag: a maintenance-line tag merged in after a
# newer release must not stand in for it (shape shipped in the newer release would read as
# window-born).
up checkout -q -b maint v1.0.0
echo "fix" > "${UP}/maint.txt"; up add maint.txt; up commit -q -m "maint fix"
up tag v1.0.9
up checkout -q main
up tag v1.1.0 "$UP_BASE~0"
up merge -q --no-edit -m "merge maint" maint >/dev/null 2>&1
MAINT_BASE="$(up rev-parse HEAD)"
clone_of "$FULL"
out="$( (cd "$FULL" && source tools/scripts/release/lib/common.sh && last_release "$MAINT_BASE") 2>&1)"
if printf '%s' "$out" | grep -qx 'LAST_RELEASE_REF=v1.1.0' && printf '%s' "$out" | grep -qx 'LAST_RELEASE_IS_FALLBACK=false'; then
  ok "COMPAT-2: the newest release by version wins over a nearer maintenance-line tag"
else bad "COMPAT-2: the newest release by version wins over a nearer maintenance tag" "$out"; fi
up tag -d v1.0.9 v1.1.0 >/dev/null
up reset -q --hard "$UP_BASE"

# A tag only this clone holds (nobody pushed it) is not a release either.
clone_of "$FULL"
git -C "$FULL" tag v1.0.1 "$UP_BASE"
out="$(derived_check "$FULL")"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'does not list v1.0.1'; then
  ok "COMPAT-2: a release-form tag the remote does not hold gives the base-only verdict"
else bad "COMPAT-2: a local-only release-form tag gives the base-only verdict" "exit=$rc" "$out"; fi

# Bite 17: the clone holds the older release, the remote a newer one the base contains.
clone_of "$FULL"
up tag v1.0.2 "$UP_BASE"
out="$(derived_check "$FULL")"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'lists v1.0.2, a newer release than v1.0.0'; then
  ok "COMPAT-2 bite 17: an older local tag, with a newer one on the remote, gives the base-only verdict"
else bad "COMPAT-2 bite 17: an older local tag with a newer remote one gives base-only" "exit=$rc" "$out"; fi
up tag -d v1.0.2 >/dev/null
# ... and a remote whose tags cannot be listed.
clone_of "$FULL"
git -C "$FULL" remote set-url origin "file://${WORK}/no-such-remote"
out="$(derived_check "$FULL")"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'could not be listed'; then
  ok "COMPAT-2 bite 17: a remote whose tags cannot be listed gives the base-only verdict"
else bad "COMPAT-2 bite 17: an unlistable remote gives the base-only verdict" "exit=$rc" "$out"; fi

# Bite 14: no release tag at all.
clone_of "$FULL"
up tag -d v1.0.0 >/dev/null
git -C "$FULL" tag -d v1.0.0 >/dev/null
out="$(derived_check "$FULL")"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'base-only verdict (COMPAT-2): no v<MAJOR>.<MINOR>.<PATCH> tag'; then
  ok "COMPAT-2 bite 14: with no release ref resolvable, the gate gives the base-only verdict"
else bad "COMPAT-2 bite 14: with no release ref, base-only" "exit=$rc" "$out"; fi

echo
echo "  ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
