#!/usr/bin/env bash
# .github/scripts/test-audit-route-auth.sh
#
# Test harness for audit-route-auth.sh's (b) WIRING assertions. Runs the auditor against the real
# routes.rs and against fixtures DERIVED from it by deleting exactly one layer mount, asserting the
# auditor fails and names the right builder.
#
# WHY A HARNESS RATHER THAN A COMMENT
# -----------------------------------
# The wiring assertion used to be a whole-file `grep -q` for each layer's name. Every signature
# gate is mounted TWICE — in `create_app` AND in `create_internal_app` — so deleting one mount left
# the name present and the auditor GREEN, while one deployed surface served that route group
# unauthenticated. A guard that cannot fail is worse than no guard: it emits a green tick that
# means nothing. The tests below are the evidence that this one CAN fail, re-run on every CI run.
#
# Fixtures are derived from the live routes.rs rather than hand-written, so they cannot rot into
# testing a shape the file no longer has.
#
#   bash .github/scripts/test-audit-route-auth.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT_SCRIPT="${SCRIPT_DIR}/audit-route-auth.sh"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
REAL_ROUTES="${REPO_ROOT}/crates/temper-api/src/routes.rs"
PASS=0
FAIL=0

FIXTURE_DIR="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_DIR"' EXIT

# run_test NAME ROUTES_FILE EXPECTED_EXIT [EXPECTED_SUBSTRING]
#
# A fixture never matches the reviewed route BASELINE, so exit code alone cannot distinguish "the
# wiring assertion bit" from "the baseline diff tripped". EXPECTED_SUBSTRING pins the actual reason.
run_test() {
    local test_name="$1"
    local routes_file="$2"
    local expected_exit="$3"
    local expected_substr="${4:-}"

    local output actual_exit
    set +e
    output="$(ROUTES_FILE="$routes_file" bash "$AUDIT_SCRIPT" 2>&1)"
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

# drop_layer_in BUILDER LAYER OUTFILE — copy the real routes.rs, deleting the line that mounts
# LAYER inside BUILDER's body only. Models a mount removed from ONE app builder: the exact edit the
# old whole-file grep could not see.
drop_layer_in() {
    awk -v fname="$1" -v layer="$2" '
        $0 ~ "^(pub )?fn "fname"\\(" { inside = 1 }
        inside && index($0, layer) > 0 { next }
        inside && /^\}/ { inside = 0 }
        { print }
    ' "$REAL_ROUTES" > "$3"
}

echo "Running audit-route-auth.sh wiring tests..."
echo ""

# --- (a) the real routes.rs passes: every layer mounted in every builder that serves it ---
run_test "real routes.rs: passes" "$REAL_ROUTES" 0

# --- (b) each signature gate dropped from create_internal_app ONLY must fail ---
# This is the regression the whole-file grep missed: the layer name is still present (create_app
# still mounts it), yet the internal Vercel function would serve the group ungated.
for layer in require_internal_signature require_slack_link_signature require_slack_mint_signature; do
    FIX="${FIXTURE_DIR}/internal_no_${layer}.rs"
    drop_layer_in create_internal_app "$layer" "$FIX"
    run_test "${layer} dropped from create_internal_app only: fails" "$FIX" 1 \
        "'${layer}' not mounted in create_internal_app()"
done

# --- (c) each signature gate dropped from create_app ONLY must fail, symmetrically ---
for layer in require_internal_signature require_slack_link_signature require_slack_mint_signature; do
    FIX="${FIXTURE_DIR}/public_no_${layer}.rs"
    drop_layer_in create_app "$layer" "$FIX"
    run_test "${layer} dropped from create_app only: fails" "$FIX" 1 \
        "'${layer}' not mounted in create_app()"
done

# --- (d) the user-auth layers, dropped from create_app ---
for layer in 'auth::require_auth' 'require_system_access'; do
    FIX="${FIXTURE_DIR}/no_$(echo "$layer" | tr -c 'a-zA-Z0-9' '_').rs"
    drop_layer_in create_app "$layer" "$FIX"
    run_test "${layer} dropped from create_app: fails" "$FIX" 1 \
        "'${layer}' not mounted in create_app()"
done

# --- (e) a renamed/removed app builder is caught rather than silently skipped ---
# An empty body would make every `grep -q` in it vacuously... absent. Assert the auditor says so
# explicitly instead of reporting a confusing per-layer miss for a builder that does not exist.
RENAMED="${FIXTURE_DIR}/renamed_builder.rs"
sed 's/^pub fn create_internal_app(/pub fn create_system_app(/' "$REAL_ROUTES" > "$RENAMED"
run_test "create_internal_app renamed: fails loudly" "$RENAMED" 1 \
    "app builder 'create_internal_app' not found"

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
