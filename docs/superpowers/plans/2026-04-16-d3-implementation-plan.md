# Plan: D3 — temper index + graph index + LLM integration

**Date:** 2026-04-16
**Context:** temper
**Goal:** llm-wiki
**Design:** `docs/superpowers/specs/2026-04-16-d3-temper-index-and-graph-index-implementation.md`
**Status:** Implementation plan

---

## Overview

D3 has 8 sub-tasks (D3a–D3h). Tasks 1–6 are sequential (each depends on the previous). Task D3g (managed meta fields) can be done in parallel with any earlier task since the managed meta fields only affect how Concept files are written — the core pipeline doesn't depend on them. D3h is integration testing at the end.

**Total estimated scope:** Large. Recommend parallel subagents for independent tasks once prerequisites are met.

---

## Step 1: D3a — HNSW feature + temper index command

### What

- Add `hnsw = ["dep:tantivy", "dep:hnsw_rs"]` feature to `crates/temper-ingest/Cargo.toml`
- Add `default = ["embed", "extract", "hnsw"]` to `crates/temper-cli/Cargo.toml` (remove explicit feature listing, rely on workspace default)
- Add `indicatif` to `temper-cli` deps (already there but not wired — verify)
- Create `crates/temper-cli/src/actions/index.rs` — `IndexParams`, `IndexReport`, `run(params) -> Result<IndexReport>`
- Create `crates/temper-cli/src/commands/index.rs` — `Index` command variant
- Wire into `crates/temper-cli/src/cli.rs` Commands enum (add `Index` variant)
- Wire into `crates/temper-cli/src/main.rs` run match arm
- Create `crates/temper-cli/src/actions/index_build.rs` — HNSW index building + sidecar manifest
- Create `crates/temper-llm/src/types.rs` — shared types: `SeedPhrase`, `Cluster`, `ConceptProposal` (used across actions)

### Files to create
- `crates/temper-cli/src/actions/index.rs`
- `crates/temper-cli/src/commands/index.rs`
- `crates/temper-cli/src/actions/index_build.rs`
- `crates/temper-llm/src/types.rs`

### Files to modify
- `crates/temper-ingest/Cargo.toml` — add `hnsw = ["dep:tantivy", "dep:hnsw_rs"]` feature
- `crates/temper-cli/Cargo.toml` — update default features
- `crates/temper-cli/src/cli.rs` — add `Index` to Commands enum
- `crates/temper-cli/src/main.rs` — wire up Index command
- `crates/temper-cli/src/commands/mod.rs` — add `pub mod index`
- `crates/temper-cli/src/actions/mod.rs` — add `pub mod index_build`, `pub mod index`

### Verify
- `cargo make check` passes
- `cargo nextest run -p temper-cli` passes
- `temper index --help` shows correctly

---

## Step 2: D3b — [graph_index] config section

### What

- Add `GraphIndexConfig` struct to `crates/temper-core/src/types/config.rs`
- Add `graph_index: GraphIndexConfig` field to `TemperConfig`
- Set default in `TemperConfig::default()`
- Add CLI flags to `Index` and `Graph` commands that mirror all config fields

### Files to modify
- `crates/temper-core/src/types/config.rs` — add `GraphIndexConfig`, update `TemperConfig`

### Verify
- `cargo make check -p temper-core` passes

---

## Step 3: D3c — TF-IDF seed extraction

### What

Create `crates/temper-cli/src/actions/graph_index/seeds.rs`:
- `extract_seeds(config, context_filter, params) -> Vec<SeedPhrase>`
- Uses `tantivy` for tokenization, Snowball stemmer, stopwords, TF-IDF scoring
- Cross-document frequency filter (phrase must appear in ≥`seed_min_doc_frequency` docs)
- Returns top N phrases by aggregate TF-IDF score

Uses existing vault-walking pattern from `graph_build.rs` (same `discover_vault` approach).

### Files to create
- `crates/temper-cli/src/actions/graph_index/mod.rs` — module entry point
- `crates/temper-cli/src/actions/graph_index/seeds.rs` — seed extraction

### Files to modify
- `crates/temper-cli/src/actions/mod.rs` — add `pub mod graph_index`

### Verify
- Unit test: known corpus → expected seeds (use a small fixture directory)

---

## Step 4: D3d — Cluster formation with HNSW

### What

Create `crates/temper-cli/src/actions/graph_index/cluster.rs`:
- `form_clusters(seeds, index, params) -> Vec<Cluster>`
- For each seed: embed phrase → HNSW nearest-neighbor search → group by doc → apply threshold/cap

Reads `.temper/index.json` (sidecar manifest) to get `doc_embedding` per file. The HNSW search returns chunk-level IDs; these map back to documents via the sidecar.

### Files to create
- `crates/temper-cli/src/actions/graph_index/cluster.rs`

### Verify
- Unit test: mock HNSW responses → expected cluster structure

---

## Step 5: D3e — LLM judgment

### What

Create `crates/temper-cli/src/actions/graph_index/judgment.rs`:
- `judge_clusters(clusters, provider, run_id, params) -> Vec<ConceptProposal>`
- Builds prompt per cluster (seed phrase + member summaries)
- Calls `Agent::run` with `max_turns: 1`, no tools, `ConceptProposal` response format
- Logs failures to `.temper/graph-index-errors-{run_id}.log`

Uses `crates/temper-llm::Agent` already scaffolded in D1/D2.

### Files to create
- `crates/temper-cli/src/actions/graph_index/judgment.rs`

### Verify
- Integration test with `MockLlmProvider` returning deterministic `ConceptProposal` → verify parse

---

## Step 6: D3f — Materialization

### What

