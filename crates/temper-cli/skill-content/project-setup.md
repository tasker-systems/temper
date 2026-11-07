# Project Setup (/temper init)

Read this when the operator runs `/temper init`, or when `guidance/fundamentals.md` has no entry
for the current project and you offered to add one. CLI only: the MCP surface has no equivalent.

## The model

- **The CLI wizard configures the machine; you orient the project**, in conversation.
- **A project's fundamentals live in its repo**: its `AGENTS.md`/`CLAUDE.md`, or a file the repo
  already carries. The repo is always the authority.
- **`guidance/fundamentals.md` is a one-hop index**: one line per project, matched by repo root,
  naming what to read in the repo. There is no per-project file between the index and the repo.
  A global copy of per-repo rules goes stale when the repo moves; a pointer does not.

```markdown
| Project | Repo root | Read |
|---|---|---|
| temper | `~/projects/tasker-systems/temper` | `AGENTS.md` (router to `internal/agents/`) |
```

## The init conversation, in order

1. **Config.** If `temper` is missing, not on PATH, or `temper check` fails, point the operator at
   the CLI wizard (`temper init`) and stop. Nothing below changes a connection.
2. **Read the project's own material**: `AGENTS.md`/`CLAUDE.md`, `docs/`, the README. If the repo
   already carries agent fundamentals, that file is its fundamentals.
3. **If the rules exist only as scattered prose, distill them into a file in the repo**, beside the
   material it came from and in the repo's own conventions. Write from the project's evidence, not
   your assumptions, and show the result to the operator.
4. **Add the index line**: repo root → the repo file to read.
5. **Introduce the arc.** Long-horizon work is a first-class flow: goals carry registers; work
   advances task over task, is recorded session over session, decomposes into sub-goals, and
   closes on purpose. The arc is in `working-a-goal.md`, and an agent picked up mid-goal reads it
   first. The operator decides when a goal opens and closes.

## Traps

- **Duplicating the repo's fundamentals into the skill directory**: the copy rots; point instead.
- **Growing an index line beyond a pointer**: content belongs in the repo it describes.
- **Rebuilding a copy when a target moves**: update the index line to the new location.
