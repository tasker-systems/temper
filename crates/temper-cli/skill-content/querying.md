# Asking Temper a Question

Two doors. `temper search` and `temper query`. This file is about **which one**, and how to use the
second — it deliberately carries no flag syntax, because `reference.md` is generated from the CLI
itself and will always be right where a copy here would rot.

## Which door?

`search` is **quick-search**: one question, answered in two arms, fast.
`query` is **interrogation**: declared acts, piped together, answered in one round trip.

That headline orients you. This is the test that decides:

> **Does answering this take more than one step, where a later step depends on what an earlier step
> found?**
>
> - **No** → `temper search`.
> - **Yes** → `temper query`.
>
> And: **do you need to combine result sets** — union, intersect — across different questions?
> **`query` only.** `search` cannot express it.

### `query` is not a better `search`

Reach for it because your question has **shape**, never because it sounds more thorough. A
composition with one find stage is a slower `temper search` with more ceremony, and today it answers
a strict **subset** of what `search` does — several per-stage capabilities still refuse.

The failure mode this warns against is real and one-directional: agents reach for `query` because it
is the more powerful-sounding tool, and get a worse answer more slowly. If you cannot name the
second step, you do not have a composition.

### What the shape actually buys — a worked case

Asking *"why is a plan declined before it runs"* semantically across everything returned notes about
subagents, verification and dogfooding: things near the **words**, not the subject.

Composing it — `find-exact "refusal"` to get the set that genuinely uses the term, then
`find-about-within` to rank semantically **inside** that set — returned the actual refusal material.
Same question, two doors, and the answers had **no resources in common**. The prefilter is doing work
no amount of better phrasing to `search` would do.

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

Four things carry the design, and each is worth understanding rather than copying:

- **Every find act carries its own `intention`.** The question lives on the stage, not on the
  envelope — two stages asking different things is the normal case, not an edge case. Omit it on a
  find act and the plan is refused.
- **`inputs` names upstream stages rather than copying ids**, and `as` says what the receiving act
  does with each set: `bound` narrows to within it, `seed` grows from it. There is no way to supply
  a set without saying which — the relation belongs to the edge.

  It is a **list**, because a stage may carry a seed *and* a bound at once — that is what a bounded
  walk is. Two inputs in the *same* relation are refused rather than merged: merging them is
  `union`, which is a stage you declare, that appears in the trace, and whose size a reader can see.

  > **The bound applies to the SEED too, and that is the trap.** `follow-from` starts only from
  > seeds that are inside the bound, so a walk seeded with a resource the bounding set does not
  > contain returns **nothing** — validly, silently, and with `--check` reporting `expressible`.
  > Bound a walk only when the seed is a member of the bounding set; otherwise walk unbounded and
  > narrow afterwards with `intersect` / `difference`.
- **`outcome.returns` is not "the last stage".** Intermediate stages pipe **ids**, not rows; only
  what you declare comes back hydrated. Returning everything is usually the wrong instinct — it is
  the intermediate stages' *trace* you want, not their rows.
- **You may supply an `embedding` on an intention, or omit it** and the server computes one. Omitting
  is correct for every caller that cannot embed.

## Read the trace, not just the hits

The response carries `trace.stages` for **every** stage, including ones whose rows never came back.
Without it a composition is a black box with an answer at the end, and you cannot tell whether a
stage earned its place.

The field that answers "did my pipe actually do anything" is `input_ids` — how many ids the stage
received from upstream. A bound stage showing `input_ids: 0` narrowed nothing, and its answer is the
unbounded one wearing a composition's costume. `input_unusable` counts ids the act could not use.

Each returned stage also carries `disposition` (`answered` / `empty` / `withheld` / `refused`) and an
`orders_by` naming the quantity it ranked on. **Two stages' scores are not comparable** — the
response keys arms separately precisely so there is no merged list for incommensurable rows to fall
into. Do not re-sort them together.

## Check before you send

`temper query --check` runs the shape pass locally: no network, no token, no server. It reports
**every** refusal at once, exits non-zero when there are any, and prints them to stdout as data.

Its limit is a real one and it states it in every report: it answers **expressibility** — is this
plan well-formed against the published contract — and it cannot speak to what the server has built.
A clean `--check` is not a promise the query will run.

**Do not maintain a list of what refuses.** Ask `--check`. The set of reachable acts and per-stage
capabilities moves as the surface grows, and a list written here would be wrong before it was read.

## When a plan is refused

A refusal is an answer, not a crash. The server returns **every** refusal at once rather than the
first, because repairing a plan one refusal per round trip is the experience this contract exists to
avoid — and the CLI prints all of them, each naming the stage it attaches to. A refusal with no stage
is about the composition as a whole: a cycle, a dangling reference, a duplicate name.

Read them all before editing. They are frequently one cause with several symptoms.
