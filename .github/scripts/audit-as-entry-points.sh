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
#   (e) vercel.json is frozen WHOLE, jq-normalized. Freezing only the routes array left the other
#       reachability channels unread: `rewrites`, `redirects`, `headers`, `cleanUrls`,
#       `trailingSlash`, `proxy` can each map or alter a public path, and `crons` vends
#       scheduled unauthenticated GETs. The whole file IS the deployment mapping — any change,
#       including a reorder (first match wins) or a benign-looking build tweak, gets human eyes.
#   (f) the three api/*.rs BINS carry no router-assembly OR route-declaration tokens. They are
#       workspace binaries a normal-looking edit could turn into second assembly sites — or into
#       a `.route(` appended to the crate-built router — one file boundary away from every other
#       guard's field of view. (Doc comments are stripped first — prose may say "Router". The
#       bin strip is `//`-to-EOL, and the bins carry no block comments today. Whitespace before
#       a call paren is not a shape these greps read — that class is held by `cargo fmt --all
#       -- --check`, which runs earlier in the same CI job.) What the bins do BEYOND assembly
#       stays with the crates' guards.
#   (g) no sibling Vercel config (vercel.toml / vercel.ts) may exist: only one config file is
#       honored per project, and this guard freezes only vercel.json.
#
# FIELD OF VIEW — stated so green is never mistaken for more than it checks:
#   - THIS guard watches `find api -type f`, vercel.json's parsed content, and the three bins'
#     text for `Router::` assembly tokens only. It does not parse handler logic, and it does not
#     watch temper-api's route set (audit-route-auth.sh) or temper-mcp's router
#     (audit-mcp-route-auth.sh).
#   - `routes[0]` is `{ "handle": "filesystem" }`: every api/** file is ALSO reachable at its own
#     path with no vercel.json line naming it. That is why there is no "orphan file" check — a
#     file no route names is still wired, legitimately. The baseline's reach column records which
#     form each entry takes; moving an entry between forms is a baseline change.
#   - A dest NOT starting /api/ names no function and is not checked.
#
# USAGE
#   .github/scripts/audit-as-entry-points.sh          # verify (CI mode)
#   .github/scripts/audit-as-entry-points.sh --list   # print the current entry set and mapping
#   UPDATE_BASELINE=1 .github/scripts/audit-as-entry-points.sh   # print baseline after review
#
# UPDATE_BASELINE=1 rewrites nothing and only prints the current sets, and refuses to run at all
# in CI or while any check above is failing — update mode cannot launder an unresolved failure.
#
# API_DIR and VERCEL_JSON may be overridden to point at fixtures (see
# test-audit-as-entry-points.sh). Under fixtures the baseline diffs (a/b/e) will of course
# disagree, which is why the test harness asserts on the FAIL MESSAGE, not just the exit code.

set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

API_DIR="${API_DIR:-api}"
VERCEL_JSON="${VERCEL_JSON:-vercel.json}"
GUARD_NAME="audit-as-entry-points"

# The three Vercel bins — the files vercel.json's `functions` map configures. None may assemble
# its own router (f); the routers live in the crates, whose guards freeze them. API_BINS_OVERRIDE
# exists for the test harness (fixtures live outside api/); CI never sets it. A SET-BUT-EMPTY
# override is refused before the default expansion can mask it: silently checking zero bins while
# reporting "bins assembly-free" is the failure this guard exists to prevent.
if [[ -n "${API_BINS_OVERRIDE+x}" && -z "$API_BINS_OVERRIDE" ]]; then
  echo "$GUARD_NAME: FAIL — API_BINS_OVERRIDE is set but empty; refusing to check zero bins." >&2
  exit 1
fi
API_BINS_OVERRIDE="${API_BINS_OVERRIDE:-api/mcp.rs api/axum.rs api/internal.rs}"
read -r -a API_BINS <<< "$API_BINS_OVERRIDE"

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

