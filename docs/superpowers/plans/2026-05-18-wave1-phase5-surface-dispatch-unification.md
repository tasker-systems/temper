# Wave 1 Phase 5 — Surface Dispatch Unification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Project-specific execution variant (Variant B — see `project_hybrid_execution_skill_idea` in user memory):**
> - Implementer subagents run ONLY the tests they wrote in their task + `cargo make check`. Not the full crate suite. Not workspace nextest.
> - Implementer subagents do NOT invoke a chained reviewer subagent. The **orchestrator** (Claude, holding plan + task context) reviews each commit before dispatching the next task. For high-stakes diffs, the orchestrator may dispatch a *targeted* reviewer subagent with explicit framing.
> - Pete reviews at the PR level, NOT per-task. The orchestrator signals Pete only when a new decision arises that wasn't covered by the spec/plan.
> - Full crate suite, workspace tests, and e2e tests run once at end-of-branch (PR-prep, Task 19). Not per-task.
> - Final opus code review at end-of-branch as usual.

**Spec:** `docs/superpowers/specs/2026-05-18-wave1-phase5-surface-dispatch-unification-design.md`

**Predecessor (merged):** PR #80 — Wave 1 Phase 4 completion (VaultBackend writes, per-doctype dispatch, helper deletions).

**Goal:** Collapse every `match VaultState` write-path branch into a single `Box<dyn Backend>` dispatcher backed by VaultBackend (local) or new CloudBackend (cloud), and route currently-local-only subcommands (`task done`, `task move-to`, `goal create/update`, `research save/finish`) through the same dispatcher.

**Architecture:** New `crates/temper-cli/src/cloud_backend/` module mirrors `vault_backend/` shape: translators (pure cmd→wire functions, unit-tested) + `CloudBackend` struct implementing `temper_core::operations::Backend` by translating commands into `temper-client` calls. A `backend_select::build_backend(config, ctx) -> (Runtime, Box<dyn Backend>)` helper centralizes mode selection. Surfaces become: clap-args → build cmd → `build_backend()` → `backend.<method>(cmd).await` → render.

**Tech Stack:** Rust workspace (temper-core, temper-cli, temper-client, temper-ingest), tokio, async-trait, sqlx (transitive via temper-api in e2e), cargo-make, cargo-nextest.

**Branch:** `jct/wave1-phase5-surface-dispatch-unification`

**Reference docs:**
- Spec: `docs/superpowers/specs/2026-05-18-wave1-phase5-surface-dispatch-unification-design.md`
- Sibling spec (Phase 4): `docs/superpowers/specs/2026-05-11-wave1-phase4-vaultbackend-design.md`
- Sibling plan (Phase 4 completion): `docs/superpowers/plans/2026-05-13-wave1-phase4-completion.md` and `2026-05-14-wave1-phase4-completion-b5b-c1-c2.md` (study these for execution shape)

---

## File Structure

**Files created:**
- `crates/temper-cli/src/cloud_backend/mod.rs` — module declarations + re-exports
- `crates/temper-cli/src/cloud_backend/ctx.rs` — `CloudBackendCtx` struct + `assemble_cloud_backend` constructor
- `crates/temper-cli/src/cloud_backend/cloud_backend.rs` — `CloudBackend` struct + `impl Backend for CloudBackend`
- `crates/temper-cli/src/cloud_backend/translators.rs` — pure cmd→wire functions (unit-tested)
- `crates/temper-cli/src/backend_select.rs` — `build_backend` helper (Vault/Cloud dispatcher constructor)
- `crates/temper-cli/src/output_helpers.rs` (or add to existing `actions::output`) — `render_write_success` helper

**Files modified:**
- `crates/temper-cli/src/lib.rs` — declare new modules (`cloud_backend`, `backend_select`)
- `crates/temper-cli/src/commands/resource.rs` — collapse 3 `match VaultState` arms (create, update, delete) into `build_backend` calls; delete `cloud_mode_update` + `delete_cloud` + cloud-mode arm of create
- `crates/temper-cli/src/commands/session.rs` — collapse `match VaultState` in `save`; route through `build_backend`
- `crates/temper-cli/src/commands/task.rs` — wire `done`, `move-to` through `build_backend` (cloud-mode capable)
- `crates/temper-cli/src/commands/goal.rs` — wire `create`, `update` through `build_backend`
- `crates/temper-cli/src/commands/research.rs` — wire `save`, `finish` through `build_backend` if applicable (audit at Task 17)

**Files potentially deleted (clippy-driven, end of 5b):**
- Whatever clippy flags as dead after the cloud-mode helper inlining. Likely: `cloud_mode_update`, `delete_cloud`. Verify at Task 13.

---

## Conventions for every task (inherited from 4a/4b plans)

- **TDD.** Write the failing test first. Confirm it fails for the right reason. Implement minimally. Confirm it passes.
- **Verify named APIs before coding.** Every API name in this plan is a hypothesis as of 2026-05-18 — `git log` may have moved code. Grep at task start before relying on a signature.
- **`cargo make check` before claiming complete.** Pre-commit hook is the backstop; subagents must run it themselves first.
- **Per-task test runs only:** tests written in the task itself + the surrounding test module's filter. Not the full crate suite. Not `--workspace`. That happens at Task 19.
- **No `#[allow(...)]` for clippy.** Use `#[expect(name, reason = "...")]` or fix the underlying issue.
- **No `cargo nextest --workspace` per task.** Final workspace + e2e run is Task 19's job.
- **Escalate, don't soften.** If a test requires loosening a contract, STOP and report BLOCKED.
- **No "for now" workarounds.** Capture as a task; don't ship a TODO comment.
- **No premature backward-compat.** Surface dispatchers replace the cloud-mode helpers; clippy-driven deletion is mandatory, not optional.
- **Existing CLI integration tests + e2e tests pass unmodified** at Task 19. A test re-write to accommodate the migration is a smell — STOP and report.
- **Subagent task hand-off:** when the implementer reports the task complete, the orchestrator (Claude) reviews the commit before dispatching the next task. No chained spec-reviewer subagent. No chained code-reviewer subagent. For high-stakes diffs, dispatch a *targeted* reviewer subagent with explicit framing — never open-ended "review this." Signal Pete only when an unplanned-for decision arises.

---

# Phase 5a — CloudBackend Foundation

**Goal of Phase 5a.** Build `crates/temper-cli/src/cloud_backend/` with translators (unit-tested), `CloudBackend` struct, and `Backend` trait impl. Dark-launched — no surface callers yet. The existing `cloud_mode_update`, `delete_cloud`, and cloud-mode-create arm in `commands/resource.rs` stay untouched until 5b.

### Task 1 — Scaffold `cloud_backend/` module + `CloudBackendCtx`

**Files:**
- Create: `crates/temper-cli/src/cloud_backend/mod.rs`
- Create: `crates/temper-cli/src/cloud_backend/ctx.rs`
- Modify: `crates/temper-cli/src/lib.rs`

**Goal.** Lay down the module skeleton + `CloudBackendCtx` struct that holds the per-request fields needed for cloud-mode dispatch. Mirror `VaultBackendCtx`'s shape from `vault_backend/vault_backend.rs:53-60`.

**Verification of API names** (grep at task start):
- `grep -n "pub struct VaultBackendCtx\|pub fn assemble_vault_backend" crates/temper-cli/src/vault_backend/`
- `grep -n "pub struct TemperClient\|pub fn resources\|pub fn ingest" crates/temper-client/src/lib.rs`
- `grep -n "pub fn from_env\|VaultState::Cloud" crates/temper-core/src/types/config.rs`

- [ ] **Step 1: Create `crates/temper-cli/src/cloud_backend/ctx.rs`**

```rust
//! Per-request context for `CloudBackend` — holds the resolved client,
//! owner, surface, and config needed to translate `temper-core::operations`
//! commands into HTTP calls.
//!
//! Mirror of `vault_backend/vault_backend.rs::VaultBackendCtx`. The two ctx
//! structs deliberately do NOT share a parent — vault-only fields
//! (`vault_root`, `manifest`) don't belong in cloud-mode; cloud-only fields
//! (if added later) won't apply to vault.

use std::sync::Arc;

use temper_client::TemperClient;
use temper_core::operations::Surface;

use crate::config::Config;
use crate::error::{Result, TemperError};

/// Builder / context for constructing a `CloudBackend`.
///
/// Holds the per-request fields needed for cloud-mode dispatch:
/// - `client`: required (cloud mode has no offline path; if there's no token,
///   `assemble_cloud_backend` errors at construction).
/// - `owner`: derived from `Config::owner_for_context(context)`.
/// - `surface`: always `Surface::CliCloud` for inbound CLI calls today.
/// - `config`: kept for future use (e.g., per-request profile resolution).
pub struct CloudBackendCtx {
    pub client: Arc<TemperClient>,
    pub owner: String,
    pub config: Arc<Config>,
    pub surface: Surface,
}

/// Build a fully-populated `CloudBackendCtx` for a Cloud-mode CLI invocation.
///
/// **Auth-required.** Unlike `assemble_vault_backend` (which tolerates a
/// missing token by leaving `client = None`), cloud mode has no offline
/// path. If the resolved token store is empty, this returns an error
/// directing the user to `temper auth login`.
pub fn assemble_cloud_backend(
    config: &Config,
    context: &str,
) -> Result<CloudBackendCtx> {
    let (_cfg, store, client) = crate::actions::runtime::build_config_store_and_client()?;
    let _token = store
        .load()
        .ok()
        .flatten()
        .ok_or_else(|| {
            TemperError::Project(
                "cloud mode requires authentication — run `temper auth login`".to_string(),
            )
        })?;

    let owner = config.owner_for_context(context);

    Ok(CloudBackendCtx {
        client: Arc::new(client),
        owner,
        config: Arc::new(config.clone()),
        surface: Surface::CliCloud,
    })
}
```

NOTE: `Surface::CliCloud` may not exist as an enum variant. Grep `enum Surface` in `temper-core/src/operations/commands.rs`. If only `CliLocalVault` exists today, either (a) add `CliCloud` to the enum in this same task as a one-line additive change, or (b) use the closest existing variant. Default to (a) — additive enum variants are safe.

- [ ] **Step 2: Create `crates/temper-cli/src/cloud_backend/mod.rs`**

```rust
//! `CloudBackend` — cloud-mode impl of [`temper_core::operations::Backend`].
//!
//! Per-request construction: CLI surfaces in Cloud mode build a
//! `CloudBackend` via `assemble_cloud_backend`, then dispatch one command
//! through it. Each trait method translates the inbound
//! `temper-core::operations` command into a `temper-client` call and
//! projects the wire response back into `CommandOutput<ResourceRow>`.
//!
//! Mirror of `vault_backend/`. Translators are pure cmd→wire functions
//! (unit-tested); dispatch is exercised end-to-end via `tests/e2e/`.
//!
//! See `docs/superpowers/specs/2026-05-18-wave1-phase5-surface-dispatch-unification-design.md`.

mod cloud_backend;
mod ctx;
mod translators;

#[cfg(test)]
mod tests;

pub use cloud_backend::CloudBackend;
pub use ctx::{assemble_cloud_backend, CloudBackendCtx};
```

The `cloud_backend.rs`, `translators.rs`, and `tests.rs` files don't exist yet — Step 4 makes the module compile with stubs; later tasks fill them in.

- [ ] **Step 3: Register the module in `lib.rs`**

Add `pub mod cloud_backend;` to `crates/temper-cli/src/lib.rs` next to the existing `pub mod vault_backend;` declaration. (Grep for `pub mod vault_backend` to find the spot.)

