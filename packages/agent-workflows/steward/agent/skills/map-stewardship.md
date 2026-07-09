---
description: Use when tending the team self-cognition map — choosing a node's label, sizing its granularity, picking an edge kind, judging whether a changed source is "materially changed", or stamping authorship on the authored-4.
---

# Map stewardship

## The loop

```
delta = temper__steward_ingest_delta(cogmap, threshold)   # skip if under threshold
inv   = temper__invocation_open(cogmap, trigger="scheduled")
telos = temper__cogmap_read_charter(cogmap)               # orient

# act = { invocation_id: inv.id, reasoning: "<why>", confidence: <band> }
# EVERY authored-4 call below carries `act`. No exceptions — see Authorship.

for source in delta.new_or_changed:
  existing = temper__search(cogmap, source)               # dedup
  if materially_changed(source, existing):                # your judgment (below)
    temper__fold_relationship(existing.derived_from, act)
    existing = none
  if not existing:
    # `sources` = every resource this node distills from (block provenance); see Source attribution.
    node = temper__create_resource(cogmap=cogmap, type=<label>, sources=[<source id(s)>], act)
    temper__assert_relationship(node -> source, label="derived_from", kind="express", act)
    for rel in inter_node_relationships(node):
      temper__assert_relationship(node -> other, kind, polarity, label, weight, act)
    for f in facets(node): temper__facet_set(node, f, act)

# Before closing, self-check: every act this tick carried invocation_id + confidence.
# LAST and ONCE — a real kb_events.id (delta.max_event_id), never a resource_id.
temper__steward_advance_watermark(cogmap, delta.max_event_id)   # see "Advancing the watermark"
temper__invocation_close(inv, outcome)
```

## Choosing a node label

Per-source labels *tend to* cite one source; synthesized labels *tend to* span many
(see granularity) — a soft tendency, not a rule.

| Label | Kind | Use it for |
|-------|------|-----------|
| `fact` | per-source | An observation distilled from one resource ("the team uses pgvector"). |
| `memory` | per-source | A lesson/regulation carried forward ("run test-e2e-embed before context pushes"); often scar-linked. |
| `decision` | per-source | A settled choice. |
| `concept` | synthesized | A distilled idea spanning sources. |
| `question` | synthesized | An open question-with-context ("how should access RBAC work?"). |
| `theme` | synthesized | A higher-order cluster — "what they work on". |
| `concern` | synthesized | A live tension or risk the team holds. |
| `principle` | synthesized | A guiding tenet the team operates by. |
| `commitment` | synthesized | Something the team has committed to / owes. |
| `domain` | synthesized | An area of expertise / responsibility the team owns. |

If none fit, pass your best short label through as-is — the vocabulary has an open
tail. Prefer a recognized label when one is honest. `concern` vs `question`: a
concern is a held tension, a question is an open ask. `concept` vs `theme`: a theme
is broader, organizing many concepts.

