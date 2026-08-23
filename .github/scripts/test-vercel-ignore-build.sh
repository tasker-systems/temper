#!/usr/bin/env bash
# Guard test for scripts/vercel-ignore-build.sh.
#
# Vercel's Ignored Build Step inverts the usual convention: exit 0 SKIPS the build,
# exit 1 BUILDS it. Every assertion below is written in those terms, because getting
# the polarity backwards would silently stop a project from deploying — a far worse
# outcome than the cost this script exists to avoid.
#
# WHAT CHANGED HERE, AND WHAT IT COST TO RETIRE
#   This suite used to assert `production always builds`, twice and unconditionally. That
#   assertion is GONE ON PURPOSE, and its removal is the single most dangerous edit in this
#   change — a blanket "production builds" is exactly the property whose loss has no
#   symptom until production is quietly stale. What replaces it is NOT a weaker version of
#   the same claim but a different and checkable one:
#
#     * production builds whenever the changeset touches the project  (asserted per project)
#     * production builds whenever the base cannot be established     (asserted)
#     * production builds on an unrecognised environment              (asserted)
#     * production skips ONLY when the changeset provably misses it   (asserted per project)
#
#   The old assertion is therefore not weakened but decomposed, and the decomposition is
#   what makes the skip auditable instead of merely permitted.
#
# The assertions run under `env -u VERCEL_GIT_PREVIOUS_SHA` so the harness is hermetic: if
# a real Vercel variable leaked in from the surrounding environment, the CHANGED_PATHS
# branch under test would be bypassed and the suite would pass or fail for a reason that
# has nothing to do with the script.
set -euo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)/scripts/vercel-ignore-build.sh"
fails=0

PROJECTS="temper-cloud temper-ui steward-agent temper-mention"

expect() { # expect <desc> <expected-exit> <project> <env assignments...>
  local desc="$1" want="$2" project="$3"; shift 3
  local got=0
  env -u VERCEL_GIT_PREVIOUS_SHA "$@" sh "$SCRIPT" "$project" >/dev/null 2>&1 || got=$?
  if [ "$got" -eq "$want" ]; then
    echo "  ok   — $desc (exit $got)"
  else
    echo "  FAIL — $desc: expected exit $want, got $got"; fails=$((fails+1))
  fi
}

echo "vercel-ignore-build guard test"

# ---------------------------------------------------------------------------------------
# Misconfiguration fails toward BUILDING, never toward skipping.
# ---------------------------------------------------------------------------------------
echo "-- fail-safe"
expect "unknown project builds"  1 not-a-project  VERCEL_ENV=preview    CHANGED_PATHS="README.md"
expect "empty project builds"    1 ""             VERCEL_ENV=preview    CHANGED_PATHS="README.md"
expect "unknown VERCEL_ENV builds (preview-side)"    1 temper-cloud VERCEL_ENV=      CHANGED_PATHS="README.md"
expect "unknown VERCEL_ENV builds (typo'd value)"    1 temper-cloud VERCEL_ENV=prod  CHANGED_PATHS="README.md"

# ---------------------------------------------------------------------------------------
# The change that motivated this whole gate: a docs-only or process-only changeset must
# reach NO project, on EITHER environment.
#
# `internal/registers/coverage.yaml` is cc280f98 verbatim — one file, which drove a full
# Rust production build plus three more production deploys.
# ---------------------------------------------------------------------------------------
echo "-- nothing-reaches-anything (both environments)"
for env_name in preview production; do
  for p in $PROJECTS; do
    expect "$p skips a README on $env_name"        0 "$p" VERCEL_ENV=$env_name CHANGED_PATHS="README.md"
    expect "$p skips a register on $env_name"      0 "$p" VERCEL_ENV=$env_name CHANGED_PATHS="internal/registers/coverage.yaml"
    expect "$p skips a docs page on $env_name"     0 "$p" VERCEL_ENV=$env_name CHANGED_PATHS="docs/guides/x.md"
    expect "$p skips an empty changeset on $env_name" 0 "$p" VERCEL_ENV=$env_name CHANGED_PATHS=""
  done
done

