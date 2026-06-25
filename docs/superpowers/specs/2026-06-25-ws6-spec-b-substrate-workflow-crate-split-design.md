# WS6 Spec B — temper-substrate / temper-workflow crate split

Status: design (brainstormed 2026-06-25). Sequenced immediately after shim-exit
(`2026-06-25-ws6-shim-exit-native-shape-design.md`). Goal:
`substrate-kernel-to-cognitive-map`.

## Problem

After the WS6 endgame (single-schema `public`, native `ResourceRow` read shape), the
codebase has the *shape* of a substrate kernel + a domain layer but not the *crate
boundaries*. Domain-A opinionation (task/goal/session/decision/research — stage, mode,
effort, doctype schemas, frontmatter, vault layout, decorated-ref synthesis) is scattered
across `temper-core` and entangled with the schema-neutral platform types. The
schema-neutral kernel (events/ledger, edges, clustering, readback, writes) lives in
`temper-next`, a migration-era name.

This spec draws the crate boundaries around the now-native types: extract a
`temper-workflow` crate for the domain-A frame, and rename `temper-next` →
`temper-substrate` so the kernel carries its real name.

The shim-exit spec already locked the relevant precondition: "`ResourceRow` re-homes to
`temper-workflow` in Spec B regardless" (decision #3). This spec executes that.

## Scope decision (locked in brainstorming)

**Mechanical re-home, not semantic neutralization.** Two end-states were considered:

1. **Mechanical re-home (THIS SPEC).** Draw crate boundaries around the types that already
   exist. `ResourceRow` (carrying `stage`/`mode`/`effort`/`seq`) moves to
   `temper-workflow`; the `Backend` trait follows the types and still traffics
   workflow-shaped rows. Mostly file moves + dependency rewiring. No type-shape changes, no
   projection seam.
2. **Neutral substrate API (DEFERRED).** Make the substrate read/write surface
   schema-neutral — it returns a neutral row (`id`, `title`, generic key→value properties)
   and `temper-workflow` projects that into `ResourceRow`. Realizes
   `project_neutral_api_temper_workflow`. Bigger refactor (changes `Backend` return types,
   adds a projection seam).

End-state 2 is the long-term north-star but explicitly **not today's lift**. This spec is
end-state 1. Where end-state 1 leaves a domain-A leak in the kernel, it is documented as
Deferred (see "Known compromises").

## Target architecture (north-star — across many PRs, NOT this spec)

For alignment. The eventual destination this split moves toward:

- **temper-core** — shared types, traits, commitments (the contract layer).
- **temper-substrate** — persistence-layer data modelling, SQL-function wrappers.
- **temper-workflow** — the opinionated goal/task/session/decision/research frame.
- **temper-client** — depends on all three.
- **temper-api** — depends on all three; shares types with `-client` through
  core/substrate/workflow.
- **temper-mcp** — same as api; no expected intersection with `-client`.
- **temper-cli** — depends on `-client`, the others transitively.

**This spec is an interim** that builds the three-crate skeleton but does NOT yet reach the
north-star wiring. Two deliberate interim divergences:

1. **temper-substrate stays independent of temper-core** today (keeps its own `ids`/
   `events`). The north-star "core = shared types/traits" implies substrate eventually leans
   on core for shared contracts — that unification is Option-2-adjacent, deferred.
2. **temper-cli / temper-mcp depend on temper-workflow + temper-core *directly*** after this
   spec (they import `operations`/`commands`/`ResourceRow`). The north-star "cli → client,
   others transitively" is a later re-routing through `temper-client`, out of scope here.

## The crate cut

### temper-substrate (= `temper-next` renamed; internals untouched)

Keeps everything `temper-next` has today, including `keys.rs` and `scenario/`:
events, payloads, edges/affinity, clustering, readback, writes, ids, content, embed, drift,
replay, fingerprint, substrate, text, keys, scenario. Dependency: `temper-ingest` only.

### temper-workflow (NEW; the domain-A cluster lifted out of `temper-core`)

