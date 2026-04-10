# Unified Rust Ingest Pipeline + MCP Round-Trip Design

**Date:** 2026-04-09
**Task:** 2026-04-09-temper-mcp-bug-fixes-and-round-trip-testing
**Context:** temper
**Mode/Effort:** build/medium (likely spans 2-3 sessions)

## Summary

Collapse the current two-path ingest architecture into a single synchronous Rust
request-response pipeline. Today the Rust `/api/ingest` endpoint requires clients to send
pre-computed chunks and embeddings, so the TypeScript `/api/content-ingest` endpoint
exists solely as an async proxy that accepts raw markdown, triggers a Vercel Workflow to
chunk + embed + store, and returns 202. MCP tools use that proxy path, which means agents
get a metadata shell back immediately with no reliable way to know when processing
finishes — the root of the round-trip UX flakiness.

After this change, the Rust `/api/ingest` endpoint optionally accepts raw markdown and
runs chunking + embedding inline via a new shared `temper_ingest::pipeline::prepare_markdown`
function. The ONNX model (`BAAI/bge-base-en-v1.5` quantized) is bundled into the Vercel
function binary via `include_bytes!`, eliminating HuggingFace Hub cold-start downloads.
The TypeScript proxy-embed path is retired entirely. MCP tools call the ingest service
directly — no HTTP detour — and return the fully processed resource in a single request,
so no polling, no status field, no fire-and-forget.

Alongside the pipeline pivot, the MCP tool surface gains doc-type schema awareness: a
new `describe_doc_type` tool exposes the JSON Schemas already in `crates/temper-core/schemas/`,
`list_doc_types` surfaces required fields, and `create_resource`/`update_resource` accept
a `managed_meta` field that is validated server-side against the schema before any compute
or DB work.

The CLI's existing precomputed-chunks path is preserved unchanged, supporting its
client-side embedding needs for bulk ingest and the upcoming local-HNSW offline search
work.

## Background

### Current architecture (what breaks)

Two ingest paths exist:

1. **Rust `POST /api/ingest`** — accepts a fully-populated `IngestPayload` including
   `chunks_packed` (base64 MessagePack of `Vec<PackedChunk>` with 768-dim embeddings)
   and `content_hash`. Used by the CLI, which runs `temper_ingest::chunk::chunk_markdown`
   + `embed::embed_texts` locally before POSTing. Atomic server-side DB write via
   `create_resource_with_manifest`.
2. **TypeScript `POST /api/content-ingest`** — accepts a markdown body, stores metadata,
   fire-and-forgets a Vercel Workflow (`api/workflows/process-content-ingest.ts`) that
   downloads the BAAI/bge-base-en-v1.5 model from HuggingFace Hub, runs chunking,
   embedding, and DB writes asynchronously. Returns 202 immediately.

MCP's `create_resource` tool currently calls the TypeScript proxy via `spawn_content_ingest_post`
(in `crates/temper-mcp/src/tools/resources.rs`), which means agents get a metadata shell
back with no signal when processing completes. Cold-start HuggingFace model downloads
compound this: the first call after a cold function boot can take 10-30s before the
embedding step even starts. No status column exists on `kb_resources`; the only signal is
a `body_processed` event in `kb_events` that requires the client to poll.

Separately, MCP `create_resource` takes only `title`, `content?`, `slug?`, `origin_uri?`,
`owner?`, `context_name`, `doc_type_name`. It does not accept doc-type-specific managed
frontmatter, and `list_doc_types` returns only `id` + `name` — so agents have no way to
discover or supply the required fields for a `task` (`temper-stage`, `temper-mode`, ...),
a `goal` (`temper-status`), etc. The CLI uses the JSON Schemas at
`crates/temper-core/schemas/*.schema.json` via `temper_core::schema::{load_schema,
validate_frontmatter, updatable_fields}`, but MCP never touches that module.

### Target architecture (what replaces it)

