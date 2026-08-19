# Managed-Meta Boundary Phase 2 — Wire-Contract Reshape Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reshape `managed_meta` so its type is *exactly* the 10 `KeyFate::Property` keys, and promote identity/home/type to first-class wire fields — across CLI, MCP, and API.

**Architecture:** `ManagedMeta` (the typed wire struct in `temper-workflow`) shrinks from 17 fields to the 10 optional Property keys. Identity (`title`/`slug`), type (`doc_type_name`), and home (`context_ref`/`cogmap`) flow through top-level command/request fields, never through `managed_meta`. The three send-side identity injectors (`ensure_managed_identity_keys` calls in the CLI translator, the two MCP handlers, and the shared DbBackend pipeline) are retired — validation's identity injection already lives in `assemble_frontmatter_document`, so schema `required` lists stay satisfied without touching a single schema file. The Property vocabulary becomes single-sourced against `MANAGED_PROPERTY_KEYS` with a compile-time drift-guard.

**Tech Stack:** Rust workspace (temper-workflow, temper-services, temper-mcp, temper-api, temper-cli, temper-substrate), sqlx/Postgres, cargo-nextest, cargo-make. serde `deny_unknown_fields`, schemars (MCP JsonSchema), utoipa (OpenAPI), clap (CLI).

**Spec:** `docs/superpowers/specs/2026-07-07-managed-open-meta-boundary-reshape-design.md` — read the "Phase 2 — resolved shape (2026-07-07 light brainstorm)" section. Every task cites the spec section it implements.

**Task:** temper task `019d7e29`. Branch: `jct/managed-meta-phase2-reshape` (already created; spec-amendment commit `ff45b2cd` is on it).

## Global Constraints

- **No backward compatibility.** Temper is cloud-native only; the local vault is a read-only projection. No shims, deprecation windows, or dual-read paths. (Spec Charter.)
- **Full surface parity, always.** MCP + CLI + API are one contract over one write path (`DbBackend`). Never drop a surface. (CLAUDE.md; memory `feedback_full_surface_parity_always`.)
- **The Property vocabulary is exactly these 10 keys**, verbatim from `MANAGED_PROPERTY_KEYS` (`crates/temper-substrate/src/keys.rs:42-53`): `temper-stage, temper-mode, temper-effort, temper-status, temper-seq, temper-llm-model, temper-llm-run, temper-provenance, temper-branch, temper-pr`.
- **The 7 fields leaving `ManagedMeta`** (`crates/temper-workflow/src/types/managed_meta.rs`): `doc_type` (temper-type), `context` (temper-context), `updated` (temper-updated), `source` (temper-source), `goal` (temper-goal), `title` (temper-title), `slug` (temper-slug).
- **Persistence layer owns SQL; surfaces dispatch through `DbBackend`.** No inline `sqlx::query!()` in a surface; no write persistence called directly from a surface. (CLAUDE.md.)
- **Typed structs over inline `json!()`** for known-shape data. (CLAUDE.md.)
- **Every `#[sqlx::test]` file needs `#![cfg(feature = "test-db")]`** (memory `project_test_db_feature_gate_convention`).
- **`cargo make` forces `SQLX_OFFLINE=true`** — after any SQL change regenerate caches (none expected in this plan; flag if one appears). `cargo make check` is the honest local probe.
- **Per-task verification:** focused test(s) + the touched crate's suite + `cargo make check` before each commit (memory `feedback_workspace_tests_at_pr_only`, `feedback_subagent_check_before_commit`). Full workspace nextest + `cargo make test-e2e` at branch end only.
- **Subagents escalate, don't soften.** If a test can only pass by weakening a contract, STOP and report BLOCKED. (memory `feedback_subagent_escalate_not_soften`.)
- **Local e2e uses a stale bin.** After any CLI change, rebuild before `test-e2e`: `cargo build -p temper-cli --bin temper` (memory `project_e2e_stale_temper_bin`).

---

## Grounded current-state map (cite, don't re-derive)

Verified on-disk 2026-07-07. Implementers treat these as the pre-grounded facts; anything not listed here, grep before use (GD-1).