| Source (in temper-core)        | Purpose                                                |
|--------------------------------|--------------------------------------------------------|
| `types/resource.rs`            | `ResourceRow` (+ `ResourceFacets`, `ResourceRelationships`) |
| `types/managed_meta.rs`        | `ManagedMeta` — temper-governed frontmatter fields     |
| `types/graph.rs` (**split**)   | the `DocType`-dependent half — see "Mixed-module splits" |
| `frontmatter/`                 | parse/validate/project; **`DocType`** (6-variant taxonomy moves here) |
| `schema.rs`                    | per-doctype JSON-Schema validation                     |
| `vault.rs`                     | vault layout + `kb://` URI construction                |
| `defaults.rs`                  | doctype-specific managed/open defaults                 |
| `hash.rs` (**split**)          | `compute_managed_hash` only — see "Mixed-module splits" |
| `operations/`                  | `Backend` trait + commands + `refs.rs` (already marked) |

Dependency: **`temper-core` only** (the domain-A modules don't touch `temper-next` —
verified: only doc-comment mentions exist in core).

#### Mixed-module splits (discovered at plan time)

Two temper-core modules are **mixed** — they hold both neutral primitives (consumed by
crates that sit below or beside temper-workflow) and domain-A logic. A wholesale move would
re-create a cycle, so each splits along the neutral/domain-A grain:

- **`hash.rs`.** `compute_body_hash`, `canonicalize_json`, `hash_canonical_json`,
  `compute_open_hash`, `doc_type_from_vault_path` **stay** in `temper-core::hash` (the first
  is consumed by `temper-ingest`, which must not depend on workflow). Only
  `compute_managed_hash` **moves** to `temper-workflow` — it strips `TIER1_SYSTEM_FIELDS`
  (frontmatter::fields) and applies `defaults::apply_managed_defaults`, both domain-A.
- **`graph.rs`.** `EdgeKind` and `Polarity` (the neutral structural edge taxonomy) **stay**
  in temper-core — `types/relationship_requests.rs` and `types/relationship_events.rs` (the
  `/api/relationships` wire types shared with `temper-client`) depend on them. The
  `DocType`-dependent half **moves** to temper-workflow: `EdgeType`, `TargetRef`,
  `ResourceRelationships`, `is_aggregator(DocType)`, `GraphNode`, `GraphEdge`,
  `GraphTraversalRow`, `GraphNeighborRow`, `GraphEdgeRow`, `ResolvedEdge`,
  `EdgeReconciliation`, `SubgraphResponse`. The moved structs import `EdgeKind`/`Polarity`
  from temper-core (workflow → core).
- **`DocType`** is defined inside `frontmatter/document.rs` and **moves to temper-workflow**
  (user decision, principled: the doc-type taxonomy is the opinionated frame; temper-core
  stays doc-type-free). All workflow consumers reference it locally; no core code references
  `DocType` after the move (verified: only `graph.rs`'s moving half and `hash.rs`'s moving
  function used it).

### temper-core (shrinks to neutral leaf)

Retains: `error`, `ids`, `hash` (neutral primitives — see split above), `validation`,
`projection`, `config`, the neutral edge taxonomy (`EdgeKind`/`Polarity`) + the
`/api/relationships` wire types (`relationship_requests`, `relationship_events`), and the
neutral platform types — `auth`, `profile`, `context`, `team`, `device`, `invitation`,
`access`, `audit`, `event`, `search`, `api`, `ownership`, `transfer`, `merge`, `conflict`,
`upload`, `ingest`, and the **invocation/cogmap types** (`invocation`, `invocation_requests` — these
are WS7 cognitive-map, not Domain-A task/goal; they stay).

### Dependency graph (post-split)

```
temper-core (leaf)              temper-substrate (leaf -> temper-ingest)
      ^                                 ^
temper-workflow (-> core)               |
      ^                                 |
      +---- temper-api (-> core + workflow + substrate; impl Backend) ----+
            temper-cli / temper-mcp  -> workflow + core
            temper-client            -> workflow + core
            temper-agents            -> substrate + core
```

