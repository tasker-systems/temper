#!/usr/bin/env bash
# .github/scripts/test-create-github-release.sh
#
# Harness for create-github-release.sh's per-file-manifest guard.
#
# Why this guard needs a harness at all: install.sh deliberately treats an
# ABSENT published manifest as `unverifiable` and installs anyway, because every
# release up to v0.2.6 predates manifests and must stay installable forever.
# That is the right consumer behavior and it has a consequence — a release that
# silently loses its manifest is indistinguishable, to every consumer, from one
# that never had it. Installs quietly stop verifying and only warn.
#
# So the producer is the only place the invariant can live, and a producer-side
# guard that rots to a no-op would restore exactly the silence it was added to
# remove. Hence: a bash stub for `gh` and three cases.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TARGET="${SCRIPT_DIR}/release/create-github-release.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# Stub `gh`, logging every invocation. The log is what makes the ORDERING
# assertion possible: the guard must refuse before anything outward-facing is
# created, so a rejected release must leave this log empty.
STUB_DIR="$TMP/bin"
mkdir -p "$STUB_DIR"
cat > "$STUB_DIR/gh" <<'STUB'
#!/bin/sh
echo "$*" >> "$GH_CALLS"
exit 0
STUB
chmod +x "$STUB_DIR/gh"

# Build an artifact dir. Each named triple gets an archive + .sha256; a triple
# listed in $2 also gets a manifest.
stage_artifacts() {
    DIR="$1"; WITH_MANIFEST="$2"; shift 2
    rm -rf "$DIR"; mkdir -p "$DIR"
    for TRIPLE in "$@"; do
        case "$TRIPLE" in
            *windows*) EXT="zip" ;;
            *)         EXT="tar.gz" ;;
        esac
        printf 'archive' > "${DIR}/temper-v9.9.9-${TRIPLE}.${EXT}"
        printf 'sha  file\n' > "${DIR}/temper-v9.9.9-${TRIPLE}.${EXT}.sha256"
        case " $WITH_MANIFEST " in
            *" $TRIPLE "*)
                printf '{"version":"9.9.9","files":[]}' \
                    > "${DIR}/temper-v9.9.9-${TRIPLE}.manifest.json" ;;
        esac
    done
}

run_target() {
    PATH="$STUB_DIR:$PATH" GH_TOKEN=stub VERSION=9.9.9 ARTIFACT_DIR="$1" \
        GH_CALLS="$2" bash "$TARGET"
}

TRIPLES=(aarch64-apple-darwin x86_64-unknown-linux-gnu x86_64-pc-windows-msvc)
ALL_THREE="${TRIPLES[*]}"

# --- 1. Every archive has a manifest: the guard passes -----------------------
# The positive control. Without it, a guard that refused EVERYTHING would still
# satisfy both negative cases below.
stage_artifacts "$TMP/good" "$ALL_THREE" "${TRIPLES[@]}"
CALLS_GOOD="$TMP/calls-good"; : > "$CALLS_GOOD"
run_target "$TMP/good" "$CALLS_GOOD" > "$TMP/good.log" 2>&1 \
    || fail "a complete artifact set was refused: $(cat "$TMP/good.log")"
grep -q "release upload" "$CALLS_GOOD" \
    || fail "a complete set never reached the upload step: $(cat "$CALLS_GOOD")"

echo "PASS: a complete artifact set publishes"

# --- 2. One target lost its manifest: refuse, and create NOTHING -------------
# The bite. The missing manifest is on the linux target specifically, and the
# assertion names it — a guard that failed for any other reason would not
# produce this message. The empty call log is the second half: refusing after
# `gh release create` would leave a published tag whose assets install
# unverified, which is worse than not publishing at all.
stage_artifacts "$TMP/partial" \
    "aarch64-apple-darwin x86_64-pc-windows-msvc" "${TRIPLES[@]}"
CALLS_PARTIAL="$TMP/calls-partial"; : > "$CALLS_PARTIAL"
if run_target "$TMP/partial" "$CALLS_PARTIAL" > "$TMP/partial.log" 2>&1; then
    fail "an artifact set missing a manifest was published"
fi
grep -q "temper-v9.9.9-x86_64-unknown-linux-gnu.tar.gz has no per-file manifest" \
    "$TMP/partial.log" \
    || fail "the guard did not name the archive missing its manifest: $(cat "$TMP/partial.log")"
[ ! -s "$CALLS_PARTIAL" ] \
    || fail "the guard ran AFTER gh was invoked — a half-published release: $(cat "$CALLS_PARTIAL")"

echo "PASS: a target missing its manifest is refused before anything is created"

# --- 3. No archives at all: refuse rather than publish an empty release ------
# `nullglob` turns a build that produced nothing into a release with no assets
# and a green tick. Distinct message from case 2 so the two stay separable.
stage_artifacts "$TMP/empty" ""
CALLS_EMPTY="$TMP/calls-empty"; : > "$CALLS_EMPTY"
if run_target "$TMP/empty" "$CALLS_EMPTY" > "$TMP/empty.log" 2>&1; then
    fail "an empty artifact dir was published as a release"
fi
grep -q "no archives for v9.9.9" "$TMP/empty.log" \
    || fail "the empty-set refusal did not name the cause: $(cat "$TMP/empty.log")"
[ ! -s "$CALLS_EMPTY" ] || fail "an empty artifact dir still invoked gh"

echo "PASS: an artifact dir with no archives is refused"
