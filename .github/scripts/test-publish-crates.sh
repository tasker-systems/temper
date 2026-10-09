#!/usr/bin/env bash
# .github/scripts/test-publish-crates.sh
#
# Harness for publish-crates.sh — its two stages (`--out DIR` verify-packages the closure;
# `--from DIR` re-packages each crate without compiling, requires the verified bytes, and
# publishes `--no-verify`), the ordered publish, the per-crate duplicate probe, both
# version-agreement guards, and the dry-run surface.
#
# Why a harness: the crates.io lane runs for real only at a tagged release, so
# the loud-skip path (a re-cut release, or a re-run after a partial publish,
# must skip what is already published — never re-publish it) would otherwise be
# witnessed by an actual registry push. Bash stubs for `cargo` and `curl` make
# the call ORDER observable — the probe must decide before each publish, and
# the closure must publish in dependency order — and let the skip, mismatch,
# and dry-run paths be bitten on every PR rather than on a release.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="${SCRIPT_DIR}/../../tools/scripts/release/publish-crates.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

VERSION="9.9.9"
# Must match publish-crates.sh's CRATES array — this IS the expected publish
# sequence case 1 asserts against. (Dev-deps impose no order; see the script.)
CRATES=(temperkb-principal temperkb-auth temperkb-core temperkb-workflow temperkb-telemetry temperkb-client temperkb-mcp)

# Stub `cargo`: log every invocation, always succeed. `package` writes
# target/package/<crate>-<version>.crate for each `-p <crate>`, with bytes that depend only on the
# crate — so packaging is deterministic, as cargo's is, and a tampered verified file differs.
STUB_DIR="$TMP/bin"
mkdir -p "$STUB_DIR"
cat > "$STUB_DIR/cargo" <<'STUB'
#!/bin/sh
echo "cargo $*" >> "$CARGO_CALLS"
if [ "$1" = "package" ]; then
    mkdir -p target/package
    prev=""
    for a in "$@"; do
        if [ "$prev" = "-p" ]; then
            printf '%s crate bytes\n' "$a" > "target/package/$a-${STUB_VERSION}.crate"
        fi
        prev="$a"
    done
fi
exit 0
STUB
chmod +x "$STUB_DIR/cargo"

# Stub `curl`: extract the crate name from the probe URL. If the crate is in
# $STUB_PUBLISHED (comma-separated), answer as published; otherwise answer an
# empty version list.
cat > "$STUB_DIR/curl" <<'STUB'
#!/bin/sh
echo "curl $*" >> "$CARGO_CALLS"
url=""
prev=""
for a in "$@"; do
    if [ "$prev" = "-A" ]; then prev="$a"; continue; fi
    case "$a" in https://crates.io/*) url="$a" ;; esac
    prev="$a"
done
crate="$(printf '%s' "$url" | sed -E 's|.*/crates/(temperkb-[a-z]+)/versions|\1|')"
case ",$STUB_PUBLISHED," in
    *",$crate,"*)
        printf '{"versions":[{"num":"%s"}]}\n' "${STUB_PUBLISHED_VERSION:-9.9.9}"
        exit 0
        ;;
esac
printf '{"versions":[]}\n'
exit 0
STUB
chmod +x "$STUB_DIR/curl"

REPO="$TMP/repo"
git init -q "$REPO"

write_manifest() {
    local ws_version="$1" core_dep_version="$2"
    {
        echo '[workspace.package]'
        printf 'version = "%s"\n' "$ws_version"
        echo ''
        echo '[workspace.dependencies]'
        for c in "${CRATES[@]}"; do
            v="$ws_version"
            [ "$c" = "temperkb-core" ] && v="$core_dep_version"
            printf '%s = { path = "crates/%s", version = "%s" }\n' "$c" "$c" "$v"
        done
    } > "${REPO}/Cargo.toml"
}

