#!/usr/bin/env bash
# tools/scripts/release/calculate-versions.sh
#
# Calculate the next version for the shared anchor and every releasable leaf,
# gated by the release-verdict register.
#
# EXTEND of tasker-core tools/scripts/release/calculate-versions.sh per the
# shared-semver-policy spec §5 (spec of record:
# temper-artifacts/specs/2026-09-09-shared-semver-policy-design.md). The temper
# deltas the spec names: the declared classes since the last release are an
# INPUT read from the register rows landing in the window (the register is in
# the tree, so the calculator can read it — PR bodies are not); any
# shape-breaking row forces the M question; behavioral rows change no number
# but must be absent-or-satisfied for the surfaces being released; leaves
# (client skins, npm packages) float with the max(bump, floor) rule.
#
# Semantics (spec §2, §3):
#   - Default next core = bump_patch(VERSION) when any bump-forcing class
#     changed (CORE / WIRE / CLIENTS / PACKAGES). SCHEMA and INFRA never force
#     a move — orchestrator ruling, recorded in detect-changes.sh.
#   - Any shape-breaking row in the window REQUIRES --minor (exit non-zero
#     otherwise, naming the rows): M is always a decision, never an accident —
#     the refusal is the forcing function. --minor hand-raises M (0.4.x -> 0.5.0).
#   - Open register rows whose surfaces intersect the surfaces being released
#     print the release-checklist warning (spec §4.1).
#   - Blocked register rows print their blocked release-class always; they
#     REFUSE a release that includes client skins (--rb/--py/--ts) unless
#     --override-blocker is passed explicitly. (The calculator can mechanically
#     identify a client-skin release and nothing finer; a blocked row naming a
#     different release class is surfaced as a warning for the human checklist.)
#   - Leaf float: --rb/--py/--ts/--telemetry/--ui/--cloud name the leaves being
#     released; each leaf version = max(bump_patch(current), next core) — never
#     below the floor; when the floor's M moves, a leaf below it re-bases and
#     its P count restarts. Crates NEVER float (spec §3 operative text).
#
# Usage:
#   ./tools/scripts/release/calculate-versions.sh [--minor] [--override-blocker]
#       [--register PATH] [--rb] [--py] [--ts] [--telemetry] [--ui] [--cloud] [--from TAG]
#
# Output (eval-safe KEY=VALUE on stdout; warnings and refusals on stderr):
#   CHANGES_BASE_REF=... and the detect-changes class booleans (re-emitted)
#   CURRENT_CORE_VERSION=0.4.0        NEXT_CORE_VERSION=0.4.1|0.5.0
#   CURRENT_RB_VERSION=0.1.0          NEXT_RB_VERSION=0.4.1|unchanged
#   CURRENT_PY_VERSION=...            NEXT_PY_VERSION=...
#   CURRENT_TS_VERSION=...            NEXT_TS_VERSION=...
#   CURRENT_TELEMETRY_VERSION=...     NEXT_TELEMETRY_VERSION=...
#   CURRENT_UI_VERSION=...            NEXT_UI_VERSION=...
#   CURRENT_CLOUD_VERSION=...         NEXT_CLOUD_VERSION=...
#   REGISTER_WINDOW=v0.4.0            REGISTER_ROWS=10
#   REGISTER_ADDITIVE=1  REGISTER_SHAPE_BREAKING=0  REGISTER_BEHAVIORAL=9
#   REGISTER_OPEN=7  REGISTER_SIGNAL_ONLY=2  REGISTER_BLOCKED=1
#   REGISTER_BLOCKED_CLASSES="..."

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "${SCRIPT_DIR}/lib/common.sh"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
MINOR=false
OVERRIDE_BLOCKER=false
REGISTER_FILE="${REPO_ROOT}/RELEASE_REGISTER.md"
RB_RELEASE=false
PY_RELEASE=false
TS_RELEASE=false
TELEMETRY_RELEASE=false
UI_RELEASE=false
CLOUD_RELEASE=false
DETECT_ARGS=()

