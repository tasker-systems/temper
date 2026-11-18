#!/usr/bin/env bash
# test-audit-inline-db-tests.sh — prove audit-inline-db-tests.sh fails where it must.
#
# Each arm is a mutation: break exactly one thing the script claims to catch, and require it to go
# red for THAT reason, not merely to go red.
#
# Run: bash .github/scripts/test-audit-inline-db-tests.sh

set -euo pipefail

SCRIPT="$(git rev-parse --show-toplevel)/.github/scripts/audit-inline-db-tests.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

pass=0
fail=0

# A fixture src/: one incumbent with an inline module, one module already in the sibling form.
setup() {
  rm -rf "$TMP/src"
  mkdir -p "$TMP/src/svc/moved"
  cat > "$TMP/src/svc/incumbent.rs" <<'RS'
pub fn f() {}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    #[sqlx::test(migrator = "crate::MIGRATOR")]
    async fn t(pool: PgPool) {}
}
RS
  cat > "$TMP/src/svc/moved.rs" <<'RS'
/// Mentions #[sqlx::test] only in a doc comment.
pub fn g() {}

#[cfg(all(test, feature = "test-db"))]
mod tests;
RS
  cat > "$TMP/src/svc/moved/tests.rs" <<'RS'
#[sqlx::test(migrator = "crate::MIGRATOR")]
async fn t(pool: PgPool) {}
RS
  printf 'svc/incumbent.rs\n' > "$TMP/baseline"
}

run() { INLINE_DB_SCAN_ROOT="$TMP/src" INLINE_DB_BASELINE_FILE="$TMP/baseline" bash "$SCRIPT"; }

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

echo "audit-inline-db-tests.sh:"

setup
check "the baseline and the sibling form pass" 0 "1 incumbent file(s) still to move"

# An inline module returns to a moved file.
setup
cat > "$TMP/src/svc/moved.rs" <<'RS'
pub fn g() {}

#[cfg(all(test, feature = "test-db"))]
mod tests {
    fn helper() {}
}
RS
check "an inline module in a non-baseline file fails" 1 "svc/moved.rs holds a DB test module inline"

# Stacked attributes between the cfg and the mod do not hide it; nor does the reversed cfg order.
setup
cat > "$TMP/src/svc/stacked.rs" <<'RS'
#[cfg(all(feature = "test-db", test))]
#[allow(clippy::unwrap_used)]
/// docs
pub(crate) mod db_tests {
}
RS
check "stacked attributes and reversed cfg order are still caught" 1 "svc/stacked.rs holds"

# A DB test under another gate in a production file.
setup
cat > "$TMP/src/svc/other_gate.rs" <<'RS'
pub fn h() {}
#[cfg(test)]
mod tests {
    #[sqlx::test]
    async fn t(pool: PgPool) {}
}
RS
check "a #[sqlx::test] in a production file under another gate fails" 1 "svc/other_gate.rs holds"

# A test-support module gated on the feature alone is not a test module.
setup
cat > "$TMP/src/svc/support.rs" <<'RS'
#[cfg(feature = "test-db")]
pub mod test_support {
}
RS
check "a feature-only test_support module passes" 0 "still to move"

# The ratchet: a baseline file that was moved must leave the baseline.
setup
printf 'pub fn f() {}\n#[cfg(all(test, feature = "test-db"))]\nmod tests;\n' > "$TMP/src/svc/incumbent.rs"
check "a moved file still in the baseline fails" 1 "svc/incumbent.rs no longer holds"

echo "  $pass passed, $fail failed"
[[ $fail -eq 0 ]]
