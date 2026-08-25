#!/usr/bin/env bash
# scripts/migrate-cutover.sh
#
# Apply a shape-breaking migration as an operator-gated cutover.
# See DEPLOYING.md § "Cutover command — what to actually run".
#
# Usage:
#   DATABASE_URL='<unpooled neondb_owner URI>' ./scripts/migrate-cutover.sh
#   DATABASE_URL='…' ./scripts/migrate-cutover.sh --yes-apply-bytes-that-are-not-in-a-pushed-commit
#
# WHY THIS SCRIPT EXISTS
#
#   The migrator applies the files in `migrations/` **as they sit on disk**. It has no
#   idea which commit they came from, or whether they came from a commit at all. sqlx
#   then records the SHA-384 of each applied file into `_sqlx_migrations.checksum`
#   (`sqlx-postgres/src/migrate.rs` CREATE TABLE `_sqlx_migrations`), permanently. So
#   whatever bytes are on disk at cutover time BECOME production's history.
#
#   `[observed — 2026-08-24, 12e6df95]` a cutover was run from a local checkout whose
#   migration had been amended in a commit that was **never pushed**. Production's
#   checksum then matched no commit reachable from `main`, and every deploy afterwards
#   halted at exit 4. Resetting the Neon preview did not help — previews branch from
#   production and inherit the divergence. The file had to be corrected to match the
#   database, because a migration already applied may never be amended. The operator
#   followed DEPLOYING.md exactly; the runbook simply had no guard.
#
#   Hence three conjuncts, and **only B bites on that incident**: the working tree WAS
#   clean. A clean-tree check alone would have waved it straight through.
#
#     0. DATABASE_URL names the target. Everything else here guards the BYTES; nothing
#        else guards WHERE THEY LAND, and the migrator's fallback is the local dev
#        database (`crates/temper-migrate/src/lib.rs:44-45`).
#     A. Every `migrations/*.sql` on disk is byte-identical to the blob in HEAD.
#     B. HEAD is contained by some remote branch — the bytes exist somewhere other than
#        this machine.
#
#   B is deliberately NOT `HEAD == origin/main`. A cutover legitimately runs from an
#   unmerged branch: DEPLOYING.md:205 records three traps, the first two of which bit
#   "one session, on a preview", and a preview is a per-PR deployment. DEPLOYING.md:160
#   makes the same point from the other side — "A merge is not a deploy." Reachability is
#   the property that matters; being on `main` is not.
#
# WHAT THIS SCRIPT DOES *NOT* COVER — the stale binary
#
#   `cargo run --release` reuses an existing artifact. EDITING a migration does invalidate
#   it: `sqlx::migrate!` emits an `include_str!` per file — `sqlx-macros-core-0.8.6`'s
#   migrate.rs:62-63 says so in a comment, *"this tells the compiler to watch this path for
#   changes"* — so rustc records every migration as a dependency, and this repo's own
#   `target/release/deps/temper_migrate-*.d` lists all 195 of them twice.
#
#   What escapes is **adding a new file**. The macro watches only the paths it read, never
#   the directory, so a set that gained a migration since the last build runs silently
#   short. (`internal/superpowers/plans/2026-08-21-data-artifact-shape-registry.md:45-47`
#   prescribes the `touch` below as the mitigation, though it states the cause too broadly
#   as "editing"; the dep file above is the counter-evidence.)
#
#   The `touch` closes that for the DEFAULT command only. It does not close it if
#   MIGRATE_CMD is overridden to a prebuilt binary — nothing rebuilds then, and this
#   script cannot tell which migrations that binary embeds. That case stays uncovered, and
#   the closing block says so to the operator's face.

set -euo pipefail

# Same shapes as tools/scripts/release/lib/common.sh, defined locally so an operator can
# run this script on its own without the release tooling's lib on hand.
log_info()    { echo "  [info] $*"; }
log_warn()    { echo "  [warn] $*" >&2; }
log_error()   { echo "  [error] $*" >&2; }
log_header()  { echo ""; echo "== $* =="; echo ""; }
log_section() { echo ""; echo "-- $* --"; }

die() { log_error "$*"; exit 1; }

# ---------------------------------------------------------------------------
# The hasher
# ---------------------------------------------------------------------------
# Defined before the checks because Check A compares hashes, not just the evidence table.
# sqlx hashes the whole file, untrimmed (`sqlx-core/src/migrate/source.rs` reads it with
# `fs::read_to_string`, and `Migration::new` digests `sql.as_bytes()`), so a plain SHA-384
# of the file reproduces `_sqlx_migrations.checksum` exactly.
if command -v shasum >/dev/null 2>&1; then
    hash_file()  { shasum -a 384 "$1" | cut -d' ' -f1; }
    hash_stdin() { shasum -a 384 - | cut -d' ' -f1; }
