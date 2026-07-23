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
# This does NOT forbid handler-side authz outright (the cognitive_maps route legitimately composes a
# gate in the handler today). It PINS the current set against a reviewed baseline, so a NEW
# handler-side authz call fails CI until a reviewer answers: should this predicate move into the
# service? See docs/development/security-audit-playbook.md § 2 and the F-3 finding.
#
# USAGE
#   .github/scripts/audit-handler-authz-drift.sh          # verify (CI mode)
#   .github/scripts/audit-handler-authz-drift.sh --list   # print current handler-side authz calls
#   UPDATE_BASELINE=1 .github/scripts/audit-handler-authz-drift.sh   # rewrite baseline after review

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

# Authorization predicates that belong in the service layer. A call to one of these from inside a
# surface (handlers/ or an mcp tool) is drift worth a second look.
PREDICATES='is_system_admin|has_system_access|can_administer_grant|grant_authority|require_cogmap_write_admin|machine_authz::authorize|attenuates_to_caller|profile_can_grant'

# Reviewed baseline: <count> <path> for each surface file that STILL gates in a handler. The
# admin-authz enclosure (2026-07-22) moved access.rs's five admin gates and embed.rs's reembed gate
# INTO the service as `&SystemAdmin` params, so F-3 is resolved for them and they leave this baseline.
# What remains is the one cognitive_maps gate: a COMPOSITIONAL (Bucket-2) check where `is_system_admin`
# is one branch of a disjunction (admin OR gating-team OR scoped grant), not the gate — deliberately
# NOT enclosed (forcing it behind the proof would deny the scoped actors it exists to admit; see the
# enclosure spec's audit inventory). A NEW handler-side predicate still reds CI here.
read -r -d '' BASELINE <<'EOF' || true
1 crates/temper-api/src/handlers/cognitive_maps.rs
EOF

current() {
  # Portable grep (no ripgrep dependency — not guaranteed on CI runners) over the two surface
  # trees. Trailing `|| true` keeps an empty result from tripping `set -e` before the diff reports.
  grep -rnE --include='*.rs' -e "$PREDICATES" \
     crates/temper-api/src/handlers crates/temper-mcp/src 2>/dev/null \
  | grep -v -E '^[^:]*:[0-9]+:[[:space:]]*//' \
  | awk -F: '{print $1}' \
  | sort | uniq -c \
  | awk '{printf "%s %s\n", $1, $2}' \
  | sort -k2 \
  || true
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
if diff <(printf '%s\n' "$NORM_BASELINE") <(printf '%s\n' "$CURRENT") >/tmp/handler-authz.diff 2>&1; then
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
cat /tmp/handler-authz.diff >&2
echo >&2
echo "If reviewed and correct: UPDATE_BASELINE=1 .github/scripts/audit-handler-authz-drift.sh" >&2
exit 1
