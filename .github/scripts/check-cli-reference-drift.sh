#!/usr/bin/env bash
#
# Fail if the committed `docs/reference/cli` tree drifts from the binary that produces it.
#
# Same shape as check-skills-drift.sh, and for the same reason: the reference is a committed
# PROJECTION, and consistency should cost a rebuild rather than a memory.
#
# ## Why the binary, and which binary
#
# The emitter shells out to a built `temper` and reads its rendered `--help`. It does not parse
# clap source, because a source parser answers "what do the derives say" — a claim about the tree —
# where a user meets "what does the binary print".
#
# This gate BUILDS that binary rather than using whatever is on PATH, because a gate must compare
# against the tree UNDER REVIEW. That is not a theoretical preference. When this gate was written,
# the PATH `temper` was v0.3.1 while the tree built v0.3.3, and the PATH binary was missing the
# entire `admin subscription` subtree — five commands. A gate trusting PATH would have deleted five
# real pages and called it drift.
#
# It builds with the SHIPPING feature set (temper-cli's defaults, `embed,extract` — what
# build-cli-binaries.yml and install.sh both use), not `--all-features`. `cli.rs` carries no
# `cfg(feature)` today so the two agree, but if a command is ever feature-gated the reference must
# follow what ships.
#
# ## Two questions, not one
#
# 1. **Is it current?** Re-emit and diff. This proves the tree is REPRODUCIBLE.
# 2. **Is it complete?** Cross-check against the clap source, via
#    scripts/check-cli-reference-complete.py.
#
# The second is not redundant, and skipping it was the first design of this gate. The emitter finds
# subcommands by parsing `Commands:` blocks out of help text. If that parse ever drops a command,
# the emitter writes a smaller tree, the diff compares it against an equally smaller committed tree,
# and this gate goes green forever while the reference quietly stops mentioning a command. An
# artifact compared against itself measures reproducibility, never correctness.
#
# ## Reachability
#
# `docs/reference/` is in detect-ci-scope.sh's RUST_COUPLED for the same reason `agent-skills/` is:
# it is markdown, so it would otherwise scope as docs-only, and a docs-only change turns
# rust-quality OFF — leaving a hand-edit to a generated tree with nothing to catch it. Only the Rust
# toolchain can verify this tree, so a change to it must summon that toolchain.
#
# Usage: bash .github/scripts/check-cli-reference-drift.sh
#
# CLI_REF_REPO_ROOT / CLI_REF_TREE / CLI_REF_BUILD_CMD / CLI_REF_EMIT_CMD / CLI_REF_COMPLETE_CMD are
# harness seams for test-check-cli-reference-drift.sh, which must run without cargo in the pure-bash
# guard-tests job. No CI job sets them; rust-quality runs this unstubbed.

set -euo pipefail

DEFAULT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REPO_ROOT="${CLI_REF_REPO_ROOT:-$DEFAULT_ROOT}"
TREE="${CLI_REF_TREE:-docs/reference/cli}"
BUILD_CMD="${CLI_REF_BUILD_CMD:-cargo build -q -p temper-cli}"
EMIT_CMD="${CLI_REF_EMIT_CMD:-python3 scripts/emit-cli-reference.py --bin target/debug/temper --out $TREE}"
COMPLETE_CMD="${CLI_REF_COMPLETE_CMD:-python3 scripts/check-cli-reference-complete.py --tree $TREE}"

# The tree must have something TRACKED before we regenerate into it. `git status` over a path git
# does not know about reports nothing, so without this a gitignored or never-committed tree would
# pass forever while checking nothing. Same reasoning as the skills, ts-rs and temper-ts gates.
if [ -z "$(git -C "$REPO_ROOT" ls-files -- "$TREE")" ]; then
    echo "ERROR: $TREE has no files tracked by git, so there is nothing to diff against." >&2
    echo "       Either the tree is gitignored, or the emit writes somewhere this path no" >&2
    echo "       longer names. Until that is fixed this gate checks nothing." >&2
    exit 1
fi

LOG="$(mktemp)"
trap 'rm -f "$LOG"' EXIT

