# Temper Cloud — Epic Design Spec

## Vision

Transform temper from a local-only CLI into a cloud-native knowledge base system where Postgres owns structured metadata and lifecycle state, git owns document content and history, and the two reconcile through temper as the intervention layer. The system supports single-developer use today with a path to small-team collaboration via GitHub auth. Document types are composable behaviors on a unified resource model, not hard-coded directory conventions.

## Guiding Constraints

- **Continuity**: Temper continues to function locally throughout — no dark period where the CLI is broken
- **Content over tooling**: The knowledge base is the unit of value, not the tool — migration path preserves all existing content
- **Research before implementation**: Each implementation phase is grounded in prior research findings
- **Extensible by data**: New document types are a data operation (database, filepath, frontmatter), not a code change

## Key Decisions

### ~~Dual Authority Model~~ → Single Authority (Postgres)

> **R2 Pivot (2026-03-26):** During R2 data model design, the dual-authority model was replaced by Postgres as single source of truth. Documents are recomposable from versioned chunks in Postgres, making git an optional materialization layer rather than a content authority. This simplifies reconciliation (6 drift types → push/pull sync), enables multi-tenancy via scoping, and unblocks Apache AGE knowledge graph integration. See `docs/superpowers/specs/2026-03-26-r2-data-model-and-schema-design.md`.

~~Git and Postgres are both authoritative, for different things:~~
~~- **Git**: Document content, prose, version history, collaboration via PRs~~
~~- **Postgres**: Structured metadata, lifecycle state, search vectors (pg_vector), user/author tracking, type behaviors~~

**Postgres** is authoritative for everything — content (via versioned chunks), metadata, vectors, events. Git and local filesystem are an optional **materialization layer**: a convenient way to have the knowledge base on disk as markdown for agents, editors, and Obsidian. `temper sync push` sends local edits to Postgres. `temper sync pull` materializes files from Postgres. Postgres always wins ties.

### Resource Model

> **R1/R2 Update:** Three resource kinds (not two) with a single `resources` table and `resource_kind` enum discriminator.

Three resource kinds stored in a single `resources` table:

- **IndexableResource**: External content referenced by URI. Lives only in Postgres (metadata + vectors). No git file. Example: a blog post at `https://example.com/post`.
- **IngestedResource**: External content fetched, converted to markdown via kreuzberg v4, stored as a git-managed file with provenance chain. Example: a PDF converted to markdown.
- **KnowledgeBaseResource**: Authored natively in the knowledge base. Git-managed markdown with YAML frontmatter. Example: a ticket, session note, research doc.

Whether something is a ticket, milestone, research doc, decision, or any future type is metadata on the resource (`kb_doc_types` table), not a function of directory path. Adding a new type requires only database records (type + behavior composition), a template, and a directory convention — no code changes.

### Composable Behaviors

Document types compose behaviors rather than inheriting from fixed schemas:
- **Workflowable**: Has lifecycle stages (backlog → in-progress → done → cancelled). Tickets compose this; research docs don't.
- **Sequenceable**: Has a `seq` field for ordering within a parent. Tickets and milestones compose this.
- **Assignable**: Has author and assignee fields. Tickets compose this.
- **Taggable**: Open-field metadata for annotation. All resources compose this.

New behaviors can be introduced and composed onto existing or new types.

### Branch-Based Cutover

The workspace restructure into crates will happen on a feature branch. Temper may be temporarily broken on that branch while imports are reorganized. Main stays functional until the branch merges.

### Auth Model

GitHub OAuth for API security and author identification. Authentication is 1:1 with authorization for now — if you can authenticate, you have full access. Single-user initially, with author tracking that sets up multi-user later.

## Research Phase

Five workstreams that produce findings before any implementation begins. Each produces research documents, decision records, or design artifacts saved to the knowledge base.

### R1: Workflow & Lifecycle Vision