elif command -v sha384sum >/dev/null 2>&1; then
    hash_file()  { sha384sum "$1" | cut -d' ' -f1; }
    hash_stdin() { sha384sum - | cut -d' ' -f1; }
elif command -v openssl >/dev/null 2>&1; then
    hash_file()  { openssl dgst -sha384 "$1" | awk '{ print $NF }'; }
    hash_stdin() { openssl dgst -sha384 | awk '{ print $NF }'; }
else
    die "No SHA-384 tool found (tried shasum, sha384sum, openssl)."
fi

# ---------------------------------------------------------------------------
# Parse arguments
# ---------------------------------------------------------------------------
# One escape hatch, in release-prepare.sh's idiom: it downgrades a `die` to a `log_warn`
# rather than skipping the check, so the evidence is still printed. Named at length
# because it must never be reachable by a reflex or a shell-history stab.
OVERRIDE=false

while [[ $# -gt 0 ]]; do
    case $1 in
        --yes-apply-bytes-that-are-not-in-a-pushed-commit) OVERRIDE=true; shift ;;
        *) die "Unknown argument: $1" ;;
    esac
done

log_header "Temper Shape-Breaking Cutover"

# The migrator reads `migrations/` relative to the repo root, so run from there whatever
# directory the operator invoked us from.
REPO_ROOT="$(git rev-parse --show-toplevel)" || die "Not inside a git repository."
cd "$REPO_ROOT"

[[ -d migrations ]] || die "No migrations/ directory at ${REPO_ROOT}."

# ---------------------------------------------------------------------------
# Pre-flight checks
# ---------------------------------------------------------------------------
log_section "Pre-flight checks"

# Check 0 — the target.
#
# `connect()` falls back to `postgresql://temper:temper@localhost:5437/temper_development`
# when DATABASE_URL is unset (crates/temper-migrate/src/lib.rs:44-45). Drop the
# `DATABASE_URL=` prefix off the documented command line and this script's every other
# check still passes, the migrator still prints success, and what got migrated is the
# operator's laptop. This repo makes that likelier than it sounds: `.env` sets the dev
# database and `.env.local` sets the POOLED Neon endpoint, which is DEPLOYING.md's own
# trap. No fallback here — an unset target is a refusal.
[[ -n "${DATABASE_URL:-}" ]] || die "DATABASE_URL is unset or empty. temper-migrate would fall back to the local dev database (crates/temper-migrate/src/lib.rs:44-45), apply everything there, and report success. Set it to the unpooled neondb_owner URI — DEPLOYING.md § 'Cutover command — what to actually run'."

# Host and database only. The password is never read out of the URI, let alone printed:
# the authority is everything before the first `/`, and credentials are everything up to
# the last `@` inside it.
TARGET_REST="${DATABASE_URL#*://}"
TARGET_AUTHORITY="${TARGET_REST%%/*}"
TARGET_HOST="${TARGET_AUTHORITY##*@}"
TARGET_DB="${TARGET_REST#"$TARGET_AUTHORITY"}"
TARGET_DB="${TARGET_DB#/}"
TARGET_DB="${TARGET_DB%%\?*}"
log_info "target: ${TARGET_HOST:-<no host>} db=${TARGET_DB:-<no database>}"

# Neon's pooled endpoint is the same hostname with `-pooler` appended to the endpoint id.
# That literal substring is the entire test — no heuristic about what a pooler looks like
# in general — so it cannot fire on `localhost` or on the unpooled Neon host. A warning,
# never a refusal: the sqlx migrator takes a session-level advisory lock and the pooled
# endpoint is PgBouncer in transaction mode (DEPLOYING.md:207-213).
case "$TARGET_HOST" in
    *-pooler.*)
        log_warn "${TARGET_HOST} looks like Neon's POOLED endpoint. The sqlx migrator takes a"
        log_warn "session-level advisory lock and the pooled endpoint is PgBouncer in transaction"
        log_warn "mode — use DATABASE_URL_UNPOOLED's host instead (DEPLOYING.md:207-213)."
        ;;
esac