while [[ $# -gt 0 ]]; do
    case $1 in
        --minor)            MINOR=true; shift ;;
        --override-blocker) OVERRIDE_BLOCKER=true; shift ;;
        --register)         REGISTER_FILE="$2"; shift 2 ;;
        --register=*)       REGISTER_FILE="${1#*=}"; shift ;;
        --rb)               RB_RELEASE=true; shift ;;
        --py)               PY_RELEASE=true; shift ;;
        --ts)               TS_RELEASE=true; shift ;;
        --telemetry)        TELEMETRY_RELEASE=true; shift ;;
        --ui)               UI_RELEASE=true; shift ;;
        --cloud)            CLOUD_RELEASE=true; shift ;;
        --from)             DETECT_ARGS+=(--from "$2"); shift 2 ;;
        --from=*)           DETECT_ARGS+=(--from "${1#*=}"); shift ;;
        *) die "Unknown argument: $1" ;;
    esac
done

# ---------------------------------------------------------------------------
# Path-class detection (only --from is meaningful to detect-changes)
# ---------------------------------------------------------------------------
# Capture, don't eval: a detect-changes death must stop this script, and
# `eval "$(dying-cmd)"` would swallow the non-zero exit (the same trap
# release-prepare.sh defends against).
DETECT_OUTPUT="$("${SCRIPT_DIR}/detect-changes.sh" ${DETECT_ARGS[@]+"${DETECT_ARGS[@]}"})" || die "detect-changes.sh failed — refusing to compute versions from an unreadable window."
# shellcheck disable=SC2046
eval "$DETECT_OUTPUT"

# ---------------------------------------------------------------------------
# Read current core version (the single anchor — spec §3)
# ---------------------------------------------------------------------------
VERSION_FILE="${REPO_ROOT}/VERSION"
if [[ ! -f "$VERSION_FILE" ]]; then
    die "VERSION file not found at ${VERSION_FILE}"
fi

CURRENT_CORE_VERSION=$(tr -d '[:space:]' < "$VERSION_FILE")
echo "CURRENT_CORE_VERSION=${CURRENT_CORE_VERSION}"

# ---------------------------------------------------------------------------
# Read the register window
# ---------------------------------------------------------------------------
REGISTER_ROWS_FILE="$(mktemp)"
trap 'rm -f "$REGISTER_ROWS_FILE"' EXIT
register_window_rows "$REGISTER_FILE" > "$REGISTER_ROWS_FILE"

REGISTER_WINDOW=""
if [[ -f "$REGISTER_FILE" ]]; then
    REGISTER_WINDOW=$(awk '/^## Since /{print $3; exit}' "$REGISTER_FILE")
fi

# ---------------------------------------------------------------------------
# Register gates
# ---------------------------------------------------------------------------
SHAPE_BREAKING_ROWS=0
BLOCKED_ROWS=0
OPEN_ROWS=0
SIGNAL_ONLY_ROWS=0
ADDITIVE_ROWS=0
BEHAVIORAL_ROWS=0
TOTAL_ROWS=0
BLOCKED_CLASSES=""

while IFS=$'\t' read -r r_pr r_classes r_surfaces r_status r_citation; do
    [[ -n "$r_citation" ]] || continue
    TOTAL_ROWS=$((TOTAL_ROWS + 1))

    if field_has_token "$r_classes" "additive"; then
        ADDITIVE_ROWS=$((ADDITIVE_ROWS + 1))
    fi
    if field_has_token "$r_classes" "behavioral"; then
        BEHAVIORAL_ROWS=$((BEHAVIORAL_ROWS + 1))
    fi
    if field_has_token "$r_classes" "shape-breaking"; then
        SHAPE_BREAKING_ROWS=$((SHAPE_BREAKING_ROWS + 1))
    fi

    case "$r_status" in
        open)
            OPEN_ROWS=$((OPEN_ROWS + 1))
            ;;
        signal-only)
            SIGNAL_ONLY_ROWS=$((SIGNAL_ONLY_ROWS + 1))
            ;;
        blocked:*)
            BLOCKED_ROWS=$((BLOCKED_ROWS + 1))
            blocked_class="${r_status#blocked:}"
            BLOCKED_CLASSES+="${blocked_class}; "
            log_warn "Register blocker in window: '${blocked_class}' (surfaces: ${r_surfaces})"
            log_warn "  ${r_citation}"
            ;;
    esac
done < "$REGISTER_ROWS_FILE"

