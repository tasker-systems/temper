#!/bin/sh
# Vercel Ignored Build Step — decides whether this deployment builds.
#
# POLARITY IS INVERTED AND LOAD-BEARING: exit 0 SKIPS the build, exit 1 BUILDS it.
#
# WHY THIS EXISTS
#   Every preview deployment on this project was previously cancelled in 9-13s while
#   production built in 4-8 min. That is a deliberate cost decision — a Rust preview
#   build is expensive — but it meant the Neon preview branch cut for every push was
#   never exercised. A preview runs the PR's binary against its own database branch,
#   which is precisely the pairing rehearsal the schema/binary goal needs, so PRs that
#   touch migrations/ are worth the build and nothing else is.
#
#   Design: docs/superpowers/specs/2026-07-30-schema-binary-pairing-design.md § 4.
#
# THIS RUNS IN VERCEL'S BUILD CONTAINER, SPAWNED VIA `sh -c` — POSIX sh, not bash.
#
# WHY THE BASE IS THE MERGE BASE WITH main, AND NOT VERCEL_GIT_PREVIOUS_SHA
#   The obvious base is VERCEL_GIT_PREVIOUS_SHA, and it is the wrong one twice over.
#
#   It is "the SHA of the last SUCCESSFUL deployment for this project and branch", so on
#   a project whose previews have never built it is simply UNSET — the very condition
#   this script exists to fix is what withholds its own input. Measured, not reasoned:
#   the first deployment through this script logged `no VERCEL_GIT_PREVIOUS_SHA` and
#   built, and it only became set once that build succeeded.
#
#   Worse, where it IS set it answers the wrong question. It asks "did the last PUSH add
#   a migration", which flickers across pushes; the question that decides whether this
#   deployment is worth rehearsing is "does this PR carry a migration", which is stable.
#   A PR that adds a migration in its first commit and changes the Rust that calls it in
#   its second would skip exactly the push where the PAIRING changed — the one failure
#   this whole design exists to catch.
#
#   The merge base with the default branch answers the stable question, and is available
#   on the first push to a brand-new branch.
#
# WHAT THE BUILD CONTAINER ACTUALLY PROVIDES (measured 2026-07-30, real preview build)
#   git binary present; .git present; the clone is SHALLOW (10 commits) and does NOT
#   carry an origin/main ref, so the base must be fetched before it can be resolved.
#   `git fetch --no-tags --depth origin main` succeeds — network and credentials are
#   both available — and the merge base then resolves.
#
# CHANGED_PATHS is injected by the guard test and is never set in a real build.
set -u

# The branch every PR is cut from and merged back into. Not derivable from the Vercel
# environment — no VERCEL_GIT_* variable carries the repo's default branch.
DEFAULT_BRANCH="main"

# How much history to fetch when resolving the merge base. A branch that diverged
# further back than this resolves no merge base and therefore builds, which is the safe
# direction: too much rehearsal, never too little.
FETCH_DEPTH=200

# Production is never skipped. A cost optimisation must not be able to stop a deploy.
if [ "${VERCEL_ENV:-}" = "production" ]; then
  echo "build: VERCEL_ENV=production"
  exit 1
fi

# Fail safe: anything we do not positively recognise as a preview gets built.
if [ "${VERCEL_ENV:-}" != "preview" ]; then
  echo "build: VERCEL_ENV='${VERCEL_ENV:-}' not recognised — building rather than guessing"
  exit 1
fi

# `+x` rather than `-n`, deliberately: CHANGED_PATHS set-but-empty means "the changeset
# is empty", while CHANGED_PATHS absent means "we have no changeset signal". Those are
# different questions with different safe answers — skip the first, derive the second —
# and `-n` would collapse them into one, sending an empty changeset down the derivation
# path and building every time the test harness asserts a skip.
if [ -n "${CHANGED_PATHS+x}" ]; then
  changed="${CHANGED_PATHS}"
else
  if ! command -v git >/dev/null 2>&1 || [ ! -d .git ]; then
    echo "build: no git checkout in the build container — cannot determine the changeset"
    exit 1
  fi

  if ! git fetch --no-tags --depth="${FETCH_DEPTH}" origin "${DEFAULT_BRANCH}" >/dev/null 2>&1; then
    echo "build: could not fetch ${DEFAULT_BRANCH} — cannot determine the changeset"
    exit 1
  fi

  base="$(git merge-base FETCH_HEAD HEAD 2>/dev/null || true)"
  if [ -z "${base}" ]; then
    # Measured, not assumed: a real preview build reports
    # `no merge base with main within 200 commits` even though the fetch succeeded.
    # Vercel's clone is SHALLOW, and a shallow boundary commit has no recorded parents,
    # so the branch's history and the fetched main are two disconnected islands — no
    # common ancestor is reachable and DEEPENING THE FETCH DOES NOT HELP.
    #
    # Comparing the two trees directly needs no ancestry, so it works regardless. It can
    # over-report — a migration that landed on main but is not on this branch shows up as
    # a difference — and over-reporting BUILDS, which is the safe direction.
    base="FETCH_HEAD"
    echo "note: no merge base (shallow clone) — comparing trees against ${DEFAULT_BRANCH} tip"
  fi

  changed="$(git diff --name-only "${base}" HEAD 2>/dev/null || echo "__UNKNOWN__")"
fi

if [ "${changed}" = "__UNKNOWN__" ]; then
  echo "build: could not diff the changeset"
  exit 1
fi

# Only the migrations/ directory counts — not a doc that merely says "migrations".
if printf '%s\n' "${changed}" | grep -q '^migrations/'; then
  echo "build: this changeset touches migrations/ — rehearsing the schema change"
  exit 1
fi

# A change to what the BUILD DOES deserves the same rehearsal, by the same argument.
#
# The build stopped being a compile on 2026-07-31: `vercel.json`'s buildCommand now runs
# `temper-migrate --additive-only`, which applies schema. A change to that script,
# or to the vercel.json that invokes it, is a change to a mechanism that can BLOCK OR BREAK
# EVERY DEPLOY — and if only migration-carrying changesets built, such a change would first
# execute on the merge to main, in production, having never run anywhere.
#
# That is not hypothetical: it is what the register already recorded about the preview path
# itself, which sat provisioned and inert through its entire history until the canary shipped.
# The same trap, one level in.
#
# `scripts/vercel-*.sh` deliberately includes THIS script. A canary whose own change skipped
# its rehearsal would be the one file in the repo exempt from the rule it enforces.
if printf '%s\n' "${changed}" | grep -qE '^(vercel\.json|scripts/vercel-[a-z0-9-]*\.sh)$'; then
  echo "build: this changeset changes the build's own configuration — rehearsing it"
  exit 1
fi

echo "skip: preview with no migration or build-configuration change"
exit 0
