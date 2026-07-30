#!/usr/bin/env sh
# scripts/install/install.sh
#
# Install the latest `temper` CLI binary on macOS (Apple Silicon) or Linux
# (x86_64). Usage:
#
#   curl -fsSL https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.sh | sh
#
# Or to install a specific version:
#
#   curl -fsSL https://raw.githubusercontent.com/tasker-systems/temper/main/scripts/install/install.sh \
#     | sh -s -- --version v0.1.0
#
# Installs to:
#   ${XDG_DATA_HOME:-$HOME/.local/share}/temper/
# with a symlink at:
#   ${XDG_BIN_HOME:-$HOME/.local/bin}/temper

set -eu

REPO="tasker-systems/temper"
REQUESTED_VERSION=""
LOCAL_ARCHIVE=""
LOCAL_MANIFEST=""
# Written into INSTALL_DIR only on a successful, verified install (never on a
# rolled-back one) — matches `MANIFEST_FILENAME` in `manifest.rs` (Task 2), so
# `temper version --verify` reads the same file this script writes.
MANIFEST_FILENAME=".temper-manifest.json"

while [ $# -gt 0 ]; do
    case $1 in
        --version) REQUESTED_VERSION="$2"; shift 2 ;;
        --version=*) REQUESTED_VERSION="${1#*=}"; shift ;;
        --archive) LOCAL_ARCHIVE="$2"; shift 2 ;;
        --archive=*) LOCAL_ARCHIVE="${1#*=}"; shift ;;
        --manifest) LOCAL_MANIFEST="$2"; shift 2 ;;
        --manifest=*) LOCAL_MANIFEST="${1#*=}"; shift ;;
        -h|--help)
            cat <<EOF