echo "REGISTER_WINDOW=${REGISTER_WINDOW}"
echo "REGISTER_ROWS=${TOTAL_ROWS}"
echo "REGISTER_ADDITIVE=${ADDITIVE_ROWS}"
echo "REGISTER_SHAPE_BREAKING=${SHAPE_BREAKING_ROWS}"
echo "REGISTER_BEHAVIORAL=${BEHAVIORAL_ROWS}"
echo "REGISTER_OPEN=${OPEN_ROWS}"
echo "REGISTER_SIGNAL_ONLY=${SIGNAL_ONLY_ROWS}"
echo "REGISTER_BLOCKED=${BLOCKED_ROWS}"
echo "REGISTER_BLOCKED_CLASSES=\"${BLOCKED_CLASSES%; }\""

# Gate 1: a shape-breaking row in the window forces the M question.
if [[ "$SHAPE_BREAKING_ROWS" -gt 0 && "$MINOR" != "true" ]]; then
    log_error "Shape-breaking rows in the release window require --minor (M is a decision, never an accident — spec §2):"
    while IFS=$'\t' read -r r_pr r_classes r_surfaces r_status r_citation; do
        if field_has_token "$r_classes" "shape-breaking"; then
            log_error "  pr:${r_pr} — ${r_citation}"
        fi
    done < "$REGISTER_ROWS_FILE"
    die "Refusing to compute a patch next for a window containing shape-breaking rows. Re-run with --minor to hand-raise M."
fi

# Gate 2: blocked rows refuse a release that includes any client leaf
# (skins and npm packages alike — spec §3 counts them all as client leaves;
# §8: "No release exposing block-grain annotations ... can be snuck past it").
if [[ "$BLOCKED_ROWS" -gt 0 ]]; then
    if [[ "$RB_RELEASE" == "true" || "$PY_RELEASE" == "true" || "$TS_RELEASE" == "true" || "$TELEMETRY_RELEASE" == "true" || "$UI_RELEASE" == "true" || "$CLOUD_RELEASE" == "true" ]]; then
        if [[ "$OVERRIDE_BLOCKER" != "true" ]]; then
            log_error "This release includes client skins and the register carries a blocked release-class:"
            log_error "  ${BLOCKED_CLASSES%; }"
            die "Refusing. The release checklist verdict on that class is owed (spec §4.1, §8) — pass --override-blocker only with the verdict owner's decision."
        else
            log_warn "BLOCKED ROW OVERRIDDEN (--override-blocker): ${BLOCKED_CLASSES%; }"
        fi
    fi
fi

# ---------------------------------------------------------------------------
# Surfaces being released (for the open-row checklist warning)
# ---------------------------------------------------------------------------
RELEASE_SURFACES=""
if [[ "$CORE_CHANGED" == "true" || "$WIRE_CHANGED" == "true" || "$CLIENTS_CHANGED" == "true" || "$PACKAGES_CHANGED" == "true" ]]; then
    # The crates ride VERSION together; any bump-forcing release moves
    # http, mcp, cli-stdout with it, and internal rows gate every release.
    RELEASE_SURFACES="http mcp cli-stdout internal"
fi
if [[ "$CLIENTS_CHANGED" == "true" || "$RB_RELEASE" == "true" || "$PY_RELEASE" == "true" || "$TS_RELEASE" == "true" || "$TELEMETRY_RELEASE" == "true" || "$UI_RELEASE" == "true" || "$CLOUD_RELEASE" == "true" ]]; then
    RELEASE_SURFACES+=" clients"
fi
if [[ "$SCHEMA_CHANGED" == "true" ]]; then
    RELEASE_SURFACES+=" schema"
fi

# Gate 3: open rows intersecting the released surfaces print the checklist
# warning (the checklist consults the register — spec §4.1).
OPEN_WARNING_EMITTED=false
while IFS=$'\t' read -r r_pr r_classes r_surfaces r_status r_citation; do
    [[ "$r_status" == "open" ]] || continue
    if surfaces_intersect "$r_surfaces" "$RELEASE_SURFACES"; then
        log_warn "Register open row gates this release (pr:${r_pr}, surfaces: ${r_surfaces}):"
        log_warn "  ${r_citation}"
        OPEN_WARNING_EMITTED=true
    fi
