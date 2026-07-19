#!/usr/bin/env bash
# =============================================================================
# Temper — Full Claude Code on the Web Environment Setup
# =============================================================================
#
# Installs all tools and starts all services needed for development and testing.
# This is the "heavy" counterpart to setup-claude-web.sh (the SessionStart hook).
#
# Run this manually when you need full capabilities:
#   ./tools/bin/setup-claude-web-full.sh
#   FORCE_SETUP=1 ./tools/bin/setup-claude-web-full.sh  # Outside remote env
#
# Individual components can also be installed separately:
#   source tools/cargo-make/scripts/claude-web/setup-common.sh
#   source tools/cargo-make/scripts/claude-web/setup-postgres.sh && setup_postgres
#   source tools/cargo-make/scripts/claude-web/setup-gh.sh && setup_gh
#
# =============================================================================

set -euo pipefail

# ---------------------------------------------------------------------------
# Environment Guard
# ---------------------------------------------------------------------------
if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ] && [ "${FORCE_SETUP:-}" != "1" ]; then
  echo "Not in Claude Code remote environment. Set FORCE_SETUP=1 to override."
  exit 0
fi

# ---------------------------------------------------------------------------
# Bootstrap
# ---------------------------------------------------------------------------
PROJECT_DIR="${CLAUDE_PROJECT_DIR:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
LIB_DIR="${PROJECT_DIR}/tools/cargo-make/scripts/claude-web"

export PATH="$HOME/.local/bin:$HOME/.cargo/bin:$PATH"

source "${LIB_DIR}/setup-common.sh"

echo ""
echo "==> Full setup: temper for Claude Code on the web"
echo "  Project: $PROJECT_DIR"

# ---------------------------------------------------------------------------
# Phase 1: Run the lightweight SessionStart setup first
# ---------------------------------------------------------------------------
FORCE_SETUP=1 "$PROJECT_DIR/tools/bin/setup-claude-web.sh"

# ---------------------------------------------------------------------------
# Phase 2: System-level dependencies
# ---------------------------------------------------------------------------
source "${LIB_DIR}/setup-system-deps.sh"
setup_system_deps

# ---------------------------------------------------------------------------
# Phase 3: Rust toolchain and cargo tools
# ---------------------------------------------------------------------------
source "${LIB_DIR}/setup-rust.sh"
setup_rust

source "${LIB_DIR}/setup-cargo-tools.sh"
setup_cargo_tools

# ---------------------------------------------------------------------------
# Phase 4: Optional tools
# ---------------------------------------------------------------------------
source "${LIB_DIR}/setup-gh.sh"
setup_gh

# ---------------------------------------------------------------------------
# Phase 5: PostgreSQL (Docker with pgvector)
# ---------------------------------------------------------------------------
source "${LIB_DIR}/setup-postgres.sh"
setup_postgres

# ---------------------------------------------------------------------------
# Phase 6: Database migrations
# ---------------------------------------------------------------------------
source "${LIB_DIR}/setup-db-migrations.sh"
setup_db_migrations

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
log_section "Full setup complete"

echo ""
echo "  Tools installed:"
command_exists cargo      && echo "    cargo:         $(cargo --version 2>/dev/null | awk '{print $2}' || echo 'yes')"
command_exists cargo-make && echo "    cargo-make:    yes"
command_exists sqlx       && echo "    sqlx-cli:      yes"
command_exists cargo-nextest && echo "    cargo-nextest: yes"
command_exists gh         && echo "    gh:            $(gh --version 2>/dev/null | head -1 | awk '{print $3}' || echo 'yes')"
command_exists rtk        && echo "    rtk:           $(rtk --version 2>/dev/null | awk '{print $2}' || echo 'yes')"
echo ""

echo "  Services:"
if pg_isready -h localhost -p "${PG_PORT:-5437}" -q 2>/dev/null; then
  echo "    PostgreSQL:    ready (port ${PG_PORT:-5437})"
else
  echo "    PostgreSQL:    NOT available (compilation will use .sqlx/ cache)"
fi
echo ""

echo "  Quick start:"
echo "    cargo make check    # Run all quality checks"
echo "    cargo make build    # Build everything"
if [ "${PG_READY:-false}" = "true" ]; then
  echo "    cargo make test     # Run tests"
  echo "    cargo make test-db  # Run database integration tests"
fi
echo ""
