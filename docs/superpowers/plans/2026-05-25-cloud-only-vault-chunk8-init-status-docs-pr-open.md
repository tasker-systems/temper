# Cloud-only vault — Chunk 8: init/status rework, Chunk-7 leftovers, hnsw + docs sweep, PR B open

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Apply the consolidated-review cadence (`feedback_subagent_review_cadence`): per-task verification stays terse (`cargo make check` + targeted nextest); the full 4-tier suite + opus code review runs ONCE at the end (Task 9).

**Goal:** Final chunk of the 8-chunk cloud-only-vault deprecation. Reworks `temper init` and `temper status` for the cloud-only world, retires the stacked Chunk-7 leftovers (`SyncResolveRequest`/`ResolutionType` orphan types + `commands/sync_cmd.rs` stub-only surface), deletes the orphan `hnsw` feature in `temper-ingest`, sweeps guidance docs to match the new world, and opens **PR B** with Chunks 3-8 stacked.

**Architecture:** Sequential leaves-inward as in Chunks 4-7. Each phase ends with `cargo make check` green and a commit; the branch stays bisectable. Sequence:

1. **Task 1 (test triage)** — empty-commit verdict for every test file touched by Phases A-D.
2. **Task 2 (Phase A)** — `commands/init.rs` rework: drop local-vault scaffold, replace with `config + auth-check + ensure default context server-side`.
3. **Task 3 (Phase B)** — `commands/status.rs` rework: report projection staleness (cursor mtime + projected count vs server count) per context.
4. **Task 4 (Phase C)** — atomic deletion: `SyncResolveRequest` + `ResolutionType` from `types/sync.rs`; `commands/sync_cmd.rs` + `SyncAction` enum + `Commands::Sync` clap variant + the e2e test that exercises the stub.
5. **Task 5 (Phase D-1)** — `temper-ingest` `hnsw` feature + `tantivy`/`hnsw_rs` deps + cargo-machete-ignore entries dropped.
6. **Task 6 (Phase D-2)** — guidance-docs sweep: CLAUDE.md family + `.claude/skills/temper/**`. Specs/plans under `docs/superpowers/` left intact as historical record.
7. **Task 7 (pub-orphan sweep)** — broader-surface audit per `feedback_sweep_time_audit_surface` after all deletions land.
8. **Task 8 (final review + PR B)** — 4-tier verification, opus consolidated review, `git merge origin/main`, then `gh pr create`.

**Tech Stack:** Rust 2024 edition, cargo-make, cargo-nextest. No SQL changes; sqlx cache untouched. Phase A introduces ONE new `temper-client` async call (`contexts().ensure(name)` or equivalent — see Task 2). Phase B introduces ONE new `temper-client` async call (`resources().count_for_context(ctx_id)` or equivalent — see Task 3).

---

## Plan gate — resolution

Applied **all three** carry-forward feedbacks:

1. `feedback_plan_gate_audit_both_ends` (Chunk 5 lesson): grep BOTH type symbols AND module function names.
2. `feedback_sweep_time_audit_surface` (Chunk 6 lesson): include `Cargo.toml` features/deps + `pub mod` decls + struct fields, not just Rust symbol names.
3. `feedback_plan_gate_name_collision_audit` (Chunk 7 lesson): enumerate every `crate::*::<name>` and `temper_<crate>::*::<name>` candidate explicitly when a module name is ambiguous across namespaces.

### Per-phase user decisions (resolved before plan-writing)

| Decision point | Choice | Rationale |
|---|---|---|
| Phase A init scope | **Medium**: config + auth + ensure default context server-side | Smallest non-trivial shape that completes init for first-time users; leaves projection pull to an explicit `temper pull` follow-up. |
| Phase B status scope | **Cursor + projected count vs server count** | More informative than cursor-only without bleeding into the deferred `temper doctor` auth/connectivity surface. |
| Phase C sync_cmd fate | **Delete entirely** | Unambiguous in cloud-only world; aligns with the breaking-change narrative of PR B. |
| Phase D docs scope | **Sweep guidance docs only** | `docs/superpowers/specs/*` and `docs/superpowers/plans/*` describe past work — rewriting them is history rewriting. Sweep CLAUDE.md family + skill docs only. |

### Type-symbol audit (orphan types in `types/sync.rs`)

| Symbol | Consumers | Verdict |
|---|---|---|
| `ResolutionType` | `types/sync.rs:158` (def), `types/sync.rs:246,250` (self-tests), `types/mod.rs:73` (re-export) | **DELETE** (Task 4) — zero external consumers |
| `SyncResolveRequest` | `types/sync.rs:166` (def), `types/mod.rs:75` (re-export) | **DELETE** (Task 4) — zero external consumers |

All other types in `types/sync.rs` (`SyncStatusRequest`, `SyncManifestResponse`, `SyncPushItem`, `SyncCompleteRequest`, etc.) **survive** — they are the cloud sync API endpoints' wire types, actively used by `temper-api/src/handlers/sync.rs` and `temper-client`. The name `temper_core::types::sync` stays; only the I6c placeholder pair dies.

### Module-path + function-name audit (`sync_cmd`)

| File | Verdict |
|---|---|
| `crates/temper-cli/src/commands/sync_cmd.rs` | **DELETE** (Task 4) — 16 LOC, one fn `run()` returning a "cloud-only" error |
| `crates/temper-cli/src/commands/mod.rs` `pub mod sync_cmd;` | **DELETE** (Task 4) |
| `crates/temper-cli/src/main.rs:4` `SyncAction` import | **DELETE** (Task 4) |
| `crates/temper-cli/src/main.rs:316-321` `Commands::Sync` dispatch arm | **DELETE** (Task 4) |
| `crates/temper-cli/src/cli.rs:88-92` `Sync { action: SyncAction }` variant | **DELETE** (Task 4) |
| `crates/temper-cli/src/cli.rs:406-417` `SyncAction` enum | **DELETE** (Task 4) |
| `tests/e2e/tests/cloud_writes_test.rs:10` doc-comment ref | **MODIFY** (Task 4) — strip `sync_cmd::run` from the file header doc |
| `tests/e2e/tests/cloud_writes_test.rs:922-953` Test 7 (`sync_run_errors_with_cloud_only_message`) | **DELETE** (Task 4) — the test exists to assert the stub error; once the surface is gone, the test is meaningless |

### Module-path + function-name audit (`init`, `status`)

