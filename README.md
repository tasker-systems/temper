# **Temper**

**/ˈtempər/** — *to make stronger and more resilient through a deliberate process*

Developer workflow tool for agent-assisted development. Semantic search across projects, session-based checkpointing, local ticket/milestone tracking, and Claude Code skill generation — all backed by a markdown vault with YAML frontmatter.

## The Problem

AI coding agents are powerful but forgetful. Every session starts blank — no memory of yesterday's decisions, no awareness of in-flight work, no sense of what matters next. The industry is starting to call this [context rot](https://www.understandingai.org/p/context-rot-the-emerging-challenge): the progressive degradation of an agent's understanding as work spans sessions, branches, and projects.

<p align="center">
  <img src="docs/diagrams/context-rot.svg" alt="Context Rot: without a knowledge base, understanding degrades; with one, it compounds" width="720" />
</p>

Developers compensate by re-explaining context, copy-pasting old chat logs, and manually steering agents through workflows that the agent should already understand. Frameworks like [superpowers](https://github.com/obra/superpowers), [speckit](https://github.com/github/spec-kit), [OpenSpec](https://github.com/Fission-AI/OpenSpec), and [GSD](https://thenewstack.io/beating-the-rot-and-getting-stuff-done/) are all working on parts of this — organizing specifications, plans, and workflows to give agents better footing. But the artifacts these frameworks produce are only as useful as they are **coherent, evolving, organized, and findable**.

The deeper issue is one of **throughline**. Knowing what's been done, what's up next, what decisions have been made and which are still open — this is the connective tissue that turns a pile of specs and tickets into a navigable development history. Without it, each session reinvents context from scratch. With it, sessions build on each other in a virtuous cycle: work concludes with explicit guidance on where it ended, what the next session should consider, and which open threads remain.

## Throughline: The Missing Layer

Temper is a local-first knowledge base that embeds **throughline** directly into the development process — not a ticketing system competing with Linear or GitHub Issues, but a way of organizing sessions into a coherent narrative that agents can read, write, and build on.

Milestones hold the project vision. Tickets carry the immediate work. Session notes capture what happened, what changed, and what comes next. Each layer provides context for the layer below, and each session's conclusions feed back up — refining the roadmap, sharpening the vision. The result is a living development history, not a pile of disconnected artifacts.

<p align="center">
  <img src="docs/diagrams/throughline-layers.svg" alt="Throughline: from project vision through milestones and tickets down to sessions" width="700" />
</p>

## Session Continuity

Every new session starts with `temper warmup`, which injects in-progress tickets, recent session summaries, and the last session's full content via a Claude Code startup hook. The agent resumes where you left off instead of starting from scratch.

At the end of each session, `temper session save` captures what happened — decisions made, tickets updated, next steps identified — and writes it back to the vault. The next session reads it. Context compounds instead of decaying.

<p align="center">
  <img src="docs/diagrams/session-continuity-cycle.svg" alt="Session Continuity Cycle: warmup, work, save — each session feeds back into the vault" width="700" />
</p>

## Adaptive Workflow

Not every task deserves the same process. A one-line typo fix shouldn't get the same brainstorm-design-plan-implement ceremony as a new authentication system. Every ticket carries a `scope` — `patch`, `feature`, or `epic` — that controls how much ceremony the agent applies.

<p align="center">
  <img src="docs/diagrams/scope-routing.svg" alt="Scope Routing: patch gets direct implementation, feature gets the full pipeline, epic produces a roadmap" width="700" />
</p>

A patch gets direct implementation. A feature gets the full design pipeline. An epic produces a strategic roadmap, not code. Scope can shift within a session — so long as the information is carried forward and the evolving decisions are tracked.

## What's Under the Hood

**A markdown vault as institutional memory.** Tickets, milestones, session notes, and research live in plain markdown files with YAML frontmatter — human-readable, git-trackable, and natively understood by language models.

**Semantic search across everything.** Temper embeds your vault using local ML models and builds an HNSW index for fast similarity search. Ask for "error handling patterns" and get relevant results across all your indexed content — no keyword guessing.

**A generated skill file that teaches the agent your workflow.** `temper skill install` produces a Claude Code skill documenting your vault structure, available commands, and scope-routing logic. The agent doesn't need to be told how to use Temper — the skill makes it a first-class capability.

## Installation

```bash
cargo install temper-cli
```

Or build from source:

```bash
git clone https://github.com/tasker-systems/temper.git
cd temper
cargo install --path .
```

## Quick Start

```bash
# Initialize a vault
temper init ~/vaults/work

# Add your projects
temper project add --name myapp --path ~/projects/myapp

# Build the search index (downloads embedding model on first run)
temper index

# Search across all indexed content
temper search "error handling patterns"

# Create a milestone and ticket
temper milestone create --title "v0.1" --project myapp
temper ticket create --title "Add authentication" --project myapp --scope feature

# Save a session note
temper session save "Implemented auth flow"

# Generate and install the Claude Code skill
temper skill install
```

## Scope Details

| Scope | Nature | Ceremony | Output |
|-------|--------|----------|--------|
| `patch` | Tactical | None — just do it | Delivered code |
| `feature` | Deliberate | Full design pipeline | Delivered code with design artifact |
| `epic` | Strategic | Deep discovery + roadmapping | Living milestone roadmap + first actionable ticket |

### Patch Workflow

Direct implementation. No spec, no plan, no brainstorming.

<p align="center">
  <img src="docs/diagrams/patch-workflow.svg" alt="Patch Workflow" width="500" />
</p>

### Feature Workflow

Full superpowers pipeline: brainstorm, design, plan, implement, finish.

<p align="center">
  <img src="docs/diagrams/feature-workflow.svg" alt="Feature Workflow" width="500" />
</p>

### Epic Workflow

Strategic planning. The output is a milestone roadmap, not delivered code. Each subsequent session works one ticket from the roadmap, learns, evolves the roadmap, and creates the next ticket.

<p align="center">
  <img src="docs/diagrams/epic-workflow.svg" alt="Epic Workflow" width="500" />
</p>

## Commands

### Core

| Command | Description |
|---------|-------------|
| `temper init [path]` | Initialize a new vault |
| `temper check` | Verify vault integrity and tool health |
| `temper status` | Vault overview with index stats |
| `temper events [--project <p>]` | Show recent vault events |
| `temper warmup [--project <p>]` | Context primer for new sessions |
| `temper normalize [--project <p>]` | Repair vault structure drift |

### Search

| Command | Description |
|---------|-------------|
| `temper search <query>` | Semantic search across indexed content |
| `temper context <topic> [--depth N]` | Show topic with related context graph |
| `temper index` | Build/rebuild semantic search index |

### Notes and Sessions

| Command | Description |
|---------|-------------|
| `temper note create <type> <title>` | Create note from template |
| `temper session save [title]` | Create/update today's session note |
| `temper session list` | List recent sessions |
| `temper research save <title>` | Create high-fidelity research note |

### Tickets and Milestones

| Command | Description |
|---------|-------------|
| `temper ticket create --title <t> [--scope patch\|feature\|epic]` | Create a ticket |
| `temper ticket move <slug> [--stage <s>] [--scope <sc>]` | Move ticket stage or update scope |
| `temper ticket done <slug>` | Mark ticket as done |
| `temper ticket show <slug>` | Show ticket content |
| `temper ticket list [--project <p>]` | List tickets |
| `temper milestone create --title <t>` | Create a milestone |
| `temper milestone list` | Roadmap view |

### Projects and Skills

| Command | Description |
|---------|-------------|
| `temper project add --name <n> --path <p>` | Add project to config |
| `temper project list` | List configured projects |
| `temper skill generate` | Preview generated Claude Code skill |
| `temper skill install` | Install skill file |
| `temper skill check` | Verify skill is current |

## Ticket Lifecycle

Tickets use four stages: `backlog` → `in-progress` → `done` (or `cancelled`).

```bash
temper ticket create --title "Fix auth bug" --scope patch
temper ticket move fix-auth-bug --stage in-progress
temper ticket done fix-auth-bug --branch feat/auth --pr https://github.com/org/repo/pull/1
```

Session notes can link to tickets, optionally transitioning their stage:

```bash
temper session save "Fixed the auth bug" --ticket fix-auth-bug --state done
```

## Configuration

Temper uses `temper.toml` at the vault root:

```toml
[vault]
sessions = "sessions"
tickets = "tickets"
milestones = "milestones"
templates = "templates"
state_dir = ".temper"

[index]
include = ["docs", "notes"]
exclude = [".git", "archive"]
sources = ["~/projects/other-repo/docs"]

[embedder]
model = "all-MiniLM-L6-v2"
cache_dir = "~/.cache/temper/models"

[projects.myapp]
repo = "org/myapp"
path = "~/projects/myapp"

[skill]
output = "~/.claude/commands/temper.md"
framework = "superpowers"
```

### Vault Resolution

Temper finds your vault using this chain:

1. `--vault <path>` CLI flag
2. `TEMPER_VAULT` environment variable
3. Walk up from CWD looking for `temper.toml`
4. `~/.config/temper/config.toml` default vault

## Claude Code Integration

Temper generates a Claude Code skill file tailored to your vault:

```bash
temper skill install           # Install globally
temper skill install --project ~/projects/myapp  # Project-scoped
```

The generated skill integrates with the [superpowers](https://github.com/anthropics/claude-code-plugins) workflow and uses scope-based routing to match process intensity to task complexity.

### Session Pre-Warming

To automatically prime new Claude Code sessions with recent context, add a `SessionStart` hook to your project's `.claude/settings.local.json`:

```json
{
  "hooks": {
    "SessionStart": [{
      "matcher": "startup",
      "hooks": [{
        "type": "command",
        "command": "temper warmup --project <your-project>"
      }]
    }]
  }
}
```

This runs `temper warmup` on every new session, injecting:
- In-progress tickets with scope labels
- Last 3 session summaries
- Full content of the most recent session note
- Last 15 project events (ticket/milestone activity)

## Semantic Search

Temper embeds your vault content using all-MiniLM-L6-v2 (via Candle, no Python required) and stores vectors in an HNSW index for fast approximate nearest-neighbor search. Indexing is incremental — only changed files are re-embedded.

```bash
temper index                   # Build/update index
temper search "design patterns" --limit 5
temper context "Authentication" --depth 2
```

## Roadmap

Temper is local-first today. **Temper Cloud** is in active design — a cloud-native extension where Postgres owns structured metadata and lifecycle state, git owns document content and history, and temper is the intervention layer that reconciles between them. The cloud layer adds multi-machine access, pg_vector-backed search, and an MCP server for direct agent integration. The guiding constraint is continuity: temper continues to function locally throughout, and the knowledge base — not the tool — is the unit of value.

See **[VISION.md](VISION.md)** for the full design philosophy, throughline concept, and Temper Cloud architecture.

## Related Work

Temper draws on ideas from several projects working on adjacent problems:

- [superpowers](https://github.com/obra/superpowers) — Structured workflow stages (brainstorm, design, plan, implement) for agent-assisted development
- [speckit](https://github.com/github/spec-kit) — Specification-driven development with AI
- [OpenSpec](https://github.com/Fission-AI/OpenSpec) — Open standard for AI-friendly project specifications
- [GSD](https://thenewstack.io/beating-the-rot-and-getting-stuff-done/) — Framework for managing context rot in agent workflows

## License

MIT
