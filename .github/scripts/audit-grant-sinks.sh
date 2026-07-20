#!/usr/bin/env bash
# audit-grant-sinks.sh — enumerate every production write-site to kb_access_grants and fail
# if the set has grown without review.
#
# WHY THIS EXISTS
# ---------------
# A grant is the one write that hands out capability. The 2026-07-18 security audit
# (docs/code-reviews/2026-07-18-authn-authz-credential-flow-audit.md) turned up F-0: a grant
# path that inserted the request's capability bits verbatim, with no `conferred ⊆ held`
# attenuation — a read+grant principal could self-escalate to write+delete+grant. The plan that
# introduced the grant chokepoint also *missed one of the callers* ("the fifth insert_grant
# caller") because the call sites were enumerated by hand.
#
# This script makes that enumeration mechanical. It does NOT prove attenuation — it pins the set
# of grant write-sites against a reviewed baseline, so a NEW sink cannot be added without a human
# acknowledging the two questions every grant write must answer:
#   1. AUTHORITY  — is the grantor authorized to administer this subject's access?
#   2. ATTENUATION — is the conferred capability a subset of what the grantor holds
#                    (`conferred ⊆ held`), and can `principal_id` be the caller itself?
# See docs/development/security-audit-playbook.md § "the one lesson that matters most".
#
# USAGE
#   .github/scripts/audit-grant-sinks.sh            # verify against the baseline (CI mode)
#   .github/scripts/audit-grant-sinks.sh --list     # just print the current sinks
#   UPDATE_BASELINE=1 .github/scripts/audit-grant-sinks.sh   # rewrite the baseline after review
#
# Exit 0 = set unchanged. Exit 1 = a sink was added/removed/moved-file — review required.

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

# Overridable so the test harness can point the SQL scan at a fixture directory.
MIGRATIONS_DIR="${MIGRATIONS_DIR:-migrations}"

# The reviewed baseline: <count> <path>, sorted by path. Each entry is a file that writes to
# kb_access_grants (via access_service::insert_grant or a raw INSERT). Some entries are test-
# module or scenario-loader seeds rather than live grant paths — that is fine; the tripwire's job
# is to freeze the SET, not to classify each line. When this list changes, a reviewer confirms the
# new/changed site answers the AUTHORITY + ATTENUATION questions above, then reruns with
# UPDATE_BASELINE=1.
#
# REVIEWED 2026-07-18 (PR #482, admin-event-sink Task 5 + 5b) — access_service.rs 2 → 1.
#   The vanished site is the raw `INSERT INTO kb_access_grants` that used to live INSIDE
#   `insert_grant`'s own body. Task 5 replaced it with a call to the SQL chokepoint
#   `_admin_grant_created` (migrations/20260718000010), which performs the upsert AND appends the
#   `grant_created` ledger event in one transaction. So this is a REDUCTION in Rust-side write
#   sites, not a new sink — the write moved down a layer, on purpose.
#   AUTHORITY: unchanged and tightened — every sink gates before writing, and both the human sink
#     (`grant_capability`) and the machine sink (`machine_authz::contain_reach`) now call ONE
#     decision, `access_service::authorize_capability_grant`.
#   ATTENUATION: newly ENFORCED, which is what F-0 asked for — a delegated administrator may confer
#     only capabilities it already holds (`conferred ⊆ held`), self-grant included, since the check
#     never consults who the principal is. System admins stay exempt so bootstrap/repair work.
#   db_backend.rs stays at 1: its raw INSERT became an `insert_grant(...)` call (5b.1), so the
#     cogmap creator-bootstrap grant is now ledgered rather than landing silently.
#
# ✅ BLIND SPOT CLOSED (2026-07-20). This used to read "KNOWN BLIND SPOT": the script scanned
#   `crates/**/src/*.rs` only, so after Task 5 moved the AUTHORITATIVE write into SQL
#   (`_admin_grant_created` / `_admin_grant_revoked`), the tripwire's baseline SHRANK and its green
#   tick covered LESS than before — the Rust count going 2 → 1 was read as "a reduction in sinks"
#   when the sink had simply stepped out of the guard's field of view. A guard whose coverage
#   quietly narrows is worse than one that never had it, because the number went the reassuring way.
#   `sql_current()` below now scans migrations/ as well. See task 019f7c98-fbea-7a62-b5f0-5f8e0556b196.
read -r -d '' BASELINE <<'EOF' || true
1 crates/temper-services/src/backend/db_backend.rs
1 crates/temper-services/src/services/access_service.rs
2 crates/temper-services/src/services/connection_service.rs
1 crates/temper-services/src/services/machine_authz.rs
1 crates/temper-services/src/services/machine_registration_service.rs
2 crates/temper-services/src/services/materialize_service.rs
1 crates/temper-services/src/services/steward_service.rs
1 crates/temper-substrate/src/scenario/access/loader.rs
EOF

