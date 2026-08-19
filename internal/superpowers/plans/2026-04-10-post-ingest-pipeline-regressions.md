# Post-Ingest-Pipeline Regressions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fix three regressions introduced by the unified Rust ingest pipeline: heading-marker stripping on reconstitution, managed-meta defaults divergence between CLI and API/MCP, and ONNX model loading failure in CLI.

**Architecture:** Three independent bug fixes that share a release window but touch different subsystems. Session 1 (this plan) is the ONNX feature-flag split since it unblocks all sync push operations. Session 2 tackles reconstitution (migration + chunk schema + reconstituter). Session 3 tackles the meta-defaults divergence (API defaults + sync pull frontmatter reconstruction). Each session produces a working, testable fix.

**Tech Stack:** Rust (temper-ingest, temper-core, temper-api, temper-cli), PostgreSQL migrations, cargo-make, cargo-nextest

---

## File Structure

### Session 1: ONNX Feature-Flag Split

| File | Action | Responsibility |
|------|--------|----------------|
| `crates/temper-ingest/Cargo.toml` | Modify | Add `embed-download` feature with hf-hub dep |
| `crates/temper-ingest/src/embed.rs` | Modify | Conditional compilation: `embed` = bundled bytes, `embed-download` = hf-hub lazy download |
| `crates/temper-cli/Cargo.toml` | Modify | Switch from `embed` to `embed-download` feature |
| `crates/temper-ingest/src/error.rs` | Modify (if needed) | Ensure error types cover download failures |

### Session 2: Reconstitution Heading-Marker Fix

| File | Action | Responsibility |
|------|--------|----------------|
| `migrations/20260411000001_add_heading_depth_to_chunks.sql` | Create | Add `heading_depth` column to `kb_chunks` |
| `crates/temper-ingest/src/chunk.rs` | Modify | Store heading depth in `ChunkData` |
| `crates/temper-core/src/types/ingest.rs` | Modify | Add `heading_depth` to `PackedChunk` |
| `crates/temper-ingest/src/pipeline.rs` | Modify | Pass depth through to `PackedChunk` |
| `crates/temper-api/src/services/resource_service.rs` | Modify | Rebuild markdown headings from depth in `get_content()` |
| `crates/temper-api/src/services/ingest_service.rs` | Modify | Persist `heading_depth` when writing chunks |

### Session 3: Managed-Meta Defaults + Sync Pull Frontmatter

| File | Action | Responsibility |
|------|--------|----------------|
| `crates/temper-api/src/services/ingest_service.rs` | Modify | Set doc-type-specific defaults for missing managed_meta fields on create |
| `crates/temper-core/src/types/resource.rs` | Modify | Add `managed_meta: Option<serde_json::Value>` to `ContentResponse` |
| `crates/temper-api/src/services/resource_service.rs` | Modify | Include managed_meta in content response |
| `crates/temper-cli/src/actions/ingest.rs` | Modify | Rewrite `build_frontmatter()` to accept and render managed_meta fields |
| `crates/temper-cli/src/actions/sync.rs` | Modify | Pass managed_meta from content response through to frontmatter builder |
| `crates/temper-client/src/resources.rs` | Modify (if needed) | Deserialize new `managed_meta` field from content endpoint |

---

## Session 1: ONNX Feature-Flag Split

### Task 1: Add `embed-download` feature to temper-ingest

**Context:** The `embed` feature currently uses `include_bytes!()` to bundle the ONNX model at compile time. When git-lfs is not fully checked out (common on dev machines), `include_bytes!()` reads the 134-byte LFS pointer instead of the 110MB model. The CLI needs a runtime-download path via hf-hub. The API/MCP deployment keeps using the bundled path.

**Files:**
- Modify: `crates/temper-ingest/Cargo.toml`
- Modify: `crates/temper-ingest/src/embed.rs`
- Modify: `crates/temper-cli/Cargo.toml`

- [ ] **Step 1: Write a failing test for the download model path**

