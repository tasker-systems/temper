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
#   scip-typescript (@sourcegraph/scip-typescript) — invoked via `bunx`, no install step needed.

# rust-analyzer — for INDEXING, take the brew formula, NOT the rustup component.
#
# This reverses an earlier call in this file, and the reason is worth writing down. The rustup
# component (`rustup component add rust-analyzer`) is pinned to the *Rust release cadence*, so it lags
# rust-analyzer's own release stream by up to SIX MONTHS. That is fine for IDE use — it is exactly the
# "version-matched to your toolchain" property you want there — and it is WRONG for indexing.
#
# We ran a January build in July and it PANICKED on a sibling repo (salsa-0.24 cycle-handling bug,
# since fixed upstream). We very nearly wrote that up as "the indexer cannot handle a legal Cargo
# dependency cycle" — a false architectural finding — when the truth was "our indexer was stale."
# A stale indexer does not merely index worse; it can fail outright and look like a property of the code.
brew "rust-analyzer"
