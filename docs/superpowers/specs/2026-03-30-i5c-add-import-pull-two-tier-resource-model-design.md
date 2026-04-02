# I5c: Add, Import, Pull — Two-Tier Resource Model

## Summary

Implement the two-tier resource model for temper: `temper add` (fire-and-forget upload), `temper import` (vault-managed with frontmatter), `temper pull` (download), and `temper remove` (delete). The CLI extracts documents to markdown locally via kreuzberg (Rust). The server handles chunking and embedding via a new inline processing endpoint (`/api/ingest`). No local embeddings — temper-embed is deferred.

## Architecture

### Two Processing Paths

```
CLI Light Path (new):
  file → kreuzberg (Rust, CLI-side) → markdown
    → POST /api/ingest (TypeScript)
      → chunkText() → embedTexts() → store to DB
    ← ResourceRow JSON

Blob Path (existing, for future web UI / large files):
  file → POST /api/upload → Vercel Blob
    → Workflow (extract → chunk → embed → store)
    ← { blob_file_id, status: "pending" }
```

The CLI extracts locally, producing markdown. For documents under the size threshold (default 2MB extracted markdown), the light path sends markdown directly to `/api/ingest` for synchronous inline processing. Above the threshold, the CLI falls back to the existing blob upload path with async workflow processing.

### Shared Processing Library

Refactor existing Vercel Workflow functions into a shared library that both the ingest endpoint and the workflow consume:

```
packages/temper-cloud/src/
  processing/              # NEW — shared processing core
    chunk.ts               # chunkText() extracted from workflow/chunk.ts
    embed.ts               # embedTexts() extracted from workflow/embed.ts
    store.ts               # storeChunks() extracted from workflow/store.ts
    index.ts               # barrel export
  workflow/
    chunk.ts               # thin wrapper → re-exports from processing/
    embed.ts               # thin wrapper → re-exports from processing/
    store.ts               # thin wrapper → re-exports from processing/
    extract.ts             # unchanged (blob path only)
    process-upload.ts      # unchanged, imports from processing/
```

## Server-Side: `/api/ingest`

### `POST /api/ingest`

Create a new resource and process its content inline.

**Accepts:** `multipart/form-data`
- `metadata` (JSON): title, kb_context_id, kb_doc_type_id, uri, slug?, mimetype?, tags?, metadata? (device_id, original_path)
- `content` (string): extracted markdown

**Returns:** `ResourceRow` JSON (id, title, slug, content_hash, kb_context_id, kb_doc_type_id, etc.)

**Processing:**
1. Validate JWT (shared `verifyJwt()` from `auth.ts`)
2. Compute SHA-256 content hash
3. Check `resources.content_hash` for existing resource with matching hash owned by the same profile — if found, return existing (idempotent)
4. Insert resource record into DB
5. `chunkText(markdown)` → `embedTexts(chunks)` → `storeChunks(resource_id, chunks, embeddings)`
6. Return resource JSON

### `PUT /api/ingest/[id]`

Update content for an existing resource (light sync path for workflow docs).

**Accepts:** `multipart/form-data`
- `content` (string): markdown

**Returns:** Updated `ResourceRow` JSON

**Processing:**
1. Validate JWT + `can_modify_resource()` check
2. Compute content hash — if unchanged, return early
3. `chunkText()` → `embedTexts()` → version bump (mark old chunks `is_current=false`) → `storeChunks()`
4. Update resource `content_hash` and `updated` timestamp
5. Return updated resource JSON

### Auth

Same JWT validation as existing endpoints via `verifyJwt()` in `packages/temper-cloud/src/auth.ts` (RS256, Auth0).

## CLI Commands

### `temper add <path>`

Fire-and-forget upload. Resource becomes searchable and pullable but not vault-managed.

```
temper add paper.pdf
temper add --path ./research/        # directory mode
temper add https://example.com/doc   # stub: "URL support not yet implemented"
```

**Single file flow:**
1. Detect URL → return "not yet implemented" error
2. Extract to markdown via kreuzberg
3. Compute SHA-256 of extracted markdown
4. `POST /api/ingest` with metadata (title from filename, context, doc_type) + content
5. Server returns ResourceRow (idempotent — duplicate hash returns existing resource)
6. Print resource ID and title

