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
# `internal/` and README.md are gate-owned, but by bash gates in quality-gate.yml's
# guard-tests job, which runs on every change without consulting this script (the
# structural pins at the end of this file). So nothing here turns on. The fixture
# names a SURVIVING internal path deliberately: the old one pointed into
# internal/superpowers/, which moved to temper-artifacts.
run_test "docs-only: everything this script decides is off" \
    "README.md
internal/agents/architecture.md
CLAUDE.md" \
    "DOCS_ONLY=true" \
    "RUN_CODE_QUALITY=false" \
    "RUN_TEST_RUST=false" \
    "RUN_TEST_TYPESCRIPT=false" \
    "RUN_TEST_RUBY=false" \
    "SCOPE_SUMMARY=docs-only: skipping code-quality, test-rust, test-typescript, test-ruby, test-python, test-agents-ts and quality-gate's rust-gate; guard-tests still runs (the docs/ and internal/ gates)"

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
    "SCOPE_SUMMARY=full-ci: code change detected — running full pipeline (test-ruby=false, test-python=false, test-agents-ts=false)"

# --- typescript source change: rust-inert (the corpus is inert to it), but
# --- code-quality is still invoked (its TS + guard jobs) and TS tests run.
# --- Covered in depth in the RUST-INERT section below; pinned here beside the
# --- rust-source case so the language asymmetry is visible at a glance.
run_test "typescript source change: rust-inert, TypeScript runs" \
    "packages/temper-cloud/src/logger.ts" \
    "DOCS_ONLY=false" \
    "RUST_INERT=true" \
    "RUN_CODE_QUALITY=true" \
    "RUN_RUST_QUALITY=false" \
    "RUN_TEST_RUST=false" \
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

run_test "temper-telemetry-ts change runs the TS SDK + agent job" \
    "clients/temper-telemetry-ts/src/otel.ts" \
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

# --- the Python SDK: path-scoped exactly as the gem is ---
run_test "temper-py source: runs test-python, rust corpus stays off" \
    "clients/temper-py/temper/client.py" \
    "DOCS_ONLY=false" \
    "RUST_INERT=true" \
    "RUN_TEST_PYTHON=true" \
    "RUN_TEST_RUBY=false" \
    "RUN_TEST_AGENTS_TS=false"

run_test "temper-py generated tree: runs test-python" \
    "clients/temper-py/temper/generated/api/resources_api.py" \
    "RUN_TEST_PYTHON=true"

# The contract restales every SDK at once, so it must run every SDK's drift gate.
# An openapi.json change that ran only two of the three would be a contract change
# whose third gate never fires — the exact gap that bit #354 for the gem.
run_test "openapi.json: runs every SDK job, not just some" \
    "openapi.json" \
    "RUN_TEST_PYTHON=true" \
    "RUN_TEST_RUBY=true" \
    "RUN_TEST_AGENTS_TS=true"

# credentials are pinned against the shared M2M contract, like the gem's and the
# TS client's.
run_test "M2M wire contract: runs test-python too" \
    "tests/contracts/m2m-token-request.json" \
    "RUN_TEST_PYTHON=true" \
    "RUN_TEST_RUBY=true" \
    "RUN_TEST_AGENTS_TS=true"

run_test "temper-py drift script changed: runs the job it gates" \
    ".github/scripts/check-temper-py-drift.sh" \
    "DOCS_ONLY=false" "RUN_TEST_PYTHON=true"

run_test "temper-py generator changed: runs the job it gates" \
    ".github/scripts/generate-temper-py.sh" \
    "DOCS_ONLY=false" "RUN_TEST_PYTHON=true"

run_test "test-python workflow changed: runs itself" \
    ".github/workflows/test-python.yml" \
    "DOCS_ONLY=false" "RUN_TEST_PYTHON=true"

# A Rust change cannot reach the Python SDK except through the contract, which is
# keyed above — so the job stays off and the runner minutes stay unspent.
run_test "rust source alone: test-python stays off" \
    "crates/temper-services/src/services/search_service.rs" \
    "RUN_TEST_PYTHON=false"

run_test "temper-py README alone: docs, so nothing runs" \
    "clients/temper-py/README.md" \
    "DOCS_ONLY=true" \
    "RUN_TEST_PYTHON=false"

run_test "self-edit forces every path-scoped job on" \
    ".github/scripts/detect-ci-scope.sh" \
    "RUN_TEST_PYTHON=true" \
    "RUN_TEST_RUBY=true" \
    "RUN_TEST_AGENTS_TS=true"

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

