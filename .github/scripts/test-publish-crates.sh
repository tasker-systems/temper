#!/usr/bin/env bash
# .github/scripts/test-publish-crates.sh
#
# Harness for publish-crates.sh — the ordered publish, the per-crate duplicate
# probe, both version-agreement guards, and the dry-run surface.
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
CRATES=(temperkb-principal temperkb-auth temperkb-core temperkb-workflow temperkb-telemetry temperkb-client)

# Stub `cargo`: log every invocation, always succeed. `publish` calls carry
# `-p <crate>`; the log line is the observable.
STUB_DIR="$TMP/bin"
mkdir -p "$STUB_DIR"
cat > "$STUB_DIR/cargo" <<'STUB'
#!/bin/sh
echo "cargo $*" >> "$CARGO_CALLS"
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
        STUB_PUBLISHED_VERSION="${STUB_PUBLISHED_VERSION:-$VERSION}" \
        bash "$TARGET" "$@")
}

line_of() {
    grep -n "$1" "$2" | head -1 | cut -d: -f1
}

write_manifest "$VERSION" "$VERSION"

# --- 1. Fresh closure: probes decide first, publishes follow dependency order -
# The positive control: a guard that refused everything would satisfy the
# negative cases below without ever being right.
CALLS_FRESH="$TMP/calls-fresh"
run_target "$CALLS_FRESH" "$VERSION" > "$TMP/fresh.log" 2>&1 \
    || fail "a fresh publish was refused: $(cat "$TMP/fresh.log")"
for c in "${CRATES[@]}"; do
    grep -q "cargo publish -p $c" "$CALLS_FRESH" \
        || fail "$c never reached publish: $(cat "$CALLS_FRESH")"
    grep -q "crates/$c/versions" "$CALLS_FRESH" \
        || fail "the duplicate probe never ran for $c: $(cat "$CALLS_FRESH")"
done
# The FULL publish sequence must equal the dependency order — not merely a
# head-vs-tail spot check. Any permutation that puts a crate before something
# it regularly depends on fails here.
PUBLISH_ORDER="$(grep '^cargo publish -p ' "$CALLS_FRESH" | sed 's/^cargo publish -p //' | paste -sd, -)"
EXPECTED_ORDER="$(IFS=,; echo "${CRATES[*]}")"
[ "$PUBLISH_ORDER" = "$EXPECTED_ORDER" ] \
    || fail "the publish sequence departed from dependency order: got [$PUBLISH_ORDER], want [$EXPECTED_ORDER] — call log: $(cat "$CALLS_FRESH")"
FIRST_CORE_PROBE=$(line_of "crates/temperkb-core/versions" "$CALLS_FRESH")
[ "$FIRST_CORE_PROBE" -gt "$(line_of 'publish -p temperkb-auth' "$CALLS_FRESH")" ] \
    && [ "$FIRST_CORE_PROBE" -lt "$(line_of 'publish -p temperkb-core' "$CALLS_FRESH")" ] \
    || fail "temperkb-core's probe did not sit between its dependency's and its own publish — the probe is decorative: $(cat "$CALLS_FRESH")"

echo "PASS: a fresh closure probes each crate, then publishes in dependency order"

# --- 2. Fully-published closure: loud skip, no publish runs -------------------
# The bite: a re-cut release must skip all six loudly and exit 0 (idempotent,
# like create-github-release.sh's "already exists").
CALLS_SKIP="$TMP/calls-skip"
STUB_PUBLISHED="$(IFS=,; echo "${CRATES[*]}")" run_target "$CALLS_SKIP" "$VERSION" > "$TMP/skip.log" 2>&1 \
    || fail "a fully-published re-run exited non-zero (idempotent skip must exit 0): $(cat "$TMP/skip.log")"
grep -q "cargo publish -p" "$CALLS_SKIP" \
    && fail "a fully-published closure still published: $(cat "$CALLS_SKIP")"
