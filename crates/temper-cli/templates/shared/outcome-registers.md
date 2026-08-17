# Outcome Registers — stating an outcome so its rigor survives decomposition

Read this when **authoring or amending a goal or sub-goal**, or when deciding whether a criterion
belongs on one. It is not needed to start a task, run a workflow, or save a session.

A register exists to give confidence about four things, and nothing else:

1. We have the necessary **priors**.
2. We have the **grounding** to begin and complete.
3. We have **clarity on what success looks like**.
4. We have clarity on **what must not be true at the end**, so regression is not accidental.

**It stands alone.** Everything here works against the surface you already have — a body you write
and two `open_meta` keys. It needs no schema change, no query, and no server feature. Where a richer
mechanism would help, this file says so and tells you what to do without it.

## Altitude — scale the register to a goal's blast radius, not its effort

The `mode × effort` workflows scale **process to job size**. This discipline answers a different
question — *what the outcome is* — and is explicitly **not** a seventh `mode × effort` cell (see the
end of *The loop*). That leaves a gap worth naming out loud: nothing scales **register depth to a
goal's consequence**, and the result is mis-scaling in *both* directions.

- **Over-armor.** A full eight-element register authored reflexively on a goal whose blast radius does
  not warrant it. The *"padding reads as coverage"* guardrail catches padding *within* a register; it
  does not catch a complete register on a throwaway goal — that is not padding, it is the wrong
  instrument, and no guardrail below names it.
- **Under-armor, the more dangerous one.** A consequential goal gets a thin register because the
  author did not *feel* the weight. Nothing signals *"this one earns the full treatment."*

**The decisive signal is blast radius, not effort**: how many downstream readers inherit this register
as ground truth — an implementing agent, a sibling session, a downstream API consumer — and how
reversible acting on it is. That is orthogonal to how much *work* the goal is. A high-radius goal
earns the full eight elements even if it is little work; a low-radius one earns a thin register even
if it is a lot of work.

**This names the signal; it does not set the threshold.** Where to draw the line is the frame owner's
call, exactly as cost and worth are — see *Enclosure of responsibility*. The discipline detects that a
goal is high-radius; it does not decide that a given goal clears the bar for the full treatment.

## Using this under fan-out — inject the discipline verbatim

This file is written to survive **propagation to a reader who does not have the surrounding
conversation** — which is the whole reason it is *principles, not checklists*, and why *"the worked
example is evidence for the principle, never the scope of it"* is stated so insistently. Those choices
are load-bearing the moment a controller runs a **fan-out**: dispatch several subagents to author or
amend registers, hand each the relevant sections **verbatim**, and they come back coherent with each
other and with the discipline rather than as several divergent interpretations.

**The contract: inject verbatim, never paraphrased.** A paraphrase propagates the paraphraser's lossy
compression — the same loss [`implementation-grounding.md`](implementation-grounding.md) names for
plans (*"paraphrase is the very loss this guidance exists to prevent"*), and the reason
[`subagent-guidance.md`](subagent-guidance.md) opens by telling a dispatcher to include principles in
full. So when a controller fans out register authoring, the intended move is to inject these sections
whole; paraphrasing them defeats the property that makes the fan-out hold together. The three
compose — read them as one contract, not three re-derivations of it.

## Read the guardrails first, because they cut against everything below

These are not caveats. A file that teaches only *how to write a register* produces exactly the
spec-fall it exists to escape, so treat these with the same weight as the elements.

- **Load-bearing intersections only.** Put clauses where drift is catastrophic, not everywhere it is
  possible. *Forty scenarios is spec-fall in a new costume.*
- **Stated silence is first-class.** *"We are intentionally not specifying X"* is itself a clause —
  it records shared incompleteness instead of leaving a reader to infer coverage from absence.
- **A named remainder beats a zero remainder.** The goal is never to fill every section. An
  explicitly unexamined cell is a finding; a quietly missing one is a hole.
- **Sections you have nothing real to say in should say so, briefly.** Padding a register reads as
  coverage and is worse than an empty heading.

## The eight elements