# ---------------------------------------------------------------------------
# RUST-INERT skip. A change whose every non-doc file lives in a Rust-inert tree
# (the TS packages, the SDK clients) skips the Rust corpus — test-rust AND the
# Rust jobs inside code-quality and quality-gate (RUN_RUST_QUALITY=false) —
# while code-quality is still INVOKED (RUN_CODE_QUALITY=true) so its TypeScript
# job runs, and TypeScript tests run too.
#
# The skip is ONE-DIRECTIONAL: a Rust change never skips TypeScript (ts-rs
# generates TS from Rust), and any edit to a Rust-COUPLED committed artifact
# (a generated TS tree, the skills/ projection, openapi.json, a wire contract) VETOES the
# skip because a Rust quality gate regenerates-and-diffs it.
# ---------------------------------------------------------------------------

# A hand-written temper-cloud TS change: the Rust corpus is inert to it.
run_test "temper-cloud TS: rust-inert skips the rust corpus, TS still runs" \
    "packages/temper-cloud/src/logger.ts" \
    "DOCS_ONLY=false" \
    "RUST_INERT=true" \
    "RUN_CODE_QUALITY=true" \
    "RUN_RUST_QUALITY=false" \
    "RUN_TEST_RUST=false" \
    "RUN_TEST_TYPESCRIPT=true"

# A hand-written temper-ui change is inert to the rust corpus too. (It still
# runs test-typescript's UI job — that is gated by RUN_TEST_TYPESCRIPT, on here.)
run_test "temper-ui hand-written: rust-inert" \
    "packages/temper-ui/src/routes/+page.svelte" \
    "RUST_INERT=true" \
    "RUN_RUST_QUALITY=false" \
    "RUN_TEST_RUST=false" \
    "RUN_TEST_TYPESCRIPT=true"

# The Ruby gem is inert to cargo for the same `members`/`workspaces` reason, so a
# gem-only change skips the rust corpus while test-ruby still runs.
run_test "ruby gem only: rust-inert, test-ruby still runs" \
    "clients/temper-rb/lib/temper/client.rb" \
    "RUST_INERT=true" \
    "RUN_RUST_QUALITY=false" \
    "RUN_TEST_RUST=false" \
    "RUN_TEST_RUBY=true"

# temper-ts / telemetry-ts / agent-workflows are inert to the rust corpus; their
# own drift gates ride test-agents-ts (keyed separately), not rust-quality.
run_test "temper-ts SDK source: rust-inert, agents-ts runs" \
    "clients/temper-ts/src/credentials.ts" \
    "RUST_INERT=true" \
    "RUN_RUST_QUALITY=false" \
    "RUN_TEST_RUST=false" \
    "RUN_TEST_AGENTS_TS=true"

run_test "agent-workflows steward source: rust-inert, agents-ts runs" \
    "packages/agent-workflows/steward/agent/agent.ts" \
    "RUST_INERT=true" \
    "RUN_RUST_QUALITY=false" \
    "RUN_TEST_RUST=false" \
    "RUN_TEST_AGENTS_TS=true"

# docs never force the rust corpus back on: docs + inert TS is still rust-inert.
run_test "docs + temper-cloud TS: still rust-inert" \
    "README.md
packages/temper-cloud/src/x.ts" \
    "DOCS_ONLY=false" \
    "RUST_INERT=true" \
    "RUN_RUST_QUALITY=false" \
    "RUN_TEST_RUST=false"

# --- VETO cases: a Rust-coupled committed artifact forces the rust corpus on ---

# The ts-rs generated tree under temper-ui: a manual edit must run ts-rs-drift
# (rust-quality). Living under an inert root must NOT wave it through.
run_test "temper-ui ts-rs generated tree: VETO, rust corpus runs" \
    "packages/temper-ui/src/lib/types/generated/ResourceRow.ts" \
    "RUST_INERT=false" \
    "RUN_RUST_QUALITY=true" \
    "RUN_TEST_RUST=true"

# The mention agent's ts-rs generated tree: same veto.
run_test "mention-agent ts-rs generated tree: VETO, rust corpus runs" \
    "packages/agent-workflows/mention/agent/generated/LinkRefusal.ts" \
    "RUST_INERT=false" \
    "RUN_RUST_QUALITY=true" \
    "RUN_TEST_RUST=true"

# openapi.json is regenerated-and-diffed by openapi-check in rust-quality.
run_test "openapi.json: VETO, rust corpus runs" \
    "openapi.json" \
    "RUST_INERT=false" \
    "RUN_RUST_QUALITY=true" \
    "RUN_TEST_RUST=true"

