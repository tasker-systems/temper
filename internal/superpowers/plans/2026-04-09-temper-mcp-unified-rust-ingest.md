# Unified Rust Ingest Pipeline + MCP Round-Trip Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse the two-path ingest architecture into a single synchronous Rust request-response pipeline. Bundle the `BAAI/bge-base-en-v1.5` quantized ONNX model into the Vercel function binary, extend MCP tools with schema-aware inputs, and retire the TypeScript `/api/content-ingest` proxy.

**Architecture:** New `temper_ingest::pipeline::prepare_markdown` shared function. `IngestPayload.chunks_packed` and `content_hash` become optional; if absent, the service layer runs chunking + embedding inline via `temper-ingest`. MCP tools call `ingest_service::create_resource_with_manifest` directly (no HTTP detour). Schema validation via `temper_core::schema::validate_frontmatter` happens before any compute or DB work. Model weights bundled via `include_bytes!` with git-LFS.

**Tech Stack:** Rust (axum, sqlx, ort v2.0.0-rc.12, tokenizers, ndarray), rmcp for MCP, bge-base-en-v1.5 ONNX (int8 quantized), git-LFS, Vercel Functions via `vercel_runtime` v2.

**Spec reference:** `docs/superpowers/specs/2026-04-09-temper-mcp-unified-rust-ingest-design.md`

**Session split:** Tasks 0-21 are session 2 (Rust pipeline + backend). Tasks 22-44 are session 3 (MCP surface, TS retirement, e2e tests, CI, docs). Pause for review between sessions.

---

## Subagent Guidance (inject into every subagent prompt)

Any subagent dispatched while executing this plan MUST be given the following guidance verbatim in its prompt. Do not summarize or paraphrase.

### SG-1: Follow Existing Patterns
Before writing anything, read the file you're modifying AND a sibling in the same module. Match the style you find: naming, imports, structure, error handling. Don't invent new patterns.

### SG-2: Single Responsibility
Each function does one thing. If it constructs AND processes AND formats — split it. Follow the project's existing layering.

### SG-3: No Logic Duplication
Would two implementations drift independently over time? Extract. Otherwise leave inline. Don't create premature abstractions for one-time operations.

### SG-4: Test Strategy
Unit tests co-located with code. Integration tests separate. One behavior per test with descriptive names. Tests must actually run — verify, don't assume.

### SG-5: Don't Over-Build
Implement exactly what the task says. No speculative features, no defensive code for impossible cases, no "nice to have" extras.

### SG-6: Verify Before Claiming Done
Run the verification command. Read the output. Don't claim success based on what you think the code does.

### SG-7: Prefer Native Solutions
Don't invent when the framework, language, or platform provides. If a proper tool exists, use it over a hand-rolled alternative. The idiomatic solution is almost always better than the clever workaround.

### SG-8: Front-Load Constraints
Before proposing anything: (1) existing abstractions for this? (2) platform/deployment limits? (3) async/performance requirements? List findings before writing code.

### SG-9: Don't Dismiss Owned Failures
If the user owns both sides of an interaction, debug the full stack. Never declare "not our problem" without proving external causation.

### SG-10: Checkpoint Before Continuing
After each major step, report: what's done, what's next, any concerns about approach drift.

### Temper project fundamentals

The following code quality rules override defaults and apply to every task in this plan:

- **Typed structs over inline JSON.** Never use `serde_json::json!()` for data with a known structure. Define a struct.
- **Shared types at boundaries.** When Rust calls TypeScript, the wire type lives in `temper-core` with `ts-rs` derives. Both sides share the generated type.
- **Service layer owns SQL.** All SQL lives in `temper-api/src/services/`. MCP tools, CLI actions, and HTTP handlers call service functions.
- **Params structs.** Functions with more than 5 domain-related parameters get a params struct.
- **Auth before writes.** Authorization checks go before any mutations.
- **Profile scoping.** All data queries scope through `resources_visible_to`, `can_modify_resource`, or equivalent.
- **All public types must implement `Debug`.**
- **All MPSC channels must be bounded** (no `unbounded_channel()`).
- **`#[expect(lint_name, reason = "...")]` instead of `#[allow]`** for lint suppression.
- **Use `--all-features`** for builds and clippy.
- **Follow existing patterns** in the crate you're modifying before inventing new ones.

---

## File Structure

### Files being created

| Path | Responsibility |
|---|---|
| `crates/temper-ingest/models/bge-base-en-v1.5/model_quantized.onnx` | Int8 quantized ONNX model weights (LFS-tracked) |
| `crates/temper-ingest/models/bge-base-en-v1.5/tokenizer.json` | HuggingFace tokenizer for bge-base-en-v1.5 |
| `crates/temper-ingest/models/bge-base-en-v1.5/config.json` | Model config (hidden_size, max_position_embeddings, etc.) |
| `crates/temper-ingest/models/bge-base-en-v1.5/special_tokens_map.json` | Tokenizer special tokens |
| `crates/temper-ingest/models/bge-base-en-v1.5/tokenizer_config.json` | Tokenizer runtime config |
| `crates/temper-ingest/src/pipeline.rs` | Shared `prepare_markdown(&str) -> Vec<PackedChunk>` function |
| `tests/e2e/tests/mcp_round_trip_test.rs` | Tier 2 e2e MCP round-trip tests |
| `packages/temper-cloud/tests/fixtures/task.md` | Valid `task` frontmatter fixture for round-trip tests |
| `packages/temper-cloud/tests/fixtures/session.md` | Valid `session` frontmatter fixture |
| `packages/temper-cloud/tests/fixtures/concept.md` | Valid `concept` frontmatter fixture |
| `packages/temper-cloud/tests/fixtures/task-invalid.md` | Task missing `temper-stage` for validation failure tests |

### Files being modified

| Path | Change |
|---|---|
| `.gitattributes` | Add LFS rule for `crates/temper-ingest/models/**/*.onnx` |
| `crates/temper-ingest/Cargo.toml` | Remove `hf-hub`, add `ort` linking features, wire model bytes |
| `crates/temper-ingest/src/lib.rs` | Export new `pipeline` module |
| `crates/temper-ingest/src/embed.rs` | Load model/tokenizer from `include_bytes!` instead of HF Hub |
| `crates/temper-core/src/types/ingest.rs` | `chunks_packed` + `content_hash` optional, delete `ContentIngestRequest` |
| `crates/temper-cli/src/actions/ingest.rs` | `build_ingest_payload` uses `prepare_markdown` |
| `crates/temper-api/Cargo.toml` | Add `ingest-pipeline` feature gating `temper-ingest` optional dep |
| `crates/temper-api/src/services/ingest_service.rs` | New validation helper, markdown-path branch, error variants |
| `crates/temper-mcp/src/tools/doc_types.rs` | Extend `list_doc_types`, add `describe_doc_type` |
| `crates/temper-mcp/src/tools/resources.rs` | `CreateResourceInput`/`UpdateResourceInput` gain `managed_meta`+`open_meta`, kill `spawn_content_ingest_post` |
| `Cargo.toml` (root workspace) | Add `temper-ingest` dep, enable `ingest-pipeline` on `temper-api` |
| `.github/workflows/test-rust.yml` | New `test-rust-embed` job |
| `.github/workflows/ci-success.yml` | Add `test-rust-embed` to merge-gate needs |
| `agent-skills/knowledge-base.md` | Document unified single-request content creation |
| `agent-skills/claude-desktop.md` | Update content creation workflow |
| `agent-skills/SKILL.md` | Verify accuracy after updates |

### Files being deleted

| Path | Reason |
|---|---|
| `api/content-ingest.ts` | TS proxy-embed path retired |
| `api/workflows/process-content-ingest.ts` | TS workflow for content-ingest retired |
| `packages/temper-cloud/src/processing/embed.ts` | TS-side embedding no longer needed |
| `packages/temper-cloud/src/workflow/chunk.ts` | TS-side chunking no longer needed |
| `packages/temper-cloud/src/workflow/store.ts` | Conditionally — review first; delete if content-ingest-only, otherwise refactor |
| TS tests covering the above | Corresponding test files |

---

# Session 2 — Rust Pipeline and Backend

## Task 0: Verify `create_resource_with_manifest` atomicity

**Files:**
- Read: `crates/temper-api/src/services/ingest_service.rs`
- Read: `migrations/` (find the SQL function definition)

**Why:** The markdown-path branch must run inside an atomic boundary with the rest of the write. If the current implementation is already one SQL function call or one sqlx transaction, we do nothing. If not, we wrap it before proceeding. This is the first step because all later service-layer changes depend on the invariant.

- [ ] **Step 1: Read the current `create_resource_with_manifest` implementation**

```bash
grep -n "fn create_resource_with_manifest" crates/temper-api/src/services/ingest_service.rs
```

Read the full function body and any SQL it calls.

- [ ] **Step 2: Identify the write operations**

Note which of these happen in the function:
- Insert/upsert into `kb_resources`
- Insert/upsert into `kb_manifests`
- Call `persist_resource_chunks` or `replace_resource_chunks` SQL function
- Insert into `kb_events`

For each, note whether it's inside a `sqlx::Transaction` or a single SQL function call.

- [ ] **Step 3: If already atomic, document and continue**

If the operations are all wrapped in one transaction or one SQL function call, add a rustdoc comment above `create_resource_with_manifest` stating the atomicity guarantee explicitly:

```rust
/// Atomicity: this function performs all writes (resource, manifest, chunks,
/// event) inside a single transaction / SQL function call. A mid-call failure
/// leaves no partial state.
pub async fn create_resource_with_manifest(/* ... */) { /* ... */ }
```

Commit the doc comment only.

- [ ] **Step 4: If NOT atomic, wrap in a transaction**

Begin a `sqlx::Transaction`, move all writes inside, commit at the end, and propagate errors. Use the existing error handling pattern in the file — do not invent a new one.

Example sketch (adapt to the actual call shape):

```rust
let mut tx = pool.begin().await.map_err(IngestError::Db)?;
// ... all writes use &mut *tx instead of &pool ...
tx.commit().await.map_err(IngestError::Db)?;
```

- [ ] **Step 5: Run existing ingest tests**

```bash
cargo nextest run -p temper-api --features test-db create_resource_with_manifest
```

