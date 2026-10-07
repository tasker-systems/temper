#!/usr/bin/env bash
# sqlx-test-template.sh — keep the test server's `template1` migrated to the current chain, so every
# `#[sqlx::test]` database starts migrated instead of applying ~270 migrations itself.
#
# WHY THIS WORKS WITH NO CALL-SITE CHANGE. sqlx 0.8 creates each test database with a bare
# `create database "<name>"` (sqlx-postgres/src/testing/mod.rs), and Postgres copies `template1`. When
# `template1` already holds the schema AND its `_sqlx_migrations` rows, the test's own migrator
# validates every checksum and applies nothing. Measured on a CI runner, same build, shard 2/3: the
# test phase went 272.8s -> 130.5s and 260.5s -> 153.6s (temper-artifacts, CI strategy research,
# Finding 10).
#
# WHAT KEEPS IT HONEST AS MIGRATIONS CHANGE. The template is stamped with a fingerprint of every
# migration file (names and bytes) as a database COMMENT. A refresh rebuilds whenever the fingerprint
# differs — a migration added, edited, renamed or removed — and never patches a template forward, so
# it can never hold an edited migration's old body. A rebuild is ~0.5s, which is why it does not try
# to be incremental. If a rebuild fails part-way, the template is put back to pristine before exiting:
# a half-migrated template must not outlive the run that made it.
#
# The template holds no test's writes: `CREATE DATABASE ... TEMPLATE` copies, and nothing connects to
# `template1` once it is built. It is migrated with plain `sqlx migrate run`, NOT `temper-migrate`,
# because a test database has never carried `kb_migration_ledger` rows and must not start to.
#
# Local hosts only: this drops and recreates `template1` on whatever server DATABASE_URL names.
#
# Usage:
#   sqlx-test-template.sh            rebuild template1 if it is not current (no-op when it is)
#   sqlx-test-template.sh --reset    put template1 back to pristine (opting out, locally)
#
# Env: DATABASE_URL (required, any database on the target server).
#      SQLX_TEST_TEMPLATE_MIGRATIONS (default: <repo>/migrations), SQLX_TEST_TEMPLATE_SQLX (default:
#      `sqlx` if on PATH, else `cargo sqlx`) — overridable for the harness.
#
# Harness: .github/scripts/test-sqlx-test-template.sh

set -euo pipefail

MODE="refresh"
case "${1:-}" in
  "") ;;
  --reset) MODE="reset" ;;
  *) echo "usage: $0 [--reset]" >&2; exit 2 ;;
esac

: "${DATABASE_URL:?DATABASE_URL must name a database on the test server}"
MIGRATIONS="${SQLX_TEST_TEMPLATE_MIGRATIONS:-$(git rev-parse --show-toplevel)/migrations}"
if [[ -n "${SQLX_TEST_TEMPLATE_SQLX:-}" ]]; then
  read -r -a SQLX <<<"$SQLX_TEST_TEMPLATE_SQLX"
elif command -v sqlx >/dev/null 2>&1; then
  SQLX=(sqlx)
else
  SQLX=(cargo sqlx)
fi

# postgres[ql]://<authority>/<database>[?<params>]
if [[ ! "$DATABASE_URL" =~ ^(postgres(ql)?://([^/@]*@)?([^/:?]+|\[[^]]+\])(:[0-9]+)?)/[^?]*(\?.*)?$ ]]; then
  echo "::error::sqlx-test-template: cannot parse DATABASE_URL" >&2
  exit 2
fi
SERVER="${BASH_REMATCH[1]}"
HOST="${BASH_REMATCH[4]}"
PARAMS="${BASH_REMATCH[6]}"
case "$HOST" in
  localhost | 127.0.0.1 | "[::1]") ;;
  *)
    echo "::error::sqlx-test-template: refusing host '$HOST' — this drops and recreates template1, local test servers only" >&2
    exit 2
    ;;
esac
ADMIN_URL="$SERVER/postgres$PARAMS"
TEMPLATE_URL="$SERVER/template1$PARAMS"

admin() { psql "$ADMIN_URL" -v ON_ERROR_STOP=1 -Atqc "$1"; }

fingerprint() {
  local f hash=(shasum -a 256)
  command -v sha256sum >/dev/null 2>&1 && hash=(sha256sum)
  [[ -d "$MIGRATIONS" ]] || { echo "::error::sqlx-test-template: no migrations directory at $MIGRATIONS" >&2; exit 2; }
  (
    cd "$MIGRATIONS"
    shopt -s nullglob
    for f in *.sql; do
      printf '%s\0' "$f"
      cat -- "$f"
      printf '\0'
    done
  ) | "${hash[@]}" | cut -d' ' -f1
}

pristine() {
  admin "ALTER DATABASE template1 IS_TEMPLATE false"
  admin "DROP DATABASE template1"
  admin "CREATE DATABASE template1 TEMPLATE template0 IS_TEMPLATE true"
  admin "COMMENT ON DATABASE template1 IS 'default template for new databases'"
}

if [[ "$MODE" == "reset" ]]; then
  pristine
  echo "template1 reset to pristine"
  exit 0
fi

MARKER="temper sqlx-test-template sha256:$(fingerprint)"
CURRENT="$(admin "SELECT coalesce(shobj_description(oid, 'pg_database'), '') FROM pg_database WHERE datname = 'template1'")"
if [[ "$CURRENT" == "$MARKER" ]]; then
  echo "template1 is current (${MARKER##*:})"
  exit 0
fi

echo "template1 is stale (${CURRENT:-unstamped}); rebuilding"
pristine
trap 'echo "::error::sqlx-test-template: migrating template1 failed; putting it back to pristine" >&2; pristine' ERR
"${SQLX[@]}" migrate run --source "$MIGRATIONS" --database-url "$TEMPLATE_URL" >/dev/null
trap - ERR
# Stamped LAST: an interrupted rebuild leaves no marker, so the next refresh rebuilds again.
admin "COMMENT ON DATABASE template1 IS '$MARKER'"
APPLIED="$(psql "$TEMPLATE_URL" -v ON_ERROR_STOP=1 -Atqc "SELECT count(*) FROM _sqlx_migrations WHERE success")"
echo "template1 rebuilt: $APPLIED migrations applied (${MARKER##*:})"
