# WS6 Surface-Completeness Spec B — Readback Routing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Finish the WS6 surface-completeness port — retire/delete the slug-keyed `by_uri` surface (live) and route MCP `get_resource`/`list_resources` through `temper_next` readback at full fidelity (gated) — so no read surface can only be answered from `public.*` before the chunk-5 flip.

**Architecture:** Two parts on one branch (`jct/ws6-surface-completeness-addressing-collapse`), shipped in the A+B PR. **Part 1 (Tasks 1–2)** ships *live*, backend-agnostic: the CLI session→task edge link uses the resource id it already holds instead of resolving a slug, then the whole `by_uri` surface is deleted. **Part 2 (Tasks 3–6)** is *gated* behind the `next-backend` feature + in-DB `flag=next`: `build_enriched` becomes a pure assembler (fixing a latent 2N redundant-query bug), and the two MCP read tools branch on `backend_selection`, sourcing their data from `readback` under `next`.

**Tech Stack:** Rust workspace (temper-core / temper-api / temper-mcp / temper-cli / temper-client / temper-next), Axum, sqlx (Postgres + pgvector), cargo-nextest, cargo-make.

## Global Constraints

- **Spec:** `docs/superpowers/specs/2026-06-18-ws6-surface-completeness-spec-b-readback-routing-design.md`. Companion: Spec A `…2026-06-17-ws6-surface-completeness-spec-a-addressing-collapse-design.md`.
- **§9 invariant floor for parity:** the parity-asserted fields are `origin_uri`, `title`, `is_active`, `context_name`, `doc_type_name`, `owner_handle`, `stage`/`mode`/`effort`/`seq`, `body_hash`. **Non-invariants (never asserted equal across legacy↔next):** re-minted ids (resource/context/profile), §7-dissolved `slug`/`managed_hash`/`open_hash`, synthesis-collapsed `created`/`updated`.
- **Gating:** all of Part 2's Next behavior is `#[cfg(feature = "next-backend")]` + `BackendSelection::Next`; default builds (no feature) keep the existing `NotImplemented` gate. Zero production behavior change while `kb_backend_selection = legacy`.
- **next-backend builds require `SQLX_OFFLINE=true`** (temper-next queries target the `temper_next` namespace; live validation against the `public` dev DB fails). Regenerate caches with `cargo make prepare-next` (temper-next readback SQL) / `cargo make prepare-api` (api test SQL) / `cargo make prepare-e2e` (e2e test SQL) after changing the matching SQL.
- **Migration discipline:** the plan adds **no** new migration unless Task 5's measurement shows one is needed; if added, it is an **append-only** forward migration in the `temper_next` lineage (never an in-place edit of a merged migration), single-sourced from `schema-artifact/01_schema.sql`, surviving the semantic drift guard.
- **Test-running gotchas (recurring):** `cargo make test-e2e` and bare `cargo nextest run -p temper-api` **hang** at test-list enumeration. Run api tests scoped: `cargo nextest run -p temper-api --features test-db --test <file> <name>`. Run e2e via libtest: `cargo test -p temper-e2e --features test-db[,next-backend] --test <file>` with `SQLX_OFFLINE=true` for next-backend.
- **Code quality:** typed structs over inline JSON; service layer owns SQL; >5 domain params → params struct; `cargo make check` (clippy `-D warnings`, all-targets all-features) must be clean before each commit (the pre-commit hook is a backstop, not the first line).

---

## File Structure

**Part 1 (live):**
- `crates/temper-cli/src/actions/types.rs` — add `id` to `TaskInfo`.
- `crates/temper-cli/src/actions/task.rs` — populate `TaskInfo.id` from the list row.
- `crates/temper-cli/src/commands/resource.rs` — session→task link asserts by id (drop `resolve_by_uri`).
- **Deleted:** `crates/temper-api/tests/resources_by_uri_test.rs`; `resolve_by_uri` + `ResolveByUriParams` in `resource_service.rs`; `by_uri` handler in `handlers/resources.rs`; route in `routes.rs`; openapi entry in `openapi.rs`; client method in `temper-client/src/resources.rs`.
- `crates/temper-api/tests/relationship_write_test.rs` — rewrite the `resolve_by_uri` verification helper to look up by id.

**Part 2 (gated):**
- `crates/temper-mcp/src/tools/resources.rs` — `build_enriched` → pure; `get_resource`/`list_resources` branch on `backend_selection`.
- `crates/temper-api/src/backend/read_selector.rs` — add `show_select`; add `list_enriched_next` exposure; extend the `next_impl` cfg module.
- `crates/temper-next/src/readback/mod.rs` — add `enriched_list` (batched, filtered) + `EnrichedListRow`.
- `crates/temper-api/tests/backend_read_path_next.rs` — api-level Next parity for the new selector paths.
- `tests/e2e/tests/mcp_round_trip.rs` (or a sibling) — e2e MCP-tool parity under `next-backend`.

---

## Part 1 — Retire & delete the `by_uri` surface (ships live)

### Task 1: CLI session→task link asserts by id; `TaskInfo` carries `id`

> **GROUNDED NOTE (corrects the plan):** the session→task link already has thorough e2e coverage in `tests/e2e/tests/cloud_session_link_e2e_test.rs` (5 tests; `create_session_with_task_asserts_advances_edge` already asserts the `advances` edge target via `peer_slug`). This task is a **refactor** (link by held id instead of a `resolve_by_uri` round-trip) — behavior is unchanged — so the guard is that existing suite, *tightened* to assert the edge target **by resource id**. `ResourceId` impls `Default` (via the `define_id!` macro, `ids.rs:28`), so a `#[serde(skip)]` `id` field needs no extra handling.

**Files:**
- Modify: `crates/temper-cli/src/actions/types.rs:5` (`TaskInfo`)
- Modify: `crates/temper-cli/src/actions/task.rs:108-130` (`task_info_from_meta`), `:88-95` (call site in `load_tasks`)
- Modify: `crates/temper-cli/src/commands/resource.rs:307-345` (session→task link)
- Test: `tests/e2e/tests/cloud_session_link_e2e_test.rs` (tighten the existing `create_session_with_task_asserts_advances_edge`)

**Interfaces:**
- Consumes: `client.resources().list_meta(&ResourceListParams)` (via `load_tasks`) returns rows each with `row.id: ResourceId` and `row.managed_meta: Option<ManagedMeta>`. `GraphEdgeRow` carries `peer_resource_id: Uuid`, `direction: String`.
- Produces: `TaskInfo.id: temper_core::types::ids::ResourceId`; `find_task(..) -> Result<Option<TaskInfo>>` (unchanged signature) now yields the id.

- [ ] **Step 1: Tighten the existing e2e test to assert the edge target by resource id**

