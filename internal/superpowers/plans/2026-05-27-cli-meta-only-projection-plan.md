# CLI meta-only projection — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `--meta-only` and `--fields` flags to `temper resource show` and `list`, plus a `fields` parameter on MCP `get_resource` and `list_resources`. Output stays in json|toon (API passthrough); top-level key projection happens client-side via a shared `temper-core::projection` module.

**Architecture:** Server adds `meta_only=true` query param on `GET /api/resources` returning a new `ResourceMetaListResponse` (mirror of `ResourceListResponse` with `Vec<ResourceMetaResponse>` rows; facets reused). Single-resource path uses the existing `GET /api/resources/{id}/meta` endpoint. The projection filter is a tiny serde_json::Value transformation owned by temper-core and shared by both CLI action layer and MCP tool handlers. Anchor key (`id` or `resource_id`) is always preserved.

**Tech Stack:** Rust workspace, axum (API), clap (CLI), rmcp (MCP), schemars/utoipa/ts-rs derives on shared types, sqlx with `--features test-db`, cargo-nextest, e2e tests in `tests/e2e/`.

**Spec:** `docs/superpowers/specs/2026-05-27-cli-meta-only-projection-design.md`

---

## File Structure

**New files:**
- `crates/temper-core/src/projection.rs` — shared top-level key filter
- `tests/e2e/tests/cli_meta_projection_test.rs` — end-to-end CLI driver tests

**Touched files:**
- `crates/temper-core/src/lib.rs` — register `projection` module
- `crates/temper-core/src/types/managed_meta.rs` — add `ResourceMetaListResponse`
- `crates/temper-core/src/types/resource.rs` — add `meta_only` to `ResourceListParams`
- `crates/temper-api/src/services/resource_service.rs` — add `list_visible_meta`
- `crates/temper-api/src/handlers/resources.rs` — dispatch `list` on `meta_only`
- `crates/temper-api/src/openapi.rs` — register new type + `oneOf` response
- `crates/temper-client/src/resources.rs` — add `list_meta` method
- `crates/temper-cli/src/cli.rs` — add `meta_only` + `fields` to `ResourceAction::Show` and `List`
- `crates/temper-cli/src/commands/resource.rs` — branch on `meta_only`; apply projection filter
- `crates/temper-mcp/src/tools/resources.rs` — add `fields` to `GetResourceInput` + `ListResourcesInput`; apply filter in handlers

**Pattern reminder:** Every task ends with a single commit. The pre-commit hook runs fmt + clippy + docs + machete + TS typecheck + biome over the whole workspace; commits validate workspace state.

---

## Task 1: Shared projection module (`temper-core::projection`)

The filter that both CLI and MCP will share. Pure module, no async, no I/O — easiest to TDD in isolation.

**Files:**
- Create: `crates/temper-core/src/projection.rs`
- Modify: `crates/temper-core/src/lib.rs` (add `pub mod projection;`)

- [ ] **Step 1: Write the failing tests**

Create `crates/temper-core/src/projection.rs`:

```rust
//! Top-level key projection over JSON values.
//!
//! Used by the CLI action layer (`--fields`) and MCP tool handlers
//! (`fields` parameter) to subselect top-level keys from an API
//! response while always preserving a designated anchor key
//! (e.g. `id` or `resource_id`).
//!
//! Nested-path projection is intentionally rejected — the boundary is
//! "we do not own a query language." Callers needing nested projection
//! pipe the unfiltered output to `jq`.

use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProjectionError {
    /// A field name contained `.`, indicating nested-path projection
    /// which is intentionally unsupported. The `hint` carries the
    /// `jq` invocation the caller should run instead.
    #[error("--fields supports top-level keys only; use jq for nested projection: {hint}")]
    DottedPath { hint: String },
    /// A field name was empty or whitespace-only.
    #[error("empty field name in --fields")]
    EmptyField,
}

/// Filter the top-level keys of a JSON value.
///
/// - If `fields` is empty, the value is returned unchanged.
/// - For an object: returns a new object containing `anchor` (always,
///   when present in the input) plus any `fields` entries that exist
///   as top-level keys. Unknown keys are silently dropped.
/// - For an array of objects: applies the filter to each element.
/// - For other shapes (scalars, mixed arrays): returns the input
///   unchanged.
///
/// Validates all field names before applying. Returns `DottedPath` if
/// any contains `.`, or `EmptyField` if any is empty/whitespace.
pub fn apply_top_level_filter(
    value: Value,
    fields: &[String],
    anchor: &str,
) -> Result<Value, ProjectionError> {
    if fields.is_empty() {
        return Ok(value);
    }

    for raw in fields {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(ProjectionError::EmptyField);
        }
        if trimmed.contains('.') {
            return Err(ProjectionError::DottedPath {
                hint: format!("pipe the unfiltered output to `jq '.{trimmed}'`"),
            });
        }
    }

    match value {
        Value::Object(map) => Ok(Value::Object(filter_object(map, fields, anchor))),
        Value::Array(items) => {
            let filtered: Vec<Value> = items
                .into_iter()
                .map(|item| match item {
                    Value::Object(m) => Value::Object(filter_object(m, fields, anchor)),
                    other => other,
                })
                .collect();
            Ok(Value::Array(filtered))
        }
        other => Ok(other),
    }
}

fn filter_object(
    mut map: Map<String, Value>,
    fields: &[String],
    anchor: &str,
) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(v) = map.remove(anchor) {
        out.insert(anchor.to_string(), v);
    }
    for raw in fields {
        let f = raw.trim();
        if f == anchor {
            continue;
        }
        if let Some(v) = map.remove(f) {
            out.insert(f.to_string(), v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_fields_returns_input_unchanged() {
        let input = json!({"id": "x", "managed_meta": {"stage": "in-progress"}});
        let out = apply_top_level_filter(input.clone(), &[], "id").unwrap();
        assert_eq!(out, input);
    }

    #[test]
    fn object_preserves_anchor_plus_requested_keys() {
        let input = json!({
            "id": "abc",
            "managed_meta": {"stage": "done"},
            "open_meta": {"tags": ["x"]},
            "managed_hash": "sha256:..."
        });
        let out = apply_top_level_filter(
            input,
            &["managed_meta".to_string()],
            "id",
        )
        .unwrap();
        assert_eq!(out, json!({"id": "abc", "managed_meta": {"stage": "done"}}));
    }

    #[test]
    fn object_drops_unknown_keys_silently() {
        let input = json!({"id": "abc", "managed_meta": {}});
        let out = apply_top_level_filter(
            input,
            &["managed_meta".to_string(), "nonexistent".to_string()],
            "id",
        )
        .unwrap();
        assert_eq!(out, json!({"id": "abc", "managed_meta": {}}));
    }

    #[test]
    fn explicit_anchor_in_fields_is_harmless() {
        let input = json!({"id": "abc", "managed_meta": {}});
        let out = apply_top_level_filter(
            input,
            &["id".to_string(), "managed_meta".to_string()],
            "id",
        )
        .unwrap();
        assert_eq!(out, json!({"id": "abc", "managed_meta": {}}));
    }

    #[test]
    fn array_of_objects_filters_each_element() {
        let input = json!([
            {"id": "a", "managed_meta": {}, "open_meta": null},
            {"id": "b", "managed_meta": {}, "open_meta": null}
        ]);
        let out = apply_top_level_filter(
            input,
            &["managed_meta".to_string()],
            "id",
        )
        .unwrap();
        assert_eq!(
            out,
            json!([
                {"id": "a", "managed_meta": {}},
                {"id": "b", "managed_meta": {}}
            ])
        );
    }

    #[test]
    fn dotted_path_returns_error_with_jq_hint() {
        let input = json!({"id": "x"});
        let err = apply_top_level_filter(
            input,
            &["managed_meta.stage".to_string()],
            "id",
        )
        .unwrap_err();
        match err {
            ProjectionError::DottedPath { hint } => {
                assert!(hint.contains("jq"), "hint must mention jq: {hint}");
                assert!(
                    hint.contains("managed_meta.stage"),
                    "hint must echo the path: {hint}"
                );
            }
            other => panic!("expected DottedPath, got {other:?}"),
        }
    }

    #[test]
    fn empty_field_name_returns_error() {
        let input = json!({"id": "x"});
        let err = apply_top_level_filter(input.clone(), &["".to_string()], "id").unwrap_err();
        assert_eq!(err, ProjectionError::EmptyField);

        let err = apply_top_level_filter(input, &["   ".to_string()], "id").unwrap_err();
        assert_eq!(err, ProjectionError::EmptyField);
    }

    #[test]
    fn scalar_value_returns_input_unchanged() {
        let input = json!("hello");
        let out = apply_top_level_filter(
            input.clone(),
            &["anything".to_string()],
            "id",
        )
        .unwrap();
        assert_eq!(out, input);
    }
}
```

