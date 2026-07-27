# The Outcome-Register Discipline — Design

**Status:** proposed · **Goal:** `019f9a34-3306-70d1-b07a-f23c99943751` · **Branch:** `jct/outcome-register-discipline-spec`

> **How to read this.** Every claim is marked **[decided — who, when]**, **[observed — evidence]**, or
> **[proposed]**. **Unmarked prose is proposed, not agreed.** Parts I–VI are what we decided and how to
> build it. The appendices are evidence and argument — you do not need them to act, and nothing in
> them is a constraint.
>
> **A sentence here should stand without its pointer.** Where a section is named, the thing it says is
> named too. A bare cross-reference doing the work of a referent is a defect.

## What this is

A discipline for stating outcomes so their rigor survives decomposition, shipped as a temper skill.

**Purpose — the four things it exists to give confidence about** [decided — Pete, 2026-07-27]:

1. We have the necessary **priors**.
2. We have the **grounding** to begin and complete.
3. We have **clarity on what success looks like**.
4. We have clarity on **what must not be true at the end**, so regression is not accidental.

**It does not decide whether a goal is worth pursuing.** Cost, churn and abandonment belong to people,
or to an agent in an explicit meta-goal frame — never to a clause inside the goal.
[decided — Pete, 2026-07-27]

## State of play — what exists, what does not

| Thing | State | Where |
|---|---|---|
| The register's 8 elements | **specified, not shipped** | Part I |
| Witness declarations on tasks | **shipped, in use** — 9 tasks carry both keys | `open_meta.witnesses` · `open_meta.witness` |
| The coverage query | **runs, not packaged** — first run caught a real dangling citation | ad-hoc script, 2026-07-26 |
| The expressibility check | **not built** | needs the taxonomy |
| Project taxonomy (`domain` resources) | **not built** — zero rows in `@me/temper` | needs `open_meta.schema.json` v3 |
| Citation as a facet on the `advances` edge | **not built; substrate never exercised** — zero edge-owned properties in production | task `019fa03a-913b-7141-a173-1c804d9b7ccd` |
| `temper warmup` as the read surface | **exists, unfit, and untouched** — still emits last session's body; carries no goals or clauses | `crates/temper-cli/src/commands/warmup.rs` |
| The installable skill | **written, not yet merged** — Part I ships as `outcome-registers.md`; the four always-in-force rules ship in `SKILL.md` itself | `crates/temper-cli/skill-content/outcome-registers.md` |
| Session-open on standing state | **written, not yet merged** — hand-rolled from `list --type goal --status active`, *not* from `warmup` | `crates/temper-cli/skill-content/session-lifecycle.md` |

## Why we are doing it

Rigor arrives late and by hand. It is found at a leaf — one adversarial pass, one careful reading, one
measurement that refutes a confident claim — then folded back manually. Things still get missed.

**Measured over five days, 2026-07-22 → 07-26** [observed — audit `019f9ee8-1675-72d0-99bb-3dea38aed84b`]:

- **12** instances of one drift shape: a declaration and its consumers with nothing tying them, or a
  check that could not fail. Each written up as *the* lesson of its own session.
- **8** ambient-documentation artifacts asserting behaviour that was not true. Every one caught by a
  person or agent reading carefully; **none by a gate**.

## What we agreed to build, and what we did not

**In scope** [decided — Pete, 2026-07-26]: the discipline as a skill · the project taxonomy · the
citation relation and its two checks.

**Out of scope, deliberately**:

- **`temper init`'s research flow.** Building it first would automate a flow nobody has run by hand.
- **The taxonomy as a cognitive map with a charter-telos.** Deferred, not rejected — edges and tagging
  carry it for now.

## How much "how" we agreed on

Enough not to wander wide, and no further:

- **Clauses state invariants and name no mechanism.** Mechanism is discovered by building. [decided]
- **Witnesses are authored during the build**, never in a preamble — a witness naming an unbuilt
  mechanism cannot bite. [decided — after it cost four sessions]
- **The taxonomy lives on `domain` resources** via recognized `open_meta` keys, not new doc types.
  [decided — `resource list` has no `open_meta` filter, so the doc type must carry the kind]
- **Payload shapes are deliberately unspecified.** A spec-invented field list is stale on arrival and
  wins over the prose beside it. [decided]

## What we will verify

- The **coverage query returns a hole** on a goal that has one. Empty on day one means it matched
  nothing, not that there is nothing.
- The **expressibility check stops an author**, rather than accepting prose.
- **Temper's own taxonomy is hand-authorable** from what the codebase already states.

**Not verified by any of this**: whether adopting the discipline was worthwhile. That is program-level
and no criterion here reaches it. Evidence bearing on it is in Appendix B, reported and not adjudicated.

---

# Part I — The discipline

