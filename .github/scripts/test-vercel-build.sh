#!/usr/bin/env bash
# .github/scripts/test-vercel-build.sh
#
# Test harness for scripts/vercel-build.sh — the Build Command that applies the deploy's own
# additive migrations (spec §3).
#
# WHAT IS ACTUALLY UNDER TEST
# ---------------------------
# Not the migrating: that is `migrate_ledger.rs`, covered by its own unit tests and by
# `migrate_additive_run.rs` against a real database. What lives ONLY in this script, and would
# otherwise be exercised for the first time in a production deploy, is three decisions:
#
#   1. WHICH CONNECTION. The migrator takes a Postgres advisory lock, which is session state, so
#      it must use Neon's direct endpoint (DATABASE_URL_UNPOOLED) and not the pooled one. Getting
#      this backwards does not fail loudly — it fails intermittently, under concurrency.
#
#   2. WHAT A MISSING DATABASE URL MEANS. `[decided — 2026-07-31, Pete]` It skips, loudly, because
#      a self-hosted or forked target legitimately has no build-time database. The hazard of that
#      decision is a renamed variable silently disabling the whole mechanism, so the test asserts
#      the runner is NOT invoked and that the log says so — a skip that ran quietly would be
#      indistinguishable from a success.
#
#   3. WHAT A HALT DOES TO THE BUILD. `[decided — 2026-07-31, Pete]` Exit 3 from the runner fails
#      the deploy. This is the whole point of the gate, and a sign error here would let a binary
#      ship ahead of its schema while every check stayed green — the same class of silent
#      inversion that `test-vercel-ignore-build.sh` exists to catch next door.
#
# The seam is MIGRATE_CMD, which the real build never sets — the same shape as CHANGED_PATHS in
# vercel-ignore-build.sh.
#
#   bash .github/scripts/test-vercel-build.sh

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
BUILD_SH="${REPO_ROOT}/scripts/vercel-build.sh"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; shift; printf '    %s\n' "$@"; FAIL=$((FAIL + 1)); }

[ -f "$BUILD_SH" ] || { echo "vercel-build.sh not found at $BUILD_SH" >&2; exit 1; }

# A stand-in for the migration runner. Records the DATABASE_URL it was handed, and exits with
# whatever STUB_RC asks for — which is how a halt (3) and a failed apply (1) are told apart.
STUB="${WORK}/stub-runner.sh"
cat > "$STUB" <<'STUBSH'
#!/bin/sh
echo "${DATABASE_URL:-<unset>}" > "${STUB_SAW}"
exit "${STUB_RC:-0}"
STUBSH
chmod +x "$STUB"

# Run the build script in a controlled environment. Every variable the script reads is cleared
# first, so a leaked value from the developer's own shell cannot decide a verdict.
run_build() {
  ( cd "$REPO_ROOT" && env -u DATABASE_URL -u DATABASE_URL_UNPOOLED -u VERCEL_ENV \
      -u VERCEL_GIT_COMMIT_SHA "$@" \
      STUB_SAW="${WORK}/saw" MIGRATE_CMD="$STUB" sh "$BUILD_SH" 2>&1 )
}

saw() { cat "${WORK}/saw" 2>/dev/null || echo "<not invoked>"; }
reset_saw() { rm -f "${WORK}/saw"; }

echo "test-vercel-build"
echo

# ── 1. The stub is live ─────────────────────────────────────────────────────────────────────────
# A stub that never ran would satisfy the skip case silently and make every other case a lie.
reset_saw
out="$(run_build DATABASE_URL=postgres://plain)"; rc=$?
if [ "$rc" -eq 0 ] && [ "$(saw)" = "postgres://plain" ]; then
  ok "sanity: the runner is invoked and sees DATABASE_URL"
else bad "sanity: the runner is invoked and sees DATABASE_URL" "exit=$rc" "saw=$(saw)" "$out"; fi

# ── 2. The unpooled URL wins when both are present ──────────────────────────────────────────────
reset_saw
out="$(run_build DATABASE_URL=postgres://pooled DATABASE_URL_UNPOOLED=postgres://direct)"; rc=$?
if [ "$rc" -eq 0 ] && [ "$(saw)" = "postgres://direct" ]; then
  ok "DATABASE_URL_UNPOOLED is preferred over the pooled URL"
else bad "DATABASE_URL_UNPOOLED is preferred over the pooled URL" "exit=$rc" "saw=$(saw)" "$out"; fi

# ── 3. The pooled URL is used when it is all there is ───────────────────────────────────────────
# A self-hosted or local target has no unpooled variable; refusing there would be a regression
# dressed as strictness.
reset_saw
out="$(run_build DATABASE_URL=postgres://only-pooled)"; rc=$?
if [ "$rc" -eq 0 ] && [ "$(saw)" = "postgres://only-pooled" ]; then
  ok "DATABASE_URL alone is used, not refused"
else bad "DATABASE_URL alone is used, not refused" "exit=$rc" "saw=$(saw)" "$out"; fi

# ── 4. No database URL at all: skip, loudly, WITHOUT invoking the runner ────────────────────────
reset_saw
out="$(run_build)"; rc=$?
if [ "$rc" -eq 0 ] \
   && [ "$(saw)" = "<not invoked>" ] \
   && printf '%s' "$out" | grep -q 'SKIPPED'; then
  ok "no database URL skips loudly and does not invoke the runner"
else bad "no database URL skips loudly and does not invoke the runner" "exit=$rc" "saw=$(saw)" "$out"; fi

# ── 5. A HALT fails the build ───────────────────────────────────────────────────────────────────
# The load-bearing one. `[decided — 2026-07-31, Pete]`
reset_saw
out="$(run_build DATABASE_URL=postgres://x STUB_RC=3)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'shape-breaking migration is pending'; then
  ok "a halted run (exit 3) fails the build, naming why"
else bad "a halted run (exit 3) fails the build, naming why" "exit=$rc" "$out"; fi

# ── 6. A FAILED apply fails the build, and is not reported as a refusal ─────────────────────────
# Exit 3 is a refusal; anything else is a migration that broke. Collapsing them would send an
# operator to the cutover runbook for a genuine error.
reset_saw
out="$(run_build DATABASE_URL=postgres://x STUB_RC=1)"; rc=$?
if [ "$rc" -ne 0 ] \
   && printf '%s' "$out" | grep -q 'exited 1' \
   && ! printf '%s' "$out" | grep -q 'shape-breaking migration is pending'; then
  ok "a failed apply fails the build and is distinguished from a refusal"
else bad "a failed apply fails the build and is distinguished from a refusal" "exit=$rc" "$out"; fi

# ── 7. vercel.json actually points at this script ───────────────────────────────────────────────
# Every assertion above is about a script that only matters if the deploy runs it. A guard that
# never checked the wiring would stay green through a rename.
if grep -q '"buildCommand": *"sh scripts/vercel-build.sh"' "${REPO_ROOT}/vercel.json"; then
  ok "vercel.json's buildCommand invokes this script"
else bad "vercel.json's buildCommand invokes this script" "$(grep -n buildCommand "${REPO_ROOT}/vercel.json" || echo '(no buildCommand at all)')"; fi

echo
echo "  ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