| # | Element | What it states |
|---|---|---|
| 1 | **Situated actors** | Entity kind, roles, memberships, grants. Never "a user" unqualified |
| 2 | **Priors vs. provided** | What the actor already knows (answerable by query) vs. must supply per act (a payload requirement) |
| 3 | **The act** | What is emitted, the on-behalf-of chain, read-vs-mutation character |
| 4 | **The three-way Then** | Synchronous postconditions · ledger/audit trace · eventual stable-state once everything downstream has settled |
| 5 | **The refusal face** | What the system *declines*. Distinct from failure: a refusal is a well-formed act the system said no to, and it is itself auditable |
| 6 | **The why-anchor** | Whose attention this saves or protects, and from what. The drift-detector |
| 7 | **Exercise status** | Has any of this ever *run*? Distinct from "specified" and from "superseded" |
| 8 | **The negative face** | What must **never** become true. A standing regression boundary |

Three of these are routinely got wrong.

**Element 1 — enumerate in place, or cite.** Listing your actors inline, in a table, is a complete
and valid register. Citing them from a project-level source buys computable staleness later, but a
register that enumerates in place is never second-class and must never be blocked for it. This is
what lets the discipline work on day one in a project that has modelled nothing.

**Element 7 — two axes, not one value.** *Has the trigger fired?* and *has the work executed?* are
independent, and reporting one as the other is a repeatable failure. A schedule fired about eighteen
times and died on every tick before doing anything. "Has it fired?" → eighteen → *exercised*. "Does
a single output row exist?" → zero → *never ran*. Both readings alone are wrong. Report both.

**Element 8 — the negative face, and what it is not.** Not the refusal face, which covers *acts the
system declines*. Not the three-way Then, which is entirely *positive* postconditions. A negative-face
clause is a state that must not obtain **regardless of which act was attempted**:

> *"a read-only member must never create a resource homed in that context"* ·
> *"standing must never be recomputed from its own prior value"* ·
> *"a folded source must never become invisible to the staleness check."*

Across an audited window, nearly every expensive finding was a standing negative that no other
element had a slot for. That is why it is here.

## Clauses are invariants. Witnesses are claims about mechanism.

**This distinction is the most load-bearing thing in this file, and it was paid for.**

| | States | Authored | Governed by |
|---|---|---|---|
| **Clause** | What must be true, or must never be true. **Names no mechanism** | Before the work | The meaning test |
| **Witness** | How we know. **A claim about mechanism** | During or after the build | The demonstrability floor |

**Why the timing is forced rather than preferred.** A witness must **fail against the state its
clause claims to change**. When the mechanism does not yet exist, "fails against current state" is
satisfied by *the absence of the feature* — vacuously, by anything you write. A bite against nothing
is not a bite. So a witness authored in a preamble cannot discriminate, no matter how carefully it
is worded.

**What follows:**

- **Witness decomposition happens inside the build, as a separately authorized act.** Not as a step
  in an authoring pass. The register's shape invites decomposition and you will feel the pull; the
  constraint has to be structural, because an instruction not to decompose was given once and did
  not hold.
- **A clause whose mechanism is unbuilt carries a declared hole, not a filed task.** Write the hole
  down. Do not file placeholder work against it.
- **The proliferation worry dissolves instead of needing a gate.** Witnesses cannot multiply up
  front because they are not authored up front.

**A clause that names a mechanism has committed to a *how*** — the part most likely to mutate — and
it propagates that commitment to every reader downstream. The worst-affected reader is an
implementing agent, which will build the mechanism the clause names rather than the outcome it meant.

> **What this cost once.** A decomposition pass over a register authored against an unbuilt subject
> produced ten witnesses. Most were neither expressible nor implementable — they did not fall out of
> any design, because there was no design to fall out of. Six were later retired; two had already
> been cancelled. The bill was four working sessions on another machine, spent recovering the intent
> the decomposition had buried. That is the drift this discipline exists to prevent, produced by the
> discipline's own instrument.

## Two floors, and they answer different questions

- **Goal → sub-goal is clause decomposition**, before the work. **The meaning test: split a clause
  when its halves can be violated independently.** If two sub-clauses can only ever fail together,
  they are one clause.