```
                            ┌── CLI (unchanged): runs prepare_markdown LOCALLY via temper-ingest,
                            │   sends IngestPayload { content, chunks_packed: Some(_), content_hash: Some(_) }
                            │
POST /api/ingest  ──────────┤── External HTTP clients: send IngestPayload { content, chunks_packed: None, content_hash: None }
                            │
MCP create_resource ────────┤── Calls ingest_service::create_resource_with_manifest directly
                            │   (same binary; no HTTP detour)
                            │
MCP update_resource ────────┘── Same service-direct call pattern
                                         │
                                         ▼
                   ingest_service::create_resource_with_manifest(payload)
                                         │
                                         ├── validate managed_meta against doc_type schema  (fail-fast)
                                         ├── if chunks_packed is None:
                                         │     compute content_hash server-side from content
                                         │     call temper_ingest::pipeline::prepare_markdown(&content)
                                         │     wire the result into payload.chunks_packed
                                         ├── atomic DB write via existing SQL function
                                         └── return fully processed EnrichedResource
```

**Key invariants:**

- The branch between the precomputed path and the markdown path lives in the service
  layer, not the HTTP handler or MCP tool. Both callers pass an `IngestPayload` and the
  service decides whether to run the pipeline.
- Atomicity is preserved: the existing DB write is a single SQL function call that wraps
  resource + manifest + chunks + event in one transaction. First implementation step is to
  confirm this by reading `create_resource_with_manifest`.
- CLI behavior is unchanged. CLI keeps running `temper-ingest` locally and sending
  precomputed chunks. This supports bulk ingest and the upcoming local-HNSW work.
- Validation runs before compute. Rejected requests cost ~zero CPU.
- If the call succeeds, the resource is fully processed and immediately searchable. No
  polling, no `processing_status` column, no event-based signaling required for correctness.
- The existing `body_processed` event in `kb_events` continues to fire for audit purposes
  but nothing depends on it for correctness.

## Wire Type Changes

In `crates/temper-core/src/types/ingest.rs`:

```rust
pub struct IngestPayload {
    pub title: String,
    pub origin_uri: String,
    pub context_name: String,
    pub doc_type_name: String,
    pub slug: String,
    pub content: String,

    /// `"sha256:<hex>"`. Server computes if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_meta: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_meta: Option<serde_json::Value>,

    /// Base64 MessagePack of `Vec<PackedChunk>`. Server computes via
    /// `temper_ingest::pipeline::prepare_markdown` if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunks_packed: Option<String>,
}

// DELETED: ContentIngestRequest (was the TS proxy wire type)
```

Because both optional fields use `#[serde(skip_serializing_if = "Option::is_none")]`,
CLI-produced JSON (with `Some(...)` for both) serializes to the same wire shape as today.
The HTTP protocol is backward-compatible; only the Rust struct type changes.

## Shared Pipeline Function

New module: `crates/temper-ingest/src/pipeline.rs`

```rust
/// Chunk markdown, embed chunks, and pack the result for storage.
///
/// This is the single source of truth for "turn raw markdown into indexed chunks."
/// Used by the API server (when `IngestPayload.chunks_packed` is absent), by MCP tools
/// via the same service function, and can be used by the CLI (via `build_ingest_payload`)
/// to avoid drift between client-side and server-side pipelines.
pub fn prepare_markdown(content: &str) -> Result<Vec<PackedChunk>, EmbedError> {
    let chunks = crate::chunk::chunk_markdown(content);
    let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
    let embeddings = crate::embed::embed_texts(&texts)?;
    // pack each chunk with its embedding and content_hash
    Ok(pack_chunks(&chunks, &embeddings))
}
```

The function takes `&str` and returns `Vec<PackedChunk>` — nothing more. It does not
touch `IngestPayload` or any other wire type, preserving `temper-ingest`'s layering
(pure compute, no dependency on `temper-core` wire types).

### CLI refactor

`crates/temper-cli/src/actions/ingest.rs:141-193` (`build_ingest_payload`) currently
inlines `chunk_markdown → embed_texts → pack`. It is refactored to call
`temper_ingest::pipeline::prepare_markdown(content)` instead, eliminating drift between
CLI and server pipelines. The CLI continues to POST the precomputed payload to
`/api/ingest`; only the intermediate function is different.

## Service Layer Changes

In `crates/temper-api/src/services/ingest_service.rs`:

- New helper `validate_managed_meta(doc_type_name, managed_meta)`: converts
  `Option<&Value>` to YAML, calls `temper_core::schema::validate_frontmatter(doc_type_name, &yaml)`,
  returns `IngestError::Validation { doc_type, issues }` if non-empty. Strips tier-1
  system-managed fields before validation with a warning log.
- New helper `strip_system_managed_fields(managed_meta)`: walks `SYSTEM_MANAGED_FIELDS`
  (re-exported from `temper_core::schema`) and removes `temper-id`, `temper-created`,
  `temper-updated`, `temper-owner`, `temper-provisional-id`, `temper-source`,
  `temper-legacy-id`.
- `create_resource_with_manifest(payload)` is updated:
  1. Call `strip_system_managed_fields` on input `managed_meta`.
  2. Call `validate_managed_meta(doc_type_name, managed_meta.as_ref())`. Fail-fast on
     error.
  3. If `payload.chunks_packed.is_none()`:
     - Compute `content_hash = format!("sha256:{hex}", hex=sha256(payload.content))`.
     - Call `temper_ingest::pipeline::prepare_markdown(&payload.content)`.
     - Base64-MessagePack-encode the result into `payload.chunks_packed`.
  4. Proceed with the existing atomic DB write path.
- Update path (`update_resource_content` or equivalent) applies the same validation +
  pipeline logic. Rejects tier-2 fields (`temper-context`, `temper-type`, `slug`) in
  `managed_meta` with `IngestError::StructuralMoveNotSupported`, pointing the agent at
  the CLI for moves.

### `IngestError` additions

```rust
pub enum IngestError {
    // ... existing variants
    Validation {
        doc_type: String,
        issues: Vec<temper_core::schema::ValidationIssue>,
    },
    StructuralMoveNotSupported {
        field: String,
        message: String,
    },
    MetaSerialize(serde_yaml::Error),
    Embed(temper_ingest::EmbedError),
    Chunk(/* if chunking can fail */),
}
```

### Atomicity verification

First implementation step: read `create_resource_with_manifest` top-to-bottom and confirm
the resource row + manifest row + chunks + event are all written inside a single SQL
function call or a single sqlx transaction. If not, wrap them. Do not proceed with the
markdown-path branch until this is proven, because a mid-request failure after the pivot
no longer has a "retry via the workflow" fallback.

## MCP Tool Surface Changes

In `crates/temper-mcp/src/tools/doc_types.rs`:

### `list_doc_types` — extended output

```rust
pub struct DocTypeSummary {
    pub id: Uuid,
    pub name: String,
    pub has_schema: bool,
    pub required_fields: Vec<String>,
}
```

Populated by loading each schema via `temper_core::schema::load_schema(name)` and reading
the `required` array. No caching in this branch; schemas are compile-time `include_str!`
so the parse cost is trivial.

### `describe_doc_type` — new tool

```rust
pub struct DescribeDocTypeInput {
    pub name: String,
}

pub struct DescribeDocTypeResponse {
    pub name: String,
    pub schema: serde_json::Value,
    pub required_fields: Vec<String>,
    pub enum_fields: BTreeMap<String, Vec<String>>,
    pub example_managed_meta: serde_json::Value,
}
```

`example_managed_meta` is constructed server-side by walking the schema's `required`
array and populating each field with the first enum value (or a type-appropriate default
for non-enum fields). For `task`: `{"temper-stage": "backlog", "slug": "<slug-placeholder>"}`.
Gives LLM clients a concrete template to copy and modify.

### `create_resource` — two new fields

```rust
pub struct CreateResourceInput {
    // ... existing: context_name, doc_type_name, title, content?, slug?, origin_uri?, owner?
    pub managed_meta: Option<serde_json::Value>,
    pub open_meta: Option<serde_json::Value>,
}
```

Both flow straight into `IngestPayload.managed_meta` / `IngestPayload.open_meta`. The
tool handler calls `ingest_service::create_resource_with_manifest(payload)` directly
(same binary — no HTTP detour). The `spawn_content_ingest_post` helper is deleted.

### `update_resource` — two new fields

```rust
pub struct UpdateResourceInput {
    pub id: Uuid,
    pub title: Option<String>,
    pub slug: Option<String>,
    pub content: Option<String>,        // already present
    pub managed_meta: Option<serde_json::Value>,
    pub open_meta: Option<serde_json::Value>,
}
```

