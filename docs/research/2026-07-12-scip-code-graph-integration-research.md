# SCIP Code-Graph Integration — Research & Architecture

**Date:** 2026-07-12
**Status:** Research complete; design proposed; not yet approved for implementation
**Scope:** Establish the reference frame (SCIP), assess what Temper would need to build to support a
native code-graph, and stake out where the code-graph is a **different thing-in-kind** from the
resource/edge knowledge graph — while remaining faithful to the event-ledger substrate and the
"no view from nowhere" perspective.

> This document is the substance of a proposed new **goal**: *Give Temper a native, event-sourced
> code-intelligence graph, sourced from SCIP, that agents can navigate with compiler-grade precision
> without polluting the curated knowledge graph.* §11 states the goal and a phased roadmap.

---

## 0. Thesis (read this first)

**SCIP and Temper's resource graph are different in kind, and the design must keep them different in
kind while letting them cite each other.**

- A **SCIP index** is a *mechanically-generated, commit-pinned, closed-ontology projection of source
  code at one commit*. Its identity model is **structural** (a symbol *is* its fully-qualified AST
  path, encoded as a string), its ontology is **fixed** (a closed set of symbol kinds, four
  relationship flags, a role bitset), and it is **disposable** — the source tree is the source of
  truth and the index is regenerated wholesale per commit.
- Temper's **resource graph** is a *curated / accreted / steward-tended, open-vocabulary,
  human-and-agent-authored knowledge graph*. Its identity model is **assigned** (UUIDv7 surrogates),
  its edge vocabulary is **open** (four structural `edge_kind`s + free-text `label`s + salience
  weights), and the **event ledger is the truth** — resources accrete and are never regenerated from
  a lower source.

Forcing code symbols into `kb_resources` and code edges into `kb_edges` would be the wrong tool for
the job: it would drown a ~1,600-resource curated corpus under millions of mechanical rows, mis-apply
the assigned-identity model to structurally-identified entities, force mechanical edge kinds into an
enum tuned for affinity clustering, and feed exact code structure to a region producer built for
fuzzy attention-weighted salience. The user's framing is correct: **SCIP is the right reference frame,
but code references do not belong in the current resource/edge machinery.**

The proposal, therefore, is a **sibling projection family** — a distinct `kb_code_*` set of tables,
event types, projectors, and read functions — that:

1. **Reuses the substrate kernel wholesale**: the `kb_events` ledger, the append-event-then-project
   pattern, CAS blob storage, replay/drop-rebuild, contexts-as-home + team-DAG authz, machine
   principals, and (optionally) embeddings/FTS for "find similar code."
2. **Builds a genuinely new structural layer**: commit-pinned index snapshots, string-keyed symbol
   identity, an occurrence table sized for millions of rows, and code-navigation reads
   (go-to-definition, find-references, find-implementations, blast-radius) that are exact traversals,
   **not** the affinity/region clusterer and **not** `graph_traverse`.
3. **Touches the curated graph only through citation** — a curated resource cites a stable symbol
   string; the two graphs are never merged into one edge set.

The rest of this document justifies and details each of these.

---

## 1. The reference frame: what SCIP is

> Authoritative source is `scip.proto` (the protobuf schema, ~962 lines) in **`github.com/sourcegraph/scip`**.
> The docs site is `scip-code.org`. (SCIP was designed at Sourcegraph as the successor to LSIF —
> typed Protobuf instead of untyped JSON graph, ~8× smaller, ~3× faster to process.) A copy of the
> proto is checked into the research scratch during this study.

### 1.1 The data model

The containment hierarchy is strictly nested — **one `Index` per repository-at-a-commit**:

```
Index
 ├── metadata: Metadata                     (1 per index: tool_info, project_root, text encoding, version)
 ├── documents: []Document                  (1 per source file)
 │    ├── occurrences: []Occurrence         (every symbol appearance in the file)
 │    └── symbols: []SymbolInformation      (symbols DEFINED in this file)
 └── external_symbols: []SymbolInformation  (symbols referenced but defined outside this index)
```

- **`Metadata`** — `version` (ProtocolVersion), `tool_info` (name/version/arguments of the indexer),
  `project_root` (URI anchoring all `relative_path`s), `text_document_encoding`.
- **`Document`** — `relative_path`, `occurrences[]`, `symbols[]`, `language`, optional full `text`,
  `position_encoding` (UTF-8/16/32 offset semantics — SCIP's fix for LSP/LSIF's UTF-16 ambiguity).
- **`SymbolInformation`** — metadata *about* a symbol: `symbol` (the symbol string), `documentation[]`
  (markdown), `relationships[]`, `kind` (80+ language-aware categories: `Class`, `Function`, `Method`,
  `Interface`, …), `display_name`, `signature_documentation`, `enclosing_symbol`.
- **`Occurrence`** — a single appearance of a symbol at a range: `range` (packed int32, half-open
  `[start,end)`), `symbol` (the string), `symbol_roles` (a **bitset**), `syntax_kind` (semantic
  highlighting class), `override_documentation`, `diagnostics[]`, plus typed range variants
  (`single_line_range`/`multi_line_range`) and enclosing-range fields.

### 1.2 Symbol strings — identity is structural, not assigned

