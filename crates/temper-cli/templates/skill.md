<!-- config-hash: {{ config_hash }} -->
---
name: temper
description: Use when managing knowledge vault tasks, sessions, or search — task start/create/done, session save, semantic search, context discovery, or any /temper command invocation
---

# Temper Workflow Skill

Vault: {{ vault_path }}

## Contexts
{{ context_list }}

Address a context by ref: `@me/<slug>` for your own, `+team-slug/<slug>` for a team. Bare context names are **not accepted**.

## How This Skill Works

This is a modular skill. SKILL.md (this file) is the router — it tells you what to
read and when. Behavioral content lives in supporting files. Do NOT read all files
upfront; read only what the current task requires.

### Supporting Files
- `reference.md` — CLI commands, stages, mode/effort definitions
- `subagent-guidance.md` — 10 universal principles for dispatched subagents
- `session-lifecycle.md` — Session start/end patterns, drift detection, checkpoints
- `knowledge-base.md` — MCP resources and tools for cloud knowledge base access
- `cognitive-maps.md` — Reading from and authoring into cognitive maps (telos-governed graphs)

### Workflow Files (`workflows/`)
One file per mode/effort combination. Read only the one that matches the current task.

### Extension Files (`guidance/`)
User-created guidance files. Read and apply any files found here.
`guidance/fundamentals.md` contains project-specific principles if it exists.

## On Task Start

> **Addressing**: resources are addressed by **ref** — a UUID or the decorated
> `sluggify(title)-<uuid>` form (resolution is trailing-UUID-only; the slug half is
> presentation, a stale slug half is harmless). Every `resource list`/`search`/`show`
> row carries a `ref` field — copy it. `resource show`/`update`/`delete` take a single
> `<ref>` (no `--type`/`--context`). `resource create` and `resource list` still take
> `--type`/`--context` (create writes *into* a context; list filters by them).
>
> **CLI sequence**: There is no `task start` command. To start a task:
> 1. `temper resource list --type task --context @me/<ctx>` — find the task, copy its `ref`
> 2. `temper resource show <ref>` — read the task content
> 3. `temper resource update <ref> --stage in-progress` — mark it active
>
> Stages are: `backlog`, `in-progress`, `done`, `cancelled` (not "active").

1. Resolve the task's ref: `temper resource list --type task --context @me/<ctx>`, find the row matching `<slug>`, copy its `ref`. Read it via `temper resource show <ref>` — extract mode and effort
2. Move the task to in-progress: `temper resource update <ref> --stage in-progress`
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
> 1. `temper resource list --type task --context @me/<ctx>` — find the task, copy its `ref`; `temper resource show <ref>` — reload the task content
> 2. `temper resource list --type session --context @me/<ctx>` — find the most recent session, copy its `ref`
> 3. `temper resource show <ref>` — read the session's "Next Steps"
> 4. Continue from the workflow file for this task's mode/effort

1. Resolve the task's ref via `temper resource list --type task --context @me/<ctx>`, then `temper resource show <ref>` — extract mode, effort, and context
2. List recent sessions: `temper resource list --type session --context @me/<ctx>`
3. Read the most recent session note: copy the `ref` of the most recent session row, then `temper resource show <ref>`
   - Match the session by its `slug`/`title` column in the `resource list` output (a unique substring is enough), then copy that row's `ref`
4. If the task is not already in-progress, move it: `temper resource update <ref> --stage in-progress`
5. Check for `guidance/fundamentals.md` — read if it exists
6. Check auto-memory for user plugin preferences
7. Scan for installed skills and plugins: check `~/.claude/skills/` for skills and `~/.claude/plugins/installed_plugins.json` for plugins (e.g. superpowers, LSP plugins, vercel-plugin)
8. Ask: "Resuming from last session. Found these skills: [list]. Want subagents to use any? Any other quality gates?"
9. Read `workflows/{mode}-{effort}.md` and continue from where the last session left off

## On Session Start

> Start a working session without a predefined task. Useful for exploration,
> ad-hoc work, or when a task hasn't been created yet.

