#!/usr/bin/env bash
# tools/scripts/release/lib/common.sh
# Shared functions for Temper release tooling.
#
# Source this from other release scripts:
#   source "$(dirname "$0")/lib/common.sh"
#
# Expects callers to set DRY_RUN=true|false before calling file-update functions.

set -euo pipefail

# Resolve repo root relative to this file (lib/ -> release/ -> scripts/ -> tools/ -> repo root)
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"

# ---------------------------------------------------------------------------
# Logging
# ---------------------------------------------------------------------------
log_info()    { echo "  [info] $*"; }
log_warn()    { echo "  [warn] $*" >&2; }
log_error()   { echo "  [error] $*" >&2; }
log_header()  { echo ""; echo "== $* =="; echo ""; }
log_section() { echo ""; echo "-- $* --"; }

die() { log_error "$*"; exit 1; }

confirm() {
    read -p "  $1 (y/N) " -n 1 -r
    echo
    [[ $REPLY =~ ^[Yy]$ ]] || exit 1
}

# ---------------------------------------------------------------------------
# Portable sed -i (GNU vs BSD/macOS)
# ---------------------------------------------------------------------------
sed_i() {
    if sed --version 2>/dev/null | grep -q 'GNU'; then
        sed -i "$@"
    else
        sed -i '' "$@"
    fi
}

# ---------------------------------------------------------------------------
# Version arithmetic
# ---------------------------------------------------------------------------

bump_patch() {
    local version="$1"
    local major minor patch
    IFS='.' read -r major minor patch <<< "$version"
    echo "${major}.${minor}.$((patch + 1))"
}

bump_minor() {
    local version="$1"
    local major minor _patch
    IFS='.' read -r major minor _patch <<< "$version"
    echo "${major}.$((minor + 1)).0"
}

bump_major() {
    local version="$1"
    local major _minor _patch
    IFS='.' read -r major _minor _patch <<< "$version"
    echo "$((major + 1)).0.0"
}

# ---------------------------------------------------------------------------
# File update helpers (all respect DRY_RUN from caller scope)
# ---------------------------------------------------------------------------

update_version_file() {
    local version="$1"
    local file="${REPO_ROOT}/VERSION"
    if [[ "${DRY_RUN:-false}" == "true" ]]; then
        log_info "Would update VERSION -> $version"
    else
        echo "$version" > "$file"
        log_info "Updated VERSION -> $version"
    fi
}

update_cargo_version() {
    local file="$1" version="$2"

    [[ "$file" != /* ]] && file="${REPO_ROOT}/${file}"

    if [[ ! -f "$file" ]]; then
        log_warn "File not found: $file"
        return
    fi

    if [[ "${DRY_RUN:-false}" == "true" ]]; then
        local current
        current=$(grep -m1 '^version = ' "$file" | sed 's/version = "\(.*\)"/\1/')
        log_info "Would update $file version: $current -> $version"
    else
        local line_num
        line_num=$(grep -n -m1 '^version = ' "$file" | cut -d: -f1)
        if [[ -n "$line_num" ]]; then
            sed_i "${line_num}s/^version = \".*\"/version = \"${version}\"/" "$file"
        fi
        log_info "Updated $file -> $version"
    fi
}
