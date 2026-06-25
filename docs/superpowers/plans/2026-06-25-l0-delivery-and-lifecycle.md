# L0 Delivery & Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship authored L0 kernel content onto the live `system-default` cognitive map (and keep it updated every release) via an idempotent, admin-gated desired-state reconcile — `PUT /api/cognitive-maps/{id}` driven by a `temper` admin command that embeds a committed manifest client-side.

**Architecture:** A committed text **manifest** (authored L0 content) is the desired state. The operator CLI embeds each entry client-side (reusing `compute_body_chunks`, exactly as `temper resource create` already does) and PUTs a **pre-embedded reconcile request**. The server (one `reconcile_cognitive_map` operations command, dispatched through the `Backend` trait) opens a `kb_invocations` `admin_reconcile` envelope (also the serialization mutex), reads L0's current `provenance: kernel` slice keyed by `origin_uri`, **diffs by body content-hash**, and applies *additive-only* mutations — `create` / `update` (re-block) / `fold` (explicit tombstone only) — through thin cogmap-homed `temper_next::writes` wrappers, then closes the envelope with `{created, updated, folded}`. Same machine whether run once (first delivery: empty → populated) or every release (lifecycle).

**Tech Stack:** Rust workspace (temper-core / temper-next / temper-api / temper-client / temper-cli), Axum + utoipa (HTTP), sqlx (compile-time-checked SQL, `temper_next` namespace for substrate queries), temper-ingest (bge-768 ONNX embedding, client-side), ts-rs (wire types), cargo-nextest, e2e crate (real Axum + Postgres + JWT).

## Global Constraints

These are copied verbatim from the spec + repo discipline and apply to **every** task.

- **Embed locus = CLIENT-SIDE (AMENDS spec §2/§4).** The spec wrote "embed server-side in the PUT handler." Grounding shows production embedding is client-side: the CLI's `compute_body_chunks` (`temper_ingest::pipeline::prepare_markdown`) embeds and ships pre-computed chunks; the server stores `IncomingChunk`s. A server-side ONNX fallback *does* exist (`temper_next::writes::create_resource` None-branch via `prepare_block`) and is shipped, so server-side is *viable but not chosen*. The reconciler stays a pure **diff + store + event** path: no ONNX on the request handler. The committed manifest stays authored-text; the CLI bridges text → pre-embedded request, mirroring `temper resource create`. (User-confirmed 2026-06-25.)
- **Additive-only + provenance-scoped (O3 + O1).** Identity key = `origin_uri`. Create on absent, update on body-hash change, **never fold on mere absence** — removal requires an explicit `fold:` tombstone. Reconcile manages **only** the `provenance: kernel` slice; `promoted` / `operator` content is untouched and unremovable by reconcile.
- **Idempotent.** Same manifest + same live state → **zero events** (content hashes match). Every test suite must include a re-run-yields-no-events assertion.
- **It's still events.** Every mutation is a `kb_events` row fired through the substrate functions (event-as-primary holds). The PUT desired-state is the contract; events are its consequence.
- **Writes route through the backend trait.** Surfaces (handler, CLI) dispatch ONE operations command per inbound call via `Backend`; never call services or `sqlx::query!` directly from a surface for a write. (CLAUDE.md "Service layer owns SQL; surfaces dispatch through DbBackend".)
- **Typed structs over inline JSON.** No `serde_json::json!()` for known-structure data. Wire types live in `temper-core` with ts-rs derives; both sides share them. (CLAUDE.md Code Quality Rules.)
- **Params structs over >5 args.** (CLAUDE.md.)
- **Auth before writes.** The admin gate runs before any mutation.
- **sqlx cache discipline.** New substrate (`temper_next`-namespace) queries → `cargo make prepare-next` (per-crate `crates/temper-next/.sqlx`, `search_path=temper_next`). New temper-api test-target queries → `cargo make prepare-api`. New e2e queries → `cargo make prepare-e2e`. Never `cargo sqlx prepare --workspace` (clobbers per-crate caches). All `cargo make` tasks set `SQLX_OFFLINE=true`.
- **Reserved ids (do not re-derive).** L0 cogmap `00000000-0000-0000-0005-000000000001`; L0 telos resource `00000000-0000-0000-0005-000000000002`; root team slug `temper-system`; system actor = profile `handle='system'` + entity `name='system'`. The L0 birth migration (`20260625000001_l0_kernel_cogmap.sql`) already created all of these.
- **Run `cargo make check` before every commit** (fmt + clippy `-D warnings` + machete + TS). Per-task: focused test + crate suite + check. Full-workspace nextest only at PR-prep.

**Grounding tags used below:** `CONFORM` = uses an existing verified API unchanged; `EXTEND` = adds to an existing module following its pattern; `AMEND` = deviates from the spec (justified inline). Every named signature was grep-verified against the tree on 2026-06-25.

---

## File Structure

**New files:**
- `crates/temper-core/src/operations/` — extend `commands.rs` (the `ReconcileCognitiveMap` command) and `backend.rs` (the trait method); new `crates/temper-core/src/types/reconcile.rs` (wire types: request entries + outcome).
- `crates/temper-next/src/writes.rs` — EXTEND with cogmap-homed wrappers (`create_kernel_resource`, `set_facet`, `mutate_block`, `assert_kernel_edge`, `fold_kernel_edge`).
- `crates/temper-next/src/readback/mod.rs` — EXTEND with `kernel_slice` (the diff read).
- `crates/temper-api/src/backend/db_backend.rs` — EXTEND with `reconcile_cognitive_map` impl (the diff+plan+apply orchestration + invocation envelope + mutex).
- `crates/temper-api/src/handlers/cognitive_maps.rs` — NEW handler module (`PUT /api/cognitive-maps/{id}`).
- `crates/temper-api/src/handlers/mod.rs` + `crates/temper-api/src/routes.rs` — register the module + route.
- `crates/temper-api/src/backend/db_backend.rs` (authz helper) — `require_cogmap_write_admin`.
- `crates/temper-client/src/` — `reconcile_cognitive_map` client method (PUT).
- `crates/temper-cli/src/commands/` + `src/actions/` — `temper admin reconcile-cogmap` command + action (read manifest → embed → PUT).
- `schema-artifact/manifests/l0-kernel.yaml` — the committed authored L0 manifest (relocated/derived from the workbench fixture, with `provenance: kernel`).
- Tests: `crates/temper-next/tests/kernel_reconcile_read.rs`, `crates/temper-next/tests/kernel_homed_writes.rs` (artifact-tests group); `crates/temper-api/tests/reconcile_cogmap_test.rs` (test-db); `tests/e2e/tests/reconcile_cogmap_e2e.rs`.

