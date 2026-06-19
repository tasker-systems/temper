# Surfacing the cognitive-map substrate (design)

Reachability matrix, the reusable surfacing pattern, and the invocation-envelope first slice.

## Summary

The cognitive-map substrate is **built and tested but unreachable**. Every cognitive-map
capability — `cogmap_genesis`, `facet_set`, `property_set`, `lens_create`,
`region_materialize`, `block_mutate`, the invocation envelope (`invocation_open` /
`invocation_close`), plus the workflow-domain straggler `resource_rehome` — exists as a
SQL function in the `temper_next` artifact, is exercised in temper-next tests, and is
fired from declarative scenarios via `SeedAction` — yet has **zero outward surface** on
the API, MCP, or CLI. The kernel of the `substrate-kernel-to-cognitive-map` goal is a
sealed room: the workflow/KB domain is fully surfaced; the cognitive-map domain has no
door.

This spec delivers three things:
1. A grep-verified **reachability matrix** (capability × surface), split into three tiers.
2. The reusable **surfacing pattern** — the canonical vertical for turning a substrate
   write into API + MCP + CLI surfaces, grounded in the existing `relationship_assert`
   path.
3. A fully-designed **first slice** — the invocation envelope on all three surfaces —
   establishing the pattern in a worked case and making "trace agentic flows" actually
   reachable (the concrete gap the temper-agents scar named).

The cognitive-map capabilities are surfaced **uniformly on all three surfaces** (API +
MCP + CLI), treating the cognitive-map domain as first-class everywhere, exactly like the
workflow/KB domain.

## Motivation

