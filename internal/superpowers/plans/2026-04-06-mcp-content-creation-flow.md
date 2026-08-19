# MCP Content Creation Flow — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **REQUIRED READING:** Before implementing any task, read `/Users/petetaylor/.claude/skills/temper/subagent-guidance.md` and apply all 10 SG principles throughout. Key principles for this plan:
> - **SG-1**: Read the file AND a sibling in the same module before writing anything
> - **SG-2**: Each function does one thing — follow the existing service layer patterns
> - **SG-4**: Unit tests co-located, integration tests separate, one behavior per test
> - **SG-5**: Implement exactly what the task says — no speculative extras
> - **SG-6**: Run verification commands, read output, report results
> - **SG-8**: Check existing abstractions before proposing anything new
> - **SG-10**: Checkpoint after each major step
>
> **Project fundamentals:** Read `/Users/petetaylor/.claude/skills/temper/guidance/fundamentals.md` for crate graph, code quality principles, and testing conventions.

**Goal:** Enable MCP-connected agents to write markdown content to the Temper knowledge base through a single tool call, with async chunk/embed/store processing via a new TypeScript endpoint.

**Architecture:** The Rust MCP `ingest_content` tool creates the resource shell (metadata + manifest + event) and POSTs the markdown content to a new TypeScript endpoint `/api/content-ingest`. That endpoint triggers a Vercel Workflow that chunks, embeds, and stores the content. The tool returns the resource_id immediately without waiting for processing.

**Tech Stack:** Rust (rmcp, sqlx, sha2, serde, schemars), TypeScript (Vercel Workflows, neon serverless postgres, ONNX embeddings), Axum (existing API services)

**Spec:** `docs/superpowers/specs/2026-04-06-mcp-content-creation-flow-design.md`

---

## File Structure

### New Files

| File | Responsibility |
|------|---------------|
| `crates/temper-mcp/src/tools/doc_types.rs` | `list_doc_types` MCP tool handler |
| `crates/temper-mcp/src/tools/ingest.rs` | `ingest_content` and `update_resource_content` MCP tool handlers |
| `crates/temper-api/src/services/doc_type_service.rs` | Doc type query (list all) |
| `api/content-ingest.ts` | Vercel Function entry point for content ingestion |
| `packages/temper-cloud/src/content-ingest.ts` | Business logic: validate + trigger workflow |
| `api/workflows/process-content-ingest.ts` | Vercel Workflow: chunk → embed → store |
| `tests/e2e/tests/doc_type_test.rs` | E2e tests for list_doc_types |
| `tests/e2e/tests/mcp_ingest_test.rs` | E2e tests for ingest_content and update_resource_content |
| `packages/temper-cloud/tests/content-ingest.test.ts` | TS unit tests for content-ingest endpoint |

### Modified Files

| File | Change |
|------|--------|
| `crates/temper-mcp/src/tools/mod.rs` | Add `pub mod doc_types;` and `pub mod ingest;` |
| `crates/temper-mcp/src/service.rs` | Register 3 new tool methods |
| `crates/temper-api/src/services/mod.rs` | Add `pub mod doc_type_service;` |
| `crates/temper-api/src/services/ingest_service.rs` | Make `resolve_doc_type` and `find_by_body_hash` pub; extract `create_resource_with_manifest` |
| `vercel.json` | Add `/api/content-ingest` route before Axum catch-all |
| `agent-skills/knowledge-base.md` | Add "Writing Content" section with `ingest_content` workflow |
| `agent-skills/claude-desktop.md` | Add content creation workflow section |
| `agent-skills/SKILL.md` | Update capability summary |

---

### Task 1: `doc_type_service` — Service Layer for Doc Type Queries

**Files:**
- Create: `crates/temper-api/src/services/doc_type_service.rs`
- Modify: `crates/temper-api/src/services/mod.rs`

- [ ] **Step 1: Read existing service patterns**

Read `crates/temper-api/src/services/context_service.rs` and `crates/temper-api/src/services/mod.rs` to understand the service module pattern.

- [ ] **Step 2: Create the doc type service**

Create `crates/temper-api/src/services/doc_type_service.rs`:

```rust
//! Doc type service — query system-level document types.

use sqlx::PgPool;
use crate::error::ApiResult;

/// A document type row from kb_doc_types.
#[derive(Debug, serde::Serialize, sqlx::FromRow)]
pub struct DocTypeRow {
    pub id: uuid::Uuid,
    pub name: String,
    pub description: Option<String>,
}

/// List all system-level document types.
pub async fn list_all(pool: &PgPool) -> ApiResult<Vec<DocTypeRow>> {
    let rows = sqlx::query_as!(
        DocTypeRow,
        r#"SELECT id, name, description FROM kb_doc_types ORDER BY name"#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows)
}
```

- [ ] **Step 3: Register the module**

In `crates/temper-api/src/services/mod.rs`, add:

```rust
pub mod doc_type_service;
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo make check`
Expected: No errors from the new service module.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-api/src/services/doc_type_service.rs crates/temper-api/src/services/mod.rs
git commit -m "feat: add doc_type_service for listing system document types"
```

---

### Task 2: `list_doc_types` MCP Tool

**Files:**
- Create: `crates/temper-mcp/src/tools/doc_types.rs`
- Modify: `crates/temper-mcp/src/tools/mod.rs`
- Modify: `crates/temper-mcp/src/service.rs`

- [ ] **Step 1: Read existing MCP tool patterns**

Read `crates/temper-mcp/src/tools/contexts.rs` (simplest existing tool) and `crates/temper-mcp/src/service.rs` to match the tool registration pattern.

- [ ] **Step 2: Create the doc_types tool handler**

Create `crates/temper-mcp/src/tools/doc_types.rs`:

```rust
//! Doc type tools — list available document types.

use rmcp::model::CallToolResult;

use crate::service::TemperMcpService;

