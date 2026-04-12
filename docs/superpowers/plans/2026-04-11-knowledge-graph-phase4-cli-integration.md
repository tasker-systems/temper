# Phase 4: CLI Integration for Knowledge Graph — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose graph search parameters in the CLI, add edge display to `resource show`, and fix the frontmatter roundtrip bug that drops arrays/objects.

**Architecture:** Four independent changes wired together: (1) CLI search flags → search actions → client, (2) API edges endpoint → client → CLI show, (3) frontmatter serialization fix, (4) E2E test proving the full flow.

**Tech Stack:** Rust (clap, axum, sqlx, serde), temper-core types, temper-client HTTP client, temper-e2e test harness.

**Subagent Guidance:** All SG principles apply (SG-1 through SG-12). Key ones:
- **SG-1**: Read the file you're modifying AND a sibling before writing.
- **SG-5**: Implement exactly what the task says, no extras.
- **SG-6**: Run verification commands, read the output, don't assume success.
- **SG-11**: The new edges endpoint goes under `/api/resources/{id}/edges` — this nests properly under the existing `{id}` path.

**Project Fundamentals:**
- Service layer owns SQL — all SQL in `temper-api/src/services/`
- Typed structs over inline JSON
- Profile scoping — all queries through `resources_visible_to` or equivalent
- Run `cargo make check` before claiming work is complete

---

### Task 1: Add Graph Search Flags to CLI

**Files:**
- Modify: `crates/temper-cli/src/cli.rs:147-166` (Search variant)
- Modify: `crates/temper-cli/src/main.rs:358-375` (Search dispatch)
- Modify: `crates/temper-cli/src/commands/search_cmd.rs` (entire file)
- Modify: `crates/temper-cli/src/actions/search.rs:46-73` (query_api, text_query_api)

This task adds `--seed`, `--edge-type`, `--depth`, and `--no-graph` flags to `temper search` and wires them through to the `search_with_params()` client method.

- [ ] **Step 1: Add clap fields to Search variant in cli.rs**

In `crates/temper-cli/src/cli.rs`, add four fields after `text_only: bool` inside the `Search` variant (line ~165):

```rust
    /// Search the knowledge base
    Search {
        /// Search query text
        query: String,
        /// Filter by context name
        #[arg(long)]
        context: Option<String>,
        /// Filter by document type
        #[arg(long)]
        doc_type: Option<String>,
        /// Maximum results (default 10)
        #[arg(long)]
        limit: Option<i64>,
        /// Output format (pretty, no-tty, json — auto-detected from TTY by default)
        #[arg(long)]
        format: Option<String>,
        /// Use text-only search (no local embedding needed)
        #[arg(long)]
        text_only: bool,
        /// Explicit seed resource IDs for graph expansion (repeatable)
        #[arg(long = "seed")]
        seed_ids: Vec<uuid::Uuid>,
        /// Edge type filter for graph expansion (repeatable)
        #[arg(long = "edge-type")]
        edge_types: Vec<String>,
        /// Max hops for graph traversal (default 2, max 10)
        #[arg(long)]
        depth: Option<i32>,
        /// Disable graph expansion (enabled by default)
        #[arg(long)]
        no_graph: bool,
    },
```

- [ ] **Step 2: Update Search dispatch in main.rs**

In `crates/temper-cli/src/main.rs`, update the `Commands::Search` match arm (~line 358) to destructure the new fields and pass them to `search_cmd::run`:

```rust
        Commands::Search {
            query,
            context,
            doc_type,
            limit,
            format,
            text_only,
            seed_ids,
            edge_types,
            depth,
            no_graph,
        } => {
            let format = temper_cli::format::resolve_format_str(format.as_deref());
            commands::search_cmd::run(
                &query,
                context.as_deref(),
                doc_type.as_deref(),
                limit,
                format,
                text_only,
                seed_ids,
                edge_types,
                depth,
                no_graph,
            )
        }
```

