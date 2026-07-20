#!/usr/bin/env bash
# .github/scripts/test-audit-grant-sinks.sh
#
# Test harness for audit-grant-sinks.sh's SQL half — the scan that closed the blind spot where the
# authoritative kb_access_grants write had moved into SQL and out of the guard's field of view.
#
# Two claims are under test, and the second is the one the design rests on:
#   1. A NEW SQL function writing kb_access_grants is caught.
#   2. A REDEFINITION of an existing one (DROP+CREATE in a later migration, which is how immutable
#      migrations change a function) is NOT caught. If redefinitions churned the baseline, the
#      guard would be UPDATE_BASELINE'd on reflex until nobody read its diffs — the failure mode
#      where a guard is still green, still running, and no longer load-bearing.
#
#   bash .github/scripts/test-audit-grant-sinks.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT_SCRIPT="${SCRIPT_DIR}/audit-grant-sinks.sh"
PASS=0
FAIL=0

FIXTURE_DIR="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_DIR"' EXIT

ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; shift; printf '    %s\n' "$@"; FAIL=$((FAIL + 1)); }

# baseline_migrations DIR — write a fixture migrations dir reproducing the four reviewed SQL sites.
baseline_migrations() {
    local d="$1"
    mkdir -p "$d"
    cat > "${d}/20260701000001_cogmap_write_tightening.sql" <<'EOF'
-- one-time backfill, top-level DML
INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id)
SELECT 'kb_cognitive_maps', id, 'kb_profiles', creator_id FROM kb_cognitive_maps;
EOF
    cat > "${d}/20260701000003_access_grants_store_migration.sql" <<'EOF'
-- one-time backfill from kb_resource_access
INSERT INTO kb_access_grants (subject_table, subject_id)
SELECT 'kb_resources', resource_id FROM kb_resource_access;
EOF
    cat > "${d}/20260718000010_admin_grant_fns.sql" <<'EOF'
CREATE FUNCTION _admin_grant_created(p_actor uuid) RETURNS void AS $$
BEGIN
    INSERT INTO kb_access_grants (subject_table, subject_id) VALUES ('x', p_actor);
END;
$$;

CREATE FUNCTION _admin_grant_revoked(p_actor uuid) RETURNS void AS $$
BEGIN
    DELETE FROM kb_access_grants WHERE subject_id = p_actor;
END;
$$;
EOF
}

# sql_sites DIR — the SQL half of `--list`.
sql_sites() {
    MIGRATIONS_DIR="$1" bash "$AUDIT_SCRIPT" --list 2>&1 | sed -n '/^--- SQL:$/,$p' | tail -n +2 | sort -u
}

# expect_sites NAME DIR EXPECTED
expect_sites() {
    local name="$1" dir="$2" expected actual
    expected="$(printf '%s' "$3" | sort -u)"
    actual="$(sql_sites "$dir")"
    if [ "$actual" = "$expected" ]; then ok "$name"
    else bad "$name" "expected: [${expected}]" "actual:   [${actual}]"; fi
}

echo "Running audit-grant-sinks.sh SQL tests..."
echo ""

# --- (a) the real migrations tree matches the reviewed SQL baseline (end-to-end, exit 0) ---
set +e
OUT="$(bash "$AUDIT_SCRIPT" 2>&1)"; RC=$?
set -e
if [ "$RC" -eq 0 ]; then ok "real repo: Rust + SQL baselines both match"
else bad "real repo: Rust + SQL baselines both match" "exit=${RC}" "output: ${OUT}"; fi

# --- (b) the fixture reproducing the reviewed set yields exactly the baseline keys ---
BASE="${FIXTURE_DIR}/base"
baseline_migrations "$BASE"
expect_sites "fixture of the reviewed set: yields the 4 baseline keys" "$BASE" \
"<top-level>:20260701000001_cogmap_write_tightening.sql
<top-level>:20260701000003_access_grants_store_migration.sql
_admin_grant_created
_admin_grant_revoked"

