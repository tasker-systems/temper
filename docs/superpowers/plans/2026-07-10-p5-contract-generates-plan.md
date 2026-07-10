# P5 — Make the Emitted OpenAPI Contract Generate: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Repair `openapi.json` so it is valid OpenAPI and `openapi-generator` produces a usable Ruby client from it, and add regression tests so neither defect can return.

**Architecture:** Two source-level fixes in `temper-api` — register two orphaned component schemas, and give every operation a unique `operation_id` — each guarded by a Rust unit test that drives the router-derived spec. The emitted `openapi.json` is regenerated from the router. No routes, handlers, or wire types change.

**Tech Stack:** Rust, utoipa 5.4.0 / utoipa-axum, cargo-make, cargo-nextest, Docker (for the generator acceptance check only).

## Global Constraints

- **utoipa version is 5.4.0.** `#[utoipa::path(...)]` accepts `operation_id = "..."` (verified: `utoipa-gen-5.4.0/src/path.rs:130`). It is the only supported spelling.
- **No behavior change.** Routes, handler bodies, wire types, and SQL are untouched. Only `#[utoipa::path]` attributes, the `components(schemas(...))` list, its `use` statement, tests, and the emitted artifact change.
- **`openapi.json` is never hand-edited.** It is a product of the router. Regenerate with `cargo make openapi`. The existing `check-openapi-spec.sh` gate diffs the committed file against a fresh emission and fails on drift.
- **A merge conflict in `openapi.json` is never resolved by hand.** Sibling sessions are landing correlation-threading work that also regenerates it, and this branch renames 27 `operationId`s, so a textual conflict is near-certain. Resolution is always: take either side wholesale, re-run `cargo make openapi`, and let `check-openapi-spec.sh` confirm. Hand-merging a generated 300KB artifact produces a file that matches no router.

  ```bash
  git checkout --theirs openapi.json && cargo make openapi
  bash .github/scripts/check-openapi-spec.sh
  ```
- **`--all-features` for builds and clippy**, per repo convention. Do **not** pass `--all-features` to the nextest commands in this plan: it enables `test-db`, which requires a live `DATABASE_URL`. These tests need no database.
- **Never `cargo nextest run -p temper-api` unscoped** — it hangs at test-list enumeration on the `temper-api` bin target. Always scope with `--lib` (verified working).
- **Lint suppression uses `#[expect(lint, reason = "...")]`**, never `#[allow]`.
- **Pre-commit runs workspace clippy** (`--all-targets`). A per-crate `cargo clippy -p temper-api` does **not** compile test targets and will not catch a broken test.
- **Anchor edits on function names, not line numbers.** Line numbers in this plan were captured before any edit; they shift as you work.
- **`DATABASE_URL` must be exported for bare `cargo`.** `sqlx::query!` macros compile against the live dev database, and P3's migration `20260709000050_act_correlation_passthrough.sql` changed the arity of `resource_create` and `relationship_assert`. If `temper-substrate` fails to compile with *"function resource_create(...) does not exist"*, the database is behind the code — run `sqlx migrate run`, not a code fix:

  ```bash
  export DATABASE_URL=postgresql://temper:temper@localhost:5437/temper_development
  sqlx migrate run
  ```

## Context an implementer needs

`openapi.json` is emitted by `crates/temper-api/src/bin/emit-openapi.rs`, which calls
`temper_api::openapi_spec()` (defined in `crates/temper-api/src/routes.rs`). That function seeds an
`OpenApiRouter` with `ApiDoc` (`crates/temper-api/src/openapi.rs`) for ambient metadata — info, tags,
security, `components(schemas(...))` — then collects paths from every `.routes(routes!(…))`
registration. There is deliberately no `paths(...)` list on `ApiDoc`.

Tests live in `crates/temper-api/src/openapi.rs` in `mod tests`, and drive
`crate::routes::openapi_spec()` rather than `ApiDoc::openapi()` — the latter has no paths.
`every_path_documents_the_surface_header` is the sibling test to model yours on.

