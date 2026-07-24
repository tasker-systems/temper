#!/usr/bin/env bash
# .github/scripts/test-check-ts-rs-drift.sh
#
# Test harness for check-ts-rs-drift.sh — the ts-rs generated-types drift gate.
#
# The gate's whole value is that it FAILS when a committed generated file stops matching what the
# Rust emits. Nothing about a passing gate distinguishes "the types are in step" from "this script
# can no longer fail", so this harness feeds it deliberately broken fixtures and asserts it goes
# red — and asserts WHY, because a gate that fails for the wrong reason is a gate that will be
# "fixed" by silencing the right one.
#
# HERMETIC: every case runs against a throwaway git repo built below, never the real working tree.
# The gate takes two harness-only seams for this — TS_RS_DRIFT_REPO_ROOT (where to look) and
# TS_RS_DRIFT_GENERATE_CMD (what to run instead of the real, cargo-dependent generator) — the same
# fixture-injection idiom audit-signature-secrets.sh uses with MIDDLEWARE_FILE. Neither is set by
# any CI job; the gate runs unstubbed in rust-quality.
#
# This is why the harness can live in the pure-bash `guard-tests` job while the gate itself lives
# in `rust-quality`: stubbing the generator is exactly what removes the cargo dependency.
#
#   bash .github/scripts/test-check-ts-rs-drift.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
GATE="${SCRIPT_DIR}/check-ts-rs-drift.sh"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Build a throwaway repo shaped like temper: a cargo-make file naming N export dirs, and a
# committed generated file in each. The gate must DERIVE the trees from that file rather than
# carry its own copy of the list — a hardcoded list is the thing that goes stale when someone
# adds a third consumer.
make_repo() {
    local root="$1"
    rm -rf "$root"
    mkdir -p "$root/tools/cargo-make" "$root/tree-a" "$root/tree-b"
    cat >"$root/tools/cargo-make/main.toml" <<'TOML'
[tasks.generate-ts-types]
script = [
  "TS_RS_EXPORT_DIR=${CARGO_MAKE_WORKING_DIRECTORY}/tree-a cargo test -p a --features typescript",
  "TS_RS_EXPORT_DIR=${CARGO_MAKE_WORKING_DIRECTORY}/tree-b cargo test -p b --features typescript export_bindings_x",
]
TOML
    echo "export type A = string;" >"$root/tree-a/a.ts"
    echo "export type B = number;" >"$root/tree-b/b.ts"
    git -C "$root" init -q
    git -C "$root" config user.email t@t.invalid
    git -C "$root" config user.name t
    git -C "$root" add -A
    git -C "$root" commit -qm fixture
}

# run_case NAME REPO GENERATE_CMD EXPECTED_EXIT [EXPECTED_SUBSTRING]
run_case() {
    local name="$1" repo="$2" gen="$3" expected_exit="$4" expected_substr="${5:-}"
    local output actual_exit
    set +e
    output="$(TS_RS_DRIFT_REPO_ROOT="$repo" TS_RS_DRIFT_GENERATE_CMD="$gen" bash "$GATE" 2>&1)"
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

echo "test-check-ts-rs-drift"

# (a) The baseline. A generator that reproduces what is committed must be GREEN — otherwise every
#     case below is meaningless, since a gate that always fails proves nothing by failing.
make_repo "$WORK/a"
run_case "clean tree passes" "$WORK/a" "true" 0

# (b) The gate's reason to exist: a tracked generated file whose content no longer matches what the
#     generator emits. This is the Rust-type-changed-but-TS-not-regenerated case.
make_repo "$WORK/b"
run_case "MODIFIED generated file fails" "$WORK/b" \
    "echo 'export type A = number;' > $WORK/b/tree-a/a.ts" 1 "out of date"

# (c) The case a plain `git diff --exit-code` MISSES, and the reason this gate uses `git status`
#     instead. A newly ts-rs-derived Rust type emits a file that has never been committed; it is
#     UNTRACKED, and `git diff` reports nothing for untracked paths. Committed as green, the new
#     type would then have no generated counterpart at all — which is exactly the state
#     packages/temper-ui/.../slack_link.ts was found in: emitted by no one, noticed by nothing.
make_repo "$WORK/c"
run_case "UNTRACKED new generated file fails" "$WORK/c" \
    "echo 'export type C = boolean;' > $WORK/c/tree-a/c.ts" 1 "out of date"

# (d) A deleted generated file is drift too — the Rust type lost its derive but the consumer still
#     imports the file. Fails as a missing artifact rather than passing silently.
make_repo "$WORK/d"
run_case "DELETED generated file fails" "$WORK/d" "rm $WORK/d/tree-b/b.ts" 1 "out of date"

# (e) THE LOAD-BEARING CASE: a tree that exists but has nothing tracked in it. `git diff` over an
#     untracked path exits 0, so without an explicit tracked-check the gate would pass forever
#     while checking nothing — green, and blind. Same reasoning as check-temper-ts-drift.sh's
#     ls-files assertion, applied per tree.
make_repo "$WORK/e"
git -C "$WORK/e" rm -q -r --cached tree-b
git -C "$WORK/e" commit -qm "untrack tree-b"
run_case "a tree with nothing tracked fails loudly" "$WORK/e" "true" 1 "nothing to diff against"

# (f) The gate must refuse to run over ZERO trees. If the derivation stops matching main.toml —
#     renamed task, reformatted script lines, a move to a different file — the loop body would
#     execute zero times and the gate would exit 0 having checked nothing. A gate that cannot fail
#     must say so instead of passing.
make_repo "$WORK/f"
: >"$WORK/f/tools/cargo-make/main.toml"
git -C "$WORK/f" commit -qam "empty cargo-make"
run_case "zero derived trees fails" "$WORK/f" "true" 1 "no ts-rs output trees"

# (g) A FAILING generator must fail the gate, not pass it. The generator is a cargo build, and the
#     most ordinary way for it to fail is a Rust compile error — precisely when someone is midway
#     through changing the types this gate protects. Exiting 0 there would report "types are up to
#     date" on the strength of files nothing regenerated, which is the most dangerous green
#     available: it is indistinguishable from a real pass.
make_repo "$WORK/g"
run_case "a FAILING generator fails the gate" "$WORK/g" "exit 3" 3

# (h) ...and must SAY WHY. Case (g) only pins the exit code, and a gate that fails silently is
#     barely better than one that passes wrongly: the first version of this script sent the
#     generator's output to /dev/null, cargo-make writes its errors to STDOUT, and the result was a
#     red CI job whose log went straight from "Regenerating" to "exit code 1" with no reason in
#     between. Cost a full round trip. Assert the generator's own words reach the operator.
make_repo "$WORK/h"
run_case "a failing generator's OUTPUT reaches the operator" "$WORK/h" \
    "echo i-am-the-generator-error; exit 3" 3 "i-am-the-generator-error"

echo
echo "passed=${PASS} failed=${FAIL}"
[ "$FAIL" -eq 0 ]
