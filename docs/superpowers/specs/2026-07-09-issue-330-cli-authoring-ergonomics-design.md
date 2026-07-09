# Issue #330 — CLI authoring ergonomics

**Date:** 2026-07-09
**Issue:** https://github.com/tasker-systems/temper/issues/330
**Goal:** Temper Cloud: CLI & API Usability (`019d5038-c7ab-7dc3-b45c-f208ef5a3a1f`)
**Status:** design approved, pending implementation plan

## Context

A batch authoring pass into a cognitive map — open invocation, create many nodes, assert
edges, set facets, materialize, close — is a common scriptable workflow. Issue #330 reports
five ergonomic rough edges that together make such a pass fiddlier than it should be. None
is a correctness bug; each has a workaround.

This design covers all five, grouped into three PRs.

## Grounding: what the issue got wrong

The issue is a hypothesis written from an authoring session. Verified against the code, two
of its five claims are factually wrong and a third has the wrong root cause. These
corrections materially reduce and reshape the work.

**`facet_set` is not missing from the CLI.** It ships as `temper resource facet <ref>
--values '<json>' [--weight]` (`temper-cli/src/cli.rs:574` → `commands/facet.rs:12`). The
issue's premise — that a script must switch to MCP mid-run to set facets — is false. The
author could not find the verb. Only `cogmap materialize` is genuinely absent, and every
layer beneath it already exists: `temper-client/src/cognitive_maps.rs:104`, route
`temper-api/src/routes.rs:185`, handler `handlers/cognitive_maps.rs:199`, trait method
`temper-workflow/src/operations/backend.rs:171`, impl
`temper-services/src/backend/db_backend.rs:2165`.

**Item 4's `managed_meta: {}` is item 3's write gap, observed.** `readback::meta`
(`temper-substrate/src/readback/mod.rs:234-278`) faithfully returns the managed property
rows that exist. The CLI create path writes only `mode` and `effort`
(`commands/resource.rs:319-322`), so there is nothing else to return. Fixing the stamp
(item 3) removes most of item 4's reported symptom. Separately, the empty `managed_hash` /
`open_hash` are a **deliberate** projection choice, documented at
`temper-services/src/backend/substrate_read.rs:249` as "§7-dissolved (emitted empty; §9
non-invariants)" — not an oversight, and not something we can honestly populate.

The *real* item-4 defect is narrower and different from what was filed: full `resource show`
serializes `ResourceRow`, which contains **neither** meta tier — only flat projections
(`stage`, `seq`, `mode`, `effort`, `body_hash`). It omits `managed_meta` as well as
`open_meta`.

**Item 5's disposition claim has no bug behind it.** `kb_invocations` has no `disposition`
column (`migrations/20260624000001_canonical_schema.sql:512-526`). The close function writes
the disposition *into* `status` (`migrations/20260624000002_canonical_functions.sql:1285-1290`),
and `InvocationView` (`temper-core/src/types/invocation.rs:64-87`) has no `disposition`
field to populate. Reading back `disposition: None` is the stack behaving as built. This is
a contract question, not a round-trip failure.

Confirmed as written: item 1 (`search_cmd.rs:41` renders a bare array; `resource.rs:1030`
prints a second document for `--edges` and `:1060` a third for `--provenance`; no envelope
type exists anywhere in the CLI), item 3 (no `--managed-meta` flag exists;
`--model`/`--invocation`/`--confidence` land on the act event via `ActArgs::into_act_input`
at `cli.rs:77-96`, never on `managed_meta`), and item 5's `--sources` claim (it writes block
provenance only; `derived_from` is an unrelated *open_meta key* on `update`, not an edge).

### Root cause: the skill content lied

The issue was filed by a competent agent with the skill content loaded. It is a user-study
result, and grounding shows the agent behaved correctly on every count — three of the five
items are direct consequences of **false statements in
`crates/temper-cli/skill-content/cognitive-maps.md`**:

| Line | Claim | Reality | Item it caused |
|------|-------|---------|----------------|
| `:98` | "**`facet_set` is agent-surface only** (the `facet_set` MCP tool)" | It is `temper resource facet` | 2 |
| `:100` | "**Materialize** … is likewise agent-surface" | True today; PR (c) makes it false | 2 |
| `:126` | "cites both in `--sources` (and one `derived_from` edge per source)" | `--sources` writes block provenance, never an edge | 5 |

