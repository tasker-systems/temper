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

# The reviewed baseline: <count> <path>, sorted by path. Each entry is a file that writes to
# kb_access_grants (via access_service::insert_grant or a raw INSERT). Some entries are test-
# module or scenario-loader seeds rather than live grant paths — that is fine; the tripwire's job
# is to freeze the SET, not to classify each line. When this list changes, a reviewer confirms the
# new/changed site answers the AUTHORITY + ATTENUATION questions above, then reruns with
# UPDATE_BASELINE=1.
read -r -d '' BASELINE <<'EOF' || true
1 crates/temper-services/src/backend/db_backend.rs
2 crates/temper-services/src/services/access_service.rs
2 crates/temper-services/src/services/connection_service.rs
1 crates/temper-services/src/services/machine_authz.rs
1 crates/temper-services/src/services/machine_registration_service.rs
2 crates/temper-services/src/services/materialize_service.rs
1 crates/temper-services/src/services/steward_service.rs
1 crates/temper-substrate/src/scenario/access/loader.rs
EOF

# Current set: grant write-sites in production src trees, per file, sorted by path.
# Patterns: a call to insert_grant(...) OR a raw INSERT INTO kb_access_grants.
# Excluded: the insert_grant definition, `use` imports, and comment lines.
current() {
  rg -n --glob 'crates/**/src/**/*.rs' \
     -e 'insert_grant\s*\(' \
     -e 'INSERT INTO kb_access_grants' \
     2>/dev/null \
  | grep -v -E 'pub async fn insert_grant|use .*insert_grant|^[^:]*:[0-9]+:\s*//' \
  | awk -F: '{print $1}' \
  | sort | uniq -c \
  | awk '{printf "%s %s\n", $1, $2}' \
  | sort -k2
}

CURRENT="$(current)"

if [[ "${1:-}" == "--list" ]]; then
  echo "$CURRENT"
  exit 0
fi

if [[ "${UPDATE_BASELINE:-}" == "1" ]]; then
  echo "$CURRENT"
  echo "^^^ copy the block above into BASELINE in this script (only after reviewing each" >&2
  echo "    changed site for AUTHORITY + ATTENUATION)." >&2
  exit 0
fi

NORM_BASELINE="$(printf '%s\n' "$BASELINE" | sort -k2)"

if diff <(printf '%s\n' "$NORM_BASELINE") <(printf '%s\n' "$CURRENT") >/tmp/grant-sinks.diff 2>&1; then
  echo "audit-grant-sinks: OK — grant write-sites unchanged ($(printf '%s\n' "$CURRENT" | grep -c . ) files)."
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
