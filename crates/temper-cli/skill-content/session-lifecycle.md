# Session Lifecycle

## Session Start

**Open on standing state, not on narrative.** What is in force — active goals and their criteria —
is the thing a new session needs. The previous session note is a *pointer* to an arc, not a
description of where the work stands, and on a machine other than the one that wrote it, it can be
confidently misleading: several sessions run concurrently, and "the last session" is frequently
someone else's.

1. Read what is in force:
   ```bash
   temper resource list --type goal --context @me/<current> --status active --all
   temper resource show <goal-ref>          # the register: clauses, negative face, coverage state
   ```
   Note which criteria hold, which are superseded, and which are **declared uncovered** — a declared
   hole is information, not an omission to fix. See `outcome-registers.md` if you will author or
   amend a goal this session.
2. Check what is open:
   ```bash
   temper resource list --type task --context @me/<current> --stage in-progress --all
   ```
3. Check recent sessions **by title** for the current context — enough to recognise which arc is
   yours, then read that one deliberately:
   ```bash
   temper resource list --type session --context @me/<current> --fields ref,title,updated
   ```
4. Search for relevant context:
   ```bash
   temper search "<topic>"
   ```
5. If starting via `task start <slug>` (skill command), load the task and route by mode/effort.

## Session End

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

Link the session to a task by updating the task's stage after saving:
```bash
temper resource update <ref> --stage done
```

**If the session moved a goal's criteria, close on them too.** Update the goal's *Exercise status*
and its declared coverage state — what now runs as distinct from what merely merged, and which
criteria are still uncovered and why. Coverage is never inferred from absence, so a hole that has
not been closed must still be **stated**. The register is the goal's body, so this is a body rewrite:

```bash
# `show` emits JSON — the body is the `content` field. Redirecting `show` straight to a file and
# piping it back would set the body to a JSON blob.
temper resource show <goal-ref> --format json | jq -r .content > register.md
# edit register.md, then:
cat register.md | temper resource update <goal-ref>
```

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
```bash
temper resource update <ref> --mode <new> --effort <new>
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