Only **temper-api** depends on both workflow and substrate (it is the glue / `Backend`
impl). The graph is shallow and acyclic.

## Why keys.rs and scenario/ stay in substrate

Verified against the code — both moving them would create a cycle:

- `readback/mod.rs` (substrate) calls `keys::is_managed_property_key` (lines 338, 431). The
  substrate read/write path routes on the managed-key set. Moving `keys.rs` to workflow
  ⇒ substrate depends on workflow ⇒ cycle.
- `events.rs:28` (substrate) uses `scenario::model::LensDef`, and `scenario/` carries **17
  `temper_next`-namespace `sqlx::query!` macros** (loader 2, runner 10, bootseed 5). Moving
  `scenario/` to workflow ⇒ cycle AND drags the namespace `.sqlx` burden into a crate that
  is otherwise pure logic.

Consequence: PR2 touches **nothing in substrate** — it is a pure `temper-core` extraction.

## Known compromises (Deferred)

1. **`MANAGED_PROPERTY_KEYS` domain leak.** The list naming `temper-stage`/`temper-mode`/
   `temper-effort`/… stays in substrate (`keys.rs`) because substrate's read/write path
   routes on it. That is domain-A vocabulary living in the neutral kernel. End-state 2
   (neutral substrate) resolves it: workflow declares the key set, substrate routes
   generically over an injected set. Not today.
2. **Postgres namespace `temper_next` not renamed.** The test-only artifact namespace (and
   its migration wrappers, `00_namespace_reset` fixture, `cargo make test-substrate`
   search_path, `prepare-*` task) keeps the name `temper_next`. Renaming it is
   Option-2-adjacent churn for zero functional gain. The crate↔namespace name mismatch
   (`temper-substrate` crate over `temper_next` namespace) becomes a documented wart.

## Sequencing — two PRs, green at each step

The whole-workspace clippy gate makes moves atomic within a PR.

### PR1 — pure rename: `temper-next` -> `temper-substrate`

Behavior-identical. Touch-list:

- `git mv crates/temper-next crates/temper-substrate`; package name + lib name in its
  `Cargo.toml`.
- Every `temper-next` Cargo.toml dependency (temper-api, temper-agents) → `temper-substrate`.
- Every `temper_next::` path across the workspace → `temper_substrate::`.
- `crates/temper-next/.sqlx` → `crates/temper-substrate/.sqlx`.
- `cargo make` tasks `*-next` (`test-next`, `prepare-next`, `flip-load-next`) →
  `*-substrate`; `.config/nextest.toml` group `temper-next-write` → `temper-substrate-write`.
- CI: `code-quality.yml` (`--exclude temper-next` + separate `-p temper-next` clippy/doc
  steps) and `test-rust.yml` (E2E/Coverage `--exclude temper-next`) → `temper-substrate`.
- Docs: CLAUDE.md references to `temper-next` crate, `cargo make test-next`,
  `prepare-next`, the per-crate `.sqlx`. (The *namespace* `temper_next` references stay.)

