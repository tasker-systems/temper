# Cloud-only vault — Chunk 6: delete local HNSW + graph-build; rework search to cloud-only

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Delete the local HNSW indexing pipeline (`actions/{index,index_build,graph_build}` and `actions/graph_index/`), the `temper graph` / `temper index` CLI subcommands (`commands/{graph,index}.rs` + their `Commands` enum variants + `GraphAction`), the three test files whose entire scope is the deleted machinery (`graph_build_test.rs`, `graph_index_integration.rs`, `graph_build_e2e_test.rs`), slim `actions/search.rs` to drop the local-manifest enrichment (keeping the query-embedding + cloud-dispatch + display helpers, which are orthogonal to HNSW), and rewrite `commands/search_cmd.rs` to dispatch through `temper-client` with no `manifest_io` dependency.

**Architecture:** Leaves inward, the shape Chunks 4 + 5 used. (1) Delete the three test files (no surface dependencies on them). (2) Delete the `Graph`/`Index` CLI surface (`commands/{graph,index}.rs` + their `Commands` enum variants + `GraphAction` enum + `main.rs` dispatch arms) — this orphans the action modules they call. (3) Delete the now-orphan action modules (`actions/{graph_build, graph_index/, index, index_build}`). (4) Slim `actions/search.rs` (drop `Manifest`/`ResourceId` imports + `enrich_results` + `EnrichedSearchResult.local`/`vault_path` fields + 3 affected tests) and rewrite `commands/search_cmd.rs` (drop `manifest_io::load_manifest` + `enrich_results` calls; format `UnifiedSearchResultRow` directly) in one atomic commit. (5) Sweep pub-orphans surfaced by `cargo make check`. (6) Run the four verification tiers + consolidated review.

**Tech Stack:** Rust 2024 edition, cargo-make, cargo-nextest, sqlx (no SQL changes in this chunk).

---

## Plan gate — resolution

Applied `feedback_plan_gate_audit_both_ends` from Chunk 5's lesson: greped BOTH type symbols AND module function names.

### Type-symbol audit (`manifest_io|Manifest\b`)

16 files surface. Classification:

| File | Verdict for Chunk 6 |
|---|---|
| `crates/temper-cli/src/actions/index_build.rs` | **DELETE** (Task 4) |
| `crates/temper-cli/src/actions/graph_index/cluster.rs` | **DELETE** (Task 4) |
| `crates/temper-cli/src/actions/search.rs` | **SLIM** — drop `use temper_core::types::{Manifest, ResourceId};` + `enrich_results` (Task 5) |
| `crates/temper-cli/src/commands/search_cmd.rs` | **REWRITE** — drop `manifest_io::load_manifest` call (Task 5) |
| `tests/e2e/tests/graph_build_e2e_test.rs` | **DELETE** (Task 2) |
| `crates/temper-cli/src/actions/doctor.rs` | Deferred to Chunk 7 |
| `crates/temper-cli/src/actions/doctor_fix.rs` | Deferred to Chunk 7 |
| `crates/temper-cli/src/commands/doctor.rs` | Deferred to Chunk 7 |
| `crates/temper-cli/src/actions/ingest.rs` | Deferred to Chunk 7 |
| `crates/temper-cli/src/actions/sync.rs` | Deferred (last 2 blockers: graph_build_e2e_test.rs going in this chunk + `doctor.rs::normalize_all_entries` in Chunk 7) |
| `crates/temper-cli/src/manifest_io.rs` | Deferred (still has 4 consumers post-Chunk-6) |
| `crates/temper-cli/src/lib.rs` | Survives (declares `pub mod manifest_io;`) |
| `crates/temper-core/src/types/manifest.rs` | Deferred to Chunk 7 |
| `crates/temper-core/src/types/mod.rs` | Survives (`pub use` lines for deferred types) |
| `crates/temper-core/src/types/sync.rs` | Deferred to Chunk 7 |
| `tests/e2e/tests/meta_test.rs` | Comment-only false positive — Chunk 7 cleanup |

### Module-path + function-name audit (`actions::{search,index_build,graph_index,graph_build}|commands::search_cmd`)

8 files surface:

| File | Verdict |
|---|---|
| `crates/temper-cli/src/actions/graph_index/materialize.rs` | DELETE (part of graph_index/ dir) |
| `crates/temper-cli/src/actions/index.rs` | **DELETE** (Task 4) — types-orchestrator paired with index_build |
| `crates/temper-cli/src/commands/graph.rs` | **DELETE** (Task 3) — `GraphAction` dispatch |
| `crates/temper-cli/src/commands/search_cmd.rs` | REWRITE (Task 5) |
| `crates/temper-cli/src/main.rs` | MODIFY (Task 3 — drop `Commands::{Graph,Index}` dispatch arms; update `Search` arm in Task 5 to drop `text_only`'s consumer if pruned) |
| `crates/temper-cli/tests/graph_build_test.rs` | **DELETE** (Task 2) |
| `crates/temper-cli/tests/graph_index_integration.rs` | **DELETE** (Task 2) |
| `tests/e2e/tests/graph_build_e2e_test.rs` | **DELETE** (Task 2) |

**Surprises vs the task brief's enumeration** (resolved here, not deferred):
- `actions/index.rs` exists alongside `index_build.rs` — types/orchestrator pair. Both die together.
- `commands/index.rs` exists (separate from `actions/index.rs`) and houses the `Commands::Index` dispatch. Dies with `commands/graph.rs` in Task 3.
- Two CLI integration tests (`graph_build_test.rs`, `graph_index_integration.rs`) were not in the brief's list — both whole-file deletes (their entire scope is the deleted machinery).
- `GraphAction` enum (in `cli.rs`) was not enumerated separately — dies in Task 3 with the `Commands::Graph` variant.

### Search semantics — query embedding is orthogonal to HNSW

The brief said "no local embedding" for the search rewrite. **Refined per user feedback (2026-05-25):** local *query* embedding (`temper_ingest::embed::embed_text` over the query string) is orthogonal to the local HNSW index. Deleting the local HNSW pipeline does not require deleting query-embedding-for-cloud-dispatch.

Decision: **keep `embed_query` and the cloud-dispatch path in slim `actions/search.rs`**; drop only the local-manifest enrichment (`EnrichedSearchResult.local`/`vault_path` fields + `enrich_results` fn + 3 affected unit tests). The CLI continues to call `temper_ingest::embed::embed_text` to produce the query embedding it sends to `/api/search`. `--text-only` flag stays meaningful (sends `embedding: None`, gets FTS-only on the cloud).

**Acceptance criterion adjustment vs the task brief:**

Brief said: *"`crates/temper-cli/src/commands/search_cmd.rs` dispatches through `temper-client::resources().search(...)`; no `manifest_io` dep; no `Manifest` ref; **no local embedding**."*

Plan resolves: cloud dispatch via `temper-client::TemperClient::search().search_with_params(&SearchParams)` (matches the *spirit* of "through temper-client"; `client.search()` returns a `SearchClient` whose `search_with_params` hits `/api/search`). No `manifest_io` dep ✓. No `Manifest` ref ✓. **Local embedding stays** — orthogonal to HNSW removal.

### Stacked-deferral progress after Chunk 6

| Symbol | Status post-Chunk-6 | Last remaining blocker |
|---|---|---|
| `tests/e2e/tests/graph_build_e2e_test.rs` | Gone | — |
| `actions/sync.rs` | Still deferred | `actions/doctor.rs::normalize_all_entries` (Chunk 7) |
| `manifest_io.rs` (consumer count: 7 → 4) | Still deferred | `actions/{doctor,doctor_fix,ingest}.rs`, `commands/doctor.rs` (Chunk 7) |
| `temper-core::types::{manifest,sync}` | Still deferred | Same as manifest_io |

---

## Cleanups bundled in this chunk

| Item | Why it travels with Chunk 6 |
|---|---|
| `commands::graph` CLI command + `Commands::Graph` variant + `GraphAction` enum + `main.rs` dispatch arm | `temper graph build` + `temper graph index` are the only surfaces that drove the deleted graph-build/graph-index pipelines |
| `commands::index` CLI command + `Commands::Index` variant + `main.rs` dispatch arm | `temper index` drove the deleted HNSW pipeline |
| `actions/index.rs`, `actions/index_build.rs`, `actions/graph_build.rs`, `actions/graph_index/` (full dir) | Now-orphan action modules after Task 3 |
| `actions/search.rs` slim: drop `EnrichedSearchResult.local` + `EnrichedSearchResult.vault_path`, `enrich_results`, `use temper_core::types::{Manifest, ResourceId}` import, 3 affected unit tests | Local-vault enrichment is dead in cloud-only mode |
| `commands/search_cmd.rs` rewrite: drop `manifest_io::load_manifest` call + `enrich_results` call + `crate::projection::warn_if_context_stale` reference (which exists for the local-projection cache, not the deleted machinery, so verify before touching) | Aligns search surface with cloud-only data path |
| 3 doomed test files | Each is structurally dependent on the deleted machinery |
| Pub-orphans surfaced by `cargo make check` after deletions | Symmetric-removal heuristic from Chunks 4 + 5 |

## Items explicitly NOT in this chunk (deferred)

- `crates/temper-cli/src/manifest_io.rs` and `pub mod manifest_io;` in `lib.rs` — wait for Chunk 7's last consumer
- `crates/temper-core/src/types/manifest.rs`, `types/sync.rs`, and the `pub use` lines in `types/mod.rs` — same
- `actions/sync.rs` — still has 1 blocker (`doctor.rs::normalize_all_entries`) post-Chunk-6
- `actions/doctor.rs`, `actions/doctor_fix.rs`, `commands/doctor.rs`, `actions/ingest.rs` — Chunk 7
- `tests/e2e/tests/meta_test.rs` — comment-only ref to "Manifest" gets cleaned in Chunk 7 (no edit here)
- `temper-ingest` crate itself — server-side embed pipeline stays for the cloud ingest workflow

## Branch

`jct/cloud-only-vault-pr-b` — **do not branch**. Chunks 3–8 accumulate on the same branch; the PR opens after Chunk 8.

## Execution discipline (carry forward from Chunks 3 + 4 + 5)

- Subagent-driven execution, fresh sonnet implementer per task; opus only for the final consolidated review (per `feedback_subagent_review_cadence`).
- Each task ends with `cargo make check` green and a commit, so the branch stays bisectable.
- Per-task verification is **tightened**: `cargo make check` + targeted `-p` nextest only. Full workspace + e2e tiers run once in the final consolidated task (Chunk 5 ran 72.83s end-to-end via `cargo make test-all` on warm cache).
- **Pub-orphan sweep audit (symmetric removal):** when deleting a reader, audit the writer side; when deleting a writer, audit the consumer side. Chunk 5's Task 9 is the reference.
- **Cargo output redirection:** always `> /tmp/foo.log 2>&1`. Never `2>&1 | tail` (silently produces 0-byte files under the harness, per `feedback_cargo_output_redirection`).
- **Plan committed early** (per Chunk 4 carry-forward lesson #7): Task 0 below commits this plan before Task 1 starts, so each subsequent commit's context references it.
- **`access_gate_test` parallel e2e flake** is environmental — if it fails in Task 7's e2e tier, re-run serial.

---

## Task 0: Commit this plan

Land the plan file before any code change so subsequent commits reference it. No code edit; one commit.

- [ ] **Step 1: Commit the plan file**

```bash
git add docs/superpowers/plans/2026-05-25-cloud-only-vault-chunk6-delete-hnsw-and-rework-search.md
git commit -m "cloud-only(ch6): record the chunk 6 implementation plan"
```

---

## Task 1: Test triage

Inventory every test file whose code path is touched by this chunk's deletions. Produce explicit delete/keep/repoint verdicts in an empty commit so the analysis is bisectable.

- [ ] **Step 1: Inventory affected test files via grep**

```bash
# Test files referencing deleting symbols (whole workspace)
rg -l 'actions::(search|index_build|index\b|graph_build|graph_index)|commands::(graph\b|index\b|search_cmd)|GraphAction|GraphBuildParams|GraphIndexParams|IndexParams\b|IndexReport\b|enrich_results' \
  --type rust 2>&1 > /tmp/ch6_test_triage.log

# Also grep for any test that constructs `Manifest` (signals manifest-dependent tests)
rg -l 'use temper_core::types::(Manifest|ManifestEntry)|temper_core::types::manifest' --type rust 2>&1 >> /tmp/ch6_test_triage.log

cat /tmp/ch6_test_triage.log
```

- [ ] **Step 2: Produce a verdict table**

For each file the grep surfaces, decide:
- **Delete with parent** — file lives inside a deleted module; dies in its parent's task
- **Delete whole file** — entire test file's scope is the deleted machinery
- **Repoint** — file references a deleting symbol but the underlying test still has value; update to cloud-mode equivalent
- **Keep (drop affected tests only)** — file is mostly fine, but specific tests inside need removal (e.g. `actions/search.rs`'s manifest-dependent inline tests)
- **Defer** — references a deferred symbol (lives until Chunk 7)

**Expected verdicts (verify with the grep above; this is the working hypothesis):**

| File | Verdict | Lands in |
|---|---|---|
| `tests/e2e/tests/graph_build_e2e_test.rs` (~400 lines) | Delete whole file | Task 2 |
| `crates/temper-cli/tests/graph_build_test.rs` | Delete whole file | Task 2 |
| `crates/temper-cli/tests/graph_index_integration.rs` (`#![cfg(feature = "test-embed")]`) | Delete whole file | Task 2 |
| `tests/e2e/tests/meta_test.rs:221` (comment-only) | Keep (false positive; Chunk 7 cleanup) | — |
| `crates/temper-cli/src/actions/search.rs` (inline `tests` mod) | Repoint — delete 3 affected tests (`test_enrich_*`, `test_format_text_output`, `test_format_text_no_local`, `test_enriched_json_shape`); keep `test_truncate_*` (orthogonal) | Task 5 |
| `crates/temper-cli/src/actions/graph_build.rs` (any inline tests) | Delete with parent | Task 4 |
| `crates/temper-cli/src/actions/graph_index/*.rs` (any inline tests) | Delete with parent | Task 4 |
| `crates/temper-cli/src/actions/index_build.rs` (any inline tests) | Delete with parent | Task 4 |
| `crates/temper-cli/src/actions/index.rs` (any inline tests) | Delete with parent | Task 4 |
| `crates/temper-cli/src/commands/graph.rs` (any inline tests) | Delete with parent | Task 3 |
| `crates/temper-cli/src/commands/index.rs` (any inline tests) | Delete with parent | Task 3 |
| `crates/temper-cli/src/commands/search_cmd.rs` (no inline tests today) | Rewrite | Task 5 |
| `crates/temper-cli/src/actions/doctor*.rs`, `actions/ingest.rs`, `actions/sync.rs`, `commands/doctor.rs`, `manifest_io.rs`, `temper-core::types::manifest`/`sync` | Defer to Chunk 7 | — |

- [ ] **Step 3: Commit the inventory (empty)**

```bash
git commit --allow-empty -m "$(cat <<'EOF'
cloud-only(ch6): test-triage inventory for chunk 6

Whole-file test deletions (Task 2):
  - tests/e2e/tests/graph_build_e2e_test.rs
  - crates/temper-cli/tests/graph_build_test.rs
  - crates/temper-cli/tests/graph_index_integration.rs

Inline-test deletions (lands with parent):
  - crates/temper-cli/src/actions/search.rs (4 manifest-dependent tests:
    test_enrich_marks_local_resources, test_enrich_preserves_kb_uri,
    test_enrich_empty_inputs, test_format_text_output, test_format_text_no_local,
    test_enriched_json_shape — Task 5)
  - crates/temper-cli/src/actions/{graph_build,graph_index/*,index,index_build}.rs
    inline tests if any (Task 4)
  - crates/temper-cli/src/commands/{graph,index}.rs inline tests if any (Task 3)

Keep / false positives:
  - tests/e2e/tests/meta_test.rs:221 — comment-only ref
  - actions/search.rs::test_truncate_* (orthogonal — text utility)

Deferred to Chunk 7:
  - All tests referencing actions/doctor*, actions/sync, actions/ingest,
    commands/doctor, manifest_io, temper-core::types::{manifest,sync}
EOF
)"
```

---

## Task 2: Delete three doomed test files

Whole-file deletions for tests whose entire scope is the deleted HNSW/graph machinery. These don't impact temper-cli's build today (the e2e test imports `actions::sync` symbols that survive; the two integration tests depend on action modules that die in Task 4 but are independent of the surface deletion in Task 3).

**Files:**
- Delete: `tests/e2e/tests/graph_build_e2e_test.rs`
- Delete: `crates/temper-cli/tests/graph_build_test.rs`
- Delete: `crates/temper-cli/tests/graph_index_integration.rs`

- [ ] **Step 1: Confirm no other test imports these files as modules**

```bash
rg -n 'mod graph_build_e2e_test|mod graph_build_test|mod graph_index_integration|graph_build_e2e_test::|graph_build_test::|graph_index_integration::' \
  tests/e2e/ crates/temper-cli/tests/ 2>&1
```

Expected: zero hits (these test files are independent siblings, not module roots).

- [ ] **Step 2: Delete the files**

```bash
git rm tests/e2e/tests/graph_build_e2e_test.rs \
       crates/temper-cli/tests/graph_build_test.rs \
       crates/temper-cli/tests/graph_index_integration.rs
```

- [ ] **Step 3: Run `cargo make check`**

```bash
cargo make check > /tmp/ch6_task2_check.log 2>&1; tail -30 /tmp/ch6_task2_check.log
```

Expected: 0 errors. (The integration tests are not part of the default workspace test path that `cargo make check` exercises, but the check command also runs clippy across all targets so any compile-time staleness will surface here.)

- [ ] **Step 4: Run targeted nextest**

```bash
cargo nextest run -p temper-cli > /tmp/ch6_task2_nextest_cli.log 2>&1; tail -30 /tmp/ch6_task2_nextest_cli.log
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db > /tmp/ch6_task2_nextest_e2e.log 2>&1; tail -30 /tmp/ch6_task2_nextest_e2e.log
```

Expected: all surviving tests pass. None of the deleted test names should appear. (Skip `test-embed` per the tightened-verification discipline.)

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "cloud-only(ch6): delete graph-build/index e2e and cli integration tests"
```

---

## Task 3: Delete `commands/{graph,index}` surface + CLI enum variants + dispatch arms

Remove the `Graph` and `Index` CLI surfaces entirely: the two command modules, their `Commands` enum variants, the `GraphAction` enum, the module declarations in `commands/mod.rs`, and the dispatch arms in `main.rs`. After this task, the action modules they call (`actions::graph_build`, `actions::graph_index`, `actions::index`, `actions::index_build`) become unreachable from any surface — Task 4 then deletes them.

**Files:**
- Delete: `crates/temper-cli/src/commands/graph.rs`
- Delete: `crates/temper-cli/src/commands/index.rs`
- Modify: `crates/temper-cli/src/commands/mod.rs` (remove `pub mod graph;` and `pub mod index;`)
- Modify: `crates/temper-cli/src/cli.rs` (remove `Commands::Graph { action: GraphAction }` variant; remove `Commands::Index { context, full }` variant; remove `pub enum GraphAction { Build {...}, Index {...} }` definition entirely)
- Modify: `crates/temper-cli/src/main.rs` (remove `Commands::Graph { action } => ...` and `Commands::Index { context, full } => ...` arms)

- [ ] **Step 1: Verify the surface footprint**

```bash
rg -n 'commands::(graph\b|index\b)|Commands::(Graph|Index)|GraphAction|graph::run|commands::index::run' \
  --type rust 2>&1
```

Expected hits — these are the files modified by this task:
- `crates/temper-cli/src/cli.rs` — enum + variant defs
- `crates/temper-cli/src/main.rs` — dispatch arms
- `crates/temper-cli/src/commands/mod.rs` — module decls
- `crates/temper-cli/src/commands/graph.rs` — module itself (deleted)
- `crates/temper-cli/src/commands/index.rs` — module itself (deleted)

If any other file constructs `Commands::Graph`/`Commands::Index` or calls `commands::{graph,index}::run`, STOP and report.

- [ ] **Step 2: Delete the two command modules**

```bash
git rm crates/temper-cli/src/commands/graph.rs \
       crates/temper-cli/src/commands/index.rs
```

- [ ] **Step 3: Remove module declarations from `commands/mod.rs`**

In `crates/temper-cli/src/commands/mod.rs`, delete:
- The line `pub mod graph;`
- The line `pub mod index;`

Read the current `mod.rs` first to confirm the line numbers — declarations are alphabetical (today: `graph` line ~9, `index` line ~10).

- [ ] **Step 4: Remove the `Commands::Graph` and `Commands::Index` variants from `cli.rs`**

In `crates/temper-cli/src/cli.rs`:
- Delete the `Commands::Graph { ... }` variant block (today around lines 171-175, including the `/// Build, inspect, or manage the knowledge graph from vault frontmatter` docstring)
- Delete the `Commands::Index { ... }` variant block (today around lines 177-184, including the `/// Build an HNSW vector index over the vault` docstring)
- Verify `Commands::Pull { context }` (above `Graph`) and the closing `}` (or other surrounding variants) stay intact

Then delete the `pub enum GraphAction { Build {...}, Index {...} }` enum definition entirely (today around lines 486-511). Verify the `#[derive(Subcommand)]` line above is for `GraphAction` and not a different enum before deleting.

- [ ] **Step 5: Remove the dispatch arms from `main.rs`**

In `crates/temper-cli/src/main.rs`, locate the `match` block over `cli.command` (around lines 360-400). Delete:

```rust
Commands::Graph { action } => {
    let config = temper_cli::config::load(cli.vault.as_deref())?;
    temper_cli::commands::graph::run(&config, action)
}
Commands::Index { context, full } => {
    let config = temper_cli::config::load(cli.vault.as_deref())?;
    temper_cli::commands::index::run(&config, context.as_deref(), full)
}
```

Leave the `Commands::Search { ... } => commands::search_cmd::run(...)` arm above intact (Task 5 modifies it).

- [ ] **Step 6: Run `cargo make check`**

```bash
cargo make check > /tmp/ch6_task3_check.log 2>&1; tail -60 /tmp/ch6_task3_check.log
```

Expected: 0 errors. **A wave of dead-code warnings is likely for `actions/{graph_build, graph_index, index, index_build}` and any of their internal helpers** — those modules are now orphaned. Task 4 deletes them.

If real errors appear (especially `unresolved import` from any file other than the four expected modifications), STOP — there's a missed consumer. Grep:
```bash
rg 'use crate::commands::graph|use crate::commands::index\b' --type rust
```
Should print nothing.

- [ ] **Step 7: Run targeted nextest**

```bash
cargo nextest run -p temper-cli > /tmp/ch6_task3_nextest.log 2>&1; tail -30 /tmp/ch6_task3_nextest.log
```

Expected: all pass.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "cloud-only(ch6): delete commands/{graph,index} and CLI Graph/Index surface"
```

---

## Task 4: Delete `actions/{graph_build, graph_index/, index, index_build}` modules

After Task 3, these action modules have zero non-internal callers. Delete the files + directory + their declarations in `actions/mod.rs`.

**Files:**
- Delete: `crates/temper-cli/src/actions/graph_build.rs`
- Delete: `crates/temper-cli/src/actions/graph_index/` (whole directory: `mod.rs`, `cluster.rs`, `judgment.rs`, `materialize.rs`, `seeds.rs`)
- Delete: `crates/temper-cli/src/actions/index.rs`
- Delete: `crates/temper-cli/src/actions/index_build.rs`
- Modify: `crates/temper-cli/src/actions/mod.rs` (remove `pub mod graph_build;`, `pub mod graph_index;`, `pub mod index;`, `pub mod index_build;`)

- [ ] **Step 1: Confirm zero non-internal callers**

```bash
rg -n 'actions::(graph_build|graph_index|index\b|index_build)' --type rust 2>&1
```

Expected hits: only references inside the four modules themselves + `actions/mod.rs`'s declarations. If any external file (in `crates/temper-cli/src/{commands,actions}/*.rs` other than the four target modules) imports from these actions, STOP — Task 3 missed a consumer.

- [ ] **Step 2: Delete the files and directory**

```bash
git rm crates/temper-cli/src/actions/graph_build.rs \
       crates/temper-cli/src/actions/index.rs \
       crates/temper-cli/src/actions/index_build.rs
git rm -r crates/temper-cli/src/actions/graph_index/
```

- [ ] **Step 3: Remove module declarations from `actions/mod.rs`**

In `crates/temper-cli/src/actions/mod.rs`, delete the four lines:
- `pub mod graph_build;`
- `pub mod graph_index;`
- `pub mod index;`
- `pub mod index_build;`

Read the current file first to confirm exact ordering.

- [ ] **Step 4: Run `cargo make check`**

```bash
cargo make check > /tmp/ch6_task4_check.log 2>&1; tail -80 /tmp/ch6_task4_check.log
```

Expected: 0 errors. **A second wave of dead-code warnings may surface** — items that were only used by the deleted action modules (e.g. types in `temper-core` that only HNSW consumed). Task 6 (pub-orphan sweep) handles them.

If real errors appear, especially `unresolved import` from `actions/search.rs` (which uses `temper-ingest`), STOP — `actions/search.rs` SHOULD still compile because its `temper-ingest` dep is independent of the deleted modules. Grep:
```bash
rg 'use crate::actions::(graph_build|graph_index|index\b|index_build)' --type rust
```
Should print nothing.

- [ ] **Step 5: Run targeted nextest**

```bash
cargo nextest run -p temper-cli > /tmp/ch6_task4_nextest.log 2>&1; tail -30 /tmp/ch6_task4_nextest.log
```

Expected: all pass. The CLI test count drops (the inline tests inside the deleted action modules are gone).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "cloud-only(ch6): delete actions/{graph_build,graph_index,index,index_build}"
```

---

## Task 5: Slim `actions/search.rs` + rewrite `commands/search_cmd.rs` (atomic)

The two changes are tightly coupled — slimming `actions/search.rs` removes symbols that `commands/search_cmd.rs` still calls; rewriting `commands/search_cmd.rs` removes calls to symbols that the slim deletes. Land them in one commit so each intermediate state of the branch compiles.

**What changes:**
- `actions/search.rs`: drop `use temper_core::types::{Manifest, ResourceId};` import; drop `EnrichedSearchResult.local` and `EnrichedSearchResult.vault_path` fields; drop the `enrich_results` function entirely; drop 4 tests that depend on `Manifest` fixtures (`test_enrich_marks_local_resources`, `test_enrich_preserves_kb_uri`, `test_enrich_empty_inputs`, `test_format_text_output`, `test_format_text_no_local`, `test_enriched_json_shape` — total 6 tests need removal; the test_truncate_* tests stay).
- `commands/search_cmd.rs`: drop `use crate::manifest_io;`; drop the `manifest = crate::manifest_io::load_manifest(...)` call; drop the `crate::projection::warn_if_context_stale` call (it touches `state_dir` from `temper_dir` derived from a now-removed local manifest path — verify in Step 1); drop the `let enriched = search_actions::enrich_results(...)` call; format `search_actions::format_text(&results)` where `results: &[UnifiedSearchResultRow]` directly (no intermediate `EnrichedSearchResult`).

**Important nuance — `format_text` signature change:** `format_text` currently takes `&[EnrichedSearchResult]` and renders the `local` flag. The slim removes that flag. Two options:

- **Option A (recommended):** Change `format_text` to take `&[UnifiedSearchResultRow]` directly. Drop the `[local]` marker entirely (no local notion in cloud-only mode). Drop `EnrichedSearchResult` struct entirely too — its only fields beyond the row's were `local` + `vault_path`, both now dead.
- **Option B:** Keep `EnrichedSearchResult` as a thin re-shape over `UnifiedSearchResultRow` (no local fields) for JSON-output stability. Future-proofs the JSON shape if cloud results ever need re-wrapping.

**Choice: Option A.** YAGNI. `UnifiedSearchResultRow` already serializes to JSON, and it's the wire type the cloud returns. Re-wrapping adds no value. Per `feedback_no_premature_backward_compat`.

**Files:**
- Modify: `crates/temper-cli/src/actions/search.rs` (~282 LOC → ~120 LOC)
- Modify: `crates/temper-cli/src/commands/search_cmd.rs` (~75 LOC → ~50 LOC)
- Modify: `crates/temper-cli/src/main.rs` (only if Task 6 of cli-flag pruning is needed — see Step 1)

- [ ] **Step 1: Verify `commands/search_cmd.rs` consumers of removed symbols**

Read the full current `commands/search_cmd.rs` (75 lines). Confirm exact symbol references:

```bash
rg -n 'manifest_io|enrich_results|EnrichedSearchResult|warn_if_context_stale|projection::' crates/temper-cli/src/commands/search_cmd.rs
```

Note each call site — the rewrite must remove all of them.

**Specifically verify `projection::warn_if_context_stale`:**

```bash
rg -n 'pub (async )?fn warn_if_context_stale' crates/temper-cli/src/projection.rs
```

If this function exists and its sole consumer is `search_cmd.rs`, it becomes a pub-orphan after the rewrite — Task 6 will sweep it.

If it has other consumers (in `commands/show.rs`, `commands/list.rs`, etc.), leave it alone and just remove the call from `search_cmd.rs`.

- [ ] **Step 2: Rewrite `commands/search_cmd.rs`**

Replace the file's body (keep the module-level doc comment) with the cloud-only dispatch flow:

```rust
//! `temper search` — thin CLI wrapper over actions::search (cloud-only).

use crate::actions::{runtime, search as search_actions};
use crate::error::Result;
use crate::format::OutputFormat;
use uuid::Uuid;

#[expect(
    clippy::too_many_arguments,
    reason = "all args are CLI-derived primitives; bundling into a struct would mirror clap-generated fields with no semantic benefit"
)]
pub fn run(
    query: &str,
    context: Option<&str>,
    doc_type: Option<&str>,
    limit: Option<i64>,
    format: &str,
    text_only: bool,
    seed_ids: Vec<Uuid>,
    edge_types: Vec<String>,
    depth: Option<i32>,
    no_graph: bool,
) -> Result<()> {
    let fmt = OutputFormat::parse(format);

    let embedding = if text_only {
        None
    } else {
        Some(search_actions::embed_query(query)?)
    };

    let results = runtime::with_client(|client| {
        let params = search_actions::build_search_params(search_actions::CliSearchArgs {
            query,
            embedding: embedding.clone(),
            context,
            doc_type,
            limit,
            seed_ids: seed_ids.clone(),
            edge_types: edge_types.clone(),
            depth,
            no_graph,
        });
        Box::pin(async move { search_actions::search_api(client, params).await })
    })?;

    if results.is_empty() {
        if fmt == OutputFormat::Json {
            crate::output::plain("[]");
        } else {
            crate::output::warning("No results found.");
        }
        return Ok(());
    }

    if fmt == OutputFormat::Json {
        crate::output::plain(serde_json::to_string_pretty(&results)?);
    } else {
        for line in search_actions::format_text(&results) {
            crate::output::plain(line);
        }
    }

    Ok(())
}
```

Verify by re-reading the file that:
- No `manifest_io` references remain
- No `enrich_results` call remains
- No `crate::projection::` reference remains (if `warn_if_context_stale` was here)
- The `with_client` closure shape matches the surviving runtime helper (read `actions/runtime.rs::with_client` first if unsure)
- `runtime::require_device_id()` is NOT called here (was used to load the manifest; cloud dispatch via `with_client` handles auth itself)

- [ ] **Step 3: Slim `actions/search.rs`**

Read the current file (282 lines) and replace its body with:

```rust
//! Search business logic — query embedding, cloud API dispatch, formatting.
//!
//! All testable functions. The CLI command is a thin wrapper over these.
//! Cloud-only: no local index, no manifest enrichment.

use temper_core::types::api::{SearchParams, UnifiedSearchResultRow};

use crate::error::{Result, TemperError};

/// Embed query text locally via temper-ingest.
#[cfg(feature = "embed")]
pub fn embed_query(text: &str) -> Result<Vec<f32>> {
    temper_ingest::embed::embed_text(text)
        .map_err(|e| TemperError::Extraction(format!("embedding failed: {e}")))
}

#[cfg(not(feature = "embed"))]
pub fn embed_query(_text: &str) -> Result<Vec<f32>> {
    Err(TemperError::Config(
        "search requires the 'embed' feature — rebuild with --features embed".into(),
    ))
}

/// CLI search arguments — bundles domain params for `build_search_params`.
pub struct CliSearchArgs<'a> {
    pub query: &'a str,
    pub embedding: Option<Vec<f32>>,
    pub context: Option<&'a str>,
    pub doc_type: Option<&'a str>,
    pub limit: Option<i64>,
    pub seed_ids: Vec<uuid::Uuid>,
    pub edge_types: Vec<String>,
    pub depth: Option<i32>,
    pub no_graph: bool,
}

/// Build a SearchParams from CLI arguments.
pub fn build_search_params(args: CliSearchArgs<'_>) -> SearchParams {
    SearchParams {
        query: Some(args.query.to_string()),
        embedding: args.embedding,
        context_name: args.context.map(String::from),
        doc_type: args.doc_type.map(String::from),
        limit: args.limit,
        seed_ids: if args.seed_ids.is_empty() {
            None
        } else {
            Some(args.seed_ids)
        },
        edge_types: if args.edge_types.is_empty() {
            None
        } else {
            Some(args.edge_types)
        },
        graph_depth: args.depth,
        graph_expand: !args.no_graph,
        ..SearchParams::default()
    }
}

/// Call the search API with full SearchParams.
pub async fn search_api(
    client: &temper_client::TemperClient,
    params: SearchParams,
) -> Result<Vec<UnifiedSearchResultRow>> {
    client
        .search()
        .search_with_params(&params)
        .await
        .map_err(crate::commands::client_err)
}

/// Truncate a snippet to max_chars (character count), breaking at word boundaries.
pub fn truncate_snippet(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let byte_offset = text
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(text.len());
    let truncated = &text[..byte_offset];
    match truncated.rfind(' ') {
        Some(pos) => format!("{}...", &text[..pos]),
        None => format!("{truncated}..."),
    }
}

/// Format cloud search results as human-readable text lines.
pub fn format_text(results: &[UnifiedSearchResultRow]) -> Vec<String> {
    let mut lines = Vec::new();
    for (i, r) in results.iter().enumerate() {
        lines.push(format!(
            "{}. {} (score: {:.2}, via {})",
            i + 1,
            r.title,
            r.combined_score,
            r.origin
        ));
        lines.push(format!("   {}", r.slug));
        lines.push(String::new());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_short() {
        assert_eq!(truncate_snippet("short", 200), "short");
    }

    #[test]
    fn test_truncate_long() {
        let long = "word ".repeat(100);
        let result = truncate_snippet(&long, 20);
        assert!(result.ends_with("..."));
        assert!(result.len() < 30);
    }

    #[test]
    fn test_truncate_no_space() {
        assert_eq!(truncate_snippet("aaaaaaaaaaaa", 5), "aaaaa...");
    }

    #[test]
    fn test_build_search_params_passes_graph_flags() {
        let args = CliSearchArgs {
            query: "hello",
            embedding: None,
            context: Some("temper"),
            doc_type: None,
            limit: Some(5),
            seed_ids: vec![],
            edge_types: vec!["broader".into()],
            depth: Some(3),
            no_graph: false,
        };
        let params = build_search_params(args);
        assert_eq!(params.query.as_deref(), Some("hello"));
        assert_eq!(params.context_name.as_deref(), Some("temper"));
        assert_eq!(params.limit, Some(5));
        assert_eq!(params.edge_types.as_deref(), Some(&["broader".to_string()][..]));
        assert_eq!(params.graph_depth, Some(3));
        assert!(params.graph_expand);
    }

    #[test]
    fn test_build_search_params_no_graph_disables_expand() {
        let args = CliSearchArgs {
            query: "x",
            embedding: None,
            context: None,
            doc_type: None,
            limit: None,
            seed_ids: vec![],
            edge_types: vec![],
            depth: None,
            no_graph: true,
        };
        let params = build_search_params(args);
        assert!(!params.graph_expand);
    }

    #[test]
    fn test_format_text_includes_score_and_origin() {
        let row = UnifiedSearchResultRow {
            resource_id: uuid::Uuid::nil(),
            title: "Test".to_string(),
            slug: "test".to_string(),
            kb_uri: "kb://x/y/z".to_string(),
            origin_uri: "file://...".to_string(),
            context: None,
            doc_type: "task".to_string(),
            fts_score: 0.5,
            vector_score: 0.0,
            combined_score: 0.5,
            origin: "fts".to_string(),
        };
        let lines = format_text(&[row]);
        assert!(lines[0].contains("Test"));
        assert!(lines[0].contains("0.50"));
        assert!(lines[0].contains("fts"));
        assert!(lines[1].contains("test"));
    }
}
```

**Important — verify the `UnifiedSearchResultRow` field names against the live struct definition** before committing this exact code:

```bash
rg -n -A 20 'pub struct UnifiedSearchResultRow' crates/temper-core/src/types/api.rs
```

If field names differ (e.g. `combined_score` is actually `score`), adjust the `format_text` body + the test fixture to match.

- [ ] **Step 4: Run `cargo make check`**

```bash
cargo make check > /tmp/ch6_task5_check.log 2>&1; tail -60 /tmp/ch6_task5_check.log
```

Expected: 0 errors. Possible new dead-code warnings: `projection::warn_if_context_stale` (if `search_cmd.rs` was its sole consumer — Task 6 sweeps).

If `unresolved import` errors surface, the most likely cause is a missed field rename or a missed callsite. Re-read both files end-to-end.

- [ ] **Step 5: Run targeted nextest**

```bash
cargo nextest run -p temper-cli > /tmp/ch6_task5_nextest.log 2>&1; tail -40 /tmp/ch6_task5_nextest.log
```

Expected: all pass, including the new `test_build_search_params_*` and `test_format_text_includes_score_and_origin` tests. The deleted manifest-dependent tests should not appear.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "cloud-only(ch6): slim actions/search.rs + rewrite search_cmd to cloud-only dispatch"
```

---

## Task 6: Sweep pub-orphans surfaced by `cargo make check`

Tasks 2–5 deleted a lot of code. Clippy under `-D warnings` will surface helpers and types that were only used by deleted callers. Walk the warnings, verify each candidate is truly unused (symmetric-removal: check producers AND consumers), delete, recompile.

**Likely sweep targets (verify each — these are starting points, not a fixed list):**

- `crates/temper-cli/src/projection.rs::warn_if_context_stale` if Task 5 was its sole consumer
- Items in `crates/temper-cli/src/actions/runtime.rs` whose only callers were in deleted modules (e.g. helpers used only by the HNSW/graph pipelines)
- `temper-core` types/methods used only by the deleted action modules (e.g. `ManifestFileView`, `IndexManifestView` — these were `pub(crate)` inside `actions/graph_index/cluster.rs` so likely die WITH that file in Task 4; but verify any cross-crate orphans)
- Configuration fields under `temper-core::types::config::GraphIndexConfig` if no surviving consumer reads them (check carefully — the field set may still be referenced by `Config`'s `Default` impl even if no code path consumes it)

**Files (candidates — verify each is unused before deleting):**

- Modify: `crates/temper-cli/src/projection.rs` (verify `warn_if_context_stale` orphan status)
- Modify: `crates/temper-cli/src/actions/runtime.rs` (verify any orphan helpers)
- Modify: `crates/temper-core/src/types/config.rs` and `temper-core/src/types/mod.rs` (verify `GraphIndexConfig` orphan status — likely STAYS deferred to Chunk 7 since `actions/doctor*` may reference; **be conservative**)

- [ ] **Step 1: Run `cargo make check` and collect dead-code warnings**

```bash
cargo make check > /tmp/ch6_task6_check_raw.log 2>&1
rg 'dead.code|never.used|is never read|never.constructed|unused_import' /tmp/ch6_task6_check_raw.log | sort -u > /tmp/ch6_task6_warnings.log
wc -l /tmp/ch6_task6_warnings.log
cat /tmp/ch6_task6_warnings.log
```

This produces the authoritative deletion target list.

- [ ] **Step 2: Symmetric-removal verification**

For each warning-surfaced item, run a final grep to confirm no surviving caller exists:

```bash
rg '\b<ITEM_NAME>\b' --type rust
```

For each, the only hits should be the definition itself (or peer items in the same dead chain). If a real caller exists in a deferred-Chunk-7 file (`actions/{doctor,doctor_fix,ingest}.rs`, `commands/doctor.rs`, `manifest_io.rs`, `actions/sync.rs`, or `temper-core::types::{manifest,sync}.rs`), **do not delete** — the item survives for the deferred consumers.

**Critical: symmetric audit.** For each `pub fn X` flagged dead:
- Check if `X` is a *reader* — what was the *writer* / producer that fed it? Is that now dead too?
- Check if `X` is a *writer* — what was the *consumer* that read its output? Is that now dead too?

Chunks 4 + 5's `set_cached_profile_slug` writer-orphan miss is the cautionary tale.

- [ ] **Step 3: Delete the verified-dead items**

Walk each warning-surfaced item top-down. Delete the item plus any `use` lines that become unused.

**For `temper-core/src/types/{config,manifest,sync}.rs`:** be **conservative**. Only delete fields/methods that grep proves have zero remaining callers across the workspace (including the deferred Chunk-7 consumers). When in doubt, leave it — Chunk 7's deletion of the whole types modules will scoop everything up.

**For `crates/temper-cli/src/projection.rs`:** if `warn_if_context_stale` has only one consumer (the rewritten `search_cmd`), and that consumer no longer calls it, delete the function. If it has multiple consumers, leave it alone.

- [ ] **Step 4: Re-run `cargo make check`**

```bash
cargo make check > /tmp/ch6_task6_check_final.log 2>&1; tail -40 /tmp/ch6_task6_check_final.log
```

Expected: 0 errors, 0 dead-code warnings. (Some pub-at-lib items may stay silent — same trap as Chunks 4 + 5. Sample-check 2–3 items from the previous warning list and confirm they're either deleted or genuinely re-consumed.)

- [ ] **Step 5: Run targeted nextest for affected crates**

```bash
cargo nextest run -p temper-cli > /tmp/ch6_task6_temper_cli.log 2>&1; tail -20 /tmp/ch6_task6_temper_cli.log
cargo nextest run -p temper-core > /tmp/ch6_task6_temper_core.log 2>&1; tail -20 /tmp/ch6_task6_temper_core.log
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "cloud-only(ch6): sweep pub-orphans after HNSW/graph deletion"
```

**Empty-commit alternative:** if the warning list is empty and no items need removal, commit empty with a justification message:

```bash
git commit --allow-empty -m "$(cat <<'EOF'
cloud-only(ch6): pub-orphan audit — no sweep needed

Verified post-Task-5 pub items: every flagged candidate either has surviving
consumers in deferred-Chunk-7 files (manifest_io, doctor*, ingest, sync) or
is already a non-pub item dying with its parent module.
EOF
)"
```

---

## Task 7: Full verification + consolidated review

Run all four verification tiers locally, then dispatch a fresh opus reviewer for consolidated review. Address findings inline; PR stays unopened (PR B accumulates Chunks 3–8).

**Files:** none modified except possible review-followup fixes.

- [ ] **Step 1: Tier 1 — `cargo make check`**

```bash
cargo make check > /tmp/ch6_task7_tier1.log 2>&1; tail -30 /tmp/ch6_task7_tier1.log
```

Expected: 0 errors, 0 warnings.

- [ ] **Step 2: Tier 2 — workspace unit + integration tests**

```bash
cargo nextest run --workspace > /tmp/ch6_task7_tier2.log 2>&1; tail -40 /tmp/ch6_task7_tier2.log
```

Expected: 100% pass.

- [ ] **Step 3: Tier 3 — e2e with `test-db`**

```bash
cargo make test-e2e > /tmp/ch6_task7_tier3.log 2>&1; tail -40 /tmp/ch6_task7_tier3.log
```

Expected: 100% pass. Per Chunks 3 + 4 + 5 session notes, the `access_gate_test` parallel e2e flake is environmental — if it fails, re-run serially:

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -j 1 access_gate > /tmp/ch6_task7_tier3_serial.log 2>&1
```

- [ ] **Step 4: Tier 4 — e2e with `test-db,test-embed`**

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed > /tmp/ch6_task7_tier4.log 2>&1; tail -40 /tmp/ch6_task7_tier4.log
```

Expected: 100% pass (modulo the known env flake). **Test count drops noticeably** — `graph_index_integration.rs` was the heaviest `test-embed` consumer and is gone in Task 2.

- [ ] **Step 5: Verify the acceptance criteria**

```bash
# (1) action modules gone
for f in crates/temper-cli/src/actions/graph_build.rs \
         crates/temper-cli/src/actions/index.rs \
         crates/temper-cli/src/actions/index_build.rs; do
  test ! -f "$f" && echo "OK: $f gone" || echo "FAIL: $f still present"
done
test ! -d crates/temper-cli/src/actions/graph_index && echo "OK: graph_index/ gone" || echo "FAIL"

# (2) command modules gone
for f in crates/temper-cli/src/commands/graph.rs \
         crates/temper-cli/src/commands/index.rs; do
  test ! -f "$f" && echo "OK: $f gone" || echo "FAIL: $f still present"
done

# (3) test files gone
for f in tests/e2e/tests/graph_build_e2e_test.rs \
         crates/temper-cli/tests/graph_build_test.rs \
         crates/temper-cli/tests/graph_index_integration.rs; do
  test ! -f "$f" && echo "OK: $f gone" || echo "FAIL: $f still present"
done

# (4) Commands::Graph + Commands::Index + GraphAction variants/enums gone
rg -n 'Commands::Graph\b|Commands::Index\b|pub enum GraphAction|GraphAction\b' crates/temper-cli/src/ && echo "FAIL: surfaces still present" || echo "OK: surfaces removed"

# (5) actions/search.rs no longer imports Manifest
rg -n 'use temper_core::types::(Manifest|ResourceId)' crates/temper-cli/src/actions/search.rs && echo "FAIL" || echo "OK: search.rs slim"

# (6) actions/search.rs no longer has enrich_results
rg -n 'fn enrich_results\b|pub struct EnrichedSearchResult' crates/temper-cli/src/actions/search.rs && echo "FAIL" || echo "OK: enrich path gone"

# (7) commands/search_cmd.rs no longer uses manifest_io
rg -n 'manifest_io' crates/temper-cli/src/commands/search_cmd.rs && echo "FAIL" || echo "OK: search_cmd no manifest dep"

# (8) Commands::Search survives + still dispatches through search_cmd
rg -n 'Commands::Search' crates/temper-cli/src/main.rs > /tmp/ch6_verify_search.log
test -s /tmp/ch6_verify_search.log && echo "OK: Search dispatch present" || echo "FAIL: Search surface removed by mistake"

# (9) temper-ingest server-side path still alive
test -f crates/temper-ingest/src/lib.rs && echo "OK: temper-ingest survives" || echo "FAIL"
rg -l 'temper_ingest::|use temper_ingest' crates/temper-api/ > /tmp/ch6_verify_ingest.log
test -s /tmp/ch6_verify_ingest.log && echo "OK: temper-api still consumes temper-ingest" || echo "FAIL"

# (10) Plan-gate documented (manual check)
echo "OK: plan-gate documented (see this plan's preamble)"
```

All ten should print OK.

**Note:** The original task brief's acceptance criterion *"no local embedding"* is NOT checked here — the plan-gate refinement explicitly keeps `embed_query` (orthogonal to HNSW). The brief's narrower acceptance criteria ("`actions/search.rs` is gone") is also adjusted — slimming is the right shape (verify in Step 5 grep #5–6).

- [ ] **Step 6: Dispatch the consolidated opus review**

Dispatch a fresh opus subagent (general-purpose) with this prompt (substitute predecessor SHA `e48e7ee` and plan-file path):

```
You are reviewing the implementation of Chunk 6 of the cloud-only-vault
deprecation on the branch `jct/cloud-only-vault-pr-b`. Inspect the
commits added since the predecessor commit e48e7ee
"cloud-only(ch5): review followups (CLAUDE.md sync-recovery guidance)".

The plan is at:
  docs/superpowers/plans/2026-05-25-cloud-only-vault-chunk6-delete-hnsw-and-rework-search.md

The plan REFINES the original task brief in two ways (both documented
in the plan's preamble):

1. The brief's "no local embedding" acceptance criterion is REFINED:
   query-side embedding (temper_ingest::embed::embed_text over the query
   string) is orthogonal to local HNSW. The plan keeps embed_query in
   slim actions/search.rs because it's only feeding the cloud /api/search
   request — not building a local index. Do NOT flag `temper_ingest`
   still being imported by `actions/search.rs` as a miss.

2. The brief says "Delete actions/search.rs" — refined to "Slim
   actions/search.rs" because most of the file (embed_query, CliSearchArgs,
   build_search_params, search_api, truncate_snippet, format_text) is
   orthogonal to manifest enrichment. Only the manifest-dependent path
   (enrich_results, EnrichedSearchResult.local/vault_path, 6 affected
   tests) is deleted.

The plan ALSO defers the same set of symbols Chunk 5 deferred (manifest_io,
temper-core::types::{manifest,sync}, actions/sync.rs) to Chunk 7. Do NOT
flag those surviving symbols as misses.

Review for:
1. Correctness — was each task implemented as specified? Did the test
   triage's verdict table get honored (no false-positive deletions)?
2. Code quality — match against the project's CLAUDE.md rules: typed
   structs over inline JSON, params structs over too-many-args, no
   premature backward-compat shims, patterns match siblings.
3. Bisectability — did intermediate commits leave the build broken
   between tasks? (e.g. did Task 4's action-module deletion happen BEFORE
   Task 3's surface cleanup, leaving commands/{graph,index}.rs with
   unresolved imports? Did Task 5's atomic slim+rewrite stay atomic, or
   was it split across commits leaving a broken intermediate state?)
4. Pub-orphan sweep completeness (symmetric removal) — are there still
   dead pub items where one side (reader OR writer) got deleted but
   the other survived? Specifically check:
   - `projection.rs::warn_if_context_stale` (was likely sole-consumer
     in search_cmd.rs)
   - `temper-core::types::config::GraphIndexConfig` (HNSW-only — verify
     it has no surviving consumer in deferred-Chunk-7 files)
   - `actions/runtime.rs` helpers that only the deleted modules used
5. Surface consistency — `temper graph` and `temper index` should
   produce clap's "unknown subcommand" error, not panics. `temper search`
   should still work end-to-end against the cloud (manual test if
   possible: `temper search "hello" --doc-type task`).
6. `actions/search.rs` slim correctness — verify the surviving file:
   - Does NOT import Manifest or ResourceId
   - DOES import temper_ingest::embed (via embed_query)
   - DOES import temper_core::types::api::{SearchParams, UnifiedSearchResultRow}
   - DOES export embed_query, CliSearchArgs, build_search_params,
     search_api, truncate_snippet, format_text
   - Inline tests cover truncate_snippet, build_search_params, format_text
     (NOT enrich_results)
7. Did the doc-comment sweep leave any dangling references? Quick grep:
   `rg 'manifest_io|enrich_results|EnrichedSearchResult|GraphAction|graph_build|graph_index|index_build'`
   should only surface deferred-Chunk-7 consumers + this plan file itself
   + the predecessor plan file.
8. The 4-tier verification suite results in Steps 1–4 above — did all
   pass? If any failed, was a documented serial-rerun applied
   (access_gate flake)?

Return READY / READY_WITH_FOLLOWUPS / NEEDS_CHANGES. List findings
by severity (critical / important / minor) with file:line refs.
```

- [ ] **Step 7: Address findings inline**

If the review returns READY_WITH_FOLLOWUPS or NEEDS_CHANGES, address critical/important findings in a single review-followup commit. Minor findings (docstring nits, naming) fold into the same commit. Per Chunks 3 + 4 + 5 precedent, this lands as one commit, not per-finding.

```bash
git add -A
git commit -m "cloud-only(ch6): review followups"
```

- [ ] **Step 8: Save session note**

Pipe the session summary via stdin:

```bash
cat <<'EOF' | temper resource create --type session --title "Cloud-only vault Chunk 6 landed (HNSW + graph-build deleted, search reworked to cloud-only)" --context temper
## Goal
(describe goal here — Chunk 6 of cloud-only-vault deprecation: delete HNSW
indexing, graph-build, graph-index, three doomed test files; slim
actions/search.rs to drop manifest enrichment; rewrite commands/search_cmd
to cloud-only dispatch through temper-client)

## What Happened
(describe execution and surprises — especially pub-orphan sweep findings,
any deferred-consumer compile issues that surfaced, and whether the
search-semantics refinement held up under review)

## Decisions
(describe key decisions — search-semantics refinement: keep query
embedding in slim actions/search.rs because it's orthogonal to HNSW;
Option A for format_text shape: drop EnrichedSearchResult entirely,
format UnifiedSearchResultRow directly)

## Connections
- Branch (no PR yet): jct/cloud-only-vault-pr-b
- Plan: docs/superpowers/plans/2026-05-25-cloud-only-vault-chunk6-delete-hnsw-and-rework-search.md
- Predecessor session: 2026-05-24-cloud-only-vault-chunk-5-landed-sync-cmd-gutted-push-research-publish-helper-deleted-4-e2e-tests-gone-sync-rs-manifest-io-deferred-to-chunk-7
- Spec: docs/superpowers/specs/2026-05-21-cloud-only-vault-deprecation-design.md

## Next Steps
- Chunk 7: doctor + ingest rework (drops actions::{doctor, doctor_fix, ingest}, commands::doctor) — also FINALLY deletes actions/sync.rs, manifest_io.rs, and temper-core::types::{manifest,sync}
- Chunk 8: docs sweep + PR open
- Project memory: project_cloud_only_vault_direction (update with Chunk 6 done)
EOF
```

- [ ] **Step 9: Mark the task done**

```bash
temper resource update 2026-05-24-cloud-only-vault-chunk-6-delete-local-hnsw-indexing-graph-build-and-rework-search-to-cloud-only --type task --context temper --stage done
```

---

## Self-Review

**Spec coverage:**

| Brief acceptance criterion | Plan coverage |
|---|---|
| `crates/temper-cli/src/actions/index_build.rs` is gone | Task 4 |
| `crates/temper-cli/src/actions/graph_build.rs` is gone | Task 4 |
| `crates/temper-cli/src/actions/graph_index/` directory is gone | Task 4 |
| `crates/temper-cli/src/actions/search.rs` is gone | **REFINED** to "slim" (Task 5) — see plan-gate resolution |
| `commands/search_cmd.rs` dispatches through `temper-client::resources().search(...)`; no `manifest_io` dep; no `Manifest` ref; no local embedding | **REFINED** — dispatches through `client.search().search_with_params(...)`; no manifest_io ✓; no Manifest ✓; query embedding **stays** (orthogonal to HNSW) |
| `tests/e2e/tests/graph_build_e2e_test.rs` is gone | Task 2 |
| `Commands::{Graph, Index}` CLI variants + dispatch arms gone | Task 3 |
| `Commands::Search` survives; `temper search` returns cloud-only results | Tasks 3 + 5 |
| `temper-ingest`'s server-side embed pipeline is unchanged | Verified in audit (consumers: `temper-api/services/ingest_service.rs`, `temper-api/backend/translators.rs`); Task 7 Step 5 grep #9 |
| Plan-gate question resolved in preamble | This file's "Plan gate — resolution" section |
| All four verification tiers green | Task 7 Steps 1–4 |
| E2E tests touching search/HNSW have explicit delete-or-repoint verdicts | Task 1 triage + Task 2 deletions |
| No PR opened | Implicit; PR B accumulates Chunks 3–8 |

**Brief acceptance criteria DEFERRED (and why):**

| Criterion | Where it lands instead | Reason |
|---|---|---|
| `crates/temper-cli/src/manifest_io.rs` is gone | Chunk 7 | 4 consumers remain (doctor, doctor_fix, ingest, commands/doctor) |
| `temper-core::types::{manifest,sync}` are gone | Chunk 7 | Same |
| `actions/sync.rs` is gone | Chunk 7 (consolidated late task) | 1 consumer remains (doctor::normalize_all_entries) |

**Plan-gate consistency check:** Tasks 2 → 3 → 4 → 5 progressively delete leaves inward (tests → surfaces → action modules → search-slim+rewrite). Each task ends with `cargo make check` green (modulo dead-code warnings, which Task 6 sweeps). Task 5 is atomic across two files so each intermediate branch state compiles. The branch stays bisectable.

**Type-consistency check:**
- `commands::search_cmd::run` signature `(query: &str, context: Option<&str>, doc_type: Option<&str>, limit: Option<i64>, format: &str, text_only: bool, seed_ids: Vec<Uuid>, edge_types: Vec<String>, depth: Option<i32>, no_graph: bool) -> Result<()>` is **preserved exactly** in Task 5 so `main.rs`'s `Commands::Search` dispatch arm continues to compile without changes.
- `actions::search`'s `embed_query`, `CliSearchArgs`, `build_search_params`, `search_api`, `truncate_snippet`, `format_text` are **all preserved**; only `enrich_results`, `EnrichedSearchResult` (struct dropped entirely), and the manifest import are removed.
- `format_text` signature **changes** from `&[EnrichedSearchResult]` to `&[UnifiedSearchResultRow]` (Option A in Task 5's preamble) — its single caller (`commands::search_cmd::run`) is rewritten in the same atomic commit.
- `temper-ingest` is **unchanged** by this chunk; the `embed` feature gate, the `embed_text` API, and the server-side consumers in `temper-api` all survive.

**Placeholder scan:** No `TBD`, no "TODO", no "implement similar to Task N". Code blocks present for every behavior-changing step. Tasks 6 and 7 describe sweeps/verifications whose source-of-truth is `cargo make check` output, grep, and the four-tier suite, not a fixed list.

**Notes for the implementer-subagent dispatch (sonnet recommended for Tasks 0–6; opus only for Task 7 Step 6):**

- Include `SG-1`, `SG-2`, `SG-5`, `SG-6`, `SG-10` from `subagent-guidance.md` verbatim in every dispatch prompt.
- Include project fundamentals references on typed structs, params structs, and "no premature backward-compat" (relevant — Task 5 picks Option A over Option B for this reason).
- For Tasks 2–6 (deletion + sweep): emphasize "verify before deleting" — every removed item must be confirmed unused by grep first. Symmetric-removal audit on every `pub` item touched.
- For Task 5 (the only non-deletion task): emphasize atomicity. Both files modified, one commit. Run `cargo make check` AFTER both files are modified, not after only one.
- Cargo output redirection discipline: `> /tmp/foo.log 2>&1`, never `2>&1 | tail`.
- Task 6's sweep instruction has a "be conservative" clause for `temper-core/src/types/{config,manifest,sync}.rs` — make sure the dispatched subagent reads it; over-deletion there breaks Chunk 7's consumers.
- `temper-ingest` is server-side **and** CLI query-side after this chunk — the `embed` feature flag stays on `temper-cli`. If a subagent suggests removing `temper-ingest` as a `temper-cli` dep, STOP and reject.