run_target() {
    CALLS="$1"; shift
    : > "$CALLS"
    (cd "$REPO" && PATH="$STUB_DIR:$PATH" CARGO_CALLS="$CALLS" \
        STUB_PUBLISHED="${STUB_PUBLISHED:-}" \
        STUB_PUBLISHED_VERSION="${STUB_PUBLISHED_VERSION:-$VERSION}" STUB_VERSION="$VERSION" \
        bash "$TARGET" "$@")
}

line_of() {
    grep -n "$1" "$2" | head -1 | cut -d: -f1
}

write_manifest "$VERSION" "$VERSION"

OUT="$TMP/out"

# --- 1. Build stage: guards, then one verifying package of the whole closure, no probe --
# The positive control: a guard that refused everything would satisfy the negative cases below
# without ever being right.
CALLS_BUILD="$TMP/calls-build"
run_target "$CALLS_BUILD" "$VERSION" --out "$OUT" > "$TMP/build.log" 2>&1 \
    || fail "a fresh build was refused: $(cat "$TMP/build.log")"
PKG_LINE="$(grep '^cargo package' "$CALLS_BUILD")"
[ "$(printf '%s\n' "$PKG_LINE" | grep -c .)" = "1" ] \
    || fail "the build stage did not package the closure in one call: $(cat "$CALLS_BUILD")"
printf '%s' "$PKG_LINE" | grep -q -- "--no-verify" \
    && fail "the build stage skipped verification — it is the stage that compiles: $PKG_LINE"
for c in "${CRATES[@]}"; do
    printf '%s' "$PKG_LINE" | grep -q -- "-p $c" || fail "the build stage did not package $c: $PKG_LINE"
    [ -s "$OUT/$c-$VERSION.crate" ] || fail "$c-$VERSION.crate was not copied out: $(ls -la "$OUT")"
done
grep -Eq "^(cargo publish|curl)" "$CALLS_BUILD" \
    && fail "the build stage probed or published: $(cat "$CALLS_BUILD")"

echo "PASS: the build stage verify-packages the whole closure in one call and publishes nothing"

# --- 2. Publish stage, fresh closure: probe, re-package unverified, publish, in order ---
CALLS_FRESH="$TMP/calls-fresh"
run_target "$CALLS_FRESH" "$VERSION" --from "$OUT" > "$TMP/fresh.log" 2>&1 \
    || fail "a fresh publish was refused: $(cat "$TMP/fresh.log")"
for c in "${CRATES[@]}"; do
    grep -q "cargo publish --no-verify -p $c" "$CALLS_FRESH" \
        || fail "$c never reached an unverified publish: $(cat "$CALLS_FRESH")"
    grep -q "crates/$c/versions" "$CALLS_FRESH" \
        || fail "the duplicate probe never ran for $c: $(cat "$CALLS_FRESH")"
done
grep -E "^cargo (package|publish)" "$CALLS_FRESH" | grep -vq -- "--no-verify" \
    && fail "the publish stage compiled (a package or publish without --no-verify): $(cat "$CALLS_FRESH")"
# The FULL publish sequence must equal the dependency order — not merely a head-vs-tail spot
# check. Any permutation that puts a crate before something it regularly depends on fails here.
PUBLISH_ORDER="$(grep '^cargo publish --no-verify -p ' "$CALLS_FRESH" | sed 's/^cargo publish --no-verify -p //' | paste -sd, -)"
EXPECTED_ORDER="$(IFS=,; echo "${CRATES[*]}")"
[ "$PUBLISH_ORDER" = "$EXPECTED_ORDER" ] \
    || fail "the publish sequence departed from dependency order: got [$PUBLISH_ORDER], want [$EXPECTED_ORDER] — call log: $(cat "$CALLS_FRESH")"
