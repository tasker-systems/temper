# Build/Medium Workflow

## When This Applies

Multi-step implementation that touches multiple areas of the codebase. Needs design
decisions, careful ordering, and verification across components. Still fits in a single
session but benefits from upfront planning. Examples: adding a new API endpoint with
handler/service/tests, refactoring a subsystem, implementing a feature with UI and backend
changes.

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
   - What is the implementation order?
   - What are the risks or edge cases?
   - Present the approach to the user for approval before coding.
6. **Planning phase** — if the user has opted into a planning skill, invoke it now.
   Otherwise, list concrete implementation steps with:
   - Files to create or modify
   - Order of operations
   - Verification criteria for each step
7. **Implement per plan** — work through the plan step by step. After each significant
   piece, run targeted tests to catch issues early.
8. **Full verification** — run the complete verification suite (test, lint, build) as
   documented in project fundamentals.
9. **Commit** — make a clean commit with a message that references the task context.

## Completion

Pipe the session summary via stdin to save it, then mark the task done:

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

temper resource update <slug> --type task --context <ctx> --stage done
```