**Why the regression gate is a Rust test and not a new shell script.** The invariant is a property of
the *spec*, and `check-openapi-spec.sh` already proves `committed == fresh`. Validating the fresh spec
therefore validates the committed artifact transitively, including against hand-edits. A third shell
script would duplicate the existing Rust test module for no added coverage.

## File Structure

| File | Responsibility | Change |
| --- | --- | --- |
| `crates/temper-api/src/openapi.rs` | Ambient OpenAPI metadata + spec-invariant tests | Modify: `use` list, `components(schemas(...))`, two new tests |
| `crates/temper-api/src/handlers/{contexts,resources,edges,teams,invitations,invocations,profiles,ingest,cognitive_maps,segments}.rs` | Route handlers + their `#[utoipa::path]` docs | Modify: add `operation_id` to 27 attributes |
| `openapi.json` | The emitted public contract | Regenerate |
| `tools/cargo-make/main.toml` | Task runner | Add `openapi-validate` task |
| `docs/superpowers/specs/2026-07-09-temper-rb-gem-design.md` | The spec | Modify: Part 1 gate description |

---

### Task 1: Register the two orphaned component schemas

Fixes the hard crash. `GET /api/resources` declares `sort` and `order` as `oneOf: [{type: null}, {$ref: …}]` pointing at `ResourceSortField` and `SortOrder`. Both enums derive `ToSchema` (`crates/temper-workflow/src/types/resource.rs:108,126`) but are reachable only through the `IntoParams` struct `ResourceListParams`, which `.routes()` does not walk. They are absent from `components.schemas`, so the reference dangles and the generator throws before emitting a file.

**P3 supplies the control case.** It added a `correlation_id` query parameter to
`DELETE /api/resources/{id}` and `PUT /api/cognitive-maps/{id}`, schema
`oneOf: [{type: null}, {$ref: ".../CorrelationId"}]` — structurally identical to the two broken
params. Yet `CorrelationId` resolves, and nobody hand-added it to `components(schemas(...))`. It
resolves because it *also* hangs off `ActInput`, which is a request-**body** schema, and `.routes()`
collects transitively from bodies. `ResourceSortField` and `SortOrder` hang off nothing but an
`IntoParams` struct. That is the precise rule: **a schema reachable only from `IntoParams` is never
collected.** Adding a query-only enum in future re-breaks the contract, which is why Step 1's test
exists.

**Files:**
- Modify: `crates/temper-api/src/openapi.rs` (the `use temper_workflow::types::resource::{…}` block at line 16; the `components(schemas(…))` list at line 28; `mod tests` at line 218)
- Regenerate: `openapi.json`

**Interfaces:**
- Consumes: nothing.
- Produces: the test helper `collect_schema_refs(&serde_json::Value, &mut BTreeSet<String>)`, private to `mod tests` in `openapi.rs`. Task 2 does not use it.

- [ ] **Step 1: Write the failing test**

Add to the bottom of `mod tests` in `crates/temper-api/src/openapi.rs` (inside the closing brace):