The backbone. It stands alone: it delivers its value with **no taxonomy, no init flow, and no
substrate change**. Parts II and III are enhancements, never prerequisites. [decided]

## I.1 A goal carries an outcome register — eight elements

Seven come from research `019f9a32-e1b2-7f43-b4cf-ac9b58447cb9`. The eighth is added here.

| # | Element | What it states |
|---|---|---|
| 1 | **Situated actors** | Entity kind, roles, memberships, grants. Never "a user" unqualified |
| 2 | **Priors vs. provided** | What the actor already knows (answerable by query) vs. must supply per act (a payload requirement) |
| 3 | **The act** | Emission topic, on-behalf-of chain, read-vs-mutation character |
| 4 | **The three-way Then** | Synchronous postconditions · ledger trace · eventual stable-state after run-to-quiescence |
| 5 | **The refusal face** | What the system *declines*. Distinct from failure: a refusal is itself an auditable act |
| 6 | **The why-anchor** | Whose attention this saves or protects, and from what. The drift-detector |
| 7 | **Exercise status** | Has any of this ever *run*? Distinct from "specified" and from "superseded" |
| 8 | **The negative face** | What must **never** become true. A standing regression boundary [added here] |

**Element 1 changed**: situated actors may be **enumerated in place** *or* **cited** from the project
taxonomy. Citation buys computable staleness; in-place enumeration remains a complete, valid register.
This is what keeps the discipline standing alone. [decided]

**Element 7 needs two axes, not one value.** [observed — the citation auditor's schedule fired ~18
times and died every tick before doing work. *"Has the schedule fired?"* → eighteen → "exercised."
*"Does any `citation_audited` event exist?"* → zero → "never ran." Both readings alone are wrong.]

**Element 8 is new, and distinct from two things it resembles.** Not the refusal face, which covers
*acts the system declines*. Not the three-way Then, which is entirely *positive* postconditions. A
negative-face clause is a state that must not obtain regardless of which act was attempted:
*"a read-only context member must never create a resource homed in that context"*; *"standing must
never be recomputed from its own prior value"*; *"a folded source block must never become invisible to
the staleness check."*
[observed — across the audited window, nearly every expensive finding was a standing negative that no
element had a slot for.]

## I.2 Clauses are invariants; witnesses are statements about mechanism

**The load-bearing distinction, and it was paid for.** [decided — Pete, 2026-07-27]

| | States | Authored | Governed by |
|---|---|---|---|
| **Clause** | What must be true, or must never be true. **Names no mechanism** | Before the work | The meaning test |
| **Witness** | How we know. **A claim about mechanism** | During or after the build | The demonstrability floor |

**Why the timing is forced, not preferred**: a witness must fail against the state its clause claims to
change. When the mechanism does not exist, "fails against current state" is satisfied by *the absence
of the feature* — vacuously, by anything. A bite against nothing is not a bite.

**Consequences** [decided]:

- **Witness decomposition happens inside the build** and is a **separately authorized act**. The
  register's shape invites decomposition and an author will decompose, so the constraint must be
  structural rather than an instruction.
- **A clause whose mechanism is unbuilt carries a declared hole, not a filed task.**
- **The proliferation worry dissolves** rather than being gated: witnesses cannot multiply up front
  because they are not authored up front.

**Two floors, and they are different questions** [decided]:

- **Goal → sub-goal** is clause decomposition, before the work. **The meaning test: split a clause when
  its halves can be violated independently.** If two sub-clauses can only fail together, they are one
  clause.
- **Sub-goal → task** is witness decomposition, inside the work. **The demonstrability floor**, and the
  only place it can honestly be applied.

The meaning test is deliberately a judgment question. The granularity of *how* is what most resists a
hard boundary, and a rule there manufactures false precision.

## I.3 Closure — the four states of a cell

For a goal, *situated-actor × act × relevant-state* is a constructible product. Every cell is in one of
four states:

| State | Meaning |
|---|---|
| **Examined-and-specified** | Covered |
| **Examined-and-deliberately-excluded** | Emitter, timestamp, stated reason. **Settled** |
| **Examined-and-inexpressible** | Wanted; the system cannot express it. **A pending fork** [added here] |
| **Unexamined** | A visible remainder, not a silent one |

**The third is a peer of the second, not a flag on it** [decided]: an exclusion is *settled*, an
inexpressible cell is *waiting*. A reader scanning "excluded, reason given" moves on — which is how the
fork silently fails to be taken.

**Equivalence claims are first-class and falsifiable.** Nobody examines cells one by one; the matrix
collapses via claims like *"all actors lacking grant G are interchangeable for this act."* Most holes
live not in unexamined cells but in **wrong equivalence claims** — cells examined under a class
abstraction that did not hold. A closure section records: dimensions considered, classes claimed, cells
examined per class, cells excluded with reasons, remainder marked unexamined.

