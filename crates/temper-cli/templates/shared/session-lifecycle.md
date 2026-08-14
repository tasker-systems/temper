# Session Lifecycle

## Session Start

**Open on standing state, not on narrative.** What is in force — active goals and their criteria —
is the thing a new session needs. The previous session note is a *pointer* to an arc, not a
description of where the work stands, and on a machine other than the one that wrote it, it can be
confidently misleading: several sessions run concurrently, and "the last session" is frequently
someone else's.

1. Read what is in force:
{%- if surface == "cli" %}
   ```bash
   temper resource list --type goal --context @me/<current> --status active --all
   temper resource show <goal-ref>          # the register: clauses, negative face, coverage state
   ```
{%- else %}
   ```
   Tool: list_resources   Input: { "doc_type_name": "goal", "context_ref": "@me/<current>" }
   Tool: get_resource     Input: { "id": "<goal uuid>" }   // the register: clauses, negative face, coverage
   ```
{%- endif %}
   Note which criteria hold, which are superseded, and which are **declared uncovered** — a declared
   hole is information, not an omission to fix. See `outcome-registers.md` if you will author or
   amend a goal this session.
2. Check what is open:
{%- if surface == "cli" %}
   ```bash
   temper resource list --type task --context @me/<current> --stage in-progress --all
   ```
{%- else %}
   ```
   Tool: list_resources   Input: { "doc_type_name": "task", "context_ref": "@me/<current>" }
   ```
   The tool returns a page. Read `total` / `returned` / `truncated` before concluding a task is
   absent, and narrow or page rather than asserting from one call.
{%- endif %}
3. Check recent sessions **by title** for the current context — enough to recognise which arc is
   yours, then read that one deliberately:
{%- if surface == "cli" %}
   ```bash
   temper resource list --type session --context @me/<current> --fields ref,title,updated
   ```
{%- else %}
   ```
   Tool: list_resources   Input: { "doc_type_name": "session", "context_ref": "@me/<current>" }
   ```
{%- endif %}
4. Search for relevant context:
{%- if surface == "cli" %}
   ```bash
   temper search "<topic>"
   ```
5. If starting via `task start <slug>` (skill command), load the task and route by mode/effort.
{%- else %}
   ```
   Tool: search   Input: { "query": "<topic>" }
   ```
5. If the user named a task, load it with `get_resource` and route by its `mode` / `effort`.
{%- endif %}

## Session End

{% if surface == "cli" -%}
Always pipe content via stdin. Without stdin, `resource create --type session` creates
placeholder boilerplate that must be edited manually.

```bash
cat <<'EOF' | temper resource create --type session --title "<title>" --context @me/<ctx>
## Goal
What we set out to do

## What Happened
Key actions, decisions, and outcomes

## Decisions
Choices made and why

## Connections
Related tasks, concepts, or contexts touched

## Next Steps
What to pick up next session
EOF
```
{%- else -%}
The whole note goes in `content` on one `create_resource` call. There is no placeholder path and
no second step — the server chunks and embeds inline and returns the finished resource.

```
Tool: create_resource
Input: {
  "context_ref": "@me/<ctx>",
  "doc_type_name": "session",
  "title": "<title>",
  "content": "## Goal\nWhat we set out to do\n\n## What Happened\nKey actions, decisions, and outcomes\n\n## Decisions\nChoices made and why\n\n## Connections\nRelated tasks, concepts, or contexts touched\n\n## Next Steps\nWhat to pick up next session"
}
```
{%- endif %}

> **Every resource a session note names goes in as `[title](./<full-uuidv7>)`.** Session notes are
> the most reference-dense documents anyone writes here, and `Connections` is nearly all references —
> so this is where an abbreviated id does the most damage. A UUIDv7 prefix is a **timestamp**: the
> goal and the task created a minute after it share their first seven characters, so a prefix names
> whichever of them the reader guesses. See *Referencing Other Resources* in `SKILL.md`.

Link the session to a task by updating the task's stage after saving:
{% if surface == "cli" -%}
```bash
temper resource update <ref> --stage done
```
{%- else -%}
```
Tool: update_resource_meta
Input: { "id": "<task uuid>", "managed_meta": { "temper-stage": "done" }, "open_meta": {} }
// Both tiers are required fields. Each is a per-key patch, so `"open_meta": {}` touches nothing.
```
{%- endif %}

**If the session moved a goal's criteria, close on them too.** Update the goal's *Exercise status*
and its declared coverage state — what now runs as distinct from what merely merged, and which
criteria are still uncovered and why. Coverage is never inferred from absence, so a hole that has
not been closed must still be **stated**. The register is the goal's body, so this is a body rewrite:

{% if surface == "cli" -%}
```bash
# `show` prints a serialized record, not the markdown body — json or toon depending on the format
# in force (`--format` → TEMPER_FORMAT → `[cli]` config → toon on a TTY, json otherwise). NEITHER
# is the body, so pin the format and extract `content`; a bare `show > file` piped back would
# write the serialized record as the body.
temper resource show <goal-ref> --format json | jq -r .content > register.md
# edit register.md, then:
cat register.md | temper resource update <goal-ref>
```
{%- else -%}
```
Tool: get_resource      Input: { "id": "<goal uuid>" }        // read `content`, edit it whole
Tool: update_resource   Input: { "id": "<goal uuid>", "content": "<the amended register>" }
```