Expected: tests pass (existing behavior unchanged).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/services/ingest_service.rs
git commit -m "feat(ingest): document or enforce create_resource_with_manifest atomicity"
```

---

## Task 1: Prototype `ort` static linking on a throwaway commit

**Files:**
- Modify: `crates/temper-ingest/Cargo.toml` (throwaway)
- Create: `crates/temper-ingest/build.rs` (throwaway)

**Why:** `ort` v2.0.0-rc.12 supports three linking modes. We need to prove static linking works on Vercel's Rust build environment before committing to the full rewrite. If static fails, we fall back to `load-dynamic` + bundled `.so`. This task produces a throwaway branch/commit that we push to a Vercel preview to verify deployability.

- [ ] **Step 1: Create a throwaway branch off `jct/mcp-bugs-and-round-trip`**

```bash
git checkout -b jct/ort-static-linking-spike
```

- [ ] **Step 2: Read `ort` v2 documentation for static linking**

Fetch the ort crate docs via Context7:

```
mcp__claude_ai_Context7__resolve-library-id query="ort rust onnxruntime"
mcp__claude_ai_Context7__query-docs library_id="<resolved id>" query="static linking ORT_LIB_LOCATION copy-dylibs"
```

Identify the exact feature flags and env vars required. Document in the branch commit message.

- [ ] **Step 3: Update `crates/temper-ingest/Cargo.toml` with the static-linking feature set found in Step 2**

Use the exact feature flags Context7 returned for static linking. The expected shape is approximately:

```toml
[dependencies]
ort = { version = "=2.0.0-rc.12", default-features = false, features = ["copy-dylibs", "std", "ndarray"] }
```

The goal: no `download-binaries`, libs sourced from `ORT_LIB_LOCATION` at build time. If the Step 2 docs reveal a different feature combination produces static linking, use that instead and record the chosen features in the spike commit message so Step 9 can document the decision.

- [ ] **Step 4: Add a minimal `build.rs` that sets `ORT_LIB_LOCATION`**

```rust
// crates/temper-ingest/build.rs
fn main() {
    // Point ort at a pre-downloaded onnxruntime release.
    // In CI we'll download this as a build step before cargo build.
    if let Ok(path) = std::env::var("ORT_LIB_LOCATION") {
        println!("cargo:rustc-env=ORT_LIB_LOCATION={path}");
    }
}
```

- [ ] **Step 5: Build locally**

```bash
ORT_LIB_LOCATION=/path/to/pre-downloaded/onnxruntime cargo build --release -p temper-ingest --features embed
```

Expected: compiles successfully with onnxruntime statically linked. If this fails, document the error, then move to the contingency in Step 7.

- [ ] **Step 6: Push throwaway branch to a Vercel preview**

```bash
git add crates/temper-ingest/Cargo.toml crates/temper-ingest/build.rs
git commit -m "spike: ort static linking prototype (DO NOT MERGE)"
git push -u origin jct/ort-static-linking-spike
```

Wait for the Vercel preview to build. Inspect the build log for errors related to ort linking.

**Decision point:**
- ✅ Preview builds successfully → static linking works; proceed to Task 3 with this configuration.
- ❌ Preview build fails → fall back to Step 7 (load-dynamic contingency).

- [ ] **Step 7 (contingency): Switch to `load-dynamic` mode**

Only execute if Step 6 fails.

```toml
[dependencies]
ort = { version = "=2.0.0-rc.12", default-features = false, features = ["load-dynamic", "std", "ndarray"] }
```

Plan to `include_bytes!` the `libonnxruntime.so` into the binary, write to `/tmp/libonnxruntime.so` on first use, and set `ORT_DYLIB_PATH`. Document this as the approach for Task 3.

- [ ] **Step 8: Clean up the spike branch**

```bash
git checkout jct/mcp-bugs-and-round-trip
git branch -D jct/ort-static-linking-spike  # local only; leave remote for reference
```

Do not merge the spike. It exists solely as a deploy-log artifact.

- [ ] **Step 9: Commit findings as a note in the task**

Create a short file `crates/temper-ingest/LINKING.md` documenting:
- Which linking mode won (static vs load-dynamic)
- Exact `ort` features used
- Any env vars or build-script requirements
- Link to the spike branch for future reference

```bash
git add crates/temper-ingest/LINKING.md
git commit -m "docs(ingest): record ort linking strategy decision"
```

---

## Task 2: Smoke-test git-LFS on a Vercel preview with a placeholder binary

**Files:**
- Create: `.gitattributes` (if not present) or update
- Create: `crates/temper-ingest/models/bge-base-en-v1.5/placeholder.onnx` (throwaway, deleted in Task 3)

**Why:** Git-LFS is enabled on Vercel for this project, but we haven't proven a real LFS-tracked file deploys correctly. Use a placeholder to verify the pipeline before committing the real 45 MB model.

- [ ] **Step 1: Configure git-LFS for `.onnx` files**

Create or update `.gitattributes`:

```
crates/temper-ingest/models/**/*.onnx filter=lfs diff=lfs merge=lfs -text
```

- [ ] **Step 2: Install git-lfs locally if not installed**

```bash
git lfs version || brew install git-lfs
git lfs install
```

- [ ] **Step 3: Create a placeholder file**

```bash
mkdir -p crates/temper-ingest/models/bge-base-en-v1.5
dd if=/dev/urandom of=crates/temper-ingest/models/bge-base-en-v1.5/placeholder.onnx bs=1M count=45
```

- [ ] **Step 4: Verify LFS tracks it**

```bash
git add .gitattributes crates/temper-ingest/models/bge-base-en-v1.5/placeholder.onnx
git status
```

Expected: git status shows `placeholder.onnx` as a new file, and `git lfs status` shows it as LFS-tracked.

```bash
git lfs status
```

Expected output contains `placeholder.onnx` under "Git LFS objects to be committed".

- [ ] **Step 5: Commit the placeholder**

```bash
git commit -m "spike: placeholder .onnx for LFS smoke test (will be replaced)"
```

- [ ] **Step 6: Push and verify Vercel preview builds**

```bash
git push
```

Wait for the Vercel preview deploy. Check the build log for LFS-related errors (e.g. "file not found", pointer file without contents, etc.).

**Decision point:**
- ✅ Preview builds and deploys → LFS pipeline works; proceed.
- ❌ LFS errors in build log → investigate whether Vercel needs additional LFS configuration beyond the project-settings toggle. Do not proceed to Task 3 until resolved.

- [ ] **Step 7: Remove the placeholder (will be replaced in Task 3)**

```bash
rm crates/temper-ingest/models/bge-base-en-v1.5/placeholder.onnx
git add -u crates/temper-ingest/models/bge-base-en-v1.5/placeholder.onnx
git commit -m "spike: remove LFS placeholder"
```

---

## Task 3: Download and commit the real bge-base-en-v1.5 quantized ONNX model

**Files:**
- Create: `crates/temper-ingest/models/bge-base-en-v1.5/model_quantized.onnx` (LFS)
- Create: `crates/temper-ingest/models/bge-base-en-v1.5/tokenizer.json`
- Create: `crates/temper-ingest/models/bge-base-en-v1.5/config.json`
- Create: `crates/temper-ingest/models/bge-base-en-v1.5/special_tokens_map.json`
- Create: `crates/temper-ingest/models/bge-base-en-v1.5/tokenizer_config.json`

- [ ] **Step 1: Download the quantized ONNX model from HuggingFace**

```bash
cd crates/temper-ingest/models/bge-base-en-v1.5
curl -LO https://huggingface.co/BAAI/bge-base-en-v1.5/resolve/main/onnx/model_quantized.onnx
curl -LO https://huggingface.co/BAAI/bge-base-en-v1.5/resolve/main/tokenizer.json
curl -LO https://huggingface.co/BAAI/bge-base-en-v1.5/resolve/main/config.json
curl -LO https://huggingface.co/BAAI/bge-base-en-v1.5/resolve/main/special_tokens_map.json
curl -LO https://huggingface.co/BAAI/bge-base-en-v1.5/resolve/main/tokenizer_config.json
cd ../../../..
```

- [ ] **Step 2: Verify file sizes**

```bash
ls -lh crates/temper-ingest/models/bge-base-en-v1.5/
```

Expected:
- `model_quantized.onnx` ~45 MB
- `tokenizer.json` ~2 MB
- other files <100 KB each

If `model_quantized.onnx` is significantly larger than 50 MB, verify we downloaded the quantized variant, not `model.onnx` (fp32).

- [ ] **Step 3: Verify LFS is tracking the .onnx file**

```bash
git add crates/temper-ingest/models/bge-base-en-v1.5/
git lfs status
```

Expected: `model_quantized.onnx` shown under "Git LFS objects to be committed".

- [ ] **Step 4: Commit**

```bash
git commit -m "feat(ingest): bundle bge-base-en-v1.5 quantized ONNX model and tokenizer"
```

- [ ] **Step 5: Push and verify Vercel deploy**

```bash
git push
```

Wait for Vercel preview. Confirm the deploy completes.

---

## Task 4: Refactor `temper-ingest::embed` to load model from `include_bytes!`

**Files:**
- Modify: `crates/temper-ingest/src/embed.rs`
- Modify: `crates/temper-ingest/Cargo.toml` (remove `hf-hub` dep)

**Why:** Replace runtime HuggingFace Hub downloads with compile-time bundled bytes. This is the core cold-start fix.

- [ ] **Step 1: Read current `embed.rs` to understand the `OnceLock<Model>` pattern**

```bash
cat crates/temper-ingest/src/embed.rs
```

Note: the current `load_model()` function, the `hf_hub::api::sync` calls, and the `OnceLock` structure. The refactor preserves the `OnceLock` — only the load body changes.

- [ ] **Step 2: Write a failing unit test**

Add to `crates/temper-ingest/src/embed.rs` at the bottom of the existing test module:

```rust
#[cfg(test)]
mod tests_bundled_model {
    use super::*;

    #[test]
    fn embeds_a_short_text_from_bundled_model() {
        let text = "hello world";
        let result = embed_text(text);
        assert!(result.is_ok(), "embed_text should succeed with bundled model: {result:?}");
        let vec = result.unwrap();
        assert_eq!(vec.len(), 768, "bge-base-en-v1.5 produces 768-dim embeddings");
        // Sanity check: embedding is L2-normalized (norm ~= 1.0)
        let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01, "embedding should be L2-normalized, got norm={norm}");
    }
}
```

- [ ] **Step 3: Run the test and verify it fails**

```bash
cargo nextest run -p temper-ingest --features embed embeds_a_short_text_from_bundled_model
```

Expected: FAIL, because current code tries to download from HF Hub which doesn't work without cache.

- [ ] **Step 4: Replace the `load_model` function body with bundled-bytes loading**

In `crates/temper-ingest/src/embed.rs`, replace the HF Hub code path with:

```rust
static MODEL_BYTES: &[u8] = include_bytes!("../models/bge-base-en-v1.5/model_quantized.onnx");
static TOKENIZER_BYTES: &[u8] = include_bytes!("../models/bge-base-en-v1.5/tokenizer.json");

fn load_model() -> Result<Model, EmbedError> {
    let tokenizer = tokenizers::Tokenizer::from_bytes(TOKENIZER_BYTES)
        .map_err(|e| EmbedError::Tokenizer(e.to_string()))?;
    let session = ort::session::Session::builder()
        .map_err(|e| EmbedError::Ort(e.to_string()))?
        .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
        .map_err(|e| EmbedError::Ort(e.to_string()))?
        .commit_from_memory(MODEL_BYTES)
        .map_err(|e| EmbedError::Ort(e.to_string()))?;
    Ok(Model { tokenizer, session })
}
```

**Important:** match the exact `Model` struct fields and `EmbedError` variants already defined in the file. Do not invent new variants; if the existing error type doesn't have `Tokenizer` / `Ort` variants, add them to the existing enum rather than creating a new type. Follow SG-1.

- [ ] **Step 5: Remove `hf_hub::api::sync` imports and the old download code**

Delete any lingering `use hf_hub::...` statements and any functions that were only used for HF Hub downloads.

- [ ] **Step 6: Remove `hf-hub` from `Cargo.toml`**

In `crates/temper-ingest/Cargo.toml`, delete the `hf-hub = "0.5"` line from `[dependencies]`.

- [ ] **Step 7: Run the test and verify it passes**

```bash
cargo nextest run -p temper-ingest --features embed embeds_a_short_text_from_bundled_model
```

Expected: PASS. First run takes ~2-5s (model load); subsequent runs in the same process <100ms.

- [ ] **Step 8: Run the full `temper-ingest` test suite**

```bash
cargo nextest run -p temper-ingest --features embed
```

Expected: all existing tests still pass. No regression in chunking, normalization, or other pure-compute paths.

- [ ] **Step 9: Verify `cargo machete` doesn't complain about removed deps**

```bash
cargo machete crates/temper-ingest
```

Expected: no unused dependencies.

- [ ] **Step 10: Commit**

```bash
git add crates/temper-ingest/src/embed.rs crates/temper-ingest/Cargo.toml
git commit -m "feat(ingest): load bge-base-en-v1.5 from bundled bytes instead of HF Hub"
```

---

## Task 5: Create `temper_ingest::pipeline::prepare_markdown`

**Files:**
- Create: `crates/temper-ingest/src/pipeline.rs`
- Modify: `crates/temper-ingest/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/temper-ingest/src/pipeline.rs`:

```rust
//! Shared markdown → packed chunks pipeline.
//!
//! Single source of truth for "turn raw markdown into indexed chunks."
//! Used by the CLI (client-side precomputed path) and by the API service
//! layer (server-side markdown path). Keeps both sides identical.

use crate::{chunk::chunk_markdown, embed::embed_texts, EmbedError};
use temper_core::types::ingest::PackedChunk;

// Implemented in Step 3.
pub fn prepare_markdown(content: &str) -> Result<Vec<PackedChunk>, EmbedError> {
    todo!("implemented in Step 3")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_markdown_produces_packed_chunks() {
        let content = "# Title\n\nThis is a paragraph.\n\n## Section\n\nMore text here.";
        let result = prepare_markdown(content);
        assert!(result.is_ok(), "prepare_markdown should succeed: {result:?}");
        let chunks = result.unwrap();
        assert!(!chunks.is_empty(), "should produce at least one chunk");
        for chunk in &chunks {
            assert_eq!(chunk.embedding.len(), 768, "each chunk has 768-dim embedding");
            assert!(!chunk.content.is_empty(), "chunk content non-empty");
            assert!(!chunk.content_hash.is_empty(), "chunk has content_hash");
        }
    }

    #[test]
    fn prepare_markdown_preserves_chunk_order() {
        let content = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph.";
        let chunks = prepare_markdown(content).expect("should succeed");
        for (i, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.chunk_index as usize, i, "chunks indexed in order");
        }
    }
}
```

- [ ] **Step 2: Wire the module into `lib.rs`**

In `crates/temper-ingest/src/lib.rs`, add:

```rust
#[cfg(feature = "embed")]
pub mod pipeline;
```

- [ ] **Step 3: Run the test, confirm it fails on `todo!`**

```bash
cargo nextest run -p temper-ingest --features embed prepare_markdown_produces_packed_chunks
```

Expected: FAIL with panic on `todo!`.

- [ ] **Step 4: Implement `prepare_markdown`**

Replace the `todo!` body:

```rust
pub fn prepare_markdown(content: &str) -> Result<Vec<PackedChunk>, EmbedError> {
    let chunks = chunk_markdown(content);
    if chunks.is_empty() {
        return Ok(Vec::new());
    }
    let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
    let embeddings = embed_texts(&texts)?;
    assert_eq!(
        chunks.len(),
        embeddings.len(),
        "chunk and embedding count must match"
    );
    let packed: Vec<PackedChunk> = chunks
        .into_iter()
        .zip(embeddings)
        .enumerate()
        .map(|(i, (chunk, embedding))| PackedChunk {
            chunk_index: i as u32,
            header_path: chunk.header_path,
            content: chunk.content,
            content_hash: chunk.content_hash,
            embedding,
        })
        .collect();
    Ok(packed)
}
```

**Important:** match the exact field names on the existing `Chunk` (output of `chunk_markdown`) and `PackedChunk` (in `temper_core::types::ingest`). If `Chunk` doesn't have `header_path` / `content_hash` exactly, adapt — do not invent fields. Follow SG-1.

- [ ] **Step 5: Run both tests**

```bash
cargo nextest run -p temper-ingest --features embed pipeline::tests
```

Expected: both tests PASS.

- [ ] **Step 6: Run clippy**

```bash
cargo clippy -p temper-ingest --features embed --all-targets -- -D warnings
```

Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-ingest/src/pipeline.rs crates/temper-ingest/src/lib.rs
git commit -m "feat(ingest): add prepare_markdown shared pipeline function"
```

---

## Task 6: Refactor CLI `build_ingest_payload` to use `prepare_markdown`

**Files:**
- Modify: `crates/temper-cli/src/actions/ingest.rs`

**Why:** CLI drift prevention. The CLI's `build_ingest_payload` currently inlines `chunk_markdown → embed_texts → pack`. We replace that inline block with a single call to `prepare_markdown` so CLI and server use the same function.

- [ ] **Step 1: Read the current `build_ingest_payload` body**

```bash
grep -n "fn build_ingest_payload" crates/temper-cli/src/actions/ingest.rs
```