- [ ] **Step 3: Update search_cmd::run to accept and pass graph params**

Rewrite `crates/temper-cli/src/commands/search_cmd.rs` to build a `SearchParams` struct and use the new `search_actions::search_api()` function:

```rust
//! `temper search` — thin CLI wrapper over actions::search.

use crate::actions::{runtime, search as search_actions};
use crate::error::Result;
use crate::format::OutputFormat;
use uuid::Uuid;

pub fn run(
    query: &str,
    context: Option<&str>,
    doc_type: Option<&str>,
    limit: Option<i64>,
    format: &str,
    text_only: bool,
    seed_ids: Vec<Uuid>,
    edge_types: Vec<String>,
    depth: Option<i32>,
    no_graph: bool,
) -> Result<()> {
    let fmt = OutputFormat::parse(format);
    let vault_root = crate::config::resolve_vault(None)?;
    let temper_dir = vault_root.join(".temper");
    let device_id = runtime::require_device_id()?;
    let manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

    let embedding = if text_only {
        None
    } else {
        Some(search_actions::embed_query(query)?)
    };

    let results = runtime::with_client(|client| {
        let params = search_actions::build_search_params(
            query,
            embedding.clone(),
            context,
            doc_type,
            limit,
            seed_ids.clone(),
            edge_types.clone(),
            depth,
            no_graph,
        );
        Box::pin(async move { search_actions::search_api(client, params).await })
    })?;

    let enriched = search_actions::enrich_results(results, &manifest);

    if enriched.is_empty() {
        if fmt == OutputFormat::Json {
            crate::output::plain("[]");
        } else {
            crate::output::warning("No results found.");
        }
        return Ok(());
    }

    if fmt == OutputFormat::Json {
        crate::output::plain(serde_json::to_string_pretty(&enriched)?);
    } else {
        for line in search_actions::format_text(&enriched) {
            crate::output::plain(line);
        }
    }

    Ok(())
}
```

- [ ] **Step 4: Add build_search_params and search_api to search actions**

In `crates/temper-cli/src/actions/search.rs`, add two new functions after `text_query_api` (~line 73). Keep the old `query_api` and `text_query_api` functions for backward compatibility (MCP or other callers may use them):

```rust
/// Build a SearchParams from CLI arguments.
pub fn build_search_params(
    query: &str,
    embedding: Option<Vec<f32>>,
    context: Option<&str>,
    doc_type: Option<&str>,
    limit: Option<i64>,
    seed_ids: Vec<uuid::Uuid>,
    edge_types: Vec<String>,
    depth: Option<i32>,
    no_graph: bool,
) -> temper_core::types::api::SearchParams {
    temper_core::types::api::SearchParams {
        query: Some(query.to_string()),
        embedding,
        search_config: "english".into(),
        context_name: context.map(String::from),
        doc_type: doc_type.map(String::from),
        limit,
        offset: None,
        seed_ids: if seed_ids.is_empty() {
            None
        } else {
            Some(seed_ids)
        },
        edge_types: if edge_types.is_empty() {
            None
        } else {
            Some(edge_types)
        },
        graph_depth: depth,
        graph_expand: !no_graph,
    }
}

/// Call the search API with full SearchParams.
pub async fn search_api(
    client: &temper_client::TemperClient,
    params: temper_core::types::api::SearchParams,
) -> Result<Vec<UnifiedSearchResultRow>> {
    client
        .search()
        .search_with_params(&params)
        .await
        .map_err(crate::commands::client_err)
}
```

- [ ] **Step 5: Verify build passes**

Run: `cargo build -p temper-cli`
Expected: Compiles with no errors.

- [ ] **Step 6: Run existing tests**

