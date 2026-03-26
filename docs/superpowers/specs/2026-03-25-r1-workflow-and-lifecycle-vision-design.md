# R1: Workflow & Lifecycle Vision — Research Design

## Purpose

Map the day-to-day workflows and responsibility boundaries for Temper Cloud. This is the foundational research that all other workstreams (R2–R5) depend on. The deliverable is a research note saved to the knowledge base, structured in three layers that serve different downstream consumers.

## Scope Corrections

During design, several corrections to the epic design spec were identified:

1. **Temper is the intended write path**: The design spec says "direct git edits and direct Postgres mutations are both valid." This is too permissive. The correct model: temper is the intended write path for both git and Postgres. Out-of-band changes in either system are possible and must be handled, but they are edge cases — not a primary workflow.

2. **No human-in-the-loop reconciliation**: All drift is resolved automatically. Last-write-wins for metadata (keyed on `updated` timestamp). Append-oriented document mutations minimize content conflicts. When something surprising happens, temper annotates the reconciliation in the event log rather than halting for human input.

3. **Workflow 5 replaced**: The original "two developers contribute to the same project" walkthrough has been replaced with "add a new document type." Multi-developer ticket coordination is explicitly out of scope — temper is not a distributed ticketing system. The extensibility concern is better served by exercising the composable resource model directly.

