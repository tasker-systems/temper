#!/usr/bin/env bash
# .github/scripts/check-guard-tests-invoked.sh
#
# Every guard test under .github/scripts (`test-*.sh`, `test-*.py`) is run by quality-gate.yml: on
# an uncommented one-line `run:` that invokes it directly, as `run: bash .github/scripts/<file>` or
# `run: python3 .github/scripts/<file>` (or the same line opening a step, `- run: ...`).
#
# WHY. A guard test is the proof that its guard can still fail (TW-5). A guard test that CI never
# runs proves nothing, however green it is locally. The set is derived by glob, never listed,
# because a hand-kept list of guards once shrank from twelve to five without anyone noticing.
#
# An empty glob FAILS: a check over zero files passes while asserting nothing, and the cause is
# mundane (a moved directory, a renamed prefix).
#
# NOT CHECKED:
#   - that the step runs unconditionally. A step with an `if:`, or in a job with an `if:`, still
#     counts as invoked.
#   - that the step can fail the job. A step with `continue-on-error: true` still counts.
#   - that the step sits in the guard-tests job. Any job in the workflow counts.
#   - that a matching line is a step's `run:`. Text inside a `run: |` block that only looks like
#     one (an indented `run: bash .github/scripts/<file>` inside an echo or heredoc) counts.
#   - an invocation spelled any other way (a multi-line `run: |` block, a variable, a loop). It
#     reads as missing: write the step as a one-line `run:`.
#   - guard tests outside .github/scripts, such as tools/roadmap/test.sh.
#
# GUARD_TESTS_SCRIPTS_DIR and GUARD_TESTS_WORKFLOW point the check at a fixture. The harness is
# test-check-guard-tests-invoked.sh.
#
#   bash .github/scripts/check-guard-tests-invoked.sh

set -euo pipefail

SCRIPTS_DIR="${GUARD_TESTS_SCRIPTS_DIR:-.github/scripts}"
WORKFLOW="${GUARD_TESTS_WORKFLOW:-.github/workflows/quality-gate.yml}"

if [[ ! -f "$WORKFLOW" ]]; then
    echo "check-guard-tests-invoked: FAIL — workflow ${WORKFLOW} not found." >&2
    exit 1
fi

found=0
missing=()
for path in "${SCRIPTS_DIR}"/test-*.sh "${SCRIPTS_DIR}"/test-*.py; do
    [[ -e "$path" ]] || continue
    found=$((found + 1))
    file="$(basename "$path")"
    case "$file" in
        *.sh) runner=bash ;;
        *.py) runner=python3 ;;
    esac
    # Literal match on the file name: escape the regex metacharacters a file name can carry.
    escaped="$(printf '%s' "$file" | sed 's/[.[\*^$+?(){}|]/\\&/g')"
    if ! grep -qE "^[[:space:]]+(-[[:space:]]+)?run: ${runner} \\.github/scripts/${escaped}[[:space:]]*$" "$WORKFLOW"; then
        missing+=("$file")
    fi
done

if [[ "$found" -eq 0 ]]; then
    echo "check-guard-tests-invoked: FAIL — found no test-*.sh or test-*.py under ${SCRIPTS_DIR}." >&2
    echo "  A check over zero files asserts nothing. Either the directory moved or the prefix changed." >&2
    exit 1
fi

if [[ "${#missing[@]}" -gt 0 ]]; then
    echo "check-guard-tests-invoked: FAIL — these guard tests exist and ${WORKFLOW} does not run them:" >&2
    printf '  %s\n' "${missing[@]}" >&2
    echo "  A guard test CI never runs proves nothing. Add a one-line step to the guard-tests job," >&2
    echo "  \`run: bash .github/scripts/<file>\` (or \`python3\` for a .py), or delete the test." >&2
    exit 1
fi

echo "check-guard-tests-invoked: OK — all ${found} guard tests under ${SCRIPTS_DIR} are run by ${WORKFLOW}."
