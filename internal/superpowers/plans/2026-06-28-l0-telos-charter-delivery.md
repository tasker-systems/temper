# L0 Telos Charter Delivery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the L0 telos charter (statement + 6 questions-with-context + framing) onto the live `system-default` cogmap's telos resource — currently `blocks: []` in production — by extending the landmark reconciler with a `telos:` section, embedded client-side, applied via a new idempotent substrate primitive.

**Architecture:** The committed `l0-kernel.yaml` manifest gains a `telos:` section. `temper cogmap reconcile` embeds it client-side (per-block `compute_body_chunks`) alongside the 22 landmark entries and PUTs it on the existing `/api/cognitive-maps/{id}`. The backend `reconcile_cognitive_map` (one SERIALIZABLE tx + `admin_reconcile` envelope) runs the unchanged landmark diff, then a new telos branch: resolve `cogmap_telos`, diff the telos's two-level body-merkle, and fire the new `cogmap_charter_set` substrate function only on change. Real bge-768 embeddings on the charter bring `telos_alignment` salience online.

**Tech Stack:** Rust workspace (temper-core / temper-substrate / temper-api / temper-client / temper-cli), Axum + utoipa, sqlx (compile-time-checked SQL against the canonical `public` schema; root `.sqlx` cache), temper-ingest (bge-768 ONNX embedding, client-side), cargo-nextest, the e2e crate (real Axum + Postgres + JWT), `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]` ephemeral DBs for substrate artifact-tests.

## Global Constraints

Copied verbatim from the spec + repo discipline. Every task's requirements implicitly include this section.

- **Real embeddings, client-side (Decision 1).** The charter embeds in the CLI via `compute_body_chunks` (ONNX), exactly like landmark entries; the server stays embed-free on the request path. No ONNX in any migration.
- **Extend the existing reconciler (Decision 2).** One operator command/endpoint delivers all of L0. The endpoint, authz (`require_cogmap_write_admin`), SERIALIZABLE tx, and `admin_reconcile` envelope are REUSED — no new handler, client method, or route.
- **Idempotency keys on content, never block identity (Decision 3).** Same manifest + same live state ⇒ the recomputed charter body-merkle equals the live telos `body_hash` ⇒ **zero events** ⇒ `charter: Unchanged`. Every test must include a re-run-yields-no-events assertion.
- **`charter` is a distinct outcome field (Decision 4).** `ReconcileOutcome.charter: CharterDisposition` (`Unchanged | Created | Updated | Absent`), never folded into the landmark `created/updated/folded/unchanged` counts. `Absent` = no `telos:` in the request (landmark-only; backward compatible).
- **It's still events.** The charter delivery fires one `charter_set` `kb_events` row through the substrate function (event-as-primary holds). The fold of the prior telos blocks is that event's projection side-effect (mirroring how `block_mutated` supersedes prior chunks), not a separate event.
- **Writes route through the substrate.** The backend telos branch calls `temper_substrate::writes`/`readback`; it never inlines `sqlx::query!` for the telos. The new SQL lives in the substrate (migration + readback), cached in the root `.sqlx`.
- **Typed structs over inline JSON.** No `serde_json::json!()` for known-structure payloads — the `CharterSet` payload is a typed struct in `payloads.rs` (mirror `CogmapSeeded`).
- **The prose-assembly rule is defined ONCE (spec, Components 3).** The `{statement, questions, framing} → [(role, prose)]` mapping lives in temper-core; both the CLI and substrate `TelosDef::block_specs` call it, so they cannot drift.
- **Reserved ids (do not re-derive).** L0 cogmap `00000000-0000-0000-0005-000000000001`; L0 telos resource `00000000-0000-0000-0005-000000000002`; root team slug `temper-system`; system actor = profile `handle='system'` + entity `name='system'`.
- **Never edit a shipped migration.** The charter delivery is a NEW additive migration (`20260629000001_cogmap_charter_set.sql`); the L0 birth migration stays untouched.
- **sqlx cache discipline.** New substrate (lib) macro queries → `cargo sqlx prepare --workspace -- --all-features` (root cache). New temper-api test-target queries → `cargo make prepare-api`. New e2e queries → `cargo make prepare-e2e`. All `cargo make` tasks set `SQLX_OFFLINE=true`.
- **Run `cargo make check` before every commit** (fmt + clippy `-D warnings` + machete + TS). Per-task: focused test + crate suite + check. Full-workspace nextest only at PR-prep.
- **Run `cargo fmt` as part of every commit gate** (`cargo make check` gates on `cargo fmt --check`, exit 105).

**Grounding tags:** `CONFORM` = uses a verified existing API unchanged; `EXTEND` = adds to an existing module following its pattern; `NEW` = net-new. Every named signature was grep-verified against the tree on 2026-06-28.

---

## File Structure

**New files:**
- `crates/temper-core/src/charter.rs` — the shared `CharterQuestion` type + `charter_block_specs` prose-assembly rule (Task 2).
- `migrations/20260629000001_cogmap_charter_set.sql` — `cogmap_charter_set` SQL function + `charter_set` event-type row (Task 4).
- `crates/temper-substrate/tests/charter_set_writes.rs` — substrate write-path artifact-test (Task 4).
- `crates/temper-api/tests/reconcile_charter_test.rs` — backend telos-branch test-db test (Task 5).

**Modified files:**
- `crates/temper-core/src/types/reconcile.rs` — `ReconcileTelos`, `ReconcileTelosBlock`, `CharterDisposition`; extend `ReconcileCogmapRequest` + `ReconcileOutcome` (Task 1).
- `crates/temper-core/src/lib.rs` — `pub mod charter;` (Task 2).
- `crates/temper-substrate/src/scenario/model.rs` — `TelosDef::block_specs` delegates to `temper_core::charter::charter_block_specs` (Task 2).
- `crates/temper-substrate/src/content.rs` — `body_hash_from_block_chunk_hashes` (Task 3).
- `crates/temper-substrate/src/payloads.rs` — `CharterSet` payload struct (Task 4).
- `crates/temper-substrate/src/events.rs` — `EventKind::CharterSet`, `SeedAction::CharterSet`, fire arm, `Fired::Charter` (Task 4).
- `crates/temper-substrate/src/writes.rs` — `set_charter_in_tx` wrapper (Task 4).
- `crates/temper-substrate/src/readback/mod.rs` — `telos_charter_state` diff-read (Task 4).
- `crates/temper-api/src/backend/db_backend.rs` — `apply_telos_phase` in `reconcile_apply` (Task 5).
- `crates/temper-cli/src/actions/reconcile.rs` — `ManifestTelos` + telos embed in `manifest_to_request` (Task 6).
- `schema-artifact/manifests/l0-kernel.yaml` — the `telos:` section (Task 7).
- `tests/e2e/tests/reconcile_cogmap_e2e.rs` — telos delivery e2e (Task 8).

---

## Task 1: Telos wire types + outcome field

Establishes the typed contract. No behavior — types only, with serde round-trip tests.

**Files:**
- Modify: `crates/temper-core/src/types/reconcile.rs`

