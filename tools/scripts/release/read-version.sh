#!/usr/bin/env bash
# tools/scripts/release/read-version.sh
#
# Read committed VERSION from the repo root.
#
# Output (suitable for eval and >> $GITHUB_OUTPUT):
#   VERSION=0.1.0

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"

VERSION_FILE="${REPO_ROOT}/VERSION"
if [[ ! -f "$VERSION_FILE" ]]; then
    echo "ERROR: VERSION file not found at ${VERSION_FILE}" >&2
    exit 1
fi

VERSION=$(tr -d '[:space:]' < "$VERSION_FILE")
echo "VERSION=${VERSION}"
