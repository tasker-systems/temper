#!/usr/bin/env bash
# tools/scripts/release/publish-ruby.sh
#
# Build and publish the temper-rb source gem to GitHub Packages
# (rubygems.pkg.github.com/tasker-systems).
#
# Usage:
#   ./tools/scripts/release/publish-ruby.sh VERSION [--dry-run]
#
# There is no native extension, so there is no platform gem matrix and no
# cross-compile: one source gem, and no cargo on the install box. That was the
# whole point of generating a client instead of writing magnus bindings.
#
# Auth: `gem push` to GitHub Packages needs a credentials entry named `github`
# holding a token with write:packages. When GITHUB_TOKEN is set (CI supplies
# github.token) this script writes ~/.gem/credentials itself, chmod 600;
# locally, an existing credentials file is kept and GITHUB_TOKEN is required
# only if there is none.
#
# Duplicate handling: GitHub Packages RubyGems has no versions-list API, so
# there is no pre-push probe (the rubygems.org one this script once carried
# does not exist on that host). The push itself is the detector: a refusal
# naming an already-published version is a loud, idempotent skip (the same
# behavior as create-github-release.sh's "already exists"), any other failure
# exits non-zero. The refusal TEXT is codified from GitHub's documented
# duplicate-push response and is the one bit to re-check at the first real
# publish.

set -euo pipefail

VERSION="${1:-}"
DRY_RUN=false
[[ "${2:-}" == "--dry-run" ]] && DRY_RUN=true

if [[ -z "$VERSION" ]]; then
    echo "Usage: $0 VERSION [--dry-run]" >&2
    exit 1
fi

GEM_NAME="temper-rb"
GEM_HOST="https://rubygems.pkg.github.com/tasker-systems"
REPO_ROOT="$(git rev-parse --show-toplevel)"
GEM_DIR="${REPO_ROOT}/clients/temper-rb"
CREDENTIALS_FILE="${HOME}/.gem/credentials"

echo "==> Publishing ${GEM_NAME} ${VERSION} to ${GEM_HOST} (dry-run: ${DRY_RUN})"

# The version the gemspec will actually stamp comes from lib/temper/version.rb.
# Publishing a gem whose contents disagree with the tag is worse than failing.
DECLARED="$(grep -oE "VERSION = '[^']+'" "${GEM_DIR}/lib/temper/version.rb" | cut -d"'" -f2)"
if [[ "$DECLARED" != "$VERSION" ]]; then
    echo "ERROR: Temper::VERSION is ${DECLARED}, but ${VERSION} was requested." >&2
    echo "       Update clients/temper-rb/lib/temper/version.rb first." >&2
    exit 1
fi

# gem push selects the credentials entry by --key; without it the push goes to
# rubygems.org, which is exactly the wrong place. Never push without the host
# and key pinned.
if [[ "$DRY_RUN" != "true" ]]; then
    if [[ ! -f "$CREDENTIALS_FILE" ]]; then
        : "${GITHUB_TOKEN:?GITHUB_TOKEN is required to publish to GitHub Packages}"
        mkdir -p "$(dirname "$CREDENTIALS_FILE")"
        printf -- "---\n:github: %s\n" "\"${GITHUB_TOKEN}\"" > "$CREDENTIALS_FILE"
        chmod 600 "$CREDENTIALS_FILE"
        echo "==> Wrote ${CREDENTIALS_FILE} from GITHUB_TOKEN"
    fi
fi

cd "$GEM_DIR"
gem build "${GEM_NAME}.gemspec"
GEM_FILE="${GEM_NAME}-${VERSION}.gem"

if [[ "$DRY_RUN" == "true" ]]; then
    echo "==> [dry-run] would push ${GEM_FILE} to ${GEM_HOST}"
    gem specification "$GEM_FILE" | head -20
    rm -f "$GEM_FILE"
    exit 0
fi

PUSH_LOG="$(mktemp)"
trap 'rm -f "$PUSH_LOG"' EXIT
if gem push --key github --host "$GEM_HOST" "$GEM_FILE" 2>&1 | tee "$PUSH_LOG"; then
    echo "==> Published ${GEM_FILE}"
    exit 0
fi

# A push failure naming an existing version is the duplicate detector firing —
# loud, and a skip rather than an error, so a re-cut release stays idempotent.
# Any other failure is real and stops the release.
if grep -qiE "already been published|version .* already exists" "$PUSH_LOG"; then
    echo "==> ${GEM_NAME} ${VERSION} is already published — nothing to do."
    exit 0
fi

echo "ERROR: gem push failed for ${GEM_NAME} ${VERSION}:" >&2
cat "$PUSH_LOG" >&2
exit 1
