#!/usr/bin/env bash
# audit-mcp-route-auth.sh — pin the auth posture of every temper-mcp router route.
#
# WHY THIS EXISTS
# ---------------
# audit-route-auth.sh freezes the unauthenticated route set of crates/temper-api/src/routes.rs —
# and ONLY that file. temper-mcp's router (crates/temper-mcp/src/router.rs) is a second routing
# surface, built in a different shape: one `build_router` function assembling sub-routers inline
# with `.merge()`. A new public route added to THAT surface trips no guard. This script freezes
# the MCP router's route set and asserts the auth layer rides the block that must carry it:
#
#   GROUP               POSTURE (set in build_router)
#   ------------------  --------------------------------------------------------------
#   discovery_routes    (none)  by design: RFC 9728 protected-resource metadata
#   registration_routes (none)  by design: thin DCR proxy returning the pre-registered client_id
#   health              (none)  by design: /mcp/health liveness
#   mcp_routes          require_mcp_auth (JWT — authenticated)
#
# The baseline freezes every (group, path, handler) triple in build_router — auth-covered entries
# too — so a route added to ANY group, including mcp_routes itself, fails until reviewed. Handler
# idents are part of the frozen triple: swapping the function behind a reviewed path is exactly the
# edit a baseline keyed on paths alone would wave through. Routes whose handler is an inline
# closure are frozen as `<inline>`, keyed by their path.
#
# The wiring assertion is made WITHIN the `let mcp_routes` block, not over the file — the same
# lesson audit-route-auth.sh learned at its (b): a name grepped anywhere is a name that can be
# present elsewhere in the file while the block that needs it has lost it. If the `let mcp_routes`
# declaration is renamed or removed, this fails loudly rather than silently skipping.
#
# FIELD OF VIEW — stated so green is never mistaken for more than it checks:
#   - THIS guard watches build_router's body in crates/temper-mcp/src/router.rs ONLY.
#   - It does not watch temper-api's routes (audit-route-auth.sh), the AS's api/** entry points
#     (audit-as-entry-points.sh), or service-layer predicate drift (audit-handler-authz-drift.sh).
#   - It watches the routes and layers declared INSIDE build_router. Layers applied outside it
#     (apply_base_layers, cors, root_span) are transport/observability; if auth logic ever moves
#     outside build_router, re-read this guard before trusting it.
#   - Vercel maps /mcp(.*) and the /.well-known, /oauth catch-alls at api/mcp.rs to THIS router
#     (see vercel.json); the AS guard freezes that mapping half.
#
# USAGE
#   .github/scripts/audit-mcp-route-auth.sh          # verify (CI mode)
#   .github/scripts/audit-mcp-route-auth.sh --list   # print the current frozen route set
#   UPDATE_BASELINE=1 .github/scripts/audit-mcp-route-auth.sh   # rewrite baseline after review
#
# ROUTER_FILE may be overridden to point at a fixture (see test-audit-mcp-route-auth.sh). Under a
# fixture the baseline (c) will of course disagree, which is why the test harness asserts on the
# FAIL MESSAGE, not just the exit code.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

ROUTER_FILE="${ROUTER_FILE:-crates/temper-mcp/src/router.rs}"
GUARD_NAME="audit-mcp-route-auth"

# The sub-router declarations in build_router and their reviewed posture. A new `let X =
# Router::new()` declaration fails until classified, even if it carries no routes yet.
KNOWN_GROUPS='discovery_routes|registration_routes|health|mcp_routes'

# The block whose body must carry the auth layer. Both halves are asserted inside the slice:
# the nest that mounts the protected service, and the layer that gates it. A block that lost
# either serves /mcp ungated, and no whole-file grep is trusted to say otherwise.
AUTH_BLOCK='mcp_routes'
AUTH_NEST='nest_service("/mcp"'
AUTH_LAYER='require_mcp_auth'

# Reviewed baseline: <group>\t<path>\t<handler> for every route/nest in build_router. A change
# here means a new, removed, or re-handled route — confirm the posture, then UPDATE_BASELINE=1.
read -r -d '' BASELINE <<'EOF' || true
discovery_routes	/.well-known/oauth-protected-resource	discovery::oauth_protected_resource
health	/mcp/health	<inline>
mcp_routes	/mcp	<nest_service>
registration_routes	/oauth/register	discovery::register_client
EOF

# Every (group, path, handler) triple declared in build_router, attributed to the most recent
# `let NAME = Router::new()` declaration. Bash 3.2 compatible.
#
# The route-call state machine: `.route(` and `.nest_service(` open a call that rustfmt lays out
# across up to three lines (open / path / handler / close) — or one, when the declaration and its
# first route share a line (`let health = Router::new().route(...)`). While inside one, the first
# `"/..."` string is the path and any `mod::fn` ident is the handler; the call ends on any line
# whose last non-space character closes a paren (`)`, `),`, `);`, `));`, `)),`). A `"/path"`
# string outside any route call is not collected — layers and state wiring carry none today, and
# one that did must be reviewed here before the guard can say what it is.
extract() {
  awk '
    # Slice to build_router first.
    /^(pub )?fn build_router\(/ { inside=1 }
    inside && /^}/ { exit }
    !inside { next }
    {
      line=$0
      # New sub-router declaration: it becomes the current attribution group. Do NOT skip the
      # rest of the line — rustfmt may lay the first `.route(` on it.
      if (match(line, /let [a-z_]+ = Router::new\(\)/)) {
        grp=substr(line, RSTART+4, RLENGTH-4)
        sub(/ = Router::new\(\).*$/, "", grp)
      }
      # Open a route call.
      if (!in_route && index(line, ".route(") > 0) { in_route=1; path=""; handler=""; got_path=0; is_nest=0 }
      if (!in_route && index(line, ".nest_service(") > 0) { in_route=1; path=""; handler=""; got_path=0; is_nest=1 }
      if (in_route) {
        if (!got_path && match(line, /"\/[^"]*"/)) {
          path=substr(line, RSTART+1, RLENGTH-2)
          got_path=1
        }
        if (!is_nest && line ~ /[a-z_]+::[a-z_]+/) {
          # First ident NOT preceded by an identifier char — `Router::new` must not yield
          # `outer::new`, but `get(discovery::handler)` must yield the handler.
          s=line
          while (match(s, /[a-z_]+::[a-z_]+/)) {
            pre = (RSTART==1) ? " " : substr(s,RSTART-1,1)
            if (pre !~ /[A-Za-z0-9_]/) { handler=substr(s,RSTART,RLENGTH); break }
            s=substr(s, RSTART+RLENGTH)
          }
        }
        # Close the call: any line whose last non-space char closes a paren. The declaration line
        # itself cannot be this (it ends in `)` of Router::new() — checked AFTER capture, and a
        # same-line route has already been captured by then).
        if (got_path && line ~ /\)[,;]?[[:space:]]*$/) {
          if (is_nest) { h="<nest_service>" } else if (handler=="") { h="<inline>" } else { h=handler }
          print grp"\t"path"\t"h
          in_route=0; is_nest=0
        }
      }
    }
  ' "$ROUTER_FILE" | sort -u
}

