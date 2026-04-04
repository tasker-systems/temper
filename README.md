<p align="center">
  <img src="docs/brand-mark.svg" alt="temper" width="200" />
</p>

<p align="center">
  <strong>/ˈtempər/</strong> — <em>to make stronger and more resilient through a deliberate process</em>
</p>

A knowledge base for builders. Temper gives your work a throughline — the connective thread across sessions, decisions, and evolving understanding that turns scattered context into a navigable history. Everything resolves to markdown. The system gets out of the way.

<p align="center">
  <a href="https://temperkb.io">temperkb.io</a> · <a href="https://temperkb.io/builders">For builders</a> · <a href="https://temperkb.io/agents">For agents</a>
</p>

## The Problem

AI coding agents are powerful but forgetful. Every session starts blank — no memory of yesterday's decisions, no awareness of in-flight work, no sense of what matters next. The industry calls this [context rot](https://www.understandingai.org/p/context-rot-the-emerging-challenge): the progressive degradation of an agent's understanding as work spans sessions.

<p align="center">
  <img src="docs/diagrams/context-rot.svg" alt="Context rot: without a knowledge base, understanding degrades; with one, it compounds" width="720" />
</p>

Developers compensate by re-explaining context, pasting old chat logs, and manually steering agents through decisions the agent should already know about. This tax grows with every session.

The deeper issue isn't memory — it's **throughline**. Knowing what's been done, what's up next, what decisions have been made and which are still open. This is the connective tissue that turns a pile of documents into a navigable development history. Without it, each session reinvents context from scratch. With it, sessions build on each other.

## Throughline

Temper embeds throughline directly into how you work. Goals hold the vision. Tasks carry the work. Sessions record what happened. Each layer provides context for the layer below, and each session's conclusions feed back up — refining the goals, sharpening the path forward.

<p align="center">
  <img src="docs/diagrams/throughline-layers.svg" alt="Throughline: from goals through tasks down to sessions" width="700" />
</p>

This isn't a ticketing system competing with Linear. It's a structured vault of markdown files where every goal, task, session, decision, and research thread has a home — and where the connections between them are always visible.

## Session Continuity

Every new session starts with `temper warmup`, which injects active tasks, recent session summaries, and the last session's full content. The agent resumes where you left off instead of starting from scratch.

At the end of each session, `temper session save` captures what happened — decisions made, tasks updated, next steps identified — and writes it back to the vault. The next session reads it. Context compounds instead of decaying.

<p align="center">
  <img src="docs/diagrams/session-continuity-cycle.svg" alt="Session continuity cycle: warmup, work, save — each session feeds back into the vault" width="700" />
</p>

## Goals and Tasks

Temper gives you two building blocks:

**Goals** are the outcomes you're working toward. A goal holds the vision and purpose of a feature, a product, a body of work. Tasks and sessions roll up to goals.

**Tasks** are units of work toward a goal. Every task has a **mode** — `build` or `plan` — and an expected **effort** — `small`, `medium`, or `large`. Your workflow preferences (set during `temper init`) shape how these translate into process — temper carries the throughline regardless of what tools and ceremonies you prefer.

## For Humans and Agents

Temper gives agents the same throughline that humans carry in their heads: what we're building, why, what we've decided, and what's deferred. Agents reach the vault three ways:

- **CLI** — `temper warmup`, `temper search`, `temper session save`. Claude Code hooks call `temper warmup` automatically at session start.
- **MCP Server** — vault operations exposed as structured tools. Agents query, read, and write through the Model Context Protocol.
- **Skill File** — `temper skill install` generates a Claude Code skill that teaches the agent your vault's structure and workflow conventions.

If it can read files, it can use temper.

## Installation

```bash
cargo install temper-cli
```

Or build from source:

```bash
git clone https://github.com/tasker-systems/temper.git
cd temper
cargo install --path crates/temper-cli
```

## Quick Start

```bash
# Initialize a vault — temper asks how you work
temper init

# Add a context for your project
temper context add myapp

# Add your docs — temper extracts markdown and indexes it
temper add --context myapp --dir ~/projects/myapp/docs

# Search across your vault
temper search "authentication decisions"

# Generate and install the Claude Code skill
temper skill install

# Save a session
temper session save "Implemented auth flow, chose JWT rotation"
```

