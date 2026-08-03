#!/usr/bin/env bash
# .github/scripts/detect-ci-scope.sh
#
# Determine which CI jobs need to run based on changed files.
#
# Temper's pipeline is small (code-quality + test-rust + test-typescript, gated
# by ci-success). Two unimpeachably-safe optimizations live here:
#
#   1. DOCS-ONLY — a change touching ONLY *.md/*.txt/*.adoc skips the WHOLE
#      pipeline.
#
#   2. RUST-INERT — a change whose every non-doc file lives in a tree that is
#      provably inert to the Rust corpus (the TS packages and the SDK clients)
#      skips the Rust half of CI: the whole test-rust workflow AND the
#      rust-quality job inside code-quality (via RUN_RUST_QUALITY). TypeScript
#      quality + tests, the pure-bash guard-tests, and the path-scoped SDK jobs
#      still run. The trees are inert BY CONSTRUCTION: cargo's
#      `members = ["crates/*", "tests/e2e"]` and bun's explicit `workspaces`
#      list keep them unreachable from any crate, so a change confined to them
#      cannot move a Rust build/test/quality-gate outcome.
#
# The RUST-INERT skip is ONE-DIRECTIONAL by design. The reverse — a "rust-only"
# change skipping TypeScript — is deliberately NOT done: ts-rs generates the
# committed TS types FROM Rust, so a Rust change can move the TypeScript surface.
# That coupling is exactly what the RUST_COUPLED veto below defends: a manual
# edit to a generated TS tree, the agent-skills projection, openapi.json, or a
# wire contract forces the Rust quality gates that regenerate-and-diff them
# (ts-rs-drift, skills-drift, openapi-check) back ON, even though those files sit
# under an inert root or look like plain data.
#
# Per-CRATE granularity (skip the corpus for a "leaf" crate — e.g. a temper-cli
# askama-template edit) is DELIBERATELY NOT attempted, and this is a recorded
# decision, not an omission. Two independent repo invariants forbid it:
# test-rust selects with `--workspace` and never sub-selects tests
# (.github/workflows/CLAUDE.md — "a filter that makes CI green is hiding a
# test"), and the CLI's shared templates RENDER the committed `agent-skills/`
# projection gated by skills-drift, so "just a template" is not a leaf change.
# Feature unification and ts-rs make the crate-level couplings non-obvious; the
# safe unit is the LANGUAGE boundary, not the crate.
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
#   DOCS_ONLY, RUST_INERT, RUN_CODE_QUALITY, RUN_RUST_QUALITY, RUN_TEST_RUST,
#   RUN_TEST_TYPESCRIPT, RUN_TEST_RUBY, RUN_TEST_AGENTS_TS, SCOPE_SUMMARY
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

# ---------------------------------------------------------------------------
# Rust-inert detection (optimization 2 in the header).
#
# RUST_INERT_ROOTS: trees cargo's `members` and bun's `workspaces` prove cannot
# reach a crate, so a change confined to them cannot change a Rust build/test/
# quality-gate outcome. The Ruby gem is here too: it is inert to cargo for the
# same reason, and its own drift gate rides test-ruby (keyed below), not the
# Rust corpus.
RUST_INERT_ROOTS='^packages/temper-cloud/|^packages/temper-ui/|^packages/agent-workflows/|^clients/temper-ts/|^clients/temper-telemetry-ts/|^clients/temper-rb/'

# RUST_COUPLED: committed artifacts that a Rust quality gate regenerates and
# diffs. Two of them live UNDER an inert root (the ts-rs generated trees), so
# the inert-roots test alone would wave them through — but a manual edit to any
# of them must run the Rust gate that owns it (ts-rs-drift, skills-drift,
# openapi-check). Their presence therefore VETOES the rust-inert skip. This is
# the send-side of the one-directional coupling explained in the header:
# Rust → TS is real, so a TS-surface edit pulls Rust back on.
RUST_COUPLED='^packages/temper-ui/src/lib/types/generated/|^packages/agent-workflows/mention/agent/generated/|^agent-skills/|^openapi\.json$|^tests/contracts/'

HAS_RUST_COUPLED=false
if changes_match "$RUST_COUPLED"; then
    HAS_RUST_COUPLED=true
fi

# Are ALL non-doc files inside an inert root? (grep -v the inert roots over the
# non-doc set must leave nothing.) An empty non-doc set makes this vacuously
# true, but RUST_INERT also requires HAS_NON_DOC, so docs-only never gets here.
# An UNRECOGNIZED non-doc file (anything outside the inert roots — a crate, a
# migration, a workflow, a githook) fails this test, so the default stays
# conservative: unknown → run the full Rust corpus.
ALL_NON_DOC_INERT=false
if [ "$HAS_NON_DOC" = "true" ]; then
    NON_INERT_FILES="$(echo "$NON_DOC_FILES" | grep -vE "$RUST_INERT_ROOTS" || true)"
    if [ -z "$NON_INERT_FILES" ]; then
        ALL_NON_DOC_INERT=true
    fi
fi

