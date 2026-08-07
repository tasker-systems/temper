#!/usr/bin/env bash
# audit-ignored-tests.sh — pin the set of `#[ignore]`d tests, each against a declared reason.
#
# WHY THIS EXISTS
# ---------------
# "Every test must run in CI, or loudly declare why it does not" has been enforced repeatedly on the
# ENABLEMENT half and never on the DECLARATION half. The enablement half has real teeth: selection is
# `--workspace`, so a new test is picked up with no CI edit, and every feature gate maps to a job
# (`test-db`, `test-embed`, `artifact-tests`, `scenario-schema`, `trybuild`). That half holds — it was
# audited on 2026-08-05 across all twelve crates against all five jobs, and nothing is silently
# compiled out.
#
# `#[ignore]` had nothing. Twenty of them, and NOTHING checked that any stated reason was still true.
#
# On 2026-08-05 three were checked. TWO WERE FALSE. Both `fts_search_test` ignores blamed a "collapsed
# search_select" that short-circuits past the FTS+vector combine; `search_select` calls
# `readback::unified_search` unconditionally (crates/temper-services/src/backend/substrate_read.rs).
# Run with `--run-ignored all`, both failed on something else entirely — the addressing collapse, a
# one-line fixture fix. So `unified_search`'s FTS+vector combine had NO e2e coverage, for a reason
# that had not been true for months.
#
# **A stale reason is worse than no reason.** A bare `#[ignore]` invites someone to ask why. A
# detailed one — and both false ones were long, specific, and cited issue numbers — is what STOPS
# anyone re-examining it. That is the same failure this repo already recorded once, when a gate's doc
# claimed it erred only toward false positives: a stated one-directional error is what stops anyone
# re-checking the other direction.
#
# WHAT THIS SCRIPT DOES AND DOES NOT DO
# -------------------------------------
# It does NOT prove any reason is true — no script can. It pins the SET, so a twenty-first cannot
# appear without a human writing down why, and so that REMOVING one is equally deliberate. Its real
# work is at review time: the diff makes an added `#[ignore]` a thing a reviewer must look at, and
# `--list` gives whoever is triaging a single command that prints every reason in the repo, which is
# what makes "re-read them" a bounded task rather than a grep expedition.
#
# It DOES hard-fail on a bare `#[ignore]` with no reason, with no baseline escape. That needs no
# judgment and no pinned set: an ignore that does not say why is the thing the rule already forbids.
#
# USAGE
#   .github/scripts/audit-ignored-tests.sh           # verify against the baseline (CI mode)
#   .github/scripts/audit-ignored-tests.sh --list    # print every ignore WITH its reason, for triage
#   UPDATE_BASELINE=1 .github/scripts/audit-ignored-tests.sh   # print a pasteable baseline
#
# Exit 0 = set unchanged and every ignore carries a reason.
# Exit 1 = an ignore was added, removed, moved file, or carries no reason.
#
# TEST SEAMS (used by test-audit-ignored-tests.sh, never in CI):
#   IGNORE_SCAN_ROOT     — scan a fixture tree instead of the repo
#   IGNORE_BASELINE_FILE — read the baseline from a file instead of the block below

set -euo pipefail

ROOT="${IGNORE_SCAN_ROOT:-$(git rev-parse --show-toplevel)}"
cd "$ROOT"

