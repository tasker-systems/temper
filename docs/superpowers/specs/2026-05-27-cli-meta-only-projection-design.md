# CLI meta-only projection and field subselection

**Date:** 2026-05-27
**Status:** Approved (brainstorm complete; implementation plan to follow)
**Parent task:** `2026-05-25-cli-read-side-meta-affordance-for-resources`
**Sibling task:** `2026-05-27-qol-for-get-meta-and-skill-refresh` (skill content
refresh; sequenced *after* this lands)
**Branch:** `jct/cli-meta-only-projection`

## Goal

Give agents a cheap **orientation projection** of one or many resources without
forcing them to read the body. Surface lives as two new flags on the existing
`temper resource show` and `temper resource list` verbs (`--meta-only`,
`--fields a,b,c`), plus a `fields` parameter on the matching MCP tools. Output
shape stays in json|toon — the CLI does not invent a new format. The whole
move is API passthrough plus top-level key projection.

## Background

### The signal

Agents have reached for `temper resource get-meta` (and `get_meta` on the
MCP) **6+ times across independent sessions**. The original task
(`2026-05-25-cli-read-side-meta-affordance-for-resources`) was created when
the count hit 4 — well past the recording threshold. The reach has continued
since.

This is not bikeshedding. It's a ratified need.

### The frame

From `2026-05-23-cognitive-maps-and-the-projection-class-insight`:

> When an agent lands on a concept in the map, what is wanted first is *cheap
> orientation*: the smallest representation that lets it decide whether to go
> deeper. The expensive thing — full graph neighborhood, source passages,
> edge-metadata, trajectory history — should be available but never
> volunteered. **Progressive disclosure is the whole game for an agent,
> because the cost of front-loading is paid in context that cannot then be
> used for anything else.**

The 8-mechanic taxonomy of how agents-and-humans use a knowledge system —
*orientation, wayfinding, recall, recognition, composition, boundary-sensing,
translation, trust-calibration* — places this work squarely in **orientation**
(and adjacent to **recognition**, since managed-meta is a resource's
recognizable fingerprint).

The existing CLI verbs map roughly onto the taxonomy:

| Verb | Primary mechanic |
|------|------------------|
| `temper search` | recall + recognition |
| `temper resource list` | orientation-of-domain |
| `temper resource show` | composition + recall (full content) |
| (missing) | cheapest orientation-of-one |
| (missing) | cheapest orientation-across-many with full meta tier |

This task closes both gaps without inventing a new verb.

### The post-cloud-only constraint

PR #97 (Group F) just collapsed the CLI's output formats to json|toon as
pure API passthrough. Adding a new YAML-shaped projection would re-open the
"bespoke output shapes" drift door that was just closed. The whole design
rests on **the cloud API response IS the output shape**; the CLI subselects
top-level keys but does not transform.

## Design Decisions