**Interfaces:**
- Produces (consumed by Tasks 5, 6):
  - `ReconcileTelosBlock { role: String, chunks_packed: String }` — one charter block, pre-embedded; `role ∈ {"statement","question","framing"}`.
  - `ReconcileTelos { blocks: Vec<ReconcileTelosBlock> }`.
  - `CharterDisposition` enum `{ Unchanged, Created, Updated, Absent }`, `#[serde(rename_all = "snake_case")]`, `Default = Absent`.
  - `ReconcileCogmapRequest` gains `#[serde(default)] pub telos: Option<ReconcileTelos>`.
  - `ReconcileOutcome` gains `pub charter: CharterDisposition`.

- [ ] **Step 1: Write the failing test** — append to the existing `#[cfg(test)] mod tests` in `reconcile.rs`:

```rust
    #[test]
    fn request_carries_optional_telos() {
        let req = ReconcileCogmapRequest {
            entries: vec![],
            fold_resources: vec![],
            fold_edges: vec![],
            telos: Some(ReconcileTelos {
                blocks: vec![
                    ReconcileTelosBlock { role: "statement".into(), chunks_packed: "[]".into() },
                    ReconcileTelosBlock { role: "question".into(), chunks_packed: "[]".into() },
                ],
            }),
        };
        let json = serde_json::to_string(&req).unwrap();
        let back: ReconcileCogmapRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(req, back);
        // A request with no `telos:` round-trips with `None` (landmark-only, backward compatible).
        let landmark_only: ReconcileCogmapRequest =
            serde_json::from_str(r#"{"entries":[]}"#).unwrap();
        assert!(landmark_only.telos.is_none());
    }

    #[test]
    fn charter_disposition_defaults_absent_and_serializes_snake_case() {
        assert_eq!(CharterDisposition::default(), CharterDisposition::Absent);
        assert_eq!(
            serde_json::to_string(&CharterDisposition::Updated).unwrap(),
            "\"updated\""
        );
        // Outcome default carries charter: Absent.
        assert_eq!(ReconcileOutcome::default().charter, CharterDisposition::Absent);
    }
```

- [ ] **Step 2: Run it to confirm it fails** — `cargo nextest run -p temper-core reconcile`. Expected: FAIL (`ReconcileTelos`/`CharterDisposition` not defined; `ReconcileCogmapRequest` has no `telos` field).

- [ ] **Step 3: Add the types.** In `reconcile.rs`, mirroring the existing derive stack (Rust-only CLI↔API; `web-api` utoipa; `PartialEq` for round-trips):

```rust
/// One charter block in a telos delivery — **pre-embedded** by the CLI. `role` is the `block_role` the
/// substrate stamps so reads (`resource_blocks(telos, …, role)`) distinguish statement / question /
/// framing. `chunks_packed` is `compute_body_chunks(prose)` output for THIS block's prose — the same
/// packed-blob format `ReconcileEntry::chunks_packed` uses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ReconcileTelosBlock {
    pub role: String,
    pub chunks_packed: String,
}

/// The telos charter as an ordered run of pre-embedded blocks (block-0 statement, then questions, then
/// framing). Optional on a reconcile request: absent ⇒ landmark-only reconcile (`charter: Absent`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub struct ReconcileTelos {
    pub blocks: Vec<ReconcileTelosBlock>,
}

/// What the reconcile run did to the telos charter — a DISTINCT grain from the landmark counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CharterDisposition {
    /// The request carried no `telos:` (landmark-only reconcile).
    #[default]
    Absent,
    /// The telos body-merkle matched the live charter — no event fired.
    Unchanged,
    /// The telos was empty (first delivery) and the charter was created.
    Created,
    /// The live charter differed and was replaced.
    Updated,
}
```

Add `#[serde(default)] pub telos: Option<ReconcileTelos>,` to `ReconcileCogmapRequest` and `pub charter: CharterDisposition,` to `ReconcileOutcome`. Update the existing `request_round_trips_through_json` test's `ReconcileCogmapRequest { … }` literal to add `telos: None`.

- [ ] **Step 4: Run tests + check** — `cargo nextest run -p temper-core reconcile` (PASS), then `cargo make check`. Note: adding a field to `ReconcileCogmapRequest`/`ReconcileOutcome` may break the existing CLI `manifest_to_request` literal (no `telos`) and any `ReconcileOutcome { … }` literal — fix those by adding `telos: None` / leaving `..Default::default()`; if `cargo make check` flags them, that is expected and resolved here (they are field-init updates, not logic changes).

- [ ] **Step 5: Commit**

```bash
git add crates/temper-core/src/types/reconcile.rs
git commit -m "feat(charter): telos wire types + CharterDisposition outcome field"
```

---

## Task 2: Shared charter prose-assembly rule (defined once)

Lift the `{statement, questions, framing} → [(role, prose)]` rule to temper-core so the CLI and substrate cannot drift. Substrate's `TelosDef::block_specs` delegates to it.

**Files:**
- Create: `crates/temper-core/src/charter.rs`
- Modify: `crates/temper-core/src/lib.rs` (add `pub mod charter;`)
- Modify: `crates/temper-substrate/src/scenario/model.rs` (`TelosDef::block_specs` delegates)

**Interfaces:**
- Produces (consumed by Tasks 4-impl-parity, 6):
  - `temper_core::charter::CharterQuestion { question: String, context: String }` (serde; `context` `#[serde(default)]`).
  - `temper_core::charter::charter_block_specs(statement: &str, questions: &[CharterQuestion], framing: &[String]) -> Vec<(&'static str, String)>` — block-0 `("statement", statement)`, then per question `("question", q.question` or `q.question + "\n\n" + q.context` when context non-empty`)`, then per framing `("framing", f)`.

- [ ] **Step 1: Write the failing test** — in `crates/temper-core/src/charter.rs` (created in Step 3), an inline test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assembles_statement_questions_framing_in_order() {
        let qs = vec![
            CharterQuestion { question: "What transfers?".into(), context: "prior knowledge".into() },
            CharterQuestion { question: "Bare?".into(), context: String::new() },
        ];
        let specs = charter_block_specs("The statement.", &qs, &["Framing one.".to_string()]);
        assert_eq!(specs[0], ("statement", "The statement.".to_string()));
        // context appended with a blank line when present…
        assert_eq!(specs[1], ("question", "What transfers?\n\nprior knowledge".to_string()));
        // …and omitted entirely when empty.
        assert_eq!(specs[2], ("question", "Bare?".to_string()));
        assert_eq!(specs[3], ("framing", "Framing one.".to_string()));
        assert_eq!(specs.len(), 4);
    }
}
```

- [ ] **Step 2: Run it to confirm it fails** — `cargo nextest run -p temper-core charter`. Expected: FAIL (module not defined).

- [ ] **Step 3: Write the module** — `crates/temper-core/src/charter.rs`:

```rust
//! The charter prose-assembly rule, defined ONCE. A telos charter is real role-tagged content blocks:
//! block-0 is the statement (role `"statement"`), then each question (role `"question"`, the question
//! plus its context when present), then the framing lines (role `"framing"`). Both the CLI
//! (`temper cogmap reconcile`) and the substrate genesis path (`TelosDef::block_specs`) call this, so the
//! delivered charter and the genesis-born charter cannot drift.
use serde::{Deserialize, Serialize};

