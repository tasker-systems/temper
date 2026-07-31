#!/usr/bin/env bash
# .github/scripts/test-audit-sqlx-macro-exceptions.sh
#
# Test harness for audit-sqlx-macro-exceptions.sh — the tripwire pinning the set of production SQL
# queries that are NOT compile-time-checked.
#
# The check is only as good as the enumerator it shells out to, so most of what is under test here
# is that enumerator's discrimination. Three of these cases are regressions with a history:
#
#   * A NESTED-GENERIC TURBOFISH must count. `sqlx::query_as::<_, (Uuid, Option<String>, i32)>(…)`
#     went unseen for a year because the pattern used `(?:::<[^>]*>)?` and a character class cannot
#     cross the `>` of `Option<String>`. It hid 10 production sites — and it hid them in the exact
#     place that mattered, because with the 56 KNOWN sites converted the old pattern reported
#     exactly 46, which is this allow-list's size. The done-number was reachable with ten
#     unconverted sites behind the blind spot.
#
#   * A NON-SQLX `.query(` must NOT count. reqwest's `RequestBuilder::query` is spelled the same;
#     a path-less pattern reported 7 "sqlx calls" in temper-client, a crate with no sqlx dependency.
#
#   * A CALL IN `#[cfg(test)] mod` must NOT count as production. The wire contract that can break a
#     deploy is the running binary's.
#
#   bash .github/scripts/test-audit-sqlx-macro-exceptions.sh

set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
AUDIT_SCRIPT="${SCRIPT_DIR}/audit-sqlx-macro-exceptions.sh"
PASS=0
FAIL=0

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

ok()  { echo "  PASS: $1"; PASS=$((PASS + 1)); }
bad() { echo "  FAIL: $1"; shift; printf '    %s\n' "$@"; FAIL=$((FAIL + 1)); }

# A fixture tree shaped like the repo (crates/<name>/src/**.rs), holding two declared exceptions.
new_fixture() {
  local d="$1"
  rm -rf "$d"
  mkdir -p "$d/crates/alpha/src" "$d/crates/beta/src"
  cat > "$d/crates/alpha/src/lib.rs" <<'RS'
// Two runtime calls, both declared in the fixture allow-list below.
async fn dump(table: &str, pool: &PgPool) {
    sqlx::query(&format!("SELECT * FROM {table}")).fetch_all(pool).await.unwrap();
    sqlx::query("UPDATE kb_chunks SET embedding = $1::vector WHERE id = $2").execute(pool).await.unwrap();
}
RS
  cat > "$d/crates/beta/src/lib.rs" <<'RS'
// A compile-checked macro. Never an exception, never counted as one.
async fn ok(pool: &PgPool) {
    sqlx::query_scalar!("SELECT id FROM kb_profiles WHERE id = $1", x).fetch_one(pool).await.unwrap();
}
RS
}

BASELINE_FILE="${WORK}/allowlist"
cat > "$BASELINE_FILE" <<'EOF'
dynamic-table 1 crates/alpha/src/lib.rs
vector-cast 1 crates/alpha/src/lib.rs
EOF

# run FIXTURE_DIR -> prints exit code, stashes combined output in $LAST_OUT
LAST_OUT=""
run() {
  LAST_OUT="$(SQLX_SCAN_ROOT="$1" SQLX_ALLOWLIST_FILE="$BASELINE_FILE" bash "$AUDIT_SCRIPT" 2>&1)"
  echo $?
}

FX="${WORK}/fx"

echo "audit-sqlx-macro-exceptions guard tests"
echo

# ── 1. the declared set passes ────────────────────────────────────────────────────────────────
new_fixture "$FX"
rc="$(run "$FX")"
if [[ "$rc" == "0" ]]; then
  ok "a tree matching the allow-list exits 0"
else
  bad "a tree matching the allow-list should exit 0 (got $rc)" "$LAST_OUT"
fi