Read lines around the match (likely 141-193 per the spec). Identify the inline chunk/embed/pack block.

- [ ] **Step 2: Write a failing test**

At the bottom of `crates/temper-cli/src/actions/ingest.rs`, add:

```rust
#[cfg(all(test, feature = "embed"))]
mod tests_build_ingest_payload {
    use super::*;

    #[test]
    fn build_ingest_payload_uses_shared_pipeline() {
        let content = "# Test\n\nSome content.";
        // Construct a minimal params struct — adapt fields to the actual function signature.
        let payload = build_ingest_payload_for_test(content).expect("should build");
        assert_eq!(payload.content, content);
        assert!(!payload.chunks_packed.is_empty(), "chunks_packed populated");
        // Verify chunks unpack correctly and have embeddings
        let chunks = temper_core::types::ingest::unpack_chunks(&payload.chunks_packed)
            .expect("should unpack");
        assert!(!chunks.is_empty());
        assert_eq!(chunks[0].embedding.len(), 768);
    }

    // Helper that calls build_ingest_payload with minimal test params.
    // Adapt to match the real function signature.
    fn build_ingest_payload_for_test(content: &str) -> Result<IngestPayload, TemperError> {
        // Fill in with actual minimum required fields based on current signature
        todo!("adapt to real signature of build_ingest_payload in Step 3")
    }
}
```

- [ ] **Step 3: Run the test to see it fails**

```bash
cargo nextest run -p temper-cli --features embed build_ingest_payload_uses_shared_pipeline
```

Expected: FAIL.

- [ ] **Step 4: Implement `build_ingest_payload_for_test` matching the real function signature**

Read the actual `build_ingest_payload` signature and construct minimal inputs. Preserve the existing signature — don't change it. The test helper just needs to call it.

- [ ] **Step 5: Replace the inline chunk/embed/pack with `prepare_markdown`**

In `build_ingest_payload`, find the block that does:

```rust
let chunks = chunk_markdown(&content);
let texts: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
let embeddings = embed_texts(&texts)?;
// ... pack into Vec<PackedChunk> ...
```

Replace it with:

```rust
let packed = temper_ingest::pipeline::prepare_markdown(&content)
    .map_err(TemperError::Embed)?;
let chunks_packed = temper_core::types::ingest::pack_chunks(&packed)
    .map_err(|e| TemperError::Pack(e.to_string()))?;
```

Preserve the rest of the function exactly.

- [ ] **Step 6: Run the test and the existing CLI ingest tests**

```bash
cargo nextest run -p temper-cli --features embed
```

Expected: all tests pass.

- [ ] **Step 7: Run clippy**

```bash
cargo clippy -p temper-cli --features embed --all-targets -- -D warnings
```

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src/actions/ingest.rs
git commit -m "refactor(cli): use temper_ingest::pipeline::prepare_markdown in build_ingest_payload"
```

---

## Task 7: Make `IngestPayload.chunks_packed` and `content_hash` optional

**Files:**
- Modify: `crates/temper-core/src/types/ingest.rs`

**Why:** Wire-type change that enables the markdown-only path. Both fields become `Option<String>` with `#[serde(skip_serializing_if)]` so existing CLI clients remain wire-compatible.

- [ ] **Step 1: Write a failing test**

Add to the test module in `crates/temper-core/src/types/ingest.rs`:

```rust
#[test]
fn payload_serializes_with_optional_chunks_absent() {
    let payload = IngestPayload {
        title: "Test".to_owned(),
        origin_uri: "kb://ctx/task/test".to_owned(),
        context_name: "ctx".to_owned(),
        doc_type_name: "task".to_owned(),
        slug: "test".to_owned(),
        content: "# Test".to_owned(),
        content_hash: None,
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: None,
    };
    let json = serde_json::to_string(&payload).unwrap();
    assert!(!json.contains("chunks_packed"), "absent field should not serialize");
    assert!(!json.contains("content_hash"), "absent field should not serialize");
}

#[test]
fn payload_deserializes_with_optional_chunks_absent() {
    let json = r#"{
        "title": "Test",
        "origin_uri": "kb://ctx/task/test",
        "context_name": "ctx",
        "doc_type_name": "task",
        "slug": "test",
        "content": "# Test"
    }"#;
    let payload: IngestPayload = serde_json::from_str(json).unwrap();
    assert!(payload.chunks_packed.is_none());
    assert!(payload.content_hash.is_none());
}

#[test]
fn payload_with_chunks_present_roundtrips() {
    // Existing precomputed path regression guard
    let payload = IngestPayload {
        title: "Test".to_owned(),
        origin_uri: "kb://ctx/task/test".to_owned(),
        context_name: "ctx".to_owned(),
        doc_type_name: "task".to_owned(),
        slug: "test".to_owned(),
        content: "# Test".to_owned(),
        content_hash: Some("sha256:abc".to_owned()),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(pack_chunks(&sample_chunks()).unwrap()),
    };
    let json = serde_json::to_string(&payload).unwrap();
    let deserialized: IngestPayload = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.content_hash, Some("sha256:abc".to_owned()));
    assert!(deserialized.chunks_packed.is_some());
}
```

- [ ] **Step 2: Run the tests, verify they fail to compile**

```bash
cargo nextest run -p temper-core payload_serializes_with_optional_chunks_absent
```

Expected: compilation error because `content_hash` and `chunks_packed` are not yet `Option`.

- [ ] **Step 3: Update the struct definition**

In `crates/temper-core/src/types/ingest.rs`, change:

```rust
pub struct IngestPayload {
    pub title: String,
    pub origin_uri: String,
    pub context_name: String,
    pub doc_type_name: String,
    /// `"sha256:<hex>"` — server computes if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    pub slug: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub managed_meta: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_meta: Option<serde_json::Value>,
    /// Base64 MessagePack of `Vec<PackedChunk>`.
    /// Server computes via `temper_ingest::pipeline::prepare_markdown` if absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunks_packed: Option<String>,
}
```

- [ ] **Step 4: Update the existing `payload_serialization_roundtrip` test**

The existing test in the file populates all fields. Wrap `content_hash` and `chunks_packed` values in `Some(...)`:

```rust
let payload = IngestPayload {
    // ...
    content_hash: Some("sha256:abc".to_owned()),
    // ...
    chunks_packed: Some(pack_chunks(&sample_chunks()).unwrap()),
};
```

And update assertions accordingly.

- [ ] **Step 5: Run tests**

```bash
cargo nextest run -p temper-core
```

Expected: all tests pass, including new ones.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-core/src/types/ingest.rs
git commit -m "refactor(core): IngestPayload chunks_packed and content_hash are optional"
```

---

## Task 8: Delete `ContentIngestRequest` type from temper-core

**Files:**
- Modify: `crates/temper-core/src/types/ingest.rs`
- Search: all callers in the workspace

**Why:** Removing dead wire type for the retired TS proxy path. Any remaining callers point to it from the TS generation path; those go away in session 3, but we remove the Rust side now because nothing in the new architecture needs it.

- [ ] **Step 1: Find all references to `ContentIngestRequest`**

```bash
grep -rn "ContentIngestRequest" crates/ api/ packages/
```

Note the files that reference it. There may be a `spawn_content_ingest_post` helper in `crates/temper-mcp/src/tools/resources.rs` that uses it.

- [ ] **Step 2: Temporarily disable the references**

If `crates/temper-mcp/src/tools/resources.rs` imports `ContentIngestRequest`, leave the import for now but note that `spawn_content_ingest_post` and its callers will be removed in Session 3 Task 26. For Session 2, we will delete `ContentIngestRequest` only after verifying nothing in the Rust tree still needs it.

**Decision point:**
- If `crates/temper-mcp` is the only Rust consumer and the function that uses it will be deleted in Task 26, hold off on deleting `ContentIngestRequest` until Task 26. Mark this task as blocked-by-Task-26 and skip to Task 9.
- If `crates/temper-mcp` no longer uses it (for example, because it already went through an intermediate refactor), proceed with deletion now.

**Default path:** hold `ContentIngestRequest` deletion for Task 26. Move to Task 9.

- [ ] **Step 3: Record the deferral**

```bash
# No commit — nothing to add. Update the task list to note Task 8 is merged into Task 26.
```

---

## Task 9: Add `ingest-pipeline` feature flag to `temper-api`

**Files:**
- Modify: `crates/temper-api/Cargo.toml`

**Why:** Gate the `temper-ingest` dependency behind a feature flag so `temper-api` as a library stays lean. The Vercel binary enables the flag; library consumers and tests that don't need embedding don't pay the compile cost.

- [ ] **Step 1: Add the optional dep and feature in `crates/temper-api/Cargo.toml`**

```toml
[dependencies]
# ... existing deps ...
temper-ingest = { path = "../temper-ingest", default-features = false, features = ["embed"], optional = true }

[features]
# ... existing features ...
ingest-pipeline = ["dep:temper-ingest"]
```

- [ ] **Step 2: Verify `cargo check` still works without the feature**

```bash
cargo check -p temper-api
```

Expected: compiles. `temper-ingest` is not pulled in.

- [ ] **Step 3: Verify `cargo check` works with the feature**

```bash
cargo check -p temper-api --features ingest-pipeline
```

Expected: compiles. `temper-ingest` is pulled in along with `embed` feature.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/Cargo.toml
git commit -m "feat(api): add ingest-pipeline feature flag gating temper-ingest"
```

---

## Task 10: Add `IngestError::Validation` and `IngestError::StructuralMoveNotSupported` variants

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs` (or wherever `IngestError` lives)

- [ ] **Step 1: Find the current `IngestError` definition**

```bash
grep -rn "enum IngestError" crates/temper-api/src
```

- [ ] **Step 2: Write a failing test**

Add a test that constructs and matches the new variants:

```rust
#[cfg(test)]
mod tests_ingest_error {
    use super::*;
    use temper_core::schema::{ValidationIssue, ValidationIssueKind};