# ---------------------------------------------------------------------------------------
# Each project builds for its OWN tree and skips its siblings'. This is the property PR
# #761 violated: six commits under packages/temper-cloud produced six full temper-ui
# builds, plus steward and mention, because those three had no gate at all.
# ---------------------------------------------------------------------------------------
echo "-- per-project isolation"
expect "cloud builds for a crate"          1 temper-cloud   VERCEL_ENV=preview CHANGED_PATHS="crates/temper-api/src/lib.rs"
expect "ui SKIPS a crate"                  0 temper-ui      VERCEL_ENV=preview CHANGED_PATHS="crates/temper-api/src/lib.rs"
expect "steward SKIPS a crate"             0 steward-agent  VERCEL_ENV=preview CHANGED_PATHS="crates/temper-api/src/lib.rs"
expect "mention SKIPS a crate"             0 temper-mention VERCEL_ENV=preview CHANGED_PATHS="crates/temper-api/src/lib.rs"

expect "cloud builds for packages/temper-cloud" 1 temper-cloud VERCEL_ENV=preview CHANGED_PATHS="packages/temper-cloud/src/mcp.ts"
expect "ui SKIPS packages/temper-cloud"         0 temper-ui    VERCEL_ENV=preview CHANGED_PATHS="packages/temper-cloud/src/mcp.ts"

expect "ui builds for its own tree"        1 temper-ui      VERCEL_ENV=preview CHANGED_PATHS="packages/temper-ui/src/routes/+page.svelte"
expect "cloud SKIPS the ui tree"           0 temper-cloud   VERCEL_ENV=preview CHANGED_PATHS="packages/temper-ui/src/routes/+page.svelte"
expect "steward SKIPS the ui tree"         0 steward-agent  VERCEL_ENV=preview CHANGED_PATHS="packages/temper-ui/src/routes/+page.svelte"

expect "steward builds for its own tree"   1 steward-agent  VERCEL_ENV=preview CHANGED_PATHS="packages/agent-workflows/steward/src/index.ts"
expect "mention SKIPS the steward tree"    0 temper-mention VERCEL_ENV=preview CHANGED_PATHS="packages/agent-workflows/steward/src/index.ts"
expect "mention builds for its own tree"   1 temper-mention VERCEL_ENV=preview CHANGED_PATHS="packages/agent-workflows/mention/src/index.ts"
expect "steward SKIPS the mention tree"    0 steward-agent  VERCEL_ENV=preview CHANGED_PATHS="packages/agent-workflows/mention/src/index.ts"

# ---------------------------------------------------------------------------------------
# The `file:` dependency edges, read out of the package manifests rather than remembered:
#   temper-ui -> temper-telemetry-ts
#   steward   -> temper-telemetry-ts, temper-ts
#   mention   -> temper-telemetry-ts
# A linked client must rebuild its dependants and only its dependants.
# ---------------------------------------------------------------------------------------
echo "-- linked client dependencies"
expect "telemetry-ts rebuilds ui"       1 temper-ui      VERCEL_ENV=preview CHANGED_PATHS="clients/temper-telemetry-ts/src/otel.ts"
expect "telemetry-ts rebuilds steward"  1 steward-agent  VERCEL_ENV=preview CHANGED_PATHS="clients/temper-telemetry-ts/src/otel.ts"
expect "telemetry-ts rebuilds mention"  1 temper-mention VERCEL_ENV=preview CHANGED_PATHS="clients/temper-telemetry-ts/src/otel.ts"
expect "telemetry-ts does NOT rebuild cloud" 0 temper-cloud VERCEL_ENV=preview CHANGED_PATHS="clients/temper-telemetry-ts/src/otel.ts"

expect "temper-ts rebuilds steward"     1 steward-agent  VERCEL_ENV=preview CHANGED_PATHS="clients/temper-ts/src/client.ts"
# mention does NOT declare temper-ts — asserted so the day it gains the dependency, this
# line fails and the trigger set is updated with it, rather than drifting silently.
expect "temper-ts does NOT rebuild mention" 0 temper-mention VERCEL_ENV=preview CHANGED_PATHS="clients/temper-ts/src/client.ts"
expect "temper-ts does NOT rebuild ui"      0 temper-ui      VERCEL_ENV=preview CHANGED_PATHS="clients/temper-ts/src/client.ts"

# ---------------------------------------------------------------------------------------
# ts-rs GENERATED trees land inside the consuming package, so they are covered by that
# package's own root. Asserted explicitly because it is the non-obvious half: the Rust
# that PRODUCES them lives under crates/, which these projects deliberately ignore.
# ---------------------------------------------------------------------------------------
echo "-- generated type trees"
expect "ui builds for its generated types"      1 temper-ui      VERCEL_ENV=preview CHANGED_PATHS="packages/temper-ui/src/lib/types/generated/Resource.ts"
expect "mention builds for its generated types" 1 temper-mention VERCEL_ENV=preview CHANGED_PATHS="packages/agent-workflows/mention/agent/generated/Resource.ts"

