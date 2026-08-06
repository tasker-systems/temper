# ResourceView Convergence Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collapse six resource representations into one `ResourceView` returned by every read and write surface, so a human or agent learns one shape instead of six.

**Architecture:** `ResourceView` replaces `ResourceRow` / `ResourceDetail` / `ResourceSummary` / `HitIdentity` on the wire and on the `Backend` trait. It carries everything a resource has except its body — including `ref`, which stops being a CLI print-time injection. Workflow metadata (`stage`/`mode`/`effort`/`seq`) is no longer hoisted onto it; `managed_meta` becomes always-present so nothing is lost. Search hits become wrappers (`{ resource, fts_norm }`) rather than flattened variants, which keeps each arm's quantity incommensurable. `--meta-only` retires in favour of a section vocabulary.

**Tech Stack:** Rust (axum, sqlx, serde, ts-rs, utoipa, schemars, clap), PostgreSQL 17/18, generated clients in Ruby and TypeScript.

---

## Authority

This plan is an **index over** the following, which are the authority. Read them; do not rely on this
document's summaries.

| | |
|---|---|
| **Decision** | [A resource has one shape — `ResourceView` collapses three projections, workflow metadata stops being hoisted, and sections replace `--meta-only`](https://temperkb.io) — `019fd71e-792e-7560-8890-8ffde06dbe24` |
| **Task** | `019fd25e-95f0-7373-9a6e-0574deea5ab3` — see its `## Scope amendment` section |
| **Frame register** | `019fbdb9-f287-79c0-aab6-efa0b1de12c8` |
| **Amends** | §3 of `docs/superpowers/specs/2026-08-05-query-builder-compositional-design.md` (its per-kind projection is narrower than what lands here) |

**Branch:** `jct/search-exact-and-wide-step1`, which already carries phase-1 steps 1, 2, 3 and 5
(`30e08f59`, `05b025c7`, `59ee9100`) plus a merge of `origin/main`.

---

## Global Constraints

Copied verbatim from repo CLAUDE.md and the decision. Every task's requirements implicitly include this section.

- **Typed structs over inline JSON.** Never `serde_json::json!()` for data with a known structure.
- **Shared types at boundaries** live in `temper-core` / `temper-workflow` with `ts-rs` derives. Never mirror a Rust struct in a hand-written zod schema or Ruby model.
- **Persistence is its own layer; surfaces dispatch through `DbBackend`.** Never inline `sqlx::query!()` in a handler, MCP tool, or CLI action. Writes go through the `Backend` trait; reads stay service-direct.
- **Auth before writes.** Authorization checks precede any mutation.
- **Profile scoping.** Every data query scopes through `resources_visible_to` / `can_modify_resource`.
- **Params structs** for functions with more than 5 domain-related parameters. `#[expect(clippy::too_many_arguments)]` is a smell to fix.
- **`#[expect(lint, reason = "...")]`, never `#[allow]`.** All public types implement `Debug`.
- **ts-rs cannot codegen `#[serde(flatten)]`.** Verified: `ResourceDetail` (`crates/temper-workflow/src/types/resource.rs:187`) drops its `TS` derive for exactly this. `ResourceView` must not use `flatten`.
- **`cargo make fix` BEFORE `cargo make openapi`.** `fix` re-wraps the utoipa `description` string that is embedded in `openapi.json`; regenerating first and formatting after silently re-stales the artifact. Recorded in session `019fd454-0105-75a1-8cfe-d027c7623ad4`.
- **`cargo make openapi` adds models but never deletes them.** Orphaned `.rb` files for retired types must be removed by hand.
- **Drift gates compare against COMMITTED state.** Correctly-regenerated artifacts stay red until committed. Expected; do not chase.
- **Never run `cargo make prepare-e2e` and commit the result blind** — it emitted 364 files of dependency closure into `tests/e2e/.sqlx` (which holds 10 real entries). Verify each cache diff entry individually.
- **`cargo nextest run -p temper-api` with no test filter HANGS** at list enumeration. Always scope to an integration test target: `--test <name>`.
- **Capture output as `cmd > log 2>&1; echo $?`.** The harness's task-completion exit code is unreliable for compound commands, and a pipe through `tail` returns tail's status.

---

## Grounding evidence

Quoted from disk on 2026-08-06 at `59ee9100` + `origin/main`. Every task below cites into this.

### The rule this change restores

`crates/temper-workflow/src/types/managed_meta.rs:102-105`:

> Named `id` (not `resource_id`) so this response is a literal strict subset of
> [`crate::types::resource::ResourceDetail`]: `--meta-only` returns the same keys the
> full `show` does, and nothing else. With two different anchor names the subset
> relation is unachievable.

### The six shapes

| shape | defined at | carried by |
|---|---|---|
| `ResourceRow` | `temper-workflow/src/types/resource.rs:18` | list default; **and** create / update / annotate / meta returns |
| `ResourceDetail` | `temper-workflow/src/types/resource.rs:187` | `show`; `#[serde(flatten)] row` — no ts-rs codegen |
| `ResourceMetaResponse` | `temper-workflow/src/types/managed_meta.rs:99` | `GET /meta` |
| `ResourceSummary` | `temper-workflow/src/operations/backend.rs:36` | `Backend::list_resources`; `{slug, doctype, context, title}` |
| `SearchHit` | `temper-workflow/src/operations/backend.rs:44` | `{ summary, score: f32 }` — a **bare `score`** |
| `HitIdentity` | `temper-substrate/src/readback/mod.rs:1475` | `hit_identities` enrichment |

`ExactHit` / `WideHit` (`temper-core/src/types/api.rs:179`, `:205`) are the wire hits.

### The hoisted fields, and their canonical homes

`ResourceRow` carries `stage`, `seq`, `mode`, `effort` under the comment `// Managed meta projections`
(`resource.rs:48`). All four have counterparts in `ManagedMeta` (`managed_meta.rs:46-86`) —
`temper-stage`, `temper-seq`, `temper-mode`, `temper-effort` — plus `temper-status` for goals.
**Dropping them from the top level loses nothing.**

### CLI-side injections that the shipped skill treats as wire properties

| affordance | injected at | on the wire? | MCP sees it? |
|---|---|---|---|
| `ref` | `temper-cli/src/commands/resource.rs:88,142,161` (via `decorated_ref`, `temper-workflow/src/operations/refs.rs:86`) | **no** | **no** |
| `returned` | `temper-cli/src/commands/resource.rs:1016` | **no** | **no** |
| `truncated` | `temper-cli/src/commands/resource.rs:1017` | **no** | **no** |

The shipped skill instructs agents to use all three. `crates/temper-mcp/src/tools/resources.rs`
emits none of them.

### Paging: defaults, not caps

`crates/temper-cli/src/commands/resource.rs:830,833`:

```rust
const DEFAULT_LIST_LIMIT: usize = 20;
const DEFAULT_META_LIST_LIMIT: usize = 50;
```

`resolve_list_limit` (`:985-991`) returns `None` for `--all`, else `limit.unwrap_or(default)`. An
explicit `--limit` is honoured unchanged (`:2679`). **No server-side clamp exists** in
`crates/temper-api/src/handlers/resources.rs`. The `cli.rs:507` help text calling this a
"Maximum results" is wrong on both counts.

### The list envelope

`crates/temper-workflow/src/types/resource.rs:311`:

```rust
pub struct ResourceListResponse {
    pub rows: Vec<ResourceRow>,
    pub total: i64,
    pub facets: ResourceFacets,
}
```

`ResourceMetaListResponse` (`managed_meta.rs:143`) is its mirror with the row type swapped to
`ResourceDetail`; `handlers/resources.rs:36` models the endpoint as
`oneOf<ResourceListResponse, ResourceMetaListResponse>`. Its own doc comment records that it cannot
derive `ts_rs::TS` because `ResourceDetail` cannot, and that "the SvelteKit UI types the list
endpoint as `ResourceListResponse` regardless; this shape is a structural superset, so the extra keys
are simply ignored there."

### The `Backend` trait surface

`crates/temper-workflow/src/operations/backend.rs:56-90` — `create_resource`, `update_resource`,
`annotate_resource` return `CommandOutput<ResourceRow>`; `show_resource` returns
`CommandOutput<ResourceDetail>`; `list_resources` returns `CommandOutput<Vec<ResourceSummary>>`.

### Blast radius, counted

| | count | character |
|---|---|---|
| `ResourceRow` references | 44 files | overwhelmingly construction and passthrough |
| `ResourceDetail` references | 19 files | concentrated in `temper-cli/src/commands/memory/*` (35 refs / 5 files) |
| **readers of the four hoisted fields** | **1** | `temper-cli/src/commands/warmup.rs:272-277` |
| vault projection writer | 0 | `temper-cli/src/projection.rs` reads `context_name`, `doc_type_name`, `title`, `id` only |
| Atlas / graph | 0 | `temper-services/src/services/graph_service.rs:345` reads `n.stage` on a graph-node row, not `ResourceRow` |
| temper-ui components | 0 | generated types re-exported from `packages/temper-ui/src/lib/types/index.ts`; no component consumes a hit or row |

---

## File structure

**Created**

| file | responsibility |
|---|---|
| `crates/temper-workflow/src/types/resource_view.rs` | `ResourceView`, `ResourceSection`, `SectionSet`, and their tests. New file rather than growing `resource.rs` (already 500+ lines and about to lose three types) |
| `crates/temper-cli/src/commands/resource_sections.rs` | `--with`/`--without` → `SectionSet` resolution and its contradiction error |

**Modified** — the load-bearing ones; construction sites not listed individually

| file | change |
|---|---|
| `crates/temper-workflow/src/types/resource.rs` | `ResourceRow` / `ResourceDetail` retired; `ResourceListResponse` gains paging state |
| `crates/temper-workflow/src/types/managed_meta.rs` | `ResourceMetaResponse` → `ResourceView`; `ResourceMetaListResponse` deleted |
| `crates/temper-workflow/src/operations/backend.rs` | trait returns `ResourceView`; `ResourceSummary` + `SearchHit` deleted |
| `crates/temper-substrate/src/readback/mod.rs` | `HitIdentity` → `ResourceView`; `hit_identities` widened |
| `crates/temper-services/src/backend/substrate_read.rs` | read paths build `ResourceView` |
| `crates/temper-services/src/backend/db_backend.rs` | write commands return `ResourceView` |
| `crates/temper-api/src/handlers/resources.rs` | `oneOf` collapsed; sections honoured |
| `crates/temper-core/src/types/api.rs` | `ExactHit` / `WideHit` become wrappers |
| `crates/temper-mcp/src/tools/resources.rs` | returns `ResourceView` |
| `crates/temper-cli/src/cli.rs` | `--meta-only` retired; `--with`/`--without`/`--page` added |
| `crates/temper-cli/src/commands/warmup.rs` | reads `managed_meta` |

---

## Beat A — the type

### Task 1: `ResourceView`

**Files:**
- Create: `crates/temper-workflow/src/types/resource_view.rs`
- Modify: `crates/temper-workflow/src/types/mod.rs`
- Test: in-file `#[cfg(test)] mod tests`

**Interfaces:**
- Produces: `pub struct ResourceView` with field set exactly as the decision records it. Anchor is `id: ResourceId` — **never `resource_id`**. Derives: `Debug, Clone, Serialize, Deserialize, FromRow`, plus feature-gated `ts_rs::TS` (`export_to = "resource.ts"`), `utoipa::ToSchema`, `schemars::JsonSchema`.
- **CONFORM** — anchor name, `managed_meta.rs:102-105`. **CONFORM** — no `flatten`, `resource.rs:187`.

**Field set** (grounded: identity/home/attribution fields carried over from `ResourceRow:19-78`;
`ref` from `refs.rs:86`; `kb_uri` from `ExactHit` at `api.rs:184`):

`id`, `r#ref`, `title`, `kb_uri`, `origin_uri` · `kb_context_id`, `context_name`, `context_slug`,
`context_owner_ref`, `context_ref`, `cogmap_id`, `cogmap_name` · `doc_type_name`, `owner_handle`,
`owner_profile_id`, `originator_profile_id`, `is_active`, `created`, `updated` ·
`body_hash`, `ingest_state`, `body_storage` · `managed_meta: ManagedMeta` (**not** `Option`),
`open_meta: Option<serde_json::Value>`, `content: Option<String>`.

> `managed_meta` is non-`Option` — that is what makes dropping the hoisted fields lossless. `content`
> is `Option` and is the `body` section; absent means not requested, never "empty body".

- [ ] **Step 1: Write the failing tests.** Four, each isolating one conjunct:
  - `anchor_is_id_not_resource_id` — serialize, assert the JSON has key `id` and **no** key `resource_id`.
  - `no_workflow_field_is_hoisted` — assert serialized JSON has no top-level `stage`, `mode`, `effort`, `seq`.
  - `managed_meta_is_always_present` — round-trip a view whose `ManagedMeta` is all-`None`; assert the `managed_meta` key is present in the JSON.
  - `body_absent_is_distinguishable_from_body_empty` — `content: None` omits the key; `content: Some(String::new())` emits `""`.
- [ ] **Step 2: Run them, verify each fails** — `cargo nextest run -p temper-workflow resource_view`. Expected: compile failure (type absent).
- [ ] **Step 3: Write the struct.** Field set above. `#[serde(rename = "ref")]` on `r#ref`. `skip_serializing_if = "Option::is_none"` on every `Option`, `content` included.
- [ ] **Step 4: Run to green.**
- [ ] **Step 5: Bite-check `managed_meta_is_always_present`** — temporarily make `managed_meta` an `Option` with `skip_serializing_if`; confirm that test and only that test fails. Restore and `git diff` to verify byte-identical.
- [ ] **Step 6: Commit** — `git commit -m "ResourceView — one shape, id-anchored, no hoisted workflow fields"`

### Task 2: sections

**Files:**
- Modify: `crates/temper-workflow/src/types/resource_view.rs`
- Test: in-file

**Interfaces:**
- Produces: `pub enum ResourceSection { Body, OpenMeta, Edges }` (serde `kebab-case`), `pub struct SectionSet(BTreeSet<ResourceSection>)` with `contains`, and `FromStr` on the enum.
- **EXTEND** — authorized by the decision's `--meta-only` retires; sections replace it.

- [ ] **Step 1: Failing tests** — `section_names_are_kebab_case` (`open-meta`, not `open_meta` or `openMeta`); `unknown_section_name_is_rejected_naming_the_valid_set` (the error message enumerates all three — an agent that guesses wrong must be able to recover from the message alone).
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Green.**
- [ ] **Step 5: Commit.**

### Task 3: list envelope carries its own paging state

**Files:**
- Modify: `crates/temper-workflow/src/types/resource.rs:311` (`ResourceListResponse`)
- Test: in-file

**Interfaces:**
- Produces: `ResourceListResponse { rows: Vec<ResourceView>, total: i64, returned: i64, truncated: bool, limit: Option<i64>, offset: i64, facets: ResourceFacets }`
- **AMEND** — `returned`/`truncated` move from CLI injection (`commands/resource.rs:1016-1017`) onto the wire. Authorized by the decision's CLI-side-injection finding.

- [ ] **Step 1: Failing tests** — `truncated_is_true_when_a_page_hides_rows` (total 100, returned 20, offset 0); `truncated_is_false_on_the_last_page` (total 25, returned 5, offset 20 — the off-by-one case, and the one a naive `total > returned` gets wrong); `all_returns_untruncated` (`limit: None`, returned == total).
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Implement**, with `truncated` derived from `offset + returned < total`.
- [ ] **Step 4: Green.**
- [ ] **Step 5: Bite-check** — change the derivation to `returned < total`; confirm `truncated_is_false_on_the_last_page` fails and the other two stay green. Restore, `git diff` to verify.
- [ ] **Step 6: Commit.**

---

## Beat B — persistence

### Task 4: `hit_identities` returns `ResourceView`

**Files:**
- Modify: `crates/temper-substrate/src/readback/mod.rs:1475-1520` (`HitIdentity`, `hit_identities`)
- Test: `crates/temper-substrate/tests/search_exact_and_wide.rs`

**Interfaces:**
- Consumes: `ResourceView` (Task 1).
- Produces: `pub async fn hit_identities(pool: &PgPool, principal: ProfileId, ids: &[ResourceId]) -> Result<Vec<ResourceView>>` — signature unchanged but for the return type. `HitIdentity` is deleted.
- **CONFORM** — the batched single-round-trip property (`readback/mod.rs:1486-1489`: "This replaces a per-hit `resource_row` call — 50 results meant 51 queries"). **The widened SELECT must not reintroduce the N+1.**

- [ ] **Step 1: Failing test** — `hit_identities_is_one_round_trip_and_carries_both_meta_tiers`: enrich 10 ids, assert every returned view has `managed_meta` populated and `content: None`.
- [ ] **Step 2: Run, verify fail.**
- [ ] **Step 3: Widen the query** to join the manifest for both meta tiers. Read the existing `SELECT` at `:1503` first — extend it, do not rewrite it; its visibility gate is load-bearing.
- [ ] **Step 4: Green.**
- [ ] **Step 5: Bite-check the visibility gate** — remove the `resources_visible_to` join; confirm an existing visibility test fails. Restore, `git diff`.
- [ ] **Step 6: Commit.**

### Task 5: read paths build `ResourceView`

**Files:**
- Modify: `crates/temper-services/src/backend/substrate_read.rs`
- Test: `crates/temper-api/tests/` (existing resource read tests)

**Interfaces:**
- Consumes: `ResourceView`, `SectionSet`.
- Produces: read functions taking a `SectionSet` and populating `content` / `open_meta` per it.
- **CONFORM** — service-direct reads (repo CLAUDE.md); do not route reads through the `Backend` trait.

- [ ] **Step 1: Failing tests** — `section_body_absent_omits_content`; `section_open_meta_absent_omits_open_meta`; `managed_meta_present_regardless_of_sections`.
- [ ] **Step 2–4:** red, implement, green.
- [ ] **Step 5: Commit.**

### Task 6: `Backend` trait returns `ResourceView`; `ResourceSummary` and `SearchHit` deleted

**Files:**
- Modify: `crates/temper-workflow/src/operations/backend.rs:36-90`, `crates/temper-workflow/src/operations/mod.rs:24`
- Modify: `crates/temper-services/src/backend/db_backend.rs`, `crates/temper-cli/src/cloud_backend/backend.rs:175,185,600,609`
- Test: `crates/temper-services/tests/segmented_backend_test.rs`

**Interfaces:**
- Produces: `create_resource`, `update_resource`, `annotate_resource`, `show_resource` all return `CommandOutput<ResourceView>`; `list_resources` returns `CommandOutput<Vec<ResourceView>>`.
- **AMEND** — deleting `ResourceSummary` (`backend.rs:36`) and `SearchHit` (`:44`). Authorized by decision §3. `SearchHit`'s bare `score: f32` is what `no_two_acts_order_by_the_same_name_and_none_of_them_is_a_bare_score` forbids on the wire; it must not survive as the trait's private exception.
- ⚠️ **Plan/reality note:** a second, unrelated `SearchHit` exists at `crates/temper-cli/src/actions/types.rs:34`. It is CLI-local and **out of scope** — do not delete it, and check imports carefully when removing the workflow one.

- [ ] **Step 1:** Update the trait signatures; compile and let the errors enumerate the call sites.
- [ ] **Step 2:** Fix each construction site. `#[act_span]` and `act_context(&cmd.act)` on write commands stay exactly as they are — this task changes return types only.
- [ ] **Step 3:** `cargo nextest run -p temper-services --features test-db --test segmented_backend_test` → green.
- [ ] **Step 4: Commit.**

---

## Beat C — surfaces

### Task 7: `/api/resources` — one row type, one list response

**Files:**
- Modify: `crates/temper-api/src/handlers/resources.rs:31-36,189,228,293`, `crates/temper-api/src/handlers/meta.rs:62`, `crates/temper-api/src/handlers/ingest.rs:181`
- Modify: `crates/temper-workflow/src/types/managed_meta.rs` — delete `ResourceMetaListResponse`; `ResourceMetaResponse` becomes an alias-free `ResourceView`
- Test: `crates/temper-api/tests/` — add `resource_view_test.rs`

**Interfaces:**
- Produces: `GET /api/resources` returns `ResourceListResponse` unconditionally. The `ResourceListRows` enum and its `oneOf` are deleted. `?meta_only=true` is replaced by `?sections=open-meta`.
- **AMEND** — the `oneOf` at `handlers/resources.rs:31-36`.

- [ ] **Step 1: Failing tests** — `list_returns_one_shape_regardless_of_sections`; `list_envelope_carries_returned_and_truncated`; `show_and_single_row_list_agree_when_body_excluded` (**the convergence witness** — serialize both, assert byte-equal).
- [ ] **Step 2–4:** red, implement, green.
- [ ] **Step 5: Bite-check the witness** — re-add a hoisted `stage` field to `ResourceView` and confirm `show_and_single_row_list_agree_when_body_excluded` still passes (it should — both sides gain it), then confirm `no_workflow_field_is_hoisted` from Task 1 fails. **This proves the witness alone is insufficient and Task 1's test is load-bearing.** Restore, `git diff`.
- [ ] **Step 6: Commit.**

### Task 8: search hits become wrappers

**Files:**
- Modify: `crates/temper-core/src/types/api.rs:179-260` (`ExactHit`, `WideHit`)
- Modify: `crates/temper-services/src/backend/substrate_read.rs` (hit assembly), `crates/temper-client/src/search.rs`
- Test: `crates/temper-api/tests/search_two_arms_test.rs`

**Interfaces:**
- Produces: `pub struct ExactHit { pub resource: ResourceView, pub fts_norm: f32 }`; `pub struct WideHit { pub resource: ResourceView, pub vec_norm: f32 }`. The inlined identity fields (`api.rs:161-168` comment) are deleted along with the comment explaining why they were inlined.
- **CONFORM** — the quantity stays on the hit, never on `ResourceView`. Frame register `no-cross-act-ranking`.

- [ ] **Step 1: Failing tests** — `hit_carries_the_same_shape_as_list` (assert an `ExactHit`'s `resource` serializes identically to a list row for the same id); `no_quantity_field_exists_on_resource_view` (assert the serialized `resource` object has neither `fts_norm` nor `vec_norm`).
- [ ] **Step 2–4:** red, implement, green.
- [ ] **Step 5: Bite-check** — move `fts_norm` onto `ResourceView`; confirm `no_quantity_field_exists_on_resource_view` fails. Restore, `git diff`.
- [ ] **Step 6: Commit.**

### Task 9: MCP resource tools return `ResourceView`

**Files:**
- Modify: `crates/temper-mcp/src/tools/resources.rs` (incl. `sample_row` at `:1566`)
- Test: in-file, plus `tests/e2e/tests/mcp_get_resource_meta_test.rs`

**Interfaces:**
- Consumes: `ResourceView`.
- **AMEND** — MCP gains `ref`, `returned` and `truncated`, which it has never emitted.

- [ ] **Step 1: Failing test** — `mcp_list_emits_ref_for_every_row`. This is the defect the shipped skill has been instructing agents around; it gets its own named witness.
- [ ] **Step 2–4:** red, implement, green.
- [ ] **Step 5: Commit.**

---

## Beat D — CLI

### Task 10: sections replace `--meta-only`

**Files:**
- Create: `crates/temper-cli/src/commands/resource_sections.rs`
- Modify: `crates/temper-cli/src/cli.rs:538-546,556-580` — delete `meta_only` from both `List` and `Show`; delete the three `conflicts_with = "meta_only"` edges at `:565,569,574`; add `--with` / `--without` (both `value_delimiter = ','`)
- Modify: `crates/temper-cli/src/commands/resource.rs`
- Test: `crates/temper-cli/src/cli.rs` `mod meta_only_flag_tests` at `:1966` — rewrite, do not delete

**Interfaces:**
- Consumes: `ResourceSection`, `SectionSet`.
- Produces: `pub fn resolve_sections(with: &[String], without: &[String], defaults: SectionSet) -> Result<SectionSet, TemperError>`.
- **AMEND** — decision, `--meta-only` retires.

- [ ] **Step 1: Failing tests** — `show_defaults_include_body`; `list_never_offers_body` (`list --with body` errors); `with_and_without_the_same_section_is_a_contradiction` (errors, does **not** pick a winner); `show_with_edges_without_body_is_accepted` (**the combination the old design forbade** — this test is the reason the vocabulary changed).
- [ ] **Step 2–4:** red, implement, green.
- [ ] **Step 5: Commit.**

### Task 11: paging — one default, `--page`, honest help text

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs:830-833` (delete `DEFAULT_META_LIST_LIMIT`), `:985-991`, `:1016-1017` (delete the injections — the wire carries them now), `:1029` (`warn_truncated`)
- Modify: `crates/temper-cli/src/cli.rs:507` (help text), `:579` / `:546` (`--fields` help)
- Test: `crates/temper-cli/src/commands/resource.rs` tests at `:2671-2712`

**Interfaces:**
- Produces: `--page <N>` (1-indexed), mutually exclusive with `--offset` and with `--all`. Resolves to `offset = (page - 1) * limit`.
- **AMEND** — one default (20), `DEFAULT_META_LIST_LIMIT` deleted.

- [ ] **Step 1: Failing tests** — `page_one_is_offset_zero`; `page_resolves_against_the_effective_limit` (`--page 3 --limit 5` → offset 10, **not** 40 — the bug a hardcoded default would cause); `page_and_offset_together_are_rejected`; `truncated_comes_from_the_wire_not_the_client`.
- [ ] **Step 2–4:** red, implement, green.
- [ ] **Step 5: Fix the two wrong help strings.** `cli.rs:507` says "Maximum results" for what is a default with no cap. `show --fields` says "resource_id always preserved"; the code passes `"id"` (`commands/resource.rs:1649`). Both are verified-wrong today.
- [ ] **Step 6: Commit.**

### Task 12: `warmup` reads `managed_meta`

**Files:**
- Modify: `crates/temper-cli/src/commands/warmup.rs:265-281` (`collect_in_progress_tasks`)
- Test: in-file

**Interfaces:**
- **CONFORM** — `ManagedMeta` field names, `managed_meta.rs:46-86`.
- This is **the only reader of the dropped fields** in the repo. If a second appears during Beat B's compile errors, stop and report — the count was wrong.

- [ ] **Step 1: Failing test** — `in_progress_filter_reads_managed_meta_stage`.
- [ ] **Step 2–4:** red, implement, green.
- [ ] **Step 5: Commit.**

---

## Beat E — artifacts and prose

### Task 13: regenerate every drift-gated artifact

**Files:** `openapi.json`, `clients/temper-rb/**`, `clients/temper-ts/src/generated/schema.ts`, `packages/temper-ui/src/lib/types/generated/*.ts`

**REQUIRED SUB-SKILL:** read the `generated-artifacts` skill before starting.

- [ ] **Step 1:** `cargo make fix` — **before** `openapi`, per Global Constraints.
- [ ] **Step 2:** `cargo make generate-ts-types`, then `cargo make openapi`.
- [ ] **Step 3:** Delete by hand the orphaned `.rb` models for retired types — `unified_search_result_row.rb` is already gone; expect `resource_row.rb`, `resource_detail.rb`, `resource_meta_list_response.rb`. `cargo make openapi` adds models but never deletes them.
- [ ] **Step 4:** `cd packages/temper-ui && bun install && bun run check`. **`cargo make check` does not cover temper-ui.** `ResourceListResponse` now codegens for the first time (it could not while `ResourceDetail` used `flatten`), so the UI may newly type-error where it previously accepted a structural superset — that is the bug being fixed, not a regression.
- [ ] **Step 5:** `cargo make check` → green.
- [ ] **Step 6: Commit** all regenerated artifacts together (they ride along; a partial commit leaves the drift gate red).

### Task 14: sqlx caches

**REQUIRED SUB-SKILL:** read the `sqlx-query-cache` skill. The workspace ritual does **not** cover test-target queries.

- [ ] **Step 1:** `cargo sqlx prepare --workspace -- --all-features`.
- [ ] **Step 2:** Inspect the diff **entry by entry**. `hit_identities`' widened query will appear. Do **not** `git add` untracked bulk — session `019fd454` records `prepare-e2e` emitting 364 files of dependency closure into a cache holding 10 real entries.
- [ ] **Step 3: Commit** only the verified entries.

### Task 15: the two stale-prose defects

**Files:**
- Modify: `crates/temper-cli/skill-content/cognitive-maps.md:42,268,274`
- Modify: `agent-skills/temper-knowledge-base/knowledge-base.md:146-147`

- [ ] **Step 1:** `cognitive-maps.md` still teaches `temper search "<query>" --wayfind --regions 20`. Those flags were deleted in `05b025c7`. Rewrite the cross-map section to describe what actually exists. **`skills-drift` is green on this** — it compares source template to projection, and both are stale, so they agree. The gate cannot catch this; a human must.
- [ ] **Step 2:** `knowledge-base.md:146-147` describes `search` as returning "scored results with snippets." It returns two arms carrying `fts_norm` / `vec_norm`, and has carried no snippet since `UnifiedSearchResultRow`. Hand-written and gate-uncovered by design.
- [ ] **Step 3:** `temper skill emit --path agent-skills/temper-knowledge-base` and confirm `cargo make check`'s `skills-drift` step is green.
- [ ] **Step 4: Commit.**

---

## Self-review

**Spec coverage.** Every clause of decision `019fd71e-792e-7560-8890-8ffde06dbe24` maps to a task:
one shape → 1, 5, 6, 7, 8, 9; no hoisted workflow fields → 1, 12; `managed_meta` always-present → 1;
`ref` on the wire → 1, 9; hits as wrappers → 8; sections replace `--meta-only` → 2, 10; terminal
`{exact, wide}` keying → **no task, correctly** (it is a decision not to act).

**Type consistency.** `ResourceView` is the single name throughout; `SectionSet` is produced in Task 2
and consumed in 5, 7, 10; `hit_identities` keeps its name and parameter list, changing only its
return type.

**Declared uncovered, not silently skipped.**
- **No clause of the frame register is witnessed by this plan.** It reshapes a contract; no act runs under the frame.
- **No answer-quality measurement**, and none owed — the standing caution stays five-for-five.
- **No payload measurement.** `managed_meta` always-present is argued from being typed and bounded, not measured on a real `list` page. Task 13's UI check will surface a size problem only if it breaks a type, not if it is merely large.
- **The `temper-ui` count of 0 consuming components is a grep result, not a rendered page.** Task 13 Step 4 is what actually tests it.
- **Two `wayfind_region_scores` properties remain orphaned** (`salience_is_normalized_per_anchor_kind`, `zero_centroid_region_does_not_hijack_the_top_n`) — inherited from `05b025c7`, re-homed when `survey` gets a door in phase 4. Untouched here.

**On code blocks.** Where superpowers:writing-plans asks for implementation bodies and this project's
`implementation-grounding.md` GD-4 says *"do not write invented code bodies into plans — the sketch
wins over the correct prose beside it"*, this plan follows the project. Type definitions and commands
quoted from disk appear; invented function bodies do not. Every task cites `file:line` so the
implementer reads the real thing.