Create a test that verifies the download-based model initialization compiles and the model struct is accessible. This test will only run with the `embed-download` feature.

In `crates/temper-ingest/src/embed.rs`, add at the bottom:

```rust
#[cfg(test)]
mod tests {
    #[test]
    #[cfg(feature = "embed-download")]
    fn download_model_loads_successfully() {
        // This test requires network access and onnxruntime installed.
        // It verifies the hf-hub download path produces a working model.
        let result = super::load_model();
        assert!(result.is_ok(), "model should load via hf-hub download: {:?}", result.err());
    }

    #[test]
    #[cfg(feature = "embed")]
    fn bundled_model_bytes_are_not_lfs_pointer() {
        // Guard: if MODEL_BYTES starts with "version https://git-lfs" it's a pointer.
        let header = &super::MODEL_BYTES[..std::cmp::min(30, super::MODEL_BYTES.len())];
        let header_str = String::from_utf8_lossy(header);
        assert!(
            !header_str.starts_with("version https://git-lfs"),
            "MODEL_BYTES contains a git-lfs pointer, not the actual model binary. \
             Run `git lfs pull` or use the `embed-download` feature instead."
        );
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo nextest run -p temper-ingest --features embed test_bundled_model_bytes_are_not_lfs_pointer
```

Expected: FAIL — the current bytes ARE a git-lfs pointer (unless LFS is checked out locally).

Note: If LFS is checked out on this machine, the test will pass. That's fine — the test is a compile-time guard for CI and other dev machines. The real fix is in the next steps.

- [ ] **Step 3: Add `embed-download` feature and hf-hub dependency to Cargo.toml**

In `crates/temper-ingest/Cargo.toml`:

```toml
[dependencies]
# ... existing deps ...
hf-hub = { version = "0.4", optional = true }

[features]
default = ["extract", "embed"]
extract = ["dep:kreuzberg"]
embed = ["dep:ort", "dep:tokenizers", "dep:ndarray"]
embed-download = ["dep:ort", "dep:tokenizers", "dep:ndarray", "dep:hf-hub"]
test-embed = ["embed"]
```

Key: `embed` and `embed-download` are mutually exclusive model-loading strategies sharing the same ORT/tokenizer deps. `embed-download` adds `hf-hub` for runtime model fetching.

- [ ] **Step 4: Split embed.rs into conditional compilation blocks**

The model loading in `crates/temper-ingest/src/embed.rs` needs two paths:

**Bundled path (feature = "embed", not "embed-download"):**
Keep existing `include_bytes!()` + `commit_from_memory()` — unchanged.

**Download path (feature = "embed-download"):**
Use hf-hub to download the model at runtime, then load from file.

Replace the current static declarations and `load_model()` with:

```rust
// ---- Bundled model bytes (compile-time inclusion) ----

#[cfg(all(feature = "embed", not(feature = "embed-download")))]
static MODEL_BYTES: &[u8] = include_bytes!("../models/bge-base-en-v1.5/model_quantized.onnx");

// Tokenizer is small enough to always bundle (695 KB)
static TOKENIZER_BYTES: &[u8] = include_bytes!("../models/bge-base-en-v1.5/tokenizer.json");

// ... (ORT_LIB_BYTES and init_ort_runtime unchanged) ...

// ---- Model management ----

struct Model {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
}

static MODEL: OnceLock<std::result::Result<Model, String>> = OnceLock::new();

/// Load model from bundled bytes (embed feature, no download).
#[cfg(all(feature = "embed", not(feature = "embed-download")))]
fn load_model() -> Result<&'static Model> {
    let result = MODEL.get_or_init(|| {
        init_ort_runtime().map_err(|e| format!("ort runtime init: {e}"))?;

        let session = Session::builder()
            .map_err(|e| format!("ort session builder: {e}"))?
            .with_intra_threads(1)
            .map_err(|e| format!("ort threads: {e}"))?
            .commit_from_memory(MODEL_BYTES)
            .map_err(|e| format!("ort load: {e}"))?;

        let tokenizer =
            Tokenizer::from_bytes(TOKENIZER_BYTES).map_err(|e| format!("load tokenizer: {e}"))?;

        Ok(Model {
            session: Mutex::new(session),
            tokenizer,
        })
    });

    match result {
        Ok(m) => Ok(m),
        Err(e) => Err(EmbedError::Embedding(format!("model init: {e}"))),
    }
}

/// Load model via hf-hub download (embed-download feature).
#[cfg(feature = "embed-download")]
fn load_model() -> Result<&'static Model> {
    let result = MODEL.get_or_init(|| {
        init_ort_runtime().map_err(|e| format!("ort runtime init: {e}"))?;

        let api = hf_hub::api::sync::Api::new()
            .map_err(|e| format!("hf-hub init: {e}"))?;
        let repo = api.model("BAAI/bge-base-en-v1.5".to_string());

        let model_path = repo
            .get("onnx/model_quantized.onnx")
            .map_err(|e| format!("download model: {e}"))?;

        let session = Session::builder()
            .map_err(|e| format!("ort session builder: {e}"))?
            .with_intra_threads(1)
            .map_err(|e| format!("ort threads: {e}"))?
            .commit_from_file(&model_path)
            .map_err(|e| format!("ort load: {e}"))?;

        let tokenizer =
            Tokenizer::from_bytes(TOKENIZER_BYTES).map_err(|e| format!("load tokenizer: {e}"))?;

        Ok(Model {
            session: Mutex::new(session),
            tokenizer,
        })
    });

    match result {
        Ok(m) => Ok(m),
        Err(e) => Err(EmbedError::Embedding(format!("model init: {e}"))),
    }
}
```

The rest of embed.rs (tokenization, inference, mean pooling, normalization) is unchanged — it calls `load_model()` regardless of which feature is active.

- [ ] **Step 5: Switch temper-cli to use `embed-download`**

In `crates/temper-cli/Cargo.toml`, change the feature forwarding:

```toml
[features]
default = ["extract", "embed"]
embed = ["temper-ingest/embed-download"]
extract = ["temper-ingest/extract"]
```

Note: the CLI's `embed` feature now maps to `temper-ingest/embed-download` (the hf-hub path), not `temper-ingest/embed` (the bundled path). This way CLI users don't need to change their build commands.

- [ ] **Step 6: Verify compilation for both feature paths**

```bash
# CLI path (embed-download):
cargo build -p temper-cli --features embed

# API path (bundled embed — only works with LFS checked out):
# This may fail on machines without LFS, which is expected.
# The important thing is that the CLI path compiles.
cargo build -p temper-ingest --features embed-download --no-default-features

# Full workspace check:
cargo make check
```

- [ ] **Step 7: Run the LFS pointer guard test**

```bash
cargo nextest run -p temper-ingest --features embed bundled_model_bytes_are_not_lfs_pointer
```

If this fails, it confirms the guard works. If it passes, LFS is checked out locally.

- [ ] **Step 8: Test CLI sync push with the download path**

```bash
# Build CLI with new feature path:
cargo build -p temper-cli

# Attempt a sync that was previously failing:
temper sync run
```

Expected: push operations should now succeed (model downloads from HF on first use, cached thereafter).

- [ ] **Step 9: Commit**

```bash
git add crates/temper-ingest/Cargo.toml crates/temper-ingest/src/embed.rs crates/temper-cli/Cargo.toml
git commit -m "fix: split ONNX model loading — CLI uses hf-hub download, API uses bundled bytes

The unified ingest pipeline bundled the ONNX model via include_bytes!(),
which breaks on machines where git-lfs returns a pointer file instead of
the actual binary. Split into two feature flags:

- embed: bundled bytes (API/MCP deployment with LFS checkout)
- embed-download: hf-hub lazy download (CLI, dev machines)

CLI's embed feature now forwards to embed-download so sync push works
without requiring git-lfs pull."
```

---

## Session 2: Reconstitution Heading-Marker Fix

### Task 2: Add `heading_depth` to chunk schema and data types

