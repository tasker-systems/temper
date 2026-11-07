# Cognitive Maps

Read this when a session reads from or authors into a cognitive map. Per-call authoring detail
(label vocabulary, edge conventions, the steward loop) lives in the steward's `map-stewardship`
skill where it is installed; without it, take the vocabulary from the map's charter and its
existing nodes and edges rather than inventing one.

## The model

- **A context homes resources as they are; a map homes distilled nodes.** A map is a
  telos-governed graph of nodes, edges, facets and regions, not a folder of your documents. Use a
  context to do work; use a map to orient across a purpose-shaped understanding of material.
- **A map node is a new resource, never the same row as its source.** A resource has exactly one
  home, and `resource create` takes `--context` or `--cogmap`, never both, so a context resource
  cannot be re-homed into a map. A node distills one or more sources and points back at them.
- **The telos-charter decides what is worth a node.** It is the map's purpose: a statement, open
  questions and framing. Read it before authoring (the `cogmap_read_charter` MCP tool, or
  `temper cogmap analytics <MAP>` for telos, staleness and regulation). Distill what is salient
  under this charter, not everything true about the source; another telos yields another node.
- **The authored set is four acts:** `resource create --cogmap` (`create_resource`), `edge assert`
  (`assert_relationship`), `resource facet` (`facet_set`) and `edge fold` (`fold_relationship`).
- **Every authored act runs under an invocation envelope.** Open one per pass. Every act carries
  `--invocation`, `--confidence` and `--reasoning` (and `--model`). An act missing them is real but
  orphaned: it never appears under `invocation show`, which breaks the accountability chain.
- **Confidence has three bands:** `confident` (a dated decision, or a claim the source states
  outright), `probable` (a synthesis the sources do not state verbatim), `tentative` (thin, noted).
- **Provenance means two things, and neither goes in a facet.** The server stamps the authorship
  trio into `managed_meta` from the envelope: `temper-provenance` (`llm-discovered` with `--model`,
  `user-created` without), `temper-llm-model`, and `temper-llm-run` (the invocation). Pass
  `managed_meta` only to override; your value wins, a missing one is filled. Source provenance rides
  on `--sources`, plus one `derived_from` edge per resource-valued source with `--sources-as-edges`.
  Facets hold semantic properties: a stance, a resolved question, or `as_of`, which anything
  volatile carries, grounded in a dated source; never assert current state by inference.
- **Supersession is fold, then recreate.** Never edit a node's body in place. When a source
  materially changes (the node would now say something different, not a typo fix), fold the stale
  `derived_from` edge and create a fresh node. In doubt, lower the node's confidence instead.
- **Regions exist only after a materialize:** `temper cogmap materialize <MAP>`
  (`cogmap_materialize`), or `temper context materialize` (`context_materialize`) for a context.
  Currency is `cogmap analytics`; a context's peer (`temper context analytics`, or `context_read`
  view `analytics`) has three fields, not five: `materialized_at`, `latest_touch`, `is_stale`.
  Staleness is reported, never blocking, and covers only what you can read, so callers can disagree.
