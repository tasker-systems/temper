#!/usr/bin/env bash
# tools/scripts/release/release-prepare.sh
#
# Prepare a release branch with a version bump, then open a PR to main.
#
# Usage:
#   ./tools/scripts/release/release-prepare.sh [--bump patch|minor|major] \
#       [--dry-run] [--yes] [--from TAG]
#
# Flow:
#   1. Pre-flight: clean tree, on main, up-to-date, gh available
#   2. Detect changes + calculate next version
#   3. Display summary, confirm
#   4. Create release/v<N.N.N> branch
#   5. Bump VERSION + temper-cli/Cargo.toml
#   6. cargo check as a sanity gate
#   7. Commit, push, open PR

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "${SCRIPT_DIR}/lib/common.sh"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
BUMP="patch"
DRY_RUN=false
YES=false
CALC_ARGS=()

while [[ $# -gt 0 ]]; do
    case $1 in
        --bump)    BUMP="$2"; CALC_ARGS+=(--bump "$2"); shift 2 ;;
        --bump=*)  BUMP="${1#*=}"; CALC_ARGS+=(--bump "${1#*=}"); shift ;;
        --dry-run) DRY_RUN=true; shift ;;
        --yes|-y)  YES=true; shift ;;
        --from)    CALC_ARGS+=(--from "$2"); shift 2 ;;
        --from=*)  CALC_ARGS+=(--from "${1#*=}"); shift ;;
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
# Change detection + version calculation
# ---------------------------------------------------------------------------
log_section "Detecting changes and calculating version"

# shellcheck disable=SC2046
eval "$("${SCRIPT_DIR}/calculate-version.sh" ${CALC_ARGS[@]+"${CALC_ARGS[@]}"})"

log_info "Base ref: ${CHANGES_BASE_REF}"
log_info "CLI changed: ${CLI_CHANGED}"

if [[ "$CLI_CHANGED" != "true" ]]; then
    log_warn "No changes to temper-cli or its deps since ${CHANGES_BASE_REF} — nothing to release"
    exit 0
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
log_section "Release Summary"

echo ""
echo "  Version: ${CURRENT_VERSION} -> ${NEXT_VERSION} (${BUMP})"
echo ""

# ---------------------------------------------------------------------------
# Dry-run: exit here
# ---------------------------------------------------------------------------
if [[ "$DRY_RUN" == "true" ]]; then
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
# Create release branch
# ---------------------------------------------------------------------------
RELEASE_BRANCH="release/v${NEXT_VERSION}"
log_section "Creating branch: ${RELEASE_BRANCH}"
git checkout -b "$RELEASE_BRANCH"

# ---------------------------------------------------------------------------
# Bump version
# ---------------------------------------------------------------------------
log_section "Bumping version"
"${SCRIPT_DIR}/update-version.sh" --version "${NEXT_VERSION}"

# ---------------------------------------------------------------------------
# Sanity check: verify workspace compiles
# ---------------------------------------------------------------------------
log_section "Sanity check (cargo check)"
SQLX_OFFLINE=true cargo check --workspace

# ---------------------------------------------------------------------------
# Commit
# ---------------------------------------------------------------------------
log_section "Committing changes"
git add -u
git commit -m "release: v${NEXT_VERSION}"

# ---------------------------------------------------------------------------
# Push + PR
# ---------------------------------------------------------------------------
log_section "Pushing and creating PR"
git push -u origin "$RELEASE_BRANCH"

PR_TITLE="release: v${NEXT_VERSION}"

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

PR_BODY="## Release v${NEXT_VERSION}"$'\n\n'
PR_BODY+="Prepared by \`cargo make release-prepare\`."$'\n\n'
PR_BODY+="### Merges since ${CHANGES_BASE_REF}"$'\n\n'
PR_BODY+="\`\`\`"$'\n'
PR_BODY+="${LOG}"$'\n'
PR_BODY+="\`\`\`"$'\n'
if (( SHOWN_LINES < TOTAL_LINES )); then
    PR_BODY+=$'\n'"_Truncated: showing ${SHOWN_LINES} of ${TOTAL_LINES} merges._"$'\n'
fi
PR_BODY+=$'\n'"[Full diff](${COMPARE_URL})"$'\n\n'
PR_BODY+="### On merge"$'\n\n'
PR_BODY+="The \`release-tag\` workflow will automatically push the \`v${NEXT_VERSION}\` tag, which triggers \`release.yml\` to build and publish the binaries."$'\n'

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