**Modified files:** see each task's `Files:` block.

---

## Decisions settled in this plan (sensible defaults; not user-gated)

1. **Manifest shape = the seed-DSL resource/edge shape** (the workbench fixture `crates/temper-next/tests/fixtures/seeds/l0-kernel.yaml` is the authored source). The committed production manifest adds a `provenance: kernel` facet to every entry (O1) and supports a top-level `fold:` tombstone list (O3). It is *text* (bodies are prose); the CLI embeds at send time. JSON-schema snapshot-tested like seeds/scenarios (Task 8).
2. **Read + cogmap-homed write wrappers live in `temper-next`** (it owns the substrate SQL + the `temper_next` `.sqlx` cache). The **orchestration** (diff/plan/apply loop, invocation envelope, mutex) lives in `temper-api`'s `DbBackend` as the `reconcile_cognitive_map` command impl — it composes `temper_next` reads/writes, never inlines substrate SQL.
3. **The mutex** is the open `admin_reconcile` invocation: before opening, the orchestration checks for an existing `status='open'` `admin_reconcile` invocation on the kernel cogmap and no-ops/errors if present (§7). (A `pg_advisory_xact_lock` is the documented alternative if the open-row check proves racy; the plan uses the invocation-row check first.)
4. **Authz = structural root-team rule + L0 whitelist.** `require_cogmap_write_admin`: if the target cogmap is joined to the gating/root team (`kb_system_settings.gating_team_slug`, i.e. `temper-system`), require `is_system_admin`. The L0 uuid is covered by this rule (it's root-team-joined) — no separate allowlist needed, but the L0 uuid is asserted in a test as the canonical case.
5. **Diff key = `origin_uri`; change signal = body `content_hash`** (substrate `body_hash` merkle over chunks). Facet/edge deltas are asserted/folded idempotently regardless (assert is idempotent; fold needs a tombstone).

---

## Task 1: Reconcile wire types + operations command + Backend trait method

Establishes the typed contract every other task consumes. No behavior yet — types, command struct, trait method signature (returning `unimplemented` until Task 4), and serde/ts-rs round-trips.

**Files:**
- Create: `crates/temper-core/src/types/reconcile.rs`
- Modify: `crates/temper-core/src/types/mod.rs` (add `pub mod reconcile;`)
- Modify: `crates/temper-core/src/operations/commands.rs` (add `ReconcileCognitiveMap`)
- Modify: `crates/temper-core/src/operations/backend.rs` (add trait method)
- Modify: `crates/temper-core/src/operations/mod.rs` (export the command)
- Test: `crates/temper-core/src/types/reconcile.rs` (inline `#[cfg(test)] mod tests`)

**Interfaces:**
- Produces (consumed by Tasks 4, 6, 7):
  - `ReconcileEntry { origin_uri: String, title: String, doc_type: String, body: String, content_hash: String, chunks_packed: String, facets: serde_json::Value, edges: Vec<ReconcileEdge> }` — one kernel landmark, **pre-embedded** (`chunks_packed` is the CLI's `compute_body_chunks` output; `content_hash` is its sibling). `facets` is the multi-key JSONB object (e.g. `{ "provenance": "kernel", "layer": "concept" }`), matching the seed YAML `facets.values` shape (CONFORM with `SeedAction::FacetSet { values }`).
  - `ReconcileEdge { to_origin_uri: String, kind: String, polarity: String, label: Option<String>, weight: f64 }` — an outgoing edge keyed by the target's `origin_uri` (resolved server-side to the target resource id).
  - `ReconcileTombstone { origin_uri: String }` (resource removal) and `ReconcileEdgeTombstone { from_origin_uri: String, to_origin_uri: String, kind: String }` (edge removal).
  - `ReconcileCogmapRequest { entries: Vec<ReconcileEntry>, fold_resources: Vec<ReconcileTombstone>, fold_edges: Vec<ReconcileEdgeTombstone> }` — the PUT body.
  - `ReconcileOutcome { created: u32, updated: u32, folded: u32, unchanged: u32 }` — the run result + `kb_invocations.outcome`.
  - `ReconcileCognitiveMap { cogmap_id: Uuid, request: ReconcileCogmapRequest, origin: Surface }` (operations command).
  - `Backend::reconcile_cognitive_map(&self, cmd: ReconcileCognitiveMap) -> Result<CommandOutput<ReconcileOutcome>, TemperError>`.

- [ ] **Step 1: Write the failing test** — `crates/temper-core/src/types/reconcile.rs`, an inline test module asserting serde round-trip stability and that the outcome sums:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_through_json() {
        let req = ReconcileCogmapRequest {
            entries: vec![ReconcileEntry {
                origin_uri: "temper://kernel/concept/cogmap".into(),
                title: "cogmap".into(),
                doc_type: "kernel_landmark".into(),
                body: "A cognitive map: a bounded, telos-governed view.".into(),
                content_hash: "deadbeef".into(),
                chunks_packed: "[]".into(),
                facets: serde_json::json!({ "provenance": "kernel", "layer": "concept" }),
                edges: vec![ReconcileEdge {
                    to_origin_uri: "temper://kernel/concept/telos".into(),
                    kind: "express".into(),
                    polarity: "forward".into(),
                    label: Some("governs".into()),
                    weight: 1.0,
                }],
            }],
            fold_resources: vec![],
            fold_edges: vec![],
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: ReconcileCogmapRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
    }

    #[test]
    fn outcome_default_is_all_zero() {
        let o = ReconcileOutcome::default();
        assert_eq!((o.created, o.updated, o.folded, o.unchanged), (0, 0, 0, 0));
    }
}
```

- [ ] **Step 2: Run it to confirm it fails** — `cargo nextest run -p temper-core reconcile`. Expected: FAIL (module/types not defined).

- [ ] **Step 3: Write the types** in `crates/temper-core/src/types/reconcile.rs`. Mirror the derive stack used by sibling wire types (grep an existing `temper-core/src/types/ingest.rs` struct for the exact cfg-gated derive set):

```rust
//! Wire types for L0 cognitive-map content reconciliation (see
//! docs/superpowers/specs/2026-06-25-l0-delivery-and-lifecycle-design.md). The PUT body is a
//! PRE-EMBEDDED desired-state manifest: the CLI embeds (compute_body_chunks) before sending, so the
//! server stays embed-free on the request path.
use serde::{Deserialize, Serialize};

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export))]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReconcileEntry {
    pub origin_uri: String,
    pub title: String,
    pub doc_type: String,
    pub body: String,
    pub content_hash: String,
    pub chunks_packed: String,
    pub facets: serde_json::Value,
    #[serde(default)]
    pub edges: Vec<ReconcileEdge>,
}
// ... ReconcileEdge, ReconcileTombstone, ReconcileEdgeTombstone, ReconcileCogmapRequest with the
// same derive stack; ReconcileOutcome additionally derives Default.
```

> Grounding note: copy the exact `#[cfg_attr(...)]` derive lines from a neighbor in `temper-core/src/types/` rather than guessing which features gate ts-rs/utoipa/schemars — the repo gates them as `typescript` / `web-api` / `mcp` (CLAUDE.md Feature Flags). `ReconcileOutcome` adds `#[derive(Default)]`.

