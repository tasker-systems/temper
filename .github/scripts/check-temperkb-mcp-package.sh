#!/usr/bin/env bash
# .github/scripts/check-temperkb-mcp-package.sh
#
# temperkb-mcp is publish-ready: prove it on every PR, so it cannot rot before its first publish.
#
#   1. `cargo publish --dry-run` over the client closure AND temperkb-mcp together. Packaging
#      several workspace crates in one invocation verifies each against its packaged siblings (a
#      local overlay), not against crates.io. A single-crate dry run would resolve temperkb-client
#      from the registry, and between releases main's client is ahead of the published one, so it
#      fails for a reason that says nothing about temperkb-mcp. The closure's own dry runs warn
#      that their versions already exist; that is expected and harmless here.
#   2. temperkb-mcp's tests, run from its extracted tarball: what a crates.io consumer builds,
#      with its siblings patched to their tarballs for the same reason as above (the patch
#      rewrites the tarball's lockfile, so this step cannot run `--locked`). A test that
#      reads a file the package does not ship fails here, not after the publish.
#
# temperkb-mcp is deliberately NOT in publish-crates.sh's CRATES yet: crates.io attaches a trusted
# publisher only to a crate that exists, so its name is claimed by a one-time local publish first
# (see that script's "Bootstrap, for the record").
#
# Usage: bash .github/scripts/check-temperkb-mcp-package.sh
set -euo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

# Dependency order, the same as publish-crates.sh, then the tool layer.
CLOSURE=(temperkb-principal temperkb-auth temperkb-core temperkb-workflow temperkb-telemetry temperkb-client)
PACKAGES=("${CLOSURE[@]}" temperkb-mcp)

VERSION="$(awk -F'"' '/^\[workspace\.package\]/{p=1; next} /^\[/{p=0} p && /^version = /{print $2; exit}' Cargo.toml)"

ARGS=()
for p in "${PACKAGES[@]}"; do ARGS+=(-p "$p"); done

echo "==> cargo publish --dry-run (closure + temperkb-mcp ${VERSION})"
cargo publish --dry-run --locked "${ARGS[@]}"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

for p in "${PACKAGES[@]}"; do
    tar -xzf "target/package/${p}-${VERSION}.crate" -C "$WORK"
done

# Config from a parent directory applies to the extracted crate without editing its manifest.
mkdir -p "$WORK/.cargo"
{
    echo '[patch.crates-io]'
    for p in "${CLOSURE[@]}"; do
        printf '%s = { path = "%s/%s-%s" }\n' "$p" "$WORK" "$p" "$VERSION"
    done
} > "$WORK/.cargo/config.toml"

echo "==> temperkb-mcp's tests from its packaged tarball"
(cd "$WORK/temperkb-mcp-${VERSION}" && CARGO_TARGET_DIR="$REPO_ROOT/target/package-test" cargo test)

echo "==> ...and with default-features = false"
(cd "$WORK/temperkb-mcp-${VERSION}" && CARGO_TARGET_DIR="$REPO_ROOT/target/package-test" cargo test --no-default-features)

echo "==> temperkb-mcp ${VERSION} packages, verifies and tests from its tarball."
