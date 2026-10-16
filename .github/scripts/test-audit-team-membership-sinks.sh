#!/usr/bin/env bash
# .github/scripts/test-audit-team-membership-sinks.sh
#
# Test harness for audit-team-membership-sinks.sh (team-role rank plan, task C6). Each case builds a
# fixture tree under mktemp and points the script at it through its CRATES_DIR / MIGRATIONS_DIR
# seams. The claims under test:
#   - the real repo is green;
#   - a new Rust sink is caught, and named;
#   - a sink inside a `#[cfg(test)] mod … { }` is ignored, and one in a module gated
#     `#[cfg(all(test, feature = "test-db"))]` behind another attribute too;
#   - a production item AFTER a test module is still counted (the skip ends at the close brace);
#   - a doc comment mentioning `#[cfg(test)]`, a `#[cfg(test)]` on a non-mod item, and a
#     `#[cfg(feature = "test-db")]` mod skip nothing;
#   - a second caller of `_admin_team_membership_write` is caught;
#   - a new SQL function writing the table is caught, including one closed with `$function$;`;
#   - a redefinition of an allowlisted function stays green;
#   - DML after a `$$ LANGUAGE plpgsql;` close is top-level, not the function above it;
#   - an empty migrations dir fails.
#
#   bash .github/scripts/test-audit-team-membership-sinks.sh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT_SCRIPT="${SCRIPT_DIR}/audit-team-membership-sinks.sh"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
PASS=0
FAIL=0

FIXTURE_DIR="$(mktemp -d)"
trap 'rm -rf "$FIXTURE_DIR"' EXIT

ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; shift; printf '    %s\n' "$@"; FAIL=$((FAIL + 1)); }

# run_audit CRATES MIGRATIONS [ARGS…] — sets OUT and RC.
run_audit() {
    local crates="$1" migrations="$2"
    shift 2
    set +e
    OUT="$(CRATES_DIR="$crates" MIGRATIONS_DIR="$migrations" bash "$AUDIT_SCRIPT" "$@" 2>&1)"
    RC=$?
    set -e
}

# A fixture crates tree reproducing the reviewed Rust baseline: the three write files, and the one
# writer caller. Paths mirror the real ones, because the baseline names real paths.
baseline_crates() {
    local d="$1"
    mkdir -p "$d/crates/temper-services/src/services" "$d/crates/temper-services/src/authz" \
             "$d/crates/temper-substrate/src/scenario/access"
    cat > "$d/crates/temper-services/src/services/access_service.rs" <<'EOF'
pub async fn promote_admin() {
    sqlx::query!("INSERT INTO kb_team_members (team_id, profile_id, role) VALUES ($1, $2, 'owner')");
}
EOF
    cat > "$d/crates/temper-services/src/services/subscription_test_support.rs" <<'EOF'
fn seed() {
    sqlx::query!("INSERT INTO kb_team_members (team_id, profile_id) VALUES ($1, $2)");
    sqlx::query!("DELETE FROM kb_team_members WHERE team_id = $1");
}
EOF
    cat > "$d/crates/temper-substrate/src/scenario/access/loader.rs" <<'EOF'
fn load() { let q = "insert into kb_team_members (team_id) values ($1)"; }
EOF
    cat > "$d/crates/temper-services/src/authz/team_membership.rs" <<'EOF'
//! `membership_write` is the only caller of `_admin_team_membership_write`.
pub(crate) async fn membership_write() {
    sqlx::query_scalar!("SELECT _admin_team_membership_write($1)");
}
EOF
}

