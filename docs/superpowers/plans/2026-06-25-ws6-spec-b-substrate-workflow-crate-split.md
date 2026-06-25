# WS6 Spec B — temper-substrate / temper-workflow crate split — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename `temper-next` → `temper-substrate` (PR1), then extract the domain-A type/logic cluster out of `temper-core` into a new `temper-workflow` crate (PR2), so the codebase has explicit substrate-kernel and workflow-frame crate boundaries.

**Architecture:** Mechanical re-home (not a behavioral refactor). temper-core stays the dependency leaf; the domain-A cluster moves *up* into temper-workflow (which depends on temper-core only); temper-substrate (the renamed temper-next) is a sibling leaf. The compiler + the existing test matrix are the verification — no new behavior, no new tests. Each PR is **atomic** (the whole-workspace clippy gate / pre-commit hook makes a half-done move fail to compile) and must leave `cargo make check` + the full test matrix green before merge.

**Tech Stack:** Rust workspace, cargo-make, cargo-nextest, sqlx (offline cache), ts-rs codegen.

## Global Constraints

- **Two separate PRs, each independently green and mergeable.** PR1 = rename. PR2 = extraction. Do not interleave.
- **Each PR is one atomic commit-state.** A partial move does not compile (workspace clippy gate). Do not try to split PR2 into intermediate compiling commits — it lands as one cohesive change. (Ref memory: cross-crate type-extension refactors need one atomic commit.)
- **The Postgres namespace `temper_next` is NOT renamed.** Only the *crate* renames. Disambiguation rule (PR1): `temper-next` (hyphen) → `temper-substrate`; `temper_next::` (underscore + `::`) → `temper_substrate::`; **bare `temper_next` (underscore, no `::`) stays** — it is the SQL schema (`search_path TO temper_next`, `table_schema='temper_next'`, `--schema=temper_next`, `DROP SCHEMA temper_next`).
- **cargo make tasks force `SQLX_OFFLINE=true`.** temper-workflow has no `sqlx::query!` macros (only the `FromRow` derive), so it needs no `.sqlx` cache and no CI exclusion. temper-substrate keeps its existing `crates/temper-substrate/.sqlx` cache (renamed dir).
- **Verification gate per PR (run all, all must pass):** `cargo make check` (fmt + clippy + machete + docs), `cargo make test`, `cargo make docker-up` then `cargo make test-db`, `cargo make test-e2e`, `cargo make test-e2e-embed`, the artifact tests (`cargo make test-next` — task name unchanged, see Task 1), and `cargo make generate-ts-types` must produce a **zero diff** in `packages/temper-ui` generated types.
- **After either PR merges (both touch temper-cli):** reinstall the PATH binary with `cargo install --path crates/temper-cli` (a merged-but-not-installed CLI fix silently behaves like the old code).
- Work on a branch off `main` per PR (`jct/<scope>`); the spec + this plan ride with PR1.

---

### Task 1: PR1 — rename `temper-next` → `temper-substrate`

Pure rename, behavior-identical. The crate's internals (including `keys.rs` and `scenario/`) do not change. The verification is "everything still compiles and every existing test still passes."

