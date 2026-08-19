#!/usr/bin/env bash
# .github/scripts/test-check-cli-reference-drift.sh
#
# Test harness for check-cli-reference-drift.sh — the docs/reference/cli drift gate.
#
# The gate's whole value is that it FAILS when the committed reference stops matching the binary
# that produces it. Nothing about a passing gate distinguishes "the tree is in step" from "this
# script can no longer fail", so this harness feeds it deliberately broken fixtures and asserts it
# goes red — and asserts WHY, because a gate that fails for the wrong reason gets "fixed" by
# silencing the right one.
#
# HERMETIC: every case runs against a throwaway git repo built below, never the real working tree.
# The gate takes harness-only seams for this — CLI_REF_REPO_ROOT, CLI_REF_TREE, CLI_REF_BUILD_CMD,
# CLI_REF_EMIT_CMD and CLI_REF_COMPLETE_CMD. None is set by any CI job; the gate runs unstubbed in
# rust-quality. Stubbing the build and the emit is what lets this harness live in the pure-bash
# `guard-tests` job while the gate itself needs cargo.
#
# THE CASE THIS SUITE EXISTS FOR is the last one: a tree that is REPRODUCIBLE but not COMPLETE. The
# diff is clean, and the gate must still go red on the cross-check. If that case ever passes, the
# gate has silently become a self-comparison — the exact failure the cross-check was added to
# prevent, and one no amount of re-emitting can detect.
#
#   bash .github/scripts/test-check-cli-reference-drift.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
GATE="${SCRIPT_DIR}/check-cli-reference-drift.sh"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# A throwaway repo shaped like temper: a committed reference tree with an index and one page.
make_repo() {
    local root="$1"
    rm -rf "$root"
    mkdir -p "$root/docs/reference/cli"
    printf '# CLI reference\n' >"$root/docs/reference/cli/README.md"
    printf '# `temper config`\n' >"$root/docs/reference/cli/config.md"
    # THREE pages, not two, and the third exists for the deletion case below: with only two, an
    # emit that drops one reports "Emitted 1", which trips the fewer-than-two refusal BEFORE the
    # diff is ever reached — so the case would assert the wrong failure and prove nothing about
    # whether deletion is detected.
    printf '# `temper status`\n' >"$root/docs/reference/cli/status.md"
    git -C "$root" init -q
    git -C "$root" config user.email t@t.invalid
    git -C "$root" config user.name t
    git -C "$root" add -A
    git -C "$root" commit -qm fixture
}

# A stub emit that reproduces the committed tree — the shape of a clean run.
clean_emit() {
    echo "printf '# CLI reference\n' > $1/docs/reference/cli/README.md; echo 'Emitted 2 cli reference files from 3 help nodes'"
}

# run_case NAME REPO BUILD EMIT COMPLETE EXPECTED_EXIT [EXPECTED_SUBSTRING]
run_case() {
    local name="$1" repo="$2" build="$3" emit="$4" complete="$5" expected_exit="$6" needle="${7:-}"
    local output actual_exit
    set +e
    output="$(CLI_REF_REPO_ROOT="$repo" CLI_REF_BUILD_CMD="$build" CLI_REF_EMIT_CMD="$emit" \
        CLI_REF_COMPLETE_CMD="$complete" bash "$GATE" 2>&1)"
    actual_exit=$?
    set -e

    if [ "$actual_exit" -ne "$expected_exit" ]; then
        echo "  FAIL: ${name}"
        echo "    expected exit=${expected_exit} actual=${actual_exit}"
        echo "    output: ${output}"
        FAIL=$((FAIL + 1))
        return
    fi
    if [ -n "$needle" ] && ! printf '%s' "$output" | grep -qF -- "$needle"; then
        echo "  FAIL: ${name}"
        echo "    exit matched but expected message missing: ${needle}"
        echo "    output: ${output}"
        FAIL=$((FAIL + 1))
        return
    fi
    echo "  ok: ${name}"
    PASS=$((PASS + 1))
}

echo "test-check-cli-reference-drift"

# --- BASELINE: a clean run must pass, or every red below proves nothing ---
R="${WORK}/clean"; make_repo "$R"
run_case "a reproducible, complete tree passes" \
    "$R" "true" "$(clean_emit "$R")" "true" 0 "is up to date with the binary"

# --- the gate must not check nothing ---
R="${WORK}/untracked"; make_repo "$R"
git -C "$R" rm -rq --cached docs/reference/cli
git -C "$R" commit -qm "untrack the tree"
run_case "a tree git does not track: REFUSES rather than passing vacuously" \
    "$R" "true" "$(clean_emit "$R")" "true" 1 "has no files tracked by git"

# --- a failed build must not read as a clean tree ---
R="${WORK}/badbuild"; make_repo "$R"
run_case "the CLI build fails: gate fails, and says the build did" \
    "$R" "echo 'error[E0432]: unresolved import'; exit 101" "$(clean_emit "$R")" "true" 101 \
    "the CLI build failed"

# --- nor a failed emit ---
R="${WORK}/bademit"; make_repo "$R"
run_case "the emit fails: gate fails, and says the emit did" \
    "$R" "true" "echo 'Traceback'; exit 3" "true" 3 "the CLI reference emit failed"