Run: `cargo nextest run -p temper-cli`
Expected: All 330 tests pass. No regressions.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs crates/temper-cli/src/commands/search_cmd.rs crates/temper-cli/src/actions/search.rs
git commit -m "feat(cli): add graph search flags --seed, --edge-type, --depth, --no-graph"
```

---

### Task 2: Add Edges API Endpoint and Service Function

**Files:**
- Create: `crates/temper-api/src/handlers/edges.rs`
- Modify: `crates/temper-api/src/handlers/mod.rs`
- Modify: `crates/temper-api/src/routes.rs:56-58`
- Modify: `crates/temper-api/src/services/edge_service.rs` (add `list_resource_edges`)

This task adds `GET /api/resources/{id}/edges` which calls `graph_resource_edges()`.

- [ ] **Step 1: Add service function to edge_service.rs**

In `crates/temper-api/src/services/edge_service.rs`, add a new public function at the end of the file. Read the existing file first to match the style (imports, error handling).

```rust
/// List all edges connected to a resource, checking visibility.
pub async fn list_resource_edges(
    pool: &PgPool,
    profile_id: Uuid,
    resource_id: Uuid,
) -> ApiResult<Vec<temper_core::types::graph::GraphEdgeRow>> {
    // Verify the resource is visible to this profile
    let _resource = crate::services::resource_service::get_visible(pool, profile_id, resource_id).await?;

    let rows = sqlx::query_as::<_, temper_core::types::graph::GraphEdgeRow>(
        "SELECT * FROM graph_resource_edges($1, $2)"
    )
    .bind(profile_id)
    .bind(resource_id)
    .fetch_all(pool)
    .await
    .map_err(crate::error::ApiError::Database)?;

    Ok(rows)
}
```

Note: This uses runtime `query_as` (not the `query_as!` macro) because `graph_resource_edges()` is a SQL function that returns a TABLE — the macro has trouble with these. This follows the same pattern as `search_service.rs` which uses runtime `query_as` for `unified_search()`.

- [ ] **Step 2: Create the edges handler**

Create `crates/temper-api/src/handlers/edges.rs`. Follow the pattern from `handlers/resources.rs` — read that file first for style:

```rust
use axum::extract::{Path, State};
use axum::Json;
use uuid::Uuid;

use crate::error::{ApiResult, ErrorBody};
use crate::middleware::auth::AuthUser;
use crate::services::edge_service;
use crate::state::AppState;
use temper_core::types::graph::GraphEdgeRow;

#[utoipa::path(
    get,
    path = "/api/resources/{id}/edges",
    tag = "Resources",
    params(("id" = Uuid, Path, description = "Resource ID")),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Resource edges", body = Vec<GraphEdgeRow>),
        (status = 401, description = "Unauthorized", body = ErrorBody),
        (status = 404, description = "Not found", body = ErrorBody),
    )
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
) -> ApiResult<Json<Vec<GraphEdgeRow>>> {
    edge_service::list_resource_edges(&state.pool, auth.0.profile.id, resource_id)
        .await
        .map(Json)
}
```

- [ ] **Step 3: Register the handler module and route**

In `crates/temper-api/src/handlers/mod.rs`, add:
```rust
pub mod edges;
```

In `crates/temper-api/src/routes.rs`, add the route inside the `gated` router, after the `/api/resources/{id}/content` route (~line 60):

```rust
        .route(
            "/api/resources/{id}/edges",
            get(handlers::edges::list),
        )
```

Make sure `get` is imported (it already is — check the `use` at the top of the file).

- [ ] **Step 4: Verify build passes**

Run: `cargo build -p temper-api`
Expected: Compiles with no errors.

- [ ] **Step 5: Run existing integration tests**

Run: `cargo nextest run -p temper-api --features test-db`
Expected: All 88 tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/handlers/edges.rs crates/temper-api/src/handlers/mod.rs crates/temper-api/src/routes.rs crates/temper-api/src/services/edge_service.rs
git commit -m "feat(api): add GET /api/resources/{id}/edges endpoint"
```

---

### Task 3: Add Edges Client Method