FIRST_CORE_PROBE=$(line_of "crates/temperkb-core/versions" "$CALLS_FRESH")
[ "$FIRST_CORE_PROBE" -gt "$(line_of 'publish --no-verify -p temperkb-auth' "$CALLS_FRESH")" ] \
    && [ "$FIRST_CORE_PROBE" -lt "$(line_of 'publish --no-verify -p temperkb-core' "$CALLS_FRESH")" ] \
    || fail "temperkb-core's probe did not sit between its dependency's and its own publish — the probe is decorative: $(cat "$CALLS_FRESH")"
CORE_PKG=$(line_of "package --no-verify -p temperkb-core" "$CALLS_FRESH")
[ -n "$CORE_PKG" ] && [ "$CORE_PKG" -lt "$(line_of 'publish --no-verify -p temperkb-core' "$CALLS_FRESH")" ] \
    || fail "temperkb-core was not re-packaged and compared before its publish: $(cat "$CALLS_FRESH")"

echo "PASS: the publish stage probes, re-packages and compares, then publishes each crate unverified, in order"

# --- 3. Fully-published closure: loud skip, no publish runs ----------------------------
# The bite: a re-cut release must skip every crate loudly and exit 0 (idempotent, like
# create-github-release.sh's "already exists").
CALLS_SKIP="$TMP/calls-skip"
STUB_PUBLISHED="$(IFS=,; echo "${CRATES[*]}")" run_target "$CALLS_SKIP" "$VERSION" --from "$OUT" > "$TMP/skip.log" 2>&1 \
    || fail "a fully-published re-run exited non-zero (idempotent skip must exit 0): $(cat "$TMP/skip.log")"
grep -q "cargo publish" "$CALLS_SKIP" \
    && fail "a fully-published closure still published: $(cat "$CALLS_SKIP")"
grep -c "already published — skipping" "$TMP/skip.log" | grep -q "^${#CRATES[@]}$" \
    || fail "not all ${#CRATES[@]} crates skipped loudly: $(cat "$TMP/skip.log")"

echo "PASS: a fully-published closure skips loudly, exits 0, and never publishes"

# --- 4. Partially-published closure: resume from the first missing crate ---------------
# A release that died after three crates must not re-publish them on re-run; the next publish
# starts at the tail.
CALLS_PARTIAL="$TMP/calls-partial"
STUB_PUBLISHED="temperkb-principal,temperkb-auth,temperkb-telemetry" run_target "$CALLS_PARTIAL" "$VERSION" --from "$OUT" > "$TMP/partial.log" 2>&1 \
    || fail "a partial re-run failed: $(cat "$TMP/partial.log")"
grep -q "cargo publish --no-verify -p temperkb-principal" "$CALLS_PARTIAL" \
    && fail "an already-published crate was re-published: $(cat "$CALLS_PARTIAL")"
FIRST_PUBLISH=$(line_of "cargo publish " "$CALLS_PARTIAL")
[ "$(sed -n "${FIRST_PUBLISH}p" "$CALLS_PARTIAL")" = "cargo publish --no-verify -p temperkb-core" ] \
    || fail "the partial re-run did not resume at temperkb-core: $(cat "$CALLS_PARTIAL")"

echo "PASS: a partial closure resumes at the first missing crate"

# --- 5. A verified .crate that differs from the re-packaged one: refuse ----------------
TAMPER="$TMP/tamper"
cp -R "$OUT" "$TAMPER"
printf 'x' >> "$TAMPER/temperkb-principal-$VERSION.crate"
CALLS_TAMPER="$TMP/calls-tamper"
if run_target "$CALLS_TAMPER" "$VERSION" --from "$TAMPER" > "$TMP/tamper.log" 2>&1; then
    fail "a .crate differing from the verified one was published"
fi
grep -q "temperkb-principal-$VERSION.crate packaged here differs from the one the build job verified" "$TMP/tamper.log" \
    || fail "the refusal did not name the differing crate: $(cat "$TMP/tamper.log")"
grep -q "cargo publish" "$CALLS_TAMPER" && fail "a differing crate still reached publish: $(cat "$CALLS_TAMPER")"

echo "PASS: the publish stage refuses a crate whose bytes differ from the verified package"

