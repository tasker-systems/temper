# Working a Goal

Read this when **a goal is being worked**: you were pointed at one, it surfaced in standing state,
or you are deciding what it needs next. It is the arc — **uptake → resume → advance → record →
decompose → close** — and defers every register rule to `outcome-registers.md`.

## 1. Uptake — read what is in force

The register (clauses, negative face, declared holes, current witnessed state) is the goal's
in-force state. A declared hole is information, not an omission to fix. Follow the link to the
evidence record when you need the proof, not to learn where the goal stands.
{%- if surface == "cli" %}

```bash
temper warmup --context @me/<ctx>              # standing state: active goals, in-progress tasks
temper resource show <goal-ref>                # the register
temper resource list --type task --goal <goal-ref> --all
temper resource list --type session --context @me/<ctx> --fields ref,title,updated
```
{%- else %}

```
Tool: list_resources   Input: { "doc_type_name": "goal", "context_ref": "@me/<ctx>", "status": "active" }
Tool: get_resource     Input: { "id": "<goal uuid>", "include_content": true }
Tool: list_resources   Input: { "doc_type_name": "task", "context_ref": "@me/<ctx>", "goal": "<goal uuid>" }
Tool: list_resources   Input: { "doc_type_name": "session", "context_ref": "@me/<ctx>" }
```
{%- endif %}

**Reconcile the linkage.** The `advances` edge is asserted at create and nothing backfills it, so
the goal filter alone can miss live work. Cross-check recent sessions and task titles, and link any
unlinked goal task you find now. To walk the goal's neighborhood (tasks, decisions, child goals)
by graph instead of by filter:
{%- if surface == "cli" %}

```bash
temper graph traverse --from <goal-ref> --depth 2
```
{%- else %}
use `run_query`: its `follow-from` act walks outward from seed refs, and the
`bound` relation keeps a walk inside a set you already found. For a one-off answer, hop by
`get_resource` on refs you hold, and say so if the picture needs more than that.
{%- endif %}

## 2. Resume — pick the next chunk

An in-flight task resumes where its latest session note's *Next Steps* says. With none in flight,
take the next chunk from the register's declared holes or the roadmap and create its task linked to
the goal. If the task will author or amend the register, or declare `witnesses` or `enables`, read
`outcome-registers.md` first.

**Taking up a chunk is when enabling work gets bounded.** Name the clause it will be judged
against. Enabling work no clause can judge is a missing clause: author it now as a child or sibling
goal (§5), not up front.

## 3. Advance — task over task

Work the task and move its stage as it progresses. When work moves a register clause, record the
evidence in the goal's evidence record, then rewrite only the goal's current-state rows. The session
note carries the account of the movement; the goal gains no dated line.

## 4. Record — session over session

Every session saves its note and attaches it to the task it served (`session-lifecycle.md`). The
notes make the next uptake cheap; a session that moves the goal without recording it costs the arc
its continuity.

## 5. Decompose — goal over goal

A sub-goal is **clause decomposition by the meaning test**: split a clause when its halves can be
violated independently. It is also the response, at uptake, to enabling work no clause can judge:
the child states the outcome that work makes true, as concretely as its cited decisions settle. A
task under the parent authors the child, and the child links back to the parent:
{%- if surface == "cli" %}

```bash
temper resource create --type goal --title "<sub-goal title>" --context @me/<ctx>
temper edge assert <child-goal-ref> <parent-goal-ref> --kind leads-to --polarity inverse --label derived_from
```
{%- else %}

```
Tool: create_resource  Input: { "context_ref": "@me/<ctx>", "doc_type_name": "goal", "title": "<sub-goal title>", "content": "…" }
Tool: relationship     Input: { "action": "assert", "source": "<child goal uuid>", "target": "<parent goal uuid>",
                                "edge_kind": "leads_to", "label": "derived_from", "polarity": "inverse",
                                "weight": 1.0 }
```
{%- endif %}

Siblings are children of one parent, whose register says which clauses each carries. Once a parent
has two or more children, run the *separability check* in `outcome-registers.md` before they land,
and again when a child is added or its clauses move.

## 6. Close — the goal ends on purpose

A goal closes by decision, never by drifting into silence:
{%- if surface == "cli" %}

```bash
temper resource update <goal-ref> --status completed   # or paused / cancelled
```
{%- else %}

```
Tool: update_resource_meta   Input: { "id": "<goal uuid>", "managed_meta": { "temper-status": "completed" }, "open_meta": {} }
```
{%- endif %}

Record any last evidence in the evidence record, then update exercise status and the current
witnessed state. The closing note carries **hard follow** / **accepted** / **for the record** /
**nothing**. A declared hole that outlives the goal is named in the close, never dropped.

## Traps

- **Trusting the goal filter alone** — the edge exists only if asserted at create; link every task
  then, and cross-check sessions at uptake.
- **Filing a placeholder task against a declared hole** — the hole waits until someone takes it up.
- **Pre-authoring sub-goals to house work nobody has taken up** — enabling goals are authored at
  uptake; splitting clauses the parent already states is decomposition and may come earlier.
- **Writing progress onto the goal** — history goes in the evidence record and session notes.