Structural moves (`context_to`, `doc_type_to`, `slug_to`) are explicitly **not** added in
this branch. If an agent passes a tier-2 field in `managed_meta`, the service layer
returns `IngestError::StructuralMoveNotSupported` with a clear message.

## Schema Validation Flow

**Where it happens:** In the service layer, at the top of both `create_resource_with_manifest`
and the update path, before any mutation or compute. Single source of truth for all
callers (HTTP, MCP, anything else that hits the service function).

**Validation logic:** Uses `temper_core::schema::validate_frontmatter(doc_type_name, yaml)`
which already exists and handles required-field checks, enum constraints, type matching,
and returns `Vec<ValidationIssue>` with `field`, `kind`, `message`. MCP surfaces these in
the tool error response as structured data.

**Synthetic frontmatter merge (important).** The doc-type schemas list some tier-2 fields
in their `required` arrays — for example `task.schema.json` requires both `temper-stage`
(tier-3, expected in `managed_meta`) and `slug` (tier-2, passed as a top-level MCP/HTTP
parameter, not inside `managed_meta`). If the validation helper naively passed just
`managed_meta` to `validate_frontmatter`, every `task` creation would fail with a
spurious "missing slug" error.

To avoid this, the `validate_managed_meta` helper constructs a synthetic frontmatter
object that merges:

1. The top-level parameters from `IngestPayload`: `slug`, `title`, `context_name → temper-context`, `doc_type_name → temper-type`.
2. Placeholder values for tier-1 identity/audit fields that the server will populate
   after validation: `temper-id`, `temper-created`, `temper-updated`, `temper-owner`,
   etc. (A zero UUID and the current timestamp are fine — `validate_frontmatter` only
   checks presence and type, not semantic correctness of these fields.)
3. The user-supplied `managed_meta` merged on top, overriding any of the above if
   specified.

This synthetic object is what gets YAML-serialized and passed to `validate_frontmatter`.
The result: only tier-3 missing/invalid fields produce `ValidationIssue`s. Tier-1 and
tier-2 required fields are satisfied by the server-populated and top-level-parameter
values respectively, without the agent having to pass them in `managed_meta`.

**YAML round-trip:** `validate_frontmatter` takes a YAML string. We convert the synthetic
`Value → YAML` via `serde_yaml::to_string` at the validation boundary. If this proves
wasteful later, add a sibling `validate_frontmatter_value(&Value)` helper in
`temper_core::schema`. Not now.

### Tier boundaries

| Tier | Fields | Handling |
|---|---|---|
| 1 — Identity/audit | `temper-id`, `temper-created`, `temper-updated`, `temper-owner`, `temper-provisional-id`, `temper-source`, `temper-legacy-id` | Silently stripped from input `managed_meta` before validation. Server-set. Warning log if agent passes them. |
| 2 — Structural | `temper-context`, `temper-type`, `slug` | Accepted via top-level params (`context_name`, `doc_type_name`, `slug`) on `create_resource`. Not accepted in `managed_meta`. `update_resource` rejects them with `StructuralMoveNotSupported` — deferred. |
| 3 — Doc-type-specific | `temper-stage`, `temper-mode`, `temper-effort`, `temper-goal`, `temper-branch`, `temper-pr`, `temper-status`, `temper-seq`, etc. | Accepted in `managed_meta`, validated against the doc_type schema. |

### Error shape surfaced to MCP clients

```rust
pub struct ValidationErrorPayload {
    pub doc_type: String,
    pub issues: Vec<temper_core::schema::ValidationIssue>,
}
```

Returned as `CallToolResult::Error` with structured data. Agents can introspect
field-level issues and retry programmatically. Chunking/embed/db errors surface as plain
text messages for now — they're rare in the request-response path and agents typically
can't retry them usefully.

## Model Packaging and Binary Size

### Approach

`include_bytes!` the quantized ONNX model and tokenizer directly into the `temper-ingest`
crate. Load from memory at runtime via `ort::Session::commit_from_memory` and
`tokenizers::Tokenizer::from_bytes`. Self-contained binary; no runtime HuggingFace Hub
access, no build-time network required, no Vercel asset-packaging dependency beyond the
binary itself.

