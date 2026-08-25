#!/usr/bin/env bash
# .github/scripts/test-migrate-cutover.sh
#
# Test harness for scripts/migrate-cutover.sh — the pre-flight an operator runs before
# applying a shape-breaking migration by hand (DEPLOYING.md § "Cutover command").
#
# WHAT IS ACTUALLY UNDER TEST
# ---------------------------
# Not the migrating: that is temper-migrate, covered by its own tests. What lives ONLY in
# this script is a refusal, and a refusal is exactly the thing that is never exercised
# until the day it matters. Three conjuncts, and they are not interchangeable:
#
#   0. DATABASE_URL names the target. `connect()` falls back to the local dev database
#      when it is unset (crates/temper-migrate/src/lib.rs:44-45), so every OTHER check
#      here can pass while the bytes land on the operator's laptop and the run reports
#      success.
#
#   A. Every `migrations/*.sql` on disk is byte-identical to HEAD's blob, and the two sets
#      agree. NOT `git status --porcelain` alone: porcelain answers "does the index
#      disagree with the worktree", and `update-index --assume-unchanged`, a
#      `.git/info/exclude` entry and a committed symlink each part those two questions
#      while porcelain stays silent.
#
#   B. HEAD is contained by some remote branch. `[observed — 2026-08-24, 12e6df95]` this is
#      the one that bites: a cutover ran from a checkout whose migration had been amended
#      in a commit that was never pushed. The tree was CLEAN. Production's checksum then
#      matched no commit reachable from main and every deploy halted at exit 4. A harness
#      that only asserted A would pass while reproducing the incident. The fetch is
#      `--prune`d, because a surviving remote-tracking ref for a deleted upstream branch
#      satisfies `git branch -r --contains` while naming nothing that exists.
#
# The assertion in every refusal case is that the RUNNER IS NOT INVOKED — a refusal that
# still applied the migration would be worse than no guard, and the exit code alone cannot
# tell those apart — AND that the refusal names ITS OWN cause. Without the message grep a
# case passes on any refusal at all, including a missing lib.rs or a failed fetch, which
# makes a green case evidence of nothing.
#
# And on the pass case, the runner's exit code must arrive unchanged: the script `exec`s
# it, because 3 / 4 / anything-else are three different next moves for the operator
# (DEPLOYING.md § the exit table).
#
# The seam is MIGRATE_CMD, the same one scripts/vercel-build.sh:151 exposes and
# test-vercel-build.sh stubs.
#
#   bash .github/scripts/test-migrate-cutover.sh

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
CUTOVER_SH="${REPO_ROOT}/scripts/migrate-cutover.sh"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; shift; printf '    %s\n' "$@"; FAIL=$((FAIL + 1)); }

[ -f "$CUTOVER_SH" ] || { echo "migrate-cutover.sh not found at $CUTOVER_SH" >&2; exit 1; }

# A stand-in for the migration runner. Records that it ran, echoes a sentinel so that
# "printed BEFORE the runner is reached" is an observable ordering rather than a claim,
# and exits with whatever STUB_RC asks for — which is how pass-through of 3 / 4 / 1 is
# observed.
STUB="${WORK}/stub-runner.sh"
cat > "$STUB" <<'STUBSH'
#!/bin/sh
echo "invoked" > "${STUB_SAW}"
echo "STUB-RUNNER-REACHED"
exit "${STUB_RC:-0}"
STUBSH
chmod +x "$STUB"

# mtime in epoch seconds, GNU and BSD `stat` disagreeing about the flag.
mtime_of() { stat -f %m "$1" 2>/dev/null || stat -c %Y "$1"; }

