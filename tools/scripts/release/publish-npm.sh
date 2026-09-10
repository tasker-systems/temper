#!/usr/bin/env bash
# tools/scripts/release/publish-npm.sh
#
# Build and publish the scoped TS client packages to GitHub Packages npm.
#
# Usage:
#   ./tools/scripts/release/publish-npm.sh VERSION [--dry-run] [--package DIR]...
#
# Default packages: clients/temper-ts and clients/temper-telemetry-ts.
#
# GitHub Packages' npm registry requires scoped names, so a package whose
# manifest name is not @tasker-systems/* is refused — publishing an unscoped
# name would target a different registry entirely.
#
# Duplicate handling is the house pattern (loud, idempotent skip — the same
# behavior as publish-ruby.sh's "already published" and create-github-release.sh's
# "already exists"): the `npm view` probe decides, and a re-run of a release for
# an existing tag skips rather than failing the whole release. GitHub Packages'
# own duplicate-push semantics are NOT relied on; the probe is load-bearing.
#
# Auth: both `npm view` and `npm publish` against npm.pkg.github.com need a
# token. CI supplies NODE_AUTH_TOKEN and an ~/.npmrc carrying
# `//npm.pkg.github.com/:_authToken`; locally, dry-run skips the requirement.

set -euo pipefail

VERSION="${1:-}"
DRY_RUN=false
shift || true
PACKAGES=()
while [[ $# -gt 0 ]]; do
    case $1 in
        --dry-run) DRY_RUN=true; shift ;;
        --package) PACKAGES+=("$2"); shift 2 ;;
        --package=*) PACKAGES+=("${1#*=}"); shift ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

if [[ -z "$VERSION" ]]; then
    echo "Usage: $0 VERSION [--dry-run] [--package DIR]..." >&2
    exit 1
fi

REPO_ROOT="$(git rev-parse --show-toplevel)"
[[ ${#PACKAGES[@]} -gt 0 ]] || PACKAGES=("clients/temper-ts" "clients/temper-telemetry-ts")

if [[ "$DRY_RUN" != "true" && -z "${GITHUB_ACTIONS:-}" ]]; then
    : "${NODE_AUTH_TOKEN:?NODE_AUTH_TOKEN is required for GitHub Packages publishing}"
fi

FAILED=0
for DIR in "${PACKAGES[@]}"; do
    PKG_DIR="${REPO_ROOT}/${DIR}"
    echo "==> ${DIR} @ ${VERSION} (dry-run: ${DRY_RUN})"

    NAME=$(grep -m1 '"name"' "${PKG_DIR}/package.json" | sed -E 's/.*"name": "([^"]+)".*/\1/')
    if [[ "$NAME" != @tasker-systems/* ]]; then
        echo "ERROR: ${DIR} manifest name is '${NAME}' — GitHub Packages npm requires an" >&2
        echo "       @tasker-systems/* scope. Scope the package first." >&2
        exit 1
    fi

    # The version npm would stamp must agree with the requested one — publishing
    # a package whose contents disagree with the tag is worse than failing
    # (publish-ruby.sh's version-agreement guard, carried over).
    DECLARED=$(grep -m1 '"version"' "${PKG_DIR}/package.json" | sed -E 's/.*"version": "([^"]+)".*/\1/')
    if [[ "$DECLARED" != "$VERSION" ]]; then
        echo "ERROR: ${DIR} manifest version is ${DECLARED}, but ${VERSION} was requested." >&2
        echo "       Run tools/scripts/release/update-versions.sh first." >&2
        exit 1
    fi

    # Duplicate probe BEFORE the build: a re-cut release skips loudly and skips
    # cheaply (npm view exits non-zero on the registry's E404 for a missing
    # version, exit 0 when the version exists).
    if npm view "${NAME}@${VERSION}" version >/dev/null 2>&1; then
        echo "==> ${NAME}@${VERSION} is already published — nothing to do."
        continue
    fi

    (
        cd "$PKG_DIR"
        npm ci --no-fund --no-audit
        npm run build
    )

    if [[ "$DRY_RUN" == "true" ]]; then
        echo "==> [dry-run] would publish ${NAME}@${VERSION}"
        (cd "$PKG_DIR" && npm publish --dry-run --no-fund --no-audit) | head -20
        continue
    fi

    if (cd "$PKG_DIR" && npm publish --no-fund --no-audit); then
        echo "==> Published ${NAME}@${VERSION}"
    else
        echo "ERROR: npm publish failed for ${NAME}@${VERSION}" >&2
        FAILED=1
    fi
done

exit "$FAILED"
