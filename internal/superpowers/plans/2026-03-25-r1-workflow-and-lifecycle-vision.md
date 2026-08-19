# R1: Workflow & Lifecycle Vision — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce the R1 research note — five workflow narratives, a responsibility matrix, and a reconciliation model — saved to the knowledge base.

**Architecture:** This is a research deliverable, not code. The output is a single markdown research note structured in three layers. Each task writes one section of the note, building a working draft that accumulates content. The final task saves the completed note via `temper research save`.

**Tech Stack:** temper CLI (for saving the research note), knowledge of the current temper codebase operations (documented in the design spec and codebase exploration).

---

## Reference Files

Read these before starting any task:

- **Design spec:** `docs/superpowers/specs/2026-03-25-r1-workflow-and-lifecycle-vision-design.md` — defines the three-layer structure, scope corrections, and acceptance criteria
- **Epic design spec:** `docs/superpowers/specs/2026-03-25-temper-cloud-design.md` — the parent design: resource model, dual authority, composable behaviors, deployment model
- **Current operations map:** The temper CLI has these commands, each of which must appear in the responsibility matrix:
  - `ticket create`, `ticket move`, `ticket done`, `ticket list`, `ticket show`, `ticket start`
  - `session save`, `session list`
  - `milestone create`, `milestone list`, `milestone update`
  - `search`, `context`
  - `index`
  - `normalize`
  - `note create`
  - `research save`
  - `warmup`, `events`, `status`
  - `project add`, `project remove`, `project list`
  - `skill generate`, `skill install`, `skill check`
  - `tui` (dispatches search, context, ticket move, index, normalize internally)

## Key Principles (from design spec)

These must be reflected consistently across all sections:

1. **Temper is the intended write path** for both git and Postgres. Out-of-band changes are edge cases.
2. **No human-in-the-loop reconciliation.** Last-write-wins for metadata, append-oriented for content, annotate surprises in event log.
3. **Client-side URI resolution.** Server stores metadata and vectors; clients resolve file://, https://, s3:// URIs.
4. **1:1 deployment.** One knowledge base = one git repo = one Postgres = one temper-cloud instance.
5. **Three resource types:** IndexableResource (pointer + embeddings), IngestedResource (fetched → markdown → stored), KnowledgeBaseResource (authored natively).
6. **Git authoritative for content**, Postgres for structured metadata and search vectors.
7. **Local HNSW is a cache**, pg_vector is canonical when online.

## Output File

The research note will be written incrementally as a working draft at:
`/tmp/r1-workflow-vision-draft.md`

The final task saves it to the knowledge base via `temper research save`.

---

### Task 1: Scaffold the research note and write Workflow 1 (Create a ticket)

**Files:**
- Create: `/tmp/r1-workflow-vision-draft.md`

- [ ] **Step 1: Create the research note scaffold with frontmatter and section headers**

Write the following to `/tmp/r1-workflow-vision-draft.md`:

```markdown
# R1: Workflow & Lifecycle Vision

## Topic

Map the day-to-day workflows and responsibility boundaries for Temper Cloud. This research establishes the foundational understanding that all downstream workstreams (R2–R5) depend on. It answers: what happens at each system boundary when a developer or agent interacts with temper, and how do git and Postgres stay in agreement?

### Scope Corrections to Epic Design Spec

During R1 design, several corrections were identified:

1. **Temper is the intended write path** for both git and Postgres. The epic spec's framing of "direct git edits and direct Postgres mutations are both valid" is too permissive. Out-of-band changes must be handled but are edge cases, not primary workflows.
2. **No human-in-the-loop reconciliation.** All drift is resolved automatically — last-write-wins for metadata, append-oriented mutations for content, event log annotations for surprising reconciliations.
3. **Workflow 5 replaced.** "Two developers on the same project" replaced with "add a new document type." Multi-developer ticket coordination is explicitly out of scope — temper is not a distributed ticketing system.
4. **Client-side URI resolution.** The server stores metadata and vectors; clients resolve URIs (file://, https://, s3://).
5. **1:1 deployment model.** One knowledge base = one git repo = one Postgres instance = one temper-cloud deployment.
6. **IngestedResource** — new third resource type. External content (PDF, HTML, DOCX) fetched, converted to markdown via kreuzberg v4, stored in the knowledge base. Original file discarded. Sits between IndexableResource (pointer only) and KnowledgeBaseResource (authored natively).

### Resource Type Summary

| Type | Content location | Stored in git? | Stored in Postgres? | Example |
|------|-----------------|----------------|---------------------|---------|
| IndexableResource | External (URI pointer) | No | Metadata + vectors | A blog post at https://example.com/post |
| IngestedResource | Knowledge base (converted markdown) | Yes | Metadata + vectors | A PDF converted to markdown via kreuzberg |
| KnowledgeBaseResource | Knowledge base (authored markdown) | Yes | Metadata + vectors | A ticket, session note, research doc |

---

## Findings

### Layer 1: Workflow Narratives

Five step-by-step stories showing what happens at each system boundary. Each step identifies the actor (client CLI/TUI, git local, git remote, Postgres/API) and the data that moves.

#### Workflow 1: Create a Ticket

**Scenario:** Developer runs `temper ticket create --title "Fix search ranking" --project temper` from their local machine, connected to the internet.

| Step | Actor | Action | Data |
|------|-------|--------|------|
| 1 | CLI | Resolve vault via `temper.toml`, resolve project from CWD or `--project` flag | Config loaded |
| 2 | CLI | Check if milestone exists; create `<project>-maintenance` if needed | Reads `milestones/<project>/` dir |
| 3 | CLI | Generate UUIDv7 id, compute seq (max existing + 10), render template | New ticket metadata |
| 4 | CLI → Git (local) | Write `tickets/temper/2026-03-26-fix-search-ranking.md` to vault working tree | Markdown file with YAML frontmatter (id, type, title, slug, project, milestone, stage: backlog, seq, created, updated) |
| 5 | CLI → Postgres | POST to temper-cloud API: create KnowledgeBaseResource record with frontmatter fields, content hash (SHA-256), and request embedding | Metadata record + vector in pg_vector |
| 6 | CLI → Local | Append `ticket_create` event to `.temper/events.jsonl` | Event JSON line |
| 7 | CLI → Local | Update `.temper/registry.json` with new file's content hash and chunk IDs | Registry entry |
| 8 | Developer → Git | `git add` and `git commit` at developer's discretion (or automated via pre-commit hooks) | Commit with new .md file |
| 9 | Developer → Git (remote) | `git push` sends the commit to the shared repo | Remote now has the file |

**Offline variant:**
- Steps 1–4, 6–7 succeed as today (purely local)
- Step 5 fails silently — the CLI queues the Postgres sync operation in `.temper/sync_queue.jsonl`
- On next connectivity, temper retries queued operations (idempotent — if the record already exists, it updates)
- The ticket is fully usable locally while offline; Postgres catches up when the network returns

**Key answers:**
- **Minimum viable offline operation:** The full ticket create works locally — file write, event log, registry. Only the Postgres sync is deferred.
- **Postgres record timing:** Created at step 5, which is during the CLI command — before the git commit. If the developer never commits, the Postgres record exists but has no corresponding git history. This is acceptable — the next sync will detect the file exists locally and reconcile.
- **Developer never pushes:** The Postgres record is authoritative for metadata. The git remote doesn't have the file. If another machine pulls, it won't see the ticket in git but can find it via cloud search. The `temper sync` or `temper pull` operation would detect this discrepancy and flag it (Postgres knows about a file that git remote doesn't have).
```