The agent did not fail to discover `resource facet`. It was told the verb did not exist, and
reached for MCP as instructed. Likewise it expected `--sources` to yield edges because line
126's parenthetical says so.

This makes agent-facing description a **first-class deliverable with its own correctness
bar**, not a doc footnote. It also means the fix must cover every surface an agent reads
from: the installable CLI skill (`skill-content/`), the MCP tool descriptions, and the
deployed steward agent (`packages/agent-workflows/steward/`).

Two incidental findings that the design depends on:

- `ManagedMeta` lives in **temper-workflow** (`src/types/managed_meta.rs:30-70`), not
  temper-core. It is a closed `deny_unknown_fields` struct of exactly ten fields, drift-guarded
  against `temper_substrate::keys::MANAGED_PROPERTY_KEYS` by
  `temper-services/tests/managed_meta_property_drift_test.rs`.
- `relationship_assert` is idempotent: it upserts on the active-edge invariant
  (`canonical_functions.sql:813-816`), updating weight and returning the existing edge id
  rather than duplicating.

## Decisions

| # | Decision | Rejected alternative |
|---|----------|----------------------|
| 1 | Minimal flat JSON fixes; **no universal envelope** | `Envelope<T>` over ~40 print sites — large mechanical diff, adds a wrapper agents must unwrap on every call |
| 2 | Only add `cogmap materialize`; leave `resource facet` alone, fix its discoverability | Renaming to `temper facet set` — churns a working verb to solve a docs problem |
| 3 | Server-side fill-missing provenance stamp in `DbBackend` | `--managed-meta` flag — pure boilerplate on every create; CLI-only stamp leaves MCP unenforced |
| 4 | New `ResourceDetail` type for `show`; delete the dead hash fields | `Option` fields on `ResourceRow` — the type would lie in list context |
| 5 | Derive `disposition` on the view; opt-in `--sources-as-edges` | A `disposition` column — two fields that must never disagree, with no reader needing it |

## PR (a) — JSON output contract

Scope: `temper-cli`, plus two ack structs in `temper-core`.

**`search` emits an object.** `search_cmd.rs:41` renders a typed
`SearchResultsResponse { results: … }` rather than a bare `Vec`. A real struct, not a
`json!()` wrapper, per the typed-structs rule.

The struct lives in `temper-cli` (not `temper-core`): the rows reaching this print site have
already passed through `inject_ref`, so they are `serde_json::Value`, not
`UnifiedSearchResultRow`. The wrapper is therefore
`struct SearchResultsResponse { results: Vec<serde_json::Value> }` — typed at the envelope,
where the contract lives, with the rows staying `Value` because ref-injection has already
made them so. Lifting the wrapper into `temper-core` would require pushing `ref` injection
server-side, which is a larger change and not in scope.

**`resource show` emits exactly one document.** Today `show()` calls `println!` up to three
times: the resource at `resource.rs:933`, `--edges` at `:1030`, `--provenance` at `:1060`.
Extract a pure `build_show_document(row, edges, provenance) -> serde_json::Value` that folds
all three into one object under `edges` and `provenance` keys. `show_edges()` and the
provenance block stop printing and start returning data; `show()` prints once.

This is the load-bearing structural choice: multiple documents become *impossible* rather
than test-detectable, and the guard collapses from an e2e that shells out to the real binary
into a DB-free unit test on the builder.

**Create-style acks carry a plain `id`.** `InvocationAck { id, invocation_id }`,
`FacetAck { id, property_id }` — the specific alias stays, `id` is added alongside, so a
generic "create X, capture its id" helper reads `id` from any of them. `resource create`
already emits `id` via the flattened `ResourceRow`.

The duplication is deliberate. The alternative — a CLI-side `inject_id(&mut Value, "invocation_id")`
mirroring the existing `inject_ref` — manipulates untyped `Value` and fixes only the CLI.
Carrying both fields on the typed wire struct fixes the MCP surface at the same source.

