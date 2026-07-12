# Development dependencies for temper.
# Install with `brew bundle`, or run `bin/setup.sh` for the full dev/admin bootstrap.
# These are the brew-installable deps; the cargo-based tools (cargo-make, cargo-nextest,
# sqlx-cli) are installed by bin/setup.sh, not brew. See docs/guides/development.md.

# ONNX Runtime — required for local embedding tests (temper-ingest)
# On Linux (Vercel deploy), the runtime is bundled in the binary.
brew "onnxruntime"

# actions checkers
brew "actionlint"
brew "shellcheck"

# YAML processor — used by scripts/bootstrap/system-bootstrap.sh to read install-profile.yaml
brew "yq"

# Postgres client (psql) — the org-bootstrap SQL root step + `cargo make seed` + DB inspection.
# Keg-only: bin/setup.sh points you at the PATH export if psql isn't already linked. The dev
# database itself runs in Docker (docker compose), so only the client is needed here.
brew "libpq"

# ── Code-intelligence (SCIP) tooling — NOT installable via brew. See bin/setup.sh step 6. ─────────
#
# ⚠️  DO NOT ADD `brew "scip"`. The Homebrew formula named `scip` is the **SCIP Optimization Suite**
#     (a mixed-integer-programming solver from scipopt.org) — an entirely unrelated product that
#     happens to share the name. Installing it gets you a MIP solver, not a code indexer.
#
# The three tools we actually need, and why none of them is a brew line:
#
#   scip CLI (github.com/sourcegraph/scip) — no brew formula exists, and `go install` is broken
#       upstream (their go.mod carries replace directives, which `go install` refuses). bin/setup.sh
#       fetches the pinned release binary and verifies its published sha256.
#
#   rust-analyzer — a *rustup component*, not a brew formula. `rustup component add rust-analyzer`
#       keeps it version-matched to your toolchain; the brew formula is a separate install that can
#       silently drift from the Rust version the repo builds with. Provides `rust-analyzer scip`.
#
#   scip-typescript (@sourcegraph/scip-typescript) — invoked via `bunx`, no install step needed.
