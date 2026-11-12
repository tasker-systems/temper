# Environment

## Local

- Postgres runs in Docker on **5437**:
  `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`. `cargo make docker-up`
  starts it and migrates; `cargo make db-migrate` migrates alone.
- `.env` (from `.env.template`) is required: every cargo-make task loads it.
- The pre-commit hook is `githooks/pre-commit`. It is not `cargo make check` — it misses the drift
  gates and `svelte-check`.

## Tooling

`mise install` from the repo root installs the pinned tools (`mise.toml`); rustup owns the Rust
toolchain. **Installed is not on PATH** for a non-interactive agent shell, which never runs mise's
prompt hooks — and macOS's `/usr/bin/python3` (3.9) is too old for the gates. Put the shims in a file
such shells read:

```bash
# ~/.zprofile (macOS); ~/.profile or ~/.bash_profile for bash
eval "$(mise activate zsh --shims)"
```

## Cloud sessions

- **Setup:** `tools/bin/setup-claude-web.sh` runs on SessionStart (`.claude/settings.json`) and must
  finish in seconds: env files, PATH, hooks, `bun install`. It also restarts Docker Postgres
  (dockerd, then `docker compose up -d`) when the image is already pulled. That matters because a
  resume or restart keeps the disk but loses processes, while the environment's own setup script
  runs only on a fresh build. It never pulls or builds; `TEMPER_SKIP_DOCKER_START=1` opts out. `tools/bin/setup-claude-web-full.sh` is
  run by hand when a task needs the toolchain, Docker Postgres and migrations. Cloud sessions do not
  use mise yet.
- **Authenticating the CLI without a browser:** `TEMPER_TOKEN` (a JWT, used in memory; a malformed
  one errors), or `TEMPER_AUTH=ambient` when an egress proxy attaches the credential (no header is
  sent and disk is never touched; refused beside `TEMPER_TOKEN`). `TEMPER_API_URL` overrides the
  base URL; `TEMPER_DEVICE_ID` pins a device id.
- A cloud session starts cold: no local vault or config, and no conversation history. A task
  written for one carries its own context — the schema it touches, the files to change, and
  acceptance criteria — or tells it how to read them from temper.
