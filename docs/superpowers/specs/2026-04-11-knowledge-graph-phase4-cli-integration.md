# Phase 4: CLI Integration for Knowledge Graph

**Date:** 2026-04-11
**Status:** Design
**Depends on:** Phase 1 (schema + SQL functions), Phase 2 (edge extraction), Phase 3 (combined search)

## Problem

Phases 1-3 built the graph schema, edge extraction pipeline, and graph-enhanced search — all server-side. The CLI has no way to control graph search parameters, display resource edges, or preserve structured frontmatter fields (arrays/objects) through the sync roundtrip.

Specifically:
1. `temper search` hardcodes `graph_expand: true` with no way to pass seeds, edge types, depth, or opt out
2. `temper show` has no way to display a resource's graph edges
3. `build_frontmatter_from_resource()` silently drops JSON arrays and objects from `open_meta`, so relationship fields (`depends_on: [slug-a, slug-b]`) and other structured frontmatter are lost on sync pull

## Design

### 1. Search CLI Flags (Composable)

Add four flags to the search command in `crates/temper-cli/src/cli.rs`:

| Flag | Type | Maps to | Notes |
|------|------|---------|-------|
| `--seed <uuid>` | repeatable | `SearchParams.seed_ids` | Explicit seed resource IDs for graph expansion |
| `--edge-type <type>` | repeatable | `SearchParams.edge_types` | Filter graph traversal to specific edge types |
| `--depth <n>` | `i32` | `SearchParams.graph_depth` | Max traversal hops (server default: 2, max: 10) |
| `--no-graph` | flag | `SearchParams.graph_expand = false` | Opt out of graph expansion entirely |

All flags are optional. Without any graph flags, behavior is unchanged (graph_expand defaults true, server picks defaults for depth/seeds).

`--seed` accepts UUIDs. Multiple seeds: `--seed <uuid1> --seed <uuid2>`.

`--edge-type` accepts the edge_type enum values: `relates_to`, `extends`, `depends_on`, `references`, `parent_of`, `tagged_with`, `preceded_by`, `derived_from`.

#### Search Action Changes

Switch `search_actions::query_api()` and `text_query_api()` to build a full `SearchParams` and call `client.search().search_with_params(&params)` instead of the old `client.search().search(query, embedding, ...)` method. This passes all graph fields through to the API.

### 2. Show Command `--edges` Flag

Add `--edges` flag to the show command (`temper resource show <slug> --type <type> --edges`).

When present:
- Display the resource content as normal
- Append an "Edges" section at the bottom

**Text format (default):**
```
[resource content as normal]

Edges:
  outgoing:
    depends_on → deployment-config (Deployment Configuration)
    references → api-schema (API Schema Reference)
  incoming:
    extends ← base-template (Base Template)
```

Each line shows: `edge_type direction_arrow peer_slug (peer_title)`

**JSON format (`--format json`):**
Include edges as a structured field alongside content:
```json
{
  "doc_type": "...",
  "slug": "...",
  "content": "...",
  "edges": [
    {
      "edge_id": "...",
      "peer_resource_id": "...",
      "peer_title": "...",
      "peer_slug": "...",
      "edge_type": "depends_on",
      "direction": "outgoing",
      "weight": 1.0
    }
  ]
}
```

#### Client Method

Add `graph_resource_edges(resource_id: Uuid)` to `temper-client` (likely on a new `GraphClient` sub-client or on the existing resource client). This calls the API endpoint that invokes the `graph_resource_edges()` SQL function.

#### API Endpoint

Add `GET /api/resources/:id/edges` handler in `temper-api`. Calls `graph_resource_edges()` from the service layer.

### 3. Fix `build_frontmatter_from_resource()` Array/Object Serialization

**File:** `crates/temper-cli/src/actions/ingest.rs`, `build_frontmatter_from_resource()` (~line 435)

Currently the function iterates `managed_meta` JSON and skips arrays and objects:
```rust
// Current: only strings, numbers, booleans pass through
```

Fix: serialize JSON arrays as YAML flow sequences and JSON objects as nested YAML maps.

**Array example** (relationship fields):
```yaml
depends_on: ["slug-a", "slug-b"]
```

**Object example** (custom structured metadata):
```yaml
config:
  key: "value"
  nested: true
```

Use YAML flow style for arrays (single-line `["a", "b"]`) since relationship fields are typically short lists of slugs/UUIDs. Use block style for objects since they may have multiple keys.

This is a general fix — not graph-specific. Any user-authored frontmatter with arrays or objects will now survive the sync roundtrip.

### 4. E2E CLI Test

Add an end-to-end test that exercises the full CLI graph flow:

1. **Setup:** Create 3 documents — A depends_on B, B references C — ingest all three
2. **Graph search with seed:** `temper search` with `--seed <A-uuid>`, verify B and C surface via graph expansion
3. **Graph search opt-out:** `temper search --no-graph "query"`, verify only direct text/vector matches return
4. **Show with edges:** `temper show <A> --edges`, verify the depends_on → B edge appears in output
5. **Frontmatter roundtrip:** Verify that relationship fields survive ingest → sync pull → re-read

## Files Changed

| File | Change |
|------|--------|
| `crates/temper-cli/src/cli.rs` | Add `--seed`, `--edge-type`, `--depth`, `--no-graph`, `--edges` flags |
| `crates/temper-cli/src/commands/search_cmd.rs` | Pass graph flags to search actions |
| `crates/temper-cli/src/commands/resource.rs` | Handle `--edges` in show command |
| `crates/temper-cli/src/actions/search.rs` | Build full `SearchParams`, use `search_with_params()` |
| `crates/temper-cli/src/actions/ingest.rs` | Fix array/object serialization in `build_frontmatter_from_resource()` |
| `crates/temper-api/src/handlers/` | Add `GET /api/resources/:id/edges` endpoint |
| `crates/temper-api/src/services/` | Add service function calling `graph_resource_edges()` |
| `crates/temper-client/src/` | Add graph edges client method |
| `crates/temper-e2e/` | E2E test for CLI graph flow |

## Out of Scope

- `--mode graph` abstraction — composable flags match the `SearchParams` structure, no mode enum needed
- `parse_source_frontmatter()` changes — raw frontmatter already passes through to server; edge extraction reads from `open_meta` server-side
- TypeScript type regeneration — post-phase-4 cleanup before PR
- Commit squashing — post-phase-4 cleanup before PR
