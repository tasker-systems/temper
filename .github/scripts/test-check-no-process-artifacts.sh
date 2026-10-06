#!/usr/bin/env bash
# .github/scripts/test-check-no-process-artifacts.sh
#
# Test harness for check-no-process-artifacts.sh.
#
# The gate reads `git ls-files`, so every case builds a SYNTHETIC git repository in
# a temp dir with the real script copied in. Nothing here touches the working tree:
# a probe that planted internal/superpowers/ in the real repo would be one
# interrupted run away from committing the exact regression the gate exists to stop.
#
# Three things are asserted beyond exit codes:
#
#   * the forbidden set is DERIVED from the script under test, not restated here. A
#     hand-copied list goes stale silently and then reports PASS about names the gate
#     no longer covers.
#   * the gate is WIRED, on an uncommented line. Commenting the CI step out would
#     otherwise leave every assertion green while restoring the whole hole.
#   * the gate is REACHABLE. This is the one that actually bit: `internal/` is a
#     NON_PRODUCT root, so an internal-only change is a full skip — and a session
#     re-creating internal/superpowers/ from a stale instruction produces exactly that
#     change. The gate once sat present and unreachable on its own failure mode. It is
#     reached now because it lives inside guard-tests, which runs on every change;
#     this file pins the placement, test-detect-ci-scope.sh the missing `if:`s.
#
#   bash .github/scripts/test-check-no-process-artifacts.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
GATE="${SCRIPT_DIR}/check-no-process-artifacts.sh"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# make_tree NAME [FILECOUNT] — a synthetic repo holding a copy of the gate and enough
# tracked files to clear the vacuity guard. Returns the root in the global TREE.
TREE=""
make_tree() {
    TREE="${WORK}/$1"
    local n="${2:-120}"
    mkdir -p "${TREE}/.github/scripts" "${TREE}/internal/agents" "${TREE}/src"
    cp "$GATE" "${TREE}/.github/scripts/"
    echo "# an agent brief" > "${TREE}/internal/agents/architecture.md"
    local i=0
    while [ "$i" -lt "$n" ]; do echo "fn f${i}() {}" > "${TREE}/src/f${i}.rs"; i=$((i + 1)); done
    ( cd "$TREE" && git init -q . && git add -A ) >/dev/null 2>&1
}

# stage the tree again after a case adds files
restage() { ( cd "$TREE" && git add -A ) >/dev/null 2>&1; }

run_case() {
    local name="$1" expected="$2" needle="${3:-}"
    local out actual=0
    out="$(cd "$TREE" && bash .github/scripts/check-no-process-artifacts.sh 2>&1)" || actual=$?

    if [ "$actual" != "$expected" ]; then
        echo "  FAIL: ${name} — expected exit ${expected}, got ${actual}"
        echo "        output: ${out}"
        FAIL=$((FAIL + 1)); return 0
    fi
    if [ -n "$needle" ] && ! echo "$out" | grep -qF "$needle"; then
        echo "  FAIL: ${name} — exit ${actual} was right but the message did not mention '${needle}'"
        echo "        output: ${out}"
        FAIL=$((FAIL + 1)); return 0
    fi
    echo "  PASS: ${name}"
    PASS=$((PASS + 1))
}

echo "Running check-no-process-artifacts.sh tests..."
echo ""

# --- POSITIVE ---
make_tree clean
run_case "a repo with no process artifacts: passes" 0 "OK:"

# --- NEGATIVE: the moved tree, by name ---
make_tree moved-tree
mkdir -p "${TREE}/internal/superpowers/specs"
echo "# a design spec" > "${TREE}/internal/superpowers/specs/2026-08-27-x.md"
restage
run_case "internal/superpowers/ returns: FAILS" 1 "internal/superpowers"

# --- NEGATIVE: every forbidden directory name, DERIVED from the gate ---
FORBIDDEN_LINE="$(grep -E "^FORBIDDEN=" "$GATE")"
eval "$FORBIDDEN_LINE"
if [ -z "${FORBIDDEN:-}" ]; then
    echo "  FAIL: could not read FORBIDDEN out of ${GATE} — the derivation is broken, so the"
    echo "        per-name cases below would silently check nothing."
    FAIL=$((FAIL + 1))
else
    for d in $FORBIDDEN; do
        make_tree "forbidden-${d}"
        mkdir -p "${TREE}/somewhere/${d}"
        echo "# process material" > "${TREE}/somewhere/${d}/doc.md"
        restage
        run_case "a '${d}/' directory anywhere: FAILS" 1 "${d}/"
    done