> **Scar, carried from the source research.** The claim that temper's categories are "closed-world in
> the dimensions that matter" is *itself* a wrong equivalence claim. Every dimension it enumerated is
> **authorization-shaped**. The property holds there; it does **not** hold for cadence, rate or volume,
> and evidential standing is saturated with those.
>
> **Closure declarations must state which axes they close over, and rate-shaped axes must be named as
> open unless explicitly enumerated.** The project taxonomy does **not** close this — personas and
> affordances are authorization-shaped, so hoisting them buys the closed-world property on those axes
> only.

**Bounds** [decided]: closure declarations only at load-bearing intersections. "Unexamined" stays a
legitimate, sayable state. The goal is a **named** remainder, not a zero remainder.

## I.4 The inexpressible intersection — two exits, and a third that is a trap

Sometimes the system **as designed cannot express** an element: the persona does not exist, the act has
no affordance, the projection a Then-clause asserts over is not built, the refusal named is one the
system cannot make.

**Two honest exits** [decided]:

- **Evolve the system** — the affordance is missing and should exist. Produces an **`enables`** task.
- **Change the goal** — the affordance should not exist, or the clause was reaching wrongly. Produces a
  **supersession**, via a decision.

**The dishonest third exit is to assume the affordance and write clauses against it.** That is the
deeper account of the four-session failure in Appendix B: a register met the edge of what the system
could express and, with no fork available to record, **hallucinated the affordance**.

**Neither exit is taken by the discipline.** Detection is in scope — the register can show that an
element cannot be expressed, and name the fork. Taking it is a judgment by whoever owns the frame.

**Check for miscategorisation first** [decided]. Three diagnoses, in order:

1. **Miscategorised** — the element is not the kind of thing the register claimed. Restate it in the
   right element. *No fork.*
2. **Missing affordance** — the element is right; the system cannot do it. → **evolve**.
3. **Wrongly reaching** — the element is right; the system should not do it. → **change the goal**.

*Worked instance of miscategorisation*: the auditor register recorded *"I cannot assess this citation"*
as a refusal the system could not express. It is not a refusal at all — it is a **verdict**, and a
strongly negative one. The auditor's role is to say whether an assertion holds up under scrutiny of its
evidence; *"I cannot assess this"* means *"this is incommunicable, or I cannot evaluate where you
derived it"*, which is worse than *"plausible but underwarranted."* Forking on it would have argued for
building a refusal the system correctly does not have. [decided — Pete, 2026-07-27]

## I.5 Verification modes

| Mode | What it requires |
|---|---|
| **Executable** | A witness **shown to fail** against the state its clause claims to change |
| **Replay-verified** | Run the cascade to quiescence against the test ledger, then assert |
| **Judged** | Satisfied *as judged from a named perspective*, exemplified by two or three cited regions |

**A judged criterion needs its perspective and its exemplars named.** *"Does this look like
`relationships.rs` looks"* is tractable; *"does this align with our patterns"* fails silently.

**The Gherkin file layer is deliberately skipped** [decided]. The value was the decomposition
discipline, which the register carries at goal and task level.

**The fixture vocabulary is corrected.** The source research names the executable mode as "rstest +
sqlx::test". [observed — `rstest` is not a dependency of this workspace: zero `#[rstest]`, `#[case]`,
`#[fixture]`, `#[values]`, against 1,186 `#[sqlx::test]`, 1,456 `#[test]`, 50 `#[tokio::test]`.] The
reference entered from an ideation link explicitly stated as not-in-the-repo. **The vocabulary
survives — `given_entity`, `given_grant`, `emit`, `project_as_of`, `run_to_quiescence` — the framework
claim does not, and this spec makes no framework decision.** [decided]

## I.6 Enclosure of responsibility — what a clause may not reach

> Criteria that describe the interiority of a thing are framed within the priors that situate them, but
> they cannot reach beyond or up into the frame of their priors to mutate them. That is an **enclosure
> of responsibility error**. [decided — Pete, 2026-07-27]

A goal's clauses may say what must be true and what must never become true **for this goal to have been
achieved well**. They may not say whether pursuing it was right, whether its cost is justified, or
whether it should be abandoned. That judgment belongs to the engineer, the product manager, the business
partner weighing cost against churn, or an agent in an explicit meta-goal frame.

**The discipline detects; it does not decide.**

| Mechanism | May | May not |
|---|---|---|
| Closure | Mark a cell unexamined or inexpressible | Decide the goal is not worth closing |
| The inexpressible intersection | **Surface** the evolve-or-change fork | **Take** it |
| Witnessing | Show a clause uncovered | Conclude the clause should be dropped |
| Exercise status | Report that something never ran | Conclude it should not exist |
| A verification mode | Say a criterion failed | Say the goal was misconceived |

