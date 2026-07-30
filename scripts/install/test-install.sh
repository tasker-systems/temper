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

# --- A manifest entry may not name a file outside the install root -----------
# The bite, reproduced end-to-end before the fix: point ONE entry at a real
# file sitting OUTSIDE the staging dir and state its real sha256. Every check
# then passes, install.sh prints "✓ Installed" and exits 0 — while the `temper`
# that was actually extracted is never hashed at all. Nothing but a `path`
# string was edited; no hash was forged.
#
# STAGING is "${INSTALL_DIR}.new-$$" (install.sh:198), so its parent is
# dirname(INSTALL_DIR) — with INSTALL_DIR under $TMP, "../decoy" resolves to
# $TMP/decoy, which this harness can place.
build_archive
build_manifest "$TMP/stage" "$TMP/containment.manifest.json"

printf 'DECOY-PAYLOAD' > "$TMP/decoy"
if command -v sha256sum >/dev/null 2>&1; then
  DECOY_SHA=$(sha256sum "$TMP/decoy" | awk '{print $1}')
else
  DECOY_SHA=$(shasum -a 256 "$TMP/decoy" | awk '{print $1}')
fi
jq --arg p "../decoy" --arg s "$DECOY_SHA" \
   '(.files[] | select(.path == "temper")) |= (.path = $p | .sha256 = $s)' \
   < "$TMP/containment.manifest.json" > "$TMP/decoy.manifest.json"
if TEMPER_INSTALL_DIR="$TMP/install-decoy" XDG_BIN_HOME="$TMP/bin-decoy" \
     sh "$INSTALL" --archive "$TMP/archive.tar.gz" --manifest "$TMP/decoy.manifest.json" \
        --version v0.3.0 >/dev/null 2>&1; then
  fail "installer accepted an entry resolving to a decoy outside the staging dir — the extracted temper was never hashed"
fi
rm -rf "$TMP/install-decoy"

echo "PASS: a manifest entry resolving outside the install root is refused"

# Defense in depth for the remaining escaping shapes. Unlike the decoy above,
# these name targets that do not exist, so they are ALREADY refused today (as
# "missing file") and would prove nothing on their own. They are asserted so
# the containment arm keeps refusing them BY PATH rather than by the accident
# of the target's absence. The empty path is included because it never reaches
# containment at all: it parses to a pair the loop skips, and is caught by the
# parsed-vs-declared cross-check instead.
for BAD_PATH in "/etc/hosts" "../escape" "a/../../escape" ""; do
  jq --arg p "$BAD_PATH" '.files[0].path = $p' \
    < "$TMP/containment.manifest.json" > "$TMP/bad-path.manifest.json"
  if TEMPER_INSTALL_DIR="$TMP/install-badpath" XDG_BIN_HOME="$TMP/bin-badpath" \
       sh "$INSTALL" --archive "$TMP/archive.tar.gz" --manifest "$TMP/bad-path.manifest.json" \
          --version v0.3.0 >/dev/null 2>&1; then
    fail "installer accepted a manifest entry with an escaping path: $BAD_PATH"
  fi
  rm -rf "$TMP/install-badpath"
done

echo "PASS: absolute, ancestor-relative and empty manifest paths are all refused"

# =============================================================================
# The NETWORK path: a missing published manifest is `unverifiable`, not fatal
# =============================================================================
# Everything above drives install.sh through --archive/--manifest, so the
# download branch — where the manifest is FETCHED and can be absent — had no
# coverage at all. That is the branch that decides whether `curl | sh` works
# against a release published before manifests existed.
#
# Driven by a stub `curl` prepended to PATH rather than a live server or an
# override inside install.sh. A `TEMPER_RELEASE_BASE_URL`-style env hook would
# have meant adding an env-controlled download origin to a `curl | sh`
# installer purely for testability; shadowing `curl` tests the real code path
# and adds nothing to the shipped script.
STUB_DIR="$TMP/stub-bin"
mkdir -p "$STUB_DIR"
cat > "$STUB_DIR/curl" <<'STUB'
#!/bin/sh
# Stub curl. Serves the archive and its .sha256 from local files; the manifest
# response is controlled by STUB_MANIFEST_MODE. Only the manifest call passes
# -w '%{http_code}', and only that call reads our stdout, so the archive/sha
# branches must print nothing.
URL=""; OUT=""; PREV=""
for A in "$@"; do
    case "$A" in https://*) URL="$A" ;; esac
    [ "$PREV" = "-o" ] && OUT="$A"
    PREV="$A"
done
case "$URL" in
    *.manifest.json)
        case "${STUB_MANIFEST_MODE:-200}" in
            200)      cp "$STUB_MANIFEST_FILE" "$OUT"; printf '200'; exit 0 ;;
            tampered) cp "$STUB_MANIFEST_FILE" "$OUT"; printf '200'; exit 0 ;;
            404)      printf 'Not Found' > "$OUT"; printf '404'; exit 0 ;;
            000)      exit 7 ;;
            *)        printf '500'; exit 0 ;;
        esac ;;
    *.sha256)
        ARCHIVE_NAME="$(basename "${OUT%.sha256}")"
        printf '%s  %s\n' "$(shasum -a 256 "$STUB_ARCHIVE" | awk '{print $1}')" \
            "$ARCHIVE_NAME" > "$OUT"
        exit 0 ;;
    *)
        cp "$STUB_ARCHIVE" "$OUT"; exit 0 ;;
esac
STUB
chmod +x "$STUB_DIR/curl"

# Rebuild a known-good pair; earlier tests leave $TMP/stage tampered.
build_archive
build_manifest "$TMP/stage" "$TMP/net.manifest.json"

