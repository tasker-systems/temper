# Cloud-Only Vault — Chunk 1: Projection Module Foundation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the read-only local projection — a `temper pull <context>` command that materializes a whole context's resources as on-disk markdown files and records a per-context staleness cursor. Purely additive: removes nothing, dark-launched alongside the existing local-vault machinery.

**Architecture:** A new `projection` module in `temper-cli` lists a context's resources via the cloud API, writes each to its canonical vault path (`{owner}/{context}/{doc_type}/{slug}.md`) using the proven frontmatter-assembly recipe, prunes files for resources no longer present, and writes one cursor sidecar (`.temper/projection/<context>.json`) holding the server's latest event id for the context. A small dedicated API endpoint (`GET /api/events/cursor`) supplies that event id. The `temper pull` command is repurposed from single-resource-by-UUID to whole-context materialization.

**Tech Stack:** Rust, Axum (API), sqlx (compile-time-checked SQL), `temper-client` (HTTP client), `cargo-nextest` (tests), `tempfile` (test fixtures). The e2e harness spawns a real Axum server + Postgres via `#[sqlx::test]`.

**Source spec:** `docs/superpowers/specs/2026-05-21-cloud-only-vault-deprecation-design.md` (Chunk 1).

---

## Conventions for the implementer

These are repository rules (`tasker-systems/temper/CLAUDE.md`). Follow them in every task:

- **Typed structs over inline JSON.** Never `serde_json::json!()` for data with a known shape — define a struct.
- **Shared wire types live in `temper-core`.** A type sent between the API and a client is defined once in `temper-core` and imported by both sides.
- **Service layer owns SQL.** All `sqlx::query!()` lives in `temper-api/src/services/`. Never inline SQL in a handler.
- **Params structs** for functions with more than 5 domain parameters.
- **Auth before writes** — not applicable here (read-only feature) but keep auth gates intact.
- **All public types implement `Debug`.**
- **`#[expect(lint, reason = "...")]`** instead of `#[allow]` if a lint must be suppressed.
- After changing any SQL, regenerate the offline cache: `cargo sqlx prepare --workspace -- --all-features` (needs `DATABASE_URL` and the Docker Postgres running on port 5437).
- Commit messages end with the `Co-Authored-By` trailer used elsewhere in this repo.

**Environment:** Docker Postgres must be running (`cargo make docker-up`). `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development`.

**Branch:** all work lands on `jct/cloud-only-vault-deprecation-spec` (the spec is already committed there) — do not create a new branch.

---

## File Structure

**Created:**
- `crates/temper-cli/src/projection.rs` — the projection module: `ProjectionCursor`, `PullSummary`, cursor IO, `prune_context`, `write_resource_file`, `pull_context`. One cohesive file (~250 lines); unit tests inline under `#[cfg(test)]`.
- `tests/e2e/tests/projection_pull_test.rs` — e2e tests against a real server.

**Modified:**
- `crates/temper-core/src/types/api.rs` — add `EventCursorParams` and `EventCursorResponse`.
- `crates/temper-api/src/services/event_service.rs` — add `latest_event_id_for_context`.
- `crates/temper-api/src/handlers/events.rs` — add the `cursor` handler.
- `crates/temper-api/src/<router file>` — register `GET /api/events/cursor`.
- `crates/temper-client/src/events.rs` — add `EventClient::latest_for_context`.
- `crates/temper-cli/src/lib.rs` — add `pub mod projection;`.
- `crates/temper-cli/src/cli.rs` — change `Commands::Pull` to take `context` instead of `resource_id`.
- `crates/temper-cli/src/main.rs` — update the `Commands::Pull` dispatch arm.
- `crates/temper-cli/src/commands/pull.rs` — rewrite to call `projection::pull_context`.
- `.sqlx/` — regenerated query cache (one new query).

---

## Task 1: Shared wire types for the event cursor

**Files:**
- Modify: `crates/temper-core/src/types/api.rs`

These two types are the wire contract for the new `GET /api/events/cursor` endpoint. They live in `temper-core` so the API handler and `temper-client` share one definition.

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-core/src/types/api.rs` (inside an existing `#[cfg(test)] mod tests` block if one exists, otherwise add the block at the end of the file):

```rust
#[cfg(test)]
mod cursor_tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn event_cursor_params_round_trips() {
        let id = Uuid::nil();
        let params = EventCursorParams { kb_context_id: id };
        let json = serde_json::to_string(&params).unwrap();
        let back: EventCursorParams = serde_json::from_str(&json).unwrap();
        assert_eq!(back.kb_context_id, id);
    }

    #[test]
    fn event_cursor_response_round_trips_some_and_none() {
        let some = EventCursorResponse { latest_event_id: Some(Uuid::nil()) };
        let none = EventCursorResponse { latest_event_id: None };
        for value in [some, none] {
            let json = serde_json::to_string(&value).unwrap();
            let back: EventCursorResponse = serde_json::from_str(&json).unwrap();
            assert_eq!(back.latest_event_id, value.latest_event_id);
        }
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run -p temper-core cursor_tests`
Expected: FAIL — `cannot find type EventCursorParams` / `EventCursorResponse`.

- [ ] **Step 3: Add the types**

In `crates/temper-core/src/types/api.rs`, immediately after the existing `EventListParams` struct, add. Match the cfg-derive pattern used by `EventListParams` and `EventRow` already in that file:

```rust
/// Query parameters for the event-cursor endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct EventCursorParams {
    /// The context whose latest event id is requested.
    pub kb_context_id: Uuid,
}

/// Response body for the event-cursor endpoint: the most recent event id
/// recorded for a context, or `None` if the context has no events.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct EventCursorResponse {
    /// Most recent `kb_events.id` for the context, newest by `created`.
    pub latest_event_id: Option<Uuid>,
}
```

If `Uuid` is not already imported at the top of the file, it is — `EventListParams` already uses it. If a compile error says otherwise, add `use uuid::Uuid;`.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo nextest run -p temper-core cursor_tests`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/api.rs
git commit -m "$(cat <<'EOF'
feat(core): EventCursorParams/EventCursorResponse wire types

Shared contract for the GET /api/events/cursor endpoint that the
cloud-only projection uses to record per-context staleness cursors.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: Context event-cursor endpoint (service + handler + route + client)

**Files:**
- Modify: `crates/temper-api/src/services/event_service.rs`
- Modify: `crates/temper-api/src/handlers/events.rs`
- Modify: `crates/temper-api/src/<router file>` (find via grep — see Step 4)
- Modify: `crates/temper-client/src/events.rs`
- Modify: `.sqlx/` (regenerated)
- Test: `tests/e2e/tests/projection_pull_test.rs` (new file — created here, extended in later tasks)

This is one atomic vertical slice — service query, handler, route, and client method — because none of it is testable until all four exist. The endpoint answers "what is the most recent event id for this context?" with a single indexed SQL query.

- [ ] **Step 1: Write the failing e2e test**

Create `tests/e2e/tests/projection_pull_test.rs`:

```rust
#![cfg(feature = "test-db")]
//! E2e tests for the cloud-only read-only projection (`temper pull`).

mod common;

use temper_core::types::ingest::{pack_chunks, IngestPayload, PackedChunk};
use temper_core::types::ResourceId;
use uuid::Uuid;