- [ ] **Step 4: Create stub files so the module compiles**

`crates/temper-cli/src/cloud_backend/cloud_backend.rs`:
```rust
//! Stub — filled in by Task 5.
//!
//! `CloudBackend` struct + `impl Backend for CloudBackend` land here.

use std::sync::Arc;

use temper_client::TemperClient;
use temper_core::operations::Surface;

use crate::config::Config;
use super::ctx::CloudBackendCtx;

#[allow(dead_code)] // wired in Task 5
pub struct CloudBackend {
    pub(crate) client: Arc<TemperClient>,
    pub(crate) owner: String,
    pub(crate) config: Arc<Config>,
    #[allow(dead_code)] // stored for forward-compat
    pub(crate) surface: Surface,
}

impl CloudBackend {
    #[allow(dead_code)] // wired in Task 5
    pub fn new(ctx: CloudBackendCtx) -> Self {
        Self {
            client: ctx.client,
            owner: ctx.owner,
            config: ctx.config,
            surface: ctx.surface,
        }
    }
}
```

`crates/temper-cli/src/cloud_backend/translators.rs`:
```rust
//! Stub — pure cmd→wire translation functions land here in Tasks 2-4.
```

`crates/temper-cli/src/cloud_backend/tests.rs`:
```rust
//! Stub — integration tests for the dispatch path land here if needed;
//! pure-function unit tests live next to their functions in translators.rs.
```

- [ ] **Step 5: Write the smoke test** — `assemble_cloud_backend` errors when no token resolves

Append to `crates/temper-cli/src/cloud_backend/ctx.rs` test module (create if absent):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::types::config::VaultState;

    #[test]
    fn assemble_cloud_backend_errors_when_no_token() {
        // Construct a Config with no auth.json path resolvable; assemble should error.
        let temp = tempfile::tempdir().unwrap();
        let config = Config {
            vault_root: temp.path().to_path_buf(),
            state_dir: temp.path().to_path_buf(),
            ..Config::default()
        };
        // Force no token: set TEMPER_TOKEN to empty string AND make state_dir unreadable.
        std::env::remove_var("TEMPER_TOKEN");
        let err = assemble_cloud_backend(&config, "temper").unwrap_err();
        assert!(
            format!("{err:?}").contains("temper auth login"),
            "expected auth-login error, got: {err:?}"
        );
    }
}
```

If `Config::default()` is not in scope or the test harness already has a richer helper (e.g. `Config::for_test()`), use it. Grep `pub fn default\|fn for_test\|fn test_config` under `crates/temper-cli/src/config.rs`.

- [ ] **Step 6: Run test to verify it fails**

Run: `cargo nextest run -p temper-cli cloud_backend::ctx::tests::assemble_cloud_backend_errors_when_no_token`

Expected: FAIL — module doesn't compile yet OR the assemble function doesn't exist yet. After fixing compile errors from Step 1, the test should PASS (the no-token error is the function's documented behavior).

- [ ] **Step 7: Run `cargo make check`**

Run: `cargo make check`

Expected: Clean. The stubs use `#[allow(dead_code)]` to suppress warnings until later tasks wire them.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-cli/src/lib.rs crates/temper-cli/src/cloud_backend/
git commit -m "$(cat <<'EOF'
phase5a-1: scaffold cloud_backend/ module + CloudBackendCtx

