#!/usr/bin/env bash
# .github/scripts/check-openapi-routes.sh
#
# Guard the "OpenAPI spec is a product of the router" invariant.
#
# Every documented route in crates/temper-api/src/routes.rs is mounted via
# `.routes(routes!(handler))`, which registers the axum route AND collects its
# `#[utoipa::path]` into the spec. A route mounted with plain `.route(...)` is
# axum-only: it never enters the OpenAPI contract. That is correct for exactly
# nine operator-only / server-to-server surfaces (the allowlist below) and a bug
# for anything else — an undocumented public route that the emitted openapi.json
# silently omits.
#
# This script fails if routes.rs mounts any plain `.route(` path that is NOT on
# the allowlist. Because `.routes(routes!(…))` is the norm and plain `.route(`
# is the rare, deliberate exception, the check is stable: a NEW plain `.route(`
# is the signal that someone added a route without documenting it.
#
# Usage:
#   .github/scripts/check-openapi-routes.sh [ROUTES_FILE]
#
# ROUTES_FILE defaults to crates/temper-api/src/routes.rs relative to the repo
# root (inferred from this script's location). A path may be passed explicitly
# for testing against fixtures.
#
# Bash 3.2 compatible (macOS default): no assoc arrays, no mapfile.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
ROUTES_FILE="${1:-${REPO_ROOT}/crates/temper-api/src/routes.rs}"

# The operator-only / server-to-server surfaces deliberately mounted with plain
# `.route()` and kept OUT of the OpenAPI contract. Keep in sync with the
# comments in routes.rs (gated_routes / internal_routes / embed_internal_routes).
ALLOWLIST='/api/access/admin/requests
/api/access/admin/requests/{id}
/api/access/admin/settings
/api/access/admin/promote
/api/machine-clients
/api/machine-clients/{id}
/api/machine-clients/{id}/rebind
/api/machine-clients/issue
/api/machine-clients/{id}/rotate-secret
/internal/saml/reconcile
/api/embed/dispatch'

if [ ! -f "$ROUTES_FILE" ]; then
    echo "ERROR: routes file not found: $ROUTES_FILE" >&2
    exit 1
fi

is_allowed() {
    printf '%s\n' "$ALLOWLIST" | grep -qxF "$1"
}

# Extract the path string mounted by each plain `.route(` call. The call may
# span lines (`.route(` then the "path" literal on the next line) or sit inline,
# so we scan forward from `.route(` for the first double-quoted string. `.route(`
# never matches `.routes(` — the `(` must immediately follow `route`.
PATHS="$(awk '
    {
        line = $0
        while (1) {
            if (capturing == 0) {
                idx = index(line, ".route(")
                if (idx == 0) break
                line = substr(line, idx + length(".route("))
                capturing = 1
            }
            if (match(line, /"[^"]*"/)) {
                print substr(line, RSTART + 1, RLENGTH - 2)
                capturing = 0
                line = substr(line, RSTART + RLENGTH)
                continue
            }
            break
        }
    }
' "$ROUTES_FILE")"

OFFENDERS=""
while IFS= read -r path; do
    [ -n "$path" ] || continue
    if ! is_allowed "$path"; then
        OFFENDERS="${OFFENDERS}  ${path}
"
    fi
done <<EOF
$PATHS
EOF

if [ -n "$OFFENDERS" ]; then
    {
        echo "ERROR: undocumented plain .route(...) mount(s) in ${ROUTES_FILE#"${REPO_ROOT}/"}:"
        printf '%s' "$OFFENDERS"
        echo ""
        echo "A plain .route(...) is axum-only — the route never enters the OpenAPI"
        echo "contract (openapi.json). Document it instead: add #[utoipa::path] to the"
        echo "handler and mount it with .routes(routes!(handler))."
        echo ""
        echo "If the route is genuinely operator-only / server-to-server and must stay"
        echo "out of the contract, add its path to the allowlist in this script (and"
        echo "keep the routes.rs comment explaining why)."
    } >&2
    exit 1
fi

echo "check-openapi-routes: all plain .route(...) mounts are on the operator-only allowlist"
