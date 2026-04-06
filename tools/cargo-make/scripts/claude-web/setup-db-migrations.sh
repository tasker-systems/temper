#!/usr/bin/env bash
# =============================================================================
# Database Migrations Setup
# =============================================================================
#
# Runs sqlx migrations against development and test databases.
# Should be called after PostgreSQL is ready.
#
# Usage:
#   source tools/cargo-make/scripts/claude-web/setup-common.sh
#   source tools/cargo-make/scripts/claude-web/setup-db-migrations.sh
#   PG_READY=true setup_db_migrations
#
# =============================================================================

set -euo pipefail

SETUP_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SETUP_LIB_DIR}/setup-common.sh"

setup_db_migrations() {
  if [ "${PG_READY:-false}" != "true" ]; then
    log_skip "PostgreSQL not ready — skipping migrations"
    return 0
  fi

  if ! command_exists sqlx; then
    log_skip "sqlx-cli not installed — skipping migrations"
    return 0
  fi

  log_section "Database migrations"

  local pg_port="${PG_PORT:-5437}"

  cd "${PROJECT_DIR}"

  # Run migrations on development database
  local dev_url="postgresql://temper:temper@localhost:${pg_port}/temper_development"
  export DATABASE_URL="$dev_url"

  sqlx database create 2>/dev/null || true

  if sqlx migrate run 2>/dev/null; then
    log_ok "migrations applied (temper_development)"
  else
    log_warn "migrations failed on temper_development"
  fi

  # Run migrations on test database
  local test_url="postgresql://temper:temper@localhost:${pg_port}/temper_test"
  export DATABASE_URL="$test_url"

  sqlx database create 2>/dev/null || true

  if sqlx migrate run 2>/dev/null; then
    log_ok "migrations applied (temper_test)"
  else
    log_warn "migrations failed on temper_test"
  fi

  # Reset DATABASE_URL to development
  export DATABASE_URL="$dev_url"
}

# Run if executed directly
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  setup_db_migrations
fi
