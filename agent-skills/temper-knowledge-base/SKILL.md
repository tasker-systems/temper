---
name: temper-knowledge-base
description: Work with a Temper knowledge base over MCP — goals, tasks, sessions,
  research, decisions, search and contexts. Use when the user mentions their knowledge
  base, contexts, session notes, or a goal.
---

# Temper — Knowledge Base Workflow (MCP)

This is the **MCP packaging** of the temper workflow skill. Everything here happens through tool
calls against a Temper server. There is no vault to find, no `temper.toml` to read, and no local
directory that holds the truth — **the server is authoritative**, and it is the only thing you are
talking to.

> If you are working inside a checkout of the temper source repository with the `temper` CLI on
> PATH, you want the CLI packaging of this skill instead. This one speaks tools.

## What lives in a knowledge base

**Contexts** are workspaces that home resources. A context is addressed by **ref** — `@me/<slug>`
for your own, `+<team>/<slug>` for a team's, or a bare UUID. **Bare names are rejected**: pass
`"@me/temper"`, never `"temper"`. Every write and list input spells this field `context_ref`.

**Resources** are the documents themselves, each with a **doc type** that decides its schema.
`references/frontmatter.md` carries the complete, generated list — read it rather than guessing a
type name, because an unrecognized `doc_type_name` is a rejected write. The ones you will reach for
most are `goal`, `task`, `session`, `research`, `decision`, and `concept`.

**Cognitive maps** are a different home: a telos-governed graph of *distilled* nodes rather than
resources as they are. See *Declared gaps* below before working in one.

## How this skill works

This file is the router. Read a supporting file when the work calls for it, not upfront.

| File | Read it when |
|------|--------------|
| `knowledge-base.md` | **The tool reference.** Which tool for which intent, reads vs writes, orientation, block-grain ingest |
| `references/frontmatter.md` | Before any write — the doc types, the two metadata tiers, what you may and may not send |
| `session-lifecycle.md` | Starting or ending a working session; mid-session drift; checkpoints |
| `memories.md` | What a `memory`-typed resource is — how to read one, and how to author, correct and supersede one |
| `subagent-guidance.md` | Before dispatching any subagent |
| `plan-verification.md` | Before acting on a written plan's claims about code |
| `implementation-grounding.md` | Writing a plan, or writing code from one — including yourself |
| `outcome-registers.md` | **Authoring or amending a goal**, or deciding whether a criterion belongs on one |

> **Read these as principles, not checklists.** Each carries a worked example from the incident that
> produced it. The example is **evidence for** the principle, never the **scope of** it. So the
> thought *"this guidance doesn't cover my case"* is the failure mode itself, not a finding — and
> *"ritual A misses this, ritual B misses this, so I'll add ritual C"* is that failure mode
> mid-sentence, growing an unbounded catalogue of edge cases instead of reasoning from the theme.
> When you get there, stop and ask what the principle is **for**.

## Outcome Discipline — applies to every task, whether or not you author a goal

Four rules. They are short because they are always in force; the reasoning, the eight register
elements, and the worked failures are in `outcome-registers.md`, which you read **only** when
authoring or amending a goal or sub-goal.

- **A clause states what must be true, or must never be true, and names no mechanism.** The *how* is
  the part that mutates, and a clause naming one propagates that commitment to every reader — most
  damagingly to an implementing agent, which builds the mechanism the clause named rather than the
  outcome it meant.
- **A witness is authored during the build, never in a preamble.** A witness must fail against the
  state its clause claims to change. When the mechanism does not exist yet, "fails against current
  state" is satisfied by the *absence of the feature* — vacuously, by anything. A clause whose
  mechanism is unbuilt carries a **declared hole, not a filed task**.
- **Coverage is never inferred from absence.** A criterion with no evidence says so explicitly. A
  retired check leaves a named remainder, not a gap that reads as clean.
- **The discipline detects; it does not decide.** Reporting that something never ran, or that a
  criterion is uncovered, never concludes that the work should be dropped. Cost, churn and whether a
  goal is worth pursuing belong to the user, not to a criterion inside it.

## Starting work on a task

There is no `task start` tool. A task is a resource, and starting one is three calls:

```
Tool: list_resources   Input: { "doc_type_name": "task", "context_ref": "@me/<ctx>",
                                "stage": "backlog" }
Tool: get_resource     Input: { "id": "<task uuid>", "include_content": true }
Tool: update_resource_meta
Input: { "id": "<task uuid>",
         "managed_meta": { "temper-stage": "in-progress" },
         "open_meta": {} }
```

Stages are `backlog`, `in-progress`, `done`, `cancelled` — there is no "active".

**`update_resource_meta` takes both tiers.** `managed_meta` and `open_meta` are required fields on
that call, and each is a **per-key patch**: a key you omit is untouched, a key you send is replaced
whole. Sending `"open_meta": {}` therefore changes nothing — it is how you say "I am only touching
the managed tier".

A task's `temper-mode` (`plan` / `build`) and `temper-effort` (`small` / `medium` / `large`) say how
much process the work deserves. If either is missing, ask. If the work drifts away from what they
say, `session-lifecycle.md` has the drift table.

## Creating a task