# --- 6. An unpublished crate with no verified .crate: refuse ---------------------------
mkdir -p "$TMP/empty"
if run_target "$TMP/calls-missing" "$VERSION" --from "$TMP/empty" > "$TMP/missing.log" 2>&1; then
    fail "a missing verified .crate for an unpublished crate exited 0"
fi
grep -q "temperkb-principal-$VERSION.crate is not in $TMP/empty" "$TMP/missing.log" \
    || fail "the refusal did not say what is missing: $(cat "$TMP/missing.log")"

echo "PASS: an unpublished crate whose verified package is absent fails loudly"

# --- 7. Workspace version disagrees with the tag: refuse --------------------------------
write_manifest "0.0.0" "$VERSION"
CALLS_WS="$TMP/calls-ws"
if run_target "$CALLS_WS" "$VERSION" --out "$TMP/ws-out" > "$TMP/ws.log" 2>&1; then
    fail "a workspace-version mismatch built anyway"
fi
grep -q "workspace.package. version is 0.0.0" "$TMP/ws.log" \
    || fail "the refusal did not name the disagreeing section: $(cat "$TMP/ws.log")"
[ ! -s "$CALLS_WS" ] \
    || fail "a version-mismatched release still invoked cargo or curl: $(cat "$CALLS_WS")"

echo "PASS: a [workspace.package] version that disagrees with the tag is refused before any call"

# --- 8. One dependency-spec version disagrees: refuse ----------------------------------
# The temperkb-* [workspace.dependencies] versions are spelled out because dependency specs cannot
# inherit [workspace.package]; this guard is what keeps them bumpable-together. One drifted line
# must stop the release, naming the crate.
write_manifest "$VERSION" "0.0.0"
CALLS_DEP="$TMP/calls-dep"
if run_target "$CALLS_DEP" "$VERSION" --out "$TMP/dep-out" > "$TMP/dep.log" 2>&1; then
    fail "a dependency-version mismatch built anyway"
fi
grep -q "temperkb-core's .workspace.dependencies. version is 0.0.0" "$TMP/dep.log" \
    || fail "the refusal did not name the disagreeing crate: $(cat "$TMP/dep.log")"
[ ! -s "$CALLS_DEP" ] \
    || fail "a dependency-version mismatch still invoked cargo or curl: $(cat "$CALLS_DEP")"

echo "PASS: a drifted [workspace.dependencies] version is refused, naming the crate"

# --- 9. Dry-run: the first unpublished crate is packaged and compared, nothing published -
# It stops at that crate: the next one cannot be packaged on its own until this one is on the
# registry.
write_manifest "$VERSION" "$VERSION"
CALLS_DRY="$TMP/calls-dry"
run_target "$CALLS_DRY" "$VERSION" --from "$OUT" --dry-run > "$TMP/dry.log" 2>&1 \
    || fail "a dry-run failed: $(cat "$TMP/dry.log")"
grep -q "package --no-verify -p temperkb-principal" "$CALLS_DRY" \
    || fail "the dry-run did not package and compare the first crate: $(cat "$CALLS_DRY")"
grep -q "cargo publish" "$CALLS_DRY" && fail "the dry-run published: $(cat "$CALLS_DRY")"
grep -q "matches the verified package" "$TMP/dry.log" \
    || fail "the dry-run did not report the comparison: $(cat "$TMP/dry.log")"

echo "PASS: a dry-run compares the first unpublished crate and never publishes"

# --- 10. Stage flags: exactly one of --out/--from, and --dry-run only with --from ------
for args in "$VERSION" "$VERSION --out a --from b" "$VERSION --out a --dry-run"; do
    # shellcheck disable=SC2086
    if run_target "$TMP/calls-flags" $args > "$TMP/flags.log" 2>&1; then
        fail "'$args' was accepted"
    fi
done

echo "PASS: a call naming no stage, both stages, or a build-stage dry-run is refused"

echo "ALL PASS"
