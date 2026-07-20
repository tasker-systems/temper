#!/usr/bin/env bash
# .github/scripts/test-detect-ci-scope.sh
#
# Test harness for detect-ci-scope.sh. Feeds mock file lists via --stdin and
# asserts the emitted KEY=VALUE flags. Run locally or in CI:
#   bash .github/scripts/test-detect-ci-scope.sh [--verbose]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
DETECT_SCRIPT="${SCRIPT_DIR}/detect-ci-scope.sh"
VERBOSE_FLAG=""
PASS=0
FAIL=0

if [ "${1:-}" = "--verbose" ]; then
    VERBOSE_FLAG="--verbose"
fi

run_test() {
    local test_name="$1"
    local file_list="$2"
    shift 2

    local output
    output="$(echo "$file_list" | bash "$DETECT_SCRIPT" --stdin $VERBOSE_FLAG 2>/dev/null)"

    local test_passed=true
    local failures=""
    while [ $# -gt 0 ]; do
        local assertion="$1"; shift
        local var_name="${assertion%%=*}"
        local expected="${assertion#*=}"
        local actual
        # `|| true`: an ABSENT key must report a FAIL with actual='', not kill the
        # harness via set -e. Without this, adding an assertion for a flag the
        # script does not yet emit aborts the whole run instead of going red.
        actual="$(echo "$output" | grep "^${var_name}=" | head -1 | cut -d= -f2- || true)"
        if [ "$actual" != "$expected" ]; then
            test_passed=false
            failures="${failures}    ${var_name}: expected='${expected}' actual='${actual}'
"
        fi
    done

    if [ "$test_passed" = "true" ]; then
        echo "  PASS: ${test_name}"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: ${test_name}"
        echo "$failures"
        FAIL=$((FAIL + 1))
    fi
}

echo "Running detect-ci-scope.sh tests..."
echo ""

# --- docs-only: skip everything ---
run_test "docs-only: all jobs skipped" \
    "README.md
docs/superpowers/plans/2026-07-03-t7-block-provenance-write-path.md
CLAUDE.md" \
    "DOCS_ONLY=true" \
    "RUN_CODE_QUALITY=false" \
    "RUN_TEST_RUST=false" \
    "RUN_TEST_TYPESCRIPT=false" \
    "RUN_TEST_RUBY=false" \
    "SCOPE_SUMMARY=docs-only: skipping code-quality, test-rust, test-typescript, test-ruby, test-agents-ts"

# --- per-crate doc files are still docs-only ---
run_test "crate-dir docs only: docs-only scope" \
    "crates/temper-core/README.md
crates/temper-mcp/CLAUDE.md" \
    "DOCS_ONLY=true" \
    "RUN_CODE_QUALITY=false" \
    "RUN_TEST_RUST=false" \
    "RUN_TEST_TYPESCRIPT=false"

# --- single rust source file: full CI ---
run_test "rust source change: full CI" \
    "crates/temper-services/src/services/search_service.rs" \
    "DOCS_ONLY=false" \
    "RUN_CODE_QUALITY=true" \
    "RUN_TEST_RUST=true" \
    "RUN_TEST_TYPESCRIPT=true" \
    "SCOPE_SUMMARY=full-ci: code change detected — running full pipeline (test-ruby=false, test-agents-ts=false)"

# --- typescript source change: full CI ---
run_test "typescript source change: full CI" \
    "packages/temper-cloud/src/logger.ts" \
    "DOCS_ONLY=false" \
    "RUN_CODE_QUALITY=true" \
    "RUN_TEST_RUST=true" \
    "RUN_TEST_TYPESCRIPT=true"

# --- migration change: full CI (not a doc) ---
run_test "migration change: full CI" \
    "migrations/20260705000001_something.sql" \
    "DOCS_ONLY=false" \
    "RUN_TEST_RUST=true"

# --- sqlx cache change: full CI ---
run_test "sqlx cache change: full CI" \
    ".sqlx/query-abc123.json" \
    "DOCS_ONLY=false" \
    "RUN_CODE_QUALITY=true"

# --- mixed docs + code: code wins (docs never reduce scope) ---
run_test "mixed docs+rust: full CI" \
    "README.md
crates/temper-cli/src/main.rs" \
    "DOCS_ONLY=false" \
    "RUN_CODE_QUALITY=true" \
    "RUN_TEST_RUST=true"

# --- self-referential: full CI even though only docs+script touched ---
run_test "detect script changed: full CI (exercised, not skipped)" \
    ".github/scripts/detect-ci-scope.sh
docs/some-note.md" \
    "DOCS_ONLY=false" \
    "RUN_CODE_QUALITY=true" \
    "RUN_TEST_RUST=true" \
    "RUN_TEST_TYPESCRIPT=true"

# --- self-referential test file: also full CI ---
run_test "detect test script changed: full CI" \
    ".github/scripts/test-detect-ci-scope.sh" \
    "DOCS_ONLY=false" \
    "RUN_CODE_QUALITY=true"

# --- workflow yaml change (non-doc): full CI ---
run_test "ci workflow change: full CI" \
    ".github/workflows/ci.yml" \
    "DOCS_ONLY=false" \
    "RUN_CODE_QUALITY=true"

# --- empty/forced fallback: full CI ---
run_test "no-diff fallback: full CI" \
    "__force_full_ci__" \
    "DOCS_ONLY=false" \
    "RUN_CODE_QUALITY=true" \
    "RUN_TEST_RUST=true"

# ---------------------------------------------------------------------------
# test-ruby is the one PATH-SCOPED job: it needs Docker for the codegen drift
# gate, so it stays off PRs that cannot possibly affect the gem. Every other
# job is all-or-nothing on docs-only. These cases pin both directions.
# ---------------------------------------------------------------------------

run_test "ruby gem source change: test-ruby runs" \
    "clients/temper-rb/lib/temper/client.rb" \
    "DOCS_ONLY=false" \
    "RUN_TEST_RUBY=true"

# The contract is the gem's generator input, so a contract change must be SEEN
# to move the gem -- that is what the drift gate exists to prove.
run_test "openapi.json change: test-ruby runs" \
    "openapi.json" \
    "DOCS_ONLY=false" \
    "RUN_TEST_RUBY=true"

run_test "ruby CI workflow change: test-ruby runs" \
    ".github/workflows/test-ruby.yml" \
    "RUN_TEST_RUBY=true"

run_test "unrelated rust change: test-ruby skipped, rust runs" \
    "crates/temper-api/src/handlers/resources.rs" \
    "RUN_TEST_RUBY=false" \
    "RUN_TEST_RUST=true"

run_test "unrelated typescript change: test-ruby skipped" \
    "packages/temper-ui/src/routes/+page.svelte" \
    "RUN_TEST_RUBY=false"

# The gem's own markdown is still just markdown.
run_test "gem README only: docs-only, test-ruby skipped" \
    "clients/temper-rb/README.md" \
    "DOCS_ONLY=true" \
    "RUN_TEST_RUBY=false"

# Mixed: docs never reduce scope, and the gem source still turns test-ruby on.
run_test "mixed docs + gem source: test-ruby runs" \
    "README.md
clients/temper-rb/lib/temper/errors.rb" \
    "DOCS_ONLY=false" \
    "RUN_TEST_RUBY=true"

# Self-referential changes force every job on, test-ruby included.
run_test "detect script changed: test-ruby runs too" \
    ".github/scripts/detect-ci-scope.sh" \
    "RUN_TEST_RUBY=true"

# The no-diff safety fallback must run EVERYTHING, including the path-scoped job.
run_test "no-diff fallback: test-ruby runs" \
    "__force_full_ci__" \
    "RUN_TEST_RUBY=true"

run_test "contract change triggers the ruby gem spec that asserts it" \
    "tests/contracts/m2m-token-request.json" \
    "DOCS_ONLY=false" "RUN_TEST_RUBY=true"

run_test "temper-ts change runs the TS SDK + agent job" \
    "clients/temper-ts/src/credentials.ts" \
    "DOCS_ONLY=false" "RUN_TEST_AGENTS_TS=true"

run_test "steward change runs the TS SDK + agent job" \
    "packages/agent-workflows/steward/agent/agent.ts" \
    "DOCS_ONLY=false" "RUN_TEST_AGENTS_TS=true"

run_test "contract change runs the TS SDK + agent job (temper-ts asserts it)" \
    "tests/contracts/m2m-token-request.json" \
    "DOCS_ONLY=false" "RUN_TEST_AGENTS_TS=true"

run_test "an unrelated rust change does not run the TS SDK + agent job" \
    "crates/temper-api/src/main.rs" \
    "DOCS_ONLY=false" "RUN_TEST_AGENTS_TS=false"

run_test "docs-only skips the TS SDK + agent job" \
    "README.md" \
    "DOCS_ONLY=true" "RUN_TEST_AGENTS_TS=false"

# --- openapi.json alone must run BOTH SDK jobs: each has a codegen drift gate
# --- against it, and a gate the contract change does not run is not a gate.
run_test "openapi.json change: runs both SDK drift gates" \
    "openapi.json" \
    "DOCS_ONLY=false" \
    "RUN_TEST_RUBY=true" \
    "RUN_TEST_AGENTS_TS=true"

# ---------------------------------------------------------------------------
# A gate's own IMPLEMENTATION must run the gate. These four cases exist because
# `git diff --exit-code -- <path-that-matches-nothing>` exits 0: a "chore: tidy
# the drift scripts" PR that typos the GENERATED path turns the gate into a
# permanent no-op that always passes — and, touching only .github/scripts/*.sh,
# it would not have run the job whose gate it just killed. It merges green and
# every later PR inherits a dead gate. The scripts belong in the trigger set of
# the job they implement.
# ---------------------------------------------------------------------------

run_test "temper-ts drift script changed: runs the job it gates" \
    ".github/scripts/check-temper-ts-drift.sh" \
    "DOCS_ONLY=false" "RUN_TEST_AGENTS_TS=true"

run_test "temper-ts generator changed: runs the job it gates" \
    ".github/scripts/generate-temper-ts.sh" \
    "DOCS_ONLY=false" "RUN_TEST_AGENTS_TS=true"

run_test "temper-rb drift script changed: runs the job it gates" \
    ".github/scripts/check-temper-rb-drift.sh" \
    "DOCS_ONLY=false" "RUN_TEST_RUBY=true"

run_test "temper-rb generator changed: runs the job it gates" \
    ".github/scripts/generate-temper-rb.sh" \
    "DOCS_ONLY=false" "RUN_TEST_RUBY=true"

# The trigger keys are the SDK scripts specifically, not .github/scripts/ wholesale
# — an unrelated script there must not drag both SDK jobs onto every PR.
run_test "an unrelated .github script does not run either SDK job" \
    ".github/scripts/check-openapi-routes.sh" \
    "DOCS_ONLY=false" "RUN_TEST_RUBY=false" "RUN_TEST_AGENTS_TS=false"

# --- the security guards must run the job that runs THEM ---
#
# The SDK gates earn their trigger keys explicitly (above) because they are path-scoped jobs. The
# security tripwires live in code-quality, which has no path scoping — so they are covered by the
# plain "any non-doc change runs everything" rule rather than by a key of their own. That is a
# CONSEQUENCE of two independent decisions, not something anyone stated, and it is exactly the
# property that would rot silently if code-quality ever gained a path scope: the PR that disarms a
# guard would be the PR that never runs it. Assert it, so adding such a scope has to break a test
# instead of quietly un-gating the security tripwires.
run_test "editing a security guard runs code-quality (the job that runs it)" \
    ".github/scripts/audit-route-auth.sh" \
    "DOCS_ONLY=false" "RUN_CODE_QUALITY=true"

run_test "editing a guard's own test harness runs code-quality too" \
    ".github/scripts/test-audit-signature-secrets.sh" \
    "DOCS_ONLY=false" "RUN_CODE_QUALITY=true"

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