    #[test]
    fn validation_error_carries_issues() {
        let err = IngestError::Validation {
            doc_type: "task".to_owned(),
            issues: vec![ValidationIssue {
                field: "temper-stage".to_owned(),
                kind: ValidationIssueKind::Missing,
                message: "temper-stage is required".to_owned(),
            }],
        };
        match err {
            IngestError::Validation { doc_type, issues } => {
                assert_eq!(doc_type, "task");
                assert_eq!(issues.len(), 1);
                assert_eq!(issues[0].field, "temper-stage");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn structural_move_not_supported_carries_field() {
        let err = IngestError::StructuralMoveNotSupported {
            field: "temper-context".to_owned(),
            message: "use `temper resource update --context-to` to move".to_owned(),
        };
        match err {
            IngestError::StructuralMoveNotSupported { field, .. } => {
                assert_eq!(field, "temper-context");
            }
            _ => panic!("wrong variant"),
        }
    }
}
```

**Important:** match the exact `ValidationIssue` struct shape from `temper_core::schema`. If the field is called `kind` but uses a different enum name, adapt. Read `crates/temper-core/src/schema.rs` first.

- [ ] **Step 3: Run, verify compile error on missing variants**

```bash
cargo nextest run -p temper-api --features ingest-pipeline validation_error_carries_issues
```

Expected: compile error.

- [ ] **Step 4: Add the variants to `IngestError`**

```rust
use temper_core::schema::ValidationIssue;

#[derive(Debug, thiserror::Error)]
pub enum IngestError {
    // ... existing variants ...
    #[error("managed_meta validation failed for doc_type={doc_type}: {} issues", .issues.len())]
    Validation {
        doc_type: String,
        issues: Vec<ValidationIssue>,
    },
    #[error("structural move via field '{field}' is not supported: {message}")]
    StructuralMoveNotSupported {
        field: String,
        message: String,
    },
    #[error("invalid managed_meta shape: {0}")]
    InvalidManagedMeta(String),
    #[error("managed_meta YAML serialization failed: {0}")]
    MetaSerialize(#[from] serde_yaml::Error),
    #[cfg(feature = "ingest-pipeline")]
    #[error("embed failed: {0}")]
    Embed(String),
    #[cfg(feature = "ingest-pipeline")]
    #[error("chunks pack failed: {0}")]
    Pack(String),
    #[cfg(feature = "ingest-pipeline")]
    #[error("chunks_packed missing: {0}")]
    MissingChunksPacked(String),
    #[cfg(feature = "ingest-pipeline")]
    #[error("content_hash missing: {0}")]
    MissingContentHash(String),
}
```

Add `serde_yaml` to `Cargo.toml` under `[dependencies]` if not already present.

- [ ] **Step 5: Run tests**

```bash
cargo nextest run -p temper-api --features ingest-pipeline tests_ingest_error
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/services/ingest_service.rs crates/temper-api/Cargo.toml
git commit -m "feat(api): add Validation and StructuralMoveNotSupported IngestError variants"
```

---

## Task 11: Add `strip_system_managed_fields` helper

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs`

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests_strip_system_managed_fields {
    use super::*;
    use serde_json::json;

    #[test]
    fn strips_tier1_fields() {
        let input = json!({
            "temper-id": "abc",
            "temper-created": "2026-04-09",
            "temper-owner": "@me",
            "temper-stage": "backlog"
        });
        let stripped = strip_system_managed_fields(input);
        let obj = stripped.as_object().unwrap();
        assert!(!obj.contains_key("temper-id"));
        assert!(!obj.contains_key("temper-created"));
        assert!(!obj.contains_key("temper-owner"));
        assert!(obj.contains_key("temper-stage"), "tier-3 fields preserved");
    }

    #[test]
    fn strips_all_system_managed_fields() {
        let input = json!({
            "temper-id": "a",
            "temper-provisional-id": "b",
            "temper-created": "c",
            "temper-updated": "d",
            "temper-owner": "e",
            "temper-source": "f",
            "temper-legacy-id": "g",
            "temper-stage": "backlog"
        });
        let stripped = strip_system_managed_fields(input);
        let obj = stripped.as_object().unwrap();
        assert_eq!(obj.len(), 1);
        assert!(obj.contains_key("temper-stage"));
    }

    #[test]
    fn handles_non_object_value() {
        // If managed_meta is not an object (e.g. null), pass through unchanged.
        let input = serde_json::Value::Null;
        let stripped = strip_system_managed_fields(input);
        assert!(stripped.is_null());
    }
}
```

- [ ] **Step 2: Run, verify compile error**

```bash
cargo nextest run -p temper-api --features ingest-pipeline strip_system_managed_fields
```

Expected: compile error (function doesn't exist).

- [ ] **Step 3: Implement `strip_system_managed_fields`**

```rust
/// Remove tier-1 identity/audit fields from input `managed_meta`.
///
/// Agents may echo these back from a `get_resource` call; they should not cause
/// validation errors. `SYSTEM_MANAGED_FIELDS` is the canonical list from
/// `temper_core::schema`. `temper-context`, `temper-type`, and `slug` are
/// tier-2 (handled separately) and are NOT stripped here — they remain present
/// so we can detect structural-move attempts in the update path.
fn strip_system_managed_fields(mut meta: serde_json::Value) -> serde_json::Value {
    use temper_core::schema::SYSTEM_MANAGED_FIELDS;
    const TIER1_FIELDS: &[&str] = &[
        "temper-id",
        "temper-provisional-id",
        "temper-created",
        "temper-updated",
        "temper-owner",
        "temper-source",
        "temper-legacy-id",
    ];
    if let Some(obj) = meta.as_object_mut() {
        for field in TIER1_FIELDS {
            if obj.remove(*field).is_some() {
                tracing::warn!(field = *field, "stripped tier-1 system-managed field from input managed_meta");
            }
        }
    }
    meta
}
```

**Note:** we define `TIER1_FIELDS` explicitly here rather than using `SYSTEM_MANAGED_FIELDS` directly, because the latter also contains tier-2 fields (`temper-context`, `temper-type`, `slug`) that we want to detect separately. Verify the constant's membership by reading `crates/temper-core/src/schema.rs:272-283`.

- [ ] **Step 4: Run tests**

```bash
cargo nextest run -p temper-api --features ingest-pipeline strip_system_managed_fields
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/services/ingest_service.rs
git commit -m "feat(api): add strip_system_managed_fields helper for tier-1 fields"
```

---

## Task 12: Add `validate_managed_meta` helper with synthetic-frontmatter merge

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs`

**Why:** Central validation path used by both create and update. The synthetic-frontmatter merge is critical — without it, every task creation fails because `slug` (a top-level param, not in `managed_meta`) is listed as `required` in `task.schema.json`.

- [ ] **Step 1: Write failing tests**

```rust
#[cfg(test)]
mod tests_validate_managed_meta {
    use super::*;
    use serde_json::json;

    #[test]
    fn validates_task_with_complete_managed_meta() {
        let managed_meta = json!({"temper-stage": "backlog", "temper-mode": "build", "temper-effort": "medium"});
        let params = ValidateParams {
            doc_type: "task",
            managed_meta: Some(&managed_meta),
            slug: "test-task",
            title: "Test Task",
            context_name: "ctx",
        };
        let result = validate_managed_meta(&params);
        assert!(result.is_ok(), "task with complete meta should validate: {result:?}");
    }

    #[test]
    fn rejects_task_missing_temper_stage() {
        let managed_meta = json!({"temper-mode": "build"});
        let params = ValidateParams {
            doc_type: "task",
            managed_meta: Some(&managed_meta),
            slug: "test-task",
            title: "Test Task",
            context_name: "ctx",
        };
        let result = validate_managed_meta(&params);
        match result {
            Err(IngestError::Validation { doc_type, issues }) => {
                assert_eq!(doc_type, "task");
                assert!(issues.iter().any(|i| i.field == "temper-stage"));
            }
            other => panic!("expected Validation error, got {other:?}"),
        }
    }

    #[test]
    fn validates_session_with_empty_managed_meta() {
        // session has no tier-3 required fields.
        let params = ValidateParams {
            doc_type: "session",
            managed_meta: None,
            slug: "test-session",
            title: "Test Session",
            context_name: "ctx",
        };
        let result = validate_managed_meta(&params);
        assert!(result.is_ok(), "session with no managed_meta should validate: {result:?}");
    }

    #[test]
    fn synthetic_merge_injects_slug_from_params_not_managed_meta() {
        // task.schema requires both temper-stage and slug; slug comes from params,
        // temper-stage from managed_meta. This tests the merge logic specifically.
        let managed_meta = json!({"temper-stage": "backlog"});
        let params = ValidateParams {
            doc_type: "task",
            managed_meta: Some(&managed_meta),
            slug: "slug-from-params",
            title: "T",
            context_name: "ctx",
        };
        let result = validate_managed_meta(&params);
        assert!(result.is_ok(), "slug from params should satisfy schema required: {result:?}");
    }

    #[test]
    fn rejects_invalid_enum_value() {
        let managed_meta = json!({"temper-stage": "bogus-stage"});
        let params = ValidateParams {
            doc_type: "task",
            managed_meta: Some(&managed_meta),
            slug: "t",
            title: "T",
            context_name: "ctx",
        };
        let result = validate_managed_meta(&params);
        match result {
            Err(IngestError::Validation { issues, .. }) => {
                assert!(issues.iter().any(|i| i.field == "temper-stage"));
            }
            other => panic!("expected Validation error, got {other:?}"),
        }
    }
}
```

- [ ] **Step 2: Run, verify compile error**

```bash
cargo nextest run -p temper-api --features ingest-pipeline validate_managed_meta
```

Expected: compile error.

- [ ] **Step 3: Implement `ValidateParams` + `validate_managed_meta`**

```rust
/// Parameters for schema validation at the service-layer boundary.
pub(crate) struct ValidateParams<'a> {
    pub doc_type: &'a str,
    pub managed_meta: Option<&'a serde_json::Value>,
    pub slug: &'a str,
    pub title: &'a str,
    pub context_name: &'a str,
}

/// Validate managed_meta against the doc_type schema, merging in top-level
/// parameters so schema-required tier-2 fields (slug, temper-context, temper-type)
/// are satisfied without the agent having to pass them inside managed_meta.
pub(crate) fn validate_managed_meta(params: &ValidateParams<'_>) -> Result<(), IngestError> {
    use serde_json::json;
    // 1. Start with managed_meta (or empty object)
    let mut synthetic: serde_json::Value = params
        .managed_meta
        .cloned()
        .unwrap_or_else(|| json!({}));
    // 2. Strip tier-1 fields (defensive: caller should already have done this)
    synthetic = strip_system_managed_fields(synthetic);
    // 3. Inject tier-2 fields and placeholders for tier-1 so schema required checks pass.
    //    Reject non-object managed_meta with a clear error.
    if !synthetic.is_object() {
        synthetic = serde_json::Value::Object(serde_json::Map::new());
    }
    let obj = synthetic
        .as_object_mut()
        .ok_or_else(|| IngestError::InvalidManagedMeta("managed_meta must be a JSON object".to_owned()))?;
    obj.insert("slug".to_owned(), json!(params.slug));
    obj.insert("title".to_owned(), json!(params.title));
    obj.insert("temper-context".to_owned(), json!(params.context_name));
    obj.insert("temper-type".to_owned(), json!(params.doc_type));
    // Tier-1 placeholders for schema required checks
    obj.insert("temper-id".to_owned(), json!("00000000-0000-0000-0000-000000000000"));
    obj.insert("temper-created".to_owned(), json!("2000-01-01T00:00:00Z"));
    // 4. Convert to YAML and validate
    let yaml = serde_yaml::to_string(&synthetic).map_err(IngestError::MetaSerialize)?;
    let issues = temper_core::schema::validate_frontmatter(params.doc_type, &yaml);
    if issues.is_empty() {
        Ok(())
    } else {
        Err(IngestError::Validation {
            doc_type: params.doc_type.to_owned(),
            issues,
        })
    }
}
```

**Important:** match the exact `validate_frontmatter` signature in `temper_core::schema`. If it returns `Result<Vec<ValidationIssue>, _>` instead of `Vec<ValidationIssue>`, adapt. Read the function's current signature before writing this.

- [ ] **Step 4: Run tests**

```bash
cargo nextest run -p temper-api --features ingest-pipeline validate_managed_meta
```

Expected: PASS. If any test fails because the schema for `session` does have a `required` tier-3 field, adjust the test to use a valid managed_meta for that type.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/services/ingest_service.rs
git commit -m "feat(api): validate_managed_meta with synthetic-frontmatter merge"
```

---

## Task 13: Wire markdown-path branch into `create_resource_with_manifest`

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs`

**Why:** The core architectural change. When `chunks_packed.is_none()`, call `prepare_markdown` and fill in `content_hash`. Validation runs before compute.

- [ ] **Step 1: Write failing test (service-layer integration)**

Add to the integration test module (or create one) in `crates/temper-api/src/services/ingest_service.rs`:

```rust
#[cfg(all(test, feature = "test-db", feature = "ingest-pipeline"))]
mod tests_markdown_path {
    use super::*;
    // Test helpers from existing test infrastructure
    use crate::test_support::{setup_test_db, seed_profile_and_context};

    #[tokio::test]
    async fn create_resource_from_markdown_round_trip() {
        let (pool, profile_id) = setup_test_db().await;
        let context_id = seed_profile_and_context(&pool, &profile_id).await;

        let payload = IngestPayload {
            title: "Round Trip Test".to_owned(),
            origin_uri: "kb://temper/concept/round-trip".to_owned(),
            context_name: "temper".to_owned(),
            doc_type_name: "concept".to_owned(),
            slug: "round-trip".to_owned(),
            content: "# Round Trip\n\nThis is a test concept for the round-trip flow.".to_owned(),
            content_hash: None,         // server computes
            metadata: None,
            managed_meta: None,
            open_meta: None,
            chunks_packed: None,        // server computes via prepare_markdown
        };

        let result = create_resource_with_manifest(&pool, &profile_id, payload).await;
        assert!(result.is_ok(), "create should succeed: {result:?}");
        let resource = result.unwrap();
        assert!(resource.id != uuid::Uuid::nil());
        assert_eq!(resource.title, "Round Trip Test");

        // Verify chunks written to DB
        let chunk_count: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM kb_resource_chunks WHERE kb_resource_id = $1",
            resource.id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(chunk_count > 0, "chunks written for markdown path");
    }

    #[tokio::test]
    async fn create_resource_from_markdown_validates_before_embed() {
        let (pool, profile_id) = setup_test_db().await;
        let _context_id = seed_profile_and_context(&pool, &profile_id).await;

        // Task missing temper-stage — validation should fail before prepare_markdown runs.
        let payload = IngestPayload {
            title: "Invalid Task".to_owned(),
            origin_uri: "kb://temper/task/invalid".to_owned(),
            context_name: "temper".to_owned(),
            doc_type_name: "task".to_owned(),
            slug: "invalid".to_owned(),
            content: "# Invalid".to_owned(),
            content_hash: None,
            metadata: None,
            managed_meta: Some(serde_json::json!({"temper-mode": "build"})),
            open_meta: None,
            chunks_packed: None,
        };

        let result = create_resource_with_manifest(&pool, &profile_id, payload).await;
        match result {
            Err(IngestError::Validation { doc_type, issues }) => {
                assert_eq!(doc_type, "task");
                assert!(issues.iter().any(|i| i.field == "temper-stage"));
            }
            other => panic!("expected Validation error, got {other:?}"),
        }

        // Verify nothing was written
        let resource_count: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) AS \"count!\" FROM kb_resources WHERE slug = 'invalid'"
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(resource_count, 0, "no resource row should exist after validation failure");
    }

    #[tokio::test]
    async fn create_resource_from_precomputed_chunks_unchanged() {
        // Regression guard: existing CLI path still works
        let (pool, profile_id) = setup_test_db().await;
        let _context_id = seed_profile_and_context(&pool, &profile_id).await;

        let precomputed = temper_ingest::pipeline::prepare_markdown("# CLI path\n\nStill works.")
            .expect("prepare_markdown should succeed");
        let chunks_packed = temper_core::types::ingest::pack_chunks(&precomputed).unwrap();

        let payload = IngestPayload {
            title: "CLI Path".to_owned(),
            origin_uri: "kb://temper/concept/cli-path".to_owned(),
            context_name: "temper".to_owned(),
            doc_type_name: "concept".to_owned(),
            slug: "cli-path".to_owned(),
            content: "# CLI path\n\nStill works.".to_owned(),
            content_hash: Some("sha256:placeholder".to_owned()),
            metadata: None,
            managed_meta: None,
            open_meta: None,
            chunks_packed: Some(chunks_packed),
        };

        let result = create_resource_with_manifest(&pool, &profile_id, payload).await;
        assert!(result.is_ok(), "precomputed path should succeed: {result:?}");
    }
}
```

- [ ] **Step 2: Run the tests, confirm they fail**

```bash
cargo nextest run -p temper-api --features "test-db,ingest-pipeline" tests_markdown_path
```

Expected: FAIL — the `None` paths trigger a failure because the function doesn't yet handle them.

- [ ] **Step 3: Implement the markdown-path branch**

In `create_resource_with_manifest`, add at the top (before any DB work, after loading doc_type):

```rust
// 1. Strip tier-1 fields and validate managed_meta against the schema.
let stripped_managed_meta = payload
    .managed_meta
    .map(strip_system_managed_fields);
let validate_params = ValidateParams {
    doc_type: &payload.doc_type_name,
    managed_meta: stripped_managed_meta.as_ref(),
    slug: &payload.slug,
    title: &payload.title,
    context_name: &payload.context_name,
};
validate_managed_meta(&validate_params)?;
payload.managed_meta = stripped_managed_meta;

// 2. If chunks_packed is absent, run the shared pipeline and compute the hash.
#[cfg(feature = "ingest-pipeline")]
if payload.chunks_packed.is_none() {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(payload.content.as_bytes());
    payload.content_hash = Some(format!("sha256:{:x}", hash));
    let packed = temper_ingest::pipeline::prepare_markdown(&payload.content)
        .map_err(|e| IngestError::Embed(e.to_string()))?;
    payload.chunks_packed = Some(
        temper_core::types::ingest::pack_chunks(&packed)
            .map_err(|e| IngestError::Pack(e.to_string()))?,
    );
}

// 3. Fail loudly if chunks_packed is still None here (e.g. feature disabled).
let chunks_packed = payload.chunks_packed.ok_or_else(|| {
    IngestError::MissingChunksPacked(
        "chunks_packed must be present; server-side pipeline requires `ingest-pipeline` feature".to_owned(),
    )
})?;
let content_hash = payload.content_hash.ok_or_else(|| {
    IngestError::MissingContentHash("content_hash must be present".to_owned())
})?;

// 4. Proceed with existing DB write path, using `chunks_packed` and `content_hash`
//    as local non-Optional values from here on.
```

**Important:** take `payload` as `mut IngestPayload` so we can fill in the optional fields. The `Pack`, `MissingChunksPacked`, `MissingContentHash`, and `Embed` variants on `IngestError` were added in Task 10 — they should already exist by the time you reach this task. Add `sha2` to `Cargo.toml` if not already a dep. Match the real variable flow in the existing function — do not break the existing precomputed path logic.

- [ ] **Step 4: Run the tests**

```bash
cargo nextest run -p temper-api --features "test-db,ingest-pipeline" tests_markdown_path
```

Expected: all three tests PASS.

- [ ] **Step 5: Run clippy**

```bash
cargo clippy -p temper-api --features "test-db,ingest-pipeline" --all-targets -- -D warnings
```

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/services/ingest_service.rs crates/temper-api/Cargo.toml
git commit -m "feat(api): markdown-path branch in create_resource_with_manifest"
```

---

## Task 14: Enable `ingest-pipeline` feature in the Vercel binary

**Files:**
- Modify: `Cargo.toml` (root)

- [ ] **Step 1: Update the root `Cargo.toml`**

Find the workspace binary section and update the `temper-api` dep to enable the new feature:

```toml
[workspace.dependencies]
# ... existing ...
temper-api = { path = "crates/temper-api", features = ["ingest-pipeline"] }
temper-ingest = { path = "crates/temper-ingest", default-features = false, features = ["embed"] }
```

Or, if using `[dependencies]` directly in the root:

```toml
[dependencies]
temper-api = { path = "crates/temper-api", features = ["ingest-pipeline"] }
temper-ingest = { path = "crates/temper-ingest", default-features = false, features = ["embed"] }
# ... existing ...
```

- [ ] **Step 2: Build the `axum` binary**

```bash
cargo build --release --bin axum
```

Expected: compiles. Binary includes `temper-ingest` with `embed` feature.

- [ ] **Step 3: Check binary size**

```bash
ls -lh target/release/axum
```

Expected: ~80-100 MB (12 MB base binary + ~20 MB onnxruntime + ~45 MB model + ~2 MB tokenizer). If the binary is over 200 MB, investigate — something is pulling in fp32 weights or unexpected deps.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml
git commit -m "feat(temper-cloud): enable ingest-pipeline feature in Vercel binary"
```

- [ ] **Step 5: Push and deploy Vercel preview**

```bash
git push
```

Wait for preview build. Inspect size and cold-start logs.

---

## Task 15: Service-layer test — `update_resource_from_markdown_replaces_chunks_atomically`

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs` (test module)

- [ ] **Step 1: Add the test**

```rust
#[tokio::test]
async fn update_resource_from_markdown_replaces_chunks_atomically() {
    let (pool, profile_id) = setup_test_db().await;
    let _context_id = seed_profile_and_context(&pool, &profile_id).await;

    // Create initial resource
    let initial = IngestPayload {
        title: "Original".to_owned(),
        origin_uri: "kb://temper/concept/atomic-update".to_owned(),
        context_name: "temper".to_owned(),
        doc_type_name: "concept".to_owned(),
        slug: "atomic-update".to_owned(),
        content: "# Original\n\nFirst version.".to_owned(),
        content_hash: None,
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: None,
    };
    let resource = create_resource_with_manifest(&pool, &profile_id, initial)
        .await
        .expect("initial create");

    let original_chunk_count: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) AS \"count!\" FROM kb_resource_chunks WHERE kb_resource_id = $1",
        resource.id
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    // Update with new content (via the update path — adapt to actual function name)
    let updated = update_resource_content(
        &pool,
        &profile_id,
        resource.id,
        "# Updated\n\nSecond version with more content to produce different chunks.".to_owned(),
    )
    .await
    .expect("update");

    let new_chunk_count: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) AS \"count!\" FROM kb_resource_chunks WHERE kb_resource_id = $1",
        resource.id
    )
    .fetch_one(&pool)
    .await
    .unwrap();

    // New chunks present, no orphans from the old version.
    assert!(new_chunk_count > 0, "new chunks written");
    let _ = original_chunk_count; // old count may or may not equal new, depending on content
    assert_eq!(updated.title, resource.title, "title unchanged unless explicitly updated");
}
```

**Note:** the update path function name may differ. Read the existing code to find the correct function and signature; adapt the test. If there is no existing update function that takes markdown, add it in a sibling task first.

- [ ] **Step 2: Run and confirm it passes (or identify missing update path)**

```bash
cargo nextest run -p temper-api --features "test-db,ingest-pipeline" update_resource_from_markdown
```

**Decision point:**
- If test passes, the update path already exists — commit and move on.
- If `update_resource_content` doesn't exist, add a minimal version in the same file that follows the same pattern as `create_resource_with_manifest` (validation → pipeline → atomic write via `replace_resource_chunks`).

- [ ] **Step 3: Commit**

```bash
git add crates/temper-api/src/services/ingest_service.rs
git commit -m "test(api): atomic chunk replacement on markdown update"
```

---

## Task 16: Service-layer test — `create_resource_dispatches_on_chunks_packed_presence`

- [ ] **Step 1: Add the test**

```rust
#[tokio::test]
async fn create_resource_dispatches_on_chunks_packed_presence() {
    let (pool, profile_id) = setup_test_db().await;
    let _context_id = seed_profile_and_context(&pool, &profile_id).await;

    // Use a pre-computed payload that sets chunks_packed to a known non-None value.
    // The service should NOT re-run prepare_markdown in this case.
    let precomputed = temper_ingest::pipeline::prepare_markdown("# Precomputed").unwrap();
    let packed = temper_core::types::ingest::pack_chunks(&precomputed).unwrap();
    let packed_clone = packed.clone();

    let payload = IngestPayload {
        title: "Pre".to_owned(),
        origin_uri: "kb://temper/concept/pre".to_owned(),
        context_name: "temper".to_owned(),
        doc_type_name: "concept".to_owned(),
        slug: "pre".to_owned(),
        content: "# Precomputed".to_owned(),
        content_hash: Some("sha256:abc".to_owned()),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(packed),
    };
    let resource = create_resource_with_manifest(&pool, &profile_id, payload)
        .await
        .expect("precomputed create");

    // Fetch stored chunks and verify they match what we sent (not a re-computed set).
    let stored_chunks = sqlx::query!(
        "SELECT content_hash FROM kb_resource_chunks WHERE kb_resource_id = $1 ORDER BY chunk_index",
        resource.id
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    let sent_chunks = temper_core::types::ingest::unpack_chunks(&packed_clone).unwrap();
    assert_eq!(stored_chunks.len(), sent_chunks.len());
    for (stored, sent) in stored_chunks.iter().zip(sent_chunks.iter()) {
        assert_eq!(stored.content_hash, sent.content_hash,
            "precomputed path should store the client's chunks verbatim");
    }
}
```

- [ ] **Step 2: Run, commit**

```bash
cargo nextest run -p temper-api --features "test-db,ingest-pipeline" create_resource_dispatches_on_chunks_packed_presence
git add crates/temper-api/src/services/ingest_service.rs
git commit -m "test(api): precomputed-path chunks stored verbatim"
```

---

## Task 17: Service-layer test — tier-1 field stripping

- [ ] **Step 1: Add the test**

```rust
#[tokio::test]
async fn create_resource_strips_tier1_fields_from_managed_meta() {
    let (pool, profile_id) = setup_test_db().await;
    let _context_id = seed_profile_and_context(&pool, &profile_id).await;

    // Agent accidentally echoes back system-managed fields from a previous get_resource.
    let managed_meta = serde_json::json!({
        "temper-id": "00000000-0000-0000-0000-000000000001",
        "temper-created": "2020-01-01",
        "temper-owner": "@someone-else",
        "temper-stage": "backlog",
        "temper-mode": "plan",
        "temper-effort": "small"
    });

    let payload = IngestPayload {
        title: "Stripped".to_owned(),
        origin_uri: "kb://temper/task/stripped".to_owned(),
        context_name: "temper".to_owned(),
        doc_type_name: "task".to_owned(),
        slug: "stripped".to_owned(),
        content: "# Stripped".to_owned(),
        content_hash: None,
        metadata: None,
        managed_meta: Some(managed_meta),
        open_meta: None,
        chunks_packed: None,
    };

    let resource = create_resource_with_manifest(&pool, &profile_id, payload)
        .await
        .expect("should succeed with stripping");

    // Verify the server-generated ID is not the one the agent tried to pass.
    assert_ne!(
        resource.id.to_string(),
        "00000000-0000-0000-0000-000000000001",
        "server should ignore agent-supplied temper-id"
    );
}
```

- [ ] **Step 2: Run, commit**

```bash
cargo nextest run -p temper-api --features "test-db,ingest-pipeline" create_resource_strips_tier1_fields_from_managed_meta
git add crates/temper-api/src/services/ingest_service.rs
git commit -m "test(api): tier-1 fields stripped from agent-supplied managed_meta"
```

---

## Task 18: Service-layer test — tier-2 fields rejected in update

- [ ] **Step 1: Add the test**

```rust
#[tokio::test]
async fn update_resource_rejects_tier2_fields_in_managed_meta() {
    let (pool, profile_id) = setup_test_db().await;
    let _context_id = seed_profile_and_context(&pool, &profile_id).await;

    // Create a resource first.
    let initial = IngestPayload {
        title: "T".to_owned(),
        origin_uri: "kb://temper/concept/t".to_owned(),
        context_name: "temper".to_owned(),
        doc_type_name: "concept".to_owned(),
        slug: "t".to_owned(),
        content: "# T".to_owned(),
        content_hash: None,
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: None,
    };
    let resource = create_resource_with_manifest(&pool, &profile_id, initial)
        .await
        .expect("create");

    // Try to update with a tier-2 field in managed_meta.
    let bad_meta = serde_json::json!({"temper-context": "some-other-context"});
    let result = update_resource_managed_meta(&pool, &profile_id, resource.id, bad_meta).await;

    match result {
        Err(IngestError::StructuralMoveNotSupported { field, .. }) => {
            assert_eq!(field, "temper-context");
        }
        other => panic!("expected StructuralMoveNotSupported, got {other:?}"),
    }
}
```

**Note:** the update function name may differ. Read the existing code and adapt; if no such path exists yet, add a minimal `update_resource_managed_meta` function that performs:
1. Load current resource to get its doc_type
2. Check if any tier-2 field is present in input `managed_meta`. If yes, return `StructuralMoveNotSupported`.
3. Strip tier-1 fields.
4. Validate.
5. Apply the updated `managed_meta` via existing manifest update path.

- [ ] **Step 2: Run, implement if needed, commit**

```bash
cargo nextest run -p temper-api --features "test-db,ingest-pipeline" update_resource_rejects_tier2_fields
git add crates/temper-api/src/services/ingest_service.rs
git commit -m "feat(api): reject tier-2 fields in update_resource_managed_meta"
```

---

## Task 19: Run full session-2 Rust test suite

- [ ] **Step 1: Run all temper-api tests**

```bash
cargo nextest run -p temper-api --features "test-db,ingest-pipeline"
```

Expected: all pass.

- [ ] **Step 2: Run all temper-ingest tests**

```bash
cargo nextest run -p temper-ingest --features embed
```

- [ ] **Step 3: Run all temper-cli tests**

```bash
cargo nextest run -p temper-cli --features embed
```

- [ ] **Step 4: Run clippy across the whole workspace**

```bash
cargo clippy --all-features --all-targets -- -D warnings
```

- [ ] **Step 5: Run cargo machete**

```bash
cargo machete
```

Expected: no unused deps flagged.

- [ ] **Step 6: Run cargo make check**

```bash
cargo make check
```

Expected: all quality checks pass.

---

## Task 20: Deploy Vercel preview and cold-start smoke test

- [ ] **Step 1: Push the current branch state**

```bash
git push
```

- [ ] **Step 2: Wait for Vercel preview deploy**

Watch the preview URL become available. Note any build warnings.

- [ ] **Step 3: Smoke test the new markdown ingest path via curl**

```bash
PREVIEW=https://<preview-url>.vercel.app
TOKEN=$(temper auth token)

curl -v -X POST "$PREVIEW/api/ingest" \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "title": "Smoke Test",
    "origin_uri": "kb://temper/concept/smoke-test",
    "context_name": "temper",
    "doc_type_name": "concept",
    "slug": "smoke-test",
    "content": "# Smoke Test\n\nCold-start verification.\n\n## Details\n\nMeasure wall clock and verify response shape."
  }'
```

Expected:
- HTTP 200 response
- Response body contains a fully processed `EnrichedResource` with non-empty chunks
- Wall clock time <3 seconds on cold start

- [ ] **Step 4: Verify via search**

```bash
curl -s -H "Authorization: Bearer $TOKEN" \
  "$PREVIEW/api/search?q=smoke+test&context=temper"
```

Expected: the smoke-test resource appears in results.

- [ ] **Step 5: Document the smoke-test results**

Append to the task description or a session note:

- Cold-start time measured
- Warm-start time measured (second invocation)
- Any observed issues

**Merge criterion for session 2 (if splitting into a PR):** smoke test succeeds, all tests green, CLI path unchanged, TS content-ingest still alive as a backup. No user-visible behavior change on the MCP side yet — that comes in session 3.

---

## Task 21: Session 2 review checkpoint

- [ ] **Step 1: Verify the session-2 merge criteria**

Run through the checklist:

- [ ] `create_resource_with_manifest` atomicity confirmed or enforced (Task 0)
- [ ] `ort` linking mode decided and documented (Task 1)
- [ ] Git-LFS working (Task 2 + Task 3)
- [ ] Bundled model loads in tests (Task 4)
- [ ] `prepare_markdown` shared function exists and has tests (Task 5)
- [ ] CLI uses the shared function (Task 6)
- [ ] `IngestPayload` fields optional with backward-compatible serialization (Task 7)
- [ ] `ingest-pipeline` feature flag wired (Task 9)
- [ ] Validation helpers implemented and tested (Tasks 10-12)
- [ ] Markdown-path branch wired into service layer (Task 13)
- [ ] Binary builds with the feature (Task 14)
- [ ] All service-layer integration tests pass (Tasks 15-18)
- [ ] Full workspace test suite green (Task 19)
- [ ] Vercel preview smoke test succeeds (Task 20)

- [ ] **Step 2: Hand off to session 3 (or continue in-session if scope permits)**

Session 3 begins at Task 22. If time permits and confidence is high, continue immediately. Otherwise, save a session note with the current state and pick up in a fresh session.

---

# Session 3 — MCP Surface, TS Retirement, E2E Tests

## Task 22: Extend `list_doc_types` with `DocTypeSummary`

**Files:**
- Modify: `crates/temper-mcp/src/tools/doc_types.rs`

- [ ] **Step 1: Read the current `list_doc_types` implementation**

```bash
cat crates/temper-mcp/src/tools/doc_types.rs
```

Note the current output type and how it's constructed.

- [ ] **Step 2: Write a failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_type_summary_includes_required_fields_for_task() {
        let summary = build_doc_type_summary(&DocType {
            id: uuid::Uuid::nil(),
            name: "task".to_owned(),
        })
        .expect("should build summary");
        assert!(summary.has_schema);
        assert!(summary.required_fields.contains(&"temper-stage".to_owned()));
    }
}
```

- [ ] **Step 3: Run, verify compile failure**

```bash
cargo nextest run -p temper-mcp doc_type_summary_includes_required_fields_for_task
```

- [ ] **Step 4: Define `DocTypeSummary` and `build_doc_type_summary`**

```rust
use schemars::JsonSchema;
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Serialize, JsonSchema)]
pub struct DocTypeSummary {
    pub id: Uuid,
    pub name: String,
    pub has_schema: bool,
    pub required_fields: Vec<String>,
}

pub(crate) fn build_doc_type_summary(dt: &DocType) -> Result<DocTypeSummary, McpError> {
    let required_fields = match temper_core::schema::load_schema(&dt.name) {
        Ok(validator) => {
            // Adapt this to the real API exposed by load_schema / Validator.
            // If load_schema returns a compiled Validator, we need a sibling
            // function that returns the raw Value or the `required` field list.
            temper_core::schema::required_fields(&dt.name).unwrap_or_default()
        }
        Err(_) => Vec::new(),
    };
    let has_schema = !required_fields.is_empty();
    Ok(DocTypeSummary {
        id: dt.id,
        name: dt.name.clone(),
        has_schema,
        required_fields,
    })
}
```

**Important:** read `temper_core::schema` for the actual functions available. If `required_fields(doc_type)` does not exist, add it to `temper-core/src/schema.rs` as a small helper that parses the schema and returns its `required` array. Keep the helper minimal — one function, one purpose.

- [ ] **Step 5: Update `list_doc_types` to return `Vec<DocTypeSummary>`**

```rust
pub async fn list_doc_types(/* ... */) -> Result<Vec<DocTypeSummary>, McpError> {
    let rows = doc_type_service::list_all(&pool).await?;
    rows.iter().map(build_doc_type_summary).collect()
}
```

- [ ] **Step 6: Run tests**

```bash
cargo nextest run -p temper-mcp
```

- [ ] **Step 7: Commit**

```bash
git add crates/temper-mcp/src/tools/doc_types.rs crates/temper-core/src/schema.rs
git commit -m "feat(mcp): list_doc_types returns DocTypeSummary with required fields"
```

---

## Task 23: Add `describe_doc_type` MCP tool

**Files:**
- Modify: `crates/temper-mcp/src/tools/doc_types.rs`
- Modify: `crates/temper-mcp/src/service.rs` (tool registration)

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn describe_doc_type_task_returns_schema_and_example() {
    let response = describe_doc_type_impl("task").expect("should succeed");
    assert_eq!(response.name, "task");
    assert!(response.required_fields.contains(&"temper-stage".to_owned()));
    assert!(response.enum_fields.contains_key("temper-stage"));
    let stages = &response.enum_fields["temper-stage"];
    assert!(stages.contains(&"backlog".to_owned()));

    // example_managed_meta must round-trip through validate_frontmatter as valid.
    let yaml = serde_yaml::to_string(&response.example_managed_meta).unwrap();
    let issues = temper_core::schema::validate_frontmatter("task", &yaml);
    // Note: issues may still contain top-level required fields that the example
    // doesn't cover (like slug or title). Filter to just tier-3 fields.
    let tier3_issues: Vec<_> = issues.iter()
        .filter(|i| !matches!(i.field.as_str(), "slug" | "title" | "temper-context" | "temper-type" | "temper-id" | "temper-created"))
        .collect();
    assert!(tier3_issues.is_empty(), "example should satisfy all tier-3 required fields: {tier3_issues:?}");
}

#[test]
fn describe_doc_type_unknown_type_errors() {
    let result = describe_doc_type_impl("nonexistent");
    assert!(result.is_err());
}
```

- [ ] **Step 2: Run, verify compile failure**

```bash
cargo nextest run -p temper-mcp describe_doc_type
```

- [ ] **Step 3: Define the response type and implementation**

```rust
use std::collections::BTreeMap;

#[derive(Debug, Serialize, JsonSchema)]
pub struct DescribeDocTypeResponse {
    pub name: String,
    pub schema: serde_json::Value,
    pub required_fields: Vec<String>,
    pub enum_fields: BTreeMap<String, Vec<String>>,
    pub example_managed_meta: serde_json::Value,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DescribeDocTypeInput {
    pub name: String,
}

/// Return the JSON Schema for a doc_type along with a usable example
/// `managed_meta` object that satisfies all tier-3 required fields.
pub(crate) fn describe_doc_type_impl(name: &str) -> Result<DescribeDocTypeResponse, McpError> {
    let schema_value = temper_core::schema::schema_value(name)
        .map_err(|e| McpError::InvalidInput(format!("unknown doc type: {e}")))?;
    let required_fields = temper_core::schema::required_fields(name).unwrap_or_default();
    let enum_fields = extract_enum_fields(&schema_value);
    let example_managed_meta = build_example_managed_meta(&schema_value, &enum_fields);
    Ok(DescribeDocTypeResponse {
        name: name.to_owned(),
        schema: schema_value,
        required_fields,
        enum_fields,
        example_managed_meta,
    })
}

fn extract_enum_fields(schema: &serde_json::Value) -> BTreeMap<String, Vec<String>> {
    let mut out = BTreeMap::new();
    if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
        for (field, def) in props {
            if let Some(enum_arr) = def.get("enum").and_then(|v| v.as_array()) {
                let values: Vec<String> = enum_arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
                if !values.is_empty() {
                    out.insert(field.clone(), values);
                }
            }
        }
    }
    out
}

fn build_example_managed_meta(
    schema: &serde_json::Value,
    enum_fields: &BTreeMap<String, Vec<String>>,
) -> serde_json::Value {
    use serde_json::json;
    let mut obj = serde_json::Map::new();
    if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
        for field in required {
            let Some(field_name) = field.as_str() else { continue };
            // Skip tier-2 fields that are supplied via top-level params, not managed_meta
            if matches!(field_name, "slug" | "title" | "temper-context" | "temper-type"
                | "temper-id" | "temper-created" | "temper-updated" | "temper-owner") {
                continue;
            }
            let value = if let Some(values) = enum_fields.get(field_name) {
                json!(values.first().cloned().unwrap_or_default())
            } else {
                json!("<example-value>")
            };
            obj.insert(field_name.to_owned(), value);
        }
    }
    serde_json::Value::Object(obj)
}
```

**Important:** `temper_core::schema::schema_value(name)` and `required_fields(name)` need to exist as public helpers. If they don't, add them to `temper-core/src/schema.rs` as thin wrappers around the existing `include_str!`-backed schemas. One small helper per concern.

- [ ] **Step 4: Register the tool in `service.rs`**

In `crates/temper-mcp/src/service.rs`, find the `#[tool_router]` impl block and add:

```rust
#[tool(description = "Return the JSON Schema and example managed_meta for a given doc_type")]
async fn describe_doc_type(
    &self,
    Parameters(input): Parameters<DescribeDocTypeInput>,
) -> Result<CallToolResult, McpError> {
    let response = describe_doc_type_impl(&input.name)?;
    Ok(CallToolResult::success(vec![Content::json(serde_json::to_value(&response)?)]))
}
```

Match the existing tool registration pattern in the file exactly. Read a sibling tool first.

- [ ] **Step 5: Run tests**

```bash
cargo nextest run -p temper-mcp describe_doc_type
```

- [ ] **Step 6: Commit**

```bash
git add crates/temper-mcp/src/tools/doc_types.rs crates/temper-mcp/src/service.rs crates/temper-core/src/schema.rs
git commit -m "feat(mcp): add describe_doc_type tool with example_managed_meta"
```

---

## Task 24: Add `managed_meta` and `open_meta` to `CreateResourceInput`

**Files:**
- Modify: `crates/temper-mcp/src/tools/resources.rs`

- [ ] **Step 1: Add the fields to `CreateResourceInput`**

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateResourceInput {
    // ... existing fields ...
    #[serde(default)]
    pub managed_meta: Option<serde_json::Value>,
    #[serde(default)]
    pub open_meta: Option<serde_json::Value>,
}
```

- [ ] **Step 2: Thread both fields into the `IngestPayload` construction**

Find where `CreateResourceInput` is transformed into an `IngestPayload` inside the `create_resource` tool handler, and pass `managed_meta` + `open_meta` through.

- [ ] **Step 3: Run existing MCP tests**

```bash
cargo nextest run -p temper-mcp
```

Expected: no regressions.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-mcp/src/tools/resources.rs
git commit -m "feat(mcp): create_resource accepts managed_meta and open_meta"
```

