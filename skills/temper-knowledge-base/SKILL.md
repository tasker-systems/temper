---
name: temper-knowledge-base
description: Use when working with a Temper knowledge base over MCP — goals, tasks, sessions, research, decisions, memories, data artifacts, search or contexts — or when the user mentions their knowledge base, a context, a session note or a goal.
---

# Temper — Knowledge Base (MCP)

Temper is where work outlives the conversation: goals state outcomes, tasks advance them, sessions
record what happened and hand off, and research and decisions stay findable. Everything here is a
tool call against a Temper server, which is authoritative — there is no local vault. With the
`temper` CLI on PATH, the CLI packaging of this skill is the better fit.

## What lives in a knowledge base

- **Contexts** home resources. Address one by ref — `@me/<slug>`, `+<team>/<slug>` or a UUID — in
  `context_ref`. Bare names are refused.
- **Resources** are documents with a doc type (`goal`, `task`, `session`, `research`, `decision`,
  `concept`, `memory`, `evidence`). `references/frontmatter.md` lists the recognized types: a name
  outside it is accepted with no schema, so a typo silently becomes a new type. `evidence`, a goal's
  evidence record, is the one unschematized type used on purpose.
- **A ref** is a UUID or `<slug>-<uuid>`; both are accepted, and the slug part is ignored.
- **Cognitive maps** are a separate home: a telos-governed graph of distilled nodes.

## Always

- **Lists are pages** (default 50, max 200). Read `truncated` before calling anything absent; narrow
  (`doc_type_name`, `stage`, `status`, `tags`, `goal`) or page (`limit`, `offset`).
- **Never abbreviate a UUID** — v7's leading bits are a timestamp, so neighbours share a prefix.
  Refer to another resource as `[<its exact title>](./<full-uuid>)`, with the title copied from
  `list_resources`/`get_resource` (never recalled) and any `[`, `]`, `(` or `)` in it escaped.
- **`get_resource` returns the body only with `"include_content": true`.** Read it that way before
  any rewrite.
- **`update_resource`'s `content` replaces the whole body.** Re-read immediately before writing,
  assert at BOTH ends of a splice — discipline, not a lock. When only frontmatter changes, use
  `update_resource_meta`: it takes both tiers, each a per-key patch, so `"open_meta": {}` touches
  nothing.
- **A task's stage, mode and effort are `managed_meta` keys:** `temper-stage` (`backlog`,
  `in-progress`, `done`, `cancelled`; `list_resources`' `stage` filter takes the same values), `temper-mode` (`plan`: a decision, spec or findings, saved as
  a `research` or `decision` resource; `build`: code) and `temper-effort` (`small`, `medium`,
  `large`). Change one with `update_resource_meta`, e.g.
  `{ "id": "<uuid>", "managed_meta": { "temper-stage": "done" }, "open_meta": {} }`. A task is `done`
  only when its work is finished. Ask for mode and effort if either is missing.
- **Know what the session is for before listing anything.** If the user's ask is ambiguous, ask
  first; a note the user points you at supplies the purpose in its *Next Steps*.
- **A task links to its goal** through `create_resource`'s `goal` field (a live `advances` edge, the
  only thing `list_resources`' `goal` filter reads); `update_resource` re-points it or `clear_goal`s it.

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

## Read when

| File | Read when |
|---|---|
| `knowledge-base.md` | Choosing a tool — which one for which intent, and the traps the schemas cannot carry |
| `references/frontmatter.md` | Before any write — doc types and the two metadata tiers |
| `session-lifecycle.md` | Starting, checkpointing or ending a session, or handing off |
| `outcome-registers.md` | Authoring or amending a goal |
| `working-a-goal.md` | Taking up, resuming, advancing, decomposing or closing a goal |
| `implementation-grounding.md` | Writing a plan, writing code from one, briefing a subagent, or deciding what review a change owes |
| `data-artifacts.md` | Storing structured data a later session must get back whole |
| `memories.md` | Reading, authoring, correcting or superseding a `memory` |

## Declared gaps

This packaging does not carry the cognitive-map authoring discipline (read maps freely; before
authoring into one, say so and ask), team offboarding's bulk reassign, or `/temper init` — each
ships only with the CLI.