Modify `crates/temper-core/src/lib.rs` — add the module registration. Read the file first to find where existing `pub mod` declarations live, then add `pub mod projection;` alphabetically near the others.

- [ ] **Step 2: Verify tests fail (module not yet wired)**

```bash
cargo nextest run -p temper-core projection
```

Expected: compile error if `pub mod projection;` was not added to `lib.rs`. Add it. Re-run.

Expected after `lib.rs` fix: all 8 tests PASS (the implementation in the file is complete).

- [ ] **Step 3: Verify the rest of the workspace still compiles**

```bash
cargo make check
```

Expected: ✓ all green.

- [ ] **Step 4: Commit**

```bash
git add crates/temper-core/src/projection.rs crates/temper-core/src/lib.rs
git commit -m "$(cat <<'EOF'
feat(core): add shared projection module for top-level key filter

Adds temper_core::projection::apply_top_level_filter — the shared
helper used by both the CLI action layer (--fields) and the MCP
tool handlers (fields parameter) to subselect top-level keys from
an API response while preserving a designated anchor key.

Nested-path projection rejected with a typed ProjectionError that
points the caller at jq. This boundary is load-bearing on the
"we do not own a query language" rejection in the spec.

Spec: docs/superpowers/specs/2026-05-27-cli-meta-only-projection-design.md
EOF
)"
```

---

## Task 2: `ResourceMetaListResponse` core type

Mirror of `ResourceListResponse` with the rows type swapped. Same `total` + `facets` envelope.

**Files:**
- Modify: `crates/temper-core/src/types/managed_meta.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-core/src/types/managed_meta.rs` (inside `#[cfg(test)] mod tests` — create the module if absent):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::resource::ResourceFacets;
    use std::collections::HashMap;

    #[test]
    fn resource_meta_list_response_roundtrip() {
        let response = ResourceMetaListResponse {
            rows: vec![],
            total: 0,
            facets: ResourceFacets {
                doc_type: HashMap::new(),
            },
        };
        let json = serde_json::to_value(&response).expect("serialize");
        let back: ResourceMetaListResponse =
            serde_json::from_value(json.clone()).expect("deserialize");
        assert_eq!(back.total, 0);
        assert!(back.rows.is_empty());
        assert_eq!(json["rows"], serde_json::json!([]));
        assert_eq!(json["total"], 0);
        assert!(json["facets"].is_object());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo nextest run -p temper-core resource_meta_list_response_roundtrip
```

Expected: compile error — `ResourceMetaListResponse` not defined.

- [ ] **Step 3: Add the type**

Add to `crates/temper-core/src/types/managed_meta.rs` (immediately after the `ResourceMetaResponse` definition):

```rust
/// Paginated meta-only response for resource list endpoints.
///
/// Mirror of [`crate::types::resource::ResourceListResponse`] with the
/// row type swapped to [`ResourceMetaResponse`]. Returned by
/// `GET /api/resources?meta_only=true`. Facets and total are computed
/// identically to the default list response — projection-independent.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS))]
#[cfg_attr(feature = "typescript", ts(export, export_to = "managed_meta.ts"))]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "mcp", derive(schemars::JsonSchema))]
pub struct ResourceMetaListResponse {
    pub rows: Vec<ResourceMetaResponse>,
    pub total: i64,
    pub facets: crate::types::resource::ResourceFacets,
}
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo nextest run -p temper-core resource_meta_list_response_roundtrip
```

Expected: PASS.

- [ ] **Step 5: Regenerate TypeScript types (ts-rs derive shipping)**

```bash
cargo make generate-ts-types
```

Expected: `packages/temper-ui/src/lib/types/managed_meta.ts` updated to include `ResourceMetaListResponse`.

- [ ] **Step 6: Commit**

```bash
git add crates/temper-core/src/types/managed_meta.rs packages/temper-ui/src/lib/types/
git commit -m "$(cat <<'EOF'
feat(core): add ResourceMetaListResponse type

Paginated meta-only response shape returned by
GET /api/resources?meta_only=true. Mirrors ResourceListResponse with
Vec<ResourceMetaResponse> rows; facets and total computed identically
in the new service path.

Includes ts-rs / utoipa / schemars derives matching the existing
managed_meta types.
EOF
)"
```

---

## Task 3: `list_visible_meta` service function (temper-api)

Same SQL filter shape as `list_visible`, but joins with `meta_service::get_meta_batch` to produce `Vec<ResourceMetaResponse>`. Facets and total queries unchanged.

**Files:**
- Modify: `crates/temper-api/src/services/resource_service.rs`
- Modify: `crates/temper-core/src/types/resource.rs` (add `meta_only` to `ResourceListParams`)

- [ ] **Step 1: Add `meta_only` query param to `ResourceListParams`**

Read `crates/temper-core/src/types/resource.rs` and locate `pub struct ResourceListParams`. Add a new field at the end (before any trailing `}`):

```rust
    /// When true, the list endpoint returns ResourceMetaListResponse
    /// (Vec<ResourceMetaResponse> rows) instead of ResourceListResponse
    /// (Vec<ResourceRow> rows). Default: false.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta_only: Option<bool>,
```

- [ ] **Step 2: Write the failing test (signature-level)**

Existing tests inside `temper-api/src/services/resource_service.rs` (the `#[cfg(test)] mod tests` block at the bottom of the file) are pure helper-function unit tests with no DB. The crate's pattern is: DB-dependent tests live in `tests/e2e/`, not inline. Match that pattern.

Append to the existing `#[cfg(test)] mod tests` block (after `meta_update_delta_serializes_to_changed_key_arrays`):

```rust
    /// Signature-level guard: confirms `list_visible_meta` exists with the
    /// expected types. Full integration coverage lives in
    /// `tests/e2e/tests/cli_meta_projection_test.rs`.
    #[test]
    fn list_visible_meta_has_expected_signature() {
        fn _assert_callable() {
            let _: fn(
                &sqlx::PgPool,
                uuid::Uuid,
                temper_core::types::resource::ResourceListParams,
            ) -> std::pin::Pin<
                Box<
                    dyn std::future::Future<
                            Output = crate::error::ApiResult<
                                temper_core::types::managed_meta::ResourceMetaListResponse,
                            >,
                        > + Send,
                >,
            > = |pool, pid, params| Box::pin(list_visible_meta(pool, pid, params));
        }
    }
```

- [ ] **Step 3: Run test to verify it fails**

```bash
cargo nextest run -p temper-api list_visible_meta_has_expected_signature
```

Expected: compile error — `list_visible_meta` not defined.

- [ ] **Step 4: Implement `list_visible_meta`**

Add to `crates/temper-api/src/services/resource_service.rs`, immediately after `list_visible`:

```rust
/// Variant of [`list_visible`] that returns each resource's meta
/// projection instead of the row scalars. Same filters, same facets,
/// same pagination; only the row type differs.
///
/// Reuses the same SQL filter pipeline and facets query; joins with
/// `meta_service::get_meta_batch` to produce `Vec<ResourceMetaResponse>`.
pub async fn list_visible_meta(
    pool: &PgPool,
    profile_id: Uuid,
    params: ResourceListParams,
) -> ApiResult<temper_core::types::managed_meta::ResourceMetaListResponse> {
    use temper_core::types::managed_meta::ResourceMetaListResponse;

    // Run the existing list query first; we reuse its rows, total, facets.
    let list_response = list_visible(pool, profile_id, params).await?;

    // Collect resource IDs for the batch meta fetch.
    let ids: Vec<temper_core::types::ResourceId> =
        list_response.rows.iter().map(|r| r.id).collect();

    if ids.is_empty() {
        return Ok(ResourceMetaListResponse {
            rows: vec![],
            total: list_response.total,
            facets: list_response.facets,
        });
    }

    let mut meta_map = crate::services::meta_service::get_meta_batch(pool, &ids).await?;

    // Preserve the row order from the list query (sort fidelity).
    let meta_rows: Vec<_> = list_response
        .rows
        .iter()
        .filter_map(|row| meta_map.remove(&row.id))
        .collect();

    Ok(ResourceMetaListResponse {
        rows: meta_rows,
        total: list_response.total,
        facets: list_response.facets,
    })
}
```

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo nextest run -p temper-api list_visible_meta_has_expected_signature
```

Expected: PASS. Full behavior is covered by Task 11's e2e tests.

- [ ] **Step 6: Verify the full workspace still compiles**

```bash
cargo make check
```

Expected: ✓ all green.

- [ ] **Step 7: Regenerate sqlx cache and TS types**

```bash
cargo sqlx prepare --workspace -- --all-features
cargo make generate-ts-types
```

Expected: `.sqlx/` cache and TS types updated (or unchanged if no new queries were added — `list_visible_meta` reuses existing queries).

- [ ] **Step 8: Commit**

```bash
git add crates/temper-api/src/services/resource_service.rs \
        crates/temper-core/src/types/resource.rs \
        packages/temper-ui/src/lib/types/ \
        .sqlx/
