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
  cargo install --path crates/temper-cli

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
  cargo install --path crates/temper-cli --features embed,extract,hnsw
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
    VERSION=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
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
curl -fsSL "$ARCHIVE_URL" -o "$TMPDIR/$ARCHIVE"
curl -fsSL "$SHA_URL" -o "$TMPDIR/$ARCHIVE.sha256"

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

INSTALL_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/temper"
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"

mkdir -p "$INSTALL_DIR" "$BIN_DIR"
echo "  Extracting to ${INSTALL_DIR}..."
tar -xzf "$TMPDIR/$ARCHIVE" -C "$INSTALL_DIR"

ln -sf "$INSTALL_DIR/temper" "$BIN_DIR/temper"

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