**Cost, churn and worth stay worth measuring — they are simply not clauses.**

## I.7 Two guardrails that cut against everything above

- **Load-bearing intersections only.** Clauses concentrated where drift is catastrophic. *"Forty
  scenarios is spec-fall in a new costume."*
- **Stated silence is first-class.** *"We are intentionally not specifying X"* is itself a clause of the
  shared-incompleteness record.

**The skill must teach these with equal weight to everything else.** A skill that only teaches how to
write a register produces exactly the spec-fall it exists to escape.

---

# Part II — The project taxonomy

## II.1 What it is, and why the word matters

**Taxonomy, not ontology** [decided — Pete, 2026-07-27]. Ontology addresses being and is-ness; taxonomy
addresses categorisation, labelling, speciation. An information domain is accurately operationalised as
a taxonomy. Calling it an ontology would imply the categories describe reality rather than our
labelling of it.

**And the word is already taken.** [observed — temper publishes a public theory page titled *"Ontology:
data, intention, information, knowledge"*, four ontological layers each derivable from those below
(`packages/temper-ui/src/routes/(public)/theory/ontology/+page.svelte`).] Same de-collision problem the
corpus solved once before, when evidential standing was renamed `evidence` to avoid colliding with
principal access-standing.

**Why it exists**: closure is only computable where the dimensions it quantifies over are enumerable. A
project that has not told temper its personas has nothing to enumerate over, so its closure sections are
prose and its remainder is unaskable.

**Three of its four categories are register elements hoisted from goal scope to project scope**, and the
fourth is the matrix they generate:

| Category | Register element | Scope |
|---|---|---|
| personas | 1 — situated actors | project |
| affordances | 3 — the act | project |
| priors and fundamentals | 2 — priors vs. provided, plus constraints | project |
| scenarios | the closure product — actor × act × state | derived |

## II.2 Carrier and shape

**Doc type `domain` carries every taxonomy entry. A recognized `open_meta` key names its kind.** [decided]

*Why the doc type carries the kind*: [observed — `resource list` filters on `--type`, `--context`,
`--cogmap`, `--title-contains`, `--stage`, `--goal`, `--status`, and has **no `open_meta` filter**.] The
closure query needs a complete enumeration; filtering client-side over a paged list is the truncation
trap. `list --type domain --all` is complete and server-side.

*Why `domain`*: semantically exact, and effectively free. [observed — zero rows in `@me/temper`, eight
repo-wide. No collision with the 66 existing `concept` rows.]

*Why a recognized `open_meta` key rather than a new doc type*: `open_meta.schema.json` exists for exactly
this and says so — *"documents the keys temper recognizes and constrains their shape. A recognized key
with a wrong shape is a bug."* It is already versioned by migration. A new doc type would mean an enum
change, a per-type schema, a seed, and full MCP + API + CLI parity.

| Key | Carries |
|---|---|
| `persona` | entity kind (required), roles, memberships, grants |
| `affordance` | the act's topic, on-behalf-of shape, read-vs-mutation character |
| `prior` | what an actor already knows vs. must supply; project constraints |

**Exactly one key per entry** [decided]. Two is a modelling error, not a multi-kind entry: a persona
that is also an affordance means actor and act have been conflated, which is the distinction elements 1
and 3 exist to hold apart. The schema should make it unrepresentable.

**Field lists are deliberately not authored here**, and are grounded against
`crates/temper-workflow/schemas/open_meta.schema.json` at implementation time.

## II.3 Two axes, both already available

- **Provenance** — the existing managed enum `temper-provenance` (`user-created` | `llm-discovered`).
  *"Here are our personas, take our word"* → `user-created`. *"Go research it"* → `llm-discovered`.
  **Required on taxonomy entries, not conventional** [decided] — [observed: absent on ~30% of sampled
  rows today: 58 `llm-discovered`, 87 `user-created`, 61 absent].
- **Exercised** — whether a real goal has **cited** it. No new field. **"Cited" means named by a
  register**, not merely referenced: an entry linked from a session note is *mentioned*, not exercised.

An entry that is `llm-discovered` and unexercised is a **proposal**. `user-created` and unexercised is
**a claim, not yet load-bearing**. Exercised is **doing work**. All three are legitimate, and the pair is
what lets a research pass be generous without inflating the domain closure computes against.

## II.4 The discipline must be useful at low resolution

A project that can say only *"we have customers and internal admins"* has a legitimate two-entry
taxonomy. Coarse domain; the discipline still runs; expressibility pressure sharpens it. **This is a
design constraint, not a tolerance** [decided] — if adoption requires a rich taxonomy first, it becomes a
large upfront cost and fails its own why-anchor.

---

# Part III — The citation relation

## III.1 Two checks, both set differences