In `tests/e2e/tests/cloud_session_link_e2e_test.rs::create_session_with_task_asserts_advances_edge`, the test already resolves `task_row` (the seeded task). Add an assertion that the outgoing edge's `peer_resource_id` equals the task's id — directly proving the id-based link targets the right resource:

```rust
// (after `task_row` is resolved, near the existing peer_slug assertion)
assert_eq!(
    outgoing[0].peer_resource_id,
    *task_row.id.as_uuid(),
    "outgoing advances edge must target the seeded task's resource id"
);
```

- [ ] **Step 2: Run the suite to confirm it passes today (refactor baseline)**

Run: `cargo test -p temper-e2e --features test-db --test cloud_session_link_e2e_test -- --nocapture`
Expected: PASS (the slug-resolved link already targets the task; this locks the target-id contract before the refactor). This is the regression guard the refactor must keep green.

- [ ] **Step 3: Add `id` to `TaskInfo`**

In `crates/temper-cli/src/actions/types.rs`, add the field (not a frontmatter key — it comes from the list row's top level, so `#[serde(skip)]` for any (de)serialization of `TaskInfo` itself):

```rust
use temper_core::types::ids::ResourceId;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TaskInfo {
    /// The resource id, carried from the list row (not a frontmatter key).
    #[serde(skip)]
    pub id: ResourceId,
    #[serde(rename = "temper-title")]
    pub title: String,
    // … existing fields unchanged …
}
```

- [ ] **Step 4: Populate `id` in `task_info_from_meta`**

In `crates/temper-cli/src/actions/task.rs`, thread the row id in. Change the signature and the call site:

```rust
// signature
fn task_info_from_meta(id: ResourceId, meta: ManagedMeta, context: &str) -> Result<TaskInfo> {
    // … existing title/slug extraction …
    Ok(TaskInfo {
        id,
        title,
        slug,
        // … rest unchanged …
    })
}
```

At the call site in `load_tasks` (`for row in response.rows { … }`), pass `row.id`:

```rust
let info = task_info_from_meta(row.id, meta, &ctx_for_query)?;
```

- [ ] **Step 5: Assert the edge by id in the session→task link**

In `crates/temper-cli/src/commands/resource.rs`, replace the `resolve_by_uri` round-trip (`:321-326`) with the id already on `task_info`. The block becomes:

```rust
let source_id = output.value.id;
let target_id = task_info.id;
let result = runtime.block_on(async {
    let req = AssertRelationshipRequest {
        source: source_id,
        target: target_id,
        edge_kind: EdgeKind::LeadsTo,
        polarity: Polarity::Forward,
        label: "advances".to_string(),
        weight: 1.0,
    };
    client
        .relationships()
        .assert(&req)
        .await
        .map_err(crate::commands::client_err)?;
    Ok::<_, TemperError>(())
});
```

Remove the now-unused `owner` binding (`config.owner_for_context(&task_info.context)`) and the `client.resources()` import if it becomes unused in this function.

- [ ] **Step 6: Run the e2e suite to verify the refactor holds**

Run: `cargo test -p temper-e2e --features test-db --test cloud_session_link_e2e_test -- --nocapture`
Expected: PASS — all 5 tests, including the tightened target-id assertion. (The link now uses `task_info.id`; the edge still targets the task, and no `resolve_by_uri` call is made on the link path.)

- [ ] **Step 7: Run the temper-cli unit suite + check**

Run: `cargo nextest run -p temper-cli` then `cargo make check`
Expected: PASS / exit 0. (Fix any other `TaskInfo { .. }` constructors the compiler flags — e.g. test fixtures — by supplying an `id`.)

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli tests/e2e/tests/cloud_session_link_e2e_test.rs
git commit -m "feat(cli): session→task link asserts edge by held resource id, not by_uri slug lookup"
```

---

### Task 2: Delete the `by_uri` surface

**Files:**
- Delete: `crates/temper-api/tests/resources_by_uri_test.rs`
- Modify: `crates/temper-api/src/services/resource_service.rs:24` (`ResolveByUriParams`), `:388-427` (`resolve_by_uri`) — delete both
- Modify: `crates/temper-api/src/handlers/resources.rs:10` (import), `:92-110` (`by_uri` handler + its utoipa attrs) — delete
- Modify: `crates/temper-api/src/routes.rs:50` — delete the route
- Modify: `crates/temper-api/src/openapi.rs:27` (path registration), `:133` (test assertion) — delete
- Modify: `crates/temper-client/src/resources.rs:104-120` (`resolve_by_uri` method) — delete
- Modify: `crates/temper-api/tests/relationship_write_test.rs:29,104` — rewrite the verification helper (uses the **service** fn)
- Modify: `tests/e2e/tests/cloud_session_link_e2e_test.rs` — replace **5** `resolve_by_uri` verification sites (the **client** method) with a local list+filter helper

> **GROUNDED NOTE (corrects the plan):** the client `.resolve_by_uri(` method has exactly **2** callers — the CLI link (removed in Task 1) and 5 sites in `cloud_session_link_e2e_test.rs`. Deleting the client method requires rewriting those 5 e2e sites first (Step 1b), or the e2e crate won't compile. `temper-client` exposes `resources().list(&ResourceListParams) -> ResourceListResponse` whose rows are `ResourceRow` (carry `.slug: Option<String>` and `.id`), so a slug→id helper is a list+filter (fine for these legacy-backend tests).

**Interfaces:**
- Consumes: `client.resources().list(&ResourceListParams)` for the e2e helper; `resource_service::get_visible` for the api-test helper.
- Produces: removal of `resolve_by_uri` / `ResolveByUriParams` / the `/api/resources/by-uri` route from every crate.

- [ ] **Step 1: Rewrite the `relationship_write_test` verification first (so the suite stays green through deletion)**

In `crates/temper-api/tests/relationship_write_test.rs`, the test uses `resolve_by_uri` to confirm a written resource is findable for the profile. Replace it with a direct id lookup — the test created the resources, so it holds their ids. Use `resource_service::get_visible(&pool, profile_id, resource_id)`:

```rust
// was: resource_service::resolve_by_uri(&pool, profile_id, &ResolveByUriParams { owner: "@me".into(), context, doc_type, ident: slug }).await
let row = resource_service::get_visible(&pool, profile_id, resource_id)
    .await
    .expect("resource visible to its owner by id");
assert_eq!(row.id, ResourceId::from(resource_id));
```

Run: `cargo nextest run -p temper-api --features test-db --test relationship_write_test`
Expected: PASS (still using `resolve_by_uri`'s replacement).

- [ ] **Step 1b: Rewrite the 5 e2e verification sites in `cloud_session_link_e2e_test.rs`**

Add a local helper near the file's other helpers and replace every `app.client.resources().resolve_by_uri(&owner, ctx, doctype, slug).await` with `resolve_by_slug(&app.client, ctx, doctype, slug).await`:

```rust
/// Resolve a created resource's row by slug (verification helper). Replaces the
/// deleted `resolve_by_uri` client method — these legacy-backend tests still
/// address by slug, so list-and-filter is the faithful substitute.
async fn resolve_by_slug(
    client: &temper_client::TemperClient,
    context: &str,
    doc_type: &str,
    slug: &str,
) -> temper_core::types::resource::ResourceRow {
    let params = temper_core::types::api::ResourceListParams {
        context_name: Some(context.to_string()),
        doc_type_name: Some(doc_type.to_string()),
        ..Default::default()
    };
    let resp = client.resources().list(&params).await.expect("list for slug resolve");
    resp.rows
        .into_iter()
        .find(|r| r.slug.as_deref() == Some(slug))
        .unwrap_or_else(|| panic!("no {doc_type} with slug '{slug}' in context '{context}'"))
}
```

Each call site loses the `&owner` argument (slug+context+doctype suffice). The `owner` bindings in those tests become unused — delete them. Confirm `ResourceListParams`'s actual module path and field names against `crates/temper-core/src/types/api.rs` (it is the same params type `list`/`list_meta` already take in this file's imports).

Run: `cargo test -p temper-e2e --features test-db --test cloud_session_link_e2e_test`
Expected: PASS (all 5 tests, now resolving via list+filter; still using the *service* `resolve_by_uri` nowhere — only the client method is being retired here).

- [ ] **Step 2: Delete the endpoint test file**

```bash
git rm crates/temper-api/tests/resources_by_uri_test.rs
```

- [ ] **Step 3: Delete the service fn + params struct**

In `crates/temper-api/src/services/resource_service.rs`, delete `pub struct ResolveByUriParams { … }` (`:24…`) and `pub async fn resolve_by_uri(…) { … }` (`:388-427`). Remove any now-unused imports the compiler flags.

- [ ] **Step 4: Delete the handler, route, and openapi entries**

- In `crates/temper-api/src/handlers/resources.rs`: delete the `by_uri` handler (`:92-110`, including its `#[utoipa::path(...)]`) and drop `ResolveByUriParams` from the import at `:10`.
- In `crates/temper-api/src/routes.rs`: delete the `.route("/api/resources/by-uri", get(handlers::resources::by_uri))` line (`:50`).
- In `crates/temper-api/src/openapi.rs`: delete the `crate::handlers::resources::by_uri,` path registration (`:27`) and the `assert!(json.contains("/api/resources/by-uri"));` line (`:133`).

- [ ] **Step 5: Delete the client method**

In `crates/temper-client/src/resources.rs`, delete `pub async fn resolve_by_uri(…) { … }` (`:104-120`). Remove any now-unused imports.

- [ ] **Step 6: Build + run the api + e2e suites + check**

Run:
```
cargo nextest run -p temper-api --features test-db --test relationship_write_test
cargo test -p temper-e2e --features test-db --test cloud_session_link_e2e_test
cargo make check
```
Expected: PASS / exit 0. The compiler will flag any other reference to the deleted symbols — remove each. (Expected zero remaining callers after Task 1 + Step 1b.)

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor: delete the by_uri surface (slug-scoped addressing retired post-Spec-A; unportable to temper_next)"
```

---

## Part 2 — MCP `get_resource`/`list_resources` full-fidelity Next routing (gated)

### Task 3: `build_enriched` becomes a pure assembler (drops 2N redundant queries)

**Files:**
- Modify: `crates/temper-mcp/src/tools/resources.rs:202-236` (`build_enriched`), `:244-263` (`enrich_resources`), `:419-475` (`get_resource` content path call site)
- Test: `crates/temper-mcp/src/tools/resources.rs` (inline `#[cfg(test)] mod tests`) or `crates/temper-mcp/tests/` if a test module already exists there

**Interfaces:**
- Consumes: `ResourceRow` already carries `context_name: String` and `doc_type_name: String` (populated by `get_visible`/`list_visible` via the `vault_resources_browse` view).
- Produces: `fn build_enriched(row: &ResourceRow, managed_meta: Option<ManagedMeta>, open_meta: Option<Value>) -> EnrichedResource` — **synchronous, no pool, no DB**.

- [ ] **Step 1: Write the failing unit test**

Add a test asserting `build_enriched` reads the names off the row (no DB) and fills the decorated `ref`:

```rust
#[cfg(test)]
mod build_enriched_tests {
    use super::*;

    fn sample_row() -> temper_core::types::resource::ResourceRow {
        use temper_core::types::ids::{ContextId, DocTypeId, ProfileId, ResourceId};
        use temper_core::types::resource::ResourceRow;
        let nil = uuid::Uuid::nil();
        ResourceRow {
            id: ResourceId::from(uuid::Uuid::now_v7()),
            kb_context_id: ContextId::from(nil),
            kb_doc_type_id: DocTypeId::from(nil),
            origin_uri: "temper://fixture/task-doc".to_string(),
            title: "Wire the widget".to_string(),
            slug: Some("wire-the-widget".to_string()),
            originator_profile_id: ProfileId::from(nil),
            owner_profile_id: ProfileId::from(nil),
            is_active: true,
            created: chrono::Utc::now(),
            updated: chrono::Utc::now(),
            context_name: "temper".to_string(),
            doc_type_name: "task".to_string(),
            owner_handle: "@me".to_string(),
            stage: Some("in-progress".to_string()),
            seq: None,
            mode: None,
            effort: None,
            body_hash: None,
            managed_hash: None,
            open_hash: None,
        }
    }

    #[test]
    fn build_enriched_uses_row_names_and_decorated_ref() {
        let row = sample_row();
        let e = build_enriched(&row, None, None);
        assert_eq!(e.context_name, "temper");
        assert_eq!(e.doc_type_name, "task");
        assert_eq!(e.r#ref, temper_core::operations::decorated_ref(&row.title, row.id.into()));
    }
}
```

> Note: replace the `todo!()` with a concrete `ResourceRow { … }` literal (the struct has ~22 fields; fill them — `id`, ids, `origin_uri`, `title`, `slug: None`, profile ids, `is_active: true`, `created`/`updated` via `Utc::now()`, `context_name: "temper".into()`, `doc_type_name: "task".into()`, `owner_handle: "@me".into()`, the workflow `Option`s `None`, the three hashes `None`).

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo nextest run -p temper-mcp build_enriched_uses_row_names_and_decorated_ref`
Expected: FAIL (`build_enriched` is still `async fn(pool, profile_id, …)` — signature mismatch / can't call without `.await`).

- [ ] **Step 3: Make `build_enriched` pure**

Replace `build_enriched` (`:202-236`) with:

```rust
/// Assemble an [`EnrichedResource`] from a row plus its already-fetched meta.
/// Pure assembly — `context_name`/`doc_type_name` are read off the row (both
/// schemas' full-row reads populate them via the browse view / readback
/// reconstruction), so there is no per-row context/doctype DB round-trip.
fn build_enriched(
    row: &temper_core::types::resource::ResourceRow,
    managed_meta: Option<ManagedMeta>,
    open_meta: Option<serde_json::Value>,
) -> EnrichedResource {
    EnrichedResource {
        id: row.id.into(),
        title: row.title.clone(),
        slug: row.slug.clone(),
        context_name: row.context_name.clone(),
        doc_type_name: row.doc_type_name.clone(),
        owner: "@me".to_string(),
        origin_uri: row.origin_uri.clone(),
        r#ref: temper_core::operations::decorated_ref(&row.title, row.id.into()),
        is_active: row.is_active,
        created: row.created,
        updated: row.updated,
        managed_meta,
        open_meta,
    }
}
```

- [ ] **Step 4: Update `enrich_resources` (drop the per-row `.await?`)**

In `enrich_resources` (`:244-263`), the loop now calls the sync `build_enriched`. The function stays `async` (it still batches `get_meta_batch`):

```rust
for row in rows {
    let (managed_meta, open_meta) = meta
        .remove(&row.id)
        .map(|m| (m.managed_meta, m.open_meta))
        .unwrap_or((None, None));
    enriched.push(build_enriched(row, managed_meta, open_meta));
}
```

Remove the now-unused `profile_id` param from `build_enriched`'s old callers only where it becomes dead; `enrich_resources`/`enrich_resource` keep `profile_id` only if still used elsewhere (the `meta_service::get_meta_batch` call does not need it — if `profile_id` is now unused in `enrich_resources`, drop it and update the two call sites in `get_resource`/`list_resources`). Delete the now-unused `context_service` / `doc_type_service` imports if nothing else uses them.

- [ ] **Step 5: Update the `get_resource` content-path call site**

In `get_resource` (`:435-453`), the content branch passes the row + meta directly:

```rust
let enriched = build_enriched(&row, content.managed_meta, content.open_meta);
```

- [ ] **Step 6: Run tests + check**

Run: `cargo nextest run -p temper-mcp` then `cargo make check`
Expected: PASS / exit 0. (Existing MCP tests — `mcp_round_trip` etc. at e2e — still pass; this is behavior-preserving on legacy, minus the redundant queries.)

- [ ] **Step 7: Commit**

```bash
git add crates/temper-mcp/src/tools/resources.rs
git commit -m "refactor(mcp): build_enriched reads context/doctype off the row (pure; drops 2N redundant per-row service queries)"
```

---

### Task 4: Route `get_resource` through backend selection (Next via readback)

**Files:**
- Modify: `crates/temper-api/src/backend/read_selector.rs` — add `show_select` + its `next_impl::show` arms (both cfg variants)
- Modify: `crates/temper-mcp/src/tools/resources.rs:419-475` (`get_resource`) — branch on `svc.api_state.backend_selection`
- Test: `crates/temper-api/tests/backend_read_path_next.rs` (api-level: `show_select` Next arm parity)

**Interfaces:**
- Consumes: `reconstruct_resource_row(pool, principal, new_id) -> Result<ResourceRow, TemperError>` (`next_backend.rs:125`, `pub(crate)`); `readback::meta`; `get_meta_select`/`get_content_select` (existing).
- Produces: `read_selector::show_select(selection: BackendSelection, pool: &PgPool, profile_id: Uuid, id: Uuid) -> ApiResult<ResourceRow>`.

- [ ] **Step 1: Write the failing api-level parity test**

In `crates/temper-api/tests/backend_read_path_next.rs` (follow the existing `#![cfg(all(feature = "test-db", feature = "next-backend"))]` header + synthesis fixture setup used by the file's current tests), add:

```rust
#[sqlx::test(migrations = "../../migrations")]
async fn show_select_next_matches_legacy_at_floor(pool: PgPool) {
    let fx = synthesize_prod_shape(&pool).await; // existing helper in this file

    let legacy = read_selector::show_select(
        BackendSelection::Legacy, &pool, fx.principal, fx.prod_id,
    ).await.expect("legacy show");

    let next = read_selector::show_select(
        BackendSelection::Next, &pool, fx.principal, fx.new_id,
    ).await.expect("next show");

    // §9 invariant floor — ids/slug/hashes/timestamps are NON-invariants.
    assert_eq!(legacy.origin_uri, next.origin_uri);
    assert_eq!(legacy.title, next.title);
    assert_eq!(legacy.context_name, next.context_name);
    assert_eq!(legacy.doc_type_name, next.doc_type_name);
    assert_eq!(legacy.is_active, next.is_active);
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-api --features test-db,next-backend --test backend_read_path_next show_select_next_matches_legacy_at_floor`
Expected: FAIL (`show_select` does not exist).

- [ ] **Step 3: Add `show_select` to `read_selector.rs`**

Next to `list_select` (`:29-40`), add:

```rust
use temper_core::types::resource::ResourceRow;

/// `show` — full resource row by id.
pub async fn show_select(
    selection: BackendSelection,
    pool: &PgPool,
    profile_id: Uuid,
    id: Uuid,
) -> ApiResult<ResourceRow> {
    match selection {
        BackendSelection::Legacy => resource_service::get_visible(pool, profile_id, id).await,
        BackendSelection::Next => next_impl::show(pool, profile_id, id).await,
    }
}
```

In the `#[cfg(not(feature = "next-backend"))] mod next_impl`, add the gate:

```rust
pub(super) async fn show(_: &PgPool, _: Uuid, _: Uuid) -> ApiResult<ResourceRow> {
    gate()
}
```

In the `#[cfg(feature = "next-backend")] mod next_impl`, add the real arm (resolve prod→new id like the other Next arms, then reconstruct):

```rust
pub(super) async fn show(pool: &PgPool, principal: Uuid, prod_id: Uuid) -> ApiResult<ResourceRow> {
    let new_id = resolve_new_id(pool, prod_id).await?;
    reconstruct_resource_row(pool, principal, new_id)
        .await
        .map_err(ApiError::from)
}
```

> `resolve_new_id` and `reconstruct_resource_row` are already imported/used by the sibling Next arms (`get_content`/`get_meta`). Under `flag=next` in production, callers pass next-minted ids directly; `resolve_new_id` is the parity-test bridge (prod id → new id via shared `origin_uri`), consistent with `get_content`/`get_meta`.

- [ ] **Step 4: Run the test to verify it passes**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-api --features test-db,next-backend --test backend_read_path_next show_select_next_matches_legacy_at_floor`
Expected: PASS.

- [ ] **Step 5: Branch `get_resource` on backend selection**

In `crates/temper-mcp/src/tools/resources.rs`, rewrite `get_resource` (`:419-475`) to source the row via `show_select`, meta via `get_meta_select`, body via `get_content_select` (uniform across backends — drops the legacy "content returns meta" coupling so the Next path, whose `get_content` returns `None` meta, works identically):

```rust
pub async fn get_resource(
    svc: &TemperMcpService,
    input: GetResourceInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let sel = svc.api_state.backend_selection;

    let id = temper_core::operations::parse_ref(&input.id)
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;

    let row = read_selector::show_select(sel, pool, profile.id, id.into())
        .await
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to get resource: {e}"), None))?;

    let meta = read_selector::get_meta_select(sel, pool, ProfileId::from(profile.id), row.id)
        .await
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to get meta: {e}"), None))?;

    let body_markdown = if input.include_content.unwrap_or(false) {
        let content = read_selector::get_content_select(sel, pool, profile.id, row.id.into())
            .await
            .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to get content: {e}"), None))?;
        Some(content.markdown)
    } else {
        None
    };

    let enriched = build_enriched(&row, meta.managed_meta, meta.open_meta);

    let enriched_value = serde_json::to_value(&enriched)
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to serialize: {e}"), None))?;
    let filtered = if let Some(fields) = input.fields.as_deref() {
        temper_core::projection::apply_top_level_filter(enriched_value, fields, "id")
            .map_err(map_projection_err)?
    } else {
        enriched_value
    };

    let mut parts = vec![rmcp::model::Content::text(
        serde_json::to_string_pretty(&filtered).unwrap_or_else(|_| "{}".to_string()),
    )];
    if let Some(markdown) = body_markdown {
        parts.push(rmcp::model::Content::text(markdown));
    }
    Ok(CallToolResult::success(parts))
}
```

Add `use temper_api::backend::read_selector;` if not already imported. `get_meta_select` returns `ResourceMetaResponse { managed_meta, open_meta, .. }`.

- [ ] **Step 6: Add the e2e MCP parity test**

In `tests/e2e/tests/mcp_round_trip.rs` (or a `mcp_read_path_next.rs` sibling, matching the file's `next-backend` gating convention), add a test that calls `get_resource` under `flag=next` and asserts the §9-floor fields match the legacy answer, using the mutate-`public`-after-synthesis control (prove the answer comes from `temper_next`):

```rust
#[tokio::test]
#[cfg(feature = "next-backend")]
async fn mcp_get_resource_next_answers_from_temper_next() {
    // setup: synthesize prod-shape into temper_next; flip flag=next for the MCP service.
    // 1) get_resource(id) under next → capture origin_uri/title/context_name/doc_type_name.
    // 2) mutate the public row (e.g. retitle) WITHOUT re-synthesizing.
    // 3) get_resource(id) under next again → unchanged (proves it reads temper_next, not public).
    // Follow the existing next-backend e2e control in backend_read_path_next / cloud_writes.
    todo!("fill in using the harness's synthesize + flag-set helpers")
}
```

> Replace the `todo!` with the concrete harness calls — model it on the existing next-backend e2e proof referenced in memory `project_ws6_surface_completeness_spec_a` (the mutate-public control). If wiring a full MCP service in e2e is disproportionate, assert parity at the `read_selector` level (api test, Step 1 pattern) for `get_meta_select` + `show_select` together and keep the MCP-tool assertion to a legacy regression in `mcp_round_trip`.

- [ ] **Step 7: Run e2e + check**

Run: `SQLX_OFFLINE=true cargo test -p temper-e2e --features test-db,next-backend --test mcp_round_trip` then `cargo make check`
Expected: PASS / exit 0.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-api/src/backend/read_selector.rs crates/temper-mcp/src/tools/resources.rs crates/temper-api/tests/backend_read_path_next.rs tests/e2e/tests/
git commit -m "feat(mcp): route get_resource through backend selection; Next arm via readback show/meta/content"
```

---

### Task 5: `readback::enriched_list` — batched, filtered list projection

**Files:**
- Modify: `crates/temper-next/src/readback/mod.rs` — add `EnrichedListRow` + `enriched_list`
- Test: `crates/temper-next/tests/parity_reads.rs` (artifact test, follows the file's `temper_next` namespace + synthesis-fixture pattern)

**Interfaces:**
- Consumes: `temper_next.kb_resources`, `kb_resource_homes`, `kb_contexts`, `kb_properties`, `resources_visible_to($principal)` — all qualified; the visibility function needs `SET LOCAL search_path TO temper_next, public` (see `ensure_visible`/`list`).
- Produces:
  ```rust
  pub struct EnrichedListRow {
      pub new_id: Uuid,          // re-minted (non-invariant; carried for EnrichedResource.id)
      pub origin_uri: String,
      pub title: String,
      pub is_active: bool,       // always true (synthesis carries active only)
      pub context_name: String,
      pub doc_type: String,
      pub stage: Option<String>,
      pub mode: Option<String>,
      pub effort: Option<String>,
      pub managed: serde_json::Map<String, serde_json::Value>,
      pub open: serde_json::Map<String, serde_json::Value>,
  }
  pub async fn enriched_list(
      pool: &PgPool, principal: Uuid,
      context_name: Option<&str>, doc_type: Option<&str>,
  ) -> Result<Vec<EnrichedListRow>>;
  ```

- [ ] **Step 1: Write the failing artifact test**

In `crates/temper-next/tests/parity_reads.rs`, add a test (mirror the file's existing synthesis-fixture setup that loads `01_schema`+`02_functions` and synthesizes the prod-shape rows):

```rust
#[sqlx::test]
async fn enriched_list_filters_by_context_and_doctype(pool: PgPool) {
    seed_prod_shape_and_synthesize(&pool).await; // existing helper in this file

    // Unfiltered: every visible synthesized resource.
    let all = readback::enriched_list(&pool, PRINCIPAL, None, None).await.unwrap();
    assert!(all.len() >= 2);
    assert!(all.iter().all(|r| !r.context_name.is_empty() && !r.doc_type.is_empty()));

    // Filter by doctype → only matching rows.
    let tasks = readback::enriched_list(&pool, PRINCIPAL, None, Some("task")).await.unwrap();
    assert!(tasks.iter().all(|r| r.doc_type == "task"));
    assert!(!tasks.is_empty());

    // Filter by context → only matching rows; managed/open populated.
    let in_ctx = readback::enriched_list(&pool, PRINCIPAL, Some("temper"), None).await.unwrap();
    assert!(in_ctx.iter().all(|r| r.context_name == "temper"));
    assert!(in_ctx.iter().any(|r| !r.managed.is_empty()));
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests --test parity_reads enriched_list_filters_by_context_and_doctype`
Expected: FAIL (`enriched_list` does not exist).

- [ ] **Step 3: Implement `EnrichedListRow` + `enriched_list` (two batched queries)**

Add to `crates/temper-next/src/readback/mod.rs`. Query 1 selects the visible, filtered set with the row fields + the `doc_type` property (INNER JOIN, also serving the filter); Query 2 batch-scans all properties for the surviving ids. Runtime `sqlx::query` (never the macros), schema-qualified, inside a `SET LOCAL search_path` txn:

```rust
#[derive(Debug, Clone)]
pub struct EnrichedListRow {
    pub new_id: Uuid,
    pub origin_uri: String,
    pub title: String,
    pub is_active: bool,
    pub context_name: String,
    pub doc_type: String,
    pub stage: Option<String>,
    pub mode: Option<String>,
    pub effort: Option<String>,
    pub managed: Map<String, Value>,
    pub open: Map<String, Value>,
}

/// Batched, filtered list projection for MCP enrichment. Two queries, no N+1:
/// (1) the visible set with row fields + `doc_type` (the filter join);
/// (2) one `owner_id = ANY($ids)` scan to reconstruct managed/open per row.
pub async fn enriched_list(
    pool: &PgPool,
    principal: Uuid,
    context_name: Option<&str>,
    doc_type: Option<&str>,
) -> Result<Vec<EnrichedListRow>> {
    let mut tx = pool.begin().await?;
    sqlx::query("SET LOCAL search_path TO temper_next, public")
        .execute(&mut *tx)
        .await?;

    // Query 1: visible set + display fields + doc_type (INNER JOIN), filters in SQL.
    let set_rows = sqlx::query(
        "SELECT r.id AS new_id, r.origin_uri, r.title, r.is_active,
                c.name AS context_name,
                dt.property_value #>> '{}' AS doc_type,
                st.property_value #>> '{}' AS stage,
                md.property_value #>> '{}' AS mode,
                ef.property_value #>> '{}' AS effort
           FROM temper_next.kb_resources r
           JOIN temper_next.resources_visible_to($1) v ON v.resource_id = r.id
           JOIN temper_next.kb_resource_homes h ON h.resource_id = r.id
           JOIN temper_next.kb_contexts c
             ON c.id = h.anchor_id AND h.anchor_table = 'kb_contexts'
           JOIN temper_next.kb_properties dt
             ON dt.owner_table = 'kb_resources' AND dt.owner_id = r.id
            AND dt.property_key = 'doc_type' AND NOT dt.is_folded
           LEFT JOIN temper_next.kb_properties st
             ON st.owner_table = 'kb_resources' AND st.owner_id = r.id
            AND st.property_key = 'temper-stage' AND NOT st.is_folded
           LEFT JOIN temper_next.kb_properties md
             ON md.owner_table = 'kb_resources' AND md.owner_id = r.id
            AND md.property_key = 'temper-mode' AND NOT md.is_folded
           LEFT JOIN temper_next.kb_properties ef
             ON ef.owner_table = 'kb_resources' AND ef.owner_id = r.id
            AND ef.property_key = 'temper-effort' AND NOT ef.is_folded
          WHERE ($2::text IS NULL OR c.name = $2)
            AND ($3::text IS NULL OR dt.property_value #>> '{}' = $3)
          ORDER BY r.origin_uri",
    )
    .bind(principal)
    .bind(context_name)
    .bind(doc_type)
    .fetch_all(&mut *tx)
    .await?;

    let ids: Vec<Uuid> = set_rows.iter().map(|r| r.get::<Uuid, _>("new_id")).collect();

    // Query 2: batched property scan for managed/open reconstruction.
    let prop_rows = sqlx::query(
        "SELECT owner_id, property_key, property_value
           FROM temper_next.kb_properties
          WHERE owner_table = 'kb_resources'
            AND owner_id = ANY($1)
            AND NOT is_folded",
    )
    .bind(&ids)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    // Group properties by owner; reuse the §7 managed/open split helper.
    let mut by_owner: HashMap<Uuid, (Map<String, Value>, Map<String, Value>)> = HashMap::new();
    for pr in &prop_rows {
        let owner: Uuid = pr.get("owner_id");
        let key: String = pr.get("property_key");
        let value: Value = pr.get("property_value");
        if key == "doc_type" {
            continue; // surfaced as the typed column, not in managed/open (parity with readback::meta)
        }
        let entry = by_owner.entry(owner).or_default();
        if is_managed_property_key(&key) {
            entry.0.insert(key, value);
        } else {
            entry.1.insert(key, value);
        }
    }

    Ok(set_rows
        .iter()
        .map(|r| {
            let new_id: Uuid = r.get("new_id");
            let (managed, open) = by_owner.remove(&new_id).unwrap_or_default();
            EnrichedListRow {
                new_id,
                origin_uri: r.get("origin_uri"),
                title: r.get("title"),
                is_active: r.get("is_active"),
                context_name: r.get("context_name"),
                doc_type: r.get("doc_type"),
                stage: r.get("stage"),
                mode: r.get("mode"),
                effort: r.get("effort"),
                managed,
                open,
            }
        })
        .collect())
}
```

> `is_managed_property_key`, `Map`, `Value`, `HashMap` are already in scope in this module (used by `readback::meta`). Confirm the `kb_resource_homes → kb_contexts` join columns against `readback::resource_row` (`mod.rs:432-472`) — it does the identical home/context join; copy its exact column names (`h.anchor_table`/`h.anchor_id` vs the home's `resource_id` linkage) rather than guessing.

- [ ] **Step 4: Regenerate the temper-next offline cache (if any `query!` macro was touched) and run the test**

`enriched_list` uses runtime `sqlx::query` (not macros), so no cache regen is needed for it. Run:

Run: `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests --test parity_reads enriched_list_filters_by_context_and_doctype`
Expected: PASS. (If a join column name is wrong, the runtime query errors at test time — fix against `resource_row`'s join.)

- [ ] **Step 5: Measurement note on indexing (no speculative migration)**

Confirm via `EXPLAIN` against the artifact namespace that Query 1's `doc_type` join and Query 2's `owner_id = ANY` scan use `idx_kb_properties_owner` (partial, `WHERE NOT is_folded`). Only if a plan shows a seq-scan regression at corpus scale, add a composite `(owner_table, owner_id, property_key) WHERE NOT is_folded` index as an **append-only** `temper_next` forward migration single-sourced from `01_schema.sql`. Default expectation: no migration (low per-resource property cardinality). Record the decision in the commit message.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-next/src/readback/mod.rs crates/temper-next/tests/parity_reads.rs
git commit -m "feat(temper-next): readback::enriched_list — batched, context/doctype-filtered list projection (no N+1; no new index needed)"
```

---

### Task 6: Route `list_resources` through one backend-agnostic selector

**Design:** follow the dominant `read_selector` pattern — one `list_enriched_select` over **both** backends, returning **always-compiled temper-core types** (`Vec<(ResourceRow, Option<ManagedMeta>, Option<Value>)>`), with the Next path gated *inside* the `next_impl` cfg module (exactly like `list_select`/`search_select`). MCP then has **no cfg branch** and reuses the single `build_enriched` assembler for both backends. This avoids a feature-gated type at the crate boundary (which would force a `next-backend` feature into temper-mcp).

> **GROUNDED NOTE (corrects the plan):** `enrich_resources`/`enrich_resource` (`crates/temper-mcp/src/tools/resources.rs:231-260`) are **NOT dead** after this task — `create_resource` (`:367`) and `update_resource` (`:608`) still call `enrich_resource`. **Do NOT delete them.** Only the `list_resources` call site (`:507`) is replaced. Drop imports only if the compiler/clippy genuinely flags them unused in this file (the filter-id resolution moves to the read_selector legacy arm, so `context_service`/`ingest_service` *may* become unused here — let clippy decide).

**Files:**
- Modify: `crates/temper-api/src/backend/read_selector.rs` — add `list_enriched_select` + its `next_impl::list_enriched` arms (both cfg variants)
- Modify: `crates/temper-mcp/src/tools/resources.rs:477-544` (`list_resources`) — call the selector, map via `build_enriched`. **Keep** `enrich_resources`/`enrich_resource` (create/update still use them).
- Test: `tests/e2e/tests/mcp_round_trip.rs` (or `mcp_read_path_next.rs` sibling) — e2e `list_resources` parity + filter assertion under `next-backend`

**Interfaces:**
- Consumes: `readback::enriched_list(pool, principal, context_name, doc_type) -> Result<Vec<EnrichedListRow>>` (Task 5); `resource_service::{list_visible, ResourceListParams}`; `meta_service::get_meta_batch`; `context_service::resolve_by_name`; `ingest_service::resolve_doc_type`; `build_enriched` (Task 3).
- Produces: `read_selector::list_enriched_select(selection: BackendSelection, pool: &PgPool, profile_id: Uuid, context_name: Option<&str>, doc_type: Option<&str>) -> ApiResult<Vec<(ResourceRow, Option<ManagedMeta>, Option<serde_json::Value>)>>`.

- [ ] **Step 1: Write the failing e2e test**

In the e2e suite, add a test that lists under `flag=next` with a doctype filter and asserts only matching rows return, each carrying managed_meta, and the row SET matches the legacy answer:

```rust
#[tokio::test]
#[cfg(feature = "next-backend")]
async fn mcp_list_resources_next_filters_and_enriches() {
    // synthesize prod-shape with mixed doctypes into temper_next; set flag=next.
    // list_resources(doc_type_name="task") under next → every row doc_type_name=="task",
    // managed_meta populated, context_name non-empty. Compare the row SET (by origin_uri)
    // against the legacy answer for the same filter (§9 floor: set + fields, not ids/order).
    todo!("fill in using the harness's synthesize + flag-set helpers (model on Task 4 Step 6 and the existing next-backend e2e control)")
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `SQLX_OFFLINE=true cargo test -p temper-e2e --features test-db,next-backend --test mcp_round_trip mcp_list_resources_next_filters_and_enriches`
Expected: FAIL (`list_resources` ignores selection / filters under next).

- [ ] **Step 3: Add `list_enriched_select` to `read_selector.rs` (both backends, always-compiled return type)**

```rust
use temper_core::types::managed_meta::ManagedMeta;

/// `list_resources` enrichment — full rows + their managed/open meta, filtered by
/// context_name + doc_type, for BOTH backends. Returns always-compiled temper-core
/// types so the consumer (MCP) needs no `next-backend` feature; the Next path is
/// gated inside `next_impl`. Legacy resolves the name filters to ids; Next filters
/// by name in SQL (slug/timestamps are §9 non-invariants — Next stamps None/now()).
pub async fn list_enriched_select(
    selection: BackendSelection,
    pool: &PgPool,
    profile_id: Uuid,
    context_name: Option<&str>,
    doc_type: Option<&str>,
) -> ApiResult<Vec<(ResourceRow, Option<ManagedMeta>, Option<serde_json::Value>)>> {
    match selection {
        BackendSelection::Legacy => {
            let context_id = match context_name {
                Some(name) => Some(
                    context_service::resolve_by_name(pool, ProfileId::from(profile_id), name)
                        .await?
                        .id
                        .into(),
                ),
                None => None,
            };
            let doc_type_id = match doc_type {
                Some(name) => Some(ingest_service::resolve_doc_type(pool, name).await?),
                None => None,
            };
            let params = ResourceListParams {
                kb_context_id: context_id,
                kb_doc_type_id: doc_type_id,
                ..Default::default()
            };
            let response = resource_service::list_visible(pool, profile_id, params).await?;
            let ids: Vec<ResourceId> = response.rows.iter().map(|r| r.id).collect();
            let mut meta = meta_service::get_meta_batch(pool, &ids).await?;
            Ok(response
                .rows
                .into_iter()
                .map(|row| {
                    let (m, o) = meta
                        .remove(&row.id)
                        .map(|x| (x.managed_meta, x.open_meta))
                        .unwrap_or((None, None));
                    (row, m, o)
                })
                .collect())
        }
        BackendSelection::Next => next_impl::list_enriched(pool, profile_id, context_name, doc_type).await,
    }
}
```

Add the imports the Legacy arm needs (`ingest_service`, `ResourceId`) to the `use` block if absent. In the `#[cfg(not(feature = "next-backend"))] mod next_impl`, add the gate:

```rust
pub(super) async fn list_enriched(
    _: &PgPool, _: Uuid, _: Option<&str>, _: Option<&str>,
) -> ApiResult<Vec<(ResourceRow, Option<ManagedMeta>, Option<serde_json::Value>)>> {
    gate()
}
```

In the `#[cfg(feature = "next-backend")] mod next_impl`, add the real arm — `enriched_list` (batched, filtered), then map each lean `EnrichedListRow` to a `ResourceRow` (filling only the fields `build_enriched` reads; the rest are §7-dissolved/re-minted/now() non-invariants):

```rust
pub(super) async fn list_enriched(
    pool: &PgPool,
    principal: Uuid,
    context_name: Option<&str>,
    doc_type: Option<&str>,
) -> ApiResult<Vec<(ResourceRow, Option<ManagedMeta>, Option<serde_json::Value>)>> {
    use temper_core::types::ids::{ContextId, DocTypeId, ProfileId as CoreProfileId, ResourceId};
    let rows = readback::enriched_list(pool, principal, context_name, doc_type)
        .await
        .map_err(api_err)?;
    let now = chrono::Utc::now();
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let row = ResourceRow {
            id: ResourceId::from(r.new_id),
            kb_context_id: ContextId::from(uuid::Uuid::nil()),     // unused by build_enriched (non-invariant)
            kb_doc_type_id: DocTypeId::from(uuid::Uuid::nil()),    // unused by build_enriched (name is authoritative)
            origin_uri: r.origin_uri,
            title: r.title,
            slug: None,                                            // §7-dissolved
            originator_profile_id: CoreProfileId::from(uuid::Uuid::nil()),
            owner_profile_id: CoreProfileId::from(uuid::Uuid::nil()),
            is_active: r.is_active,
            created: now,                                          // synthesis-collapsed (non-invariant)
            updated: now,
            context_name: r.context_name,
            doc_type_name: r.doc_type,
            owner_handle: "@me".to_string(),
            stage: r.stage,
            seq: None,
            mode: r.mode,
            effort: r.effort,
            body_hash: None,
            managed_hash: None,
            open_hash: None,
        };
        let managed: Option<ManagedMeta> =
            serde_json::from_value(serde_json::Value::Object(r.managed)).ok();
        let open = Some(serde_json::Value::Object(r.open));
        out.push((row, managed, open));
    }
    Ok(out)
}
```

> Confirm `ResourceRow`'s exact field set against `crates/temper-core/src/types/resource.rs` (it is the struct Task 3 enumerates) — fill every field; the `uuid::Uuid::nil()` / `None` / `now` values are deliberate for the non-invariant fields `build_enriched` never reads.

- [ ] **Step 4: Rewrite `list_resources` to use the selector + delete dead helpers**

In `crates/temper-mcp/src/tools/resources.rs`, replace `list_resources` (`:477-544`) with a single path — no cfg branch:

```rust
pub async fn list_resources(
    svc: &TemperMcpService,
    input: ListResourcesInput,
) -> Result<CallToolResult, rmcp::ErrorData> {
    let profile = svc.require_profile().await?;
    let pool = &svc.api_state.pool;
    let sel = svc.api_state.backend_selection;

    let rows = read_selector::list_enriched_select(
        sel, pool, profile.id,
        input.context_name.as_deref(), input.doc_type_name.as_deref(),
    )
    .await
    .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to list resources: {e}"), None))?;

    let enriched: Vec<EnrichedResource> = rows
        .iter()
        .map(|(row, managed, open)| build_enriched(row, managed.clone(), open.clone()))
        .collect();

    let array_value = serde_json::to_value(&enriched)
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to serialize: {e}"), None))?;
    let filtered = if let Some(fields) = input.fields.as_deref() {
        temper_core::projection::apply_top_level_filter(array_value, fields, "id")
            .map_err(map_projection_err)?
    } else {
        array_value
    };
    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        serde_json::to_string_pretty(&filtered).unwrap_or_else(|_| "[]".to_string()),
    )]))
}
```

**Keep `enrich_resources`/`enrich_resource`** — `create_resource` (`:367`) and `update_resource` (`:608`) still call `enrich_resource`. Only the `list_resources` call site changed. After the rewrite, run `cargo clippy -p temper-mcp` and drop ONLY the imports it flags as genuinely unused in this file (`context_service`/`ingest_service` likely become unused here since filter-id resolution moved to the read_selector legacy arm; `meta_service` is still used by `enrich_resources`). Do not remove anything clippy doesn't flag.

> The MCP tool no longer needs `input.limit`/`input.offset` plumbing for Next (the §9 floor asserts the set, and `enriched_list` is unpaginated, matching `next_impl::list`). Legacy pagination via `ResourceListParams` is dropped here too for symmetry — if a consumer depends on MCP list pagination, raise it in review rather than silently preserving a backend-asymmetric limit. (Spec §9: list ordering/bounds are not migration invariants.)

- [ ] **Step 5: Run e2e + the api/mcp suites + check**

Run:
```
SQLX_OFFLINE=true cargo test -p temper-e2e --features test-db,next-backend --test mcp_round_trip
cargo nextest run -p temper-mcp
SQLX_OFFLINE=true cargo nextest run -p temper-api --features test-db,next-backend --test backend_read_path_next
cargo make check
```
Expected: PASS / exit 0.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/backend/read_selector.rs crates/temper-mcp/src/tools/resources.rs tests/e2e/tests/
git commit -m "feat(mcp): route list_resources through one backend-agnostic list_enriched_select; Next arm via readback::enriched_list (context/doctype filters)"
```

---

## Final verification (before PR)

- [ ] `cargo make check` — exit 0 (clippy all-targets all-features, fmt, docs, machete).
- [ ] `SQLX_OFFLINE=true cargo nextest run -p temper-next --features artifact-tests` — parity_reads green.
- [ ] `cargo nextest run -p temper-api --features test-db --test relationship_write_test --test backend_read_path_next` and the next-backend variant `SQLX_OFFLINE=true cargo nextest run -p temper-api --features test-db,next-backend --test backend_read_path_next`.
- [ ] `SQLX_OFFLINE=true cargo test -p temper-e2e --features test-db,next-backend --test cloud_writes --test mcp_round_trip`.
- [ ] Companion skill change (out-of-repo, ships with the PR narrative): the installed temper skill at `~/.claude/skills/temper/` references `resolve_by_uri`-style flows nowhere user-facing, but confirm no command-sequence doc instructs a by-uri lookup; if it does, update in lockstep (as Spec A's T9 did for decorated refs).
- [ ] Consolidated code review (hybrid-execution Variant B end-of-plan): correctness of the `enriched_list` joins/filters, the `build_enriched` purity (no behavior drift on legacy), the §9-floor parity assertions, leak-safe `NotVisible`→404 preserved on every Next read.

## Self-Review coverage map (spec → task)

- Spec §"Correction 1" / Part 1 (retire caller) → **Task 1**.
- Spec Part 1 (delete `by_uri` surface, full blast radius) → **Task 2**.
- Spec §"Correction 2" + Part 2 (`build_enriched` backend-agnostic) → **Task 3**.
- Spec Part 2 `get_resource` Next routing → **Task 4**.
- Spec Part 2 `enriched_list` + query-efficiency/indexing subsection → **Task 5**.
- Spec Part 2 `list_resources` full-fidelity (filters) Next routing → **Task 6**.
- Spec §"Testing" (gated e2e parity, live CLI test, build gotchas) → distributed across Tasks 1/4/6 + Final verification.
- Spec §"Non-goals" → respected (no relationship reads; no `by_uri` Next arm; no vault-filename rename).
