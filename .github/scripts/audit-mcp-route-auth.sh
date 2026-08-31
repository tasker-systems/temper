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
# too — so a route added to ANY group, including mcp_routes itself, fails until reviewed. The
# handler column freezes the full `::`-chained ident set, joined with `+` when one route carries
# several (a method appended to a reviewed route changes the triple); routes whose handler is an
# inline closure are frozen as `<inline>`, keyed by their path.
#
# The wiring assertion is made WITHIN the `let mcp_routes` block, not over the file — the same
# lesson audit-route-auth.sh learned at its (b): a name grepped anywhere is a name that can be
# present elsewhere in the file while the block that needs it has lost it. The match is a WHOLE
# ident on a comment-stripped line, so `require_mcp_auth_v2` or the name surviving only in a
# comment does not satisfy it. If the `let mcp_routes` declaration is renamed or removed, this
# fails loudly rather than silently skipping.
#
# ASSEMBLY SHAPES the parser cannot freeze are refused, not skipped:
#   - `.merge(` may name only the four reviewed group idents — a helper function's router merged
#     in, whose routes live outside build_router, is exactly a new public surface the baseline
#     cannot see;
#   - `.nest(` fails outright (its inner routes would live outside every triple);
#   - `.nest_service(` must be the reviewed `("/mcp", mcp_service)` and nothing else;
#   - a `.route(` call that yields no literal path string (a const path, a concatenated path)
#     prints an UNPARSEABLE marker and fails — a non-literal path cannot be frozen.
#
# FIELD OF VIEW — stated so green is never mistaken for more than it checks:
#   - THIS guard watches build_router's body in crates/temper-mcp/src/router.rs ONLY — and it
#     MECHANICALLY backs the "only" with check (a2): Router::new() or Router::default() anywhere
#     else under crates/temper-mcp/src fails, so a second router-assembly site cannot grow
#     silently outside the frozen one.
#   - It does not watch temper-api's routes (audit-route-auth.sh), the AS's api/** entry points
#     (audit-as-entry-points.sh), or service-layer predicate drift (audit-handler-authz-drift.sh).
#     The api/*.rs BINS' contents are checked for assembly tokens by the AS guard, which owns
#     those files.
#   - Layers applied outside build_router (apply_base_layers, cors, root_span) are
#     transport/observability; if auth logic ever moves outside build_router, re-read this guard
#     before trusting it.
#   - Vercel maps /mcp(.*) and the /.well-known, /oauth catch-alls at api/mcp.rs to THIS router
#     (see vercel.json); the AS guard freezes that mapping half.
#
# COMMENTS are stripped before matching, so prose may name route-shaped text; a comment cannot
# freeze or trip anything — `//` to end-of-line AND `/* ... */` across lines (a block comment
# naming require_mcp_auth must not satisfy the wiring assertion). (This assumes no `//` or `/*`
# inside a string literal in build_router — true of every path and handler in the file today; a
# route path containing either would truncate its line at the wrong place and RED here, which is
# the safe direction.)
strip_comments() {
  awk '
    BEGIN { in_block=0 }
    {
      line=$0; out=""
      while (length(line) > 0) {
        if (in_block) {
          e = index(line, "*/")
          if (e == 0) { line="" } else { in_block=0; line=substr(line, e+2) }
        } else {
          s = index(line, "/*"); h = index(line, "//")
          if (s == 0 && h == 0) { out = out line; line = "" }
          else if (s > 0 && (h == 0 || s < h)) { out = out substr(line, 1, s-1); in_block=1; line=substr(line, s+2) }
          else { out = out substr(line, 1, h-1); line = "" }
        }
      }
      print out
    }
  '
}
#
# UPDATE_BASELINE=1 rewrites nothing and only prints the current set, and refuses to run at all
# in CI or while any check above is failing — update mode cannot launder an unresolved failure.
#
# USAGE
#   .github/scripts/audit-mcp-route-auth.sh          # verify (CI mode)
#   .github/scripts/audit-mcp-route-auth.sh --list   # print the current frozen route set
#   UPDATE_BASELINE=1 .github/scripts/audit-mcp-route-auth.sh   # print baseline after review
#
# ROUTER_FILE / MCP_SRC_DIR may be overridden to point at fixtures (see
# test-audit-mcp-route-auth.sh). Under a fixture the baseline (c) will of course disagree, which
# is why the test harness asserts on the FAIL MESSAGE, not just the exit code.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

