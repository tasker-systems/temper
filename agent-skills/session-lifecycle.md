# Session Lifecycle

## Session Start

**Open on standing state, not on narrative.** What is in force — active goals and their criteria —
is the thing a new session needs. The previous session note is a *pointer* to an arc, not a
description of where the work stands, and on a machine other than the one that wrote it, it can be
confidently misleading: several sessions run concurrently, and "the last session" is frequently
someone else's.

1. Read what is in force:
   ```
   Tool: list_resources   Input: { "doc_type_name": "goal", "context_ref": "@me/<current>" }
   Tool: get_resource     Input: { "id": "<goal uuid>" }   // the register: clauses, negative face, coverage
   ```
   Note which criteria hold, which are superseded, and which are **declared uncovered** — a declared
   hole is information, not an omission to fix. See `outcome-registers.md` if you will author or
   amend a goal this session.
2. Check what is open:
   ```
   Tool: list_resources   Input: { "doc_type_name": "task", "context_ref": "@me/<current>" }
   ```
   The tool returns a page. Read `total` / `returned` / `truncated` before concluding a task is
   absent, and narrow or page rather than asserting from one call.
3. Check recent sessions **by title** for the current context — enough to recognise which arc is
   yours, then read that one deliberately:
   ```
   Tool: list_resources   Input: { "doc_type_name": "session", "context_ref": "@me/<current>" }
   ```
4. Search for relevant context:
   ```
   Tool: search   Input: { "query": "<topic>" }
   ```
5. If the user named a task, load it with `get_resource` and route by its `mode` / `effort`.

## Session End

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

> **Every resource a session note names goes in as `[title](./<full-uuidv7>)`.** Session notes are
> the most reference-dense documents anyone writes here, and `Connections` is nearly all references —
> so this is where an abbreviated id does the most damage. A UUIDv7 prefix is a **timestamp**: the
> goal and the task created a minute after it share their first seven characters, so a prefix names
> whichever of them the reader guesses. See *Referencing Other Resources* in `SKILL.md`.

Link the session to a task by updating the task's stage after saving:
```
Tool: update_resource_meta
Input: { "id": "<task uuid>", "managed_meta": { "temper-stage": "done" }, "open_meta": {} }
// Both tiers are required fields. Each is a per-key patch, so `"open_meta": {}` touches nothing.
```

**If the session moved a goal's criteria, close on them too.** Update the goal's *Exercise status*
and its declared coverage state — what now runs as distinct from what merely merged, and which
criteria are still uncovered and why. Coverage is never inferred from absence, so a hole that has
not been closed must still be **stated**. The register is the goal's body, so this is a body rewrite:

```
Tool: get_resource      Input: { "id": "<goal uuid>" }        // read `content`, edit it whole
Tool: update_resource   Input: { "id": "<goal uuid>", "content": "<the amended register>" }
```

`update_resource`'s `content` **replaces** the body. Send the whole amended register, never a
fragment — a partial body is a silent truncation of the goal, not an append.

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
```
Tool: update_resource_meta
Input: { "id": "<task uuid>", "open_meta": {},
         "managed_meta": { "temper-mode": "<new>", "temper-effort": "<new>" } }
```

## Checkpoint Pattern

For medium and large efforts, checkpoint after each major step:

> "Checkpoint: (1) What's done, (2) What's next, (3) Any concerns about approach drift,
> (4) Does anything conflict with project fundamentals?"

Checkpoints serve two purposes:
- **Visibility:** The user knows where things stand without asking.
- **Correction:** Drift caught early costs minutes; drift caught late costs sessions.

For large efforts, consider saving a mid-session note if a checkpoint reveals significant
decisions or direction changes worth preserving.