git commit -m "$(cat <<'EOF'
feat(api): add list_visible_meta service function

Variant of list_visible that returns Vec<ResourceMetaResponse> rows
via meta_service::get_meta_batch. Same filter pipeline, same facets,
same pagination; only the row type differs.

ResourceListParams grows meta_only: Option<bool> so the handler can
dispatch the new variant from a single query param.
EOF
)"
```

---

## Task 4: API handler dispatch on `meta_only`

Wrap the response shape variation in an enum that implements `IntoResponse` and `ToSchema`, so utoipa produces `oneOf<ResourceListResponse, ResourceMetaListResponse>`.

**Files:**
- Modify: `crates/temper-api/src/handlers/resources.rs`
- Modify: `crates/temper-api/src/openapi.rs`

- [ ] **Step 1: Read the existing handler to understand patterns**

```bash
sed -n '1,60p' crates/temper-api/src/handlers/resources.rs
```

Note the current `list` signature, its imports, and how `ApiResult<Json<T>>` is returned.

- [ ] **Step 2: Plan the test coverage**

The temper-api crate does not run handler-level integration tests inline — those live in `tests/e2e/`. Task 11 already covers the wire-shape dispatch via real CLI binary against a real Axum + DB.

For this task, the test is the compile-time guarantee: after Steps 3-5, `cargo make check` must still pass and the new `ListResourcesResponse` enum must derive correctly. No new inline test is added for this task.

If you want a stronger local check, run the matching e2e test after this task lands:

```bash
cargo nextest run -p temper-e2e --features test-db list_meta_only_returns_meta_list_response_shape
```

That test belongs to Task 11 — running it now will fail because the CLI flag wiring isn't in place yet. It will pass once Task 11 lands.

- [ ] **Step 3: Skip (no failing test in this task; proceeds via compile-time guarantees)**

Proceed to Step 4.

- [ ] **Step 4: Implement the dispatch**

Modify `crates/temper-api/src/handlers/resources.rs`. Add the response enum near the top of the file (after imports):

```rust
use temper_core::types::managed_meta::ResourceMetaListResponse;

/// Combined response for `GET /api/resources`.
///
/// Returned shape depends on the `meta_only` query parameter. utoipa
/// represents this as `oneOf<ResourceListResponse, ResourceMetaListResponse>`.
#[derive(serde::Serialize)]
#[serde(untagged)]
#[cfg_attr(feature = "web-api", derive(utoipa::ToSchema))]
pub enum ListResourcesResponse {
    Default(temper_core::types::resource::ResourceListResponse),
    Meta(ResourceMetaListResponse),
}

impl axum::response::IntoResponse for ListResourcesResponse {
    fn into_response(self) -> axum::response::Response {
        match self {
            Self::Default(r) => axum::Json(r).into_response(),
            Self::Meta(r) => axum::Json(r).into_response(),
        }
    }
}
```

Update the existing `list` handler to dispatch:

```rust
pub async fn list(
    State(state): State<crate::state::ApiState>,
    auth: crate::middleware::AuthedRequest,
    Query(params): Query<temper_core::types::resource::ResourceListParams>,
) -> crate::error::ApiResult<ListResourcesResponse> {
    if params.meta_only.unwrap_or(false) {
        let response = crate::services::resource_service::list_visible_meta(
            &state.pool,
            auth.0.profile.id,
            params,
        )
        .await?;
        Ok(ListResourcesResponse::Meta(response))
    } else {
        let response = crate::services::resource_service::list_visible(
            &state.pool,
            auth.0.profile.id,
            params,
        )
        .await?;
        Ok(ListResourcesResponse::Default(response))
    }
}
```

Match the existing imports for `State`, `Query`, `AuthedRequest`, etc. — read the file's current `use` block.

- [ ] **Step 5: Update OpenAPI registration**

Modify `crates/temper-api/src/openapi.rs`. Find the existing list endpoint registration and update its response type. Add `ResourceMetaListResponse` and `ListResourcesResponse` to the components list. Match the existing `#[utoipa::path]` annotation on the `list` handler — change response body to `ListResourcesResponse`.

Read first:
```bash
grep -n "list\|ResourceListResponse\|ResourceMetaResponse" crates/temper-api/src/openapi.rs
```

Adjust the annotations on the `list` handler in `handlers/resources.rs` to declare the new response body, and register the new types in `openapi.rs`'s schema list.

- [ ] **Step 6: Verify compile + format**

```bash
cargo make check
```

Expected: ✓ all green. The enum, `IntoResponse`, and OpenAPI registration must compile cleanly with utoipa's macros.

- [ ] **Step 7: Verify the workspace compiles + regenerate**

```bash
cargo make check && cargo sqlx prepare --workspace -- --all-features
```

Expected: ✓ all green.

- [ ] **Step 8: Commit**

```bash
git add crates/temper-api/src/handlers/ crates/temper-api/src/openapi.rs .sqlx/
git commit -m "$(cat <<'EOF'
feat(api): dispatch GET /api/resources on meta_only query param

ListResourcesResponse is an untagged serde enum + IntoResponse over
the two possible shapes. utoipa represents this as oneOf for OpenAPI
clients. The default arm (no meta_only) is unchanged; meta_only=true
calls list_visible_meta.

Closes the API half of the meta-only projection spec.
EOF
)"
```

---

## Task 5: `list_meta` method on temper-client

Sibling of `list` — same params, same path, just appends `meta_only=true` to the query string and returns the meta shape.

**Files:**
- Modify: `crates/temper-client/src/resources.rs`

- [ ] **Step 1: Read existing `list` for the pattern**

```bash
sed -n '20,50p' crates/temper-client/src/resources.rs
```

Note how `list` constructs the URL, encodes `ResourceListParams`, deserializes the response.

- [ ] **Step 2: Write the failing test (compile-only check via type signature)**

Most client method tests live in tests/e2e (which we'll cover in Task 11). For unit-level coverage here, add a compile-time signature assertion via a doctest or skip and rely on e2e. Add this small unit test to confirm the type signature:

```rust
#[cfg(test)]
mod meta_list_tests {
    use super::*;

    #[test]
    fn list_meta_signature_check() {
        // Pure type-level check: this should compile if and only if
        // `list_meta` exists on Resources with the right signature.
        fn _assert_callable() {
            let _: fn(
                &Resources,
                &temper_core::types::resource::ResourceListParams,
            )
                -> std::pin::Pin<
                    Box<dyn std::future::Future<
                        Output = crate::Result<
                            temper_core::types::managed_meta::ResourceMetaListResponse,
                        >,
                    > + Send>,
                > = |client, params| Box::pin(client.list_meta(params));
        }
    }
}
```

This is unusual but cheap: the only purpose is to fail at compile time if the method signature drifts.

- [ ] **Step 3: Run test to verify it fails**

```bash
cargo nextest run -p temper-client list_meta_signature_check
```

Expected: compile error — `list_meta` not defined.

- [ ] **Step 4: Implement `list_meta`**

The existing `list` method (line 32 of `crates/temper-client/src/resources.rs`) uses the crate's `HttpClient` helper pattern: `self.http.get(path).query(params)` + `self.http.send_json(...)`. Match it exactly. Also add the `ResourceMetaListResponse` type to the imports.

Update imports at the top of the file:

```rust
use temper_core::types::managed_meta::{
    MetaUpdatePayload, ResourceMetaListResponse, ResourceMetaResponse,
};
```

Add this method immediately after `pub async fn list` (line 32-38):

```rust
    /// List visible resources with meta projection (Vec<ResourceMetaResponse>
    /// rows). Sibling of [`ResourceClient::list`]; forces
    /// `meta_only=true` on the wire.
    pub async fn list_meta(
        &self,
        params: &ResourceListParams,
    ) -> Result<ResourceMetaListResponse> {
        let mut params = params.clone();
        params.meta_only = Some(true);
        let token = self.http.resolve_token()?;
        let req = self.http.get("/api/resources").query(&params);
        self.http
            .send_json(&Method::GET, "/api/resources", req, Some(&token))
            .await
    }
```

Key invariants enforced here:
- `meta_only = Some(true)` is set on a clone so the caller's params struct is untouched.
- The path is `/api/resources` (same as default `list`); the dispatch happens server-side on the `meta_only` query string.
- `send_json` deserializes the response into the inferred return type.

- [ ] **Step 5: Run test to verify it passes**

```bash
cargo nextest run -p temper-client list_meta_signature_check
```

Expected: PASS.

- [ ] **Step 6: Workspace check**

```bash
cargo make check
```

Expected: ✓ all green.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-client/src/resources.rs
git commit -m "$(cat <<'EOF'
feat(client): add list_meta method for meta-only list responses

Sibling of list() — forces meta_only=true, returns
ResourceMetaListResponse. Used by the CLI's `list --meta-only`
dispatch (next task).
EOF
)"
```

---

## Task 6: CLI clap args — `--meta-only` and `--fields`

Add the two new flags to `ResourceAction::Show` and `ResourceAction::List`. `--meta-only` conflicts with `--edges` on Show.

**Files:**
- Modify: `crates/temper-cli/src/cli.rs`

- [ ] **Step 1: Write the failing test**

Add to `crates/temper-cli/src/cli.rs` (in the existing `#[cfg(test)] mod tests` block, or create one at file end):

