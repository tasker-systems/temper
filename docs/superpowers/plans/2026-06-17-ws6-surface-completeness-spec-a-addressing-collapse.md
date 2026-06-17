# WS6 Surface-Completeness Spec A — Addressing-Model Collapse Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Retire slug-scoped resource addressing (`ResourceRef::Scoped` + edge `target_slug`) in favor of the Adjudication-5 decorated-ref identity contract — UUID or `sluggify(title)-<uuid>`, trailing-UUID-only resolution, one resolver — across CLI, MCP, and the API backends.

**Architecture:** Additive-then-remove ordering keeps every task green. First land the `temper-core` primitives and the identity-out `ref` rendering (additive). Then repoint each surface to build `ResourceRef::Uuid` via `parse_ref` (the `Scoped` variant still exists, just unused). Then collapse the edge source+target. Finally delete the `ResourceRef` type entirely, replacing every field with `ResourceId` — the compiler drives the last mechanical sweep. Backend-agnostic: this changes addressing on the **legacy** `public.*` backend today; the only `temper_next` touch is closing the native-id write-addressing stub for free.

**Tech Stack:** Rust workspace (temper-core, temper-cli, temper-mcp, temper-api, temper-client), clap, sqlx (offline macros + runtime queries), axum, rmcp (MCP). Tests via cargo-nextest. Build/check via cargo-make.

**Spec:** `docs/superpowers/specs/2026-06-17-ws6-surface-completeness-spec-a-addressing-collapse-design.md`

## Global Constraints

- **No premature backward compat** — delete `ResourceRef::Scoped` (and the whole `ResourceRef` type at the end), do not deprecate. The project keeps no compat shims.
- **`cargo make check` before every commit** — fmt + clippy (`-D warnings`) + docs + TS typecheck + biome. The pre-commit hook is a backstop, not the first line. A failing check on untouched files is a scope-creep signal — stop and report.
- **`cargo make` forces `SQLX_OFFLINE=true`** — after changing any `sqlx::query!`/`query_as!`/`query_scalar!` macro (or the schema it hits), regenerate the per-crate cache: `cargo make prepare-api` (temper-api) and `cargo make prepare-e2e` (e2e) — never `cargo sqlx prepare --workspace`. The `next_backend.rs` edge queries are runtime `sqlx::query_scalar(..)` (string), NOT macros — those need no cache. `relationship_service.rs` / `edge_service.rs` use macros — regen if their SQL changes.
- **ts-rs types** — if a changed type carries `#[cfg_attr(feature = "typescript", derive(TS))]` (check `AssertRelationshipRequest`, command structs), run `cargo make generate-ts-types` and commit the regenerated `packages/temper-ui` types. `ResourceRef` (`resource_ref.rs`) carries only `utoipa::ToSchema`, no ts-rs.
- **Typed structs over inline JSON; params structs for >5 args; service layer owns SQL; writes route through the backend trait, reads stay service-direct** — match existing patterns in each file.
- **Per-task testing** — focused test(s) + the touched crate's suite + `cargo make check`. Full-workspace nextest only at PR-prep (Task 9), not per task.
- **Decorated-ref shape** — `decorated_ref(title, id)` = `"{sluggify(title)}-{id}"`. Resolution is trailing-UUID-only: split on the last `-`-delimited UUID, ignore everything before it; a bare UUID is also valid. No fuzzy/fragment fallback — unparseable input is a typed error.

---

## File Structure

**Created:**
- `crates/temper-core/src/operations/refs.rs` — `sluggify`, `decorated_ref`, `parse_ref`. The one resolver (migrates to `temper-workflow` post-cutover).

**Modified (by task):**
- T1: `crates/temper-core/src/operations/mod.rs` (re-export refs), `crates/temper-cli/src/actions/ingest.rs` (delegate `slug_from_title` to core).
- T2: `crates/temper-cli/src/commands/resource.rs` (list/search/show render), `crates/temper-cli/src/commands/task.rs` / `session.rs` (show render), `crates/temper-mcp/src/tools/resources.rs` (`EnrichedResource`).
- T3: `crates/temper-cli/src/cloud_backend/backend.rs`, `crates/temper-cli/src/cloud_backend/translators.rs`.
- T4: `crates/temper-cli/src/cli.rs` (arg structs), `crates/temper-cli/src/commands/resource.rs` (`show`/`update`/`delete`), `task.rs`/`session.rs` (show by id), `crates/temper-cli/src/projection.rs` (remove-by-id).
- T5: `crates/temper-mcp/src/tools/resources.rs` (`get_resource`, `GetResourceInput`).
- T6: `crates/temper-core/src/operations/commands.rs` (`AssertRelationship`), `crates/temper-core/src/types/relationship_requests.rs`, `crates/temper-cli/src/commands/edge.rs`, `crates/temper-cli/src/cli.rs` (edge args), `crates/temper-mcp/src/tools/relationships.rs`, `crates/temper-api/src/handlers/edges.rs`, `crates/temper-api/src/backend/db_backend.rs` (assert), `crates/temper-api/src/backend/next_backend.rs` (assert), `crates/temper-api/src/services/edge_service.rs`.
- T7: `crates/temper-core/src/operations/resource_ref.rs` (deleted), `commands.rs`, `actions.rs`, all `ResourceRef` match sites (db_backend, next_backend, api translators, cloud_backend translators).
- T8: round-trip e2e (`tests/e2e/tests/`).
- T9: docs/templates + verification.

---

## Task 1: `temper-core` addressing primitives

**Files:**
- Create: `crates/temper-core/src/operations/refs.rs`
- Modify: `crates/temper-core/src/operations/mod.rs` (add `pub mod refs;` + re-exports)
- Modify: `crates/temper-cli/src/actions/ingest.rs:15-21` (delegate to core)
- Test: inline `#[cfg(test)]` in `refs.rs`

**Interfaces:**
- Produces:
  - `temper_core::operations::sluggify(title: &str) -> String`
  - `temper_core::operations::decorated_ref(title: &str, id: ResourceId) -> String`
  - `temper_core::operations::parse_ref(s: &str) -> Result<ResourceId, TemperError>`
- Consumes: `temper_core::types::ids::ResourceId` (newtype over `uuid::Uuid`), `temper_core::error::TemperError`.

- [ ] **Step 1: Write the failing tests** in `crates/temper-core/src/operations/refs.rs`

