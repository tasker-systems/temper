#!/usr/bin/env bash
# .github/scripts/test-audit-mcp-route-auth.sh
#
# Test harness for audit-mcp-route-auth.sh. Runs the auditor against the real router.rs and
# against fixtures DERIVED from it by one edit each, asserting the auditor fails and says WHY:
#
#   - a new public route in a NEW sub-router group fails as an unknown posture
#   - a new public route in a REVIEWED public group fails the baseline diff
#   - require_mcp_auth deleted from the mcp_routes block fails the wiring assertion
#   - nest_service("/mcp" retargeted fails the wiring assertion
#   - the mcp_routes declaration renamed fails loudly rather than silently skipping
#   - the handler behind a reviewed public path swapped fails the baseline diff
#   - a router with no extractable routes fails loudly rather than passing vacuously
#
# WHY A HARNESS RATHER THAN A COMMENT
# -----------------------------------
# A guard that cannot fail is worse than no guard: it emits a green tick that means nothing. The
# tests below are the evidence that this one CAN fail, re-run on every CI run. Fixtures are
# derived from the live router.rs rather than hand-written, so they cannot rot into testing a
# shape the file no longer has.
#
#   bash .github/scripts/test-audit-mcp-route-auth.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT_SCRIPT="${SCRIPT_DIR}/audit-mcp-route-auth.sh"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
REAL_ROUTER="${REPO_ROOT}/crates/temper-mcp/src/router.rs"
PASS=0
FAIL=0

FIXTURE_DIR="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_DIR"' EXIT

# run_test NAME ROUTER_FILE EXPECTED_EXIT [EXPECTED_SUBSTRING]
#
# A fixture never matches the reviewed route BASELINE, so exit code alone cannot distinguish "the
# wiring assertion bit" from "the baseline diff tripped". EXPECTED_SUBSTRING pins the actual reason.
run_test() {
    local test_name="$1"
    local router_file="$2"
    local expected_exit="$3"
    local expected_substr="${4:-}"

    local output actual_exit
    set +e
    output="$(ROUTER_FILE="$router_file" bash "$AUDIT_SCRIPT" 2>&1)"
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

echo "Running audit-mcp-route-auth.sh tests..."
echo ""

# --- (1) the real router.rs passes: the reviewed route set, wiring present ---
run_test "real router.rs: passes" "$REAL_ROUTER" 0

# --- (2) a new public route in a NEW group: unknown posture, fails naming the group ---
NEW_GROUP="${FIXTURE_DIR}/new_group.rs"
awk '{ print } /let health = Router::new\(\)/ { print "    let admin_routes = Router::new().route(\"/admin/ping\", get(|| async { \"ok\" }));" }' "$REAL_ROUTER" > "$NEW_GROUP"
run_test "new route in new group: fails as unknown posture" "$NEW_GROUP" 1 \
    "UNKNOWN auth posture"

# --- (3) a new public route in a REVIEWED public group: the baseline diff is the trip ---
RENAMED_PATH="${FIXTURE_DIR}/renamed_public_path.rs"
sed 's#"/mcp/health"#"/mcp/health-v2"#' "$REAL_ROUTER" > "$RENAMED_PATH"
run_test "public path changed: fails baseline diff" "$RENAMED_PATH" 1 \
    "route set changed"

# --- (4) require_mcp_auth deleted from the mcp_routes block: the wiring assertion ---
# The `,` keeps the use-line (which ends in `;`) out of the deletion.
NO_AUTH="${FIXTURE_DIR}/no_require_mcp_auth.rs"
sed '/require_mcp_auth,/d' "$REAL_ROUTER" > "$NO_AUTH"
run_test "require_mcp_auth deleted: fails wiring" "$NO_AUTH" 1 \
    "'require_mcp_auth' not mounted in the mcp_routes"

# --- (5) the nest retargeted: the auth layer no longer rides /mcp ---
NO_NEST="${FIXTURE_DIR}/no_nest.rs"
sed 's#"/mcp", mcp_service#"/mcp-v2", mcp_service#' "$REAL_ROUTER" > "$NO_NEST"
run_test "nest_service(\"/mcp\" retargeted: fails wiring" "$NO_NEST" 1 \
    "'nest_service(\"/mcp\"' not present in the mcp_routes"

# --- (6) the mcp_routes declaration renamed: fails loudly, never silently skips ---
RENAMED_BLOCK="${FIXTURE_DIR}/renamed_block.rs"
sed 's/let mcp_routes/let mcp_service_routes/' "$REAL_ROUTER" > "$RENAMED_BLOCK"
run_test "mcp_routes renamed: fails loudly" "$RENAMED_BLOCK" 1 \
    "sub-router group 'mcp_routes' not found"

# --- (7) the handler behind a reviewed public path swapped: baseline diff, path alone insufficient ---
SWAPPED_HANDLER="${FIXTURE_DIR}/swapped_handler.rs"
sed 's/discovery::oauth_protected_resource/discovery::some_other_metadata/' "$REAL_ROUTER" > "$SWAPPED_HANDLER"
run_test "handler swapped behind reviewed path: fails baseline diff" "$SWAPPED_HANDLER" 1 \
    "route set changed"

# --- (8) no extractable routes: vacuous green is a failure mode ---
EMPTY_ROUTER="${FIXTURE_DIR}/empty.rs"
: > "$EMPTY_ROUTER"
run_test "router with no build_router: fails loudly" "$EMPTY_ROUTER" 1 \
    "no routes extracted"

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