| File | Surfaces touched | Verdict |
|---|---|---|
| `crates/temper-cli/src/commands/init.rs` | `pub fn run`, `pub fn run_non_interactive`, `pub fn apply_answers`, `pub fn render_config_toml`, `pub enum AuthChoice`, `pub struct WizardAnswers` + 10 inline tests | **REWRITE** (Task 2) — keep `run`/`run_non_interactive`/`render_config_toml`; rework `apply_answers` to drop manifest/events writes and add server-side context ensure; rewrite inline tests |
| `crates/temper-cli/src/commands/status.rs` | `pub fn run`, `pub fn count_md_files` | **REWRITE** (Task 3) — `count_md_files` deleted (no longer applicable); `run` rewired to use `projection::check_context_staleness` + new `temper-client` count call |
| `crates/temper-cli/src/main.rs:88` `Commands::Init` dispatch | unchanged signature | **KEEP** (Task 2 amends only if the entry signature changes — current `(path, no_interactive, register_global)` stays) |
| `crates/temper-cli/src/main.rs:96` `Commands::Status` dispatch | `commands::status::run(&config, verbose)` | **REVIEW** (Task 3) — signature may need an async hop; if so, wrap in `Runtime::new().block_on()` per `feedback_runtime_helper_choice` |
| `crates/temper-cli/tests/init_test.rs` | `test_init_creates_vault_structure` — asserts `.temper/manifest.json`, `.temper/events.jsonl` | **REWRITE** (Task 2) — assert new cloud-only init invariants (config file written, no manifest/events sidecars) |
| `crates/temper-cli/tests/status_test.rs` | Three tests of `count_md_files` | **DELETE** the file (Task 3) — `count_md_files` no longer exists; new status tests live alongside `commands/status.rs` as inline tests |
| `crates/temper-cli/src/commands/init.rs` inline tests (lines 375-425) | `apply_answers_warns_on_existing_vault_but_succeeds`, `apply_answers_creates_vault_structure`, `no_interactive_defaults_and_applies` (all assert manifest.json/events.jsonl) | **REWRITE** (Task 2) — assert new invariants |
| `crates/temper-cli/tests/check_test.rs` | Calls `init::run` as test fixture | **REVIEW** (Task 2) — likely no functional change if `run` signature is preserved |

### Sweep-time audit surface (`temper-ingest` hnsw)

| Surface | Status | Verdict |
|---|---|---|
| `crates/temper-ingest/Cargo.toml:42` — `hnsw = ["dep:tantivy", "dep:hnsw_rs"]` | feature exists, **zero Rust consumers** | **DELETE** feature line (Task 5) |
| `crates/temper-ingest/Cargo.toml:27` — `tantivy = { version = "0.22", optional = true }` | optional dep, only referenced via `hnsw` feature | **DELETE** dep line (Task 5) |
| `crates/temper-ingest/Cargo.toml:28` — `hnsw_rs = { version = "0.3", optional = true }` | optional dep, only referenced via `hnsw` feature | **DELETE** dep line (Task 5) |
| `crates/temper-ingest/Cargo.toml:7-8` — `[package.metadata.cargo-machete]` `ignored = ["tantivy", "hnsw_rs"]` | guards machete against the now-deleted optionals | **DELETE** metadata stanza (Task 5) |
| `rg '#\[cfg(feature = "hnsw"' crates/temper-ingest/` | empty (no gated code) | nothing to remove in Rust |
| workspace-root `Cargo.toml` hnsw mentions | empty | n/a |
| `crates/temper-cli/Cargo.toml` hnsw/tantivy/hnsw_rs mentions | empty | n/a (Chunk 6 already cleaned the consumer side) |

Final sanity: after the cuts, `cargo make check` must stay green, `cargo machete` must stay clean. If either fails, the broader-surface sweep in Task 7 surfaces what was missed.

### Sweep-time audit surface (docs sweep, Phase D-2)

Plan-gate grep:

```bash
rg -l --hidden 'local vault|local-vault|VaultState|manifest|sync engine|temper sync|temper push|graph build|local mode' \
   /Users/petetaylor/projects/CLAUDE.md \
   /Users/petetaylor/projects/tasker-systems/CLAUDE.md \
   /Users/petetaylor/projects/tasker-systems/temper/CLAUDE.md \
   /Users/petetaylor/projects/tasker-systems/temper/docs/guides/ \
   /Users/petetaylor/.claude/skills/temper/
```

| File | Notes |
|---|---|
| `/Users/petetaylor/projects/tasker-systems/temper/CLAUDE.md` | Contains the **"Cloud mode operations"** paragraph that now describes the only mode — rewrite to drop the "when `TEMPER_VAULT_STATE=cloud`…" framing. Strip mentions of `temper sync run` redirect (the surface is gone). Strip mentions of `manifest`-based sync. Keep the cloud-mode body-edit forms section (still accurate). |
| `/Users/petetaylor/projects/tasker-systems/temper/CLAUDE.md` "Code Quality Rules" — bullet 4 ("Vault file IO and manifest IO live in `vault_backend/`") | Entire bullet **DELETE** — `vault_backend/` was removed in Chunk 4, `manifest_io` in Chunk 7. Replace with a one-line "All writes go through `temper-client` / `temper-api`" guideline. |
| `/Users/petetaylor/projects/tasker-systems/temper/CLAUDE.md` "Resource deletion is always explicit" paragraph | Update to drop the "implicit-delete-via-`rm`" framing — `rm` on a projected file is now just a local-cache miss, not a delete. Recovery via `temper pull <context>` still correct. |
| `/Users/petetaylor/projects/tasker-systems/temper/CLAUDE.md` "Environment / Pre-commit hook" — generic, unrelated | **KEEP** |
| `/Users/petetaylor/projects/tasker-systems/CLAUDE.md` | Project-level intro; contains no temper-specific local-vault language. **VERIFY** at sweep-time. |
| `/Users/petetaylor/projects/CLAUDE.md` | Cross-project knowledge-base intro; contains no temper-specific local-vault language. **VERIFY** at sweep-time. |
| `/Users/petetaylor/.claude/skills/temper/SKILL.md` | Generic skill metadata; **VERIFY** the cloud-only "On Session Start" wording. |
| `/Users/petetaylor/.claude/skills/temper/reference.md` | CLI command list — strip `sync`, `push`, `graph build`, `graph index`, `add`, `doctor`. |
| `/Users/petetaylor/.claude/skills/temper/session-lifecycle.md` | Session save/end patterns — should not mention local-vault. **VERIFY**. |
| `/Users/petetaylor/.claude/skills/temper/subagent-guidance.md` | Subagent prompt principles; **VERIFY** no local-vault refs. |
| `/Users/petetaylor/.claude/skills/temper/knowledge-base.md` | MCP resource patterns; **VERIFY**. |
| `/Users/petetaylor/.claude/skills/temper/guidance/fundamentals.md` | Project fundamentals — contains "Two-tier resources: `temper add` (fire-and-forget) vs `temper import` (vault-managed, synced)" nomenclature paragraph. **REWRITE** that paragraph: `temper add` was removed in Chunk 7; the two-tier framing is gone. |
| `/Users/petetaylor/.claude/skills/temper/workflows/*.md` | Per-mode workflow files; **VERIFY** no local-vault refs. |
| `crates/*/CLAUDE.md`, `crates/*/src/CLAUDE.md`, `crates/*/src/commands/CLAUDE.md` etc. | Most are `claude-mem-context` activity logs (auto-generated). **LEAVE** unless a manual section references local-vault concepts. |
| `docs/guides/cloud-agents.md` | Cloud-agent task-prep guide; **VERIFY** no contradictory local-vault refs. |
| `docs/guides/*.md` (other) | Sweep at task-time. |
| `docs/superpowers/specs/2026-*.md` | **DO NOT TOUCH** — historical record. |
| `docs/superpowers/plans/2026-*.md` | **DO NOT TOUCH** — historical record. |
| `docs/2026-*.md` (top-level docs, e.g. handoff notes) | **DO NOT TOUCH** — historical record. |
| `README.md` (top-level temper) | Sweep at task-time if it mentions local-vault sync. |

### Name-collision audit

Per `feedback_plan_gate_name_collision_audit`, the modules touched in this chunk are checked against every namespace candidate:

| Module name | Candidate namespaces | Live consumers | Risk |
|---|---|---|---|
| `init` | `temper_cli::commands::init` only | main.rs, init_test.rs, check_test.rs | **CLEAN** — no other crate exposes a module named `init` |
| `status` | `temper_cli::commands::status` only | main.rs, status_test.rs | **CLEAN** — `temper-cli::actions` has no `status`; nothing in `temper-api` or `temper-core` named `status` |
| `sync` | `std::sync` (stdlib), `temper_core::types::sync` (cloud API wire types — **stays**), `temper_cli::commands::sync_cmd` (this chunk deletes), `temper_api::handlers::sync` (cloud API handler — **stays**), `temper_client::api::sync` (client wrapper — **stays**) | confirmed via `rg "::sync::" --type rust` | **AUDIT-CRITICAL** — the bare grep `sync` returns many `std::sync` hits; the meaningful surfaces are `temper_cli::commands::sync_cmd` (delete) and the wire-type pair in `temper_core::types::sync` (lines 155-170 die, rest survives). Task 4 must not touch `temper-api/src/handlers/sync.rs` or its client/server counterparts. |
| `manifest` | (no surviving module, only string literals in paths) | none | **CLEAN** — Chunk 7 already deleted `manifest_io` and `temper_core::types::manifest`; only the wire field name `manifest` remains in `SyncManifestResponse` etc., which are unaffected |
| `projection` | `temper_cli::projection` only | many in temper-cli | **CLEAN** — sole namespace, no collisions |
| `hnsw` | `temper_ingest` Cargo feature only; no Rust modules | none | **CLEAN** — feature/dep deletion only |

### Stacked-deferral retirement after Chunk 8

After this chunk lands, **every** deferral accumulated since Chunk 4 is resolved. The cloud-only-vault deprecation is complete:

| Symbol / surface | Status post-Chunk-8 |
|---|---|
| `actions/sync.rs` | Gone (Chunk 7) |
| `manifest_io.rs` (+ lib decl) | Gone (Chunk 7) |
| `temper-core::types::manifest` | Gone (Chunk 7) |
| `temper-core::types::sync::{SyncResolveRequest,ResolutionType}` | **Gone (Chunk 8 Task 4)** |
| `temper-core::types::sync` (other wire types) | KEEP — cloud sync API |
| `actions/doctor.rs` / `doctor_fix.rs` / `commands/doctor.rs` | Gone (Chunk 7) |
| `commands/add.rs` + CLI `Commands::Add` | Gone (Chunk 7) |
| `actions/ingest.rs` | Slim (Chunk 7) |
| `commands/sync_cmd.rs` + `SyncAction` + `Commands::Sync` | **Gone (Chunk 8 Task 4)** |
| `commands/init.rs` local-vault scaffold | **Gone (Chunk 8 Task 2)** |
| `commands/status.rs` local file-count surface | **Gone (Chunk 8 Task 3)** |
| `temper-ingest` `hnsw` feature + `tantivy`/`hnsw_rs` deps | **Gone (Chunk 8 Task 5)** |
| Guidance docs referencing local-vault | **Swept (Chunk 8 Task 6)** |

---

## Items explicitly NOT in this chunk (deferred to follow-on tasks)

- **Cloud-only `temper doctor`** — auth + connectivity check. Schedule as a fresh task post-PR-B; not blocking this chunk. The Phase B status decision keeps health-check overlap minimal by reporting only projection staleness, not connectivity.
- **`temper-cli` warmup task** (`2026-05-22-cloud-ified-temper-warmup-replace-local-scan-in-progress-task-lookup`) — still pending in backlog; separate.
- **Cloud-mode session-task linking** (`2026-05-22-cloud-mode-session-task-linking-re-implement-link-session-to-task-for-cloud`) — still pending; separate.
- **CLI read-side meta affordance** (`2026-05-25-cli-read-side-meta-affordance-for-resources`) — backlog; separate.
- **Re-shaping `cloud_writes_test.rs` after Test 7 deletion** — only that one test is removed; the rest of the file remains valid.
- **Spec/plan-file rewriting** under `docs/superpowers/{specs,plans}/2026-*` — user decision: historical record, leave untouched.
- **Top-level `docs/2026-*.md` handoff notes** — same rationale, historical record.

## Branch

`jct/cloud-only-vault-pr-b` — **do not branch**. Chunks 3–8 accumulate on the same branch; PR B opens at the end of Task 8. Branch is at ~56 commits at Chunk 8 start.

## Execution discipline (carry forward from Chunks 3–7)

