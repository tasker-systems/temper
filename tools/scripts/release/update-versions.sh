#!/usr/bin/env bash
# tools/scripts/release/update-versions.sh
#
# One writer for every version site in the repo.
#
# EXTEND of tasker-core tools/scripts/release/update-versions.sh per the
# shared-semver-policy spec §5 (spec of record:
# temper-artifacts/specs/2026-09-09-shared-semver-policy-design.md). Sites,
# each verified on disk before wiring (SG-6):
#   VERSION                                    — the shared anchor (CLI stdout rides it)
#   Cargo.toml (root) + crates/*/Cargo.toml    — every workspace [package] version,
#                                                enumerated from the live tree
#                                                (crates NEVER float — spec §3)
#   clients/temper-rb/lib/temper/version.rb    — skin float (--rb)
#   clients/temper-py/temper/version.py        — skin float (--py)
#   clients/temper-ts/package.json             — skin float (--ts)
#   packages/temper-ui/package.json            — npm float (--ui)
#   packages/temper-cloud/package.json         — npm float (--cloud)
#
# openapi.json's info.version is NOT written here — beat 3 derives it from
# VERSION at emit time (spec D-S3). Inter-crate =x.y.z dep pins and registry
# duplicate detection are out of scope for this beat (no publish workflows
# exist; SG-5).
#
# Usage:
#   ./tools/scripts/release/update-versions.sh --core VERSION [--rb VER]
#       [--py VER] [--ts VER] [--ui VER] [--cloud VER] [--dry-run]
#
# --core is always required. Leaf versions are optional and only needed when
# those leaves are being released.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "${SCRIPT_DIR}/lib/common.sh"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
CORE_VERSION=""
RB_VERSION=""
PY_VERSION=""
TS_VERSION=""
UI_VERSION=""
CLOUD_VERSION=""
DRY_RUN=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --core)  CORE_VERSION="$2"; shift 2 ;;
        --core=*) CORE_VERSION="${1#*=}"; shift ;;
        --rb)    RB_VERSION="$2"; shift 2 ;;
        --rb=*)  RB_VERSION="${1#*=}"; shift ;;
        --py)    PY_VERSION="$2"; shift 2 ;;
        --py=*)  PY_VERSION="${1#*=}"; shift ;;
        --ts)    TS_VERSION="$2"; shift 2 ;;
        --ts=*)  TS_VERSION="${1#*=}"; shift ;;
        --ui)    UI_VERSION="$2"; shift 2 ;;
        --ui=*)  UI_VERSION="${1#*=}"; shift ;;
        --cloud) CLOUD_VERSION="$2"; shift 2 ;;
        --cloud=*) CLOUD_VERSION="${1#*=}"; shift ;;
        --dry-run) DRY_RUN=true; shift ;;
        *) die "Unknown argument: $1" ;;
    esac
done

if [[ -z "$CORE_VERSION" ]]; then
    die "Usage: $0 --core VERSION [--rb VER] [--py VER] [--ts VER] [--ui VER] [--cloud VER] [--dry-run]"
fi

# ---------------------------------------------------------------------------
# Shared anchor + workspace crates (the fleet rides VERSION together)
# ---------------------------------------------------------------------------
log_section "Core: VERSION + workspace crates"

update_version_file "$CORE_VERSION"

# The root Cargo.toml is itself a package (temper-cloud, the Vercel adapter
# bin) — distinct from the packages/temper-cloud npm package.
update_cargo_version "Cargo.toml" "$CORE_VERSION"

for crate_toml in "${REPO_ROOT}"/crates/*/Cargo.toml; do
    [[ -f "$crate_toml" ]] || continue
    grep -q '^version = ' "$crate_toml" || continue
    update_cargo_version "$crate_toml" "$CORE_VERSION"
done

# ---------------------------------------------------------------------------
# Client skins (float P above the floor — spec §3)
# ---------------------------------------------------------------------------
if [[ -n "$RB_VERSION" ]]; then
    log_section "temper-rb (client skin)"
    update_ruby_version "$RB_VERSION"
fi

if [[ -n "$PY_VERSION" ]]; then
    log_section "temper-py (client skin)"
    update_python_version "$PY_VERSION"
fi

if [[ -n "$TS_VERSION" ]]; then
    log_section "temper-ts (client skin)"
    update_package_json_version "clients/temper-ts/package.json" "$TS_VERSION"
fi

# ---------------------------------------------------------------------------
# npm packages (float P above the floor — spec §3)
# ---------------------------------------------------------------------------
if [[ -n "$UI_VERSION" ]]; then
    log_section "temper-ui (npm package)"
    update_package_json_version "packages/temper-ui/package.json" "$UI_VERSION"
fi

if [[ -n "$CLOUD_VERSION" ]]; then
    log_section "temper-cloud (npm package)"
    update_package_json_version "packages/temper-cloud/package.json" "$CLOUD_VERSION"
fi
