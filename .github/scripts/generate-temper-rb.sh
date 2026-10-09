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
#   - the temper-rb Rakefile's `generate` task (regen from inside the gem)
#   - check-temper-rb-drift.sh (verify), which both `cargo make openapi-rb-drift` and
#     the `test-ruby` CI job's `rake drift` delegate to
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

# Escape `.` in path segments. `CGI.escape` leaves `.` alone, and the transport then resolves a
# `..` segment, so a path value of `..` climbs to the parent route instead of reaching the server as
# a literal. temperkb-client (Rust) encodes `.` for the same reason. The generator has no option for
# it, so every path substitution under api/ is rewritten here (python3, so this path still needs no
# Ruby). It fails if it finds no site, or if any `CGI.escape(` survives in a shape it did not
# rewrite: a generator upgrade that changes the shape must be looked at, not silently skipped.
python3 - "$REPO_ROOT/clients/temper-rb/lib/temper/generated/api" <<'PY'
import pathlib, re, sys
site = re.compile(r"CGI\.escape\(([a-z_][a-z0-9_]*)\.to_s\)")
total = 0
for path in sorted(pathlib.Path(sys.argv[1]).glob("*.rb")):
    src = path.read_text(encoding="utf-8")
    new, n = site.subn(r"CGI.escape(\1.to_s).gsub('.', '%2E')", src)
    if new.count("CGI.escape(") != n:
        sys.exit(f"generate-temper-rb: {path} has a `CGI.escape(` this patch does not recognise")
    total += n
    if n:
        path.write_text(new, encoding="utf-8")
if total == 0:
    sys.exit("generate-temper-rb: found no `CGI.escape(<param>.to_s)` path site to patch")
PY

# Refuse a path value of `.` or `..` outright, the same rule the Rust, TS and Python clients apply.
# The `.` escape above already keeps such a value literal on this transport; the refusal is the
# second layer, so no client sends a request the others would refuse. It sits in the one URL
# builder every operation calls, and decodes each segment so the escaped form is caught too.
# Templates carry no dot segments, so only a substituted value can trip it. Exactly one anchor.
python3 - "$REPO_ROOT/clients/temper-rb/lib/temper/generated/api_client.rb" <<'PY'
import sys
path = sys.argv[1]
src = open(path, encoding="utf-8").read()
old = "      path = \"/#{path}\".gsub(/\\/+/, '/')\n"
new = old + (
    "      if path.split('/').any? { |segment| ['.', '..'].include?(CGI.unescape(segment)) }\n"
    "        raise ArgumentError, \"a path value of `.` or `..` would address the parent route: #{path}\"\n"
    "      end\n"
)
if src.count(old) != 1:
    sys.exit(f"generate-temper-rb: expected exactly one path-normalising line in {path}, found {src.count(old)}")
open(path, "w", encoding="utf-8").write(src.replace(old, new))
PY