4. **Client-side URI resolution**: The cloud server does not fetch documents from arbitrary locations. IndexableResources carry URI pointers (file://, https://, s3://) that clients resolve locally. The server stores metadata, search vectors, and lifecycle state.

5. **1:1 deployment model**: One knowledge base = one git repo = one Postgres instance = one temper-cloud deployment. This is not a multi-tenant or multi-vault system.

## Deliverable Structure

Three layers, each building on the previous:

### Layer 1: Walkthroughs

Five narratives told as concrete step-by-step stories. Each step shows what happens on three axes: **client** (CLI/TUI), **git** (local repo + remote), and **Postgres** (cloud API + database).

#### Workflow 1: Create a ticket (CLI → Postgres + git)

Developer runs `temper ticket create` from their local machine. Steps cover:
- Local file write (markdown to vault working tree)
- Postgres metadata write via cloud API (stage, seq, milestone, timestamps, vectors)
- Event log append
- Git commit at developer's discretion (or via hooks)
- Push to remote, confirming git and Postgres agreement
- Offline variant: local write succeeds, Postgres sync queued, reconciliation on reconnect

Key questions this walkthrough answers:
- What is the minimum viable operation when offline?
- When does the Postgres record come into existence relative to the git commit?
- What happens if the developer never pushes?

#### Workflow 2: Search across projects

Developer runs `temper search` or uses TUI search. Steps cover:
- Online path: query → cloud API → pg_vector similarity search → ranked results with URI pointers
- Client resolves URIs locally (file:// for vault docs, https:// for web resources, s3:// for cloud storage)
- Offline fallback: query → local HNSW index → same result format, potentially stale
- Results may include both KnowledgeBaseResources (in the vault) and IndexableResources (external, Postgres-only)

Key questions this walkthrough answers:
- How does the client decide whether to use cloud or local search?
- What does the result format look like when mixing vault docs and external resources?
- What's the degraded experience when offline?

#### Workflow 3: Pick up a session on a new machine

Developer clones the knowledge base repo on a fresh machine. Steps cover:
- Clone detects vault via `temper.toml`
- GitHub OAuth authentication with temper-cloud
- CLI configured to use cloud API for search and metadata
- All sessions, tickets, milestones immediately available in git checkout (TUI, Obsidian, any editor)
- Optional `temper index` builds local HNSW cache for offline use
- Subsequent workflow: create a session, search, edit tickets — all working against cloud API with local git

Key questions this walkthrough answers:
- What's the minimum setup to be productive? (clone + auth)
- What local state needs to be built vs. what comes from the cloud?
- How does the TUI work before local index is built?

#### Workflow 4: Agent indexes external research

An agent (via MCP) authenticates with temper-cloud and registers external content. Steps cover:
- MCP authentication (token propagation)
- API call to register IndexableResource with https:// URI, metadata (title, provenance, tags)
- Agent provides content inline for embedding (the server does not fetch from URIs — clients resolve URIs)
- Postgres creates record: metadata + pg_vector embeddings
- No git file created — this is a Postgres-only IndexableResource
- Resource appears in search results with its URI; human developer can retrieve content via their client

Key questions this walkthrough answers:
- What's the boundary between what agents can do autonomously vs. what needs confirmation?
- How does an IndexableResource differ from a KnowledgeBaseResource in practice?
- What metadata is required vs. optional for external resources?

#### Workflow 5: Add a new document type

A developer wants to add "decision records" as a new document type. Steps cover:
- Define the type's composed behaviors: Workflowable (has lifecycle stages), Taggable (open metadata). Not Sequenceable (no ordering within parent).
- Create Postgres records: type definition with behavior composition
- Establish filepath convention: `decisions/<project>/YYYY-MM-DD-slug.md`
- Create template: `templates/decision.md` with frontmatter schema
- First document created via `temper note create decision "API versioning strategy"` — writes markdown file, creates Postgres metadata, embeds for search
- Existing search, context, TUI browse all pick up the new type with no code changes

Key questions this walkthrough answers:
- What data operations are needed to add a type? (Postgres + template + filepath convention)
- What code changes are needed? (None — that's the point)
- How do existing commands (search, context, list) handle unknown types gracefully?

### Layer 2: Responsibility Matrix

A single reference table synthesized from the walkthroughs. Covers every current temper command plus new cloud operations.

Columns:
- **Operation**: the temper command or system action
- **Git (local)**: what happens in the local working tree
- **Git (remote)**: what happens on push/pull
- **Postgres**: what happens in the cloud database
- **Authority**: which system is the source of truth for this operation's data
- **Reconciliation trigger**: what causes the systems to sync

Principles the matrix encodes:
- Temper is the intended write path for both backends
- Git is authoritative for document content; Postgres for structured metadata and search vectors
- Last-write-wins for metadata conflicts, keyed on `updated` timestamp
- Append-oriented mutations for document content to minimize merge conflicts
- Local HNSW is a cache; pg_vector is canonical when online

The matrix will cover:
- All existing commands: ticket (create, move, done, list, show), session (save, list), milestone (create, list, update), search, context, index, normalize, note create, research save, warmup, events, status, project (add, remove, list)
- New cloud operations: authenticate, register IndexableResource, sync (push metadata to cloud), pull (refresh local from cloud), webhook processing

### Layer 3: Reconciliation Model

Edge cases organized by drift type. Each entry: what it looks like, how temper detects it, how temper resolves it automatically.

#### Metadata drift
Frontmatter on disk disagrees with Postgres (e.g., `stage: backlog` locally, `stage: in-progress` in Postgres). Caused by out-of-band git edit or delayed sync.

- **Detection**: content-hash comparison (SHA-256) on sync — leverages the existing registry system
- **Resolution**: compare `updated` timestamps, last-write-wins. Overwrite the loser. Annotate in event log.

#### Content drift
Markdown body changed via direct git edit (vim, Obsidian, IDE) without going through temper.

- **Detection**: content-hash mismatch in registry on next sync or index run
- **Resolution**: git is authoritative for content. Re-embed from new content, update Postgres vectors. No conflict — just re-indexing.

#### Index staleness
Local HNSW doesn't reflect recent changes (new IndexableResources via MCP, writes from another machine).

- **Detection**: local registry `last_indexed` timestamp vs. cloud API's latest change timestamp
- **Resolution**: client uses cloud search when online (staleness is invisible). Local HNSW rebuild on demand or background refresh. Cache freshness problem, not a data integrity problem.

#### Partial sync failure
Git push succeeds but Postgres write fails (or vice versa) due to network interruption.

- **Detection**: sync queue in `.temper/` tracks pending operations
- **Resolution**: retry on next startup or connectivity. Idempotent writes — creating a record that exists updates it, updating with the same timestamp is a no-op. Event log annotates the retry.

#### Orphaned references
An IndexableResource's URI becomes unreachable (file deleted, URL 404, S3 object removed).

- **Detection**: periodic or on-demand validation pass, client-side (clients resolve URIs)
- **Resolution**: mark resource as `unreachable` in Postgres metadata. Retain the record and search vectors — metadata and prior content still have value. Surface in `temper status` or maintenance view.

#### Out-of-band Postgres mutation
Raw SQL or admin tooling modifies Postgres directly, bypassing temper.

- **Detection**: on sync, frontmatter-to-Postgres comparison reveals mismatch with no matching event log entry
- **Resolution**: same as metadata drift — last-write-wins by `updated` timestamp, annotate the reconciliation.

## Downstream Consumers

| Downstream | Primary layer consumed |
|------------|----------------------|
| R2: Data Model & Schema Design | Layer 2 (responsibility matrix) — what the schema must support |
| R3: Deployment Platform Evaluation | Layer 1 (walkthroughs) — what the platform must enable |
| R4: Crate Architecture & Auth | Layer 2 (responsibility matrix) — where operations live |
| R5: Indexing, Sync & Resource Management | Layer 3 (reconciliation model) — sync and drift handling |

## Output Format

A single research note saved via `temper research save`, containing all three layers. The note lives in the knowledge base alongside other research artifacts, indexed and searchable. Sections should be self-contained enough that downstream workstreams can reference specific layers without reading the whole document.

## Acceptance Criteria

- [ ] All five narrative walkthroughs written with step-by-step system interactions
- [ ] Each walkthrough answers its key questions explicitly
- [ ] Responsibility matrix covers all current temper commands plus new cloud operations
- [ ] Matrix principles (authority, reconciliation triggers) are stated and consistent
- [ ] All six drift types documented with detection and resolution strategies
- [ ] Reconciliation model leverages existing systems (event log, registry, content hashing)
- [ ] Scope corrections from this design are reflected (no human-in-the-loop, temper as write path, 1:1 deployment, client-side URI resolution)
- [ ] Research note saved to knowledge base via `temper research save`
