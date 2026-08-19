# Code Review Audit — 2026-03-31

Scope: `crates/temper-cli`, `crates/temper-api`, `crates/temper-core`, `crates/temper-client`, `crates/temper-embed`, and `packages/temper-cloud`.

---

## Executive Summary

The codebase is well-structured for a pre-alpha project. The crate boundaries are clean, `temper-core` is a pure vocabulary crate with no business logic, and the API follows a disciplined handler → service → SQL layering. The CLI has a good `commands/` → `actions/` separation that is already paying off for testability.

The primary issues are pattern duplication in the CLI (body-replacement, context-resolution, format-dispatch, validation constants), some SRP violations in the larger action functions, and a `workflow/` → `processing/` indirection layer in the TypeScript that adds surface area without value.

---

## 1. Pattern Duplication

### 1a. `replace_body` is duplicated across CLI commands

The function that preserves YAML frontmatter and replaces the markdown body is implemented identically in three places:

- `commands/session.rs` (lines 363–374)
- `commands/note.rs` (`merge_stdin_with_template`, lines 94–104)
- `commands/research.rs` (lines 106–116)

All three strip the `---…---` block and prepend new body content. The session version is named `replace_body`, the note version is `merge_stdin_with_template`, and the research version is again `replace_body`. The logic is identical.

**Recommendation:** Extract a single `vault::replace_body(existing, new_body) -> String` into `vault.rs` alongside the existing `parse_frontmatter` and `set_frontmatter_field`. All three callsites become one-liners.

### 1b. Context-from-CWD resolution is repeated in `main.rs`

The pattern:
```rust path=null start=null
let cwd = std::env::current_dir().unwrap_or_default();
let resolved = temper_cli::project::resolve_from_cwd(&cwd, &config.projects);
let context = context
    .as_deref()
    .or_else(|| resolved.map(|r| r.name.as_str()))
```

…appears 8+ times in `main.rs` (`Task::Create`, `Task::Move`, `Task::Done`, `Task::List`, `Task::Show`, `Goal::Create`, `Goal::List`, `Goal::Update`, `Events`, `Warmup`, `Research::Save`). The `ok_or_else` variant with "no context specified" appears 3 times.

**Recommendation:** Add a helper to `Config` or a free function:

```rust path=null start=null
fn resolve_context<'a>(
    explicit: Option<&'a str>,
    projects: &'a HashMap<String, ResolvedProject>,
) -> Option<&'a str>
```

And an `or_context_required()` variant that returns `Result<&str>`.

### 1c. `templates_dir` extraction is repeated

The snippet:
```rust path=null start=null
let templates_rel = config
    .templates_dir
    .strip_prefix(&config.vault_root)
    .map(|p| p.to_string_lossy().into_owned())
    .unwrap_or_else(|_| "templates".to_string());
```

…appears in `commands/session.rs`, `commands/note.rs`, `commands/research.rs`, and slightly differently in `actions/task.rs` (`templates_dir_str`). The task variant already factors it out, but the others don't use it.

**Recommendation:** Move `templates_dir_str` (or a method on `Config`) to a shared location and use it everywhere.

### 1d. Validation constants are duplicated between `actions/task.rs` create and move

`valid_stages`, `valid_modes`, and `valid_efforts` arrays with the same validation pattern appear in both `create()` (lines 142–161) and `move_task()` (lines 226–254). If a new mode or effort value is added, both locations must be updated.

**Recommendation:** Define these as module-level constants and extract a `validate_mode(m) -> Result<()>` / `validate_effort(e) -> Result<()>` / `validate_stage(s) -> Result<()>` set of helpers.

### 1e. JSON format dispatch pattern

The pattern:
```rust path=null start=null
if format == "json" {
    let json = serde_json::to_string_pretty(&data)?;
    println!("{json}");
} else {
    // text output
}
```

…appears in `commands/task.rs`, `commands/goal.rs`, `commands/session.rs`, `commands/note.rs`, `commands/research.rs`, `commands/warmup.rs`, `commands/events.rs`. The `format.rs` module already has `OutputFormat` and `output()`, but most commands don't use them — they inline the dispatch instead.

**Recommendation:** Consistently use the existing `format::OutputFormat::parse(format)` and `format::output()` throughout, or build a small `emit_json_or(data, || { text_output() })` helper that combines the common pattern.

### 1f. Event service SQL — query variants with optional filters

`temper-api/src/services/event_service.rs` has four near-identical SQL queries (lines 22–102) that differ only by whether `resource_id` and `event_type` are included in the WHERE clause. The CTE, column list, ORDER BY, and LIMIT/OFFSET are identical in every variant.

**Recommendation:** Build the query dynamically using a single base query with optional `AND` clauses, similar to what `search_service.rs` already does with `build_filter_clause`.

---

## 2. Single Responsibility Principle

### 2a. `commands/session.rs::save()` does too much

This ~137-line function (lines 23–137) handles:
1. Date computation
2. Project resolution from CWD
3. Path construction
4. Idempotent update-or-create logic
5. Template rendering with variable substitution
6. Stdin body merging
7. Frontmatter patching
8. File writing
9. JSON vs text output formatting
10. Event emission
11. Task linking (which itself reads/parses/mutates another file)

