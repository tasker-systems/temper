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

# Refuse anything that is not a semver version before it is written anywhere. The value becomes a
# step output, a tag name, and an argument in a job that pushes a tag; whitespace stripping alone
# would pass `0.6.0$(id)` straight through. Strict on purpose: a VERSION this rejects is wrong.
SEMVER='^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?(\+[0-9A-Za-z.-]+)?$'
if [[ ! "$VERSION" =~ $SEMVER ]]; then
    echo "ERROR: VERSION '${VERSION}' is not a semver version (MAJOR.MINOR.PATCH[-pre][+build])" >&2
    exit 1
fi

echo "VERSION=${VERSION}"
