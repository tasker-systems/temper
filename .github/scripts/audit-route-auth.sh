#!/usr/bin/env bash
# audit-route-auth.sh — pin the auth posture of every temper-api route.
#
# WHY THIS EXISTS
# ---------------
# temper-api's routes live in sub-router functions in crates/temper-api/src/routes.rs, and each
# function's auth posture is set ONCE by the `.layer(...)` applied to it in `create_app`:
#
#   GROUP                         POSTURE (set in create_app)
#   ----------------------------  --------------------------------------------------------------
#   auth_only_routes              require_auth                          (JWT — authenticated)
#   gated_routes                  require_auth + require_system_access  (JWT + system access)
#   public_routes                 (none)                                by-design public: /health
#   embed_internal_routes         (none)                                self-gated: EMBED_DISPATCH_SECRET
#   internal_routes               require_internal_signature            HMAC (INTERNAL_RECONCILE_SECRET)
#   slack_link_internal_routes    require_slack_link_signature          HMAC (SLACK_LINK_SECRET)
#   slack_link_public_routes      (none)                                by-design public: PKCE+state callback
#
# A route added to auth_only/gated is authenticated by construction — safe, no review needed.
# A route added to any of the OTHER groups is unauthenticated-at-the-middleware (public, a
# self-checked secret, or a signature the handler trusts) and MUST be reviewed: does it really
# self-gate / carry its own compensating control? This script freezes the set of routes in those
# review-required groups, and asserts the layer wiring is still present, so:
#   - a new unauthenticated/self-gated/signature route FAILS until acknowledged, and
#   - a silently deleted auth layer FAILS immediately.
# Auth-covered routes (auth_only/gated) grow freely and never trip this.
# See docs/development/security-audit-playbook.md § 1.
#
# USAGE
#   .github/scripts/audit-route-auth.sh          # verify (CI mode)
#   .github/scripts/audit-route-auth.sh --list   # print current review-required routes
#   UPDATE_BASELINE=1 .github/scripts/audit-route-auth.sh   # rewrite baseline after review

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

ROUTES_FILE="crates/temper-api/src/routes.rs"

# Groups whose routes are authenticated by construction (a require_auth layer). They grow freely.
AUTH_COVERED='auth_only_routes|gated_routes'
# Groups whose routes are NOT behind require_auth — every entry is a reviewed compensating control.
REVIEW_GROUPS='public_routes|embed_internal_routes|internal_routes|slack_link_internal_routes|slack_link_public_routes'

# Reviewed baseline: <group>\t<handler> for every route in a REVIEW group. Each is unauthenticated
# at the middleware and carries its own control (see the table above). A change here means a new or
# removed unauthenticated/self-gated/signature route — confirm the control, then UPDATE_BASELINE=1.
read -r -d '' BASELINE <<'EOF' || true
embed_internal_routes	handlers::embed::dispatch
embed_internal_routes	handlers::embed::warm
internal_routes	handlers::internal_saml::reconcile
public_routes	handlers::health::health_check
slack_link_internal_routes	handlers::slack_link::slack_link_state
slack_link_public_routes	handlers::slack_link::callback
EOF

# Every (sub-router group, handler) pair declared in routes.rs, keyed on the handler ident (stable
# across single-line and multi-line `.route(` / `routes!(` forms).
extract() {
  awk '
    function grpname(s,   r){ if (match(s,/fn [a-z_]+_routes\(/)){ r=substr(s,RSTART+3); sub(/\(.*/,"",r); return r } return "" }
    { g=grpname($0); if(g!=""){grp=g; next} }
    /^(pub )?fn (create_app|create_internal_app|openapi_spec|apply_transport_layers|cors_layer|fallback_handler)/ { grp="_x_"; next }
    grp=="_x_" || grp=="" { next }
    { s=$0; while (match(s,/handlers::[a-z_]+::[a-z_]+/)) { print grp"\t"substr(s,RSTART,RLENGTH); s=substr(s,RSTART+RLENGTH) } }
  ' "$ROUTES_FILE" | sort -u
}

ALL="$(extract)"
REVIEW_CURRENT="$(printf '%s\n' "$ALL" | grep -E "^($REVIEW_GROUPS)"$'\t' || true)"

if [[ "${1:-}" == "--list" ]]; then
  printf '%s\n' "$REVIEW_CURRENT"
  exit 0
fi

fail=0

# (a) An unknown sub-router group = a group with no known posture. Fail: its layer wiring is unreviewed.
UNKNOWN_GROUPS="$(printf '%s\n' "$ALL" | cut -f1 | sort -u | grep -Ev "^($AUTH_COVERED|$REVIEW_GROUPS)$" || true)"
if [[ -n "$UNKNOWN_GROUPS" ]]; then
  echo "audit-route-auth: FAIL — sub-router group(s) with UNKNOWN auth posture:" >&2
  printf '  %s\n' $UNKNOWN_GROUPS >&2
  echo "  Add it to AUTH_COVERED or REVIEW_GROUPS after confirming its create_app layer wiring." >&2
  fail=1
fi

# (b) The layer wiring must still be present — guards against a silently deleted auth layer.
require_substr() {
  grep -q -- "$1" "$ROUTES_FILE" || { echo "audit-route-auth: FAIL — missing auth wiring: '$1' not found in $ROUTES_FILE" >&2; fail=1; }
}
require_substr 'auth::require_auth'
require_substr 'require_system_access'
require_substr 'require_internal_signature'
require_substr 'require_slack_link_signature'

# (c) The review-required route set must match the reviewed baseline.
NORM_BASELINE="$(printf '%s\n' "$BASELINE" | sort -u)"
if [[ "${UPDATE_BASELINE:-}" == "1" ]]; then
  printf '%s\n' "$REVIEW_CURRENT"
  echo "^^^ copy into BASELINE after confirming each route is intentionally unauthenticated and self-gates." >&2
  exit 0
fi
if ! diff <(printf '%s\n' "$NORM_BASELINE") <(printf '%s\n' "$REVIEW_CURRENT") >/tmp/route-auth.diff 2>&1; then
  echo "audit-route-auth: FAIL — the set of unauthenticated/self-gated/signature routes changed." >&2
  echo "A route NOT behind require_auth must carry its own compensating control (secret, signature, PKCE)." >&2
  echo "diff (baseline -> current):" >&2
  cat /tmp/route-auth.diff >&2
  echo "If reviewed and correct: UPDATE_BASELINE=1 .github/scripts/audit-route-auth.sh" >&2
  fail=1
fi

if [[ "$fail" == "0" ]]; then
  echo "audit-route-auth: OK — $(printf '%s\n' "$REVIEW_CURRENT" | grep -c .) reviewed unauth routes; $(printf '%s\n' "$ALL" | grep -Ec "^($AUTH_COVERED)"$'\t') auth-covered; wiring present."
fi
exit "$fail"