| Decision | Choice | Reason |
|----------|--------|--------|
| Verb vs flag | Flags on existing `show` / `list` | No new verb; preserves the json\|toon passthrough invariant established by PR #97 |
| Output format | json\|toon, unchanged | Bespoke YAML projection would reopen the format-drift door PR #97 just closed |
| Single-resource shape | `ResourceMetaResponse` | Already exists in temper-core, already exposed at `GET /api/resources/{id}/meta`, already in temper-client |
| Multi-resource shape | New `ResourceMetaListResponse { rows, total, facets }` | Mirrors `ResourceListResponse` shape; agent only has to learn "rows changes type" |
| `--meta-only` on `list` semantic | Switch row type to `ResourceMetaResponse` | List rows today (`ResourceRow`) only carry *projected scalars* of managed_meta (stage, mode, effort) and no open_meta. Meta-mode gives agents the full meta tier per item |
| Server wire shape | Same route, `?meta_only=true` query param, `oneOf` response | utoipa OpenAPI uses `oneOf<ResourceListResponse, ResourceMetaListResponse>`; client has two typed methods (`list`, `list_meta`) |
| Field grammar | Top-level keys only | Smallest viable rule; nested projection is what `jq` is for |
| Field anchor | `resource_id` (or surface-appropriate equivalent) always preserved | Trust-calibration: agent can always look it back up regardless of `--fields` |
| Nested-path handling | Rejected with explicit `jq` pointer in error | Honest about scope; teaches agents the escape valve |
| Projection module | Shared in `temper-core::projection` | Used by CLI action layer AND MCP tool handlers; one source of truth for filter semantics |
| MCP parity | `fields: Option<Vec<String>>` on `get_resource` + `list_resources` | MCP responses go straight to agent context; no shell pipe escape valve. Projection is *more* load-bearing on MCP than CLI |
| MCP `meta_only` parameter | **Not added** | MCP `get_resource(include_content=false)` already serves cheap orientation by default (returns `EnrichedResource`, no body). Adding `meta_only` would force-narrow to `ResourceMetaResponse`, which is *strictly poorer* (loses joined display fields like context_name, doc_type_name, owner_handle) |
| Facets in meta-mode | Included, identical shape | Computed by separate SQL aggregation in parallel via `tokio::try_join!` (`resource_service.rs:270-285`); projection-independent; zero marginal cost |
| Skill content refresh | Out of scope here; sibling task `2026-05-27-qol-for-get-meta-and-skill-refresh` | Sequenced after this lands so SKILL.md is touched once (drops dead vault-mode references + promotes `--meta-only`) |

## Surface Contract

### CLI — `temper resource show`

```bash
# default — unchanged
$ temper resource show <slug> --type task --context temper
# returns ResourceRow + body in json|toon

# orientation projection of one
$ temper resource show <slug> --type task --context temper --meta-only
# returns ResourceMetaResponse: {resource_id, managed_meta, open_meta,
#                               managed_hash, open_hash}

# subselect top-level keys (resource_id always preserved)
$ temper resource show <slug> --type task --context temper --meta-only \
    --fields managed_meta
# returns: {resource_id, managed_meta}

# nested projection rejected
$ temper resource show <slug> --type task --context temper --meta-only \
    --fields managed_meta.stage
# error: --fields supports top-level keys only; use jq for nested projection:
#   temper resource show <slug> --type task --context temper --meta-only | \
#     jq '.managed_meta.stage'

# --meta-only is mutually exclusive with --edges
$ temper resource show <slug> --type task --meta-only --edges
# clap error: arguments cannot be used together
```

### CLI — `temper resource list`

```bash
# default — unchanged
$ temper resource list --type task --context temper --stage in-progress
# returns ResourceListResponse {rows: Vec<ResourceRow>, total, facets}

# orientation projection across many — full meta tier per item
$ temper resource list --type task --context temper --stage in-progress \
    --meta-only
# returns ResourceMetaListResponse {rows: Vec<ResourceMetaResponse>, total,
#                                   facets}

# subselect on each item
$ temper resource list --type task --context temper --stage in-progress \
    --meta-only --fields managed_meta
# returns: {rows: [{resource_id, managed_meta}, ...], total, facets}

# --fields applies to non-meta list rows too
$ temper resource list --type task --context temper --fields slug,stage,mode
# returns: {rows: [{id, slug, stage, mode}, ...], total, facets}
# (anchor field `id` always preserved on ResourceRow)
```

### MCP — `get_resource`, `list_resources`

```jsonc
// New input parameter on existing tools
{
  "name": "get_resource",
  "arguments": {
    "slug": "...",
    "context_name": "temper",
    "include_content": false,
    "fields": ["managed_meta"]    // NEW — top-level key subselection
  }
}

// Existing default behavior unchanged: include_content=false returns
// EnrichedResource (row + meta, no body). The new `fields` parameter
// applies the same projection filter as the CLI before serialization.
```

No `meta_only` parameter on MCP — see Design Decisions for the reason.

## Wire Shape

### API

| Method | Path | Query | Response (utoipa) |
|--------|------|-------|-------------------|
| GET | `/api/resources/{id}/meta` | — | `ResourceMetaResponse` (already exists, unchanged) |
| GET | `/api/resources` | `meta_only=false` (default) | `ResourceListResponse` (existing) |
| GET | `/api/resources` | `meta_only=true` | `ResourceMetaListResponse` (new) |