`update_resource`'s `content` **replaces** the body. Send the whole amended register, never a
fragment — a partial body is a silent truncation of the goal, not an append.
{%- endif %}

## Handing Off to the Next Session

**End by echoing the invocation that starts the next session, ready to paste.** The handoff is worth
nothing if the user has to assemble it from four resource ids scattered through a note. What goes
*in* the preamble is `session-wrap.md`'s subject; this is the literal form.

Take the ids from the calls you just made — the session note's id comes back in the create response,
so capture it rather than looking it up. **Full UUIDv7s, never abbreviated**: a prefix is a
timestamp, so the goal and the task written a minute later share theirs.

{% if surface == "cli" -%}
```
/temper session start --context @me/<ctx> \
  --task <task-uuid> \
  --goal <goal-uuid> \
  --from-session <the-note-you-just-wrote> \
  --note "<the preamble>"
```

**These are skill-command arguments, not `temper` CLI flags.** There is no `temper session start`
binary subcommand; the router in `SKILL.md` reads them. Writing them as flags is a convention that
makes the handoff copy-pasteable, nothing more.
{%- else -%}
There is no slash-command router on this surface, so the handoff is a block the user pastes as their
next prompt. Same four facts, spelled as prose:

```
Continue in @me/<ctx>.
Task:    <task-uuid>
Goal:    <goal-uuid>
Follows: <the session note you just wrote>

<the preamble>
```
{%- endif %}

**Every field is conditional, and an omitted one is an honest statement.** Carry the task only if a
task genuinely continues, the goal only if the work sits under one, and the preceding session always,
since you just wrote it. A handoff naming a task that closed points the next session at finished
work.

### When there is no next session, say that instead

**Do not manufacture an invocation to fill the slot.** Some sessions end the work: a goal closes, a
task is done and nothing succeeds it, the arc finishes. Emitting a `session start` line there implies
work remains and sends the next reader looking for it.

Say plainly that the thread is closed and what closed it. If something *else* is now the obvious next
thing, name it as a new starting point rather than as a continuation of this one — those are
different claims, and only the second is a handoff.

This is the same rule as the all-clear below: an ending is information, and the habit of filling
every slot is what suppresses it.

## Closing Notes Carry a Status, or They Don't Ship

This governs the **summary you write to the user at the end of a session**, not the session
resource. It exists because an unlabeled critical note makes the reader run a triage pass to work
out whether it is actionable — attention spent on your prose instead of on the work.

**Every closing note carries one of four statuses. A note that cannot take one does not ship.**

| Status | Meaning |
|---|---|
| **Hard follow** | Something is broken, or a gap shipped. Act on it |
| **Accepted** | A real tradeoff, decided and recorded so it is not re-litigated. No action |
| **For the record** | Context a future reader needs. No action now |
| **Nothing** | Delete it. This is where the genre noise lives |

**When a session has no hard follows, say so explicitly.** This is the higher-value half. The habit
suppresses *"this shipped clean"* because a clean ending feels unfinished — so a reader may never
receive a plain all-clear even when one is true. An all-clear is information.

**The self-check**: *am I reaching for this note because I can point at what caused it, or because
the paragraph feels unfinished without it?* The second is the failure. Work that is genuinely clean
is allowed to read as clean.

**Why the slot generates false notes.** During the work, criticism is *caused by material* — you
read a constant, something is wrong, you say so. In a summary there is no fresh material, so a
critical note has to be generated to fill the slot, and generated criticism reaches for whatever is
nearest. That is how a true observation gets deformed into a false one: a real *cost* finding once
became a *credit deduction* that was simply false, because "here is a cost fact" did not scan as a
humbling closing note.

**This restrains summary-time criticism only.** Nothing here reduces grounding rigor, adversarial
review, or in-flight self-correction — those are caused by material and stay exactly as they are.

## Mid-Session Drift Detection

Watch for mismatches between assigned mode/effort and actual work:

| Signal | Likely Drift | Action |
|--------|-------------|--------|
| build/small needing design decisions, touching 3+ areas | Effort too low | Suggest build/medium |
| build/medium needing decomposition into multiple deliverables | Effort too low | Suggest build/large, create sub-tasks |
| plan/large with obvious first task, roadmap has 1-2 items | Effort too high | Suggest plan/medium or start building |
| Software task hitting non-software questions | Domain mismatch | Pause, reassess scope |

On confirmation, update the task:
{% if surface == "cli" -%}
```bash
temper resource update <ref> --mode <new> --effort <new>
```
{%- else -%}
```
Tool: update_resource_meta
Input: { "id": "<task uuid>", "open_meta": {},
         "managed_meta": { "temper-mode": "<new>", "temper-effort": "<new>" } }
```
{%- endif %}

## Checkpoint Pattern

For medium and large efforts, checkpoint after each major step:

> "Checkpoint: (1) What's done, (2) What's next, (3) Any concerns about approach drift,
> (4) Does anything conflict with project fundamentals?"

Checkpoints serve two purposes:
- **Visibility:** The user knows where things stand without asking.
- **Correction:** Drift caught early costs minutes; drift caught late costs sessions.

For large efforts, consider saving a mid-session note if a checkpoint reveals significant
decisions or direction changes worth preserving.