---

## Task 25: Add `managed_meta` and `open_meta` to `UpdateResourceInput`

**Files:**
- Modify: `crates/temper-mcp/src/tools/resources.rs`

- [ ] **Step 1: Add the fields**

```rust
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateResourceInput {
    pub id: Uuid,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub managed_meta: Option<serde_json::Value>,
    #[serde(default)]
    pub open_meta: Option<serde_json::Value>,
}
```

- [ ] **Step 2: Thread fields into the update path**

Call `update_resource_managed_meta` (or equivalent) from the tool handler when `managed_meta` is present. Similarly for `open_meta`. If update paths don't exist, they were added in Task 18.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-mcp/src/tools/resources.rs
git commit -m "feat(mcp): update_resource accepts managed_meta and open_meta"
```

---

## Task 26: Delete `spawn_content_ingest_post` and call `ingest_service` directly

**Files:**
- Modify: `crates/temper-mcp/src/tools/resources.rs`
- Delete: `ContentIngestRequest` from `crates/temper-core/src/types/ingest.rs` (the deferred deletion from Task 8)

- [ ] **Step 1: Find all callers of `spawn_content_ingest_post`**

```bash
grep -n "spawn_content_ingest_post" crates/temper-mcp/src/tools/resources.rs
```

- [ ] **Step 2: Replace each caller with a direct service call**

For each `spawn_content_ingest_post(...)` call, replace with:

```rust
// Build IngestPayload and call the service layer directly.
let payload = IngestPayload {
    title: input.title.clone(),
    origin_uri: /* construct */,
    context_name: input.context_name.clone(),
    doc_type_name: input.doc_type_name.clone(),
    slug: input.slug.clone().unwrap_or_default(),
    content: input.content.clone().unwrap_or_default(),
    content_hash: None,
    metadata: None,
    managed_meta: input.managed_meta.clone(),
    open_meta: input.open_meta.clone(),
    chunks_packed: None,
};
let resource = ingest_service::create_resource_with_manifest(&self.pool, &profile_id, payload)
    .await
    .map_err(|e| McpError::IngestFailed(e.to_string()))?;