OpenAPI represents the list endpoint response as `oneOf<ResourceListResponse,
ResourceMetaListResponse>`.

The handler dispatches on the query param. The default path is unchanged.
The `meta_only=true` path runs the same visibility-scoped list query, then
joins via `meta_service::get_meta_batch` (already used internally by the MCP
`enrich_resources` flow) to produce `Vec<ResourceMetaResponse>`. The
facets query runs unchanged in parallel via `tokio::try_join!`.

### temper-client

| Method | Returns |
|--------|---------|
| `client.resources.get(id)` (existing) | `ResourceRow` |
| `client.resources.get_meta(id)` (existing) | `ResourceMetaResponse` |
| `client.resources.list(params)` (existing) | `ResourceListResponse` |
| `client.resources.list_meta(params)` (new) | `ResourceMetaListResponse` |

`list_meta` is a thin sibling of `list` — same `ResourceListParams`, same
URL path, just adds `meta_only=true` to the query string and parses into
the meta response type.

### New core type

```rust
// temper-core::types::managed_meta

#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "managed_meta.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceMetaListResponse {
    pub rows: Vec<ResourceMetaResponse>,
    pub total: i64,
    pub facets: ResourceFacets,
}
```

Mirrors `ResourceListResponse` field-for-field; just the row type differs.

## Architecture

### Module layout

**New file:** `crates/temper-core/src/projection.rs`

The projection filter module. Single source of truth for "anchor key always
preserved + top-level subselection + dotted-path rejection" semantics. Used
by both the CLI action layer and the MCP tool handlers. ~50-80 LOC + tests.

```rust
/// Top-level key projection over a serde_json::Value.
///
/// Filters an object's top-level keys to those in `fields`, plus the
/// anchor key (which is always preserved). For an array of objects,
/// applies the filter to each element.
///
/// Returns ProjectionError::DottedPath if any field name contains `.`,
/// with an error message pointing the caller at `jq` for nested
/// projection.
pub fn apply_top_level_filter(
    value: serde_json::Value,
    fields: &[String],
    anchor: &str,
) -> Result<serde_json::Value, ProjectionError>;

#[derive(Debug, thiserror::Error)]
pub enum ProjectionError {
    #[error("--fields supports top-level keys only; use jq for nested projection: {hint}")]
    DottedPath { hint: String },
    #[error("empty field name")]
    EmptyField,
}
```

**Touched file:** `crates/temper-core/src/types/managed_meta.rs`

Add `ResourceMetaListResponse` (new type, mirrors `ResourceListResponse`).

**Touched file:** `crates/temper-api/src/handlers/resources.rs`

The existing `list` handler dispatches on the `meta_only` query param.
Default arm unchanged (calls `resource_service::list_visible`). New arm
calls the new `resource_service::list_visible_meta` (see below) and
returns `ResourceMetaListResponse`.

**Touched file:** `crates/temper-api/src/services/resource_service.rs`

Add a `list_visible_meta` function (or extend `list_visible` with a
`meta_only: bool` argument) that returns `ResourceMetaListResponse`. Wires
together the existing row query, the existing facets query, and the
existing `meta_service::get_meta_batch`.

**Touched file:** `crates/temper-client/src/resources.rs`

Add `list_meta` method.

**Touched file:** `crates/temper-cli/src/cli.rs`

Add `meta_only: bool` and `fields: Vec<String>` to `ResourceAction::Show` and
`ResourceAction::List`. On `Show`, `meta_only` `conflicts_with` `edges`.

**Touched file:** `crates/temper-cli/src/commands/resource.rs`

In `show`: branch on `meta_only` — call `client.resources.get_meta(id)`
instead of the default path. Apply projection filter (if `fields` non-empty)
to the response Value before passing to `render`.

In `list`: branch on `meta_only` — call `client.resources.list_meta(params)`
instead of `list`. Apply projection filter per-item (the response shape is
an object `{rows: [...], total, facets}`, and we filter the `rows` array
elements; `total` and `facets` pass through untouched).

