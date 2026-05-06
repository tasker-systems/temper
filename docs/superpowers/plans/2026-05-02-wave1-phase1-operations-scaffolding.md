# Wave 1 Phase 1 — Operations Module Scaffolding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Scaffold the new `temper-core/src/operations/` module with command structs, `ResourceRef` enum, `Surface` enum, `Backend` trait, `DomainEvent` enum, and `CommandOutput<T>`. No behavior change. This is the type-level foundation that every later Wave 1 phase rides on.

**Architecture:** Add a new sibling module `operations/` to `temper-core/src/` that depends on existing `types/` and `frontmatter/` modules. Define types only; no implementations of the trait yet. Each command struct uses `ResourceRef` for resource-action commands (`Show`, `Update`, `Delete`) per the spec; `Create`, `List`, and `Search` carry their own identification or query inputs.

**Tech Stack:** Rust 2021, `async_trait` for trait async dispatch, `serde` for command/event serialization, `thiserror` for error types. Tests use the existing `cargo nextest` harness.

**Specs:**
- `docs/superpowers/specs/2026-05-01-shared-core-execution-paths-design.md` (#4)
- `docs/superpowers/specs/2026-05-01-cloud-first-reframe-and-manifest-redefinition-design.md` (#3)

**Out of scope for this plan:** Backend implementations (Phases 3–4), shared action migrations (Phase 2), state machines (Phase 6 / spec #3), surface dispatch unification (Phase 5).

---

## File Structure

**New files (all under `crates/temper-core/src/operations/`):**

| File | Responsibility |
|---|---|
| `mod.rs` | Module root + public re-exports for the operations layer |
| `surface.rs` | `Surface` enum (`CliLocalVault`, `CliCloud`, `Mcp`, `ApiHttp`) |
| `resource_ref.rs` | `ResourceRef` enum (`Uuid` / `Scoped`) |
| `commands.rs` | Command structs: `CreateResource`, `ShowResource`, `UpdateResource`, `DeleteResource`, `ListResources`, `SearchResources` + supporting input types (`BodyUpdate`, etc.) |
| `events.rs` | `DomainEvent` enum with backend-qualified variants |
| `output.rs` | `CommandOutput<T>` |
| `backend.rs` | `Backend` async trait |

**Modified files:**

| File | Change |
|---|---|
| `crates/temper-core/src/lib.rs` | Add `pub mod operations;` |
| `crates/temper-core/Cargo.toml` | Add `async-trait = "0.1"` to `[dependencies]` |

**Conventions followed:**
- Each file under `operations/` declares its own `#[cfg(test)] mod tests` for unit tests local to that file (matches existing `defaults.rs` and `validation.rs` patterns in `temper-core`).
- Public types re-exported from `mod.rs` for ergonomic imports (`use temper_core::operations::{ResourceRef, Surface, ...};`).
- Commands carry `pub origin: Surface` per the spec sketch.

---

## Task 1: Add `async-trait` dependency to temper-core

**Files:**
- Modify: `crates/temper-core/Cargo.toml`

- [ ] **Step 1: Inspect current dependencies block**

Run: `grep -n "async-trait\|^\[dependencies\]" crates/temper-core/Cargo.toml`

Expected: shows `[dependencies]` line near the top, no existing `async-trait` entry.

- [ ] **Step 2: Add `async-trait` to dependencies**

In `crates/temper-core/Cargo.toml`, under `[dependencies]`, alphabetically place:

```toml
async-trait = "0.1"
```

Place it directly after the `[dependencies]` line (or alphabetically among existing entries — `async-trait` sorts before `base64`).

- [ ] **Step 3: Verify the crate still builds**

Run: `cargo build -p temper-core`

Expected: builds cleanly. `async-trait` is a small macro crate with minimal compile cost.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/Cargo.toml Cargo.lock
git commit -m "chore(core): add async-trait dep for operations Backend trait"
```

---

## Task 2: Create `operations/` module skeleton

**Files:**
- Create: `crates/temper-core/src/operations/mod.rs`
- Modify: `crates/temper-core/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/temper-core/src/operations/mod.rs` with this content:

```rust
//! Operations layer — commands, actions, events, and the Backend trait.
//!
//! This module is the canonical home for command/action/event vocabulary
//! shared across all surfaces (CLI-local-vault, CLI-cloud, MCP, API-HTTP)
//! and both backends (DbBackend in temper-api, VaultBackend in temper-cli).
//!
//! See `docs/superpowers/specs/2026-05-01-shared-core-execution-paths-design.md`.

#[cfg(test)]
mod smoke {
    /// Smoke test: the module compiles and is reachable.
    #[test]
    fn module_exists() {
        // No-op; existence of this test passing means the module compiled.
    }
}
```

- [ ] **Step 2: Wire module into `lib.rs`**

In `crates/temper-core/src/lib.rs`, after the existing `pub mod` declarations, add:

```rust
pub mod operations;
```

Place it alphabetically — between `pub mod normalize;` and `pub mod schema;`.

- [ ] **Step 3: Run the smoke test**

Run: `cargo nextest run -p temper-core operations::smoke::module_exists`

Expected: PASS. (One test passed.)

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/src/operations/mod.rs crates/temper-core/src/lib.rs
git commit -m "feat(core): scaffold operations module with smoke test"
```

---

## Task 3: Define `Surface` enum

**Files:**
- Create: `crates/temper-core/src/operations/surface.rs`
- Modify: `crates/temper-core/src/operations/mod.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/temper-core/src/operations/surface.rs`:

```rust
//! Surface enum — identifies the originating surface of a command.
//!
//! Each command carries a `Surface` so backends can adjust output formatting,
//! error shaping, and telemetry tagging based on where the command came from.

use serde::{Deserialize, Serialize};

/// The originating surface of a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// CLI binary with a local vault (no `TEMPER_VAULT_STATE=cloud`).
    CliLocalVault,
    /// CLI binary in cloud mode (`TEMPER_VAULT_STATE=cloud`).
    CliCloud,
    /// MCP server (rmcp tools, in-process to temper-api).
    Mcp,
    /// API server (Axum handlers receiving inbound HTTP).
    ApiHttp,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn surface_serializes_snake_case() {
        let s = serde_json::to_string(&Surface::CliLocalVault).unwrap();
        assert_eq!(s, "\"cli_local_vault\"");
    }

    #[test]
    fn surface_round_trips() {
        for variant in [
            Surface::CliLocalVault,
            Surface::CliCloud,
            Surface::Mcp,
            Surface::ApiHttp,
        ] {
            let s = serde_json::to_string(&variant).unwrap();
            let back: Surface = serde_json::from_str(&s).unwrap();
            assert_eq!(variant, back);
        }
    }
}
```

In `crates/temper-core/src/operations/mod.rs`, add at the top (after the doc-comment block):

```rust
mod surface;

pub use surface::Surface;
```

- [ ] **Step 2: Run the failing test**

Run: `cargo nextest run -p temper-core operations::surface`

Expected: PASS. The tests cover the only defined behavior (snake_case serialization + round-trip). They serve as a smoke check that the type compiles and serde derives work.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-core/src/operations/
git commit -m "feat(core): add Surface enum to operations module"
```

---

## Task 4: Define `ResourceRef` enum

**Files:**
- Create: `crates/temper-core/src/operations/resource_ref.rs`
- Modify: `crates/temper-core/src/operations/mod.rs`

- [ ] **Step 1: Write the file with tests first**

Create `crates/temper-core/src/operations/resource_ref.rs`:

```rust
//! ResourceRef — identifier for resource-action commands.
//!
//! Slug uniqueness is scoped to (owner, context, doctype); UUID is globally
//! unique. Every resource-action command (`Show`, `Update`, `Delete`, sync
//! variants) accepts either form. The enum shape (rather than two `Option`
//! fields) makes "exactly one form populated" a compile-time guarantee.

use serde::{Deserialize, Serialize};

use crate::types::ids::ResourceId;

/// Identifies a resource for a command that targets an existing resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ResourceRef {
    /// Globally-unique reference. Resolves directly without scoping fields.
    Uuid {
        #[serde(rename = "resource_id")]
        id: ResourceId,
    },
    /// Slug-based reference scoped by doctype + context.
    Scoped {
        slug: String,
        doctype: String,
        context: String,
    },
}

impl ResourceRef {
    /// Construct a UUID-based reference.
    pub fn uuid(id: ResourceId) -> Self {
        Self::Uuid { id }
    }

    /// Construct a scoped (slug-based) reference.
    pub fn scoped(
        slug: impl Into<String>,
        doctype: impl Into<String>,
        context: impl Into<String>,
    ) -> Self {
        Self::Scoped {
            slug: slug.into(),
            doctype: doctype.into(),
            context: context.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn scoped_constructor_sets_fields() {
        let r = ResourceRef::scoped("hello-world", "task", "temper");
        match r {
            ResourceRef::Scoped { slug, doctype, context } => {
                assert_eq!(slug, "hello-world");
                assert_eq!(doctype, "task");
                assert_eq!(context, "temper");
            }
            ResourceRef::Uuid { .. } => panic!("expected Scoped variant"),
        }
    }

    #[test]
    fn uuid_constructor_sets_id() {
        let id = ResourceId(Uuid::nil());
        let r = ResourceRef::uuid(id.clone());
        match r {
            ResourceRef::Uuid { id: got } => assert_eq!(got, id),
            ResourceRef::Scoped { .. } => panic!("expected Uuid variant"),
        }
    }

    #[test]
    fn scoped_round_trips_via_serde() {
        let r = ResourceRef::scoped("foo", "task", "temper");
        let s = serde_json::to_string(&r).unwrap();
        let back: ResourceRef = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);
    }

    #[test]
    fn uuid_round_trips_via_serde() {
        let r = ResourceRef::uuid(ResourceId(Uuid::nil()));
        let s = serde_json::to_string(&r).unwrap();
        let back: ResourceRef = serde_json::from_str(&s).unwrap();
        assert_eq!(r, back);
    }
}
```

Note on `ResourceId`: confirm via `grep -n "pub struct ResourceId\|ResourceId" crates/temper-core/src/types/ids.rs` that `ResourceId(pub Uuid)` exists. The macro-generated newtype is exported via `crate::types::ids::ResourceId`. If the path is different, adjust the import accordingly — do not invent a new id type for this task.

In `crates/temper-core/src/operations/mod.rs`, add:

```rust
mod resource_ref;

pub use resource_ref::ResourceRef;
```

Place this `mod`/`pub use` pair after the existing `surface` pair, alphabetically.

- [ ] **Step 2: Run the tests**

Run: `cargo nextest run -p temper-core operations::resource_ref`

Expected: 4 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-core/src/operations/
git commit -m "feat(core): add ResourceRef enum (uuid | scoped)"
```

---

## Task 5: Define `BodyUpdate` and command-input types

**Files:**
- Create: `crates/temper-core/src/operations/inputs.rs`
- Modify: `crates/temper-core/src/operations/mod.rs`

- [ ] **Step 1: Create the inputs module**

Create `crates/temper-core/src/operations/inputs.rs`:

```rust
//! Input types used by operation commands.
//!
//! `BodyUpdate` represents the new body content for an update; `ListFilter`
//! and `SearchQuery` carry list/search inputs. Kept small and serde-friendly.

use serde::{Deserialize, Serialize};

/// New body content for an `UpdateResource` (or `CreateResource`) command.
///
/// Wraps a String so we can extend with body-meta fields (e.g., explicit
/// content hash, encoding) without breaking the command struct.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BodyUpdate {
    pub content: String,
}

impl BodyUpdate {
    pub fn new(content: impl Into<String>) -> Self {
        Self { content: content.into() }
    }
}

/// Filter inputs for `ListResources`.
///
/// All fields optional — caller passes the subset they want to filter by.
/// Stage / doctype / context filters mirror what the API surface accepts today.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListFilter {
    pub doctype: Option<String>,
    pub context: Option<String>,
    pub stage: Option<String>,
    pub goal: Option<String>,
    pub limit: Option<u32>,
}

/// Query input for `SearchResources`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchQuery {
    pub query: String,
    pub doctype: Option<String>,
    pub context: Option<String>,
    pub limit: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_update_new_wraps_content() {
        let b = BodyUpdate::new("hello");
        assert_eq!(b.content, "hello");
    }

    #[test]
    fn list_filter_default_is_all_none() {
        let f = ListFilter::default();
        assert!(f.doctype.is_none());
        assert!(f.context.is_none());
        assert!(f.stage.is_none());
        assert!(f.goal.is_none());
        assert!(f.limit.is_none());
    }

    #[test]
    fn search_query_round_trips() {
        let q = SearchQuery {
            query: "rust".to_string(),
            doctype: Some("task".to_string()),
            context: None,
            limit: Some(10),
        };
        let s = serde_json::to_string(&q).unwrap();
        let back: SearchQuery = serde_json::from_str(&s).unwrap();
        assert_eq!(q, back);
    }
}
```

In `crates/temper-core/src/operations/mod.rs`, add (alphabetically among existing `mod` lines):

```rust
mod inputs;

pub use inputs::{BodyUpdate, ListFilter, SearchQuery};
```

- [ ] **Step 2: Run the tests**

Run: `cargo nextest run -p temper-core operations::inputs`

Expected: 3 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-core/src/operations/
git commit -m "feat(core): add BodyUpdate, ListFilter, SearchQuery inputs"
```

---

## Task 6: Define command structs

**Files:**
- Create: `crates/temper-core/src/operations/commands.rs`
- Modify: `crates/temper-core/src/operations/mod.rs`

- [ ] **Step 1: Create the commands module**

Create `crates/temper-core/src/operations/commands.rs`:

```rust
//! Command structs — declarative intent for a single operation.
//!
//! Commands are pure data. Surfaces translate user input (clap args, MCP
//! tool params, HTTP body) into a command; backends consume commands and
//! emit `CommandOutput`.
//!
//! Resource-action commands (`Show`, `Update`, `Delete`) carry a `ResourceRef`,
//! not a bare slug — slug uniqueness is scoped to (owner, context, doctype),
//! UUID is globally unique. `Create` carries the new resource's identity
//! directly. `List` and `Search` carry filter/query inputs.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::managed_meta::ManagedMeta;

use super::{
    inputs::{BodyUpdate, ListFilter, SearchQuery},
    resource_ref::ResourceRef,
    surface::Surface,
};

/// Create a new resource. The resource's identity is specified directly
/// (not via `ResourceRef`) since it doesn't exist yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CreateResource {
    pub slug: String,
    pub doctype: String,
    pub context: String,
    pub title: String,
    pub body: Option<BodyUpdate>,
    /// Caller-supplied managed_meta. Backends will apply doctype defaults
    /// for any required fields the caller omitted.
    pub managed_meta: ManagedMeta,
    /// Free-form user metadata (open_meta tier).
    pub open_meta: Option<Value>,
    pub origin: Surface,
}

/// Show a single resource.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShowResource {
    pub resource: ResourceRef,
    pub origin: Surface,
}

/// Update a resource — partial; any combination of body, managed_meta,
/// open_meta may be supplied.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateResource {
    pub resource: ResourceRef,
    pub body: Option<BodyUpdate>,
    pub managed_meta: Option<ManagedMeta>,
    pub open_meta: Option<Value>,
    pub origin: Surface,
}

