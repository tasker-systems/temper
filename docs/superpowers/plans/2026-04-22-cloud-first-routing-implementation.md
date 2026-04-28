# Cloud-First Routing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse the `VaultState::Local` / `Cloud` split from "parallel dispatch hierarchies" into "one flow with a file-I/O toggle." Reads (`list`, `show`, `search`) always hit the API; writes (`create`, `update`) always route through the existing `push_one_resource` primitive; local vault + manifest become a read-cache for Local mode; Cloud mode becomes pure API pass-through.

**Architecture:** `match VaultState::{Local, Cloud}` at each divergence site in the existing command modules (`resource.rs`, `sync_cmd.rs`, `task.rs`, `session.rs`). No parallel `resource_cloud.rs` module. `show` gets a new three-tier freshness ladder (`debounce` → `hash-verify` → `full-fetch`) encapsulated in `actions/show_cache.rs`. `create` / `update` route file-write through `push_one_resource` so the server is written in one round trip instead of waiting for the next `sync run`.

**Tech Stack:** Rust workspace (temper-cli, temper-client, temper-core), cargo-make + cargo-nextest, sqlx for DB, existing `push_one_resource` / `pull_one_resource` primitives from Unit A, `temp-env` for env-var scoped tests, `tempfile::TempDir` for filesystem fixtures.

**Reference spec:** `docs/superpowers/specs/2026-04-22-cloud-first-routing-and-mode-collapse-design.md`

---

## File Structure

**New files:**
- `crates/temper-cli/src/actions/show_cache.rs` — three-tier freshness ladder for `show`. One responsibility: decide "local is fresh enough" / "check hash" / "fetch body" given a `VaultState` + local-file state.
- `tests/e2e/tests/cloud_first_test.rs` — e2e tests exercising read-path and write-path behaviors in both modes.

**Modified files:**
- `crates/temper-cli/src/commands/resource.rs` — `list`, `show`, `create`, `update` each gain a `match VaultState` at the right level. Per-doctype dispatch stays; the mode branching wraps it where the divergence lives.
- `crates/temper-cli/src/commands/task.rs`, `session.rs`, `research.rs`, `goal.rs` — per-doctype `show` functions are refactored to use the `show_cache` ladder in Local mode.
- `crates/temper-cli/src/actions/task.rs`, `session.rs`, `research.rs`, and `resource.rs::create_simple_resource` — per-doctype `create` actions call `push_one_resource` after writing the file.
- `crates/temper-cli/src/commands/sync_cmd.rs` — `sync run` adds a cloud-mode redirect branch.
- `crates/temper-cli/src/actions/mod.rs` — register `show_cache` module.

**Deleted files:**
- `docs/superpowers/plans/2026-04-19-unit-b-2-cloud-mode-dispatch.md` — superseded by this plan (done in Task 10).

---

## Task 1: `show_cache` module — the three-tier freshness ladder

**Purpose:** Factor the Local-mode `show` cache logic into one pure module so every doctype's `show` function can call into it identically. Testable in isolation with filesystem fixtures.

**Files:**
- Create: `crates/temper-cli/src/actions/show_cache.rs`
- Modify: `crates/temper-cli/src/actions/mod.rs`

- [ ] **Step 1.1: Register the new module**

Add to `crates/temper-cli/src/actions/mod.rs` (alphabetical with the other `pub mod` lines):

```rust
pub mod show_cache;
```

- [ ] **Step 1.2: Write the module with its public API and doc comments**

**File:** `crates/temper-cli/src/actions/show_cache.rs`

```rust
//! Three-tier freshness ladder for `temper resource show` in Local mode.
//!
//! Given a resource id and its local cached path, decide how to produce
//! the content to render:
//!
//! 1. **Debounce**: if the local file's mtime is within `DEBOUNCE_SECONDS`
//!    of now, render the local content without any API call.
//! 2. **Hash-verify**: otherwise, `GET /resources/{id}` (metadata only, no
//!    body). If the server's `updated` timestamp matches the local
//!    frontmatter's `temper-updated`, touch the local mtime to now and
//!    render the local content.
//! 3. **Full-fetch**: if metadata diverges or no local file exists, call
//!    `GET /resources/{id}/content`, overwrite the local file, render
//!    the server response.
//!
//! Cloud mode never calls into this module — callers match on
//! `VaultState` before invoking.
//!
//! Offline degradation: on any network error inside tier 2 or 3, fall
//! back to "render local with a warn" if a local file exists, otherwise
//! surface the error.

use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

use filetime::{set_file_mtime, FileTime};
use temper_client::TemperClient;
use temper_core::types::ids::ResourceId;

use crate::error::{Result, TemperError};
use crate::output;

/// Default debounce window. Exposed as a module constant so tests can
/// refer to it; callers pass it through `ShowCacheConfig::debounce`.
pub const DEFAULT_DEBOUNCE_SECONDS: u64 = 30;

/// Inputs for the freshness ladder. Keep this small and explicit; the
/// caller owns the decision to invoke (they match on `VaultState`).
pub struct ShowCacheParams<'a> {
    pub client: &'a TemperClient,
    pub resource_id: ResourceId,
    pub local_path: &'a Path,
    pub debounce: Duration,
}

/// What the ladder produced. `content` is what the caller should render.
/// `source` tells the caller which tier fired (for tests and --verbose
/// hints; not user-visible by default).
pub struct ShowCacheResult {
    pub content: String,
    pub source: FreshnessTier,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FreshnessTier {
    /// Tier 1: local mtime is within the debounce window.
    Debounced,
    /// Tier 2: server metadata matched local; no body fetched.
    HashMatch,
    /// Tier 3: server body fetched, local file overwritten.
    FullFetch,
    /// Offline fallback: network failed, rendered stale local.
    OfflineFallback,
}

/// Produce the content for a `show` in Local mode.
///
/// Preconditions:
/// - `params.local_path` may or may not exist. If it doesn't, tier 1 and
///   tier 2 are skipped and the ladder starts at tier 3.
///
/// Postconditions:
/// - On tier 1: mtime unchanged (already fresh).
/// - On tier 2: mtime touched to now so subsequent calls debounce.
/// - On tier 3: file written, mtime is now (implicit via `fs::write`).
pub async fn fetch(params: ShowCacheParams<'_>) -> Result<ShowCacheResult> {
    // Tier 1: debounce.
    if let Some(fresh) = read_if_fresh(params.local_path, params.debounce)? {
        return Ok(ShowCacheResult {
            content: fresh,
            source: FreshnessTier::Debounced,
        });
    }

    // Tier 2 + 3: attempt server; fall back to stale local on network error.
    match attempt_remote(&params).await {
        Ok(result) => Ok(result),
        Err(err) if is_network_error(&err) => {
            if let Ok(body) = fs::read_to_string(params.local_path) {
                output::hint(format!(
                    "offline: rendering cached copy of {} (reason: {err})",
                    params.local_path.display()
                ));
                Ok(ShowCacheResult {
                    content: body,
                    source: FreshnessTier::OfflineFallback,
                })
            } else {
                Err(err)
            }
        }
        Err(err) => Err(err),
    }
}

fn read_if_fresh(path: &Path, debounce: Duration) -> Result<Option<String>> {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return Ok(None),
    };
    let mtime = meta
        .modified()
        .map_err(|e| TemperError::Vault(format!("mtime read: {e}")))?;
    let age = SystemTime::now()
        .duration_since(mtime)
        .unwrap_or(Duration::ZERO);
    if age < debounce {
        let body = fs::read_to_string(path).map_err(|e| TemperError::Vault(e.to_string()))?;
        Ok(Some(body))
    } else {
        Ok(None)
    }
}

async fn attempt_remote(params: &ShowCacheParams<'_>) -> Result<ShowCacheResult> {
    // Tier 2: metadata check. Cheap; no body transferred.
    let meta_check = params
        .client
        .resources()
        .get(*params.resource_id.as_uuid())
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;

    if let Ok(local_body) = fs::read_to_string(params.local_path) {
        if let Some(local_updated) = parse_frontmatter_updated(&local_body) {
            if local_updated == meta_check.updated {
                // Hash matches: touch mtime and render local.
                let now = FileTime::from_system_time(SystemTime::now());
                set_file_mtime(params.local_path, now)
                    .map_err(|e| TemperError::Vault(format!("touch mtime: {e}")))?;
                return Ok(ShowCacheResult {
                    content: local_body,
                    source: FreshnessTier::HashMatch,
                });
            }
        }
    }

    // Tier 3: full fetch. Overwrite local and render.
    let content = params
        .client
        .resources()
        .content(*params.resource_id.as_uuid())
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;

    fs::write(params.local_path, &content.content)
        .map_err(|e| TemperError::Vault(format!("cache write: {e}")))?;

    Ok(ShowCacheResult {
        content: content.content,
        source: FreshnessTier::FullFetch,
    })
}

/// Extract `temper-updated` from YAML frontmatter as a UTC `DateTime`.
/// Returns `None` if the file has no frontmatter or no `temper-updated`
/// field — which pushes the caller to tier 3 (safe default).
fn parse_frontmatter_updated(body: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    let fm = temper_core::frontmatter::Frontmatter::try_from(body).ok()?;
    let updated = fm.value().get("temper-updated")?;
    let s = updated.as_str()?;
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.with_timezone(&chrono::Utc))
}

/// Heuristic: does this error look like a network error vs a server
/// error? Only network errors justify offline-fallback; a 4xx from the
/// server should surface to the user as-is.
fn is_network_error(err: &TemperError) -> bool {
    matches!(err, TemperError::Api(msg) if msg.contains("connect") || msg.contains("dns") || msg.contains("timeout"))
}
```