**Touched file:** `crates/temper-mcp/src/tools/resources.rs`

Add `fields: Option<Vec<String>>` to `GetResourceInput` and
`ListResourcesInput`. In each tool handler, after building the
`EnrichedResource` / `Vec<EnrichedResource>`, serialize to `Value`, apply the
shared projection filter, then write the filtered text content into the tool
result.

### Projection filter behavior

| Input | Behavior |
|-------|----------|
| `--fields managed_meta` | output contains anchor + `managed_meta` only |
| `--fields managed_meta,open_meta` | output contains anchor + both keys |
| `--fields managed_meta,unknown_key` | output contains anchor + `managed_meta` only; unknown key silently dropped |
| `--fields ""` or `--fields " "` | `ProjectionError::EmptyField` |
| `--fields managed_meta.stage` | `ProjectionError::DottedPath` with jq pointer |
| `--fields <anchor>` (e.g. `resource_id` or `id`) | output contains anchor only — explicit listing is harmless because the anchor is always preserved |
| Empty `--fields` (flag not passed) | no filtering; full response returned |

Filter applies *only* to the top-level object (or to each element of a
top-level array). Wrapping containers (`rows`, `total`, `facets`) are not
themselves filtered — only the items inside `rows` are filtered. This keeps
the response envelope stable across with/without `--fields`.

### Anchor key per surface

| Surface | Response type | Anchor key |
|---------|---------------|------------|
| `temper resource show` (default) | `ResourceRow` | `id` |
| `temper resource show --meta-only` | `ResourceMetaResponse` | `resource_id` |
| `temper resource list` (default) | `ResourceListResponse.rows[i]` (ResourceRow) | `id` |
| `temper resource list --meta-only` | `ResourceMetaListResponse.rows[i]` (ResourceMetaResponse) | `resource_id` |
| MCP `get_resource` | `EnrichedResource` | `id` |
| MCP `list_resources` | `EnrichedResource[i]` | `id` |

The anchor field name differs between `ResourceRow` (`id`) and
`ResourceMetaResponse` (`resource_id`); the projection filter takes the
anchor name as a parameter so both surfaces can call the same function.

## Tests

TDD — write the test first, watch it fail, implement, watch it pass.

### Unit tests — `temper-core::projection`

- `apply_top_level_filter` preserves anchor when fields list is empty.
- `apply_top_level_filter` preserves anchor when fields list contains other
  keys.
- `apply_top_level_filter` drops unknown top-level keys silently.
- `apply_top_level_filter` returns `DottedPath` for `"managed_meta.stage"`,
  with the error message including the jq invocation hint.
- `apply_top_level_filter` returns `EmptyField` for `""` or `"   "`.
- `apply_top_level_filter` works on `Value::Object` (single resource).
- `apply_top_level_filter` works on `Value::Array` of objects (list rows).
- `apply_top_level_filter` is a no-op when `fields.is_empty()` (returns
  input value unchanged).

### Unit tests — `temper-cli::commands::resource`

- `show --meta-only` rendering smoke test under both `Json` and `Toon`
  formats: response shape preserved through `render`.
- `list --meta-only --fields managed_meta` rendering smoke test: anchor
  preserved, only `managed_meta` retained on each row.
- Clap validation: `--meta-only` and `--edges` rejected together with a
  clear conflict error.

### Integration tests — `temper-api` (`--features test-db`)

- `GET /api/resources?meta_only=true` returns `ResourceMetaListResponse`
  with the right item count and facets identical to the default-arm
  response over the same filter set.
- `GET /api/resources` (default) unchanged — regression guard.
- `GET /api/resources?meta_only=true&context_name=X&stage=Y` honors all
  list filters; just the row type differs.

### Integration tests — `temper-mcp` (`--features test-db,test-embed`)

- `get_resource` with `fields=["managed_meta"]` returns text content whose
  JSON shape is `{id, managed_meta}` only.
- `list_resources` with `fields=["managed_meta"]` returns each row filtered
  to `{id, managed_meta}`.