```rust
//! Resource addressing primitives — the one decorated-ref resolver.
//!
//! Identity contract (Adjudication 5): a resource is addressed by a bare
//! UUID or the decorated form `sluggify(title)-<uuid>`. Resolution is
//! trailing-UUID-only — the decoration is parsed off and ignored, so a
//! stale or wrong slug half is harmless. Decorations are never stored,
//! never authoritative. This module migrates to `temper-workflow` at
//! post-cutover crate extraction.

use crate::error::TemperError;
use crate::types::ids::ResourceId;
use uuid::Uuid;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sluggify_lowercases_and_dashes() {
        // Behavior is IDENTICAL to the legacy `slug_from_title`: each
        // non-alphanumeric, non-dash char → `-` (runs are NOT collapsed),
        // unicode letters are KEPT (is_alphanumeric is unicode-aware),
        // ends trimmed. The slug half of a ref is cosmetic (ignored on
        // parse), so exact prettiness doesn't matter — fidelity to the
        // existing contract does.
        assert_eq!(sluggify("Hello, World!"), "hello--world");
        assert_eq!(sluggify("  Trim --Me-- "), "trim---me");
        assert_eq!(sluggify("Café déjà"), "café-déjà");
    }

    #[test]
    fn decorated_ref_is_slug_dash_uuid() {
        let id = ResourceId(Uuid::parse_str("019e84ab-26ba-7560-9d34-c60d74a9fbe2").unwrap());
        assert_eq!(
            decorated_ref("My Task", id),
            "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2"
        );
    }

    #[test]
    fn parse_ref_accepts_bare_uuid() {
        let s = "019e84ab-26ba-7560-9d34-c60d74a9fbe2";
        assert_eq!(parse_ref(s).unwrap(), ResourceId(Uuid::parse_str(s).unwrap()));
    }

    #[test]
    fn parse_ref_accepts_decorated_and_ignores_slug_half() {
        let uuid = "019e84ab-26ba-7560-9d34-c60d74a9fbe2";
        let want = ResourceId(Uuid::parse_str(uuid).unwrap());
        // correct decoration
        assert_eq!(parse_ref(&format!("my-task-{uuid}")).unwrap(), want);
        // STALE/WRONG decoration resolves identically — harmless by construction
        assert_eq!(parse_ref(&format!("totally-wrong-slug-{uuid}")).unwrap(), want);
    }

    #[test]
    fn parse_ref_round_trips_decorated_ref() {
        let id = ResourceId(Uuid::now_v7());
        for title in ["A B C", "", "punct!@#", "already-slug"] {
            assert_eq!(parse_ref(&decorated_ref(title, id)).unwrap(), id);
        }
    }

    #[test]
    fn parse_ref_rejects_fragments_and_garbage() {
        // no trailing uuid → error, NO fuzzy fallback
        assert!(parse_ref("just-a-slug").is_err());
        assert!(parse_ref("").is_err());
        assert!(parse_ref("not-a-uuid-1234").is_err());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail to compile** (functions undefined)

Run: `cargo nextest run -p temper-core refs::`
Expected: FAIL — `cannot find function sluggify/decorated_ref/parse_ref`.

- [ ] **Step 3: Implement the three functions** in `refs.rs` (above the `tests` module)

```rust
/// Slugify a title for the decoration half of a ref / a filename.
/// Lowercase, non-alphanumeric (ascii) runs → `-`, trimmed.
pub fn sluggify(title: &str) -> String {
    title
        .to_lowercase()
        .replace(|c: char| !c.is_alphanumeric() && c != '-', "-")
        .trim_matches('-')
        .to_owned()
}

/// The decorated, self-resolving form printed for every resource:
/// `sluggify(title)-<uuid>`.
pub fn decorated_ref(title: &str, id: ResourceId) -> String {
    format!("{}-{}", sluggify(title), id.0)
}

/// Resolve a ref string to a `ResourceId`. Accepts a bare UUID or a
/// decorated `…-<uuid>` form; resolution is trailing-UUID-only (the
/// decoration is ignored). No fuzzy/fragment matching — unparseable input
/// is an error, never a guess.
pub fn parse_ref(s: &str) -> Result<ResourceId, TemperError> {
    let s = s.trim();
    // Bare UUID.
    if let Ok(id) = Uuid::parse_str(s) {
        return Ok(ResourceId(id));
    }
    // Decorated: the trailing UUID is the last 5 hyphen-delimited groups
    // (UUIDs contain 4 internal hyphens). Walk from the right.
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() >= 5 {
        let tail = parts[parts.len() - 5..].join("-");
        if let Ok(id) = Uuid::parse_str(&tail) {
            return Ok(ResourceId(id));
        }
    }
    Err(TemperError::Project(format!(
        "not a resource ref (expected a UUID or `slug-<uuid>`): {s:?}"
    )))
}
```

Note for the implementer: `sluggify` is byte-for-byte the existing `slug_from_title` body — `char::is_alphanumeric` is unicode-aware so accented letters are KEPT, and separator runs are NOT collapsed (`Hello, World!` → `hello--world`). Keep this behavior identical (the `café-déjà` / `hello--world` / `trim---me` assertions pin it); the slug half of a ref is cosmetic and ignored on parse, so fidelity to the existing contract matters more than prettiness.

- [ ] **Step 4: Wire the module** in `crates/temper-core/src/operations/mod.rs`

Add `pub mod refs;` with the other `pub mod` lines, and re-export alongside the existing operations re-exports:
```rust
pub use refs::{decorated_ref, parse_ref, sluggify};
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo nextest run -p temper-core refs::`
Expected: PASS (6 tests).

- [ ] **Step 6: Delegate the CLI's `slug_from_title` to core (DRY)** — `crates/temper-cli/src/actions/ingest.rs:15-21`

Replace the body so there is one slug function:
```rust
/// Slugify a title for use in URIs and slugs.
///
/// Thin delegator to `temper_core::operations::sluggify` — the canonical
/// slug function lives in core so the CLI projector and the ref resolver
/// agree by construction.
pub fn slug_from_title(title: &str) -> String {
    temper_core::operations::sluggify(title)
}
```

- [ ] **Step 7: Verify + commit**

Run: `cargo make check` then `cargo nextest run -p temper-core -p temper-cli refs:: slug`
Expected: check clean; slug/ref tests PASS.
```bash
git add crates/temper-core/src/operations/refs.rs crates/temper-core/src/operations/mod.rs crates/temper-cli/src/actions/ingest.rs
git commit -m "feat(core): decorated-ref addressing primitives (sluggify, decorated_ref, parse_ref)"
```

---

## Task 2: Identity-out `ref` rendering

Add a derived `ref` field (the decorated form) wherever a resource is printed, so callers can copy a self-resolving identifier. Additive — no addressing behavior changes yet.

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs` (`list` envelope injection; `show_generic` / `show_meta_only` metadata)
- Modify: `crates/temper-cli/src/commands/task.rs` / `session.rs` (`show` metadata)
- Modify: `crates/temper-cli/src/actions/search.rs` (search row rendering)
- Modify: `crates/temper-mcp/src/tools/resources.rs:183-237` (`EnrichedResource` + `build_enriched`)
- Test: inline tests in each + an MCP test mirroring the existing `fields_projection_tests`.