**Closure staleness.** A goal's closure declaration records the persona refs it closed over. A persona
exists whose ref is not in that list ⇒ the declaration is visibly incomplete. Set difference against
`list --type domain --all`.

**Expressibility.** At **authoring** time, not in a sweep: **where a register elects to cite the
taxonomy**, its situated actors must resolve to persona refs. One that does not resolve is the signal —
either the clause is vague, or the project's self-model has a gap and the persona is born.

> **Both checks are conditional, and that is load-bearing.** A register that enumerates its actors **in
> place** is complete and must not be blocked. An implementation making persona resolution unconditional
> breaks the standalone property and is wrong. A closure declaration over an in-place enumeration is
> **outside the query's domain** — reported as such, never silently counted clean.

Synchronous-at-authoring is the better property for expressibility: it puts the discovery at the only
moment the answer is cheap.

## III.2 A defect the first run of this query found

**Two spellings of "this task belongs to that goal" exist, and nothing ties them** [observed]:

- the `advances` **edge**, the only thing `resource list --goal` filters on;
- `open_meta.witnesses.goal`, the **citation** the witness convention uses.

[observed — task `019f975e-7be9-7ff3-a5bd-ef7ea72ff4a5` carries the citation and **no edge**, so it is
invisible to `list --goal`. A clause migration built from that list missed it and left a dangling
citation. The migration was hand-built by this spec's author an hour after specifying that the query
should exist; the query caught it on first run.]

**Resolution: the citation becomes a facet on the `advances` edge** — a `kb_properties` row with
`owner_table = 'kb_edges'`. The link is the edge; the clause qualifies the link. [decided — Pete,
2026-07-26]

| Grounding | Evidence |
|---|---|
| `advances` is a **label** on a `leads_to` edge, not an `edge_kind` | `db_backend.rs:94` · `substrate_read.rs:201` |
| The goal *link* being an edge and not a property is already settled | `keys.rs:20` — `KeyFate::Edge`, for `temper-goal` |
| `kb_properties` admits `owner_table='kb_edges'` by design | `canonical_schema.sql:656` — *"§4a edges carry facets"* |
| Uniqueness is `(owner, key, value)` | N clauses = N rows, no encoding needed |

**Why this over the alternatives**: it makes the divergence **unrepresentable** rather than detected — a
citation's owner must be an edge id. Putting the clause in the edge `label` would also work but puts
structure in a string. Making the edge a projection of the citation was rejected: it reverses a decided
thing with no evidence against it.

## III.3 The precondition — never run is not wrong

[observed — production has **zero** edge-owned properties: `kb_resources` 10,590 · `kb_content_blocks`
37 · `kb_edges` **0** · `kb_cogmaps` **0**. No surface can write one: `FacetSetInput` takes a `resource`.]

That is a second instance of a confident DDL comment describing something nobody executed. **But
unexercised is not wrong** — element 7 exists precisely to convert a never-run artifact from an apparent
constraint into **open design space**. Edges were always intended to carry facets; the write path was
never built.

**So this goal takes on a precondition**: expand the read and write surfaces for edge-owned properties,
across MCP, API and CLI. [decided — Pete, 2026-07-26] Task `019fa03a-913b-7141-a173-1c804d9b7ccd`.

Until it lands, `open_meta` holds, and **any query over goal membership must read the citation, not the
edge** — a query built on `--goal` silently under-reports.

## III.4 A task either *witnesses* a clause or *enables* one

[decided — Pete, 2026-07-26]

- **`witnesses`** — this task is the evidence. Subject to the bite requirement and to the rule that no
  witness precedes its mechanism.
- **`enables`** — this task builds the mechanism that makes a clause witnessable. Not evidence, not
  subject to bite, and legitimately filed before the witness exists.

Without the split, the rule forbidding premature witnesses would also forbid the work that makes
witnesses possible.

[observed — the deleted task *"Make the material-event set a first-class enumerable object (**unblocks**
W2 and W8)"* declared itself a witness while its own title says it unblocks two others. It witnessed
nothing. Enabling work miscast as evidence.]

---

# Part IV — The workflow vocabulary and the loop

| Member | Role |
|---|---|
| **Goal** | Carries the register |
| **Sub-goal** | A non-leaf node — decomposes **clauses** by the meaning test. Names no mechanism |
| **Task** | A leaf — carries `witnesses` or `enables`, and a witness descriptor |
| **Research** | Where a grounding pass lands |
| **Decision** | The supersession vehicle |
| **Session** | Opens with the criteria-in-force projection; closes by updating exercise status |

**Research is placed by evidence**: across the audited sessions the expensive findings all came from
grounding passes, and they had nowhere to live but a session note that then had to be re-read.

