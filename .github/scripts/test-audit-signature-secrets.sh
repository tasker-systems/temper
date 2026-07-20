#!/usr/bin/env bash
# .github/scripts/test-audit-signature-secrets.sh
#
# Test harness for audit-signature-secrets.sh. Runs the auditor against the real internal_auth.rs
# and against fixtures derived from it, asserting exit code AND failure reason.
#
# The load-bearing test is (d): a reviewer who "acknowledges" a secret collapse by running
# UPDATE_BASELINE must STILL fail. A tripwire whose baseline can absorb the defect it exists to
# catch is a tripwire that will be stepped over exactly once, by the person in a hurry.
#
#   bash .github/scripts/test-audit-signature-secrets.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT_SCRIPT="${SCRIPT_DIR}/audit-signature-secrets.sh"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
REAL_MW="${REPO_ROOT}/crates/temper-api/src/middleware/internal_auth.rs"
PASS=0
FAIL=0

FIXTURE_DIR="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_DIR"' EXIT

# run_test NAME MIDDLEWARE_FILE EXPECTED_EXIT [EXPECTED_SUBSTRING] [SCRIPT_OVERRIDE]
run_test() {
    local test_name="$1" mw="$2" expected_exit="$3" expected_substr="${4:-}" script="${5:-$AUDIT_SCRIPT}"
    local output actual_exit
    set +e
    output="$(MIDDLEWARE_FILE="$mw" bash "$script" 2>&1)"
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
        echo "    exit matched but expected message missing: ${expected_substr}"
        echo "    output: ${output}"
        FAIL=$((FAIL + 1))
        return
    fi
    echo "  PASS: ${test_name}"
    PASS=$((PASS + 1))
}

echo "Running audit-signature-secrets.sh tests..."
echo ""

# --- (a) the real middleware passes: three gates, three distinct keys ---
run_test "real internal_auth.rs: passes" "$REAL_MW" 0

# --- (b) mint collapsed onto the link gate's key: fails as a SHARED SECRET ---
# The expensive capability (act-as-the-human mint) reusing the cheap one's key (link-state).
COLLAPSE="${FIXTURE_DIR}/collapse_mint_onto_link.rs"
sed 's/slack_mint_secret/hmac_secret/' "$REAL_MW" > "$COLLAPSE"
run_test "mint gate reusing the link gate's key: fails" "$COLLAPSE" 1 "signature gates SHARE a secret"

# --- (c) the reconcile gate collapsed onto mint's key: fails symmetrically ---
COLLAPSE2="${FIXTURE_DIR}/collapse_reconcile_onto_mint.rs"
sed 's/internal_reconcile_secret/slack_mint_secret/' "$REAL_MW" > "$COLLAPSE2"
run_test "reconcile gate reusing the mint key: fails" "$COLLAPSE2" 1 "signature gates SHARE a secret"

# --- (d) THE ONE THAT MATTERS: a baseline "acknowledging" the collapse must NOT silence it ---
# Simulates a reviewer running UPDATE_BASELINE to make the red go away. The distinctness check is
# computed from the source, not diffed against the baseline, so it survives.
BLESSED="${FIXTURE_DIR}/blessed-audit.sh"
sed 's/^require_slack_mint_signature	slack_mint_secret$/require_slack_mint_signature\thmac_secret/' \
    "$AUDIT_SCRIPT" > "$BLESSED"
run_test "collapse blessed into the baseline: STILL fails" "$COLLAPSE" 1 \
    "signature gates SHARE a secret" "$BLESSED"

# --- (e) a gate repointed at a NEW distinct key: fails on the baseline diff, not distinctness ---
# Distinct keys are necessary but not sufficient — a repoint still wants a human to look.
REPOINT="${FIXTURE_DIR}/repointed.rs"
sed 's/slack_mint_secret/some_other_secret/' "$REAL_MW" > "$REPOINT"
run_test "gate repointed at a new distinct key: fails on baseline" "$REPOINT" 1 \
    "the gate -> secret mapping changed"

# --- (f) a file with no gates at all must FAIL, not pass vacuously ---
# An empty set satisfies "all pairwise distinct". If the extraction ever stops matching the gates'
# shape, this guard must go red rather than quietly asserting nothing forever.
EMPTY="${FIXTURE_DIR}/no_gates.rs"
cat > "$EMPTY" <<'EOF'
//! No signature gates here at all.
pub async fn something_else(request: Request, next: Next) -> Response {
    next.run(request).await
}
EOF
run_test "no gates found: fails rather than passing vacuously" "$EMPTY" 1 "no signature gates found"

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