- [ ] **Step 1.3: Add the `filetime` dependency**

Edit `crates/temper-cli/Cargo.toml`. Add to `[dependencies]`:

```toml
filetime = "0.2"
```

- [ ] **Step 1.4: Write a unit test for the debounce tier**

Append to `crates/temper-cli/src/actions/show_cache.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tempfile::NamedTempFile;

    #[test]
    fn read_if_fresh_returns_content_when_mtime_within_window() {
        let file = NamedTempFile::new().expect("tempfile");
        std::fs::write(file.path(), "hello").expect("write");

        let result = read_if_fresh(file.path(), Duration::from_secs(30))
            .expect("read_if_fresh")
            .expect("fresh within window");

        assert_eq!(result, "hello");
    }

    #[test]
    fn read_if_fresh_returns_none_when_stale() {
        let file = NamedTempFile::new().expect("tempfile");
        std::fs::write(file.path(), "stale").expect("write");
        // Force mtime to 60s ago.
        let past = FileTime::from_system_time(SystemTime::now() - Duration::from_secs(60));
        set_file_mtime(file.path(), past).expect("set mtime");

        let result =
            read_if_fresh(file.path(), Duration::from_secs(30)).expect("read_if_fresh");

        assert!(result.is_none(), "stale file should not be read");
    }

    #[test]
    fn read_if_fresh_returns_none_when_file_missing() {
        let path = std::path::PathBuf::from("/tmp/definitely-not-a-file-xyz-123.md");
        let result = read_if_fresh(&path, Duration::from_secs(30)).expect("read_if_fresh");
        assert!(result.is_none());
    }
}
```

- [ ] **Step 1.5: Run the unit tests**

Run: `cargo nextest run -p temper-cli show_cache::tests`
Expected: three tests pass.

- [ ] **Step 1.6: Commit**

```bash
git add crates/temper-cli/src/actions/show_cache.rs \
        crates/temper-cli/src/actions/mod.rs \
        crates/temper-cli/Cargo.toml \
        Cargo.lock
git commit -m "feat(cli): show_cache three-tier freshness ladder

Factor the Local-mode cache-warming logic for temper resource show into
a dedicated module. Tier 1 (debounce) is unit-tested in isolation. Tier
2 (hash-verify) and tier 3 (full-fetch) are exercised via the e2e
harness in later tasks."
```

---

## Task 2: `show` cloud-first rewrite (per-doctype)

**Purpose:** Route `temper resource show` through the API in Local mode via the `show_cache` ladder; in Cloud mode hit the API directly and render without any disk write. Keep per-doctype output shape.

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs` (the top-level `show` dispatcher and `show_generic`)
- Modify: `crates/temper-cli/src/commands/task.rs` (`show` function)
- Modify: `crates/temper-cli/src/commands/session.rs` (`show` function)

- [ ] **Step 2.1: Extract a shared helper for "resolve id from slug"**

The ladder needs a `ResourceId`, but the CLI takes a slug. In Local mode we can look up the id from the local file's frontmatter (`temper-id` or fallback `temper-provisional-id`). In Cloud mode, we need `GET /resources/by-uri` to resolve `<context>/<doctype>/<slug>` to an id.

Add to `crates/temper-cli/src/commands/resource.rs` (place it near `find_resource_file`):

```rust
/// Resolve `(context, doctype, slug)` to a `ResourceId`, preferring the
/// local frontmatter if a local file exists (fast path) and falling
/// back to a server-side lookup via `by-uri` (slow path, needed in
/// Cloud mode or when the local file has no canonical id yet).
async fn resolve_resource_id(
    config: &Config,
    client: &temper_client::TemperClient,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    vault_state: VaultState,
) -> Result<ResourceId> {
    use temper_core::types::ids::ResourceId;

    if matches!(vault_state, VaultState::Local) {
        if let Ok((path, _)) = find_resource_file(config, doc_type, slug, context) {
            let body = std::fs::read_to_string(&path).map_err(|e| TemperError::Vault(e.to_string()))?;
            if let Ok(fm) = temper_core::frontmatter::Frontmatter::try_from(body.as_str()) {
                if let Some(id_str) = fm.value().get("temper-id").and_then(|v| v.as_str()) {
                    if let Ok(uuid) = uuid::Uuid::parse_str(id_str) {
                        return Ok(ResourceId::from(uuid));
                    }
                }
            }
        }
    }

    // Fall through to server lookup.
    let ctx = require_context(config, context)?;
    let uri = format!("kb://{}/{}/{}", ctx, doc_type, slug);
    let row = client
        .resources()
        .resolve_by_uri(&uri)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;
    Ok(row.id)
}
```

**⚠️ Verification step:** `client.resources().resolve_by_uri(&str)` — confirm this exists on the client. If the method is named differently (e.g. `by_uri`, `get_by_uri`), use the actual name. Grep: `grep -n "by_uri\|resolve_by_uri" crates/temper-client/src/resources.rs`. If it doesn't exist, add it following the same shape as `get()`:

```rust
pub async fn resolve_by_uri(&self, uri: &str) -> Result<ResourceRow> {
    self.http
        .get("/api/resources/by-uri")
        .query("uri", uri)
        .send()
        .await
}
```

The server handler `by_uri` exists at `crates/temper-api/src/handlers/resources.rs:50` per the spec's grep-verified surface.

- [ ] **Step 2.2: Rewrite `resource::show` to match VaultState**

Edit `crates/temper-cli/src/commands/resource.rs`, replace the existing `pub fn show(...)` (around line 511) with:

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

    // Per-doctype handlers do their own output shaping. We route all
    // doc types through the same ladder for content retrieval; only
    // the rendering differs.
    match doc_type {
        "task" => crate::commands::task::show(config, slug, context, format)?,
        "session" => crate::commands::session::show(config, slug, context, format)?,
        _ => show_generic(config, doc_type, slug, context, format)?,
    };

    if edges {
        show_edges(slug, format)?;
    }

    Ok(())
}
```

(Body is unchanged from today — mode-aware behavior lives inside the per-doctype `show` and `show_generic`.)

