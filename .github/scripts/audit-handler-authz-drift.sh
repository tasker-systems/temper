#!/usr/bin/env bash
# audit-handler-authz-drift.sh — flag authorization predicates invoked from a SURFACE
# (temper-api handlers / temper-mcp tools) rather than from a shared service.
#
# WHY THIS EXISTS
# ---------------
# Authorization must live in the shared temper-services layer, reachable identically from both
# surfaces — because temper-mcp and temper-api share that layer, an authz check that sits in ONE
# surface's handler is a hole for the other surface, and a check that sits in a handler at all is a
# check the service itself does not enforce. The 2026-07-18 audit found exactly this (finding F-3):
# `promote_admin`'s `is_system_admin` gate lived in the handler (access.rs) while the service
# `access_service::promote_admin` performed none — so a future second caller of the service would
# grant admin with no check. The admin-authz enclosure (2026-07-22, the sealed `SystemAdmin` proof)
# resolved that whole class: every pure-admin service fn now REQUIRES a `&SystemAdmin` param, so the
# check IS the signature and both surfaces inherit it — those handler gates are gone (see below).
#
# This does NOT forbid handler-side authz outright. It PINS the current set against a reviewed
# baseline, so a NEW handler-side authz call fails CI until a reviewer answers: should this
# predicate move into the service? See internal/development/security-audit-playbook.md § 2 and the
# F-3 finding.
#
# WHAT IT SCANS, AND WHY IT CAN FAIL
# ----------------------------------
# The needles are the PUBLIC authorization predicates of `temper_services::services::
# access_service`, the only ones a surface crate can name. The scoped predicates
# (`can_administer_grant`, `profile_can_grant`, `machine_authz::authorize`, `authz::authorize`) are
# `pub(crate)`: no surface can call them, so listing them would watch nothing.
#
# The field of view is `crates/temper-api/src/handlers` and `crates/temper-mcp/src`. The MCP tool
# layer has no temper-services dependency (its source gate keeps it absent) and holds no pool, so
# today only the handlers tree can reach the needles; the MCP tree is scanned so that a change to
# that boundary shows up here too.
#
# A guard that reads an empty tree as "no drift" is green forever, so the scan FAILS, before any
# mode runs, when:
#   * a scan root (or the definition root) does not exist — a moved or renamed tree; or
#   * a listed predicate no longer resolves to a `pub fn` definition under the definition root —
#     a renamed, deleted or narrowed predicate is a needle that can never match again.
#
# USAGE
#   .github/scripts/audit-handler-authz-drift.sh          # verify (CI mode)
#   .github/scripts/audit-handler-authz-drift.sh --list   # print current handler-side authz calls
#   UPDATE_BASELINE=1 .github/scripts/audit-handler-authz-drift.sh   # rewrite baseline after review
#   REPO_ROOT_OVERRIDE=<dir> .github/scripts/audit-handler-authz-drift.sh   # scan a fixture tree
#
# Harness: .github/scripts/test-audit-handler-authz-drift.sh

set -euo pipefail
cd "${REPO_ROOT_OVERRIDE:-$(git rev-parse --show-toplevel)}"

# Authorization predicates that belong in the service layer. A call to one of these from inside a
# surface is drift worth a second look. Each must resolve to a `pub fn` under DEF_ROOT.
PREDICATE_NAMES=(is_system_admin has_system_access)
DEF_ROOT='crates/temper-services/src'
SCAN_ROOTS=(crates/temper-api/src/handlers crates/temper-mcp/src)

PREDICATES="$(IFS='|'; echo "${PREDICATE_NAMES[*]}")"

for root in "${SCAN_ROOTS[@]}" "$DEF_ROOT"; do
  if [[ ! -d "$root" ]]; then
    echo "audit-handler-authz-drift: FAIL — scan root '$root' does not exist; the scan is broken." >&2
    exit 1
  fi
done

for name in "${PREDICATE_NAMES[@]}"; do
  # `pub fn` / `pub async fn` only: a `pub(crate)` definition is one no surface crate can call.
  if ! grep -rqE --include='*.rs' \
       "^[[:space:]]*pub[[:space:]]+(async[[:space:]]+)?fn[[:space:]]+${name}[[:space:]]*[(<]" \
       "$DEF_ROOT"; then
    echo "audit-handler-authz-drift: FAIL — predicate '$name' has no pub fn definition under" \
         "$DEF_ROOT; it was renamed, removed or narrowed, so it can no longer be watched." \
         "Update PREDICATE_NAMES." >&2
    exit 1
  fi
done

# Reviewed baseline: <count> <path> for each surface file that STILL gates in a handler. It is
# empty: every pure-admin service fn takes a `&SystemAdmin` proof, so the check is the signature,
# and reconcile's regime decision (system admin for the L0 kernel and gating-team maps, authorship
# for every other map) lives in `DbBackend::reconcile_cognitive_map`, so the shared write path
# enforces it for every caller. A NEW handler-side predicate reds CI here.
read -r -d '' BASELINE <<'EOF' || true
EOF

current() {
  # Portable grep (no ripgrep dependency — not guaranteed on CI runners). grep exits 1 on no match
  # and 2 on an error; only the error fails the scan — an unreadable file is a call this guard
  # stopped seeing.
  local hits rc=0
  hits="$(grep -rnE --include='*.rs' -e "$PREDICATES" "${SCAN_ROOTS[@]}")" || rc=$?
  if (( rc > 1 )); then
    echo "audit-handler-authz-drift: FAIL — grep errored (exit $rc) over the scan roots." >&2
    return 1
  fi
  [[ -z "$hits" ]] && return 0
  printf '%s\n' "$hits" \
  | { grep -v -E '^[^:]*:[0-9]+:[[:space:]]*//' || true; } \
  | awk -F: 'NF {print $1}' \
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
  echo "^^^ copy into BASELINE after confirming each new handler-side authz call should not move into a service." >&2
  exit 0
fi

NORM_BASELINE="$(printf '%s\n' "$BASELINE" | sort -k2)"
DIFF_FILE="$(mktemp)"
trap 'rm -f "$DIFF_FILE"' EXIT
if diff <(printf '%s\n' "$NORM_BASELINE") <(printf '%s\n' "$CURRENT") >"$DIFF_FILE" 2>&1; then
  echo "audit-handler-authz-drift: OK — handler-side authz calls unchanged."
  exit 0
fi

cat >&2 <<'MSG'
audit-handler-authz-drift: FAIL — handler-side authorization calls changed.

Authorization belongs in the shared temper-services layer (both surfaces enforce it there).
A predicate called from a handler/mcp tool is a check the SERVICE does not enforce — the F-3
drift shape. Before accepting, confirm the check should not move into the service.

diff (baseline -> current):
MSG
cat "$DIFF_FILE" >&2
echo >&2
echo "If reviewed and correct: UPDATE_BASELINE=1 .github/scripts/audit-handler-authz-drift.sh" >&2
exit 1
