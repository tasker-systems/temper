<!-- config-hash: {{ config_hash }} -->
---
name: temper
description: Use when managing knowledge vault tasks, sessions, or search — task start/create/done, session save, semantic search, context discovery, or any /temper command invocation
---

# Temper Workflow Skill

Vault: {{ vault_path }}

## Contexts
{{ context_list }}

## How This Skill Works

This is a modular skill. SKILL.md (this file) is the router — it tells you what to
read and when. Behavioral content lives in supporting files. Do NOT read all files
upfront; read only what the current task requires.

### Supporting Files
- `reference.md` — CLI commands, stages, mode/effort definitions
- `subagent-guidance.md` — 10 universal principles for dispatched subagents
- `session-lifecycle.md` — Session start/end patterns, drift detection, checkpoints

### Workflow Files (`workflows/`)
One file per mode/effort combination. Read only the one that matches the current task.

### Extension Files (`guidance/`)
User-created guidance files. Read and apply any files found here.
`guidance/fundamentals.md` contains project-specific principles if it exists.

## On Task Start

> **CLI sequence**: There is no `task start` command. To start a task:
> 1. `temper task show <slug>` — read the task content
> 2. `temper task move <slug> --stage in-progress` — mark it active
>
> Stages are: `backlog`, `in-progress`, `done`, `cancelled` (not "active").

1. Read the task content via `temper task show <slug>` — extract mode and effort
2. Move the task to in-progress: `temper task move <slug> --stage in-progress`
3. If mode or effort is missing, ask: "What mode (plan/build) and effort (small/medium/large)?"
4. Infer or ask the domain: "What kind of work is this? (a) Software development, (b) Writing/documentation, (c) Research/analysis, (d) Design/architecture, (e) Something else"
5. Check for `guidance/fundamentals.md`:
   - If it exists, read it and apply its principles
   - If it doesn't, offer: "This context has no project fundamentals. Want to set them up? (`/temper init`)"
6. Check auto-memory for user plugin preferences (skills they've said they rely on)
7. Scan for installed skills and plugins: check `~/.claude/skills/` for skills and `~/.claude/plugins/installed_plugins.json` for plugins (e.g. superpowers, LSP plugins, vercel-plugin)
8. Ask: "I found [list]. Want subagents to use any of these? Any other quality gates?"
9. Read `workflows/{mode}-{effort}.md` and follow it

## On Task Resume

> **CLI sequence**: To resume a task from a previous session:
> 1. `temper task show <slug>` — reload the task content
> 2. `temper session list --context <ctx>` — find the most recent session
> 3. `temper session show <title-slug> --context <ctx>` — read the session's "Next Steps"
> 4. Continue from the workflow file for this task's mode/effort

1. Read the task content via `temper task show <slug>` — extract mode, effort, and context
2. List recent sessions: `temper session list --context <ctx>`
3. Read the most recent session note: `temper session show <title-slug> --context <ctx>`
   - The slug is the title column from `session list` output
   - Supports partial matching — a unique substring of the slug is enough
4. If the task is not already in-progress, move it: `temper task move <slug> --stage in-progress`
5. Check for `guidance/fundamentals.md` — read if it exists
6. Check auto-memory for user plugin preferences
7. Scan for installed skills and plugins: check `~/.claude/skills/` for skills and `~/.claude/plugins/installed_plugins.json` for plugins (e.g. superpowers, LSP plugins, vercel-plugin)
8. Ask: "Resuming from last session. Found these skills: [list]. Want subagents to use any? Any other quality gates?"
9. Read `workflows/{mode}-{effort}.md` and continue from where the last session left off

## On Session Start

> Start a working session without a predefined task. Useful for exploration,
> ad-hoc work, or when a task hasn't been created yet.

1. If `--context <ctx>` provided, use it. Otherwise ask which context to work in.
2. List in-progress tasks: `temper task list --context <ctx>`
3. If tasks exist, ask: "Working on one of these, or something new?"
   - If existing task: pivot to **On Task Resume** with that slug
   - If new: continue as open session
4. Check for `guidance/fundamentals.md` — read if it exists
5. Check auto-memory for user plugin preferences
6. Scan for installed skills and plugins: check `~/.claude/skills/` for skills and `~/.claude/plugins/installed_plugins.json` for plugins (e.g. superpowers, LSP plugins, vercel-plugin)
7. Proceed with the user's request. At session end, save via:
   ```bash
   cat <<'EOF' | temper session save "<title>" --context <ctx> --state done
   ## Goal
   ...
   EOF
   ```

## On Task Create

> Guided interactive task creation. Gathers context, title, mode, effort,
> goal linkage, and acceptance criteria through conversation.

1. If `--context <ctx>` provided, use it. Otherwise list available contexts and ask.
2. Ask: "What's the title or problem statement for this task?"
3. Infer or ask mode:
   - "Is this (a) research/design/discovery (plan) or (b) implementation/building (build)?"
4. Infer or ask effort:
   - "How big is this? (a) small — single session, (b) medium — multi-step but bounded, (c) large — multi-session, may need decomposition"
5. List goals in context: `temper goal list --context <ctx>`
   - If goals exist, ask: "Link to a goal? [list] or (none)"
6. Ask: "Any specific acceptance criteria or outcomes?" (optional — user can skip)
7. Create the task (pipe the problem statement and acceptance criteria via stdin):
   ```bash
   cat <<'EOF' | temper task create --title "<title>" --context <ctx> --mode <mode> --effort <effort> [--goal <slug>]
   # <title>

   <problem statement from step 2>

   ## Acceptance Criteria

   <criteria from step 6, or omit section if skipped>
   EOF
   ```
8. Ask: "Task created. Want to start working on it now?"
   - If yes: pivot to **On Task Start** with the new slug

## Command Routing

| Invocation Pattern | Route To |
|-------------------|----------|
| `task start <slug>` | On Task Start |
| `task resume <slug>` | On Task Resume |
| `task create [--context <ctx>]` | On Task Create |
| `session start [--context <ctx>]` | On Session Start |
| Other commands (search, session save, etc.) | Read `reference.md` for syntax |

## Subagent Dispatch

Before dispatching any subagent:
1. Read `subagent-guidance.md`
2. Include all applicable principles in the subagent prompt (verbatim, not summarized)
3. Include project fundamentals from `guidance/fundamentals.md` if available
4. Include any user-selected plugin skills

## Session Lifecycle

Read `session-lifecycle.md` for:
- Session start checklist
- Session end save pattern
- Mid-session drift detection
- Checkpoint prompts
