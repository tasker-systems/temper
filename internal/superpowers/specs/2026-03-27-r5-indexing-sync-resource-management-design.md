# R5: Indexing, Sync & Resource Management — Design Spec

**Date:** 2026-03-27
**Ticket:** 2026-03-27-r5-indexing-sync-resource-management
**Depends on:** R1 (workflow vision), R2 (data model), R3 (deployment platform), R4 (crate architecture, auth, access control)

## Overview

R5 designs the operational layer that bridges Postgres-as-authority (R2) with files-on-disk as working artifacts. It defines the API contract between all temper clients (CLI, web UI, MCP) and the server, then derives the sync protocol, vault management, conflict resolution, and CLI surface from that contract.

The API-contract-first approach ensures that every client speaks the same language, the crate split from R4 (temper-client as shared auth-aware wrapper) has a concrete interface to implement against, and the manifest/subscription model on the client side is shaped by what the API needs rather than by local convenience.

## Design Decisions

| Area | Decision |
|------|----------|
| Sync orientation | Hybrid: reads always cloud API, writes explicit, auto-sync opt-in |
| Sync command | Single bidirectional `temper sync` — no pull/push split |
| Subscription model | config.toml defines context/team/doc-type subscriptions; remote profile.preferences as default, local as override |
| Conflict resolution | Side-by-side `.conflict.md` with TEMPER-SYSTEM annotations; `temper merge` for inline reconciliation; `temper sync resolve` to commit |
| Merge policy | Per-subscription `merge = "manual"|"auto"` in config.toml |
| Vault onboarding | Vault is the mutability boundary; `vault add` outside = copy+stamp, inside = stamp in place; non-markdown extracted via kreuzberg |
| Frontmatter | Always injected for vault-managed files |
| Upload pipeline | Batched zip with manifest → presigned R2 URL → background temper-embed chunks+embeds → Neon |
| Embedding | Server-side only via background worker; CLI never embeds |
| Event scoping | Time-bounded visibility + actor-always-sees-own |
| Resource transfer | Two-step offer/accept for personal; bulk reassign for team/deactivation |
| Search | Unified `temper search` always hits cloud API; `context` becomes alias for `--mode graph` |
| CLI/skill split | CLI = CRUD primitives + aliases; skill = workflow orchestration |

---

## 1. API Surface — Resource Lifecycle

### Resources

| Method | Endpoint | Purpose |
|--------|----------|---------|
| `GET` | `/api/resources` | List resources (scoped by `resources_visible_to`) |
| `GET` | `/api/resources/:id` | Get resource metadata + current chunk content |
| `GET` | `/api/resources/:id/content` | Reconstitute full markdown from current chunks |
| `POST` | `/api/resources` | Create resource (metadata only, content via sync/upload) |
| `PATCH` | `/api/resources/:id` | Update resource metadata (title, context, doc_type, tags) |
| `DELETE` | `/api/resources/:id` | Soft-delete (sets `is_active = false`) |

All list/get endpoints are scoped through `resources_visible_to(profile_id)` as a query precondition. Access control lives in the database, not the application layer.

### Search

| Method | Endpoint | Purpose |
|--------|----------|---------|
| `GET` | `/api/search` | Unified search — `?q=<query>&mode=semantic|keyword|graph` |

Query params: `q`, `mode` (default: semantic), `context`, `doc_type`, `team`, `depth` (for graph mode), `limit`. All results scoped through `resources_visible_to`. Results include resource metadata, relevance score, and content snippet.

The `graph` mode replaces the standalone `context` command — traverses relationships from nearest semantic matches. `temper context <topic>` becomes an alias for `temper search <topic> --mode graph`.

### Upload (R2 Pipeline)

| Method | Endpoint | Purpose |
|--------|----------|---------|
| `POST` | `/api/upload/init` | Request presigned URL; body: `{filename, size, mime}` |
| `POST` | `/api/upload/complete` | Confirm upload + queue processing; body: `{key, resource_ids[], manifest_hash}` |
| `GET` | `/api/upload/:key/status` | Poll processing status (chunking/embedding progress) |

The sync flow uses these same endpoints — sync is a specialized upload workflow where the CLI batches changed files into a zip with an embedded manifest.

### Events

| Method | Endpoint | Purpose |
|--------|----------|---------|
| `GET` | `/api/events` | Event stream scoped by time-bounded resource visibility + actor-own |

Query params: `since` (timestamp), `context`, `resource_id`, `limit`. Events are visible if: (a) you generated them, or (b) they occurred on a resource visible to you AND after that resource became visible to you. Each event includes an event ID usable in TEMPER-SYSTEM annotations.

