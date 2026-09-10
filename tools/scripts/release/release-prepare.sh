#!/usr/bin/env bash
# tools/scripts/release/release-prepare.sh
#
# Prepare a release branch with the version bumps, then open a PR to main.
#
# EXTEND of temper's prior-generation release-prepare.sh (pre-flight, merge-log
# PR section kept) onto the shared-semver-policy spine, mirroring tasker-core's
# release-prepare.sh structure. Spec of record:
# temper-artifacts/specs/2026-09-09-shared-semver-policy-design.md §5; flow
# CONFORM to RELEASING.md (merge → release-tag.yml pushes v<VERSION> →
# release.yml builds the GitHub Release — this PR is the version bump on main).
#
# Usage:
#   ./tools/scripts/release/release-prepare.sh [--minor] [--override-blocker]
#       [--rb] [--py] [--ts] [--telemetry] [--ui] [--cloud] [--dry-run] [--yes] [--from TAG]
#
# Flow:
#   1. Pre-flight: clean tree, on main, up-to-date, gh available
#   2. Detect changes + calculate versions (register-gated)
#   3. Summary: version table + register roll-up
#   4. Create release/v<NEXT> branch, bump every version site
#   5. cargo check as a sanity gate
#   6. Commit, push, open PR (template-conformant body with the Compat class
#      declaration built from the register roll-up)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "${SCRIPT_DIR}/lib/common.sh"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
DRY_RUN=false
YES=false
CALC_ARGS=()

while [[ $# -gt 0 ]]; do
    case $1 in
        --dry-run)          DRY_RUN=true; shift ;;
        --yes|-y)           YES=true; shift ;;
        --minor)            CALC_ARGS+=(--minor); shift ;;
        --override-blocker) CALC_ARGS+=(--override-blocker); shift ;;
        --rb)               CALC_ARGS+=(--rb); shift ;;
        --py)               CALC_ARGS+=(--py); shift ;;
        --ts)               CALC_ARGS+=(--ts); shift ;;
        --telemetry)        CALC_ARGS+=(--telemetry); shift ;;
        --ui)               CALC_ARGS+=(--ui); shift ;;
        --cloud)            CALC_ARGS+=(--cloud); shift ;;
        --from)             CALC_ARGS+=(--from "$2"); shift 2 ;;
        --from=*)           CALC_ARGS+=(--from "${1#*=}"); shift ;;
        *) die "Unknown argument: $1" ;;
    esac
done

log_header "Temper Release Preparation"

# ---------------------------------------------------------------------------
# Pre-flight checks
# ---------------------------------------------------------------------------
log_section "Pre-flight checks"

if ! git diff-index --quiet HEAD -- 2>/dev/null; then
    if [[ "$DRY_RUN" == "true" ]]; then
        log_warn "Uncommitted changes detected (ignored in dry-run mode)"
    else
        die "Uncommitted changes detected. Commit or stash first."
    fi
else
    log_info "Working tree is clean"
fi

BRANCH=$(git branch --show-current)
if [[ "$BRANCH" != "main" ]]; then
    if [[ "$DRY_RUN" == "true" ]]; then
        log_warn "On branch '$BRANCH', not 'main' (ignored in dry-run mode)"
    else
        die "Must be on main branch (currently on '$BRANCH')"
    fi
else
    log_info "On main branch"
fi

git fetch origin --quiet
LOCAL_SHA=$(git rev-parse HEAD)
REMOTE_SHA=$(git rev-parse origin/main 2>/dev/null || echo "unknown")
if [[ "$LOCAL_SHA" != "$REMOTE_SHA" ]]; then
    if [[ "$DRY_RUN" == "true" ]]; then
        log_warn "Local branch is not up-to-date with origin/main (ignored in dry-run mode)"
    else
        die "Local main is not up-to-date with origin/main. Run: git pull"
    fi
else
    log_info "main is up-to-date with origin"
fi

if ! command -v gh &>/dev/null; then
    die "gh CLI not found. Install: https://cli.github.com/"
fi
log_info "gh CLI available"

