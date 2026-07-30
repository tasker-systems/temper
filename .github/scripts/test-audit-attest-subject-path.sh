#!/usr/bin/env bash
# .github/scripts/test-audit-attest-subject-path.sh
#
# Test harness for audit-attest-subject-path.sh. Runs the auditor against the real
# build-cli-binaries.yml and against fixtures derived from it, asserting exit code AND failure
# reason.
#
# The load-bearing test is (b): deleting the `.manifest.json` line from `subject-path` is a
# one-line, entirely silent edit — the signing job still succeeds (@actions/glob only errors when
# the COMBINED match set is empty), the archive is still attested, and nothing else in the repo
# reads that block. The first symptom would be a real release failing in a user's terminal. If this
# harness ever stops going red on that fixture, the green tick in CI means nothing.
#
# Every fixture is compared against the real workflow before use: a `sed` whose pattern silently
# stopped matching would produce a fixture identical to the original, which the guard would
# correctly pass — and the harness would report a false PASS about a bite that never happened.
#
#   bash .github/scripts/test-audit-attest-subject-path.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT_SCRIPT="${SCRIPT_DIR}/audit-attest-subject-path.sh"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
REAL_WF="${REPO_ROOT}/.github/workflows/build-cli-binaries.yml"
PASS=0
FAIL=0

FIXTURE_DIR="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_DIR"' EXIT

# The subject-path entries are the only lines that are a bare `temper-v…` filename carrying the
# matrix triple. That excludes the emit step's `OUTPUT:` line (prefixed by the key) and the upload
# step's `temper-v…-*.…` globs (no triple), so these patterns edit the attestation block alone.
#
# ERE throughout (`sed -E`), never BRE alternation: `\(a\|b\)` is a GNU extension that BSD sed
# silently treats as a literal, which would make the archive fixture a no-op copy on macOS.
SUBJECT_MANIFEST_RE='^[[:space:]]*temper-v.*matrix\.target\.triple.*\.manifest\.json$'
SUBJECT_ARCHIVE_RE='^[[:space:]]*temper-v.*matrix\.target\.triple.*\.(tar\.gz|zip)$'

# make_fixture NAME SED_EXPR... — derive a fixture from the real workflow and prove it differs.
# The path lands in the global FIXTURE rather than on stdout: a function that returns its result on
# stdout cannot also report a failure there (the message would be captured into the caller's
# variable and vanish), and its FAIL++ would be lost to the command-substitution subshell. That is
# exactly how the archive case silently ran zero assertions in the first draft of this harness.
FIXTURE=""
make_fixture() {
    local name="$1"; shift
    FIXTURE="${FIXTURE_DIR}/${name}.yml"
    sed -E "$@" "$REAL_WF" > "$FIXTURE"
    if cmp -s "$REAL_WF" "$FIXTURE"; then
        echo "  FAIL: fixture '${name}' is identical to the real workflow — the sed pattern no longer matches."
        echo "        A fixture that is not broken cannot prove the guard bites."
        FAIL=$((FAIL + 1))
        return 1
    fi
    return 0
}

# run_test NAME WORKFLOW_FILE EXPECTED_EXIT [EXPECTED_SUBSTRING]
run_test() {
    local test_name="$1" wf="$2" expected_exit="$3" expected_substr="${4:-}"
    local output actual_exit
    set +e
    output="$(bash "$AUDIT_SCRIPT" "$wf" 2>&1)"
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

echo "Running audit-attest-subject-path.sh tests..."
echo ""

# --- (a) the real workflow passes: archive and manifest both attested ---
run_test "real build-cli-binaries.yml: passes" "$REAL_WF" 0 \
    "archive and manifest"

# --- (b) THE ONE THAT MATTERS: the manifest line deleted from subject-path ---
# One line, no other signal anywhere. This silently defeats version.rs's `--verify --online` and
# update.rs's manifest attestation, both of which resolve the MANIFEST's own digest.
if make_fixture no-manifest "/${SUBJECT_MANIFEST_RE}/d"; then
    run_test "manifest line deleted from subject-path: fails" "$FIXTURE" 1 \
        "no \`subject-path\` entry ends in .manifest.json"
fi

# --- (c) the archive half bites symmetrically ---
if make_fixture no-archive "/${SUBJECT_ARCHIVE_RE}/d"; then
    run_test "archive lines deleted from subject-path: fails" "$FIXTURE" 1 \
        "no \`subject-path\` entry names a release archive"
fi

# --- (d) an emptied subject list must FAIL, not pass vacuously ---
# Nothing attested satisfies "attests the archive" and "attests the manifest" equally well.
if make_fixture empty-subjects -e "/${SUBJECT_MANIFEST_RE}/d" -e "/${SUBJECT_ARCHIVE_RE}/d"; then
    run_test "empty subject-path: fails rather than passing vacuously" "$FIXTURE" 1 \
        "no \`subject-path\` entries found"
fi

# --- (e) one-sided rename: attested name drifts from the emitted name ---
# The `.manifest.json` suffix is deliberately left INTACT so assertion (c) is satisfied and cannot
# be what fires. This is caught only by the emitted-vs-attested cross-check, and it would ship an
# attestation over a path the release never writes while the real manifest goes unattested.
if make_fixture renamed-subject-only "/${SUBJECT_MANIFEST_RE}/ s|temper-v|temper-RENAMED-v|"; then
    run_test "subject renamed but emit OUTPUT unchanged: fails" "$FIXTURE" 1 \
        "the attested manifest is not the one the release emits"
fi

# --- (f) coordinated rename on BOTH sides: still fails, on the pinned suffix ---
# emit OUTPUT and subject-path agree, so the cross-check is happy. It is refused because update.rs,
# version.rs and install.sh all construct the literal `.manifest.json`, and a rename that only moved
# the workflow would break every one of them at release time with CI green.
if make_fixture renamed-both "s|\.manifest\.json\$|.manifest.txt|"; then
    run_test "manifest renamed on both emit and subject-path: still fails" "$FIXTURE" 1 \
        "no \`subject-path\` entry ends in .manifest.json"
fi

# --- (g) the attest step removed entirely: fails ---
# The ref is matched as `@<anything>` rather than a literal `@v4`: the workflow SHA-pins its actions
# (`@<40-hex> # v4`), so a literal version here would stop matching on the next pin bump and the
# fixture would silently equal the real workflow. `make_fixture`'s cmp guard catches that — this
# pattern is what keeps it from firing every time the pin moves.
if make_fixture no-attest-step "s|actions/attest-build-provenance@[^[:space:]]+.*\$|actions/upload-artifact@v7|"; then
    run_test "attest-build-provenance step gone: fails" "$FIXTURE" 1 \
        "no \`subject-path\` entries found"
fi

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