- **Subagent-driven execution** (per `feedback_prefer_subagent`), fresh sonnet implementer per task. **Consolidated review only** — opus reviewer fires once at Task 8 (per `feedback_subagent_review_cadence`).
- Each task ends with `cargo make check` green and a commit → branch stays bisectable.
- **Per-task verification is tightened**: `cargo make check` + targeted `-p` nextest only. Full workspace + e2e + embed tiers run **once** in Task 8.
- **Cargo output redirection**: always `> /tmp/foo.log 2>&1`. Never `2>&1 | tail` (silently produces 0-byte files under the harness — `feedback_cargo_output_redirection`).
- **Plan-committed-early**: Task 0 commits this plan before Task 1 starts.
- **Atomic deletion for tightly-coupled removals**: Task 4 lands all 4 file-level deletions in ONE commit (`SyncResolveRequest`+`ResolutionType` from `types/sync.rs` + `types/mod.rs` re-export trim, `commands/sync_cmd.rs`, `cli.rs` `SyncAction` enum + variant, `main.rs` dispatch arm + import, `cloud_writes_test.rs` Test 7) because the unit-of-removal is the whole surface; partial removal breaks `cargo check`.
- **Mid-execution amendments are normal at this scale** (per Chunk 5/7's lesson). If a task surfaces a blocker mid-execution, follow the Chunk 5/7 pattern: ask the user, get an Option, amend the plan (separate commit), continue.
- **Runtime-helper choice** (per `feedback_runtime_helper_choice`): Tasks 2 and 3 introduce one async `temper-client` call each. Use `tokio::runtime::Runtime::new().block_on()` inline; do NOT use `with_client`. The CLI dispatch entry stays synchronous.
- **PR open at end** — `gh pr create` follows after `git merge origin/main` (per `feedback_merge_main_before_pushing_pr`).

---

## Task 0: Commit this plan

Land the plan file before any code change so subsequent commits reference it. No code edit; one commit.

- [ ] **Step 1: Commit the plan file**

```bash
git add docs/superpowers/plans/2026-05-25-cloud-only-vault-chunk8-init-status-docs-pr-open.md
git commit -m "cloud-only(ch8): record the chunk 8 implementation plan"
```

---

## Task 1: Test triage

Inventory every test file whose code path is touched by this chunk. Produce explicit delete/keep/repoint verdicts in an **empty commit** so the analysis is bisectable.

Target files (from plan-gate audit):

- `crates/temper-cli/tests/init_test.rs` → **REWRITE** (Task 2): asserts will move from `manifest.json`/`events.jsonl` existence to config-file existence + absence of vault-scaffold sidecars.
- `crates/temper-cli/tests/status_test.rs` → **DELETE the file** (Task 3): tests `count_md_files` which is being removed. New status tests live as `#[cfg(test)] mod tests` inside `commands/status.rs`.
- `crates/temper-cli/tests/check_test.rs` → **KEEP** (Task 2): calls `init::run` as a test fixture; if `run` signature is preserved (which the plan requires), no edit. Verify at task-time.
- `crates/temper-cli/src/commands/init.rs` inline tests (lines 264-425) → **REWRITE** (Task 2): the 3 `apply_answers_*` tests that assert manifest/events sidecars need new assertions; the 7 `render_config_toml_*` tests stay as-is (TOML rendering is unchanged).
- `tests/e2e/tests/cloud_writes_test.rs` → **MODIFY** (Task 4): delete Test 7 (`sync_run_errors_with_cloud_only_message`, lines 922-953) + the `sync_cmd::run` reference in the file-header doc-comment (line 10). The rest of the file is unaffected.
- All other test files in `crates/temper-cli/tests/` and `tests/e2e/tests/` → **KEEP** unchanged.

Embed-gated tests (`test-embed`): none in this chunk's scope. Phase D's hnsw removal is feature-only; no test bodies are gated on `hnsw`.

- [ ] **Step 1: Verify the inventory holds against current code**

```bash
rg -l 'sync_cmd::run|SyncAction|Commands::Sync|count_md_files|manifest\.json|events\.jsonl' \
   crates/temper-cli/tests/ tests/e2e/tests/ > /tmp/ch8-test-triage.log 2>&1
```

Compare hits to the inventory above; flag any miss before Task 2 begins.

- [ ] **Step 2: Empty commit recording the verdict**

```bash
git commit --allow-empty -m "cloud-only(ch8): test-triage inventory for chunk 8

Touched-file verdicts for Phases A–D:
- crates/temper-cli/tests/init_test.rs        REWRITE (Task 2)
- crates/temper-cli/tests/status_test.rs       DELETE  (Task 3)
- crates/temper-cli/tests/check_test.rs        KEEP    (Task 2 verifies)
- crates/temper-cli/src/commands/init.rs inline 3 apply_answers tests REWRITE (Task 2)
- crates/temper-cli/src/commands/init.rs inline 7 render_config_toml tests KEEP
- tests/e2e/tests/cloud_writes_test.rs Test 7 + L10 doc-comment MODIFY (Task 4)
- all other tests KEEP

No embed-gated tests touched. Per-task verdicts are bisectable."
```

---

## Task 2: Phase A — `commands/init.rs` rework (cloud-only)

Reshape `init` to: ensure config exists at `~/.config/temper/config.toml`, verify auth (or prompt for the device-flow login), and ensure the default context exists server-side. Drop the local-vault scaffold (`.temper/manifest.json`, `.temper/events.jsonl`, vault directory creation, per-context subdirectory creation).

**Signature preservation:** `pub fn run(path: &Path, no_interactive: bool, register_global: bool) -> Result<()>` stays — `main.rs:88` dispatch is unchanged. The `path` argument is now ignored for vault-scaffold purposes; it is kept for backward compatibility with shell scripts that pass a path and to preserve the interactive prompt for vault path (which still informs the rendered TOML's `[vault] path` for the local projection cache).

**Surface to keep:**

- `pub enum AuthChoice` (only used by `WizardAnswers`)
- `pub struct WizardAnswers` (already cloud-ready)
- `pub fn run` + `pub fn run_non_interactive` (entry points)
- `pub fn render_config_toml` (TOML rendering — unchanged)
- `gather_answers`, `print_summary`, `prompt_err`, `default_vault_path`, `resolve_initial_vault` (helpers)
- All 7 `render_config_toml_*` inline tests

**Surface to delete from `apply_answers`:**

- `let manifest_path = state_dir.join("manifest.json");` block (lines 173-176) → DELETE
- `let events_path = state_dir.join("events.jsonl");` block (lines 177-180) → DELETE
- `std::fs::create_dir_all(vault.join("default"))?;` (line 183) → DELETE
- `for ctx in &answers.extra_contexts { std::fs::create_dir_all(vault.join(ctx))?; }` (lines 184-186) → DELETE
- `std::fs::create_dir_all(&state_dir)?;` (line 172) → KEEP **only if** the projection cursor sidecar location `.temper/projection/<context>.json` requires it; otherwise DELETE (projection.rs already does `create_dir_all(dir)?` lazily — confirm at task-time and prefer DELETE).
- The `vault.join(".temper")` marker check (lines 160-167) → KEEP semantically but change wording: "vault already initialized" instead of "vault already exists".

**Surface to add to `apply_answers`:**

- Auth verification: call `temper_client::auth::ensure_authenticated_if_provider_set(&config)` (or the equivalent existing helper — verify at task-time; fallback is a synchronous check of the cached token's presence/expiry). If the user picked `AuthChoice::None`, skip.
- Server-side default-context ensure: one async call via `tokio::runtime::Runtime::new().block_on()`. Pseudo-code:
  ```rust
  let rt = tokio::runtime::Runtime::new()?;
  let client = TemperClient::from_config(&loaded_config)?;
  let contexts = rt.block_on(client.contexts().list())?;
  if !contexts.iter().any(|c| c.name == "default") {
      rt.block_on(client.contexts().create(/* name = "default" */))?;
  }
  for ctx in &answers.extra_contexts {
      if !contexts.iter().any(|c| c.name == *ctx) {
          rt.block_on(client.contexts().create(/* name = ctx */))?;
      }
  }
  ```
  **Verify** the actual `temper-client` API surface for contexts at task-time (`crates/temper-client/src/api/contexts.rs` or similar). If `create` is not available client-side, use the closest equivalent. If neither exists, surface as a Plan amendment and stop.
- The auth + context ensure step happens AFTER the config file is written (if `register_global == true`), so the client can read the config to discover the cloud API URL and auth provider.

**Side effects after rework:**

- `~/.config/temper/config.toml` written (if `register_global`)
- Server-side: default context exists (idempotent)
- NO disk writes anywhere under the vault path
- NO `.temper/manifest.json`, NO `.temper/events.jsonl`

**Output messaging:**

- Drop "Vault initialized successfully" → "Temper initialized successfully. Run `temper pull default` to materialize a local projection."
- Drop the existing-vault marker check OR rephrase it: "Temper config already exists at `<config_path>` — re-running init is idempotent. To change settings, run `temper config edit`."

**Test rewrites:**

- `crates/temper-cli/tests/init_test.rs::test_init_creates_vault_structure`: rewrite to assert config file exists, no manifest sidecar, no events sidecar.
- Inline `apply_answers_warns_on_existing_vault_but_succeeds`: rewrite to use the config-file marker instead of `.temper/manifest.json`.
- Inline `apply_answers_creates_vault_structure`: rewrite to assert config-file existence + absence of vault-scaffold sidecars.
- Inline `no_interactive_defaults_and_applies`: same rewrite. **Beware:** this test currently runs `apply_answers` against a `tempfile::tempdir()` — if `apply_answers` now requires a working `temper-client`, the test needs either a mock client or feature-gating (`#[cfg(feature = "test-db")]`). Prefer the **mock-client** approach: introduce a thin `ContextEnsure` trait so the tests can stub out the server call. If that's too invasive, gate the test on `feature = "test-db"` and run it under `--features test-db` only. Surface the trade-off in the task subagent's prompt; let it decide based on the smallest delta.

**Step 1: Rework `apply_answers`**

- [ ] Delete the manifest.json, events.jsonl, and per-context directory writes.
- [ ] Add the auth-verification + server-side context-ensure block (async via `Runtime::new().block_on()`).
- [ ] Reword output messages.

**Step 2: Rewrite tests**

- [ ] Rewrite `crates/temper-cli/tests/init_test.rs`.
- [ ] Rewrite the 3 `apply_answers_*` inline tests in `commands/init.rs`.
- [ ] Confirm `check_test.rs` still passes (it likely uses `init::run` only to set up a config, which still works).

**Step 3: Verification**

```bash
cargo make check > /tmp/ch8-task2-check.log 2>&1
cargo nextest run -p temper-cli > /tmp/ch8-task2-cli.log 2>&1
```

**Step 4: Commit**

```bash
git add -A
git commit -m "cloud-only(ch8): rework commands/init.rs — drop local-vault scaffold

apply_answers no longer writes .temper/manifest.json, .temper/events.jsonl,
or per-context vault subdirectories. New flow: write config, verify auth,
ensure default + extra contexts exist server-side via temper-client.

Signature preserved (main.rs dispatch unchanged). Inline render_config_toml
tests untouched; apply_answers_* tests rewritten for new invariants.
Top-level tests/init_test.rs rewritten to assert config file existence
instead of vault-scaffold sidecars."
```

---

## Task 3: Phase B — `commands/status.rs` rework (projection staleness)

Replace the local file-count surface with a per-context projection-staleness report. For each configured context: report cursor freshness (Fresh/Stale/NotProjected/Skipped) + projected count (local md count under projection dirs) vs server count (one `temper-client` call per context).

**Surface to delete:**

- `pub fn count_md_files` — no replacement at this scope; new logic lives inline in `run`. If anywhere else in temper-cli still calls `count_md_files`, surface as Plan amendment (audit was clean per Task 1 triage; only `status_test.rs` consumes it, and that file dies).
- Top-level `crates/temper-cli/tests/status_test.rs` — delete the file.

**Surface to add to `commands/status.rs`:**

- New helper `count_projected_md_files(vault_root, owner, context)` — local; uses `Vault::doc_type_dir` and walks `.md` files under projection dirs. Keep narrow and inline-test in `commands/status.rs::tests`.
- Async block (via `tokio::runtime::Runtime::new().block_on()`) that:
  1. Builds a `TemperClient` from config.
  2. For each context, calls `projection::check_context_staleness(&client, &state_dir, ctx)` (existing API, already in projection.rs:113).
  3. For each context that is Fresh or Stale (i.e. has a cursor), calls a new `temper-client` API to get the server-side resource count.
- New `temper-client` API needed: `client.resources().count_for_context(ctx_id)` — verify presence at task-time. If it doesn't exist, the smallest delta is to add a new `GET /api/contexts/<id>/count` endpoint (server side) + client wrapper. If that's too much surface, the alternative is to call `client.resources().list({ context: Some(ctx_id), limit: 1 })` and read the `total` from the paginated response — most `ResourceListResponse` shapes carry a `total` count. **Verify at task-time and pick the smallest-delta path.** If neither path exists and adding new server surface is rejected, drop server-count from the report and stay cursor-only — surface as a Plan amendment.

**Output format:**

```
Temper Status
  Config: ~/.config/temper/config.toml
  Cloud:  https://temperkb.io  (auth: auth0, token: cached, expires 2026-06-01T…)

Contexts
  default       Fresh    [12 projected / 12 server]
  temper        Stale    [47 projected / 51 server]  → run `temper pull temper`
  writing       —        (not projected — run `temper pull writing`)
```

**Signature:**

- Current: `pub fn run(config: &Config, _verbose: bool) -> Result<()>`. **KEEP** — `verbose` is reserved; main.rs:96 dispatch is unchanged.

**Tests:**

- Delete `crates/temper-cli/tests/status_test.rs` entirely.
- Add inline tests in `commands/status.rs::tests` for `count_projected_md_files` (pure file-walk; can use `tempfile::tempdir()`).
- Async path tested manually or via e2e (do not add new e2e tests in this chunk; defer).

**Step 1: Delete `count_md_files` and rewrite `run`**

- [ ] Replace `count_md_files` with `count_projected_md_files` (narrower scope: projection dirs only).
- [ ] Build async block via `Runtime::new().block_on()` that fetches per-context staleness + server count.
- [ ] Render the new output format.

**Step 2: Delete top-level test file**

- [ ] `git rm crates/temper-cli/tests/status_test.rs`

**Step 3: Add inline tests**

- [ ] `#[cfg(test)] mod tests` in `commands/status.rs` for `count_projected_md_files`.

**Step 4: Verification**

```bash
cargo make check > /tmp/ch8-task3-check.log 2>&1
cargo nextest run -p temper-cli > /tmp/ch8-task3-cli.log 2>&1
```

**Step 5: Commit**

```bash
git add -A
git commit -m "cloud-only(ch8): rework commands/status.rs — projection staleness report

Local file-count surface replaced with per-context projection staleness
+ projected-count-vs-server-count via temper-client. Output groups by
context: Fresh, Stale (with pull hint), or NotProjected.

count_md_files deleted; new count_projected_md_files is narrower
(projection dirs only) and inline-tested. Top-level status_test.rs
deleted; new tests live inline in commands/status.rs."
```

---

## Task 4: Phase C — atomic deletion of orphan sync types + sync_cmd surface

Single atomic commit. All four surfaces die together because the unit-of-removal is "everything that supported `temper sync run` as a stub error". Partial removal leaves `Commands::Sync` with an empty body or dangling imports.

**Files modified/deleted in this task (one commit):**

1. `crates/temper-core/src/types/sync.rs` — delete lines 151-170 (`ResolutionType` + `SyncResolveRequest` definitions, plus the `// I6c — placeholder types` divider comment) and lines 246, 250 in the test module (the `ResolutionType::Local`/`::Merged` serde assertions). If the test module shrinks to nothing meaningful, leave the surviving tests in place; do not delete the `mod tests` block wholesale.
2. `crates/temper-core/src/types/mod.rs` — line 73 drop `ResolutionType`; line 75 drop `SyncResolveRequest`. The `pub use sync::{ … };` group stays.
3. `crates/temper-cli/src/commands/sync_cmd.rs` — `git rm` the file.
4. `crates/temper-cli/src/commands/mod.rs` — delete `pub mod sync_cmd;`.
5. `crates/temper-cli/src/cli.rs` — delete lines 87-92 (`/// Sync vault state with the cloud … Sync { #[command(subcommand)] action: SyncAction }`) and lines 406-417 (`#[derive(Subcommand)] pub enum SyncAction { Run { … } }`).
6. `crates/temper-cli/src/main.rs` — line 4 drop `SyncAction` from the import list; lines 316-321 delete the `Commands::Sync { action } => match action { … }` dispatch arm.
7. `tests/e2e/tests/cloud_writes_test.rs` — strip `sync_cmd::run` from the file-header doc-comment at line 10; delete Test 7 (`sync_run_errors_with_cloud_only_message`, lines ~920-953 including the comment header).

**Per-symbol audit before edits:** at task-time re-grep `rg 'SyncResolveRequest|ResolutionType|sync_cmd|SyncAction|Commands::Sync' --type rust` to confirm no consumer surfaced since plan-writing. If a new consumer exists, surface as Plan amendment.

**Step 1: Re-grep all symbols**

```bash
rg 'SyncResolveRequest|ResolutionType|sync_cmd|SyncAction|Commands::Sync' --type rust > /tmp/ch8-task4-symbols.log 2>&1
```

Expected hits: only the sites listed in this task. Any other hit is a blocker.

**Step 2: Apply all deletions atomically**

- [ ] Edit `crates/temper-core/src/types/sync.rs` (delete lines 151-170 and self-test references).
- [ ] Edit `crates/temper-core/src/types/mod.rs` (drop two symbols from `pub use sync::{…}`).
- [ ] `git rm crates/temper-cli/src/commands/sync_cmd.rs`.
- [ ] Edit `crates/temper-cli/src/commands/mod.rs` (drop `pub mod sync_cmd;`).
- [ ] Edit `crates/temper-cli/src/cli.rs` (delete `Sync` variant + `SyncAction` enum).
- [ ] Edit `crates/temper-cli/src/main.rs` (drop import + dispatch arm).
- [ ] Edit `tests/e2e/tests/cloud_writes_test.rs` (delete Test 7 + scrub doc-comment).

**Step 3: Verification**

```bash
cargo make check > /tmp/ch8-task4-check.log 2>&1
cargo nextest run -p temper-cli -p temper-core > /tmp/ch8-task4-units.log 2>&1
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -E 'test(cloud_writes_test::)' > /tmp/ch8-task4-e2e.log 2>&1
```

The e2e target check is narrowed to `cloud_writes_test::` because that's the only file modified in the e2e tier. Full e2e + embed tiers run in Task 8.

**Step 4: Commit (atomic)**

```bash
git add -A
git commit -m "cloud-only(ch8): atomic delete of sync_cmd surface + sync resolve placeholders

Last of the Chunk-7 deferred cleanups. Deletes:
- temper-core::types::sync::{ResolutionType,SyncResolveRequest} (orphan
  I6c placeholder types; only self-test consumers since added)
- types/mod.rs re-exports of the above
- commands/sync_cmd.rs (16-LOC stub returning a cloud-mode error)
- commands/mod.rs pub mod decl
- cli.rs Commands::Sync variant + SyncAction enum
- main.rs SyncAction import + dispatch arm
- cloud_writes_test.rs Test 7 (asserted the stub error) + file-header
  doc-comment reference

Atomic because the unit of removal is the whole 'temper sync run as
stub' surface — partial removal leaves dangling imports."
```

---

## Task 5: Phase D-1 — `temper-ingest` `hnsw` feature + dep cleanup

Feature/dep-only deletion. No Rust code is gated on `hnsw` (audited; zero `#[cfg(feature = "hnsw")]` blocks). `tantivy` and `hnsw_rs` are optional deps only reachable through the `hnsw` feature, which has no consumers since Chunk 6 deleted the local HNSW pipeline.

**Files modified in this task:**

1. `crates/temper-ingest/Cargo.toml` — delete lines 27 (`tantivy = …`), 28 (`hnsw_rs = …`), 42 (`hnsw = ["dep:tantivy", "dep:hnsw_rs"]`), and lines 7-8 (`[package.metadata.cargo-machete]` + `ignored = ["tantivy", "hnsw_rs"]` — the machete-ignore stanza is now meaningless).

**Step 1: Re-verify zero Rust consumers**

```bash
rg 'tantivy|hnsw_rs|hnsw\b' --type rust crates/ tests/ > /tmp/ch8-task5-grep.log 2>&1
```

Expected: zero hits (or only documentation/string-literal hits; assert manually).

**Step 2: Apply Cargo.toml edits**

- [ ] Delete the 4 lines listed above.

**Step 3: Verification**

```bash
cargo make check > /tmp/ch8-task5-check.log 2>&1
cargo machete > /tmp/ch8-task5-machete.log 2>&1
cargo nextest run -p temper-ingest > /tmp/ch8-task5-ingest.log 2>&1
```

`cargo machete` must stay clean (it was clean before — the machete-ignore stanza only suppressed warnings on the now-deleted deps).

**Step 4: Commit**

```bash
git add -A
git commit -m "cloud-only(ch8): drop orphan hnsw feature + tantivy/hnsw_rs deps from temper-ingest

Chunk 6 deleted the local HNSW pipeline; the hnsw feature in
temper-ingest's Cargo.toml became orphan (no #[cfg(feature = \"hnsw\")]
blocks survive). The tantivy + hnsw_rs optional deps were only reachable
through this feature.

Also drops the [package.metadata.cargo-machete] ignored stanza that
existed only to suppress machete warnings on these now-deleted optionals."
```

---

## Task 6: Phase D-2 — guidance docs sweep

Sweep CLAUDE.md family + `.claude/skills/temper/**` to match the cloud-only world. Specs/plans under `docs/superpowers/` and historical handoff notes under `docs/2026-*.md` are **untouched** (historical record).

**Files to inspect and edit (per plan-gate sweep audit):**

| File | Action |
|---|---|
| `/Users/petetaylor/projects/tasker-systems/temper/CLAUDE.md` | Rewrite — see specific edits below |
| `/Users/petetaylor/projects/tasker-systems/CLAUDE.md` | Verify; edit if hits |
| `/Users/petetaylor/projects/CLAUDE.md` | Verify; edit if hits |
| `/Users/petetaylor/.claude/skills/temper/SKILL.md` | Verify |
| `/Users/petetaylor/.claude/skills/temper/reference.md` | Strip `sync`/`push`/`graph build`/`graph index`/`add`/`doctor` CLI surfaces |
| `/Users/petetaylor/.claude/skills/temper/session-lifecycle.md` | Verify |
| `/Users/petetaylor/.claude/skills/temper/subagent-guidance.md` | Verify |
| `/Users/petetaylor/.claude/skills/temper/knowledge-base.md` | Verify |
| `/Users/petetaylor/.claude/skills/temper/guidance/fundamentals.md` | Rewrite "Two-tier resources: `temper add` vs `temper import`" para — `temper add` removed in Chunk 7 |
| `/Users/petetaylor/.claude/skills/temper/workflows/*.md` | Verify |
| `crates/*/CLAUDE.md` etc. | Most are auto-generated `claude-mem-context` blocks; leave unless manual sections reference local vault |
| `docs/guides/cloud-agents.md` | Verify |
| `docs/guides/*.md` other | Sweep at task-time |
| `README.md` (top-level temper) | Sweep at task-time if it mentions local-vault sync |

**Specific edits in `/Users/petetaylor/projects/tasker-systems/temper/CLAUDE.md`:**

- "Cloud mode operations" paragraph: rewrite to drop the "when `TEMPER_VAULT_STATE=cloud`…" framing. Cloud is now the only mode. Strip the `temper sync run` redirect mention (the surface is gone). Strip mentions of `manifest`-based sync. Keep the body-edit forms section (still accurate).
- "Resource deletion is always explicit" paragraph: update to drop the "implicit-delete-via-`rm`" framing — `rm` on a projected file is now just a local-cache miss. Recovery via `temper pull <context>` correct.
- "Code Quality Rules" bullet 4 ("Vault file IO and manifest IO live in `vault_backend/`"): DELETE the entire bullet. `vault_backend/` was removed in Chunk 4, `manifest_io` in Chunk 7. Replace with one line under bullet 3 ("Service layer owns SQL…"): "All vault writes route through `temper-client` to `temper-api` — there is no local-write surface."
- The "Sync protocol" bullet under "Key Patterns": DELETE.
- "TUI" / `TEMPER_VAULT_STATE` / `local-vault` mentions anywhere: DELETE or rewrite.

**Style discipline for the sweep:**

- Use grep to find every hit; edit in place to match the new world.
- Do NOT delete entire sections wholesale unless every paragraph is local-vault-specific. Most sections (auth, service layer, profile scoping, sqlx, params structs) are unaffected and stay verbatim.
- Where a paragraph mixes "local vs cloud" framing, simplify to the cloud-only narrative and remove the comparison.

**Step 1: Run the plan-gate sweep grep at task-time to catch drift**

```bash
rg -l --hidden 'local vault|local-vault|VaultState|TEMPER_VAULT_STATE|sync engine|temper sync|temper push|graph build|temper add|temper doctor' \
   /Users/petetaylor/projects/tasker-systems/temper/CLAUDE.md \
   /Users/petetaylor/projects/tasker-systems/CLAUDE.md \
   /Users/petetaylor/projects/CLAUDE.md \
   /Users/petetaylor/.claude/skills/temper/ \
   /Users/petetaylor/projects/tasker-systems/temper/docs/guides/ \
   /Users/petetaylor/projects/tasker-systems/temper/README.md \
   > /tmp/ch8-task6-grep.log 2>&1
```

Beware: "manifest" appears in many false-positive contexts (database row names, JSON manifest fields, etc.). Use additional context grep (`-B1 -A2`) to filter.

**Step 2: Apply edits per file**

- [ ] `/Users/petetaylor/projects/tasker-systems/temper/CLAUDE.md` — rewrite the four sections listed above.
- [ ] `/Users/petetaylor/.claude/skills/temper/reference.md` — strip removed CLI surfaces.
- [ ] `/Users/petetaylor/.claude/skills/temper/guidance/fundamentals.md` — rewrite two-tier paragraph.
- [ ] Other files — verify and edit only where hits exist.

**Step 3: Verification — second grep run, must come back clean**

```bash
rg -l --hidden 'local vault|local-vault|VaultState|TEMPER_VAULT_STATE|sync engine|temper sync|temper push|graph build|temper add|temper doctor' \
   /Users/petetaylor/projects/tasker-systems/temper/CLAUDE.md \
   /Users/petetaylor/projects/tasker-systems/CLAUDE.md \
   /Users/petetaylor/projects/CLAUDE.md \
   /Users/petetaylor/.claude/skills/temper/ \
   /Users/petetaylor/projects/tasker-systems/temper/docs/guides/ \
   /Users/petetaylor/projects/tasker-systems/temper/README.md \
   > /tmp/ch8-task6-grep-final.log 2>&1
```

False-positive `manifest` hits are acceptable (filter manually). Zero hits of the other patterns is the goal. If any remain, classify each before commit.

**Step 4: Commit**

```bash
git add -A
git commit -m "cloud-only(ch8): sweep guidance docs to match the cloud-only world

CLAUDE.md family + .claude/skills/temper/** rewritten where they
described local-vault, sync engine, manifest IO, vault_backend, or the
removed CLI surfaces (sync/push/graph build/add/doctor).

Specs and plans under docs/superpowers/ and historical handoff notes
under docs/ left untouched — historical record."
```

---

## Task 7: Pub-orphan sweep (broader-surface audit)

After Tasks 2–6 land, run the broader-surface audit per `feedback_sweep_time_audit_surface`. Pub items at lib-level are silenced under `clippy -D warnings`; the `cargo make check` green light is not proof of cleanliness.

**Audit surface (run each grep at task-time):**

1. **Cargo.toml `[features]` and `[dependencies]` audit:**

   ```bash
   for crate in crates/*/; do
     echo "=== $crate ==="
     awk '/\[features\]/,/^\[/' "$crate/Cargo.toml" 2>/dev/null
   done > /tmp/ch8-task7-features.log 2>&1
   cargo machete > /tmp/ch8-task7-machete.log 2>&1
   ```

   Look for: vestigial features that no longer gate anything (e.g. an `embed-download` feature whose only consumer was removed). machete should be clean post-Task 5.

2. **`pub mod` declaration audit:**

   ```bash
   rg -n '^pub mod \w+;' crates/ > /tmp/ch8-task7-pubmod.log 2>&1
   ```

   For each `pub mod X;` line, verify there's at least one consumer in or outside the crate. The orphan candidates to scrutinize post-this-chunk:

   - `crates/temper-cli/src/lib.rs`: was `pub mod manifest_io;` (Chunk 7 already removed); confirm gone. Any other once-orphan candidates?
   - `crates/temper-cli/src/commands/mod.rs`: confirm `pub mod sync_cmd;` is gone (Task 4); confirm `pub mod init;` and `pub mod status;` still live (rewritten in Tasks 2–3, still consumed by main.rs).
   - `crates/temper-cli/src/actions/mod.rs`: Chunk 7 removed `doctor`, `doctor_fix`, `sync`; confirm only `ingest` remains. Any others orphan?

3. **Struct-field audit:**

   ```bash
   rg -n 'pub struct \w+Config' crates/ > /tmp/ch8-task7-configs.log 2>&1
   ```

   Look for nested config struct fields whose type became dead. Chunk 6's `GraphIndexConfig` orphan is the canonical example — verify nothing similar survived from the local-vault era.

4. **`pub fn` / `pub struct` / `pub enum` at lib.rs level:**

   ```bash
   for crate in crates/*/; do
     name=$(basename "$crate")
     rg -n '^pub (fn|struct|enum|type)' "$crate/src/lib.rs" 2>/dev/null
   done > /tmp/ch8-task7-pubitems.log 2>&1
   ```

   For each `pub` at lib.rs, grep workspace for consumers; flag any that are zero-consumer outside the crate.

5. **Doc-comment audit:** `rg -n '<!--|TODO\(' docs/ crates/*/src/ | grep -i 'manifest\|sync\|graph\|vault'` to surface stale comment references.

**Step 1: Run all five audit greps**

- [ ] Capture outputs to `/tmp/ch8-task7-*.log` per the commands above.

**Step 2: Review each grep output and produce a verdict**

- [ ] For each candidate orphan, decide: DELETE (this task), KEEP (still consumed), or DEFER (out-of-scope for Chunk 8 — surface in PR B body).

**Step 3: Apply deletions**

- [ ] Edit each file to delete the verified orphans.

**Step 4: Verification**

```bash
cargo make check > /tmp/ch8-task7-check.log 2>&1
cargo machete > /tmp/ch8-task7-machete-final.log 2>&1
```

**Step 5: Commit**

If deletions were made:

```bash
git add -A
git commit -m "cloud-only(ch8): sweep pub-orphans after Phases A–D (broader-surface audit)

Per feedback_sweep_time_audit_surface — clippy under -D warnings
silences orphan pub items at lib level. Audit surface: Cargo.toml
features/deps, pub mod decls, struct fields, lib.rs pub items,
doc-comments.

[List specific orphans deleted, or 'no orphans found — audit log
captured at /tmp/ch8-task7-*.log for the record']"
```

If no deletions:

```bash
git commit --allow-empty -m "cloud-only(ch8): pub-orphan sweep — no orphans found

Broader-surface audit per feedback_sweep_time_audit_surface ran clean
after Phases A–D landed. Audit logs at /tmp/ch8-task7-*.log."
```

---

## Task 8: Final consolidated review + 4-tier verification + PR B open

Single task that runs the full 4-tier suite, dispatches the opus reviewer for consolidated code review, addresses any blockers (mid-task amendments allowed per Chunk 5/7 lesson), then merges `origin/main` and opens PR B.

**Step 1: Final 4-tier verification — all green required before PR open**

```bash
cargo make check > /tmp/ch8-task8-check.log 2>&1
cargo nextest run --workspace > /tmp/ch8-task8-workspace.log 2>&1
cargo make test-e2e > /tmp/ch8-task8-e2e.log 2>&1
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed > /tmp/ch8-task8-embed.log 2>&1
```

Each log must end with a green summary. Tail the last 10 lines of each to confirm.

**Step 2: Opus consolidated review**

Dispatch an opus subagent with this brief:

> Review the Chunk 8 implementation on branch `jct/cloud-only-vault-pr-b` — commits since `4630c76` (the Chunk 7 tip). Focus areas:
> - **Phase A**: `commands/init.rs` rework. Did the auth + context-ensure flow handle the `AuthChoice::None` case correctly? Is the async-via-`Runtime::new().block_on()` pattern correctly applied? Are the tests asserting the right invariants?
> - **Phase B**: `commands/status.rs` rework. Is the server-count call the right shape (new endpoint vs paginated-list-total)? Does the output format degrade gracefully when offline?
> - **Phase C**: atomic deletion. Verify nothing in `temper-core::types::sync` (the surviving wire types) was accidentally damaged. Verify the e2e test deletion didn't leave dangling helpers.
> - **Phase D**: docs sweep completeness. Spot-check 3 random skill files for stale mentions.
> - **Sweep**: pub-orphan audit thoroughness.
> Return READY_TO_MERGE / READY_WITH_FOLLOWUPS / BLOCKERS_FOUND.

**Step 3: Address blockers / followups inline**

- BLOCKERS_FOUND → fix immediately, commit, re-run Step 1.
- READY_WITH_FOLLOWUPS → fix if trivial (one commit); otherwise list in the PR body's "Reviewer notes" section.
- READY_TO_MERGE → continue.

**Step 4: Merge origin/main**

Per `feedback_merge_main_before_pushing_pr`:

```bash
git fetch origin > /tmp/ch8-task8-fetch.log 2>&1
git merge origin/main > /tmp/ch8-task8-merge.log 2>&1
```

If a merge conflict surfaces, resolve it (mid-task amendment), commit, re-run Step 1 (the conflict's resolution may have changed semantics).

**Step 5: Push and open PR B**

```bash
git push -u origin jct/cloud-only-vault-pr-b > /tmp/ch8-task8-push.log 2>&1

gh pr create --base main --head jct/cloud-only-vault-pr-b \
  --draft \
  --title "cloud-only(PR B): retire local vault — Chunks 3-8" \
  --body "$(cat <<'BODY'
## Summary

PR B of the cloud-only-vault deprecation. Spec:
\`docs/superpowers/specs/2026-05-21-cloud-only-vault-deprecation-design.md\`.

Chunks 3-8 land together as one atomic breaking change. PR A (#90)
merged the additive \`projection\` module foundation in advance; this
PR removes the local-vault machinery.

## What's removed

- Local-vault sync engine (\`actions/sync.rs\`, ~4400 LOC)
- \`VaultBackend\` + \`vault_backend/\` directory
- \`manifest_io\` + \`Manifest\`/\`ManifestEntry\` types
- \`temper-core::types::manifest\` + the I6c \`SyncResolveRequest\`/\`ResolutionType\` placeholders in \`types/sync.rs\`
- Local HNSW + graph-build pipeline + \`temper-ingest\` \`hnsw\` feature + \`tantivy\`/\`hnsw_rs\` optional deps
- \`temper sync\`/\`push\`/\`graph build\`/\`graph index\`/\`index\`/\`add\`/\`doctor\` CLI surfaces
- \`commands/sync_cmd.rs\` stub + \`SyncAction\` enum + \`Commands::Sync\` clap variant
- 1500+ LOC of doctor stack + 1370 LOC of \`add.rs\` + 426-LOC \`init.rs\` reshaped to ~150 LOC
- Local file-count \`temper status\` surface replaced with projection staleness report

## What's new

- \`temper resource create --from <path|url>\` — collapses \`temper add\` UX into \`resource create\` with kreuzberg extract
- Cloud-only \`temper init\`: config + auth + ensure default context server-side
- Cloud-only \`temper status\`: per-context projection staleness + projected count vs server count
- Projection staleness pre-flight on context-touching commands (warns if stale)

## Test plan

- [x] \`cargo make check\` green
- [x] \`cargo nextest run --workspace\` green
- [x] \`cargo make test-e2e\` green
- [x] \`cargo nextest --features test-db,test-embed\` green
- [ ] CI Embed job passes (verify after push)
- [ ] Manual smoke: \`temper init\` flow against a fresh config dir
- [ ] Manual smoke: \`temper status\` against multiple contexts
- [ ] Manual smoke: \`temper resource create --from <file>\` extracts and ingests
- [ ] Manual smoke: \`temper pull <context>\` materializes projection

## Reviewer notes

Atomic breaking change. No backward compat, no migration; users who
still have local-vault data should sync to cloud via the pre-PR-B path,
then re-pull as a projection.

Stacked deferrals retired:
- \`actions/sync.rs\`, \`manifest_io.rs\`, \`temper-core::types::manifest\` (Chunk 7)
- \`commands/sync_cmd.rs\` + \`SyncAction\` + \`Commands::Sync\` (Chunk 8 Task 4)
- \`init.rs\` local-vault scaffold (Chunk 8 Task 2)
- \`status.rs\` local file-count surface (Chunk 8 Task 3)
- \`temper-ingest\` \`hnsw\` feature + \`tantivy\`/\`hnsw_rs\` deps (Chunk 8 Task 5)

Linked task: \`2026-05-25-cloud-only-vault-chunk-8-final-docs-sweep-init-status-rework-small-cleanups-pr-open\`
BODY
)"
```

Mark as **Draft** initially; flip to ready after CI passes.

**Step 6: Acceptance**

- [ ] PR URL returned and recorded.
- [ ] CI watch — verify Embed job passes (the only tier with ONNX runtime; per Chunk 7's lesson, the Embed job catches workspace-feature-unification surprises).
- [ ] Flip Draft → Ready after CI green.

---

## Lessons to bake into the next chunk-style plan (if any)

This is the **final chunk** of the cloud-only-vault migration. No "next chunk" follows. Lessons for future migrations of similar scale:

- The "plan-committed-early" + "test-triage Task 1" + "consolidated review at end" trio worked uniformly across Chunks 4–8. Keep.
- Atomic-deletion tasks for tightly-coupled surfaces (Chunk 7 Task 7, Chunk 8 Task 4) are bisectable-friendly because the surface is "one unit of removal" — partial removal breaks the build. Keep.
- The plan-gate audit grew from one feedback (Chunk 4) to three feedbacks (Chunks 5, 6, 7) — each chunk added a dimension the prior chunk missed. By Chunk 8 the audit is comprehensive (both-ends + sweep-time-surface + name-collision). Future migrations should bake all three from the start.
- User-driven out-of-band `cargo make test-all` runs in Chunks 5-7 saved per-task full-suite time. Continue the pattern.
- Subagent-driven execution with consolidated review (per `feedback_subagent_review_cadence`) is the dominant cost-effective pattern at this plan size.