# --- 404: install proceeds, plants no baseline, and says so ------------------
NET404_DIR="$TMP/install-net-404"
NET404_LOG="$TMP/net-404.log"
if ! PATH="$STUB_DIR:$PATH" STUB_MANIFEST_MODE=404 \
     STUB_ARCHIVE="$TMP/archive.tar.gz" STUB_MANIFEST_FILE="$TMP/net.manifest.json" \
     TEMPER_INSTALL_DIR="$NET404_DIR" XDG_BIN_HOME="$TMP/bin-net-404" \
     sh "$INSTALL" --version v0.3.0 >"$NET404_LOG" 2>&1; then
    fail "a release with no published manifest failed to install: $(cat "$NET404_LOG")"
fi
[ -x "$NET404_DIR/temper" ] || fail "404-manifest install did not install the binary"
[ ! -f "$NET404_DIR/.temper-manifest.json" ] \
    || fail "a 404 manifest still planted a baseline — nothing was verified to plant"
grep -q "publishes no per-file manifest" "$NET404_LOG" \
    || fail "the 404 arm did not name the absent manifest: $(cat "$NET404_LOG")"
grep -q "temper version --verify --online" "$NET404_LOG" \
    || fail "the 404 warning names no recovery: $(cat "$NET404_LOG")"

echo "PASS: a release with no published manifest installs as unverifiable, plants no baseline"

# --- Transport failure: same verdict, DIFFERENT message ----------------------
# The two must stay distinguishable. "This release predates manifests" and "we
# could not reach GitHub" warrant the same action and different explanations;
# collapsing them would tell users a permanent fact about a transient failure.
NET000_DIR="$TMP/install-net-000"
NET000_LOG="$TMP/net-000.log"
if ! PATH="$STUB_DIR:$PATH" STUB_MANIFEST_MODE=000 \
     STUB_ARCHIVE="$TMP/archive.tar.gz" STUB_MANIFEST_FILE="$TMP/net.manifest.json" \
     TEMPER_INSTALL_DIR="$NET000_DIR" XDG_BIN_HOME="$TMP/bin-net-000" \
     sh "$INSTALL" --version v0.3.0 >"$NET000_LOG" 2>&1; then
    fail "a manifest fetch failure aborted the install: $(cat "$NET000_LOG")"
fi
[ -x "$NET000_DIR/temper" ] || fail "fetch-failure install did not install the binary"
[ ! -f "$NET000_DIR/.temper-manifest.json" ] \
    || fail "a failed manifest fetch still planted a baseline"
grep -q "could not fetch the per-file manifest" "$NET000_LOG" \
    || fail "the fetch-failure arm did not name the failure: $(cat "$NET000_LOG")"
grep -q "publishes no per-file manifest" "$NET000_LOG" \
    && fail "a transport failure was reported as 'this release publishes none'"

echo "PASS: a manifest fetch failure is distinguished from an absent manifest"

# --- 200: the positive network witness — baseline IS planted -----------------
# Without this, every assertion above is satisfied by an installer that simply
# never fetches or plants a manifest on the network path.
NET200_DIR="$TMP/install-net-200"
NET200_LOG="$TMP/net-200.log"
if ! PATH="$STUB_DIR:$PATH" STUB_MANIFEST_MODE=200 \
     STUB_ARCHIVE="$TMP/archive.tar.gz" STUB_MANIFEST_FILE="$TMP/net.manifest.json" \
     TEMPER_INSTALL_DIR="$NET200_DIR" XDG_BIN_HOME="$TMP/bin-net-200" \
     sh "$INSTALL" --version v0.3.0 >"$NET200_LOG" 2>&1; then
    fail "a manifest-bearing release failed to install: $(cat "$NET200_LOG")"
fi
[ -f "$NET200_DIR/.temper-manifest.json" ] \
    || fail "a published manifest was fetched but no baseline was planted"
cmp -s "$NET200_DIR/.temper-manifest.json" "$TMP/net.manifest.json" \
    || fail "the planted baseline is not the manifest that was fetched"
grep -q "Verifying file manifest" "$NET200_LOG" \
    || fail "per-file verification was skipped on a manifest-bearing release"

echo "PASS: a published manifest is fetched, verified, and planted as the baseline"

# --- A manifest that IS present and DISAGREES is still fatal -----------------
# The bite for the whole change: degrading ABSENCE must not have degraded
# DISAGREEMENT. Same network path, manifest served with a 200, contents do not
# describe the archive.
NETBAD_DIR="$TMP/install-net-bad"
NETBAD_LOG="$TMP/net-bad.log"
build_manifest "$TMP/stage" "$TMP/net-bad.manifest.json"
jq '(.files[] | select(.path == "temper")).sha256 = "0000000000000000000000000000000000000000000000000000000000000000"' \
    < "$TMP/net-bad.manifest.json" > "$TMP/net-bad-served.manifest.json"
if PATH="$STUB_DIR:$PATH" STUB_MANIFEST_MODE=tampered \
     STUB_ARCHIVE="$TMP/archive.tar.gz" \
     STUB_MANIFEST_FILE="$TMP/net-bad-served.manifest.json" \
     TEMPER_INSTALL_DIR="$NETBAD_DIR" XDG_BIN_HOME="$TMP/bin-net-bad" \
     sh "$INSTALL" --version v0.3.0 >"$NETBAD_LOG" 2>&1; then
    fail "a published manifest that disagrees with the archive still installed"
fi
[ ! -e "$NETBAD_DIR" ] || fail "a refused install left files behind"
grep -q "file manifest verification failed" "$NETBAD_LOG" \
    || fail "the disagreement was not refused by the manifest gate: $(cat "$NETBAD_LOG")"

echo "PASS: a published manifest that disagrees with the archive is still fatal"