- **Wire struct:** `crates/temper-workflow/src/types/managed_meta.rs:20-94` — `#[serde(deny_unknown_fields)]`, no `extra` bucket (Phase 1 removed it). 17 `Option` fields; the 7 to remove are `doc_type` (:27), `context` (:31), `updated` (:35), `source` (:39), `goal` (:55), `title` (:88), `slug` (:93).
- **Fate table (authoritative Property set):** `crates/temper-substrate/src/keys.rs` — `KeyFate` (:10-26), `MANAGED_PROPERTY_KEYS` (:42-53, 10 keys), `key_fate` (:65-78), `is_managed_property_key` (:58-60). `temper-goal => KeyFate::Edge` (:68) — leave this arm; it is the §7 synthesis fate (substrate seed/scenario), independent of the wire struct.
- **Identity injection (the redundant one):** `assemble_frontmatter_document` (`crates/temper-workflow/src/operations/actions.rs:141-191`) strips `IDENTITY_FIELDS`+`TIER1_SYSTEM_FIELDS` then injects `temper-id/-created/-type/-context/-title/-slug` from the typed `FrontmatterIdentity`. `validate_managed_meta` (`actions.rs:475-524`) builds that identity from `ValidateManagedMetaParams` (`:458-468`, has `title`/`slug`/`doc_type`/`context`) and calls it. ⇒ the `ensure_managed_identity_keys` call at `db_backend.rs:250` is redundant for validation, and schema `required` stays satisfied after removal.
- **`ensure_managed_identity_keys` (to delete):** defined `actions.rs:49-63`. Callers: CLI translator `crates/temper-cli/src/cloud_backend/translators.rs:86` (create only); MCP `crates/temper-mcp/src/tools/resources.rs:477` (create) + `:732` (update); shared pipeline `crates/temper-services/src/backend/db_backend.rs:250`.
- **Command structs:** `CreateResource` (`crates/temper-workflow/src/operations/commands.rs:26-63`, has top-level `title: String`, `slug: String`, `doctype: String`, `home: HomeAnchor`); `UpdateResource` (`:88-113`, **no** top-level title/slug today; `move_to: Option<MoveSpec>`, `MoveSpec` at `:80-84` = `context_to: Option<ContextId>`, `type_to: Option<String>`).
- **Create write path:** `db_backend.rs` `create_resource` (~:843-935) sets `title: &cmd.title`, `doc_type: &cmd.doctype`; properties via `properties_from_meta` (`:197-217`, keeps only `KeyFate::Property` managed keys + all open keys). Substrate `CreateParams` (`crates/temper-substrate/src/writes.rs:94-113`) takes top-level `title`/`doc_type`; title is written to `kb_resources.title`, never from managed_meta.
- **Update write path:** `db_backend.rs:980-1049` — currently digs `temper-title` (`:1014`) and `temper-type` (`:998`) out of `managed_meta`, falls back to `move_to.type_to` (`:1001`). Uses shared `validate_managed_meta_pipeline` (`:245-266`).
- **API handlers:** ingest create `crates/temper-api/src/handlers/ingest.rs:30-120` (top-level `title`/`slug`/`doc_type_name`/`context_ref`, untyped `Value` managed_meta parsed to `ManagedMeta`); resources create `handlers/resources.rs:177-219` (`ManagedMeta::default()`); **PATCH fold** `handlers/resources.rs:260-272` (folds top-level `title`/`slug` INTO `ManagedMeta.title/.slug` → the API-side mirror to retire); meta PUT `handlers/meta.rs:55-75` (full-replace, identity only inside managed_meta). Request types: `IngestPayload` (`crates/temper-core/src/types/ingest.rs:13-53`), `ResourceCreateRequest`/`ResourceUpdateRequest` (`crates/temper-workflow/src/types/resource.rs:152-241`), `MetaUpdatePayload` (`managed_meta.rs:148-173`).
- **MCP handlers:** `CreateResourceInput` (`resources.rs:26-70`), `UpdateResourceInput` (`:116-150`, has top-level `title`/`slug`), `UpdateResourceMetaInput` (`:164-177`, non-Option `managed_meta`). Create handler injects+deserializes (`:473-489`); update handler mirrors title/slug into managed_meta (`:715-749`).
- **CLI:** clap `Create` (`crates/temper-cli/src/cli.rs:298-352`, has `goal` :314) and `Update` (`:409-489`, has `goal` :458); `CreateResourceArgs` (`commands/resource.rs:159-182`, `goal` :167) → create `ManagedMeta` literal (`:306-311`, sets `goal` :309); `UpdateParams` (`:1006-1047`, `goal` :1024); `build_partial_managed_meta_from_args` (`:1056-1083`, `goal` :1063,:1076); `validate_update_args` (`:1287-1335`, `temper-goal` :1296); send-side translators `cmd_to_ingest_payload` (`cloud_backend/translators.rs:61-132`, inject :86) and `cmd_to_resource_update_request` (`:155-235`, no inject, lifts `title` from `managed_meta.title` :208-210, sends `slug: None` :214). **No `--updated`/`--source` CLI flags exist** — those keys are struct-only. List `--goal` filter (`cli.rs:369`, `ListParams.goal` :501) is a query param, **out of scope**.
- **Merge helpers (test-only):** `merge_managed_meta` (`actions.rs:257-309`) and `merge_open_meta` (`:320-332`) are called **only from their own `#[cfg(test)]` module** (verified — no live caller; the update path comment at `db_backend.rs:1027-1028` states no merge is needed). `merge_managed_meta` touches every typed field, incl. the 7 leaving.
- **Projection render:** `document.rs::set_managed_meta` (`crates/temper-workflow/src/frontmatter/document.rs:251-301`) renders `temper-goal` (:273-275), `temper-title` (:297-299), `temper-slug` (:300-302) from struct fields. Vault-file identity is *also* injected from the resource row on the projection path (`crates/temper-cli/src/actions/ingest.rs:~210-250`).
- **Schemas:** `crates/temper-workflow/schemas/base.schema.json` `required: [temper-id, temper-type, temper-context, temper-created, temper-title]`; `task.schema.json` `required: [temper-stage, temper-slug]`, declares `temper-goal` in `properties`. **No schema `required` change needed** (validation injects identity). Only cleanup: drop `temper-goal` from `task.schema.json` `properties` (P2.5).
- **OpenAPI:** `crates/temper-api/src/openapi.rs:96` registers `ManagedMeta` directly as a component schema; `ResourceCreateRequest`/`ResourceUpdateRequest`/`MetaUpdatePayload` at `:81,:82,:94`.

---

## Task ordering rationale

Field removal in a `deny_unknown_fields` world has a hard ordering constraint: the send-side injectors write `temper-title` into an untyped `Value` that is then deserialized into `ManagedMeta`. Once the field is gone, that deserialize **rejects**. So injectors must be retired **before** the struct shrinks. Sequence: (1) stop the CLI writing `goal`, (2) give `UpdateResource` a top-level identity path, (3) retire all injectors/mirrors, (4) shrink the struct + drift-guard, (5) default + round-trip safety tests, (6) blast-radius regen. Each task leaves the tree compiling and green.