- **Sub-goal → task is witness decomposition**, inside the work. **The demonstrability floor**, and
  the only place it can honestly be applied.

The meaning test is deliberately a judgment question. Granularity of *how* is what most resists a
hard boundary, and a rule there manufactures false precision.

## Closure — four states, and equivalence claims are the real risk

For a goal, *situated-actor × act × relevant-state* is a constructible product. Every cell is in one
of four states:

| State | Meaning |
|---|---|
| **Examined-and-specified** | Covered |
| **Examined-and-deliberately-excluded** | Emitter, timestamp, stated reason. **Settled** |
| **Examined-and-inexpressible** | Wanted; the system cannot express it. **A pending fork** |
| **Unexamined** | A visible remainder, not a silent one |

**The third is a peer of the second, not a flag on it.** An exclusion is *settled*; an inexpressible
cell is *waiting*. A reader scanning "excluded, reason given" moves on — which is exactly how the
fork silently fails to be taken.

**Nobody examines cells one by one, and that is where the holes are.** The matrix collapses via
claims like *"all actors lacking grant G are interchangeable for this act."* Most holes live not in
unexamined cells but in **wrong equivalence claims** — cells examined under a class abstraction that
did not hold. So state your equivalence claims explicitly, in a form that can be attacked.

A closure section records: dimensions considered · classes claimed · cells examined per class ·
cells excluded with reasons · remainder marked unexamined.

> **Carried scar — name your axes.** A prior closure claimed a domain was "closed-world in the
> dimensions that matter." Every dimension it had enumerated was **authorization-shaped**. The
> property holds there. It does **not** hold for cadence, rate, or volume — and the subject was
> saturated with those, so the decisive axis was an open one presented as closed.
>
> **State which axes you close over, and name rate-shaped axes as open unless you have explicitly
> enumerated them.**

## When the system cannot express an element

Sometimes the system **as designed cannot express** what an element needs: the actor does not exist
as a modelled thing, the act has no affordance, the projection a Then-clause asserts over is not
built, the refusal named is one the system cannot make.

**Diagnose in this order — miscategorisation first:**

1. **Miscategorised** — the element is not the kind of thing you claimed. Restate it in the right
   element. **No fork.**
2. **Missing affordance** — the element is right; the system cannot do it. → **evolve the system**.
3. **Wrongly reaching** — the element is right; the system *should not* do it. → **change the goal**.

*Worked instance of (1).* A register recorded *"I cannot assess this citation"* as a refusal the
system could not express. It is not a refusal at all — it is a **verdict**, and a strongly negative
one: *"this is incommunicable, or I cannot evaluate where you derived it"* is worse than *"plausible
but underwarranted."* Forking on it would have argued for building a refusal the system correctly
does not have.

**Then there are exactly two honest exits**, and taking either is a judgment for whoever owns the
frame — not for you, and not for the register:

- **Evolve the system** — the affordance is missing and should exist. Produces an **`enables`** task.
- **Change the goal** — via a decision that records the supersession. A visible scar, never a silent
  edit.

**The dishonest third exit is to assume the affordance and write clauses against it.** That is the
deeper account of the four-session failure above: a register met the edge of what the system could
express and, with no fork available to record, hallucinated the affordance.

## Verification modes

| Mode | What it requires |
|---|---|
| **Executable** | A witness **shown to fail** against the state its clause claims to change |
| **Replay-verified** | Run the cascade to quiescence against a test ledger, then assert |
| **Judged** | Satisfied *as judged from a named perspective*, exemplified by two or three cited instances |

**A judged criterion needs its perspective and its exemplars named.** *"Does this look the way
`relationships.rs` looks"* is tractable. *"Does this align with our patterns"* fails silently.

**This file makes no test-framework decision.** The vocabulary a witness reaches for —
`given_entity`, `given_grant`, `emit`, `project_as_of`, `run_to_quiescence` — is a way of thinking
about fixtures, not a dependency claim. Use whatever the project actually has. A framework named in
a spec you read is not thereby a framework the repo uses; check before you write against it.