# Throwaway repo + a bare "origin" it can actually fetch from. Nothing here touches the
# real repository: the script resolves its root with `git rev-parse --show-toplevel`, so
# running it with CWD inside the fixture is what points it at the fixture.
#
# `crates/temper-migrate/src/lib.rs` is created because the script refuses without it —
# it is the file whose mtime forces `sqlx::migrate!` to re-read migrations/.
make_repo() {
  rm -rf "${WORK}/origin.git" "${WORK}/repo"
  git init --quiet --bare "${WORK}/origin.git"
  git init --quiet -b main "${WORK}/repo"
  git -C "${WORK}/repo" config user.email harness@example.invalid
  git -C "${WORK}/repo" config user.name  harness
  git -C "${WORK}/repo" remote add origin "${WORK}/origin.git"
  mkdir -p "${WORK}/repo/migrations" "${WORK}/repo/crates/temper-migrate/src"
  echo "SELECT 1;" > "${WORK}/repo/migrations/20260101000010_first.sql"
  echo "// migrate" > "${WORK}/repo/crates/temper-migrate/src/lib.rs"
  git -C "${WORK}/repo" add -A
  git -C "${WORK}/repo" commit --quiet -m "base"
}

push_repo() { git -C "${WORK}/repo" push --quiet origin main; }

# Every variable the script reads is cleared first, so a leaked value from the developer's
# own shell cannot decide a verdict. DATABASE_URL is then supplied deliberately, because
# the script now refuses without one — and the password in it is a fixture on purpose: one
# case asserts it never reaches the output.
#
# `$@` is environment for the script; CUT_ARGS is its command line. They cannot share one
# list, because `env` would read a leading `--flag` as the command to run.
CUT_ARGS=""
CUT_DB_URL="postgresql://cutover_user:s3cr3t-never-print-me@db.example.invalid:5432/cutover_target"
run_cutover() {
  # shellcheck disable=SC2086
  ( cd "${WORK}/repo" && env -u MIGRATE_CMD -u DATABASE_URL \
      STUB_SAW="${WORK}/saw" MIGRATE_CMD="$STUB" \
      ${CUT_DB_URL:+DATABASE_URL="$CUT_DB_URL"} \
      "$@" bash "$CUTOVER_SH" $CUT_ARGS 2>&1 )
}

saw() { cat "${WORK}/saw" 2>/dev/null || echo "<not invoked>"; }
reset_saw() { rm -f "${WORK}/saw"; }

# First 1-based line number in "$1" matching the fixed string "$2"; empty if absent.
line_of() { printf '%s\n' "$1" | grep -nF -- "$2" | head -1 | cut -d: -f1; }

echo "test-migrate-cutover"
echo

# ── 1. The stub is live ─────────────────────────────────────────────────────────────────
# A stub that never ran would satisfy every refusal case silently and make them all a lie.
make_repo; push_repo; reset_saw
out="$(run_cutover)"; rc=$?
if [ "$rc" -eq 0 ] && [ "$(saw)" = "invoked" ]; then
  ok "sanity: a clean, pushed tree invokes the runner"
else bad "sanity: a clean, pushed tree invokes the runner" "exit=$rc" "saw=$(saw)" "$out"; fi

# ── 2. Evidence is printed BEFORE the runner is reached ─────────────────────────────────
# The version and its SHA-384 are what the operator compares against
# `SELECT version, encode(checksum, 'hex') FROM _sqlx_migrations` afterwards. Asserted on
# the real digest, not on the label: a heading with no hash under it would pass a grep for
# "sha-384" and tell the operator nothing.
#
# "Before" is asserted as an ordering, against the stub's sentinel. Evidence printed AFTER
# the apply would be evidence about something already irreversible, and a case that only
# greps the output cannot see the difference.
make_repo; push_repo; reset_saw
EXPECT="$(shasum -a 384 "${WORK}/repo/migrations/20260101000010_first.sql" 2>/dev/null | cut -d' ' -f1)"
[ -n "$EXPECT" ] || EXPECT="$(sha384sum "${WORK}/repo/migrations/20260101000010_first.sql" | cut -d' ' -f1)"
out="$(run_cutover)"; rc=$?
d_line="$(line_of "$out" "$EXPECT")"
s_line="$(line_of "$out" "STUB-RUNNER-REACHED")"
if [ "$rc" -eq 0 ] \
   && printf '%s' "$out" | grep -q '20260101000010' \
   && [ -n "$d_line" ] && [ -n "$s_line" ] && [ "$d_line" -lt "$s_line" ]; then
  ok "the version and its real SHA-384 are printed before the runner is reached"
