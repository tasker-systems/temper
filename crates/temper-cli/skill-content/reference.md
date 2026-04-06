# CLI Reference

## Invocation

**Always run `temper` directly from PATH.** Never use `cargo run -p temper-cli`, `python`,
full paths, or any indirect invocation method — even when working inside the temper source
repository. The installed binary may differ from the in-development code, and that is
intentional: we use the installed CLI to manage our own workflow while evolving the crate.

**Before running any temper command**, verify the binary exists:
```bash
which temper
```
If `temper` is not on PATH, **stop and warn the user**:
> "The `temper` binary is not installed or not on PATH. Install it with
> `cargo install --path crates/temper-cli` or ensure `~/.cargo/bin` is in your PATH."

Do not fall back to `cargo run` as a workaround.

## Commands

| Command | Syntax |
|---------|--------|
| search | `temper search "<query>" [--context <ctx>] [--type <doctype>]` |
| context | `temper context [<name>]` |
| session save | `temper session save [<title>] [--context <ctx>] [--task <slug>] [--state <state>]` |
| session list | `temper session list [--context <ctx>] [--limit <n>]` |
| session show | `temper session show <slug> [--context <ctx>]` |
| task create | `temper task create --title "<title>" --context <ctx> [--goal <slug>] [--mode <mode>] [--effort <effort>]` |
| task list | `temper task list [--context <ctx>] [--goal <slug>] [--stage <stage>]` |
| task move | `temper task move <slug> [--stage <stage>] [--goal <slug>] [--context <ctx>] [--mode <mode>] [--effort <effort>]` |
| task done | `temper task done <slug> [--branch <name>] [--pr <url>] [--context <ctx>]` |
| task show | `temper task show <slug-or-suffix-or-seq> [--context <ctx>]` |
| goal list | `temper goal list [--context <ctx>]` |
| note create | `temper note create "<title>" [--context <ctx>] [--type <doctype>]` |
| research save | `temper research save "<title>" [--task <slug>]` |
| normalize | `temper normalize [--dry-run]` |
| events | `temper events [--limit <n>]` |
| warmup | `temper warmup [--context <ctx>]` |
| index | `temper index [--force]` |
| status | `temper status` |

Pipe content via stdin for `session save`, `note create`, and `research save`.

## Task Stages

| Stage | Meaning |
|-------|---------|
| backlog | Not yet started |
| in-progress | Actively being worked |
| done | Completed |
| cancelled | Abandoned or no longer relevant |

## Modes

| Mode | Purpose |
|------|---------|
| plan | Research, design, discovery -- understanding before building |
| build | Implementation -- producing artifacts |

## Effort Levels

| Effort | Scope |
|--------|-------|
| small | Single session, focused deliverable |
| medium | Multi-step, bounded to a clear outcome |
| large | Multi-session, may require decomposition |

## Discovery Workflow

1. `temper search "<topic>"` -- find relevant documents and notes
2. `temper context [<name>]` -- understand current context and recent activity
3. Use search results to guide targeted file reads

Search first, read second. Don't guess at file paths.

## Template Access

Use `--show-template` on creation commands to display the expected frontmatter and body
structure without creating anything:

```bash
temper note create --show-template
temper task create --show-template
temper research save --show-template
```

## Skill-Only Commands

These commands are handled by the skill routing layer, not the temper CLI directly.
They compose multiple CLI commands into guided workflows.

| Skill Command | What It Does |
|---------------|-------------|
| `task start <slug>` | Shows task, moves to in-progress, routes to workflow |
| `task resume <slug>` | Shows task, reads last session, continues workflow |
| `task create` | Guided interactive task creation with prompts |
| `session start` | Start a session without a predefined task |
