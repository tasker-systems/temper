# Session Lifecycle

## Session Start

1. Check recent sessions for the current context:
   ```bash
   temper session list --context <current>
   ```
2. If resuming previous work, read the last session note for continuity.
3. Search for relevant context:
   ```bash
   temper search "<topic>"
   ```
4. If starting via `temper task start <slug>`, load the task and route by mode/effort.

## Session End

Always pipe content via stdin. Without stdin, `session save` creates placeholder boilerplate
that must be edited manually.

```bash
cat <<'EOF' | temper session save "<title>" --task <slug> --state <state>
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

The `--state` flag records the task's state at session end (e.g., `in-progress`, `done`).
The `--task` flag links the session to a specific task for traceability.

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
temper task move <slug> --mode <new> --effort <new>
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
