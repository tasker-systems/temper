#!/usr/bin/env bash
# tools/scripts/release/detect-changes.sh
#
# Detect whether temper-cli or its workspace deps changed since the last
# `v*` tag.
#
# Usage:
#   ./tools/scripts/release/detect-changes.sh [--from TAG]
#
# Output (eval-safe KEY=VALUE):
#   CLI_CHANGED=true|false
#   CHANGES_BASE_REF=<tag|commit>

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "${SCRIPT_DIR}/lib/common.sh"

FROM_REF=""

while [[ $# -gt 0 ]]; do
    case $1 in
        --from) FROM_REF="$2"; shift 2 ;;
        --from=*) FROM_REF="${1#*=}"; shift ;;
        *) die "Unknown argument: $1" ;;
    esac
done

if [[ -n "$FROM_REF" ]]; then
    BASE_REF="$FROM_REF"
elif BASE_REF=$(git describe --tags --match 'v*' --abbrev=0 HEAD 2>/dev/null); then
    :
else
    BASE_REF=$(git rev-list --max-parents=0 HEAD 2>/dev/null | head -n1)
fi

log_info "Comparing HEAD to ${BASE_REF}" >&2

CHANGED_FILES=$(git diff "${BASE_REF}" HEAD --name-only 2>/dev/null || true)

if [[ -z "$CHANGED_FILES" ]]; then
    log_info "No files changed since ${BASE_REF}" >&2
fi

changes_match() {
    local pattern="$1"
    grep -qE "$pattern" <<< "$CHANGED_FILES"
}

CLI_CHANGED=false
if changes_match '^crates/(temper-cli|temper-core|temper-client|temper-ingest)/'; then
    CLI_CHANGED=true
fi

if changes_match '^(scripts/install/|tools/scripts/release/|\.github/workflows/(release|build-cli-binaries|release-tag)\.yml|\.github/scripts/release/)'; then
    CLI_CHANGED=true
fi

echo "CHANGES_BASE_REF=${BASE_REF}"
echo "CLI_CHANGED=${CLI_CHANGED}"
