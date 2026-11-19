# Resource erasure — when a whole resource must not exist

**For operators and auditors.** Anyone deciding whether a leak calls for erasing a whole resource,
running the act, or reading its record afterwards.

## Its purpose

Resource erasure serves **remediation** ([purpose (b)](./erasure-acts.md#the-two-purposes)) when
the confidentiality of the whole document is at stake: the resource is erased together with its
relationships and its history. It is also the engine of person erasure ([purpose (a)](./erasure-acts.md#the-two-purposes)),
which runs this same act over every resource in the person's own contexts.

When only one field or one block carries the leak, a narrower act leaves the resource live:
[field scrub](./field-scrub.md) or [block history scrub](./block-erasure-and-history-scrub.md).

## Running it

`temper admin erasure resource <resource>` surveys: it counts the blocks, revisions, chunks,
artifacts and edges the act would reach, and names what it would leave. `--execute` erases. The
API doors are `POST /api/admin/resources/erasure/survey` and `POST /api/admin/resources/erasure`.

The act reaches a resource wherever it is homed, in a personal context or a team's. The operator is
acting on the resource, not on a person, so no team boundary applies.

- **A soft-deleted resource can be erased.** That is the commonest case: the content was deleted,
  and then a compliance need arrives that demands it not exist at all.
- **An ingest in flight ends** with the erasure, and the record names it.
- **A cognitive map's charter is refused.** A charter is erased only with its map, and map-grain
  erasure does not exist yet.
- **An already-erased resource** is refused, except that running the act again over a husk
  completes any ledger rewrite an earlier version of the act did not make.

## What it erases

The resource becomes a **husk**: the row stays, marked erased, with placeholder title and origin.
A husk answers `410 Gone` to those who held it directly: the owner of its home, or a holder of a
read grant on the resource itself. Everyone else, including those who read it only through its
context or a cognitive map, gets the same not-found an unknown id gets. Nothing can write to it
afterwards.

In one transaction the act empties, or replaces with placeholders:

- every block revision and every chunk, current and superseded, and their headings and embeddings;
- the search data;
- every data artifact on the resource, whichever principal's kind owns it;
- every property: those of the resource, of its blocks, and of every edge touching it;
- every relationship touching it, whoever authored it: each is folded and its label cleared;
- citation-audit reasons on its blocks, the remote sources its blocks cite, its ingestion record
  and its workflow jobs;
- derived vectors: every region holding the resource is recomputed without it, or zeroed if the
  region is folded, and its current home context's telos snapshot is cleared.

**The ledger is rewritten too.** Every event in the resource's trail has its copies of that text
replaced by placeholders, agent reasoning included, so replaying the ledger cannot bring the
content back. The record's `redacted_fields` lists every path the act rewrote.

## Blobs

Blobs related to the resource are named in the survey and are **not** struck unless the operator
lists them: `--also-strike-blob <blob>`, repeatable, only with `--execute`. A listed blob that the
survey did not name is refused, and nothing is struck. A struck blob's bytes are released only
when no live record carries the same content hash. See [Deleting a blob](./blob-delete-and-erasure.md).

## What it leaves

The act does not reach:

- copies of the home's telos snapshot already on the ledger, and snapshots held by contexts that
  homed the resource before its current home;
- text about the resource in other resources, citation-audit reasons on other resources' blocks,
  and other free text that quotes it: finding those is the sensitivity sweep's work;
- the resource's id, in the husk, its homes and grants, and the edges that touched it. The id is
  what lets a holder be told the resource was erased.

## Further reading

- **The two purposes and the vocabulary:** [Erasure acts](./erasure-acts.md).
- **Person erasure, which runs this act across an estate:** [Erasure and the terms of use](./erasure-and-terms-of-use.md).