**Test note:** `actions/search.rs:358` has a test named
`render_search_results_json_is_passthrough_array` asserting `out.starts_with('[')`. It
encodes the contract this PR deliberately breaks. It gets **rewritten** to assert an object
with a `results` key, not deleted.

## PR (b) — the managed tier becomes real

Scope: `temper-services`, `temper-workflow`, `temper-api`, `temper-client`, `temper-cli`,
plus the stewardship docs. The widest blast radius of the three.

### The stamp

`DbBackend::create_resource` already receives the `ActContext`. It fills `managed_meta` where
the caller left holes:

- `llm_model` ← `act.model`
- `llm_run` ← `act.invocation_id`
- `provenance` ← `"llm-discovered"` when a model is present, `"user-created"` when it is not

Fill-missing, never overwrite: an explicit caller value always wins, so MCP agents that
already pass the trio are untouched. This is the same receive-side symmetric-defense pattern
as `ensure_managed_identity_keys`.

Stamping `user-created` on non-LLM creates goes slightly beyond the issue's ask. It is
included deliberately: `ManagedMeta.provenance` is documented as "LLM-discovered or
user-created", but `user-created` appears nowhere in the codebase — the field has a
documented second value nothing produces. Stamping both makes the tier meaningful and gives
a real LLM-versus-human differentiator on every resource.

`provenance` stays `Option<String>` with the two values as named constants. Promoting it to
a closed enum — which the "no stringly-typed matches over bounded sets" rule would normally
demand — is **rejected**: `ManagedMeta` is a `deny_unknown_fields` deserialization target for
rows already in the database, and a closed enum would turn any historical value outside the
pair into a hard readback failure.

Applies to **create only**. Update-path stamping is out of scope (YAGNI).

### The read

New `ResourceDetail` in `temper-workflow/src/types/resource.rs`:

```rust
pub struct ResourceDetail {
    #[serde(flatten)]
    pub row: ResourceRow,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_meta: Option<ManagedMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_meta: Option<Value>,
}
```

The two meta fields carry serde attributes **identical** to `ResourceMetaResponse`'s, so the
subset relation holds field-for-field.

`GET /api/resources/{id}` returns `ResourceDetail`. `GET /api/resources` keeps the lean
`ResourceRow`, so a 200-row list pays nothing.

Composition happens in a single `temper-services` read function that calls the two existing
readbacks (`resource row` + `readback::meta`) and assembles them, rather than a new joined
query. Two round-trips inside one service call — a deliberate trade: **no new `sqlx::query!`
macro means no sqlx cache regeneration ritual.** At one resource per call this is not an N+1.

### The rename and the deletion

`ResourceMetaResponse.resource_id` → `id`. This makes `{id, managed_meta, open_meta}` a
*literal* strict subset of `ResourceDetail`; as currently typed (`resource_id` vs `ResourceRow.id`)
the anchor keys differ and the subset relation is unachievable. It also lets the `--fields`
anchor-key logic stop special-casing two names.

`managed_hash` and `open_hash` are **deleted** from `ResourceMetaResponse`. They have been
empty strings since the §7 dissolve and no consumer can use them. The real `body_hash` on
`ResourceRow` stays.

### Docs

`packages/agent-workflows/steward/agent/skills/map-stewardship.md:105-112`,
`crates/temper-cli/skill-content/cognitive-maps.md:143-158`, and
`packages/agent-workflows/steward/agent/instructions.md:51-52` all instruct agents to pass
the provenance trio by hand on every `create_resource`. Once the stamp lands, that
instruction is obsolete. These edits ship **in this PR**, alongside the behavior they
describe.

### Testing

The acceptance criterion is a differential test, not a hand-written expectation:

1. Create a resource with `--model` and `--invocation`.
2. Assert full `show` contains a populated `managed_meta` carrying the trio.
3. Assert every key of the `--meta-only` object is present, with an equal value, in the full
   `show` object.

Neither path encodes an author's belief about the shape; the two paths check each other.

Required adjacent tests, because this is a `DbBackend` command behavior change:

- `crates/temper-api/tests` `genesis_cogmap_test`, run directly against that target
- `managed_meta_property_drift_test` in `temper-services` — the auto-stamp must write only
  keys inside the guarded ten
