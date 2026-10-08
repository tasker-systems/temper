#!/usr/bin/env bash
# tools/scripts/release/publish-ruby.sh
#
# Build, then publish, the temper-rb source gem to rubygems.org. The two stages
# run in separate jobs of release.yml.
#
# Usage:
#   ./tools/scripts/release/publish-ruby.sh VERSION --out DIR
#   ./tools/scripts/release/publish-ruby.sh VERSION --from DIR [--dry-run]
#
#   --out DIR   the BUILD stage: `gem build` into DIR. Holds no registry credential.
#   --from DIR  the PUBLISH stage: `gem push` the gem in DIR. Builds nothing.
#
# WHY TWO STAGES, for a lane that installs nothing: `gem build` evaluates only
# this repo's gemspec, so the Ruby lane runs no third-party build code today.
# It is split anyway so every registry lane has one shape — the job that can
# mint the OIDC token pushes bytes and runs nothing else — and so a later
# `bundle install` in the build cannot land beside the credential unnoticed.
#
# There is no native extension, so there is no platform gem matrix and no
# cross-compile: one source gem, and no cargo on the install box. That was the
# whole point of generating a client instead of writing magnus bindings.
#
# Auth: OIDC trusted publishing. In CI, rubygems/configure-rubygems-credentials
# mints short-lived credentials from the job's identity token — no API key
# secret exists or is wanted. The trusted publisher is registered on
# rubygems.org against this repository and the workflow whose job performs the
# push (release.yml — the identity claim names the job's OWN workflow file, not
# the chain's entry; a publisher registered against release-tag.yml is
# silently unauthorized at push).
# Locally, an existing ~/.gem/credentials from `gem login` works for a manual
# push; this script is not the local path.
#
# Duplicate handling: rubygems.org HAS a versions API, so the probe is a real
# check in both stages — a version already listed is a loud, idempotent skip (the
# same behavior as publish-npm.sh's `npm view` probe and
# create-github-release.sh's "already exists"). A push failure after a clean
# probe is real and stops the release.

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

GEM_NAME="temper-rb"
VERSIONS_API="https://rubygems.org/api/v1/versions/${GEM_NAME}.json"
REPO_ROOT="$(git rev-parse --show-toplevel)"
GEM_DIR="${REPO_ROOT}/clients/temper-rb"
GEM_FILE="${GEM_NAME}-${VERSION}.gem"

if curl -sf "$VERSIONS_API" | grep "\"number\":\"${VERSION}\"" > /dev/null; then
    echo "==> ${GEM_NAME} ${VERSION} is already published — nothing to do."
    exit 0
fi

if [[ -n "$OUT_DIR" ]]; then
    echo "==> build ${GEM_NAME} ${VERSION} into ${OUT_DIR}"
    DECLARED="$(grep -oE "VERSION = '[^']+'" "${GEM_DIR}/lib/temper/version.rb" | cut -d"'" -f2)"
    if [[ "$DECLARED" != "$VERSION" ]]; then
        echo "ERROR: Temper::VERSION is ${DECLARED}, but ${VERSION} was requested." >&2
        echo "       Update clients/temper-rb/lib/temper/version.rb first." >&2
        exit 1
    fi
    mkdir -p "$OUT_DIR"
    OUT_ABS="$(cd "$OUT_DIR" && pwd)"
    (cd "$GEM_DIR" && gem build "${GEM_NAME}.gemspec" --output "${OUT_ABS}/${GEM_FILE}")
    exit 0
fi

echo "==> publish ${GEM_FILE} from ${FROM_DIR} (dry-run: ${DRY_RUN})"
GEM_PATH="${FROM_DIR}/${GEM_FILE}"
[[ -s "$GEM_PATH" ]] || { echo "ERROR: ${GEM_FILE} is not in ${FROM_DIR}, and ${GEM_NAME} ${VERSION} is not published." >&2; exit 1; }
# The name and version the gem itself declares, not its filename's: publishing a
# gem whose contents disagree with the tag is worse than failing.
IN_GEM="$(ruby -rrubygems/package -e 'spec = Gem::Package.new(ARGV[0]).spec; print "#{spec.name} #{spec.version}"' "$GEM_PATH")"
if [[ "$IN_GEM" != "${GEM_NAME} ${VERSION}" ]]; then
    echo "ERROR: ${GEM_FILE} declares '${IN_GEM}', expected '${GEM_NAME} ${VERSION}'." >&2
    exit 1
fi

if [[ "$DRY_RUN" == "true" ]]; then
    echo "==> [dry-run] would push ${GEM_FILE} to rubygems.org"
    exit 0
fi

if gem push "$GEM_PATH"; then
    echo "==> Published ${GEM_FILE}"
    exit 0
fi

echo "ERROR: gem push failed for ${GEM_NAME} ${VERSION}:" >&2
exit 1
