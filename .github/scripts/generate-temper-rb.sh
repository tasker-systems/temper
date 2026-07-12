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
# Ruby is NOT required — this path is deliberately toolchain-light so a Rust dev
# who changed a DTO can regenerate the gem without standing up the gem's Ruby 3.4
# bundle. It runs the pinned generator one of two ways, preferring whichever the
# host has:
#   1. Docker (the openapi-generator image) — the CI path.
#   2. Java + the pinned generator jar from Maven Central — the Docker-less
#      fallback (web sessions, sandboxes). Same pinned VERSION → identical output.
# The jar is cached under ${OPENAPI_GENERATOR_JAR_CACHE:-~/.cache/temper}.
#
# Usage: bash .github/scripts/generate-temper-rb.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SPEC="$REPO_ROOT/openapi.json"

# Pinned deliberately. `latest` resolves to a moving *-SNAPSHOT build; a moving
# generator makes the drift gate fail on days when nothing in this repo changed.
# The Docker tag and the jar coordinate MUST name the same generator version, or
# the two host paths would emit divergent gems.
GENERATOR_VERSION="7.23.0"
GENERATOR_IMAGE="openapitools/openapi-generator-cli:v${GENERATOR_VERSION}"

if [ ! -s "$SPEC" ]; then
  echo "ERROR: openapi.json is missing or empty — run: cargo make openapi" >&2
  exit 1
fi

# gemVersion tracks the contract's info.version so the generated gemspec/version
# stay in step with the spec. python3 ships on the CI runner and on dev machines.
VERSION="$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['info']['version'])" "$SPEC")"

# The generate args are identical across host paths; only the file-path prefix
# and the runner differ. Shared here so the two branches cannot drift.
GEN_PROPS="gemName=temper/generated,moduleName=Temper::Generated,gemVersion=$VERSION"

run_with_docker() {
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
    --additional-properties="$GEN_PROPS"
}

run_with_jar() {
  local cache_dir="${OPENAPI_GENERATOR_JAR_CACHE:-$HOME/.cache/temper}"
  local jar="$cache_dir/openapi-generator-cli-${GENERATOR_VERSION}.jar"
  local url="https://repo1.maven.org/maven2/org/openapitools/openapi-generator-cli/${GENERATOR_VERSION}/openapi-generator-cli-${GENERATOR_VERSION}.jar"

  if [ ! -s "$jar" ]; then
    echo "  fetching openapi-generator-cli ${GENERATOR_VERSION} jar → $jar" >&2
    mkdir -p "$cache_dir"
    curl -fsSL -o "$jar" "$url"
  fi

  java -jar "$jar" \
    generate \
    -i "$SPEC" \
    -g ruby \
    --library=faraday \
    -o "$REPO_ROOT/clients/temper-rb" \
    --additional-properties="$GEN_PROPS"
}

# Prefer Docker (the CI path); fall back to a Java + pinned-jar run when the
# daemon is unavailable. Both pin the same GENERATOR_VERSION, so the emitted gem
# is identical either way and the drift gate stays honest.
if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
  run_with_docker
elif command -v java >/dev/null 2>&1; then
  echo "  Docker unavailable — using the Java + pinned-jar fallback" >&2
  run_with_jar
else
  echo "ERROR: need either a running Docker daemon or a Java runtime to run" >&2
  echo "       openapi-generator ${GENERATOR_VERSION} (both were absent)." >&2
  exit 1
fi
