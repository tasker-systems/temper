# Asking Temper a Question

Read this when you are asking the knowledge base a question and must choose between `temper search`
and `temper query`, write a composition, or read a trace. Flag syntax lives in `reference.md`.

## Which door

`search` is **quick-search**: one question, answered in two arms, fast. `query` is
**interrogation**: declared acts, piped together, answered in one round trip.

> **Does answering take more than one step, where a later step depends on what an earlier one
> found?** No → `temper search`. Yes → `temper query`. **Combining result sets** — union, intersect —
> across different questions is `query` only.

**`query` is not a better `search`.** Reach for it because the question has shape, never because it
sounds more thorough. A composition with one find stage is a slower `search` with more ceremony,
and answers a strict subset of what `search` does. If you cannot name the second step, you do not
have a composition.

What the shape buys: asked semantically, *"why is a plan declined before it runs"* returns material
near the words. `find-exact "refusal"` first, then `find-about-within` ranking inside that set,
returns the refusal material itself — a prefilter no phrasing of `search` reproduces.

## Writing a composition

A plan is JSON: `stages` (a DAG) and `outcome` (which stages return rows).

```json
{
  "stages": [
    { "name": "refusals", "act": "find-exact",
      "intention": { "query": "refusal" }, "terms": { "limit": 40 } },
    { "name": "why", "act": "find-about-within",
      "intention": { "query": "why a plan is declined before it runs" },
      "inputs": [ { "from": "upstream", "as": "bound", "stage": "refusals" } ],
      "terms": { "limit": 5 } }
  ],
  "outcome": { "returns": [ { "stage": "why", "with": [] } ] }
}
```

- **Every find act carries its own `intention`** — the question lives on the stage, and two stages
  asking different things is normal. A find act without one is refused.
- **`inputs` names upstream stages instead of copying ids**, and `as` says what the receiving act
  does with each set: `bound` narrows to within it, `seed` grows from it. It is a list, so one stage
  can carry a seed and a bound — a bounded walk. Two inputs in the same relation are refused; merging
  them is a `union` stage you declare, visible in the trace.
- **`outcome.returns` is not "the last stage".** Intermediate stages pipe ids, not rows; only what
  you declare comes back hydrated. You usually want the intermediate stages' trace, not their rows.
- **An `embedding` on an intention is optional**; omit it and the server computes one.

**Selecting by data artifacts.** `find-resources-with` takes `artifact_filter` beside
`resource_filter`: a list of asks, each a shape id and predicates over its paths. It answers
resources, never artifacts; `temper data-artifact ask` reads the artifacts themselves.
`list-resources-with` takes the same two filters and returns the matching resources as rows —
newest first, paged by `limit` and `offset`, with a `total` — and feeds no later stage.
`data-artifacts.md` carries the ask's semantics and refusals.

## Read the trace, not just the hits

The response carries `trace.stages` for **every** stage, including ones whose rows never came back.
Without it you cannot tell whether a stage earned its place.

- **`input_ids`** — ids received from upstream. A bound stage with `input_ids: 0` narrowed nothing;
  its answer is the unbounded one.
- **`produced_ids`** — ids the stage produced, on every stage. Read it rather than a downstream
  `input_ids`: a combinator reports only the sum of its arms, and a terminal one has no downstream.
  It can exceed the rows returned, because an id that stopped being visible is dropped from the rows
  but still counted.
- **`input_unusable`** — ids the act could not use: invisible, nonexistent and malformed are one number.
- **`terms_applied`** — the page the stage actually ran: your `limit` clamped to the act's ceiling, or
  the ceiling where you named none. **`extent: partial`** — it filled that page and there may be more.
  This is the only place a truncated intermediate shows: a `follow-from` clamped to 50 feeding an
  `intersect` yields a well-formed answer over a cut-off set.
- **`disposition`** (`answered` / `empty` / `withheld` / `refused`) and **`orders_by`** — on each
  returned stage. **Two stages' scores are not comparable**; do not re-sort them together.
- **`artifact_asks`** — on a `find-resources-with` stage that asked artifacts, one entry per ask in
  filter order: `shape_version` validated against, `in_force` when it ran (`null` if you can no
  longer read the shape's home), and the `population` counted. An empty selection explains itself:
  all zeros, nothing governed is visible; counts with `matched: 0`, none satisfied the ask; `matched`
  above zero, another ask or the `resource_filter` excluded the resources; a large
  `declared_not_yet_checked`, the sweep is behind your `shape_states` narrowing. The counts are of
  artifacts, each ask's own, and do not add or intersect into `produced_ids`.

**Settle `partial` by asking for the next page.** It over-reports at the boundary — a stage that
filled its page exactly looks the same as one with more waiting. Every act that admits `limit` admits
`offset`, `follow-from` included; an empty second page means the first was the whole set. Narrow the
walk instead when you wanted a neighbourhood, not an enumeration.

## Traps

- **A bound applies to the seed too.** `follow-from` starts only from seeds inside the bound, so a
  walk seeded outside its bounding set returns nothing — validly, silently, and `--check` says
  `expressible`. Bound a walk only when the seed is a member; otherwise walk unbounded and narrow
  afterwards with `intersect` / `difference`.
- **Pages are separate statements, not a cursor.** A write between requests re-ranks the set, so a row
  can be skipped or returned twice; over a busy graph an empty second page is evidence, not proof.
- **Never union per-page answers through a `difference`.** `X − (A∪B)` is not `(X−A) ∪ (X−B)`; the
  union is over-inclusive. Page the walk, union the walk's pages, take the difference once. Unioning
  pages that feed a `union` or `intersect` is sound.
- **An act that admits no `limit` returns a set, not a page** — no `offset`, never `partial`.
  `find-resources-with` is the one to know. Its rows come back through `list-resources-with`, whose
  `total` counts the whole match set on every page.
- **A clean `--check` is not a promise the query runs.** It is local (no network, no token), reports
  every refusal at once with a non-zero exit, and answers expressibility against the published
  contract only. It loads no shape, so it lists artifact asks under `shapes_not_checked`.
- **Do not keep a list of what refuses** — ask `--check`; the reachable acts and per-stage
  capabilities move as the surface grows.
- **A refusal is an answer, not a crash.** The server returns every refusal at once, each naming its
  stage; one with no stage is about the whole plan — a cycle, a dangling reference, a duplicate name.
  Under JSON output they print in `details.refusals`, each with its `reason` and `teaches`. Read them
  all before editing: they are often one cause with several symptoms.
