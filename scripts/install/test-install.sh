#!/usr/bin/env bash
# Harness for install.sh. Feeds it a locally-built archive so the download path
# is bypassed, and asserts the flags and gates behave.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL="${SCRIPT_DIR}/install.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }

# Build a fake release archive whose "binary" is a script that prints a version,
# so install.sh's run-gate (install.sh:171) can pass without a real build.
build_archive() {
    STAGE="$TMP/stage"; rm -rf "$STAGE"; mkdir -p "$STAGE/lib"
    printf '#!/bin/sh\necho "0.3.0"\n' > "$STAGE/temper"
    chmod +x "$STAGE/temper"
    printf 'ort' > "$STAGE/lib/libonnxruntime.so"
    tar -czf "$TMP/archive.tar.gz" -C "$STAGE" .
}

# Manifest producer used by both this harness's baseline install and the
# tampered-archive test below. Runs the real release-workflow script
# (`.github/scripts/release/emit-manifest.sh`) against a staging dir so the
# manifest shape the harness feeds install.sh is the one CI actually publishes
# — jq is fine here (the harness runs in bash/CI), even though install.sh
# itself must not depend on it.
build_manifest() {
    STAGE="$1"; OUT="$2"
    VERSION=0.3.0 TARGET=x86_64-unknown-linux-gnu STAGING="$STAGE" OUTPUT="$OUT" \
        bash "$(cd "$SCRIPT_DIR/../../.github/scripts/release" && pwd)/emit-manifest.sh" >/dev/null
}

build_archive
build_manifest "$TMP/stage" "$TMP/manifest.json"

# --archive/--manifest use the supplied files and never reach the network.
# Unsetting the network is proven by pointing the repo at an unroutable host:
# if install.sh tried to download, it would fail.
INSTALL_DIR="$TMP/install"
TEMPER_INSTALL_DIR="$INSTALL_DIR" XDG_BIN_HOME="$TMP/bin" \
    sh "$INSTALL" --archive "$TMP/archive.tar.gz" --manifest "$TMP/manifest.json" \
       --version v0.3.0 >/dev/null 2>&1 \
    || fail "--archive install failed"

[ -x "$INSTALL_DIR/temper" ] || fail "binary not installed"
[ -f "$INSTALL_DIR/lib/libonnxruntime.so" ] || fail "ort lib not installed"
[ -f "$INSTALL_DIR/.temper-manifest.json" ] || fail "manifest not written into install dir"

echo "PASS: install.sh --archive installs from a local archive without downloading"

# --archive without --manifest has no source for per-file verification and
# must be a hard error, not a silent skip.
INSTALL_DIR2="$TMP/install-no-manifest"
if TEMPER_INSTALL_DIR="$INSTALL_DIR2" XDG_BIN_HOME="$TMP/bin-no-manifest" \
     sh "$INSTALL" --archive "$TMP/archive.tar.gz" --version v0.3.0 >/dev/null 2>&1; then
    fail "--archive without --manifest installed successfully — per-file verification was skipped"
fi
[ ! -e "$INSTALL_DIR2" ] || fail "a rejected --manifest-less install left files behind"

echo "PASS: install.sh --archive without --manifest is rejected"

# --- A tampered file must roll back, not install ------------------------------
# The bite: alter one byte of the binary AFTER the manifest is written, and the
# post-swap gate must reject the install and leave the PRIOR install in place.

# 1. Establish a known-good prior install.
GOOD_DIR="$TMP/install2"
build_archive
build_manifest "$TMP/stage" "$TMP/good.manifest.json"
TEMPER_INSTALL_DIR="$GOOD_DIR" XDG_BIN_HOME="$TMP/bin2" \
    sh "$INSTALL" --archive "$TMP/archive.tar.gz" --manifest "$TMP/good.manifest.json" \
       --version v0.3.0 >/dev/null 2>&1 || fail "baseline install failed"
BASELINE=$(cat "$GOOD_DIR/temper")

# 2. Build a TAMPERED archive whose manifest no longer describes it.
rm -rf "$TMP/stage"; mkdir -p "$TMP/stage/lib"
printf '#!/bin/sh\necho "0.3.0"\n' > "$TMP/stage/temper"; chmod +x "$TMP/stage/temper"
printf 'ort' > "$TMP/stage/lib/libonnxruntime.so"
build_manifest "$TMP/stage" "$TMP/tampered.manifest.json"
printf '#!/bin/sh\necho "0.3.0"\n# EVIL\n' > "$TMP/stage/temper"; chmod +x "$TMP/stage/temper"
tar -czf "$TMP/tampered.tar.gz" -C "$TMP/stage" .