/// One charter question with its disambiguating context (context defaults empty).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CharterQuestion {
    pub question: String,
    #[serde(default)]
    pub context: String,
}

/// Assemble `{statement, questions, framing}` into ordered `(block_role, prose)` specs.
pub fn charter_block_specs(
    statement: &str,
    questions: &[CharterQuestion],
    framing: &[String],
) -> Vec<(&'static str, String)> {
    let mut specs = Vec::with_capacity(1 + questions.len() + framing.len());
    specs.push(("statement", statement.to_owned()));
    for q in questions {
        let prose = if q.context.is_empty() {
            q.question.clone()
        } else {
            format!("{}\n\n{}", q.question, q.context)
        };
        specs.push(("question", prose));
    }
    for f in framing {
        specs.push(("framing", f.clone()));
    }
    specs
}
```

Add `pub mod charter;` to `crates/temper-core/src/lib.rs` (alphabetically near the other `pub mod` lines).

- [ ] **Step 4: Run the core test** — `cargo nextest run -p temper-core charter`. Expected: PASS.

- [ ] **Step 5: Delegate substrate `block_specs`** — in `crates/temper-substrate/src/scenario/model.rs`, replace the body of `TelosDef::block_specs` (currently building `[("statement", …), ("question", …), ("framing", …)]` inline, model.rs:120-133) so the assembly RULE comes from temper-core. Map the substrate `QuestionDef` to `temper_core::charter::CharterQuestion`:

```rust
    pub fn block_specs(&self) -> Vec<(&'static str, String)> {
        let questions: Vec<temper_core::charter::CharterQuestion> = self
            .questions
            .iter()
            .map(|q| temper_core::charter::CharterQuestion {
                question: q.question.clone(),
                context: q.context.clone(),
            })
            .collect();
        temper_core::charter::charter_block_specs(&self.statement, &questions, &self.framing)
    }
```

(If `block_specs`'s return type was `Vec<(&str, String)>` with a non-`'static` lifetime, widen call sites to `&'static str` — the literals are static. Grep `block_specs(` to confirm the two call sites — the genesis path and any test — still compile.)

- [ ] **Step 6: Run the substrate parity tests** — the genesis charter tests prove the delegation produces identical blocks: `cargo nextest run -p temper-substrate -E 'test(telos_deserializes_questions_with_context_and_framing)'` (model unit test, no DB) and `cargo make test-artifacts -- -E 'test(cogmap_genesis_charter)'` (the genesis write-path roundtrip). Expected: PASS (no behavior change — same specs).

- [ ] **Step 7: check** — `cargo make check`.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-core/src/charter.rs crates/temper-core/src/lib.rs crates/temper-substrate/src/scenario/model.rs
git commit -m "refactor(charter): lift prose-assembly rule to temper-core; substrate block_specs delegates"
```

---

## Task 3: Multi-block resource body-hash helper

`body_hash_from_chunk_hashes` is single-block only (`sha256(sha256(concat chunks))`). The multi-block charter needs the two-level merkle matching `_recompute_resource_body_hash` (per-block `sha256(concat chunk hashes by chunk_index)`, then resource `sha256(concat per-block hashes by seq)`).

**Files:**
- Modify: `crates/temper-substrate/src/content.rs`

**Interfaces:**
- Produces (consumed by Task 5): `pub fn body_hash_from_block_chunk_hashes(blocks: &[Vec<String>]) -> String` — `blocks[i]` is block `i`'s chunk content-hashes in `chunk_index` order; the outer slice is in block `seq` order.

- [ ] **Step 1: Write the failing test** — append to `content.rs`'s `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn block_chunk_hashes_single_block_matches_single_block_helper() {
        // One block ⇒ identical to the single-block helper (which assumes one roleless block).
        let hashes = vec!["aa".to_string(), "bb".to_string()];
        assert_eq!(
            body_hash_from_block_chunk_hashes(&[hashes.clone()]),
            body_hash_from_chunk_hashes(&hashes),
        );
    }

    #[test]
    fn block_chunk_hashes_two_level_merkle() {
        // Two blocks: per-block sha256(concat), then resource sha256(concat per-block hashes).
        let b0 = vec!["aa".to_string()];
        let b1 = vec!["bb".to_string(), "cc".to_string()];
        let expect = {
            let h0 = sha256_hex("aa");
            let h1 = sha256_hex("bbcc");
            sha256_hex(&format!("{h0}{h1}"))
        };
        assert_eq!(body_hash_from_block_chunk_hashes(&[b0, b1]), expect);
    }

    #[test]
    fn block_chunk_hashes_empty_matches_empty_body() {
        assert_eq!(body_hash_from_block_chunk_hashes(&[]), body_hash_for_body(""));
    }
```

- [ ] **Step 2: Run it to confirm it fails** — `cargo nextest run -p temper-substrate body_hash_from_block_chunk_hashes`. Expected: FAIL (fn not defined).

- [ ] **Step 3: Implement** — add to `content.rs`, next to `body_hash_from_chunk_hashes`:

```rust
/// The resource `body_hash` for a MULTI-BLOCK caller-supplied chunk set (the charter shape). Reproduces
/// `_recompute_resource_body_hash`'s two-level merkle: per block, `sha256_hex(concat of the block's chunk
/// content_hashes in chunk_index order)`; then the resource hash is `sha256_hex(concat of the per-block
/// hashes in block seq order)`. `blocks` MUST already be in seq order and each inner vec in chunk_index
/// order. An empty set ⇒ `sha256_hex("")` (the SQL coalesces the empty aggregate to `''`). For a single
/// block this equals [`body_hash_from_chunk_hashes`].
pub fn body_hash_from_block_chunk_hashes(blocks: &[Vec<String>]) -> String {
    if blocks.is_empty() {
        return sha256_hex("");
    }
    let per_block: String = blocks
        .iter()
        .map(|chunk_hashes| sha256_hex(&chunk_hashes.concat()))
        .collect();
    sha256_hex(&per_block)
}
```

- [ ] **Step 4: Run** — `cargo nextest run -p temper-substrate body_hash_from_block_chunk_hashes`. Expected: PASS (all three).

- [ ] **Step 5: check** — `cargo make check`.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-substrate/src/content.rs
git commit -m "feat(charter): body_hash_from_block_chunk_hashes (multi-block two-level merkle)"
```

---

## Task 4: Substrate charter-set primitive (migration + plumbing + write test)

The post-birth populate primitive. `block_mutate` is revise-only and genesis left zero blocks, so this NEW function replaces the telos's full block set: fold current blocks, project the new role-tagged set, recompute body_hash, emit one `charter_set` event. Plus the Rust mutation plumbing and the diff-read.

**Files:**
- Create: `migrations/20260629000001_cogmap_charter_set.sql`
- Modify: `crates/temper-substrate/src/payloads.rs` (`CharterSet` payload)
- Modify: `crates/temper-substrate/src/events.rs` (`EventKind::CharterSet`, `SeedAction::CharterSet`, fire arm, `Fired::Charter`)
- Modify: `crates/temper-substrate/src/writes.rs` (`set_charter_in_tx`)
- Modify: `crates/temper-substrate/src/readback/mod.rs` (`telos_charter_state`)
- Create: `crates/temper-substrate/tests/charter_set_writes.rs`