# skills/ is the committed projection gated by skills-drift in rust-quality.
# Its files are *.md, so without the RUST_COUPLED clause on DOCS_ONLY a hand-edit
# would be classified docs-only and skip the very gate that owns it — this pins
# that the coupling defeats the docs-only skip.
run_test "skills/ projection (*.md): defeats docs-only, rust corpus runs" \
    "skills/temper-knowledge-base/SKILL.md" \
    "DOCS_ONLY=false" \
    "RUST_INERT=false" \
    "RUN_RUST_QUALITY=true" \
    "RUN_TEST_RUST=true"

# docs/reference/ is the committed projection of the BUILT CLI's --help tree, gated
# by check-cli-reference-drift.sh in rust-quality. Same shape as agent-skills above
# and one step more treacherous: its files are *.md AND they sit under docs/, so both
# the extension test and the eye classify them as documentation. Measured before the
# coupling was added — a lone edit to a generated page gave DOCS_ONLY=true,
# SKIP_ALL=true, RUN_RUST_QUALITY=false, leaving a hand-edit to generated content with
# nothing whatsoever to catch it.
run_test "generated CLI reference (*.md under docs/): defeats docs-only, rust corpus runs" \
    "docs/reference/cli/config.md" \
    "DOCS_ONLY=false" \
    "RUST_INERT=false" \
    "RUN_RUST_QUALITY=true" \
    "RUN_TEST_RUST=true"

# The carve-out must stay a CARVE-OUT. This is the counterpart to the case above and
# the reason the entry is `^docs/reference/` rather than `^docs/`: an ordinary guide
# keeps the cheap path, because no Rust gate owns it.
run_test "an ordinary guide is untouched by the reference coupling" \
    "docs/guides/install.md" \
    "DOCS_ONLY=true" \
    "SKIP_ALL=true" \
    "RUN_CODE_QUALITY=false" \
    "RUN_RUST_QUALITY=false"

# The REST of docs/ is owned by check-docs-public-only.sh, which lives in quality-gate's
# guard-tests job. The regression that gate exists to catch is a returning internal
# tree, which is pure markdown — so the extension test classifies it docs-only. What
# a docs/ change owes is REACHABILITY, and it gets it structurally: neither
# guard-tests nor ci.yml's call to quality-gate carries an `if:` (pinned below), so
# no verdict from this script can switch the gate off.
#
# What it does NOT owe is the Rust corpus. `^docs/` used to sit in RUST_COUPLED,
# which bought reachability by conscripting the whole pipeline — a one-line docs
# edit ran test-rust + test-typescript + rust-quality to reach one second of bash.
# These assertions previously encoded exactly that (RUN_RUST_QUALITY=true, and
# DOCS_ONLY/SKIP_ALL forced false); they now pin the narrower contract. The
# RUN_*=false lines are the load-bearing half — without them the widening could
# come back unnoticed.
run_test "docs/ page (*.md): a full skip — the docs gate is reached structurally" \
    "docs/superpowers/plans/2026-08-19-something.md" \
    "DOCS_ONLY=true" \
    "SKIP_ALL=true" \
    "RUST_INERT=false" \
    "RUN_CODE_QUALITY=false" \
    "RUN_RUST_QUALITY=false" \
    "RUN_TEST_RUST=false" \
    "RUN_TEST_TYPESCRIPT=false" \
    "RUN_TEST_RUBY=false" \
    "RUN_TEST_AGENTS_TS=false"

# Mixed with a non-product tree: still nothing load-bearing, so SKIP_ALL holds.
run_test "docs/ page + non-product spike tree: still a full skip" \
    "docs/guides/install.md
scripts/wayfind-spike/queries.sql" \
    "SKIP_ALL=true" \
    "NON_PRODUCT=true" \
    "RUN_CODE_QUALITY=false" \
    "RUN_RUST_QUALITY=false" \
    "RUN_TEST_RUST=false"

# Mixed with an inert TS file there IS something load-bearing, so the rust-inert arm
# takes over: code-quality is invoked for its TypeScript job and TS tests run,
# but the Rust corpus stays off — a docs/ page must not drag it back on.
run_test "docs/ page + inert TS: rust-inert, the Rust corpus stays off" \
    "docs/guides/install.md
packages/temper-cloud/src/logger.ts" \
    "DOCS_ONLY=false" \
    "RUST_INERT=true" \
    "RUN_CODE_QUALITY=true" \
    "RUN_RUST_QUALITY=false" \
    "RUN_TEST_RUST=false" \
    "RUN_TEST_TYPESCRIPT=true"

# Markdown outside docs/ is the same full skip — no arm of this script tells doc
# trees apart any more; which bash gate reads which tree is guard-tests' business.
run_test "markdown outside docs/: a full skip as well" \
    "crates/temper-core/README.md" \
    "DOCS_ONLY=true" \
    "SKIP_ALL=true" \
    "RUN_CODE_QUALITY=false"