Lays down the module skeleton mirroring vault_backend/. Introduces
CloudBackendCtx + assemble_cloud_backend. Cloud mode has no offline
path: assemble_cloud_backend errors when no token resolves
(unlike VaultBackendCtx's offline tolerance).

Adds Surface::CliCloud enum variant in temper-core/operations/commands.rs
(additive).

Stubs for CloudBackend struct + translators land in subsequent tasks.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2 — Translator: `cmd_to_ingest_payload`

**Files:**
- Modify: `crates/temper-cli/src/cloud_backend/translators.rs`

**Goal.** Pure function: `CreateResource` cmd → `IngestPayload` wire type. This is the cloud-mode equivalent of `vault_backend/translators.rs`'s `cmd_to_*` functions.

**Verification of API names** (grep at task start):
- `grep -n "pub struct IngestPayload\|pub struct CreateResource\|pub struct BodyUpdate\|pub struct ManagedMeta" crates/temper-core/src/`
- Read `crates/temper-cli/src/actions/ingest.rs::build_ingest_payload` (the existing analog) for the body-trio computation pattern.

- [ ] **Step 1: Read existing `build_ingest_payload` to understand the pattern**

Run: `grep -n "fn build_ingest_payload\|compute_body_chunks" crates/temper-cli/src/actions/ingest.rs`

Record the signature, what fields it sets, how it handles body-trio computation. The translator below reuses this pattern but reframes it as cmd-based rather than args-based.

- [ ] **Step 2: Write the failing test for the basic round-trip**

Append to `crates/temper-cli/src/cloud_backend/translators.rs`:

```rust
//! Pure cmd → wire translation functions for `CloudBackend`.
//!
//! Each function takes a `temper-core::operations` command struct and
//! produces the wire payload that `temper-client` accepts. Translators
//! are pure — they don't perform I/O or async work. The async dispatch
//! lives in `cloud_backend.rs::impl Backend`.
//!
//! Mirror of `vault_backend/translators.rs` — same pattern, different
//! target type set.

#[cfg(feature = "embed")]
use temper_core::operations::CreateResource;
#[cfg(feature = "embed")]
use temper_core::types::ingest::IngestPayload;
#[cfg(feature = "embed")]
use crate::error::{Result, TemperError};

/// Translate a `CreateResource` command into an `IngestPayload` wire
/// payload suitable for `POST /api/ingest`.
///
/// **Body-trio computation.** If `cmd.body` is present, runs the same
/// `compute_body_chunks` pipeline that the existing `cloud_mode_create`
/// arm uses (see `commands/resource.rs:226-234`). If `cmd.body` is
/// absent, sends a placeholder `# {title}\n` body to match today's
/// cloud-create behavior.
///
/// **`origin_uri`** is currently None — server constructs the canonical
/// URI from `(owner, context, doctype, slug)`.
#[cfg(feature = "embed")]
pub(crate) fn cmd_to_ingest_payload(
    cmd: &CreateResource,
) -> Result<IngestPayload> {
    // Step 1: resolve body content.
    let content = match &cmd.body {
        Some(b) if !b.content.is_empty() => b.content.clone(),
        _ => format!("# {}\n", cmd.title),
    };

    // Step 2: compute body-trio (content_hash + chunks_packed).
    let chunks = crate::actions::ingest::compute_body_chunks(&content)?;

    // Step 3: serialize managed_meta + open_meta to JSON (IngestPayload uses Value, not the struct).
    let managed_meta = if cmd.managed_meta == temper_core::types::ManagedMeta::default() {
        None
    } else {
        Some(serde_json::to_value(&cmd.managed_meta).map_err(|e| {
            TemperError::Project(format!("serialize managed_meta: {e}"))
        })?)
    };

    let open_meta = cmd
        .open_meta
        .as_ref()
        .map(|m| serde_json::to_value(m))
        .transpose()
        .map_err(|e| TemperError::Project(format!("serialize open_meta: {e}")))?;

    Ok(IngestPayload {
        title: cmd.title.clone(),
        origin_uri: String::new(), // server constructs canonical URI
        context_name: cmd.context.clone(),
        doc_type_name: cmd.doctype.clone(),
        content_hash: Some(chunks.content_hash),
        slug: cmd.slug.clone(),
        content,
        metadata: None,
        managed_meta,
        open_meta,
        chunks_packed: Some(chunks.chunks_packed),
    })
}

#[cfg(feature = "embed")]
#[cfg(test)]
mod tests {
    use super::*;
    use temper_core::operations::{BodyUpdate, CreateResource, Surface};
    use temper_core::types::ManagedMeta;

    fn sample_cmd() -> CreateResource {
        CreateResource {
            slug: "2026-05-18-test".to_string(),
            doctype: "task".to_string(),
            context: "temper".to_string(),
            title: "Test task".to_string(),
            body: Some(BodyUpdate {
                content: "# Test\n\nBody.\n".to_string(),
                content_hash: None,
                chunks_packed: None,
            }),
            managed_meta: ManagedMeta {
                mode: Some("plan".to_string()),
                effort: Some("small".to_string()),
                goal: Some("temper-maintenance".to_string()),
                ..ManagedMeta::default()
            },
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            origin: Surface::CliCloud,
        }
    }

    #[test]
    fn cmd_to_ingest_payload_round_trips_basic_fields() {
        let cmd = sample_cmd();
        let payload = cmd_to_ingest_payload(&cmd).expect("should succeed");
        assert_eq!(payload.slug, "2026-05-18-test");
        assert_eq!(payload.title, "Test task");
        assert_eq!(payload.context_name, "temper");
        assert_eq!(payload.doc_type_name, "task");
        assert_eq!(payload.content, "# Test\n\nBody.\n");
        assert!(payload.chunks_packed.is_some());
        assert!(payload.content_hash.is_some());
    }
}
```

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo nextest run -p temper-cli --features embed cloud_backend::translators::tests::cmd_to_ingest_payload_round_trips_basic_fields`

Expected: FAIL — function doesn't exist OR signature mismatch.

- [ ] **Step 4: Confirm test passes after implementation**

Step 2's code block contains the implementation. Re-run:

Run: `cargo nextest run -p temper-cli --features embed cloud_backend::translators::tests::cmd_to_ingest_payload_round_trips_basic_fields`

Expected: PASS.

- [ ] **Step 5: Add coverage for managed_meta serialization and empty body**

Append two more tests:

```rust
    #[test]
    fn cmd_to_ingest_payload_serializes_managed_meta_to_json() {
        let cmd = sample_cmd();
        let payload = cmd_to_ingest_payload(&cmd).expect("should succeed");
        let mm = payload.managed_meta.expect("managed_meta should be present");
        assert_eq!(mm["mode"], "plan");
        assert_eq!(mm["effort"], "small");
        assert_eq!(mm["goal"], "temper-maintenance");
    }

    #[test]
    fn cmd_to_ingest_payload_synthesizes_body_when_absent() {
        let mut cmd = sample_cmd();
        cmd.body = None;
        let payload = cmd_to_ingest_payload(&cmd).expect("should succeed");
        assert_eq!(payload.content, "# Test task\n", "placeholder body uses title");
    }

    #[test]
    fn cmd_to_ingest_payload_skips_managed_meta_when_default() {
        let mut cmd = sample_cmd();
        cmd.managed_meta = ManagedMeta::default();
        let payload = cmd_to_ingest_payload(&cmd).expect("should succeed");
        assert!(payload.managed_meta.is_none(), "default managed_meta omitted from wire");
    }
```

Run: `cargo nextest run -p temper-cli --features embed cloud_backend::translators::tests`

Expected: All 4 PASS.

- [ ] **Step 6: Run `cargo make check`**

Run: `cargo make check`

Expected: Clean.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/cloud_backend/translators.rs
git commit -m "$(cat <<'EOF'
phase5a-2: translator cmd_to_ingest_payload

Pure function translating CreateResource cmd into IngestPayload wire
payload. Mirrors the existing cloud_mode_create arm's behavior
(body fallback to `# {title}\n`, body-trio computation via
compute_body_chunks, managed_meta/open_meta serialization).

Will replace the inline IngestPayload construction in
commands/resource.rs::create's cloud arm when 5b lands.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 3 — Translator: `cmd_to_resource_update_request`

**Files:**
- Modify: `crates/temper-cli/src/cloud_backend/translators.rs`

**Goal.** Pure function: `UpdateResource` cmd → `ResourceUpdateRequest`. Handles partial-merge semantics (only set fields are sent), body-trio computation, and the move_to → managed_meta synthesis from PR #79.

**Verification of API names** (grep at task start):
- `grep -n "pub struct ResourceUpdateRequest\|pub struct UpdateResource\|pub struct MoveSpec\|pub struct BodyUpdate" crates/temper-core/src/`
- Read `crates/temper-cli/src/commands/resource.rs::cloud_mode_update` lines 1491-1575 to mirror its translation logic.
- Read `crates/temper-cli/src/vault_backend/translators.rs` for the `move_to → managed_meta` synthesis test pattern from PR #79; port it.

- [ ] **Step 1: Read `cloud_mode_update` to record the translation shape**

Read `crates/temper-cli/src/commands/resource.rs:1491-1575`.

Record:
- Body resolution: `resolve_body_source` → `compute_body_chunks` (content_hash + chunks_packed)
- `managed_meta` from `build_partial_managed_meta_from_args`
- `open_meta` from `build_partial_open_meta_from_args`
- `title` from `params.title.map(String::from)`
- `slug: None` (slug-rename not exposed in this API today)

The translator reframes this as cmd-input, not params-input. The body computation logic stays identical.

- [ ] **Step 2: Write failing test for basic partial-merge round-trip**

Append to `cloud_backend/translators.rs`:

```rust
/// Translate an `UpdateResource` command into a `ResourceUpdateRequest`
/// wire payload suitable for `PATCH /api/resources/{id}`.
///
/// **Partial-merge semantics:** only fields present in the cmd are
/// serialized on the wire. The server applies the partial-merge per
/// `resource_service::update`'s contract.
///
/// **Move-to → managed_meta synthesis:** when the cmd carries
/// `move_to: Some(MoveSpec { context_to, type_to })` but no
/// `managed_meta.context` / `managed_meta.doc_type`, synthesizes
/// minimal managed_meta entries so the server-side row reflects the
/// move. Mirror of `vault_backend/translators.rs`'s synthesize_move_to
/// helper (PR #79).
///
/// Body-trio computation runs in this function only when `cmd.body` is
/// `Some`; absent body means "no body update requested" and content_hash
/// / chunks_packed stay `None`.
#[cfg(feature = "embed")]
pub(crate) fn cmd_to_resource_update_request(
    cmd: &temper_core::operations::UpdateResource,
) -> Result<temper_core::types::ResourceUpdateRequest> {
    use temper_core::types::ManagedMeta;

    // Body-trio computation (only when body present).
    let (content, content_hash, chunks_packed) = match &cmd.body {
        Some(b) => {
            let chunks = crate::actions::ingest::compute_body_chunks(&b.content)?;
            (Some(b.content.clone()), Some(chunks.content_hash), Some(chunks.chunks_packed))
        }
        None => (None, None, None),
    };

    // Move_to → managed_meta synthesis: if move_to is set but managed_meta
    // doesn't carry context/doc_type, fill them from move_to.
    let mut managed_meta = cmd.managed_meta.clone().unwrap_or_default();
    if let Some(move_to) = &cmd.move_to {
        if let Some(ctx_to) = &move_to.context_to {
            if managed_meta.context.is_none() {
                managed_meta.context = Some(ctx_to.clone());
            }
        }
        if let Some(type_to) = &move_to.type_to {
            if managed_meta.doc_type.is_none() {
                managed_meta.doc_type = Some(type_to.clone());
            }
        }
    }

    let managed_meta_wire = if managed_meta == ManagedMeta::default() {
        None
    } else {
        Some(serde_json::to_value(&managed_meta).map_err(|e| {
            TemperError::Project(format!("serialize managed_meta: {e}"))
        })?)
    };

    let open_meta_wire = cmd
        .open_meta
        .as_ref()
        .map(|m| serde_json::to_value(m))
        .transpose()
        .map_err(|e| TemperError::Project(format!("serialize open_meta: {e}")))?;

    // title field: ResourceUpdateRequest carries a Title field today (see
    // resource.rs:1523-1531); cmd carries it as a managed_meta key. If
    // managed_meta.title is set, lift it to the request's title field too
    // for symmetry with today's cloud_mode_update path.
    let title = managed_meta.title.clone();

    Ok(temper_core::types::ResourceUpdateRequest {
        title,
        slug: None,
        managed_meta: managed_meta_wire,
        open_meta: open_meta_wire,
        content,
        content_hash,
        chunks_packed,
    })
}

#[cfg(feature = "embed")]
#[cfg(test)]
mod update_translator_tests {
    use super::*;
    use temper_core::operations::{BodyUpdate, MoveSpec, ResourceRef, Surface, UpdateResource};
    use temper_core::types::ManagedMeta;

    fn sample_update() -> UpdateResource {
        UpdateResource {
            resource: ResourceRef::scoped("@me".to_string(), "temper", "task", "test-slug"),
            body: None,
            managed_meta: None,
            open_meta: None,
            move_to: None,
            origin: Surface::CliCloud,
        }
    }

    #[test]
    fn cmd_to_resource_update_request_omits_absent_fields() {
        let cmd = sample_update();
        let req = cmd_to_resource_update_request(&cmd).expect("should succeed");
        assert!(req.title.is_none());
        assert!(req.managed_meta.is_none());
        assert!(req.open_meta.is_none());
        assert!(req.content.is_none());
        assert!(req.content_hash.is_none());
        assert!(req.chunks_packed.is_none());
    }

    #[test]
    fn cmd_to_resource_update_request_synthesizes_managed_meta_from_move_to() {
        let mut cmd = sample_update();
        cmd.move_to = Some(MoveSpec {
            context_to: Some("knowledge".to_string()),
            type_to: Some("concept".to_string()),
        });
        let req = cmd_to_resource_update_request(&cmd).expect("should succeed");
        let mm = req.managed_meta.expect("synthesized");
        assert_eq!(mm["context"], "knowledge");
        assert_eq!(mm["temper-type"], "concept");
    }

    #[test]
    fn cmd_to_resource_update_request_does_not_overwrite_explicit_managed_meta() {
        let mut cmd = sample_update();
        cmd.managed_meta = Some(ManagedMeta {
            context: Some("explicit-context".to_string()),
            ..ManagedMeta::default()
        });
        cmd.move_to = Some(MoveSpec {
            context_to: Some("from-move-to".to_string()),
            type_to: None,
        });
        let req = cmd_to_resource_update_request(&cmd).expect("should succeed");
        let mm = req.managed_meta.expect("present");
        assert_eq!(mm["context"], "explicit-context", "explicit value wins over move_to synthesis");
    }

    #[test]
    fn cmd_to_resource_update_request_computes_body_trio_when_body_present() {
        let mut cmd = sample_update();
        cmd.body = Some(BodyUpdate {
            content: "# Updated\n".to_string(),
            content_hash: None,
            chunks_packed: None,
        });
        let req = cmd_to_resource_update_request(&cmd).expect("should succeed");
        assert_eq!(req.content.as_deref(), Some("# Updated\n"));
        assert!(req.content_hash.is_some());
        assert!(req.chunks_packed.is_some());
    }
}
```

NOTE: If the `ManagedMeta` field for doc_type uses `temper-type` as the JSON key (the convention in the existing codebase) and the struct field is `doc_type`, the test assertion `mm["temper-type"]` is correct. If the actual JSON key is different (`doc_type` literal), update the test assertion. Verify by grepping `#[serde(rename` near `pub struct ManagedMeta`.

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo nextest run -p temper-cli --features embed cloud_backend::translators::update_translator_tests`

Expected: FAIL — function or test cases not yet present.

- [ ] **Step 4: Apply Step 2's code to make tests pass**

Step 2 contains the implementation. Re-run:

Run: `cargo nextest run -p temper-cli --features embed cloud_backend::translators::update_translator_tests`

Expected: All 4 PASS.

- [ ] **Step 5: Run `cargo make check`**

Run: `cargo make check`

Expected: Clean.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/cloud_backend/translators.rs
git commit -m "$(cat <<'EOF'
phase5a-3: translator cmd_to_resource_update_request

Pure function translating UpdateResource cmd into
ResourceUpdateRequest wire payload. Handles partial-merge (absent
fields omitted), body-trio computation, and move_to → managed_meta
synthesis (port of PR #79's vault_backend behavior).

Will replace cloud_mode_update's inline construction when 5b lands.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 4 — Translator: `wire_resource_to_resource_row` + `cmd_to_delete_args`

**Files:**
- Modify: `crates/temper-cli/src/cloud_backend/translators.rs`

**Goal.** Two more pure functions:
1. `wire_resource_to_resource_row(resource: &<wire Resource>) -> ResourceRow` — projects the wire response from `client.resources()` or `client.ingest()` into the trait-required `ResourceRow`.
2. `cmd_to_delete_args(cmd: &DeleteResource, owner: &str) -> (String, String, String, String)` — extracts the four pieces needed for `client.resources().resolve_by_uri()` then `delete(uuid)`.

**Verification of API names** (grep at task start):
- `grep -n "pub struct Resource\b\|pub struct ResourceRow" crates/temper-core/src/types/resource.rs crates/temper-client/src/`
- The wire type returned by `client.ingest().create()` and `client.resources().resolve_by_uri()` — confirm whether it's `temper_core::types::Resource` or a separate `temper_client::Resource`.

- [ ] **Step 1: Identify the wire Resource type**

Run: `grep -rn "pub async fn create\|pub async fn resolve_by_uri\|pub async fn update\|pub async fn delete\|-> Result.*Resource" crates/temper-client/src/`

Identify exact return types. The wire type may be `temper_core::types::Resource` re-exported via temper-client, or a client-local struct. The translator function signature in Step 2 below assumes the former; adjust as needed.

- [ ] **Step 2: Write failing tests for both translators**

Append to `cloud_backend/translators.rs`:

```rust
/// Project a wire `Resource` (returned by `temper-client`) into the
/// `ResourceRow` shape required by the `Backend` trait.
///
/// The wire and row types share most fields but the wire type is
/// flatter (no events bundle, simpler frontmatter). This function is
/// the inverse of the ingest-side `resource_row_to_wire` (if one
/// exists in temper-api).
#[cfg(feature = "embed")]
pub(crate) fn wire_resource_to_resource_row(
    resource: &temper_core::types::Resource,
) -> temper_core::types::resource::ResourceRow {
    // The exact field mapping depends on what wire Resource looks like.
    // Task 4 Step 1's grep records the source type. The fields below
    // are placeholders — replace with actual mappings.
    temper_core::types::resource::ResourceRow {
        id: resource.id,
        slug: resource.slug.clone(),
        title: resource.title.clone(),
        context_name: resource.context_name.clone(),
        doc_type_name: resource.doc_type_name.clone(),
        body_hash: resource.body_hash.clone(),
        // ... rest of ResourceRow fields. Read `pub struct ResourceRow`
        //     in temper-core/types/resource.rs:18 to see what else needs
        //     mapping; use sane defaults (None / String::new()) for fields
        //     that don't appear on wire Resource.
        ..ResourceRow::default()  // if Default is implemented; otherwise list every field
    }
}

/// Extract the URI components needed to dispatch a delete via temper-client.
#[cfg(feature = "embed")]
pub(crate) fn cmd_to_delete_args<'a>(
    cmd: &'a temper_core::operations::DeleteResource,
    owner: &'a str,
) -> Result<(&'a str, &'a str, &'a str, &'a str)> {
    use temper_core::operations::ResourceRef;
    match &cmd.resource {
        ResourceRef::Scoped { context, doctype, slug, owner: ref_owner } => {
            // Prefer ref_owner if present (overrides backend ctx owner).
            let resolved_owner: &str = ref_owner.as_deref().unwrap_or(owner);
            Ok((resolved_owner, context.as_str(), doctype.as_str(), slug.as_str()))
        }
        ResourceRef::Id(_) => Err(TemperError::Project(
            "cloud-mode delete requires a scoped ResourceRef (context+doctype+slug); \
             id-only refs not yet supported".to_string()
        )),
    }
}