pub async fn list_doc_types(svc: &TemperMcpService) -> Result<CallToolResult, rmcp::ErrorData> {
    let _profile = svc.require_profile().await?;

    let rows = temper_api::services::doc_type_service::list_all(&svc.api_state.pool)
        .await
        .map_err(|e| {
            rmcp::ErrorData::internal_error(format!("Failed to list doc types: {e}"), None)
        })?;

    let text = serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        text,
    )]))
}
```

- [ ] **Step 3: Register the module in tools/mod.rs**

Add to `crates/temper-mcp/src/tools/mod.rs`:

```rust
pub mod doc_types;
```

- [ ] **Step 4: Register the tool method in service.rs**

In `crates/temper-mcp/src/service.rs`, add inside the `#[tool_router] impl TemperMcpService` block, after the existing `get_profile` tool:

```rust
    #[tool(
        description = "List all available document types in the knowledge base. Returns id, name, and description for each type. Use these when creating resources to specify the correct doc_type_name."
    )]
    async fn list_doc_types(
        &self,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::doc_types::list_doc_types(self).await
    }
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo make check`
Expected: No errors.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-mcp/src/tools/doc_types.rs crates/temper-mcp/src/tools/mod.rs crates/temper-mcp/src/service.rs
git commit -m "feat: add list_doc_types MCP tool"
```

---

### Task 3: Refactor `ingest_service` — Extract Shared Functions

**Files:**
- Modify: `crates/temper-api/src/services/ingest_service.rs`

- [ ] **Step 1: Read the full ingest_service.rs**

Read `crates/temper-api/src/services/ingest_service.rs` to understand what needs to change.

- [ ] **Step 2: Make `resolve_doc_type` public**

Change the function signature from:

```rust
async fn resolve_doc_type(pool: &PgPool, name: &str) -> ApiResult<Uuid> {
```

to:

```rust
pub async fn resolve_doc_type(pool: &PgPool, name: &str) -> ApiResult<Uuid> {
```

- [ ] **Step 3: Make `find_by_body_hash` public**

Change the function signature from:

```rust
async fn find_by_body_hash(
    pool: &PgPool,
    profile_id: ProfileId,
    body_hash: &str,
) -> ApiResult<Option<ResourceRow>> {
```

to:

```rust
pub async fn find_by_body_hash(
    pool: &PgPool,
    profile_id: ProfileId,
    body_hash: &str,
) -> ApiResult<Option<ResourceRow>> {
```

- [ ] **Step 4: Extract `create_resource_with_manifest`**

Add a new public function that encapsulates the resource + manifest + event creation from the `ingest()` function. This is the transaction body from steps 6-8 of the existing `ingest()`, minus the chunk insertion:

```rust
/// Create a resource with its manifest and event trail in a single transaction.
/// Does NOT insert chunks — that's handled separately by the caller or an async workflow.
#[expect(
    clippy::too_many_arguments,
    reason = "resource creation requires all metadata fields"
)]
pub async fn create_resource_with_manifest(
    pool: &PgPool,
    profile_id: ProfileId,
    device_id: &str,
    context_id: ContextId,
    doc_type_id: uuid::Uuid,
    title: &str,
    slug: Option<&str>,
    origin_uri: &str,
    content_hash: &str,
    managed_meta: &serde_json::Value,
    open_meta: &serde_json::Value,
) -> ApiResult<ResourceRow> {
    let managed_hash = hash_json_value(managed_meta);
    let open_hash = hash_json_value(open_meta);

    let mut tx = pool.begin().await?;

    let resource_id = ResourceId::new();
    let resource = sqlx::query_as!(
        ResourceRow,
        r#"
        INSERT INTO kb_resources (
            id, kb_context_id, kb_doc_type_id, origin_uri, title, slug,
            originator_profile_id, owner_profile_id,
            created, updated
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now(), now())
        RETURNING id, kb_context_id, kb_doc_type_id, origin_uri, title,
                  slug as "slug: _",
                  originator_profile_id, owner_profile_id, is_active,
                  created, updated
        "#,
        *resource_id,
        *context_id,
        doc_type_id,
        origin_uri,
        title,
        slug,
        *profile_id,
        *profile_id,
    )
    .fetch_one(&mut *tx)
    .await?;

    sqlx::query!(
        r#"
        INSERT INTO kb_resource_manifests (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
        VALUES ($1, $2, $3, $4, $5, $6, now())
        "#,
        *resource_id,
        content_hash,
        managed_meta,
        open_meta,
        managed_hash,
        open_hash,
    )
    .execute(&mut *tx)
    .await?;

    insert_event_and_audit(
        &mut tx,
        profile_id,
        device_id,
        context_id,
        resource_id,
        "resource_created",
        "create",
        content_hash,
        &managed_hash,
        &open_hash,
    )
    .await?;

    tx.commit().await?;

    Ok(resource)
}
```

- [ ] **Step 5: Refactor `ingest()` to use `create_resource_with_manifest`**

Replace the transaction body in `ingest()` (steps 6-8) with a call to the extracted function, then add the chunk insertion in a separate transaction:

```rust
pub async fn ingest(
    pool: &PgPool,
    profile_id: ProfileId,
    device_id: &str,
    payload: IngestPayload,
) -> ApiResult<ResourceRow> {
    // 1. Resolve context
    let context = context_service::resolve_by_name(pool, profile_id, &payload.context_name).await?;
    let context_id = context.id;

    // 2. Resolve doc_type
    let doc_type_id = resolve_doc_type(pool, &payload.doc_type_name).await?;

    // 3. Body-hash dedup
    if let Some(existing) = find_by_body_hash(pool, profile_id, &payload.content_hash).await? {
        return Ok(existing);
    }

    // 4. Decode chunks
    let chunks = unpack_chunks(&payload.chunks_packed)
        .map_err(|e| ApiError::BadRequest(format!("invalid chunks_packed: {e}")))?;

    // 5. Compute meta
    let empty_json = serde_json::json!({});
    let managed_meta = payload.managed_meta.clone().unwrap_or_else(|| empty_json.clone());
    let open_meta = payload.open_meta.clone().unwrap_or_else(|| empty_json.clone());

    // 6. Create resource + manifest + event
    let resource = create_resource_with_manifest(
        pool,
        profile_id,
        device_id,
        context_id,
        doc_type_id,
        &payload.title,
        Some(&payload.slug),
        &payload.origin_uri,
        &payload.content_hash,
        &managed_meta,
        &open_meta,
    )
    .await?;

    // 7. Insert chunks in a separate transaction
    if !chunks.is_empty() {
        let mut tx = pool.begin().await?;
        persist_chunks(&mut tx, ResourceId::from(resource.id), &chunks).await?;
        tx.commit().await?;
    }

    Ok(resource)
}
```

- [ ] **Step 6: Run existing tests to verify no regressions**

Run: `cargo make test`
Expected: All unit tests pass (including `hash_empty_object`, `hash_key_order_independent`, `hash_json_shared_fixture`).

Run: `cargo make check`
Expected: No clippy or format warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-api/src/services/ingest_service.rs
git commit -m "refactor: extract create_resource_with_manifest from ingest_service"
```

---

### Task 4: `ingest_content` MCP Tool

**Files:**
- Create: `crates/temper-mcp/src/tools/ingest.rs`
- Modify: `crates/temper-mcp/src/tools/mod.rs`
- Modify: `crates/temper-mcp/src/service.rs`

- [ ] **Step 1: Read existing tool patterns**

Read `crates/temper-mcp/src/tools/resources.rs` (the most complex existing tool file) and `crates/temper-mcp/src/service.rs` to match patterns.

- [ ] **Step 2: Create the ingest tool handler**

Create `crates/temper-mcp/src/tools/ingest.rs`:

```rust
//! Ingest tools — create and update resource content in the knowledge base.

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use temper_core::types::ids::ProfileId;

use crate::service::TemperMcpService;

/// MCP input for ingest_content.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct IngestContentInput {
    /// Resource title.
    pub title: String,
    /// Markdown content body.
    pub content: String,
    /// Context name — resolved to UUID server-side. Auto-creates if missing.
    pub context_name: String,
    /// Document type name (e.g. "research", "session"). Use list_doc_types to discover valid names.
    pub doc_type_name: String,
    /// Optional URL-friendly slug. Auto-generated from title if omitted.
    pub slug: Option<String>,
    /// Optional origin URI. Defaults to mcp://agent/<resource_id>.
    pub origin_uri: Option<String>,
}

/// MCP input for update_resource_content.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateResourceContentInput {
    /// UUID of the existing resource to update.
    pub resource_id: Uuid,
    /// New markdown content body.
    pub content: String,
}

/// Response returned to the agent after content ingestion.
#[derive(Debug, Serialize)]
struct IngestResponse {
    resource_id: Uuid,
    title: String,
    context_name: String,
    status: &'static str,
}

/// Compute `sha256:<hex>` of raw content bytes.
fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// POST the content to the TypeScript content-ingest endpoint for async processing.
async fn trigger_content_processing(
    base_url: &str,
    token: &str,
    resource_id: Uuid,
    content: &str,
    replace: bool,
) -> Result<(), String> {
    let url = format!("{base_url}/api/content-ingest");
    let body = serde_json::json!({
        "resource_id": resource_id.to_string(),
        "content": content,
        "replace": replace,
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(&url)
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Failed to call content-ingest: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("content-ingest returned {status}: {text}"));
    }

    Ok(())
}

/// Extract the bearer token from the HTTP request parts.
fn extract_token(parts: &http::request::Parts) -> Option<String> {
    parts
        .headers
        .get(http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(|s| s.to_string())
}

pub async fn ingest_content(
    svc: &TemperMcpService,
    input: IngestContentInput,
    parts: &http::request::Parts,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);

    // 1. Resolve context (auto-creates if missing)
    let context = temper_api::services::context_service::resolve_by_name(
        pool,
        profile_id,
        &input.context_name,
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to resolve context: {e}"), None))?;

    // 2. Resolve doc type (errors if not found)
    let doc_type_id = temper_api::services::ingest_service::resolve_doc_type(
        pool,
        &input.doc_type_name,
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Unknown doc type: {e}"), None))?;

    // 3. Compute body hash and check for dedup
    let body_hash = content_hash(&input.content);
    if let Some(existing) = temper_api::services::ingest_service::find_by_body_hash(
        pool,
        profile_id,
        &body_hash,
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Dedup check failed: {e}"), None))?
    {
        let resp = IngestResponse {
            resource_id: existing.id,
            title: existing.title,
            context_name: input.context_name,
            status: "existing",
        };
        let text = serde_json::to_string_pretty(&resp).unwrap_or_else(|_| "{}".to_string());
        return Ok(CallToolResult::success(vec![rmcp::model::Content::text(text)]));
    }

    // 4. Create resource + manifest + event
    let origin_uri = input
        .origin_uri
        .unwrap_or_else(|| format!("mcp://agent/{}", uuid::Uuid::new_v4()));
    let empty_json = serde_json::json!({});

    let resource = temper_api::services::ingest_service::create_resource_with_manifest(
        pool,
        profile_id,
        "mcp",
        context.id,
        doc_type_id,
        &input.title,
        input.slug.as_deref(),
        &origin_uri,
        &body_hash,
        &empty_json,
        &empty_json,
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to create resource: {e}"), None))?;

    // 5. Trigger async content processing (fire and forget)
    let token = extract_token(parts).unwrap_or_default();
    let base_url = &svc.api_state.config.base_url();
    if let Err(e) = trigger_content_processing(
        base_url,
        &token,
        resource.id,
        &input.content,
        false,
    )
    .await
    {
        tracing::warn!(resource_id = %resource.id, "Content processing trigger failed: {e}");
    }

    // 6. Return immediately
    let resp = IngestResponse {
        resource_id: resource.id,
        title: resource.title,
        context_name: input.context_name,
        status: "created",
    };
    let text = serde_json::to_string_pretty(&resp).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(text)]))
}

pub async fn update_resource_content(
    svc: &TemperMcpService,
    input: UpdateResourceContentInput,
    parts: &http::request::Parts,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let profile_id = ProfileId::from(profile.id);
    let resource_id = temper_core::types::ids::ResourceId::from(input.resource_id);

    // 1. Verify ownership
    let can_modify = sqlx::query_scalar!(
        "SELECT true FROM can_modify_resource($1, $2)",
        *profile_id,
        *resource_id,
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Auth check failed: {e}"), None))?;

    if can_modify.is_none() {
        return Err(rmcp::ErrorData::internal_error(
            "Resource not found or not authorized".to_string(),
            None,
        ));
    }

    // 2. Compute new body hash
    let body_hash = content_hash(&input.content);
    let empty_json = serde_json::json!({});
    let managed_hash = temper_api::services::ingest_service::hash_json_value(&empty_json);
    let open_hash = temper_api::services::ingest_service::hash_json_value(&empty_json);

    // 3. Get resource context_id for the event
    let resource = sqlx::query_as!(
        temper_core::types::resource::ResourceRow,
        r#"
        SELECT id, kb_context_id, kb_doc_type_id, origin_uri, title,
               slug as "slug: _",
               originator_profile_id, owner_profile_id, is_active,
               created, updated
        FROM kb_resources WHERE id = $1
        "#,
        *resource_id,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Resource lookup failed: {e}"), None))?;

    // 4. Update manifest + event in transaction
    let mut tx = pool.begin().await.map_err(|e| {
        rmcp::ErrorData::internal_error(format!("Transaction start failed: {e}"), None)
    })?;

    sqlx::query!(
        r#"
        INSERT INTO kb_resource_manifests (resource_id, body_hash, managed_meta, open_meta, managed_hash, open_hash, updated)
        VALUES ($1, $2, $3, $4, $5, $6, now())
        ON CONFLICT (resource_id)
        DO UPDATE SET body_hash = $2, managed_meta = $3, open_meta = $4,
                      managed_hash = $5, open_hash = $6, updated = now()
        "#,
        *resource_id,
        body_hash,
        empty_json,
        empty_json,
        managed_hash,
        open_hash,
    )
    .execute(&mut *tx)
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Manifest update failed: {e}"), None))?;

    sqlx::query!("UPDATE kb_resources SET updated = now() WHERE id = $1", *resource_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Resource update failed: {e}"), None))?;

    let context_id = temper_core::types::ids::ContextId::from(resource.kb_context_id);
    temper_api::services::ingest_service::insert_event_and_audit(
        &mut tx,
        profile_id,
        "mcp",
        context_id,
        resource_id,
        "body_updated",
        "update_body",
        &body_hash,
        &managed_hash,
        &open_hash,
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Event creation failed: {e}"), None))?;

    tx.commit().await.map_err(|e| {
        rmcp::ErrorData::internal_error(format!("Transaction commit failed: {e}"), None)
    })?;

    // 5. Trigger async content processing with replace=true
    let token = extract_token(parts).unwrap_or_default();
    let base_url = &svc.api_state.config.base_url();
    if let Err(e) = trigger_content_processing(
        base_url,
        &token,
        input.resource_id,
        &input.content,
        true,
    )
    .await
    {
        tracing::warn!(resource_id = %input.resource_id, "Content processing trigger failed: {e}");
    }

    let resp = serde_json::json!({
        "resource_id": input.resource_id,
        "status": "processing",
    });
    let text = serde_json::to_string_pretty(&resp).unwrap_or_else(|_| "{}".to_string());
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(text)]))
}
```

**Important:** The `base_url()` method may not exist on `ApiConfig`. Check `crates/temper-api/src/config.rs` — if it doesn't exist, the MCP tool needs to get the base URL another way. The MCP config has `mcp_base_url` which points to `https://temperkb.io`. Since `/api/content-ingest` is on the same deployment, use `svc.api_state.config` or fall back to the MCP config's `mcp_base_url`. Read both config files to determine the right source.

- [ ] **Step 3: Register the module in tools/mod.rs**

Add to `crates/temper-mcp/src/tools/mod.rs`:

```rust
pub mod ingest;
```

- [ ] **Step 4: Register tool methods in service.rs**

In `crates/temper-mcp/src/service.rs`, add inside the `#[tool_router] impl TemperMcpService` block:

```rust
    #[tool(
        description = "Ingest markdown content into the knowledge base. Creates a new resource with the given content. The content is processed asynchronously (chunked, embedded, stored) and becomes searchable shortly after creation. Use list_doc_types to discover valid doc_type_name values. Use list_contexts to check existing contexts before creating new ones."
    )]
    async fn ingest_content(
        &self,
        Parameters(input): Parameters<tools::ingest::IngestContentInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::ingest::ingest_content(self, input, &parts).await
    }

    #[tool(
        description = "Update the content of an existing resource. Replaces the current content with new markdown. The updated content is processed asynchronously. You must have write access to the resource."
    )]
    async fn update_resource_content(
        &self,
        Parameters(input): Parameters<tools::ingest::UpdateResourceContentInput>,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<CallToolResult, rmcp::ErrorData> {
        self.ensure_profile_from_parts(&parts).await?;
        tools::ingest::update_resource_content(self, input, &parts).await
    }
```

- [ ] **Step 5: Add reqwest dependency to temper-mcp**

Check `crates/temper-mcp/Cargo.toml` — if `reqwest` is not already a dependency, add it:

```toml
reqwest = { version = "0.12", features = ["json"] }
```

Also ensure `sha2` and `hex` are available (they may already be transitive from temper-api).

- [ ] **Step 6: Verify it compiles**

Run: `cargo make check`
Expected: No errors. Address any missing imports, type mismatches, or config access issues.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-mcp/src/tools/ingest.rs crates/temper-mcp/src/tools/mod.rs crates/temper-mcp/src/service.rs crates/temper-mcp/Cargo.toml
git commit -m "feat: add ingest_content and update_resource_content MCP tools"
```

---

### Task 5: TypeScript Content-Ingest Endpoint

**Files:**
- Create: `packages/temper-cloud/src/content-ingest.ts`
- Create: `api/content-ingest.ts`

- [ ] **Step 1: Read existing endpoint patterns**

Read `api/upload.ts` and `packages/temper-cloud/src/middleware.ts` to match the auth + business logic pattern.

- [ ] **Step 2: Create the business logic module**

Create `packages/temper-cloud/src/content-ingest.ts`:

```typescript
import { z } from "zod";

export const ContentIngestSchema = z.object({
  resource_id: z.string().uuid(),
  content: z.string().min(1),
  replace: z.boolean(),
});

export type ContentIngestPayload = z.infer<typeof ContentIngestSchema>;

/**
 * Validate the content-ingest request body.
 * Returns the parsed payload or null with an error message.
 */
export function validatePayload(
  body: unknown,
): { ok: true; payload: ContentIngestPayload } | { ok: false; error: string } {
  const result = ContentIngestSchema.safeParse(body);
  if (!result.success) {
    return { ok: false, error: result.error.issues.map((i) => i.message).join(", ") };
  }
  return { ok: true, payload: result.data };
}
```

- [ ] **Step 3: Create the Vercel Function entry point**

Create `api/content-ingest.ts`:

```typescript
export const config = { runtime: "nodejs" };

export async function POST(req: Request): Promise<Response> {
  const { authenticateRequest } = await import(
    "../packages/temper-cloud/src/middleware.js"
  );
  const { validatePayload } = await import(
    "../packages/temper-cloud/src/content-ingest.js"
  );
  const { processContentIngest } = await import(
    "./workflows/process-content-ingest.js"
  );

  // Authenticate
  const auth = await authenticateRequest(req);
  if (!auth.ok) return auth.response;

  // Parse and validate body
  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return new Response(
      JSON.stringify({ error: "Invalid JSON body" }),
      { status: 400, headers: { "Content-Type": "application/json" } },
    );
  }

  const validation = validatePayload(body);
  if (!validation.ok) {
    return new Response(
      JSON.stringify({ error: validation.error }),
      { status: 400, headers: { "Content-Type": "application/json" } },
    );
  }

  const { resource_id, content, replace } = validation.payload;

  // Verify the caller can access this resource
  const visibleResources = await auth.db`
    SELECT resource_id FROM resources_visible_to(${auth.profileId}::uuid)
    WHERE resource_id = ${resource_id}::uuid
  `;
  if (visibleResources.length === 0) {
    return new Response(
      JSON.stringify({ error: "Resource not found or not accessible" }),
      { status: 404, headers: { "Content-Type": "application/json" } },
    );
  }

  // Trigger the processing workflow
  try {
    await processContentIngest(resource_id, content, replace, auth.profileId);
  } catch (err) {
    console.error("Failed to trigger content processing workflow:", err);
  }

  return new Response(
    JSON.stringify({ resource_id, status: "processing" }),
    { status: 202, headers: { "Content-Type": "application/json" } },
  );
}
```

- [ ] **Step 4: Verify TypeScript compiles**

Run: `cd packages/temper-cloud && bun run typecheck`
Expected: No type errors.

- [ ] **Step 5: Commit**

```bash
git add packages/temper-cloud/src/content-ingest.ts api/content-ingest.ts
git commit -m "feat: add /api/content-ingest TypeScript endpoint"
```

---

### Task 6: Vercel Workflow for Content Processing

**Files:**
- Create: `api/workflows/process-content-ingest.ts`

- [ ] **Step 1: Read the existing workflow pattern**

Read `api/workflows/process-upload.ts` to match the `"use workflow"` / `"use step"` pattern exactly.

- [ ] **Step 2: Create the workflow**

Create `api/workflows/process-content-ingest.ts`:

```typescript
import { chunkText } from "../../packages/temper-cloud/src/workflow/chunk.js";
import { embedTexts } from "../../packages/temper-cloud/src/workflow/embed.js";
import {
  chunksToJsonb,
  type ChunkRow,
} from "../../packages/temper-cloud/src/workflow/store.js";
import { getDb } from "../../packages/temper-cloud/src/db.js";
import { canonicalJsonHash } from "../../packages/temper-cloud/src/hash.js";
import { DEVICE_ID_CLOUD, insertEventAndAudit } from "../../packages/temper-cloud/src/events.js";

export async function processContentIngest(
  resourceId: string,
  content: string,
  replace: boolean,
  profileId: string,
) {
  "use workflow";

  console.log(`[content-ingest] Starting processing for resource ${resourceId}, replace=${replace}`);
  const chunks = await chunkStep(content);
  const embeddings = await embedStep(chunks.map((c) => c.content));
  await storeStep(resourceId, chunks, embeddings, replace, profileId);
}

async function chunkStep(
  text: string,
): Promise<
  Array<{
    header_path: string;
    content: string;
    content_hash: string;
    chunk_index: number;
  }>
> {
  "use step";
  console.log(`[content-ingest:chunk] Chunking ${text.length} chars`);
  const chunks = chunkText(text);
  console.log(`[content-ingest:chunk] Produced ${chunks.length} chunks`);
  return chunks;
}

async function embedStep(texts: string[]): Promise<number[][]> {
  "use step";
  console.log(`[content-ingest:embed] Embedding ${texts.length} chunks`);
  const embeddings = await embedTexts(texts);
  console.log(`[content-ingest:embed] Done`);
  return embeddings;
}

async function storeStep(
  resourceId: string,
  chunks: Array<{
    header_path: string;
    content: string;
    content_hash: string;
    chunk_index: number;
  }>,
  embeddings: number[][],
  replace: boolean,
  profileId: string,
): Promise<void> {
  "use step";

  console.log(`[content-ingest:store] Storing ${chunks.length} chunks for resource ${resourceId}, replace=${replace}`);
  const db = getDb();

  const chunkRows: ChunkRow[] = chunks.map((chunk, i) => ({
    id: "",
    resource_id: resourceId,
    chunk_index: chunk.chunk_index,
    version: 0,
    header_path: chunk.header_path,
    content: chunk.content,
    content_hash: chunk.content_hash,
    embedding: embeddings[i],
  }));

  const chunksJson = JSON.stringify(chunksToJsonb(chunkRows));

  if (replace) {
    await db`SELECT replace_resource_chunks(${resourceId}::uuid, ${chunksJson}::jsonb)`;
  } else {
    await db`SELECT persist_resource_chunks(${resourceId}::uuid, ${chunksJson}::jsonb)`;
  }

  // Fire body_processed event
  const contextRows = await db`
    SELECT kb_context_id FROM kb_resources WHERE id = ${resourceId}::uuid
  `;
  if (contextRows.length > 0) {
    const contextId = contextRows[0].kb_context_id as string;
    const emptyHash = canonicalJsonHash({});

    // Get current body hash from manifest
    const manifestRows = await db`
      SELECT body_hash FROM kb_resource_manifests WHERE resource_id = ${resourceId}::uuid
    `;
    const bodyHash = manifestRows.length > 0 ? (manifestRows[0].body_hash as string) : emptyHash;

    await insertEventAndAudit(db, {
      profileId,
      deviceId: DEVICE_ID_CLOUD,
      contextId,
      resourceId,
      eventType: "body_processed",
      action: "process_content",
      bodyHash,
      managedHash: emptyHash,
      openHash: emptyHash,
    });
  }
}
```

- [ ] **Step 3: Verify TypeScript compiles**

Run: `cd packages/temper-cloud && bun run typecheck`
Expected: No type errors.

- [ ] **Step 4: Commit**

```bash
git add api/workflows/process-content-ingest.ts
git commit -m "feat: add process-content-ingest Vercel Workflow"
```

---

### Task 7: Vercel Routing

**Files:**
- Modify: `vercel.json`

- [ ] **Step 1: Read current routing**

Read `vercel.json` to see existing routes.

- [ ] **Step 2: Add the content-ingest route**

The `"handle": "filesystem"` rule at the top means Vercel will check for a matching file in `api/` before falling through. Since we named the file `api/content-ingest.ts`, the filesystem handler should pick it up automatically — **no route change needed**.

Verify by checking: `api/content-ingest.ts` exists and the `handle: filesystem` rule is first.

If for some reason the Axum catch-all `{ "src": "/(.*)", "dest": "/api/axum" }` matches first, add an explicit route before it:

```json
{ "src": "/api/content-ingest", "dest": "/api/content-ingest" }
```

- [ ] **Step 3: Commit (only if vercel.json changed)**

```bash
git add vercel.json
git commit -m "chore: add content-ingest route to vercel.json"
```

---

### Task 8: E2e Tests for `list_doc_types`

**Files:**
- Create: `tests/e2e/tests/doc_type_test.rs`

- [ ] **Step 1: Read existing test patterns**

Read `tests/e2e/tests/ingest_test.rs` and `tests/e2e/tests/common/mod.rs` for the test setup pattern.

- [ ] **Step 2: Write the test**

Create `tests/e2e/tests/doc_type_test.rs`:

```rust
#![cfg(feature = "test-db")]

mod common;

/// list_doc_types returns the system document types.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_doc_types_returns_system_types(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    // Ensure profile exists.
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");

    // Call the doc_types endpoint via raw HTTP (no client method yet).
    let resp = app
        .reqwest_client
        .get(app.url("/api/doc-types"))
        .bearer_auth(&app.token)
        .send()
        .await
        .expect("request failed");

    assert_eq!(resp.status(), 200);

    let body: Vec<serde_json::Value> = resp.json().await.expect("parse failed");
    assert!(!body.is_empty(), "expected at least one doc type");

    // Check that well-known types exist.
    let names: Vec<&str> = body
        .iter()
        .filter_map(|v| v.get("name")?.as_str())
        .collect();
    assert!(names.contains(&"research"), "expected 'research' doc type");
    assert!(names.contains(&"session"), "expected 'session' doc type");
}
```

**Note:** This test calls the REST API, not MCP directly. The `list_doc_types` tool delegates to the same service function, so testing via REST is a valid proxy. If there's no REST endpoint for doc types, this test should instead call the service function directly:

```rust
use temper_api::services::doc_type_service;

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_doc_types_returns_system_types(pool: sqlx::PgPool) {
    let rows = doc_type_service::list_all(&pool).await.expect("list_all failed");
    assert!(!rows.is_empty());
    assert!(rows.iter().any(|r| r.name == "research"));
    assert!(rows.iter().any(|r| r.name == "session"));
}
```

Use whichever pattern fits — check if a REST endpoint exists, otherwise test the service directly.

- [ ] **Step 3: Run the test**

Run: `cargo nextest run -p temper-e2e --features test-db doc_type`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add tests/e2e/tests/doc_type_test.rs
git commit -m "test: add e2e test for list_doc_types"
```

---

### Task 9: E2e Tests for `ingest_content` and `update_resource_content`

**Files:**
- Create: `tests/e2e/tests/mcp_ingest_test.rs`

- [ ] **Step 1: Read the ingest test patterns**

Read `tests/e2e/tests/ingest_test.rs` for the setup pattern with contexts and payloads.

- [ ] **Step 2: Write the tests**

Create `tests/e2e/tests/mcp_ingest_test.rs`:

```rust
#![cfg(feature = "test-db")]

mod common;

use temper_api::services::{context_service, doc_type_service, ingest_service};
use temper_core::types::ids::ProfileId;

/// create_resource_with_manifest creates resource + manifest + event.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn create_resource_with_manifest_inserts_all_records(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;

    // Ensure profile exists.
    app.client.profile().get().await.expect("profile pre-flight failed");

    // Get the test user's profile_id.
    let profile_id = ProfileId::from(
        sqlx::query_scalar!(
            "SELECT id FROM kb_profiles WHERE id IN (SELECT profile_id FROM kb_profile_auth_links WHERE auth_provider_user_id = 'e2e-test-user') LIMIT 1"
        )
        .fetch_one(&pool)
        .await
        .expect("profile lookup"),
    );

    // Create context.
    let context = context_service::resolve_by_name(&pool, profile_id, "mcp-test").await.expect("context");
    let doc_type_id = ingest_service::resolve_doc_type(&pool, "research").await.expect("doc_type");

    let content = "# MCP Test\n\nThis is test content from MCP ingest.";
    let body_hash = format!("sha256:{}", sha2_hex(content));
    let empty = serde_json::json!({});

    let resource = ingest_service::create_resource_with_manifest(
        &pool,
        profile_id,
        "mcp-test",
        context.id,
        doc_type_id,
        "MCP Test Resource",
        Some("mcp-test-resource"),
        "mcp://test/create",
        &body_hash,
        &empty,
        &empty,
    )
    .await
    .expect("create_resource_with_manifest");

    assert_eq!(resource.title, "MCP Test Resource");
    assert!(resource.is_active);

    // Verify manifest exists with correct hash.
    let manifest_hash: String = sqlx::query_scalar!(
        "SELECT body_hash FROM kb_resource_manifests WHERE resource_id = $1",
        resource.id,
    )
    .fetch_one(&pool)
    .await
    .expect("manifest lookup");

    assert_eq!(manifest_hash, body_hash);

    // Verify event was created.
    let event_count: i64 = sqlx::query_scalar!(
        "SELECT count(*) FROM kb_events WHERE resource_id = $1 AND event_type = 'resource_created'",
        resource.id,
    )
    .fetch_one(&pool)
    .await
    .expect("event count")
    .unwrap_or(0);

    assert_eq!(event_count, 1);
}

/// Dedup: same body hash returns existing resource.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn find_by_body_hash_returns_existing(pool: sqlx::PgPool) {
    let app = common::setup(pool.clone()).await;
    app.client.profile().get().await.expect("profile pre-flight failed");

    let profile_id = ProfileId::from(
        sqlx::query_scalar!(
            "SELECT id FROM kb_profiles WHERE id IN (SELECT profile_id FROM kb_profile_auth_links WHERE auth_provider_user_id = 'e2e-test-user') LIMIT 1"
        )
        .fetch_one(&pool)
        .await
        .expect("profile lookup"),
    );

    let context = context_service::resolve_by_name(&pool, profile_id, "dedup-test").await.expect("context");
    let doc_type_id = ingest_service::resolve_doc_type(&pool, "research").await.expect("doc_type");

    let content = "# Dedup Test\n\nIdentical content for dedup testing.";
    let body_hash = format!("sha256:{}", sha2_hex(content));
    let empty = serde_json::json!({});

    // Create first resource.
    let first = ingest_service::create_resource_with_manifest(
        &pool, profile_id, "test", context.id, doc_type_id,
        "First", None, "mcp://test/dedup-1", &body_hash, &empty, &empty,
    )
    .await
    .expect("first create");

    // Dedup check should find it.
    let existing = ingest_service::find_by_body_hash(&pool, profile_id, &body_hash)
        .await
        .expect("dedup check")
        .expect("should find existing");

    assert_eq!(existing.id, first.id);
}

/// Unknown doc_type returns error.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resolve_unknown_doc_type_errors(pool: sqlx::PgPool) {
    let result = ingest_service::resolve_doc_type(&pool, "nonexistent-type").await;
    assert!(result.is_err());
}

/// list_all doc types returns known system types.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_all_doc_types(pool: sqlx::PgPool) {
    let rows = doc_type_service::list_all(&pool).await.expect("list_all");
    assert!(!rows.is_empty());
    assert!(rows.iter().any(|r| r.name == "research"));
    assert!(rows.iter().any(|r| r.name == "session"));
}

fn sha2_hex(content: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}
```

- [ ] **Step 3: Add sha2 and hex to e2e test dependencies**

Check `tests/e2e/Cargo.toml` — add `sha2` and `hex` if not already present:

```toml
[dev-dependencies]
sha2 = "0.10"
hex = "0.4"
```

- [ ] **Step 4: Run the tests**

Run: `cargo nextest run -p temper-e2e --features test-db mcp_ingest`
Expected: All tests PASS.

- [ ] **Step 5: Commit**

```bash
git add tests/e2e/tests/mcp_ingest_test.rs tests/e2e/Cargo.toml
git commit -m "test: add e2e tests for MCP content ingestion"
```

---

### Task 10: TypeScript Unit Tests

**Files:**
- Create: `packages/temper-cloud/tests/content-ingest.test.ts`

- [ ] **Step 1: Read existing TS test patterns**

Read `packages/temper-cloud/tests/hash.test.ts` for the test framework pattern (Vitest).

- [ ] **Step 2: Write the validation tests**

Create `packages/temper-cloud/tests/content-ingest.test.ts`:

```typescript
import { describe, expect, it } from "vitest";
import { validatePayload } from "../src/content-ingest.js";

describe("validatePayload", () => {
  it("accepts valid payload", () => {
    const result = validatePayload({
      resource_id: "019d6313-0e44-7842-9256-9ee385be3a51",
      content: "# Hello\n\nWorld",
      replace: false,
    });
    expect(result.ok).toBe(true);
    if (result.ok) {
      expect(result.payload.resource_id).toBe("019d6313-0e44-7842-9256-9ee385be3a51");
      expect(result.payload.replace).toBe(false);
    }
  });

  it("rejects missing resource_id", () => {
    const result = validatePayload({ content: "hello", replace: false });
    expect(result.ok).toBe(false);
  });

  it("rejects empty content", () => {
    const result = validatePayload({
      resource_id: "019d6313-0e44-7842-9256-9ee385be3a51",
      content: "",
      replace: false,
    });
    expect(result.ok).toBe(false);
  });

  it("rejects invalid UUID", () => {
    const result = validatePayload({
      resource_id: "not-a-uuid",
      content: "hello",
      replace: false,
    });
    expect(result.ok).toBe(false);
  });

  it("rejects missing replace flag", () => {
    const result = validatePayload({
      resource_id: "019d6313-0e44-7842-9256-9ee385be3a51",
      content: "hello",
    });
    expect(result.ok).toBe(false);
  });
});
```

- [ ] **Step 3: Run the tests**

Run: `cd packages/temper-cloud && bun run test`
Expected: All tests PASS.

- [ ] **Step 4: Commit**

```bash
git add packages/temper-cloud/tests/content-ingest.test.ts
git commit -m "test: add unit tests for content-ingest validation"
```

---

### Task 11: Agent Skills Documentation Updates

**Files:**
- Modify: `agent-skills/knowledge-base.md`
- Modify: `agent-skills/claude-desktop.md`
- Modify: `agent-skills/SKILL.md`

- [ ] **Step 1: Read the current agent-skills files**

Read all three files to understand what needs updating.

- [ ] **Step 2: Update knowledge-base.md**

Replace the "Writing Content" section (starting at line 67) with expanded content. The key changes:

1. Add `ingest_content` to the decision table:

```markdown
| Write content to knowledge base | Tool: `ingest_content` | Creates resource + async content processing |
| Discover valid document types | Tool: `list_doc_types` | Returns id, name, description for each type |
| Update existing content | Tool: `update_resource_content` | Re-processes content for existing resource |
```

2. Replace the "Writing Content" section with:

```markdown
## Writing Content

All mutations go through tools. There are no writable resources.

### Creating Content (Recommended)

Use `ingest_content` to write markdown content directly to the knowledge base:

```
Tool: ingest_content
Input: {
  "title": "Session: Authentication Research",
  "content": "# Authentication Research\n\nFindings from today's investigation...",
  "context_name": "myproject",
  "doc_type_name": "session",
  "slug": "2026-04-06-auth-research",       // optional
  "origin_uri": "mcp://agent/my-session"     // optional
}
```

**What happens:**
1. The resource is created immediately with metadata and manifest
2. Content is processed asynchronously (chunked, embedded, stored)
3. The resource becomes searchable shortly after creation
4. Returns `{ resource_id, title, context_name, status: "created" }`

**Deduplication:** If identical content already exists (same SHA256 hash), the
existing resource is returned with `status: "existing"` instead of creating a
duplicate.

**Context handling:** Before using a context_name that might not exist yet,
check with `list_contexts` first. If the context doesn't exist, **ask the user**
whether to create a new context or use an existing one. Do not silently auto-create
contexts without user confirmation.

### Discovering Document Types

Use `list_doc_types` to see what document types are available:

```
Tool: list_doc_types
Input: {}  (no parameters)
```

Common types for agent-created content:
- `session` — session notes, conversation summaries
- `research` — investigation findings, analysis
- `concept` — ideas, patterns, cross-cutting themes
- `task` — task definitions and tracking
- `goal` — goal definitions and progress

### Updating Content

Use `update_resource_content` to replace the content of an existing resource:

```
Tool: update_resource_content
Input: {
  "resource_id": "<UUID from previous creation>",
  "content": "# Updated Content\n\nRevised findings..."
}
```

The updated content replaces the previous version. Old chunks are version-bumped
and new chunks are processed asynchronously.

### Creating Metadata-Only Resources

Use `create_resource` when you need to create a resource shell without content
(e.g., as a placeholder or for file-based upload later):

```
Tool: create_resource
Input: {
  "kb_context_id": "<context UUID>",
  "kb_doc_type_id": "<doc type UUID>",
  "origin_uri": "the source or reference URL",
  "title": "Human-readable title"
}
```

Note: `create_resource` uses UUIDs for context and doc_type. Prefer `ingest_content`
which accepts human-readable names (context_name, doc_type_name) and resolves them
for you.
```

3. Update the search tip (around line 125) — remove the note about search requiring an embedding vector, since text-based search now works:

```markdown
- **Search supports text queries** — the `search` tool accepts a plain text
  `query` parameter for full-text search. No embedding vector needed.
```

- [ ] **Step 3: Update claude-desktop.md**

Add to the "Write operations" list (around line 57):

```markdown
- `ingest_content` — write markdown content to the knowledge base (recommended)
- `update_resource_content` — replace content of an existing resource
- `list_doc_types` — discover available document types
```

Add a new workflow section after "Save what we discussed" (around line 85):

```markdown
### "Write content to the knowledge base"

1. Use `list_doc_types` to see available document types
2. Use `list_contexts` to check existing contexts
3. Ask Claude to use `ingest_content` with your content, context, and doc type
4. The resource is created immediately and becomes searchable shortly after
5. Use the returned resource_id to reference the content later
```

- [ ] **Step 4: Update SKILL.md**

In the cloud access note (around line 27), update to mention content creation:

```markdown
> **Cloud access**: If a Temper MCP server is configured, you can also access the knowledge
> base through MCP resources and tools instead of (or alongside) local file access. The MCP
> server supports both reading and writing — use `ingest_content` to create resources with
> full content, and `search` for text-based discovery. See `knowledge-base.md` for MCP
> access patterns and `claude-desktop.md` for Claude Desktop setup.
```

- [ ] **Step 5: Commit**

```bash
git add agent-skills/knowledge-base.md agent-skills/claude-desktop.md agent-skills/SKILL.md
git commit -m "docs: update agent-skills with content creation workflow"
```

---

### Task 12: Regenerate sqlx Offline Cache

**Files:**
- Modify: `.sqlx/` (auto-generated)

- [ ] **Step 1: Regenerate the cache**

Any new `sqlx::query!()` or `sqlx::query_as!()` calls need to be cached for CI builds:

Run: `cargo sqlx prepare --workspace -- --all-features`
Expected: Regenerates `.sqlx/` query cache files.

- [ ] **Step 2: Run full verification**

Run: `cargo make check`
Expected: All checks pass.

Run: `cargo make test`
Expected: All unit tests pass.

Run: `cd packages/temper-cloud && bun run check && bun run test`
Expected: All TS checks and tests pass.

- [ ] **Step 3: Commit**

```bash
git add .sqlx/
git commit -m "chore: regenerate sqlx offline cache"
```

---

### Task 13: Integration Test (Docker Postgres Required)

**Files:** None (runs existing + new tests)

- [ ] **Step 1: Ensure Docker is running**

Run: `cargo make docker-up`
Expected: Postgres container starts on port 5437.

- [ ] **Step 2: Run all e2e tests**

Run: `cargo make test-db`
Expected: All tests pass, including the new `doc_type_test` and `mcp_ingest_test`.

- [ ] **Step 3: Run full test suite**

Run: `cargo make test-all`
Expected: All Rust + TypeScript tests pass.

- [ ] **Step 4: Fix any failures and re-run**

If tests fail, fix the issues and re-run until green.

---

## Follow-Up (Not In This Plan)

1. **Crate graph cleanup:** Extract shared service traits so `temper-mcp` doesn't depend on `temper-api` directly. Target: `temper-services` crate or trait-based abstraction in `temper-core`.
2. **MCP auth for Claude Code:** Investigate the localhost redirect issue that prevents the MCP connector from completing OAuth in Claude Code (works in Claude Desktop).
3. **Content status polling:** Consider a `get_ingest_status` tool that lets agents check whether content processing is complete.
