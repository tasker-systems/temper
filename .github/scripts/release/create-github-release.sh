#!/usr/bin/env bash
# .github/scripts/release/create-github-release.sh
#
# Create a GitHub Release for the current tag and attach CLI binary artifacts.
#
# Required env:
#   GH_TOKEN     — GitHub token with contents:write
#   VERSION      — e.g. 0.1.0 (without the "v" prefix)
#   ARTIFACT_DIR — directory containing temper-v*.{tar.gz,zip,sha256}

set -euo pipefail

: "${GH_TOKEN:?GH_TOKEN required}"
: "${VERSION:?VERSION required}"
: "${ARTIFACT_DIR:?ARTIFACT_DIR required}"

TAG="v${VERSION}"

# --- Every archive must ship a per-file manifest ------------------------------
# The upload loop below runs under `nullglob`, so a glob that matches nothing
# contributes nothing and the release publishes with a green tick. That is fine
# for a genuinely optional asset and wrong for this one: `install.sh` degrades a
# missing manifest to `unverifiable` and installs anyway (deliberately — every
# release up to v0.2.6 predates manifests and must stay installable). Which
# means a release that silently loses its manifest is INDISTINGUISHABLE, to
# every consumer, from one that never had it — the installer would stop
# verifying and only warn, forever, and nothing here would have objected.
#
# So the invariant lives on this side: the consumer degrades honestly, the
# producer refuses. Expected manifests are derived from the archives actually
# built rather than a hardcoded triple list, so a matrix change cannot leave
# this check asserting a target that no longer exists (or, worse, silently not
# asserting a new one). All three targets emit a manifest — the "Emit per-file
# manifest" step in build-cli-binaries.yml is unconditional, including Windows.
shopt -s nullglob
ARCHIVES=( "${ARTIFACT_DIR}"/temper-v"${VERSION}"-*.tar.gz \
           "${ARTIFACT_DIR}"/temper-v"${VERSION}"-*.zip )

if [ ${#ARCHIVES[@]} -eq 0 ]; then
    echo "error: no archives for v${VERSION} in ${ARTIFACT_DIR} — refusing to publish an empty release" >&2
    exit 1
fi

MANIFEST_MISSING=0
for archive in "${ARCHIVES[@]}"; do
    base="$(basename "$archive")"
    stem="${base%.tar.gz}"
    stem="${stem%.zip}"
    if [ ! -f "${ARTIFACT_DIR}/${stem}.manifest.json" ]; then
        echo "error: ${base} has no per-file manifest (expected ${stem}.manifest.json)" >&2
        MANIFEST_MISSING=1
    fi
done

if [ "$MANIFEST_MISSING" -ne 0 ]; then
    echo "error: refusing to publish v${VERSION} — at least one target built an archive but no" >&2
    echo "       manifest. Publishing it would ship a release that installs unverified." >&2
    exit 1
fi

echo "Per-file manifest present for all ${#ARCHIVES[@]} archive(s)."

# Deliberately AFTER the manifest check: this is the first step that produces
# anything outward-facing. Refusing before it means a build that lost its
# manifest leaves no half-published release tag behind for someone to find,
# install from, and get an `unverifiable` install out of.
if ! gh release view "$TAG" >/dev/null 2>&1; then
    echo "Creating release $TAG..."
    gh release create "$TAG" \
        --title "temper $TAG" \
        --generate-notes
else
    echo "Release $TAG already exists, skipping create step."
fi

echo "Uploading artifacts from ${ARTIFACT_DIR}..."
for f in "${ARTIFACT_DIR}"/temper-*.tar.gz \
         "${ARTIFACT_DIR}"/temper-*.zip \
         "${ARTIFACT_DIR}"/temper-*.sha256 \
         "${ARTIFACT_DIR}"/temper-*.manifest.json; do
    echo "  Uploading $(basename "$f")..."
    gh release upload "$TAG" "$f" --clobber
done

echo "Done."
