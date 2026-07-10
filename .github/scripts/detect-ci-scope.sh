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
# ONE exception: test-ruby is path-scoped to the gem (clients/temper-rb/**), the
# contract it is generated from (openapi.json), and its own workflow. It pulls a
# ~1GB openapi-generator image for the codegen drift gate, and nothing outside
# that set can affect it. The scoping is safe because the gem is inert to both
# cargo (`members = ["crates/*", "tests/e2e"]`) and bun (an explicit two-entry
# `workspaces` list) — no Rust or TS change can reach it except through the
# contract, which is in its trigger set.
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
#   RUN_TEST_RUBY, SCOPE_SUMMARY
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

# Ruby SDK: the gem's own tree, the contract it is generated from, and its CI
# workflow. openapi.json is in this set precisely because a contract change must
# be SEEN to move the gem -- that is what the codegen drift gate proves.
#
# The no-diff safety fallback must run everything, this job included.
HAS_RUBY=false
if changes_match '^clients/temper-rb/|^openapi\.json$|^\.github/workflows/test-ruby\.yml$|^__force_full_ci__$'; then
    HAS_RUBY=true
fi

# docs_only: at least one doc file AND no non-doc file AND not self-referential.
DOCS_ONLY=false
if [ "$HAS_DOCS" = "true" ] && [ "$HAS_NON_DOC" = "false" ] && [ "$HAS_SELF" = "false" ]; then
    DOCS_ONLY=true
fi

debug "HAS_DOCS=$HAS_DOCS HAS_SELF=$HAS_SELF HAS_NON_DOC=$HAS_NON_DOC HAS_RUBY=$HAS_RUBY -> DOCS_ONLY=$DOCS_ONLY"

# ---------------------------------------------------------------------------
# Compute job flags — every job runs unless the change is docs-only.
#
# test-ruby is the one exception, and the only PATH-SCOPED job: it pulls a ~1GB
# openapi-generator image for the codegen drift gate, so it stays off the
# critical path of PRs that cannot possibly affect the gem. A self-referential
# change to this script forces it on, matching the conservative posture above.
# ---------------------------------------------------------------------------
if [ "$DOCS_ONLY" = "true" ]; then
    RUN_CODE_QUALITY=false
    RUN_TEST_RUST=false
    RUN_TEST_TYPESCRIPT=false
    RUN_TEST_RUBY=false
    SCOPE_SUMMARY="docs-only: skipping code-quality, test-rust, test-typescript, test-ruby"
else
    RUN_CODE_QUALITY=true
    RUN_TEST_RUST=true
    RUN_TEST_TYPESCRIPT=true
    if [ "$HAS_RUBY" = "true" ] || [ "$HAS_SELF" = "true" ]; then
        RUN_TEST_RUBY=true
    else
        RUN_TEST_RUBY=false
    fi
    SCOPE_SUMMARY="full-ci: code change detected — running full pipeline (test-ruby=${RUN_TEST_RUBY})"
fi

# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------
printf 'DOCS_ONLY=%s\n' "$DOCS_ONLY"
printf 'RUN_CODE_QUALITY=%s\n' "$RUN_CODE_QUALITY"
printf 'RUN_TEST_RUST=%s\n' "$RUN_TEST_RUST"
printf 'RUN_TEST_TYPESCRIPT=%s\n' "$RUN_TEST_TYPESCRIPT"
printf 'RUN_TEST_RUBY=%s\n' "$RUN_TEST_RUBY"
printf 'SCOPE_SUMMARY=%s\n' "$SCOPE_SUMMARY"

if [ "$USE_GITHUB_OUTPUT" = "true" ] && [ -n "${GITHUB_OUTPUT:-}" ]; then
    {
        echo "docs-only=${DOCS_ONLY}"
        echo "run-code-quality=${RUN_CODE_QUALITY}"
        echo "run-test-rust=${RUN_TEST_RUST}"
        echo "run-test-typescript=${RUN_TEST_TYPESCRIPT}"
        echo "run-test-ruby=${RUN_TEST_RUBY}"
        echo "scope-summary=${SCOPE_SUMMARY}"
    } >> "$GITHUB_OUTPUT"
fi
