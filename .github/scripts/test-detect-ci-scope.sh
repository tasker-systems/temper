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
        actual="$(echo "$output" | grep "^${var_name}=" | head -1 | cut -d= -f2-)"
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
    "SCOPE_SUMMARY=docs-only: skipping code-quality, test-rust, test-typescript"

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
    "SCOPE_SUMMARY=full-ci: code change detected — running full pipeline"

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

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
