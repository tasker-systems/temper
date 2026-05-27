# Plan/Large Workflow

## When This Applies

Deep discovery leading to a goal roadmap. The problem space is broad or poorly understood
and needs systematic mapping before any implementation begins. The output is a structured
goal with sequenced deliverables. Examples: planning a major feature epic, mapping a
migration strategy, designing a new subsystem from scratch, scoping a multi-week effort.

The roadmap guides session work, not task-spread. Each session: work the current task,
learn, evolve the roadmap, create the next task.

## Steps

1. **Read the task** — run `temper resource show <slug> --type task` to load the full task content.
2. **Deep discovery** — cast a wide net:
   - `temper search "<relevant terms>"` across multiple angles
   - `temper context` to review the current context landscape
   - Codebase exploration: read key files, trace data flows, map dependencies
   - Check recent sessions: `temper resource list --type session --context <ctx>`
3. **Map the problem space** — if the user has opted into a brainstorming skill, invoke
   it to MAP the problem space, NOT to design an implementation. Otherwise, explore
   these questions inline:
   - What are the sub-problems? List them exhaustively.
   - What is the dependency order between sub-problems?
   - What are the unknowns that need resolution before building?
   - What external constraints exist (APIs, compatibility, performance)?
   - Where are the highest-risk areas?
   - Present the map to the user before proceeding.
4. **Produce a goal roadmap** — create a structured goal:
   ```bash
   temper resource create --type goal --title "<goal title>" --context <ctx>
   ```
   The roadmap should include:
   - Throughline summary: what this goal achieves and why it matters
   - Sequenced deliverable chunks, each sized for a single session
   - Validation gates: how to know each chunk is done
   - Open questions that need resolution during implementation
   - Dependencies between chunks
5. **Create the FIRST actionable task** — pick the first chunk from the roadmap:
   ```bash
   temper resource create --type task --title "<first task title>" --context <ctx> --mode build --effort <effort>
   ```
6. **Code only if pushed** — the primary output of plan/large is the roadmap and first
   task, not code. Only write code if the user actively requests it in this session.

## Completion

Pipe the session summary via stdin. Plan/large tasks may not reach done in a single
session — that is expected. Use the appropriate stage:

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

If the roadmap is complete and the first task is created, the plan/large task is done even
though the actual implementation work has not started. The roadmap is the deliverable.