**Context:** The chunker in `temper-ingest/src/chunk.rs` extracts heading level from the regex match (`caps[1].len()`) but only stores a breadcrumb string. The reconstituter in `resource_service.rs` has no way to rebuild `##`-prefixed headings because depth is lost. Fix: persist depth through the entire pipeline.

**Files:**
- Create: `migrations/20260411000001_add_heading_depth_to_chunks.sql`
- Modify: `crates/temper-ingest/src/chunk.rs` (lines 66-74: `ChunkData` struct)
- Modify: `crates/temper-core/src/types/ingest.rs` (`PackedChunk` struct)
- Modify: `crates/temper-ingest/src/pipeline.rs` (lines 29-39: `prepare_markdown`)

- [ ] **Step 1: Write the migration**

Create `migrations/20260411000001_add_heading_depth_to_chunks.sql`:

```sql
-- Add heading_depth to kb_chunks so reconstitution can rebuild markdown headings.
-- depth 0 = no heading (preamble text), 1 = #, 2 = ##, etc.
ALTER TABLE kb_chunks ADD COLUMN heading_depth SMALLINT NOT NULL DEFAULT 0;
```

- [ ] **Step 2: Apply the migration locally**

```bash
cargo make docker-up
sqlx migrate run --database-url postgresql://temper:temper@localhost:5437/temper_development
```

- [ ] **Step 3: Add `heading_depth` to `ChunkData`**

In `crates/temper-ingest/src/chunk.rs`, modify the `ChunkData` struct:

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkData {
    pub chunk_index: u32,
    /// Heading breadcrumb trail, e.g. `"Design > API > Auth"`.
    pub header_path: String,
    /// Depth of the innermost heading: 0 = no heading, 1 = `#`, 2 = `##`, etc.
    pub heading_depth: u8,
    pub content: String,
    /// Lowercase hex SHA-256 of `content.trim()`.
    pub content_hash: String,
}
```

- [ ] **Step 4: Populate `heading_depth` in `collect_sections` and `emit_chunks`**

In `collect_sections`, the `RawSection` struct also needs depth:

```rust
struct RawSection {
    header_path: String,
    heading_depth: u8,
    lines: Vec<String>,
}
```

Update `flush` to pass depth, and in the heading match:

```rust
header_stack.push((level, text));
```

The depth for a section is the level of its innermost heading = `level` at the point of flush. Store it in `RawSection.heading_depth = level as u8`.

When emitting `ChunkData` from sections, carry the depth through:

```rust
ChunkData {
    chunk_index,
    header_path: section.header_path.clone(),
    heading_depth: section.heading_depth,
    content: chunk_text,
    content_hash: sha256_hex(chunk_text.trim()),
}
```

- [ ] **Step 5: Add `heading_depth` to `PackedChunk` in temper-core**

In `crates/temper-core/src/types/ingest.rs`, find the `PackedChunk` struct and add:

```rust
pub heading_depth: u8,
```

Update `pack_chunks()` and `unpack_chunks()` in the same file to include the new field.

- [ ] **Step 6: Pass depth through in `prepare_markdown`**

In `crates/temper-ingest/src/pipeline.rs`, update the chunk→packed mapping:

```rust
PackedChunk {
    chunk_index: chunk.chunk_index,
    header_path: chunk.header_path,
    heading_depth: chunk.heading_depth,
    content: chunk.content,
    content_hash: chunk.content_hash,
    embedding,
}
```

- [ ] **Step 7: Verify compilation**

```bash
cargo build --workspace
```

Fix any compilation errors from the new field propagating through. The ingest service will need to persist the new column — check `insert_chunks` or equivalent in `ingest_service.rs`.

- [ ] **Step 8: Commit the schema + type changes**

```bash
git add migrations/ crates/temper-ingest/src/chunk.rs crates/temper-core/src/types/ingest.rs crates/temper-ingest/src/pipeline.rs
git commit -m "feat: add heading_depth to chunk pipeline and database schema

