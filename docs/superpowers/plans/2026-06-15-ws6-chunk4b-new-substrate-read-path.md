# WS6 chunk 4b — new-substrate read path: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. NOTE: this repo's `hybrid-execution` skill governs the execute/review cadence — last session used Variant A (inline with targeted subagents); honor that gating over the default per-task reviewer cycle.

**Goal:** Make production reads answerable from the `temper_next.*` substrate behind `kb_backend_selection = next` (still gated OFF in prod), at the §9 data-parity floor — completing the read half of WS6 chunk 4.

**Architecture:** Approach A. A feature-gated `NextBackend` in temper-api implements `temper_core::operations::Backend`, delegating reads to `temper-next::readback` and stubbing writes (`NotImplemented`, 4c fills them). A small **read selector** mirrors `select_backend` for the service-direct read handlers (list / by_uri / get_content / get_meta / search) that bypass the trait by design. The `next` arm of `select_backend` constructs `NextBackend`. The parity floor for the full-row reads (`show` / `by_uri`) is the **migration-invariant field subset** — re-minted IDs and §7-dissolved fields (`slug`, `managed_hash`, `open_hash`) and synthesis-collapsed timestamps are declared non-invariants (mirrors chunk-3's ordering-non-invariant precedent).

**Tech Stack:** Rust (axum, sqlx runtime queries, async-trait), temper-next readback over `temper_next.*`, e2e harness (real Axum + Postgres), cargo-nextest, cargo-make.

---

## Grounding established (read before starting)

These facts were verified against the tree at plan time. They are load-bearing — do not re-derive, but do re-verify with the cited `file:line` if a step's premise looks wrong.

- **`ResourceRow`** (`crates/temper-core/src/types/resource.rs:18-54`) has 21 fields. Mapping to `temper_next.*`:
  - **Invariant (assert parity):** `origin_uri`, `title`, `is_active`, `context_name`, `doc_type_name`, `owner_handle`, `stage`, `mode`, `effort`, `seq`, `body_hash`.
  - **Non-invariant — re-minted identities** (synthesis assigns fresh UUIDs; `crates/temper-next/src/synthesis/bootstrap.rs:121,149` + `source.rs` join-by-`origin_uri`): `id`, `kb_context_id`, `owner_profile_id`, `originator_profile_id`.
  - **Non-invariant — `kb_doc_type_id`:** §7 keeps only the doc_type *name* (a `kb_properties` row); the UUID is resolved by a **transitional `public.kb_doc_types` name→id lookup** (valid during the migration window; `public` still exists pre-flip).
  - **Non-invariant — §7-dissolved:** `slug` (`KeyFate::Die`, `key_fate.rs:59`), `managed_hash`, `open_hash` (production `kb_resource_manifests` has no `temper_next` counterpart — the manifest dissolved into `kb_properties`).
  - **Non-invariant — synthesis-collapsed:** `created`, `updated` (sourced from genesis `occurred_at` = `now()` = synthesis tx-time; `readback/mod.rs:141-145`).
- **Source columns:** `temper_next.kb_resources(id,title,origin_uri,body_hash,is_active,created,updated)` (`schema-artifact/01_schema.sql:162-174`); `kb_resource_homes(resource_id,anchor_table,anchor_id,originator_profile_id,owner_profile_id)` (`:222-230`); `kb_contexts(id,name)` (`:105-113`); `kb_profiles(id,handle)` (`:71-77`); `kb_properties` keyed by `(owner_table='kb_resources', owner_id, property_key)` with `property_value` a JSON scalar extracted via `#>> '{}'` (see `readback::list`/`meta`).
- **temper-next has NO `chrono` dep and its sqlx lacks the `chrono` feature** (`crates/temper-next/Cargo.toml:24`). So `readback::resource_row` MUST NOT select `created`/`updated` into typed `DateTime`. NextBackend fills `ResourceRow.created`/`updated` with `chrono::Utc::now()` (temper-api already has chrono); they are non-invariants.
- **Chunk 3 built NO `readback::show`** — "meta reconstruction IS the show+meta parity" (`tests/parity_reads.rs:182`). 4b adds a new full-row `readback::resource_row`.
- **Backend trait** (`crates/temper-core/src/operations/backend.rs:45-72`): `create_resource`, `show_resource`, `update_resource`, `delete_resource`, `list_resources`, `search_resources`. `show_resource`→`CommandOutput<ResourceRow>`; `list_resources`→`CommandOutput<Vec<ResourceSummary>>`; `search_resources`→`CommandOutput<Vec<SearchHit>>`. `ResourceSummary{slug,doctype,context,title}`, `SearchHit{summary,score}`.
- **The gate seam (4a)** `crates/temper-api/src/backend/selection.rs`: `select_backend` returns `Box<dyn Backend>`; its `Next` arm currently `Err(NotImplemented)`. `BackendSelection::{Legacy,Next}`. `AppState.backend_selection` is read once at startup.
- **Which API reads route through the trait vs service-direct today:**
  - Trait (`select_backend`): `GET /api/resources/{id}` (show) `handlers/resources.rs:119-144`; create/update/delete; `meta::update_meta`.
  - Service-direct (need the new read selector): `list` `resources.rs:69`, `by_uri` `resources.rs:97` (**full `ResourceRow`**), `get_content` (body) `resources.rs:158`, `get_meta` `meta.rs:29`, `search` `handlers/search.rs:21`.
  - **`GET /api/graph/subgraph`** `handlers/graph.rs:35` is a **depth-2 concept-aggregator** read (`aggregator_subgraph`), NOT the §9 1-hop neighbor floor that `readback::neighbors` implements. It is **OUT of 4b's selector** — stays on legacy. The 1-hop neighbor parity is proven at the harness level (Task 11), not through an HTTP endpoint. Recorded so it isn't mistaken for a gap.
- **MCP read tools** `crates/temper-mcp/src/tools/`: `resources::get_resource` (`:413`, service-direct `get_visible`/`get_by_slug`+`get_content`), `resources::list_resources` (`:505`, `list_visible`), `search::search` (`:9`, `search_service::search` direct). These call `temper_api::services::*` directly.
- **temper-api ↔ temper-next deps:** temper-api does NOT yet depend on temper-next. temper-next dev-depends on temper-api (`parity_reads.rs` uses `temper_api::services`). A temper-api→temper-next *normal* dep + temper-next→temper-api *dev*-dep is a cargo-permitted cycle. temper-next pulls `temper-ingest` (onnx) as a **normal** dep (`Cargo.toml:23`), so the temper-api dep MUST be feature-gated like the existing `ingest-pipeline = ["dep:temper-ingest"]` (`temper-api/Cargo.toml:35`).
- **e2e gate test to mirror:** `tests/e2e/tests/backend_selection_gate.rs` (flips `kb_backend_selection` to `next` before `common::setup`, asserts the behavior flip).
- **Parity harness to extend:** `crates/temper-next/tests/parity_reads.rs` (`#![cfg(feature = "artifact-tests")]`, `common::seed_and_synthesize`, `ResolvedIds::load`).

---

## File structure

- **Modify** `docs/superpowers/specs/2026-06-15-ws6-chunk4b-new-substrate-read-path-design.md` — add the show/by_uri parity-floor amendment.
- **Modify** `crates/temper-api/Cargo.toml` — feature-gated `temper-next` dep + a `next-backend` feature.
- **Modify** `crates/temper-next/src/readback/mod.rs` — add `ResourceRowParity` + `resource_row()`.
- **Create** `crates/temper-api/src/backend/next_backend.rs` — `NextBackend` (feature-gated).
- **Create** `crates/temper-api/src/backend/read_selector.rs` — the service-direct read selector (feature-gated next arm).
- **Modify** `crates/temper-api/src/backend/{mod.rs,selection.rs}` — export + wire the `next` arm.
- **Modify** `crates/temper-api/src/handlers/{resources.rs,meta.rs,search.rs}` — route service-direct reads through the selector.
- **Modify** `crates/temper-mcp/src/tools/{resources.rs,search.rs}` — route MCP read tools through the selector.
- **Modify** `api/axum.rs`, `api/mcp.rs` — enable the `next-backend` feature in the deployed adapters (so the flip is config-only, not a recompile).
- **Modify** `crates/temper-next/tests/parity_reads.rs` — add `resource_row` parity + route a read through `NextBackend`.
- **Create** `tests/e2e/tests/backend_read_path_next.rs` — HTTP read-set-equality under `flag=next` vs `legacy`.

---

## Task 1: Amend the 4b spec with the show parity-floor decision

**Files:**
- Modify: `docs/superpowers/specs/2026-06-15-ws6-chunk4b-new-substrate-read-path-design.md`

- [ ] **Step 1: Insert a parity-floor subsection after the "Architecture (Approach A, continued)" section**

Add this block immediately before `## Proof gates`:

```markdown
### Full-row read parity floor (show / by_uri) — amendment 2026-06-15

Grounding during planning showed the spec's "reconstructed into `ResourceRow`" cannot be byte-identical: synthesis re-mints all identity UUIDs (resource `id`, `kb_context_id`, `owner_profile_id`, `originator_profile_id`; `bootstrap.rs`/`source.rs`), §7 dissolved `slug`/`managed_hash`/`open_hash` (no `kb_resource_manifests` in `temper_next`), and `created`/`updated` collapse to synthesis tx-time. So the full-row parity floor is the **migration-invariant field subset** — exactly the chunk-3 precedent that list/FTS *ordering* is not a migration invariant.

- **Invariant (asserted equal across `legacy` and `next`):** `origin_uri`, `title`, `is_active`, `context_name`, `doc_type_name`, `owner_handle`, `stage`, `mode`, `effort`, `seq`, `body_hash`.
- **Non-invariant (NOT asserted):** `id`, `kb_context_id`, `owner_profile_id`, `originator_profile_id` (re-minted); `slug`, `managed_hash`, `open_hash` (§7-dissolved); `created`, `updated` (synthesis-collapsed).
- `NextBackend::show_resource` fills the non-invariant fields with best-effort values: re-minted IDs as-is, `kb_doc_type_id` via a **transitional `public.kb_doc_types` name→id lookup** (valid during the migration window), `slug`/`managed_hash`/`open_hash` = `None`, `created`/`updated` = read-time `Utc::now()`.

**Graph scope:** `GET /api/graph/subgraph` is a depth-2 concept-aggregator read, not the §9 1-hop neighbor floor. It stays on legacy in 4b; 1-hop neighbor parity is proven at the parity-harness level, not via an HTTP endpoint.
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/specs/2026-06-15-ws6-chunk4b-new-substrate-read-path-design.md
git commit -m "WS6 4b: spec amendment — show/by_uri invariant-subset parity floor"
```

---

## Task 2: Add the feature-gated `temper-next` dependency to temper-api

**Files:**
- Modify: `crates/temper-api/Cargo.toml`

- [ ] **Step 1: Add the dep + feature**

In `[dependencies]`, add (mark optional so the feature gates it):

```toml
temper-next = { path = "../temper-next", optional = true }
```

In `[features]`, add the gate (it pulls temper-next, which transitively pulls onnx — same tradeoff as `ingest-pipeline`):

```toml
next-backend = ["dep:temper-next"]
```

- [ ] **Step 2: Verify temper-api still builds without the feature (gate is clean)**

Run: `cargo build -p temper-api`
Expected: PASS (temper-next not linked; nothing references it yet).

- [ ] **Step 3: Verify temper-api builds WITH the feature**

Run: `SQLX_OFFLINE=true cargo build -p temper-api --features next-backend`
Expected: PASS (temper-next + onnx link; no code uses it yet, but the dep resolves).

**CRITICAL (verified at build):** temper-next's `sqlx::query!` macros target the `temper_next` namespace and CANNOT validate live against the dev DB. So ANY temper-api build with `next-backend` MUST set `SQLX_OFFLINE=true` (raw `cargo build` without it fails with 58 "cannot infer type" errors from temper-next). `cargo make` tasks already set it; the deployed adapter build (Task 9) MUST set it too.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/Cargo.toml
git commit -m "WS6 4b: feature-gated temper-next dep on temper-api (next-backend)"
```

---

## Task 3: `readback::resource_row` — full-row reconstruction (the invariant subset)

**Files:**
- Modify: `crates/temper-next/src/readback/mod.rs`
- Test: `crates/temper-next/tests/parity_reads.rs` (added in Task 11; here add a unit-style assertion inside the new fn's doctest-free path via the harness in Task 11). For this task the failing test is the compile + a targeted parity test stub.

- [ ] **Step 1: Write the failing parity test for `resource_row`**

Append to `crates/temper-next/tests/parity_reads.rs` (it already has `common::seed_and_synthesize` + `ResolvedIds`):

```rust
/// §9 — full-row (`show`/`by_uri`) parity at the INVARIANT-FIELD subset. The non-invariant fields
/// (re-minted ids, §7-dissolved slug/hashes, synthesis-collapsed timestamps) are deliberately NOT
/// compared — see the 4b spec's parity-floor amendment.
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn resource_row_parity(pool: sqlx::PgPool) {
    use temper_api::services::resource_service;

    common::seed_and_synthesize(&pool).await;
    let ids = ResolvedIds::load(&pool).await.expect("ResolvedIds::load");

    for new_id in ids.new_ids() {
        let origin_uri = ids.origin_uri_for_new(new_id).expect("origin_uri").to_string();
        let old_id = ids.to_old(new_id).expect("maps back to prod");

        let rb = readback::resource_row(&pool, new_id).await.expect("readback::resource_row");
        // Production oracle: get_visible returns the full ResourceRow for the owner P1.
        let prod = resource_service::get_visible(&pool, common::fixture_ids::P1, old_id)
            .await
            .expect("prod get_visible");

        assert_eq!(rb.origin_uri, prod.origin_uri, "{origin_uri}: origin_uri");
        assert_eq!(rb.title, prod.title, "{origin_uri}: title");
        assert_eq!(rb.is_active, prod.is_active, "{origin_uri}: is_active");
        assert_eq!(rb.context_name, prod.context_name, "{origin_uri}: context_name");
        assert_eq!(rb.doc_type_name, prod.doc_type_name, "{origin_uri}: doc_type_name");
        assert_eq!(rb.owner_handle, prod.owner_handle, "{origin_uri}: owner_handle");
        assert_eq!(rb.stage, prod.stage, "{origin_uri}: stage");
        assert_eq!(rb.mode, prod.mode, "{origin_uri}: mode");
        assert_eq!(rb.effort, prod.effort, "{origin_uri}: effort");
        assert_eq!(rb.seq, prod.seq, "{origin_uri}: seq");
        assert_eq!(rb.body_hash, prod.body_hash, "{origin_uri}: body_hash");
    }
}
```

NOTE: confirm `common::fixture_ids::P1` exists; if the harness exposes the owner profile id under a different name, use that (grep `crates/temper-next/tests/common/` for the P1/owner const). If absent, add `pub const P1: Uuid = …;` mirroring the existing `RESOURCE_TASK` const.

- [ ] **Step 2: Run it to verify it fails (no `resource_row` yet)**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'test(resource_row_parity)'`
(Requires the artifact loaded + a live DB; same harness as the other parity tests.)
Expected: FAIL — `readback::resource_row` does not exist (compile error).

- [ ] **Step 3: Add `ResourceRowParity` + `resource_row()` to `readback/mod.rs`**

Add after the `meta()` function. `seq` is a JSON scalar — extract as text then parse to `i64` to avoid relying on JSONB→i64 sqlx coercion:

```rust
/// The migration-invariant subset of production's `ResourceRow`, reconstructed from `temper_next.*`
/// for the full-row reads (`show` / `by_uri`). Excludes the non-invariant fields by construction:
/// re-minted identity UUIDs (resource id / context id / profile ids), §7-dissolved
/// `slug`/`managed_hash`/`open_hash`, and the synthesis-collapsed `created`/`updated`. The caller
/// (`NextBackend::show_resource`) supplies those from elsewhere (re-minted ids verbatim, a transitional
/// `public.kb_doc_types` lookup for the doctype id, `None` for the dissolved fields, `Utc::now()` for
/// the timestamps). See the 4b spec parity-floor amendment.
///
/// `re_minted_id` / `re_minted_context_id` / `owner_profile_id` / `originator_profile_id` are carried
/// so the caller can populate `ResourceRow`'s non-optional UUID fields with the synthesized values —
/// they are NOT migration invariants and are never asserted in parity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceRowParity {
    /// The synthesized resource id (re-minted; not a parity invariant).
    pub re_minted_id: Uuid,
    /// The synthesized home-anchor context id (re-minted; not a parity invariant).
    pub re_minted_context_id: Uuid,
    /// The synthesized owner profile id (re-minted; not a parity invariant).
    pub owner_profile_id: Uuid,
    /// The synthesized originator profile id (re-minted; not a parity invariant).
    pub originator_profile_id: Uuid,
    /// Verbatim-carried, UNIQUE origin_uri (invariant).
    pub origin_uri: String,
    /// Resource title (invariant).
    pub title: String,
    /// Active flag (invariant; synthesis carries only active resources, so always true here).
    pub is_active: bool,
    /// Home context display name (invariant).
    pub context_name: String,
    /// Authoritative doctype name (invariant) — the `doc_type` property.
    pub doc_type_name: String,
    /// Owner profile handle (invariant).
    pub owner_handle: String,
    /// `temper-stage`, if present (invariant).
    pub stage: Option<String>,
    /// `temper-mode`, if present (invariant).
    pub mode: Option<String>,
    /// `temper-effort`, if present (invariant).
    pub effort: Option<String>,
    /// `temper-seq` parsed to i64, if present (invariant).
    pub seq: Option<i64>,
    /// Denormalized body merkle hash (invariant) — `kb_resources.body_hash`.
    pub body_hash: Option<String>,
}

/// Port of production's full-row read (`resource_service::get_visible` / `resolve_by_uri`, behind
/// `show` / `by_uri`) onto `temper_next.*`, at the §9 INVARIANT-FIELD floor. Joins the home (→ context
/// + owner profile), the `doc_type` property, and the workflow properties. Deliberately does NOT select
/// `created`/`updated` (temper-next's sqlx has no `chrono` feature, and they are synthesis-collapsed
/// non-invariants).
///
/// Read-only; no writes. Runtime, schema-qualified `sqlx::query` (NEVER the `query!` macros) — see the
/// module-level note.
pub async fn resource_row(pool: &PgPool, new_id: Uuid) -> Result<ResourceRowParity> {
    let row = sqlx::query(
        "SELECT r.id              AS re_minted_id,
                r.origin_uri,
                r.title,
                r.is_active,
                r.body_hash,
                c.id              AS re_minted_context_id,
                c.name            AS context_name,
                h.owner_profile_id,
                h.originator_profile_id,
                p.handle          AS owner_handle,
                dt.property_value #>> '{}' AS doc_type_name,
                st.property_value #>> '{}' AS stage,
                md.property_value #>> '{}' AS mode,
                ef.property_value #>> '{}' AS effort,
                sq.property_value #>> '{}' AS seq
           FROM temper_next.kb_resources r
           JOIN temper_next.kb_resource_homes h ON h.resource_id = r.id
           JOIN temper_next.kb_contexts c
             ON c.id = h.anchor_id AND h.anchor_table = 'kb_contexts'
           JOIN temper_next.kb_profiles p ON p.id = h.owner_profile_id
           JOIN temper_next.kb_properties dt
             ON dt.owner_table = 'kb_resources' AND dt.owner_id = r.id
            AND dt.property_key = 'doc_type'
           LEFT JOIN temper_next.kb_properties st
             ON st.owner_table = 'kb_resources' AND st.owner_id = r.id
            AND st.property_key = 'temper-stage'
           LEFT JOIN temper_next.kb_properties md
             ON md.owner_table = 'kb_resources' AND md.owner_id = r.id
            AND md.property_key = 'temper-mode'
           LEFT JOIN temper_next.kb_properties ef
             ON ef.owner_table = 'kb_resources' AND ef.owner_id = r.id
            AND ef.property_key = 'temper-effort'
           LEFT JOIN temper_next.kb_properties sq
             ON sq.owner_table = 'kb_resources' AND sq.owner_id = r.id
            AND sq.property_key = 'temper-seq'
          WHERE r.id = $1",
    )
    .bind(new_id)
    .fetch_one(pool)
    .await?;

    let seq_text: Option<String> = row.get("seq");
    let seq = match seq_text {
        Some(s) => Some(s.parse::<i64>().map_err(|e| {
            anyhow::anyhow!("temper-seq {s:?} is not an i64 for resource {new_id}: {e}")
        })?),
        None => None,
    };

    Ok(ResourceRowParity {
        re_minted_id: row.get("re_minted_id"),
        re_minted_context_id: row.get("re_minted_context_id"),
        owner_profile_id: row.get("owner_profile_id"),
        originator_profile_id: row.get("originator_profile_id"),
        origin_uri: row.get("origin_uri"),
        title: row.get("title"),
        is_active: row.get("is_active"),
        context_name: row.get("context_name"),
        doc_type_name: row.get("doc_type_name"),
        owner_handle: row.get("owner_handle"),
        stage: row.get("stage"),
        mode: row.get("mode"),
        effort: row.get("effort"),
        seq,
        body_hash: row.get("body_hash"),
    })
}
```

- [ ] **Step 4: Run the parity test to verify it passes**

Run: `cargo nextest run -p temper-next --features artifact-tests -E 'test(resource_row_parity)'`
Expected: PASS — every active fixture resource's invariant fields match production `get_visible`.

- [ ] **Step 5: Run the temper-next artifact suite to guard no regression**

Run: `cargo nextest run -p temper-next --features artifact-tests`
Expected: PASS (was 105/105; now +1).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-next/src/readback/mod.rs crates/temper-next/tests/parity_reads.rs
git commit -m "WS6 4b: readback::resource_row — full-row reconstruction at the invariant floor"
```

---

## Task 4: `NextBackend` skeleton — Backend trait impl, reads delegate, writes stub

**Files:**
- Create: `crates/temper-api/src/backend/next_backend.rs`
- Modify: `crates/temper-api/src/backend/mod.rs`
- Test: inline `#[cfg(test)]` unit (object-safety + write-stub error); behavioral parity is Task 11.

- [ ] **Step 1: Write the failing unit test (object safety + write stubs error)**

Create `crates/temper-api/src/backend/next_backend.rs` with ONLY the test first:

```rust
//! `NextBackend` (WS6 chunk 4b) — the `temper_next.*` substrate behind the `Backend` trait.
//! Feature-gated behind `next-backend` (pulls temper-next + onnx). Reads delegate to
//! `temper_next::readback`; writes stub `NotImplemented` until 4c.
#![cfg(feature = "next-backend")]

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_object_safe() {
        fn assert_obj(_: &dyn temper_core::operations::Backend) {}
        // Compile-time only: NextBackend must be usable as `dyn Backend`.
        let _ = assert_obj;
    }
}
```

- [ ] **Step 2: Run to verify it fails (NextBackend not defined)**

Run: `cargo test -p temper-api --features next-backend --no-run 2>&1 | head -30`
Expected: FAIL — `super::*` resolves nothing; references will not compile once the test body uses `NextBackend`. (At this step the test is trivial; it exists to anchor the file. Real failure appears when Step 3's body lands.)

- [ ] **Step 3: Implement `NextBackend`**

Replace the file body (keep the `#![cfg(...)]` at top):

```rust
//! `NextBackend` (WS6 chunk 4b) — the `temper_next.*` substrate behind the `Backend` trait.
//! Feature-gated behind `next-backend` (pulls temper-next + onnx). Reads delegate to
//! `temper_next::readback`; writes stub `NotImplemented` until 4c.
#![cfg(feature = "next-backend")]

use async_trait::async_trait;
use chrono::Utc;
use sqlx::{PgPool, Row};

use temper_core::error::TemperError;
use temper_core::operations::backend::{Backend, ResourceSummary, SearchHit};
use temper_core::operations::commands::{
    CreateResource, DeleteResource, ListResources, ResourceRef, SearchResources, ShowResource,
    UpdateResource,
};
use temper_core::operations::output::CommandOutput;
use temper_core::types::ids::{ContextId, DocTypeId, ProfileId, ResourceId};
use temper_core::types::resource::ResourceRow;

use temper_next::readback;

/// The `temper_next.*` backend. Holds a pool + the caller profile (for symmetry with `DbBackend`;
/// 4b reads are visibility-UNSCOPED per the §9 floor — access-scoping is a named flip prerequisite).
pub struct NextBackend {
    pool: PgPool,
    #[allow(dead_code)] // used once access-scoping lands (WS2 / flip prerequisite)
    profile_id: ProfileId,
}

impl NextBackend {
    pub fn new(pool: PgPool, profile_id: ProfileId) -> Self {
        Self { pool, profile_id }
    }

    /// Resolve the new-substrate resource id for a `ResourceRef`. 4b supports `Uuid` refs only
    /// (the HTTP show path always passes a `Uuid`); `Scoped` refs map by `origin_uri` and land in 4c
    /// alongside the write paths that need them.
    async fn resolve_new_id(&self, refr: &ResourceRef) -> Result<uuid::Uuid, TemperError> {
        match refr {
            ResourceRef::Uuid { id } => {
                // The HTTP id is the PRODUCTION id; map it to the synthesized id by origin_uri.
                let ids = readback::ResolvedIds::load(&self.pool)
                    .await
                    .map_err(|e| TemperError::Internal(e.to_string()))?;
                ids.to_new((*id).into()).ok_or_else(|| {
                    TemperError::NotFound(format!("resource {id} not in temper_next"))
                })
            }
            ResourceRef::Scoped { .. } => Err(TemperError::NotImplemented(
                "scoped resource refs on the next backend (WS6 4c)".into(),
            )),
        }
    }

    /// Transitional `public.kb_doc_types` name→id lookup (valid during the migration window; `public`
    /// still exists pre-flip). §7 dissolved the typed `DocTypeId`; the substrate keeps only the name.
    async fn doc_type_id_by_name(&self, name: &str) -> Result<DocTypeId, TemperError> {
        let row = sqlx::query("SELECT id FROM public.kb_doc_types WHERE name = $1")
            .bind(name)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| TemperError::Internal(format!("doc_type lookup for {name:?}: {e}")))?;
        Ok(DocTypeId::from(row.get::<uuid::Uuid, _>("id")))
    }
}

#[async_trait]
impl Backend for NextBackend {
    async fn create_resource(
        &self,
        _cmd: CreateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        Err(TemperError::NotImplemented(
            "create over the next backend (WS6 4c)".into(),
        ))
    }

    async fn show_resource(
        &self,
        cmd: ShowResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        let new_id = self.resolve_new_id(&cmd.resource).await?;
        let p = readback::resource_row(&self.pool, new_id)
            .await
            .map_err(|e| TemperError::Internal(e.to_string()))?;
        let kb_doc_type_id = self.doc_type_id_by_name(&p.doc_type_name).await?;

        // Non-invariant fields filled best-effort per the 4b parity-floor amendment.
        let now = Utc::now();
        let row = ResourceRow {
            id: ResourceId::from(p.re_minted_id),
            kb_context_id: ContextId::from(p.re_minted_context_id),
            kb_doc_type_id,
            origin_uri: p.origin_uri,
            title: p.title,
            slug: None,
            originator_profile_id: ProfileId::from(p.originator_profile_id),
            owner_profile_id: ProfileId::from(p.owner_profile_id),
            is_active: p.is_active,
            created: now,
            updated: now,
            context_name: p.context_name,
            doc_type_name: p.doc_type_name,
            owner_handle: p.owner_handle,
            stage: p.stage,
            seq: p.seq,
            mode: p.mode,
            effort: p.effort,
            body_hash: p.body_hash,
            managed_hash: None,
            open_hash: None,
        };
        Ok(CommandOutput::new(row))
    }

    async fn update_resource(
        &self,
        _cmd: UpdateResource,
    ) -> Result<CommandOutput<ResourceRow>, TemperError> {
        Err(TemperError::NotImplemented(
            "update over the next backend (WS6 4c)".into(),
        ))
    }

    async fn delete_resource(
        &self,
        _cmd: DeleteResource,
    ) -> Result<CommandOutput<()>, TemperError> {
        Err(TemperError::NotImplemented(
            "delete over the next backend (WS6 4c)".into(),
        ))
    }

    async fn list_resources(
        &self,
        _cmd: ListResources,
    ) -> Result<CommandOutput<Vec<ResourceSummary>>, TemperError> {
        let rows = readback::list(&self.pool)
            .await
            .map_err(|e| TemperError::Internal(e.to_string()))?;
        let summaries = rows
            .into_iter()
            .map(|r| ResourceSummary {
                // slug is §7-dissolved; the list summary uses origin_uri as the stable handle.
                slug: r.origin_uri,
                doctype: r.doc_type,
                context: String::new(), // context filter is a flip-prerequisite scoping concern (WS2)
                title: r.title,
            })
            .collect();
        Ok(CommandOutput::new(summaries))
    }

    async fn search_resources(
        &self,
        cmd: SearchResources,
    ) -> Result<CommandOutput<Vec<SearchHit>>, TemperError> {
        // 4b: FTS only (the text query). Vector search needs a query embedding the command does not
        // carry at this layer; the HTTP search selector handles vector mode directly (Task 8).
        let uris = readback::fts_search(&self.pool, &cmd.query.text)
            .await
            .map_err(|e| TemperError::Internal(e.to_string()))?;
        let hits = uris
            .into_iter()
            .map(|uri| SearchHit {
                summary: ResourceSummary {
                    slug: uri,
                    doctype: String::new(),
                    context: String::new(),
                    title: String::new(),
                },
                score: 0.0, // §9 floor asserts the matching SET, not the score
            })
            .collect();
        Ok(CommandOutput::new(hits))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_object_safe() {
        fn assert_obj(_: &dyn Backend) {}
        let _ = assert_obj;
        // If NextBackend were not object-safe, `select_backend`'s next arm (Task 5) would not compile.
    }
}
```

NOTE — verify these symbols before relying on them; substitute the real variant if a name differs:
- `TemperError::{Internal, NotFound, NotImplemented}` — grep `crates/temper-core/src/error.rs`. `NotImplemented` exists (4a). Confirm `Internal`/`NotFound` spellings; if `NotFound` is absent, reuse the error the legacy show path returns for a missing id.
- `SearchResources.query.text` — grep `crates/temper-core/src/operations/commands.rs` for the `SearchResources`/`SearchQuery` shape; use the actual text field name.
- `CommandId` / `CommandOutput::new` — `crates/temper-core/src/operations/output.rs`.
- `ContextId`/`DocTypeId`/`ProfileId`/`ResourceId` `From<Uuid>` impls — `crates/temper-core/src/types/ids.rs`.

- [ ] **Step 2 (export): add the module to `backend/mod.rs`**

In `crates/temper-api/src/backend/mod.rs`, add (gated):

```rust
#[cfg(feature = "next-backend")]
mod next_backend;
#[cfg(feature = "next-backend")]
pub use next_backend::NextBackend;
```

- [ ] **Step 3: Run the unit test + a feature-off build**

Run: `cargo test -p temper-api --features next-backend next_backend 2>&1 | tail -20`
Expected: PASS (object-safety compiles).
Run: `cargo build -p temper-api`
Expected: PASS (feature off — NextBackend not compiled).

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/src/backend/next_backend.rs crates/temper-api/src/backend/mod.rs
git commit -m "WS6 4b: NextBackend — reads delegate to readback, writes stub NotImplemented"
```

---

## Task 5: Wire `select_backend`'s `next` arm to construct `NextBackend`

**Files:**
- Modify: `crates/temper-api/src/backend/selection.rs`

- [ ] **Step 1: Update the `next` arm (feature-gated)**

In `select_backend`, replace the `BackendSelection::Next` arm. Keep a feature-off fallback so the build without `next-backend` still compiles (it returns the same NotImplemented as today):

```rust
        BackendSelection::Next => {
            #[cfg(feature = "next-backend")]
            {
                Ok(Box::new(crate::backend::NextBackend::new(
                    pool.clone(),
                    profile_id,
                )))
            }
            #[cfg(not(feature = "next-backend"))]
            {
                let _ = (pool, profile_id, device_id, surface);
                Err(TemperError::NotImplemented(
                    "next backend requires the `next-backend` build feature".into(),
                ))
            }
        }
```

(`require_legacy_backend`'s `Next` arm stays erroring — relationship/edge writes are 4c.)

- [ ] **Step 2: Update the existing `select_backend_next_errors` unit test**

That test (`selection.rs:106`) asserts the `next` arm errors. Under `next-backend` it now SUCCEEDS. Gate the assertion:

```rust
    #[sqlx::test(migrations = "../../migrations")]
    async fn select_backend_next_constructs_or_gates(pool: PgPool) {
        let b = select_backend(
            BackendSelection::Next,
            &pool,
            pid(),
            "api".to_string(),
            Surface::ApiHttp,
        );
        #[cfg(feature = "next-backend")]
        assert!(b.is_ok(), "with next-backend, the next arm constructs NextBackend");
        #[cfg(not(feature = "next-backend"))]
        assert!(
            matches!(b, Err(TemperError::NotImplemented(_))),
            "without the feature, the next arm gates"
        );
    }
```

Remove the old `select_backend_next_errors` test (superseded).

- [ ] **Step 3: Run both feature configurations**

Run: `cargo nextest run -p temper-api --features test-db select_backend`
Expected: PASS (feature off — gates).
Run: `cargo nextest run -p temper-api --features "test-db,next-backend" select_backend`
Expected: PASS (feature on — constructs).

- [ ] **Step 4: Commit**

```bash
git add crates/temper-api/src/backend/selection.rs
git commit -m "WS6 4b: select_backend next arm constructs NextBackend (feature-gated)"
```

---

## Task 6: Read selector for the service-direct reads

**Files:**
- Create: `crates/temper-api/src/backend/read_selector.rs`
- Modify: `crates/temper-api/src/backend/mod.rs`

The selector exposes one function per service-direct read. Each takes `BackendSelection` + the inputs the handler already has, and under `Next` calls `readback::*` (mapping to the handler's response type), under `Legacy` calls the existing service unchanged. This keeps the read architecture service-direct (the trait projections are lossy and don't cover meta/body/edges).

- [ ] **Step 1: Write a failing unit test (legacy passthrough shape)**

Create `crates/temper-api/src/backend/read_selector.rs`:

```rust
//! Read selector (WS6 chunk 4b) — routes the service-direct read paths (list / by_uri / content /
//! meta / search) to either the legacy `public.*` services or the `temper_next.*` readback, per
//! `AppState.backend_selection`. The `Next` arms are feature-gated behind `next-backend`; without the
//! feature they return the same `NotImplemented` gate as `select_backend`.

use sqlx::PgPool;
use uuid::Uuid;

use temper_core::error::TemperError;
use temper_core::types::ids::ProfileId;

use crate::backend::selection::BackendSelection;
use crate::error::ApiError;
use crate::services::resource_service::{self, ResourceListParams, ResourceListResponse, ResourceRow};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_without_feature_gates() {
        // Compile-time anchor; behavioral tests live in the e2e read-path suite (Task 12).
        let _ = BackendSelection::Next;
    }
}
```

- [ ] **Step 2: Run to verify it compiles/fails appropriately**

Run: `cargo test -p temper-api read_selector --no-run 2>&1 | tail -15`
Expected: PASS to compile (anchor test only).

- [ ] **Step 3: Implement the selector functions**

Replace the file body. Implement one fn per read. Mapping notes:
- `by_uri` (full row): `Next` builds a `ResourceRow` exactly as `NextBackend::show_resource` does — to avoid duplication, delegate through `NextBackend`. So `by_uri_select` (Next) resolves the resource by `origin_uri` via readback and reuses the same assembly. Simplest: under `Next`, construct a `NextBackend` and call `show_resource` with a `Scoped`→`origin_uri` path. Since 4b's `resolve_new_id` only supports `Uuid`, add an `origin_uri` resolver here (read `temper_next.kb_resources` by the resolved `origin_uri`). See Step 3b.
- `get_meta` (Next): map `readback::meta` → `ResourceMetaResponse` (managed/open/doc_type → the response's fields; grep `crates/temper-core/src/types/managed_meta.rs` for `ResourceMetaResponse` shape).
- `get_content`/body (Next): `readback::body` → `ContentResponse { content }` (grep `ContentResponse` in `temper-core/src/types/resource.rs`).
- `list` (Next): `readback::list` → `ResourceListResponse` (rows + facets; under the §9 floor facets are empty and pagination is the full set — document this).
- `search` (Next): `readback::fts_search` (text) or `readback::vector_search` (embedding) → `Vec<UnifiedSearchResultRow>` minimal rows (matching SET; score not asserted).

```rust
use temper_core::types::managed_meta::ResourceMetaResponse;
use temper_core::types::resource::ContentResponse;
use crate::services::meta_service;
use crate::services::search_service::{self, SearchParams, UnifiedSearchResultRow};

/// `list` — list visible resources.
pub async fn list_select(
    selection: BackendSelection,
    pool: &PgPool,
    profile_id: ProfileId,
    params: ResourceListParams,
) -> Result<ResourceListResponse, ApiError> {
    match selection {
        BackendSelection::Legacy => Ok(resource_service::list_visible(pool, *profile_id, params).await?),
        BackendSelection::Next => next_list(pool).await,
    }
}

/// `by_uri` / `show` full-row read.
pub async fn resolve_by_uri_select(
    selection: BackendSelection,
    pool: &PgPool,
    profile_id: ProfileId,
    params: &resource_service::ResolveByUriParams,
) -> Result<ResourceRow, ApiError> {
    match selection {
        BackendSelection::Legacy => Ok(resource_service::resolve_by_uri(pool, *profile_id, params).await?),
        BackendSelection::Next => next_resolve_by_uri(pool, profile_id, params).await,
    }
}

/// `get_content` body read.
pub async fn get_content_select(
    selection: BackendSelection,
    pool: &PgPool,
    profile_id: ProfileId,
    resource_id: Uuid,
) -> Result<ContentResponse, ApiError> {
    match selection {
        BackendSelection::Legacy => Ok(resource_service::get_content(pool, *profile_id, resource_id).await?),
        BackendSelection::Next => next_get_content(pool, resource_id).await,
    }
}

/// `get_meta`.
pub async fn get_meta_select(
    selection: BackendSelection,
    pool: &PgPool,
    profile_id: ProfileId,
    resource_id: Uuid,
) -> Result<ResourceMetaResponse, ApiError> {
    match selection {
        BackendSelection::Legacy => Ok(meta_service::get_meta(pool, profile_id, resource_id.into()).await?),
        BackendSelection::Next => next_get_meta(pool, resource_id).await,
    }
}

/// `search`.
pub async fn search_select(
    selection: BackendSelection,
    pool: &PgPool,
    profile_id: ProfileId,
    params: SearchParams,
) -> Result<Vec<UnifiedSearchResultRow>, ApiError> {
    match selection {
        BackendSelection::Legacy => Ok(search_service::search(pool, *profile_id, params).await?),
        BackendSelection::Next => next_search(pool, params).await,
    }
}
```

- [ ] **Step 3b: Implement the `Next` arms (feature-gated)**

Add the `next_*` helpers. Without the feature, each returns the gate error. With it, each maps `readback`. The exact response-type construction (`ResourceListResponse`, `ResourceMetaResponse`, `ContentResponse`, `UnifiedSearchResultRow`) must match the real struct fields — grep them first and fill every field (no `..Default::default()` unless the struct derives Default).

```rust
#[cfg(not(feature = "next-backend"))]
mod next_impl {
    use super::*;
    fn gate<T>() -> Result<T, ApiError> {
        Err(ApiError::from(TemperError::NotImplemented(
            "next backend requires the `next-backend` build feature".into(),
        )))
    }
    pub(super) async fn list(_: &PgPool) -> Result<ResourceListResponse, ApiError> { gate() }
    pub(super) async fn resolve_by_uri(_: &PgPool, _: ProfileId, _: &resource_service::ResolveByUriParams) -> Result<ResourceRow, ApiError> { gate() }
    pub(super) async fn get_content(_: &PgPool, _: Uuid) -> Result<ContentResponse, ApiError> { gate() }
    pub(super) async fn get_meta(_: &PgPool, _: Uuid) -> Result<ResourceMetaResponse, ApiError> { gate() }
    pub(super) async fn search(_: &PgPool, _: SearchParams) -> Result<Vec<UnifiedSearchResultRow>, ApiError> { gate() }
}

#[cfg(feature = "next-backend")]
mod next_impl {
    use super::*;
    use temper_next::readback;
    // Implement each using readback::* and the resolved origin_uri/new_id mapping.
    // Fill response structs field-for-field — see the grep notes in Step 3.
    // ... (one fn per read; bodies map readback output to the response type) ...
}

async fn next_list(pool: &PgPool) -> Result<ResourceListResponse, ApiError> { next_impl::list(pool).await }
async fn next_resolve_by_uri(pool: &PgPool, pid: ProfileId, p: &resource_service::ResolveByUriParams) -> Result<ResourceRow, ApiError> { next_impl::resolve_by_uri(pool, pid, p).await }
async fn next_get_content(pool: &PgPool, id: Uuid) -> Result<ContentResponse, ApiError> { next_impl::get_content(pool, id).await }
async fn next_get_meta(pool: &PgPool, id: Uuid) -> Result<ResourceMetaResponse, ApiError> { next_impl::get_meta(pool, id).await }
async fn next_search(pool: &PgPool, p: SearchParams) -> Result<Vec<UnifiedSearchResultRow>, ApiError> { next_impl::search(pool, p).await }
```

IMPLEMENTATION NOTE for the `#[cfg(feature = "next-backend")]` bodies (do this work in this step, not later — no placeholders): for each read, (1) for id-keyed reads (`get_content`, `get_meta`), map the production resource id → synthesized id via `readback::ResolvedIds::load(pool)` then `to_new(id)`; (2) call the matching `readback::*`; (3) construct the response struct with every field populated (origin_uri-as-slug where slug is dissolved; empty facets/zero scores at the §9 floor). If a response struct cannot be honestly constructed from readback output (a field with no substrate source that the handler/test depends on), STOP and escalate per the implementation-grounding guidance — do not invent a value that a test then asserts.

- [ ] **Step 4: Register the module**

In `crates/temper-api/src/backend/mod.rs`:

```rust
pub mod read_selector;
```

- [ ] **Step 5: Build both feature configs**

Run: `cargo build -p temper-api` then `cargo build -p temper-api --features next-backend`
Expected: both PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/backend/read_selector.rs crates/temper-api/src/backend/mod.rs
git commit -m "WS6 4b: read selector for service-direct reads (list/by_uri/content/meta/search)"
```

---

## Task 7: Route the API read handlers through the selector

**Files:**
- Modify: `crates/temper-api/src/handlers/resources.rs` (`list`, `by_uri`, `get_content`)
- Modify: `crates/temper-api/src/handlers/meta.rs` (`get_meta`)
- Modify: `crates/temper-api/src/handlers/search.rs` (`search`)

- [ ] **Step 1: Re-point `list`**

In `resources.rs::list`, replace the direct `resource_service::list_visible` call (the non-meta branch) with:

```rust
        let response = crate::backend::read_selector::list_select(
            state.backend_selection,
            &state.pool,
            ProfileId::from(auth.0.profile.id),
            params,
        )
        .await?;
        Ok(ListResourcesResponse::Default(response))
```

(Leave the `meta_only` branch on `list_visible_meta` for now — meta-list parity over the substrate is not in the §9 floor's list read; note it as a deferred item in the commit message. If grep shows a `readback` meta-list equivalent exists, route it; otherwise legacy.)

- [ ] **Step 2: Re-point `by_uri`**

```rust
pub async fn by_uri(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(params): Query<ResolveByUriParams>,
) -> ApiResult<Json<ResourceRow>> {
    let row = crate::backend::read_selector::resolve_by_uri_select(
        state.backend_selection,
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        &params,
    )
    .await?;
    Ok(Json(row))
}
```

- [ ] **Step 3: Re-point `get_content`**

```rust
pub async fn get_content(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(resource_id): Path<Uuid>,
) -> ApiResult<Json<ContentResponse>> {
    let resp = crate::backend::read_selector::get_content_select(
        state.backend_selection,
        &state.pool,
        ProfileId::from(auth.0.profile.id),
        resource_id,
    )
    .await?;
    Ok(Json(resp))
}
```

- [ ] **Step 4: Re-point `get_meta` (meta.rs) and `search` (search.rs)** the same way, calling `get_meta_select` / `search_select`.

- [ ] **Step 5: Build + run the API suite under legacy default**

Run: `cargo nextest run -p temper-api --features test-db`
Expected: PASS (legacy path unchanged — was 222/222; the selector's Legacy arm is a passthrough).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api/src/handlers/resources.rs crates/temper-api/src/handlers/meta.rs crates/temper-api/src/handlers/search.rs
git commit -m "WS6 4b: route API service-direct reads through the read selector"
```

---

## Task 8: Route the MCP read tools through the selector

**Files:**
- Modify: `crates/temper-mcp/src/tools/resources.rs` (`get_resource`, `list_resources`)
- Modify: `crates/temper-mcp/src/tools/search.rs` (`search`)

MCP tools hold `svc.api_state` (the same `AppState`, carrying `backend_selection`). Route each read tool's underlying `resource_service`/`search_service` call through the matching `read_selector::*_select`, passing `svc.api_state.backend_selection`.

- [ ] **Step 1: `search` tool** — replace the `search_service::search(...)` call (`search.rs:15`) with `temper_api::backend::read_selector::search_select(svc.api_state.backend_selection, &svc.api_state.pool, profile.id.into(), input)`. Confirm the import path (`temper_api::backend::read_selector`) is exported (Task 6 made `read_selector` `pub mod`).

- [ ] **Step 2: `get_resource` tool** — it currently branches on id vs slug (`resources.rs:413-466`), then fetches content. Route the full-row fetch through `resolve_by_uri_select` (slug branch → build the `ResolveByUriParams`) / a show via `select_backend` (id branch, already trait-routed in 4a? confirm). For the body, route `get_content` through `get_content_select`. Keep the legacy behavior byte-identical (the selector's Legacy arm passes through).

- [ ] **Step 3: `list_resources` tool** — route `list_visible` (`resources.rs:551`) through `list_select`.

- [ ] **Step 4: Build + run the MCP suite under legacy default**

Run: `cargo nextest run -p temper-mcp --features test-db`
Expected: PASS (was 25/25 — Legacy passthrough unchanged).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-mcp/src/tools/resources.rs crates/temper-mcp/src/tools/search.rs
git commit -m "WS6 4b: route MCP read tools through the read selector"
```

---

## Task 9: Enable `next-backend` in the deployed adapters — DEFERRED to flip-prep (chunk 5)

**Status: DEFERRED. Do not do in 4b.**

**Why deferred (verified during 4b):** The deployed bins (`temper-cloud` package `[[bin]]` axum/mcp) build from the root `Cargo.toml`'s `temper-api = { features = [...] }` line — the Vercel `vercel-rust` builder has no per-deploy `--features` injection point (`vercel.json` only sets `build.env.SQLX_OFFLINE=true`; it cannot add cargo features). So enabling `next-backend` for deployment *requires* adding it to the committed default `temper-cloud` build. But that pulls `temper-next` into the **default** build, and `temper-next`'s `temper_next`-namespace `sqlx::query!` macros CANNOT validate live — the pre-commit hook's clippy and CI's `code-quality` clippy both compile **live** (no `SQLX_OFFLINE`), so they fail with "function cogmap_genesis does not exist" (validating against the dev DB's `public` search_path). Verified: adding the feature to root `Cargo.toml` breaks `git commit` (pre-commit clippy).

**Why it's safe to defer:** 4b is gated OFF and the **flip is chunk 5** (gated additionally on 4c writes + WS2 access). 4b's correctness is proven by Tasks 10–11, which enable `next-backend` **explicitly** in their test builds (`--features ...,next-backend`). The deployed enable is only needed so the chunk-5 flip is config-only; making it then lets the `SQLX_OFFLINE` strategy for the pre-commit hook + CI `code-quality` clippy be decided deliberately (e.g. switch those to offline with the committed `.sqlx` caches — the same stance `cargo make check` already takes — or scope temper-next out of the live clippy job).

**Carried to chunk-5 flip prep:** add `"next-backend"` to root `Cargo.toml`'s `temper-api` features, AND resolve the live-clippy validation (pre-commit hook + CI `code-quality`) so the workspace build with `next-backend` validates offline.

---

## Task 10: Re-point the parity harness through `NextBackend` / the selector

**Files:**
- Modify: `crates/temper-next/tests/parity_reads.rs`

The chunk-3 parity tests call `readback::*` directly. Add a layer that drives one read through `NextBackend` (proving the wiring preserves the §9 floor, per spec proof gate 1). This requires the `next-backend` feature on temper-api in the test build; gate the new test on it.

- [ ] **Step 1: Add a `NextBackend`-driven show parity test**

```rust
#[cfg(feature = "next-backend")] // temper-api's NextBackend; requires `cargo nextest -p temper-next --features artifact-tests,next-backend`
#[sqlx::test(migrator = "temper_next::MIGRATOR")]
async fn show_through_next_backend_matches_resource_row(pool: sqlx::PgPool) {
    use temper_api::backend::NextBackend;
    use temper_core::operations::backend::Backend;
    use temper_core::operations::commands::{ResourceRef, ShowResource, Surface};

    common::seed_and_synthesize(&pool).await;
    let ids = ResolvedIds::load(&pool).await.expect("ids");
    let backend = NextBackend::new(pool.clone(), common::fixture_ids::P1.into());

    for new_id in ids.new_ids() {
        let old_id = ids.to_old(new_id).expect("maps back");
        let out = backend
            .show_resource(ShowResource {
                resource: ResourceRef::Uuid { id: old_id.into() },
                origin: Surface::ApiHttp,
            })
            .await
            .expect("NextBackend::show_resource");
        // The invariant subset must equal what readback::resource_row produced directly.
        let direct = readback::resource_row(&pool, new_id).await.expect("direct");
        assert_eq!(out.value.origin_uri, direct.origin_uri);
        assert_eq!(out.value.title, direct.title);
        assert_eq!(out.value.doc_type_name, direct.doc_type_name);
        assert_eq!(out.value.context_name, direct.context_name);
        assert_eq!(out.value.owner_handle, direct.owner_handle);
        // Non-invariants present but not asserted against production: slug None, hashes None.
        assert!(out.value.slug.is_none(), "slug dissolved (§7)");
        assert!(out.value.managed_hash.is_none() && out.value.open_hash.is_none());
    }
}
```

NOTE: add `temper-next`'s test build a path to temper-api's `next-backend` feature — temper-next already dev-depends on temper-api; add a temper-next feature `next-backend = ["temper-api/next-backend"]` in its `[features]` so `--features next-backend` propagates. Confirm the dev-dep is `temper-api = { path = "../temper-api" }` and add the feature passthrough.

- [ ] **Step 2: Run it**

Run: `cargo nextest run -p temper-next --features "artifact-tests,next-backend" -E 'test(show_through_next_backend)'`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-next/tests/parity_reads.rs crates/temper-next/Cargo.toml
git commit -m "WS6 4b: parity harness drives show through NextBackend (wiring preserves §9 floor)"
```

---

## Task 11: HTTP read-set-equality e2e test (flag=next vs legacy)

**Files:**
- Create: `tests/e2e/tests/backend_read_path_next.rs`
- Possibly modify: `tests/e2e/tests/common/` (synthesis invocation helper, if not present)

This is spec proof gate 2 — the surface analogue of 4a's gate test. Seed `public`, run synthesis into `temper_next`, then assert the read endpoints return the same invariant data under `flag=next` as under `flag=legacy`.

- [ ] **Step 1: Confirm the e2e harness can invoke synthesis**

Run: `grep -rn "synthes\|seed_and_synthesize\|temper_next" tests/e2e/tests/common/ tests/e2e/`
If there is no synthesis trigger in the e2e harness, add a helper that calls the same synthesis entrypoint `crates/temper-next/tests/common` uses (grep that common module for the exact `synthesize`/`run` call). The e2e crate would need a dev-dep on `temper-next` (+ its features). If wiring synthesis into e2e proves heavy, FALL BACK to asserting the HTTP layer at the parity-harness level (Task 10 already proves NextBackend show; a thinner e2e can assert only the gate flip on a read endpoint reaching the substrate) — and record the reduced scope explicitly in the session note rather than silently dropping the gate.

- [ ] **Step 2: Write the test** (mirror `backend_selection_gate.rs` setup)

```rust
#![cfg(all(feature = "test-db", feature = "next-backend"))]
//! WS6 chunk 4b: under flag=next the read endpoints answer from temper_next at the §9 invariant floor.
mod common;
use reqwest::StatusCode;

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn next_show_returns_invariant_fields(pool: sqlx::PgPool) {
    // 1. Seed a known resource in public via the normal API/ingest path (use the existing e2e seed helper).
    // 2. Synthesize public -> temper_next (Step 1 helper).
    // 3. Read GET /api/resources/by-uri under legacy -> capture invariant fields.
    // 4. Flip kb_backend_selection to 'next', spawn a fresh app (startup reads the flag once).
    // 5. Read the same by-uri under next -> assert invariant fields equal the legacy capture
    //    (origin_uri, title, context_name, doc_type_name, owner_handle, stage/mode/effort/seq, body_hash).
    //    Do NOT assert id / kb_context_id / slug / hashes / timestamps.
}
```

Fill the body using the existing e2e seeding + the two-phase (legacy capture, then next-spawn) flag pattern from `backend_selection_gate.rs`. Every assertion compares only the invariant subset.

- [ ] **Step 3: Run the e2e gate + new read-path tests**

Run: `cargo nextest run -p temper-e2e --features "test-db,next-backend" -E 'test(backend)'`
Expected: PASS (4a gate test + new 4b read-path test).

- [ ] **Step 4: Regenerate the e2e .sqlx cache if any macro query was added**

Run: `cargo make prepare-e2e` (only if the new test uses `sqlx::query!` macros; the harness uses runtime queries mostly).

- [ ] **Step 5: Commit**

```bash
git add tests/e2e/tests/backend_read_path_next.rs tests/e2e/tests/common/ tests/e2e/.sqlx 2>/dev/null
git commit -m "WS6 4b: e2e HTTP read-set-equality under flag=next (invariant floor)"
```

---

## Task 12: Goal-record note + branch verification

**Files:** none (records + verification)

- [ ] **Step 1: Full check + suites (NOT per-task — branch-level)**

Run, in order:
```bash
cargo make check
cargo nextest run -p temper-api --features test-db
cargo nextest run -p temper-mcp --features test-db
cargo nextest run -p temper-next --features "artifact-tests,next-backend"
cargo nextest run -p temper-e2e --features "test-db,next-backend"
```
Expected: all green. Record exact pass counts.

NOTE the artifact-tests + e2e-next runs are LOCAL-ONLY (no CI job enables `artifact-tests` or `next-backend`); confirm the dev DB has the artifact loaded (`cargo make prepare-next` lineage) and a clean `temper_next`. If a checksum drift appears, the recorded fix is: `psql .../postgres -c "DROP DATABASE temper_development WITH (FORCE)"` + recreate + `sqlx migrate run`.

- [ ] **Step 2: Update the goal record** `substrate-kernel-to-cognitive-map` WS6 paragraph — note 4b LANDED (reads answer from `temper_next` behind flag=next at the invariant floor), and that **access-scoping over `temper_next` is now a named flip prerequisite** (carried to WS2). Use the show-edit-cat idiom:

```bash
temper resource show substrate-kernel-to-cognitive-map --type goal --context temper > /tmp/goal.md
# edit /tmp/goal.md — append the 4b-landed + access-scoping-flip-prereq note to the WS6 section
cat /tmp/goal.md | temper resource update substrate-kernel-to-cognitive-map --type goal --context temper
```

- [ ] **Step 3: Do NOT open a PR.** 4a + 4b + 4c ship as ONE PR off `jct/ws6-chunk4-gate-decomposition` (owner's call). 4c (writes) is the next plan. Leave the branch open.

---

## Self-review notes (author checklist results)

- **Spec coverage:** §9 reads — list (T7), show/by_uri (T3+T4+T7), body (T6/T7), meta (T6/T7), FTS+vector search (T6/T7/T8). Proof gate 1 (harness through NextBackend) → T10. Proof gate 2 (HTTP set-equality) → T11. Access-scoping deferred (spec out-of-scope) → recorded T12. Writes stub → T4 (4c fills). `select_backend` next arm → T5. Graph/subgraph explicitly out (depth-2 ≠ §9 1-hop) → recorded in grounding + T1 amendment.
- **Type consistency:** `ResourceRowParity` (T3) consumed by `NextBackend::show_resource` (T4) and asserted in T10. `read_selector::*_select` signatures (T6) consumed verbatim by handlers (T7) and MCP tools (T8). `next-backend` feature threaded: temper-api (T2) → selection/NextBackend/selector (T4/T5/T6) → adapters (T9) → temper-next test passthrough (T10) → e2e (T11).
- **Known verify-before-relying items (flagged inline, not placeholders):** `TemperError` variant names (`Internal`/`NotFound`), `SearchResources.query.text` field, `ResourceMetaResponse`/`ContentResponse`/`UnifiedSearchResultRow`/`ResourceListResponse` field shapes, `common::fixture_ids::P1`, the deployed-adapter feature-enabling mechanism, and whether e2e can invoke synthesis (T11 Step 1 has a recorded fallback). Each step says to grep the cited file and substitute the real symbol — and to escalate (not invent) if a response struct can't be honestly built from readback output.
```
