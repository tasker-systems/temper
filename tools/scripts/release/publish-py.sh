#!/usr/bin/env bash
# tools/scripts/release/publish-py.sh
#
# Build and publish the temperkb-py wheel + sdist to pypi.org.
#
# Usage:
#   ./tools/scripts/release/publish-py.sh VERSION [--dry-run]
#
# The DISTRIBUTION is temperkb-py; the IMPORT package stays `temper`. PyPI has
# no scopes, and both natural names were taken by unrelated projects long
# before this one (temper-py — a TEMPer USB-device reader; temper — an HTML
# DSL), so the distribution carries the kb. Nothing under temper/ changes with
# the name: hatchling maps packages = ["temper"] regardless.
#
# Auth: PyPI trusted publishing. `uv publish` exchanges the CI job's OIDC
# identity token for a short-lived upload token (--trusted-publishing
# automatic) — no API token secret exists or is wanted. The trusted publisher
# is registered on pypi.org against this repository and the workflow whose job
# performs the push (release.yml — the OIDC claim names the job's OWN workflow
# file; a publisher registered against the chain's entry release-tag.yml is
# silently unauthorized at push, the same refusal publish-ruby.sh's
# registration hit first). Unlike npm, PyPI attaches publishers to
# NOT-YET-EXISTING projects ("pending publisher"), so a new name is
# pre-registered on pypi.org and the first CI publish claims it — no local
# bootstrap upload exists or is needed. Locally, an emergency re-push can set
# UV_PUBLISH_TOKEN; this script is not the path for that.
#
# Duplicate handling: pypi.org HAS a JSON API, so the probe is a real
# pre-build check — a version already listed is a loud, idempotent skip (the
# same behavior as publish-ruby.sh's versions-API probe and
# create-github-release.sh's "already exists"). A publish failure after a
# clean probe is real and stops the release.

set -euo pipefail

VERSION="${1:-}"
DRY_RUN=false
[[ "${2:-}" == "--dry-run" ]] && DRY_RUN=true

if [[ -z "$VERSION" ]]; then
    echo "Usage: $0 VERSION [--dry-run]" >&2
    exit 1
fi

DIST_NAME="temperkb-py"
VERSIONS_API="https://pypi.org/pypi/${DIST_NAME}/json"
REPO_ROOT="$(git rev-parse --show-toplevel)"
PY_DIR="${REPO_ROOT}/clients/temper-py"

echo "==> Publishing ${DIST_NAME} ${VERSION} to pypi.org (dry-run: ${DRY_RUN})"

# The version hatchling will stamp comes from temper/version.py, read (never
# imported) at build time. Publishing a distribution whose contents disagree
# with the tag is worse than failing (publish-ruby.sh's version-agreement
# guard, carried over).
DECLARED="$(grep -oE '__version__ = "[^"]+"' "${PY_DIR}/temper/version.py" | cut -d'"' -f2)"
if [[ "$DECLARED" != "$VERSION" ]]; then
    echo "ERROR: temper.__version__ is ${DECLARED}, but ${VERSION} was requested." >&2
    echo "       Update clients/temper-py/temper/version.py first." >&2
    exit 1
fi

# Duplicate probe BEFORE the build: pypi.org's JSON API answers
# unauthenticated, so a re-cut release skips loudly and skips cheaply. The
# version is captured and compared afterward — a 404 (project or version
# absent) or any probe failure yields an empty capture, and empty != VERSION
# means not yet published. No grep-in-pipeline: the early-exit race
# publish-ruby.sh's probe works around doesn't exist on this shape.
PUBLISHED="$(curl -sf "$VERSIONS_API" | python3 -c 'import json,sys; print(json.load(sys.stdin)["info"]["version"])' 2>/dev/null || true)"
if [[ "$PUBLISHED" == "$VERSION" ]]; then
    echo "==> ${DIST_NAME} ${VERSION} is already published — nothing to do."
    exit 0
fi

if ! command -v uv > /dev/null 2>&1; then
    echo "ERROR: uv is not installed — it builds and publishes the distributions." >&2
    exit 1
fi

cd "$PY_DIR"
uv build

if [[ "$DRY_RUN" == "true" ]]; then
    echo "==> [dry-run] would publish clients/temper-py/dist/* to pypi.org"
    ls -1 dist
    rm -rf dist
    exit 0
fi

# automatic is uv's default; it is spelled out for the same reason
# publish-npm.sh spells out --provenance: the OIDC-only auth story is visible
# where the publish happens. Outside Actions it fails loudly rather than
# falling back to anything unauthenticated.
if uv publish --trusted-publishing automatic; then
    echo "==> Published ${DIST_NAME} ${VERSION}"
    exit 0
fi

echo "ERROR: uv publish failed for ${DIST_NAME} ${VERSION}:" >&2
exit 1
