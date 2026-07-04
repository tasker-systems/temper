# T7 — Block-Level Provenance Write-Path Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

> **Phased across three goal tasks (see "Phasing" below).** This plan is written full-detail for Phases A + B (Tasks 1–8); Phase C (Tasks 9–12) is at design granularity — expand to full TDD steps when T7c is picked up. Phase C's one schema question is **decided** (Task 9: the `kb_remote_sources` table).

**Goal:** Un-stub the block-provenance write path so that when a resource block is born-from or revised-from N sources, each source is recorded in `kb_block_provenance` — activating the already-built read + region-salience signals that read that table but see zero today.

**Architecture:** `ProvenanceSource` becomes a shared wire type in `temper-core` (re-exported from `temper-substrate::payloads`, exactly as `AgentAuthorship` was). A `sources: Vec<ProvenanceSource>` field is threaded through the **one** continuous body carrier — `BodyUpdate` (`temper-workflow`) — so it flows symmetrically through *both* create and update paths down into `CreateParams`/`UpdateParams` (`temper-substrate`), into the `ResourceCreated`/`BlockMutated` event payloads, and finally into `kb_block_provenance` via a new additive migration that teaches **both** projectors (`_project_blocks`, `_project_block_mutated`) to INSERT provenance rows. A new read function + MCP tool + CLI flag surface per-block provenance.

**Tech Stack:** Rust (temper-core, temper-substrate, temper-workflow, temper-services, temper-api, temper-mcp, temper-cli), PostgreSQL 18 + pgvector (sqlx, additive migration), cargo-nextest, `#[sqlx::test]` (artifact-tests + test-db features).

## Global Constraints

- **Additive-only-on-`main`.** The projection change ships as a **new** migration (`CREATE OR REPLACE` of the two projector functions), never by editing a birth migration. Next number: `migrations/20260704000003_block_provenance_write_path.sql`.
- **`provenance_source_kind` is `('event','resource')` for Phases A + B; `'remote'` is ADDED in Phase C (T7c).** Phases A/B resolve **resource refs only** → `ProvenanceSource::Resource` (and the internal `Event` variant). Phase C un-YAGNIs `'remote'` (URL/external sources) as a full first-class source kind — enum value, `ProvenanceSource::Remote`, storage, and URL-accepting `--sources`. It is **in scope for T7**, just sequenced last (it carries a schema-design fork — see Phase C Task 9).
- **Wire type lives in `temper-core`** (CLAUDE.md). `ProvenanceSource` moves there; `temper-substrate` re-exports it. Never define a parallel type.
- **Typed structs over inline JSON** — no `serde_json::json!()` for structured payload data (test fixtures excepted).
- **Auth before writes; profile scoping** — the read function gates through `resources_readable_by`; no new write bypasses `can_modify_resource` (both paths already auth in `DbBackend`).
- **SQL macros + cache** — after touching SQL, regenerate: `cargo sqlx prepare --workspace -- --all-features`, then per-crate as needed (`cargo make prepare-services`, `prepare-api`, `prepare-e2e`).
- **Resource-level grain this branch; per-block is the North Star.** The eventual target CLI is `temper resource update <uuid> --content-block <uuid> --sources <urls|refs> [--body …]`. Design the payload layer per-block (sources ride each `BlockManifest`) so that target slots in without a payload change; the *surface* this branch exposes a resource-level `--sources` that applies to the resource's (single body) block.
- **Run `cargo make check` before every commit.** `SQLX_OFFLINE=true` is forced by cargo-make, so `cargo make check` is the honest local probe of the committed `.sqlx` caches.

## Phasing (three goal tasks under goal `019f1ac7`)

The effort scale tops out at "large," so this is **~2 larges + a medium**. Split into three sequenced, independently-shippable goal tasks. Each phase is its own PR (or PR stack); each leaves the tree green and useful.

| Phase | Goal task | Tasks | Effort | Deliverable |
|-------|-----------|-------|--------|-------------|
| **A** | **T7a — write-path foundation (substrate + migration)** | 1–3 | large | A resource/block written with resource+event sources records `kb_block_provenance`; `resource_block_provenance` read fn exists; `resource_blocks.reinforce_count` + region `reference_standing` light up. Proven via artifact-tests. **No surface yet.** |
| **B** | **T7b — surface parity (MCP + API + CLI)** | 4–8 | large | MCP/API/CLI carry `sources` (resource refs) + a `--provenance` read; e2e round-trip green. The **live steward can attribute resource sources end-to-end.** |
| **C** | **T7c — `remote` sources + per-content-block addressing + steward wiring** | 9–12 | medium | `'remote'` source kind (URL/external), the North Star `--content-block <uuid>` per-block addressing, and the steward agent actually calling `--sources` on distillation. Carries one schema-design fork (Task 9). |

**Dependency:** A → B → C (strict). A is pure substrate/DB and self-contained. B depends on A's payload/migration. C depends on B's surface plumbing (it extends `--sources`, the `sources` wire fields, and the projectors).

**Splitting knob:** if an 8-crate B branch is too big to review at once, B itself can stack — B1 = operations + API (Tasks 4–5), B2 = MCP + CLI + e2e (Tasks 6–8) — retargeting the upper PR's base to `main` before merge (stacked-PR discipline).

---

## North Star — now IN scope (Phase C), design must not corner it in A/B

```
temper resource update <uuid> --content-block <uuid> --sources <urls|refs> [--body @file]
```
- **Per-block addressing** (`--content-block <uuid>`): Phases A/B apply resource-level sources to the resource's single body block; the payload already carries `incorporated` per-`BlockManifest`, so per-block addressing (Phase C Task 11) is a surface-only change — no payload/projector rework.
- **URL sources** (`--sources https://…`): require the `'remote'` enum value + storage, landed in Phase C (Tasks 9–10). Phases A/B resolve resource refs only, so nothing there needs to change when `'remote'` arrives — `ProvenanceSource` gains a variant, not a reshape.

---

## File Structure