```

**Important:** match the real `create_resource_with_manifest` signature. Do not construct an HTTP client; this is an in-process call.

- [ ] **Step 3: Delete `spawn_content_ingest_post` function**

```bash
grep -n "fn spawn_content_ingest_post" crates/temper-mcp/src/tools/resources.rs
```

Delete the function and any helpers it used.

- [ ] **Step 4: Delete `ContentIngestRequest` type**

In `crates/temper-core/src/types/ingest.rs`, delete the `ContentIngestRequest` struct and any associated impls.

- [ ] **Step 5: Verify no other references remain**

```bash
grep -rn "ContentIngestRequest" crates/ api/ packages/
grep -rn "spawn_content_ingest_post" crates/
grep -rn "content-ingest" crates/
```

All should return nothing from the Rust tree. TypeScript references are handled in Task 34.

- [ ] **Step 6: Build + test**

```bash
cargo build --release --bin axum
cargo nextest run -p temper-mcp
cargo nextest run -p temper-core
```

Expected: all pass.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-mcp/src/tools/resources.rs crates/temper-core/src/types/ingest.rs
git commit -m "refactor(mcp): call ingest_service directly, delete content-ingest proxy"
```

---

## Task 27: Create new test fixtures

**Files:**
- Create: `packages/temper-cloud/tests/fixtures/task.md`
- Create: `packages/temper-cloud/tests/fixtures/session.md`
- Create: `packages/temper-cloud/tests/fixtures/concept.md`
- Create: `packages/temper-cloud/tests/fixtures/task-invalid.md`

- [ ] **Step 1: Create `task.md`**

```markdown
---
temper-type: task
temper-stage: backlog
temper-mode: build
temper-effort: small
title: "Fixture task for round-trip tests"
slug: "fixture-task"
---

# Fixture Task

This fixture drives the round-trip test for the task doc type.

## Acceptance Criteria

- Used only by tests.
- Never part of real vault content.
```

- [ ] **Step 2: Create `session.md`**

```markdown
---
temper-type: session
title: "Fixture session for round-trip tests"
slug: "fixture-session"
---

## Goal

Round-trip test fixture.

## What Happened

Placeholder content used by the Rust and TS test suites.

## Next Steps

None — this is a fixture.
```

- [ ] **Step 3: Create `concept.md`**

```markdown
---
temper-type: concept
title: "Fixture concept for round-trip tests"
slug: "fixture-concept"
---

# Fixture Concept

A concept used only by round-trip tests. Content is arbitrary but stable across test runs.
```

- [ ] **Step 4: Create `task-invalid.md`** (missing `temper-stage`)

```markdown
---
temper-type: task
temper-mode: build
title: "Fixture invalid task"
slug: "fixture-invalid-task"
---

# Intentionally Invalid Task

Missing `temper-stage`, used by validation-failure tests.
```

- [ ] **Step 5: Commit**

```bash
git add packages/temper-cloud/tests/fixtures/task.md \
        packages/temper-cloud/tests/fixtures/session.md \
        packages/temper-cloud/tests/fixtures/concept.md \
        packages/temper-cloud/tests/fixtures/task-invalid.md
git commit -m "test: add shared round-trip fixtures for task, session, concept, task-invalid"
```

---

## Task 28: E2E test — `mcp_create_resource_with_markdown_is_searchable`

**Files:**
- Create: `tests/e2e/tests/mcp_round_trip_test.rs`

- [ ] **Step 1: Read an existing e2e test for the setup pattern**

```bash
cat tests/e2e/tests/mcp_ingest_test.rs
```

Note the db setup, auth token minting, and MCP service spawn pattern.

- [ ] **Step 2: Write the test**

```rust
//! End-to-end MCP round-trip tests covering the unified markdown ingest path.

use temper_e2e_support::{setup_test_db, spawn_mcp_service, mint_test_token};

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../packages/temper-cloud/tests/fixtures"
);

#[tokio::test]
async fn mcp_create_resource_with_markdown_is_searchable() {
    let (pool, profile_id) = setup_test_db().await;
    let service = spawn_mcp_service(pool.clone(), profile_id.clone()).await;
    let client = service.in_process_client();

    // 1. list_doc_types includes concept with has_schema=true
    let doc_types = client.call_tool("list_doc_types", serde_json::json!({})).await.unwrap();
    let summaries: Vec<temper_mcp::tools::doc_types::DocTypeSummary> =
        serde_json::from_value(doc_types.content[0].json_value().unwrap().clone()).unwrap();
    let concept = summaries.iter().find(|s| s.name == "concept").expect("concept present");
    assert!(concept.has_schema);

    // 2. describe_doc_type returns an example
    let desc = client
        .call_tool("describe_doc_type", serde_json::json!({"name": "concept"}))
        .await
        .unwrap();
    let desc_value: temper_mcp::tools::doc_types::DescribeDocTypeResponse =
        serde_json::from_value(desc.content[0].json_value().unwrap().clone()).unwrap();
    assert_eq!(desc_value.name, "concept");

    // 3. create_resource with markdown body
    let content = std::fs::read_to_string(format!("{FIXTURES}/concept.md")).unwrap();
    let create_input = serde_json::json!({
        "context_name": "temper",
        "doc_type_name": "concept",
        "title": "Round-trip concept",
        "slug": "round-trip-concept",
        "content": content,
        "managed_meta": desc_value.example_managed_meta,
    });
    let created = client.call_tool("create_resource", create_input).await.unwrap();
    let resource_id = created.content[0]
        .json_value()
        .unwrap()
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap();

    // 4. search finds the new resource
    let search_result = client
        .call_tool(
            "search",
            serde_json::json!({"query": "round-trip test fixture", "context_name": "temper"}),
        )
        .await
        .unwrap();
    let hits: serde_json::Value = search_result.content[0].json_value().unwrap().clone();
    let found = hits
        .as_array()
        .unwrap()
        .iter()
        .any(|hit| hit.get("resource_id").and_then(|v| v.as_str()) == Some(resource_id));
    assert!(found, "newly-created resource should be searchable immediately");
}
```

**Important:** the `temper_e2e_support` module name is a placeholder — read the existing e2e tests and use the real helper names. Adapt the MCP client method names (`call_tool`, `in_process_client`, etc.) to match the rmcp API used by existing tests.

- [ ] **Step 3: Run the test**

```bash
cargo nextest run -p temper-e2e --features "test-db,ingest-pipeline" mcp_create_resource_with_markdown_is_searchable
```

- [ ] **Step 4: Commit**

```bash
git add tests/e2e/tests/mcp_round_trip_test.rs
git commit -m "test(e2e): MCP create+search round trip for markdown path"
```

---

## Task 29: E2E test — `mcp_create_resource_schema_validation_surfaces_structured_error`

- [ ] **Step 1: Add the test to `mcp_round_trip_test.rs`**

```rust
#[tokio::test]
async fn mcp_create_resource_schema_validation_surfaces_structured_error() {
    let (pool, profile_id) = setup_test_db().await;
    let service = spawn_mcp_service(pool.clone(), profile_id.clone()).await;
    let client = service.in_process_client();

    // Task missing temper-stage
    let content = std::fs::read_to_string(format!("{FIXTURES}/task-invalid.md")).unwrap();
    let input = serde_json::json!({
        "context_name": "temper",
        "doc_type_name": "task",
        "title": "Invalid Task",
        "slug": "invalid-task",
        "content": content,
        "managed_meta": {"temper-mode": "build"},
    });

    let result = client.call_tool("create_resource", input).await;
    match result {
        Err(e) => {
            // Expect structured validation error data
            let msg = format!("{e:?}");
            assert!(msg.contains("Validation") || msg.contains("temper-stage"),
                "error should mention validation or missing field: {msg}");
        }
        Ok(_) => panic!("expected validation error"),
    }

    // Verify no resource was created
    let search = client
        .call_tool("search", serde_json::json!({"query": "invalid-task", "context_name": "temper"}))
        .await
        .unwrap();
    let hits = search.content[0].json_value().unwrap().as_array().cloned().unwrap_or_default();
    assert!(hits.is_empty() || !hits.iter().any(|h| h.get("slug").and_then(|v| v.as_str()) == Some("invalid-task")));
}
```

- [ ] **Step 2: Run, commit**

```bash
cargo nextest run -p temper-e2e --features "test-db,ingest-pipeline" mcp_create_resource_schema_validation
git add tests/e2e/tests/mcp_round_trip_test.rs
git commit -m "test(e2e): MCP create_resource surfaces structured validation errors"
```

---

## Task 30: E2E test — `mcp_describe_doc_type_returns_usable_example`

- [ ] **Step 1: Add the test**

```rust
#[tokio::test]
async fn mcp_describe_doc_type_returns_usable_example() {
    let (pool, profile_id) = setup_test_db().await;
    let service = spawn_mcp_service(pool.clone(), profile_id.clone()).await;
    let client = service.in_process_client();

    let doc_types = ["task", "goal", "session", "research", "concept", "decision"];
    for name in doc_types {
        let desc = client
            .call_tool("describe_doc_type", serde_json::json!({"name": name}))
            .await
            .unwrap_or_else(|e| panic!("describe_doc_type({name}) failed: {e:?}"));
        let response: temper_mcp::tools::doc_types::DescribeDocTypeResponse =
            serde_json::from_value(desc.content[0].json_value().unwrap().clone()).unwrap();

        // Use the example directly in create_resource
        let create_input = serde_json::json!({
            "context_name": "temper",
            "doc_type_name": name,
            "title": format!("Example {name}"),
            "slug": format!("example-{name}"),
            "content": format!("# Example {name}\n\nGenerated from describe_doc_type."),
            "managed_meta": response.example_managed_meta,
        });
        let result = client.call_tool("create_resource", create_input).await;
        assert!(
            result.is_ok(),
            "create_resource should succeed using example_managed_meta from {name}: {result:?}"
        );
    }
}
```

- [ ] **Step 2: Run, commit**

```bash
cargo nextest run -p temper-e2e --features "test-db,ingest-pipeline" mcp_describe_doc_type_returns_usable_example
git add tests/e2e/tests/mcp_round_trip_test.rs
git commit -m "test(e2e): describe_doc_type example round-trips through create_resource"
```

---

## Task 31: E2E test — `mcp_list_doc_types_includes_required_fields`

- [ ] **Step 1: Add the test**

```rust
#[tokio::test]
async fn mcp_list_doc_types_includes_required_fields() {
    let (pool, profile_id) = setup_test_db().await;
    let service = spawn_mcp_service(pool.clone(), profile_id.clone()).await;
    let client = service.in_process_client();

    let result = client.call_tool("list_doc_types", serde_json::json!({})).await.unwrap();
    let summaries: Vec<temper_mcp::tools::doc_types::DocTypeSummary> =
        serde_json::from_value(result.content[0].json_value().unwrap().clone()).unwrap();

    let task = summaries.iter().find(|s| s.name == "task").expect("task present");
    assert!(task.has_schema);
    assert!(task.required_fields.contains(&"temper-stage".to_owned()));

    let session = summaries.iter().find(|s| s.name == "session").expect("session present");
    assert!(session.has_schema);
}
```

- [ ] **Step 2: Run, commit**

```bash
cargo nextest run -p temper-e2e --features "test-db,ingest-pipeline" mcp_list_doc_types_includes_required_fields
git add tests/e2e/tests/mcp_round_trip_test.rs
git commit -m "test(e2e): list_doc_types exposes required_fields"
```

---

## Task 32: E2E test — `mcp_update_resource_changes_content_and_reindexes`

- [ ] **Step 1: Add the test**

```rust
#[tokio::test]
async fn mcp_update_resource_changes_content_and_reindexes() {
    let (pool, profile_id) = setup_test_db().await;
    let service = spawn_mcp_service(pool.clone(), profile_id.clone()).await;
    let client = service.in_process_client();

    // Create
    let create = client
        .call_tool(
            "create_resource",
            serde_json::json!({
                "context_name": "temper",
                "doc_type_name": "concept",
                "title": "Changing Concept",
                "slug": "changing-concept",
                "content": "# Original\n\nFirst version content.",
            }),
        )
        .await
        .unwrap();
    let resource_id = create.content[0]
        .json_value()
        .unwrap()
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_owned();

    // Search finds the original
    let search_orig = client
        .call_tool(
            "search",
            serde_json::json!({"query": "first version", "context_name": "temper"}),
        )
        .await
        .unwrap();
    assert!(
        search_orig.content[0]
            .json_value()
            .unwrap()
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "original content searchable"
    );

    // Update with new content
    client
        .call_tool(
            "update_resource",
            serde_json::json!({
                "id": resource_id,
                "content": "# Updated\n\nSecond version with different wording.",
            }),
        )
        .await
        .unwrap();

    // Search no longer finds old terms, does find new ones
    let search_new = client
        .call_tool(
            "search",
            serde_json::json!({"query": "second version", "context_name": "temper"}),
        )
        .await
        .unwrap();
    assert!(
        search_new.content[0]
            .json_value()
            .unwrap()
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "updated content searchable"
    );
}
```

- [ ] **Step 2: Add a sibling regression test for the legacy metadata-shell path**

```rust
#[tokio::test]
async fn mcp_create_resource_without_content_creates_metadata_shell() {
    // Regression guard for the legacy two-step pattern: agents that want to
    // create a metadata shell first and add content later must still be able to.
    // (The unified path is preferred, but we don't break the old behavior.)
    let (pool, profile_id) = setup_test_db().await;
    let service = spawn_mcp_service(pool.clone(), profile_id.clone()).await;
    let client = service.in_process_client();

    let result = client
        .call_tool(
            "create_resource",
            serde_json::json!({
                "context_name": "temper",
                "doc_type_name": "concept",
                "title": "Shell Only",
                "slug": "shell-only",
                // No content field — metadata-only create
            }),
        )
        .await
        .unwrap();

    let resource: serde_json::Value = result.content[0].json_value().unwrap().clone();
    assert!(resource.get("id").is_some(), "metadata shell created");

    // Verify no chunks were created (no content to chunk)
    let resource_id = resource.get("id").and_then(|v| v.as_str()).unwrap();
    let chunk_count: i64 = sqlx::query_scalar!(
        "SELECT COUNT(*) AS \"count!\" FROM kb_resource_chunks WHERE kb_resource_id::text = $1",
        resource_id
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(chunk_count, 0, "metadata shell has no chunks");
}
```