**Files:**
- Modify: `crates/temper-client/src/resources.rs` (add `edges` method)

This adds a method to the existing `ResourceClient` to fetch edges for a resource. No new sub-client needed — edges are a sub-resource of resources.

- [ ] **Step 1: Read the existing ResourceClient file**

Read `crates/temper-client/src/resources.rs` to understand the pattern (method signatures, error handling, HTTP method usage).

- [ ] **Step 2: Add the edges method**

Add to `ResourceClient` in `crates/temper-client/src/resources.rs`:

```rust
    /// List edges connected to a resource.
    pub async fn edges(
        &self,
        resource_id: uuid::Uuid,
    ) -> Result<Vec<temper_core::types::graph::GraphEdgeRow>> {
        let token = self.http.resolve_token()?;
        let path = format!("/api/resources/{resource_id}/edges");
        let req = self.http.get(&path);
        self.http
            .send_json(&reqwest::Method::GET, &path, req, Some(&token))
            .await
    }
```

Match the exact import style already in the file — check if `reqwest::Method` is imported at the top or used inline. Follow what you see.

- [ ] **Step 3: Verify build passes**

Run: `cargo build -p temper-client`
Expected: Compiles with no errors.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-client/src/resources.rs
git commit -m "feat(client): add edges() method to ResourceClient"
```

---

### Task 4: Add `--edges` Flag to `resource show`

**Files:**
- Modify: `crates/temper-cli/src/cli.rs:227-240` (Show variant)
- Modify: `crates/temper-cli/src/main.rs:169-183` (Show dispatch)
- Modify: `crates/temper-cli/src/commands/resource.rs:511-574` (show, show_generic)

This adds `--edges` to `temper resource show` that appends edge information after the resource content.

- [ ] **Step 1: Add `edges` flag to Show variant in cli.rs**

In `crates/temper-cli/src/cli.rs`, add to the `Show` variant inside `ResourceAction` (~line 239, before the closing brace):

```rust
    Show {
        /// Resource slug
        slug: String,
        /// Resource type (task, goal, session, research, concept, decision)
        #[arg(long)]
        r#type: String,
        /// Filter by context
        #[arg(long)]
        context: Option<String>,
        /// Output format (pretty, no-tty, json — auto-detected from TTY by default)
        #[arg(long)]
        format: Option<String>,
        /// Show graph edges connected to this resource
        #[arg(long)]
        edges: bool,
    },
```

- [ ] **Step 2: Update Show dispatch in main.rs**

In `crates/temper-cli/src/main.rs`, update the `ResourceAction::Show` match arm (~line 169) to destructure `edges` and pass it:

```rust
                ResourceAction::Show {
                    slug,
                    r#type,
                    context,
                    format,
                    edges,
                } => {
                    let format = temper_cli::format::resolve_format_str(format.as_deref());
                    temper_cli::commands::resource::show(
                        &config,
                        &r#type,
                        &slug,
                        context.as_deref(),
                        format,
                        edges,
                    )
                }
