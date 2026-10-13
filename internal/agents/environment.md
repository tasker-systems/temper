# Environment & Cloud Agents

> Shared agent guidance — the source of truth for `AGENTS.md` and `CLAUDE.md`.

## Environment

- Docker Postgres on port **5437** (not 5432, to avoid conflicts).
- `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`
- Pre-commit hook in `githooks/pre-commit`.

## Tooling (mise)

[`mise.toml`](../../mise.toml) pins the dev tooling the gates call: Python, uv, Node, Bun, the lint
and secret-scan binaries, cargo-make, cargo-nextest, sqlx-cli, rust-analyzer and scip. It also says
what is deliberately left out, such as the Rust toolchain, which rustup owns. `mise install` from the
repo root installs everything.

**Installed is not the same as on PATH.** `mise activate` works through prompt and directory-change
hooks, which an agent's non-interactive shell may never run. `python3` then falls through to
whatever the machine has. For example, macOS's `/usr/bin/python3` is 3.9, and the repo's gates need 3.11 or
later. Put mise's shims on PATH in a file non-interactive shells do read:

```bash
# ~/.zprofile (macOS); for bash, ~/.profile or ~/.bash_profile
eval "$(mise activate zsh --shims)"
```

Shims resolve per directory, so the repo's pins apply inside the checkout and nowhere else. To check
the setup, `python3 --version` inside the repo should print the `mise.toml` line (3.12.x). `mise
doctor` reports anything missing.

Cloud sessions do not use mise yet. They still install tools through `tools/bin/setup-claude-web*.sh`.
mise needs a committed `mise.lock` there, because a cloud session cannot resolve tool versions
itself: its GitHub API access is scoped to this repository.

## Cloud Agents

For tasks delegated to cloud-based agent sessions, see [cloud-agents.md](./cloud-agents.md) for the task preparation guide and environment setup.
