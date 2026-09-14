#!/usr/bin/env bash
# tools/scripts/release/publish-ruby.sh
#
# Build and publish the temper-rb source gem to rubygems.org.
#
# Usage:
#   ./tools/scripts/release/publish-ruby.sh VERSION [--dry-run]
#
# There is no native extension, so there is no platform gem matrix and no
# cross-compile: one source gem, and no cargo on the install box. That was the
# whole point of generating a client instead of writing magnus bindings.
#
# Auth: OIDC trusted publishing. In CI, rubygems/configure-rubygems-credentials
# mints short-lived credentials from the job's identity token — no API key
# secret exists or is wanted. The trusted publisher is registered on
# rubygems.org against this repository and the CHAIN'S ENTRY workflow
# (release-tag.yml — the workflow claim names the entry, not the job's file).
# Locally, an existing ~/.gem/credentials from `gem login` works for a manual
# push; this script is not the local path.
#
# Duplicate handling: rubygems.org HAS a versions API, so the probe is a real
# pre-push check — a version already listed is a loud, idempotent skip (the
# same behavior as publish-npm.sh's `npm view` probe and
# create-github-release.sh's "already exists"). A push failure after a clean
# probe is real and stops the release.

set -euo pipefail

VERSION="${1:-}"
DRY_RUN=false
[[ "${2:-}" == "--dry-run" ]] && DRY_RUN=true

if [[ -z "$VERSION" ]]; then
    echo "Usage: $0 VERSION [--dry-run]" >&2
    exit 1
fi

GEM_NAME="temper-rb"
VERSIONS_API="https://rubygems.org/api/v1/versions/${GEM_NAME}.json"
REPO_ROOT="$(git rev-parse --show-toplevel)"
GEM_DIR="${REPO_ROOT}/clients/temper-rb"

echo "==> Publishing ${GEM_NAME} ${VERSION} to rubygems.org (dry-run: ${DRY_RUN})"

# The version the gemspec will actually stamp comes from lib/temper/version.rb.
# Publishing a gem whose contents disagree with the tag is worse than failing.
DECLARED="$(grep -oE "VERSION = '[^']+'" "${GEM_DIR}/lib/temper/version.rb" | cut -d"'" -f2)"
if [[ "$DECLARED" != "$VERSION" ]]; then
    echo "ERROR: Temper::VERSION is ${DECLARED}, but ${VERSION} was requested." >&2
    echo "       Update clients/temper-rb/lib/temper/version.rb first." >&2
    exit 1
fi

# Duplicate probe BEFORE the build: rubygems.org's versions API answers
# unauthenticated, so a re-cut release skips loudly and skips cheaply. A 404
# (gem or version absent) means not yet published.
if curl -sf "$VERSIONS_API" | grep -q "\"number\":\"${VERSION}\""; then
    echo "==> ${GEM_NAME} ${VERSION} is already published — nothing to do."
    exit 0
fi

cd "$GEM_DIR"
gem build "${GEM_NAME}.gemspec"
GEM_FILE="${GEM_NAME}-${VERSION}.gem"

if [[ "$DRY_RUN" == "true" ]]; then
    echo "==> [dry-run] would push ${GEM_FILE} to rubygems.org"
    gem specification "$GEM_FILE" | head -20
    rm -f "$GEM_FILE"
    exit 0
fi

if gem push "$GEM_FILE"; then
    echo "==> Published ${GEM_FILE}"
    exit 0
fi

echo "ERROR: gem push failed for ${GEM_NAME} ${VERSION}:" >&2
exit 1