```

- [ ] **Step 3: Update show() and show_generic() in resource.rs**

In `crates/temper-cli/src/commands/resource.rs`, update the `show` function signature (~line 512) to accept `edges: bool` and pass it through:

```rust
pub fn show(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    format: &str,
    edges: bool,
) -> Result<()> {
    validate_doc_type(doc_type)?;

    match doc_type {
        "task" => crate::commands::task::show(config, slug, context, format),
        "session" => crate::commands::session::show(config, slug, context, format),
        _ => show_generic(config, doc_type, slug, context, format),
    }?;

    if edges {
        show_edges(config, doc_type, slug, context, format)?;
    }

    Ok(())
}
```

Note: The `task::show` and `session::show` delegates don't need `edges` — edge display is appended separately.

- [ ] **Step 4: Add the show_edges function**

Add a new function in `crates/temper-cli/src/commands/resource.rs` after `show_generic`:

```rust
/// Fetch and display edges for a resource via the API.
fn show_edges(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    use crate::actions::runtime;
    use temper_core::types::graph::GraphEdgeRow;

    // Resolve resource ID — we need the UUID to call the edges API.
    // Look up from the local manifest by slug.
    let vault_root = crate::config::resolve_vault(None)?;
    let temper_dir = vault_root.join(".temper");
    let device_id = runtime::require_device_id()?;
    let manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;

    let resource_id = manifest
        .entries
        .iter()
        .find(|(_, entry)| {
            // Match by slug in path: the path ends with `/{slug}.md`
            entry
                .path
                .strip_suffix(".md")
                .and_then(|p| p.rsplit('/').next())
                == Some(slug)
        })
        .map(|(id, _)| uuid::Uuid::from(*id))
        .ok_or_else(|| {
            crate::error::TemperError::Vault(format!(
                "resource '{slug}' not found in manifest — sync first to use --edges"
            ))
        })?;

    let edges: Vec<GraphEdgeRow> = runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .edges(resource_id)
                .await
                .map_err(crate::commands::client_err)
        })
    })?;

    if edges.is_empty() {
        if format != "json" {
            println!("\nEdges: (none)");
        }
        return Ok(());
    }

    if format == "json" {
        // JSON mode — print edges array
        let json = serde_json::to_string_pretty(&edges).unwrap_or_default();
        println!("{json}");
    } else {
        // Text mode — group by direction
        println!("\nEdges:");
        let outgoing: Vec<_> = edges.iter().filter(|e| e.direction == "outgoing").collect();
        let incoming: Vec<_> = edges.iter().filter(|e| e.direction == "incoming").collect();

        if !outgoing.is_empty() {
            println!("  outgoing:");
            for e in &outgoing {
                println!("    {} \u{2192} {} ({})", e.edge_type, e.peer_slug, e.peer_title);
            }
        }
        if !incoming.is_empty() {
            println!("  incoming:");
            for e in &incoming {
                println!("    {} \u{2190} {} ({})", e.edge_type, e.peer_slug, e.peer_title);
            }
        }
    }

    Ok(())
}
```

- [ ] **Step 5: Verify build passes**

Run: `cargo build -p temper-cli`
Expected: Compiles with no errors.

- [ ] **Step 6: Run existing tests**

Run: `cargo nextest run -p temper-cli`
Expected: All 330 tests pass.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs crates/temper-cli/src/commands/resource.rs
git commit -m "feat(cli): add --edges flag to resource show for graph edge display"
```

---

### Task 5: Fix build_frontmatter_from_resource Array/Object Serialization

**Files:**
- Modify: `crates/temper-cli/src/actions/ingest.rs:482-489`
- Test: `crates/temper-cli/src/actions/ingest.rs` (existing test module)

This fixes the bug where JSON arrays and objects in `managed_meta` are silently dropped, breaking the sync roundtrip for relationship fields and any other structured frontmatter.

- [ ] **Step 1: Write a failing test**

In `crates/temper-cli/src/actions/ingest.rs`, find the test module (search for `#[cfg(test)]`). Add a new test:

```rust
    #[test]
    fn test_build_frontmatter_from_resource_preserves_arrays_and_objects() {
        let resource = temper_core::types::ResourceRow {
            id: uuid::Uuid::nil(),
            profile_id: uuid::Uuid::nil(),
            context_id: uuid::Uuid::nil(),
            doc_type_id: uuid::Uuid::nil(),
            title: "Test".to_string(),
            slug: Some("test-slug".to_string()),
            origin_uri: "test://origin".to_string(),
            kb_uri: "kb://test".to_string(),
            content_hash: None,
            owner_handle: "@me".to_string(),
            created: chrono::Utc::now(),
            updated: chrono::Utc::now(),
        };

        let meta = serde_json::json!({
            "depends_on": ["slug-a", "slug-b"],
            "extends": ["parent-doc"],
            "tags": ["rust", "graph"],
            "config": {"key": "value", "nested": true}
        });

        let fm = build_frontmatter_from_resource(&resource, "temper", "research", Some(&meta));

        assert!(
            fm.contains("depends_on:"),
            "depends_on array should be present. Got:\n{fm}"
        );
        assert!(
            fm.contains("slug-a"),
            "depends_on should contain slug-a. Got:\n{fm}"
        );
        assert!(
            fm.contains("slug-b"),
            "depends_on should contain slug-b. Got:\n{fm}"
        );
        assert!(
            fm.contains("extends:"),
            "extends array should be present. Got:\n{fm}"
        );
        assert!(
            fm.contains("config:"),
            "config object should be present. Got:\n{fm}"
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run -p temper-cli test_build_frontmatter_from_resource_preserves_arrays_and_objects`
Expected: FAIL — arrays and objects are currently skipped.

- [ ] **Step 3: Add a json_value_to_yaml helper function**

In `crates/temper-cli/src/actions/ingest.rs`, add a helper near `yaml_escape_string` (~line 498):

```rust
/// Serialize a JSON value to a YAML string fragment (no trailing newline).
/// Arrays use flow style: `["a", "b"]`
/// Objects use block style with 2-space indent.
fn json_value_to_yaml(value: &serde_json::Value, indent: usize) -> String {
    match value {
        serde_json::Value::String(s) => format!("\"{}\"", yaml_escape_string(s)),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(arr) => {
            // Flow style for arrays: ["a", "b", "c"]
            let items: Vec<String> = arr.iter().map(|v| json_value_to_yaml(v, indent)).collect();
            format!("[{}]", items.join(", "))
        }
        serde_json::Value::Object(obj) => {
            // Block style for objects
            let prefix = " ".repeat(indent + 2);
            let mut lines = Vec::new();
            for (k, v) in obj {
                match v {
                    serde_json::Value::Object(_) => {
                        lines.push(format!("{prefix}{k}:"));
                        // Recursively render nested object keys
                        if let serde_json::Value::Object(inner) = v {
                            for (ik, iv) in inner {
                                let nested_prefix = " ".repeat(indent + 4);
                                lines.push(format!(
                                    "{nested_prefix}{ik}: {}",
                                    json_value_to_yaml(iv, indent + 4)
                                ));
                            }
                        }
                    }
                    _ => {
                        lines.push(format!("{prefix}{k}: {}", json_value_to_yaml(v, indent + 2)));
                    }
                }
            }
            format!("\n{}", lines.join("\n"))
        }
    }
}
```

- [ ] **Step 4: Replace the skip branch in build_frontmatter_from_resource**

In `crates/temper-cli/src/actions/ingest.rs`, change the match block inside `build_frontmatter_from_resource` (~line 482-489):

Replace:
```rust
                serde_json::Value::Null => fm.push_str(&format!("{key}: null\n")),
                _ => {} // Skip arrays/objects — not representable as scalar YAML fields
```

With:
```rust
                serde_json::Value::Null => fm.push_str(&format!("{key}: null\n")),
                serde_json::Value::Array(_) => {
                    fm.push_str(&format!("{key}: {}\n", json_value_to_yaml(value, 0)));
                }
                serde_json::Value::Object(_) => {
                    fm.push_str(&format!("{key}:{}\n", json_value_to_yaml(value, 0)));
                }
```

- [ ] **Step 5: Run the test to verify it passes**

Run: `cargo nextest run -p temper-cli test_build_frontmatter_from_resource_preserves_arrays_and_objects`
Expected: PASS

- [ ] **Step 6: Run all temper-cli tests**

Run: `cargo nextest run -p temper-cli`
Expected: All tests pass including the existing `build_frontmatter_from_resource` tests.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/actions/ingest.rs
git commit -m "fix(cli): serialize arrays and objects in build_frontmatter_from_resource

