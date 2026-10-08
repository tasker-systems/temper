#!/usr/bin/env bash
# Fail if a public doc tells people to brew-install a temper minor other than the workspace's.
#
# WHY: Homebrew trusts a third-party formula when it is installed by its FULL name
# (`tasker-systems/tap/temper@0.6`), but not through the `temper` alias — the alias refuses until
# the user runs `brew trust`. So the docs lead with the versioned name, and a versioned name goes
# stale at every new minor: the release job renders the new formula and moves the alias in the
# tap, but nothing rewrites these lines here. Copying a stale line installs the OLD minor, quietly.
#
# A minor bump moves the workspace `version` in Cargo.toml, so this pins every versioned install
# reference under README.md and docs/ to that minor — the bump PR is the one that fails.
#
# It also refuses to find NO references: the docs dropping the versioned line is a decision this
# gate should surface, not one it should pass silently.
#
# Usage: bash .github/scripts/check-brew-install-minor.sh

set -euo pipefail

ROOT="${BREW_MINOR_REPO_ROOT:-$(git rev-parse --show-toplevel)}"
cd "$ROOT"

version="$(sed -n 's/^version = "\([0-9]*\.[0-9]*\)\.[0-9].*"/\1/p' Cargo.toml | head -1)"
if [ -z "$version" ]; then
  echo "::error::could not read the workspace version from Cargo.toml"
  exit 1
fi

refs="$(grep -rnoE 'tasker-systems/tap/temper@[0-9]+\.[0-9]+' README.md docs/ \
  --exclude-dir=node_modules || true)"
if [ -z "$refs" ]; then
  echo "::error::no versioned brew install reference found in README.md or docs/ — if dropping it was deliberate, retire this check"
  exit 1
fi

stale="$(printf '%s\n' "$refs" | grep -v "temper@${version}\$" || true)"
if [ -n "$stale" ]; then
  echo "::error::brew install references name a minor other than the workspace's ${version}:"
  printf '%s\n' "$stale"
  echo "Update them to tasker-systems/tap/temper@${version}."
  exit 1
fi

echo "brew install references: $(printf '%s\n' "$refs" | wc -l | tr -d ' ') pinned to ${version}"
