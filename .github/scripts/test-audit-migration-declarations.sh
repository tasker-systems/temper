#!/usr/bin/env bash
# .github/scripts/test-audit-migration-declarations.sh
#
# Test harness for audit-migration-declarations.sh — the checker enforcing spec §1's
# "silence is a failure" half: a migration that declares no classification fails CI.
#
# WHAT IS ACTUALLY UNDER TEST
# ---------------------------
# The rule is NOT "every file contains exactly one declaration". The backfill migration
# `20260731000020` declares 148 versions in one file, so a per-file-exactly-once rule would fail
# the repo on the day it shipped. The rule is set-shaped instead:
#
#   A. every migration version is declared by SOME migration file,
#   B. every declared version corresponds to a migration file that exists,
#   C. every class token is one of the two, and every reason is non-empty.
#
# A+B together already catch the copy-paste this was worried about: paste
# `declare_migration(<some other version>, …)` into a new migration and that version is now
# declared twice (B is satisfied, but the DB's primary key rejects it on apply) while the new
# migration's own version is declared nowhere — so A fails. No separate filename-match rule needed,
# and no legacy exemption to carve out, because the backfill covers the pre-declaration 148.
#
# Two discriminations carry history rather than theory:
#
#   * A DECLARATION SPANNING LINES MUST COUNT. Every declaration in this repo is written across
#     three or four lines, because the reason is a sentence. A line-oriented grep passes its own
#     unit tests and then reports every real migration as undeclared.
#
#   * A DECLARATION INSIDE A COMMENT MUST NOT COUNT. Migration headers in this repo are long and
#     quote SQL freely; `DEPLOYING.md`'s worked example is a `declare_migration` call. A checker
#     that counts commented text lets a migration satisfy the rule by talking about it.
#
#   bash .github/scripts/test-audit-migration-declarations.sh

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT_SCRIPT="${SCRIPT_DIR}/audit-migration-declarations.sh"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; shift; printf '    %s\n' "$@"; FAIL=$((FAIL + 1)); }

# Run the checker against a fixture migrations directory. Echoes output; returns its exit code.
run_audit() {
  MIGRATIONS_DIR="$1" bash "$AUDIT_SCRIPT" 2>&1
}

# A fixture holding one well-formed, multi-line declaration — the house style.
new_fixture() {
  local d="$1"
  rm -rf "$d"
  mkdir -p "$d"
  cat > "$d/20260801000001_add_thing.sql" <<'SQL'
-- A new table nothing reads yet.
CREATE TABLE kb_thing (id uuid PRIMARY KEY);

SELECT declare_migration(
    20260801000001,
    'additive',
    'One new table. Nothing pre-existing is altered or dropped.'
);
SQL
}

echo "test-audit-migration-declarations"
echo

# ── 1. The boring case: a valid, multi-line declaration passes ──────────────────────────────────
D="${WORK}/valid"; new_fixture "$D"
out="$(run_audit "$D")"; rc=$?
if [ "$rc" -eq 0 ]; then ok "a valid multi-line declaration passes"
else bad "a valid multi-line declaration passes" "exit=$rc" "$out"; fi

# ── 2. Silence fails — the whole point of spec §1 ───────────────────────────────────────────────
D="${WORK}/silent"; new_fixture "$D"
cat > "$D/20260801000002_undeclared.sql" <<'SQL'
CREATE TABLE kb_other (id uuid PRIMARY KEY);
SQL
out="$(run_audit "$D")"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q '20260801000002'; then
  ok "a migration with no declaration fails, naming it"
else bad "a migration with no declaration fails, naming it" "exit=$rc" "$out"; fi

# ── 3. A declaration naming a version that has no migration file fails ──────────────────────────
D="${WORK}/ghost"; new_fixture "$D"
cat > "$D/20260801000003_typo.sql" <<'SQL'
SELECT declare_migration(
    20260801000003,
    'additive',
    'Fine.'
);
SELECT declare_migration(
    29999999999999,
    'additive',
    'This version has no migration file.'
);
SQL
out="$(run_audit "$D")"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q '29999999999999'; then
  ok "a declaration for a nonexistent version fails, naming it"
else bad "a declaration for a nonexistent version fails, naming it" "exit=$rc" "$out"; fi