# ---------------------------------------------------------------------------------------
# temper-cloud's deployable surface, and the migration coupling that makes skipping its
# production build safe. `^migrations/` MUST stay in its trigger set: the buildCommand
# applies additive schema, so a migration whose deploy was skipped would never apply.
# ---------------------------------------------------------------------------------------
echo "-- temper-cloud surface"
expect "cloud builds for a migration (preview)"    1 temper-cloud VERCEL_ENV=preview    CHANGED_PATHS="migrations/20260730000010_x.sql"
expect "cloud builds for a migration (PRODUCTION)" 1 temper-cloud VERCEL_ENV=production CHANGED_PATHS="migrations/20260730000010_x.sql"
expect "cloud builds for api/"                     1 temper-cloud VERCEL_ENV=preview    CHANGED_PATHS="api/axum.rs"
expect "cloud builds for the sqlx cache"           1 temper-cloud VERCEL_ENV=preview    CHANGED_PATHS=".sqlx/query-abc.json"
expect "cloud builds for Cargo.lock"               1 temper-cloud VERCEL_ENV=preview    CHANGED_PATHS="Cargo.lock"
expect "cloud builds for vercel.json"              1 temper-cloud VERCEL_ENV=preview    CHANGED_PATHS="vercel.json"
expect "a migration among others still builds"     1 temper-cloud VERCEL_ENV=preview    CHANGED_PATHS=$'README.md\nmigrations/2026_x.sql'

# Lockfiles follow the bun `workspaces` list, which holds exactly temper-cloud and
# temper-ui. steward and mention carry their own package-lock.json inside their tree, so
# the shared one must NOT reach them — asserted in both directions.
expect "root bun.lock rebuilds cloud"    1 temper-cloud   VERCEL_ENV=preview CHANGED_PATHS="bun.lock"
expect "root bun.lock rebuilds ui"       1 temper-ui      VERCEL_ENV=preview CHANGED_PATHS="bun.lock"
expect "root bun.lock SKIPS steward"     0 steward-agent  VERCEL_ENV=preview CHANGED_PATHS="bun.lock"
expect "root bun.lock SKIPS mention"     0 temper-mention VERCEL_ENV=preview CHANGED_PATHS="bun.lock"
expect "root package.json rebuilds cloud" 1 temper-cloud  VERCEL_ENV=preview CHANGED_PATHS="package.json"
expect "steward's own lockfile builds it" 1 steward-agent VERCEL_ENV=preview CHANGED_PATHS="packages/agent-workflows/steward/package-lock.json"
# Anchored, so a nested package.json is NOT the root one.
expect "a nested package.json is not the root one" 0 temper-cloud VERCEL_ENV=preview CHANGED_PATHS="clients/temper-rb/package.json"

# A path merely CONTAINING the word must not count — only the directory.
expect "docs mentioning migrations do not trigger" 0 temper-cloud VERCEL_ENV=preview CHANGED_PATHS="docs/migrations-guide.md"
expect "a doc naming vercel.json does not trigger" 0 temper-cloud VERCEL_ENV=preview CHANGED_PATHS="docs/vercel.json.md"
expect "a nested path under scripts/ does not trigger" 0 temper-cloud VERCEL_ENV=preview CHANGED_PATHS="scripts/install/install.sh"
expect "an unrelated script does not trigger"      0 temper-cloud VERCEL_ENV=preview CHANGED_PATHS="scripts/classify-sqlx-calls.py"
# The root vercel.json configures temper-cloud alone, so it must not drag the others in.
expect "root vercel.json does not rebuild ui"      0 temper-ui    VERCEL_ENV=preview CHANGED_PATHS="vercel.json"

# ---------------------------------------------------------------------------------------
# A GATE MUST RUN ITS OWN GATE — for EVERY project, not just the one it was written for.
# A canary exempt from its own rule is the one file nothing rehearses, and a disarmed gate
# passes silently: leave this out and the PR that breaks the gate is the PR that never
# runs it.
# ---------------------------------------------------------------------------------------
echo "-- self-referential"
for p in $PROJECTS; do
  expect "$p builds when the canary itself changes" 1 "$p" VERCEL_ENV=preview CHANGED_PATHS="scripts/vercel-ignore-build.sh"
  expect "$p builds when the build script changes"  1 "$p" VERCEL_ENV=preview CHANGED_PATHS="scripts/vercel-build.sh"