**Files:**
- Rename: `crates/temper-next/` → `crates/temper-substrate/` (whole dir via `git mv`, carries `.sqlx`, `src/`, `tests/`, `Cargo.toml`)
- Modify (Cargo deps): `crates/temper-api/Cargo.toml`, `crates/temper-agents/Cargo.toml`
- Modify (Rust paths): every `*.rs` containing `temper_next::` (api `backend/db_backend.rs`, `backend/substrate_read.rs`; agents `envelope.rs`; substrate's own `src/**` and `tests/**`)
- Modify (build/CI): `Makefile.toml`, `.config/nextest.toml`, `.github/workflows/code-quality.yml`, `.github/workflows/test-rust.yml`
- Modify (docs): `CLAUDE.md` (root-of-repo temper crate doc), `tests/e2e/tests/mcp_get_resource_meta_test.rs:81` (comment ref `temper-next keys.rs`)

**Interfaces:**
- Consumes: nothing (entry task).
- Produces: crate `temper-substrate` (Rust path `temper_substrate`) exporting exactly what `temper_next` did (`ids`, `payloads`, `keys`, `readback`, `writes`, `scenario`, `affinity`, `cluster`, etc.). Its package name is `temper-substrate`; default bin name becomes `temper-substrate`.

- [ ] **Step 1: Create the work branch**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
git checkout main && git pull
git checkout -b jct/ws6-pr1-rename-substrate
```

- [ ] **Step 2: Move the crate directory (preserves git history + `.sqlx` + tests)**

```bash
git mv crates/temper-next crates/temper-substrate
```

- [ ] **Step 3: Rename the package + every hyphenated `temper-next` (crate-only; SQL namespace is underscore so untouched)**

Hyphen never appears in a Postgres identifier, so replacing `temper-next` → `temper-substrate` across these file types is safe. This covers: package `name`, dependency entries + `path = "../temper-next"`, `-p temper-next`, `--bin temper-next`, `--exclude temper-next`, nextest `package(temper-next)` + group `temper-next-write`, and `crates/temper-next/...` paths inside `Makefile.toml`.

```bash
# Package + dependency declarations
sed -i '' 's/temper-next/temper-substrate/g' \
  crates/temper-substrate/Cargo.toml \
  crates/temper-api/Cargo.toml \
  crates/temper-agents/Cargo.toml
# Build orchestration + CI (hyphen form only)
sed -i '' 's/temper-next/temper-substrate/g' \
  Makefile.toml .config/nextest.toml \
  .github/workflows/code-quality.yml .github/workflows/test-rust.yml
```

- [ ] **Step 4: Rename the Rust crate path `temper_next::` → `temper_substrate::` (the `::` proves it is a crate path, never SQL)**

```bash
git ls-files '*.rs' | xargs grep -l 'temper_next::' | xargs sed -i '' 's/temper_next::/temper_substrate::/g'
```

- [ ] **Step 5: Verify the rename is complete and the SQL namespace survived**

```bash
# A) No hyphenated crate refs remain in code/build/CI (docs may keep historical prose):
grep -rn 'temper-next' --include='*.toml' --include='*.yml' --include='*.rs' crates tests .github Makefile.toml .config
# Expected: NO output.

# B) No crate-path refs remain:
grep -rn 'temper_next::' --include='*.rs' crates tests
# Expected: NO output.

# C) Remaining bare `temper_next` (underscore, no `::`) must ALL be the SQL namespace:
grep -rn 'temper_next' --include='*.rs' crates tests | grep -v 'temper_next::'
# Expected: only SQL/namespace hits — `search_path TO temper_next`, `table_schema='temper_next'`,
# `--schema=temper_next`, `DROP SCHEMA ... temper_next`, and comments about the namespace. DO NOT
# change these. The make tasks `prepare-next`/`test-next`/`flip-load-next` are namespace-oriented
# and KEEP their `-next` names (they operate on the temper_next namespace, which is not renamed).
```

- [ ] **Step 6: Fix the lingering doc-comment crate ref in the e2e test**

`tests/e2e/tests/mcp_get_resource_meta_test.rs:81` mentions `temper-next keys.rs:66` (a hyphenated crate ref in a comment — Step 3 only swept `crates tests .github`-rooted globs above; confirm it was caught, else fix):

```bash
sed -i '' 's/temper-next keys.rs/temper-substrate keys.rs/' tests/e2e/tests/mcp_get_resource_meta_test.rs
```

- [ ] **Step 7: Update `CLAUDE.md` crate references (keep namespace + task-name references)**

In the root-of-repo `CLAUDE.md` (the temper project file), update prose that names the **crate**/**dir**/**`.sqlx` path** — `crates/temper-next`, "temper-next is the only crate", "temper-next's per-crate `.sqlx`", "temper-next carries", "depending on temper-next" → `temper-substrate`. **Keep** every `temper_next` *namespace* reference (search_path, artifact namespace, `temper_next` schema) and **keep** the make-task names `cargo make test-next` / `prepare-next` (namespace-oriented). Do this as a careful manual edit, not a blind sed (the file mixes both heavily). Verify after:

```bash
grep -n 'temper-next' CLAUDE.md
# Expected: NO output (all crate-form hyphen refs updated; namespace form is underscore).
```

- [ ] **Step 8: Build + regenerate the lockfile**

```bash
cargo build --workspace --all-features 2>&1 | tail -20
# Expected: compiles. Cargo.lock updates the package name; stage it.
```

- [ ] **Step 9: Run the full verification gate**

```bash
cargo make check 2>&1 | tail -30
cargo make test 2>&1 | tail -20
cargo make docker-up && cargo make test-db 2>&1 | tail -20
cargo make test-e2e 2>&1 | tail -20
cargo make test-next 2>&1 | tail -20   # artifact tests; task name unchanged, runs -p temper-substrate vs temper_next namespace
cargo make generate-ts-types && git diff --stat packages/temper-ui
```
Expected: all green; `generate-ts-types` yields no diff (rename does not change any exported type).

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "WS6 Spec B PR1: rename temper-next -> temper-substrate (crate only; temper_next namespace unchanged)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: PR2 — create `temper-workflow`, extract the domain-A cluster from `temper-core`

One atomic move. Create the crate, `git mv` the domain-A modules into it, split the two mixed modules (`hash.rs`, `graph.rs`), re-point every consumer's imports, update the ts-rs task. Nothing in temper-substrate changes. The only green point is the end — drive the import fixes with the compiler using the deterministic rewrite rule below.

**Files:**
- Create: `crates/temper-workflow/Cargo.toml`, `crates/temper-workflow/src/lib.rs`
- Move (git mv, temper-core → temper-workflow): `src/frontmatter/` (incl. `DocType` in `document.rs`), `src/schema.rs`, `src/vault.rs`, `src/defaults.rs`, `src/operations/`, `src/types/resource.rs`, `src/types/managed_meta.rs`
- Split (edit in place, move part): `src/hash.rs` (move `compute_managed_hash`), `src/types/graph.rs` (move the `DocType`-dependent half)
- Modify (temper-core): `src/lib.rs`, `src/types/mod.rs` (drop moved mods + re-exports), `src/hash.rs`, `src/types/graph.rs`
- Modify (consumers — Cargo deps + imports): `crates/temper-api/`, `crates/temper-cli/`, `crates/temper-mcp/`, `crates/temper-client/`
- Modify (codegen): `Makefile.toml` (`generate-ts-types` task)

**Interfaces:**
- Consumes: `temper-substrate` (from Task 1), `temper-core` (the neutral leaf).
- Produces: crate `temper-workflow` (Rust path `temper_workflow`) exporting:
  - `temper_workflow::types::{resource::ResourceRow, managed_meta::ManagedMeta, graph::{ResourceRelationships, GraphNode, EdgeType, ...}}`
  - `temper_workflow::frontmatter::{...}` incl. `frontmatter::document::DocType`
  - `temper_workflow::{schema, vault, defaults}`
  - `temper_workflow::operations::{Backend, commands, refs, ...}` (the `Backend` trait now lives here, returning `temper_workflow::types::resource::ResourceRow`)
  - `temper_workflow::hash::compute_managed_hash(doc_type: &str, managed_meta: &serde_json::Value) -> String`
- temper-core after this task retains `temper_core::hash::{compute_body_hash, canonicalize_json, hash_canonical_json, compute_open_hash, doc_type_from_vault_path}` and `temper_core::types::graph::{EdgeKind, Polarity}`.

**Rewrite rule (apply inside every moved file):** a `crate::X` reference resolves against temper-workflow now, so:
- references to modules that **stayed in core** become `temper_core::`: `crate::error` → `temper_core::error`; `crate::types::ids` → `temper_core::types::ids`; `crate::hash::{the neutral fns}` → `temper_core::hash::...`; `crate::validation`/`config`/`projection` → `temper_core::...`; `crate::types::graph::{EdgeKind, Polarity}` → `temper_core::types::graph::...`; any neutral `crate::types::<neutral>` → `temper_core::types::<neutral>`.
- references to modules that **moved together** stay `crate::`: `crate::frontmatter`, `crate::schema`, `crate::vault`, `crate::defaults`, `crate::operations`, `crate::types::{resource, managed_meta}`, and the moved half of `graph`.
- `super::ids` (in `types/resource.rs`) → `temper_core::types::ids`; `super::managed_meta` stays `super::managed_meta` (moves alongside).

- [ ] **Step 1: Branch off main (PR1 merged)**

```bash
cd /Users/petetaylor/projects/tasker-systems/temper
git checkout main && git pull
git checkout -b jct/ws6-pr2-extract-workflow
```

- [ ] **Step 2: Scaffold the `temper-workflow` crate**

The workspace `members = ["crates/*", ...]` auto-includes the new dir. Create `crates/temper-workflow/Cargo.toml` (mirror the deps the moved modules use; `cargo machete` prunes any unused at the gate):

```toml
[package]
name = "temper-workflow"
version = "0.1.0"
edition = "2021"

[dependencies]
temper-core = { path = "../temper-core" }
async-trait = "0.1"
chrono = { version = "0.4", features = ["serde"] }
jsonschema = "0.45"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
serde_yaml = "0.9"
sha2 = "0.10"
sqlx = { version = "0.8", features = ["chrono", "json", "macros", "postgres", "runtime-tokio-rustls", "uuid"] }
thiserror = "2"
uuid = { version = "1", features = ["serde", "v7"] }
schemars = { version = "1", features = ["chrono04", "uuid1"], optional = true }
ts-rs = { version = "10", features = ["chrono-impl", "serde-json-impl", "uuid-impl"], optional = true }
utoipa = { version = "5", features = ["uuid"], optional = true }

[dev-dependencies]
tempfile = "3"
tokio = { version = "1", features = ["macros", "rt-multi-thread"] }

[features]
mcp = ["schemars", "temper-core/mcp"]
typescript = ["ts-rs", "temper-core/typescript"]
web-api = ["utoipa", "utoipa/chrono", "temper-core/web-api"]
```

Create `crates/temper-workflow/src/lib.rs` with the module tree (populated in Step 4):

```rust
pub mod defaults;
pub mod frontmatter;
pub mod hash;
pub mod operations;
pub mod schema;
pub mod types;
pub mod vault;
```

- [ ] **Step 3: Move the cleanly-domain-A modules (whole-file `git mv`)**

```bash
mkdir -p crates/temper-workflow/src/types
git mv crates/temper-core/src/frontmatter        crates/temper-workflow/src/frontmatter
git mv crates/temper-core/src/schema.rs          crates/temper-workflow/src/schema.rs
git mv crates/temper-core/src/vault.rs           crates/temper-workflow/src/vault.rs
git mv crates/temper-core/src/defaults.rs        crates/temper-workflow/src/defaults.rs
git mv crates/temper-core/src/operations         crates/temper-workflow/src/operations
git mv crates/temper-core/src/types/resource.rs  crates/temper-workflow/src/types/resource.rs
git mv crates/temper-core/src/types/managed_meta.rs crates/temper-workflow/src/types/managed_meta.rs
```

Create `crates/temper-workflow/src/types/mod.rs` re-exporting the moved + new-split graph types (fill the `pub use` lines to match what `temper-core/src/types/mod.rs` exported for these types — copy those exact re-export lines out of core's `types/mod.rs`):

```rust
pub mod graph;          // the moved half (Step 5)
pub mod managed_meta;
pub mod resource;
// pub use lines copied from temper-core/src/types/mod.rs for resource/managed_meta/graph
```

- [ ] **Step 4: Split `hash.rs` — move `compute_managed_hash` to workflow**

In `crates/temper-core/src/hash.rs`, **delete** `compute_managed_hash` (lines 59–79) and its sole-purpose import `use crate::frontmatter::fields::TIER1_SYSTEM_FIELDS;` (line 10). The remaining functions (`compute_body_hash`, `canonicalize_json`, `hash_canonical_json`, `compute_open_hash`, `doc_type_from_vault_path`) stay — none touch frontmatter/defaults.

Create `crates/temper-workflow/src/hash.rs` with the moved function, re-pointed per the rewrite rule:

```rust
//! Managed-metadata hash (domain-A: strips tier-1 system fields + applies doc-type defaults).

use crate::frontmatter::fields::TIER1_SYSTEM_FIELDS;
use temper_core::hash::hash_canonical_json;

/// Hash managed metadata: strip tier-1 system fields, apply doc-type defaults,
/// then hash the canonical JSON. Mirrors the client/server agreement contract.
pub fn compute_managed_hash(doc_type: &str, managed_meta: &serde_json::Value) -> String {
    let mut meta = managed_meta.clone();
    if let Some(obj) = meta.as_object_mut() {
        for &field in TIER1_SYSTEM_FIELDS {
            obj.remove(field);
        }
    }
    crate::defaults::apply_managed_defaults(doc_type, &mut meta);
    hash_canonical_json(&meta)
}
```

(Carry over the original doc comment verbatim from the deleted function.)

- [ ] **Step 5: Split `graph.rs` — keep `EdgeKind`/`Polarity` in core, move the `DocType`-dependent half**

In `crates/temper-core/src/types/graph.rs`, **keep** only the neutral edge taxonomy: `EdgeKind` (enum, ~line 72), `Polarity` (enum, ~line 95). Remove its `use crate::frontmatter::document::DocType;` (line 4) — the staying half no longer needs it. The staying `relationship_requests.rs`/`relationship_events.rs` already import `EdgeKind`/`Polarity` from here, so they keep working.

Create `crates/temper-workflow/src/types/graph.rs` with the **moved** half: `EdgeType`, `TargetRef`, `ResourceRelationships`, `is_aggregator`, `GraphNode`, `GraphEdge`, `GraphTraversalRow`, `GraphNeighborRow`, `GraphEdgeRow`, `ResolvedEdge`, `EdgeReconciliation`, `SubgraphResponse`, and the `impl EdgeType { legacy_mapping }`. Re-point their references:
- `use crate::frontmatter::document::DocType;` stays `crate::frontmatter::document::DocType` (frontmatter moved into workflow).
- `EdgeKind`/`Polarity` references → `use temper_core::types::graph::{EdgeKind, Polarity};`.

- [ ] **Step 6: Update `temper-core` module declarations**

In `crates/temper-core/src/lib.rs`, remove the moved top-level mods: `pub mod defaults;`, `pub mod frontmatter;`, `pub mod operations;`, `pub mod schema;`, `pub mod vault;`. Keep `pub mod hash;` and `pub mod types;`.

In `crates/temper-core/src/types/mod.rs`, remove `pub mod resource;`, `pub mod managed_meta;` and their `pub use` re-export blocks; trim the `graph` re-export block down to `EdgeKind`/`Polarity` (drop `ResourceRelationships`, `GraphNode`, `EdgeType`, `TargetRef`, etc. — they moved).

- [ ] **Step 7: Apply the rewrite rule inside the moved files**

Apply the rewrite rule (see Interfaces) across `crates/temper-workflow/src/**`. Start with the known `crate::error`/`crate::types::ids` cases, then let the compiler enumerate the rest:

```bash
cargo build -p temper-workflow --all-features 2>&1 | tail -40
```
Fix each unresolved-import error by the rule (stayed-in-core → `temper_core::`; moved-together → `crate::`). Repeat until `temper-workflow` compiles in isolation.

- [ ] **Step 8: Re-point consumers — Cargo deps**

Add `temper-workflow = { path = "../temper-workflow" }` to: `crates/temper-api/Cargo.toml`, `crates/temper-cli/Cargo.toml`, `crates/temper-mcp/Cargo.toml`, `crates/temper-client/Cargo.toml`. Propagate the `typescript`/`web-api`/`mcp` feature wiring where each crate already forwards those to temper-core (mirror the existing `temper-core/<feat>` lines with `temper-workflow/<feat>`).

- [ ] **Step 9: Re-point consumers — imports**

Across `temper-api`, `temper-cli`, `temper-mcp`, `temper-client`, rewrite import paths for the moved items:

```bash
# operations layer (Backend trait, commands, refs):
git ls-files 'crates/temper-api/*.rs' 'crates/temper-cli/*.rs' 'crates/temper-mcp/*.rs' 'crates/temper-client/*.rs' \
  | xargs sed -i '' \
    -e 's/temper_core::operations/temper_workflow::operations/g' \
    -e 's/temper_core::types::resource/temper_workflow::types::resource/g' \
    -e 's/temper_core::types::managed_meta/temper_workflow::types::managed_meta/g' \
    -e 's/temper_core::frontmatter/temper_workflow::frontmatter/g' \
    -e 's/temper_core::schema/temper_workflow::schema/g' \
    -e 's/temper_core::vault/temper_workflow::vault/g' \
    -e 's/temper_core::defaults/temper_workflow::defaults/g' \
    -e 's/temper_core::hash::compute_managed_hash/temper_workflow::hash::compute_managed_hash/g'
```

`temper_core::types::graph::{...}` imports need a hand split: `EdgeKind`/`Polarity` stay `temper_core::types::graph`; `ResourceRelationships`/`GraphNode`/`EdgeType`/etc. become `temper_workflow::types::graph`. Find them with:

```bash
grep -rn 'temper_core::types::graph' crates/temper-api crates/temper-cli crates/temper-mcp crates/temper-client --include='*.rs'
```
and fix each per which symbol it imports. In `temper-api/src/backend/db_backend.rs`, confirm `ResourceRow` now comes from `temper_workflow::types::resource` while `key_fate`/`KeyFate` still come from `temper_substrate::keys`.

- [ ] **Step 10: Compile the whole workspace; drive remaining fixes with the compiler**

```bash
cargo build --workspace --all-features 2>&1 | tail -60
```
Resolve every unresolved-import error by the rewrite rule. Repeat until the workspace builds.

- [ ] **Step 11: Update the `generate-ts-types` task**

In `Makefile.toml`, the `generate-ts-types` task runs the ts-rs export. Add temper-workflow to it so the moved `ts(export, …)` types (`ResourceRow`, `ManagedMeta`, moved `graph` types) regenerate. Mirror the existing temper-core invocation (e.g. add `cargo test -p temper-workflow --features typescript export_bindings` alongside the temper-core one — match the exact command shape already in the task).

- [ ] **Step 12: Run the full verification gate**

```bash
cargo make check 2>&1 | tail -40           # fmt + clippy + machete (prunes unused temper-workflow deps) + docs
cargo make test 2>&1 | tail -20
cargo make docker-up && cargo make test-db 2>&1 | tail -20
cargo make test-e2e 2>&1 | tail -20
cargo make test-e2e-embed 2>&1 | tail -20
cargo make test-next 2>&1 | tail -20       # substrate untouched, must still pass
cargo make generate-ts-types && git diff --stat packages/temper-ui
```
Expected: all green. `generate-ts-types` yields **zero diff** in `packages/temper-ui` (types unchanged, only their crate home moved). If machete flags a temper-workflow dep, remove it from `Cargo.toml`.

- [ ] **Step 13: Commit (single atomic commit)**

```bash
git add -A
git commit -m "WS6 Spec B PR2: extract temper-workflow from temper-core (domain-A cluster)

ResourceRow/ManagedMeta/DocType/frontmatter/schema/vault/defaults/operations +
graph(DocType-half) + hash::compute_managed_hash move to new temper-workflow crate
(deps temper-core only). EdgeKind/Polarity + neutral hash primitives + relationship
wire types stay in temper-core. Surfaces re-point imports. temper-substrate untouched.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- Rename (PR1) → Task 1. ✓
- temper-workflow creation + domain-A extraction (PR2) → Task 2. ✓
- Mixed-module splits (`hash.rs`, `graph.rs`) → Task 2 Steps 4–5. ✓
- `DocType` → workflow → Task 2 Step 3 (moves inside `frontmatter/document.rs`) + Step 5 (graph re-points to it). ✓
- keys.rs/scenario/ stay in substrate → honored (Task 2 touches only temper-core; substrate untouched). ✓
- Feature-flag propagation + ts-rs regen + no `.sqlx`/CI-exclusion → Task 2 Steps 2, 8, 11, 12. ✓
- Namespace `temper_next` not renamed → Task 1 disambiguation rule + Step 5 verification. ✓
- CI exclude rename → Task 1 Step 3 (sweeps `.github/workflows/*.yml`). ✓
- Green-at-each-step gate → both tasks' final steps run the full matrix. ✓

**Placeholder scan:** No TBD/TODO. The two compiler-driven steps (Task 2 Steps 7, 10) give the deterministic rewrite rule rather than enumerating every site — appropriate for a move-refactor where the compiler is authoritative; the *rule* is fully specified.

**Type consistency:** `compute_managed_hash` signature identical across delete (core) and recreate (workflow). `ResourceRow` consistently `temper_workflow::types::resource::ResourceRow`. `EdgeKind`/`Polarity` consistently `temper_core::types::graph`. `Backend` trait consistently in `temper_workflow::operations`.

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-25-ws6-spec-b-substrate-workflow-crate-split.md`.