Persists heading depth (1=#, 2=##, etc.) through ChunkData, PackedChunk,
and kb_chunks table. Previously only a breadcrumb string was stored,
making it impossible to reconstruct markdown headings on read."
```

### Task 3: Fix reconstitution in `get_content()` + round-trip test

**Context:** Now that heading_depth is available in the database, the reconstituter can rebuild proper markdown headings instead of using breadcrumb strings.

**Files:**
- Modify: `crates/temper-api/src/services/resource_service.rs` (lines 366-397)
- Modify or create: integration test for round-trip fidelity

- [ ] **Step 1: Write a round-trip integration test**

In the appropriate test file (likely `crates/temper-api/tests/` or a test module in `resource_service.rs`), add:

```rust
#[cfg(feature = "test-db")]
#[tokio::test]
async fn test_reconstitution_preserves_heading_markers() {
    let pool = test_pool().await;
    let profile_id = test_profile(&pool).await;

    let input_markdown = "\
## Decision

We chose option B.

### Rationale

It was simpler.

## Implementation

Code goes here.
";

    // Create resource via ingest
    let resource = create_test_resource(&pool, profile_id, input_markdown).await;

    // Read back via get_content
    let output = resource_service::get_content(&pool, profile_id, resource.id).await.unwrap();

    // Heading markers must survive the round-trip
    assert!(output.contains("## Decision"), "expected '## Decision', got: {output}");
    assert!(output.contains("### Rationale"), "expected '### Rationale', got: {output}");
    assert!(output.contains("## Implementation"), "expected '## Implementation', got: {output}");
}
```

- [ ] **Step 2: Run the test — expect failure**

```bash
cargo nextest run -p temper-api --features test-db test_reconstitution_preserves_heading_markers
```

Expected: FAIL — current `get_content()` returns breadcrumb form.

- [ ] **Step 3: Fix `get_content()` to rebuild headings from depth**

In `crates/temper-api/src/services/resource_service.rs`, update the `ContentChunk` struct to include `heading_depth` and modify the reconstitution logic:

```rust
struct ContentChunk {
    chunk_index: i32,
    header_path: String,
    heading_depth: i16,
    content: String,
}

pub async fn get_content(pool: &PgPool, profile_id: Uuid, resource_id: Uuid) -> ApiResult<String> {
    get_visible(pool, profile_id, resource_id).await?;

    let chunks = sqlx::query_as!(
        ContentChunk,
        r#"
        SELECT chunk_index as "chunk_index!: i32",
               header_path as "header_path!: String",
               heading_depth as "heading_depth!: i16",
               content as "content!: String"
          FROM kb_current_chunks
         WHERE resource_id = $1
         ORDER BY chunk_index
        "#,
        resource_id,
    )
    .fetch_all(pool)
    .await?;

    let markdown = chunks
        .into_iter()
        .map(|c| {
            if c.heading_depth == 0 || c.header_path.is_empty() {
                c.content
            } else {
                // Extract the innermost heading title from the breadcrumb
                let title = c.header_path.rsplit(" > ").next().unwrap_or(&c.header_path);
                let hashes = "#".repeat(c.heading_depth as usize);
                format!("{hashes} {title}\n\n{}", c.content)
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    Ok(markdown)
}
```

- [ ] **Step 4: Update the `kb_current_chunks` view if needed**

Check whether the view includes `heading_depth`. If not, update the view definition in a migration or alter it:

```sql
-- May need to recreate the view to include heading_depth
CREATE OR REPLACE VIEW kb_current_chunks AS
SELECT id, resource_id, chunk_index, header_path, heading_depth, content, embedding
  FROM kb_chunks
 WHERE is_current = true;
```

- [ ] **Step 5: Run the test — expect pass**

```bash
cargo nextest run -p temper-api --features test-db test_reconstitution_preserves_heading_markers
```

- [ ] **Step 6: Regenerate sqlx cache and run full check**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make check
cargo make test-db
```

- [ ] **Step 7: Commit**

```bash
git add crates/temper-api/src/services/resource_service.rs migrations/ .sqlx/
git commit -m "fix: reconstitute markdown headings from stored depth instead of breadcrumbs

get_content() now rebuilds ## headings using the heading_depth column
instead of prepending the breadcrumb path as plain text. Adds a
round-trip fidelity test to prevent future regressions."
```

---

## Session 3: Managed-Meta Defaults + Sync Pull Frontmatter

### Task 4: API sets doc-type defaults on create

**Context:** When MCP/API creates a resource, `managed_meta` fields like `temper-stage` (for tasks) and `date` (for sessions) are not defaulted. The validation step injects placeholder values to pass schema checks but doesn't persist them. Fix: the ingest service should set doc-type-specific defaults for missing fields before persisting.

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs`

- [ ] **Step 1: Write a test for default managed_meta on task creation**

```rust
#[cfg(feature = "test-db")]
#[tokio::test]
async fn test_ingest_sets_default_stage_for_tasks() {
    let pool = test_pool().await;
    let profile_id = test_profile(&pool).await;

    let payload = IngestPayload {
        title: "Test task".to_owned(),
        slug: "test-task".to_owned(),
        context_name: "default".to_owned(),
        doc_type_name: "task".to_owned(),
        origin_uri: "kb://test/task/test-task".to_owned(),
        content: "Task content".to_owned(),
        managed_meta: None, // No managed_meta provided
        ..Default::default()
    };

    let resource = ingest(&pool, profile_id, "test-device", payload).await.unwrap();

    // Check that managed_meta was populated with defaults
    let manifest = get_manifest(&pool, resource.id).await;
    let stage = manifest.managed_meta.get("temper-stage")
        .and_then(|v| v.as_str());
    assert_eq!(stage, Some("backlog"), "tasks should default to backlog stage");
}
```

- [ ] **Step 2: Run — expect failure**

```bash
cargo nextest run -p temper-api --features test-db test_ingest_sets_default_stage
```

- [ ] **Step 3: Add `apply_doc_type_defaults()` function**

In `ingest_service.rs`, add a function that fills in missing doc-type-specific defaults:

```rust
/// Apply doc-type-specific defaults to managed_meta before persisting.
/// Only sets fields that are absent — never overwrites caller-provided values.
fn apply_doc_type_defaults(doc_type: &str, meta: &mut serde_json::Value) {
    let obj = match meta.as_object_mut() {
        Some(o) => o,
        None => return,
    };

    let now_date = chrono::Utc::now().format("%Y-%m-%d").to_string();

    match doc_type {
        "task" => {
            obj.entry("temper-stage").or_insert_with(|| json!("backlog"));
        }
        "goal" => {
            obj.entry("temper-status").or_insert_with(|| json!("active"));
        }
        "session" => {
            obj.entry("date").or_insert_with(|| json!(now_date));
        }
        "research" => {
            obj.entry("date").or_insert_with(|| json!(now_date));
        }
        _ => {}
    }
}
```

Call it in `ingest()` after stripping tier-1 fields and before validation:

```rust
let mut managed = payload.managed_meta.take().map(strip_system_managed_fields)
    .unwrap_or_else(|| json!({}));
apply_doc_type_defaults(&payload.doc_type_name, &mut managed);
payload.managed_meta = Some(managed);
```

Do the same in `update()`.

- [ ] **Step 4: Run test — expect pass**

```bash
cargo nextest run -p temper-api --features test-db test_ingest_sets_default_stage
```

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/services/ingest_service.rs
git commit -m "fix: apply doc-type defaults to managed_meta on API create/update

Tasks default to stage=backlog, goals to status=active, sessions and
research to date=today. Only fills absent fields, never overwrites
caller-provided values."
```

### Task 5: Sync pull reconstructs complete frontmatter from server data

**Context:** `build_frontmatter()` only outputs `temper-id`, `temper-type`, `temper-context`, `temper-created`, and `title`. It ignores `managed_meta` fields like `temper-stage`, `slug`, `temper-owner`, `date`, `mode`, `effort`, etc. This means sync pull overwrites locally-fixed frontmatter with incomplete versions.

Two changes needed: (a) the content endpoint should return `managed_meta` so the CLI has the data, and (b) `build_frontmatter()` should render all managed_meta fields.

**Files:**
- Modify: `crates/temper-core/src/types/resource.rs` (`ContentResponse`)
- Modify: `crates/temper-api/src/services/resource_service.rs` (content endpoint)
- Modify: `crates/temper-api/src/handlers/resources.rs` (if the handler assembles the response)
- Modify: `crates/temper-cli/src/actions/ingest.rs` (`build_frontmatter`)
- Modify: `crates/temper-cli/src/actions/sync.rs` (`pull_resource`)

- [ ] **Step 1: Add `managed_meta` to `ContentResponse`**

In `crates/temper-core/src/types/resource.rs`:

```rust
pub struct ContentResponse {
    pub resource_id: ResourceId,
    pub markdown: String,
    pub managed_meta: Option<serde_json::Value>,
}
```

- [ ] **Step 2: Return managed_meta from the content service/handler**

In the service or handler that builds `ContentResponse`, fetch `managed_meta` from `kb_resource_manifests` and include it:

```rust
let manifest_meta = sqlx::query_scalar!(
    "SELECT managed_meta FROM kb_resource_manifests WHERE resource_id = $1",
    resource_id,
)
.fetch_optional(pool)
.await?;

ContentResponse {
    resource_id,
    markdown,
    managed_meta: manifest_meta,
}
```

- [ ] **Step 3: Rewrite `build_frontmatter()` to render managed_meta**

Replace the current `build_frontmatter()` in `crates/temper-cli/src/actions/ingest.rs`:

```rust
/// Generate YAML frontmatter for a vault file from server data.
///
/// Combines the resource-level fields (id, type, context, created, title)
/// with managed_meta fields (stage, mode, effort, etc.) and optional extras.
pub fn build_frontmatter_from_resource(
    resource: &temper_core::types::ResourceRow,
    context: &str,
    doc_type: &str,
    managed_meta: Option<&serde_json::Value>,
) -> String {
    let mut fm = format!(
        "---\ntemper-id: {}\ntemper-type: {doc_type}\ntemper-context: {context}\n\
         temper-created: {}\ntitle: \"{}\"\n",
        resource.id,
        resource.created.to_rfc3339(),
        resource.title,
    );

    // Slug from resource
    if let Some(ref slug) = resource.slug {
        fm.push_str(&format!("slug: \"{slug}\"\n"));
    }

    // Owner handle
    if !resource.owner_handle.is_empty() {
        fm.push_str(&format!("temper-owner: \"{}\"\n", resource.owner_handle));
    }

    // Managed meta fields (stage, mode, effort, date, goal, etc.)
    if let Some(meta) = managed_meta.and_then(|m| m.as_object()) {
        // Tier-1 and tier-2 fields already rendered above — skip them
        let skip = [
            "temper-id", "temper-type", "temper-context", "temper-created",
            "temper-updated", "temper-owner", "temper-provisional-id",
            "temper-source", "temper-legacy-id", "title", "slug",
        ];
        for (key, value) in meta {
            if skip.contains(&key.as_str()) {
                continue;
            }
            match value {
                serde_json::Value::String(s) => fm.push_str(&format!("{key}: \"{s}\"\n")),
                serde_json::Value::Number(n) => fm.push_str(&format!("{key}: {n}\n")),
                serde_json::Value::Bool(b) => fm.push_str(&format!("{key}: {b}\n")),
                serde_json::Value::Null => fm.push_str(&format!("{key}: null\n")),
                _ => fm.push_str(&format!("{key}: {value}\n")),
            }
        }
    }

    fm.push_str("---\n\n");
    fm
}
```

Keep the old `build_frontmatter()` for non-sync callers, or migrate them all to the new function.

- [ ] **Step 4: Update `pull_resource()` to use managed_meta**

In `crates/temper-cli/src/actions/sync.rs`, the pull path currently calls `build_frontmatter()` with just id/title/context/doc_type. Update to:

```rust
let frontmatter = ingest::build_frontmatter_from_resource(
    &resource,
    &ctx,
    &doc_type,
    content_response.managed_meta.as_ref(),
);
```

Apply this in both the "overwrite existing" and "write new" branches of `pull_resource()`.

- [ ] **Step 5: Update temper-client deserialization if needed**

Check that the client's `content()` method deserializes the new `managed_meta` field from the API response. Since `managed_meta` is `Option<Value>`, it should be backwards-compatible (absent = None).

- [ ] **Step 6: Run integration tests**

```bash
cargo make test-db
```

- [ ] **Step 7: Run full verification**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make check
cargo make test-all
```

- [ ] **Step 8: Commit**

```bash
git add crates/temper-core/src/types/resource.rs crates/temper-api/src/services/ crates/temper-cli/src/actions/ crates/temper-client/ .sqlx/
git commit -m "fix: sync pull reconstructs complete frontmatter from server managed_meta

ContentResponse now includes managed_meta from kb_resource_manifests.
build_frontmatter_from_resource() renders all managed fields (stage,
mode, effort, date, owner, etc.) instead of just the five core fields.
Fixes the doctor-fix-then-sync-breaks-it cycle where locally-repaired
frontmatter was overwritten with incomplete server data."
```

### Task 6: Validator error detail improvement

**Context:** All three failure paths (`structural move via field 'temper-context'`, `managed_meta validation failed for doc_type=session: 1 issues`, `managed_meta validation failed for doc_type=research: 1 issues`) return issue counts without field-level detail. The validator knows what failed; the HTTP layer discards it.

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs` (error variant)
- Modify: `crates/temper-core/src/schema.rs` or wherever `validate_frontmatter` returns issues

- [ ] **Step 1: Check the current error format**

Read the `IngestError::Validation` variant and how it's rendered. Currently it formats as `"{count} issues"` without listing the issues.

- [ ] **Step 2: Write a test that asserts field-level detail in validation errors**

```rust
#[test]
fn test_validation_error_includes_field_details() {
    let params = ValidateParams {
        doc_type: "task",
        managed_meta: Some(&json!({})), // Missing required temper-stage
        slug: "test",
        title: "test",
        context_name: "test",
    };

    let err = validate_managed_meta(&params).unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("temper-stage"), "error should name the failing field: {msg}");
}
```

- [ ] **Step 3: Update the error variant to include field-level issues**

Modify `IngestError::Validation` to include the issue details in its `Display` impl:

```rust
Validation { doc_type: String, issues: Vec<ValidationIssue> }
```

And its Display:

```rust
fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "managed_meta validation failed for doc_type={}: ", self.doc_type)?;
    for (i, issue) in self.issues.iter().enumerate() {
        if i > 0 { write!(f, "; ")?; }
        write!(f, "{issue}")?;
    }
    Ok(())
}
```

- [ ] **Step 4: Run test — expect pass**

```bash
cargo nextest run -p temper-api test_validation_error_includes_field_details
```

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/services/ingest_service.rs
git commit -m "fix: include field-level detail in managed_meta validation errors

Validation errors now list which fields failed instead of just a count.
Unblocks debugging for CLI sync, MCP create, and any validator-gated path."
```

---

## Session Order and Dependencies

```
Session 1: Task 1 (ONNX split)
  └── Unblocks all sync push operations immediately

Session 2: Tasks 2-3 (Reconstitution)
  └── Migration + type changes + reconstituter fix
  └── Independent of Session 1

Session 3: Tasks 4-6 (Meta defaults + sync pull + validator detail)
  └── Task 4 (API defaults) → Task 5 (sync pull) — sequential
  └── Task 6 (validator detail) — independent, can run in parallel with 4-5
```

Sessions 1, 2, and 3 are independent of each other and can be done in any order. Within Session 3, Task 5 depends on Task 4 (the API needs to set defaults before sync pull can read them back). Task 6 is independent.