A symbol is a URI-like standardized string. The grammar (verbatim from `scip.proto`):

```
<symbol>      ::= <scheme> ' ' <package> ' ' (<descriptor>)+ | 'local ' <local-id>
<package>     ::= <manager> ' ' <package-name> ' ' <version>
<descriptor>  ::= <namespace> '/' | <type> '#' | <term> '.' | <method> '(' <disambiguator>? ').'
               |  <type-parameter> '[' ']' | <parameter> '(' ')' | <meta> ':' | <macro> '!'
```

- The descriptor list "should together form a fully qualified name … a unique identifier across the
  package … one descriptor for every node in the AST between the root of the file and the node."
  **The symbol string *is* the AST path.** The suffix character encodes each segment's kind
  (`/`=namespace, `#`=type, `.`=term, `().`=method, `[]`=type-param, `()`=param, `:`=meta, `!`=macro).
- **Global symbols** (`<scheme> <package> <descriptors>`) resolve across documents and across indexes.
- **Local symbols** (`local <id>`) are document-scoped; their `enclosing_symbol` names the global
  parent.

Examples:

```
scip-typescript npm @types/node 18.0.0 fs/readFileSync().
scip-java maven com.google.guava/guava 31.0 com/google/common/collect/ImmutableList#of().
rust-analyzer cargo std 1.65.0 …
local 4
```

The consequence that drives our design: **two indexers (or two commits) naming the same code element
produce the same string by construction.** Cross-index joins are *string equality*, not entity
resolution. There are no opaque surrogate node IDs to reconcile.

### 1.3 Occurrences, roles, relationships

- The **reference-vs-definition distinction is carried entirely by the `symbol_roles` bitset** —
  there is no separate message type. `Definition=0x1`, `Import=0x2`, `WriteAccess=0x4`,
  `ReadAccess=0x8`, `Generated=0x10`, `Test=0x20`, `ForwardDefinition=0x40`. Go-to-definition = the
  occurrence of the symbol with the `Definition` bit; find-references = all occurrences of the symbol
  string.
- **`Relationship`** (on `SymbolInformation`) expresses cross-symbol edges with four booleans:
  `is_reference`, `is_implementation`, `is_type_definition`, `is_definition`. Inheritance /
  interface-implementation / type-of are *all* expressed through this one message via flag
  combinations — there is no separate typed "extends" vs "implements" edge. The flags are defined by
  *which navigation query they feed*, and they are stored on the subtype pointing at the supertype;
  consumers materialize the inverse.

### 1.4 How SCIP is produced and consumed

- **Producers** are per-language indexers, each a separate tool that emits a `.scip` protobuf for a
  repo at a commit: `scip-typescript`, `scip-java` (Java/Scala/Kotlin), `scip-python`, `scip-clang`,
  `scip-ruby`, `scip-dotnet`, `scip-php`; **`rust-analyzer scip`** emits natively. **Temper does not
  produce SCIP — it consumes it.**
- The **`scip` CLI** operates *on* indexes (`lint`, `print`, `snapshot`, `test`, `stats`,
  experimental `expt-convert` to SQLite). Useful for our validation/golden tests.
- **Consumers** (Sourcegraph et al.) serve go-to-definition, find-references, find-implementations,
  go-to-type-definition, hover (from `documentation`/`signature_documentation`), and semantic
  highlighting (from `syntax_kind`). Cross-repo navigation works because symbol strings are globally
  unique and `external_symbols` carries out-of-index metadata.

### 1.5 Scale & lifecycle characteristics

- **Whole-repo, commit-pinned artifact.** An index is generated for a repo at a specific commit; the
  prevailing model is **whole-repo re-indexing per commit / per CI run**, not incremental patching.
- **Size.** Tens-to-hundreds of MB for large repos; occurrences dominate the payload.
- **Staleness.** An index is valid only for its commit. When a file changes, occurrences must be
  re-derived; authoritative navigation requires a fresh index. `Metadata.version` versions the
  *format*; *content* is versioned externally by the commit SHA.

### 1.6 Why SCIP is "different in kind" — side-by-side

| Axis | SCIP code-graph | Temper resource/edge graph |
|---|---|---|
| **Origin** | Mechanically generated by a compiler-grade indexer; deterministic | Human/agent authored; curated, accreted, steward-tended |
| **Identity** | Structural — the symbol string *is* the AST path; joins by string equality | Assigned — UUIDv7 surrogate; joins by id resolution |
| **Ontology** | Closed & fixed — symbol `Kind` enum, 4 relationship bools, role bitset | Open — 4 structural `edge_kind`s + **free-text `label`** + weights |
| **Truth locus** | Source tree is truth; the index is a disposable projection | The `kb_events` ledger *is* truth; resources are the durable store |
| **Lifecycle** | Regenerated wholesale per commit; commit-pinned & disposable | Accretes over time; non-destructive fold, never regenerated |
| **Weighting** | None — every fact is equally, mechanically true | Salience-weighted; telos-relative; region-clustered |
| **Purpose** | Answer 4–5 precise navigation queries | Situated understanding, wayfinding, decision memory |

This table is the load-bearing justification for a sibling architecture rather than an extension.

---

## 2. The central architectural claim

