#!/usr/bin/env bash
#
# Regenerate clients/temper-ts/src/generated/schema.ts from the repo-root openapi.json.
#
# The generated schema is a committed *product of openapi.json* (itself a product of
# the Axum router), so a new field on a response DTO leaves temper-ts stale — the
# same class of drift the openapi-check gate guards for the spec itself, and the same
# one generate-temper-rb.sh guards for the gem.
#
# This script is the single source of truth for the generator invocation. Called from
# four places that must agree, or the drift gate would be checking a different
# artifact than the one it tells you to regenerate:
#   - `cargo make openapi` / `cargo make openapi-ts` (local dev, regen)
#   - `cargo make openapi-ts-drift` → check-temper-ts-drift.sh (local dev, verify)
#   - `npm run generate` from inside clients/temper-ts (what its README tells you)
#   - the temper-ts CI job's drift step (.github/workflows/test-agents-ts.yml)
#
# Unlike the gem's generator this needs neither Docker nor Java — openapi-typescript
# is an npm devDependency, so a Rust dev who changed a DTO regenerates with Node
# alone (~70ms). That is why, unlike openapi-rb-drift, the TS drift gate NEVER skips.
#
# The generator version is pinned EXACTLY in clients/temper-ts/package.json (no
# caret) and locked in package-lock.json: a moving generator makes the drift gate
# fail on days when nothing in this repo changed.
#
# Usage: bash .github/scripts/generate-temper-ts.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SPEC="$REPO_ROOT/openapi.json"
PKG="$REPO_ROOT/clients/temper-ts"
OUT="$PKG/src/generated/schema.ts"

if [ ! -s "$SPEC" ]; then
  echo "ERROR: openapi.json is missing or empty — run: cargo make openapi" >&2
  exit 1
fi

# The pin above is only load-bearing if the binary we RUN is the pinned one, so check
# the INSTALLED version against the PINNED one — presence is not enough. An earlier
# guard tested only that node_modules/.bin/openapi-typescript existed, which made the
# pin a comment: a stale binary from a previous checkout emitted silently, and the
# failure was local-green / CI-red. Bump the pin, regenerate with the stale binary,
# and `cargo make openapi-ts-drift` PASSES — it regenerates with that same stale
# binary, so it is merely self-consistent. Commit, and CI's fresh `npm ci` emits
# different bytes and fails, telling you to run the very command you just ran. (7.4.0
# against a 7.13.0 pin: 252 insertions, 504 deletions, no warning.) The gem's
# generator cannot drift this way — it has no node_modules cache to go stale.
#
# npm ci (not install) so the LOCKED version is what lands. Only on a mismatch, so a
# regen with the right binary already in place stays free. temper-ts is
# workspace-isolated: npm MUST run from inside it (a root install inherits the root's
# bun overrides and fails).
PINNED="$(node -p "require('$PKG/package.json').devDependencies['openapi-typescript']")"
INSTALLED="$(node -p "require('$PKG/node_modules/openapi-typescript/package.json').version" 2>/dev/null || echo none)"
if [ "$PINNED" != "$INSTALLED" ]; then
  echo "  installing temper-ts devDependencies (openapi-typescript $PINNED, found $INSTALLED)…" >&2
  (cd "$PKG" && npm ci)
fi

mkdir -p "$(dirname "$OUT")"
(cd "$PKG" && ./node_modules/.bin/openapi-typescript "$SPEC" -o "$OUT")