- [ ] **Step 4: Add the command + trait method.** In `commands.rs` add `ReconcileCognitiveMap` (CONFORM: every command carries `origin: Surface`; derive `Debug, Clone, PartialEq, Serialize, Deserialize` like `AssertRelationship` at `commands.rs:120`). In `backend.rs` add to the `Backend` trait (CONFORM: `Result<CommandOutput<T>, TemperError>` return shape):

```rust
async fn reconcile_cognitive_map(
    &self,
    cmd: ReconcileCognitiveMap,
) -> Result<CommandOutput<crate::types::reconcile::ReconcileOutcome>, TemperError>;
```

Export `ReconcileCognitiveMap` from `operations/mod.rs` (CONFORM with the existing re-export block at `mod.rs:25-29`).

- [ ] **Step 5: Run tests + check** — `cargo nextest run -p temper-core reconcile` (PASS), then `cargo make generate-ts-types` (confirms ts-rs export compiles), then `cargo make check`. Note: this step makes `DbBackend` fail to compile (unimplemented trait method) — that is expected and resolved in Task 4; to keep the tree compiling between tasks, add a temporary `reconcile_cognitive_map` impl in `db_backend.rs` returning `Err(TemperError::Api("not yet implemented".into()))` and delete it in Task 4.

- [ ] **Step 6: Commit** — `git add crates/temper-core crates/temper-api/src/backend/db_backend.rs && git commit -m "feat(reconcile): L0 reconcile wire types + Backend::reconcile_cognitive_map signature"`

---

## Task 2: `temper_next::readback::kernel_slice` — the diff read

The server reads L0's current `provenance: kernel` resources keyed by `origin_uri`, with each one's body `body_hash` and current facets, so the orchestration can diff against the incoming entries.

**Files:**
- Modify: `crates/temper-next/src/readback/mod.rs` (add `kernel_slice` + `KernelSliceRow`)
- Test: `crates/temper-next/tests/kernel_reconcile_read.rs` (NEW; `#![cfg(feature = "artifact-tests")]`, `temper-next-write` nextest group — it seeds + owns the namespace)

**Interfaces:**
- Produces: `pub struct KernelSliceRow { pub resource_id: Uuid, pub origin_uri: String, pub body_hash: Option<String>, pub facets: serde_json::Value }` and `pub async fn kernel_slice(pool: &PgPool, cogmap_id: Uuid) -> Result<Vec<KernelSliceRow>>`.
- Consumes: nothing new (CONFORM: mirrors the existing `readback::resource_row` query shape at `readback/mod.rs:507`, which already exposes `body_hash`).

- [ ] **Step 1: Write the failing test** — seed the L0 slice (reuse the workbench `l0-kernel.yaml` seed loader, or a minimal inline seed homing two `provenance: kernel` resources to a cogmap), then assert `kernel_slice` returns exactly those two keyed by `origin_uri`, that a `provenance: promoted` resource homed to the same cogmap is **excluded**, and that `body_hash` is populated:

```rust
#![cfg(feature = "artifact-tests")]
//! Reconcile diff-read: kernel_slice returns only provenance:kernel resources homed to the cogmap,
//! keyed by origin_uri, carrying body_hash. Owns the temper_next namespace (resets + seeds).
mod common; // the temper-next-write harness (resets namespace, seeds 01+02 functions)

#[tokio::test]
async fn kernel_slice_returns_only_kernel_provenance() {
    let pool = common::reset_and_pool().await; // mirror the existing write-path test setup
    // seed: cogmap C; resource A (provenance:kernel), resource B (provenance:promoted), both homed to C
    // ... use loader/fire helpers as the existing artifact tests do ...
    let rows = temper_next::readback::kernel_slice(&pool, cogmap_id).await.unwrap();
    let uris: Vec<_> = rows.iter().map(|r| r.origin_uri.as_str()).collect();
    assert!(uris.contains(&"temper://kernel/concept/cogmap"));
    assert!(!uris.iter().any(|u| u.contains("/promoted/")));
    assert!(rows.iter().all(|r| r.body_hash.is_some()));
}
```

> Grounding note: copy the exact namespace-reset + seed harness from an existing `temper-next-write` test (e.g. `crates/temper-next/tests/scenario_load.rs` and its `common` module) — do not invent a new harness.

