# Build/Small Workflow

## When This Applies

Single-session implementation of a well-defined task. The scope is clear, no significant
design decisions are needed, and the work fits comfortably in one sitting. Examples: adding
a CLI flag, fixing a known bug, writing a utility function, updating a configuration.

## Steps

1. **Read the task** — run `temper resource show <slug> --type task` to load the full task content.
2. **Read project fundamentals** — if `guidance/fundamentals.md` exists in the skill
   directory, read it for project-specific conventions, test commands, and lint rules.
3. **Read subagent guidance** — read `subagent-guidance.md` and apply all principles
   throughout the session. This is not optional.
4. **Implement with tests** — write the code and corresponding tests. Follow existing
   patterns in the codebase. Keep changes minimal and focused on the task.
5. **Verify** — run the project's standard verification commands (test, lint, build) as
   documented in project fundamentals. If no fundamentals exist, use the project's
   conventional test and lint commands.
6. **Commit** — make a clean commit with a message that references the task context.

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

temper resource update <slug> --type task --stage done
```