# ---------------------------------------------------------------------------
# Change detection + version calculation (register-gated)
#
# Captured, not eval'd directly: a register refusal must stop this script, and
# `eval "$(dying-cmd)"` would swallow the non-zero exit.
# ---------------------------------------------------------------------------
log_section "Detecting changes and calculating versions"

CALC_OUTPUT="$("${SCRIPT_DIR}/calculate-versions.sh" ${CALC_ARGS[@]+"${CALC_ARGS[@]}"})" \
    || die "calculate-versions.sh failed (register gates refuse, or a version site was unreadable — see its output above)"
eval "$CALC_OUTPUT"

log_info "Base ref: ${CHANGES_BASE_REF}"
log_info "Core changed: ${CORE_CHANGED} (wire: ${WIRE_CHANGED}, clients: ${CLIENTS_CHANGED}, packages: ${PACKAGES_CHANGED}, schema: ${SCHEMA_CHANGED}, infra: ${INFRA_CHANGED})"

# ---------------------------------------------------------------------------
# Nothing to release?
# ---------------------------------------------------------------------------
NOTHING_TO_RELEASE=true
if [[ "$NEXT_CORE_VERSION" != "$CURRENT_CORE_VERSION" ]]; then
    NOTHING_TO_RELEASE=false
fi
for leaf in RB PY TS TELEMETRY UI CLOUD; do
    NEXT_VAL_VAR="NEXT_${leaf}_VERSION"
    if [[ "${!NEXT_VAL_VAR}" != "unchanged" ]]; then
        NOTHING_TO_RELEASE=false
    fi
done

if [[ "$NOTHING_TO_RELEASE" == "true" ]]; then
    log_warn "No bump-forcing changes and no leaves to release since ${CHANGES_BASE_REF} — nothing to release"
    exit 0
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
log_section "Release Summary"

echo ""
printf "  %-16s %s -> %s\n" "Core (VERSION):" "${CURRENT_CORE_VERSION}" "${NEXT_CORE_VERSION}"

print_leaf_row() {
    local label="$1" leaf="$2"
    local cur_var="CURRENT_${leaf}_VERSION" next_var="NEXT_${leaf}_VERSION"
    if [[ "${!next_var}" == "unchanged" ]]; then
        printf "  %-16s %s (unchanged)\n" "${label}:" "${!cur_var}"
    else
        printf "  %-16s %s -> %s\n" "${label}:" "${!cur_var}" "${!next_var}"
    fi
}
print_leaf_row "temper-rb" RB
print_leaf_row "temper-py" PY
print_leaf_row "temper-ts" TS
print_leaf_row "temper-telemetry-ts" TELEMETRY
print_leaf_row "temper-ui" UI
print_leaf_row "temper-cloud" CLOUD

echo ""
echo "  Register roll-up (window since ${REGISTER_WINDOW:-<no '## Since' section>}): ${REGISTER_ROWS} rows"
echo "    classes: additive ${REGISTER_ADDITIVE}, shape-breaking ${REGISTER_SHAPE_BREAKING}, behavioral ${REGISTER_BEHAVIORAL}"
echo "    status:  open ${REGISTER_OPEN}, signal-only ${REGISTER_SIGNAL_ONLY}, blocked ${REGISTER_BLOCKED}"

# Named roll-up, read fresh from the register (same parser the gates used)
REGISTER_FILE="${REPO_ROOT}/RELEASE_REGISTER.md"
register_window_rows "$REGISTER_FILE" | while IFS=$'\t' read -r r_pr r_classes r_surfaces r_status r_citation; do
    [[ -n "$r_citation" ]] || continue
    case "$r_status" in
        open)
            echo "    open (gates this release until carried): pr:${r_pr} [${r_surfaces}] ${r_citation}"
            ;;
        blocked:*)
            echo "    blocked: '${r_status#blocked:}' — pr:${r_pr} [${r_surfaces}] ${r_citation}"
            ;;
    esac
done

