# Environment & Cloud Agents

> Shared agent guidance — the source of truth for `AGENTS.md` and `CLAUDE.md`.

## Environment

- Docker Postgres on port **5437** (not 5432, to avoid conflicts).
- `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`
- Pre-commit hook in `githooks/pre-commit`.

## Cloud Agents

For tasks delegated to cloud-based agent sessions, see [docs/guides/cloud-agents.md](../guides/cloud-agents.md) for the task preparation guide and environment setup.