- [ ] **Step 3: Run both tests**

```bash
cargo nextest run -p temper-e2e --features "test-db,ingest-pipeline" \
    mcp_update_resource_changes_content_and_reindexes \
    mcp_create_resource_without_content_creates_metadata_shell
```

- [ ] **Step 4: Commit**

```bash
git add tests/e2e/tests/mcp_round_trip_test.rs
git commit -m "test(e2e): update reindex and metadata-shell regression guard"
```

---

## Task 33: Add `test-rust-embed` CI job

**Files:**
- Modify: `.github/workflows/test-rust.yml`
- Modify: `.github/workflows/ci-success.yml`

- [ ] **Step 1: Read the current `test-rust.yml`**

```bash
cat .github/workflows/test-rust.yml
```

Note the existing job structure, service containers, and env vars.

- [ ] **Step 2: Add `test-rust-embed` as a new job**

Append to `.github/workflows/test-rust.yml`:

```yaml
  test-rust-embed:
    runs-on: ubuntu-latest
    needs: [code-quality]  # adapt to existing dependency name
    services:
      postgres:
        image: postgres:18
        env:
          POSTGRES_USER: postgres
          POSTGRES_PASSWORD: postgres
          POSTGRES_DB: temper_test
        ports:
          - 5432:5432
        options: >-
          --health-cmd pg_isready
          --health-interval 10s
          --health-timeout 5s
          --health-retries 5
    steps:
      - uses: actions/checkout@v4
        with:
          lfs: true
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
        with:
          shared-key: test-rust-embed
      - name: Install nextest
        uses: taiki-e/install-action@nextest
      - name: Run migrations
        run: cargo run -p temper-cli --bin temper-migrate
        env:
          DATABASE_URL: postgresql://postgres:postgres@localhost:5432/temper_test
      - name: Run markdown-path service tests
        run: |
          cargo nextest run -p temper-api \
            --features "test-db,ingest-pipeline" \
            --test-threads 1 \
            tests_markdown_path tests_ingest_error tests_validate_managed_meta \
            tests_strip_system_managed_fields
        env:
          DATABASE_URL: postgresql://postgres:postgres@localhost:5432/temper_test
          SQLX_OFFLINE: "false"
      - name: Run MCP round-trip e2e tests
        run: |
          cargo nextest run -p temper-e2e \
            --features "test-db,ingest-pipeline" \
            --test-threads 1 \
            mcp_round_trip
        env:
          DATABASE_URL: postgresql://postgres:postgres@localhost:5432/temper_test
          SQLX_OFFLINE: "false"
```

**Important:** adapt the migration command and service container setup to match what the existing `test-rust` job uses. Do not invent a new pattern.

- [ ] **Step 3: Add the job to `ci-success.yml` merge gate**

```bash
grep -n "needs:" .github/workflows/ci-success.yml
```

Add `test-rust-embed` to the list of required jobs.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/test-rust.yml .github/workflows/ci-success.yml
git commit -m "ci: add test-rust-embed job for round-trip tests with LFS model"
```

- [ ] **Step 5: Push and verify the new job runs**

```bash
git push
```

Monitor the GitHub Actions run. The new job should appear in the workflow and pass.

---

## Task 34: Delete `api/content-ingest.ts`

**Files:**
- Delete: `api/content-ingest.ts`

- [ ] **Step 1: Verify no Rust code references it**

```bash
grep -rn "content-ingest" crates/ api/ packages/
```

Expected: references only in TS tests (to be deleted) and in `api/content-ingest.ts` itself.

- [ ] **Step 2: Delete the file**

```bash
rm api/content-ingest.ts
```

- [ ] **Step 3: Update routing config if it mentions the path**

```bash
grep -n "content-ingest" vercel.json vercel.ts 2>/dev/null
```

If any route pattern references `/api/content-ingest`, remove it.

- [ ] **Step 4: Run TypeScript typecheck**

```bash
cd packages/temper-cloud && bun run typecheck
cd ../..
```

Expected: passes. Any errors indicate lingering references to be cleaned up.

- [ ] **Step 5: Commit**

```bash
git add api/content-ingest.ts vercel.json vercel.ts 2>/dev/null
git commit -m "chore(api): delete api/content-ingest.ts proxy endpoint"
```

---

## Task 35: Delete `api/workflows/process-content-ingest.ts`

- [ ] **Step 1: Delete the file**

```bash
rm api/workflows/process-content-ingest.ts
```

- [ ] **Step 2: Find and delete its tests**

```bash
find packages/temper-cloud/tests -name "*content-ingest*"
```

Delete any matching test files.

- [ ] **Step 3: Run TS tests**

```bash
cd packages/temper-cloud && bun run test
cd ../..
```

Expected: all remaining tests pass.

- [ ] **Step 4: Commit**

```bash
git add api/workflows/process-content-ingest.ts packages/temper-cloud/tests/
git commit -m "chore(workflows): delete process-content-ingest workflow"
```

---

## Task 36: Delete `packages/temper-cloud/src/processing/embed.ts`

- [ ] **Step 1: Delete**

```bash
rm packages/temper-cloud/src/processing/embed.ts
```

- [ ] **Step 2: Find and delete tests**

```bash
find packages/temper-cloud/tests -name "*embed*"
```

- [ ] **Step 3: Check for remaining imports**

```bash
grep -rn "processing/embed" packages/temper-cloud/
```

- [ ] **Step 4: Typecheck and test**

```bash
cd packages/temper-cloud && bun run typecheck && bun run test
cd ../..
```

- [ ] **Step 5: Commit**

```bash
git add packages/temper-cloud/
git commit -m "chore(temper-cloud): delete TS-side embedding pipeline"
```

---

## Task 37: Delete `packages/temper-cloud/src/workflow/chunk.ts`

- [ ] **Step 1: Delete**

```bash
rm packages/temper-cloud/src/workflow/chunk.ts
```

- [ ] **Step 2: Find and delete tests + imports**

```bash
find packages/temper-cloud/tests -name "*chunk*"
grep -rn "workflow/chunk" packages/temper-cloud/
```

- [ ] **Step 3: Typecheck, test, commit**

```bash
cd packages/temper-cloud && bun run typecheck && bun run test
cd ../..
git add packages/temper-cloud/
git commit -m "chore(temper-cloud): delete TS-side chunker"
```

---

## Task 38: Review `packages/temper-cloud/src/workflow/store.ts`

**Decision task:** determine if `store.ts` is content-ingest-specific (delete) or shared with the upload path (refactor).

- [ ] **Step 1: Read the file**

```bash
cat packages/temper-cloud/src/workflow/store.ts
```

- [ ] **Step 2: Find its callers**

```bash
grep -rn "workflow/store" packages/temper-cloud/ api/
```

**Decision point:**
- Only `process-content-ingest.ts` (deleted) uses it → delete `store.ts`.
- `process-upload.ts` (still alive for binary Vercel Blob path) also uses it → keep `store.ts` but verify it still compiles and tests pass.
- Shared → leave it, note in commit message.

- [ ] **Step 3: Execute the decision**

Either delete the file or leave it. If deleted:

```bash
rm packages/temper-cloud/src/workflow/store.ts
git add packages/temper-cloud/
git commit -m "chore(temper-cloud): delete workflow/store.ts (only used by deleted content-ingest path)"
```

If kept:

```bash
cd packages/temper-cloud && bun run typecheck && bun run test
cd ../..
# no commit needed unless edits were required
```

---

## Task 39: Remove `ContentIngestRequest` from ts-rs generated types

**Files:**
- Modify: `packages/temper-ui/src/lib/types/ingest.ts` (or wherever generated types live)
- Verify: `crates/temper-core/src/types/ingest.rs` (already deleted in Task 26)

- [ ] **Step 1: Regenerate TS types from Rust**

```bash
cargo make generate-ts-types
```

- [ ] **Step 2: Verify `ContentIngestRequest` is gone from generated files**

```bash
grep -rn "ContentIngestRequest" packages/temper-ui/src packages/temper-cloud/src
```

Expected: no matches.

- [ ] **Step 3: Commit regenerated files**

```bash
git add packages/temper-ui/src packages/temper-cloud/src
git commit -m "chore: regenerate TS types after ContentIngestRequest removal"
```

---

## Task 40: Update `agent-skills/knowledge-base.md`

**Files:**
- Modify: `agent-skills/knowledge-base.md`

- [ ] **Step 1: Read current content**

```bash
cat agent-skills/knowledge-base.md
```

- [ ] **Step 2: Remove sections describing the two-step shell-and-upload workflow**

Identify and delete any sections that describe:
- "Create resource shell, then upload content" pattern
- Polling for `body_processed` events
- The HTTP POST to `/api/content-ingest` or `/api/upload` for markdown content

- [ ] **Step 3: Add a "Writing Content" section using the unified path**

```markdown
## Writing Content

Use the `create_resource` MCP tool directly with both metadata and content:

1. (Optional) Call `list_doc_types` to see available types and which have schemas.
2. (Optional) Call `describe_doc_type` for a specific type to get the JSON Schema and a usable `example_managed_meta` template.
3. Call `create_resource` with:
   - `context_name` — the context to write into
   - `doc_type_name` — the type of resource (task, session, concept, etc.)
   - `title` — human-readable title
   - `slug` — URL-safe identifier
   - `content` — full markdown body
   - `managed_meta` — doc-type-specific frontmatter fields (optional for types with no required fields)
   - `open_meta` — free-form user fields (optional)

The server validates `managed_meta` against the schema, runs chunking and embedding inline, and returns the fully processed resource in a single call. No polling, no intermediate state.

If validation fails, the tool returns a structured error listing each issue with its field name. Fix the fields and retry.
```

- [ ] **Step 4: Commit**

```bash
git add agent-skills/knowledge-base.md
git commit -m "docs(agent-skills): document unified create_resource path with managed_meta"
```

---

## Task 41: Update `agent-skills/claude-desktop.md`

- [ ] **Step 1: Read and update**

Apply similar updates as Task 40 to `claude-desktop.md`. Remove references to the upload endpoint URL for markdown content and point agents at the MCP tool directly.

- [ ] **Step 2: Commit**

```bash
git add agent-skills/claude-desktop.md
git commit -m "docs(agent-skills): update Claude Desktop guide for unified content path"
```

---

## Task 42: Verify `agent-skills/SKILL.md` accuracy

- [ ] **Step 1: Read `SKILL.md`**

- [ ] **Step 2: Update any references to content-ingest / two-step shell workflow**

- [ ] **Step 3: Commit any changes**

```bash
git add agent-skills/SKILL.md
git commit -m "docs(agent-skills): sync SKILL.md with unified ingest path"
```

---

## Task 43: Full session-3 test and lint sweep

- [ ] **Step 1: Run all Rust tests with all features**

```bash
cargo nextest run --workspace --features "test-db,ingest-pipeline"
```

- [ ] **Step 2: Run all TS tests**

```bash
cd packages/temper-cloud && bun run test && bun run test:integration
cd ../..
```

- [ ] **Step 3: Run cargo make check**

```bash
cargo make check
```

- [ ] **Step 4: Verify no stale references**

```bash
grep -rn "content-ingest" . 2>/dev/null | grep -v "\.git" | grep -v "node_modules" | grep -v "target" | grep -v "docs/superpowers"
```

Expected: only matches in `docs/superpowers/specs/` and `docs/superpowers/plans/` (historical references).

```bash
grep -rn "ContentIngestRequest" . 2>/dev/null | grep -v "\.git" | grep -v "node_modules" | grep -v "target" | grep -v "docs/superpowers"
```

Expected: no matches outside docs.

---

## Task 44: Final deploy smoke test

- [ ] **Step 1: Push the branch**

```bash
git push
```

- [ ] **Step 2: Wait for Vercel preview and run the curl smoke tests from Task 20**

Re-run the smoke test from Task 20 against the new preview URL. Additionally:

```bash
# Verify describe_doc_type works via MCP
# (Use the MCP client directly or via `claude mcp call`)
```

- [ ] **Step 3: Verify a full MCP round-trip via Claude Desktop or claude.ai**

- Connect the MCP server
- Call `list_doc_types`
- Call `describe_doc_type` for `concept`
- Call `create_resource` with the example frontmatter and a test markdown body
- Call `search` and confirm the new resource appears

- [ ] **Step 4: Open the PR if not already open**

```bash
gh pr create --title "feat: unified Rust ingest pipeline + MCP round-trip" \
  --body "$(cat <<'EOF'
## Summary

- Collapses the two-path ingest architecture into a single synchronous Rust request-response pipeline
- Bundles `BAAI/bge-base-en-v1.5` quantized ONNX via `include_bytes!` + git-LFS
- New `temper_ingest::pipeline::prepare_markdown` shared function used by CLI and API
- MCP tools gain schema awareness: `describe_doc_type`, `managed_meta`/`open_meta` on create/update
- Retires the TypeScript `/api/content-ingest` proxy path entirely
- Adds service-layer + e2e round-trip tests with a new `test-rust-embed` CI job

## Test plan

- [x] Service-layer tests with real embedding pass
- [x] MCP round-trip e2e tests pass
- [x] CLI precomputed path unchanged
- [x] Vercel preview cold-start smoke test <3s
- [x] Cargo make check passes

Closes task 2026-04-09-temper-mcp-bug-fixes-and-round-trip-testing.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Plan Complete

After Task 44, the full pivot is live: single synchronous markdown ingest path, MCP schema awareness, and retired TypeScript proxy. Deferred work (structural moves via MCP, blob-temp model cache) is documented in the spec for future follow-up tasks.
