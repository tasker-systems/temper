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

# ---------------------------------------------------------------------------------
# The DERIVATION path — no CHANGED_PATHS, so the script must work out the changeset
# from git itself. This is the half that runs in a real build, and leaving it uncovered
# is how the first version shipped depending on VERCEL_GIT_PREVIOUS_SHA, a variable that
# is unset precisely when previews have never built.
#
# Everything below is hermetic: a throwaway origin plus clone under mktemp, never the
# repo this script lives in.
# ---------------------------------------------------------------------------------
expect_in() { # expect_in <dir> <desc> <expected-exit> <env assignments...>
  local dir="$1" desc="$2" want="$3"; shift 3
  local got=0
  ( cd "$dir" && env -u VERCEL_GIT_PREVIOUS_SHA -u CHANGED_PATHS "$@" sh "$SCRIPT" ) >/dev/null 2>&1 || got=$?
  if [ "$got" -eq "$want" ]; then
    echo "  ok   — $desc (exit $got)"
  else
    echo "  FAIL — $desc: expected exit $want, got $got"; fails=$((fails+1))
  fi
}

FIXTURE="$(mktemp -d)"
trap 'rm -rf "$FIXTURE"' EXIT
git init -q --bare "$FIXTURE/origin.git"
git -c init.defaultBranch=main clone -q "file://$FIXTURE/origin.git" "$FIXTURE/work" 2>/dev/null
(
  cd "$FIXTURE/work"
  git config user.email t@example.com; git config user.name t
  git checkout -q -b main 2>/dev/null || true
  echo hello > README.md && git add README.md && git commit -qm base
  git push -q origin main
  # A branch whose diff against main carries a migration.
  git checkout -q -b with-migration
  mkdir -p migrations && echo "SELECT 1;" > migrations/20260730_x.sql
  git add migrations && git commit -qm "add migration"
  # A branch whose diff against main carries no migration.
  git checkout -q main && git checkout -q -b without-migration
  echo more >> README.md && git add README.md && git commit -qm "docs only"
)

( cd "$FIXTURE/work" && git checkout -q with-migration )
expect_in "$FIXTURE/work" "derived: branch adding a migration builds" 1 VERCEL_ENV=preview

( cd "$FIXTURE/work" && git checkout -q without-migration )
expect_in "$FIXTURE/work" "derived: branch with no migration skips" 0 VERCEL_ENV=preview

# No git checkout at all: build rather than guess. This is the case CHANGED_PATHS
# set-but-empty does NOT cover — empty means "the changeset is empty", absent means
# "we have no signal", and the script must not collapse them.
mkdir -p "$FIXTURE/nogit"
expect_in "$FIXTURE/nogit" "no git checkout builds" 1 VERCEL_ENV=preview

# Production still wins over everything, even where a changeset could be derived.
( cd "$FIXTURE/work" && git checkout -q without-migration )
expect_in "$FIXTURE/work" "production builds even where derivation would skip" 1 VERCEL_ENV=production

if [ "$fails" -gt 0 ]; then echo "FAILED: $fails assertion(s)"; exit 1; fi
echo "all assertions passed"
