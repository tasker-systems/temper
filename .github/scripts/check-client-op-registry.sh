#!/usr/bin/env bash
# .github/scripts/check-client-op-registry.sh
#
# Guard the "every temper-client request names a registry operation" invariant.
#
# temper-client's endpoint registry (crates/temper-client/src/ops.rs) is the one place a request
# path is spelled. Its parity test holds the registry to openapi.json, and dead-code analysis holds
# every registry constant to a method that uses it. Neither can see a method that skips the registry
# and spells its own path; that method's operation would be reachable without being counted. This
# script closes that hole: it fails on any "/api/ or "api/ string literal in the crate's src outside
# ops.rs. The slashless form counts because HttpClient::url trims a leading slash, so "api/x" reaches
# the same route as "/api/x".
#
# What it does not scan, and why:
#   - ops.rs itself: the registry is where paths belong.
#   - Comment lines (// and ///): doc comments name routes for the reader, and send nothing.
#   - #[cfg(test)] modules: tests assert the rendered paths as literals on purpose. That is an
#     independent check of the rendering, which is worth more than a test spelled through the
#     registry it is testing. The block is skipped by brace depth from its `mod … {` line.
#
# Only the api/ prefix is checked. Every temper API route sits under /api/; the client's other
# URLs (the OAuth token endpoint, the login callback) come from configuration, not literals.
#
# Usage:
#   .github/scripts/check-client-op-registry.sh [SRC_DIR]
#
# With no argument it scans crates/temper-client/src. A directory may be passed for testing
# against fixtures.
#
# Bash 3.2 compatible (macOS default): no assoc arrays, no mapfile.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
SRC_DIR="${1:-${REPO_ROOT}/crates/temper-client/src}"

if [ ! -d "$SRC_DIR" ]; then
    echo "ERROR: client source directory not found: $SRC_DIR" >&2
    exit 1
fi

OFFENDERS="$(find "$SRC_DIR" -name '*.rs' ! -name 'ops.rs' | sort | while IFS= read -r file; do
    awk -v file="${file#"${REPO_ROOT}/"}" '
        # A #[cfg(test)] attribute arms the skip; the next `mod … {` line starts it.
        /^[[:space:]]*#\[cfg\(test\)\][[:space:]]*$/ { armed = 1; next }
        armed && /^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?mod[[:space:]]+[A-Za-z0-9_]+[[:space:]]*;/ {
            armed = 0; next
        }
        armed && /^[[:space:]]*(pub(\([a-z]+\))?[[:space:]]+)?mod[[:space:]]+[A-Za-z0-9_]+[[:space:]]*\{/ {
            armed = 0; skipping = 1; depth = 0
        }
        skipping {
            line = $0
            opens = gsub(/\{/, "{", line)
            closes = gsub(/\}/, "}", line)
            depth += opens - closes
            if (depth <= 0) { skipping = 0 }
            next
        }
        { armed = 0 }
        /^[[:space:]]*\/\// { next }
        /"\/?api\// { printf "  %s:%d: %s\n", file, FNR, $0 }
    ' "$file"
done)"

if [ -n "$OFFENDERS" ]; then
    {
        echo "ERROR: temper-client spells a request path outside its endpoint registry:"
        printf '%s\n' "$OFFENDERS"
        echo ""
        echo "Every request goes through a constant in crates/temper-client/src/ops.rs, so the"
        echo "openapi parity test can count it. Build the path with ops::<OP>.path(&[..]) and the"
        echo "request with HttpClient::request(op, &path). If the operation has no constant yet,"
        echo "add one: under 'published' with its operationId when openapi.json documents it,"
        echo "or under 'unpublished' with the reason when it does not."
    } >&2
    exit 1
fi

echo "check-client-op-registry: every temper-client request path comes from the registry"