- **Reading is not authoring.** A team joined to a map reads what it homes. Authoring needs an
  explicit write grant (`temper cogmap grant … --write`), not membership. A principal who may
  author a map may also modify the nodes homed in it (`can_modify_resource`'s container arm), so a
  map-write grant reaches every node in that map.
- **One map at a time.** `temper search "<query>" --cogmap <MAP>` scopes to one map; a second
  `--cogmap` is refused, not unioned, and there is no cross-map pooling. Search each map and compare
  yourself. Name neighbouring maps explicitly if a node should reference one; nothing sweeps them.

## Methodology

- **Search the map before creating;** never duplicate a node. Two sources asserting one concept
  get one node citing both in `--sources`.
- **One charter question per invocation pass.** Cover many instances thinly and only where you can
  ground them; skip `tentative` on a first breadth pass. Questions that build: what recurs, where it
  structurally breaks (themes linking back), what is settled vs still open (dated nodes).
- **Calibrate on node #1.** Author one node, `resource show` it, and confirm depth and voice with
  the map owner before batching; check in again between passes.
- **Keep a node manifest.** Record and commit every created id as you go; skip sources it already
  records, so a re-run creates only what is missing and `created=0` proves the pass done. It is how
  the next pass wires edges back to this one.
- **Link every pass back into the earlier ones.** The substrate does not infer cross-question
  edges; assert them with labels the map already uses (a theme `breaks →` the concept it defeats,
  a status node `concerns →` its topic), or you get parallel piles, not a graph.

## One authoring pass, end to end

```bash
# Orient: the charter, then what the map already holds.
temper cogmap analytics <MAP>
temper search "<concept>" --cogmap <MAP>
temper resource show <NODE_REF> --edges        # the stale node's derived_from correlation id

# Open the envelope; create-style responses carry a generic `id`. Check the exit code first:
# on failure stdout is {"error":{code,message,hint?}} and jq yields "null" for every later step.
inv=$(temper invocation open --cogmap <MAP> --trigger-kind manual --format json | jq -r .id)

# Supersede, only if the source materially changed.
temper edge fold <DERIVED_FROM_CORRELATION_ID> --reason "source materially changed" \
    --invocation "$inv" --confidence probable --reasoning "folding stale derived_from"

# The fresh node. --sources-as-edges asserts derived_from with the canonical shape.
temper resource create --cogmap <MAP> --type concept --title "<ascii title>" \
    --sources <SOURCE_REF> --sources-as-edges --body @node-body.md \
    --invocation "$inv" --confidence confident --model "<your-model>" \
    --reasoning "why this node under this telos"

# A semantic facet, and an inter-node edge.
temper resource facet <NEW_NODE_REF> --values '{"as_of":"2026-07-09"}' \
    --invocation "$inv" --confidence confident --reasoning "dated status source"
temper edge assert <NEW_NODE_REF> <OTHER_NODE_REF> --kind near --polarity forward \
    --label relates_to --weight 0.6 \
    --invocation "$inv" --confidence probable --reasoning "adjacent claims under this telos"

# Recompute regions, close, verify.
temper cogmap materialize <MAP>
temper invocation close "$inv" --disposition completed --outcome '{"nodes":1,"edges":2,"folds":1}'
temper invocation show "$inv"
temper resource show <NEW_NODE_REF> --edges     # managed_meta trio + the derived_from edge
```

## Traps

- **Hand-assert `derived_from` as `--kind leads-to --polarity inverse`,** the shape that
  `--sources-as-edges` writes; `express`/`forward` shares the label, not the shape region math reads.
- **`--sources-as-edges` skips remote URLs and event-id sources:** neither has a node to point at.
- **Use ASCII in node titles.** Slug generation has failed creates on non-ASCII titles.
- **Provenance never goes in `open_meta`;** if you pass a provenance key at all, use `managed_meta`.
- **An empty `cogmap shape` says why in `emptiness`.** Arms: `never_clustered` (materialize it),
  `nothing_visible`, `lens_narrowed` (your `--lens` excluded them), `unreadable_or_absent`. An
  empty `regions` list does not mean "not materialized yet".
- **`nothing_visible` is not a permissions diagnosis.** It covers both "no regions formed" and "no
  region holds a member you can read", alike by design so it never reveals what is beyond your
  reach. Check the map's contents and re-materialize before suspecting access.
- **`cogmap list`'s `region_count` and `shape`'s `population` disagree by design.** `region_count`
  is every non-folded region, ungated; `population` counts regions holding a member you can read.
  A lower `population` means regions outside your reach, not missing ones.
- **Cross-map edges are inert at the region layer.** Materialize accepts them but they add nothing
  to the target map's regions; do not use them to make two maps "talk".