**New files:**
- `crates/temper-core/src/types/provenance.rs` — the `ProvenanceSource` shared wire type (moved from substrate). One responsibility: the tagged `{kind, value}` provenance-source sum + its tests.
- `migrations/20260704000003_block_provenance_write_path.sql` — `CREATE OR REPLACE` both projectors to INSERT `kb_block_provenance`; add read fn `resource_block_provenance`.
- `crates/temper-mcp/src/tools/` — extend `resources.rs`; new read tool arm `get_block_provenance` (in the existing tools module, not a new file, matching the module's one-file-per-domain convention).

**Modified files (with the field/line each touches):**
- `crates/temper-core/src/types/mod.rs` — `pub mod provenance;` (after `authorship`).
- `crates/temper-substrate/src/payloads.rs:449` — delete local `ProvenanceSource`, re-export from core (mirror `payloads.rs:501` authorship re-export); add `incorporated: Vec<Incorporation>` to `BlockManifest` (`:112`) and `ResourceCreated` stays block-carried.
- `crates/temper-substrate/src/content.rs:46` — add `incorporated: Vec<Incorporation>` to `PreparedBlock` (so the manifest `From` impl carries it).
- `crates/temper-substrate/src/writes.rs:95,168` — add `sources: &[Incorporation]` to `CreateParams`/`UpdateParams`; thread into block prep.
- `crates/temper-substrate/src/events.rs:738` — replace `incorporated: Vec::new()` with the threaded list; `ResourceCreate` arm (`:495`) carries per-block incorporation.
- `crates/temper-workflow/src/operations/inputs.rs:19` — add `sources: Vec<ProvenanceSource>` to `BodyUpdate`.
- `crates/temper-services/src/backend/db_backend.rs:855,1014` — pass `sources` into `CreateParams`/`UpdateParams`.
- `crates/temper-core/src/types/ingest.rs:15` — add `sources: Vec<ProvenanceSource>` to `IngestPayload`.
- `crates/temper-workflow/src/types/resource.rs:179` — add `sources: Vec<ProvenanceSource>` to `ResourceUpdateRequest`.
- `crates/temper-api/src/handlers/ingest.rs:87,149` + `handlers/resources.rs:222` — map wire `sources` into `BodyUpdate.sources`.
- `crates/temper-cli/src/cloud_backend/translators.rs:64,150` — carry `sources` onto the wire DTOs.
- `crates/temper-mcp/src/tools/resources.rs:27,103` — add `sources: Option<Vec<Uuid>>` to `CreateResourceInput`/`UpdateResourceInput`; new `get_block_provenance` tool.
- `crates/temper-cli/src/cli.rs:298,398` + `commands/resource.rs:160,944` — add `--sources` to create/update; `--provenance` to show.

**Test files:**
- `crates/temper-substrate/tests/content_mutation.rs` — projection-level provenance tests (artifact-tests, `#[sqlx::test(migrator = MIGRATOR)]`).
- `crates/temper-core/src/types/provenance.rs` — inline `#[cfg(test)]` serde tests.
- `tests/e2e/tests/` — new `block_provenance_test.rs` (CLI `--sources` round-trip through real Axum + Postgres).

---

## Task 1: `ProvenanceSource` shared wire type in temper-core

Moves the type to its canonical home so the command layer (`temper-workflow`) can reference it. Pure type move + re-export; no behavior change yet.

**Files:**
- Create: `crates/temper-core/src/types/provenance.rs`
- Modify: `crates/temper-core/src/types/mod.rs` (add `pub mod provenance;`)
- Modify: `crates/temper-substrate/src/payloads.rs:449-452` (delete local def), `:456-459` (`Incorporation` now uses core type)

**Interfaces:**
- Produces: `temper_core::types::provenance::ProvenanceSource` — `enum { Event(Uuid), Resource(Uuid) }`, serde-tagged `#[serde(tag = "kind", content = "value", rename_all = "snake_case")]`. Substrate re-exports it as `payloads::ProvenanceSource`.

- [ ] **Step 1: Write the failing test** (inline in the new file)

```rust
// crates/temper-core/src/types/provenance.rs  (tests module)
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provenance_source_is_tagged_kind_value() {
        let s = ProvenanceSource::Resource(uuid::Uuid::nil());
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["kind"], "resource");
        assert_eq!(v["value"], "00000000-0000-0000-0000-000000000000");
        let back: ProvenanceSource = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn provenance_source_event_variant_roundtrips() {
        let s = ProvenanceSource::Event(uuid::Uuid::nil());
        let back: ProvenanceSource =
            serde_json::from_value(serde_json::to_value(&s).unwrap()).unwrap();
        assert_eq!(back, s);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-core provenance`
Expected: FAIL — `ProvenanceSource` not found in `temper-core`.

- [ ] **Step 3: Write the type** (body of the new file, above the tests). Mirror the `authorship.rs` derive stack verbatim so it works across every surface (utoipa/ts-rs/schemars-inline). The `schemars(inline)` attribute is load-bearing — a `$ref` reaches the Anthropic tool-use layer as `null` (see the `EdgeKind`/`ConfidenceBand` scar).

```rust
//! Block-provenance source — the shared wire carrier for "where an addressable block came from".
//!
//! Canonical home (CLAUDE.md: "the wire type lives in temper-core"). `temper-substrate` re-exports
//! `ProvenanceSource` from here (the same chain as `crate::ids` and `authorship`) and records it into
//! `kb_block_provenance` via the `_project_blocks` / `_project_block_mutated` projectors.
//!
//! Tagged to match the DDL's `provenance_source_kind` ENUM (`('event','resource')`); the `'remote'`
//! value (URL/external sources) is deferred with Linear/GitHub ingest, so this enum has no URL variant.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Where an addressable block's content was incorporated from. `{kind, value}` sum, tagged to mirror
/// the DDL's `provenance_source_kind`. `Resource` is a `kb_resources` id (a distilled-from source);
/// `Event` is a `kb_events` id (used by the scar/correction path).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(
    any(feature = "mcp", feature = "scenario-schema"),
    derive(schemars::JsonSchema)
)]
#[cfg_attr(feature = "mcp", schemars(inline))]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ProvenanceSource {
    Event(Uuid),
    Resource(Uuid),
}
```

- [ ] **Step 4: Register the module.** In `crates/temper-core/src/types/mod.rs`, add after line 15 (`pub mod authorship;`):

```rust
pub mod provenance;
```

- [ ] **Step 5: Re-export from substrate; delete the local def.** In `crates/temper-substrate/src/payloads.rs`, delete the local `ProvenanceSource` enum (`:445-452`) and re-export from core alongside the existing authorship re-export (`:501`). `Incorporation` (`:456`) keeps its local definition — its `source` field now resolves to the re-exported type:

```rust
// payloads.rs — near the authorship re-export (~:501)
pub use temper_core::types::provenance::ProvenanceSource;
```

Keep the `scenario-schema` JsonSchema derive working: `Incorporation` still derives `schemars::JsonSchema` under `scenario-schema`, and `ProvenanceSource` now derives it in core under `scenario-schema` too (the derive stack above includes it). Confirm the scenario JSON-Schema snapshot test still passes (Step 7).

- [ ] **Step 6: Run the core + substrate builds**

Run: `cargo nextest run -p temper-core provenance && cargo build -p temper-substrate`
Expected: PASS — type moved, substrate compiles against the re-export.

- [ ] **Step 7: Guard the scenario schema snapshot**

Run: `cargo nextest run -p temper-substrate --features scenario-schema scenario_schema`
Expected: PASS. If the snapshot changed (it should not — same JSON shape, tagged identically), review the diff to confirm it's byte-identical structure and update the snapshot only if the change is purely the type's module path (which does not affect emitted JSON Schema). If the emitted schema differs, STOP — the tag/content attributes drifted.

- [ ] **Step 8: `cargo make check` then commit**

```bash
cargo make check
git add crates/temper-core/src/types/provenance.rs crates/temper-core/src/types/mod.rs crates/temper-substrate/src/payloads.rs
git commit -m "refactor(provenance): move ProvenanceSource to temper-core, re-export from substrate

Canonical wire-type home (CLAUDE.md), so the command layer can reference it.
Mirrors the AgentAuthorship move. No behavior change.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Thread `sources` through the substrate write path (payload + writes + fire arms)

Carry an incorporation list from `CreateParams`/`UpdateParams` into the `ResourceCreated`/`BlockMutated` event payloads. After this task the payloads *carry* provenance but the projectors still ignore it (Task 3 wires the INSERT), so the assertion here is on the **fired payload**, not on `kb_block_provenance`.

**Files:**
- Modify: `crates/temper-substrate/src/content.rs:46` (`PreparedBlock`), `:95,:129` (prep fns)
- Modify: `crates/temper-substrate/src/payloads.rs:112` (`BlockManifest` gets `incorporated`), `:130` (`From<&PreparedBlock>`)
- Modify: `crates/temper-substrate/src/writes.rs:95` (`CreateParams`), `:168` (`UpdateParams`), `:120,:208` (thread into prep + fire)
- Modify: `crates/temper-substrate/src/events.rs:495-506` (`ResourceCreate` arm — manifests already carry it), `:730-753` (`BlockMutate` arm — replace `Vec::new()`)
- Test: `crates/temper-substrate/tests/content_mutation.rs`

**Interfaces:**
- Consumes: `payloads::{Incorporation, ProvenanceSource}` (Task 1).
- Produces:
  - `content::PreparedBlock` gains `pub incorporated: Vec<Incorporation>`.
  - `payloads::BlockManifest` gains `pub incorporated: Vec<Incorporation>` (`#[serde(default, skip_serializing_if = "Vec::is_empty")]`).
  - `writes::CreateParams` / `writes::UpdateParams` each gain `pub sources: Vec<Incorporation>`.

- [ ] **Step 1: Write the failing test** — a create carrying one resource-source fires a `ResourceCreated` whose block manifest carries the incorporation. Drive `writes::create_resource_with` directly (the artifact-tests pattern) and read the fired event payload back off `kb_events`.

```rust
// content_mutation.rs
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn create_with_sources_fires_incorporation_in_payload(pool: sqlx::PgPool) {
    use temper_substrate::payloads::{Incorporation, ProvenanceSource};
    let f = seed_minimal(&pool).await; // existing helper: profiles/context/emitter
    let src = uuid::Uuid::now_v7();

    let out = temper_substrate::writes::create_resource_with(
        &pool,
        temper_substrate::writes::CreateParams {
            title: "distilled concept",
            origin_uri: "temper://cm/beta",
            body: "one body block",
            doc_type: "concept",
            home: f.home,
            owner: f.owner,
            originator: f.owner,
            emitter: f.emitter,
            properties: &[],
            chunks: None,
            sources: vec![Incorporation { source: ProvenanceSource::Resource(src), seq: 0 }],
        },
        temper_substrate::events::EventContext::default(),
    )
    .await
    .expect("create");

    // The most recent resource_created event's payload carries the incorporation on its block.
    let payload: serde_json::Value = sqlx::query_scalar(
        "SELECT e.payload FROM kb_events e JOIN kb_event_types et ON et.id=e.event_type_id \
         WHERE et.name='resource_created' ORDER BY e.id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let inc = &payload["blocks"][0]["incorporated"][0];
    assert_eq!(inc["source"]["kind"], "resource");
    assert_eq!(inc["source"]["value"], src.to_string());
    assert_eq!(inc["seq"], 0);
    let _ = out;
}
```

> ⚠️ **Plan/reality gap for the implementer:** `seed_minimal`/the exact `CreateParams` field set is from the surface map (`writes.rs:95`). Confirm the real field names against `writes.rs` before writing — the map lists `title, origin_uri, body, doc_type, home, owner, originator, emitter, properties, chunks`. `home: AnchorRef`, `owner/originator: profile ids`, `emitter: entity id`. If `content_mutation.rs` has no reusable seed helper, reuse the one in `charter_yaml_roundtrip.rs` or add a small local one — do not invent field names.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-substrate --features artifact-tests create_with_sources_fires_incorporation`
Expected: FAIL — `CreateParams` has no `sources` field (compile error).

- [ ] **Step 3: Add `incorporated` to `PreparedBlock` + prep fns** (`content.rs`). Default empty; `prepare_block_from_chunks` / `prepare_block` gain a `sources: Vec<Incorporation>` parameter (or set the field after building — implementer's call, but the value must reach the struct):

```rust
// content.rs — PreparedBlock (~:46)
pub struct PreparedBlock {
    pub block_id: BlockId,
    pub seq: i32,
    pub role: Option<String>,
    pub chunks: Vec<PreparedChunk>,
    pub incorporated: Vec<Incorporation>, // NEW — provenance for this block's content
}
```

- [ ] **Step 4: Carry it onto the manifest** (`payloads.rs`). `BlockManifest` gains the field; the `From<&PreparedBlock>` impl copies it:

```rust
// payloads.rs — BlockManifest (~:112)
pub struct BlockManifest {
    pub block_id: BlockId,
    pub seq: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub chunks: Vec<ChunkManifest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub incorporated: Vec<Incorporation>, // NEW
}

// From<&PreparedBlock> (~:130) — add:
incorporated: b.incorporated.clone(),
```

- [ ] **Step 5: Add `sources` to `CreateParams`/`UpdateParams` and thread into block prep** (`writes.rs`). On create, the resource-level `sources` apply to the single body block built at `:125`. On update, they apply to the revised body block prepared before the `BlockMutate` fire at `:243`.

```rust
// writes.rs — CreateParams (~:95): add field
pub sources: Vec<Incorporation>,
// create_resource_with (~:125): after building `blocks`, set the body block's incorporation:
//   blocks[0].incorporated = p.sources.clone();   // resource-level → the one body block
// (Confirm `blocks` is the Vec<PreparedBlock> built by prepare_block*; set before the fire at :133.)

// writes.rs — UpdateParams (~:168): add field
pub sources: Vec<Incorporation>,
// update_resource_in_tx (~:230): after `prepared` block is built, before the BlockMutate fire (:243):
//   prepared.incorporated = p.sources.clone();
```

- [ ] **Step 6: Un-stub the fire arms** (`events.rs`). The `BlockMutate` arm at `:735-738` must carry the block's incorporation instead of `Vec::new()`. `SeedAction::BlockMutate` (`:730`) currently carries `{ block, chunks, emitter }` — extend it with `incorporated: &'a [Incorporation]` (or read it off the `PreparedBlock` the caller already holds). The `ResourceCreate` arm (`:495-506`) builds `blocks` from manifests that now carry `incorporated`, so no change there beyond confirming the `BlockManifest::from` copies it.

```rust
// events.rs — SeedAction::BlockMutate arm (~:735)
let payload = payloads::BlockMutated {
    block_id: block,
    chunks: chunks.iter().map(payloads::ChunkManifest::from).collect(),
    incorporated: incorporated.to_vec(), // was: Vec::new()  // ← un-stubbed
};
```

> ⚠️ **Plan/reality gap:** `SeedAction::BlockMutate` is a borrowed-field enum variant (`chunks: &'a [...]`). Adding `incorporated: &'a [Incorporation]` keeps it borrow-consistent. The caller in `writes.rs:243` fires `SeedAction::BlockMutate { block, chunks: &prepared.chunks, emitter }` — extend to `..., incorporated: &prepared.incorporated }`. Also the standalone `mutate_block` (`writes.rs:598`) fires the same variant — pass `&[]` there (scenario/test path with no sources) unless a caller supplies them.

- [ ] **Step 7: Run the test to verify it passes**

Run: `cargo nextest run -p temper-substrate --features artifact-tests create_with_sources_fires_incorporation`
Expected: PASS — the fired `resource_created` payload carries the incorporation on `blocks[0]`.

- [ ] **Step 8: Guard the existing content_mutation + replay suites** (the fire/replay byte-identity invariant must survive the new field)

Run: `cargo nextest run -p temper-substrate --features artifact-tests -E 'test(content_mutation) or test(replay) or test(seed_corpus)'`
Expected: PASS. `incorporated` defaults empty and skip-serializes, so existing byte-identical-replay fixtures are unchanged.

- [ ] **Step 9: `cargo make check` then commit**

```bash
cargo make check
git add crates/temper-substrate/
git commit -m "feat(provenance): thread sources through substrate create/revise payloads

CreateParams/UpdateParams gain `sources`; carried onto BlockManifest.incorporated
and BlockMutated.incorporated (un-stubs events.rs:738). Projectors still ignore it
(Task 3 wires the INSERT). Fire/replay byte-identity preserved (field skip-serializes).

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Migration — both projectors INSERT `kb_block_provenance`; add read function

The load-bearing task. A new additive migration `CREATE OR REPLACE`s `_project_blocks` (create) and `_project_block_mutated` (revise) to read `incorporated` from the payload and INSERT provenance rows, and adds `resource_block_provenance` for the read surface.

**Files:**
- Create: `migrations/20260704000003_block_provenance_write_path.sql`
- Test: `crates/temper-substrate/tests/content_mutation.rs`

**Interfaces:**
- Consumes: payloads carrying `incorporated` (Task 2); the existing `kb_block_provenance` DDL (`20260624000001:603`): columns `block_id, source_kind, source_id, contributed_by_event_id, accretion_seq, is_corrected`, UNIQUE `(block_id, source_kind, source_id, contributed_by_event_id)`.
- Produces: SQL fn `resource_block_provenance(p_resource uuid, p_principal_kind text, p_principal_id uuid) RETURNS TABLE(block_id uuid, block_seq int, source_kind text, source_id uuid, accretion_seq int, contributed_by_event_id uuid, created timestamptz)` — access-gated via `resources_readable_by`, excludes `is_corrected`.

- [ ] **Step 1: Write the failing test** — a create with a resource-source produces a `kb_block_provenance` row and `resource_blocks.reinforce_count` becomes 1.

```rust
// content_mutation.rs
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn create_with_sources_writes_block_provenance(pool: sqlx::PgPool) {
    use temper_substrate::payloads::{Incorporation, ProvenanceSource};
    let f = seed_minimal(&pool).await;
    let src = uuid::Uuid::now_v7();

    temper_substrate::writes::create_resource_with(
        &pool,
        temper_substrate::writes::CreateParams {
            title: "c", origin_uri: "temper://cm/gamma", body: "b", doc_type: "concept",
            home: f.home, owner: f.owner, originator: f.owner, emitter: f.emitter,
            properties: &[], chunks: None,
            sources: vec![Incorporation { source: ProvenanceSource::Resource(src), seq: 0 }],
        },
        temper_substrate::events::EventContext::default(),
    ).await.expect("create");

    let (kind, sid, seq): (String, uuid::Uuid, i32) = sqlx::query_as(
        "SELECT source_kind::text, source_id, accretion_seq FROM kb_block_provenance \
         WHERE NOT is_corrected ORDER BY created DESC LIMIT 1",
    ).fetch_one(&pool).await.unwrap();
    assert_eq!(kind, "resource");
    assert_eq!(sid, src);
    assert_eq!(seq, 0);
}

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn revise_accretes_a_second_source(pool: sqlx::PgPool) {
    // create with source A, then update body with source B → two provenance rows on the same block,
    // reinforce_count == 2 via resource_blocks(...).
    // (Drive create_resource_with then update_resource_with with distinct Resource sources; assert
    //  count(*) over kb_block_provenance for the block == 2 and both source_ids present.)
}
```

> ⚠️ **Plan/reality gap:** `resource_blocks(...)` (the read that exposes `reinforce_count`) requires a principal that can read the resource — call it as `SELECT reinforce_count FROM resource_blocks($resource, 'profile', $owner, NULL)`. The direct `kb_block_provenance` assertion above avoids the access gate for the write-path unit; use `resource_blocks` in the `revise_accretes` test to also prove the read wiring lights up.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-substrate --features artifact-tests -E 'test(create_with_sources_writes_block_provenance)'`
Expected: FAIL — no row in `kb_block_provenance` (projector ignores `incorporated`).

- [ ] **Step 3: Write the migration.** The two projectors are `CREATE OR REPLACE`d to append a provenance-INSERT loop after their existing chunk/revision work. `accretion_seq` := the payload `seq` (caller-declared incorporation order); `contributed_by_event_id` := `p_event`; `source_kind` maps the tagged `{kind}` to the enum. The UNIQUE constraint makes a re-fire idempotent — use `ON CONFLICT DO NOTHING`.

Because `_project_blocks` iterates block manifests and `_project_block_mutated` has a single `block_id`, factor the source-INSERT into a shared helper `_insert_block_provenance(p_block uuid, p_event uuid, p_incorporated jsonb)` so both projectors call one code path (DRY — CLAUDE.md "DRY SQL", the same discipline as `_insert_chunk`).

```sql
-- migrations/20260704000003_block_provenance_write_path.sql
-- Un-stub the block-provenance write path (T7 foundation). Additive: CREATE OR REPLACE the two
-- projectors to record `incorporated` into kb_block_provenance, plus a read fn. No schema change;
-- provenance_source_kind stays ('event','resource') — 'remote' deferred with external ingest.

-- ── shared source-INSERT helper (mirrors _insert_chunk's role) ────────────────
-- accretion_seq = caller-declared incorporation `seq` (replay-stable, from the immutable payload).
-- contributed_by_event_id = the event that added the source. Idempotent on the DDL UNIQUE key.
CREATE FUNCTION _insert_block_provenance(p_block uuid, p_event uuid, p_incorporated jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_inc jsonb;
BEGIN
    IF p_incorporated IS NULL THEN RETURN; END IF;
    FOR v_inc IN SELECT jsonb_array_elements(p_incorporated) LOOP
        INSERT INTO kb_block_provenance
            (block_id, source_kind, source_id, contributed_by_event_id, accretion_seq)
        VALUES (
            p_block,
            (v_inc #>> '{source,kind}')::provenance_source_kind,
            (v_inc #>> '{source,value}')::uuid,
            p_event,
            (v_inc ->> 'seq')::int
        )
        ON CONFLICT (block_id, source_kind, source_id, contributed_by_event_id) DO NOTHING;
    END LOOP;
END;
$$;

-- ── _project_blocks: create path — record per-block incorporation ─────────────
-- (Full body = the current definition from 20260624000002:619, with one added call inside the
--  block loop, right after the kb_block_revisions INSERT. Copy the current body verbatim and add
--  the marked line — do NOT hand-reconstruct the chunk/merkle logic.)
CREATE OR REPLACE FUNCTION _project_blocks(p_resource uuid, p_event uuid, p_manifests jsonb, p_content jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_block uuid; v_chunk uuid;
    v_block_json jsonb; v_chunk_json jsonb; v_side jsonb;
    v_block_hash text; v_chunk_hashes text; v_chunk_count int;
    v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
BEGIN
    FOR v_block_json IN SELECT jsonb_array_elements(p_manifests) LOOP
        v_block := (v_block_json->>'block_id')::uuid;
        -- ... [existing body verbatim: kb_content_blocks INSERT, block_role property, chunk loop,
        --      merkle hash, kb_block_revisions INSERT] ...
        PERFORM _insert_block_provenance(v_block, p_event, v_block_json->'incorporated'); -- ← NEW
    END LOOP;
    PERFORM _recompute_resource_body_hash(p_resource, v_occurred);
END;
$$;

-- ── _project_block_mutated: revise path — accrete incorporation ───────────────
-- Latest definition is the FTS override (20260626000001:85). Copy THAT body verbatim (it has the
-- `_rebuild_resource_search_vector` beat the canonical one lacks) and add the marked line before RETURN.
CREATE OR REPLACE FUNCTION _project_block_mutated(p_event uuid, p_payload jsonb, p_content jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_block    uuid := (p_payload->>'block_id')::uuid;
        v_resource uuid; v_next_ver int;
        v_chunk_json jsonb; v_chunk uuid; v_side jsonb;
        v_chunk_hashes text := ''; v_chunk_count int := 0; v_block_hash text;
BEGIN
    -- ... [existing FTS-override body verbatim through _rebuild_resource_search_vector] ...
    PERFORM _insert_block_provenance(v_block, p_event, p_payload->'incorporated'); -- ← NEW
    RETURN v_block;
END;
$$;

-- ── read surface: itemized per-block provenance for a resource ────────────────
-- Access-gated (resources_readable_by), excludes corrected rows. resource_blocks(...) already
-- exposes the AGGREGATE reinforce_count; this returns the individual source rows the read tool needs.
CREATE FUNCTION resource_block_provenance(
    p_resource uuid, p_principal_kind text, p_principal_id uuid
) RETURNS TABLE(block_id uuid, block_seq int, source_kind text, source_id uuid,
                accretion_seq int, contributed_by_event_id uuid, created timestamptz)
LANGUAGE sql STABLE AS $$
    SELECT b.id, b.seq, p.source_kind::text, p.source_id, p.accretion_seq,
           p.contributed_by_event_id, p.created
    FROM kb_content_blocks b
    JOIN kb_block_provenance p ON p.block_id = b.id AND NOT p.is_corrected
    WHERE b.resource_id = p_resource AND NOT b.is_folded
      AND p_resource IN (SELECT resource_id FROM resources_readable_by(p_principal_kind, p_principal_id))
    ORDER BY b.seq, p.accretion_seq;
$$;
```

> ⚠️ **Plan/reality gap (do not skip):** the two `CREATE OR REPLACE` bodies above are **elided** (`-- ...`). The implementer MUST copy the *current* full bodies — `_project_blocks` from `20260624000002_canonical_functions.sql:619-652`, `_project_block_mutated` from `20260626000001_fts_search_index.sql:85-121` (the FTS override is the live version) — and insert only the one marked `_insert_block_provenance` call. Reconstructing the chunk/merkle/FTS logic by hand is a correctness hazard (GD-1: cite the on-disk body, don't invent it).

- [ ] **Step 4: Run the projection tests to verify they pass**

Run: `cargo nextest run -p temper-substrate --features artifact-tests -E 'test(create_with_sources_writes_block_provenance) or test(revise_accretes_a_second_source)'`
Expected: PASS.

- [ ] **Step 5: Guard replay byte-identity + the full artifact suite** (projectors are replay-critical; provenance must reproduce under replay because it reads only the immutable payload)

Run: `cargo make test-artifacts`
Expected: PASS. Confirm the `replay`/`seed_corpus` equivalence tests are green — `_insert_block_provenance` is a pure function of `(p_event, payload)`, so fire and replay produce identical `kb_block_provenance` rows (ON CONFLICT makes re-projection idempotent).

- [ ] **Step 6: Regenerate the SQL cache**

```bash
cargo sqlx prepare --workspace -- --all-features
```
Expected: `.sqlx/` updates for any new `query!` macros. (The tests above use runtime `query`/`query_as`, so may need none — but run it to be safe.)

- [ ] **Step 7: `cargo make check` then commit**

```bash
cargo make check
git add migrations/20260704000003_block_provenance_write_path.sql crates/temper-substrate/tests/content_mutation.rs .sqlx
git commit -m "feat(provenance): projectors INSERT kb_block_provenance; add read fn

Both _project_blocks (create) and _project_block_mutated (revise) record `incorporated`
into kb_block_provenance via shared _insert_block_provenance. Activates the already-wired
resource_blocks.reinforce_count + cogmap_region_reference_standing signals (zero until now).
New resource_block_provenance read fn (access-gated). Replay-stable + idempotent.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Operations layer — `BodyUpdate.sources` + command threading

Add `sources` to the one continuous body carrier, so both `CreateResource` and `UpdateResource` commands carry it, and `DbBackend` forwards it into `CreateParams`/`UpdateParams`.

**Files:**
- Modify: `crates/temper-workflow/src/operations/inputs.rs:19` (`BodyUpdate`)
- Modify: `crates/temper-services/src/backend/db_backend.rs:855` (create), `:1014` (update)
- Test: `crates/temper-services/` (an existing backend test module) or extend the e2e in Task 8. Prefer a focused DbBackend test if the crate has `#[sqlx::test]` coverage; otherwise rely on Task 8's e2e (note this explicitly in the commit).

**Interfaces:**
- Consumes: `temper_core::types::provenance::ProvenanceSource` (Task 1); `writes::{CreateParams,UpdateParams}.sources: Vec<Incorporation>` (Task 2).
- Produces: `BodyUpdate` gains `pub sources: Vec<ProvenanceSource>` (`#[serde(default)]`). `DbBackend` converts `Vec<ProvenanceSource>` → `Vec<Incorporation>` by zipping with the position index as `seq`.

- [ ] **Step 1: Add the field to `BodyUpdate`** (`operations/inputs.rs:19`):

```rust
pub struct BodyUpdate {
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunks_packed: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<temper_core::types::provenance::ProvenanceSource>, // NEW
}
```

> ⚠️ **Plan/reality gap:** `BodyUpdate::new(content)` is a constructor used by MCP (`resources.rs:662` builds `BodyUpdate::new`). Adding a field means `new` must default `sources` to empty — confirm the constructor and update it (or add a `with_sources`). Grep `BodyUpdate::new` and `BodyUpdate {` across the workspace before editing; every literal-construction site needs the field.

- [ ] **Step 2: Thread into `DbBackend::create_resource`** (`db_backend.rs:855`). Convert the command's `body.sources` (position → `seq`) into `Incorporation`s and pass on `CreateParams`:

```rust
// db_backend.rs — in create_resource, before the writes::create_resource_with call (~:855)
let sources: Vec<temper_substrate::payloads::Incorporation> = cmd
    .body
    .as_ref()
    .map(|b| b.sources.iter().enumerate()
        .map(|(i, s)| temper_substrate::payloads::Incorporation { source: *s, seq: i as i32 })
        .collect())
    .unwrap_or_default();
// ... CreateParams { ..., chunks: incoming_chunks, sources }
```

- [ ] **Step 3: Thread into `DbBackend::update_resource`** (`db_backend.rs:1014`) — same conversion off `cmd.body.sources`, passed on `UpdateParams`.

- [ ] **Step 4: Build + test**

Run: `cargo build -p temper-services && cargo nextest run -p temper-services --features test-db -E 'test(backend)'`
Expected: PASS (or compile-clean if backend provenance coverage lands in Task 8's e2e).

- [ ] **Step 5: `cargo make check` then commit**

```bash
cargo make check
git add crates/temper-workflow/src/operations/inputs.rs crates/temper-services/src/backend/db_backend.rs
git commit -m "feat(provenance): carry sources on BodyUpdate; DbBackend → CreateParams/UpdateParams

Single continuous carrier threads provenance through both create and update commands.
Position in the sources list becomes accretion seq.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: API wire types + handlers + CloudBackend translators

Surface parity part 1 (HTTP). Add `sources` to the create wire type (`IngestPayload`) and the update wire type (`ResourceUpdateRequest`), map them into `BodyUpdate.sources` in the handlers, and carry them on the CLI's outbound DTOs.

**Files:**
- Modify: `crates/temper-core/src/types/ingest.rs:15` (`IngestPayload`)
- Modify: `crates/temper-workflow/src/types/resource.rs:179` (`ResourceUpdateRequest`; ts-rs-derived — regenerates TS types)
- Modify: `crates/temper-api/src/handlers/ingest.rs:87,149`, `handlers/resources.rs:222`
- Modify: `crates/temper-cli/src/cloud_backend/translators.rs:64,150`
- Test: `tests/e2e/tests/block_provenance_test.rs` (added in Task 8; the wire round-trip is proven there)

**Interfaces:**
- Consumes: `ProvenanceSource` (Task 1), `BodyUpdate.sources` (Task 4).
- Produces: `IngestPayload.sources: Vec<ProvenanceSource>` and `ResourceUpdateRequest.sources: Vec<ProvenanceSource>` (both `#[serde(default)]`), mapped into `BodyUpdate.sources` at every handler that builds a `BodyUpdate`.

- [ ] **Step 1: Add the field to `IngestPayload`** (`ingest.rs:15`), `#[serde(default)]` so existing clients/tests keep deserializing:

```rust
#[serde(default)]
pub sources: Vec<crate::types::provenance::ProvenanceSource>,
```

- [ ] **Step 2: Add the field to `ResourceUpdateRequest`** (`resource.rs:179`). It derives `ts_rs::TS` under `typescript` — mark it `#[ts(optional)]`/`#[serde(default)]` consistent with the neighboring optional fields:

```rust
#[serde(default)]
pub sources: Vec<temper_core::types::provenance::ProvenanceSource>,
```

- [ ] **Step 3: Map into `BodyUpdate` at the handlers.**
  - `ingest::create` (`ingest.rs:87`) builds `CreateResource { body: … }` — ensure the constructed `BodyUpdate` carries `payload.sources`.
  - `ingest::update` (`ingest.rs:149`) → `UpdateResource.body` gets `payload.sources`.
  - `resources::update` (`resources.rs:222`) builds `BodyUpdate { content, content_hash, chunks_packed }` — add `sources: req.sources`.

> ⚠️ **Plan/reality gap:** `resources::create` (`resources.rs:150`) passes `body: None` (metadata-only) — it takes `ResourceCreateRequest`, which has no body. Do NOT add sources there this branch (no body block to attribute). The create-with-sources HTTP path is `POST /api/ingest` (`IngestPayload`), which the CLI uses.

- [ ] **Step 4: Carry sources on the CLI's outbound DTOs** (`translators.rs`). Create → `IngestPayload` (`:64`): set `sources` from the command's `BodyUpdate.sources`. Update → `ResourceUpdateRequest` (`:150`): same.

- [ ] **Step 5: Regenerate TS types** (ts-rs change to `ResourceUpdateRequest`)

```bash
cargo make generate-ts-types
```
Expected: `ResourceUpdateRequest.ts` gains a `sources` field. Commit the regenerated types.

- [ ] **Step 6: Build the API + CLI, regenerate api sqlx cache if any query changed**

Run: `cargo build -p temper-api -p temper-cli`
Expected: compile-clean. (No new SQL here — handlers only assemble commands.)

- [ ] **Step 7: `cargo make check` then commit**

```bash
cargo make check
git add crates/temper-core/src/types/ingest.rs crates/temper-workflow/src/types/resource.rs crates/temper-api/src/handlers/ packages/temper-ui/src/lib/types/ crates/temper-cli/src/cloud_backend/translators.rs
git commit -m "feat(provenance): sources on ingest + update wire types; handlers map to BodyUpdate

POST /api/ingest and PATCH /api/resources/{id} accept `sources`; CLI carries them on
outbound DTOs. resources::create (metadata-only) unchanged. TS types regenerated.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: MCP tools — `sources` on create/update + `get_block_provenance` read tool

Surface parity part 2 (MCP). The steward reaches temper over MCP, so this is the tool surface the live steward will use.

**Files:**
- Modify: `crates/temper-mcp/src/tools/resources.rs:27` (`CreateResourceInput`), `:103` (`UpdateResourceInput`), `:309` (create fn maps sources), `:603` (update fn maps sources)
- Modify: the MCP tool registry / dispatch (wherever `get_resource` is registered) to add `get_block_provenance`
- Test: `crates/temper-mcp/` tool tests if present; else the e2e in Task 8

**Interfaces:**
- Consumes: `resource_block_provenance` SQL fn (Task 3); `BodyUpdate.sources` (Task 4).
- Produces: `CreateResourceInput.sources: Option<Vec<Uuid>>`, `UpdateResourceInput.sources: Option<Vec<Uuid>>` (resource ids → `ProvenanceSource::Resource`); a `GetBlockProvenanceInput { resource: Uuid }` tool returning the itemized rows.

- [ ] **Step 1: Add `sources` to the two inputs** (`resources.rs:27,103`). Resource-ids (the MCP caller already addresses resources by UUID):

```rust
// CreateResourceInput / UpdateResourceInput — add:
#[serde(default, skip_serializing_if = "Option::is_none")]
pub sources: Option<Vec<Uuid>>,
```

- [ ] **Step 2: Map into the command's `BodyUpdate.sources`.** In `create_resource` (`:309→:433`) and `update_resource` (`:603→:662`), convert `input.sources` (each `Uuid` → `ProvenanceSource::Resource(id)`) onto the `BodyUpdate`:

```rust
// where BodyUpdate is built (update path builds BodyUpdate::new(content) at :662):
let sources = input.sources.unwrap_or_default().into_iter()
    .map(temper_core::types::provenance::ProvenanceSource::Resource)
    .collect::<Vec<_>>();
// set body.sources = sources  (create path builds body from `content` similarly)
```

> ⚠️ **Plan/reality gap:** the create tool builds `CreateResource { body, … }` at `:433`; confirm whether `body` is `Option<BodyUpdate>` built from `input.content` — if `content` is `None`, there is no block to attribute, so drop the sources with a `BadRequest` ("sources supplied without content") rather than silently discarding them (parse-don't-validate / escalate).

- [ ] **Step 3: Add the `get_block_provenance` read tool.** New input struct + handler that calls `resource_block_provenance` service-direct (reads stay service-direct on both surfaces — CLAUDE.md). Returns a typed `Vec<BlockProvenanceRow>` (define the row struct in `temper-core` or the services read module, ts-rs/schemars-derived — never inline JSON).

```rust
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetBlockProvenanceInput {
    /// The resource whose per-block provenance to read.
    pub resource: Uuid,
}
// handler: resolve principal from auth → call the read service → return rows.
```

- [ ] **Step 4: Register the tool** in the MCP tool list/dispatch next to `get_resource`. Follow the existing registration pattern exactly (grep how `get_resource` is wired).

- [ ] **Step 5: Build + test**

Run: `cargo build -p temper-mcp && cargo nextest run -p temper-mcp`
Expected: compile-clean; existing MCP tests green.

- [ ] **Step 6: `cargo make check` then commit**

```bash
cargo make check
git add crates/temper-mcp/
git commit -m "feat(provenance): MCP sources on create/update + get_block_provenance read tool

The steward's tool surface: create/update accept resource-id sources; new read tool
surfaces itemized per-block provenance service-direct.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: CLI — `--sources` on create/update + `--provenance` on show

Surface parity part 3 (CLI). Lands the foundation of the North Star's `--sources` flag (resource-refs this branch; URLs deferred). Resolves refs via `parse_ref`.

**Files:**
- Modify: `crates/temper-cli/src/cli.rs:298` (`Create` gets `--sources`), `:398` (`Update` gets `--sources`), the `Show` variant (add `--provenance`)
- Modify: `crates/temper-cli/src/commands/resource.rs:160` (`CreateResourceArgs`), `:944` (`UpdateParams`), the show action; resolve `--sources` values via `temper_workflow::operations::parse_ref`
- Test: `tests/e2e/tests/block_provenance_test.rs` (Task 8)

**Interfaces:**
- Consumes: `parse_ref` (pure ref→UUID), the create/update backends carrying `BodyUpdate.sources` (Task 4), `resource_block_provenance` (Task 3).
- Produces: `--sources <comma-separated refs>` on `resource create`/`update`; `--provenance` on `resource show` (like the existing `--edges`).

- [ ] **Step 1: Add the clap flags** (`cli.rs`). On both `Create` (`:298`) and `Update` (`:398`):

```rust
/// Provenance sources this body was distilled from — comma-separated resource refs
/// (UUID or decorated). URLs are deferred (need the 'remote' source kind). Each becomes a
/// block-provenance record on the resource's body block.
#[arg(long, value_delimiter = ',')]
sources: Vec<String>,
```
On `Show`: `#[arg(long)] provenance: bool` (mutually exclusive with `--meta-only`, like `--edges`).

- [ ] **Step 2: Resolve refs → `ProvenanceSource::Resource` in the actions** (`resource.rs`). For each `--sources` value, `parse_ref` → UUID → `ProvenanceSource::Resource`; set `BodyUpdate.sources`. A ref that fails to parse is a hard `BadRequest` (escalate, don't silently drop):

```rust
let sources = args.sources.iter()
    .map(|r| parse_ref(r).map(|id| ProvenanceSource::Resource(id)))
    .collect::<Result<Vec<_>, _>>()?;
```

> ⚠️ **Plan/reality gap:** `--sources` requires a body to attribute. On create, body comes from `--body`/`--from`/stdin; on update likewise. If `--sources` is passed with no body update, error clearly ("--sources requires a body update; add --body/--from or pipe content"). This branch does NOT implement `--content-block` (per-block addressing) — that's the North Star follow-up.

- [ ] **Step 3: Wire `--provenance` on show.** When set, call the read path (`resource_block_provenance` via the backend/services read) and render the itemized rows in the resolved output format (JSON/toon), routed through `output/` (never raw ANSI).

- [ ] **Step 4: Rebuild the CLI binary** (e2e uses a stale bin otherwise — see the e2e-stale-bin gotcha):

```bash
cargo build -p temper-cli --bin temper
```

- [ ] **Step 5: `cargo make check` then commit**

```bash
cargo make check
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/resource.rs
git commit -m "feat(provenance): CLI --sources on create/update, --provenance on show

--sources resolves resource refs via parse_ref → block provenance (URLs deferred to 'remote').
--provenance surfaces itemized per-block provenance. Foundation for the per-content-block
North Star (temper resource update <uuid> --content-block <uuid> --sources ...).

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: E2E round-trip through real Axum + Postgres

Proves the whole spine end-to-end at the production caller's level: the CLI drives `--sources` through `temper-client` → Axum → `DbBackend` → substrate → `kb_block_provenance`, then `resource show --provenance` reads it back. Pairs a direct-call assertion with a caller-level e2e (per the e2e-at-production-caller discipline).

**Files:**
- Create: `tests/e2e/tests/block_provenance_test.rs`
- Modify: `tests/e2e/.sqlx/` (if new macro queries) via `cargo make prepare-e2e`

**Interfaces:**
- Consumes: the full stack from Tasks 1–7.

- [ ] **Step 1: Write the e2e test** — create a "source" resource, then create a "distilled" resource with `--sources <source-ref>` through the CLI code path the harness drives; assert a `kb_block_provenance` row links the distilled block to the source; then update the distilled resource body with a second source and assert accretion (2 rows); then `resource show --provenance` returns both.

```rust
// tests/e2e/tests/block_provenance_test.rs  (sketch — follow common/ harness conventions)
#[tokio::test]
async fn sources_round_trip_through_cli_api_db() {
    let ctx = common::TestServer::spawn().await; // real Axum + test Postgres
    // 1. create source resource → capture its ref/uuid
    // 2. create distilled resource with --sources <source_uuid> and a body
    // 3. assert one kb_block_provenance row (source_kind='resource', source_id=source_uuid)
    // 4. update distilled body with --sources <source2_uuid> → assert 2 rows (accretion)
    // 5. `resource show <distilled> --provenance` → both sources present, ordered by accretion_seq
}
```

> ⚠️ **Plan/reality gap:** this suite exercises the embed pipeline (body chunks) — it likely needs `test-embed`. Run under `cargo make test-e2e-embed`, not plain `test-e2e`, and gate the file/test with the embed feature if the harness requires it (see the embed-gated-e2e note in CLAUDE.md).

- [ ] **Step 2: Run the e2e**

```bash
cargo build -p temper-cli --bin temper   # ensure fresh bin
cargo make test-e2e-embed 2>&1 | tee /tmp/e2e-prov.log
```
Expected: the new test PASSES; grep the log for `FAIL [` / `error: test run failed` (don't trust the per-binary Summary line).

- [ ] **Step 3: Regenerate e2e sqlx cache if needed**

```bash
cargo make prepare-e2e
```

- [ ] **Step 4: `cargo make check` then commit**

```bash
cargo make check
git add tests/e2e/tests/block_provenance_test.rs tests/e2e/.sqlx
git commit -m "test(provenance): e2e round-trip CLI --sources → API → DB → --provenance readback

Proves the full spine at the production caller's level: create-with-sources, revise-accretes,
and show --provenance readback through real Axum + Postgres.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

---

# Phase C (T7c) — `remote` sources + per-content-block addressing + steward wiring

> Design granularity. Expand each task to full TDD steps when T7c is picked up — **after** Task 9's schema fork is decided. Phase C depends on B being merged.

## Task 9: `'remote'` source kind — schema + type

**DECIDED: Option A — a `kb_remote_sources` table** (mint a UUID per distinct URL; `source_id` references it). Chosen because it preserves the uniform "`source_id` is always a UUID" invariant that the projectors, read fn, and UNIQUE key all lean on — so the migration stays purely additive (new table + one enum value) with no column-nullability churn — and it makes a remote source a first-class addressable thing, consistent with how resources/events are referenced. (Rejected alternative: a nullable `source_uri` column on `kb_block_provenance` — lighter but breaks the `source_id NOT NULL` invariant and forces a UNIQUE-key rework.)

**URL normalization (find-by / dedup key).** `kb_remote_sources` stores the URL twice: the raw value as given, and a **normalized** canonical form that is the UNIQUE / dedup key — so identical-intent URLs collapse to one row and a lookup is deterministic (the old-URL-shortener move: normalize, then key on the normalized form).

```
CREATE TABLE kb_remote_sources (
    id             UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    uri            TEXT NOT NULL,               -- the URL as supplied (display / audit)
    uri_normalized TEXT NOT NULL UNIQUE,        -- canonical dedup + find-by key
    first_seen     TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

Normalization is **simple and total** (no network, replay-stable — it must be a pure string function so a re-projection yields the identical `uri_normalized`): trim leading/trailing whitespace, lowercase the scheme + host, drop a trailing slash on an empty path, collapse `http(s)://` default ports. Keep it conservative — do NOT strip query strings or fragments (they can be semantically load-bearing). Land it as one helper `normalize_remote_uri(text) → text` (SQL or a `temper-core` pure fn re-used by both the CLI validator and the projector) so send-side and write-side can never disagree (the symmetric-defense pattern). A short base62 token for shortener-style lookup is a **nice-to-have, deferred** — `uri_normalized UNIQUE` already gives deterministic find-by.

**Scope:**
- Migration: `ALTER TYPE provenance_source_kind ADD VALUE 'remote';` in its own statement, **committed before any migration/query uses the value** — Postgres can't add an enum value and use it in the same transaction. Plus `CREATE TABLE kb_remote_sources` + an `_upsert_remote_source(p_uri text) → uuid` helper (`INSERT … ON CONFLICT (uri_normalized) DO UPDATE … RETURNING id`, normalizing `p_uri` first) called from `_insert_block_provenance` when `kind='remote'`.
- `ProvenanceSource` (temper-core): add a `Remote(String)` variant — the URL rides the tagged wire shape (`{"kind":"remote","value":"https://…"}`); the projector resolves it to a `kb_remote_sources.id` at write time via `_upsert_remote_source`.
- Read: `resource_block_provenance` returns the resolved `uri` (raw form) for `'remote'` rows (LEFT JOIN `kb_remote_sources`), so the read surfaces the human URL, not the minted UUID.

## Task 10: URL sources on `--sources` (CLI + MCP + API)

- CLI `--sources` (`cli.rs`): accept URL values alongside refs — a value parsing as a URL → `ProvenanceSource::Remote(url)`, else `parse_ref` → `Resource`. (This is the "jsonified-safe-strings-or-comma-sep-list-of-urls" shape from the North Star.)
- MCP: the `sources` input widens from `Vec<Uuid>` to the tagged `Vec<ProvenanceSource>` (or a `Vec<String>` resolved the same way as the CLI).
- API wire (`IngestPayload.sources`, `ResourceUpdateRequest.sources`) already carry `Vec<ProvenanceSource>` (Phase B) — the `Remote` variant flows with no wire change.

## Task 11: Per-content-block addressing — `--content-block <uuid>` on update

The North Star update semantics. Phases A/B apply resource-level sources to the single body block; this adds addressing a specific block.

- CLI `resource update`: `--content-block <uuid>` targets one `kb_content_blocks` row; `--sources` (and optional `--body`) apply to *that* block.
- MCP `update_resource` / API `ResourceUpdateRequest`: add an optional `content_block: Option<Uuid>`; when present, the update targets that block instead of the resource's default body block.
- `DbBackend::update_resource` / `writes::update_resource_in_tx`: resolve the target block from `content_block` when supplied (today it resolves "the single non-folded body block" at `writes.rs:~230`); validate the block belongs to the resource and is non-folded (parse-don't-validate; escalate on mismatch).

## Task 12: Steward agent wiring

The steward already writes the resource-level `derived_from` edge (MVP). Now it also attributes block provenance.

- `agent-workflows/steward/`: when the steward distills a concept/fact/question from N source resources, it calls the MCP `create_resource`/`update_resource` with `sources: [<source resource ids>]` (and, once Task 10 lands, remote URLs for external sources).
- Verify against the live cogmap (`019f2391`): a steward tick that distills from ≥1 source produces `kb_block_provenance` rows, and `cogmap_region_reference_standing` reflects them in region salience.

---

## Final verification (before PR)

- [ ] Full workspace test sweep: `cargo make test-all`
- [ ] Embed + artifacts jobs locally: `cargo make test-artifacts && cargo make test-e2e-embed`
- [ ] SQL caches current: `cargo sqlx prepare --workspace -- --all-features && cargo make prepare-services && cargo make prepare-api && cargo make prepare-e2e` (prune any orphaned `.sqlx` files)
- [ ] `cargo make check` clean
- [ ] Update the T7 task (`019f1ad3`) to `done`; note in the goal that the block-provenance foundation shipped and the per-content-block `--content-block`/`--sources` North Star + `'remote'` source kind remain as the next increment.

---

## Self-Review

**Spec coverage** (against the T7 task body):
- "Un-stub the write path: thread `incorporated` through the block write into a `kb_block_provenance` INSERT" → Tasks 2 (thread) + 3 (INSERT). ✓
- "Expose source attribution on the create/block MCP tools" → Task 6. ✓ (+ CLI Task 7, API Task 5 for full parity per the always-parity rule.)
- "Add a read tool to surface per-block provenance" → Task 3 (SQL fn) + Task 6 (MCP tool) + Task 7 (`--provenance`). ✓
- "`provenance_source_kind` extend with `'remote'`/`'external'`" → **IN scope, Phase C Task 9** (un-YAGNI'd; carries a schema-design fork). ✓
- "Steward writes BOTH the resource-level `derived_from` edge AND block-level provenance" → the steward already writes `derived_from` (MVP); Phases A/B give it the block-provenance channel, Phase C Task 12 wires the steward agent to call it. ✓

**Decisions locked** (from the scoping conversation): D1 = **both paths** (create + revise projectors) ✓; D3 = **resource-level surface in A/B**, per-block North Star = Phase C Task 11 ✓. **`'remote'` = in scope, Phase C** ✓. **`'remote'` storage = `kb_remote_sources` table with a normalized-URI unique key** (Task 9) ✓ — no open decisions remain.

**Placeholder scan:** the two `CREATE OR REPLACE` bodies in Task 3 are deliberately elided with an explicit ⚠️ instruction to copy the on-disk bodies verbatim (GD-1) — this is a grounding directive, not a placeholder. All other steps carry concrete code or exact field/line targets.

**Type consistency:** `ProvenanceSource` (core) → `Incorporation { source, seq }` (substrate) → `BlockManifest.incorporated` / `BlockMutated.incorporated` (payload) → `kb_block_provenance` (`source_kind`, `source_id`, `accretion_seq`). `BodyUpdate.sources: Vec<ProvenanceSource>` (surface/command) → `Incorporation` (DbBackend zips index as `seq`). Names consistent across tasks.

**Open risk to confirm at execution (plan/reality):** the borrowed-lifetime shape of `SeedAction::BlockMutate` (Task 2 Step 6) and every literal construction site of `BodyUpdate`/`CreateParams`/`UpdateParams` (grep before editing). The controller must grep-verify these at dispatch (plan-verification discipline).
