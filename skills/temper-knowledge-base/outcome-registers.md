# Outcome Registers

Read this when **authoring or amending a goal or sub-goal**, or deciding whether a criterion belongs
on one. A register states an outcome so its rigor survives decomposition. Use its vocabulary
exactly: it is what keeps goals written by different agents legible to each other.

It exists to make four things clear: the priors, the grounding, success, and what must not be true
at the end. **Scale it to blast radius, not effort** — how many readers inherit it as ground truth,
and how reversible acting on it is. Under-armoring a consequential goal is the worse miss.

## The register is the goal's body, in these sections

1. **Why-anchor** — whose attention this saves or protects, and from what. The drift-detector.
2. **Situated actors** — entity kind, roles, memberships, grants; never "a user" unqualified.
   Enumerate in place (complete and valid) or cite a project source; never block for enumerating.
3. **Priors vs. provided** — what the actor knows (answerable by query) vs. supplies per act.
4. **The act** — what is emitted, the on-behalf-of chain, and whether it reads or mutates.
5. **The three-way Then** — what holds synchronously · what the ledger/audit trace records · the
   stable state once everything downstream has eventually settled. All positive postconditions.
6. **The refusal face** — a well-formed act the system declines: not a failure, and auditable.
7. **Exercise status** — has any of this ever *run*? Distinct from specified and from superseded.
   Two axes, never one reported as the other: *has the trigger fired?* and *has the work executed?*
8. **The clauses** — what must be true, with readable names (`no-witness-precedes-its-mechanism`),
   never indices like `C3`: no downstream reader has the goal open in front of them.
9. **The negative face** — what must **never** become true, whatever act was attempted: a standing
   boundary, not a refusal or a Then: *"read-only members never create resources in that context."*
10. **Closure** — which actor × act × state cells were examined (below).
11. **Stated silence** — what the goal deliberately does not specify, and why. *"We are
    intentionally not specifying X"* is itself a clause: it records shared incompleteness instead of
    leaving a reader to infer coverage from absence.
12. **Current witnessed state** — the table under *Tasks, evidence and current state*.

Put clauses only where drift is catastrophic. *"We are intentionally not specifying X"* is itself a
clause; a section with nothing real to say says so briefly, since padding reads as coverage.

## Clauses, witnesses and declared holes

- **A clause** states what must be true or must never be true, **names no mechanism still open at
  its altitude**, and is authored before the work it governs. Implementers build what it names.
  **Altitude** is how far the goal sits from the work: at a parent goal the mechanism is usually
  open; it closes only when a cited decision chooses it. A clause that dodges a mechanism everyone
  has already chosen says less, not more — that is a loss of rigor passing as compliance.
  *Test it on the goal "a member can take their resources with them":* "as CSV" names a format no
  decision has chosen, so it is open — the clause says "every resource the member owns is in the
  export". Once a cited decision picks CSV, a child goal's clause may say "the export is valid CSV".
- **A witness** is how we know, authored **during the build, as a separately authorized act**. It
  must **fail against the state its clause claims to change**; before the mechanism exists anything
  "fails", so a preamble witness discriminates nothing. Modes: *executable* (shown to fail),
  *replay-verified* (run to quiescence on a test ledger, then assert), *judged* (from a named
  perspective, two or three cited exemplars). Use the test framework the repo actually has.
- **A clause whose mechanism is unbuilt carries a declared hole, not a filed task.**
- **Decision priors.** A mechanism chosen by a cited `decision`, made or ratified by the frame owner
  and standing before the clause, is a prior: be precise about it. One named in passing (a parent
  register, a session note) is assumed, not decided, and **a register cannot license itself** with a
  decision its author drafts in the same pass. A decision licenses precise clauses, not witnesses.
- **Two-readers test**: two competent readers with the same priors agree whether it holds right
  now. **State, not step**: it holds or fails at a moment and can regress; "apply X to staging" is
  a step, "staging matches the committed definition" a state.
- **Two floors.** Goal → sub-goal is clause decomposition by **the meaning test** (split a clause
  when its halves can be violated independently), before the work it governs; for enabling work, at
  uptake. Sub-goal → task is witness decomposition, in the work, by **the demonstrability floor**:
  split until each piece can be shown, on its own, to hold or fail.

## Closure

