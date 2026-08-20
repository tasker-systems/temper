# Bootstrap a Team's Self-Cognition

**For operators.** This playbook births and binds a **team self-cognition
cognitive map**: a cognitive map born 1:1 with a team, whose ingest source is
the team's own Temper resources, and whose telos is "understand how this team
works." This is the foundation the team's steward tends going forward — this
playbook covers birth and bind, not ongoing stewardship.

## Outcome

By the end you will have a team, its working context, a self-cognition
cognitive map born with the templated telos charter, and the map bound to the
team so it reaches the team's shared corpus. Every step is an idempotent
`temper` command (with one exception noted in step 2).

| Outcome | Produced by |
|---------|-------------|
| The team (you become its owner) | `temper team create <team-slug> --name "<Team>"` |
| The team's working context — the ingest source | `temper context create <ctx> --owner +<team-slug>` |
| A self-cognition cognitive map, born with the templated telos charter | `temper cogmap create --manifest team-self-cognition.yaml --name "<Team> — self-cognition"` |
| The map reaching the team's shared corpus | `temper cogmap bind <cogmap-ref> <team-slug>` |

## Prerequisites

- **A usable instance**, and the rights for the two gated steps. `cogmap
  create` is open to any authenticated profile — the creator is granted
  read+write+grant on the map they make. `cogmap bind` needs system-admin
  **or** that you manage the target team (owner/maintainer) *and* administer
  the map. `cogmap reconcile` is admin-only. See
  [self-hosting Temper](./self-host-temper.md) if this is a fresh install, or
  [bootstrap an org](./bootstrap-an-org.md) for the full org setup sequence.
- **An `embed`-capable `temper` binary.** `cogmap create` embeds the charter
  client-side (ONNX). A non-`embed` build returns a clear `requires the
  'embed' feature` error rather than running.
- **Authentication.** You must be logged in (`temper auth login`, or
  `TEMPER_TOKEN` exported) as a system admin before step 3/4 below (steps 1/2
  only need an authenticated profile, not admin).
- **The genesis manifest.** Save the following to a file named
  `team-self-cognition.yaml`:

```yaml
# Team self-cognition cognitive map — genesis manifest (reusable template).
#
# Consumed by:  temper cogmap create --manifest team-self-cognition.yaml \
#                 --name "<Team> — self-cognition"
# Then bound 1:1 to the team:  temper cogmap bind <minted-cogmap-ref> <team>
#
# The prose is deliberately team-agnostic ("this team") so the single artifact
# serves every team without interpolation — the team's identity rides on the
# map's --name at apply time.
#
# IDENTITY: omit cogmap_id / telos_resource_id and the CLI mints stable uuidv7s
# and prints them. Pin them once you want a reproducible, re-runnable genesis —
# a re-run at the same id is an idempotent no-op (created: false).

name: "Team — self-cognition"
telos_title: "How this team works"
telos:
  statement: >-
    Understand how this team works — what it is actively working on, the problems it solves and for
    whom, the domains it owns, the decisions it has settled and the commitments it holds, and the
    concerns and open questions it carries. This map is the team's self-cognition, dogfed from the
    team's own temper resources: its nodes are distilled from those resources and situated by this
    telos. Salience is judged under this purpose — never universally.
  questions:
    - question: "What is this team actively working on?"
      context: "Surfaces the live themes and the most active threads — the team's current front."
    - question: "What problems does this team solve, and for whom?"
      context: "The team's reason-for-being, distilled from its work rather than declared abstractly."
    - question: "What does this team know — its domains of expertise and responsibility?"
      context: "The areas the team owns; where its judgment is authoritative."
    - question: "What has this team decided, and what has it committed to?"
      context: "Settled decisions and outstanding commitments — the load-bearing choices to honor."
    - question: "What concerns or open questions is the team holding?"
      context: "Live tensions and unresolved questions worth tracking before they are settled."
  framing:
    - "Nodes are distilled from the team's own resources and carry a `derived_from` edge to their source(s)."
    - "The steward tends declared structure (create / assert / facet / fold); regions emerge from `materialize` — the steward never clusters."
    - "Node labels are expressive: concept, fact, memory, question, theme, concern, principle, commitment, domain."
```

