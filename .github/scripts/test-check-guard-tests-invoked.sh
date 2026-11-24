#!/usr/bin/env bash
# .github/scripts/test-check-guard-tests-invoked.sh
#
# Bite test for check-guard-tests-invoked.sh. A guard test the workflow does not run, in either
# language, fails and is named. Wired tests pass. Commented, multi-line and lookalike invocations
# do not count. An empty scan and a missing workflow fail.
#
#   bash .github/scripts/test-check-guard-tests-invoked.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CHECK="${SCRIPT_DIR}/check-guard-tests-invoked.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
PASS=0
FAIL=0

# run_case NAME EXPECTED_EXIT [EXPECTED_SUBSTRING]: runs the check over $TMP/scripts and $TMP/wf.yml.
run_case() {
    local name="$1" want="$2" substr="${3:-}" out rc
    set +e
    out="$(GUARD_TESTS_SCRIPTS_DIR="$TMP/scripts" GUARD_TESTS_WORKFLOW="$TMP/wf.yml" bash "$CHECK" 2>&1)"
    rc=$?
    set -e
    if [[ "$rc" -ne "$want" ]]; then
        echo "  FAIL: ${name} (want exit ${want}, got ${rc}): ${out}"; FAIL=$((FAIL + 1)); return
    fi
    if [[ -n "$substr" && "$out" != *"$substr"* ]]; then
        echo "  FAIL: ${name} (exit matched, '${substr}' not in output): ${out}"; FAIL=$((FAIL + 1)); return
    fi
    echo "  PASS: ${name}"; PASS=$((PASS + 1))
}

# fixture: test-a.sh and test-b.py, both wired.
reset() {
    rm -rf "$TMP/scripts"; mkdir -p "$TMP/scripts"
    : > "$TMP/scripts/test-a.sh"; : > "$TMP/scripts/test-b.py"; : > "$TMP/scripts/audit-x.sh"
    cat > "$TMP/wf.yml" <<'YAML'
jobs:
  guard-tests:
    steps:
      - name: a
        run: bash .github/scripts/test-a.sh
      - name: b
        run: python3 .github/scripts/test-b.py
YAML
}

echo "Running check-guard-tests-invoked.sh tests..."

reset
run_case "every guard test wired: passes" 0 "all 2 guard tests"

reset
: > "$TMP/scripts/test-new.sh"
run_case "an unwired .sh guard test: fails, naming it" 1 "test-new.sh"

reset
: > "$TMP/scripts/test-new.py"
run_case "an unwired .py guard test: fails, naming it" 1 "test-new.py"

reset
sed -i.bak 's|^        run: bash .github/scripts/test-a.sh|        # run: bash .github/scripts/test-a.sh|' "$TMP/wf.yml"
run_case "a commented-out step does not count" 1 "test-a.sh"

reset
sed -i.bak 's|run: bash .github/scripts/test-a.sh|run: bash .github/scripts/test-a.sh.disabled|' "$TMP/wf.yml"
run_case "a lookalike file name does not count" 1 "test-a.sh"

reset
sed -i.bak 's|run: bash .github/scripts/test-a.sh|run: bash .github/scripts/test-aXsh|' "$TMP/wf.yml"
run_case "a '.' in the name is matched literally" 1 "test-a.sh"

reset
sed -i.bak 's|run: python3 .github/scripts/test-b.py|run: bash .github/scripts/test-b.py|' "$TMP/wf.yml"
run_case "a .py run by bash does not count" 1 "test-b.py"

reset
python3 - "$TMP/wf.yml" <<'PYEOF'
import sys
p = sys.argv[1]
s = open(p).read()
s = s.replace("        run: bash .github/scripts/test-a.sh\n",
              "        run: |\n          bash .github/scripts/test-a.sh\n")
open(p, "w").write(s)
PYEOF
run_case "a multi-line run: block does not count" 1 "test-a.sh"

reset
python3 - "$TMP/wf.yml" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
s = s.replace("      - name: a\n        run: bash .github/scripts/test-a.sh\n",
              "      - run: bash .github/scripts/test-a.sh\n")
open(p, "w").write(s)
PY
run_case "a step opened by its run: line counts" 0 "all 2 guard tests"

reset
rm -f "$TMP/scripts/test-a.sh" "$TMP/scripts/test-b.py"
run_case "an empty scan: fails" 1 "found no test-*.sh or test-*.py"

reset
rm -f "$TMP/wf.yml"
run_case "a missing workflow: fails" 1 "not found"

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed"
[[ "$FAIL" -eq 0 ]]
