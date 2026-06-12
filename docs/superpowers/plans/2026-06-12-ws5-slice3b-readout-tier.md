# WS5 Slice 3b — Readout-drift tier end-to-end + readout-refresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the WS5 Readout drift tier reachable and proven end-to-end by adding a content-only mutation (`block_mutated` = revise a concept's prose), and close the slice-2 reuse gap so incremental materialization re-runs readouts over fixed membership for content-drifted reused components — proving `incremental ≡ full` on readouts, not just membership.

**Architecture:** A `revise` scenario Step fires a `block_mutated` event carrying new chunks (re-embedded inline, payload-first, exactly like `resource_create`). Block-body content is **not** part of `component_fingerprint` (which is over members + edges + facets + lens), so a revision changes a member's embedding without changing any component's membership inputs → `drift::lens_drift` reports `Readout`. Incremental materialize then detects a content touch and re-runs the SQL readouts (`populate_readouts`) over the reused components' existing regions (fixed membership, no re-cluster, no new region ids), so its readout values match a full recompute.

**Tech Stack:** Rust (temper-next crate), PostgreSQL artifact functions (`schema-artifact/02_functions.sql`), sqlx `query!` macros (temper_next namespace, per-crate `.sqlx` cache via `cargo make prepare-next`), `cargo nextest` with the `artifact-tests` feature, ONNX/bge-768 embeddings.

**Scope decision (settled):** body-revision only. `block_mutated`'s `incorporated` provenance array (→ `reference_standing`) is deferred; `kb_block_provenance` is uniformly empty today, so reference_standing stays 0 and the readout proof rests on the embedding-driven readouts (centroid → content_cohesion, telos_alignment, salience). A follow-up task will wire provenance accretion.

**Pre-built (verified, do NOT re-create):** the `BlockMutated` / `Incorporation` / `ProvenanceSource` payload structs already exist (`crates/temper-next/src/payloads.rs:327-358`); the `block_mutated` event type is already registered in `schema-artifact/seeds/system.yaml:25` and in `payloads::TYPED_EVENT_NAMES`; the payload JSON-Schema exists at `schema-artifact/payloads/block_mutated.v1.schema.json`; `formation_touched_since` (`crates/temper-next/src/replay.rs:278`) already lists `block_mutated` in its touch set.

---

## File Structure

| File | Responsibility | Change |
|------|----------------|--------|
| `schema-artifact/02_functions.sql` | `_project_block_mutated` + `block_mutate` SQL (mirrors `_project_resource_created` + `facet_set` anchor derivation) | Modify (add ~70 lines after `facet_set`, ~line 744) |
| `crates/temper-next/src/events.rs` | `EventKind::BlockMutated`, `SeedAction::BlockMutate`, `fire()` arm | Modify |
| `crates/temper-next/src/replay.rs` | content-bearing replay support: snapshot sidecar for `block_mutated`, `_project_block_mutated` branch; new `content_touched_since` | Modify |
| `crates/temper-next/src/scenario/model.rs` | `Step::Revise`, `Expectation::DriftTier` | Modify |
| `crates/temper-next/src/scenario/runner.rs` | `revise` mutation arm, `drift_tier` expectation eval | Modify |
| `crates/temper-next/src/write.rs` | factor `populate_readouts`; readout-refresh of reused components in `incremental_materialize_cogmap` | Modify |
| `crates/temper-next/tests/readout_tier.rs` | deliverable-2 proof: revise → `Readout`, no component changed | Create |
| `crates/temper-next/tests/common/mod.rs` | `telos_default_readout_signature` (UUID-independent, readout-inclusive) | Modify |
| `crates/temper-next/tests/incremental_equivalence.rs` | deliverable-3 proof: `incremental ≡ full` on readouts after a revision | Modify |
| `schema-artifact/scenarios/storyteller-readout.yaml` | a readout-only growth scenario over the storyteller seed | Create |

---

## Task 1: SQL `block_mutate` + `_project_block_mutated`

**Files:**
- Modify: `schema-artifact/02_functions.sql` (insert after `facet_set`, which ends at line 744)
- Test: `crates/temper-next/tests/readout_tier.rs` (the Rust artifact test in Task 5 is the first caller; this task is verified by a temporary inline check)

