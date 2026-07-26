# The Outcome-Register Discipline — Design

**Status**: proposed
**Goal**: `019f9a34-3306-70d1-b07a-f23c99943751` — Adopt the Outcome-Register Convention for Temper Development
**Source research**: `019f9a32-e1b2-7f43-b4cf-ac9b58447cb9` (outcome registers) · `019f9a33-90e5-7882-bf63-61898a33e78d` (the refusal surface)
**Audit backing this spec**: `019f9ee8-1675-72d0-99bb-3dea38aed84b` — five days read against the convention (2026-07-22 → 07-26)

---

## 1. Why this exists

Rigor in this repository arrives **grass-roots and late**. It is discovered at a leaf — one adversarial
pass, one careful reading, one measurement that refutes a confident claim — and then folded back by
hand into a doc, a guidance file, or a memory. Things still get missed, and the same shapes recur.

The audit measured this over five days. Twelve distinct instances of one drift shape — a declaration
and its consumers with nothing tying them, or a check that could not fail — each written up as *the*
lesson of its own session note. Separately, eight ambient-documentation artifacts asserted behaviour
that was not true, every one caught by a human or agent reading carefully and **none by a gate**.

The corrective is not more documents. It is a **structural discipline**: a stated convention for what
an outcome *is*, decomposed so its rigor survives, with the holes computable rather than discovered by
incident.

The heart of that convention is the **outcome register**. Witnessing, closure, and verification modes
are the mechanisms that preserve a register's rigor through decomposition — they preserve nothing if
the criterion is unspecific. The research document already states the negative test:

> a criterion that could be satisfied by a system nobody needed is malformed.

**The discipline is the backbone and stands alone.** It delivers its value with no ontology, no init
flow, and no substrate change. Everything in §4 onward is an enhancement to it, never a prerequisite.

---

## 2. Scope — the cut, and what is deferred

The full picture is five parts:

1. **The discipline**, as an installable skill.
2. **The project ontology** — personas, affordances, priors as recognized `open_meta` conventions.
3. **`/temper init`** — the flow that births an ontology at whatever resolution a project offers.
4. **The citation relation and its two queries** — closure-staleness and expressibility.
5. **The ontology as a cognitive map with its own charter-telos.**

**This spec covers 1, 2 and 4. Part 3 is the second spec. Part 5 is deferred.**

2 and 4 are inseparable: a convention whose holes are not queryable is documentation with a schema
attached. 1 ships without 3 because an ontology can be hand-authored.

**Why init is deliberately second.** Building the research flow first means automating a flow nobody
has ever run by hand — element 7's failure in its purest form, committed by the work that introduces
element 7. Hand-authoring Temper's own ontology (§9) *is* the requirements-gathering for what init
should automate.

---

## 3. The discipline — complete contents

This section is the backbone. It is enumerated exhaustively because a partial restatement of the
research document would be the serialization loss the document itself diagnoses.

### 3.1 The register — eight elements

Seven from the research document, plus a negative face this spec adds (element 8).

A goal carries an outcome register:

1. **Situated actors** — never "a user." A perspective-position: entity kind, roles held, team
   memberships, grants. The convention forbids the word "user" unqualified.
   *Changed by this spec*: enumerated **in place**, or **cited** from the ontology (§4). Citation is
   the enhancement; in-place enumeration remains a complete, valid register.
2. **Priors vs. provided context** — what the actor already knows (latent, answerable by a projection
   query) versus what it must supply per action (a payload requirement). Architecturally different
   things, distinguished up front.
3. **The act** — emission topic, on-behalf-of chain, read-vs-mutation character.
4. **The three-way Then**:
   - *Synchronous postconditions* — what projections now show
   - *Ledger trace* — what the ledger records, including the on-behalf-of chain
   - *Eventual stable-state* — what cascades through event-driven consumers and sweeps, asserted
     after run-to-quiescence
5. **The refusal face** — refusal as first-class, distinct from success and failure. Failure: the act
   was attempted and did not complete. Refusal: the system declined, and the declining is itself an
   auditable act. Grounds, disclosure ladder, recourse, inertness — full treatment in research
   `019f9a33-90e5-7882-bf63-61898a33e78d`.
6. **The why-anchor** — one sentence tying the outcome to an attention claim: whose attention does
   this save or protect, and from what. The drift-detector: when a why-anchor no longer describes
   anything anyone cares about, the criterion is due for supersession, visibly.