- [ ] **Step 2: Review the walkthrough against the current codebase**

Verify the step sequence matches the actual `temper ticket create` implementation. Read:
- `src/commands/ticket.rs` — the create handler
- `src/actions/ticket.rs` — the ticket creation logic
- `src/discovery.rs` — event emission

Confirm: file write order, event types, registry update behavior. The walkthrough should accurately reflect today's local behavior and extend it with the new cloud steps (5 and the offline variant).

- [ ] **Step 3: Commit progress**

```bash
git add docs/superpowers/plans/2026-03-25-r1-workflow-and-lifecycle-vision.md
git commit -m "docs: add R1 implementation plan — Task 1 in progress"
```

Note: The plan itself is committed here. The research note draft at `/tmp/` is a working file — not committed until the final `temper research save`.

---

### Task 2: Write Workflow 2 (Search across projects) and Workflow 3 (New machine)

**Files:**
- Modify: `/tmp/r1-workflow-vision-draft.md`

- [ ] **Step 1: Write Workflow 2 — Search across projects**

Append after Workflow 1 in the draft:

```markdown
#### Workflow 2: Search Across Projects

**Scenario:** Developer runs `temper search "embedding pipeline architecture"` from their local machine, connected to the internet.

| Step | Actor | Action | Data |
|------|-------|--------|------|
| 1 | CLI | Resolve vault, load config | Config loaded |
| 2 | CLI | Check connectivity to temper-cloud API | Online/offline determination |
| 3a (online) | CLI → API | POST search query to temper-cloud API | Query string, optional filters (--type, --project) |
| 4a | API → Postgres | Embed query via server-side model, pg_vector similarity search | Ranked candidate set |
| 5a | API → CLI | Return ranked results: score, resource type, URI, title, chunk content, metadata | Mixed results: KnowledgeBaseResources (file:// URIs), IndexableResources (https:// URIs), IngestedResources (file:// URIs to converted markdown) |
| 6a | CLI | Display results, resolving file:// URIs to show local path relative to vault root | Formatted output with scores and snippets |
| 3b (offline) | CLI → Local | Load `.temper/index.bin`, rebuild HNSW graph in memory | Local index loaded |
| 4b | CLI → Local | Embed query via local model (all-MiniLM-L6-v2), HNSW nearest-neighbor search | Ranked results from local index only |
| 5b | CLI | Display results — same format, but only includes locally-indexed content | No IndexableResources (Postgres-only), no content from other machines not yet pulled |

**Online vs. offline decision:**
- CLI attempts an API health check (lightweight endpoint) with a short timeout (e.g., 2 seconds)
- If reachable: cloud search (steps 3a–6a)
- If unreachable: local fallback (steps 3b–5b)
- User can force local with a flag (e.g., `--local`) for predictability or speed

**Mixed result types:**
- KnowledgeBaseResource results have `file://` URIs pointing to vault-relative paths — the file is on disk
- IngestedResource results also have `file://` URIs — the converted markdown is in the vault
- IndexableResource results have `https://`, `s3://`, or other scheme URIs — the client must fetch externally
- The result format is uniform: `{score, resource_type, uri, title, snippet, metadata}` — the client decides how to present or resolve each URI scheme