# ---------------------------------------------------------------------------
# Sanity check
#
# Dry-run validates the current tree (no bump has happened); the live path
# validates the release branch after the bump.
# ---------------------------------------------------------------------------
log_section "Sanity check (cargo check)"
if [[ "$DRY_RUN" == "true" ]]; then
    log_info "Dry-run: checking the current (pre-bump) tree"
fi
SQLX_OFFLINE=true cargo check --workspace

# ---------------------------------------------------------------------------
# Dry-run: exit here
# ---------------------------------------------------------------------------
if [[ "$DRY_RUN" == "true" ]]; then
    echo ""
    log_info "DRY RUN -- no branch created, no PR opened"
    exit 0
fi

# ---------------------------------------------------------------------------
# Confirm
# ---------------------------------------------------------------------------
if [[ "$YES" != "true" ]]; then
    echo ""
    confirm "Create release branch and prepare PR?"
fi

# ---------------------------------------------------------------------------
# Build update-versions arguments
# ---------------------------------------------------------------------------
UPDATE_ARGS="--core ${NEXT_CORE_VERSION}"
for leaf in RB PY TS TELEMETRY UI CLOUD; do
    NEXT_VAL_VAR="NEXT_${leaf}_VERSION"
    if [[ "${!NEXT_VAL_VAR}" != "unchanged" ]]; then
        UPDATE_ARGS+=" --$(echo "${leaf}" | tr '[:upper:]' '[:lower:]') ${!NEXT_VAL_VAR}"
    fi
done

# ---------------------------------------------------------------------------
# Create release branch
# ---------------------------------------------------------------------------
RELEASE_BRANCH="release/v${NEXT_CORE_VERSION}"
log_section "Creating branch: ${RELEASE_BRANCH}"
git checkout -b "$RELEASE_BRANCH"

# ---------------------------------------------------------------------------
# Bump versions
# ---------------------------------------------------------------------------
log_section "Bumping versions"
# shellcheck disable=SC2086
"${SCRIPT_DIR}/update-versions.sh" ${UPDATE_ARGS}

# ---------------------------------------------------------------------------
# Sanity check on the bumped tree
# ---------------------------------------------------------------------------
log_section "Sanity check (cargo check)"
SQLX_OFFLINE=true cargo check --workspace

# ---------------------------------------------------------------------------
# Commit
# ---------------------------------------------------------------------------
log_section "Committing changes"
git add -u
git commit -m "chore(release): prepare v${NEXT_CORE_VERSION}"

# ---------------------------------------------------------------------------
# Push + PR
# ---------------------------------------------------------------------------
log_section "Pushing and creating PR"
git push -u origin "$RELEASE_BRANCH"

PR_TITLE="chore(release): prepare v${NEXT_CORE_VERSION}"

# GitHub rejects PR bodies over 65536 characters. Summarize the merge stream
# rather than every commit, and cap it so a long release cannot overrun.
MAX_LOG_BYTES=50000

REPO_URL="$(gh repo view --json url --jq .url)"
COMPARE_URL="${REPO_URL}/compare/${CHANGES_BASE_REF}...${RELEASE_BRANCH}"

FULL_LOG="$(git log "${CHANGES_BASE_REF}..HEAD" --oneline --no-decorate --first-parent)"
TOTAL_LINES="$(printf '%s\n' "$FULL_LOG" | wc -l | tr -d ' ')"
LOG="$(printf '%s\n' "$FULL_LOG" |
    awk -v max="$MAX_LOG_BYTES" '{ n += length($0) + 1; if (n > max) exit; print }')"
SHOWN_LINES="$(printf '%s\n' "$LOG" | wc -l | tr -d ' ')"

# Compat class declaration, from the register roll-up (the release carries
# every class present in the window).
COMPAT_CLASSES=""
[[ "$REGISTER_ADDITIVE" -gt 0 ]] && COMPAT_CLASSES+=", additive"
[[ "$REGISTER_BEHAVIORAL" -gt 0 ]] && COMPAT_CLASSES+=", behavioral"
if [[ "$REGISTER_SHAPE_BREAKING" -gt 0 ]]; then
    COMPAT_CLASSES+=", shape-breaking (M hand-raised to ${NEXT_CORE_VERSION})"
