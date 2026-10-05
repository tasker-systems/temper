#!/usr/bin/env bash
# .github/scripts/check-wire-types-in-core.sh
#
# Guard the "every published wire type lives in temper-core" invariant (ruling 2026-10-04).
#
# temper-core (published as temperkb-core) is the crate-as-public-surface: a client names every
# request and response shape of the published contract from it, without depending on the server
# crates. A type the contract publishes carries utoipa's ToSchema (a body) or IntoParams (a route's
# query parameters), so a derive or impl of either outside temper-core is a published wire type
# living in the wrong crate. This script fails on one.
#
# The one allowed exception is temper-principal: temper-core depends on it, so its three wire types
# (Refusal, Standing, ActorAuthority) cannot move without a cycle, and they already reach every
# client through core.
#
# What this does not cover, stated so nobody reads it as covering more: a type that rides an
# INTERNAL door (a cron, an HMAC-signed call, a third-party webhook; the routes on
# check-openapi-routes.sh's allowlist) carries no utoipa derive, because no published contract
# describes it. Those types stay in temper-api, where only the server needs them (ruling
# 2026-10-05).
#
# Usage:
#   .github/scripts/check-wire-types-in-core.sh [CRATES_DIR]
#
# With no argument it scans crates/. A directory may be passed for testing against fixtures.
#
# Bash 3.2 compatible (macOS default): no assoc arrays, no mapfile.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
CRATES_DIR="${1:-${REPO_ROOT}/crates}"

if [ ! -d "$CRATES_DIR" ]; then
    echo "ERROR: crates directory not found: $CRATES_DIR" >&2
    exit 1
fi

# A derive naming either trait (bare or utoipa::-qualified, directly or inside cfg_attr), or a
# hand-written impl of either.
PATTERN='derive\([^)]*\b(ToSchema|IntoParams)\b|impl[[:space:]]+(<[^>]*>[[:space:]]*)?(utoipa::)?(ToSchema|IntoParams)\b'

OFFENDERS="$(find "$CRATES_DIR" -name '*.rs' \
    -not -path "${CRATES_DIR}/temper-core/*" \
    -not -path "${CRATES_DIR}/temper-principal/*" \
    -not -path '*/target/*' |
    sort |
    while IFS= read -r file; do
        grep -nE "$PATTERN" "$file" | sed "s|^|  ${file#"${REPO_ROOT}/"}:|" || true
    done)"

if [ -n "$OFFENDERS" ]; then
    {
        echo "ERROR: a published wire type is declared outside temper-core:"
        printf '%s\n' "$OFFENDERS"
        echo ""
        echo "ToSchema / IntoParams put a type on the published contract (openapi.json), and every"
        echo "published wire type lives in temper-core so a client can name it without the server"
        echo "crates. Move the type to crates/temper-core/src/types/, deriving the trait behind"
        echo "core's feature: #[cfg_attr(feature = \"web-api\", derive(utoipa::ToSchema))]. The"
        echo "server crate imports it from there (or re-exports it, as temper_substrate::payloads"
        echo "does). If the type rides an internal door no client calls, it needs no utoipa"
        echo "derive at all."
    } >&2
    exit 1
fi

echo "check-wire-types-in-core: every published wire type is declared in temper-core"