---

## Task 1: Remove the `--goal` write flag from CLI create/update (P2.1, CLI surface)

Stops the CLI from setting `ManagedMeta.goal`, so the field is unreferenced by CLI code before the struct shrink. `temper-goal` is `KeyFate::Edge` and inert on the live write path (never becomes a property or a live edge); the real first-class-goal feature is deferred to task `019f3d55`. The List `--goal` *filter* stays (query param, out of scope).

**Files:**
- Modify: `crates/temper-cli/src/cli.rs` (remove `Create.goal` :313-315, `Update.goal` :457-459)
- Modify: `crates/temper-cli/src/commands/resource.rs` (remove `CreateResourceArgs.goal` :167; create `ManagedMeta` literal `goal` :309; `UpdateParams.goal` :1024; `build_partial_managed_meta_from_args` `goal` :1063,:1076; `validate_update_args` `temper-goal` :1296; the `Create`/`Update` clap destructuring that binds `goal`; the `empty_update_params` test helper `goal: None` :1360)
- Test: inline `#[cfg(test)]` modules in `crates/temper-cli/src/commands/resource.rs`

**Interfaces:**
- Produces: `CreateResourceArgs` and `UpdateParams` no longer carry `goal`; `build_partial_managed_meta_from_args` builds a `ManagedMeta` with no `goal`. Later tasks rely on the create `ManagedMeta` literal (`resource.rs:306-311`) setting only `mode`/`effort`.

- [ ] **Step 1: Write the failing test** — assert `--goal` is no longer a valid create flag.

Add to the CLI parse test module (find the existing `mod` that constructs `Cli::try_parse_from`; match its style):

```rust
#[test]
fn create_rejects_removed_goal_flag() {
    // temper-goal is KeyFate::Edge, not a managed property; --goal is removed
    // (goal-as-edge is deferred to task 019f3d55). clap must reject it.
    let res = crate::cli::Cli::try_parse_from([
        "temper", "resource", "create", "--type", "task",
        "--title", "T", "--context", "@me/temper", "--goal", "some-goal",
    ]);
    assert!(res.is_err(), "--goal must be rejected on create");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo nextest run -p temper-cli create_rejects_removed_goal_flag`
Expected: FAIL — `--goal` currently parses, so `res.is_err()` is false.

- [ ] **Step 3: Remove the `goal` clap fields and all CLI plumbing**

In `cli.rs`, delete the `Create.goal` field (`:313-315`) and `Update.goal` field (`:457-459`). In `commands/resource.rs`: delete `CreateResourceArgs.goal` (`:167`); remove `goal` from the create-arm destructuring and from the `ManagedMeta` literal so `:306-311` becomes:

```rust
        managed_meta: ManagedMeta {
            mode: mode.map(String::from),
            effort: effort.map(String::from),
            ..ManagedMeta::default()
        },
```

Delete `UpdateParams.goal` (`:1024`) and its binding in the `Update` arm; remove the `goal` lines from `build_partial_managed_meta_from_args` (`:1063` in the `any_set` chain and `:1076` in the struct literal); remove the `("temper-goal", …)` row from `validate_update_args` (`:1296`); remove `goal: None` from `empty_update_params` (`:1360`).

- [ ] **Step 4: Run the test + CLI suite to verify green**

Run: `cargo nextest run -p temper-cli create_rejects_removed_goal_flag && cargo nextest run -p temper-cli`
Expected: PASS. (If any test set `--goal` or `params.goal`, update it to drop the goal expectation — do not re-add the flag.)

- [ ] **Step 5: `cargo make check`**

Run: `cargo make check`
Expected: clean (fmt + clippy + machete + TS).

- [ ] **Step 6: Commit**

```bash
git add crates/temper-cli/src/cli.rs crates/temper-cli/src/commands/resource.rs
git commit -m "cloud-only(meta-p2): drop CLI --goal write flag (temper-goal is Edge, deferred to 019f3d55)"
```

---

## Task 2: Add top-level `title`/`slug` to `UpdateResource` and wire the update handlers (P2.2)

`UpdateResource` gains a first-class identity path so identity no longer needs to ride inside `managed_meta`. Added alongside the existing path (no removal yet) so the tree stays green.

**Files:**
- Modify: `crates/temper-workflow/src/operations/commands.rs:88-113` (add fields to `UpdateResource`)
- Modify: `crates/temper-services/src/backend/db_backend.rs:980-1046` (prefer `cmd.title`/`cmd.slug` for effective title/slug)
- Modify: `crates/temper-api/src/handlers/resources.rs:236-307` (set `cmd.title`/`cmd.slug` from `req`)
- Modify: `crates/temper-mcp/src/tools/resources.rs` update handler (set `cmd.title`/`cmd.slug` from `input`)
- Modify: `crates/temper-cli/src/cloud_backend/translators.rs:155-235` (set `title`/`slug` on the cmd it builds — verify the CLI builds an `UpdateResource` cmd; if it builds `ResourceUpdateRequest` directly, thread top-level title/slug there)
- Test: `crates/temper-api/tests/resource_update_merge_test.rs` (or a new `resource_update_identity_test.rs`)

**Interfaces:**
- Produces: `UpdateResource { title: Option<String>, slug: Option<String>, .. }`. Task 3 relies on these being populated so it can delete the managed_meta identity dig.

- [ ] **Step 1: Add the fields**