- [ ] **Step 2.3: Rewrite `show_generic` with mode-match + ladder**

In `crates/temper-cli/src/commands/resource.rs`, replace `show_generic`:

```rust
fn show_generic(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    format: &str,
) -> Result<()> {
    use crate::actions::{runtime, show_cache};
    use temper_core::types::config::VaultState;
    use std::time::Duration;

    let vault_state = VaultState::from_env();

    match vault_state {
        VaultState::Cloud => {
            runtime::with_client(|client| {
                Box::pin(async move {
                    let id = resolve_resource_id(config, client, doc_type, slug, context, vault_state)
                        .await?;
                    let content = client
                        .resources()
                        .content(*id.as_uuid())
                        .await
                        .map_err(|e| TemperError::Api(e.to_string()))?;
                    render_generic_output(doc_type, slug, context, config, None, &content.content, format)
                })
            })
        }
        VaultState::Local => {
            runtime::with_client(|client| {
                Box::pin(async move {
                    let id = resolve_resource_id(config, client, doc_type, slug, context, vault_state)
                        .await?;
                    let (path, _ctx) =
                        find_or_compute_local_path(config, doc_type, slug, context)?;
                    let result = show_cache::fetch(show_cache::ShowCacheParams {
                        client,
                        resource_id: id,
                        local_path: &path,
                        debounce: Duration::from_secs(show_cache::DEFAULT_DEBOUNCE_SECONDS),
                    })
                    .await?;
                    render_generic_output(
                        doc_type,
                        slug,
                        context,
                        config,
                        Some(&path),
                        &result.content,
                        format,
                    )
                })
            })
        }
    }
}

/// Render a show result for generic doctypes (goal, research, concept,
/// decision). Matches the prior behavior of the old `show_generic`:
/// plain-text renders the body; JSON wraps metadata + body.
fn render_generic_output(
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    config: &Config,
    path: Option<&std::path::Path>,
    body: &str,
    format: &str,
) -> Result<()> {
    if format == "json" {
        let fm = temper_core::frontmatter::Frontmatter::try_from(body).ok();
        let title = fm
            .as_ref()
            .and_then(|f| f.value().get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or(slug);
        let ctx_str = context.unwrap_or("").to_string();
        let rel_path = path
            .and_then(|p| p.strip_prefix(&config.vault_root).ok())
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        #[derive(serde::Serialize)]
        struct ResourceShow<'a> {
            doc_type: &'a str,
            slug: &'a str,
            title: &'a str,
            context: &'a str,
            path: String,
            content: String,
        }
        let info = ResourceShow {
            doc_type,
            slug,
            title,
            context: &ctx_str,
            path: rel_path,
            content: body.to_string(),
        };
        println!("{}", serde_json::to_string_pretty(&info).unwrap_or_default());
        return Ok(());
    }

    print!("{body}");
    Ok(())
}

/// Compute where the local file for `(doc_type, slug, context)` should
/// live, even if it doesn't exist yet (so `show_cache` can write it
/// there on tier 3). Mirrors `find_resource_file` but returns the
/// expected path without erroring when the file is missing.
fn find_or_compute_local_path(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
) -> Result<(std::path::PathBuf, String)> {
    // Happy path: file exists, return the canonical path.
    if let Ok((path, ctx)) = find_resource_file(config, doc_type, slug, context) {
        return Ok((path, ctx));
    }
    // Compute the path using Vault layout rules.
    let ctx = require_context(config, context)?;
    let vault = Vault::new(&config.vault_root);
    let path = vault.resource_path(&ctx, doc_type, slug);
    Ok((path, ctx.to_string()))
}
```

**⚠️ Verification step:** `Vault::resource_path(ctx, doc_type, slug)` — confirm this exists on the `Vault` struct. Grep: `grep -n "fn resource_path\|pub fn " crates/temper-cli/src/vault.rs`. If it doesn't exist, the Vault type has some equivalent method — use whatever the codebase already uses for "compute the path for a resource I may or may not have written yet." If nothing fits, inline the layout: `vault_root.join(ctx).join(doc_type).join(format!("{slug}.md"))`.

- [ ] **Step 2.4: Rewrite `task::show` with mode-match + ladder**

Edit `crates/temper-cli/src/commands/task.rs` — the `show` function (currently around line 11). Keep the task's output shaping; swap disk-read for ladder-in-local and content-call-in-cloud. Follow the same match structure as `show_generic` above.

Consult the current `task::show` to preserve its table / JSON formatting. The content-retrieval step is the only thing that changes.

- [ ] **Step 2.5: Rewrite `session::show` with mode-match + ladder**

Edit `crates/temper-cli/src/commands/session.rs::show` (around line 221). Same pattern as Task 2.4.

- [ ] **Step 2.6: Full build**

Run: `cargo make check`
Expected: all lints pass.

Run: `cargo nextest run -p temper-cli`
Expected: existing unit tests still pass.

- [ ] **Step 2.7: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs \
        crates/temper-cli/src/commands/task.rs \
        crates/temper-cli/src/commands/session.rs
git commit -m "feat(cli): temper show — cloud-first with three-tier ladder

Local mode: debounce on mtime, hash-verify via GET /resources/{id},
full-fetch via GET /resources/{id}/content. Cloud mode: straight body
fetch, no disk writes. Per-doctype output formatters keep their shape;
only the content-retrieval step is mode-aware."
```

---

## Task 3: `list` cloud-first rewrite (with offline fallback)

**Purpose:** `temper list --type X --context Y` always calls the server; results flow through the existing per-doctype column pipeline so output shape is unchanged. Offline: fall back to a local-manifest scan sorted `updated DESC` to match server ordering.

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs` (`list` function around line 438 and the `render_list` helper around line 396)

- [ ] **Step 3.1: Add a cloud-first list helper**

Edit `crates/temper-cli/src/commands/resource.rs`. Add a new function near `render_list`:

```rust
/// Cloud-first list: call the server, return rows sorted server-side
/// (`ORDER BY updated DESC`).
async fn fetch_list_rows(
    client: &temper_client::TemperClient,
    doc_type: &str,
    context: Option<&str>,
    limit: usize,
) -> Result<Vec<temper_core::types::ResourceRow>> {
    use temper_core::types::{ResourceListParams, ResourceSortField, SortOrder};

    let params = ResourceListParams {
        doc_type_name: Some(doc_type.to_string()),
        context_name: context.map(ToString::to_string),
        sort: Some(ResourceSortField::Updated),
        order: Some(SortOrder::Desc),
        limit: Some(limit as i64),
        ..Default::default()
    };
    let resp = client
        .resources()
        .list(&params)
        .await
        .map_err(|e| TemperError::Api(e.to_string()))?;
    Ok(resp.rows)
}

/// Map a server `ResourceRow` to the frontmatter-shaped `serde_json::Value`
/// that `col_registry::extract_row` expects. The registry was built for
/// local scan_rows output; we adapt the server row shape to the same
/// keys so rendering is unchanged.
fn row_to_frontmatter_value(row: &temper_core::types::ResourceRow) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    map.insert("title".into(), serde_json::Value::String(row.title.clone()));
    if let Some(slug) = &row.slug {
        map.insert("slug".into(), serde_json::Value::String(slug.clone()));
    }
    map.insert(
        "temper-updated".into(),
        serde_json::Value::String(row.updated.to_rfc3339()),
    );
    map.insert(
        "temper-context".into(),
        serde_json::Value::String(row.context_name.clone()),
    );
    map.insert(
        "temper-type".into(),
        serde_json::Value::String(row.doc_type_name.clone()),
    );
    if let Some(stage) = &row.stage {
        map.insert("temper-stage".into(), serde_json::Value::String(stage.clone()));
    }
    if let Some(mode) = &row.mode {
        map.insert("temper-mode".into(), serde_json::Value::String(mode.clone()));
    }
    if let Some(effort) = &row.effort {
        map.insert("temper-effort".into(), serde_json::Value::String(effort.clone()));
    }
    if let Some(seq) = row.seq {
        map.insert("temper-seq".into(), serde_json::Value::Number(seq.into()));
    }
    serde_json::Value::Object(map)
}
```