# --- (c) BITE: a NEW SQL function writing the table is caught ---
NEWFN="${FIXTURE_DIR}/newfn"
baseline_migrations "$NEWFN"
cat > "${NEWFN}/20260801000001_sneaky_grant.sql" <<'EOF'
CREATE FUNCTION _bulk_grant_everyone(p_team uuid) RETURNS void AS $$
BEGIN
    INSERT INTO kb_access_grants (subject_table, subject_id)
    SELECT 'kb_resources', id FROM kb_resources;
END;
$$;
EOF
expect_sites "NEW SQL function writing kb_access_grants: detected" "$NEWFN" \
"<top-level>:20260701000001_cogmap_write_tightening.sql
<top-level>:20260701000003_access_grants_store_migration.sql
_admin_grant_created
_admin_grant_revoked
_bulk_grant_everyone"

set +e
OUT="$(MIGRATIONS_DIR="$NEWFN" bash "$AUDIT_SCRIPT" 2>&1)"; RC=$?
set -e
if [ "$RC" -eq 1 ] && printf '%s' "$OUT" | grep -qF "_bulk_grant_everyone"; then
    ok "  ...and fails the guard, naming the new sink"
else
    bad "  ...and fails the guard, naming the new sink" "exit=${RC}" "output: ${OUT}"
fi

# --- (d) THE DESIGN CLAIM: a redefinition of an existing function does NOT churn the set ---
REDEF="${FIXTURE_DIR}/redef"
baseline_migrations "$REDEF"
cat > "${REDEF}/20260801000002_patch_admin_grant.sql" <<'EOF'
-- How a shipped, immutable SQL function is changed: a NEW migration doing DROP+CREATE.
DROP FUNCTION IF EXISTS _admin_grant_created(uuid);
CREATE FUNCTION _admin_grant_created(p_actor uuid, p_reason text) RETURNS void AS $$
BEGIN
    INSERT INTO kb_access_grants (subject_table, subject_id) VALUES ('x', p_actor);
END;
$$;
EOF
expect_sites "redefinition of an existing function: set UNCHANGED" "$REDEF" \
"<top-level>:20260701000001_cogmap_write_tightening.sql
<top-level>:20260701000003_access_grants_store_migration.sql
_admin_grant_created
_admin_grant_revoked"

set +e
MIGRATIONS_DIR="$REDEF" bash "$AUDIT_SCRIPT" >/dev/null 2>&1; RC=$?
set -e
if [ "$RC" -eq 0 ]; then ok "  ...and the guard stays green (no reflex UPDATE_BASELINE)"
else bad "  ...and the guard stays green (no reflex UPDATE_BASELINE)" "exit=${RC}"; fi

# --- (e) BITE: a NEW top-level backfill is caught (keyed by basename, so it cannot hide) ---
BACKFILL="${FIXTURE_DIR}/backfill"
baseline_migrations "$BACKFILL"
cat > "${BACKFILL}/20260801000003_new_backfill.sql" <<'EOF'
INSERT INTO kb_access_grants (subject_table, subject_id) SELECT 'kb_teams', id FROM kb_teams;
EOF
expect_sites "NEW top-level backfill: detected" "$BACKFILL" \
"<top-level>:20260701000001_cogmap_write_tightening.sql
<top-level>:20260701000003_access_grants_store_migration.sql
<top-level>:20260801000003_new_backfill.sql
_admin_grant_created
_admin_grant_revoked"

# --- (f) an empty/missing migrations dir FAILS rather than passing vacuously ---
EMPTY="${FIXTURE_DIR}/empty"
mkdir -p "$EMPTY"
set +e
OUT="$(MIGRATIONS_DIR="$EMPTY" bash "$AUDIT_SCRIPT" 2>&1)"; RC=$?
set -e
if [ "$RC" -eq 1 ] && printf '%s' "$OUT" | grep -qF "found NO kb_access_grants writes"; then
    ok "empty migrations dir: fails rather than passing vacuously"
else
    bad "empty migrations dir: fails rather than passing vacuously" "exit=${RC}" "output: ${OUT}"
fi

echo ""
echo "Results: ${PASS} passed, ${FAIL} failed (total: $((PASS + FAIL)))"
[ "$FAIL" -eq 0 ]