/// Delete a resource. In the cloud-first model this is soft-delete on the
/// server with optional local-file removal as a tail action (handled by
/// VaultBackend in CliLocalVault surface).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeleteResource {
    pub resource: ResourceRef,
    /// Bypass the local-file confirmation prompt (required for non-TTY).
    pub force: bool,
    pub origin: Surface,
}

/// List resources, filtered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ListResources {
    pub filter: ListFilter,
    pub origin: Surface,
}

/// Semantic / hybrid search.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchResources {
    pub query: SearchQuery,
    pub origin: Surface,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_resource_carries_resource_ref() {
        let cmd = ShowResource {
            resource: ResourceRef::scoped("hello", "task", "temper"),
            origin: Surface::CliLocalVault,
        };
        // Exercises the type compiles + the field is reachable.
        assert!(matches!(cmd.resource, ResourceRef::Scoped { .. }));
    }

    #[test]
    fn update_resource_all_optional_fields_default_none() {
        let cmd = UpdateResource {
            resource: ResourceRef::scoped("x", "task", "temper"),
            body: None,
            managed_meta: None,
            open_meta: None,
            origin: Surface::ApiHttp,
        };
        assert!(cmd.body.is_none());
        assert!(cmd.managed_meta.is_none());
        assert!(cmd.open_meta.is_none());
    }

    #[test]
    fn create_resource_does_not_use_resource_ref() {
        let cmd = CreateResource {
            slug: "new-task".to_string(),
            doctype: "task".to_string(),
            context: "temper".to_string(),
            title: "New task".to_string(),
            body: None,
            managed_meta: ManagedMeta::default(),
            open_meta: None,
            origin: Surface::CliCloud,
        };
        assert_eq!(cmd.slug, "new-task");
    }

    #[test]
    fn list_resources_default_filter_serializes() {
        let cmd = ListResources {
            filter: ListFilter::default(),
            origin: Surface::Mcp,
        };
        let s = serde_json::to_string(&cmd).unwrap();
        let back: ListResources = serde_json::from_str(&s).unwrap();
        assert_eq!(cmd, back);
    }
}
```

In `crates/temper-core/src/operations/mod.rs`, add (alphabetically among existing `mod` lines):

```rust
mod commands;

