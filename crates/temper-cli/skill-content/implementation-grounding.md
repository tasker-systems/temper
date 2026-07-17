# Grounding Discipline for Plan-Writing and Implementation

For anyone **writing an implementation plan** from a spec, and anyone **writing code** from a plan.

Inject the relevant principles **verbatim** into plan-writing and implementing subagent prompts (do
not paraphrase — paraphrase is the very loss this guidance exists to prevent). Composes with
`plan-verification.md` (controller-side pre-dispatch verification) and `subagent-guidance.md`
SG-1 / SG-5 / SG-6.

> **This applies to you in the main loop, too.** When *you* write a plan, nobody dispatches you, so
> nothing injects anything — and that is exactly how an ungrounded plan gets authored and then
> stamped "verified" by its own author. Apply this to your own drafting before asking anyone else to.

## The crux: the bug is not invention — it is invention laundered as grounding

A spec exists to go *beyond* current affordances, so a plan/implementation author **must** invent in
some places and conform in others, and the line between "this constraint is movable (the spec
authorizes exceeding it)" and "this constraint is load-bearing (design *with* it, never *around* it)"
is a genuine judgment. That tension is irreducible — do not try to suppress invention.

The failure mode is narrower and more treacherous: presenting **invention as if it were grounded** —
"I checked the schema," "this matches the existing pattern" — when no check happened and whole
sections were invented. This is usually not a lie of intent. We ask for a *confident, complete,
grounded* plan, so *narrating* groundedness becomes part of the genre: the "I grounded this"
sentence is generated text, not a record of a check. The discipline below makes grounding and
invention **structurally un-blurrable**, so invention stays visibly invention (with its spec
authorization) and grounding always carries evidence.

## Principles

### GD-1: Cite or it's invention (evidence is the deliverable, not confidence)
Do not produce "a grounded plan/implementation." Produce the **grounding evidence first** — quoted
`file:line` excerpts, quoted DDL, real command output — and *then* the proposal that cites it. A
claim with no citation above it is, by construction and visibly, invention. Never narrate that you
checked something; **show the excerpt**. Reject any "grounded in X-on-disk" claim lacking a quoted
excerpt or command output.

### GD-2: Prefer executable grounding (run, don't narrate)
Where the target is runnable, "grounded" means *executed*, not *read*. For schema/SQL work: print
the live object (`\d` the table, `\sf` the function), run the predicate, quote the output. Executed
grounding is unfakeable in a way "I read the file" is not. Reserve labeled-invention-with-evidence
(GD-1/GD-3) for what genuinely can't be run yet (new modules, pattern-conformance).

**Executing a claim is not the same as validating it.** Running `SELECT <predicate>` proves it
executes, not that it is the predicate the system uses. Print the incumbent *beside* it.

### GD-3: Tag every step's relationship to disk — CONFORM / EXTEND / AMEND
Make the movable-vs-load-bearing judgment a **required, declared field** per step, not a silent guess:
- **CONFORM** — honor an existing load-bearing constraint. Cite the disk thing (`file:line` / DDL).
  Do not route around it.
- **EXTEND** — build beyond an existing affordance. Cite the **spec section** that authorizes it.
- **AMEND** — deliberately change an existing thing. Cite **both** the disk thing *and* the spec's
  authorization to change it.

A step with none of these tags, or an EXTEND/AMEND with no spec citation, is unreviewable — send it
back. This lets the controller audit the judgment up front instead of discovering at invocation that
a load-bearing constraint was treated as movable.

This tag is what catches a re-implemented predicate: a step that reinvents an existing check is
CONFORM, and CONFORM demands you cite the thing you are conforming to — which you cannot do for a
predicate you just made up, however true it reads.

### GD-4: A plan is an index, not a summary — and **do not author code bodies**
Summary is lossy translation. Carry the spec's load-bearing invariants into each step **quoted, not
paraphrased**, and have each step **cite the spec section the implementer must read** rather than
restating it. The plan is an *index + sequence + grounding-evidence* over the spec — **not** a
self-contained summary that supersedes it.

**Corollary, learned the hard way: do not write invented code bodies into plans.** A plan's *intent*
(design, sequencing, rationale) reliably survives contact; its *specifics* (named functions, file
lists, SQL bodies) are reliably stale on arrival, because authoring them means reasoning from a
mental model rather than from disk. And the sketch is not merely waste — **it wins**: implementers
build the code block, not the correct prose beside it. Where a body is genuinely required, it must
carry a `file:line` citation (GD-1) or an EXTEND/AMEND tag (GD-3).

### GD-5: Escalate, don't fabricate-to-complete
The corollary of GD-1: when a step **cannot** be grounded and is **not** a sanctioned invention (no
spec authorization to EXTEND/AMEND), STOP and report **BLOCKED** with what's missing. Never fill the
gap with confident prose to make the deliverable look complete. (This is `subagent-guidance`'s
escalate-not-soften, pointed at grounding specifically.)

## Division of labor

The **plan-writer is the first line**: a well-grounded plan propagates groundedness to every
implementer downstream; a laundered plan propagates fabrication. So load this discipline most
heavily on the plan-writer — its deliverable is *evidence + sequenced steps + per-step
CONFORM/EXTEND/AMEND tags + quoted invariants*.

The **implementer** treats the plan's citations as the **only pre-grounded facts**. Anything the
plan does not cite is verified on disk (GD-1/GD-2) before use, or flagged as an explicit
assumption — never silently trusted because it "sounds right."

## This guidance evolves

A living artifact — review it against what actually goes wrong and tighten it over time.