**Interfaces:**
- Consumes: `temper_core::operations::decorated_ref` (T1).
- Produces: a `ref` JSON key on every resource-shaped output row (CLI list/search/show, MCP enriched). For CLI, a helper `inject_ref(row: &mut serde_json::Value)` that reads `title`+`id` and inserts `ref`.

- [ ] **Step 1: Write the failing CLI helper test** — add to the `#[cfg(test)]` module in `crates/temper-cli/src/commands/resource.rs`

```rust
#[test]
fn inject_ref_adds_decorated_form_from_title_and_id() {
    let mut row = serde_json::json!({
        "id": "019e84ab-26ba-7560-9d34-c60d74a9fbe2",
        "title": "My Task",
    });
    super::inject_ref(&mut row);
    assert_eq!(
        row.get("ref").and_then(|v| v.as_str()),
        Some("my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2")
    );
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-cli inject_ref`
Expected: FAIL — `inject_ref` undefined.

- [ ] **Step 3: Implement `inject_ref`** in `crates/temper-cli/src/commands/resource.rs` (near the other module helpers)

```rust
/// Insert a derived `ref` key (the decorated, self-resolving identifier)
/// into a serialized resource row, computed from its id + `title`. The
/// `ref` is render-time only — never persisted, never on the wire type.
/// Reads the anchor id from `id` (ResourceRow) OR `resource_id`
/// (UnifiedSearchResultRow). No-op if the id is absent or unparseable.
pub(crate) fn inject_ref(row: &mut serde_json::Value) {
    let id = row
        .get("id")
        .or_else(|| row.get("resource_id"))
        .and_then(|v| v.as_str());
    let Some(id) = id else { return };
    let title = row.get("title").and_then(|v| v.as_str()).unwrap_or("");
    if let Ok(uuid) = uuid::Uuid::parse_str(id) {
        let decorated =
            temper_core::operations::decorated_ref(title, temper_core::types::ids::ResourceId(uuid));
        if let Some(obj) = row.as_object_mut() {
            obj.insert("ref".to_string(), serde_json::Value::String(decorated));
        }
    }
}
```

- [ ] **Step 4: Inject into the `list` envelope** — `crates/temper-cli/src/commands/resource.rs:376-458`, after the `envelope` is built and before field-filtering

Insert (right after `let mut envelope = serde_json::to_value(&response)…?;`):
```rust
    // Identity-out: every printed row carries its decorated `ref`.
    if let Some(rows) = envelope.get_mut("rows").and_then(|r| r.as_array_mut()) {
        for row in rows.iter_mut() {
            inject_ref(row);
        }
    }
```
(Field-filtering runs after, so `--fields ref` works; the anchor `id` is always preserved by `apply_top_level_filter`.)

- [ ] **Step 5: Inject into `show` + `search`**

- `show_generic` (`commands/resource.rs:739-741`): inject into `metadata` right before `render_resource_show(&metadata, …)` → `inject_ref(&mut metadata);`.
- `show_meta_only` (`commands/resource.rs:617-663`): inject into the `filtered` resource Value right before `render(&filtered, fmt)` at `:663`.
- `task::show` / `session::show` (`commands/task.rs`, `commands/session.rs`): each builds a metadata Value and calls `render`/`render_resource_show` — add `inject_ref(&mut metadata)` before that render call (find the `to_value(&row)` → render site in each).
- **search** (`commands/search_cmd.rs:46`): the path renders `&results: Vec<UnifiedSearchResultRow>` directly. Convert to a Value array first, inject per row, then render the Value:
  ```rust
  let mut results_value = serde_json::to_value(&results)
      .map_err(|e| crate::error::TemperError::Api(format!("search serialize: {e}")))?;
  if let Some(arr) = results_value.as_array_mut() {
      for row in arr.iter_mut() { crate::commands::resource::inject_ref(row); }
  }
  let rendered = crate::format::render(&results_value, fmt)?;
  ```
  (`inject_ref` reads `resource_id` for search rows — handled by the dual-key lookup.)

- [ ] **Step 6: Add `ref` to MCP `EnrichedResource`** — `crates/temper-mcp/src/tools/resources.rs:183-237`

Add the field to the struct (after `pub origin_uri: String,`):
```rust
    /// Decorated, self-resolving identifier: `sluggify(title)-<uuid>`.
    pub r#ref: String,
```
And set it in `build_enriched` (in the returned `EnrichedResource { … }`, using `row.id`):
```rust
        r#ref: temper_core::operations::decorated_ref(&row.title, row.id),
```

- [ ] **Step 7: MCP test** — add to `fields_projection_tests` in `resources.rs`

```rust
#[test]
fn enriched_resource_carries_decorated_ref() {
    let id = uuid::Uuid::parse_str("019e84ab-26ba-7560-9d34-c60d74a9fbe2").unwrap();
    let got = temper_core::operations::decorated_ref(
        "My Task",
        temper_core::types::ids::ResourceId(id),
    );
    assert_eq!(got, "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2");
}
```
(Unit-level proof of the value; full `build_enriched` is DB-bound and covered by e2e in Task 8.)

- [ ] **Step 8: Verify + commit**

Run: `cargo make check` then `cargo nextest run -p temper-cli -p temper-mcp inject_ref decorated_ref`
Expected: check clean; tests PASS.
```bash
git add crates/temper-cli/src/commands/resource.rs crates/temper-cli/src/commands/task.rs crates/temper-cli/src/commands/session.rs crates/temper-cli/src/actions/search.rs crates/temper-mcp/src/tools/resources.rs
git commit -m "feat(cli,mcp): emit decorated ref on every printed resource (identity-out)"
```

---

## Task 3: CloudBackend UUID dispatch (resource update/delete writes)

Make the CLI's `CloudBackend` handle `ResourceRef::Uuid` directly via the by-id client methods, skipping the `resolve_by_uri` round-trip. Additive — `Scoped` arms stay; this just adds the `Uuid` path the CLI will use in Task 4.

**VERIFIED scope correction:** Only `update_resource` + `delete_resource` go through `CloudBackend` (cloud writes route through the backend trait). `CloudBackend::show_resource` is a **stub** (`backend.rs:141-149`, `"reads stay surface-direct"`) — CLI reads use `runtime::with_client` + `client.resources().get(id)`/`content(id)` directly, NOT the backend. So this task touches update/delete only; leave `show_resource` as the stub.