Each *situated-actor × act × relevant-state* cell is **examined-and-specified**,
**examined-and-excluded** (emitter, timestamp, reason: settled), **examined-and-inexpressible**
(wanted but inexpressible: a pending fork, a peer of exclusion, not a flag on it) or **unexamined**
(a visible remainder). Holes hide in wrong **equivalence claims** ("all actors lacking grant G are
interchangeable here"), so state each attackably, with classes, cells examined, exclusions and
remainder. **Name the axes you close over**; rate axes (cadence, volume) are open unless enumerated.

**When the system cannot express an element**, diagnose in order: (1) *miscategorised*: restate
it, no fork; (2) *missing affordance*: evolve the system, by `enables` work bounded by a clause
stating the affordance; (3) *wrongly reaching*: change the goal by a superseding `decision`. The
owner takes the exit. **Never assume the affordance and write clauses against it.**

## Enabling work is bounded

Goal-attached work is always held by a goal with a clause it can be judged against. **At uptake**,
name it, here or on another goal: *when this is done, what will be true that is not now?* If the
only answer is "the next piece can start", the clause is missing: author it as a child or sibling
goal, concrete about what is decided and thin as its blast radius allows, and file the work against
it. Not a relabelled task; not pre-authored for work nobody has taken up (children splitting clauses
the parent already states may land early, with declared holes); not for chores; not where a clause
already judges the work. Unhoused work is a detection, never a verdict to drop it.

**The discipline detects; it does not decide.** Clauses say what makes the goal achieved *well*,
never whether it is worth pursuing (the owner's call; cost and worth go in a program-level record).
A clause whose job is to retire its own goal is the violation: withdraw it and name its source.

## The loop

**Open** what is in force → **author** clauses and negative face → **decompose** clauses, declaring
holes → **ground** (as a `research` resource) → **take up** → **build and witness** → **contradict**
(a refuted clause gets a superseding `decision`, never a silent edit) → **close** (evidence into the
record, then only the current-state rows and exercise status change; the closing note carries
**hard follow** / **accepted** / **for the record** / **nothing**).

**Before a register lands, ask:** does a clause name a mechanism still open, or one no prior
decision chose (a flag, function, table or field in a clause body)? Does it hold witnesses
at all? Is every EXTEND/AMEND claim cited (`implementation-grounding.md`)? Is every uncovered clause
declared so? Could two readers disagree whether a clause holds? Does a clause name an action?

## Separability — before two or more children land

Children may build on each other but never intertwine for sequence or segmentation. At
decomposition, and when a child is added or its clauses move, the parent states: **one carrier per
clause** (a table; a boundary binding every child is carried by the parent, and a child needing its
own form carries a named instance); **hard and soft dependencies**, hard edges a DAG with no witness
or closure waiting on a later child; **each child's standalone value** (thin is fine; unstated is
not); **postponable children**, which nothing earlier waits on, their needs reaching earlier ones
only as cheap guards in stated silence; **one owner per shared substrate**; **a shipping order**.
Independent read-only reviewers test it, not the author: 2–3, one lens each (*dependencies and
separable delivery*, *what is deferred and must not be foreclosed*, *clause and face coherence*).

## Tasks, evidence and current state

A task declares **`witnesses`** (it **is** the evidence, so it must bite) or **`enables`** (it makes
a clause witnessable; no bite, may precede witnesses, and its `clauses` name the judging clause),
never both. "Make X enumerable (unblocks W2)" is `enables`. Shape: `witnesses: {goal, clauses}`,
plus the witness itself as `witness: {id, mode, clause, floor, bites_against}`, all in `open_meta`;
an enabling task carries `enables: {goal, clauses}`. Give witnesses readable ids, like clauses.

The **evidence record** is one `evidence` resource per goal, linked with `goal`, a section per
clause: witness drafts, proofs, measurements, status history. Create it with the first witness,
never as a placeholder. **The goal shows shape, not history** — a session's account belongs in its
session note and never in the goal: intentions, clauses, exercise status and this table, columns in this order (tooling takes the first *witness* header as that column):

```markdown
| Clause | Witnesses | State | Evidence |
|---|---|---|---|
| `clause-name` | `the_witness_fn` · `.github/scripts/the-gate.sh` | witnessed | [Evidence: <goal title>](./<record-uuid>) — `clause-name` |
```

**Coverage is never inferred from absence.** State is exactly *witnessed*, *not witnessed — why*,
or *withdrawn — the remainder named*; nothing partial. Every standing witness stays named; a retired
one is struck (~~`old_name`~~), remainder in the record. When sources disagree, this table wins.

## Traps

- **Goal membership has two spellings**: the `goal` field makes the one `advances` edge the
  `list_resources` filter reads; `open_meta.witnesses.goal` is a citation, and may name another
  goal. Write both; diff them.
- **`update_resource`'s `content` replaces the body**: send the whole register, never a fragment.
- **There is no section-skeleton tool**: take the headings from this file, not `describe_schema`.
- **Find the evidence record** with `Tool: list_resources  Input: { "doc_type_name": "evidence",
  "goal": "<goal uuid>" }`.
- **`open_meta` replaces each key it names whole**: send the complete `witness` object.
- **Link the evidence record as `[title](./<uuid>)`**: a `#block` fragment is not a citation.
- **Fanning out register work?** Hand subagents these sections verbatim; a paraphrase loses rigor.