# Reviewed SQL baseline: the function names (and top-level backfill files) whose bodies write
# kb_access_grants. REVIEWED 2026-07-20 — 2 functions + 2 immutable one-time backfills.
#   _admin_grant_created / _admin_grant_revoked (migrations/20260718000010_admin_grant_fns.sql)
#     — the live authoritative write path; upsert/delete plus the ledger event, in one transaction.
#   <top-level>:20260701000001_cogmap_write_tightening.sql — one-time BACKFILL-FIRST-then-FLIP DML.
#   <top-level>:20260701000003_access_grants_store_migration.sql — one-time backfill from
#     kb_resource_access.
# Both backfills are in shipped, immutable migrations: they ran once and cannot run again.
read -r -d '' SQL_BASELINE <<'EOF' || true
<top-level>:20260701000001_cogmap_write_tightening.sql
<top-level>:20260701000003_access_grants_store_migration.sql
_admin_grant_created
_admin_grant_revoked
EOF

# Current set: grant write-sites in production src trees, per file, sorted by path.
# Patterns: a call to insert_grant(...) OR a raw INSERT INTO kb_access_grants.
# Excluded: the insert_grant definition, `use` imports, and comment lines.
current() {
  # Portable grep (no ripgrep dependency — it is not guaranteed on CI runners). The `/src/`
  # path filter restores the "src trees only" scope; the trailing `|| true` keeps a legitimately
  # empty result from tripping `set -e` before the diff can report it.
  grep -rnE --include='*.rs' \
     -e 'insert_grant[[:space:]]*\(' \
     -e 'INSERT INTO kb_access_grants' \
     crates 2>/dev/null \
  | grep -E '^[^:]*/src/[^:]*\.rs:' \
  | grep -vE 'pub async fn insert_grant|use .*insert_grant|^[^:]*:[0-9]+:[[:space:]]*//' \
  | awk -F: '{print $1}' \
  | sort | uniq -c \
  | awk '{printf "%s %s\n", $1, $2}' \
  | sort -k2 \
  || true
}