1. If `--context @me/<ctx>` provided, use it. Otherwise ask which context to work in.
2. List in-progress tasks: `temper resource list --type task --context @me/<ctx>`
3. If tasks exist, ask: "Working on one of these, or something new?"
   - If existing task: pivot to **On Task Resume** with that slug
   - If new: continue as open session
4. Check for `guidance/fundamentals.md` — read if it exists
5. Check auto-memory for user plugin preferences
6. Scan for installed skills and plugins: check `~/.claude/skills/` for skills and `~/.claude/plugins/installed_plugins.json` for plugins (e.g. superpowers, LSP plugins, vercel-plugin)
7. Proceed with the user's request. At session end, save via:
   ```bash
   cat <<'EOF' | temper resource create --type session --title "<title>" --context @me/<ctx>
   ## Goal
   ...
   EOF
   ```

## On Task Create

> Guided interactive task creation. Gathers context, title, mode, effort,
> goal linkage, and acceptance criteria through conversation.

1. If `--context @me/<ctx>` provided, use it. Otherwise list available contexts and ask.
2. Ask: "What's the title or problem statement for this task?"
3. Infer or ask mode:
   - "Is this (a) research/design/discovery (plan) or (b) implementation/building (build)?"
4. Infer or ask effort:
   - "How big is this? (a) small — single session, (b) medium — multi-step but bounded, (c) large — multi-session, may need decomposition"
5. List goals in context: `temper resource list --type goal --context @me/<ctx>`
   - If goals exist, ask: "Link to a goal? [list] or (none)"
6. Ask: "Any specific acceptance criteria or outcomes?" (optional — user can skip)
7. Create the task (pipe the problem statement and acceptance criteria via stdin):
   ```bash
   cat <<'EOF' | temper resource create --type task --title "<title>" --context @me/<ctx> --mode <mode> --effort <effort> [--goal <slug>]
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
| `task create [--context @me/<ctx>]` | On Task Create |
| `session start [--context @me/<ctx>]` | On Session Start |
| Anything touching a cognitive map (read/author a map, telos, nodes/edges, wayfind) | Read `cognitive-maps.md` |
| Other commands (search, session save, etc.) | Read `reference.md` for syntax |

## Cheap Orientation (read-side projection)

When you need to peek at a resource or scan a list without paying for the full
body, use the projection flags. They make orientation reads dramatically
cheaper, both in tokens and in API work:

- `temper resource show <ref> --meta-only` — frontmatter (managed +
  open) and hashes only; no body. Calls `GET /api/resources/<id>/meta`.
- `temper resource list --type <t> --context @me/<ctx> --meta-only` — meta tier per
  row instead of full row payloads.
- `--fields <a,b,c>` on either of the above — subselect top-level response
  keys (the anchor key — `id` or `resource_id` — is always preserved). For
  nested projection, pipe through `jq`.
- `temper resource show <ref> --edges` — adds the graph edges
  connected to this resource. Cannot be combined with `--meta-only`.

Reach for these first when triaging a context, comparing a few resources, or
deciding whether to read the body. Fall back to the full `show` only when you
need the body.

## Vault Projection (local cache)

The vault directory is a **read-only projection cache** of cloud state, not the
source of truth. To refresh missing or stale projected files:

```bash
temper pull <context>
```

Deleting a projected file with `rm` has no server effect — it just creates a
local cache miss. To actually delete a resource, use `temper resource delete
<ref> [--force]` (the `<ref>` is the resource's `ref` field from `list`/`show`).

## Cognitive Maps

A **context** homes resources as they are; a **cognitive map** homes *distilled nodes* in a
telos-governed graph (nodes · edges · facets · regions). They share storage but mean
different things — a map node is a **new** resource that distills from its source(s), never
the same row. Authoring into a map (the authored-4 under an invocation envelope,
provenance, fold-then-recreate supersession, the access model, and cross-map wayfind) is
its own discipline.

When a task involves reading from or authoring into a map, **read `cognitive-maps.md`** —
don't reconstruct the model from scratch. It cross-links the steward's `map-stewardship`
skill for the exhaustive per-call mechanics.

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
