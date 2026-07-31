#!/usr/bin/env bash
# .github/scripts/test-sqlx-wire-diff.sh
#
# Test harness for sqlx-wire-diff.sh — the change detector that answers "did this PR move the wire
# contract between the binary and the schema?" by diffing the .sqlx caches against a base revision.
#
# WHAT THE KEYING MAKES TRUE, AND WHY IT MATTERS HERE
# ---------------------------------------------------
# A cache entry's filename is `query-<sha256 of the query text>.json`, and that is the WHOLE key.
# Verified 2026-07-31 across `.sqlx`: filename hash == the `hash` field == sha256(query), 351
# distinct queries, zero duplicate query texts.
#
# So a changed query text can NEVER show up as a modified entry — it is a new file plus a deleted
# one. A MODIFIED entry means the same query text now describes differently, which can only be the
# schema moving underneath it. That is exactly and only the signal spec §2 asks for, and it is why
# "changed query, identical types" needs no special-casing: the keying makes it structurally
# impossible to confuse with a move.
#
#   bash .github/scripts/test-sqlx-wire-diff.sh

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DIFF_SCRIPT="${SCRIPT_DIR}/sqlx-wire-diff.sh"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; shift; printf '    %s\n' "$@"; FAIL=$((FAIL + 1)); }

# Exit-code contract under test: 0 = unchanged, 10 = the wire contract moved, 2 = base unresolvable.
UNCHANGED=0
MOVED=10

# Write one cache entry. $1 dir, $2 hash, $3 columns-json, $4 parameters-json, [$5 query text]
entry() {
  local dir="$1" h="$2" cols="$3" params="$4" q="${5:-SELECT 1}"
  mkdir -p "$dir"
  cat > "${dir}/query-${h}.json" <<JSON
{
  "db_name": "PostgreSQL",
  "query": "${q}",
  "describe": {
    "columns": ${cols},
    "parameters": ${params},
    "nullable": [false]
  },
  "hash": "${h}"
}
JSON
}

COL_UUID='[{"ordinal":0,"name":"id","type_info":"Uuid"}]'
COL_UUID_ARRAY='[{"ordinal":0,"name":"id","type_info":"UuidArray"}]'
COL_RENAMED='[{"ordinal":0,"name":"identifier","type_info":"Uuid"}]'
COL_TWO='[{"ordinal":0,"name":"id","type_info":"Uuid"},{"ordinal":1,"name":"slug","type_info":"Text"}]'
P_ONE='{"Left":["Uuid"]}'
P_TWO='{"Left":["Uuid","Text"]}'

# A base/head pair of repo-shaped roots, identical to start with.
new_pair() {
  rm -rf "${WORK}/base" "${WORK}/head"
  entry "${WORK}/base/.sqlx" aaa "$COL_UUID" "$P_ONE"
  entry "${WORK}/head/.sqlx" aaa "$COL_UUID" "$P_ONE"
  entry "${WORK}/base/crates/temper-services/.sqlx" bbb "$COL_TWO" "$P_TWO"
  entry "${WORK}/head/crates/temper-services/.sqlx" bbb "$COL_TWO" "$P_TWO"
}

run_diff() { bash "$DIFF_SCRIPT" --base-dir "${WORK}/base" --head-dir "${WORK}/head" 2>&1; }

echo "test-sqlx-wire-diff"
echo

# ── 1. Identical caches: no move ────────────────────────────────────────────────────────────────
new_pair
out="$(run_diff)"; rc=$?
if [ "$rc" -eq "$UNCHANGED" ]; then ok "identical caches report no move"
else bad "identical caches report no move" "exit=$rc" "$out"; fi

# ── 2. A changed type_info is a move — the 2026-07-30 shape exactly ──────────────────────────────
new_pair
entry "${WORK}/head/.sqlx" aaa "$COL_UUID_ARRAY" "$P_ONE"
out="$(run_diff)"; rc=$?
if [ "$rc" -eq "$MOVED" ] && printf '%s' "$out" | grep -q 'UuidArray'; then
  ok "a changed type_info is a wire move, naming the new type"
else bad "a changed type_info is a wire move, naming the new type" "exit=$rc" "$out"; fi

# ── 3. A changed parameters.Left[] is a move ────────────────────────────────────────────────────
new_pair
entry "${WORK}/head/.sqlx" aaa "$COL_UUID" "$P_TWO"
out="$(run_diff)"; rc=$?
if [ "$rc" -eq "$MOVED" ] && printf '%s' "$out" | grep -qi 'parameter'; then
  ok "a changed parameters.Left[] is a wire move"
else bad "a changed parameters.Left[] is a wire move" "exit=$rc" "$out"; fi

# ── 4. A NEW entry is not a move ────────────────────────────────────────────────────────────────
new_pair
entry "${WORK}/head/.sqlx" ccc "$COL_TWO" "$P_TWO"
out="$(run_diff)"; rc=$?
if [ "$rc" -eq "$UNCHANGED" ]; then ok "a new cache entry is not a wire move"
else bad "a new cache entry is not a wire move" "exit=$rc" "$out"; fi

# ── 5. A DELETED entry is not a move ────────────────────────────────────────────────────────────
# Decision, recorded rather than left implicit: a query that no longer exists cannot be decoded by
# the new binary, and the OLD binary's copy of it is unaffected by anything in the cache. Removal is
# a code change, not a schema move. The migration-side risk of deleting a query is covered by the
# declaration, not here.
new_pair
rm "${WORK}/head/.sqlx/query-aaa.json"
out="$(run_diff)"; rc=$?
if [ "$rc" -eq "$UNCHANGED" ]; then ok "a deleted cache entry is not a wire move"
else bad "a deleted cache entry is not a wire move" "exit=$rc" "$out"; fi

