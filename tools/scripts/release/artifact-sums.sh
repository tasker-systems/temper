#!/usr/bin/env bash
# tools/scripts/release/artifact-sums.sh
#
# Bind a registry lane's publish job to the exact bytes its own build job produced.
#
# Usage:
#   ./tools/scripts/release/artifact-sums.sh emit DIR     # build job: print `<sha256>  <file>` lines
#   ./tools/scripts/release/artifact-sums.sh verify DIR   # publish job: check DIR against $SUMS
#
# WHY. Every job in a workflow run shares the run's artifact namespace, so a compromised step
# in ONE lane's build job could replace ANOTHER lane's artifact under the same name before that
# lane's publish job downloads it — and the publish stages check only names and versions, which a
# substitute can match. A job's outputs, by contrast, are written by that job alone. So each
# build job publishes its files' hashes as a job output (`emit`), and each publish job requires
# the downloaded directory to hold exactly those files with exactly those hashes (`verify`), with
# the expected list taken from `needs.<its build job>.outputs`, never from the artifact.
#
# An empty DIR is a lane whose versions were already published (the scripts' registry probe made
# the build job upload nothing): `emit` prints nothing and `verify` accepts an empty or missing
# DIR only when the expected list is empty too.

set -euo pipefail

MODE="${1:-}"
DIR="${2:-}"
if [[ -z "$MODE" || -z "$DIR" ]] || [[ "$MODE" != "emit" && "$MODE" != "verify" ]]; then
    echo "Usage: $0 (emit|verify) DIR" >&2
    exit 1
fi

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
    else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

# `<sha256>  <file>` for every regular, non-hidden file directly in DIR, sorted by name. Nothing
# for a missing or empty DIR. Hidden files are skipped on both sides because upload-artifact
# leaves them out by default (`uv build` writes a `.gitignore` into its output directory), so
# they never reach the publish job and are never pushed.
sums_of() {
    [[ -d "$1" ]] || return 0
    local f
    while IFS= read -r f; do
        printf '%s  %s\n' "$(sha256 "$1/$f")" "$f"
    done < <(cd "$1" && find . -mindepth 1 -maxdepth 1 -type f ! -name '.*' | sed 's|^\./||' | LC_ALL=C sort)
}

if [[ "$MODE" == "emit" ]]; then
    sums_of "$DIR"
    exit 0
fi

EXPECTED="$(printf '%s' "${SUMS:-}" | sed '/^[[:space:]]*$/d' | LC_ALL=C sort -k2)"
ACTUAL="$(sums_of "$DIR")"
if [[ "$ACTUAL" != "$EXPECTED" ]]; then
    echo "ERROR: ${DIR} does not hold exactly the files, with exactly the hashes, its build job produced." >&2
    echo "--- expected (the build job's output)" >&2
    printf '%s\n' "${EXPECTED:-<nothing>}" >&2
    echo "--- found" >&2
    printf '%s\n' "${ACTUAL:-<nothing>}" >&2
    exit 1
fi
echo "==> ${DIR}: $(printf '%s' "$EXPECTED" | grep -c . || true) file(s) match the build job's hashes."
