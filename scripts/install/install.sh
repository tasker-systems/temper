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

while [ $# -gt 0 ]; do
    case $1 in
        --version) REQUESTED_VERSION="$2"; shift 2 ;;
        --version=*) REQUESTED_VERSION="${1#*=}"; shift ;;
        -h|--help)
            cat <<EOF
Usage: install.sh [--version VERSION]

  --version VERSION   Install a specific release tag (e.g. v0.1.0).
                      Defaults to the latest release.
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

TMPDIR=$(mktemp -d)
trap 'rm -rf "$TMPDIR"' EXIT

echo "  Downloading ${ARCHIVE}..."
# Bounded timeouts + a couple of retries so a stalled/black-hole network fails
# with an error instead of hanging forever (the archive is the large ORT-bearing
# tarball, so a mid-stream stall is the common bad-network case).
curl -fsSL --connect-timeout 10 --max-time 300 --retry 2 --retry-connrefused \
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

# Re-point the symlink, then confirm the LIVE binary runs before discarding the
# backup. Only now — after the on-disk install is proven good — is OLD dropped.
# If the live check fails, roll all the way back to the preserved backup.
ln -sf "$INSTALL_DIR/temper" "$BIN_DIR/temper"
if "$INSTALL_DIR/temper" --version >/dev/null 2>&1; then
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
