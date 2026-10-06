#!/usr/bin/env bash
# SPIKE ONLY — never merged. Task 01a10ec4-c7bb-70f0-9d32-78e13976bd17.
#
# Measures what `#[sqlx::test]` pays per test with a pristine vs a migrated `template1`.
# sqlx 0.8.6 creates each test database with a bare `create database "<name>"`, so Postgres copies
# `template1`. If `template1` already carries the migrated schema (and `_sqlx_migrations`), the
# test's own `Migrator::run_direct` finds every migration applied and applies nothing.
set -euo pipefail

PGURL_BASE="postgresql://temper:temper@localhost:${SPIKE_PGPORT:-5437}"
q() { psql "$PGURL_BASE/postgres" -v ON_ERROR_STOP=1 -Atqc "$1"; }
now_ms() { perl -MTime::HiRes=time -e 'printf "%d\n", time*1000'; }

reset_template1() {
  q "ALTER DATABASE template1 IS_TEMPLATE false"
  q "DROP DATABASE template1"
  q "CREATE DATABASE template1 TEMPLATE template0 IS_TEMPLATE true"
}

migrate_template1() {
  local t0; t0=$(now_ms)
  cargo sqlx migrate run --source migrations --database-url "$PGURL_BASE/template1" >/dev/null
  echo "migrate template1: $(( $(now_ms) - t0 )) ms"
}

# Median and mean of the numbers on stdin.
stats() { sort -n | awk '{a[NR]=$1; s+=$1} END {printf "n=%d median=%d mean=%.1f min=%d max=%d\n", NR, a[int((NR+1)/2)], s/NR, a[1], a[NR]}'; }

# CREATE DATABASE timings, server-side (psql \timing), excluding connection setup.
bench_create() {
  local n=$1
  {
    echo '\timing on'
    for i in $(seq 1 "$n"); do
      echo "CREATE DATABASE bench_$i;"
      echo "DROP DATABASE bench_$i;"
    done
  } | psql "$PGURL_BASE/postgres" -v ON_ERROR_STOP=1 -q 2>&1 \
    | awk '/^Time:/ {n++; if (n % 2 == 1) print int($2)}' | stats
}

# What a test's migrator costs on a database copied from the current template1: the full chain on a
# pristine one, nothing-to-apply on a migrated one. sqlx-cli runs the same `Migrator::run` the test
# harness does, plus process start-up, so this is an upper bound on the harness's share.
bench_migrate() {
  local n=$1
  for i in $(seq 1 "$n"); do
    q "CREATE DATABASE bench_m_$i"
    local t0; t0=$(now_ms)
    cargo sqlx migrate run --source migrations --database-url "$PGURL_BASE/bench_m_$i" >/dev/null
    echo $(( $(now_ms) - t0 ))
    q "DROP DATABASE bench_m_$i"
  done | stats
}

micro() {
  echo "== postgres: $(q 'SELECT version()')"
  echo "== durability: fsync=$(q 'SHOW fsync') synchronous_commit=$(q 'SHOW synchronous_commit') full_page_writes=$(q 'SHOW full_page_writes')"
  echo "== migrations on disk: $(ls migrations/*.sql | wc -l)"
  echo "== cargo sqlx no-op (process start + connect), against temper_development:"
  for i in 1 2 3 4 5; do
    t0=$(now_ms); cargo sqlx migrate run --source migrations >/dev/null; echo $(( $(now_ms) - t0 ))
  done | stats

  reset_template1
  echo "== PRISTINE template1 ($(q "SELECT pg_size_pretty(pg_database_size('template1'))"))"
  echo -n "create+drop db (create ms): "; bench_create 30
  echo -n "full migrate on fresh db (ms): "; bench_migrate 10

  migrate_template1
  echo "== MIGRATED template1 ($(q "SELECT pg_size_pretty(pg_database_size('template1'))"), $(psql "$PGURL_BASE/template1" -Atqc 'SELECT count(*) FROM _sqlx_migrations') applied)"
  echo -n "create+drop db (create ms): "; bench_create 30
  echo -n "no-op migrate on template copy (ms): "; bench_migrate 10

  reset_template1
}

run_arm() {
  local arm=$1
  reset_template1
  if [ "$arm" = template ]; then migrate_template1; fi
  echo "== ARM $arm: template1 has $(psql "$PGURL_BASE/template1" -Atqc "SELECT count(*) FROM pg_tables WHERE schemaname NOT IN ('pg_catalog','information_schema')") tables"
  local t0 rc=0; t0=$(now_ms)
  local sel=${SPIKE_SELECTION:-p2}
  local -a cmd
  case "$sel" in
    p[123]) cmd=(--workspace --exclude temper-cloud --features test-db,test-embed --partition "count:${sel#p}/3") ;;
    artifacts) cmd=(-p temper-substrate --features artifact-tests) ;;
  esac
  cargo nextest run "${cmd[@]}" --profile ci --locked --no-fail-fast 2>&1 | tee "nextest-$arm.log" | grep -E '^\s*(Summary|FAIL|FLAKY|TIMEOUT)' || rc=$?
  echo "ARM $arm wall: $(( $(now_ms) - t0 )) ms"
  mv target/nextest/ci/junit.xml "junit-$arm.xml"
}

case "${1:?usage: tpl.sh micro|fresh|template}" in
  micro) micro ;;
  fresh|template) run_arm "$1" ;;
esac
