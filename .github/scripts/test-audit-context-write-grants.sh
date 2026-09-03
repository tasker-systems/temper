#!/usr/bin/env bash
# test-audit-context-write-grants.sh — harness for audit-context-write-grants.sh.
#
# The guard's claims, each proven here rather than asserted:
#   1. RED: a grant door accepting kb_contexts subjects fails, naming the fixture.
#   2. RED: a raw mint statement (kb_contexts + can_write) fails — SQL or Rust-src resident.
#   3. RED-direction of the canaries: an empty scan fails ("the scan broke"), it does not pass.
#   4. GREEN field-of-view pins: write-class bits on non-context subjects, read-only context
#      grants, and fixtures under tests/ all stay green — the guard watches exactly what its
#      header says and its silence is never broader than that.
#
# Each RED case is single-cause: fixtures carry enough clean doors and clean migrations to
# satisfy both canaries, so the only thing that can fire is the claim under test.
#
#   bash .github/scripts/test-audit-context-write-grants.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT_SCRIPT="${SCRIPT_DIR}/audit-context-write-grants.sh"
PASS=0
FAIL=0

FIXTURE_DIR="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_DIR"' EXIT

ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; shift; printf '    %s\n' "$@"; FAIL=$((FAIL + 1)); }

# doors TREE — a crates-like tree with FOUR clean doors (satisfies the door canary).
doors() {
    local d="$1" i door
    rm -rf "$d"
    for i in 1 2 3 4; do
        door="$d/pkg$i/src"
        mkdir -p "$door"
        if (( i % 2 == 1 )); then
            cat > "${door}/mod.rs" <<EOF
fn build(id: uuid::Uuid) -> GrantCapabilityRequest {
    GrantCapabilityRequest {
        subject_table: "kb_resources".to_string(),
        subject_id: id,
    }
}
EOF
        else
            cat > "${door}/mod.rs" <<EOF
fn build(id: uuid::Uuid) -> GrantCapabilityRequest {
    GrantCapabilityRequest {
        subject_table: "kb_cogmaps".to_string(),
        subject_id: id,
    }
}
EOF
        fi
    done
}

# migs DIR — migrations reproducing the reviewed non-context SQL writes (satisfies the SQL canary).
migs() {
    local d="$1"
    rm -rf "$d"; mkdir -p "$d"
    cat > "${d}/20260701000001_cogmap_write_tightening.sql" <<'EOF'
INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id,
                              can_read, granted_by_profile_id)
SELECT 'kb_cogmaps', id, 'kb_profiles', creator_id FROM kb_cogmaps;
EOF
    cat > "${d}/20260701000003_access_grants_store_migration.sql" <<'EOF'
INSERT INTO kb_access_grants (subject_table, subject_id)
SELECT 'kb_resources', resource_id FROM kb_resource_access;
EOF
}

# expect_guard NAME WANT_RC WANT_SUBSTR CRATES_DIR MIGRATIONS_DIR
expect_guard() {
    local name="$1" want_rc="$2" want_msg="$3" crates="$4" mdir="$5"
    local out rc
    set +e
    out="$(CRATES_DIR="$crates" MIGRATIONS_DIR="$mdir" bash "$AUDIT_SCRIPT" 2>&1)"; rc=$?
    set -e
    if [ "$rc" -eq "$want_rc" ] && printf '%s' "$out" | grep -qF "$want_msg"; then
        ok "$name"
    else
        bad "$name" "want rc=${want_rc} msg=[${want_msg}]" "got rc=${rc}" "output: ${out}"
    fi
}

DOORS="${FIXTURE_DIR}/doors";        doors "$DOORS"
MIGS="${FIXTURE_DIR}/migs";          migs "$MIGS"
NO_MIGS="${FIXTURE_DIR}/vacuum"      # deliberately never created
NO_CRATES="${FIXTURE_DIR}/void";     mkdir -p "$NO_CRATES"   # a crates dir with no .rs at all

echo "Running audit-context-write-grants.sh tests..."
echo ""

# --- (a) the real tree is green end-to-end ---
set +e
OUT="$(bash "$AUDIT_SCRIPT" 2>&1)"; RC=$?
set -e
if [ "$RC" -eq 0 ]; then ok "real repo: no context write-grant mint surface"
else bad "real repo: no context write-grant mint surface" "exit=${RC}" "output: ${OUT}"; fi

