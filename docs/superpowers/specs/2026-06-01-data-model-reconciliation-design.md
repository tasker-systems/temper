# Data-Model Reconciliation: Neutral Kernel, Domain-A Opinionation, and the `kb_properties` Canonical Model

**Date:** 2026-06-01
**Status:** Design — approved in brainstorming, pending plan
**Goal:** `substrate-kernel-to-cognitive-map`, Arc 1 (shared-kernel completion)
**Supersedes (framing only):** the in-place migration framing of `2026-05-27-access-wrapper-extraction-and-polymorphic-projection-substrate`
**Extends:** `2026-06-01-the-shared-kernel-boundary-temper-substrate-beneath-two-domains-workflow-kb-and-cognitive-map`

> **⚠ Vocabulary + cross-spec note (added 2026-06-04, coherence pass; CS-3 swept the same day).** This
> spec was written in the `scope` vocabulary; the **CS-3 terminology sweep landed 2026-06-04**, so the body
> now reads in the settled `kb_cogmaps` / `cogmap_*` vocabulary (canonical record + naming rationale:
> [`2026-06-02-map-regions-self-materialized-shape-surface-design.md`](2026-06-02-map-regions-self-materialized-shape-surface-design.md) §0).
> The built `kb_events.scope_id` producing-anchor column is left named as-built where it's referenced (its
> rename is a migration-time concern, not a spec-vocabulary one). Two **substantive** reconciliations —
> *not* rename drift — also surfaced in the same pass and **remain open**: the `kb_resource_access`
> grant-anchor set vs the resolved access model (see the note in §2) and `resource.body_hash`'s home after
> manifest dissolution meets the content-block block-merkle (see the note in §3).

## Context

The shared-kernel boundary decision (2026-06-01) settled *that* temper becomes a permanent
`temper-substrate` kernel beneath two domains (Domain A = workflow+KB; Domain B = cognitive-map).
It explicitly deferred three things; this spec settles the first, and part of the third's crate shape:

1. **Data-model reconciliation** — the concrete table-DDL revisions to the 2026-05-27 access-wrapper
   given the June-1 *doctype→behavior* turn, and the kernel-vs-Domain-A service split.