---

## 2. API Surface — Sync & Reconciliation

### Sync Endpoints

| Method | Endpoint | Purpose |
|--------|----------|---------|
| `POST` | `/api/sync/status` | Compare client manifest against server state |
| `POST` | `/api/sync/pull` | Fetch content for changed/new resources |
| `POST` | `/api/sync/complete` | Finalize sync round, mark device as caught up |
| `POST` | `/api/sync/resolve` | Commit conflict resolution |

### Sync Protocol

```
CLI                              API                           R2 / Neon
 |                                |                               |
 |-- POST /api/sync/status -----→| (compare manifest vs server)  |
 |  {subscriptions, manifest}     |                               |
 |←- {to_pull[], to_push[],      |                               |
 |    conflicts[]}  --------------│                               |
 |                                |                               |
 |--- [for to_push: zip+upload via /api/upload/* pipeline] -----→|
 |                                |                               |
 |--- POST /api/sync/pull ------→|                               |
 |  {resource_ids: to_pull}       |←-- fetch content -------------│
 |←- {zip of markdown + meta} ---|                               |
 |                                |                               |
 |  [CLI: materialize files,      |                               |
 |   detect conflicts,            |                               |
 |   write .conflict.md]          |                               |
 |                                |                               |
 |--- POST /api/sync/complete --→| (mark device as synced)       |
 |←- {ok, event_ids[]} ----------|                               |
```

### /api/sync/status — The Decision Point

Request body:
```json
{
  "subscriptions": [
    {"context": "temper"},
    {"context": "tasker", "doc_types": ["research", "concept"]},
    {"team": "platform-team"}
  ],
  "manifest_entries": [
    {"resource_id": "uuid", "content_hash": "sha256:...", "updated_at": "..."}
  ]
}
```

Response:
```json
{
  "to_pull": [{"resource_id": "uuid", "content_hash": "sha256:...", "title": "..."}],
  "to_push": [{"resource_id": "uuid", "reason": "local_modified"}],
  "conflicts": [{"resource_id": "uuid", "local_hash": "...", "remote_hash": "..."}],
  "removed": [{"resource_id": "uuid", "reason": "deleted|unshared"}]
}
```

Four lists drive the sync round:
- **to_pull** — server has newer content or new resources matching subscriptions that the client doesn't have (or has stale versions of)
- **to_push** — client sent manifest entries with content hashes that differ from what the server has, and the server's version hasn't changed since last sync (safe to overwrite)
- **conflicts** — both sides changed since last sync (client hash != manifest hash AND server hash != manifest hash)
- **removed** — resources the client has in its manifest but that are no longer visible (deleted, unshared, or no longer matching subscriptions). Client should remove local files and manifest entries.

### /api/sync/pull

Request: `{resource_ids: []}`. Response: zip of markdown files with metadata sidecar (JSON per resource: UUID, title, context, doc_type, content_hash, tags). CLI materializes files to vault paths based on context/doc_type and updates manifest.

### /api/sync/complete

Request: `{resource_ids[], manifest_hash}`. Server records sync timestamp for this device. Response includes event IDs generated during the sync for audit trail.

### /api/sync/resolve

Request: `{resource_id, resolution: "local"|"remote"|"merged", content_hash}`. Called after conflict resolution. The winning content is pushed through the normal upload pipeline before this call.

### Device Tracking

Each device gets a `client_id` generated at `temper init` time, stored in `~/.config/temper/devices/<id>.json`, sent as `X-Temper-Client-Id` header on all API calls. The server tracks last-sync-at per device to scope sync/status responses.

---

## 3. API Surface — Teams, Profiles & Transfer

### Profiles

| Method | Endpoint | Purpose |
|--------|----------|---------|
| `GET` | `/api/profile` | Current authenticated profile (from JWT) |
| `PATCH` | `/api/profile` | Update display_name, preferences, vault_config |
| `GET` | `/api/profile/auth-links` | List linked auth providers |
| `POST` | `/api/profile/deactivate` | Pre-check: returns `DeactivationCheck` |
| `DELETE` | `/api/profile` | Execute deactivation (only if pre-check passed) |

### Teams

| Method | Endpoint | Purpose |
|--------|----------|---------|
| `GET` | `/api/teams` | List teams the profile belongs to |
| `POST` | `/api/teams` | Create team; body: `{name, description}` |
| `GET` | `/api/teams/:id` | Team detail + member list |
| `PATCH` | `/api/teams/:id` | Update team metadata (owner/maintainer only) |
| `DELETE` | `/api/teams/:id` | Soft-delete team (owner only) |

