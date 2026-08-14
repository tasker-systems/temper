# Wrapping a Session

`session-lifecycle.md` has the **mechanics** — which calls to make, in what order, and how to amend
a goal's register. This file is about the **content**: what a session note has to hold to be worth
reading, and how to write the handoff that starts the next session.

Read it when a session is ending. It names no command on either surface, deliberately, so the
mechanics have exactly one home and cannot drift from it.

## Why this exists

A session's value to the next session is entirely in what got written down. The failure is not
forgetting to save — it is saving something that reads like a summary and answers none of the
questions the next agent will actually have. That agent is cold. It has no memory of the reasoning,
only the artifacts.

Two things are being produced at the end of a session, and they are **different documents for
different readers**:

| Artifact | Reader | Answers |
|---|---|---|
| The **session note** | whoever reads the arc later, possibly months on | *what happened, what was decided, and what was found* |
| The **handoff preamble** | the next session, cold, starting now | *what not to re-open, what not to re-derive, what to settle first* |

Writing the second as a summary of the first is the common failure. A note describes; a preamble
*instructs*.

## The session note

### It is a findings document, not a diary

The sections in `session-lifecycle.md` are a skeleton, not a target. A note that fills each heading
with a faithful account of what happened is still a bad note if the reader cannot tell **which part
mattered**. Lead with what was learned; let the chronology serve it.

The test: *if a reader takes one thing from this note, is the thing they take the thing I most want
them to have?* If the most important finding is in paragraph nine under "What Happened", it will not
survive.

### Name the finding, then show what caused it

A finding stated as a conclusion is forgettable and unverifiable. A finding stated with the material
that produced it is neither. Prefer *"the gate looked for a literal `FROM <fn>(`, which a
runtime-composed call can never contain"* over *"fixed a gate bug"*.

This is the same rule the grounding discipline applies to plans, pointed at prose: cite or it is
invention, and that applies to your own account of your own session.

### Write down what you got wrong, in the note, not just in the conversation

This is the highest-value habit here and the easiest to skip, because the fix is already made and
the record feels redundant. It is not. A correction that lives only in a conversation is lost when
the conversation ends, and the *class* of error recurs long after the instance is fixed.

Worth recording, specifically:

- **A claim you made and then disproved**, with what disproved it. Including — especially — one you
  made confidently.
- **A rationale you invented for correct code.** Pattern-recognition and pattern-*invention* are
  hard to tell apart from the inside, which is exactly why the instance belongs in writing.
- **A green result that turned out not to mean what it appeared to.** A test that passed by skipping
  its subject is a finding, not a non-event.

### Declare what was NOT done, and never let it be inferred

Coverage is never inferred from absence. A note that describes what ran, and is silent about what
did not, reads as complete to every future reader — and that reading is the note's fault, not
theirs.

State plainly: what was not run and whose job it is, which criteria stayed uncovered, which
verification was skipped and why. A named remainder is information. A gap is a trap.

### Reference every resource as a link

`[the resource's exact title](./<full-uuidv7>)`. Session notes are the most reference-dense
documents anyone writes, so this is where an abbreviated id does the most damage — a UUIDv7's
leading characters are a **timestamp**, so a goal and the task written a minute later share their
prefix. See *Referencing Other Resources* in `SKILL.md`.

## The handoff preamble

The next session starts cold. It has the task body, the goal register, and whatever the previous
note says — and it will happily spend its first hour re-deriving grounding that already exists, or
re-opening a question that was settled, unless told not to.

The preamble is what prevents that. It is short, it is imperative, and it is **not a summary**.

### The four things it carries

1. **What is DECIDED, and must not be re-opened.** Name the decision and say it is closed. Without
   this, a fresh agent re-argues a settled design in good faith, because re-deriving is what a
   careful agent does when nothing tells it the question is answered.

2. **What must not be RE-DERIVED, and where the grounding already lives.** Different from the above
   and just as costly. Point at the artifact — *"the task body carries the working reference for all
   seven predicates and the field-fate table"* — so the next session reads it rather than
   rebuilding it.

3. **What must be RULED before building, and why each matters.** The open questions whose answers
   change what gets built. Give each one its stakes, not just its name: *"it decides the act's
   parameter shape"* tells the next session why it cannot defer.

4. **The one trap, if there is one.** The tempting-and-wrong implementation, or the place this
   change could reintroduce the thing it exists to remove. One is usually enough; a list of five is
   a document, not a preamble.

### What it must not carry

- **A recap of what happened.** The note holds that, and the next session can read it.
- **Reassurance.** *"Good progress was made"* instructs nobody.
- **Everything.** A preamble that covers every angle is one nobody acts on. If it cannot be held in
  mind while starting work, it is too long.

### The test

*Would a cold agent, given only this, avoid the specific wrong turns available to it here?* If the
preamble does not name a wrong turn, it is probably a summary wearing a preamble's costume.

## Closing the loop before you stop

Three things go stale the moment a session ends, and each is cheap now and expensive later:

- **The task.** Its stage, and its body if the work changed shape. A task body describing a plan
  that was superseded is worse than no body — it is a plan a future session will follow.
- **The goal's register.** If the session moved a criterion, say so *and* say what is still
  uncovered. `outcome-registers.md` governs the amendment; the rule that matters here is that a hole
  which has not closed must still be **stated**.
- **Work that was extracted rather than done.** Scope that moved must land somewhere addressable —
  a task, with the reasoning that moved it. Scope that is only mentioned in a session note has been
  dropped, slowly.

Scaling work down is the user's call, never yours. Recommend the extraction, and say plainly that it
is a recommendation.

## The closing message to the user

Governed by *Closing Notes Carry a Status* in `session-lifecycle.md` — every note is a **hard
follow**, **accepted**, **for the record**, or **nothing**, and a note that cannot take one does not
ship.

Two additions specific to wrapping:

- **State the standing of the work in one line.** Is this mergeable, or are there open pieces? That
  is usually the only thing the user needs and the thing a long summary buries.
- **Say what you need from them, or say that you need nothing.** A wrap that ends without either
  leaves the user to infer whether they are blocking — which is a question they should never have to
  ask.