**The two `open_meta` keys already in use stay as they are** [observed — across the 50 most recent tasks
in `@me/temper`: 9 carry both, 4 carry only the pointer]: `witnesses` = `{goal, clauses, child_of}` is
the pointer up; `witness` = `{id, mode, clause, floor, bites_against, route}` is the witness describing
itself. Two concepts, not a drifted spelling.

## The loop

1. **Open** — the criteria-in-force projection: what holds, what is superseded, where the scars are.
2. **Author or resume** — the invariants and the negative face. **No mechanism named.** Expressibility
   fires here.
3. **Decompose the clauses** — by the meaning test. A clause whose mechanism is unbuilt carries a
   **declared hole**, not a filed task.
4. **Ground** — a grounding pass lands as research.
5. **Build — and author the witnesses here.** First honest point; a separately authorized act.
6. **Contradict** — measurement refutes a clause ⇒ decision, supersession, visible scar. Never a silent
   edit.
7. **Close** — exercise status updates; every closing note carries **hard follow** / **accepted** / **for
   the record** / **nothing**.

> **Steps 3 and 5 were one step in an earlier draft, and that was the defect.** Collapsing them is what
> lets a preamble pass emit mechanism-shaped tasks for a mechanism nobody has built.

**This does not become a seventh workflow.** The six `mode × effort` files answer *how much process for
how big a job*; the register answers *what the outcome is*. They compose.

---

# Part V — Where this lands

## V.1 Three families in the installed skill

| Family | Files today | Where it should live |
|---|---|---|
| **The tool** | `reference.md` (generated from the clap tree), `cognitive-maps.md`, `teams.md`, `knowledge-base.md` | Shipped, generated, universal. Unchanged |
| **The discipline** | `session-lifecycle.md`, `subagent-guidance.md`, the grounding pair, `workflows/*` | Shipped, universal, **repo-agnostic**. Where Part I lands |
| **The project** | `guidance/` — created empty, shape undefined | Lives in temper; see below |

**Why `guidance/fundamentals.md` keeps getting relocated and rewritten in other projects**: we name the
slot, define no shape, and write nothing into it. [observed — `install` creates `guidance/` and
deliberately never writes there (`skill.rs:451`); `SKILL.md` references the file three times without
saying what it contains; `/temper init` ships no template.] The shape is inferred per project from
surrounding context that is entirely temper-flavoured, so elsewhere the inference is wrong and the agent
rewrites. That is the correct move given what it was handed.

**The shape it has been missing is the project taxonomy.** A commands table is not wrong — it is **one
category of prior**, in a document that never named its categories.

**And a durability argument explains the rewriting better than shape does** [decided — Pete,
2026-07-27]: a file under `~/.claude/skills/temper/guidance/` is per-machine, unversioned and invisible
to the team — structurally a scratch file. As `domain` resources it is versioned, superseded and
team-contributable. That is the difference between a thing that gets **rewritten** and a thing that gets
**amended**.

**Named risk**: a fresh clone with no auth has no taxonomy until it pulls. The skill must degrade to
*"no project taxonomy yet"* rather than erroring.

## V.2 The read surface — `temper warmup`, redesigned

**`warmup` is the incumbent** — *"Context primer for new sessions"* — so no second command is built. **It
is not fit for purpose as designed** [decided — Pete, 2026-07-27]. Three reasons:

1. **It primes on narrative recency.** [observed — it emits `last_session_content`, the whole previous
   note capped at 500 lines, and carries no goals, no clauses, no in-force state, no scars, no
   fundamentals.]
2. **"The last session" is not *your* last session.** Several sessions run concurrently on different
   machines. [observed — this spec's own authoring session opened on a note from another machine
   describing a branch and commit that did not exist locally.] A primer leading with that is **actively
   misleading**, and its confidence scales with how well the sibling writes.
3. **It is itself unexercised.** [observed — the installed skill references `warmup` exactly once, in
   `reference.md`, which is *generated from the clap tree*. No session-start routine calls it.]

**What it grounds in instead**: **guidance, goals, and tasks** — standing state.

**`last_session_content` is dropped, not shrunk.** What survives is `recent_sessions` — **titles and
dates only**, with the count **configurable**. [decided — Pete, 2026-07-27]

**Why titles rather than bodies, and it is not a compromise**: a title is a **pointer**; a body is a
**claim**. Pointers let a reader recognise which arc is theirs and go read it deliberately. A body
asserts a relevance the primer cannot establish, first, with the authority of being the only prose in
the payload.

[observed — blast radius is near zero: `WarmupResult` is a CLI-local struct with no `ts-rs` and no
OpenAPI presence, and no consumer of the shape was found.]

**Reading live is what settles drift**: any on-disk copy becomes an offline cache, not a source.

## V.3 Changes to the shipped skill

- **A new supporting file** carrying Part I, at the skill root beside `subagent-guidance.md` — **not**
  into `guidance/`, which is the project's namespace.