In `commands.rs`, extend `UpdateResource` (keep existing fields; add near `resource`):

```rust
pub struct UpdateResource {
    pub resource: ResourceId,
    /// New title (identity). First-class; not carried in managed_meta.
    pub title: Option<String>,
    /// New slug (identity). Optional; server derives from title when absent.
    pub slug: Option<String>,
    pub body: Option<BodyUpdate>,
    pub managed_meta: Option<ManagedMeta>,
    // … unchanged …
}
```

This breaks every `UpdateResource { … }` literal (missing fields). Fix each construction site to add `title`/`slug`:
- `crates/temper-api/src/handlers/resources.rs` PATCH (`~:291`): `title: req.title.clone()`, `slug: req.slug.clone()` — **and leave the existing fold for now** (removed in Task 3).
- `crates/temper-api/src/handlers/ingest.rs` update (`~:170`): `title: None, slug: None` (ingest-update has no separate identity input).
- `crates/temper-api/src/handlers/meta.rs` (`~:65`): `title: None, slug: None` (meta path is Property-only per Fork 2).
- `crates/temper-mcp/src/tools/resources.rs` update handler: `title: input.title.clone()`, `slug: input.slug.clone()`.
- `crates/temper-cli/src/cloud_backend/translators.rs`: if a `UpdateResource` is built here, set from the cmd; else no-op.

- [ ] **Step 2: Write the failing test** — a PATCH that changes only the title via the top-level field updates the row.

In `crates/temper-api/tests/resource_update_merge_test.rs` (match the `#[sqlx::test(migrator = "temper_api::MIGRATOR")]` + `common::setup_test_app` style; file already `#![cfg(feature = "test-db")]`):

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn patch_title_only_updates_row_title(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let (token, resource_id) = setup_profile_and_resource(&app).await; // shared helper

    let resp = app.client.patch(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({ "title": "Renamed via top-level field" }))
        .send().await.expect("PATCH failed");
    assert_eq!(resp.status().as_u16(), 200, "body: {}", resp.text().await.unwrap_or_default());

    let got = app.client.get(app.url(&format!("/api/resources/{resource_id}")))
        .header("Authorization", format!("Bearer {token}")).send().await.unwrap()
        .json::<serde_json::Value>().await.unwrap();
    assert_eq!(got["title"], "Renamed via top-level field");
}
```

(If `setup_profile_and_resource` is private to another test file, add a local copy or lift it to `common` — match the existing create-then-patch helper at `resource_update_validation_test.rs:28-64`.)

- [ ] **Step 3: Run test — expect it to pass already OR fail on wiring**

Run: `cargo nextest run -p temper-api --features test-db --test resource_update_merge_test patch_title_only_updates_row_title`
Expected: with the fold still present it may already pass. If it fails, the wiring gap is real — proceed to Step 4.

- [ ] **Step 4: Make the update write path prefer `cmd.title`/`cmd.slug`**

In `db_backend.rs:980-1046`, change `effective_title` to prefer the top-level cmd field, falling back to the current row (drop reliance on `temper-title` from managed_meta):

```rust
let effective_title = cmd.title.clone().unwrap_or_else(|| current.title.clone());
let effective_slug = cmd.slug.clone()
    .unwrap_or_else(|| temper_workflow::operations::sluggify(&effective_title));
// title column update: Some when the caller changed it
title = cmd.title.clone();
```

Keep the `temper-type` type-move detection reading `move_to.type_to` (it already falls back there at `:1001`); the `incoming.get("temper-type")` dig is removed in Task 3 when the field is gone. **Do not** remove the managed_meta title dig yet if the fold still writes it — Task 3 removes both together. (If keeping both compiles and the test passes, that is the intended green intermediate.)

- [ ] **Step 5: Run test + API suite**

Run: `cargo nextest run -p temper-api --features test-db --test resource_update_merge_test && cargo nextest run -p temper-workflow`
Expected: PASS.

- [ ] **Step 6: `cargo make check`**

Run: `cargo make check`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-workflow/src/operations/commands.rs crates/temper-services/src/backend/db_backend.rs crates/temper-api crates/temper-mcp crates/temper-cli
git commit -m "cloud-only(meta-p2): add first-class title/slug to UpdateResource, wire update handlers"
```

---

## Task 3: Retire the identity injectors/mirrors and delete `ensure_managed_identity_keys` (P2.2)

With identity flowing through top-level fields, every send-side injection and handler mirror is now either redundant (validation re-injects via `assemble_frontmatter_document`) or actively harmful (injecting `temper-title` into a `Value` that `deny_unknown_fields` will reject once the field is gone). Retire all four call sites, then delete the function.

**Files:**
- Modify: `crates/temper-services/src/backend/db_backend.rs:245-266` (drop the `ensure_managed_identity_keys` line from `validate_managed_meta_pipeline`; simplify `ManagedValidationParams` if `identity_slug` becomes unused)
- Modify: `crates/temper-mcp/src/tools/resources.rs` (create `:473-489` — stop injecting + re-deserializing; update `:715-749` — stop mirroring title/slug into managed_meta)
- Modify: `crates/temper-cli/src/cloud_backend/translators.rs:81-87` (drop the create-side inject; `managed_meta` serializes straight from the typed cmd)
- Modify: `crates/temper-api/src/handlers/resources.rs:260-272` (delete the PATCH fold; `title`/`slug` already set on the cmd in Task 2)
- Modify: `crates/temper-workflow/src/operations/actions.rs:49-63` (delete `ensure_managed_identity_keys`) and `crates/temper-workflow/src/operations/mod.rs` (drop its re-export)

