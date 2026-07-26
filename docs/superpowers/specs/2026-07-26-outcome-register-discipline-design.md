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

### 3.1 The register — seven elements

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
- **The decomposition floor is demonstrability, not size.** Stop when a criterion's witnesses are
  directly demonstrable. If a child still needs children to be checkable, keep going.

### 3.3 The closure discipline

For a goal, the space of *situated-actor × act × relevant-state* is a constructible product. Every
cell is in one of three states:

- **Examined-and-specified**
- **Examined-and-deliberately-excluded** — with emitter, timestamp, and stated reason
- **Unexamined** — a *visible* remainder, not a silent one

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

**Goal, sub-goal and task are one kind of node at three heights.** The demonstrability floor decides
which you are writing: a task when its witnesses are directly demonstrable, a sub-goal when they still
need children. This replaces *"how big is this"* with a question that has an answer.

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
2. **Author or resume** — write or reload the register. The expressibility check fires here.
3. **Decompose** — witnesses down to the demonstrability floor.
4. **Ground** — a grounding pass lands as research.
5. **Build** — tasks execute; witnesses must bite.
6. **Contradict** — measurement refutes a clause ⇒ decision, supersession, visible scar. Never a
   silent edit.
7. **Close** — exercise status updates to what actually ran; every closing note carries
   **hard follow** / **accepted** / **for the record** / **nothing**.

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

**The kill-switch is unchanged and still unmet.** Goal `019f9a34`'s revision-economics clause needs a
reading from a clause superseded *after* witnesses are filed and code written against it. Three data
points exist; none is that. **This spec does not supply it and must not be read as supplying it.**

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
| D9 | Sub-goals need no new mechanism — a sub-goal is a non-leaf witness | The demonstrability floor already describes a tree of arbitrary depth |
| D10 | The register does not become a seventh mode×effort workflow | Orthogonal axes: how much process vs. what the outcome is |
| D11 | No framework decision on the fixture vocabulary | The `rstest` claim was a think-with reference mistaken for an affordance. The vocabulary survives; the framework claim does not |

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

**Opened by this spec:**

- **Termination of witness generation.** A convention that generates witnesses under use has no stated
  termination proof. Observed once: a decomposition pass produced two *new* witnesses. Nothing shows
  it expanding faster than it closes; named so a later reader checks rather than assumes.

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

1. The discipline as a shipped skill file plus router entry (§3, §8) — self-contained, no dependency
   on anything below.
2. The recognized `open_meta` conventions (§4.2) — an `open_meta.schema.json` version bump.
3. The two queries (§5).
4. Dogfood: hand-author Temper's own ontology (§9), which produces spec 2's requirements.
5. Amend goal `019f9a34-3306-70d1-b07a-f23c99943751`'s executable-spine clause (§3.4.1).

Steps 1 and 2 are independent and can land in either order. Step 3 depends on 2. Step 4 depends on all
three and is where the non-vacuity and bite checks in §9 are actually taken.
