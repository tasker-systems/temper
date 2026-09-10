#!/usr/bin/env bash
# .github/scripts/test-publish-npm.sh
#
# Harness for publish-npm.sh — the duplicate probe, the scope guard, and the
# version-agreement guard.
#
# Why a harness: the publish lanes run for real only at a tagged release, so
# the loud-skip path (a re-cut release must skip an already-published version,
# never overwrite it) would otherwise be witnessed by an actual registry push.
# A bash stub for `npm` makes the call ORDER observable — the probe must decide
# before any build or publish runs — and lets the skip/refusal paths be bitten
# on every PR rather than on a release.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="${SCRIPT_DIR}/../../tools/scripts/release/publish-npm.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# Stub `npm`, logging every invocation. Dispatch: `view` exits per
# $STUB_VIEW_RESULT (0 = version exists, 1 = E404); everything else succeeds.
STUB_DIR="$TMP/bin"
mkdir -p "$STUB_DIR"
cat > "$STUB_DIR/npm" <<'STUB'
#!/bin/sh
echo "npm $*" >> "$NPM_CALLS"
if [ "$1" = "view" ]; then
    exit "${STUB_VIEW_RESULT:-1}"
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
    (cd "$REPO" && PATH="$STUB_DIR:$PATH" NPM_CALLS="$CALLS" NODE_AUTH_TOKEN=stub \
        STUB_VIEW_RESULT="${STUB_VIEW_RESULT:-1}" bash "$TARGET" "$@")
}

BOTH_DIRS=(clients/temper-ts clients/temper-telemetry-ts)
for d in "${BOTH_DIRS[@]}"; do
    stage_package "$d" "@tasker-systems/$(basename "$d")" "9.9.9"
done

# --- 1. Fresh version: probe precedes publish, both packages publish ---------
# The positive control: a guard that refused everything would satisfy the
# negative cases below without ever being right.
CALLS_GOOD="$TMP/calls-good"
run_target "$CALLS_GOOD" "9.9.9" > "$TMP/good.log" 2>&1 \
    || fail "a fresh publish was refused: $(cat "$TMP/good.log")"
for d in "${BOTH_DIRS[@]}"; do
    pkg="@tasker-systems/$(basename "$d")"
    grep -q "npm view ${pkg}@9.9.9 version" "$CALLS_GOOD" \
        || fail "the duplicate probe never ran for ${pkg}: $(cat "$CALLS_GOOD")"
    grep -q "npm publish --no-fund" "$CALLS_GOOD" \
        || fail "${pkg} never reached publish: $(cat "$CALLS_GOOD")"
done
VIEW_LINE=$(grep -n "view @tasker-systems/temper-ts@" "$CALLS_GOOD" | head -1 | cut -d: -f1)
PUBLISH_LINE=$(grep -n "publish --no-fund" "$CALLS_GOOD" | head -1 | cut -d: -f1)
[ "$VIEW_LINE" -lt "$PUBLISH_LINE" ] \
    || fail "publish ran before the duplicate probe — the probe is decorative: $(cat "$CALLS_GOOD")"

echo "PASS: a fresh version probes the registry, then publishes both packages"

# --- 2. Already-published version: loud skip, no build or publish runs --------
# The bite. The stub can't vary its answer per package, so this case runs with
# the probe reporting EVERY version as existing — the strictest form: a re-cut
# release must skip both packages loudly, exit 0 (idempotent, like
# create-github-release.sh's "already exists"), and run no build and no
# publish. The `view` probes themselves are expected — they are what decided.
for d in "${BOTH_DIRS[@]}"; do
    stage_package "$d" "@tasker-systems/$(basename "$d")" "9.9.9"
done
CALLS_SKIP_ALL="$TMP/calls-skip-all"
STUB_VIEW_RESULT=0 run_target "$CALLS_SKIP_ALL" "9.9.9" > "$TMP/skip.log" 2>&1 \
    || fail "an all-published re-run exited non-zero (idempotent skip must exit 0): $(cat "$TMP/skip.log")"
grep -Eq "npm (publish|ci|run)" "$CALLS_SKIP_ALL" \
    && fail "a fully-published version still built or published: $(cat "$CALLS_SKIP_ALL")"
grep -c "already published — nothing to do" "$TMP/skip.log" | grep -q "^2$" \
    || fail "both packages did not skip loudly: $(cat "$TMP/skip.log")"

echo "PASS: an already-published version skips loudly and invokes no npm commands"

# --- 3. Manifest version disagrees with the requested version: refuse --------
stage_package "clients/temper-ts" "@tasker-systems/temper-ts" "0.0.0"
stage_package "clients/temper-telemetry-ts" "@tasker-systems/temper-telemetry-ts" "9.9.9"
CALLS_MISMATCH="$TMP/calls-mismatch"
if run_target "$CALLS_MISMATCH" "9.9.9" > "$TMP/mismatch.log" 2>&1; then
    fail "a version mismatch published anyway"
fi
grep -q "clients/temper-ts manifest version is 0.0.0" "$TMP/mismatch.log" \
    || fail "the refusal did not name the disagreeing manifest: $(cat "$TMP/mismatch.log")"
[ ! -s "$CALLS_MISMATCH" ] \
    || fail "a mismatched version still invoked npm: $(cat "$CALLS_MISMATCH")"

echo "PASS: a manifest whose version disagrees with the tag is refused before any npm call"

# --- 4. Unscoped manifest name: refuse ---------------------------------------
# GitHub Packages npm requires scopes; an unscoped name would target a
# different registry. Distinct message from case 3 so the two stay separable.
stage_package "clients/temper-ts" "temper-ts" "9.9.9"
CALLS_UNSCOPED="$TMP/calls-unscoped"
if run_target "$CALLS_UNSCOPED" "9.9.9" > "$TMP/unscoped.log" 2>&1; then
    fail "an unscoped package was published anyway"
fi
grep -Fq '@tasker-systems/* scope' "$TMP/unscoped.log" \
    || fail "the scope refusal did not name its cause: $(cat "$TMP/unscoped.log")"
[ ! -s "$CALLS_UNSCOPED" ] \
    || fail "an unscoped package still invoked npm: $(cat "$CALLS_UNSCOPED")"

echo "PASS: an unscoped manifest name is refused before any npm call"

# --- 5. Dry-run: build and pack run, publish never does -----------------------
for d in "${BOTH_DIRS[@]}"; do
    stage_package "$d" "@tasker-systems/$(basename "$d")" "9.9.9"
done
CALLS_DRY="$TMP/calls-dry"
(cd "$REPO" && PATH="$STUB_DIR:$PATH" NPM_CALLS="$CALLS_DRY" NODE_AUTH_TOKEN=stub \
    bash "$TARGET" 9.9.9 --dry-run) > "$TMP/dry.log" 2>&1 \
    || fail "a dry-run failed: $(cat "$TMP/dry.log")"
grep -q "npm publish --dry-run" "$CALLS_DRY" \
    || fail "the dry-run never packed: $(cat "$CALLS_DRY")"
grep -q "npm publish --no-fund" "$CALLS_DRY" \
    && fail "the dry-run performed a real publish: $(cat "$CALLS_DRY")"
grep -q "would publish" "$TMP/dry.log" \
    || fail "the dry-run did not say what it would do: $(cat "$TMP/dry.log")"

echo "PASS: a dry-run packs but never publishes"
