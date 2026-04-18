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

if ! gh release view "$TAG" >/dev/null 2>&1; then
    echo "Creating release $TAG..."
    gh release create "$TAG" \
        --title "temper $TAG" \
        --generate-notes
else
    echo "Release $TAG already exists, skipping create step."
fi

echo "Uploading artifacts from ${ARTIFACT_DIR}..."
shopt -s nullglob
for f in "${ARTIFACT_DIR}"/temper-*.tar.gz \
         "${ARTIFACT_DIR}"/temper-*.zip \
         "${ARTIFACT_DIR}"/temper-*.sha256; do
    echo "  Uploading $(basename "$f")..."
    gh release upload "$TAG" "$f" --clobber
done

echo "Done."
