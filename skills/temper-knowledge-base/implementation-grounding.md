# Grounding, Verification and Review

Read when writing a plan, writing code from one, briefing a subagent, or deciding what review a
change owes before its PR. It applies to you in the main loop as much as to anyone you dispatch.

## Grounding — invention must stay visibly invention

A plan must invent where the spec goes beyond what exists and conform where it does not. The failure
is invention presented as grounded ("this matches the existing pattern") with no check behind it.

- **GD-1 — Cite or it is invention.** Show the evidence (quoted `file:line`, DDL, command output),
  then the claim that rests on it. Never narrate that you checked.
- **GD-2 — Run, don't narrate.** For schema and SQL, print the live object and run the predicate.
  Running a claim proves it executes, not that it is what the system uses: print the incumbent
  beside it.
- **GD-3 — Tag every step CONFORM, EXTEND or AMEND.** CONFORM cites the disk thing it honors; EXTEND
  cites the spec section that authorizes going beyond; AMEND cites both. An untagged step is
  unreviewable — and a reinvented check cannot be tagged CONFORM, which is what catches it.
- **GD-4 — A plan is an index, not a summary, and carries no invented code bodies.** Quote the
  spec's invariants and point at its sections. Implementers build the code block, not the prose beside
  it, so a sketched body wins even when it is wrong.
- **GD-5 — Escalate, don't fabricate to complete.** A step that cannot be grounded and is not a
  sanctioned invention is BLOCKED, with what is missing.

## Verifying a plan before acting on it

A plan is a hypothesis; the code is ground truth. That includes a plan you wrote this session.

- Open the files a task touches and grep every function, type and field the plan names. Nothing
  found means the plan is wrong; design the right approach first.
- **For every predicate the plan authors, ask whether the codebase already has a name for it** — and
  call that. A restated predicate is a true statement about real columns, so grepping and printing
  both pass while the copy drifts from the incumbent.
- Trace one level of indirection; a command often delegates.
- Where the plan's prose and its sketch disagree, the sketch is what ships. Correct it in the brief.

## Briefing a subagent

A subagent knows only its brief. Give it the goal and why, what is already ruled out, the files and
docs to read, and an explicit, narrow scope. For a subagent that writes a plan or code from one,
include this file's grounding rules verbatim — paraphrase is the loss they exist to prevent — and the
project's fundamentals (via `guidance/fundamentals.md`). Treat what comes back as a claim to verify,
not a result.

## Review gates

CI is a safeguard, not a review: a review is a judgment by a reader who did not write the diff.

- **RG-1 — Review is risk-gated.** A code review pass is owed for any change touching auth or authz,
  a migration, a wire contract (`openapi.json`, an SDK, a ts-rs tree) or the ingest/embed pipeline,
  at any size; for `build/medium` work (one proportionate pass); and for `build/large` (an intense
  pass, or several when the work spans surfaces). Not owed for `build/small`, docs-only or test-only.
- **RG-2 — Security review is its own pass,** always for auth changes and whenever access might move
  (a widened gate, a new door, a new caller of a gate). Its lens: disclosure through returned values,
  gate completeness against the canonical visibility predicate, what a refusal discloses, tenant
  isolation, credentials, injection.
- **RG-3 — Reviewers are independent,** each in its own working tree; the author never reviews its own
  diff. A reviewer escalates, never softens.
- **RG-4 — A review returns coverage, not just verdicts:** findings with locations, and what was
  examined and what could not be seen. Whether to act on a finding is the user's call.
- **RG-5 — Review at the boundary** — before the PR opens, not per commit.
- **RG-6 — Ask which review tools to use** when a review is imminent: the user may have installed
  review skills, LSP plugins or scanners for exactly this.
