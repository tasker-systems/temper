# Block erasure and history scrub — when one block holds the leak

**For operators and auditors.** Anyone remediating a secret or a personal identifier that sits in
one block of a resource that should otherwise survive.

## Its purpose

Block history scrub serves **remediation** ([purpose (b)](./erasure-acts.md#the-two-purposes)) at
the grain of one block: a single block holding a social security number or a secret is removed
without destroying the resource around it. It is a scrub, not an erasure: it trims the named
blocks' history and leaves the resource live.

## Erasing one block today

There is no single act that erases a block. The sanctioned path is two steps:

1. **Fold the block.** It stops reading, and its text still exists in its history.
2. **Scrub its history:** `temper admin erasure block-history <resource> --block <block> --execute`.
   A folded block loses every revision and every chunk.

When the leak is in an earlier revision and today's text is clean, skip the fold: the scrub
keeps a live block's current text.

## Running it

`temper admin erasure block-history <resource> --block <block>` surveys; `--block` is repeatable
and needs at least one; `--execute` scrubs. The API doors are
`POST /api/admin/resources/block-history-scrub/survey` and
`POST /api/admin/resources/block-history-scrub`.

The act is refused for an erased resource and for a cognitive map's charter. A soft-deleted
resource can be scrubbed, and stays soft-deleted. An ingest in flight is cancelled, and the
record says so.

## What it empties

For each named block:

- **a live block:** every revision except its current one, and every chunk that is not current,
  with their headings and embeddings;
- **a folded block:** every revision and every chunk.

It rewrites no ledger payload. Block and chunk text never sit on the ledger, only their hashes,
so replaying the ledger reproduces the scrubbed state.

## What it leaves

- the current revision and current chunks of a live named block;
- the block's provenance, including the remote source it cites;
- citation-audit reasons on the named blocks, and their copies on the ledger;
- the blocks' share of derived vectors: the home's telos snapshot and folded regions still average
  the scrubbed chunks' vectors, and live regions do until their next recompute;
- the content hashes of the emptied chunks.

When any of these matters, [resource erasure](./resource-erasure.md) reaches them, at the cost of
the whole resource.

## Further reading

- **The two purposes and the vocabulary:** [Erasure acts](./erasure-acts.md).
- **The metadata-only counterpart:** [Field scrub](./field-scrub.md).
