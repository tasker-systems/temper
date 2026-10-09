#!/usr/bin/env bash
# .github/scripts/test-publish-npm.sh
#
# Harness for publish-npm.sh — its two stages (`--out DIR` builds and packs; `--from DIR`
# publishes the packed tarballs), the duplicate probe in both, the scope guard, and the
# version-agreement guard.
#
# Why a harness: the publish lanes run for real only at a tagged release, so the loud-skip path
# (a re-cut release must skip an already-published version, never overwrite it) would otherwise
# be witnessed by an actual registry push. A bash stub for `npm` makes the call ORDER observable —
# the probe must decide before any build or publish runs — and lets the skip/refusal paths be
# bitten on every PR rather than on a release. The publish stage's guards read the manifest INSIDE
# the tarball, so the stub's `pack` writes a real one.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="${SCRIPT_DIR}/../../tools/scripts/release/publish-npm.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# Stub `npm`, logging every invocation. `view` exits per $STUB_VIEW_RESULT (0 = version exists,
# 1 = E404). `pack --pack-destination D` writes D/<scope-name>-<version>.tgz holding the cwd's
# package.json as package/package.json, as npm does. Everything else succeeds.
STUB_DIR="$TMP/bin"
mkdir -p "$STUB_DIR"
cat > "$STUB_DIR/npm" <<'STUB'
#!/bin/sh
echo "npm $*" >> "$NPM_CALLS"
if [ "$1" = "view" ]; then
    exit "${STUB_VIEW_RESULT:-1}"