The Postgres namespace `temper_next` is **not** renamed (see Known compromises #2).

### PR2 — create `temper-workflow`, extract domain-A from `temper-core`

One large atomic move. Substrate is **not touched**.

- New crate `crates/temper-workflow` with features `typescript`/`web-api`/`mcp` + deps
  `ts-rs`/`utoipa`/`schemars` (mirror the gated derives that move with the types) + `sqlx`
  (for the `FromRow` derive on `ResourceRow`) + `temper-core`.
- Move the domain-A modules (table above) from `temper-core` into `temper-workflow`.
- Re-point every consumer's imports:
  - `temper_core::operations` → `temper_workflow::operations` (temper-cli:
    `backend_select`, `actions/ingest`, `commands/{edge,resource}`, `cloud_backend/*`;
    temper-mcp: `tools/{relationships,resources}`; temper-api; temper-client:
    `ingest`, `resources`).
  - `temper_core::types::{resource,managed_meta,graph}` →
    `temper_workflow::types::{…}`; `temper_core::{frontmatter,schema,vault,defaults}` →
    `temper_workflow::{…}`.
- `temper-api/backend/db_backend.rs` imports `ResourceRow` from temper-workflow and keeps
  `key_fate`/`KeyFate` from temper-substrate (the glue spans both).
- `cargo make generate-ts-types`: add temper-workflow's bindings export to the task so the
  `ts(export, export_to=…)` types regenerate. Generated `.ts` is byte-identical →
  temper-ui unaffected. (Verify with a no-op `git diff` on `packages/temper-ui` generated
  types.)
- temper-workflow has **no** `sqlx::query!` macros (only the `FromRow` derive) → no `.sqlx`
  cache, no `prepare-*` task, **no CI exclusion** needed (unlike substrate).
- `temper-core` feature set may shrink only if no remaining neutral type uses a given
  derive; in practice `typescript`/`web-api`/`mcp` stay (profile/team/access/etc. still
  export). Do not remove them speculatively.

## Risks / gates

- **Green-at-each-step is the invariant.** Each PR passes whole-workspace
  `cargo make check` + the full test matrix before merge.
- **PR1 — low risk.** Pure rename. Gate: `cargo make check`, `cargo make test`,
  `cargo make test-substrate` (renamed `test-next`), `cargo make test-e2e`. Grep for any
  stray `temper_next` (crate path) survivor after the sweep — distinguish crate-path hits
  (must change) from namespace/SQL hits (must stay).
- **PR2 — medium risk** (large import churn across api/cli/mcp/client). Mitigations: type
  shapes don't change; it is mechanical re-point; the compiler + workspace clippy catch
  every missed import. Gate: full matrix incl. `cargo make test-e2e` and
  `cargo make test-e2e-embed`. Run `cargo make generate-ts-types` and confirm zero diff in
  temper-ui generated types. machete (via `cargo make check`) confirms the new crate's deps
  are all used.
- **CI exclusion correctness** (`project_temper_next_unconditional_dep_ci_exclusion`):
  PR1 must rename the existing `--exclude temper-next` entries (temper-agents, temper-api
  paths) to `temper-substrate`. PR2 adds no new exclusion (temper-workflow is macro-free).

## Testing

- PR1: rename is behavior-preserving; the existing suites must stay green unchanged.
  Specifically run the artifact tests under their renamed task (`test-substrate`) to prove
  the namespace/search_path wiring survived the crate rename.
- PR2: existing unit + e2e suites cover the moved code unchanged (the types/logic don't
  change, only their crate home). No new tests required; the value is the compiler proving
  the boundary. Confirm `ts-rs` regen is a no-op diff.

## Rejected / Deferred

**Rejected:**
- Splitting `ResourceRow` into a substrate base + workflow extension. Contradicts the
  shim-exit native-shape decision (#3) and re-introduces a projection seam this spec is
  explicitly deferring. The native `ResourceRow` moves whole.
- Making `temper-core` depend on `temper-workflow` (the naive cut). Creates a cycle: the
  domain-A types depend on `temper-core::{error,ids}`. Resolved by keeping core the leaf and
  moving the `Backend` trait + commands up with the types.
- Moving `keys.rs` / `scenario/` to temper-workflow. Cycle + namespace-sqlx burden
  (see "Why keys.rs and scenario/ stay in substrate").

**Deferred:**
- Neutral substrate API (end-state 2) — the real `substrate-kernel-to-cognitive-map`
  payoff; separate spec.
- Postgres namespace `temper_next` → `temper_substrate` rename.
- North-star wiring: substrate→core shared-types unification; cli/mcp routing through
  temper-client.

## References

- `docs/superpowers/specs/2026-06-25-ws6-shim-exit-native-shape-design.md` (precondition;
  decision #3 re-homes ResourceRow here).
- Memories: `project_shared_kernel_two_domains`, `project_neutral_api_temper_workflow`,
  `project_temper_next_unconditional_dep_ci_exclusion`,
  `project_temper_agents_is_contract_not_client`.
- CLAUDE.md: artifact-tests / `temper_next` namespace / per-crate `.sqlx` sections.
