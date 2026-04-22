#!/usr/bin/env bash
# =============================================================================
# PostgreSQL Setup (Docker pgvector + native fallback)
# =============================================================================
#
# Handles PostgreSQL setup for temper development/testing:
#   1. Start PostgreSQL (Docker preferred with pgvector, native fallback)
#   2. Create temper role and databases (development + test)
#   3. Install pgvector extension
#
# Temper uses PostgreSQL 18 with pgvector on port 5437 (avoids conflicts
# with tasker-core on 5432).
#
# Outputs:
#   Sets PG_READY=true/false indicating whether PostgreSQL is available.
#   Sets PSQL_CMD to the connection command string.
#
# Usage:
#   source tools/cargo-make/scripts/claude-web/setup-common.sh
#   source tools/cargo-make/scripts/claude-web/setup-postgres.sh
#   setup_postgres
#
# =============================================================================

set -euo pipefail

SETUP_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SETUP_LIB_DIR}/setup-common.sh"

PG_READY=false
PSQL_CMD=""

# Temper's standard port and credentials
PG_PORT="${TEMPER_PG_PORT:-5437}"
PG_USER="temper"
PG_PASSWORD="temper"
PG_DB_DEV="temper_development"
PG_DB_TEST="temper_test"

# ---------------------------------------------------------------------------
# Strategy 1: Docker (preferred — gives us pg18 + pgvector)
# ---------------------------------------------------------------------------
setup_postgres_docker() {
  if ! command_exists docker; then
    return 1
  fi

  if ! docker info >/dev/null 2>&1; then
    return 1
  fi

  local compose_file="${PROJECT_DIR}/docker-compose.yml"
  if [ ! -f "$compose_file" ]; then
    return 1
  fi

  echo "  Docker available, starting PostgreSQL with pgvector via docker-compose..."
  docker compose -f "$compose_file" up -d 2>/dev/null || \
    docker-compose -f "$compose_file" up -d 2>/dev/null || \
    return 1

  # Wait for readiness (up to 30 seconds)
  echo "  Waiting for PostgreSQL on port $PG_PORT..."
  local retries=30
  while [ $retries -gt 0 ]; do
    if pg_isready -h localhost -p "$PG_PORT" -U "$PG_USER" -q 2>/dev/null; then
      PSQL_CMD="PGPASSWORD=$PG_PASSWORD psql -h localhost -p $PG_PORT -U $PG_USER"
      log_ok "PostgreSQL ready (Docker, pg18 + pgvector, port $PG_PORT)"
      return 0
    fi
    retries=$((retries - 1))
    sleep 1
  done

  log_warn "PostgreSQL Docker container started but not ready"
  return 1
}

# ---------------------------------------------------------------------------
# Install the matching `postgresql-<major>-pgvector` apt package if we're
# on Debian/Ubuntu and it isn't already present. No-op on other distros or
# when apt/sudo aren't available — the subsequent CREATE EXTENSION falls
# through to its own log_warn in that case.
# ---------------------------------------------------------------------------
install_pgvector_if_needed() {
  command_exists psql || return 0
  command_exists apt-get || return 0
  command_exists dpkg || return 0

  local pg_major
  pg_major=$(psql --version 2>/dev/null | awk '{print $3}' | cut -d. -f1)
  [ -n "$pg_major" ] || return 0

  local pkg="postgresql-${pg_major}-pgvector"
  if dpkg -s "$pkg" >/dev/null 2>&1; then
    return 0
  fi

  echo "  Installing ${pkg} for pgvector support..."
  DEBIAN_FRONTEND=noninteractive sudo -E apt-get install -y "$pkg" >/dev/null 2>&1 || \
    log_warn "Failed to install ${pkg} — pgvector will be unavailable"
}

