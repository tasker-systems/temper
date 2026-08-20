# Ingest a corpus

**For individual users** — someone getting a body of existing material into a Temper
context.

By the end of this playbook you will have a [context](../concepts/contexts-and-refs.md)
full of faithful, well-identified, individually citable source material — the raw
substrate a [cognitive map](https://temperkb.io/cognitive-maps/what-lives-in-a-map) is
later distilled from, and a corpus you can already search. Ingestion is the cheap,
mechanical half of that work; the distillation that follows is a different act with a
different discipline.

## Prerequisites

- **Temper installed.** If you have not, [install Temper](../playbooks/install-temper.md)
  first.
- **A context to ingest into.** A
  [context](../concepts/contexts-and-refs.md) is a named, owned boundary for resources —
  every resource you ingest lives in exactly one. You can create one for this corpus, or
  reuse an existing one.
- **Authenticated to a Temper instance.** Run `temper init` and sign in (or point the CLI
  at your self-hosted instance) so `temper resource create` can reach the API.

## The distinction that governs everything

**A context homes material as it is. A cognitive map homes a purpose-shaped understanding
of it.**

These are different acts, and conflating them is the most common mistake. Putting the
documents somewhere is not building an understanding. Ingestion is cheap, mechanical, and
should be boring. Save your judgment for the distillation.

So: get the corpus in faithfully, attach the identity you will want to filter on later,
and stop. Do not start deciding what matters while you are still deciding what exists.

## Create the context (once)

```bash
temper context create "Corpus"
```

The slug is derived from the name you give, so the ref becomes `@me/corpus`. Note the
`id` and `ref` in the response — you will pass the ref to every `resource create` below.

`context create` is **not idempotent.** Re-running it with the same name does not return
the existing context; it auto-suffixes and mints a new one — `Corpus` becomes `Corpus 2`
(slug `corpus-2`). A script that assumes idempotency will create duplicates. Run it once,
capture the ref, and reuse it. If you need to recover the ref later, `temper context list`
shows it.

## Check the maximum line length before you ingest

Do this first, before anything else touches a document.

A single very long line with no internal newline — a wide table row, an unwrapped
paragraph, a base64 blob — can blow past a chunker's size guard and then dominate
tokenization cost. Wrap it, chunk that document by hand, or skip it. Do not feed it whole
and hope.

```bash
awk '{ if (length($0) > max) max = length($0) } END { print FILENAME, max }' doc.md
```

## Chunk at semantic seams, not byte offsets

Split large documents at their own structural boundaries — headings, sections — and
greedily pack small sibling sections up to a safe size ceiling. Do not chunk at a fixed
character count.

The reason is not aesthetic. A distilled node cites *specific* chunks, and a chunk that
begins mid-sentence makes a poor citation. Section-aligned chunks are individually
meaningful, which is what makes them worth pointing at months later. You are choosing
your future citations now.

## Make the run resumable with a manifest

Long ingests get interrupted. Background jobs get reaped, auth expires, machines reboot.
The fix is not to make the run more reliable — it is to make interruption free.

Key each chunk by `path + chunk-index` and record the
[ref](../concepts/contexts-and-refs.md) (and `id`) the create returned:

```jsonl
{"path": "docs/spec.md", "chunk": 0, "id": "019f47e2-0126-7a23-a905-20dc97848af6"}
{"path": "docs/spec.md", "chunk": 1, "id": "019f47e2-e268-7930-ac11-3f89f8e8f84c"}
```

`resource create` prints a `ref` and an `id` on every create — read the `id` straight out
of the JSON response. Temper's output is JSON on a non-TTY, so a driver script needs no
flag to parse it.

Three rules make the manifest earn its keep:

- **Checkpoint incrementally and atomically.** Write a temp file and rename it, after each
  chunk. A manifest written once at the end protects nothing.
- **Skip what exists, backfill the rest.** A re-run should cost nothing for work already
  done. `resource create` is *not* idempotent — content dedup is gone — so the manifest is
  the only thing standing between a crash and a pile of duplicate chunks.
- **Commit it as you go.** A later revert, or a fresh session tomorrow, inherits the
  resume state.

**Verify completeness by re-running to a fixpoint.** `created=0` on a clean re-run is a
stronger proof that you ingested everything than any progress log you could write.

If a background job keeps getting reaped mid-run, detaching it (`nohup`, a new session)
lets it finish untouched — at the cost of polling for completion instead of receiving a
signal. Choose per how long the job runs.

## After a write error, reconcile — don't retry

A `resource create` or `resource update` can print `network error: error sending request
for url …/api/ingest` and exit non-zero **even though the write already committed on the
server**. This is a *lost acknowledgment, not a lost write*: the request reaches the
backend, the mutation is persisted, and the connection then drops before the response is
returned. The client cannot tell "never committed" from "committed but un-acked" — it only
sees the dropped connection.

The danger is the *reaction*. Because `create` mints a fresh identifier on each invocation
and the write path carries no client-supplied idempotency key, a **blind retry creates a
duplicate** rather than converging on the already-committed resource. The correct recovery
is **reconcile, then re-issue only if the write genuinely did not land**:

- **create** — `temper resource list --title-contains "<title>"` reports whether the
  resource is present.
- **update** — `temper resource show <ref>` (add `--edges` for a `--goal`/link change)
  reports whether the mutation applied.

Detection is deterministic even though the fault is not: the reconcile check unambiguously
tells you whether to re-issue, and it is what prevents the duplicate. The CLI prints this
reconcile-don't-retry reminder to stderr whenever a `create`/`update` fails with a network
error, so the hazard is visible at the moment it bites.

A manifest-driven bulk run already encodes this recovery: its exact-key skip-what-exists
pass **is** the reconcile, so a re-run to a fixpoint adopts the committed-but-unrecorded
write instead of duplicating it. Single, ad-hoc `create`/`update` calls have no such key —
there the reconcile is manual, which is exactly why it is easy to forget.

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
whose shape a telos (a purpose statement) decides. Two teloi over the same corpus yield
two different maps, and that is the feature. See
[Building a cognitive map](../guides/building-a-cognitive-map.md), and
[how a map grows](https://temperkb.io/cognitive-maps/how-a-map-grows) for the arc that
follows this one.

For day-to-day CLI usage while you ingest, see
[using Temper from the CLI](https://temperkb.io/using-temper).
