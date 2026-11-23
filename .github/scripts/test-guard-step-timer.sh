#!/usr/bin/env bash
# .github/scripts/test-guard-step-timer.sh
#
# Witnesses for guard-step-timer.sh, the shell guard-tests' steps run under. It must:
#   - pass a step's exit code through unchanged, under the semantics GitHub gives a step that names
#     no shell (`bash -e`: errexit on, pipefail off);
#   - warn, naming the step, when the step outlasts the budget, and stay silent when it does not;
#   - never turn a slow step into a failing one;
#   - refuse (exit 2, step not run) when the budget is unset or not a whole number.
#
#   bash .github/scripts/test-guard-step-timer.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TIMER="${SCRIPT_DIR}/guard-step-timer.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
PASS=0
FAIL=0

ok() { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; FAIL=$((FAIL + 1)); }

# step NAME BODY: write a step script the way GitHub does, one file holding the run: text.
step() { printf '%s\n' "$2" > "$TMP/$1"; printf '%s' "$TMP/$1"; }

# run BUDGET SCRIPT: sets OUT and RC. BUDGET "unset" runs with the variable unset.
run() {
    set +e
    if [[ "$1" == "unset" ]]; then
        OUT="$(env -u GUARD_STEP_BUDGET_SECS bash "$TIMER" "$2" 2>&1)"
    else
        OUT="$(GUARD_STEP_BUDGET_SECS="$1" bash "$TIMER" "$2" 2>&1)"
    fi
    RC=$?
    set -e
}

echo "Running guard-step-timer.sh tests..."

# --- exit codes pass through, under bash -e ---
run 600 "$(step ok.sh 'true')"
if [[ $RC -eq 0 && "$OUT" != *"::warning"* ]]; then
    ok "a fast passing step exits 0, silently"
else
    bad "a fast passing step: rc=$RC out=$OUT"
fi

run 600 "$(step exit3.sh 'exit 3')"
if [[ $RC -eq 3 ]]; then
    ok "a step's exit code passes through (3)"
else
    bad "exit 3 became rc=$RC"
fi

run 600 "$(step errexit.sh $'false\necho reached')"
if [[ $RC -ne 0 && "$OUT" != *reached* ]]; then
    ok "-e: a failing command stops the step"
else
    bad "-e not in force: rc=$RC out=$OUT"
fi

# Pipefail stays off, as it is for a step with no `shell:`: turning it on would change what an
# existing step means (an early-exiting `grep -q` reader would fail its writer by SIGPIPE).
run 600 "$(step pipefail.sh 'false | true')"
if [[ $RC -eq 0 ]]; then
    ok "no pipefail: only a pipeline's last stage decides, as with no shell: named"
else
    bad "pipefail is in force, unlike GitHub's default: rc=$RC"
fi

# --- the warning: over budget names the step; it never fails a passing step ---
SLOW="$(step slow.sh $'# a comment line, not the name\n\nsleep 2 && echo done')"
run 1 "$SLOW"
if [[ $RC -eq 0 ]]; then
    ok "a slow passing step still exits 0"
else
    bad "a slow step became rc=$RC"
fi
if [[ "$OUT" == *"::warning title=Slow guard step::sleep 2 && echo done took "*"over the 1s per-step budget"* ]]; then
    ok "a step over budget is warned about, by its first command line"
else
    bad "no named warning for a slow step: $OUT"
fi

run 600 "$SLOW"
if [[ "$OUT" != *"::warning"* ]]; then
    ok "the same step under its budget is not warned about"
else
    bad "warned under budget: $OUT"
fi

run 0 "$(step slowfail.sh $'sleep 1\nexit 4')"
if [[ $RC -eq 4 && "$OUT" == *"::warning"*"exited 4."* ]]; then
    ok "a slow failing step warns and keeps its exit code"
else
    bad "slow failing step: rc=$RC out=$OUT"
fi

run 0 "$(step pct.sh 'sleep 1 && echo 100%')"
if [[ "$OUT" == *"echo 100%25 took"* && "$OUT" != *"100% took"* ]]; then
    ok "a % in the step name is escaped for the workflow command"
else
    bad "% not escaped: $OUT"
fi

# --- a broken budget refuses the step without running it ---
for b in unset "" "60s" "-1" "1.5"; do
    rm -f "$TMP/ran"
    run "$b" "$(step marker.sh "touch '$TMP/ran'")"
    if [[ $RC -eq 2 && ! -e "$TMP/ran" && "$OUT" == *"GUARD_STEP_BUDGET_SECS must be a whole number"* ]]; then
        ok "budget '${b}' is refused before the step runs"
    else
        bad "budget '${b}': rc=$RC ran=$([[ -e "$TMP/ran" ]] && echo yes || echo no) out=$OUT"
    fi
done

run 600 "$TMP/does-not-exist.sh"
if [[ $RC -eq 2 ]]; then
    ok "a missing step script is refused"
else
    bad "missing script: rc=$RC"
fi

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed"
[[ $FAIL -eq 0 ]]