### Files to bundle

New directory: `crates/temper-ingest/models/bge-base-en-v1.5/`

```
crates/temper-ingest/models/bge-base-en-v1.5/
├── model_quantized.onnx       (~45 MB, int8)
├── tokenizer.json             (~2 MB)
├── config.json
├── special_tokens_map.json
└── tokenizer_config.json
```

Sourced from the upstream `BAAI/bge-base-en-v1.5` repo on HuggingFace under its `onnx/`
directory.

### Git storage

Git LFS for `model_quantized.onnx`. Plain git for the rest. New `.gitattributes`:

```
crates/temper-ingest/models/**/*.onnx filter=lfs diff=lfs merge=lfs -text
```

Vercel LFS support is confirmed enabled on the temper-cloud project. GitHub Actions
handles LFS via `actions/checkout@v4` with `lfs: true`. The new `test-rust-embed` CI job
sets this.

### Loading code

In `crates/temper-ingest/src/embed.rs`, replace the current `hf_hub::api::sync` code path
(`embed.rs:31-42`) with:

```rust
static MODEL_BYTES: &[u8] = include_bytes!("../models/bge-base-en-v1.5/model_quantized.onnx");
static TOKENIZER_BYTES: &[u8] = include_bytes!("../models/bge-base-en-v1.5/tokenizer.json");

fn load_model() -> Result<Model, EmbedError> {
    let tokenizer = tokenizers::Tokenizer::from_bytes(TOKENIZER_BYTES)
        .map_err(EmbedError::Tokenizer)?;
    let session = ort::session::Session::builder()?
        .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)?
        .commit_from_memory(MODEL_BYTES)?;
    Ok(Model { tokenizer, session })
}
```

Lazy `OnceLock<Result<Model, String>>` pattern stays, model loads on first embed call
rather than at function boot. Per Phase 2b decision: most Vercel function cold starts do
not need the model, so eager init would slow the common path to benefit the uncommon one.

The `hf-hub` dependency is removed from `temper-ingest/Cargo.toml` after this change.

### `ort` linking strategy

`ort` v2.0.0-rc.12 offers three linking modes:

- `download-binaries` (default) — downloads prebuilt `libonnxruntime` at compile time.
  Requires network during `cargo build`. Blocked on Vercel's sandboxed build env.
- `load-dynamic` — expects `libonnxruntime.so` at runtime via `ORT_DYLIB_PATH`. Requires
  shipping the `.so` alongside the binary.
- Static linking via `copy-dylibs` / `ORT_LIB_LOCATION` — pre-stage `libonnxruntime`
  during build, statically include into the binary.

**Plan:** static linking. Download ORT release as a CI/Vercel build step (tiny `build.rs`
or direct CI script), point `ort` at the pre-staged libs, let `cargo build` statically
include what it needs. Output: a single binary with onnxruntime baked in. No runtime
`.so` loading, no extra files in the Vercel bundle beyond the binary.

**Contingency:** if static linking with `ort` v2.0.0-rc.12 turns out to be broken or
undocumented, fall back to `load-dynamic` + `include_bytes!` the `.so`, write it to
`/tmp/libonnxruntime.so` on first use, and point `ORT_DYLIB_PATH` at it. Hacky but
guaranteed to work.

Both paths are proven in the session 2 prototype before committing to the full rewrite.
This is the highest-risk unknown in the design.

### Binary size budget

| Component | Size |
|---|---|
| Stripped Rust binary (Axum + sqlx + auth + mcp) | ~12 MB |
| `onnxruntime` (statically linked) | ~20 MB |
| `bge-base-en-v1.5` quantized ONNX | ~45 MB |
| Tokenizer + configs | ~2 MB |
| **Total** | **~79 MB** |
| Vercel function limit (uncompressed) | **250 MB** |
| Headroom | ~171 MB |

Comfortable. Even if unexpected bloat pushes the binary to 120 MB, we are still at less
than half the limit. Fp32 model fallback (if quantized accuracy ever regresses) adds
~85 MB and still clears by 45 MB.

### Cargo.toml changes

Root `Cargo.toml` (workspace binary for the Vercel function):

