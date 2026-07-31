#!/usr/bin/env bash
# .github/scripts/test-sqlx-schema-crosscheck.sh
#
# Test harness for sqlx-schema-crosscheck.sh — the asymmetric cross-check that closes
# `a-classification-is-checkable-against-the-migration-it-describes`.
#
# THE ASYMMETRY IS THE DESIGN, NOT AN OVERSIGHT
# ---------------------------------------------
# Spec §2's table, verbatim:
#
#   | wire-diff non-empty, no migration declares shape-breaking | fail, naming the query files
#                                                                 and the migration |
#   | migration declares shape-breaking, wire-diff empty        | pass, noted — a break can be
#                                                                 invisible to the cache; failing
#                                                                 here trains under-declaration |
#   | migration added with no declaration                       | fail |
#
# The middle row is the one a well-meaning implementer "simplifies" into a failure, because a
# declaration with no corroborating evidence looks like a mistake. It is not: the caches only see
# what a checked query touches, so a genuine break can be invisible to them, and failing an
# honest over-declaration teaches people to under-declare. Case 4 is what prevents that
# simplification from ever landing.
#
#   bash .github/scripts/test-sqlx-schema-crosscheck.sh

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
CHECK="${SCRIPT_DIR}/sqlx-schema-crosscheck.sh"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; shift; printf '    %s\n' "$@"; FAIL=$((FAIL + 1)); }

MIGDIR="${WORK}/migrations"

# Reset the fixture migrations dir. It always holds one pre-existing, already-declared migration
# so that "added in this change" is a real distinction rather than "everything present".
reset_migs() {
  rm -rf "$MIGDIR"; mkdir -p "$MIGDIR"
  cat > "${MIGDIR}/20260101000001_pre_existing.sql" <<'SQL'
SELECT declare_migration(
    20260101000001,
    'additive',
    'Already shipped before this change.'
);
SQL
  : > "${WORK}/added.txt"
}

# Add a migration and mark it as added-in-this-change. $1 version, $2 class ('' = declare nothing)
add_mig() {
  local v="$1" cls="${2:-}"
  local f="${MIGDIR}/${v}_added.sql"
  if [ -n "$cls" ]; then
    cat > "$f" <<SQL
SELECT declare_migration(
    ${v},
    '${cls}',
    'Fixture declaration.'
);
SQL
  else
    echo "CREATE TABLE kb_fixture_${v} (id uuid PRIMARY KEY);" > "$f"
  fi
  echo "migrations/${v}_added.sql" >> "${WORK}/added.txt"
}

run_check() {
  MIGRATIONS_DIR="$MIGDIR" bash "$CHECK" \
    --wire-verdict "$1" --added-migrations "${WORK}/added.txt" 2>&1
}

echo "test-sqlx-schema-crosscheck"
echo

# ── 1. The boring case: nothing added, nothing moved ────────────────────────────────────────────
reset_migs
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -eq 0 ]; then ok "no migration, no wire move: passes"
else bad "no migration, no wire move: passes" "exit=$rc" "$out"; fi

# ── 2. ROW 1 — the wire moved and nothing declares shape-breaking: FAIL ─────────────────────────
reset_migs; add_mig 20260801000001 additive
out="$(run_check moved)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q '20260801000001'; then
  ok "wire moved + only additive declared: fails, naming the migration"
else bad "wire moved + only additive declared: fails, naming the migration" "exit=$rc" "$out"; fi

# ── 3. The wire moved and a migration declares shape-breaking: pass ─────────────────────────────
reset_migs; add_mig 20260801000002 shape-breaking
out="$(run_check moved)"; rc=$?
if [ "$rc" -eq 0 ]; then ok "wire moved + shape-breaking declared: passes"
else bad "wire moved + shape-breaking declared: passes" "exit=$rc" "$out"; fi

# ── 4. ROW 2 — declared shape-breaking, wire-diff EMPTY: pass, NOTED ────────────────────────────
# The row most likely to be "simplified" into a failure. A break can be invisible to the caches,
# and failing an honest over-declaration trains under-declaration. It must pass, and it must say
# something.
reset_migs; add_mig 20260801000003 shape-breaking
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -qi 'noted'; then
  ok "declared shape-breaking + empty wire diff: passes AND is noted"
else bad "declared shape-breaking + empty wire diff: passes AND is noted" "exit=$rc" "$out"; fi

# ── 5. ROW 3 — a migration added with no declaration: FAIL ──────────────────────────────────────
# Delegated to audit-migration-declarations.sh rather than re-implemented, so the rule has exactly
# one spelling. The cross-check still has to FAIL on it, which is what this asserts.
reset_migs; add_mig 20260801000004 ''
out="$(run_check unchanged)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q '20260801000004'; then
  ok "a migration that declares nothing fails the cross-check too"
else bad "a migration that declares nothing fails the cross-check too" "exit=$rc" "$out"; fi

# ── 6. The row spec §2 did not write down: the wire moved with NO migration at all ──────────────
# Not in the table, and a silent pass would be a hole. The caches say the schema returns something
# different while nothing in this change altered the schema — which is either a cache regenerated
# against a database the repo does not describe, or a migration applied out of band. Both are the
# drift this goal exists to catch, so it fails and says which.
reset_migs
out="$(run_check moved)"; rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -qi 'no migration in this change'; then
  ok "wire moved with no migration at all: fails, and says why"
else bad "wire moved with no migration at all: fails, and says why" "exit=$rc" "$out"; fi

# ── 7. An undeterminable wire diff never renders as a pass when a migration is present ──────────
# The canary's lesson: a safeguard that derives its input from the healthy state cannot bootstrap
# from the unhealthy one. "I could not look" is not "nothing moved".
reset_migs; add_mig 20260801000005 additive
out="$(run_check undeterminable)"; rc=$?
# Asserting only `rc != 0` would be satisfied by the script not existing at all — a green for the
# wrong reason, which this harness caught on its own first run. The message has to be there too.
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -qi 'undeterminable'; then
  ok "undeterminable wire diff + a migration present: fails, never passes"
else bad "undeterminable wire diff + a migration present: fails, never passes" "exit=$rc" "$out"; fi

# ── 8. ...but with no migration present there is nothing to cross-check ─────────────────────────
reset_migs
out="$(run_check undeterminable)"; rc=$?
if [ "$rc" -eq 0 ] && printf '%s' "$out" | grep -qi 'undeterminable'; then
  ok "undeterminable wire diff + no migration: passes, loudly"
else bad "undeterminable wire diff + no migration: passes, loudly" "exit=$rc" "$out"; fi

# ── 9. Several added migrations, one of them shape-breaking, satisfies row 1 ────────────────────
reset_migs; add_mig 20260801000006 additive; add_mig 20260801000007 shape-breaking
out="$(run_check moved)"; rc=$?
if [ "$rc" -eq 0 ]; then ok "one shape-breaking among several added migrations satisfies the move"
else bad "one shape-breaking among several added migrations satisfies the move" "exit=$rc" "$out"; fi

echo
echo "  ${PASS} passed, ${FAIL} failed"
[ "$FAIL" -eq 0 ]
