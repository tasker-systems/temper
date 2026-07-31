#!/bin/sh
# Vercel Build Command — the deploy applies its own additive schema.
#
# WHY THIS EXISTS
#   Spec §3 (docs/superpowers/specs/2026-07-30-schema-binary-pairing-design.md):
#
#     "A migration declaring itself additive is applied during the build phase, by a
#      binary that contains it."
#     "Shape-breaking migrations are never auto-applied. They remain operator-gated
#      cutover."
#     "The build applies the pending set only while every member of it is additive, and
#      halts at the first shape-breaking migration rather than running past it."
#
#   Before this, `vercel.json` had no buildCommand at all and production migrations were
#   applied entirely by hand. The consequence was not merely manual: the retroactive
#   backfill in `20260731000020` has already fired and can never fire again, so every
#   hand-applied production migration from here leaves a PERMANENT NULL in
#   `migration_current.state` — a gap nothing later fills. Production's outcome axis stays
#   empty until this script runs. `[observed — 2026-07-31]`
#
#   On a PREVIEW this applies the PR's own migration to the PR's own Neon branch — the
#   rehearsal half. On PRODUCTION it applies the additive backlog at deploy time, which is
#   the manual step it replaces.
#
# THIS RUNS IN VERCEL'S BUILD CONTAINER, SPAWNED VIA `sh -c` — POSIX sh, not bash.
#
# WHAT THE BUILD CONTAINER PROVIDES (measured 2026-07-30, real preview build; spec § Evidence)
#   cargo and rustc pre-installed at /rust/bin and already on PATH; DATABASE_URL and
#   DATABASE_URL_UNPOOLED both SET; SQLX_OFFLINE set by vercel.json. A buildCommand runs
#   BEFORE the @vercel/rust builders (@vercel/static-build priority 0, @vercel/rust
#   priority 1), which is why the schema is in place before any function is built.
#
# THE @vercel/rust BUILDERS DO NOT SHARE THIS STEP'S TARGET DIRECTORY
#   `[observed — 2026-07-31]` Settled by the temper-migrate extraction, which was an
#   A/B on exactly this question. No log can answer it directly: those builders emit ZERO
#   lines, so the ~6m they run is silent (dpl_DrKxz1F67xiPe1zztMerweYZ7QTE). The earlier
#   note here inferred "they do not reuse it" from that silence, which is what BOTH answers
#   look like — the inference happened to be right and was not evidence.
#
#   The extraction removed ort, tokenizers, temper-core and temper-substrate from THIS
#   step's compile. If the directory were shared, the builders had been getting all of that
#   pre-built for free and would now have to build it themselves — ~3m more work:
#
#     dpl_DrKxz…  before  this step 2m55s   builders 5m56s   total 9m23s   cache 1.61 GB
#     dpl_C6tEG…  after   this step 1m24s   builders 6m11s   total 8m04s   cache 1.23 GB
#
#   The builders moved +15s, not +3m. They were always compiling that base themselves, so
#   the ~1m31s this step no longer spends is a real saving rather than work displaced
#   downstream. (Totals reconcile: 1m31s saved − 15s = 1m16s ≈ the 1m19s observed.)
#
# AND THE BUILD CACHE NOW FITS, WHICH MAY MATTER MORE THAN THE MINUTE
#   Before, EVERY build ended `Build cache size 1.61 GB exceeds limit of 1.50 GB.
#   Invalidating cache.` — so the next one always started clean and cold was the steady
#   state, structurally. Dropping ort from this step's target directory took the cache to
#   1.23 GB, and dpl_C6tEG… is the first build here to report `Created build cache` and
#   `Build cache uploaded` instead. A warm restore has NOT been observed yet — the first
#   production build after the extraction still began clean, because its predecessor on
#   that lineage carried the old oversized cache. Watch for `Restored build cache from
#   previous deployment` on a subsequent build; if it appears, warm builds become possible
#   here for the first time, and this step's cost drops again.
#
# WHY THE UNPOOLED URL
#   sqlx's Migrator takes a Postgres advisory lock for exclusive access
#   (sqlx-core-0.8.6/src/migrate/migrator.rs:53,146). An advisory lock is session state,
#   and Neon's pooled endpoint is PgBouncer in transaction mode, which does not keep a
#   session pinned to a client. DATABASE_URL_UNPOOLED is the direct connection.
#
# WHY THE HALT FAILS THE BUILD
#   `[decided — 2026-07-31, Pete]` The spec deliberately left this to step 4. Failing
#   means the binary stays at N-1 against schema N-1 — consistent, and the safe direction.
#   Warning would ship a binary that expects schema it does not have: the 2026-07-30
#   outage inverted, and there is no way at build time to know whether the new code path
#   is exercised. The cost is real and accepted: a merged-but-unapplied shape-breaking
#   migration blocks every subsequent deploy until an operator takes it or it is reverted.
#   That is loud, which is the point.
#
#   The SAME rule runs on preview. One rule, no environment branch — a preview whose
#   binary and schema were deliberately mispaired is a confusing thing to hand a reviewer,
#   and "this PR needs a cutover" is a verdict worth getting at PR time.
#
# WHY A MISSING DATABASE URL SKIPS RATHER THAN FAILS
#   `[decided — 2026-07-31, Pete]` Not every deploy target has a build-time database:
#   DEPLOYING.md documents self-hosted projects as operator-migrated, and a fork has no
#   Neon at all. Failing there would break deploys that were never broken. The skip is
#   LOUD on purpose — its risk is that a renamed variable silently disables the whole
#   mechanism, and the only trace would be the absence of `recorded_by = 'runner'` rows in
#   `kb_migration_ledger`, which is exactly the kind of absence nobody notices.
#
# WHY THIS WRITES A MARKER FILE — measured twice, and neither guess was right
#   Setting a buildCommand AT ALL makes Vercel run a STATIC-BUILD phase, and that phase
#   demands an output directory even on a project whose only outputs are `api/*.rs`. Two
#   builds established the exact requirement, because the first fix was still wrong:
#
#     `[observed — 2026-07-31, dpl_CrJj58WNjcWZfhuDfwHRHSr7DKwj]`
#       Error: No Output Directory named "public" found after the Build completed.
#     `[observed — 2026-07-31, dpl_6kUZW1iTyzDkt1KvDrX6uAHNAjK2]`  (with `mkdir -p public`)
#       Error: The Output Directory "public" is empty.
#
#   So the directory must EXIST and be NON-EMPTY. An empty directory was the tidier answer
#   and it is simply not one Vercel accepts.
#
#   The spec's § Evidence probe established that a buildCommand RUNS here, that cargo is on
#   PATH, and that DATABASE_URL_UNPOOLED is set — all true, and none of it establishes that
#   a build with a buildCommand set SUCCEEDS. It was quoted from the HEAD of its log; both
#   of these errors appear at the tail.
#
# WHY THIS PARTICULAR FILENAME
#   `vercel.json`'s first route is `{ "handle": "filesystem" }`, so anything in the output
#   directory is SERVED and shadows the function routes at that exact path. The name is
#   therefore chosen not to collide with anything that must reach a function:
#     * not `index.html` — that would answer `/`, which belongs to `/api/axum`;
#     * not anything under `.well-known/` — those route to `/api/mcp` (OAuth discovery);
#     * not `_vercel/…` — reserved by the platform.
#   `/build-marker.txt` collides with nothing and answers a path nothing else claims.
#
# MIGRATE_CMD is injected by the guard test and is never set in a real build.
set -u

