# Working a Goal

Read this when **a goal is being worked** — you were pointed at one, it surfaced in standing
state, or you are deciding what a goal needs next. This file is the *arc*, not a reference: each
phase points at the discipline that owns its detail, and nothing here restates one.

The arc: **uptake → resume → advance → record → decompose → close.** A goal's work advances task
over task and is recorded session over session; decomposition produces sub-goals; closing is a
decision recorded on the goal, never a drift into silence.

## 1. Uptake — take the goal up

Read what is in force before doing anything. The register — clauses, negative face, declared
holes, coverage state — is the goal's in-force state, and a declared hole is information, not an
omission to fix:
{%- if surface == "cli" %}

```bash
temper warmup --context @me/<ctx>              # standing state: active goals, in-progress tasks
temper resource show <goal-ref>                # the register
```
{%- else %}

```
Tool: list_resources   Input: { "doc_type_name": "goal", "context_ref": "@me/<ctx>", "status": "active" }
Tool: get_resource     Input: { "id": "<goal uuid>" }
```
{%- endif %}

Then establish where the arc stands. List the tasks advancing the goal:
{%- if surface == "cli" %}

```bash
temper resource list --type task --goal <goal-ref> --all
temper resource list --type session --context @me/<ctx> --fields ref,title,updated
```
{%- else %}

```
Tool: list_resources   Input: { "doc_type_name": "task", "context_ref": "@me/<ctx>", "goal": "<goal uuid>" }
Tool: list_resources   Input: { "doc_type_name": "session", "context_ref": "@me/<ctx>" }
```
{%- endif %}

**Reconcile the linkage.** A task can advance a goal without carrying the edge — the edge is
asserted at create and nothing backfills it, so the goal filter alone can miss live work.
Cross-check recent sessions and task titles for goal work the filter does not show, and link
every task you create to the goal at create time so the next uptake sees it. To walk the goal's
whole neighborhood — tasks, decisions, child goals — by graph instead of by filter:
{%- if surface == "cli" %}

```bash
temper graph traverse --from <goal-ref> --depth 2
```
{%- else %}

The MCP surface has no traverse tool; hop by `get_resource` on the refs you already hold, and
say so if the picture needs more than that.
{%- endif %}

## 2. Resume — return to the arc

An in-flight task resumes by the task-resume path in `SKILL.md` — the most recent session note's
"Next Steps" is where you pick up. With no in-flight task, choose the next chunk from the
register's declared holes or the roadmap, and create that task linked to the goal. If the task
will author or amend the register, read `outcome-registers.md` first; witness and `enables`
declarations belong to it too.

## 3. Advance — task over task

Work the task per its `mode × effort` workflow (`workflows/{mode}-{effort}.md`); stages move as
it progresses. When work moves a register criterion, amend the register — `outcome-registers.md`
governs the amendment, and the session note closes on the movement. The advances edge is asserted
at create time; a task found unlinked at uptake gets linked then, not left for the next session
to miss again.

## 4. Record — session over session

Every session saves its note and attaches it to the task it served — the patterns and the
handoff preamble live in `session-lifecycle.md` and `session-wrap.md`. The notes are what make
the next uptake cheap; a session that moves the goal without recording it costs the arc its
continuity.

## 5. Decompose — goal over goal

A sub-goal is **clause decomposition by the meaning test** — split a clause when its halves can
be violated independently; read `outcome-registers.md` before authoring it. The practiced shape:
a task under the parent goal authors the child goal, and the child links back to its parent:
{%- if surface == "cli" %}

```bash
temper resource create --type goal --title "<sub-goal title>" --context @me/<ctx>
temper edge assert <child-goal-ref> <parent-goal-ref> --kind leads-to --polarity forward --label derived_from
```
{%- else %}

```
Tool: create_resource  Input: { "context_ref": "@me/<ctx>", "doc_type_name": "goal", "title": "<sub-goal title>", "content": "…" }
Tool: relationships    Input: { "source": "<child goal uuid>", "target": "<parent goal uuid>",
                                "action": "assert", "edge_kind": "leads_to", "label": "derived_from",
                                "polarity": "forward" }
```
{%- endif %}

Sibling goals are children of the same parent; the parent's register says which clauses each
one carries.

## 6. Close — the goal ends on purpose

A goal closes by decision, with its status set and its register's loose ends named:
{%- if surface == "cli" %}

```bash
temper resource update <goal-ref> --status completed   # or paused / cancelled
```
{%- else %}

```
Tool: update_resource_meta   Input: { "id": "<goal uuid>", "managed_meta": { "temper-status": "completed" }, "open_meta": {} }
```
{%- endif %}

The closing note carries one of: **hard follow** / **accepted** / **for the record** /
**nothing**. Update the register's exercise status and declared coverage state — a declared hole
that outlives the goal is named in the close, never dropped. The full closing discipline is
*The loop*, step 7, in `outcome-registers.md`.