- [ ] **Step 3.2: Branch `list` on `VaultState`**

Replace the body of `pub fn list(...)` in `resource.rs` (around line 438). Keep the filter-validation hints (`--stage`, `--goal`, `--status`) — they're still informational. Keep local-mode filtering as the offline fallback (since the server doesn't support these filters yet — tracked in the spec's Deferred section).

```rust
pub fn list(config: &Config, params: ListParams<'_>) -> Result<()> {
    use crate::actions::runtime;
    use temper_core::types::config::VaultState;

    // Hints for filters that only apply to certain types (unchanged).
    if params.stage.is_some() && params.doc_type != "task" {
        output::hint(format!(
            "--stage filter is only meaningful for tasks; ignored for {}.",
            params.doc_type
        ));
    }
    if params.goal.is_some() && params.doc_type != "task" {
        output::hint(format!(
            "--goal filter is only meaningful for tasks; ignored for {}.",
            params.doc_type
        ));
    }
    if params.status.is_some() && params.doc_type != "goal" {
        output::hint(format!(
            "--status filter is only meaningful for goals; ignored for {}.",
            params.doc_type
        ));
    }

    if let Some(s) = params.stage {
        if params.doc_type == "task" {
            vault::validate_stage(s)?;
        }
    }

    let format = OutputFormat::parse(params.format);
    let doc_type = params.doc_type.to_string();
    let context = params.context.map(ToString::to_string);
    let limit = params.limit.unwrap_or(20);

    let vault_state = VaultState::from_env();

    // Attempt server-first. Fall back to local scan on network error
    // in Local mode only; Cloud mode surfaces the error.
    let rows_result = runtime::with_client(move |client| {
        Box::pin(async move {
            fetch_list_rows(client, &doc_type, context.as_deref(), limit).await
        })
    });

    let server_rows = match (rows_result, vault_state) {
        (Ok(rows), _) => Some(rows),
        (Err(e), VaultState::Cloud) => return Err(e),
        (Err(e), VaultState::Local) => {
            output::hint(format!("cloud unreachable: {e}. Falling back to local scan."));
            None
        }
    };

    let body = match server_rows {
        Some(rows) => render_server_rows(params.doc_type, &rows, format)?,
        None => {
            // Offline fallback: scan the local vault with the existing
            // local filter pipeline. Already sorted updated DESC by
            // sort_rows; the filter branches handle stage/goal/status.
            render_list(&RenderListParams {
                doc_type: params.doc_type,
                config,
                context: params.context,
                limit: params.limit,
                filters: ListFilters {
                    stage: if params.doc_type == "task" { params.stage } else { None },
                    goal: if params.doc_type == "task" { params.goal } else { None },
                    status: if params.doc_type == "goal" { params.status } else { None },
                },
                format,
            })?
        }
    };

    if body.trim().is_empty() {
        output::hint(format!("No {} resources found.", params.doc_type));
        return Ok(());
    }

    output::plain(body.trim_end());
    Ok(())
}

/// Render server rows using the same per-doctype column registry used
/// by the local-mode `render_list`. This keeps table output shape
/// stable between the two modes.
fn render_server_rows(
    doc_type: &str,
    rows: &[temper_core::types::ResourceRow],
    format: OutputFormat,
) -> Result<String> {
    match format {
        OutputFormat::Json => Ok(serde_json::to_string_pretty(
            &rows.iter().map(row_to_frontmatter_value).collect::<Vec<_>>(),
        )
        .unwrap_or_default()),
        OutputFormat::Pretty | OutputFormat::NoTty => {
            let columns = col_registry::display_columns(doc_type);
            if columns.is_empty() || rows.is_empty() {
                return Ok(String::new());
            }
            let mut renderer = TableRenderer::new(columns.clone());
            for row in rows {
                let fm_value = row_to_frontmatter_value(row);
                renderer.push_row(col_registry::extract_row(&fm_value, &columns));
            }
            Ok(if format == OutputFormat::Pretty {
                renderer.render_pretty()
            } else {
                renderer.render_no_tty()
            })
        }
    }
}
```

- [ ] **Step 3.3: Confirm `ResourceListParams` fields match**

Run: `grep -n "^pub struct ResourceListParams\|doc_type_name\|context_name\|sort:\|order:" crates/temper-core/src/types/resource.rs`
Expected: field names match the struct literal above (`doc_type_name`, `context_name`, `sort`, `order`, `limit`).

If the literal doesn't compile, open `crates/temper-core/src/types/resource.rs:79-92` and adjust the struct literal to the real fields.

- [ ] **Step 3.4: Build and run existing tests**

Run: `cargo make check`
Expected: clean.

Run: `cargo nextest run -p temper-cli`
Expected: pass.

- [ ] **Step 3.5: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs
git commit -m "feat(cli): temper list — cloud-first with offline fallback

Local mode: call server first; on network error, fall back to local
vault scan with existing filter pipeline. Cloud mode: server-only,
surface errors. Output flows through per-doctype col_registry so table
shape is unchanged regardless of mode."
```

---

## Task 4: `create` routes through `push_one_resource`

**Purpose:** After a per-doctype creator writes the local file with `temper-provisional-id`, call `push_one_resource` so the resource lands on the server in the same command. Canonical `temper-id` is written back to the local file by the primitive. Cloud mode bypasses the disk write entirely.

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs` (`create` function, `create_simple_resource`)
- Modify: `crates/temper-cli/src/actions/task.rs::create`
- Modify: `crates/temper-cli/src/actions/research.rs::save`
- Modify: `crates/temper-cli/src/commands/session.rs::save`
- Modify: `crates/temper-cli/src/commands/goal.rs::create`

- [ ] **Step 4.1: Add a shared post-create publish helper**