## Enclosure of responsibility — what a clause may not reach

> Criteria that describe the interiority of a thing are framed within the priors that situate them.
> They cannot reach beyond, or up into, the frame of their priors to mutate them.

A goal's clauses say what must be true and what must never become true **for this goal to have been
achieved well**. They may not say whether pursuing it was right, whether its cost is justified, or
whether it should be abandoned. That judgment belongs to the engineer, the product manager, the
partner weighing cost against churn, or an agent reasoning in an explicit meta-goal frame.

**The discipline detects. It does not decide.**

| Mechanism | May | May not |
|---|---|---|
| Closure | Mark a cell unexamined or inexpressible | Decide the goal is not worth closing |
| The inexpressible intersection | **Surface** the evolve-or-change fork | **Take** it |
| Witnessing | Show a clause uncovered | Conclude the clause should be dropped |
| Exercise status | Report that something never ran | Conclude it should not exist |
| A verification mode | Say a criterion failed | Say the goal was misconceived |

Cost, churn and worth stay worth measuring — record them as a **program-level record**, reported and
not adjudicated. They are simply not clauses.

**This has been violated in practice, so it is worth recognising by shape.** A clause whose job is
to retire the goal containing it is the recognisable form. It was never a constraint anyone set: it
entered as a line in a draft, was carried forward unratified into a named clause, and had a whole
argument built on it before anyone noticed. When you find one, withdraw it and say where it came
from — the provenance matters more than the clause did.

## The loop

1. **Open** — read what is in force: which clauses hold, which are superseded, where the scars are.
2. **Author or resume** — the invariants and the negative face. **No mechanism named.**
3. **Decompose the clauses** — by the meaning test. A clause whose mechanism is unbuilt carries a
   **declared hole**, not a filed task.
4. **Ground** — a grounding pass lands as a `research` resource, not as prose in a session note that
   has to be re-read later.
5. **Build — and author the witnesses here.** The first honest point, and a separately authorized act.
6. **Contradict** — measurement refutes a clause ⇒ a `decision` recording the supersession. A visible
   scar, never a silent edit.
7. **Close** — update exercise status. Every closing note carries **hard follow** / **accepted** /
   **for the record** / **nothing**.

> **Steps 3 and 5 were one step once, and that was the defect.** Collapsing them is precisely what
> lets a preamble pass emit mechanism-shaped tasks for a mechanism nobody has built.

**This is not a seventh `mode × effort` workflow.** Those answer *how much process for how big a
job*. The register answers *what the outcome is*. They compose; neither replaces the other.

## The pre-land check — four questions before a register lands

