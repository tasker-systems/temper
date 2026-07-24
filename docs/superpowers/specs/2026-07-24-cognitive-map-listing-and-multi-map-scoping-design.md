# Cognitive-map listing, orientation, and multi-map scoping

- **Date:** 2026-07-24
- **Status:** Design — draft for review
- **Goal:** Unified visibility semantics for contexts and cognitive maps (temper resource `019f5c66-755e-7fc1-bd87-ee2de8e4cd3f`)
- **Tasks:**
  - `cogmap list` — enumerate a profile's visible maps (`019f9432-40a7-7b80-8ba5-19b1e4acd2ba`)
  - `cogmap show` — one-read orientation: charter + foundational resources (`019f9432-f1b0-7cc3-b3ef-c96720182891`)
  - Multi-map scope — repeatable `--cogmap` on search and resource list (`019f9433-5e08-7fb2-9816-2d89aa1e0cdb`)
- **Sits under:** the visibility-semantics goal, upstream of the region tier it governs — you cannot orient on a region whose parent map you cannot list.

## Motivation

The substrate can already answer the three questions that precede any map-framed work — *which
cognitive maps may I see*, *what is each one for*, and *which resources is each built on* — and it
gates every answer correctly. None of it is reachable as a first move.

Every cogmap read and write in the system takes a `cogmap` **ref** as input. Nothing hands a caller
the list of refs to begin with. An agent or human who wants to "search from the frame of the
architecture map" has no command that names the map, no command that shows what the map is for, and
no way to search across a chosen *set* of maps rather than exactly one. Wayfinding already pools
regions across every visible map; the explicit scope selector goes the other way and admits one map
only. The fundamentals are built and shipped — this is a surface-completeness gap, not a substrate
gap.

This spec exposes reads that already exist and widens one scope selector from a scalar to a set. It
introduces **no new access-control surface**.

## What is true today (verified against the codebase, 2026-07-24)

| Capability | Where it lives | Reachable from |
|---|---|---|
| The set of maps a principal may see | `cogmap_visible_maps(principal)` — `20260701000002_cogmap_read_up_flip.sql:53` | nothing directly |
| That set joined to name / owner_ref / team_ids / region_count / resource_count | `graph_home_cogmaps(principal)` — `20260707140000_graph_home_build_research_reads.sql:97` | Atlas only (`graph_service::atlas_home`) |
| A map's charter blocks (statement / question / framing, in seq) | `cogmap_charter_select` → `CharterBlock` — `substrate_read.rs:818` | MCP `cogmap_read_charter` |
| A map's map-level analytics (telos / staleness / regulation) | `cogmap_analytics` — `readback/mod.rs:1378` | CLI `cogmap analytics`, MCP, API |
| A map's homed resources, visibility-intersected | `cogmap_scope_ids(principal, map)` — `20260629000005_cogmap_home_authz_and_scope.sql:12` | search `--cogmap` (as a scope, not a listing) |
| Single-map search scope | `SearchParams.cogmap_id` → `cogmap_scope_ids` → `unified_search.p_scope_ids` | CLI/MCP/API search |

The invariant the whole design leans on, stated in the migration headers and enforced:
**map-read = resource-read agree by construction** — `cogmap_visible_maps` and
`cogmap_readable_by_profile` resolve to the same set (up-expanded team membership ∪ explicit read
grant), and the cogmap branch of `resources_visible_to` mirrors it.

## Three omissions, one cause

**1. There is no way to enumerate cognitive maps — on any surface.** Not CLI, not MCP, not the
public API, not the client. `graph_home_cogmaps` is the missing read *already written*; it is trapped
behind the Atlas graph handler and returns no charter/telos identity to orient on.

**2. There is no single "orient me on this map" read.** Charter and analytics exist as separate MCP
calls; neither carries the map's own name or id, and neither points at the resources that shaped the
map. A newcomer to a map cannot in one step see *what it is for* and *what it is built on*.

**3. Scope is single-valued where the mental model is plural.** The one-map selector narrows a
set-shaped sink to one element. `resource list` has no cogmap scope at all.

### The structural tell

