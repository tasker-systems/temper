#!/usr/bin/env bash
# tools/scripts/release/roll-register.sh — after a release is tagged, move the register entries it
# shipped out of the open window.
#
# The release calculator reads only release-register/unreleased/. Rolling moves what a release
# shipped into release-register/v<version>/, so the next release's roll-up starts clean.
#
# WHAT "SHIPPED" MEANS: an entry is shipped when the tag's own tree carries it. That is the roll
# "by ancestry" that RELEASING.md used to ask for by hand. An entry merged after the tag stays in
# unreleased/, because the tag did not ship it. The old manual roll (retitle a heading) could not
# make that distinction, and skipping it silently inflated the next release's arithmetic. The
# calculator now refuses an unrolled window instead (calculate-versions.sh, "An unrolled window
# fails loudly").
#
# Spec of record: temper-artifacts:specs/2026-10-11-release-register-entries-design.md, D6.
#
# Usage:
#   tools/scripts/release/roll-register.sh v<x.y.z>
#
# Idempotent: a second run finds nothing left to move and says so. The moves are staged (git mv);
# committing them is the roll PR.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "${SCRIPT_DIR}/lib/common.sh"

TAG="${1:-}"
[[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "Usage: roll-register.sh v<x.y.z> (got '${TAG}')"

cd "$REPO_ROOT"
git rev-parse -q --verify "refs/tags/${TAG}" >/dev/null \
    || die "Tag ${TAG} is not present locally — fetch it first (git fetch --tags). The roll is keyed to the tag's own tree."

REGISTER_REL="release-register"
UNRELEASED="${REGISTER_REL}/unreleased"
TARGET="${REGISTER_REL}/${TAG}"

MOVED=0
while IFS= read -r shipped; do
    entry="$(basename "$shipped")"
    if [[ -f "${UNRELEASED}/${entry}" ]]; then
        mkdir -p "$TARGET"
        git mv "${UNRELEASED}/${entry}" "${TARGET}/${entry}"
        MOVED=$((MOVED + 1))
    fi
done < <(git ls-tree --name-only "$TAG" -- "${UNRELEASED}/" | grep '\.toml$' || true)

REMAINING="$(find "$UNRELEASED" -maxdepth 1 -name '*.toml' | wc -l | tr -d ' ')"
if [[ "$MOVED" -eq 0 ]]; then
    echo "==> Nothing to roll: no entry in ${UNRELEASED}/ was shipped by ${TAG}. (${REMAINING} entries remain open.)"
else
    echo "==> Rolled ${MOVED} entries into ${TARGET}/; ${REMAINING} remain in ${UNRELEASED}/ (merged after ${TAG})."
fi

python3 "$MANIFEST_PY" register validate --dir "${REPO_ROOT}/${REGISTER_REL}"
