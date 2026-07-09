# Building a cognitive map from a large corpus

How to turn a context full of source material into a telos-governed graph of distilled
nodes — an understanding shaped by a purpose, not a folder of your documents.

This guide covers the second half of the arc. Getting the corpus in, and understanding it,
comes first: see [Ingesting a corpus into a context](corpus-ingestion.md).

> **Source.** This guide generalizes a report from the first non-temperkb.io Temper
> deployment: dozens of source documents (~1,600 chunk resources) reduced to ~50 distilled
> nodes across three telos questions, with ~225 edges and ~55 facets, authored under a
> handful of accountable invocations. The vault resource is
> `external-deployment-feedback-agent-playbook-for-building-a-cognitive-map-from-a-large-corpus-019f4766-dba9-7970-af4b-69b2f6760348`.
>
> The per-call mechanics — label vocabulary, edge kind/polarity/weight conventions, the
> "materially changed" judgment — live in the installable skill content
> (`temper skill install`) and in the steward's `map-stewardship` skill. This guide teaches
> the *method*.

## When to build a map at all

Reach for a map when you want a **purpose-shaped distillation** — an understanding whose
shape a telos decides. Not when you want to store working artifacts; a context does that.

Two teloi over the same sources yield two different maps. That is the feature, not a
redundancy to eliminate.

The crux most authors miss: **a map node is not the same row as its source.** It is a new
resource, created into the map, that distills from one or more sources and carries a
`derived_from` edge and a `sources` provenance list back to them. Nodes are distilled,
never re-homed.

## Read the telos-charter first — it is the rubric

Every map has a telos-charter: a purpose statement plus a set of open questions. It decides
what earns a node. Read it before you author anything.

```bash
temper cogmap analytics <MAP>     # surfaces telos, staleness, regulation
```

Distill what is **salient under this charter**, not everything true about a source. The
same document read under two teloi yields two different nodes, and neither is wrong.

A well-formed charter decomposes into **question shapes that build on each other**. Three
generalize cleanly:

| Pass | Question | Produces |
|------|----------|----------|
| Q1 | **What recurs?** The shared vocabulary of the domain, and how each source parameterizes it. | `concept` nodes + `fact` instances |
| Q2 | **Where does it structurally break?** The recurring classes of incompatibility where one model cannot absorb the variation. | `theme` nodes, cross-linked back to Q1 |
| Q3 | **What is settled vs still open?** What has been banked, what remains divergent. | `decision` / `concern` / `question` nodes, each dated |

Later shapes — how things compose within one source, which provisions drive which
downstream effects — layer on the same way.

## One telos-question per pass

Do **one** charter question per invocation-pass. Each is a clean, reviewable unit, and the
acceptance bar is per-pass: at least one on-telos node per sub-topic, authored under a
closed invocation, materialized.

Checkpoint with the map's owner between passes. A pass is the natural unit of review.

## Breadth-with-confidence before depth

Cover many instances thinly, and **only where you can ground them** — rather than a few
instances deeply.

Depth invites over-committing to claims you have not yet reconciled across sources. Save it
for later passes, where certainty has been earned by the passes before.

Let the confidence band do the honest work:

- `confident` — the source states it outright.
- `probable` — a synthesis you drew across sources that none states verbatim.
- `tentative` — thin or unverifiable evidence.

**On a first breadth pass, skip the `tentative` band entirely.** If you cannot ground it,
it does not get a node yet. A map is not improved by nodes you would not defend.

## Calibrate the voice on node #1

Author **one** node. Read it back with `temper resource show <ref>`. Confirm the depth and
voice with the map's owner. *Then* batch the other twenty.

This is cheap insurance against redoing an entire layer, and it costs one round-trip.

## Layer the nodes

Two layers per abstraction pass:

- A small set of **`concept`** nodes for the shared vocabulary. Each: the definition → the
  axes it varies on → how the target system represents it → why it matters.
- A larger set of **`fact`** nodes for how specific sources instantiate each concept.

Concept nodes anchor regions; facts populate them.

Before creating any node, search the map for an existing one — `temper search "<concept>"
--cogmap <MAP>`. When two sources both assert one concept, distill **one** node citing both,
not two near-duplicates.

## The invocation flow

Every authoring pass is one invocation, opened before the first act and closed after the
last, so every act is correlated and auditable.