7. **Exercise status** — has any of this ever *run*? Orthogonal to stated silence ("we did not specify
   X") and to supersession ("X was decided and is now replaced"). The third state is **unexercised**:
   specified, merged, present in the corpus, never executed.
   *Extended by this spec*: exercise status also applies to ontology entries (§4.2).
8. **The negative face — what must never become true.** *Added by this spec.* A standing regression
   boundary, not a consequence of any particular act.

   It is **distinct from the refusal face**, which covers acts the system declines. A negative-face
   clause is a state that must not obtain regardless of which act was attempted or whether anything
   was refused: *"a read-only context member must never create a resource homed in that context"*,
   *"standing must never be recomputed from its own prior value"*, *"a folded source block must never
   become invisible to the staleness check."*

   It is also distinct from the three-way Then, which is entirely positive postconditions.

   **Why it earns a place**: across the audited window, nearly every expensive finding was the
   violation of a standing negative that no element of the register had a slot for. The create-into-
   context gate (`READ` where `WRITE` was required), the auditor dispatch callable by any principal,
   the proposed `NOT sb.is_folded` filter that would have made content removal invisible — each is a
   boundary crossed, and none is expressible as a postcondition or a refusal.

> **Element 7 needs two axes, not one value.** Established the hard way: the citation auditor's
> schedule had fired ~18 times and died every tick before doing any work. *"Has the schedule fired?"*
> → eighteen → "exercised." *"Does any `citation_audited` event exist?"* → zero → "never ran." Both
> readings alone are wrong. Trigger-fires and work-executes are independent.

### 3.2 The witnessing invariant

- Every child criterion names which parent clause it **witnesses**. A parent clause is satisfied only
  when its witnesses jointly cover it.
- **Coverage gaps are computable absences** — a parent Then-clause with no witnessing child is a
  visible hole, not a week-three surprise.
- **Drift is locally scoped.** When implementation reality forces divergence the question is precise:
  *does this break my witness-duty to clause N, or is clause N itself now wrong?* The first is a
  task-level fix; the second is a supersession event, visible to sibling tasks.

#### 3.2.1 Clauses are invariants; witnesses are statements about mechanism

**This is the load-bearing correction, and it was paid for.** A register authored over a subject that
did not exist produced ten witnesses, most of them unwritable, filed against an explicit instruction
not to file unvetted tasks — and cost four working sessions to recover the intent the decomposition
had buried. Full accounting in §9.1.

The distinction that prevents it:

- **A clause states what must be true, or what must never be true. It names no mechanism.** It has to
  survive any implementation, because *the how is the part most likely to mutate*. The clause that
  survived every revision of the auditor work — *"an unrefreshed verdict is a verdict about something
  that has not changed"* — names nothing that could be built two ways.
- **A witness states how we know. It is a claim about mechanism, so it can only be authored once the
  mechanism is known — which is during or after the doing, never before.**

Every one of the ten hallucinated witnesses was a *how* wearing a clause's clothes: *"every member of
the material-event set has a production write path"* presumes a material-event set exists as an
object; *"the partition is exhaustive over `domain`"* presumes a partition. Neither had been built and
neither could be known yet.

**The bite requirement proves the timing rather than merely suggesting it.** A witness must be shown
to fail against the state the clause claims to change. When the mechanism does not exist, "fails
against current state" is satisfied by *the absence of the feature* — vacuously, by anything. A
witness that fails only because nothing exists discriminates nothing.

**Consequences:**

- **Decomposition into witnesses is not a preamble step.** It happens inside the doing (§6, loop
  step 5), and it is a **separately authorized act** — the register's shape invites decomposition and
  an author will decompose, so the constraint has to be structural rather than an instruction.
- **A clause whose mechanism is unbuilt carries a declared hole, not a filed task.** "One clause with
  a named remainder" is the honest state; "nine witnesses" was not.
- **The termination worry dissolves rather than being gated.** Witnesses cannot proliferate up front
  because they are not authored up front.

#### 3.2.2 The decomposition floor — a meaning test, not a demonstrability test

The research document's floor — *stop when a criterion's witnesses are directly demonstrable* — cannot
govern clause decomposition once witnesses come later. The clause-level rule is:

> **Split a clause when its halves can be violated independently. If two sub-clauses can only ever
> fail together, they are one clause.**

This is deliberately a **question judgment can answer, not a boundary that pretends to**. The
granularity of *how* is the thing hardest to communicate up and down a decomposition ladder, and a
hard rule there produces false precision. The demonstrability floor survives, relocated: it governs
**witness** decomposition, during the doing, where mechanism is known.

**The boundary this whole section defends**: too much decomposition or verification up front is the
task performed in preamble. The only way to know what will work, how it will work, and how to prove
it, is ultimately to do it. The methodology's job is to state what must be true and what must never
be true — direction, and the means to validate success and failure — and to leave the *how* alone.

### 3.3 The closure discipline

For a goal, the space of *situated-actor × act × relevant-state* is a constructible product. Every
cell is in one of three states:

- **Examined-and-specified**
- **Examined-and-deliberately-excluded** — with emitter, timestamp, and stated reason. *Settled.*
- **Examined-and-inexpressible** — the cell is wanted; the system cannot express it. **Carries a
  pending fork** (§3.4.2): evolve the system, or change the goal. Not a hole — it was examined — and
  not an exclusion — it was not chosen. *Added by this spec.*
- **Unexamined** — a *visible* remainder, not a silent one

**Why the third is a peer of the second and not a flag on it**: an exclusion is settled, an
inexpressible cell is waiting. A reader scanning "excluded, reason given" moves on, which is exactly
how the fork silently fails to be taken.

The deterministic property is that there is **no ambiguity between the last two**. An unmarked gap is
the most dangerous kind of undocumented negative decision, because it re-litigates itself as an
incident.

**Equivalence claims are first-class and falsifiable.** Nobody examines cells one by one; the matrix
collapses via claims like *"all actors lacking grant G are interchangeable for this act."* Most holes
live not in unexamined cells but in **wrong equivalence claims** — cells examined under a class
abstraction that did not hold. The closure section records: dimensions considered, classes claimed,
cells examined per class, cells excluded with reasons, and the remainder explicitly marked unexamined.
A reviewer can attack a stated class; nobody can attack an unuttered assumption.

> **⟲ Scar, carried verbatim in intent from the research document.** The claim that Temper's ontology
> is "closed-world in the dimensions that matter" is *itself* a wrong equivalence claim, and the type
> specimen the section describes. Every dimension it enumerated is **authorization-shaped**. The
> closed-world property genuinely holds there. It does **not** hold for cadence, rate, or volume — and
> those are not decorative here, since evidential standing is saturated with them.
>
> **Closure declarations must state which axes they close over, and rate-shaped axes must be named as
> open unless explicitly enumerated.**
>
> **Consequence for §4**: personas and affordances are authorization-shaped. Hoisting them to project
> scope buys the closed-world property *on those axes only*. A closure declaration citing the persona
> list still has to name its rate-shaped axes as open. **The ontology must not be read as closing
> this scar.**

**Bounds.** Closure declarations only at load-bearing intersections. "Unexamined" remains a
legitimate, sayable state. The goal is a **named** remainder, not a zero remainder — total enclosure
is fantasy, and exhaustion-driven false claims are worse than honest gaps.

### 3.4 Verification modes

Each terminal criterion carries a mode marker.

- **Executable.** A witness must be shown to **FAIL against the state the clause claims to change**.
  A test that passes identically before and after discriminates nothing, however well it is named.
  Witness-coverage without a bite requirement degenerates into a coverage metric — precisely the
  instrument this convention exists to escape.
  **The bite requirement is also what fixes *when* a witness may be authored** (§3.2.1): when the
  mechanism does not exist, "fails against current state" is satisfied by the absence of the feature,
  vacuously, by anything. A bite against nothing is not a bite.
- **Replay-verified.** Eventual stable-state cases: run the cascade to quiescence deterministically
  against the test ledger, then assert. Event-sourcing makes this deterministic rather than
  timing-dependent.
- **Judged.** Subjective criteria are not perspective-free predicates and cannot be made so.
  The honest form: satisfied *as judged from a named perspective*, exemplified by two or three cited
  exemplar regions. *"Does this look like `relationships.rs` looks"* is tractable; *"does this align
  with our patterns"* is a vibes query that fails silently. Reviewed at the consolidated end-of-plan
  review, which finally gets a rubric instead of a re-derivation. When an exemplar is superseded, the
  criterion visibly needs revisiting.

**The Gherkin file layer is deliberately skipped.** The value was the decomposition discipline, which
the register carries at the goal/task level. If stakeholder-legible scenario text is ever wanted,
generate it *from* the criteria.

#### 3.4.1 The fixture vocabulary — corrected

The research document names the executable mode as *"plain rstest + sqlx::test, with a small fixture
vocabulary (`given_entity`, `given_grant`, `emit`, `project_as_of`, `run_to_quiescence`)."*

**`rstest` is not a dependency of this workspace.** Measured: `rg rstest Cargo.toml crates/*/Cargo.toml
tests/e2e/Cargo.toml` returns nothing, and the workspace contains **zero** occurrences of `#[rstest]`,
`#[case]`, `#[fixture]` and `#[values]`. What exists is 1,186 `#[sqlx::test]`, 1,456 `#[test]`, and 50
`#[tokio::test]` — no parametrized-test framework at all.

The reference entered the goal from an ideation session where `leynos/rstest-bdd` was linked
explicitly as a **think-with reference, stated as not in the repo**. It came out the other side of a
goal-authoring pass as a specified affordance.

**This is a distinct drift species and the discipline should name it.** Element 7 catches
*shipped-but-never-run*. This is one step earlier: **referenced-to-think-with, mistaken for adopted.**
Both are "present in the corpus, never chosen," and neither is caught by checking whether the claim is
*true* — `rstest-bdd` is a real project and the link resolves.

**Resolution.** The fixture *vocabulary* is right and survives. The framework claim does not. This
spec makes no framework decision: the vocabulary must be expressed in the idiom the workspace already
carries, and introducing a parametrized-test framework is a separate, costed decision.
**Goal `019f9a34-3306-70d1-b07a-f23c99943751` needs its executable-spine clause amended accordingly.**

### 3.4.2 The inexpressible intersection — where a goal exceeds the system's affordances

A goal states an outcome. Sometimes the system **as designed cannot express it** — the persona does
not exist, the act has no affordance, the projection the Then-clause asserts over is not built, the
refusal named is one the system cannot make.

**That intersection is a first-class, detectable state, and it has exactly two honest exits:**

- **Evolve the system.** The affordance is missing and should exist. Produces an **`enables`** task
  (§5.1.3) — a precondition, legitimately filed before any witness.
- **Change the goal.** The affordance should not exist, or the clause was reaching wrongly. Produces
  a **supersession**, via a decision.

**The dishonest third exit is to assume the affordance and write clauses against it.** That is what
produces mechanism-shaped clauses and premature witnesses, and it is the deeper account of §9.1 than
"witnesses were authored too early": a register met the edge of what the system could express and,
instead of forking, **hallucinated the affordance**. Ten witnesses were written against a
material-event set, a partition, and a tier 2 — none of which existed. Neither honest exit was taken
because neither was named.

**Both exits already have worked instances in this corpus, and neither was recognised as a fork at
the time:**

| Exit | Instance |
|---|---|
| Evolve the system | The witness citation wants to be a facet on the `advances` edge; edges have never carried properties and no surface can write one. Precondition filed (§5.1.2) |
| Change the goal | The auditor register's R10 — *"the refusal the system cannot make"* — recorded in its own table as **inexpressible**, *"does not exist"*. **But see §3.4.3: R10 is a poor example of this exit, and why is the more useful finding** |

#### 3.4.3 Check for miscategorisation before taking either exit

R10 was used above as the worked instance of *change the goal*. PR #550's disposition shows that is
not what happened, and the real story names a **third diagnosis that must be ruled out first**.

R10 recorded *"I cannot assess this citation"* as a refusal the system could not express. It splits:

- **Evidential inability** — *"the citations do not warrant this"* — was **never a refusal**. It is a
  **verdict**, a signed value `<= 0`. The standing model already separates *evaluated-but-weak*
  (carried by quality) from *unevaluated* (carried by the band's coverage-ratio gate); R10 proposed a
  third state between them, and the three-axis design says there is none.

  **And it is not a neutral abstention — it is among the most damning verdicts available.** The
  auditor's whole role is to say whether an assertion holds up under scrutiny of its evidence, and
  with what confidence. *"I cannot assess this"* is therefore not *"plausible but not fully warranted
  by the citations and reasoning"*; it is *"this is incommunicable, or I cannot even evaluate where
  you derived it."* That is worse, and the value should reflect it.
- **Structural inability** — the gate `NotFound`s a self-authored citation, so the auditor is
  *forbidden* to verdict — is real, and survived as D7's conjunct.

**So the cell was not inexpressible. It was miscategorised**: a verdict filed in the refusal face, and
a real structural refusal tangled together with it.

**The diagnosis to run before forking**: *is this element the kind of thing I have claimed it is?*
Forking on a miscategorised cell evolves or changes the wrong thing — here it would have argued for
building a refusal the system correctly does not have, when the actual repair was to move the concern
to the verdict space and let the standing model do what it already does.

Three diagnoses, in order:

1. **Miscategorised** — the element is not the kind of thing the register claimed. Repair: restate it
   in the right element. *No fork.*
2. **Missing affordance** — the element is right and the system cannot do it. → **evolve**.
3. **Wrongly reaching** — the element is right and the system should not do it. → **change the goal**.

**This is not an ontology feature.** An earlier draft framed the detection narrowly, as situated
actors failing to resolve against a persona list. That is one instance. The general form needs no
ontology: *any* register element can hit the edge, and R10 hit it in the refusal face.

#### Where this lands in the closure section

The cell state this produces — **examined-and-inexpressible** — is defined once, with the other three,
in §3.3. It is deliberately not restated here.

*An earlier draft of this spec restated all four states in both places. Two copies of one enumeration
is the same drift this document diagnoses, committed inside it; caught by printing the section rather
than by re-reading it.*

A register with inexpressible cells and no fork recorded against them is incomplete in a way a
coverage count will not show.

### 3.5 Two disciplines throughout, and they cut against the rest

- **Load-bearing intersections only.** Clauses concentrated where drift is catastrophic. *"Forty
  scenarios is spec-fall in a new costume."*
- **Stated silence as first-class.** *"We are intentionally not specifying X"* is itself a clause of
  the shared-incompleteness record — the difference between an agent exercising delegated judgment and
  an agent guessing whether it has any.

**The skill must teach these with equal weight to everything above.** A skill that only teaches how to
write a register produces exactly the spec-fall it exists to escape. Where **not** to put clauses, why
"unexamined" is a legitimate answer, and why a named remainder beats a false zero are load-bearing
content, not caveats.

---

## 4. The project ontology

### 4.1 What it is, and why it is not configuration

The closure discipline is only *computable* where the dimensions it quantifies over are enumerable.
Temper's own entity model makes them enumerable for Temper. Another project has no such model inside
temper, so its closure sections are prose and its "unexamined remainder" is unaskable.

The ontology gives a project that enumeration. It is not configuration: three of its four categories
are **the register's own elements hoisted from goal scope to project scope**, and the fourth is the
matrix they generate.

| Category | Register element | Scope |
|---|---|---|
| personas | 1 — situated actors | project |
| affordances | 3 — the act | project |
| priors and fundamentals | 2 — priors vs. provided, plus constraints | project |
| scenarios | the closure product — actor × act × state | derived |

### 4.2 Carrier and shape

**Doc type `domain` carries every ontology entry. A recognized `open_meta` key names its kind.**

*Why the doc type carries the kind*: `temper resource list` has no `open_meta` filter — its filters are
`--type`, `--context`, `--cogmap`, `--title-contains`, `--stage`, `--goal`, `--status`. The closure
query needs a **complete** enumeration of the domain; filtering client-side over a paged list is the
truncation trap the skill already warns about. `list --type domain --all` is a complete server-side
enumeration.

*Why `domain`*: semantically exact — the closure section quantifies over a domain — and effectively
free. **Zero rows in `@me/temper`**, eight repo-wide. No collision with the 66 existing `concept` rows.

*Why a recognized `open_meta` key rather than a new doc type*: `open_meta.schema.json` already exists
for exactly this purpose, and states it:

> *"The open tier is generally free-form… This schema documents the keys temper **recognizes** and
> constrains their shape. A recognized key with a wrong shape is a bug."*

It is already versioned by migration (v1: `keywords` + `descriptor`; v2: `tags`). A new doc type would
mean an enum change in `base.schema.json`, a per-type schema, a seed, and full MCP + API + CLI parity —
for browsing affordances an ordinary resource already has.

Three recognized keys:

| Key | Carries |
|---|---|
| `persona` | entity kind (required), roles, memberships, grants |
| `affordance` | the act's topic, on-behalf-of shape, read-vs-mutation character |
| `prior` | what an actor already knows vs. must supply; project constraints |

**Exactly one of the three keys per entry.** An entry carrying two is a modelling error, not a
multi-kind entry: a persona that is also an affordance means the act and the actor have been conflated,
which is precisely the distinction elements 1 and 3 exist to hold apart. The schema should make this
unrepresentable rather than merely discouraged.

Exact field lists are an implementation concern for the plan, grounded against
`crates/temper-workflow/schemas/open_meta.schema.json` at implementation time. Deliberately not
authored here: a spec-invented field list is stale on arrival and, worse, wins over the prose beside it.

**Ontology entries are ordinary resources**, so edges, supersession, team authorship, and promotion
into a shared cognitive map are all inherited. Nothing new is built for any of them.

### 4.3 Two orthogonal axes

- **Provenance** — the existing managed enum `temper-provenance` (`user-created` | `llm-discovered`,
  `base.schema.json`). *"Here are our personas, take our word"* → `user-created`. *"Go research it"* →
  `llm-discovered`. Import-and-map is `user-created` for the entry and `llm-discovered` for the mapping
  edges.
  **Grounded caveat**: sampled across `@me/temper`, `temper-provenance` is absent on roughly 30% of
  rows (58 `llm-discovered`, 87 `user-created`, 61 absent). It must be **required** for ontology
  entries, not conventional.
- **Exercised** — whether a real goal has cited it. No new field; it is the citation relation read
  backwards.
  **"Cited" means named by a register** — as a situated actor, an act, or a prior — **not merely
  referenced.** An ontology entry linked from a session note, a research doc, or another ontology
  entry is *mentioned*, not exercised. The distinction is the whole point of the axis: exercise
  status asks whether this entry has done work in an outcome, and a mention is not work.

An entry that is `llm-discovered` and unexercised is a **proposal**. `user-created` and unexercised is
**a claim, not yet load-bearing**. Exercised is **doing work**. All three are legitimate.

This pair is what lets a research pass be generous: over-proposing costs nothing when every proposal
is visibly unexercised until cited.

### 4.4 The discipline must be useful at low resolution

A project that can say only *"we have customers and internal admins"* has a legitimate two-entry
ontology. Coarse domain; the discipline still runs; expressibility pressure sharpens it. This is a
**design constraint, not a tolerance**: if adoption requires a rich ontology first, the ontology
becomes a large upfront cost and the convention fails its own why-anchor.

---

## 5. The citation relation and its two queries

Both directions are set differences over two lists. Nothing is built in substrate.

**Closure staleness.** A goal's closure declaration records the persona refs it closed over. A persona
exists whose ref is not in that list ⇒ the declaration is visibly incomplete. Set difference against
`list --type domain --all`. No timestamps, no watermark.

**Expressibility.** At **authoring** time, not in a sweep: **where a register elects to cite the
ontology**, its situated actors must resolve to persona refs. One that does not resolve is the signal
— either the clause is vague, or the project's self-model has a gap and the persona is born
(`user-created`, unexercised until cited).

Synchronous-at-authoring is the better property: it puts the discovery at the moment someone is
already thinking about the outcome, which is the only moment the answer is cheap.

> **This check is conditional, and that is load-bearing (D1/D3).** A register that enumerates its
> situated actors **in place** is complete and must not be blocked — a project with no ontology, or a
> goal whose actors genuinely have no project-scope counterpart, is authoring correctly. Expressibility
> fires only on the citing form. An implementation that makes persona resolution unconditional breaks
> the discipline's standalone property and is wrong, however well-intentioned.
>
> The same conditionality governs closure staleness: a declaration that closed over an in-place
> enumeration has no persona refs to diff, and is neither stale nor fresh — it is **outside the query's
> domain**, and must be reported as such rather than silently counted as clean.

> **This closes an open question.** The research document lists *"closure-declaration staleness
> projection — the ontology-drift falsification is described as computable; the actual projection query
> shape is unspecified and depends on how closure declarations are encoded (structured section?
> open_meta? eventually facets?)."* Answered: recognized `open_meta` conventions on `domain`-typed
> resources, and the query is a set difference.

### 5.1 A defect the first run of this query found, in the convention's own plumbing

**Two spellings of "this task belongs to that goal" exist, and nothing ties them.**

- the `advances` **edge**, asserted by `resource create/update --goal`, and the only thing
  `resource list --goal <ref>` filters on;
- `open_meta.witnesses.goal`, the **citation** the witness convention uses.

They can disagree, silently. Found empirically on 2026-07-26, on the first run of the uncovered-clause
query: task `019f975e-7be9-7ff3-a5bd-ef7ea72ff4a5` cites goal
`019f9a34-3306-70d1-b07a-f23c99943751` through `open_meta` and carries **no** `advances` edge. It
therefore does not appear in `list --goal`, and a clause migration built from that list missed it and
left a dangling citation.

**The migration was hand-built by the author of this spec, an hour after specifying that the query
should exist. The query caught it.** That is the non-vacuity evidence §9 asks for, taken earlier and
more cheaply than expected, and it is worth more than a green run would have been.

This is the audit's own type specimen — same shape, two spellings, nothing linking them — sitting
inside the mechanism meant to catch it.

#### 5.1.1 Resolution: the citation becomes a facet on the `advances` edge

**Decided 2026-07-26 (Pete).** The clause citation is a `kb_properties` row owned by the `advances`
edge (`owner_table = 'kb_edges'`). The link is the edge; the clause **qualifies the link**.

Grounding for the decision:

| Fact | Evidence |
|---|---|
| `advances` is a **label** on a `leads_to` edge, not an `edge_kind` | `db_backend.rs:94`; `substrate_read.rs:201` |
| The goal *link* being an edge and **not** a property is already settled | `keys.rs:20` — `KeyFate::Edge`, for `temper-goal` |
| `kb_properties` admits `owner_table='kb_edges'`, by design | `canonical_schema.sql:656` — *"§4a edges carry facets"* |
| The event and payload schema admit `kb_edges` as owner | `property_asserted` · `AnchorTable` enum |
| Uniqueness is `(owner, key, value) WHERE NOT is_folded` | so N clauses = N rows, no encoding needed |

**Why this over the alternatives.** It makes the divergence **unrepresentable** rather than detected:
a citation's owner *must* be an edge id, so a citation without an edge cannot exist. That is rung 4
against a checked-pair's rung 2. Putting the clause in the edge `label` would also work — `label` is
part of `uq_kb_edges_assertion` — but it puts structure in a string, against this repo's own rule, and
`list --goal` binds `label = 'advances'` exactly. Making the edge a projection of the `open_meta`
citation was rejected outright: it reverses `KeyFate::Edge` with no evidence against that decision.

#### 5.1.2 The precondition, and why "never run" is not "wrong"

**Production has zero edge-owned properties.** `kb_resources` 10,590 · `kb_content_blocks` 37 ·
`kb_edges` **0** · `kb_cogmaps` **0**. No surface can write one: `FacetSetInput` takes a `resource`.

**That is a second instance of a confident DDL comment describing something nobody has executed** —
after `init.rs`'s *"No export layer, deliberately."* One instance was an anecdote; two is the species
the discipline names, and it is worth recording that the pattern reaches production DDL and not only
Rust doc comments.

**But unexercised is not wrong.** Element 7 exists precisely to convert a never-run artifact from an
apparent constraint into **open design space**. Edges were always intended to carry facets; the write
path simply never got built. So this goal takes on a **precondition**: expand the read and write
surfaces so an edge can own a property — across MCP, API and CLI, per the standing parity intention.

The citation moves onto that path once it exists. Until then `open_meta` holds, and the standing rule
is unchanged: **any query over goal membership must read the citation, not the edge**, because a query
built on `--goal` silently under-reports.

#### 5.1.3 A task either *witnesses* a clause or *enables* one

Surfaced by 5.1.2's precondition, which is neither evidence nor a clause: it is build work that makes
a clause witnessable.

The model as inherited says tasks carry witness declarations, full stop. That forces enabling work to
**pretend to be evidence**, and there is evidence it already did: the deleted child task *"Make the
material-event set a first-class enumerable object (**unblocks** W2 and W8)"* declared itself a
witness while its own title says it *unblocks* two others. It witnessed nothing. It was enabling work
miscast as evidence, and that miscasting is part of how a decomposition pass produced ten unbuildable
witnesses.

So a task declares one of two relations to a clause:

- **`witnesses`** — this task is the evidence. Subject to the bite requirement and to
  `no-witness-precedes-its-mechanism`.
- **`enables`** — this task builds the mechanism that makes a clause witnessable. **Not** evidence,
  **not** subject to the bite requirement, and legitimately filed before the witness exists — indeed
  it is what `no-witness-precedes-its-mechanism` implies must exist first.

Without the distinction, the clause forbidding premature witnesses would also forbid the work that
makes witnesses possible, which is incoherent.

---

## 6. The workflow vocabulary and the loop

| Member | Role |
|---|---|
| **Goal** | Carries the register |
| **Sub-goal** | A **non-leaf witness** — witnesses named parent clauses; its own witnesses are clauses, not tests. Cites the parent's dimensions and narrows them; never restates |
| **Task** | A **leaf witness** — carries `witnesses` (goal ref → clauses) and `witness` (id, mode, floor, bites-against) |
| **Research** | Where a grounding pass lands |
| **Decision** | The supersession vehicle |
| **Session** | Opens with the criteria-in-force projection; closes by updating exercise status and carrying a status on every closing note |

**Goal, sub-goal and task are one kind of node at three heights, but two different floors govern the
descent** (§3.2.1, §3.2.2):

- **Goal → sub-goal** is clause decomposition, before the doing. Governed by the **meaning test**:
  split while the halves can be violated independently. A sub-goal names no mechanism.
- **Sub-goal → task** is witness decomposition, inside the doing, once mechanism is known. Governed by
  the **demonstrability floor**, which is where that floor belongs and the only place it can be
  honestly applied.

So *"which am I writing?"* has an answer at both heights, and neither answer is "how big is this."
A task filed before its mechanism exists is the failure §9.1 records.

**Research is placed by evidence.** The research document leaves it unplaced. Across the audited
sessions the expensive findings all came from grounding passes, and they had nowhere to live but a
session note that then had to be re-read to recover them.

**The two `open_meta` keys already in production use** — measured across the 50 most recent tasks in
`@me/temper`: 9 carry both, 4 carry only the pointer, 0 carry only the descriptor — are kept as-is:
`witnesses` = `{goal, clauses, child_of}` (the pointer up) and `witness` =
`{id, mode, clause, floor, bites_against, route}` (the witness describing itself). They are two
concepts, not a drifted spelling.

### The loop

1. **Open** — the criteria-in-force projection: which clauses hold for this region, which are
   superseded, where the scars are.
2. **Author or resume** — write or reload the register: the invariants (what must be true) and the
   negative face (what must never become true). No mechanism named. The expressibility check fires
   here.
3. **Decompose the clauses** — by the meaning test (§3.2.2), not to a demonstrability floor. Stop
   when halves can no longer be violated independently. A clause whose mechanism is unbuilt carries a
   **declared hole**, not a filed task.
4. **Ground** — a grounding pass lands as research.
5. **Build — and author the witnesses here.** Mechanism becomes known by building, so this is the
   first point at which a witness can be written honestly, and it is a **separately authorized act**.
   Witnesses must bite against a state that exists.
6. **Contradict** — measurement refutes a clause ⇒ decision, supersession, visible scar. Never a
   silent edit.
7. **Close** — exercise status updates to what actually ran; every closing note carries
   **hard follow** / **accepted** / **for the record** / **nothing**.

> **Steps 3 and 5 were one step in the draft of this spec, and that was the defect.** Collapsing them
> is what lets a preamble pass emit mechanism-shaped tasks for a mechanism nobody has built.

---

## 7. The reach bound — reach by relocation

The research document states the choice as: either the convention's reach extends to the ambient
corpus, or it *"should say plainly that it does not, so nobody reads a clean register as evidence that
the environment is clean."*

**Resolution: reach extends by relocation, not by governance.**

The ambient corpus stays explicitly ungoverned, and the skill says so plainly. But the **load-bearing
content** — who acts, what the system affords, what has run, what the project's priors are — relocates
into ontology entries that version, supersede, carry provenance, and carry exercise status.

The audit's evidence supports this over gating. Of eight ambient-documentation drift instances, the
expensive ones were all **ontological claims wearing prose**:

- `temper-telemetry`'s `init.rs` asserted *"No export layer, deliberately … not an omission to tidy
  up"* — a decision nobody made (repaired, `#540`).
- `TeamInvitation`'s doc comment asserted a link-based invitation flow that was never built — and that
  false premise is what made a routing decision look hard (repaired, `#543`).
- A spec marked *"accepted; all eight decisions settled"* took six corrections on first grounding.

The genuinely-ambient remainder — a stale line in a setup guide — is cheap and stays cheap.

**Why not gate the prose.** Prose has no compiler. A gate could check a marker's *presence*, never its
*truth* — a rung-1 check by the discipline's own standard, and one that would read as coverage.

---

## 8. Where this lands in the shipped skill

`temper skill install` renders the skill from askama templates, parameterized per install
(`crates/temper-cli/src/commands/skill.rs`). Two existing properties matter:

- The installed `SKILL.md` embeds a **config hash**, and `check_config_hash_staleness` detects when it
  no longer matches the config it was generated from.
- `guidance/` is deliberately **the project's namespace**: install creates it and writes nothing
  there — *"shipping into it would clobber user files on every regen."*

Changes:

- **A new supporting file** carrying the discipline (§3), shipped at the skill root beside
  `subagent-guidance.md`, `plan-verification.md` and `implementation-grounding.md`. **Not** into
  `guidance/`.
- **A router entry** in `SKILL.md`.
- **`session-lifecycle.md`** gains the criteria-in-force projection at session open (loop step 1).
- **The summary-statement discipline folds in** at loop step 7 rather than shipping separately —
  already filed as task `019f9aa5-ec66-7230-812d-4c14b5d7ed58` (backlog, build/small).

**This does not become a seventh workflow.** The six `mode × effort` files answer *how much process for
how big a job*; the register answers *what the outcome is*. They compose — a build/small still has an
outcome, it just has one clause instead of five.

---

## 9. Verification — how we would know

The discipline demands non-vacuity and bite of everything else, so it owes both of itself.

- **Non-vacuity.** The closure-staleness query must be shown to return a hole on a goal that genuinely
  has one. If it returns empty on day one that is not coverage — it is a selector that matched nothing.
- **Bite.** The expressibility check must be shown to *stop* an author: a register whose actor cannot
  resolve must refuse to resolve, not quietly accept prose.
- **Dogfood as the acceptance path.** Temper's own ontology is hand-authored from what the codebase
  already states — the machine principals, the steward and auditor personas, the human profile, the
  admin emitter, and the affordances the `Backend` trait already enumerates. Doing that by hand *is*
  the requirements-gathering for `/temper init` (spec 2).
- **The honest bound.** Whether drift actually drops is a **judged** criterion — perspective: Pete, as
  maintainer, across several sessions. It cannot be made executable, and claiming otherwise would be
  the flattening the discipline forbids elsewhere.

### 9.1 A reading on the kill-switch — negative, and it is why §3.2.1 exists

Goal `019f9a34-3306-70d1-b07a-f23c99943751`'s revision-economics clause is the kill-switch: *when
measurement contradicted a clause, what did the supersession cost?* It specifies the reading it needs
as one taken from a clause superseded **after witnesses are filed and code written against it**.

**A reading now exists, and by the goal's own criteria it does not qualify — yet it is worse in the
dimension that matters.**

What happened: a decomposition pass over a register authored against an unbuilt subject produced ten
witnesses, filed against an explicit instruction not to create unvetted tasks. Grounding showed most
were **not expressible and not implementable** — they did not fall out of any design, because there
was no design to fall out of. Six were subsequently deleted; two had already been cancelled with
reasons; the remainder stand.

**Cost: four working sessions on a sibling machine, recovering the original intent.**

Why it does not qualify, stated honestly: no code was written against the clauses, and nothing built
was discarded. Why it is nevertheless the more serious reading: **the cost was not correcting a
clause. It was recovering the intent the decomposition had buried** — which is precisely the drift
this convention exists to prevent, produced by the convention's own instrument.

Against the goal's why-anchor — *"if register-authoring itself becomes ritual costlier than the drift
it prevents, the goal has failed by its own anchor and should be superseded, not persisted with"* —
this is the anchor's stated failure condition.

**The verdict is separable, and the separation is the finding** — but the separation is weaker than
an earlier draft of this section claimed, and the correction is recorded rather than edited away.

That draft asserted the register's *elements* "earned their keep on the record" and cited three
findings. Re-examined against PR #550's disposition of the auditor register, **one of the three
survives**:

| Claimed attribution | Verdict |
|---|---|
| **Exercise status** found a schedule firing hourly and dying for a day while the spec asserted nothing was deployed | **Holds.** Attributed to element 7 at the time, not retrofitted, and acted on by PR #541 |
| **The closure discipline** found eight unclassified `domain` event types | **Weakened.** PR #550 marks that section **moot**: the classification existed to serve D3's material-event allow-list, and D3 turned out not to be needed. The discipline correctly found a gap in a mechanism that should not have existed |
| **Situated actors** — the Set 5 Critical was a situated-actors omission | **Withdrawn.** That was found by a three-lens adversarial review during Set 5 implementation, *before this register existed*. The attribution was retrofitted to strengthen an argument |

**And the arc's own conclusion cuts further** (PR #550, addressed to this goal):

> The register apparatus did not produce this arc's useful findings — **reading and executing SQL
> did**. Every correction that changed the work came from `psql` or `pg_proc` … What caught each
> round was executing the predicate, not reviewing it.

**What survives of the separation.** One element — exercise status — has a clean, contemporaneous
attribution and a production consequence. That is enough to say the decomposition instrument and the
register are separable failures, which is why §3.2.1 changes the instrument. It is **not** enough to
say the register has earned its cost. On the current record the strongest honest claim is: *one
element paid for itself once; the rest is unproven, and one arc's findings came from execution rather
than from the apparatus.*

That claim is weaker than this spec's existence implies, and it is stated here so nobody has to
discover the gap by reading PR #550 afterwards.

**Two prior warnings existed and neither became a rule.** The goal's own C3 section recorded the
termination worry as a counter-observation (*"a convention that generates work under use is not
obviously cheap… no stated termination proof"*). Arc 3 concluded *"the ambitious route has no ground
to build on so the shape evolves too much in unbuilt futures"* and *"the fail-before/pass-after bar is
trivially met by anything when 'before' means 'unbuilt.'"* Both were written down as observations.
Neither was written down as a constraint. **§3.2.1 is that constraint.**

**What is still unmet.** The reading the clause actually specified — a supersession *after code* —
remains untaken, and this spec does not supply it.

---

## 10. Decisions taken

| # | Decision | Rationale |
|---|---|---|
| D1 | The discipline is the backbone and stands alone | It delivers value with no ontology, no init flow, no substrate change. The ontology is an enhancement, never a prerequisite |
| D2 | Spec covers parts 1, 2, 4; init is spec 2; cogmap deferred | Building init first automates a flow nobody has run by hand |
| D3 | Situated actors may be enumerated in place **or** cited | Preserves D1. Citation buys computable staleness; in-place remains a complete register |
| D4 | Ontology entries are `domain`-typed resources with recognized `open_meta` keys | `list` has no `open_meta` filter, so the doc type must carry the kind for the enumeration to be complete. `open_meta.schema.json` already exists to constrain recognized keys |
| D5 | `temper-provenance` is **required** on ontology entries | It is absent on ~30% of sampled rows today; the trust axis cannot ride on a conventional field |
| D6 | Exercise status extends to ontology entries | Lets a research pass be generous without inflating the domain closure is computed against |
| D7 | Expressibility fires at authoring time, not in a sweep | Puts discovery at the only moment the answer is cheap |
| D8 | Reach extends by relocation, not governance | Prose has no compiler; a marker-presence gate would read as coverage. The load-bearing content moves instead |
| D9 | Sub-goals need no new mechanism — a sub-goal is a non-leaf node in the same tree | Amended by D12/D15: a sub-goal decomposes **clauses** by the meaning test, before the doing. Only the sub-goal→task descent is witness decomposition, and only that descent uses the demonstrability floor |
| D10 | The register does not become a seventh mode×effort workflow | Orthogonal axes: how much process vs. what the outcome is |
| D11 | No framework decision on the fixture vocabulary | The `rstest` claim was a think-with reference mistaken for an affordance. The vocabulary survives; the framework claim does not |
| D12 | **Clauses are invariants and name no mechanism; witnesses name mechanism and are authored during or after the doing** | The *how* is what mutates. Ten mechanism-shaped witnesses over an unbuilt subject cost four sessions (§9.1). The bite requirement proves the timing: "fails against current state" is vacuous when the mechanism does not exist |
| D13 | **Witness decomposition is a separately authorized act, inside the build** | The register's shape invites decomposition and an author will decompose. An explicit instruction not to was given and not held, so the constraint must be structural, not instructional |
| D14 | **Element 8 — the negative face:** what must never become true | Distinct from the refusal face (acts declined) and from the three-way Then (positive postconditions). Nearly every expensive finding in the audited window was a standing negative that no element had a slot for |
| D15 | **The clause-level floor is a meaning test — split when halves can be violated independently** | The demonstrability floor cannot govern clause splitting once witnesses come later. Granularity of *how* needs judgment; a hard rule there manufactures false precision. The demonstrability floor survives, relocated to witness decomposition |
| D16 | **The clause citation is a facet on the `advances` edge** (`kb_properties`, `owner_table='kb_edges'`) | Makes the two-spellings divergence *unrepresentable* rather than detected — rung 4, not rung 2. The link is the edge (settled by `KeyFate::Edge`); the clause qualifies the link. Alternatives put structure in a string or reverse a decided thing |
| D17 | **A task declares `witnesses` OR `enables`** | Enabling work is not evidence. Without the split, `no-witness-precedes-its-mechanism` would forbid the very work that makes witnesses possible. Evidenced: the deleted "material-event set (**unblocks** W2 and W8)" task declared itself a witness and witnessed nothing |
| D18 | **The inexpressible intersection is first-class, with exactly two honest exits** — evolve the system (`enables` task) or change the goal (supersession) | The third exit, assuming the affordance, is the deeper account of §9.1: a register met the edge of what the system could express and hallucinated it. Both honest exits already have worked instances (edge facets; R10) and neither was recognised as a fork |
| D19 | **Closure gains a fourth cell state — examined-and-inexpressible** | *Excluded-with-reason* is settled; *inexpressible* is a pending fork. Collapsing them makes a waiting cell look decided, which is how the third exit gets taken by default |
| D20 | **Miscategorisation is checked before either exit is taken** | R10 was not inexpressible — it was a verdict filed in the refusal face, tangled with a real structural refusal. Forking on a miscategorised cell evolves or changes the wrong thing. Three diagnoses, in order: miscategorised (no fork) · missing affordance (evolve) · wrongly reaching (change the goal) |
| D21 | **The register's cost is NOT claimed as proven** | Of three element-attributions an earlier draft made, one holds (exercise status), one is weakened (closure found a gap in a mechanism PR #550 marks moot), one is withdrawn (the Set 5 Critical predates the register). PR #550's own conclusion is that this arc's findings came from executing SQL, not from the apparatus |

---

## 11. Open questions

**Closed by this spec:**

- *Closure-declaration staleness projection* — the query shape is specified (§5).
- *Reach bound* — resolved as relocation (§7).

**Still open, carried forward unchanged:**

- **Witnessing economics, measured as revision cost.** The kill-switch. Still unmet (§9).
- **Clause-count discipline** — what "load-bearing intersections only" means operationally at
  authoring time. Heuristic ceiling, or judgment?
- **Evidential refusal ground** — whether refusal on evidential state is a third ground alongside
  standing and commitment, or a commitment sub-flavor.
- **M2M recourse** — what rung-3 recourse means for a machine actor.
- **The register's authoring cost** — the convention must be cheaper than the drift it prevents.
- **Generated scenario prose** — deferred; only worth building if a stakeholder audience materializes.

**Closed by evidence during this spec's own authoring:**

- **Termination of witness generation.** An earlier draft of this section read: *"Nothing shows it
  expanding faster than it closes; named so a later reader checks rather than assumes."* **That is
  false, and it was false when written.** The check came back positive — see §9.1. The resolution is
  §3.2.1: witnesses are not authored up front, so they cannot proliferate up front.
  Left as a visible scar rather than an edit, because a spec about drift that silently corrected its
  own would be the specimen.

**Still open, and newly opened:**

- **Where the granularity of *how* is communicated.** §3.2.2 makes clause splitting a judgment
  question on purpose. That leaves genuinely unresolved how the right granularity of mechanism is
  conveyed up and down a decomposition ladder — the thing that most needs subjective judgement and
  most resists a hard boundary. No mechanism is proposed here; naming it as open is the honest state.
- **Goal membership has two spellings** (§5.1) — the `advances` edge and `open_meta.witnesses.goal` —
  and nothing ties them. Found by the first run of the uncovered-clause query, on a migration the
  author of this spec had just built by hand. Three resolutions are named in §5.1; none is costed.
- **Whether the negative face is one element or an arm of the three-way Then.** Element 8 is
  introduced as standing rather than act-scoped (§3.1). A standing negative outlives any act, which is
  what regressions are — but it has not been exercised in a real register yet, and the first one may
  argue for folding it into element 4.

---

## 12. Non-goals — stated silence

Deliberately not specified here:

- **`/temper init`'s research flow** — spec 2, and named as such.
- **The ontology as a cognitive map with a charter-telos** — examined and deferred. Edges and tagging
  carry it for now.
- **Promotion of criteria to substrate mechanics** (criteria as first-class resources with an
  in-force/superseded lifecycle). Unchanged from the research document: deliberately deferred.
- **A parametrized-test framework** — see D11.
- **Governing the ambient corpus** — see D8; declared out of scope rather than silently omitted.
- **Any application outside Temper development** until the discipline has been dogfooded here.

---

## 13. Sequencing

0. **Precondition — edge-owned properties get a read and write surface** (§5.1.2). Nothing has ever
   written one; `FacetSetInput` takes a `resource`. MCP + API + CLI, per the standing parity
   intention. Everything in §5.1.1 waits on this, and nothing else here does.
1. The discipline as a shipped skill file plus router entry (§3, §8) — self-contained, no dependency
   on anything below.
2. The recognized `open_meta` conventions (§4.2) — an `open_meta.schema.json` version bump.
3. The two queries (§5).
4. Dogfood: hand-author Temper's own ontology (§9), which produces spec 2's requirements.
5. **Redraft goal `019f9a34-3306-70d1-b07a-f23c99943751` as an outcome register under this
   discipline** — the first register authored against a subject with **no incumbent substrate**, which
   is the test that goal's own overfit clause asks for. It must:
   - preserve the accumulated discovery — the three revision-cost data points, the superseded pilot
     subject, and the named remainders (the greenfield instance, the third refusal ground);
   - amend the executable-spine clause per §3.4.1;
   - record §9.1's verdict as a clause, not a footnote;
   - carry an element-8 negative face, which the current goal has nowhere to put;
   - migrate the four live clause citations (`019f9bcf-bdba-…`, `019f9bcf-e4ba-…`, `019f9a34-c8ca-…`,
     `019f9a34-8098-…`) and give clauses **readable names rather than letter-number indices**, since an
     index carries no information to a reader who does not have the document open.

   Landed through a **decision** resource — the discipline's own supersession vehicle — not as an
   in-place edit.

Steps 1 and 2 are independent and can land in either order. Step 3 depends on 2. Step 4 depends on all
three and is where the non-vacuity and bite checks in §9 are actually taken. Step 5 depends on nothing
technical and can run at any point; it is sequenced last because a register written before §3.2.1 was
understood is what §9.1 records.