**Key answers:**
- **Cloud vs. local decision:** Automatic with short-timeout health check; `--local` flag for override.
- **Mixed result format:** Uniform structure regardless of resource type. URI scheme signals where content lives.
- **Degraded offline experience:** Only locally-indexed content appears. IndexableResources are invisible. Results may be stale if local index hasn't been rebuilt recently. Functionally identical to today's temper search.
```

- [ ] **Step 2: Write Workflow 3 — Pick up a session on a new machine**

Append after Workflow 2:

```markdown
#### Workflow 3: Pick Up a Session on a New Machine

**Scenario:** Developer sets up a new laptop. They have the knowledge base git repo URL and a GitHub account with access.

| Step | Actor | Action | Data |
|------|-------|--------|------|
| 1 | Developer → Git | `git clone <knowledge-base-repo-url>` | Full repo checkout: sessions/, tickets/, milestones/, research/, templates/, temper.toml |
| 2 | Developer | `cd knowledge-base && temper status` | Temper detects vault via `temper.toml`, reports file counts. No index yet — `.temper/` may not exist or has no `index.bin` |
| 3 | Developer | `temper auth login` (new command) | Opens browser for GitHub OAuth flow. On callback, stores auth token in `~/.config/temper/credentials.json` (not in vault — credentials are machine-local) |
| 4 | CLI → API | Validates token against temper-cloud API | Confirms identity, receives API base URL |
| 5 | Developer | `temper search "recent work"` | Online — routes to cloud API. Results appear immediately. No local index needed for cloud search. |
| 6 | Developer | `temper tui` | TUI launches. Search tab works via cloud API. Projects tab reads local git files for ticket/milestone listing. Viewer reads local files. |
| 7 | Developer (optional) | `temper index` | Builds local HNSW cache for offline fallback. Downloads embedding model from HuggingFace hub on first run (~80MB). Embeds all vault documents. |
| 8 | Developer | Normal workflow: create tickets, save sessions, search | All operations work: local file writes + cloud API sync. Offline fallback available after step 7. |

**What comes from where:**
- **From git clone (immediate):** All document content — sessions, tickets, milestones, research, templates, config. Browsable in TUI, Obsidian, any editor.
- **From cloud API (after auth):** Search results, metadata queries, lifecycle state that may have been updated from another machine.
- **Built locally (optional):** `.temper/index.bin` (HNSW cache), `.temper/registry.json` (file hashes). Only needed for offline search.

**TUI before local index:**
- Search tab: works via cloud API (online). Shows "offline search unavailable — run `temper index`" message if offline and no local index exists.
- Projects/tickets/milestones: reads from local git files — works immediately.
- Context tab: requires embeddings, so same behavior as search (cloud online, unavailable offline without local index).
- Maintain tab: `temper index` and `temper normalize` available.

**Key answers:**
- **Minimum setup:** `git clone` + `temper auth login`. Two commands to be productive.
- **Local state to build:** Only the HNSW cache (optional, for offline). Everything else comes from git or the cloud API.
- **TUI without local index:** Fully functional when online. Search/context degrade gracefully offline.
```

- [ ] **Step 3: Review walkthroughs 2 and 3 for consistency**

Check that:
- The search result format described in Workflow 2 is consistent with how Workflow 3 describes TUI search behavior
- The offline fallback behavior is described consistently
- The three resource types (IndexableResource, IngestedResource, KnowledgeBaseResource) are used consistently
- The auth model from Workflow 3 (`temper auth login`, credentials in `~/.config/temper/`) is referenced but not over-specified (R4 owns auth design)

---

### Task 3: Write Workflow 4 (Agent indexes research) and Workflow 5 (New document type)

**Files:**
- Modify: `/tmp/r1-workflow-vision-draft.md`

- [ ] **Step 1: Write Workflow 4 — Agent indexes external research**

Append after Workflow 3:

```markdown
#### Workflow 4: Agent Indexes External Research

**Scenario:** A Claude Code agent, working via MCP, encounters a useful research paper at `https://arxiv.org/abs/2401.12345` and wants to make it findable in the knowledge base.

