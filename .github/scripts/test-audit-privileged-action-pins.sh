#!/usr/bin/env bash
# test-audit-privileged-action-pins.sh — prove audit-privileged-action-pins.sh can fail, and fails
# for the reason each fixture breaks.
#
# Every fixture is a synthetic workflows directory in a temp dir; nothing here reads or touches the
# real tree except the first case, which runs the guard against it as-is.
#
# Usage: bash .github/scripts/test-audit-privileged-action-pins.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
GUARD="${SCRIPT_DIR}/audit-privileged-action-pins.sh"
PASS=0
FAIL=0
SHA=3d3c42e5aac5ba805825da76410c181273ba90b1

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# run_case <name> <expect: pass|fail> <needle-on-fail or ''> ; reads the fixture from $TMP/<n>
case_n=0
fixture() {
    case_n=$((case_n + 1))
    FIX="${TMP}/${case_n}"
    mkdir -p "${FIX}/.github/workflows"
}

check() {
    local name="$1" expect="$2" needle="$3" out rc=0
    out="$(PIN_AUDIT_WORKFLOWS_DIR="${FIX}/.github/workflows" PIN_AUDIT_ACTIONS_ROOT="${FIX}" \
        bash "$GUARD" 2>&1)" || rc=$?
    if [ "$expect" = pass ] && [ "$rc" -eq 0 ]; then
        echo "  PASS: ${name}"; PASS=$((PASS + 1))
    elif [ "$expect" = fail ] && [ "$rc" -ne 0 ] && echo "$out" | grep -qF -- "$needle"; then
        echo "  PASS: ${name}"; PASS=$((PASS + 1))
    else
        echo "  FAIL: ${name} (expected ${expect}${needle:+ mentioning '${needle}'}, rc=${rc})"
        echo "$out" | sed 's/^/        /'
        FAIL=$((FAIL + 1))
    fi
}

echo "Running audit-privileged-action-pins.sh tests..."

# The real tree passes.
out_rc=0
bash "$GUARD" >/dev/null 2>&1 || out_rc=$?
if [ "$out_rc" -eq 0 ]; then
    echo "  PASS: the repository's own workflows pass"; PASS=$((PASS + 1))
else
    echo "  FAIL: the repository's own workflows do not pass"; FAIL=$((FAIL + 1))
fi

fixture
cat > "${FIX}/.github/workflows/a.yml" <<EOF
permissions:
  contents: read
jobs:
  publish:
    permissions:
      id-token: write
    steps:
      - uses: actions/checkout@${SHA} # v7.0.1
      - uses: actions/setup-node@v7
EOF
check "a tag ref in a job holding id-token: write FAILS" fail "actions/setup-node@v7"

fixture
cat > "${FIX}/.github/workflows/a.yml" <<EOF
jobs:
  sign:
    permissions:
      attestations: write
    steps:
      - uses: dtolnay/rust-toolchain@stable
EOF
check "a branch ref in a job holding attestations: write FAILS" fail "dtolnay/rust-toolchain@stable"

fixture
cat > "${FIX}/.github/workflows/a.yml" <<EOF
jobs:
  publish:
    permissions:
      id-token: write
    steps:
      - uses: actions/checkout@${SHA}
EOF
check "a SHA pin with no tag trailer FAILS" fail "no \`# <tag>\` trailer"

fixture
cat > "${FIX}/.github/workflows/a.yml" <<EOF
permissions:
  contents: write
  id-token: write
jobs:
  inherits:
    steps:
      - uses: actions/checkout@v7
EOF
check "a job inheriting the workflow-level grant is checked (FAILS)" fail "job \`inherits\`"

fixture
cat > "${FIX}/.github/workflows/a.yml" <<EOF
permissions:
  id-token: write
jobs:
  narrowed:
    permissions:
      contents: read
    steps:
      - uses: actions/checkout@v7
  signer:
    permissions:
      id-token: write
    steps:
      - uses: actions/checkout@${SHA} # v7.0.1
EOF
check "a job that narrows away the grant may use a tag (passes)" pass ""

fixture
cat > "${FIX}/.github/workflows/a.yml" <<EOF
jobs:
  everything:
    permissions: write-all
    steps:
      - uses: actions/checkout@v7
EOF
check "permissions: write-all counts as privileged (FAILS)" fail "job \`everything\`"

fixture
mkdir -p "${FIX}/.github/actions/setup"
cat > "${FIX}/.github/actions/setup/action.yml" <<EOF
runs:
  using: composite
  steps:
    - uses: Swatinem/rust-cache@v2
EOF
cat > "${FIX}/.github/workflows/a.yml" <<EOF
jobs:
  publish:
    permissions:
      id-token: write
    steps:
      - uses: ./.github/actions/setup
EOF
check "a tag ref inside a composite action a privileged job runs FAILS" fail "Swatinem/rust-cache@v2"

fixture
cat > "${FIX}/.github/workflows/caller.yml" <<EOF
jobs:
  build:
    permissions:
      id-token: write
      attestations: write
    uses: ./.github/workflows/callee.yml
EOF
cat > "${FIX}/.github/workflows/callee.yml" <<EOF
on:
  workflow_call:
jobs:
  inner:
    steps:
      - uses: actions/upload-artifact@v7
EOF
check "a called workflow's permission-less job runs under the caller's grant (FAILS)" fail "actions/upload-artifact@v7"

fixture
cat > "${FIX}/.github/workflows/a.yml" <<EOF
permissions:
  contents: read
jobs:
  test:
    steps:
      - uses: actions/checkout@v7
EOF
check "no privileged job anywhere FAILS (the parser stopped seeing them)" fail "no job in"

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