# ---------------------------------------------------------------------------
# Strategy 2: Native PostgreSQL (web environment fallback)
# ---------------------------------------------------------------------------
setup_postgres_native() {
  if ! command_exists psql; then
    return 1
  fi

  # Try to start PostgreSQL if not running
  if ! pg_isready -q 2>/dev/null; then
    sudo service postgresql start 2>/dev/null || \
      sudo systemctl start postgresql 2>/dev/null || \
      true

    local retries=10
    while [ $retries -gt 0 ]; do
      pg_isready -q 2>/dev/null && break
      retries=$((retries - 1))
      sleep 1
    done
  fi

  if ! pg_isready -q 2>/dev/null; then
    return 1
  fi

  echo "  Native PostgreSQL is running"

  # Determine superuser connection method
  local psql_super=""
  if sudo -u postgres psql -c "SELECT 1" >/dev/null 2>&1; then
    psql_super="sudo -u postgres psql"
  elif psql -U postgres -c "SELECT 1" >/dev/null 2>&1; then
    psql_super="psql -U postgres"
  elif psql -c "SELECT 1" >/dev/null 2>&1; then
    psql_super="psql"
  else
    log_warn "Cannot connect to PostgreSQL as superuser"
    return 1
  fi

  # Create temper role if it doesn't exist
  if ! $psql_super -tAc "SELECT 1 FROM pg_roles WHERE rolname='$PG_USER'" 2>/dev/null | grep -q 1; then
    $psql_super -c "CREATE ROLE $PG_USER WITH LOGIN PASSWORD '$PG_PASSWORD' SUPERUSER;" 2>/dev/null || true
  fi

  # Create development database
  if ! $psql_super -tAc "SELECT 1 FROM pg_database WHERE datname='$PG_DB_DEV'" 2>/dev/null | grep -q 1; then
    $psql_super -c "CREATE DATABASE $PG_DB_DEV OWNER $PG_USER;" 2>/dev/null || true
  fi

  # Create test database
  if ! $psql_super -tAc "SELECT 1 FROM pg_database WHERE datname='$PG_DB_TEST'" 2>/dev/null | grep -q 1; then
    $psql_super -c "CREATE DATABASE $PG_DB_TEST OWNER $PG_USER;" 2>/dev/null || true
  fi

  # Install pgvector extension. On Debian/Ubuntu the extension is packaged
  # separately as `postgresql-<major>-pgvector` — install it if missing
  # before we try to CREATE EXTENSION, otherwise every web session hits
  # `extension "vector" is not available` and migrations fail.
  install_pgvector_if_needed

  $psql_super -d "$PG_DB_DEV" -c "CREATE EXTENSION IF NOT EXISTS vector;" 2>/dev/null || \
    log_warn "pgvector extension not available — vector search will fail"
  $psql_super -d "$PG_DB_TEST" -c "CREATE EXTENSION IF NOT EXISTS vector;" 2>/dev/null || true

  # Native PG runs on default port 5432, override the temper port
  PG_PORT="5432"
  PSQL_CMD="PGPASSWORD=$PG_PASSWORD psql -h localhost -p $PG_PORT -U $PG_USER"

  log_ok "PostgreSQL configured (native, port $PG_PORT)"
  return 0
}

# ---------------------------------------------------------------------------
# Main entry point
# ---------------------------------------------------------------------------
setup_postgres() {
  log_section "PostgreSQL database"

  # Try Docker first (preferred: pg18 + pgvector), then native
  if setup_postgres_docker; then
    PG_READY=true

    # Create test database if it doesn't exist (Docker only creates dev DB)
    PGPASSWORD="$PG_PASSWORD" psql -h localhost -p "$PG_PORT" -U "$PG_USER" -d "$PG_DB_DEV" \
      -tAc "SELECT 1 FROM pg_database WHERE datname='$PG_DB_TEST'" 2>/dev/null | grep -q 1 || \
      PGPASSWORD="$PG_PASSWORD" psql -h localhost -p "$PG_PORT" -U "$PG_USER" -d "$PG_DB_DEV" \
        -c "CREATE DATABASE $PG_DB_TEST OWNER $PG_USER;" 2>/dev/null || true

    # Ensure pgvector in test database
    PGPASSWORD="$PG_PASSWORD" psql -h localhost -p "$PG_PORT" -U "$PG_USER" -d "$PG_DB_TEST" \
      -c "CREATE EXTENSION IF NOT EXISTS vector;" 2>/dev/null || true

    return 0
  fi

  if setup_postgres_native; then
    PG_READY=true
    return 0
  fi

  log_warn "PostgreSQL not available — database tests will fail"
  log_warn "Compilation will still work using the SQLx offline query cache (.sqlx/)"
  return 0
}

# Run if executed directly
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  setup_postgres
  echo ""
  echo "PG_READY=$PG_READY"
  echo "PG_PORT=$PG_PORT"
fi
