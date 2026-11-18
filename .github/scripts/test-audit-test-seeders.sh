#!/usr/bin/env bash
# test-audit-test-seeders.sh — prove audit-test-seeders.sh fails where it must.
#
# Each arm is a mutation: break exactly one thing the script claims to catch, and require it to go
# red for THAT reason, not merely to go red.
#
# Run: bash .github/scripts/test-audit-test-seeders.sh

set -euo pipefail

SCRIPT="$(git rev-parse --show-toplevel)/.github/scripts/audit-test-seeders.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

pass=0
fail=0

# A fixture tests/: one baselined seeder, one builder-backed helper, one test with an inline insert.
setup() {
  rm -rf "$TMP/tests"
  mkdir -p "$TMP/tests/common"
  cat > "$TMP/tests/a_test.rs" <<'RS'
async fn person(pool: &PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_profiles (handle, display_name, email) VALUES ($1, $1, $1) RETURNING id")
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn profile(pool: &PgPool, handle: &str) -> Uuid {
    temper_services::test_support::profile::insert(pool, handle).await
}

#[sqlx::test(migrator = "temper_services::MIGRATOR")]
#[allow(clippy::unwrap_used)]
async fn t(pool: PgPool) {
    sqlx::query("INSERT INTO kb_teams (slug, name) VALUES ('x', 'x')").execute(&pool).await.unwrap();
}
RS
  printf 'a_test.rs::person\n' > "$TMP/baseline"
}

run() { TEST_SEEDER_SCAN_ROOT="$TMP/tests" TEST_SEEDER_BASELINE_FILE="$TMP/baseline" bash "$SCRIPT"; }

check() { # <name> <expected-exit> <must-contain>
  local name="$1" want="$2" needle="$3"
  local out rc
  set +e
  out="$(run 2>&1)"; rc=$?
  set -e
  if [[ "$rc" == "$want" ]] && grep -qF -- "$needle" <<<"$out"; then
    echo "  ok  — $name"; pass=$((pass+1))
  else
    echo "  FAIL — $name (exit $rc, wanted $want)"; echo "$out" | sed 's/^/        /'
    fail=$((fail+1))
  fi
}

echo "audit-test-seeders.sh:"

setup
check "the baseline, a builder-backed helper and an inline test insert pass" 0 "1 genuinely-different seeder(s)"

# A new hand-rolled profile seeder.
setup
cat >> "$TMP/tests/a_test.rs" <<'RS'

async fn seed_profile(pool: &PgPool, handle: &str) -> Uuid {
    sqlx::query_scalar("INSERT INTO kb_profiles (handle, display_name) VALUES ($1, $1) RETURNING id")
        .bind(handle)
        .fetch_one(pool)
        .await
        .unwrap()
}
RS
check "a new profile seeder fails" 1 "a_test.rs::seed_profile inserts"

# A team seeder in a shared module, with a multi-line signature and a pub(crate) qualifier.
setup
cat > "$TMP/tests/common/mod.rs" <<'RS'
pub(crate) async fn team(
    pool: &PgPool,
    slug: &str,
) -> Uuid {
    sqlx::query_scalar(
        "INSERT INTO kb_teams (slug, name) VALUES ($1, $1) RETURNING id",
    )
    .bind(slug)
    .fetch_one(pool)
    .await
    .unwrap()
}
RS
check "a team seeder in a nested module file fails" 1 "common/mod.rs::team inserts"

# A context seeder indented inside a mod block, behind a doc comment.
setup
cat >> "$TMP/tests/a_test.rs" <<'RS'

mod helpers {
    /// docs
    pub async fn ctx(pool: &PgPool, owner: Uuid) -> Uuid {
        sqlx::query_scalar("INSERT INTO kb_contexts (owner_table, owner_id, slug, name) VALUES ('kb_profiles', $1, 's', 's') RETURNING id")
            .bind(owner)
            .fetch_one(pool)
            .await
            .unwrap()
    }
}
RS
check "an indented context seeder fails" 1 "a_test.rs::ctx inserts"

# A table whose name only starts with a watched one is not watched.
setup
cat >> "$TMP/tests/a_test.rs" <<'RS'

async fn link(pool: &PgPool, parent: Uuid, child: Uuid) {
    sqlx::query("INSERT INTO kb_teams_parents (parent_id, child_id) VALUES ($1, $2)")
        .bind(parent)
        .bind(child)
        .execute(pool)
        .await
        .unwrap();
}
RS
check "kb_teams_parents is not kb_teams" 0 "1 genuinely-different seeder(s)"

# A test fn's attributes do not leak onto the helper after it.
setup
cat >> "$TMP/tests/a_test.rs" <<'RS'

#[tokio::test]
async fn plain() {}

async fn after(pool: &PgPool) {
    sqlx::query("INSERT INTO kb_teams (slug, name) VALUES ('y', 'y')").execute(pool).await.unwrap();
}
RS
check "a helper after a test is still a helper" 1 "a_test.rs::after inserts"

# The ratchet: a baselined seeder collapsed onto a builder must leave the baseline.
setup
python3 -I - "$TMP/tests/a_test.rs" <<'PY'
import sys
p = sys.argv[1]
s = open(p).read()
s = s.replace('INSERT INTO kb_profiles (handle, display_name, email)', 'SELECT 1 /* collapsed */ --')
open(p, 'w').write(s)
PY
check "a collapsed baseline seeder still in the baseline fails" 1 "a_test.rs::person no longer seeds"

echo "  $pass passed, $fail failed"
[[ $fail -eq 0 ]]
