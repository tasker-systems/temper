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
    node = temper__create_resource(cogmap=cogmap, type=<label>, act)
    temper__assert_relationship(node -> source, label="derived_from", kind="express", act)
    for rel in inter_node_relationships(node):
      temper__assert_relationship(node -> other, kind, polarity, label, weight, act)
    for f in facets(node): temper__facet_set(node, f, act)

# Before closing, self-check: every act this tick carried invocation_id + confidence.
temper__steward_advance_watermark(cogmap, delta.max_event_id)
temper__invocation_close(inv, outcome)
```

## Choosing a node label

Per-source labels cite ~one source; synthesized labels span many (see granularity).

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

## Granularity

- **Per-source** (`fact`/`memory`/`decision`): one node cites ~one source — a single
  `derived_from` edge.
- **Synthesized** (`concept`/`question`/`theme`/`concern`/`principle`/`commitment`/
  `domain`): one node spans many sources — many `derived_from` edges into it.

## Edge conventions

The structural `edge_kind` carries affinity; the free-text `label` carries meaning.

| Semantic label | edge_kind | polarity | Use |
|----------------|-----------|----------|-----|
| `derived_from` | `express` | forward | node ← source provenance (every node). |
| `relates_to` | `near` | forward | symmetric affinity between nodes. |
| `part_of` | `contains` | inverse | whole–part. |
| `answers` | `leads_to` | forward | a fact/concept answers a question. |
| `supports` / `contradicts` | `leads_to` | forward / inverse | stance between nodes. |

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
  other authorship field is set — it is the gate for the whole envelope.
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