- **A router entry** in `SKILL.md`.
- **`session-lifecycle.md`** gains the criteria-in-force projection at session open.
- **The summary-statement discipline folds in** at loop step 7 rather than shipping separately — already
  filed as task `019f9aa5-ec66-7230-812d-4c14b5d7ed58`.

**Built 2026-07-27. The routing split is decided here rather than above** [decided — Pete,
2026-07-27]. `outcome-registers.md` is ~230 lines that only a goal-authoring session needs, so loading
it on every task start would be a standing tax on work that authors no goal. It is reached **on
demand** from the routing table, while the four rules always in force — clauses name no mechanism ·
witnesses are authored during the build · coverage is never inferred from absence · the discipline
detects and does not decide — ship **inside `SKILL.md`**, which is loaded whole.

The failure this split has to survive already happened once: `implementation-grounding.md` shipped
correct and unread for weeks because `SKILL.md`'s numbered steps named only `guidance/fundamentals.md`.
Being shipped is not being reachable, and `skill check` reports the file present either way. So the
stanza is pinned by its **content**, not its heading, in
`skill_md_carries_the_outcome_discipline_stanza_not_just_the_pointer`.

**The fold got wider than "loop step 7."** Task `019f9aa5-ec66-7230-812d-4c14b5d7ed58` specifies four
rules and only the first is a status vocabulary. The other three — say so explicitly when there are
no hard follows · the *caused-by-material vs. filling-a-slot* self-check · the restraint applies to
summary-time criticism only, never to in-flight rigor — landed in `session-lifecycle.md`, where a
session actually closes, rather than in the on-demand file. That task stays open until this merges.

---

# Part VI — Sequencing

| # | Step | Depends on |
|---|---|---|
| 0 | **Precondition** — edge-owned properties get read and write surfaces, MCP + API + CLI | nothing |
| 1 | The discipline as a shipped skill file plus router entry | nothing |
| 2 | The recognized `open_meta` conventions — an `open_meta.schema.json` version bump | nothing |
| 3 | The two checks, **and `warmup` redesigned to consume them** | step 2 |
| 4 | **Dogfood** — hand-author temper's own taxonomy | steps 1–3 |
| 5 | Amend goal `019f9a34-3306-70d1-b07a-f23c99943751`'s executable-spine clause | nothing |

Steps 1 and 2 are independent. **Step 0 is not on the critical path for guidance-in-temper** — it serves
the citation-as-edge-facet work only. Step 4 is where the coverage and bite checks are actually taken.

---
---

# Appendix A — Decisions

Everything below this line is **supporting evidence and argument**. It is not a constraint.

| # | Decision | Why |
|---|---|---|
| D1 | The discipline is the backbone and stands alone | Delivers value with no taxonomy, no init, no substrate change |
| D2 | Spec covers the discipline, the taxonomy, and the citation relation; init is spec 2 | Building init first automates a flow nobody has run by hand |
| D3 | Situated actors may be enumerated in place **or** cited | Preserves the standalone property |
| D4 | Taxonomy entries are `domain`-typed resources with recognized `open_meta` keys | `list` has no `open_meta` filter, so the doc type must carry the kind |
| D5 | `temper-provenance` is **required** on taxonomy entries | Absent on ~30% of sampled rows; the trust axis cannot ride on a convention |
| D6 | Exercise status extends to taxonomy entries | Lets a research pass be generous without inflating the domain |
| D7 | Expressibility fires at authoring time, not in a sweep | Puts discovery at the only moment the answer is cheap |
| D8 | Reach extends by **relocation**, not governance | Prose has no compiler; a marker-presence gate reads as coverage |
| D9 | A sub-goal is a non-leaf node decomposing **clauses** | Only the sub-goal→task descent uses the demonstrability floor |
| D10 | The register is not a seventh mode×effort workflow | Orthogonal axes |
| D11 | No framework decision on the fixture vocabulary | `rstest` was a think-with reference mistaken for an affordance |
| D12 | Clauses name no mechanism; witnesses are authored during the build | The *how* is what mutates; bite is vacuous against an absent mechanism |
| D13 | Witness decomposition is a separately authorized act | An instruction not to decompose was given and did not hold |
| D14 | Element 8 — the negative face | Nearly every expensive finding was a standing negative with no slot |
| D15 | The clause-level floor is a meaning test | Granularity of *how* needs judgment; a rule manufactures false precision |
| D16 | The clause citation is a facet on the `advances` edge | Makes the divergence unrepresentable rather than detected |
| D17 | A task declares `witnesses` **or** `enables` | Enabling work is not evidence |
| D18 | The inexpressible intersection has exactly two honest exits | The third — assuming the affordance — is the deeper account of Appendix B's failure |
| D19 | Closure gains a fourth cell state | Excluded is settled; inexpressible is a pending fork |
| D20 | Miscategorisation is checked **before** either exit | Forking on a miscategorised cell changes the wrong thing |
| D21 | Cost and worth are reported, never claimed or adjudicated | Enclosure of responsibility |
| D22 | `warmup` is the read surface, redesigned rather than extended | It primes on narrative recency; "the last session" is a sibling's |
| D23 | Project guidance lives in temper; any file is an offline cache | A per-machine unversioned file is structurally a scratch file |
| D24 | **Enclosure of responsibility** — a clause may not reach up into its frame | The discipline detects; it does not decide |
| D25 | **Taxonomy, not ontology** | Ontology is being; taxonomy is categorisation. Also collides with temper's published theory page |