> **A SCIP index is itself a "view from somewhere": the code as observed from commit `C`, by
> indexer tool `T`, at ingest time `t`. So a code-graph belongs in Temper not as a floating
> "current state of the code," but as an event-sourced accumulation of attributed, commit-pinned
> observations — projected into queryable structural tables.**

This is the bridge between "code-graph is different in kind" and "stay faithful to the event ledger
and no view from nowhere." The substrate already models exactly this shape for everything else:
truth is an append-only ledger of attributed acts (`kb_events`), and every queryable structure is a
**projection** rebuildable by replay (`docs/event-sourced-architecture-design.md`; canonical schema
`migrations/20260624000001_canonical_schema.sql:465-506`). We apply the same shape to code
intelligence:

- **The act of ingesting a SCIP index is an event** (`code_index_ingested`), emitted by a specific
  actor (`emitter_entity_id` — the CI indexer machine principal), at `occurred_at`, carrying the
  index's `(repo-context, commit_sha, tool, tool_version)` and a CAS hash of the `.scip` blob.
- **The symbol / document / occurrence / relationship tables are projections** of accumulated
  `code_index_ingested` events. Drop them, replay the events (re-reading blobs from CAS), and rebuild
  identically — the same invariant the substrate proves for its own projections
  (`crates/temper-substrate/tests/replay_roundtrip.rs`).
- **There is no single canonical code graph.** A read resolves a *vantage*: "as of which commit /
  which index." Default is the latest index for the repo's default branch, but the model natively
  supports querying any commit's index and **diffing across indexes** (a PR's blast radius). Two
  branches' indexes coexist; neither is canonical — the code-graph analog of "two teams weight the
  same artifact differently and both are right" (`docs/cognitive-maps/05-how-maps-relate.md:32-42`).
  Here it is "two commits observe the same symbol differently, and both are true observations."

So the code-graph is *different in kind* (structural, closed-ontology, disposable projection) while
*mechanically the same substrate* (attributed events → rebuildable projections, deny-is-zero-rows,
additive-migration deploy discipline). That duality is the whole design.

---

## 3. Data architecture (research question 1)

A distinct `kb_code_*` projection family. All tables carry event lineage
(`ingested_by_event_id`, and where mutable, `last_event_id` + `is_superseded`) exactly like the
existing projections (`asserted_by_event_id`/`last_event_id`/`is_folded` on `kb_edges`,
`canonical_schema.sql:640-642`).

### 3.1 `kb_code_indexes` — the "view from a commit" (the vantage row)

One row per ingested SCIP index. **This is the perspective anchor**: every code fact is attributed to
exactly one index, and thereby to a `(repo, commit, tool)` vantage.

```
kb_code_indexes(
  id                 UUID PK DEFAULT uuid_generate_v7(),
  context_id         UUID NOT NULL → kb_contexts(id),     -- the repo maps to a context (authz home)
  commit_sha         TEXT NOT NULL,                        -- the pin
  ref_name           TEXT,                                 -- branch/tag the commit was tipped on (nullable)
  tool_name          TEXT NOT NULL,                        -- Metadata.tool_info.name  (scip-typescript, rust-analyzer …)
  tool_version       TEXT,
  project_root       TEXT NOT NULL,                        -- Metadata.project_root
  text_encoding      SMALLINT NOT NULL,                    -- Metadata.text_document_encoding
  blob_hash          TEXT NOT NULL,                        -- CAS hash of the .scip protobuf (payload carries this)
  document_count     INT NOT NULL,
  symbol_count       INT NOT NULL,
  occurrence_count   BIGINT NOT NULL,
  ingested_by_event_id UUID NOT NULL → kb_events(id),
  is_superseded      BOOLEAN NOT NULL DEFAULT false,       -- a newer index for the same (context, ref) exists
  occurred_at        TIMESTAMPTZ NOT NULL,                 -- from the event, replay-stable (never now())
  UNIQUE (context_id, commit_sha, tool_name)               -- one index per (repo, commit, tool)
)
```

`context_id` is the seam to Temper authz (§7.1). `is_superseded` is a projection-maintained pointer,
not truth — truth is "a later `code_index_ingested` event exists for the same `(context, ref)`."

### 3.2 `kb_code_symbols` — the string-keyed symbol dictionary

The global symbol table, **keyed by the SCIP symbol string** (natural key preserved). Symbols are
deduplicated across indexes — the same `ImmutableList#of().` observed by 50 commits is one row.

```
kb_code_symbols(
  id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,  -- interned surrogate, STORAGE ONLY
  symbol_string TEXT NOT NULL UNIQUE,        -- the identity; joins are on this
  scheme        TEXT NOT NULL,
  manager       TEXT,                          -- '.' placeholder normalized to NULL
  package_name  TEXT,
  package_version TEXT,
  kind          SMALLINT,                      -- SymbolInformation.Kind (latest observation wins)
  display_name  TEXT,
  is_local      BOOLEAN NOT NULL DEFAULT false -- 'local <id>' symbols are per-document (see 3.3)
)
```

