#!/usr/bin/env bash
# tools/scripts/release/publish-py.sh
#
# Build, then publish, the temperkb-py wheel + sdist to pypi.org. The two
# stages run in separate jobs of release.yml.
#
# Usage:
#   ./tools/scripts/release/publish-py.sh VERSION --out DIR
#   ./tools/scripts/release/publish-py.sh VERSION --from DIR [--dry-run]
#
#   --out DIR   the BUILD stage: `uv build` into DIR. Holds no registry credential.
#   --from DIR  the PUBLISH stage: `uv publish` the two distributions in DIR, and
#               nothing else. Builds nothing.
#
# WHY TWO STAGES. The publish job holds `id-token: write`, and any step in a job
# that holds it can mint the OIDC token PyPI accepts. `uv build` runs the build
# backend, which is third-party code, so it runs in the build job, which cannot
# mint one. The backend is also pinned by hash (clients/temper-py/
# build-constraints.txt), because the split does not stop a compromised build
# tampering with the distributions the publish job then pushes.
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
# Duplicate handling: pypi.org HAS a JSON API, so the probe is a real check in
# both stages — a version already listed is a loud, idempotent skip (the
# same behavior as publish-ruby.sh's versions-API probe and
# create-github-release.sh's "already exists"). A publish failure after a
# clean probe is real and stops the release.

set -euo pipefail

VERSION="${1:-}"
DRY_RUN=false
OUT_DIR=""
FROM_DIR=""
shift || true
while [[ $# -gt 0 ]]; do
    case $1 in
        --dry-run) DRY_RUN=true; shift ;;
        --out) OUT_DIR="$2"; shift 2 ;;
        --from) FROM_DIR="$2"; shift 2 ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

if [[ -z "$VERSION" ]] || [[ -n "$OUT_DIR" && -n "$FROM_DIR" ]] || [[ -z "$OUT_DIR" && -z "$FROM_DIR" ]]; then
    echo "Usage: $0 VERSION (--out DIR | --from DIR [--dry-run])" >&2
    exit 1
fi
if [[ -n "$OUT_DIR" && "$DRY_RUN" == "true" ]]; then
    echo "ERROR: --dry-run belongs to the publish stage (--from); the build stage publishes nothing." >&2
    exit 1
fi

DIST_NAME="temperkb-py"
REPO_ROOT="$(git rev-parse --show-toplevel)"
PY_DIR="${REPO_ROOT}/clients/temper-py"
# What `uv build` names the two distributions (PEP 625 / PEP 427 normalize `-` to `_`).
SDIST="temperkb_py-${VERSION}.tar.gz"
WHEEL="temperkb_py-${VERSION}-py3-none-any.whl"

# The per-version endpoint answers 200 for a published version and 404 for any
# other, including a version older than the latest — so a re-run of an older
# tag is still a skip. Any probe failure reads as not published, and the
# publish itself is then the check.
if curl -sf -o /dev/null "https://pypi.org/pypi/${DIST_NAME}/${VERSION}/json"; then
    echo "==> ${DIST_NAME} ${VERSION} is already published — nothing to do."
    exit 0
fi

if ! command -v uv > /dev/null 2>&1; then
    echo "ERROR: uv is not installed — it builds and publishes the distributions." >&2
    exit 1
fi

if [[ -n "$OUT_DIR" ]]; then
    echo "==> build ${DIST_NAME} ${VERSION} into ${OUT_DIR}"
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
    mkdir -p "$OUT_DIR"
    OUT_ABS="$(cd "$OUT_DIR" && pwd)"
    (cd "$PY_DIR" && uv build --build-constraint build-constraints.txt --require-hashes --out-dir "$OUT_ABS")
    for f in "$SDIST" "$WHEEL"; do
        [[ -s "${OUT_ABS}/${f}" ]] || { echo "ERROR: uv build did not produce ${f}" >&2; exit 1; }
    done
    exit 0
fi

echo "==> publish ${DIST_NAME} ${VERSION} from ${FROM_DIR} (dry-run: ${DRY_RUN})"
# Exactly the two distributions, named for this version: the filenames carry the
# name and version PyPI checks against the metadata inside, and anything else in
# the directory would be pushed too.
ACTUAL="$(cd "$FROM_DIR" && ls -1 | sort | tr '\n' ' ')"
EXPECTED="$(printf '%s\n' "$SDIST" "$WHEEL" | sort | tr '\n' ' ')"
if [[ "$ACTUAL" != "$EXPECTED" ]]; then
    echo "ERROR: ${FROM_DIR} holds '${ACTUAL% }', expected exactly '${EXPECTED% }'." >&2
    exit 1
fi

if [[ "$DRY_RUN" == "true" ]]; then
    echo "==> [dry-run] would publish ${SDIST} and ${WHEEL} to pypi.org"
    exit 0
fi

# automatic is uv's default; it is spelled out for the same reason
# publish-npm.sh spells out --provenance: the OIDC-only auth story is visible
# where the publish happens. Outside Actions it fails loudly rather than
# falling back to anything unauthenticated.
if uv publish --trusted-publishing automatic "${FROM_DIR}/${SDIST}" "${FROM_DIR}/${WHEEL}"; then
    echo "==> Published ${DIST_NAME} ${VERSION}"
    exit 0
fi

echo "ERROR: uv publish failed for ${DIST_NAME} ${VERSION}:" >&2
exit 1
