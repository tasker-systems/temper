#!/usr/bin/env bash
# .github/scripts/test-check-reblock-callers.sh
#
# Test harness for check-reblock-callers.sh.
#
# The gate reads `git grep` against the working repository, so every case builds a SYNTHETIC
# git repository in a temp dir with the real script copied in. Nothing here touches the real
# tree: a probe that planted a rogue re-block caller in the real repo would be one interrupted
# run away from committing the exact regression the gate exists to stop.
#
# Four things are asserted beyond exit codes:
#
#   * the fenced symbol is DERIVED from the script under test, not restated here — the rogue
#     case reads its needle out of the gate's own ALLOWLIST/PATTERN text. A hand-copied list
#     goes stale silently and then reports PASS about names the gate no longer covers.
#   * the read-only survey is NOT fenced: a caller of `survey_reblock_resource` outside the
#     allowlist must pass. The gate keys on the WRITE op's reachability, not on the substring;
#     the word boundary in PATTERN is load-bearing, so it is exercised behaviourally from the
#     one side that matters (an identifier that merely CONTAINS the substring is not a caller).
#   * the gate is WIRED, on uncommented lines — both in code-quality.yml and as a `cargo make
#     check` dependency, mirroring the no-process-artifacts gate it is modelled on.
#   * the allowlist direction is tested too: an entry that no longer corresponds to a caller
#     must fail, not just an extra caller.
#
#   bash .github/scripts/test-check-reblock-callers.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
GATE="${SCRIPT_DIR}/check-reblock-callers.sh"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

TREE=""
make_tree() {
    TREE="${WORK}/$1"
    mkdir -p "${TREE}/.github/scripts" "${TREE}/src"
    cp "$GATE" "${TREE}/.github/scripts/"
    # The allowlist, verbatim from the gate, realized as real files in the synthetic repo.
    while IFS= read -r allowed; do
        [ -z "$allowed" ] && continue
        mkdir -p "${TREE}/$(dirname "$allowed")"
        echo "fn placeholder() { let _ = reblock_resource_with; }" > "${TREE}/${allowed}"
    done < <(grep -E '^crates/' "$GATE")
    local i=0
    while [ "$i" -lt 60 ]; do echo "fn f${i}() {}" > "${TREE}/src/f${i}.rs"; i=$((i + 1)); done
    ( cd "$TREE" && git init -q . && git add -A ) >/dev/null 2>&1
}

restage() { ( cd "$TREE" && git add -A ) >/dev/null 2>&1; }

run_case() {
    local name="$1" expected="$2" needle="${3:-}"
    local out actual=0
    out="$(cd "$TREE" && bash .github/scripts/check-reblock-callers.sh 2>&1)" || actual=$?

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

echo "Running check-reblock-callers.sh tests..."
echo ""

# --- POSITIVE ---
make_tree clean
run_case "a repo whose callers all match the allowlist: passes" 0 "OK:"

# --- NEGATIVE: a direct caller outside the allowlist, DERIVED from the gate's allowlist text ---
SYMBOL="$(grep -oE 'reblock_resource(_with|_in_tx)?' "$GATE" | head -1)"
if [ -z "$SYMBOL" ]; then
    echo "  FAIL: could not derive the fenced symbol from ${GATE} — the rogue cases below"
    echo "        would silently check nothing."
    FAIL=$((FAIL + 1))
else
    make_tree rogue
    echo "fn rogue_call() { let _ = ${SYMBOL}; }" > "${TREE}/src/rogue.rs"
    restage
    run_case "a direct ${SYMBOL} caller outside the allowlist: FAILS, named" 1 "src/rogue.rs"

    # The substring inside a LONGER identifier is not a caller: the word boundary excludes it,
    # so the gate must stay green. (A gate that fenced the bare substring would red-flag the
    # read-only survey's own symbol and every future reblock_resource-prefixed helper.)
    make_tree embedded
    echo "fn x${SYMBOL}y() {}" > "${TREE}/src/embedded.rs"
    restage
    run_case "the substring embedded in a longer identifier: passes (boundary holds)" 0 "OK:"
fi

# --- NEGATIVE: the read-only survey must NOT be fenced ---
make_tree survey
echo "use temper_substrate::writes::survey_reblock_resource;
async fn surveyor(pool: &sqlx::PgPool) { let _ = survey_reblock_resource(pool, unimplemented!()).await; }" \
    > "${TREE}/src/survey_caller.rs"
restage
run_case "a survey_reblock_resource caller outside the allowlist: passes (the survey is read-only)" 0 "OK:"

# --- NEGATIVE: a dropped allowlist entry leaves the caller exposed ---
make_tree unlisted
sed -i.bak '/crates\/temper-substrate\/tests\/reblock_survey.rs/d' "${TREE}/.github/scripts/check-reblock-callers.sh"
( cd "$TREE" && git add -A ) >/dev/null 2>&1
run_case "a real caller removed from the allowlist: FAILS, named" 1 "non-allowlisted"

# --- NEGATIVE: an allowlist entry whose file no longer mentions the op ---
make_tree stale
echo "fn placeholder() {}" > "${TREE}/crates/temper-substrate/tests/reblock_survey.rs"
restage
run_case "an allowlist entry whose file no longer mentions the op: FAILS (stale entry)" 1 "stale entry"

# --- NEGATIVE: the vacuous scan must never report clean ---
make_tree vacuous
( cd "$TREE" && git rm -rq --cached . ) >/dev/null 2>&1 || true
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

assert_uncommented "the gate runs in code-quality.yml, on a live (uncommented) line" \
    ".github/workflows/code-quality.yml" \
    "bash .github/scripts/check-reblock-callers.sh"

assert_uncommented "the gate runs in cargo make check, on a live (uncommented) line" \
    "tools/cargo-make/main.toml" \
    "check-reblock-callers.sh"

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