fi
COMPAT_CLASSES="${COMPAT_CLASSES#, }"
[[ -z "$COMPAT_CLASSES" ]] && COMPAT_CLASSES="none declared in the window"

OPEN_LIST="$(register_window_rows "$REGISTER_FILE" | awk -F'\t' '$4 == "open" { print $1 }' | awk '!seen[$0]++' | awk '{ n++; prs = prs (n > 1 ? ", " : "") "pr:" $1 } END { print prs }')"

PR_BODY="## What"$'\n\n'
PR_BODY+="Prepare release v${NEXT_CORE_VERSION}: the shared-version bump across every version site (VERSION, workspace crates, client skins, npm packages), with the release-verdict register rolled up below."$'\n\n'
PR_BODY+="## Why"$'\n\n'
PR_BODY+="Cutting the release — the versioned source that the \`v${NEXT_CORE_VERSION}\` tag and the cross-platform CLI binaries derive from ([RELEASING.md](RELEASING.md))."$'\n\n'
PR_BODY+="## Verification"$'\n\n'
PR_BODY+="- \`SQLX_OFFLINE=true cargo check --workspace\` green on this branch (run in preparation)."$'\n'
PR_BODY+="- Roll-up reproducible with: \`tools/scripts/release/calculate-versions.sh\`."$'\n\n'
PR_BODY+="## Compat class"$'\n\n'
PR_BODY+="${COMPAT_CLASSES} — rows live in [RELEASE_REGISTER.md](RELEASE_REGISTER.md) under \`## Since ${REGISTER_WINDOW}\`: ${REGISTER_OPEN} open${OPEN_LIST:+ (${OPEN_LIST})}, ${REGISTER_SIGNAL_ONLY} signal-only, ${REGISTER_BLOCKED} blocked."$'\n\n'
PR_BODY+="### Version Changes"$'\n\n'
PR_BODY+="| Component | Current | Next |"$'\n'
PR_BODY+="|-----------|---------|------|"$'\n'
PR_BODY+="| Core (VERSION, workspace crates) | ${CURRENT_CORE_VERSION} | ${NEXT_CORE_VERSION} |"$'\n'
for pair in "temper-rb:RB" "temper-py:PY" "temper-ts:TS" "temper-telemetry-ts:TELEMETRY" "temper-ui:UI" "temper-cloud:CLOUD"; do
    label="${pair%%:*}"; leaf="${pair##*:}"
    cur_var="CURRENT_${leaf}_VERSION"; next_var="NEXT_${leaf}_VERSION"
    if [[ "${!next_var}" != "unchanged" ]]; then
        PR_BODY+="| ${label} | ${!cur_var} | ${!next_var} |"$'\n'
    fi
done
PR_BODY+=$'\n'"### Merges since ${CHANGES_BASE_REF}"$'\n\n'
PR_BODY+="\`\`\`"$'\n'
PR_BODY+="${LOG}"$'\n'
PR_BODY+="\`\`\`"$'\n'
if (( SHOWN_LINES < TOTAL_LINES )); then
    PR_BODY+=$'\n'"_Truncated: showing ${SHOWN_LINES} of ${TOTAL_LINES} merges._"$'\n'
fi
PR_BODY+=$'\n'"[Full diff](${COMPARE_URL})"$'\n\n'
PR_BODY+="### On merge"$'\n\n'
PR_BODY+="The \`release-tag\` workflow will automatically push the \`v${NEXT_CORE_VERSION}\` tag, which triggers \`release.yml\` to build and publish the binaries."$'\n'

PR_BODY_FILE="$(mktemp)"
trap 'rm -f "$PR_BODY_FILE"' EXIT
printf '%s' "$PR_BODY" > "$PR_BODY_FILE"

gh pr create \
    --title "$PR_TITLE" \
    --body-file "$PR_BODY_FILE" \
    --base main \
    --head "$RELEASE_BRANCH"

log_section "Done"
echo ""
echo "  Release branch: ${RELEASE_BRANCH}"
echo "  PR created — merge to main to trigger the release build."
echo ""
