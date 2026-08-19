# Design: D3 — temper index + graph index + LLM integration

**Date:** 2026-04-16
**Context:** temper
**Goal:** llm-wiki
**Previous:** D1 (temper-llm scaffold), D2 (real provider implementations)
**Status:** Design

---

## Overview

D3 implements the integration layer for LLM-assisted graph indexing:

1. **`temper index`** — builds an HNSW vector index over the vault
2. **`temper graph index`** — 4-phase pipeline: TF-IDF seed extraction → cluster formation → LLM judgment → Concept materialization
3. **Config extension** — `[graph_index]` section in `TemperConfig`, graph-index thresholds in the same file as LLM provider settings
4. **Managed meta fields** — `temper-provenance`, `temper-llm-model`, `temper-llm-run` registered and typed

This design is grounded in the actual codebase. Decisions marked with **RESOLVED** have been verified against existing code.

---

## Phase 1: `temper index`

### What it does

Walks the vault, embeds every markdown file (via `temper-ingest::embed`), builds an HNSW index, and writes it to `.temper/index.bin` with a sidecar `.temper/index.json`.

### Crate layout

`temper-ingest` gains a new `hnsw` feature:

```toml
[features]
hnsw = ["dep:tantivy", "dep:hnsw_rs"]
```

Feature groups for `temper-ingest`:
- `extract` — kreuzberg document extraction
- `embed` — ONNX embedding (ort/ndarray/tokenizers), **implies `embed-download` for temper-cli default**
- `hnsw` — tantivy (TF-IDF) + hnsw_rs (vector index)

`temper-cli` enables all three by default (`default = ["embed", "extract", "hnsw"]`).

`temper-api` does not need the `hnsw` feature — the server-side path is deferred to iteration 2.

### Data model

**Sidecar manifest** (`.temper/index.json`):

```json
{
  "version": 1,
  "run_at": "2026-04-16T...",
  "model": "BAAI/bge-base-en-v1.5",
  "dimension": 768,
  "file_count": 312,
  "files": [
    {
      "rel_path": "@me/temper/task/2026-04-11-something.md",
      "content_hash": "sha256:abc123",
      "mtime_ns": 1713200000000000000,
      "doc_embedding": [0.1, -0.2, ...],   // mean-pool of all chunk embeddings
      "chunks": [
        { "index": 0, "header_path": "", "content_hash": "sha256:def456", "vector_id": 0 },
        { "index": 1, "header_path": "Design > API", "content_hash": "sha256:ghi789", "vector_id": 1 }
      ]
    }
  ]
}
```

- `content_hash` on files and chunks enables incremental indexing: skip files/chunks whose hash hasn't changed since last run
- `doc_embedding` is the **representative chunk** — mean-pool of all chunk vectors in the document, used for cluster formation in Phase 2
- `vector_id` maps each chunk to its position in the HNSW index (`hnsw_rs` uses contiguous integer IDs)

### Index building algorithm

```
1. Load .temper/index.json (if exists) → build set of known (path, hash) pairs
2. Walk vault (same context/owner partitioning as graph_build)
3. For each .md file:
   a. If (rel_path, content_hash) already in known set → skip (no change)
   b. Otherwise: strip frontmatter, chunk via temper-ingest::chunk (already exists)
   c. Embed all chunks via temper-ingest::embed_texts
   d. Compute doc_embedding = mean_pool(all chunk vectors)
   e. For each chunk: hnsw.add_vector(chunk_vector, chunk.vector_id)
   f. Write entry to sidecar
4. hnsw.write_index() → .temper/index.bin
5. Write sidecar → .temper/index.json
```

Incremental: only changed files are re-embedded. Unchanged files are preserved in the existing HNSW index. `hnsw_rs` supports `add_vector` (no need to rebuild from scratch).

`--full` flag forces a full rebuild: delete `.temper/index.bin` and `.temper/index.json`, start fresh.

### Progress indication

Use `indicatif` (already a dep in temper-cli, not yet wired up). Format:
- Per-file progress with filename
- Chunk count per file
- Total files / total chunks counters

