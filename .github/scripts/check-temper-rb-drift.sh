#!/usr/bin/env bash
#
# Fail if the committed temper-rb generated core drifts from openapi.json.
#
# Regenerates the gem (via generate-temper-rb.sh) and diffs the result against what
# is committed. This is the ONE implementation of the gem's drift assertion: the
# `test-ruby` CI job reaches it through the Rakefile's `rake drift`, and local dev
# through `cargo make openapi-rb-drift`. It exists so a DTO change cannot pass a green
# local `cargo make check` and then die in the Ruby CI drift gate (which is what bit
# issue #354). The Rakefile used to carry its own copy of the diff; that copy drifted —
# it never grew the manifest and orphan checks below — which is the whole argument for
# there being one.
#
# WHEN THIS SKIPS. It runs whichever generator path the host HAS — Docker or a JVM —
# and skips only when BOTH are missing, which is the weakest form that still catches
# the artifact's failure mode. `cargo make check` must stay runnable on a machine with
# neither, and the test-ruby CI job resolves to the Docker path (the runner has a
# daemon, and generate-temper-rb.sh prefers it), so in CI this never skips.
#
# This used to skip whenever Docker was absent, which was stricter than its own
# generator needs — generate-temper-rb.sh has had a Java + pinned-jar fallback all
# along, emitting byte-identical output from the same pinned version. The cost of the
# stricter rule was not theoretical: the two orphans this gate now catches sat in the
# tree while a Docker-less `cargo make check` printed SKIP. Matches
# check-temper-py-drift.sh, which shares this generator.
#
# Usage: bash .github/scripts/check-temper-rb-drift.sh

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GENERATED="clients/temper-rb/lib/temper/generated"
GENERATED_RB="clients/temper-rb/lib/temper/generated.rb"
MANIFEST="clients/temper-rb/.openapi-generator"

if ! (command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1) \
   && ! command -v java >/dev/null 2>&1; then
  echo "SKIP: temper-rb drift check — neither Docker nor a Java runtime is available." >&2
  echo "      The gem is generated from openapi.json; run 'cargo make openapi-rb' on a" >&2
  echo "      host with either one. The test-ruby CI job is the backstop." >&2
  exit 0
fi

bash "$REPO_ROOT/.github/scripts/generate-temper-rb.sh"

# Assert all three diff targets are TRACKED before diffing them. `git diff --exit-code -- <path>`
# exits 0 when the path matches nothing — untracked, ignored, moved, renamed — so the diff
# alone cannot distinguish "identical to what is committed" from "not committed at all". This
# gate is MORE exposed than the temper-ts sibling: it diffs a whole directory ($GENERATED), so
# a generator config change that relocated the output would silently empty the gate rather than
# fail it — a permanently-green no-op. (`ls-files --error-unmatch` on a directory succeeds only
# when at least one tracked file lives under it.) A gate that cannot fail is not a gate; make
# that state loud instead of green. Mirrors check-temper-ts-drift.sh.
#
# $MANIFEST is a diff target and not merely a witness: `.openapi-generator/FILES` is the
# generator's own list of what it wrote, so a file that STOPS being generated shows up there as
# a deletion. A diff over $GENERATED alone can never see that — openapi-generator does not
# delete output it no longer emits, so the retired file just sits on disk, tracked and stale,
# and `git diff` reports no change because nothing changed it. That is exactly how
# `reassign_api.rb` (from a tag the contract dropped) and `artifact_count_row.rb` survived a
# green gate. Ported from check-temper-py-drift.sh, which got this right first.
if ! git -C "$REPO_ROOT" ls-files --error-unmatch -- "$GENERATED" "$GENERATED_RB" "$MANIFEST" >/dev/null 2>&1; then
  echo "ERROR: $GENERATED, $GENERATED_RB or $MANIFEST is not tracked by git, so there is" >&2
  echo "       nothing to diff against. Either it is gitignored or the paths here have drifted" >&2
  echo "       from what generate-temper-rb.sh writes. Until that is fixed this gate checks" >&2
  echo "       nothing." >&2
  exit 1
fi

if ! git -C "$REPO_ROOT" diff --exit-code -- "$GENERATED" "$GENERATED_RB" "$MANIFEST"; then
  echo >&2
  echo "ERROR: temper-rb generated core is out of date with openapi.json." >&2
  echo "       Run: cargo make openapi   (regenerates the spec and the gem)" >&2
  echo "       then commit the regenerated clients/temper-rb files." >&2
  exit 1
fi

# An orphan is a file the generator no longer writes but git still tracks: the manifest diff
# above catches its REMOVAL from the list, and this catches the file itself, which no diff of
# generated content can see. The two halves are complementary — the manifest goes red only on
# the commit that retires the file, while this stays red for as long as the file is tracked, so
# an orphan committed before the manifest was a diff target is still caught. Cheap, and it is
# the failure mode this gate was blind to until the temper-py gate showed the fix.
ORPHANS="$(
  comm -23 \
    <(git -C "$REPO_ROOT" ls-files -- "$GENERATED" "$GENERATED_RB" | sed 's|^clients/temper-rb/||' | sort) \
    <(sort "$REPO_ROOT/clients/temper-rb/.openapi-generator/FILES")
)"
if [ -n "$ORPHANS" ]; then
  echo "ERROR: these files are tracked under clients/temper-rb but the generator no longer" >&2
  echo "       emits them. A retired operation or model leaves its file behind, where it keeps" >&2
  echo "       loading and keeps looking current:" >&2
  echo "$ORPHANS" | sed 's/^/         /' >&2
  echo "       Delete them and commit." >&2
  exit 1
fi

echo "temper-rb generated core is up to date with openapi.json"
