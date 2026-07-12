#!/usr/bin/env bash
#
# setup.sh — one-shot, idempotent dev/admin onboarding for a temper checkout.
#
# Takes a fresh clone to "able to run `cargo make check` + `cargo make test-db`": brew deps,
# the cargo-based tooling, git hooks, a running Docker Postgres, and migrations applied to the
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
# installing — wire up your package manager from that list (the cargo + docker steps are identical).
#
# Full walk-through + troubleshooting: docs/guides/development.md
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
    cargo + docker steps below by hand. (Unvalidated pointers — adjust for your distro.)

  System packages (apt example):
    sudo apt-get install -y libpq-dev postgresql-client docker.io docker-compose-plugin
    # ONNX Runtime: download from https://github.com/microsoft/onnxruntime/releases
    #   (or set ORT_DYLIB_PATH); actionlint + shellcheck + yq from your package manager.

  Cargo tooling (same as macOS):
    cargo install cargo-make cargo-nextest sqlx-cli

  Code-intelligence (SCIP) tooling — same on every platform, none of it is a package-manager line:
    # rust-analyzer is a rustup COMPONENT (stays version-matched to your toolchain):
    rustup component add rust-analyzer

    # scip CLI — no distro package, and `go install` is broken upstream (replace directives in
    # their go.mod). Fetch the pinned release binary and verify its published sha256:
    ver=v0.9.0; arch=$(uname -m); case "$arch" in x86_64) arch=amd64 ;; aarch64) arch=arm64 ;; esac
    base=https://github.com/sourcegraph/scip/releases/download/$ver/scip-linux-$arch.tar.gz
    curl -sSfL -o /tmp/scip.tar.gz "$base" && curl -sSfL -o /tmp/scip.sha256 "$base.sha256"
    (cd /tmp && sha256sum -c scip.sha256) && tar xzf /tmp/scip.tar.gz -C /tmp \
      && install -m755 /tmp/scip "$HOME/.local/bin/scip"   # ensure ~/.local/bin is on PATH

    # scip-typescript needs no install — it is invoked via `bunx @sourcegraph/scip-typescript`.

    # DO NOT `apt install scip` / `brew install scip`: that name belongs to the SCIP Optimization
    # Suite (a mixed-integer-programming solver), an unrelated product. See the Brewfile.

  Then:
    git config core.hooksPath "$(git rev-parse --show-toplevel)/githooks"
    docker compose up -d
    DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development sqlx migrate run
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

# ── 2. Cargo-based tooling (not brew formulae) ────────────────────────────────────────────────────
info "Cargo tooling (cargo-make, cargo-nextest, sqlx-cli)"
have cargo || die "cargo not found — install Rust: https://rustup.rs"
if have cargo-make;    then skip "cargo-make present";    else run cargo install cargo-make; fi
if have cargo-nextest; then skip "cargo-nextest present"; else run cargo install cargo-nextest; fi
if have sqlx; then
  skip "sqlx-cli present"
else
  run cargo install sqlx-cli --no-default-features --features native-tls,postgres
fi

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
run sqlx migrate run --source "$REPO_ROOT/migrations"

# ── 6. Code-intelligence (SCIP) tooling ───────────────────────────────────────────────────────────
#
# Feeds the code-graph work (goal: trunk-change awareness). None of these is a brew formula, and one
# of them is an outright name trap — see the Brewfile for the full story. Non-fatal: a network hiccup
# here must not block someone who just wants `cargo make check` to run.
info "Code-intelligence tooling (SCIP)"

# rust-analyzer is a rustup COMPONENT, not a brew formula — this keeps it version-matched to the
# toolchain the repo builds with. Note `command -v rust-analyzer` is NOT a sufficient check: rustup
# installs a shim that exists on PATH and then fails at runtime if the component is absent.
if rust-analyzer --version >/dev/null 2>&1; then
  skip "rust-analyzer present ($(rust-analyzer --version 2>/dev/null))"
else
  run rustup component add rust-analyzer || info "rust-analyzer component install failed — 'rustup component add rust-analyzer' by hand"
fi

# scip CLI — pinned release binary + published-checksum verification. There is no brew formula
# (`brew install scip` gets you a mixed-integer-programming solver), and `go install` is broken
# upstream by replace directives in their go.mod.
SCIP_CLI_VERSION="v0.9.0"
if have scip && scip --version 2>/dev/null | grep -q '^scip version'; then
  skip "scip CLI present ($(scip --version 2>/dev/null))"
elif have scip; then
  info "a DIFFERENT 'scip' is on your PATH (likely the SCIP Optimization Suite, a MIP solver)."
  info "  Sourcegraph's scip is a different product. Install it elsewhere and order your PATH."
elif [ "$DRY_RUN" -eq 1 ]; then
  printf '    (dry-run) download + verify scip %s into ~/.local/bin\n' "$SCIP_CLI_VERSION"
else
  scip_arch="$(uname -m)"; [ "$scip_arch" = "x86_64" ] && scip_arch="amd64"
  scip_url="https://github.com/sourcegraph/scip/releases/download/${SCIP_CLI_VERSION}/scip-darwin-${scip_arch}.tar.gz"
  scip_tmp="$(mktemp -d)"
  if curl -sSfL -o "$scip_tmp/scip.tar.gz" "$scip_url" \
     && curl -sSfL -o "$scip_tmp/scip.sha256" "${scip_url}.sha256" \
     && [ "$(shasum -a 256 "$scip_tmp/scip.tar.gz" | cut -d' ' -f1)" = "$(cut -d' ' -f1 < "$scip_tmp/scip.sha256")" ] \
     && tar xzf "$scip_tmp/scip.tar.gz" -C "$scip_tmp"; then
    mkdir -p "$HOME/.local/bin"
    install -m 755 "$scip_tmp/scip" "$HOME/.local/bin/scip"
    info "installed scip ${SCIP_CLI_VERSION} → ~/.local/bin/scip"
    # shellcheck disable=SC2016  # $HOME/$PATH are literals we want printed verbatim to paste
    have scip || printf '      (add to your shell profile: export PATH="$HOME/.local/bin:$PATH")\n'
  else
    info "scip CLI install failed (download or checksum) — fetch it by hand from"
    info "  https://github.com/sourcegraph/scip/releases/tag/${SCIP_CLI_VERSION}"
  fi
  rm -rf "$scip_tmp"
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