`link_session_to_task` (lines 140–221) is another 80+ lines that does frontmatter YAML list manipulation via string insertion, git branch detection, stage validation, and file writing — all in one function.

**Recommendation:** Split `save()` into:
- A pure `resolve_session_path(config, project, title) -> PathBuf`
- A `create_session_note(config, path, title, project, stdin) -> Result<String>` that returns content
- `link_session_to_task` should be further decomposed: the frontmatter list-append logic should be a `vault::append_frontmatter_list(content, key, value) -> String` utility (complementing the existing `set_frontmatter_field`).

### 2b. `commands/add.rs::run_directory()` combines orchestration with output

`run_directory` (lines 252–426) mixes async concurrency management (semaphore, spawn, Arc<Mutex>) with progress bar rendering, JSON output assembly, and error classification (duplicate vs. failure). This makes it hard to unit-test the orchestration without UI concerns.

**Recommendation:** Extract the per-file ingest logic into a separate function and the final summary computation into a pure function. The orchestration can then be tested by mocking the per-file operation.

### 2c. `actions/normalize.rs::process_file()` has too many concerns

`process_file()` (lines 123–250) handles ID backfill, stage migration, context-directory correction, slug consistency checking, effort field backfill, and conditional write — all in one function with multiple re-parses of frontmatter.

**Recommendation:** Structure as a pipeline of `NormalizePass` steps, each of which takes `(content, frontmatter, summary)` and returns `(content, summary)`. This makes each pass independently testable and makes it trivial to add future normalization steps.

---

## 3. Testability

### 3a. CLI commands that call `std::env::current_dir()` inline

Several commands in `main.rs` call `std::env::current_dir()` directly, making them untestable without filesystem manipulation. The `resolve_from_cwd` function itself is testable, but the integration in `main.rs` is not.

**Recommendation:** Thread CWD through from the top level (or via Config) rather than calling `std::env` inline.

### 3b. `link_session_to_task` shells out to `git`

`commands/session.rs` lines 195–205 call `std::process::Command::new("git")` to get the current branch. This makes the function untestable in environments without git or when the CWD isn't a git repo.

**Recommendation:** Extract the git-branch detection into a fallible helper function, and make `link_session_to_task` accept the branch as an optional parameter.

### 3c. Service functions take `&PgPool` directly

All `temper-api` service functions take `&PgPool`, which means tests require a real database. While `#[sqlx::test]` handles this, the logic (e.g., visibility scoping, pagination clamping, update coalescing) is tightly coupled to SQL.

This is actually fine for the current scale and is consistent with the "database is the authority" philosophy documented in the codebase. No change needed now, but note that if service logic grows more complex, introducing a repository trait boundary would make unit tests faster.

### 3d. Missing tests for several CLI commands

