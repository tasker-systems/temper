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
# that safely is a separate change. Keep this script conservative: it only ever
# turns jobs OFF for pure-docs changes.
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
#   DOCS_ONLY, RUN_CODE_QUALITY, RUN_TEST_RUST, RUN_TEST_TYPESCRIPT, SCOPE_SUMMARY
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

# docs_only: at least one doc file AND no non-doc file AND not self-referential.
DOCS_ONLY=false
if [ "$HAS_DOCS" = "true" ] && [ "$HAS_NON_DOC" = "false" ] && [ "$HAS_SELF" = "false" ]; then
    DOCS_ONLY=true
fi

debug "HAS_DOCS=$HAS_DOCS HAS_SELF=$HAS_SELF HAS_NON_DOC=$HAS_NON_DOC -> DOCS_ONLY=$DOCS_ONLY"

# ---------------------------------------------------------------------------
# Compute job flags — every job runs unless the change is docs-only.
# ---------------------------------------------------------------------------
if [ "$DOCS_ONLY" = "true" ]; then
    RUN_CODE_QUALITY=false
    RUN_TEST_RUST=false
    RUN_TEST_TYPESCRIPT=false
    SCOPE_SUMMARY="docs-only: skipping code-quality, test-rust, test-typescript"
else
    RUN_CODE_QUALITY=true
    RUN_TEST_RUST=true
    RUN_TEST_TYPESCRIPT=true
    SCOPE_SUMMARY="full-ci: code change detected — running full pipeline"
fi

# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------
printf 'DOCS_ONLY=%s\n' "$DOCS_ONLY"
printf 'RUN_CODE_QUALITY=%s\n' "$RUN_CODE_QUALITY"
printf 'RUN_TEST_RUST=%s\n' "$RUN_TEST_RUST"
printf 'RUN_TEST_TYPESCRIPT=%s\n' "$RUN_TEST_TYPESCRIPT"
printf 'SCOPE_SUMMARY=%s\n' "$SCOPE_SUMMARY"

if [ "$USE_GITHUB_OUTPUT" = "true" ] && [ -n "${GITHUB_OUTPUT:-}" ]; then
    {
        echo "docs-only=${DOCS_ONLY}"
        echo "run-code-quality=${RUN_CODE_QUALITY}"
        echo "run-test-rust=${RUN_TEST_RUST}"
        echo "run-test-typescript=${RUN_TEST_TYPESCRIPT}"
        echo "scope-summary=${SCOPE_SUMMARY}"
    } >> "$GITHUB_OUTPUT"
fi