echo "Building the CLI (shipping feature set) to document the tree under review"
# Status captured OUTSIDE any `if !`: after `if ! cmd; then`, `$?` is the status of the NEGATION
# (i.e. 0), so the obvious spelling exits 0 on a failed build and the gate passes the very case it
# exists to fail.
set +e
# shellcheck disable=SC2086 # the stub form in tests is a compound command, so word-splitting is wanted
(cd "$REPO_ROOT" && eval $BUILD_CMD) >"$LOG" 2>&1
build_status=$?
set -e
if [ "$build_status" -ne 0 ]; then
    echo >&2
    echo "ERROR: the CLI build failed, so no binary exists to read `--help` from and this gate" >&2
    echo "       cannot tell you whether the committed reference is current. Its output:" >&2
    echo >&2
    cat "$LOG" >&2
    exit "$build_status"
fi

echo "Re-emitting the CLI reference into: $TREE"
set +e
# shellcheck disable=SC2086
(cd "$REPO_ROOT" && eval $EMIT_CMD) >"$LOG" 2>&1
emit_status=$?
set -e
if [ "$emit_status" -ne 0 ]; then
    echo >&2
    echo "ERROR: the CLI reference emit failed, so nothing was regenerated and this gate cannot" >&2
    echo "       tell you whether the committed tree is current. Its output:" >&2
    echo >&2
    cat "$LOG" >&2
    exit "$emit_status"
fi
cat "$LOG"

# An emit that succeeds while writing NOTHING would leave the tree clean and this gate green,
# having checked nothing. The count is read from the emit's own report, so if that report ever
# changes shape this refuses loudly rather than silently passing.
#
# The `|| true` is load-bearing, not defensive noise. Under `set -o pipefail` a non-matching `grep`
# fails the whole pipeline and `set -e` then kills the script AT THIS LINE — exiting 1 with none of
# the diagnosis below ever printed.
emitted="$(grep -oE 'Emitted [0-9]+ cli reference files' "$LOG" | grep -oE '[0-9]+' | head -1 || true)"
if [ -z "$emitted" ] || [ "$emitted" -lt 2 ]; then
    echo >&2
    echo "ERROR: the emit reported fewer than two files, so this gate compared nothing of" >&2
    echo "       substance. Expected a line like 'Emitted N cli reference files' with N >= 2" >&2
    echo "       (an index plus at least one command). Its output:" >&2
    echo >&2
    cat "$LOG" >&2
    exit 1
fi

# `git status --porcelain`, NOT `git diff --exit-code`. The diff form reports only tracked-file
# changes, so a NEWLY generated page — one nobody has committed yet — is invisible to it and the
# gate passes while the reference ships without it. `status` covers modified, deleted AND untracked,
# and deleted matters most here: the emitter wipes the tree before writing, so a command that no
# longer exists shows up as a deletion rather than as a stale file nothing looks at.
DIRTY="$(git -C "$REPO_ROOT" status --porcelain -- "$TREE")"
if [ -n "$DIRTY" ]; then
    echo >&2
    echo "ERROR: the committed CLI reference is out of date with the binary that produces it." >&2
    echo >&2
    printf '%s\n' "$DIRTY" >&2
    echo >&2
    echo "       Run: cargo build -p temper-cli && \\" >&2
    echo "            python3 scripts/emit-cli-reference.py --bin target/debug/temper" >&2
    echo "       then COMMIT the result. Staging is NOT enough: this gate uses" >&2
    echo "       \`git status --porcelain\`, which reports staged-vs-HEAD changes too." >&2
    exit 1
fi

# The independent half. Everything above compares the tree against ITSELF one commit later; this
# compares it against the clap source, a genuinely different derivation. See the header.
echo "Cross-checking the reference against the clap source"
set +e
# shellcheck disable=SC2086
(cd "$REPO_ROOT" && eval $COMPLETE_CMD)
complete_status=$?
set -e
if [ "$complete_status" -ne 0 ]; then
    echo >&2
    echo "       ^ The tree is REPRODUCIBLE (the diff above was clean) but not COMPLETE." >&2
    echo "       That combination is exactly what the drift check alone cannot detect." >&2
    exit "$complete_status"
fi

echo "docs/reference/cli is up to date with the binary (${emitted} generated file(s) checked)"