- [ ] **Step 2: Run it to confirm it fails** — `cargo make test-next` filtered: the task's nextest invocation is `DATABASE_URL=...search_path...%3Dtemper_next,public cargo nextest run -p temper-next --features artifact-tests -E 'test(kernel_slice_returns_only_kernel_provenance)'` (use `cargo make test-next` which sets the search_path URL; see CLAUDE.md). Expected: FAIL (`kernel_slice` not found).

- [ ] **Step 3: Implement `kernel_slice`.** Add to `readback/mod.rs`. The query joins visible kernel resources homed to the cogmap with their `provenance` property and `body_hash`. Use a `sqlx::query!` macro (temper_next namespace) — base it on the provenance-filter sketch validated during grounding:

```rust
pub struct KernelSliceRow {
    pub resource_id: Uuid,
    pub origin_uri: String,
    pub body_hash: Option<String>,
    pub facets: serde_json::Value,
}

/// All `provenance: kernel` resources homed to `cogmap_id`, keyed (by caller) on `origin_uri`,
/// with the current body merkle (`body_hash`) and the merged facet object. The reconcile diff source.
pub async fn kernel_slice(pool: &PgPool, cogmap_id: Uuid) -> Result<Vec<KernelSliceRow>> {
    // SELECT r.id, r.origin_uri, <body_hash subquery as in resource_row>,
    //        <facets: jsonb_object_agg of non-folded properties for this resource> AS facets
    //   FROM kb_resources r
    //   JOIN kb_resource_homes h ON h.resource_id = r.id
    //                           AND h.anchor_table = 'kb_cogmaps' AND h.anchor_id = $1
    //   JOIN kb_properties prov ON prov.owner_table='kb_resources' AND prov.owner_id=r.id
    //                          AND prov.property_key='provenance' AND NOT prov.is_folded
    //                          AND prov.property_value #>> '{}' = 'kernel'
    //  ORDER BY r.origin_uri;
    // (verify the exact body_hash expression against readback::resource_row:507; verify property
    //  column names against `\d kb_properties` per the verify-columns-against-live-DB discipline.)
    todo!("query per the comment; map rows to KernelSliceRow")
}
```

> Grounding note: confirm `kb_properties` columns (`owner_table`, `owner_id`, `property_key`, `property_value`, `is_folded`) against the live DB (`psql \d kb_properties`) before finalizing — per repo discipline, not the migration text. The `body_hash` expression must match `resource_row`'s exactly so the diff compares like-for-like.

- [ ] **Step 4: Regenerate the temper_next cache + run** — `cargo make prepare-next` (the new macro query needs the `temper_next` cache), then `cargo make test-next -E 'test(kernel_slice_returns_only_kernel_provenance)'`. Expected: PASS.

- [ ] **Step 5: Run the temper-next-write group + check** — `cargo make test-next` (full write-path group: must stay green), `cargo make check`.

- [ ] **Step 6: Commit** — `git add crates/temper-next && git commit -m "feat(reconcile): readback::kernel_slice (provenance:kernel diff read)"`

---

## Task 3: Cogmap-homed mutation wrappers in `temper_next::writes`

The existing `writes` wrappers home resources/edges to a **context**; reconcile homes them to the **cogmap**. Add thin public wrappers (callable from temper-api) that fire the same `SeedAction`s the loader/runner fire, but cogmap-homed, plus facet + block-mutate wrappers (currently fired raw).

**Files:**
- Modify: `crates/temper-next/src/writes.rs`
- Test: `crates/temper-next/tests/kernel_homed_writes.rs` (NEW; `#![cfg(feature = "artifact-tests")]`, `temper-next-write` group)

**Interfaces:**
- Produces (consumed by Task 4):
  - `create_kernel_resource(pool, KernelCreateParams) -> Result<ResourceId>` where `KernelCreateParams { cogmap: CogmapId, title, origin_uri, doc_type, body, chunks: Option<Vec<IncomingChunk>>, owner: ProfileId, emitter: EntityId }` — fires `SeedAction::ResourceCreate { home: AnchorRef::cogmap(cogmap), .. }` (EXTEND: existing `create_resource` hardcodes `AnchorRef::context`). Honors client chunks via `prepare_block_from_chunks`, else server-embeds via `prepare_block` (CONFORM with `create_resource:104-108`).
  - `set_facet(pool, resource: ResourceId, values: &serde_json::Value, weight: f64, emitter: EntityId) -> Result<()>` — wraps `SeedAction::FacetSet` (EXTEND: currently raw in `loader.rs:131`).
  - `mutate_block(pool, block: BlockId, chunks: &[PreparedChunk], emitter: EntityId) -> Result<()>` — wraps `SeedAction::BlockMutate` (EXTEND: raw in `runner.rs:307`).
  - `assert_kernel_edge(pool, KernelEdgeParams) -> Result<EdgeId>` with `home: EdgeHome::Cogmap(cogmap)` (EXTEND: `assert_relationship` hardcodes `EdgeHome::Context`; `EdgeHome::Cogmap` already exists, used by `runner.rs:236`).
  - `fold_kernel_edge` — reuse existing `fold_relationship(pool, edge, reason, emitter)` (CONFORM, edge id already resolved); no new wrapper needed — listed here only so Task 4 knows to call the existing one.

- [ ] **Step 1: Write the failing test** — create a kernel resource homed to a cogmap with client-supplied chunks, set a `provenance: kernel` facet, assert a cogmap-homed edge to a second kernel resource; read back via `kernel_slice` (Task 2) and `neighbors` to confirm home + facet + edge landed:

```rust
#![cfg(feature = "artifact-tests")]
mod common;
use temper_next::writes;

#[tokio::test]
async fn create_kernel_resource_homes_to_cogmap_with_facet_and_edge() {
    let pool = common::reset_and_pool().await;
    // genesis a cogmap (reuse cogmap_genesis helper) -> cogmap, emitter, owner
    let a = writes::create_kernel_resource(&pool, writes::KernelCreateParams {
        cogmap, title: "cogmap", origin_uri: "temper://kernel/concept/cogmap",
        doc_type: "kernel_landmark", body: "A cognitive map.", chunks: None, owner, emitter,
    }).await.unwrap();
    writes::set_facet(&pool, a, &serde_json::json!({"provenance":"kernel","layer":"concept"}), 1.0, emitter).await.unwrap();
    let slice = temper_next::readback::kernel_slice(&pool, cogmap.uuid()).await.unwrap();
    assert_eq!(slice.len(), 1);
    assert_eq!(slice[0].origin_uri, "temper://kernel/concept/cogmap");
}
```

- [ ] **Step 2: Run it to confirm it fails** — `cargo make test-next -E 'test(create_kernel_resource_homes_to_cogmap_with_facet_and_edge)'`. Expected: FAIL (`create_kernel_resource` not found).

- [ ] **Step 3: Implement the wrappers** in `writes.rs`. Mirror `create_resource:104` exactly but with `home: AnchorRef::cogmap(p.cogmap)` and `originator: None` (kernel content's originator COALESCEs to owner = system). For `set_facet`/`mutate_block`/`assert_kernel_edge`, lift the raw `fire(&mut tx, SeedAction::X{..})` shapes verbatim from `loader.rs:131` / `runner.rs:307` / `runner.rs:229` into `pub async fn` wrappers with `begin_scoped` + `tx.commit()` (CONFORM with the wrapper pattern already in `writes.rs`).

- [ ] **Step 4: Regenerate cache + run** — `cargo make prepare-next` (only if the wrappers add new macro queries; the SeedAction fires reuse cached function calls, so this may be a no-op — run it to be safe), then `cargo make test-next -E 'test(create_kernel_resource_homes_to_cogmap_with_facet_and_edge)'`. Expected: PASS.

- [ ] **Step 5: temper-next-write group + check** — `cargo make test-next`, `cargo make check`.

- [ ] **Step 6: Commit** — `git add crates/temper-next && git commit -m "feat(reconcile): cogmap-homed writes wrappers (create_kernel_resource, set_facet, mutate_block, assert_kernel_edge)"`

---

## Task 4: The reconciler — `DbBackend::reconcile_cognitive_map` (diff + plan + apply + envelope + mutex)

The heart of the feature: orchestrate one idempotent reconcile run inside an `admin_reconcile` `kb_invocations` envelope. No HTTP/authz yet (Tasks 5–6); this is the backend command, tested directly with `test-db`.

**Files:**
- Modify: `crates/temper-api/src/backend/db_backend.rs` (replace the Task-1 stub with the real impl)
- Test: `crates/temper-api/tests/reconcile_cogmap_test.rs` (NEW; `#![cfg(feature = "test-db")]`)

**Interfaces:**
- Consumes: `kernel_slice` (Task 2), `create_kernel_resource`/`set_facet`/`mutate_block`/`assert_kernel_edge`/`fold_relationship` (Task 3), `open_invocation`/`close_invocation` (CONFORM, `writes.rs:368/392`), `unpack_incoming_chunks` (CONFORM, `db_backend.rs:90` — make it visible to the impl or duplicate the 3-line body), the system entity/owner lookup (CONFORM: the `handle='system'` / `name='system'` lookup from the L0 birth migration).
- Produces: a working `reconcile_cognitive_map` returning `CommandOutput<ReconcileOutcome>`.

**Algorithm (apply inside one transaction-scoped run; each `fire` is already tx-scoped per call — the envelope serializes them logically):**
1. **Mutex:** query `kb_invocations` for an open `admin_reconcile` on `cogmap_id`; if found, return `TemperError::Conflict` ("reconcile already in progress"). (CONFORM: the table has `status`, `trigger_kind`, `originating_cogmap_id`.)
2. **Open** the invocation: `open_invocation(pool, OpenParams { trigger_kind: "admin_reconcile".into(), originating: cogmap_id, parent: None, scoped_entity: system_entity, emitter: system_entity })` (CONFORM; `admin_reconcile` needs **no schema change** — `trigger_kind` is free TEXT). AMEND note: this is a top-level (non-delegated, `parent: None`) invocation, so the delegation gate is not exercised.
3. **Read** `live = kernel_slice(pool, cogmap_id)` → index by `origin_uri`.
4. **Diff entries:** for each incoming `ReconcileEntry`:
   - absent in `live` → **create**: `create_kernel_resource(chunks = Some(unpack(entry.chunks_packed)))`, then `set_facet(entry.facets)` (ensure `provenance: kernel` present), then assert its `edges`; `created += 1`.
   - present, `entry.content_hash` != live `body_hash` → **update**: resolve the body block id for the resource, `mutate_block(prepared chunks)`; re-assert facets (idempotent) + edges; `updated += 1`.
   - present, hashes equal → re-assert facets + edges (idempotent, fires zero events when unchanged) → `unchanged += 1`.
5. **Apply tombstones:** for each `fold_resources` entry present in `live`, `delete_resource` (soft) ; for each `fold_edges`, resolve the edge + `fold_relationship`; `folded += count`. (O3: absence alone never folds.)
6. **Close** the invocation: `close_invocation(pool, inv, cogmap_id, Disposition::Completed, serde_json::to_value(&outcome)?, system_entity)`; on any error path, close with `Disposition::Failed` (CONFORM: `Disposition` enum + `outcome` jsonb).
7. Return `CommandOutput::new(outcome)`.

- [ ] **Step 1: Write the failing tests** (one file, several `#[sqlx::test]` cases — `test-db`). The four invariants the Global Constraints demand:

```rust
#![cfg(feature = "test-db")]
//! Reconcile is additive-only, provenance-scoped, and idempotent. Drives DbBackend directly.

// (a) first delivery: empty L0 slice + N entries -> created=N, slice now has N
// (b) idempotency: re-run the SAME request -> created=0, updated=0, unchanged=N, and assert
//     NO new kb_events rows fired between the two runs (snapshot event count).
// (c) update: change one entry's body (new content_hash + chunks) -> updated=1, others unchanged;
//     the live body_hash now matches the new content_hash.
// (d) provenance isolation: a promoted resource homed to the cogmap is untouched (still present,
//     no events), AND it cannot be folded by absence.
// (e) explicit fold: a fold_resources tombstone for a present kernel resource -> folded=1, gone from slice.
// (f) mutex: open an admin_reconcile invocation manually, then call reconcile -> Err(Conflict).
```

Write each as a concrete `#[sqlx::test]` using the L0 birth migration's reserved cogmap (or a freshly-genesis'd cogmap joined to a team), building requests with the Task-1 types. (Use `tests/e2e` fixtures only if JWT is needed — here it is not.)