- `cargo make generate-ts-types` (wire types changed)
- `cargo make check` under `SQLX_OFFLINE=true` — the honest local probe of the committed cache

## PR (c) — authoring ergonomics

Scope: `temper-cli`, plus `InvocationView` in `temper-core`.

**`temper cogmap materialize <ref> [--threshold N]`.** A `CogmapCmd::Materialize` variant
(`cli.rs:928`), a dispatch arm (`main.rs:775`), and a body in `commands/cogmap.rs` calling the
existing `client.cognitive_maps().materialize(id, threshold)`. Nothing below the CLI moves.
It emits the existing `MaterializeAck` — an acknowledgment of counts, not a create, so PR (a)'s
`id` convention does not apply.

**`--sources-as-edges` on `resource create`.** After the create succeeds, the CLI asserts one
`derived_from` edge from the new resource to each **resource-valued** source, skipping remote
URLs (which have no target). `EdgeType::DerivedFrom` already exists
(`temper-workflow/src/types/graph.rs:37`, mapping to `(EdgeKind::LeadsTo, Polarity::Inverse,
"derived_from")`).

This is **not atomic.** The new resource's id is the edge source, so the asserts necessarily
follow the create. If the third of five edges fails, the resource exists with two edges. That
is accepted rather than pushed server-side, for two reasons: `relationship_assert` is
idempotent, so re-running the same command converges rather than duplicating; and a
server-side flag would make `CreateResource` mutate graph shape as a side effect of a create,
exactly the hidden coupling the command layer exists to prevent.

The create response gains an `edges_asserted` array so the outcome is visible. A partial
failure errors with the created resource's ref named, so the author can resume.

**Derived `disposition` on `invocation show`.** `InvocationView` gains
`disposition: Option<Disposition>`, computed where `InvocationShowRow`
(`temper-substrate/src/readback/mod.rs:844-856`) is mapped to the view: `open` yields `None`,
any terminal status parses into the enum via a new `TryFrom<&str>` on `Disposition`.

An unparseable status **propagates as an error** rather than degrading to `None`. The database
`CHECK` constrains the column to four values, so an unknown one means an invariant broke and
should be loud (error-escalation rule).

**The `cancelled` papercut.** No alias is added. Two words mapping to one disposition muddies
a vocabulary that the schema `CHECK`, the SQL function, and the core enum all agree on. Clap's
`ValueEnum` already prints `[possible values: completed, failed, abandoned]`; what it does not
say is which one an author reaching for "cancelled" should pick. The fix is a `long_help` on
the flag spelling out that `abandoned` covers cancelled and aborted runs. Smallest honest
change; recorded as a documentation gap rather than bespoke suggestion machinery.

### Agent-facing description truth

The false claims that produced this issue are corrected across **every surface an agent reads
from**. This is the deliverable, not an afterthought.

**The installable CLI skill** (`crates/temper-cli/skill-content/`, `include_str!`'d into the
binary at `commands/skill.rs:16-26`):

- `cognitive-maps.md:96-101` — the authored-4 paragraph. `facet_set` is **`temper resource
  facet`**, not agent-surface-only. `materialize` becomes `temper cogmap materialize`.
- `cognitive-maps.md:126` — the parenthetical must stop implying `--sources` creates edges.
  Post-PR-(c) the honest sentence names `--sources-as-edges` as the flag that does.
- `cognitive-maps.md:229` — the worked example's "then `cogmap_materialize` (MCP)" comment
  becomes the CLI verb, so the example is a runnable single-surface script. That was the
  issue's actual ask.
- `reference.md` — currently mentions neither `facet` nor `materialize`. Both get listed.

**The MCP tool descriptions** (`crates/temper-mcp/src/tools/`, rmcp doc comments). Only one
description anywhere names a CLI equivalent (`cognitive_maps.rs:170`). `facet_set` and
`cogmap_materialize` name theirs, so an agent on the MCP surface learns the CLI verb exists.