# The frozen vercel.json, jq-normalized (-S sorts keys, -c one line). This is the deployment
# mapping ENTIRE — routes, crons, functions config, and every future reachability key. A change
# here means confirm it with human eyes, then UPDATE_BASELINE=1.
read -r -d '' VERCEL_BASELINE <<'VERCEL_EOF' || true
{"$schema":"https://openapi.vercel.sh/vercel.json","build":{"env":{"SQLX_OFFLINE":"true"}},"buildCommand":"sh scripts/vercel-build.sh","crons":[{"path":"/api/embed/dispatch?shard=0","schedule":"* * * * *"},{"path":"/api/embed/dispatch?shard=1","schedule":"* * * * *"},{"path":"/api/embed/dispatch?shard=2","schedule":"* * * * *"},{"path":"/api/embed/dispatch?shard=3","schedule":"* * * * *"},{"path":"/api/embed/warm","schedule":"*/2 * * * *"},{"path":"/api/slack/intents/reap","schedule":"0 * * * *"},{"path":"/api/region/dispatch","schedule":"* * * * *"},{"path":"/api/as/reap","schedule":"17 3 * * *"},{"path":"/api/internal-calls/health","schedule":"*/15 * * * *"}],"framework":null,"functions":{"api/axum.rs":{"maxDuration":60,"memory":3009},"api/internal.rs":{"maxDuration":300,"memory":3009},"api/mcp.rs":{"maxDuration":60,"memory":3009}},"ignoreCommand":"sh \"$(git rev-parse --show-toplevel)/scripts/vercel-ignore-build.sh\" temper-cloud","installCommand":"cd packages/temper-cloud && bun install","routes":[{"handle":"filesystem"},{"dest":"/api/mcp","src":"/mcp"},{"dest":"/api/mcp","src":"/mcp/(.*)"},{"dest":"/api/internal","src":"/api/embed/dispatch"},{"dest":"/api/internal","src":"/api/embed/warm"},{"dest":"/api/internal","src":"/api/slack/intents/reap"},{"dest":"/api/internal","src":"/api/region/dispatch"},{"dest":"/api/internal","src":"/api/as/reap"},{"dest":"/api/internal","src":"/api/internal-calls/health"},{"dest":"/api/internal","src":"/internal/(.*)"},{"dest":"/api/oauth/authorization-server","src":"/.well-known/oauth-authorization-server"},{"dest":"/api/oauth/authorization-server?issuer_path=$issuer_path","src":"/.well-known/oauth-authorization-server/(?<issuer_path>.*)"},{"dest":"/api/oauth/jwks","src":"/oauth/jwks"},{"dest":"/api/oauth/authorize","src":"/oauth/authorize"},{"dest":"/api/oauth/saml/login","src":"/oauth/saml/login"},{"dest":"/api/oauth/saml/acs","src":"/oauth/saml/acs"},{"dest":"/api/oauth/saml/metadata","src":"/oauth/saml/metadata"},{"dest":"/api/oauth/token","src":"/oauth/token"},{"dest":"/api/mcp","src":"/oauth/(.*)"},{"dest":"/api/mcp","src":"/.well-known/(.*)"},{"dest":"/api/axum","src":"/(.*)"}]}
VERCEL_EOF

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
VERCEL_CURRENT="$(jq -S -c '.' "$VERCEL_JSON")"

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
  echo "# frozen vercel.json:"
  printf '%s\n' "$VERCEL_CURRENT"
  exit 0
fi

# (a/b) The entry set must match the reviewed baseline, both directions.
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