- [ ] **Step 2: Run to confirm failure** — `cargo nextest run -p temper-api --features test-db --test reconcile_cogmap_test`. Expected: FAIL (stub returns `Api("not yet implemented")`).

- [ ] **Step 3: Implement** `reconcile_cognitive_map` per the algorithm. Resolve the system entity/owner once at the top (CONFORM lookup). Keep all SQL in `temper_next` calls (Tasks 2–3) + the one mutex `sqlx::query!` (this is a temper-api query → public schema → `cargo make prepare-api`, *not* prepare-next). Build the `ReconcileOutcome`, close the envelope, return.

- [ ] **Step 4: Regenerate api cache + run** — `cargo make prepare-api` (the mutex query is a new temper-api test-reachable macro), then `cargo nextest run -p temper-api --features test-db --test reconcile_cogmap_test`. Expected: PASS (all of a–f).

- [ ] **Step 5: crate suite + check** — `cargo nextest run -p temper-api --features test-db` (scoped to integration test targets — never bare `-p temper-api`, it hangs per CLAUDE.md), `cargo make check`.

- [ ] **Step 6: Commit** — `git add crates/temper-api && git commit -m "feat(reconcile): DbBackend::reconcile_cognitive_map (diff+apply, admin_reconcile envelope + mutex, idempotent)"`

---

## Task 5: Authz helper — root-team-cogmap write gate

A reusable check the handler calls before dispatching: writing to a root-team-joined cogmap requires `is_system_admin`.

**Files:**
- Modify: `crates/temper-api/src/services/access_service.rs` (add `require_cogmap_write_admin`)
- Test: `crates/temper-api/tests/reconcile_cogmap_test.rs` (extend with authz cases) OR a focused `access_service` test — prefer extending the existing access test file if one exists (`grep -l access_service crates/temper-api/tests`).

