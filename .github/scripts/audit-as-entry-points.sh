#!/usr/bin/env bash
# audit-as-entry-points.sh — pin the entry-point set of the Temper AS and its vercel.json wiring.
#
# WHY THIS EXISTS
# ---------------
# The Temper AS's public surface is TWO trees that must agree: the api/** files Vercel builds into
# serverless functions, and the vercel.json `routes`/`functions` that map public paths onto them.
# The three shapes audit-route-auth.sh and audit-mcp-route-auth.sh watch are router-in-code; THIS
# surface's routing is a filesystem convention plus a JSON map, and neither of those guards reads
# either. A new api/** entry point — or a new route line naming one — is reachable the moment it
# lands, and until now tripped no guard.
#
# Every entry point below is public BY PROTOCOL (discovery, JWKS, authorize, token, SAML
# login/ACS/metadata, the two OAuth callbacks) or is a full server carrying its own reviewed
# posture (axum → audit-route-auth.sh, mcp → audit-mcp-route-auth.sh, internal → its HMAC gates).
# So the property this guard wants is NOT a posture judgement: it is *"a tenth became an
# eleventh"*. Its value is that the set cannot grow — or silently shrink, or dangle — without a
# human looking at it and recording that they did:
#
#   (a) every file under api/** is a reviewed baseline entry — a new file fails until reviewed;
#   (b) every baseline entry still exists on disk — a stale entry fails symmetrically;
#   (c) every vercel.json `routes` dest starting /api/ resolves to a file that exists — a route
#       mapping a public path at nothing fails;
#   (d) every vercel.json `functions` key is a file that exists — a renamed/deleted file left in
#       the functions map fails;
#   (e) the `routes` array itself is frozen, jq-normalized, as a whole — a NEW route line mapping
#       a public path at an ALREADY-REVIEWED file is a new public path, and freezing only the
#       dest/file pairs would wave it through. Reordering is also a semantic change in
#       vercel.json (first match wins) and fails with the rest.
#
# FIELD OF VIEW — stated so green is never mistaken for more than it checks:
#   - THIS guard watches `find api -type f` and vercel.json's `functions` and `routes` arrays.
#     It does not read the crons (their dests are checked transitively via routes) and does not
#     parse the served files' contents — what each function does is the other guards' job.
#   - `routes[0]` is `{ "handle": "filesystem" }`: every api/** file is ALSO reachable at its own
#     path with no vercel.json line naming it. That is why there is no "orphan file" check — a
#     file no route names is still wired, legitimately. The baseline's reach column records which
#     form each entry takes; moving an entry between forms is a baseline change.
#   - A dest NOT starting /api/ names no function and is not checked.
#
# USAGE
#   .github/scripts/audit-as-entry-points.sh          # verify (CI mode)
#   .github/scripts/audit-as-entry-points.sh --list   # print the current entry set
#   UPDATE_BASELINE=1 .github/scripts/audit-as-entry-points.sh   # rewrite baseline after review
#
# VERCEL_JSON and API_DIR may be overridden to point at fixtures (see
# test-audit-as-entry-points.sh). Under fixtures the baseline diff (a/b) will of course disagree,
# which is why the test harness asserts on the FAIL MESSAGE, not just the exit code.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

API_DIR="${API_DIR:-api}"
VERCEL_JSON="${VERCEL_JSON:-vercel.json}"
GUARD_NAME="audit-as-entry-points"

# Reviewed baseline: <file>\t<reach>\t<role> for every AS entry point. The reach column is how a
# request reaches it (filesystem path, a named vercel.json route, the functions map); the role is
# what it is. A change means a new, removed, or re-wired entry — confirm it, then UPDATE_BASELINE=1.
read -r -d '' BASELINE <<'EOF' || true
api/auth/cli-callback.ts	filesystem path /api/auth/cli-callback	CLI OAuth callback — PKCE code exchange
api/auth/mcp-callback.ts	filesystem path /api/auth/mcp-callback	MCP client OAuth callback
api/axum.rs	functions map; routes catch-all /(.*)	temper-api function — audit-route-auth.sh freezes its route set
api/internal.rs	functions map; routes /internal/(.*) and the cron dests	internal HMAC door (require_internal_signature)
api/mcp.rs	functions map; routes /mcp(.*) and the /.well-known,/oauth catch-alls	MCP door — audit-mcp-route-auth.sh freezes its router
api/oauth/authorization-server.ts	route /.well-known/oauth-authorization-server	RFC 8414 AS discovery
api/oauth/authorize.ts	route /oauth/authorize	authorize endpoint
api/oauth/jwks.ts	route /oauth/jwks	JWKS
api/oauth/saml/acs.ts	route /oauth/saml/acs	SAML ACS
api/oauth/saml/login.ts	route /oauth/saml/login	SAML login
api/oauth/saml/metadata.ts	route /oauth/saml/metadata	SAML metadata
api/oauth/token.ts	route /oauth/token	token endpoint
EOF