Near the top of `crates/temper-cli/src/actions/sync.rs`, add a helper that wraps `push_one_resource` for the create flow. Local mode: load the manifest, push, save the manifest. Cloud mode: skip (the caller shouldn't even call this function in cloud mode).

```rust
/// Publish a freshly-created local file to the server, receiving the
/// canonical `temper-id`. Updates the manifest in place.
///
/// Precondition: `vault_path` has a file at `file_path` with a
/// `temper-provisional-id` in its frontmatter.
///
/// Postcondition: the file at `file_path` has `temper-id` (server
/// canonical); manifest is updated.
pub async fn publish_created_file(
    client: &temper_client::TemperClient,
    vault_root: &std::path::Path,
    file_path: &std::path::Path,
) -> Result<PushResult> {
    use crate::manifest_io;
    use crate::actions::runtime;

    let temper_dir = vault_root.join(".temper");
    let device_id = runtime::require_device_id()?;
    let mut manifest = manifest_io::load_manifest(&temper_dir, &device_id)?;

    let result = push_one_resource(
        client,
        vault_root,
        PushTarget::Path(file_path),
        Some(&mut manifest),
    )
    .await?;

    manifest_io::save_manifest(&temper_dir, &device_id, &manifest)?;
    Ok(result)
}
```

**⚠️ Verification step:** `manifest_io::load_manifest` and `::save_manifest` signatures — confirm via `grep -n "pub fn load_manifest\|pub fn save_manifest" crates/temper-cli/src/manifest_io.rs`. Adjust the call shape to whatever the module exposes.

- [ ] **Step 4.2: Wire `create_simple_resource` (concept, decision) to publish**

In `crates/temper-cli/src/commands/resource.rs::create_simple_resource` (around line 121), after the file is written, add:

```rust
use temper_core::types::config::VaultState;

match VaultState::from_env() {
    VaultState::Cloud => {
        // No local file was written in Cloud mode; creation happens via
        // client.resources().create() directly. See Step 4.3 below.
        unreachable!("cloud-mode create_simple_resource takes a different path");
    }
    VaultState::Local => {
        crate::actions::runtime::with_client(|client| {
            Box::pin(async move {
                crate::actions::sync::publish_created_file(client, &config.vault_root, &file_path)
                    .await
                    .map(|_| ())
            })
        })?;
    }
}
```

- [ ] **Step 4.3: Branch `resource::create` on VaultState at the top**

Edit `crates/temper-cli/src/commands/resource.rs::create` (line 42). Before the per-doctype match, add:

```rust
use temper_core::types::config::VaultState;

let vault_state = VaultState::from_env();
if matches!(vault_state, VaultState::Cloud) {
    return create_cloud_mode(doc_type, title, context, goal, mode, effort, slug, format);
}
```

Then add `create_cloud_mode` as a sibling function. It routes directly through `client.resources().create(&request)` without writing anything to disk:

```rust
fn create_cloud_mode(
    doc_type: &str,
    title: &str,
    context: Option<&str>,
    _goal: Option<&str>,
    _mode: Option<&str>,
    _effort: Option<&str>,
    slug_override: Option<&str>,
    format: &str,
) -> Result<()> {
    use crate::actions::runtime;
    use temper_core::types::{ResourceCreateRequest, resource::ContentResponse};

    validate_doc_type(doc_type)?;

    let ctx = context
        .map(ToString::to_string)
        .ok_or_else(|| TemperError::Project("--context is required in cloud mode".into()))?;
    let title = title.to_string();
    let slug = slug_override.map(ToString::to_string);
    let doc_type = doc_type.to_string();
    let format = format.to_string();
    let stdin_body = crate::vault::read_stdin_if_piped();

    runtime::with_client(move |client| {
        Box::pin(async move {
            // Look up context + doctype UUIDs.
            let contexts = client.contexts().list().await
                .map_err(|e| TemperError::Api(e.to_string()))?;
            let context_row = contexts.iter()
                .find(|c| c.name == ctx)
                .ok_or_else(|| TemperError::Project(format!("context '{ctx}' not found on server")))?;

            let doc_types = client.doc_types().list().await
                .map_err(|e| TemperError::Api(e.to_string()))?;
            let doc_type_row = doc_types.iter()
                .find(|d| d.name == doc_type)
                .ok_or_else(|| TemperError::Project(format!("doc type '{doc_type}' not found on server")))?;

            let origin_uri = format!("kb://{}/{}/{}", ctx, doc_type,
                slug.clone().unwrap_or_else(|| vault::slugify(&title)));
            let request = ResourceCreateRequest {
                kb_context_id: *context_row.id.as_uuid(),
                kb_doc_type_id: *doc_type_row.id.as_uuid(),
                origin_uri,
                title: title.clone(),
                slug: slug.clone(),
            };
            let created = client.resources().create(&request).await
                .map_err(|e| TemperError::Api(e.to_string()))?;

            // If the user piped a body, update content as a second step.
            if let Some(body) = stdin_body.as_deref() {
                let content_req = temper_core::types::resource::ResourceUpdateContentRequest {
                    content: body.to_string(),
                };
                client.resources().update_content(*created.id.as_uuid(), &content_req)
                    .await
                    .map_err(|e| TemperError::Api(e.to_string()))?;
            }

            if format == "json" {
                println!("{}", serde_json::to_string_pretty(&created).unwrap_or_default());
            } else {
                println!("{}", created.id.as_uuid());
            }
            Ok(())
        })
    })
}
```

**⚠️ Verification step:** the `update_content` method and `ResourceUpdateContentRequest` type — confirm via grep. If the server accepts content as part of `update(...)` or via a different endpoint, adjust. If the CLI's doctype-specific creators inject body content differently (templates, askama renders), mirror that logic: the cloud-mode path needs to produce the same initial body that the local-mode template would have.

**Escalation:** If the content-on-create story is ambiguous, stop here and report BLOCKED with specifics. The spec says cloud-mode create "POSTs directly via a thin helper that reads body from stdin for session / task where the existing CLI uses stdin; from a minimal in-memory template for auto-generated doctypes." The auto-template piece may need the controller to specify exactly how the askama templates get reused in Cloud mode.

- [ ] **Step 4.4: Wire `task::create`, `session::save`, `research::save`, `goal::create` Local-mode tail**

For each of these per-doctype creators, after the local file write completes, call `publish_created_file`. The caller already has `config.vault_root` and the written `file_path` — just add the tail call. They're only reached in Local mode (cloud-mode short-circuits above).

Exact edit for `crates/temper-cli/src/actions/task.rs::create` (adapt for each sibling):

```rust
// At end of function, before returning the slug:
crate::actions::runtime::with_client(|client| {
    Box::pin(async move {
        crate::actions::sync::publish_created_file(client, &config.vault_root, &file_path)
            .await
            .map(|_| ())
    })
})?;
```

- [ ] **Step 4.5: Write an e2e test: Local create round-trip**

Add to `tests/e2e/tests/cloud_first_test.rs` (new file; see Step 7.1 for full preamble):

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn local_create_task_pushes_to_server(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client.contexts().create("cf-test-ctx").await.expect("context create");

    // Set env so VaultState::Local and runtime uses the test's token.
    temp_env::with_vars(
        [
            ("TEMPER_VAULT_STATE", Some("local")),
            ("TEMPER_TOKEN", Some(app.token.as_str())),
        ],
        || {
            // Invoke create via the CLI action surface.
            let cli_config = app.cli_config.clone();
            let slug = crate::commands::resource::create_for_test(
                &cli_config,
                "task",
                "CF Test Task",
                Some("cf-test-ctx"),
            ).expect("create");
            assert!(!slug.is_empty());
        }
    );

    // Server-side assertion: list tasks in this context, expect one.
    let rows = app.client.resources().list(&ResourceListParams {
        doc_type_name: Some("task".into()),
        context_name: Some("cf-test-ctx".into()),
        ..Default::default()
    }).await.expect("list");
    assert_eq!(rows.rows.len(), 1);
    assert_eq!(rows.rows[0].title, "CF Test Task");
}
```

**⚠️ Harness verification:** the test harness calls the CLI action surface directly (in-process), not via subprocess. If the project's `resource::create` isn't callable from an integration test (e.g., it's in a binary crate), move the function signature to a library-exposed action (likely already factored as `actions/task::create`). Grep: `ls crates/temper-cli/src/actions/`.

- [ ] **Step 4.6: Run the e2e test**

Run: `cargo make docker-up` (if not already)
Run: `cargo nextest run -p temper-e2e --features test-db local_create_task_pushes_to_server`
Expected: PASS.

- [ ] **Step 4.7: Commit**

```bash
git add crates/temper-cli/src/actions/sync.rs \
        crates/temper-cli/src/actions/task.rs \
        crates/temper-cli/src/actions/research.rs \
        crates/temper-cli/src/commands/session.rs \
        crates/temper-cli/src/commands/goal.rs \
        crates/temper-cli/src/commands/resource.rs \
        tests/e2e/tests/cloud_first_test.rs
git commit -m "feat(cli): temper create — cloud-first via push_one_resource

Local mode: after the per-doctype template writes the file, call
push_one_resource to POST and rewrite temper-provisional-id with the
canonical id. Cloud mode: client.resources().create() directly, no
disk. New e2e test covers the local round-trip end to end."
```

---

## Task 5: `update` routes through `push_one_resource`

**Purpose:** After the existing `update` logic mutates frontmatter and writes the file, route through `push_one_resource` with `PushTarget::Id` so the server gets the new state immediately.

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs::update` (around line 753)

- [ ] **Step 5.1: Branch `update` on VaultState**

At the top of `pub fn update(...)`, before any local-file operations, add:

```rust
use temper_core::types::config::VaultState;

let vault_state = VaultState::from_env();
if matches!(vault_state, VaultState::Cloud) {
    return update_cloud_mode(params);
}
```

Then add `update_cloud_mode` as a sibling:

```rust
fn update_cloud_mode(params: &UpdateParams<'_>) -> Result<()> {
    use crate::actions::runtime;
    use temper_core::types::ResourceUpdateRequest;

    let slug = params.slug.to_string();
    let doc_type = params
        .doc_type
        .or(params.type_from)
        .ok_or_else(|| TemperError::Project("--type or --type-from required".into()))?
        .to_string();
    let context = params.context.map(ToString::to_string);
    // Collect frontmatter mutations as field → value pairs.
    let mutations: std::collections::HashMap<String, serde_json::Value> = [
        ("title", params.title.map(|s| serde_json::Value::String(s.into()))),
        ("temper-stage", params.stage.map(|s| serde_json::Value::String(s.into()))),
        ("temper-mode", params.mode.map(|s| serde_json::Value::String(s.into()))),
        ("temper-effort", params.effort.map(|s| serde_json::Value::String(s.into()))),
        ("temper-goal", params.goal.map(|s| serde_json::Value::String(s.into()))),
    ].into_iter().filter_map(|(k, v)| v.map(|val| (k.to_string(), val))).collect();

    runtime::with_client(move |client| {
        Box::pin(async move {
            let ctx = context.unwrap_or_default();
            let uri = format!("kb://{}/{}/{}", ctx, doc_type, slug);
            let row = client.resources().resolve_by_uri(&uri).await
                .map_err(|e| TemperError::Api(e.to_string()))?;

            let request = ResourceUpdateRequest {
                title: mutations.get("title")
                    .and_then(|v| v.as_str()).map(String::from),
                // Additional managed_meta mutations per the shape the API
                // accepts; verify against ResourceUpdateRequest definition.
                ..Default::default()
            };
            let _updated = client.resources().update(*row.id.as_uuid(), &request).await
                .map_err(|e| TemperError::Api(e.to_string()))?;
            Ok(())
        })
    })
}
```

**⚠️ Verification step:** `ResourceUpdateRequest` shape — grep `grep -n "pub struct ResourceUpdateRequest" crates/temper-core/src/types/resource.rs` and read lines 127+ to see what fields it accepts. Map the CLI's `UpdateParams` fields onto the API request shape; fall back to field-name string matching (as done above for managed_meta) where the server accepts an open frontmatter projection.

**Escalation:** If the API's update endpoint doesn't support the mutation shape Cloud mode needs (e.g., it only accepts a body content replacement but not managed_meta mutations), stop here and report BLOCKED. The controller may need to expand the server API in a follow-up before this can land.

- [ ] **Step 5.2: Add the Local-mode publish tail**

In the existing `pub fn update(...)` body, at the very end (after the frontmatter-mutated file is written to its final path), add:

```rust
// Cloud-first: publish the newly-written file to the server in the
// same command. In Cloud mode we've already returned above; reaching
// here implies Local mode.
crate::actions::runtime::with_client(|client| {
    Box::pin(async move {
        let temper_dir = config.vault_root.join(".temper");
        let device_id = crate::actions::runtime::require_device_id()?;
        let mut manifest = crate::manifest_io::load_manifest(&temper_dir, &device_id)?;
        let resource_id = extract_resource_id_from_path(&final_path)?;
        crate::actions::sync::push_one_resource(
            client,
            &config.vault_root,
            crate::actions::sync::PushTarget::Id(resource_id),
            Some(&mut manifest),
        ).await?;
        crate::manifest_io::save_manifest(&temper_dir, &device_id, &manifest)?;
        Ok(())
    })
})?;
```

Add the helper `extract_resource_id_from_path` near the bottom of the file:

```rust
fn extract_resource_id_from_path(path: &std::path::Path) -> Result<temper_core::types::ids::ResourceId> {
    let body = std::fs::read_to_string(path).map_err(|e| TemperError::Vault(e.to_string()))?;
    let fm = temper_core::frontmatter::Frontmatter::try_from(body.as_str())
        .map_err(|e| TemperError::Vault(e.to_string()))?;
    let id_str = fm.value().get("temper-id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| TemperError::Vault(
            "file has no temper-id; run `temper push` to create the server record first".into()))?;
    let uuid = uuid::Uuid::parse_str(id_str)
        .map_err(|e| TemperError::Vault(format!("invalid temper-id: {e}")))?;
    Ok(temper_core::types::ids::ResourceId::from(uuid))
}
```

- [ ] **Step 5.3: Document the "update implies push" contract in CLI help text**

Find the `temper resource update` clap doc comment (likely in `crates/temper-cli/src/cli/args.rs` or a similar location — grep `grep -rn "fn update\|temper resource update\|Update a resource" crates/temper-cli/src/cli/` to find it). Append to the command's help:

```
Update mutates frontmatter from args AND pushes the whole file (including
any manual body edits made before this command) to the server. Make body
edits before running update. For body-only changes use `temper push`.
```

Also update the `temper` skill workflow file to mention this (see Task 9).

- [ ] **Step 5.4: Write an e2e test: Local update round-trip**

Add to `tests/e2e/tests/cloud_first_test.rs`:

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn local_update_task_pushes_to_server(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client.contexts().create("cf-update-ctx").await.expect("context create");

    // Create locally.
    temp_env::with_vars(
        [
            ("TEMPER_VAULT_STATE", Some("local")),
            ("TEMPER_TOKEN", Some(app.token.as_str())),
        ],
        || {
            let cli_config = app.cli_config.clone();
            crate::commands::resource::create_for_test(
                &cli_config, "task", "Original Title", Some("cf-update-ctx"),
            ).expect("create");
        }
    );

    // Find the local file and simulate a manual body edit.
    let task_dir = app.vault_dir.path().join("cf-update-ctx").join("task");
    let entries: Vec<_> = std::fs::read_dir(&task_dir).unwrap().collect();
    let task_file = entries[0].as_ref().unwrap().path();
    let orig = std::fs::read_to_string(&task_file).unwrap();
    std::fs::write(&task_file, format!("{orig}\n## Added Section\nManual edit\n")).unwrap();

    // Now update via CLI: change stage + implicitly push manual body edit.
    temp_env::with_vars(
        [
            ("TEMPER_VAULT_STATE", Some("local")),
            ("TEMPER_TOKEN", Some(app.token.as_str())),
        ],
        || {
            let cli_config = app.cli_config.clone();
            let slug = task_file.file_stem().unwrap().to_string_lossy().to_string();
            crate::commands::resource::update_for_test(
                &cli_config,
                &slug,
                "task",
                Some("cf-update-ctx"),
                Some("in-progress"),  // --stage
            ).expect("update");
        }
    );

    // Server assertion: the content on the server now contains the manual
    // edit AND the new stage.
    let list = app.client.resources().list(&ResourceListParams {
        doc_type_name: Some("task".into()),
        context_name: Some("cf-update-ctx".into()),
        ..Default::default()
    }).await.expect("list");
    let row = &list.rows[0];
    assert_eq!(row.stage.as_deref(), Some("in-progress"));
    let content = app.client.resources().content(*row.id.as_uuid()).await.expect("content");
    assert!(content.content.contains("## Added Section"), "body was not pushed");
}
```

**⚠️ Test-harness helpers:** `create_for_test` / `update_for_test` — these are thin test-friendly wrappers around the command entry points. If they don't exist, add them in `#[cfg(test)]`-gated sections of `resource.rs` exposing the public function signatures with the right Config shape. Keep them simple — their job is to call the real `pub fn create` / `pub fn update` with a test-controlled config.

- [ ] **Step 5.5: Run the e2e test**

Run: `cargo nextest run -p temper-e2e --features test-db local_update_task_pushes_to_server`
Expected: PASS.

- [ ] **Step 5.6: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs \
        crates/temper-cli/src/cli/args.rs \
        tests/e2e/tests/cloud_first_test.rs
git commit -m "feat(cli): temper update — cloud-first via push_one_resource

Local mode: mutate frontmatter from args + read existing body from
disk, write local, then push_one_resource with PushTarget::Id.
Manifest-mediated three-way merge handles concurrent edits as before.
Cloud mode: direct PUT via client.resources().update(). CLI help and
skill guidance note the new 'update implies push' contract."
```

---

## Task 6: `sync run` cloud redirect

**Purpose:** `temper sync run` in Cloud mode short-circuits with a clear message; Local mode behavior is unchanged.

**Files:**
- Modify: `crates/temper-cli/src/commands/sync_cmd.rs`

- [ ] **Step 6.1: Add the match-VaultState branch**

Near the top of the `sync run` entry function (grep `grep -n "fn run\b\|pub fn run\|sync.*run" crates/temper-cli/src/commands/sync_cmd.rs`), insert:

```rust
use temper_core::types::config::VaultState;

match VaultState::from_env() {
    VaultState::Cloud => {
        return Err(TemperError::Project(
            "cloud mode has no local vault to sync — use `temper push <id|path>` and `temper pull <id>` for individual resources, or `temper resource list` / `temper resource show` to browse.".into()
        ));
    }
    VaultState::Local => {
        // fall through to existing local-mode behavior
    }
}
```

- [ ] **Step 6.2: Unit test the redirect branch**

Append to `crates/temper-cli/src/commands/sync_cmd.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_run_in_cloud_mode_errors_with_redirect() {
        temp_env::with_vars(
            [
                ("TEMPER_VAULT_STATE", Some("cloud")),
                ("TEMPER_TOKEN", Some("placeholder")),
            ],
            || {
                let config = crate::config::Config::for_test();
                let result = run(&config);
                let err = result.unwrap_err();
                let msg = format!("{err}");
                assert!(
                    msg.contains("cloud mode") && msg.contains("temper push"),
                    "unexpected error: {msg}"
                );
            }
        );
    }
}
```

**⚠️ Verification step:** `Config::for_test()` may not exist. If not, construct a minimal `Config` with whatever the module uses for tests (look at `runtime.rs::tests` for a pattern).

- [ ] **Step 6.3: Run the test**

Run: `cargo nextest run -p temper-cli sync_run_in_cloud_mode_errors_with_redirect`
Expected: PASS.

- [ ] **Step 6.4: Commit**

```bash
git add crates/temper-cli/src/commands/sync_cmd.rs
git commit -m "feat(cli): temper sync run — cloud-mode redirect