```bash
inv=$(temper invocation open --cogmap <MAP> --trigger-kind manual --format json | jq -r .id)

# per node — the act envelope is a hard invariant
temper resource create --cogmap <MAP> --type concept --title "<ascii title>" \
    --sources <SOURCE_REF>,<SOURCE_REF2> --sources-as-edges \
    --invocation "$inv" --confidence confident \
    --reasoning "why this node earns a place under this telos" \
    --model "<your-model>" --body @node.md

# per typed axis
temper resource facet <NODE_REF> --values '{"as_of":"2026-07-09"}' \
    --invocation "$inv" --confidence confident --reasoning "dated status claim"

temper invocation close "$inv" --disposition completed \
    --outcome '{"nodes":12,"edges":31,"facets":9}'

temper cogmap materialize <MAP>
```

**The act envelope is a hard invariant.** Every authored act carries `--invocation`,
`--confidence`, `--reasoning`, and `--model`. An act missing them is real but *orphaned* —
it will not appear under `invocation show`, and the audit chain is broken.

**Regions only exist after a materialize.** It is a safe no-op below its formation-delta
threshold, so run it at the end of every pass. Verify with `temper cogmap shape <MAP>`:
multi-member, high-cohesion regions forming around your concept nodes is the signal that
the structure took. Singletons are fine for genuinely distinctive nodes.

## Script the pass, and manifest the ids

This is the single highest-leverage habit in the whole method.

A pass is: open the invocation → loop `resource create` and `edge assert` from a small
driver → **record every created id to a node-manifest file**. Commit the manifest.

The manifest is what lets the *next* pass wire edges to *this* pass's nodes. Without it,
cross-pass linkage is guesswork — you are searching the map by title and hoping. With it,
the next pass reads a file.

`resource create` returns both an `id` and a decorated `ref` in its JSON response, so a
driver captures linkage state for free.

## Cross-question linkage makes it a graph, not a pile

This is what separates a map from a tagged list. **Every pass after the first links back
into the earlier ones.** The substrate will not infer these edges. Author them deliberately.

Concretely, later passes assert edges like:

- a structural-break theme `breaks →` the concept it defeats, and is `exhibited_by →` the
  specific fact instances that show it;
- a status node `settles →` or `concerns →` the concept or theme it reports on, and
  `classifies →` the patterns a methodology sorts.

The payoff is visible at materialize: a status node about a topic *joins the region* of the
concept it concerns, while meta-level nodes — methodology, principles — form their own
regions. The map ends up traversable as *what recurs → why it breaks → what's settled about
it* in one connected structure.

## Provenance, two ways

`--sources` records **block provenance** on the node's body. A `derived_from` **edge** makes
that lineage visible in the graph. These are different records, and you want both.

`--sources-as-edges` on `resource create` asserts one `derived_from` edge per
resource-valued source, with the canonical `(leads-to, inverse)` shape. Prefer it to
hand-asserting the edges; a hand-rolled `(express, forward)` edge shares the label but not
the shape, and the region math treats the two differently.

The authorship trio — `temper-provenance`, `temper-llm-model`, `temper-llm-run` — is
**stamped into `managed_meta` by the server**, derived from the act envelope. Do not pass it
by hand.

**Distill the abstraction from the analysis artifacts, and anchor it to raw sources.**
Concept and theme nodes honestly cite the downstream analyses; fact nodes cite the specific
raw chunks. The transitive chain — theme `exhibited_by→` fact `derived_from→` raw chunk —
grounds the abstraction without forcing a raw citation onto every node.

## Ground volatile claims in a dated source

For anything that drifts — status, "what's still open", current ownership — cite a **dated**
source and stamp an `as_of` facet. Never assert current state by inference.

A node that claims live truth without a date will be wrong silently, and nothing in the map
will tell you when it turned.

## Facets are the axes you want the map organized by

Set the typed axes — source-family, segment, category, status, `as_of` — on the instance and
status nodes. After materialize they surface as cross-cutting region views.

Put the same values in the node's body and title too: content-based region formation
reinforces what the facets declare.

Facets carry a node's *semantic* properties. Source provenance never goes in a facet.

## A compact checklist for one distillation pass

1. Read the telos-charter. Pick the **one** question this pass answers.
2. Ingest any missing citable sources — check max line length first.
3. Decide the node layers and the breadth-with-confidence bar for this pass.
4. `invocation open`.
5. Author node #1. Show it. Calibrate voice with the owner.
6. Batch the rest: `resource create --cogmap … --sources … --sources-as-edges`, recording
   every id to the node manifest.
7. Assert the cross-question edges back into earlier passes' nodes.
8. `resource facet` the typed axes. Stamp `as_of` on anything volatile.
9. `invocation close` with an outcome count.
10. `cogmap materialize`, then `cogmap shape` to confirm regions formed.
11. Commit the node manifest.