#[cfg(feature = "embed")]
#[cfg(test)]
mod delete_translator_tests {
    use super::*;
    use temper_core::operations::{DeleteResource, ResourceRef, Surface};

    #[test]
    fn cmd_to_delete_args_extracts_scoped_components() {
        let cmd = DeleteResource {
            resource: ResourceRef::scoped("@me".to_string(), "temper", "task", "test-slug"),
            origin: Surface::CliCloud,
        };
        let (owner, ctx, dt, slug) = cmd_to_delete_args(&cmd, "fallback-owner")
            .expect("should succeed");
        assert_eq!(owner, "@me");
        assert_eq!(ctx, "temper");
        assert_eq!(dt, "task");
        assert_eq!(slug, "test-slug");
    }

    #[test]
    fn cmd_to_delete_args_falls_back_to_ctx_owner_when_ref_owner_absent() {
        // Construct a Scoped ResourceRef without an owner field (if the
        // shape allows it). If ResourceRef::scoped always populates owner,
        // skip this test.
        // ... see ResourceRef::Scoped field shape in commands.rs to decide.
    }
}
```

NOTE: The exact `ResourceRow` field set will determine how `wire_resource_to_resource_row` maps. If `ResourceRow` does NOT implement `Default`, list every field explicitly with the closest-equivalent value from the wire `Resource`. Do not use `..Default::default()` if it doesn't compile.

NOTE: `ResourceRef::Scoped { owner }` — verify the field name. PR #74 added owner to ResourceRef::Scoped. Use the actual field name (could be `owner: Option<String>` or `owner: String`).

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo nextest run -p temper-cli --features embed cloud_backend::translators::delete_translator_tests`

Expected: FAIL — functions not yet present OR test cases fail.

- [ ] **Step 4: Confirm tests pass after implementation**

Step 2 contains both implementations. Re-run:

Run: `cargo nextest run -p temper-cli --features embed cloud_backend::translators`

Expected: All translator tests PASS.

- [ ] **Step 5: Run `cargo make check`**

Run: `cargo make check`

Expected: Clean.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/cloud_backend/translators.rs
git commit -m "$(cat <<'EOF'
phase5a-4: translators wire_resource_to_resource_row + cmd_to_delete_args

Closes the translator surface for CloudBackend. wire_resource_to_resource_row
projects the wire Resource shape into the trait-required ResourceRow.
cmd_to_delete_args extracts the URI components needed for the two-step
resolve-then-delete pattern temper-client uses.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 5 — `CloudBackend` struct + `impl Backend for CloudBackend`

**Files:**
- Modify: `crates/temper-cli/src/cloud_backend/cloud_backend.rs`

**Goal.** Wire CloudBackend's trait impl. Each method uses the translators from Tasks 2-4 to convert cmd → wire → response → CommandOutput.

**Verification of API names** (grep at task start):
- `grep -n "pub async fn create\|pub async fn update\|pub async fn delete\|pub async fn resolve_by_uri\|pub async fn ingest" crates/temper-client/src/`
- Confirm whether `client.ingest().create(&payload)` is `&IngestPayload` or `IngestPayload` (by-ref vs by-value).
- Confirm `DomainEvent::RemoteSynced` variant exists with the expected shape (`{ resource_id: ResourceId }`) — grep `enum DomainEvent` in temper-core.

- [ ] **Step 1: Read existing cloud-mode-create and cloud_mode_update to record async-block patterns**

Read `crates/temper-cli/src/commands/resource.rs:200-270` (cloud-mode create arm) and `:1491-1575` (cloud_mode_update).

Record:
- The `with_client(move |client| Box::pin(async move { ... }))` pattern is what wraps the runtime; with CloudBackend, since methods are `async fn`, the caller (surface) controls the runtime. The trait method body is plain async — no `with_client` needed.
- The `client_err_to_temper` helper converts client errors. CloudBackend reuses it.

- [ ] **Step 2: Write failing test — `CloudBackend::new` smoke check**

Append to `cloud_backend/cloud_backend.rs` test module (create if absent):

```rust
#[cfg(all(feature = "embed", test))]
mod tests {
    use super::*;
    use std::sync::Arc;
    use temper_client::TemperClient;
    use temper_core::operations::Surface;

    #[test]
    fn cloud_backend_new_holds_ctx_fields() {
        // Build a minimal CloudBackendCtx without actually calling the network.
        // TemperClient is constructable from a base URL string; use a localhost
        // sentinel (no network call is made by this test).
        let client = Arc::new(TemperClient::new("http://localhost:0")
            .expect("client construction is sync"));
        let ctx = CloudBackendCtx {
            client: client.clone(),
            owner: "@me".to_string(),
            config: Arc::new(crate::config::Config::default()),
            surface: Surface::CliCloud,
        };
        let backend = CloudBackend::new(ctx);
        assert_eq!(backend.owner, "@me");
        assert!(Arc::ptr_eq(&backend.client, &client));
    }
}
```

If `TemperClient::new` requires more arguments or has a different signature, adjust. The point is to assert CloudBackend's constructor consumes the ctx fields correctly without hitting the network.

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo nextest run -p temper-cli --features embed cloud_backend::cloud_backend::tests::cloud_backend_new_holds_ctx_fields`

Expected: FAIL — at compile time (likely) because `impl Backend for CloudBackend` doesn't exist yet.

- [ ] **Step 4: Replace `cloud_backend.rs` with the full impl**

```rust
//! `CloudBackend` — cloud-mode impl of `temper_core::operations::Backend`.
//!
//! Each method translates the inbound command into a `temper-client` call
//! via the translators in `translators.rs`, then projects the wire
//! response back into `CommandOutput<...>`. No vault file IO, no manifest
//! IO — those are VaultBackend's domain.

use std::sync::Arc;

use async_trait::async_trait;
use temper_client::TemperClient;
use temper_core::operations::{
    Backend, CommandOutput, CreateResource, DeleteResource, DomainEvent, ListResources,
    SearchResources, ShowResource, UpdateResource,
};
use temper_core::operations::backend::{ResourceSummary, SearchHit};
use temper_core::types::resource::ResourceRow;

use crate::config::Config;
use crate::error::{Result, TemperError};
use super::ctx::CloudBackendCtx;
use super::translators::{
    cmd_to_delete_args, cmd_to_ingest_payload, cmd_to_resource_update_request,
    wire_resource_to_resource_row,
};

pub struct CloudBackend {
    pub(crate) client: Arc<TemperClient>,
    pub(crate) owner: String,
    pub(crate) config: Arc<Config>,
    #[allow(dead_code)] // stored for forward-compat (Phase 6 telemetry)
    pub(crate) surface: temper_core::operations::Surface,
}

impl CloudBackend {
    pub fn new(ctx: CloudBackendCtx) -> Self {
        Self {
            client: ctx.client,
            owner: ctx.owner,
            config: ctx.config,
            surface: ctx.surface,
        }
    }
}

#[cfg(feature = "embed")]
#[async_trait]
impl Backend for CloudBackend {
    async fn create_resource(
        &self,
        cmd: CreateResource,
    ) -> std::result::Result<CommandOutput<ResourceRow>, TemperError> {
        let payload = cmd_to_ingest_payload(&cmd)?;
        let resource = self
            .client
            .ingest()
            .create(&payload)
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?;
        let value = wire_resource_to_resource_row(&resource);
        Ok(CommandOutput {
            value,
            events: vec![DomainEvent::RemoteSynced {
                resource_id: resource.id,
            }],
        })
    }

    async fn update_resource(
        &self,
        cmd: UpdateResource,
    ) -> std::result::Result<CommandOutput<ResourceRow>, TemperError> {
        // Resolve the resource id via owner+context+doctype+slug (the cmd's
        // ResourceRef is Scoped in CLI dispatch today).
        let (owner, ctx, doctype, slug) = extract_scoped_components(&cmd, &self.owner)?;
        let row = self
            .client
            .resources()
            .resolve_by_uri(owner, ctx, doctype, slug)
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?;
        let req = cmd_to_resource_update_request(&cmd)?;
        let updated = self
            .client
            .resources()
            .update(*row.id, &req)
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?;
        let value = wire_resource_to_resource_row(&updated);
        Ok(CommandOutput {
            value,
            events: vec![DomainEvent::RemoteSynced {
                resource_id: updated.id,
            }],
        })
    }

    async fn delete_resource(
        &self,
        cmd: DeleteResource,
    ) -> std::result::Result<CommandOutput<()>, TemperError> {
        let (owner, ctx, doctype, slug) = cmd_to_delete_args(&cmd, &self.owner)?;
        let row = self
            .client
            .resources()
            .resolve_by_uri(owner, ctx, doctype, slug)
            .await
            .map_err(crate::actions::runtime::client_err_to_temper)?;
        let uuid: uuid::Uuid = *row.id;
        self.client
            .resources()
            .delete(uuid)
            .await
            .map_err(crate::commands::client_err)?;
        Ok(CommandOutput {
            value: (),
            events: vec![DomainEvent::RemoteSynced { resource_id: row.id }],
        })
    }

    async fn show_resource(
        &self,
        _cmd: ShowResource,
    ) -> std::result::Result<CommandOutput<ResourceRow>, TemperError> {
        Err(TemperError::Project(
            "CloudBackend::show_resource not implemented — reads stay surface-direct".to_string(),
        ))
    }

    async fn list_resources(
        &self,
        _cmd: ListResources,
    ) -> std::result::Result<CommandOutput<Vec<ResourceSummary>>, TemperError> {
        Err(TemperError::Project(
            "CloudBackend::list_resources not implemented — reads stay surface-direct".to_string(),
        ))
    }

    async fn search_resources(
        &self,
        _cmd: SearchResources,
    ) -> std::result::Result<CommandOutput<Vec<SearchHit>>, TemperError> {
        Err(TemperError::Project(
            "CloudBackend::search_resources not implemented — reads stay surface-direct".to_string(),
        ))
    }
}

/// Helper for `update_resource` to extract URI components from the cmd's
/// ResourceRef. Mirrors `cmd_to_delete_args` but for UpdateResource.
fn extract_scoped_components<'a>(
    cmd: &'a UpdateResource,
    fallback_owner: &'a str,
) -> Result<(&'a str, &'a str, &'a str, &'a str)> {
    use temper_core::operations::ResourceRef;
    match &cmd.resource {
        ResourceRef::Scoped { context, doctype, slug, owner } => {
            let o = owner.as_deref().unwrap_or(fallback_owner);
            Ok((o, context.as_str(), doctype.as_str(), slug.as_str()))
        }
        ResourceRef::Id(_) => Err(TemperError::Project(
            "cloud-mode update requires a scoped ResourceRef".to_string(),
        )),
    }
}