### Team Membership

| Method | Endpoint | Purpose |
|--------|----------|---------|
| `POST` | `/api/teams/:id/invite` | Create invitation; body: `{email, role}` |
| `GET` | `/api/teams/:id/invitations` | List pending invitations (owner/maintainer) |
| `POST` | `/api/invitations/:token/accept` | Accept invitation (creates TeamMember) |
| `POST` | `/api/invitations/:token/decline` | Decline invitation |
| `DELETE` | `/api/teams/:id/members/:profile_id` | Remove member (owner/maintainer, or self) |
| `PATCH` | `/api/teams/:id/members/:profile_id` | Change role (owner/maintainer only) |

All membership mutations enforce `can_manage_team(profile_id, team_id, action)` in the database.

### Team Resources

| Method | Endpoint | Purpose |
|--------|----------|---------|
| `POST` | `/api/teams/:id/resources` | Share resource to team; body: `{resource_id, access_level}` |
| `PATCH` | `/api/teams/:id/resources/:resource_id` | Change access level |
| `DELETE` | `/api/teams/:id/resources/:resource_id` | Remove (vault = full delete, others = unlink) |

### Resource Transfer

| Method | Endpoint | Purpose |
|--------|----------|---------|
| `POST` | `/api/transfers` | Offer ownership transfer; body: `{resource_id, to_profile_id}` |
| `GET` | `/api/transfers` | List pending transfers (incoming and outgoing) |
| `POST` | `/api/transfers/:id/accept` | Accept (updates `owner_profile_id`) |
| `POST` | `/api/transfers/:id/decline` | Decline |
| `POST` | `/api/teams/:id/reassign` | Bulk reassign; body: `{from_profile_id, to_profile_id}` |

Transfer is two-step offer/accept for personal transfers. Bulk reassign is owner/maintainer-driven for team scenarios (member departure, deactivation cascade).

### Auth

| Method | Endpoint | Purpose |
|--------|----------|---------|
| `POST` | `/api/auth/callback` | Exchange auth code for JWT (Neon Auth flow) |
| `POST` | `/api/auth/refresh` | Refresh JWT |
| `GET` | `/api/auth/providers` | List available auth providers |

CLI login flow: `temper auth login` → opens browser → Neon Auth OAuth → callback to local ephemeral HTTP server → JWT stored at `~/.config/temper/auth.json`.

---

## 4. Client-Side State — Manifest, Config & Subscriptions

### File Layout

```
~/.config/temper/
├── config.toml          # Vault path, subscriptions, sync preferences
├── auth.json            # JWT + refresh token
└── devices/
    └── <device_id>.json # Generated at init, sent as X-Temper-Client-Id

<vault_path>/
├── .temper/
│   ├── manifest.json    # Resource UUID → local path → content hash → sync state
│   ├── events.jsonl     # Local event log (drains to server on sync)
│   └── conflicts/       # Metadata for active conflicts
├── <context>/           # Context directories matching subscriptions
│   ├── tickets/
│   ├── sessions/
│   └── research/
└── ...
```

### config.toml

```toml
[vault]
path = "~/projects/knowledge"

[sync]
auto = false                    # opt-in: run sync pre-flight on every temper command

[[sync.subscriptions]]
context = "temper"
merge = "manual"

[[sync.subscriptions]]
context = "tasker"
doc_types = ["research", "concept"]
merge = "manual"

[[sync.subscriptions]]
team = "platform-team"
doc_types = ["research", "concept"]
merge = "auto"

[[sync.subscriptions]]
team = "platform-team"
doc_types = ["ticket", "milestone"]
merge = "manual"

[cli]
progress = "bar"                # "bar" (default) | "json" (JSONL event stream)
```

Remote default stored in `profile.preferences.sync.subscriptions`. On first sync after `temper init`, CLI merges remote defaults with local config (local wins on conflicts). User can push local config to remote via `temper profile sync-preferences`.

### manifest.json

```json
{
  "device_id": "d7e8f9...",
  "last_sync": "2026-03-27T18:30:00Z",
  "entries": {
    "<resource_uuid>": {
      "path": "temper/tickets/2026-03-27-r5-indexing.md",
      "content_hash": "sha256:ab3f...",
      "remote_hash": "sha256:ab3f...",
      "synced_at": "2026-03-27T18:30:00Z",
      "state": "clean"
    }
  }
}
```

