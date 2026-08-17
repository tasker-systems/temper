#!/usr/bin/env bash
#
# Harness for check-register-coverage-drift.sh.
#
# A gate is only worth having if it can both PASS and FAIL for the right reasons, so every case here
# asserts one of those and the skip cases assert that a skip is visibly a skip. The seam is
# REGISTER_COVERAGE_RUN_CMD: a stub standing in for the projection tool, which lets the exit-code
# routing be exercised without a network, credentials, or the 60-odd seconds a real run costs.
#
# Usage: bash .github/scripts/test-check-register-coverage-drift.sh

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
GATE="${SCRIPT_DIR}/check-register-coverage-drift.sh"
PASS=0
FAIL=0

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT

stub() { # $1 = exit code, $2 = message on stderr
  local path="${work}/stub-$1.sh"
  cat >"${path}" <<EOF
#!/usr/bin/env bash
echo "${2}" >&2
exit ${1}
EOF
  chmod +x "${path}"
  echo "${path}"
}

check() { # $1 = label, $2 = expected exit, $3 = expected substring, $4... = env assignments
  local label="$1" want_exit="$2" want_text="$3"
  shift 3
  local out status
  out="$(env "$@" bash "${GATE}" 2>&1)"
  status=$?
  if [[ "${status}" -ne "${want_exit}" ]]; then
    echo "FAIL: ${label} — expected exit ${want_exit}, got ${status}"
    echo "${out}" | sed 's/^/      /'
    FAIL=$((FAIL + 1))
    return
  fi
  if ! grep -qF "${want_text}" <<<"${out}"; then
    echo "FAIL: ${label} — output did not contain: ${want_text}"
    echo "${out}" | sed 's/^/      /'
    FAIL=$((FAIL + 1))
    return
  fi
  echo "ok: ${label}"
  PASS=$((PASS + 1))
}

# ── The gate can pass ───────────────────────────────────────────────────────────────────────────
check "a current artifact passes" 0 "OK:" \
  "REGISTER_COVERAGE_RUN_CMD=$(stub 0 'current')" \
  "REGISTER_COVERAGE_REPO_ROOT=${work}"

# ── The gate can fail. A gate that cannot fail is not a gate. ───────────────────────────────────
check "a stale artifact fails" 1 "DRIFT:" \
  "REGISTER_COVERAGE_RUN_CMD=$(stub 1 'stale')" \
  "REGISTER_COVERAGE_REPO_ROOT=${work}"

# ── Failing tells the reader the remote may be the cause, not this tree ─────────────────────────
check "a drift message names the remote as a possible cause" 1 "REMOTE knowledge base" \
  "REGISTER_COVERAGE_RUN_CMD=$(stub 1 'stale')" \
  "REGISTER_COVERAGE_REPO_ROOT=${work}"

# ── An unreachable source skips rather than failing ─────────────────────────────────────────────
check "an unreachable source skips" 0 "SKIPPED:" \
  "REGISTER_COVERAGE_RUN_CMD=$(stub 2 'no vault')" \
  "REGISTER_COVERAGE_REPO_ROOT=${work}"

# ── ...and the skip is never readable as a pass. This is the assertion that matters most: a gate
#    going quietly green while checking nothing is the exact defect this whole area exists to catch.
check "a skip states that nothing was verified" 0 "A SKIP IS NOT A PASS" \
  "REGISTER_COVERAGE_RUN_CMD=$(stub 2 'no vault')" \
  "REGISTER_COVERAGE_REPO_ROOT=${work}"

# ── An unexpected exit code is not swallowed into a pass ────────────────────────────────────────
check "an unrecognised failure still fails" 3 "DRIFT:" \
  "REGISTER_COVERAGE_RUN_CMD=$(stub 3 'exploded')" \
  "REGISTER_COVERAGE_REPO_ROOT=${work}"

echo ""
echo "${PASS} passed, ${FAIL} failed"
[[ "${FAIL}" -eq 0 ]]
