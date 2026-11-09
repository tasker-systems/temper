#!/usr/bin/env bash
# audit-raw-standing-writers.sh — pin the set of production code that calls the standing committer
# `principal_standing_apply` directly, instead of through `standing_service`.
#
# WHY THIS EXISTS
# ---------------
# Standing moves by "decide, then commit" (`standing_service.rs`): `temper_principal::transition`
# judges the act against the person's LOCKED current standing, and only a legal move reaches the SQL
# committer. The committer itself "does NOT decide legality" (`20260923000010`) — it writes whatever
# state it is handed. So every raw call is a writer that skips the transition table, and is exactly
# as safe as the reasoning of whoever wrote it.
#
# That was not hypothetical. Until 2026-11-01 an admin's decision on a join request
# (`access_service::review_request`) and a promotion to system admin (`promote_admin`) both called
# the committer raw, with a hard-coded target. A pending request can outlive the standing it was
# filed under, so rejecting a stale one moved Revoked → Denied (the laundering D15 forbids) and
# approving one lifted a deactivation that only `Reactivate` may lift. Both now go through
# `standing_service::judge_and_apply_conn`.
#
# This does NOT forbid a raw writer. A mint door that births standing inside the transaction that
# creates the profile, a scenario loader, the bootstrap template — each has a stated reason. It PINS
# the current set against a reviewed baseline, so a NEW raw writer fails CI until a reviewer
# answers: why can this not go through `standing_service`?
#
# WHAT IT SCANS
# -------------
#   * Rust under crates/*/src, minus `standing_service.rs` (the seam itself). Each file is read only
#     up to its first test-gated MODULE (`#[cfg(test)]` / `#[cfg(all(test …))]` directly above a
#     `mod`), so an in-file test module that seeds standing raw is not a production writer, while
#     an item-level `#[cfg(test)] use …` does not end the scan. Comment lines are dropped.
#   * SQL under migrations/templates/ — operator-run templates execute against a live database.
#     Migrations themselves are not scanned: the committer's own definition lives there, and a
#     shipped migration is history, not a caller anyone can add to.
# Integration tests (crates/*/tests) are outside the field of view on purpose.
#
# USAGE
#   .github/scripts/audit-raw-standing-writers.sh          # verify (CI mode)
#   .github/scripts/audit-raw-standing-writers.sh --list   # print current raw writers
#   UPDATE_BASELINE=1 .github/scripts/audit-raw-standing-writers.sh   # print a baseline after review

set -euo pipefail
cd "${REPO_ROOT_OVERRIDE:-$(git rev-parse --show-toplevel)}"

SEAM='crates/temper-services/src/services/standing_service.rs'

# Reviewed baseline: <count> <path>. Every entry is a reasoned exception:
#   machine_registration_service.rs  the two machine mint doors birth a machine's `denied` standing
#                                    inside the transaction that creates its profile (the
#                                    "machine-door pattern"); `standing_service::provision_conn`
#                                    is the seam's twin for that and these predate it.
#   scenario/access/loader.rs,       scenario loaders seed a declared world's standing as it is
#   scenario/loader.rs               written, not as it was reached; they do not run on a request.
#   scenario/bootseed.rs             genesis: the first admin's `approved`, guarded on first seed.
#   templates/system_initialization.sql  the operator's root bootstrap, the same genesis by hand.
read -r -d '' BASELINE <<'EOF' || true
2 crates/temper-services/src/services/machine_registration_service.rs
1 crates/temper-substrate/src/scenario/access/loader.rs
1 crates/temper-substrate/src/scenario/bootseed.rs
1 crates/temper-substrate/src/scenario/loader.rs
1 migrations/templates/system_initialization.sql
EOF

PATTERN='principal_standing_apply[[:space:]]*\('

current() {
  {
    # Rust: production region only (everything before the first test-gated line), comments dropped.
    find crates -path '*/src/*' -name '*.rs' -not -path "$SEAM" -print 2>/dev/null \
    | sort \
    | while IFS= read -r f; do
        # The pattern rides ENVIRON, not `-v`: `-v` escape-processes its value, so gawk turns
        # `\(` into a bare `(` (an invalid regex) where mawk keeps it.
        SCAN_PAT="$PATTERN" awk -v f="$f" '
          # A test gate cuts the scan only when the item it gates is a MODULE: an item-level
          # `#[cfg(test)] use …` near the top of a production file must not hide the rest of it.
          pending && /^[[:space:]]*$/ { next }
          pending && /^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?mod[[:space:]]/ { exit }
          { pending = 0 }
          /^[[:space:]]*#\[cfg\((all\()?test/ { pending = 1; next }
          /^[[:space:]]*\/\// { next }
          $0 ~ ENVIRON["SCAN_PAT"] { print f }
        ' "$f" || echo "SCAN-ERROR $f"
      done
    # Operator-run SQL templates. `--` comment lines dropped.
    if [ -d migrations/templates ]; then
      grep -rnE --include='*.sql' -e "$PATTERN" migrations/templates 2>/dev/null \
      | grep -v -E '^[^:]*:[0-9]+:[[:space:]]*--' \
      | awk -F: '{print $1}' || true
    fi
  } \
  | sort | uniq -c \
  | awk '{c = $1; $1 = ""; sub(/^ /, ""); printf "%s %s\n", c, $0}' \
  | sort -k2 \
  || true
}

CURRENT="$(current)"

# An awk that dies on a file must FAIL the scan, never shrink it: a silently skipped file is a
# writer this guard stopped seeing.
if grep -q 'SCAN-ERROR' <<< "$CURRENT"; then
  echo "audit-raw-standing-writers: FAIL — the scan errored on:" >&2
  grep 'SCAN-ERROR' <<< "$CURRENT" >&2
  exit 1
fi

if [[ "${1:-}" == "--list" ]]; then
  echo "$CURRENT"
  exit 0
fi
if [[ "${UPDATE_BASELINE:-}" == "1" ]]; then
  echo "$CURRENT"
  echo "^^^ copy into BASELINE after confirming each new raw writer cannot go through standing_service." >&2
  exit 0
fi

# A scan that finds NOTHING fails: the committer has callers today, so an empty result means the
# scan broke (a moved tree, a renamed function), not that the set emptied.
if [[ -z "$CURRENT" ]]; then
  echo "audit-raw-standing-writers: FAIL — the scan found no callers at all; the scan is broken." >&2
  exit 1
fi

NORM_BASELINE="$(printf '%s\n' "$BASELINE" | sort -k2)"
DIFF_FILE="$(mktemp)"
trap 'rm -f "$DIFF_FILE"' EXIT
if diff <(printf '%s\n' "$NORM_BASELINE") <(printf '%s\n' "$CURRENT") >"$DIFF_FILE" 2>&1; then
  echo "audit-raw-standing-writers: OK — raw principal_standing_apply callers unchanged."
  exit 0
fi

cat >&2 <<'MSG'
audit-raw-standing-writers: FAIL — the set of raw principal_standing_apply callers changed.

The committer does not decide legality; `standing_service` does (transition table, locked current
standing). A raw call skips that judgement. Before accepting a new one, confirm it cannot go
through `standing_service::apply` / `apply_conn` / `judge_and_apply_conn` / `provision_conn`.

diff (baseline -> current):
MSG
cat "$DIFF_FILE" >&2
echo >&2
echo "If reviewed and correct: UPDATE_BASELINE=1 .github/scripts/audit-raw-standing-writers.sh" >&2
exit 1