# Appendix B — Program-level record

**Reported, not adjudicated.** Cost and churn evidence, kept because it was paid for. Nothing here is a
criterion and nothing here decides anything.

## B.1 The expensive event, and why the clause-versus-witness rule exists

A decomposition pass over a register authored against an **unbuilt** subject produced ten witnesses,
filed against an explicit instruction not to create unvetted tasks. Most were **not expressible and not
implementable** — they did not fall out of any design, because there was no design to fall out of. Six
were later deleted; two had already been cancelled with reasons.

**Cost: four working sessions on a sibling machine, recovering the original intent.**

The cost was not correcting a clause. It was **recovering the intent the decomposition had buried** —
the drift this convention exists to prevent, produced by the convention's own instrument.

**The verdict is separable, and weaker than an earlier draft claimed.** That draft asserted the
register's elements "earned their keep" and cited three findings. Re-examined, **one of three survives**:

| Claimed | Verdict |
|---|---|
| **Exercise status** found a schedule firing hourly and dying while a spec asserted nothing was deployed | **Holds.** Attributed contemporaneously; acted on by PR #541 |
| **Closure** found eight unclassified `domain` event types | **Weakened.** PR #550 marks that section moot — the classification served a mechanism that turned out not to be needed |
| **Situated actors** — the Set 5 authorization critical | **Withdrawn.** Found by adversarial review *before this register existed*. Retrofitted to strengthen an argument |

**And the arc's own conclusion cuts further** (PR #550, addressed to this goal):

> The register apparatus did not produce this arc's useful findings — **reading and executing SQL did**.
> What caught each round was executing the predicate, not reviewing it.

**Strongest honest claim**: one element paid for itself once; the rest is unproven; one arc's findings
came from execution rather than from the apparatus.

## B.2 Two prior warnings that never became rules

The goal recorded the proliferation worry as a *counter-observation*. A session concluded that *"the
fail-before/pass-after bar is trivially met by anything when 'before' means unbuilt."* **Both were
written down as observations. Neither was written down as a constraint.** The negative face and the
clause-versus-witness rule are that constraint.

## B.3 Corrections this document made to itself

- An earlier draft wrote *"nothing shows it expanding faster than it closes."* **That was false when
  written.** Left as a visible scar rather than edited away.
- An earlier draft restated the closure cell states in **two places**. Two copies of one enumeration is
  the drift this document diagnoses, committed inside it.
- An earlier draft carried a clause whose job was to **retire the goal** if correction proved
  expensive — carried forward unratified from the pre-redraft goal body, then amplified into a named
  clause and a section. Withdrawn as an enclosure error; decision
  `019fa343-d423-72e2-96a2-2cd122410ba3`.
- This document was **1,087 lines, 67% prose**, before this rewrite. Verbosity is not only a reading
  cost: exposition is where unratified assertions hide, because prose reads as uniformly agreed.

# Appendix C — Open questions and stated silence

## C.1 Still open

- **Witnessing economics, measured as revision cost.** Unmeasured. Program-level material, not a
  criterion.
- **Clause-count discipline** — what "load-bearing intersections only" means operationally.
- **Evidential refusal ground** — whether refusal on evidential state is a third ground alongside
  standing and commitment.
- **M2M recourse** — what recourse means for a machine actor.
- **Where the granularity of *how* is communicated** up and down a decomposition ladder. Named as open
  because the meaning test deliberately leaves it to judgment.
- **Whether the negative face is one element or an arm of the three-way Then.**
- **Goal membership has two spellings** and nothing ties them until the edge-facet work lands.

## C.2 Stated silence — deliberately not specified

`temper init`'s research flow · the taxonomy as a cognitive map with a charter-telos · promotion of
criteria to substrate mechanics · a parametrized-test framework · governing the ambient corpus · any
application outside temper development until this is dogfooded here.

## C.3 Named remainders — examined and deferred

- **The greenfield instance.** The steward promotion-to-action gate remains the designated subject for a
  register written with no incumbent, and the only one that can settle whether evidential refusal is a
  genuine third ground.
- **The two-versus-three-ground refusal taxonomy** therefore stays open.
