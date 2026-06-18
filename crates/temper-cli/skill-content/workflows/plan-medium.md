# Plan/Medium Workflow

## When This Applies

Discovery leading to a design specification. The problem space needs exploration before
committing to an implementation approach. The output is a spec document and optionally
follow-up build tasks. Examples: designing a new feature's architecture, evaluating
trade-offs between approaches, creating a technical design for a subsystem change.

## Steps

1. **Read the task** — find it via `temper resource list --type task` (copy its `ref`), then run `temper resource show <ref>` to load the full task content.
2. **Read project fundamentals** — if `guidance/fundamentals.md` exists in the skill
   directory, read it for project-specific conventions and architectural context.
3. **Discovery** — search for related work and context:
   - `temper search "<relevant terms>"` to find related documents
   - `temper context` to review the current context landscape
   - Check recent sessions for prior work in this area
4. **Brainstorm** — if the user has opted into a brainstorming skill, invoke it to
   explore the problem space. Otherwise, work through these questions inline:
   - What problem are we solving? What is the user-facing impact?
   - What are the constraints (technical, timeline, compatibility)?
   - What approaches exist? List at least two.
   - What are the trade-offs between approaches?
   - Present the analysis to the user before proceeding.
5. **Produce a design spec** — document the chosen approach:
   - Problem statement
   - Chosen approach and rationale
   - Components affected and their responsibilities
   - Key decisions and trade-offs accepted
   - Open questions or risks
6. **Save the spec and create follow-ups** — persist the design:
   ```bash
   cat <<'EOF' | temper resource create --type research --title "<spec title>" --context <ctx>
   <spec content>
   EOF
   ```
   If implementation tasks are clear, create them:
   ```bash
   temper resource create --type task --title "<build task title>" --context <ctx> --mode build --effort <effort>
   ```

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

temper resource update <ref> --stage done
```
