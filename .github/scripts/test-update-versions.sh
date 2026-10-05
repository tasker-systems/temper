#!/usr/bin/env bash
# .github/scripts/test-update-versions.sh
#
# Test harness for tools/scripts/release/update-versions.sh — the one writer for
# every version site. Its core bump must move the shared anchor (VERSION), the
# [workspace.package] version every crate inherits, and every temperkb-*
# [workspace.dependencies] spec, then read all of them back.
#
# WHY THE REAL MANIFEST, NOT A FIXTURE
# ------------------------------------
# The defect this exists for was a drift between the writer's pattern and the
# manifest's line shapes: a spec line gained a key after its version
# (`temperkb-client = { …, version = "…", default-features = false }`), the
# rewrite stopped matching it, and the bump died at its own read-back — on the
# release day, because `release-check`'s dry run never exercises the rewrite.
# A fixture would encode the shapes as they were when it was written. So the
# sandbox takes the repository's own Cargo.toml and VERSION: whatever shape a
# spec line takes next, this harness bumps it.
#
# The leaf floats (--rb/--py/--ts/…) are not exercised: their writers read files
# outside the release tree and are covered by their own read-backs.
#
# Bite direction: restoring the old closing-brace-anchored pattern (`(" \})`)
# reds the bump arm on today's manifest.
#
#   bash .github/scripts/test-update-versions.sh

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO="$(cd "${SCRIPT_DIR}/../.." && pwd)"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; shift; printf '    %s\n' "$@"; FAIL=$((FAIL + 1)); }

sandbox() {
    rm -rf "${WORK:?}/sb"
    mkdir -p "${WORK}/sb/tools/scripts"
    cp -R "${REPO}/tools/scripts/release" "${WORK}/sb/tools/scripts/release"
    cp "${REPO}/Cargo.toml" "${REPO}/VERSION" "${WORK}/sb/"
}

ws_version() {
    awk -F'"' '/^\[workspace\.package\]/{p=1; next} /^\[/{p=0} p && /^version = /{print $2; exit}' "$1"
}

TARGET="9.9.9"
SPEC_COUNT="$(grep -cE '^temperkb-[a-z]+ = \{ path = ' "${REPO}/Cargo.toml")"

echo "test-update-versions"

# --- the bump moves every core site -------------------------------------------
sandbox
OUT="$(cd "${WORK}/sb" && bash tools/scripts/release/update-versions.sh --core "$TARGET" 2>&1)"
STATUS=$?
if [[ $STATUS -eq 0 ]]; then
    ok "core bump exits 0 on the repository's manifest"
else
    bad "core bump exits 0 on the repository's manifest" "exit ${STATUS}" "$(printf '%s' "$OUT" | tail -5)"
fi

if [[ "$(tr -d '[:space:]' < "${WORK}/sb/VERSION")" == "$TARGET" ]]; then
    ok "VERSION moves"
else
    bad "VERSION moves" "VERSION is $(cat "${WORK}/sb/VERSION")"
fi

if [[ "$(ws_version "${WORK}/sb/Cargo.toml")" == "$TARGET" ]]; then
    ok "[workspace.package] version moves"
else
    bad "[workspace.package] version moves" "is $(ws_version "${WORK}/sb/Cargo.toml")"
fi

STALE="$(awk -F'"' -v want="$TARGET" '/^temperkb-[a-z]+ = \{ path = / && $4 != want { print $1 "-> " $4 }' "${WORK}/sb/Cargo.toml")"
MOVED="$(grep -cE "^temperkb-[a-z]+ = \\{ path = \"[^\"]+\", version = \"${TARGET}\"" "${WORK}/sb/Cargo.toml")"
if [[ -z "$STALE" && "$MOVED" == "$SPEC_COUNT" && "$SPEC_COUNT" -gt 0 ]]; then
    ok "all ${SPEC_COUNT} temperkb-* [workspace.dependencies] specs move"
else
    bad "all ${SPEC_COUNT} temperkb-* [workspace.dependencies] specs move" "moved ${MOVED}" "stale: ${STALE:-none}"
fi

# Keys after the version survive the rewrite: only the version value changes.
BEFORE="$(grep -E '^temperkb-[a-z]+ = \{ path = ' "${REPO}/Cargo.toml" | sed -E 's/version = "[^"]+"/version = "X"/')"
AFTER="$(grep -E '^temperkb-[a-z]+ = \{ path = ' "${WORK}/sb/Cargo.toml" | sed -E 's/version = "[^"]+"/version = "X"/')"
if [[ "$BEFORE" == "$AFTER" ]]; then
    ok "spec lines change only their version value"
else
    bad "spec lines change only their version value" "$(diff <(printf '%s\n' "$BEFORE") <(printf '%s\n' "$AFTER") | head -6)"
fi

# --- the dry run writes nothing ----------------------------------------------
sandbox
(cd "${WORK}/sb" && bash tools/scripts/release/update-versions.sh --core "$TARGET" --dry-run > /dev/null 2>&1)
if cmp -s "${REPO}/Cargo.toml" "${WORK}/sb/Cargo.toml" && cmp -s "${REPO}/VERSION" "${WORK}/sb/VERSION"; then
    ok "--dry-run leaves Cargo.toml and VERSION untouched"
else
    bad "--dry-run leaves Cargo.toml and VERSION untouched"
fi

echo ""
echo "test-update-versions: ${PASS} passed, ${FAIL} failed"
[[ $FAIL -eq 0 ]]
