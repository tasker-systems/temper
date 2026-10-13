# Development dependencies for temper that mise does not manage.
# Install with `brew bundle`, or run `bin/setup.sh` for the full dev/admin bootstrap.
# Everything with a version worth pinning is installed by mise from mise.toml instead: language
# runtimes, lint binaries, cargo tooling, and the code-intelligence tools.
# See internal/development/development.md.

# mise — installs and activates the tools pinned in mise.toml.
brew "mise"

# ONNX Runtime — required for local embedding tests (temper-ingest)
# On Linux (Vercel deploy), the runtime is bundled in the binary.
brew "onnxruntime"

# Postgres client (psql) — the org-bootstrap SQL root step + `cargo make seed` + DB inspection.
# Keg-only: bin/setup.sh points you at the PATH export if psql isn't already linked. The dev
# database itself runs in Docker (docker compose), so only the client is needed here.
brew "libpq"

# ⚠️  DO NOT ADD `brew "scip"`. The Homebrew formula named `scip` is the **SCIP Optimization Suite**
#     (a mixed-integer-programming solver from scipopt.org), an unrelated product that happens to
#     share the name. Sourcegraph's scip CLI comes from mise.toml (`github:sourcegraph/scip`).
#
# rust-analyzer for indexing also comes from mise.toml, as a dated upstream release. The reason it
# is neither the rustup component nor an unpinned brew formula is written down there.