> **Identity nuance.** `symbol_string` is the identity (unique index); the `BIGINT id` is a storage
> optimization — an *interned* surrogate so the multi-million-row occurrence table stores an 8-byte
> FK instead of repeating a long string. This is **not** the assigned-identity model of
> `kb_resources` (UUIDv7 with semantic meaning); it is a dictionary intern key, and it never leaves
> the code-graph internals. External surfaces address symbols by string. Local symbols
> (`local 4`) are only unique within a `(index, document)` and are stored per-document, not interned
> globally.

`documentation` and `signature_documentation` are per-observation (a docstring can change across
commits), so they live on an index-scoped table (`kb_code_symbol_info`, keyed by
`(index_id, symbol_id)`), not on the dedup dictionary — mirroring how `kb_block_provenance` accretes
per-observation while `kb_resources` holds stable identity.

### 3.3 `kb_code_documents` — per (index, file)

```
kb_code_documents(
  id            UUID PK,
  index_id      UUID NOT NULL → kb_code_indexes(id) ON DELETE CASCADE,
  relative_path TEXT NOT NULL,
  language      TEXT,
  UNIQUE (index_id, relative_path)
)
```

Optionally, a document may be *linked* to a `kb_resources` row if the file has also been ingested as a
knowledge resource (§7.3) — but that link is a citation, not a merge.

### 3.4 `kb_code_occurrences` — the big table

Every symbol appearance. This is by far the largest table (millions of rows per large repo per
commit) and is the reason the code-graph must be architecturally distinct — it cannot be interleaved
with the curated corpus.

```
kb_code_occurrences(
  id             BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
  index_id       UUID NOT NULL → kb_code_indexes(id) ON DELETE CASCADE,
  document_id    UUID NOT NULL → kb_code_documents(id) ON DELETE CASCADE,
  symbol_id      BIGINT NOT NULL → kb_code_symbols(id),   -- interned; join to string via dictionary
  start_line     INT NOT NULL,
  start_char     INT NOT NULL,
  end_line       INT NOT NULL,
  end_char       INT NOT NULL,
  symbol_roles   INT NOT NULL,                            -- the SCIP bitset (Definition/Import/Read/Write/…)
  syntax_kind    SMALLINT
)
-- indexes: (symbol_id) WHERE symbol_roles & 1 = 1   -- fast go-to-definition
--          (symbol_id)                              -- fast find-references
--          (document_id, start_line)                -- fast position→symbol lookup (hover)
--          (index_id)                               -- fast per-index GC/rebuild
```

Partitioning by `index_id` (or by `context_id` via the index) is on the table from day one so old
indexes can be detached/pruned cheaply (§9).

### 3.5 `kb_code_relationships` — the closed-ontology edge table

```
kb_code_relationships(
  id                UUID PK,
  index_id          UUID NOT NULL → kb_code_indexes(id) ON DELETE CASCADE,
  from_symbol_id    BIGINT NOT NULL → kb_code_symbols(id),   -- the subtype/implementer (SCIP stores it here)
  to_symbol_id      BIGINT NOT NULL → kb_code_symbols(id),   -- the supertype/interface/type
  is_reference       BOOLEAN NOT NULL DEFAULT false,
  is_implementation  BOOLEAN NOT NULL DEFAULT false,
  is_type_definition BOOLEAN NOT NULL DEFAULT false,
  is_definition      BOOLEAN NOT NULL DEFAULT false,
  UNIQUE (index_id, from_symbol_id, to_symbol_id)
)
```

Note the ontology is **closed and boolean-flagged**, matching SCIP exactly. This is deliberately
*not* modeled as `kb_edges` rows (which carry an open `label` and an affinity `weight` and are
consumed by the region clusterer). The two edge tables are different in kind — see §7 and §8.

### 3.6 What we explicitly do NOT do

- We do **not** add code symbols to `kb_resources`.
- We do **not** add code edges to `kb_edges`, and we do **not** widen the `edge_kind` enum
  (`express/contains/leads_to/near`, `canonical_schema.sql:95`) with mechanical kinds like `calls`
  or `imports`.
- We do **not** run the region/lens/salience producer over code structure.
- We do **not** mint UUIDv7 identities for symbols — identity stays the string.

---

## 4. Event ledger integration & "no view from nowhere" (research question 4)

### 4.1 New event types (additive, per the deploy discipline)

Following the strict-registration pattern — `_event_append` rejects any unseeded event-type name
(`migrations/20260624000002_canonical_functions.sql:765-787`) — we add, in a new additive migration:

| Event type | Payload (typed struct, never `json!()`) | Projector |
|---|---|---|
| `code_index_ingested` | `{ context_id, commit_sha, ref_name, tool_name, tool_version, project_root, text_encoding, blob_hash, counts }` | `_project_code_index_ingested` |
| `code_index_superseded` | `{ index_id }` (flips `is_superseded`, optional GC trigger) | `_project_code_index_superseded` |
| `code_index_pruned` | `{ index_id }` (projection-only GC; the *event* stays, blob may stay in CAS) | `_project_code_index_pruned` |

**One event per index ingest, not one per symbol/occurrence.** The `.scip` blob (tens-to-hundreds of
MB) is stored in CAS; the event payload carries only its `blob_hash` + metadata + counts. The
projector reads the blob, decodes the protobuf, and fan-out-expands documents → occurrences → symbols
→ relationships into the `kb_code_*` tables in one transaction. This mirrors exactly how block content
is CAS-referenced by manifest/hash in payloads rather than inlined
(`crates/temper-substrate/src/replay.rs` reconstructs chunk prose + embeddings from the CAS during
replay; payloads carry manifests/hashes, never prose).