SCAN_DIRS=()
[[ -d crates ]] && SCAN_DIRS+=(crates)
[[ -d tests ]] && SCAN_DIRS+=(tests)
if [[ ${#SCAN_DIRS[@]} -eq 0 ]]; then
  echo "audit-ignored-tests: no crates/ or tests/ under $ROOT" >&2
  exit 1
fi

# Every `#[ignore` occurrence, as "<path>:<line>:<text>". `--include=*.rs` keeps this off fixtures
# and docs. Note it scans src/ as well as tests/ — two of the twenty live in `src/`.
scan_raw() {
  grep -rn --include='*.rs' -- '#\[ignore' "${SCAN_DIRS[@]}" 2>/dev/null || true
}

# Only attribute lines count. A doc comment that MENTIONS `#[ignore]` (there is one, in
# cloud_warmup_e2e_test.rs, explaining a past incident) must not inflate the count — that is a
# false positive that would make the baseline churn on prose edits.
scan_attrs() {
  scan_raw | grep -vE ':[[:space:]]*(///|//!|//|\*)' || true
}

current_counts() {
  scan_attrs | cut -d: -f1 | sort | uniq -c | awk '{printf "%s %s\n", $1, $2}' | sort -k2
}

# ── The pinned set ───────────────────────────────────────────────────────────────────────────────
# "<count> <path>". Regenerate with UPDATE_BASELINE=1 — and when you do, say in the PR which ignore
# you added or removed and why. A baseline updated on reflex is the failure this file exists to stop.
read -r -d '' BASELINE <<'EOF' || true
1 crates/temper-substrate/src/cluster.rs
1 crates/temper-substrate/tests/context_formation_cost.rs
2 tests/e2e/tests/cloud_session_link_e2e_test.rs
1 tests/e2e/tests/cloud_warmup_e2e_test.rs
3 tests/e2e/tests/cloud_writes_test.rs
1 tests/e2e/tests/fts_search_test.rs
5 tests/e2e/tests/projection_pull_test.rs
1 tests/e2e/tests/show_cache_e2e_test.rs
EOF
if [[ -n "${IGNORE_BASELINE_FILE:-}" ]]; then
  BASELINE="$(cat "$IGNORE_BASELINE_FILE")"
fi

# ── --list: every ignore with its reason, for triage ─────────────────────────────────────────────
if [[ "${1:-}" == "--list" ]]; then
  echo "# Every #[ignore] in the repo, with its stated reason."
  echo "# The reason is UNVERIFIED. Two of the three checked on 2026-08-05 were false."
  echo
  scan_attrs | while IFS= read -r line; do
    file="${line%%:*}"
    rest="${line#*:}"
    lineno="${rest%%:*}"
    text="${line#*:*:}"
    printf '%s:%s\n    %s\n\n' "$file" "$lineno" "$(echo "$text" | sed 's/^[[:space:]]*//')"
  done
  exit 0
fi

# ── Hard failure, no baseline escape: an ignore with no reason ───────────────────────────────────
# `#[ignore]` or `#[ignore ]` — anything without `= "..."`. This needs no pinned set and no judgment.
BARE="$(scan_attrs | grep -E '#\[ignore[[:space:]]*\]' || true)"
if [[ -n "$BARE" ]]; then
  echo "FAIL: #[ignore] with no reason." >&2
  echo >&2
  printf '%s\n' "$BARE" >&2
  echo >&2
  echo "Write #[ignore = \"...\"] saying what blocks it and what would unblock it." >&2
  echo "An ignore that does not say why is exactly what the rule forbids." >&2
  exit 1
fi

# ── The pinned set ───────────────────────────────────────────────────────────────────────────────
CURRENT="$(current_counts)"

if [[ "${UPDATE_BASELINE:-}" == "1" ]]; then
  echo "# Paste into BASELINE in $0 — and say in your PR what changed and why."
  printf '%s\n' "$CURRENT"
  exit 0
fi

EXPECTED="$(printf '%s\n' "$BASELINE" | grep -vE '^\s*(#|$)' | sort -k2)"

if ! diff_out="$(diff <(printf '%s\n' "$EXPECTED") <(printf '%s\n' "$CURRENT"))"; then
  echo "FAIL: the set of #[ignore]d tests changed." >&2
  echo >&2
  echo "  < expected (pinned)   > found (working tree)" >&2
  printf '%s\n' "$diff_out" >&2
  echo >&2
  echo "If you ADDED one: say in the PR what blocks the test and what would unblock it, then" >&2
  echo "rerun with UPDATE_BASELINE=1. An ignore is a declaration, not a place to put things." >&2
  echo >&2
  echo "If you REMOVED one (un-ignored it — thank you): rerun with UPDATE_BASELINE=1 and drop the" >&2
  echo "line. Say in the PR whether the test now passes, or whether it failed for a DIFFERENT" >&2
  echo "reason than the one it recorded — that second case is the common one." >&2
  exit 1
fi

count="$(printf '%s\n' "$CURRENT" | awk '{s+=$1} END {print s+0}')"
echo "OK: $count #[ignore]d tests, set unchanged, every one carrying a reason."
echo "    Reasons are NOT verified by this check — run --list and read them."
