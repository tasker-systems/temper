# Sessions

Read when starting, checkpointing or ending a session, or handing off to the next one. A session's
value to the next one is only what it writes down, and the next reader starts cold.

## Starting

Once the purpose is known, open on **what is in force**, not on the last note — several sessions run
concurrently, and "the last session" is often someone else's.

{%- if surface == "cli" %}
```bash
temper resource list --type goal --context @me/<ctx> --status active --all
temper resource list --type task --context @me/<ctx> --stage in-progress --all
temper resource list --type session --context @me/<ctx> --fields ref,title,updated
temper search "<topic>"
```
{%- else %}
```
Tool: list_resources   Input: { "doc_type_name": "goal", "context_ref": "@me/<ctx>", "status": "active" }
Tool: list_resources   Input: { "doc_type_name": "task", "context_ref": "@me/<ctx>", "stage": "in-progress" }
Tool: list_resources   Input: { "doc_type_name": "session", "context_ref": "@me/<ctx>" }
Tool: search           Input: { "query": "<topic>" }
```
Each list returns a page: read `total` and `truncated` before calling anything absent.
{%- endif %}

Pick out your arc by title, then read that note deliberately. In a goal, note which clauses hold
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
5. **Not done** — what did not run and whose it is, clauses still uncovered, verification skipped
   and why. Coverage is never inferred from absence.
6. **Connections** — every resource as `[exact title](./<full-uuid>)`.
7. **Next steps** — the handoff preamble (below), or a plain statement that the thread closed.

{% if surface == "cli" -%}
```bash
cat note.md | temper resource create --type session --title "<title>" --context @me/<ctx> --task <task-slug>
```
Always pipe the body: without stdin, `create` writes only a title heading. `--task` takes the task's
`slug` from its list row and links the note to it, which is how a resume finds the task's latest
note. `create` prints the new resource as JSON; its `ref` is what `--from-session` takes.
{%- else -%}
```
Tool: create_resource
Input: { "context_ref": "@me/<ctx>", "doc_type_name": "session", "title": "<title>", "content": "<the note>" }
```
{%- endif %}

## The handoff preamble

It instructs; it does not summarize. It carries:

1. **Decided — do not re-open.** Each settled question, named as closed.
2. **Do not re-derive — and where the grounding lives.** Point at the artifact.
3. **Rule before building.** Each open question whose answer changes what gets built, with its stakes.
4. **The trap,** if there is one: the tempting wrong implementation.

Test: would a cold agent given only this avoid the wrong turns available here? **No next session?**
Say what closed. Never manufacture a handoff to fill the slot; name a new starting point as new.

{% if surface == "cli" -%}
Echo the thin invocation in your closing reply, not inside the note it points at:
`/temper session start --context @me/<ctx> --from-session <the-note-you-just-wrote>`. These are
skill-command arguments, not `temper` CLI flags.
{%- else -%}
There is no slash-command router here, so give the user a block to paste as their next prompt:
`Continue in @me/<ctx>. Follows: <the note you just wrote>.`
{%- endif %}

## Closing the loop

- **The task:** its stage, and its body if the work changed shape — a superseded plan in a task body
  is a plan someone will follow.
- **The goal:** record evidence in its evidence record first, then rewrite only its current-state
  rows; a hole that has not closed is still stated, and the goal never gains a dated account
  (`outcome-registers.md`). This is a whole-body rewrite:
{%- if surface == "cli" %}
  `temper resource show <ref> --format json | jq -r .content > register.md`, edit, then
  `cat register.md | temper resource update <ref>`. Check the exit code first: on error `.content`
  is `null`, and piping that back is a silent wrong answer.
{%- else %}
  `get_resource` with `"include_content": true`, edit `content`, `update_resource` with the whole register — `content` replaces the
  body, so a fragment truncates the goal.
{%- endif %}
- **Extracted scope** becomes a task with the reasoning that moved it; scope only mentioned in a note
  has been dropped. Scaling work down is the user's call.
- **A memory that caught you:** record the date and, if the catch showed a shape its body lacks,
  amend the body (`memories.md`).
{%- if surface == "cli" %}
  `temper resource update <memory-ref> --open-meta-add '{"reinforced":["YYYY-MM-DD"]}'` —
  never `--open-meta`, which replaces the list and discards every stored date.
{%- else %}
  `update_resource` with `"open_meta_add": { "reinforced": ["YYYY-MM-DD"] }`, which unions with the
  stored list; never `open_meta`, which replaces it.
{%- endif %}
- **The task's stage:** `done` only when its work is finished; work that continues into another
  session stays `in-progress`.

## The closing message

Every note to the user carries a status, or it does not ship:

| Status | Meaning |
|---|---|
| **Hard follow** | Something is broken, or a gap shipped. Act on it |
| **Accepted** | A recorded tradeoff. No action |
| **For the record** | Context a later reader needs. No action |
| **Nothing** | Drop the item from the message |

Say the all-clear when there is no hard follow — clean work may read as clean. State the work's
standing in one line, and say what you need from the user, or that you need nothing.