# --- an emit that writes nothing is the dead-gate case ---
R="${WORK}/noop"; make_repo "$R"
run_case "the emit reports zero files: REFUSES rather than comparing nothing" \
    "$R" "true" "echo 'Emitted 0 cli reference files from 0 help nodes'" "true" 1 \
    "fewer than two files"

# A reworded report must also refuse. The count is parsed out of the emit's own output, so a
# changed message means the gate can no longer prove the emit did anything — and the sibling gate
# was once fixed for silently exiting here instead of diagnosing.
R="${WORK}/reworded"; make_repo "$R"
run_case "the emit's report changes shape: REFUSES rather than assuming success" \
    "$R" "true" "echo 'wrote 25 pages'" "true" 1 "fewer than two files"

# --- actual drift, in all three shapes git reports ---
R="${WORK}/modified"; make_repo "$R"
run_case "a page's content changed: gate fails" \
    "$R" "true" "printf 'CHANGED\n' > $R/docs/reference/cli/config.md; echo 'Emitted 2 cli reference files'" \
    "true" 1 "out of date with the binary"

R="${WORK}/added"; make_repo "$R"
run_case "a NEW page appears: gate fails (git diff could not see this; status can)" \
    "$R" "true" "printf '# new\n' > $R/docs/reference/cli/steward.md; echo 'Emitted 3 cli reference files'" \
    "true" 1 "out of date with the binary"

# Deletion is the shape that matters most here: the emitter wipes the tree before writing, so a
# command that no longer exists must surface as a deleted page rather than as a stale file nothing
# ever looks at again.
R="${WORK}/deleted"; make_repo "$R"
run_case "a page disappears: gate fails" \
    "$R" "true" "rm -f $R/docs/reference/cli/config.md; echo 'Emitted 2 cli reference files'" \
    "true" 1 "out of date with the binary"

# --- THE CASE THIS SUITE EXISTS FOR ---
#
# Reproducible but NOT complete. The emit reproduces the committed tree exactly, so the diff is
# clean and every check above is satisfied — and the tree is still wrong, because the emitter
# dropped a command from BOTH sides. Only the independent cross-check can see it. If this case ever
# goes green, the gate has decayed into a self-comparison.
R="${WORK}/incomplete"; make_repo "$R"
run_case "reproducible but incomplete: the diff is clean and the gate STILL fails" \
    "$R" "true" "$(clean_emit "$R")" \
    "echo 'IN SOURCE, NOT DOCUMENTED  temper admin subscription' >&2; exit 1" 1 \
    "REPRODUCIBLE (the diff above was clean) but not COMPLETE"

# --- WIRING: a gate that runs nowhere passes everywhere ---
assert_uncommented() {
    local name="$1" file="$2" needle="$3"
    if grep -F "$needle" "${REPO_ROOT}/${file}" | grep -qvE '^[[:space:]]*#'; then
        echo "  ok: ${name}"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: ${name} — '${needle}' absent or only commented-out in ${file}"
        FAIL=$((FAIL + 1))
    fi
}

assert_uncommented "the gate runs in code-quality.yml, on a live line" \
    ".github/workflows/code-quality.yml" "bash .github/scripts/check-cli-reference-drift.sh"
assert_uncommented "the gate is a dependency of \`cargo make check\`" \
    "tools/cargo-make/main.toml" '"cli-reference-drift",'

# And it must be REACHABLE. docs/reference/** is markdown under docs/, so without a RUST_COUPLED
# entry a lone edit to a generated page scopes as docs-only, rust-quality is switched off, and the
# gate above never runs on the one change it exists to catch. Asserted behaviourally through the
# real detector, via --stdin — without --stdin the detector falls back to `git diff` against the
# base ref and the assertion passes no matter what the detector contains.
ref_verdict="$(printf 'docs/reference/cli/config.md\n' \
    | bash "${REPO_ROOT}/.github/scripts/detect-ci-scope.sh" --stdin 2>/dev/null || true)"
if echo "$ref_verdict" | grep -q '^RUN_RUST_QUALITY=true'; then
    echo "  ok: an edit to a generated reference page summons the Rust corpus"
    PASS=$((PASS + 1))
else
    echo "  FAIL: an edit to docs/reference/ does not run rust-quality — the drift gate is"
    echo "        unreachable on precisely the change it exists to catch"
    echo "        detector said: $(echo "$ref_verdict" | grep -E '^(DOCS_ONLY|SKIP_ALL|RUN_RUST_QUALITY)=' | tr '\n' ' ')"
    FAIL=$((FAIL + 1))
fi

# The converse, so the entry above stays SCOPED. An ordinary docs page must keep its cheap path;
# an over-broad `^docs/` would make this gate reachable by conscripting the whole pipeline for
# every guide edit, which is the bill-sized-to-the-wrong-gate mistake this repo already corrected
# once.
guide_verdict="$(printf 'docs/guides/install.md\n' \
    | bash "${REPO_ROOT}/.github/scripts/detect-ci-scope.sh" --stdin 2>/dev/null || true)"
if echo "$guide_verdict" | grep -q '^RUN_RUST_QUALITY=false'; then
    echo "  ok: an ordinary guide edit still skips the Rust corpus"
    PASS=$((PASS + 1))
else
    echo "  FAIL: an ordinary docs/guides/ edit now runs rust-quality — the reference entry"
    echo "        has leaked into the rest of docs/"
    FAIL=$((FAIL + 1))
fi

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