`cogmap_scope_ids(principal, cogmap)` takes a single map, but the thing it feeds —
`unified_search`'s `p_scope_ids uuid[]` (`20260629000004_search_scope_ids.sql:35`) — has always been
a *set*. The plural was there in the corpus CTE the whole time; only the resolver and the wire field
are singular. Widening the selector is closing a narrowing that was never load-bearing.

## Decisions

### D1 — `temper cogmap list`: the missing enumeration, charter-bearing

**Decision.** Add a `CogmapCmd::List` variant and the full chain beneath it: a `CogmapRow` identity
type, a `cogmap_service::list_visible` read, `GET /api/cognitive-maps`, `CognitiveMapClient::list()`,
and an MCP `cogmap_list` tool.

The listing read is `graph_home_cogmaps` **plus two columns it does not carry**: `telos_resource_id`
and the charter **statement** (block-0 of the telos). The statement is the one field that turns a
name into an orientation — "what is this map for" answered in the list, not one round-trip later.
Rather than mutate `graph_home_cogmaps` (consumed by the Atlas, which must not change shape), add a
sibling SQL function `cogmap_list_rows(p_profile)` that returns the superset:

```sql
CREATE FUNCTION cogmap_list_rows(p_profile uuid)
RETURNS TABLE(
    cogmap_id uuid, name text, owner_ref text, team_ids uuid[],
    region_count int, resource_count int,
    telos_resource_id uuid, charter_statement text
) LANGUAGE sql STABLE AS $$
    -- body of graph_home_cogmaps, plus:
    --   c.telos_resource_id
    --   the block-0 'statement' body of the telos, via a LATERAL over resource_blocks(
    --       c.telos_resource_id, 'profile', p_profile, NULL) WHERE role = 'statement' LIMIT 1
    -- statement is member-gated by the same resource_blocks projection the charter read uses;
    -- map-read = resource-read means a listed map's telos statement is always readable.
$$;
```

`charter_statement` is nullable in the row type — a map whose charter has not been authored yet (MCP
genesis mints an empty charter) lists with a null statement, not a hidden row.

**`CogmapRow`** (temper-core, `types/cognitive_maps.rs`, standard `typescript` / `web-api` / `mcp`
derive triad as on `CharterBlock`):

```rust
pub struct CogmapRow {
    pub id: Uuid,
    pub name: String,
    pub owner_ref: String,              // held-by scope: "+team-slug" or "temper"
    pub team_ids: Vec<Uuid>,
    pub region_count: i32,
    pub resource_count: i32,            // homed-resource count (graph_home_cogmaps' facet_count)
    pub telos_resource_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub charter_statement: Option<String>,
}
```

The decorated `ref` (`sluggify(name)-<uuid>`) is **render-time only**, injected by the CLI exactly as
`inject_context_ref` does for contexts — never persisted, never on the wire type. A new
`inject_cogmap_ref` helper computes it from `id` + `name`.

**Filters.** `--name-contains <substr>` (case-insensitive, mirrors `list --title-contains`) and
`--team <ref>` (resolved via the shared `resolve_team_id`, filters `team_ids @> {team}`). Both are
optional; the default is every visible map. Filtering is cheap and can be applied in Rust over the
returned rows, or pushed into SQL — either is fine; the row set is small (tens of maps).

**Rationale.** No new access predicate: `cogmap_visible_maps` already *is* the "maps I can access"
primitive, and `graph_home_cogmaps` already wraps it with counts. We are giving that read a
charter-bearing sibling and a front door. The Atlas's consumer is untouched, so no visualization
regresses.

### D2 — `temper cogmap show <ref>`: one-read orientation

**Decision.** Add a `CogmapCmd::Show` variant returning a `CogmapDetail` aggregate: the `CogmapRow`
identity, the full charter (`Vec<CharterBlock>`, reusing `cogmap_charter_select` unchanged), and the
map's **foundational resources**.

```rust
pub struct CogmapDetail {
    pub cogmap: CogmapRow,
    pub charter: Vec<CharterBlock>,
    pub foundations: Vec<CogmapFoundationRow>,
}

pub struct CogmapFoundationRow {
    pub resource_id: Uuid,
    pub title: String,
    pub doc_type: String,
    pub is_telos: bool,                 // the charter/telos resource, flagged
    // ref injected render-time (inject_ref); context_ref where resolvable
}
```

