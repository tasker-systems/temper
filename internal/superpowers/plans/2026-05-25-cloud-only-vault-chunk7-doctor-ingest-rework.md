# Cloud-only vault — Chunk 7: doctor delete, `add`→`resource create --from` collapse, ingest slim, manifest/sync atomic deletion

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Apply the consolidated-review cadence (`feedback_subagent_review_cadence`): per-task verification stays terse (`cargo make check` + targeted nextest); the full 4-tier suite + opus code review runs ONCE at the end (Task 9).

**Goal:** Two-phase chunk that finally retires the stacked-deferral set accumulating since Chunk 4.

**Phase A — surface rework.** Delete the doctor stack outright (5 files: `actions/doctor.rs`, `actions/doctor_fix.rs`, `commands/doctor.rs`, `tests/doctor_test.rs`, `tests/doctor_fix_integration_test.rs`); collapse `temper add` into `temper resource create --from <path|url>` (delete `commands/add.rs` entirely after extending `resource create`'s surface); slim `actions/ingest.rs` to keep only the pure helpers consumed by cloud-mode paths (`compute_body_chunks`, `normalize_body_for_vault`, `build_frontmatter_from_resource`, `title_from_path`, `slug_from_title`, `derive_context_from_uri`, `parse_source_frontmatter`, `strip_frontmatter`, `ParsedFrontmatter`, `build_ingest_payload`, `BodyChunks`, `fetch_url_to_tempfile`).

**Phase B — consolidated atomic deletion.** After Phase A clears the last consumers, delete in one atomic commit: `actions/sync.rs`, `manifest_io.rs` (+ `pub mod manifest_io;` in `lib.rs`), `temper-core::types::manifest`, `temper-core::types::sync` (+ their `pub use` lines in `types/mod.rs`).

**Architecture:** Leaves inward, the shape Chunks 4 + 5 + 6 used. (1) Test triage as Task 1 with empty-commit verdict. (2) Delete the doctor stack — surface + actions + tests in one task because the surface is small (3 source files + 2 test files) and the dispatch arms can land together. (3) Extend `resource create` with `--from <path|url>` BEFORE deleting `add.rs`, so users have the cloud-only equivalent online when the old surface goes. (4) Delete `commands/add.rs` + CLI Add surface. (5) Slim `actions/ingest.rs` to drop the manifest-coupled tail (`ingest_file`/`ingest_url`/`write_vault_file_and_register` + local-vault path helpers). (6) Cleanup the `meta_test.rs:221` comment-only `Manifest` ref. (7) Phase B atomic deletion — one big commit because each file is the last consumer of the others. (8) Pub-orphan sweep with broader-surface audit per Chunk 6's `feedback_sweep_time_audit_surface` lesson. (9) Consolidated final review + 4-tier verification.

**Tech Stack:** Rust 2024 edition, cargo-make, cargo-nextest. No SQL changes; sqlx cache untouched.

---

## Plan gate — resolution

Applied **both** carry-forward feedbacks:

1. `feedback_plan_gate_audit_both_ends` (from Chunk 5's lesson): grep BOTH type symbols AND module function names.
2. `feedback_sweep_time_audit_surface` (from Chunk 6's lesson): include `Cargo.toml` features/deps + `pub mod` decls + struct fields, not just Rust symbol names.

### Type-symbol audit (`manifest_io|Manifest\b`)

11 source files surface:

| File | Verdict |
|---|---|
| `crates/temper-core/src/types/manifest.rs` | **DELETE** (Task 7, Phase B) |
| `crates/temper-core/src/types/sync.rs` | **DELETE** (Task 7, Phase B) |
| `crates/temper-core/src/types/mod.rs` | **MODIFY** (Task 7) — drop `pub use manifest::*;` and `pub use sync::*;` |
| `crates/temper-cli/src/manifest_io.rs` | **DELETE** (Task 7, Phase B) |
| `crates/temper-cli/src/lib.rs` | **MODIFY** (Task 7) — drop `pub mod manifest_io;` |
| `crates/temper-cli/src/actions/sync.rs` | **DELETE** (Task 7, Phase B) |
| `crates/temper-cli/src/actions/doctor.rs` | **DELETE** (Task 2) |
| `crates/temper-cli/src/actions/doctor_fix.rs` | **DELETE** (Task 2) |
| `crates/temper-cli/src/actions/ingest.rs` | **SLIM** (Task 5) — strip manifest-coupled tail |
| `crates/temper-cli/src/commands/doctor.rs` | **DELETE** (Task 2) |
| `tests/e2e/tests/meta_test.rs` | **CLEANUP** (Task 6) — comment-only ref at line 221 |

### Module-path + function-name audit (`actions::{sync,doctor,doctor_fix,ingest}|commands::doctor|sync_orchestration|normalize_all_entries`)

10 files surface beyond the deletion targets themselves:

| File | Verdict |
|---|---|
| `crates/temper-cli/src/main.rs` | **MODIFY** (Tasks 2, 4) — drop `Commands::Doctor` + `Commands::Add` dispatch arms |
| `crates/temper-cli/src/actions/show_cache.rs` | **KEEP** — uses `ingest::normalize_body_for_vault` (surviving helper); no edit |
| `crates/temper-cli/src/projection.rs` | **KEEP** — uses `ingest::{slug_from_title, build_frontmatter_from_resource, normalize_body_for_vault}` (surviving helpers); no edit |
| `crates/temper-cli/src/cloud_backend/translators.rs` | **KEEP** — uses `ingest::compute_body_chunks` (surviving helper); no edit |
| `crates/temper-cli/src/commands/add.rs` | **DELETE** (Task 4) |
| `crates/temper-cli/tests/doctor_test.rs` | **DELETE** (Task 2) |
| `crates/temper-cli/tests/doctor_fix_integration_test.rs` | **DELETE** (Task 2) |
| `tests/e2e/tests/cloud_writes_test.rs` | **KEEP** — references `commands::sync_cmd::run` (survives Chunk 7; deferred to a later cleanup); confirm Task 4 doesn't touch it |

### Sweep-time audit surface (Cargo.toml, `pub mod` decls, struct fields)

| Surface | Status |
|---|---|
| `crates/temper-cli/src/lib.rs` — `pub mod manifest_io;` | DELETE (Task 7) |
| `crates/temper-cli/src/actions/mod.rs` — `pub mod doctor;`, `pub mod doctor_fix;`, `pub mod ingest;`, `pub mod sync;` | DELETE `doctor`, `doctor_fix`, `sync` (Tasks 2, 7); KEEP `ingest` (Task 5 slims, doesn't delete) |
| `crates/temper-cli/src/commands/mod.rs` — `pub mod doctor;`, `pub mod add;` | DELETE both (Tasks 2, 4) |
| `crates/temper-core/src/types/mod.rs` — `pub mod manifest;`, `pub mod sync;`, `pub use manifest::*;`, `pub use sync::*;` | DELETE all 4 lines (Task 7) |
| `crates/temper-cli/src/cli.rs` — `Commands::Doctor`, `Commands::Add`, `DoctorAction` enum | DELETE (Tasks 2, 4) |
| `temper-cli/Cargo.toml` deps that may go transitive after add.rs deletion | VERIFY in Task 8 (likely `regex` if only `add.rs` used it for `--ignore`; check before removing) |
| `actions/sync.rs` inline tests (`normalize_all_entries_*` × 5 + `sync_orchestration` tests) | DELETE with parent file (Task 7) |

### Helper dependency map for `actions/ingest.rs` slim (Task 5)

| Helper | Consumer(s) outside `add.rs` | Slim verdict |
|---|---|---|
| `compute_body_chunks` | `cloud_backend/translators.rs` (×2) | **KEEP** |
| `normalize_body_for_vault` | `projection.rs`, `actions/show_cache.rs` | **KEEP** |
| `build_frontmatter_from_resource` | `projection.rs` | **KEEP** |
| `slug_from_title` | `projection.rs` (`add.rs` consumer dies in Task 4) | **KEEP** |
| `title_from_path` | `add.rs` only (consumer dies) | **KEEP** — needed by Task 3's `--from` path inference |
| `parse_source_frontmatter` / `strip_frontmatter` / `ParsedFrontmatter` | `add.rs` only (consumer dies) | **KEEP** — needed by Task 3's `--from` source-FM merge (deferred per scope note below; helpers stay available) |
| `build_ingest_payload` | `add.rs` only (consumer dies) | **KEEP** — the cloud `IngestPayload` builder; Task 3 uses it |
| `derive_context_from_uri` | `add.rs` only (consumer dies) | **KEEP** — needed for URL flow context inference |
| `fetch_url_to_tempfile` | `add.rs` only (consumer dies) | **KEEP** — Task 3 uses it for `--from <url>` |
| `BodyChunks` (struct) | Returned by `compute_body_chunks` | **KEEP** |
| `build_uri` | Only inside `ingest.rs` self + add.rs | **STRIP** — local-vault flavor; if unused after slim, delete |
| `ingest_file` | `add.rs` only | **STRIP** — replaced by Task 3 inline flow |
| `ingest_url` | `add.rs` only | **STRIP** — replaced by Task 3 inline flow |
| `write_vault_file_and_register` | `add.rs` only + 1 self-test | **STRIP** (manifest_io tail) |
| `build_vault_path` | `ingest.rs` self + `add.rs` | **STRIP** (local-vault path) |
| `dedup_vault_slug` | `add.rs` only | **STRIP** (local-vault dedup) |
| `build_frontmatter` (local-vault flavor) | `ingest.rs` self + `add.rs` | **STRIP** |
| `build_provisional_frontmatter` | `add.rs` only | **STRIP** |
| `infer_context_and_doctype` | `add.rs` only | **STRIP** (local-vault flavor) |

**STRIP candidates that are also `pub`**: clippy under `-D warnings` won't catch unused pub items inside the same crate (per `feedback_sweep_time_audit_surface`). Task 5 must delete each STRIP candidate explicitly. Task 8's broader-surface sweep re-verifies.

### Stacked-deferral retirement after Chunk 7

After this chunk lands, the stacked deferrals from Chunks 4 + 5 + 6 are fully resolved. Only Chunk 8 remains (docs sweep + PR open):

| Symbol | Status post-Chunk-7 |
|---|---|
| `actions/sync.rs` | Gone |
| `manifest_io.rs` (+ lib decl) | Gone |
| `temper-core::types::manifest` (+ mod decl) | Gone |
| `temper-core::types::sync` (+ mod decl) | Gone |
| `actions/doctor.rs` / `doctor_fix.rs` / `commands/doctor.rs` | Gone |
| `commands/add.rs` + CLI `Commands::Add` | Gone |
| `actions/ingest.rs` | Slim (cloud-only helpers) |
| `tests/e2e/tests/meta_test.rs:221` comment | Cleaned |

---

## Per-file decisions (Phase A — resolved with user before plan-writing)

- **`actions/doctor.rs` / `actions/doctor_fix.rs` / `commands/doctor.rs`**: DELETE all 5 files (3 source + 2 test). No cloud-only doctor rewrite in this chunk. Rationale: server validates on write (per `feedback_temper_sync_resource_show_failure`); "doctor scan" of frontmatter has no cloud equivalent. If a thin auth/connectivity-check `temper doctor` is wanted later, it's a fresh implementation on top of `temper-client` — schedule as a follow-on task post-Chunk 8 PR.
- **`commands/add.rs`**: DELETE entirely. Collapse the surface into `temper resource create --from <path|url>` to preserve the kreuzberg extract path under a single surface verb. UUID-promotion flow drops entirely (cloud-only equivalent: `temper pull <context>`). Directory batch flow drops (shell loop suffices: `find … | xargs -I {} temper resource create --from {} --type …`).
- **`actions/ingest.rs`**: SLIM. Keep the pure helpers consumed by cloud-mode paths (`compute_body_chunks`, `normalize_body_for_vault`, `build_frontmatter_from_resource`, `title_from_path`, `slug_from_title`, `derive_context_from_uri`, `parse_source_frontmatter`, `strip_frontmatter`, `ParsedFrontmatter`, `build_ingest_payload`, `BodyChunks`, `fetch_url_to_tempfile`). Strip the manifest-coupled tail and local-vault path helpers.
- **`init.rs` / `status.rs`**: Deferred to Chunk 8 per spec — neither is a Phase B blocker.

## Items explicitly NOT in this chunk (deferred)

- `temper-ingest` crate itself — server-side extract+embed pipeline stays for cloud ingest workflow. `temper-cli/src/extract.rs` re-exports `temper_ingest::extract::ExtractionResult`; that relationship is intact and orthogonal to `actions/ingest.rs`.
- `commands/init.rs` rework (spec: "drops local-vault scaffold") — Chunk 8.
- `commands/status.rs` rework (spec: "reports projection staleness") — Chunk 8.
- Cloud-only `temper doctor` (auth + connectivity check) — follow-on task post-Chunk 8.
- **Source-frontmatter merge on `--from`**: Task 3's `--from` extracts markdown body via kreuzberg and feeds it into the existing body-resolution flow. The legacy `add.rs` had an additional UX where YAML frontmatter in the source file would be parsed and selected fields merged into `open_meta`. This is **NOT in Task 3's minimum-viable scope**. The helpers (`parse_source_frontmatter`, `strip_frontmatter`, `ParsedFrontmatter`) survive the slim so this can be added as a follow-on without re-thrashing `ingest.rs`. If you want it in this chunk, surface during Task 3 and amend.

## Branch

`jct/cloud-only-vault-pr-b` — **do not branch**. Chunks 3–8 accumulate on the same branch; the PR opens after Chunk 8. Branch is at ~47 commits at Chunk 7 start.

## Execution discipline (carry forward from Chunks 3 + 4 + 5 + 6)

- **Subagent-driven execution** (per `feedback_prefer_subagent`), fresh sonnet implementer per task. **Consolidated review only** — opus reviewer fires once at Task 9 (per `feedback_subagent_review_cadence`).
- Each task ends with `cargo make check` green and a commit → branch stays bisectable.
- **Per-task verification is tightened**: `cargo make check` + targeted `-p` nextest only. Full workspace + e2e + embed tiers run **once** in Task 9.
- **Cargo output redirection**: always `> /tmp/foo.log 2>&1`. Never `2>&1 | tail` (silently produces 0-byte files under the harness — `feedback_cargo_output_redirection`).
- **Pub-orphan sweep audit (broader surface)**: Task 8 applies `feedback_sweep_time_audit_surface` — grep `Cargo.toml` features/deps + `pub mod` decls + struct fields, not just Rust symbol names.
- **Plan-committed-early**: Task 0 commits this plan before Task 1 starts.
- **Atomic deletion for tightly-coupled removals**: Task 7 (Phase B) lands all 4 file deletes + decl removals in ONE commit because each file is the last consumer of the others.
- **Mid-execution amendments are normal at this scale** (per Chunk 5's lesson #8). If a task surfaces a blocker mid-execution, follow the Chunk 5 pattern: ask the user, get an Option, amend the plan (separate commit), continue.

---

## Task 0: Commit this plan

Land the plan file before any code change so subsequent commits reference it. No code edit; one commit.

- [ ] **Step 1: Commit the plan file**

```bash
git add docs/superpowers/plans/2026-05-25-cloud-only-vault-chunk7-doctor-ingest-rework.md
git commit -m "cloud-only(ch7): record the chunk 7 implementation plan"
```

---

## Task 1: Test triage

Inventory every test file whose code path is touched by this chunk's deletions. Produce explicit delete/keep/repoint verdicts in an empty commit so the analysis is bisectable.

- [ ] **Step 1: Inventory affected test files via grep**

```bash
# Test files referencing deleting symbols (whole workspace)
rg -l 'actions::(doctor\b|doctor_fix|ingest|sync\b)|commands::(doctor\b|add\b)|manifest_io|Manifest\b|sync_orchestration|normalize_all_entries|ingest_file|ingest_url|write_vault_file_and_register' \
  --type rust > /tmp/ch7_test_triage.log 2>&1

cat /tmp/ch7_test_triage.log
```

- [ ] **Step 2: Produce a verdict table**

For each file the grep surfaces, decide:
- **Delete whole file** — entire test file's scope is the deleted machinery
- **Repoint** — file references a deleting symbol but the test still has value; update to cloud-mode equivalent
- **Keep (drop affected tests only)** — file is mostly fine, but specific tests inside need removal
- **Cleanup only** — false positive or comment-only ref (e.g. `meta_test.rs:221`)

**Expected verdicts (verify with the grep above; this is the working hypothesis):**

| File | Verdict | Lands in |
|---|---|---|
| `crates/temper-cli/tests/doctor_test.rs` (389 LOC, 11 tests) | Delete whole file | Task 2 |
| `crates/temper-cli/tests/doctor_fix_integration_test.rs` (168 LOC, 2 tests) | Delete whole file | Task 2 |
| `crates/temper-cli/src/actions/sync.rs` inline tests (5 `normalize_all_entries_*` + `sync_orchestration` tests) | Delete with parent | Task 7 |
| `crates/temper-cli/src/actions/doctor.rs` inline tests (if any) | Delete with parent | Task 2 |
| `crates/temper-cli/src/actions/doctor_fix.rs` inline tests (if any) | Delete with parent | Task 2 |
| `crates/temper-cli/src/actions/ingest.rs` inline tests (e.g. `write_vault_file_and_register` self-test at line 1264) | Drop tests that exercise STRIP candidates; keep tests that exercise KEEP helpers | Task 5 |
| `crates/temper-cli/src/commands/add.rs` inline tests (if any) | Delete with parent | Task 4 |
| `tests/e2e/tests/meta_test.rs:221` (comment-only ref) | Cleanup only | Task 6 |
| `tests/e2e/tests/cloud_writes_test.rs` (references `commands::sync_cmd::run` not the deleting set) | Keep — false positive | — |
| Any e2e test that exercises `temper add` directly | Verify — likely delete or repoint to `temper resource create --from` | Task 4 (re-scoped if found) |

Note: e2e tests that drive `temper add` via spawned binary need explicit verdict. Common patterns: `temper add <file>`, `temper add <url>`. If found, decide repoint (rewrite to `temper resource create --from`) vs delete.

- [ ] **Step 3: Grep for e2e tests driving `temper add`**

```bash
rg -n '"add"|temper.*add ' tests/e2e/tests/ 2>&1 | grep -v 'context add' > /tmp/ch7_e2e_add_callsites.log
cat /tmp/ch7_e2e_add_callsites.log
```

Each hit needs a verdict in the commit message. Note: `context add` (sub-command of `temper context`) is a different surface — exclude false positives.

- [ ] **Step 4: Commit the inventory (empty)**

```bash
git commit --allow-empty -m "$(cat <<'EOF'
cloud-only(ch7): test-triage inventory for chunk 7

Whole-file test deletions (Task 2):
  - crates/temper-cli/tests/doctor_test.rs (389 LOC, 11 tests)
  - crates/temper-cli/tests/doctor_fix_integration_test.rs (168 LOC, 2 tests)

Inline-test deletions (lands with parent):
  - crates/temper-cli/src/actions/sync.rs — normalize_all_entries_* (5) + sync_orchestration tests (Task 7)
  - crates/temper-cli/src/actions/{doctor,doctor_fix}.rs inline tests if any (Task 2)
  - crates/temper-cli/src/commands/add.rs inline tests if any (Task 4)

Drop-tests-only:
  - crates/temper-cli/src/actions/ingest.rs — write_vault_file_and_register self-test
    at line 1264 and any test exercising STRIP candidates; keep tests
    exercising KEEP helpers (Task 5)

Cleanup only:
  - tests/e2e/tests/meta_test.rs:221 — comment-only "Manifest" ref (Task 6)

E2E tests driving `temper add` directly: see /tmp/ch7_e2e_add_callsites.log;
each hit needs an explicit delete-or-repoint verdict in Task 4.

Keep / false positives:
  - tests/e2e/tests/cloud_writes_test.rs — references commands::sync_cmd::run
    which survives Chunk 7

Deferred (Chunk 8):
  - commands/init.rs, commands/status.rs reworks per spec
EOF
)"
```

---

## Task 2: Delete the doctor stack

5-file deletion (3 source + 2 test) + CLI surface (Commands::Doctor variant + DoctorAction enum + main.rs dispatch arm + commands/mod.rs decl + actions/mod.rs decls).

**Files:**
- Delete: `crates/temper-cli/src/actions/doctor.rs` (722 LOC)
- Delete: `crates/temper-cli/src/actions/doctor_fix.rs` (1758 LOC)
- Delete: `crates/temper-cli/src/commands/doctor.rs` (112 LOC)
- Delete: `crates/temper-cli/tests/doctor_test.rs` (389 LOC)
- Delete: `crates/temper-cli/tests/doctor_fix_integration_test.rs` (168 LOC)
- Modify: `crates/temper-cli/src/actions/mod.rs` — remove `pub mod doctor;` and `pub mod doctor_fix;`
- Modify: `crates/temper-cli/src/commands/mod.rs` — remove `pub mod doctor;`
- Modify: `crates/temper-cli/src/cli.rs` — remove `Commands::Doctor { ... }` variant + `DoctorAction` enum
- Modify: `crates/temper-cli/src/main.rs` — remove `Commands::Doctor { ... } => { ... }` dispatch arm

- [ ] **Step 1: Verify surface footprint**

```bash
rg -n 'actions::doctor|actions::doctor_fix|commands::doctor|Commands::Doctor|DoctorAction|temper_cli::commands::doctor|temper_cli::actions::doctor' \
  --type rust > /tmp/ch7_task2_surface.log 2>&1
cat /tmp/ch7_task2_surface.log
```

Expected hits: only files in the modify/delete list above. If any other file imports from these (e.g. an e2e test that spawns `temper doctor`), STOP and report.

- [ ] **Step 2: Delete the files**

```bash
git rm crates/temper-cli/src/actions/doctor.rs \
       crates/temper-cli/src/actions/doctor_fix.rs \
       crates/temper-cli/src/commands/doctor.rs \
       crates/temper-cli/tests/doctor_test.rs \
       crates/temper-cli/tests/doctor_fix_integration_test.rs
```

- [ ] **Step 3: Remove module declarations**

In `crates/temper-cli/src/actions/mod.rs`, delete the lines `pub mod doctor;` and `pub mod doctor_fix;`.

In `crates/temper-cli/src/commands/mod.rs`, delete the line `pub mod doctor;`.

Read each `mod.rs` first to confirm line numbers — declarations are alphabetical.

- [ ] **Step 4: Remove the `Commands::Doctor` variant and `DoctorAction` enum from `cli.rs`**

In `crates/temper-cli/src/cli.rs`:
- Delete the `Commands::Doctor { ... }` variant block (today around lines covering `action`, `context`, `format` fields)
- Delete the `pub enum DoctorAction { Fix { dry_run: bool } }` definition entirely (today after `ResourceAction`)
- Verify surrounding `Commands::` variants and the closing `}` stay intact

- [ ] **Step 5: Remove the dispatch arm from `main.rs`**

In `crates/temper-cli/src/main.rs`, locate `Commands::Doctor { action, context, format } => { ... }` (currently around lines 269-285). Delete the whole arm including its body. Leave the surrounding `match` arms intact.

- [ ] **Step 6: Run `cargo make check`**

```bash
cargo make check > /tmp/ch7_task2_check.log 2>&1; tail -60 /tmp/ch7_task2_check.log
```

Expected: 0 errors.

If `unresolved import` errors appear from any file other than the expected modifications, STOP — there's a missed consumer. Grep:
```bash
rg 'use crate::commands::doctor|use crate::actions::doctor|use crate::actions::doctor_fix' --type rust
```
Should print nothing after Task 2.

- [ ] **Step 7: Run targeted nextest**

```bash
cargo nextest run -p temper-cli > /tmp/ch7_task2_nextest.log 2>&1; tail -30 /tmp/ch7_task2_nextest.log
```

Expected: all surviving tests pass. The 13 deleted doctor tests should not appear in the output.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "cloud-only(ch7): delete doctor stack (5 files + CLI surface)"
```

---

## Task 3: Add `--from <path|url>` to `resource create` (TDD)

Extend `commands/resource.rs::create` and the clap surface in `cli.rs` so that `temper resource create --type <T> --from <path-or-url>` extracts content via kreuzberg (`temper-ingest`), then feeds the extracted markdown into the existing cloud ingest flow.

**Why TDD here, not delete-tasks:** This is the only task in Chunk 7 that adds behavior. Write a failing test first to lock the contract, then implement.

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` — add `from: Option<String>` field to `ResourceAction::Create`
- Modify: `crates/temper-cli/src/main.rs` — pass `from` through to `commands::resource::create`
- Modify: `crates/temper-cli/src/commands/resource.rs::create` — handle `--from` ahead of body resolution
- Add: A unit test in `crates/temper-cli/src/commands/resource.rs` (or a new integration test in `crates/temper-cli/tests/`) — covers `--from <file>` happy path

**Surface design:**
- `--from <path>` and `--from <url>` are detected by URL prefix (`http://` / `https://`)
- Mutually exclusive with `--body` (returns `TemperError::Config("--from cannot be combined with --body")`)
- Mutually exclusive with non-TTY stdin (returns `TemperError::Config("--from cannot be combined with piped stdin")`)
- File path validated to exist before extract attempt
- Extraction failures bubble up as `TemperError::Extraction(...)`

- [ ] **Step 1: Read existing `commands/resource.rs::create` end-to-end**

```bash
sed -n '160,300p' crates/temper-cli/src/commands/resource.rs > /tmp/ch7_task3_resource_create.log
cat /tmp/ch7_task3_resource_create.log
```

Confirm where body resolution happens (today: line 186-191, `resolve_body_source(body_flag, stdin_is_tty, stdin)`). Task 3's `--from` resolution lands AHEAD of that call.

- [ ] **Step 2: Read `actions/runtime.rs::with_client`**

```bash
rg -n -A 15 'pub fn with_client' crates/temper-cli/src/actions/runtime.rs
```

The `--from <url>` flow needs async (kreuzberg extract returns a future). Mirror the pattern from `commands/add.rs::run_url` (currently at line 411): use `with_client` (or `with_runtime` if it exists) for the async extract. The closure may discard `client` for the extract step; the cloud ingest later in `create` already runs through `with_client`.

- [ ] **Step 3: Write the failing test FIRST**

Add to `crates/temper-cli/src/commands/resource.rs` inline `#[cfg(test)]` mod (or create a new integration test file if the inline mod doesn't exist):

```rust
#[test]
fn from_and_body_are_mutually_exclusive() {
    // resolve_from_or_body errors when both --from and --body are provided.
    // Implementation: write a small helper resolve_from_input(from, body, stdin_is_tty)
    // that returns Result<Option<String>>; this test calls it directly.
    let err = resolve_from_input(Some("/tmp/x.md"), Some("@body.md"), true)
        .expect_err("should error on mutex");
    assert!(format!("{err}").contains("--from cannot be combined with --body"));
}
```

Run the test — it MUST fail to compile (the helper doesn't exist yet) or fail at runtime.

```bash
cargo nextest run -p temper-cli from_and_body_are_mutually_exclusive > /tmp/ch7_task3_test_red.log 2>&1; tail -20 /tmp/ch7_task3_test_red.log
```

Confirm: failure expected (red state). Do NOT continue to implementation until red is confirmed.

- [ ] **Step 4: Implement the `--from` flag in clap (`cli.rs`)**

In `crates/temper-cli/src/cli.rs`, add to `ResourceAction::Create`:

```rust
/// Source path or URL — extract markdown via temper-ingest and use as body.
/// Mutually exclusive with --body. URL detected by http:// or https:// prefix.
#[arg(long, conflicts_with = "body")]
from: Option<String>,
```

Use clap's `conflicts_with = "body"` for the static mutex (compile-time guarantee). Stdin-mutex stays runtime (Step 6).

- [ ] **Step 5: Pass `from` through in `main.rs`**

In `crates/temper-cli/src/main.rs`, locate the `ResourceAction::Create { ... }` destructure (around line 111) and add `from` to the bound variables. Pass it as a new arg to `temper_cli::commands::resource::create(...)`.

- [ ] **Step 6: Implement `resolve_from_input` + extend `commands/resource.rs::create`**

Add helper inside `commands/resource.rs`:

```rust
/// Resolve `--from <path|url>` into a body string via kreuzberg extraction.
/// Returns `Some(body)` if `from` is set; `None` otherwise. Errors when
/// `from` conflicts with `body` or with piped stdin.
async fn resolve_from_input(
    from: Option<&str>,
    body_flag: Option<&str>,
    stdin_is_tty: bool,
) -> crate::error::Result<Option<String>> {
    let Some(from) = from else { return Ok(None) };

    if body_flag.is_some() {
        return Err(crate::error::TemperError::Config(
            "--from cannot be combined with --body".to_string(),
        ));
    }
    if !stdin_is_tty {
        return Err(crate::error::TemperError::Config(
            "--from cannot be combined with piped stdin".to_string(),
        ));
    }

    let extracted = if from.starts_with("http://") || from.starts_with("https://") {
        let (tmp, _name) = crate::actions::ingest::fetch_url_to_tempfile(from).await?;
        crate::extract::extract_to_markdown(tmp.as_ref()).await?
    } else {
        let path = std::path::Path::new(from);
        if !path.exists() {
            return Err(crate::error::TemperError::Config(format!(
                "--from path does not exist: {from}"
            )));
        }
        crate::extract::extract_to_markdown(path).await?
    };

    Ok(Some(extracted.content))
}
```

Add `from: Option<String>` to `create`'s signature; resolve `--from` ahead of the existing `resolve_body_source` call:

```rust
pub fn create(
    config: &Config,
    doc_type: &str,
    title: &str,
    context: Option<&str>,
    goal: Option<&str>,
    mode: Option<&str>,
    effort: Option<&str>,
    slug: Option<&str>,
    body_flag: Option<String>,
    from: Option<String>,        // NEW
    format: &str,
) -> Result<()> {
    // ... existing setup ...
    let stdin_is_tty = std::io::stdin().is_terminal();

    // NEW: --from extraction (async; wrap in runtime helper)
    let from_body = if let Some(from_arg) = from.as_deref() {
        Some(crate::actions::runtime::with_client(|_client| {
            let from_owned = from_arg.to_string();
            let body_flag_owned = body_flag.clone();
            Box::pin(async move {
                resolve_from_input(Some(&from_owned), body_flag_owned.as_deref(), stdin_is_tty)
                    .await
                    .map(|opt| opt.unwrap_or_default())
            })
        })?)
    } else {
        None
    };

    let body_opt = if let Some(b) = from_body {
        Some(b)
    } else {
        crate::actions::body_source::resolve_body_source(
            body_flag.as_deref(),
            stdin_is_tty,
            std::io::stdin(),
        )?
    };
    // ... rest of existing implementation ...
}
```

**Adjust the closure shape to match `with_client`'s signature** — if `with_client` requires a `&temper_client::TemperClient` arg, the closure body just ignores it. If the codebase has a `with_runtime` helper that doesn't need a client, prefer that. Read `actions/runtime.rs` to pick.

- [ ] **Step 7: Run the test (green now)**

```bash
cargo nextest run -p temper-cli from_and_body_are_mutually_exclusive > /tmp/ch7_task3_test_green.log 2>&1; tail -20 /tmp/ch7_task3_test_green.log
```

Expected: pass. If still failing, the implementation has a bug — debug before continuing.

- [ ] **Step 8: Add a positive `--from <file>` happy-path test**

```rust
#[tokio::test]
async fn resolve_from_input_reads_file() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), "# Hello\n\nBody.\n").unwrap();
    let body = resolve_from_input(Some(tmp.path().to_str().unwrap()), None, true)
        .await
        .unwrap();
    assert!(body.is_some());
    assert!(body.unwrap().contains("Hello"));
}
```

This requires the `embed` or `extract` feature for `temper_ingest::extract` to function. Verify the test crate's feature flags; gate the test with `#[cfg(feature = "extract")]` if needed. Run:

```bash
cargo nextest run -p temper-cli --features test-db,extract resolve_from_input > /tmp/ch7_task3_happy.log 2>&1; tail -20 /tmp/ch7_task3_happy.log
```

If the `extract` feature isn't available at the CLI crate level (it lives in `temper-ingest`), gate the test as `#[cfg(all(test, feature = "<whatever-flag-pulls-temper-ingest-extract>"))]` or run it manually with the right features. **Don't gold-plate** — if gating is fiddly, lock the mutex test as the contract test and add an integration test under `tests/` instead.

- [ ] **Step 9: Run `cargo make check`**

```bash
cargo make check > /tmp/ch7_task3_check.log 2>&1; tail -60 /tmp/ch7_task3_check.log
```

Expected: 0 errors. Possible warnings around unused fields if the clap struct field `from` is on a code path not yet exercised by all tests — `#[expect(dead_code, reason = "...")]` is not the right answer; the test added in Step 8 should exercise it.

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "cloud-only(ch7): add --from <path|url> to resource create"
```

---

## Task 4: Delete `commands/add.rs` + CLI Add surface

After Task 3, `temper resource create --from` covers the single-file and URL flows. Delete `commands/add.rs` entirely and its CLI surface.

**Files:**
- Delete: `crates/temper-cli/src/commands/add.rs` (1368 LOC)
- Modify: `crates/temper-cli/src/commands/mod.rs` — remove `pub mod add;`
- Modify: `crates/temper-cli/src/cli.rs` — remove `Commands::Add { ... }` variant
- Modify: `crates/temper-cli/src/main.rs` — remove `Commands::Add { ... } => { commands::add::run(...) }` dispatch arm (lines ~330-353)
- Modify (if needed): e2e tests driving `temper add` → repoint to `temper resource create --from` OR delete (per Task 1's verdict)

- [ ] **Step 1: Verify surface footprint**

```bash
rg -n 'commands::add\b|Commands::Add|temper_cli::commands::add' --type rust > /tmp/ch7_task4_surface.log 2>&1
cat /tmp/ch7_task4_surface.log
```

Expected hits: only files in the modify/delete list. If an e2e test spawns `temper add ...`, that's the verdict applied from Task 1 — handle here.

- [ ] **Step 2: Delete the file**

```bash
git rm crates/temper-cli/src/commands/add.rs
```

- [ ] **Step 3: Remove `pub mod add;` from `commands/mod.rs`**

- [ ] **Step 4: Remove `Commands::Add { ... }` variant from `cli.rs`**

Today around lines covering `path`, `dir`, `context`, `doc_type`, `format`, `force`, `dry_run`, `ignore` fields. Delete the whole variant block including its docstring.

- [ ] **Step 5: Remove the `Commands::Add { ... } => { ... }` arm from `main.rs`**

Today around lines 330-353. Delete the whole arm.

- [ ] **Step 6: Repoint or delete e2e tests driving `temper add`**

For each hit from Task 1's e2e grep:
- If the test's intent was "verify add works", REPOINT to `temper resource create --from <file>` (the equivalent cloud-only surface).
- If the test was structurally about local-vault behavior that doesn't exist in cloud-only mode, DELETE.

Apply the verdict committed in Task 1. Commit message in this task must list every e2e test touched.

- [ ] **Step 7: Run `cargo make check`**

```bash
cargo make check > /tmp/ch7_task4_check.log 2>&1; tail -60 /tmp/ch7_task4_check.log
```

Expected: 0 errors. **Dead-code warnings expected** on `actions/ingest.rs`'s `ingest_file`, `ingest_url`, `write_vault_file_and_register`, `build_vault_path`, `dedup_vault_slug`, `build_provisional_frontmatter`, `infer_context_and_doctype`, `build_uri` (all are `pub` but `add.rs` was their sole consumer). Task 5 sweeps them.

If `unresolved import` errors appear, grep for stragglers:
```bash
rg 'use crate::commands::add|commands::add::' --type rust
```
Should print nothing.

- [ ] **Step 8: Run targeted nextest**

```bash
cargo nextest run -p temper-cli > /tmp/ch7_task4_nextest.log 2>&1; tail -30 /tmp/ch7_task4_nextest.log
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db > /tmp/ch7_task4_e2e.log 2>&1; tail -30 /tmp/ch7_task4_e2e.log
```

Expected: all pass. E2E tests repointed in Step 6 should pass under their new `temper resource create --from` shape.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "cloud-only(ch7): delete commands/add.rs + CLI Add surface"
```

---

## Task 5: Slim `actions/ingest.rs`

Strip the manifest-coupled tail and local-vault path helpers. Keep the pure helpers consumed by `cloud_backend/translators.rs`, `projection.rs`, `actions/show_cache.rs`, and `commands/resource.rs::create --from`.

**KEEP (verified consumers exist outside `ingest.rs`):**
- `compute_body_chunks`, `BodyChunks` — translators.rs (×2)
- `normalize_body_for_vault` — projection.rs, show_cache.rs
- `build_frontmatter_from_resource` — projection.rs
- `slug_from_title` — projection.rs
- `title_from_path` — Task 3's `--from` path (verify consumer exists post-Task 4)
- `derive_context_from_uri` — Task 3's `--from` URL flow (verify; if Task 3 didn't actually use it, mark as TBD-and-keep for the source-FM-merge follow-on)
- `parse_source_frontmatter`, `strip_frontmatter`, `ParsedFrontmatter` — kept for the deferred source-FM-merge follow-on; mark as `#[expect(dead_code, reason = "kept for source-frontmatter-merge follow-on")]` if no current consumer
- `build_ingest_payload` — if Task 3's create flow uses it; if not, mark as expect-dead-code or strip
- `fetch_url_to_tempfile` — Task 3's `--from <url>` path

**STRIP (verify no consumers outside the deletion set):**
- `ingest_file`, `ingest_url` — only `add.rs` (deleted in Task 4) used them
- `write_vault_file_and_register` — only `add.rs` + self-test
- `build_vault_path`, `dedup_vault_slug` — local-vault path helpers
- `build_frontmatter` (the local-vault flavor with `temper_dir` arg) — local-vault only
- `build_provisional_frontmatter` — local-vault only
- `infer_context_and_doctype` — local-vault flavor
- `build_uri` — if no surviving consumer

**Imports to remove from `ingest.rs`:**
- `use crate::manifest_io;` (and any per-fn `use crate::manifest_io;`)
- Imports of `Manifest`, `ManifestEntry`, `ManifestEntryState` — verify against final KEEP-list helpers

**Files:**
- Modify: `crates/temper-cli/src/actions/ingest.rs` — slim per above

- [ ] **Step 1: Re-verify each STRIP candidate has no surviving consumer**

```bash
for sym in ingest_file ingest_url write_vault_file_and_register build_vault_path dedup_vault_slug build_provisional_frontmatter infer_context_and_doctype build_uri; do
  echo "=== $sym ==="
  rg -n "ingest::$sym|::$sym\(" --type rust | grep -v 'crates/temper-cli/src/actions/ingest.rs'
done > /tmp/ch7_task5_strip_audit.log 2>&1
cat /tmp/ch7_task5_strip_audit.log
```

Expected: for each STRIP candidate, ZERO external consumers (only self-references inside `ingest.rs`). If any external consumer surfaces, STOP and amend the plan.

- [ ] **Step 2: Re-verify each KEEP candidate still has a consumer**

```bash
for sym in compute_body_chunks normalize_body_for_vault build_frontmatter_from_resource slug_from_title title_from_path derive_context_from_uri fetch_url_to_tempfile build_ingest_payload parse_source_frontmatter strip_frontmatter; do
  echo "=== $sym ==="
  rg -n "ingest::$sym|::$sym\(" --type rust | grep -v 'crates/temper-cli/src/actions/ingest.rs'
done > /tmp/ch7_task5_keep_audit.log 2>&1
cat /tmp/ch7_task5_keep_audit.log
```

For each KEEP candidate with ZERO external consumers post-Task-4: decide between
(a) `#[expect(dead_code, reason = "kept for source-frontmatter-merge follow-on")]` to preserve, OR
(b) strip and accept that the follow-on task re-adds it.

Default to (b) per `feedback_no_premature_backward_compat` — don't keep dead code "in case". The follow-on task can re-add cleanly from git history.

Document the final per-symbol verdict in the commit message.

- [ ] **Step 3: Strip the deleted functions and their helpers**

In `crates/temper-cli/src/actions/ingest.rs`:
- Delete each STRIP function body + signature + docstring
- Delete any private helpers used only by STRIP functions (re-grep to confirm)
- Delete the `use crate::manifest_io;` import
- Delete `Manifest`-family imports
- Delete inline tests that exercise STRIP candidates (e.g. the `write_vault_file_and_register` self-test at line 1264)

After the edit, the file should be substantially smaller. Read end-to-end before committing.

- [ ] **Step 4: Run `cargo make check`**

```bash
cargo make check > /tmp/ch7_task5_check.log 2>&1; tail -80 /tmp/ch7_task5_check.log
```

Expected: 0 errors.

The dead-code warnings from Task 4 should now be cleared (the offending pub items were deleted). New warnings may surface on items inside `actions/sync.rs` (it'll be even more orphan-shaped now — all consumed by `doctor.rs::normalize_all_entries` and `sync_cmd::run` and `cloud_writes_test.rs`'s reference to `sync_cmd::run`; `sync.rs` proper dies in Task 7).

If `unresolved import` from `commands/resource.rs::create` (Task 3 work), debug — likely a feature flag or signature drift.

- [ ] **Step 5: Run targeted nextest**

```bash
cargo nextest run -p temper-cli > /tmp/ch7_task5_nextest.log 2>&1; tail -40 /tmp/ch7_task5_nextest.log
```

Expected: all pass. Test count drops (inline tests removed).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "cloud-only(ch7): slim actions/ingest.rs to cloud-only helpers"
```

---

## Task 6: Cleanup `meta_test.rs:221` comment-only `Manifest` ref

Trivial follow-up. The comment was deferred from Chunk 5. Delete or rephrase the comment so the `Manifest\b` workspace-wide grep returns zero matches by Task 9.

- [ ] **Step 1: Read the line in context**

```bash
sed -n '215,230p' tests/e2e/tests/meta_test.rs
```

Today: `// Manifest: body_hash unchanged, managed/open hashes advanced.`

- [ ] **Step 2: Rephrase or delete**

Edit to remove the word `Manifest`. Likely replacement:
```rust
// After update: body_hash unchanged, managed/open hashes advanced.
```

The semantic content is preserved; just the word `Manifest` (now a deleted type) is removed.

- [ ] **Step 3: Verify zero `Manifest\b` hits in non-deletion files**

```bash
rg 'Manifest\b|manifest_io' --type rust > /tmp/ch7_task6_manifest_check.log 2>&1
cat /tmp/ch7_task6_manifest_check.log
```

Expected: only hits inside `actions/sync.rs`, `manifest_io.rs`, `temper-core/src/types/{manifest,sync}.rs`, `types/mod.rs`, `lib.rs` — all targeted by Task 7. Zero hits anywhere else.

- [ ] **Step 4: Run targeted nextest** (the meta_test runs in e2e tier)

```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db -E 'test(meta_test)' > /tmp/ch7_task6_nextest.log 2>&1; tail -20 /tmp/ch7_task6_nextest.log
```

Expected: pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "cloud-only(ch7): drop comment-only Manifest ref in meta_test.rs"
```

---

## Task 7: Phase B — atomic deletion of `sync.rs` + `manifest_io.rs` + `temper-core::types::{manifest,sync}`

After Tasks 2 + 4 + 5, the only remaining consumers of `manifest_io`, `Manifest`, `actions::sync::*` are inside the deletion set itself: `actions/sync.rs` references `manifest_io`, `manifest_io.rs` references `temper-core::types::Manifest`, etc. Each is the last consumer of the others. Delete in ONE atomic commit.

**Files:**
- Delete: `crates/temper-cli/src/actions/sync.rs` (4369 LOC)
- Delete: `crates/temper-cli/src/manifest_io.rs`
- Delete: `crates/temper-core/src/types/manifest.rs`
- Delete: `crates/temper-core/src/types/sync.rs`
- Modify: `crates/temper-cli/src/actions/mod.rs` — remove `pub mod sync;`
- Modify: `crates/temper-cli/src/lib.rs` — remove `pub mod manifest_io;`
- Modify: `crates/temper-core/src/types/mod.rs` — remove `pub mod manifest;`, `pub mod sync;`, `pub use manifest::*;`, `pub use sync::*;`

- [ ] **Step 1: Final pre-deletion audit — confirm zero external consumers**

```bash
# Type-symbol scan — should be zero hits outside the deletion set
rg 'Manifest\b|manifest_io|actions::sync\b' --type rust \
  | grep -v -E '(actions/sync\.rs|manifest_io\.rs|types/manifest\.rs|types/sync\.rs|types/mod\.rs|lib\.rs|actions/mod\.rs)' \
  > /tmp/ch7_task7_external_audit.log 2>&1
cat /tmp/ch7_task7_external_audit.log
```

Expected: zero lines. If anything surfaces, STOP — there's a missed consumer. Common false positives to ignore:
- `docs/superpowers/specs/...` (markdown design docs)
- `docs/superpowers/plans/...` (this plan, Chunk 4/5/6 plans)
- `docs/2026-03-31-user-workflow-analysis.md` (historical doc)

Filter with `--type rust` is required — the `grep -v` only excludes the deletion-set files.

- [ ] **Step 2: Delete the four files**

```bash
git rm crates/temper-cli/src/actions/sync.rs \
       crates/temper-cli/src/manifest_io.rs \
       crates/temper-core/src/types/manifest.rs \
       crates/temper-core/src/types/sync.rs
```

- [ ] **Step 3: Remove module declarations and `pub use` lines**

In `crates/temper-cli/src/actions/mod.rs`: delete `pub mod sync;`

In `crates/temper-cli/src/lib.rs`: delete `pub mod manifest_io;`

In `crates/temper-core/src/types/mod.rs`: delete:
- `pub mod manifest;`
- `pub mod sync;`
- `pub use manifest::*;`
- `pub use sync::*;`

Read each file first to confirm exact line numbers.

- [ ] **Step 4: Run `cargo make check`**

```bash
cargo make check > /tmp/ch7_task7_check.log 2>&1; tail -80 /tmp/ch7_task7_check.log
```

Expected: 0 errors.

If `unresolved import` errors surface from any non-deletion file, STOP — the audit in Step 1 missed something. Common locations to grep:
```bash
rg 'use temper_core::types::(Manifest|ManifestEntry|sync::|manifest::)' --type rust
rg 'use crate::manifest_io' --type rust
rg 'use crate::actions::sync' --type rust
```
All should be zero.

**Possible new dead-code warnings**: items in `temper-core::types::*` that were only used by the deleted sync/manifest modules. Task 8 sweeps.

- [ ] **Step 5: Run all four tiers** (Phase B is structurally invasive — verify end-to-end now, then again in Task 9)

```bash
cargo nextest run -p temper-cli > /tmp/ch7_task7_cli.log 2>&1; tail -30 /tmp/ch7_task7_cli.log
cargo nextest run -p temper-core > /tmp/ch7_task7_core.log 2>&1; tail -30 /tmp/ch7_task7_core.log
cargo nextest run -p temper-api --features test-db > /tmp/ch7_task7_api.log 2>&1; tail -30 /tmp/ch7_task7_api.log
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db > /tmp/ch7_task7_e2e.log 2>&1; tail -30 /tmp/ch7_task7_e2e.log
```

Expected: all pass. This is a deeper verification than other tasks because Phase B is large.

- [ ] **Step 6: Commit (atomic — single commit for the whole Phase B deletion)**

```bash
git add -A
git commit -m "$(cat <<'EOF'
cloud-only(ch7): atomic delete of sync.rs + manifest_io.rs + manifest/sync types

Phase B of Chunk 7: delete the four files and their decl lines in one
commit because each is the last consumer of the others.

Files removed:
  - crates/temper-cli/src/actions/sync.rs (4369 LOC)
  - crates/temper-cli/src/manifest_io.rs
  - crates/temper-core/src/types/manifest.rs
  - crates/temper-core/src/types/sync.rs

Decl removals:
  - crates/temper-cli/src/actions/mod.rs — pub mod sync;
  - crates/temper-cli/src/lib.rs — pub mod manifest_io;
  - crates/temper-core/src/types/mod.rs — pub mod manifest;, pub mod sync;,
    pub use manifest::*;, pub use sync::*;

Retires the stacked-deferral set accumulating since Chunk 4.
EOF
)"
```

---

## Task 8: Pub-orphan sweep with broader-surface audit

Tasks 2–7 deleted a lot of code. Apply `feedback_sweep_time_audit_surface`: grep `Cargo.toml` features/deps + `pub mod` decls + struct fields, not just Rust symbol names. Pub items inside the same crate are NOT flagged by clippy under `-D warnings`, so manual sweep is required.

**Likely sweep candidates (verify each — these are starting points, not a fixed list):**

- `temper-core::types::*` items that were only used by the deleted `manifest`/`sync` modules (e.g. helper types declared in adjacent files that the now-deleted modules consumed)
- `temper-cli/src/actions/runtime.rs` helpers (e.g. `require_device_id` — if its only callers were `manifest_io::load_manifest` indirect users, it's orphan now)
- `temper-cli/Cargo.toml` dependencies — verify each dep is still consumed (e.g. `regex` was used by `add.rs::--ignore`; if no other consumer exists, it goes transitive)
- `temper-core/Cargo.toml` dependencies — similar
- `temper-cli/src/extract.rs` — confirm still consumed (likely by Task 3's `--from` path)
- `commands/sync_cmd.rs` — the `temper sync run` cloud-guard command; surfaces `commands::sync_cmd::run` which is still referenced by `cloud_writes_test.rs`. Does it depend on the now-deleted `actions::sync`? Likely NO (it's the guard that errors with "cloud mode" message) — verify

- [ ] **Step 1: Run `cargo make check` and collect ALL warnings**

```bash
cargo make check > /tmp/ch7_task8_check.log 2>&1; cat /tmp/ch7_task8_check.log | tail -120
```

Look for `warning: unused import`, `warning: function is never used`, `warning: variant is never constructed`, `warning: field is never read`. Each is a candidate.

- [ ] **Step 2: Broader-surface audit (manual grep)**

```bash
echo "=== Cargo.toml dep audit ===" > /tmp/ch7_task8_broader.log
for dep in regex tempfile reqwest url; do
  echo "--- $dep ---" >> /tmp/ch7_task8_broader.log
  rg -l "(^use $dep|extern crate $dep|$dep::)" --type rust >> /tmp/ch7_task8_broader.log
done

echo "=== pub mod decl audit ===" >> /tmp/ch7_task8_broader.log
rg -n '^pub mod ' crates/temper-cli/src/lib.rs crates/temper-cli/src/actions/mod.rs crates/temper-cli/src/commands/mod.rs crates/temper-core/src/lib.rs crates/temper-core/src/types/mod.rs >> /tmp/ch7_task8_broader.log

echo "=== suspected pub-orphan helpers in runtime.rs ===" >> /tmp/ch7_task8_broader.log
rg -n '^pub (fn|struct|enum)' crates/temper-cli/src/actions/runtime.rs >> /tmp/ch7_task8_broader.log

cat /tmp/ch7_task8_broader.log
```

For each suspect, run a `rg -l` grep for external consumers (outside the file itself). If zero consumers, candidate for deletion.

- [ ] **Step 3: Verify each sweep candidate**

For every candidate identified in Steps 1 + 2, run a consumer audit:

```bash
rg "<symbol-name>" --type rust | grep -v "<defining-file>"
```

If ZERO external consumers, delete. If even one consumer exists, KEEP.

- [ ] **Step 4: Apply deletions**

For each verified orphan, edit the defining file to remove the item. Run `cargo make check` after each batch of related deletions to catch second-order orphans (an item's helper may become orphan once the item is deleted).

- [ ] **Step 5: Final-pass check**

```bash
cargo make check > /tmp/ch7_task8_final_check.log 2>&1; tail -80 /tmp/ch7_task8_final_check.log
```

Expected: 0 warnings, 0 errors.

- [ ] **Step 6: Commit**

If any sweep deletions happened:
```bash
git add -A
git commit -m "cloud-only(ch7): sweep pub-orphans after Phase A+B (broader-surface audit)"
```

If the audit found NO orphans (Chunk 6's "no sweep needed" pattern — but verify; Chunk 6's was incorrect per consolidated review):
```bash
git commit --allow-empty -m "$(cat <<'EOF'
cloud-only(ch7): pub-orphan audit — no sweep needed

Applied feedback_sweep_time_audit_surface — checked:
  - Cargo.toml deps (regex, tempfile, reqwest, url, ...)
  - pub mod decls across lib.rs / actions/mod.rs / commands/mod.rs / types/mod.rs
  - pub helpers in actions/runtime.rs

All surviving items still have at least one consumer. Empty commit marks
the verification was performed (Chunk 6's lesson: explicitly record
"audit done, zero orphans" so future chunks know it was checked).
EOF
)"
```

---

## Task 9: Consolidated final review + 4-tier verification

Per `feedback_subagent_review_cadence`, this is THE review for Chunk 7. Per-task subagents executed without code-review subagents chained; opus reviewer fires once here.

- [ ] **Step 1: Full 4-tier verification**

Run in sequence (sleep not needed between; let each complete):

```bash
cargo make check > /tmp/ch7_task9_check.log 2>&1; tail -30 /tmp/ch7_task9_check.log
cargo nextest run --workspace > /tmp/ch7_task9_workspace.log 2>&1; tail -30 /tmp/ch7_task9_workspace.log
cargo make test-e2e > /tmp/ch7_task9_e2e.log 2>&1; tail -30 /tmp/ch7_task9_e2e.log
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db,test-embed > /tmp/ch7_task9_embed.log 2>&1; tail -30 /tmp/ch7_task9_embed.log
```

All four MUST be green. If `access_gate_test` flakes under parallel e2e (Chunk 6's known environmental flake), re-run serial:
```bash
cargo nextest run --manifest-path tests/e2e/Cargo.toml --features test-db --test-threads=1 access_gate_test
```

- [ ] **Step 2: Final acceptance-criteria audit**

```bash
echo "=== Manifest\b workspace audit (should be zero) ===" > /tmp/ch7_acceptance.log
rg 'Manifest\b|manifest_io' --type rust >> /tmp/ch7_acceptance.log

echo "=== actions::sync workspace audit (should be zero) ===" >> /tmp/ch7_acceptance.log
rg 'actions::sync\b' --type rust >> /tmp/ch7_acceptance.log

echo "=== commands::add workspace audit (should be zero) ===" >> /tmp/ch7_acceptance.log
rg 'commands::add\b|Commands::Add\b' --type rust >> /tmp/ch7_acceptance.log

echo "=== commands::doctor workspace audit (should be zero) ===" >> /tmp/ch7_acceptance.log
rg 'commands::doctor\b|Commands::Doctor\b|DoctorAction\b' --type rust >> /tmp/ch7_acceptance.log

cat /tmp/ch7_acceptance.log
```

All four sections MUST be empty. Any hit is a blocker.

- [ ] **Step 3: Dispatch opus consolidated reviewer**

Use the superpowers:requesting-code-review skill via subagent dispatch. Include in the prompt:
- This is the consolidated review for Chunk 7 of an 8-chunk migration on `jct/cloud-only-vault-pr-b`.
- Reviewer should focus on: (a) Task 3's `--from` flag semantics (mutex with --body, mutex with piped stdin, error messages, async-runtime composition); (b) Task 5's ingest.rs slim — were any KEEP candidates incorrectly stripped, or vice versa?; (c) Task 7's Phase B atomic deletion — any stale `pub use` or `pub mod` lines missed?; (d) Task 8's pub-orphan sweep completeness (Chunk 6's lesson — the broader-surface audit must include Cargo.toml deps + pub mod decls + struct fields, not just Rust symbol names).
- Reviewer should NOT re-litigate the per-file delete-vs-rework verdicts (locked pre-plan with the user).
- Branch is `jct/cloud-only-vault-pr-b` at HEAD; review the chunk's commits (this chunk's commits are the ones beginning with `cloud-only(ch7):`).

Capture the review output in `/tmp/ch7_review.log` for the session note.

- [ ] **Step 4: Address review findings**

If READY_TO_MERGE: proceed to Step 5.

If READY_WITH_FOLLOWUPS: address minor findings inline (one commit per finding category) or document them in the session note as deferred items for Chunk 8.

If CHANGES_REQUIRED: surface to the user. Don't auto-fix structural issues — surface and ask.

- [ ] **Step 5: Final session note**

Save a session note via the temper CLI (per `feedback_temper_invocation`, use `temper` directly from PATH):

```bash
cat <<'EOF' | temper resource create --type session --title "Cloud-only vault Chunk 7 landed (doctor deleted, add collapsed into resource create --from, ingest slim, manifest/sync atomic deletion)" --context temper
## Goal

Chunk 7 of the 8-chunk cloud-only-vault deprecation. Retire the
stacked-deferral set accumulating since Chunk 4 by deleting the doctor
stack, collapsing `temper add` into `temper resource create --from`,
slimming `actions/ingest.rs` to cloud-only helpers, and atomically
deleting `actions/sync.rs` + `manifest_io.rs` + `temper-core::types::{manifest,sync}`.

## What Happened

[FILL IN — what each task did, any mid-execution amendments, the review verdict, total commit count]

## Decisions

[FILL IN — any judgment calls the implementer made, especially Task 5's KEEP-vs-STRIP edge cases on `parse_source_frontmatter`/`strip_frontmatter`/`build_ingest_payload` etc.]

## Connections

- **PR:** Not opened — Chunk 8 follows on the same branch
- **Branch:** `jct/cloud-only-vault-pr-b`
- **Spec:** `docs/superpowers/specs/2026-05-21-cloud-only-vault-deprecation-design.md`
- **Plan:** `docs/superpowers/plans/2026-05-25-cloud-only-vault-chunk7-doctor-ingest-rework.md`
- **Predecessor session:** `2026-05-25-cloud-only-vault-chunk-6-landed-hnsw-graph-build-deleted-search-reworked-to-cloud-only`
- **Goal:** `path-to-alpha`

## Next Steps

Chunk 8: docs sweep + `temper-ingest` server-side `hnsw` feature
cleanup + PR open. Also: `init.rs` (drop local-vault scaffold) and
`status.rs` (report projection staleness) reworks per spec, deferred
from Chunk 7.

Follow-on (post-Chunk 8 PR): cloud-only `temper doctor` for auth +
connectivity check (replacement for the deleted local doctor).
EOF
```

- [ ] **Step 6: Mark task done**

```bash
temper resource update 2026-05-25-cloud-only-vault-chunk-7-doctor-ingest-rework-consolidated-deletion-of-actions-sync-rs-manifest-io-rs-and-temper-core-types-manifest-sync --type task --stage done --context temper
```

---

## Acceptance criteria — final checklist

- [ ] `Manifest\b|manifest_io` workspace-wide grep returns zero hits in Rust files (only deletion-set markdown docs may reference)
- [ ] `actions::sync\b` workspace-wide grep returns zero hits
- [ ] `Commands::Doctor|Commands::Add|DoctorAction|commands::doctor\b|commands::add\b` returns zero hits
- [ ] `temper resource create --from <file>` extracts via kreuzberg and ingests successfully (manual smoke test or Task 3 happy-path test)
- [ ] `cargo make check` green
- [ ] `cargo nextest run --workspace` green
- [ ] `cargo make test-e2e` green
- [ ] `cargo nextest --features test-db,test-embed` green (Embed tier; only CI's Embed job has ONNX, so run locally with the right features per CLAUDE.md guidance)
- [ ] Opus consolidated review = READY_TO_MERGE or READY_WITH_FOLLOWUPS (documented in session note)
- [ ] Plan-gate question resolved in this plan's preamble (BOTH `feedback_plan_gate_audit_both_ends` + `feedback_sweep_time_audit_surface` applied — Task 8's broader-surface audit explicitly grepped Cargo.toml + pub mod decls + struct fields)
- [ ] No PR opened (PR B accumulates Chunks 3–8; opens after Chunk 8)
- [ ] Session note saved + task marked done