pub use commands::{
    CreateResource, DeleteResource, ListResources, SearchResources, ShowResource, UpdateResource,
};
```

- [ ] **Step 2: Run the tests**

Run: `cargo nextest run -p temper-core operations::commands`

Expected: 4 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-core/src/operations/
git commit -m "feat(core): add command structs (Create/Show/Update/Delete/List/Search)"
```

---

## Task 7: Define `DomainEvent` enum

**Files:**
- Create: `crates/temper-core/src/operations/events.rs`
- Modify: `crates/temper-core/src/operations/mod.rs`

- [ ] **Step 1: Create the events module**

Create `crates/temper-core/src/operations/events.rs`:

```rust
//! DomainEvent — past-tense facts emitted by backend actions.
//!
//! Events are backend-qualified: `DbResourceCreated` / `VaultFileWritten`
//! describe state transitions in a specific backend. The `CliLocalVault`
//! surface composes events from both backends when its operation chains
//! them (e.g., write file + push, which emits Vault* + Db* events).
//!
//! Initial variant set covers the operations defined in Phase 1 commands.
//! Phase 6 (companion spec #3) adds state-machine-related variants.

use serde::{Deserialize, Serialize};

use crate::types::ids::ResourceId;

/// A past-tense fact about something that happened during command execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum DomainEvent {
    // -------- DbBackend events --------
    /// A new resource row was inserted in the database.
    DbResourceCreated { resource_id: ResourceId },
    /// A resource row was updated; version increments on the server side.
    DbResourceUpdated { resource_id: ResourceId },
    /// A resource row was soft-deleted (`is_active = false`).
    DbResourceSoftDeleted { resource_id: ResourceId },
    /// Chunks were regenerated for a resource (body changed).
    DbChunksGenerated { resource_id: ResourceId },
    /// Embedding was triggered (asynchronous on the server).
    DbEmbeddingTriggered { resource_id: ResourceId },

    // -------- VaultBackend events --------
    /// A vault file was written (created or modified).
    VaultFileWritten { path: String },
    /// The manifest entry for a resource was updated.
    VaultManifestUpdated { path: String },
    /// A vault file was removed.
    VaultFileRemoved { path: String },

    // -------- Composite / cross-backend events --------
    /// A vault-side change was successfully pushed to the API (DbBackend).
    RemoteSynced { resource_id: ResourceId },
    /// A push attempt was deferred (offline / not authed); manifest tracks pending.
    PushDeferred { reason: PushDeferReason },
}

/// Reason a push was deferred to bulk-recovery sync.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushDeferReason {
    Offline,
    NotAuthed,
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn db_event_round_trips() {
        let e = DomainEvent::DbResourceCreated {
            resource_id: ResourceId(Uuid::nil()),
        };
        let s = serde_json::to_string(&e).unwrap();
        let back: DomainEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn vault_event_round_trips() {
        let e = DomainEvent::VaultFileWritten {
            path: "@me/temper/task/foo.md".to_string(),
        };
        let s = serde_json::to_string(&e).unwrap();
        let back: DomainEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn push_deferred_carries_reason() {
        let e = DomainEvent::PushDeferred {
            reason: PushDeferReason::Offline,
        };
        let s = serde_json::to_string(&e).unwrap();
        assert!(s.contains("offline"));
    }
}
```