**Directory mode** (`--path <dir>`):
1. Pre-flight: walk directory, compute total size, report file count
2. Abort if guardrails exceeded (user can `--force` for size limit)
3. Extract each file locally via kreuzberg
4. Upload sequentially with concurrency limit (default 4)
5. Progress output (TTY: progress bar, non-TTY/`--format json`: JSONL)
6. Summary: `✓ 15 added, ✗ 2 failed, 3 skipped (duplicate)`

**Metadata stored on resource:**
- `ingestion_source`: original file path
- Tags/metadata: device ID, original path (provenance tracking)

### `temper import <path|resource_id>`

Vault-managed. Extracts markdown, writes to vault with frontmatter, uploads to cloud, registers in manifest.

**From file:**
```
temper import paper.pdf
temper import ./notes/meeting.md
temper import --path ./research/     # directory mode
```

1. Extract to markdown (kreuzberg if non-markdown, read directly if already markdown)
2. `POST /api/ingest` with metadata + content → ResourceRow
3. Write vault file at `{context}/{doc_type}/{uuid}.md` with frontmatter:
   ```yaml
   ---
   temper-id: 019537a2-...
   title: "Paper Title"
   context: temper
   doc_type: resource
   ingestion_source: /Users/pete/paper.pdf
   created: 2026-03-30T16:00:00Z
   ---
   ```
4. Register in manifest (UUID → vault path, content_hash, remote_hash, state: clean)

**Promotion from added** (`temper import <resource_id>`):
1. `GET /api/resources/:id/content` — fetch reconstituted markdown from cloud
2. Write vault file with frontmatter (using resource metadata for context, doc_type, title)
3. Register in manifest
4. Resource now enrolled for future I6 sync

**Directory mode:** same guardrails and concurrency as `temper add --path`, each file goes through import flow.

### `temper pull <resource_id>`

Download a resource to the local filesystem.

- **Added resource** (not in manifest): Download reconstituted markdown as `{uuid}.md` in current directory. Read-only snapshot, no vault enrollment.
- **Imported resource** (in manifest): Download to vault path from manifest entry, update frontmatter if changed, mark manifest entry clean.

### `temper remove <resource_id>`

Delete from cloud. If imported, prompt for vault cleanup.

```
temper remove 019537a2-...
temper remove 019537a2-... --force   # skip confirmation
```

1. `DELETE /api/resources/:id` via existing client
2. If resource is in manifest: prompt "Also remove vault file at {path}? [y/N]"
3. On confirm: delete vault file, remove manifest entry

## CLI/Rust Design

### kreuzberg Integration

Feature-flagged dependency in `temper-cli`:

```toml
[features]
default = ["extract"]
extract = ["dep:kreuzberg"]
```

When `extract` is disabled, `temper add` and `temper import` only accept markdown files. Clear error if a non-markdown file is provided: "PDF extraction requires the 'extract' feature. Install with: cargo install temper-cli --features extract"

Extraction wrapper in `src/extract.rs`:
- `extract_to_markdown(path: &Path) -> Result<ExtractionResult>`
- Returns `ExtractionResult { content: String, mime_type: String }`
- Markdown files: read directly (no kreuzberg needed)
- Other formats: delegate to kreuzberg

### temper-client Additions

New `IngestClient` on `TemperClient`:

```rust
pub struct IngestClient { /* HttpClient */ }

impl IngestClient {
    /// POST /api/ingest — create resource + process content inline
    pub async fn create(&self, request: &IngestRequest) -> Result<ResourceRow>;

    /// PUT /api/ingest/:id — update content for existing resource
    pub async fn update(&self, id: Uuid, content: &str) -> Result<ResourceRow>;
}

pub struct IngestRequest {
    pub content: String,                    // extracted markdown
    pub title: String,
    pub kb_context_id: Uuid,
    pub kb_doc_type_id: Uuid,
    pub uri: String,
    pub slug: Option<String>,
    pub mimetype: Option<String>,
    pub tags: Option<Vec<String>>,
    pub metadata: Option<serde_json::Value>, // device_id, original_path
}
```

### Vault Path Convention

Imported resources are stored at deterministic paths:

```
{vault_root}/{context}/{doc_type}/{uuid}.md
```

Example: `knowledge/temper/resource/019537a2-1234-7890-abcd-ef0123456789.md`

Frontmatter, tags, and search carry findability — directory structure is for manifest management, not human navigation.

### Manifest

On `temper import`, write a manifest entry immediately:

```rust
manifest.entries.insert(resource_id, ManifestEntry {
    path: "temper/resource/019537a2-....md",
    content_hash: sha256_of_vault_file,
    remote_hash: response.content_hash,
    synced_at: Utc::now(),
    state: ManifestEntryState::Clean,
});
```

This prepares for I6 sync without implementing the full sync protocol.

### Directory Guardrails

```rust
pub struct DirectoryConfig {
    pub max_depth: usize,              // default: 2
    pub max_total_bytes: u64,          // default: 50MB (pre-extraction size)
    pub max_concurrent: usize,         // default: 4
    pub allowed_extensions: Vec<String>,
    pub ignore_patterns: Vec<String>,  // .gitignore + .temperignore
}
```

Pre-flight walk reports file count and total size. Abort with message if exceeded. `--force` overrides size limit.

### CLI Output

**TTY mode** (default): `indicatif` multi-progress bar

```
temper add --path ./research/
  Extracting files...
  [████████████░░░░] 12/17  extracting: analysis.pdf

  Uploading...
  [████████░░░░░░░░]  8/17  uploading: analysis.md

  ✓ 17 resources added (3 skipped, duplicate hash)
```

Single file:
```
temper add paper.pdf
  Extracting... done (142 KB markdown)
  Uploading... done
  ✓ Added: "Research Paper Title" (019537a2-...)
```

**JSON mode** (`--format json` or non-TTY):
```jsonl
{"event":"extract","file":"paper.pdf","status":"done","size_bytes":145832}
{"event":"upload","file":"paper.pdf","status":"done","resource_id":"019537a2-..."}
{"event":"complete","added":17,"skipped":3,"failed":0}
```

### Size-Based Path Routing

The CLI chooses the processing path based on extracted markdown size:

- **≤ 2MB**: Light path via `POST /api/ingest` (synchronous)
- **> 2MB**: Blob upload via existing `POST /api/upload` (async workflow)
- Threshold configurable in `~/.config/temper/config.toml` under `[sync]`

## Error Handling

**Network failures:**
- Retry with exponential backoff (3 attempts) for transient HTTP errors (429, 502, 503)
- During directory operations: continue on failure, report at end
- Partial success: `✓ 15 added, ✗ 2 failed (see above)`

**Extraction failures:**
- kreuzberg can't process a file: skip with warning in batch mode, error in single-file mode
- Unsupported format without `extract` feature: clear error with install instructions

**Auth failures:**
- 401: `Run 'temper auth login' to authenticate`

**Idempotency:**
- Content hash collision on `POST /api/ingest`: return existing resource (200), CLI reports "skipped, duplicate"

## Testing Strategy

**Rust (temper-cli, temper-client):**
- Unit: extraction wrapper (mock kreuzberg, verify markdown + hash)
- Unit: `IngestClient` request construction (multipart format, headers)
- Unit: directory walking (guardrails, ignore patterns, depth limits)
- Unit: manifest entry creation on import
- Integration (`test-db`): full add/import/pull/remove against local API

**TypeScript (temper-cloud):**
- Unit: shared processing library (chunk, embed, store)
- Integration: `/api/ingest` POST creates resource + chunks
- Integration: `/api/ingest/:id` PUT updates with version bump, old chunks marked not current

**CLI output:**
- Snapshot tests for TTY progress format
- Structured tests for JSONL output (parse and verify event shapes)

**End-to-end:**
- `temper add <file>` → `temper pull <id>` → verify markdown round-trips
- `temper import <file>` → verify vault file with correct frontmatter + manifest entry
- `temper import <resource_id>` → verify promotion from added to vault-managed
- `temper remove <id>` → verify cloud deletion + vault cleanup
- Directory add with mixed file types, some failing extraction

## Deferred

- **URL support** (`temper add <url>`): Stubbed with "not yet implemented" message. Stretch goal for end of I5c if time permits.
- **Zip-batch upload**: Sequential + concurrency for I5c. Zip-and-batch documented as future optimization for large directory imports.
- **temper-embed crate**: No local embeddings. Server handles all embedding. Deferred indefinitely.
- **Full sync protocol** (I6): I5c writes manifest entries but does not implement bidirectional sync.
- **Light sync for workflow docs**: `PUT /api/ingest/:id` endpoint is built; wiring existing vault commands (task, session, goal) to auto-upload on modification is a follow-up.