else bad "the version and its real SHA-384 are printed before the runner is reached" \
     "exit=$rc" "expected=$EXPECT" "digest@=${d_line:-<absent>}" "stub@=${s_line:-<absent>}" "$out"; fi

# ── 2b. DATABASE_URL is required, and its host/db are shown without its password ─────────
# Every other check in the script guards the BYTES. Nothing guarded WHERE THEY LAND, and
# the migrator's fallback when DATABASE_URL is unset is the local dev database
# (crates/temper-migrate/src/lib.rs:44-45): drop the `DATABASE_URL=` prefix off the
# documented command and the operator migrates their laptop and is told it worked.
make_repo; push_repo; reset_saw
SAVED_DB_URL="$CUT_DB_URL"; CUT_DB_URL=""
out="$(run_cutover)"; rc=$?
CUT_DB_URL="$SAVED_DB_URL"
if [ "$rc" -ne 0 ] \
   && [ "$(saw)" = "<not invoked>" ] \
   && printf '%s' "$out" | grep -q 'DATABASE_URL is unset or empty'; then
  ok "an unset DATABASE_URL refuses and does not invoke the runner"
else bad "an unset DATABASE_URL refuses and does not invoke the runner" "exit=$rc" "saw=$(saw)" "$out"; fi

# The other half: the target IS shown, so the evidence block says where the hashes land —
# and the password is not, because this output is what an operator pastes into an incident
# channel.
make_repo; push_repo; reset_saw
out="$(run_cutover)"; rc=$?
if [ "$rc" -eq 0 ] \
   && printf '%s' "$out" | grep -q 'db.example.invalid' \
   && printf '%s' "$out" | grep -q 'cutover_target' \
   && ! printf '%s' "$out" | grep -q 's3cr3t-never-print-me'; then
  ok "the target host and database are printed, the password is not"
else bad "the target host and database are printed, the password is not" "exit=$rc" "$out"; fi

# ── 2c. A pooled endpoint warns but is not refused ──────────────────────────────────────
# The sqlx migrator takes a session-level advisory lock and Neon's pooled endpoint is
# PgBouncer in transaction mode (DEPLOYING.md:207-213). It is a warning and not a refusal
# on purpose: the test is the literal `-pooler.` substring Neon puts in the hostname, and a
# substring is not a good enough reason to block a cutover.
make_repo; push_repo; reset_saw
SAVED_DB_URL="$CUT_DB_URL"
CUT_DB_URL="postgresql://cutover_user:s3cr3t-never-print-me@ep-fixture-9999-pooler.c-5.us-east-1.aws.neon.tech:5432/neondb?sslmode=require"
out="$(run_cutover)"; rc=$?
CUT_DB_URL="$SAVED_DB_URL"
if [ "$rc" -eq 0 ] \
   && [ "$(saw)" = "invoked" ] \
   && printf '%s' "$out" | grep -q 'POOLED endpoint'; then
  ok "a pooled endpoint warns and still applies"
else bad "a pooled endpoint warns and still applies" "exit=$rc" "saw=$(saw)" "$out"; fi

# And the unpooled default must NOT warn, or the warning is noise that trains a bypass.
make_repo; push_repo; reset_saw
out="$(run_cutover)"; rc=$?
if [ "$rc" -eq 0 ] && ! printf '%s' "$out" | grep -q 'POOLED endpoint'; then
  ok "an unpooled host does not warn"