```rust
#[cfg(test)]
mod meta_only_flag_tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn show_accepts_meta_only_and_fields() {
        let cmd = Cli::command();
        let m = cmd.try_get_matches_from([
            "temper", "resource", "show",
            "my-slug",
            "--type", "task",
            "--context", "temper",
            "--meta-only",
            "--fields", "managed_meta,open_meta",
        ]);
        assert!(m.is_ok(), "show with --meta-only and --fields failed to parse: {:?}", m.err());
    }

    #[test]
    fn show_meta_only_conflicts_with_edges() {
        let cmd = Cli::command();
        let m = cmd.try_get_matches_from([
            "temper", "resource", "show",
            "my-slug",
            "--type", "task",
            "--meta-only",
            "--edges",
        ]);
        assert!(m.is_err(), "--meta-only and --edges must conflict");
    }

    #[test]
    fn list_accepts_meta_only_and_fields() {
        let cmd = Cli::command();
        let m = cmd.try_get_matches_from([
            "temper", "resource", "list",
            "--type", "task",
            "--meta-only",
            "--fields", "managed_meta",
        ]);
        assert!(m.is_ok(), "list with --meta-only and --fields failed: {:?}", m.err());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo nextest run -p temper-cli meta_only_flag
```

Expected: compile error — unknown flags.

- [ ] **Step 3: Add the clap args**

Modify `crates/temper-cli/src/cli.rs`. Find `ResourceAction::Show` and add inside its struct variant:

```rust
        /// Return only the resource's meta tier (managed + open
        /// frontmatter, hashes); no body. Calls GET /meta endpoint.
        #[arg(long, conflicts_with = "edges")]
        meta_only: bool,
        /// Subselect top-level response keys (resource_id always
        /// preserved). Use jq for nested projection.
        #[arg(long, value_delimiter = ',')]
        fields: Vec<String>,
```

Find `ResourceAction::List` and add to its struct variant:

```rust
        /// Return Vec<ResourceMetaResponse> rows instead of
        /// Vec<ResourceRow> rows. Hits GET /api/resources?meta_only=true.
        #[arg(long)]
        meta_only: bool,
        /// Subselect top-level response keys on each row (anchor key
        /// always preserved). Use jq for nested projection.
        #[arg(long, value_delimiter = ',')]
        fields: Vec<String>,
```

- [ ] **Step 4: Run test to verify it passes**

```bash
cargo nextest run -p temper-cli meta_only_flag
```

Expected: All 3 PASS.

- [ ] **Step 5: Workspace check**

```bash
cargo make check
```

Expected: workspace will fail to compile if the handler functions in `commands/resource.rs` need to be updated to accept the new struct fields. Update the function signatures in `commands/resource.rs` to accept (but ignore) the new args for now — Tasks 7 and 8 will wire them.

Pragmatic temporary patch: in the `Show` and `List` action dispatch points in `main.rs` (or wherever `match action {...}` lives), accept the new fields with `_meta_only` and `_fields` (underscore-prefixed) until Tasks 7/8 use them.

- [ ] **Step 6: Re-run workspace check**

```bash
cargo make check
```

Expected: ✓ all green.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/
git commit -m "$(cat <<'EOF'
feat(cli): add --meta-only and --fields clap args on resource show/list

Wiring of the action layer to actually consume these flags lands in
the next two commits. This commit just teaches clap to parse them,
including the --meta-only conflicts_with --edges constraint on show.
EOF
)"
```

---

## Task 7: Wire `temper resource show --meta-only` action

Branch on `meta_only`: call `client.resources.get_meta(id)` instead of the default show path. Apply the projection filter (if `fields` non-empty) before rendering.

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-cli/src/commands/resource.rs` test module:

```rust
#[cfg(test)]
mod show_meta_only_tests {
    use super::*;
    use temper_core::projection::apply_top_level_filter;
    use temper_core::types::managed_meta::{ManagedMeta, ResourceMetaResponse};

    fn fake_meta_response() -> ResourceMetaResponse {
        ResourceMetaResponse {
            resource_id: temper_core::types::ResourceId::new(uuid::Uuid::new_v4()),
            managed_meta: Some(ManagedMeta {
                title: Some("test".to_string()),
                ..Default::default()
            }),
            open_meta: Some(serde_json::json!({"tags": ["x"]})),
            managed_hash: "sha256:test".to_string(),
            open_hash: "sha256:test".to_string(),
        }
    }

    #[test]
    fn show_meta_only_fields_filter_preserves_anchor_and_managed_meta_only() {
        let response = fake_meta_response();
        let value = serde_json::to_value(&response).expect("serialize");
        let filtered = apply_top_level_filter(
            value,
            &["managed_meta".to_string()],
            "resource_id",
        )
        .expect("filter");
        assert!(filtered.get("resource_id").is_some(), "anchor missing");
        assert!(filtered.get("managed_meta").is_some(), "managed_meta missing");
        assert!(filtered.get("open_meta").is_none(), "open_meta should be filtered out");
        assert!(filtered.get("managed_hash").is_none(), "managed_hash should be filtered out");
    }

    #[test]
    fn show_meta_only_no_fields_returns_full_response() {
        let response = fake_meta_response();
        let value = serde_json::to_value(&response).expect("serialize");
        let unfiltered = apply_top_level_filter(value.clone(), &[], "resource_id")
            .expect("filter");
        assert_eq!(unfiltered, value);
    }
}
```

These tests exercise the projection filter behavior at the response shape level — they are the contract that the `show` action's filter step must preserve.

- [ ] **Step 2: Run test to verify it passes (the filter already works)**

```bash
cargo nextest run -p temper-cli show_meta_only
```

Expected: both tests PASS — the projection filter is already implemented; this just smoke-tests its application to the response shape.

- [ ] **Step 3: Wire the action layer**

Read `show_generic` in `crates/temper-cli/src/commands/resource.rs` (around lines 444-505) — that is the pattern to mirror. Note the key idioms:
- `runtime::with_client(|client| Box::pin(async move { ... }))` — closure receives `&TemperClient` and returns a pinned boxed future.
- `client.resources()` is an accessor method, not a field.
- `config.owner_for_context(&ctx)` resolves the owner handle.
- `*row.id.as_uuid()` converts a `ResourceId` to the bare `Uuid` that `get_meta` takes.
- `crate::actions::runtime::client_err_to_temper` maps client errors.

Update the existing `pub fn show` at `crates/temper-cli/src/commands/resource.rs:415-437`. The current shape is:

```rust
pub fn show(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    format: &str,
    edges: bool,
) -> Result<()> {
    let _ = temper_core::frontmatter::DocType::from_str(doc_type)?;

    match doc_type {
        "task" => crate::commands::task::show(config, slug, context, format),
        "session" => crate::commands::session::show(config, slug, context, format),
        _ => show_generic(config, doc_type, slug, context, format),
    }?;

    if edges {
        let ctx = require_context(context)?;
        show_edges(config, &ctx, doc_type, slug, format)?;
    }

    Ok(())
}
```

Important details from the real code: `edges` is *additive* (printed after `show_generic`), not exclusive. The clap `conflicts_with = "edges"` on `meta_only` prevents the user from requesting both, so this branch only needs to handle `meta_only` as an early return *before* the per-doctype dispatch.