# A wire contract is asserted from the Rust side too.
run_test "wire contract: VETO, rust corpus runs" \
    "tests/contracts/m2m-token-request.json" \
    "RUST_INERT=false" \
    "RUN_RUST_QUALITY=true" \
    "RUN_TEST_RUST=true"

# --- the one-directional guarantee and conservative defaults ---

# A Rust change NEVER skips TypeScript (ts-rs generates TS from Rust).
run_test "rust source: full corpus, TypeScript still runs (one-directional)" \
    "crates/temper-services/src/services/search_service.rs" \
    "RUST_INERT=false" \
    "RUN_RUST_QUALITY=true" \
    "RUN_TEST_RUST=true" \
    "RUN_TEST_TYPESCRIPT=true"

# Mixed rust + inert TS: the rust part wins, full corpus runs.
run_test "mixed rust + temper-cloud TS: full corpus" \
    "crates/temper-api/src/main.rs
packages/temper-cloud/src/x.ts" \
    "RUST_INERT=false" \
    "RUN_RUST_QUALITY=true" \
    "RUN_TEST_RUST=true"

# An UNRECOGNIZED non-doc file (outside every inert root) is conservatively
# treated as rust-affecting — unknown never skips.
run_test "unknown top-level non-doc file: not rust-inert (conservative)" \
    "some-new-tool.sh" \
    "RUST_INERT=false" \
    "RUN_RUST_QUALITY=true" \
    "RUN_TEST_RUST=true"

# A self-edit to the detector forces a full run even when the rest is inert TS,
# so the change is actually exercised.
run_test "self-edit + inert TS: full corpus (exercised, not skipped)" \
    ".github/scripts/detect-ci-scope.sh
packages/temper-cloud/src/x.ts" \
    "RUST_INERT=false" \
    "RUN_RUST_QUALITY=true" \
    "RUN_TEST_RUST=true"

# docs-only leaves the new axes off (RUST_INERT is a non-docs concept). README.md
# is read by the public-surface drift gate, which runs in guard-tests and so is
# reached on this change without this script turning anything on.
run_test "README.md: docs-only, everything this script decides is off" \
    "README.md" \
    "DOCS_ONLY=true" \
    "RUST_INERT=false" \
    "RUN_CODE_QUALITY=false" \
    "RUN_RUST_QUALITY=false" \
    "RUN_TEST_RUST=false"

# Every OTHER root-level doc keeps the old property: docs-only, everything off.
run_test "other root doc: docs-only, rust-quality off, code-quality off" \
    "CONTRIBUTING.md" \
    "DOCS_ONLY=true" \
    "RUST_INERT=false" \
    "RUN_CODE_QUALITY=false" \
    "RUN_RUST_QUALITY=false" \
    "RUN_TEST_RUST=false"

# The no-diff safety fallback runs everything, rust-quality included.
run_test "no-diff fallback: rust-quality runs" \
    "__force_full_ci__" \
    "RUST_INERT=false" \
    "RUN_RUST_QUALITY=true" \
    "RUN_TEST_RUST=true"

# ---------------------------------------------------------------------------
# NON-PRODUCT ROOTS — scripts/wayfind-spike/ is consumed by nothing, so a change
# confined to it skips the whole pipeline exactly as docs do. DOCS_ONLY stays
# FALSE throughout: the summary must not call a pile of .sql and .tsv "docs".
# ---------------------------------------------------------------------------
run_test "non-product only: whole pipeline skipped, but NOT reported as docs" \
    "scripts/wayfind-spike/h1-orient.sql
scripts/wayfind-spike/gen_salnorm_reach.py
scripts/wayfind-spike/vectors-24.tsv" \
    "SKIP_ALL=true" \
    "DOCS_ONLY=false" \
    "NON_PRODUCT=true" \
    "RUN_CODE_QUALITY=false" \
    "RUN_TEST_RUST=false" \
    "RUN_TEST_TYPESCRIPT=false"

run_test "non-product + its own README: still skipped, still not docs-only" \
    "scripts/wayfind-spike/README.md
scripts/wayfind-spike/h6-mechanism.sql" \
    "SKIP_ALL=true" \
    "DOCS_ONLY=false" \
    "RUN_CODE_QUALITY=false" \
    "RUN_TEST_RUST=false"

# The load-bearing direction: one real file anywhere else and the skip is off.
run_test "non-product + a crate file: full CI" \
    "scripts/wayfind-spike/h1-orient.sql
crates/temper-api/src/lib.rs" \
    "SKIP_ALL=false" \
    "RUN_CODE_QUALITY=true" \
    "RUN_TEST_RUST=true"

