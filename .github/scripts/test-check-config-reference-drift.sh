#!/usr/bin/env bash
# .github/scripts/test-check-config-reference-drift.sh
#
# Test harness for check-config-reference-drift.sh — the docs/reference/config drift gate.
#
# Sibling of test-check-cli-reference-drift.sh; read that one first. Everything about the shape is
# the same: hermetic throwaway repos, harness-only seams (CONFIG_REF_*) so this runs cargo-free in
# the pure-bash guard-tests job while the gate itself needs cargo, and deliberately broken fixtures
# asserting the gate goes red AND says why.
#
# THE CASE THIS SUITE EXISTS FOR is, again, the last one: a page that is REPRODUCIBLE but not
# COMPLETE. For the CLI reference that was a precaution. Here it is a regression test — the
# renderer's tree walk had three traversal bugs that silently omitted 10 of 27 fields, every one of
# them behind a clean diff.
#
#   bash .github/scripts/test-check-config-reference-drift.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
GATE="${SCRIPT_DIR}/check-config-reference-drift.sh"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

make_repo() {
    local root="$1"
    rm -rf "$root"
    mkdir -p "$root/docs/reference/config"
    printf '# Configuration reference\n' >"$root/docs/reference/config/README.md"
    git -C "$root" init -q
    git -C "$root" config user.email t@t.invalid
    git -C "$root" config user.name t
    git -C "$root" add -A
    git -C "$root" commit -qm fixture
}

# A stub emit must print a NON-EMPTY document to stdout — the gate checks for that
# separately from the exit status, because a build that "succeeds" while emitting
# nothing is its own failure mode.
clean_emit() { echo 'echo "{\"schema\":{\"properties\":{}}}"'; }

# A stub render that reproduces the committed page.
clean_render() {
    echo "printf '# Configuration reference\n' > $1/docs/reference/config/README.md; echo 'Emitted 1 config reference files describing 27 fields (9 undocumented)'"
}