Update the signature and add the early return:

```rust
pub fn show(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    format: &str,
    edges: bool,
    meta_only: bool,
    fields: &[String],
) -> Result<()> {
    let _ = temper_core::frontmatter::DocType::from_str(doc_type)?;

    if meta_only {
        return show_meta_only(config, doc_type, slug, context, format, fields);
    }

    match doc_type {
        "task" => crate::commands::task::show(config, slug, context, format),
        "session" => crate::commands::session::show(config, slug, context, format),
        _ => show_generic(config, doc_type, slug, context, format),
    }?;

    if edges {
        let ctx = require_context(context)?;
        show_edges(config, &ctx, doc_type, slug, format)?;
    }

    Ok(())
}
```

`show_meta_only` bypasses the per-doctype `task::show` / `session::show` routing — the meta endpoint is doctype-uniform (returns the same `ResourceMetaResponse` shape regardless of doc_type).

Add the new `show_meta_only` function alongside `show_generic`:

```rust
/// `show --meta-only`: hit GET /api/resources/{id}/meta and emit the
/// ResourceMetaResponse shape under the chosen format. Applies the
/// shared top-level projection filter when `fields` is non-empty.
///
/// Cloud-only: resolves the resource id via `resolve_by_uri` using
/// the same (owner, context, doc_type, slug) quadruple `show_generic`
/// uses, then calls `get_meta` instead of `content`.
fn show_meta_only(
    config: &Config,
    doc_type: &str,
    slug: &str,
    context: Option<&str>,
    format: &str,
    fields: &[String],
) -> Result<()> {
    use crate::actions::runtime;

    let _ = temper_core::frontmatter::DocType::from_str(doc_type)?;

    let config_clone = config.clone();
    let doc_type_inner = doc_type.to_string();
    let slug_inner = slug.to_string();
    let ctx_inner = context.map(str::to_string);
    let fields_inner = fields.to_vec();

    let meta = runtime::with_client(|client| {
        Box::pin(async move {
            let ctx = ctx_inner
                .as_deref()
                .ok_or_else(|| {
                    TemperError::Project("no context specified — use --context <name>".into())
                })?
                .to_string();
            let owner = config_clone.owner_for_context(&ctx);
            let row = client
                .resources()
                .resolve_by_uri(&owner, &ctx, &doc_type_inner, &slug_inner)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            let meta = client
                .resources()
                .get_meta(*row.id.as_uuid())
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)?;
            Ok(meta)
        })
    })?;

    let value = serde_json::to_value(&meta)
        .map_err(|e| TemperError::Api(format!("meta serialize: {e}")))?;
    let filtered = temper_core::projection::apply_top_level_filter(
        value,
        &fields_inner,
        "resource_id",
    )
    .map_err(map_projection_error)?;
    let fmt = crate::format::OutputFormat::resolve(Some(format));
    let rendered = crate::format::render(&filtered, fmt)?;
    println!("{rendered}");
    Ok(())
}

fn map_projection_error(err: temper_core::projection::ProjectionError) -> TemperError {
    use temper_core::projection::ProjectionError;
    match err {
        ProjectionError::DottedPath { hint } => TemperError::Project(format!(
            "--fields supports top-level keys only; use jq for nested projection: {hint}"
        )),
        ProjectionError::EmptyField => {
            TemperError::Project("--fields contained an empty field name".into())
        }
    }
}
```

Note: the parameter `fields_inner` is created and shadowed back to a local `&[String]` after the `with_client` block returns — this keeps the projection filter call outside the async closure (the filter is sync). If you'd prefer to apply the filter inside the closure, that's equally fine; the boundary is where you handle the error type conversion.

- [ ] **Step 4: Update the dispatch site**

Find where `ResourceAction::Show { ... }` is matched (typically `crates/temper-cli/src/main.rs` or `crates/temper-cli/src/commands/mod.rs`). Update the match arm to thread `meta_only` and `fields` through to the `show` function call.

- [ ] **Step 5: Manual smoke test (optional but recommended)**

If a development server is running and a known resource exists:

```bash
temper resource show <known-slug> --type task --context <ctx> --meta-only --format json | jq '.resource_id'
```

Expected: prints the resource UUID, surrounded by quotes.

```bash
temper resource show <known-slug> --type task --context <ctx> --meta-only --fields managed_meta.stage 2>&1
```

Expected: stderr error mentioning `jq` and the path `managed_meta.stage`.

- [ ] **Step 6: Run workspace tests**

```bash
cargo make check && cargo nextest run -p temper-cli
```

Expected: ✓ all green.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/
git commit -m "$(cat <<'EOF'
feat(cli): wire show --meta-only and --fields through the action layer

Branches show's dispatch on --meta-only: when set, resolves the
resource id and calls client.resources.get_meta() instead of the
full show path. --fields applies the shared projection filter to
the response before rendering. Anchor key resource_id is always
preserved.

Dotted-path --fields values produce a project error pointing at jq
for nested projection.
EOF
)"
```

---

## Task 8: Wire `temper resource list --meta-only` action

Mirror of Task 7 for the list verb. Calls `client.resources.list_meta(params)` and applies the projection filter to each row.

**Files:**
- Modify: `crates/temper-cli/src/commands/resource.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/temper-cli/src/commands/resource.rs` test module:

```rust
#[cfg(test)]
mod list_meta_only_tests {
    use super::*;
    use temper_core::projection::apply_top_level_filter;

