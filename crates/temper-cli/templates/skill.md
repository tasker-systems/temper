---
name: temper
description: Use when managing knowledge base tasks, sessions, or search — task start/create/done, session save, semantic search, context discovery, or any /temper command invocation
---

<!-- config-hash: {{ config_hash }} -->

# Temper Workflow Skill

## Contexts
{{ context_list }}

Address a context by ref: `@me/<slug>` for your own, `+team-slug/<slug>` for a team. Bare context names are **not accepted**.

## How This Skill Works

This is a modular skill. SKILL.md (this file) is the router — it tells you what to
read and when. Behavioral content lives in supporting files. Do NOT read all files
upfront; read only what the current task requires.

**Read everything as principles, not checklists**: a worked example is evidence *for* its
principle, never the scope of it.

### Supporting Files
- `reference.md` — CLI commands, stages, mode/effort, listing flags, body-source precedence
- `subagent-guidance.md` — 10 universal principles for dispatched subagents
- `plan-verification.md` — Verify a plan's claims against the code **before** dispatching from it
- `implementation-grounding.md` — Writing a plan, or code from one — including you in the main
  loop. Inject verbatim into plan-writing/implementing subagents
- `outcome-registers.md` — Stating an outcome so its rigor survives decomposition. For
  **authoring or amending a goal**, not to start a task
- `data-artifacts.md` — **Before storing structured data**: data artifact vs. resource body,
  the selection vocabulary, shape state
- `session-lifecycle.md` — Session start/end patterns, drift detection, checkpoints
- `session-wrap.md` — **Ending a session**: what the note must hold, and the handoff preamble
- `memories.md` — Durable memories: the `memory` type, populating the store, this machine's `MEMORY.md`
- `knowledge-base.md` — MCP resources and tools for cloud knowledge base access
- `cognitive-maps.md` — Reading from and authoring into cognitive maps (telos-governed graphs)
- `teams.md` — Teams: create, invite (email as correlator), join, roles, offboarding
- `querying.md` — **Which door to ask through** (`search` vs `query`), compositions, reading a trace

### Workflow Files (`workflows/`)
One file per mode/effort combination. Read only the one that matches the current task.

### Extension Files (`guidance/`)
The project's own rules — `guidance/fundamentals.md` holds its conventions if it exists. Read
them before substantive work in a project; `/temper init` offers to create them where missing.

## Outcome Discipline — applies to every task, whether or not you author a goal

Four rules, always in force; the reasoning and worked failures are in `outcome-registers.md`
(read **only** when authoring or amending a goal or sub-goal).

- **A clause states what must be true, or must never be true, and names no mechanism.** A clause
  naming a mechanism commits every reader to it — most damagingly the implementing agent, which
  builds the mechanism a clause named rather than the outcome it meant.
- **A witness is authored during the build, never in a preamble.** A witness must fail against the
  state its clause claims to change; against an unbuilt mechanism anything passes vacuously. A
  clause whose mechanism is unbuilt carries a **declared hole, not a filed task**.
- **Coverage is never inferred from absence.** A criterion with no evidence says so explicitly; a
  retired check leaves a named remainder, not a gap that reads as clean.
- **The discipline detects; it does not decide.** Reporting that something never ran never
  concludes the work should be dropped — cost and whether to pursue belong to the user.

## On Task Start

> **Addressing**: every `resource list`/`search`/`show` row carries a `ref` — copy it.
> `resource show`/`update`/`delete` take a single `<ref>` (no `--type`/`--context`);
> `resource create`/`list` still take `--type`/`--context`. Stages: `backlog`,
> `in-progress`, `done`, `cancelled` (not "active"). Full details: `reference.md`.

There is no `task start` CLI command; the sequence is:

1. Resolve the task's ref: `temper resource list --type task --context @me/<ctx>`, find the row matching `<slug>`, copy its `ref`. Read it via `temper resource show <ref>` — extract mode and effort
2. Move the task to in-progress: `temper resource update <ref> --stage in-progress`
3. If mode or effort is missing, ask: "What mode (plan/build) and effort (small/medium/large)?"
4. Apply *Outcome Discipline* above. Check `guidance/fundamentals.md`: apply it if present; if
   not, offer "This context has no project fundamentals. Want to set them up? (`/temper init`)".
   If this task authors or amends a goal, read `outcome-registers.md` first
5. Read `workflows/{mode}-{effort}.md` and follow it

## On Task Resume

1. Resolve the task's ref via `temper resource list --type task --context @me/<ctx>`, then `temper resource show <ref>` — extract mode, effort, and context
2. List recent sessions: `temper resource list --type session --context @me/<ctx>`
3. Read the most recent session note: match a row by its `slug`/`title` (a unique substring is
   enough), copy that row's `ref`, then `temper resource show <ref>` — its "Next Steps" is where
   you resume