# run_case NAME REPO EMIT RENDER COMPLETE EXPECTED_EXIT [NEEDLE]
run_case() {
    local name="$1" repo="$2" emit="$3" render="$4" complete="$5" expected_exit="$6" needle="${7:-}"
    local output actual_exit
    set +e
    output="$(CONFIG_REF_REPO_ROOT="$repo" CONFIG_REF_EMIT_CMD="$emit" \
        CONFIG_REF_RENDER_CMD="$render" CONFIG_REF_COMPLETE_CMD="$complete" bash "$GATE" 2>&1)"
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

echo "test-check-config-reference-drift"

# --- BASELINE: without this, every red below proves nothing ---
R="${WORK}/clean"; make_repo "$R"
run_case "a reproducible, complete page passes" \
    "$R" "$(clean_emit)" "$(clean_render "$R")" "true" 0 "is up to date with TemperConfig"

R="${WORK}/untracked"; make_repo "$R"
git -C "$R" rm -rq --cached docs/reference/config
git -C "$R" commit -qm "untrack the tree"
run_case "a tree git does not track: REFUSES rather than passing vacuously" \
    "$R" "$(clean_emit)" "$(clean_render "$R")" "true" 1 "has no files tracked by git"

R="${WORK}/badschema"; make_repo "$R"
run_case "the schema emit fails: gate fails, and says the emit did" \
    "$R" "echo 'error: could not compile' >&2; exit 101" "$(clean_render "$R")" "true" 101 \
    "the schema emit failed"

# An emit that exits 0 while printing nothing. Distinct from the case above and worth its own
# fixture: the exit status says success, so only the emptiness check catches it, and without that
# check the renderer would be handed an empty file and the failure would be attributed to it.
R="${WORK}/emptyschema"; make_repo "$R"
run_case "the emit succeeds but prints nothing: gate fails, and blames the EMIT" \
    "$R" "true" "$(clean_render "$R")" "true" 1 "produced an empty document"

R="${WORK}/badrender"; make_repo "$R"
run_case "the render fails: gate fails, and says the render did" \
    "$R" "$(clean_emit)" "echo 'Traceback' ; exit 3" "true" 3 "rendering the config reference failed"

R="${WORK}/noop"; make_repo "$R"
run_case "the render reports zero fields: REFUSES rather than comparing nothing" \
    "$R" "$(clean_emit)" "echo 'Emitted 1 config reference files describing 0 fields'" "true" 1 \
    "reported no fields"

R="${WORK}/reworded"; make_repo "$R"
run_case "the render's report changes shape: REFUSES rather than assuming success" \
    "$R" "$(clean_emit)" "echo 'wrote the page'" "true" 1 "reported no fields"

R="${WORK}/modified"; make_repo "$R"
run_case "the page's content changed: gate fails" \
    "$R" "$(clean_emit)" \
    "printf 'CHANGED\n' > $R/docs/reference/config/README.md; echo 'Emitted 1 config reference files describing 27 fields'" \
    "true" 1 "out of date with TemperConfig"

# --- THE CASE THIS SUITE EXISTS FOR ---
R="${WORK}/incomplete"; make_repo "$R"
run_case "reproducible but incomplete: the diff is clean and the gate STILL fails" \
    "$R" "$(clean_emit)" "$(clean_render "$R")" \
    "echo 'IN SCHEMA, NOT RENDERED  \`index_path\`' >&2; exit 1" 1 \
    "REPRODUCIBLE (the diff above was clean) but not COMPLETE"

# --- WIRING ---
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
    ".github/workflows/code-quality.yml" "bash .github/scripts/check-config-reference-drift.sh"
assert_uncommented "the gate is a dependency of \`cargo make check\`" \
    "tools/cargo-make/main.toml" '"config-reference-drift",'

# The example the gate runs must stay feature-gated. Without `required-features` a plain
# `cargo build --examples` compiles it without `config-schema`, the JsonSchema derives are absent,
# and the build breaks for everyone who never asked for the docs machinery.
if grep -A3 '^\[\[example\]\]' "${REPO_ROOT}/crates/temper-core/Cargo.toml" \
    | grep -q 'required-features = \["config-schema"\]'; then
    echo "  ok: the config-reference example is gated behind its feature"
    PASS=$((PASS + 1))
else
    echo "  FAIL: crates/temper-core/Cargo.toml does not gate the config-reference example"
    echo "        behind required-features = [\"config-schema\"]"
    FAIL=$((FAIL + 1))
fi

# The rendered page states where config lives. That sentence is the renderer's own prose, so it is
# a TRANSCRIPTION of a path that is actually decided in Rust — exactly the kind of claim that rots
# silently. Pinned against the source of truth.
if grep -q '~/.config/temper/config.toml' "${REPO_ROOT}/crates/temper-core/src/types/config.rs" \
    && grep -q '~/.config/temper/config.toml' "${REPO_ROOT}/docs/reference/config/README.md"; then
    echo "  ok: the documented config path still matches the one config.rs resolves"
    PASS=$((PASS + 1))
else
    echo "  FAIL: docs/reference/config/README.md and config.rs disagree about where config lives"
    FAIL=$((FAIL + 1))
fi

# Reachability. docs/reference/ is markdown under docs/, so without the RUST_COUPLED entry a lone
# edit to a generated page scopes docs-only and rust-quality — where this gate lives — is off.
# Behavioural, through the real detector, via --stdin: without --stdin the detector falls back to
# `git diff` against the base ref and the assertion passes regardless.
verdict="$(printf 'docs/reference/config/README.md\n' \
    | bash "${REPO_ROOT}/.github/scripts/detect-ci-scope.sh" --stdin 2>/dev/null || true)"
if echo "$verdict" | grep -q '^RUN_RUST_QUALITY=true'; then
    echo "  ok: an edit to the generated config page summons the Rust corpus"
    PASS=$((PASS + 1))
else
    echo "  FAIL: an edit to docs/reference/config does not run rust-quality — this gate is"
    echo "        unreachable on precisely the change it exists to catch"
    FAIL=$((FAIL + 1))
fi

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