**Interfaces:**
- Produces: `validate_managed_meta_pipeline` no longer mutates identity into the managed value; `managed_meta` on the wire carries only caller-supplied managed keys. Task 4 (shrink) depends on no code deserializing an injected `temper-title` into `ManagedMeta`.

- [ ] **Step 1: Write the failing test** — MCP create no longer round-trips `temper-title` through managed_meta.

The precise failure this guards: after Task 4 shrinks the struct, MCP create's `serde_json::from_value::<ManagedMeta>(injected_value)` (`resources.rs:488`) would reject. Prove the inject is gone now. Add to the MCP create test module (match its style; if MCP create is only covered via e2e, add the assertion to the e2e create test instead — see `tests/e2e/tests/`):

```rust
// The MCP create handler must NOT inject temper-title/temper-slug into the
// managed_meta value it sends to the backend — identity travels top-level.
// (Regression guard for the deny_unknown_fields shrink in Task 4.)
```

If no unit seam exists, make this an e2e assertion: create a task via the MCP tool path with `managed_meta: None` and assert success + the returned resource has the right title (identity came from the top-level `title`, not managed_meta).

- [ ] **Step 2: Run it to confirm current behavior**

Run the chosen test; expected to pass trivially today (inject is present but harmless pre-shrink) or fail if you assert on internal absence. Use it as the post-condition oracle for Step 3–4.

- [ ] **Step 3: Retire the DbBackend pipeline injection**

In `db_backend.rs`, delete the `ensure_managed_identity_keys(&mut managed, params.title, params.identity_slug);` call in `validate_managed_meta_pipeline` (`:250-254`). Validation still assembles identity via `validate_managed_meta` → `assemble_frontmatter_document`. If `identity_slug` on `ManagedValidationParams` is now unused, remove the field and its two call-site assignments (create + update).

- [ ] **Step 4: Retire the MCP inject + mirror**

Create handler (`resources.rs:470-489`): delete the `ensure_managed_identity_keys` block and the round-trip; build the cmd's `managed_meta` directly from `input.managed_meta.unwrap_or_default()`:

```rust
let managed_meta = input.managed_meta.unwrap_or_default();
```

Keep `slug` derivation from title for the top-level cmd field (`resources.rs:460-462`) — that feeds `cmd.slug`, not managed_meta. Update handler (`:715-749`): delete the title/slug mirror into managed_meta; `cmd.title`/`cmd.slug` were set in Task 2.

- [ ] **Step 5: Retire the CLI translator inject**

`translators.rs:81-87` becomes:

```rust
let managed_meta = Some(serde_json::to_value(&cmd.managed_meta)
    .map_err(|e| TemperError::Project(format!("serialize managed_meta: {e}")))?);
```

`IngestPayload.title`/`.slug` are already set from `cmd.title`/`cmd.slug` (`:109,:115`). Update the translator test `cmd_to_ingest_payload_always_injects_identity_keys` (`translators.rs:337-350`) to assert the **inverse**: `managed_meta` does NOT contain `temper-title`/`temper-slug`, and the top-level `payload.title`/`payload.slug` carry identity.

- [ ] **Step 6: Retire the API PATCH fold + delete the function**

`resources.rs:260-272`: delete the fold; pass `req.managed_meta` straight through (identity is `cmd.title`/`cmd.slug` from Task 2). Then delete `ensure_managed_identity_keys` (`actions.rs:49-63`), its `mod.rs` re-export, and its unit tests. `rg -n ensure_managed_identity_keys crates` must return zero hits.

- [ ] **Step 7: Run affected suites**

Run: `cargo nextest run -p temper-workflow && cargo nextest run -p temper-cli && cargo nextest run -p temper-api --features test-db && cargo nextest run -p temper-mcp`
Expected: PASS. Rebuild the CLI bin if any e2e follows: `cargo build -p temper-cli --bin temper`.

- [ ] **Step 8: `cargo make check` + commit**

Run: `cargo make check`

```bash
git add crates/temper-services crates/temper-mcp crates/temper-cli crates/temper-api crates/temper-workflow
git commit -m "cloud-only(meta-p2): retire ensure_managed_identity_keys + all send-side identity injectors/mirrors"
```

---

## Task 4: Shrink `ManagedMeta` to the 10 Property keys + drift-guard (P2.1 + P2.4)

The core reshape. Remove the 7 fields; fix the now-broken references (merge helper, projection render, remaining tests); add the compile-time drift-guard that pins `ManagedMeta`'s Property fields to `MANAGED_PROPERTY_KEYS` and `key_fate`.

