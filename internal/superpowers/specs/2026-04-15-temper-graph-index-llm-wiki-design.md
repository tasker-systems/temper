# Design: `temper graph index` — LLM-Assisted Semantic Graph Indexing

**Date:** 2026-04-15
**Context:** temper
**Goal:** llm-wiki
**Related research:**
- `2026-04-10-decision-concept-doc-types` — the asymmetric role of Concept as accretive read/link-time enrichment
- `2026-04-13-r11-knowledge-graph-visualization-design` — participant vs aggregator distinction; concepts as aggregators
- Karpathy's llm-wiki note (<https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f>) — "the tedious part is not the reading or the thinking, it's the bookkeeping"

---

## Problem Statement

Temper's graph subsystem currently has two layers:

1. **Lexical discovery** (`temper graph build`) — walks the vault, extracts wikilinks / bare UUIDs / markdown links via pulldown-cmark, writes resolved references back to frontmatter
2. **Server-side edge materialization** — sync pushes the enriched frontmatter to Postgres, where `edge_service` materializes `kb_resource_edges` from declared relationships

Both layers are **mechanical**. They surface connections only where an author has already typed them into a document body or meta field. The connections that exist *latently* — two research docs that discuss the same concept without ever mentioning each other, a task that implicitly extends an earlier one because the later author didn't know the earlier work existed — remain invisible.

This invisibility is precisely the bookkeeping burden Karpathy identifies: the LLM can read and think about these documents trivially, but a human doing the cross-referencing loses hours and misses most of what's there.

Temper also has a pre-existing architectural commitment to **Concept** as a first-class doc type (see `2026-04-10-decision-concept-doc-types`). Concepts are meant to be accretive: named handles for clusters of ideas that acquire associations over time. Until now, the creation mechanism for Concepts has been hand-waved — "they emerge from the data somehow" or "users create them explicitly." Neither path has been implemented, and neither on its own is sufficient.

`temper graph index` is the missing mechanism: an LLM-assisted pipeline that surfaces latent conceptual structure, materializes it as Concept resources with body content and bidirectional edges, and — over time — maintains that structure as the vault evolves.

---

## Scope and Phasing

This design captures the **full vision** for LLM-assisted graph indexing. Implementation follows an explicit **phases-to-learn-with** strategy: the first shipped iteration is deliberately narrow so that real data and real LLM output can inform the shape of later iterations.

**What ships in Iteration 1 (the first slice):**

- `temper index` — local HNSW index builder writing `.temper/index.bin` (new CLI feature, prerequisite)
- `temper graph index` — TF-IDF seed extraction, HNSW + graph clustering, LLM judgment, **Concept resource creation with body content and bidirectional `relates-to` edges**
- LLM provider abstraction supporting Claude and any OpenAI-compatible endpoint (ollama, etc.)
- Global config with standard precedence (env var → CLI switch → config file)

**What is captured by this design but deferred to later iterations:**

- `temper graph build --llm` — semantic edge enrichment on existing resources (adding `relates-to`, `depends-on`, etc. between existing docs without creating new Concepts)
- Concept **lifecycle operations** — drift detection, split, merge, absorption
- **LLM-assisted seed extraction** (Phase 1 Option C) — using the LLM rather than TF-IDF to produce initial concept candidates
- Server-side indexing path — a `POST /api/graph/index` that runs the pipeline with full DB access (pgvector, FTS, edge topology) instead of the local HNSW

The deferral is not scope reduction — it is a learning strategy. Drift detection has no drift to detect until real concepts have accumulated. LLM seed extraction is only meaningful to evaluate once TF-IDF seeds have been measured against ground truth. Server-side indexing should be informed by what the CLI-local pipeline teaches us about prompt quality and model behavior.

---

## Guiding Principles

**1. Temper is opinionated and highly-connected.** The vault is a managed artifact. The LLM is a first-class maintainer, not a suggestion engine. Trust model: fully automatic write-back with `--dry-run` for inspection. No proposal queue, no interactive accept/reject flow in iteration 1.

**2. Mechanical fallbacks are load-bearing.** `temper graph build` (lexical) remains the deterministic floor. `temper graph index` (semantic) is an additive enrichment layer on top. If the LLM is unavailable, build still works. If the model produces nonsense, users can re-run with different prompts or models without losing the lexical edges.

**3. Explicit over implicit.** `graph index` fails with a helpful message if `.temper/index.bin` does not exist. No silent auto-invocation of `temper index`. Users should know when they are about to do an embedding-intensive pass.