else bad "an unpooled host does not warn" "exit=$rc" "$out"; fi

# ── 3. A dirty migrations/ refuses WITHOUT invoking the runner ──────────────────────────
# An UNTRACKED file, deliberately: that is what a new migration is, and it is precisely the
# case `git diff-index` is blind to. A guard written with diff-index would pass this test
# case while applying the file.
make_repo; push_repo; reset_saw
echo "SELECT 2;" > "${WORK}/repo/migrations/20260101000020_untracked.sql"
out="$(run_cutover)"; rc=$?
if [ "$rc" -ne 0 ] \
   && [ "$(saw)" = "<not invoked>" ] \
   && printf '%s' "$out" | grep -q 'not clean'; then
  ok "an untracked migration refuses and does not invoke the runner"
else bad "an untracked migration refuses and does not invoke the runner" "exit=$rc" "saw=$(saw)" "$out"; fi

# ── 3b. A MODIFIED tracked migration refuses too ────────────────────────────────────────
# With its own message grep: without one this case passes on ANY refusal — a missing
# lib.rs, a fetch that failed — and would go green while the guard it names was gone.
make_repo; push_repo; reset_saw
echo "SELECT 99;" > "${WORK}/repo/migrations/20260101000010_first.sql"
out="$(run_cutover)"; rc=$?
if [ "$rc" -ne 0 ] \
   && [ "$(saw)" = "<not invoked>" ] \
   && printf '%s' "$out" | grep -q 'not clean'; then
  ok "an edited migration refuses and does not invoke the runner"
else bad "an edited migration refuses and does not invoke the runner" "exit=$rc" "saw=$(saw)" "$out"; fi

# ── 3c. assume-unchanged: porcelain goes silent, the byte comparison does not ───────────
# `git update-index --assume-unchanged` tells git to stop stat-ing the file. The worktree
# then holds bytes no commit has, `git status --porcelain` prints nothing, and the old
# guard said "migrations/ is clean" and applied them. The fixture asserts that silence
# first, so this case cannot pass by accident on a porcelain that did fire.
make_repo; push_repo; reset_saw
git -C "${WORK}/repo" update-index --assume-unchanged migrations/20260101000010_first.sql
echo "SELECT 12345;" > "${WORK}/repo/migrations/20260101000010_first.sql"
if [ -n "$(git -C "${WORK}/repo" status --porcelain -- migrations/)" ]; then
  bad "fixture precondition: assume-unchanged must leave porcelain silent" \
      "$(git -C "${WORK}/repo" status --porcelain -- migrations/)"
fi
out="$(run_cutover)"; rc=$?
if [ "$rc" -ne 0 ] \
   && [ "$(saw)" = "<not invoked>" ] \
   && printf '%s' "$out" | grep -q 'not what HEAD holds' \
   && printf '%s' "$out" | grep -q '!= HEAD'; then
  ok "an assume-unchanged edit refuses even though porcelain is silent"
else bad "an assume-unchanged edit refuses even though porcelain is silent" "exit=$rc" "saw=$(saw)" "$out"; fi

# ── 3d. An IGNORED migration: invisible to porcelain, present to the evidence glob ──────
# The sharpest version of the same split. `.git/info/exclude` keeps the file out of
# `git status` entirely, while `migrations/*.sql` still matches it — so one run said
# "clean" and then printed the file it had just vouched for.
make_repo; push_repo; reset_saw
echo "migrations/20260101000020_ignored.sql" >> "${WORK}/repo/.git/info/exclude"
echo "SELECT 777;" > "${WORK}/repo/migrations/20260101000020_ignored.sql"
if [ -n "$(git -C "${WORK}/repo" status --porcelain -- migrations/)" ]; then
  bad "fixture precondition: an excluded path must leave porcelain silent" \
      "$(git -C "${WORK}/repo" status --porcelain -- migrations/)"