**Question**: What does it look like, step by step, for one or more developers and agents to use Temper Cloud day to day? What are the responsibility boundaries between Postgres and git, and how do they intersect and reconcile?

**Outputs**:
- Narrative walkthrough of key workflows: create a ticket, search across projects, pick up a session on a new machine, agent indexes external research, two developers contribute to the same project (multi-developer walkthrough is for informing data model extensibility only — not for driving implementation phase work)
- Responsibility matrix: for each operation, which system is authoritative, which is derived, and what triggers reconciliation
- Edge case catalog: what happens on conflict, stale cache, offline work, partial sync failure
- Decision: what requires human-in-the-loop vs. what can be automated

### R2: Data Model & Schema Design

**Question**: What are the Postgres schemas and relationships that support the workflows and resource model described in R1?

**Depends on**: R1

**Outputs**:
- Resource model: IndexableResource and KnowledgeBaseResource as foundational types, with type-as-metadata and composable behaviors
- Schema design: tables, relationships, pg_vector column strategy, frontmatter-to-Postgres field mapping
- Drift detection design: how frontmatter on disk and Postgres metadata stay in agreement, what constitutes drift, how it's resolved
- UUIDv7 and content-hash (sha2) strategy for identity and change detection
- Migration plan: how existing markdown files, frontmatter, and HNSW indexes map to the new Postgres schema — preserving all existing knowledge base content

### R3: Deployment Platform Evaluation

**Question**: Given the workflow goals from R1 and the data model from R2, should Temper Cloud deploy on Vercel, Shuttle.dev, or another platform?

**Depends on**: R1, R2

**Outputs**:
- Side-by-side comparison: Vercel vs Shuttle.dev (and any others surfaced during research)
- Evaluation axes: Rust-native support, cold start latency, Postgres connectivity, git operations from serverless context, cost model, deployment ergonomics, long-running process support
- Spike implementation: minimal Axum endpoint on each candidate talking to Neon Postgres
- Recommendation with rationale

### R4: Crate Architecture & Auth Design

**Question**: Given the workflow, data model, and deployment target, what are the crate boundaries, how does the workspace restructure accommodate them, and how does auth work end-to-end?

**Depends on**: R1, R2, R3

**Outputs**:
- Crate boundary map: temper-core, temper-cli, temper-tui, temper-api, temper-mcp, temper-cloud — what lives where, dependency graph
- Trait boundaries: the backend abstraction that lets local-file and Postgres coexist
- Auth flow design: GitHub OAuth from CLI (browser redirect and back), token management, propagation through API calls
- MCP protocol adapter: how temper-mcp composes temper-core for agent access

### R5: Indexing, Sync & Resource Management

**Question**: How do local and remote indexing work together? How do external resources get tracked? What can be automated vs. what needs human review?

**Depends on**: R2, R3, R4

**Outputs**:
- Embedding model strategy: what model, what dimensionality, how it relates to the current candle-based local pipeline, and implications for schema and deployment resources
- Indexing architecture: can stream-based chunking/vectorization work serverless, or must the CLI do local embedding and push vectors to the API?
- Sync model: does the cloud deployment need a full checkout, a shallow clone, or can it work stream-based against the git remote?
- Local HNSW role: persist as local cache/offline fallback, or fully replaced by pg_vector?
- External resource model: FQDN/URL tracking, what "indexing an external resource" means in practice, deferred transport layer
- Process model: is `temper-cloud` a long-running service crate, or does the deployment platform compose `temper-api` + `temper-mcp` as serverless functions directly?

### Research Dependency Graph

```
R1 (Workflow) ──→ R2 (Data Model) ──→ R4 (Crates & Auth)
      │                 │                      │
      └────────→ R3 (Deployment) ──────→ R4    ↓
                        │               R5 (Indexing & Sync)
                        │                 ↑    ↑
                        └─────────────────┘    │
                                          R2 ──┘
```

R1 is the foundation. R2 and R3 can proceed in parallel once R1 is complete. R4 and R5 depend on earlier findings.