Per-resource sync states:
- **clean** — local hash = manifest hash = remote hash
- **local_modified** — local hash != manifest hash (local edits since last sync)
- **remote_modified** — detected on next sync/status check
- **conflict** — both sides changed; `.conflict.md` materialized alongside
- **pending** — subscribed but not yet materialized (new resource from server)

### Sync Pre-flight (auto mode)

When `sync.auto = true`, every temper command runs a lightweight local-only check:

1. Hash local files in manifest where `state = clean` (skip if file mtime unchanged since last check)
2. Any changed? Mark `local_modified` in manifest
3. No network call — just keeps the manifest accurate for the next explicit `temper sync`

Full network reconciliation remains `temper sync`. Auto-sync as "reconcile on every command" is a future evolution once the protocol is proven stable.

---

## 5. Vault Onboarding & Extraction Pipeline

### `temper vault add <path|url>`

Three input types, one output: managed markdown in the vault.

**Markdown file outside vault:**
```
temper vault add ~/docs/architecture.md --context temper
→ copies to <vault>/temper/[inferred_doc_type]/architecture.md
→ injects frontmatter: temper-id (UUIDv7), title, context, doc_type, created
→ manifest entry created with state: local_modified
→ next temper sync pushes through R2 pipeline
```

**Non-markdown file (PDF, DOCX, HTML):**
```
temper vault add ~/papers/attention-is-all-you-need.pdf --context temper
→ kreuzberg extracts to markdown
→ writes to <vault>/temper/source/attention-is-all-you-need.md
→ frontmatter includes temper-id, title, doc_type: source,
  ingestion_source: file:///Users/.../attention-is-all-you-need.pdf
→ original file NOT copied (temper is a knowledge base, not a filestore)
→ kb_ingestion_records tracks provenance
```

**URL:**
```
temper vault add https://arxiv.org/abs/2401.12345 --context temper
→ fetch HTML/PDF, kreuzberg extracts to markdown
→ writes to <vault>/temper/source/attention-is-all-you-need.md
→ ingestion_source set to the URL
→ kb_ingestion_records tracks provenance
```

**Markdown file already in vault:**
```
temper vault add temper/research/my-notes.md
→ already in vault, mutate in place
→ inject frontmatter if missing (stamp with UUIDv7)
→ manifest entry created with state: local_modified
```

### Bulk onboarding

```
temper vault add ./docs/**/*.md --context temper --doc-type research
```

Glob expansion, each file processed individually. Progress bar for per-file status. Next `temper sync` batches into a single zip upload.

### Frontmatter shape

```yaml
---
temper-id: 019537a2-...
title: "Attention Is All You Need"
context: temper
doc_type: source
ingestion_source: "https://arxiv.org/abs/2401.12345"
created: 2026-03-27T19:00:00Z
---
```

Minimal identity anchor. Everything else (tags, behaviors, team associations, access levels) lives in Postgres and is managed through the API.

---

## 6. Conflict Resolution — Full Lifecycle

### Detection

`/api/sync/status` returns conflicts where both local and remote hashes diverge from last-known manifest hash. CLI materializes each conflict:

```
temper/research/sync-design.md              ← local version (untouched)
temper/research/sync-design.conflict.md     ← remote version with annotations
```

### TEMPER-SYSTEM annotation format

The `.conflict.md` contains the remote version with change annotations:

```markdown
---
temper-id: 019537a2-...
title: "Sync Design"
conflict-with: sha256:ab3f...
---

# Sync Design

## Architecture

**TEMPER-SYSTEM: modified by pete@example.com on 2026-03-27T18:45:00Z (event:019537b1-...)**

The sync protocol uses a three-phase approach...

**TEMPER-SYSTEM: end modified**

## Unchanged section

This content is identical in both versions.
```

Only sections that differ from the local version get annotated. Unchanged sections appear as-is for context.

### Resolution paths

**Pick a winner:**
```
temper sync resolve <uuid> --keep local     # discard remote, push local
temper sync resolve <uuid> --keep remote    # overwrite local with remote
```

**Merge both:**
```
temper merge <uuid>
```

Parses TEMPER-SYSTEM blocks, produces merged file with both contributions. Annotations converted to section headers with attribution:

```markdown
## Architecture

### Modified by pete@example.com — 2026-03-27T18:45:00Z
The sync protocol uses a three-phase approach...

### Modified by dana@example.com — 2026-03-27T19:02:00Z
The sync protocol uses a four-phase approach with pre-validation...
```

User edits the merged result, then resolves:
```
temper sync resolve <uuid>    # no --keep flag; current file is the resolution
```

### Auto-merge