**4. Ollama-first in documentation and defaults.** Claude is supported and often higher-quality, but users should not need a paid API key to experiment. Default examples and getting-started docs assume local ollama.

**5. Local models are real but narrow.** 27-31B parameter models run on consumer hardware (Mac M4/M5 with 64GB) but consume nearly all system resources. Prompts should be designed so narrow, well-constrained judgments can use local models, while generative work (writing a good concept body) benefits from cloud-tier models when available. This design does not require routing different prompts to different backends in iteration 1, but the provider abstraction should not preclude it.

**6. Compounding over completeness.** Every run leaves the vault richer. Existing Concepts become additional search anchors for future runs. Users can hand-create Concepts as domain guidance. The system does not need to be complete on the first pass; it needs to improve monotonically.

---

## Architecture

### Command Topology

```
temper index                     # (new) build/refresh .temper/index.bin HNSW
temper graph build               # (exists) lexical reference scanning, no LLM
temper graph build --llm         # (iteration 2) lexical + LLM edge enrichment
temper graph index               # (iteration 1) concept discovery + creation
temper graph index --local-only  # (iteration 2) run pipeline up to but not including the LLM call
```

Iteration 1 ships `temper index` and `temper graph index` without `--local-only`.

### Dependency Chain

```
┌──────────────────────────────────────────────────────────────┐
│                    temper graph index                         │
│                                                               │
│  Phase 1: Concept Seed Extraction                             │
│    TF-IDF + stemming + cross-document frequency               │
│    Produces: Vec<SeedPhrase>                                  │
│                                                               │
│  Phase 2: Cluster Formation                                   │
│    For each seed: HNSW semantic search + graph neighbor hops  │
│    Threshold/limit per cluster                                │
│    Produces: Vec<Cluster { seed, members }>                   │
│                                                               │
│  Phase 3: LLM Judgment                                        │
│    For each cluster: call LLM with seed + member summaries    │
│    LLM decides: real concept? body content? edge set?         │
│    Produces: Vec<ConceptProposal>                             │
│                                                               │
│  Phase 4: Materialization                                     │
│    Write Concept resources with body + managed meta           │
│    Add relates-to edges on member documents (bidirectional)   │
│    Dry-run mode: report only, no writes                       │
│                                                               │
└──────────────────────────────────────────────────────────────┘
                              ▲
                              │ requires
                              │
┌──────────────────────────────────────────────────────────────┐
│                      temper index                             │
│                                                               │
│  Walks vault markdown                                         │
│  Embeds with BAAI/bge-base-en-v1.5 via ort                    │
│  Writes HNSW index to .temper/index.bin                       │
│  Incremental updates on subsequent runs                       │
│                                                               │
└──────────────────────────────────────────────────────────────┘
```

### Crate Layout