## Implementation Phase

Five workstreams that execute after research phases produce their findings. Each is grounded in specific research outputs.

### I1: Workspace Restructure & Crate Extraction

**Grounded in**: R4 (crate boundaries)

**Goal**: Restructure the repo into `crates/{temper-core, temper-cli, temper-tui, temper-api, temper-mcp, temper-cloud}` with trait boundaries for the backend abstraction. Stubs for crates that don't have implementation yet. The `temper` binary still works at the end — same functionality, new structure.

**Key concerns**:
- HNSW and embedder logic land in temper-core
- TUI is its own crate, composed by temper-cli for the single binary
- Backend trait defined so local-file and Postgres are swappable

### I2: Postgres & pg_vector — Local Development

**Grounded in**: R2 (schema design), R5 (indexing architecture)

**Goal**: Implement the Postgres backend behind the trait boundary from I1. Run against a local Postgres. Migrate the existing knowledge base into the new schema. Test ownership boundaries and reconciliation approaches in practice.

**Key concerns**:
- Frontmatter ↔ Postgres sync and drift detection
- Content-hash based change detection
- pg_vector replacing HNSW for search
- Embedding pipeline adaptation

### I3: API & Auth — Local Verification

**Grounded in**: R3 (deployment platform), R4 (auth design)

**Goal**: Axum API handlers in temper-api, GitHub OAuth flow from CLI, token management. Verified locally before deploying.

**Key concerns**:
- CLI-to-browser-and-back auth flow
- Token storage and refresh
- API route design that works for both direct calls and MCP

### I4: First Cloud Deployment

**Grounded in**: R3 (platform choice), R5 (sync model)

**Goal**: Deploy to the chosen platform with remote Neon Postgres, API endpoints, and git-remote actions.

**Key concerns**:
- Cold start performance
- Git operations from serverless context
- Whether temper-cloud needs local disk or can work stream-based
- Connection pooling to Neon
- Shallow clone viability vs. stream-based doc processing vs. local-only checkout model

### I5: MCP & Agent Integration

**Grounded in**: R4 (MCP adapter design), I3 (API), I4 (deployment)

**Goal**: temper-mcp as an authenticated MCP server that agents can use to read, write, and search the knowledge base using temper's workflows.

**Key concerns**:
- MCP protocol compliance
- Auth token propagation
- Which operations are safe for autonomous agent use vs. requiring confirmation

### Implementation Dependency Graph

```
I1 (Crate Restructure) ──→ I2 (Postgres Local) ──→ I4 (First Deploy)
          │                                                │
          └──→ I3 (API & Auth) ──────────────────→ I4     │
                                                           │
                                                    I5 (MCP & Agents)
```

## Open Questions

These are captured here for tracking. Each should be resolved by the research phase it belongs to.

- **R1**: What is the reconciliation strategy when a user edits a file directly via git (bypassing temper) and the frontmatter diverges from Postgres state?
- **R1**: What operations should agents be able to perform autonomously vs. with confirmation?
- **R2**: Should composable behaviors be modeled as Postgres columns, JSONB fields, or a separate behaviors table?
- **R3**: Can Rust serverless functions on Vercel/Shuttle perform git operations (clone, push) within acceptable latency and resource constraints?
- **R4**: Does temper-cloud exist as its own crate or is it just temper-api + temper-mcp composed by the deployment platform?
- **R5**: Is a shallow clone sufficient for cloud-side operations, or does the cloud deployment need full history for drift detection?
- **R5**: Can embedding generation happen serverless, or must it always be CLI-side with vectors pushed to the API?

## Focused Scope Reminder

Temper is not a Linear competitor. It is a unified knowledge base, cross-project session store, and lightweight ticketing system. It supports rapid ideation-to-delivery for single developers and small teams. The cloud migration extends this to multi-machine, multi-agent access — not to org-level coordination, burndown charts, or team management.