# A sibling under scripts/ is NOT covered — inertness is proven per tree, never
# inherited from a top-level directory name.
# ---------------------------------------------------------------------------
# internal/ — the process tree. Added to NON_PRODUCT_ROOTS because it is
# overwhelmingly .md (already handled by the extension test) but NOT entirely:
# `internal/registers/coverage.yaml` is a projection over a REMOTE knowledge
# base, so it is committed on its own and often. Being .yaml it failed the
# extension test, and cc280f98 — that one file, alone — therefore scoped as
# full-ci and ran the entire pipeline.
#
# The .yaml case is the one that matters, so it is asserted directly rather
# than through a .md that would pass for the wrong reason.
# ---------------------------------------------------------------------------
run_test "a register reprojection alone: whole pipeline skipped (cc280f98)" \
    "internal/registers/coverage.yaml" \
    "SKIP_ALL=true" \
    "DOCS_ONLY=false" \
    "NON_PRODUCT=true" \
    "RUN_CODE_QUALITY=false" \
    "RUN_TEST_RUST=false" \
    "RUN_TEST_TYPESCRIPT=false"

# THE GATE'S OWN FAILURE MODE. A session re-creating internal/superpowers/ from a
# stale instruction produces exactly this change: one markdown file under a
# non-product root. It is a full skip here; check-no-process-artifacts.sh reaches
# it from guard-tests, which no scope verdict can switch off.
run_test "an internal spec alone: a full skip — the process-artifact gate is reached structurally" \
    "internal/superpowers/specs/2026-07-30-schema-binary-pairing-design.md" \
    "SKIP_ALL=true" \
    "DOCS_ONLY=true" \
    "RUN_CODE_QUALITY=false" \
    "RUN_TEST_RUST=false"

# The load-bearing direction, asserted for this root as for every other: the
# skip must be off the moment one real file joins the changeset.
run_test "internal/ + a crate file: full CI" \
    "internal/registers/coverage.yaml
crates/temper-api/src/lib.rs" \
    "SKIP_ALL=false" \
    "RUN_CODE_QUALITY=true" \
    "RUN_TEST_RUST=true"

# A change that moves a page out of internal/ and into docs/ carries both paths.
# Still a full skip: check-docs-public-only.sh — the gate that exists because
# internal security audits were once published — runs from guard-tests regardless.
run_test "internal/ + docs/: a full skip" \
    "internal/code-reviews/2026-07-18-audit.md
docs/guides/getting-started.md" \
    "SKIP_ALL=true" \
    "RUN_CODE_QUALITY=false" \
    "RUN_TEST_RUST=false" \
    "RUN_TEST_TYPESCRIPT=false"

run_test "a different scripts/ subtree is not non-product" \
    "scripts/install/install.sh" \
    "SKIP_ALL=false" \
    "NON_PRODUCT=false" \
    "RUN_TEST_RUST=true"

# Mixed non-product + inert TS is still rust-inert (probes cannot reach a crate).
run_test "non-product + inert TS: rust-inert, TS still runs" \
    "scripts/wayfind-spike/h1-orient.sql
packages/temper-cloud/src/x.ts" \
    "SKIP_ALL=false" \
    "RUST_INERT=true" \
    "RUN_TEST_RUST=false" \
    "RUN_TEST_TYPESCRIPT=true"

# A self-edit still forces a full run, even with everything else inert.
run_test "self-edit + non-product: full run (exercised, not skipped)" \
    ".github/scripts/detect-ci-scope.sh
scripts/wayfind-spike/h1-orient.sql" \
    "SKIP_ALL=false" \
    "RUN_CODE_QUALITY=true" \
    "RUN_TEST_RUST=true"

# ---------------------------------------------------------------------------
# REGRESSION — doc-extension files a Rust gate OWNS must never be docs-only.
# All three were measured skipping the ENTIRE pipeline, including the very gate
# that reads them. Each assertion below fails against the pre-fix script.
# ---------------------------------------------------------------------------
run_test "skill-content .md: source of the agent-skills projection, runs skills-drift" \
    "crates/temper-cli/skill-content/workflows/plan-small.md" \
    "DOCS_ONLY=false" \
    "SKIP_ALL=false" \
    "RUN_CODE_QUALITY=true" \
    "RUN_TEST_RUST=true"

run_test "containment corpus .txt: include_str!d by the path-traversal tests" \
    "scripts/install/containment-corpus.txt" \
    "DOCS_ONLY=false" \
    "SKIP_ALL=false" \
    "RUN_CODE_QUALITY=true" \
    "RUN_TEST_RUST=true"

