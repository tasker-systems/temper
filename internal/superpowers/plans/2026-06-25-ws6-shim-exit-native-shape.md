# WS6 Shim-Exit — Native Read Shape Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `api/cli/mcp/ui` read the native schema-derived `ResourceRow` — real (event-sourced) `created`/`updated`, name-only doc type, slug-less, no managed/open hashes — and delete the `reconstruct_resource_row` shim, instead of serving a fabricated reproduction of the old production shape.

**Architecture:** Two substantive commits. **Task 1** is additive and behavior-changing only for timestamps: the `temper-next` readbacks (`resource_row`, `enriched_list`) start carrying the real `kb_resources.created`/`updated` columns, and the two `temper-api` consumers stop stamping `Utc::now()`. **Task 2** is the atomic cutover: drop the four dead fields (`kb_doc_type_id`, `slug`, `managed_hash`, `open_hash`) from `ResourceRow`, gut-and-rename the shim to a no-fabrication mapper, fix every consumer, and regenerate the TS type — all in one commit because the whole-workspace clippy pre-commit gate requires the tree to compile clean at every commit. **Task 3** (optional) is a cosmetic rename of the `read_selector` misnomer.

**Tech Stack:** Rust (sqlx 0.8 runtime queries, axum, async-trait), `temper-next` substrate readbacks, `temper-core` shared wire types with `ts-rs`/`utoipa`/`schemars` derives, `temper-cli`, `temper-mcp`, SvelteKit `temper-ui` (ts-rs-generated types), cargo-make + cargo-nextest, e2e suite via `#[sqlx::test]`.

## Global Constraints

