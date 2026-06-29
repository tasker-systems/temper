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
