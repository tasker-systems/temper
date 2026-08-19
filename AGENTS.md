# AGENTS.md

This file provides guidance to AI agents working with code in this repository. The shared
reference material lives in `internal/agents/` — read the relevant file before starting work.

## Quick reference

| Topic | Read |
|-------|------|
| What Temper is + full architecture (crates, packages, deployment, DB) | [internal/agents/architecture.md](internal/agents/architecture.md) |
| Build & test commands (cargo make, single tests, embed-gated, TS/UI) | [internal/agents/build-and-test.md](internal/agents/build-and-test.md) |
| Branch/commit conventions + feature flags | [internal/agents/conventions.md](internal/agents/conventions.md) |
| Key patterns — the operational invariants you must not break | [internal/agents/key-patterns.md](internal/agents/key-patterns.md) |
| Code quality rules + SQL query checking | [internal/agents/code-quality.md](internal/agents/code-quality.md) |
| Environment (Docker Postgres port, DATABASE_URL, pre-commit) + cloud agents | [internal/agents/environment.md](internal/agents/environment.md) |

## The temper workflow

Temper is a knowledge base for AI-assisted development. It maintains a vault of markdown files
with YAML frontmatter so that goals, tasks, sessions, research, and decisions persist across
conversations. The `temper` CLI manages the vault; the cloud API syncs it and provides semantic
search.

To start a work session, run `temper warmup` for a context primer. The `temper` CLI is
agent-first: with a non-TTY stdout (how agents invoke it) output defaults to JSON and ANSI-free.

## Skills

This repo ships a temper skill that teaches the workflow. Install it with
`temper skill install --target opencode` (or `--target claude` for Claude Code). The skill teaches
session lifecycle, grounding, and outcome registers — read it after install.

## Where specs and plans go

**Specs: `internal/superpowers/specs/`. Plans: `internal/superpowers/plans/`.**

Not `docs/` — everything in `docs/` is public and synced to the documentation
site. See `internal/README.md`.
