#!/usr/bin/env bash
# .github/scripts/detect-ci-scope.sh
#
# Determine which CI jobs need to run based on changed files.
#
# Temper's pipeline is small (code-quality + test-rust + test-typescript, gated
# by ci-success). The single high-value, unimpeachably-safe optimization is:
# when a change touches ONLY documentation, skip the whole pipeline. Everything
# else runs the full pipeline. Per-language granularity (rust-only vs ts-only)
# is deliberately NOT attempted here — ts-rs generates TS types from Rust, so a
# "rust-only" change can still move the committed TypeScript surface; splitting
# that safely is a separate change.
#
# Two exceptions are path-scoped:
#
# - test-ruby: the gem (clients/temper-rb/**), the contract it is generated
#   from (openapi.json), its own workflow, and the scripts implementing its
#   codegen drift gate. It pulls a ~1GB openapi-generator image for that gate,
#   and nothing outside that set can affect it.
# - test-agents-ts: the TS SDK (clients/temper-ts/**) and the eve agents that
#   consume it (packages/agent-workflows/**), plus the wire contracts both are
#   asserted against (tests/contracts/**), its own workflow, and the scripts
#   implementing its codegen drift gate.
#
# Both scopings are safe for the same reason: each project is inert to both
# cargo (`members = ["crates/*", "tests/e2e"]`) and bun (an explicit two-entry
# `workspaces` list) — no Rust or TS change can reach it except through a
# contract, which is in its trigger set.
#
# Both trigger sets include their own gate's SCRIPTS for a sharper reason: a gate
# can be disarmed by editing it, and a disarmed gate passes. Leave those scripts
# out and the PR that breaks a gate is precisely the PR that never runs it — it
# merges green and every later PR inherits a dead check.
#
# Keep this script conservative: for every OTHER job it only ever turns things
# OFF for pure-docs changes, and a self-referential edit forces a full run.
#
# Usage:
#   .github/scripts/detect-ci-scope.sh [OPTIONS]
#
# Options:
#   --github-output   Write kebab-case outputs to $GITHUB_OUTPUT (for Actions)
#   --stdin           Read newline-separated file list from stdin (for tests)
#   --base REF        Override base ref for git diff
#   --verbose         Print debug info to stderr
#
# Output (stdout, eval-safe KEY=VALUE):
#   DOCS_ONLY, RUN_CODE_QUALITY, RUN_TEST_RUST, RUN_TEST_TYPESCRIPT,
#   RUN_TEST_RUBY, RUN_TEST_AGENTS_TS, SCOPE_SUMMARY
#
# Bash 3.2 compatible (macOS default): no ${var^^}, no mapfile, no assoc arrays.

set -euo pipefail

USE_GITHUB_OUTPUT=false
USE_STDIN=false
BASE_REF_OVERRIDE=""
VERBOSE=false

while [ $# -gt 0 ]; do
    case $1 in
        --github-output) USE_GITHUB_OUTPUT=true; shift ;;
        --stdin)         USE_STDIN=true; shift ;;
        --base)          BASE_REF_OVERRIDE="$2"; shift 2 ;;
        --base=*)        BASE_REF_OVERRIDE="${1#*=}"; shift ;;
        --verbose)       VERBOSE=true; shift ;;
        *)               echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

debug() {
    if [ "$VERBOSE" = "true" ]; then
        echo "[detect-ci-scope] $*" >&2
    fi
}

# ---------------------------------------------------------------------------
# Get changed files
# ---------------------------------------------------------------------------
if [ "$USE_STDIN" = "true" ]; then
    CHANGED_FILES="$(cat)"
else
    if [ -n "$BASE_REF_OVERRIDE" ]; then
        BASE_REF="$BASE_REF_OVERRIDE"
    elif [ -n "${GITHUB_BASE_REF:-}" ]; then
        # PR context: diff against the merge-base with the target branch.
        BASE_REF="$(git merge-base "origin/${GITHUB_BASE_REF}" HEAD 2>/dev/null || echo "origin/${GITHUB_BASE_REF}")"
    elif [ "${GITHUB_EVENT_NAME:-}" = "push" ] && [ "${GITHUB_REF:-}" = "refs/heads/main" ]; then
        # Push to main: diff the pushed commit against its parent.
        BASE_REF="HEAD~1"
    else
        # Local dev: compare against main.
        BASE_REF="$(git merge-base origin/main HEAD 2>/dev/null || echo "origin/main")"
    fi
    debug "Base ref: ${BASE_REF}"
    CHANGED_FILES="$(git diff "${BASE_REF}" HEAD --name-only 2>/dev/null || true)"
