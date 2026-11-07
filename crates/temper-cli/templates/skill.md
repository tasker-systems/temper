---
metadata:
  internal: true
name: temper
description: Use when work involves Temper, the knowledge base that carries goals, tasks, sessions, research and decisions across conversations — starting, resuming or wrapping a session or task, authoring or working a goal, searching or querying the knowledge base, storing structured data, cognitive maps, teams, memories, or any /temper command.
---

<!-- config-hash: {{ config_hash }} -->

# Temper

Temper is where work outlives the conversation: goals state outcomes, tasks advance them, sessions
record what happened and hand off, and research and decisions stay findable. The `temper` CLI is
agent-first (JSON on a non-TTY stdout); `temper <command> --help` is the syntax authority.

## Contexts
{{ context_list }}

Address a context by ref — `@me/<slug>` for your own, `+team-slug/<slug>` for a team. Bare names are
refused, so commands take `--context @me/<ctx>`.

## Always

- **Copy the `ref` from any list, search or show row.** `show`/`update`/`delete` take one ref; only
  `create`/`list` take `--type`/`--context`. Stages: `backlog`, `in-progress`, `done`, `cancelled`.
- **Never abbreviate a UUID** — v7's leading bits are a timestamp, so neighbours share a prefix.
  Refer to another resource as `[<its exact title>](./<full-uuid>)`.
- **A list is a page.** Check `total` and `truncated` before calling anything absent; narrow
  (`--title-contains`, `--stage`) or enumerate (`--all`).
- **Implicit stdin on `update` is a whole-body rewrite.** Never call `update` inside a
  `while read … < file` loop. A body write replaces everything: Re-read immediately before writing,
  assert at BOTH ends of a splice, prefer the narrowest write — discipline, not a lock.
- **Read only what you need:** `show <ref> --without body`, `list … --with open-meta`, `--fields`.

## Outcome discipline — every task, goal or not

- **A clause states what must be true, or must never be true, and names no mechanism still open at
  its altitude.** The implementer builds what a clause names. A mechanism chosen in a cited decision
  is a prior: be precise about it.
- **A witness is authored during the build, never in a preamble** — it must fail against the state
  its clause claims to change. Until then the clause carries a declared hole, not a filed task.
- **Coverage is never inferred from absence.** No evidence is said explicitly; a retired check
  leaves a named remainder.
- **The discipline detects; it does not decide.** Whether to pursue something is the user's call.

## Project fundamentals

`guidance/fundamentals.md` is a machine-local index from repo root to the project's own guidance
(usually its `AGENTS.md`). Read through to the repo before substantive work; the repo wins. No
entry for this project: offer `/temper init` (see `project-setup.md`).

## Commands

| `/temper …` | Do |
|---|---|
| `session start [--context] [--purpose]` | Get the purpose first — ask before anything else if it is missing. If an in-progress task matches it, offer to resume that task. |
| `task start <slug>` / `task resume <slug>` | Resolve the ref, set `--stage in-progress`, ask for mode and effort if absent. On resume, the latest session note's *Next Steps* is where you start. |
| `task create` | Title and problem, mode (`plan`: the output is a decision or spec; `build`: code), effort (`small` one session, `medium` bounded, `large` multi-session), acceptance criteria; `--goal <ref>` links it. |
| `session wrap` | Read `session-lifecycle.md`. Done work ends with a session note and `--stage done`. |

## Read when

| File | Read when |
|---|---|
| `outcome-registers.md` | Authoring or amending a goal, or deciding whether a criterion belongs on one |
| `working-a-goal.md` | Taking up, resuming, advancing, decomposing or closing a goal |
| `session-lifecycle.md` | Ending a session, checkpointing, or handing off to a successor |
| `implementation-grounding.md` | Writing a plan, writing code from one, briefing a subagent, or deciding what review a change owes before its PR |
| `data-artifacts.md` | Storing structured data (JSON, measurements, query plans) a later session must get back whole |
| `querying.md` | Asking the knowledge base a question — `search` vs `query`, compositions, reading a trace |
| `cognitive-maps.md` | Reading or authoring a cognitive map |
| `teams.md` | Creating, inviting to, joining or leaving a team |
| `memories.md` | Saving or reading durable memories, or this machine's `MEMORY.md` |
| `reference.md` | Command table, body-source precedence, listing flags, *Block-Grain Ingest & Attribution* |
| `knowledge-base.md` | Working through the MCP tools rather than the CLI |
| `project-setup.md` | `/temper init` |
{%- if include_admin %}
| `admin.md` | Administering the deployment — principals, machine clients, connections, instance settings, or an admin refusal |
{%- endif %}