/// Ingest one resource into `context` and return its id. The ingest path
/// emits a creation event into `kb_events`, so the context will have at
/// least one event afterward.
async fn seed_resource(
    app: &common::E2eTestApp,
    context: &str,
    doc_type: &str,
    title: &str,
) -> ResourceId {
    let body = format!("# {title}\n\nBody text for {title}.");
    let chunk = PackedChunk {
        chunk_index: 0,
        header_path: String::new(),
        heading_depth: 0,
        content: body.clone(),
        content_hash: temper_core::hash::compute_body_hash(&body),
        embedding: vec![0.0_f32; 768],
    };
    let slug = title.to_lowercase().replace(' ', "-");
    let payload = IngestPayload {
        title: title.to_string(),
        origin_uri: format!("test://{slug}"),
        context_name: context.to_string(),
        doc_type_name: doc_type.to_string(),
        content_hash: Some(temper_core::hash::compute_body_hash(&body)),
        slug,
        content: body.clone(),
        metadata: None,
        managed_meta: None,
        open_meta: None,
        chunks_packed: Some(pack_chunks(&[chunk]).expect("pack chunks")),
    };
    app.client.ingest().create(&payload).await.expect("ingest").id
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn events_cursor_returns_latest_event_for_context(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client.profile().get().await.expect("profile pre-flight");
    app.client.contexts().create("cursor-ctx").await.expect("ctx");

    seed_resource(&app, "cursor-ctx", "research", "Cursor Doc").await;

    // Resolve the context's UUID from a listed resource row.
    let listed = app
        .client
        .resources()
        .list(&temper_core::types::resource::ResourceListParams {
            context_name: Some("cursor-ctx".to_string()),
            ..Default::default()
        })
        .await
        .expect("list");
    let context_id = Uuid::from(listed.rows.first().expect("one row").kb_context_id);

    let latest = app
        .client
        .events()
        .latest_for_context(context_id)
        .await
        .expect("latest_for_context");
    assert!(latest.is_some(), "ingest must have emitted at least one event");

    // An unknown context has no events.
    let empty = app
        .client
        .events()
        .latest_for_context(Uuid::nil())
        .await
        .expect("latest_for_context empty");
    assert!(empty.is_none(), "unknown context has no events");
}
```

> Note: `ResourceListParams` derives `Default`, so `..Default::default()` is valid. Its path is `temper_core::types::resource::ResourceListParams` — confirm by checking the `use` lines in `crates/temper-client/src/resources.rs` if the compiler disagrees.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db events_cursor_returns_latest_event_for_context`
Expected: FAIL — `no method named latest_for_context`.

- [ ] **Step 3: Add the service query**

In `crates/temper-api/src/services/event_service.rs`, add this function after `list_visible`. It mirrors the visibility CTE that `list_visible` uses:

```rust
/// The most recent event id for a context, scoped to events the profile
/// may see. Returns `None` when the context has no visible events.
pub async fn latest_event_id_for_context(
    pool: &PgPool,
    profile_id: Uuid,
    kb_context_id: Uuid,
) -> ApiResult<Option<Uuid>> {
    let id = sqlx::query_scalar!(
        r#"
        WITH visible AS (SELECT resource_id FROM resources_visible_to($1))
        SELECT e.id
          FROM kb_events e
         WHERE (e.profile_id = $1 OR e.resource_id IN (SELECT resource_id FROM visible))
           AND e.kb_context_id = $2
         ORDER BY e.created DESC
         LIMIT 1
        "#,
        profile_id,
        kb_context_id,
    )
    .fetch_optional(pool)
    .await?;
    Ok(id)
}
```

- [ ] **Step 4: Add the handler and register the route**

In `crates/temper-api/src/handlers/events.rs`, add (the file already has `use` lines for `State`, `Query`, `Json`, `AuthUser`, `AppState`, `event_service`):

```rust
use temper_core::types::api::{EventCursorParams, EventCursorResponse};

#[utoipa::path(
    get,
    path = "/api/events/cursor",
    tag = "Events",
    params(EventCursorParams),
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Latest event id for the context", body = EventCursorResponse),
        (status = 401, description = "Unauthorized", body = ErrorBody),
    )
)]
pub async fn cursor(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<EventCursorParams>,
) -> ApiResult<Json<EventCursorResponse>> {
    let latest_event_id = event_service::latest_event_id_for_context(
        &state.pool,
        auth.0.profile.id,
        params.kb_context_id,
    )
    .await?;
    Ok(Json(EventCursorResponse { latest_event_id }))
}
```

Find where the existing `/api/events` route is registered:

Run: `grep -rn "events::list\|/api/events" crates/temper-api/src --include=*.rs`

In the router file that registers `events::list` (a `.route("/api/events", get(events::list))` call), add a sibling line:

```rust
.route("/api/events/cursor", get(events::cursor))
```

If that file has a `#[openapi(paths(...))]` utoipa registration that lists `events::list`, add `events::cursor` to the same `paths(...)` list.

- [ ] **Step 5: Add the client method**

In `crates/temper-client/src/events.rs`, change the import line and add the method to `impl<'a> EventClient<'a>`:

```rust
use temper_core::types::api::{EventCursorParams, EventCursorResponse, EventListParams, EventRow};
use uuid::Uuid;
```

```rust
    /// GET /api/events/cursor — the most recent event id for a context.
    pub async fn latest_for_context(&self, kb_context_id: Uuid) -> Result<Option<Uuid>> {
        let token = self.http.resolve_token()?;
        let params = EventCursorParams { kb_context_id };
        let req = self.http.get("/api/events/cursor").query(&params);
        let resp: EventCursorResponse = self
            .http
            .send_json(&Method::GET, "/api/events/cursor", req, Some(&token))
            .await?;
        Ok(resp.latest_event_id)
    }
```

- [ ] **Step 6: Regenerate the sqlx offline cache**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo sqlx prepare --workspace -- --all-features`
Expected: a new file appears under `.sqlx/` for the `latest_event_id_for_context` query.

- [ ] **Step 7: Run the test to verify it passes**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db events_cursor_returns_latest_event_for_context`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-core crates/temper-api crates/temper-client tests/e2e .sqlx
git commit -m "$(cat <<'EOF'
feat(api): GET /api/events/cursor — latest event id per context

Service query + handler + route + temper-client method. Supplies the
event id that the cloud-only projection records as its per-context
staleness cursor. One indexed query (ORDER BY created DESC LIMIT 1).

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: Projection module skeleton + `ProjectionCursor`

**Files:**
- Create: `crates/temper-cli/src/projection.rs`
- Modify: `crates/temper-cli/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/temper-cli/src/projection.rs`:

```rust
//! The read-only local projection of cloud vault state.
//!
//! `temper pull <context>` materializes every resource in a context as an
//! on-disk markdown file and records a per-context staleness cursor. The
//! projection is read-only by convention: editing a projected file changes
//! nothing on the server. See
//! `docs/superpowers/specs/2026-05-21-cloud-only-vault-deprecation-design.md`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// The per-context staleness cursor, written to
/// `.temper/projection/<context>.json` after every successful pull.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectionCursor {
    /// Server's latest event id for the context at pull time. `None` when
    /// the context had no events.
    pub last_event_id: Option<Uuid>,
    /// When the projection for this context was last refreshed.
    pub pulled_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_cursor_round_trips() {
        let cursor = ProjectionCursor {
            last_event_id: Some(Uuid::nil()),
            pulled_at: Utc::now(),
        };
        let json = serde_json::to_string(&cursor).unwrap();
        let back: ProjectionCursor = serde_json::from_str(&json).unwrap();
        assert_eq!(back.last_event_id, cursor.last_event_id);
        assert_eq!(back.pulled_at, cursor.pulled_at);
    }
}
```

In `crates/temper-cli/src/lib.rs`, add `pub mod projection;` in alphabetical position (between `pub mod output;` and `pub mod templates;` — actually between `output` and `templates`; place it as `pub mod projection;` right after `pub mod output;`).

- [ ] **Step 2: Run the test to verify it fails, then passes**

Run: `cargo nextest run -p temper-cli projection_cursor_round_trips`
Expected: PASS (the module compiles and the test passes on first run — this task is module scaffolding, so there is no red phase; the test guards the serde shape).

- [ ] **Step 3: Commit**

```bash
git add crates/temper-cli/src/projection.rs crates/temper-cli/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(cli): projection module skeleton + ProjectionCursor

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: Cursor sidecar IO

**Files:**
- Modify: `crates/temper-cli/src/projection.rs`

Atomic read/write of the cursor sidecar at `<state_dir>/projection/<context>.json`, copying the temp-file-then-rename pattern from `manifest_io.rs`.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `crates/temper-cli/src/projection.rs`:

```rust
    #[test]
    fn cursor_write_then_read_round_trips() {
        let dir = tempfile::TempDir::new().unwrap();
        let state_dir = dir.path().join(".temper");
        let cursor = ProjectionCursor {
            last_event_id: Some(Uuid::nil()),
            pulled_at: Utc::now(),
        };
        write_cursor(&state_dir, "myctx", &cursor).unwrap();
        let back = read_cursor(&state_dir, "myctx").unwrap();
        assert!(back.is_some());
        assert_eq!(back.unwrap().last_event_id, cursor.last_event_id);
    }

    #[test]
    fn read_cursor_returns_none_when_absent() {
        let dir = tempfile::TempDir::new().unwrap();
        let state_dir = dir.path().join(".temper");
        assert!(read_cursor(&state_dir, "never-pulled").unwrap().is_none());
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run -p temper-cli projection::tests::cursor`
Expected: FAIL — `cannot find function write_cursor` / `read_cursor`.

- [ ] **Step 3: Implement cursor IO**

Add to `crates/temper-cli/src/projection.rs` (after the `ProjectionCursor` struct, before the test module). Add `use std::path::{Path, PathBuf};` to the top-of-file imports:

```rust
use crate::error::{Result, TemperError};

/// Absolute path of a context's cursor sidecar.
fn cursor_path(state_dir: &Path, context: &str) -> PathBuf {
    state_dir.join("projection").join(format!("{context}.json"))
}

/// Read a context's cursor sidecar. Returns `None` when the file is absent
/// or unparseable (a corrupt sidecar is treated as "never pulled" rather
/// than a hard error — the next pull overwrites it).
pub fn read_cursor(state_dir: &Path, context: &str) -> Result<Option<ProjectionCursor>> {
    let path = cursor_path(state_dir, context);
    if !path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&path)?;
    Ok(serde_json::from_str::<ProjectionCursor>(&content).ok())
}

/// Atomically write a context's cursor sidecar (temp file + rename, the
/// pattern used by `manifest_io::save_manifest`).
pub fn write_cursor(state_dir: &Path, context: &str, cursor: &ProjectionCursor) -> Result<()> {
    let path = cursor_path(state_dir, context);
    let dir = path.parent().ok_or_else(|| {
        TemperError::Config(format!("cursor path has no parent: {}", path.display()))
    })?;
    std::fs::create_dir_all(dir)?;
    let tmp_path = dir.join(format!("{context}.json.tmp"));
    let content = serde_json::to_string_pretty(cursor)?;
    std::fs::write(&tmp_path, content)?;
    std::fs::rename(&tmp_path, &path)?;
    Ok(())
}
```

> `TemperError` and `Result` come from `crate::error`. Confirm the `TemperError::Config` variant exists (it is used throughout `manifest_io.rs` callers and `vault.rs`). `std::io::Error` converts into `TemperError` automatically — `manifest_io.rs` relies on the same `?` conversion.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo nextest run -p temper-cli projection::tests::cursor projection::tests::read_cursor`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/projection.rs
git commit -m "$(cat <<'EOF'
feat(cli): atomic cursor sidecar IO for the projection

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: `prune_context` — remove stale projection files

**Files:**
- Modify: `crates/temper-cli/src/projection.rs`

Prune walks every owner directory under the vault root, descends into `<owner>/<context>/<doc_type>/`, and removes any `.md` file not in the keep-set (the set of files the current pull wrote). It touches only the target context and only `.md` files.

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block:

```rust
    #[test]
    fn prune_removes_stale_md_keeps_listed_and_other_contexts() {
        use std::collections::HashSet;

        let dir = tempfile::TempDir::new().unwrap();
        let root = dir.path();

        let task_dir = root.join("@me/myctx/task");
        std::fs::create_dir_all(&task_dir).unwrap();
        let keep = task_dir.join("keep.md");
        let stale = task_dir.join("stale.md");
        let notes = task_dir.join("notes.txt");
        std::fs::write(&keep, "keep").unwrap();
        std::fs::write(&stale, "stale").unwrap();
        std::fs::write(&notes, "notes").unwrap();

        let other_ctx = root.join("@me/otherctx/task");
        std::fs::create_dir_all(&other_ctx).unwrap();
        let other = other_ctx.join("other.md");
        std::fs::write(&other, "other").unwrap();

        let mut keep_set = HashSet::new();
        keep_set.insert(keep.clone());

        let pruned = prune_context(root, "myctx", &keep_set).unwrap();

        assert_eq!(pruned, 1, "exactly one stale .md removed");
        assert!(keep.exists(), "listed file kept");
        assert!(!stale.exists(), "unlisted .md removed");
        assert!(notes.exists(), "non-.md file untouched");
        assert!(other.exists(), "other context untouched");
    }
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run -p temper-cli prune_removes_stale`
Expected: FAIL — `cannot find function prune_context`.

- [ ] **Step 3: Implement `prune_context`**

Add to `crates/temper-cli/src/projection.rs`. Add `use std::collections::HashSet;` to the top-of-file imports:

```rust
/// Remove projection `.md` files for resources no longer present in the
/// context. `keep` is the set of absolute file paths the current pull
/// wrote. Walks `<vault_root>/<owner>/<context>/<doc_type>/*.md` across
/// every owner directory. Only `.md` files are considered; other files
/// and other contexts are never touched. Returns the number of files removed.
pub fn prune_context(
    vault_root: &Path,
    context: &str,
    keep: &HashSet<PathBuf>,
) -> Result<usize> {
    let mut removed = 0usize;
    let owner_iter = match std::fs::read_dir(vault_root) {
        Ok(iter) => iter,
        Err(_) => return Ok(0), // vault root absent → nothing to prune
    };
    for owner_entry in owner_iter.flatten() {
        if !owner_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        // Skip hidden dirs such as `.temper`.
        if owner_entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let context_dir = owner_entry.path().join(context);
        if !context_dir.is_dir() {
            continue;
        }
        for doctype_entry in std::fs::read_dir(&context_dir)?.flatten() {
            if !doctype_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                continue;
            }
            for file_entry in std::fs::read_dir(doctype_entry.path())?.flatten() {
                let path = file_entry.path();
                if path.extension().and_then(|e| e.to_str()) != Some("md") {
                    continue;
                }
                if !keep.contains(&path) {
                    std::fs::remove_file(&path)?;
                    removed += 1;
                }
            }
        }
    }
    Ok(removed)
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo nextest run -p temper-cli prune_removes_stale`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/projection.rs
git commit -m "$(cat <<'EOF'
feat(cli): prune_context — drop stale projection files

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: `write_resource_file` — materialize one resource

**Files:**
- Modify: `crates/temper-cli/src/projection.rs`
- Test: `tests/e2e/tests/projection_pull_test.rs`

Fetches a resource's body + meta from the API and writes a complete, canonically-ordered markdown file at its vault path, reusing the proven assembly recipe from `actions::ingest` (the same one `sync::pull_one_resource` uses). Takes a `ResourceRow` already obtained from a `list` call, so it only needs one extra API call (`content`).

- [ ] **Step 1: Write the failing e2e test**

Add to `tests/e2e/tests/projection_pull_test.rs`:

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn write_resource_file_materializes_a_document(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client.profile().get().await.expect("profile pre-flight");
    app.client.contexts().create("wctx").await.expect("ctx");
    seed_resource(&app, "wctx", "research", "Write Me").await;

    let listed = app
        .client
        .resources()
        .list(&temper_core::types::resource::ResourceListParams {
            context_name: Some("wctx".to_string()),
            ..Default::default()
        })
        .await
        .expect("list");
    let row = listed.rows.first().expect("one row");

    let vault_root = app.vault_dir.path();
    let path = temper_cli::projection::write_resource_file(&app.client, vault_root, row)
        .await
        .expect("write_resource_file");

    let expected = vault_root
        .join("@me")
        .join("wctx")
        .join("research")
        .join("write-me.md");
    assert_eq!(path, expected);
    assert!(path.exists(), "file written at canonical path");

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.starts_with("---\n"), "has frontmatter fence");
    assert!(content.contains("temper-id:"), "has identity frontmatter");
    assert!(content.contains("Body text for Write Me"), "has body");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db write_resource_file_materializes`
Expected: FAIL — `no function write_resource_file in projection`.

- [ ] **Step 3: Implement `write_resource_file`**

Add to `crates/temper-cli/src/projection.rs`. Add these imports to the top of the file:

```rust
use temper_client::TemperClient;
use temper_core::types::ResourceRow;
use temper_core::vault::Vault;
```

```rust
/// Fetch a resource's content and write it as a complete markdown file at
/// its canonical vault path. Returns the absolute path written.
///
/// `row` is a resource summary already obtained from a `list` call; this
/// makes one further API call (`content`) for the body + frontmatter meta.
/// Frontmatter assembly reuses `actions::ingest::build_frontmatter_from_resource`
/// — the same recipe `sync::pull_one_resource` uses — so projected files are
/// byte-identical to sync-pulled ones.
pub async fn write_resource_file(
    client: &TemperClient,
    vault_root: &Path,
    row: &ResourceRow,
) -> Result<PathBuf> {
    use crate::actions::ingest;

    let content = client
        .resources()
        .content(Uuid::from(row.id))
        .await
        .map_err(crate::commands::client_err)?;

    // `owner_handle` is literal "@me" for the requester's own resources and
    // "+team-slug" for team contexts — both are canonical vault directory
    // components, so use it directly. Empty handle defends against a sparse
    // server row.
    let owner: &str = if row.owner_handle.is_empty() {
        "@me"
    } else {
        &row.owner_handle
    };
    let context = row.context_name.as_str();
    let doc_type = row.doc_type_name.as_str();

    let slug_owned;
    let slug: &str = match row.slug.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => {
            slug_owned = ingest::slug_from_title(&row.title);
            slug_owned.as_str()
        }
    };

    let managed_value = content
        .managed_meta
        .as_ref()
        .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null));

    let fm = ingest::build_frontmatter_from_resource(
        row,
        context,
        doc_type,
        owner,
        ingest::normalize_body_for_vault(&content.markdown),
        managed_value.as_ref(),
        content.open_meta.as_ref(),
    )?;

    let path = Vault::new(vault_root).doc_file(owner, context, doc_type, slug);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    fm.write_to(&path)
        .map_err(|e| TemperError::Config(format!("projection write {}: {e}", path.display())))?;
    Ok(path)
}
```

> `crate::commands::client_err` is the existing client-error adapter used by `pull.rs` and `sync.rs`. `ingest::build_frontmatter_from_resource`, `ingest::normalize_body_for_vault`, and `ingest::slug_from_title` are all `pub` in `crates/temper-cli/src/actions/ingest.rs`. If the import `temper_core::types::ResourceRow` fails, the type is also reachable as `temper_core::types::resource::ResourceRow` — match whichever path `actions/ingest.rs` uses for the same type.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db write_resource_file_materializes`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/projection.rs tests/e2e/tests/projection_pull_test.rs
git commit -m "$(cat <<'EOF'
feat(cli): write_resource_file — materialize one projected document

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: `pull_context` — full-context materialization

**Files:**
- Modify: `crates/temper-cli/src/projection.rs`
- Test: `tests/e2e/tests/projection_pull_test.rs`

Orchestrates the pull: list every resource in the context (paginated), write each file, prune stale files, fetch the context's latest event id, and write the cursor sidecar.

- [ ] **Step 1: Write the failing e2e test**

Add to `tests/e2e/tests/projection_pull_test.rs`:

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn pull_context_materializes_tree_and_writes_cursor(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client.profile().get().await.expect("profile pre-flight");
    app.client.contexts().create("pctx").await.expect("ctx");
    seed_resource(&app, "pctx", "research", "Doc One").await;
    seed_resource(&app, "pctx", "research", "Doc Two").await;

    let config = projection_test_config(&app);
    let summary = temper_cli::projection::pull_context(&app.client, &config, "pctx")
        .await
        .expect("pull_context");

    assert_eq!(summary.written, 2, "both resources written");
    assert_eq!(summary.pruned, 0, "nothing stale on a first pull");

    let vault_root = app.vault_dir.path();
    assert!(vault_root.join("@me/pctx/research/doc-one.md").exists());
    assert!(vault_root.join("@me/pctx/research/doc-two.md").exists());

    let cursor = temper_cli::projection::read_cursor(&config.state_dir, "pctx")
        .expect("read_cursor")
        .expect("cursor written");
    assert!(
        cursor.last_event_id.is_some(),
        "cursor records the context's latest event id"
    );
}

/// Build a CLI `Config` whose vault root is the e2e harness's temp vault.
fn projection_test_config(app: &common::E2eTestApp) -> temper_cli::config::Config {
    let vault_root = app.vault_dir.path().to_path_buf();
    temper_cli::config::Config {
        state_dir: vault_root.join(".temper"),
        vault_root,
        contexts: Vec::new(),
        subscriptions: Vec::new(),
        skill_output: app.vault_dir.path().join("temper.md"),
        profile_slug: None,
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db pull_context_materializes_tree`
Expected: FAIL — `no function pull_context in projection`.

- [ ] **Step 3: Implement `PullSummary` and `pull_context`**

Add to `crates/temper-cli/src/projection.rs`. Add imports `use crate::config::Config;` and `use temper_core::types::resource::ResourceListParams;`:

```rust
/// Outcome of a `pull_context` call, for the command's output line.
#[derive(Debug, Clone)]
pub struct PullSummary {
    pub context: String,
    pub written: usize,
    pub pruned: usize,
}

/// Page size for listing a context's resources. Contexts are small (tens to
/// low hundreds of resources); this paginates defensively regardless of the
/// server's own list cap.
const PULL_PAGE_SIZE: i64 = 200;

/// Materialize a whole context's resources into the local projection:
/// list every resource, write each file, prune files for resources no
/// longer present, then record the per-context staleness cursor.
///
/// Idempotent — re-running produces the same tree.
pub async fn pull_context(
    client: &TemperClient,
    config: &Config,
    context: &str,
) -> Result<PullSummary> {
    // 1. List every resource in the context (paginated).
    let mut rows: Vec<ResourceRow> = Vec::new();
    let mut offset: i64 = 0;
    loop {
        let params = ResourceListParams {
            context_name: Some(context.to_string()),
            limit: Some(PULL_PAGE_SIZE),
            offset: Some(offset),
            ..Default::default()
        };
        let resp = client
            .resources()
            .list(&params)
            .await
            .map_err(crate::commands::client_err)?;
        let page_len = resp.rows.len() as i64;
        rows.extend(resp.rows);
        if page_len < PULL_PAGE_SIZE {
            break;
        }
        offset += PULL_PAGE_SIZE;
    }

    // 2. Write each resource's file.
    let mut keep: HashSet<PathBuf> = HashSet::new();
    for row in &rows {
        let path = write_resource_file(client, &config.vault_root, row).await?;
        keep.insert(path);
    }

    // 3. Prune files for resources no longer in the context.
    let pruned = prune_context(&config.vault_root, context, &keep)?;

    // 4. Record the staleness cursor. The context's UUID comes from any
    //    listed row; an empty context yields no event id.
    let context_id = rows.first().map(|r| Uuid::from(r.kb_context_id));
    let last_event_id = match context_id {
        Some(cid) => client
            .events()
            .latest_for_context(cid)
            .await
            .map_err(crate::commands::client_err)?,
        None => None,
    };
    write_cursor(
        &config.state_dir,
        context,
        &ProjectionCursor {
            last_event_id,
            pulled_at: Utc::now(),
        },
    )?;

    Ok(PullSummary {
        context: context.to_string(),
        written: keep.len(),
        pruned,
    })
}
```

> `Uuid::from(r.kb_context_id)` — `kb_context_id` is a `ContextId` newtype over `Uuid` (the same pattern as `ResourceId`, which `sync.rs` converts with `Uuid::from`). If `From` is not implemented, use `r.kb_context_id.into()` or `r.kb_context_id.0`.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db pull_context_materializes_tree`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/projection.rs tests/e2e/tests/projection_pull_test.rs
git commit -m "$(cat <<'EOF'
feat(cli): pull_context — full-context projection materialization

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Prune-on-delete and idempotency e2e coverage

**Files:**
- Test: `tests/e2e/tests/projection_pull_test.rs`

Two acceptance criteria from the spec: a soft-deleted resource's file is pruned by the next pull, and re-pulling an unchanged context produces the identical tree.

- [ ] **Step 1: Write the failing tests**

Add to `tests/e2e/tests/projection_pull_test.rs`:

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn pull_prunes_resources_deleted_on_server(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client.profile().get().await.expect("profile pre-flight");
    app.client.contexts().create("dctx").await.expect("ctx");
    let keep_id = seed_resource(&app, "dctx", "research", "Keeper").await;
    let doomed_id = seed_resource(&app, "dctx", "research", "Doomed").await;

    let config = projection_test_config(&app);
    temper_cli::projection::pull_context(&app.client, &config, "dctx")
        .await
        .expect("first pull");

    let vault_root = app.vault_dir.path();
    assert!(vault_root.join("@me/dctx/research/keeper.md").exists());
    assert!(vault_root.join("@me/dctx/research/doomed.md").exists());

    // Soft-delete one resource on the server, then re-pull.
    app.client
        .resources()
        .delete(Uuid::from(doomed_id))
        .await
        .expect("delete");
    let summary = temper_cli::projection::pull_context(&app.client, &config, "dctx")
        .await
        .expect("second pull");

    assert_eq!(summary.written, 1, "only the survivor is written");
    assert_eq!(summary.pruned, 1, "the deleted resource's file is pruned");
    assert!(vault_root.join("@me/dctx/research/keeper.md").exists());
    assert!(!vault_root.join("@me/dctx/research/doomed.md").exists());
    let _ = keep_id;
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn pull_is_idempotent(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client.profile().get().await.expect("profile pre-flight");
    app.client.contexts().create("ictx").await.expect("ctx");
    seed_resource(&app, "ictx", "research", "Stable Doc").await;

    let config = projection_test_config(&app);
    let path = app.vault_dir.path().join("@me/ictx/research/stable-doc.md");

    temper_cli::projection::pull_context(&app.client, &config, "ictx")
        .await
        .expect("first pull");
    let first = std::fs::read_to_string(&path).unwrap();

    let summary = temper_cli::projection::pull_context(&app.client, &config, "ictx")
        .await
        .expect("second pull");
    let second = std::fs::read_to_string(&path).unwrap();

    assert_eq!(first, second, "re-pull produces byte-identical content");
    assert_eq!(summary.written, 1);
    assert_eq!(summary.pruned, 0);
}
```

- [ ] **Step 2: Run the tests to verify they pass**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db pull_prunes_resources_deleted pull_is_idempotent`
Expected: PASS (2 tests). These exercise already-implemented code, so they should pass immediately — they lock in the prune-on-delete and idempotency behavior.

> `ResourceClient::delete` takes a `Uuid` and returns `DeleteResponse`. Soft-deleted resources drop out of `list`, so the next pull's prune removes the file. If a test fails because the deleted resource still appears in `list`, the API list path is not filtering `is_active` — report that as a finding rather than working around it.

- [ ] **Step 3: Commit**

```bash
git add tests/e2e/tests/projection_pull_test.rs
git commit -m "$(cat <<'EOF'
test(cli): projection prune-on-delete and idempotency e2e coverage

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 9: Repurpose the `temper pull` command

**Files:**
- Modify: `crates/temper-cli/src/cli.rs`
- Modify: `crates/temper-cli/src/main.rs`
- Modify: `crates/temper-cli/src/commands/pull.rs`

`temper pull` changes from `pull <resource-uuid>` (single-resource snapshot via the sync engine) to `pull <context>` (full-context projection). This is the spec's "repurposed" command. The old `pull_one_resource` function in `sync.rs` is untouched and stays until Chunk 5.

- [ ] **Step 1: Change the clap command definition**

In `crates/temper-cli/src/cli.rs`, find the `Pull` variant in the `Commands` enum:

```rust
    /// Pull a resource from the cloud
    Pull {
        /// Resource UUID
        resource_id: String,
    },
```

Replace it with:

```rust
    /// Materialize a context's resources into the local read-only projection
    Pull {
        /// Context name to pull
        context: String,
    },
```

- [ ] **Step 2: Rewrite the command handler**

Replace the entire contents of `crates/temper-cli/src/commands/pull.rs` with:

```rust
//! `temper pull <context>` — materialize a context into the local
//! read-only projection. See `crate::projection`.

use crate::actions::runtime;
use crate::output;

pub fn run(context: &str) -> crate::error::Result<()> {
    let context = context.to_string();
    let summary = runtime::with_client(|client| {
        let context = context.clone();
        Box::pin(async move {
            let config = crate::config::load(None)?;
            crate::projection::pull_context(client, &config, &context).await
        })
    })?;

    output::success(format!(
        "Pulled context '{}': {} written, {} pruned",
        summary.context, summary.written, summary.pruned
    ));
    Ok(())
}
```

- [ ] **Step 3: Update the dispatch arm**

In `crates/temper-cli/src/main.rs`, find:

```rust
        Commands::Pull { resource_id } => commands::pull::run(&resource_id),
```

Replace with:

```rust
        Commands::Pull { context } => commands::pull::run(&context),
```

- [ ] **Step 4: Verify it compiles and nothing else references the old signature**

Run: `grep -rn "Commands::Pull\|pull::run" crates/temper-cli/src`
Expected: only the two sites above (`main.rs` dispatch, and none other). If a third site exists, update it to the `context` form.

Run: `cargo build -p temper-cli`
Expected: clean build.

> This task has no dedicated test: `commands/pull.rs` is now a thin wrapper (`runtime::with_client` + `config::load` + `pull_context`). Its logic — `pull_context` — is fully covered by the e2e tests in Tasks 7 and 8. Correctness here is compilation plus the existing suite staying green (Task 10).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/main.rs crates/temper-cli/src/commands/pull.rs
git commit -m "$(cat <<'EOF'
feat(cli): repurpose `temper pull` to context-wide projection

`temper pull <context>` now materializes a whole context into the local
read-only projection, replacing the single-resource-by-UUID snapshot.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

## Task 10: Full verification

**Files:** none (verification only)

- [ ] **Step 1: Quality gates**

Run: `cargo make check`
Expected: fmt, clippy (`-D warnings`), docs, machete, and TypeScript checks all pass.

> If clippy fails with `error communicating with database`, the Docker Postgres is down or `DATABASE_URL` is unset — start it (`cargo make docker-up`) or prefix with `SQLX_OFFLINE=true` to check against the committed `.sqlx/` cache.

- [ ] **Step 2: Workspace test sweep**

Run: `cargo nextest run --workspace`
Expected: PASS. Per the `feedback_workspace_test_surfaces_pipeline_bugs` lesson, the workspace run activates feature unification that narrower runs miss — it must be green, not just the per-crate runs.

- [ ] **Step 3: e2e suite**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db`
Expected: PASS — including the four new `projection_pull_test.rs` tests and all pre-existing tests (this chunk is additive; nothing should regress).

- [ ] **Step 4: Confirm additivity**

Run: `grep -rn "VaultState\|VaultBackend\|sync_orchestration" crates/temper-cli/src/projection.rs crates/temper-cli/src/commands/pull.rs`
Expected: no matches — the projection module and the repurposed command do not depend on the local-vault mode switch or the sync engine.

- [ ] **Step 5: Mark the task done in the vault**

Run: `temper resource update 2026-05-21-cloud-only-vault-chunk-1-projection-module-foundation --type task --stage done`
Expected: stage updated.

---

## Self-Review

**Spec coverage (Chunk 1 acceptance criteria):**
- "`temper pull <context>` materializes every active resource as a file at its canonical path" → Tasks 6, 7, 9. ✓
- "A soft-deleted resource's projection file is pruned by the next pull" → Tasks 5, 8. ✓
- "`.temper/projection/<context>.json` written with the server's latest event id" → Tasks 1, 2, 4, 7. ✓
- "`pull` is idempotent" → Task 8. ✓
- "Additive only — local mode and existing tests still pass" → Task 10 Steps 3–4. ✓
- "`cargo make check` and `cargo nextest run --workspace` green" → Task 10 Steps 1–2. ✓

**Spec refinement note:** the design spec placed the "latest event id for context" client method in Chunk 2. It is pulled into Chunk 1 here because the cursor sidecar (Chunk 1's headline new state) is only half-built without `last_event_id`, and a dedicated `GET /api/events/cursor` endpoint is cleaner than bloating `event_service::list_visible`'s four static-SQL variants into eight. Chunk 2 still owns the staleness *check* that consumes the cursor and the per-resource `show` refresh. This is a chunk-boundary refinement, not a scope change.

**Placeholder scan:** no TBD/TODO; every code step contains complete code. Two spots use grep-to-locate (Task 2 Step 4 router file, Task 9 Step 4 reference check) because the exact router file name was not captured during research — each gives the exact grep and the exact line to add. Three `>`-noted fallbacks (`ResourceRow` import path, `ContextId` conversion, `TemperError::Config` variant) name the concrete alternative rather than leaving a guess.

**Type consistency:** `ProjectionCursor` (fields `last_event_id: Option<Uuid>`, `pulled_at`) is defined in Task 3 and used identically in Tasks 4, 7. `EventCursorResponse.latest_event_id` (Task 1) is consumed by `EventClient::latest_for_context` returning `Option<Uuid>` (Task 2) and stored into `ProjectionCursor.last_event_id` (Task 7) — consistent. `PullSummary` (Task 7) fields `context`/`written`/`pruned` match the command's output line (Task 9). `write_resource_file`/`pull_context`/`prune_context`/`read_cursor`/`write_cursor` signatures are stable across their definition and call sites.
