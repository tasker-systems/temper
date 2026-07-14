#!/usr/bin/env bash
#
# Fail if the committed temper-rb generated core drifts from openapi.json.
#
# Regenerates the gem (via generate-temper-rb.sh) and diffs the result against
# what is committed — the local mirror of the `test-ruby` CI job's `rake drift`.
# This exists so a DTO change cannot pass a green local `cargo make check` and
# then die in the Ruby CI drift gate (which is what bit issue #354).
#
# Requires Docker. When Docker is unavailable this SKIPS with a loud notice
# rather than failing: `cargo make check` must stay runnable on a machine with no
# Docker daemon (the CI ruby job is the backstop that never skips). Mirrors the
# reasoning that keeps the Docker-based `openapi-validate` out of the default gate.
#
# Usage: bash .github/scripts/check-temper-rb-drift.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GENERATED="clients/temper-rb/lib/temper/generated"
GENERATED_RB="clients/temper-rb/lib/temper/generated.rb"

if ! docker info >/dev/null 2>&1; then
  echo "SKIP: temper-rb drift check — Docker is not available." >&2
  echo "      The gem is generated from openapi.json; run 'cargo make openapi-rb' with" >&2
  echo "      Docker running to regenerate it. The test-ruby CI job is the backstop." >&2
  exit 0
fi

bash "$REPO_ROOT/.github/scripts/generate-temper-rb.sh"

# Assert both diff targets are TRACKED before diffing them. `git diff --exit-code -- <path>`
# exits 0 when the path matches nothing — untracked, ignored, moved, renamed — so the diff
# alone cannot distinguish "identical to what is committed" from "not committed at all". This
# gate is MORE exposed than the temper-ts sibling: it diffs a whole directory ($GENERATED), so
# a generator config change that relocated the output would silently empty the gate rather than
# fail it — a permanently-green no-op. (`ls-files --error-unmatch` on a directory succeeds only
# when at least one tracked file lives under it.) A gate that cannot fail is not a gate; make
# that state loud instead of green. Mirrors check-temper-ts-drift.sh.
if ! git -C "$REPO_ROOT" ls-files --error-unmatch -- "$GENERATED" "$GENERATED_RB" >/dev/null 2>&1; then
  echo "ERROR: $GENERATED or $GENERATED_RB is not tracked by git, so there is nothing to diff" >&2
  echo "       against. Either it is gitignored or the paths here have drifted from what" >&2
  echo "       generate-temper-rb.sh writes. Until that is fixed this gate checks nothing." >&2
  exit 1
fi

if ! git -C "$REPO_ROOT" diff --exit-code -- "$GENERATED" "$GENERATED_RB"; then
  echo >&2
  echo "ERROR: temper-rb generated core is out of date with openapi.json." >&2
  echo "       Run: cargo make openapi   (regenerates the spec and the gem)" >&2
  echo "       then commit the regenerated clients/temper-rb files." >&2
  exit 1
fi

echo "temper-rb generated core is up to date with openapi.json"
