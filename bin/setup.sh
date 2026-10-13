#!/usr/bin/env bash
#
# setup.sh — one-shot, idempotent dev/admin onboarding for a temper checkout.
#
# Takes a fresh clone to "able to run `cargo make check` + `cargo make test-db`": brew deps, the
# tools pinned in mise.toml, git hooks, a running Docker Postgres, and migrations applied to the
# dev database. Re-running converges (each step is skip-if-present), it does not re-install.
#
# This is for CONTRIBUTORS and OPERATORS working from a source checkout. End-users who just want
# the `temper` binary should use scripts/install/install.sh (release download) instead.
#
# Usage:
#   bin/setup.sh [--with-cli] [--dry-run]
#
#   --with-cli   also `cargo install --path crates/temper-cli --locked` so `temper` is on your PATH
#                (built from this checkout — the latest local CLI, incl. the bootstrap applier).
#   --dry-run    print what each step would do without executing.
#
# Platform: macOS-first (Homebrew). On Linux it prints the dependency pointers and exits without
# installing — wire up your package manager from that list (the mise + docker steps are identical).
#
# Full walk-through + troubleshooting: internal/development/development.md
set -euo pipefail

WITH_CLI=0
DRY_RUN=0

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DATABASE_URL_DEFAULT="postgresql://temper:temper@localhost:5437/temper_development"

die()  { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }
info() { printf '\033[1m==>\033[0m %s\n' "$*"; }
skip() { printf '    \033[2m✓ %s\033[0m\n' "$*"; }
have() { command -v "$1" >/dev/null 2>&1; }

# Run a command, honoring --dry-run.
run() {
  if [ "$DRY_RUN" -eq 1 ]; then
    printf '    (dry-run) %s\n' "$*"
  else
    "$@"
  fi
}

while [ $# -gt 0 ]; do
  case "$1" in
    --with-cli) WITH_CLI=1; shift ;;
    --dry-run)  DRY_RUN=1; shift ;;
    -h|--help)  sed -n '2,30p' "$0"; exit 0 ;;
    *)          die "unknown argument: $1 (try --help)" ;;
  esac
done

cd "$REPO_ROOT"

# ── Linux: print pointers and stop (unvalidated; wire up your own package manager) ────────────────
if [ "$(uname -s)" = "Linux" ]; then
  cat <<'EOF'
==> Linux is not auto-provisioned by this script (macOS-first). Install these, then run the
    mise + docker steps below by hand. (Unvalidated pointers — adjust for your distro.)

  System packages (apt example):
    sudo apt-get install -y libpq-dev postgresql-client docker.io docker-compose-plugin
    # ONNX Runtime: download from https://github.com/microsoft/onnxruntime/releases
    #   (or set ORT_DYLIB_PATH).

  Rust: https://rustup.rs (mise builds sqlx-cli with cargo, so install Rust first).

  mise, then everything pinned in mise.toml (runtimes, lint binaries, cargo tooling, and the
  code-intelligence tools rust-analyzer + scip). Same on every platform:
    curl https://mise.run | sh        # or your package manager; see https://mise.jdx.dev
    mise trust && mise install        # from the checkout root
    # Activate it so the pinned tools win on PATH. internal/development/development.md has the
    # rc-file lines, including the --shims form that non-interactive shells (agents, IDEs) need.

  Then:
    git config core.hooksPath "$(git rev-parse --show-toplevel)/githooks"
    docker compose up -d
    DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development mise exec -- sqlx migrate run
    cargo make check && cargo make test-db
EOF
  exit 0
fi

[ "$(uname -s)" = "Darwin" ] || die "unsupported platform: $(uname -s) (macOS-first; see --help)"

# ── 1. Homebrew dependencies ──────────────────────────────────────────────────────────────────────
info "Homebrew packages (Brewfile)"
have brew || die "Homebrew not found — install it first: https://brew.sh"
run brew bundle --file="$REPO_ROOT/Brewfile"

# psql ships from the keg-only `libpq`; surface the PATH export if it isn't linked.
if ! have psql; then
  libpq_bin="$(brew --prefix 2>/dev/null)/opt/libpq/bin"
  if [ -x "$libpq_bin/psql" ]; then
    info "psql is installed via keg-only libpq but not on PATH. Add to your shell profile:"
    # shellcheck disable=SC2016  # $PATH is a literal we want printed verbatim for the user to paste
    printf '      export PATH="%s:$PATH"\n' "$libpq_bin"
  else
    info "psql not found — install it (brew install libpq) for the org-bootstrap root step."
  fi
else
  skip "psql present"
fi