Usage: install.sh [--version VERSION] [--archive PATH] [--manifest PATH]

  --version VERSION   Install a specific release tag (e.g. v0.1.0).
                      Defaults to the latest release.
  --archive PATH      Install from an already-downloaded, already-verified
                      archive instead of downloading one. Used by
                      \`temper update\`, which verifies provenance itself before
                      handing the archive over.
  --manifest PATH     Use this per-file manifest instead of fetching the
                      published one. Required alongside --archive, since
                      there is then no network fetch to source it from.
EOF
            exit 0
            ;;
        *) echo "error: unknown argument: $1" >&2; exit 1 ;;
    esac
done

OS=$(uname -s)
ARCH=$(uname -m)

case "$OS" in
    Darwin)
        if [ "$ARCH" != "arm64" ]; then
            cat >&2 <<EOF
error: no prebuilt binary for macOS ${ARCH}.

Temper v1 only ships macOS arm64 (Apple Silicon) binaries. On Intel Macs,
build from source:

  git clone https://github.com/${REPO}
  cd temper
  cargo install --path crates/temper-cli --locked

If you are on Apple Silicon and seeing this message, you may be running
under Rosetta. Run the installer in a native arm64 terminal.
EOF
            exit 1
        fi
        TARGET="aarch64-apple-darwin"
        ;;
    Linux)
        if [ "$ARCH" != "x86_64" ]; then
            cat >&2 <<EOF
error: no prebuilt binary for Linux ${ARCH}.

Temper v1 only ships Linux x86_64 binaries. Build from source:

  git clone https://github.com/${REPO}
  cd temper
  cargo install --path crates/temper-cli --locked --features embed,extract
EOF
            exit 1
        fi
        TARGET="x86_64-unknown-linux-gnu"
        ;;
    *)
        echo "error: unsupported OS: $OS" >&2
        exit 1
        ;;
esac

if [ -z "$REQUESTED_VERSION" ]; then
    VERSION=$(curl -fsSL --connect-timeout 10 --max-time 30 --retry 2 --retry-connrefused \
        "https://api.github.com/repos/${REPO}/releases/latest" \
        | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' \
        | head -n1)
    if [ -z "$VERSION" ]; then
        echo "error: could not determine latest release from GitHub API" >&2
        exit 1
    fi
else
    VERSION="$REQUESTED_VERSION"
fi

echo "Installing temper ${VERSION} (${TARGET})..."

ARCHIVE="temper-${VERSION}-${TARGET}.tar.gz"
URL_BASE="https://github.com/${REPO}/releases/download/${VERSION}"
ARCHIVE_URL="${URL_BASE}/${ARCHIVE}"
SHA_URL="${URL_BASE}/${ARCHIVE}.sha256"
MANIFEST="temper-${VERSION}-${TARGET}.manifest.json"
MANIFEST_URL="${URL_BASE}/${MANIFEST}"

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

if [ -n "$LOCAL_ARCHIVE" ]; then
    # Handed a pre-verified archive by `temper update`, which has already
    # checked provenance (attestation) and integrity (manifest). Re-downloading
    # would reopen exactly the TOCTOU gap that flag exists to close: verify one
    # object, install another.
    [ -f "$LOCAL_ARCHIVE" ] || { echo "error: --archive file not found: $LOCAL_ARCHIVE" >&2; exit 1; }
    cp "$LOCAL_ARCHIVE" "$TMPDIR/$ARCHIVE"
    echo "  Using pre-verified archive: ${LOCAL_ARCHIVE}"
else
    echo "  Downloading ${ARCHIVE}..."
    # A STALL detector, not a duration cap.
    #
    # `--max-time 300` was a hard wall-clock ceiling on the whole transfer. The archive now carries the
    # embedding model (~81 MB gzipped, ~99 MB total), which turns that ceiling into a minimum required
    # throughput of ~330 KB/s — below which curl exits 28, `set -eu` aborts, and `--retry` restarts from
    # byte 0 under a fresh 300 s budget, so it fails on EVERY attempt, permanently. A slow-but-working
    # connection would simply never be able to install.
    #
    # `--speed-limit`/`--speed-time` keep the original intent (a black-holed or stalled transfer fails
    # instead of hanging forever) without punishing a slow one: abort only if throughput stays under
    # 1 KB/s for 60 s straight. `-C -` resumes a partial file across retries rather than restarting.
    curl -fsSL --connect-timeout 10 --speed-limit 1024 --speed-time 60 \
        --retry 2 --retry-connrefused -C - \
        "$ARCHIVE_URL" -o "$TMPDIR/$ARCHIVE"
    curl -fsSL --connect-timeout 10 --max-time 60 --retry 2 --retry-connrefused \
        "$SHA_URL" -o "$TMPDIR/$ARCHIVE.sha256"

    echo "  Verifying checksum..."
    cd "$TMPDIR"
    if [ "$OS" = "Darwin" ]; then
        EXPECTED=$(awk '{print $1}' "$ARCHIVE.sha256")
        ACTUAL=$(shasum -a 256 "$ARCHIVE" | awk '{print $1}')
        [ "$EXPECTED" = "$ACTUAL" ] || { echo "error: checksum mismatch"; exit 1; }
    else
        sha256sum -c "$ARCHIVE.sha256" >/dev/null
    fi
    cd - >/dev/null
fi

# --- Per-file manifest acquisition -------------------------------------------
# An explicit --manifest always wins (this is what lets the test harness, and
# a future `temper update --archive` caller who already fetched+verified one,
# avoid a network round-trip). Absent that, a fresh download fetches the
# published manifest alongside the archive; --archive with no --manifest has
# no other source to draw one from, so it is a hard error rather than a
# silent skip of per-file verification.
#
# ON THE NETWORK PATH, AN ABSENT MANIFEST IS `unverifiable`, NOT `mismatch`.
# This script is fetched from `main` (unversioned — see the curl line at the top
# of this file), but it installs *versioned* release artifacts. Those two move
# independently, so this script is routinely newer than the release it is asked
# to install: every release up to and including v0.2.6 published only
# `.tar.gz`/`.zip` + `.sha256`, and `--version v0.2.6` must keep working
# forever. A hard failure here would mean "the manifest asset is missing"
# renders as "refusing to install", which is the one thing the verdict
# trichotomy exists to prevent — the same reason `attest.rs` reports
# `Unverifiable` for a `cargo install` build instead of `Mismatch`.
#
# It is also a hole that buys nothing to close. This script never verifies the
# release ATTESTATION — that lives in Rust (`temper update`, which supplies
# `--manifest` explicitly and stays hard-fail below). The manifest fetched here
# is uploaded by the same credential as the archive AND its `.sha256`, so an
# actor who can delete the manifest asset to force this arm can equally upload
# one that matches a tampered archive and collect a pass. Failing closed on
# absence does not raise that bar; it only breaks every pre-manifest release.
#
# What per-file verification still buys on this path is the DURABLE BASELINE
# planted into INSTALL_DIR below, which later offline `temper version --verify`
# runs read. The archive `.sha256` — verified unconditionally above, and never
# skipped — is what gates these bytes. So a missing manifest degrades exactly
# one thing: this install plants no baseline, and says so.
#
# A manifest that IS present and does NOT verify remains a hard failure. That
# is a disagreement, not an absence.
MANIFEST_AVAILABLE=1
if [ -n "$LOCAL_MANIFEST" ]; then
    [ -f "$LOCAL_MANIFEST" ] || { echo "error: --manifest file not found: $LOCAL_MANIFEST" >&2; exit 1; }
    cp "$LOCAL_MANIFEST" "$TMPDIR/$MANIFEST"
    echo "  Using supplied manifest: ${LOCAL_MANIFEST}"
elif [ -n "$LOCAL_ARCHIVE" ]; then
    echo "error: --archive requires --manifest (no network fetch to source per-file verification from)" >&2
    exit 1
else
    echo "  Downloading manifest..."
    # `-f` is deliberately dropped here (unlike every other curl in this file):
    # with it, a 404 and a black-holed connection both surface as exit 22/28 and
    # the two become indistinguishable. The HTTP status is the whole point —
    # "this release predates manifests" and "we could not reach GitHub" warrant
    # different messages even though they warrant the same verdict. Dropping -f
    # means a 404 BODY lands in the output file, so every non-200 must delete it
    # (a stray error page would otherwise be parsed as a manifest below).
    MANIFEST_HTTP=$(curl -sSL --connect-timeout 10 --max-time 60 \
        --retry 2 --retry-connrefused -w '%{http_code}' \
        -o "$TMPDIR/$MANIFEST" "$MANIFEST_URL" 2>/dev/null) || MANIFEST_HTTP="000"
    if [ "$MANIFEST_HTTP" != "200" ]; then
        rm -f "$TMPDIR/$MANIFEST"
        MANIFEST_AVAILABLE=0
        echo "" >&2
        if [ "$MANIFEST_HTTP" = "404" ]; then
            echo "warning: ${VERSION} publishes no per-file manifest." >&2
            echo "         Releases before per-file manifests shipped do not carry one, so there is" >&2
            echo "         nothing to verify this install against file-by-file." >&2
        else
            echo "warning: could not fetch the per-file manifest for ${VERSION} (HTTP ${MANIFEST_HTTP})." >&2
            echo "         This says nothing about whether the archive is genuine — only that this" >&2
            echo "         check could not run." >&2
        fi
        echo "" >&2
        echo "         The archive checksum WAS verified against the published .sha256, so the" >&2
        echo "         bytes being installed are the bytes GitHub serves for ${VERSION}." >&2
        echo "         What is skipped is per-file verification and the offline baseline:" >&2
        echo "         \`temper version --verify\` will report \`unverifiable\` on this install." >&2
        echo "         To reach a verified state, run: temper version --verify --online" >&2
        echo "" >&2
    fi
fi

# INSTALL_DIR is overridable via TEMPER_INSTALL_DIR so `temper update` can aim
# the swap at the directory the *running* binary actually lives in (which can
# differ from the XDG default), keeping the caller's provenance detection
# authoritative. A fresh curl|sh install leaves it unset and uses the default.
INSTALL_DIR="${TEMPER_INSTALL_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/temper}"
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"

# Extract into a staging dir beside the final INSTALL_DIR (same filesystem, so
# swapping it in is an atomic rename). Nothing touches the live install until
# the freshly-extracted binary has been proven to RUN on this host.
PARENT_DIR=$(dirname "$INSTALL_DIR")
mkdir -p "$PARENT_DIR" "$BIN_DIR"
STAGING="${INSTALL_DIR}.new-$$"
OLD="${INSTALL_DIR}.old-$$"
# Clean the tmp download + staging on any exit. OLD (the rollback backup) is
# deliberately NOT in this trap: it is the only recovery copy, and is dropped
# explicitly only once the new install is verified live. So even a SIGKILL
# leaves a recoverable backup rather than nothing.
trap 'rm -rf "$TMPDIR" "$STAGING"' EXIT

rm -rf "$STAGING"
mkdir -p "$STAGING"
echo "  Extracting..."
tar -xzf "$TMPDIR/$ARCHIVE" -C "$STAGING"

# --- Per-file manifest verification ------------------------------------------
# The archive-level .sha256 (or the caller's own check, on --archive) proves
# the ARCHIVE arrived intact; this proves each FILE inside it is the one
# published, which is what makes "is my temper the one you shipped?" answerable
# at all. Deliberately dependency-free (no jq): install.sh is a `curl | sh`
# installer whose only tools are curl, tar, and shasum/sha256sum, and a
# manifest gate that requires jq would be a regression for anyone without it.
#
# emit-manifest.sh (the producer) writes default jq-pretty-printed JSON, which
# puts one key per line and preserves the `{path, sha256, size}` construction
# order within each object — so a `path` line is always immediately followed
# by its `sha256` line. That is what makes this awk pass tractable: no bracket
# counting or JSON parsing, just "remember the last path, emit on sha256".
#
# That layout is no longer an assumption this parser RESTS on; it is one it
# VERIFIES. When it does not hold, the awk pass below does not fail — it emits
# ZERO pairs (on compact JSON `path` and `sha256` share a line, so the `next`
# in the path rule means the sha256 rule never fires), and every per-file check
# is then silently skipped rather than failed. So
# `verify_manifest_against_dir` cross-checks the pairs actually parsed against
# an independent count of the entries the manifest declares, and refuses to
# install on any disagreement.
sha_of_file() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    else
        shasum -a 256 "$1" | awk '{print $1}'
    fi
}

manifest_pairs() {
    awk '
        /"path":/ {
            p = $0
            sub(/^[^"]*"path": *"/, "", p)
            sub(/".*$/, "", p)
            path = p
            next
        }
        /"sha256":/ {
            s = $0
            sub(/^[^"]*"sha256": *"/, "", s)
            sub(/".*$/, "", s)
            printf "%s\t%s\n", path, s
        }
    ' "$1"
}

# How many entries the manifest *claims*, counted independently of the awk
# pair parser above. `"path"` appears exactly once per entry and nowhere else
# in the manifest shape ({version, target, files:[{path, sha256, size}]}), so
# occurrence count is entry count. Counting OCCURRENCES rather than LINES is
# what makes this catch compact JSON, where every entry shares one line.
manifest_declared_count() {
    grep -o '"path":' "$1" | wc -l | tr -d ' '
}

# Checks every file the manifest lists against $1 (a directory). Runs the
# whole list before returning, so a single invocation reports every mismatch
# rather than just the first. Returns non-zero if anything failed.
verify_manifest_against_dir() {
    CHECK_DIR="$1"
    CHECK_FAILED=0
    PAIR_COUNT=0
    while IFS="$(printf '\t')" read -r REL EXPECTED_SHA; do
        [ -n "$REL" ] || continue
        PAIR_COUNT=$((PAIR_COUNT + 1))
        # A manifest entry may only name a file strictly INSIDE $CHECK_DIR.
        # Without this, "../decoy" reads a file outside the staging/install
        # root: point one entry at a file an attacker placed there and state
        # its REAL sha256, and that entry verifies while the extracted `temper`
        # is never hashed at all — a verdict forged by editing a `path` string,
        # never a hash. Reproduced end-to-end in test-install.sh, where the
        # unfixed installer printed "✓ Installed" and exited 0.
        #
        # An absolute $REL does not escape here the way it does in Rust's
        # `Path::join` — "$CHECK_DIR/$REL" concatenates to "$CHECK_DIR//etc/…",
        # which stays inside — so that arm is defense in depth: refuse it by
        # path, loudly, rather than by the accident of a missing file. The `..`
        # arm is the one closing a live hole.
        #
        # These run AFTER the PAIR_COUNT increment on purpose: a rejected entry
        # still counts as parsed, so the parsed-vs-declared cross-check below
        # stays meaningful and the failure is reported by CHECK_FAILED.
        #
        # `case` on the path wrapped in SLASHES tests for a `..` COMPONENT, so
        # an ordinary filename that merely contains dots (foo..bar) is still
        # accepted. Do not simplify it to *..*, which would refuse a
        # legitimate shipped file. (An empty $REL never reaches here — the
        # line above skips it, and the count cross-check catches the gap.)
        case "$REL" in
            /*)
                echo "error: manifest entry has an absolute path: $REL" >&2
                CHECK_FAILED=1
                continue
                ;;
        esac
        case "/$REL/" in
            *"/../"*)
                echo "error: manifest entry escapes the install root: $REL" >&2
                CHECK_FAILED=1
                continue
                ;;
        esac
        if [ ! -f "$CHECK_DIR/$REL" ]; then
            echo "error: manifest lists $REL but it is missing from $CHECK_DIR" >&2
            CHECK_FAILED=1
            continue
        fi
        ACTUAL_SHA=$(sha_of_file "$CHECK_DIR/$REL")
        if [ "$ACTUAL_SHA" != "$EXPECTED_SHA" ]; then
            echo "error: $REL has sha256 $ACTUAL_SHA, expected $EXPECTED_SHA" >&2
            CHECK_FAILED=1
        fi
    done <<EOF
$(manifest_pairs "$TMPDIR/$MANIFEST")
EOF
    # The loop above is fed by a HERE-DOCUMENT, not a pipe, so its body runs in
    # this shell and the counters survive. Do not restructure it into
    # `manifest_pairs … | while read`: that puts the loop in a subshell, both
    # counters reset to zero on exit, and the fail-open below returns.
    #
    # Nothing parsed means nothing was checked — and an unconditional success
    # return here is a fail-open, not a pass. Two ways in, one floor: a
    # manifest that genuinely lists no files, and a manifest whose entries we
    # failed to parse (compact JSON defeats the awk pass above). The count
    # cross-check separates them so the error message is true in both cases.
    DECLARED=$(manifest_declared_count "$TMPDIR/$MANIFEST")
    if [ "$PAIR_COUNT" -eq 0 ]; then
        echo "error: manifest lists no verifiable files (declared entries: $DECLARED) — refusing to install" >&2
        return 1
    fi
    if [ "$PAIR_COUNT" != "$DECLARED" ]; then
        echo "error: parsed $PAIR_COUNT of $DECLARED manifest entries — refusing to install on a partial parse" >&2
        return 1
    fi
    [ "$CHECK_FAILED" -eq 0 ]
}

if [ "$MANIFEST_AVAILABLE" -eq 1 ]; then
    echo "  Verifying file manifest..."
    if ! verify_manifest_against_dir "$STAGING"; then
        echo "error: file manifest verification failed; your existing install was left untouched." >&2
        exit 1
    fi
else
    echo "  Skipping file manifest verification (no manifest published for ${VERSION})."
fi

# --- Verification gate -------------------------------------------------------
# Prove the new binary runs on THIS host BEFORE disturbing the live install. A
# correctly-checksummed archive can still be unrunnable here (wrong arch,
# incompatible glibc, a truncated/ABI-broken ORT lib the loader needs).
# `temper --version` is served by the arg parser and exercises exec + dynamic-
# link resolution without needing config or network. If it can't run, abort now
# — the existing install is never touched.
chmod +x "$STAGING/temper" 2>/dev/null || true
echo "  Verifying the new binary runs..."
if ! "$STAGING/temper" --version >/dev/null 2>&1; then
    echo "error: the downloaded temper binary failed to run on this host" >&2
    echo "       (architecture/libc mismatch, or a corrupt download)." >&2
    echo "       Your existing install was left untouched." >&2
    exit 1
fi

# --- Atomic swap -------------------------------------------------------------
# Move the live install aside (preserved as OLD), then swing the verified
# staging dir in. The running binary's inode stays valid across the rename
# (unix), so an in-place `temper update` keeps working until it exits.
echo "  Installing to ${INSTALL_DIR}..."
if [ -d "$INSTALL_DIR" ]; then
    rm -rf "$OLD"
    mv "$INSTALL_DIR" "$OLD"
fi
if mv "$STAGING" "$INSTALL_DIR"; then
    :
else
    # Swap-in failed. Restore the previous install if we moved it aside; if even
    # that fails, do NOT lose it — report exactly where the backup survives.
    if [ -d "$OLD" ]; then
        if mv "$OLD" "$INSTALL_DIR" 2>/dev/null; then
            echo "error: failed to install to ${INSTALL_DIR}; restored previous install" >&2
        else
            echo "error: failed to install to ${INSTALL_DIR} AND could not restore it." >&2
            echo "       Your previous install is preserved at: ${OLD}" >&2
        fi
    else
        echo "error: failed to install to ${INSTALL_DIR} (fresh install; nothing to restore)" >&2
    fi
    exit 1
fi

# Re-point the symlink, then confirm the LIVE binary runs AND still matches the
# manifest before discarding the backup. Only now — after the on-disk install
# is proven good — is OLD dropped, and only now is the manifest written into
# INSTALL_DIR: a rolled-back install must never leave a manifest describing an
# install that is no longer there. If either check fails, roll all the way
# back to the preserved backup through the same machinery as the run-gate
# failure above — no second rollback path.
#
# The run-gate is unconditional; the manifest gate runs only when a manifest was
# acquired. Written as a function rather than an `a && b` condition because the
# manifest half is now conditional, and `[ "$MANIFEST_AVAILABLE" -eq 1 ] && cp …`
# as a bare statement would ABORT the script under `set -eu` whenever the test is
# false: an AND-list's status is its short-circuiting member's, and a top-level
# non-zero status is exactly what `set -e` acts on.
post_install_ok() {
    "$INSTALL_DIR/temper" --version >/dev/null 2>&1 || return 1
    if [ "$MANIFEST_AVAILABLE" -eq 1 ]; then
        verify_manifest_against_dir "$INSTALL_DIR" || return 1
    fi
    return 0
}

ln -sf "$INSTALL_DIR/temper" "$BIN_DIR/temper"
if post_install_ok; then
    if [ "$MANIFEST_AVAILABLE" -eq 1 ]; then
        cp "$TMPDIR/$MANIFEST" "$INSTALL_DIR/$MANIFEST_FILENAME"
    fi
    rm -rf "$OLD"
else
    echo "error: the installed binary failed its post-install check; rolling back..." >&2
    rm -rf "$INSTALL_DIR"
    if [ -d "$OLD" ]; then
        mv "$OLD" "$INSTALL_DIR"
        ln -sf "$INSTALL_DIR/temper" "$BIN_DIR/temper"
        echo "error: rolled back to the previous install." >&2
    fi
    exit 1
fi

case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *)
        cat <<EOF

⚠️  $BIN_DIR is not on your PATH. Add it by running ONE of the following,
   depending on your shell:

   # bash
   echo 'export PATH="\$PATH:$BIN_DIR"' >> ~/.bashrc

   # zsh
   echo 'export PATH="\$PATH:$BIN_DIR"' >> ~/.zshrc

   # fish
   fish_add_path $BIN_DIR
EOF
        ;;
esac

cat <<EOF

✓ Installed temper ${VERSION} to ${INSTALL_DIR}
  Run:  temper --help
EOF