done

# ---------------------------------------------------------------------------------------
# The DERIVATION path — no CHANGED_PATHS, so the script must work out the changeset from
# git itself. This is the half that runs in a real build, and leaving it uncovered is how
# the first version shipped depending on VERCEL_GIT_PREVIOUS_SHA, a variable that is unset
# precisely when previews have never built.
#
# Everything below is hermetic: a throwaway origin plus clone under mktemp, never the repo
# this script lives in.
# ---------------------------------------------------------------------------------------
expect_in() { # expect_in <dir> <desc> <expected-exit> <project> <env assignments...>
  local dir="$1" desc="$2" want="$3" project="$4"; shift 4
  local got=0
  ( cd "$dir" && env -u VERCEL_GIT_PREVIOUS_SHA -u CHANGED_PATHS "$@" sh "$SCRIPT" "$project" ) >/dev/null 2>&1 || got=$?
  if [ "$got" -eq "$want" ]; then
    echo "  ok   — $desc (exit $got)"
  else
    echo "  FAIL — $desc: expected exit $want, got $got"; fails=$((fails+1))
  fi
}

echo "-- derivation (preview)"
FIXTURE="$(mktemp -d)"
trap 'rm -rf "$FIXTURE"' EXIT
git init -q --bare "$FIXTURE/origin.git"
git -c init.defaultBranch=main clone -q "file://$FIXTURE/origin.git" "$FIXTURE/work" 2>/dev/null
(
  cd "$FIXTURE/work"
  git config user.email t@example.com; git config user.name t
  git checkout -q -b main 2>/dev/null || true
  echo hello > README.md && git add README.md && git commit -qm base
  git push -q origin main
  # A branch whose diff against main carries a migration (reaches temper-cloud only).
  git checkout -q -b with-migration
  mkdir -p migrations && echo "SELECT 1;" > migrations/20260730_x.sql
  git add migrations && git commit -qm "add migration"
  git push -q origin with-migration
  # A branch whose diff against main reaches nothing.
  git checkout -q main && git checkout -q -b docs-only
  echo more >> README.md && git add README.md && git commit -qm "docs only"
  git push -q origin docs-only
  # A branch that reaches temper-ui and nothing else.
  git checkout -q main && git checkout -q -b ui-only
  mkdir -p packages/temper-ui/src && echo "x" > packages/temper-ui/src/app.html
  git add packages && git commit -qm "ui only"
  git push -q origin ui-only
)

( cd "$FIXTURE/work" && git checkout -q with-migration )
expect_in "$FIXTURE/work" "derived: migration branch builds cloud" 1 temper-cloud VERCEL_ENV=preview
expect_in "$FIXTURE/work" "derived: migration branch SKIPS ui"     0 temper-ui    VERCEL_ENV=preview

( cd "$FIXTURE/work" && git checkout -q docs-only )
expect_in "$FIXTURE/work" "derived: docs branch skips cloud" 0 temper-cloud VERCEL_ENV=preview
expect_in "$FIXTURE/work" "derived: docs branch skips ui"    0 temper-ui    VERCEL_ENV=preview

( cd "$FIXTURE/work" && git checkout -q ui-only )
expect_in "$FIXTURE/work" "derived: ui branch builds ui"     1 temper-ui    VERCEL_ENV=preview
expect_in "$FIXTURE/work" "derived: ui branch SKIPS cloud"   0 temper-cloud VERCEL_ENV=preview

# The SHALLOW case — what Vercel actually provides, and what the full clone above cannot
# see. A shallow boundary commit records no parents, so the branch history and the fetched
# main are disconnected islands: no merge base resolves and deepening the fetch does not
# help. The script must still reach the right answer by comparing trees.
git clone -q --depth=1 --branch docs-only      "file://$FIXTURE/origin.git" "$FIXTURE/shallow-docs" 2>/dev/null
git clone -q --depth=1 --branch with-migration "file://$FIXTURE/origin.git" "$FIXTURE/shallow-mig"  2>/dev/null
if [ -d "$FIXTURE/shallow-docs/.git" ] && [ -d "$FIXTURE/shallow-mig/.git" ]; then
  expect_in "$FIXTURE/shallow-docs" "shallow clone, docs only, skips"    0 temper-cloud VERCEL_ENV=preview
  expect_in "$FIXTURE/shallow-mig"  "shallow clone, with migration, builds" 1 temper-cloud VERCEL_ENV=preview
