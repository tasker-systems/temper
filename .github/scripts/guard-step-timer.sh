#!/usr/bin/env bash
# .github/scripts/guard-step-timer.sh
#
# The shell every `run:` step of quality-gate.yml's guard-tests job runs under (the job's
# `defaults.run.shell`). It runs the step as GitHub runs a step that names no shell (`bash -e
# <script>`: errexit, and no pipefail, which only an explicit `shell: bash` adds), exits with the
# step's own exit code, and measures the step's wall time. A step that takes longer than
# GUARD_STEP_BUDGET_SECS gets a `::warning::` annotation naming it. It never fails a step for being
# slow.
#
# WHY. The job runs every harness serially under one `timeout-minutes`. When a harness grows or a
# runner is slow, the job is cancelled after its harnesses have passed, and the cancellation names
# nothing. The annotation names the slow step on every run that is merely slow, before any run is
# cancelled.
#
# A step is named by the first line of its script that is neither blank nor a comment: for a
# guard-tests step, the `bash .github/scripts/<harness>.sh` command it runs.
#
# NOT CHECKED: the job's total time. The budget is per step. The job's own `timeout-minutes` is the
# only bound on the total.
#
# GUARD_STEP_BUDGET_SECS must be a whole number of seconds, written without a leading zero (bash
# arithmetic reads `08` as bad octal). When it is unset or anything else, the step is refused
# (exit 2) without running, so a broken budget cannot pass for "nothing was slow".
#
#   bash .github/scripts/guard-step-timer.sh <step-script>

set -euo pipefail

script="${1:-}"
if [[ -z "$script" || ! -f "$script" ]]; then
    echo "guard-step-timer: no step script given, or '${script}' is not a file." >&2
    exit 2
fi
budget="${GUARD_STEP_BUDGET_SECS-}"
if ! [[ "$budget" =~ ^(0|[1-9][0-9]*)$ ]]; then
    echo "guard-step-timer: GUARD_STEP_BUDGET_SECS must be a whole number of seconds; got '${budget}'." >&2
    exit 2
fi

name="$(grep -m1 -vE '^[[:space:]]*(#|$)' "$script" || true)"
name="${name#"${name%%[![:space:]]*}"}"
name="${name:0:160}"
[[ -n "$name" ]] || name="(empty step)"

start=$SECONDS
set +e
bash -e "$script"
rc=$?
set -e
elapsed=$((SECONDS - start))

if (( elapsed > budget )); then
    # Workflow-command escaping: the message may carry `%`, and a name never carries a newline.
    msg="${name//%/%25} took ${elapsed}s, over the ${budget}s per-step budget (GUARD_STEP_BUDGET_SECS); the step itself exited ${rc}."
    # A workflow command must start its own line, and the step's last output may lack a newline.
    echo ""
    echo "::warning title=Slow guard step::${msg}"
fi
exit "$rc"