- **Whole-workspace clippy pre-commit gate** — `githooks/pre-commit` runs `--all-features` clippy with `-D warnings` across the workspace. **Every commit must leave the entire workspace compiling clean** (this is why Task 2's cross-crate field removal is one atomic commit, per repo convention for type refactors).
- **`--all-features` for all builds and clippy.** `#[expect(lint, reason = "...")]`, never `#[allow]`, if a suppression is ever needed (aim for none).
- **readback SQL is runtime `sqlx::query`, NOT the `query!` macros** (see `crates/temper-next/src/readback/mod.rs` module note). Adding columns to those queries needs **no `.sqlx` cache regen** and no `cargo make prepare-next`. Do not convert them to macros.
- **`cargo make` tasks force `SQLX_OFFLINE=true`** (matches CI). `cargo make check` is the honest local gate.
- **Typed structs over inline JSON; service/readback layer owns SQL.** Never inline `sqlx::query!()` outside a service/readback.
- **ts-rs types are generated** — regenerate with `cargo make generate-ts-types`; never hand-edit `packages/temper-ui/src/lib/types/generated/*.ts`.
- **`temper` binary** is invoked directly from PATH; this plan changes no CLI output format.
- Run `temper` commands directly; reinstall the PATH binary (`cargo install --path crates/temper-cli`) only if you need to exercise CLI behavior end-to-end (not required for the automated gates here).

---

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `crates/temper-next/Cargo.toml` | add `chrono` dep + sqlx `chrono` feature | 1 |
| `crates/temper-next/src/readback/mod.rs` | `ResourceRowParity` + `EnrichedListRow` gain `created`/`updated`; both SELECTs add `r.created, r.updated` | 1 |
| `crates/temper-api/src/backend/db_backend.rs` | shim consumes real timestamps (T1); then gutted+renamed to `native_resource_row` (T2) | 1, 2 |
| `crates/temper-api/src/backend/read_selector.rs` | `list_enriched_select` real timestamps (T1); drop 4 fields + rename call sites (T2) | 1, 2 |
| `crates/temper-core/src/types/resource.rs` | remove 4 fields from `ResourceRow` | 2 |
| `crates/temper-cli/src/projection.rs` | slug readers → `slug_from_title` | 2 |
| `crates/temper-cli/src/actions/ingest.rs` | drop always-None `temper-slug` block | 2 |
| `crates/temper-cli/src/actions/show_cache.rs` | delete dead tier-2 hash path; drop `temper-slug` block | 2 |
| `crates/temper-cli/src/commands/resource.rs` | delete-result slug → `slug_from_title`; fixtures | 2 |
| `crates/temper-cli/src/cloud_backend/translators.rs` | fixture | 2 |
| `crates/temper-mcp/src/tools/resources.rs` | `build_enriched` slug → `None`; fixture | 2 |
| `crates/temper-mcp/src/service.rs` | drop stale "requires managed_hash/open_hash" sentence | 2 |
| `packages/temper-ui/src/lib/types/generated/resource.ts` | regenerated (drops 4 fields) | 2 |
| `tests/e2e/tests/resource_crud_test.rs` | timestamp test (T1) + native-shape test (T2) | 1, 2 |

---

## Task 1: Surface real, event-sourced timestamps

The `kb_resources.created`/`updated` columns are already populated from `kb_events.occurred_at` at write time (create sets both to the genesis event's `occurred_at`; `resource_updated`/`resource_deleted`/body-hash-recompute bump `updated` from the mutation event). The shim discards them and stamps `Utc::now()` per read. This task surfaces the real columns. Additive — `ResourceRow` keeps `created`/`updated`; nothing is removed yet, so the tree stays green.

**Files:**
- Modify: `crates/temper-next/Cargo.toml`
- Modify: `crates/temper-next/src/readback/mod.rs` (`ResourceRowParity` ~454-485, `resource_row` ~495-568, `EnrichedListRow` ~211-236, `enriched_list` ~257-358)
- Modify: `crates/temper-api/src/backend/db_backend.rs:141-174` (`reconstruct_resource_row`)
- Modify: `crates/temper-api/src/backend/read_selector.rs:331-373` (`list_enriched_select`)
- Test: `tests/e2e/tests/resource_crud_test.rs`

**Interfaces:**
- Produces: `temper_next::readback::ResourceRowParity` gains `pub created: chrono::DateTime<chrono::Utc>` and `pub updated: chrono::DateTime<chrono::Utc>`. `EnrichedListRow` gains the same two fields. `reconstruct_resource_row` keeps its signature `async fn(&PgPool, Uuid, Uuid) -> Result<ResourceRow, TemperError>` (Task 2 renames it).

- [ ] **Step 1: Write the failing timestamp test**

Append to `tests/e2e/tests/resource_crud_test.rs`:

```rust
/// Timestamps are real and stable across reads (not read-time `now()`), and an
/// update advances `updated` without moving `created`. Pre-shim-exit the backend
/// stamped `Utc::now()` per read, so two reads of the same resource returned
/// different `created` — this test pins the native, event-sourced behavior.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_timestamps_are_real_and_stable(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");
    let context = app
        .client
        .contexts()
        .create("e2e-resource-timestamps")
        .await
        .expect("context create failed");

    let created = app
        .client
        .resources()
        .create(&ResourceCreateRequest {
            kb_context_id: context.id.into(),
            doc_type: "research".to_string(),
            origin_uri: "test://e2e/resource-timestamps".to_string(),
            title: "Timestamp Test".to_string(),
            slug: None,
        })
        .await
        .expect("resource create failed");

    let first = app
        .client
        .resources()
        .get(created.id.into())
        .await
        .expect("first get failed");
    let second = app
        .client
        .resources()
        .get(created.id.into())
        .await
        .expect("second get failed");

    assert_eq!(
        first.created, second.created,
        "created must be stable across reads, not read-time now()"
    );
    assert_eq!(
        first.updated, second.updated,
        "updated must be stable across reads"
    );

    app.client
        .resources()
        .update(
            created.id.into(),
            &ResourceUpdateRequest {
                title: Some("Timestamp Test v2".to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("update failed");

    let after = app
        .client
        .resources()
        .get(created.id.into())
        .await
        .expect("get after update failed");
    assert_eq!(
        after.created, first.created,
        "created must not change on update"
    );
    assert!(
        after.updated >= first.updated,
        "updated must advance (or hold) after an update"
    );
}
```

- [ ] **Step 2: Run the test, verify it FAILS**

Run: `cargo make test-e2e` (or scope: `cargo nextest run -p temper-e2e --features test-db resource_timestamps_are_real_and_stable`)
Expected: FAIL on `assert_eq!(first.created, second.created, ...)` — the shim stamps a fresh `Utc::now()` on each read, so the two reads differ.

- [ ] **Step 3: Add `chrono` to temper-next**

In `crates/temper-next/Cargo.toml`, add `"chrono"` to the sqlx feature list and add a direct `chrono` dependency. Mirror `crates/temper-core/Cargo.toml`'s `chrono` declaration form exactly (same version/workspace style). Result:

```toml
sqlx = { version = "0.8", features = ["runtime-tokio-rustls", "postgres", "uuid", "json", "macros", "migrate", "chrono"] }
```

plus a `chrono = ...` line under `[dependencies]` matching temper-core's.

- [ ] **Step 4: Carry timestamps through `resource_row`**

In `crates/temper-next/src/readback/mod.rs`:

Add the import near the top of the file (with the other `use` lines):
```rust
use chrono::{DateTime, Utc};
```

Add two fields to `ResourceRowParity` (after `is_active`):
```rust
    /// Real genesis timestamp — `kb_resources.created` (event `occurred_at` at create).
    pub created: DateTime<Utc>,
    /// Real last-mutation timestamp — `kb_resources.updated` (event `occurred_at` at last write).
    pub updated: DateTime<Utc>,
```

In the `resource_row` SELECT, add the two columns (e.g. after `r.is_active,`):
```sql
                r.created,
                r.updated,
```

In the `Ok(ResourceRowParity { ... })` build, add:
```rust
        created: row.get("created"),
        updated: row.get("updated"),
```

Update the `resource_row` doc-comment: delete the "Deliberately does NOT select `created`/`updated` (temper-next's sqlx has no `chrono` feature … caller stamps read-time `now()`)" sentence — it is now false.

- [ ] **Step 5: Carry timestamps through `enriched_list`**

Same file. Add to `EnrichedListRow` (after `is_active`):
```rust
    /// Real genesis timestamp — `kb_resources.created`.
    pub created: DateTime<Utc>,
    /// Real last-mutation timestamp — `kb_resources.updated`.
    pub updated: DateTime<Utc>,
```

In the `enriched_list` Query-1 SELECT, add (after `r.is_active,`):
```sql
                r.created,
                r.updated,
```

In the `EnrichedListRow { ... }` build inside the final `.map(...)`, add:
```rust
                created: r.get("created"),
                updated: r.get("updated"),
```

- [ ] **Step 6: Consume real timestamps in the shim (`reconstruct_resource_row`)**

In `crates/temper-api/src/backend/db_backend.rs`, in `reconstruct_resource_row`:
- Delete the `let now = Utc::now();` line.
- Change `created: now,` → `created: p.created,` and `updated: now,` → `updated: p.updated,`.
- Remove the now-unused `use chrono::Utc;` import at the top of the file **only if** `Utc` is unused elsewhere in the file (let `cargo make check` confirm; this file's only `Utc::now()` was here).

(Leave `slug: None`, `kb_doc_type_id: …nil()`, `managed_hash: None`, `open_hash: None` for now — Task 2 removes them.)

- [ ] **Step 7: Consume real timestamps in `list_enriched_select`**

In `crates/temper-api/src/backend/read_selector.rs`, in `list_enriched_select`:
- Delete the `let now = chrono::Utc::now();` line.
- Change `created: now,` → `created: r.created,` and `updated: now,` → `updated: r.updated,`.

- [ ] **Step 8: Run the test, verify it PASSES**

Run: `cargo make test-e2e` (the `resource_timestamps_are_real_and_stable` test)
Expected: PASS.

- [ ] **Step 9: Full local gate**

Run: `cargo make check` then `cargo make test-next`
Expected: clean (no clippy warnings; temper-next tests green with the two new fields).

- [ ] **Step 10: Commit**

```bash
git add crates/temper-next/Cargo.toml crates/temper-next/src/readback/mod.rs \
        crates/temper-api/src/backend/db_backend.rs \
        crates/temper-api/src/backend/read_selector.rs \
        tests/e2e/tests/resource_crud_test.rs
git commit -m "feat(ws6): surface real event-sourced created/updated in readback

Stop fabricating Utc::now() per read. resource_row + enriched_list now select
the real kb_resources.created/updated columns (populated from kb_events.occurred_at
at write time). ResourceRowParity/EnrichedListRow carry them; the two temper-api
consumers pass them through. Adds chrono to temper-next sqlx (runtime queries, no
cache regen). E2e test pins stable-across-reads + updated-advances-on-update.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Drop the four shim fields and retire `reconstruct_resource_row`

Atomic cutover: `ResourceRow` loses `kb_doc_type_id`, `slug`, `managed_hash`, `open_hash`; the shim becomes a pure no-fabrication mapper renamed `native_resource_row`; every consumer is fixed; the TS type is regenerated. One commit (whole-workspace clippy gate).

**Files:** (all from the table above for Task 2)

**Interfaces:**
- Consumes: `ResourceRowParity`/`EnrichedListRow` with real `created`/`updated` (Task 1).
- Produces: `ResourceRow` with 17 fields (the four dropped). Shim renamed: `pub(crate) async fn native_resource_row(pool: &PgPool, principal: uuid::Uuid, new_id: uuid::Uuid) -> Result<ResourceRow, TemperError>` (same body shape, no fabricated fields). All three `read_selector` arms (`list_select`, `show_select`, `search_select`) and the six `DbBackend` methods call `native_resource_row`.

- [ ] **Step 1: Write the failing native-shape test**

Append to `tests/e2e/tests/resource_crud_test.rs`:

```rust
/// The native ResourceRow drops the four shim fields (kb_doc_type_id, slug,
/// managed_hash, open_hash) and keeps name-only doc type. Asserts on the
/// serialized wire shape so it fails (red) while the fields still exist.
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn resource_row_native_shape_drops_shim_fields(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client
        .profile()
        .get()
        .await
        .expect("profile pre-flight failed");
    let context = app
        .client
        .contexts()
        .create("e2e-native-shape")
        .await
        .expect("context create failed");

    let created = app
        .client
        .resources()
        .create(&ResourceCreateRequest {
            kb_context_id: context.id.into(),
            doc_type: "research".to_string(),
            origin_uri: "test://e2e/native-shape".to_string(),
            title: "Native Shape".to_string(),
            slug: None,
        })
        .await
        .expect("resource create failed");

    let fetched = app
        .client
        .resources()
        .get(created.id.into())
        .await
        .expect("get failed");

    let json = serde_json::to_value(&fetched).expect("serialize ResourceRow");
    let obj = json.as_object().expect("ResourceRow serializes to an object");
    for k in ["kb_doc_type_id", "slug", "managed_hash", "open_hash"] {
        assert!(
            !obj.contains_key(k),
            "native ResourceRow must drop `{k}`, got: {json}"
        );
    }
    assert_eq!(
        obj.get("doc_type_name").and_then(|v| v.as_str()),
        Some("research"),
        "native ResourceRow keeps name-only doc type"
    );
}
```

- [ ] **Step 2: Run the test, verify it FAILS**

Run: `cargo make test-e2e` (`resource_row_native_shape_drops_shim_fields`)
Expected: FAIL — the serialized row still contains `"slug"` (and the other three).

- [ ] **Step 3: Guard — confirm no `FromRow`/`query_as` consumer of `ResourceRow`**

Run: `grep -rn "query_as.*ResourceRow\|as::<.*ResourceRow\|fetch.*ResourceRow" crates/ tests/`
Expected: no production query fetches into `ResourceRow` via `FromRow` (reads go through `readback` + manual `row.get`). If any hit exists, that query must be reconciled in this task — inspect and fix before removing fields. (The `FromRow` derive on `ResourceRow` is retained but unused.)

- [ ] **Step 4: Remove the four fields from `ResourceRow`**

In `crates/temper-core/src/types/resource.rs`, delete these four field lines from `struct ResourceRow` (with their doc-comments): `kb_doc_type_id` (21), `slug` (24), and `managed_hash`/`open_hash` (44-53). Keep `created`/`updated`/`body_hash`. Then remove `DocTypeId` from the `use super::ids::{...}` import **iff** it is now unused in the file (let `cargo make check` confirm — `ResourceListParams.kb_doc_type_id` is `Option<Uuid>`, not `DocTypeId`, so it likely becomes unused).

- [ ] **Step 5: Gut + rename the shim to `native_resource_row`**

In `crates/temper-api/src/backend/db_backend.rs`:
- Rename `reconstruct_resource_row` → `native_resource_row`.
- Delete the four field lines `kb_doc_type_id: …`, `slug: None,`, `managed_hash: None,`, `open_hash: None,` from the `ResourceRow { … }` build.
- Replace the function doc-comment with a one-paragraph "maps the substrate readback (`readback::resource_row`) to the native `ResourceRow` — real timestamps, name-only doc type, no fabrication. Shared by `show_resource` and the read selector arms." (Drop all §9-floor / "fabricated" / "re-minted nil" language.)
- Update the module-level doc-comment at the top of the file (lines 7-11) the same way — the "reconstructs the migration-invariant subset … fills the non-invariant fields best-effort … `Utc::now()`" paragraph is no longer true; replace with a native-shape description.
- Remove `DocTypeId` from the `use temper_core::types::ids::{...}` import iff now unused (clippy will confirm).
- Update the six in-file call sites (`create_resource` dedup + echo-back, `show_resource`, `update_resource`, `delete_resource`, `reassert_relationship`) from `reconstruct_resource_row(` → `native_resource_row(`.

- [ ] **Step 6: Update `read_selector` call sites + the enriched constructor**

In `crates/temper-api/src/backend/read_selector.rs`:
- Update the import `use crate::backend::db_backend::{map_readback_err, reconstruct_resource_row};` → `native_resource_row`, and the three call sites in `list_select`, `show_select`, `search_select`.
- In `list_enriched_select`, delete the four field lines (`kb_doc_type_id: …`, `slug: None,`, `managed_hash: None,`, `open_hash: None,`) from the `ResourceRow { … }` build.
- Remove `DocTypeId` from the `use temper_core::types::ids::{...}` import iff now unused (clippy confirms; `ContextId`/`ProfileId`/`ResourceId` stay).
- Update the module doc-comment (lines 1-13) and the `show_select`/`list_enriched_select` doc-comments to drop "reconstructing the production-shaped types at the §9 floor" / "slug/timestamps are §9 non-invariants (None/now())" — they now read native rows.

- [ ] **Step 7: Simplify the slug readers in `projection.rs`**

In `crates/temper-cli/src/projection.rs`, replace **both** slug-resolution blocks (in `write_resource_file_from_parts` ~227-234 and `remove_resource_file_for_row` ~294-301) — `row.slug` is gone, and both already fall back to title-derivation — with the unconditional:

```rust
    let slug = ingest::slug_from_title(&row.title);
```

(Then use `slug.as_str()` / `&slug` as the existing call sites expect; drop the now-unused `slug_owned` binding.)

- [ ] **Step 8: Drop the always-None `temper-slug` block in `build_frontmatter_from_resource`**

In `crates/temper-cli/src/actions/ingest.rs`, delete the block (lines ~198-200):

```rust
    if let Some(slug) = &resource.slug {
        fm.set_managed_field("temper-slug", serde_json::Value::String(slug.clone()));
    }
```

(`temper-slug` is §7-dissolved and `resource.slug` was always `None`, so this never executed — removal is behavior-preserving.)

- [ ] **Step 9: Delete the dead tier-2 hash path in `show_cache.rs`**

In `crates/temper-cli/src/actions/show_cache.rs`:
- In `attempt_remote`, delete the entire tier-2 short-circuit — the `let mut local_was_corrupted = false;` binding, the `if let Ok(local_body) = fs::read_to_string(...) { match try_hash_match(...) { ... } }` block (lines ~117-144), and the later `if local_was_corrupted { output::warning(...) }` block (lines ~155-160). After this, `attempt_remote` fetches `meta_check`, then fetches `content`, calls `reconstruct_full_file_content(&meta_check, &content)`, writes the file, and returns `FreshnessTier::FullFetch`. (`meta_check` stays — tier-3 reconstruction uses it.)
- Delete the `try_hash_match` function (lines ~177-207) and the `HashMatchOutcome` enum (lines ~170-175).
- In `reconstruct_full_file_content`, delete the `if let Some(slug) = &meta.slug { ... }` block (lines ~257-259).
- Remove any tests in this file that exercise `try_hash_match`/`HashMatchOutcome` (the tier-2 path is gone).
- Remove now-unused imports flagged by clippy (e.g. `FileTime`, `set_file_mtime`; **keep** `SystemTime` — `read_if_fresh` still uses it).

- [ ] **Step 10: Fix the delete-result slug in `commands/resource.rs`**

In `crates/temper-cli/src/commands/resource.rs` (~577), replace:

```rust
        slug: row.slug.clone().unwrap_or_default(),
```

with:

```rust
        slug: crate::actions::ingest::slug_from_title(&row.title),
```

- [ ] **Step 11: Fix `build_enriched` slug in temper-mcp**

In `crates/temper-mcp/src/tools/resources.rs` (~210), replace `slug: row.slug.clone(),` with `slug: None,` (the enriched output's addressable form is the `r#ref` decorated-ref on the next line; `row.slug` was always `None`). Leave `EnrichedResource.slug` itself in place — removing that field is out of scope.

- [ ] **Step 12: Drop the stale hash claim in the MCP tool description**

In `crates/temper-mcp/src/service.rs` (~177), delete the sentence "Requires current managed_hash and open_hash for the updated payloads." from the `update_resource_meta` tool `description` (the hashes are not required and are no longer on the row).

- [ ] **Step 13: Update the six test fixtures**

Remove the four field lines (`kb_doc_type_id`, `slug`, `managed_hash`, `open_hash`) from each `ResourceRow { … }` test fixture:
- `crates/temper-cli/src/actions/show_cache.rs` `test_resource_row()` (~339-361)
- `crates/temper-cli/src/actions/ingest.rs` `test_resource_row()` (~353-376)
- `crates/temper-cli/src/commands/resource.rs` `make_resource_row()` (~1106-1128) and the inline vec in `render_resource_list_json_passes_wire_type_with_internals()` (~1296-1318)
- `crates/temper-cli/src/cloud_backend/translators.rs` `sample_resource_row()` (~412-434)
- `crates/temper-mcp/src/tools/resources.rs` `sample_row()` (~735-757)

In `translators.rs` (~441) also remove the `assert!(row.slug == Some("test-task".to_string()))` assertion (field gone). The `managed_hash`-absent assertions in `commands/resource.rs` (~1561, ~1616) still pass (the key is absent) — keep them.

- [ ] **Step 14: Regenerate the TypeScript type**

Run: `cargo make generate-ts-types`
Expected: `packages/temper-ui/src/lib/types/generated/resource.ts` `ResourceRow` loses `kb_doc_type_id`, `slug`, `managed_hash`, `open_hash`. (No non-generated UI code references these — verified — so nothing else in temper-ui changes.)

- [ ] **Step 15: Full local gate (fix unused imports as flagged)**

Run: `cargo make check`
Expected: clean. Resolve any `unused import` warnings clippy surfaces (the `DocTypeId` imports in the three files; show_cache imports). Re-run until green.

- [ ] **Step 16: Run the native-shape test + the broader suite**

Run: `cargo make test-e2e` then `cargo make test`
Expected: `resource_row_native_shape_drops_shim_fields` PASSES; the Task 1 timestamp test still PASSES; unit tests green.

- [ ] **Step 17: Commit (atomic)**

```bash
git add -A
git commit -m "feat(ws6): native ResourceRow — drop shim fields, retire reconstruct_resource_row

ResourceRow loses kb_doc_type_id/slug/managed_hash/open_hash (all §7-dissolved,
fabricated). reconstruct_resource_row becomes native_resource_row: a pure
no-fabrication map from readback::resource_row, with real timestamps from Task 1.
Consumers updated (projection + delete-result slug -> slug_from_title; dead
show_cache tier-2 hash path removed; mcp enriched slug -> None; stale mcp tool
desc fixed). ts-rs regenerated; temper-ui ResourceRow slimmed. One atomic commit
(whole-workspace clippy gate). E2e asserts the native wire shape.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3 (optional): Rename `read_selector` → `substrate_read` + doc scrubs

Cosmetic. The module dispatches reads; it no longer selects a backend (the #166 collapse removed the backend-switch machinery). Skip if it adds review noise; it changes no behavior.

**Files:**
- Rename: `crates/temper-api/src/backend/read_selector.rs` → `substrate_read.rs`
- Modify: `crates/temper-api/src/backend/mod.rs` (the `mod read_selector;` declaration + any `read_selector::` references)
- Modify: any `crate::backend::read_selector::` references across `temper-api`

- [ ] **Step 1: Rename the module file**

Run: `git mv crates/temper-api/src/backend/read_selector.rs crates/temper-api/src/backend/substrate_read.rs`

- [ ] **Step 2: Update the module declaration and references**

Run: `grep -rn "read_selector" crates/temper-api/src/` — update `mod read_selector;` → `mod substrate_read;` in `backend/mod.rs`, and every `read_selector::` path → `substrate_read::`. Update the new file's module doc-comment to drop the "dispatcher (misnomer)" framing — it is now named for what it does.

- [ ] **Step 3: Gate + commit**

Run: `cargo make check && cargo make test`
Expected: clean, green.

```bash
git add -A
git commit -m "refactor(ws6): rename read_selector -> substrate_read (the misnomer)

The module dispatches substrate reads; it no longer selects a backend (the #166
collapse removed the switch machinery). Pure rename + doc scrub, no behavior change.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**
- Native shape: real timestamps (T1), drop `kb_doc_type_id`/`slug`/`managed_hash`/`open_hash` (T2.4), name-only doc type kept (T2 step-1 assertion). ✓
- Retire `reconstruct_resource_row` (T2.5, renamed to `native_resource_row` no-fabrication mapper). ✓
- ts-rs regen for temper-ui (T2.14). ✓
- Surface migration api→cli→mcp→ui (T2.5-14). ✓
- Cosmetic `read_selector` rename folded in, flagged optional (T3). ✓
- Rejected/Deferred (event-sourcing via `kb_events.resource_id`; crate split) — correctly NOT in this plan. ✓
- Gates: F3 usability floor (every commit green; e2e parity tests), timestamp-observability (timestamp test), hash-drop safety (tier-2 deletion + retained assertions). ✓
- `updated`-maintenance gate: spec flagged it for verification — confirmed already satisfied during planning (canonical functions bump `updated` from event `occurred_at`); no function change needed, so no task. ✓

**Placeholder scan:** No TBD/TODO/"add error handling"/vague tests. Every code step shows real code; the only "let clippy confirm" items are unused-import removals (a legitimate compiler-guided cleanup, not a placeholder). ✓

**Type consistency:** `native_resource_row` signature is identical to `reconstruct_resource_row` (rename only); all nine call sites updated. `ResourceRowParity`/`EnrichedListRow` field names `created`/`updated` match between definition (T1.4-5) and consumption (T1.6-7). `slug_from_title(&row.title)` used consistently for slug derivation (projection, delete-result). ✓