fi
if [ "$1" = "pack" ]; then
    dest=""
    prev=""
    for a in "$@"; do
        [ "$prev" = "--pack-destination" ] && dest="$a"
        prev="$a"
    done
    name=$(grep -m1 '"name"' package.json | sed -E 's/.*"name": "([^"]+)".*/\1/')
    version=$(grep -m1 '"version"' package.json | sed -E 's/.*"version": "([^"]+)".*/\1/')
    stage=$(mktemp -d)
    mkdir -p "$stage/package"
    cp package.json "$stage/package/package.json"
    tarball="$(echo "${name#@}" | tr '/' '-')-${version}.tgz"
    tar -czf "$dest/$tarball" -C "$stage" package
    rm -rf "$stage"
    echo "$tarball"
fi
exit 0
STUB
chmod +x "$STUB_DIR/npm"

REPO="$TMP/repo"
git init -q "$REPO"

stage_package() {
    DIR="$1"; NAME="$2"; VERSION="$3"
    mkdir -p "${REPO}/${DIR}"
    printf '{\n  "name": "%s",\n  "version": "%s"\n}\n' "$NAME" "$VERSION" \
        > "${REPO}/${DIR}/package.json"
}

run_target() {
    CALLS="$1"; shift
    : > "$CALLS"
    (cd "$REPO" && PATH="$STUB_DIR:$PATH" NPM_CALLS="$CALLS" \
        STUB_VIEW_RESULT="${STUB_VIEW_RESULT:-1}" bash "$TARGET" "$@")
}

BOTH_DIRS=(clients/temper-ts clients/temper-telemetry-ts)
stage_both() {
    for d in "${BOTH_DIRS[@]}"; do
        stage_package "$d" "@tasker-systems/$(basename "$d")" "9.9.9"
    done
}
stage_both

# --- 1. Build stage, fresh version: probe, then install with no scripts, build, pack ------
# The positive control: a guard that refused everything would satisfy the negative cases below
# without ever being right.
OUT="$TMP/out"
CALLS_BUILD="$TMP/calls-build"
run_target "$CALLS_BUILD" "9.9.9" --out "$OUT" > "$TMP/build.log" 2>&1 \
    || fail "a fresh build was refused: $(cat "$TMP/build.log")"
for d in "${BOTH_DIRS[@]}"; do
    pkg="@tasker-systems/$(basename "$d")"
    grep -q "npm view ${pkg}@9.9.9 version" "$CALLS_BUILD" \
        || fail "the duplicate probe never ran for ${pkg}: $(cat "$CALLS_BUILD")"
    [ -s "$OUT/tasker-systems-$(basename "$d")-9.9.9.tgz" ] \
        || fail "${pkg} was not packed into the output directory: $(ls -la "$OUT")"
done
grep -q "npm ci --ignore-scripts" "$CALLS_BUILD" \
    || fail "the build installed with install scripts enabled: $(cat "$CALLS_BUILD")"
grep -q "npm pack --ignore-scripts" "$CALLS_BUILD" \
    || fail "the pack ran package scripts: $(cat "$CALLS_BUILD")"
grep -q "npm publish" "$CALLS_BUILD" \
    && fail "the build stage published: $(cat "$CALLS_BUILD")"
VIEW_LINE=$(grep -n "view @tasker-systems/temper-ts@" "$CALLS_BUILD" | head -1 | cut -d: -f1)
CI_LINE=$(grep -n "npm ci" "$CALLS_BUILD" | head -1 | cut -d: -f1)
[ "$VIEW_LINE" -lt "$CI_LINE" ] \
    || fail "the build ran before the duplicate probe — the probe is decorative: $(cat "$CALLS_BUILD")"

echo "PASS: the build stage probes, installs without scripts, builds and packs, and never publishes"

# --- 2. Publish stage, fresh version: probe, then publish each tarball, no install -------
CALLS_PUB="$TMP/calls-pub"
run_target "$CALLS_PUB" "9.9.9" --from "$OUT" > "$TMP/pub.log" 2>&1 \
    || fail "a fresh publish was refused: $(cat "$TMP/pub.log")"
for d in "${BOTH_DIRS[@]}"; do
    grep -q "npm publish $OUT/tasker-systems-$(basename "$d")-9.9.9.tgz --access public" "$CALLS_PUB" \
        || fail "@tasker-systems/$(basename "$d") was not published from its tarball: $(cat "$CALLS_PUB")"
done
grep -q -- "--ignore-scripts" "$CALLS_PUB" \
    || fail "the publish did not disable package scripts: $(cat "$CALLS_PUB")"
grep -Eq "npm (ci|run|pack)" "$CALLS_PUB" \
    && fail "the publish stage installed, built or packed: $(cat "$CALLS_PUB")"
VIEW_LINE=$(grep -n "view @tasker-systems/temper-ts@" "$CALLS_PUB" | head -1 | cut -d: -f1)
PUBLISH_LINE=$(grep -n "publish .*temper-ts-9.9.9.tgz" "$CALLS_PUB" | head -1 | cut -d: -f1)
[ "$VIEW_LINE" -lt "$PUBLISH_LINE" ] \
    || fail "publish ran before the duplicate probe — the probe is decorative: $(cat "$CALLS_PUB")"

echo "PASS: the publish stage probes, then publishes each packed tarball and runs nothing else"

# --- 3. Already-published version: both stages skip loudly, nothing runs ----------------
# The bite. The stub can't vary its answer per package, so this runs with the probe reporting
# EVERY version as existing — the strictest form: a re-cut release must skip both packages
# loudly, exit 0, and run no build and no publish in either stage.
for stage in out from; do
    CALLS_SKIP="$TMP/calls-skip-$stage"
    STUB_VIEW_RESULT=0 run_target "$CALLS_SKIP" "9.9.9" "--$stage" "$TMP/skipdir" > "$TMP/skip.log" 2>&1 \
        || fail "an all-published re-run of --$stage exited non-zero (idempotent skip must exit 0): $(cat "$TMP/skip.log")"
    grep -Eq "npm (publish|ci|run|pack)" "$CALLS_SKIP" \
        && fail "a fully-published version still built or published in --$stage: $(cat "$CALLS_SKIP")"
    grep -c "already published — nothing to do" "$TMP/skip.log" | grep -q "^2$" \
        || fail "both packages did not skip loudly in --$stage: $(cat "$TMP/skip.log")"
done

echo "PASS: an already-published version skips loudly in both stages and builds or publishes nothing"

# --- 4. Manifest version disagrees with the requested version: refuse before any npm call -
stage_package "clients/temper-ts" "@tasker-systems/temper-ts" "0.0.0"
CALLS_MISMATCH="$TMP/calls-mismatch"
if run_target "$CALLS_MISMATCH" "9.9.9" --out "$TMP/mismatch-out" > "$TMP/mismatch.log" 2>&1; then
    fail "a version mismatch built anyway"
fi
grep -q "clients/temper-ts manifest version is 0.0.0" "$TMP/mismatch.log" \
    || fail "the refusal did not name the disagreeing manifest: $(cat "$TMP/mismatch.log")"
[ ! -s "$CALLS_MISMATCH" ] \
    || fail "a mismatched version still invoked npm: $(cat "$CALLS_MISMATCH")"

echo "PASS: a manifest whose version disagrees with the tag is refused before any npm call"

# --- 5. Unscoped manifest name: refuse before any npm call ------------------------------
stage_package "clients/temper-ts" "temper-ts" "9.9.9"
CALLS_UNSCOPED="$TMP/calls-unscoped"
if run_target "$CALLS_UNSCOPED" "9.9.9" --out "$TMP/unscoped-out" > "$TMP/unscoped.log" 2>&1; then
    fail "an unscoped package was built anyway"
fi
grep -Fq '@tasker-systems/* scope' "$TMP/unscoped.log" \
    || fail "the scope refusal did not name its cause: $(cat "$TMP/unscoped.log")"
[ ! -s "$CALLS_UNSCOPED" ] \
    || fail "an unscoped package still invoked npm: $(cat "$CALLS_UNSCOPED")"
stage_both

echo "PASS: an unscoped manifest name is refused before any npm call"

# --- 6. Publish stage: a tarball whose own manifest disagrees is refused ----------------
# The publish stage must judge the bytes it is about to push, not the source tree.
TAMPER="$TMP/tamper"
cp -R "$OUT" "$TAMPER"
mkdir -p "$TMP/t/package"
printf '{\n  "name": "@tasker-systems/temper-ts",\n  "version": "9.9.8"\n}\n' > "$TMP/t/package/package.json"
tar -czf "$TAMPER/tasker-systems-temper-ts-9.9.9.tgz" -C "$TMP/t" package
CALLS_TAMPER="$TMP/calls-tamper"
if run_target "$CALLS_TAMPER" "9.9.9" --from "$TAMPER" > "$TMP/tamper.log" 2>&1; then
    fail "a tarball declaring another version was published"
fi
grep -q "tasker-systems-temper-ts-9.9.9.tgz manifest version is 9.9.8" "$TMP/tamper.log" \
    || fail "the refusal did not name the tarball's own version: $(cat "$TMP/tamper.log")"
grep -q "npm publish" "$CALLS_TAMPER" \
    && fail "a tampered tarball still reached publish: $(cat "$CALLS_TAMPER")"

echo "PASS: the publish stage refuses a tarball whose own manifest disagrees with the tag"

# --- 7. Publish stage: an unpublished version with no tarball is refused ----------------
mkdir -p "$TMP/empty"
if run_target "$TMP/calls-missing" "9.9.9" --from "$TMP/empty" > "$TMP/missing.log" 2>&1; then
    fail "a missing tarball for an unpublished version exited 0"
fi
grep -q "is not in $TMP/empty, and @tasker-systems/temper-ts@9.9.9 is not published" "$TMP/missing.log" \
    || fail "the refusal did not say what is missing: $(cat "$TMP/missing.log")"

echo "PASS: an unpublished version whose tarball is absent fails loudly"

# --- 8. Dry-run: the publish stage packs nothing and never really publishes -------------
CALLS_DRY="$TMP/calls-dry"
run_target "$CALLS_DRY" "9.9.9" --from "$OUT" --dry-run > "$TMP/dry.log" 2>&1 \
    || fail "a dry-run failed: $(cat "$TMP/dry.log")"
grep -q -- "--dry-run" "$CALLS_DRY" \
    || fail "the dry-run never ran npm publish --dry-run: $(cat "$CALLS_DRY")"
grep -E "npm publish " "$CALLS_DRY" | grep -vq -- "--dry-run" \
    && fail "the dry-run performed a real publish: $(cat "$CALLS_DRY")"

echo "PASS: a dry-run never really publishes"

# --- 9. Stage flags: exactly one of --out/--from, and --dry-run only with --from --------
for args in "9.9.9" "9.9.9 --out a --from b" "9.9.9 --out a --dry-run"; do
    # shellcheck disable=SC2086
    if run_target "$TMP/calls-flags" $args > "$TMP/flags.log" 2>&1; then
        fail "'$args' was accepted"
    fi
done

echo "PASS: a call naming no stage, both stages, or a build-stage dry-run is refused"
