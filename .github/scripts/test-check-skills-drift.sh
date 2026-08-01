#!/usr/bin/env bash
# .github/scripts/test-check-skills-drift.sh
#
# Test harness for check-skills-drift.sh — the agent-skills projection drift gate.
#
# The gate's whole value is that it FAILS when the committed projection stops matching what the
# templates emit. Nothing about a passing gate distinguishes "the tree is in step" from "this script
# can no longer fail", so this harness feeds it deliberately broken fixtures and asserts it goes
# red — and asserts WHY, because a gate that fails for the wrong reason gets "fixed" by silencing
# the right one.
#
# HERMETIC: every case runs against a throwaway git repo built below, never the real working tree.
# The gate takes three harness-only seams for this — SKILLS_DRIFT_REPO_ROOT (where to look),
# SKILLS_DRIFT_TREE (which tree), and SKILLS_DRIFT_EMIT_CMD (what to run instead of the real,
# cargo-dependent emit). None is set by any CI job; the gate runs unstubbed in rust-quality.
#
# That stubbing is exactly what lets this harness live in the pure-bash `guard-tests` job while the
# gate itself lives in `rust-quality`.
#
#   bash .github/scripts/test-check-skills-drift.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
GATE="${SCRIPT_DIR}/check-skills-drift.sh"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# A throwaway repo shaped like temper: a committed skill tree holding both GENERATED files and a
# hand-written one the emit never touches.
make_repo() {
    local root="$1"
    rm -rf "$root"
    mkdir -p "$root/agent-skills/references"
    echo "# skill" >"$root/agent-skills/SKILL.md"
    echo "# frontmatter" >"$root/agent-skills/references/frontmatter.md"
    echo "# hand-written tool reference" >"$root/agent-skills/knowledge-base.md"
    git -C "$root" init -q
    git -C "$root" config user.email t@t.invalid
    git -C "$root" config user.name t
    git -C "$root" add -A
    git -C "$root" commit -qm fixture
}

# A stub emit that reproduces the committed tree — the shape of a clean run. Takes the repo root.
clean_emit() {
    echo "printf '# skill\n' > $1/agent-skills/SKILL.md; echo 'Emitted 2 agent-skill files'"
}

# run_case NAME REPO EMIT_CMD EXPECTED_EXIT [EXPECTED_SUBSTRING]
run_case() {
    local name="$1" repo="$2" emit="$3" expected_exit="$4" expected_substr="${5:-}"
    local output actual_exit
    set +e
    output="$(SKILLS_DRIFT_REPO_ROOT="$repo" SKILLS_DRIFT_EMIT_CMD="$emit" bash "$GATE" 2>&1)"
    actual_exit=$?
    set -e

    if [ "$actual_exit" -ne "$expected_exit" ]; then
        echo "  FAIL: ${name}"
        echo "    expected exit=${expected_exit} actual=${actual_exit}"
        echo "    output: ${output}"
        FAIL=$((FAIL + 1))
        return
    fi
    if [ -n "$expected_substr" ] && ! printf '%s' "$output" | grep -qF -- "$expected_substr"; then
        echo "  FAIL: ${name}"
        echo "    exit matched but expected message missing: ${expected_substr}"
        echo "    output: ${output}"
        FAIL=$((FAIL + 1))
        return
    fi
    echo "  ok: ${name}"
    PASS=$((PASS + 1))
}

echo "test-check-skills-drift"

# (a) The baseline. An emit that reproduces what is committed must be GREEN — otherwise every case
#     below is meaningless, since a gate that always fails proves nothing by failing.
make_repo "$WORK/a"
run_case "clean tree passes" "$WORK/a" "$(clean_emit "$WORK/a")" 0

# (b) The gate's reason to exist: a committed generated file whose content no longer matches what
#     the templates emit. This is the template-edited-but-tree-not-re-emitted case.
make_repo "$WORK/b"
run_case "MODIFIED generated file fails" "$WORK/b" \
    "echo '# skill CHANGED' > $WORK/b/agent-skills/SKILL.md; echo 'Emitted 2 agent-skill files'" \
    1 "out of date"