The WS7 runtime investigation established that agentic flows reach temper as an *external
resource* — over the `temper-mcp` URL surface or the `temper` CLI binary (sandbox +
injected secrets) — **not** as a linked library. The companion decision (the temper-agents
neutral-contract spec's *Rejected* section) retracted the idea of a Rust client inside
`temper-agents`: `temper-agents` is a contract crate that *declares* how flows are invoked
and traced; it is the **surfaces** that let a flow actually act on the substrate.

That reframe exposed the real blocker for the WS7 agents (Eve steward,
charter-bootstrapper): not a missing client, but a **missing surface**. The steward needs
`facet_set` to tend structure; the bootstrapper needs `cogmap_genesis`; both need the
invocation envelope to be traceable. All of these are built into the substrate and
unreachable. This spec is the bridge from "substrate kernel" to "cognitive map an agent
can actually operate."

## Preconditions

- **WS6 chunk-5 flip landed.** `NextBackend` (temper-next → `temper_next` SQL functions) is
  the live write path; the legacy `DbBackend` event-sourcing path is retired. This spec
  designs against the NextBackend path only. A sibling session closes the neon-branch
  verification / production promotion before implementation begins.
- **The temper-agents scar holds.** `temper-agents` never links `temper-client`; this spec
  adds surfaces, not a client.

## Section 1 — The reachability matrix

Capabilities are grep-verified against `crates/temper-mcp/src`, `crates/temper-api/src`,
`crates/temper-cli/src` (surface callers) and `crates/temper-next/src` + `tests`
(substrate exercise).

### Tier 1 — available (surfaced on ≥1 of API/MCP/CLI)
resource CRUD, relationship assert/fold/retype/reweight, contexts, search, profile,
events (read-only), sync, access-requests, graph/subgraph.

### Tier 2 — built-but-sealed (exercised in temper-next tests, **zero** outward surface)

| Substrate fn | temper-next refs | API | MCP | CLI |
|---|---|---|---|---|
| `invocation_open` / `invocation_close` | 2 / 2 | ❌ | ❌ | ❌ |
| `facet_set` | 6 | ❌ | ❌ | ❌ |
| `property_set` | 5 | ❌ | ❌ | ❌ |
| `cogmap_genesis` | 13 | ❌ | ❌ | ❌ |
| `lens_create` | 3 | ❌ | ❌ | ❌ |
| `region_materialize` | 1 | ❌ | ❌ | ❌ |
| `block_mutate` | 3 | ❌ | ❌ | ❌ |
| `resource_rehome` | 2 | ❌ | ❌ | ❌ |

### Tier 3 — SQL-only (defined in `schema-artifact/02_functions.sql`, **no Rust binding**)
`cogmap_shape`, `cogmap_telos`, `cogmap_staleness`, `cogmap_regulation`, and the region
metrics `cogmap_region_centrality` / `_content_cohesion` / `_internal_tension` /
`_reference_standing` / `_telos_alignment` (all 0 references in temper-next). These are
**reads/analytics**: surfacing them requires building a Rust binding first, then a
service-direct read path. Deferred to a follow-up spec (see *Out of scope*).

## Section 2 — The surfacing pattern (the reusable vertical)

Grounded in the live `relationship_assert` path. The three surfaces **diverge only in
transport/parsing**; they converge on a single shared command, dispatch through one trait
method, and merge into the NextBackend → temper-next → SQL vertical.

The existing vertical (anchors current as of this writing):

| Layer | File:line (relationship_assert) |
|---|---|
| CLI command | `crates/temper-cli/src/commands/edge.rs:35` |
| CLI → HTTP bridge | `crates/temper-client/src/relationships.rs:30` |
| API handler | `crates/temper-api/src/handlers/edges.rs:59` |
| MCP tool | `crates/temper-mcp/src/tools/relationships.rs:96` |
| Shared command struct | `crates/temper-core/src/operations/commands.rs:121` |
| Backend trait method | `crates/temper-core/src/operations/backend.rs:80` |
| Backend selection | `crates/temper-api/src/backend/selection.rs:42` |
| NextBackend impl | `crates/temper-api/src/backend/next_backend.rs:424` |
| temper-next `writes::` | `crates/temper-next/src/writes.rs:262` |
| `fire` / `fire_with` | `crates/temper-next/src/events.rs:359` / `:366` |
| SQL function | `schema-artifact/02_functions.sql` (`relationship_assert`) |

### To surface a Tier-2 write, add one hop per layer

1. **Command struct** — `temper-core/src/operations/commands.rs`. Carries refs (decorated
   or UUID) + payload fields + the `Surface` origin enum. Mirror `AssertRelationship`.
2. **Trait method** — `temper-core/src/operations/backend.rs`. `async fn <verb>(&self, cmd)
   -> Result<CommandOutput<T>, TemperError>`.
3. **NextBackend impl** — `temper-api/src/backend/next_backend.rs`. Resolve refs → temper_next
   ids; **`check_can_modify_next` before any write** (auth-before-writes); resolve
   emitter/owner profiles; dispatch to `writes::`.
4. **temper-next `writes::` wrapper** — `temper-next/src/writes.rs`. `begin_scoped(pool) →
   fire(&mut tx, SeedAction::X { .. }) → commit`. **The `SeedAction` variant and the SQL
   function already exist for every Tier-2 capability** — this layer is thin glue. (Verify
   per capability: `writes.rs` currently has wrappers only for already-surfaced caps.)
5. **Three surfaces** (thin dispatch): MCP tool in `temper-mcp/src/tools/`; API handler +
   a route in `temper-api/src/routes.rs`; CLI command in `temper-cli/src/commands/` plus a
   `temper-client` method (the CLI reaches the API over HTTP; MCP and API are in-process).

### Tier-3 reads are a different pattern
Per the read-path rule (CLAUDE.md: list/show/get_meta/search stay service-direct, not
through the Backend trait), Tier-3 analytics get a service-direct read surface — **but only
after** a Rust binding exists for the SQL function. That binding work is the reason Tier-3
is deferred.

## Section 3 — Prioritized surfacing backlog

Ordering principle: **unblock the WS7 agents + goal-centrality**.

1. `invocation_open` / `invocation_close` — the trace primitive. **First slice** (Section 4).
2. `facet_set` + `property_set` — "agents tend structure"; the steward's core verbs.
3. `cogmap_genesis` — the charter-bootstrapper's core verb.
4. `lens_create`, `region_materialize`, `block_mutate` — remaining structure-tending verbs.
5. `resource_rehome` — Tier-1 workflow-domain straggler; folds in cheaply on the same pattern.
6. *(follow-up spec)* Tier-3 analytics reads — flagged "needs Rust binding first."

Each of items 2–5 is a repeat application of the Section 2 pattern and gets its own
implementation plan; this spec fully designs only item 1.

## Section 4 — First slice: the invocation envelope (full vertical)

Makes "trace agentic flows" reachable. Reuses the entire lower substrate; adds the trait,
backend, glue, and three surfaces.

### Reuses (already built — do not rebuild)
- `SeedAction::InvocationOpen` (`events.rs:223`) / `InvocationClose` (`events.rs:231`).
- `fire` / `fire_with` dispatch (`events.rs:359` / `:366`).
- SQL `invocation_open` / `invocation_close` (`schema-artifact/02_functions.sql`; mirror in
  `migrations/20260618000001_temper_next_invocation_envelope.sql`).
- Payloads `DelegatedLaunch` (`payloads.rs:517`) and `InvocationClosed` (`payloads.rs:529`),
  re-exported by `temper-agents::envelope`.

### New (the genuine gaps)
- **Commands** (`operations/commands.rs`):
  - `OpenInvocation { trigger_kind: String, originating_cogmap: Ref, parent_cogmap:
    Option<Ref>, scoped_entity: Ref, origin: Surface } → CommandOutput<InvocationId>`
  - `CloseInvocation { invocation_id: InvocationId, disposition: Disposition, outcome:
    serde_json::Value, origin: Surface } → CommandOutput<()>`
- **Trait methods** (`operations/backend.rs`): `open_invocation`, `close_invocation`.
- **NextBackend impls** (`next_backend.rs`): resolve `originating_cogmap` / `scoped_entity`
  refs → temper_next ids; **`check_can_modify` on the scoped cogmap/entity before firing**;
  dispatch to the new `writes::` wrappers.
- **temper-next `writes::` wrappers** (`writes.rs`, currently absent for invocation):
  `open_invocation(pool, params)` and `close_invocation(pool, params)`, each
  `begin_scoped → fire(SeedAction::InvocationOpen/Close) → commit`. `open_invocation`
  returns the minted `InvocationId` (extracted via the existing helper at `events.rs:327`).
- **MCP tools** (`temper-mcp/src/tools/`, new `invocations.rs`): `invocation_open`,
  `invocation_closed`.
- **API** (`routes.rs` + `handlers/`): `POST /api/invocations` (open),
  `POST /api/invocations/{id}/close` (close).
- **CLI** (`commands/`, new `invocation.rs`): `temper invocation open …` /
  `temper invocation close …`, plus a `temper-client` `invocations()` sub-client method
  for the HTTP path.

### Auth
`check_can_modify` on the scoped cogmap/entity precedes the fire in the NextBackend impl —
defense-in-depth even though the MCP/API/CLI caller is already authenticated (matches the
NextBackend `assert_relationship` shape at `next_backend.rs:438`).

## Section 5 — Testing

TDD per layer, red→green, matching the `synthesis_preserves_production_resource_ids`
precedent:
- **`writes::` wrappers** — artifact test under `--features artifact-tests` (the
  `temper-next-write` nextest group): open then close round-trips through the SQL functions
  and projects the envelope rows.
- **NextBackend** — a backend-level test asserting auth-before-write and ref resolution.
- **One e2e** — `tests/e2e` exercises the CLI ↔ API ↔ DB path for the envelope (open →
  close) through the real Axum server.
- **MCP tool** — a tool-level test for `invocation_open` / `invocation_closed`.

## Out of scope

### Rejected (load-bearing — resist scope creep)
- **A Rust client in `temper-agents`.** Already rejected (temper-agents neutral-contract
  spec). Surfaces are how flows act; the contract crate only declares the grain.
- **Re-surfacing the legacy DbBackend event-sourcing write path.** Dead post-flip; this
  spec targets NextBackend only.

### Deferred (in scope elsewhere / later)
- **Tier-3 analytics read-path** (`cogmap_shape` / `telos` / `staleness` / `regulation` /
  region metrics). Needs a Rust binding before any surface; separate follow-up spec,
  service-direct read pattern.
- **Backlog items 2–5** (`facet_set`, `property_set`, `cogmap_genesis`, `lens_create`,
  `region_materialize`, `block_mutate`, `resource_rehome`). Each is a repeat application of
  the Section 2 pattern with its own implementation plan.
- **The agents themselves** (Eve steward, charter-bootstrapper) — downstream WS7 threads
  that consume these surfaces once they exist.

## Connections

- Realizes the surface half of the `substrate-kernel-to-cognitive-map` goal.
- Builds on the temper-agents neutral-contract spec
  (`docs/superpowers/specs/2026-06-18-temper-agents-neutral-contract-crate-design.md`) and
  its *Rejected* scar (surfaces, not a client).
- Grounded in the WS7 runtime investigation
  (`docs/research/2026-06-18-vercel-eve-and-claude-managed-agents-investigation.md`).
- Depends on the WS6 chunk-5 flip (NextBackend live) — the in-progress flip task + sibling
  neon-branch verification.
- The invocation-envelope first slice makes the "trace" corollary of the temper-agents scar
  real.
