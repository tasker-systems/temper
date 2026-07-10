#!/usr/bin/env bash
#
# Fail if the committed temper-rb generated core drifts from openapi.json.
#
# Regenerates the gem (via generate-temper-rb.sh) and diffs the result against
# what is committed — the local mirror of the `test-ruby` CI job's `rake drift`.
# This exists so a DTO change cannot pass a green local `cargo make check` and
# then die in the Ruby CI drift gate (which is what bit issue #354).
#
# Requires Docker. When Docker is unavailable this SKIPS with a loud notice
# rather than failing: `cargo make check` must stay runnable on a machine with no
# Docker daemon (the CI ruby job is the backstop that never skips). Mirrors the
# reasoning that keeps the Docker-based `openapi-validate` out of the default gate.
#
# Usage: bash .github/scripts/check-temper-rb-drift.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GENERATED="clients/temper-rb/lib/temper/generated"
GENERATED_RB="clients/temper-rb/lib/temper/generated.rb"

if ! docker info >/dev/null 2>&1; then
  echo "SKIP: temper-rb drift check — Docker is not available." >&2
  echo "      The gem is generated from openapi.json; run 'cargo make openapi-rb' with" >&2
  echo "      Docker running to regenerate it. The test-ruby CI job is the backstop." >&2
  exit 0
fi

bash "$REPO_ROOT/.github/scripts/generate-temper-rb.sh"

if ! git -C "$REPO_ROOT" diff --exit-code -- "$GENERATED" "$GENERATED_RB"; then
  echo >&2
  echo "ERROR: temper-rb generated core is out of date with openapi.json." >&2
  echo "       Run: cargo make openapi   (regenerates the spec and the gem)" >&2
  echo "       then commit the regenerated clients/temper-rb files." >&2
  exit 1
fi

echo "temper-rb generated core is up to date with openapi.json"