# ── 6. A changed QUERY TEXT is a new+deleted pair, not a move ───────────────────────────────────
# The structural case. Types differ between the two entries, but they are different queries, so
# nothing moved under anything.
new_pair
rm "${WORK}/head/.sqlx/query-aaa.json"
entry "${WORK}/head/.sqlx" ddd "$COL_UUID_ARRAY" "$P_TWO" "SELECT 2"
out="$(run_diff)"; rc=$?
if [ "$rc" -eq "$UNCHANGED" ]; then ok "a changed query text (new+deleted) is not a wire move"
else bad "a changed query text (new+deleted) is not a wire move" "exit=$rc" "$out"; fi

# ── 7. A renamed output column is a move ────────────────────────────────────────────────────────
# sqlx's macros bind by NAME. Same query text with a differently-named column means a function's
# output was renamed underneath it, and the deployed binary's decode breaks.
new_pair
entry "${WORK}/head/.sqlx" aaa "$COL_RENAMED" "$P_ONE"
out="$(run_diff)"; rc=$?
if [ "$rc" -eq "$MOVED" ] && printf '%s' "$out" | grep -q 'identifier'; then
  ok "a renamed output column is a wire move"
else bad "a renamed output column is a wire move" "exit=$rc" "$out"; fi

# ── 8. A column APPENDED under an unchanged query text is a move ────────────────────────────────
# Only reachable via SELECT * over a set-returning function, which is why it is rare — but the
# ordinals of everything after it shift, so it is not benign.
new_pair
entry "${WORK}/head/.sqlx" aaa "$COL_TWO" "$P_ONE"
out="$(run_diff)"; rc=$?
if [ "$rc" -eq "$MOVED" ]; then ok "a column count change under an unchanged query is a wire move"
else bad "a column count change under an unchanged query is a wire move" "exit=$rc" "$out"; fi

# ── 9. The SECOND cache is actually read ────────────────────────────────────────────────────────
# crates/temper-services/.sqlx holds 441 of the repo's 802 in-scope entries — more than the
# workspace cache. A differ that only walks ./.sqlx would silently ignore the majority and report
# a clean verdict, which is the exact failure mode this whole goal exists to prevent.
new_pair
entry "${WORK}/head/crates/temper-services/.sqlx" bbb "$COL_UUID_ARRAY" "$P_TWO"
out="$(run_diff)"; rc=$?
if [ "$rc" -eq "$MOVED" ] && printf '%s' "$out" | grep -q 'temper-services'; then
  ok "the temper-services cache is in scope and reported by path"
else bad "the temper-services cache is in scope and reported by path" "exit=$rc" "$out"; fi

# ── 10. tests/e2e is OUT of scope, and says so out loud ─────────────────────────────────────────
# Decision: the wire contract that can break a deploy is the running binary's, and tests/e2e is not
# deployed — the same scoping spec §6 applies to the macro allow-list. Recorded as an announcement
# rather than a silence, because a silently-omitted cache is indistinguishable from a bug.
new_pair
entry "${WORK}/base/tests/e2e/.sqlx" eee "$COL_UUID" "$P_ONE"
entry "${WORK}/head/tests/e2e/.sqlx" eee "$COL_UUID_ARRAY" "$P_ONE"
out="$(run_diff)"; rc=$?
if [ "$rc" -eq "$UNCHANGED" ] && printf '%s' "$out" | grep -q 'tests/e2e'; then
  ok "tests/e2e is excluded, and the exclusion is announced"
else bad "tests/e2e is excluded, and the exclusion is announced" "exit=$rc" "$out"; fi

# ── 11. nullable-only change is NOTED but does not trigger the verdict ──────────────────────────
# Spec §2 decided on type_info and parameters.Left[]. A nullability flip is a real decode risk
# (T vs Option<T>), but sqlx infers nullability heuristically and conservatively, so folding it into
# the verdict would train people to ignore the verdict. Reported under its own heading instead — a
# declared hole rather than a silent narrowing.
new_pair
python3 - "${WORK}/head/.sqlx/query-aaa.json" <<'PY'
import json,sys
p=sys.argv[1]; d=json.load(open(p)); d['describe']['nullable']=[True]; json.dump(d,open(p,'w'))
PY
out="$(run_diff)"; rc=$?
if [ "$rc" -eq "$UNCHANGED" ] && printf '%s' "$out" | grep -qi 'nullab'; then
  ok "a nullable-only change is noted but does not trigger the verdict"
else bad "a nullable-only change is noted but does not trigger the verdict" "exit=$rc" "$out"; fi

# ── 12. An unresolvable base is exit 2, never a clean pass ──────────────────────────────────────
# The canary's lesson, carried: a safeguard that derives its input from the healthy state cannot
# bootstrap from the unhealthy one. "I could not look" must never render as "nothing moved".
out="$(bash "$DIFF_SCRIPT" --base-dir "${WORK}/does-not-exist" --head-dir "${WORK}/head" 2>&1)"; rc=$?
if [ "$rc" -eq 2 ]; then ok "an unreadable base is exit 2, not a clean pass"
else bad "an unreadable base is exit 2, not a clean pass" "exit=$rc" "$out"; fi

echo
echo "  ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