4. If the task is not already in-progress, move it: `temper resource update <ref> --stage in-progress`
5. Apply *Outcome Discipline*; if this task authors or amends a goal, read `outcome-registers.md` first
6. Read `workflows/{mode}-{effort}.md` and continue from where the last session left off

## On Session Start

> Start a working session without a predefined task. Useful for exploration,
> ad-hoc work, or when a task hasn't been created yet.

**The purpose comes first.** Every step below runs only once the purpose is known — a
session ritual performed before the purpose is known is effort spent on the wrong question.

1. Get the purpose: use `--purpose <text>` when provided; otherwise **ask before doing
   anything else** — no task listing, no guidance reads, no scans until it is answered.
2. If `--context @me/<ctx>` provided, use it. Otherwise ask which context to work in.
3. List in-progress tasks: `temper resource list --type task --context @me/<ctx>`. If one
   matches the purpose, ask "Working on this?" — if yes, pivot to **On Task Resume** with that
   slug; otherwise continue as an open session.
4. Proceed with the purpose. Load the heavy context **lazily**, when the work reaches it:
   - `workflows/{mode}-{effort}.md` — when the session's mode/effort is known
   - `plan-verification.md` / `implementation-grounding.md` — when writing or executing a plan,
     or dispatching subagents
   - `outcome-registers.md` — before authoring or amending a goal
5. At session end, save via:
   ```bash
   cat <<'EOF' | temper resource create --type session --title "<title>" --context @me/<ctx>
   ## Goal
   ...
   EOF
   ```

## On Task Create

> Guided interactive task creation. Gathers context, title, mode, effort,
> and acceptance criteria through conversation.

1. If `--context @me/<ctx>` provided, use it. Otherwise list available contexts and ask.
2. Ask: "What's the title or problem statement for this task?"
3. Infer or ask mode:
   - "Is this (a) research/design/discovery (plan) or (b) implementation/building (build)?"
4. Infer or ask effort:
   - "How big is this? (a) small — single session, (b) medium — multi-step but bounded, (c) large — multi-session, may need decomposition"
5. Ask: "Any specific acceptance criteria or outcomes?" (optional — user can skip)
6. Optionally link a goal: pass `--goal <ref>` (a goal resource's UUID or decorated
   `slug-<uuid>` ref, from `temper resource list --type goal`). This projects a live
   `advances`→goal edge; later, `temper resource list --type task --goal <ref>` filters
   tasks by it, and `temper resource update <ref> --clear-goal` retracts the link.
7. Create the task (pipe the problem statement and acceptance criteria via stdin — the pattern is
   `cat <<'EOF' | temper resource create --type task --title "<title>" --context @me/<ctx> --mode <mode> --effort <effort> [--goal <ref>]`, body `# <title>`, problem statement, `## Acceptance Criteria`)
8. Ask: "Task created. Want to start working on it now?"
   - If yes: pivot to **On Task Start** with the new slug

## Command Routing

| Invocation Pattern | Route To |
|-------------------|----------|
| `task start <slug>` | On Task Start |
| `task resume <slug>` | On Task Resume |
| `task create [--context @me/<ctx>]` | On Task Create |
| `session start [--context @me/<ctx>] [--purpose <text>]` | On Session Start |
| `session wrap` | Read `session-wrap.md`, then follow *Session End* in `session-lifecycle.md` |
| Authoring or amending a goal or sub-goal, or deciding whether a criterion belongs on one | Read `outcome-registers.md` |
| Storing structured data (JSON, YAML, measurements, query plans) a later session must retrieve whole | Read `data-artifacts.md` |
| Anything touching a cognitive map (read/author a map, telos, nodes/edges, regions) | Read `cognitive-maps.md` |
| Block-level / segmented / attributable writes (per-block provenance/sources, citation-grade docs, `annotate`, `segmented_ingest` lifecycle) | Read `reference.md` → *Block-Grain Ingest & Attribution* |
| Asking a question of the knowledge base — deciding between `temper search` and `temper query`, writing or debugging a composition | Read `querying.md` |
| Other commands (search, session save, etc.) | Read `reference.md` for syntax |

## Listing Is Truncated — Enumerate Before Asserting

> **Never claim a goal/task/session is absent, or that a set is complete, from a
> default `temper resource list`.** The list returns a capped page (20 rows),
> so a resource you "don't see" may just be past the cap — this has repeatedly
> led agents to assert wrong backlog/status.

Every list response carries `total` (all matching rows), `returned` (this page),
and `truncated`. When `truncated` is `true`, there is more than you can see:
**narrow** (`--title-contains`, `--stage`, `--status`, `--sort`) or **enumerate
fully** (`--all`, a larger `--limit`, `--page`). Full flag set: `reference.md` →
*Listing: truncation, sort, and filters*.

## Cheap Orientation (read-side projection)

`show` and `list` answer in the same shape; `--with <section>` / `--without <section>`
say which parts you want — `body`, `open-meta`, `edges` (the managed tier is always
there, so it is not a section).

