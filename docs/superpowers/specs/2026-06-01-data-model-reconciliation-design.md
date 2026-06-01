# Data-Model Reconciliation: Neutral Kernel, Domain-A Opinionation, and the `kb_properties` Canonical Model

**Date:** 2026-06-01
**Status:** Design — approved in brainstorming, pending plan
**Goal:** `substrate-kernel-to-cognitive-map`, Arc 1 (shared-kernel completion)
**Supersedes (framing only):** the in-place migration framing of `2026-05-27-access-wrapper-extraction-and-polymorphic-projection-substrate`
**Extends:** `2026-06-01-the-shared-kernel-boundary-temper-substrate-beneath-two-domains-workflow-kb-and-cognitive-map`

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
  `kb_teams_parents`, or `kb_team_scopes` exist. This is a pure design exercise — there is no
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
| access | substrate | `resources_visible_to`, `resources_accessible_to_scope`, grants |
| profile | substrate | kernel identity |
| context | substrate | `kb_contexts` = a kernel anchor type in the access wrapper |
| **resource** | **SPLITS** | identity/content/homes/access SQL → substrate; typed-field assembly + frontmatter projection → `temper-workflow` |
| **meta** | **SPLITS** | raw hashes + property storage → substrate; typed-view assembly → `temper-workflow` |
| **search** | **SPLITS** | FTS+vector projection query → substrate (exposed neutrally by API); doctype-faceting → `temper-workflow` |
| **ingest** | **SPLITS** | generic chunk+embed+store → API / `temper-ingest` (doctype-blind); frontmatter/doctype prep → `temper-workflow` (client-side, pre-POST) |
| doc_type | **temper-workflow (wholly)** | no kernel half — doctype is represented in the kernel *only* as an opaque `kb_properties` facet; registry + schemas + ts-rs types all Domain-A |
| sync | **temper-workflow** | vault projection / manifest is pure Domain-A (reads the substrate's `last_event_id` for staleness, but the projection logic is Domain-A) |

## DDL Revisions

The 2026-05-27 access-wrapper DDL carries **verbatim** except for the deltas below. The polymorphic-
projection substance the boundary decision committed to preserving — `(anchor_table, anchor_id)`
discriminator polymorphism, homes-vs-access split, producer/consumer access bifurcation, teams-DAG +
recursive CTE, the access functions — is unchanged.

### 1. `kb_resources` drops `kb_doc_type_id`

The kernel has no doctype concept. `kb_resources` is pure identity + content:

```sql
kb_resources (
    id             uuid pk default uuid_generate_v7(),
    title          text not null,
    body           text,
    content_hash   varchar(64),
    mimetype       varchar(128),
    resource_mode  varchar(16) not null default 'added' check (resource_mode in ('added','imported')),
    is_active      boolean not null default true,
    created        timestamptz not null,
    updated        timestamptz not null
);
```

### 2. `kb_doc_types` is removed from the kernel schema

The doctype catalog (the seven `*.schema.json` + descriptions) becomes `temper-workflow` code/config.
If a server-side doctype registry table is ever wanted, it is a Domain-A-owned table under the revised
SQL-ownership rule. YAGNI for now.

### 3. `kb_properties` is the canonical structured-meta model (two-variant kind)

`property_kind` carries **no** frontmatter-tier awareness. The managed/open distinction only ever
earned its keep when a human could hand-edit header YAML and expect round-trip; under cloud-only every
write goes through the API and provenance lives in the event ledger (`asserted_by_event_id`). So the
enum is exactly the 2026-05-27 salience taxonomy:

```sql
create type property_kind as enum ('keyword', 'facet');
--   facet    → key-value structured meta. doc_type, behavior:*, the typed workflow fields
--              (stage/mode/effort/…), and arbitrary user metadata all live here.
--   keyword  → bare salience tag (no key).

kb_properties (
    id                    uuid pk default uuid_generate_v7(),
    owner_table           varchar(64) not null check (owner_table in ('kb_resources','kb_scopes')),
    owner_id              uuid not null,
    property_kind         property_kind not null,
    property_key          text,              -- required for facet; null for keyword
    property_value        jsonb not null,
    weight                float not null default 1.0,
    asserted_by_event_id  uuid not null references kb_events(id),
    last_event_id         uuid not null references kb_events(id),
    is_folded             boolean not null default false,
    created               timestamptz not null default now(),
    unique (owner_table, owner_id, property_kind, property_key, property_value),
    check ((property_kind = 'keyword') = (property_key is null))
);
create index idx_kb_properties_owner    on kb_properties(owner_table, owner_id) where not is_folded;
create index idx_kb_properties_value_gin on kb_properties using gin (property_value jsonb_path_ops);
create index idx_kb_properties_kind_key  on kb_properties(property_kind, property_key) where not is_folded;
```

**Reserved facet keys (a documented convention, not DDL):**

| key | meaning | set by |
|---|---|---|
| `doc_type` | the demoted type tag (`"task"`, `"session"`, …) | `temper-workflow` on create |
| `behavior:*` | triage-time behavior signal (Domain B) | triage / authoring |
| workflow fields (`stage`, `mode`, `effort`, `seq`, …) | typed Domain-A metadata | `temper-workflow` |
| arbitrary keys | former `open_meta` user metadata | any surface |

### 4. Slug uniqueness drops doctype

`kb_resource_homes` keeps the 2026-05-27 `unique (anchor_table, anchor_id, slug)`. With doctype gone
from the kernel, slug uniqueness is **context-wide and doctype-independent** — a `task` and a `goal`
named `foo` in one context would collide. Decision: accept context-wide slug uniqueness;
`temper-workflow` is responsible for choosing unique slugs. (Alternative — fold doctype into the stored
slug — rejected as leaking Domain-A semantics back into the kernel key.)

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
pub enum PropertyKind { Facet, Keyword }
pub struct PropertyAssertion { pub kind: PropertyKind, pub key: Option<String>, pub value: Value, pub weight: f64 }
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
  (scalar fields → facets; relational fields like `goal`/links → edges). A thin newtype satisfies the
  orphan rule (`impl From<&WorkflowFields> for PropertyAssertions`).
- **read:** facets/edges → a typed view (for `--stage` display etc.) or → YAML (for the vault).

Never a tier round-trip. YAML key order is **not** preserved (`kb_properties` is a set) — acceptable
under "files are derivative projection artifacts."

## Consequences & Committed Rules

- **SQL-ownership rule revision** (from the boundary decision): "all SQL in `temper-api/services`" →
  "**kernel SQL in `temper-substrate`; each domain owns its domain SQL.**" Update the code-quality
  sections of `temper/CLAUDE.md` and `temper-api/CLAUDE.md` when this lands.
- **No server-side doctype backstop.** The CLAUDE.md "schema-required defaults at create/update" rule
  becomes a `temper-workflow` client-side contract. The symmetric send/receive enforcement collapses to
  send-side only.
- **URI addressing** (`kb://owner/context/doctype/uuid`) is a `temper-workflow` affordance, reading the
  `doc_type` facet. The kernel addresses by UUID + anchor.
- **Provenance** (system- vs user-set) is answered by the event ledger, not a tier column.
- **`temper-core` sqlx slimming** is part of this work, not a follow-up.

## Out of Scope

- **Domain-B table design** (telos-as-`kb_properties`-facet, questions-as-resources, regulation-as-
  resource, `express`/`near` edge semantics) — spine #2, successor spec.
- **Migration phase-ordering / sequencing** (build Limb 1c → extract `temper-substrate` → birth
  `temper-cogmap`) — spine #3, a plan-level decision. This spec defines the *target* shape, not the
  path to it.
- **Domain-B operational tables** and whether they earn a `cogmap.*` schema namespace.

## Connections

- Extends: `2026-06-01-the-shared-kernel-boundary-temper-substrate-beneath-two-domains-workflow-kb-and-cognitive-map`
- Supersedes (framing): `2026-05-27-access-wrapper-extraction-and-polymorphic-projection-substrate`
- Conceptual lineage: `2026-06-01-seed-skill-scope-portable-vs-bound-awareness-access-bounded`,
  `2026-05-31-definitional-fallacy-concept-as-basin-telos-resolves-threshold-primitive`,
  `2026-05-31-temper-confidence-inventory`
- Code anchors: `temper-core::operations` (the command base), `temper-events` (the ledger to absorb),
  `temper-llm` (Domain B's engine)
