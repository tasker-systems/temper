#!/usr/bin/env bash
#
# Verify the committed openapi.json matches a fresh emission from the router.
#
# The spec is a product of the Axum router (utoipa-axum's OpenApiRouter), so a
# route added via `routes!(...)` changes the contract. This gate fails when the
# committed artifact no longer matches what the router produces.
#
# Invoked two ways, so the diff logic lives here rather than in either caller:
#   - `cargo make openapi-check` (local dev)
#   - the `rust-quality` CI job, directly
#
# CI cannot use cargo-make: the root Makefile.toml declares `env_files = ["./.env"]`
# and no `.env` exists on a runner, so every `cargo make` invocation fails before
# reaching a task.
#
# Usage: bash .github/scripts/check-openapi-spec.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
COMMITTED="$REPO_ROOT/openapi.json"

if [ ! -s "$COMMITTED" ]; then
  echo "ERROR: openapi.json is missing or empty — run: cargo make openapi" >&2
  exit 1
fi

FRESH="$(mktemp)"
trap 'rm -f "$FRESH"' EXIT

# `emit-openapi` is a pure function of the router: no AppState, no database, no
# ONNX at runtime (ort loads lazily). Build chatter goes to stderr.
cargo run --quiet --manifest-path "$REPO_ROOT/Cargo.toml" -p temper-api --bin emit-openapi > "$FRESH"

if ! diff -u "$COMMITTED" "$FRESH"; then
  echo >&2
  echo "ERROR: openapi.json is out of date with the router — run: cargo make openapi" >&2
  exit 1
fi

echo "openapi.json is up to date with the router"
