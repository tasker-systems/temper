# Cognitive Maps

Read this when a session needs to **read from or author into a cognitive map** — not
the everyday task/session/resource CRUD the rest of the skill covers. Maps are a
by-design capability of temper; this file makes them usable without rediscovering the
model from scratch.

The exhaustive authoring mechanics (label vocabulary, edge kind/polarity/weight
conventions, the "materially changed" judgment, the steward loop) live in the steward's
**`map-stewardship`** skill — `packages/agent-workflows/steward/agent/skills/map-stewardship.md`
in the temper repo. This file teaches the *model* and the human+agent flow; reach for
`map-stewardship` for the per-call detail.

## Map vs context — the duality (the crux)

A **context** homes resources **as they are**: your tasks, sessions, research — a working
set you produce and consume directly. `resource create --context …` writes *into* one.

A **cognitive map** homes **distilled nodes** in a **telos-governed graph**: nodes, edges,
facets, and emergent regions, all shaped by the map's charter (its telos). A map is not a
folder of your documents — it is a purpose-built *understanding* of some body of material.

The crux an agent misses: **a map node is not the same row as its source.** It is a *new*
resource, created into the map, that **distills** from one or more sources and carries a
`derived_from` edge + a `sources` provenance list back to them.

- `kb_resource_homes.resource_id` is **UNIQUE** — a resource has exactly **one** home.
  `resource create` is `--context` **XOR** `--cogmap`; you cannot re-home a context
  resource into a map.
- So authoring a node means: read the source(s) → decide what the *telos* makes worth
  saying → write a fresh node into the map with `--sources` pointing back. Nodes are
  **distilled, never re-homed.** (An earlier framing called them "often the same rows" —
  that is wrong in practice; this is the correction.)

| | Context | Cognitive map |
|---|---------|---------------|
| Homes | resources as-is | distilled nodes (new resources) |
| Organizing principle | a working set | a telos (charter) + graph |
| Structure | flat list | nodes · edges · facets · regions |
| A "row" is | the thing itself | a distillation of source rows |
| Authored via | `resource create/update` | the authored-4 under an invocation |
| Read via | `list` / `show` / `search --context` | `cogmap shape` / `search --cogmap` / `--wayfind` |

## When to reach for a map vs a context

- **Context** — you are *doing* work: producing and tracking tasks, sessions, research.
- **Cognitive map** — you want a *purpose-shaped distillation* of material: an understanding
  a telos decides the shape of. The same sources under a *different* telos yield a
  *different* map — this is demonstrated, not asserted (two teloi over the same 8 docs
  produced two distinct, each-telos-coherent node sets; see the telos-differentiation
  findings). Reach for a map to **orient across a distilled understanding**, not to store
  working artifacts.

## The telos-charter — read it first

Every map has a **telos-charter**: its purpose, expressed as a statement + open questions +
framing. The charter is the **rubric for what is worth distilling** — a node earns its
place only if it serves the telos.

- Read it **before authoring** — it decides what counts as a node. Agent surface:
  the `cogmap_read_charter` MCP tool. From the CLI, `temper cogmap analytics <cogmap>`
  surfaces telos/staleness/regulation.
- Distill what is **salient under this charter**, not everything true about the source.
  A `character-modeling` doc read under *"design a narrative system"* yields "what the
  character model is"; read under *"how would temper's maps manage that information"* it
  yields "where temper's single-weight edges strain against a relational web." Same
  source, telos-attributable difference.

## Authoring into a map

### The shape — the authored-4 under an invocation

Open one **invocation envelope** per authoring pass; every authored act carries its id.

```bash
inv=$(temper invocation open --cogmap <MAP> --trigger-kind manual)   # server mints the id
# ... read the charter, then for each source you distill:

# 1. create the node INTO the map, citing its source(s)
temper resource create --cogmap <MAP> --type concept --title "<ascii title>" \
    --sources <SOURCE_REF>[,<SOURCE_REF2>] \
    --invocation <INV> --confidence confident \
    --reasoning "why this node under this telos" --model "<your-model>" \
    --body @node-body.md

# 2. assert its provenance edge back to the source (+ any inter-node edges)
temper edge assert <NODE_REF> <SOURCE_REF> --kind express --polarity forward \
    --label derived_from --weight 1.0 \
    --invocation <INV> --confidence confident --reasoning "distills <SOURCE_REF>"

# 3. close the invocation
temper invocation close <INV> --disposition completed \
    --outcome '{"nodes":N,"edges":E}'
```

The **authored-4** are `create_resource` · `assert_relationship` · `facet_set` ·
`fold_relationship`. On the CLI these are `resource create --cogmap`, `edge assert`, and
`edge fold`; **`facet_set` is agent-surface only** (the `facet_set` MCP tool) — use it for a
node's *semantic* properties (a resolved question, a stance), never for provenance.
**Materialize** (recompute regions) is likewise agent-surface: `cogmap_materialize` /
`cogmap_materialize_delta`. Regions only exist *after* a materialize.

### The act envelope — a hard invariant

Every authored act carries `--invocation` + `--confidence` + `--reasoning` (and `--model`).
An act missing them is real but **orphaned** — it will not appear under
`invocation show`, breaking the accountability chain. No exceptions, not just `create`.

### The confidence-band rubric

`--confidence` is required on every act. Bands:

- **`confident`** — an explicit, dated decision or a direct claim the source states outright.
- **`probable`** — a synthesis you drew across sources that they don't state verbatim.
- **`tentative`** — thin or uncertain evidence; noted, not leaned on.