run_test "migration-declaration corpus .txt: held by both parsers" \
    "scripts/migration-declaration-corpus.txt" \
    "DOCS_ONLY=false" \
    "SKIP_ALL=false" \
    "RUN_CODE_QUALITY=true" \
    "RUN_TEST_RUST=true"

# A RUST_COUPLED doc cannot be laundered by pairing it with a non-product file.
run_test "coupled doc + non-product: veto survives the wider predicate" \
    "scripts/install/containment-corpus.txt
scripts/wayfind-spike/h1-orient.sql" \
    "SKIP_ALL=false" \
    "RUN_TEST_RUST=true"

# ---------------------------------------------------------------------------
# CLASS TRIPWIRE — the three fixes above are instances. This guards the CLASS by
# deriving the set from the SOURCE: every doc-extension file `include_str!`d
# anywhere in the Rust corpus must be vetoed. A new one added later reopens the
# hole silently, and the enumerated list in detect-ci-scope.sh cannot notice.
# ---------------------------------------------------------------------------
assert_every_compiled_in_doc_is_vetoed() {
    local repo_root unresolved="" checked=0
    repo_root="$(cd "${SCRIPT_DIR}/../.." && pwd)"

    # grep, not ripgrep: `rg` is not guaranteed on a CI runner (and in some local
    # shells it is a function, not a binary, so `command -v` lies). There is
    # deliberately NO "tool missing -> skip" arm. A gate that can silently
    # no-op is the dead-gate failure this suite exists to prevent, so if the
    # scan cannot run it must go RED.
    local hits
    hits="$(cd "$repo_root" && grep -rn --include='*.rs' -E 'include_str!|include_bytes!' \
              crates tests 2>/dev/null | grep -E '"[^"]*\.(md|txt|adoc)"' || true)"

    if [ -z "$hits" ]; then
        echo "  FAIL: compiled-in-doc scan found NOTHING — the scan itself is broken"
        echo "    (agent-skills/knowledge-base.md and the two corpora are known to exist)"
        FAIL=$((FAIL + 1))
        return 0
    fi

    while IFS= read -r hit; do
        [ -n "$hit" ] || continue
        local src rel abs resolved out
        src="${hit%%:*}"
        rel="$(echo "$hit" | sed -E 's/.*"([^"]*\.(md|txt|adoc))".*/\1/')"
        abs="$(cd "${repo_root}/$(dirname "$src")" 2>/dev/null && cd "$(dirname "$rel")" 2>/dev/null && pwd || true)"
        [ -n "$abs" ] || continue
        resolved="${abs#"${repo_root}/"}/$(basename "$rel")"
        checked=$((checked + 1))

        # Ask the detector directly: would a lone edit to this file skip CI?
        out="$(printf '%s\n' "$resolved" | bash "$DETECT_SCRIPT" --stdin 2>/dev/null)"
        if echo "$out" | grep -q '^SKIP_ALL=true'; then
            unresolved="${unresolved}    ${resolved} (compiled into ${src}, yet SKIP_ALL=true)
"
        fi
    done <<EOF
$hits
EOF

    if [ -z "$unresolved" ]; then
        echo "  PASS: all ${checked} include_str!d doc files are vetoed out of the skip"
        PASS=$((PASS + 1))
    else
        echo "  FAIL: a doc file is compiled into Rust but still skips CI"
        echo "$unresolved"
        echo "    -> add it to RUST_COUPLED in detect-ci-scope.sh"
        FAIL=$((FAIL + 1))
    fi
}

assert_every_compiled_in_doc_is_vetoed

# ---------------------------------------------------------------------------
# THE QUALITY GATE IS UNCONDITIONAL, AND NOTHING THAT COMPILES RUST STARTS
# WITHOUT IT.
#
# Two properties, both structural, both spanning ci.yml and quality-gate.yml, so
# each is asserted at every link rather than at the ends.
#
# REACHABILITY. Every assertion above that leaves code-quality OFF for a docs/,
# internal/ or README.md change is safe only because the bash gates that own
# those trees run in quality-gate.yml's guard-tests job, and that job is reached
# on EVERY change: ci.yml calls the workflow with no `if:`, and guard-tests
# carries none of its own. Either `if:` appearing makes those skips unsafe — the
# gate would be present and unreachable on its own failure mode.
#
# ORDERING. A red gate must stop the Rust-compiling calls (code-quality,
# test-rust) from starting, and must never read green. ci-success runs
# `always()`, so it sees the gate only if quality-gate is in its `needs:` AND is
# passed to check_job as always-in-scope: without that, a red gate on a skip-all
# change leaves every downstream job `skipped` with should_run=false, and
# check_job prints "correctly skipped". The downstream `if:` must not accept a
# `skipped` gate (the gate is never legitimately skipped, so that only opens a
# way to run ungated) and must not use `always()`.
# ---------------------------------------------------------------------------

# Line range of a top-level job block: prints "<start> <end>" (end exclusive),
# or nothing when the job is absent.
job_block() {
    local file="$1" job="$2" start next
    start="$(grep -n "^  ${job}:\$" "$file" | head -1 | cut -d: -f1 || true)"
    [ -n "$start" ] || return 0
    next="$(awk -v s="$start" 'NR > s && /^  [a-z][a-z0-9-]*:$/ { print NR; exit }' "$file")"
    # `tr -d`: BSD `wc -l` pads its output, and the range is compared numerically.
    [ -n "$next" ] || next="$(( $(wc -l < "$file" | tr -d '[:space:]') + 1 ))"
    echo "$start $next"
}

# Prints the job-level `if:` line inside the block, if any.
job_if() {
    local file="$1" range="$2"
    awk -v a="${range% *}" -v b="${range#* }" 'NR > a && NR < b && /^    if:/ { print; exit }' "$file"
}

pass() { echo "  PASS: $1"; PASS=$((PASS + 1)); }
fail() { echo "  FAIL: $1"; shift; for l in "$@"; do echo "        $l"; done; FAIL=$((FAIL + 1)); }

assert_quality_gate_is_unconditional_and_first() {
    local repo_root ci qg cq range
    repo_root="$(cd "${SCRIPT_DIR}/../.." && pwd)"
    ci="${repo_root}/.github/workflows/ci.yml"
    qg="${repo_root}/.github/workflows/quality-gate.yml"
    cq="${repo_root}/.github/workflows/code-quality.yml"

    # --- reachability, link 1: ci.yml calls quality-gate.yml with no `if:` ---
    range="$(job_block "$ci" quality-gate)"
    if [ -z "$range" ]; then
        fail "no quality-gate job in ci.yml — the docs/ gate has no caller"
    elif ! awk -v a="${range% *}" -v b="${range#* }" 'NR > a && NR < b' "$ci" \
            | grep -qE '^    uses: \./\.github/workflows/quality-gate\.yml$'; then
        fail "ci.yml's quality-gate job does not call ./.github/workflows/quality-gate.yml"
    elif [ -n "$(job_if "$ci" "$range")" ]; then
        fail "ci.yml's quality-gate call grew an \`if:\` — the docs/ gate can now be" \
             "switched off, and every docs skip above is unsafe: $(job_if "$ci" "$range")"
    else
        pass "ci.yml calls quality-gate.yml unconditionally"
    fi

    # --- reachability, link 2: guard-tests carries no `if:` of its own ---
    range="$(job_block "$qg" guard-tests)"
    if [ -z "$range" ]; then
        fail "no guard-tests job in quality-gate.yml — the docs/ gate has no home"
    elif [ -n "$(job_if "$qg" "$range")" ]; then
        fail "guard-tests grew a job-level \`if:\` — the docs/ gate can now be switched" \
             "off, and every docs skip above is unsafe: $(job_if "$qg" "$range")"
    else
        pass "guard-tests carries no gate, so every change reaches docs/"
    fi

    # --- ordering, link 1: ci-success needs the gate and reads it as always in scope ---
    # Read inside ci-success's own block: code-quality and test-rust also list
    # quality-gate in their `needs:`, so a file-wide grep passes with it gone from here.
    range="$(job_block "$ci" ci-success)"
    if [ -n "$range" ] && awk -v a="${range% *}" -v b="${range#* }" 'NR > a && NR < b' "$ci" \
        | grep -qE '^    needs: \[.*\bquality-gate\b.*\]$'; then
        pass "ci-success needs quality-gate"
    else
        fail "ci-success does not list quality-gate in needs — its result is empty"
    fi
    # check_job's three arguments span three lines; the third must be the literal
    # "true". Scope-derived should_run here is exactly the green-on-red hole.
    if grep -A3 -E '^[[:space:]]*check_job "quality-gate" \\$' "$ci" \
        | grep -qE '^[[:space:]]*"true"[[:space:]]*$'; then
        pass "ci-success validates quality-gate as always in scope"
    else
        fail "ci-success does not call \`check_job \"quality-gate\" <result> \"true\"\` —" \
             "a red gate on an out-of-scope change would read green"
    fi

    # --- ordering, link 2: every Rust-compiling call waits on a SUCCESSFUL gate ---
    local job iff
    for job in code-quality test-rust; do
        range="$(job_block "$ci" "$job")"
        if [ -z "$range" ]; then
            fail "no ${job} job in ci.yml — this check is blind"
            continue
        fi
        if awk -v a="${range% *}" -v b="${range#* }" 'NR > a && NR < b' "$ci" \
            | grep -qE '^    needs: \[.*\bquality-gate\b.*\]$'; then
            pass "${job} needs quality-gate"
        else
            fail "${job} does not need quality-gate — it starts before the gate answers"
        fi
        iff="$(job_if "$ci" "$range")"
        if echo "$iff" | grep -qF "needs.quality-gate.result == 'success'" \
           && ! echo "$iff" | grep -qE "skipped|always\(\)"; then
            pass "${job} runs only on a successful gate"
        else
            fail "${job}'s \`if:\` does not require needs.quality-gate.result == 'success'," \
                 "or admits a skipped gate / always(): ${iff}"
        fi
    done

    # A guard-tests job left behind in code-quality.yml would be a second copy
    # whose gates run only when code-quality is invoked — the stale half of a move.
    if grep -qE '^  guard-tests:$' "$cq"; then
        fail "code-quality.yml still carries a guard-tests job — it moved to quality-gate.yml"
    else
        pass "guard-tests lives only in quality-gate.yml"
    fi
}

assert_quality_gate_is_unconditional_and_first

# ---------------------------------------------------------------------------
# EVERY PATH-SCOPED JOB MUST BE GATED *AND* VALIDATED.
#
# ci-success runs `if: always()`, so a job that fails does not fail the gate by
# itself — the gate's verdict comes entirely from its own `check_job` calls. A job
# listed in `needs:` but never passed to `check_job` is therefore a job whose red is
# read by nobody: the pipeline goes green with a failing SDK in it.
#
# The symmetric hole is a job wired into `needs:` with no `if:` — it would run on
# every PR, quietly restoring the bill the scoping exists to avoid.
#
# Asserted per LINK and per JOB rather than once, and derived from a list, so the
# fourth SDK is covered by adding one word here rather than by remembering three
# separate edits.
# ---------------------------------------------------------------------------
assert_scoped_jobs_are_gated_and_validated() {
    local repo_root ci
    repo_root="$(cd "${SCRIPT_DIR}/../.." && pwd)"
    ci="${repo_root}/.github/workflows/ci.yml"

    local job output
    for job in test-ruby test-python test-agents-ts; do
        output="run-${job}"

        # link 1: the detector emits the flag at all.
        # No `grep -q` here: -q exits at the first match, the detector's
        # remaining output lines hit the closed pipe, and under the harness's
        # `set -o pipefail` the matched pipeline reads as a failure — a race
        # that lands as an intermittent red on this exact assertion. Letting
        # grep drain the output (no -q, matches discarded) is deterministic.
        if bash "$DETECT_SCRIPT" --stdin </dev/null 2>/dev/null \
            | grep "^$(echo "$output" | tr 'a-z-' 'A-Z_')=" > /dev/null; then
            echo "  PASS: detect-ci-scope emits ${output}"
            PASS=$((PASS + 1))
        else
            echo "  FAIL: detect-ci-scope emits no ${output} — ci.yml's gate reads an"
            echo "        empty string, which is never 'true', so ${job} never runs"
            FAIL=$((FAIL + 1))
        fi

        # link 2: ci.yml gates the job on THAT output, not on a constant or a typo.
        if grep -qE "^[[:space:]]*if: needs\.detect-scope\.outputs\.${output} == 'true'$" "$ci"; then
            echo "  PASS: ci.yml gates ${job} on ${output}"
            PASS=$((PASS + 1))
        else
            echo "  FAIL: ci.yml does not gate ${job} on ${output} — it runs on every PR"
            FAIL=$((FAIL + 1))
        fi

        # link 3: ci-success reads the job's result. Without this its red is invisible.
        if grep -qE "^[[:space:]]*check_job \"${job}\" " "$ci"; then
            echo "  PASS: ci-success validates ${job}"
            PASS=$((PASS + 1))
        else
            echo "  FAIL: ci-success never calls check_job for ${job} — it runs \`always()\`,"
            echo "        so an unvalidated job failing leaves the pipeline green"
            FAIL=$((FAIL + 1))
        fi

        # link 4: and it must be in `needs:`, or `needs.<job>.result` is empty and
        # check_job compares against nothing.
        if grep -qE "^[[:space:]]*needs: \[.*\b${job}\b.*\]$" "$ci"; then
            echo "  PASS: ci-success needs ${job}"
            PASS=$((PASS + 1))
        else
            echo "  FAIL: ci-success does not list ${job} in needs — its result is empty"
            FAIL=$((FAIL + 1))
        fi
    done
}

assert_scoped_jobs_are_gated_and_validated

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