**Files:**
- Modify: `crates/temper-workflow/src/types/managed_meta.rs:20-94` (remove 7 fields; update `#[cfg(test)]` fixtures that set them)
- Modify: `crates/temper-workflow/src/operations/actions.rs:257-309` (`merge_managed_meta` — drop the 7 arms; update the `merge_managed_meta_covers_all_typed_fields` test)
- Modify: `crates/temper-workflow/src/frontmatter/document.rs:251-302` (remove the `temper-goal`/`temper-title`/`temper-slug` render arms from `set_managed_meta`)
- Modify (read-path goal, discovered during Task 1 — breaks compile when the field goes): `crates/temper-cli/src/actions/task.rs` — `TaskInfo.goal` field populated from `meta.goal` (`:122`), and the `load_tasks` `goal_slug` param + filter (`:38,:41,:91-92`). Live callers (`warmup.rs`, internal) pass `None`; drop the param and update them. `t.goal` and goal-filtering leave with the vocab key.
- Modify (inert `list --goal` flag — already a no-op; `list()` builds `ResourceListParams` with no goal field, so it only warns): remove `ListParams.goal` (`commands/resource.rs:498`), its two hint blocks (`:514-519`), the clap `List.goal` (`cli.rs:~366`), and the `main.rs` List-arm `goal` binding. Honest cleanup — stop advertising a filter that never filtered.
- Modify: `tests/e2e/tests/cloud_task_lookup_e2e_test.rs` — it seeds `temper-goal` via a raw API payload and asserts `t.goal` / goal-filtering; both leave with the vocab key. Remove the goal seeding + the `load_tasks_filters_by_goal_slug` test (its capability moves to `019f3d55`).
- Create: drift-guard test in `crates/temper-workflow/src/types/managed_meta.rs` `#[cfg(test)]` (or a dedicated `tests/` if a cross-crate view of `MANAGED_PROPERTY_KEYS` is needed — `temper-workflow` depends on `temper-substrate`, so import `temper_substrate::keys::MANAGED_PROPERTY_KEYS` directly)

> **Scope note (discovered in Task 1):** list-by-goal is not a live feature — the CLI `--goal` list flag never filtered (no `goal` on `ResourceListParams`), and `load_tasks`' filter is exercised only by an e2e test. So goal leaves *entirely* in this arc (write flag ✓ Task 1, vocab + read-path here). The follow-up task `019f3d55` reintroduces goal as a first-class field with a live **edge**, and list-by-goal returns as edge-based filtering there.

**Interfaces:**
- Produces: `ManagedMeta` with exactly `stage, mode, effort, status, seq, branch, pr, llm_model, llm_run, provenance` (all `Option`). Every downstream serialize/deserialize now round-trips only Property keys.

- [ ] **Step 1: Write the drift-guard test (failing until the struct matches)**

In `managed_meta.rs` tests:

```rust
#[test]
fn managed_meta_property_fields_match_single_source() {
    use temper_substrate::keys::{MANAGED_PROPERTY_KEYS, key_fate, KeyFate};
    // Every serde(rename) on ManagedMeta must be a MANAGED_PROPERTY_KEY, and
    // every MANAGED_PROPERTY_KEY must be a field on ManagedMeta. This kills the
    // temper-llm-model drift class (spec P2.4).
    let serialized = serde_json::to_value(ManagedMeta {
        stage: Some("s".into()), mode: Some("m".into()), effort: Some("e".into()),
        status: Some("a".into()), seq: Some(1), branch: Some("b".into()),
        pr: Some("p".into()), llm_model: Some("x".into()), llm_run: Some("r".into()),
        provenance: Some("llm-discovered".into()),
    }).unwrap();
    let field_keys: std::collections::BTreeSet<String> =
        serialized.as_object().unwrap().keys().cloned().collect();
    let source_keys: std::collections::BTreeSet<String> =
        MANAGED_PROPERTY_KEYS.iter().map(|s| s.to_string()).collect();
    assert_eq!(field_keys, source_keys,
        "ManagedMeta fields drifted from MANAGED_PROPERTY_KEYS");
    // And each is Property-fated.
    for k in MANAGED_PROPERTY_KEYS {
        assert_eq!(key_fate(k), KeyFate::Property, "{k} must be Property-fated");
    }
}
```

Note: this test literal has no `..Default::default()` — after the shrink it must name **exactly** the 10 fields, so it doubles as a compile-time guard that no field was missed.

- [ ] **Step 2: Run it to verify it fails to compile**

Run: `cargo nextest run -p temper-workflow managed_meta_property_fields_match_single_source`
Expected: FAIL to compile — the struct literal names 10 fields but the struct still has 17 without them being set, and includes removed fields when built elsewhere. (Compile failure is the red state here.)

- [ ] **Step 3: Remove the 7 fields**

In `managed_meta.rs`, delete `doc_type` (:26-27), `context` (:30-31), `updated` (:34-35), `source` (:38-39), `goal` (:54-55), `title` (:87-88), `slug` (:92-93). Update the struct doc-comment (`:7-17`) to state the closed Property vocabulary. Fix the in-file test fixtures that set these (`managed_meta_yaml_roundtrip` sets `title`/`slug` :248-249; `managed_meta_accepts_the_closed_vocabulary` includes `temper-title`/`temper-slug`/`temper-type` :318-322; the `*_title_*`/`*_slug_*` tests :378-424 — delete those, they assert removed fields; `managed_meta_rejects_unknown_keys` :300 uses `temper-type`/`temper-title` — change the accepted-key example to a Property key and keep a rejection case for a now-removed key like `temper-title`).

Add a rejection regression:

```rust
#[test]
fn managed_meta_rejects_former_identity_keys() {
    for k in ["temper-title","temper-slug","temper-type","temper-context","temper-goal","temper-updated","temper-source"] {
        let json = format!(r#"{{"{k}":"x"}}"#);
        assert!(serde_json::from_str::<ManagedMeta>(&json).is_err(),
            "{k} left the managed vocabulary and must now be rejected");
    }
}
```

- [ ] **Step 4: Fix `merge_managed_meta` and `document.rs`**

`merge_managed_meta` (`actions.rs:257-309`): remove the arms for the 7 removed fields; it now merges only the 10 Property fields. Update `merge_managed_meta_covers_all_typed_fields` (`:659-688`) to drop `goal`/`title` assertions. (These helpers are test-only per the current-state map — if the team prefers, they may be deleted as dead code; keeping them shrunk is the lower-risk default.)

