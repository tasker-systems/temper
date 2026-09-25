# Review Gates

When a change owes a review before its PR opens, and what each review must be. This
discipline applies to dispatched subagents and to you in the main loop alike.

## What a review is

CI green is a **safeguard, not a review**. Gates and scanners are nets: they catch what
they were built to catch and say nothing about the rest. A review is a judgment by a
reader who did not author the diff. A dispatched subagent counts; CI and CodeQL never
do, however green.

### RG-1: Review is risk-gated, not default-on

A code review pass is owed before the PR when any of these hold:

- **Risk classes, any size** — the change touches auth/authz, a migration, a wire
  contract (`openapi.json`, a generated SDK, a ts-rs type tree), or the ingest/embed
  pipeline.
- **build/medium** — one reasonable code review pass: proportionate to the change, not
  an extensive ceremony.
- **build/large** — an intense review, or multiple independent passes when the work
  spans surfaces or contracts. Scale with the work, not the ritual.

Not owed: `build/small`, docs-only, and test-only changes — the gates carry them.

### RG-2: Security review is its own pass

Distinct from the code review — its own reviewer, its own instructions, its own lens.
Owed when:

- auth/authz changes, **always**;
- implied access or permissions might be impacted — a widened gate, a new door, a new
  caller of an existing gate, a capability reaching a new surface.

Its lens: **disclosure** (what a returned value admits about rows the caller cannot
enumerate), **gate completeness** (every conjunct of the canonical visibility
predicate, never a clever subset), **the refusal's voice** (what a denial discloses),
**tenant isolation**, secrets and credential handling, injection. Widening a gate
across an endpoint family is unsafe when one member confers more than it operates on —
check overlap before treating siblings as alike.

### RG-3: Reviewers are independent

The author-agent never reviews its own diff. Dispatch reviewers as subagents, each in
its own working tree — reviewers sharing one tree report each other's experiments as
defects. A reviewer escalates, never softens.

### RG-4: A review returns coverage, not just verdicts

Findings are named defects with locations. A pass that found nothing states what it
examined and what it could not see — silence must never read as clean, and coverage is
never inferred from absence. The discipline detects; it does not decide — cost and
whether to pursue belong to the user.

### RG-5: Reviews meet the change at its boundary

The best review happens when the change's state is commensurate with its intention —
at the arc boundary, before the PR opens, not per commit and not after merge. Use the
client's review skill when one exists (e.g. `superpowers:requesting-code-review`) to
structure the pass; the gates above decide *whether* and *how many*, not which tool.

### RG-6: Look up the user's review surface before dispatching

Before any review dispatch, check whether the user has installed skills or tools that
are expected parts of a code or security review — review-structuring skills, LSP
plugins, security scanners — and ask which to use when relevant. The lookup happens
when the review is imminent, never as an opening flourish; and it is an **ask**, not an
assumption — a tool the user installed for this purpose is their choice to apply.