The following commands have no unit or integration tests:
- `commands/warmup.rs` (complex, aggregates multiple data sources)
- `commands/skill.rs` (generates a large template, checks hashes)
- `commands/status.rs`
- `commands/events.rs` (load_events is testable, but the format/output path isn't)

The `actions/` layer is better covered. The `ingest`, `search`, and `sync` action modules have good pure-function test suites.

**Recommendation:** Prioritize testing `warmup` and `skill` — both have testable pure functions embedded inside that could be extracted and tested.

---

## 4. Code Modularization

### 4a. `temper-core` — clean, no issues

The core crate is purely types + ID generation. No logic, no dependencies on other temper crates. The `types/mod.rs` re-export surface is well-organized. The feature-gated `#[cfg(feature = "web-api")]` for utoipa derives is a good pattern.

### 4b. `temper-cli` — the `commands/` → `actions/` split is good but inconsistent

The split was clearly introduced during a refactoring. `task.rs` and `goal.rs` in `commands/` are thin wrappers that delegate to `actions/`. However:

- `commands/session.rs` has substantial business logic inline (no matching `actions/session.rs`)
- `commands/note.rs` has business logic inline
- `commands/research.rs` has business logic inline
- `commands/warmup.rs` has significant data-aggregation logic inline

These should have corresponding `actions/` modules for consistency and testability.

### 4c. `temper-client` — well-decomposed sub-client pattern

The sub-client pattern (`resources::ResourceClient<'_>`, `search::SearchClient<'_>`, etc.) is clean. Each sub-client is a thin typed wrapper over `HttpClient`. The `http.rs` module's `map_status_to_error` pure function is well-tested. No issues here.

### 4d. `temper-embed` — clean, feature-gated correctly

The `extract` and `embed` features are properly gated. The embed module has good unit tests for pure functions (`l2_normalize`, `mean_pool`, `build_input_tensors`) and feature-gated integration tests. No issues.

### 4e. `temper-api` — handler → service layering is solid

Handlers are thin (extract params → call service → wrap in Json). Services own the SQL and business logic. The auth middleware is well-structured with a clear 8-step pipeline. No layering violations.

The `openapi.rs` module cleanly aggregates all path and schema registrations. The test that validates the spec structure is a good pattern.

---

## 5. TypeScript — `packages/temper-cloud`

### 5a. `workflow/` is a pure re-export layer with no value

Every file in `src/workflow/` is a one-line re-export from `src/processing/`:

- `workflow/chunk.ts` → `export { chunkText } from "../processing/chunk.js"`
- `workflow/embed.ts` → `export { embedTexts } from "../processing/embed.js"`
- `workflow/store.ts` → `export { buildStoreChunksQuery, ... } from "../processing/store.js"`

The only file with actual logic is `workflow/extract.ts`, which wraps kreuzberg.

**Recommendation:** Remove the `workflow/` directory. Import directly from `processing/` (or move `extract.ts` into `processing/`). The re-export layer adds import indirection, makes it harder to find the real implementation, and creates surface area for stale re-exports.

### 5b. `ingest.ts` — well-structured, but `getProfileId` should be in `middleware.ts`

`getProfileId` is used by `middleware.ts` but defined in `ingest.ts`. This creates a circular conceptual dependency where the middleware module imports from a business-logic module.

**Recommendation:** Move `getProfileId` into `middleware.ts` or a dedicated `profile.ts` module.

### 5c. `processing/embed.ts` — manual model download is fragile

The `ensureModel()` function (lines 31–52) manually downloads the ONNX model via `fetch()` and writes it to `/tmp`. This duplicates what `@huggingface/transformers` already does for the tokenizer (via `AutoTokenizer.from_pretrained` with `cache_dir`). If the CDN URL changes or requires authentication, this will break silently.

**Recommendation:** Use the HuggingFace `hf_hub` (or `@huggingface/hub`) client for model download as well, matching the Rust crate's approach.

### 5d. `processing/store.ts` — manual SQL parameter indexing is error-prone

`buildStoreChunksQuery` (lines 24–59) manually tracks `paramIndex` and constructs `$N` placeholders. This is a common source of off-by-one bugs when the schema changes.

**Recommendation:** Consider using a query builder or at minimum extract the parameter-index tracking into a small helper.

### 5e. `sync.ts` — `completeSyncRound` sequential updates

`completeSyncRound` (lines 170–197) runs one `UPDATE` per merged resource in a loop. For sync rounds with many resources, this will be slow.

**Recommendation:** Use a single batch update with `unnest()` or similar — the same pattern the Rust side's `sync_diff_for_device` SQL function uses.

### 5f. TypeScript test coverage is inconsistent

- `sync.test.ts` tests the pure `categorizeDiffRows` function — good.
- `workflow/chunk.test.ts`, `embed.test.ts`, `extract.test.ts`, `store.test.ts` cover the processing pipeline — good.
- `auth.test.ts` and `middleware.test.ts` cover auth — good.
- **Missing:** no unit tests for `ingest.ts` functions (`resolveContextId`, `resolveDocTypeId`, `insertResource`, `findByContentHash`). These are integration-level but the pure logic (e.g., "if context_name is set, use it instead of kb_context_id") should have unit-level coverage.

---

## 6. Cross-Crate Observations

### 6a. `temper-cli` error.rs and ids.rs are pure re-exports

```rust path=null start=null
// error.rs
pub use temper_core::error::{Result, TemperError};
// ids.rs
pub use temper_core::ids::{generate_id, generate_id_from_date};
```

This is fine for ergonomics — callsites write `crate::error::Result` instead of `temper_core::error::Result`. No action needed, just noting the pattern is intentional and consistent.

### 6b. Two different `VaultConfig` types exist

`temper-core/src/types/vault_config.rs` defines `VaultConfig` (server-side, stored in Postgres JSONB). `temper-cli/src/config.rs` also defines `VaultConfig` (local, from `temper.toml`). They have completely different fields and purposes. This is not a bug — they represent different domains — but may confuse contributors.

**Recommendation:** Rename the CLI-local one to `LocalVaultConfig` or `TomlVaultConfig` to make the distinction explicit.

---

## Priority Summary

| Priority | Issue | Effort |
|----------|-------|--------|
| High | 1a. Extract shared `replace_body` | Small |
| High | 1d. Shared validation constants | Small |
| High | 5a. Remove `workflow/` re-export layer | Small |
| Medium | 1b. Extract CWD context resolution | Small |
| Medium | 1c. Shared templates_dir helper | Small |
| Medium | 2a. Decompose `session::save()` | Medium |
| Medium | 4b. Add `actions/` modules for session, note, research | Medium |
| Medium | 5b. Move `getProfileId` out of `ingest.ts` | Small |
| Low | 1e. Consistent format dispatch | Medium |
| Low | 1f. Dynamic event query builder | Medium |
| Low | 2b. Separate orchestration from output in `add::run_directory` | Medium |
| Low | 2c. Pipeline `normalize::process_file` | Medium |
| Low | 3d. Tests for warmup, skill, status | Medium |
| Low | 5c. Use HuggingFace hub for model download | Small |
| Low | 5d. Parameter index helper for store.ts | Small |
| Low | 5e. Batch sync complete updates | Small |
| Low | 6b. Rename CLI VaultConfig | Small |
