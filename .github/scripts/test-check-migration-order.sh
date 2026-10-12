#!/usr/bin/env bash
# .github/scripts/test-check-migration-order.sh
#
# Test harness for check-migration-order.sh.
#
# Every case builds a SYNTHETIC git repository in a temp dir with the real script copied in: a base
# commit, then a "branch" state in the index. Nothing here touches the working tree.
#
# Beyond exit codes, each red case asserts the OUTPUT names the offending file, so a guard that
# crashes on bad input cannot pass as a guard that caught it. And the gate's WIRING is asserted: an
# uncommented CI step in guard-tests (which runs on every change) and a call in the pre-commit hook.
#
#   bash .github/scripts/test-check-migration-order.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
GATE="${SCRIPT_DIR}/check-migration-order.sh"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# make_repo NAME VERSION... — a repo whose base commit (tagged `base`) holds one migration per
# VERSION. The returned root is in the global TREE; the index then starts equal to the base.
TREE=""
make_repo() {
    TREE="${WORK}/$1"
    shift
    mkdir -p "${TREE}/.github/scripts" "${TREE}/migrations/templates"
    cp "$GATE" "${TREE}/.github/scripts/"
    echo "-- not a migration" > "${TREE}/migrations/templates/system_initialization.sql"
    local v
    for v in "$@"; do echo "SELECT 1;" > "${TREE}/migrations/${v}_m${v}.sql"; done
    (
        cd "$TREE" && git init -q . && git add -A &&
            git -c user.email=t@t -c user.name=t commit -qm base && git tag base
    ) >/dev/null 2>&1
}

# add_migration VERSION [NAME] — stage a migration in the index (the "branch" side).
add_migration() {
    echo "SELECT 2;" > "${TREE}/migrations/${1}_${2:-added}.sql"
    (cd "$TREE" && git add -A) >/dev/null 2>&1
}

# run_case NAME EXPECTED_EXIT [NEEDLE] [ENV...] — run the gate against the `base` tag.
run_case() {
    local name="$1" expected="$2" needle="${3:-}"
    shift 3 || shift $#
    local out actual=0
    out="$(cd "$TREE" && env "$@" bash .github/scripts/check-migration-order.sh base 2>&1)" || actual=$?
    if [ "$actual" != "$expected" ]; then
        echo "FAIL: ${name}: exit ${actual}, expected ${expected}"
        echo "$out" | sed 's/^/    /'
        FAIL=$((FAIL + 1))
        return
    fi
    if [ -n "$needle" ] && ! grep -qF -- "$needle" <<<"$out"; then
        echo "FAIL: ${name}: output does not name '${needle}'"
        echo "$out" | sed 's/^/    /'
        FAIL=$((FAIL + 1))
        return
    fi
    echo "PASS: ${name}"
    PASS=$((PASS + 1))
}

make_repo above 20261025100000 20261026100000
add_migration 20261027100000
run_case "a migration above the base's newest passes" 0 "OK: 1 added"

make_repo below 20261025100000 20261027100000
add_migration 20261026100000 slack_vault
run_case "a migration below the base's newest fails, naming it" 1 "migrations/20261026100000_slack_vault.sql"

make_repo equal 20261025100000 20261027100000
add_migration 20261027100000 other
run_case "a migration equal to the base's newest fails" 1 "migrations/20261027100000_other.sql"

make_repo none 20261025100000 20261027100000
run_case "no added migration passes" 0 "OK: 0 added"

make_repo behind 20261025100000 20261026100000 20261027100000
(cd "$TREE" && git rm -q --cached migrations/20261027100000_m20261027100000.sql) >/dev/null 2>&1
add_migration 20261028100000
run_case "a branch behind its base (missing a base migration) is judged on what it adds" 0 "OK: 1 added"

make_repo short 20261025100000
add_migration 9999999999999 short
run_case "a shorter version does not slip past a lexical comparison" 1 "migrations/9999999999999_short.sql"

make_repo dupes 20261025100000
add_migration 20261026100000 one
add_migration 20261026100000 two
run_case "two files sharing a version fail, naming both" 1 "migrations/20261026100000_two.sql"

make_repo nobase 20261025100000
(cd "$TREE" && git tag -d base) >/dev/null 2>&1
run_case "an unresolvable base fails in CI" 2 "cannot resolve" GITHUB_ACTIONS=true
run_case "an unresolvable base only warns outside CI" 0 "NOT checked"

# --- wiring ---------------------------------------------------------------------------------
wired() {
    local name="$1" file="$2" pattern="$3"
    if grep -vE '^[[:space:]]*#' "$file" | grep -qF -- "$pattern"; then
        echo "PASS: ${name}"
        PASS=$((PASS + 1))
    else
        echo "FAIL: ${name}: no uncommented '${pattern}' in ${file#"$REPO_ROOT"/}"
        FAIL=$((FAIL + 1))
    fi
}
wired "the gate runs in CI" "${REPO_ROOT}/.github/workflows/quality-gate.yml" "bash .github/scripts/check-migration-order.sh"
wired "its harness runs in CI" "${REPO_ROOT}/.github/workflows/quality-gate.yml" "bash .github/scripts/test-check-migration-order.sh"
# The base must reach THIS step: other steps in the file set GITHUB_BASE_SHA too, so a file-wide
# grep would pass with the gate's own env block deleted. Without it, a PR run exits 0 as a "push".
gate_line="$(grep -nF 'run: bash .github/scripts/check-migration-order.sh' "${REPO_ROOT}/.github/workflows/quality-gate.yml" | head -1 | cut -d: -f1)"
if [ -n "$gate_line" ] && sed -n "$((gate_line - 3)),$((gate_line - 1))p" "${REPO_ROOT}/.github/workflows/quality-gate.yml" |
    grep -qF 'GITHUB_BASE_SHA: ${{ github.event.pull_request.base.sha || github.event.merge_group.base_sha }}'; then
    echo "PASS: CI hands the gate step the base revision"
    PASS=$((PASS + 1))
else
    echo "FAIL: CI hands the gate step the base revision: no GITHUB_BASE_SHA env on its step"
    FAIL=$((FAIL + 1))
fi
wired "the pre-commit hook runs it" "${REPO_ROOT}/githooks/pre-commit" "check-migration-order.sh"

echo
echo "check-migration-order: ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