# ── 2. an undeclared runtime query fails ──────────────────────────────────────────────────────
new_fixture "$FX"
cat >> "$FX/crates/beta/src/lib.rs" <<'RS'
async fn sneaky(pool: &PgPool) {
    sqlx::query("SELECT id FROM kb_resources").fetch_all(pool).await.unwrap();
}
RS
rc="$(run "$FX")"
if [[ "$rc" == "1" ]]; then
  ok "a NEW undeclared runtime query fails the check"
else
  bad "a new undeclared runtime query should fail (got $rc)" "$LAST_OUT"
fi

# ── 3. THE REGRESSION: a nested-generic turbofish is not invisible ────────────────────────────
new_fixture "$FX"
cat >> "$FX/crates/beta/src/lib.rs" <<'RS'
async fn tuple_projection(pool: &PgPool) {
    sqlx::query_as::<_, (Uuid, Option<String>, i32)>("SELECT a, b, c FROM t")
        .fetch_all(pool).await.unwrap();
}
RS
rc="$(run "$FX")"
if [[ "$rc" == "1" ]]; then
  ok "a nested-generic turbofish call is COUNTED (the blind spot that hid 10 sites)"
else
  bad "query_as::<_, (Uuid, Option<String>, i32)>(..) must be counted (got $rc)" "$LAST_OUT"
fi

# ── 4. a non-sqlx .query( is not mistaken for one ─────────────────────────────────────────────
new_fixture "$FX"
cat >> "$FX/crates/beta/src/lib.rs" <<'RS'
async fn http(client: reqwest::Client) {
    client.get("https://example.test").query(&[("page", "2")]).send().await.unwrap();
}
RS
rc="$(run "$FX")"
if [[ "$rc" == "0" ]]; then
  ok "reqwest's RequestBuilder::query is NOT counted as an sqlx call"
else
  bad "a non-sqlx .query( must not count (got $rc)" "$LAST_OUT"
fi

# ── 5. a call inside #[cfg(test)] mod is not production ───────────────────────────────────────
new_fixture "$FX"
cat >> "$FX/crates/beta/src/lib.rs" <<'RS'
#[cfg(test)]
mod tests {
    async fn fixture(pool: &PgPool) {
        sqlx::query("INSERT INTO kb_profiles (id) VALUES ($1)").execute(pool).await.unwrap();
    }
}
RS
rc="$(run "$FX")"
if [[ "$rc" == "0" ]]; then
  ok "a runtime query inside #[cfg(test)] mod is not a production site"
else
  bad "a #[cfg(test)] call must not count (got $rc)" "$LAST_OUT"
fi

# ── 6. converting a site also trips the check ─────────────────────────────────────────────────
# A guard that only notices ADDITIONS lets the baseline drift above the truth, and an allow-list
# larger than reality is exactly the "place to put things" this exists to prevent.
new_fixture "$FX"
cat > "$FX/crates/alpha/src/lib.rs" <<'RS'
async fn dump(table: &str, pool: &PgPool) {
    sqlx::query(&format!("SELECT * FROM {table}")).fetch_all(pool).await.unwrap();
}
RS
rc="$(run "$FX")"
if [[ "$rc" == "1" ]]; then
  ok "REMOVING a declared exception also trips the check (baseline cannot drift above reality)"
else
  bad "a removed exception should trip the check (got $rc)" "$LAST_OUT"
fi

# ── 7. --list reports counts without judging them ─────────────────────────────────────────────
new_fixture "$FX"
listed="$(SQLX_SCAN_ROOT="$FX" SQLX_ALLOWLIST_FILE="$BASELINE_FILE" bash "$AUDIT_SCRIPT" --list 2>&1)"
if [[ "$listed" == *"2 crates/alpha/src/lib.rs"* ]]; then
  ok "--list prints the current per-file counts"
else
  bad "--list should report 2 for the fixture's alpha crate" "$listed"
fi

echo
echo "  ${PASS} passed, ${FAIL} failed"
[[ "$FAIL" -eq 0 ]]
