#!/usr/bin/env bash
# tools/scripts/release/publish-crates.sh
#
# Verify, then publish, the seven temperkb-* crates to crates.io (the client
# closure, then the MCP tool layer), in dependency order. The two stages run in
# separate jobs of release.yml.
#
# Usage:
#   ./tools/scripts/release/publish-crates.sh VERSION --out DIR
#   ./tools/scripts/release/publish-crates.sh VERSION --from DIR [--dry-run]
#
#   --out DIR   the BUILD stage: `cargo package` all seven, which verify-compiles
#               each one (build scripts and proc macros included), and copy the
#               `.crate` files into DIR. Holds no registry credential.
#   --from DIR  the PUBLISH stage: for each crate, in order, re-package it without
#               compiling, require its bytes to equal the verified `.crate` in DIR,
#               then `cargo publish --no-verify`. Compiles nothing.
#
# WHY TWO STAGES. The publish job holds the crates.io token, and `cargo publish`
# verify-compiles by default, running every dependency's build script beside it.
# So the compile runs in the build job, which cannot mint a token, and the publish
# job compiles nothing. Packaging is deterministic (same sources, same bytes), so
# the publish job checks it pushes exactly what the build job verified, rather
# than trusting that two packagings of one commit agree.
#
# The closure, at the workspace lockstep version:
#   temperkb-principal -> temperkb-auth -> temperkb-core -> temperkb-workflow
#   -> temperkb-telemetry -> temperkb-client -> temperkb-mcp
#
# Ordered, and not as style: a published crate's manifest must resolve its
# dependencies from the REGISTRY (cargo strips the path and keeps the version
# when packing), so a crate whose workspace siblings are not yet on crates.io
# cannot be packed on its own. The dependency order is the publish order. The
# build stage packages all seven in one `cargo package`, which resolves the
# unpublished siblings from a local overlay, so the whole closure is verified
# before anything is published. The per-crate
# duplicate probe makes re-runs idempotent: a re-cut release, or a re-run after
# crate 4 of 6 failed, skips what is already published and continues.
#
# Version agreement: the requested VERSION must equal [workspace.package]'s
# version (what every member inherits) AND each crate's [workspace.dependencies]
# version (dependency specs cannot inherit [workspace.package], so those six
# versions are spelled out and must be bumped together — this guard is what
# makes that discipline load-bearing).
#
# Auth: crates.io trusted publishing. In CI, rust-lang/crates-io-auth-action
# exchanges the job's OIDC identity token (id-token: write) for a short-lived
# upload token exposed as CARGO_REGISTRY_TOKEN — no API token secret exists or
# is wanted. crates.io matches the token's CALLING workflow, so inside the
# release-tag.yml → release.yml chain the name it checks is release-tag.yml;
# the tag-push and dispatch recovery doors present release.yml. Trusted
# publishers for both are registered on every temperkb-* name (RELEASING.md,
# "Publishing-side auth").
#
# Bootstrap, for the record: crates.io attaches a trusted publisher only to an
# EXISTING crate — there is no pending-publisher pre-registration — so the
# 0.5.3 initial versions were published once with an API token via this same
# script locally (auth falls back to the stored credential), and the token was
# revoked when the publishers were configured. temperkb-mcp was bootstrapped
# the same way at 0.6.0, its first version. Every later version of each crate
# rides the OIDC exchange only.
#
# Duplicate handling: crates.io's API answers unauthenticated (it requires a
# User-Agent identifying the caller), so the probe is a real pre-push check —
# a version already listed is a loud, idempotent skip. A publish failure after
# a clean probe is real and stops the release.

set -euo pipefail

VERSION="${1:-}"
DRY_RUN=false
OUT_DIR=""
FROM_DIR=""
shift || true
while [[ $# -gt 0 ]]; do
    case $1 in
        --dry-run) DRY_RUN=true; shift ;;
        --out) OUT_DIR="$2"; shift 2 ;;
        --from) FROM_DIR="$2"; shift 2 ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done

if [[ -z "$VERSION" ]] || [[ -n "$OUT_DIR" && -n "$FROM_DIR" ]] || [[ -z "$OUT_DIR" && -z "$FROM_DIR" ]]; then
    echo "Usage: $0 VERSION (--out DIR | --from DIR [--dry-run])" >&2
    exit 1