## The sequence

Placeholders: `<team-slug>` (globally-unique team slug), `<Team>` (display
name), `<ctx>` (the team's working context name, e.g. `building`).

### 1. Create the team

```bash
temper team create <team-slug> --name "<Team>"
```

You become the team's owner. Idempotent by slug — re-running against an
existing slug is a no-op (no duplicate team, no error escalation beyond the
existing-slug case).

### 2. Create the team's working context

This is the **ingest source** the self-cognition map eventually feeds from —
resources written here are what the steward distills nodes from. See
[contexts and refs](../concepts/contexts-and-refs.md) for context addressing.

```bash
temper context create <ctx> --owner +<team-slug>
```

`--owner +<team-slug>` marks this a team-owned context (requires
owner/maintainer on the team, which step 1 already granted you). Omitting
`--owner` defaults to a personal `@me`-owned context, which is **not** what a
team self-cognition map wants — always pass `--owner` here.

> **`context create` is not idempotent.** Re-running it with the same name and
> owner creates a second context with an auto-suffixed name (e.g.
> `building-2`), not a no-op. Check whether the context already exists
> (`temper context list`) before re-running this step.

### 3. Birth the self-cognition cognitive map

Genesis births a new map with its telos charter from the reusable,
team-agnostic genesis manifest — no per-team edits to the manifest are needed,
only the `--name` override:

```bash
temper cogmap create --manifest team-self-cognition.yaml \
  --name "<Team> — self-cognition"
```

The output reports the realized identity:

```json
{ "cogmap_id": "019f…", "telos_resource_id": "019f…", "created": true }
```

**Capture `cogmap_id`** — step 4 needs it. Genesis is idempotent at a given
id: pin `cogmap_id` (and optionally `telos_resource_id`) and pass `--id
<cogmap-ref>`, and a re-run is a no-op (`created: false`). Without a pinned id
the CLI mints a fresh uuidv7 each run. For a one-off team this playbook is
typically run by hand and the printed id is captured directly into step 4; pin
it only when you want a reproducible, re-runnable genesis (e.g. driving this
from a script).

### 4. Bind the map to the team

Binding widens the map's reach to the team's shared resources (an unbound map
reaches nothing through the team — empty join, default-closed):

```bash
temper cogmap bind <captured-cogmap-ref> <team-slug>
```

This is the **1:1 team↔cogmap join**. Idempotent on the join primary key —
re-running is a no-op.

The team's self-cognition is now live: the map is born with the templated "how
this team works" charter, and it reaches resources written into
`+<team-slug>/<ctx>`. From here the steward tends it (create / assert / facet /
fold acts against the team's own resources); regions emerge from
`materialize` — the steward never clusters directly. For more on how a map
grows over time, see
[how a map grows](https://temperkb.io/cognitive-maps/how-a-map-grows).

## Verification

```bash
temper cogmap shape <captured-cogmap-ref>
```

Shows the map (initially with no materialized regions — it was just born,
nothing has been asserted/folded/materialized into it yet). The map's telos
carries the templated statement, five questions, and three framing lines
verbatim from the manifest. Today the charter prose itself is not surfaced by
`cogmap shape` (a regions/analytics view, not a charter read) — confirm the
charter landed correctly by inspecting the `telos_resource_id` resource (e.g.
`temper resource show <telos_resource_id>`).

## Idempotency

Idempotency is **inherited from the primitives**: `team create` is idempotent
by slug, `cogmap create` is idempotent at a given id (pinned or captured), and
`cogmap bind` is idempotent on the join PK. **`context create` is the
exception** — it is not idempotent: re-running with the same name and owner
creates a duplicate context (auto-suffixed to `-2`), not a no-op. Check for
an existing context (`temper context list`) before re-running step 2; steps
1, 3, and 4 converge on re-run.
