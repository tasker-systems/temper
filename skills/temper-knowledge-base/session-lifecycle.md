# Sessions

Read when starting, checkpointing or ending a session, or handing off to the next one. A session's
value to the next one is only what it writes down, and the next reader starts cold.

## Starting

Once the purpose is known, open on **what is in force**, not on the last note — several sessions run
concurrently, and "the last session" is often someone else's.
```
Tool: list_resources   Input: { "doc_type_name": "goal", "context_ref": "@me/<ctx>" }
Tool: list_resources   Input: { "doc_type_name": "task", "context_ref": "@me/<ctx>" }
Tool: list_resources   Input: { "doc_type_name": "session", "context_ref": "@me/<ctx>" }
Tool: search           Input: { "query": "<topic>" }
```
Each list returns a page: read `total` and `truncated` before calling anything absent.

Pick out your arc by title, then read that note deliberately. In a goal, note which criteria hold
and which are **declared uncovered** — a declared hole is information, not an omission to fix.

## During

- **Checkpoint after each major step** on medium and large work: what is done, what is next, any
  drift from the plan or the project's fundamentals.
- **Drift:** small work that needs design, or medium work that splits into deliverables, is under-sized;
  suggest the new mode or effort and update the task once the user agrees.

## The session note

A findings document, not a diary. Its parts, in order:

1. **Goal** — what the session set out to do, and what it continues (linked).
2. **What was learned** — the findings, most important first, each with the material that caused it
   ("the gate matched a literal `FROM <fn>(`, which a composed call never contains", not "fixed a
   gate bug").
3. **What I got wrong** — a claim made and disproved, a rationale invented for correct code, a green
   result that did not mean what it seemed. The class recurs after the instance is fixed.
4. **What happened** and **Decisions** — brief; decisions with who made them.
5. **Not done** — what did not run and whose it is, criteria still uncovered, verification skipped
   and why. Coverage is never inferred from absence.
6. **Connections** — every resource as `[exact title](./<full-uuid>)`.
7. **Next steps** — the handoff preamble (below), or a plain statement that the thread closed.

```
Tool: create_resource
Input: { "context_ref": "@me/<ctx>", "doc_type_name": "session", "title": "<title>", "content": "<the note>" }
```

## The handoff preamble

It instructs; it does not summarize. It carries:

1. **Decided — do not re-open.** Each settled question, named as closed.
2. **Do not re-derive — and where the grounding lives.** Point at the artifact.
3. **Rule before building.** Each open question whose answer changes what gets built, with its stakes.
4. **The trap,** if there is one: the tempting wrong implementation.

Test: would a cold agent given only this avoid the wrong turns available here? **No next session?**
Say what closed. Never manufacture a handoff to fill the slot; name a new starting point as new.

There is no slash-command router here, so give the user a block to paste as their next prompt:
`Continue in @me/<ctx>. Follows: <the note you just wrote>.`

## Closing the loop

- **The task:** its stage, and its body if the work changed shape — a superseded plan in a task body
  is a plan someone will follow.
- **The goal:** record evidence in its evidence record first, then rewrite only its current-state
  rows; a hole that has not closed is still stated, and the goal never gains a dated account
  (`outcome-registers.md`). This is a whole-body rewrite:
  `get_resource`, edit `content`, `update_resource` with the whole register — `content` replaces the
  body, so a fragment truncates the goal.
- **Extracted scope** becomes a task with the reasoning that moved it; scope only mentioned in a note
  has been dropped. Scaling work down is the user's call.
- **A memory that caught you:** record the date and, if the catch showed a shape its body lacks,
  amend the body (`memories.md`).
  `open_meta` is a per-key set here: read the stored `reinforced` list, merge, then write it whole.
- **Done work** ends with the note saved and the task's stage set to `done`.

## The closing message

Every note to the user carries a status, or it does not ship:

| Status | Meaning |
|---|---|
| **Hard follow** | Something is broken, or a gap shipped. Act on it |
| **Accepted** | A recorded tradeoff. No action |
| **For the record** | Context a later reader needs. No action |
| **Nothing** | Delete it |

Say the all-clear when there is no hard follow — clean work may read as clean. State the work's
standing in one line, and say what you need from the user, or that you need nothing.