    #[test]
    fn list_meta_filter_applies_per_row_and_preserves_envelope() {
        // Build a stub ResourceMetaListResponse-shaped JSON
        let envelope = serde_json::json!({
            "rows": [
                {
                    "resource_id": "11111111-1111-1111-1111-111111111111",
                    "managed_meta": {"stage": "in-progress"},
                    "open_meta": {"tags": []},
                    "managed_hash": "sha256:a",
                    "open_hash": "sha256:b"
                },
                {
                    "resource_id": "22222222-2222-2222-2222-222222222222",
                    "managed_meta": {"stage": "done"},
                    "open_meta": null,
                    "managed_hash": "sha256:c",
                    "open_hash": "sha256:d"
                }
            ],
            "total": 2,
            "facets": {"doc_type": {"task": 2}}
        });

        // Filter the rows array (the action layer will apply the filter
        // to envelope.rows specifically, not to the whole envelope).
        let rows = envelope.get("rows").cloned().expect("rows");
        let filtered_rows = apply_top_level_filter(
            rows,
            &["managed_meta".to_string()],
            "resource_id",
        )
        .expect("filter");

        // Each row should have only resource_id + managed_meta
        let arr = filtered_rows.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        for row in arr {
            assert!(row.get("resource_id").is_some(), "anchor missing in {row}");
            assert!(row.get("managed_meta").is_some(), "managed_meta missing in {row}");
            assert!(row.get("open_meta").is_none(), "open_meta should be dropped");
            assert!(row.get("managed_hash").is_none(), "hash should be dropped");
        }
    }
}
```

- [ ] **Step 2: Run test to verify it passes (filter already works)**

```bash
cargo nextest run -p temper-cli list_meta_filter
```

Expected: PASS.

- [ ] **Step 3: Wire the action**

Read the existing `pub fn list(config: &Config, params: ListParams<'_>) -> Result<()>` in `crates/temper-cli/src/commands/resource.rs` (around line 306) to understand the current dispatch pattern and the `ListParams<'_>` struct shape. Mirror it.

First, extend `ListParams<'_>` (the struct definition is in the same file) with two new fields. Read the struct first to confirm the existing field names, then add:

```rust
    pub meta_only: bool,
    pub fields: &'a [String],
```

Update the existing `list` function to branch on `meta_only`:

```rust
pub fn list(config: &Config, params: ListParams<'_>) -> Result<()> {
    let _ = temper_core::frontmatter::DocType::from_str(params.doc_type)?;
    // ... keep the existing stage/goal/status filter-applicability checks ...

    if params.meta_only {
        list_meta_only(config, params)
    } else {
        list_default(config, params)  // existing body, extracted if needed
    }
}
```

If the existing `list` body is inline (not already factored into `list_default`), the cleanest move is to extract it into a `list_default` helper first, then add the branch. Alternatively, keep the existing body inline and add the meta branch as an early return — match the local style.

Add `list_meta_only`:

```rust
/// `list --meta-only`: call client.resources().list_meta() and emit
/// the ResourceMetaListResponse shape. Applies the shared top-level
/// projection filter to each row in the envelope when `fields` is
/// non-empty; the envelope keys (`rows`, `total`, `facets`) are
/// preserved untouched.
fn list_meta_only(config: &Config, params: ListParams<'_>) -> Result<()> {
    use crate::actions::runtime;

    let api_params = build_list_request_params(config, &params)?;
    let fields_inner = params.fields.to_vec();
    let format_str = params.format.to_string();

    let response = runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .list_meta(&api_params)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let mut envelope = serde_json::to_value(&response)
        .map_err(|e| TemperError::Api(format!("meta list serialize: {e}")))?;

    if !fields_inner.is_empty() {
        let rows = envelope
            .get_mut("rows")
            .ok_or_else(|| TemperError::Api("response missing `rows` envelope key".into()))?
            .take();
        let filtered_rows = temper_core::projection::apply_top_level_filter(
            rows,
            &fields_inner,
            "resource_id",
        )
        .map_err(map_projection_error)?;
        envelope["rows"] = filtered_rows;
    }

    let fmt = crate::format::OutputFormat::resolve(Some(&format_str));
    let rendered = crate::format::render(&envelope, fmt)?;
    println!("{rendered}");
    Ok(())
}
```

Notes:
- The existing `list` body builds `ResourceListParams` inline (no helper function). Don't extract one for this task — match the existing inline style. Read the current `fetch_list_rows` (around line 281) to see what fields the existing list path sets on `ResourceListParams`. The meta path needs the same fields (`doc_type_name`, `context_name`, `sort`, `order`, `limit`) plus `meta_only: Some(true)`. If `list_meta` (Task 5) already sets `meta_only`, the explicit set here is double-defense.
- `map_projection_error` was added in Task 7. Reuse it.
- `ListParams<'_>` has a `format: &'a str` field (verified at `commands/resource.rs:276`). Resolve via `crate::format::OutputFormat::resolve(Some(params.format))`.
- The envelope filter follows the spec invariant: `total` and `facets` pass through untouched; only `rows` is filtered.
- Replace the sketched `build_list_request_params` call with the inline construction. The corrected `list_meta_only` body builds `ResourceListParams` directly:

```rust
fn list_meta_only(config: &Config, params: ListParams<'_>) -> Result<()> {
    use crate::actions::runtime;
    use temper_core::types::resource::{ResourceListParams, ResourceSortField, SortOrder};

    let limit = params.limit.unwrap_or(50);
    let api_params = ResourceListParams {
        doc_type_name: Some(params.doc_type.to_string()),
        context_name: params.context.map(ToString::to_string),
        sort: Some(ResourceSortField::Updated),
        order: Some(SortOrder::Desc),
        limit: Some(limit as i64),
        meta_only: Some(true),
        ..Default::default()
    };
    let format_str = params.format.to_string();
    let fields_owned: Vec<String> = params.fields.to_vec();

    let response = runtime::with_client(|client| {
        Box::pin(async move {
            client
                .resources()
                .list_meta(&api_params)
                .await
                .map_err(crate::actions::runtime::client_err_to_temper)
        })
    })?;

    let mut envelope = serde_json::to_value(&response)
        .map_err(|e| TemperError::Api(format!("meta list serialize: {e}")))?;

    if !fields_owned.is_empty() {
        let rows = envelope
            .get_mut("rows")
            .ok_or_else(|| TemperError::Api("response missing `rows` envelope key".into()))?
            .take();
        let filtered_rows = temper_core::projection::apply_top_level_filter(
            rows,
            &fields_owned,
            "resource_id",
        )
        .map_err(map_projection_error)?;
        envelope["rows"] = filtered_rows;
    }

    let fmt = crate::format::OutputFormat::resolve(Some(&format_str));
    let rendered = crate::format::render(&envelope, fmt)?;
    println!("{rendered}");
    Ok(())
}
```

Note the use of `params.limit.unwrap_or(50)` and `as i64` — match the existing pagination defaults if they differ. Stage/goal/status filters are not threaded into the wire params here because the default `list` path also doesn't forward them via `ResourceListParams` (they're CLI-only hints in the existing code). If the spec wants them on the wire, that's a separate task — out of scope here.

- [ ] **Step 4: Update the dispatch site**

Find `ResourceAction::List { .. }` match in `main.rs` (or wherever). Pass `meta_only` and `fields` into the `ListParams` struct.

- [ ] **Step 5: Run the tests**

```bash
cargo nextest run -p temper-cli list_meta
```

Expected: PASS.

- [ ] **Step 6: Workspace check**

```bash
cargo make check
```

Expected: ✓ all green.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-cli/
git commit -m "$(cat <<'EOF'
feat(cli): wire list --meta-only and --fields through the action layer

list now branches on --meta-only: when set, calls
client.resources.list_meta() and returns
ResourceMetaListResponse. --fields filters each row in the rows
array (envelope total/facets are preserved untouched).

This closes the CLI half of the meta-only projection spec.
EOF
)"
```

---

## Task 9: MCP `get_resource` — add `fields` parameter

Apply the same projection filter at the MCP tool boundary. No `meta_only` parameter — MCP's existing `include_content=false` default already serves cheap orientation.

**Files:**
- Modify: `crates/temper-mcp/src/tools/resources.rs`

- [ ] **Step 1: Write the failing test**

Add or extend the MCP tests for `get_resource`. Look at `tests/e2e/tests/mcp_get_resource_meta_test.rs` or similar for the existing harness pattern.

Append to `crates/temper-mcp/src/tools/resources.rs` (or a sibling test file matching crate conventions):

```rust
#[cfg(all(test, feature = "test-db"))]
mod fields_projection_tests {
    use super::*;

    #[test]
    fn get_resource_input_accepts_fields() {
        // Compile-time check that GetResourceInput grows the field.
        let _input = GetResourceInput {
            id: None,
            slug: Some("x".to_string()),
            context_name: Some("y".to_string()),
            include_content: Some(false),
            fields: Some(vec!["managed_meta".to_string()]),
        };
    }

    #[test]
    fn enriched_resource_filtered_by_fields_preserves_id_and_managed_meta() {
        // Stub an EnrichedResource value
        let value = serde_json::json!({
            "id": "11111111-1111-1111-1111-111111111111",
            "title": "Test",
            "slug": "test",
            "context_name": "temper",
            "doc_type_name": "task",
            "owner": "@me",
            "origin_uri": "",
            "is_active": true,
            "created": "2026-05-27T00:00:00Z",
            "updated": "2026-05-27T00:00:00Z",
            "managed_meta": {"stage": "in-progress"},
            "open_meta": {"tags": []}
        });
        let filtered = temper_core::projection::apply_top_level_filter(
            value,
            &["managed_meta".to_string()],
            "id",
        )
        .expect("filter");
        assert!(filtered.get("id").is_some(), "anchor id missing");
        assert!(filtered.get("managed_meta").is_some(), "managed_meta missing");
        assert!(filtered.get("title").is_none(), "title should be dropped");
        assert!(filtered.get("open_meta").is_none(), "open_meta should be dropped");
    }
}
```

A full e2e MCP test with the live server lands in Task 11.

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo nextest run -p temper-mcp --features test-db fields_projection
```

Expected: compile error — `fields` field not on `GetResourceInput`.

- [ ] **Step 3: Add `fields` to `GetResourceInput`**

In `crates/temper-mcp/src/tools/resources.rs`, find `pub struct GetResourceInput` (around line 50). Add the new field:

```rust
    /// Subselect top-level response keys. Anchor key `id` is always
    /// preserved. Nested paths (containing `.`) rejected with a hint
    /// pointing at `jq` — MCP callers should perform deeper projection
    /// at their own end. When None or empty, no filtering is applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
```

- [ ] **Step 4: Apply the filter in `get_resource` handler**

In the `get_resource` function (around line 375 of the same file), after the response is built but before `to_text(...)` is called, apply the filter:

```rust
    // After building `enriched` (and optional `content.markdown`),
    // serialize, filter, then text-encode.
    let enriched_value = serde_json::to_value(&enriched).map_err(|e| {
        rmcp::ErrorData::internal_error(format!("Failed to serialize: {e}"), None)
    })?;

    let filtered = if let Some(fields) = input.fields.as_deref() {
        temper_core::projection::apply_top_level_filter(enriched_value, fields, "id")
            .map_err(|e| match e {
                temper_core::projection::ProjectionError::DottedPath { hint } => {
                    rmcp::ErrorData::invalid_params(
                        format!(
                            "fields supports top-level keys only; use jq for nested projection: {hint}"
                        ),
                        None,
                    )
                }
                temper_core::projection::ProjectionError::EmptyField => {
                    rmcp::ErrorData::invalid_params(
                        "fields contained an empty entry".to_string(),
                        None,
                    )
                }
            })?
    } else {
        enriched_value
    };

    // Build the result content. include_content branch still adds the
    // markdown body as a second content part.
    let mut parts = vec![rmcp::model::Content::text(
        serde_json::to_string_pretty(&filtered).unwrap_or_else(|_| "{}".to_string()),
    )];
    if let Some(markdown) = body_markdown {
        parts.push(rmcp::model::Content::text(markdown));
    }
    Ok(CallToolResult::success(parts))
```

