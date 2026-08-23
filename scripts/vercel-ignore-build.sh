#!/bin/sh
# Vercel Ignored Build Step — decides whether this deployment builds.
#
# POLARITY IS INVERTED AND LOAD-BEARING: exit 0 SKIPS the build, exit 1 BUILDS it.
#
# Usage (from a project's vercel.json `ignoreCommand`):
#   sh "$(git rev-parse --show-toplevel)/scripts/vercel-ignore-build.sh" <project>
#
# where <project> is one of: temper-cloud, temper-ui, steward-agent, temper-mention.
#
# WHY THE INVOCATION GOES THROUGH `git rev-parse --show-toplevel`
#   Vercel runs the ignoreCommand from the project's **Root Directory**, which is a
#   dashboard setting this file cannot read and which differs per project — temper-cloud
#   builds from the repo root, the other three from their own package tree. A relative
#   path would therefore be correct for exactly one of the four. Resolving the repo root
#   from git makes one spelling work everywhere.
#
#   It is also fail-safe in the right direction: with no git checkout the substitution is
#   empty, `sh` cannot find the script, and a non-zero exit BUILDS.
#
# ── WHAT THIS REPLACED, AND WHY ────────────────────────────────────────────────────────
#
# This script used to serve temper-cloud alone, and only on preview. Two gaps followed
# from that, both measured rather than argued:
#
#   1. THE OTHER THREE PROJECTS HAD NO IGNORE STEP AT ALL. temper-ui, steward-agent and
#      temper-mention built on every push to every branch. PR #761 was six commits, all of
#      them under packages/temper-cloud, and it produced six full READY temper-ui preview
#      builds (dpl_2rBw18QYJQeveA3tv2opDfmjjXFr and five siblings) plus the same again for
#      steward and mention — ~18 builds of projects the PR did not touch. temper-cloud,
#      the actual subject, correctly cancelled all six.
#
#   2. PRODUCTION WAS EXEMPT WHOLESALE. The old rule was "production is never skipped: a
#      cost optimisation must not be able to stop a deploy". Sound as far as it goes, but
#      it meant a changeset that cannot alter the deployed artifact still paid a full
#      build on all four projects. Measured: cc280f98 changed exactly one file,
#      `internal/registers/coverage.yaml`, and drove production deploys of all four
#      including a full Rust build (dpl_Becya1uvj8yojtuEkKSwKoZodcg3). Ten of the forty
#      non-merge commits before it were the same shape.
#
# The old rule's INTENT survives and is what the trigger sets below encode: a cost
# optimisation must not be able to stop a deploy that matters. The change is that
# "matters" is now answered by asking whether the changeset can reach THIS project's
# output, instead of by declining to ask.
#
# ── WHY SKIPPING A PRODUCTION DEPLOY IS SAFE ───────────────────────────────────────────
#
# A skipped deployment leaves the production alias pointing at the previous one. That is
# the correct outcome precisely when the changeset could not have changed this project's
# output — the artifact that would have been built is the one already serving.
#
# The interaction that needs stating rather than assuming: since 2026-07-31 temper-cloud's
# buildCommand APPLIES additive schema (scripts/vercel-build.sh), so skipping its
# production build also skips a migration apply. That is safe only because `^migrations/`
# is in temper-cloud's trigger set — a changeset carrying a migration always builds, so
# there is never a pending migration whose deploy was skipped. Remove that entry and this
# paragraph stops being true.
#
# ── THIS RUNS IN VERCEL'S BUILD CONTAINER, SPAWNED VIA `sh -c` — POSIX sh, not bash ─────
#
# WHY THE PREVIEW BASE IS THE MERGE BASE WITH main, AND NOT VERCEL_GIT_PREVIOUS_SHA
#   The obvious base is VERCEL_GIT_PREVIOUS_SHA, and for a PREVIEW it is wrong twice over.
#
#   It is "the SHA of the last SUCCESSFUL deployment for this project and branch", so on
#   a project whose previews have never built it is simply UNSET — the very condition
#   this script exists to fix is what withholds its own input. Measured, not reasoned:
#   the first deployment through this script logged `no VERCEL_GIT_PREVIOUS_SHA` and
#   built, and it only became set once that build succeeded.
#
#   Worse, where it IS set it answers the wrong question. It asks "did the last PUSH
#   change something", which flickers across pushes; the question that decides whether
#   this deployment is worth building is "does this PR carry such a change", which is
#   stable. A PR that adds a migration in its first commit and changes the Rust that
#   calls it in its second would skip exactly the push where the PAIRING changed — the
#   one failure the schema/binary design exists to catch.
#
# WHY PRODUCTION USES VERCEL_GIT_PREVIOUS_SHA ANYWAY
#   Both objections are preview-specific and neither survives the move to main. Production
#   has always built, so the variable is set; and on main "what changed since the last
#   thing we deployed" IS the question — there is no PR whose interior could be split
#   across pushes.
#
#   Its behaviour under a SKIP is the property that makes this safe to iterate. A skipped
#   deployment is CANCELED, not successful, so the base does not advance and the changeset
#   accumulates until something in it triggers a build. The answer is in fact correct
#   either way: if the base did advance to a skipped commit, the diff from there still
#   contains every impacting change, because a skipped changeset by definition held none.
#
# WHAT THE BUILD CONTAINER ACTUALLY PROVIDES (measured 2026-07-30, real preview build)
#   git binary present; .git present; the clone is SHALLOW (10 commits) and does NOT
#   carry an origin/main ref, so the base must be fetched before it can be resolved.
#   `git fetch --no-tags --depth origin main` succeeds — network and credentials are
#   both available — and the merge base then resolves.
#
# CHANGED_PATHS and VERCEL_IGNORE_PROJECT are injected by the guard test and are never set
# in a real build.
set -u