# A fixture migrations dir reproducing the reviewed SQL baseline.
baseline_migrations() {
    local d="$1"
    mkdir -p "$d"
    cat > "$d/20260624000002_canonical_functions.sql" <<'EOF'
CREATE OR REPLACE FUNCTION sync_system_membership() RETURNS trigger AS $$
BEGIN
    INSERT INTO kb_team_members (team_id, profile_id, role) VALUES (NEW.a, NEW.b, 'owner');
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE FUNCTION sync_personal_team(p uuid) RETURNS void AS $$
BEGIN
    INSERT INTO kb_team_members (team_id, profile_id, role) VALUES (p, p, 'owner')
        ON CONFLICT DO NOTHING;
END;
$$;
EOF
    cat > "$d/20260629000002_auto_join.sql" <<'EOF'
CREATE FUNCTION ensure_auto_join_memberships(p uuid) RETURNS void AS $fn$
BEGIN
    INSERT INTO kb_team_members (team_id, profile_id, role) SELECT id, p, auto_join_role FROM kb_teams;
END;
$fn$;
create function backfill_auto_join_team(t uuid) returns void as $function$
begin
    insert into kb_team_members (team_id, profile_id, role) select t, id, 'member' from kb_profiles;
end;
$function$;
CREATE FUNCTION auto_join_reconcile() RETURNS void AS $$
BEGIN
    DELETE FROM kb_team_members WHERE false;
END;
$$;
EOF
    cat > "$d/20261019100000_machines_never_govern.sql" <<'EOF'
UPDATE kb_team_members SET role = 'member' WHERE role IN ('owner', 'maintainer');
EOF
    cat > "$d/20261024100000_machine_guards.sql" <<'EOF'
CREATE OR REPLACE FUNCTION machine_client_caps_profile() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE kb_team_members SET role = 'member' WHERE profile_id = NEW.profile_id;
    RETURN NEW;
END;
$$;
EOF
    cat > "$d/20261101200000_team_membership_changed.sql" <<'EOF'
-- A comment naming INSERT INTO kb_team_members is not a write.
CREATE FUNCTION _admin_team_membership_write(p_team uuid) RETURNS text AS $$
BEGIN
    INSERT INTO kb_team_members (team_id) VALUES (p_team);
    UPDATE kb_team_members SET role = 'owner' WHERE team_id = p_team;
    DELETE FROM kb_team_members WHERE team_id = p_team;
    RETURN 'added';
END;
$$;
EOF
}

# fresh NAME — a fixture root holding the reviewed baseline; prints its path.
fresh() {
    local root="$FIXTURE_DIR/$1"
    baseline_crates "$root"
    baseline_migrations "$root/migrations"
    printf '%s' "$root"
}

expect_green() {
    local name="$1" root="$2"
    run_audit "$root/crates" "$root/migrations"
    if [ "$RC" -eq 0 ]; then ok "$name"; else bad "$name" "exit=${RC}" "output: ${OUT}"; fi
}

# expect_red NAME ROOT NEEDLE — the guard fails, and its output names NEEDLE.
expect_red() {
    local name="$1" root="$2" needle="$3"
    run_audit "$root/crates" "$root/migrations"
    if [ "$RC" -eq 1 ] && printf '%s' "$OUT" | grep -qF -- "$needle"; then ok "$name"
    else bad "$name" "exit=${RC} (want 1, naming ${needle})" "output: ${OUT}"; fi
}

echo "Running audit-team-membership-sinks.sh tests..."
echo ""

# --- the real repo ---
set +e
OUT="$(cd "$REPO_ROOT" && bash "$AUDIT_SCRIPT" 2>&1)"; RC=$?
set -e
if [ "$RC" -eq 0 ]; then ok "real repo: every baseline matches"
else bad "real repo: every baseline matches" "exit=${RC}" "output: ${OUT}"; fi

# --- the fixture of the reviewed set is green ---
expect_green "fixture of the reviewed set: green" "$(fresh base)"

# --- BITE: a new Rust sink in production code ---
R="$(fresh rust-sink)"
cat > "$R/crates/temper-services/src/services/sneaky_service.rs" <<'EOF'
pub async fn make_owner() {
    sqlx::query!("UPDATE kb_team_members SET role = 'owner' WHERE profile_id = $1");
}
EOF
expect_red "a new Rust sink: caught and named" "$R" "sneaky_service.rs"

# --- PIN: a sink inside a #[cfg(test)] mod is ignored, nested braces and all ---
R="$(fresh test-mod)"
cat > "$R/crates/temper-services/src/services/tested_service.rs" <<'EOF'
pub fn production() -> u32 { 1 }

#[cfg(test)]
mod tests {
    #[test]
    fn seeds() {
        if true {
            let s = format!("{}", "INSERT INTO kb_team_members (team_id) VALUES ($1)");
            let _ = '{';
        }
    }
}
EOF
cat > "$R/crates/temper-services/src/services/db_tested_service.rs" <<'EOF'
#[cfg(all(test, feature = "test-db"))]
#[expect(clippy::unwrap_used, reason = "test")]
pub(crate) mod db_tests {
    fn seed() { sqlx::query!("DELETE FROM kb_team_members"); }
}
EOF
expect_green "a sink inside a test module (and an all(test, …) one behind another attribute): ignored" "$R"

