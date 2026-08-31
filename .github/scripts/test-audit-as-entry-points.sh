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
#   - a NEW public path at an ALREADY-REVIEWED file fails the whole-file freeze
#   - a reorder of the routes array fails the whole-file freeze (first-match semantics)
#   - a `rewrites` key — a reachability channel outside the routes array — fails the freeze
#   - a crons schedule change fails the freeze (crons are scheduled unauthenticated GETs)
#   - a router-assembly token in an api bin fails the bin check
#   - UPDATE_BASELINE=1 refuses to run on a failing tree, and runs clean on a passing one
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

# run_test NAME API_DIR VERCEL_JSON EXPECTED_EXIT [EXPECTED_SUBSTRING] [EXTRA_ENV]
run_test() {
    local test_name="$1"
    local api_dir="$2"
    local vercel_json="$3"
    local expected_exit="$4"
    local expected_substr="${5:-}"
    local extra_env="${6:-}"

    local output actual_exit
    set +e
    # shellcheck disable=SC2086
    output="$(env $extra_env API_DIR="$api_dir" VERCEL_JSON="$vercel_json" bash "$AUDIT_SCRIPT" 2>&1)"
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
# rm -rf first: a second cp -R into an existing target would NEST the copy (api/api/...) and
# poison every later probe.
fresh_api_copy() {
    rm -rf "${FIXTURE_DIR}/api"
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

# --- (7) a new public path at an ALREADY-REVIEWED file: the whole-file freeze is the trip ---
NEW_SRC_VERCEL="${FIXTURE_DIR}/vercel-new-src.json"
jq '.routes += [{"src": "/anything", "dest": "/api/oauth/token"}]' "$REAL_VERCEL" > "$NEW_SRC_VERCEL"
run_test "new public path at reviewed file: fails the freeze" "api" "$NEW_SRC_VERCEL" 1 \
    "vercel.json changed"

# --- (8) a reorder: first-match semantics are part of the mapping, and the freeze bites ---
REORDER_VERCEL="${FIXTURE_DIR}/vercel-reorder.json"
jq '.routes = (.routes | reverse)' "$REAL_VERCEL" > "$REORDER_VERCEL"
run_test "routes reordered: fails the freeze" "api" "$REORDER_VERCEL" 1 \
    "vercel.json changed"

# --- (9) a `rewrites` key: a reachability channel the routes array never carried ---
REWRITES_VERCEL="${FIXTURE_DIR}/vercel-rewrites.json"
jq '.rewrites = [{"source": "/hidden", "destination": "/api/internal"}]' "$REAL_VERCEL" > "$REWRITES_VERCEL"
run_test "rewrites key: fails the freeze" "api" "$REWRITES_VERCEL" 1 \
    "vercel.json changed"

# --- (10) a crons change: crons are scheduled unauthenticated GETs, and are frozen ---
CRONS_VERCEL="${FIXTURE_DIR}/vercel-crons.json"
jq '.crons[0].schedule = "0 0 31 2 *"' "$REAL_VERCEL" > "$CRONS_VERCEL"
run_test "crons schedule change: fails the freeze" "api" "$CRONS_VERCEL" 1 \
    "vercel.json changed"

# --- (11) a router-assembly token in an api bin: the bin check bites ---
# API_BINS_OVERRIDE points at a scratch copy — with the real tree as API_DIR, check (a) stays
# green and the bin check is the ONLY thing that can fire.
SCRATCH_BIN="${FIXTURE_DIR}/mcp-assembly.rs"
cp "${REPO_ROOT}/api/mcp.rs" "$SCRATCH_BIN"
printf '\nfn stray() { let _r = Router::new(); }\n' >> "$SCRATCH_BIN"
run_test "assembly token in api bin: fails bin check" "api" "$REAL_VERCEL" 1 \
    "router assembly or route-declaration token" "API_BINS_OVERRIDE=${SCRATCH_BIN}"

# --- (11b) a .route( appended in a bin: reachable via the /mcp(.*) mapping, refused ---
SCRATCH_BIN_ROUTE="${FIXTURE_DIR}/mcp-route.rs"
cp "${REPO_ROOT}/api/mcp.rs" "$SCRATCH_BIN_ROUTE"
printf '\nlet r = r.route("/mcp/admin/ping", get(|| async { "ok" }));\n' >> "$SCRATCH_BIN_ROUTE"
run_test ".route( appended in api bin: fails bin check" "api" "$REAL_VERCEL" 1 \
    "route-declaration token" "API_BINS_OVERRIDE=${SCRATCH_BIN_ROUTE}"

# --- (11c) an EMPTY API_BINS_OVERRIDE: refused — it would silently check zero bins ---
run_test "empty API_BINS_OVERRIDE: refused" "api" "$REAL_VERCEL" 1 \
    "refusing to check zero bins" "API_BINS_OVERRIDE="

# --- (11d) a comment saying "Router" in an api bin: GREEN direction — prose is not assembly ---
SCRATCH_BIN_COMMENT="${FIXTURE_DIR}/mcp-comment.rs"
cp "${REPO_ROOT}/api/mcp.rs" "$SCRATCH_BIN_COMMENT"
printf '\n// the axum Router::new lives in the crate, not here\n' >> "$SCRATCH_BIN_COMMENT"
run_test "comment saying Router in api bin: stays green" "api" "$REAL_VERCEL" 0 "" \
    "API_BINS_OVERRIDE=${SCRATCH_BIN_COMMENT}"

# --- (12) a sibling Vercel config: only one config file is honored, and only vercel.json is frozen ---
touch vercel.toml
run_test "vercel.toml sibling: fails" "api" "$REAL_VERCEL" 1 \
    "vercel.toml exists"
rm -f vercel.toml
run_test "sibling config removed: green again" "api" "$REAL_VERCEL" 0

# --- (13) UPDATE_BASELINE=1 on a failing tree: refused, cannot launder ---
# In CI the guard refuses update mode outright (CI check fires first), so the expected message
# depends on where the harness runs; the exit is 1 either way.
API_COPY="$(fresh_api_copy)"
mkdir -p "${FIXTURE_DIR}/api/admin"
: > "${FIXTURE_DIR}/api/admin/ping.ts"
if [[ -n "${CI:-}" ]]; then
    UPDATE_FAIL_MSG="UPDATE_BASELINE is not available in CI"
else
    UPDATE_FAIL_MSG="UPDATE_BASELINE refused"
fi
run_test "UPDATE_BASELINE on failing tree: refused" "$API_COPY" "$REAL_VERCEL" 1 \
    "$UPDATE_FAIL_MSG" "UPDATE_BASELINE=1"

# --- (14) UPDATE_BASELINE=1 on the clean tree: prints the baseline, exits 0 — locally only ---
if [[ -n "${CI:-}" ]]; then
    run_test "UPDATE_BASELINE on clean tree: refused in CI" "api" "$REAL_VERCEL" 1 \
        "UPDATE_BASELINE is not available in CI" "UPDATE_BASELINE=1"
else
    run_test "UPDATE_BASELINE on clean tree: prints and exits 0" "api" "$REAL_VERCEL" 0 \
        "copy into BASELINE" "UPDATE_BASELINE=1"
fi

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