# ── 2. Pinned tooling (mise.toml) ─────────────────────────────────────────────────────────────────
#
# One `mise install` covers the language runtimes, the lint binaries, the cargo tooling (cargo-make,
# cargo-nextest, sqlx-cli) and the code-intelligence tools (rust-analyzer, scip). The versions and
# the reasons for them live in mise.toml. sqlx-cli is built with cargo, so Rust must come first.
info "Pinned tooling (mise.toml)"
have cargo || die "cargo not found — install Rust: https://rustup.rs"
have mise  || die "mise not found — 'brew bundle' (step 1) installs it"
run mise trust "$REPO_ROOT/mise.toml"
run mise install

# Installed is not the same as on PATH: mise's tools win only in a shell that activates mise.
# Both forms of activation resolve python3 to something under mise's data dir (an install or a shim).
mise_data_dir="${MISE_DATA_DIR:-${XDG_DATA_HOME:-$HOME/.local/share}/mise}"
case "$(command -v python3 2>/dev/null)" in
  "$mise_data_dir"/*) skip "mise is activated in this shell (python3 → $(command -v python3))" ;;
  *)
    info "mise is not activated in this shell, so the pinned tools are not on your PATH yet."
    # shellcheck disable=SC2016  # literals we want printed verbatim for the user to paste
    printf '      add to ~/.zshrc:     eval "$(mise activate zsh)"\n'
    # shellcheck disable=SC2016
    printf '      add to ~/.zprofile:  eval "$(mise activate zsh --shims)"   # agents, IDEs, non-interactive shells\n'
    ;;
esac

# ── 3. Git hooks ──────────────────────────────────────────────────────────────────────────────────
info "Git hooks"
run bash "$REPO_ROOT/scripts/install-hooks.sh"

# ── 4. Docker Postgres ────────────────────────────────────────────────────────────────────────────
info "Docker Postgres (docker compose up -d)"
have docker || die "docker not found — install Docker Desktop: https://www.docker.com/products/docker-desktop"
run docker compose up -d
if [ "$DRY_RUN" -eq 0 ]; then
  printf '    waiting for postgres to accept connections'
  for _ in $(seq 1 30); do
    if docker compose exec -T temper-postgres pg_isready -U temper -d temper_development >/dev/null 2>&1; then
      printf ' ready\n'; break
    fi
    printf '.'; sleep 1
  done
fi

# ── 5. Migrations on the dev database ─────────────────────────────────────────────────────────────
info "Apply migrations (sqlx migrate run)"
export DATABASE_URL="${DATABASE_URL:-$DATABASE_URL_DEFAULT}"
run mise exec -- sqlx migrate run --source "$REPO_ROOT/migrations"

# ── 6. Code-intelligence (SCIP) tooling ───────────────────────────────────────────────────────────
#
# Installed by step 2 from mise.toml, which says why rust-analyzer is a dated upstream release and not
# the rustup component. This step only checks the result, and is non-fatal: someone who just wants
# `cargo make check` to run must not be blocked here.
info "Code-intelligence tooling (SCIP)"
if [ "$DRY_RUN" -eq 1 ]; then
  printf '    (dry-run) check rust-analyzer + scip from mise\n'
else
  # `mise exec` puts mise's install dirs first, so a rustup rust-analyzer shim cannot answer here.
  if ra_ver="$(mise exec -- rust-analyzer --version 2>/dev/null)"; then
    skip "rust-analyzer present ($ra_ver)"
  else
    info "rust-analyzer not runnable via mise — re-run 'mise install' and check its output."
  fi
  if scip_ver="$(mise exec -- scip --version 2>/dev/null)" && [ "${scip_ver#scip version}" != "$scip_ver" ]; then
    skip "scip CLI present ($scip_ver)"
  else
    info "scip CLI not runnable via mise — re-run 'mise install' and check its output."
  fi
  # A different `scip` earlier on PATH would shadow mise's in shells that don't activate mise.
  if have scip && ! scip --version 2>/dev/null | grep -q '^scip version'; then
    info "a DIFFERENT 'scip' is on your PATH (likely the SCIP Optimization Suite, a MIP solver)."
    info "  Sourcegraph's scip is a different product. Uninstall that one or order your PATH."
  fi
fi

# scip-typescript needs no install step — invoked as `bunx @sourcegraph/scip-typescript`.
skip "scip-typescript via bunx (no install needed)"

# ── 7. (optional) Install the temper CLI from this checkout ───────────────────────────────────────
if [ "$WITH_CLI" -eq 1 ]; then
  info "Install temper CLI from checkout (--with-cli)"
  # --locked: install against the committed Cargo.lock. Without it, `cargo install`
  # re-resolves dependencies and can pull a semver-compatible-but-broken upstream combo
  # (e.g. time 0.3.52 breaks cookie 0.18.1), failing the build. The lock is the tested set.
  run cargo install --path "$REPO_ROOT/crates/temper-cli" --locked --force
fi

info "Setup complete. Verify with:  cargo make check  &&  cargo make test-db"