Refactor the existing `if input.include_content.unwrap_or(false) { ... } else { ... }` branch so both arms share the filter-and-encode tail. Pseudocode:

```rust
let (enriched, body_markdown) = if input.include_content.unwrap_or(false) {
    let content = resource_service::get_content(pool, profile.id, row.id.into()).await?;
    let enriched = build_enriched(pool, profile_id, &row, content.managed_meta, content.open_meta).await?;
    (enriched, Some(content.markdown))
} else {
    (enrich_resource(pool, profile_id, &row).await?, None)
};
// then apply filter + emit (as above)
```

The exact rework requires reading the current function — keep the contract unchanged: with `include_content=true`, the markdown body is still returned as a separate content part, unfiltered.

- [ ] **Step 5: Run tests**

```bash
cargo nextest run -p temper-mcp --features test-db fields_projection
```

Expected: both PASS.

- [ ] **Step 6: Workspace check**

```bash
cargo make check
```

Expected: ✓ all green.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-mcp/
git commit -m "$(cat <<'EOF'
feat(mcp): add fields parameter to get_resource for top-level projection

GetResourceInput grows fields: Option<Vec<String>>, applied via the
shared temper_core::projection filter before serializing the
EnrichedResource into the MCP text content. Anchor id always
preserved. Dotted paths return invalid_params with a jq hint.

include_content=true behavior unchanged: markdown body still emitted
as a separate content part, never filtered.

No meta_only parameter — MCP default (include_content=false) already
serves cheap orientation via EnrichedResource, which is strictly
richer than ResourceMetaResponse (joined display fields).
EOF
)"
```

---

## Task 10: MCP `list_resources` — add `fields` parameter

Same pattern as Task 9, applied to the list handler. The filter applies to each element of the returned array.

**Files:**
- Modify: `crates/temper-mcp/src/tools/resources.rs`

- [ ] **Step 1: Write the failing test**

Add to the `fields_projection_tests` module from Task 9 (or a sibling):

```rust
    #[test]
    fn list_resources_input_accepts_fields() {
        let _input = ListResourcesInput {
            // existing fields ...
            fields: Some(vec!["managed_meta".to_string()]),
            // ... any other fields needed to construct the struct
        };
    }

    #[test]
    fn enriched_resource_array_filtered_by_fields() {
        let value = serde_json::json!([
            {
                "id": "11111111-1111-1111-1111-111111111111",
                "title": "A",
                "managed_meta": {"stage": "done"}
            },
            {
                "id": "22222222-2222-2222-2222-222222222222",
                "title": "B",
                "managed_meta": {"stage": "in-progress"}
            }
        ]);
        let filtered = temper_core::projection::apply_top_level_filter(
            value,
            &["managed_meta".to_string()],
            "id",
        )
        .expect("filter");
        let arr = filtered.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        for row in arr {
            assert!(row.get("id").is_some());
            assert!(row.get("managed_meta").is_some());
            assert!(row.get("title").is_none());
        }
    }
```

The first test requires reading `ListResourcesInput` to know what fields to fill in — match the existing struct.

- [ ] **Step 2: Run test to verify it fails**

```bash
cargo nextest run -p temper-mcp --features test-db list_resources_input_accepts_fields
```

Expected: compile error — `fields` not on `ListResourcesInput`.

- [ ] **Step 3: Add `fields` to `ListResourcesInput`**

In `crates/temper-mcp/src/tools/resources.rs`, find `pub struct ListResourcesInput`. Add the field:

```rust
    /// Subselect top-level response keys for each row. Anchor key `id`
    /// is always preserved per row. Nested paths rejected with a jq
    /// hint. When None or empty, no filtering is applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fields: Option<Vec<String>>,
```

- [ ] **Step 4: Apply the filter in `list_resources` handler**

Find the existing `list_resources` function. After it builds `Vec<EnrichedResource>` (typically via `enrich_resources`), serialize to JSON, apply the filter to the array, then emit. Sketch:

```rust
    let enriched: Vec<EnrichedResource> = enrich_resources(pool, profile_id, &rows).await?;
    let array_value = serde_json::to_value(&enriched).map_err(|e| {
        rmcp::ErrorData::internal_error(format!("Failed to serialize: {e}"), None)
    })?;

    let filtered = if let Some(fields) = input.fields.as_deref() {
        temper_core::projection::apply_top_level_filter(array_value, fields, "id")
            .map_err(|e| match e {
                temper_core::projection::ProjectionError::DottedPath { hint } => {
                    rmcp::ErrorData::invalid_params(
                        format!(
                            "fields supports top-level keys only; use jq for nested projection: {hint}"
                        ),
                        None,
                    )
                }
                temper_core::projection::ProjectionError::EmptyField => {
                    rmcp::ErrorData::invalid_params(
                        "fields contained an empty entry".to_string(),
                        None,
                    )
                }
            })?
    } else {
        array_value
    };

    Ok(CallToolResult::success(vec![rmcp::model::Content::text(
        serde_json::to_string_pretty(&filtered).unwrap_or_else(|_| "[]".to_string()),
    )]))
```

If the existing handler does additional work (totals, facets, pagination info), preserve it — wrap the array filtering inside whatever envelope exists.

- [ ] **Step 5: Run tests**

```bash
cargo nextest run -p temper-mcp --features test-db
```

Expected: PASS.

- [ ] **Step 6: Workspace check**

```bash
cargo make check
```

Expected: ✓ all green.

- [ ] **Step 7: Commit**

```bash
git add crates/temper-mcp/
git commit -m "$(cat <<'EOF'
feat(mcp): add fields parameter to list_resources

ListResourcesInput.fields applies the shared projection filter to
each EnrichedResource row before MCP text encoding. Anchor id
preserved per row; dotted paths return invalid_params with jq hint.

Closes the MCP half of the meta-only projection spec.
EOF
)"
```

---

## Task 11: End-to-end CLI driver tests

Drive `temper resource show --meta-only` and `temper resource list --meta-only` through the real binary against a real Axum server + Postgres test DB.

**Files:**
- Create: `tests/e2e/tests/cli_meta_projection_test.rs`

- [ ] **Step 1: Write the failing tests**

Create `tests/e2e/tests/cli_meta_projection_test.rs`:

```rust
#![cfg(feature = "test-db")]

mod common;

use serde_json::Value;
use temper_core::types::ingest::IngestPayload;

