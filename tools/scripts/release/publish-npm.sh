#!/usr/bin/env bash
# tools/scripts/release/publish-npm.sh
#
# Build, then publish, the scoped TS client packages to the public npm registry
# (registry.npmjs.org). The two stages run in separate jobs of release.yml.
#
# Usage:
#   ./tools/scripts/release/publish-npm.sh VERSION --out DIR  [--package DIR]...
#   ./tools/scripts/release/publish-npm.sh VERSION --from DIR [--dry-run] [--package DIR]...
#
#   --out DIR   the BUILD stage: install, build and `npm pack` each package into DIR.
#               Holds no registry credential and publishes nothing.
#   --from DIR  the PUBLISH stage: push the tarballs in DIR. Installs and builds
#               nothing, and runs no package script (`--ignore-scripts`).
#
# Default packages: clients/temper-ts and clients/temper-telemetry-ts.
#
# WHY TWO STAGES. The publish job holds `id-token: write`, and any step in a job
# that holds it can mint the OIDC token npm accepts. `npm ci` runs every
# dependency's install scripts (esbuild's postinstall, through vitest, among
# them), so it runs in the build job, which cannot mint one. The build installs
# with `--ignore-scripts` as well: the build is `tsc`, which needs none of them.
# What the split does not stop is a compromised build tampering with the tarball
# the publish job then pushes; the scope and version guards below read the
# tarball's own manifest, but they cannot see inside its code.
#
# The package names are scoped (@tasker-systems/*) and published with
# --access public; a manifest name outside that scope would be a different
# package entirely and is refused, in both stages.
#
# Duplicate handling is the house pattern (loud, idempotent skip — the same
# behavior as publish-ruby.sh's versions-API probe and
# create-github-release.sh's "already exists"): the `npm view` probe decides,
# in both stages, and a re-run of a release for an existing tag skips rather
# than failing the whole release. The probe is unauthenticated on the public
# registry.
#
# Auth: npm trusted publishing. In CI, `npm publish --provenance` authenticates
# via the job's OIDC identity token (id-token: write) — no NPM_TOKEN secret
# exists or is wanted. The trusted publisher is registered on npmjs.com
# against this repository and the workflow whose job performs the push
# (release.yml — the identity claim names the job's OWN workflow file, not the
# chain's entry; a publisher registered against release-tag.yml is silently
# unauthorized at push). Locally, `npm login` plus a
# plain `npm publish --access public` is the bootstrap path for claiming a new
# package name before its trusted publisher is registered; this script's
# provenance flag is CI-only.

set -euo pipefail

VERSION="${1:-}"
DRY_RUN=false
OUT_DIR=""
FROM_DIR=""
shift || true
PACKAGES=()
while [[ $# -gt 0 ]]; do
    case $1 in
        --dry-run) DRY_RUN=true; shift ;;
        --out) OUT_DIR="$2"; shift 2 ;;
        --from) FROM_DIR="$2"; shift 2 ;;
        --package) PACKAGES+=("$2"); shift 2 ;;
        --package=*) PACKAGES+=("${1#*=}"); shift ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

if [[ -z "$VERSION" ]] || [[ -n "$OUT_DIR" && -n "$FROM_DIR" ]] || [[ -z "$OUT_DIR" && -z "$FROM_DIR" ]]; then
    echo "Usage: $0 VERSION (--out DIR | --from DIR [--dry-run]) [--package DIR]..." >&2
    exit 1
fi
if [[ -n "$OUT_DIR" && "$DRY_RUN" == "true" ]]; then
    echo "ERROR: --dry-run belongs to the publish stage (--from); the build stage publishes nothing." >&2
    exit 1
fi

