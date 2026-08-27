#!/usr/bin/env bash
#
# Fail if the committed temper-py generated package drifts from openapi.json.
#
# Regenerates the package (via generate-temper-py.sh) and diffs the result against
# what is committed — the local mirror of the `test-python` CI job's drift step, and
# the sibling of check-temper-rb-drift.sh and check-temper-ts-drift.sh.
#
# WHEN THIS SKIPS, AND WHY IT IS NOT THE GEM'S RULE. The gem's gate skips whenever
# Docker is absent, even on a host that could run the pinned generator from the jar.
# That is a stricter skip than its own generator needs, and the reason it is tolerable
# there is that `test-ruby` pulls the image anyway. This gate runs whichever path the
# host HAS — Docker or a JVM — and skips only when BOTH are missing, which is the
# weakest form that still catches the artifact's failure mode. GitHub runners ship a
# JDK, so in CI it never skips; the `test-python` job is the backstop either way.
#
# Usage: bash .github/scripts/check-temper-py-drift.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GENERATED="clients/temper-py/temper/generated"
MANIFEST="clients/temper-py/.openapi-generator"

if ! (command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1) \
   && ! command -v java >/dev/null 2>&1; then
  echo "SKIP: temper-py drift check — neither Docker nor a Java runtime is available." >&2
  echo "      The package is generated from openapi.json; run 'cargo make openapi-py' on" >&2
  echo "      a host with either one. The test-python CI job is the backstop." >&2
  exit 0
fi

bash "$REPO_ROOT/.github/scripts/generate-temper-py.sh"

# Assert both diff targets are TRACKED before diffing them. `git diff --exit-code -- <path>`
# exits 0 when the path matches nothing — untracked, ignored, moved, renamed — so the diff
# alone cannot distinguish "identical to what is committed" from "not committed at all". Like
# the gem's gate and unlike temper-ts's, this diffs whole DIRECTORIES, so a generator config
# change that relocated the output would silently empty the gate rather than fail it — a
# permanently-green no-op. (`ls-files --error-unmatch` on a directory succeeds only when at
# least one tracked file lives under it.) A gate that cannot fail is not a gate; make that
# state loud instead of green.
#
# The manifest is a diff target and not merely a witness: `.openapi-generator/FILES` is the
# generator's own list of what it wrote, so a file that STOPS being generated shows up there
# as a deletion. `git diff` over $GENERATED alone would never see it — the orphan just sits
# on disk, tracked and stale, matching nothing the generator emits. (The gem has exactly that
# hole today: `reassign_api.rb` survives from a tag the contract no longer carries.)
if ! git -C "$REPO_ROOT" ls-files --error-unmatch -- "$GENERATED" "$MANIFEST" >/dev/null 2>&1; then
  echo "ERROR: $GENERATED or $MANIFEST is not tracked by git, so there is nothing to" >&2
  echo "       diff against. Either it is gitignored or the paths here have drifted from" >&2
  echo "       what generate-temper-py.sh writes. Until that is fixed this gate checks" >&2
  echo "       nothing." >&2
  exit 1
fi

if ! git -C "$REPO_ROOT" diff --exit-code -- "$GENERATED" "$MANIFEST"; then
  echo >&2
  echo "ERROR: temper-py's generated package is out of date with openapi.json." >&2
  echo "       Run: cargo make openapi   (regenerates the spec and all three SDKs)" >&2
  echo "       then stage the regenerated clients/temper-py files." >&2
  exit 1
fi

# An orphan is a file the generator no longer writes but git still tracks: the manifest
# above catches its REMOVAL from the list, and this catches the file itself, which no
# diff of generated content can see. Cheap, and it is the one failure mode the gem's
# equivalent gate is blind to.
ORPHANS="$(
  comm -23 \
    <(git -C "$REPO_ROOT" ls-files -- "$GENERATED" | sed 's|^clients/temper-py/||' | sort) \
    <(sort "$REPO_ROOT/clients/temper-py/.openapi-generator/FILES")
)"
if [ -n "$ORPHANS" ]; then
  echo "ERROR: these files are tracked under $GENERATED but the generator no longer" >&2
  echo "       emits them. A retired operation or model leaves its file behind, where it" >&2
  echo "       keeps importing and keeps looking current:" >&2
  echo "$ORPHANS" | sed 's/^/         /' >&2
  echo "       Delete them and commit." >&2
  exit 1
fi

echo "temper-py generated package is up to date with openapi.json"
