# Unify `temper add` and `temper import`

**Date:** 2026-04-04
**Task:** 2026-04-04-unify-temper-add-and-temper-import
**Branch:** jcoletaylor/import-add-bug-html

## Problem

`temper add` and `temper import` are two commands that confuse users about their purpose. `add` was a fire-and-forget cloud upload (no vault file); `import` created vault-managed files with frontmatter and manifest tracking. The vault-managed path is the only one that matters now — the cloud-only `add` concept should have been removed when the vault model solidified.

## Decision

Merge both commands into a single `temper add`. The unified command behaves like current `import` (always writes a vault file, registers in manifest) with URL support from current `add`. Remove `import` entirely — no aliases, no backward compat (pre-alpha).

## Unified Command

```
temper add <path_or_url>
  --dir                    # batch directory mode
  --context <name>         # optional (derived from frontmatter in --doc-type auto)
  --doc-type <type>        # default: "resource", or "auto" for frontmatter detection
  --format <fmt>           # text or json (default: text)
  --force                  # override size guardrails
  --dry-run                # preview without uploading
  --ignore <regex>         # exclude files in batch mode
```

## Processing Paths

All paths write a vault file and register in the manifest.

1. **URL** (`http://` or `https://`) — fetch content, extract to markdown, write vault file, upload with embeddings
2. **UUID** — promote an existing cloud resource into the vault (fetch from cloud, write vault file, register manifest)
3. **Directory** (`--dir`) — walk directory, apply ignore/extension filters, preflight size check, batch import all files
4. **Single file** — extract content, write vault file with frontmatter, upload. Supports `--doc-type auto` to derive metadata from existing YAML frontmatter.

## Files Changed

### Deleted
- `crates/temper-cli/src/commands/import_cmd.rs` — logic absorbed into `add.rs`

### Rewritten
- `crates/temper-cli/src/commands/add.rs` — rebuilt from `import_cmd.rs` logic, plus URL-to-vault handling

### Modified
- `crates/temper-cli/src/cli.rs` — `Commands::Add` gets import's args (`--dry-run`, `--ignore`, optional `--context`), `Commands::Import` removed
- `crates/temper-cli/src/main.rs` — remove `Import` match arm, update `Add` arm to pass new args
- `crates/temper-cli/src/commands/mod.rs` — remove `import_cmd` module declaration
- `crates/temper-cli/src/actions/ingest.rs` — add `ingest_url_to_vault()` that fetches URL, extracts, writes vault file, and uploads (combining existing `ingest_url()` with vault-write logic)
- `README.md` — remove `import` row, update `add` description

### Unchanged
- `ingest.rs` shared utilities (extraction, chunking, embedding, `write_vault_file_and_register`)
- Directory walking utilities (already shared between both commands)
- `pull`, `remove`, `sync` commands (reference resources/manifest, not command names)

## Testing

- Merge test suites from both `add.rs` and `import_cmd.rs`
- URL detection tests stay
- UUID detection/promotion tests stay
- Directory walking, extension filtering, preflight check tests stay
- Context validation tests updated (context now optional like import)
- New: URL path produces a vault file (not just a cloud resource)
