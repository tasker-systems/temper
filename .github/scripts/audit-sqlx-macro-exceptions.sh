#!/usr/bin/env bash
# audit-sqlx-macro-exceptions.sh — pin the set of production SQL queries that are NOT
# compile-time-checked, each against a declared reason.
#
# WHY THIS EXISTS
# ---------------
# A production outage on 2026-07-30 came from a migration applied ahead of the binary that speaks
# it. Every check was green, because a merge is not a deploy and nothing compared the schema against
# the running code. The safeguard being built for that reads the `.sqlx` cache — the compiler's own
# record of the binary↔schema wire contract.
#
# That record is only faithful for `query!`-family macros. A runtime `sqlx::query(...)` leaves NO
# cache entry, so it is invisible to the detector — and a mechanism that is blind exactly where it
# matters is worse than one that is absent, because it reads as coverage.
#
# The rule "use the macros" already existed as prose in
# docs/development/code-quality-best-practices.md and was unenforced, which is the same defect class
# as the outage itself. When it was finally measured, 66 of 112 production runtime call sites had no
# technical obstacle at all — the exemption was habit, and in four places a COMMENT ASSERTING a
# reason that did not hold. One of those false claims had propagated by citation into another file.
# All 66 were converted; these 46 are what remains.
#
# This script does not prove those 46 are justified. It pins the SET, so a 47th cannot appear
# without a human writing down which of the four reasons it claims — and, if none fits, converting
# it instead. A reason turns an exception into a declaration; a bare exemption list decays into a
# place to put things.
#
# USAGE
#   .github/scripts/audit-sqlx-macro-exceptions.sh          # verify against the baseline (CI mode)
#   .github/scripts/audit-sqlx-macro-exceptions.sh --list    # print the current per-file counts
#   UPDATE_BASELINE=1 .github/scripts/audit-sqlx-macro-exceptions.sh   # print a pasteable baseline
#
# Exit 0 = set unchanged. Exit 1 = a runtime query was added, removed, or moved file — review.
#
# TEST SEAMS (used by test-audit-sqlx-macro-exceptions.sh, never in CI):
#   SQLX_SCAN_ROOT       — scan a fixture tree instead of the repo
#   SQLX_ALLOWLIST_FILE  — read the baseline from a file instead of the block below

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

ENUMERATOR="scripts/classify-sqlx-calls.py"

# WHY SHELL OUT RATHER THAN RE-GREP HERE
# --------------------------------------
# The enumerator already solves the hard parts, each learned by getting it wrong: brace-matched
# `#[cfg(test)] mod` spans (a call in a test module is not a production call), the required `sqlx::`
# path (a path-less `.query(` matches reqwest's RequestBuilder — temper-client has no sqlx
# dependency and still reported 7 "sqlx calls"), and a depth-counted turbofish (`[^>]*` cannot cross
# `Option<String>`, which hid 10 sites for a year). Re-implementing that in awk would be a second
# spelling of one predicate, and the two would drift — with the drift landing on whether a query is
# exempt from a safety check.
#
# It also means the check's coverage IS the enumerator's coverage. That is the point, and it is why
# the enumerator now carries its own guard test rather than being trusted.

# ── the allow-list ────────────────────────────────────────────────────────────────────────────
#
# FORMAT: <reason> <count> <path>. A file may appear on more than one line when its exemptions
# differ (embed.rs holds two of each). The checker sums a path's declared counts and compares that
# total to what the enumerator finds in that file.
#
# WHAT IS DELIBERATELY NOT HERE: line numbers. They churn on every edit above them, and a baseline
# that churns for cosmetic reasons gets UPDATE_BASELINE'd on reflex until nobody reads its diffs —
# the failure mode where a guard is still green, still running, and no longer load-bearing.
# `audit-grant-sinks.sh` records the same lesson twice, in both directions.
#
# THE FOUR REASONS
#
#   dynamic-table     The table name is a loop variable. No macro can express it, since a parameter
#                     cannot be a table name. NOTE the near-miss: "a parameter cannot be a table
#                     name" is true and does NOT imply "therefore runtime" when the set of tables is
#                     closed and small — `region_clocks.rs` picked between two static literals and
#                     read as a legitimate `dynamic-table` exemption for exactly that reason. It is
#                     now a `query_scalar!` per match arm. Claim this only when the table set is
#                     genuinely open.
#
#   vector-cast       A `$n::vector` bind. pgvector's type is not one the macros can infer, so the
#                     cast forces runtime. This is the ORIGINAL exception and the one the other 16
#                     sites in readback/mod.rs used to follow "for consistency" — it covered 3 of
#                     19 there. Claim it only when the statement actually casts.
#
#   dynamic-sql       The statement TEXT is assembled before the call (`sqlx::query(&sql)`). Not to
#                     be confused with a static statement carrying NULL-passthrough parameters
#                     (`$2::uuid IS NULL OR col = $2`), which is a normal macro query — that one
#                     shipped a comment claiming it was "the dynamic-predicate case" while its own
#                     next sentence conceded nothing was interpolated.
#
#   dynamic-order-by  Only the ORDER BY clause varies, interpolated from a CLOSED set of column
#                     names validated at the call site. Kept separate from `dynamic-sql` rather than
#                     folded into it — a decision, so here is the reason: the two carry different
#                     risk. This one is a bounded, auditable pattern over a fixed column list;
#                     `dynamic-sql` is arbitrary statement assembly. One name for both would let a
#                     future arbitrary-SQL site justify itself by pointing at the ORDER BY
#                     precedent, which is precisely how the "for consistency" exemptions spread.
#
# REVIEWED 2026-07-30 (Arc A, task 019fb4e4-a331-7e01-b35a-78523ef663ec) — seeded at 46.
#   Seeded from the CLASSIFIED set, never by transcribing the state of the tree. Transcribing would
#   have blessed the 66 sites with no reason — a baseline that blesses the thing it was built to
#   prevent. Reproduce the classification with `python3 scripts/classify-sqlx-calls.py`; the
#   reasoning is docs/development/sqlx-macro-exception-classification.md.
read -r -d '' BASELINE <<'EOF' || true
dynamic-order-by 1 crates/temper-services/src/backend/substrate_read.rs
dynamic-sql 2 crates/temper-substrate/src/embed.rs
dynamic-table 36 crates/temper-substrate/src/replay.rs
vector-cast 2 crates/temper-substrate/src/embed.rs
vector-cast 2 crates/temper-substrate/src/write.rs
vector-cast 4 crates/temper-substrate/src/readback/mod.rs
EOF