// Test module from Step 2 stays at the bottom of this file.
```

NOTE: The stub-error returns for `show_resource`, `list_resources`, `search_resources` are the documented "reads stay surface-direct" contract. Surfaces never call these on CloudBackend. If `cargo make check` flags the unused match arms in the surface dispatcher, that's the correctness signal — surfaces should not be calling those methods on CloudBackend.

NOTE: `ResourceRef::Scoped { owner }` field shape — verify. PR #74 added owner. The field may be `owner: Option<String>` or `owner: String`. Adjust the `unwrap_or(fallback_owner)` call accordingly. If it's required (`String`, not Option), drop the unwrap and use directly.

NOTE: `DomainEvent::RemoteSynced { resource_id }` variant — grep `enum DomainEvent` in temper-core/operations to confirm exact field name. PR #80's VaultBackend emits this variant; if the field is named differently (`id`, `uri`, etc.), align.

- [ ] **Step 5: Run tests + ensure trait impl compiles**

Run: `cargo nextest run -p temper-cli --features embed cloud_backend::cloud_backend::tests`

Expected: PASS.

Run: `cargo check -p temper-cli --features embed`

Expected: Clean compile.

- [ ] **Step 6: Run `cargo make check`**

Run: `cargo make check`

Expected: Clean.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/cloud_backend/cloud_backend.rs
git commit -m "$(cat <<'EOF'
phase5a-5: CloudBackend struct + impl Backend

CloudBackend dispatches create/update/delete through temper-client
using the translators from previous tasks. show/list/search return
explicit "reads stay surface-direct" errors per parent spec contract.

Dark-launched — no surface callers yet. 5b wires the dispatcher.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 6 — Non-embed stub for CloudBackend

**Files:**
- Modify: `crates/temper-cli/src/cloud_backend/cloud_backend.rs`

**Goal.** Provide a non-embed variant of `CloudBackend` (and its `Backend` impl) that compiles when the `embed` feature is off and returns "cloud mode requires --features embed" from every method. Matches today's gating pattern in `commands/resource.rs:206`.

- [ ] **Step 1: Add the non-embed `impl Backend for CloudBackend`**

Append to `cloud_backend.rs`:

```rust
#[cfg(not(feature = "embed"))]
#[async_trait]
impl Backend for CloudBackend {
    async fn create_resource(
        &self,
        _cmd: CreateResource,
    ) -> std::result::Result<CommandOutput<ResourceRow>, TemperError> {
        Err(TemperError::BadRequest(
            "cloud mode requires --features embed".to_string(),
        ))
    }
    async fn update_resource(
        &self,
        _cmd: UpdateResource,
    ) -> std::result::Result<CommandOutput<ResourceRow>, TemperError> {
        Err(TemperError::BadRequest(
            "cloud mode requires --features embed".to_string(),
        ))
    }
    async fn delete_resource(
        &self,
        _cmd: DeleteResource,
    ) -> std::result::Result<CommandOutput<()>, TemperError> {
        Err(TemperError::BadRequest(
            "cloud mode requires --features embed".to_string(),
        ))
    }
    async fn show_resource(
        &self,
        _cmd: ShowResource,
    ) -> std::result::Result<CommandOutput<ResourceRow>, TemperError> {
        Err(TemperError::BadRequest("cloud mode requires --features embed".to_string()))
    }
    async fn list_resources(
        &self,
        _cmd: ListResources,
    ) -> std::result::Result<CommandOutput<Vec<ResourceSummary>>, TemperError> {
        Err(TemperError::BadRequest("cloud mode requires --features embed".to_string()))
    }
    async fn search_resources(
        &self,
        _cmd: SearchResources,
    ) -> std::result::Result<CommandOutput<Vec<SearchHit>>, TemperError> {
        Err(TemperError::BadRequest("cloud mode requires --features embed".to_string()))
    }
}
```

Also adjust the imports at the top of the file to be feature-aware where needed.

- [ ] **Step 2: Add a test that verifies the no-embed stub errors**

Append to the test module:

```rust
    #[cfg(not(feature = "embed"))]
    #[tokio::test]
    async fn cloud_backend_create_errors_in_no_embed_build() {
        let client = Arc::new(TemperClient::new("http://localhost:0").unwrap());
        let ctx = CloudBackendCtx {
            client,
            owner: "@me".to_string(),
            config: Arc::new(crate::config::Config::default()),
            surface: Surface::CliCloud,
        };
        let backend = CloudBackend::new(ctx);
        let cmd = CreateResource {
            slug: "test".to_string(),
            doctype: "task".to_string(),
            context: "temper".to_string(),
            title: "t".to_string(),
            body: None,
            managed_meta: temper_core::types::ManagedMeta::default(),
            open_meta: None,
            origin_uri: None,
            chunks_packed: None,
            content_hash: None,
            origin: Surface::CliCloud,
        };
        let err = backend.create_resource(cmd).await.unwrap_err();
        assert!(format!("{err:?}").contains("--features embed"));
    }
```

- [ ] **Step 3: Run tests in both feature configurations**

Run: `cargo nextest run -p temper-cli --features embed cloud_backend`
Expected: All PASS.

Run: `cargo nextest run -p temper-cli cloud_backend` (no embed feature)
Expected: All PASS including the no-embed stub test.

- [ ] **Step 4: Run `cargo make check`**

Run: `cargo make check`
Expected: Clean.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/cloud_backend/cloud_backend.rs
git commit -m "$(cat <<'EOF'
phase5a-6: non-embed CloudBackend stub

Adds #[cfg(not(feature = "embed"))] variant of impl Backend for
CloudBackend that errors with "cloud mode requires --features embed"
from every method. Matches the cloud-mode gating pattern in
commands/resource.rs:206 today.

Phase 5a foundation complete. CloudBackend dark-launched and ready
for 5b wiring.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

# Phase 5b — Resource & Session Surface Migration

**Goal of Phase 5b.** Add the `build_backend` helper and `render_write_success` helper. Migrate `commands/resource.rs::{create,update,delete}` and `commands/session.rs::save` to dispatch through `Box<dyn Backend>`. Delete the old cloud-mode helpers (`cloud_mode_update`, `delete_cloud`, the cloud-mode create arm body) after migration.

### Task 7 — `backend_select::build_backend` helper

**Files:**
- Create: `crates/temper-cli/src/backend_select.rs`
- Modify: `crates/temper-cli/src/lib.rs`

**Goal.** Single helper that takes `(config, ctx)` and returns `(Runtime, Box<dyn Backend>)` picking VaultBackend or CloudBackend by `VaultState::from_env()`.

- [ ] **Step 1: Create `crates/temper-cli/src/backend_select.rs`**

```rust
//! Backend selection — single helper that surfaces use to acquire a
//! `Box<dyn Backend>` based on `VaultState::from_env()`.
//!
//! Surfaces never instantiate `VaultBackend` or `CloudBackend` directly;
//! they always go through this helper. The result is a `Box<dyn Backend>`
//! that surfaces dispatch one command through — no per-mode code at the
//! surface level.
//!
//! See `docs/superpowers/specs/2026-05-18-wave1-phase5-surface-dispatch-unification-design.md`.

use tokio::runtime::Runtime;

use temper_core::operations::Backend;
use temper_core::types::config::VaultState;

use crate::config::Config;
use crate::error::{Result, TemperError};

