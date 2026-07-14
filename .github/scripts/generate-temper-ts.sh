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
# three places that must agree, or the drift gate would be checking a different
# artifact than the one it tells you to regenerate:
#   - `cargo make openapi` / `cargo make openapi-ts` (local dev, regen)
#   - `cargo make openapi-ts-drift` → check-temper-ts-drift.sh (local dev, verify)
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

# npm ci (not install) so the LOCKED generator version is what emits — but only when
# the binary is absent, since a wholesale reinstall on every regen would make
# `cargo make check` pay seconds for nothing. temper-ts is workspace-isolated: npm
# MUST run from inside it (a root install inherits the root's bun overrides and fails).
if [ ! -x "$PKG/node_modules/.bin/openapi-typescript" ]; then
  echo "  installing temper-ts devDependencies (pinned openapi-typescript)…" >&2
  (cd "$PKG" && npm ci)
fi

mkdir -p "$(dirname "$OUT")"
(cd "$PKG" && ./node_modules/.bin/openapi-typescript "$SPEC" -o "$OUT")