2. ~~Domain-B table design~~ — **out of scope** (spine #2, a successor spec).
3. **Crate-extraction shape** — *in scope here* to the extent of: the `temper-substrate` crate API
   (the substrate-command base), what peels into `temper-api`, and where Domain-A opinionation lands.
   The actual migration phase-ordering (sequencing) is **out of scope** (a plan-level decision).

### What the codebase actually looks like at design time

- **The access-wrapper (Limb 1c) is unbuilt.** Migrations stop at `20260522100002_edges_as_projection.sql`.
  No `kb_resource_homes`, `kb_resource_access`, polymorphic `kb_edges`/`kb_properties`,
  `kb_teams_parents`, or `kb_team_cogmaps` exist. This is a pure design exercise — there is no
  on-disk schema to reconcile against, only the 2026-05-27 design document.
- **`operations::Backend` is minimal** — six resource methods (create/show/update/delete/list/search).
  The relationship commands (`AssertRelationship`/`Retype`/`Reweight`/`Fold`) exist as command *structs*
  in `temper-core::operations::commands` but are **not on the trait** yet.
- **Domain-A opinionation is distributed:** the *shared* logic mostly lives in `temper-core`
  (`frontmatter/`, `schema.rs`, `vault.rs`, `defaults.rs`, the seven `schemas/*.schema.json`), with
  enforcement call-sites symmetrically on every surface — `temper-cli`, `temper-api` ingest/resource
  services, and `temper-mcp`. Workflow writes are inherently **multi-surface**.

### The refinement of the boundary decision

The boundary decision drew `temper-api = Domain A`. Pressure-testing the service split surfaced a
question that framing collapsed — *how thick is `temper-api`?* The decision taken here:

> **`temper-api` is domain-neutral.** It exposes generalized affordances over the substrate (resources
> with arbitrary properties, edges, search, access). The workflow opinionation (doctype, frontmatter,
> vault projection) moves *up* into a new `temper-workflow` crate consumed by CLI/MCP/UI. Domain B is a
> symmetric consumer of the same neutral substrate. The server stops being a doctype-schema backstop;
> doctype enforcement becomes a client-side contract in `temper-workflow`.

This is the logical endpoint of "doctype is demoted": neither the kernel nor the neutral API has any
concept of a "task" or a "session" — those are interpretations applied by a domain layer.

## Crate Topology

Two kernel crates (not three — `temper-events` folds into `temper-substrate`), a neutral API tier, and
a Domain-A opinionation crate:

```
KERNEL (domain-neutral)
  temper-core       kernel types ONLY (no DB driver): resource identity, edge/event/access
                    types, ids, hash, error, operations command-base + Backend trait.
                    sqlx slimmed to derive-only (drop the postgres driver + runtime-tokio
                    features it carries today). Consumed by EVERY surface.
  temper-substrate  NEW. The DB-bearing kernel: a `ledger` module (absorbing temper-events) +
                    the access-wrapper tables + access fns + polymorphic projection. Takes a
                    PgConnection (caller-owned-connection pattern). ALL kernel SQL lives here.
                    Impls the substrate-command base against Postgres.
                    deps: temper-core
  temper-ingest     content→vectors (extraction + embedding). Unchanged; domain-neutral.
  temper-api        NEUTRAL HTTP server over the substrate. No doctype semantics.
                    deps: temper-substrate, temper-core, temper-ingest
  temper-client     auth-aware HTTP client to the neutral API. deps: temper-core

DOMAIN A (workflow + KB)
  temper-workflow   NEW. doctype schemas + registry, typed workflow fields, frontmatter
                    PROJECTION (one-way), defaults, vault projection, sync/manifest, templates.
                    Decorates a Backend with client-side doctype enforcement. ts-rs types here
                    feed the SvelteKit UI. deps: temper-core, temper-client
  temper-cli / mcp  deps: temper-workflow, temper-client, temper-core
  temper-ui (TS)    ts-rs types from temper-core (neutral) + temper-workflow (Domain-A)

DOMAIN B (cognitive-map) — named only; tables are spine #2
  temper-cogmap     NEW. deps: temper-substrate, temper-llm
```

### The moves vs. today

- **`temper-events` → `temper-substrate::ledger`.** The ledger's only in-tree consumer is the
  substrate (and `temper-cogmap`, transitively). No surface wants the bare ledger. Kept as a
  clearly-bounded module so the caller-owned-connection seam and test-isolation survive; re-extraction
  is trivial if a real reuse case appears.
- **`temper-core` slims.** Its Domain-A modules (`frontmatter/`, `schema.rs`, `vault.rs`, `defaults.rs`,
  `schemas/*.json`) emigrate to `temper-workflow`. What remains is neutral kernel types + the command
  base. Its `sqlx` dependency drops to derive-only features — a real weight win for the lightweight
  consumers (`cli`/`client`/`mcp`) that currently inherit the full Postgres driver + multi-thread tokio
  runtime just for `FromRow` derives.
- **`temper-api` sheds doctype.** The ~10 files referencing `frontmatter`/`ManagedMeta`/`doc_type`/`schema`
  lose those references; the kernel SQL relocates into `temper-substrate`; handlers become thin neutral
  pass-throughs.

## The Three-Bucket Service Split

Each of today's 14 services lands in one of three buckets. The throughline: **anything that reads/writes
identity, content, edges, properties, or access → substrate; anything that knows what a "task" *is* →
`temper-workflow`; `temper-api` is the thin neutral HTTP pass-through between them.**

| Service | Bucket | Notes |
|---|---|---|
| event | substrate | the `ledger` module |
| edge | substrate | polymorphic projection |
| relationship | substrate | edge assert/retype/reweight/fold = the edge command layer |
| graph | substrate | traversal / neighbors over edges |
| access | substrate | `resources_visible_to`, `resources_accessible_to_cogmap`, grants |
| profile | substrate | kernel identity |
| context | substrate | `kb_contexts` = a kernel anchor type in the access wrapper |
| **resource** | **SPLITS** | identity/content/homes/access SQL → substrate; typed-field assembly + frontmatter projection → `temper-workflow` |
| **meta** | **SPLITS** | raw hashes + property storage → substrate; typed-view assembly → `temper-workflow` |
| **search** | **SPLITS** | FTS+vector projection query → substrate (exposed neutrally by API); doctype-faceting → `temper-workflow` |
| **ingest** | **SPLITS** | generic chunk+embed+store → API / `temper-ingest` (doctype-blind); frontmatter/doctype prep → `temper-workflow` (client-side, pre-POST) |
| doc_type | **temper-workflow (wholly)** | no kernel half — doctype is represented in the kernel *only* as an opaque `kb_properties` facet; registry + schemas + ts-rs types all Domain-A |
| sync | **temper-workflow** | vault projection / manifest is pure Domain-A (reads the substrate's `last_event_id` for staleness, but the projection logic is Domain-A) |

## DDL Revisions

> **Grounding note.** This section is written against the **actual current schema** (the base
> `20260330000001_consolidated_schema.sql` plus all 39 later migrations, reconciled), **not** the
> 2026-05-27 design document — which proposed a `kb_resources` shape (`body text`, `content_hash`,
> `mimetype`) that was never built and conflicts with reality. The polymorphic-projection *substance*
> of 2026-05-27 (the `(anchor_table, anchor_id)` discriminator, homes-vs-access split, producer/consumer
> access bifurcation, teams-DAG + recursive CTE) is preserved; the table shapes below are the real
> transformations.

### Current reality the access-wrapper transforms

- `kb_resources` today is already lean: `(id, kb_context_id, kb_doc_type_id, origin_uri [no longer
  unique], title, slug [nullable], originator_profile_id, owner_profile_id, is_active, created, updated)`.
  `content_hash`/`mimetype`/`resource_mode` were **already dropped** in `20260404000002` — so the spec's
  earlier "drop `resource_mode`" item is moot; it's gone.
- **Content** is externalized: `kb_chunks` → `kb_chunk_content` (TOAST), versioned via
  `kb_resource_revisions`, rendered through the `kb_current_chunks` view. There is no `body` column.
- **Frontmatter** lives in `kb_resource_manifests.{managed_meta, open_meta}` (JSONB) with
  `{body,managed,open}_hash`.
- **`kb_cogmaps` already exists** (`id, name, porosity`), and `kb_events` is already the unified
  append-only ledger (`event_type_id`→`kb_event_types`, `topic_id`, `scope_id`, `correlation_id`).

### 1. `kb_resources` slims to identity; anchor/ownership move to homes; doctype + slug leave entirely

```sql
kb_resources (
    id          uuid pk,            -- uuid_generate_v7
    title       text not null,
    origin_uri  text not null,      -- canonical source uri; not unique
    is_active   boolean not null default true,
    created     timestamptz not null,
    updated     timestamptz not null
);
```

Moves **out** of `kb_resources`:
- `kb_context_id`, `owner_profile_id`, `originator_profile_id` → `kb_resource_homes` (navigation/anchor)
- `kb_doc_type_id` → **dropped**; doctype becomes a `kb_properties` row `key='doc_type'` (see §3)
- `slug` → **dropped entirely** (see §4 — slug retirement)

### 2. `kb_resource_homes` / `kb_resource_access` (the access wrapper)

> **⚠ PROVISIONAL — gated on the access/capability-model spec.** The `kb_resource_access` shape below,
> and specifically the `access_level` enum (`vault | mutable | immutable`) it reuses, are **not final**.
> Research (see the access/RBAC problem-shape, sibling spec
> `2026-06-02-access-capability-model-design`) established that `vault/mutable/immutable` is a vault-era
> artifact that double-encodes a single write boolean with `team_role`, is undefined across the
> polymorphic anchor set, and ignores a second orthogonal axis (resolution-permeability). The
> **navigation-vs-grant split, the polymorphic anchors, and the producer/consumer functions are stable**;
> the *capability vocabulary* on grants is being redesigned. Treat `access_level` here as a placeholder
> until that spec lands. `kb_resource_homes` is unaffected.

```sql
kb_resource_homes (                  -- navigation: where a resource lives. one per resource.
    id                    uuid pk,
    resource_id           uuid not null unique references kb_resources(id),
    anchor_table          varchar(64) not null check (anchor_table in ('kb_contexts','kb_cogmaps')),
    anchor_id             uuid not null,
    originator_profile_id uuid not null references kb_profiles(id),
    owner_profile_id      uuid not null references kb_profiles(id),
    created               timestamptz not null default now()
);
create index idx_kb_resource_homes_anchor on kb_resource_homes(anchor_table, anchor_id);

kb_resource_access (                 -- additive grants beyond the home anchor. subsumes kb_team_resources.
    id                    uuid pk,
    resource_id           uuid not null references kb_resources(id),
    anchor_table          varchar(64) not null
                            check (anchor_table in ('kb_contexts','kb_cogmaps','kb_teams','kb_profiles')),
    anchor_id             uuid not null,
    access_level          access_level not null,   -- existing enum: vault | mutable | immutable
    granted_by_profile_id uuid not null references kb_profiles(id),
    granted_at            timestamptz not null default now(),
    unique (resource_id, anchor_table, anchor_id)
);
create index idx_kb_resource_access_anchor   on kb_resource_access(anchor_table, anchor_id);
create index idx_kb_resource_access_resource on kb_resource_access(resource_id);
```

The home anchor confers implicit `vault` access; `kb_resource_access` extends to additional anchors.
`kb_resource_homes` has **no `slug`** (slug retired) and therefore no `(anchor, slug)` uniqueness — the
only resource identity is the UUID PK. Teams-DAG (`kb_teams_parents`), `kb_team_cogmaps`, and the
producer/consumer access functions (`resources_visible_to`, `resources_accessible_to_cogmap`) carry from
2026-05-27, rewritten against these tables.

> **⚠ Reconciliation item A2 — grant-anchor set vs the resolved access model (added 2026-06-04, coherence
> pass).** The `kb_resource_access.anchor_table` check above admits `kb_cogmaps` as a grantee
> anchor. The access/capability spec — which **un-gates** this table — resolved a model where additive
> grants are **teams-RBAC only** (individual→team, team→team), and **maps do not receive per-resource
> grants**: a map's read-reach is *computed* (`resources_accessible_to_cogmap` = the DAG-expanded team
> intersection), and there is explicitly *no `grant` at the concept level*
> ([`2026-06-02-access-capability-model-design.md`](2026-06-02-access-capability-model-design.md) §2/§4).
> So the grantee anchors should be `kb_teams` / `kb_profiles` (and possibly `kb_contexts`); `kb_cogmaps` /
> `kb_cogmaps` as a `kb_resource_access` grantee contradicts the maps-read-via-intersection model. The access
> spec un-gated the table but never restated the corrected anchor set inline — reconcile when the DDL is
> written (the access spec carries a reciprocal pointer in its §2).

### 3. `kb_properties` — the canonical structured-meta model (single shape, non-null key)

No `property_kind` enum. Every property is a non-null `(key, value)` pair; a bare keyword/tag is named
explicitly as `key='tag'`. This kills the nullable-in-`UNIQUE` smell (Postgres treats NULLs as distinct,
so a nullable key never dedups) and preserves the symmetric salience-overlap self-join (one shape, both
resource-side and cogmap-side). `is_folded` is the **event-projection soft-retract**, identical to the
built `kb_resource_edges` pattern: a `property_retracted` event folds the row; live reads filter
`WHERE NOT is_folded`; the row survives for event-history correspondence.

```sql
kb_properties (
    id                    uuid pk default uuid_generate_v7(),
    owner_table           varchar(64) not null check (owner_table in ('kb_resources','kb_cogmaps')),
    owner_id              uuid not null,
    property_key          text not null,
    property_value        jsonb not null,
    weight                float not null default 1.0,   -- meaningful for salience; 1.0 for plain metadata
    asserted_by_event_id  uuid not null references kb_events(id),
    last_event_id         uuid not null references kb_events(id),
    is_folded             boolean not null default false,
    created               timestamptz not null default now(),
    unique (owner_table, owner_id, property_key, property_value)
);
create index idx_kb_properties_owner    on kb_properties(owner_table, owner_id) where not is_folded;
create index idx_kb_properties_value_gin on kb_properties using gin (property_value jsonb_path_ops);
create index idx_kb_properties_key      on kb_properties(property_key) where not is_folded;
```

**Reserved keys (a documented convention, not DDL):**

| key | meaning | set by |
|---|---|---|
| `doc_type` | the demoted type tag (`"task"`, `"session"`, …) | `temper-workflow` on create |
| `tag` | a bare salience keyword (the named ex-null state); multiple rows per owner | any surface |
| `behavior:*` | triage-time behavior signal (Domain B) | triage / authoring |
| workflow fields (`stage`, `mode`, `effort`, `seq`, …) | typed Domain-A metadata | `temper-workflow` |
| arbitrary keys | former `open_meta` user metadata | any surface |

**`kb_resource_manifests` dissolves into `kb_properties`.** Its `managed_meta`/`open_meta` JSONB keys
backfill as `kb_properties` rows (via genesis events, `intent=migration`, mirroring the
`edges_as_projection` backfill). `body_hash` already lives on `kb_resource_revisions`;
`managed_hash`/`open_hash` were frontmatter-tier sync aids and become obsolete under cloud-only — they
retire with the sync rework, not here.

> **⚠ Reconciliation item A1 — `resource.body_hash`'s home after manifest dissolution (added 2026-06-04,
> coherence pass).** This spec dissolves `kb_resource_manifests` and parks `body_hash` on
> `kb_resource_revisions`. The sibling
> [`2026-06-03-content-block-primitive-design.md`](2026-06-03-content-block-primitive-design.md) goes the
> other way: it **retires `kb_resource_revisions`** (→ `kb_block_revisions` at block grain) and keeps
> `kb_resource_manifests.body_hash` as the resource-level sync hash, redefined as a **merkle over the
> ordered `(block_id, block_body_hash)` tuples**. The two specs thus point `body_hash` at **opposite
> survivors** — after both land, *neither* `kb_resource_manifests` nor `kb_resource_revisions` exists, and
> the resource-level sync hash `sync_diff_for_device` reads is unowned. Decide its post-both home (a
> denormalized column on `kb_resources`, a reserved `kb_properties` row, or composed-on-read) when these
> two specs are sequenced together. Cross-ref: content-block Plan-level Q3.