- `temper resource show <ref> --without body` — full view minus the body; skips the round-trip
- `temper resource list … --with open-meta` — rows plus the open tier, no bodies: triages a whole
  context in one call (`list` offers no `--with body`: use `show` per row)
- `--fields <a,b,c>` — subselect top-level response keys; pipe through `jq` for nested
- `--edges` (long form `--with edges`) — adds graph edges; **composes** with `--without body`

Reach for these whenever you need metadata but not prose. Naming one section in both
`--with` and `--without` is an error, not a precedence rule. Full table: `reference.md` →
*Orientation Projection*.

## Referencing Other Resources — full UUIDv7, and link it

**A UUID is not a SHA. Never abbreviate one.** A UUIDv7's leading bits are a **timestamp**,
so resources created near each other share a prefix *by construction* — a goal and the task
written a minute later routinely agree on their first seven characters, and the prefix
resolves to nothing a reader can follow. Write the full 36 characters everywhere: prose,
tables, `open_meta`, commit messages.

**When a document refers to another resource, write it as a markdown link:**

```markdown
[<the resource's exact title>](./<full-uuidv7>)
```

Resources are addressed **flatly** — there is no directory tree to be relative to — so
`./<uuid>` is the entire path and resolves wherever the body renders. Take the title from
`resource list`/`show`, not from memory (an approximate title is a citation that looks
precise and is not), and escape any `[`, `]`, `(` or `)` it contains, or the link will not
render. To delete a resource: `temper resource delete <ref> [--force]`.

## Editing Frontmatter vs Body — Avoid the stdin Footgun

`temper resource update <ref>` treats **implicit non-TTY stdin as a full-body
rewrite**. To change only frontmatter (e.g. `--title`, `--stage`), invoke
`update` **one resource per call with stdin untouched**. **Never** run `update`
inside a redirected loop:

```bash
# WRONG — each `update` inherits the loop's stdin (refs.txt) and rewrites the
# body with the leftover lines; one resource clobbered, the rest skipped, no error.
while read n ref; do temper resource update "$ref" --title "…"; done < refs.txt
```
Rewrite a body only with an explicit, intended `cat file.md | temper resource
update <ref>` (or `--body @file.md`, which always wins over stdin). Full
precedence table: `reference.md` → *Body Source*.

**A body write replaces the WHOLE body, so verify what you are about to send.**
`show` hands you a *snapshot*; the gap between reading it and writing it back is
where content disappears — to another session, another machine, or your own splice.

- **Re-read immediately before writing**, not once at the start of the work
- **Assert at BOTH ends of a splice** — confirming your new text landed says nothing
  about what it displaced; prefer counting sections or naming markers over trusting a
  byte count
- **Prefer the narrowest write** — one spliced line beats regenerating a document you
  did not author

This is operating discipline, **not a lock** — cloud writes are deliberately not mutexed,
and the same care is what a stale local edit needs anyway.

## Cognitive Maps

A **context** homes resources as they are; a **cognitive map** homes *distilled nodes* in a
telos-governed graph — a map node is a **new** resource that distills from its source(s), never
the same row. Anything touching a map — reading, and especially **authoring** into one (the
authored-4 under an invocation envelope, provenance, fold-then-recreate supersession) — is its
own discipline: **read `cognitive-maps.md`**, don't reconstruct the model from scratch.

## Subagent Dispatch

Before dispatching any subagent:
1. Read `subagent-guidance.md`
2. Include all applicable principles in the subagent prompt (verbatim, not summarized)
3. Include project fundamentals from `guidance/fundamentals.md` if available
4. **If the subagent will write a plan, or write code from one, inject
   `implementation-grounding.md` verbatim.** That is what it exists for, and it is the
   guidance most often skipped.
5. Include any user-selected plugin skills

> **This applies to you, too.** In the main loop nobody injects anything into you — load
> `implementation-grounding.md` yourself, and apply it to your own drafting, before you write a
> plan or ask anyone else to follow one.

**Skills and plugins are looked up on request, not on arrival.** The first time a session
is about to dispatch a subagent — or the user asks about quality gates or available skills
— scan `~/.claude/skills/` and `~/.claude/plugins/installed_plugins.json` (e.g.
superpowers, LSP plugins, vercel), check auto-memory for skills the user said they rely
on, present the list, and ask what subagents should use. Do not run this scan during
session or task start.

## Session Lifecycle & Memories

**Ending a session? Read `session-wrap.md` first** — it governs what the note must hold and how to
write the handoff preamble; the save pattern lives in `session-lifecycle.md`, which also carries
the session-start checklist, mid-session drift detection, and checkpoints.

**Read `memories.md` before assuming this machine's memories are in Temper** — `MEMORY.md` is a
generated projection, not a hand-edited file, and `temper memory status` reports what is still
local-only.
