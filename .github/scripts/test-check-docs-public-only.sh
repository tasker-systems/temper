#!/usr/bin/env bash
# .github/scripts/test-check-docs-public-only.sh
#
# Test harness for check-docs-public-only.sh. Every sibling gate in the guard-tests
# job has one of these; this one did not, and a gate with no guard test is the
# unverified-verifier shape the whole suite exists to prevent — the branch that
# disarms a gate is exactly the branch that never exercises it.
#
# The gate `cd`s to its own ../.. and scans `docs/` there, so every case runs against
# a SYNTHETIC repo root in a temp dir with the real script copied in. Nothing here
# touches the working tree; a probe that plants a forbidden directory in the real
# repo would be one interrupted run away from a committed regression.
#
# Two things are asserted beyond exit codes:
#
#   * the forbidden set is DERIVED from the script under test, not restated here. A
#     hand-copied list would go stale silently and report PASS about names the gate
#     no longer covers — and it is a name going missing from that list that this gate
#     fails to catch in the first place.
#   * the gate is WIRED. It spent this branch's whole life in `cargo make check` and
#     in no workflow, so it could not fail on any pull request. A gate that runs
#     nowhere passes everywhere, and exit codes alone cannot see that.
#
#   bash .github/scripts/test-check-docs-public-only.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
GATE="${SCRIPT_DIR}/check-docs-public-only.sh"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# make_tree NAME — a synthetic repo root holding a copy of the gate and a minimal
# CLEAN docs/ (one page under one section, which is the shape the target structure
# has). Returns the root in the global TREE; callers add the offending file.
TREE=""
make_tree() {
    TREE="${WORK}/$1"
    mkdir -p "${TREE}/.github/scripts" "${TREE}/docs/guides"
    cp "$GATE" "${TREE}/.github/scripts/"
    echo "# a public guide" > "${TREE}/docs/guides/releasing.md"
}

# run_case NAME EXPECTED_EXIT [EXPECTED_SUBSTRING] — run the gate in $TREE.
run_case() {
    local name="$1" expected="$2" needle="${3:-}"
    local out actual=0
    out="$(cd "$TREE" && bash .github/scripts/check-docs-public-only.sh 2>&1)" || actual=$?

    if [ "$actual" != "$expected" ]; then
        echo "  FAIL: ${name} — expected exit ${expected}, got ${actual}"
        echo "        output: ${out}"
        FAIL=$((FAIL + 1))
        return 0
    fi
    if [ -n "$needle" ] && ! echo "$out" | grep -qF "$needle"; then
        echo "  FAIL: ${name} — exit ${actual} was right but the message did not mention '${needle}'"
        echo "        output: ${out}"
        FAIL=$((FAIL + 1))
        return 0
    fi
    echo "  PASS: ${name}"
    PASS=$((PASS + 1))
}

echo "Running check-docs-public-only.sh tests..."
echo ""

# --- POSITIVE: a clean tree passes, and index.md is the one permitted root page ---

make_tree clean
run_case "clean docs/ tree: passes" 0 "OK:"

make_tree index
echo "# landing" > "${TREE}/docs/index.md"
run_case "docs/index.md at the root: permitted (the one legitimate root page)" 0 "OK:"

# --- NEGATIVE (b): EVERY forbidden directory name, derived from the gate itself ---
#
# One instance would prove the check runs; the whole set is what proves the list the
# gate carries is the list it enforces. Derived by sourcing the assignment out of the
# script rather than restating it — see the header.
FORBIDDEN_LINE="$(grep -E "^FORBIDDEN=" "$GATE")"
eval "$FORBIDDEN_LINE"
if [ -z "${FORBIDDEN:-}" ]; then
    echo "  FAIL: could not read FORBIDDEN out of ${GATE} — the derivation is broken, so the"
    echo "        per-name cases below would silently check nothing."
    FAIL=$((FAIL + 1))
else
    for d in $FORBIDDEN; do
        make_tree "forbidden-${d}"
        mkdir -p "${TREE}/docs/${d}"
        echo "# internal material" > "${TREE}/docs/${d}/whatever.md"
        run_case "docs/${d}/ returns: FAILS" 1 "FAIL: docs/${d} exists"
    done

    # cognitive-maps is the one name whose absence is a DECISION (retired, not moved),
    # so its presence in the list is pinned by name as well as by the loop above.
    case " $FORBIDDEN " in
        *" cognitive-maps "*) echo "  PASS: cognitive-maps is on the denylist (retired, so its return is a regression)"; PASS=$((PASS + 1)) ;;
        *) echo "  FAIL: cognitive-maps is absent from FORBIDDEN — a retired tree could return green"; FAIL=$((FAIL + 1)) ;;
    esac
fi

# --- NEGATIVE (c): a loose page at the docs/ root ---

make_tree loose-md
echo "# an internal audit" > "${TREE}/docs/2026-03-31-code-review-audit.md"
run_case "loose .md at the docs/ root: FAILS" 1 "loose page at the docs/ root"

# The forbidden-name check reads DIRECTORIES, so a loose page is a genuinely separate
# failure mode — eleven of them sat at the root while (b) reported clean.
make_tree loose-md-beside-index
echo "# landing" > "${TREE}/docs/index.md"
echo "# brand strategy" > "${TREE}/docs/brand-direction.md"
run_case "loose .md alongside a permitted index.md: still FAILS" 1 "brand-direction.md"

# --- NEGATIVE (a): the empty scan must never report clean ---

make_tree empty
rm -f "${TREE}/docs/guides/releasing.md"
run_case "docs/ present but empty: FAILS rather than reporting a vacuous clean" 1

# --- WIRING: a gate that runs nowhere passes everywhere ---

assert_wired() {
    local name="$1" file="$2" needle="$3"
    if grep -qF "$needle" "${REPO_ROOT}/${file}"; then
        echo "  PASS: ${name}"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: ${name} — '${needle}' not found in ${file}"
        FAIL=$((FAIL + 1))
    fi
}

assert_wired "the gate runs in code-quality.yml (not only in cargo make check)" \
    ".github/workflows/code-quality.yml" \
    "bash .github/scripts/check-docs-public-only.sh"

# The regression this gate catches is a tree of *.md files, and detect-ci-scope.sh
# skips the entire pipeline for a markdown-only change. Wiring without this veto
# leaves the gate present and unreachable on precisely its own failure mode.
assert_wired "a docs/ change vetoes the docs-only CI skip (^docs/ in RUST_COUPLED)" \
    ".github/scripts/detect-ci-scope.sh" \
    '|^docs/'

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
