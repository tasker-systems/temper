# Project Setup (/temper init)

Read this when setting a project up for temper-assisted work — the operator invoked `/temper
init`, or a project has no `guidance/fundamentals.md` and you offered to create it. This flow
is **agent-authored in conversation**: the CLI wizard configures the machine; you orient the
project. (CLI-packaging only: the MCP surface has no slash command, no skill directory, and no
wizard, so it carries no arm of this flow.)

## 1. Config first, if needed

If `temper` is not installed, not on PATH, or `temper check` fails, the machine is not
configured yet — point the operator at the CLI wizard (`temper init`) and stop there. Everything
below needs a working connection; none of it changes one.

## 2. Fundamentals — the project's own rules, in the project

Fundamentals live **in the repo they describe** — the project's `AGENTS.md`/`CLAUDE.md`, or a
dedicated file the repo already carries. The skill directory's `guidance/fundamentals.md` is a
**pointer file**, not the fundamentals themselves: one entry per project, naming where that
project's fundamentals live. A global copy of per-repo rules goes stale the day the repo moves;
a pointer cannot.

Setting a project up, then:

1. Read the project's own material — its `AGENTS.md`/`CLAUDE.md`, its `docs/`, its README.
   If the repo already carries agent fundamentals, that file **is** its fundamentals; do not
   duplicate it.
2. If the repo carries its rules only in scattered prose, distill them into a fundamentals file
   **in the repo**, beside the material it distilled (the repo decides where — match its own
   conventions). Write from the project's evidence, never from your assumptions about it, and
   show the result to the operator.
3. Add or refresh the pointer entry in the skill directory's `guidance/fundamentals.md`:
   project path → fundamentals location. Keep the file a bare index — if an entry grows
   beyond one line, the content belongs in the repo it describes.

Reading fundamentals before substantive work in a project goes through the pointer: no entry
for the current project is exactly the `/temper init` offer. An entry whose target has moved
or rotted is updated — the pointer is refreshed, never the copy rebuilt.

## 3. The arc

Say what this project's agents can do here: long-horizon work is a first-class flow. Goals
carry registers; work advances task over task, is recorded session over session, decomposes
into sub-goals, and closes on purpose — the arc lives in `working-a-goal.md`, and an agent
picked up mid-goal reads it first. The operator chooses when a goal is opened and when it
closes; the arc is how the space between is walked.