```toml
temper-api    = { path = "crates/temper-api", features = ["ingest-pipeline"] }
temper-mcp    = { path = "crates/temper-mcp" }
temper-ingest = { path = "crates/temper-ingest", default-features = false, features = ["embed"] }
```

`crates/temper-ingest/Cargo.toml`:

- Remove `hf-hub` from `[dependencies]`.
- `embed` feature unchanged in what it gates (`ort`, `tokenizers`, `ndarray`).

`crates/temper-api/Cargo.toml`:

- New optional dep: `temper-ingest = { path = "../temper-ingest", default-features = false, features = ["embed"], optional = true }`.
- New feature flag `ingest-pipeline` that enables the dep and exposes the markdown-path
  code in `ingest_service`. Off by default, on when built as part of the temper-cloud binary.

This keeps `temper-api` lean as a library (tests, tooling, etc.) and only pulls in
`temper-ingest` when the binary asks for it.

## Testing

Three tiers, one new CI job, fixtures reused from the TS side.

### Tier 1 — Service-layer tests

`crates/temper-api/src/services/ingest_service.rs` under `#[cfg(test)]`. Direct function
calls against a real Docker Postgres with the real embedding model loaded. Fast iteration,
no HTTP or MCP layer.

Test cases:

1. `create_resource_from_markdown_round_trip` — `chunks_packed: None`, assert returned
   `EnrichedResource` has non-empty chunks in DB, `content_hash` matches sha256, event
   emitted.
2. `create_resource_from_markdown_validates_managed_meta_before_embed` — task with
   missing `temper-stage`. Assert `IngestError::Validation`, asserts zero chunks and
   zero resource rows written.
3. `create_resource_from_precomputed_chunks_unchanged` — CLI path regression guard.
4. `create_resource_dispatches_on_chunks_packed_presence` — parameterized; asserts
   `prepare_markdown` only called when `chunks_packed.is_none()`.
5. `update_resource_from_markdown_replaces_chunks_atomically` — create, then update with
   new content; assert old chunks gone and new chunks present in a single transaction.
6. `validate_managed_meta_strips_tier1_fields_silently` — pass `temper-id` etc.; assert
   stripped before validation, no error.
7. `validate_managed_meta_rejects_tier2_fields_in_update` — pass `temper-context` in an
   update. Assert `StructuralMoveNotSupported` with clear message.

### Tier 2 — E2E MCP round-trip tests

`tests/e2e/tests/mcp_round_trip_test.rs` (new file). Spawns the MCP service in-process
against a real Docker Postgres, uses an in-memory rmcp client. Reuses existing e2e
fixtures for db setup, auth token minting, and context seeding.

Test cases:

1. `mcp_create_resource_with_markdown_is_searchable` — `list_doc_types` → `describe_doc_type("concept")`
   → `create_resource` with its `example_managed_meta` + markdown body → `search` finds it.
2. `mcp_create_resource_schema_validation_surfaces_structured_error` — task missing
   required fields. Assert `CallToolResult::Error` with `ValidationErrorPayload`.
3. `mcp_describe_doc_type_returns_usable_example` — for each of 7 doc types, use
   `example_managed_meta` directly in `create_resource`. Assert success. Guards against
   example drift.
4. `mcp_list_doc_types_includes_required_fields` — assert each summary has correct
   `required_fields`.
5. `mcp_update_resource_changes_content_and_reindexes` — create, then update, then
   search; assert old content no longer found and new content found.
6. `mcp_create_resource_without_content_creates_metadata_shell` — legacy two-step path
   regression guard.

### Tier 3 — Deploy-time smoke test

Manual. After the session 3 PR lands, hit a Vercel preview URL with a curl POST to
`/api/ingest` carrying raw markdown and verify cold-start latency and round-trip
correctness. Documented as a pre-merge verification step. Not in CI.

### CI

New job `test-rust-embed` in `.github/workflows/test-rust.yml`:

```yaml
test-rust-embed:
  runs-on: ubuntu-latest
  needs: [code-quality]
  services:
    postgres:
      # ... same setup as existing test-rust job
  steps:
    - uses: actions/checkout@v4
      with:
        lfs: true
    - uses: dtolnay/rust-toolchain@stable
    - uses: Swatinem/rust-cache@v2
    - name: Run round-trip tests
      run: |
        cargo nextest run -p temper-api \
          --features "test-db,ingest-pipeline" \
          --test-threads 1 \
          round_trip
        cargo nextest run -p temper-e2e \
          --features "test-db,ingest-pipeline" \
          mcp_round_trip
      env:
        DATABASE_URL: postgresql://postgres:postgres@localhost:5432/temper_test
        SQLX_OFFLINE: "false"
```

Picked up by the `ci-success.yml` merge gate via a one-line edit. Existing `test-rust`
job unchanged — still excludes `embed`, still fast.

`--test-threads 1` because the model's `OnceLock` is process-global and parallel tests
would serialize anyway. Single-threaded path also keeps timing assertions stable.

### Fixtures

Reused from existing TS tests at `packages/temper-cloud/tests/fixtures/`:

- `simple.md` — existing baseline.

New fixtures added in the same directory (shared by Rust and TS tests):

- `task.md` — valid `task` frontmatter.
- `session.md` — valid `session` frontmatter.
- `concept.md` — valid `concept` frontmatter.
- `task-invalid.md` — `task` missing `temper-stage`, used by validation tests.

Rust tests reference them via:

```rust
const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../packages/temper-cloud/tests/fixtures"
);
```

## TypeScript Retirement

Deleted in session 3:

- `api/content-ingest.ts`
- `api/workflows/process-content-ingest.ts`
- `packages/temper-cloud/src/processing/embed.ts`
- `packages/temper-cloud/src/workflow/chunk.ts`
- `packages/temper-cloud/src/workflow/store.ts` — review first; if it's content-ingest-specific, delete; if shared with upload path, extract what upload needs.
- TS tests covering the above files.
- `ContentIngestRequest` removal from the ts-rs export in `crates/temper-core/src/types/ingest.rs`.

**Not deleted** (explicitly kept):

- `api/upload.ts` — Vercel Blob multipart upload path for binary files (PDF, DOCX, etc.).
- `api/workflows/process-upload.ts` — extraction + embedding workflow for uploaded binaries. Still uses TS-side embedding since binary extraction (kreuzberg) lives TS-side. This is a separate use case from markdown content creation and is out of scope.

## Agent-Skills Updates

Revise content in `agent-skills/` to reflect the unified single-request path:

- `agent-skills/knowledge-base.md` — remove the "create shell then upload" workflow
  documentation. Replace with the unified `create_resource` usage including
  `managed_meta` and schema discovery via `describe_doc_type`.
- `agent-skills/claude-desktop.md` — update content creation workflow to use the single
  MCP call.
- `agent-skills/SKILL.md` — verify accuracy after above updates.

The earlier task `2026-04-06-temper-mcp-content-creation-agent-workflow` proposed the
two-step shell-and-upload workflow; this task supersedes it and the agent-skills updates
close the loop.

## Scope Boundaries

### In scope

- Shared `prepare_markdown` function in `temper-ingest`.
- `IngestPayload` optionality changes in `temper-core`.
- Service-layer markdown-path branch with validation and atomicity guarantee.
- ONNX model bundling via `include_bytes!` with git-LFS.
- `ort` static linking into the Vercel binary.
- `ingest-pipeline` feature flag on `temper-api`.
- MCP tool changes: `list_doc_types` extension, new `describe_doc_type`, `create_resource`
  + `update_resource` new fields, direct service calls.
- TypeScript content-ingest proxy retirement.
- Tier 1 + Tier 2 tests.
- New `test-rust-embed` CI job.
- Shared fixtures in `packages/temper-cloud/tests/fixtures/`.
- Agent-skills documentation updates.

### Deferred