if [[ -n "${SQLX_ALLOWLIST_FILE:-}" ]]; then
  BASELINE="$(cat "$SQLX_ALLOWLIST_FILE")"
fi

if ! command -v python3 >/dev/null 2>&1; then
  echo "audit-sqlx-macro-exceptions: python3 not found — it is required to run ${ENUMERATOR}." >&2
  exit 2
fi

if [[ ! -f "$ENUMERATOR" ]]; then
  echo "audit-sqlx-macro-exceptions: ${ENUMERATOR} is missing — the check cannot run." >&2
  exit 2
fi

# Declared totals per path, summed across that path's reason lines. `<count> <path>`, sorted.
declared() {
  printf '%s\n' "$BASELINE" \
    | grep -vE '^[[:space:]]*(#|$)' \
    | awk '{ total[$3] += $2 } END { for (p in total) printf "%s %s\n", total[p], p }' \
    | sort -k2
}

# Actual per-path counts of non-macro production call sites, from the enumerator.
current() {
  python3 "$ENUMERATOR" \
    | sed -n '/^NON-MACRO by file:$/,$p' | tail -n +2 \
    | awk 'NF { printf "%s %s\n", $1, $2 }' \
    | sort -k2
}

DECLARED="$(declared)"
CURRENT="$(current)"

if [[ "${1:-}" == "--list" ]]; then
  echo "$CURRENT"
  exit 0
fi

if [[ "${UPDATE_BASELINE:-}" == "1" ]]; then
  echo "$CURRENT"
  echo >&2
  echo "^^^ current per-file counts. DO NOT paste these in as-is: the baseline is <reason> <count>" >&2
  echo "    <path>, so each changed file needs a REASON chosen from the four documented in this" >&2
  echo "    script — and if none of them fits, the site should become a macro instead. That" >&2
  echo "    judgement is the whole value of this check; a count alone records nothing." >&2
  exit 0
fi

if ! diff -u <(printf '%s\n' "$DECLARED") <(printf '%s\n' "$CURRENT") > /tmp/sqlx-exceptions.diff 2>&1; then
  echo "❌ The set of non-macro production SQL queries has changed." >&2
  echo >&2
  sed -n '3,$p' /tmp/sqlx-exceptions.diff >&2
  echo >&2
  echo "  -  = declared in this script's allow-list" >&2
  echo "  +  = found in the tree right now" >&2
  echo >&2
  echo "A runtime sqlx::query leaves NO .sqlx cache entry, so it is invisible to the schema/binary" >&2
  echo "change detector. If you ADDED one, either:" >&2
  echo "  (a) convert it to query!/query_as!/query_scalar! — the default, and what 66 sites did; or" >&2
  echo "  (b) add it to BASELINE above with one of the four documented reasons, and say in your PR" >&2
  echo "      which reason and why. If no reason fits, that IS the answer: it converts." >&2
  echo >&2
  echo "If you REMOVED one (converted it — thank you), rerun with UPDATE_BASELINE=1 and drop the" >&2
  echo "line, keeping the remaining reasons intact." >&2
  exit 1
fi

echo "✅ sqlx macro exceptions unchanged: $(printf '%s\n' "$CURRENT" | awk '{s+=$1} END {print s+0}') non-macro production call sites, all declared."
