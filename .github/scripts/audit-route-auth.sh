#!/usr/bin/env bash
# audit-route-auth.sh — pin the auth posture of every temper-api route.
#
# WHY THIS EXISTS
# ---------------
# temper-api's routes live in per-group files under crates/temper-api/src/routes/, and the route
# TABLE in that module's mod.rs maps every group to its auth tier; the tier's middleware stack is
# applied in exactly one place (apply_tier). The table's rows are data, and this script asserts
# them — the group set and each row's tier:
#
#   GROUP                        TIER                          POSTURE
#   ---------------------------  ----------------------------  --------------------------------------------------------------
#   auth_only_routes             AuthOnly                      require_auth                          (JWT — authenticated)
#   gated_routes                 Gated                         require_auth + require_system_access  (JWT + system access)
#   admin_routes                 Gated                         same stack as gated_routes            (+ &SystemAdmin proof in every service)
#   public_routes                Public                        (none)                                by-design public: /health
#   blob_commit_routes           Gated (+ inner body limit)    same stack as gated_routes
#   blob_segment_routes          Gated (+ inner body limit)    same stack as gated_routes
#   embed_internal_routes        SelfGated                     (none)                                self-gated: EMBED_DISPATCH_SECRET
#   internal_routes              InternalHmac(Reconcile)       require_internal_signature            HMAC (INTERNAL_RECONCILE_SECRET)
#   slack_link_internal_routes   InternalHmac(SlackLink)       require_slack_link_signature          HMAC (SLACK_LINK_SECRET)
#   slack_mint_internal_routes   InternalHmac(SlackMint)       require_slack_mint_signature          HMAC (SLACK_MINT_SECRET)
#   slack_link_public_routes     SelfGated                     (none)                                by-design public: PKCE+state callback
#   webhook_intake_routes        SelfGated                     (none)                                self-gated: broker RS256 attestation
#
# On webhook_intake_routes: the caller is Vercel Connect forwarding a third-party system's event.
# It is not a temper principal, holds no temper token, and never will -- require_auth is not a
# tightening available here, it is a category error. Its compensating control is inside the
# handler: CredentialBroker::verify_inbound performs RS256-over-JWKS with set_required_spec_claims,
# asserts issuer/audience, asserts the anti-decoy client_id ("api-connex"), and reads the connector
# from the SIGNED trigger claim rather than the unsigned x-trigger-* mirror headers. The anti-decoy
# assertion is load-bearing and non-obvious: the attestation is claim-for-claim identical to the
# deployment's OWN ambient x-vercel-oidc-token except for client_id and trigger, and that ambient
# token rides on every inbound request -- so a verifier that stops at "valid Vercel OIDC token
# naming our project" accepts the deployment's own identity as a forged webhook.
#
# It is a group of its own rather than a route on embed_internal_routes because the controls are
# different in kind, not merely in key: that group compares a shared secret temper issued, this one
# verifies a third party's signature against a remote JWKS. The baseline's job is that each entry
# names the control it actually carries, and one shared group would blur exactly that.
#
# NOTE it gets no tier-stack assertion below, and that is not an omission -- SelfGated applies no
# middleware. Like embed_internal_routes, its gate is inside the handler, so the baseline diff (c)
# is the whole of its protection here: a second route added to this group fails until reviewed.
#
# On slack_mint_internal_routes specifically: it is a THIRD signature group rather than a route on
# slack_link_internal_routes because the keys must differ. Link-state answers "is this principal
# linked?"; the mint route vends an act-as-the-human access token carrying that human's FULL reach
# (resources_visible_to takes a profile and nothing else — there is no narrowing behind it). One
# shared key would make compromise of the cheap capability yield the expensive one. Its signature
# gate is ALSO the only thing enforcing "naming a principal must not be sufficient to mint its
# token" — mint_access_token authorizes nothing itself — so a lost layer here is not a downgrade
# to authenticated-but-broad, it is act-as-any-user.
#
# A route added to auth_only/gated is authenticated by construction — safe, no review needed.
# A route added to any of the OTHER groups is unauthenticated-at-the-middleware (public, a
# self-checked secret, or a signature the handler trusts) and MUST be reviewed: does it really
# self-gate / carry its own compensating control? This script freezes the set of routes in those
# review-required groups, and asserts the table's rows and tier stacks are still present, so:
#   - a new unauthenticated/self-gated/signature route FAILS until acknowledged,
#   - a silently deleted auth layer FAILS immediately,
#   - a group's tier quietly changed FAILS immediately.
# Auth-covered routes (auth_only/gated) grow freely and never trip this.
# See internal/development/security-audit-playbook.md § 1.
#
# USAGE
#   .github/scripts/audit-route-auth.sh          # verify (CI mode)
#   .github/scripts/audit-route-auth.sh --list   # print current review-required routes
#   UPDATE_BASELINE=1 .github/scripts/audit-route-auth.sh   # rewrite baseline after review
#
# ROUTES_FILE may be overridden to point at a single file OR a directory of them (a fixture copy
# of the routes module — see test-audit-route-auth.sh). Under a fixture the baseline diff (c) will
# of course disagree, which is why the test harness asserts on the FAIL MESSAGE, not just the exit
# code.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

