# Temper Doctor Fix Enhancements — Design Spec

**Date:** 2026-04-05
**Branch:** jcoletaylor/temper-normalize-to-temper-doctor-fix
**Status:** Approved

## Summary

Enhance `temper doctor fix` to be the single vault health tool — subsuming `temper normalize`, adding filename slugification, file relocation, missing field inference, and manifest reconciliation. Remove the deprecated `temper normalize` command. Fix session and research creation commands to produce slugified filenames from the start.

## Context

After merging PR #16 (three-tier sync protocol, managed/open frontmatter model), the vault has 584 remaining issues: 484 missing slugs, 53 missing dates, 20 missing titles, 13 missing stages, 13 empty contexts, and 1 file with no frontmatter. Additionally, sync encounters 29 "resource already exists" conflicts (from prior duplicate-creation bug) and 11 "vault file not found" errors (stale manifest entries from filename format changes). The current `temper doctor fix` only handles field renames and two backfills (temper-created, temper-id). `temper normalize` is deprecated and handles some legacy scenarios but doesn't fix filenames or update the manifest.

## Architecture: Composable Fix Actions

### Action Model

Every mutation doctor fix can perform is represented as a `FixAction`:

```rust
enum FixAction {
    RenameField { path, old_key, new_key },
    SetField { path, key, value, reason },
    RenameFile { old_path, new_path, reason },
    RelocateFile { old_path, new_path, reason },
    UpdateManifest { temper_id, old_path, new_path },
    RemoveManifest { temper_id, reason },
}
```

### Pipeline

One walk over the vault. Each file is processed through a pipeline of composable fix functions. Each function takes `(file_path, frontmatter, config)` and returns `Vec<FixAction>`. Actions are collected across all files, sorted (field fixes → file moves → manifest updates), then applied. Dry-run mode collects and prints actions without applying.

### Fix Functions

**F1: Legacy field renames** (existing logic)
- Maps old names to temper-* names via `LEGACY_FIELD_MAP` (13 mappings)
- Emits `RenameField` actions

**F2: Infer missing required fields**
- `temper-id` — generate UUIDv7 (existing)
- `temper-created` — derive from `date` field, or filename date prefix, or file mtime as last resort
- `temper-type` — infer from directory path (`{context}/{doc_type}/...`)
- `temper-context` — infer from directory path; warn if relocation needed
- `title` — derive from filename: strip date prefix and extension, humanize the slug (`"my-feature-x"` → `"My Feature X"`)
- `slug` — slugify the title (or filename if no title); pattern: `^[a-z0-9][a-z0-9-]*$`
- `date` — (session/research only) extract from filename date prefix or `temper-created`
- `temper-stage` — default to `backlog` for tasks only
- Emits `SetField` actions with a `reason` string

**F3: Context/doctype relocation**
- Compare frontmatter `temper-context` + `temper-type` against actual directory location
- If mismatched, emit `RelocateFile` with warning-level reason
- Research lives under `{context}/research/` (e.g., `temper/research/`). Legacy `research/{context}/` paths should be relocated to `{context}/research/`
- Output shows `⚠` (yellow) for relocations to distinguish from simple field fixes

**F4: Filename slugification**
- Doc-type-specific rules:
  - **session, research**: `{date}-{slug}.md` (date prefix preserved, no em-dashes or spaces)
  - **task, goal, decision, concept**: `{slug}.md` (pure slug, no date prefix)
- If current filename doesn't match expected format, emit `RenameFile`
- Deduplication: if target path already exists, append `-2`, `-3`, etc.

**F5: Manifest reconciliation**
- For any `RenameFile` or `RelocateFile` action, look up the entry by `temper-id` in `.temper/manifest.json` and emit `UpdateManifest` with old and new path
- For manifest entries whose files don't exist on disk, emit `RemoveManifest`

### Execution Order

Actions are sorted before application:
1. `RenameField` / `SetField` — frontmatter fixes (file content changes)
2. `RenameFile` / `RelocateFile` — filesystem moves (after content is correct)
3. `UpdateManifest` / `RemoveManifest` — manifest bookkeeping (after files are in final locations)

## CLI Surface Changes

### Remove `temper normalize`

- Delete `commands/normalize.rs`, `actions/normalize.rs`, and associated tests
- Remove from command enum registration

### Enhanced `temper doctor` output

- Scan mode (no `fix`): same as today — `!` (auto-fixable), `✗` (manual)
- New: relocations show as `⚠` (yellow/warn level)

### Enhanced `temper doctor fix`

- `--dry-run` flag (existing) — collect and print all actions without applying
- Output groups actions by file:
  ```
  temper/task/2026-04-02-e2e-test-coverage-gaps.md
    ! rename field: doc_type → temper-type
    + set field: slug = "e2e-test-coverage-gaps" (inferred from filename)
    ⚠ rename file → e2e-test-coverage-gaps.md (slugify: remove date prefix for task)
    ↻ update manifest: path updated
  ```

### Fix creation commands

- `session save`: change filename format from `{date} — {slug}.md` to `{date}-{slug}.md`
- `research save`: change filename format from `{date} — {title}.md` to `{date}-{slug}.md`

This is a breaking change for new files going forward. Doctor fix will rename existing files to match, so the vault converges to one format.

## Testing Strategy

### Unit tests for fix functions

- Each F1–F5 function gets tests with synthetic frontmatter and paths
- Inference logic: title from filename, date from various sources, context from path
- Slug generation edge cases: unicode, consecutive hyphens, empty strings
- Doc-type-specific filename rules (session keeps date, task drops it)

### Integration tests for the pipeline

- Create a temp vault with known broken files (legacy fields, wrong directories, non-slug filenames)
- Run doctor fix in dry-run → assert correct actions collected
- Run doctor fix in apply mode → assert files renamed, frontmatter updated, manifest entries updated
- Run doctor scan after fix → assert zero auto-fixable issues remain

### Regression tests for creation commands

- `session save` produces `{date}-{slug}.md` format
- `research save` produces `{date}-{slug}.md` format
- Both produce valid frontmatter that passes doctor scan with zero issues

### Cleanup

- Verify normalize command and tests are removed cleanly (no dangling references)
- Existing doctor tests still pass (field renames, backfills)
