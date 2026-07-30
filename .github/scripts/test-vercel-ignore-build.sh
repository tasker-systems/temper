#!/usr/bin/env bash
# Guard test for scripts/vercel-ignore-build.sh.
#
# Vercel's Ignored Build Step inverts the usual convention: exit 0 SKIPS the build,
# exit 1 BUILDS it. Every assertion below is written in those terms, because getting
# the polarity backwards would silently stop production from deploying — a far worse
# outcome than the cost this script exists to avoid.
#
# The assertions run the script under `env -u VERCEL_GIT_PREVIOUS_SHA` so the harness
# is hermetic: if a real Vercel variable leaked in from the surrounding environment,
# the CHANGED_PATHS branch under test would be bypassed and the suite would pass or
# fail for a reason that has nothing to do with the script.
set -euo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/scripts/vercel-ignore-build.sh"
fails=0

expect() { # expect <desc> <expected-exit> <env assignments...>
  local desc="$1" want="$2"; shift 2
  local got=0
  env -u VERCEL_GIT_PREVIOUS_SHA "$@" sh "$SCRIPT" >/dev/null 2>&1 || got=$?
  if [ "$got" -eq "$want" ]; then
    echo "  ok   — $desc (exit $got)"
  else
    echo "  FAIL — $desc: expected exit $want, got $got"; fails=$((fails+1))
  fi
}

echo "vercel-ignore-build guard test"

# Production must ALWAYS build, whatever changed.
expect "production always builds" 1 VERCEL_ENV=production CHANGED_PATHS=""
expect "production builds even with no migration" 1 VERCEL_ENV=production CHANGED_PATHS="README.md"

# Preview builds only when migrations/ moved.
expect "preview WITH a migration builds"    1 VERCEL_ENV=preview CHANGED_PATHS="migrations/20260730000010_x.sql"
expect "preview with a migration among others builds" 1 VERCEL_ENV=preview CHANGED_PATHS=$'README.md\nmigrations/2026_x.sql'
expect "preview WITHOUT a migration skips"  0 VERCEL_ENV=preview CHANGED_PATHS="README.md"
expect "preview with an empty changeset skips" 0 VERCEL_ENV=preview CHANGED_PATHS=""

# A path merely CONTAINING the word must not count — only the migrations/ directory.
expect "docs mentioning migrations do not trigger" 0 VERCEL_ENV=preview CHANGED_PATHS="docs/migrations-guide.md"

# Fail SAFE: an unknown environment builds rather than silently skipping.
expect "unknown VERCEL_ENV builds" 1 VERCEL_ENV= CHANGED_PATHS="README.md"

# A preview that cannot determine its changeset at all builds rather than guessing.
# This is the case the assertion above it does NOT cover: CHANGED_PATHS set-but-empty
# means "the changeset is empty", while CHANGED_PATHS absent means "we do not know".
# Those are different questions and the script must not collapse them.
expect "preview with no changeset signal at all builds" 1 VERCEL_ENV=preview

if [ "$fails" -gt 0 ]; then echo "FAILED: $fails assertion(s)"; exit 1; fi
echo "all assertions passed"