ROUTES="${ROUTES_FILE:-crates/temper-api/src/routes}"

# Resolve ROUTES to the list of .rs files to scan: a directory's *.rs (sorted, so group
# attribution is deterministic), or the one file named.
if [ -d "$ROUTES" ]; then
  ROUTES_FILES="$(find "$ROUTES" -maxdepth 1 -name '*.rs' | sort)"
elif [ -f "$ROUTES" ]; then
  ROUTES_FILES="$ROUTES"
else
  echo "audit-route-auth: FAIL — routes source not found: $ROUTES" >&2
  exit 1
fi

# The app builders in routes/mod.rs, and which of them must mount the table. Every
# signature-gated group is served by BOTH builders (see create_internal_app's doc comment: the
# split exists only for Vercel's per-function maxDuration), so both mounts are load-bearing.
APP_BUILDERS='create_app create_internal_app'

# Groups whose routes are authenticated by construction (a require_auth layer). They grow freely.
# `query_routes` is auth-covered NOT by carrying the layers itself but by being MERGED into
# `gated_routes` — it is a sub-router of one only so that its `DefaultBodyLimit` binds to `/api/query`
# and to nothing else. That indirection is exactly what makes a bare AUTH_COVERED entry too weak
# here: the entry asserts a posture, and the posture is a property of where the merge lands. The
# merge-landing assertion below is what closes that, and it is why this name may sit here at all.
AUTH_COVERED='auth_only_routes|gated_routes|admin_routes|query_routes|blob_segment_routes|blob_commit_routes'
# Groups whose routes are NOT behind require_auth — every entry is a reviewed compensating control.
REVIEW_GROUPS='public_routes|embed_internal_routes|internal_routes|slack_link_internal_routes|slack_mint_internal_routes|slack_link_public_routes|webhook_intake_routes'

# Reviewed baseline: <group>\t<handler> for every route in a REVIEW group. Each is unauthenticated
# at the middleware and carries its own control (see the table above). A change here means a new or
# removed unauthenticated/self-gated/signature route — confirm the control, then UPDATE_BASELINE=1.
read -r -d '' BASELINE <<'EOF' || true
embed_internal_routes	handlers::as_reap::reap_as_tables
embed_internal_routes	handlers::embed::dispatch
embed_internal_routes	handlers::embed::warm
embed_internal_routes	handlers::erasure::drain
embed_internal_routes	handlers::internal_call_health::check_internal_calls
embed_internal_routes	handlers::region::dispatch
embed_internal_routes	handlers::sensitivity_sweep::sweep
embed_internal_routes	handlers::slack_disconnect::reap_intents
internal_routes	handlers::internal_saml::reconcile
internal_routes	handlers::internal_saml::resolve_principal
public_routes	handlers::health::health_check
slack_link_internal_routes	handlers::slack_link::slack_link_state
slack_mint_internal_routes	handlers::slack_mint::slack_mint
slack_link_public_routes	handlers::slack_link::callback
webhook_intake_routes	handlers::webhook_intake::receive
EOF