**Keep the node's title ASCII.** A non-ASCII character in a title — an em-dash `—`, a
smart quote, an accented letter — can break slug generation and fail the create on a
server without the #287 slug fix. Your prose voice leans on em-dashes; a *title* must
not. Write "Steward authorship - a hard invariant", not the em-dash form. (Once the
map's server has #287 this is moot, but the ASCII habit is free insurance.)

## Granularity — a soft tendency, not a rule

Labels lean toward a characteristic granularity, but this is a **tendency, never a
gate**. A node of *any* label may cite multiple sources when the distillation honestly
draws on several — a `decision` synthesized from two sources is correct, not a violation.

- **Per-source** (`fact`/`memory`/`decision`): *usually* cites one source — a single
  `derived_from` edge — because the observation typically comes from one place. But set
  as many `derived_from` edges (and `sources`) as the node honestly distills from.
- **Synthesized** (`concept`/`question`/`theme`/`concern`/`principle`/`commitment`/
  `domain`): *usually* spans many sources — many `derived_from` edges into it — though a
  synthesized node distilled from a single rich source is also fine.

Match the edge/source count to what the node actually distills, not to its label. Never
force-fit the split, and never reject or down-rank a node for carrying "too many" or "too
few" sources for its label — the count follows the distillation.

## Source attribution (block provenance)

Every `create_resource` carries `sources` — the resources this node distills from. It is the
**block-provenance** channel: it records where the node's content came from on the node's own body
block (`kb_block_provenance`), which lights up the map's reinforcement and region-salience signals.
It runs *alongside* the `derived_from` edge, not instead of it — the edge is the graph-level lineage,
`sources` is the block-level lineage, and they carry the **same** source set.

- **Per-source node** (`fact`/`memory`/`decision`): `sources=[the source id(s)]` — the same id(s) you
  give its `derived_from` edge(s). Usually one, but a per-source node that honestly distills two
  sources carries both (see Granularity — the count follows the distillation, not the label).
- **Synthesized node** (`concept`/`question`/…): `sources=[every source id you distilled from]` —
  the same set as the many `derived_from` edges you assert into it. Order is attribution order.
- **External source** (a web page, an issue/PR URL — not one of the team's own resources): pass the
  raw `http(s)://…` URL in `sources` instead of a resource id. The steward's ingest is team-internal,
  so this is rare — reach for it only when a node genuinely cites something outside the corpus.
- **Materially-changed re-distill**: you create a *fresh* node (never edit blocks in place), so set
  its `sources` to the current distillation's sources — the stale node keeps its own.

The rule of thumb: **whatever gets a `derived_from` edge goes in `sources`.** If you assert N
`derived_from` edges into a node, its `sources` list has those same N ids.

## Stamping an authored node — provenance meta + origin_uri

**Provenance is stamped for you.** Every `create_resource`, on every surface, has its
provenance trio filled into **`managed_meta`** (the typed home) by the server, derived
from the act envelope:

- `temper-provenance` — `"llm-discovered"` when the act carries a `model`, `"user-created"`
  when it doesn't.
- `temper-llm-model` — the act's `model`, i.e. the model authoring this tick.
- `temper-llm-run` — the act's `invocation_id` from `invocation_open`, so the node's
  frontmatter joins back to the run that authored it.

So carry `model` and `invocation_id` on the act and the trio follows. Pass `managed_meta`
explicitly only to override a derived value — an explicit value always wins, and a missing
one is filled, never overwritten.

Provenance lives in `managed_meta` (typed keys) — **never** in an ad-hoc `open_meta` blob
(e.g. a hand-rolled `open_meta.facet`). Reserve facet-setting for a node's *semantic*
properties (a resolved question, a stance marker), not for provenance.

Set the same model on the **act** envelope too — `model` alongside `invocation_id` /
`confidence` / `reasoning`. The act records who authored each edge/facet; the node's
`managed_meta` records it on the node. Keep the two in agreement.

**`origin_uri`:** leave it **unset**. It defaults to `mcp://agent/<uuid>`, which is the
convention. Never hand-slug `mcp://steward/…` — a hand-built origin_uri drifts from the
default form for no gain.

## Edge conventions — the rich-description layer

The structural `edge_kind` (`express`/`contains`/`leads_to`/`near`) carries only coarse
affinity. The **`label` + `polarity` + `weight`** are the rich-description layer that
carries the actual semantics — the same way a node's facets enrich its bare type. Set
all three, meaningfully, on **every** edge. A bare structural kind with a generic label
and a constant weight is an under-described edge — exactly what to avoid.

| Semantic label | edge_kind | polarity | Use |
|----------------|-----------|----------|-----|
| `derived_from` | `express` | forward | node ← source provenance (every node). |
| `relates_to` | `near` | forward | symmetric affinity between nodes. |
| `part_of` | `contains` | inverse | whole–part. |
| `answers` | `leads_to` | forward | a fact/concept answers a question. |
| `supports` / `contradicts` | `leads_to` | forward / inverse | stance between nodes. |

**Weight** (0.0–1.0) is your graded strength/confidence in the relationship — never a
constant. A `derived_from` to the source a node distills is strong (~1.0); a `supports`
you're sure of sits high (~0.8); a `relates_to` affinity you're noting but not leaning on
is weak (~0.4–0.6). A map where every edge is `1.0` has thrown the weight signal away.

**Polarity** is the direction of the relation, chosen deliberately: `supports` and
`answers` are forward; `contradicts` is inverse; `part_of` is inverse (the part points at
the whole).

Worked examples — label + polarity + weight, each with the act envelope:

    # a concept answers an open question — strong, directional
    temper__assert_relationship(concept → question, edge_kind="leads_to", polarity="forward",
        label="answers", weight=0.9, invocation_id=inv.id, confidence="confident",
        reasoning="answers: this concept resolves the question's open ask")

    # two nodes in tension — inverse polarity carries "contradicts"
    temper__assert_relationship(node_a → node_b, edge_kind="leads_to", polarity="inverse",
        label="contradicts", weight=0.7, invocation_id=inv.id, confidence="probable",
        reasoning="contradicts: a's stance reverses b's")

    # a loose thematic affinity — real but weak
    temper__assert_relationship(node → theme, edge_kind="near", polarity="forward",
        label="relates_to", weight=0.45, invocation_id=inv.id, confidence="tentative",
        reasoning="relates_to: tangential thematic overlap, noted not leaned on")

## "Materially changed"

Read the changed source against the existing node. It is **materially changed** if
the distillation would now say something different — a new claim, a reversed
decision, a dropped commitment — not if the source merely got a typo fix or a
reworded sentence. When materially changed: **fold** the stale node's `derived_from`
edge and create a fresh node. Never edit the node's blocks in place. When in doubt,
prefer leaving the node and lowering your confidence over churning a fold.

## Authorship — a hard invariant on every authored act

Every one of the authored-4 — `create_resource`, `assert_relationship`,
`facet_set`, `fold_relationship` — **MUST** carry the act envelope. This is not
per-call discretionary; a node or edge without it is real but *orphaned*.

- **`invocation_id`** — the id returned by `invocation_open`. This is what
  correlates the act to the run. **Drop it and the act does not appear under
  `invocation_show`** — the map's nodes/edges become uncorrelated to the tick that
  authored them, breaking the accountability chain. Carry it on *every* call.
- **`confidence`** — `tentative` / `probable` / `confident`. Required whenever any
  other authorship field is set — it is the gate for the whole envelope. Which band
  when: **`confident`** = an explicit, dated decision or a claim the source states
  outright; **`probable`** = a synthesis you drew across sources they don't state
  verbatim; **`tentative`** = thin or uncertain evidence, noted not leaned on. Set the
  band from the *distillation's* footing, not a blanket default — a map where every act
  is `confident` has thrown the confidence signal away.
- **`reasoning`** — one line on *why* this act: which source it distills, why this
  label, why this edge. Required on create / edge / fold; set it on facets too.

Same envelope on every call — not just `create`:

    temper__assert_relationship(source, target, edge_kind, polarity, label, weight,
        invocation_id=inv.id, confidence="confident",
        reasoning="derived_from: this node distills source 019f…")

    temper__facet_set(resource, values,
        invocation_id=inv.id, confidence="confident",
        reasoning="marks the map's own question node resolved")

    temper__fold_relationship(edge_handle,
        invocation_id=inv.id, confidence="probable",
        reasoning="source materially changed; folding the stale derived_from")

**Before `invocation_close`, self-check:** every act you emitted this tick carried
`invocation_id` and `confidence`. If you authored a node, edge, or facet without
them, you broke the accountability chain — the acts exist but nothing ties them to
this run. Close with an outcome summarizing nodes / edges / facets / folds.

## Advancing the watermark — last, once, to a real event id

`steward_advance_watermark` is the **final** act of a tick, and it fires **exactly once**:

- **Sequence — after everything, never mid-run.** Advance only once *all* authored-4 acts
  are done, immediately before `invocation_close`. The watermark marks the whole delta as
  ingested; firing it partway through claims sources you have not distilled yet. Do not
  call it between `create_resource` batches or before your edges and facets land.
- **Id hygiene — a `kb_events.id`, not a `resource_id`.** Advance to the `max_event_id`
  from *this* tick's `steward_ingest_delta` — a real row in `kb_events` the session
  observed. A node or edge id you just created is **not** an event row; passing one 404s as
  "event … not found". The watermark is an *event* cursor, not a resource cursor.

Concretely: hold `delta.max_event_id` from the top of the tick, do every act, then pass
that same `max_event_id` to `steward_advance_watermark` right before you close.