`document.rs::set_managed_meta` (`:251-302`): delete the `meta.goal` (:273-275), `meta.title` (:297-299), `meta.slug` (:300-302) render arms. **Verify** the projected vault file still carries `temper-title`/`temper-slug`: those come from the row-driven injection on the projection path — add/confirm a test in `document.rs` or `crates/temper-cli/src/actions/` that a projected frontmatter contains `temper-title` sourced from identity, not from `ManagedMeta`.

- [ ] **Step 5: Run the workflow suite + drift-guard**

Run: `cargo nextest run -p temper-workflow`
Expected: PASS, including `managed_meta_property_fields_match_single_source` and `managed_meta_rejects_former_identity_keys`.

- [ ] **Step 6: Full-touch compile across dependents**

Run: `cargo nextest run -p temper-services --features test-db --no-run && cargo nextest run -p temper-mcp --no-run && cargo nextest run -p temper-cli --no-run && cargo nextest run -p temper-api --features test-db --no-run`
Expected: all compile. Fix any remaining `.title`/`.slug`/`.goal`/`.doc_type`/`.context`/`.updated`/`.source` field access on a `ManagedMeta` (grep: `rg -n "managed_meta\.(title|slug|goal|doc_type|context|updated|source)\b|mm\.(title|slug|goal)\b" crates`). Each such site must now read the top-level identity field instead.

- [ ] **Step 7: `cargo make check` + commit**

Run: `cargo make check`

```bash
git add crates/temper-workflow
git commit -m "cloud-only(meta-p2): shrink ManagedMeta to the 10 Property keys + drift-guard vs MANAGED_PROPERTY_KEYS"
```

---

## Task 5: Defaults + round-trip safety tests (P2.3 + risk closure)

Prove the caller-never-required invariant and close the two grounded risks: `updated`/`source` no longer accepted, and the meta-only PUT path stays coherent Property-only (Fork 2).

**Files:**
- Test: `crates/temper-api/tests/managed_meta_reject_test.rs` (extend) and `crates/temper-api/tests/resource_update_validation_test.rs` (extend), plus an e2e in `tests/e2e/tests/` for the CLI create-with-no-managed-meta path.

- [ ] **Step 1: Write the "no managed_meta" create test (P2.3)**