# 3. Installing it must FAIL and leave the baseline intact.
if TEMPER_INSTALL_DIR="$GOOD_DIR" XDG_BIN_HOME="$TMP/bin2" \
     sh "$INSTALL" --archive "$TMP/tampered.tar.gz" --manifest "$TMP/tampered.manifest.json" \
        --version v0.3.0 >/dev/null 2>&1; then
    fail "tampered archive installed successfully — the manifest gate does not bite"
fi
[ "$(cat "$GOOD_DIR/temper")" = "$BASELINE" ] \
    || fail "prior install was clobbered by a rejected update"

echo "PASS: a manifest mismatch is rejected and the prior install survives"

# --- Isolate the post-extract gate from the post-swap gate -------------------
# The test above only proves SOME gate rejected the install; it doesn't prove
# WHICH one. Two load-bearing gates (post-extract, post-swap) sharing a single
# witness means either could silently rot to a no-op and this suite would stay
# green — the "test that can no longer fail" failure mode. This test pins the
# EARLY (post-extract) gate specifically, using a fresh install dir (nothing
# to roll back to) plus the two gates' distinguishable stderr messages:
#   post-extract: "file manifest verification failed"
#   post-swap:    "rolling back"
# and confirms the install dir was never even created — the post-extract gate
# fires before the atomic swap touches anything.
FRESH_DIR="$TMP/install-fresh"
STDERR_LOG="$TMP/fresh-stderr.log"
rm -rf "$FRESH_DIR"
if TEMPER_INSTALL_DIR="$FRESH_DIR" XDG_BIN_HOME="$TMP/bin-fresh" \
     sh "$INSTALL" --archive "$TMP/tampered.tar.gz" --manifest "$TMP/tampered.manifest.json" \
        --version v0.3.0 >/dev/null 2>"$STDERR_LOG"; then
    fail "tampered archive installed successfully on a fresh install — the post-extract gate does not bite"
fi
grep -q "file manifest verification failed" "$STDERR_LOG" \
    || fail "post-extract gate did not fire (expected message missing from stderr): $(cat "$STDERR_LOG")"
grep -q "rolling back" "$STDERR_LOG" \
    && fail "post-swap gate fired instead of post-extract — the early gate did not catch it first"
[ ! -e "$FRESH_DIR" ] || fail "a fresh install directory was created despite the post-extract gate rejecting the install"

echo "PASS: a tampered archive is rejected by the post-extract gate specifically (isolated from post-swap)"

# --- A manifest that verifies nothing must not verify everything -------------
# Reproduced during Arc 2: an archive whose binary contained `# EVIL PAYLOAD`
# installed with exit 0 and printed "✓ Installed", because zero parsed entries
# left CHECK_FAILED at 0. Both variants below reach that same fail-open.

# Rebuild a known-good archive+manifest pair to test against.
build_archive
build_manifest "$TMP/stage" "$TMP/vacuity.manifest.json"

# 1. An empty file list: the manifest genuinely asserts nothing.
printf '{"version":"0.3.0","target":"x86_64-unknown-linux-gnu","files":[]}\n' \
  > "$TMP/empty.manifest.json"
if TEMPER_INSTALL_DIR="$TMP/install-empty" XDG_BIN_HOME="$TMP/bin-empty" \
     sh "$INSTALL" --archive "$TMP/archive.tar.gz" --manifest "$TMP/empty.manifest.json" \
        --version v0.3.0 >/dev/null 2>&1; then
  fail "installer accepted a manifest listing zero files"
fi
[ ! -e "$TMP/install-empty" ] || fail "a rejected empty-manifest install left files behind"

echo "PASS: a manifest listing zero files is refused"

# 2. Compact JSON: the manifest declares real entries, but the awk pair parser
#    emits none (the /"path":/ rule ends in `next`, so /"sha256":/ never fires
#    when both keys share a line). The bytes are otherwise IDENTICAL to the
#    good manifest — same files, same hashes — so this is not "compact JSON is
#    invalid"; it is "a partial parse must refuse, not silently verify nothing."
#    The zero floor alone would catch this too, but the count cross-check is
#    what makes the error message true.
jq -c . < "$TMP/vacuity.manifest.json" > "$TMP/compact.manifest.json"
if TEMPER_INSTALL_DIR="$TMP/install-compact" XDG_BIN_HOME="$TMP/bin-compact" \
     sh "$INSTALL" --archive "$TMP/archive.tar.gz" --manifest "$TMP/compact.manifest.json" \
        --version v0.3.0 >/dev/null 2>&1; then
  fail "installer accepted a compact manifest whose entries it could not parse"
fi

echo "PASS: a manifest the parser cannot read is refused, not silently accepted"
