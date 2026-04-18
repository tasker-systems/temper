#!/usr/bin/env bash
# .github/scripts/release/check-failures.sh
#
# Emit has_failures=true|false to $GITHUB_OUTPUT based on per-platform
# build results.
#
# Required env:
#   DARWIN_ARM64_RESULT
#   LINUX_X64_RESULT
#   WINDOWS_X64_RESULT

set -euo pipefail

: "${GITHUB_OUTPUT:?GITHUB_OUTPUT required}"

HAS_FAILURES=false

for var in DARWIN_ARM64_RESULT LINUX_X64_RESULT WINDOWS_X64_RESULT; do
    value="${!var:-unknown}"
    if [[ "$value" != "success" && "$value" != "skipped" ]]; then
        HAS_FAILURES=true
        echo "::warning::$var = $value"
    fi
done

echo "has_failures=${HAS_FAILURES}" >> "$GITHUB_OUTPUT"