**New crate:** `temper-llm` — **minimal agent harness** with provider abstraction
- Designed as a harness from day 1, not a thin completion wrapper. See the [Agent Harness](#agent-harness) section below for details.
- Trait `LlmProvider` with a tool-aware completion API; implementations for Claude and any OpenAI-compatible endpoint (ollama, etc.).
- Iteration 1's `graph index` uses the harness with `max_turns: 1` (single-turn structured output). Iteration 2's drift/split/merge and `build --llm` use the same harness with more turns and richer tool sets. No rewrites between iterations.
- **Build in-house, not rig.** Our needs are narrow (tool registration, turn loop, provider dispatch, structured output), rig is a much larger surface area than we need, and we benefit from being able to read every line when debugging LLM behavior. Revisit only if we hit a concrete ceiling.

**Extended crate:** `temper-ingest`
- Already has `embed.rs` using `BAAI/bge-base-en-v1.5` via ort (gated behind `embed` feature)
- Add HNSW index builder/loader (new module) using the `hnsw_rs` crate
- Gated behind the existing `embed` feature or a new `hnsw` feature (decide during planning based on whether HNSW is always-on when embeddings are present)
- Same embedding model (`BAAI/bge-base-en-v1.5`) is used for both the server-side pgvector pipeline and the local HNSW — do not fork the model

**Extended crate:** `temper-cli`
- `src/commands/index.rs` — new `temper index` command
- `src/commands/graph.rs` — add `Index` variant to `GraphAction` enum
- `src/actions/index_build.rs` — new action for HNSW construction
- `src/actions/graph_index.rs` — new action orchestrating the 4-phase pipeline
- TF-IDF seed extraction lives in a new module — `src/actions/graph_index/seeds.rs` or similar — using `tantivy` for tokenization, stemming, and document-frequency scoring

**Extended crate:** `temper-core`
- `config.rs` grows a `LlmConfig` section
- `types/` gains `ConceptProposal` and supporting types for LLM structured output

**No server-side changes in iteration 1.** Concept resources created by `graph index` sync to the server via the existing sync pipeline; `edge_service` already handles `relates-to` edge materialization from frontmatter.

### Data Flow

**`temper index` run:**

1. Load `.temper/config.toml` to resolve vault path
2. Walk vault for markdown files (respect same context/doc-type partitioning as `graph build`)
3. For each file: extract body text (strip frontmatter), chunk if needed
4. Embed each chunk via `temper-ingest::embed` (BAAI/bge-base-en-v1.5)
5. Build or update HNSW index at `.temper/index.bin`
6. Write a small sidecar manifest (`.temper/index.json`) with file → chunk-id mappings, mtime hashes for incremental re-indexing

**`temper graph index` run:**

1. Verify `.temper/index.bin` exists; exit with helpful error if not
2. Load HNSW index and sidecar manifest
3. **Phase 1 (seeds):** walk vault, run TF-IDF across all documents in scope (context-scoped by default), extract top-N high-salience phrases. Cross-document frequency filter: phrases must appear in ≥2 documents to be candidates. Phrases also filtered against existing Concept titles (if a Concept already exists for a phrase, the existing Concept becomes the cluster anchor instead of a new seed).
4. **Phase 2 (clusters):** for each seed phrase:
   - Embed the phrase (same model, same path)
   - HNSW nearest-neighbor search → candidate member set
   - Graph neighbor hops (via existing Postgres `graph_traverse` if synced, else skip and rely on vector alone) — adds topologically-related docs the embeddings miss
   - Apply threshold (cosine similarity cutoff) and size cap (max N members per cluster, configurable)
5. **Phase 3 (LLM):** for each cluster:
   - Build a prompt with: seed phrase, existing concepts in the owner/context, summary of each member document (title + first N lines or LLM-extracted summary)
   - Call configured LLM provider with structured output schema
   - LLM returns: `{ is_concept: bool, slug: string, title: string, body_markdown: string, member_edges: [{ target_slug, edge_type }] }`
   - `is_concept: false` → skip cluster (LLM judged it incoherent or duplicative)
6. **Phase 4 (materialize):** for each accepted ConceptProposal:
   - Create Concept resource file at `{vault}/{owner}/{context}/concept/{slug}.md` with frontmatter + body
   - For each member in `member_edges`: add edge (default `relates-to`) in that document's frontmatter open_meta
   - Dry-run: emit a structured report, no file writes
   - Non-dry-run: write files, then trigger sync (or suggest the user run it)

### Agent Harness

`temper-llm` is built as a minimal agent harness from day 1. The shape:

```rust
// Provider abstraction — the thing that actually talks to Claude or ollama.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(
        &self,
        system: &str,
        messages: &[Message],
        tools: &[ToolSchema],          // may be empty (single-turn structured output)
        response_format: Option<&JsonSchema>,
    ) -> Result<LlmResponse, LlmError>;

    fn provider_name(&self) -> &str;
    fn model(&self) -> &str;
}

// LlmResponse carries either a final structured output or a set of tool calls
// the model wants us to execute.
pub enum LlmResponse {
    Final { content: serde_json::Value },
    ToolUse { calls: Vec<ToolCall> },
}

// Tools are registered with the harness; each has a name, JSON-schema input,
// and an async handler.
pub struct Tool<S> {
    pub name: String,
    pub description: String,
    pub input_schema: JsonSchema,
    pub handler: Box<dyn ToolHandler<S>>,
}

// The harness drives the turn loop: call provider, dispatch any tool calls,
// feed results back, repeat until Final or max_turns reached.
pub struct Agent<S> {
    provider: Arc<dyn LlmProvider>,
    tools: Vec<Tool<S>>,
    max_turns: usize,
    state: S,  // caller-supplied state passed to tool handlers
}

impl<S> Agent<S> {
    pub async fn run(&mut self, system: &str, user: &str) -> Result<AgentOutcome, AgentError> {
        // Loop: complete → dispatch tool calls → feed tool results → repeat.
        // Exits on Final, max_turns exhaustion, or unrecoverable error.
    }
}
```

**Provider implementations:**

- `ClaudeProvider` — Anthropic Messages API with native tool use and structured output via `response_format` or equivalent
- `OpenAiCompatibleProvider` — handles ollama, any OpenAI-spec endpoint; uses function-calling or JSON mode depending on server capability (model-dependent for ollama — llama3.2 and qwen3 support tool use, older models may not)

**Iteration 1 use:** `graph index` invokes `Agent::run` with `max_turns: 1`, no tools registered, and a `response_format` for `ConceptProposal`. The harness reduces to a single structured-output call — functionally equivalent to a direct `provider.complete()` but built on the harness so iteration 2 gets multi-turn for free.

**Iteration 2+ use:** drift detection, concept split/merge, and `build --llm` register tools like `search_vault`, `read_resource`, `get_neighbors`, `propose_edge`, and let the agent run for N turns. The harness's turn loop, tool dispatch, and error handling are already in place.

**Tool-call transparency:** every tool call and its result is logged with the run-id. This is the audit trail for later iterations — "why did the LLM do X?" is answered by reading the log.

**Determinism note:** agent loops are not deterministic. Iteration 1's single-turn use preserves the determinism of a fixed pipeline (modulo LLM sampling). Later iterations accept some probabilistic termination behavior in exchange for the capabilities the harness enables. Tests use a mock `LlmProvider` that returns canned responses.

Provider selection via config:

```toml
[llm]
provider = "ollama"           # or "claude", "openai-compatible"
url = "http://localhost:11434"
model = "llama3.2:latest"
# api_key read from env: TEMPER_LLM_API_KEY (for claude or authenticated ollama)

[llm.claude]
# optional overrides when provider = "claude"
model = "claude-sonnet-4-5"
```

Precedence: `TEMPER_LLM_*` env vars → `--llm-provider`, `--llm-url`, `--llm-model` CLI flags → config file.

### Concept Resource Shape

The existing schema (`crates/temper-core/schemas/concept.schema.json`) requires only `slug`, `date`, and `temper-type: concept`. LLM-created concepts honor this with additional managed-meta fields (`temper-provenance`, `temper-llm-model`, `temper-llm-run`) and open-meta edge declarations (`relates-to`):

```yaml
---
temper-id: <uuid-v7>               # assigned on first sync, temper-provisional-id pre-sync
temper-type: concept
title: "Narrative Topology"
slug: narrative-topology
date: 2026-04-15
temper-context: temper
temper-owner: "@me"
temper-created: 2026-04-15T...
temper-provenance: llm-discovered   # managed meta — distinguishes llm-discovered from user-created
temper-llm-model: llama3.2:latest   # managed meta — which model produced this resource
temper-llm-run: <uuidv7-run-id>     # managed meta — groups resources created in one graph-index run
relates-to:
  - 2026-04-10-decision-concept-doc-types
  - 2026-04-13-r11-knowledge-graph-visualization-design
---

# Narrative Topology

A description of the concept — what it means, why it matters, which
resources share this conceptual space and how they relate to it
specifically.

## Members

- **2026-04-10-decision-concept-doc-types** — introduces the narrative-topology
  phrase as an example of a concept that is expensive to re-establish session-over-session
- **2026-04-13-r11-knowledge-graph-visualization-design** — extends the idea with
  the participant/aggregator distinction, treating topology as emergent rather than declared

## Context

(LLM-written prose explaining the conceptual thread that binds members)
```

**Provenance is managed meta.** `temper-provenance`, `temper-llm-model`, and `temper-llm-run` are new managed-meta fields. They need to be registered in the managed-meta registry (`KNOWN_MANAGED_FIELDS` or equivalent) alongside existing fields like `temper-id`, `temper-type`, and `temper-context`. Users remain free to edit, split, or supersede LLM-created concepts — provenance tracks where a resource came from, not who is allowed to modify it. The field is informational: useful for future lifecycle operations (e.g., "only re-evaluate llm-discovered concepts during drift detection") but not enforcement.

**Body content is first-class.** The LLM writes a substantive explanation, not a stub. This is the substrate that enables future evolution: drift detection compares current cluster membership against the body's described thread; concept splitting examines which members still fit the body and which have drifted to a new idea.

### Bidirectional Edges

For each member the LLM identifies, the CLI writes a `relates-to: <concept-slug>` entry into that member's frontmatter open_meta (using the same `write_back_references` pattern `graph build` uses). On next sync, `edge_service::reconcile_edges` materializes the DB edge.

Edge direction matches the current graph semantics: `member relates-to concept`. The reverse direction (`concept relates-to member`) is also written to the Concept resource's own frontmatter, so the graph is fully navigable from either node. The Postgres unique constraint on `(source_resource_id, target_resource_id, edge_type)` allows both directions to coexist as separate rows, which is consistent with how `edge_service` handles symmetric relationships today.

---

## Configuration

```toml
# ~/.config/temper/config.toml  (or $TEMPER_GLOBAL_CONFIG)

[llm]
provider = "ollama"
url = "http://localhost:11434"
model = "llama3.2:latest"

[graph_index]
# cluster formation thresholds
seed_min_doc_frequency = 2        # phrase must appear in ≥N docs to be a seed candidate
seed_top_n = 50                   # max seeds per run
cluster_similarity_threshold = 0.70  # cosine cutoff for HNSW neighbors
cluster_max_members = 12          # cap cluster size to keep LLM context bounded
cluster_graph_hop_depth = 1       # 0 = vector only; 1 = include 1-hop neighbors

[graph_index.concept]
# concept acceptance criteria
min_members = 3                   # LLM's accepted concept must link ≥N docs or is rejected
default_edge_type = "relates-to"  # LLM can propose others but this is the fallback
```

Env var overrides: `TEMPER_LLM_PROVIDER`, `TEMPER_LLM_URL`, `TEMPER_LLM_MODEL`, `TEMPER_LLM_API_KEY`.

CLI flag overrides: `--llm-provider`, `--llm-url`, `--llm-model`, `--threshold`, `--max-members`, `--dry-run`, `--verbose`, `--context`.

---

## Error Handling

**LLM failures are first-class citizens.** The pipeline must never corrupt the vault mid-run. Specifically:

- Provider unreachable → fail early before any writes
- Per-cluster LLM call failures → log, skip that cluster, continue
- Structured output parsing failures → log the raw response to `.temper/graph-index-errors.log`, skip cluster, continue
- File write failures during materialization → transactional semantics per concept (Concept file + all member edge writes succeed or all roll back for that one concept)

**Dry-run is the safety net.** `--dry-run` produces a structured report showing: seeds extracted, clusters formed (size, members), LLM proposals accepted/rejected, files that would be written, edges that would be added. No vault mutations.

**Run IDs for traceability.** Every `graph index` run generates a UUIDv7 run-id. All Concepts created in the run carry this in the managed-meta `temper-llm-run` field. This lets a user audit "what did this run produce?" and, in iteration 2, enables undo/rollback operations.

---

## Testing

Three layers:

**Unit tests:**
- TF-IDF seed extraction — given a known corpus, extract expected seeds
- Cluster formation logic — given mock HNSW responses and graph hops, produce expected clusters
- LLM provider abstraction — mock provider returning canned JSON, verify parsing

**Integration tests:**
- End-to-end `temper index` on a fixture vault → verify `.temper/index.bin` exists and is queryable
- End-to-end `temper graph index --dry-run` with a mock LlmProvider (returning deterministic proposals) → verify report structure, no vault writes
- End-to-end `temper graph index` (non-dry-run) with mock provider → verify Concept files created, member edges written, idempotent on second run

**E2E (in `temper-e2e`):**
- Full flow: `index` → `graph index` → `sync` → verify server-side Concept resource and edges in Postgres
- Requires the Embed CI job (ONNX Runtime) per the workspace feature unification note
- Real LLM calls are NOT part of CI. A dedicated `temper graph index --smoke` manual test with real ollama is documented in the plan.

---

## Open Questions

**Q1 (resolved): HNSW crate is `hnsw_rs`.** This matches what the project used in its pre-temper-cloud history (confirmed in git archaeology). Revisit only if we hit a concrete blocker during implementation.

**Q2 (resolved): use `tantivy` for TF-IDF.** Neither `tantivy` nor `rust-stemmers` are currently direct deps in the workspace. Adopting `tantivy` adds build weight but removes the ongoing maintenance burden of an in-tree tokenize/stem/count/normalize implementation (including Unicode edge cases, stopword lists, and language handling that would otherwise grow over time). Revisit only if `tantivy`'s surface area or build time becomes a concrete problem.

**Q3: Chunking for embeddings.** `temper-ingest` already embeds chunks for search. Does `temper index` use the same chunking strategy or embed full documents? For clustering purposes, document-level embeddings may be too coarse for long research docs. Initial guess: reuse existing chunking, treat the most representative chunk per document as the "document embedding" for cluster formation. Verify during implementation.

**Q4: Summary generation for LLM prompts.** When sending a cluster to the LLM, each member needs a summary to fit in the context. Options: (a) first N lines of body, (b) LLM pre-pass to summarize (adds LLM calls), (c) use the existing chunk most similar to the seed (free — it's already in HNSW). Initial lean: (c).

**Q5: Concurrency model.** How many agent runs execute in parallel across clusters? Ollama's concurrency behavior depends on the model and hardware (local GPU is typically a bottleneck). Claude allows more parallelism at the API level but subject to rate limits. Default to serial for iteration 1 (one cluster at a time), make configurable in iteration 2. The harness itself is agnostic — concurrency is managed at the graph-index orchestration layer, not inside `Agent::run`.

**Q6: Deferred — server-side indexing.** At what vault size does `graph index` stop being a sensible CLI-local operation? The server already has all resources, pgvector, and FTS. A `POST /api/graph/index` endpoint could run the pipeline centrally. Defer decision until iteration 1 teaches us what the realistic vault-size ceiling is.

---

## Success Criteria (Iteration 1)

1. `temper index` produces a working HNSW index at `.temper/index.bin` over a real vault in reasonable time (target: < 2 minutes for ~1000 documents on consumer hardware).
2. `temper graph index --dry-run` with a small local ollama model produces at least one plausible ConceptProposal on the Temper project's own vault (we are literally using ourselves as the test corpus).
3. `temper graph index` (non-dry-run) writes syntactically valid Concept resources that pass existing frontmatter validation (`temper doctor`).
4. Concept resources created by the pipeline round-trip through sync: they appear in Postgres, their `relates-to` edges materialize in `kb_resource_edges`, and they are visible via existing `/api/resources` and MCP tool endpoints.
5. Running `temper graph index` a second time with no vault changes produces **no new concepts** (idempotent on a stable corpus — either the LLM rejects duplicates, or the pre-filter catches them before the LLM call).
6. A user can disable LLM integration entirely and still use `temper graph build` as before. Lexical discovery is untouched by this work.

---

## Long-Term Vision (Context for Later Iterations)

The iteration-1 pipeline is the scaffold. The full vision extends it in these directions:

- **Drift detection** — a `temper graph index --maintain` mode that re-evaluates existing Concepts: are current members still semantically aligned with the concept body? Has the neighborhood grown in a way that suggests a split? Has it shrunk in a way that suggests absorption into another concept?
- **Concept evolution operations** — split, merge, supersede. LLM-driven but with user approval for destructive operations (unlike creation, which is fully automatic).
- **Decision resources** — the asymmetric counterpart. Where Concepts are accretive read/link-time enrichment, Decisions are write-amplifying events that sweep the vault on creation. A `temper decision record` workflow that uses the same LLM infrastructure to annotate or update affected resources.
- **Semantic edge enrichment on `graph build`** — `temper graph build --llm` adds `relates-to`, `depends-on`, `extends` edges between *existing* resources (no new Concept creation), using the same cluster-then-judge pipeline but with a different prompt.
- **Server-side indexing** — offload the pipeline from CLI to API for users with large vaults or who want scheduled indexing (nightly concept refresh, etc.).
- **Cross-provider prompt routing** — use local models for narrow judgments ("does this cluster cohere?"), cloud models for generative work ("write a good concept body"). Configurable per-phase.

These are not iteration-1 commitments. They are captured here so the iteration-1 design does not foreclose them. All of them compose naturally on top of the `temper-llm` agent harness — they register additional tools (`search_vault`, `read_resource`, `propose_split`, etc.) and run with `max_turns > 1`. The single-turn iteration-1 use of the harness is a deliberate first rung on a ladder whose upper rungs are already structurally in scope.

---

## References

- Current `graph build` implementation: `crates/temper-cli/src/actions/graph_build.rs`
- Current edge service: `crates/temper-api/src/services/edge_service.rs`
- Current embedding pipeline: `crates/temper-ingest/src/embed.rs`
- Concept schema: `crates/temper-core/schemas/concept.schema.json`
- Graph edge schema migration: `migrations/20260411000002_knowledge_graph_edges.sql`
- Graph search function: `migrations/20260411000003_graph_search.sql`