Mirror `_project_resource_created` (the chunk insert + body_hash merkle) and `facet_set` (the envelope-anchor derivation from the resource's home, preferring a cogmap home). Key differences from the create path: (1) supersede the block's current chunks (`is_current=false`) before inserting the new revision's chunks at `version = max(version)+1`; (2) recompute the resource `body_hash` over the **current visible chunks** (replay-deterministic — never relies on revision recency).

- [ ] **Step 1: Add the projection + mutation functions**

Insert after line 744 (the end of `facet_set`):

```sql
-- ── block_mutated (content revision) ─────────────────────────────────────────
-- Projection half (BlockMutated, payloads.rs): supersede the block's current chunks, insert the new
-- revision's chunks (re-embedded inline, carried in the sidecar like resource_created), record the
-- revision, bump the block's last_event_id, recompute the resource body_hash merkle. Block-body content
-- is NOT a region-formation input (affinity is declared-only) — this moves a member's embedding, which
-- the downstream SQL readouts (centroid → content_cohesion/telos_alignment) read, without touching any
-- component's membership inputs. Resource body_hash is recomputed from CURRENT visible chunk hashes
-- (ordered by block seq then chunk_index) — a pure function of visible state, so replay reproduces it.
CREATE FUNCTION _project_block_mutated(p_event uuid, p_payload jsonb, p_content jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_occurred timestamptz := (SELECT occurred_at FROM kb_events WHERE id = p_event);
        v_block    uuid := (p_payload->>'block_id')::uuid;
        v_resource uuid;
        v_next_ver int;
        v_chunk_json jsonb; v_chunk uuid; v_side jsonb; v_emb jsonb;
        v_chunk_hashes text := ''; v_chunk_count int := 0; v_block_hash text;
        v_resource_hashes text;
BEGIN
    SELECT resource_id INTO v_resource FROM kb_content_blocks WHERE id = v_block;
    IF v_resource IS NULL THEN
        RAISE EXCEPTION '_project_block_mutated: block % not found', v_block;
    END IF;
    -- supersede the prior revision's chunks (is_current is the chunk-currency flag; the rows stay for CAS)
    UPDATE kb_chunks SET is_current = false WHERE block_id = v_block AND is_current;
    SELECT coalesce(max(version), 0) + 1 INTO v_next_ver FROM kb_chunks WHERE block_id = v_block;
    FOR v_chunk_json IN SELECT jsonb_array_elements(p_payload->'chunks') LOOP
        v_chunk := (v_chunk_json->>'chunk_id')::uuid;
        v_side  := p_content->(v_chunk_json->>'chunk_id');
        IF v_side IS NULL THEN
            RAISE EXCEPTION '_project_block_mutated: content sidecar missing chunk %', v_chunk;
        END IF;
        v_emb := v_side->'embedding';
        INSERT INTO kb_chunks (id, block_id, resource_id, chunk_index, version, content_hash,
                               embedding, is_current, created)
            VALUES (v_chunk, v_block, v_resource, (v_chunk_json->>'chunk_index')::int, v_next_ver,
                    v_chunk_json->>'content_hash',
                    CASE
                        WHEN v_emb IS NULL OR jsonb_typeof(v_emb) = 'null' THEN NULL
                        WHEN jsonb_typeof(v_emb) = 'string' THEN (v_emb #>> '{}')::vector  -- replay: pgvector text
                        ELSE (v_emb::text)::vector                                          -- fire: JSON array
                    END,
                    true, v_occurred);
        INSERT INTO kb_chunk_content (chunk_id, content) VALUES (v_chunk, v_side->>'content');
        v_chunk_hashes := v_chunk_hashes || (v_chunk_json->>'content_hash');
        v_chunk_count := v_chunk_count + 1;
    END LOOP;
    v_block_hash := encode(sha256(convert_to(v_chunk_hashes, 'UTF8')), 'hex');
    INSERT INTO kb_block_revisions (block_id, block_body_hash, chunk_count, created)
        VALUES (v_block, v_block_hash, v_chunk_count, v_occurred);
    UPDATE kb_content_blocks SET last_event_id = p_event WHERE id = v_block;
    -- resource body_hash = sha256 merkle over each non-folded block's (sha256 of its is_current chunk
    -- hashes, in chunk_index order), blocks in seq order — recomputed from current visible state.
    SELECT string_agg(bh, '' ORDER BY seq) INTO v_resource_hashes FROM (
        SELECT b.seq,
               encode(sha256(convert_to(string_agg(ch.content_hash, '' ORDER BY ch.chunk_index), 'UTF8')),
                      'hex') AS bh
        FROM kb_content_blocks b
        JOIN kb_chunks ch ON ch.block_id = b.id AND ch.is_current
        WHERE b.resource_id = v_resource AND NOT b.is_folded
        GROUP BY b.seq
    ) per_block;
    UPDATE kb_resources
        SET body_hash = encode(sha256(convert_to(coalesce(v_resource_hashes, ''), 'UTF8')), 'hex'),
            updated = v_occurred
        WHERE id = v_resource;
    RETURN v_block;
END;
$$;

-- Mutate a block's content (revise its prose). The producing anchor is an ENVELOPE concern derived
-- from the block's resource home (prefer a cogmap home — same discipline as facet_set), never payload
-- data. Emits `block_mutated` + projects, one txn.
CREATE FUNCTION block_mutate(p_payload jsonb, p_content jsonb, p_emitter uuid)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_ev uuid; v_block uuid := (p_payload->>'block_id')::uuid;
        v_resource uuid; v_anchor_tbl text; v_anchor uuid;
BEGIN
    SELECT resource_id INTO v_resource FROM kb_content_blocks WHERE id = v_block;
    IF v_resource IS NULL THEN
        RAISE EXCEPTION 'block_mutate: block % not found', v_block;
    END IF;
    SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor FROM kb_resource_homes
        WHERE resource_id = v_resource ORDER BY (anchor_table = 'kb_cogmaps') DESC LIMIT 1;
    IF v_anchor IS NULL THEN
        RAISE EXCEPTION 'block_mutate: resource % has no home to anchor the event', v_resource;
    END IF;
    v_ev := _event_append('block_mutated', p_emitter, v_anchor_tbl, v_anchor, p_payload);
    RETURN _project_block_mutated(v_ev, p_payload, p_content);
END;
$$;
```

- [ ] **Step 2: Verify the artifact loads cleanly**

Run: `psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f schema-artifact/01_schema.sql -f schema-artifact/02_functions.sql`
Expected: no errors (the two new functions create successfully).

- [ ] **Step 3: Commit**

```bash
git add schema-artifact/02_functions.sql
git commit -m "WS5 slice 3b: block_mutate SQL — content revision write path"
```

---

## Task 2: Rust fire wiring (`EventKind::BlockMutated`, `SeedAction::BlockMutate`)

**Files:**
- Modify: `crates/temper-next/src/events.rs`

The `BlockMutated` payload struct already exists in `payloads.rs`. Wire the event taxonomy + action + fire dispatch, mirroring `ResourceCreate` (which builds a sidecar from prepared blocks).

- [ ] **Step 1: Add the `EventKind` variant**

In `events.rs`, add to `enum EventKind` (after `RelationshipFolded`):

```rust
    BlockMutated,
```

And to `as_canonical_name`:

```rust
            EventKind::BlockMutated => "block_mutated",
```

- [ ] **Step 2: Add the `SeedAction::BlockMutate` variant**

In `enum SeedAction<'a>` (after `RelationshipFold`):

```rust
    BlockMutate {
        block: BlockId,
        /// The revised body as a single prepared block's worth of chunks (re-embedded inline).
        chunks: &'a [PreparedChunk],
        emitter: EntityId,
    },
```

Add the imports at the top: `use crate::content::PreparedChunk;` (alongside the existing `PreparedBlock`) and `BlockId` to the `ids` import line.

- [ ] **Step 3: Map it in `event_type`**

```rust
            SeedAction::BlockMutate { .. } => EventKind::BlockMutated,
```

- [ ] **Step 4: Add the `fire()` arm**

In `fire()` (after the `RelationshipFold` arm), build the `BlockMutated` payload + a sidecar over the new chunks. `content_sidecar` takes `&[PreparedBlock]`, so build the sidecar from the chunks directly:

```rust
        SeedAction::BlockMutate {
            block,
            chunks,
            emitter,
        } => {
            let payload = payloads::BlockMutated {
                block_id: block,
                chunks: chunks.iter().map(payloads::ChunkManifest::from).collect(),
                incorporated: Vec::new(), // body-revision only; provenance accretion deferred
            };
            let mut sidecar = std::collections::HashMap::new();
            for c in chunks {
                sidecar.insert(
                    c.chunk_id.to_string(),
                    payloads::ChunkContent {
                        content: c.content.clone(),
                        embedding: Some(payloads::EmbeddingRepr::Vector(c.embedding.clone())),
                    },
                );
            }
            let id = sqlx::query_scalar!(
                "SELECT block_mutate($1,$2,$3)",
                serde_json::to_value(&payload)?,
                serde_json::to_value(&sidecar)?,
                emitter.uuid(),
            )
            .fetch_one(&mut *conn)
            .await?
            .context("block_mutate returned null")?;
            Ok(Fired::Block(BlockId::from(id)))
        }
```

Add a `ChunkManifest::from(&PreparedChunk)` impl in `payloads.rs` (next to the existing `BlockManifest::from`):

```rust
impl From<&crate::content::PreparedChunk> for ChunkManifest {
    fn from(c: &crate::content::PreparedChunk) -> Self {
        ChunkManifest {
            chunk_id: c.chunk_id,
            chunk_index: c.chunk_index,
            content_hash: c.content_hash.clone(),
        }
    }
}
```

Add a `Fired::Block(BlockId)` variant + a `block()` accessor in `events.rs`:

```rust
    Block(BlockId),
```
```rust
    /// Extract the block id a `BlockMutate` fire produced.
    pub fn block(self) -> Result<BlockId> {
        match self {
            Fired::Block(id) => Ok(id),
            other => anyhow::bail!("expected Fired::Block, got {other:?}"),
        }
    }
```

- [ ] **Step 5: Verify it compiles (offline sqlx will fail until prepared — that's expected)**

Run: `DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development cargo check -p temper-next --features artifact-tests`
Expected: compiles (live DB validates the new `block_mutate` query). If you see an `event_type_maps_each_action` test gap, extend that unit test with the `BlockMutate` case asserting `"block_mutated"`.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-next/src/events.rs crates/temper-next/src/payloads.rs
git commit -m "WS5 slice 3b: SeedAction::BlockMutate fire wiring"
```

---

## Task 3: Replay support for `block_mutated` (content-bearing)

**Files:**
- Modify: `crates/temper-next/src/replay.rs`

`block_mutated` carries new chunk embeddings, so `verify_ledger_roundtrip` (called by the incremental-equivalence harness) needs: (a) its sidecar reconstructed from the CAS in `snapshot()`, and (b) a `_project_block_mutated` branch in `replay()`. Without these, replay bails with "no projector for event type block_mutated".

- [ ] **Step 1: Add `block_mutated` to the content-bearing snapshot query**

In `snapshot()`, change the event-type filter (currently `IN ('cogmap_seeded','resource_created')`) to include `'block_mutated'`, and extract its chunk manifest. The manifests live at different payload paths per type:

```rust
        let manifests = match name.as_str() {
            "cogmap_seeded" => payload.pointer("/telos/blocks").cloned(),
            "resource_created" => payload.get("blocks").cloned(),
            "block_mutated" => {
                // block_mutated carries a flat `chunks` array (one block); wrap it as a single
                // pseudo-block so the chunk-extraction loop below is uniform.
                payload
                    .get("chunks")
                    .cloned()
                    .map(|chunks| serde_json::json!([{ "chunks": chunks }]))
            }
            _ => None,
        }
        .context("content-bearing payload missing blocks")?;
```

Update the WHERE clause:

```rust
          WHERE et.name IN ('cogmap_seeded','resource_created','block_mutated') ORDER BY e.id",
```

- [ ] **Step 2: Add the `replay()` projection branch**

In `replay()`'s match (after the `resource_created` arm):

```rust
            "block_mutated" => {
                let side = snap.sidecars.get(&id).context("missing sidecar")?;
                sqlx::query("SELECT _project_block_mutated($1,$2,$3)")
                    .bind(id)
                    .bind(&payload)
                    .bind(side)
                    .execute(pool)
                    .await?;
            }
```

- [ ] **Step 3: Add `content_touched_since` (gates the Task 5 readout-refresh)**

Add a sibling to `formation_touched_since` that filters to **content** events only (the readout-only input set — body revisions; provenance correction reserved for the follow-up). This lets incremental refresh readouts on reused components ONLY when content actually moved, not on every structural touch:

```rust
/// True iff a CONTENT event (a block-body revision — the readout-only formation input) touched the
/// cogmap after `watermark`. Distinct from `formation_touched_since` (which includes edge/facet
/// structural events): incremental materialization re-runs readouts over reused components only when
/// THIS is true, so a purely-structural change does no redundant readout work on the reused side.
pub async fn content_touched_since(pool: &PgPool, cogmap: Uuid, watermark: Uuid) -> Result<bool> {
    Ok(sqlx::query_scalar(
        "SELECT EXISTS ( \
            SELECT 1 FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
             WHERE e.id > $2 \
               AND e.producing_anchor_table = 'kb_cogmaps' AND e.producing_anchor_id = $1 \
               AND et.name IN ('block_mutated'))",
    )
    .bind(cogmap)
    .bind(watermark)
    .fetch_one(pool)
    .await?)
}
```

- [ ] **Step 4: Verify compile**

Run: `DATABASE_URL=… cargo check -p temper-next --features artifact-tests`
Expected: compiles.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-next/src/replay.rs
git commit -m "WS5 slice 3b: replay + content-touch support for block_mutated"
```

---

## Task 4: `revise` scenario Step + runner arm

**Files:**
- Modify: `crates/temper-next/src/scenario/model.rs`, `crates/temper-next/src/scenario/runner.rs`

A `revise` Step targets a keyed resource's body block and supplies new prose. Concept resources created via `create_resource` have exactly one non-folded block (the roleless body at seq 0), so the runner resolves the target block as "the resource's single non-folded block" (erroring if ambiguous, which only a multi-block resource like a charter would be — out of scope here).

- [ ] **Step 1: Add the `Step::Revise` variant**

In `model.rs`, add to `enum Step` (before `Materialize`):

```rust
    /// Revise a concept resource's body prose (a content-only mutation: new chunk embeddings, no edge
    /// or facet change). Fires `block_mutated` on the resource's single body block. `resource` is a key.
    Revise {
        resource: String,
        body: String,
    },
```

- [ ] **Step 2: Add the `Expectation::DriftTier` variant**

In `enum Expectation`:

```rust
    DriftTier {
        lens: String,
        tier: DriftTierName,
    },
```

Add the serde-friendly tier name enum at the bottom of `model.rs`:

```rust
/// The drift tier names as they appear in scenario YAML (`check: drift_tier, tier: readout`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub enum DriftTierName {
    Fresh,
    Readout,
    Structural,
}
```

- [ ] **Step 3: Add the runner mutation arm**

In `runner.rs` `apply_mutation`, add (before the `Step::Materialize { .. } | Step::Assert { .. }` unreachable arm):

```rust
        Step::Revise { resource, body } => {
            let rid = lookup(&loaded.keys, resource)?;
            // resolve the resource's single non-folded body block (concept resources have exactly one)
            let block_ids: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM kb_content_blocks WHERE resource_id=$1 AND NOT is_folded ORDER BY seq",
            )
            .bind(rid)
            .fetch_all(&mut *tx)
            .await?;
            let block_id = match block_ids.as_slice() {
                [one] => *one,
                [] => bail!("revise: resource {resource} has no live block"),
                _ => bail!("revise: resource {resource} has >1 block (multi-block revise unsupported)"),
            };
            // re-chunk + re-embed the new body inline (payload-first, like create_resource)
            let prepared = crate::content::prepare_block(0, None, body)?;
            fire(
                &mut tx,
                SeedAction::BlockMutate {
                    block: crate::ids::BlockId::from(block_id),
                    chunks: &prepared.chunks,
                    emitter: EntityId::from(loaded.emitter),
                },
            )
            .await?;
        }
```

- [ ] **Step 4: Eval the `drift_tier` expectation**

In `runner.rs` `eval_expectation`, add an arm:

```rust
        Expectation::DriftTier { lens, tier } => {
            let (got, _diff) = crate::drift::lens_drift(pool, loaded.cogmap, lens).await?;
            let want = match tier {
                DriftTierName::Fresh => crate::drift::DriftTier::Fresh,
                DriftTierName::Readout => crate::drift::DriftTier::Readout,
                DriftTierName::Structural => crate::drift::DriftTier::Structural,
            };
            if got != want {
                bail!("drift_tier: expected {want:?}, got {got:?} (lens {lens})");
            }
        }
```

And add `Expectation::DriftTier { lens, .. } => vec![lens.as_str()]` to `expectation_lenses` so lens validation covers it.

- [ ] **Step 5: Verify compile**

Run: `DATABASE_URL=… cargo check -p temper-next --features artifact-tests`
Expected: compiles.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-next/src/scenario/model.rs crates/temper-next/src/scenario/runner.rs
git commit -m "WS5 slice 3b: revise Step + drift_tier expectation"
```

---

## Task 5: Deliverable-2 proof — Readout tier reachable end-to-end

**Files:**
- Create: `crates/temper-next/tests/readout_tier.rs`

Mirror `drift_signal.rs`: seed → embed → materialize → `lens_drift` is `Fresh` → `revise` a member's body → `lens_drift` is `Readout`, with **no** component changed (`diff.has_structural_change()` is false; every prior component stays in `unchanged`). This is the assertion the existing `drift_signal.rs` header explicitly defers ("the Readout tier is unit-proven in `drift`" — never reached end-to-end until now).

- [ ] **Step 1: Write the failing test**

```rust
#![cfg(feature = "artifact-tests")]
//! WS5 slice 3b: the Readout drift tier, reached end-to-end. A content-only revision (block_mutated)
//! moves a member's chunk embedding — a readout input — WITHOUT changing any component's membership
//! inputs (affinity is declared-only; content is not in the component fingerprint). So `lens_drift`
//! reports `Readout`: something touched the map, but no component must re-cluster.
mod common;

use temper_next::drift::{self, DriftTier};
use temper_next::ids::BlockId;
use temper_next::events::{fire, SeedAction};
use temper_next::ids::EntityId;
use temper_next::scenario::{bootseed, loader, model::Seed};
use temper_next::{content, embed, substrate, write};
use uuid::Uuid;

const SEED: &str = r#"
name: readout-tier-test
cogmap:
  telos: { title: "Min", statement: "A tiny telos about onboarding.", questions: [{ question: "why?" }] }
  owner: alice
  emitter: "agent#1"
world:
  profiles: [{ handle: alice, display_name: Alice, system_access: approved }]
  entities: [{ name: "agent#1", profile: alice }]
resources:
  - { key: a, origin_uri: "temper://readout/a", home: cogmap, body: "alpha concept about deployment confidence" }
  - { key: b, origin_uri: "temper://readout/b", home: cogmap, body: "beta concept about deployment confidence" }
edges:
  - { from: a, to: b, kind: leads_to, label: then, weight: 1.0 }
  - { from: telos, to: a, kind: express, label: operationalized_by, weight: 1.0 }
uses_lenses: [telos-default]
"#;

#[tokio::test]
async fn revise_reaches_readout_tier_no_component_changes() {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();
    let seed: Seed = serde_yaml::from_str(SEED).unwrap();
    let loaded = loader::load_seed(&pool, &seed).await.unwrap();

    embed::embed_chunks(&pool).await.unwrap();
    write::materialize_cogmap(&pool, loaded.cogmap, "telos-default", loaded.emitter)
        .await
        .unwrap();

    let (tier, diff) = drift::lens_drift(&pool, loaded.cogmap, "telos-default").await.unwrap();
    assert_eq!(tier, DriftTier::Fresh, "fresh right after materialize");
    let prior = diff.unchanged.len();
    assert!(prior >= 1);

    // revise concept `a`'s body — a content-only change to a member's prose
    let block_id: Uuid = sqlx::query_scalar(
        "SELECT b.id FROM kb_content_blocks b JOIN kb_resources r ON r.id=b.resource_id \
         WHERE r.origin_uri='temper://readout/a' AND NOT b.is_folded",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let prepared = content::prepare_block(0, None, "alpha concept — now entirely about quantum chromodynamics and lattice gauge theory").unwrap();
    let mut tx = pool.begin().await.unwrap();
    fire(
        &mut tx,
        SeedAction::BlockMutate {
            block: BlockId::from(block_id),
            chunks: &prepared.chunks,
            emitter: EntityId::from(loaded.emitter),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // Readout — touched, but no component's membership inputs changed.
    let (tier2, diff2) = drift::lens_drift(&pool, loaded.cogmap, "telos-default").await.unwrap();
    assert_eq!(tier2, DriftTier::Readout, "a body revision is a readout-tier change");
    assert!(!diff2.has_structural_change(), "no component must re-cluster");
    assert_eq!(diff2.unchanged.len(), prior, "every component stays provably current");
    assert!(diff2.changed.is_empty() && diff2.stale.is_empty());
}
```

- [ ] **Step 2: Run it to verify it fails (before Tasks 1-4 land it would; confirm it now PASSES given 1-4 are in)**

Run: `DATABASE_URL=… cargo nextest run -p temper-next --features artifact-tests -E 'test(revise_reaches_readout_tier)'`
Expected: PASS. If `block_mutate` SQL isn't prepared, the macro call inside `fire` validates live against the dev DB — ensure `01_schema`+`02_functions` are loaded (`common::reset_artifact` does this). Tier must be `Readout`, not `Structural` or `Fresh`.

- [ ] **Step 3: Commit**

```bash
git add crates/temper-next/tests/readout_tier.rs
git commit -m "WS5 slice 3b: prove the Readout tier reachable end-to-end"
```

---

## Task 6: Deliverable-3 mechanism — `populate_readouts` factor + readout-refresh

**Files:**
- Modify: `crates/temper-next/src/write.rs`

Factor the readout tail of `assert_region` (the centroid UPDATE + readouts UPDATE + salience UPDATE, lines ~396-436) into a reusable `populate_readouts`. Then, in `incremental_materialize_cogmap`, after handling changed/stale components, re-run readouts over the reused (unchanged) components' live regions when a content event touched since the last materialize — fixed membership, no re-cluster, no new region ids, bump `last_event_id` to the act.

- [ ] **Step 1: Extract `populate_readouts`**

Replace the three readout UPDATE statements at the end of `assert_region` (everything after the member INSERT loop, i.e. the centroid UPDATE + readouts UPDATE + salience UPDATE) with a single call:

```rust
    populate_readouts(tx, region, lens, zero_centroid).await
}

/// Re-derive a region's SQL readouts over its CURRENT members + embeddings: centroid (mean of
/// per-member pooled chunk vectors), then content_cohesion / telos_alignment / reference_standing /
/// centrality / internal_tension, then lens-weighted salience. Idempotent over fixed membership — the
/// readout-refresh tier (drift §1) calls this on reused components whose content moved; assert_region
/// calls it on a freshly-asserted region. Membership must already be inserted.
async fn populate_readouts(
    tx: &mut PgConnection,
    region: Uuid,
    lens: &Lens,
    zero_centroid: &str,
) -> Result<()> {
    // (move the three existing UPDATE statements here verbatim, binding `region` / `zero_centroid` /
    //  lens.s_telos / lens.s_ref / lens.s_central exactly as they were in assert_region)
    Ok(())
}
```

(Move the existing centroid UPDATE, the readouts UPDATE, and the salience UPDATE bodies into `populate_readouts` unchanged — they already reference only `region`, `zero_centroid`, and the three lens salience weights.)

- [ ] **Step 2: Add the readout-refresh of reused components in `incremental_materialize_cogmap`**

After the `for (comp, comp_region_ids) in changed.iter().zip(&new_region_ids)` loop (which asserts the changed components' new regions) and before `tx.commit()`, add:

```rust
    // Readout-refresh (drift §1, slice 3b): reused components keep their membership AND their region
    // ids, but a content revision since the last materialize moves a member's embedding — so their
    // stored readouts are stale. Re-run the readouts over the reused regions' fixed membership (no
    // re-cluster, no new region ids) so incremental matches a full recompute. Gated on a CONTENT touch
    // so a purely-structural pass does no redundant readout work here.
    let content_touched = match priors.first() {
        // priors is non-empty here (we returned early to a full pass when it was empty)
        Some(_) => {
            let watermark = last_materialize_watermark(&mut tx, cogmap, s.lens_id).await?;
            match watermark {
                Some(w) => crate::replay::content_touched_since(pool, cogmap, w).await?,
                None => false,
            }
        }
        None => false,
    };
    if content_touched {
        for prior_id in &diff.unchanged {
            let region_ids: Vec<Uuid> = sqlx::query_scalar(
                "SELECT id FROM kb_cogmap_regions WHERE component_id=$1 AND NOT is_folded",
            )
            .bind(prior_id)
            .fetch_all(&mut *tx)
            .await?;
            for rid in region_ids {
                populate_readouts(&mut tx, rid, &s.lens, &zero).await?;
                sqlx::query("UPDATE kb_cogmap_regions SET last_event_id=$1 WHERE id=$2")
                    .bind(ev)
                    .bind(rid)
                    .execute(&mut *tx)
                    .await?;
            }
        }
    }
```

Note: `diff` is currently consumed (`let stale_prior: Vec<Uuid> = diff.stale;`). Change that line to clone what's needed so `diff.unchanged` survives:

```rust
    let stale_prior: Vec<Uuid> = diff.stale.clone();
```

- [ ] **Step 3: Add the `last_materialize_watermark` helper**

This is the watermark recorded by the PRIOR materialize for this lens (so "content touched since" means since that prior projection). Add near `current_watermark`:

```rust
/// The event id of the most recent region_materialized act for (cogmap, lens) BEFORE the current
/// transaction's event — the point-in-time the reused regions' readouts were last computed against.
async fn last_materialize_watermark(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    cogmap: Uuid,
    lens_id: Uuid,
) -> Result<Option<Uuid>> {
    // the act THIS pass just appended is the latest; the one before it is the prior projection.
    Ok(sqlx::query_scalar(
        "SELECT e.id FROM kb_events e JOIN kb_event_types et ON et.id = e.event_type_id \
         WHERE et.name='region_materialized' \
           AND (e.payload->>'cogmap_id')::uuid=$1 AND (e.payload->>'lens_id')::uuid=$2 \
         ORDER BY e.id DESC OFFSET 1 LIMIT 1",
    )
    .bind(cogmap)
    .bind(lens_id)
    .fetch_optional(&mut **tx)
    .await?
    .flatten())
}
```

(`query_scalar` over a nullable column returns `Option<Option<Uuid>>` only if the column is nullable; `e.id` is NOT NULL so this returns `Option<Uuid>` — drop the `.flatten()` if the type checker objects.)

- [ ] **Step 4: Verify compile**

Run: `DATABASE_URL=… cargo check -p temper-next --features artifact-tests`
Expected: compiles.

- [ ] **Step 5: Commit**

```bash
git add crates/temper-next/src/write.rs
git commit -m "WS5 slice 3b: readout-refresh of reused components in incremental materialize"
```

---

## Task 7: Deliverable-3 proof — `incremental ≡ full` on readouts

**Files:**
- Modify: `crates/temper-next/tests/common/mod.rs`, `crates/temper-next/tests/incremental_equivalence.rs`

The existing byte-identical proof compares membership only (`telos_default_partition`). Add a readout-inclusive, UUID-independent signature and a test that revises a member, then asserts the incremental readout signature equals the full one. Full recomputes all readouts on new region ids; incremental refreshes the reused components' readouts in place — both read the same post-revision embeddings, so the values must match.

- [ ] **Step 1: Add `telos_default_readout_signature` to `common/mod.rs`**

```rust
/// Like [`telos_default_partition`] but also folds each region's READOUT values into the signature
/// (content_cohesion, salience, member_count), so it distinguishes a stale-readout reuse from a fresh
/// recompute. UUID-independent (keyed by sorted member origin_uris); floats rounded to 6 places to
/// absorb text-formatting noise (identical SQL over identical embeddings is already bit-stable).
const READOUT_SIG_SQL: &str = r#"
SELECT md5(string_agg(sig, '|' ORDER BY sig)) FROM (
  SELECT string_agg(res.origin_uri, ',' ORDER BY res.origin_uri)
         || ':' || coalesce(round(r.content_cohesion::numeric, 6)::text, 'null')
         || ',' || coalesce(round(r.salience::numeric, 6)::text, 'null')
         || ',' || r.member_count::text AS sig
  FROM kb_cogmap_region_members m
  JOIN kb_cogmap_regions r ON r.id = m.region_id AND NOT r.is_folded
  JOIN kb_cogmap_lenses  l ON l.id = r.lens_id AND l.name = 'telos-default'
  JOIN kb_resources    res ON res.id = m.member_id
  WHERE r.cogmap_id = $1
  GROUP BY r.id, r.content_cohesion, r.salience, r.member_count
) g
"#;

pub async fn telos_default_readout_signature(pool: &sqlx::PgPool, cogmap: uuid::Uuid) -> String {
    sqlx::query_scalar(READOUT_SIG_SQL)
        .bind(cogmap)
        .fetch_one(pool)
        .await
        .unwrap()
}
```

- [ ] **Step 2: Write the failing test**

In `incremental_equivalence.rs`, add a helper that runs the storyteller-readout scenario (Task 8 creates it) in a mode and returns the readout signature, plus the equivalence test:

```rust
async fn run_readout_scenario(file: &str, mode: MaterializeMode) -> String {
    common::reset_artifact();
    let pool = substrate::connect().await.unwrap();
    bootseed::seed_system(&pool).await.unwrap();
    let path = format!("{}/../../schema-artifact/scenarios/{file}", env!("CARGO_MANIFEST_DIR"));
    let scenario: Scenario = serde_yaml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let base = Path::new(&path).parent().unwrap();
    runner::run_scenario_with(&pool, &scenario, base, mode)
        .await
        .unwrap_or_else(|e| panic!("{file} ({mode:?}) failed: {e:#}"));
    temper_next::payloads::verify_ledger_roundtrip(&pool).await.expect("ledger roundtrip");
    let cogmaps: Vec<uuid::Uuid> = sqlx::query_scalar("SELECT id FROM kb_cogmaps").fetch_all(&pool).await.unwrap();
    assert_eq!(cogmaps.len(), 1);
    common::telos_default_readout_signature(&pool, cogmaps[0]).await
}

#[tokio::test]
async fn readout_refresh_incremental_equals_full() {
    let full = run_readout_scenario("storyteller-readout.yaml", MaterializeMode::Full).await;
    let incremental = run_readout_scenario("storyteller-readout.yaml", MaterializeMode::Incremental).await;
    assert_eq!(
        full, incremental,
        "after a body revision, incremental readouts must match a full recompute (not reuse stale readouts)"
    );
}
```

- [ ] **Step 3: Run it to verify it FAILS without the refresh (regression guard)**

Temporarily comment out the `if content_touched { … }` block from Task 6 Step 2, run the test, confirm it FAILS (incremental reuses stale readouts → signatures differ), then restore the block.

Run: `DATABASE_URL=… cargo nextest run -p temper-next --features artifact-tests -E 'test(readout_refresh_incremental_equals_full)'`
Expected: FAIL when the refresh is disabled (proves the test bites), PASS with it enabled.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-next/tests/common/mod.rs crates/temper-next/tests/incremental_equivalence.rs
git commit -m "WS5 slice 3b: prove incremental ≡ full on readouts after a revision"
```

---

## Task 8: Readout-only scenario over the storyteller seed

**Files:**
- Create: `schema-artifact/scenarios/storyteller-readout.yaml`

A scenario that materializes, revises a member's body (no edge/facet change), re-materializes, and asserts `drift_tier: readout`. This is the YAML-level proof of deliverable 2 and the fixture Task 7 runs in both modes. Reference the existing storyteller seed; pick a member key that exists in `schema-artifact/seeds/storyteller.yaml` (read it to confirm a concept key, e.g. one of its `resources[].key`).

- [ ] **Step 1: Read the storyteller seed to get real keys**

Run: `grep -n "key:" schema-artifact/seeds/storyteller.yaml`
Note the lens it uses (`uses_lenses`) and one concept resource key to revise.

- [ ] **Step 2: Write the scenario**

```yaml
name: storyteller-readout
seed: ../seeds/storyteller.yaml
steps:
  - { do: materialize, lens: telos-default }
  - do: assert
    checks:
      - { check: drift_tier, lens: telos-default, tier: fresh }
  # revise one concept's prose — a content-only change (no edge, no facet)
  - { do: revise, resource: <REAL_KEY>, body: "A substantially rewritten body that shifts this concept's embedding into a different semantic neighborhood while leaving every edge and facet untouched." }
  - do: assert
    checks:
      - { check: drift_tier, lens: telos-default, tier: readout }
  - { do: materialize, lens: telos-default }
  - do: assert
    checks:
      - { check: drift_tier, lens: telos-default, tier: fresh }
```

Replace `<REAL_KEY>` with a concept key from Step 1. Confirm `telos-default` is the seed's lens (else use the seed's actual lens name).

- [ ] **Step 3: Run the scenario in full mode directly**

Run: `DATABASE_URL=… cargo nextest run -p temper-next --features artifact-tests -E 'test(readout_refresh_incremental_equals_full)'`
Expected: PASS (this also exercises the scenario end-to-end in both modes).

- [ ] **Step 4: Commit**

```bash
git add schema-artifact/scenarios/storyteller-readout.yaml
git commit -m "WS5 slice 3b: storyteller readout-only scenario (drift_tier proof)"
```

---

## Task 9: Schema snapshot, sqlx cache, full verification

**Files:**
- Modify: `crates/temper-next/.sqlx` (regenerated), possibly `crates/temper-next/tests/scenario_schema.rs` snapshot

- [ ] **Step 1: Regenerate the scenario JSON-Schema snapshot (new `Step::Revise` + `Expectation::DriftTier`)**

The `scenario-schema` test snapshots the YAML model schema. Regenerate/refresh it:

Run: `cargo nextest run -p temper-next --features scenario-schema -E 'test(scenario_schema)'`
If it fails on a snapshot mismatch, update the committed schema (`schema-artifact/scenarios/scenario.schema.json`) per the test's diff/instructions, then re-run to PASS.

- [ ] **Step 2: Regenerate the per-crate sqlx cache**

Run: `cargo make prepare-next`
Expected: `crates/temper-next/.sqlx` updated with the new `block_mutate` macro query (and any other new `query!` calls). Stage the `.sqlx` changes.

- [ ] **Step 3: Run the full temper-next-write artifact suite**

Run: `cargo nextest run -p temper-next --features artifact-tests`
Expected: all green — `readout_tier`, `drift_signal`, `incremental_equivalence` (incl. the new readout test), `corpus_smoke`, `corpus_growth`, `scenario_steps`, `replay_roundtrip`, `seed_corpus_sweep`, etc. Verify by exit code / absence of `FAIL [` — do NOT trust the per-binary summary line (`--no-fail-fast` makes it lie).

- [ ] **Step 4: Quality gate**

Run: `cargo make check`
Expected: fmt + clippy (`-D warnings`) + machete clean across the workspace. Fix any lint on touched files. (`cargo make` forces `SQLX_OFFLINE=true` — this is the honest probe that the committed `.sqlx` cache covers the new queries.)

- [ ] **Step 5: Commit**

```bash
git add crates/temper-next/.sqlx schema-artifact/scenarios/scenario.schema.json crates/temper-next/tests/scenario_schema.rs
git commit -m "WS5 slice 3b: regenerate sqlx cache + scenario schema snapshot"
```

---

## Self-Review Notes

- **Spec coverage:** Deliverable 1 (content-mutation Step) = Tasks 1-4. Deliverable 2 (Readout tier end-to-end) = Tasks 5, 8. Deliverable 3 (readout-refresh + incremental≡full) = Tasks 6, 7. Replay parity (a hard requirement for the equivalence harness's `verify_ledger_roundtrip`) = Task 3.
- **Determinism:** body_hash recompute reads only current visible chunks (no revision-recency dependence) → replay-stable. New chunk `version = max+1` is identical on fire and replay. Embeddings ride the sidecar (fire = f32 array, replay = pgvector text) — both cast by the existing `CASE` in the projector.
- **Why Readout, not Structural:** `component_fingerprint(members, edges, facets, lens)` excludes content; `current_component_fingerprints` therefore unchanged after a revision → `diff.has_structural_change()` false → `tier(false, touched=true)` = `Readout`.
- **Mechanism guard:** readout-refresh bumps reused regions' `last_event_id` (honest — they changed) but NOT `asserted_by_event_id`, so the existing `incremental_actually_reuses_the_untouched_component` guard (which counts distinct `asserted_by_event_id`) still holds.
- **Deferred (named):** `incorporated` provenance accretion → `reference_standing` (a follow-up; `kb_block_provenance` is empty today); multi-block (charter) revise; `block_created` / `block_folded` Steps.
