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
# The npm leaves (--ts/--telemetry/--ui/--cloud) run the same way, against the
# repository's own package.json files and lockfiles: each package's version must
# move in its package.json and in the lockfile that records it (bun.lock for the
# workspace members, package-lock.json for the clients), and the lockfiles must
# change in nothing but those lines. `bun install --frozen-lockfile` accepts a
# stale workspace version, so a lockfile the bump leaves behind fails nothing
# else. The --rb/--py floats are not exercised: their writers are covered by
# their own read-backs.
#
# Bite direction: restoring the old closing-brace-anchored pattern (`(" \})`)
# reds the bump arm on today's manifest; dropping the update_lockfile_version
# calls from update-versions.sh reds the lockfile arms.
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
    cp "${REPO}/Cargo.toml" "${REPO}/VERSION" "${REPO}/bun.lock" "${WORK}/sb/"
    mkdir -p "${WORK}/sb/.github/scripts"
    cp "${REPO}/.github/scripts/check-lockfile-versions.sh" "${WORK}/sb/.github/scripts/"
    for pkg in "${NPM_PACKAGES[@]}"; do
        mkdir -p "${WORK}/sb/${pkg}"
        cp "${REPO}/${pkg}/package.json" "${WORK}/sb/${pkg}/"
        [[ -f "${REPO}/${pkg}/package-lock.json" ]] && cp "${REPO}/${pkg}/package-lock.json" "${WORK}/sb/${pkg}/"
    done
}

NPM_PACKAGES=(packages/temper-ui packages/temper-cloud clients/temper-ts clients/temper-telemetry-ts)

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

# --- the npm leaves move their package.json and their lockfile record ---------
# Distinct versions per leaf, so a write landing on the wrong package's entry shows.
LEAF_ARGS=(--ui 9.9.4 --cloud 9.9.5 --ts 9.9.6 --telemetry 9.9.7)
leaf_version() {
    case "$1" in
        packages/temper-ui) echo 9.9.4 ;;
        packages/temper-cloud) echo 9.9.5 ;;
        clients/temper-ts) echo 9.9.6 ;;
        clients/temper-telemetry-ts) echo 9.9.7 ;;
    esac
}
lockfile_of() {
    if [[ -f "${2}/${1}/package-lock.json" ]]; then echo "${1}/package-lock.json"; else echo bun.lock; fi
}

sandbox
OUT="$(cd "${WORK}/sb" && bash tools/scripts/release/update-versions.sh --core "$TARGET" "${LEAF_ARGS[@]}" 2>&1)"
STATUS=$?
if [[ $STATUS -eq 0 ]]; then
    ok "leaf bump exits 0 on the repository's lockfiles"
else
    bad "leaf bump exits 0 on the repository's lockfiles" "exit ${STATUS}" "$(printf '%s' "$OUT" | tail -5)"
fi

for pkg in "${NPM_PACKAGES[@]}"; do
    want="$(leaf_version "$pkg")"
    lock="$(lockfile_of "$pkg" "${WORK}/sb")"
    # Every line the lockfile changed is a version line now reading this leaf's version: one
    # for a bun.lock workspace entry, two (top level + packages[""]) for a package-lock.json.
    changed="$(diff "${REPO}/${lock}" "${WORK}/sb/${lock}" | grep '^>' || true)"
    expect=1; [[ "$lock" == */package-lock.json ]] && expect=2
    mine="$(grep -cF "\"version\": \"${want}\"" <<< "$changed" || true)"
    if [[ "$mine" == "$expect" ]]; then
        ok "${pkg}: ${lock} records ${want}"
    else
        bad "${pkg}: ${lock} records ${want}" "${mine} of ${expect} version lines moved" "$(printf '%s' "$changed" | head -4)"
    fi
done

CHANGED="$(for lock in bun.lock clients/temper-ts/package-lock.json clients/temper-telemetry-ts/package-lock.json; do
    diff "${REPO}/${lock}" "${WORK}/sb/${lock}" | grep '^[<>]' || true
done | grep -vcE '^[<>] +"version": "' || true)"
if [[ "$CHANGED" == "0" ]]; then
    ok "the lockfiles change only their version lines"
else
    bad "the lockfiles change only their version lines" "${CHANGED} other lines changed"
fi

if (cd "${WORK}/sb" && bash .github/scripts/check-lockfile-versions.sh > /dev/null 2>&1); then
    ok "check-lockfile-versions passes after the bump"
else
    bad "check-lockfile-versions passes after the bump" "$(cd "${WORK}/sb" && bash .github/scripts/check-lockfile-versions.sh 2>&1 | grep error | head -4)"
fi

# The check reds a package.json the lockfile did not follow.
sandbox
sed -i -E '0,/"version": "[^"]+"/s//"version": "9.9.3"/' "${WORK}/sb/packages/temper-cloud/package.json"
if (cd "${WORK}/sb" && bash .github/scripts/check-lockfile-versions.sh > /dev/null 2>&1); then
    bad "check-lockfile-versions fails on a package.json its lockfile did not follow"
else
    ok "check-lockfile-versions fails on a package.json its lockfile did not follow"
fi

# --- the dry run writes nothing ----------------------------------------------
sandbox
(cd "${WORK}/sb" && bash tools/scripts/release/update-versions.sh --core "$TARGET" "${LEAF_ARGS[@]}" --dry-run > /dev/null 2>&1)
UNTOUCHED=true
for f in Cargo.toml VERSION bun.lock "${NPM_PACKAGES[@]/%//package.json}" clients/temper-ts/package-lock.json clients/temper-telemetry-ts/package-lock.json; do
    cmp -s "${REPO}/${f}" "${WORK}/sb/${f}" || { UNTOUCHED=false; echo "    changed: ${f}"; }
done
if $UNTOUCHED; then
    ok "--dry-run leaves every version site untouched"
else
    bad "--dry-run leaves every version site untouched"
fi

echo ""
echo "test-update-versions: ${PASS} passed, ${FAIL} failed"
[[ $FAIL -eq 0 ]]