done < "$REGISTER_ROWS_FILE"
if [[ "$OPEN_WARNING_EMITTED" == "true" ]]; then
    log_warn "Release checklist: the open rows above must be carried and marked satisfied, or this release waits (spec §4.1)."
fi

# ---------------------------------------------------------------------------
# Next core version
# ---------------------------------------------------------------------------
FORCING=false
if [[ "$CORE_CHANGED" == "true" || "$WIRE_CHANGED" == "true" || "$CLIENTS_CHANGED" == "true" || "$PACKAGES_CHANGED" == "true" ]]; then
    FORCING=true
fi

if [[ "$MINOR" == "true" ]]; then
    NEXT_CORE_VERSION=$(bump_minor "$CURRENT_CORE_VERSION")
elif [[ "$FORCING" == "true" ]]; then
    NEXT_CORE_VERSION=$(bump_patch "$CURRENT_CORE_VERSION")
else
    NEXT_CORE_VERSION="$CURRENT_CORE_VERSION"
fi
echo "NEXT_CORE_VERSION=${NEXT_CORE_VERSION}"

# ---------------------------------------------------------------------------
# Leaf float: max(bump_patch(current), next core) — never below the floor
# (spec §3). Unnamed leaves are unchanged; releasing is an act, merging is
# not (spec §9).
# ---------------------------------------------------------------------------
read_leaf_version() {
    local file="$1" pattern="$2"
    if [[ ! -f "${REPO_ROOT}/${file}" ]]; then
        log_warn "Leaf version file not found: ${file}"
        echo ""
        return
    fi
    # A pattern that matches nothing must be loud, not an empty read: the gem's
    # version line is indented (`  VERSION = `), so the anchor admits leading
    # whitespace without widening to `CONTRACT_VERSION = `.
    local matched
    matched=$(grep -m1 -E "$pattern" "${REPO_ROOT}/${file}") || die "Leaf version pattern '${pattern}' matched nothing in ${file} — refusing to compute from an unreadable version site."
    sed -E "s/.*['\"]([0-9]+\.[0-9]+\.[0-9]+)['\"].*/\1/" <<< "$matched"
}

leaf_next() {
    local current="$1"
    if [[ -z "$current" ]]; then
        echo "unchanged"
        return
    fi
    local bumped
    bumped=$(bump_patch "$current")
    if semver_ge "$bumped" "$NEXT_CORE_VERSION"; then
        echo "$bumped"
    else
        echo "$NEXT_CORE_VERSION"
    fi
}

emit_leaf() {
    local name="$1" file="$2" pattern="$3" releasing="$4"
    local current
    current=$(read_leaf_version "$file" "$pattern")
    echo "CURRENT_${name}_VERSION=${current}"
    if [[ "$releasing" == "true" ]]; then
        echo "NEXT_${name}_VERSION=$(leaf_next "$current")"
    else
        echo "NEXT_${name}_VERSION=unchanged"
    fi
}

emit_leaf RB "clients/temper-rb/lib/temper/version.rb" "^[[:space:]]*VERSION = "   "$RB_RELEASE"
emit_leaf PY "clients/temper-py/temper/version.py"     "__version__"  "$PY_RELEASE"
emit_leaf TS "clients/temper-ts/package.json"          '"version"'    "$TS_RELEASE"
emit_leaf TELEMETRY "clients/temper-telemetry-ts/package.json" '"version"' "$TELEMETRY_RELEASE"
emit_leaf UI "packages/temper-ui/package.json"         '"version"'    "$UI_RELEASE"
emit_leaf CLOUD "packages/temper-cloud/package.json"   '"version"'    "$CLOUD_RELEASE"

# ---------------------------------------------------------------------------
# Re-emit detect-changes variables so callers get everything in one eval
# ---------------------------------------------------------------------------
echo "CHANGES_BASE_REF=${CHANGES_BASE_REF}"
echo "CORE_CHANGED=${CORE_CHANGED}"
echo "WIRE_CHANGED=${WIRE_CHANGED}"
echo "CLIENTS_CHANGED=${CLIENTS_CHANGED}"
echo "PACKAGES_CHANGED=${PACKAGES_CHANGED}"
echo "SCHEMA_CHANGED=${SCHEMA_CHANGED}"
echo "INFRA_CHANGED=${INFRA_CHANGED}"