### Error handling

- If `.temper/` directory doesn't exist, create it
- If HNSW index write fails, propagate error — do not leave a partial index on disk
- If embedding fails for a file, log to stderr and skip that file (don't abort the whole run)

---

## Phase 2: `temper graph index`

### What it does

Four-phase pipeline that discovers Concept candidates and materializes them as vault resources.

### Phase 2.1 — Seed Extraction

Uses `tantivy` (already added via `hnsw` feature) for TF-IDF:

- Tokenize: lowercase, strip punctuation, split on whitespace
- Stem: Snowball English stemmer via `tantivy::core::stemmer`
- Stopwords: tantivy's built-in English stopword set
- Scoring: `tfidf = (term_freq_in_doc / doc_len) * log(total_docs / docs_with_term)`
- Cross-document frequency filter: phrase must appear in ≥`seed_min_doc_frequency` docs (configurable, default 2)

**Output:** `Vec<SeedPhrase>` where each `SeedPhrase` is `(phrase: String, doc_frequency: usize, top_doc_ids: Vec<DocId>)`

The phrase extraction walks the vault with the same context partitioning as `graph_build`. It reads body text (stripped of frontmatter), uses `tantivy` for tokenization/stemming/scoring, and produces candidate phrases.

### Phase 2.2 — Cluster Formation

For each seed phrase:
1. Embed the phrase using `temper-ingest::embed_texts`
2. HNSW nearest-neighbor search → candidate chunk set
3. Group chunks by document (one doc may contribute multiple chunks)
4. Apply `cluster_similarity_threshold` (cosine similarity cutoff, default 0.70)
5. Apply `cluster_max_members` cap (default 12)
6. If graph hops enabled (`cluster_graph_hop_depth > 0`): query existing Postgres `graph_traverse` function for 1-hop neighbors (only if vault is synced); this adds topologically-related docs that embeddings alone might miss

**Output:** `Vec<Cluster>` where each `Cluster` is `(seed: SeedPhrase, members: Vec<DocId>, member_embeddings: Vec<Vec<f32>>)`

### Phase 2.3 — LLM Judgment

For each cluster, build a prompt:

```
You are analyzing a cluster of documents to determine if they represent
a coherentConcept. A Concept is a named idea, pattern, or domain term
that recurs across multiple documents.

Seed phrase: "{seed_phrase}"

Existing concepts in this context (do not duplicate these):
- concept-slug-1: title
- concept-slug-2: title

Cluster members:
{for each doc in cluster}
- {slug}: {summary}
{/for}

Respond with JSON:
{{
  "is_concept": true/false,
  "slug": "proposed-slug-if-true",
  "title": "Human-readable title if true",
  "body_markdown": "## Members\\n\\n- ...",
  "member_edges": [
    {{"target_slug": "...", "edge_type": "relates-to"}}
  ]
}}
```

LLM is called via `Agent::run` with `max_turns: 1`, no tools registered, and a `response_format` for `ConceptProposal`. This is the single-turn structured output use case the `temper-llm` harness was designed for.

If `is_concept: false` or if the LLM call fails, skip the cluster and log to `.temper/graph-index-errors-{run_id}.log`.

**Output:** `Vec<ConceptProposal>`

### Phase 2.4 — Materialization

For each accepted `ConceptProposal`:
1. Generate `temper-provisional-id` (UUIDv7, pre-sync identifier)
2. Generate `temper-llm-run` (run ID, same for all concepts in this run)
3. Write Concept file at `{vault}/{owner}/{context}/concept/{slug}.md`
4. For each `member` in `member_edges`: add `relates-to: {concept_slug}` to that member's frontmatter `open_meta`

**Transactional per-concept:** if Concept file write succeeds but any member edge write fails, roll back the Concept file (delete it). Don't leave partial state.

**Bidirectional edges:** Concept's own frontmatter also lists all members as `relates-to` entries.

### Configuration

Extends `TemperConfig` with a new `[graph_index]` section (in `crates/temper-core/src/types/config.rs`):

```rust
// temper-core/src/types/config.rs

pub struct GraphIndexConfig {
    // seed extraction
    pub seed_min_doc_frequency: usize,   // default: 2
    pub seed_top_n: usize,               // default: 50

    // cluster formation
    pub cluster_similarity_threshold: f32,  // default: 0.70
    pub cluster_max_members: usize,        // default: 12
    pub cluster_graph_hop_depth: usize,     // default: 0

    // concept acceptance
    pub concept_min_members: usize,        // default: 3
    pub concept_default_edge_type: String,  // default: "relates-to"
}

pub struct TemperConfig {
    pub vault: CloudVaultConfig,
    #[serde(default)]
    pub sync: UnifiedSyncConfig,
    // ... existing sections ...
    #[serde(default)]
    pub llm: LlmConfig,           // already exists, lines 173-202
    #[serde(default)]
    pub graph_index: GraphIndexConfig,  // NEW
}
```

CLI flags (all overridable via `--` prefix, e.g. `--seed-top-n 100`) mirror all config fields for interactive experimentation.

### Error log

Per-run file: `.temper/graph-index-errors-{run_id}.log`
Format: one JSON object per line (parseable by `jq`)

```json
{"run_id":"...","phase":"llm_judgment","seed":"...","error":"...","raw_response":"..."}
```

---

## Managed Meta Fields

**RESOLVED: fields.rs pattern**

`KNOWN_TEMPER_FIELDS` lives in `crates/temper-core/src/frontmatter/fields.rs` (line 29). It's a flat string slice — used for typo detection in `check_unknown_temper_fields`. The `ManagedMeta` struct in `types/managed_meta.rs` is the typed serde struct — unknown fields round-trip through `#[serde(flatten)] extra: HashMap<String, Value>`.

The JSON schema (`schemas/concept.schema.json`) has `additionalProperties: true`, so schema validation won't reject any new field.

**Changes:**

1. **`schemas/base.schema.json`** — add four new optional properties:
   ```json
   "temper-provisional-id": {
     "type": "string",
     "pattern": "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
   },
   "temper-provenance": {
     "type": "string",
     "enum": ["llm-discovered", "user-created"],
     "description": "How this resource was created"
   },
   "temper-llm-model": {
     "type": "string",
     "description": "Model that produced this resource"
   },
   "temper-llm-run": {
     "type": "string",
     "description": "UUIDv7 of the graph-index run that created this resource"
   }
   ```

   `temper-provisional-id` goes in base because all doc types can be pre-sync.

2. **`crates/temper-core/src/frontmatter/fields.rs`** — add `temper-provenance`, `temper-llm-model`, `temper-llm-run` to `KNOWN_TEMPER_FIELDS`. Note: `temper-provisional-id` is already implicitly covered via `temper-id` pattern but should be explicitly listed.

3. **`crates/temper-core/src/types/managed_meta.rs`** — add typed fields for the three LLM fields:
   ```rust
   #[serde(rename = "temper-provenance", skip_serializing_if = "Option::is_none")]
   pub provenance: Option<String>,

   #[serde(rename = "temper-llm-model", skip_serializing_if = "Option::is_none")]
   pub llm_model: Option<String>,

   #[serde(rename = "temper-llm-run", skip_serializing_if = "Option::is_none")]
   pub llm_run: Option<String>,
   ```

   `SYSTEM_MANAGED_FIELDS` is **not** updated — these are informational fields that users can edit (e.g., correcting provenance if they hand-edit an LLM-created concept).

**TypeScript generation:** `ManagedMeta` derives `ts_rs::TS`, so adding typed fields generates corresponding TypeScript in `packages/temper-ui/src/lib/types/generated/managed_meta.ts` automatically via `cargo make generate-ts-types`.

---

## CLI Commands

### `temper index`

```
temper index [--context <ctx>] [--full]
```

- `--context` — scope to a specific context (default: all configured contexts)
- `--full` — force full rebuild (delete existing index)

**Requires:** `temper-ingest` embedded model available (ONNX). Fail early with clear message if model can't load.

### `temper graph index`

```
temper graph index [--context <ctx>] [--dry-run] [--verbose]
  [--seed-top-n N] [--threshold F] [--max-members N] [--min-members N]
  [--llm-provider <name>] [--llm-url <url>] [--llm-model <model>]
```

- `--dry-run` — emit report, no file writes
- `--verbose` — include per-file detail in report
- `--context` — scope to specific context (default: all)
- Config-file thresholds override defaults; CLI flags override both

**Requires:** `.temper/index.bin` exists. Fail with:
> "HNSW index not found. Run `temper index` first to build the index."

---

## Reference Implementation Patterns

**Vault walking** — `crates/temper-cli/src/actions/graph_build.rs` is the reference:
- `discover_vault()` → `SlugMap`, `UuidMap`, `Vec<DiscoveredFile>`
- Same context/owner partitioning
- File write-back uses frontmatter parsing + re-serialization

**Managed meta write-back** — `graph_build` uses `open_meta` directly (raw `serde_json::Value`). For `relates-to` additions in Phase 4, we read the existing frontmatter, add to the `open_meta` hashmap, serialize back. Use the same `Frontmatter::try_from` + `write_to` pattern.

**ManagedMeta round-trip** — `ManagedMeta` has `#[serde(flatten)] extra: HashMap<String, Value>`, so any field not explicitly named in the struct survives serialization. This means adding new managed fields to `ManagedMeta` is safe: existing resources with these fields (written before the typed fields existed) will deserialize correctly.

**Error handling in pipeline** — Phase 3 LLM call failures are logged and skipped. Phase 4 materialization is transactional per concept (Concept + all member edge writes succeed or roll back).

---

## Open Questions (resolved)

| Q | Resolution | Rationale |
|---|---|---|
| **Crate for tantivy + hnsw_rs?** | `temper-ingest/hnsw` feature | temper-cli already pulls in ingest; no cost to MCP/serverless |
| **Representative chunk?** | Mean-pool all chunk vectors → `doc_embedding` in sidecar | Free at index time, better than first-chunk heuristic |
| **Incremental indexing?** | Sidecar manifest (path→hash) + `hnsw_rs::add_vector` | O(changed files), not O(all); `--full` for rebuild |
| **Schema validation?** | `base.schema.json` update + `ManagedMeta` typed fields | `additionalProperties: true` means no breaking change; add explicit for docs |
| **SYSTEM_MANAGED_FIELDS?** | Not updated — these are informational, user-editable | Provenance/model/run are not enforcement fields |
| **Error log?** | Per-run file `.temper/graph-index-errors-{run_id}.log` | Overwrite keeps clean; run-id in filename enables traceability |
| **Concept schema update?** | Add fields to `base.schema.json`, not concept.schema.json | base is inherited by all doc types; `additionalProperties: true` already |

---

## Success Criteria

1. `temper index` on a ~300-file vault completes in < 2 minutes on consumer hardware
2. `temper graph index --dry-run` on the temper project's own vault produces at least one plausible `ConceptProposal`
3. `temper graph index` (non-dry-run) writes syntactically valid Concept resources that pass `temper doctor`
4. Running `temper graph index` twice on a stable corpus produces no new concepts (idempotent)
5. `temper graph build` continues to work unchanged (lexical discovery untouched)
6. `cargo make check` passes after all changes

---

## Implementation Order

1. **D3a:** Add `hnsw` feature to `temper-ingest`, wire up `indicatif` progress, implement `temper index`
2. **D3b:** Add `[graph_index]` config section to `TemperConfig`
3. **D3c:** TF-IDF seed extraction module (Phase 2.1)
4. **D3d:** Cluster formation with HNSW (Phase 2.2)
5. **D3e:** LLM judgment via `temper-llm` Agent (Phase 2.3)
6. **D3f:** Materialization — Concept file writes + member edge writes (Phase 2.4)
7. **D3g:** Managed meta fields (`ManagedMeta` + `base.schema.json` + `fields.rs`)
8. **D3h:** Integration test + smoke test