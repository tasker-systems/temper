#!/usr/bin/env bash
#
# Regenerate clients/temper-rb/lib/temper/generated/** from the repo-root openapi.json.
#
# The generated core is a committed *product of openapi.json* (itself a product of
# the Axum router), so a new field on a response DTO leaves the gem stale — the
# same class of drift the openapi-check gate guards for the spec itself.
#
# This script is the single source of truth for the generator pin + parameters.
# Invoked three ways, so the docker invocation lives here rather than in any caller:
#   - `cargo make openapi` / `cargo make openapi-rb` (local dev, regen)
#   - `cargo make openapi-rb-drift` → check-temper-rb-drift.sh (local dev, verify)
#   - the temper-rb Rakefile's `generate` task, which the `test-ruby` CI job drives
#     via `rake drift`
#
# Requires Docker (the openapi-generator image). Ruby is NOT required — this path
# is deliberately toolchain-light so a Rust dev who changed a DTO can regenerate
# the gem without standing up the gem's Ruby 3.4 bundle.
#
# Usage: bash .github/scripts/generate-temper-rb.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SPEC="$REPO_ROOT/openapi.json"

# Pinned deliberately. `latest` resolves to a moving *-SNAPSHOT build; a moving
# generator makes the drift gate fail on days when nothing in this repo changed.
GENERATOR_IMAGE="openapitools/openapi-generator-cli:v7.23.0"

if [ ! -s "$SPEC" ]; then
  echo "ERROR: openapi.json is missing or empty — run: cargo make openapi" >&2
  exit 1
fi

# gemVersion tracks the contract's info.version so the generated gemspec/version
# stay in step with the spec. python3 ships on the CI runner and on dev machines.
VERSION="$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['info']['version'])" "$SPEC")"

# --user keeps the emitted files owned by the invoking user. Without it the
# container writes as root on Linux (CI), and the drift gate cannot read them.
docker run --rm \
  --user "$(id -u):$(id -g)" \
  -v "$REPO_ROOT:/local" \
  "$GENERATOR_IMAGE" \
  generate \
  -i /local/openapi.json \
  -g ruby \
  --library=faraday \
  -o /local/clients/temper-rb \
  --additional-properties="gemName=temper/generated,moduleName=Temper::Generated,gemVersion=$VERSION"