# (e) vercel.json is frozen whole. A new src naming an already-reviewed file is still a new
# public path; a reorder changes first-match semantics; and `rewrites`, `redirects`, `headers`,
# `cleanUrls`, `trailingSlash`, `proxy` and `crons` are reachability channels the routes array
# alone never froze.
if [[ "$VERCEL_CURRENT" != "$VERCEL_BASELINE" ]]; then
  echo "$GUARD_NAME: FAIL — vercel.json changed." >&2
  echo "The whole file is frozen: it IS the deployment mapping. routes/reorders/rewrites/" >&2
  echo "redirects/headers/crons are all reachability channels; even a build tweak gets human" >&2
  echo "eyes here. diff (baseline -> current):" >&2
  diff <(printf '%s' "$VERCEL_BASELINE" | jq 'del(.routes)' ) <(printf '%s' "$VERCEL_CURRENT" | jq 'del(.routes)') >&2 || true
  diff <(printf '%s' "$VERCEL_BASELINE" | jq -c '.routes[]') <(printf '%s' "$VERCEL_CURRENT" | jq -c '.routes[]') >&2 || true
  echo "If reviewed and correct: UPDATE_BASELINE=1 .github/scripts/$GUARD_NAME.sh" >&2
  fail=1
fi

# (f) The api bins must not assemble OR extend a router — a second assembly site one file
# boundary outside every crate guard's field of view, or a `.route(` appended to the crate-built
# router before Vercel wrapping (reachable via the /mcp(.*) mapping without any baseline entry).
# Comments stripped: prose may say "Router". (The set-but-empty override refusal lives at the
# top of the script, before the default expansion would mask it.)
for bin in "${API_BINS[@]}"; do
  if [[ -f "$bin" ]] && sed 's#//.*##' "$bin" | grep -qE 'Router::|\.route\(|\.nest\(|\.merge\(|\.nest_service\('; then
    echo "$GUARD_NAME: FAIL — router assembly or route-declaration token in $bin." >&2
    echo "  The Vercel bins are entry points, not assembly sites; the routers live in the crates," >&2
    echo "  whose guards freeze them. Route through temper_mcp::build_router / temper_api's" >&2
    echo "  create_app, or extend this guard deliberately." >&2
    fail=1
  fi
done

# (g) Exactly ONE Vercel configuration file may exist per project, and this guard freezes only
# vercel.json. A sibling config would supersede the frozen mapping without tripping anything.
for alt in vercel.toml vercel.ts; do
  if [[ -f "$alt" ]]; then
    echo "$GUARD_NAME: FAIL — $alt exists. Only one Vercel config file is honored per project," >&2
    echo "  and this guard freezes only vercel.json; a sibling config can supersede it. Remove" >&2
    echo "  the file or fold this guard's freeze onto it deliberately." >&2
    fail=1
  fi
done

# Update mode runs LAST and refuses to launder: not in CI, and not while any check is failing.
if [[ -n "${UPDATE_BASELINE:-}" ]]; then
  if [[ -n "${CI:-}" ]]; then
    echo "$GUARD_NAME: UPDATE_BASELINE is not available in CI — re-baseline locally after review." >&2
    exit 1
  fi
  if [[ "$fail" != "0" ]]; then
    echo "$GUARD_NAME: UPDATE_BASELINE refused — resolve the failures above first; update mode" >&2
    echo "  cannot launder an unresolved failure." >&2
    exit 1
  fi
  while IFS= read -r f; do
    role="$(printf '%s\n' "$BASELINE" | awk -F'\t' -v f="$f" '$1==f {print $2"\t"$3; found=1} END{if(!found) print "UNREVIEWED\tUNREVIEWED"}')"
    printf '%s\t%s\n' "$f" "$role"
  done <<< "$ENTRY_CURRENT"
  echo "" >&2
  echo "# frozen vercel.json — copy into VERCEL_BASELINE after confirming every line:" >&2
  printf '%s\n' "$VERCEL_CURRENT" >&2
  echo "^^^ copy into BASELINE after confirming each entry is intentionally public-by-protocol" >&2
  echo "    (or a reviewed server) and the reach column says how a request reaches it." >&2
  exit 0
fi

if [[ "$fail" == "0" ]]; then
  echo "$GUARD_NAME: OK — $(printf '%s\n' "$ENTRY_CURRENT" | grep -c .) entry points, all reviewed; $(printf '%s\n' "$DEST_LIST" | grep -c . || true) /api/ dests and $(printf '%s\n' "$FNKEY_LIST" | grep -c . || true) functions keys resolve; vercel.json frozen; bins assembly-free."
fi
exit "$fail"