Previously JSON arrays and objects in managed_meta were silently dropped,
causing relationship fields and other structured frontmatter to be lost
on sync pull."
```

---

### Task 6: Add Edges Integration Test

**Files:**
- Create: `crates/temper-api/tests/edges_test.rs` (or add to existing edge test file)

This adds an integration test that verifies the new `GET /api/resources/{id}/edges` endpoint returns correct results.

- [ ] **Step 1: Check for existing edge tests**

Read `crates/temper-api/tests/` to find where edge-related tests live. The edge_service already has tests — check if there's a dedicated test file for the edges endpoint or if it should be added to an existing graph test file.

- [ ] **Step 2: Write the integration test**

Follow the existing test pattern in `crates/temper-api/tests/`. The test should:

1. Create a profile and context
2. Ingest two resources (A depends_on B) using the ingest service
3. Call `edge_service::list_resource_edges()` for resource A
4. Assert the response contains one edge: `depends_on → B`
5. Assert the direction is `outgoing`, edge_type is correct, peer slug matches

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_resource_edges_returns_connected_edges(pool: sqlx::PgPool) {
    // Setup profile + context (follow existing test pattern in this file)
    // ...

    // Ingest B first (no edges), then A (depends_on B)
    // ...

    // Call list_resource_edges for A
    let edges = temper_api::services::edge_service::list_resource_edges(
        &pool,
        profile_id,
        resource_a_id,
    )
    .await
    .expect("list edges");

    assert_eq!(edges.len(), 1, "A should have 1 edge");
    assert_eq!(edges[0].edge_type.to_string(), "depends_on");
    assert_eq!(edges[0].direction, "outgoing");
    assert_eq!(edges[0].peer_slug, "resource-b-slug");
}
```

Adapt the exact setup code from the existing edge/graph tests in the same test directory.

- [ ] **Step 3: Run the test**