**Files:**
- Modify: `crates/temper-cli/src/cloud_backend/backend.rs:86-139` (`update_resource` / `delete_resource` trait impls + the inherent `extract_scoped_update_components` helper at `:215`)
- Modify: `crates/temper-cli/src/cloud_backend/translators.rs:200-234` (`cmd_to_delete_args` → add a Uuid-aware discriminator)
- Test: `crates/temper-cli/src/cloud_backend/backend.rs` tests (mirror the existing scoped-component tests)

**Interfaces:**
- Consumes: `temper_client::resources()::{update(id: Uuid, req), delete(id: Uuid)}` (already by-UUID), `ResourceRef::{Uuid, Scoped}`.
- Produces: `CloudBackend::{update_resource, delete_resource}` resolve a `Uuid` ref to the by-id client call directly; a `Scoped` ref keeps the existing resolve-by-uri path (removed in Task 7).

- [ ] **Step 1: Write failing tests** — `crates/temper-cli/src/cloud_backend/backend.rs` tests module

```rust
#[test]
fn delete_uuid_ref_dispatches_by_id() {
    use temper_core::operations::{DeleteResource, ResourceRef};
    use temper_core::types::ids::ResourceId;
    let id = ResourceId(uuid::Uuid::now_v7());
    let cmd = DeleteResource {
        resource: ResourceRef::Uuid { id },
        force: false,
        origin: Surface::CliCloud,
    };
    // resolve_delete_target returns Right(uuid) for a Uuid ref (no resolve-by-uri)
    assert_eq!(resolve_delete_target(&cmd, "fallback").unwrap(), DeleteTarget::Id(uuid::Uuid::from(id)));
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo nextest run -p temper-cli delete_uuid_ref`
Expected: FAIL — `resolve_delete_target` / `DeleteTarget` undefined.

- [ ] **Step 3: Add a `DeleteTarget` discriminator + uuid-aware resolution** — `crates/temper-cli/src/cloud_backend/translators.rs`

```rust
/// How a delete command addresses its target.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DeleteTarget {
    /// Direct by-UUID (no resolve-by-uri round-trip).
    Id(uuid::Uuid),
    /// Legacy scoped slug — resolved via resolve_by_uri (removed in Task 7).
    Scoped { owner: String, context: String, doctype: String, slug: String },
}

#[cfg(feature = "embed")]
pub(crate) fn resolve_delete_target(
    cmd: &temper_core::operations::DeleteResource,
    fallback_owner: &str,
) -> Result<DeleteTarget> {
    use temper_core::operations::ResourceRef;
    match &cmd.resource {
        ResourceRef::Uuid { id } => Ok(DeleteTarget::Id(uuid::Uuid::from(*id))),
        ResourceRef::Scoped { owner, context, doctype, slug } => {
            let owner = if owner.is_empty() { fallback_owner } else { owner.as_str() };
            Ok(DeleteTarget::Scoped {
                owner: owner.to_string(),
                context: context.clone(),
                doctype: doctype.clone(),
                slug: slug.clone(),
            })
        }
    }
}
```
(The old `cmd_to_delete_args` stays for now; `delete_resource` in `backend.rs` switches to `resolve_delete_target` and branches: `Id` → `client.resources().delete(uuid)`; `Scoped` → existing resolve-by-uri-then-delete. Do the equivalent `UpdateTarget` discriminator for `update_resource` / `extract_scoped_update_components`. Do NOT touch `show_resource` — it is a stub by design.)

- [ ] **Step 4: Branch the two write trait impls on the target** in `crates/temper-cli/src/cloud_backend/backend.rs:86-139`

For `delete_resource` and `update_resource`: add the `Uuid` fast path (`client.resources().{delete,update}` by id) and keep the `Scoped` path. The current impls (`:111-139` delete, `:86-109` update) call `cmd_to_delete_args`/`extract_scoped_update_components` then `resolve_by_uri`; replace the head with the discriminator. `delete_resource` shape:
```rust
match resolve_delete_target(&cmd, &self.owner)? {
    DeleteTarget::Id(id) => {
        self.client.resources().delete(id).await.map_err(crate::commands::client_err)?;
        id
    }
    DeleteTarget::Scoped { owner, context, doctype, slug } => {
        let row = self.client.resources().resolve_by_uri(&owner, &context, &doctype, &slug)
            .await.map_err(crate::actions::runtime::client_err_to_temper)?;
        let id = *row.id.as_uuid();
        self.client.resources().delete(id).await.map_err(crate::commands::client_err)?;
        id
    }
};
```
(`self.owner` is the fallback owner field, `resolve_by_uri(owner, context, doc_type, ident)` per `crates/temper-client/src/resources.rs:105`. Mirror for `update_resource` with `client.resources().update(id, &req)`. Preserve the existing `DomainEvent::RemoteSynced { resource_id }` return + the `client_err` vs `client_err_to_temper` mapping the current delete uses.)

- [ ] **Step 5: Run tests + verify the by-id path**

Run: `cargo nextest run -p temper-cli delete_uuid_ref update_uuid_ref`
Expected: PASS.

- [ ] **Step 6: Verify + commit**

Run: `cargo make check`
```bash
git add crates/temper-cli/src/cloud_backend/
git commit -m "feat(cli): CloudBackend resolves Uuid resource refs by id (skip resolve-by-uri)"
```

---

## Task 4: CLI resource-command surface (show / update / delete)

Repoint `show`/`update`/`delete` to a single decorated-ref positional; drop `--type`/`--context`/`--owner`; read doctype/context from the resolved row where the command needs them.

**VERIFIED simplification:** `task::show` / `session::show` / `show_generic` are byte-identical except the hardcoded doctype string passed to `resolve_by_uri` — there is NO doctype-specific rendering (session.rs even documents "same as task::show and show_generic"). They are called ONLY from `resource::show`'s doctype dispatch (`resource.rs:619-621`), nowhere else. Under id addressing the dispatch is unnecessary: **`show` collapses to one read-by-id-and-render**, and `task::show` + `session::show` are deleted. Reads are context-free via `runtime::with_client` + `client.resources().get(id)`/`content(id)`/`get_meta(id)` — no backend, no context. Only writes (update/delete) need `build_backend(config, ctx)`; get the ctx from the fetched row's `context_name`.

