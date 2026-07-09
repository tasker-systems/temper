# Ingesting a corpus into a context

How to get a large body of source documents into temper so that it can later be
*understood* — and, if a telos calls for it, distilled into a cognitive map.

This guide covers the first two movements of that work: getting the corpus in, and
understanding it. The distillation that follows is a different act with a different
discipline — see [Building a cognitive map](building-a-cognitive-map.md).

> **Source.** This guide generalizes a report from the first non-temperkb.io Temper
> deployment: one agent ingested dozens of documents (~1,600 chunk resources), understood
> them, and distilled ~50 nodes across three telos questions. The vault resource is
> `external-deployment-feedback-agent-playbook-for-building-a-cognitive-map-from-a-large-corpus-019f4766-dba9-7970-af4b-69b2f6760348`.

## The distinction that governs everything

**A context homes material as it is. A map homes a purpose-shaped understanding of it.**

These are different acts, and conflating them is the most common mistake. Putting the
documents somewhere is not building an understanding. Ingestion is cheap, mechanical, and
should be boring. Save your judgment for the distillation.

So: get the corpus in faithfully, attach the identity you will want to filter on later,
and stop. Do not start deciding what matters while you are still deciding what exists.

## Chunk at semantic seams, not byte offsets

Split large documents at their own structural boundaries — headings, sections — and
greedily pack small sibling sections up to a safe size ceiling. Do not chunk at a fixed
character count.

The reason is not aesthetic. A distilled node cites *specific* chunks, and a chunk that
begins mid-sentence makes a poor citation. Section-aligned chunks are individually
meaningful, which is what makes them worth pointing at months later. You are choosing
your future citations now.

## Check the maximum line length before you ingest

**Do this first, before anything else touches the document.**

A single very long line with no internal newline — a wide table row, an unwrapped
paragraph, a base64 blob — can blow past a chunker's size guard and then dominate
tokenization cost. This is not hypothetical; it bit the source deployment twice, once in
the corpus proper and once in a status document carrying a 9,500-character line. It is
tracked as [issue #316](https://github.com/tasker-systems/temper/issues/316).

```bash
awk '{ if (length($0) > max) max = length($0) } END { print FILENAME, max }' doc.md
```

If a line is enormous, wrap it, chunk that document by hand, or skip it. Do not feed it
whole and hope.

## Make the ingest idempotent with a manifest

Long ingests get interrupted. Background jobs get reaped, auth expires, machines reboot.
The fix is not to make the run more reliable — it is to make interruption free.

Key each chunk by `path + chunk-index` and record the resource id the create returned:

```jsonl
{"path": "docs/spec.md", "chunk": 0, "id": "019f47e2-0126-7a23-a905-20dc97848af6"}
{"path": "docs/spec.md", "chunk": 1, "id": "019f47e2-e268-7930-ac11-3f89f8e8f84c"}
```

`resource create` prints a `ref` and an `id` on every create — read the `id` straight out
of the JSON response. Because temper's output is JSON on a non-TTY, a driver script needs
no flag to parse it.

Three rules make the manifest earn its keep:

- **Checkpoint incrementally and atomically.** Write a temp file and rename it, after each
  chunk. A manifest written once at the end protects nothing.
- **Skip what exists, backfill the rest.** A re-run should cost nothing for work already
  done. Note that `resource create` is *not* idempotent — content dedup was retired — so
  the manifest is the only thing standing between a crash and a pile of duplicate chunks.
- **Commit it as you go.** A later revert, or a fresh session tomorrow, inherits the
  resume state.

**Verify completeness by re-running to a fixpoint.** `created=0` on a clean re-run is a
stronger proof that you ingested everything than any progress log you could write.

When a harness-managed background job kept getting reaped mid-run, detaching it (`nohup`,
a new session) let it finish untouched — at the cost of polling for completion instead of
receiving a signal. Choose per how long the job runs.

## Attach identity at ingest

Put the structured properties you will want to filter and facet by — source type,
sub-unit, region, version, role — onto each resource as metadata **during** ingestion.

```bash
temper resource create --type research --title "Spec §3 — retry semantics" \
    --context @me/corpus \
    --open-meta '{"source-doc":"spec.md","section":"3","doc-family":"protocol"}' \
    --body @chunk-003.md
```

Re-deriving this later means re-reading every document. Attaching it now costs one flag.

When one document yields many chunks, add a per-document **index resource** and a
`contains` edge from it to each chunk, so the document stays navigable as a unit:

```bash
temper edge assert <INDEX_REF> <CHUNK_REF> --kind contains --polarity forward \
    --label contains --weight 1.0
```

## Understand before you distill

Resist authoring anything until you can answer, from evidence: what *is* each source, and
what has already been said about it?

**Resolve identity from the body, not the frontmatter.** Frontmatter is reliable for
provenance fields — source file, dates, section list — but is usually silent on the things
that matter for organizing an understanding: what claim the document makes, what it
settles, what it leaves open. The body prose is the dependable signal.

**Cross-validate identity across independent sources, and record your confidence.** Where
a naive rule — a filename prefix, a shared identifier — would fabricate a fact, a
cross-source join *flags the ambiguity* instead. That flag is worth more than a confident
guess. When two sources disagree, the disagreement is the finding.

**Inventory the artifacts a downstream effort has already produced** about the corpus:
analyses, catalogs, status snapshots. These are often the richest, most *distilled*
sources available — better than the raw documents for saying "what recurs." Ingest the
good ones as citable resources too.

Consider homing those analysis artifacts in a **separate context** from the raw corpus, so
the raw material stays pure. A map cites resources by id regardless of which context homes
them, so context boundaries cost you nothing at citation time.

## What comes next

You now have a context full of faithful, well-identified, individually citable material,
and an understanding of what it is. That is not yet a map, and it may never need to be —
`temper search --context @me/corpus` is already useful.

Reach for a cognitive map when you want a *purpose-shaped distillation*: an understanding
whose shape a telos decides. Two teloi over the same corpus yield two different maps, and
that is the feature. See [Building a cognitive map](building-a-cognitive-map.md).