# ── 4. The copy-paste: declaring someone else's version leaves your own undeclared ──────────────
D="${WORK}/pasted"; new_fixture "$D"
cat > "$D/20260801000004_pasted.sql" <<'SQL'
-- Copy-pasted from the migration above and the version never updated.
SELECT declare_migration(
    20260801000001,
    'additive',
    'Wrong version argument.'
);
SQL
out="$(run_audit "$D")"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q '20260801000004'; then
  ok "a copy-pasted version leaves the file itself undeclared, and fails"
else bad "a copy-pasted version leaves the file itself undeclared, and fails" "exit=$rc" "$out"; fi

# ── 5. An unknown class token fails ─────────────────────────────────────────────────────────────
D="${WORK}/badclass"; new_fixture "$D"
cat > "$D/20260801000005_badclass.sql" <<'SQL'
SELECT declare_migration(
    20260801000005,
    'destructive',
    'destructive is not one of the two classes.'
);
SQL
out="$(run_audit "$D")"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'destructive'; then
  ok "an unknown class token fails, naming it"
else bad "an unknown class token fails, naming it" "exit=$rc" "$out"; fi

# ── 6. An empty reason fails — a verdict without its argument ───────────────────────────────────
D="${WORK}/noreason"; new_fixture "$D"
cat > "$D/20260801000006_noreason.sql" <<'SQL'
SELECT declare_migration(
    20260801000006,
    'additive',
    '   '
);
SQL
out="$(run_audit "$D")"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q '20260801000006'; then
  ok "an empty reason fails, naming the migration"
else bad "an empty reason fails, naming the migration" "exit=$rc" "$out"; fi

# ── 7. A declaration inside a comment does NOT count ────────────────────────────────────────────
D="${WORK}/commented"; new_fixture "$D"
cat > "$D/20260801000007_commented.sql" <<'SQL'
-- Declare it like this:
--
--   SELECT declare_migration(
--       20260801000007,
--       'additive',
--       'An example in a header, not a declaration.'
--   );
--
CREATE TABLE kb_yet_another (id uuid PRIMARY KEY);
SQL
out="$(run_audit "$D")"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q '20260801000007'; then
  ok "a declaration inside a comment does not satisfy the rule"
else bad "a declaration inside a comment does not satisfy the rule" "exit=$rc" "$out"; fi

# ── 8. One file may declare many versions — the backfill's shape ────────────────────────────────
D="${WORK}/backfill"; new_fixture "$D"
cat > "$D/20260801000008_legacy_a.sql" <<'SQL'
CREATE TABLE kb_legacy_a (id uuid PRIMARY KEY);
SQL
cat > "$D/20260801000009_legacy_b.sql" <<'SQL'
CREATE TABLE kb_legacy_b (id uuid PRIMARY KEY);
SQL
cat > "$D/20260801000010_backfill.sql" <<'SQL'
SELECT declare_migration(
    20260801000010,
    'additive',
    'Declares itself and the two that predate the mechanism.'
);
SELECT declare_migration(
    20260801000008,
    'additive',
    'Adds only.'
);
SELECT declare_migration(
    20260801000009,
    'shape-breaking',
    'Dropped a column the deployed binary still selects.'
);
SQL
out="$(run_audit "$D")"; rc=$?
if [ "$rc" -eq 0 ]; then ok "one file may declare many versions (the backfill shape)"
else bad "one file may declare many versions (the backfill shape)" "exit=$rc" "$out"; fi

# ── 9. The function's own DEFINITION is not a declaration ───────────────────────────────────────
# Found by case 10, not by design: `20260731000010` both creates `declare_migration` and COMMENTs
# ON it, so `CREATE FUNCTION declare_migration(p_version bigint, …)` and
# `COMMENT ON FUNCTION declare_migration(bigint, migration_class, text)` were being read as two
# malformed calls. The fixtures were all well-formed callers and could not see it.
D="${WORK}/definition"; new_fixture "$D"
cat > "$D/20260801000011_defines_it.sql" <<'SQL'
CREATE FUNCTION declare_migration(p_version bigint, p_class migration_class, p_reason text)
RETURNS void LANGUAGE sql AS $$
    INSERT INTO kb_migration_classifications (version, class, reason)
    VALUES (p_version, p_class, p_reason);
$$;

COMMENT ON FUNCTION declare_migration(bigint, migration_class, text) IS 'Declare a class.';

