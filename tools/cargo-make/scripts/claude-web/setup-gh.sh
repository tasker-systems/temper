#!/usr/bin/env bash
# =============================================================================
# GitHub CLI Setup
# =============================================================================
#
# Installs the GitHub CLI (gh) for PR creation and CI interaction.
# Idempotent: skips if gh is already installed.
#
# Usage:
#   source tools/cargo-make/scripts/claude-web/setup-common.sh
#   source tools/cargo-make/scripts/claude-web/setup-gh.sh
#   setup_gh
#
# =============================================================================

set -euo pipefail

SETUP_LIB_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "${SETUP_LIB_DIR}/setup-common.sh"

setup_gh() {
  log_section "GitHub CLI"

  if command_exists gh; then
    log_ok "gh $(gh --version 2>/dev/null | head -1 | awk '{print $3}' || echo 'installed')"
    return 0
  fi

  if ! command_exists apt-get; then
    log_skip "apt-get not available — cannot install gh"
    return 0
  fi

  log_install "GitHub CLI (gh)"

  # Use the official GitHub CLI apt repository
  (
    curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg | \
      sudo dd of=/usr/share/keyrings/githubcli-archive-keyring.gpg 2>/dev/null
    sudo chmod go+r /usr/share/keyrings/githubcli-archive-keyring.gpg
    echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" | \
      sudo tee /etc/apt/sources.list.d/github-cli.list > /dev/null
    sudo apt-get update -qq 2>/dev/null
    sudo apt-get install -y -qq gh 2>/dev/null
  ) || log_warn "gh installation failed"

  if command_exists gh; then
    log_ok "gh installed"
  fi
}

# Run if executed directly
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  setup_gh
fi