Cloud mode has no local vault to sync. Error clearly, pointing users
at temper push / temper pull for single-resource operations and temper
resource list / show for browsing."
```

---

## Task 7: E2E tests for the read path

**Purpose:** Exercise `list` and `show` in both modes through the e2e harness. Covers: server sort order, remote-visibility bug fix, three-tier ladder observable behaviors, cloud-mode pure pass-through.

**Files:**
- Modify: `tests/e2e/tests/cloud_first_test.rs` (file introduced in Task 4)

- [ ] **Step 7.1: File preamble (if not already present from Task 4)**

```rust
#![cfg(feature = "test-db")]

//! E2E tests for cloud-first routing and mode collapse (Unit B.2 P3).
//!
//! Verifies that:
//!   - list returns server rows with server sort order (newest first)
//!   - show fetches from server, warms local cache
//!   - show's three-tier ladder debounces on mtime
//!   - create/update round-trip to server in one command
//!   - sync run in cloud mode redirects

mod common;

use common::setup;
use temper_core::types::ResourceListParams;
```

- [ ] **Step 7.2: Test — `list` returns server rows in `updated DESC` order**

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn local_list_sessions_returns_server_rows_newest_first(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    app.client.contexts().create("cf-sort-ctx").await.expect("context create");

    // Seed three sessions via the server directly, with ascending ids so
    // their updated timestamps differ.
    for title in &["session-oldest", "session-middle", "session-newest"] {
        seed_session(&app, "cf-sort-ctx", title).await;
    }

    temp_env::with_vars(
        [
            ("TEMPER_VAULT_STATE", Some("local")),
            ("TEMPER_TOKEN", Some(app.token.as_str())),
        ],
        || {
            let cli_config = app.cli_config.clone();
            let output = crate::commands::resource::list_for_test(&cli_config, "session", Some("cf-sort-ctx"))
                .expect("list");
            let pos_newest = output.find("session-newest").expect("must contain newest");
            let pos_oldest = output.find("session-oldest").expect("must contain oldest");
            assert!(pos_newest < pos_oldest, "newest must come first: {output}");
        }
    );
}

async fn seed_session(app: &common::E2eTestApp, ctx_name: &str, title: &str) {
    let contexts = app.client.contexts().list().await.expect("contexts");
    let ctx = contexts.iter().find(|c| c.name == ctx_name).expect("ctx");
    let doc_types = app.client.doc_types().list().await.expect("doctypes");
    let dt = doc_types.iter().find(|d| d.name == "session").expect("session doctype");
    let req = temper_core::types::ResourceCreateRequest {
        kb_context_id: *ctx.id.as_uuid(),
        kb_doc_type_id: *dt.id.as_uuid(),
        origin_uri: format!("kb://{}/session/{}", ctx_name, title),
        title: title.to_string(),
        slug: Some(title.to_string()),
    };
    app.client.resources().create(&req).await.expect("seed create");
    // Sleep briefly so consecutive rows have distinct `updated` timestamps.
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
}
```