**The crown-jewel rule — `no-witness-precedes-its-mechanism`, with its siblings `no-clause-names-a-
mechanism` and `no-clause-is-uncovered-silently` — aspires to structural enforcement, and this file is
self-aware that prose alone will not hold it** (*"the constraint has to be structural, because an
instruction not to decompose was given once and did not hold"*). But the one structural hook that
ships is the `witnesses`-vs-`enables` split at **task-create time** — which is *downstream* of
authoring. At the moment that matters most — a drafter writing the register — enforcement is otherwise
pure prose: the author reads the rule and internalizes it. In a fan-out dogfood it held only because
the controller re-read every draft hunting for mechanism-in-clauses and premature witnesses. **That is
diligence, not structure.**

So before a freshly drafted or amended register **lands**, run these four questions against it. They do
not replace the principles above — they are the pre-land gate that turns *"be diligent"* into *"answer
these four,"* the way [`plan-verification.md`](plan-verification.md) gates a plan before dispatch:

1. **Does any clause name a mechanism?** A clause states what must be true; the *how* belongs to the
   build. A `--flag`, a function name, a table, an envelope field appearing in a clause *body* is the
   slip. (→ `no-clause-names-a-mechanism`)
2. **Does the register contain witness definitions at all?** Witnesses are authored *inside the build*.
   A register that already carries `W1`/`W2` has decomposed in the preamble — the exact slip this file
   records happening once. (→ `no-witness-precedes-its-mechanism`)
3. **Is every EXTEND / AMEND-shaped claim spec-cited?** A register that claims to *extend* or *change*
   an existing thing carries the citation that authorizes it, per
   [`implementation-grounding.md`](implementation-grounding.md) GD-3 — otherwise it is invention
   laundered as grounding.
4. **Is every uncovered clause declared uncovered — not silently absent?** Coverage is never inferred
   from absence; a clause with no witness says so, and a retired witness leaves a named remainder. (→
   `no-clause-is-uncovered-silently`)

**This is the shape check, not the meaning check.** It catches a clause that *reads* like mechanism and
a register that *contains* witnesses — most of what has slipped historically — and it cannot judge
whether a mechanism-free clause is the *right* invariant; that judgment stays with the reader. Where a
mechanical form of these checks earns its keep, it belongs in a `temper goal lint`-shaped verb filed to
the code repo — flagging clause bodies with imperative/mechanism language, witness-shaped content in a
goal body, and EXTEND/AMEND without a citation. The doc names the check; the owner decides whether to
mechanize it (enclosure of responsibility again).

{% if surface == "cli" -%}
## Doing it with the CLI you have

The register itself is **the goal's body** — write it as markdown sections. `resource create --type
goal --show-template` prints the section skeleton.

```bash
# 1. Open — what is in force
temper resource list --type goal --context @me/<ctx> --status active --all
temper resource show <goal-ref>            # the register; --without body to skip the prose

# 2. Author or amend. The register is the body, so a body rewrite is the edit.
#    `show` prints a serialized RECORD, not the markdown body — json or toon depending on the
#    format in force. Neither is the body, so extract `content` explicitly; a bare redirect of
#    `show` into a file and back would write the serialized record as the body.
#
#    ERROR PATH: check the exit code first. A non-zero exit means `show` failed — in JSON mode the
#    explanation is a structured payload on stdout (`{ "error": { code, message, hint? } }`), not the
#    record. Parse `.error.code` before `.content`; `jq -r .content` on an error payload yields
#    `null`, which is a silent wrong answer, not a caught one.
temper resource show <goal-ref> --format json | jq -r .content > register.md
cat register.md | temper resource update <goal-ref>

# 5. During the build, a task declares what it is doing for the goal.
temper resource create --type task --title "…" --context @me/<ctx> \
  --mode build --effort small --goal <goal-ref> \
  --open-meta '{"witnesses":{"goal":"<goal-uuid>","clauses":["clause-name"]},
                "witness":{"id":"W1","mode":"executable","clause":"clause-name",
                           "floor":"…","bites_against":"…"}}'
```
{%- else -%}
## Doing it with the tools you have

The register itself is **the goal's body** — write it as markdown sections. There is no
section-skeleton tool on this surface: `describe_doc_type` returns the frontmatter JSON Schema and
an `example_managed_meta`, which is metadata, not an outline. Take the headings from *The eight
elements* above.

```
# 1. Open — what is in force
Tool: list_resources   Input: { "doc_type_name": "goal", "context_ref": "@me/<ctx>",
                                "status": "active", "limit": 200 }
Tool: get_resource     Input: { "id": "<goal uuid>", "include_content": true }   // the register

# 2. Author or amend. The register is the body, so a body rewrite is the edit.
#    `content` REPLACES the body. Send the whole amended register, never a fragment — a partial
#    body is a silent truncation of the goal, not an append.
Tool: update_resource  Input: { "id": "<goal uuid>", "content": "<the amended register>" }