| Step | Actor | Action | Data |
|------|-------|--------|------|
| 1 | Agent → MCP | Authenticate with temper-cloud using propagated token (inherited from the developer's session) | Auth token validated |
| 2 | Agent | Fetch the document content from the URL (client-side — the agent's machine resolves the URI) | Raw document content (HTML, PDF, etc.) |
| 3 | Agent → MCP | Call `temper.register_resource` with: URI (`https://arxiv.org/abs/2401.12345`), title, tags, content for embedding | Registration request |
| 4 | API → Postgres | Create IndexableResource record: FQDN, URI, mimetype, title, tags, provenance (who registered, when, from where) | Metadata row in resources table |
| 5 | API → Postgres | Chunk the provided content, embed via server-side model, store vectors in pg_vector | Vector embeddings linked to resource |
| 6 | API → Agent | Confirm registration, return resource ID | Success response |
| 7 | Any user | `temper search "embedding architectures"` | The arxiv paper appears in results with its https:// URI. User clicks/opens the link in their browser. |

**What does NOT happen:**
- No git file is created. IndexableResources live only in Postgres.
- The server does not fetch the URL. The agent provides content inline.
- The original document is not stored. Only metadata and vectors persist.

**IngestedResource variant:**
If the agent (or developer) wants the content permanently captured as markdown in the knowledge base:

| Step | Actor | Action | Data |
|------|-------|--------|------|
| 1–2 | Same as above | Authenticate and fetch content | Raw document |
| 3 | Agent → MCP | Call `temper.ingest_resource` with: URI, title, tags, raw content | Ingest request |
| 4 | API | Convert content to markdown via kreuzberg v4 pipeline | Extracted markdown |
| 5 | API → Git | Write converted markdown to `ingested/<project>/YYYY-MM-DD-title-slug.md` with frontmatter including provenance (original URI, conversion date, kreuzberg version) | New file in vault |
| 6 | API → Postgres | Create IngestedResource record with metadata, content hash, and vectors | Metadata + vectors |
| 7 | API → Git | Commit and push the new file | File in git history |
| 8 | Any user | The ingested document appears in search, is browsable in TUI/Obsidian, and has full provenance chain back to the original URI | Permanent knowledge base content |

**Agent autonomy boundaries:**
- **Autonomous (no confirmation):** Register IndexableResource (pointer only — low-impact, reversible). Search and read operations.
- **Requires confirmation:** Ingest resource (creates permanent knowledge base content — higher impact). Delete or modify existing resources. Move tickets between stages.
- These boundaries are configurable — the MCP adapter exposes a permission model that the developer sets up during auth configuration (R4 scope).

**Key answers:**
- **Agent autonomy vs. confirmation:** Read and register-pointer operations are autonomous. Content-creating and state-changing operations require confirmation by default, configurable.
- **IndexableResource vs. KnowledgeBaseResource in practice:** IndexableResource is a search-only pointer. It appears in results but the content lives elsewhere. KnowledgeBaseResource is a first-class vault document. IngestedResource bridges the gap — external content converted and stored permanently.
- **Required metadata:** URI (mandatory), title (mandatory), mimetype (inferred or provided), tags (optional), provenance chain (auto-generated: who, when, source).
```

- [ ] **Step 2: Write Workflow 5 — Add a new document type**

Append after Workflow 4:

```markdown
#### Workflow 5: Add a New Document Type

**Scenario:** A developer wants to add "decision records" to their knowledge base — documents that capture architectural decisions with context, alternatives considered, and rationale. Decision records should have lifecycle stages (proposed → accepted → superseded) and be taggable.

| Step | Actor | Action | Data |
|------|-------|--------|------|
| 1 | Developer → Postgres | Register the new type via API or admin CLI: type name "decision", composed behaviors [Workflowable, Taggable] | Type definition record in Postgres |
| 2 | Developer → Postgres | Define lifecycle stages for this type's Workflowable behavior: proposed → accepted → superseded → withdrawn | Stage definitions linked to type |
| 3 | Developer → Git | Create filepath convention directory: `decisions/<project>/` | Empty directory (or first file creates it) |
| 4 | Developer → Git | Create template: `templates/decision.md` with frontmatter schema | Template file |

Template content:
```
---
id: "{{id}}"
type: decision
title: "{{title}}"
slug: "{{slug}}"
project: "{{project}}"
stage: proposed
tags: []
created: {{created}}
updated: {{updated}}
---

# {{title}}

## Context

What situation or problem prompted this decision.

## Decision

What was decided.

## Alternatives Considered

What other options were evaluated and why they were not chosen.

## Consequences

What follows from this decision — both positive and negative.
```

| Step | Actor | Action | Data |
|------|-------|--------|------|
| 5 | Developer | `temper note create decision "Use pg_vector for semantic search" --project temper` | CLI reads template, renders with variables, writes to `decisions/temper/2026-03-26-use-pg-vector-for-semantic-search.md` |
| 6 | CLI → Postgres | Create KnowledgeBaseResource record with type "decision", behaviors [Workflowable, Taggable], metadata, vectors | Metadata + embeddings |
| 7 | CLI → Local | Event log append, registry update | Local state updated |
| 8 | Any user | `temper search "search architecture decisions"` | The decision record appears in results, filterable by `--type decision` |
| 9 | Any user | `temper tui` → Projects tab | Decision records appear alongside tickets and milestones. Stage column shows "proposed/accepted/superseded" for decisions. |

**What code changes are needed:** None. The key design principle is that document types are data, not code:
- The CLI's `note create` command already reads templates dynamically from `templates/`
- The chunker extracts `type` from frontmatter and includes it in chunk metadata — search filtering works automatically
- The TUI reads `type` from frontmatter for display — new types appear without code changes
- `temper normalize` handles any new type that has `id`, `type`, and `project` in frontmatter
- The Workflowable behavior gives the type lifecycle stages — `temper ticket move` (or a generalized `temper move`) works on any Workflowable resource

**What if the type needs custom behavior?**
- If a behavior doesn't exist yet (e.g., "Reviewable" — requires approval before stage transition), that's a code change to define the new behavior trait
- But composing existing behaviors onto new types is purely data
- This is the extensibility boundary: new types = data, new behaviors = code

**Key answers:**
- **Data operations needed:** Postgres type registration (name + behavior composition), template file, directory convention. Three artifacts, no code.
- **Code changes needed:** None for types that compose existing behaviors. Code only for new behaviors.
- **Existing commands handle unknown types gracefully:** search, context, list, and TUI all read `type` from frontmatter dynamically. The `note create` command reads templates by name. The normalize command handles any frontmatter with standard fields.
```

- [ ] **Step 3: Review walkthroughs 4 and 5 for consistency with 1–3**

Check that:
- The three resource types are used consistently across all five walkthroughs
- The IngestedResource variant in Workflow 4 is consistent with the resource type summary in the scaffold
- The agent autonomy boundaries in Workflow 4 don't contradict the "no human-in-the-loop" principle from scope corrections (they don't — confirmation is for agents, not for reconciliation)
- The `note create` behavior in Workflow 5 matches the current CLI (it does — `note create <type> <title>` with template lookup)
- Workflow 5's claim that "no code changes needed" is accurate given the current codebase structure

---

### Task 4: Write the Responsibility Matrix (Layer 2)

**Files:**
- Modify: `/tmp/r1-workflow-vision-draft.md`

- [ ] **Step 1: Write the responsibility matrix**

Append after Workflow 5, still under the `## Findings` section:

```markdown
### Layer 2: Responsibility Matrix

Synthesized from the five walkthroughs. For every temper operation: what happens locally, what happens in git remote, what happens in Postgres, who is authoritative, and what triggers reconciliation.

#### Principles

1. **Temper is the intended write path** for both git and Postgres. Out-of-band changes are edge cases handled by reconciliation, not primary workflows.
2. **Git is authoritative for document content** (prose, markdown body). Postgres is authoritative for structured metadata (lifecycle state, search vectors, type definitions, behavior composition).
3. **Last-write-wins** for metadata conflicts, keyed on the `updated` timestamp. No human-in-the-loop.
4. **Append-oriented document mutations** for content. Temper commands that modify documents append or update specific frontmatter fields rather than rewriting entire files, minimizing merge conflicts.
5. **Local HNSW is a cache.** pg_vector is the canonical search index when online. Local index is rebuilt on demand for offline fallback.
6. **The event log is the reconciliation audit trail.** Every automatic resolution is annotated in `.temper/events.jsonl`.

#### Existing Operations

| Operation | Git (local) | Git (remote) | Postgres | Authority | Reconciliation |
|-----------|-------------|--------------|----------|-----------|----------------|
| `ticket create` | Write .md file | On push | Create resource record + vectors | Git: content. Postgres: metadata | Sync on API call; queued if offline |
| `ticket move` | Update frontmatter field (stage, milestone, scope) | On push | Update metadata fields | Postgres: lifecycle state | Sync on API call; queued if offline |
| `ticket done` | Update frontmatter (stage: done, branch, pr) | On push | Update metadata | Postgres: lifecycle state | Sync on API call |
| `ticket list` | Read .md files in tickets/ | — | Query resource records | Postgres when online, git when offline | — (read-only) |
| `ticket show` | Read .md file | — | — (content from git) | Git: content | — (read-only) |
| `ticket start` | Update frontmatter (stage: in-progress) | On push | Update metadata | Postgres: lifecycle state | Sync on API call |
| `session save` | Write/update .md file | On push | Create/update resource record + vectors | Git: content. Postgres: metadata | Sync on API call |
| `session list` | Read session .md files | — | Query resource records | Postgres when online, git when offline | — (read-only) |
| `milestone create` | Write .md file | On push | Create resource record | Git: content. Postgres: metadata | Sync on API call |
| `milestone list` | Read milestone .md files | — | Query resource records | Postgres when online, git when offline | — (read-only) |
| `milestone update` | Update frontmatter (status) | On push | Update metadata | Postgres: lifecycle state | Sync on API call |
| `search` | Fallback: local HNSW query | — | Primary: pg_vector query | Postgres: vectors | Local index rebuilt on demand |
| `context` | Fallback: local HNSW multi-hop | — | Primary: pg_vector multi-hop | Postgres: vectors | Local index rebuilt on demand |
| `index` | Build .temper/index.bin + registry.json | — | — (local cache only) | Local: cache. Postgres: canonical | — (local operation) |
| `normalize` | Repair frontmatter, backfill ids, fix structure | On push | Sync repaired metadata | Git: content. Postgres: metadata | Sync after normalize completes |
| `note create` | Write .md file from template | On push | Create resource record + vectors | Git: content. Postgres: metadata | Sync on API call |
| `research save` | Write/update .md file | On push | Create/update resource record + vectors | Git: content. Postgres: metadata | Sync on API call |
| `warmup` | Read sessions, events, tickets | — | Could also query Postgres for richer context | Hybrid | — (read-only) |
| `events` | Read .temper/events.jsonl | — | — (local log) | Local | — (read-only) |
| `status` | Read vault structure, index stats | — | Could also report cloud sync status | Hybrid | — (read-only) |
| `project add/remove` | Update temper.toml | On push | Sync project registry | Git: config | Sync on API call |
| `project list` | Read temper.toml | — | — | Git: config | — (read-only) |
| `skill generate/install/check` | Read temper.toml, write skill file | — | — | Local only | — (no cloud involvement) |
| `tui` | All of the above depending on user action | — | All of the above | Per-operation | Per-operation |

#### New Cloud Operations

| Operation | Git (local) | Git (remote) | Postgres | Authority | Reconciliation |
|-----------|-------------|--------------|----------|-----------|----------------|
| `auth login` | — | — | Validate OAuth token | Postgres: auth | — |
| `register IndexableResource` | — (no file) | — | Create resource record + vectors | Postgres: full authority | — (no git involvement) |
| `ingest resource` | Write converted .md file | On push | Create resource record + vectors | Git: converted content. Postgres: metadata + provenance | Sync on API call |
| `sync push` | Read local changes since last sync | — | Update Postgres from local state | Per-field authority | Content-hash comparison |
| `sync pull` | Update local from Postgres state | — | Read Postgres state | Per-field authority | Content-hash comparison |
| `webhook (git push)` | — | Receives push event | Re-index changed files, update metadata | Git: content trigger | Automatic on push |
```

- [ ] **Step 2: Validate matrix completeness**

Cross-reference the matrix against the full command list in the Reference Files section of this plan. Confirm every command appears. Check that the new cloud operations cover the gaps identified in the walkthroughs (auth, resource registration, ingestion, sync, webhooks).

- [ ] **Step 3: Check matrix consistency with walkthroughs**

Verify:
- Workflow 1 (ticket create): matches the `ticket create` row
- Workflow 2 (search): matches the `search` row, including online/offline behavior
- Workflow 3 (new machine): the auth and sync operations are in the New Cloud Operations table
- Workflow 4 (agent): `register IndexableResource` and `ingest resource` rows match the walkthrough
- Workflow 5 (new type): `note create` row handles dynamic types

---

### Task 5: Write the Reconciliation Model (Layer 3)

**Files:**
- Modify: `/tmp/r1-workflow-vision-draft.md`

- [ ] **Step 1: Write the reconciliation model**

Append after the responsibility matrix:

```markdown
### Layer 3: Reconciliation Model

How temper automatically resolves drift between git and Postgres. Organized by drift type. Each entry: what it looks like, how temper detects it, how it resolves automatically, and what gets annotated.

All reconciliation is automatic. There is no human-in-the-loop step. The event log serves as the audit trail — if something surprising was resolved, the annotation makes it visible after the fact.

#### Drift Type 1: Metadata Drift

**What it looks like:** Frontmatter on disk says `stage: backlog`, Postgres says `stage: in-progress`. Or `updated` timestamps disagree.

**Cause:** Out-of-band git edit (developer changed frontmatter in vim/Obsidian without running temper), or delayed sync (temper wrote Postgres but git commit hasn't happened yet), or two machines synced at different times.

**Detection:** On sync (push or pull), temper compares the SHA-256 content hash of each file against the registry. If the hash changed, it parses frontmatter and compares field-by-field against Postgres. Leverages the existing `.temper/registry.json` infrastructure.

**Resolution:**
1. Compare `updated` timestamps from frontmatter and Postgres
2. Most recent `updated` wins — that side's values overwrite the other
3. If timestamps are identical (unlikely but possible), Postgres wins (it's the metadata authority)
4. Overwrite the loser: if git is stale, update frontmatter in-place via `set_frontmatter_field`; if Postgres is stale, update via API
5. Update content hash in registry
6. Append `metadata_reconciled` event to events.jsonl: `{type: "metadata_reconciled", ts, file, field, from_value, to_value, winner: "git"|"postgres", reason: "newer_timestamp"}`

#### Drift Type 2: Content Drift

**What it looks like:** The markdown body of a document changed on disk but Postgres vectors are stale (based on the old content).

**Cause:** Developer edited the file directly (vim, Obsidian, IDE) without running temper. Or a `git pull` brought in changes from another machine.

**Detection:** Content-hash mismatch — the SHA-256 in the registry doesn't match the file on disk. Detected during sync or index operations.

**Resolution:**
1. Git is authoritative for content — the file on disk is correct by definition
2. Re-chunk the document using the existing chunker pipeline
3. Re-embed all chunks, update pg_vector
4. Update content hash in registry
5. Append `content_reindexed` event: `{type: "content_reindexed", ts, file, old_hash, new_hash, chunks_updated}`
6. No frontmatter changes needed — this is a content-only drift

#### Drift Type 3: Index Staleness

**What it looks like:** Local HNSW returns different (older) results than cloud pg_vector search for the same query.

**Cause:** New content indexed on another machine or via MCP agent. IndexableResources registered via API (these never appear in local HNSW). Time elapsed since last `temper index`.

**Detection:** Compare local registry `last_indexed` timestamp against the cloud API's latest change timestamp (a lightweight metadata endpoint).

**Resolution:**
1. When online, this is invisible — cloud search is the primary path, local HNSW is only used offline
2. If the user runs `temper index`, the local cache rebuilds from current vault content
3. IndexableResources (Postgres-only) are never in the local HNSW — this is by design, not a bug
4. Optional enhancement: `temper index --include-remote` could pull vectors from Postgres for a richer local cache (deferred to R5)
5. No event log entry — cache staleness is expected, not an anomaly

#### Drift Type 4: Partial Sync Failure

**What it looks like:** A temper operation partially completed — e.g., the local file was written but the Postgres API call failed (network dropped), or vice versa.

**Cause:** Network interruption mid-operation. API timeout. Server error during a multi-step operation.

**Detection:** Temper tracks pending sync operations in `.temper/sync_queue.jsonl`. Each entry records: operation type, file path, timestamp, payload, retry count. On CLI startup (or on a periodic check), temper scans the queue.

**Resolution:**
1. On next startup or next successful API health check, process the sync queue
2. For each queued operation, retry the API call
3. Operations are idempotent: creating a record that already exists updates it; updating with the same timestamp is a no-op
4. On success: remove from queue, append `sync_retry_succeeded` event
5. On repeated failure (e.g., 3 retries): keep in queue, append `sync_retry_failed` event, surface in `temper status` output
6. The queue file uses the same append-only JSONL format as events.jsonl for consistency

**Sync queue entry format:**
```json
{"op": "create_resource", "file": "tickets/temper/2026-03-26-fix-search-ranking.md", "ts": "2026-03-26T10:00:00Z", "retries": 0, "payload": {"type": "ticket", "title": "Fix search ranking", "project": "temper", "stage": "backlog"}}
```

#### Drift Type 5: Orphaned References

**What it looks like:** An IndexableResource's URI is no longer reachable — the web page was taken down, the file was deleted, the S3 object expired.

**Cause:** External content changes outside temper's control.

**Detection:** Periodic or on-demand validation. Since clients resolve URIs, validation runs client-side: `temper check --validate-uris` attempts HEAD requests (or file existence checks) for all IndexableResource URIs. This is an opt-in operation — not every sync.

**Resolution:**
1. Mark resource as `unreachable` in Postgres metadata (new field: `reachability: reachable|unreachable|unknown`)
2. Retain the resource record and search vectors — the metadata, title, and cached embeddings still have value for discovery
3. Surface unreachable resources in `temper status` output and in TUI maintenance view
4. If content was previously provided inline (at registration time), the vectors remain useful even though the URI is dead
5. Append `resource_unreachable` event: `{type: "resource_unreachable", ts, uri, resource_id, last_reachable}`
6. Developer can manually delete or re-point the resource if desired

#### Drift Type 6: Out-of-Band Postgres Mutation

**What it looks like:** A Postgres record was modified directly (raw SQL, admin tool, migration script) without a corresponding temper operation. Frontmatter and Postgres disagree, and there's no event log entry explaining why.

**Cause:** Database maintenance, manual fix, migration, or accidental direct modification.

**Detection:** On sync, temper compares frontmatter-to-Postgres for each file. If they disagree and the event log has no recent operation for that file, the mutation was out-of-band.

**Resolution:**
1. Same as metadata drift — compare `updated` timestamps, last-write-wins
2. If the Postgres record has a newer `updated` (which it likely does if someone intentionally modified it), Postgres values propagate to frontmatter
3. If the Postgres record has no `updated` or an older one, git frontmatter wins
4. Append `out_of_band_reconciled` event with source: `{type: "out_of_band_reconciled", ts, file, source: "postgres"|"git", fields_changed: [...]}`
5. This event type is distinct from `metadata_reconciled` to make audit trail queries easier — "show me all times something was changed outside temper"
```

- [ ] **Step 2: Validate reconciliation model completeness**

Check the six drift types against:
- The design spec's edge case catalog (metadata drift, stale cache, offline work, partial sync failure)
- The walkthroughs (does each walkthrough's offline/failure variant map to a drift type?)
- The responsibility matrix (do the "Reconciliation" column entries all have corresponding drift type coverage?)

- [ ] **Step 3: Check consistency with Layers 1 and 2**

Verify:
- The sync queue format in Drift Type 4 is consistent with the offline variant in Workflow 1
- The event types (`metadata_reconciled`, `content_reindexed`, `sync_retry_succeeded`, etc.) extend the existing event type system (which has `ticket_create`, `ticket_move`, `note_create`, etc.)
- The `reachability` field in Drift Type 5 is compatible with the resource model described in the walkthroughs
- The content-hash detection mechanism is consistently described as SHA-256 leveraging the existing registry

---

### Task 6: Write Implications, Open Questions, and Sources sections

**Files:**
- Modify: `/tmp/r1-workflow-vision-draft.md`

- [ ] **Step 1: Write the Implications section**

Append after Layer 3:

```markdown
## Implications

### For R2: Data Model & Schema Design

The responsibility matrix defines what the Postgres schema must support:
- Resource records for three types (IndexableResource, IngestedResource, KnowledgeBaseResource) with a shared base and type-specific metadata
- Composable behavior composition: type definitions link to behaviors, behaviors define available fields and lifecycle rules
- Content hashes (SHA-256) stored per-resource for drift detection
- `updated` timestamps with sufficient precision for last-write-wins (RFC 3339 with subsecond granularity)
- UUIDv7 for time-ordered identity, consistent with current temper implementation
- Sync queue state may live in Postgres (for cross-machine visibility) or remain local (for simplicity) — R2 should decide
- Event types for reconciliation (`metadata_reconciled`, `content_reindexed`, etc.) need schema support if events move to Postgres

### For R3: Deployment Platform Evaluation

The walkthroughs constrain what the deployment platform must support:
- HTTP API for search queries, metadata CRUD, and resource registration
- pg_vector for semantic search (rules out platforms that can't host Postgres extensions)
- Webhook endpoint for git push notifications (or polling alternative)
- Server-side embedding for IndexableResource content provided via API
- GitHub OAuth callback endpoint
- The server does NOT need: git clone capability, file storage, long-running background processes (unless webhooks require async processing)

### For R4: Crate Architecture & Auth

The responsibility matrix maps operations to system boundaries:
- Operations that touch both git and Postgres need a sync layer — this is likely a trait in temper-core
- Auth tokens stored in `~/.config/temper/credentials.json` (machine-local, not in vault)
- The MCP adapter needs a permission model for agent autonomy boundaries
- `temper auth login` is a new CLI command — lives in temper-cli, delegates to temper-core auth module
- The sync queue (`.temper/sync_queue.jsonl`) is a new local state file — lives alongside events.jsonl and registry.json

### For R5: Indexing, Sync & Resource Management

The reconciliation model defines the sync architecture:
- Content-hash based change detection (existing registry infrastructure)
- Sync queue with idempotent retry for partial failures
- Server-side embedding for API-submitted content
- Local HNSW as optional offline cache, not authoritative
- URI validation as opt-in maintenance operation
- kreuzberg v4 as the ingestion pipeline for IngestedResources — evaluate also as potential replacement for current chunking/embedding preprocessing
```

- [ ] **Step 2: Write the Open Questions section**

```markdown
## Open Questions

Questions surfaced during R1 that belong to downstream workstreams:

1. **R2:** Should the sync queue live in Postgres (visible across machines) or remain local in `.temper/sync_queue.jsonl`? Local is simpler but means a machine that goes permanently offline leaves orphaned queue entries.
2. **R2:** How should composable behaviors be modeled in Postgres — columns on a wide table, JSONB fields, or a separate behaviors join table? The workflow walkthroughs don't constrain this, but the "add a new type" workflow (WF5) requires that new behaviors can be linked without schema migration.
3. **R3:** Server-side embedding is assumed for API-submitted content (IndexableResource registration). Does the deployment platform support running an embedding model, or should the API require pre-embedded vectors?
4. **R4:** The `temper auth login` flow assumes browser-based OAuth redirect. How does this work in headless/SSH environments? Device code flow as fallback?
5. **R4:** Should the MCP permission model be per-operation or per-resource-type? Workflow 4 suggests per-operation (read vs. register vs. ingest), but per-resource-type might be more intuitive.
6. **R5:** The IngestedResource pipeline uses kreuzberg v4 for document-to-markdown conversion. Should this also replace the current local chunking and text preprocessing pipeline (wikilink stripping, markdown link simplification, etc.)?
7. **R5:** What triggers the git-push webhook processing? Is it a GitHub webhook to the temper-cloud API, or does the CLI push and then explicitly notify the API?
```

- [ ] **Step 3: Write the Sources section**

```markdown
## Sources

- Temper Cloud Epic Design Spec: `docs/superpowers/specs/2026-03-25-temper-cloud-design.md`
- R1 Research Design Spec: `docs/superpowers/specs/2026-03-25-r1-workflow-and-lifecycle-vision-design.md`
- Current temper codebase: CLI commands (`src/cli.rs`), vault operations (`src/vault.rs`), indexing pipeline (`src/hnsw.rs`, `src/embedder.rs`, `src/chunker.rs`), event system (`src/discovery.rs`), registry (`src/registry.rs`)
- kreuzberg v4: https://github.com/kreuzberg-dev/kreuzberg — document-to-markdown conversion pipeline, candidate for ingestion and potentially broader text processing
```

---

### Task 7: Final review and save to knowledge base

**Files:**
- Read: `/tmp/r1-workflow-vision-draft.md` (complete draft)
- Create: `research/temper/` directory in knowledge base (via temper research save)

- [ ] **Step 1: Full document review**

Read the complete draft end-to-end. Check:
- All five walkthroughs are present and answer their key questions
- The responsibility matrix covers all commands from the reference list
- All six drift types are documented
- The three resource types (IndexableResource, IngestedResource, KnowledgeBaseResource) are used consistently
- No placeholders, TBDs, or vague references
- The scope corrections are stated clearly and reflected throughout
- Implications section correctly maps findings to downstream workstreams

- [ ] **Step 2: Check against acceptance criteria**

From the design spec:
- [ ] All five narrative walkthroughs written with step-by-step system interactions
- [ ] Each walkthrough answers its key questions explicitly
- [ ] Responsibility matrix covers all current temper commands plus new cloud operations
- [ ] Matrix principles (authority, reconciliation triggers) are stated and consistent
- [ ] All six drift types documented with detection and resolution strategies
- [ ] Reconciliation model leverages existing systems (event log, registry, content hashing)
- [ ] Scope corrections from this design are reflected
- [ ] Ready for `temper research save`

- [ ] **Step 3: Save to knowledge base**

```bash
cat /tmp/r1-workflow-vision-draft.md | temper research save "R1 Workflow and Lifecycle Vision" --project temper
```

This creates `research/temper/2026-03-26 — R1 Workflow and Lifecycle Vision.md` in the knowledge base with proper frontmatter.

- [ ] **Step 4: Verify the saved note**

```bash
temper search "workflow lifecycle responsibility matrix" --project temper
```

Confirm the research note appears in search results.

- [ ] **Step 5: Update the ticket**

```bash
temper session save "R1 Workflow and Lifecycle Vision — Research Complete" --ticket 2026-03-25-r1-workflow-and-lifecycle-vision --state done --project temper
```