**Foundational resources = the map's homed resources** (`kb_resource_homes WHERE
anchor_table='kb_cogmaps' AND anchor_id = X`), intersected with `resources_visible_to`, with the
telos/charter resource flagged `is_telos = true`. This is the body of `cogmap_scope_ids` minus the
search plumbing — a read the substrate already computes. The telos is sorted first; the rest follow
by title (or seq if we later thread one through).

This is a deliberate choice over a region-centrality survey. Region anchors are **empty for an
un-materialized map** and belong to the region tier the sibling goal governs; "what the map is built
on" is a structural, always-available fact about its homes. Region-anchor surfacing is explicitly a
non-goal here and can layer on later as a `--shape` section once the region-read unification of
`019f5c66` lands.

`GET /api/cognitive-maps/{id}` returns `CogmapDetail`; an unreadable map is `NotFound` (the map-read
gate), never a partial leak. MCP `cogmap_show` returns the same aggregate (the charter tool
`cogmap_read_charter` stays as the narrow primitive; `cogmap_show` is the composed orientation).

**Rationale.** One command answers "what is this map, what is it for, what is it built on." Every
ingredient exists; this composes them and flags the telos so the constitutive resource is legible
among its homes.

### D3 — Multi-map scope: one set-shaped selector, additive on the wire

**Decision.** `--cogmap` becomes **repeatable** on both `temper search` and `temper resource list`.
The wire change is **additive**: a new `cogmap_ids: Option<Vec<Uuid>>` beside the existing
`cogmap_id: Option<Uuid>` on `SearchParams`, and a new `cogmap_ids` filter on the list params. No
existing field changes type; no client breaks.

**Search.**

- `SearchParams` gains `#[serde(default)] pub cogmap_ids: Option<Vec<Uuid>>`. `cogmap_id` stays for
  back-compat and single-map callers (temper-rb, older CLIs).
- New SQL `cogmap_scope_ids_multi(p_principal uuid, p_cogmaps uuid[]) RETURNS SETOF uuid` — the union
  of per-map `cogmap_scope_ids`, each independently `cogmap_readable_by_profile`-gated, so an
  unreadable element in the set contributes nothing (never an error). One round-trip.
- `resolve_search_scope` (`substrate_read.rs:393`): when `cogmap_ids` is non-empty, resolve through
  the multi function; else the existing single-`cogmap_id` arm. `SearchScope::Cogmap` is unchanged
  (the enum is a frozen wire contract — see the `classify_scope` note at `substrate_read.rs:549`);
  `scope_size` reports the union size. The `context_ref` mutual-exclusion still holds against the
  cogmap set.
