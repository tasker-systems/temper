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

- **Copy the `ref` from any list, search or show row.** A ref is a UUID or `<slug>-<uuid>`; both are
  accepted, and the slug part is ignored. `show`/`update`/`delete` take one ref; only `create`/`list` take
  `--type`/`--context`. Stages (`backlog`, `in-progress`, `done`, `cancelled`) change with
  `temper resource update <ref> --stage <stage>`.
- **Never abbreviate a UUID** — v7's leading bits are a timestamp, so neighbours share a prefix.
  Refer to another resource as `[<its exact title>](./<full-uuid>)`, with the title copied from
  `list`/`show` (never recalled) and any `[`, `]`, `(` or `)` in it escaped.
- **A list is a page.** Check `total` and `truncated` before calling anything absent; narrow
  (`--title-contains`, `--stage`) or enumerate (`--all`).
- **Implicit stdin on `update` is a whole-body rewrite.** Never call `update` inside a
  `while read … < file` loop. A body write replaces everything: Re-read immediately before writing,
  assert at BOTH ends of a splice, prefer the narrowest write — discipline, not a lock.
- **Two writers to one meta key overwrite each other unless the write says what it expects.**
  `resource meta set` and `resource update` take `--expect '{"open":{"claim_id":null}}'` (a
  top-level `null` is "absent") and `--expect-absent open:<key>`: if any expectation fails, nothing
  is written, the command exits non-zero, and `--format json` prints code `PRECONDITION_FAILED`
  with each failed key's current value in `details.failed`. Guard on a value unique to you: a
  `current` equal to your own value means your earlier attempt landed.
- **Read only what you need:** `show <ref> --without body`, `list … --with open-meta`, `--fields`.

## Outcome discipline — every task, goal or not

- **A clause states what must be true, or must never be true, and names no mechanism still open at
  its altitude.** The implementer builds what a clause names. A goal's altitude is how far it sits
  from the work; a mechanism is open there until a cited `decision`, made by the goal's owner before
  the clause, chooses it. A chosen one is a prior: be precise about it.
- **A witness is authored during the build, never in a preamble** — it must fail against the state
  its clause claims to change, which nothing can before the mechanism exists. Until then the clause
  carries a **declared hole**: the register says it is not yet witnessed, and why. A task is filed
  when someone takes the hole up.
- **Coverage is never inferred from absence.** A clause with no evidence says so; a retired check
  names what it no longer covers.
- **The discipline detects; it does not decide.** Whether to pursue something is the user's call.

## Project fundamentals

`guidance/fundamentals.md` is a machine-local index from repo root to the project's own guidance
(usually its `AGENTS.md`). Read through to the repo before substantive work; the repo wins. No
entry for this project: offer `/temper init` (see `project-setup.md`).

## Commands

| `/temper …` | Do |
|---|---|
| `session start [--context] [--purpose] [--from-session <ref>]` | With `--from-session`, read that note first: its *Next Steps* is the purpose. Otherwise get the purpose first — ask before anything else if it is missing. If an in-progress task matches it, offer to resume that task. |
| `task start <slug>` / `task resume <slug>` | Find the task's ref (`resource list --type task --title-contains …`), set `--stage in-progress`, ask for mode and effort if absent. On resume, the task's latest session note's *Next Steps* is where you start. |
| `task create` | Title and problem, mode, effort (`small` one session, `medium` bounded, `large` multi-session), acceptance criteria; `--goal <ref>` links it. `plan` work produces a decision, spec or findings, saved as a `research` or `decision` resource (a large plan: a goal and its first linked task); `build` work produces code. |
| `session wrap` | Read `session-lifecycle.md`. A task is `done` only when its work is finished; work that continues stays `in-progress`. |

## Read when

| File | Read when |
|---|---|
| `outcome-registers.md` | Authoring or amending a goal, or deciding whether a criterion belongs on one |
| `working-a-goal.md` | Taking up, resuming, advancing, decomposing or closing a goal |
| `session-lifecycle.md` | Starting, checkpointing or ending a session, or handing off to a successor |
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
