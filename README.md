# Temper

Developer workflow tool for agent-assisted development. Semantic search across projects, session-based checkpointing, local ticket/milestone tracking, and Claude Code skill generation — all backed by a markdown vault with YAML frontmatter.

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
temper ticket create --title "Add authentication" --project myapp

# Save a session note
temper session save "Implemented auth flow"

# Generate and install the Claude Code skill
temper skill install
```

## Commands

| Command | Description |
|---------|-------------|
| `temper init [path]` | Initialize a new vault |
| `temper check` | Verify vault integrity and tool health |
| `temper status` | Vault overview with index stats |
| `temper search <query>` | Semantic search across indexed content |
| `temper context <topic>` | Show topic with related context |
| `temper index` | Build/rebuild semantic search index |
| `temper note create <type> <title>` | Create note from template |
| `temper session save [title]` | Create/update today's session note |
| `temper session list` | List recent sessions |
| `temper ticket create --title <t>` | Create a ticket |
| `temper ticket move <slug> --stage <s>` | Move ticket to a new stage |
| `temper ticket done <slug>` | Mark ticket as done |
| `temper ticket list` | List tickets |
| `temper ticket board` | Board view |
| `temper milestone create --title <t>` | Create a milestone |
| `temper milestone list` | Roadmap view |
| `temper project add --name <n> --path <p>` | Add project to config |
| `temper project list` | List configured projects |
| `temper events` | Show recent vault events |
| `temper warmup` | Context primer for new sessions |
| `temper skill generate` | Preview generated Claude Code skill |
| `temper skill install` | Install skill file |
| `temper skill check` | Verify skill is current |

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

The generated skill integrates with the [superpowers](https://github.com/anthropics/claude-code-plugins) workflow: brainstorm, design, plan, implement, finish.

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

## License

MIT
