# T1 — facet_set + charter-read + DocType parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring the steward's act vocabulary to MCP+CLI+API parity: (A) extend the recognized cogmap-resource label set with an open tail, (B) add `facet_set` as a full vertical slice (it exists only at the substrate layer today), and (C) add a telos/charter-block **read** tool so the steward can orient on its telos.

**Architecture:** Three independent sequences. **A** extends the `temper-workflow` `DocType` enum with 8 recognized variants and loosens 4 parse/validate gates so genuinely-unknown labels pass through as strings (no `Unknown(String)` variant — `DocType` stays `Copy`). **B** mirrors the existing relationship/edge write stack (`assert_relationship`/`fold_relationship`) at every layer down to the existing `writes::set_facet_in_tx` + `facet_set` SQL function. **C** adds a services read wrapping `resource_blocks(cogmap_telos($map), …)` plus an MCP tool.

**Tech Stack:** Rust, sqlx/Postgres, rmcp (MCP), Axum (API), clap (CLI), schemars/utoipa/ts-rs derives, cargo-make + cargo-nextest.

## Global Constraints

- Quality gate before every commit: `cargo make check` (fmt + clippy `-D warnings` + docs + machete + TS). The pre-commit hook runs it too.
- Build/clippy with `--all-features`.
- Persistence layering (CLAUDE.md): SQL/writes live in `temper-substrate`/`temper-services`; **surfaces dispatch one operations command through `DbBackend`** — never inline `sqlx::query!` in a handler/tool/CLI action. Reads may be service-direct.
- Typed structs over inline JSON; params structs over >5 args; **auth before writes** (`check_can_modify_next` precedes any mutation).
- After SQL changes: regenerate caches — workspace `cargo sqlx prepare --workspace -- --all-features`, then per-crate `cargo make prepare-services` / `prepare-api` / `prepare-e2e` as touched. (This task adds **no** new migration; only Sequence B may add a `sqlx::query!` to surface the new property id — regen services if so.)
- After temper-cli changes the e2e suite spawns: `cargo build -p temper-cli --bin temper`.
- MCP enum params must be `schemars(inline)` (see `EdgeKind`) — `$ref` enums reach Anthropic tool-use as null.
- **Note on the label registry home:** D3 of the spec extends the *existing* `DocType` in `temper-workflow` (single source of truth for the `doc_type` property's recognized values). The brainstem instinct "enums in temper-core" is honored in spirit — `DocType` may relocate as the Domain-A/B split matures; do not fork a parallel cogmap enum now.

---

# Sequence A — DocType extension + open tail

Today `DocType::from_str` **errors** on unknown ("unknown doctypes fail at parse" — explicit current contract; comments at `document.rs:6-7,92-93`). The wire (`--type` is `String` at `cli.rs:290-292`), storage (`kb_properties key='doc_type'`), and read (`substrate_read.rs` string projection) are already string-tolerant. Only 4 gates enforce the closed set.

## Task A1: Add the 8 recognized variants

**Files:**
- Modify: `crates/temper-workflow/src/frontmatter/document.rs` (enum `:14-21`, `ALL` `:32-39`, `as_str` `:42-51`, `schema_json` `:75-84`, comments `:6-7,92-93`, tests `:426-457`)
- Create (×8): `crates/temper-workflow/schemas/{fact,memory,question,theme,concern,principle,commitment,domain}.schema.json`
- Modify exhaustive match sites the compiler will flag: `defaults.rs:27-37`, `schema.rs:233-242`, `operations/actions.rs:355-381`
- Modify name-list: `schema.rs:292-293` (`DOC_TYPE_NAMES`)

**Interfaces:**
- Produces: `DocType::{Fact,Memory,Question,Theme,Concern,Principle,Commitment,Domain}` recognized everywhere `concept`/`decision` are.

- [ ] **Step 1: Update the round-trip test to expect the new set (failing)**

In `document.rs` tests, extend `doc_type_round_trip_all_six` (rename → `_all`) to iterate the full list and add the 8 names:

```rust
#[test]
fn doc_type_round_trip_all() {
    for name in ["task","goal","session","research","decision","concept",
                 "fact","memory","question","theme","concern","principle","commitment","domain"] {
        let dt = DocType::from_str(name).expect("known");
        assert_eq!(dt.as_str(), name);
        let _ = dt.schema_json(); // include_str! must resolve
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo nextest run -p temper-workflow doc_type_round_trip_all -v`
Expected: FAIL — `from_str("fact")` Errs / `schema_json` has no arm (compile error after you add the enum, missing schema files).

- [ ] **Step 3: Extend the enum and all exhaustive impls**

`document.rs` — add variants:

```rust
pub enum DocType {
    Task, Goal, Session, Research, Decision, Concept,
    // Cognitive-map node labels (spec D3). Open tail handled at the validation gates (Task A2).
    Fact, Memory, Question, Theme, Concern, Principle, Commitment, Domain,
}
```

Add the 8 to `ALL` (`:32-39`), to `as_str` (`:43-50`, snake_case names), to `from_str` (`:55-65`, map each name → variant — keep the `other => Err(...)` arm for now; A2 changes the gate behavior), and to `schema_json` (`:76-83`, one `Self::Fact => include_str!("../../schemas/fact.schema.json")` arm each). Update the closed-set comments at `:6-7,92-93` to note the cogmap labels + the forthcoming open tail.

- [ ] **Step 4: Create the 8 schema files**

For each label, create `crates/temper-workflow/schemas/<label>.schema.json` mirroring `concept.schema.json`, changing only the `$id` and the `temper-type` const:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "$id": "https://temperkb.io/schemas/fact.schema.json",
  "allOf": [ { "$ref": "base.schema.json" } ],
  "properties": {
    "temper-type": { "const": "fact" },
    "temper-slug": { "type": "string", "pattern": "^[a-z0-9][a-z0-9-]*$", "description": "URL-safe identifier" }
  },
  "required": ["temper-slug"],
  "additionalProperties": true
}
```

- [ ] **Step 5: Fix the other exhaustive matches**

- `defaults.rs:27-37` (`apply_managed_defaults`): add the 8 to the no-op arm → `Session | Research | Decision | Concept | Fact | Memory | Question | Theme | Concern | Principle | Commitment | Domain => {}`.
- `schema.rs:233-242` (`display_fields` extras): add the 8 to the `=> &[]` arm.
- `operations/actions.rs:355-381` (`validate_create` per-doctype match): add the 8 to the no-extra-rules arm.
- `schema.rs:292-293` (`DOC_TYPE_NAMES`): add the 8 (keep alphabetical if the slice is sorted).
- `types/graph.rs:264-269` (`is_aggregator`, `matches!`): **decision** — cogmap labels are NOT aggregators (leave them out; they're leaf concepts). No code change needed unless you want `theme` to aggregate; for MVP, no.

- [ ] **Step 6: Run the test + crate tests**

Run: `cargo nextest run -p temper-workflow`
Expected: PASS. Also delete/skip `doc_type_rejects_unknown` (`:434-438`) and `try_from_str_fails_on_unknown_temper_type` (`:454-457`) **only if** A2 lands in the same PR (they still pass after A1 alone — unknown still Errs until A2). Keep them green here; A2 inverts them.

- [ ] **Step 7: `cargo make check` + commit**

```bash
cargo make check
git add crates/temper-workflow/src/frontmatter/document.rs crates/temper-workflow/schemas/ \
        crates/temper-workflow/src/defaults.rs crates/temper-workflow/src/schema.rs \
        crates/temper-workflow/src/operations/actions.rs
