#!/usr/bin/env bash
# tools/scripts/release/publish-ruby.sh
#
# Build and publish the temper-rb source gem to RubyGems.
#
# Usage:
#   ./tools/scripts/release/publish-ruby.sh VERSION [--dry-run]
#
# There is no native extension, so there is no platform gem matrix and no
# cross-compile: one source gem, and no cargo on the install box. That was the
# whole point of generating a client instead of writing magnus bindings.
#
# Requires GEM_HOST_API_KEY for local publishing (skipped in dry-run and in CI,
# where the OIDC-provisioned credential is already in the environment).

set -euo pipefail

VERSION="${1:-}"
DRY_RUN=false
[[ "${2:-}" == "--dry-run" ]] && DRY_RUN=true

if [[ -z "$VERSION" ]]; then
    echo "Usage: $0 VERSION [--dry-run]" >&2
    exit 1
fi

GEM_NAME="temper-rb"
REPO_ROOT="$(git rev-parse --show-toplevel)"
GEM_DIR="${REPO_ROOT}/clients/temper-rb"

if [[ "$DRY_RUN" != "true" && -z "${GITHUB_ACTIONS:-}" ]]; then
    : "${GEM_HOST_API_KEY:?GEM_HOST_API_KEY is required for RubyGems publishing}"
fi

echo "==> Publishing ${GEM_NAME} ${VERSION} (dry-run: ${DRY_RUN})"

# The version the gemspec will actually stamp comes from lib/temper/version.rb.
# Publishing a gem whose contents disagree with the tag is worse than failing.
DECLARED="$(grep -oE "VERSION = '[^']+'" "${GEM_DIR}/lib/temper/version.rb" | cut -d"'" -f2)"
if [[ "$DECLARED" != "$VERSION" ]]; then
    echo "ERROR: Temper::VERSION is ${DECLARED}, but ${VERSION} was requested." >&2
    echo "       Update clients/temper-rb/lib/temper/version.rb first." >&2
    exit 1
fi

# Idempotency guard: never attempt to re-push an existing version. Query
# RubyGems before building, as tasker-core's publish-ruby.sh does.
if curl -sf "https://rubygems.org/api/v1/versions/${GEM_NAME}.json" 2>/dev/null \
    | grep -q "\"number\":\"${VERSION}\""; then
    echo "==> ${GEM_NAME} ${VERSION} is already published — nothing to do."
    exit 0
fi

cd "$GEM_DIR"
gem build "${GEM_NAME}.gemspec"
GEM_FILE="${GEM_NAME}-${VERSION}.gem"

if [[ "$DRY_RUN" == "true" ]]; then
    echo "==> [dry-run] would publish ${GEM_FILE}"
    gem specification "$GEM_FILE" | head -20
    rm -f "$GEM_FILE"
    exit 0
fi

gem push "$GEM_FILE"
echo "==> Published ${GEM_FILE}"