# rust_inert: real (non-doc) changes exist, every one is in an inert tree, none
# touches a Rust-coupled committed artifact, and this is not a self-edit to the
# detector (which forces a full run so the change is actually exercised).
# HAS_NON_DOC=true is itself proof the change is not docs-only — the two are
# mutually exclusive — so there is no DOCS_ONLY clause to add here.
RUST_INERT=false
if [ "$HAS_SELF" = "false" ] && \
   [ "$HAS_NON_DOC" = "true" ] && \
   [ "$ALL_NON_DOC_INERT" = "true" ] && \
   [ "$HAS_RUST_COUPLED" = "false" ]; then
    RUST_INERT=true
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
#
# clients/temper-telemetry-ts is here too: it is the shared OTel bootstrap package the
# eve agents (and temper-ui) consume as a `file:` dependency, and it carries its own
# typecheck+vitest job in this workflow. temper-ui rebuilds against it through the
# always-on test-typescript job (RUN_TEST_TYPESCRIPT fires on any non-docs change), so
# no separate key is needed there.
HAS_AGENTS_TS=false
if changes_match '^clients/temper-ts/|^clients/temper-telemetry-ts/|^packages/agent-workflows/|^tests/contracts/|^openapi\.json$|^\.github/workflows/test-agents-ts\.yml$|^\.github/scripts/(generate-temper-ts|check-temper-ts-drift)\.sh$|^__force_full_ci__$'; then
    HAS_AGENTS_TS=true
fi

# docs_only: at least one doc file AND no non-doc file AND not self-referential
# AND no Rust-coupled committed artifact. The last clause closes a latent hole:
# `agent-skills/**` is the committed skill projection and holds *.md files, so a
# hand-edit to one is all-docs by extension and would otherwise skip the WHOLE
# pipeline — including the skills-drift gate that owns it, letting the projection
# drift from its source unnoticed. A RUST_COUPLED touch is never "just docs".
DOCS_ONLY=false
if [ "$HAS_DOCS" = "true" ] && [ "$HAS_NON_DOC" = "false" ] && \
   [ "$HAS_SELF" = "false" ] && [ "$HAS_RUST_COUPLED" = "false" ]; then
    DOCS_ONLY=true
fi

debug "HAS_DOCS=$HAS_DOCS HAS_SELF=$HAS_SELF HAS_NON_DOC=$HAS_NON_DOC HAS_RUBY=$HAS_RUBY HAS_AGENTS_TS=$HAS_AGENTS_TS HAS_RUST_COUPLED=$HAS_RUST_COUPLED ALL_NON_DOC_INERT=$ALL_NON_DOC_INERT -> DOCS_ONLY=$DOCS_ONLY RUST_INERT=$RUST_INERT"

# ---------------------------------------------------------------------------
# Compute job flags.
#
#   docs-only    -> nothing runs.
#   rust-inert   -> the Rust corpus (test-rust + the rust-quality job) is off,
#                   but code-quality is still INVOKED so its typescript-quality
#                   and guard-tests jobs run; TypeScript tests run too. This is
#                   the RUN_RUST_QUALITY axis: RUN_CODE_QUALITY still gates
#                   *whether code-quality.yml is called at all*, RUN_RUST_QUALITY
#                   gates the rust-quality job *inside* it.
#   otherwise    -> full pipeline.
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
    RUN_RUST_QUALITY=false
    RUN_TEST_RUST=false
    RUN_TEST_TYPESCRIPT=false
    RUN_TEST_RUBY=false
    RUN_TEST_AGENTS_TS=false
    SCOPE_SUMMARY="docs-only: skipping code-quality, test-rust, test-typescript, test-ruby, test-agents-ts"
else
    # code-quality.yml is invoked for every non-docs change so its TypeScript
    # and guard-test jobs always run; the Rust half is the part that scopes off.
    RUN_CODE_QUALITY=true
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
    if [ "$RUST_INERT" = "true" ]; then
        RUN_RUST_QUALITY=false
        RUN_TEST_RUST=false
        SCOPE_SUMMARY="rust-inert: change confined to Rust-inert trees — skipping test-rust + rust-quality; running TypeScript + guards (test-ruby=${RUN_TEST_RUBY}, test-agents-ts=${RUN_TEST_AGENTS_TS})"
    else
        RUN_RUST_QUALITY=true
        RUN_TEST_RUST=true
        SCOPE_SUMMARY="full-ci: code change detected — running full pipeline (test-ruby=${RUN_TEST_RUBY}, test-agents-ts=${RUN_TEST_AGENTS_TS})"
    fi
fi

# ---------------------------------------------------------------------------
# Output
# ---------------------------------------------------------------------------
printf 'DOCS_ONLY=%s\n' "$DOCS_ONLY"
printf 'RUST_INERT=%s\n' "$RUST_INERT"
printf 'RUN_CODE_QUALITY=%s\n' "$RUN_CODE_QUALITY"
printf 'RUN_RUST_QUALITY=%s\n' "$RUN_RUST_QUALITY"
printf 'RUN_TEST_RUST=%s\n' "$RUN_TEST_RUST"
printf 'RUN_TEST_TYPESCRIPT=%s\n' "$RUN_TEST_TYPESCRIPT"
printf 'RUN_TEST_RUBY=%s\n' "$RUN_TEST_RUBY"
printf 'RUN_TEST_AGENTS_TS=%s\n' "$RUN_TEST_AGENTS_TS"
printf 'SCOPE_SUMMARY=%s\n' "$SCOPE_SUMMARY"

if [ "$USE_GITHUB_OUTPUT" = "true" ] && [ -n "${GITHUB_OUTPUT:-}" ]; then
    {
        echo "docs-only=${DOCS_ONLY}"
        echo "rust-inert=${RUST_INERT}"
        echo "run-code-quality=${RUN_CODE_QUALITY}"
        echo "run-rust-quality=${RUN_RUST_QUALITY}"
        echo "run-test-rust=${RUN_TEST_RUST}"
        echo "run-test-typescript=${RUN_TEST_TYPESCRIPT}"
        echo "run-test-ruby=${RUN_TEST_RUBY}"
        echo "run-test-agents-ts=${RUN_TEST_AGENTS_TS}"
        echo "scope-summary=${SCOPE_SUMMARY}"
    } >> "$GITHUB_OUTPUT"
fi
