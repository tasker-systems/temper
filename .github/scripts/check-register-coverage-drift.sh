#!/usr/bin/env bash
#
# Fail if `internal/registers/coverage.yaml` drifts from what the projection would produce now.
#
# ## What this gate is, and how it differs from every sibling in this directory
#
# The other `check-*-drift.sh` scripts project REPO content — ts-rs types, the OpenAPI document, the
# skills tree — so they build from source and compare against the tree under review. Their check is
# hermetic: same tree in, same answer out, on any machine.
#
# This one cannot be. Goals, clauses and citations live in a remote knowledge base with no in-tree
# representation, so the projection reads BOTH the repo (does this test exist?) and the remote (what
# does this register claim?). Two consequences, and neither is a defect to be engineered away:
#
#   1. **A diff here can mean the REMOTE changed**, not the repository — the opposite of what drift
#      means everywhere else in this directory. A red run is a prompt to look, not proof of a bad
#      commit.
#   2. **Without the remote there is no check at all.** It cannot be made hermetic by trying harder.
#
# ## Why an unreachable source SKIPS rather than fails
#
# A source that cannot be reached is not evidence of drift, and failing on it would red every fork,
# every offline run, and every CI job without vault credentials — which trains people to ignore the
# gate, the one outcome that makes it worthless.
#
# **But a skip is NOT a pass, and this script never lets one read as one.** It says SKIPPED, names
# the reason, and says in the output that nothing was verified. Silence is the failure mode this
# whole area exists to catch; a gate that goes quietly green while checking nothing would be the
# instrument committing the defect it detects.
#
# Usage: bash .github/scripts/check-register-coverage-drift.sh
#
# REGISTER_COVERAGE_REPO_ROOT / REGISTER_COVERAGE_RUN_CMD are harness seams for the test script.

set -uo pipefail

REPO_ROOT="${REGISTER_COVERAGE_REPO_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
ARTIFACT="internal/registers/coverage.yaml"

skip() {
  echo "SKIPPED: register-coverage drift not checked — $1"
  echo "         A SKIP IS NOT A PASS. Nothing about ${ARTIFACT} was verified by this run."
  exit 0
}

if [[ -n "${REGISTER_COVERAGE_RUN_CMD:-}" ]]; then
  # shellcheck disable=SC2206
  RUN_CMD=( ${REGISTER_COVERAGE_RUN_CMD} )
else
  command -v uv >/dev/null 2>&1 || skip "uv is not installed (see tools/pyproject.toml)"
  command -v temper >/dev/null 2>&1 || skip "the temper CLI is not on PATH, so the remote cannot be read"
  RUN_CMD=( uv run --project "${REPO_ROOT}/tools" register-projection )
fi

"${RUN_CMD[@]}" --repo-root "${REPO_ROOT}" --out "${ARTIFACT}" --check
status=$?

case "${status}" in
  0)
    echo "OK: ${ARTIFACT} matches the projection."
    exit 0
    ;;
  2)
    # The tool's own "I could not read the source" code, distinct from "I read it and it differs".
    skip "the knowledge base could not be read (see the message above)"
    ;;
  *)
    echo ""
    echo "DRIFT: ${ARTIFACT} does not match what the projection produces now."
    echo ""
    echo "  Regenerate:  uv run --project tools register-projection"
    echo ""
    echo "  Read the diff before assuming this repository is at fault. The projection reads a"
    echo "  REMOTE knowledge base as well as this tree, so a register edited elsewhere moves this"
    echo "  file without any commit here touching it."
    exit "${status}"
    ;;
esac
