# I5: temper Developer Experience — Epic Design Spec

## Overview

I5 redefines how developers and agents interact with temper end-to-end. It delivers: a nomenclature rename aligning CLI/vault/docs with the system's identity as a knowledge base (not a PM tool), the temper-client crate as the shared auth-aware API layer, the two-tier resource model (add vs. import), cloud-routed search, and auth compatibility with a future SvelteKit UI.

This is an epic. The deliverable is a goal roadmap with five sequenced sub-tasks (I5a–I5e).

## Two-Tier Resource Model

### Tier 1: Added Resources (`temper add`)

`temper add <file> --context <ctx>` is fire-and-forget:

1. Extract file to markdown locally via kreuzberg
2. Create resource via `POST /api/resources` (Rust endpoint)
3. Upload extracted markdown via `POST /api/upload` (TypeScript endpoint)
4. The async workflow runs: chunk → embed → store
5. Done. No local manifest entry, no ongoing sync relationship.

The resource exists in the cloud — searchable, pullable, deletable. The original file is never touched.

**Behaviors:**
- Re-running `temper add` on the same file is idempotent by content hash (no-op if unchanged, new version if changed)
- `temper pull <resource_id>` downloads as `<resource-uuid>.md` to current directory (or `--output` path). No frontmatter, no sync enrollment — read-only snapshot.
- `temper remove <resource_id>` deletes from cloud.

**Directory support:** `temper add --path <dir> --context <ctx>` with guardrails:
- Max depth: 2 (configurable in `~/.config/temper/config.toml`)
- Max total upload size per invocation: 50MB (configurable)
- Directory must be in `[add.allowed_paths]` config, or user gets a one-time confirmation prompt that adds it
- Respects `.gitignore` and `.temperignore` patterns
- Skips files kreuzberg can't extract

### Tier 2: Imported Resources (`temper import`)

`temper import <file> --context <ctx>` is vault-managed:

1. Extract to markdown via kreuzberg if needed
2. Copy markdown to vault at `<vault>/<context>/<slugified-name>.md`
3. Inject frontmatter: `temper_id`, `context`, `doctype`, `created`
4. Upload via temper-client (same two-step flow)
5. Register in vault manifest: `{ resource_id, vault_path, content_hash, synced_at }`

The vault copy is temper's — safe to annotate with `relates_to`/`extends`/tags, safe for bidirectional sync, conflict resolution, and version history.

**Behaviors:**
- `temper pull <resource_id>` for imported resources writes to vault path with frontmatter, enrolled in sync
- Other devices materialize imported resources via subscriptions or explicit pull
- Participates in knowledge graph (frontmatter edges)
- `temper remove <resource_id>` deletes from cloud and removes vault file + manifest entry (with confirmation)

### Promotion

`temper import <resource_id>` promotes an added resource to imported:
1. Download cloud content via `client.resources().content(id)`
2. Write to vault with frontmatter
3. Register in vault manifest
4. Cloud resource_id preserved

### Workflow Documents (Tasks, Goals, Sessions)

Tasks, goals, and sessions are always vault-managed (Tier 2). They are small markdown documents modified frequently (stage changes, session saves). They use a **lightweight sync path** that avoids Vercel Blob storage.

**Light path:**
1. CLI modifies the file on disk, updates frontmatter
2. `PUT /api/resources/:id/content` uploads the raw markdown directly
3. The same TypeScript workflow triggers, but with an inline content source
4. Workflow skips blob download and kreuzberg extraction, goes straight to chunk → embed → store

**Heavy path (files added/imported by users):**
1. File uploaded to Vercel Blob via `POST /api/upload`
2. Workflow downloads from blob, extracts via kreuzberg, then chunk → embed → store

The workflow discriminates via `content_source`:
- `{ blob: blob_url }` — heavy path, download and extract
- `{ inline: markdown_string }` — light path, skip to chunk

This keeps the chunk → embed → store pipeline shared. No duplication.

### Database Implication

The `blob_files` / `kb_chunks` schema doesn't change. The tier distinction lives in:
- Client-side vault manifest (for local state)
- A `resource_tier` field on the resource record (so the API can communicate tier in search results and sync)

## Nomenclature

### Renames

| Old | New | DB layer |
|-----|-----|----------|
| ticket | **task** | n/a (vault-only today) |
| milestone | **goal** | n/a (vault-only today) |
| project | **context** | `kb_contexts` (already) |
| scope: patch/feature/epic | **mode** + **effort** | n/a |

