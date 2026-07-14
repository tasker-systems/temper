#!/usr/bin/env bash
#
# Fail if the committed temper-ts schema drifts from openapi.json.
#
# Regenerates the schema (via generate-temper-ts.sh) and diffs the result against what
# is committed — the local mirror of the temper-ts CI job's drift step.
#
# Unlike check-temper-rb-drift.sh this NEVER skips. That one needs a Docker daemon and
# exits 0 when it is absent (the test-ruby CI job being the never-skipping backstop);
# openapi-typescript needs only Node, so there is no environment in which we would
# rather guess. `cargo make check` therefore gains a gate that is a real gate.
#
# Usage: bash .github/scripts/check-temper-ts-drift.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GENERATED="clients/temper-ts/src/generated/schema.ts"

bash "$REPO_ROOT/.github/scripts/generate-temper-ts.sh"

if ! git -C "$REPO_ROOT" diff --exit-code -- "$GENERATED"; then
  echo >&2
  echo "ERROR: temper-ts's generated schema is out of date with openapi.json." >&2
  echo "       Run: cargo make openapi   (regenerates the spec, the gem, and the schema)" >&2
  echo "       then commit the regenerated $GENERATED" >&2
  exit 1
fi

echo "temper-ts generated schema is up to date with openapi.json"