- `get_resource` with `fields=["managed_meta.stage"]` returns an MCP error
  with the jq pointer in the message.

### End-to-end tests — `tests/e2e/`

Real Axum + Postgres + CLI binary through `temper-client`. Drive the full
flow.

- `temper resource show <slug> --type task --context <ctx> --meta-only`
  produces `ResourceMetaResponse` shape under json output.
- `temper resource list --type task --context <ctx> --stage in-progress
  --meta-only` produces `ResourceMetaListResponse` shape.
- `temper resource show <slug> --type task --context <ctx> --meta-only
  --fields managed_meta` produces filtered shape with `resource_id` +
  `managed_meta` only.
- Nested-path rejection: `--fields managed_meta.stage` produces non-zero
  exit and a stderr message containing `jq` invocation.

## Design Paths Rejected

These are not deferred — they are deliberate rejections on design merit.
The reasons should not erode under future pressure.

### MCP `meta_only` parameter

MCP `get_resource` and `list_resources` already serve cheap orientation
by default (`include_content=false` skips body fetch; result is
`EnrichedResource`, which is row + meta). Adding `meta_only` would force
the narrower `ResourceMetaResponse` shape, which is *strictly poorer*
than `EnrichedResource` (loses joined display fields like `context_name`,
`doc_type_name`, `owner`). MCP gets the `fields` projection parameter
only — that's the actual load-bearing gap for MCP, since MCP responses
cannot be piped through `jq`.

### `--fields` with nested paths or jq expressions

Top-level keys only is a deliberate boundary. Anything deeper is a
projection that wants the full power of `jq`, and the CLI emits
`jq`-pipeable json, so the path is already open. Embedding `jaq-core`
to support nested paths would mean owning a jq compatibility layer —
not a scope choice, a category rejection. The boundary lives at "we do
not own a query language."

### YAML output / on-disk frontmatter parity

PR #97 closed the bespoke-output-shape door. The on-disk YAML
frontmatter exists for humans-and-Obsidian; the CLI does not aim for
parity with it. Agents who want the on-disk shape can `cat` the
projected file directly (when the projection cache is populated). This
is not a "yet" — it is a "no", load-bearing on the cloud-only-vault
direction.

## Deferred

These are in-scope-elsewhere or in-scope-later. Capturing them keeps
this design honest about what it touches and what it doesn't.

### Skill content refresh

The temper skill (SKILL.md, workflows, reference.md) has stale content
from the local-vault era — including `task start` sequences that
predate the requirement to pass `--context` explicitly. This is real
work, but it lives in sibling task
`2026-05-27-qol-for-get-meta-and-skill-refresh`. That task is sequenced
*after* this one so SKILL.md is touched once (dropping dead vault-mode
references and promoting the new `--meta-only` flag in the same pass).

### Resource-by-slug for the meta endpoint

`GET /api/resources/{id}/meta` is by-ID. The CLI resolves slug-to-id
through the existing show path before calling `get_meta`. A by-slug
meta endpoint is doable but unmotivated for this task — the CLI's
slug-to-id resolution is already a single cheap call. Add it when a
direct caller (not the CLI) demonstrates the need.

## Connections

- **Parent task:** `2026-05-25-cli-read-side-meta-affordance-for-resources`
- **Sibling task:** `2026-05-27-qol-for-get-meta-and-skill-refresh`
- **Frame document:** `2026-05-23-cognitive-maps-and-the-projection-class-insight`
- **Predecessor PR:** PR #97 (CLI output collapse to json|toon) — closes the
  "bespoke output shape" door that this design carefully does not reopen
- **Existing affordances reused:**
  - `meta_service::get_meta` and `meta_service::get_meta_batch`
  - `temper_client::resources::get_meta`
  - `GET /api/resources/{id}/meta`
  - `ResourceMetaResponse` (temper-core)
  - `EnrichedResource` (temper-mcp)
- **Direction memory:** `project_cli_output_format_simplification` (landed
  via PR #97 — this design honors the constraint it established)
- **Goal:** `path-to-alpha`