# The gated↔auth_only widening boundary: auth_only is the ONE auth-covered group a handler can
# move into from gated to SILENTLY DROP the system-access gate while keeping JWT auth (moves the
# other way — into a REVIEW group — trip the review baseline above; gated itself grows freely by
# design, so freezing ITS set would trip on every new documented route). Baseline auth_only's
# handler set: a handler ADDED here (a gated→auth_only move among them) fails until acknowledged.
read -r -d '' AUTH_ONLY_BASELINE <<'EOF' || true
auth_only_routes	handlers::access::create_request
auth_only_routes	handlers::access::create_review_request
auth_only_routes	handlers::access::get_own_request
auth_only_routes	handlers::access::get_settings
auth_only_routes	handlers::access::withdraw_request
auth_only_routes	handlers::invitations::accept
auth_only_routes	handlers::invitations::count_mine
auth_only_routes	handlers::invitations::decline
auth_only_routes	handlers::invitations::list_mine
auth_only_routes	handlers::profiles::get
auth_only_routes	handlers::profiles::list_auth_links
auth_only_routes	handlers::profiles::update
auth_only_routes	handlers::slack_disconnect::disconnect_me
EOF

# Every (sub-router group, handler) pair declared in the routes module, keyed on the handler ident
# (stable across single-line and multi-line `.route(` / `routes!(` forms). Each group fn lives in
# its own file with its comments, so attribution is per-file and exact.
extract() {
  awk '
    function grpname(s,   r){ if (match(s,/fn [a-z_]+_routes\(/)){ r=substr(s,RSTART+3); sub(/\(.*/,"",r); return r } return "" }
    { g=grpname($0); if(g!=""){grp=g; next} }
    /^(pub )?fn (create_app|create_internal_app|openapi_spec|apply_transport_layers|cors_layer|fallback_handler|apply_tier|mount_group|route_table)/ { grp="_x_"; next }
    grp=="_x_" || grp=="" { next }
    { s=$0; while (match(s,/handlers::[a-z_]+::[a-z_]+/)) { print grp"\t"substr(s,RSTART,RLENGTH); s=substr(s,RSTART+RLENGTH) } }
  ' $ROUTES_FILES | sort -u
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
  echo "  Add a table row for it in routes/mod.rs (with a tier), then add it to AUTH_COVERED" >&2
  echo "  or REVIEW_GROUPS here after confirming the tier." >&2
  fail=1
fi

# (b) The table's rows and tier stacks must still be present — guards against a silently deleted
#     auth layer or a quietly changed tier.
#
# The pre-table form of this check grepped each app builder's body for each layer's name, sliced
# per builder, because the layers were mounted SEPARATELY in create_app and create_internal_app and
# deleting one mount left the name present elsewhere. The table made that regression structurally
# impossible — both builders mount the same rows through the same apply_tier — and moved the
# surface: now a layer can be lost from a TIER stack, a tier can be changed on a ROW, or a builder
# can stop consuming the table. Each is asserted explicitly.

# Print the body of function NAME from the routes module: its signature line through the closing
# brace at column 0. Handles bare `fn`, `pub fn` and `pub(super)`/`pub(crate)` spellings.
# Bash 3.2 compatible (no assoc arrays) — recomputed per call, the files are small.
fn_body() {
  awk -v fname="$1" '
    $0 ~ "^(pub(\\([a-z]+\\))? )?fn "fname"\\(" { inside=1 }
    inside { print }
    inside && /^\}/ { exit }
  ' $ROUTES_FILES
}

# require_row GROUP FULL_ROW_TEXT — the row asserting GROUP must exist VERBATIM: tier, build fn,
# body_limit and serves all pinned. The #[rustfmt::skip] guarantee makes the whole row one
# greppable line, so a one-token edit anywhere on it (a tier changed, a build fn pointed at
# another group's fn, a body limit nulled, a group mounted by a builder that never served it)
# fails here instead of passing green through a table that no longer says what it mounts.
require_row() {
  local group="$1" row="$2"
  grep -qF -- "$row" $ROUTES_FILES || {
    echo "audit-route-auth: FAIL — table row changed: no row matching: $row" >&2
    echo "  A row's tier, build fn, body limit and served-by set are its reviewed posture. If the" >&2
    echo "  change is intentional it must be reviewed and this assertion updated with routes/mod.rs." >&2
    fail=1
  }
}

require_row 'public_routes'              'Group { key: "public_routes", tier: Tier::Public, build: Documented(public_routes), body_limit: None, serves: Serves::AppOnly },'
require_row 'auth_only_routes'           'Group { key: "auth_only_routes", tier: Tier::AuthOnly, build: Documented(auth_only_routes), body_limit: None, serves: Serves::AppOnly },'
require_row 'gated_routes'               'Group { key: "gated_routes", tier: Tier::Gated, build: Documented(gated_routes), body_limit: None, serves: Serves::AppOnly },'
require_row 'admin_routes'               'Group { key: "admin_routes", tier: Tier::Gated, build: Documented(admin_routes), body_limit: None, serves: Serves::AppOnly },'
require_row 'blob_commit_routes'         'Group { key: "blob_commit_routes", tier: Tier::Gated, build: Documented(blob_commit_routes), body_limit: Some(BodyLimit::CommitDoor), serves: Serves::AppOnly },'
require_row 'blob_segment_routes'        'Group { key: "blob_segment_routes", tier: Tier::Gated, build: Documented(blob_segment_routes), body_limit: Some(BodyLimit::Fixed(blob_doors::BLOB_SEGMENT_MAX_BODY_BYTES)), serves: Serves::AppOnly },'
require_row 'internal_routes'            'Group { key: "internal_routes", tier: Tier::InternalHmac(SignatureKind::Reconcile), build: Undocumented(internal_routes), body_limit: None, serves: Serves::BothBuilders },'
require_row 'slack_link_internal_routes' 'Group { key: "slack_link_internal_routes", tier: Tier::InternalHmac(SignatureKind::SlackLink), build: Undocumented(slack_link_internal_routes), body_limit: None, serves: Serves::BothBuilders },'
require_row 'slack_mint_internal_routes' 'Group { key: "slack_mint_internal_routes", tier: Tier::InternalHmac(SignatureKind::SlackMint), build: Undocumented(slack_mint_internal_routes), body_limit: None, serves: Serves::BothBuilders },'
require_row 'slack_link_public_routes'   'Group { key: "slack_link_public_routes", tier: Tier::SelfGated, build: Undocumented(slack_link_public_routes), body_limit: None, serves: Serves::AppOnly },'
require_row 'embed_internal_routes'      'Group { key: "embed_internal_routes", tier: Tier::SelfGated, build: Undocumented(embed_internal_routes), body_limit: None, serves: Serves::BothBuilders },'
require_row 'webhook_intake_routes'      'Group { key: "webhook_intake_routes", tier: Tier::SelfGated, build: Undocumented(webhook_intake_routes), body_limit: None, serves: Serves::AppOnly },'

# The tier stacks: apply_tier is the one place a middleware stack is spelled. Every middleware
# name the postures promise must appear in its body — a name dropped here un-gates every group
# whose tier names it. (Bodies are captured and matched as strings, never piped to `grep -q`:
# under `set -o pipefail`, grep's early exit on match SIGPIPEs the producer mid-stream and turns
# a MATCH into a pipeline failure.)
TIER_BODY="$(fn_body apply_tier)"
assert_in_body() {
  local layer="$1"; shift
  [[ "$TIER_BODY" == *"$layer"* ]] || {
    echo "audit-route-auth: FAIL — missing auth wiring: '$layer' not applied by apply_tier in $ROUTES" >&2
    echo "  A tier's stack is applied exactly once; a middleware name missing from it serves every" >&2
    echo "  group of that tier unauthenticated (or unlimited, for the rate seam)." >&2
    fail=1
  }
}
assert_in_body 'auth::require_auth'
assert_in_body 'require_system_access'
assert_in_body 'require_relay_trust'
assert_in_body 'require_route_rate_limit'
assert_in_body 'require_internal_signature'
assert_in_body 'require_slack_link_signature'
assert_in_body 'require_slack_mint_signature'
# The body ceilings are security controls too: the gated router's 25 MB inheritance and the
# doors' inner limits. A name dropped here re-widens a streamed-body window silently.
assert_in_body 'DefaultBodyLimit::max(GATED_MAX_BODY_BYTES)'

# ORDER is load-bearing in apply_tier, not just presence: the arms add middlewares INNER first
# (last-added is outermost and runs first, per the module doc), so the byte offsets must be
# strictly increasing along each arm's addition order. A swapped pair changes which refusal an
# unauthenticated caller meets first — a behavior change presence-greps stay green through.
require_order() {
  local fn_name="$1"; shift
  fn_body "$fn_name" | awk -v fn="$fn_name" -v names="$*" '
    BEGIN { n = split(names, arr, " ") }
    { body = body $0 "\n" }
    END {
      # No `next`/`continue` here: gawk fatally rejects `next` in an END action, and the
      # macOS awk that accepted it is not the awk CI runs.
      cursor = 1; bad = 0
      for (i = 1; i <= n; i++) {
        # Sequential search: each name must first occur AFTER the previous one, so a name
        # reused across match arms (auth::require_auth is in AuthOnly AND Gated) resolves to
        # THIS arm occurrence, not the first one in the file.
        idx = index(substr(body, cursor), arr[i])
        if (idx == 0) {
          print "audit-route-auth: FAIL — order pin: \x27" arr[i] "\x27 not found in " fn "'"'"'s body (after offset " cursor-1 ")" > "/dev/stderr"
          bad = 1
        } else {
          cursor = cursor + idx - 1 + length(arr[i])
        }
      }
      exit bad
    }' || fail=1
}
require_order apply_tier 'system_access::require_system_access' 'auth::require_auth' 'require_relay_trust' 'DefaultBodyLimit::max(GATED_MAX_BODY_BYTES)'
require_order apply_tier 'rate_limit::require_route_rate_limit' 'internal_auth::require_internal_signature'

# Both builders must mount FROM the table. A builder that stops consuming it (e.g. re-derives its
# own router) is a second, drift-prone wiring path — the exact thing the table exists to prevent.
for app in $APP_BUILDERS; do
  fn_body "$app" | grep -q 'mount_group' || {
    echo "audit-route-auth: FAIL — app builder '$app' does not mount from the route table" >&2
    echo "  Both builders must consume route_table() via mount_group; a builder assembling its" >&2
    echo "  own router is a parallel wiring path the posture assertions cannot see." >&2
    fail=1
  }
done

# A merged sub-router inherits its posture from the group it lands in, so the LANDING is the wiring.
# `query_routes` carries no auth layer of its own — it exists to scope a body limit — and is
# auth-covered only for as long as `gated_routes` is where it is merged.
GATED_BODY="$(fn_body gated_routes)"
[[ "$GATED_BODY" == *'query_routes()'* ]] || {
  echo "audit-route-auth: FAIL — missing merge: 'query_routes()' not merged into gated_routes()" >&2
  echo "  The query door is auth-covered only for as long as it merges into the gated group." >&2
  fail=1
}

# (b2) The admin group is admin-ONLY by membership, not just by tier. Its tier is the gated stack,
# which admits any approved principal; what makes a route there admin-only is the handler minting
# the sealed `&SystemAdmin` proof (`require_system_admin`, or the erasure doors' 404-rendering
# `require_erasure_operator`) that its service then requires. A scoped handler (owner/maintainer/
# actor arms — the ledger, machine-client siblings, reblock) mounted here would be documented under
# the `Admin` tag as admin-only while admitting non-admins. This asserts PRESENCE of the mint in the
# handler's body; that the mint precedes any lookup is the service signature's job (the proof is a
# parameter, so nothing can run on the service side without it) plus review.
HANDLERS_DIR="${HANDLERS_DIR:-crates/temper-api/src/handlers}"
ADMIN_HANDLERS="$(printf '%s\n' "$ALL" | awk -F'\t' '$1=="admin_routes"{print $2}')"
if [[ -z "$ADMIN_HANDLERS" ]]; then
  echo "audit-route-auth: FAIL — admin_routes mounts no handlers (or the group was renamed)" >&2
  fail=1
fi
for h in $ADMIN_HANDLERS; do
  mod="$(printf '%s' "$h" | cut -d: -f3)"
  fname="$(printf '%s' "$h" | cut -d: -f5)"
  body="$(awk -v f="pub async fn ${fname}(" 'index($0,f){p=1} p{print} p&&/^}/{exit}' "${HANDLERS_DIR}/${mod}.rs" 2>/dev/null || true)"
  if [[ -z "$body" ]]; then
    echo "audit-route-auth: FAIL — admin_routes mounts $h but its body was not found in ${HANDLERS_DIR}/${mod}.rs" >&2
    fail=1
  elif [[ "$body" != *"require_system_admin("* && "$body" != *"require_erasure_operator("* ]]; then
    echo "audit-route-auth: FAIL — admin_routes mounts $h, which does not mint the &SystemAdmin proof" >&2
    echo "  Every route in routes/admin.rs must be admin-only: its handler mints the proof via" >&2
    echo "  require_system_admin (or require_erasure_operator). A scoped handler belongs in gated_routes." >&2
    fail=1
  fi
done

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

# (d) The gated→auth_only widening boundary: a handler ADDED to auth_only fails until
# acknowledged — the one auth-covered group a silent move into drops the system-access gate.
AUTH_ONLY_CURRENT="$(printf '%s\n' "$ALL" | grep -E '^auth_only_routes'$'\t' || true)"
NORM_AUTH_ONLY="$(printf '%s\n' "$AUTH_ONLY_BASELINE" | sort -u)"
if ! diff <(printf '%s\n' "$NORM_AUTH_ONLY") <(printf '%s\n' "$AUTH_ONLY_CURRENT") >/tmp/route-auth-authonly.diff 2>&1; then
  echo "audit-route-auth: FAIL — the auth_only group's handler set changed." >&2
  echo "A handler moved INTO auth_only from a gated group drops require_system_access while keeping" >&2
  echo "JWT auth — a privilege widening. diff (baseline -> current):" >&2
  cat /tmp/route-auth-authonly.diff >&2
  echo "If reviewed and correct: extend AUTH_ONLY_BASELINE in this script." >&2
  fail=1
fi

if [[ "$fail" == "0" ]]; then
  echo "audit-route-auth: OK — $(printf '%s\n' "$REVIEW_CURRENT" | grep -c .) reviewed unauth routes; $(printf '%s\n' "$ALL" | grep -Ec "^($AUTH_COVERED)"$'\t') auth-covered; table rows and wiring present."
fi
exit "$fail"
