#!/usr/bin/env bash
# Emit the per-file release manifest for one target's staging directory.
#
# The manifest is generated HERE, from the staging tree, rather than by running
# the freshly-built binary. All three matrix runners are native so the artifact
# COULD describe itself — and that is exactly why it must not: a compromised
# artifact would faithfully attest to its own compromise.
#
# Required env:
#   VERSION  — e.g. 0.3.0 (no leading v)
#   TARGET   — target triple
#   STAGING  — the assembled staging dir
#   OUTPUT   — path to write the manifest JSON to
set -euo pipefail

: "${VERSION:?VERSION required}"
: "${TARGET:?TARGET required}"
: "${STAGING:?STAGING required}"
: "${OUTPUT:?OUTPUT required}"

# macOS ships shasum, Linux ships sha256sum. Same branch as install.sh:128-134
# and build-cli-binaries.yml:177,189 — never assume one.
sha_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

size_of() {
    wc -c < "$1" | tr -d ' '
}

ENTRIES=""
# -print0/read -d '' so paths with spaces survive. Sorted for a stable manifest.
while IFS= read -r -d '' f; do
    REL="${f#"$STAGING"/}"
    SHA=$(sha_of "$f")
    SIZE=$(size_of "$f")
    ENTRY=$(jq -n --arg path "$REL" --arg sha256 "$SHA" --argjson size "$SIZE" \
        '{path: $path, sha256: $sha256, size: $size}')
    ENTRIES="${ENTRIES}${ENTRY}"
done < <(find "$STAGING" -type f -print0 | sort -z)

printf '%s' "$ENTRIES" | jq -s \
    --arg version "$VERSION" \
    --arg target "$TARGET" \
    '{version: $version, target: $target, files: .}' > "$OUTPUT"

echo "Wrote manifest: $OUTPUT"
cat "$OUTPUT"
