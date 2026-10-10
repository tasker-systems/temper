#!/usr/bin/env bash
# Fail if a workflow in the release-signing chain grows a trigger it does not already have.
#
# WHY: installed clients verify a release by its attestation's signer identity, and that
# identity is the REF the chain ran on. crates/temper-cli/src/attest.rs pins the certificate
# SAN to `build-cli-binaries.yml@refs/heads/main` and the predicate's entry workflow to
# `release-tag.yml`; npm and crates.io trusted publishing match the same entry workflow. The
# chain is branch-triggered by construction — a VERSION push to main runs release-tag.yml,
# which tags and calls release.yml → build-cli-binaries.yml — so every release signs as main.
#
# The trigger most likely to be added by mistake is `merge_group`: ci.yml carries it for the
# merge queue, and "make the release workflows queue-aware too" reads like tidying. It is not.
# A merge_group run's ref is `refs/heads/gh-readonly-queue/main/pr-N-<sha>`, so its signature
# names a ref every installed client rejects, and it runs BEFORE the merge — a release cut
# from a queue entry that is then ejected. `pull_request` is worse again. So this is an
# ALLOWLIST per file, not a denylist of merge_group: any new event fails, and adding one is a
# deliberate edit to the list below, made next to this paragraph.
#
# WHAT IT ASSERTS
#   (a) each chain workflow exists and its top-level `on:` block yields at least one event —
#       a parse that sees nothing must not report clean.
#   (b) the events in that block are exactly the allowlisted set for the file.
#   (c) release-tag.yml's push trigger names `main` and nothing else — the `@refs/heads/main`
#       half of the identity.
#
#   bash .github/scripts/check-release-chain-triggers.sh [--root DIR]
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
if [ "${1:-}" = "--root" ]; then ROOT="$2"; fi
WF="${ROOT}/.github/workflows"

# file:space-separated allowed events (sorted)
ALLOWED='
release-tag.yml:push
release.yml:push workflow_call workflow_dispatch
build-cli-binaries.yml:workflow_call workflow_dispatch
'

# events FILE — the keys of the top-level `on:` block, sorted, space-separated.
events() {
    awk '/^on:[[:space:]]*$/ { on = 1; next }
         on && /^[^[:space:]#]/ { exit }
         on && /^  [A-Za-z_]+:/ { sub(/^  /, ""); sub(/:.*/, ""); print }' "$1" | sort | tr '\n' ' ' | sed 's/ $//'
}

failed=0
while IFS= read -r line; do
    [ -z "$line" ] && continue
    file="${line%%:*}"; want="${line#*:}"
    path="${WF}/${file}"
    if [ ! -f "$path" ]; then
        echo "FAIL: ${file} is missing — the release chain this gate pins is not where it was." >&2
        failed=1; continue
    fi
    got="$(events "$path")"
    if [ -z "$got" ]; then
        echo "FAIL: ${file}: no events parsed from its \`on:\` block — refusing to report clean on a scan that saw nothing." >&2
        failed=1; continue
    fi
    if [ "$got" != "$want" ]; then
        echo "FAIL: ${file} triggers on [${got}], allowed [${want}]." >&2
        echo "      A release-chain run signs as the ref it ran on, and installed clients accept only" >&2
        echo "      refs/heads/main. See this script's header before widening the list." >&2
        failed=1
    else
        echo "PASS: ${file} triggers on [${got}]"
    fi
done <<< "$ALLOWED"

# (c) release-tag.yml pushes from main only.
branches="$(awk '/^on:[[:space:]]*$/ { on = 1; next }
                 on && /^[^[:space:]#]/ { exit }
                 on && /^  push:/ { push = 1; next }
                 on && /^  [A-Za-z_]+:/ { push = 0 }
                 push && /^    branches:/ { br = 1; sub(/^    branches:[[:space:]]*/, ""); if ($0 != "") print; next }
                 push && /^    [A-Za-z_]+:/ { br = 0 }
                 br && /^      - / { sub(/^      - /, ""); print }' "${WF}/release-tag.yml" 2>/dev/null \
            | tr -d "'\"[] " | tr ',' '\n' | sed '/^$/d' | sort -u | tr '\n' ' ' | sed 's/ $//')"
if [ "$branches" != "main" ]; then
    echo "FAIL: release-tag.yml's push trigger names branches [${branches}], expected [main] alone." >&2
    failed=1
else
    echo "PASS: release-tag.yml pushes from [main] only"
fi

exit "$failed"