ALL="$(extract)"
if [[ -z "$ALL" ]]; then
  echo "$GUARD_NAME: FAIL — no routes extracted from $ROUTER_FILE. build_router renamed, moved," >&2
  echo "  or reshaped past this parser? The guard must be re-read, not re-baselined blind." >&2
  exit 1
fi

if [[ "${1:-}" == "--list" ]]; then
  printf '%s\n' "$ALL"
  exit 0
fi

fail=0

# (a) An unknown sub-router group = a declaration with no reviewed posture. Fail even if it is
# empty: its posture is unreviewed by definition.
UNKNOWN_GROUPS="$(printf '%s\n' "$ALL" | cut -f1 | sort -u | grep -Ev "^($KNOWN_GROUPS)$" || true)"
if [[ -n "$UNKNOWN_GROUPS" ]]; then
  echo "$GUARD_NAME: FAIL — sub-router group(s) with UNKNOWN auth posture:" >&2
  printf '  %s\n' $UNKNOWN_GROUPS >&2
  echo "  Classify it: public-by-design (join the baseline with its reason) or gated (mount the" >&2
  echo "  auth layer in its own block and add it to KNOWN_GROUPS)." >&2
  fail=1
fi

# (b) The auth wiring must still be present — asserted WITHIN the mcp_routes block, not the file.
# Slice from the `let mcp_routes = Router::new()` line through the first line ending the let.
auth_block_body="$(awk -v blk="$AUTH_BLOCK" '
  $0 ~ "let "blk" = Router::new\\(\\)" { inside=1 }
  inside { print }
  inside && /;[[:space:]]*$/ { exit }
' "$ROUTER_FILE")"
if [[ -z "$auth_block_body" ]]; then
  echo "$GUARD_NAME: FAIL — sub-router group '$AUTH_BLOCK' not found in $ROUTER_FILE (renamed or" >&2
  echo "  removed?). /mcp is the gated surface; the guard must be re-read, not re-baselined blind." >&2
  fail=1
else
  printf '%s\n' "$auth_block_body" | grep -qF -- "$AUTH_NEST" || {
    echo "$GUARD_NAME: FAIL — missing wiring: '$AUTH_NEST' not present in the $AUTH_BLOCK block" >&2
    echo "  of $ROUTER_FILE. The protected MCP service is no longer mounted where the auth layer" >&2
    echo "  gates it. Re-read the router, then re-baseline." >&2
    fail=1
  }
  printf '%s\n' "$auth_block_body" | grep -qF -- "$AUTH_LAYER" || {
    echo "$GUARD_NAME: FAIL — missing auth wiring: '$AUTH_LAYER' not mounted in the $AUTH_BLOCK" >&2
    echo "  block of $ROUTER_FILE. A name elsewhere in the file does not gate THIS block; a /mcp" >&2
    echo "  request would be served ungated. See the block-level lesson in audit-route-auth.sh (b)." >&2
    fail=1
  }
fi

# (c) The frozen route set must match the reviewed baseline — additions, removals, and handler
# swaps all land here.
NORM_BASELINE="$(printf '%s\n' "$BASELINE" | sort -u)"
if [[ "${UPDATE_BASELINE:-}" == "1" ]]; then
  printf '%s\n' "$ALL"
  echo "^^^ copy into BASELINE after confirming each route's posture." >&2
  exit 0
fi
DIFF_FILE="$(mktemp)"
trap 'rm -f "$DIFF_FILE"' EXIT
if ! diff <(printf '%s\n' "$NORM_BASELINE") <(printf '%s\n' "$ALL") >"$DIFF_FILE" 2>&1; then
  echo "$GUARD_NAME: FAIL — the MCP router's route set changed." >&2
  echo "Every route in build_router is frozen: public-by-design entries carry their reason in the" >&2
  echo "guard header, and anything gated must mount its auth layer in its own block." >&2
  echo "diff (baseline -> current):" >&2
  cat "$DIFF_FILE" >&2
  echo "If reviewed and correct: UPDATE_BASELINE=1 .github/scripts/$GUARD_NAME.sh" >&2
  fail=1
fi

if [[ "$fail" == "0" ]]; then
  echo "$GUARD_NAME: OK — $(printf '%s\n' "$ALL" | grep -c .) frozen routes (3 public-by-design, /mcp gated); auth wiring present in $AUTH_BLOCK."
fi
exit "$fail"