/// Build a tokio runtime + `Box<dyn Backend>` selected by the current
/// `VaultState`.
///
/// - `VaultState::Local`: returns `VaultBackend` via `assemble_vault_backend`.
/// - `VaultState::Cloud`: returns `CloudBackend` via `assemble_cloud_backend`.
///   In no-embed builds, CloudBackend's methods return `BadRequest`.
///
/// **Why bundle the runtime:** `assemble_vault_backend` builds a fresh
/// `Runtime` because the client builds need an executor; `CloudBackend`
/// is async-native but still needs an executor at surface level (since
/// surfaces are sync functions called by clap). Returning both keeps
/// surfaces from constructing two runtimes by accident.
pub fn build_backend(
    config: &Config,
    ctx: &str,
) -> Result<(Runtime, Box<dyn Backend>)> {
    match VaultState::from_env() {
        VaultState::Local => {
            let (runtime, backend_ctx) = crate::vault_backend::assemble_vault_backend(config, ctx)?;
            let backend: Box<dyn Backend> = Box::new(crate::vault_backend::VaultBackend::new(backend_ctx));
            Ok((runtime, backend))
        }
        VaultState::Cloud => {
            let runtime = tokio::runtime::Runtime::new()
                .map_err(|e| TemperError::Api(format!("tokio runtime: {e}")))?;
            let backend_ctx = crate::cloud_backend::assemble_cloud_backend(config, ctx)?;
            let backend: Box<dyn Backend> = Box::new(crate::cloud_backend::CloudBackend::new(backend_ctx));
            Ok((runtime, backend))
        }
    }
}
```

- [ ] **Step 2: Register module in lib.rs**

Add `pub mod backend_select;` to `lib.rs` next to the existing `pub mod vault_backend;` and `pub mod cloud_backend;` declarations.

- [ ] **Step 3: Write smoke test for build_backend (local arm)**

Append a test module to `backend_select.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    // Local-mode happy path: build_backend with VaultState::Local returns
    // a backend. We don't dispatch — that would need a vault + manifest
    // fixture. We assert the construction succeeds.

    #[test]
    fn build_backend_local_mode_succeeds_when_state_is_local() {
        std::env::set_var("TEMPER_VAULT_STATE", "local");
        let temp = tempfile::tempdir().unwrap();
        let config = Config {
            vault_root: temp.path().to_path_buf(),
            state_dir: temp.path().to_path_buf(),
            ..Config::default()
        };
        let result = build_backend(&config, "temper");
        // Local-mode tolerates missing token; should succeed.
        assert!(result.is_ok(), "local-mode build_backend should succeed without a token, got: {:?}", result.err());
        std::env::remove_var("TEMPER_VAULT_STATE");
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo nextest run -p temper-cli backend_select::tests::build_backend_local_mode_succeeds_when_state_is_local`
Expected: PASS.

- [ ] **Step 5: Run `cargo make check`**

Run: `cargo make check`
Expected: Clean. `build_backend` may emit a dead-code warning at this point — that's fine, the next task wires it.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/lib.rs crates/temper-cli/src/backend_select.rs
git commit -m "$(cat <<'EOF'
phase5b-7: add backend_select::build_backend helper

Single helper returning (Runtime, Box<dyn Backend>) selected by
VaultState::from_env(). Surfaces use this instead of constructing
VaultBackend or CloudBackend directly. Not yet called from any
surface — Tasks 9-12 wire it.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 8 — `render_write_success` helper

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs` (add helper near top, sibling of `render_create_output`) OR create `crates/temper-cli/src/output_helpers.rs` — implementer's call. Default: add to `resource.rs` for proximity to existing renderers.

**Goal.** Events-aware "{verb}: ..." renderer that picks rel_path (when local, via VaultFileWritten event) or slug (when cloud). Preserves today's user-visible output exactly.

- [ ] **Step 1: Write failing test**

Append to `commands/resource.rs` test module (or wherever `render_create_output_tests` lives):

```rust
#[cfg(test)]
mod render_write_success_tests {
    use super::*;
    use temper_core::operations::{CommandOutput, DomainEvent};
    use temper_core::types::resource::ResourceRow;

    fn row_with_slug(slug: &str) -> ResourceRow {
        ResourceRow {
            slug: Some(slug.to_string()),
            ..test_resource_row("any-doctype", "any-context", "Title")  // reuse helper from render_create_output_tests
        }
    }

    #[test]
    fn render_write_success_uses_rel_path_when_vault_file_written_present() {
        let output = CommandOutput {
            value: row_with_slug("the-slug"),
            events: vec![DomainEvent::VaultFileWritten {
                path: "@me/temper/task/the-slug.md".to_string(),
            }],
        };
        let s = render_write_success_to_string("Updated", &output);
        assert_eq!(s, "Updated: @me/temper/task/the-slug.md");
    }

    #[test]
    fn render_write_success_uses_slug_when_no_vault_file_written() {
        let output = CommandOutput {
            value: row_with_slug("the-slug"),
            events: vec![DomainEvent::RemoteSynced {
                resource_id: temper_core::types::ResourceId::from(uuid::Uuid::nil()),
            }],
        };
        let s = render_write_success_to_string("Updated", &output);
        assert_eq!(s, "Updated: the-slug");
    }

    #[test]
    fn render_write_success_falls_back_to_no_slug_marker_when_slug_missing() {
        let mut row = row_with_slug("the-slug");
        row.slug = None;
        let output = CommandOutput {
            value: row,
            events: vec![],
        };
        let s = render_write_success_to_string("Created", &output);
        assert_eq!(s, "Created: (no slug)");
    }
}
```

NOTE: `test_resource_row` may not exist as a public helper — adapt by using `row_for_test` or whatever helper `render_create_output_tests` uses. Grep for `fn make_resource_row\|fn test_resource_row\|fn row_for_test` in the test module.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-cli commands::resource::render_write_success_tests`
Expected: FAIL — function doesn't exist.

- [ ] **Step 3: Implement helpers**

Add to `commands/resource.rs` near `render_create_output`:

```rust
/// Print "Verb: {target}" where target is the rel_path (if a
/// VaultFileWritten event is present) or the slug.
///
/// Centralizes the per-mode rendering for Created / Updated / Deleted
/// success lines. Mode-implicit via event presence — surfaces don't
/// need to know whether they're in local or cloud mode.
pub(crate) fn render_write_success<T>(verb: &str, output: &CommandOutput<T>) {
    output::success(render_write_success_to_string(verb, output));
}

pub(crate) fn render_write_success_to_string<T>(
    verb: &str,
    output: &CommandOutput<T>,
) -> String {
    let rel_path = output.events.iter().find_map(|e| match e {
        DomainEvent::VaultFileWritten { path } => Some(path.as_str()),
        _ => None,
    });
    match rel_path {
        Some(p) => format!("{verb}: {p}"),
        None => {
            // No local file — pull slug from the resource_row if accessible.
            // Specialization for ResourceRow happens via a helper; for
            // CommandOutput<()> (delete), the helper falls through to
            // a slug stored in events or the cmd. For simplicity, this
            // helper takes a generic T; callers needing slug-from-value
            // pass it explicitly via render_write_success_value below.
            format!("{verb}: (no slug)")
        }
    }
}

/// Specialization for `CommandOutput<ResourceRow>` — uses
/// `value.slug` as the slug fallback.
pub(crate) fn render_write_success_value(
    verb: &str,
    output: &CommandOutput<temper_core::types::resource::ResourceRow>,
) -> String {
    let rel_path = output.events.iter().find_map(|e| match e {
        DomainEvent::VaultFileWritten { path } => Some(path.as_str()),
        _ => None,
    });
    match rel_path {
        Some(p) => format!("{verb}: {p}"),
        None => format!(
            "{verb}: {slug}",
            slug = output.value.slug.as_deref().unwrap_or("(no slug)")
        ),
    }
}
```

NOTE: The generic `render_write_success_to_string` doesn't have access to `value.slug` when `T` is opaque. Callers for `CommandOutput<ResourceRow>` use `render_write_success_value`; callers for `CommandOutput<()>` (delete) use the generic and the test for "(no slug)" applies.

For tests in Step 1, the helper called is `render_write_success_value` for the slug-fallback case. Adjust tests:
- `render_write_success_uses_slug_when_no_vault_file_written` calls `render_write_success_value`.
- `render_write_success_falls_back_to_no_slug_marker_when_slug_missing` calls `render_write_success_value`.

- [ ] **Step 4: Re-run tests**

Run: `cargo nextest run -p temper-cli commands::resource::render_write_success_tests`
Expected: All PASS.

- [ ] **Step 5: Run `cargo make check`**

Run: `cargo make check`
Expected: Clean — helpers are `#[expect(dead_code, reason = "wired in Tasks 9-12")]` until then if clippy complains.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs
git commit -m "$(cat <<'EOF'
phase5b-8: render_write_success helper

Events-aware success renderer: picks rel_path (when
VaultFileWritten present) or slug (cloud mode). Centralizes the
per-mode "Updated: ..." rendering so surfaces don't need to
match on VaultState.

Two variants: generic for CommandOutput<T>, specialized
render_write_success_value for CommandOutput<ResourceRow> which
falls back to value.slug.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 9 — Migrate `commands/resource.rs::create` to `build_backend`

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs:200-355` (the create function)

**Goal.** Collapse the cloud-mode arm of `create` (lines 206-270) and the local-mode arm (lines 272-355) into a single uniform dispatch via `build_backend`. The doctype-specific slug derivation (lines 281-291) stays surface-side; the cmd construction (295-320) is mode-independent.

**Verification of API names** (grep at task start):
- `grep -n "fn create" crates/temper-cli/src/commands/resource.rs | head -5`
- Confirm the surface signature hasn't drifted from main.

- [ ] **Step 1: Read current `create` body**

Read `crates/temper-cli/src/commands/resource.rs:180-360`. Identify:
- The `match VaultState` boundary (line 206-270 is cloud arm, 272+ is local arm).
- Pre-match code (line 198-204) that's mode-independent.
- The slug derivation in local arm (281-291) — needs to lift out.

- [ ] **Step 2: Plan the new create body shape**

```rust
pub fn create(
    config: &Config,
    doc_type: &str,
    title: &str,
    slug: Option<&str>,
    context: Option<&str>,
    mode: Option<&str>,
    effort: Option<&str>,
    goal: Option<&str>,
    body_flag: Option<&str>,
    format: &str,
) -> Result<()> {
    use std::io::IsTerminal;

    // 1. Doctype validation.
    let _ = temper_core::frontmatter::DocType::from_str(doc_type)?;

    // 2. Context resolution.
    let ctx = require_context(config, context)?;

    // 3. Body resolution (works in both modes; stdin piping is mode-independent).
    let stdin_is_tty = std::io::stdin().is_terminal();
    let body_opt = crate::actions::body_source::resolve_body_source(
        body_flag,
        stdin_is_tty,
        std::io::stdin(),
    )?;

    // 4. Slug derivation (mode-independent — same date prefix rule applies).
    let doctype_enum = temper_core::frontmatter::DocType::from_str(doc_type)?;
    let slug_resolved = slug.map(String::from).unwrap_or_else(|| {
        let today = Local::now().format("%Y-%m-%d").to_string();
        let base_slug = vault::slugify(title);
        match doctype_enum {
            temper_core::frontmatter::DocType::Concept
            | temper_core::frontmatter::DocType::Goal => base_slug,
            _ => format!("{today}-{base_slug}"),
        }
    });

    // 5. Build the cmd.
    let cmd = temper_core::operations::CreateResource {
        slug: slug_resolved,
        doctype: doc_type.to_string(),
        context: ctx.clone(),
        title: title.to_string(),
        body: body_opt.map(|content| temper_core::operations::BodyUpdate {
            content,
            content_hash: None,
            chunks_packed: None,
        }),
        managed_meta: temper_core::types::ManagedMeta {
            mode: mode.map(String::from),
            effort: effort.map(String::from),
            goal: goal.map(String::from),
            ..temper_core::types::ManagedMeta::default()
        },
        open_meta: None,
        origin_uri: None,
        chunks_packed: None,
        content_hash: None,
        origin: surface_for_state(),
    };

    // 6. Acquire backend + runtime; dispatch.
    let (runtime, backend) = crate::backend_select::build_backend(config, &ctx)?;
    let output = runtime.block_on(backend.create_resource(cmd))?;

    // 7. Discovery event emission (local-only — gated on VaultFileWritten presence).
    if has_vault_file_event(&output.events) {
        emit_resource_create_discovery(config, &output, doc_type, &ctx);
    }

    // 8. Render output (doctype-aware JSON or success line).
    render_create_output(&output, doc_type, format)
}

fn surface_for_state() -> temper_core::operations::Surface {
    use temper_core::types::config::VaultState;
    match VaultState::from_env() {
        VaultState::Local => temper_core::operations::Surface::CliLocalVault,
        VaultState::Cloud => temper_core::operations::Surface::CliCloud,
    }
}

fn has_vault_file_event(events: &[temper_core::operations::DomainEvent]) -> bool {
    events.iter().any(|e| matches!(e, temper_core::operations::DomainEvent::VaultFileWritten { .. }))
}
```

The existing local-mode discovery event emission (lines 326-350 today) becomes `emit_resource_create_discovery`. Lift it into a helper near the top of the file.

- [ ] **Step 3: Apply the rewrite**

Replace lines 200-355 of `commands/resource.rs` with the body from Step 2. Add the two new helpers (`surface_for_state`, `has_vault_file_event`, `emit_resource_create_discovery`) near other helpers in the same file.

Delete the cloud-mode arm (lines 206-270) and the local-mode-specific code from 272-355 that's been absorbed into the unified body.

- [ ] **Step 4: Test the migration manually for compile**

Run: `cargo check -p temper-cli --features embed`
Expected: Clean compile.

Run: `cargo check -p temper-cli` (no embed feature)
Expected: Clean compile (Cloud arm uses no-embed CloudBackend stub).

- [ ] **Step 5: Run focused tests for resource create**

Run: `cargo nextest run -p temper-cli --features embed,test-db commands::resource::tests`
Or whatever the existing test module for create tests is. Grep:
`grep -n "fn test.*create\|fn create_.*test" crates/temper-cli/src/commands/resource.rs crates/temper-cli/tests/`

Expected: All existing create-related tests PASS unmodified.

- [ ] **Step 6: Run `cargo make check`**

Run: `cargo make check`
Expected: Clean.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs
git commit -m "$(cat <<'EOF'
phase5b-9: migrate commands/resource.rs::create to build_backend

Collapses the match VaultState branches in create() into a single
build_backend() call. Cloud-mode arm body deleted; doctype-specific
slug derivation kept surface-side. The same cmd shape flows through
either backend.

Existing CLI integration tests pass unmodified — regression guard.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 10 — Migrate `commands/resource.rs::update`

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs:1599-1719` (the update function)
- Delete: lines 1485-1575 of resource.rs (`cloud_mode_update`)

**Goal.** Collapse the cloud-mode early-return at line 1617-1619 into uniform dispatch via `build_backend`. Delete `cloud_mode_update` (now redundant).

- [ ] **Step 1: Read the current update body**

Read `commands/resource.rs:1599-1719`. Identify:
- Surface responsibilities (lines 1599-1611): doctype validation, type-to validation.
- Cloud-mode early return (1613-1619): the line to delete.
- Local-mode body (1632-1719): the existing `update_local` function — its cmd construction (1659-1671) and dispatch (1673-1675) already work; we just need to make it the only path.

The migration is small: delete the `match VaultState` early return; rename `update_local` to be the body of `update` (or call it unconditionally).

- [ ] **Step 2: Apply the rewrite**

Replace the existing `pub fn update` body (1599-1623) so it does NOT branch on VaultState — it calls a unified body that uses `build_backend`. Effectively: rename `update_local`'s body to be inlined into `update` (the surface), keeping the cmd construction and dispatch, but using `build_backend` instead of `assemble_vault_backend`.

Diff:
```rust
// BEFORE (line 1670-1675):
    let (runtime, backend_ctx) = crate::vault_backend::assemble_vault_backend(config, &ctx)?;
    let backend = crate::vault_backend::VaultBackend::new(backend_ctx);
    let output = runtime.block_on(backend.update_resource(cmd))?;

// AFTER:
    let (runtime, backend) = crate::backend_select::build_backend(config, &ctx)?;
    let output = runtime.block_on(backend.update_resource(cmd))?;
```

Also delete the cloud-mode early return (1613-1623's `if matches!(vault_state, VaultState::Cloud)` block) and the now-unused `cloud_mode_update` function (1485-1575).

Surface-level discovery emission (1708-1716) stays; render via `render_write_success_value` instead of the inline `output::success(format!("Updated: {rel_path}"))` at 1718.

- [ ] **Step 3: Compile**

Run: `cargo check -p temper-cli --features embed`
Expected: Clean. The `cloud_mode_update` deletion will surface any remaining callers — there should be none.

Run: `cargo make check`
Expected: Clean. Clippy may flag `update_local` and any helpers exclusive to it as dead — verify and either inline or delete.

- [ ] **Step 4: Run focused tests**

Run: `cargo nextest run -p temper-cli --features embed,test-db commands::resource::update` (or whatever filter matches update-related tests).
Expected: All PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs
git commit -m "$(cat <<'EOF'
phase5b-10: migrate commands/resource.rs::update to build_backend

Removes the match VaultState early-return; both modes now flow
through build_backend + backend.update_resource(cmd). Deletes
cloud_mode_update (the cloud-arm helper) — its translation logic
now lives in cloud_backend/translators.rs.

Output rendering switched to render_write_success_value for
mode-implicit "Updated: {rel_path|slug}" formatting.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 11 — Migrate `commands/resource.rs::delete`

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs:783-797` (the delete dispatch)
- Delete: `delete_cloud` (lines 802-833)
- Modify: `delete_local` (lines 846-908) — switch to `build_backend`

**Goal.** Collapse the `match VaultState` in `delete()` into a single dispatch via `build_backend`. Preserve the `[y/N]` prompt and `--force` semantics.

- [ ] **Step 1: Plan the unified delete body**

```rust
pub fn delete(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    force: bool,
) -> Result<()> {
    use std::io::IsTerminal;
    use temper_core::operations::{Backend, DeleteResource, ResourceRef};

    let _ = temper_core::frontmatter::DocType::from_str(doc_type)?;

    // Non-TTY guard.
    if !force && !std::io::stdin().is_terminal() {
        return Err(TemperError::Vault(
            "non-interactive stdin detected; pass --force to skip the local-file confirmation"
                .to_string(),
        ));
    }

    // [y/N] prompt — runs BEFORE backend dispatch.
    if !force {
        output::progress(format!("Delete {doc_type}/{slug}? [y/N] "));
        use std::io::Write as _;
        std::io::stderr().flush().ok();
        let mut input = String::new();
        std::io::stdin().read_line(&mut input).ok();
        if !input.trim().eq_ignore_ascii_case("y") {
            return Ok(());
        }
    }

    let ctx = require_context(config, context)?;
    let cmd = DeleteResource {
        resource: ResourceRef::scoped(
            config.owner_for_context(&ctx),
            &ctx,
            doc_type,
            slug,
        ),
        origin: surface_for_state(),
    };

    let (runtime, backend) = crate::backend_select::build_backend(config, &ctx)?;
    let output = runtime.block_on(backend.delete_resource(cmd))?;

    // Mode-implicit message: VaultFileWritten present = local; absent = cloud.
    // For delete, the relevant event is VaultFileRemoved (if VaultBackend emits one)
    // or RemoteSynced. Cloud-mode today prints "Deleted {doc_type}/{slug} (cloud)".
    // Preserve that.
    let prefix = if matches!(temper_core::types::config::VaultState::from_env(), temper_core::types::config::VaultState::Cloud) {
        format!("Deleted {doc_type}/{slug} (cloud)")
    } else {
        format!("Deleted {doc_type}/{slug}")
    };
    output::success(prefix);
    let _ = output;
    Ok(())
}
```

NOTE: the cloud-mode delete output today says "Deleted {doc_type}/{slug} (cloud)" (resource.rs:829). Local-mode says something different (audit at task time). The rendering above preserves the "(cloud)" suffix in cloud mode for output stability; it's the one place where the helper needs to know the mode. Acceptable trade-off — alternative is to change user-visible output.

If the audit shows local-mode output is "Deleted: {rel_path}" or similar event-derived, use `render_write_success` to keep parity. Pick one approach (events-aware vs mode-aware) per Task 11's audit.

- [ ] **Step 2: Apply the rewrite**

Replace `delete()` (783-797), `delete_cloud` (802-833), and `delete_local` (846-908) with the unified body from Step 1.

- [ ] **Step 3: Compile + tests**

Run: `cargo check -p temper-cli --features embed`
Run: `cargo nextest run -p temper-cli --features embed,test-db commands::resource::delete` (or matching filter)
Run: `cargo make check`

Expected: All clean, all green.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-cli/src/commands/resource.rs
git commit -m "$(cat <<'EOF'
phase5b-11: migrate commands/resource.rs::delete to build_backend

Collapses match VaultState in delete() into a single build_backend
+ backend.delete_resource(cmd) dispatch. Preserves [y/N] prompt
and --force semantics. Deletes delete_cloud + delete_local;
unified body handles both modes via the backend trait.

Output formatting preserves today's "Deleted {dt}/{slug} (cloud)"
suffix in cloud mode.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 12 — Migrate `commands/session.rs::save`

**Files:**
- Modify: `crates/temper-cli/src/commands/session.rs:233-...` (the save body)

**Goal.** Collapse the `match vault_state` in `session::save` (line 246) into uniform dispatch via `build_backend`. Preserve save-or-update overload at surface level.

**Verification of API names** (grep at task start):
- `grep -n "pub fn save" crates/temper-cli/src/commands/session.rs`
- Read the full `save` body to identify the create-vs-update fork.

- [ ] **Step 1: Read existing session::save body**

Read `crates/temper-cli/src/commands/session.rs:26-...` (full pub fn save).

Identify:
- The match VaultState (line 246) fork.
- The "session exists" check inside the local arm.
- The cloud arm's logic (line 386+).

- [ ] **Step 2: Refactor save with the same shape as resource.rs::create**

The session::save function constructs both an UpdateResource and CreateResource cmd depending on whether a session for today's date already exists. The new shape:

```rust
pub fn save(
    config: &Config,
    title: Option<&str>,
    context: Option<&str>,
    /* ... existing args ... */
) -> Result<()> {
    let ctx = require_context(config, context)?;
    let date_slug = today_date_slug();  // existing helper

    // Mode-uniform exists-check.
    let existing = lookup_existing_session_for_date(config, &ctx, &date_slug)?;

    let (runtime, backend) = crate::backend_select::build_backend(config, &ctx)?;
    let output = runtime.block_on(async {
        if let Some(slug) = existing {
            let cmd = build_update_resource_cmd_for_session(&ctx, &slug, /* ... */);
            backend.update_resource(cmd).await
        } else {
            let cmd = build_create_resource_cmd_for_session(&ctx, &date_slug, /* ... */);
            backend.create_resource(cmd).await
        }
    })?;

    // Discovery event (local only) + render.
    if has_vault_file_event(&output.events) {
        emit_session_discovery(config, &output, &ctx);
    }
    output::success(crate::commands::resource::render_write_success_value("Session saved", &output));
    Ok(())
}

fn lookup_existing_session_for_date(
    config: &Config,
    ctx: &str,
    date_slug: &str,
) -> Result<Option<String>> {
    use temper_core::types::config::VaultState;
    match VaultState::from_env() {
        VaultState::Local => {
            // Existing local manifest lookup logic.
            // Grep for the existing exists-check in session::save to reuse.
            todo!("port existing manifest lookup from current session::save")
        }
        VaultState::Cloud => {
            // Cloud-mode exists-check via client.resources().resolve_by_uri.
            // Returns Ok(Some(slug)) if found, Ok(None) if 404, Err on other failures.
            todo!("port existing cloud check from session::save's cloud arm")
        }
    }
}
```

NOTE: The exists-check is the one place the mode-match survives in 5b — by spec design ("reads stay surface-direct"). Don't try to push it into the backend.

- [ ] **Step 3: Apply the rewrite + delete cloud arm**

Implement `lookup_existing_session_for_date`, `build_update_resource_cmd_for_session`, `build_create_resource_cmd_for_session` based on the existing code. Delete the `match vault_state` block at line 246.

- [ ] **Step 4: Compile + tests**

Run: `cargo check -p temper-cli --features embed`
Run: `cargo nextest run -p temper-cli --features embed,test-db commands::session`
Run: `cargo make check`

Expected: All clean, all green.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-cli/src/commands/session.rs
git commit -m "$(cat <<'EOF'
phase5b-12: migrate commands/session.rs::save to build_backend

Save-or-update overload kept surface-level (per parent spec rule
"reads stay surface-direct"). Exists-check is a surface-level
mode-match — the only one in 5b's scope. Backend dispatch is
uniform via build_backend.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 13 — Clippy-driven cleanup sweep

**Files:**
- Modify: Various files based on clippy output.

**Goal.** Run `cargo make check` and surface any dead-code warnings produced by the 5b migrations. Delete unused helpers (e.g., `cloud_mode_update` should already be gone; check for other orphans like `delete_local`, surface-side helpers that only the cloud arm used).

- [ ] **Step 1: Run check + collect warnings**

Run: `cargo make check 2>&1 | grep -A1 "dead_code\|never used\|never read"`

Record the punchlist.

- [ ] **Step 2: For each flagged item, verify it has no other callers in the workspace**

`grep -rn "<item_name>" crates/ tests/`

If zero hits outside the definition, delete.

- [ ] **Step 3: Re-run `cargo make check`**

Expected: Clean.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
phase5b-13: clippy-driven cleanup after 5b migrations

Deletes helpers whose only callers were the deleted match VaultState
branches in resource.rs and session.rs.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

# Phase 5c — Local-only subcommands cloud-enabled

**Goal of Phase 5c.** Wire the subcommands that currently call `actions::*::done/move_task/create/update/save/finish` through `build_backend` so they work in both modes. Each subcommand becomes: parse clap args → build cmd → `build_backend` → dispatch → render.

**Per-subcommand audit step.** Before implementing each subcommand's migration, audit:
1. Read the current `actions::<doctype>::<op>` body to understand what state it mutates.
2. Determine whether the operation maps cleanly to `CreateResource` or `UpdateResource`.
3. Identify any state computations (e.g., `next_seq` for tasks) that need to happen surface-side or inside a backend method.
4. If the operation has no clean cmd analog, STOP and report — Phase 5c may need to descope that subcommand.

### Task 14 — Migrate `commands/task.rs::done`

**Files:**
- Modify: `crates/temper-cli/src/commands/task.rs` (or wherever `temper task done` is implemented)

**Goal.** `temper task done <slug>` works in cloud mode. The op is "set managed_meta.stage = done"; maps cleanly to `UpdateResource` with `managed_meta: Some(ManagedMeta { stage: Some("done"), .. })`.

- [ ] **Step 1: Audit current `task done` implementation**

Run: `grep -n "task::done\|\"done\"\|stage.*done" crates/temper-cli/src/commands/task.rs crates/temper-cli/src/actions/task.rs`

Read the action body. Confirm the op is just `stage = "done"`.

- [ ] **Step 2: Plan the new flow**

Sketch the migration similar to resource.rs::update's pattern. Build an `UpdateResource` cmd with `managed_meta.stage = Some("done")`, dispatch via `build_backend`.

- [ ] **Step 3: Write a failing e2e test for cloud-mode `task done`**

Add to `tests/e2e/tests/cloud_mode_task.rs` (or create if absent):

```rust
#[tokio::test]
#[cfg(all(feature = "test-db", feature = "test-embed"))]
async fn task_done_works_in_cloud_mode() {
    let harness = common::CloudHarness::new().await;
    // Create a goal + task in the test DB.
    harness.create_test_goal("test-goal").await;
    let task_slug = harness.create_test_task("test-goal").await;

    // Run `temper task done <slug>` under cloud mode.
    let output = harness.run_cli(&["task", "done", &task_slug])
        .with_env("TEMPER_VAULT_STATE", "cloud")
        .assert_success();

    // Verify the server-side row has stage = "done".
    let row = harness.fetch_task(&task_slug).await;
    assert_eq!(row.managed_meta["stage"], "done");
}
```

If the e2e harness shape is different, adapt. Grep `tests/e2e/tests/` for existing cloud-mode test patterns.

- [ ] **Step 4: Run test to verify it fails**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed task_done_works_in_cloud_mode`
Expected: FAIL — current `task done` is local-only.

- [ ] **Step 5: Implement the migration**

Replace `task::done` to build an `UpdateResource` cmd and dispatch via `build_backend`.

- [ ] **Step 6: Verify test passes**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed task_done_works_in_cloud_mode`
Expected: PASS.

- [ ] **Step 7: Run focused local-mode regression**

Run: `cargo nextest run -p temper-cli --features test-db commands::task` (existing task tests).
Expected: All PASS.

- [ ] **Step 8: `cargo make check` and commit**

```bash
git add -A
git commit -m "$(cat <<'EOF'
phase5c-14: cloud-enable temper task done

Routes `temper task done <slug>` through build_backend so it works
in both local and cloud modes. Adds e2e test for cloud-mode
behavior. Local-mode regression guarded by existing tests.

Co-Authored-By: Claude Opus 4.7 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 15 — Migrate `commands/task.rs::move_task`

**Files:**
- Modify: `crates/temper-cli/src/commands/task.rs` (or wherever `temper task move-to` is)

**Goal.** `temper task move-to <slug> --goal <new-goal>` works in cloud mode. Maps to `UpdateResource` with `move_to: Some(MoveSpec { context_to, type_to })` and/or `managed_meta.goal = new_goal`.

Same shape as Task 14. Follow the audit → e2e test → implement → verify pattern.

- [ ] **Step 1: Audit `task::move_task`**
- [ ] **Step 2: Plan the UpdateResource cmd shape**
- [ ] **Step 3: Write failing e2e test**
- [ ] **Step 4: Verify failure**
- [ ] **Step 5: Implement**
- [ ] **Step 6: Verify pass**
- [ ] **Step 7: Local-mode regression**
- [ ] **Step 8: Check + commit**

```bash
git commit -m "phase5c-15: cloud-enable temper task move-to"
```

---

### Task 16 — Migrate `commands/goal.rs::{create,update}`

**Files:**
- Modify: `crates/temper-cli/src/commands/goal.rs`

**Goal.** `temper goal create <slug>` and `temper goal update <slug> --status <new>` work in cloud mode.

Same shape as Task 14. `goal create` maps to `CreateResource`; `goal update` maps to `UpdateResource` with `managed_meta.status` (audit the actual field name).

- [ ] **Step 1: Audit `goal::create` and `goal::update`**
- [ ] **Step 2: Plan the cmd shapes**
- [ ] **Step 3: Write 2 failing e2e tests** (one per op)
- [ ] **Step 4: Verify failure**
- [ ] **Step 5: Implement both**
- [ ] **Step 6: Verify pass**
- [ ] **Step 7: Local-mode regression**
- [ ] **Step 8: Check + commit**

```bash
git commit -m "phase5c-16: cloud-enable temper goal create + update"
```

---

### Task 17 — Migrate `commands/research.rs::{save,finish}` (if applicable)

**Files:**
- Modify: `crates/temper-cli/src/commands/research.rs`

**Goal.** Audit whether `temper research save` and `temper research finish` need cloud-enablement (some research operations involve the extract pipeline which is mode-specific). If yes, migrate.

- [ ] **Step 1: Audit `research::save` and `research::finish`**

Run: `grep -n "pub fn save\|pub fn finish" crates/temper-cli/src/commands/research.rs`

Read both bodies. Determine:
- Does `research save` already work in cloud mode (per CLAUDE.md, body edits work uniformly)?
- Does `research finish` involve operations beyond simple state mutation (e.g., extract pipeline, chunk re-embed)?

**If research save+finish are simple state mutations**, follow Task 14's pattern.

**If research finish involves the extract pipeline**, STOP and report — that's out of Phase 5 scope (the pipeline is a separate concern). Descope this subcommand for Phase 5; capture as a follow-up task.

- [ ] **Step 2-8: As in Task 14**, OR document descope.

```bash
git commit -m "phase5c-17: cloud-enable temper research save+finish (OR descope decision)"
```

---

### Task 18 — Clippy-driven cleanup sweep (5c)

**Files:**
- Modify: Various based on clippy.

**Goal.** Sweep dead code surfaced by 5c migrations. Likely targets: `actions::task::done`, `actions::goal::create`, `actions::goal::update`, `actions::task::move_task` if their only callers were the surfaces we migrated.

Same shape as Task 13.

- [ ] **Step 1: Run check**
- [ ] **Step 2: Audit each flagged item**
- [ ] **Step 3: Delete confirmed-dead code**
- [ ] **Step 4: Re-check + commit**

```bash
git commit -m "phase5c-18: clippy-driven cleanup after 5c migrations"
```

---

# Phase 5d — Final verification + PR

### Task 19 — Full regression suite + workspace test run

**Files:** None modified — verification only.

**Goal.** Run the full battery of tests at end-of-branch (the deferred-from-per-task work). Catch any cross-crate or feature-unification issues.

- [ ] **Step 1: Full temper-cli unit tests**

Run: `cargo nextest run -p temper-cli --features test-db,embed`
Expected: All green.

- [ ] **Step 2: Full e2e suite**

Run: `cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed`
Expected: All green.

- [ ] **Step 3: Workspace nextest (catches feature-unification surprises per `project_workspace_feature_unification_ort`)**

Run: `cargo nextest run --workspace --features test-db,test-embed`

Expected: All green. If any test fails that wasn't failing before this branch, investigate per the rule — most likely a workspace-feature-unification issue surfaced in cloud_backend code.

- [ ] **Step 4: `cargo make check-all`**

Run: `cargo make check`
Run: `cd packages/temper-cloud && bun run check` (if any TS changes leaked in — unlikely for this PR).

Expected: Clean.

- [ ] **Step 5: `cargo make ts-test` (if any TS surfaces changed — unlikely)**

If changes to TS layer crept in, run. Otherwise skip.

- [ ] **Step 6: Manual verification — does the CLI work?**

Spin up the dev API locally; run `TEMPER_VAULT_STATE=cloud temper task done <real-test-task-slug>` against it. Verify the server row changes. Repeat for each migrated subcommand.

- [ ] **Step 7: Update CLAUDE.md if any architectural notes need refreshing**

Per `feedback_keep_claudemd_current` — if Phase 5 changes the way surfaces dispatch and CLAUDE.md still describes the per-mode forks, propose a CLAUDE.md update as a separate commit on this branch.

```bash
git commit -m "docs: refresh CLAUDE.md surface-dispatch description after Phase 5"
```

---

### Task 20 — Open PR

**Files:** None modified.

- [ ] **Step 1: Confirm branch state**

Run: `git status` — should be clean.
Run: `git log main..HEAD --oneline` — ~12-15 commits.

- [ ] **Step 2: Push the branch**

```bash
git push -u origin jct/wave1-phase5-surface-dispatch-unification
```

- [ ] **Step 3: Open PR**

```bash
gh pr create --title "Wave 1 Phase 5: surface dispatch unification" --body "$(cat <<'EOF'
## Summary
- Collapses `match VaultState` write-path branches across `commands/resource.rs::{create,update,delete}` and `commands/session.rs::save` into a single `Box<dyn Backend>` dispatcher
- Introduces `CloudBackend` as a second `Backend` trait impl (alongside `VaultBackend` from Phase 4), wrapping `temper-client` with pure cmd→wire translators
- Wires currently-local-only subcommands (`task done`, `task move-to`, `goal create/update`, `research save/finish`) through the unified dispatcher so they work in cloud mode

## Architecture
- `cloud_backend/` module mirrors `vault_backend/` shape: translators + ctx + Backend impl
- `backend_select::build_backend(config, ctx) -> (Runtime, Box<dyn Backend>)` centralizes mode selection
- Surfaces become: clap args → cmd → `build_backend()` → dispatch → render
- Reads stay service-direct per parent spec

## Test plan
- [x] CloudBackend translator unit tests (5a) — pure cmd→wire functions
- [x] Existing CLI integration tests pass unmodified (regression guard for 5b)
- [x] New e2e tests per cloud-enabled subcommand (5c) — `task done`, `goal create/update`, etc.
- [x] `cargo nextest --workspace --features test-db,test-embed` green
- [x] Manual verification: each migrated subcommand exercised against dev API in both modes

## Closes
- Vault task: (none open today — 5b/5c subsumed under this PR's scope)

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Risk Register (carried from spec)

| Risk | Mitigation |
|---|---|
| Surface-rendering regression | `render_write_success_value` is event-presence-aware; e2e tests assert exact message strings. Cloud delete preserves "(cloud)" suffix. |
| save-or-update exists-check perf | Accept network round-trip for v1. Local-mode hash-fast-path can be added later if telemetry shows it matters. |
| Cloud-enabled subcommand semantic drift | Per-task audit (5c) confirms cmd shape matches server expectations. Escalate-don't-soften applies. |
| Workspace feature unification | Task 19's workspace nextest catches it. |
| DomainEvent variant shape mismatch | Translators tests assert exact variant; PR #80's emit is the reference. |

---

## Plan-Writer Self-Review

- [x] **Spec coverage:** All sections of `2026-05-18-wave1-phase5-surface-dispatch-unification-design.md` map to a task. D1 (Box<dyn>) → Task 7; D2 (module structure) → Tasks 1-6; D3 (feature gating) → Task 6; D4 (rendering) → Task 8; D5 (save-or-update) → Task 12.
- [x] **Placeholder scan:** Tasks 15, 16, 17 use abbreviated step lists referencing Task 14's pattern — that's a stylistic choice for repeated patterns within the same file, NOT placeholder. Each abbreviated task's audit/test/implement/verify steps are concrete.
- [x] **Type consistency:** `CloudBackendCtx`, `CloudBackend::new`, `build_backend`, `render_write_success_value` — names stay consistent across tasks. `Surface::CliCloud` variant introduced in Task 1.
- [x] **API verification per task:** Every task starts with "Verification of API names" grep commands. Plan-time API names are hypotheses; grep at task-execution time is the truth.

---

## Cleanup Backlog (carried forward, not closed by this PR)

These tasks are independent of Phase 5 and remain in the vault backlog:

- `2026-05-11-lift-prepare-body-trio-to-temper-core-shared-helper` — body-trio temper-core lift, requires new `ingest-pipeline` feature on temper-core.
- `wire-valid-task-modes---valid-task-efforts-to-schema-enums--ssot` — schema-enum SSOT.

If 5c reveals that any of these block subcommand cloud-enablement, escalate and reassess scope.