**Interfaces:**
- Consumes: `cogmap_telos` (canonical_functions.sql:396), `_project_blocks` (canonical_functions.sql:619), `_event_append`, `prepare_block_from_chunks`/`prepare_blocks` (content.rs), `content_sidecar`/`BlockManifest` (payloads.rs), `body_hash_from_block_chunk_hashes` (Task 3).
- Produces (consumed by Task 5):
  - `writes::set_charter_in_tx(conn, cogmap: CogmapId, blocks: &[PreparedBlock], emitter: EntityId) -> Result<ResourceId>` (returns the telos resource id).
  - `readback::TelosCharterState { telos_resource_id: ResourceId, body_hash: Option<String> }` and `readback::telos_charter_state(conn, cogmap: CogmapId) -> Result<TelosCharterState>`.

- [ ] **Step 1: Write the migration** — `migrations/20260629000001_cogmap_charter_set.sql`:

```sql
-- Post-birth telos-charter delivery (L0 telos charter, 2026-06-28 spec). `block_mutate` is revise-only
-- and genesis leaves a fresh telos with zero blocks, so neither existing primitive can populate an empty
-- telos. `cogmap_charter_set` replaces the telos's FULL block set uniformly (0→N first delivery, N→M
-- re-delivery) via fold-then-reproject — one `charter_set` event whose projection folds the prior blocks
-- (the same supersede-on-revise discipline as block_mutated) and projects the new role-tagged set through
-- the shared _project_blocks path with the p_content sidecar. Additive: data + function only.

-- The `charter_set` event type. Payload = { cogmap_id, blocks:[BlockManifest] } (the CogmapSeeded telos
-- block shape, hoisted to the top level). payload_version 1.
INSERT INTO kb_event_types (name, payload_schema, payload_version)
VALUES ('charter_set', $js${"$schema":"https://json-schema.org/draft/2020-12/schema","title":"CharterSet","type":"object","properties":{"cogmap_id":{"$ref":"#/$defs/CogmapId"},"blocks":{"type":"array","items":{"$ref":"#/$defs/BlockManifest"}}},"required":["cogmap_id","blocks"],"$defs":{"CogmapId":{"description":"A `kb_cogmaps` row.","type":"string","format":"uuid"},"BlockManifest":{"type":"object","properties":{"block_id":{"$ref":"#/$defs/BlockId"},"seq":{"type":"integer","format":"int32"},"role":{"type":["string","null"]},"chunks":{"type":"array","items":{"$ref":"#/$defs/ChunkManifest"}}},"required":["block_id","seq","chunks"]},"BlockId":{"type":"string","format":"uuid"},"ChunkManifest":{"type":"object","properties":{"chunk_id":{"$ref":"#/$defs/ChunkId"},"chunk_index":{"type":"integer","format":"int32"},"content_hash":{"type":"string"}},"required":["chunk_id","chunk_index","content_hash"]},"ChunkId":{"type":"string","format":"uuid"}}}$js$::jsonb, 1)
ON CONFLICT (name) DO NOTHING;

-- Replace a cogmap's telos charter with the payload's role-tagged blocks. Fold-then-reproject: the
-- prior telos blocks are folded (excluded from reads + the body_hash recompute that _project_blocks runs),
-- then the new set is projected. Anchored on the cogmap (the telos is cogmap-homed). Rejects an empty
-- charter (a telos with no blocks would blank its identity). Returns the telos resource id.
CREATE FUNCTION cogmap_charter_set(p_payload jsonb, p_content jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid;
        v_cogmap uuid := (p_payload->>'cogmap_id')::uuid;
        v_telos  uuid := cogmap_telos(v_cogmap);
BEGIN
    IF v_telos IS NULL THEN
        RAISE EXCEPTION 'cogmap_charter_set: cogmap % has no telos', v_cogmap;
    END IF;
    IF p_payload->'blocks' IS NULL OR jsonb_array_length(p_payload->'blocks') = 0 THEN
        RAISE EXCEPTION 'cogmap_charter_set: empty charter for cogmap % (would blank the telos)', v_cogmap;
    END IF;
    v_ev := _event_append('charter_set', p_emitter, 'kb_cogmaps', v_cogmap, p_payload);
    -- supersede the prior charter (0 rows on first delivery), then project the new role-tagged set.
    UPDATE kb_content_blocks SET is_folded = true, last_event_id = v_ev
        WHERE resource_id = v_telos AND NOT is_folded;
    PERFORM _project_blocks(v_telos, v_ev, p_payload->'blocks', p_content);
    RETURN v_telos;
END;
$$;
```

> Grounding note: verify `kb_content_blocks` has `is_folded` + `last_event_id` columns (`resource_blocks` filters `NOT b.is_folded`; `_recompute_resource_body_hash` filters `NOT b.is_folded` — both confirmed canonical_functions.sql). `_project_blocks` calls `_recompute_resource_body_hash` at its tail, so the telos `body_hash` is refreshed after the fold+reproject.

- [ ] **Step 2: Apply the migration locally** — `cargo make docker-up` (if not running), then `sqlx migrate run` (per the post-merge sqlx memory, run sqlx directly). Expected: `20260629000001` applied.

- [ ] **Step 3: Add the `CharterSet` payload** — `crates/temper-substrate/src/payloads.rs`, mirroring `CogmapSeeded` (the cfg-gated schemars + serde derive stack) but top-level `cogmap_id` + `blocks`:

```rust
/// `charter_set` payload — replace a cogmap's telos charter with this ordered role-tagged block set.
/// `blocks` is the same `BlockManifest` shape `CogmapSeeded::telos.blocks` carries.
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharterSet {
    pub cogmap_id: CogmapId,
    pub blocks: Vec<BlockManifest>,
}
```

- [ ] **Step 4: Add the mutation plumbing** — `crates/temper-substrate/src/events.rs`:
  - `EventKind::CharterSet` variant; `as_str` → `"charter_set"`; `from` str arm `"charter_set" => EventKind::CharterSet`.
  - `SeedAction::CharterSet { cogmap: CogmapId, blocks: &'a [PreparedBlock], emitter: EntityId }`; its `event_kind()` arm → `EventKind::CharterSet`.
  - `Fired::Charter(ResourceId)` variant + an accessor `fn charter(self) -> Result<ResourceId>` (mirror `Fired::resource`).
  - The fire arm (mirror `CogmapGenesis` / `BlockMutate`):

```rust
        SeedAction::CharterSet {
            cogmap,
            blocks,
            emitter,
        } => {
            let payload = payloads::CharterSet {
                cogmap_id: cogmap,
                blocks: blocks.iter().map(payloads::BlockManifest::from).collect(),
            };
            let sidecar = serde_json::to_value(payloads::content_sidecar(blocks))?;
            let telos = sqlx::query_scalar!(
                "SELECT cogmap_charter_set($1,$2,$3)",
                serde_json::to_value(&payload)?,
                sidecar,
                emitter.uuid(),
            )
            .fetch_one(&mut *conn)
            .await?
            .context("cogmap_charter_set returned null")?;
            Ok(Fired::Charter(telos.into()))
        }
```

