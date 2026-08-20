# Resource-owned data artifacts — schema-bounded structured data with a home

**Status:** design, approved in session 2026-08-20. No implementation.
**Context:** `@j-cole-taylor/temper`

## The problem, measured

Agents already persist structured data in Temper. In the enterprise edition they write JSON and
YAML into fenced code blocks inside resource bodies, and later agent sessions read those fences
back to ground the next stage of work — measurements, query plans, computation artifacts,
structured inputs and outputs. The practice exists and is load-bearing. It has no home.

Three independent failures were measured against this repo on 2026-08-20, not argued.

### 1. Properties reject anything over 2704 bytes

`open_meta` writes land in `kb_properties`, which carries `uq_kb_properties_active` — a btree over
`(owner_table, owner_id, property_key, property_value)`. Probed against local Postgres:

```
n=100 (3600 bytes json) FAIL: index row size 3648 exceeds btree version 4 maximum 2704
n=230 (8280 bytes json) FAIL: index row requires 8328 bytes, maximum size is 8191
```

The ceiling is the btree v4 maximum, **2704 bytes**, and the write is rejected at INSERT. A minimal
query composition is ~450 bytes; one carrying a 768-float `embedding` on an intention is ~15KB. So
`open_meta` is not a poor fit for artifacts — it is mechanically unable to hold them.

### 2. Bodies shred fenced data, at its own comment lines

`temper-ingest`'s chunker has **no fence-state tracking**. `collect_sections_with_stack` applies
`heading_re()` (`^(#{1,6})\s+(.+)$`) line by line with no awareness of ``` delimiters, so a YAML
comment at column 0 is parsed as a markdown heading. Executed probe (throwaway, removed):

Input — a 12-line YAML fence under a `## Measurement run` heading. Output of `chunk_markdown`:

