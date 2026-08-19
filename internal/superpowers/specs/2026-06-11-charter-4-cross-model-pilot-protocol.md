# Charter 4 cross-model pilot — protocol

**Date:** 2026-06-11
**Charter:** learning-maths (corpus entry 4)
**Task:** `2026-06-11-charter-4-learning-maths-elaboration-session-corpus-entry-4-cross-model-pilot` (temper context)
**Decided:** charter 2 session next-steps (2026-06-10), deferred at charter 3 session start, taken up 2026-06-11.

## What the pilot tests

Charters 1–3 were elaborated by Fable sessions that also designed the procedure — the
skill text has never had to carry the procedure alone. The pilot runs charter 4's
elaboration in a **fresh non-Fable session** (recommended: Opus 4.8; a Sonnet 4.6 run is
an optional later second data point) with the skill text **frozen**, so that differences
in how the session runs attribute to the operator model rather than to skill drift. The
distinctive yield is not the charter itself but the places where the skill text turns out
to have been leaning on operator priors — each such place is a candidate lesson about the
*procedure as written*.

## The freeze

The skill at `~/.claude/skills/charter-bootstrapping/` is frozen at these fingerprints
from the moment this protocol lands until the pilot session's own step-8 regulation sort:

```
f0ee913bf63fb7277505fb5493e6707650b673e5930bcfda27b2682304ec6c24  SKILL.md
7bc3809271c83d8daebe78013061368719b31483b8d259f92758741566687e26  guidance/elaboration-lessons.md
```

Guidance updates produced by the pilot's *own* regulation sort are output, not
contamination — the freeze forbids pre-pilot tuning, not the procedure's normal
session-end accretion. The review session re-verifies the hashes before reading anything
else.

Ambient parity caveat (acknowledged, not controlled): the pilot session shares the
project's auto-memory and CLAUDE.md with the Fable sessions, and Pete is the same
elicitation partner in both arms. The isolation claim covers the skill text only.

## Setup checklist (before launching)

- [ ] cwd: the temper repo, branch `jct/charter-bootstrapping-procedure` (clean)
- [ ] Docker Postgres up (`cargo make docker-up`) — the corpus sweep needs it
- [ ] ONNX Runtime available (dev box default)
- [ ] Model: Opus 4.8 (`/model` in the fresh session if needed)

## Launch prompt (paste verbatim)

> Use the charter-bootstrapping skill to run an elaboration session for a new
> telos-charter: learning-maths, corpus entry 4. The temper task is
> `2026-06-11-charter-4-learning-maths-elaboration-session-corpus-entry-4-cross-model-pilot`
> (temper context) — start it. Prior material for this charter lives in the
> `learning-maths` vault context. I am present for elicitation throughout.

Nothing else: no map-kind hint, no pointers at specific lessons, no procedural coaching.
If the session asks for things the skill should have told it, answer minimally and note
the gap — that is pilot data.

## Expected outputs (the task's acceptance criteria)

1. `schema-artifact/seeds/learning-maths.yaml` on the branch
2. Corpus sweep green (5 seeds): `every_corpus_seed_loads_and_charter_roundtrips`
3. Step-8 regulation sort run, gated by Pete
4. Temper session note saved, next-steps thread carried

## Post-pilot review (a follow-up Fable session)

1. Re-verify the skill fingerprints above (any mismatch voids attribution).
2. Read the seed against its corpus siblings: thin-who discipline, question-set as
   load-bearing primitive, framing register, charter-only-at-birth, publishable register.
3. Walk the pilot transcript/session note against the skill's eight steps: where did the
   operator diverge, hesitate, or need Pete to fill a gap the skill text should cover?
4. Check the regulation sort: directionality-under-transfer applied? Any portable
   candidate that is actually charter-bound (the binding failure mode)?
5. Pressure check on observed-once lessons if the session touched them
   (`salience-is-always-salience-for-this-telos` awaits its second observation;
   `the-domain's-failure-mode-translates` is foundational-scoped and likely dormant here).
6. Sort the pilot's own findings: gaps in the skill text become candidate skill edits
   (post-freeze, human-gated as ever); operator-model observations land in the session
   note and the braid task's record.