SELECT declare_migration(
    20260801000011,
    'additive',
    'Creates the function and declares itself with it.'
);
SQL
out="$(run_audit "$D")"; rc=$?
if [ "$rc" -eq 0 ]; then ok "a CREATE FUNCTION / COMMENT ON of the same name is not a declaration"
else bad "a CREATE FUNCTION / COMMENT ON of the same name is not a declaration" "exit=$rc" "$out"; fi

# ── 10. The four-argument form parses — the backfill's shape ────────────────────────────────────
# `declare_migration` takes an optional `p_recorded_by`, which the backfill passes as 'backfill'
# because the migration being described is not the one writing. A parser that took "the last quoted
# string" as the reason would read 'backfill' as the reason and never notice.
D="${WORK}/fourarg"; new_fixture "$D"
cat > "$D/20260801000012_backfilled.sql" <<'SQL'
SELECT declare_migration(
    20260801000012,
    'additive',
    'A real reason, followed by a provenance argument.',
    'backfill'
);
SQL
out="$(run_audit "$D")"; rc=$?
if [ "$rc" -eq 0 ]; then ok "the four-argument declare_migration form parses"
else bad "the four-argument declare_migration form parses" "exit=$rc" "$out"; fi

# ── 11. A reclassification is validated but does NOT declare ────────────────────────────────────
# It names a migration that already shipped. Letting it satisfy "this version is declared" would let
# a migration declare itself by revising someone else.
D="${WORK}/reclass"; new_fixture "$D"
cat > "$D/20260801000013_only_reclassifies.sql" <<'SQL'
SELECT reclassify_migration(
    20260801000001,
    'shape-breaking',
    'Revised: the earlier additive call missed a required parameter.'
);
SQL
out="$(run_audit "$D")"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q '20260801000013'; then
  ok "a migration that only reclassifies still fails for declaring nothing itself"
else bad "a migration that only reclassifies still fails for declaring nothing itself" "exit=$rc" "$out"; fi

# ── 12. A reclassification with a bad class still fails ─────────────────────────────────────────
D="${WORK}/reclassbad"; new_fixture "$D"
cat > "$D/20260801000014_reclass_bad.sql" <<'SQL'
SELECT declare_migration(
    20260801000014,
    'additive',
    'Declares itself properly.'
);
SELECT reclassify_migration(
    20260801000001,
    'destructive',
    'Not one of the two classes.'
);
SQL
out="$(run_audit "$D")"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'destructive'; then
  ok "a reclassification is validated like a declaration"
else bad "a reclassification is validated like a declaration" "exit=$rc" "$out"; fi

# ── 13. A reclassification naming a nonexistent version fails ───────────────────────────────────
D="${WORK}/reclassghost"; new_fixture "$D"
cat > "$D/20260801000015_reclass_ghost.sql" <<'SQL'
SELECT declare_migration(
    20260801000015,
    'additive',
    'Declares itself properly.'
);
SELECT reclassify_migration(
    29999999999999,
    'additive',
    'No migration file has this version.'
);
SQL
out="$(run_audit "$D")"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q '29999999999999'; then
  ok "a reclassification naming no real migration fails"
else bad "a reclassification naming no real migration fails" "exit=$rc" "$out"; fi

# ── 14. An EMPTY class token is rejected, and the message names the token it rejected ───────────
# The verdict was never in doubt; the message was. `IFS=$'\t' read` collapses a run of tabs, so an
# empty class shifted every field left and the checker reported the REASON as the offending class.
# A guard on the verdict alone would have stayed green through that.
D="${WORK}/emptyclass"; new_fixture "$D"
cat > "$D/20260801000016_empty_class.sql" <<'SQL'
SELECT declare_migration(20260801000016, '', 'A reason, but no class at all.');
SQL
out="$(run_audit "$D")"; rc=$?
if [ "$rc" -ne 0 ] \
   && printf '%s' "$out" | grep -q "declares class ''" \
   && ! printf '%s' "$out" | grep -q "declares class 'A reason"; then
  ok "an empty class token is rejected, naming the empty token and not the reason"
else bad "an empty class token is rejected, naming the empty token and not the reason" "exit=$rc" "$out"; fi

# ── 15. The real repo passes — the check is live, not just fixture-true ─────────────────────────
out="$(bash "$AUDIT_SCRIPT" 2>&1)"; rc=$?
if [ "$rc" -eq 0 ]; then ok "the repo's own migrations/ passes"
else bad "the repo's own migrations/ passes" "exit=$rc" "$out"; fi

echo
echo "  ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
