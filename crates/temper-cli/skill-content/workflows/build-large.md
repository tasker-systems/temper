# Build/Large Workflow

## When This Applies

Multi-session implementation that is too large for a single sitting. The work spans many
files, requires iterative design, or involves sequential deliverables that each need their
own verification. Examples: building an entire new subsystem, migrating a major component,
implementing a feature that touches every layer of the stack.

Don't try to do everything in one session. Decompose, deliver one piece, and leave a clear
trail for the next session.

## Steps

1. **Read the task** — run `temper resource show <slug> --type task` to load the full task content.
2. **Read project fundamentals** — if `guidance/fundamentals.md` exists in the skill
   directory, read it for project-specific conventions, test commands, and lint rules.
3. **Discovery** — search for related work and context:
   - `temper search "<relevant terms>"` to find related documents
   - `temper context` to review the current context landscape
   - Check recent sessions for prior work in this area
4. **Read subagent guidance** — read `subagent-guidance.md` and apply all principles
   throughout the session.
5. **Design phase** — if the user has opted into a brainstorming skill, invoke it now.
   Otherwise, outline the approach inline:
   - What components are affected?
   - What is the implementation order and dependency graph?
   - What are the risks or edge cases?
   - How does the work decompose into session-sized pieces?
   - Present the approach to the user for approval before coding.
6. **Planning phase** — if the user has opted into a planning skill, invoke it now.
   Otherwise, list concrete implementation steps with:
   - Files to create or modify
   - Order of operations and session boundaries
   - Verification criteria for each step
7. **Implement this session's piece** — focus on one coherent deliverable. After each
   significant piece, run targeted tests to catch issues early.
8. **Full verification** — run the complete verification suite (test, lint, build) as
   documented in project fundamentals.
9. **Commit** — make a clean commit with a message that references the task context.
10. **Create sub-tasks for remaining work** — for each remaining piece:
    ```bash
    temper resource create --type task --title "<next piece title>" --context <ctx> --mode build --effort <effort>
    ```

## Session Rhythm

Each session follows this cycle:
1. Pick up the current task (or the next sub-task)
2. Implement, verify, commit
3. Save the session
4. Create the next task if work remains

## Completion

Pipe the session summary via stdin. Use `--stage done` only if THIS sub-task is complete.
The parent task may remain in-progress across sessions.

```bash
cat <<'EOF' | temper resource create --type session --title "<title>" --context <ctx>
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

temper resource update <slug> --type task --stage done
```

If the overall work is not yet finished, the next session picks up from the trail left here.
