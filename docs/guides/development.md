# Development setup

This guide takes a fresh checkout to a working dev/admin environment: able to run
`cargo make check` and `cargo make test-db`. It is for **contributors and operators
working from source** — end-users who only want the `temper` binary should use the
[one-liner installer](install.md) instead.

## Quick start

```bash
git clone git@github.com:tasker-systems/temper.git
cd temper
bin/setup.sh            # or: bin/setup.sh --with-cli   to also install the `temper` CLI
```

[`bin/setup.sh`](../../bin/setup.sh) is **idempotent** — re-running converges (each step
is skip-if-present) rather than re-installing. Pass `--dry-run` to see what it would do
without touching anything.

Then verify:

```bash
cargo make check        # fmt + clippy + docs + machete, TS typecheck + biome
cargo make test-db      # integration tests against the Docker Postgres
```

## What setup.sh does

macOS-first (Homebrew). On Linux it prints the dependency pointers and exits without
installing — the cargo + docker steps are identical; only the system-package step differs.

| Step | What | Source |
|------|------|--------|
| 1 | Homebrew packages: `onnxruntime`, `actionlint`, `shellcheck`, `yq`, `libpq` | [`Brewfile`](../../Brewfile) |
| 2 | Cargo tooling: `cargo-make`, `cargo-nextest`, `sqlx-cli` (not brew formulae) | `cargo install` |
| 3 | Git hooks → `core.hooksPath = githooks/` | [`scripts/install-hooks.sh`](../../scripts/install-hooks.sh) |
| 4 | Docker Postgres on port **5437**, waited until healthy | `docker compose up -d` |
| 5 | Migrations applied to the dev database | `sqlx migrate run` |
| 6 | *(opt-in `--with-cli`)* install `temper` from this checkout | `cargo install --path crates/temper-cli --locked` |

### The `psql` (libpq) PATH note

`libpq` is keg-only on Homebrew, so its `psql` may not land on your PATH. The dev
database runs in Docker, so `psql` is only needed for the [org-bootstrap SQL root
step](org-bootstrap.md), `cargo make seed`, and manual DB inspection. If `setup.sh`
reports `psql` isn't linked, add the printed export to your shell profile.

## Daily commands

All task orchestration is [cargo-make](https://github.com/sagiegurari/cargo-make):

```bash
cargo make check        # quality gate (what CI's code-quality job runs offline)
cargo make fix          # auto-fix fmt + clippy
cargo make test         # unit tests (no database)
cargo make test-db      # integration tests (Docker Postgres)
cargo make test-e2e     # CLI ↔ API ↔ DB end-to-end
cargo make test-e2e-embed   # + the embed pipeline (ONNX); matches CI's Embed job
cargo make run          # run the API server locally
```

The dev database connection string is
`postgresql://temper:temper@localhost:5437/temper_development` (port 5437 avoids
conflicts). Most tests provision their own ephemeral databases via the embedded
migrator, so they only need Postgres *running*, not the dev DB migrated — but
`cargo make run` and `cargo make seed` use the dev DB directly, which is why setup
applies migrations to it.

### After changing SQL

Regenerate the offline sqlx cache so `cargo make check` (which runs `SQLX_OFFLINE=true`)
stays honest:

```bash
cargo sqlx prepare --workspace -- --all-features   # production targets
cargo make prepare-services && cargo make prepare-e2e   # test-target queries
```

## Admin / operator extras

Some operator tasks need tools `setup.sh` does not auto-install:

- **`neonctl`** (npm: `npm i -g neonctl`) — Neon cloud branch/backup management for
  prod operations. See [releasing.md](releasing.md) and the Neon backup convention.
- **Provisioning an org** on a running instance — [org-bootstrap.md](org-bootstrap.md)
  (the `bin/setup.sh` dev environment is a prerequisite for running its applier locally).

## Troubleshooting

- **`cargo make check` fails with `relation "…" does not exist`** — the dev DB is behind;
  run `sqlx migrate run` (or re-run `bin/setup.sh`). This is a local-only signal; CI uses
  `SQLX_OFFLINE=true`, so it is not a push blocker.
- **Embed tests can't find ONNX Runtime** — `brew bundle` installs `onnxruntime`; the
  default `temper` build bundles the runtime via the `embed-download` feature.
- **Homebrew not installed** — `setup.sh` stops with a pointer to https://brew.sh.