| Item | Rationale | Follow-up |
|---|---|---|
| Structural moves via MCP (`context_to`, `doc_type_to`, `slug_to`) | Requires lifting CLI's move logic out of `commands/resource.rs` into the service layer first. Separate review surface. | Dedicated follow-up task. |
| Retirement of `/api/upload` (Vercel Blob) | Different use case (binary extraction via kreuzberg). Cheap to keep. | Indefinite. |
| Blob-temp-store warm model cache | Only matters if cold-start latency proves problematic in production. `OnceLock` + `include_bytes!` gets most of the win. | Conditional on production feedback. |
| Eager model warm-up at boot | Slows every cold boot including ones that don't need embedding. | None. |
| Retries on transient embed/db failures inside the service | Fail-fast design; retries belong on the client. | None. |
| Partial-success semantics | Reintroduces the polling/status problem. | None. |
| Local HNSW / find-fast offline mode | Upstream task. CLI's client-side embedding preserved specifically for this. | Separate task already planned. |
| Vercel Function cold-start latency in CI | Cannot realistically measure from GitHub Actions. Manual smoke test post-deploy. | None. |
| Fp32 model fallback | Quantized int8 has negligible retrieval quality loss. | Conditional on evidence of regression. |

## Session Split

**Session 1 — Brainstorm + design + plan (this session)**

- Phase 2a verification spike (complete).
- Phase 2b brainstorm (complete).
- Write this spec.
- Commit spec, save to vault as research doc.
- Hand off to `writing-plans` for implementation plan.
- No code changes.

**Session 2 — Pipeline + Rust side**

- Prototype `ort` static linking against the Vercel build env. Resolve the linking-mode
  question early; fall back to `load-dynamic` + `include_bytes!` the `.so` only if
  static fails.
- Verify `create_resource_with_manifest` atomicity.
- Bundle model via `include_bytes!`, set up git-LFS, verify `cargo build --release`
  succeeds and binary size is within budget.
- Extract `prepare_markdown` into `temper_ingest::pipeline`, refactor CLI's
  `build_ingest_payload` to use it.
- Make `IngestPayload` fields optional, delete `ContentIngestRequest`.
- Add service-layer branch with schema validation.
- Tier 1 service-layer tests passing.
- Vercel preview deploy, cold-start smoke test via curl.
- Merge criteria (if session 2 becomes its own PR): new Rust path works, CLI path
  unchanged, TS content-ingest still alive as backup, no user-visible behavior change.

**Session 3 — MCP wiring, TS retirement, tests**

- MCP tool changes (`list_doc_types`, `describe_doc_type`, `create_resource`,
  `update_resource`).
- Kill `spawn_content_ingest_post`; MCP tools call `ingest_service` directly.
- Delete TS content-ingest files, workflow, embedder, chunker, tests.
- Tier 2 e2e MCP round-trip tests.
- Add `test-rust-embed` CI job.
- Update `agent-skills/` docs.
- Final deploy smoke test.
- Merge criteria: full pivot complete, all tests green, docs updated, TS proxy path gone.

## Risks and Open Items

Surfaced so the implementation plan can front-load them:

1. **`ort` static linking feasibility on Vercel's Rust build env.** Highest-risk unknown.
   Prototype in session 2 before committing to the full rewrite. Contingency:
   `load-dynamic` + bundled `.so`.
2. **Vercel git-LFS smoke test with the 45 MB `.onnx` file in a preview deploy.** LFS is
   enabled; verify with a throwaway commit before relying on it in the real design.
3. **`create_resource_with_manifest` atomicity.** Believed to be one SQL function; first
   plan step confirms.
4. **`tokenizers::Tokenizer::from_bytes` + `ort::Session::commit_from_memory` API surface
   on exact crate versions.** Both exist per docs; versions matter. Verified in session 2.
5. **CLI's `build_ingest_payload` refactor.** Small, but touches a hot path. Tier 1 tests
   guard against regression.

## Success Criteria

The task is done when:

- An MCP client can call `create_resource` with `content` + valid `managed_meta` and
  receive back a fully processed resource whose chunks are immediately findable via
  `search`, in a single request-response.
- The same call with invalid `managed_meta` returns a structured validation error with
  field-level issues, without writing to the DB or running the embed model.
- `describe_doc_type` returns a usable `example_managed_meta` that passes validation
  when fed back into `create_resource`.
- The CLI `temper resource create` command still works exactly as before, using its
  local `temper-ingest` pipeline.
- `api/content-ingest.ts` and the TS content-ingest workflow are deleted, with no
  references remaining in the codebase.
- Cold-start latency on a Vercel preview deploy for a ~20-chunk markdown POST is under
  3 seconds.
- `cargo make test-all-rust` passes with the `ingest-pipeline` feature enabled on the
  new CI job.