(The same three bands appear in `map-stewardship`; this rubric is the missing "which band
when" a charter-only author otherwise has to invent.)

### Distillation, dedup & merge

Before creating a node, **search the map for an existing one**:
`temper search "<concept>" --cogmap <MAP>`. If a node already covers it, don't duplicate.

When **two sources both assert one concept**, distill **one** node that cites **both** in
`--sources` (and one `derived_from` edge per source) — not two near-duplicate nodes. Match
the source count to what the node honestly distills, not to its label (a `decision`
synthesized from two sources is fine).

### Supersession — fold, then recreate

**Never edit a node's body in place.** When a source *materially* changes — the distillation
would now say something different (a new claim, a reversed decision), not a typo fix — **fold**
the stale node's `derived_from` edge and **create a fresh node**:

```bash
temper edge fold <CORRELATION_ID> --reason "source materially changed" \
    --invocation <INV> --confidence probable --reasoning "folding stale derived_from"
```

When in doubt, prefer leaving the node and lowering confidence over churning a fold.

### Provenance stamping

Every node carries the provenance trio in **`managed_meta`** (the typed home), not an
ad-hoc `open_meta` blob:

- `temper-provenance: "llm-discovered"`
- `temper-llm-model: "<your model>"`
- `temper-llm-run: "<the invocation id>"` — joins the node back to the run that authored it.

### Two authoring gotchas

- **Use ASCII characters in node titles.** A non-ASCII title char once broke slug
  generation and failed the create (bug B2, fixed in PR #287). The ASCII habit costs
  nothing and stays safe regardless of which build a given map's server is running.
- **Provenance belongs in `managed_meta`.** `temper-llm-model` once landed in `open_meta`
  (bug B1, fixed in #287). Stamp the trio into managed_meta and verify on read-back.

## Who may author — the access reality

Reading a map is **not** authoring it.

- **Read** — a team joined to a map can read the resources homed in it.
- **Author** — needs an **explicit write grant** on the map (`temper cogmap grant … --write`),
  **not** mere team membership.
- **Modify an existing node** — needs modify-access **to that node**. Whether a
  container-level (map) write grant should *confer* node-level write is an **open
  precedence decision** — task `019f3739` ("should container `can_write` confer node-level
  `can_write`?"). The full human+agent re-distill flow's viability tracks that decision, so
  don't assume a map-write grant lets you fold/supersede a node you don't own until it lands.

## Cross-map — wayfind, not edges

You have several visible maps. To draw on more than one:

- **Cross-map EDGES are inert at the region layer.** You *can* assert an edge from a node in
  one map to a node in another, and materialize accepts it — but it contributes nothing to
  the target map's regions. Don't reach for cross-map edges to make two maps "talk."
- **Cross-map VALUE lives in wayfind.** `temper search "<query>" --wayfind --regions 20`
  pools regions across *all* your visible maps, ranked by query relevance + each region's
  own-telos salience — surfacing, e.g., one map's "Narrative Gravity" beside another's
  "narrative gravity as a runtime-recomputed field" (same concept, two teloi). The
  single-map **lens** (`--cogmap` / `cogmap shape`) is single-map by construction; **wayfind**
  is the cross-map mechanism.
  - Bump `--regions` (default is deliberately narrow) when pooling across several maps, or
    the top-N will not reach past the nearest one.
- **Cross-map linking is a capability, not an instinct.** An agent won't search neighboring
  maps unprompted. If you *want* a node to reference a concept already present in a visible
  neighbor rather than re-distilling it, **say so explicitly** — when directed, it's high
  value (one pass asserted 16 quality cross-links and rejected 5 loose ones).

## Worked example — a human+agent re-distill

A source that a map already has a node for gets materially updated; the human asks the agent
to bring the map's node current. End to end:

```bash
# 1. Orient: read the charter so you distill on-telos.
#    (cogmap_read_charter MCP tool, or:)
temper cogmap analytics <MAP>

# 2. Find the existing node and its derived_from edge.
temper search "relational character web" --cogmap <MAP>
temper resource show <NODE_REF> --edges          # note the derived_from correlation id

# 3. Open the accountability envelope.
inv=$(temper invocation open --cogmap <MAP> --trigger-kind manual)

# 4. Re-read the (changed) source and judge: materially changed? If yes —
temper edge fold <DERIVED_FROM_CORRELATION_ID> --reason "source materially changed" \
    --invocation "$inv" --confidence probable --reasoning "folding stale derived_from"

# 5. Create the FRESH node (never edit in place), citing the source, stamping provenance.
temper resource create --cogmap <MAP> --type concept \
    --title "Relational web edges exceed temper's single-weight edge" \
    --sources <SOURCE_REF> \
    --invocation "$inv" --confidence confident \
    --reasoning "distills the updated character-modeling doc under the map's telos" \
    --model "<your-model>" --body @new-node.md

# 6. Re-assert provenance + any inter-node edges, with graded weights.
temper edge assert <NEW_NODE_REF> <SOURCE_REF> --kind express --polarity forward \
    --label derived_from --weight 1.0 \
    --invocation "$inv" --confidence confident --reasoning "distills <SOURCE_REF>"

# 7. Close, then materialize so regions pick up the change.
temper invocation close "$inv" --disposition completed --outcome '{"nodes":1,"edges":1,"folds":1}'
#    then cogmap_materialize (MCP) on <MAP>
```

The result: one closed invocation correlating every act, a fresh on-telos node with clean
provenance, the stale node superseded (not mutated), and regions recomputed. That is the
human+agent authoring loop — the same shape the steward runs on a schedule, done once by
hand under a human's direction.