fi

# Nesting: the regression will not necessarily reappear at the same depth it left.
make_tree nested
mkdir -p "${TREE}/internal/foo/bar/plans"
echo "# a plan" > "${TREE}/internal/foo/bar/plans/deep.md"
restage
run_case "a plans/ directory nested several levels deep: FAILS" 1 "plans/"

# --- NEGATIVE: the vacuous scan must never report clean ---
# An empty index satisfies both checks above by having nothing to find. This is the
# shape that turns a gate into decoration.
make_tree vacuous 0
rm -f "${TREE}/internal/agents/architecture.md"
( cd "$TREE" && git rm -r -q --cached . ) >/dev/null 2>&1 || true
run_case "an empty index: FAILS rather than reporting a vacuous clean" 1 "refusing to report clean"

# --- WIRING: a gate that runs nowhere passes everywhere ---
assert_uncommented() {
    local name="$1" file="$2" needle="$3"
    if grep -F "$needle" "${REPO_ROOT}/${file}" | grep -qvE '^[[:space:]]*#'; then
        echo "  PASS: ${name}"; PASS=$((PASS + 1))
    else
        echo "  FAIL: ${name} — '${needle}' absent or only present commented-out in ${file}"
        FAIL=$((FAIL + 1))
    fi
}

assert_uncommented "the gate runs in quality-gate.yml, on a live (uncommented) line" \
    ".github/workflows/quality-gate.yml" \
    "bash .github/scripts/check-no-process-artifacts.sh"

assert_uncommented "the gate runs in cargo make check, on a live (uncommented) line" \
    "tools/cargo-make/main.toml" \
    "check-no-process-artifacts.sh"

# --- REACHABILITY ---
#
# The regression is a markdown file under internal/, a NON_PRODUCT root, which
# detect-ci-scope.sh scopes as a full skip.
# Reachability is STRUCTURAL now. The gate runs in quality-gate.yml's guard-tests job; neither
# that job nor ci.yml's call to quality-gate.yml carries an `if:` (test-detect-ci-scope.sh pins
# both), so no scope verdict can switch it off. What this file owes is that the gate stays INSIDE
# guard-tests — under quality-gate's rust-gate job, `run-rust-quality` could switch it off on
# exactly the markdown-only change it exists for. Checked structurally: the invocation must sit
# between the `guard-tests:` header and the next job header.
wf="${REPO_ROOT}/.github/workflows/quality-gate.yml"
guard_line="$(grep -n '^  guard-tests:$' "$wf" | head -1 | cut -d: -f1 || true)"
guard_next=""
[ -z "$guard_line" ] || guard_next="$(awk -v s="$guard_line" 'NR > s && /^  [a-z][a-z0-9-]*:$/ { print NR; exit }' "$wf")"
[ -n "$guard_next" ] || guard_next=999999
gate_line="$(grep -n 'bash .github/scripts/check-no-process-artifacts.sh' "$wf" \
    | grep -vE ':[[:space:]]*#' | head -1 | cut -d: -f1 || true)"
if [ -n "$guard_line" ] && [ -n "$gate_line" ] && [ "$gate_line" -gt "$guard_line" ] \
   && [ "$gate_line" -lt "$guard_next" ]; then
    echo "  PASS: the gate runs inside guard-tests, the job nothing can switch off"
    PASS=$((PASS + 1))
else
    echo "  FAIL: the gate is not inside guard-tests (guard-tests at line '${guard_line}',"
    echo "        gate at line '${gate_line}') — reaching it may now require run-rust-quality"
    FAIL=$((FAIL + 1))
fi

# And that the change stays cheap — a full skip, with the Rust pipeline off. Input MUST go
# through --stdin: without it the detector diffs against the base ref, i.e. the real branch.
verdict="$(echo 'internal/superpowers/specs/2026-08-27-x.md' \
    | bash "${REPO_ROOT}/.github/scripts/detect-ci-scope.sh" --stdin 2>/dev/null || true)"
if echo "$verdict" | grep -qE '^RUN_TEST_RUST=false'; then
    echo "  PASS: reaching it does not conscript the Rust pipeline"
    PASS=$((PASS + 1))
else
    echo "  FAIL: an internal/ change now runs test-rust — reachability was bought at the wrong price"
    FAIL=$((FAIL + 1))
fi

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