### Mode and Effort

**Mode** — what is the outcome:
- `plan` — the outcome is a plan: what to do, in what order, with what constraints. Discovery, research, design specs, roadmaps.
- `build` — the outcome is an artifact: code, a document, a design, a slide deck, a config. The thing itself, not a description of the thing.

**Effort** — how much ceremony:
- `small` — direct execution, no brainstorming
- `medium` — brainstorming + execution
- `large` — full pipeline or deep discovery, potentially multi-session

**Superpowers routing:**

| Mode | Effort | Workflow |
|------|--------|----------|
| build | small | Implement directly with tests |
| build | medium | Brainstorm → plan → implement |
| build | large | Brainstorm → plan → implement (multi-session, checkpoints) |
| plan | small | Quick research, write up findings |
| plan | medium | Brainstorm → design spec |
| plan | large | Deep discovery → goal roadmap → first actionable task |

### CLI Commands After Rename

```
temper task create/list/show/move/done/start
temper goal create/list/update
temper context add/remove/list          # replaces `temper project`
temper session save/list                # unchanged
temper search                           # unchanged
temper add / import / pull / remove     # new (I5c)
temper auth login/logout/status         # new (I5b)
```

`--context` flag everywhere (replaces `--project`).

### Frontmatter Migration

```yaml
# Before
project: temper
milestone: temper-cloud
scope: feature
stage: in-progress

# After
context: temper
goal: temper-cloud
mode: build
effort: medium
stage: in-progress
```

### Vault Directory Rename

- `tickets/<context>/` → `tasks/<context>/`
- `milestones/<context>/` → `goals/<context>/`
- `sessions/`, `research/`, `templates/` — unchanged

### What Gets Removed in I5a

- `temper index` command
- Local HNSW indexing code
- `candle-*` and `instant-distance` dependencies
- Embedder config section
- Old nomenclature (ticket, milestone, project, scope commands and types)

## Auth Architecture

### Provider-Agnostic OAuth with PKCE

`~/.config/temper/config.toml`:
```toml
[auth]
provider = "neon_auth"

[auth.providers.neon_auth]
authorize_url = "https://auth.neon.tech/authorize"
token_url = "https://auth.neon.tech/token"
client_id = "temper-cli"
scopes = ["openid", "email", "profile"]
```

### `temper auth login` Flow

1. Read provider config from `config.toml`
2. Generate PKCE challenge (code_verifier + code_challenge)
3. Open browser to `authorize_url` with `redirect_uri=http://localhost:{random_port}/callback`
4. Ephemeral local HTTP server listens for callback
5. Receive auth code, exchange at `token_url` for JWT + refresh token
6. Store at `~/.config/temper/auth.json`

### Token Storage

```json
{
  "provider": "neon_auth",
  "access_token": "eyJ...",
  "refresh_token": "...",
  "expires_at": "2026-03-29T19:00:00Z",
  "profile_id": "019537a2-..."
}
```

### Token Lifecycle

- temper-client checks `expires_at` before every request
- Auto-refresh when within 5 minutes of expiry
- `temper auth logout` clears `auth.json`
- `temper auth status` shows provider, profile email, expiry as JSON

### SvelteKit Compatibility

Same Neon Auth OAuth app issues tokens for both CLI and web UI. CLI uses authorization code flow with PKCE (public client, no client_secret). Web UI uses the same flow via browser redirect. Both produce JWTs verified by the same JWKS endpoint. Same `sub` claim → same profile. No special coordination needed beyond sharing the OAuth client_id or having two client registrations under the same provider.

## temper-client Crate

### Responsibility

Auth-aware HTTP client wrapping the temper cloud API. Shared by temper-cli, temper-mcp, and future consumers.

### Structure

```
crates/temper-client/src/
├── lib.rs              # TemperClient constructor, config
├── auth.rs             # Token storage, refresh, OAuth flow
├── http.rs             # Base reqwest client, auth header injection, retry
├── resources.rs        # ResourceClient — CRUD, content
├── upload.rs           # UploadClient — two-step add/import flow
├── search.rs           # SearchClient — query, modes
├── profile.rs          # ProfileClient — get, update, auth links
├── events.rs           # EventClient — list, filter
├── error.rs            # ClientError enum, HTTP status mapping
```

### API Surface