# The migration set, collected once and reused by Check A and the evidence table.
#
# `nullglob` rather than the old `[[ -e "$f" ]] || break` guard for an unmatched glob: `-e`
# follows symlinks, so ONE dangling entry ended the loop early, and the emptiness sentinel
# was already true by then — four migrations would be applied and one row printed.
shopt -s nullglob
MIGRATION_FILES=(migrations/*.sql)
shopt -u nullglob
(( ${#MIGRATION_FILES[@]} )) || die "migrations/ contains no .sql files — is ${REPO_ROOT} the right repo?"

# Check A — the bytes on disk are the bytes in HEAD.
#
# `git status --porcelain` answers "does the index disagree with the worktree", which is
# not the question. Three ways of parting the two need nothing exotic: `git update-index
# --assume-unchanged` on a migration then editing it; a path excluded through
# `.git/info/exclude`; a committed symlink whose target is mutated off-tree. In each,
# `--porcelain` is silent while what the migrator will read is not what any commit holds.
#
# The porcelain check stays — it is the clearest message for the ordinary case, because it
# names the offending paths in git's own vocabulary. The authoritative conjunct underneath
# it is a byte comparison: HEAD's `migrations/*.sql` set must equal the on-disk set, and
# every member must hash equal to `HEAD:`'s blob. The script already computed one side of
# that comparison for the evidence table and never looked at the other.
DIRTY="$(git status --porcelain -- migrations/)"
if [[ -n "$DIRTY" ]]; then
    if [[ "$OVERRIDE" == "true" ]]; then
        log_warn "migrations/ is not clean (ignored by --yes-apply-bytes-that-are-not-in-a-pushed-commit)"
        printf '%s\n' "$DIRTY" >&2
    else
        log_error "migrations/ is not clean:"
        printf '%s\n' "$DIRTY" >&2
        die "Whatever is on disk becomes production's permanent checksum. Commit and push first."
    fi
else
    log_info "migrations/ is clean"
fi

# HEAD's own view of the set, not the index's: the index is a third thing that can agree
# with neither. `|| true` because grep exits 1 on an empty match and this is a pipeline
# under `pipefail` — an empty HEAD set is a finding for the comparison below, not a crash.
HEAD_SET="$(git ls-tree -r --name-only HEAD -- migrations/ 2>/dev/null | grep -E '^migrations/[^/]+\.sql$' | LC_ALL=C sort || true)"
DISK_SET="$(printf '%s\n' "${MIGRATION_FILES[@]}" | LC_ALL=C sort)"

DIVERGED_BYTES=""
if [[ "$HEAD_SET" != "$DISK_SET" ]]; then
    while IFS= read -r f; do
        [[ -n "$f" ]] && DIVERGED_BYTES+="  ${f} — on disk, in no commit (untracked, or ignored)"$'\n'
    done < <(comm -13 <(printf '%s\n' "$HEAD_SET") <(printf '%s\n' "$DISK_SET"))
    while IFS= read -r f; do
        [[ -n "$f" ]] && DIVERGED_BYTES+="  ${f} — in HEAD, missing from disk (the migrator will not apply it)"$'\n'
    done < <(comm -23 <(printf '%s\n' "$HEAD_SET") <(printf '%s\n' "$DISK_SET"))
fi

# Hash each file once: this loop feeds both Check A and the evidence table.
MIGRATION_HASHES=()
for f in "${MIGRATION_FILES[@]}"; do
    # `-r`, not `-e`: a dangling symlink and an unreadable file both fail it, and either
    # one means this script cannot say what the migrator would read there.
    [[ -r "$f" ]] || die "${f} is in migrations/ but cannot be read — a dangling symlink, or no permission. This script cannot say what bytes the migrator would find there, and those bytes become production's permanent checksum."
    disk_hash="$(hash_file "$f")"
    MIGRATION_HASHES+=("$disk_hash")

    if ! git cat-file -e "HEAD:${f}" 2>/dev/null; then
        continue  # already reported by the set comparison above
    fi
    head_hash="$(git cat-file -p "HEAD:${f}" | hash_stdin)"
    if [[ "$disk_hash" != "$head_hash" ]]; then
        DIVERGED_BYTES+="  ${f} — disk ${disk_hash} != HEAD ${head_hash}"$'\n'
    fi
done

if [[ -n "$DIVERGED_BYTES" ]]; then
    if [[ "$OVERRIDE" == "true" ]]; then
        log_warn "migrations/ on disk is not what HEAD holds (ignored by --yes-apply-bytes-that-are-not-in-a-pushed-commit)"
        printf '%s' "$DIVERGED_BYTES" >&2
    else
        log_error "migrations/ on disk is not what HEAD holds:"
        printf '%s' "$DIVERGED_BYTES" >&2
        die "The migrator applies the bytes on disk, and sqlx records THEIR checksum forever. Commit and push the bytes you intend to apply."
    fi
else
    log_info "all ${#MIGRATION_FILES[@]} migrations are byte-identical to HEAD"
fi

# Check B — reachability.
#
# The one that bites on 12e6df95. Not `HEAD == origin/main`: a cutover legitimately runs
# from an unmerged branch (DEPLOYING.md:205, a preview). What must be true is that these
# bytes exist somewhere other than this laptop.
#
# `--prune`, and it is load-bearing. Without it a remote-tracking ref for a branch that was
# deleted or force-pushed upstream survives locally, and `git branch -r --contains HEAD`
# then reports containment against a ref that names no remote branch — which is the 12e6df95
# shape exactly, wearing a green check.
git fetch --quiet --prune origin || die "git fetch origin failed — cannot establish whether HEAD is pushed."

HEAD_SHA="$(git rev-parse HEAD)"
BRANCH="$(git branch --show-current)"
REMOTE_BRANCHES="$(git branch -r --contains HEAD 2>/dev/null || true)"

if [[ -z "$REMOTE_BRANCHES" ]]; then
    if [[ "$OVERRIDE" == "true" ]]; then
        log_warn "HEAD (${HEAD_SHA}) is on no remote branch (ignored by --yes-apply-bytes-that-are-not-in-a-pushed-commit)"
    else
        log_error "HEAD (${HEAD_SHA}) is contained by no remote branch."
        log_error "These migration bytes exist only on this machine. Applying them writes a"
        log_error "checksum into _sqlx_migrations that matches no pushed commit, and every"
        log_error "deploy afterwards halts at exit 4 — see the playbook § 4d."
        die "Push this commit first."
    fi
else
    log_info "HEAD is contained by:$(printf '%s' "$REMOTE_BRANCHES" | tr -s ' \n' ' ')"
fi

# ---------------------------------------------------------------------------
# Evidence — printed BEFORE anything is applied
# ---------------------------------------------------------------------------
# These are the numbers to compare against `SELECT version, encode(checksum, 'hex') FROM
# _sqlx_migrations` afterwards, and the target is on the same block because a table of
# correct hashes says nothing about which database received them.
log_section "What is about to be applied"

echo ""
echo "  HEAD:   ${HEAD_SHA}"
echo "  branch: ${BRANCH:-<detached>}"
echo "  target: ${TARGET_HOST:-<no host>} db=${TARGET_DB:-<no database>}"
echo ""
echo "  version         sha-384 (what _sqlx_migrations will record)"

for i in "${!MIGRATION_FILES[@]}"; do
    base="${MIGRATION_FILES[$i]##*/}"
    printf '  %-15s %s\n' "${base%%_*}" "${MIGRATION_HASHES[$i]}"
