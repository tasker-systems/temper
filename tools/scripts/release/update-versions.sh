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
#   Cargo.toml (root) [workspace.package]      — the anchor every workspace member
#                                                inherits (crates NEVER float — spec §3)
#   Cargo.toml (root) [workspace.dependencies] — the client closure's temperkb-* path+version
#                                                specs (dependency specs cannot inherit the
#                                                package anchor; the crates.io publish
#                                                script's version-agreement guard reads them)
#   clients/temper-rb/lib/temper/version.rb    — skin float (--rb)
#   clients/temper-py/temper/version.py        — skin float (--py)
#   clients/temper-ts/package.json             — skin float (--ts)
#   clients/temper-telemetry-ts/package.json   — skin float (--telemetry)
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
#       [--py VER] [--ts VER] [--telemetry VER] [--ui VER] [--cloud VER] [--dry-run]
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
TELEMETRY_VERSION=""
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
        --telemetry)    TELEMETRY_VERSION="$2"; shift 2 ;;
        --telemetry=*)  TELEMETRY_VERSION="${1#*=}"; shift ;;
        --ui)    UI_VERSION="$2"; shift 2 ;;
        --ui=*)  UI_VERSION="${1#*=}"; shift ;;
        --cloud) CLOUD_VERSION="$2"; shift 2 ;;
        --cloud=*) CLOUD_VERSION="${1#*=}"; shift ;;
        --dry-run) DRY_RUN=true; shift ;;
        *) die "Unknown argument: $1" ;;
    esac
done

if [[ -z "$CORE_VERSION" ]]; then
    die "Usage: $0 --core VERSION [--rb VER] [--py VER] [--ts VER] [--telemetry VER] [--ui VER] [--cloud VER] [--dry-run]"
fi

# ---------------------------------------------------------------------------
# Shared anchor + workspace crates (the fleet rides VERSION together)
# ---------------------------------------------------------------------------
log_section "Core: VERSION + workspace crates"

update_version_file "$CORE_VERSION"

# The root Cargo.toml is itself a package (temper-cloud, the Vercel adapter
# bin) — distinct from the packages/temper-cloud npm package. Since the
# workspace-inheritance consolidation, the root's only line-start `version =`
# is [workspace.package]'s — the anchor every workspace member inherits — so
# update_cargo_version's first-match rewrite lands exactly there, and the
# member manifests carry no version sites of their own at all.
update_cargo_version "Cargo.toml" "$CORE_VERSION"

# The client closure's [workspace.dependencies] entries carry path + version
# (dependency specs cannot inherit [workspace.package]), and a published
# crate's manifest resolves its siblings from the registry — so these specs
# MUST move with the anchor on every bump. The crates.io publish script's
# version-agreement guard fails the release if they drift; a missed site here
# is that guard firing in CI hours later, with the release red. Generic over
# the temperkb- prefix: a seventh published crate needs no edit here.
log_section "Client closure: [workspace.dependencies] version specs"

if ! grep -qE '^temperkb-[a-z]+ = \{ path = ' Cargo.toml; then
    die "No temperkb-* [workspace.dependencies] entries found in Cargo.toml — the closure's version sites moved or vanished; update this writer."
fi

if [[ "${DRY_RUN:-false}" == "true" ]]; then
    while IFS= read -r line; do
        dep="$(printf '%s' "$line" | sed -E 's/^(temperkb-[a-z]+) = \{.*/\1/')"
        current="$(printf '%s' "$line" | sed -E 's/^temperkb-[a-z]+ = \{ path = "[^"]+", version = "([^"]+)".*/\1/')"
        log_info "Would update ${dep} workspace-dependency spec: ${current} -> ${CORE_VERSION}"
    done < <(grep -E '^temperkb-[a-z]+ = \{ path = ' Cargo.toml)
else
    sed_i -E 's/^(temperkb-[a-z]+ = \{ path = "[^"]+", version = ")[^"]+(" \})/\1'"${CORE_VERSION}"'\2/' Cargo.toml
    log_info "Updated temperkb-* workspace-dependency specs -> ${CORE_VERSION}"
fi

# Verify the whole Rust half reads back as the requested version (SG-6): the
# anchor, zero stale spec entries. Counting sites, not trusting the rewrite
# to have matched everything: an uncounted miss is exactly the silent drift
# the guard would catch later, in CI, with the release red.
ws_version="$(awk -F'"' '/^\[workspace\.package\]/{p=1; next} /^\[/{p=0} p && /^version = /{print $2; exit}' Cargo.toml)"
if [[ "${DRY_RUN:-false}" != "true" ]]; then
    [[ "$ws_version" == "$CORE_VERSION" ]] ||
        die "workspace anchor is ${ws_version}, expected ${CORE_VERSION}"
    spec_total="$(grep -cE '^temperkb-[a-z]+ = \{ path = ' Cargo.toml)"
    stale="$(awk -F'"' -v want="${CORE_VERSION}" '/^temperkb-[a-z]+ = \{ path = / && $4 != want { print "  " $1 " -> " $4 }' Cargo.toml)"
    if [[ -n "$stale" ]]; then
        die "stale temperkb-* [workspace.dependencies] version sites after the bump (${spec_total} sites total):
${stale}"
    fi
    log_info "Verified: workspace anchor + ${spec_total} closure specs at ${CORE_VERSION}"
fi

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

if [[ -n "$TELEMETRY_VERSION" ]]; then
    log_section "temper-telemetry-ts (client skin)"
    update_package_json_version "clients/temper-telemetry-ts/package.json" "$TELEMETRY_VERSION"
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