```
Tool: create_resource
Input: {
  "context_ref": "@me/<ctx>",
  "doc_type_name": "task",
  "title": "<title>",
  "content": "# <title>\n\n<problem statement>\n\n## Acceptance Criteria\n\n<criteria>",
  "managed_meta": { "temper-stage": "backlog", "temper-mode": "build",
                    "temper-effort": "medium" },
  "goal": "<goal ref>"
}
```

`goal` is a first-class field, not metadata: it projects a live `advances`→goal edge, which is the
only thing `list_resources`' `goal` filter reads. `update_resource` carries `goal` to re-point it and
`clear_goal: true` to retract it.

## Listing is capped — enumerate before asserting

**Never claim a goal, task, or session is absent — or that a set is complete — from one
`list_resources` call.** The response is a **page**: default 50 rows, `limit` maxes at 200. It
carries `total` alongside `rows`, and `rows.len() < total` means there is more you have not seen.

Before asserting absence or completeness, either **narrow** (`doc_type_name`, `stage`, `status`,
`tags`, `goal`) or **page** (`limit` + `offset`) until you have the whole set. An agent that reads a
short page as the full backlog reports the wrong state confidently, which is worse than reporting
nothing.

## Referencing other resources — full UUIDv7, and link it

**A UUID is not a SHA. Never abbreviate one.** A UUIDv7's leading bits are a **timestamp**, so
resources created near each other share a prefix *by construction* — a goal and the task written a
minute later routinely agree on their first seven characters:

```
019fbb77-72a3-72e1-bbbd-13eb6aa64982   <- a goal
019fbb78-657b-7380-9063-212727cfe390   <- its task, 62 seconds later
```

A prefix is therefore *systematically* ambiguous between exactly the resources most likely to be
cited together, and resolves to nothing a reader can follow. Write the full 36 characters
everywhere — prose, tables, `open_meta`, commit messages.

**When a document refers to another resource, write it as a markdown link:**

```markdown
[<the resource's exact title>](./<full-uuidv7>)
```

Resources are addressed **flatly**; there is no directory tree to be relative to, so `./<uuid>` is
the entire path and it resolves wherever the body renders. The reader then sees *what* is cited
without a round-trip, and can navigate to it instead of copying an id into a tool call.

Take the title from `list_resources` / `get_resource`, not from memory — an approximate title inside
a link is a citation that looks precise and is not. Escape any `[`, `]`, `(` or `)` the title
contains, or the link will not render.

## Writing bodies

`create_resource` takes the whole markdown body in `content` and returns the finished resource — the
server chunks and embeds inline, so there is no second step and nothing to poll.

`update_resource`'s `content` **replaces** the body. Send the whole amended document, never a
fragment: a partial body is a silent truncation, not an append. To change only frontmatter, use
`update_resource_meta` and send no body at all.

**Because it replaces, verify what you are about to send.** `get_resource` hands you a *snapshot*,
and the gap between reading it and writing it back is where content disappears — to another session,
another machine, or your own splice. Both happened in one session: a concurrent edit from a second
machine was nearly overwritten, and then a single-section edit that spliced on a heading silently
truncated everything after it. The second had no concurrency at all.

- **Re-read immediately before writing**, not once at the start of the work. Assemble the amended
  document, then `get_resource` again and re-apply to what is actually stored now.
- **Assert at BOTH ends of a splice.** Confirming your new text landed says nothing about what it
  displaced. Check that the content you did *not* intend to touch survived, and prefer counting
  sections or naming markers over trusting a length.
- **Prefer the narrowest write** — `update_resource_meta` when only frontmatter changes, and one
  spliced section over regenerating a document you did not author.

This is operating discipline, not a lock. Writes are **not** mutexed, and deliberately so — locking
them would be overkill for essentially every workflow here.

For a body too large for one call, for a resumable build, or for citation-grade per-block
attribution, use the segmented `ingest_*` lifecycle and `annotate_resource` — `knowledge-base.md`
has both.

## Subagent dispatch

Before dispatching any subagent:

1. Read `subagent-guidance.md`.
2. Include the applicable principles in the prompt **verbatim, not summarized** — paraphrase is the
   loss that guidance exists to prevent.
3. If the subagent will write a plan, or write code from one, inject `implementation-grounding.md`
   verbatim. That is what it is for, and it is the one most often skipped.

> **This applies to you, too.** When *you* write a plan, nobody dispatches you, so nothing injects
> anything — and that is exactly how an ungrounded plan gets authored and then stamped "verified" by
> its own author.

## Declared gaps — what this packaging does not carry

Stated rather than left to be discovered, because coverage is never inferred from absence.

- **Cognitive maps.** The tool surface exists (`cogmap_*`, and `knowledge-base.md` covers the
  orientation trio), but the *authoring discipline* — the authored-4 under an invocation envelope,
  provenance, fold-then-recreate supersession, cross-map linking — ships only in the CLI packaging.
  Read from a map freely; before **authoring** into one, say so and ask.
- **Teams.** Creating teams, invitations, roles and offboarding have no guide here.
- **Per-cell workflow files.** The CLI packaging carries a file per `mode × effort` cell. They are
  written against CLI commands, so they are not shipped here rather than shipped wrong.