fi
if [[ -n "$OUT_DIR" && "$DRY_RUN" == "true" ]]; then
    echo "ERROR: --dry-run belongs to the publish stage (--from); the build stage publishes nothing." >&2
    exit 1
fi

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"
TARGET_DIR="${CARGO_TARGET_DIR:-${REPO_ROOT}/target}"

CRATES=(temperkb-principal temperkb-auth temperkb-core temperkb-workflow temperkb-telemetry temperkb-client temperkb-mcp)

WS_VERSION="$(awk -F'"' '/^\[workspace\.package\]/{p=1; next} /^\[/{p=0} p && /^version = /{print $2; exit}' Cargo.toml)"
if [[ "$WS_VERSION" != "$VERSION" ]]; then
    echo "ERROR: [workspace.package] version is ${WS_VERSION}, but ${VERSION} was requested." >&2
    echo "       Bump [workspace.package].version and every temperkb-* [workspace.dependencies] version together." >&2
    exit 1
fi

for crate in "${CRATES[@]}"; do
    DEP_VERSION="$(sed -n -E "s/^${crate} = \\{.*version = \"([^\"]+)\".*/\\1/p" Cargo.toml)"
    if [[ "$DEP_VERSION" != "$VERSION" ]]; then
        echo "ERROR: ${crate}'s [workspace.dependencies] version is ${DEP_VERSION:-absent}, but ${VERSION} was requested." >&2
        echo "       Bump [workspace.package].version and every temperkb-* [workspace.dependencies] version together." >&2
        exit 1
    fi
done

crate_published() {
    local crate="$1"
    curl -sf -A "temper-release (github.com/tasker-systems/temper)" \
        "https://crates.io/api/v1/crates/${crate}/versions" 2>/dev/null |
        grep "\"num\":\"${VERSION}\"" > /dev/null
}

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
    else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

if [[ -n "$OUT_DIR" ]]; then
    echo "==> verify-package the temperkb-* crates ${VERSION} into ${OUT_DIR}"
    PKG_ARGS=()
    for crate in "${CRATES[@]}"; do PKG_ARGS+=(-p "$crate"); done
    cargo package "${PKG_ARGS[@]}"
    mkdir -p "$OUT_DIR"
    for crate in "${CRATES[@]}"; do
        cp "${TARGET_DIR}/package/${crate}-${VERSION}.crate" "$OUT_DIR/"
    done
    exit 0
fi

echo "==> publish the temperkb-* crates ${VERSION} from ${FROM_DIR} (dry-run: ${DRY_RUN})"
PUBLISHED=0
for crate in "${CRATES[@]}"; do
    if crate_published "$crate"; then
        echo "==> ${crate} ${VERSION} is already published — skipping."
        PUBLISHED=$((PUBLISHED + 1))
        continue
    fi

    VERIFIED="${FROM_DIR}/${crate}-${VERSION}.crate"
    [[ -s "$VERIFIED" ]] || { echo "ERROR: ${crate}-${VERSION}.crate is not in ${FROM_DIR}." >&2; exit 1; }

    # Re-package without compiling and require the bytes the build job verified. A
    # dry run stops here: the next crate cannot be packaged on its own until this
    # one is on the registry.
    cargo package --no-verify -p "$crate"
    if [[ "$(sha256 "${TARGET_DIR}/package/${crate}-${VERSION}.crate")" != "$(sha256 "$VERIFIED")" ]]; then
        echo "ERROR: ${crate}-${VERSION}.crate packaged here differs from the one the build job verified." >&2
        exit 1
    fi

    if [[ "$DRY_RUN" == "true" ]]; then
        echo "==> [dry-run] ${crate} ${VERSION} matches the verified package; would publish it (and stops here)"
        exit 0
    fi

    echo "==> Publishing ${crate} ${VERSION}"
    cargo publish --no-verify -p "$crate"
    sleep 2
    PUBLISHED=$((PUBLISHED + 1))
done

if [[ "$PUBLISHED" -eq "${#CRATES[@]}" && "$DRY_RUN" != "true" ]]; then
    echo "==> All temperkb-* crates ${VERSION} present on crates.io."
fi
