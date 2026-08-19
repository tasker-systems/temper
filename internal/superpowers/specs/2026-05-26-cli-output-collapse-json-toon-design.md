# CLI output collapse to JSON | TOON

**Date:** 2026-05-26
**Status:** Approved (brainstorm complete; implementation plan to follow)
**Parent task:** `2026-05-26-post-cloud-only-qol-sweep-...` (Group F)
**Branch:** `jct/post-cloud-only-cli-output-json-toon`

## Goal

Collapse the `temper` CLI's three output formats (`Pretty`, `NoTty`, `Json`)
to two: `Json` and `Toon` (https://toonformat.dev). In cloud-only mode, the
cloud API response **is** the source of truth — the CLI's job is dispatch
and render, not data re-shaping. Rip out the bespoke per-command text
formatters and per-doctype JSON shapers that exist today.

## Background

Cloud-only mode landed in PR #94, retiring the local-write vault. The CLI
still carries display code from the local-vault era: column registries
(`output/columns.rs`), table renderers (`output/table.rs`), per-command
`format_text` helpers, and a 7-variant per-doctype JSON shape map in
`commands/resource.rs:72-159`. None of this is needed when the wire
type is the source of truth and Toon provides human-readable rendering.

Direction captured 2026-05-25 in memory
`project_cli_output_format_simplification`. Bundled in the post-cloud-only
QoL sweep task (Group F).

## Design Decisions

| Decision | Choice | Reason |
|----------|--------|--------|
| JSON shape strictness | Strict wire-type passthrough | Smallest scope; matches "cloud API IS the truth" framing literally |
| Default `--format` | Auto: Toon (TTY) / Json (non-TTY) | Mirrors current Pretty/NoTty auto-detection; lowest friction for humans + agents |
| Toon implementation | `toon-format` crate v0.5 (TOON v3.0 spec) | Most-downloaded, actively maintained, serde-native; org-owned repo; spec-current |
| `resource show` | Toon = raw markdown + frontmatter; Json = full API response | Body is the meaningful output for the `show` idiom |
| Non-API commands | All data-emitting commands get `--format`; action commands stay terse | Universal coverage without forcing JSON noise on every interaction |
| Migration strategy | Rip-and-replace, no deprecation shim | Per `feedback_no_premature_backward_compat`; project is one month old |

## Architecture

### Module layout

**New:** `crates/temper-cli/src/output/format.rs` (existing file, rewritten)

```rust
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq)]
pub enum OutputFormat {
    Json,
    Toon,
}

impl OutputFormat {
    /// Resolve an optional explicit `--format` value to a concrete format.
    /// `None` → TTY → Toon; non-TTY → Json.
    pub fn resolve(explicit: Option<&str>) -> Self;
}

/// Render any `Serialize` value in the chosen format.
pub fn render<T: Serialize>(value: &T, fmt: OutputFormat) -> Result<String, TemperError>;

/// `temper resource show` exception: Toon emits markdown body with
/// frontmatter; Json emits the full API response struct.
pub fn render_resource_show(
    metadata: &serde_json::Value,
    body: &str,
    fmt: OutputFormat,
) -> Result<String, TemperError>;
```

### Encapsulation invariant

**The `toon-format` crate is hidden behind our `output/format.rs`
abstraction.** Concretely:

- `toon-format` is listed *only* in `crates/temper-cli/Cargo.toml`. No
  other crate depends on it directly.
- `toon-format` types are imported *only* in `output/format.rs`. No call
  site references them.
- Our public surface from `output/format.rs` is the `OutputFormat` enum,
  the `render<T>` function, and the `render_resource_show` function.
  Nothing else escapes.

This way, swapping the Toon backend — to a competing crate or a
hand-rolled implementation — touches a single file.

### Files deleted at end of PR

- `crates/temper-cli/src/output/columns.rs` — per-doctype column registry
- `crates/temper-cli/src/output/table.rs` — markdown table + tab-delimited renderers
- `actions/search.rs::format_text` and its tests
- `commands/events.rs::format_event` and friends
- `commands/resource.rs:72-159` — 7-variant per-doctype JSON shape map
- `OutputFormat::Pretty` and `OutputFormat::NoTty` variants (and tests asserting them)

## Per-command Behavior

| Command | Today | After |
|---------|-------|-------|
| `temper search` | Pretty/NoTty: numbered text list. Json: passes wire type. | Toon + Json pass `Vec<UnifiedSearchResultRow>` directly. |
| `temper resource list` | Pretty: markdown table per-doctype columns. NoTty: tab-delimited. Json: re-shaped `temper-*` frontmatter dict. | Toon + Json pass `Vec<ResourceRow>` directly. |
| `temper resource show` | Pretty/NoTty: raw markdown + frontmatter. Json: `{ doc_type, slug, title, context, path, content }`. | Toon (default in TTY): raw markdown + frontmatter. Json: full API response struct (metadata + body). |
| `temper resource create` | Per-doctype JSON shape map (7 variants). | Toon + Json pass `CommandOutput<ResourceRow>` directly. |
| `temper resource update/delete` | Confirmation text. | Default: confirmation text. `--format json`: `{ status: "ok", slug, id }`. |
| `temper events` | Tab-separated text. Json: passes wire type. | Toon + Json pass `Vec<Event>` directly. |
| `temper auth status` | JSON-only (no `--format` flag). | Add `--format` flag. Toon: human-readable status. Json: pass `AuthStatus` directly. |
| `temper auth login/logout` | JSON-only. | Add `--format` flag. Default: confirmation text. `--format json`: as today. |
| `temper context list` | Text-only aligned columns. | Add `--format` flag. Toon: today's list. Json: `Vec<String>`. |
| `temper status` | Text-only. | Add `--format` flag. Toon: today's staleness report. Json: `{ contexts: [{ name, staleness, projected, server }] }`. |
| `temper warmup` | Has `--format` using old enum. | Migrate to new enum; passthrough. |
| `temper init` | Interactive prompts; `--no-interactive` exists. | Keep interactive in TTY. Non-interactive + `--format json`: emit `{ vault_path, contexts, auth }` summary. |
| `temper doctor` / `check` | Text status report. | Add `--format` flag. Json: `{ checks: [{ name, status, message }] }`. |
| `temper config edit` | Opens `$EDITOR`. | Unchanged. |

## Implementation Approach

**Approach B: foundation → per-command → cleanup.** One PR with 11 discrete
commits; bisectable.

### Commit sequence

**C1 — Foundation.** Add `toon-format = "0.5"` to `temper-cli/Cargo.toml`.
Add `OutputFormat::Toon` variant to existing enum (`Pretty`/`NoTty` remain
working until C11). Add `render<T: Serialize>(&T, OutputFormat) ->
Result<String>` helper handling Json + Toon. Update
`OutputFormat::resolve(Auto)` to pick Toon (TTY) / Json (non-TTY). Add
`output/format.rs` unit tests for the new render path on a representative
`Serialize` fixture.

**C2 — `events`.** Drop `format_event` in `commands/events.rs:42-83`.
JSON + Toon pass `Vec<Event>` directly. Smallest API-shaped surface;
validates the foundation in a real command.

**C3 — `auth status/login/logout`.** Add `--format` flag to all three.
Toon renders the existing `AuthStatus` struct; Json keeps current shape.

**C4 — `context list`.** Add `--format` flag. Toon: today's aligned list.
Json: `Vec<String>`.

**C5 — `status`.** Add `--format` flag. Define new JSON shape:
`{ contexts: [{ name, staleness, projected, server }] }`. Toon: today's
staleness report.

**C6 — `search`.** Drop `actions/search.rs::format_text`. JSON + Toon
pass `Vec<UnifiedSearchResultRow>` directly. Delete the
`format_text_includes_score_and_origin` test; keep `build_search_params`
and `truncate` tests (unrelated to format).

**C7 — `resource list`.** Drop `render_server_rows`,
`row_to_frontmatter_value`, the call into `extract_row`. JSON + Toon
pass `Vec<ResourceRow>` directly. This is the commit that lands the
"JSON gets noisy" wire-type passthrough.

**C8 — `resource show`.** Special-case implementation. Toon (default in
TTY): raw markdown body with frontmatter as today. Json: composite
shape `{ ...ResourceRow, content: "<markdown body>" }` (metadata struct
joined with body string — the two come from separate API calls today:
`GET /api/resources/{id}` + `GET /api/resources/{id}/content`). New
`render_resource_show(&ResourceRow, body: &str, fmt)` helper in
`output/format.rs`.

**C9 — `resource create/update/delete`.** Drop the 7-variant per-doctype
JSON shape map at `commands/resource.rs:72-159`. Default action-command
output is confirmation text. `--format json` emits `{ status: "ok",
...wire-type-fields }`.

**C10 — `warmup` + `doctor` + `init`.** Migrate `warmup` to new enum.
Add `--format` to `doctor`/`check`. `init` stays interactive in TTY;
non-interactive + `--format json` emits final summary.

**C11 — Cleanup.** Delete `OutputFormat::{Pretty, NoTty}` variants.
Delete `output/columns.rs`, `output/table.rs`, and their tests. Drop
now-unused imports. Update `CLAUDE.md` "Build & Test" section if it
references format flags. Update any clap help text mentioning
Pretty/NoTty.

## Testing Strategy

- **C1 foundation:** Unit tests for `FormatChoice::resolve()` (TTY vs
  non-TTY), unit tests for `render<T>` on a representative Serialize
  fixture verifying both Json and Toon outputs are non-empty and
  parseable.
- **C2–C10 per-command:** Each commit either updates an existing test or
  adds one asserting wire-type passthrough in JSON and a minimal
  contains-check for Toon (exact-string Toon assertions are brittle).
- **C11 cleanup:** Delete all `Pretty` / `NoTty` assertions left behind.
  Verify no test references the deleted columns/table modules.

## Error Handling

No new error paths. Toon serialization can fail (it's serde-backed);
errors bubble up as `TemperError`. `--format` flag parsing reuses clap's
`ValueEnum`. Backward-incompatible `--format pretty` / `--format no-tty`
return a standard clap error with the new accepted values listed.

## Breaking Changes (Accepted)

Per `feedback_no_premature_backward_compat` (project is one month old):

- `--format pretty` becomes a clap error.
- `--format no-tty` becomes a clap error.
- `temper resource list --format json` returns the full `ResourceRow`
  shape (with `body_hash`, `managed_hash`, `kb_context_id`, etc.), not
  the `temper-*` frontmatter dict. Scripts piping into `jq` with the old
  shape break.
- `temper resource create --format json` returns wire fields (`slug`,
  `id`, etc.), not the per-doctype `temper-slug`/`temper-title` shape.

These breaks are accepted: the project has no public consumers, the new
shapes are more honest about what the system actually returns, and
preserving the re-shaped JSON would defeat the simplification.

## Out of Scope

- **Wire-type tightening.** The "Wire types, but tighten the wire shape"
  alternative (dropping `body_hash` et al. from `ResourceRow` at the
  API level) was considered but deferred — it doubles the blast radius
  by touching `temper-core` types and ts-rs bindings. If JSON noise
  becomes a real complaint downstream, that work happens in its own PR.
- **View-type indirection.** Introducing slim view structs separate
  from wire types was considered and rejected — doubles the type
  surface for marginal gain.

## Connections

- **Direction memory:** `project_cli_output_format_simplification`
- **Predecessor PR (post-cloud-only sweep PR 1):** #96
- **Parent task:** `2026-05-26-post-cloud-only-qol-sweep-vestige-env-vars-optional-temper-token-cli-output-collapse-to-json-toon-plus-three-minor-followups`
- **TOON spec reference:** https://toonformat.dev/
- **Selected crate:** https://crates.io/crates/toon-format (v0.5.0, MIT)
- **Goal:** `path-to-alpha`