```rust
let client = TemperClient::new(config)?;

// Auth
client.auth().login().await?;
client.auth().logout()?;
client.auth().status()?;

// Resources
client.resources().list(context, doc_type).await?;
client.resources().get(id).await?;
client.resources().create(request).await?;
client.resources().update(id, request).await?;
client.resources().delete(id).await?;
client.resources().content(id).await?;
client.resources().sync_content(id, markdown).await?;  // Light path

// Upload (two-step abstracted)
client.upload().add(file_path, context).await?;
client.upload().import(file_path, context).await?;

// Search
client.search().query(q, mode, filters).await?;

// Profile
client.profile().get().await?;
client.profile().update(request).await?;

// Events
client.events().list(filters).await?;
```

### Automatic Behaviors

- Auth header injected on every request
- Token refresh before expiry
- `X-Temper-Client-Id` header from device identity
- Retry on 429/503 with exponential backoff
- All responses mapped to `Result<T, ClientError>`

### Error Types

```rust
enum ClientError {
    NotAuthenticated,
    TokenExpired,
    Forbidden,
    NotFound { resource: String },
    Conflict { message: String },
    RateLimited { retry_after: Duration },
    Server { status: u16, message: String },
    Network(reqwest::Error),
}
```

### Dependencies

`temper-core` (types), `reqwest` + rustls, `tokio`, `serde`/`serde_json`, `open` (browser), `hyper` (callback server).

## Search

- Always routes through temper-client to cloud API
- Requires authentication (errors with guidance to run `temper auth login` if not)
- No local HNSW fallback
- Output is always JSON
- `--pretty` flag for human-readable formatted JSON (default when stdout is a TTY)
- Raw JSON when piped

### Result Schema

```json
{
  "results": [
    {
      "resource_id": "019537a2-...",
      "title": "Design sync protocol",
      "context": "temper",
      "doc_type": "note",
      "score": 0.87,
      "snippet": "The sync protocol uses...",
      "tier": "imported",
      "local": true,
      "vault_path": "goals/temper/design-sync-protocol.md"
    }
  ],
  "mode": "semantic",
  "total": 12
}
```

## Roadmap

| Ticket | Title | Deliverable | Mode | Effort |
|--------|-------|-------------|------|--------|
| **I5a** | Nomenclature rename | Rename CLI commands, vault dirs, frontmatter, skill, docs, memories. Remove local HNSW/indexing. Clean break migration. | build | medium |
| **I5b** | temper-client + CLI auth | temper-client crate with typed methods for all API endpoints. `temper auth login/logout/status`. Token lifecycle. Integration tests against live API. | build | medium |
| **I5c** | Add, import, pull | `temper add` / `temper import` / `temper pull` / `temper remove`. Two-tier model. Light path for workflow docs. kreuzberg extraction in CLI. Directory support with guardrails. | build | large |
| **I5d** | Cloud search | Cloud-routed `temper search`. JSON-only output. Agent-friendly result schema with tier and locality info. | build | medium |
| **I5e** | UI auth compatibility | Validate Neon Auth OAuth works for both CLI and SvelteKit. Document shared auth model. | plan | small |

### Sequencing

I5a → I5b → I5c → I5d → I5e (strictly sequential — each builds on the prior).

### Dependencies

- I5a: none (works on existing vault + CLI code)
- I5b: I5a (uses new nomenclature in client types)
- I5c: I5b (uses temper-client for uploads), requires `PUT /api/resources/:id/content` endpoint (light path)
- I5d: I5b (uses temper-client for search)
- I5e: I5b (validates auth flow)

## Open Questions (Deferred to Sub-Tickets)

1. **Batch zip upload** (I5c): Should `temper add --path` zip files into a single upload? If so, the API needs a bulk resource creation endpoint and the TypeScript workflow needs zip-awareness. Research during I5c scoping.

2. **Light path API design** (I5c): `PUT /api/resources/:id/content` vs. extending the existing upload endpoint. The workflow needs the `content_source` discriminator (`blob` vs. `inline`). Design during I5c.

3. **Effort defaults** (I5a): Should mode/effort have smart defaults based on content? e.g., `temper task create` defaults to `mode: build, effort: medium`. Decide during I5a implementation.

4. **Config migration** (I5a): The `~/.config/temper/config.toml` structure may need new sections (`[auth]`, `[add]`). Define during I5a/I5b.

5. **`.temperignore` patterns** (I5c): What default patterns to exclude. Define during I5c.