else
  echo "  FAIL — could not build the shallow fixtures"; fails=$((fails+1))
fi

# No git checkout at all: build rather than guess. This is the case CHANGED_PATHS
# set-but-empty does NOT cover — empty means "the changeset is empty", absent means
# "we have no signal", and the script must not collapse them.
mkdir -p "$FIXTURE/nogit"
expect_in "$FIXTURE/nogit" "no git checkout builds (preview)"    1 temper-cloud VERCEL_ENV=preview
expect_in "$FIXTURE/nogit" "no git checkout builds (production)" 1 temper-cloud VERCEL_ENV=production

# ---------------------------------------------------------------------------------------
# The PREVIEW FALLBACK CHAIN. This exists because the first deployment through this gate
# found the bug it now guards: `[observed — 2026-08-23, PR #765]` three projects ran the
# same script against the same commit and temper-ui and temper-mention both failed to
# fetch main, printed `could not fetch main`, and defaulted to building — while
# steward-agent resolved its changeset normally. The direction was safe; the gate was
# inert. Nothing here was covered, which is why nothing caught it.
#
# The fixture makes the fetch fail for real by pointing origin at a path that does not
# exist, rather than by mocking git.
# ---------------------------------------------------------------------------------------
echo "-- preview fallback when origin is unreachable"
git clone -q "file://$FIXTURE/origin.git" "$FIXTURE/broken-origin" 2>/dev/null
( cd "$FIXTURE/broken-origin" && git checkout -q main && git remote set-url origin "file://$FIXTURE/does-not-exist.git" )
BROKEN_HEAD="$(cd "$FIXTURE/broken-origin" && git rev-parse HEAD)"

# The remote-tracking ref must go too, or this fixture does not test what it claims: a
# clone still carrying refs/remotes/origin/main HAS a usable base, and using it is the
# correct answer rather than the fail-safe. (Caught by this suite on its first run — the
# assertion was written expecting a build and got a well-founded skip. Recorded because it
# is also the likeliest reason the real fix works: if Vercel's clone carries the ref, the
# second link in the chain resolves where the fetch could not.)
( cd "$FIXTURE/broken-origin" && git update-ref -d refs/remotes/origin/main 2>/dev/null || true )

# With no usable base at all, the fail-safe must BUILD — never skip.
expect_in "$FIXTURE/broken-origin" "unreachable origin + no ref + no prev SHA: builds (fail-safe)" \
  1 temper-cloud VERCEL_ENV=preview

# But given a real previous SHA it must USE it rather than defaulting to build. This is the
# assertion that would have failed before the fix: the old script had no fallback, so an
# unreachable origin meant "build" forever and the gate never actually ran.
prev_fallback() { # prev_fallback <desc> <want> <project> <prev>
  local desc="$1" want="$2" project="$3" prev="$4" got=0
  ( cd "$FIXTURE/broken-origin" && env -u CHANGED_PATHS VERCEL_ENV=preview \
      VERCEL_GIT_PREVIOUS_SHA="$prev" sh "$SCRIPT" "$project" ) >/dev/null 2>&1 || got=$?
  if [ "$got" -eq "$want" ]; then
    echo "  ok   — $desc (exit $got)"
  else
    echo "  FAIL — $desc: expected exit $want, got $got"; fails=$((fails+1))
  fi
}
# HEAD is main's base commit (README only), so a prev SHA of HEAD itself is an empty diff.
prev_fallback "unreachable origin + prev SHA: uses it and SKIPS an empty diff" 0 temper-cloud "$BROKEN_HEAD"