# (c) The case a plain `git diff --exit-code` MISSES, and the reason this gate uses `git status`.
#     A newly added template emits a file that has never been committed; it is UNTRACKED, and
#     `git diff` reports nothing for untracked paths. Committed as green, the skill would ship
#     without a file its own router names.
make_repo "$WORK/c"
run_case "UNTRACKED new generated file fails" "$WORK/c" \
    "echo '# teams' > $WORK/c/agent-skills/teams.md; echo 'Emitted 3 agent-skill files'" \
    1 "out of date"

# (d) A deleted generated file is drift too — a file the router still names but nothing emits.
make_repo "$WORK/d"
run_case "DELETED generated file fails" "$WORK/d" \
    "rm $WORK/d/agent-skills/references/frontmatter.md; echo 'Emitted 1 agent-skill files'" \
    1 "out of date"

# (e) THE LOAD-BEARING CASE: a tree that exists but has nothing tracked in it. `git status` over an
#     untracked path is silent, so without an explicit tracked-check the gate would pass forever
#     while checking nothing — green, and blind.
make_repo "$WORK/e"
git -C "$WORK/e" rm -q -r --cached agent-skills
git -C "$WORK/e" commit -qm "untrack the tree"
run_case "a tree with nothing tracked fails loudly" "$WORK/e" "$(clean_emit "$WORK/e")" \
    1 "nothing to diff against"

# (f) An emit that SUCCEEDS while writing nothing must fail the gate rather than pass it. The tree
#     would be clean and the gate green, having compared nothing — the most dangerous green
#     available, because it is indistinguishable from a real pass.
make_repo "$WORK/f"
run_case "an emit that writes NOTHING fails" "$WORK/f" "echo 'Emitted 0 agent-skill files'" \
    1 "reported no files written"

# (g) ...and so must an emit whose report this gate can no longer read. If the emit's output changes
#     shape, the count check silently stops discriminating; refusing is the only safe polarity.
#     The stub reproduces the tree correctly and prints ONLY the reworded report — so the tree is
#     clean and the ONLY thing standing between this and a false green is the count check.
make_repo "$WORK/g"
run_case "an unreadable emit report fails rather than passing" "$WORK/g" \
    "printf '# skill\n' > $WORK/g/agent-skills/SKILL.md; echo 'wrote some files, who knows how many'" \
    1 "reported no files written"

# (h) A FAILING emit must fail the gate. The emit is a cargo build, and the most ordinary way for it
#     to fail is a Rust compile error — precisely when someone is midway through changing the
#     templates this gate protects. Exiting 0 there would report "up to date" on the strength of
#     files nothing regenerated.
make_repo "$WORK/h"
run_case "a FAILING emit fails the gate" "$WORK/h" "exit 3" 3

# (i) ...and must SAY WHY. Case (h) only pins the exit code, and a gate that fails silently is
#     barely better than one that passes wrongly: cargo-make writes its errors to STDOUT, so
#     discarding stdout discards the reason. Assert the emit's own words reach the operator.
make_repo "$WORK/i"
run_case "a failing emit's OUTPUT reaches the operator" "$WORK/i" \
    "echo i-am-the-emit-error; exit 3" 3 "i-am-the-emit-error"

# (j) The hand-written siblings are NOT regenerated, and the gate must not demand that they be.
#     `knowledge-base.md` and `claude-desktop.md` are deliberately hand-maintained; an emit that
#     leaves them alone is correct, and a gate that reddened over them would force them into
#     generation. This pins the boundary the gate's header states.
make_repo "$WORK/j"
run_case "an emit that leaves hand-written files alone passes" "$WORK/j" \
    "$(clean_emit "$WORK/j")" 0

echo
echo "passed=${PASS} failed=${FAIL}"
[ "$FAIL" -eq 0 ]