## The Vault

The vault is a directory of markdown files with YAML frontmatter. This is deliberate:

- **Human-readable.** Browse your vault in any editor, in Obsidian, or on GitHub. No proprietary formats.
- **Version-controllable.** Git tracks changes. Diffs are readable. History is auditable.
- **AI-native.** Language models understand markdown and YAML frontmatter natively. No parsing overhead.
- **Portable.** The knowledge base is the unit of value, not the tool.

## Commands

### Core

| Command | Description |
|---------|-------------|
| `temper init` | Initialize a new vault |
| `temper check` | Verify vault integrity and tool health |
| `temper status` | Vault overview |
| `temper events` | Show recent vault events |
| `temper warmup` | Context primer for new sessions |
| `temper normalize` | Repair vault structure drift |

### Search

| Command | Description |
|---------|-------------|
| `temper search <query>` | Semantic search across the knowledge base |
| `temper context <n>` | Show topic with related context |

### Content

| Command | Description |
|---------|-------------|
| `temper note create <type> <title>` | Create note from template |
| `temper session save [title]` | Create/update session note |
| `temper session list` | List recent sessions |
| `temper research save <title>` | Create research note |
| `temper add <path>` | Add a file, URL, or directory to the vault (managed, frontmatter, sync-ready) |

### Goals and Tasks

| Command | Description |
|---------|-------------|
| `temper task create --title <t>` | Create a task |
| `temper task list` | List tasks |
| `temper goal create --title <t>` | Create a goal |
| `temper goal list` | List goals |

### Contexts and Skills

| Command | Description |
|---------|-------------|
| `temper context add <n>` | Add a context |
| `temper context list` | List contexts |
| `temper skill generate` | Preview generated Claude Code skill |
| `temper skill install` | Install skill file |

### Cloud

| Command | Description |
|---------|-------------|
| `temper auth` | Authenticate with temper cloud |
| `temper sync` | Sync local vault with temper cloud |
| `temper pull <resource>` | Pull a resource from the cloud |
| `temper remove <resource>` | Remove a resource from the cloud |

## Semantic Search

Temper embeds your vault content using all-MiniLM-L6-v2 (via Candle, no Python required) and stores vectors for fast approximate nearest-neighbor search. Indexing is incremental — only changed files are re-embedded.

```bash
temper search "design patterns" --limit 5
```

## Claude Code Integration

Temper generates a Claude Code skill file tailored to your vault:

```bash
temper skill install
```

### Session Pre-Warming

To automatically prime new Claude Code sessions with recent context, add a `SessionStart` hook to your project's `.claude/settings.local.json`:

```json
{
  "hooks": {
    "SessionStart": [{
      "hooks": [{
        "type": "command",
        "command": "temper warmup --context myapp"
      }]
    }]
  }
}
```

This runs `temper warmup` on every new session, injecting active tasks, recent sessions, open decisions, and project events.

## Temper Cloud

Your vault stays as markdown files on your machine. `temper sync` uses a manifest-based protocol to compute diffs between your local state and the server — a three-way comparison of local file, manifest record, and remote content. Non-conflicting changes merge automatically at the paragraph level using Rust-native diffing. Genuine conflicts are annotated in `.conflict.md` files for human resolution.

Your vault can also live in a git repo, sync to Obsidian, or coexist with any other tool that reads files — temper's sync is self-contained and doesn't depend on or interfere with external version control.

What cloud adds:

- **Cross-machine sync** with manifest-based diffing and auto-merge
- **Semantic search** powered by pgvector embeddings
- **MCP server** for direct agent integration
- **Team contexts** with granular access control
- **Self-host or use temperkb.io** — same protocol, your choice

## Related Work

Temper draws on ideas from several projects working on adjacent problems:

- [superpowers](https://github.com/obra/superpowers) — Structured workflow stages for agent-assisted development
- [speckit](https://github.com/github/spec-kit) — Specification-driven development with AI
- [OpenSpec](https://github.com/Fission-AI/OpenSpec) — Open standard for AI-friendly project specifications
- [GSD](https://thenewstack.io/beating-the-rot-and-getting-stuff-done/) — Framework for managing context rot in agent workflows

## License

MIT