fi
out="$(run_cutover)"; rc=$?
if [ "$rc" -ne 0 ] \
   && [ "$(saw)" = "<not invoked>" ] \
   && printf '%s' "$out" | grep -q 'in no commit'; then
  ok "an ignored, untracked migration refuses even though porcelain is silent"
else bad "an ignored, untracked migration refuses even though porcelain is silent" "exit=$rc" "saw=$(saw)" "$out"; fi

# ── 3e. A dangling symlink dies instead of silently truncating the evidence ─────────────
# The old loop guarded the empty glob with `[[ -e "$f" ]] || break`, and `-e` follows
# symlinks. A committed symlink whose target does not exist sorts FIRST here, so the loop
# ended before printing a single row while the sentinel that would have caught an empty
# migrations/ was already satisfied: every migration applied, one row of evidence, no
# error. The refusal must name the unreadable path.
make_repo
ln -s /nonexistent/elsewhere.sql "${WORK}/repo/migrations/20260101000005_dangling.sql"
git -C "${WORK}/repo" add -A
git -C "${WORK}/repo" commit --quiet -m "a symlink migration"
push_repo; reset_saw
if [ -n "$(git -C "${WORK}/repo" status --porcelain -- migrations/)" ]; then
  bad "fixture precondition: the symlink case must have a CLEAN tree" \
      "$(git -C "${WORK}/repo" status --porcelain -- migrations/)"
fi
out="$(run_cutover)"; rc=$?
if [ "$rc" -ne 0 ] \
   && [ "$(saw)" = "<not invoked>" ] \
   && printf '%s' "$out" | grep -q 'cannot be read' \
   && printf '%s' "$out" | grep -q '20260101000005_dangling.sql'; then
  ok "a dangling symlink refuses by name instead of truncating the evidence"
else bad "a dangling symlink refuses by name instead of truncating the evidence" "exit=$rc" "saw=$(saw)" "$out"; fi

# ── 4. An UNPUSHED commit refuses, with the tree clean ──────────────────────────────────
# The incident case. `[observed — 2026-08-24, 12e6df95]` Both halves are asserted, because
# only the second one bites: the tree is clean here, so a clean-tree-only guard passes and
# reproduces the outage. The commit exists, `git status` is empty, and the bytes still
# exist nowhere but this machine.
make_repo; push_repo
echo "SELECT 3;" > "${WORK}/repo/migrations/20260101000030_amended.sql"
git -C "${WORK}/repo" add -A
git -C "${WORK}/repo" commit --quiet -m "amended, never pushed"
reset_saw
if [ -n "$(git -C "${WORK}/repo" status --porcelain -- migrations/)" ]; then
  bad "fixture precondition: the unpushed case must have a CLEAN tree" "$(git -C "${WORK}/repo" status --porcelain)"
fi
out="$(run_cutover)"; rc=$?
if [ "$rc" -ne 0 ] \
   && [ "$(saw)" = "<not invoked>" ] \
   && printf '%s' "$out" | grep -q 'no remote branch'; then
  ok "a clean tree on an unpushed commit refuses and does not invoke the runner"
else bad "a clean tree on an unpushed commit refuses and does not invoke the runner" "exit=$rc" "saw=$(saw)" "$out"; fi

# ── 4b. A STALE remote-tracking ref does not count as pushed ────────────────────────────
# `git branch -r --contains HEAD` reads refs/remotes, which is a cache. Push a branch,
# delete it upstream, and without `--prune` the local `origin/<branch>` survives the fetch
# and vouches for bytes that are on no remote — the 12e6df95 shape again, this time with
# the guard printing "HEAD is contained by: origin/jct/stale". The fixture asserts the
# stale ref really is present before running, so the case cannot pass because the setup
# silently failed.
make_repo; push_repo
git -C "${WORK}/repo" checkout --quiet -b jct/stale
echo "SELECT 6;" > "${WORK}/repo/migrations/20260101000060_stale.sql"
git -C "${WORK}/repo" add -A
git -C "${WORK}/repo" commit --quiet -m "pushed, then deleted upstream"
git -C "${WORK}/repo" push --quiet origin jct/stale
git -C "${WORK}/origin.git" update-ref -d refs/heads/jct/stale
reset_saw
if ! git -C "${WORK}/repo" show-ref --quiet --verify refs/remotes/origin/jct/stale; then
  bad "fixture precondition: the stale remote-tracking ref must still exist locally" \
      "$(git -C "${WORK}/repo" branch -r)"
