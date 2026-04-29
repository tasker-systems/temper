# CLI Reference (Static Source)

> **Note:** This file is a static source reference only. The installed `reference.md`
> skill file is **generated dynamically** from the clap command tree via
> `temper skill generate`. Edit this file if you need to update footer content;
> edit `src/commands/skill.rs` (REFERENCE_FOOTER) for the authoritative version.

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

## Commands (generated — run `temper skill generate` for current)

| Command | Syntax |
|---------|--------|
| search | `temper search "<query>" [--context <ctx>] [--doc-type <type>]` |
| context | `temper context [<name>]` |
| resource create | `temper resource create --type <type> --title <title> --context <ctx> [options]` |
| resource list | `temper resource list --type <type> [--context <ctx>] [--limit <n>] [--stage <stage>]` |
| resource show | `temper resource show <slug> --type <type> [--context <ctx>]` |
| resource update | `temper resource update <slug> --type <type> [--stage <stage>] [--mode <mode>] [--effort <effort>]` |
| resource delete | `temper resource delete <slug> --type <type> [--context <ctx>] [--force]` |
| warmup | `temper warmup [--context <ctx>]` |
| events | `temper events [--limit <n>]` |
| status | `temper status` |

Pipe content via stdin for `resource create` (all types accept stdin body).

## Resource Types

| Type | Description |
|------|-------------|
| task | Work items with stage, mode, effort tracking |
| goal | High-level objectives that group tasks |
| session | Timestamped work session notes |
| research | Research notes and findings |
| concept | Named ideas, patterns, or domain terms |
| decision | Point-in-time choices with rationale (ADR-like) |

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

Use `--show-template` on `resource create` to display the expected frontmatter and body
structure without creating anything:

```bash
temper resource create --type session --show-template
temper resource create --type task --show-template
temper resource create --type research --show-template
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