fi

if [ -z "$CHANGED_FILES" ]; then
    # Safety fallback: no diff detected -> run everything.
    debug "No changed files detected — defaulting to full CI"
    CHANGED_FILES="__force_full_ci__"
fi

if [ "$VERBOSE" = "true" ]; then
    debug "Changed files:"
    echo "$CHANGED_FILES" | while IFS= read -r f; do echo "  $f" >&2; done
fi

changes_match() {
    echo "$CHANGED_FILES" | grep -qE "$1"
}

# ---------------------------------------------------------------------------
# Detection
# ---------------------------------------------------------------------------
HAS_DOCS=false
HAS_SELF=false
HAS_NON_DOC=false

# Documentation: markdown / text files anywhere (including CLAUDE.md, READMEs,
# docs/** and per-crate doc files — none of these affect a build or test).
if changes_match '\.(md|txt|adoc)$'; then
    HAS_DOCS=true
fi

# Self-referential: this script (or its test) changed -> never skip, so the
# change is actually exercised by a full run.
if changes_match '^\.github/scripts/detect-ci-scope'; then
    HAS_SELF=true
fi

# Any non-doc file present means we cannot treat the change as docs-only.
NON_DOC_FILES="$(echo "$CHANGED_FILES" | grep -vE '\.(md|txt|adoc)$' || true)"
if [ -n "$NON_DOC_FILES" ]; then
    HAS_NON_DOC=true
fi

# Ruby SDK: the gem's own tree, the contracts it is asserted against, its CI
# workflow, and the scripts that IMPLEMENT its codegen drift gate. openapi.json is
# in this set precisely because a contract change must be SEEN to move the gem --
# that is what the codegen drift gate proves. The same logic applies to
# tests/contracts/: credentials_spec.rb reads m2m-token-request.json and asserts the
# gem emits it, so a contract change that does not run this job is a contract change
# nothing checks.
#
# generate-temper-rb.sh / check-temper-rb-drift.sh are here because a gate's own
# implementation must run the gate. A gate can be silently disarmed by editing it --
# `git diff --exit-code -- <path-that-matches-nothing>` exits 0, so one typo'd path
# turns the check into a permanent pass. Without this key, the PR that broke the gate
# is exactly the PR that would not run it: it merges green, and every later PR
# inherits a dead gate.
#
# The no-diff safety fallback must run everything, this job included.
HAS_RUBY=false
if changes_match '^clients/temper-rb/|^tests/contracts/|^openapi\.json$|^\.github/workflows/test-ruby\.yml$|^\.github/scripts/(generate-temper-rb|check-temper-rb-drift)\.sh$|^__force_full_ci__$'; then
    HAS_RUBY=true
fi

# TypeScript SDK + agent workflows: clients/temper-ts (the TS client) and
# packages/agent-workflows/** (the eve agents that consume it), plus BOTH wire
# contracts they are asserted against.
#
# openapi.json is in this set for the same reason it is in test-ruby's: temper-ts
# commits a generated schema.ts, so a contract change that does not run this job is
# a contract change whose drift gate never fires. tests/contracts/ is the other
# contract (the m2m token request).
#
# generate-temper-ts.sh / check-temper-ts-drift.sh are here for the reason spelled
# out above test-ruby: a gate's own implementation must run the gate, or the one PR
# that can disarm it is the one PR that never exercises it.
#
# crates/** deliberately stays OUT: openapi.json is committed, and openapi-check in
# code-quality already forces a DTO change to land a regenerated spec in the same
# PR. The contract is therefore both sufficient and precise as the trigger key.
#
# Path-scoped for exactly the reason test-ruby is: these projects are inert to
# both cargo (`members = ["crates/*", "tests/e2e"]`) and bun (an explicit
# two-entry `workspaces` list), so no Rust or TS change can reach them except
# through a contract, which is in the trigger set.
HAS_AGENTS_TS=false
if changes_match '^clients/temper-ts/|^packages/agent-workflows/|^tests/contracts/|^openapi\.json$|^\.github/workflows/test-agents-ts\.yml$|^\.github/scripts/(generate-temper-ts|check-temper-ts-drift)\.sh$|^__force_full_ci__$'; then
    HAS_AGENTS_TS=true