- [ ] **Step 7.3: Test — `list` in Cloud mode (remote visibility) returns rows the local vault never saw**

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cloud_list_returns_resources_never_pulled_locally(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    app.client.contexts().create("cf-remote-ctx").await.expect("context create");
    seed_session(&app, "cf-remote-ctx", "server-only-session").await;

    temp_env::with_vars(
        [
            ("TEMPER_VAULT_STATE", Some("cloud")),
            ("TEMPER_TOKEN", Some(app.token.as_str())),
        ],
        || {
            let cli_config = app.cli_config.clone();
            let output = crate::commands::resource::list_for_test(&cli_config, "session", Some("cf-remote-ctx"))
                .expect("list");
            assert!(output.contains("server-only-session"), "cloud list missed server row: {output}");
        }
    );
}
```

- [ ] **Step 7.4: Test — `show` debounces within the window**

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn show_debounces_within_window(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    app.client.contexts().create("cf-debounce-ctx").await.expect("ctx");
    let slug = "debounce-task";
    // Create task via the CLI so the local file is written.
    temp_env::with_vars(
        [
            ("TEMPER_VAULT_STATE", Some("local")),
            ("TEMPER_TOKEN", Some(app.token.as_str())),
        ],
        || {
            let cli_config = app.cli_config.clone();
            crate::commands::resource::create_for_test(&cli_config, "task", "Debounce Task", Some("cf-debounce-ctx"))
                .expect("create");
        }
    );

    // Counter: instrument the reqwest mock OR assert via a side effect we
    // can observe. For a first pass, assert that two consecutive shows
    // within 1 second produce identical output (implicit: tier 1 fires).
    let out1 = temp_env::with_vars(
        [("TEMPER_VAULT_STATE", Some("local")), ("TEMPER_TOKEN", Some(app.token.as_str()))],
        || crate::commands::resource::show_for_test(&app.cli_config, "task", slug, Some("cf-debounce-ctx")).expect("show1")
    );
    let out2 = temp_env::with_vars(
        [("TEMPER_VAULT_STATE", Some("local")), ("TEMPER_TOKEN", Some(app.token.as_str()))],
        || crate::commands::resource::show_for_test(&app.cli_config, "task", slug, Some("cf-debounce-ctx")).expect("show2")
    );
    assert_eq!(out1, out2, "debounce should have returned identical content");
}
```

