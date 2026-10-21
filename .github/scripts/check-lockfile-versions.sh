#!/usr/bin/env bash
# .github/scripts/check-lockfile-versions.sh
#
# Every npm package's version agrees with the version its lockfile records for it: bun.lock's
# workspace entry for packages/temper-ui and packages/temper-cloud, the package-lock.json
# top-level and root-package entries for clients/temper-ts, clients/temper-telemetry-ts and
# packages/temper-design.
#
# `bun install --frozen-lockfile` and `npm ci` both accept a stale record, so a release bump that
# moves package.json and leaves the lockfile behind passes everything else. The release's version
# writer (tools/scripts/release/update-versions.sh) moves both; this check holds the tree to it.
# It reads the lockfiles through `lockfile_recorded_versions` in the release tooling's common.sh,
# where the writer's rewrite and the reader share one statement of each lockfile's version sites.
#
#   bash .github/scripts/check-lockfile-versions.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
# shellcheck source=../../tools/scripts/release/lib/common.sh
source "${SCRIPT_DIR}/../../tools/scripts/release/lib/common.sh"

PACKAGES=(packages/temper-ui packages/temper-cloud clients/temper-ts clients/temper-telemetry-ts packages/temper-design)
FAIL=0

for dir in "${PACKAGES[@]}"; do
    want="$(awk -F'"' '/^  "version": "/ { print $4; exit }' "${REPO_ROOT}/${dir}/package.json")"
    recorded="$(lockfile_recorded_versions "$dir")"
    if [[ -z "$recorded" ]]; then
        log_error "${dir}: no lockfile records its version"
        FAIL=1
    elif stale="$(grep -vxF "$want" <<< "$recorded")"; then
        log_error "${dir}: package.json is ${want}, the lockfile records $(tr '\n' ' ' <<< "$stale")— run tools/scripts/release/update-versions.sh, or update the lockfile's record by hand"
        FAIL=1
    else
        log_info "${dir}: ${want}"
    fi
done

exit "$FAIL"