Run: `cargo nextest run -p temper-api --features test-db list_resource_edges`
Expected: PASS

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/tests/
git commit -m "test(api): add integration test for GET /api/resources/{id}/edges"
```

---

### Task 7: Add E2E CLI Graph Test

**Files:**
- Modify: `tests/e2e/tests/graph_search_test.rs` (add new test functions)

This adds end-to-end tests exercising the full CLI graph flow through the API.

- [ ] **Step 1: Read the existing graph_search_test.rs**

Read `tests/e2e/tests/graph_search_test.rs` thoroughly — understand the `test_payload` helper, the `common::setup()` pattern, and how the existing graph search E2E test works.

- [ ] **Step 2: Add E2E test for edges endpoint**

Add to `tests/e2e/tests/graph_search_test.rs`:

```rust
/// Verify the edges endpoint returns correct edges after ingest.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn edges_endpoint_returns_resource_edges(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client.profile().get().await.expect("profile pre-flight");
    app.client
        .contexts()
        .create("edges-e2e")
        .await
        .expect("create context");

    // Ingest B (leaf), then A (depends_on B)
    let payload_b = test_payload("Base Doc", "base-doc", "edges-e2e", None);
    let resource_b = app
        .client
        .ingest()
        .create(&payload_b)
        .await
        .expect("ingest B");

    let payload_a = test_payload(
        "Dependent Doc",
        "dependent-doc",
        "edges-e2e",
        Some(json!({"depends_on": ["base-doc"]})),
    );
    let resource_a = app
        .client
        .ingest()
        .create(&payload_a)
        .await
        .expect("ingest A");

    // Fetch edges for A
    let edges = app
        .client
        .resources()
        .edges(resource_a.id.into())
        .await
        .expect("fetch edges");

    assert_eq!(edges.len(), 1, "A should have 1 edge");
    assert_eq!(edges[0].edge_type.to_string(), "depends_on");
    assert_eq!(edges[0].direction, "outgoing");
    assert_eq!(edges[0].peer_slug, "base-doc");
    assert_eq!(edges[0].peer_resource_id, resource_b.id.into());

    // Fetch edges for B (should have incoming)
    let edges_b = app
        .client
        .resources()
        .edges(resource_b.id.into())
        .await
        .expect("fetch edges for B");

    assert_eq!(edges_b.len(), 1, "B should have 1 incoming edge");
    assert_eq!(edges_b[0].direction, "incoming");
    assert_eq!(edges_b[0].peer_slug, "dependent-doc");
}
```

- [ ] **Step 3: Add E2E test for search with graph flags via client**

Add to the same file:

```rust
/// Verify search_with_params respects graph flags end-to-end.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn search_no_graph_flag_disables_expansion(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client.profile().get().await.expect("profile pre-flight");
    app.client
        .contexts()
        .create("nograph-e2e")
        .await
        .expect("create context");

    // Ingest B, then A (depends_on B)
    let payload_b = test_payload("Leaf Node", "leaf-node", "nograph-e2e", None);
    let resource_b = app
        .client
        .ingest()
        .create(&payload_b)
        .await
        .expect("ingest B");

    let payload_a = test_payload(
        "Root Node",
        "root-node",
        "nograph-e2e",
        Some(json!({"depends_on": ["leaf-node"]})),
    );
    let resource_a = app
        .client
        .ingest()
        .create(&payload_a)
        .await
        .expect("ingest A");

    // Search with explicit seed and graph enabled
    let params_graph = SearchParams {
        query: None,
        embedding: None,
        search_config: "english".into(),
        context_name: Some("nograph-e2e".into()),
        doc_type: None,
        limit: Some(10),
        offset: None,
        seed_ids: Some(vec![resource_a.id.into()]),
        edge_types: None,
        graph_depth: Some(2),
        graph_expand: true,
    };

    let results_graph = app
        .client
        .search()
        .search_with_params(&params_graph)
        .await
        .expect("graph search");

    let graph_ids: Vec<uuid::Uuid> = results_graph.iter().map(|r| r.resource_id).collect();
    assert!(
        graph_ids.contains(&resource_b.id.into()),
        "Leaf should appear via graph expansion"
    );

    // Same search but graph_expand: false
    let params_no_graph = SearchParams {
        graph_expand: false,
        ..params_graph.clone()
    };

    let results_no_graph = app
        .client
        .search()
        .search_with_params(&params_no_graph)
        .await
        .expect("no-graph search");

    let no_graph_ids: Vec<uuid::Uuid> = results_no_graph.iter().map(|r| r.resource_id).collect();
    assert!(
        !no_graph_ids.contains(&resource_b.id.into()),
        "Leaf should NOT appear without graph expansion"
    );
}
```

- [ ] **Step 4: Run E2E tests**

Run: `cargo nextest run -p temper-e2e --features test-db`
Expected: All E2E tests pass including the new ones.

- [ ] **Step 5: Commit**

```bash
git add tests/e2e/tests/graph_search_test.rs
git commit -m "test(e2e): add edge endpoint and graph flag E2E tests"
```

---

### Task 8: Regenerate sqlx Cache and Final Verification

**Files:**
- Modify: `.sqlx/` (regenerated cache files)

- [ ] **Step 1: Regenerate sqlx cache**

Run: `cargo sqlx prepare --workspace -- --all-features`
Expected: Cache regenerated with no errors. New queries from edge_service show up.

- [ ] **Step 2: Run cargo make check**

Run: `cargo make check`
Expected: All checks pass (fmt, clippy, docs, TypeScript, Biome).

- [ ] **Step 3: Run full test suite**

Run: `cargo make test-all`
Expected: All Rust tests (unit + integration + E2E) and TypeScript tests pass.

- [ ] **Step 4: Commit cache if changed**

```bash
git add .sqlx/
git commit -m "chore: regenerate sqlx cache for edges endpoint"
```