Create `crates/temper-cli/src/actions/graph_index/materialize.rs`:
- `materialize_concepts(proposals, config, run_id, dry_run) -> MaterializeReport`
- Per-concept: generate `temper-provisional-id` (UUIDv7), `temper-llm-run` (same for all)
- Write Concept file at `{vault}/{owner}/{context}/concept/{slug}.md`
- For each member: read frontmatter, add `relates-to` to `open_meta`, serialize back
- Transactional per-concept: if any member write fails, delete the Concept file

Depends on D3g (managed meta fields) being done first — `ManagedMeta` needs the `provenance`, `llm_model`, `llm_run` fields to exist before we can set them.

### Files to create
- `crates/temper-cli/src/actions/graph_index/materialize.rs`

### Verify
- Integration test with mock provider → verify Concept files written with correct frontmatter, member edges added

---

## Step 7: D3g — Managed meta fields + schema

### What

Three coordinated changes:

**a) `crates/temper-core/schemas/base.schema.json`**
Add four optional properties:
- `temper-provisional-id` (string, UUIDv7 pattern)
- `temper-provenance` (enum: "llm-discovered" | "user-created")
- `temper-llm-model` (string)
- `temper-llm-run` (string)

**b) `crates/temper-core/src/frontmatter/fields.rs`**
Add to `KNOWN_TEMPER_FIELDS`:
- `temper-provenance`
- `temper-llm-model`
- `temper-llm-run`
- Also add `temper-provisional-id` (it's already implicitly covered but should be explicit)

**c) `crates/temper-core/src/types/managed_meta.rs`**
Add typed fields:
```rust
#[serde(rename = "temper-provenance", skip_serializing_if = "Option::is_none")]
pub provenance: Option<String>,

#[serde(rename = "temper-llm-model", skip_serializing_if = "Option::is_none")]
pub llm_model: Option<String>,

#[serde(rename = "temper-llm-run", skip_serializing_if = "Option::is_none")]
pub llm_run: Option<String>,
```

Note: Do NOT add to `SYSTEM_MANAGED_FIELDS` — these are informational, user-editable.

### Files to modify
- `crates/temper-core/schemas/base.schema.json`
- `crates/temper-core/src/frontmatter/fields.rs`
- `crates/temper-core/src/types/managed_meta.rs`

### Verify
- `cargo make check -p temper-core` passes
- Round-trip test: serialize ManagedMeta with new fields → deserialize → verify fields present

---

## Step 8: D3h — Integration test + smoke test

### What

**Unit tests** (in `crates/temper-cli/tests/`):
1. `temper index` on a fixture vault → verify `.temper/index.bin` exists and is queryable
2. `temper graph index --dry-run` with mock provider → verify report structure, no vault writes
3. `temper graph index` (non-dry-run) with mock provider → verify Concept files created, member edges written, idempotent on second run

**Smoke test binary** (from D2, already at `crates/temper-llm/src/bin/smoke_test.rs`):
- Extend to cover `temper index` (if ollama running) — don't fail if no ollama

### Verify
- `cargo nextest run -p temper-cli` — all new tests pass
- `cargo make check` — full workspace clean

---

## Dependencies Summary

```
D3a (hnsw feature + temper index)
  └─ D3b (graph_index config)
        └─ D3c (TF-IDF seeds)
              └─ D3d (cluster formation)
                    └─ D3e (LLM judgment)
                          └─ D3f (materialization)
                                └─ D3h (tests)

D3g (managed meta) — can run in parallel with D3a–D3d; needed before D3f
```

---

## Parallel Execution

Recommend:
- **Subagent 1:** D3a → D3b → D3c → D3d → D3e → D3f (sequential chain)
- **Subagent 2:** D3g (independent, can start immediately)
- **Subagent 3:** D3h after both chains complete

Alternatively, given D3f depends on D3g and D3e/f are the most complex, could do:
- **Subagent 1:** D3a → D3b → D3c → D3d (index build + seed extraction)
- **Subagent 2:** D3g (managed meta fields, independent)
- **Subagent 3:** D3e → D3f (LLM judgment + materialization, depends on D3d and D3g)
- **Main session:** D3h (integration tests)

---

## Critical Files to Reference

| Pattern | File |
|---|---|
| Vault walking | `crates/temper-cli/src/actions/graph_build.rs` (`discover_vault`, `ENTITY_DOC_TYPES`, `DiscoveredFile`) |
| Config section pattern | `crates/temper-core/src/types/config.rs` (`LlmConfig` lines 173-202, `TemperConfig` lines 256-274) |
| Managed meta registration | `crates/temper-core/src/frontmatter/fields.rs` (`KNOWN_TEMPER_FIELDS` line 29) |
| ManagedMeta struct | `crates/temper-core/src/types/managed_meta.rs` |
| JSON schema pattern | `crates/temper-core/schemas/base.schema.json` |
| Agent harness | `crates/temper-llm/src/agent.rs` (`Agent::run` with `max_turns: 1`, no tools) |
| CLI command pattern | `crates/temper-cli/src/commands/graph.rs` |
| CLI enum wiring | `crates/temper-cli/src/cli.rs` |
| indicatifs wiring | `crates/temper-cli/src/actions/progress.rs` |

---

## Success Criteria

1. `cargo make check` passes after all changes
2. `cargo nextest run -p temper-cli` passes
3. `temper index` on the temper project's vault produces `.temper/index.bin` + `.temper/index.json`
4. `temper graph index --dry-run` produces a structured report with seeds, clusters, proposals
5. `temper graph index` (non-dry-run) creates Concept resources that pass `temper doctor`
6. Running `temper graph index` twice on stable vault produces no new concepts