ROUTER_FILE="${ROUTER_FILE:-crates/temper-mcp/src/router.rs}"
MCP_SRC_DIR="${MCP_SRC_DIR:-crates/temper-mcp/src}"
# Absolute form of ROUTER_FILE: grep -rl prints paths relative to MCP_SRC_DIR's form, so the
# self-exclusion in (a2) must compare like with like.
ROUTER_ABS="$(cd "$(dirname "$ROUTER_FILE")" && pwd)/$(basename "$ROUTER_FILE")"
GUARD_NAME="audit-mcp-route-auth"

# The sub-router declarations in build_router and their reviewed posture. A new `let X =
# Router::new()` declaration fails until classified, even if it carries no routes yet.
KNOWN_GROUPS='discovery_routes|registration_routes|health|mcp_routes'
# The only group idents `.merge(` may name — anything else is router assembly this guard's
# baseline cannot see (the routes would live outside build_router's slice).
REVIEWED_MERGES='discovery_routes|registration_routes|health|mcp_routes'

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
# Preprocessing: `//` comments are stripped first, so prose can never freeze or trip anything.
#
# The route-call state machine: `.route(` and `.nest_service(` open a call that rustfmt lays out
# across up to three lines (open / path / handler / close) — or one, when the declaration and its
# first route share a line (`let health = Router::new().route(...)`). While inside one, the first
# `"/..."` string is the path and every `::`-chained ident NOT preceded by an identifier char is
# collected (joined with `+`), so a method appended to a reviewed route changes the triple and
# `Router::new` yields nothing. The call ends on any line whose last non-space character closes a
# paren (`)`, `),`, `);`, `));`, `)),`); a call that closes WITHOUT a captured literal path emits
# an UNPARSEABLE marker — a non-literal path cannot be frozen, and the guard fails on the marker.
extract() {
  awk '
    BEGIN { in_block=0 }
    # Slice to build_router first.
    /^(pub )?fn build_router\(/ { inside=1 }
    inside && /^}/ { exit }
    !inside { next }
    {
      line=$0
      # Strip // to EOL and /* ... */ across lines, so prose can neither freeze nor trip.
      out=""
      while (length(line) > 0) {
        if (in_block) {
          e = index(line, "*/")
          if (e == 0) { line="" } else { in_block=0; line=substr(line, e+2) }
        } else {
          s = index(line, "/*"); h = index(line, "//")
          if (s == 0 && h == 0) { out = out line; line = "" }
          else if (s > 0 && (h == 0 || s < h)) { out = out substr(line, 1, s-1); in_block=1; line=substr(line, s+2) }
          else { out = out substr(line, 1, h-1); line = "" }
        }
      }
      line=out
      # New sub-router declaration: it becomes the current attribution group. Do NOT skip the
      # rest of the line — rustfmt may lay the first `.route(` on it — but DO blank the
      # declaration itself, or the ident scan below would freeze `Router::new` as a handler.
      if (match(line, /let [a-z_]+ = Router::new\(\)/)) {
        line=substr(line, 1, RSTART-1) substr(line, RSTART+RLENGTH)
        grp=substr($0, RSTART+4, RLENGTH-4)
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
        if (!is_nest && line ~ /[A-Za-z_][A-Za-z0-9_]*(::[A-Za-z_][A-Za-z0-9_]*)+/) {
          s=line
          while (match(s, /[A-Za-z_][A-Za-z0-9_]*(::[A-Za-z_][A-Za-z0-9_]*)+/)) {
            pre = (RSTART==1) ? " " : substr(s,RSTART-1,1)
            post = (RSTART+RLENGTH > length(s)) ? " " : substr(s,RSTART+RLENGTH,1)
            if (pre !~ /[A-Za-z0-9_]/ && post !~ /[A-Za-z0-9_]/) {
              cand=substr(s,RSTART,RLENGTH)
              if (index(handler, cand) == 0) handler = (handler=="") ? cand : handler "+" cand
            }
            s=substr(s, RSTART+RLENGTH)
          }
        }
        # Close the call: any line whose last non-space char closes a paren.
        if (line ~ /\)[,;]?[[:space:]]*$/) {
          if (got_path) {
            if (is_nest) { h="<nest_service>" } else if (handler=="") { h="<inline>" } else { h=handler }
            print grp"\t"path"\t"h
          } else {
            print "UNPARSEABLE\t"grp"\t<no literal path>"
          }
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
# empty: its posture is unreviewed by definition. UNPARSEABLE rows are not groups — they are
# already reported by (a5) with their own message.
UNKNOWN_GROUPS="$(printf '%s\n' "$ALL" | grep -v '^UNPARSEABLE' | cut -f1 | sort -u | grep -Ev "^($KNOWN_GROUPS)$" || true)"
if [[ -n "$UNKNOWN_GROUPS" ]]; then
  echo "$GUARD_NAME: FAIL — sub-router group(s) with UNKNOWN auth posture:" >&2
  printf '  %s\n' $UNKNOWN_GROUPS >&2
  echo "  Classify it: public-by-design (join the baseline with its reason) or gated (mount the" >&2
  echo "  auth layer in its own block and add it to KNOWN_GROUPS)." >&2
  fail=1
fi

# (a2) Router assembly must stay inside the one file this guard freezes. A second Router::new()
# or Router::default() site elsewhere under the crate would be invisible to every check above —
# this is the mechanical form of the field-of-view statement in the header.
OTHER_ASSEMBLY="$(grep -rlE 'Router::(new|default)' "$MCP_SRC_DIR" --include='*.rs' | while IFS= read -r p; do
  p_abs="$(cd "$(dirname "$p")" && pwd)/$(basename "$p")"
  [[ "$p_abs" != "$ROUTER_ABS" ]] && printf '%s\n' "$p"
done || true)"
if [[ -n "$OTHER_ASSEMBLY" ]]; then
  echo "$GUARD_NAME: FAIL — router assembly outside the frozen router file:" >&2
  printf '  %s\n' $OTHER_ASSEMBLY >&2
  echo "  This guard can only see build_router in $ROUTER_FILE. A second assembly site" >&2
  echo "  needs its own reviewed posture BEFORE it can go green — extend this guard or fold" >&2
  echo "  the site into build_router." >&2
  fail=1
fi

# The build_router body, comments stripped — the slice every structural check below reads, so
# prose can neither satisfy nor evade them.
ROUTER_BODY="$(awk '/^(pub )?fn build_router\(/ {inside=1} inside && !/^(pub )?fn build_router\(/ {print} inside && /^\}/ {exit}' "$ROUTER_FILE" | strip_comments)"

# (a3) `.merge(` may name only the reviewed group idents — checked PER OCCURRENCE, not per line:
# one whitelisted merge on a line must not launder a second, non-whitelisted merge beside it.
# A nested or multiline merge argument does not match the extractable form and is flagged too.
BAD_MERGE="$(printf '%s\n' "$ROUTER_BODY" | grep -oE '\.merge\([^)]*\)' | grep -vE "^\.merge\(($REVIEWED_MERGES)\)$" || true)"
TRAILING_MERGE="$(printf '%s\n' "$ROUTER_BODY" | grep -E '\.merge\([[:space:]]*$' || true)"
if [[ -n "$BAD_MERGE" || -n "$TRAILING_MERGE" ]]; then
  echo "$GUARD_NAME: FAIL — .merge() argument outside the reviewed groups:" >&2
  printf '  %s\n' $BAD_MERGE $TRAILING_MERGE >&2
  echo "  A merged router whose routes are declared elsewhere is invisible to the frozen" >&2
  echo "  baseline. Inline the routes into a named group in build_router, or extend this" >&2
  echo "  guard deliberately." >&2
  fail=1
fi

# (a4) `.nest(` is refused outright: its inner routes would live outside every frozen triple.
# `.nest_service(` must be the reviewed ("/mcp", mcp_service) form and nothing else.
BAD_NEST="$(printf '%s\n' "$ROUTER_BODY" | grep -E '\.nest\(' || true)"
if [[ -n "$BAD_NEST" ]]; then
  echo "$GUARD_NAME: FAIL — .nest() found in build_router:" >&2
  printf '  %s\n' "$BAD_NEST" >&2
  echo "  A nested inline router's routes live outside every frozen triple. Inline them into" >&2
  echo "  a named group, or re-read and extend this guard deliberately." >&2
  fail=1
fi
BAD_NEST_SVC="$(printf '%s\n' "$ROUTER_BODY" | grep -E '\.nest_service\(' | grep -vE '\.nest_service\("/mcp", *mcp_service\)' || true)"
if [[ -n "$BAD_NEST_SVC" ]]; then
  echo "$GUARD_NAME: FAIL — .nest_service() with an unreviewed argument:" >&2
  printf '  %s\n' "$BAD_NEST_SVC" >&2
  echo "  The only reviewed nest is nest_service(\"/mcp\", mcp_service), gated by the auth layer" >&2
  echo "  asserted in (b). Any other nest serves its target outside the frozen baseline." >&2
  fail=1
fi

# (a5) A route call that closed without a literal path cannot be frozen — a const or composed
# path would be invisible to the baseline while its route serves.
if printf '%s\n' "$ALL" | grep -q '^UNPARSEABLE'; then
  echo "$GUARD_NAME: FAIL — a .route( call produced no literal path string:" >&2
  printf '%s\n' "$(printf '%s\n' "$ALL" | grep '^UNPARSEABLE')" >&2
  echo "  Paths must be literals in build_router; a const or composed path cannot be frozen by" >&2
  echo "  this guard. Inline the literal, or re-read and extend this guard deliberately." >&2
  fail=1
fi

# (b) The auth wiring must still be present — asserted WITHIN the mcp_routes block, not the file,
# against a comment-stripped slice, as a WHOLE ident: `require_mcp_auth_v2` does not satisfy it,
# and the real name surviving only in a comment does not either.
auth_block_body="$(awk -v blk="$AUTH_BLOCK" '
  $0 ~ "let "blk" = Router::new\\(\\)" { inside=1 }
  inside { print }
  inside && /;[[:space:]]*$/ { exit }
' "$ROUTER_FILE" | strip_comments)"
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
  printf '%s\n' "$auth_block_body" | grep -Eq "(^|[^A-Za-z0-9_])${AUTH_LAYER}([^A-Za-z0-9_]|$)" || {
    echo "$GUARD_NAME: FAIL — missing auth wiring: '$AUTH_LAYER' not mounted in the $AUTH_BLOCK" >&2
    echo "  block of $ROUTER_FILE (as a whole ident on a non-comment line). A name elsewhere in" >&2
    echo "  the file does not gate THIS block; a /mcp request would be served ungated. See the" >&2
    echo "  block-level lesson in audit-route-auth.sh (b)." >&2
    fail=1
  }
fi

# (c) The frozen route set must match the reviewed baseline — additions, removals, handler swaps,
# and appended methods all land here.
NORM_BASELINE="$(printf '%s\n' "$BASELINE" | sort -u)"
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

# Update mode runs LAST and refuses to launder: not in CI, and not while any check is failing.
if [[ -n "${UPDATE_BASELINE:-}" ]]; then
  if [[ -n "${CI:-}" ]]; then
    echo "$GUARD_NAME: UPDATE_BASELINE is not available in CI — re-baseline locally after review." >&2
    exit 1
  fi
  if [[ "$fail" != "0" ]]; then
    echo "$GUARD_NAME: UPDATE_BASELINE refused — resolve the failures above first; update mode" >&2
    echo "  cannot launder an unresolved wiring or assembly failure." >&2
    exit 1
  fi
  printf '%s\n' "$ALL"
  echo "^^^ copy into BASELINE after confirming each route's posture." >&2
  exit 0
fi

if [[ "$fail" == "0" ]]; then
  echo "$GUARD_NAME: OK — $(printf '%s\n' "$ALL" | grep -c .) frozen routes (3 public-by-design, /mcp gated); auth wiring present in $AUTH_BLOCK; assembly shapes clean."
fi
exit "$fail"
