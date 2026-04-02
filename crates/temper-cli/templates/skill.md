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

1. Read the task content — extract mode and effort
2. If mode or effort is missing, ask: "What mode (plan/build) and effort (small/medium/large)?"
3. Infer or ask the domain: "What kind of work is this? (a) Software development, (b) Writing/documentation, (c) Research/analysis, (d) Design/architecture, (e) Something else"
4. Check for `guidance/fundamentals.md`:
   - If it exists, read it and apply its principles
   - If it doesn't, offer: "This context has no project fundamentals. Want to set them up? (`/temper init`)"
5. Check auto-memory for user plugin preferences (skills they've said they rely on)
6. Scan for installed skills: check `~/.claude/skills/` and plugins cache
7. Ask: "I found [list]. Want subagents to use any of these? Any other quality gates?"
8. Read `workflows/{mode}-{effort}.md` and follow it

## On Other Commands

For non-task-start invocations (search, session save, etc.), read `reference.md`
for command syntax and follow standard patterns.

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
