# R8: Local Search — HNSW Indexing & PageIndex Tree Retrieval — Research & Proposal

**Date:** 2026-04-01
**Type:** Research (R-phase) + Implementation Proposal
**Scope:** Local HNSW vector index revival, PageIndex-inspired hierarchical tree search, Rust crate design for offline-capable knowledge retrieval
**Depends on:** R2 (data model — done), R5 (indexing design — done), I5e (local KB restructure — done), I6a (sync — done)
**Blocks:** Offline search capability, MCP local search tools, advanced retrieval strategies

---

## Table of Contents

1. [Problem Statement](#problem-statement)
2. [Two-Tier Search Architecture](#two-tier-search-architecture)
3. [HNSW Local Index Revival](#hnsw-local-index-revival)
4. [PageIndex Analysis & Rust Port](#pageindex-analysis--rust-port)
5. [MCP Tool Surface](#mcp-tool-surface)
6. [Index Build Pipeline](#index-build-pipeline)
7. [Data Model](#data-model)
8. [Integration with Cloud Sync](#integration-with-cloud-sync)
9. [Implementation Plan](#implementation-plan)
10. [Risk Analysis](#risk-analysis)
11. [Alternatives Considered](#alternatives-considered)
12. [Open Questions](#open-questions)
13. [Decision Log](#decision-log)
14. [Appendix A: PageIndex Attribution](#appendix-a-pageindex-attribution)
15. [Appendix B: Registry v2 Full Schema](#appendix-b-registry-v2-full-schema)
16. [Appendix C: Related Tickets & Dependencies](#appendix-c-related-tickets--dependencies)

---

## Problem Statement

Temper's current search architecture is **cloud-only by design**. The I5d implementation established `temper search` as a cloud-routed command: CLI embeds queries locally via bge-base-en-v1.5 ONNX, sends 768-dim vectors to the Rust API, and receives pgvector cosine similarity results filtered by `resources_visible_to()` access control. The prior local HNSW index at `.temper/index.bin` (77MB binary, all-MiniLM-L6-v2) and `.temper/registry.json` (6MB file→chunk mapping) were deliberately dropped in favor of this cloud-first approach.

This was the right call for establishing a single source of truth. But it creates three structural gaps:

### Gap 1: Offline Work Is Impossible

When the developer has no internet connectivity — on a plane, in a dead zone, behind a restrictive VPN — `temper search` returns an error. The entire knowledge retrieval surface goes dark. The vault files are right there on disk, fully readable, but unsearchable.

### Gap 2: Agent-Local Retrieval Pays Cloud Costs

MCP-connected agents (Claude Desktop, Claude Code, custom agent loops) route all search through the temper-cloud API. Every `temper search` call from an agent is an API hit against Vercel serverless functions. For high-frequency agent workflows — research loops, context assembly, iterative refinement — this creates:
- Latency: cold-start serverless + network round-trip per query
- Cost: Vercel function invocations, Postgres connection time, bandwidth
- Rate pressure: agent loops can issue dozens of searches per session

The agent's LLM provider (OpenAI, Anthropic, local Ollama) is already paid for by the user. But the search infrastructure cost falls on temper-cloud's serverless budget.

### Gap 3: Similarity ≠ Relevance (The PageIndex Insight)

Vector search finds text that is *similar* to the query. But similar text is not always *relevant* text. A developer searching for "how does the sync protocol handle conflicts" needs the conflict resolution section, not every paragraph that mentions "sync" or "conflict" — those are similar but not necessarily the answer.

PageIndex (VectifyAI/PageIndex, MIT licensed) demonstrates an alternative: **reasoning-based retrieval** using document structure. Instead of embedding and ranking, an LLM agent navigates a hierarchical table-of-contents tree, reasoning about which sections contain the answer. On FinanceBench, this achieves 98.7% accuracy (SOTA), outperforming vector-based RAG.

Temper's vault is **entirely markdown with heading structure**. The `# → ## → ### → ####` hierarchy is already there in every file. PageIndex's tree-based approach is a natural fit — and the LLM cost for tree traversal is borne by the user's own environment (their API keys, their local model), not temper-cloud's budget.

### The Opportunity

The vault already has all markdown files locally. The heading structure is already there. The embedding model is already available via `temper-ingest`. What's missing is:
1. A local HNSW index for fast vector search (reviving the dropped capability with the correct embedding model)
2. A PageIndex-style tree index for reasoning-based retrieval (new capability)
3. MCP tools that expose both to agents

These are **complementary** retrieval strategies that operate entirely on local data, at the user's own compute cost.

---

## Two-Tier Search Architecture

The vision is two independent, complementary search tiers:

```
┌─────────────────────────────────────────────────────────────────────┐
│                         CLOUD TIER                                  │
│                                                                     │
│  temper search "query" ──► temper-cloud API                        │
│                                                                     │
│  • pgvector HNSW on kb_chunks (768-dim bge-base-en-v1.5)          │
│  • Access control via resources_visible_to()                        │
│  • Cross-team, cross-device visibility                              │
│  • Future: graph traversal (R7)                                     │
│  • Authoritative — the canonical search answer                      │
│  • Cost: temper-cloud serverless budget                             │
│                                                                     │
│  When: Online, team search, canonical results needed                │
└─────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────┐
│                         LOCAL TIER                                   │
│                                                                     │
│  temper search --local "query" ──► .temper/index.bin               │
│  MCP vault_search_local ──► .temper/index.bin                      │
│  MCP vault_search_tree ──► .temper/trees/*.json                    │
│                                                                     │
│  • HNSW vector search on local index (768-dim bge-base-en-v1.5)   │
│  • PageIndex tree traversal (agent-driven, LLM-reasoned)           │
│  • Personal vault only — no cross-team visibility                   │
│  • Offline-capable — zero network dependencies                      │
│  • Cost: user's own compute (ONNX inference, LLM API keys)        │
│                                                                     │
│  When: Offline, fast iteration, agent workflows, MCP tools          │
└─────────────────────────────────────────────────────────────────────┘
```

### Design Principles

1. **Cloud is canonical, local is fast.** When online, cloud search is the authoritative answer. Local search is the fast, personal, agent-friendly answer. They are not competitive — they serve different use cases.

2. **Local index is ephemeral and reproducible.** The `.temper/index.bin` and `.temper/trees/` directory are gitignored. They can be rebuilt from vault content at any time via `temper index`. No data is lost if they're deleted.

3. **User pays their own LLM costs.** PageIndex tree traversal requires LLM reasoning. This happens in the user's environment — their local Ollama, their OpenAI API key, their Claude Desktop session. temper-cloud never pays for LLM inference on behalf of local search.

4. **Embedding model alignment is mandatory.** The local HNSW index MUST use the same 768-dim bge-base-en-v1.5 model as the cloud pgvector index. This ensures that search quality is comparable and that embeddings could theoretically be shared across tiers.

5. **The tree index is LLM-free to build.** Building the PageIndex tree structure from markdown headings is pure parsing — no LLM needed. Summary generation (optional enrichment) requires an LLM, but the base tree is constructed from the markdown AST alone.

---

## HNSW Local Index Revival

### Background

The legacy local index used:
- **HNSW binary:** `.temper/index.bin` (77MB, all-MiniLM-L6-v2 384-dim)
- **Registry:** `.temper/registry.json` (6MB, file→chunk_id→content_hash mapping)

This was dropped when cloud search became the primary path. The revival uses the same storage locations but with the correct embedding model (bge-base-en-v1.5, 768-dim) and an updated registry format.

### Storage Layout

```
{vault-root}/
├── .temper/
│   ├── manifest.json          ← existing: sync state (unchanged)
│   ├── events.jsonl           ← existing: local audit trail (unchanged)
│   ├── index.bin              ← REVIVED: serialized HNSW graph (768-dim)
│   ├── registry.json          ← REVIVED: v2 format, file→chunk→embedding mapping
│   └── trees/                 ← NEW: PageIndex tree structures
│       ├── {resource-id}.json ← per-document tree (heading hierarchy + optional summaries)
│       └── ...
```

All three index artifacts (`.temper/index.bin`, `.temper/registry.json`, `.temper/trees/`) are gitignored. They are reproducible from vault content.

### Embedding Model Alignment

| Property | Cloud (pgvector) | Local (HNSW) |
|----------|-----------------|--------------|
| **Model** | bge-base-en-v1.5 | bge-base-en-v1.5 |
| **Dimensions** | 768 | 768 |
| **Inference** | temper-cloud serverless (ONNX) | temper-ingest local (ONNX) |
| **Normalization** | L2-normalized | L2-normalized |
| **Distance metric** | Cosine similarity (pgvector `<=>`) | Cosine similarity (HNSW config) |

The `temper-ingest` crate already implements bge-base-en-v1.5 embedding via ONNX Runtime (`crates/temper-ingest/src/embed.rs`). The same `embed_text()` and `embed_texts()` functions used for cloud-routed search query embedding will be used for local index construction.

### Rust Crate Options for HNSW

| Crate | Version | Notes |
|-------|---------|-------|
| [`hnsw_rs`](https://crates.io/crates/hnsw_rs) | 0.3.x | Pure Rust, serializable, supports cosine distance. Used by several RAG projects. |
| [`instant-distance`](https://crates.io/crates/instant-distance) | 0.6.x | Pure Rust, simple API, serializable. Originally from Ditto. |
| [`hnswlib-rs`](https://crates.io/crates/hnswlib-rs) | 0.3.x | Rust bindings to C++ hnswlib. Mature but requires C++ toolchain. |
| [`usearch`](https://crates.io/crates/usearch) | 2.x | USearch Rust bindings. High-performance, multi-metric, serializable. |

**Recommendation: `hnsw_rs`** — Pure Rust (no C++ toolchain dependency), serialization support via serde, configurable distance metrics, and reasonable performance for vault-scale datasets (thousands to low tens-of-thousands of vectors). If performance becomes a bottleneck at scale, `usearch` is the upgrade path.

### Incremental Indexing

The registry tracks `content_hash` per file. On `temper index`:

```
For each file in vault matching [index] config:
  1. Compute SHA-256 of file content
  2. Look up file path in registry
  3. If content_hash matches → skip (no changes)
  4. If content_hash differs or file is new:
     a. Parse frontmatter, extract temper-id
     b. Chunk content (header-based splitting)
     c. Embed each chunk via temper-ingest
     d. Remove old vectors from HNSW (if updating)
     e. Insert new vectors into HNSW
     f. Build/update PageIndex tree
     g. Update registry entry
  5. If file was in registry but no longer on disk → remove from HNSW + registry
```

This makes incremental indexing O(changed files) rather than O(all files). The `--full` flag bypasses the content_hash check and rebuilds everything.

### Index Lifecycle

```
temper index              # Incremental build (skip unchanged files)
temper index --full       # Full rebuild (re-embed everything)
temper index --status     # Show index stats (files indexed, chunks, last build time)
temper index --clear      # Delete index.bin, registry.json, trees/ — start fresh
```

The index is **never** automatically rebuilt. It's an explicit user action. The filesystem watcher (R6) could trigger re-indexing of changed files in the future, but that's a separate concern.

---

## PageIndex Analysis & Rust Port

### Core Algorithm

PageIndex (VectifyAI/PageIndex) implements a three-phase retrieval approach:

**Phase 1: Tree Construction (offline, at index time)**
1. Parse markdown files by heading structure (`# → ## → ### → ####`)
2. Each heading becomes a node in a tree; the text between headings is the node's content
3. Optionally generate LLM summaries for each node (enrichment)
4. Apply "tree thinning" — merge small nodes (below a token threshold) into their parent
5. Serialize the tree as JSON

**Phase 2: Document Registration (offline, at index time)**
1. Generate a document-level description (LLM or metadata-based)
2. Store document metadata (title, description, node count, token count)

**Phase 3: Agentic Retrieval (online, at query time)**
1. Agent calls `get_document()` — receives document metadata and description
2. Agent calls `get_document_structure()` — receives the tree without text content (just titles + summaries)
3. Agent reasons about which sections are relevant to the query
4. Agent calls `get_page_content(node_id)` — retrieves full text for specific nodes
5. Agent synthesizes the answer from retrieved content

The key insight: **the LLM does the relevance reasoning**, not a distance metric. The tree structure gives the LLM a "table of contents" to navigate, just as a human expert would scan a book's index before reading specific chapters.

### Python Implementation Analysis

The PageIndex Python codebase (`page_index_md.py`, `retrieve.py`, `utils.py`) has these components:

| Component | Python Implementation | Rust Port Strategy |
|-----------|----------------------|-------------------|
| Heading extraction | Regex: `r'^(#{1,6})\s+(.*)'` | `pulldown-cmark` AST (proper markdown parsing) |
| Tree building | Dict-of-dicts with `title`, `node_id`, `line_num`, `text`, `nodes` | Typed `PageNode` struct with `Vec<PageNode>` children |
| Tree thinning | Recursive merge of small nodes by token count | Pure function on `PageTree` |
| Summary generation | OpenAI API call per node | **Not ported** — consumer's responsibility |
| Node content extraction | String slicing by line numbers | Zero-copy `&str` slicing with line index |
| Tree serialization | `json.dumps` / `json.loads` | `serde_json` |
| Token counting | `tiktoken` library | `tokenizers` crate (already a dependency in temper-ingest) |
| Retrieval tools | Three Python functions for LLM tool-use | MCP tool definitions in `temper-mcp` |
| LLM completion | `openai.ChatCompletion.create()` | **Not ported** — the MCP agent IS the LLM |

### What to Port to Rust

Everything that is pure computation — no LLM dependency:

1. **Tree structure generation** — Parse markdown headings into a hierarchical tree
2. **Tree thinning** — Merge small nodes below a token threshold into their parent
3. **Node content extraction** — Retrieve text for specific nodes by ID or line range
4. **Tree serialization/deserialization** — JSON round-trip for persistence
5. **Token counting** — Approximate token counts for tree thinning decisions
6. **Tree traversal utilities** — Find node by ID, list children, get path from root

### What Stays as LLM Interaction (Not Ported)

1. **Summary generation** — Requires LLM. The consumer (MCP agent, CLI plugin) provides this.
2. **Document description generation** — Requires LLM. Optional enrichment.
3. **Tree search reasoning** — This IS the agent. The agent receives tree structure via MCP tools and reasons about which nodes to retrieve.

### Crate Design: `temper-pageindex`

A standalone crate within the temper workspace, designed to be independently publishable:

```
crates/temper-pageindex/
├── Cargo.toml
├── LICENSE-MIT              ← MIT license (matching VectifyAI/PageIndex)
├── README.md                ← Standalone crate documentation with attribution
├── src/
│   ├── lib.rs               ← Public API re-exports
│   ├── tree.rs              ← PageTree, PageNode, TreeConfig types
│   ├── parser.rs            ← parse_markdown_to_tree() — heading extraction via pulldown-cmark
│   ├── thinning.rs          ← thin_tree() — merge small nodes
│   ├── content.rs           ← get_node_content(), get_node_by_id() — text extraction
│   ├── token.rs             ← approximate_token_count() — fast token estimation
│   └── error.rs             ← PageIndexError type
└── tests/
    ├── parser_tests.rs      ← Markdown → tree round-trip tests
    ├── thinning_tests.rs    ← Tree thinning behavior tests
    └── fixtures/
        ├── simple.md        ← Basic heading hierarchy
        ├── deep_nesting.md  ← 6-level heading depth
        ├── frontmatter.md   ← YAML frontmatter + headings
        └── sparse.md        ← Few headings, large text blocks
```

### Core Types

```rust
use serde::{Deserialize, Serialize};

/// A single node in the PageIndex tree, corresponding to a markdown heading
/// and its content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageNode {
    /// Human-readable section title (the heading text, without `#` markers).
    pub title: String,

    /// Unique node identifier within the document (e.g., "0001", "0002").
    /// Assigned during tree construction in depth-first order.
    pub node_id: String,

    /// Heading level: 1 for `#`, 2 for `##`, etc. 0 for the root node.
    pub level: u8,

    /// Line number (1-based) where this heading appears in the source document.
    pub line_num: usize,

    /// Line number (1-based) where this node's content ends (exclusive).
    /// Used for zero-copy text extraction.
    pub end_line: usize,

    /// Full text content of this section (between this heading and the next
    /// heading at the same or higher level). Empty string if `strip_text` was
    /// used during serialization.
    pub text: String,

    /// LLM-generated summary of the section content. Empty until enriched
    /// by an external LLM call. The crate never generates this — it's the
    /// consumer's responsibility.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub summary: String,

    /// Approximate token count of `text`. Used for tree thinning decisions.
    pub token_count: usize,

    /// Child nodes (subsections).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<PageNode>,
}

/// The root container for a document's PageIndex tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageTree {
    /// Document title (from frontmatter `title:` field or first heading).
    pub title: String,

    /// Temper resource ID (UUIDv7) for cross-referencing with the registry
    /// and manifest.
    pub resource_id: String,

    /// Relative vault path to the source document.
    pub source_path: String,

    /// SHA-256 content hash at the time the tree was built.
    /// Used to detect staleness.
    pub content_hash: String,

    /// ISO 8601 timestamp of when this tree was built.
    pub built_at: String,

    /// Total token count across all nodes.
    pub total_tokens: usize,

    /// LLM-generated document-level description. Empty until enriched.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,

    /// The root node containing all top-level sections.
    pub root: PageNode,
}

/// Configuration for tree construction and thinning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeConfig {
    /// Minimum token count for a node to remain independent during thinning.
    /// Nodes below this threshold are merged into their parent.
    /// Default: 128 tokens.
    #[serde(default = "default_min_tokens")]
    pub min_node_tokens: usize,

    /// Maximum heading depth to include in the tree (1-6).
    /// Headings deeper than this are treated as body text.
    /// Default: 6 (include all heading levels).
    #[serde(default = "default_max_depth")]
    pub max_depth: u8,

    /// Whether to include the YAML frontmatter as a special root-level
    /// metadata node. Default: true.
    #[serde(default = "default_include_frontmatter")]
    pub include_frontmatter: bool,
}

fn default_min_tokens() -> usize { 128 }
fn default_max_depth() -> u8 { 6 }
fn default_include_frontmatter() -> bool { true }

impl Default for TreeConfig {
    fn default() -> Self {
        Self {
            min_node_tokens: default_min_tokens(),
            max_depth: default_max_depth(),
            include_frontmatter: default_include_frontmatter(),
        }
    }
}
```

### Core Functions

```rust
use crate::{PageTree, PageNode, TreeConfig, PageIndexError};

/// Parse a markdown document into a PageIndex tree.
///
/// Uses pulldown-cmark for proper markdown AST parsing (not regex).
/// Extracts heading hierarchy, assigns node IDs, computes token counts,
/// and builds the tree structure.
///
/// # Arguments
/// * `source` — The full markdown document text (including frontmatter)
/// * `resource_id` — The temper resource ID (UUIDv7) for this document
/// * `source_path` — Relative vault path for metadata
/// * `content_hash` — SHA-256 hash of the source content
/// * `config` — Tree construction configuration
///
/// # Returns
/// A `PageTree` with the heading hierarchy. Node `text` fields contain
/// the full section text. `summary` fields are empty (LLM enrichment
/// is the consumer's responsibility).
pub fn parse_markdown_to_tree(
    source: &str,
    resource_id: &str,
    source_path: &str,
    content_hash: &str,
    config: &TreeConfig,
) -> Result<PageTree, PageIndexError>;

/// Apply tree thinning: merge nodes with fewer than `config.min_node_tokens`
/// tokens into their parent node.
///
/// This reduces tree breadth for documents with many small sections,
/// improving the signal-to-noise ratio when an LLM reads the tree structure.
///
/// Thinning is applied bottom-up: leaf nodes are evaluated first, then
/// their parents (which may have absorbed children and grown above threshold).
pub fn thin_tree(tree: &mut PageTree, config: &TreeConfig);

/// Retrieve a specific node by its node_id.
///
/// Returns `None` if the node_id doesn't exist in the tree.
pub fn get_node_by_id<'a>(tree: &'a PageTree, node_id: &str) -> Option<&'a PageNode>;

/// Retrieve the text content for a node, optionally including children's text.
///
/// When `include_children` is true, returns the full text from this node's
/// start line to the last child's end line. When false, returns only the
/// text directly under this heading (before the first child heading).
pub fn get_node_content(
    source: &str,
    node: &PageNode,
    include_children: bool,
) -> &str;

/// Return the tree structure without text content — just titles, node_ids,
/// summaries, and nesting. Used for the `get_document_structure()` MCP tool
/// to give agents a lightweight overview.
pub fn tree_structure_only(tree: &PageTree) -> PageTree;

/// List all node_ids with their titles and depths, in depth-first order.
/// Useful for flat navigation or selection UIs.
pub fn list_nodes(tree: &PageTree) -> Vec<(String, String, u8)>;

/// Fast approximate token count. Uses the heuristic:
/// tokens ≈ (byte_len / 4) for English text.
///
/// This avoids loading a full tokenizer for tree thinning decisions.
/// When precise counts are needed (e.g., for LLM context budgeting),
/// the consumer should use their own tokenizer.
pub fn approximate_token_count(text: &str) -> usize;
```

### Key Rust Advantages over Python Implementation

| Concern | Python (PageIndex) | Rust (temper-pageindex) |
|---------|-------------------|------------------------|
| **Heading parsing** | Regex `r'^(#{1,6})\s+(.*)'` — breaks on headings in code blocks, HTML comments, frontmatter | `pulldown-cmark` AST — correct markdown parsing that respects code fences, HTML blocks, frontmatter boundaries |
| **Error handling** | `try/except ImportError` for optional dependencies, silent failures | `Result<T, PageIndexError>` with typed error variants, no silent failures |
| **Type safety** | Dict-of-dicts (`node["title"]`, `node["nodes"]`) — runtime KeyError | `PageNode` struct — compile-time field access, exhaustive pattern matching |
| **Text extraction** | String copying via line splitting and rejoining | Zero-copy `&str` slicing with precomputed line index |
| **Serialization** | `json.dumps` with manual dict construction | `serde_json` with `#[derive(Serialize, Deserialize)]` — round-trip guaranteed |
| **Concurrency** | GIL-limited, sequential file processing | `rayon` parallel iteration for multi-file tree building |
| **WASM target** | Not possible | `pulldown-cmark` + `serde_json` compile to WASM — future web UI tree viewer |
| **Distribution** | `pip install` with Python version constraints | Single binary (compiled into `temper` CLI) or standalone WASM module |

### Cargo.toml

```toml
[package]
name = "temper-pageindex"
version = "0.1.0"
edition = "2021"
description = "Hierarchical tree index for markdown documents — reasoning-based retrieval inspired by PageIndex"
license = "MIT"
repository = "https://github.com/tasker-systems/temper"
keywords = ["pageindex", "rag", "markdown", "tree-index", "retrieval"]

[dependencies]
# Markdown parsing — proper AST, not regex
pulldown-cmark = "0.12"

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Error handling
thiserror = "2"

# Timestamps for tree metadata
chrono = { version = "0.4", features = ["serde"] }

[dev-dependencies]
tempfile = "3"
pretty_assertions = "1"
```

Note: **no LLM dependencies**. No `ort`, no `hf-hub`, no `tokenizers`, no `reqwest`. The crate is pure computation. LLM interaction is the consumer's responsibility.

---

## MCP Tool Surface

PageIndex's retrieval pattern maps directly to MCP tool definitions. The `temper-mcp` crate exposes three tree-based tools and one vector-based tool:

### Tool: `vault_search_local`

HNSW vector search on the local index. The fast, traditional semantic search path.

```json
{
  "name": "vault_search_local",
  "description": "Semantic vector search across locally-indexed vault documents. Uses HNSW index with bge-base-en-v1.5 embeddings. Returns ranked results with snippets. Works offline — no cloud connectivity required.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "query": {
        "type": "string",
        "description": "Natural language search query"
      },
      "limit": {
        "type": "integer",
        "description": "Maximum results to return (default 10, max 50)",
        "default": 10
      },
      "context": {
        "type": "string",
        "description": "Filter by context name (e.g., 'temper', 'tasker')"
      },
      "doc_type": {
        "type": "string",
        "description": "Filter by document type (e.g., 'tickets', 'research')"
      }
    },
    "required": ["query"]
  }
}
```

### Tool: `vault_search_tree`

Returns the PageIndex tree structure for document(s). This is the agent's "table of contents" for reasoning-based retrieval.

```json
{
  "name": "vault_search_tree",
  "description": "Get the hierarchical heading structure of vault documents. Returns a tree of section titles, node IDs, and optional summaries — WITHOUT full text content. Use this to understand document structure before retrieving specific sections with vault_get_section. This is the 'agentic vectorless RAG' pattern: reason about the tree, then retrieve what's relevant.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "path": {
        "type": "string",
        "description": "Vault-relative path to a specific document (e.g., 'temper/research/sync-protocol.md')"
      },
      "resource_id": {
        "type": "string",
        "description": "Temper resource ID (UUIDv7) — alternative to path"
      },
      "query": {
        "type": "string",
        "description": "Optional search query to find relevant documents by title/path matching before returning trees"
      },
      "max_documents": {
        "type": "integer",
        "description": "Maximum number of document trees to return (default 5)",
        "default": 5
      }
    }
  }
}
```

### Tool: `vault_get_section`

Retrieves full text content for specific tree nodes. This is the targeted retrieval after the agent has reasoned about the tree.

```json
{
  "name": "vault_get_section",
  "description": "Retrieve the full text content of specific sections from a vault document. Use after inspecting the tree structure from vault_search_tree. You can request one or more sections by node_id.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "resource_id": {
        "type": "string",
        "description": "Temper resource ID of the document"
      },
      "path": {
        "type": "string",
        "description": "Vault-relative path — alternative to resource_id"
      },
      "node_ids": {
        "type": "array",
        "items": { "type": "string" },
        "description": "Node IDs to retrieve content for (from vault_search_tree results)"
      },
      "include_children": {
        "type": "boolean",
        "description": "Include child section content in each node's text (default false)",
        "default": false
      }
    },
    "required": ["node_ids"]
  }
}
```

### Tool: `vault_document_list`

Lists available documents with metadata. The entry point for an agent discovering what's in the vault.

```json
{
  "name": "vault_document_list",
  "description": "List indexed vault documents with metadata. Returns titles, paths, contexts, document types, and node counts. Use to discover what documents are available before using vault_search_tree.",
  "inputSchema": {
    "type": "object",
    "properties": {
      "context": {
        "type": "string",
        "description": "Filter by context name"
      },
      "doc_type": {
        "type": "string",
        "description": "Filter by document type"
      },
      "limit": {
        "type": "integer",
        "description": "Maximum documents to list (default 20)",
        "default": 20
      }
    }
  }
}
```

### Agent Retrieval Flow

The typical agent interaction pattern with these tools:

```
Agent: "Find information about how temper handles sync conflicts"

Step 1: vault_search_local(query: "sync conflict resolution")
  → Returns top-5 chunks ranked by cosine similarity
  → Agent sees snippets mentioning conflicts from multiple documents

Step 2: vault_search_tree(path: "temper/research/sync-protocol.md")
  → Returns tree structure:
    ├── Protocol Overview (node_id: 0001)
    ├── Manifest State Machine (node_id: 0002)
    ├── Conflict Detection (node_id: 0003)
    │   ├── Hash Comparison (node_id: 0004)
    │   └── Merge Policy Evaluation (node_id: 0005)
    ├── Conflict Resolution (node_id: 0006)     ← agent reasons: this is relevant
    │   ├── Manual Resolution (node_id: 0007)   ← and this
    │   └── Auto-Merge Strategy (node_id: 0008) ← and this
    └── Edge Cases (node_id: 0009)

Step 3: vault_get_section(
    path: "temper/research/sync-protocol.md",
    node_ids: ["0006", "0007", "0008"]
  )
  → Returns full text of the three relevant sections

Step 4: Agent synthesizes answer from the retrieved content
```

This is **fundamentally different** from pure vector search. The agent used similarity search (Step 1) to identify the right *document*, then used tree reasoning (Steps 2-3) to identify the right *sections*. The combination is more powerful than either approach alone.

---

## Index Build Pipeline

### Overview

The `temper index` command orchestrates the full local index build:

```
temper index
├── 1. Load config (temper.toml [index] section)
├── 2. Load or create registry.json
├── 3. Scan vault directories
│   ├── Include: [index].include patterns
│   └── Exclude: [index].exclude patterns
├── 4. Scan external sources
│   └── [index].sources paths
├── 5. For each markdown file:
│   ├── 5a. Compute content_hash (SHA-256)
│   ├── 5b. Check registry — skip if unchanged
│   ├── 5c. Parse frontmatter (extract temper-id, metadata)
│   ├── 5d. Build PageIndex tree (pure Rust, pulldown-cmark)
│   ├── 5e. Chunk content (header-based splitting)
│   ├── 5f. Generate embeddings (temper-ingest embed_texts)
│   ├── 5g. Write tree to .temper/trees/{resource-id}.json
│   ├── 5h. Update HNSW index (remove old vectors, insert new)
│   └── 5i. Update registry entry
├── 6. Remove orphaned entries (files deleted from disk)
├── 7. Serialize HNSW to .temper/index.bin
└── 8. Write registry.json
```

### Config Integration

The existing `temper.toml` `[index]` section drives the pipeline:

```toml
[index]
# Directories within the vault to include in the index
include = ["concepts", "sources", "research"]

# Directories to exclude from indexing
exclude = [".git", ".obsidian", "drafts", "docs"]

# External source directories (outside the vault) to also index
sources = [
    "~/projects/writing",
    "~/projects/tasker-systems/storyteller/docs",
]
```

New config additions for R8:

```toml
[index]
# ... existing fields ...

# PageIndex tree configuration
[index.tree]
# Minimum tokens for a node to remain independent during thinning
min_node_tokens = 128

# Maximum heading depth to include (1-6)
max_depth = 6

# Include YAML frontmatter as a metadata node
include_frontmatter = true

# HNSW configuration
[index.hnsw]
# Maximum number of connections per node (M parameter)
# Higher = better recall, more memory
m = 16

# Size of the dynamic candidate list during construction (ef_construction)
# Higher = better index quality, slower build
ef_construction = 200

# Size of the dynamic candidate list during search (ef_search)
# Higher = better recall, slower search
ef_search = 100
```

### Chunking Strategy

Header-based chunking aligns the local index with the cloud chunking strategy. Each chunk boundary occurs at a heading:

```rust
/// A chunk of document content, aligned to heading boundaries.
#[derive(Debug, Clone)]
pub struct ContentChunk {
    /// Chunk identifier: "{file_path}#chunk:{index}"
    pub chunk_id: String,

    /// The temper resource ID of the parent document.
    pub resource_id: String,

    /// The heading path (e.g., "## Protocol Overview > ### State Machine")
    pub header_path: String,

    /// The full text of this chunk (heading + body until next heading).
    pub text: String,

    /// The 768-dim embedding vector (populated after embedding).
    pub embedding: Option<Vec<f32>>,
}
```

Chunks map 1:1 to PageIndex leaf nodes (or merged nodes after thinning). This means the HNSW index and the PageIndex tree reference the same content boundaries — a vector search hit can be cross-referenced to a tree node and vice versa.

### Embedding Pipeline

```rust
/// Embed all chunks for a single document.
///
/// Uses temper-ingest's embed_texts() for batched inference.
/// bge-base-en-v1.5 via ONNX Runtime, 768-dim output.
fn embed_document_chunks(
    chunks: &mut [ContentChunk],
) -> Result<(), IndexError> {
    let texts: Vec<&str> = chunks.iter().map(|c| c.text.as_str()).collect();

    // Batch embed — temper-ingest handles tokenization, inference, pooling, normalization
    let embeddings = temper_ingest::embed::embed_texts(&texts)?;

    for (chunk, embedding) in chunks.iter_mut().zip(embeddings) {
        chunk.embedding = Some(embedding);
    }

    Ok(())
}
```

### External Sources

Files from `[index].sources` are indexed identically to vault files, but tracked separately in the registry with `source: { "type": "external", "path": "/absolute/path" }`. This preserves the legacy behavior where external markdown sources contribute to local search without being part of the managed vault.

---

## Data Model

### Registry v2 Format

The registry evolves from v1 (flat file→chunk mapping) to v2 (resource IDs, tree references, embedding model version):

```json
{
  "version": 2,
  "embedding_model": "BAAI/bge-base-en-v1.5",
  "embedding_dim": 768,
  "hnsw_params": {
    "m": 16,
    "ef_construction": 200
  },
  "last_indexed": "2026-04-01T13:30:00Z",
  "stats": {
    "total_files": 247,
    "total_chunks": 1893,
    "total_tokens": 412650,
    "index_size_bytes": 82419712
  },
  "files": {
    "concepts/embedding-models.md": {
      "resource_id": "01960a3e-7b2c-7000-8000-000000000001",
      "content_hash": "sha256:a1b2c3d4e5f6...",
      "chunk_ids": [
        "concepts/embedding-models.md#chunk:0",
        "concepts/embedding-models.md#chunk:1",
        "concepts/embedding-models.md#chunk:2"
      ],
      "hnsw_ids": [0, 1, 2],
      "tree_file": "01960a3e-7b2c-7000-8000-000000000001.json",
      "source": { "type": "vault" },
      "title": "Embedding Models Overview",
      "context": "temper",
      "doc_type": "concepts",
      "node_count": 8,
      "token_count": 2340,
      "last_indexed": "2026-04-01T13:28:15Z"
    },
    "~/projects/writing/essays/on-search.md": {
      "resource_id": "01960a3e-9d1f-7000-8000-000000000042",
      "content_hash": "sha256:f6e5d4c3b2a1...",
      "chunk_ids": [
        "~/projects/writing/essays/on-search.md#chunk:0"
      ],
      "hnsw_ids": [247],
      "tree_file": "01960a3e-9d1f-7000-8000-000000000042.json",
      "source": {
        "type": "external",
        "path": "/Users/dev/projects/writing/essays/on-search.md"
      },
      "title": "On Search",
      "context": null,
      "doc_type": null,
      "node_count": 3,
      "token_count": 890,
      "last_indexed": "2026-04-01T13:28:16Z"
    }
  }
}
```

### Registry v2 Rust Types

```rust
use std::collections::HashMap;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Registry {
    pub version: u32,
    pub embedding_model: String,
    pub embedding_dim: usize,
    pub hnsw_params: HnswParams,
    pub last_indexed: String,
    pub stats: RegistryStats,
    pub files: HashMap<String, RegistryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HnswParams {
    pub m: usize,
    pub ef_construction: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryStats {
    pub total_files: usize,
    pub total_chunks: usize,
    pub total_tokens: usize,
    pub index_size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    pub resource_id: String,
    pub content_hash: String,
    pub chunk_ids: Vec<String>,
    pub hnsw_ids: Vec<u64>,
    pub tree_file: String,
    pub source: RegistrySource,
    pub title: Option<String>,
    pub context: Option<String>,
    pub doc_type: Option<String>,
    pub node_count: usize,
    pub token_count: usize,
    pub last_indexed: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RegistrySource {
    #[serde(rename = "vault")]
    Vault,
    #[serde(rename = "external")]
    External { path: String },
}

impl Registry {
    /// Create an empty v2 registry with default HNSW params.
    pub fn new() -> Self {
        Self {
            version: 2,
            embedding_model: "BAAI/bge-base-en-v1.5".to_string(),
            embedding_dim: 768,
            hnsw_params: HnswParams {
                m: 16,
                ef_construction: 200,
            },
            last_indexed: String::new(),
            stats: RegistryStats {
                total_files: 0,
                total_chunks: 0,
                total_tokens: 0,
                index_size_bytes: 0,
            },
            files: HashMap::new(),
        }
    }

    /// Check if a file needs re-indexing by comparing content hashes.
    pub fn needs_reindex(&self, path: &str, current_hash: &str) -> bool {
        match self.files.get(path) {
            Some(entry) => entry.content_hash != current_hash,
            None => true, // New file — needs indexing
        }
    }

    /// Migrate a v1 registry to v2 format.
    /// Drops all HNSW IDs (index must be rebuilt with new embeddings)
    /// but preserves content hashes to avoid unnecessary re-parsing.
    pub fn migrate_v1(v1_json: &str) -> Result<Self, serde_json::Error>;
}
```

### Tree Storage Format

Each document's PageIndex tree is stored as a separate JSON file in `.temper/trees/`:

```
.temper/trees/
├── 01960a3e-7b2c-7000-8000-000000000001.json
├── 01960a3e-9d1f-7000-8000-000000000042.json
└── ...
```

File naming uses the resource's UUIDv7, ensuring uniqueness and enabling direct lookup from the registry. The JSON content is the `PageTree` struct serialized via `serde_json`.

A compact tree for a small document:

```json
{
  "title": "Sync Protocol Design",
  "resource_id": "01960a3e-7b2c-7000-8000-000000000001",
  "source_path": "temper/research/sync-protocol.md",
  "content_hash": "sha256:a1b2c3d4e5f6...",
  "built_at": "2026-04-01T13:28:15Z",
  "total_tokens": 2340,
  "description": "",
  "root": {
    "title": "Sync Protocol Design",
    "node_id": "0000",
    "level": 0,
    "line_num": 1,
    "end_line": 142,
    "text": "",
    "token_count": 0,
    "nodes": [
      {
        "title": "Protocol Overview",
        "node_id": "0001",
        "level": 1,
        "line_num": 8,
        "end_line": 35,
        "text": "The sync protocol uses a content-addressed...",
        "token_count": 450,
        "nodes": []
      },
      {
        "title": "Manifest State Machine",
        "node_id": "0002",
        "level": 1,
        "line_num": 36,
        "end_line": 78,
        "text": "Each ManifestEntry tracks five states...",
        "token_count": 680,
        "nodes": [
          {
            "title": "State Transitions",
            "node_id": "0003",
            "level": 2,
            "line_num": 52,
            "end_line": 78,
            "text": "The state machine transitions are...",
            "token_count": 410,
            "nodes": []
          }
        ]
      },
      {
        "title": "Conflict Resolution",
        "node_id": "0004",
        "level": 1,
        "line_num": 79,
        "end_line": 142,
        "text": "When both local and remote have changed...",
        "token_count": 800,
        "nodes": [
          {
            "title": "Manual Resolution",
            "node_id": "0005",
            "level": 2,
            "line_num": 95,
            "end_line": 118,
            "text": "The user resolves conflicts via...",
            "token_count": 380,
            "nodes": []
          },
          {
            "title": "Auto-Merge Strategy",
            "node_id": "0006",
            "level": 2,
            "line_num": 119,
            "end_line": 142,
            "text": "When merge_policy is Auto, the system...",
            "token_count": 310,
            "nodes": []
          }
        ]
      }
    ]
  }
}
```

### HNSW Index Binary

The HNSW graph is serialized to `.temper/index.bin` using the chosen crate's native serialization. Key considerations:

| Concern | Approach |
|---------|----------|
| **Format** | Crate-native binary (e.g., `hnsw_rs::Hnsw::write_bincode()`) |
| **Versioning** | Registry's `hnsw_params` + `embedding_model` + `embedding_dim` serve as the compatibility key. If any change, full rebuild is required. |
| **Portability** | NOT portable across platforms (endianness, pointer sizes). Index is rebuilt on each machine. |
| **Size estimate** | 768 dims × 4 bytes × ~2000 vectors × HNSW overhead ≈ ~30-80MB for a typical vault |
| **Load time** | Deserialization from mmap'd file, sub-second for typical sizes |

### Compatibility with Cloud Data Model

| Property | Cloud (pgvector) | Local (HNSW + PageIndex) |
|----------|-----------------|--------------------------|
| **Embedding model** | bge-base-en-v1.5 (768-dim) | bge-base-en-v1.5 (768-dim) ✅ |
| **Chunk boundaries** | Header-based splitting | Header-based splitting ✅ |
| **Resource IDs** | UUIDv7 in `resources.id` | UUIDv7 in registry + tree `resource_id` ✅ |
| **Content hashes** | SHA-256 in `kb_chunks.content_hash` | SHA-256 in registry `content_hash` ✅ |
| **Access control** | `resources_visible_to()` SQL function | None (local = personal, all visible) |
| **Graph edges** | Future R7 vertex-edge model | Not applicable locally |

---

## Integration with Cloud Sync

### Post-Sync Re-indexing

After `temper sync pull` downloads updated files, the local index becomes stale for those files. The integration:

```
temper sync pull
  ├── Downloads changed files to vault
  ├── Updates manifest entries (content_hash, state → Clean)
  └── Prints: "3 files updated. Run `temper index` to update local search index."

temper index
  ├── Detects changed files via content_hash comparison against registry
  ├── Re-indexes only the changed files (incremental)
  └── Updated index.bin, registry.json, trees/
```

There is **no automatic re-indexing** after sync. The user explicitly runs `temper index`. This keeps the sync and index operations decoupled and predictable.

### Future: Watcher-Triggered Incremental Indexing

When `temper watch` (R6/I7) is running, it could detect `Modified` events and trigger incremental re-indexing of changed files. This is out of scope for R8 but architecturally compatible — the `VaultEvent::Modified` event includes the new `content_hash`, which is exactly what the registry comparison needs.

### Local Index Is Not Uploaded

The local index artifacts (`.temper/index.bin`, `.temper/registry.json`, `.temper/trees/`) are:
- Gitignored (binary, large)
- Not synced to temper-cloud
- Not shared across devices
- Rebuilt independently on each machine

The local index is ephemeral infrastructure, not content.

### Future: Tree Upload for Cloud Enhancement

PageIndex tree structures could theoretically be uploaded to enhance cloud search:
- Tree structures are small (JSON, kilobytes per document)
- Cloud could use trees for structural navigation in addition to vector similarity
- Cloud could pre-generate summaries using server-side LLM (but this contradicts the "user pays LLM costs" principle)

This is explicitly **future work** and not part of this proposal.

---

## Implementation Plan

### Phase 1: `temper-pageindex` Crate — Pure Rust Markdown Tree Parser

**Scope:** Standalone crate, no temper dependencies, independently testable.

| Deliverable | Description |
|-------------|-------------|
| `tree.rs` | `PageTree`, `PageNode`, `TreeConfig` types with serde derives |
| `parser.rs` | `parse_markdown_to_tree()` using `pulldown-cmark` |
| `thinning.rs` | `thin_tree()` bottom-up merge algorithm |
| `content.rs` | `get_node_by_id()`, `get_node_content()`, `tree_structure_only()` |
| `token.rs` | `approximate_token_count()` heuristic |
| `error.rs` | `PageIndexError` with typed variants |
| Test fixtures | Multiple markdown files exercising edge cases |
| `LICENSE-MIT` | MIT license with PageIndex attribution |

**Exit criteria:** `cargo test -p temper-pageindex` passes. Crate has zero non-Rust dependencies. Markdown files round-trip through parse → serialize → deserialize → content extraction.

**Estimated effort:** 2-3 sessions.

### Phase 2: Local HNSW Index Revival

**Scope:** Registry v2 format, HNSW construction, serialization, search query execution.

| Deliverable | Description |
|-------------|-------------|
| Registry v2 types | `Registry`, `RegistryEntry`, `RegistrySource` in `temper-core` |
| v1 → v2 migration | `Registry::migrate_v1()` for existing vaults |
| HNSW wrapper | Thin abstraction over `hnsw_rs` for build, search, serialize, deserialize |
| Content chunker | Header-based splitting aligned with cloud chunking |
| Embedding integration | Wire `temper-ingest::embed_texts()` into the build pipeline |
| Local search function | `search_local(query_embedding, limit) → Vec<LocalSearchResult>` |

**Exit criteria:** Can build an HNSW index from a vault directory, serialize to disk, deserialize, and execute a search query returning ranked results.

**Estimated effort:** 2-3 sessions.

### Phase 3: `temper index` Command Integration

**Scope:** CLI command, incremental indexing, progress reporting.

| Deliverable | Description |
|-------------|-------------|
| `commands/index_cmd.rs` | CLI argument parsing (`--full`, `--status`, `--clear`) |
| `actions/index.rs` | Index build orchestration (scan → hash → embed → store) |
| Progress reporting | File-by-file progress with `indicatif` progress bar |
| `temper search --local` | Add `--local` flag to search command for local-only search |
| Config parsing | Read `[index.tree]` and `[index.hnsw]` from `temper.toml` |

**Exit criteria:** `temper index` builds both HNSW index and PageIndex trees from vault content. `temper search --local "query"` returns results from the local index. `temper index --status` shows index statistics.

**Estimated effort:** 2-3 sessions.

### Phase 4: MCP Tool Integration

**Scope:** Expose local search and tree retrieval as MCP tools in `temper-mcp`.

| Deliverable | Description |
|-------------|-------------|
| `vault_search_local` tool | HNSW search via MCP |
| `vault_search_tree` tool | Tree structure retrieval via MCP |
| `vault_get_section` tool | Node content extraction via MCP |
| `vault_document_list` tool | Document listing with metadata via MCP |
| Integration tests | MCP tool round-trips with test vault |

**Exit criteria:** Claude Desktop (or equivalent MCP client) can search the local vault, browse document trees, and retrieve specific sections — all without cloud connectivity.

**Estimated effort:** 2-3 sessions.

### Phase 5: Combined Local Search (HNSW + Tree Reasoning)

**Scope:** Integrate both retrieval strategies into a unified local search experience.

| Deliverable | Description |
|-------------|-------------|
| Hybrid search flow | Vector search to identify documents, tree reasoning to identify sections |
| `vault_search_smart` tool | MCP tool that combines both strategies with guided retrieval prompts |
| Agent prompt engineering | System prompts that teach the agent the two-step retrieval pattern |
| Documentation | User guide for local search setup and MCP integration |

**Exit criteria:** An MCP-connected agent can execute the full hybrid retrieval flow (vector search → tree navigation → section extraction) in a single conversation turn.

**Estimated effort:** 1-2 sessions.

---

## Risk Analysis

### Technical Risks

| Risk | Severity | Likelihood | Mitigation |
|------|----------|------------|------------|
| **ONNX model download on first index** | Medium | High | `temper-ingest` already handles model download from HuggingFace hub (~420MB for bge-base-en-v1.5). Document the requirement. Show download progress. Cache in `~/.cache/huggingface/`. |
| **Index build time for large vaults** | Medium | Medium | Parallel file processing via `rayon`. Incremental indexing skips unchanged files. Typical vault (200-500 files) should index in under 2 minutes. |
| **HNSW memory usage during build** | Low | Low | 768-dim × 4 bytes × 5000 vectors = ~15MB of vector data. HNSW graph overhead adds ~2-3x. Total: ~50MB RAM during build. Acceptable. |
| **pulldown-cmark heading edge cases** | Low | Medium | Code blocks containing `#` lines, HTML headings, setext-style headings. `pulldown-cmark` handles all of these correctly via AST (unlike regex). Add test fixtures for edge cases. |
| **HNSW crate serialization stability** | Medium | Low | Pin `hnsw_rs` version in `Cargo.toml`. If format changes, registry's `hnsw_params` detect incompatibility and trigger full rebuild. |
| **Token count heuristic accuracy** | Low | High | The `bytes / 4` heuristic is approximate. Acceptable for tree thinning decisions. Consumers needing precise counts should use their own tokenizer. |
| **Large files with deep heading nesting** | Low | Low | Some files may have 100+ headings across 6 levels. Tree thinning handles this by merging small nodes. Set reasonable defaults (`min_node_tokens: 128`). |

### Architectural Risks

| Risk | Severity | Mitigation |
|------|----------|------------|
| **Embedding model divergence** | High | Registry stores `embedding_model` and `embedding_dim`. If cloud changes models, local index detects the mismatch and requires rebuild. Alignment is enforced by convention, not code. |
| **Two indexes = two maintenance burdens** | Medium | Local index is opt-in (`temper index` is explicit). Users who only need cloud search never interact with local index infrastructure. |
| **MCP tool proliferation** | Low | Four tools is manageable. Group under `vault_*` namespace. Document the intended flow (list → tree → section). |
| **PageIndex tree structures grow stale** | Medium | Trees include `content_hash`. Staleness is detectable. `temper index` rebuilds stale trees. Watcher integration (future) automates this. |

---

## Alternatives Considered

### 1. Cloud Search for Everything (Status Quo)

**Approach:** Keep `temper search` cloud-only. Accept that offline search is unavailable.

**Pros:** No additional complexity. Single search implementation. No local index maintenance.
**Cons:** Zero offline capability. Agent workflows pay cloud costs. No reasoning-based retrieval. The vault files are right there — not being able to search them locally is a significant UX gap.

**Verdict:** Insufficient. Offline capability and agent-local search are real requirements.

### 2. SQLite FTS5 for Local Search

**Approach:** Use SQLite with FTS5 (full-text search) as the local search engine instead of HNSW.

**Pros:** No embedding model dependency. Fast keyword search. Tiny index size. SQLite is ubiquitous.
**Cons:** Keyword matching, not semantic search. "embedding pipeline architecture" wouldn't match a document about "vector inference systems." The cloud tier uses semantic search — local should match the modality. FTS5 could complement semantic search but not replace it.

**Verdict:** Could be a useful addition (especially for exact term search) but doesn't replace semantic search. Consider as future enhancement.

### 3. Ship Embeddings from Cloud

**Approach:** Download pre-computed embeddings from temper-cloud and build the local HNSW index from those, avoiding local ONNX inference entirely.

**Pros:** No local model download. Guaranteed embedding alignment. Faster index build.
**Cons:** Requires cloud connectivity to build local index (defeats the offline purpose). Embeddings for external sources aren't in the cloud. Adds API surface for embedding download.

**Verdict:** Interesting hybrid for future work (pre-seeding local index from cloud) but can't be the primary strategy.

### 4. Standard RAG Chunking Without PageIndex

**Approach:** Just do HNSW vector search locally. Skip the tree structure entirely.

**Pros:** Simpler implementation. No tree storage. Fewer MCP tools.
**Cons:** Misses the key insight: similarity ≠ relevance. Vector search alone returns similar chunks but may miss the contextually relevant sections. The tree structure is free to build (pure markdown parsing) and provides significant value for agent-driven retrieval.

**Verdict:** HNSW alone is Phase 2. PageIndex tree is additive value at low cost. Include both.

### 5. Use PageIndex Python Package Directly

**Approach:** Shell out to the PageIndex Python package instead of porting to Rust.

**Pros:** No port needed. Get PageIndex features immediately.
**Cons:** Python dependency in a Rust project. Python environment management. Performance overhead for tree building. Can't compile to WASM. Can't share types with the rest of the temper codebase. The porting effort is modest — PageIndex is ~500 lines of Python, and the tree-building logic is straightforward.

**Verdict:** Not worth the Python dependency. The Rust port is small and yields better integration.

---

## Open Questions

### Q1: Local Embedding Strategy — ONNX Inference vs API Call

The local index requires embeddings. Two options:

- **Local ONNX** (current `temper-ingest` approach): Download bge-base-en-v1.5 ONNX model (~420MB), run inference locally. No network needed after initial download. CPU inference is ~50ms per text on M-series Apple Silicon.
- **API call**: Send text to OpenAI/Anthropic/local Ollama embedding endpoint. Requires network (unless Ollama). Different model = different embeddings = not aligned with cloud.

**Recommendation:** Local ONNX is the right default. It's already implemented in `temper-ingest`. The model download happens once. Inference is fast enough for index-time batch processing.

### Q2: Tree Summary Generation — On-Demand vs At Index Time

PageIndex optionally generates LLM summaries per tree node. When should this happen?

- **At index time**: `temper index --summarize` generates summaries for all nodes. Requires LLM API key configured. Slow (one LLM call per node). Summaries stored in tree JSON.
- **On-demand**: Summaries generated when an agent first requests a tree structure. Cached in the tree JSON for subsequent requests. Lazy, but requires LLM availability at query time.
- **Never (agent reasons from titles only)**: The agent gets titles and node_ids, reasons from those, and retrieves full text. No summaries needed.

**Recommendation:** Start with "never" (titles only). The agent is an LLM — it can reason from a table of contents without pre-generated summaries. Add `--summarize` as an optional enrichment in a future iteration if the title-only approach proves insufficient.

### Q3: Maximum Vault Size for Reasonable Index Times

At what vault size does `temper index` become unreasonably slow?

- **Embedding is the bottleneck**: ~50ms per chunk on Apple Silicon. 2000 chunks = 100 seconds. 10000 chunks = 500 seconds (~8 minutes).
- **Tree building is fast**: Pure parsing, ~1ms per file. 500 files = 0.5 seconds.
- **HNSW construction is moderate**: O(n log n) for n vectors. 5000 vectors = ~2 seconds.

For a typical knowledge worker vault (200-500 markdown files, ~2000-5000 chunks), incremental indexing completes in seconds (only changed files). Full rebuild completes in 1-3 minutes.

**Threshold:** Vaults with >10,000 chunks (~2000+ files) may need optimization (batched GPU inference, parallel ONNX sessions). Flag this as a scaling concern.

### Q4: Should `temper search --local` Be the Default When Offline?

When `temper search "query"` is invoked and the cloud API is unreachable, should it automatically fall back to local search?

- **Yes (transparent fallback)**: Better UX. User always gets results. Risk: user may not realize they're seeing local-only results (potentially stale).
- **No (explicit flag only)**: User must pass `--local`. Clear about what they're getting. Risk: worse UX when offline.

**Recommendation:** Transparent fallback with a clear warning: `"⚠ Cloud unreachable — showing local results (indexed 2026-04-01T13:30:00Z)"`. The user sees results and knows they're local.

### Q5: Should External Sources Get PageIndex Trees?

External sources (from `[index].sources`) are markdown files outside the vault. Should they get PageIndex trees?

**Recommendation:** Yes. If they're markdown with headings, tree construction works identically. The tree files go in `.temper/trees/` like vault files. No reason to exclude them.

---

## Decision Log

| # | Decision | Choice | Rationale |
|---|----------|--------|-----------|
| D1 | HNSW crate | `hnsw_rs` (pure Rust) | No C++ toolchain dependency, serde support, sufficient performance for vault-scale |
| D2 | Embedding model | bge-base-en-v1.5 (768-dim) | Must match cloud pgvector index. Already implemented in `temper-ingest`. |
| D3 | Markdown parser | `pulldown-cmark` | Proper AST parsing, not regex. Handles code blocks, HTML, frontmatter correctly. |
| D4 | Crate design | Standalone `temper-pageindex` | Independently testable, no LLM dependency, could be published as open-source |
| D5 | LLM cost model | User pays own costs | PageIndex tree traversal uses the agent's LLM. temper-cloud never pays for local search LLM inference. |
| D6 | Tree summaries | Titles only (no LLM summaries) for v1 | Agent can reason from titles. Add `--summarize` enrichment later if needed. |
| D7 | Registry format | v2 with resource IDs, tree references, model version | Forward-compatible. v1 migration preserves content hashes. |
| D8 | Index trigger | Explicit `temper index` only | No automatic re-indexing. Watcher integration is future work (R6/I7). |
| D9 | Offline fallback | Transparent with warning | `temper search` falls back to local when cloud is unreachable. Clear warning shown. |
| D10 | Tree storage | Per-document JSON in `.temper/trees/` | Simple, inspectable, individually updatable. Named by resource UUIDv7. |
| D11 | MCP tool design | Four tools: search_local, search_tree, get_section, document_list | Covers the full retrieval flow: discover → navigate → retrieve. |
| D12 | Chunk alignment | Header-based splitting, same boundaries as cloud | Chunks map 1:1 to tree nodes. Vector hits cross-reference to tree nodes. |

---

## Appendix A: PageIndex Attribution

This research and implementation is inspired by and partially derived from:

**PageIndex: Agentic Vectorless RAG**
- **Repository:** [VectifyAI/PageIndex](https://github.com/VectifyAI/PageIndex)
- **License:** MIT
- **Authors:** VectifyAI
- **Paper:** "PAGEINDEX: Agentic Vectorless RAG via LLM Reasoning and Hierarchical Page Indexing"

### What Was Ported

The following components were ported from PageIndex's Python implementation to Rust:

| Component | Python Source | Rust Destination | Nature of Port |
|-----------|-------------|-----------------|----------------|
| Heading extraction & tree building | `page_index_md.py` | `temper-pageindex/src/parser.rs` | Algorithmic port (regex → pulldown-cmark AST) |
| Tree thinning | `page_index_md.py` | `temper-pageindex/src/thinning.rs` | Direct port of merge algorithm |
| Node content extraction | `page_index_md.py` | `temper-pageindex/src/content.rs` | Algorithmic port (string slicing → zero-copy) |
| Tree data structure | `page_index_md.py` | `temper-pageindex/src/tree.rs` | Type-safe reimplementation |

### What Was NOT Ported (Independently Implemented)

| Component | Reason |
|-----------|--------|
| LLM integration | The MCP agent IS the LLM — no separate LLM client needed |
| Retrieval tools | Implemented as MCP tool definitions, not Python functions |
| Token counting | Uses byte-length heuristic, not tiktoken |
| Configuration | Uses temper's existing TOML config, not PageIndex's JSON config |
| Summary generation | Deferred to consumer (agent/CLI plugin) |

### License Compliance

The `temper-pageindex` crate is licensed under MIT, consistent with the original PageIndex license. The `LICENSE-MIT` file in the crate root includes attribution to VectifyAI/PageIndex.

---

## Appendix B: Registry v2 Full Schema

### Migration from v1 to v2

The v1 registry format:

```json
{
  "version": 1,
  "last_indexed": "2026-03-29T20:58:18Z",
  "files": {
    "path/to/file.md": {
      "content_hash": "sha256:...",
      "chunk_ids": ["path/to/file.md#chunk:0", "path/to/file.md#chunk:1"],
      "source": { "type": "vault" },
      "last_indexed": "2026-03-26T20:00:03Z"
    }
  }
}
```

Migration rules:
1. Set `version: 2`
2. Add `embedding_model: "BAAI/bge-base-en-v1.5"` and `embedding_dim: 768`
3. Add default `hnsw_params: { m: 16, ef_construction: 200 }`
4. For each file entry:
   - Generate `resource_id` from frontmatter `temper-id` if available, else mint new UUIDv7
   - Set `hnsw_ids: []` (HNSW must be rebuilt with new embedding model)
   - Set `tree_file: "{resource_id}.json"` (tree must be built)
   - Preserve `content_hash` (avoids re-hashing unchanged files)
   - Set `node_count: 0`, `token_count: 0` (populated on next index)
5. Initialize empty `stats` (populated on next index)

The migration preserves content hashes so that `temper index` after migration only re-embeds files (for the new model) without re-parsing unchanged content.

---

## Appendix C: Related Tickets & Dependencies

| Ticket | Relationship | Status |
|--------|-------------|--------|
| **R2** — Data Model & Schema Design | Foundation — resource IDs, content hashing, chunk model | ✅ Done |
| **R5** — Indexing, Sync & Resource Management | Foundation — registry format, search types, sync protocol | ✅ Done |
| **R6** — Filesystem Watcher | Future integration — watcher events trigger incremental re-indexing | Research complete |
| **I5d** — Cloud-Routed Search | Foundation — `temper-ingest` embedding, cloud search API | ✅ Done |
| **I5e** — Local KB Restructure | Foundation — vault layout, manifest, `.temper/` directory structure | ✅ Done |
| **I6a** — Sync Infrastructure | Foundation — sync protocol, manifest operations | ✅ Done |
| **R7** — Knowledge Graph | Parallel — graph traversal on cloud, tree traversal locally | Research |
| **MCP** — Agent workflow server | Consumer — `temper-mcp` will expose local search tools | Stub crate exists |

### New Tickets to Create

| Ticket | Scope | Phase |
|--------|-------|-------|
| **I8a** — `temper-pageindex` Crate | Pure Rust markdown tree parser, types, tests | Phase 1 |
| **I8b** — Local HNSW Index Revival | Registry v2, HNSW build/search/serialize, embedding integration | Phase 2 |
| **I8c** — `temper index` Command | CLI integration, incremental indexing, progress, config | Phase 3 |
| **I8d** — MCP Local Search Tools | Four MCP tools for local vault search and tree retrieval | Phase 4 |
| **I8e** — Combined Search & Agent Flow | Hybrid HNSW + tree retrieval, smart MCP tool, agent prompts | Phase 5 |