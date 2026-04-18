#!/usr/bin/env bash
# tools/scripts/release/calculate-version.sh
#
# Calculate the next temper-cli version.
#
# Usage:
#   ./tools/scripts/release/calculate-version.sh [--bump patch|minor|major] [--from TAG]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "${SCRIPT_DIR}/lib/common.sh"

BUMP=""
DETECT_ARGS=()

while [[ $# -gt 0 ]]; do
    case $1 in
        --bump) BUMP="$2"; shift 2 ;;
        --bump=*) BUMP="${1#*=}"; shift ;;
        --from) DETECT_ARGS+=(--from "$2"); shift 2 ;;
        --from=*) DETECT_ARGS+=(--from "${1#*=}"); shift ;;
        *) die "Unknown argument: $1" ;;
    esac
done

# shellcheck disable=SC2046
eval "$("${SCRIPT_DIR}/detect-changes.sh" ${DETECT_ARGS[@]+"${DETECT_ARGS[@]}"})"

eval "$("${SCRIPT_DIR}/read-version.sh")"
CURRENT_VERSION="$VERSION"
echo "CURRENT_VERSION=${CURRENT_VERSION}"

if [[ "$CLI_CHANGED" != "true" ]]; then
    NEXT_VERSION="$CURRENT_VERSION"
else
    case "$BUMP" in
        major) NEXT_VERSION=$(bump_major "$CURRENT_VERSION") ;;
        minor) NEXT_VERSION=$(bump_minor "$CURRENT_VERSION") ;;
        patch|"") NEXT_VERSION=$(bump_patch "$CURRENT_VERSION") ;;
        *) die "Unknown --bump level: $BUMP (expected patch|minor|major)" ;;
    esac
fi
echo "NEXT_VERSION=${NEXT_VERSION}"

echo "CHANGES_BASE_REF=${CHANGES_BASE_REF}"
echo "CLI_CHANGED=${CLI_CHANGED}"