# The SQL half. Keyed on the FUNCTION NAME whose body writes the table, never on file or line.
#
# That key choice is the whole design. Migrations are append-only and immutable: a shipped SQL
# function is changed by a NEW migration doing DROP+CREATE, so a per-file or per-line baseline
# would churn on every routine redefinition of `_admin_grant_created` and get UPDATE_BASELINE'd
# reflexively until it meant nothing. The set of function NAMES that write kb_access_grants is
# stable across redefinition and changes only when a genuinely new write path appears — which is
# exactly the event worth a human's attention.
#
# Writes outside any function (one-time backfill DML) are keyed `<top-level>:<basename>`. Keeping
# the basename means a NEW backfill trips the guard while the two existing immutable ones stay put.
#
# Body-terminator note: every function here is `$$`-quoted, so `$$;` resets scope. A custom dollar
# tag (`$fn$`) would need the reset pattern widened — no such tag exists in migrations/ today.
sql_current() {
  awk '
    /^[[:space:]]*CREATE([[:space:]]+OR[[:space:]]+REPLACE)?[[:space:]]+FUNCTION/ {
      line = $0
      sub(/.*[Ff][Uu][Nn][Cc][Tt][Ii][Oo][Nn][[:space:]]+/, "", line)
      sub(/[[:space:]]*\(.*/, "", line)
      fn = line
      next
    }
    /^[[:space:]]*\$\$;[[:space:]]*$/ { fn = "" }
    /(INSERT INTO|UPDATE|DELETE FROM)[[:space:]]+kb_access_grants/ {
      if ($0 ~ /^[[:space:]]*--/) next
      if (fn != "") { print fn }
      else { base = FILENAME; sub(/.*\//, "", base); print "<top-level>:" base }
    }
  ' "$MIGRATIONS_DIR"/*.sql 2>/dev/null | sort -u || true
}

CURRENT="$(current)"
SQL_CURRENT="$(sql_current)"

if [[ "${1:-}" == "--list" ]]; then
  echo "$CURRENT"
  echo "--- SQL:"
  echo "$SQL_CURRENT"
  exit 0
fi

if [[ "${UPDATE_BASELINE:-}" == "1" ]]; then
  echo "$CURRENT"
  echo "--- SQL:"
  echo "$SQL_CURRENT"
  echo "^^^ copy the two blocks above into BASELINE / SQL_BASELINE in this script (only after" >&2
  echo "    reviewing each changed site for AUTHORITY + ATTENUATION)." >&2
  exit 0
fi

fail=0

NORM_BASELINE="$(printf '%s\n' "$BASELINE" | sort -k2)"
if ! diff <(printf '%s\n' "$NORM_BASELINE") <(printf '%s\n' "$CURRENT") >/tmp/grant-sinks.diff 2>&1; then
  fail=1
fi

# A SQL scan that finds nothing must fail, not pass. The authoritative write lives in SQL now; an
# empty result means the scan stopped matching (renamed dir, changed quoting), and an empty set
# diffs clean against nothing while asserting nothing at all.
NORM_SQL_BASELINE="$(printf '%s\n' "$SQL_BASELINE" | sort -u)"
if [[ -z "$SQL_CURRENT" ]]; then
  echo "audit-grant-sinks: FAIL — the migrations scan found NO kb_access_grants writes." >&2
  echo "  The authoritative write is in SQL (_admin_grant_created/_admin_grant_revoked); finding" >&2
  echo "  zero means the scan broke, not that the sinks are gone. Check MIGRATIONS_DIR=$MIGRATIONS_DIR." >&2
  fail=1
elif ! diff <(printf '%s\n' "$NORM_SQL_BASELINE") <(printf '%s\n' "$SQL_CURRENT") >/tmp/grant-sinks-sql.diff 2>&1; then
  echo "audit-grant-sinks: FAIL — the set of SQL-side kb_access_grants write-sites changed." >&2
  echo "diff (SQL baseline -> current):" >&2
  cat /tmp/grant-sinks-sql.diff >&2
  echo >&2
  fail=1
fi

if [[ "$fail" == "0" ]]; then
  echo "audit-grant-sinks: OK — grant write-sites unchanged ($(printf '%s\n' "$CURRENT" | grep -c . ) Rust files, $(printf '%s\n' "$SQL_CURRENT" | grep -c . ) SQL sites)."
  exit 0
fi

cat >&2 <<'MSG'
audit-grant-sinks: FAIL — the set of kb_access_grants write-sites changed.

A grant write hands out capability. Before accepting this change, confirm each new/changed
site answers BOTH:
  1. AUTHORITY   — the grantor is authorized to administer this subject's access.
  2. ATTENUATION — conferred capability ⊆ what the grantor holds, and principal_id is not a
                   silent self-grant (a read+grant holder must not confer write/delete/grant).
See docs/development/security-audit-playbook.md.

diff (baseline → current):
MSG
cat /tmp/grant-sinks.diff >&2
echo >&2
echo "If the change is reviewed and correct: UPDATE_BASELINE=1 .github/scripts/audit-grant-sinks.sh" >&2
exit 1