**Files:**
- Modify: `crates/temper-cli/src/cli.rs:255-380` (Show/Update/Delete arg structs)
- Modify: `crates/temper-cli/src/commands/resource.rs` (`show`, `update`, `delete`, `show_meta_only`, `show_generic`, `show_edges`; `ShowParams`/`UpdateParams`)
- Delete: `crates/temper-cli/src/commands/task.rs::show`, `crates/temper-cli/src/commands/session.rs::show` (now dead)
- Modify: `crates/temper-cli/src/projection.rs` (a `remove_resource_file_for_row(vault_root, &ResourceRow)` wrapper)
- Modify: the `main.rs`/dispatch site mapping clap args → these functions
- Test: `crates/temper-cli/src/cli.rs` arg-parse tests + `crates/temper-cli/src/commands/resource.rs` tests

**Interfaces:**
- Consumes: `parse_ref` (T1), `CloudBackend` Uuid write dispatch (T3), `client.resources().{get(id), content(id), get_meta(id)}` (context-free reads via `runtime::with_client`), `build_backend(config, &row.context_name)` for writes.
- Produces: `show`/`update`/`delete` that take one `r#ref: String` positional → `parse_ref → id`. `show` reads + renders by id uniformly. `update`/`delete` fetch the row by id first (context-free) to learn `context_name` (for `build_backend`) + `doc_type_name` (update validation) + projection path, then dispatch the write with `ResourceRef::Uuid { id }`.

- [ ] **Step 1: Write the failing arg-parse test** — `crates/temper-cli/src/cli.rs` (or the CLI parse test module)