**Interfaces:**
- Produces: `pub async fn require_cogmap_write_admin(pool: &PgPool, profile_id: Uuid, cogmap_id: Uuid) -> Result<(), ApiError>` — `Ok(())` if the cogmap is **not** root-team-joined (gate doesn't apply) OR the profile `is_system_admin`; `Err(ApiError::Forbidden)` otherwise. (CONFORM: reuses `access_service::is_system_admin:37` + reads `kb_system_settings.gating_team_slug` like `is_system_admin` does internally, joined through `kb_team_cogmaps`.)

- [ ] **Step 1: Write the failing test** — three cases: (a) L0 uuid + non-admin profile → `Forbidden`; (b) L0 uuid + admin (owner of `temper-system`) → `Ok`; (c) a non-root-team cogmap + non-admin → `Ok` (gate doesn't apply). Seed team membership via the same path `sync_system_membership` uses (set `kb_profiles.system_access`).

```rust
#[sqlx::test]
async fn l0_write_requires_system_admin(pool: PgPool) {
    // non-admin profile -> Forbidden on the L0 cogmap
    let r = access_service::require_cogmap_write_admin(&pool, non_admin, L0_COGMAP).await;
    assert!(matches!(r, Err(ApiError::Forbidden)));
    // admin -> Ok
    access_service::require_cogmap_write_admin(&pool, admin, L0_COGMAP).await.unwrap();
}
```

- [ ] **Step 2: Run to confirm failure** — `cargo nextest run -p temper-api --features test-db --test reconcile_cogmap_test l0_write_requires`. Expected: FAIL (fn not found).

- [ ] **Step 3: Implement** `require_cogmap_write_admin`: `SELECT EXISTS(SELECT 1 FROM kb_team_cogmaps tc JOIN kb_teams t ON t.id=tc.team_id JOIN kb_system_settings s ON t.slug=s.gating_team_slug WHERE tc.cogmap_id=$1)` → if false, `Ok(())`; if true, gate on `is_system_admin`. (Verify `kb_system_settings.gating_team_slug` column name against the live DB.)

- [ ] **Step 4: prepare-api + run** — `cargo make prepare-api`, then the focused test. Expected: PASS.

- [ ] **Step 5: crate suite + check** — `cargo nextest run -p temper-api --features test-db`, `cargo make check`.

- [ ] **Step 6: Commit** — `git add crates/temper-api && git commit -m "feat(reconcile): require_cogmap_write_admin (root-team-cogmap write gate)"`

---

## Task 6: `PUT /api/cognitive-maps/{id}` handler + route + client method

The HTTP surface. Admin-gated, dispatches the Task-4 command, returns the outcome. Plus the temper-client method the CLI (Task 7) calls.

**Files:**
- Create: `crates/temper-api/src/handlers/cognitive_maps.rs`
- Modify: `crates/temper-api/src/handlers/mod.rs` (add `pub mod cognitive_maps;`)
- Modify: `crates/temper-api/src/routes.rs` (register on the **gated** router, before the `require_system_access`/`require_auth` layers — see `routes.rs:84` for the `/api/ingest/{id}` PUT pattern)
- Modify: `crates/temper-api/src/openapi` (add the handler to the utoipa `ApiDoc` paths, mirroring an existing `#[utoipa::path]` registration)
- Modify: `crates/temper-client/src/` (add `reconcile_cognitive_map(cogmap_id, ReconcileCogmapRequest) -> Result<ReconcileOutcome>` — mirror an existing PUT method, e.g. the ingest-update client call)
- Test: `tests/e2e/tests/reconcile_cogmap_e2e.rs` (NEW; real Axum + DB + JWT)

**Interfaces:**
- Consumes: `Backend::reconcile_cognitive_map` (Task 4), `require_cogmap_write_admin` (Task 5), `AuthUser` extractor (CONFORM, `middleware/auth.rs`), `DbBackend::new(pool, ProfileId)` (CONFORM).
- Produces: the route + the client method.

- [ ] **Step 1: Write the failing e2e test** — drive the production caller (real HTTP). Cases: (a) admin JWT PUTs a 2-entry request → 200, outcome `created=2`; (b) re-PUT identical → 200, `created=0, unchanged=2`; (c) non-admin JWT → 403 (or leak-safe 404 — match `show_resource`'s deny convention; pick 403 since the cogmap's existence is not secret for L0). Use the e2e JWKS fixtures + harness (`tests/e2e/tests/common/`).

```rust
// tests/e2e/tests/reconcile_cogmap_e2e.rs
#[tokio::test]
async fn admin_reconcile_l0_is_idempotent() {
    let h = common::harness_with_admin().await;
    let req = /* ReconcileCogmapRequest with 2 pre-embedded entries */;
    let out1: ReconcileOutcome = h.put_json(&format!("/api/cognitive-maps/{L0}"), &req).await;
    assert_eq!(out1.created, 2);
    let out2: ReconcileOutcome = h.put_json(&format!("/api/cognitive-maps/{L0}"), &req).await;
    assert_eq!((out2.created, out2.unchanged), (0, 2));
}
```

- [ ] **Step 2: Run to confirm failure** — `cargo make test-e2e-embed -E 'test(admin_reconcile_l0_is_idempotent)'` (embed feature: the e2e harness builds pre-embedded entries via the client's `compute_body_chunks`, which needs ONNX). Expected: FAIL (404 — no route).

- [ ] **Step 3: Implement the handler** (CONFORM with `ingest::update:79`):

```rust
pub async fn reconcile(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(cogmap_id): Path<Uuid>,
    Json(request): Json<ReconcileCogmapRequest>,
) -> ApiResult<Json<ReconcileOutcome>> {
    access_service::require_cogmap_write_admin(&state.pool, auth.0.profile.id, cogmap_id).await?;
    let cmd = ReconcileCognitiveMap { cogmap_id, request, origin: Surface::ApiHttp };
    let backend = DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id));
    let out = backend.reconcile_cognitive_map(cmd).await.map_err(ApiError::from)?;
    Ok(Json(out.value))
}
```

Register the route: `.route("/api/cognitive-maps/{id}", put(handlers::cognitive_maps::reconcile))` on the gated router. Add `#[utoipa::path(put, path = "/api/cognitive-maps/{id}", ...)]` and wire into `ApiDoc`. Add the client method.

- [ ] **Step 4: prepare-e2e + run** — `cargo make prepare-e2e` (if the e2e test adds macro queries), then `cargo make test-e2e-embed -E 'test(admin_reconcile_l0_is_idempotent)'` and the non-admin case. Expected: PASS.

- [ ] **Step 5: e2e suite + check** — `cargo make test-e2e-embed` (this path needs the embed feature), `cargo make check`.

- [ ] **Step 6: Commit** — `git add crates/temper-api crates/temper-client tests/e2e && git commit -m "feat(reconcile): PUT /api/cognitive-maps/{id} handler + client method (admin-gated, idempotent e2e)"`

---

## Task 7: `temper admin reconcile-cogmap` operator command

The client surface: read the committed manifest, embed each entry (client-side), build the request, PUT.

**Files:**
- Create: `crates/temper-cli/src/commands/admin.rs` (or extend an existing admin command group — `grep -rn "admin" crates/temper-cli/src/commands/` first)
- Create: `crates/temper-cli/src/actions/reconcile.rs` (manifest → request translation)
- Modify: `crates/temper-cli/src/commands/mod.rs` + the clap command tree
- Test: `crates/temper-cli/src/actions/reconcile.rs` inline tests (manifest→request, `#[cfg(feature = "embed")]`) + `tests/e2e/tests/reconcile_cogmap_e2e.rs` (extend: drive the CLI binary end-to-end)

**Interfaces:**
- Consumes: `compute_body_chunks` (CONFORM, `actions/ingest.rs:35`, `feature = "embed"`), the manifest YAML model (Task 8), the client method (Task 6).
- Produces: `temper admin reconcile-cogmap <cogmap-ref> --manifest <path>` → prints the `ReconcileOutcome`.

- [ ] **Step 1: Write the failing test** — `manifest_to_request` reads a small YAML manifest and produces a `ReconcileCogmapRequest` whose entries carry non-empty `content_hash` + `chunks_packed` (proving the embed step ran) and the `provenance: kernel` facet:

```rust
#[cfg(feature = "embed")]
#[test]
fn manifest_to_request_embeds_and_tags_provenance() {
    let manifest = parse_manifest(SAMPLE_YAML).unwrap();
    let req = manifest_to_request(&manifest).unwrap();
    assert!(req.entries.iter().all(|e| !e.content_hash.is_empty()));
    assert!(req.entries.iter().all(|e| e.facets.get("provenance").is_some()));
}
```

- [ ] **Step 2: Run to confirm failure** — `cargo nextest run -p temper-cli --features embed manifest_to_request`. Expected: FAIL.

- [ ] **Step 3: Implement** `parse_manifest` (serde_yaml into the manifest model) + `manifest_to_request` (per entry: `compute_body_chunks(&entry.body)` → `content_hash` + `chunks_packed`; carry facets, ensuring `provenance: kernel`; map edges). Wire the clap command to resolve the cogmap ref (CONFORM: `temper_core::operations::parse_ref`), read `--manifest`, translate, call the client, print the outcome via the `output/` helpers (no raw ANSI; honor `--format`).

- [ ] **Step 4: Run** — `cargo nextest run -p temper-cli --features embed manifest_to_request`. Expected: PASS. Then add/extend the e2e CLI-drive case (`temper admin reconcile-cogmap` against the live harness) and run `cargo make test-e2e-embed`.

- [ ] **Step 5: crate suite + check** — `cargo nextest run -p temper-cli --features embed` (note the env-leak guard: new temper-cli test files route through `init_isolated_auth` per CLAUDE memory), `cargo make check`.

- [ ] **Step 6: Commit** — `git add crates/temper-cli tests/e2e && git commit -m "feat(reconcile): temper admin reconcile-cogmap (manifest -> client-embed -> PUT)"`

---

## Task 8: The committed L0 manifest + JSON-schema snapshot test

The authored kernel content as a committed production artifact (relocated/derived from the workbench fixture), with `provenance: kernel` on every entry, schema-snapshot-tested like seeds/scenarios.

**Files:**
- Create: `schema-artifact/manifests/l0-kernel.yaml` (the authored content — promote `crates/temper-next/tests/fixtures/seeds/l0-kernel.yaml`, adding `provenance: kernel` to every entry's facets alongside the existing `layer:` facet)
- Create: `schema-artifact/manifests/l0-kernel.schema.json` (snapshot)
- Test: a schema snapshot test mirroring the existing seed/scenario JSON-schema snapshot tests (`grep -rn "schema.json" crates/temper-next/tests` for the pattern; reuse the `scenario-schema` derive approach)

**Interfaces:**
- Consumes: the manifest model from Task 7.
- Produces: the shippable L0 manifest the operator passes to Task 7's command.

- [ ] **Step 1: Write the failing snapshot test** — assert the manifest parses into the model AND that the generated JSON Schema matches the committed `.schema.json` (drift guard). Mirror the seeds/scenarios snapshot test exactly.

- [ ] **Step 2: Run to confirm failure** — the test fails (no manifest/schema yet). Run the matching nextest target.

- [ ] **Step 3: Author the manifest** — relocate the six questions-with-context + four landmark categories (concept/invariant/reference/boundary) from the workbench seed, add `provenance: kernel` to each entry, keep the edges. Generate the schema snapshot.

- [ ] **Step 4: Run** — the snapshot test passes. Then a **smoke**: run Task 7's command against a local stack with this manifest and confirm `created = <N landmarks>`, then re-run and confirm `created=0` (idempotency on the real content).

- [ ] **Step 5: check** — `cargo make check`.

- [ ] **Step 6: Commit** — `git add schema-artifact && git commit -m "feat(reconcile): committed L0 kernel manifest + schema snapshot (provenance:kernel)"`

---

## Self-Review

**1. Spec coverage** (against `2026-06-25-l0-delivery-and-lifecycle-design.md`):
- §1 M1 desired-state reconcile → Task 4 (diff/plan/apply). ✓
- §2 embed-capable applier, no migration ONNX → Tasks 3 (wrappers reuse `prepare_block`/`prepare_block_from_chunks`) + 7 (client embed). **AMENDED to client-side embed** (Global Constraints; user-confirmed) — the "lift embed+mutate out of the test crate" sub-task the spec feared is **already done** (`writes::create_resource` is shipped in temper-next and called by temper-api), so it is *not* a task here. ✓ (with documented amendment)
- §3 O3 additive/update-only + O1 provenance → Task 4 steps 4–5 + the (d) isolation test. ✓
- §4 `PUT /api/cognitive-maps/{id}`, admin-gated, dispatch through backend → Tasks 5–6. ✓
- §5 audited via `kb_invocations admin_reconcile` → Task 4 steps 2/6. ✓
- §6 triggers (release step / operator command) → Task 7 (operator CLI); the release-pipeline step is a thin wrapper over Task 7, noted as deferred ops wiring (not code). ✓
- §7 concurrency (reconcile-vs-reconcile mutex; reconcile-vs-promotion safe-by-construction) → Task 4 step 1 (mutex) + (d) isolation test. ✓
- Components list → Tasks map 1:1 (manifest=Task 8, reconciler=Task 4, handler=Task 6, operator surface=Task 7, embed+mutate lift=already-done). ✓
- Deferred/out-of-scope (promotion-from-maps, `POST /api/events` admin door, embedding-model migration) → **not** in this plan, correctly. ✓

**2. Placeholder scan:** The `todo!()` in Task 2 Step 3 and the `// ...` in Task 1 Step 3 are *grounded scaffolds* with the exact query/derive source named, not blind placeholders — acceptable because the verified neighbor (`resource_row:507`, `types/ingest.rs`) is cited for the implementer to copy. No "add error handling"/"write tests for the above"/"TBD" placeholders. ✓

**3. Type consistency:** `ReconcileCogmapRequest` / `ReconcileEntry` / `ReconcileOutcome` / `ReconcileCognitiveMap` / `reconcile_cognitive_map` are spelled identically across Tasks 1, 4, 6, 7. `kernel_slice` / `KernelSliceRow` consistent across Tasks 2, 4. `create_kernel_resource` / `KernelCreateParams` / `set_facet` / `mutate_block` / `assert_kernel_edge` consistent across Tasks 3, 4. `require_cogmap_write_admin` consistent across Tasks 5, 6. ✓

**Open implementation risks flagged for the implementer (verify-don't-assume):**
- The exact `body_hash` SQL expression (Task 2) must byte-match `readback::resource_row` or the diff mis-fires every run. Confirm against `resource_row:507` and `psql \d kb_properties`.
- The body **block id** resolution for an *update* (Task 4 step 4 "resolve the body block id") — confirm how `update_resource`/`BlockMutate` find the block to mutate (it currently looks one up; grep `runner.rs:290-307`'s `Revise` path for the lookup).
- The deny convention (403 vs leak-safe 404) for a non-admin write — Task 6 picks 403 for L0 (not secret); confirm against the codebase's existing admin-endpoint deny (`access.rs:90` returns `Forbidden`). ✓ aligns.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-25-l0-delivery-and-lifecycle.md`. Two execution options:

1. **Subagent-Driven (recommended)** — fresh subagent per task, review between tasks, fast iteration. (Note this repo's hybrid-execution skill: Variant B consolidates review; the temper convention is per-task focused tests + `cargo make check`, full workspace at PR-prep.)
2. **Inline Execution** — execute tasks in this session with checkpoints for review.

Which approach?
