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
# CHANGED_PATHS is injected by the guard test and is never set in a real build; there
# the changeset is derived from VERCEL_GIT_PREVIOUS_SHA, which Vercel sets in the build
# environment.
set -u

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

# ---------------------------------------------------------------------------------
# TEMPORARY DIAGNOSTIC — remove once the changeset source is settled.
#
# The first real deployment through this script hit the `no VERCEL_GIT_PREVIOUS_SHA`
# branch and built. That variable is "the SHA of the last SUCCESSFUL deployment for this
# project and branch", and previews on this project have never built successfully — so
# the condition this script exists to fix is what makes its own input unavailable.
#
# This block measures what a changeset could actually be derived from, rather than
# guessing. It prints NAMES and yes/no facts only, never a variable's value.
# It does not affect the decision below: exit codes are unchanged, so the guard test
# still passes and CI stays green.
# ---------------------------------------------------------------------------------
diag() { echo "diag: $*"; }
diag "VERCEL_GIT_* variables SET (names only):"
env | sed -n 's/^\(VERCEL_GIT_[A-Z_]*\)=.*/  \1/p' | sort || true
for v in VERCEL_GIT_PREVIOUS_SHA VERCEL_GIT_COMMIT_SHA VERCEL_GIT_COMMIT_REF VERCEL_ENV; do
  eval "val=\${$v+set}"
  diag "$v: ${val:-UNSET}"
done
if command -v git >/dev/null 2>&1; then
  diag "git binary: present"
  if [ -d .git ]; then
    diag ".git directory: present"
    diag "is-shallow: $(git rev-parse --is-shallow-repository 2>/dev/null || echo '?')"
    diag "commits reachable from HEAD: $(git rev-list --count HEAD 2>/dev/null || echo '?')"
    diag "remotes configured (names only): $(git remote 2>/dev/null | tr '\n' ' ' || echo none)"
    if git rev-parse --verify origin/main >/dev/null 2>&1; then
      diag "origin/main ref: already present locally"
    else
      diag "origin/main ref: absent locally"
    fi
    if git fetch --no-tags --depth=50 origin main >/dev/null 2>&1; then
      diag "fetch origin main: SUCCEEDED"
      diag "merge-base with FETCH_HEAD: $(git merge-base FETCH_HEAD HEAD >/dev/null 2>&1 && echo resolvable || echo unresolvable)"
    else
      diag "fetch origin main: FAILED (no network, no credentials, or no remote)"
    fi
  else
    diag ".git directory: ABSENT — no git history in the build container"
  fi
else
  diag "git binary: ABSENT"
fi
# --------------------------- end temporary diagnostic ---------------------------

# `+x` rather than `-n`, deliberately: CHANGED_PATHS set-but-empty means "the changeset
# is empty", while CHANGED_PATHS absent means "we have no changeset signal". Those are
# different questions with different safe answers — skip the first, build the second —
# and `-n` would collapse them into one, sending an empty changeset down the
# cannot-determine path and building every time the test harness asserts a skip.
if [ -n "${CHANGED_PATHS+x}" ]; then
  changed="${CHANGED_PATHS}"
elif [ -n "${VERCEL_GIT_PREVIOUS_SHA:-}" ]; then
  changed="$(git diff --name-only "${VERCEL_GIT_PREVIOUS_SHA}" HEAD 2>/dev/null || echo "__UNKNOWN__")"
else
  # No previous SHA (first deployment on a branch): we cannot tell, so build.
  echo "build: no VERCEL_GIT_PREVIOUS_SHA — cannot determine the changeset"
  exit 1
fi

if [ "${changed}" = "__UNKNOWN__" ]; then
  echo "build: could not diff against VERCEL_GIT_PREVIOUS_SHA"
  exit 1
fi

# Only the migrations/ directory counts — not a doc that merely says "migrations".
if printf '%s\n' "${changed}" | grep -q '^migrations/'; then
  echo "build: this changeset touches migrations/ — rehearsing the schema change"
  exit 1
fi

echo "skip: preview with no migration change"
exit 0