# The frozen vercel.json routes array, jq-normalized (-S sorts object keys, -c one line). This is
# the public-path mapping itself: a NEW route line mapping a public path at an ALREADY-REVIEWED
# file is still a new public path, and dest/file checks alone cannot see it; a reorder changes
# first-match semantics. A change here means confirm every src is intentional, then
# UPDATE_BASELINE=1.
read -r -d '' ROUTES_BASELINE <<'ROUTES_EOF' || true
[{"handle":"filesystem"},{"dest":"/api/mcp","src":"/mcp"},{"dest":"/api/mcp","src":"/mcp/(.*)"},{"dest":"/api/internal","src":"/api/embed/dispatch"},{"dest":"/api/internal","src":"/api/embed/warm"},{"dest":"/api/internal","src":"/api/slack/intents/reap"},{"dest":"/api/internal","src":"/api/region/dispatch"},{"dest":"/api/internal","src":"/api/as/reap"},{"dest":"/api/internal","src":"/api/internal-calls/health"},{"dest":"/api/internal","src":"/internal/(.*)"},{"dest":"/api/oauth/authorization-server","src":"/.well-known/oauth-authorization-server"},{"dest":"/api/oauth/authorization-server?issuer_path=$issuer_path","src":"/.well-known/oauth-authorization-server/(?<issuer_path>.*)"},{"dest":"/api/oauth/jwks","src":"/oauth/jwks"},{"dest":"/api/oauth/authorize","src":"/oauth/authorize"},{"dest":"/api/oauth/saml/login","src":"/oauth/saml/login"},{"dest":"/api/oauth/saml/acs","src":"/oauth/saml/acs"},{"dest":"/api/oauth/saml/metadata","src":"/oauth/saml/metadata"},{"dest":"/api/oauth/token","src":"/oauth/token"},{"dest":"/api/mcp","src":"/oauth/(.*)"},{"dest":"/api/mcp","src":"/.well-known/(.*)"},{"dest":"/api/axum","src":"/(.*)"}]
ROUTES_EOF

fail=0

# The entry set: every file Vercel could build a function from.
ENTRY_CURRENT="$(find "$API_DIR" -type f | sort)"
if [[ -z "$ENTRY_CURRENT" ]]; then
  echo "$GUARD_NAME: FAIL — no files found under $API_DIR. The tree moved, or API_DIR is wrong." >&2
  echo "  The guard must be re-read, not re-baselined blind." >&2
  exit 1
fi

# The reference set: every api/** file vercel.json names. routes dests are normalized — query
# suffix stripped, leading /api/ dropped, then resolved to the .rs or .ts file that exists.
resolve_dest() {
  local rest
  rest="$(printf '%s' "$1" | sed 's/?.*$//; s#^/api/##')"
  for cand in "$API_DIR/$rest.rs" "$API_DIR/$rest.ts"; do
    [[ -f "$cand" ]] && { printf '%s\n' "$cand"; return 0; }
  done
  return 1
}

# Bash 3.2 compatible (no mapfile/assoc arrays): keep the parsed lists as newline strings and
# iterate with while-read. `|| true` because grep exits 1 on zero matches under pipefail.
DEST_LIST="$(jq -r '.routes[]? | .dest // empty' "$VERCEL_JSON" | grep '^/api/' | sort -u || true)"
FNKEY_LIST="$(jq -r '.functions // {} | keys[]' "$VERCEL_JSON" | sort -u || true)"

REFERENCE_CURRENT="$(
  {
    printf '%s\n' "$DEST_LIST" | while IFS= read -r d; do [[ -n "$d" ]] && resolve_dest "$d" || true; done
    printf '%s\n' "$FNKEY_LIST"
  } | sort -u
)"

if [[ "${1:-}" == "--list" ]]; then
  echo "# entry files:"
  printf '%s\n' "$ENTRY_CURRENT"
  echo "# named by vercel.json (routes dests + functions keys):"
  printf '%s\n' "$REFERENCE_CURRENT"
  echo "# frozen routes array:"
  jq -S -c '.routes' "$VERCEL_JSON"
  exit 0