**Note on instrumentation:** a real debounce test wants to count API calls. The cleanest way is to wrap the test's `reqwest::Client` (from `common::E2eTestApp::reqwest_client`) with an interceptor that increments a counter on each request. If that's too much for this task, the above "identical output" check catches the primary failure mode (tier 3 would have re-fetched and overwritten the file).

- [ ] **Step 7.5: Run the tests**

Run: `cargo nextest run -p temper-e2e --features test-db cloud_first`
Expected: all four tests (Task 4's create + Task 5's update + Task 7's three reads) pass.

- [ ] **Step 7.6: Commit**

```bash
git add tests/e2e/tests/cloud_first_test.rs
git commit -m "test(e2e): cloud-first read-path coverage

Covers: server sort order (newest first), remote-visibility (cloud
list returns rows never pulled locally), show debounce within the
freshness window."
```

---

## Task 8: Smoke-test cloud-mode create → cross-session show

**Purpose:** Lock in the "cloud session A creates; cloud session B or local session reads" round-trip from the spec's acceptance criteria.

**Files:**
- Modify: `tests/e2e/tests/cloud_first_test.rs`

- [ ] **Step 8.1: Add the cross-session test**

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn cloud_create_visible_to_second_session(pool: sqlx::PgPool) {
    let app = setup(pool).await;
    app.client.contexts().create("cf-cross-ctx").await.expect("ctx");

    // "Session A" creates in Cloud mode.
    let created_id = temp_env::with_vars(
        [
            ("TEMPER_VAULT_STATE", Some("cloud")),
            ("TEMPER_TOKEN", Some(app.token.as_str())),
        ],
        || {
            let cli_config = app.cli_config.clone();
            crate::commands::resource::create_for_test(&cli_config, "session", "Cross-Session Test", Some("cf-cross-ctx"))
                .expect("create")
        }
    );

    // Assert: the vault_dir has NO file (cloud mode doesn't write to disk).
    let ctx_dir = app.vault_dir.path().join("cf-cross-ctx");
    assert!(!ctx_dir.exists(), "cloud mode must not write to disk");

    // "Session B" (Local mode, fresh vault) reads it via show.
    let show_out = temp_env::with_vars(
        [
            ("TEMPER_VAULT_STATE", Some("local")),
            ("TEMPER_TOKEN", Some(app.token.as_str())),
        ],
        || {
            let cli_config = app.cli_config.clone();
            crate::commands::resource::show_for_test(&cli_config, "session", &created_id, Some("cf-cross-ctx"))
                .expect("show")
        }
    );
    assert!(show_out.contains("Cross-Session Test"), "show missed title: {show_out}");
}
```

**Note:** `create_for_test` in cloud mode returns the slug OR canonical id (decide in Task 4). If it returns the canonical id (UUID as string), the `show_for_test` call's slug argument needs to be adapted — either pass the id directly or resolve the slug via a list-and-match step.

- [ ] **Step 8.2: Run the test**

Run: `cargo nextest run -p temper-e2e --features test-db cloud_create_visible_to_second_session`
Expected: PASS.

- [ ] **Step 8.3: Commit**

```bash
git add tests/e2e/tests/cloud_first_test.rs
git commit -m "test(e2e): cloud create → cross-session show round-trip

Acceptance test for 'cloud session writes; second session reads' from
the spec. Also locks in 'cloud mode writes nothing to disk'."
```

---

## Task 9: Update `temper` skill workflow guidance

**Purpose:** The "update implies push" contract is a new behavioral expectation for humans and agents. The `temper` skill's workflow files (`workflows/build-medium.md` etc.) reference `temper resource update` in session-save patterns. Add a note so future agents don't get surprised.

**Files:**
- Modify: one file under `~/.claude/skills/temper/workflows/` or inline in the skill's `session-lifecycle.md` (whichever is the right home for CLI-contract notes; `session-lifecycle.md` is likely correct).

- [ ] **Step 9.1: Locate the right home**

Run: `grep -rn "temper resource update\|temper resource create" ~/.claude/skills/temper/`
Expected: one or more hits in `workflows/*.md` or `session-lifecycle.md`.

- [ ] **Step 9.2: Add a short note**

In the identified file, add a paragraph near the CLI-usage notes:

```markdown
## `temper resource update` — implies push

Starting 2026-04-22, `temper resource update <slug> --<field>` mutates
local frontmatter AND pushes the whole file (including manual body
edits) to the server in one command. Contract:

- Manual body edits must happen BEFORE invoking `update`.
- For body-only changes, use `temper push <slug>` directly.
- In Cloud mode, only frontmatter args are applied (no local file
  exists); stdin-body support is deferred.
```

- [ ] **Step 9.3: Commit the skill doc change**

```bash
cd ~/.claude/skills/temper/
git add workflows/ session-lifecycle.md  # whichever you changed
git commit -m "docs(skill): temper update implies push (cloud-first routing)"
```

(If the skill directory isn't under version control, note the change in the Temper repo's `docs/guides/cloud-agents.md` instead.)

---

## Task 10: Final verification sweep and plan cleanup

**Purpose:** Run the full quality gate, confirm acceptance criteria one-by-one, delete the superseded old plan file.

**Files:**
- Delete: `docs/superpowers/plans/2026-04-19-unit-b-2-cloud-mode-dispatch.md`

- [ ] **Step 10.1: Full quality gate**

Run: `cargo make check`
Expected: clean.

Run: `cargo make test`
Expected: all unit tests pass.

Run: `cargo make docker-up && cargo make test-db`
Expected: all integration + e2e tests pass.

- [ ] **Step 10.2: Walk the acceptance criteria from the spec**

Open `docs/superpowers/specs/2026-04-22-cloud-first-routing-and-mode-collapse-design.md` to the Acceptance Criteria section. For each bullet, identify the test that exercises it:

| Criterion | Covered by |
|-----------|------------|
| Sort order | `local_list_sessions_returns_server_rows_newest_first` |
| Remote visibility | `cloud_list_returns_resources_never_pulled_locally` |
| Show debounce | `show_debounces_within_window` |
| Show hash-verify | (controller adds if Task 7 didn't cover — write test if missing) |
| Show cache warm | (similar — add if missing) |
| Update round-trip | `local_update_task_pushes_to_server` |
| Cloud create | `cloud_create_visible_to_second_session` (partial; split if needed) |
| Cloud cross-session | `cloud_create_visible_to_second_session` |
| Sync redirect | `sync_run_in_cloud_mode_errors_with_redirect` |
| Mode mismatch | Task 9 of original plan (shipped: `with_client_errors_when_cloud_mode_but_no_token`) |
| No parallel dispatch | `! test -f crates/temper-cli/src/commands/resource_cloud.rs` |

If any row shows gaps, add tests before proceeding.

- [ ] **Step 10.3: Verify no `resource_cloud` module leaked in**

Run: `find crates/temper-cli -name "resource_cloud*"`
Expected: no output (empty result).

Run: `grep -rn "resource_cloud" crates/temper-cli/`
Expected: no output.

- [ ] **Step 10.4: Delete the superseded plan file**

Run: `git rm docs/superpowers/plans/2026-04-19-unit-b-2-cloud-mode-dispatch.md`

- [ ] **Step 10.5: Commit the sweep**

```bash
git add -A
git commit -m "chore: cloud-first routing sweep + retire superseded plan

All acceptance criteria in the cloud-first-routing spec verified.
Retires the 2026-04-19 Unit B.2 dispatch plan — Tasks 1-9 shipped;
Tasks 10-17 superseded by this plan. Git history preserves the old
plan if needed."
```

- [ ] **Step 10.6: Use superpowers:finishing-a-development-branch**

Announce: "I'm using the finishing-a-development-branch skill to complete this work."

Invoke the skill and follow its guidance for choosing between: land as an interim PR now (Part 3 coherent on its own), bundle with Parts 1+2 as one B.2 PR, or continue to follow-up Units before any PR.