In `crates/temper-core/src/operations/mod.rs`, add (alphabetically):

```rust
mod events;

pub use events::{DomainEvent, PushDeferReason};
```

- [ ] **Step 2: Run the tests**

Run: `cargo nextest run -p temper-core operations::events`

Expected: 3 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-core/src/operations/
git commit -m "feat(core): add DomainEvent enum with Db/Vault/composite variants"
```

---

## Task 8: Define `CommandOutput<T>`

**Files:**
- Create: `crates/temper-core/src/operations/output.rs`
- Modify: `crates/temper-core/src/operations/mod.rs`

- [ ] **Step 1: Create the output module**

Create `crates/temper-core/src/operations/output.rs`:

```rust
//! CommandOutput — the value-plus-events return shape for every Backend method.

use serde::{Deserialize, Serialize};

use super::events::DomainEvent;

/// What a backend returns from a command: the typed value plus any events
/// emitted during execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommandOutput<T> {
    pub value: T,
    pub events: Vec<DomainEvent>,
}

impl<T> CommandOutput<T> {
    /// Build a `CommandOutput` with no events. Useful for trivial returns.
    pub fn new(value: T) -> Self {
        Self { value, events: Vec::new() }
    }

    /// Build a `CommandOutput` with an explicit events vector.
    pub fn with_events(value: T, events: Vec<DomainEvent>) -> Self {
        Self { value, events }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_output_has_empty_events() {
        let out = CommandOutput::new(42_u32);
        assert_eq!(out.value, 42);
        assert!(out.events.is_empty());
    }

    #[test]
    fn with_events_keeps_events() {
        use crate::operations::events::PushDeferReason;
        let events = vec![DomainEvent::PushDeferred { reason: PushDeferReason::Offline }];
        let out = CommandOutput::with_events("hello", events);
        assert_eq!(out.value, "hello");
        assert_eq!(out.events.len(), 1);
    }
}
```

In `crates/temper-core/src/operations/mod.rs`, add (alphabetically):

```rust
mod output;

pub use output::CommandOutput;
```

- [ ] **Step 2: Run the tests**

Run: `cargo nextest run -p temper-core operations::output`

Expected: 2 tests pass.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-core/src/operations/
git commit -m "feat(core): add CommandOutput<T> wrapper for backend return values"
```

---

## Task 9: Define `Backend` trait

**Files:**
- Create: `crates/temper-core/src/operations/backend.rs`
- Modify: `crates/temper-core/src/operations/mod.rs`

- [ ] **Step 1: Create the backend module**

Create `crates/temper-core/src/operations/backend.rs`:

```rust
//! Backend trait — the contract every operations backend implements.
//!
//! Two impls are planned:
//! - `DbBackend` in `temper-api` (Postgres persistence + chunking + embedding)
//! - `VaultBackend` in `temper-cli` (local-file persistence with optional sync)
//!
//! Both produce `CommandOutput<T>` per command — typed value + events emitted.
//!
//! The trait is intentionally minimal in Phase 1: each method takes a command
//! and returns a `CommandOutput<T>`. Backend-specific operations (manifest
//! refresh, sync push/pull) live on the backend's concrete type, not on
//! the shared trait.

use async_trait::async_trait;

use crate::error::TemperError;
use crate::types::resource::ResourceRow;

use super::commands::{
    CreateResource, DeleteResource, ListResources, SearchResources, ShowResource, UpdateResource,
};
use super::output::CommandOutput;

/// Lightweight summary of a resource for `list` results.
#[derive(Debug, Clone)]
pub struct ResourceSummary {
    pub slug: String,
    pub doctype: String,
    pub context: String,
    pub title: String,
}

/// A search hit — a resource summary plus relevance metadata.
#[derive(Debug, Clone)]
pub struct SearchHit {
    pub summary: ResourceSummary,
    pub score: f32,
}

/// The shared contract for both DbBackend (in temper-api) and VaultBackend
/// (in temper-cli). Each command method takes a command struct, executes it
/// against the backend's persistence, and returns a `CommandOutput<T>` with
/// the typed value plus emitted events.
#[async_trait]
pub trait Backend: Send + Sync {
    async fn create_resource(
        &self,
        cmd: CreateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError>;

    async fn show_resource(
        &self,
        cmd: ShowResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError>;

    async fn update_resource(
        &self,
        cmd: UpdateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError>;

    async fn delete_resource(
        &self,
        cmd: DeleteResource,
    ) -> Result<CommandOutput<()>, TemperError>;

    async fn list_resources(
        &self,
        cmd: ListResources,
    ) -> Result<CommandOutput<Vec<ResourceSummary>>, TemperError>;

    async fn search_resources(
        &self,
        cmd: SearchResources,
    ) -> Result<CommandOutput<Vec<SearchHit>>, TemperError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verify the trait is object-safe (callable via `dyn Backend`).
    /// If this compiles, dispatch through trait objects works.
    #[allow(dead_code)]
    fn assert_object_safe(_: &dyn Backend) {}

    #[test]
    fn resource_summary_can_be_constructed() {
        let s = ResourceSummary {
            slug: "x".to_string(),
            doctype: "task".to_string(),
            context: "temper".to_string(),
            title: "X".to_string(),
        };
        assert_eq!(s.slug, "x");
    }
}
```

Note on imports — verified at plan-write time:
- `TemperError` lives in `crates/temper-core/src/error.rs:19` (`pub enum TemperError`).
- `ResourceRow` lives in `crates/temper-core/src/types/resource.rs:18` (`pub struct ResourceRow`).
- Both are public.

Do not create new error or resource types for this task.

In `crates/temper-core/src/operations/mod.rs`, add (alphabetically — `backend` sorts to the top of the existing list):

```rust
mod backend;

pub use backend::{Backend, ResourceSummary, SearchHit};
```

- [ ] **Step 2: Run the tests**

Run: `cargo nextest run -p temper-core operations::backend`

Expected: 1 test passes (`resource_summary_can_be_constructed`). Compilation success of `assert_object_safe` is the trait-object-safety check.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-core/src/operations/
git commit -m "feat(core): add Backend async trait with all command methods"
```

---

## Task 10: Verify the operations module via the workspace check

**Files:**
- No new files; this is a verification task.

- [ ] **Step 1: Run cargo make check**

Run: `cargo make check`

Expected: passes. Specifically:
- `cargo fmt --check` — passes (no formatting drift in newly added files)
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — passes (no lint failures)
- `cargo doc --workspace --no-deps` — passes (doc comments compile)
- `cargo machete` — passes (`async-trait` is used in `backend.rs`)

If clippy flags any issues in the new files, fix them before proceeding. Common items:
- Missing `#[must_use]` on builder-style functions — add if clippy asks.
- Doc comment punctuation — fix per clippy guidance.

- [ ] **Step 2: Run the operations test set**

Run: `cargo nextest run -p temper-core operations`

Expected: all operations:: tests pass (smoke + surface (2) + resource_ref (4) + inputs (3) + commands (4) + events (3) + output (2) + backend (1) = 19+ tests).

- [ ] **Step 3: Verify no behavior change**

Run: `cargo nextest run -p temper-core` (full crate test suite)

Expected: all existing tests still pass. Phase 1 is type-only; nothing pre-existing should break.

- [ ] **Step 4: Commit any small fixes**

If Step 1 surfaced fmt/clippy items, fix them and commit:

```bash
git add crates/temper-core/src/operations/
git commit -m "chore(core): fmt + clippy fixes in operations scaffolding"
```

If no fixes needed, skip the commit.

---

## Task 11: Final smoke — Phase 1 ready for Phase 2 hand-off

**Files:**
- No code changes; documentation polish + final verification.

- [ ] **Step 1: Verify the public surface**

Run: `cargo doc -p temper-core --no-deps --open` (omit `--open` in CI / if no browser).

Expected: `temper_core::operations` module documents exist. The page should list re-exports: `Surface`, `ResourceRef`, `BodyUpdate`, `ListFilter`, `SearchQuery`, `CreateResource`, `ShowResource`, `UpdateResource`, `DeleteResource`, `ListResources`, `SearchResources`, `DomainEvent`, `PushDeferReason`, `CommandOutput`, `Backend`, `ResourceSummary`, `SearchHit`.

- [ ] **Step 2: Verify the dep graph is unchanged**

Run: `grep -E "^temper-" crates/temper-cli/Cargo.toml crates/temper-api/Cargo.toml`

Expected:
- `temper-cli` deps: `temper-client`, `temper-core`, `temper-ingest`, `temper-llm` (unchanged — no dep on `temper-api`).
- `temper-api` deps: `temper-core`, `temper-ingest` (unchanged).

This is the architecture-rule check from `feedback_pre_propose_arch_review.md`: confirm Phase 1 didn't accidentally introduce a forbidden dep.

- [ ] **Step 3: Final test sweep**

Run: `cargo make test`

Expected: full unit test suite passes.

- [ ] **Step 4: Tag a milestone commit (optional but encouraged)**

```bash
git log --oneline -12
# Confirm the Phase 1 commits are present (Tasks 1-9 + maybe Task 10's fix commit).
```

No additional commit needed — Phase 1 is complete when this task's verifications pass.

---

## Phase 1 Completion Checklist

When all tasks above pass:

- [ ] `crates/temper-core/src/operations/` exists with 7 source files (`mod.rs`, `surface.rs`, `resource_ref.rs`, `inputs.rs`, `commands.rs`, `events.rs`, `output.rs`, `backend.rs`).
- [ ] `Surface` enum has 4 variants matching the spec.
- [ ] `ResourceRef` enum is sum-typed with `Uuid` and `Scoped` variants; constructors verified.
- [ ] All 6 commands (Create/Show/Update/Delete/List/Search) defined, with the resource-action commands using `ResourceRef`.
- [ ] `DomainEvent` enum has Db/Vault/composite variants.
- [ ] `Backend` async trait defined; object-safe.
- [ ] `cargo make check` passes.
- [ ] `cargo make test` passes.
- [ ] No new dep added to `temper-cli` or `temper-api` (architecture rule preserved).

**Hand-off to Phase 2 plan:** Phase 2 will move shared logic (`apply_doc_type_defaults`, validation helpers, merge logic) into `operations/actions.rs` and have CLI command code + API services call into the shared module rather than duplicating logic.