fi

# docs_only: at least one doc file AND no non-doc file AND not self-referential.
DOCS_ONLY=false
if [ "$HAS_DOCS" = "true" ] && [ "$HAS_NON_DOC" = "false" ] && [ "$HAS_SELF" = "false" ]; then
    DOCS_ONLY=true
fi

debug "HAS_DOCS=$HAS_DOCS HAS_SELF=$HAS_SELF HAS_NON_DOC=$HAS_NON_DOC HAS_RUBY=$HAS_RUBY HAS_AGENTS_TS=$HAS_AGENTS_TS -> DOCS_ONLY=$DOCS_ONLY"

# ---------------------------------------------------------------------------
# Compute job flags — every job runs unless the change is docs-only.
#
# test-ruby and test-agents-ts are the PATH-SCOPED jobs: test-ruby pulls a
# ~1GB openapi-generator image for the codegen drift gate, so it stays off the
# critical path of PRs that cannot possibly affect the gem; test-agents-ts
# runs two `npm install`s across two projects that most PRs never touch. A
# self-referential change to this script forces both on, matching the
# conservative posture above.
# ---------------------------------------------------------------------------
if [ "$DOCS_ONLY" = "true" ]; then
    RUN_CODE_QUALITY=false
    RUN_TEST_RUST=false
    RUN_TEST_TYPESCRIPT=false
    RUN_TEST_RUBY=false
    RUN_TEST_AGENTS_TS=false
    SCOPE_SUMMARY="docs-only: skipping code-quality, test-rust, test-typescript, test-ruby, test-agents-ts"
else
    RUN_CODE_QUALITY=true
    RUN_TEST_RUST=true
    RUN_TEST_TYPESCRIPT=true
    if [ "$HAS_RUBY" = "true" ] || [ "$HAS_SELF" = "true" ]; then
        RUN_TEST_RUBY=true
    else
        RUN_TEST_RUBY=false
    fi
    if [ "$HAS_AGENTS_TS" = "true" ] || [ "$HAS_SELF" = "true" ]; then
        RUN_TEST_AGENTS_TS=true
    else
        RUN_TEST_AGENTS_TS=false
    fi
    SCOPE_SUMMARY="full-ci: code change detected — running full pipeline (test-ruby=${RUN_TEST_RUBY}, test-agents-ts=${RUN_TEST_AGENTS_TS})"
fi

# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------
printf 'DOCS_ONLY=%s\n' "$DOCS_ONLY"
printf 'RUN_CODE_QUALITY=%s\n' "$RUN_CODE_QUALITY"
printf 'RUN_TEST_RUST=%s\n' "$RUN_TEST_RUST"
printf 'RUN_TEST_TYPESCRIPT=%s\n' "$RUN_TEST_TYPESCRIPT"
printf 'RUN_TEST_RUBY=%s\n' "$RUN_TEST_RUBY"
printf 'RUN_TEST_AGENTS_TS=%s\n' "$RUN_TEST_AGENTS_TS"
printf 'SCOPE_SUMMARY=%s\n' "$SCOPE_SUMMARY"

if [ "$USE_GITHUB_OUTPUT" = "true" ] && [ -n "${GITHUB_OUTPUT:-}" ]; then
    {
        echo "docs-only=${DOCS_ONLY}"
        echo "run-code-quality=${RUN_CODE_QUALITY}"
        echo "run-test-rust=${RUN_TEST_RUST}"
        echo "run-test-typescript=${RUN_TEST_TYPESCRIPT}"
        echo "run-test-ruby=${RUN_TEST_RUBY}"
        echo "run-test-agents-ts=${RUN_TEST_AGENTS_TS}"
        echo "scope-summary=${SCOPE_SUMMARY}"
    } >> "$GITHUB_OUTPUT"
fi