# 5. During the build, a task declares what it is doing for the goal.
Tool: create_resource
Input: {
  "context_ref": "@me/<ctx>",
  "doc_type_name": "task",
  "title": "…",
  "goal": "<goal ref>",
  "managed_meta": { "temper-mode": "build", "temper-effort": "small" },
  "open_meta": {
    "witnesses": { "goal": "<goal uuid>", "clauses": ["clause-name"] },
    "witness": { "id": "W1", "mode": "executable", "clause": "clause-name",
                 "floor": "…", "bites_against": "…" }
  }
}
```
{%- endif %}

**A task declares `witnesses` *or* `enables`, never both.**

- **`witnesses`** — this task **is** the evidence. Subject to the bite requirement, and to the rule
  that no witness precedes its mechanism.
- **`enables`** — this task builds the mechanism that **makes** a clause witnessable. Not evidence,
  not subject to bite, and legitimately filed before any witness exists.

Without that split, the rule forbidding premature witnesses would also forbid the work that makes
witnesses possible. The tell for a miscast one is in its own title: a task called *"make X
enumerable (**unblocks** W2 and W8)"* declared itself a witness while saying, in its own words, that
it enables two others. It witnessed nothing.

**Clause names are readable names, not indices.** `no-witness-precedes-its-mechanism`, not `C3`. An
index carries no information to a reader who does not have the goal open in front of them, and every
downstream reader is that reader.

### Three things that will bite you today

{% if surface == "cli" -%}
- **Goal membership has two spellings and nothing ties them.** `--goal <ref>` projects an `advances`
  **edge**, which is the only thing `resource list --type task --goal <ref>` filters on.
  `open_meta.witnesses.goal` is a **citation**. A task can carry one without the other — one does, and
  a clause migration built from `list --goal` silently missed it. **Pass `--goal` *and* write the
  citation**, and when you enumerate a goal's tasks, do not trust either spelling alone.

  `scripts/register-coverage.py <goal-ref>` reads the citation and compares the two, so you no longer
  have to eyeball it. Note what the divergence usually *is*: not a missing edge, but an edge pointing
  at a **different** goal, because a task may advance one goal while evidencing a clause of another —
  and a resource gets only one `advances`→goal edge.
- **`--open-meta` on `update` is a per-key PATCH, and the key is the unit.** Keys you do not supply
  are untouched, but a key you do supply is **replaced whole** — sending `{"witness":{"mode":"judged"}}`
  drops every other field of `witness`. Send the complete key value.
- **`resource list` is capped and will lie by omission.** Default page is 20 rows, whatever
  sections you ask for. Check `truncated` in the response, and reach for `--all` before you conclude a
  clause has no tasks, or that a set is complete.
{%- else -%}
- **Goal membership has two spellings and nothing ties them.** `create_resource`'s / `update_resource`'s
  `goal` field projects an `advances` **edge**, which is the only thing `list_resources`' `goal` filter
  reads. `open_meta.witnesses.goal` is a **citation**. A task can carry one without the other — one
  does, and a clause migration built from the edge filter silently missed it. **Send `goal` *and* write
  the citation**, and when you enumerate a goal's tasks, do not trust either spelling alone. There is
  no tool that reconciles them, so the comparison is yours to make: list by the `goal` filter, then
  read `open_meta.witnesses.goal` off the same set and diff the two by hand. The divergence is usually
  not a missing edge but an edge pointing at a **different** goal, because a task may advance one goal
  while evidencing a clause of another — and a resource gets only one `advances`→goal edge.
- **`open_meta` on a write is a per-key PATCH, and the key is the unit.** Keys you do not supply are
  untouched, but a key you do supply is **replaced whole** — sending
  `{"witness":{"mode":"judged"}}` drops every other field of `witness`. Send the complete key value.
  (`update_resource`'s `open_meta_add` is the narrow exception: it unions **array**-valued keys rather
  than replacing them. It cannot express a partial object update, so it does not help here.)
- **`list_resources` is capped and will lie by omission.** Default page is 50 rows, `limit` maxes at
  200. Compare `rows.len()` against the response's `total` before you conclude a clause has no tasks,
  or that a set is complete, and page with `offset` when it is short.
{%- endif %}

### Declaring coverage without a query

There is no coverage query. Until there is, the register carries its own **declared coverage state**
— a table of clause → witnesses → covered / declared-uncovered, updated when you close. This is not
a workaround for a missing tool; it is the discipline's own rule applied to itself, because
**coverage is never inferred from absence**. A clause with no witness declares it and says why. A
retired witness leaves a named remainder rather than a gap that reads as clean.

Retiring a witness is fine and often correct. What must never happen is the **clause** it pointed at
silently reading as covered.
