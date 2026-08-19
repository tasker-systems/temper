# `temper graph build` — Refined Design Spec

**Date:** 2026-04-14
**Task:** `2026-04-11-knowledge-graph-ui-and-seeding-the-vault-for-relationships`
**Supersedes (Part B only):** `2026-04-11-open-meta-intentionality-and-graph-build-design.md`
**Depends on:**
- Sync metadata-only PATCH (done, PR #42)
- Frontmatter consolidation (done, PR #44)
- `KNOWN_OPEN_FIELDS` registry + server-side validation (done, PR #45)
- R7 knowledge graph foundations (done, PR #41)

**Mode:** build
**Effort:** medium (1-2 sessions, materially smaller than original spec's 4-session estimate)

---

## Why this spec exists

The 2026-04-11 spec defined "Part A: Known Open Fields Registry" and
"Part B: `temper graph build`" as a single effort. Part A shipped in
PR #45. Part B was deferred through the frontmatter consolidation work
and is now ready to implement.

In the interim, frontmatter consolidation landed a materially simpler
`Frontmatter` aggregate API (`crates/temper-core/src/frontmatter/`), the
`KNOWN_OPEN_FIELDS` registry is in place with canonical serialization,
and several assumptions in the original Part B design were revisited.
This spec captures the refined design and replaces the original spec's
Part B.

## Problem

The R7 knowledge graph infrastructure is built end-to-end — schema,
edge extraction, graph traversal, combined search — but the vault has
almost no relationship frontmatter. Without seeding, the graph is
empty, the downstream KG UI has nothing to display, and the value of
the R7 work is latent.

`temper graph build` is a deterministic, local, additive seeder. It
reads every markdown file in the vault, scans the body for explicit
reference syntax, resolves the references against a per-owner slug/UUID
map, and writes resolved references back into each file's `open_meta`.
On the next `temper sync run`, the server's existing `reconcile_edges`
path picks up the new declarations and materializes edges in
`kb_resource_edges`.

## First principles

### Deterministic, local, additive

- **Deterministic:** identical inputs produce identical outputs. No
  API calls, no LLM, no cross-file inference. Re-running is a no-op.
- **Local:** no network. No server dependency beyond the fact that the
  companion server-side change (below) must be deployed before edges
  will appear in `kb_resource_edges`.
- **Additive:** the command never removes existing frontmatter
  relationship values. If the user previously had
  `references: [foo]` and the current body scan finds `[bar]`, the
  result is `references: [foo, bar]`. Stale entries remain for the
  server's `reconcile_edges` to handle (it already supports
  unresolvable targets via the deferred-edges mechanism). Graph build
  is a seeder, not a validator or a cleaner.

### Owner boundary is a hard wall

**Edges never cross owner boundaries.** Owner is derived from the
first path segment of each vault file
(`<vault>/@me/<ctx>/<doctype>/<slug>.md` → owner `@me`). Slug→file and
UUID→file resolution maps are partitioned by owner. A reference from
a `@me/` file cannot resolve to a `@team-x/` resource even if the
profile has visibility into both. This is not a flag, not a tunable —
it is a structural property of the resolution step enforced by the
map partitioning. Even a slug or UUID crossing an owner boundary is
a data leak.

LLM-driven enrichment (future `temper graph infer`) will inherit the
same constraint.

## Command surface

```
temper graph build [--context <ctx>] [--dry-run] [-v]
```

| Flag | Default | Description |
|------|---------|-------------|
| `--context <ctx>` | all contexts | Scope the walk to a single context |
| `--dry-run` | false | Print report without writing files |
| `-v` / `--verbose` | false | Include per-file edge-level detail in the report |

### Subcommand group

`graph` is a new clap subcommand group, populated today with `build`
and leaving room for the near-future LLM-enrichment sibling
(`temper graph infer`, name tentative; `temper graph index` is on the
table as a rename TBD):

```
temper graph build     # this spec
temper graph infer     # future (LLM-driven, Karpathy-style wiki enrichment)
```

### Implementation location

| File | Role |
|------|------|
| `crates/temper-cli/src/commands/graph.rs` (new) | Clap `graph` subcommand group + `build` dispatch |
| `crates/temper-cli/src/actions/graph_build.rs` (new) | Pipeline: walk, scan, merge, write-back |
| `crates/temper-cli/src/commands/mod.rs` | Register the `graph` group |
| `crates/temper-cli/src/main.rs` | Wire into root command dispatch |
| `crates/temper-api/src/services/edge_service.rs` | Server-side companion (see below) |

Uses existing APIs:
- `temper_core::frontmatter::Frontmatter` — `try_from`, `value_mut()`,
  canonical serialize (verified present at
  `crates/temper-core/src/frontmatter/document.rs`)
- `temper_core::config` for vault root resolution
- `temper_core::types::graph::TargetRef` for reference parsing
- `pulldown-cmark` — **new workspace dependency** (verified not yet
  in workspace `Cargo.toml`). Added to `temper-cli/Cargo.toml` only,
  not `temper-core`, to keep core dep-lean.

No new shared types needed in `temper-core`.

## Pipeline

### Pass 1 — vault discovery

Walk the vault filesystem. For each `<vault>/<owner>/<context>/<doctype>/<slug>.md`:

1. Derive `owner` from the first path segment.
2. Parse frontmatter via `Frontmatter::try_from`. On parse failure,
   log at `tracing::debug!` and skip the file. No warnings — graph
   build is additive-only, not a validator.
3. Extract `temper-id` if present (may be absent on unsynced files).
4. Record into per-owner, per-context maps:
   - `slug_map: HashMap<Owner, HashMap<Context, HashMap<String, PathBuf>>>`
   - `uuid_map: HashMap<Owner, HashMap<Uuid, PathBuf>>` — UUIDs are
     globally unique, no context partitioning needed

Slug resolution mirrors the server's `edge_service::resolve_target`
behavior: **same-context first, then cross-context within the same
owner only if exactly one match exists.** Ambiguous cross-context
matches (2+ same-owner contexts contain the same slug) are skipped
silently with a `tracing::debug!`.

`--context <ctx>` filters the walk of files that will be *scanned* in
Pass 2, but the slug/UUID maps still include all same-owner files
across all contexts so that cross-context-same-owner references inside
the scanned context can still resolve.

### Pass 2 — body reference discovery

For each file in the (filtered) walk, parse the body with
`pulldown-cmark` (not the frontmatter — body text only). Walk the
event stream:

**`Event::Start(Tag::Link { dest_url, .. })`** — markdown link. Resolve
`dest_url`:

- `http://` / `https://` / `mailto:` / anchor-only (`#...`) → skip
- Relative path ending in `.md` → resolve against the scanning file's
  directory, canonicalize against vault root, look up stem in the
  same-owner slug map
- Absolute path ending in `.md` → resolve against vault root, same check
- Anything else (no extension, non-`.md`, bare filename without
  extension) → skip

**`Event::Text(text)`** (outside code contexts, which pulldown-cmark
gives us structurally) — regex-scan for:

- **Wikilinks** `[[slug]]`, `[[slug|display]]`, `[[slug#section]]`,
  `[[slug.md]]`, `[[slug|display#section]]` — strip `|display`,
  `#section`, `.md` suffix; look up in same-owner slug map. `folder/`
  prefixes rejected as ambiguous with vault layout.
- **Bare UUIDs** — regex for UUIDv4/v7 shape; look up in same-owner
  UUID map.

**`Event::Code(text)`** and fenced code blocks — emitted as distinct
events by pulldown-cmark. Ignored. No wikilink or UUID scanning inside
code.

Resolved references produce `(source_path, target_slug_or_uuid)`
tuples. Unresolved matches are dropped silently; `tracing::debug!` for
visibility. Results accumulate into
`discovered: HashMap<PathBuf, HashSet<String>>` keyed by source file.

### Pass 3 — merge + write-back

For each file in `discovered` with at least one resolved reference:

1. Read the file's frontmatter (cached from Pass 1 or re-read).
2. Read existing `open_meta.references` — may be `Vec<String>`,
   missing, or a different type. Missing or wrong type → treat as
   empty.
3. Compute `merged = existing ∪ discovered`, preserving `existing`
   order and appending new entries in insertion order (order in which
   Pass 2 encountered them, which is markdown reading order). Dedupe
   by string value.
4. If `merged == existing`, skip the file (no write).
5. Otherwise, mutate `open_meta.references` via
   `Frontmatter::value_mut()` structured access and write the file
   back via the canonical serialize path.
6. If `--dry-run`, accumulate what-would-change and skip the write.
7. Track changes for the report.

### Report output

Default (non-verbose):

```
temper graph build — 847 resources scanned (owner: @me)

Pass 1 (discovery):   847 files walked
Pass 2 (scanning):    412 references found
Pass 3 (merge):
  Files modified:     194
  References added:   259
  Already present:    153

Modified files:
  @me/temper/task/2026-04-10-knowledge-graph-foundations.md  (+3 references)
  @me/temper/research/2026-04-01-r7-vertex-edge...md          (+2 references)
  ...
```

`--dry-run` replaces "Modified" with "Would modify." `-v` adds a
per-file listing of the specific references added.

## Server-side companion change

One small non-CLI change makes `temper-goal` produce a graph edge:
`edge_service` needs to extract `ParentOf` from
`managed_meta.temper_goal` on task resources, not just from `open_meta`.

### Why server-side, not CLI write-back

`temper-goal` is the authoritative task-to-goal relationship and lives
in `managed_meta`. If the CLI wrote a redundant `parent: <goal-slug>`
into `open_meta`, it would drift whenever the user edits `temper-goal`.
Server-side derivation is the single source of truth: authoritative
field stays where it already lives, edge is auto-rederived on every
sync via `reconcile_edges`, no drift.

### Change

Rename `extract_declarations_from_open_meta` →
`extract_declarations_from_resource` and broaden the signature:

```rust
pub fn extract_declarations_from_resource(
    doc_type: DocType,
    managed_meta: &serde_json::Value,
    open_meta: &serde_json::Value,
) -> Vec<(EdgeType, TargetRef)>
```

Body unchanged for `open_meta` extraction. New logic: if
`doc_type == DocType::Task` and `managed_meta.temper_goal` is a
non-empty string, append
`(EdgeType::ParentOf, TargetRef::parse(&goal_slug))` to the result.

Callers (`reconcile_edges`, `extract_and_upsert_edges`) pass both
meta tiers to the new function. Provenance metadata stays
`"frontmatter"` — reconcile's diff logic is uniform because the new
declarations are re-derived on every call. No schema change, no
migration.

### Scope boundary

This companion change is the **only** server-side work in this spec.
Everything else (vault walking, body scanning, merge, write-back) is
CLI-local.

## Testing strategy

### Unit tests

**`crates/temper-cli/src/actions/graph_build.rs`:**

- Vault walk builds slug and UUID maps correctly, partitioned by
  owner and by context-within-owner, skipping unparseable files
- Slug resolution prefers same-context match over cross-context
- Slug resolution with ambiguous cross-context match (2+ same-owner
  contexts contain the same slug) resolves to `None` and skips
- Markdown link resolution: relative `.md`, absolute `.md`, URL / anchor
  / mailto skipped, bare filename, extensionless path
- Wikilink extraction: `[[slug]]`, `[[slug|display]]`,
  `[[slug#section]]`, `[[slug.md]]`, `[[slug|display#section]]`
- UUID extraction: valid v4/v7 shapes in body, not in code blocks, not
  in frontmatter
- `pulldown-cmark` code-block exclusion: fenced blocks, inline code,
  indented code blocks — no false positives
- Owner boundary: reference from `@me/foo.md` to a resource in a
  `@team-x/` slug map must not resolve even if the slug matches
- Merge semantics: union with existing, preserve order, dedupe, no
  removal of user-authored entries

**`crates/temper-api/src/services/edge_service.rs`:**

- `task_with_temper_goal_produces_parent_edge` — task with
  `managed_meta.temper_goal = "some-goal"` produces
  `(ParentOf, TargetRef::Slug("some-goal"))` in the declarations list
- Existing `extract_*` tests updated for the new signature

### Integration tests

**New `crates/temper-cli/tests/graph_build_test.rs`:**

- Fixture vault with a handful of contexts, mixed doc types, explicit
  wikilinks, markdown links, code-block false positives, and at least
  one cross-owner temptation
- Run `graph build --dry-run`, assert the report shape
- Run without `--dry-run`, assert file contents, assert idempotency
  (second run is a no-op)

### E2E test

**`crates/temper-e2e` (with `test-db`):**

- Seed a fixture vault, run `temper graph build`, then
  `temper sync run`
- Verify edges appear in `kb_resource_edges` with expected
  `source / target / edge_type`, `provenance = 'frontmatter'`
- Verify `temper-goal → ParentOf` is derived server-side on the same
  path without CLI write-back of `parent:` (the vault file's frontmatter
  should still have `temper-goal` in managed meta and `parent` NOT in
  open_meta)

## Scope boundary

### In scope

- `temper graph build` command in a new `graph` subcommand group
- Pass 1 vault walk building per-owner slug/UUID maps
- Pass 2 body scanning via `pulldown-cmark`: markdown `.md` links,
  wikilinks, bare UUIDs
- Pass 3 additive merge + write-back via `Frontmatter` canonical
  serialize
- `--context`, `--dry-run`, `-v` flags
- Server-side companion: `extract_declarations_from_resource` extended
  to emit `ParentOf` from `managed_meta.temper_goal` on tasks

### Out of scope

- **LLM-driven enrichment** — future `temper graph infer` (name
  tentative, `graph index` on the table as a rename TBD). Karpathy-style
  LLM-wiki enrichment approach, likely local Ollama model. Additive
  and non-blocking; will reuse the owner-boundary and additive-only
  principles established here.
- **Rename / move / cross-context-move cascade semantics** — there is
  no explicit model today for how resource movement (rename, doc-type
  change, context move, future team transfer) should cascade through
  references, graph edges, manifests, or the server-side edge tables.
  Tracked under the new `vault-change-management` goal.
- **`reconcile_edges` rename fragility** — today, when a previously-
  resolved edge's target becomes unresolvable (e.g., slug rename), the
  live edge is deleted and stashed in `kb_deferred_edges`, where it
  cannot be resolved because the target slug has changed. This is a
  real bug but it cannot be fixed in isolation — it's a symptom of
  the missing cascade model. Tracked under `vault-change-management`.
- **Reference validation / dead-link cleanup** — future `temper doctor`
  concern. Graph build is additive, not a validator.
- **JSON / structured report output** — add `--format json` only if a
  downstream consumer needs it.
- **Rate limiting or interactive confirmation on large vaults** — local
  filesystem walk has been fast enough in practice; revisit only if it
  becomes a problem.
- **Suppression mechanism** for "user removed a reference, don't re-add
  it" — unnecessary under additive-only semantics. No graph build pass
  removes anything.
- **Bare-word slug matching in body text** — originally listed as a
  sub-pass of Pass 2 in the 2026-04-11 spec, dropped here. Too noisy:
  slugs like `task`, `notes`, `auth` would produce phantom edges on
  every casual prose mention. LLM enrichment will cover the
  "implicit-mention" case properly.
- **Session ↔ task same-day heuristic** — originally listed as a
  sub-pass of Pass 1 in the 2026-04-11 spec, dropped here. Time
  proximity is too weak a signal for a pass called "deterministic."

## Effort

**1-2 sessions.** Materially smaller than the original spec's
4-session estimate because Part A is done, Pass 3 is dropped, Pass 1
moves to the server as a small extraction change, and Pass 2 sub-pass
4 is dropped.

**Session 1 (core implementation):**
- Clap `graph` subcommand group wired into main
- `actions/graph_build.rs` with walk, scan, merge, write-back
- Server-side `extract_declarations_from_resource` rename and
  `temper-goal → ParentOf` extraction
- Unit tests per pipeline step and for the server-side extraction
- Integration test with fixture vault
- E2E test for the full graph-build → sync → reconcile path

**Session 2 (only if review surfaces issues):** polish, edge cases,
possibly a real-vault byte-diff gate mirroring the pattern that
became load-bearing during frontmatter consolidation Session 3.

## Dependencies

- Sync metadata-only PATCH (done) — required so re-ingest is cheap
- Frontmatter consolidation (done) — provides the `Frontmatter`
  aggregate API for clean read/mutate/write
- `KNOWN_OPEN_FIELDS` registry (done) — canonical serialize order
- R7 graph foundations (done) — edge extraction, traversal, combined
  search

## Decisions made during refinement (2026-04-14)

For traceability. The original 2026-04-11 spec was coherent on its
own; these decisions refine it against the current state of the
codebase after frontmatter consolidation landed.

- **Drop Pass 1 session ↔ task same-day heuristic.** Too weak a
  signal; produces dense noise hairballs in the KG UI. Decision was
  previously made in a prior session but didn't make it into the
  written spec.
- **Drop Pass 2 sub-pass 4 (bare-word slug matching).** Precision
  killer; every prose mention of words like `task`, `notes`, `auth`
  would produce phantom edges.
- **Drop Pass 3 (heuristic search) entirely.** Self-reinforcing or
  spurious results; signal quality too uncertain. Future LLM
  enrichment will do this better.
- **Push `temper-goal → ParentOf` extraction to the server side**
  instead of CLI write-back. Single source of truth, no drift.
- **Use `pulldown-cmark`** for body parsing instead of hand-rolled
  regex. Correct by construction on code blocks, inline code, and
  fenced blocks.
- **Owner boundary is a hard wall.** Edges never cross `@owner/`
  boundaries. Even incidental slug/UUID leakage across owners is a
  data leak. Enforced by partitioning the slug/UUID maps.
- **Additive-only merge, unresolved refs skipped silently.** Graph
  build is a seeder, not a validator. `tracing::debug!` only.
- **Dry-run is opt-in, write is default.** Matches `temper doctor fix`.
- **Default `--context` scope is all contexts.** Local filesystem
  walk is fast enough; matches `temper doctor` sweep behavior.
- **Subcommand group from day one.** `temper graph build` under a
  `graph` group to make room for the future LLM-enrichment sibling.