REPO_ROOT="$(git rev-parse --show-toplevel)"
[[ ${#PACKAGES[@]} -gt 0 ]] || PACKAGES=("clients/temper-ts" "clients/temper-telemetry-ts")

# Provenance attests the publish via the job's OIDC identity — CI-only. A local
# run (the name-claiming bootstrap) publishes under the operator's own login.
PUBLISH_FLAGS=(--access public --no-fund --no-audit --ignore-scripts)
if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
    PUBLISH_FLAGS+=(--provenance)
fi

# The scope and version guards, shared by both stages: the build stage applies
# them to the source manifest, the publish stage to the manifest inside the
# tarball it is about to push.
check_manifest() {
    local where="$1" name="$2" declared="$3"
    if [[ "$name" != @tasker-systems/* ]]; then
        echo "ERROR: ${where} manifest name is '${name}' — these packages publish" >&2
        echo "       under the @tasker-systems/* scope. Scope the package first." >&2
        exit 1
    fi
    # Publishing a package whose contents disagree with the tag is worse than
    # failing (publish-ruby.sh's version-agreement guard, carried over).
    if [[ "$declared" != "$VERSION" ]]; then
        echo "ERROR: ${where} manifest version is ${declared}, but ${VERSION} was requested." >&2
        echo "       Run tools/scripts/release/update-versions.sh first." >&2
        exit 1
    fi
}

# npm view exits non-zero on the registry's E404 for a missing version, 0 when
# the version exists.
already_published() {
    npm view "$1@${VERSION}" version >/dev/null 2>&1
}

FAILED=0
for DIR in "${PACKAGES[@]}"; do
    PKG_DIR="${REPO_ROOT}/${DIR}"
    NAME=$(grep -m1 '"name"' "${PKG_DIR}/package.json" | sed -E 's/.*"name": "([^"]+)".*/\1/')
    # `npm pack` names the tarball after the manifest: scope's `@` dropped, `/` → `-`.
    TARBALL="$(echo "${NAME#@}" | tr '/' '-')-${VERSION}.tgz"

    # The build stage refuses a bad manifest before it calls npm at all; the
    # publish stage reads the manifest from the tarball, after its own probe.
    if [[ -n "$OUT_DIR" ]]; then
        DECLARED=$(grep -m1 '"version"' "${PKG_DIR}/package.json" | sed -E 's/.*"version": "([^"]+)".*/\1/')
        check_manifest "$DIR" "$NAME" "$DECLARED"
    fi

    if already_published "$NAME"; then
        echo "==> ${NAME}@${VERSION} is already published — nothing to do."
        continue
    fi

    if [[ -n "$OUT_DIR" ]]; then
        echo "==> build ${DIR} @ ${VERSION}"
        mkdir -p "$OUT_DIR"
        OUT_ABS="$(cd "$OUT_DIR" && pwd)"
        (
            cd "$PKG_DIR"
            npm ci --ignore-scripts --no-fund --no-audit
            npm run build
            npm pack --ignore-scripts --pack-destination "$OUT_ABS"
        )
        [[ -s "${OUT_ABS}/${TARBALL}" ]] || { echo "ERROR: npm pack did not produce ${TARBALL}" >&2; exit 1; }
        continue
    fi

    echo "==> publish ${NAME}@${VERSION} from ${FROM_DIR} (dry-run: ${DRY_RUN})"
    TARBALL_PATH="${FROM_DIR}/${TARBALL}"
    if [[ ! -s "$TARBALL_PATH" ]]; then
        echo "ERROR: ${TARBALL} is not in ${FROM_DIR}, and ${NAME}@${VERSION} is not published." >&2
        exit 1
    fi
    IN_TARBALL="$(tar -xOzf "$TARBALL_PATH" package/package.json)"
    check_manifest "$TARBALL" \
        "$(printf '%s' "$IN_TARBALL" | node -e 'process.stdout.write(JSON.parse(require("fs").readFileSync(0, "utf8")).name)')" \
        "$(printf '%s' "$IN_TARBALL" | node -e 'process.stdout.write(JSON.parse(require("fs").readFileSync(0, "utf8")).version)')"

    if [[ "$DRY_RUN" == "true" ]]; then
        npm publish "$TARBALL_PATH" --dry-run "${PUBLISH_FLAGS[@]}" | head -20
        continue
    fi

    if npm publish "$TARBALL_PATH" "${PUBLISH_FLAGS[@]}"; then
        echo "==> Published ${NAME}@${VERSION}"
    else
        echo "ERROR: npm publish failed for ${NAME}@${VERSION}" >&2
        FAILED=1
    fi
done

exit "$FAILED"
