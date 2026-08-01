#!/usr/bin/env bash
#
# Fail if the committed `agent-skills/` tree drifts from the templates that produce it.
#
# Re-emits the MCP skill projection and fails if the result differs from what is committed — the
# same shape as check-ts-rs-drift.sh, for the same reason one tier up.
#
# ## Why this exists
#
# `agent-skills/` is an installable skill for MCP-only clients. Hand-maintained, it drifted past
# staleness into being WRONG about the system: it taught "Vault Primitives: Tickets / Milestones"
# and an "Epic Workflow", none of which temper has ever shipped, and its frontmatter reference
# documented Ticket and Milestone schemas. Nothing noticed, because nothing connects prose to the
# thing it describes. An installable skill teaching primitives that do not exist is a defect that
# ships straight into an agent's context window.
#
# The fix is not a rewrite — a rewrite rots the same way. The tree is now a PROJECTION of shared
# templates plus schema-derived content, and this gate is what makes consistency cost a rebuild
# rather than a memory.
#
# ## Why it builds from source rather than shelling out to `temper`
#
# This repo deliberately keeps a STALE installed `temper` for parity validation (an old CLI is how
# we notice when a change breaks older clients). A gate invoking the installed binary would compare
# the committed projection against whatever happens to be on PATH instead of against what is in the
# tree — green or red for reasons unrelated to the diff under review. `cargo make openapi` and the
# ts-rs gate build from source for exactly this reason; so does this.
#
# ## What it does NOT cover
#
# Only the GENERATED files. `knowledge-base.md` — the hand-written MCP tool reference — is
# intentionally not emitted, so a stale statement in it is invisible here. Do not read a green run
# as "the MCP skill is correct"; read it as "the generated part of the MCP skill matches its
# source". The tool-NAME half of that gap is closed separately, by a test in temper-mcp that
# checks every tool this tree names against the live router.
#
# Usage: bash .github/scripts/check-skills-drift.sh
#
# SKILLS_DRIFT_REPO_ROOT / SKILLS_DRIFT_TREE / SKILLS_DRIFT_EMIT_CMD are harness seams for
# test-check-skills-drift.sh (which must run without cargo, in the pure-bash guard-tests job).
# No CI job sets them; rust-quality runs this unstubbed.

set -euo pipefail

DEFAULT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REPO_ROOT="${SKILLS_DRIFT_REPO_ROOT:-$DEFAULT_ROOT}"
TREE="${SKILLS_DRIFT_TREE:-agent-skills/temper-knowledge-base}"
EMIT_CMD="${SKILLS_DRIFT_EMIT_CMD:-cargo run -q -p temper-cli -- skill emit --path $TREE}"

# The tree must have something TRACKED before we regenerate into it. `git status` over a path git
# does not know about reports nothing, so without this a gitignored or never-committed tree would
# pass forever while checking nothing. Same reasoning as the ts-rs and temper-ts gates.
if [ -z "$(git -C "$REPO_ROOT" ls-files -- "$TREE")" ]; then
    echo "ERROR: $TREE has no files tracked by git, so there is nothing to diff against." >&2
    echo "       Either the tree is gitignored, or the emit writes somewhere this path no" >&2
    echo "       longer names. Until that is fixed this gate checks nothing." >&2
    exit 1
fi

echo "Re-emitting the agent-skills projection into: $TREE"

# Capture the emit's output and replay it ONLY on failure — same reasoning as the ts-rs gate: a
# cargo build writes errors to stdout, so discarding stdout discards the reason, and a red job whose
# log goes straight from "Re-emitting" to "exit 1" costs a full round trip to diagnose.
EMIT_LOG="$(mktemp)"
trap 'rm -f "$EMIT_LOG"' EXIT
# Status captured OUTSIDE any `if !`: after `if ! cmd; then`, `$?` is the status of the NEGATION
# (i.e. 0), so the obvious spelling exits 0 on a failed emit and the gate passes the very case it
# exists to fail.
set +e
# shellcheck disable=SC2086 # the stub form in tests is a compound command, so word-splitting is wanted
(cd "$REPO_ROOT" && eval $EMIT_CMD) >"$EMIT_LOG" 2>&1
emit_status=$?
set -e

if [ "$emit_status" -ne 0 ]; then
    echo >&2
    echo "ERROR: the skill emit failed, so nothing was regenerated and this gate cannot tell you" >&2
    echo "       whether the committed tree is current. Its output:" >&2
    echo >&2
    cat "$EMIT_LOG" >&2
    exit "$emit_status"
fi

# An emit that succeeds while writing NOTHING would leave the tree clean and this gate green,
# having checked nothing — the same "a gate that cannot fail is worse than no gate" failure the
# ts-rs gate refuses via its zero-trees check. The count is read from the emit's own report, so if
# that report ever changes shape this refuses loudly rather than silently passing.
#
# The `|| true` is load-bearing, not defensive noise. Under `set -o pipefail` a non-matching `grep`
# fails the whole pipeline, and `set -e` then kills the script AT THIS LINE — exiting 1 with none of
# the diagnosis below ever printed. That is the silent-failure mode this gate's sibling was fixed
# for once already, reintroduced through a different door; the harness case for a reworded report
# is what caught it.
emitted="$(grep -oE 'Emitted [0-9]+ agent-skill files' "$EMIT_LOG" | grep -oE '[0-9]+' | head -1 || true)"
if [ -z "$emitted" ] || [ "$emitted" -lt 1 ]; then
    echo >&2
    echo "ERROR: the emit reported no files written, so this gate compared nothing." >&2
    echo "       Expected a line like 'Emitted N agent-skill files' with N >= 1. Its output:" >&2
    echo >&2
    cat "$EMIT_LOG" >&2
    exit 1
fi

# `git status --porcelain`, NOT `git diff --exit-code`. The diff form reports only tracked-file
# changes, so a NEWLY generated file — one nobody has committed yet — is invisible to it and the
# gate passes while the skill ships without it. `status` covers modified, deleted, AND untracked.
DIRTY="$(git -C "$REPO_ROOT" status --porcelain -- "$TREE")"

if [ -n "$DIRTY" ]; then
    echo >&2
    echo "ERROR: the committed agent-skills tree is out of date with the templates that produce it." >&2
    echo >&2
    printf '%s\n' "$DIRTY" >&2
    echo >&2
    echo "       Run: cargo run -p temper-cli -- skill emit --path $TREE" >&2
    echo "       then COMMIT the result. Staging is NOT enough: this gate uses" >&2
    echo "       \`git status --porcelain\` (it has to — see the comment above; the diff form" >&2
    echo "       cannot see a newly generated untracked file), and status reports staged-vs-HEAD" >&2
    echo "       changes too. A line above with 'M ' in the first column is exactly that case." >&2
    exit 1
fi

echo "agent-skills is up to date with its templates (${emitted} generated file(s) checked)"