New `kb_event_types` rows + payload schemas + `_project_*` functions ship in one additive migration.
`kb_events` itself is unchanged — this is the same carry-over pattern the design doc describes for
adding new event names (`docs/event-sourced-architecture-design.md:315-320`).

### 4.2 Replay & the drop-rebuild invariant

Because the code-graph tables are pure projections of `code_index_ingested` events (+ CAS blobs),
they inherit the substrate's central guarantee: **drop the `kb_code_*` tables, replay the events,
rebuild byte-identically.** We extend the replay harness so the code-graph projections are dumped and
diffed like the others (`replay.rs` `dump_projections` / masked-surrogate rule for reference-free
surrogate ids — `kb_code_symbols.id` and `kb_code_occurrences.id` are exactly such masked surrogates,
diffed with `id` masked, ordered by natural key: `symbol_string`, then `(document, range, symbol)`).
The `.scip` blobs join the CAS sidecars that `snapshot()` already captures.

### 4.3 Attribution — the code graph is authored by somebody

Every `code_index_ingested` event carries `emitter_entity_id` — never a bare profile
(`canonical_schema.sql:455-458`). The emitter is the **CI indexer machine principal**: a
`kb_machine_clients` allowlist entry (`migrations/20260711000010_machine_clients.sql`) for the bot
that runs `scip-typescript` / `rust-analyzer scip` in the pipeline. This is the concrete, schema-level
form of "no view from nowhere" for code: *the graph knows which tool, run by which registered
principal, produced these facts at which commit and when.* `invocation_id` optionally threads the CI
run; `correlation_id` groups a multi-tool ingest (TS + Rust indexes for the same commit).

### 4.4 "No view from nowhere" as a query contract

The philosophy (`docs/cognitive-maps/05-how-maps-relate.md:37`; enforced as *deny = zero rows, never
an error* across the read functions, `crates/temper-services/src/backend/substrate_read.rs:366`)
lands on the code-graph as two hard rules:

1. **Every code-navigation read resolves a vantage.** There is no "the definition of `X`" in the
   abstract — only "the definition of `X` as of index `I` (commit `C`)." The API default is
   *latest non-superseded index on the context's default branch*, but the vantage is always explicit
   in the resolved query, and any commit's index is addressable. Cross-vantage **diff** is a
   first-class read (blast radius of a PR = symbols whose occurrence set differs between the base
   index and the head index).
2. **Visibility gates through the repo's context.** A code index is homed to a `kb_contexts` row;
   its facts are visible exactly to who can read that context (`contexts_readable_by(profile)`,
   `migrations/20260712000010_context_read_predicates.sql:84-124`). An unauthorized reader gets zero
   rows, never a 403 — the operational form of the principle, identical to the resource graph.

---

## 5. Tooling & ingest mechanics (research question 2)

### 5.1 Production is external; Temper consumes

Temper does not generate SCIP. The index is produced in the repo's CI by the language-appropriate
indexer (`scip-typescript`, `rust-analyzer scip`, `scip-python`, …) and uploaded to Temper. For this
very monorepo, that means `rust-analyzer scip` over the Rust workspace and `scip-typescript` over
`packages/`.

### 5.2 The ingest path

```
CI job (per commit / per push)
  └─ run indexer → index.scip (protobuf blob, 10s–100s MB)
       └─ upload to Temper:
            • CLI:  temper code index --repo @me/temper --commit <sha> --ref main index.scip
            • API:  POST /api/code/index   (multipart or segmented upload of the blob)
                 └─ blob → CAS (blob_hash), fire `code_index_ingested`
                      └─ projector decodes proto, expands into kb_code_* (one txn)
```

Because indexes are large, the upload reuses the **streaming/segmented upload** mechanics already
built for resource bodies (`ingest_begin` / `ingest_append` / `ingest_finalize`,
`crates/temper-mcp/src/tools/ingest.rs`; `migrations/20260708000012_streaming_ingest.sql`) or Vercel
Blob for the raw `.scip` — the blob is opaque bytes to the uploader; only the projector decodes it.
Upload is **idempotent on `blob_hash`**: re-uploading the same index for the same
`(context, commit, tool)` is a no-op (the `UNIQUE` constraint in §3.1 + hash check), mirroring
`block_append`'s idempotency.

### 5.3 The decoder

A new Rust module (proposed crate **`temper-scip`**, or a `scip` feature-module in `temper-ingest`)
decodes `scip.proto` via **`prost`** into typed structs (no `serde_json`), validates
(`scheme`/`package`/descriptor grammar; range well-formedness), parses symbol strings into their
components for the dictionary, and hands a structured `DecodedIndex` to the projector. The `scip`
CLI's `snapshot`/`test` golden files become our fixture corpus for round-trip tests (a small `.scip`
fixture → ingest → assert occurrences/definitions match the golden snapshot).

### 5.4 Incremental reality & retention

SCIP is whole-repo per commit, so **each ingest is a full index for one commit** — there is no
in-place patching. Retention is therefore a projection policy, not a truth question:

- Keep materialized `kb_code_*` rows for a bounded working set: default-branch tip + open-PR head
  commits (configurable per context).
- When an index is superseded and falls out of the working set, fire `code_index_pruned` — the
  projector detaches that index's partition (fast, thanks to §3.4 partitioning). **The
  `code_index_ingested` event and the CAS blob remain**, so any pruned index is one replay away from
  rehydration ("projection is a rebuildable cache, never the truth"). This is a retention *policy*
  and must `log`/record what it dropped (no silent truncation).

---

## 6. Read surface — code navigation for agents

The payoff: agents get **compiler-grade code navigation grounded in the same authz + provenance
fabric as the knowledge base.** These are a *new read family*, peer to `graph_traverse` /
`search_graph_expand` (`canonical_functions.sql:1308`), **not** reuses of them (those gate on
resource visibility over `kb_edges`; code reads gate on context visibility over `kb_code_*`).

Proposed reads (SQL functions + MCP tools + CLI), each taking an explicit-or-defaulted vantage
`(context, commit|index)`:

| Read | Definition |
|---|---|
| `code_definition(symbol, vantage)` | occurrence(s) of `symbol` with the `Definition` role bit |
| `code_references(symbol, vantage)` | all occurrences of `symbol`, expanded via `is_reference` relationships |
| `code_implementations(symbol, vantage)` | via `is_implementation` relationships |
| `code_type_definition(symbol, vantage)` | via `is_type_definition` |
| `code_hover(file, line, char, vantage)` | position → symbol → `SymbolInformation` (docs/signature) |
| `code_blast_radius(symbol, base, head)` | cross-index diff: what references change between two commits |

MCP tool names (agent-facing): `code_definition`, `code_references`, `code_implementations`,
`code_hover`, `code_blast_radius`. Each returns typed DTOs (in `temper-core`, `ts-rs`-derived) —
never inline JSON. Deny = empty result set.

Optionally, embed docstrings / symbol signatures as chunks so `unified_search`
(`migrations/20260711000050_search_vector_scope_aware.sql`) answers "find code *like* this" — see
§8. But structural navigation (def/refs/impls) is **exact traversal over `kb_code_*`**, never the
fuzzy vector/region path.

---

## 7. Bridging to contexts, cogmaps, and the curated graph (research question 3)

The two graphs stay distinct and **touch only through citation**. Three concrete seams:

### 7.1 Contexts — the repo is the home & the authz anchor

A repository maps cleanly to a `kb_contexts` row (owner + slug + team-sharing,
`canonical_schema.sql:159-168`). `kb_code_indexes.context_id` points at it, so the code-graph inherits
the full Temper access model for free: `contexts_readable_by` for reads,
`context_authorable_by_profile` for who may upload indexes (the machine principal must be authorable
in that context). No new authz machinery. This is the single biggest reuse.

### 7.2 The curated resource graph — citation, not merge

A curated resource (a `decision`, `research`, `task`, `concern`, …) can **cite a code symbol** by its
stable symbol string. This is the one place the worlds connect, and it belongs in the *curated* graph
because the link is human/agent-authored and opinionated:

- **Mechanism A (reuse span-locators + provenance — zero new schema).** The annotate-only provenance
  path (issue #355, `migrations/20260710000001_block_provenance_annotate.sql`; MCP `annotate_resource`)
  already lets a resource's block cite a remote source by URI with a fragment
  (`file.rs#L120-L180`) *without re-embedding*. Extend the URI convention to carry a **symbol string**
  (`scip://<context>/<symbol-string>` or `repo://…/file.rs#L120-L180`). A decision "we chose the
  ring-buffer allocator" cites the exact symbol. The fragment round-trips verbatim through reads
  (`normalize_remote_uri` preserves it), so distinct symbols are distinct provenance rows. **This is
  the recommended default seam — it exists today.**
- **Mechanism B (a curated edge with an open label — optional).** If a first-class, queryable
  resource↔symbol link is wanted, add a narrow `kb_code_citations(resource_id, symbol_string,
  label, asserted_by_event_id)` table (curated, event-sourced, open `label` like `implements` /
  `documented_by` / `concerns`). This lives on the *curated* side (open vocabulary, authored) with the
  *target* being a stable code-graph symbol string. It is **not** a `kb_edges` row and **not** a
  `kb_code_relationships` row — it is the explicit membrane between the two ontologies.

Either way: **the code graph is never queried through `graph_traverse`, and curated edges never enter
`kb_code_relationships`.** The join is always symbol-string ↔ citation.

### 7.3 Cogmaps — situated meaning over ground-truth structure

A cognitive map is *curated, telos-seeded, steward-tended understanding*
(`docs/cognitive-maps/01-what-a-cognitive-map-is.md`). The code graph is *mechanical ground truth*.
They compose without merging:

- A cogmap region ("the auth subsystem") can be **backed by a set of symbols** via citations (§7.2),
  so an agent orienting in the cogmap can descend to exact code. The cogmap supplies *why this matters
  under our telos*; the code graph supplies *what the code actually is at this commit*.
- We do **not** run the region/lens/salience producer over code edges. Regions are density clusters
  for the *attention economy* — computed from affinity + embedding cosine
  (`crates/temper-substrate/src/affinity.rs`, `write::materialize`). Code structure is exact
  reachability, not affinity; feeding `is_implementation` edges to a salience clusterer is a category
  error. Reuse embeddings for "similar code" (§8), build traversal separately (§6).
- A code index is closer to a **context** (accreted, mechanical) than a **cogmap** (curated, tended).
  It needs no charter, telos, regulation, or promotion machinery. So cogmaps *reference* the code
  graph but do not *contain* it.

---

## 8. Shared primitives vs. build-new (research question 5)

The reuse/build matrix — the direct answer to "where can we rely on shared primitives and where do we
need to build something different."

| Primitive | Verdict | Why |
|---|---|---|
| **`kb_events` ledger + `_event_append` + projector pattern** | **Reuse wholesale** | Code facts become `code_index_ingested` events + projectors; inherit replay, provenance, audit, additive-deploy safety for free. |
| **CAS blob storage + payload-carries-hash** | **Reuse wholesale** | The `.scip` protobuf is a CAS blob; the event payload carries its hash. Exactly the block-content pattern. |
| **`kb_contexts` as home + team-DAG authz** | **Reuse wholesale** | Repo → context; `contexts_readable_by` / `context_authorable_by_profile` gate all code reads/writes. Zero new authz. |
| **`kb_machine_clients`** | **Reuse wholesale** | The CI indexer is a registered machine principal; it authors the ingest events. |
| **Streaming/segmented upload** | **Reuse** | Large `.scip` blobs use `ingest_begin/append/finalize` or Vercel Blob; idempotent on hash. |
| **Replay / drop-rebuild invariant + masked-surrogate diffing** | **Reuse (extend harness)** | `kb_code_*` are pure projections; add them to `dump_projections`; `id` columns are masked surrogates. |
| **Embeddings / FTS / `unified_search`** | **Reuse selectively** | For "find similar code" / docstring search only. Embed docstrings/signatures as chunks. **Not** the path for structural navigation. |
| **Span-locators + annotate-only provenance** | **Reuse & lean on heavily** | The zero-schema URI-fragment convention is *exactly* the resource→symbol citation seam (§7.2). |
| **`kb_edges` (open-label, weighted, homed)** | **Do NOT reuse for code edges** | Its `edge_kind` enum + affinity weight + region-clusterer consumption are wrong for a closed mechanical ontology. Reuse it *only* for the optional curated resource→symbol citation (Mechanism B), never for `calls`/`imports`/`implements` between symbols. |
| **Region / lens / salience / cogmap layer** | **Do NOT reuse; build distinct** | Attention-weighted affinity clustering is a category error over exact code structure. Build code traversal reads as a new family. |
| **`kb_resources` / `parse_ref` / UUIDv7 identity** | **Do NOT reuse for symbols** | Symbol identity is the SCIP string; interning is a storage detail, not assigned identity. |
| **`kb_code_*` tables + `temper-scip` decoder + code-nav reads + commit/index versioning + diff** | **Build new** | The genuinely novel structural layer sized for millions of occurrences and exact navigation. |

**One-line summary:** reuse the *substrate kernel* (ledger, CAS, authz, replay, provenance) and the
*embedding stack for fuzzy code search*; build a new *structural code-graph* (tables, decoder,
traversal reads, commit versioning) and keep it out of the *curated graph's edge/region/cogmap
machinery*, joining the two only by symbol-string citation.

---

## 9. Scale, retention, replay

- **`kb_code_occurrences` is the sizing driver** — millions of rows per large repo per commit.
  Partition by `index_id`/`context` from day one so superseded indexes detach cheaply. This single
  table is sufficient justification for a distinct architecture: it must not be interleaved with the
  ~1,600-row curated corpus or the region compute.
- **Retention is a projection policy, not truth.** Materialize a bounded working set (default-branch
  tip + open-PR heads); `code_index_pruned` detaches the rest. Events + CAS blobs persist, so any
  commit's graph is one replay away. Log what is pruned.
- **Replay** rebuilds every retained index's projection from `code_index_ingested` events + CAS
  blobs, and the round-trip test asserts byte-identity under the masked-surrogate rule.

---

## 10. Risks & open questions

1. **Occurrence-table cost at scale.** Even bounded to a working set, a large monorepo's tip commit
   is a lot of rows. Open: partition granularity, whether to store only definitions + relationships
   eagerly and lazily expand references on demand from the CAS blob.
2. **Local symbols across indexes.** `local <id>` is only unique within a `(index, document)`; the
   dedup dictionary must scope them correctly (§3.2). Getting this wrong corrupts find-references.
3. **Cross-repo / external symbols.** `external_symbols` names symbols defined in *other* repos'
   indexes. Cross-repo navigation requires those repos to also be indexed into Temper (their own
   contexts) and joins by symbol string across contexts — with authz intersection. Scope decision:
   in-repo navigation first; cross-repo later.
4. **Vantage defaulting.** "Latest index on the default branch" needs a crisp definition when
   multiple tools index the same commit (TS + Rust), and when the default branch has no fresh index.
5. **Diff semantics.** `code_blast_radius` across two indexes needs a defined notion of "changed
   occurrence set" that is stable under pure line-shift (a symbol that only moved down 10 lines is not
   a semantic change). SCIP itself is commit-pinned and does not define cross-commit mapping —
   Sourcegraph does approximate range mapping. We must pick: exact-per-commit only, or approximate
   mapping.