# The branch every PR is cut from and merged back into. Not derivable from the Vercel
# environment — no VERCEL_GIT_* variable carries the repo's default branch.
DEFAULT_BRANCH="main"

# How much history to fetch when resolving the merge base. A branch that diverged
# further back than this resolves no merge base and therefore builds, which is the safe
# direction: too much building, never too little.
FETCH_DEPTH=200

PROJECT="${1:-${VERCEL_IGNORE_PROJECT:-}}"

# ---------------------------------------------------------------------------------------
# Per-project trigger sets.
#
# Each is an ERE matched against the changed-path list. A project builds when the
# changeset touches something that can reach ITS output, and skips otherwise.
#
# TWO RULES GOVERN WHAT BELONGS IN A SET, and both have already been paid for elsewhere in
# this repo:
#
#   * A GATE MUST RUN ITS OWN GATE. Every set includes `scripts/vercel-*.sh`, this script
#     among them. A canary whose own change skipped its rehearsal would be the one file
#     exempt from the rule it enforces — and a disarmed gate passes silently, so the PR
#     that breaks it would be precisely the PR that never runs it.
#
#   * INERTNESS IS PROVEN, NOT ASSUMED FROM A DIRECTORY NAME. The `file:` dependency
#     edges below were read out of the package manifests, not remembered:
#       packages/temper-ui            -> temper-telemetry-ts
#       packages/agent-workflows/steward -> temper-telemetry-ts, temper-ts
#       packages/agent-workflows/mention -> temper-telemetry-ts
#     A change to a linked client therefore rebuilds its dependants, and each dependant's
#     own tree covers its `vercel.json` and its ts-rs GENERATED types, which land inside
#     that tree (packages/temper-ui/src/lib/types/generated/,
#     packages/agent-workflows/mention/agent/generated/) rather than beside the Rust that
#     produced them.
#
#     The LOCKFILES divide on the same evidence. bun's `workspaces` list at the repo root
#     holds exactly two entries — packages/temper-cloud and packages/temper-ui — so the
#     root `bun.lock` and `package.json` govern those two builds and appear in both sets.
#     steward and mention each carry their OWN package-lock.json inside their tree, so
#     their tree root already covers it and the shared lockfile does not reach them.
# ---------------------------------------------------------------------------------------
case "${PROJECT}" in
  temper-cloud)
    # The Rust API and its deployable surface. `api/` holds the entrypoints, `crates/` the
    # workspace they compile from, `.sqlx/` the offline query cache the build reads under
    # SQLX_OFFLINE=true, and `migrations/` the schema this project's build APPLIES.
    TRIGGERS='^crates/|^packages/temper-cloud/|^api/|^migrations/|^\.sqlx/|^Cargo\.(toml|lock)$|^rust-toolchain(\.toml)?$|^(bun\.lock|package\.json)$|^vercel\.json$|^scripts/vercel-[a-z0-9-]*\.sh$'
    ;;
  temper-ui)
    TRIGGERS='^packages/temper-ui/|^clients/temper-telemetry-ts/|^(bun\.lock|package\.json)$|^scripts/vercel-[a-z0-9-]*\.sh$'
    ;;
  steward-agent)
    TRIGGERS='^packages/agent-workflows/steward/|^clients/temper-ts/|^clients/temper-telemetry-ts/|^scripts/vercel-[a-z0-9-]*\.sh$'
    ;;
  temper-mention)
    TRIGGERS='^packages/agent-workflows/mention/|^clients/temper-telemetry-ts/|^scripts/vercel-[a-z0-9-]*\.sh$'
    ;;
  *)
    # An unnamed or unrecognised project is a MISCONFIGURATION, and the safe response to
    # one is to build. Skipping would mean a project silently stopped deploying because
    # somebody renamed it — the failure mode with no symptom until production is stale.
    echo "build: project '${PROJECT}' is not one this script knows — building rather than guessing"
    exit 1
    ;;
esac

