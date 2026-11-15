#!/usr/bin/env bash
# tools/scripts/release/detect-changes.sh
#
# Detect what changed since the last release tag, by temper path class.
#
# EXTEND of tasker-core tools/scripts/release/detect-changes.sh, authorized by
# the shared-semver-policy spec §5 class list (spec of record:
# temper-artifacts/specs/2026-09-09-shared-semver-policy-design.md). tasker-core's
# FFI/SERVER split does not transfer — temper's surfaces are the classes below.
# Structure CONFORM: eval-safe KEY=VALUE on stdout, logs on stderr.
#
# Usage:
#   ./tools/scripts/release/detect-changes.sh [--from TAG]
#
# Output (eval-safe KEY=VALUE):
#   CHANGES_BASE_REF=<tag|commit>
#   CHANGES_BASE_IS_FALLBACK=true|false - true: the base is not a proven release (COMPAT-2:
#                                  no tag, a shallow clone, a newer tag on the remote, or a
#                                  hand-given --from). A fallback is never a release.
#   CHANGES_BASE_REASON=<why>    - empty when the base is a proven release
#   CORE_CHANGED=true|false      - any workspace crate changed (^crates/)
#   WIRE_CHANGED=true|false      - wire-emitting crates or the emitted contract
#                                  (^crates/temper-(api|mcp|client)/, ^openapi\.json$)
#   CLIENTS_CHANGED=true|false   - client skins (^clients/)
#   PACKAGES_CHANGED=true|false  - npm packages (^packages/)
#   SCHEMA_CHANGED=true|false    - migrations (^migrations/)
#   INFRA_CHANGED=true|false     - tooling, CI, docs, skills, this register
#
# ORCHESTRATOR RULING (recorded 2026-09-09, beat 2 of the semver mechanism
# build): SCHEMA_CHANGED and INFRA_CHANGED are detected and reported but NEVER
# force a version move. Schema additive is the deploy invariant; shape-breaking
# migrations are operator cutovers, and a shape-breaking migration pairs with
# code changes that force the move through their own class.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=lib/common.sh
source "${SCRIPT_DIR}/lib/common.sh"

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
FROM_REF=""

while [[ $# -gt 0 ]]; do
    case $1 in
        --from) FROM_REF="$2"; shift 2 ;;
        --from=*) FROM_REF="${1#*=}"; shift ;;
        *) die "Unknown argument: $1" ;;
    esac
done

# ---------------------------------------------------------------------------
# Determine base reference (temper tags are v<VERSION>; RELEASING.md)
# ---------------------------------------------------------------------------
# The derivation is last_release in lib/common.sh: the one the declared-class gate calls too
# (conventions COMPAT-2), so the two never disagree about which window a change is in.
if [[ -n "$FROM_REF" ]]; then
    BASE_REF="$FROM_REF"
    BASE_IS_FALLBACK=true
    BASE_REASON="a hand-given --from is not a derived release"
else
    RELEASE_OUTPUT="$(last_release HEAD)" || die "last_release failed — refusing to guess a window."
    eval "$RELEASE_OUTPUT"
    BASE_REF="$LAST_RELEASE_REF"
    BASE_IS_FALLBACK="$LAST_RELEASE_IS_FALLBACK"
    BASE_REASON="$LAST_RELEASE_REASON"
    if [[ "$BASE_IS_FALLBACK" == "true" ]]; then
        # Not provably the last release. When no tag was found at all the ref is the root
        # commit and every class reads true against it, so say so loudly instead of silently
        # diffing all of history.
        log_warn "The window base ${BASE_REF} is a fallback, not a proven release: ${BASE_REASON}."
        log_warn "Fetch the full history and tags (git fetch --unshallow --tags) before releasing."
    fi
fi

if ! git cat-file -e "${BASE_REF}^{commit}" 2>/dev/null; then
    die "Base ref does not resolve: ${BASE_REF}. A diff base that cannot be read must not render as an empty diff."
fi

log_info "Comparing HEAD to ${BASE_REF}" >&2

# ---------------------------------------------------------------------------
# Get changed files (no `|| true`: a failed diff is not an empty diff)
# ---------------------------------------------------------------------------
CHANGED_FILES=$(git diff "${BASE_REF}" HEAD --name-only)

if [[ -z "$CHANGED_FILES" ]]; then
    log_info "No files changed since ${BASE_REF}" >&2
fi

# ---------------------------------------------------------------------------
# Classify changes
# ---------------------------------------------------------------------------

# Helper: check if any changed file matches a pattern.
# Uses herestring to avoid SIGPIPE with large file lists.
changes_match() {
    local pattern="$1"
    grep -qE "$pattern" <<< "$CHANGED_FILES"
}

# Any workspace crate
CORE_CHANGED=false
if changes_match '^crates/'; then
    CORE_CHANGED=true
fi

# Wire-emitting crates or the emitted contract document
WIRE_CHANGED=false
if changes_match '^crates/temper-(api|mcp|client)/|^openapi\.json$'; then
    WIRE_CHANGED=true
fi

# Client skins
CLIENTS_CHANGED=false
if changes_match '^clients/'; then
    CLIENTS_CHANGED=true
fi

# npm packages
PACKAGES_CHANGED=false
if changes_match '^packages/'; then
    PACKAGES_CHANGED=true
fi

# Migrations — reported, never forces a move (see orchestrator ruling above)
SCHEMA_CHANGED=false
if changes_match '^migrations/'; then
    SCHEMA_CHANGED=true
fi

# Tooling, CI, docs, tests, agent skills, the register itself — never forces
# a move (see orchestrator ruling above)
INFRA_CHANGED=false
if changes_match '^(tools|\.github|docs|internal|tests|skills)/|^release-register/'; then
    INFRA_CHANGED=true
fi

# ---------------------------------------------------------------------------
# Output — eval-safe KEY=VALUE pairs
# ---------------------------------------------------------------------------
printf 'CHANGES_BASE_REF=%q\n' "${BASE_REF}"
echo "CHANGES_BASE_IS_FALLBACK=${BASE_IS_FALLBACK}"
printf 'CHANGES_BASE_REASON=%q\n' "${BASE_REASON}"
echo "CORE_CHANGED=${CORE_CHANGED}"
echo "WIRE_CHANGED=${WIRE_CHANGED}"
echo "CLIENTS_CHANGED=${CLIENTS_CHANGED}"
echo "PACKAGES_CHANGED=${PACKAGES_CHANGED}"
echo "SCHEMA_CHANGED=${SCHEMA_CHANGED}"
echo "INFRA_CHANGED=${INFRA_CHANGED}"
