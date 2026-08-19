# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this
repository. The shared reference material lives in `docs/agents/` — read the relevant file before
starting work. The same content is also reachable via [AGENTS.md](AGENTS.md) for other agents.

## Quick reference

| Topic | Read |
|-------|------|
| What Temper is + full architecture (crates, packages, deployment, DB) | [docs/agents/architecture.md](docs/agents/architecture.md) |
| Build & test commands (cargo make, single tests, embed-gated, TS/UI) | [docs/agents/build-and-test.md](docs/agents/build-and-test.md) |
| Branch/commit conventions + feature flags | [docs/agents/conventions.md](docs/agents/conventions.md) |
| Key patterns — the operational invariants you must not break | [docs/agents/key-patterns.md](docs/agents/key-patterns.md) |
| Code quality rules + SQL query checking | [docs/agents/code-quality.md](docs/agents/code-quality.md) |
| Environment (Docker Postgres port, DATABASE_URL, pre-commit) + cloud agents | [docs/agents/environment.md](docs/agents/environment.md) |

## The temper workflow

Temper is a knowledge base for AI-assisted development. It maintains a vault of markdown files
with YAML frontmatter so that goals, tasks, sessions, research, and decisions persist across
conversations. The `temper` CLI manages the vault; the cloud API syncs it and provides semantic
search.

To start a work session, run `temper warmup` for a context primer. The `temper` CLI is
agent-first: with a non-TTY stdout (how agents invoke it) output defaults to JSON and ANSI-free.

## Skills

This repo ships a temper skill that teaches the workflow. Install it with
`temper skill install --target claude`. The skill teaches session lifecycle, grounding, and
outcome registers — read it after install.
## Where specs and plans go

**Specs: `internal/superpowers/specs/`. Plans: `internal/superpowers/plans/`.**

Not `docs/`. `docs/` is synced to the public documentation site, and everything
in it is public — so process artifacts must not be written there. The
superpowers skills default to `docs/superpowers/...`; this instruction overrides
that default. See `internal/README.md`.