# FAIL CLOSED on an empty trigger set. `grep -qE ''` matches EVERY line, so an emptied
# TRIGGERS would make every changeset look impacting — which merely over-builds — but the
# reverse spelling of the same slip is what this guards: a set that silently matches
# nothing would skip every deploy forever. Checked rather than trusted, because it is one
# deleted string away and its symptom is a project that quietly stops shipping.
if [ -z "${TRIGGERS}" ]; then
  echo "FATAL: empty trigger set for '${PROJECT}' — refusing to decide." >&2
  exit 1
fi

# ---------------------------------------------------------------------------------------
# Validate the environment BEFORE the changeset, so the fail-safe is unconditional.
#
# This check used to live inside the derivation branch, which made it unreachable whenever
# a changeset was supplied directly. In a real build that is a distinction without a
# difference — CHANGED_PATHS is only ever set by the guard test — but "an unrecognised
# environment builds" is a safety guarantee, and a guarantee that holds only on the path
# somebody happened to exercise is not one.
# ---------------------------------------------------------------------------------------
if [ "${VERCEL_ENV:-}" != "production" ] && [ "${VERCEL_ENV:-}" != "preview" ]; then
  echo "build: VERCEL_ENV='${VERCEL_ENV:-}' not recognised — building rather than guessing"
  exit 1
fi

# ---------------------------------------------------------------------------------------
# Determine the changeset.
# ---------------------------------------------------------------------------------------

# `+x` rather than `-n`, deliberately: CHANGED_PATHS set-but-empty means "the changeset
# is empty", while CHANGED_PATHS absent means "we have no changeset signal". Those are
# different questions with different safe answers — skip the first, derive the second —
# and `-n` would collapse them into one, sending an empty changeset down the derivation
# path and building every time the test harness asserts a skip.
if [ -n "${CHANGED_PATHS+x}" ]; then
  changed="${CHANGED_PATHS}"
else
  if ! command -v git >/dev/null 2>&1 || [ ! -d .git ]; then
    # Root Directory may put us inside a package; the repo root is where .git lives.
    if command -v git >/dev/null 2>&1 && git rev-parse --show-toplevel >/dev/null 2>&1; then
      cd "$(git rev-parse --show-toplevel)" || {
        echo "build: cannot reach the repo root — cannot determine the changeset"
        exit 1
      }
    else
      echo "build: no git checkout in the build container — cannot determine the changeset"
      exit 1
    fi
  fi

  if [ "${VERCEL_ENV:-}" = "production" ]; then
    # See "WHY PRODUCTION USES VERCEL_GIT_PREVIOUS_SHA ANYWAY" above.
    if [ -n "${VERCEL_GIT_PREVIOUS_SHA:-}" ]; then
      base="${VERCEL_GIT_PREVIOUS_SHA}"
      # The shallow clone may not carry it. Deepen once; if it still will not resolve,
      # fall through to the parent commit rather than guessing.
      if ! git cat-file -e "${base}^{commit}" 2>/dev/null; then
        git fetch --no-tags --depth="${FETCH_DEPTH}" origin "${base}" >/dev/null 2>&1 || true
      fi
      if ! git cat-file -e "${base}^{commit}" 2>/dev/null; then
        echo "note: VERCEL_GIT_PREVIOUS_SHA ${base} is not reachable in this clone — using HEAD~1"
        base="HEAD~1"
      fi
    else
      echo "note: no VERCEL_GIT_PREVIOUS_SHA (first deploy through this gate) — using HEAD~1"
      base="HEAD~1"
    fi

    if ! git cat-file -e "${base}^{commit}" 2>/dev/null; then
      echo "build: no usable base on production — building rather than guessing"
      exit 1
    fi
  else
    # Preview — validated above, so this arm is reached only for VERCEL_ENV=preview.
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
      # over-report — a change that landed on main but is not on this branch shows up as a
      # difference — and over-reporting BUILDS, which is the safe direction.
      base="FETCH_HEAD"
      echo "note: no merge base (shallow clone) — comparing trees against ${DEFAULT_BRANCH} tip"
    fi
  fi

  changed="$(git diff --name-only "${base}" HEAD 2>/dev/null || echo "__UNKNOWN__")"
fi

if [ "${changed}" = "__UNKNOWN__" ]; then
  echo "build: could not diff the changeset"
  exit 1
fi

# ---------------------------------------------------------------------------------------
# Decide.
# ---------------------------------------------------------------------------------------
if printf '%s\n' "${changed}" | grep -qE "${TRIGGERS}"; then
  echo "build: ${PROJECT} — the changeset touches something this project is built from"
  printf '%s\n' "${changed}" | grep -E "${TRIGGERS}" | sed 's/^/  /'
  exit 1
fi

echo "skip: ${PROJECT} (${VERCEL_ENV:-unknown}) — nothing in this changeset reaches it"
exit 0
