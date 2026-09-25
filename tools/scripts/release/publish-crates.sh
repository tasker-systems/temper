#!/usr/bin/env bash
# tools/scripts/release/publish-crates.sh
#
# Publish the six temperkb-* client-closure crates to crates.io, in dependency
# order.
#
# Usage:
#   ./tools/scripts/release/publish-crates.sh VERSION [--dry-run]
#
# The closure, at the workspace lockstep version:
#   temperkb-principal -> temperkb-auth -> temperkb-telemetry -> temperkb-core
#   -> temperkb-workflow -> temperkb-client
#
# Ordered, and not as style: a published crate's manifest must resolve its
# dependencies from the REGISTRY (cargo strips the path and keeps the version
# when packing), so a crate whose workspace siblings are not yet on crates.io
# cannot be packed at all. The dependency order is the publish order, and the
# tail of the closure cannot be verified before the head is published — the
# same chicken-and-egg tasker-core's release validator documents. The per-crate
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
# is wanted. The trusted publisher is registered on crates.io against this
# repository and the workflow whose job performs the push (release.yml — the
# identity claim names the job's OWN workflow file, the same rule the
# RubyGems and PyPI registrations encode). crates.io accepts a pending
# publisher for a not-yet-created crate, so the six names are pre-registered
# there and the first CI publish claims them.
#
# Duplicate handling: crates.io's API answers unauthenticated (it requires a
# User-Agent identifying the caller), so the probe is a real pre-push check —
# a version already listed is a loud, idempotent skip. A publish failure after
# a clean probe is real and stops the release.

set -euo pipefail

VERSION="${1:-}"
DRY_RUN=false
[[ "${2:-}" == "--dry-run" ]] && DRY_RUN=true

if [[ -z "$VERSION" ]]; then
    echo "Usage: $0 VERSION [--dry-run]" >&2
    exit 1
fi

REPO_ROOT="$(git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

# Dependency order: a crate publishes only after everything it depends on.
CRATES=(temperkb-principal temperkb-auth temperkb-telemetry temperkb-core temperkb-workflow temperkb-client)

echo "==> Publishing the client closure ${VERSION} to crates.io (dry-run: ${DRY_RUN})"

# Version-agreement guard, twice over, BEFORE any registry or cargo call.
WS_VERSION="$(awk -F'"' '/^\[workspace\.package\]/{p=1; next} /^\[/{p=0} p && /^version = /{print $2; exit}' Cargo.toml)"
if [[ "$WS_VERSION" != "$VERSION" ]]; then
    echo "ERROR: [workspace.package] version is ${WS_VERSION}, but ${VERSION} was requested." >&2
    echo "       Bump [workspace.package].version and the six [workspace.dependencies] versions together." >&2
    exit 1
fi

for crate in "${CRATES[@]}"; do
    DEP_VERSION="$(sed -n -E "s/^${crate} = \\{.*version = \"([^\"]+)\".*/\\1/p" Cargo.toml)"
    if [[ "$DEP_VERSION" != "$VERSION" ]]; then
        echo "ERROR: ${crate}'s [workspace.dependencies] version is ${DEP_VERSION:-absent}, but ${VERSION} was requested." >&2
        echo "       Bump [workspace.package].version and the six [workspace.dependencies] versions together." >&2
        exit 1
    fi
done

# The probe: crates.io's API answers unauthenticated but requires a User-Agent
# identifying the caller (a bare curl is refused outright). No `grep -q` —
# the early-exit race under pipefail reads as a failed probe, and a duplicate
# that slips past lands as a loud registry refusal, which is the safe
# direction.
crate_published() {
    local crate="$1"
    curl -sf -A "temper-release (github.com/tasker-systems/temper)" \
        "https://crates.io/api/v1/crates/${crate}/versions" 2>/dev/null |
        grep "\"num\":\"${VERSION}\"" > /dev/null
}

PUBLISHED=0
for crate in "${CRATES[@]}"; do
    if crate_published "$crate"; then
        echo "==> ${crate} ${VERSION} is already published — skipping."
        PUBLISHED=$((PUBLISHED + 1))
        continue
    fi

    if [[ "$DRY_RUN" == "true" ]]; then
        echo "==> [dry-run] would publish ${crate} ${VERSION}"
        cargo publish --dry-run -p "$crate"
        continue
    fi

    echo "==> Publishing ${crate} ${VERSION}"
    cargo publish -p "$crate"
    # A fresh publish needs a moment before the versions API serves it; the
    # next crate's packaging does not depend on this (its publish resolves the
    # registry index, not the API), but an immediately-following re-run's probe
    # would otherwise miss it and re-publish.
    sleep 2
    PUBLISHED=$((PUBLISHED + 1))
done

if [[ "$PUBLISHED" -eq "${#CRATES[@]}" && "$DRY_RUN" != "true" ]]; then
    echo "==> Client closure ${VERSION} fully present on crates.io."
fi