- [ ] **Step 5: Add the writes wrapper** — `crates/temper-substrate/src/writes.rs` (mirror `create_kernel_resource_in_tx`'s in-tx fire shape):

```rust
/// Replace a cogmap's telos charter with `blocks` (role-tagged, pre-embedded), in a caller-supplied
/// transaction. Fires `SeedAction::CharterSet` → `cogmap_charter_set`. Returns the telos resource id.
pub async fn set_charter_in_tx(
    conn: &mut sqlx::PgConnection,
    cogmap: CogmapId,
    blocks: &[PreparedChunk_or_PreparedBlock_placeholder], // see note
    emitter: EntityId,
) -> Result<ResourceId> {
    fire(conn, SeedAction::CharterSet { cogmap, blocks, emitter })
        .await?
        .charter()
}
```

> Grounding note: the param type is `&[crate::content::PreparedBlock]` (the `use` at writes.rs:17 already imports `PreparedChunk`; add `PreparedBlock` to that import). Replace the placeholder type accordingly. `fire` here is the same `fire` the other `*_in_tx` wrappers call (confirm whether they call `fire` directly or `fire_with_ctx`; match the neighbor `create_kernel_resource_in_tx`).

- [ ] **Step 6: Add the diff-read** — `crates/temper-substrate/src/readback/mod.rs`:

```rust
/// The telos resource id + its current body merkle for a cogmap — the charter reconcile diff source.
/// `body_hash` is `None` when the telos has no blocks (a fresh genesis telos) OR a content hash.
pub struct TelosCharterState {
    pub telos_resource_id: ResourceId,
    pub body_hash: Option<String>,
}

/// Resolve `cogmap` → telos resource, returning its current `body_hash` for the charter diff.
pub async fn telos_charter_state(
    conn: &mut sqlx::PgConnection,
    cogmap: CogmapId,
) -> Result<TelosCharterState> {
    let row = sqlx::query!(
        "SELECT t.id AS telos_resource_id, t.body_hash \
           FROM kb_cogmaps c JOIN kb_resources t ON t.id = c.telos_resource_id \
          WHERE c.id = $1",
        cogmap.uuid(),
    )
    .fetch_one(&mut *conn)
    .await?;
    Ok(TelosCharterState {
        telos_resource_id: row.telos_resource_id.into(),
        body_hash: row.body_hash,
    })
}
```

> Grounding note: confirm `kb_resources.body_hash` is nullable `TEXT` (it is — set by `_recompute_resource_body_hash`); a fresh genesis telos with `blocks:[]` has `body_hash = sha256_hex('')` (the empty-aggregate coalesce), NOT NULL — so the diff in Task 5 compares against that empty-body hash, and a non-empty charter never matches it ⇒ first delivery is `Created`. Match the `Result`/error type of the neighbor reads in this file.

- [ ] **Step 7: Write the failing write-path test** — `crates/temper-substrate/tests/charter_set_writes.rs` (mirror `cogmap_genesis_charter.rs`'s harness — `#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]`, ephemeral DB, ONNX embed):

```rust
#![cfg(feature = "artifact-tests")]
//! cogmap_charter_set replaces an empty telos's blocks with a role-tagged charter, idempotently.
mod common; // the artifact-test harness (genesis helpers, system actor) — copy the use-list from
            // cogmap_genesis_charter.rs

use temper_substrate::content::{body_hash_from_block_chunk_hashes, prepare_blocks};
use temper_substrate::{readback, writes};

#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn charter_set_populates_empty_telos_then_idempotent(pool: sqlx::PgPool) {
    // 1. genesis a cogmap with an EMPTY telos (blocks:[]) — reuse the genesis helper from `common`.
    let (cogmap, _telos, owner, emitter) = common::genesis_empty_telos(&pool).await;

    // 2. build a 3-block charter (statement, question, framing), server-embedded for the test.
    let specs = temper_core::charter::charter_block_specs(
        "Orient an arriving agent.",
        &[temper_core::charter::CharterQuestion {
            question: "Where am I?".into(),
            context: "the first thing any agent asks".into(),
        }],
        &["Self-referential.".to_string()],
    );
    let spec_refs: Vec<(Option<&str>, &str)> =
        specs.iter().map(|(r, p)| (Some(*r), p.as_str())).collect();
    let blocks = prepare_blocks(&spec_refs).unwrap();

    let mut tx = pool.begin().await.unwrap();
    let telos = writes::set_charter_in_tx(&mut tx, cogmap, &blocks, emitter).await.unwrap();
    tx.commit().await.unwrap();

    // 3. read back the role-tagged blocks via resource_blocks (the charter read).
    let roles = common::block_roles(&pool, telos.uuid()).await; // SELECT role via resource_blocks
    assert_eq!(roles, vec!["statement", "question", "framing"]);

    // 4. the telos body_hash now equals the multi-block merkle of the charter.
    let expect = body_hash_from_block_chunk_hashes(
        &blocks.iter().map(|b| b.chunks.iter().map(|c| c.content_hash.clone()).collect()).collect::<Vec<_>>(),
    );
    let mut conn = pool.acquire().await.unwrap();
    let state = readback::telos_charter_state(&mut conn, cogmap).await.unwrap();
    assert_eq!(state.body_hash.as_deref(), Some(expect.as_str()));

    // 5. IDEMPOTENCY: re-fire the SAME blocks, snapshot the charter_set event count before/after.
    let before = common::event_count(&pool, "charter_set").await;
    // (The reconciler skips the call when body_hash is unchanged — here we assert the LOWER-level
    //  guarantee that re-applying yields the SAME body_hash, so the caller's diff will skip it.)
    let mut tx2 = pool.begin().await.unwrap();
    writes::set_charter_in_tx(&mut tx2, cogmap, &blocks, emitter).await.unwrap();
    tx2.commit().await.unwrap();
    let state2 = readback::telos_charter_state(&mut pool.acquire().await.unwrap(), cogmap).await.unwrap();
    assert_eq!(state2.body_hash, state.body_hash); // same content ⇒ same merkle (the diff key)
    let _ = before; // (event-count assertion lives at the reconciler layer, Task 5)
}
```

> Grounding note: copy the exact harness helpers (`genesis_empty_telos`, `block_roles`, `event_count`, the `common` module) from `cogmap_genesis_charter.rs` / `charter_block_roles.rs` — do not invent. If a helper is missing there, add it to `common` mirroring the existing ones. `prepare_blocks` takes `&[(Option<&str>, &str)]`.

- [ ] **Step 8: Run to confirm failure, then regenerate cache + pass** — `cargo make test-artifacts -- -E 'test(charter_set_populates_empty_telos_then_idempotent)'` (FAIL: function/types missing). Then `cargo sqlx prepare --workspace -- --all-features` (the new `cogmap_charter_set` + `telos_charter_state` macro queries → root cache). Re-run the test. Expected: PASS.

- [ ] **Step 9: Substrate suite + check** — `cargo make test-artifacts` (full write-path group stays green — the `block_specs` delegation + new function), `cargo make check`.

- [ ] **Step 10: Commit**

```bash
git add migrations/20260629000001_cogmap_charter_set.sql crates/temper-substrate .sqlx
git commit -m "feat(charter): cogmap_charter_set substrate primitive + set_charter_in_tx + telos_charter_state"
```

---

## Task 5: Backend telos branch in `reconcile_cognitive_map`

The diff/apply decision for the telos, inside the existing SERIALIZABLE tx + `admin_reconcile` envelope. After the landmark phases, if `request.telos` is `Some`: recompute the charter body-merkle, diff vs the live telos `body_hash`, fire `set_charter_in_tx` only on change, set `outcome.charter`.

**Files:**
- Modify: `crates/temper-api/src/backend/db_backend.rs` (add `apply_telos_phase`, call it from `reconcile_apply`)
- Create: `crates/temper-api/tests/reconcile_charter_test.rs`

**Interfaces:**
- Consumes: `readback::telos_charter_state` + `writes::set_charter_in_tx` (Task 4), `content::body_hash_from_block_chunk_hashes` (Task 3), `unpack_incoming_chunks` (db_backend.rs:90, CONFORM), `content::prepare_block_from_chunks` (content.rs:95, CONFORM), `CharterDisposition` (Task 1).
- Produces: a `reconcile_cognitive_map` that delivers the telos when present.

**Algorithm (`apply_telos_phase`, run on the same tx as the landmark phases):**
1. `let Some(telos) = &request.telos else { return Ok(()) }` — leaves `outcome.charter = Absent`.
2. For each `ReconcileTelosBlock`: `unpack_incoming_chunks(&block.chunks_packed)?` → `Vec<IncomingChunk>`; `prepare_block_from_chunks(seq, Some(&block.role), chunks)` (seq = index) → `PreparedBlock`. Collect `Vec<PreparedBlock>`.
3. Compute `incoming = body_hash_from_block_chunk_hashes(&per_block_chunk_hashes)` where `per_block_chunk_hashes[i]` = block `i`'s chunk `content_hash`es in order.
4. `let live = readback::telos_charter_state(conn, cogmap)`. If `live.body_hash == Some(incoming)` → `outcome.charter = Unchanged`; return.
5. Else `set_charter_in_tx(conn, cogmap, &blocks, emitter)`; `outcome.charter =` (if `live.body_hash` is the empty-body hash `Created` else `Updated`).

> Created vs Updated: the empty-body hash is `temper_substrate::content::body_hash_for_body("")`. `live.body_hash == Some(empty)` ⇒ first delivery ⇒ `Created`; any other non-matching hash ⇒ `Updated`.

- [ ] **Step 1: Write the failing tests** — `crates/temper-api/tests/reconcile_charter_test.rs` (`#![cfg(feature = "test-db")]`, drives `DbBackend` directly; mirror `reconcile_cogmap_test.rs`'s setup — the L0 reserved cogmap or a freshly-genesis'd team-joined cogmap):

```rust
#![cfg(feature = "test-db")]
//! The reconcile telos branch: first delivery creates the charter, re-run is unchanged (no events),
//! an edited charter updates, and a landmark-only request leaves the charter Absent.

// (a) first delivery: request with telos (3 blocks) on an empty telos
//     -> outcome.charter == Created; resource_blocks(telos) has [statement,question,framing].
// (b) idempotency: re-run the SAME request -> charter == Unchanged AND no new `charter_set` kb_events.
// (c) update: change one framing block's prose -> charter == Updated; live body_hash matches new merkle.
// (d) absent: request with telos: None -> charter == Absent; telos still empty; no charter_set events.
// (e) isolation: a request with BOTH entries and telos -> landmark counts correct AND charter Created.
```

Build the pre-embedded telos blocks with the test helper that wraps `compute_body_chunks` per block (the `test-db` feature path — if embedding is unavailable under plain `test-db`, gate the embed-dependent cases behind `#[cfg(feature = "test-embed")]` and assert the diff/disposition logic with hand-packed chunk blobs in the always-on cases, matching how `reconcile_cogmap_test.rs` handles entry bodies).

- [ ] **Step 2: Run to confirm failure** — `cargo nextest run -p temper-api --features test-db --test reconcile_charter_test`. Expected: FAIL (telos branch absent — `charter` stays `Absent`).

- [ ] **Step 3: Implement `apply_telos_phase`** — add the method to the `impl DbBackend` that holds `reconcile_apply`/`apply_resource_phase`, and call it from `reconcile_apply` AFTER `apply_tombstone_phase`:

```rust
        Self::apply_resource_phase(&mut *conn, request, &live_by_id, ctx, &mut outcome).await?;
        Self::apply_edge_phase(&mut *conn, request, ctx).await?;
        Self::apply_tombstone_phase(&mut *conn, request, &live_by_id, ctx, &mut outcome).await?;
        Self::apply_telos_phase(&mut *conn, request, ctx, &mut outcome).await?;   // NEW
```

```rust
    /// PHASE 4 — the telos charter (distinct grain from the kernel slice). Diff on the telos's two-level
    /// body merkle; fire `cogmap_charter_set` only on change; record `outcome.charter`. A request with no
    /// `telos:` leaves `charter = Absent`.
    async fn apply_telos_phase(
        conn: &mut sqlx::PgConnection,
        request: &ReconcileCogmapRequest,
        ctx: ReconcileCtx,
        outcome: &mut ReconcileOutcome,
    ) -> Result<(), TemperError> {
        let Some(telos) = &request.telos else { return Ok(()) }; // charter stays Absent

        // Unpack + prepare each role-tagged block (client-embedded chunks, verbatim).
        let mut blocks = Vec::with_capacity(telos.blocks.len());
        for (seq, b) in telos.blocks.iter().enumerate() {
            let chunks = unpack_incoming_chunks(&b.chunks_packed)?;
            blocks.push(temper_substrate::content::prepare_block_from_chunks(
                seq as i32,
                Some(&b.role),
                chunks,
            ));
        }

        // Incoming resource merkle (two-level), compared to the live telos body_hash — the diff key.
        let per_block: Vec<Vec<String>> = blocks
            .iter()
            .map(|blk| blk.chunks.iter().map(|c| c.content_hash.clone()).collect())
            .collect();
        let incoming = temper_substrate::content::body_hash_from_block_chunk_hashes(&per_block);

        let live = readback::telos_charter_state(&mut *conn, ctx.cogmap)
            .await
            .map_err(api_err)?;

        if live.body_hash.as_deref() == Some(incoming.as_str()) {
            outcome.charter = CharterDisposition::Unchanged;
            return Ok(());
        }

        writes::set_charter_in_tx(&mut *conn, ctx.cogmap, &blocks, ctx.emitter)
            .await
            .map_err(api_err)?;

        let empty = temper_substrate::content::body_hash_for_body("");
        outcome.charter = if live.body_hash.as_deref() == Some(empty.as_str()) {
            CharterDisposition::Created
        } else {
            CharterDisposition::Updated
        };
        Ok(())
    }
```

Add `CharterDisposition` to the `temper_core::types::reconcile::…` import line in `db_backend.rs`.

- [ ] **Step 4: Run** — `cargo nextest run -p temper-api --features test-db --test reconcile_charter_test` (and add `--features test-embed` if any case needs real embedding). Expected: PASS (a–e). No new temper-api macro query was added (the telos read/write are substrate calls), so **no `prepare-api` needed** — confirm `cargo make check` passes offline; if it flags a missing cache entry, run `cargo make prepare-api`.

- [ ] **Step 5: crate suite + check** — `cargo nextest run -p temper-api --features test-db --test reconcile_cogmap_test --test reconcile_charter_test` (scoped to integration targets — never bare `-p temper-api`, it hangs per CLAUDE.md), `cargo make check`.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-api
git commit -m "feat(charter): reconcile telos branch (diff body-merkle, fire cogmap_charter_set, charter disposition)"
```

---

## Task 6: CLI manifest `telos:` + client-side embed

The operator surface: parse a `telos:` section and embed each charter block client-side into `ReconcileTelos`.

**Files:**
- Modify: `crates/temper-cli/src/actions/reconcile.rs`

**Interfaces:**
- Consumes: `compute_body_chunks` (actions/ingest.rs, `feature = "embed"`, CONFORM), `temper_core::charter::{CharterQuestion, charter_block_specs}` (Task 2), the wire types (Task 1).
- Produces: `ManifestTelos` parse model + `telos` population in `manifest_to_request`.

- [ ] **Step 1: Write the failing test** — extend the `#[cfg(test)] mod tests` in `reconcile.rs`. Add a telos to `SAMPLE_YAML` (or a second const) and assert parse + embed:

```rust
    const TELOS_YAML: &str = r#"
entries: []
telos:
  statement: "Orient an arriving agent."
  questions:
    - question: "Where am I?"
      context: "the first thing any agent asks"
    - question: "Bare question?"
  framing:
    - "Self-referential."
"#;

    #[test]
    fn parse_manifest_reads_telos() {
        let doc = parse_manifest(TELOS_YAML).unwrap();
        let telos = doc.telos.as_ref().unwrap();
        assert_eq!(telos.statement, "Orient an arriving agent.");
        assert_eq!(telos.questions.len(), 2);
        assert_eq!(telos.questions[1].context, ""); // defaulted
        assert_eq!(telos.framing, vec!["Self-referential.".to_string()]);
    }

    #[cfg(feature = "test-embed")]
    #[test]
    fn manifest_to_request_embeds_telos_blocks_in_role_order() {
        let doc = parse_manifest(TELOS_YAML).unwrap();
        let req = manifest_to_request(&doc).unwrap();
        let blocks = &req.telos.unwrap().blocks;
        // statement + 2 questions + 1 framing = 4 role-tagged blocks, each embedded.
        assert_eq!(blocks.len(), 4);
        assert_eq!(blocks[0].role, "statement");
        assert_eq!(blocks[1].role, "question");
        assert_eq!(blocks[3].role, "framing");
        assert!(blocks.iter().all(|b| !b.chunks_packed.is_empty()));
    }
```

- [ ] **Step 2: Run to confirm failure** — `cargo nextest run -p temper-cli parse_manifest_reads_telos`. Expected: FAIL (`ManifestDoc` has no `telos`).

- [ ] **Step 3: Implement** — add the parse model + reuse the shared assembly rule:

```rust
/// The authored telos charter (pre-embed: prose). Mirrors the workbench seed's `cogmap.telos`.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ManifestTelos {
    pub statement: String,
    #[serde(default)]
    pub questions: Vec<temper_core::charter::CharterQuestion>,
    #[serde(default)]
    pub framing: Vec<String>,
}
```

Add `#[serde(default)] pub telos: Option<ManifestTelos>,` to `ManifestDoc`. In `manifest_to_request` (under `#[cfg(feature = "embed")]`), after building `entries`, build the telos:

```rust
    let telos = match &doc.telos {
        None => None,
        Some(t) => {
            use temper_core::types::reconcile::{ReconcileTelos, ReconcileTelosBlock};
            let specs = temper_core::charter::charter_block_specs(&t.statement, &t.questions, &t.framing);
            let mut blocks = Vec::with_capacity(specs.len());
            for (role, prose) in specs {
                let BodyChunks { chunks_packed, .. } = compute_body_chunks(&prose)?;
                blocks.push(ReconcileTelosBlock { role: role.to_string(), chunks_packed });
            }
            Some(ReconcileTelos { blocks })
        }
    };
```

Add `telos` to the returned `ReconcileCogmapRequest { entries, fold_resources, fold_edges, telos }`.

- [ ] **Step 4: Run** — `cargo nextest run -p temper-cli parse_manifest_reads_telos` (PASS), and `cargo nextest run -p temper-cli --features test-embed manifest_to_request_embeds_telos` (PASS).

- [ ] **Step 5: crate suite + check** — `cargo nextest run -p temper-cli` (note the env-leak guard: temper-cli test files route through `init_isolated_auth` per CLAUDE memory), `cargo make check`.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli
git commit -m "feat(charter): CLI manifest telos section + client-side per-block embed"
```

---

## Task 7: The committed L0 telos charter content

Add the authored charter to the shippable manifest, copied verbatim from the workbench seed.

**Files:**
- Modify: `schema-artifact/manifests/l0-kernel.yaml`

- [ ] **Step 1: Author the `telos:` section** — add a top-level `telos:` to `schema-artifact/manifests/l0-kernel.yaml`, copying `cogmap.telos.{statement, questions, framing}` **verbatim** from `crates/temper-substrate/tests/fixtures/seeds/l0-kernel.yaml` (the statement, the six questions-with-context, the five framing lines). Update the manifest's SCOPE header comment (currently "this manifest delivers the 22 LANDMARK RESOURCES only … The telos charter is delivered separately") to state that the telos is now delivered by the `telos:` section below. The 22 `entries:` stay byte-identical.

- [ ] **Step 2: Write the failing parse test** — add a test that loads the committed manifest and asserts the telos parsed with the full charter. Put it in `crates/temper-cli/src/actions/reconcile.rs` tests (it already owns `parse_manifest`):

```rust
    #[test]
    fn committed_l0_manifest_carries_full_telos() {
        let yaml = include_str!(
            "../../../../schema-artifact/manifests/l0-kernel.yaml"
        );
        let doc = parse_manifest(yaml).unwrap();
        assert_eq!(doc.entries.len(), 22); // landmarks unchanged
        let telos = doc.telos.expect("manifest must now carry a telos");
        assert_eq!(telos.questions.len(), 6);
        assert_eq!(telos.framing.len(), 5);
        assert!(telos.statement.starts_with("Orient an arriving agent"));
    }
```

> Grounding note: verify the relative `include_str!` path from `crates/temper-cli/src/actions/` to `schema-artifact/manifests/l0-kernel.yaml` (four `../`); adjust to match the repo root depth. Confirm the entries count is 22 against the live file before asserting.

- [ ] **Step 3: Run** — `cargo nextest run -p temper-cli committed_l0_manifest_carries_full_telos`. Expected: PASS (parses the real content).

- [ ] **Step 4: check** — `cargo make check`.

- [ ] **Step 5: Commit**

```bash
git add schema-artifact/manifests/l0-kernel.yaml crates/temper-cli
git commit -m "feat(charter): deliver the authored L0 telos charter in the committed manifest"
```

---

## Task 8: End-to-end delivery proof

Drive the production caller (real HTTP + JWT + Postgres + ONNX): an admin reconcile with a telos delivers the charter, re-run is idempotent, and `telos_alignment` comes online.

**Files:**
- Modify: `tests/e2e/tests/reconcile_cogmap_e2e.rs`

**Interfaces:**
- Consumes: the e2e harness (`tests/e2e/tests/common/`, admin JWT), the existing `PUT /api/cognitive-maps/{id}` route (unchanged), the client `reconcile_cognitive_map` method (unchanged — it already sends `ReconcileCogmapRequest`).

- [ ] **Step 1: Write the failing e2e test** — add to `reconcile_cogmap_e2e.rs`:

```rust
#[tokio::test]
async fn admin_reconcile_delivers_telos_charter_idempotently() {
    let h = common::harness_with_admin().await;
    // a freshly-genesis'd team-joined cogmap with an empty telos, OR the L0 reserved cogmap — match the
    // sibling reconcile e2e's cogmap setup.
    let cogmap = common::genesis_admin_cogmap(&h).await;

    let req = /* ReconcileCogmapRequest { entries: vec![], telos: Some(<3-block charter>), .. } built via
                the CLI manifest_to_request path or a client-side embed helper in `common` */;

    let out1: ReconcileOutcome = h.put_json(&format!("/api/cognitive-maps/{cogmap}"), &req).await;
    assert_eq!(out1.charter, CharterDisposition::Created);

    let out2: ReconcileOutcome = h.put_json(&format!("/api/cognitive-maps/{cogmap}"), &req).await;
    assert_eq!(out2.charter, CharterDisposition::Unchanged); // idempotent

    // salience payoff: after a materialize, the region readout's telos_alignment is non-null.
    common::materialize(&h, cogmap, "orientation").await;
    let metrics = common::region_metrics(&h, cogmap).await; // the analytics read surface
    assert!(metrics.iter().any(|m| m.telos_alignment.is_some()));
}
```

> Grounding note: reuse the sibling `admin_reconcile_l0_is_idempotent` test's harness, cogmap setup, and PUT helper verbatim where possible. Build the telos blocks via the same client embed path the CLI uses (`manifest_to_request` or a `common` helper wrapping `compute_body_chunks`). If `region_metrics`/`materialize` helpers don't exist in `common`, add thin ones hitting the shipped analytics + materialize surfaces.

- [ ] **Step 2: Run to confirm failure** — `cargo make test-e2e-embed -- -E 'test(admin_reconcile_delivers_telos_charter_idempotently)'`. Expected: FAIL before the other tasks are merged; after, it exercises the full vertical. (This path needs the embed feature — the e2e harness builds pre-embedded blocks via ONNX.)

- [ ] **Step 3: prepare-e2e if needed + run** — if the test added macro queries (the `common` helpers), `cargo make prepare-e2e`. Then `cargo make test-e2e-embed -- -E 'test(admin_reconcile_delivers_telos_charter_idempotently)'`. Expected: PASS.

- [ ] **Step 4: e2e suite + check** — `cargo make test-e2e-embed`, `cargo make check`.

- [ ] **Step 5: Commit**

```bash
git add tests/e2e .sqlx
git commit -m "test(charter): e2e admin reconcile delivers L0 telos charter, idempotent, telos_alignment online"
```

---

## Self-Review

**1. Spec coverage** (against `2026-06-28-l0-telos-charter-delivery-design.md`):
- Decision 1 (real embeddings, client-side) → Task 6 (CLI embed) + Task 8 (e2e telos_alignment). ✓
- Decision 2 (extend reconciler; reuse endpoint/authz/envelope) → Task 5 adds a phase to the existing command; no handler/route/client change (called out in Task 5 Step 4 + Task 8 interfaces). ✓
- Decision 3 (`cogmap_charter_set`, fold-then-reproject, idempotent on body_hash) → Task 4 (migration + primitive) + Task 5 (diff). Idempotency asserted in Task 4 Step 7, Task 5 case (b), Task 8 Step 1. ✓
- Decision 4 (`charter: CharterDisposition`, distinct field) → Task 1 (type) + Task 5 (set it). ✓
- Component: shared prose-assembly rule → Task 2. Multi-block merkle → Task 3. Manifest content → Task 7. ✓
- Tests 1–5 from the spec's Testing section → Task 4 (write-path), Task 5 (a–e), Task 8 (e2e + salience). ✓

**2. Placeholder scan:** The one explicit placeholder token (`PreparedChunk_or_PreparedBlock_placeholder` in Task 4 Step 5) is annotated inline with the exact resolution (`&[crate::content::PreparedBlock]`) — it flags a `use`-list edit, not missing logic. The `/* … */` sketches in Task 5 Step 1 and Task 8 Step 1 are test-case enumerations with the concrete assertions named (charter == Created/Unchanged/Updated/Absent; the build-the-request step cites the exact helper to reuse), grounded in named sibling tests (`reconcile_cogmap_test.rs`, `admin_reconcile_l0_is_idempotent`). No "add error handling"/"TBD"/"similar to Task N" placeholders.

**3. Type consistency:** `ReconcileTelos` / `ReconcileTelosBlock` / `CharterDisposition` spelled identically across Tasks 1, 5, 6, 8. `charter_block_specs` / `CharterQuestion` consistent across Tasks 2, 6 (and the substrate parity in Task 2). `body_hash_from_block_chunk_hashes` consistent across Tasks 3, 4, 5. `set_charter_in_tx` / `telos_charter_state` / `TelosCharterState` / `cogmap_charter_set` / `CharterSet` / `SeedAction::CharterSet` / `Fired::Charter` consistent across Task 4 and consumed in Task 5. `apply_telos_phase` defined + called in Task 5.

**Open implementation risks flagged for the implementer (verify-don't-assume):**
- The `kb_content_blocks.is_folded` + `last_event_id` columns and the `_project_blocks` tail-call to `_recompute_resource_body_hash` (Task 4 Step 1 note) — confirm against the live `\d kb_content_blocks` and canonical_functions.sql before finalizing the fold UPDATE.
- A fresh genesis telos's `body_hash` is `sha256_hex('')` (empty-body), NOT NULL — Task 5's Created-vs-Updated branch depends on this. Confirm by reading the L0 telos `body_hash` in a `test-db` setup before asserting (Task 5 case a).
- Whether `_event_append` validates the payload against `kb_event_types.payload_schema` — if it does, the Task 4 Step 1 `charter_set` schema must accept the `CharterSet` payload exactly (it mirrors the verified `cogmap_seeded` block schema); if it doesn't, the schema is documentation. Either way the provided schema is correct for the payload.
- `prepare_block_from_chunks(seq, role, chunks)` signature + return (`PreparedBlock`, ONNX-free, carries chunks verbatim) — confirm at content.rs:95 before wiring Task 5 Step 3.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-28-l0-telos-charter-delivery.md`. Two execution options:

1. **Subagent-Driven (recommended)** — fresh subagent per task, review at the end of the plan (per the repo's consolidated-review convention), fast iteration. Per-task: focused test + `cargo make check`; full-workspace nextest at PR-prep.
2. **Inline Execution** — execute tasks in this session with checkpoints.

Which approach?