done
echo ""

# ---------------------------------------------------------------------------
# Force the macro to re-expand
# ---------------------------------------------------------------------------
# `sqlx::migrate!` lives in crates/temper-migrate/src/lib.rs:33 and nowhere else — the
# other crates only re-export MIGRATOR, so touching one of THOSE rebuilds nothing that
# reads migrations/. Touching mtime only; the content is unchanged, so this does not
# dirty the tree.
MIGRATE_LIB="crates/temper-migrate/src/lib.rs"
[[ -f "$MIGRATE_LIB" ]] || die "${MIGRATE_LIB} is missing — the sqlx::migrate! declaration moved, and this script can no longer force it to re-read migrations/."
touch "$MIGRATE_LIB"
log_info "touched ${MIGRATE_LIB} so sqlx::migrate! re-reads migrations/"

# ---------------------------------------------------------------------------
# What the pre-flight did NOT establish
# ---------------------------------------------------------------------------
# Three real limits lived only in the comments above this line, where the operator running
# the script never meets them. Kept to a few lines on purpose: githooks/pre-commit:5-9 is
# this repo's own note that output nobody can read is what trains a bypass.
log_section "Not checked — still yours"

echo ""
echo "  * Authenticity. The checks above prove these bytes are in a commit that left this"
echo "    machine. They prove nothing about whether anyone reviewed them."
echo "  * The binary, if MIGRATE_CMD was overridden to a prebuilt one. Nothing rebuilds"
echo "    then, and which migrations it embeds is not knowable from here."
echo "  * The database. Nothing here has read it. AFTERWARDS, run"
echo "      SELECT version, encode(checksum, 'hex') FROM _sqlx_migrations ORDER BY version;"
echo "    and compare it against the table above. That comparison is the only thing that"
echo "    shows the bytes printed here are the bytes production recorded."
echo ""

# ---------------------------------------------------------------------------
# Apply
# ---------------------------------------------------------------------------
# No `--additive-only`: that flag is precisely what halts at a shape-breaking migration,
# and taking it is the whole point of a cutover (DEPLOYING.md:192-198).
#
# MIGRATE_CMD is the same seam scripts/vercel-build.sh:151 uses, so the guard test can
# stub the runner the same way. It is never set in a real cutover.
: "${MIGRATE_CMD:=cargo run --release --locked -p temper-migrate --bin temper-migrate}"

log_section "Applying"

# `exec` on purpose: the runner's exit codes are the operator's whole diagnosis (3 refusal,
# 4 disagreement about history, anything else a genuine failure — DEPLOYING.md:239-245) and
# exec hands them back untouched. This script must never reinterpret them; vercel-build.sh
# already owns that translation for the build path.
# shellcheck disable=SC2086
exec $MIGRATE_CMD
