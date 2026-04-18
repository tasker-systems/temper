#!/usr/bin/env bash
# .github/scripts/release/generate-summary.sh
#
# Print a per-platform summary to $GITHUB_STEP_SUMMARY.
#
# Required env:
#   VERSION              — e.g. 0.1.0
#   DARWIN_ARM64_RESULT  — success|failure|cancelled|skipped
#   LINUX_X64_RESULT
#   WINDOWS_X64_RESULT
#   GITHUB_STEP_SUMMARY  — auto-provided by GitHub Actions

set -euo pipefail

: "${VERSION:?VERSION required}"
: "${GITHUB_STEP_SUMMARY:?GITHUB_STEP_SUMMARY required}"

DARWIN_ARM64_RESULT="${DARWIN_ARM64_RESULT:-unknown}"
LINUX_X64_RESULT="${LINUX_X64_RESULT:-unknown}"
WINDOWS_X64_RESULT="${WINDOWS_X64_RESULT:-unknown}"

icon() {
    case "$1" in
        success) echo "✅" ;;
        failure) echo "❌" ;;
        cancelled) echo "⏹️" ;;
        skipped) echo "⏭️" ;;
        *) echo "❓" ;;
    esac
}

{
    echo "## Release v${VERSION}"
    echo ""
    echo "| Platform | Result |"
    echo "|---|---|"
    echo "| darwin-arm64 | $(icon "$DARWIN_ARM64_RESULT") $DARWIN_ARM64_RESULT |"
    echo "| linux-x64    | $(icon "$LINUX_X64_RESULT") $LINUX_X64_RESULT |"
    echo "| windows-x64  | $(icon "$WINDOWS_X64_RESULT") $WINDOWS_X64_RESULT |"
} >> "$GITHUB_STEP_SUMMARY"
