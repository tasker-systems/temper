#!/usr/bin/env bash
# tools/scripts/release/update-version.sh
#
# Update the VERSION file and temper-cli/Cargo.toml version.
#
# Usage:
#   ./tools/scripts/release/update-version.sh --version 0.1.0 [--dry-run]

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "${SCRIPT_DIR}/lib/common.sh"

VERSION=""
DRY_RUN=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --version)   VERSION="$2"; shift 2 ;;
        --version=*) VERSION="${1#*=}"; shift ;;
        --dry-run)   DRY_RUN=true; shift ;;
        *) die "Unknown argument: $1" ;;
    esac
done

if [[ -z "$VERSION" ]]; then
    die "Usage: $0 --version VERSION [--dry-run]"
fi

export DRY_RUN

log_section "Updating version to ${VERSION}"

update_version_file "$VERSION"
update_cargo_version "crates/temper-cli/Cargo.toml" "$VERSION"