When a subscription has `merge = "auto"`, conflicts skip the `.conflict.md` step. The merge algorithm runs inline (both contributions kept with section attribution), result written directly. Event log records auto-merge for after-the-fact review.

### Cleanup

`temper sync resolve` in all cases:
1. Removes `.conflict.md`
2. Updates manifest: new content_hash, state → `local_modified`
3. Clears conflict metadata from `.temper/conflicts/`
4. Next `temper sync` pushes resolution through upload pipeline
5. `/api/sync/resolve` called with resolution type and new content hash

### Partial sync with conflicts

`temper sync` with outstanding conflicts proceeds normally for non-conflicted resources. Only the conflicted resources are blocked. `temper sync status` lists unresolved conflicts.

---

## 7. CLI Command Surface

### New commands

```
temper auth login                       # Browser OAuth → local JWT
temper auth logout                      # Clear local auth.json
temper auth status                      # Current profile + provider

temper sync                             # Bidirectional reconcile (progress bar)
temper sync status                      # Pending changes without syncing
temper sync resolve <uuid>              # Resolve conflict (--keep local|remote)
temper merge <uuid>                     # Parse TEMPER-SYSTEM, produce merged file

temper vault add <path|url|glob>        # Onboard into knowledge base
temper vault status                     # Untracked, modified, conflicts

temper team create <name>               # Create team
temper team list                        # Teams you belong to
temper team show <slug>                 # Team detail + members
temper team invite <email>              # Invite (--role flag)
temper team join <token>                # Accept invitation
temper team leave <slug>                # Self-removal

temper resource create --type <t>       # Generic resource creation
temper resource share <uuid>            # --team <slug> --access <level>
temper resource transfer <uuid>         # --to <profile>
temper resource accept <uuid>           # Accept incoming transfer
```

### Evolved commands

```
temper search <query>                   # Always hits cloud API
  --mode semantic|keyword|graph         # Default: semantic
  --context <ctx>                       # Scope filter
  --team <slug>                         # Scope filter
  --depth <n>                           # For graph mode

temper context <topic>                  # Alias: search --mode graph
temper status                           # Sync state + vault state + auth state
temper events                           # Time-bounded visibility scoping
temper init <path>                      # Creates config.toml, device ID, manifest
temper warmup                           # Context primer (recent events, active tickets, sessions)
```

### Thin aliases (ergonomic sugar over resource commands)

```
temper ticket create "title"            # resource create --type ticket
temper ticket list                      # resource list --type ticket
temper ticket show <slug>               # resource get by slug
temper ticket move <slug> --stage X     # update workflowable state
temper ticket done <slug>               # move --stage done
temper milestone list                   # resource list --type milestone
temper milestone show <slug>            # resource get by slug
```

### Conventions

- All resource-based commands accept `--context <ctx>` for scoping
- All create commands accept stdin (content piped in, frontmatter prepended)
- All commands producing progress or streaming output support `--format json` for JSONL event stream
- Default output is human-friendly: progress bars, tables, color

---

## 8. CLI Opinion Boundaries — Primitives vs Orchestration

### Principle

CLI = CRUD primitives that map 1:1 to API calls or local file operations. Skill = workflow orchestration with judgment.

### Split

| Operation | CLI (primitive) | Skill (orchestration) |
|-----------|----------------|----------------------|
| Create a ticket | `temper ticket create "title"` | `/temper ticket start` (create + brainstorm + scope) |
| Move a ticket | `temper ticket move <slug> --stage X` | `/temper ticket start` (move + show + route) |
| List tickets | `temper ticket list` | — |
| Save a session | `temper resource create --type session` | `/temper session save` (summarize + link + sync) |
| Create research | `temper resource create --type research` | `/temper research save` (gather + create + populate) |
| Search | `temper search <query>` | — |
| Sync | `temper sync` | — |
| Vault add | `temper vault add <path>` | — |
| Warmup | `temper warmup` | — (hook target for session start) |

The skill layer composes CLI primitives into workflows: scope routing, brainstorming gates, session summarization, context discovery. Because the skill invokes CLI commands, everything flows through the same API contract.

---

## Deferred to Future Work

- **Chunk version retention policy** — design OTEL metrics first, decide with real data
- **Access control performance at scale** — premature to optimize before the system exists
- **Auto-sync as full network reconciliation** — future evolution once sync protocol is stable
- **Embedding model upgrades** — 768-dim bge-base-en-v1.5 is the baseline; re-embedding strategy TBD
- **Mobile/web client specifics** — they consume the same API contract but their UX is out of scope for R5