git commit -m "feat(workflow): recognize 8 cognitive-map node labels in DocType"
```

## Task A2: Open tail — unknown labels pass through

**Files:**
- Modify: `crates/temper-workflow/src/operations/actions.rs` (`validate_doctype` `:239-248`, `validate_create` `:352`, tests `:600-612`)
- Modify: `crates/temper-workflow/src/schema.rs` (`load_schema`/`validate_frontmatter` `:62-96`)
- Modify: `crates/temper-cli/src/commands/resource.rs:202,251`, `crates/temper-cli/src/actions/ingest.rs:200` (fail-fast `from_str?` calls)

**Interfaces:**
- Produces: an unrecognized non-empty `doc_type` string is accepted and stored verbatim (no frontmatter-schema enforcement), rather than rejected.

- [ ] **Step 1: Write the failing test (unknown passes the gates)**

In `actions.rs` tests, replace `validate_doctype_rejects_memory_not_a_real_doctype` (`:607-612`) with:

```rust
#[test]
fn validate_doctype_accepts_unknown_label_passthrough() {
    // memory is now recognized (A1); use a genuinely-unknown label for the tail.
    assert!(validate_doctype("anecdote").is_ok(), "unknown labels pass through (open tail)");
    assert!(validate_doctype("").is_err(), "empty is still rejected");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo nextest run -p temper-workflow validate_doctype_accepts_unknown -v`
Expected: FAIL — `validate_doctype("anecdote")` is currently `Err` (not in `DOC_TYPE_NAMES`).

- [ ] **Step 3: Loosen the gates**

- `validate_doctype` (`actions.rs:239-248`): replace the `DOC_TYPE_NAMES` membership test with a non-empty check:

```rust
pub fn validate_doctype(doc_type: &str) -> Result<(), TemperError> {
    if doc_type.trim().is_empty() {
        return Err(TemperError::Config("doc_type must be non-empty".into()));
    }
    Ok(()) // recognized OR open-tail: the label is a free string at the kernel
}
```

- `validate_create` (`actions.rs:352`): make the per-doctype match conditional on recognition:

```rust
if let Ok(dt) = DocType::from_str(&cmd.doctype) {
    match dt { /* existing per-doctype arms */ }
} // unknown label: no per-doctype rules
```

- `schema.rs` `load_schema`/`validate_frontmatter` (`:62-96`): skip schema validation for unrecognized labels:

```rust
let Ok(dt) = DocType::from_str(doc_type) else {
    return Ok(()); // open tail: unrecognized doctype carries no frontmatter schema to enforce
};
let schema_str = dt.schema_json();
// ... existing validation against schema_str ...
```

- CLI fail-fast calls — `commands/resource.rs:202` (`let _ = DocType::from_str(doc_type)?;`) → delete (the server gate + `validate_doctype` now govern); `:251` (slug derivation) → fall back to the existing `_ =>` catch-all in `derive_create_slug` when `from_str` Errs; `actions/ingest.rs:200` → same tolerance (don't `?`-reject; use the enum only when `Ok`).

- [ ] **Step 4: Run tests across the wire crates**

Run: `cargo nextest run -p temper-workflow -p temper-cli`
Expected: PASS. Confirm no remaining test asserts unknown-doctype rejection.

- [ ] **Step 5: `cargo make check` + commit**

```bash
cargo make check
git add crates/temper-workflow/src/operations/actions.rs crates/temper-workflow/src/schema.rs \
        crates/temper-cli/src/commands/resource.rs crates/temper-cli/src/actions/ingest.rs
git commit -m "feat(workflow): open-tail doc_type — unrecognized labels pass through as strings"
```

---

# Sequence B — facet_set vertical slice

`facet_set` exists only at the substrate (`facet_set` SQL fn `canonical_functions.sql:888-905`; `writes::set_facet_in_tx` `writes.rs:467-487`; `SeedAction::FacetSet` `events.rs:546-570`). There is **no** operations command, Backend method, DbBackend impl, core wire type, API route, client method, CLI command, or MCP tool. Build all of them, mirroring the relationship/edge stack.

## Task B1: operations command + Backend trait method

**Files:**
- Modify: `crates/temper-workflow/src/operations/commands.rs` (new `SetFacet`, mirror `FoldRelationship` `:189-200`)
- Modify: `crates/temper-workflow/src/operations/mod.rs` (`:25-27` re-export)
- Modify: `crates/temper-workflow/src/operations/backend.rs` (`:48-136` trait; add `set_facet`)

**Interfaces:**
- Produces: `pub struct SetFacet { pub resource: ResourceId, pub values: serde_json::Value, pub weight: f64, pub act: ActContext, pub origin: Surface }`; `async fn set_facet(&self, cmd: SetFacet) -> Result<CommandOutput<PropertyId>, TemperError>` on `Backend`.

- [ ] **Step 1** — Add `SetFacet` to `commands.rs` mirroring `FoldRelationship` (same `act: ActContext` + `origin: Surface` tail); re-export in `mod.rs`. `PropertyId` is `temper_core::types::ids::PropertyId`.
- [ ] **Step 2** — Add the trait method signature to `backend.rs` next to `fold_relationship` (`:97-100`).
- [ ] **Step 3** — `cargo build -p temper-workflow --all-features` (no impl yet — this is the trait; the impl is B2; the crate compiles because the impl lives in temper-services).
- [ ] **Step 4: Commit**

```bash
git add crates/temper-workflow/src/operations/
git commit -m "feat(workflow): SetFacet operations command + Backend::set_facet"
```

## Task B2: DbBackend impl + substrate wrapper surfacing the new id

**Files:**
- Modify: `crates/temper-substrate/src/writes.rs` (`set_facet_in_tx` `:467-487` / `set_facet_with` `:451` — surface the returned `PropertyId` from `Fired::Facet`)
- Modify: `crates/temper-services/src/backend/db_backend.rs` (`impl Backend` `:771+`; mirror `fold_relationship` `:1232-1262` + `assert_relationship` home/emitter resolution `:1107-1164`)

**Interfaces:**
- Consumes: B1's `SetFacet`. Produces: `DbBackend::set_facet` → `CommandOutput<PropertyId>`.

- [ ] **Step 1: Write the failing integration test**

In `crates/temper-services` test target (mirror the existing backend write tests; `#[sqlx::test]`), assert a facet set on a cogmap-homed resource returns a `PropertyId` and that an unauthorized profile is rejected before any write:

```rust
#[sqlx::test(migrator = "temper_substrate::MIGRATOR")]
async fn set_facet_returns_property_id_and_gates_auth(pool: PgPool) {
    // arrange: seed a profile + a cogmap-homed resource owned by it (reuse the suite's helpers)
    // act: DbBackend::new(pool, owner).set_facet(SetFacet { resource, values: json!({"k":"v"}), weight: 1.0, act, origin: Surface::ApiHttp })
    // assert: Ok(CommandOutput { value: PropertyId(..) }); a non-owner profile → Err(Forbidden)
}
```

- [ ] **Step 2: Run it to verify it fails** — `cargo nextest run -p temper-services set_facet_returns_property_id --features test-db` → FAIL (no `set_facet` impl).
- [ ] **Step 3: Surface the PropertyId in the wrapper** — change `set_facet_in_tx`/add `set_facet_with` to return `Result<PropertyId>` by threading `Fired::Facet(PropertyId)` out of `fire_with` (the SQL `facet_set` already `RETURNS uuid`).
- [ ] **Step 4: Implement `DbBackend::set_facet`** — mirror `fold_relationship` (`:1232-1262`): `self.check_can_modify_next(uuid::Uuid::from(cmd.resource)).await?` (auth before write), `check_act_invocation`, resolve owner+emitter (`writes::resolve_profile`/`resolve_emitter` with `surface_marker(cmd.origin)`), build `EventContext { invocation, authorship }`, call `writes::set_facet_with(&self.pool, cmd.resource, &cmd.values, cmd.weight, emitter, act_ctx)`, return `CommandOutput::new(property_id)`.
- [ ] **Step 5: Run the test** — PASS. Regenerate services sqlx cache if a `query!` was added: `cargo make prepare-services`.
- [ ] **Step 6: Commit**

```bash
git add crates/temper-substrate/src/writes.rs crates/temper-services/src/backend/db_backend.rs crates/temper-services/.sqlx
git commit -m "feat(services): DbBackend::set_facet over writes::set_facet (auth-gated)"
```

## Task B3: core wire types

**Files:** Modify `crates/temper-core/src/types/` (new `facet_requests.rs` mirroring `relationship_requests.rs`); export it.

- [ ] **Step 1** — Define `FacetSetRequest { resource: Uuid, values: serde_json::Value, weight: f64, #[serde(default, flatten)] act: ActInput }` and `FacetAck { property_id: Uuid }`, with the same gated derives as `AssertRelationshipRequest` (`#[cfg_attr(feature="web-api", derive(utoipa::ToSchema))]`, `#[cfg_attr(feature="typescript", derive(ts_rs::TS))]`, `#[cfg_attr(feature="mcp", derive(schemars::JsonSchema))]`).
- [ ] **Step 2** — `cargo build -p temper-core --all-features`; add a serde round-trip test mirroring the relationship-request tests. Run, commit.

```bash
git add crates/temper-core/src/types/
git commit -m "feat(core): FacetSetRequest + FacetAck wire types"
```

## Task B4: API handler + route + openapi

**Files:** Modify `crates/temper-api/src/handlers/` (new `facets.rs` mirroring `handlers/edges.rs:59-83`), `routes.rs:61-72`, `openapi.rs`.

- [ ] **Step 1** — `set_facet` handler: `#[utoipa::path(post, path="/api/facets", ...)]`, `State<AppState>`, `AuthUser`, `Json<FacetSetRequest>`; build `SetFacet { origin: Surface::ApiHttp, .. }`, `DbBackend::new(state.pool.clone(), ProfileId::from(auth.0.profile.id))`, dispatch, `Json(FacetAck { property_id })`.
- [ ] **Step 2** — Register `.route("/api/facets", post(handlers::facets::set_facet))` in `routes.rs`; add `FacetSetRequest`/`FacetAck` to `openapi.rs` schema list.
- [ ] **Step 3** — Add an API integration test (mirror the edges handler test, `temper-api --features test-db --test ...`). Run; regenerate api sqlx cache if needed (`cargo make prepare-api`). Commit.

```bash
git add crates/temper-api/src/handlers/facets.rs crates/temper-api/src/routes.rs crates/temper-api/src/openapi.rs crates/temper-api/.sqlx
git commit -m "feat(api): POST /api/facets handler + route"
```

## Task B5: temper-client method

**Files:** Modify `crates/temper-client/src/` (new `facets.rs` mirroring `relationships.rs:24-68`), `lib.rs:122` (accessor).

- [ ] **Step 1** — `Facets::set(&self, req: &FacetSetRequest) -> Result<FacetAck>` POSTing `/api/facets`; expose `Client::facets()`. Build, commit.

```bash
git add crates/temper-client/src/
git commit -m "feat(client): facets().set() over POST /api/facets"
```

## Task B6: CLI command

**Files:** Modify `crates/temper-cli/src/cli.rs` (a `Facet` action — likely under `resource` or a new top-level `facet` subcommand), `crates/temper-cli/src/commands/` (handler mirroring `commands/edge.rs:35-135`).

- [ ] **Step 1** — clap action `temper resource facet <ref> --values <json> [--weight <f64>]` + authorship flags (reuse `ActInput`/`into_act_input`); resolve `<ref>` via `parse_ref`; dispatch `runtime::with_client(|c| c.facets().set(&req))`; print the ack.
- [ ] **Step 2** — clap parse test (mirror `edge.rs:142-313`). Run; `cargo build -p temper-cli --bin temper`. Commit.

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/
git commit -m "feat(cli): resource facet — set a facet over the client"
```

## Task B7: MCP tool

**Files:** Create `crates/temper-mcp/src/tools/facets.rs` (mirror `tools/relationships.rs` `fold_relationship` `:224-256`); modify `tools/mod.rs:6` (`pub mod facets;`), `service.rs` (`#[tool]` method next to `fold_relationship` `:247-257`).

**Interfaces:** Produces MCP tool `facet_set` with `FacetSetInput { resource: String, values: serde_json::Value, weight: Option<f64>, #[serde(flatten)] act: ActInput }`.

- [ ] **Step 1: Write the failing deserialize test** — in `facets.rs` `#[cfg(test)]`, assert `FacetSetInput` deserializes with and without authorship fields (mirror `fold_relationship_input_deserializes_*` `:384-393`). Run → FAIL (struct undefined).
- [ ] **Step 2: Implement the handler** — mirror `fold_relationship` (`:224-256`): `require_profile`, parse `resource` via `parse_ref` → `ResourceId`, `input.act.into_act_context()`, build `SetFacet { resource, values: input.values, weight: input.weight.unwrap_or(1.0), act, origin: Surface::Mcp }`, `DbBackend::new(pool.clone(), profile_id)`, `backend.set_facet(cmd)`, map errors via the local `map_err`, return `FacetAck`-as-text. Add the local `to_text`/`map_err` (copy from relationships.rs `:92-110`).
- [ ] **Step 3: Register** — `pub mod facets;` in `mod.rs`; a `#[tool(description = "Set a facet (typed property) on a resource ...")]` method in `service.rs` calling `tools::facets::facet_set(self, input).await` (mirror `:247-257`).
- [ ] **Step 4** — `cargo nextest run -p temper-mcp`; `cargo make check`. Commit.

```bash
git add crates/temper-mcp/src/tools/facets.rs crates/temper-mcp/src/tools/mod.rs crates/temper-mcp/src/service.rs
git commit -m "feat(mcp): facet_set tool"
```

---

# Sequence C — telos/charter-block read tool

No "read the charter prose" surface exists. Compose the existing primitives: `cogmap_telos(p_cogmap) -> uuid` (`canonical_functions.sql:396-399`) + `resource_blocks(p_resource, p_principal_kind, p_principal_id, p_role DEFAULT NULL)` (`canonical_functions.sql:371-389`, access-gated, returns `(seq, block_id, body_text, role, reinforce_count, last_reinforced_at)`). Block roles (`statement`/`question`/`framing`) per `temper-core/src/charter.rs`.

## Task C1: services-direct charter read

**Files:** Modify `crates/temper-services/src/backend/substrate_read.rs` (new `cogmap_charter_select`, mirror `cogmap_shape_select` called from `tools/cognitive_maps.rs:41`); add a result type in `temper-core` (e.g. `CharterBlock { seq: i32, role: String, body: String }`, gated derives for ts-rs/utoipa/schemars).

- [ ] **Step 1: Write the failing read test** — `#[sqlx::test(migrator=...)]`: genesis a cogmap with a charter (reuse a fixture / the genesis path), then assert `cogmap_charter_select(pool, cogmap_id, principal)` returns the statement/question/framing blocks in seq order; and that a principal who can't read the cogmap gets an empty result (the SQL is access-gated via `resources_readable_by`).
- [ ] **Step 2: Run → FAIL** (function undefined).
- [ ] **Step 3: Implement** — a service-direct read (reads may be service-direct per CLAUDE.md): `SELECT seq, role, body_text AS body FROM resource_blocks(cogmap_telos($1), $2, $3, NULL) ORDER BY seq` via `sqlx::query_as!` into `CharterBlock`. Principal kind/id from the calling profile (mirror how `cogmap_shape_select` passes the principal).
- [ ] **Step 4: Run → PASS**; `cargo make prepare-services`. Commit.

```bash
git add crates/temper-core/src/types/ crates/temper-services/src/backend/substrate_read.rs crates/temper-services/.sqlx
git commit -m "feat(services): cogmap_charter_select — read telos charter blocks"
```

## Task C2: MCP charter-read tool

**Files:** Modify `crates/temper-mcp/src/tools/cognitive_maps.rs` (new `cogmap_read_charter`, mirror `cogmap_shape` `:22-54`), `service.rs` (register among the cogmap tools `:310-340`).

- [ ] **Step 1: Write the failing input deserialize test** — `CogmapReadCharterInput { cogmap: String }` deserializes; run → FAIL.
- [ ] **Step 2: Implement** — handler resolves `cogmap` via `parse_cogmap`, calls `C1::cogmap_charter_select(pool, cogmap_id, principal)` (service-direct read; mirror `cogmap_shape`'s service-direct call), returns the `Vec<CharterBlock>` as text.
- [ ] **Step 3: Register** the `#[tool(description = "Read a cognitive map's telos/charter blocks (statement/questions/framing) — the steward orients on this ...")]` method in `service.rs`.
- [ ] **Step 4** — `cargo nextest run -p temper-mcp`; `cargo make check`. Commit.

```bash
git add crates/temper-mcp/src/tools/cognitive_maps.rs crates/temper-mcp/src/service.rs
git commit -m "feat(mcp): cogmap_read_charter tool — telos orientation for the steward"
```

---

## Self-Review

- **Spec coverage (T1 acceptance):** `facet_set` callable over MCP with the same access gate as the resource-facet write path → Sequence B (auth via `check_can_modify_next` in B2, exposed through B7). Telos/charter read tool → Sequence C. Doctype label passes CLI→API→MCP per the D3 vocabulary → Sequence A (recognized 8 + open tail). sqlx caches regenerated → noted per-task.
- **Scope correction baked in:** the spec/task said "facets exist in substrate/CLI/API but not MCP"; the agents proved facets exist **only** in the substrate — hence B is a 9-layer slice, not a one-tool add. This plan reflects the real scope.
- **Placeholder scan:** the faithful-copy layers (B3–B7, C2) give the new type/signature + an exact copy-source `file:line` + the test; that is a copy-match instruction (the agent mapped each source), not a TODO. The novel/decision code (A1/A2 gates, B1 command, B2 impl + wrapper, C1 read) is shown in full.
- **Type consistency:** `SetFacet` (B1) → `DbBackend::set_facet` (B2) → `FacetSetRequest`/`FacetAck` (B3) → handler (B4) → client (B5) → CLI (B6) → `FacetSetInput`→`SetFacet` (B7) thread the same `resource`/`values`/`weight`/`act`/`origin` fields and `PropertyId` return. `CharterBlock { seq, role, body }` is consistent across C1/C2.
- **Sequencing:** A, B, C are independent and can be implemented/reviewed in parallel. Within B, B1→B2→B3→…→B7 is strictly ordered (each consumes the prior). T5 (Eve agent) consumes B7's `facet_set` + C2's `cogmap_read_charter` + A's labels.