echo "build: VERCEL_ENV=${VERCEL_ENV:-unset} commit=${VERCEL_GIT_COMMIT_SHA:-unknown}"

mkdir -p public
cat > public/build-marker.txt <<MARKER
This project has no static output. Its deployable surface is api/*.rs.

This file exists only because Vercel's static-build phase — which a buildCommand
turns on — refuses both a missing output directory and an empty one. See
scripts/vercel-build.sh.
MARKER

# Prefer the direct connection; fall back to whatever DATABASE_URL is, which is what a
# local or self-hosted invocation has.
if [ -n "${DATABASE_URL_UNPOOLED:-}" ]; then
  MIGRATE_URL="${DATABASE_URL_UNPOOLED}"
  echo "migrate: using DATABASE_URL_UNPOOLED (the migrator needs a session-pinned connection)"
elif [ -n "${DATABASE_URL:-}" ]; then
  MIGRATE_URL="${DATABASE_URL}"
  echo "migrate: DATABASE_URL_UNPOOLED is unset — falling back to DATABASE_URL"
else
  echo "migrate: SKIPPED — neither DATABASE_URL_UNPOOLED nor DATABASE_URL is set."
  echo "migrate: no migration was applied and none was checked. If this target is expected"
  echo "migrate: to self-migrate, its build environment is misconfigured; if it is"
  echo "migrate: operator-migrated (DEPLOYING.md), this is the intended path."
  exit 0
fi

# The runner reads DATABASE_URL (temper-migrate's `connect`), so the choice above is
# expressed by exporting it rather than by a flag. Keeping the Vercel-specific variable
# knowledge in this script leaves the Rust side with one way to be told.
export DATABASE_URL="${MIGRATE_URL}"

: "${MIGRATE_CMD:=cargo run --release --locked -p temper-migrate --bin temper-migrate -- --additive-only}"

# shellcheck disable=SC2086
$MIGRATE_CMD
rc=$?

if [ "$rc" -eq 0 ]; then
  exit 0
fi

echo "" >&2
if [ "$rc" -eq 3 ]; then
  # Exit 3 is the runner's refusal, distinct from anyhow's 1, so this branch cannot be
  # reached by a migration that merely failed to apply.
  echo "BUILD FAILED: a shape-breaking migration is pending and the deploy will not apply it." >&2
  echo "" >&2
  echo "An operator must take it as a cutover (DEPLOYING.md § 'Shape-breaking migration')." >&2
  echo "Until then this deploy is refused, because shipping the binary without its schema is" >&2
  echo "the failure this gate exists to prevent." >&2
else
  echo "BUILD FAILED: the migration runner exited ${rc}." >&2
  echo "This is an apply that FAILED, not one that was refused — see the ledger:" >&2
  echo "  SELECT * FROM migration_current WHERE state <> 'success' ORDER BY version;" >&2
fi
exit "$rc"
