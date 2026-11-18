#!/usr/bin/env bash
# audit-inline-db-tests.sh — no DB test module sits inline in a temper-services production file.
#
# WHY THIS EXISTS
# ---------------
# Ruling R24 (codebase recon rulings, 2026-11-08): inline DB tests keep their module's scope but
# leave the production file. A module's DB tests live in a sibling file the module declares,
#
#     #[cfg(all(test, feature = "test-db"))]
#     mod tests;
#
# so `foo.rs` declares `foo/tests.rs` (or `mod.rs` declares a sibling `tests.rs`). On 2026-11-18,
# 38 files under crates/temper-services/src carried the inline form, some of them half test code
# by line count. They are moved in batches, and this script is the ratchet that holds each batch:
# a file that has been moved can never take an inline module back, and no new file can start one.
#
# WHAT IT CHECKS
# --------------
# Two arms, over every .rs file under crates/temper-services/src:
#
#   1. INLINE MODULE — a `cfg(all(test, feature = "test-db"))` attribute whose item is `mod NAME {`.
#      The sibling form (`mod NAME;`) passes. Attributes stacked between the cfg and the `mod`
#      (`#[allow(..)]`, a doc comment) are skipped over.
#   2. STRAY DB TEST — a `#[sqlx::test` attribute in a file that is not a test-module file. A
#      test-module file is named `tests.rs` or `*_tests.rs`. This arm catches a DB test that
#      reached a production file under a different gate (`#[cfg(test)] mod tests {`, say), which
#      arm 1 alone would miss.
#
# A file in the BASELINE below is an incumbent still waiting for its batch: both arms skip it. The
# baseline only shrinks. A baseline file that no longer trips either arm fails the run until its line
# is deleted, so a moved file cannot silently stay exempt.
#
# WHAT IT DOES NOT CHECK
# ----------------------
# That fixtures come from `test_support`, or that public-API-only tests moved to `tests/`: those
# halves of R24 are review's. It reads text, not the module tree: a test-module file that no parent
# declares compiles to nothing and passes here (the test count recorded per batch is what catches a
# test that stopped running). Only crates/temper-services/src is scanned.
#
# USAGE
#   .github/scripts/audit-inline-db-tests.sh          # verify (CI and `cargo make tripwires`)
#   UPDATE_BASELINE=1 .github/scripts/audit-inline-db-tests.sh   # print the current offender set
#
# Exit 0 = no offender outside the baseline, and every baseline file still offends.
# Exit 1 = otherwise.
#
# TEST SEAMS (used by test-audit-inline-db-tests.sh, never in CI):
#   INLINE_DB_SCAN_ROOT     — scan this directory instead of crates/temper-services/src
#   INLINE_DB_BASELINE_FILE — read the baseline from a file instead of the block below

set -euo pipefail

REPO="$(git rev-parse --show-toplevel)"
SCAN_ROOT="${INLINE_DB_SCAN_ROOT:-$REPO/crates/temper-services/src}"
if [[ ! -d "$SCAN_ROOT" ]]; then
  echo "audit-inline-db-tests: no directory $SCAN_ROOT" >&2
  exit 1
fi

# Paths relative to crates/temper-services/src. Delete a line in the PR that moves its file.
read -r -d '' BASELINE <<'EOF' || true
backend/db_backend.rs
services/auditor_service.rs
services/materialize_service.rs
services/reassign_service.rs
services/standing_service.rs
services/steward_service.rs
services/workflow_job_service.rs
EOF
if [[ -n "${INLINE_DB_BASELINE_FILE:-}" ]]; then
  BASELINE="$(cat "$INLINE_DB_BASELINE_FILE")"
fi

# Arm 1: print "<line>" for each cfg(all(test, feature = "test-db")) whose item is an inline mod.
inline_modules() {
  awk '
    function is_cfg(s) {
      return s ~ /^[[:space:]]*#\[cfg\(all\((test,[[:space:]]*feature[[:space:]]*=[[:space:]]*"test-db"|feature[[:space:]]*=[[:space:]]*"test-db",[[:space:]]*test)\)\)\]/
    }
    pending && /^[[:space:]]*(#\[|\/\/)/ { next }
    pending {
      if ($0 ~ /^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?mod[[:space:]]+[A-Za-z_][A-Za-z0-9_]*[[:space:]]*\{/) print at
      pending = 0
    }
    is_cfg($0) { pending = 1; at = NR }
  ' "$1"
}

# Arm 2: print "<line>" for each #[sqlx::test attribute (not in a comment) in a non-test-module file.
stray_db_tests() {
  case "$(basename "$1")" in
    tests.rs | *_tests.rs) return 0 ;;
  esac
  grep -n '^[[:space:]]*#\[sqlx::test' "$1" | cut -d: -f1 || true
}

offenders() {
  (cd "$SCAN_ROOT" && find . -name '*.rs' -type f | sed 's|^\./||' | sort) | while read -r rel; do
    local f="$SCAN_ROOT/$rel"
    local hits
    hits="$(inline_modules "$f"; stray_db_tests "$f")"
    if [[ -n "$hits" ]]; then echo "$rel"; fi
  done
}

current="$(offenders)"

if [[ -n "${UPDATE_BASELINE:-}" ]]; then
  echo "$current"
  exit 0
fi

fail=0
in_baseline() { grep -qxF -- "$1" <<<"$BASELINE"; }

while read -r rel; do
  [[ -z "$rel" ]] && continue
  if ! in_baseline "$rel"; then
    echo "audit-inline-db-tests: $rel holds a DB test module inline (or a #[sqlx::test] outside a test-module file)." >&2
    echo "  Move it to a sibling file: #[cfg(all(test, feature = \"test-db\"))] mod tests; (R24)." >&2
    fail=1
  fi
done <<<"$current"

while read -r rel; do
  [[ -z "$rel" ]] && continue
  if ! grep -qxF -- "$rel" <<<"$current"; then
    echo "audit-inline-db-tests: $rel no longer holds an inline DB test module; delete its baseline line." >&2
    fail=1
  fi
done <<<"$BASELINE"

if [[ $fail -eq 0 ]]; then
  n=$(grep -c . <<<"$BASELINE" || true)
  echo "audit-inline-db-tests: no new inline DB test module; $n incumbent file(s) still to move"
fi
exit $fail