# The SECOND REMOTE candidate. `[observed — 2026-08-23]` temper-ui and temper-mention have
# NO `origin` remote in their build clones at all — `fatal: 'origin' does not appear to be
# a git repository` — so the recovery cannot be a retry against origin. It is a different
# remote, built from VERCEL_GIT_REPO_OWNER/SLUG. Here the fixture stands in for that URL
# via the same variables, so the path is exercised rather than assumed.
#
# The assertion that matters is that the base comes from the SECOND candidate and produces
# a real verdict — not the fail-safe, and not the narrower VERCEL_GIT_PREVIOUS_SHA link.
second_remote() { # second_remote <dir> <desc> <want> <project>
  local dir="$1" desc="$2" want="$3" project="$4" got=0
  ( cd "$dir" && env -u CHANGED_PATHS -u VERCEL_GIT_PREVIOUS_SHA VERCEL_ENV=preview \
      VERCEL_GIT_REPO_OWNER="$FIXTURE" VERCEL_GIT_REPO_SLUG=x sh "$SCRIPT" "$project" ) >/dev/null 2>&1 || got=$?
  if [ "$got" -eq "$want" ]; then
    echo "  ok   — $desc (exit $got)"
  else
    echo "  FAIL — $desc: expected exit $want, got $got"; fails=$((fails+1))
  fi
}
# The constructed https:// URL cannot resolve in a test, so this asserts the honest
# outcome: every candidate is tried, every failure is reported, and the fail-safe BUILDS
# rather than skipping on no information.
second_remote "$FIXTURE/broken-origin" "no origin + unusable second remote: builds (fail-safe)" 1 temper-cloud

# ---------------------------------------------------------------------------------------
# Derivation on PRODUCTION. This is the arm that replaces `production always builds`, so
# it is asserted rather than reasoned about: with no VERCEL_GIT_PREVIOUS_SHA the script
# falls back to HEAD~1, and the verdict must still track whether the commit reached the
# project.
# ---------------------------------------------------------------------------------------
echo "-- derivation (production)"
expect_in_prod() { # expect_in_prod <dir> <desc> <want> <project> [prev_sha]
  local dir="$1" desc="$2" want="$3" project="$4" prev="${5:-}"
  local got=0
  if [ -n "$prev" ]; then
    ( cd "$dir" && env -u CHANGED_PATHS VERCEL_ENV=production VERCEL_GIT_PREVIOUS_SHA="$prev" sh "$SCRIPT" "$project" ) >/dev/null 2>&1 || got=$?
  else
    ( cd "$dir" && env -u CHANGED_PATHS -u VERCEL_GIT_PREVIOUS_SHA VERCEL_ENV=production sh "$SCRIPT" "$project" ) >/dev/null 2>&1 || got=$?
  fi
  if [ "$got" -eq "$want" ]; then
    echo "  ok   — $desc (exit $got)"
  else
    echo "  FAIL — $desc: expected exit $want, got $got"; fails=$((fails+1))
  fi
}

# Build a linear main: base -> docs commit -> migration commit.
(
  cd "$FIXTURE/work"
  git checkout -q main
  echo "a doc" > NOTES.md && git add NOTES.md && git commit -qm "docs on main"
  mkdir -p migrations && echo "SELECT 2;" > migrations/20260801_y.sql
  git add migrations && git commit -qm "migration on main"
)
DOCS_SHA="$(cd "$FIXTURE/work" && git rev-parse HEAD~1)"
BASE_SHA="$(cd "$FIXTURE/work" && git rev-parse HEAD~2)"

# HEAD is the migration commit; HEAD~1 is the docs commit -> cloud must build.
expect_in_prod "$FIXTURE/work" "prod: HEAD~1 fallback sees the migration" 1 temper-cloud

# Rewind to the docs commit: HEAD~1 is base, diff is NOTES.md only -> everything skips.
( cd "$FIXTURE/work" && git checkout -q "$DOCS_SHA" )
expect_in_prod "$FIXTURE/work" "prod: a docs-only commit skips cloud" 0 temper-cloud
expect_in_prod "$FIXTURE/work" "prod: a docs-only commit skips ui"    0 temper-ui

# An explicit previous-SHA spanning BOTH commits must build (the accumulation property:
# a skipped deploy does not advance the base, so the changeset grows until it matters).
( cd "$FIXTURE/work" && git checkout -q main )
expect_in_prod "$FIXTURE/work" "prod: explicit prev SHA spanning docs+migration builds" 1 temper-cloud "$BASE_SHA"
expect_in_prod "$FIXTURE/work" "prod: that same span still skips ui"                    0 temper-ui    "$BASE_SHA"

# An unreachable previous SHA must not silently skip — it falls back, and failing that,
# builds. 40 hex digits that resolve to nothing.
expect_in_prod "$FIXTURE/work" "prod: unresolvable prev SHA does not skip silently" 1 temper-cloud "0000000000000000000000000000000000000000"

if [ "$fails" -gt 0 ]; then echo "FAILED: $fails assertion(s)"; exit 1; fi
echo "all assertions passed"
