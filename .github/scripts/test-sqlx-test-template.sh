#!/usr/bin/env bash
# test-sqlx-test-template.sh — prove sqlx-test-template.sh rebuilds when it must, and only then.
#
# The script's value is that a template is never served stale: a migration added, edited, renamed or
# removed must rebuild it, and a rebuild that fails must not leave a half-migrated template stamped as
# current. Each arm below breaks one of those and requires the script to react for THAT reason.
# `psql` and `sqlx` are stubs that record what they were asked, so this needs no Postgres and no
# toolchain; the real-server behaviour is exercised by every CI job that runs the script.
#
# Run: bash .github/scripts/test-sqlx-test-template.sh

set -euo pipefail

SCRIPT="$(git rev-parse --show-toplevel)/.github/scripts/sqlx-test-template.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

mkdir -p "$TMP/bin"
# psql stub: logs each statement; answers the marker read from $TMP/comment; COMMENT writes it.
cat > "$TMP/bin/psql" <<'SH'
#!/usr/bin/env bash
sql="${!#}"
echo "psql $1 :: $sql" >> "$STUB_LOG"
case "$sql" in
  *shobj_description*) cat "$STUB_DIR/comment" 2>/dev/null || true ;;
  "COMMENT ON DATABASE template1 IS '"*)
    v="${sql#COMMENT ON DATABASE template1 IS \'}"; printf '%s\n' "${v%\'}" > "$STUB_DIR/comment" ;;
  *"count(*) FROM _sqlx_migrations"*) echo 3 ;;
esac
SH
cat > "$TMP/bin/sqlx" <<'SH'
#!/usr/bin/env bash
echo "sqlx $*" >> "$STUB_LOG"
exit "$(cat "$STUB_DIR/sqlx_rc" 2>/dev/null || echo 0)"
SH
chmod +x "$TMP/bin/psql" "$TMP/bin/sqlx"

pass=0
fail=0

setup() {
  rm -rf "$TMP/m" "$TMP/comment" "$TMP/sqlx_rc"
  mkdir -p "$TMP/m"
  echo "CREATE TABLE a (id int);" > "$TMP/m/20260101000000_a.sql"
  echo "CREATE TABLE b (id int);" > "$TMP/m/20260102000000_b.sql"
}

URL="postgresql://temper:temper@localhost:5437/temper_development"
run() {
  : > "$TMP/log"
  set +e
  OUT="$(PATH="$TMP/bin:$PATH" STUB_LOG="$TMP/log" STUB_DIR="$TMP" DATABASE_URL="${RUN_URL:-$URL}" \
    SQLX_TEST_TEMPLATE_MIGRATIONS="$TMP/m" SQLX_TEST_TEMPLATE_SQLX=sqlx bash "$SCRIPT" "$@" 2>&1)"
  RC=$?
  set -e
  LOG="$(cat "$TMP/log")"
}

ok() { echo "  ok  — $1"; pass=$((pass + 1)); }
bad() {
  echo "  FAIL — $1 (exit $RC)"
  { echo "$OUT"; echo "-- calls:"; echo "$LOG"; } | sed 's/^/        /'
  fail=$((fail + 1))
}
rebuilt() { grep -q "DROP DATABASE template1" <<<"$LOG" && grep -q "^sqlx migrate run" <<<"$LOG"; }

echo "sqlx-test-template.sh:"

setup
run
if [[ $RC == 0 ]] && rebuilt && grep -q "sha256:" "$TMP/comment"; then ok "an unstamped template is rebuilt and stamped"; else bad "an unstamped template is rebuilt and stamped"; fi

run
if [[ $RC == 0 ]] && ! grep -q "DROP DATABASE" <<<"$LOG" && ! grep -q "^sqlx" <<<"$LOG"; then ok "a current template is left alone"; else bad "a current template is left alone"; fi

for change in edit add remove rename; do
  setup; run # stamp the base set
  case "$change" in
    edit) echo "-- changed" >> "$TMP/m/20260102000000_b.sql" ;;
    add) echo "SELECT 1;" > "$TMP/m/20260103000000_c.sql" ;;
    remove) rm "$TMP/m/20260102000000_b.sql" ;;
    rename) mv "$TMP/m/20260102000000_b.sql" "$TMP/m/20260102000001_b.sql" ;;
  esac
  run
  if [[ $RC == 0 ]] && rebuilt; then ok "a migration $change rebuilds the template"; else bad "a migration $change rebuilds the template"; fi
done

# Stamped only AFTER the migrate succeeds: the COMMENT must follow the sqlx call in the log.
setup; run
order="$(grep -nE "^sqlx migrate run|COMMENT ON DATABASE template1 IS 'temper" <<<"$LOG" | cut -d: -f2 | cut -c1-4 | tr '\n' ' ')"
if [[ "$order" == "sqlx psql " ]]; then ok "the stamp is written after the migrate"; else bad "the stamp is written after the migrate (order: $order)"; fi

setup
echo 1 > "$TMP/sqlx_rc"
run
drops="$(grep -c "DROP DATABASE template1" <<<"$LOG" || true)"
if [[ $RC != 0 ]] && [[ "$drops" == 2 ]] && ! grep -q "temper sqlx-test-template" "$TMP/comment" 2>/dev/null; then
  ok "a failed migrate exits non-zero, restores pristine, and stamps nothing"
else
  bad "a failed migrate exits non-zero, restores pristine, and stamps nothing (drops: $drops)"
fi

setup
RUN_URL="postgresql://u:p@ep-cool-name.neon.tech/db?sslmode=require" run
RUN_URL=""
if [[ $RC == 2 ]] && [[ -z "$LOG" ]] && grep -q "refusing host" <<<"$OUT"; then ok "a non-local host is refused before any call"; else bad "a non-local host is refused before any call"; fi

setup; run; run --reset
if [[ $RC == 0 ]] && grep -q "DROP DATABASE template1" <<<"$LOG" && ! grep -q "^sqlx" <<<"$LOG" \
  && [[ "$(cat "$TMP/comment")" == "default template for new databases" ]]; then
  ok "--reset restores a pristine, unstamped template"
else
  bad "--reset restores a pristine, unstamped template"
fi

echo "  $pass passed, $fail failed"
[[ $fail == 0 ]]