**The deployed steward** (`packages/agent-workflows/steward/agent/`): inherits the corrected
authored-4 statement in `skills/map-stewardship.md`. Note the division of labour — the
*manual-trio* removal from `instructions.md:51-52` and `map-stewardship.md:105-112` ships in
**PR (b)**, alongside the auto-stamp that obsoletes it. PR (c) touches only the
verb-and-surface claims.

### The guard

Prose cannot be type-checked, but the *referents* can. A `temper-cli` unit test introspects
the clap `Command` tree and asserts that every CLI verb the skill content names actually
resolves — `resource create`, `resource facet`, `edge assert`, `edge fold`, `cogmap
materialize`, `invocation open`, `invocation close`. If someone renames or removes a verb, the
test fails and points at the doc that now lies.

This does not verify prose claims (nothing cheap does). It pins the existence claims the prose
depends on, which is exactly the class of error that produced this issue.

### Testing

`temper-cli` unit tests for the new clap wiring, the source-partitioning logic (resource refs
versus remote URLs), and the verb-existence guard above; one e2e proving `--sources-as-edges`
produces exactly the resource-valued edges; `temper-api`'s invocation integration target for
the `disposition` round-trip.

## PR sequencing

Three PRs, merged **sequentially onto `main`, not stacked** (squash has stranded stacked
branches here before). Order: (a) → (b) → (c). PR (b) changes `ResourceMetaResponse`, which
the `--meta-only` print site consumes, so merging it between the two CLI-heavy PRs keeps the
conflict surface small.

Grouping is by story, not by issue item: item 4's acceptance criterion is *unverifiable* until
item 3 merges, so they are one PR. Splitting them would hand a reviewer a `show` that
correctly displays an empty managed tier.

One known overlap: both (b) and (c) edit
`crates/temper-cli/skill-content/cognitive-maps.md` — (b) removes the now-obsolete
manual-trio instruction around lines 143-158, (c) adds the missing verb documentation. They
touch different sections, but (c) rebases on (b) to keep the merge clean.

## Test strategy

Local runs stay targeted; CI carries regression coverage.

| PR | Local | Cost |
|----|-------|------|
| (a) | `cargo nextest run -p temper-cli`, `cargo make check` | DB-free |
| (b) | `cargo make check`, `generate-ts-types`, `genesis_cogmap_test`, `managed_meta_property_drift_test`, one filtered e2e | heaviest |
| (c) | `cargo nextest run -p temper-cli`, `temper-api` invocation target, one filtered e2e | light |

`cargo make test-all` runs on none of them. Before any e2e, rebuild the binary
(`cargo build -p temper-cli --bin temper`) — `test-e2e` does not rebuild it. Do not run two
e2e suites concurrently.

## Out of scope

- A universal JSON envelope (`Envelope<T>`) across all print sites.
- A `--managed-meta` flag on create/update.
- A `disposition` column on `kb_invocations`.
- Renaming `resource facet` to `facet set`.
- Provenance stamping on the **update** path.
- Populating `managed_hash` / `open_hash` — they are §7-dissolved by design.

## Acceptance criteria

Restated from the issue, corrected for what grounding revealed:

- [ ] `--format json` yields exactly one JSON document per invocation; `search` returns an
      object; `--edges` and `--provenance` fold into the resource object; create-style acks
      carry a consistent `id`.
- [ ] `temper cogmap materialize` exists. `temper resource facet` is documented where authors
      will find it. *(No `temper facet set` — it already exists under another name.)*
- [ ] The provenance trio is stamped server-side on create, for every surface, fill-missing.
      *(No `--managed-meta` flag.)*
- [ ] Full `resource show` includes both meta tiers; `--meta-only` is a literal strict subset.
      *(No hashes — they are dissolved, and the dead fields are removed.)*
- [ ] `--sources-as-edges` asserts `derived_from` edges for resource-valued sources.
      `invocation show` reports a derived `disposition`. `--disposition` help names
      `abandoned` for cancelled runs.
- [ ] Every agent-facing surface tells the truth: the skill content no longer calls
      `facet_set` agent-surface-only or implies `--sources` creates edges; MCP descriptions for
      `facet_set` / `cogmap_materialize` name their CLI equivalents; the steward drops the
      obsolete manual-trio instruction. A clap-introspection test pins every CLI verb the
      skill content names.