```rust
#[test]
fn show_accepts_bare_ref_and_rejects_type_flag() {
    use clap::Parser;
    // A single ref positional parses.
    assert!(Cli::try_parse_from([
        "temper", "resource", "show",
        "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2",
    ]).is_ok());
    // The removed --type flag is now an unknown arg → parse error.
    assert!(Cli::try_parse_from([
        "temper", "resource", "show", "some-ref", "--type", "task",
    ]).is_err());
}
```
(`Cli` is the top-level clap parser in `cli.rs`; place the test in `cli.rs`'s test module where other `try_parse_from` tests live. If the subcommand path differs, mirror the existing parse-test idiom in that module.)

- [ ] **Step 2: Run to verify it fails** (the struct still has `slug`/`r#type`, so `--type task` currently parses OK → the `is_err()` assertion fails)

Run: `cargo nextest run -p temper-cli show_accepts_bare_ref`
Expected: FAIL on the `--type` assertion (it still parses today).

- [ ] **Step 3: Rewrite the Show / Update / Delete clap args** — `crates/temper-cli/src/cli.rs`

Show becomes (drop `r#type`, `context`; keep `edges`/`meta_only`/`fields`):
```rust
    /// Show a resource's content
    Show {
        /// Resource ref: a UUID or the decorated `slug-<uuid>` form
        r#ref: String,
        /// Show graph edges connected to this resource
        #[arg(long)]
        edges: bool,
        /// Return only the resource's meta tier (no body)
        #[arg(long, conflicts_with = "edges")]
        meta_only: bool,
        /// Subselect top-level response keys
        #[arg(long, value_delimiter = ',')]
        fields: Vec<String>,
    },
```
Update: replace `slug` + `r#type`/`type_from`/`context` with `r#ref: String`; **keep** `type_to`/`context_to` (those are *mutations* — move/retype, not addressing), and all schema flags. Delete: replace `slug` + `r#type` + `context` with `r#ref: String`; keep `force`.

- [ ] **Step 4: Rewrite `delete`** — `crates/temper-cli/src/commands/resource.rs:527-571` (do first)

```rust
pub fn delete(config: &Config, r#ref: &str, force: bool, fmt: crate::format::OutputFormat) -> Result<()> {
    use temper_core::operations::{DeleteResource, ResourceRef};
    let id = temper_core::operations::parse_ref(r#ref)?;

    // Context-free read: fetch the row by id to learn its context (for the
    // write backend), doctype + slug (for projection removal + result shape).
    let row = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            client.resources().get(uuid::Uuid::from(id))
                .await.map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let cmd = DeleteResource {
        resource: ResourceRef::Uuid { id },
        force,
        origin: temper_core::operations::Surface::CliCloud,
    };
    let (runtime, backend, _client) = crate::backend_select::build_backend(config, &row.context_name)?;
    runtime.block_on(backend.delete_resource(cmd))?;

    if let Err(e) = crate::projection::remove_resource_file_for_row(&config.vault_root, &row) {
        output::warning(format!("could not remove projection file: {e}"));
    }

    let result = DeleteActionResult {
        status: "ok",
        slug: row.slug.clone().unwrap_or_default(),
        doc_type: row.doc_type_name.clone(),
    };
    println!("{}", crate::format::render(&result, fmt)?);
    Ok(())
}
```
Note: `remove_resource_file_for_row(vault_root, &ResourceRow)` wraps the existing `remove_resource_file`, deriving `owner` from `config.owner_for_context(&row.context_name)` and `context`/`doctype`/`slug` from the row (`row.context_name`, `row.doc_type_name`, `row.slug`). Add it to `projection.rs`. (Confirm `ResourceRow` carries `context_name`/`doc_type_name`/`slug` — it does, per the list output shape.)

- [ ] **Step 5: Unify `show`** — one read-by-id-and-render; delete the doctype dispatch and `task::show`/`session::show`

`ShowParams` loses `doc_type`/`context`, gains `r#ref`. Replace the whole `show` + the `show_generic` body with one function (the three old shows were identical):
```rust
pub fn show(config: &Config, params: ShowParams<'_>) -> Result<()> {
    let id = temper_core::operations::parse_ref(params.r#ref)?;
    if params.meta_only {
        return show_meta_only(config, id, params.format, params.fields);
    }
    let config_clone = config.clone();
    let (mut metadata, body) = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            let row = client.resources().get(uuid::Uuid::from(id))
                .await.map_err(crate::actions::runtime::client_err_to_temper)?;
            let resp = client.resources().content(uuid::Uuid::from(id))
                .await.map_err(crate::actions::runtime::client_err_to_temper)?;
            // best-effort projection refresh (unchanged helper)
            let _ = crate::projection::write_resource_file_from_parts(&config_clone.vault_root, &row, &resp);
            let metadata = serde_json::to_value(&row)
                .map_err(|e| TemperError::Api(format!("metadata serialize: {e}")))?;
            Ok((metadata, resp.markdown))
        })
    })?;
    inject_ref(&mut metadata);
    println!("{}", crate::format::render_resource_show(&metadata, &body, params.format)?);
    if params.edges {
        show_edges(config, id, params.format)?;   // re-key show_edges to take an id
    }
    Ok(())
}
```
Then: delete `crate::commands::task::show` + `crate::commands::session::show` and the `match params.doc_type` dispatch; re-key `show_edges` to fetch edges by id (`client.resources().edges(id)` per `resources.rs:95`); update `show_meta_only` to take `(config, id, fmt, fields)` and read via `client.resources().get_meta(id)` (`resources.rs:139`) instead of resolve-by-uri.

- [ ] **Step 6: Rewrite `update`** — resolve id, fetch row for doctype-scoped validation, update by id

`UpdateParams` loses `doc_type`/`type_from`/`context`, gains `r#ref`. Body:
```rust
    let id = temper_core::operations::parse_ref(params.r#ref)?;
    // Context-free read for doctype (validation) + context (write backend).
    let row = crate::actions::runtime::with_client(|client| {
        Box::pin(async move {
            client.resources().get(uuid::Uuid::from(id))
                .await.map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;
    let current_type = row.doc_type_name.clone();
    if let Some(tt) = params.type_to { let _ = temper_core::frontmatter::DocType::from_str(tt)?; }
    validate_update_args(params, &current_type)?;   // keyed by the resolved doctype
    // … --body resolution unchanged …
    let cmd = UpdateResource {
        resource: ResourceRef::Uuid { id },
        body: resolved_body.map(BodyUpdate::new),
        managed_meta: build_partial_managed_meta_from_args(params),
        open_meta: build_partial_open_meta_from_args(params),
        move_to: build_move_spec_from_args(params),   // uses type_to/context_to
        origin: temper_core::operations::Surface::CliCloud,
    };
    let (runtime, backend, client) = crate::backend_select::build_backend(config, &row.context_name)?;
    let output = runtime.block_on(backend.update_resource(cmd))?;
```
The projection-rewrite tail (`write_resource_file` from `output.value`) is unchanged.

- [ ] **Step 7: Update the dispatch site** (`main.rs` / wherever clap variants call these) to pass `r#ref` instead of `slug`/`type`/`context`.

- [ ] **Step 8: Run tests + verify**

Run: `cargo nextest run -p temper-cli` (the crate suite) and a manual smoke: `cargo run -p temper-cli -- resource show <a-real-decorated-ref>` against a dev login if available.
Expected: arg-parse tests PASS; crate suite green.

- [ ] **Step 9: `cargo make check` + commit**
```bash
git add crates/temper-cli/
git commit -m "feat(cli): show/update/delete address by decorated ref; drop slug-scope flags"
```

---

## Task 5: MCP `get_resource` collapses to id-only

**Files:**
- Modify: `crates/temper-mcp/src/tools/resources.rs:50-67` (`GetResourceInput`), `:420-510` (`get_resource`)
- Test: `resources.rs` tests

**Interfaces:**
- Consumes: `parse_ref` (T1), `resource_service::get_visible(pool, profile, id)`.
- Produces: `get_resource` accepting `id` (UUID or decorated, via `parse_ref`) only; the `slug`/`context_name` lookup arm and fields are gone.

- [ ] **Step 1: Failing test** — input no longer has `slug`/`context_name`

```rust
#[test]
fn get_resource_input_is_ref_only() {
    let raw = serde_json::json!({ "id": "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2" });
    let input: GetResourceInput = serde_json::from_value(raw).unwrap();
    assert_eq!(input.id, "my-task-019e84ab-26ba-7560-9d34-c60d74a9fbe2");
}
```

- [ ] **Step 2: Run to verify it fails** (field is `Option<Uuid>`, struct still has `slug`)

Run: `cargo nextest run -p temper-mcp get_resource_input_is_ref_only`
Expected: FAIL to compile.

- [ ] **Step 3: Rewrite `GetResourceInput`** — `resources.rs:50-67`

```rust
/// MCP input for get_resource.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetResourceInput {
    /// Resource ref: a UUID or the decorated `slug-<uuid>` form.
    pub id: String,
    /// If true, includes the full reconstituted markdown content.
    pub include_content: Option<bool>,
    /// Subselect top-level response keys (anchor `id` always preserved).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
}
```

- [ ] **Step 4: Rewrite `get_resource`** — `resources.rs:420-468` (replace the `match (id, slug)` block)

```rust
    let id = temper_core::operations::parse_ref(&input.id)
        .map_err(|e| rmcp::ErrorData::invalid_params(e.to_string(), None))?;
    let row = resource_service::get_visible(pool, profile.id, id.into())
        .await
        .map_err(|e| rmcp::ErrorData::internal_error(format!("Failed to get resource: {e}"), None))?;
```
The `context_service`/`get_by_slug` branch and its imports are deleted. The rest of `get_resource` (content/enrich/fields) is unchanged.

- [ ] **Step 5: Run + verify**

Run: `cargo nextest run -p temper-mcp`
Expected: crate suite green (delete the now-stale `get_resource_input_accepts_fields` stub's `slug`/`context_name` fields if present).

- [ ] **Step 6: `cargo make check` + commit**
```bash
git add crates/temper-mcp/src/tools/resources.rs
git commit -m "feat(mcp): get_resource addresses by decorated ref only (drop slug+context arm)"
```

---

## Task 6: Edge addressing collapse (source + target)

Collapse both edge endpoints to resolved ids. `AssertRelationship.target_slug: String` → `target: ResourceId`; source built as `ResourceRef::Uuid` (the variant is removed in Task 7). Atomic across the edge path.

**Files:**
- Modify: `crates/temper-core/src/operations/commands.rs:119-127` (`AssertRelationship`)
- Modify: `crates/temper-core/src/types/relationship_requests.rs:18-27` (`AssertRelationshipRequest`)
- Modify: `crates/temper-cli/src/cli.rs` (edge Assert args), `crates/temper-cli/src/commands/edge.rs:36-70`
- Modify: `crates/temper-mcp/src/tools/relationships.rs:28-49,104-148`
- Modify: `crates/temper-api/src/handlers/edges.rs:60-75`
- Modify: `crates/temper-api/src/backend/db_backend.rs:240-325` (assert: `TargetEndpoint::Resource` always)
- Modify: `crates/temper-api/src/backend/next_backend.rs:433-477` (assert: target id mapping; remove slug lookup + Scoped source arm)
- Modify: `crates/temper-api/src/services/edge_service.rs:240-256` (remove `asserted_payload_slug` live path; keep replay)
- Test: `crates/temper-api/tests/relationship_*_test.rs`, `commands.rs`/`relationship_requests.rs` unit tests

**Interfaces:**
- Consumes: `parse_ref` (T1), `ResolvedIds` bimap (next_backend), `TargetEndpoint::Resource`.
- Produces: `AssertRelationship { source: ResourceRef, target: ResourceId, edge_kind, polarity, label, weight, origin }` (source becomes `ResourceId` in Task 7); `AssertRelationshipRequest { source: ResourceRef, target: ResourceId, … }`.

- [ ] **Step 1: Failing test** — `commands.rs` (or a new edge test)

```rust
#[test]
fn assert_relationship_carries_resolved_target_id() {
    use temper_core::types::ids::ResourceId;
    let cmd = AssertRelationship {
        source: ResourceRef::Uuid { id: ResourceId(uuid::Uuid::nil()) },
        target: ResourceId(uuid::Uuid::now_v7()),
        edge_kind: EdgeKind::Near, polarity: Polarity::Forward,
        label: "rel".into(), weight: 1.0, origin: Surface::Mcp,
    };
    assert_ne!(uuid::Uuid::from(cmd.target), uuid::Uuid::nil());
}
```

- [ ] **Step 2: Run to verify it fails** (`target` field doesn't exist; `target_slug` does)

Run: `cargo nextest run -p temper-core assert_relationship_carries_resolved_target`
Expected: FAIL to compile.

- [ ] **Step 3: Change the command + request types**

`commands.rs:119-127`:
```rust
pub struct AssertRelationship {
    pub source: ResourceRef,
    pub target: ResourceId,          // was: target_slug: String
    pub edge_kind: crate::types::graph::EdgeKind,
    pub polarity: crate::types::graph::Polarity,
    pub label: String,
    pub weight: f64,
    pub origin: Surface,
}
```
`relationship_requests.rs:18-27`: same `target_slug: String → target: ResourceId` (update the doc comments — both endpoints are resolved ids now). If this struct derives `TS`, note it for `generate-ts-types` (Global Constraints).

- [ ] **Step 4: Update the API consumers (resolve target as an id, not a slug)**

- `handlers/edges.rs:60-75`: pass `target: req.target` (a `ResourceId`) into the command instead of `target_slug: req.target_slug`.
- `db_backend.rs:240-325`: the assert path builds `TargetEndpoint::Resource(uuid::Uuid::from(cmd.target))` unconditionally; **delete** the `None => TargetEndpoint::Slug(cmd.target_slug)` arm (`:323`) and the resolve-or-slug selection above it.
- `next_backend.rs:433-477`: replace the `public` slug-in-context target lookup (`:458-474`) with `let target_pub = uuid::Uuid::from(cmd.target);` then the existing `ids.to_new(target_pub)` map. Replace the `Scoped { .. } => NotImplemented` source arm with the source `ResourceId` (after Task 7 the match is gone; here, since `source` is still `ResourceRef`, build `let source_pub = match &cmd.source { ResourceRef::Uuid { id } => (*id).into(), ResourceRef::Scoped { .. } => unreachable!("surfaces build uuid refs") }` — Task 7 removes the dead arm).
- `edge_service.rs:240-256`: `asserted_payload_slug` is now unused by the live path — remove it (and any now-dead `TargetEndpoint::Slug` *construction* in the live assert). Keep `relationship_service.rs:222-223` (`TargetEndpoint::Slug` **replay** arm) untouched — historical events still carry slug targets.

- [ ] **Step 5: Update the surfaces to send resolved target ids**

- CLI `edge.rs:36-70`: the Assert arm takes a single `source` ref and a single `target` ref (clap args in `cli.rs`: replace `source_owner/context/doctype/source_slug` + `target` with `source: String` + `target: String`). Build:
  ```rust
  let source = ResourceRef::Uuid { id: temper_core::operations::parse_ref(&source)? };
  let target = temper_core::operations::parse_ref(&target)?;
  let req = AssertRelationshipRequest { source, target, edge_kind: kind.into(), polarity: polarity.into(), label, weight };
  ```
- MCP `relationships.rs:28-49`: `AssertRelationshipInput` loses `source_owner/context/doctype/source_slug` + `target_slug`; gains `source: String` + `target: String`. `assert_relationship` (`:104-148`) parses both via `parse_ref` and builds the command with `source: ResourceRef::Uuid{..}`, `target: <ResourceId>`.

- [ ] **Step 6: Regenerate sqlx cache if macro SQL changed**

If `edge_service.rs` / `db_backend.rs` edits touched any `sqlx::query!` macro:
Run: `cargo make prepare-api` (and `cargo make prepare-e2e` if e2e edge tests changed).

- [ ] **Step 7: Run edge tests**

Run: `cargo nextest run -p temper-api --features test-db relationship`
Expected: PASS (update fixtures that built `target_slug` to build a resolved `target` id; the `relationship_handler_test` / `relationship_write_test` refs).

- [ ] **Step 8: `cargo make check` + commit**
```bash
git add crates/temper-core crates/temper-cli crates/temper-mcp crates/temper-api
git commit -m "feat: collapse edge addressing — source+target resolved ids; target_slug retired from live asserts"
```

---

## Task 7: Delete `ResourceRef` — collapse to `ResourceId`

The final structural sweep. No surface builds `Scoped` anymore; delete the type and let the compiler point at every remaining reference.

**Files:**
- Delete: `crates/temper-core/src/operations/resource_ref.rs`
- Modify: `crates/temper-core/src/operations/mod.rs` (drop the module + re-export), `commands.rs` (`resource`/`source` fields → `ResourceId`), `actions.rs:389-395` (delete `validate_resource_ref` Scoped check)
- Modify: every match/construction site — `db_backend.rs:99-126`, `next_backend.rs:178-192,438-445`, `backend/translators.rs:175-200`, `cloud_backend/translators.rs:200-234`, `cloud_backend/backend.rs`, MCP/CLI ref constructions
- Test: existing suites (the collapse is type-driven; green = correct)

**Interfaces:**
- Produces: command fields are `ResourceId` directly. `parse_ref` output flows straight into commands. `NextBackend::resolve_new_id(&self, id: ResourceId)` (native-id write addressing closed).

- [ ] **Step 1: Change command field types to `ResourceId`** — `commands.rs`

`ShowResource`/`UpdateResource`/`DeleteResource` `pub resource: ResourceRef` → `pub resource: ResourceId`; `AssertRelationship` `pub source: ResourceRef` → `pub source: ResourceId`.

- [ ] **Step 2: Delete the type + its uses** — remove `resource_ref.rs`, the `pub mod resource_ref;` + re-export in `mod.rs`, and `validate_resource_ref` (`actions.rs`); delete its callers (the validation call in `validate_*` for resource refs — a `ResourceId` is always well-formed).

- [ ] **Step 3: Compile-drive the call sites**

Run: `cargo build -p temper-core` then `cargo build --workspace --features test-db,next-backend`
Fix each error mechanically:
- `db_backend.rs:99-126`: `show_resource` — drop the `match`, call `get_visible(pool, profile, *cmd.resource)` directly.
- `backend/translators.rs:175-200`: `resolve_resource_ref` — delete it (a `ResourceId` needs no resolution); callers use the id directly.
- `next_backend.rs:178-192`: `resolve_new_id` takes `ResourceId`, maps via `ResolvedIds`; `:438-445` source arm becomes `let source_pub = uuid::Uuid::from(cmd.source);`.
- `cloud_backend/translators.rs:200-234`: delete `cmd_to_delete_args` + the `Scoped` arm of `resolve_delete_target`/`extract_*` (only the `Id`/uuid path remains); `cloud_backend/backend.rs` delete/update/show drop their `Scoped` branches.
- CLI/MCP construction sites: `ResourceRef::Uuid { id }` → `id`.

- [ ] **Step 4: Run the affected suites**

Run: `cargo nextest run -p temper-core -p temper-cli -p temper-mcp -p temper-api --features test-db`
Expected: green. Regenerate caches if any macro SQL shifted (`cargo make prepare-api`).

- [ ] **Step 5: `cargo make check` (also `--features next-backend` for the gated arms)**

Run: `cargo make check` then `SQLX_OFFLINE=true cargo clippy -p temper-api --features next-backend -- -D warnings`
Expected: clean.

- [ ] **Step 6: Commit**
```bash
git add -A
git commit -m "refactor: delete ResourceRef; resource/edge commands carry ResourceId (native-id write addressing closed)"
```

---

## Task 8: Round-trip e2e (copy→paste loop)

Prove a decorated ref printed by `list`/`search` round-trips through `show`/`update`/`delete`, and an edge asserts by source+target ref — on the legacy backend.

**Files:**
- Create/extend: `tests/e2e/tests/` (a `decorated_ref_roundtrip` test in the resource e2e group)

- [ ] **Step 1: Write the e2e** (drives the real CLI/client → Axum → Postgres)

```rust
// 1. create a resource (legacy backend); capture its id.
// 2. `list` → assert a row carries `ref` == decorated_ref(title, id).
// 3. take that exact `ref` string, `show <ref>` → same resource (200, matching id).
// 4. `update <ref> --stage done` → 200; re-show → stage updated.
// 5. create a second resource; `edge assert <ref1> <ref2> near …` → 200; neighbors show the edge.
// 6. `delete <ref1>` → 200; show → 404.
// Also: a stale-decoration ref (wrong slug half, right uuid) resolves identically in step 3.
```

- [ ] **Step 2: Run**

Run: `cargo make test-e2e -E 'test(decorated_ref_roundtrip)'`
Expected: PASS. (Regenerate `cargo make prepare-e2e` if the test uses macro queries.)

- [ ] **Step 3: Commit**
```bash
git add tests/e2e/
git commit -m "test(e2e): decorated-ref round-trips list→show→update→edge→delete; stale slug-half harmless"
```

---

## Task 9: Docs/skill companion + final verification

**Files:**
- Modify: in-repo command-sequence docs/templates that speak `<slug> --type --context` — inventory first.
- Flag (out-of-repo): the installed temper skill at `~/.claude/skills/temper/` (SKILL.md / reference.md command sequences).

- [ ] **Step 1: Inventory in-repo command-sequence references**

Run:
```bash
rg -n "resource (show|update|delete).*--(type|context)|--type (task|goal|session)" docs crates/temper-cli/templates crates/temper-cli/src 2>/dev/null
```
Update each in-repo doc/help-text/template to the decorated-ref form (`temper resource show <ref>`). The clap `about`/`long_about` strings in `cli.rs` and any `templates/*.md` the CLI generates are in scope.

- [ ] **Step 2: Record the out-of-repo skill change**

The installed `~/.claude/skills/temper/` command sequences (`<slug> --type <t> --context <ctx>`) must be rewritten to decorated refs in lockstep — note this in the PR description as a required companion change (it ships with, but lives outside, this repo). Do not edit it as part of the repo commit; surface it to the user.

- [ ] **Step 3: Full-workspace verification (PR-prep)**

Run:
```bash
cargo make check
cargo make test-all
cargo make test-e2e
SQLX_OFFLINE=true cargo clippy -p temper-api --features next-backend -- -D warnings
```
Expected: all green. If any `sqlx` macro changed and a cache is stale, `cargo make prepare-api` / `prepare-e2e` and re-run.

- [ ] **Step 4: Commit any doc/help/template updates**
```bash
git add docs crates/temper-cli
git commit -m "docs(cli): command sequences address by decorated ref (slug-scope flags retired)"
```

---

## Self-Review

**Spec coverage:**
- §2 collapse + resolver → T1 (`parse_ref`/`sluggify`/`decorated_ref`) + T7 (delete `ResourceRef`).
- §3 CLI/MCP surface simplification → T4 (CLI show/update/delete) + T5 (MCP get_resource); fetch-then-dispatch-on-doctype caveat → T4 Steps 5-6.
- §3a edge source+target collapse → T6 (with the `TargetEndpoint::Slug`-replay-stays nuance, T6 Step 4).
- §4 identity-out `ref` rendering → T2; vault-filename deferral → honored (no projection-filename task exists; T4 only re-keys removal by the resolved row, not a rename).
- §5 native-id write addressing → T7 Step 3 (`resolve_new_id(ResourceId)`, dead arms removed).
- §6 no DB migration (none in plan); tests at each task; skill companion → T9.

**Placeholder scan:** no TBD/TODO; mechanical sweeps (T7 Step 3) enumerate exact sites with the transform shown. Code-bearing steps carry code. The few "confirm at the call site" notes point at a named file:line, not a vague instruction.

**Type consistency:** `parse_ref → ResourceId` consumed identically in T2/T4/T5/T6/T7; `AssertRelationship.target: ResourceId` (T6) matches `AssertRelationshipRequest.target` (T6) and the `TargetEndpoint::Resource(uuid::Uuid::from(target))` consumer; `decorated_ref(title, id)` signature stable across T1/T2.

**Known plan-time confirmations (named, not placeholders):** `build_backend_uuid` vs reusing `build_backend` with a default context (T4 Step 4) — confirm what `build_backend` needs the context for; the exact `resolve_by_uri` client arg order (T3 Step 4); whether `AssertRelationshipRequest` derives `TS` (Global Constraints → `generate-ts-types`).