fi

# (a/b) The entry set must match the reviewed baseline, both directions.
ROUTES_CURRENT="$(jq -S -c '.routes' "$VERCEL_JSON")"
if [[ "${UPDATE_BASELINE:-}" == "1" ]]; then
  while IFS= read -r f; do
    role="$(printf '%s\n' "$BASELINE" | awk -F'\t' -v f="$f" '$1==f {print $2"\t"$3; found=1} END{if(!found) print "UNREVIEWED\tUNREVIEWED"}')"
    printf '%s\t%s\n' "$f" "$role"
  done <<< "$ENTRY_CURRENT"
  echo "" >&2
  echo "# frozen routes array — copy into ROUTES_BASELINE after confirming every src:" >&2
  printf '%s\n' "$ROUTES_CURRENT" >&2
  echo "^^^ copy into BASELINE after confirming each entry is intentionally public-by-protocol" >&2
  echo "    (or a reviewed server) and the reach column says how a request reaches it." >&2
  exit 0
fi
BASELINE_FILES="$(printf '%s\n' "$BASELINE" | cut -f1 | sort -u)"
DIFF_FILE="$(mktemp)"
trap 'rm -f "$DIFF_FILE"' EXIT
if ! diff <(printf '%s\n' "$BASELINE_FILES") <(printf '%s\n' "$ENTRY_CURRENT") >"$DIFF_FILE" 2>&1; then
  echo "$GUARD_NAME: FAIL — the AS entry-point set changed." >&2
  echo "Every file under api/ is a Vercel function and is publicly reachable (filesystem-first" >&2
  echo "routing in vercel.json). A new one must be reviewed as public-by-protocol or given its" >&2
  echo "own compensating control BEFORE it is reachable, not after." >&2
  echo "diff (baseline -> current):" >&2
  cat "$DIFF_FILE" >&2
  echo "If reviewed and correct: UPDATE_BASELINE=1 .github/scripts/$GUARD_NAME.sh" >&2
  fail=1
fi

# (c) Every /api/ dest must resolve to a file that exists — a route mapping a public path at
# nothing is its own bug, and silently 404s/500s on a public path.
printf '%s\n' "$DEST_LIST" | while IFS= read -r d; do
  [[ -z "$d" ]] && continue
  if ! resolve_dest "$d" >/dev/null; then
    echo "$GUARD_NAME: FAIL — vercel.json routes dest '$d' resolves to no file under $API_DIR" >&2
    echo "  (tried $API_DIR/${d#\/api\/}.rs and .ts, query suffix stripped). A public path now" >&2
    echo "  maps at nothing." >&2
    exit 1
  fi
done || fail=1

# (d) Every functions key must be a file that exists — a renamed or deleted file left in the
# functions map is a deploy-time surprise, not a review.
printf '%s\n' "$FNKEY_LIST" | while IFS= read -r k; do
  [[ -z "$k" ]] && continue
  if [[ ! -f "$k" ]]; then
    echo "$GUARD_NAME: FAIL — vercel.json functions key '$k' is not a file in the tree." >&2
    exit 1
  fi
done || fail=1

# (e) The routes array is frozen whole. A new src naming an already-reviewed file is still a new
# public path; a reorder changes first-match semantics. Neither is visible to (c)/(d).
if [[ "$ROUTES_CURRENT" != "$ROUTES_BASELINE" ]]; then
  echo "$GUARD_NAME: FAIL — the vercel.json routes array changed." >&2
  echo "The routes array IS the public-path mapping. A new line mapping a public path at an" >&2
  echo "already-reviewed file is still a new public path, and reordering changes which route" >&2
  echo "wins. diff (baseline -> current), one element per line:" >&2
  diff <(printf '%s' "$ROUTES_BASELINE" | jq '.[]') <(printf '%s' "$ROUTES_CURRENT" | jq '.[]') >&2 || true
  echo "If reviewed and correct: UPDATE_BASELINE=1 .github/scripts/$GUARD_NAME.sh" >&2
  fail=1
fi

if [[ "$fail" == "0" ]]; then
  echo "$GUARD_NAME: OK — $(printf '%s\n' "$ENTRY_CURRENT" | grep -c .) entry points, all reviewed; $(printf '%s\n' "$DEST_LIST" | grep -c . || true) /api/ dests and $(printf '%s\n' "$FNKEY_LIST" | grep -c . || true) functions keys resolve; routes array frozen."
fi
exit "$fail"