```
chunk 0 | header_path="Measurement run"  depth=2 → "Here is the output.\n\n```yaml"
chunk 1 | header_path="candidate weights" depth=1 → "w_express: 0.4\nw_contains: 0.2"
chunk 2 | header_path="scoring inputs"    depth=1 → "s_telos: 0.9\ns_ref: 0.1\n```\n\nTrailing prose."
```

The document is split at its own comment lines; the opening delimiter is orphaned onto the prose
chunk and the closing one onto the trailing prose; and the comment text is promoted into
`header_path`, becoming a search breadcrumb. Reassembling the original is not reliably possible.

Independently, `MAX_CHARS ≈ 1428` (`MAX_TOKENS × CHARS_PER_TOKEN`), so any fence above ~1.4KB is
split mid-document regardless of comments.

### 3. Each fragment is embedded into the search corpus

Every chunk gets a 768-dim vector and an FTS entry. JSON and YAML fragments are semantic noise in
a corpus built for prose.

## Who this is for

**Agent sessions, plural and separated in time.** The writer is one session's process output; the
reader is a later, unrelated session that must ground its next stage on that output. They cannot
negotiate, share no context, and the reader has only what was stored.

**Why-anchor:** this protects the *next session's* attention. Today that session must locate a
fence inside prose, guess which one, reassemble it from chunks that were split at arbitrary
boundaries, and infer a shape nobody declared.

## Jobs to be done

- **Reproducibility** — a structured value produced by process A is re-read, or re-run, by session
  B to ground stage N+1.
- **Attachment** — the resource genuinely *has* this data as part of what it is.

## The load-bearing constraint: findability is delegated

Data artifacts are **not queryable and are never made queryable**. Their relationship to the
surrounding Temper infrastructure — resources, edges, properties — is what makes them findable.
The graph is the index; the artifact is the payload at the end of it.

This is a design commitment, not a simplification. `kb_properties` already is an effective
key-value JSONB store with predicates and facets, and artifacts deliberately do not compete with
it. Nothing here should ever be searched, embedded, or given a query surface of its own.

## Model

### Ownership is has-many, unconditionally

A resource owns an ordered collection of data artifacts. There is no partial unique index, no
one-live-per-kind rule, no registry-declared cardinality.

Ordering, precedence and revision are **properties the artifact carries about itself**, not
constraints the store enforces. This diverges deliberately from the substrate's assert/fold pattern
for edges and properties, and the reason is decisive: **the store cannot know whether run #2
supersedes run #1.** Only the writer knows. Encoding supersession as a uniqueness constraint would
make the store assert a relationship it has no basis for.

The cost is borne by readers, who must apply the writer's declared intent rather than trusting an
index to have kept only the right row. That cost is paid for by the intent vocabulary below.

### Intent — a closed vocabulary of three terms

`intent` answers exactly one question a reader cannot function without: *given a collection, which
do I take?* It does not carry ordering (precedence does) or timing (timestamps do).

| Term | Reader's contract |
|---|---|
| `current` | Replaces earlier artifacts of its kind. Take the newest `current`. |
| `member` | A peer in a series, not a replacement. Take them all, ordered by precedence, and compare. Measurement runs live here. |
| `pinned` | Never auto-selected. Addressable by explicit reference only. |

The vocabulary is **closed**, and an unrecognized term is refused with the vocabulary named in the
refusal. This mirrors `describe_open_meta`, which is already rendered from one source
(`temper_workflow::schema::describe_open_meta()`) by both a CLI verb and an MCP tool, and which the
`open_meta` shape refusal already points at. The pattern is incumbent; artifacts conform to it
rather than inventing a second style.

**Deferred, declared, not dropped:** a process-role vocabulary (`input` / `output` / `observation`
/ `specification`). Adding a second closed field later is additive; retiring one already in
production data is not. Role and other descriptive metadata are expected to ride on
`kb_properties` rather than on new columns — see below.

**Known weakness, accepted:** `pinned` does double duty ("historically significant, don't bury it"
and "addressed by name only"). Left as one term until usage says otherwise.

### Descriptive metadata rides the existing property machinery

`kb_properties.owner_table` is already polymorphic. Admitting artifacts as an owner kind reuses
assert/fold, predicates, facets and the property vocabulary wholesale for the *small* descriptive
metadata — role labels, tags, process identifiers.

`20260727000030_edge_owned_properties.sql` establishes the exact cost: widen the CHECK, add an arm
to `_property_owner_anchor`, cascade folds. The anchor arm is the substantive part — properties are
homed, so an owner must resolve a home anchor for visibility gating. An artifact resolves one
trivially through its owning resource's `kb_resource_homes` row.

That migration also carries a discipline note adopted here verbatim: unused owner kinds **RAISE
rather than get a speculative branch** — *"an unused arm is state we do not need, and a silent NULL
anchor would be worse than a clear refusal."*

The 2704-byte ceiling is irrelevant for a role label and disqualifying for a payload. That split is
the whole point: **big payload in the artifact, small metadata in properties.**

### The kind registry

Artifact kinds are registered in a **new first-class table**, sibling in spirit to
`kb_event_types`: a kind name, its JSON Schema, a schema version, and the governing
`asserted_by_event_id`. A registered schema never enters the chunk/embed pipeline and never appears
in search.

**Registry tenancy is undecided and must not be read as settled.** `kb_event_types.name` is
globally UNIQUE with no scoping, which is adequate for a closed system-defined set and is *not*
adequate for a registry callers extend — two teams both wanting a `query-plan` kind is the
immediate collision. Whether kinds are global, team-scoped, or context-scoped, and who may
register one, is an open question carried in the remainder below.

Rejected: making a schema a resource (`doc_type: schema`). It would inherit visibility, RBAC, edges
and supersession for free, but `body_storage` is *derived* from block coverage
(`_recompute_body_storage`), never asserted — so a schema-as-resource cannot opt out of being
blocked, chunked, embedded and searched. That is the exact failure mode this design exists to end.

Rejected: table-plus-companion-resource. Two spellings of one thing, which this codebase has been
bitten by before (goal membership: `advances` edge vs `open_meta.witnesses.goal` citation, where a
migration built from one silently missed the other).

### Binding is write-first, bind-later

**An artifact can be persisted with a declared kind name and no schema.** Writing never requires a
prior registration step.

This is a deliberate anti-friction choice with a stated rationale: agents route around tools that
do not help them, and a mandatory pre-registration step would send them straight back to fenced
code blocks — the failure this design exists to end. It also has an incumbent precedent:
`kb_event_types.payload_schema` is nullable, documented *"NULL = unregistered/permissive —
foreign/webhook types may stay NULL."*

**Registering a schema for a kind that already has artifacts triggers a validation sweep.** Every
existing artifact of that kind receives a recorded verdict — conformant, non-conformant, or
unchecked. Non-conformant artifacts are **marked, never destroyed or mutated**. A reader always
knows which of the three it holds.

Rejected: refusing registration when the backlog does not conform (one malformed artifact from
months ago blocks the whole kind). Rejected: prospective-only binding (a kind stays permanently
mixed with no path to consistency). Rejected: validate-on-read with no stored claim (moves cost to
every read, and two readers can disagree if the schema changed between them).

### Replay: an incumbent pattern, not a new category

**This section supersedes an earlier draft that claimed data artifacts needed a third replay
category. That claim was wrong** — it was authored from a partial read of `replay.rs`, and checking
the live code found the pattern already there. Recorded rather than silently edited.

`replay.rs` has three arms today, not two:

1. **Byte-reconstructible from the ledger** — everything in `PROJECTION_DUMPS`, diffed
   byte-identically after replay into a fresh namespace.
2. **Re-materializable from inputs** — region tables, *"their proof is re-materialization — the
   membership fingerprint must equal the one recorded in the `region_materialized` payload."*
3. **Projected in Rust inside the acting transaction, never rebuilt** — `kb_subscription_deliveries`
   and webhook intake, which get no-op arms with a stated warrant: a rebuild *"would silently
   reproject today's declarations onto yesterday's events."*

Data artifacts need none of these, because the **metadata/content split already solves it**.

`kb_block_content` is the model, and should be followed rather than paraphrased:

```sql
CREATE TABLE kb_block_content (
    block_revision_id uuid PRIMARY KEY REFERENCES kb_block_revisions(id) ON DELETE CASCADE,
    content      text NOT NULL,
    content_hash text NOT NULL   -- bare sha256 hex of content's raw bytes (Rust `sha256_hex` twin)
);
```

It sits **in** `PROJECTION_DUMPS` and is byte-diffed (surrogate key masked, ordered by
`content_hash, content`), and replay obtains the bytes not by reconstructing them from an event
payload but by **re-supplying them as a sidecar input** — `replay.rs:265`, *"Re-supply the
`__blocks` sidecar (verbatim block bytes, PR 3) from kb_block_content."*

So for artifacts:

- The **metadata row** — which resource owns it, its family, selection intent, precedence,
  timestamps, governing event ids, and the **content hash** — is entirely payload-derivable, goes
  into `PROJECTION_DUMPS`, and diffs byte-identically like any other projection. Identity is
  payload-carried (`identity-as-input`, as `property_set` already does with `property_id`), so ids
  reproduce across replay.
- The **content bytes** live in a companion table shaped like `kb_block_content`, ride the sidecar,
  and are proved by hash.

The event payload therefore carries the hash and not the body — which is what replay's purpose
demands. Replay for the ledger is **provenance**: resources are the replayable-difference core, and
artifacts are event-sourced for governance and consistency. Nothing about this requires a new
category, and the design should not claim one.

Consequence unchanged: the ledger stays light regardless of artifact size, and replay fidelity
imposes no size ceiling.

## Invariants — the negative face

Standing regression boundaries, not acceptance criteria.

1. **An artifact never enters the search corpus.** Not chunked, not embedded, not in FTS. This must
   be standing, because the helpful-seeming future change is precisely "why don't we index artifact
   content too," which reintroduces the shredding described above.
2. **An artifact is never visible to a principal who cannot read its owning resource.** It inherits
   the resource's read gate exactly and never widens it. Noted risk: the
   `array_agg` over an empty scope returning NULL and falling open is a live scar in this codebase,
   and a new read path is exactly where it recurs.
3. **An unvalidated artifact never reads as validated.** Absence of a conformance verdict is not a
   pass.
4. **Registering a schema never destroys or mutates an existing artifact.**
5. **The store never asserts a supersession the writer did not declare.** No index, no heuristic,
   no "newest wins" inferred by the store.

## Refusals

Well-formed acts the system declines, and says why.

- An unrecognized `intent` — refused, naming the three terms, `describe-open-meta`-style.
- A write against a bound kind whose payload does not conform — refused with the validation error.

## Out of scope

**Rejected:**
- Making artifacts queryable, searchable, or embeddable. Findability is delegated to the graph, by
  design, permanently.
- Storing artifact payloads in `kb_properties`. Mechanically impossible above 2704 bytes.
- Schema-as-resource, and table-plus-companion-resource. See above.
- Registry-declared or index-enforced cardinality. The store cannot know supersession.

**Deferred:**
- A process-role vocabulary as a second closed field.
- Owner kinds beyond `kb_resources` (edges, cogmaps, blocks). Admitted by nothing until a caller
  exists; per the incumbent discipline, an unused arm RAISEs rather than being speculatively built.
- Whether artifact revision should express supersession through `kb_events."references"`. The
  `RefRel::Supersedes` variant is documented *"RESERVED AND UNCLAIMED — retired 2026-08-19, not
  activated. Written by nothing, and deliberately so"*, and directs any claimant to re-open
  [The ledger as a readable surface — lineage traversal, as-of reads, and effective time](./019f51e3-726b-75e3-ab55-0b80524073f2),
  which is an **active** goal. This design does not claim that variant.

## Named remainder — unexamined

**Registry tenancy and registration authority.** Whether an artifact kind is global,
team-scoped or context-scoped, and which principal may register or amend one, was raised in
design and never resolved. It is not a detail: it decides collision behaviour between tenants and
the RBAC surface of the registry, and an open registry cannot ship without an answer. Unexamined.

**Volume, size and write-rate are unmeasured.** No distribution of real artifact sizes or write
frequencies from the enterprise usage has been gathered. Per the carried closure scar — *"state
which axes you close over, and name rate-shaped axes as open unless you have explicitly enumerated
them"* — these axes are **open**. The hash-in-payload decision removes replay fidelity as a reason
for a size ceiling; it does not establish that no ceiling is needed for other reasons (retrieval
latency, transfer cost, storage). Unexamined, not excluded.

## Exercise status

**Nothing here has run.** Neither axis: no trigger has fired and no work has executed. The
*existing practice* (fenced blocks in enterprise) has run extensively — that is the prior that
motivates this, not evidence that any of this design has been exercised.
