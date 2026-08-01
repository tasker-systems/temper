#!/usr/bin/env bash
#
# Package the committed MCP skill tree as an uploadable Agent Skill bundle.
#
# Produces `temper-skill-v<VERSION>.zip` (plus a `.sha256` sidecar) containing the skill folder
# at the archive root — the layout Claude requires for a custom-Skill upload:
#
#     temper-skill-v0.2.7.zip
#     └── temper-knowledge-base/
#         ├── SKILL.md
#         ├── knowledge-base.md
#         └── references/frontmatter.md
#
# ## The name is load-bearing — do NOT "regularise" it to temper-v<ver>-skill.zip
#
# `create-github-release.sh` derives the set of archives that MUST ship a per-file manifest from
# the glob `temper-v${VERSION}-*.{tar.gz,zip}`. A bundle named `temper-v0.2.7-skill.zip` matches
# that glob and would be REFUSED for lacking a manifest it has no business carrying: the manifest
# exists so `install.sh` can verify each extracted file before an atomic swap, and nothing installs
# this bundle — a human uploads it to Claude. Named `temper-skill-v<ver>.zip` it misses the
# manifest glob while still matching the upload globs (`temper-*.zip`, `temper-*.sha256`), so it
# rides the existing publish loop with no edit to that script.
#
# The failure direction is safe either way: rename it back and the release fails loudly at the
# guard rather than publishing something unverifiable.
#
# ## Why the version is stamped here and not in the tree
#
# `update-version.sh` rewrites exactly two files (`VERSION`, `crates/temper-cli/Cargo.toml`) and
# regenerates nothing. If the emitted tree carried the version, every `release-prepare` would leave
# it stale and red the drift gate. So the source tree stays version-free and the version enters at
# packaging time: in the archive name, and in a `VERSION` file dropped into the bundle — the only
# way a user who has already uploaded the zip can tell which one they have, since the Skills UI
# shows the name and not the file it came from.
#
# Usage:
#   VERSION=0.2.7 bash .github/scripts/release/package-skill.sh [OUT_DIR]
#
# SKILL_BUNDLE_ROOT / SKILL_BUNDLE_NAME are harness seams for test-package-skill.sh. No CI job sets
# them; the release workflow and `cargo make skill-package` both run this unstubbed.

set -euo pipefail

: "${VERSION:?VERSION required (e.g. VERSION=0.2.7)}"

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
BUNDLE_ROOT="${SKILL_BUNDLE_ROOT:-${REPO_ROOT}/agent-skills}"
BUNDLE_NAME="${SKILL_BUNDLE_NAME:-temper-knowledge-base}"
OUT_DIR="${1:-${REPO_ROOT}/dist}"

SRC="${BUNDLE_ROOT}/${BUNDLE_NAME}"

if [ ! -f "${SRC}/SKILL.md" ]; then
    echo "ERROR: ${SRC}/SKILL.md not found — there is no skill to package." >&2
    echo "       Run: cargo run -p temper-cli -- skill emit --path ${BUNDLE_ROOT}/${BUNDLE_NAME}" >&2
    exit 1
fi

# The upload requires the folder name to match the skill's declared `name`. They are set in two
# different files by two different people, so check rather than assume: a mismatch uploads as a
# skill whose folder and identity disagree, which is the kind of thing nobody notices until the
# skill fails to load.
DECLARED_NAME="$(sed -n 's/^name:[[:space:]]*//p' "${SRC}/SKILL.md" | head -1 | tr -d '\r')"
if [ "$DECLARED_NAME" != "$BUNDLE_NAME" ]; then
    echo "ERROR: the bundle folder is '${BUNDLE_NAME}' but SKILL.md declares 'name: ${DECLARED_NAME}'." >&2
    echo "       Claude requires the folder name to match the skill name. Rename one of them." >&2
    exit 1
fi

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"
ARCHIVE="${OUT_DIR}/temper-skill-v${VERSION}.zip"

# Stage a copy so the VERSION stamp never lands in the working tree — writing it into the committed
# tree would red the drift gate, which is the whole reason it is added here.
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
cp -R "$SRC" "${STAGE}/${BUNDLE_NAME}"
printf 'temper-skill v%s\n' "$VERSION" > "${STAGE}/${BUNDLE_NAME}/VERSION"

rm -f "$ARCHIVE"
# `-X` drops platform extra-fields (uid/gid, Finder attrs) so the archive does not vary with who
# built it. Timestamps still ride along, so this is build-stable, not bit-reproducible — enough for
# an artifact whose integrity rests on the sha256 + provenance attestation published beside it.
(cd "$STAGE" && zip -q -r -X "$ARCHIVE" "$BUNDLE_NAME")

# Same sha tool branch as emit-manifest.sh: sha256sum on Linux/Git-Bash, shasum on macOS.
if command -v sha256sum >/dev/null 2>&1; then
    (cd "$OUT_DIR" && sha256sum "$(basename "$ARCHIVE")" > "$(basename "$ARCHIVE").sha256")
else
    (cd "$OUT_DIR" && shasum -a 256 "$(basename "$ARCHIVE")" > "$(basename "$ARCHIVE").sha256")
fi

FILE_COUNT="$(cd "$STAGE" && find "$BUNDLE_NAME" -type f | wc -l | tr -d ' ')"
echo "Packaged ${FILE_COUNT} file(s) into $(basename "$ARCHIVE")"
cat "${ARCHIVE}.sha256"
