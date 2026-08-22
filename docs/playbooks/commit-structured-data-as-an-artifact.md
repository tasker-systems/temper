# Commit structured data as an artifact

**For anyone whose agent sessions produce structured data** — measurements, query plans,
computation outputs, structured inputs and outputs — that a later session must retrieve whole.

## Outcome

By the end of this playbook you will have committed structured output as a data artifact owned
by a resource, retrieved it whole in a later session, and understood when to reach for this
instead of writing fenced JSON into a resource body.

## Prerequisites

- **Temper installed** — see [Install Temper](./install-temper.md).
- **Authenticated** — you have signed in and your access is approved. See
  [Authenticate](./authenticate.md).
- **Familiar with contexts and refs** — data artifacts are owned by resources, which live in
  [contexts](../concepts/contexts-and-refs.md).

## The problem: why fenced JSON in resource bodies fails

Agents already persist structured data in Temper. The instinct is to write JSON or YAML into a
fenced code block inside a resource body — a session note, a task, a research document. This
works well enough for a human reading the note, but it fails the next agent session that needs
the data whole:

1. **The chunker shreds it.** Temper's ingest pipeline chunks resource bodies for search, and it
   has no fence-state tracking. A YAML comment at column 0 is parsed as a markdown heading, so a
   structured block is split at its own comment lines. Reassembling the original is not reliably
   possible.
2. **The fragments pollute the corpus.** Every chunk gets a 768-dim vector and a full-text search
   entry. JSON and YAML fragments are semantic noise in a corpus built for prose.
3. **There is no shape.** A fence inside prose has no declared family, no selection intent, and
   no way to say "this one supersedes that one." A reader must guess which fence is the right one
   and infer a shape nobody declared.

Data artifacts solve all three: the content is stored whole (never chunked, never embedded),
carries a family and selection intent, and is reached only through the resource that owns it.

## Commit a data artifact

Create a JSON file with structured output — for example, a measurement:

```json
{
  "metric": "p95_latency_ms",
  "value": 142.3,
  "endpoint": "/api/search",
  "timestamp": "2026-08-21T22:00:00Z"
}
```

Commit it to a resource you have write standing on — a task, a session, a research document:

```bash
temper data-artifact commit <resource-ref> \
  --kind "measurement" \
  --intent "current" \
  --content @measurement.json
```

- **`--kind`** — the bare family name. Free-form, but be consistent within a family
  (`"measurement"`, `"query-plan"`, `"computation-output"`).
- **`--intent`** — `current` (this is the one to take by default), `member` (one of several,
  ordered by `--precedence`), or `pinned` (explicitly held). A closed vocabulary — the system
  refuses an unrecognized value and tells you what is accepted.
- **`--content`** — `@path/to/file.json` (file), `-` (stdin), or omit for implicit stdin. Must be
  valid JSON.

The commit returns the artifact with its server-assigned ID, content hash, and shape state.

## Retrieve it later

A later, unrelated session retrieves the artifact by its ID:

```bash
temper data-artifact show <resource-ref> <artifact-id>
```

Or lists all artifacts owned by a resource, with optional filters:

```bash
temper data-artifact list <resource-ref> --kind "measurement" --intent "current"
```

The content round-trips byte-identical — the same JSON value the writer committed, without
reconstruction, reassembly, or inference about its shape. This is the core guarantee: what
survives the gap between sessions is the data itself, whole.

## Selection among many

When a resource owns several artifacts of one family, the selection vocabulary lets a reader
decide which to take from the stored record alone:

```bash
# The current measurement (the one to take by default)
temper data-artifact list <resource-ref> --kind "measurement" --intent "current"

# All measurements, including superseded ones
temper data-artifact list <resource-ref> --kind "measurement" --include-folded
```

### Supersession

When a new measurement replaces an old one, the writer declares it:

```bash
temper data-artifact commit <resource-ref> \
  --kind "measurement" \
  --intent "current" \
  --content @measurement-v2.json \
  --supersedes <old-artifact-id>
```

The old artifact is folded out of the default list — `--include-folded` returns it. The system
never infers supersession from recency or ordering; a writer must declare it.

## When not to use this

Data artifacts are for **structured content a later session must retrieve whole**. They are not
a replacement for:

- **Resource bodies** — session notes, problem statements, design rationale, and prose of any
  kind belong in the body. The body is chunked, embedded, and searchable; that is the right
  treatment for prose.
- **`open_meta`** — small, key-scoped metadata values (a `status`, a `verified` date, a
  `descriptor`) belong in open metadata. Data artifacts are for structured content of any size
  that does not fit in a property value.

The distinction is not about format — it is about whether the content is data that a later
session must retrieve whole, or prose that a reader must understand. A fenced JSON block in a
session note is prose with a shape; a data artifact is the shape without the prose.

## Further reading

- **CLI reference for `temper data-artifact`:**
  [data-artifact](../reference/cli/data-artifact.md).
- **Contexts and resource refs:** [Contexts and Refs](../concepts/contexts-and-refs.md).
- **Using Temper from the CLI:** [temperkb.io/using-temper](https://temperkb.io/using-temper).