# --- BITE: a production item AFTER a test module is still counted ---
R="$(fresh after-mod)"
cat > "$R/crates/temper-services/src/services/after_service.rs" <<'EOF'
#[cfg(test)]
mod tests {
    fn nested() { { } }
}

pub async fn after_the_tests() {
    sqlx::query!("INSERT INTO kb_team_members (team_id) VALUES ($1)");
}
EOF
expect_red "a production sink after a test module: still counted" "$R" "after_service.rs"

# --- BITE: a doc comment mentioning #[cfg(test)] starts no skip ---
R="$(fresh doc-mention)"
cat > "$R/crates/temper-services/src/services/doc_service.rs" <<'EOF'
/// Not to be confused with a #[cfg(test)] module.
mod not_tests {
    pub async fn write() {
        sqlx::query!("INSERT INTO kb_team_members (team_id) VALUES ($1)");
    }
}
EOF
expect_red "a doc comment mentioning #[cfg(test)]: skips nothing" "$R" "doc_service.rs"

# --- BITE: #[cfg(test)] on a non-mod item skips nothing after it ---
R="$(fresh item-cfg)"
cat > "$R/crates/temper-services/src/services/item_service.rs" <<'EOF'
#[cfg(test)]
fn helper() {}

pub async fn write() {
    sqlx::query!("INSERT INTO kb_team_members (team_id) VALUES ($1)");
}
EOF
expect_red "#[cfg(test)] on a non-mod item: skips nothing" "$R" "item_service.rs"

# --- BITE: a feature-only cfg is not a test cfg ---
R="$(fresh feature-cfg)"
cat > "$R/crates/temper-services/src/services/feature_service.rs" <<'EOF'
#[cfg(feature = "test-db")]
mod gated {
    fn write() { sqlx::query!("INSERT INTO kb_team_members (team_id) VALUES ($1)"); }
}
EOF
expect_red "#[cfg(feature = \"test-db\")] mod: not skipped" "$R" "feature_service.rs"

# --- BITE: a second caller of the SQL writer ---
R="$(fresh second-caller)"
cat > "$R/crates/temper-services/src/services/bypass_service.rs" <<'EOF'
pub async fn bypass() {
    sqlx::query!("SELECT _admin_team_membership_write($1)");
}
EOF
expect_red "a second caller of _admin_team_membership_write: caught" "$R" "bypass_service.rs"

# --- BITE: a new SQL writer, closed with $function$; ---
R="$(fresh sql-new)"
cat > "$R/migrations/20261201000000_sneaky.sql" <<'EOF'
CREATE FUNCTION _bulk_owner(p uuid) RETURNS void AS $function$
BEGIN
    UPDATE kb_team_members SET role = 'owner' WHERE profile_id = p;
END;
$function$;
EOF
expect_red "a new SQL function writing the table: caught and named" "$R" "_bulk_owner"

# --- PIN: DML after a function closed by `$$ LANGUAGE plpgsql;` is top-level ---
R="$(fresh sql-after)"
cat > "$R/migrations/20261201000001_after.sql" <<'EOF'
CREATE FUNCTION sync_personal_team(p uuid) RETURNS void AS $$
BEGIN
    PERFORM 1;
END;
$$ LANGUAGE plpgsql;
DELETE FROM kb_team_members WHERE role = 'watcher';
EOF
expect_red "top-level DML after a \$\$ LANGUAGE close: its own key, not the function's" "$R" \
    "<top-level>:20261201000001_after.sql"

# --- PIN: a redefinition of an allowlisted function stays green ---
R="$(fresh sql-redef)"
cat > "$R/migrations/20261201000002_redefine.sql" <<'EOF'
DROP FUNCTION IF EXISTS _admin_team_membership_write(uuid);
CREATE FUNCTION _admin_team_membership_write(p_team uuid) RETURNS text AS $fn$
BEGIN
    INSERT INTO kb_team_members (team_id) VALUES (p_team) ON CONFLICT DO NOTHING;
    RETURN 'added';
END;
$fn$;
EOF
expect_green "a redefinition of an allowlisted function: green" "$R"

# --- an empty migrations dir fails ---
R="$(fresh sql-empty)"
rm -f "$R"/migrations/*.sql
expect_red "an empty migrations dir: fails" "$R" "found NO kb_team_members writes"

echo ""
echo "audit-team-membership-sinks tests: $PASS passed, $FAIL failed"
[ "$FAIL" -eq 0 ]
