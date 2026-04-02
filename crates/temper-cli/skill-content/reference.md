# CLI Reference

## Invocation

Always run `temper` directly from PATH. Never use `cargo run`, `python`, full paths, or
any indirect invocation method.

## Commands

| Command | Syntax |
|---------|--------|
| search | `temper search "<query>" [--context <ctx>] [--type <doctype>]` |
| context | `temper context [<name>]` |
| session save | `temper session save "<title>" [--task <slug>] [--state <state>]` |
| session list | `temper session list [--context <ctx>] [--limit <n>]` |
| task create | `temper task create "<title>" --mode <mode> --effort <effort> [--context <ctx>]` |
| task list | `temper task list [--stage <stage>] [--context <ctx>]` |
| task move | `temper task move <slug> [--stage <stage>] [--mode <mode>] [--effort <effort>]` |
| task done | `temper task done <slug>` |
| task show | `temper task show <slug>` |
| goal list | `temper goal list [--context <ctx>]` |
| note create | `temper note create "<title>" [--context <ctx>] [--type <doctype>]` |
| research save | `temper research save "<title>" [--task <slug>]` |
| normalize | `temper normalize [--dry-run]` |
| events | `temper events [--limit <n>]` |
| warmup | `temper warmup` |
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