grep -c "already published — skipping" "$TMP/skip.log" | grep -q "^6$" \
    || fail "all six crates did not skip loudly: $(cat "$TMP/skip.log")"

echo "PASS: a fully-published closure skips loudly, exits 0, and never publishes"

# --- 3. Partially-published closure: resume from the first missing crate ------
# A release that died after three crates must not re-publish them on re-run;
# the next publish starts at the tail.
CALLS_PARTIAL="$TMP/calls-partial"
STUB_PUBLISHED="temperkb-principal,temperkb-auth,temperkb-telemetry" run_target "$CALLS_PARTIAL" "$VERSION" > "$TMP/partial.log" 2>&1 \
    || fail "a partial re-run failed: $(cat "$TMP/partial.log")"
grep -q "cargo publish -p temperkb-principal" "$CALLS_PARTIAL" \
    && fail "an already-published crate was re-published: $(cat "$CALLS_PARTIAL")"
FIRST_PUBLISH=$(line_of "cargo publish -p " "$CALLS_PARTIAL")
[ "$(sed -n "${FIRST_PUBLISH}p" "$CALLS_PARTIAL")" = "cargo publish -p temperkb-core" ] \
    || fail "the partial re-run did not resume at temperkb-core: $(cat "$CALLS_PARTIAL")"

echo "PASS: a partial closure resumes at the first missing crate"

# --- 4. Workspace version disagrees with the tag: refuse ----------------------
write_manifest "0.0.0" "$VERSION"
CALLS_WS="$TMP/calls-ws"
if run_target "$CALLS_WS" "$VERSION" > "$TMP/ws.log" 2>&1; then
    fail "a workspace-version mismatch published anyway"
fi
grep -q "workspace.package. version is 0.0.0" "$TMP/ws.log" \
    || fail "the refusal did not name the disagreeing section: $(cat "$TMP/ws.log")"
[ ! -s "$CALLS_WS" ] \
    || fail "a version-mismatched release still invoked cargo or curl: $(cat "$CALLS_WS")"

echo "PASS: a [workspace.package] version that disagrees with the tag is refused before any call"

# --- 5. One dependency-spec version disagrees: refuse -------------------------
# The six [workspace.dependencies] versions are spelled out because dependency
# specs cannot inherit [workspace.package]; this guard is what keeps them
# bumpable-together. One drifted line must stop the release, naming the crate.
write_manifest "$VERSION" "0.0.0"
CALLS_DEP="$TMP/calls-dep"
if run_target "$CALLS_DEP" "$VERSION" > "$TMP/dep.log" 2>&1; then
    fail "a dependency-version mismatch published anyway"
fi
grep -q "temperkb-core's .workspace.dependencies. version is 0.0.0" "$TMP/dep.log" \
    || fail "the refusal did not name the disagreeing crate: $(cat "$TMP/dep.log")"
[ ! -s "$CALLS_DEP" ] \
    || fail "a dependency-version mismatch still invoked cargo or curl: $(cat "$CALLS_DEP")"

echo "PASS: a drifted [workspace.dependencies] version is refused, naming the crate"

# --- 6. Dry-run: validates each crate but never publishes ---------------------
write_manifest "$VERSION" "$VERSION"
CALLS_DRY="$TMP/calls-dry"
run_target "$CALLS_DRY" "$VERSION" --dry-run > "$TMP/dry.log" 2>&1 \
    || fail "a dry-run failed: $(cat "$TMP/dry.log")"
grep -c "cargo publish --dry-run" "$CALLS_DRY" | grep -q "^6$" \
    || fail "the dry-run did not validate all six crates: $(cat "$CALLS_DRY")"
grep -E "^cargo publish -p " "$CALLS_DRY" \
    && fail "the dry-run performed a real publish: $(cat "$CALLS_DRY")"

echo "PASS: a dry-run validates all six crates and never publishes"

echo "ALL PASS"
