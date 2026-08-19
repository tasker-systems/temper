# Shared Core-Code-Execution Paths — Design Spec

**Date:** 2026-05-01
**Context:** `temper`
**Mode:** build
**Effort:** large (multi-session; pairs with cloud-first-reframe spec)
**Branch:** `jct/wave1-shared-execution-paths-and-cloud-first-reframe`

**Related work:**
- Path-to-alpha goal item #4
- Companion spec: `2026-05-01-cloud-first-reframe-and-manifest-redefinition-design.md` (#3 — manifest fate, state machines, conflict semantics). The two specs are halves of the Wave 1 cloud-first reframe; #4 sets the architectural shape, #3 sets the conceptual model and lifecycle semantics that ride on it.

---

## Problem

Today three execution paths cover what is logically one set of operations on resources:

1. **CLI-local-vault**: command code in `crates/temper-cli/src/commands/*.rs` reaches into `temper-core` for vault file I/O and frontmatter, but the orchestration (validate, apply doctype defaults, write file, update manifest, optionally push to API) lives in CLI-side code.
2. **CLI-cloud** (`TEMPER_VAULT_STATE=cloud`): the same command modules branch on `VaultState::Cloud` and call out via `temper-client` to hit the API.
3. **MCP and API HTTP**: both reach the service layer at `crates/temper-api/src/services/*.rs`. MCP delegates in-process via `temper_api::services::*`. API handlers do the same over HTTP.

Each new feature ends up implemented in two or three places. The drift this produces is observable:

- The phantom show-edit-cat idiom in `CLAUDE.md` (existed in docs, didn't exist in code) — a CLI-local update path that didn't accept body, while the docs described one.
- Silent `✓ Updated` despite dropped body content — a CLI-local path that returned success when the body update was a no-op.
- Cloud-first delete missing until PR #64, while the underlying server soft-delete had been present for weeks.
- Schema-required-defaults rule (CLAUDE.md) had to be written because different write paths populated different default fields.

The fix isn't "service layer owns operations" in the literal sense — that was the framing in the path-to-alpha goal text but it doesn't fit the dependency graph. `temper-cli` does not and should not depend on `temper-api`. The shared layer is `temper-core`. The fix is: **operations live in `temper-core`; both `temper-api/services` (DB-backed) and `temper-cli` local-mode paths (vault-file-backed) implement the operations against their respective persistence; surfaces (CLI, MCP, API HTTP) are thin adapters that turn user input into operations and operation output into surface-shaped responses.**

## The Reframe

Three layers, two backends, four surfaces. Distinct concerns get distinct types and distinct module homes.

### Layer 1 — Operations (`temper-core`)

Declarative. No I/O, no transport.

- **Commands** — `struct CreateResource { ... }`, `struct UpdateResource { ... }`, `struct DeleteResource { ... }`, etc. Pure data. A surface translates clap args / MCP tool params / HTTP body into a command.
- **Actions** — units of work composed into a command's execution chain. Some live in `temper-core` and run identically in any backend (e.g., `validate`, `applyDoctypeDefaults`, `mergeUpdate`). Some are backend-specific (`writeFile`, `updateManifest` are vault-only; `persistDb`, `regenerateChunks`, `triggerEmbedding` are db-only).
- **Events** — past-tense facts emitted by actions. Backend-qualified: `DbResourceUpdated`, `VaultFileWritten`, `VaultManifestUpdated`, `RemoteSynced`, `PushDeferred`. Feeds state machines (companion spec #3) and observability (#19).
- **Backend trait** — defines the operations every backend supports. Each command corresponds to a method; the trait abstracts persistence + side effects without dictating how either backend realizes them.
- **State machines** — resource lifecycle definitions. Detailed in companion spec #3.

### Layer 2 — Backends

Two peers, both implement Layer-1's `Backend` trait.

- **`DbBackend`** — Postgres persistence + chunking + embedding. Implementation lives in `temper-api` (Rust crate location is unchanged from today; the `services` module evolves into the trait impl). Server-side handlers are this backend's natural home.
- **`VaultBackend`** — local file + manifest persistence. Implementation lives in `temper-cli` (in a new `crates/temper-cli/src/vault_backend/` module that owns local-mode paths previously scattered across `commands/*.rs`). For commands that need to reach the server (push as tail action, sync-recovery), `VaultBackend` calls `DbBackend` *via* `temper-client` over HTTP — not via direct dependency.

The dep direction stays anchored on `temper-core`. Neither backend depends on the other; they share the operation contract via the trait in `temper-core`.

### Layer 3 — Surfaces

Four originating surfaces, modeled as `enum Surface`:

```rust
// crates/temper-core/src/operations/surface.rs
pub enum Surface {
    CliLocalVault,  // CLI binary, local vault present
    CliCloud,       // CLI binary, TEMPER_VAULT_STATE=cloud
    Mcp,            // MCP rmcp tools (in-process to temper-api)
    ApiHttp,        // Axum handlers receiving inbound HTTP
}
```

Surface is what the inbound side is — origination + capability profile. It feeds output formatting (clap text/json vs MCP tool-result vs HTTP JSON), error shaping, and telemetry tagging.

Surface-to-backend dispatch is an explicit function:

| Surface | Default backend | Notes |
|---|---|---|
| `CliLocalVault` | `VaultBackend` | with optional tail-action call into `DbBackend` via `temper-client` |
| `CliCloud` | `DbBackend` via HTTP | over `temper-client` → API → in-process `DbBackend` |
| `Mcp` | `DbBackend` in-process | rmcp server runs alongside API (Vercel function) |
| `ApiHttp` | `DbBackend` in-process | the API server itself |

The mapping is itself the cloud-first reframe: only one surface (`CliLocalVault`) routes anywhere other than `DbBackend`. Everywhere else the backend is the database.

## Commands and Actions

Resource-level commands are **coarse with optional fields**, not fine-grained per modification axis. `UpdateResource` is one command; whether it carries `body`, `managed_meta`, `open_meta`, or any combination is a property of the input, not a separate command.

```rust
// crates/temper-core/src/operations/commands.rs (sketch)
pub struct UpdateResource {
    pub resource: ResourceRef,
    pub body: Option<BodyUpdate>,
    pub managed_meta: Option<ManagedMetaPartial>,
    pub open_meta: Option<OpenMetaPartial>,
    pub origin: Surface,
}
```

### Resource Identification — `ResourceRef`

Every resource-action command (`ShowResource`, `UpdateResource`, `DeleteResource`, plus sync-time variants like `SyncPushResource` and `SyncPullResource`) identifies its target through a `ResourceRef`, **not** a bare slug. Slug uniqueness is scoped to `(owner, context, doctype)`; UUID is globally unique. Both forms must be accepted everywhere a resource is named, so callers that hold a UUID (cross-context references, MCP tool handlers, agents that already resolved a resource earlier) don't have to round-trip through scoping fields.

```rust
// crates/temper-core/src/operations/commands.rs (sketch, continued)
pub enum ResourceRef {
    /// Globally-unique reference. Resolves directly without scoping fields.
    Uuid(ResourceUuid),

    /// Scoped reference. Slug + the fields needed to disambiguate it.
    Scoped {
        slug: String,
        doctype: DocType,
        context: ContextId,
    },
}
```

The enum shape (rather than a struct with two `Option` fields) makes the "exactly one form populated" invariant a compile-time guarantee, and forces the slug variant to carry its scoping fields explicitly.

`CreateResource` does not use `ResourceRef` — the resource doesn't exist yet, so it carries the future identity directly:

```rust
pub struct CreateResource {
    pub slug: String,
    pub doctype: DocType,
    pub context: ContextId,
    pub title: String,
    pub body: Option<BodyUpdate>,
    pub managed_meta: ManagedMeta,
    pub open_meta: OpenMeta,
    pub origin: Surface,
}
```

`ListResources` and `SearchResources` similarly do not use `ResourceRef` — they take filter and query inputs, not single-resource identifiers.

Resolve actions branch on the `ResourceRef` variant:

- `Uuid(_)` — `DbBackend.resolveDbRow` does a uuid lookup; `VaultBackend.resolveVaultFile` does a manifest reverse-index lookup (manifest stores `temper_id ↔ path` mapping).
- `Scoped { slug, doctype, context }` — `DbBackend` does a `(owner, context, doctype, slug)` SQL lookup; `VaultBackend` walks the doctype's vault directory.

Surfaces translate user input into the appropriate `ResourceRef` variant:

- CLI parses `--uuid <UUID>` into `ResourceRef::Uuid`; `<slug> --type <doctype> --context <context>` into `ResourceRef::Scoped`. (CLI accepts either form everywhere it accepts a positional slug today.)
- MCP tool params accept both `resource_uuid` and a (`slug`, `doctype`, `context`) triple; the tool handler picks one variant.
- API handlers accept both `?uuid=<...>` and `?slug=<...>&doctype=<...>&context=<...>` query/path patterns.

### Action chain by surface

The action chain for `UpdateResource` differs by surface:

| Step | CliLocalVault | CliCloud | Mcp | ApiHttp |
|---|---|---|---|---|
| authn | local token (if pushing) | TEMPER_TOKEN | jwt | jwt |
| authz | server enforces on push | server | profile-scoped | profile-scoped |
| validate | ✓ shared | ✓ shared | ✓ shared | ✓ shared |
| resolve | resolveVaultFile | (none, sent to server) | resolveDbRow | resolveDbRow |
| applyDefaults | ✓ shared | ✓ shared | ✓ shared | ✓ shared |
| mergeUpdate | ✓ shared | ✓ shared | ✓ shared | ✓ shared |
| persist | writeFile + updateManifest | (server persists) | persistDb | persistDb |
| body side-effects | (none locally) | (server) | regenerateChunks + triggerEmbedding | regenerateChunks + triggerEmbedding |
| push | pushToApi → DbBackend.UpdateResource | n/a (already at API) | n/a | n/a |
| events | VaultFileWritten + VaultManifestUpdated + RemoteSynced (push ok) / PushDeferred (push fail) | server emits | DbResourceUpdated + body-triggered events | DbResourceUpdated + body-triggered events |
| surface output | clap text/json | clap text/json | MCP tool result | HTTP JSON |

Read for `CliLocalVault`: the row labeled "push" is the action that invokes `DbBackend.UpdateResource` via `temper-client`. The Db chain runs end-to-end on the server. From the CLI's perspective, push is one action in its chain that emits a composite event. Two backend chains, one logical operation.

The companion spec #3 defines what happens when push fails (graceful fallback, `PendingPush` state, manifest as recovery artifact).

## Backend Trait

```rust
// crates/temper-core/src/operations/backend.rs (sketch)
#[async_trait]
pub trait Backend {
    async fn create_resource(&self, cmd: CreateResource) -> Result<CommandOutput<ResourceRecord>>;
    async fn show_resource(&self, cmd: ShowResource) -> Result<CommandOutput<ResourceRecord>>;
    async fn update_resource(&self, cmd: UpdateResource) -> Result<CommandOutput<ResourceRecord>>;
    async fn delete_resource(&self, cmd: DeleteResource) -> Result<CommandOutput<()>>;
    async fn list_resources(&self, cmd: ListResources) -> Result<CommandOutput<Vec<ResourceSummary>>>;
    async fn search_resources(&self, cmd: SearchResources) -> Result<CommandOutput<Vec<SearchHit>>>;
}

pub struct CommandOutput<T> {
    pub value: T,
    pub events: Vec<DomainEvent>,
}
```

`VaultBackend` adds vault-specific operations (manifest refresh, sync push/pull) outside the shared trait — they are not cross-backend operations.

```rust
// crates/temper-cli/src/vault_backend/mod.rs (sketch)
impl VaultBackend {
    pub async fn refresh_manifest(&self, cmd: RefreshManifest) -> Result<CommandOutput<ManifestSnapshot>>;
    pub async fn sync_push(&self, cmd: SyncPushResource) -> Result<CommandOutput<SyncReport>>;
    pub async fn sync_pull(&self, cmd: SyncPullResource) -> Result<CommandOutput<SyncReport>>;
}
```

Async trait choice: `async_trait` macro is acceptable for now; if we hit object-safety issues we can switch to generic dispatch (each surface knows its concrete backend type at compile time, so dynamic dispatch is not strictly required).

## Module Placement in `temper-core`

```
crates/temper-core/src/
├── operations/
│   ├── mod.rs
│   ├── commands.rs        // Command structs (CreateResource, UpdateResource, ...)
│   ├── events.rs          // DomainEvent enum + variants
│   ├── actions.rs         // Shared pure actions (validate, applyDefaults, mergeUpdate)
│   ├── backend.rs         // Backend trait
│   ├── surface.rs         // Surface enum
│   └── state.rs           // State machine definitions (detail in companion spec #3)
├── types/                 // existing — domain models stay here
├── vault/                 // existing — file-system primitives (used by VaultBackend impl)
└── ...
```

The existing `temper-core/types/` and `temper-core/vault/` modules stay where they are. `operations/` is a new sibling that depends on `types/` for domain models and on `vault/` for vault primitives that pure actions need (e.g., frontmatter parsing).

## Implementation Phases

This is a large refactor. Sequence:

**Phase 1 — Scaffolding.** Add `temper-core/operations/` with empty modules and the `Backend` trait. Define `Surface` enum. Define `Command` structs (no implementations yet). No behavior change.

**Phase 2 — Pure shared actions.** Migrate `validate`, `applyDoctypeDefaults`, `mergeUpdate` from wherever they live today (scattered across `temper-core/types/schemas/`, `temper-cli/src/actions/`, `temper-api/src/services/`) into `temper-core/operations/actions.rs`. Both existing call sites (CLI command code + API services) call into the shared module.

**Phase 3 — `DbBackend` impl.** Make `temper-api/services/resource_service.rs` (and its peers) implement `Backend`. API handlers call through the trait instead of through ad-hoc service functions. MCP tools migrate to call the trait too. Behavior unchanged; structure tightened.

**Phase 4 — `VaultBackend` impl.** Extract local-mode logic from `temper-cli/src/commands/resource.rs` (currently 2125 lines) into `crates/temper-cli/src/vault_backend/`. Each command method delegates to the new impl. CLI command modules become thin clap-to-command translators.

**Phase 5 — Surface dispatch unification.** `commands/resource.rs` collapses the `match VaultState` branches into a single `Surface::dispatch` call that routes to `VaultBackend` or invokes `temper-client` (which hits API → `DbBackend`).

**Phase 6 — Companion spec #3 hookup.** Wire state machines, events, and the manifest-narrowing changes (companion spec details).

Each phase is independently shippable. Phases 3 and 4 are the largest and could each be split further during plan-writing.

## Acceptance Criteria

- [ ] `temper-core/operations/` exists with `Backend` trait, `Surface` enum, command structs, event enum, and shared actions.
- [ ] `ResourceRef` is the identifier type for every resource-action command. Both `Uuid` and `Scoped { slug, doctype, context }` variants accepted everywhere. CLI accepts `--uuid` alongside positional slug; MCP tool params accept either form; API handlers accept either form. No call site requires a slug where a uuid would also work.
- [ ] All shared validation / default / merge logic lives in `temper-core/operations/actions.rs`. No duplicate copies in `temper-cli` or `temper-api`.
- [ ] `temper-api/services/` implements `Backend`. API handlers and MCP tools both dispatch through the trait.
- [ ] `temper-cli/src/vault_backend/` implements `Backend` for vault-file persistence.
- [ ] `temper-cli/src/commands/resource.rs` is reduced to surface adapters: clap → command → `Surface::dispatch` → output formatting.
- [ ] `temper-cli` continues to depend on `temper-core` and `temper-client` only (no dep on `temper-api`).
- [ ] All existing `cargo make test`, `test-db`, `test-e2e` suites pass.
- [ ] New trait-level tests cover backend contract: each `Backend` impl satisfies the same observable behavior for the same command (where backend-specific divergence doesn't apply).
- [ ] `CLAUDE.md` updated: "service layer owns operations" rule rewritten as "operations layer (`temper-core/operations/`) defines commands; backends implement; surfaces adapt."

## Out of Scope

- Manifest semantics, state machines, conflict resolution. Owned by companion spec #3.
- New commands (the command set is the existing CRUD surface — no new verbs added under #4).
- Authn/authz refactor. Existing auth plumbing stays where it is; backends consume it as a parameter.
- Surface output formatting refactor (clap output, MCP tool-result shaping). Each surface keeps its current output style; the change is the dispatch path, not the formatting.
- `temper-mcp` extracting from `temper-api`. Today MCP depends on `temper-api`. After this work it depends on `temper-api` for the `Backend` impl. Splitting `temper-services` out of `temper-api` is a future architectural unit, not part of this scope.

## Open Questions

- **Backend trait async dispatch shape.** `async_trait` works fine for now; if ergonomics or perf push us elsewhere, the choice is generic (compile-time) dispatch. Not blocking.
- **Event emission mechanism.** Sketch returns `Vec<DomainEvent>` from each command. An alternative is an injected `EventSink` (callback / channel). Decide during Phase 1 implementation; either is workable.
- **Surface enum scope.** Web UI is not currently a Layer-3 surface in this enum because it goes through the API HTTP surface (it's a CliCloud-equivalent client). If we ever add direct web-UI-to-backend paths, a fifth surface variant gets added. Out of scope for alpha.