/// `temper resource show <slug> --meta-only --format json` returns
/// the ResourceMetaResponse shape (resource_id + managed_meta + ...).
#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn show_meta_only_returns_meta_response_shape(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client.profile().get().await.expect("profile pre-flight");
    app.client.contexts().create("meta-cli").await.expect("ctx create");

    let payload = IngestPayload {
        title: "Show Meta Test".to_string(),
        origin_uri: "test://e2e/show-meta".to_string(),
        context_name: "meta-cli".to_string(),
        doc_type_name: "task".to_string(),
        content_hash: Some(
            "showmeta0000000000000000000000000000000000000000000000000000000".to_string(),
        ),
        slug: "show-meta-test".to_string(),
        content: "# Show Meta\n\nBody here.".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"stage": "in-progress"})),
        open_meta: None,
        chunks_packed: Some(temper_core::types::ingest::pack_chunks(&[]).unwrap()),
    };

    let _ = app.client.ingest().create(&payload).await.expect("ingest");

    // Run the CLI binary against the test server
    let output = common::run_temper_cli(
        &app,
        &[
            "resource", "show", "show-meta-test",
            "--type", "task",
            "--context", "meta-cli",
            "--meta-only",
            "--format", "json",
        ],
    )
    .await
    .expect("cli run");

    assert!(output.status.success(), "cli failed: stderr={}", String::from_utf8_lossy(&output.stderr));
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("json parse");
    assert!(stdout.get("resource_id").is_some(), "missing resource_id: {stdout}");
    assert!(stdout.get("managed_meta").is_some(), "missing managed_meta");
    assert!(stdout.get("open_meta").is_some() || stdout.get("open_meta").is_none(), "open_meta present or absent");
    // Confirm we DON'T have the body or row fields
    assert!(stdout.get("content").is_none(), "should not include body");
    assert!(stdout.get("title").is_none(), "should not include row title");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn show_meta_only_with_fields_filters_response(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;

    app.client.profile().get().await.expect("profile pre-flight");
    app.client.contexts().create("meta-cli").await.expect("ctx create");

    // ... ingest a resource (copy structure from previous test) ...
    let payload = IngestPayload {
        title: "Fields Filter Test".to_string(),
        origin_uri: "test://e2e/fields-filter".to_string(),
        context_name: "meta-cli".to_string(),
        doc_type_name: "task".to_string(),
        content_hash: Some(
            "fieldsfilt0000000000000000000000000000000000000000000000000000000".to_string(),
        ),
        slug: "fields-filter-test".to_string(),
        content: "# Test".to_string(),
        metadata: None,
        managed_meta: Some(serde_json::json!({"stage": "backlog"})),
        open_meta: None,
        chunks_packed: Some(temper_core::types::ingest::pack_chunks(&[]).unwrap()),
    };
    app.client.ingest().create(&payload).await.expect("ingest");

    let output = common::run_temper_cli(
        &app,
        &[
            "resource", "show", "fields-filter-test",
            "--type", "task",
            "--context", "meta-cli",
            "--meta-only",
            "--fields", "managed_meta",
            "--format", "json",
        ],
    )
    .await
    .expect("cli run");

    assert!(output.status.success(), "cli failed: stderr={}", String::from_utf8_lossy(&output.stderr));
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("json parse");
    assert!(stdout.get("resource_id").is_some(), "anchor missing");
    assert!(stdout.get("managed_meta").is_some(), "managed_meta missing");
    assert!(stdout.get("open_meta").is_none(), "open_meta should be filtered");
    assert!(stdout.get("managed_hash").is_none(), "hash should be filtered");
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn show_meta_only_with_dotted_path_errors(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client.profile().get().await.expect("profile");
    app.client.contexts().create("meta-cli").await.expect("ctx");

    // No ingest needed — the error happens before the network call
    let output = common::run_temper_cli(
        &app,
        &[
            "resource", "show", "any-slug",
            "--type", "task",
            "--context", "meta-cli",
            "--meta-only",
            "--fields", "managed_meta.stage",
        ],
    )
    .await
    .expect("cli run");

    assert!(!output.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("jq"), "stderr should mention jq: {stderr}");
    assert!(
        stderr.contains("managed_meta.stage"),
        "stderr should echo the rejected path: {stderr}"
    );
}

#[sqlx::test(migrator = "temper_api::MIGRATOR")]
async fn list_meta_only_returns_meta_list_response_shape(pool: sqlx::PgPool) {
    let app = common::setup(pool).await;
    app.client.profile().get().await.expect("profile");
    app.client.contexts().create("meta-cli").await.expect("ctx");

    // Ingest two task resources
    for (slug, hash) in &[
        ("list-meta-a", "lista00000000000000000000000000000000000000000000000000000000000"),
        ("list-meta-b", "listb00000000000000000000000000000000000000000000000000000000000"),
    ] {
        let payload = IngestPayload {
            title: format!("List Meta {}", slug),
            origin_uri: format!("test://e2e/{}", slug),
            context_name: "meta-cli".to_string(),
            doc_type_name: "task".to_string(),
            content_hash: Some(hash.to_string()),
            slug: slug.to_string(),
            content: "# Test".to_string(),
            metadata: None,
            managed_meta: Some(serde_json::json!({"stage": "in-progress"})),
            open_meta: None,
            chunks_packed: Some(temper_core::types::ingest::pack_chunks(&[]).unwrap()),
        };
        app.client.ingest().create(&payload).await.expect("ingest");
    }

    let output = common::run_temper_cli(
        &app,
        &[
            "resource", "list",
            "--type", "task",
            "--context", "meta-cli",
            "--meta-only",
            "--format", "json",
        ],
    )
    .await
    .expect("cli run");

    assert!(output.status.success(), "cli failed: stderr={}", String::from_utf8_lossy(&output.stderr));
    let stdout: Value = serde_json::from_slice(&output.stdout).expect("json parse");
    let rows = stdout.get("rows").expect("envelope.rows").as_array().expect("array");
    assert!(rows.len() >= 2, "expected at least 2 rows: {stdout}");
    for row in rows {
        assert!(row.get("resource_id").is_some(), "row missing resource_id");
        assert!(row.get("managed_meta").is_some(), "row missing managed_meta");
    }
    assert!(stdout.get("total").is_some(), "envelope missing total");
    assert!(stdout.get("facets").is_some(), "envelope missing facets");
}
```

The `common::run_temper_cli` helper does not currently exist — Task 11 adds it. Add this to `tests/e2e/tests/common/mod.rs` near the existing `setup` function:

```rust
/// Run the `temper` CLI binary against the in-process Axum server.
///
/// Sets `TEMPER_API_URL` to the test server's URL and `TEMPER_TOKEN`
/// to the test JWT so the CLI hits the real handler stack without
/// needing a separate auth round-trip. Spawned via `spawn_blocking`
/// so we don't block the runtime.
///
/// Verified env-var names against `crates/temper-client/src/config.rs`
/// (`TEMPER_API_URL`) and `crates/temper-cli/src/actions/runtime.rs`
/// (`TEMPER_TOKEN`).
pub async fn run_temper_cli(
    app: &E2eTestApp,
    args: &[&str],
) -> std::io::Result<std::process::Output> {
    let url = app.base_url();
    let token = app.token.clone();
    let args_owned: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    tokio::task::spawn_blocking(move || {
        std::process::Command::new(env!("CARGO_BIN_EXE_temper"))
            .env("TEMPER_API_URL", &url)
            .env("TEMPER_TOKEN", &token)
            .args(&args_owned)
            .output()
    })
    .await
    .expect("spawn_blocking join")
}
```

The binary path is resolved via `env!("CARGO_BIN_EXE_temper")`, which Cargo populates when the `temper-cli` package is a dev-dependency of the e2e test crate. Verify the existing `tests/e2e/Cargo.toml` lists `temper-cli` under `[dev-dependencies]`; if not, add it. The pattern is established by other binary-invocation tests in the workspace.

- [ ] **Step 2: Run the tests**

```bash
cargo make test-e2e
```

Expected: all 4 new tests PASS. The existing e2e tests should also still pass (regression).

If `cargo make test-e2e` doesn't include the new file, check `tests/e2e/Cargo.toml` or `Makefile.toml` to confirm the e2e test crate picks up new `*.rs` files automatically (typically it does via the `[[test]]` convention or `name = "tests"`).

- [ ] **Step 3: Workspace check**

```bash
cargo make check
```

Expected: ✓ all green.

- [ ] **Step 4: Commit**

```bash
git add tests/e2e/tests/
git commit -m "$(cat <<'EOF'
test(e2e): cover CLI meta-only projection through the binary

Drives `temper resource show --meta-only [--fields X]` and
`temper resource list --meta-only` through the real binary against
a real Axum server + Postgres test DB.

Covers:
- show --meta-only returns ResourceMetaResponse shape
- show --meta-only --fields managed_meta filters response keys
- --fields with dotted path produces clear jq pointer in stderr
- list --meta-only returns ResourceMetaListResponse shape
EOF
)"
```

---

## Final verification

After all tasks land:

- [ ] Full workspace check

```bash
cargo make check
```

- [ ] Full test sweep

```bash
cargo make test-all
```

Expected: All Rust tests pass, all TypeScript tests pass.

- [ ] E2E with embed feature (mirror CI's matrix)

```bash
cargo make test-e2e-embed
```

Expected: PASS — the projection filter is feature-flag-independent, so this should be a no-op regression.

- [ ] Smoke-test the binary against a running dev server

```bash
# Start the dev server in another terminal: cargo make run
temper resource list --type task --context temper --meta-only --format json | jq '.rows | length'
temper resource list --type task --context temper --meta-only --fields managed_meta --format toon | head -20
temper resource show <some-slug> --type task --context temper --meta-only --format json | jq '.resource_id'
temper resource show <some-slug> --type task --context temper --meta-only --fields managed_meta.stage 2>&1 | grep jq
```

Each should behave as the spec describes.

- [ ] Update the parent task in the vault to mark progress

```bash
temper resource update 2026-05-25-cli-read-side-meta-affordance-for-resources \
  --type task --context temper --stage done
```

(Only when the implementation lands on main, not on the branch.)

- [ ] Open PR

```bash
git merge origin/main  # per feedback_merge_main_before_pushing_pr
git push -u origin jct/cli-meta-only-projection
gh pr create --title "feat: CLI meta-only projection + --fields subselection" --body "..."
```