fi
out="$(run_cutover)"; rc=$?
if [ "$rc" -ne 0 ] \
   && [ "$(saw)" = "<not invoked>" ] \
   && printf '%s' "$out" | grep -q 'no remote branch'; then
  ok "a remote-tracking ref for a deleted upstream branch does not count as pushed"
else bad "a remote-tracking ref for a deleted upstream branch does not count as pushed" "exit=$rc" "saw=$(saw)" "$out"; fi

# ── 5. An unmerged but PUSHED branch is allowed ─────────────────────────────────────────
# Reachability, not `HEAD == origin/main`. A cutover legitimately runs from a preview's
# branch (DEPLOYING.md:205 — the first two traps "bit one session, on a preview"). A guard
# that demanded main would refuse the normal case and get switched off.
make_repo; push_repo
git -C "${WORK}/repo" checkout --quiet -b jct/some-cutover
echo "SELECT 4;" > "${WORK}/repo/migrations/20260101000040_branch.sql"
git -C "${WORK}/repo" add -A
git -C "${WORK}/repo" commit --quiet -m "on a branch"
git -C "${WORK}/repo" push --quiet origin jct/some-cutover
reset_saw
out="$(run_cutover)"; rc=$?
if [ "$rc" -eq 0 ] && [ "$(saw)" = "invoked" ]; then
  ok "a pushed but unmerged branch is allowed through"
else bad "a pushed but unmerged branch is allowed through" "exit=$rc" "saw=$(saw)" "$out"; fi

# ── 6. The runner's exit code passes through unchanged ──────────────────────────────────
# The script `exec`s the runner. 3 (refusal), 4 (disagreement about history) and 1 (a
# migration genuinely broke) are three different next moves; collapsing them to 1 would
# send an operator to the wrong section of the runbook.
for code in 1 3 4; do
  make_repo; push_repo; reset_saw
  out="$(run_cutover STUB_RC=$code)"; rc=$?
  if [ "$rc" -eq "$code" ] && [ "$(saw)" = "invoked" ]; then
    ok "the runner's exit ${code} passes through unchanged"
  else bad "the runner's exit ${code} passes through unchanged" "exit=$rc" "saw=$(saw)" "$out"; fi
done

# ── 6b. The stale-binary mitigation actually happens ────────────────────────────────────
# `sqlx::migrate!` emits an `include_str!` per file, so EDITING a migration does invalidate
# the build — what escapes is ADDING one, because the macro watches the files it read and
# never the directory. The whole mitigation is one `touch` of the file that carries the
# macro, and it was asserted by nothing: replacing it with `:` left the harness fully
# green. Both halves are checked, because the log line without the syscall is a lie and
# the syscall without the log line is invisible to the operator.
make_repo; push_repo; reset_saw
LIB="${WORK}/repo/crates/temper-migrate/src/lib.rs"
touch -t 200001010000 "$LIB"
before_m="$(mtime_of "$LIB")"
out="$(run_cutover)"; rc=$?
after_m="$(mtime_of "$LIB")"
if [ "$rc" -eq 0 ] \
   && [ "$(saw)" = "invoked" ] \
   && [ "${after_m:-0}" -gt "${before_m:-0}" ] \
   && printf '%s' "$out" | grep -q 'touched crates/temper-migrate/src/lib.rs'; then
  ok "the sqlx::migrate! carrier is touched so a new migration cannot be missed"
