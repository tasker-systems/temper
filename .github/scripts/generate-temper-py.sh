#!/usr/bin/env bash
#
# Regenerate clients/temper-py/temper/generated/** from the repo-root openapi.json.
#
# The generated package is a committed *product of openapi.json* (itself a product of
# the Axum router), so a new field on a response DTO leaves the client stale — the
# same class of drift the openapi-check gate guards for the spec itself, and the same
# one generate-temper-rb.sh guards for the gem.
#
# This script is the single source of truth for the generator pin + parameters.
# Invoked three ways, so the runner invocation lives here rather than in any caller:
#   - `cargo make openapi` / `cargo make openapi-py` (local dev, regen)
#   - `cargo make openapi-py-drift` → check-temper-py-drift.sh (local dev, verify)
#   - the `test-python` CI job's drift step, which runs that same check script
#
# Python is NOT required — this path is deliberately toolchain-light so a Rust dev
# who changed a DTO can regenerate the client without standing up its venv. It runs
# the pinned generator one of two ways, preferring whichever the host has:
#   1. Docker (the openapi-generator image) — the CI path when a daemon is present.
#   2. Java + the pinned generator jar from Maven Central — the Docker-less
#      fallback (web sessions, sandboxes, GitHub runners, which ship a JDK).
# Same pinned VERSION → identical output. The jar is cached under
# ${OPENAPI_GENERATOR_JAR_CACHE:-~/.cache/temper}.
#
# Usage: bash .github/scripts/generate-temper-py.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SPEC="$REPO_ROOT/openapi.json"

# Pinned deliberately, and pinned to the SAME version generate-temper-rb.sh uses.
# `latest` resolves to a moving *-SNAPSHOT build; a moving generator makes the drift
# gate fail on days when nothing in this repo changed. The Docker tag and the jar
# coordinate MUST name the same generator version, or the two host paths would emit
# divergent packages.
GENERATOR_VERSION="7.23.0"
GENERATOR_IMAGE="openapitools/openapi-generator-cli:v${GENERATOR_VERSION}"

if [ ! -s "$SPEC" ]; then
  echo "ERROR: openapi.json is missing or empty — run: cargo make openapi" >&2
  exit 1
fi

# packageVersion tracks the contract's info.version so the generated package's
# __version__ stays in step with the spec — the analogue of the gem's gemVersion,
# and what `temper.CONTRACT_VERSION` aliases. python3 ships on the CI runner and on
# dev machines.
VERSION="$(python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['info']['version'])" "$SPEC")"

# `packageName=temper.generated` puts the generated tree at temper/generated/, one
# level under the hand-written `temper` package — the same shape as the gem's
# lib/temper/generated/. The generator therefore also wants to write an empty
# temper/__init__.py; .openapi-generator-ignore stops it.
#
# `library=urllib3` is the generator default and the synchronous one. The async
# libraries (asyncio, httpx) would give the client a second, divergent call surface
# for no caller that exists: every temper Python consumer today is a synchronous
# worker, and the gem it is a sibling of is synchronous too.
GEN_PROPS="packageName=temper.generated,projectName=temper-py,packageVersion=$VERSION"

# The generate args are identical across host paths; only the file-path prefix
# and the runner differ. Shared here so the two branches cannot drift.
run_with_docker() {
  # --user keeps the emitted files owned by the invoking user. Without it the
  # container writes as root on Linux (CI), and the drift gate cannot read them.
  docker run --rm \
    --user "$(id -u):$(id -g)" \
    -v "$REPO_ROOT:/local" \
    "$GENERATOR_IMAGE" \
    generate \
    -i /local/openapi.json \
    -g python \
    --library=urllib3 \
    -o /local/clients/temper-py \
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
    -g python \
    --library=urllib3 \
    -o "$REPO_ROOT/clients/temper-py" \
    --additional-properties="$GEN_PROPS"
}

# Prefer Docker (parity with the gem's CI path); fall back to a Java + pinned-jar
# run when the daemon is unavailable. Both pin the same GENERATOR_VERSION, so the
# emitted package is identical either way and the drift gate stays honest.
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