```rust
    /// Walk the serialized spec and collect every `#/components/schemas/<Name>` reference.
    fn collect_schema_refs(value: &serde_json::Value, out: &mut std::collections::BTreeSet<String>) {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(serde_json::Value::String(reference)) = map.get("$ref") {
                    if let Some(name) = reference.strip_prefix("#/components/schemas/") {
                        out.insert(name.to_owned());
                    }
                }
                for nested in map.values() {
                    collect_schema_refs(nested, out);
                }
            }
            serde_json::Value::Array(items) => {
                for nested in items {
                    collect_schema_refs(nested, out);
                }
            }
            _ => {}
        }
    }

    /// A `$ref` to a component that does not exist makes the document invalid OpenAPI.
    /// `openapi-generator`'s 3.1 dereferencer throws on it and emits zero files, so this is
    /// the difference between a generatable contract and an unusable one.
    ///
    /// Enums reachable only through an `IntoParams` query struct are NOT auto-collected by
    /// `.routes()` — they must be named in `components(schemas(...))` by hand.
    #[test]
    fn every_schema_ref_resolves() {
        use std::collections::BTreeSet;

        let spec = crate::routes::openapi_spec();
        let json = serde_json::to_value(&spec).expect("spec serializes to JSON");

        let defined: BTreeSet<String> = json["components"]["schemas"]
            .as_object()
            .expect("components.schemas is an object")
            .keys()
            .cloned()
            .collect();

        let mut referenced = BTreeSet::new();
        collect_schema_refs(&json, &mut referenced);

        let dangling: Vec<&String> = referenced.difference(&defined).collect();
        assert!(
            dangling.is_empty(),
            "spec $refs component schemas that are not defined: {dangling:?}",
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo nextest run -p temper-api --lib -E 'test(every_schema_ref_resolves)'
```

Expected: **FAIL**, with
`spec $refs component schemas that are not defined: ["ResourceSortField", "SortOrder"]`

- [ ] **Step 3: Register the two schemas**

In `crates/temper-api/src/openapi.rs`, extend the existing import (line 16) — add `ResourceSortField` and `SortOrder`, keeping alphabetical order:

```rust
use temper_workflow::types::resource::{
    ContentResponse, DeleteResponse, ResourceCreateRequest, ResourceDetail, ResourceFacets,
    ResourceListResponse, ResourceRow, ResourceSortField, ResourceUpdateRequest, SortOrder,
};
```

Then in `components(schemas(...))`, add the two entries immediately after `ResourceFacets,`:

```rust
        ResourceFacets,
        ResourceSortField,
        SortOrder,
        ResourceCreateRequest,
```

- [ ] **Step 4: Run the test to verify it passes**

```bash
cargo nextest run -p temper-api --lib -E 'test(every_schema_ref_resolves)'
```

Expected: **PASS**, `1 test run: 1 passed`.

- [ ] **Step 5: Regenerate the contract and confirm the two schemas landed**

```bash
cargo make openapi
python3 -c "
import json; d=json.load(open('openapi.json'))
s=d['components']['schemas']
print('schemas:', len(s))
print('ResourceSortField:', 'ResourceSortField' in s)
print('SortOrder:', 'SortOrder' in s)
"
```

Expected: `schemas: 155`, both `True`. (153 today — P3 added `CorrelationId` — plus these two.)

- [ ] **Step 6: Confirm the whole suite is still green**

```bash
cargo nextest run -p temper-api --lib
```

Expected: all tests pass, none fail. (Several `SKIP` lines for `test-db`-gated tests are normal.)

- [ ] **Step 7: Commit**

```bash
git add crates/temper-api/src/openapi.rs openapi.json
git commit -m "P5: register the two orphaned enum schemas that dangled a \$ref

ResourceSortField and SortOrder derive ToSchema and are \$ref'd from the
sort/order query params on GET /api/resources, but they are reachable only
through the IntoParams struct ResourceListParams, which .routes() does not
walk. They were never in the hand-maintained components(schemas(...)) list,
so the emitted contract referenced two components it never defined.

openapi-generator's 3.1 dereferencer throws on a dangling \$ref and emits
zero files. Guarded by a new test that walks the serialized spec and asserts
every \$ref resolves."
```

---

### Task 2: Give every operation a unique `operation_id`

OpenAPI requires `operationId` to be unique across the document. We never set one, so utoipa falls back to the handler's fn name: 79 operations collapse to 62 unique ids. Generation needs `--skip-validate-spec` (18 errors), and the one within-tag collision — `resources.rs::list` and `edges.rs::list`, both tagged `Resources` — makes the generator emit `ResourcesApi#list_0`.

Nothing in the codebase reads `operationId` (verified by grep across all non-generated files), so renaming is safe.

**Files:**
- Modify: `crates/temper-api/src/openapi.rs` (`mod tests`)
- Modify: 10 handler files (27 `#[utoipa::path]` attributes — full table below)
- Regenerate: `openapi.json`

**Interfaces:**
- Consumes: nothing from Task 1 (the test helper is not reused).
- Produces: 27 stable `operationId` strings that become the generated Ruby method names. Downstream (the gem build) depends on these exact names.

- [ ] **Step 1: Write the failing test**

Add to the bottom of `mod tests` in `crates/temper-api/src/openapi.rs`:

```rust
    /// Every HTTP method that OpenAPI allows on a path item.
    const HTTP_METHODS: [&str; 8] = [
        "get", "put", "post", "delete", "options", "head", "patch", "trace",
    ];

    /// OpenAPI requires `operationId` to be present and unique across the whole document.
    /// utoipa defaults it to the handler's fn name, so two handlers both named `list` collide —
    /// and a generator partitions methods by tag, so a within-tag collision silently emits
    /// `list_0`. Uniqueness is what makes the generated client's method names stable.
    #[test]
    fn operation_ids_are_present_and_unique() {
        use std::collections::BTreeMap;

        let spec = crate::routes::openapi_spec();
        let json = serde_json::to_value(&spec).expect("spec serializes to JSON");
        let paths = json["paths"].as_object().expect("paths is an object");

        let mut owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut missing: Vec<String> = Vec::new();

        for (path, item) in paths {
            for method in HTTP_METHODS {
                let Some(operation) = item.get(method) else {
                    continue;
                };
                let location = format!("{} {path}", method.to_uppercase());
                match operation.get("operationId").and_then(|id| id.as_str()) {
                    Some(id) => owners.entry(id.to_owned()).or_default().push(location),
                    None => missing.push(location),
                }
            }
        }

        assert!(missing.is_empty(), "operations without an operationId: {missing:?}");

        let duplicates: BTreeMap<&String, &Vec<String>> =
            owners.iter().filter(|(_, ops)| ops.len() > 1).collect();
        assert!(
            duplicates.is_empty(),
            "operationId must be unique across the document; duplicates: {duplicates:#?}",
        );
    }
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo nextest run -p temper-api --lib -E 'test(operation_ids_are_present_and_unique)'
```

Expected: **FAIL**, listing 7 duplicated ids (`create`, `delete`, `get`, `grant`, `list`, `revoke`, `update`) with the operations owning each.

- [ ] **Step 3: Add `operation_id` to all 27 handler attributes**

For each row, open the file, find the `#[utoipa::path(` attribute directly above `pub async fn <fn>`, and add an `operation_id = "…"` line immediately after the HTTP-method line. **Anchor on the function name — the line numbers below were captured before any edit and will shift.**

| File (`crates/temper-api/src/handlers/`) | fn | `operation_id` |
| --- | --- | --- |
| `contexts.rs` | `list` | `list_contexts` |
| `contexts.rs` | `create` | `create_context` |
| `contexts.rs` | `get` | `get_context` |
| `resources.rs` | `list` | `list_resources` |
| `resources.rs` | `get` | `get_resource` |
| `resources.rs` | `create` | `create_resource` |
| `resources.rs` | `update` | `update_resource` |
| `resources.rs` | `delete` | `delete_resource` |
| `resources.rs` | `grant` | `grant_resource_access` |
| `resources.rs` | `revoke` | `revoke_resource_access` |
| `edges.rs` | `list` | `list_resource_edges` |
| `teams.rs` | `list` | `list_teams` |
| `teams.rs` | `create` | `create_team` |
| `teams.rs` | `update` | `update_team` |
| `teams.rs` | `delete` | `delete_team` |
| `invitations.rs` | `create` | `create_team_invitation` |
| `invitations.rs` | `list` | `list_team_invitations` |
| `invocations.rs` | `list` | `list_invocations` |
| `profiles.rs` | `get` | `get_profile` |
| `profiles.rs` | `update` | `update_profile` |
| `ingest.rs` | `create` | `create_ingest` |
| `ingest.rs` | `update` | `update_ingest` |
| `cognitive_maps.rs` | `grant` | `grant_cogmap_access` |
| `cognitive_maps.rs` | `revoke` | `revoke_cogmap_access` |
| `segments.rs` | `append_block_handler` | `append_block` |
| `segments.rs` | `finalize_handler` | `finalize_resource` |
| `segments.rs` | `list_blocks_handler` | `list_blocks` |

The last three are not collisions — they are fn names whose `_handler` suffix leaks into the contract, which would produce `IngestApi#append_block_handler` in the generated client.

The edit shape, shown for three representative rows. `resources.rs`, `pub async fn list`:

```rust
#[utoipa::path(
    get,
    operation_id = "list_resources",
    path = "/api/resources",
    tag = "Resources",
    params(ResourceListParams),
```

`edges.rs`, `pub async fn list` — the within-tag collision:

```rust
#[utoipa::path(
    get,
    operation_id = "list_resource_edges",
    path = "/api/resources/{id}/edges",
    tag = "Resources",
    params(("id" = Uuid, Path, description = "Resource ID")),
```

`segments.rs`, `pub async fn append_block_handler`:

```rust
#[utoipa::path(
    post,
    operation_id = "append_block",
    path = "/api/resources/{id}/blocks",
```

- [ ] **Step 4: Run the test to verify it passes**

```bash
cargo nextest run -p temper-api --lib -E 'test(operation_ids_are_present_and_unique)'
```

Expected: **PASS**. If it still fails, the failure message names exactly which operation is missing an id or which id is duplicated — fix that row and re-run.

- [ ] **Step 5: Regenerate the contract and verify 79 unique ids**

```bash
cargo make openapi
python3 -c "
import json
from collections import Counter
d=json.load(open('openapi.json'))
ops=[op['operationId'] for p in d['paths'].values() for m,op in p.items()
     if m in ('get','post','put','patch','delete')]
c=Counter(ops)
print('operations:', len(ops), 'unique:', len(c))
print('duplicates:', {k:v for k,v in c.items() if v>1} or 'none')
print('any _handler suffix:', [o for o in ops if o.endswith('_handler')] or 'none')
"
```

Expected: `operations: 79 unique: 79`, `duplicates: none`, `any _handler suffix: none`.

- [ ] **Step 6: Confirm the whole crate is still green**

```bash
cargo nextest run -p temper-api --lib
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

Expected: all tests pass; clippy clean.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-api/src/handlers crates/temper-api/src/openapi.rs openapi.json
git commit -m "P5: give every operation a unique operation_id

OpenAPI requires operationId to be unique across the document. utoipa
defaults it to the handler's fn name, so 79 operations collapsed to 62
unique ids across 7 collisions. Generation required --skip-validate-spec,
and the one within-tag collision (resources::list vs edges::list, both
tagged Resources) made the generator emit ResourcesApi#list_0.

Also drops the _handler suffix from the three segmented-ingest operations,
which would otherwise surface as IngestApi#append_block_handler.

Nothing in the codebase reads operationId, so the rename is contained to
the contract and its downstream generated clients. Guarded by a new test."
```

---

### Task 3: Prove the contract generates, and make the proof re-runnable

The Rust tests assert the two invariants we know about. This task runs the real generator to catch anything we do not know about, and leaves a task behind so the check is one command.

The generator is **not** added to CI. It needs a ~1GB Docker image pull for the same signal the two Rust tests already give in 40ms; the acceptance run happens here, and `cargo make openapi-validate` makes it repeatable on demand.

**Files:**
- Modify: `tools/cargo-make/main.toml` (add `[tasks.openapi-validate]` after `[tasks.openapi-check]`, which ends at line 232)

**Interfaces:**
- Consumes: the repaired `openapi.json` from Tasks 1 and 2.
- Produces: `cargo make openapi-validate`.

- [ ] **Step 1: Add the cargo-make task**

In `tools/cargo-make/main.toml`, immediately after the `[tasks.openapi-check]` block:

```toml
[tasks.openapi-validate]
description = "Validate the committed openapi.json with openapi-generator (requires Docker)"
# Deliberately NOT a CI gate: this pulls a ~1GB image for the same signal that
# `openapi::tests::every_schema_ref_resolves` and
# `openapi::tests::operation_ids_are_present_and_unique` give in milliseconds.
# Run it when changing the shape of the contract, and before cutting a client SDK.
script = [
  "docker run --rm -v ${CARGO_MAKE_WORKING_DIRECTORY}:/local openapitools/openapi-generator-cli validate -i /local/openapi.json"
]
```

- [ ] **Step 2: Run it and read the output**

```bash
cargo make openapi-validate
```

Expected: `Validating spec (/local/openapi.json)` followed by `No validation issues detected.` and exit 0. Any warning or error printed here is a real defect — do not proceed past it.

- [ ] **Step 3: Prove a Ruby client actually generates, with no `--skip-validate-spec`**

```bash
OUT=$(mktemp -d)
docker run --rm -v "$PWD:/local" -v "$OUT:/out" openapitools/openapi-generator-cli \
  generate -i /local/openapi.json -g ruby --library=faraday -o /out \
  --additional-properties=gemName=temper_rb,moduleName=Temper
echo "--- generated files: $(find "$OUT" -name '*.rb' | wc -l | tr -d ' ') ---"
echo "--- any _0 suffixed method? ---"
grep -rn "def list_0" "$OUT/lib" || echo "none (correct)"
echo "--- the two formerly-dangling enums became models? ---"
ls "$OUT/lib/temper_rb/models/" | grep -iE 'sort_order|resource_sort_field' || echo "MISSING"
echo "--- resources api methods ---"
grep -oE "def [a-z_0-9]+" "$OUT/lib/temper_rb/api/resources_api.rb" | sort -u | head -20
rm -rf "$OUT"
```

Expected: a non-zero `.rb` count (~340), `none (correct)` for `list_0`, both enum models present, and `resources_api.rb` exposing `create_resource`, `delete_resource`, `get_resource`, `list_resources`, `list_resource_edges`, `update_resource`, `grant_resource_access`, `revoke_resource_access`, `get_content`, `provenance`.

If the generator exits non-zero, capture its stderr verbatim and stop — do not add `--skip-validate-spec` to make it pass. That flag is what this task exists to eliminate.

- [ ] **Step 4: Confirm the committed artifact still matches the router**

```bash
bash .github/scripts/check-openapi-spec.sh
```

Expected: `openapi.json is up to date with the router`.

- [ ] **Step 5: Commit**

```bash
git add tools/cargo-make/main.toml
git commit -m "P5: add cargo make openapi-validate

Runs openapi-generator's validator against the committed contract via
Docker. Deliberately not a CI gate — it pulls a ~1GB image for the same
signal the two new Rust tests give in milliseconds. Run it when changing
the shape of the contract, and before cutting a client SDK."
```

---

### Task 4: Reconcile the spec with what was built

The spec's Part 1 says the gate would extend `openapi-check` and run `openapi-generator validate` in CI. Implementation put the invariants in Rust tests (their natural home, beside `every_path_documents_the_surface_header`) and left the generator as an on-demand task. Record the decision and the reason, so a later reader does not think the plan was abandoned.

**Files:**
- Modify: `docs/superpowers/specs/2026-07-09-temper-rb-gem-design.md` (the `**Scope of P5:**` numbered list, item 4, and the `## Open threads` section)

**Interfaces:**
- Consumes: everything from Tasks 1–3.
- Produces: nothing consumed by later work.

- [ ] **Step 1: Rewrite item 4 of the P5 scope list**

Replace item 4 under `**Scope of P5:**` with:

```markdown
4. Guard both invariants with Rust unit tests in `crates/temper-api/src/openapi.rs`, beside the
   existing `every_path_documents_the_surface_header`: `every_schema_ref_resolves` walks the
   serialized spec and fails on a dangling `$ref`; `operation_ids_are_present_and_unique` fails on a
   missing or duplicated `operationId`. Together with the existing `check-openapi-spec.sh` — which
   proves `committed == fresh` — these validate the published artifact transitively, including
   against hand-edits. `openapi-generator`'s own validator is exposed as `cargo make openapi-validate`
   rather than wired into CI: it needs a ~1GB image pull for the same signal the tests give in
   milliseconds.
```

- [ ] **Step 2: Add the closed thread**

Append to `## Open threads`:

```markdown
- **The P5 gate landed as Rust tests, not a third shell script.** The spec originally proposed
  extending `openapi-check`. Implementation found the invariants are properties of the *spec*, and
  `check-openapi-spec.sh` already proves the committed artifact equals a fresh emission — so a Rust
  test on the fresh spec covers the artifact too. `openapi-generator validate` is
  `cargo make openapi-validate`, run on demand.
```

- [ ] **Step 3: Verify no other spec claim is now false**

```bash
rg -n "skip-validate-spec|list_0|openapi-check" docs/superpowers/specs/2026-07-09-temper-rb-gem-design.md
```

Read each hit. Every remaining mention must be describing the **old, broken** state (in "What discovery established", "Defect 1/2", or the Rejected list), not promising future behavior. Fix any that reads as a promise.

- [ ] **Step 4: Commit**

```bash
git add docs/superpowers/specs/2026-07-09-temper-rb-gem-design.md
git commit -m "P5: record that the gate landed as Rust tests, not a shell script

The invariants are properties of the spec, and check-openapi-spec.sh already
proves committed == fresh, so testing the fresh spec covers the artifact.
openapi-generator's validator is cargo make openapi-validate, on demand."
```

---

## Final verification

- [ ] **Full local gate**

```bash
cargo make check
cargo nextest run -p temper-api --lib
cargo make openapi-validate
bash .github/scripts/check-openapi-spec.sh
bash .github/scripts/check-openapi-routes.sh
```

All five must pass. `cargo make check` forces `SQLX_OFFLINE=true` and is the honest probe of the committed sqlx caches — no SQL changed here, so it should be unaffected.

- [ ] **Prove the new tests actually bite** (they are worthless if they pass vacuously)

Temporarily remove `SortOrder,` from `components(schemas(...))`, run
`cargo nextest run -p temper-api --lib -E 'test(every_schema_ref_resolves)'`, and confirm it **FAILS**
naming `SortOrder`. Restore it.

Temporarily delete `operation_id = "list_resource_edges",` from `edges.rs`, run
`cargo nextest run -p temper-api --lib -E 'test(operation_ids_are_present_and_unique)'`, and confirm it
**FAILS** naming `list` as duplicated across `GET /api/resources` and `GET /api/resources/{id}/edges`.
Restore it.

Then re-run `cargo nextest run -p temper-api --lib` and confirm green.

- [ ] **Push and open the PR**

The branch `jct/p5-contract-generates` already carries the design spec commit and a merge of
`origin/main` through **P3 (#345)**, so `ActInput` already has `correlation` and `CorrelationId` is
already a registered component. Re-merge before pushing anyway — CI builds `pull/<n>/merge`.

```bash
git fetch origin && git merge origin/main
cargo make openapi && git diff --stat openapi.json   # expect no diff unless main moved again
git push -u origin jct/p5-contract-generates
```

PR body must state: 79 operations, 79 unique operationIds, 155 component schemas, zero dangling
`$ref`s, `openapi-generator validate` clean, and a Ruby client generating with no
`--skip-validate-spec`. Note the contract-visible change — 27 `operationId`s are renamed — and that
nothing in the codebase consumes them.
