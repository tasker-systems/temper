#!/usr/bin/env bash
# .github/scripts/test-audit-as-entry-points.sh
#
# Test harness for audit-as-entry-points.sh. Runs the auditor against the real api/ tree and
# vercel.json and against fixtures DERIVED from them by one edit each, asserting the auditor
# fails and says WHY:
#
#   - a new api/** file fails the baseline diff (the entry set grew)
#   - a removed api/** file fails the baseline diff symmetrically (stale baseline entry)
#   - a vercel.json route dest with no file behind it fails resolution
#   - a vercel.json functions key with no file behind it fails
#   - a new api/** file PLUS the route naming it still fails: the file is the thing without a
#     baseline entry — wiring it does not review it
#
# WHY A HARNESS RATHER THAN A COMMENT
# -----------------------------------
# A guard that cannot fail is worse than no guard: it emits a green tick that means nothing. The
# tests below are the evidence that this one CAN fail, re-run on every CI run. Fixtures are
# derived from the live tree rather than hand-written, so they cannot rot into testing a shape
# the tree no longer has.
#
# The API_DIR fixture must live under the repo root (the auditor `cd`s to the toplevel and
# resolves API_DIR from there), so it is created inside .github/scripts/ and removed on exit.
#
#   bash .github/scripts/test-audit-as-entry-points.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT_SCRIPT="${SCRIPT_DIR}/audit-as-entry-points.sh"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
REAL_VERCEL="${REPO_ROOT}/vercel.json"
PASS=0
FAIL=0

FIXTURE_DIR="${SCRIPT_DIR}/.tmp-test-as-entry-points.$$"
mkdir -p "$FIXTURE_DIR"
trap 'rm -rf "$FIXTURE_DIR"' EXIT

# run_test NAME API_DIR VERCEL_JSON EXPECTED_EXIT [EXPECTED_SUBSTRING]
run_test() {
    local test_name="$1"
    local api_dir="$2"
    local vercel_json="$3"
    local expected_exit="$4"
    local expected_substr="${5:-}"

    local output actual_exit
    set +e
    output="$(API_DIR="$api_dir" VERCEL_JSON="$vercel_json" bash "$AUDIT_SCRIPT" 2>&1)"
    actual_exit=$?
    set -e

    if [ "$actual_exit" -ne "$expected_exit" ]; then
        echo "  FAIL: ${test_name}"
        echo "    expected exit=${expected_exit} actual exit=${actual_exit}"
        echo "    output: ${output}"
        FAIL=$((FAIL + 1))
        return
    fi
    if [ -n "$expected_substr" ] && ! printf '%s' "$output" | grep -qF -- "$expected_substr"; then
        echo "  FAIL: ${test_name}"
        echo "    exit code matched but expected message not found: ${expected_substr}"
        echo "    output: ${output}"
        FAIL=$((FAIL + 1))
        return
    fi
    echo "  PASS: ${test_name}"
    PASS=$((PASS + 1))
}

# fresh_api_copy — a full copy of the live api/ tree inside the fixture dir (relative path).
fresh_api_copy() {
    cp -R "${REPO_ROOT}/api" "${FIXTURE_DIR}/api"
    printf '%s' ".github/scripts/.tmp-test-as-entry-points.$$/api"
}

echo "Running audit-as-entry-points.sh tests..."
echo ""

# --- (1) the real tree passes ---
run_test "real tree: passes" "api" "$REAL_VERCEL" 0

# --- (2) a new api/** file: the entry set grew, fails naming the shape of the failure ---
API_COPY="$(fresh_api_copy)"
mkdir -p "${FIXTURE_DIR}/api/admin"
: > "${FIXTURE_DIR}/api/admin/ping.ts"
run_test "new api file: fails baseline diff" "$API_COPY" "$REAL_VERCEL" 1 \
    "entry-point set changed"

# --- (3) a removed api/** file: the baseline went stale, fails symmetrically ---
API_COPY="$(fresh_api_copy)"
rm "${FIXTURE_DIR}/api/oauth/token.ts"
run_test "removed api file: fails baseline diff symmetrically" "$API_COPY" "$REAL_VERCEL" 1 \
    "entry-point set changed"

# --- (4) a route dest with no file behind it: a public path mapping at nothing ---
GHOST_DEST_VERCEL="${FIXTURE_DIR}/vercel-ghost-dest.json"
jq '.routes += [{"src": "/ghost", "dest": "/api/ghost"}]' "$REAL_VERCEL" > "$GHOST_DEST_VERCEL"
run_test "dangling routes dest: fails resolution" "api" "$GHOST_DEST_VERCEL" 1 \
    "'/api/ghost' resolves to no file"

# --- (5) a functions key with no file behind it ---
GHOST_FN_VERCEL="${FIXTURE_DIR}/vercel-ghost-fn.json"
jq '.functions["api/gone.rs"] = {"memory": 1024}' "$REAL_VERCEL" > "$GHOST_FN_VERCEL"
run_test "dangling functions key: fails" "api" "$GHOST_FN_VERCEL" 1 \
    "'api/gone.rs' is not a file"

# --- (6) a new file AND the route naming it: still fails — wiring is not reviewing ---
API_COPY="$(fresh_api_copy)"
mkdir -p "${FIXTURE_DIR}/api/admin"
: > "${FIXTURE_DIR}/api/admin/ping.ts"
WIRED_VERCEL="${FIXTURE_DIR}/vercel-wired-new.json"
jq '.routes += [{"src": "/admin/ping", "dest": "/api/admin/ping"}]' "$REAL_VERCEL" > "$WIRED_VERCEL"
run_test "new file wired by a new route: still fails (the file lacks a baseline entry)" \
    "$API_COPY" "$WIRED_VERCEL" 1 \
    "entry-point set changed"

# --- (7) a new public path at an ALREADY-REVIEWED file: the frozen routes array is the trip ---
NEW_SRC_VERCEL="${FIXTURE_DIR}/vercel-new-src.json"
jq '.routes += [{"src": "/anything", "dest": "/api/oauth/token"}]' "$REAL_VERCEL" > "$NEW_SRC_VERCEL"
run_test "new public path at reviewed file: fails frozen routes array" "api" "$NEW_SRC_VERCEL" 1 \
    "routes array changed"

# --- (8) a reorder: first-match semantics are part of the mapping, and the freeze bites ---
REORDER_VERCEL="${FIXTURE_DIR}/vercel-reorder.json"
jq '.routes = (.routes | reverse)' "$REAL_VERCEL" > "$REORDER_VERCEL"
run_test "routes reordered: fails frozen routes array" "api" "$REORDER_VERCEL" 1 \
    "routes array changed"

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
