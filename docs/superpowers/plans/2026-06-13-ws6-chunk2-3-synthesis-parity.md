# WS6 Chunks 2+3 — Additive Install + Synthesis-from-State + Parity-Read Harness — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **Grounding discipline (binding):** This plan was written under `~/.claude/skills/temper/guidance/implementation-grounding.md`. Every code-touching task is tagged **CONFORM** / **EXTEND** / **AMEND** and cites the disk thing or the spec section. The "Grounding Evidence" section below carries the quoted `file:line` facts the tasks build on — treat **those** as the only pre-grounded facts. Anything a task does not cite, verify on disk before use (GD-1/GD-2). If a step cannot be grounded and is not a spec-authorized EXTEND/AMEND, STOP and report BLOCKED (GD-5).

**Goal:** Install the temper-next destination schema into the live database *additively* (zero production behavior change), build the explicitly-invoked **synthesis-from-state** operation that regenerates that schema from current production state via genesis-event synthesis, gate it with a per-resource body-text parity check, and prove it correct with a **parity-read harness** covering the full §9 migration-time read floor.

**Architecture:** The destination shape already exists as the evolving artifact `schema-artifact/01_schema.sql`+`02_functions.sql` in the `temper_next` Postgres namespace, exercised by test-reset. Chunk 2 (a) factors that artifact into a *shared, namespace-agnostic DDL body* consumed by both the test-reset path and a generated **run-once additive migration** (single source of truth — Pete's call), (b) extends the artifact chunk model to carry production's per-chunk heading metadata verbatim (§8), and (c) adds a `temper-next synthesize` bin subcommand that reads `public.*` and fires genesis events into `temper_next.*` through the existing `events::fire` surface, ending in a per-resource hash-parity gate. Chunk 3 adds read implementations over `temper_next.*` and a harness that diffs them against the production `public` reads (list / show / get_meta / body / FTS / vector / graph). Nothing in chunks 2+3 reads `temper_next` from a production surface, archives the old ledger, or runs synthesis at migrate time — those belong to chunk 4 and the flip.

**Tech Stack:** Rust (temper-next, temper-core, temper-ingest), PostgreSQL 18 + pgvector in the `temper_next` namespace, sqlx (`query_scalar!`/`query!` macros + the per-crate `crates/temper-next/.sqlx` offline cache), cargo-nextest with the serialized `temper-next-write` test group, psql-driven artifact reset (`tests/common/mod.rs`).

---

## Grounding Evidence (quoted; the pre-grounded facts these tasks cite)

### G1 — The artifact is explicitly NOT a migration; it self-resets destructively
`schema-artifact/01_schema.sql:1-32` (header + preamble):
```sql
-- Temper — Arc-1 destination schema (one-shot artifact, NOT a migration)
-- ... written as a *target* so it can be loaded into a separate Postgres namespace ...
-- Namespace: everything lands in `temper_next`. Extensions (`vector`, the
-- `uuid_generate_v7()` generator) live in `public` and are reached via search_path.
DROP SCHEMA IF EXISTS temper_next CASCADE;
CREATE SCHEMA temper_next;
SET search_path TO temper_next, public;
```
The `DROP SCHEMA ... CASCADE` is the test-reset preamble. It must **never** run as a production migration (§D: "a destructive migration here would move the blocker earlier — the discipline is load-bearing").

### G2 — Precedent: a sqlx migration that creates a non-`public` schema (run-once, no DROP)
`migrations/20260518000001_event_substrate_schema.sql:1-9`:
```sql
-- Event substrate v1 schema.
CREATE SCHEMA event_substrate;
CREATE TYPE event_substrate.porosity AS ENUM ('access', 'attention');
CREATE TABLE event_substrate.profiles ( id uuid PRIMARY KEY DEFAULT public.uuid_generate_v7(), ... );
```
The additive install migration follows this shape: `CREATE SCHEMA temper_next;` (run-once) + the shared DDL body, **no DROP**.

### G3 — The artifact mutation functions (signatures + emitted events), all through one writer
`schema-artifact/02_functions.sql`:
- `resource_create(p_payload jsonb, p_content jsonb, p_emitter uuid) RETURNS uuid` (699-707) → emits `resource_created`, projects via `_project_resource_created` → `_project_blocks`.
- `relationship_assert(p_payload jsonb, p_emitter uuid) RETURNS uuid` (761-769) → emits `relationship_asserted`.
- `relationship_fold(p_payload jsonb, p_emitter uuid) RETURNS uuid` (788-801) → emits `relationship_folded`; reads the edge's own home for the anchor.
- `facet_set(p_payload jsonb, p_emitter uuid) RETURNS uuid` (823-836) → emits `property_asserted`; anchor derived from the **owner resource's home** (errors if homeless).
- `block_mutate(p_payload jsonb, p_content jsonb, p_emitter uuid) RETURNS uuid` (888-911) → emits `block_mutated`; rejects empty chunk sets.
- `_event_append(p_type_name, p_emitter, p_anchor_table, p_anchor_id, p_payload, ...)` (715-733) — the single ledger writer; `correlation_id` self-defaults to the event id.

### G4 — The create-path projector and the shared chunk writer
`schema-artifact/02_functions.sql`:
- `_project_resource_created` (676-697): inserts `kb_resources (id, title, origin_uri, created, updated)`, `kb_resource_homes (resource_id, anchor_table, anchor_id, originator_profile_id, owner_profile_id, created)` from `p_payload#>>'{home,table}'`/`{home,id}` and `owner_profile_id`; calls `_project_blocks(v_resource, p_event, p_payload->'blocks', p_content)`; optional `doc_type` property.
- `_project_blocks` (573-615): per BlockManifest → `kb_content_blocks (id, resource_id, seq, genesis_event_id, last_event_id, created)`; optional `block_role` property; per chunk → `_insert_chunk(...)`; derives `block_body_hash` = `sha256(ordered chunk content_hashes)`; calls `_recompute_resource_body_hash`.
- `_insert_chunk(p_chunk, p_block, p_resource, p_chunk_index, p_version, p_content_hash, p_emb jsonb, p_is_current, p_content, p_occurred)` (518-534): inserts `kb_chunks (id, block_id, resource_id, chunk_index, version, content_hash, embedding, is_current, created)` + `kb_chunk_content (chunk_id, content)`. **`header_path` and `heading_depth` are NOT written** (left NULL). `p_emb` handling: JSON null⇒NULL; JSON string⇒`(p_emb#>>'{}')::vector` (replay); JSON array⇒`(p_emb::text)::vector` (fire).
- The content sidecar shape (566): `{ "<chunk_id>": { "content": text, "embedding": [f32;768] | "[...]" | null } }`.
- `_recompute_resource_body_hash` (541-559): `body_hash` = hex sha256 merkle over per-block(sha256 of `is_current` chunk content_hashes in `chunk_index` order), blocks in `seq` order. **This is a structural merkle, not the markdown sha256.**

### G5 — Artifact destination tables (columns the synthesis must populate)
`schema-artifact/01_schema.sql`:
- `kb_resources (id, title, origin_uri, body_hash, is_active, created, updated)` (150-162) — slimmed; **no** `slug`, `kb_context_id`, `doc_type`, profile columns.
- `kb_resource_homes (id, resource_id UNIQUE, anchor_table CHECK IN ('kb_contexts','kb_cogmaps'), anchor_id, originator_profile_id, owner_profile_id, created)` (210-219).
- `kb_chunks (id, block_id, resource_id, chunk_index, version, header_path TEXT, heading_depth SMALLINT, content_hash TEXT, embedding vector(768), is_current, created)` (343-364) — note `header_path`/`heading_depth` columns **exist** but are unwritten by `_insert_chunk`.
- `kb_chunk_content (chunk_id PK, content TEXT)` (367-370).
- `kb_content_blocks (id, resource_id, seq, is_folded, genesis_event_id, last_event_id, created, UNIQUE(resource_id, seq))` (325-338).
- `kb_edges (id, source_table, source_id, target_table, target_id, edge_kind, polarity, label, weight, home_anchor_table, home_anchor_id, asserted_by_event_id, last_event_id, is_folded, created)` (412-434).
- `kb_properties (id, owner_table CHECK IN ('kb_resources','kb_cogmaps','kb_edges','kb_content_blocks'), owner_id, property_key, property_value JSONB, weight, asserted_by_event_id, last_event_id, is_folded, created, UNIQUE(owner_table, owner_id, property_key, property_value))` (438-453).
- `kb_contexts (id, name UNIQUE, created)` (98-102) — thin, unowned.
- `kb_entities (id, profile_id, name, metadata JSONB, created)` (88-95).
- `kb_events (... emitter_entity_id, producing_anchor_table CHECK IN ('kb_contexts','kb_cogmaps'), producing_anchor_id, payload, "references", payload_version, occurred_at, created)` (275-295).
- `kb_team_contexts (context_id, team_id, PRIMARY KEY(context_id, team_id))` (201-206) — the chunk-1 amendment; present in the artifact already.

### G6 — Production source tables (what synthesis reads FROM, `public.*`)
- `kb_resources (id, kb_context_id, kb_doc_type_id, origin_uri UNIQUE, title, slug VARCHAR(256), originator_profile_id, owner_profile_id, is_active, created, updated)` — `migrations/20260330000001:66-89`, slug partial-unique `20260513065121:17-19`.
- `kb_resource_manifests (resource_id PK, body_hash, managed_meta JSONB, open_meta JSONB, managed_hash, open_hash, updated)` — `20260404000002:10-18`.
- `kb_chunks (id, resource_id, chunk_index, version, header_path TEXT DEFAULT '', heading_depth SMALLINT DEFAULT 0, content_hash VARCHAR(64), embedding vector(768), is_current, created, ...)` — `20260330000001:93-111`, `20260411000001:3`.
- `kb_chunk_content (chunk_id PK, content TEXT)` — `20260401000002:7-10`.
- `kb_resource_edges (id, source_resource_id, target_resource_id, edge_kind, polarity, label, weight, asserted_by_event_id, last_event_id, is_folded, created, updated)` — `20260411000002` + `20260522100002`.
- `kb_contexts (id, name, kb_owner_table, kb_owner_id, created, updated, UNIQUE(kb_owner_table, kb_owner_id, name))` — `20260330000001:19-29`.
- View `kb_current_chunks` (used by production `get_content`) and view `vault_resources_browse` (used by `list_visible`).
- **`kb_properties` does NOT exist in production** — it is a destination-only shape; production carries workflow fields as `managed_meta` keys.

### G7 — The §7 manifest-key fates (production keys → destination)
Exactly 16 managed keys exist in production. Spec §7 fate table (binding):
| Key | Fate |
|---|---|
| `temper-title`, `temper-slug`, `temper-id`, `temper-context` | **die** (title is `kb_resources.title`; slug is render-time decoration; id is `kb_resources.id`; context derives from home) |
| `temper-stage`, `temper-mode`, `temper-effort`, `temper-status`, `temper-seq` | `kb_properties` rows (workflow fields) |
| `temper-llm-run`, `temper-provenance`, `temper-branch`, `temper-pr`, `date` | `kb_properties` rows verbatim |
| `temper-goal` | **edge**, not a property (handled by Task 8) |
| `temper-type` | reconcile against the doctype column — **column wins** (stray dies) |
| all `open_meta` keys | `kb_properties` rows verbatim |

### G8 — temper-goal → edge mapping (verified against graph.rs)
`crates/temper-core/src/types/graph.rs:105-115`:
```rust
pub fn legacy_mapping(self) -> (EdgeKind, Polarity, &'static str) {
    match self {
        Self::ParentOf => (EdgeKind::Contains, Polarity::Forward, "parent_of"),
        ...
```
`crates/temper-api/src/services/edge_service.rs:370-403` — for `doc_type == "task"`, a non-empty `managed_meta["temper-goal"]` yields `(EdgeType::ParentOf, TargetRef)`, **reversed at resolution** (edge_service.rs:132-135) so the edge runs **goal → task**, with `intent="derived"` event metadata. So a temper-goal becomes: `kb_edges` row `source=goal, target=task, edge_kind=contains, polarity=forward, label='parent_of', weight=1.0`. **Production has only 68 `contains` edges but 363 `temper-goal` keys** — most are NOT materialized in `kb_resource_edges`, so synthesis must MINT them from the key and dedup against any already present (Task 8).

### G9 — Production body reconstruction + hashing (the parity oracle)
`crates/temper-api/src/services/resource_service.rs:438-494` (`get_content`): fetches from `kb_current_chunks` ordered by `chunk_index` (`chunk_index, header_path, heading_depth, content`), then:
```rust
let markdown = chunks.into_iter().map(|c| {
    if c.heading_depth == 0 { c.content }
    else {
        let title = if c.header_path.is_empty() { "Untitled" }
            else { c.header_path.rsplit(" > ").next().unwrap_or(&c.header_path) };
        let depth = (c.heading_depth as usize).min(6);
        let hashes = "#".repeat(depth);
        format!("{hashes} {title}\n\n{}", c.content)
    }
}).collect::<Vec<_>>().join("\n\n");
```
`crates/temper-core/src/hash.rs:19` (`compute_body_hash`): `format!("sha256:{}", hex::encode(Sha256(body)))` — sha256 of the assembled markdown, **"sha256:"-prefixed**. `crates/temper-ingest/src/chunk.rs:41-45,343` (`sha256_hex`): per-chunk `content_hash` = lowercase hex sha256 of the **trimmed** chunk content (no heading prefix, no "sha256:" prefix).

> **Load-bearing consequence (do not get this wrong):** the destination `body_hash` (G4 merkle) and production `body_hash` (G9 markdown sha256) are **different values by construction**. The §8 parity gate compares **reconstructed body TEXT** (string equality), never the two `body_hash` columns.

### G10 — temper-next Rust surfaces synthesis will use/extend
- `crates/temper-next/src/main.rs` — current bin is a positional-arg harness (`embed_chunks` → `materialize_cogmap`); will gain clap subcommand dispatch (Task 4).
- `crates/temper-next/src/events.rs:217-452` — `fire()` dispatch; each action is `sqlx::query_scalar!("SELECT <fn>($1,...)", serde_json::to_value(&payload)?, ...)`. `Fired` record-set enum (163-176).
- `crates/temper-next/src/content.rs:57-101` — `prepare_block`/`prepare_blocks` (re-chunks + re-embeds from prose — **NOT** the synthesis path; synthesis carries verbatim).
- `crates/temper-next/src/payloads.rs:85-104` — `ChunkManifest { chunk_id, chunk_index, content_hash }`, `BlockManifest { block_id, seq, role, chunks }`.
- `crates/temper-next/src/ids.rs:11-98` — typed-UUID newtypes (`ResourceId`, `BlockId`, `ChunkId`, `EdgeId`, `PropertyId`, `EntityId`, `ContextId`, …), `.uuid()` at the sqlx-bind boundary.
- `crates/temper-next/src/substrate.rs:14-29` — `connect()` sets `search_path = temper_next, public` (so the same pool reads `public.*` and writes `temper_next.*`).
- `crates/temper-next/tests/common/mod.rs` — `reset_artifact()` loads `01_schema`+`02_functions` via psql; `load_files` runs `psql -v ON_ERROR_STOP=1 -f`.
- `crates/temper-next/Cargo.toml:7-20` — `artifact-tests` gates the write-path tests (own the namespace, serialized via `temper-next-write` nextest group in `.config/nextest.toml`). Regenerate the offline cache after SQL changes with `cargo make prepare-next`.

---

## Spec Invariants (carried verbatim — GD-4; read the cited §§ before implementing)

From `docs/superpowers/specs/2026-06-12-ws6-convergence-delta-adjudication-design.md`:

- **§0 (backfill unit):** *"Backfill is genesis-event synthesis from current projected state — the old ledger is not the migration source."* *"Synthesis covers active state only: soft-deleted resources … are not synthesized."* *"Per live resource: `resource_created` (with block/chunk manifests per §8) → `property_asserted` per surviving key (§7) → `relationship_asserted` per edge (§4); folded rows synthesize as assert + fold event pairs."*
- **§1c (producing anchor):** *"the subject's home anchor — `('kb_contexts', ctx)` for context-homed resources; edge events anchor at the edge's home."*
- **§2 (homes):** *"Every context-homed resource → home row `('kb_contexts', ctx)` carrying its current originator/owner."* *"`temper-context` frontmatter key dies — derivable from the home row at render time."*
- **§4 (edges):** *"Per live edge: synthesize `relationship_asserted` from the `kb_resource_edges` row — kind, polarity, label, weight verbatim; home anchor per §1c; the one folded edge synthesizes as an assert+fold pair."* *"The 8→4+label mapping is final and frozen … No event-vocabulary remap exists anywhere in the migration."*
- **§7 (manifest dissolution):** *"The manifests table drops entirely — no successor reads."* Plus the per-key fate table (G7). *"`temper-goal` → edge, using the kind+label the existing frontmatter-edge projection emits — verified against `graph.rs`."* *"`temper-type` … column wins."*
- **§8 (content tier):** *"Single block per resource at migration — every existing resource backfills as one up-front content block containing its current chunk-set verbatim: chunks, sha256 content hashes, and bge-768 embeddings carry as-is (embeddings are non-replayed derived state; carrying beats recomputing)."* *"The per-resource hash-parity gate: recomputing the body text from synthesized blocks/chunks must reproduce the same content the production read path serves today, per resource, before cutover proceeds."*
- **§9 (read homes / migration-time floor):** *"No functionality regression at cutover: today's FTS, vector search, and graph reads carry, rebuilt against the new schema in the homes above."* Graph traversal/neighbors → kernel; FTS/unified search → Domain-A operational rebuilt in the API tier; URI/addressing → temper-workflow.
- **§D (deployment):** *"Chunk 2's migrations stay strictly additive — new tables/schema alongside the live ones, synthesis an explicitly-invoked operation, never a migrate-time side effect."* *"Chunk 3 is read-only parity tooling."*

---

## File Structure

**Created:**
- `schema-artifact/00_namespace_reset.sql` — the destructive test-reset preamble factored out of `01_schema.sql` (DROP+CREATE SCHEMA+search_path).
- `migrations/<ts>_install_temper_next.sql` — **generated**, run-once additive install (`CREATE SCHEMA temper_next;` + shared body of `01_schema.sql` sans reset + `02_functions.sql`).
- `crates/temper-next/src/synthesis/mod.rs` — synthesis entry (`run`), the orchestration over §0's per-resource sequence.
- `crates/temper-next/src/synthesis/source.rs` — typed reads of `public.*` (resources, manifests, chunks, edges, contexts).
- `crates/temper-next/src/synthesis/bootstrap.rs` — migration/per-surface entities + profiles + contexts (§1).
- `crates/temper-next/src/synthesis/parity.rs` — the per-resource body-text parity gate (§8) + the production-reconstruction algorithm port.
- `crates/temper-next/src/readback/mod.rs` — chunk-3 read implementations over `temper_next.*` (list/show/meta/body/fts/vector/graph).
- `crates/temper-next/tests/fixtures/prod_shape.sql` — a small production-shape `public.*` fixture for synthesis + parity tests.
- `crates/temper-next/tests/synthesis.rs`, `crates/temper-next/tests/parity_reads.rs` — the integration tests.
- `tools/gen-install-migration.sh` (or a `cargo make gen-install-migration` task) — the generator that emits the install migration from the shared artifact body.

**Modified:**
- `schema-artifact/01_schema.sql` — remove the inline destructive preamble (moves to `00_namespace_reset.sql`); add `header_path`/`heading_depth` carry to the chunk write path is in `02_functions.sql`.
- `schema-artifact/02_functions.sql` — EXTEND `_insert_chunk`, `_project_blocks`, `_project_block_mutated` to carry `header_path`/`heading_depth`; sidecar entry gains those keys.
- `crates/temper-next/src/payloads.rs` — sidecar serialization gains `header_path`/`heading_depth`.
- `crates/temper-next/src/content.rs` — `PreparedChunk` gains `header_path`/`heading_depth` (defaults for the existing scenario path).
- `crates/temper-next/src/main.rs` — clap subcommand dispatch (`materialize` | `synthesize`).
- `crates/temper-next/src/lib.rs` — `pub mod synthesis; pub mod readback;`.
- `crates/temper-next/tests/common/mod.rs` — add a `reset_artifact` variant that also seeds the prod-shape fixture into `public`.
- `crates/temper-next/Cargo.toml` + `.config/nextest.toml` — add the new test binaries to the `temper-next-write` group; add `clap` dep.
- `Makefile.toml` — `gen-install-migration` task + a drift-check wired into `check`.

---

## CHUNK 2 — PHASE A: Shared idempotent schema source + additive install migration

### Task 1: Factor the artifact into a shared body + reset preamble; add the install-migration generator + drift guard

**Tag: AMEND** — changes `schema-artifact/01_schema.sql` (G1) to separate the destructive reset (load-bearing test behavior) from the DDL body. Authorized by Pete's "make artifact idempotent & shared" decision and §D ("strictly additive … new tables/schema alongside the live ones").

**Files:**
- Create: `schema-artifact/00_namespace_reset.sql`
- Modify: `schema-artifact/01_schema.sql:30-32` (remove inline DROP/CREATE/SET)
- Modify: `crates/temper-next/tests/common/mod.rs` (prepend the reset file)
- Create: `tools/gen-install-migration.sh`, `Makefile.toml` task
- Test: `crates/temper-next/tests/schema_drift.rs`

- [ ] **Step 1: Write the failing drift test**

```rust
// crates/temper-next/tests/schema_drift.rs
#![cfg(feature = "artifact-tests")]
//! Guards the single-source invariant: the committed install migration must equal the generator's
//! output from the shared artifact body. If artifact SQL changes without regenerating, this fails.
mod common;
use std::process::Command;

#[test]
fn install_migration_matches_generated() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
    let out = Command::new("bash")
        .arg(format!("{root}/tools/gen-install-migration.sh"))
        .arg("--stdout")
        .output()
        .expect("generator runs");
    assert!(out.status.success(), "generator failed: {}", String::from_utf8_lossy(&out.stderr));
    let generated = String::from_utf8(out.stdout).unwrap();
    let committed = common::read_latest_install_migration(root);
    assert_eq!(generated, committed, "install migration is stale — run `cargo make gen-install-migration`");
}
```

- [ ] **Step 2: Run it; verify it fails**

Run: `cargo nextest run -p temper-next --features artifact-tests install_migration_matches_generated`
Expected: FAIL — generator script and `read_latest_install_migration` don't exist yet.

- [ ] **Step 3: Factor the reset preamble out of `01_schema.sql`**

Create `schema-artifact/00_namespace_reset.sql`:
```sql
-- Destructive namespace reset — TEST-ONLY preamble (NOT part of the additive install migration).
-- The artifact body (01_schema.sql) is namespace-resident DDL with no DROP; this file is what the
-- test harness prepends to own + reset the namespace. The production install migration prepends a
-- run-once `CREATE SCHEMA temper_next;` instead (see tools/gen-install-migration.sh).
DROP SCHEMA IF EXISTS temper_next CASCADE;
CREATE SCHEMA temper_next;
SET search_path TO temper_next, public;
```
Remove lines 30-32 of `01_schema.sql` (the DROP/CREATE/SET) and replace with a `SET search_path TO temper_next, public;` only (the body still needs the search_path; it just must not DROP/CREATE the schema). The install generator supplies its own `CREATE SCHEMA` + `SET search_path`.

- [ ] **Step 4: Write the generator**

`tools/gen-install-migration.sh`:
```bash
#!/usr/bin/env bash
# Emits the run-once additive install migration from the shared artifact body. Single source of truth:
# schema-artifact/01_schema.sql (body, no DROP) + 02_functions.sql. Output is deterministic.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
ART="$ROOT/schema-artifact"
emit() {
  printf -- '-- GENERATED by tools/gen-install-migration.sh — do not edit by hand.\n'
  printf -- '-- Source of truth: schema-artifact/01_schema.sql + 02_functions.sql.\n'
  printf -- '-- Additive, run-once: creates the temper_next namespace alongside public (WS6 chunk 2, §D).\n\n'
  printf -- 'CREATE SCHEMA temper_next;\nSET search_path TO temper_next, public;\n\n'
  # 01_schema.sql already had its DROP/CREATE/SET removed (Task 1 Step 3); strip any residual SET search_path.
  grep -vE '^SET search_path' "$ART/01_schema.sql"
  printf -- '\n'
  grep -vE '^SET search_path' "$ART/02_functions.sql"
}
if [[ "${1:-}" == "--stdout" ]]; then emit; else
  TS="$(ls "$ROOT/migrations" | grep -oE '^[0-9]+' | sort | tail -1)"  # operator picks the real ts; placeholder for regen-in-place
  emit > "$ROOT/migrations/${TS}_install_temper_next.sql"
fi
```
Add to `Makefile.toml`:
```toml
[tasks.gen-install-migration]
script = ["bash tools/gen-install-migration.sh"]
```
Add `common::read_latest_install_migration` to `tests/common/mod.rs` (reads `migrations/*_install_temper_next.sql`), and update `reset_artifact()` to load `["00_namespace_reset", "01_schema", "02_functions"]`.

- [ ] **Step 5: Run the test; verify it passes**

Run: `cargo nextest run -p temper-next --features artifact-tests install_migration_matches_generated`
Expected: PASS (after Task 2 commits the generated migration; if run before Task 2, this task's test asserts generator==committed — commit the generated file in Task 2 and re-run). Also run `cargo nextest run -p temper-next --features artifact-tests` to confirm existing write-path tests still reset cleanly through the new 00+01+02 sequence.

- [ ] **Step 6: Commit**

```bash
git add schema-artifact/00_namespace_reset.sql schema-artifact/01_schema.sql tools/gen-install-migration.sh Makefile.toml crates/temper-next/tests/common/mod.rs crates/temper-next/tests/schema_drift.rs
git commit -m "WS6 chunk2: factor artifact reset from body; install-migration generator + drift guard"
```

### Task 2: Generate & commit the additive install migration; prove it applies without touching `public`

**Tag: EXTEND** — new migration alongside live tables; authorized by §D. CONFORM to the migrator's run-once contract (G2).

**Files:**
- Create: `migrations/<ts>_install_temper_next.sql` (generated)
- Test: `crates/temper-next/tests/install_migration.rs`

- [ ] **Step 1: Write the failing test**

```rust
// crates/temper-next/tests/install_migration.rs
#![cfg(feature = "artifact-tests")]
//! Proves the install migration is additive: applied to a DB with `public` present, it creates the
//! temper_next namespace + tables and leaves public's table set unchanged.
mod common;
use sqlx::Row;

#[tokio::test]
async fn install_is_additive_and_creates_namespace() {
    let pool = temper_next::substrate::connect().await.unwrap();
    // public table count before (a clean migrated dev DB).
    let before: i64 = sqlx::query("SELECT count(*) FROM information_schema.tables WHERE table_schema='public'")
        .fetch_one(&pool).await.unwrap().get(0);
    // apply the generated install migration into a fresh temper_next.
    common::apply_install_migration(&pool).await;
    let after: i64 = sqlx::query("SELECT count(*) FROM information_schema.tables WHERE table_schema='public'")
        .fetch_one(&pool).await.unwrap().get(0);
    assert_eq!(before, after, "install migration must not touch public");
    let next_tables: i64 = sqlx::query("SELECT count(*) FROM information_schema.tables WHERE table_schema='temper_next'")
        .fetch_one(&pool).await.unwrap().get(0);
    assert!(next_tables >= 20, "temper_next tables created, got {next_tables}");
}
```

- [ ] **Step 2: Run it; verify it fails**

Run: `cargo nextest run -p temper-next --features artifact-tests install_is_additive`
Expected: FAIL — migration file and `common::apply_install_migration` don't exist.

- [ ] **Step 3: Generate & commit the migration; add the test helper**

```bash
cargo make gen-install-migration   # writes migrations/<ts>_install_temper_next.sql
```
Add `common::apply_install_migration(pool)` that drops `temper_next` then runs the committed install migration via psql (proving the run-once `CREATE SCHEMA` path works on an absent namespace).

- [ ] **Step 4: Run; verify it passes**

Run: `cargo nextest run -p temper-next --features artifact-tests install_is_additive`
Expected: PASS. Also: `cargo make docker-up && DATABASE_URL=... sqlx migrate run` then `psql -c "\dn"` shows both `public` and `temper_next` — confirm by hand the dev DB migrates cleanly.

- [ ] **Step 5: Regenerate the temper-next offline cache and run check**

Run: `cargo make prepare-next && cargo make check`
Expected: PASS (no SQL macro changes yet, but the new test queries must be cached).

- [ ] **Step 6: Commit**

```bash
git add migrations/*_install_temper_next.sql crates/temper-next/tests/install_migration.rs crates/temper-next/tests/common/mod.rs crates/temper-next/.sqlx
git commit -m "WS6 chunk2: generated additive install migration for temper_next (public untouched)"
```

---

## CHUNK 2 — PHASE B: Carry production heading metadata into the chunk model (§8 EXTEND)

### Task 3: EXTEND the chunk write path to carry `header_path` + `heading_depth` verbatim

**Tag: EXTEND** — §8 (*"chunks … carry as-is … recomputing the body text from synthesized blocks/chunks must reproduce the same content the production read path serves today"*). The artifact's `_insert_chunk` (G4) does not write `header_path`/`heading_depth`, and production reconstructs body using exactly those (G9). Without this, body-text parity (Task 9) is impossible.

**Invariant to preserve (verbatim, §8):** *"chunks, sha256 content hashes, and bge-768 embeddings carry as-is."* The carry must not alter `content_hash` or the body_hash merkle — `header_path`/`heading_depth` are render metadata in the **sidecar**, never in the manifest/CAS hash.

**Files:**
- Modify: `schema-artifact/02_functions.sql` (`_insert_chunk` 518-534, `_project_blocks` 601-603, `_project_block_mutated` 869-871)
- Modify: `crates/temper-next/src/content.rs` (`PreparedChunk`), `crates/temper-next/src/payloads.rs` (sidecar serialization)
- Test: `crates/temper-next/tests/chunk_heading_carry.rs`

- [ ] **Step 1: Write the failing artifact test**

```rust
// crates/temper-next/tests/chunk_heading_carry.rs
#![cfg(feature = "artifact-tests")]
//! A resource_create whose sidecar carries header_path + heading_depth persists them onto kb_chunks,
//! so a downstream read can reconstruct headed markdown identically to production.
mod common;
use sqlx::Row;

#[tokio::test]
async fn chunk_carries_header_path_and_heading_depth() {
    common::reset_artifact();
    let pool = temper_next::substrate::connect().await.unwrap();
    temper_next::scenario::bootseed::seed_system(&pool).await.unwrap();
    // fire a resource_create with a single block, one chunk carrying heading metadata in the sidecar.
    let resource = common::fire_resource_with_headed_chunk(&pool, "Intro > Goals", 2_i16).await;
    let row = sqlx::query("SELECT header_path, heading_depth FROM kb_chunks WHERE resource_id=$1")
        .bind(resource).fetch_one(&pool).await.unwrap();
    assert_eq!(row.get::<String,_>("header_path"), "Intro > Goals");
    assert_eq!(row.get::<i16,_>("heading_depth"), 2);
}
```

- [ ] **Step 2: Run; verify it fails**

Run: `cargo nextest run -p temper-next --features artifact-tests chunk_carries_header_path`
Expected: FAIL — `header_path` reads back NULL (or the helper doesn't compile).

- [ ] **Step 3: EXTEND the SQL writer + the Rust sidecar**

In `schema-artifact/02_functions.sql`, change `_insert_chunk` to accept and write the two columns:
```sql
CREATE FUNCTION _insert_chunk(p_chunk uuid, p_block uuid, p_resource uuid, p_chunk_index int,
                              p_version int, p_content_hash text, p_emb jsonb, p_is_current boolean,
                              p_content text, p_header_path text, p_heading_depth smallint,
                              p_occurred timestamptz)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO kb_chunks (id, block_id, resource_id, chunk_index, version, content_hash,
                           embedding, is_current, header_path, heading_depth, created)
        VALUES (p_chunk, p_block, p_resource, p_chunk_index, p_version, p_content_hash,
                CASE
                    WHEN p_emb IS NULL OR jsonb_typeof(p_emb) = 'null' THEN NULL
                    WHEN jsonb_typeof(p_emb) = 'string' THEN (p_emb #>> '{}')::vector
                    ELSE (p_emb::text)::vector
                END,
                p_is_current, p_header_path, p_heading_depth, p_occurred);
    INSERT INTO kb_chunk_content (chunk_id, content) VALUES (p_chunk, p_content);
END;
$$;
```
Update both call sites to read the sidecar's new keys (`_project_blocks:601-603` and `_project_block_mutated:869-871`):
```sql
PERFORM _insert_chunk(v_chunk, v_block, p_resource, (v_chunk_json->>'chunk_index')::int,
                      1, v_chunk_json->>'content_hash', v_side->'embedding', true,
                      v_side->>'content', v_side->>'header_path',
                      NULLIF(v_side->>'heading_depth','')::smallint, v_occurred);
```
In `crates/temper-next/src/content.rs`, add `header_path: Option<String>` and `heading_depth: Option<i16>` to `PreparedChunk` (default `None` for the existing `prepare_block` scenario path — those chunks have no production headings). In `crates/temper-next/src/payloads.rs`, the sidecar entry serialization gains `"header_path"` and `"heading_depth"` (skip-if-none).

- [ ] **Step 4: Run; verify it passes + existing write-path suite still green**

Run: `cargo nextest run -p temper-next --features artifact-tests chunk_carries_header_path`
Expected: PASS.
Run: `cargo nextest run -p temper-next --features artifact-tests`
Expected: PASS (the existing scenario tests use `header_path=None` → writes NULL, unchanged behavior).

- [ ] **Step 5: Regenerate caches + install migration + check**

Run: `cargo make gen-install-migration && cargo make prepare-next && cargo make check`
Expected: PASS. (The install migration must regenerate so its drift guard from Task 1 stays green.)

- [ ] **Step 6: Commit**

```bash
git add schema-artifact/02_functions.sql crates/temper-next/src/content.rs crates/temper-next/src/payloads.rs crates/temper-next/tests/chunk_heading_carry.rs migrations/*_install_temper_next.sql crates/temper-next/.sqlx
git commit -m "WS6 chunk2 §8: carry header_path + heading_depth through the chunk write path"
```

---

## CHUNK 2 — PHASE C: The synthesis-from-state operation

### Task 4: Synthesis scaffolding — module, source reads, and the `synthesize` bin subcommand

**Tag: EXTEND** — new operation; §0/§D authorize it. CONFORM to `substrate::connect()` (G10, reads `public` + writes `temper_next` on one pool) and the `events::fire` surface (G3/G10).

**Files:**
- Create: `crates/temper-next/src/synthesis/mod.rs`, `crates/temper-next/src/synthesis/source.rs`
- Modify: `crates/temper-next/src/lib.rs`, `crates/temper-next/src/main.rs`, `crates/temper-next/Cargo.toml`
- Test: `crates/temper-next/tests/synthesis_source.rs`

- [ ] **Step 1: Write the failing source-read test**

```rust
// crates/temper-next/tests/synthesis_source.rs
#![cfg(feature = "artifact-tests")]
//! synthesis::source reads active production-shape rows from public.* (the synthesis source).
mod common;

#[tokio::test]
async fn source_reads_active_resources_only() {
    let pool = temper_next::substrate::connect().await.unwrap();
    common::seed_prod_shape_fixture(&pool).await;   // inserts 3 active + 1 soft-deleted into public.*
    let rows = temper_next::synthesis::source::active_resources(&pool).await.unwrap();
    assert_eq!(rows.len(), 3, "soft-deleted resource excluded (§0 active-only)");
}
```

- [ ] **Step 2: Run; verify it fails**

Run: `cargo nextest run -p temper-next --features artifact-tests source_reads_active`
Expected: FAIL — `synthesis` module absent; `seed_prod_shape_fixture` absent.

- [ ] **Step 3: Implement the source reads + the prod-shape fixture + bin dispatch**

Create `crates/temper-next/tests/fixtures/prod_shape.sql` — a minimal `public.*` seed: 3 active + 1 soft-deleted `kb_resources` with `kb_resource_manifests` (managed/open meta covering the §7 key spread incl. one task with `temper-goal`), `kb_chunks`+`kb_chunk_content` (with header_path/heading_depth + fixed 768-d embeddings), one `kb_resource_edges` row (and one folded), two `kb_contexts`. Add `common::seed_prod_shape_fixture`.

`crates/temper-next/src/synthesis/source.rs` — typed reads of `public.*` using runtime `sqlx::query_as` where pgvector is involved, macros otherwise. Newtype-bind via `.uuid()`:
```rust
pub struct SourceResource {
    pub id: uuid::Uuid, pub title: String, pub origin_uri: String,
    pub kb_context_id: uuid::Uuid, pub doc_type: String,
    pub originator_profile_id: uuid::Uuid, pub owner_profile_id: uuid::Uuid,
    pub managed_meta: serde_json::Value, pub open_meta: serde_json::Value,
}
pub async fn active_resources(pool: &PgPool) -> Result<Vec<SourceResource>> { /* SELECT ... FROM public.kb_resources r JOIN public.kb_resource_manifests m ON m.resource_id=r.id JOIN public.kb_doc_types dt ON dt.id=r.kb_doc_type_id WHERE r.is_active */ }
pub struct SourceChunk { pub chunk_index: i32, pub content_hash: String, pub content: String,
    pub header_path: String, pub heading_depth: i16, pub embedding: Vec<f32> }
pub async fn chunks_for(pool: &PgPool, resource: uuid::Uuid) -> Result<Vec<SourceChunk>> { /* public.kb_current_chunks JOIN kb_chunk_content ORDER BY chunk_index; embedding via runtime query */ }
pub struct SourceEdge { pub id: uuid::Uuid, pub source: uuid::Uuid, pub target: uuid::Uuid,
    pub edge_kind: String, pub polarity: String, pub label: String, pub weight: f64, pub is_folded: bool }
pub async fn edges(pool: &PgPool) -> Result<Vec<SourceEdge>> { /* public.kb_resource_edges, both endpoints active */ }
pub struct SourceContext { pub id: uuid::Uuid, pub name: String }
pub async fn contexts(pool: &PgPool) -> Result<Vec<SourceContext>> { /* public.kb_contexts */ }
```

`crates/temper-next/src/main.rs` — replace positional dispatch with clap:
```rust
#[derive(clap::Parser)]
enum Cmd {
    /// Embed + materialize a cogmap's regions (the existing harness).
    Materialize { #[arg(default_value="onboarding-cogmap")] cogmap: String,
                  #[arg(default_value="telos-default")] lens: String },
    /// Synthesize the temper_next substrate from current public.* state (WS6 §0).
    Synthesize { /// stop after N resources (rehearsal); 0 = all
                 #[arg(long, default_value_t=0)] limit: usize },
}
```
`Cmd::Synthesize` calls `synthesis::run(&pool, RunOpts { limit })`. Add `clap` to `Cargo.toml` (workspace version). `synthesis::run` is a stub returning `Ok(SynthReport::default())` for now (filled in Tasks 5-9).

Add `pub mod synthesis;` to `lib.rs`.

- [ ] **Step 4: Run; verify it passes**

Run: `cargo nextest run -p temper-next --features artifact-tests source_reads_active`
Expected: PASS.

- [ ] **Step 5: Regenerate caches + check**

Run: `cargo make prepare-next && cargo make check`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-next/src/synthesis/ crates/temper-next/src/lib.rs crates/temper-next/src/main.rs crates/temper-next/Cargo.toml crates/temper-next/tests/fixtures/ crates/temper-next/tests/synthesis_source.rs crates/temper-next/tests/common/mod.rs crates/temper-next/.sqlx
git commit -m "WS6 chunk2: synthesis scaffolding — source reads, prod-shape fixture, synthesize subcommand"
```

### Task 5: Synthesis bootstrap — migration entity, per-surface entities, profiles, contexts (§1, §2)

**Tag: EXTEND/CONFORM** — §1a (*"one `migration` entity bound to Pete's profile, `metadata: {intent: 'migration', …}`"*), §1b (per-(profile,surface) entities `pete@cli`/`pete@mcp`/`pete@web`), §2 (contexts migrate by name into the thin unowned `kb_contexts`). Entity creation is administrative — **no event** (§1 open residue: *"Entity creation stays administrative (no event)"*).

**Files:** Create logic in `crates/temper-next/src/synthesis/bootstrap.rs`; Test: `crates/temper-next/tests/synthesis_bootstrap.rs`

- [ ] **Step 1: Failing test** — after `bootstrap::run`, assert: `temper_next.kb_contexts` has the fixture's context names; a `migration` entity exists with `metadata->>'intent' = 'migration'`; `pete@cli`/`pete@mcp`/`pete@web` entities exist; `kb_profiles` carries the originator/owner profile(s) from the fixture.
- [ ] **Step 2: Run; verify FAIL.**
- [ ] **Step 3: Implement** `bootstrap::run(pool, source) -> BootstrapMaps`: insert `kb_profiles` (from distinct originator/owner ids in `source::active_resources`), `kb_entities` (migration + the three surfaces, all bound to Pete's profile; `metadata` per §1a), `kb_contexts` (name-only, from `source::contexts`). Return a `BootstrapMaps { context_id_by_old: HashMap<Uuid,Uuid>, migration_entity: EntityId, ... }` for the resource synthesis to consume. **Direct inserts** (these are administrative infrastructure, not event-sourced — CONFORM to §1 residue). Bind through newtypes' `.uuid()`.
- [ ] **Step 4: Run; verify PASS.**
- [ ] **Step 5:** `cargo make prepare-next && cargo make check`.
- [ ] **Step 6: Commit** `WS6 chunk2 §1/§2: synthesis bootstrap — migration/surface entities, profiles, contexts`.

### Task 6: Synthesize resources + homes + single content block, chunks carried verbatim (§8, §2, §1c)

**Tag: EXTEND** — §8 (*"Single block per resource … one up-front content block containing its current chunk-set verbatim"*), §2 (home `('kb_contexts', ctx)` carrying originator/owner), §1c (producing anchor = home). CONFORM to `resource_create`/`_project_blocks` payload+sidecar contract (G3/G4) and the Task-3 heading carry.

**Files:** `crates/temper-next/src/synthesis/mod.rs` (resource pass); Test: `crates/temper-next/tests/synthesis.rs` (resource assertions)

- [ ] **Step 1: Failing test** — run `synthesis::run` (resources only) over the fixture; assert each active resource has: a `kb_resources` row (title carried), a `kb_resource_homes` row anchored `('kb_contexts', mapped_ctx)` with the fixture's originator/owner, exactly one `kb_content_blocks` (seq 0), and `kb_chunks` rows whose `content_hash`/`header_path`/`heading_depth`/`content` equal the fixture's `public.kb_chunks`/`kb_chunk_content` verbatim, and a non-NULL `embedding`.
- [ ] **Step 2: Run; verify FAIL.**
- [ ] **Step 3: Implement** the resource pass in `synthesis::run`. Per `source::active_resources`:
  - Build a `BlockManifest { block_id: new, seq: 0, role: None, chunks: [ChunkManifest{chunk_id:new, chunk_index, content_hash}] }` from `source::chunks_for`.
  - Build the content sidecar map `{ chunk_id: { content, embedding: [f32;768], header_path, heading_depth } }` carrying the fixture values verbatim (embedding as a JSON array — the "fire" path of `_insert_chunk`, G4).
  - Build the `ResourceCreated` payload: `title`, `origin_uri`, `home: { table: 'kb_contexts', id: mapped_ctx }`, `owner_profile_id` (the home insert uses owner for both originator+owner per G4 `_project_resource_created`; if originator≠owner, set originator via a follow-up `UPDATE kb_resource_homes` — verify at implementation whether `_project_resource_created` supports a distinct originator, else AMEND it with a note), `blocks: [BlockManifest]`, optional `doc_type`.
  - Fire via `events::fire(SeedAction::ResourceCreate { payload, sidecar })` using the migration entity (§1c anchor is the home, set inside the payload). Record `old_resource_id -> new ResourceId`.
  > **⚠️ Plan/reality gap to verify at implementation (GD-2):** `_project_resource_created` (G4) sets BOTH `originator_profile_id` and `owner_profile_id` to `p_payload->>'owner_profile_id'`. Production carries distinct originator vs owner on some rows. Confirm by reading the function; if distinct originators must be preserved, EXTEND the payload to carry `originator_profile_id` and the projector to use it (cite §2 "carrying its current originator/owner"). Do not silently collapse them.
- [ ] **Step 4: Run; verify PASS.**
- [ ] **Step 5:** `cargo make prepare-next && cargo make check`.
- [ ] **Step 6: Commit** `WS6 chunk2 §8/§2: synthesize resources + homes + single block, chunks carried verbatim`.

### Task 7: Synthesize properties from manifest keys per the §7 fate table

**Tag: CONFORM/EXTEND** — §7 fate table (G7). CONFORM to `facet_set` (G3: anchor derived from the owner's home — the resource must already be homed, so this pass runs **after** Task 6).

**Invariant (verbatim, §7):** *"where a stray manifest key conflicts with authoritative state, state wins; the stray dies in the archive."* (`temper-type` reconciles to the doctype column — column wins; `temper-title`/`temper-slug`/`temper-id`/`temper-context` die.)

**Files:** `crates/temper-next/src/synthesis/mod.rs` (property pass) + a `key_fate` table module; Test: `crates/temper-next/tests/synthesis.rs` (property assertions)

- [ ] **Step 1: Failing test** — assert: for the fixture's task resource, `temper-stage`/`temper-mode`/`temper-effort` became `kb_properties` rows with the right `property_value`; `temper-title`/`temper-slug`/`temper-id`/`temper-context` produced **no** property; `temper-goal` produced **no property** (it's an edge, Task 8); all `open_meta` keys became properties verbatim; `temper-type` produced no property (doc_type already a property from `resource_create`).
- [ ] **Step 2: Run; verify FAIL.**
- [ ] **Step 3: Implement** a `key_fate(key: &str) -> KeyFate` enum (`Property`, `Die`, `Edge`, `ReconcileToDocType`) encoding G7 exactly (no stringly-typed scatter — one match in one place, per the "no stringly-typed matches over bounded sets" rule). For each manifest key on each resource, fire `facet_set` with `owner: { table: 'kb_resources', id: new_resource_id }`, `property_key`, `value`, `weight: 1.0` when `KeyFate::Property`. Skip `Die`/`Edge`/`ReconcileToDocType`. Process `managed_meta` then `open_meta`.
- [ ] **Step 4: Run; verify PASS.**
- [ ] **Step 5:** `cargo make prepare-next && cargo make check`.
- [ ] **Step 6: Commit** `WS6 chunk2 §7: synthesize properties from manifest keys (exhaustive fate table)`.

### Task 8: Synthesize edges (§4) + mint temper-goal edges (§7), dedup

**Tag: CONFORM/EXTEND** — §4 (per `kb_resource_edges` row → `relationship_assert` verbatim; the one folded edge → assert+fold pair), §7+G8 (temper-goal → `Contains`/`forward`/`parent_of`, reversed goal→task, minted from the key, deduped).

**Invariant (verbatim, §4):** *"kind, polarity, label, weight verbatim; home anchor per §1c; the one folded edge synthesizes as an assert+fold pair."*

**Files:** `crates/temper-next/src/synthesis/mod.rs` (edge pass); Test: `crates/temper-next/tests/synthesis.rs` (edge assertions)

- [ ] **Step 1: Failing test** — assert: every active-endpoint `public.kb_resource_edges` row produced a `temper_next.kb_edges` row with kind/polarity/label/weight verbatim, endpoints remapped to new ids, `source_table=target_table='kb_resources'`, home anchored at the edge's home (§1c — see gap note); the folded fixture edge is `is_folded=true` and has both an `asserted` and a `folded` event in `kb_events`; the fixture's `temper-goal` task produced exactly one `contains`/`forward`/`parent_of` edge `goal→task`, and it was **not** double-created if also present in `kb_resource_edges`.
- [ ] **Step 2: Run; verify FAIL.**
- [ ] **Step 3: Implement** the edge pass:
  - For each `source::edges` row: build `RelationshipAsserted` payload (`edge_id: new`, `source/target` as `{table:'kb_resources', id: new}`, `edge_kind`, `polarity`, `label`, `weight`, `home: {table:'kb_contexts', id: <edge home ctx>}`). Fire `relationship_assert`. If `is_folded`, then fire `relationship_fold` with `{edge_id}` (the assert+fold pair).
  - For each task resource with a non-empty `temper-goal`: resolve the goal target (the value is a slug/uuid — resolve against the fixture's resources by slug or trailing-uuid; CONFORM to edge_service's `TargetRef::parse`). Mint `goal→task` `(contains, forward, "parent_of", 1.0)` (G8) **unless** an edge with the same `(source,target,edge_kind,label)` was already synthesized from `kb_resource_edges` (dedup set keyed on `(new_source,new_target,kind,label)`).
  > **⚠️ Plan/reality gap to verify (GD-2):** the edge's home context. `public.kb_resource_edges` has no home column (G6). §1c says "edge events anchor at the edge's home" — for context-homed resources the home is the context. Determine the edge home at implementation: the spec's edge-home polymorphism (settled) homes a resource↔resource edge in the shared context. If both endpoints share a context, use it; if they differ, pick the source's context and record the choice. Cite the data-model edge-home decision; if ambiguous, escalate (GD-5) rather than guessing.
- [ ] **Step 4: Run; verify PASS.**
- [ ] **Step 5:** `cargo make prepare-next && cargo make check`.
- [ ] **Step 6: Commit** `WS6 chunk2 §4/§7: synthesize edges + minted temper-goal edges with dedup`.

### Task 9: The per-resource body-text parity gate (§8)

**Tag: CONFORM** — to production's `get_content` reconstruction (G9) as the parity oracle. **Compares body TEXT, not body_hash values** (the two body_hash columns differ by construction — G4 vs G9).

**Invariant (verbatim, §8):** *"recomputing the body text from synthesized blocks/chunks must reproduce the same content the production read path serves today, per resource, before cutover proceeds."*

**Files:** `crates/temper-next/src/synthesis/parity.rs`; Test: `crates/temper-next/tests/synthesis.rs` (gate assertions)

- [ ] **Step 1: Failing test** — after a full `synthesis::run`, assert `parity::body_parity_report(pool)` returns `mismatches == 0` over the fixture, and that a deliberately corrupted chunk (e.g. tweak one `temper_next.kb_chunk_content.content`) makes the report flag exactly that resource.
- [ ] **Step 2: Run; verify FAIL.**
- [ ] **Step 3: Implement** `parity::reconstruct_body(chunks: &[ReadChunk]) -> String` as a verbatim port of G9's algorithm (heading_depth==0 ⇒ content; else `format!("{hashes} {title}\n\n{}", content)` with `title = header_path.rsplit(" > ").next()`, `join("\n\n")`). `body_parity_report(pool)` reads, per synthesized resource: the production body via `public.kb_current_chunks` (the same query `get_content` uses) and the new-substrate body via `temper_next.kb_chunks WHERE is_current JOIN kb_chunk_content ORDER BY block seq, chunk_index`, runs `reconstruct_body` on both, and string-compares. `synthesis::run` calls this at the end and refuses to report success if `mismatches > 0` (returns the per-resource mismatch list in `SynthReport`).
- [ ] **Step 4: Run; verify PASS.**
- [ ] **Step 5:** `cargo make prepare-next && cargo make check`.
- [ ] **Step 6: Commit** `WS6 chunk2 §8: per-resource body-text parity gate`.

### Task 10: End-to-end synthesis integration test over the prod-shape fixture

**Tag: test** — proves the full §0 per-resource sequence (`resource_created → property_asserted → relationship_asserted`, folds as pairs) end-to-end.

**Files:** `crates/temper-next/tests/synthesis.rs` (top-level e2e); ensure the binary is in the `temper-next-write` nextest group.

- [ ] **Step 1: Write** `synthesizes_fixture_end_to_end`: reset artifact, seed prod-shape fixture, `synthesis::run(pool, RunOpts::all())`, then assert aggregate counts: `kb_resources` == 3, one `kb_content_blocks` per resource, `kb_properties` count == (sum of `Property`-fated keys), `kb_edges` count == (source edges + minted goal edges − dedup), `kb_events` has the expected event-type histogram (`resource_created` ×3, `property_asserted` ×N, `relationship_asserted` ×M, `relationship_folded` ×1), and `parity report mismatches == 0`.
- [ ] **Step 2: Run; verify FAIL → implement any missing aggregation in `SynthReport` → PASS.**
- [ ] **Step 3:** Add the binary to `.config/nextest.toml`'s `temper-next-write` filter regex (it owns/serializes the namespace).
- [ ] **Step 4: Run** `cargo nextest run -p temper-next --features artifact-tests` (full write-path suite) — verify green and serialized.
- [ ] **Step 5:** `cargo make prepare-next && cargo make check`.
- [ ] **Step 6: Commit** `WS6 chunk2: end-to-end synthesis integration test over prod-shape fixture`.

---

## CHUNK 3 — Parity-read harness (full §9 migration-time floor)

> **Read-only** (§D: "Chunk 3 is read-only parity tooling"). Each task ports one production read to `temper_next.*` and asserts identical output for the same logical query over the synthesized fixture. Shared harness scaffold first, then one read per task.

### Task 11: Parity harness scaffold + readback module

**Tag: EXTEND** — new read surface over `temper_next`; §9 floor.

**Files:** Create `crates/temper-next/src/readback/mod.rs`; Create `crates/temper-next/tests/parity_reads.rs`; Modify `lib.rs`.

- [ ] **Step 1: Failing test** — `parity_harness_setup_synthesizes`: reset, seed fixture, synthesize, then assert `readback` is reachable and returns a non-empty resource id set from `temper_next` matching the synthesized count. (A smoke test that the harness fixture + synthesis + a trivial readback compose.)
- [ ] **Step 2: Run; verify FAIL.**
- [ ] **Step 3: Implement** `readback` module skeleton with a `ResolvedIds` helper mapping `old_public_id ↔ new_temper_next_id` (read from synthesized state by `origin_uri`, which is carried verbatim and unique in both schemas — CONFORM to G5/G6 `origin_uri UNIQUE`), and a `common::synthesized_pool()` test helper. Add `pub mod readback;` to `lib.rs`.
- [ ] **Step 4-6:** Run → PASS; `cargo make prepare-next && cargo make check`; commit `WS6 chunk3: parity-read harness scaffold + readback module`.

### Task 12: `list` parity

**Tag: EXTEND** — port `list_visible` projection (G6 `vault_resources_browse`, resource_service.rs:235-295) over `temper_next`. §9 floor.

- [ ] **Step 1: Failing test** `list_parity`: for the fixture, production `list_visible` (filterless, owner-scoped) and `readback::list` return the same ordered set of `(origin_uri, title, doc_type, stage, mode, effort)` projections. (Map workflow fields from `kb_properties` in the new substrate vs `managed_meta` in production.)
- [ ] **Step 2-3:** Run → FAIL; implement `readback::list(pool) -> Vec<ListRow>` joining `kb_resources` + `kb_resource_homes` + `kb_properties` (stage/mode/effort/seq as property lookups) ordered by `updated DESC` to match production's order.
- [ ] **Step 4-6:** PASS; cache+check; commit `WS6 chunk3 §9: list parity`.

### Task 13: `show` + `get_meta` parity

**Tag: EXTEND** — port `get_visible` + `get_meta` (resource_service.rs:341-360, meta_service.rs:21-56). Reconstruct `managed_meta`/`open_meta` from `kb_properties` (the inverse of Task 7's fate table).

- [ ] **Step 1: Failing test** `show_and_meta_parity`: for each fixture resource, production `get_meta` `managed_meta`/`open_meta` equals `readback::meta` reconstructed from `kb_properties` — **modulo the §7-died keys** (`temper-title`/`temper-slug`/`temper-id`/`temper-context` are absent by design; the test asserts the *surviving* key set matches, and that died keys are absent).
- [ ] **Step 2-3:** Run → FAIL; implement `readback::meta(pool, id)` grouping `kb_properties` back into a managed/open split (managed = the §7 workflow+provenance keys + `doc_type`; open = the rest). Document that died keys never reappear (CONFORM §7).
- [ ] **Step 4-6:** PASS; cache+check; commit `WS6 chunk3 §9: show + get_meta parity (properties→meta reconstruction)`.

### Task 14: body-reconstruction parity at the read surface

**Tag: CONFORM** — reuse `synthesis::parity::reconstruct_body` (Task 9) as the read-surface body assembly; assert string-equality vs production `get_content` for every fixture resource. (Overlaps the synthesis gate but exercises it as a *read*, closing §9's body floor.)

- [ ] **Step 1: Failing test** `body_read_parity`: `readback::body(pool, id)` == production `get_content(...).body` for all fixture resources.
- [ ] **Step 2-3:** Run → FAIL; implement `readback::body` delegating to the shared reconstruction over `temper_next` chunks.
- [ ] **Step 4-6:** PASS; cache+check; commit `WS6 chunk3 §9: body-reconstruction read parity`.

### Task 15: FTS search parity

**Tag: EXTEND** — §9 (*"FTS / unified search → Domain-A operational, rebuilt … against the new columns (title-only weight-A; doctype filters become property lookups)"*). Build a tsvector read over `temper_next` and assert top-N identical to production FTS for a fixed query set.

- [ ] **Step 1: Failing test** `fts_parity`: for ~3 fixed query strings, production FTS (`unified_search` FTS-only weighting, search_service.rs:54-103) and `readback::fts_search` return the same ordered `origin_uri` list. Build the new tsvector from `title` (weight A) + reconstructed body (weight B) over `temper_next` (CONFORM §9 weighting; doctype filter via the `doc_type` property).
- [ ] **Step 2-3:** Run → FAIL; implement `readback::fts_search` (a `to_tsvector`/`plainto_tsquery` ranking query over a CTE assembling title+body per resource). Note this rebuilds, not ports, the index — §9 says FTS is rebuilt against new columns.
- [ ] **Step 4-6:** PASS; cache+check; commit `WS6 chunk3 §9: FTS search parity`.

### Task 16: vector search parity

**Tag: EXTEND** — §9 vector search over `temper_next.kb_chunks.embedding` (carried verbatim, Task 6). Runtime `query_as` for the `::vector` cast (CONFORM to the established search_service exception).

- [ ] **Step 1: Failing test** `vector_parity`: for a fixed query embedding, production vector search (cosine over `public.kb_chunks WHERE is_current`) and `readback::vector_search` return the same ordered `origin_uri` list (embeddings are identical bytes — carried verbatim — so rankings must match exactly).
- [ ] **Step 2-3:** Run → FAIL; implement `readback::vector_search` (runtime `query_as`, `embedding <=> $1::vector` ordering, joined to resource via `kb_chunks.resource_id`).
- [ ] **Step 4-6:** PASS; cache+check; commit `WS6 chunk3 §9: vector search parity`.

### Task 17: graph neighbors parity

**Tag: EXTEND** — §9 (graph traversal/neighbors → kernel). Port `aggregator_subgraph`/neighbors (graph_service.rs:107-215) over `temper_next.kb_edges` with `is_folded` gating.

- [ ] **Step 1: Failing test** `graph_parity`: for a seed resource, production neighbors (1-hop over `public.kb_resource_edges WHERE NOT is_folded`) and `readback::neighbors` return the same set of `(neighbor_origin_uri, edge_kind, polarity, label)` tuples — including the minted `parent_of` edge, and **excluding** the folded edge.
- [ ] **Step 2-3:** Run → FAIL; implement `readback::neighbors(pool, id, depth)` over `temper_next.kb_edges` (both endpoints `kb_resources`, `NOT is_folded`).
- [ ] **Step 4-6:** PASS; cache+check; commit `WS6 chunk3 §9: graph neighbors parity`.

### Task 18: Branch-level verification + WS6 status update

**Tag: verification** — per `feedback_workspace_tests_at_pr_only` (full workspace tests at PR-prep) and `feedback_long_verification_runs_inline`.

- [ ] **Step 1:** `cargo make prepare-next && cargo make check` (offline cache honest probe).
- [ ] **Step 2:** `cargo nextest run -p temper-next --features artifact-tests` (full serialized write-path + parity suite) — confirm green via exit code, not the summary line (`feedback_nextest_summary_lies`).
- [ ] **Step 3:** `cargo make docker-up && sqlx migrate run` on a clean dev DB — confirm the install migration applies and `\dn` shows `public` + `temper_next`, `public` table count unchanged.
- [ ] **Step 4:** Update the spec's Connections line and the `substrate-kernel-to-cognitive-map` goal WS6 status (chunks 2+3 landed). Update `MEMORY.md` per the project memory.
- [ ] **Step 5: Commit** `WS6 chunks 2+3: status update + spec connection`.

---

## Self-Review (run against the spec with fresh eyes)

**Spec coverage (chunks 2+3):** §0 per-resource sequence → Tasks 6/7/8/10; active-only → Task 4; §1 entities/anchor → Tasks 5/6; §2 contexts+homes → Tasks 5/6; §4 edges incl. folded pair → Task 8; §7 key fates incl. temper-goal edge + temper-type column-wins → Tasks 7/8; §8 single block + verbatim carry + parity gate → Tasks 3/6/9; §9 full read floor (list/show/meta/body/FTS/vector/graph) → Tasks 12-17; §D additive migration + explicit invocation + read-only chunk 3 → Tasks 1/2/4. **Not in scope** (correctly deferred): ledger archive, surface repoint, the flip, crate extraction, `kb_team_contexts` DDL (already in the artifact).

**Type consistency:** `ResourceCreated`/`RelationshipAsserted`/`property` payload shapes are the artifact's `payloads.rs` structs (G3/G4/G10); the sidecar gains `header_path`/`heading_depth` consistently in Task 3 (SQL) and Task 6 (Rust producer). `reconstruct_body` is defined once (Task 9) and reused (Task 14). `key_fate` is one enum in one place (Task 7), consumed by Task 8 for the `Edge` branch.

**Open verification gaps flagged inline for the implementer (GD-2/GD-5), not fabricated:** (a) distinct originator vs owner in `_project_resource_created` (Task 6); (b) the edge-home context choice for resource↔resource edges (Task 8). Both carry "verify against disk / escalate if ambiguous" notes rather than a guessed resolution.

---

## Execution Handoff

**Plan complete and saved to `docs/superpowers/plans/2026-06-13-ws6-chunk2-3-synthesis-parity.md`. Two execution options:**

**1. Subagent-Driven (recommended)** — dispatch a fresh subagent per task, review between tasks, fast iteration. Per `hybrid-execution` Variant B: orchestrator reviews each commit; the two flagged verification gaps (Task 6 originator, Task 8 edge-home) get controller grounding before those tasks dispatch.

**2. Inline Execution** — execute tasks in this session via `superpowers:executing-plans`, batch execution with checkpoints.

**Which approach?**
