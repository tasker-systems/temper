#!/usr/bin/env bash
# .github/scripts/test-package-skill.sh
#
# Test harness for release/package-skill.sh — the Agent Skill bundle packager.
#
# What makes this worth testing is that its output is consumed by two things that will not tell you
# they are unhappy in any useful way: Claude's Skills uploader (which needs an exact archive layout)
# and create-github-release.sh (whose manifest guard keys on the FILE NAME). Both fail late — at
# upload time, or in a release job — so the cheap checks belong here.
#
# HERMETIC: every case builds a throwaway bundle under a temp dir and packages that, never the real
# agent-skills tree. SKILL_BUNDLE_ROOT / SKILL_BUNDLE_NAME are the seams; no CI job sets them.
#
#   bash .github/scripts/test-package-skill.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PACKAGER="${SCRIPT_DIR}/release/package-skill.sh"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Build a fixture bundle. `declared_name` is what SKILL.md claims; `folder` is what it sits in —
# separate parameters so the mismatch case is expressible.
make_bundle() {
    local root="$1" folder="$2" declared_name="$3"
    rm -rf "$root"
    mkdir -p "$root/$folder/references"
    cat >"$root/$folder/SKILL.md" <<EOF
---
name: $declared_name
description: A fixture skill.
---

# Fixture
EOF
    echo "# frontmatter" >"$root/$folder/references/frontmatter.md"
}

ok()   { echo "  ok: $1";   PASS=$((PASS + 1)); }
bad()  { echo "  FAIL: $1"; echo "    $2"; FAIL=$((FAIL + 1)); }

echo "test-package-skill"

# (a) The layout Claude requires: the skill FOLDER is the archive root, not the files. An archive
#     with SKILL.md at the top level is rejected by the uploader, and nothing else here checks it.
make_bundle "$WORK/a" "temper-knowledge-base" "temper-knowledge-base"
out="$WORK/a-out"
if VERSION=9.9.9 SKILL_BUNDLE_ROOT="$WORK/a" bash "$PACKAGER" "$out" >/dev/null 2>&1; then
    listing="$(unzip -Z1 "$out/temper-skill-v9.9.9.zip")"
    if printf '%s\n' "$listing" | grep -qx "temper-knowledge-base/SKILL.md" &&
        ! printf '%s\n' "$listing" | grep -qx "SKILL.md"; then
        ok "the skill folder is the archive root, not the files"
    else
        bad "the skill folder is the archive root, not the files" "listing: $listing"
    fi
else
    bad "the skill folder is the archive root, not the files" "packager exited non-zero"
fi

# (b) THE NAMING BOUNDARY. create-github-release.sh requires a per-file manifest for every archive
#     matching `temper-v<version>-*.{tar.gz,zip}`. This bundle must NOT match that glob (it carries
#     no manifest and needs none), while still matching the upload globs `temper-*.zip` /
#     `temper-*.sha256`. Asserted here because the only other place it is checked is a release job.
name="$(basename "$out"/temper-skill-v9.9.9.zip)"
# shellcheck disable=SC2254 # the glob is the subject under test
case "$name" in
    temper-v9.9.9-*.zip) bad "the archive name dodges the manifest guard" "matched the manifest glob: $name" ;;
    temper-*.zip)        ok  "the archive name dodges the manifest guard but rides the upload glob" ;;
    *)                   bad "the archive name dodges the manifest guard" "matched no upload glob: $name" ;;
esac

# (c) The sha256 sidecar must exist and be right — it is the only integrity statement the bundle
#     carries other than its attestation, and a sidecar over the wrong bytes is worse than none.
sidecar="$out/temper-skill-v9.9.9.zip.sha256"
if [ -f "$sidecar" ]; then
    recorded="$(awk '{print $1}' "$sidecar")"
    if command -v sha256sum >/dev/null 2>&1; then
        actual="$(sha256sum "$out/temper-skill-v9.9.9.zip" | awk '{print $1}')"
    else
        actual="$(shasum -a 256 "$out/temper-skill-v9.9.9.zip" | awk '{print $1}')"
    fi
    [ "$recorded" = "$actual" ] &&
        ok "the sha256 sidecar matches the archive" ||
        bad "the sha256 sidecar matches the archive" "recorded=$recorded actual=$actual"
else
    bad "the sha256 sidecar matches the archive" "no sidecar written"
fi

# (d) The VERSION stamp rides in the ARCHIVE, never the source tree — the source tree stays
#     version-free so `release-prepare` cannot red the drift gate. Both halves asserted: present in
#     the zip, absent from the bundle on disk.
if unzip -Z1 "$out/temper-skill-v9.9.9.zip" | grep -qx "temper-knowledge-base/VERSION" &&
    [ ! -f "$WORK/a/temper-knowledge-base/VERSION" ]; then
    ok "the VERSION stamp is in the archive and NOT in the source tree"
else
    bad "the VERSION stamp is in the archive and NOT in the source tree" "one half is wrong"
fi

# (e) Folder name vs declared skill name. Claude requires them to match, and they live in two
#     different files — so a rename of one is exactly the kind of half-done change that uploads a
#     broken skill. Refuse rather than package it.
make_bundle "$WORK/e" "temper-knowledge-base" "something-else"
if VERSION=9.9.9 SKILL_BUNDLE_ROOT="$WORK/e" bash "$PACKAGER" "$WORK/e-out" >/dev/null 2>&1; then
    bad "a folder/name mismatch is refused" "packager exited 0 on a mismatched bundle"
else
    ok "a folder/name mismatch is refused"
fi

# (f) A missing bundle must fail loudly rather than publishing an empty or absent archive. The
#     realistic cause is a packaging step that runs before the emit.
make_bundle "$WORK/f" "temper-knowledge-base" "temper-knowledge-base"
rm "$WORK/f/temper-knowledge-base/SKILL.md"
if VERSION=9.9.9 SKILL_BUNDLE_ROOT="$WORK/f" bash "$PACKAGER" "$WORK/f-out" >/dev/null 2>&1; then
    bad "a bundle with no SKILL.md is refused" "packager exited 0"
else
    ok "a bundle with no SKILL.md is refused"
fi

# (g) VERSION is required. Defaulting it would silently publish an archive claiming a version
#     nobody chose.
make_bundle "$WORK/g" "temper-knowledge-base" "temper-knowledge-base"
if (unset VERSION; SKILL_BUNDLE_ROOT="$WORK/g" bash "$PACKAGER" "$WORK/g-out" >/dev/null 2>&1); then
    bad "a missing VERSION is refused" "packager exited 0 with no VERSION"
else
    ok "a missing VERSION is refused"
fi

echo
echo "passed=${PASS} failed=${FAIL}"
[ "$FAIL" -eq 0 ]