else bad "the sqlx::migrate! carrier is touched so a new migration cannot be missed" \
     "exit=$rc" "saw=$(saw)" "mtime ${before_m:-?} -> ${after_m:-?}" "$out"; fi

# ── 7. The escape hatch exists and cannot be typed by accident ──────────────────────────
# It downgrades the refusal rather than skipping the check, so the evidence still prints.
# Asserted on the hatch's OWN warning text, not on a bare `[warn]`: other warnings exist
# now (a pooled endpoint is one), and a grep for `[warn]` would pass on any of them.
#
# The second half is the real assertion: any OTHER argument must be rejected, so a
# half-remembered flag fails loudly instead of quietly meaning nothing.
make_repo; push_repo
echo "SELECT 5;" > "${WORK}/repo/migrations/20260101000050_untracked.sql"
reset_saw
CUT_ARGS="--yes-apply-bytes-that-are-not-in-a-pushed-commit"
out="$(run_cutover)"; rc=$?
if [ "$rc" -eq 0 ] && [ "$(saw)" = "invoked" ] \
   && printf '%s' "$out" | grep -q 'ignored by --yes-apply-bytes-that-are-not-in-a-pushed-commit'; then
  ok "the override applies anyway, and warns"
else bad "the override applies anyway, and warns" "exit=$rc" "saw=$(saw)" "$out"; fi

reset_saw
CUT_ARGS="--force"
out="$(run_cutover)"; rc=$?
if [ "$rc" -ne 0 ] && [ "$(saw)" = "<not invoked>" ] && printf '%s' "$out" | grep -q 'Unknown argument'; then
  ok "an unrecognised flag is rejected, not silently ignored"
else bad "an unrecognised flag is rejected, not silently ignored" "exit=$rc" "saw=$(saw)" "$out"; fi
CUT_ARGS=""

# ── 8. DEPLOYING.md's cutover FENCE invokes this script ─────────────────────────────────
# Every assertion above is about a script that only matters if the runbook tells the
# operator to run it — the incident happened because the runbook had no guard at all.
#
# Asserted inside the fenced block under the cutover heading, not against the whole file.
# A grep over all of DEPLOYING.md passes on the prose bullet that merely MENTIONS the
# script, so reverting the fence to the old bare `cargo run … temper-migrate` — the exact
# regression that reintroduces the incident — left this case green.
CUTOVER_FENCE="$(awk '
  /^### Cutover command/ { insec = 1 }
  insec && /^```/        { nf++; if (nf == 1) { inb = 1; next } else { exit } }
  insec && inb           { print }
' "${REPO_ROOT}/DEPLOYING.md")"

if [ -n "$CUTOVER_FENCE" ] \
   && printf '%s' "$CUTOVER_FENCE" | grep -q 'scripts/migrate-cutover.sh' \
   && ! printf '%s' "$CUTOVER_FENCE" | grep -qE 'cargo run.*temper-migrate'; then
  ok "DEPLOYING.md's cutover fence runs the script, not the bare binary"
else bad "DEPLOYING.md's cutover fence runs the script, not the bare binary" \
     "fence was:" "${CUTOVER_FENCE:-(no fenced block under '### Cutover command')}"; fi

# ── 9. The runbook does not oversell the guard ──────────────────────────────────────────
# The script cannot check authenticity, cannot know what a prebuilt binary embeds, and
# never reads the database. It now prints those three limits before it applies anything;
# the runbook must not contradict that by describing the guard as total.
if grep -q 'What it does not check' "${REPO_ROOT}/DEPLOYING.md"; then
  ok "DEPLOYING.md names what the guard does not check"
else bad "DEPLOYING.md names what the guard does not check" \
     "$(grep -n 'migrate-cutover.sh' "${REPO_ROOT}/DEPLOYING.md" || echo '(script not mentioned at all)')"; fi

echo
echo "  ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