### 4. Slug retirement — UUID is the sole resource identity

`kb_resources.slug` and any homes-level slug are **dropped**. Resolution is already UUID-first
(`resource_for_uri` casts the trailing segment to UUID before any slug fallback; `kb_resource_uri` emits
`COALESCE(slug, id::text)`), so this formalizes existing behavior. The human/agent-friendly identifier
becomes a **render-time decoration** — Notion-style `sluggify(title)-<uuid>` — produced and parsed by
`temper-workflow`, never stored as a key. (`kb_profiles.slug` / `kb_teams.slug` are *owner sigils*
`@me` / `+team` and are **out of scope** — left as-is.)

This is a *direction commitment* with a deliberately split blast radius (per the agreed scope): the
kernel DDL change (drop the columns/constraints) lands here; the `ResourceRef` collapse and the
CLI/MCP/skill identifier-UX rework are a tracked Domain-A follow-up spec.

### 5. `kb_edges` — smaller than 2026-05-27 implied

The built `kb_resource_edges` is **already** the event-sourced projection shape: it carries `edge_kind`
(`express|contains|leads_to|near`), `polarity` (`forward|inverse`), `label`, `asserted_by_event_id`,
`last_event_id`, `is_folded`. The only transformation is **adding source/target polymorphism** plus a
nullable cognitive-map `scope_id`:

```sql
alter table kb_resource_edges rename to kb_edges;
alter table kb_edges
    add column source_table varchar(64) not null default 'kb_resources'
        check (source_table in ('kb_resources','kb_cogmaps')),
    add column target_table varchar(64) not null default 'kb_resources'
        check (target_table in ('kb_resources','kb_cogmaps')),
    add column scope_id uuid references kb_cogmaps(id);   -- nullable; cognitive-map-layer edges
-- widen uq_resource_edge and the source/target indexes to include the *_table discriminators.
```

> **⚠ Superseded (added 2026-06-04, coherence pass).** The nullable `scope_id` column above is **superseded
> by the access spec §3 polymorphic edge-home** `(anchor_table, anchor_id)` with `anchor_table ∈
> ('kb_contexts','kb_cogmaps')`. The `scope_id` column does **not** survive — an edge homes in the
> same resource-terms as everything else, gated by `edges_visible_to`. (Already recorded in
> [`map-regions`](2026-06-02-map-regions-self-materialized-shape-surface-design.md) §0's edge-home note.)
> The `source_table`/`target_table` *endpoint* polymorphism is unaffected (endpoints now `('kb_resources','kb_cogmaps')`).

Relational frontmatter fields (e.g. a task's `goal`) project to `kb_edges` rows, not `kb_properties`.

### 6. Affected SQL functions (the slug + doctype blast radius)

Dropping slug and demoting doctype to a property forces rewrites of the functions that join
`kb_doc_types` or read `r.slug`. The spec records the surface; the SQL is plan/implementation work:

- **Slug-bearing:** `fts_search` / `rebuild_resource_search_vector` (drop slug from tsvector weight-A,
  fall back to `title` alone), `unified_search` / `graph_search` / `graph_subgraph_nodes` /
  `graph_resource_edges` / the `vault_resources_browse` view (drop the returned `slug` column).
- **Doctype-bearing:** `graph_subgraph_nodes` / `graph_search` doctype filters and
  `graph_subgraph_nodes`'s `kb_resource_manifests` stage read become `kb_properties` lookups
  (`key='doc_type'`, `key='stage'`); the `kb_doc_types` joins disappear.
- **Addressing:** `kb_resource_uri` / `resource_for_uri` — UUID resolution stays kernel; the
  human-readable `kb://sigil/context/doctype/slug` *construction* moves to `temper-workflow` (addressing
  is Domain-A under the neutral-API decision). `kb_doc_types` leaves the kernel entirely.

## The Command-Base Seam (Crate-Extraction Shape)

`temper-core::operations::Backend` is already the write seam every surface dispatches through. This
spec makes it the **neutral substrate-command base** and adds a decorator tier for Domain A.

### The base vocabulary (`temper-core::operations`, neutral)

- resource: `CreateResource / Show / Update / Delete / List`
- edge: `AssertRelationship / Retype / Reweight / Fold` (structs exist; this adds them to the trait)
- property: `AssertProperty / Retract / Reweight` (new — the `kb_properties` command layer)
- access: `Grant / Revoke` (new)
- `SearchResources` (neutral FTS+vector)

```rust
// no PropertyKind enum — every property is a non-null (key, value) pair; a bare tag is key = "tag".
pub struct PropertyAssertion { pub key: String, pub value: Value, pub weight: f64 }
pub struct Property { /* row form: + owner, asserted/last event ids, is_folded, … */ }
```

### The one real refactor: genericize the commands off `ManagedMeta`

`CreateResource`/`UpdateResource` today carry `managed_meta: ManagedMeta` — a Domain-A type. For the base
to be neutral, those fields genericize to property/edge assertions. The kernel no longer knows what a key
*means*; it stores facets and edges.

### Impl'd twice, decorated once

- **`temper-substrate`** impls `Backend` against a `PgConnection` (the kernel SQL; today's `DbBackend`
  SQL relocates here, composing the `ledger` module).
- **`temper-client`** impls `Backend` over HTTP (the existing `CloudBackend`, talking to the neutral
  `temper-api`).
- **`temper-workflow`** *decorates* a `Backend`: before dispatch it runs doctype defaults, builds the
  `doc_type` + workflow-field facets and relational edges, and constructs URI addressing — translating
  "create a `task` with these required fields" into neutral substrate commands. `cli`/`mcp` talk only to
  `temper-workflow`.

Data flow for a workflow write:
`cli → temper-workflow` (doctype opinionation, build facets/edges) `→ temper-client` (HTTP)
`→ temper-api` (neutral handler) `→ temper-substrate::Backend` (kernel SQL).
Domain B's eventual path is the symmetric `temper-cogmap → temper-substrate::Backend` in-process.

## Frontmatter as a One-Way Projection

There is no `Frontmatter` round-trip type and no DB tier awareness. Frontmatter is a **render-only**
projection living in `temper-workflow`, serving `show` and context-projection-to-disk (the read-only
vault):

```rust
fn render_frontmatter(props: &[Property], edges: &[Edge]) -> YamlFrontmatter   // DB → YAML, one-way
```

Conversions in `temper-workflow` are strictly directional:

- **write:** typed workflow input → `Vec<PropertyAssertion>` + `Vec<EdgeAssertion>`
  (scalar fields → `kb_properties` rows; relational fields like `goal`/links → `kb_edges`). A thin
  newtype satisfies the orphan rule (`impl From<&WorkflowFields> for PropertyAssertions`).
- **read:** properties/edges → a typed view (for `--stage` display etc.) or → YAML (for the vault).

Never a tier round-trip. YAML key order is **not** preserved (`kb_properties` is a set) — acceptable
under "files are derivative projection artifacts."

## Consequences & Committed Rules

- **SQL-ownership rule revision** (from the boundary decision): "all SQL in `temper-api/services`" →
  "**kernel SQL in `temper-substrate`; each domain owns its domain SQL.**" Update the code-quality
  sections of `temper/CLAUDE.md` and `temper-api/CLAUDE.md` when this lands.
- **No server-side doctype backstop.** The CLAUDE.md "schema-required defaults at create/update" rule
  becomes a `temper-workflow` client-side contract. The symmetric send/receive enforcement collapses to
  send-side only.
- **URI addressing** is a `temper-workflow` affordance: it builds the human-readable
  `kb://sigil/context/doctype/sluggify(title)-<uuid>` by reading the `doc_type` property and the title.
  The kernel resolves and addresses purely by UUID + anchor.
- **Provenance** (system- vs user-set) is answered by the event ledger, not a tier column.
- **`temper-core` sqlx slimming** is part of this work, not a follow-up.
- **`resource_mode` is already gone** (`20260404000002`) — no action; noted to correct the record.

## Out of Scope

- **Domain-B table design** (telos-as-`kb_properties`-facet, questions-as-resources, regulation-as-
  resource, `express`/`near` edge semantics) — spine #2, successor spec. **Carve-out (added 2026-06-03):**
  an access-gated *projection* read cross-map through the kernel access layer is **not** a spine-#2 table —
  it lives in `temper-substrate`. The test: **does the kernel interpret the content?** No → kernel
  access-gated projection; Yes → spine-#2 / Domain-B. `kb_cogmap_regions`
  (`2026-06-02-map-regions-self-materialized-shape-surface-design`) is the first instance — the kernel
  stores + access-gates regions but never clusters or interprets them.
- **Migration phase-ordering / sequencing** (build Limb 1c → extract `temper-substrate` → birth
  `temper-cogmap`) — spine #3, a plan-level decision. This spec defines the *target* shape, not the
  path to it.
- **Slug-retirement surface rework** — the `ResourceRef::Scoped(owner,context,doctype,slug)` collapse to
  UUID, the Notion-style `Title-<uuid>` identifier format/parsing, and the fuzzy title-lookup ergonomics
  across CLI/MCP/skill. This spec commits the *kernel-side* slug drop (§4); the surface UX is a tracked
  Domain-A follow-up spec, since it touches every command.
- **Domain-B operational tables** and whether they earn a `cogmap.*` schema namespace.
- **The access/capability (RBAC) model** — retiring/replacing `access_level` (vault/mutable/immutable),
  resolving its overlap with `team_role`/`watcher`, representing the two orthogonal axes (read/write
  porosity × resolution-permeability), cogmap `contains`-hierarchy visibility, and default-safety/cogmap
  lifecycle. This is its own foundational sibling spec (`2026-06-02-access-capability-model-design`).
  **This spec's `kb_resource_access`/`access_level` DDL is gated on its outcome** (see §2) — the
  navigation/grant/producer-consumer backbone is stable; the capability vocabulary is not.

## Connections

- Extends: `2026-06-01-the-shared-kernel-boundary-temper-substrate-beneath-two-domains-workflow-kb-and-cognitive-map`
- Supersedes (framing): `2026-05-27-access-wrapper-extraction-and-polymorphic-projection-substrate`
- Conceptual lineage: `2026-06-01-seed-skill-scope-portable-vs-bound-awareness-access-bounded`,
  `2026-05-31-definitional-fallacy-concept-as-basin-telos-resolves-threshold-primitive`,
  `2026-05-31-temper-confidence-inventory`
- Code anchors: `temper-core::operations` (the command base), `temper-events` (the ledger to absorb),
  `temper-llm` (Domain B's engine)
