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

## 2. Fundamentals — the project's own rules

`guidance/fundamentals.md` in the skill directory holds the project's conventions. Read the
project's own material first — its `AGENTS.md`/`CLAUDE.md`, its `docs/`, its README — and
distill what an agent must know before substantive work here: build and test commands, branch
and commit conventions, the patterns one must not break. Then create the file and show it to
the operator. A fundamentals file is the project speaking to every future session — write it
from the project's evidence, never from your assumptions about it.

If fundamentals already exist, read them before any substantive work in this project, and
offer to update them when you find they have drifted from the project's reality.

## 3. The arc

Say what this project's agents can do here: long-horizon work is a first-class flow. Goals
carry registers; work advances task over task, is recorded session over session, decomposes
into sub-goals, and closes on purpose — the arc lives in `working-a-goal.md`, and an agent
picked up mid-goal reads it first. The operator chooses when a goal is opened and when it
closes; the arc is how the space between is walked.