In `managed_meta_reject_test.rs` (or a `create_defaults_test.rs`):

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn ingest_with_no_managed_meta_succeeds_with_defaults(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("nomm-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);

    let resp = app.client.post(app.url("/api/ingest"))
        .header("Authorization", format!("Bearer {token}"))
        .json(&json!({
            "title": "No managed meta", "origin_uri": format!("test://{}", uuid::Uuid::new_v4()),
            "context_ref": context_id.to_string(), "doc_type_name": "task",
            "slug": "no-managed-meta", "content": "body"
            // no managed_meta key at all
        })).send().await.expect("ingest failed");
    assert_eq!(resp.status().as_u16(), 200, "body: {}", resp.text().await.unwrap_or_default());
    // Default temper-stage=backlog was applied server-side (before validation).
    // Assert via a follow-up GET on the resource's stage/managed_meta.
}
```

- [ ] **Step 2: Write the `updated`/`source` rejection test (risk closure)**

```rust
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn ingest_rejects_temper_updated_and_source(pool: PgPool) {
    let app = common::setup_test_app(pool).await;
    let email = format!("sys-{}@example.com", uuid::Uuid::new_v4());
    let (profile_id, context_id) =
        common::fixtures::create_test_profile_with_context(&app.pool, &email).await;
    let token = common::generate_test_jwt(&format!("test|{profile_id}"), &email);
    for key in ["temper-updated", "temper-source"] {
        let resp = app.client.post(app.url("/api/ingest"))
            .header("Authorization", format!("Bearer {token}"))
            .json(&json!({
                "title": "sys", "origin_uri": format!("test://{}", uuid::Uuid::new_v4()),
                "context_ref": context_id.to_string(), "doc_type_name": "task",
                "slug": "sys", "content": "b", "managed_meta": { key: "x" }
            })).send().await.expect("ingest failed");
        assert_eq!(resp.status().as_u16(), 400, "{key} must be rejected as a non-managed key");
    }
}
```

- [ ] **Step 3: Meta-only PUT stays Property-only (Fork 2)**

Confirm an existing or new test in `resource_update_validation_test.rs`: `PUT /api/resources/{id}/meta` with `managed_meta: {"temper-stage":"done"}` succeeds, and one attempting identity (`{"temper-title":"x"}`) is rejected (400). Identity changes require the full PATCH path.

- [ ] **Step 4: e2e — CLI create with no workflow flags succeeds**

In `tests/e2e/tests/` (match the harness; run needs the rebuilt bin). Assert `temper resource create --type session --title "S" --context @me/<ctx>` (no stage/mode/effort) returns success and the created resource has `temper-stage`/defaults where applicable. Rebuild first: `cargo build -p temper-cli --bin temper`.

- [ ] **Step 5: Run the tests**

Run: `cargo nextest run -p temper-api --features test-db --test managed_meta_reject_test --test resource_update_validation_test`
Expected: PASS. Then the e2e subset per `cargo make test-e2e` (rebuild bin first).

- [ ] **Step 6: `cargo make check` + commit**

```bash
git add crates/temper-api/tests tests/e2e
git commit -m "cloud-only(meta-p2): tests — no-managed_meta create defaults, updated/source rejection, meta-only Property-only"
```

---

## Task 6: Regenerate the blast radius + doc-comment cleanup (P2.5)

Update the four generated/derived surfaces and fix the stale `extra`-bucket comments. No behavior change — this is the contract surface catching up to the code.

**Files:**
- Modify: `crates/temper-mcp/src/tools/resources.rs` — `CreateResourceInput`/`UpdateResourceInput`/`UpdateResourceMetaInput` doc-comments (`:56-64`, `:137-145`, `:159-177`) describing `managed_meta`; remove stale `extra`-bucket prose (`:159-163`, `:486`).
- Modify: `crates/temper-workflow/src/types/managed_meta.rs:111-113,:155-158` — stale `extra` references in `ResourceMetaResponse` / `MetaUpdatePayload` doc-comments.
- Modify: CLI `--help` text in `crates/temper-cli/src/cli.rs` for `create`/`update` where it enumerates managed fields (ensure no `--goal` remains; managed flags list matches the Property vocabulary).
- Modify: `crates/temper-workflow/schemas/task.schema.json` — remove `temper-goal` from `properties` (it left the vocabulary; not in `required`, so safe).
- Modify: temper skill docs — `~/.claude/skills/temper/reference.md` and any managed-meta prose (the skill is generated from `crates/temper-cli` per memory `project_teach_agents_goal_and_telos_differentiation`; regenerate rather than hand-edit if a generator exists — check `cargo make` tasks / `crates/temper-cli` for the skill-gen path first).
- Verify: `crates/temper-api/src/openapi.rs` — `ManagedMeta` schema (`:96`) regenerates from the shrunk struct automatically; no code change, but confirm the OpenAPI snapshot/tests (if any) are refreshed.

- [ ] **Step 1: Update MCP + type doc-comments**

Rewrite the `managed_meta` field docs to: *"Managed workflow/provenance metadata — a closed, temper-owned vocabulary of optional `temper-*` keys (stage/mode/effort/status/seq/branch/pr/llm-model/llm-run/provenance). Identity (`title`/`slug`), type (`doc_type_name`), and home (`context_ref`/`cogmap`) are first-class fields, not metadata. Unknown keys are rejected; caller-defined fields belong in `open_meta`."* Delete every sentence referencing an `extra` bucket.

- [ ] **Step 2: `describe_doc_type` reflects the shrunk vocabulary**

Confirm `crates/temper-mcp/src/tools/doc_types.rs` `describe_doc_type` surfaces the 10-key managed vocabulary (it derives from the schema/struct — verify no hardcoded identity keys remain in its output). Add/adjust a test asserting `temper-title` no longer appears as a managed key in `describe_doc_type` output.

- [ ] **Step 3: Schema cleanup**

Remove `temper-goal` from `task.schema.json` `properties`. Run the schema-driven tests (`cargo nextest run -p temper-workflow`) — if a hash fixture in `frontmatter/tiers.rs` moves, investigate before regenerating (memory: hashes are regression anchors).

- [ ] **Step 4: Regenerate/verify the temper skill docs**

Check for a skill-generation path (`rg -n "skill" crates/temper-cli/src` / `cargo make` tasks). If generated: regenerate and commit the diff. If hand-maintained: update `reference.md` managed-meta prose to the Property-only vocabulary and drop `--goal`.

- [ ] **Step 5: Full check + OpenAPI**

Run: `cargo make check` (compiles the utoipa derives; `ManagedMeta` schema regenerates). If an OpenAPI snapshot test exists, update it. If TS types are generated from `ManagedMeta` (`ts-rs`), run `cargo make generate-ts-types` and commit the regenerated `managed_meta.ts`.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "cloud-only(meta-p2): regenerate blast radius (MCP/CLI/OpenAPI/skill docs) + drop stale extra-bucket comments"
```

---

## Branch-End Verification (before PR)

- [ ] `cargo make check` — clean.
- [ ] Full workspace tests: `cargo make test-all` (memory: full workspace nextest at branch end only).
- [ ] Rebuild CLI bin then e2e: `cargo build -p temper-cli --bin temper && cargo make test-e2e` (and `cargo make test-e2e-embed` if any push-body/ingest fixture was touched).
- [ ] `rg -n "ensure_managed_identity_keys" crates` → zero hits.
- [ ] `rg -n "managed_meta\.(title|slug|goal|doc_type|context|updated|source)\b" crates` → zero hits.
- [ ] `rg -ni "extra.*bucket|flatten.*extra" crates` → zero stale references.
- [ ] Open PR (regular merge, not squash — memory `feedback_prefer_regular_merge_not_squash`). Mark task `019d7e29` done after merge.

## Self-Review notes (spec coverage)

- **P2.1** (shrink to Property-only): Task 4 + Task 1 (CLI goal). **P2.2** (promote identity/home/type, retire injectors): Tasks 2–3. **P2.3** (create with no managed_meta): Task 5 Step 1. **P2.4** (single-source + drift-guard): Task 4 Step 1. **P2.5** (blast radius + doc cleanup): Task 6.
- **No schema `required` changes** — validation injects identity via `assemble_frontmatter_document`; confirmed on-disk. Only `task.schema.json` `properties` loses `temper-goal` (cosmetic single-source).
- **Deferred (task `019f3d55`):** first-class `goal` field + live edge projection. The List `--goal` *filter* is intentionally untouched (query concern, not the write contract).
- **Open verification risks flagged inline:** (a) projected vault file must still carry `temper-title` post-shrink (Task 4 Step 4 verify); (b) `updated`/`source` round-trip safety (Task 5 Step 2); (c) whether the temper skill docs are generated or hand-maintained (Task 6 Step 4 — grep first).