6. **Which indexers, which languages, first.** For dogfooding: `rust-analyzer scip` + `scip-typescript`
   over this monorepo.
7. **Membrane discipline.** The value of the whole design depends on *never* letting code edges leak
   into `kb_edges` or code structure into the region producer. This needs an explicit invariant
   (and ideally a test) analogous to the additive-only-on-`main` guard.

---

## 11. Proposed goal & phased roadmap

**Goal (the deliverable this research produces):** *Give Temper a native, event-sourced
code-intelligence graph, sourced from SCIP, that agents can navigate with compiler-grade precision —
built as a sibling projection family on the substrate kernel, kept architecturally distinct from the
curated resource/edge/cogmap graph, and joined to it only by symbol-string citation.*

Suggested phasing (each phase an additive, independently-shippable slice — matching the repo's
wave/phase convention and additive-migration discipline):

- **Phase 0 — Spec & schema.** Ratify §3 tables, §4.1 event types, and the membrane invariant (§7,
  §10.7) as a design doc under `docs/superpowers/specs/`. Decide the §10 open questions (esp.
  occurrence retention, vantage default, diff semantics).
- **Phase 1 — Decoder + ingest.** `temper-scip` crate (prost decode + symbol-string parser +
  validation); `code_index_ingested` event + projector; CAS blob storage; idempotent upload path
  (CLI `temper code index` + `/api/code/index`). Fixture round-trip test from `scip` golden snapshots.
- **Phase 2 — Read surface.** `code_definition` / `code_references` / `code_implementations` /
  `code_hover` SQL + MCP tools + CLI, vantage-resolved and context-gated. Replay round-trip test
  extended to `kb_code_*`.
- **Phase 3 — Bridge.** Symbol-string span-locator citation via annotate-only provenance (§7.2
  Mechanism A); optional `kb_code_citations` (Mechanism B). Cogmap region → symbol backing (§7.3).
- **Phase 4 — Versioning & diff.** Multi-index vantage, supersession, `code_index_pruned` retention
  GC, `code_blast_radius` cross-index diff.
- **Phase 5 (optional) — Fuzzy code search.** Embed docstrings/signatures as chunks; wire into
  `unified_search` for "find similar code," kept strictly separate from structural navigation.
- **Dogfood throughout:** index this monorepo (`rust-analyzer scip` + `scip-typescript`) into its own
  context so agents navigate Temper's own code through Temper.

---

## Appendix — key source citations

**SCIP:** `scip.proto` (github.com/sourcegraph/scip); docs `scip-code.org`. Data model §1.1; symbol
grammar §1.2; roles/relationships §1.3.

**Temper substrate (all verified against `migrations/` — the canonical baseline, not the older
`docs/event-sourced-architecture-design.md` row-shapes):**

- Event ledger: `migrations/20260624000001_canonical_schema.sql:465-506` (append-only trigger
  `:498-506`); strict event-type registration `migrations/20260624000002_canonical_functions.sql:765-787`.
- Projections & mutation/projector pattern: `canonical_functions.sql` `_project_*` family;
  replay + invariant `crates/temper-substrate/src/replay.rs`,
  `crates/temper-substrate/tests/replay_roundtrip.rs`.
- Edges (curated graph): `kb_edges` `canonical_schema.sql:628-650`; `edge_kind` enum `:95`;
  open `label` `:636`; `graph_traverse` `canonical_functions.sql:1308`.
- Contexts & authz: `kb_contexts` `:159-168`; `kb_resource_homes` `:276-285`;
  `contexts_readable_by` / `context_authorable_by_profile`
  `migrations/20260712000010_context_read_predicates.sql:84-124,171-199`.
- Cogmaps & regions: `kb_cogmaps` `:243-251`; region tables `:684-755`; producer
  `crates/temper-substrate/src/{substrate,write,affinity}.rs`;
  `docs/cognitive-maps/*.md`; wayfinding `docs/superpowers/specs/2026-07-11-context-regions-and-wayfinding-design.md`.
- Ingest, blocks, provenance, embeddings, search: `crates/temper-ingest/src/{embed,chunk,pipeline}.rs`;
  streaming ingest `migrations/20260708000012_streaming_ingest.sql`,
  `crates/temper-mcp/src/tools/ingest.rs`; annotate-only provenance + span locators (issue #355)
  `migrations/20260710000001_block_provenance_annotate.sql`,
  `docs/superpowers/specs/2026-07-10-issue-355-annotate-only-provenance-and-span-locators-design.md`;
  `unified_search` `migrations/20260711000050_search_vector_scope_aware.sql`,
  `crates/temper-substrate/src/readback/mod.rs`.
- Machine principals: `migrations/20260711000010_machine_clients.sql`.
- "No view from nowhere": `docs/cognitive-maps/05-how-maps-relate.md:32-42`;
  deny-is-zero-rows `crates/temper-services/src/backend/substrate_read.rs:366`.
- Deploy discipline (additive-only-on-`main`): `DEPLOYING.md:38-65`.