- CLI: `Commands::Search.cogmap` becomes `Vec<String>` (repeatable). `build_search_params` parses
  each ref (trailing-UUID-only) into `cogmap_ids`; the `--context`/`--cogmap` client-side guard
  fires when the vec is non-empty. `--wayfind` still composes with a single cogmap anchor; a
  *multi*-map `--wayfind` is rejected client-side (wayfind's anchor is one home) — documented, not
  silently dropped.
- temper-client and MCP `search` carry `cogmap_ids` through.

**Resource list.**

- `Commands::List` gains a repeatable `--cogmap`. Today `resource list` has **no** cogmap scope at
  all.
- `filtered_visible_page` (`substrate_read.rs:104`) gains an optional `cogmap_ids` filter clause,
  mirroring the existing `context_id` clause at `:158`: `h.anchor_table='kb_cogmaps' AND h.anchor_id
  = ANY($N)`. A resource has exactly one home (`kb_resource_homes.resource_id` is UNIQUE), so this is
  a clean home-anchor filter, and the query already runs through `resources_visible_to`, so access is
  gated with no extra predicate.
- List params type, client, and MCP `list_resources` gain the cogmap-ids filter.

**Rationale.** The sink was always a `uuid[]`; this restores the plural the resolver dropped. Additive
fielding keeps every current client working. List and search share the same "resources homed in this
set of maps" corpus definition, differing only in ranking vs. enumeration.

### D4 — Access, errors, and the empty set

**Decision.** Every new read is scoped through the existing predicates — `cogmap_visible_maps` for
listing, `resources_visible_to` / `cogmap_readable_by_profile` for scoping and foundations. The
conventions already in force everywhere in the cogmap surface hold verbatim:

- **Deny → empty, never error** for list/scope reads (an unreachable map is invisible, not a 403).
- **`cogmap show` on an unreadable map → `NotFound`** (a single addressed resource that you may not
  see is a 404, matching `cogmap analytics`' `.ok_or(NotFound)`).
- **No returned value is computed over members the caller cannot see** — the foundations read
  intersects with `resources_visible_to`; the charter statement rides the same member-gated
  `resource_blocks` projection the charter read uses.

**No new access-control surface is introduced.** This is the load-bearing constraint: the goal is
exposure and widening, not a new gate.

## Surface parity (the goal's through-line)

Each capability lands on CLI **and** MCP in the same beat, because "the CLI, the API, and the MCP each
see a different subset" is precisely the asymmetry `019f5c66` exists to end:

| | CLI | API | client | MCP |
|---|---|---|---|---|
| list maps | `cogmap list` | `GET /api/cognitive-maps` | `list()` | `cogmap_list` |
| show map | `cogmap show <ref>` | `GET /api/cognitive-maps/{id}` | `show(id)` | `cogmap_show` |
| multi-map search | `search --cogmap A --cogmap B` | `SearchParams.cogmap_ids` | pass-through | `search` |
| multi-map list | `list --cogmap A --cogmap B` | list params | pass-through | `list_resources` |

## Generated-artifact impact

`CogmapRow`, `CogmapDetail`, `CogmapFoundationRow`, and the `SearchParams.cogmap_ids` field are all
wire DTOs, so the three router-derived artifacts restale and must be regenerated with
`cargo make openapi` and **staged** (the drift gates compare against git, not a fresh build):

- `openapi.json`
- the temper-rb gem under `clients/temper-rb/lib/temper/generated`
- `clients/temper-ts/src/generated/schema.ts`

`CogmapRow` et al. also carry `ts-rs` derives, so `cargo make generate-ts-types` writes the
`search.ts`/cognitive-maps TS types and `ts-rs-drift` must pass (a newly-exported type is exactly the
untracked-file case that gate is built to catch). `SearchScope` is **not** widened, so the temper-rb
`search_scope.rb` enum (which `raise`s on unknown values) is safe.

## Testing

- **Unit:** `build_search_params` populates `cogmap_ids` from repeated `--cogmap`; the multi-map
  `--context`/`--cogmap` and multi-map `--wayfind` guards; `CogmapRow`/`CogmapDetail` render as
  objects/arrays with the injected `ref`.
- **e2e (`test-e2e-embed`, since ingest/search touch ONNX):**
  - `cogmap list` returns only visible maps; a map joined to a team the caller is not in is absent.
  - `cogmap show` returns charter + foundations with the telos flagged; an unreadable map is 404.
  - multi-map search returns the union of two maps' homed resources; a deny map in the set adds
    nothing.
  - `resource list --cogmap A --cogmap B` lists resources homed in either map, and only those the
    caller can see.
- **Substrate:** `cogmap_scope_ids_multi` unions correctly and denies per-element; `cogmap_list_rows`
  returns the statement and nulls it for an empty charter.

## Acceptance

- A principal can, in one CLI command **and** one MCP call, list every cognitive map they can reach,
  see what each is for (charter statement), and copy a ref to address it — with no prior knowledge of
  any map id.
- `cogmap show` presents a map's charter and the resources it is built on in one read, telos
  distinguished from the rest.
- `--cogmap` accepts multiple maps on both `search` and `resource list`; the corpus is the union of
  the chosen maps' visible homed resources; the wire change is additive and no existing client breaks.
- Every new read is access-scoped through the existing visibility predicates; an unreachable map is
  invisible (list/scope) or 404 (show), never a leak.
- The three router artifacts and the ts-rs trees are regenerated and staged; `cargo make check` and
  the embed-gated e2e suite are green.

## Non-goals

- **Region-anchor / centrality surveys in `cogmap show`.** Foundations are the homed set; the region
  tier is the sibling goal's territory and layers on later.
- **Any change to `SearchScope`** or to `graph_home_cogmaps`' shape (the Atlas consumer).
- **New authorization semantics.** Nothing here mints, widens, or reinterprets a grant.
