#!/usr/bin/env bash
# .github/scripts/test-check-release-chain-triggers.sh
#
# Test harness for check-release-chain-triggers.sh. Every case copies the three real chain
# workflows into a temp tree, mutates the copy, and runs the gate against it with --root —
# nothing here edits the working tree's workflows.
#
# Beyond exit codes it asserts the gate is WIRED on an uncommented line of quality-gate.yml,
# in guard-tests, which runs on every change: a workflows-only PR is exactly the change that
# would add the trigger.
#
#   bash .github/scripts/test-check-release-chain-triggers.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
GATE="${SCRIPT_DIR}/check-release-chain-triggers.sh"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

TREE=""
make_tree() {
    TREE="${WORK}/$1"
    mkdir -p "${TREE}/.github/workflows"
    for f in release-tag.yml release.yml build-cli-binaries.yml; do
        cp "${REPO_ROOT}/.github/workflows/${f}" "${TREE}/.github/workflows/"
    done
}

# insert_after FILE PATTERN TEXT — add TEXT as a new line after the first line matching PATTERN.
insert_after() {
    awk -v pat="$2" -v txt="$3" '{ print } !done && $0 ~ pat { print txt; done = 1 }' "$1" > "$1.tmp" && mv "$1.tmp" "$1"
}

run_case() {
    local name="$1" expected="$2" needle="${3:-}"
    local out actual=0
    out="$(bash "$GATE" --root "$TREE" 2>&1)" || actual=$?
    if [ "$actual" != "$expected" ]; then
        echo "  FAIL: ${name} (expected exit ${expected}, got ${actual})"; echo "$out" | sed 's/^/        /'
        FAIL=$((FAIL + 1)); return
    fi
    if [ -n "$needle" ] && ! printf '%s' "$out" | grep -qF -- "$needle"; then
        echo "  FAIL: ${name} (output lacks '${needle}')"; echo "$out" | sed 's/^/        /'
        FAIL=$((FAIL + 1)); return
    fi
    echo "  PASS: ${name}"; PASS=$((PASS + 1))
}

echo "-- the committed chain"
make_tree clean
run_case "the committed workflows pass" 0

echo "-- a new trigger on any chain workflow fails"
make_tree mg-tag
insert_after "${TREE}/.github/workflows/release-tag.yml" '^on:' '  merge_group:'
run_case "merge_group on release-tag.yml" 1 "release-tag.yml triggers on [merge_group push]"

make_tree mg-release
insert_after "${TREE}/.github/workflows/release.yml" '^on:' '  merge_group:'
run_case "merge_group on release.yml" 1 "release.yml triggers on"

make_tree pr-build
insert_after "${TREE}/.github/workflows/build-cli-binaries.yml" '^on:' '  pull_request:'
run_case "pull_request on build-cli-binaries.yml" 1 "build-cli-binaries.yml triggers on"

make_tree commented
insert_after "${TREE}/.github/workflows/release-tag.yml" '^on:' '  # merge_group:'
run_case "a commented-out trigger is not a trigger" 0

echo "-- release-tag.yml pushes from main alone"
make_tree branch-list
insert_after "${TREE}/.github/workflows/release-tag.yml" '^      - main' '      - release/*'
run_case "a second push branch fails" 1 "expected [main] alone"

make_tree flow-ok
sed -i '/^    branches:$/{N;s/^    branches:\n      - main$/    branches: [main]/}' "${TREE}/.github/workflows/release-tag.yml"
run_case "flow-style [main] passes" 0

make_tree flow-bad
sed -i '/^    branches:$/{N;s/^    branches:\n      - main$/    branches: [main, dev]/}' "${TREE}/.github/workflows/release-tag.yml"
run_case "flow-style [main, dev] fails" 1 "expected [main] alone"

echo "-- a scan that sees nothing does not report clean"
make_tree one-line
sed -i 's/^on:$/on: workflow_dispatch/' "${TREE}/.github/workflows/build-cli-binaries.yml"
run_case "an unparseable on: block fails" 1 "no events parsed"

make_tree missing
rm "${TREE}/.github/workflows/release.yml"
run_case "a missing chain workflow fails" 1 "release.yml is missing"

echo "-- wiring"
if grep -qE '^[[:space:]]+run: bash \.github/scripts/check-release-chain-triggers\.sh$' "${REPO_ROOT}/.github/workflows/quality-gate.yml"; then
    echo "  PASS: quality-gate.yml runs the gate"; PASS=$((PASS + 1))
else
    echo "  FAIL: quality-gate.yml does not run check-release-chain-triggers.sh on an uncommented line"; FAIL=$((FAIL + 1))
fi

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