# --- (b) RED: a fifth door accepting kb_contexts subjects ---
doors "${FIXTURE_DIR}/ctxdoor"
cat > "${FIXTURE_DIR}/ctxdoor/pkg1/src/contexts.rs" <<'EOF'
fn build(id: uuid::Uuid) -> GrantCapabilityRequest {
    GrantCapabilityRequest {
        subject_table: "kb_contexts".to_string(),
        subject_id: id,
    }
}
EOF
expect_guard "new context-subject door: detected and named" 1 "kb_contexts" \
    "${FIXTURE_DIR}/ctxdoor" "$MIGS"

# --- (c) RED: the door canary — a crates tree with zero doors breaks the scan, not the property ---
expect_guard "door canary: zero doors fails the scan loudly" 1 "The scan broke" \
    "$NO_CRATES" "$MIGS"

# --- (d) RED: a raw kb_contexts+can_write mint statement in SQL ---
mkdir -p "${FIXTURE_DIR}/sqlmint"
cat > "${FIXTURE_DIR}/sqlmint/20260903000001_delegated_context_authorship.sql" <<'EOF'
INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id,
                              can_read, can_write, granted_by_profile_id)
VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, true, $3);
EOF
expect_guard "raw kb_contexts+can_write mint: detected and named" 1 "delegated_context_authorship" \
    "$DOORS" "${FIXTURE_DIR}/sqlmint"

# --- (e) RED: the same mint resident in a Rust src tree ---
doors "${FIXTURE_DIR}/rsmint"
mkdir -p "${FIXTURE_DIR}/rsmint/svc/src"
cat > "${FIXTURE_DIR}/rsmint/svc/src/grants.rs" <<'EOF'
pub(crate) async fn mint(pool: &sqlx::PgPool) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (subject_table, subject_id, principal_table, principal_id, can_read, can_write) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, true)",
    )
    .execute(pool)
    .await
    .unwrap();
}
EOF
expect_guard "Rust src-resident kb_contexts+can_write mint: detected" 1 "grants.rs" \
    "${FIXTURE_DIR}/rsmint" "$MIGS"

# --- (f) RED: the SQL canary — a migrations dir with no kb_access_grants writes in view ---
expect_guard "SQL canary: zero kb_access_grants writes in view fails loudly" 1 "The scan broke" \
    "$DOORS" "$NO_MIGS"

# --- (g) GREEN field of view: write-class bits on NON-context subjects are grant-sinks' territory ---
migs "${FIXTURE_DIR}/nonctx"
cat >> "${FIXTURE_DIR}/nonctx/20260701000003_access_grants_store_migration.sql" <<'EOF'
INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id,
                              can_read, can_write, granted_by_profile_id)
VALUES ('kb_resources', $1, 'kb_profiles', $2, true, true, $3);
EOF
expect_guard "resource write-grant mint: not this guard's surface" 0 "no kb_contexts write-grant" \
    "$DOORS" "${FIXTURE_DIR}/nonctx"

# --- (h) GREEN field of view: a READ-ONLY kb_contexts grant is the designed read axis ---
migs "${FIXTURE_DIR}/readonly"
cat >> "${FIXTURE_DIR}/readonly/20260701000003_access_grants_store_migration.sql" <<'EOF'
INSERT INTO kb_access_grants (subject_table, subject_id, principal_table, principal_id,
                              can_read, granted_by_profile_id)
VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, $3);
EOF
expect_guard "read-only context grant: designed axis, stays green" 0 "no kb_contexts write-grant" \
    "$DOORS" "${FIXTURE_DIR}/readonly"

# --- (i) GREEN field of view: tests/ directories are out of scope ---
doors "${FIXTURE_DIR}/withtests"
mkdir -p "${FIXTURE_DIR}/withtests/pkg1/tests"
cat > "${FIXTURE_DIR}/withtests/pkg1/tests/arm_coverage.rs" <<'EOF'
async fn seed_context_write(pool: &sqlx::PgPool, ctx: uuid::Uuid) {
    sqlx::query(
        "INSERT INTO kb_access_grants \
           (subject_table, subject_id, principal_table, principal_id, can_read, can_write) \
         VALUES ('kb_contexts', $1, 'kb_profiles', $2, true, true)",
    )
    .execute(pool)
    .await
    .unwrap();
}
EOF
expect_guard "tests/ fixture minting the row: out of the guard's field of view" 0 "no kb_contexts write-grant" \
    "${FIXTURE_DIR}/withtests" "$MIGS"

echo ""
echo "$PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
