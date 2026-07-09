# P0 — Make the OpenAPI spec authoritative (implementation plan)

Task: `p0-make-the-openapi-spec-authoritative-019f4911-fbe5-7a72-8ef4-e4f506edbec3`
Goal: `temper-rb — a native Ruby client for the temper API` (`019f4910-…`)
Design: `docs/superpowers/specs/2026-07-09-temper-rb-ruby-bindings-design.md`

## Decision

The spec becomes a **product of the router**, not a hand-maintained parallel list.
`utoipa-axum` 0.2 (`axum ^0.8`, `utoipa ^5` — both already in tree) provides `OpenApiRouter`:
`.routes(routes!(handler))` mounts the axum route *and* collects its `#[utoipa::path]`, deriving
the axum path from the annotation. `ApiDoc`'s 54-entry `paths(...)` list is deleted. Path/method
drift becomes unrepresentable rather than detected.

Registration is separated from layering so the spec is a pure function:

```rust
fn gated_routes() -> OpenApiRouter<AppState> { … }   // no state value needed
pub fn create_app(state: AppState) -> Router { gated_routes().layer(…).merge(…) }
pub fn openapi_spec() -> utoipa::openapi::OpenApi { … split_for_parts().1 }
```

`openapi_spec()` touches no `AppState` and no database.

## Exclusions (deliberate, two mechanisms)

| Surface | Mechanism | Why |
|---|---|---|
| `internal_saml::reconcile` | sub-router omitted from `openapi_spec()`'s merge | server-to-server, shared-secret gated |
| `access::{list_pending, review_request, get_admin_settings, update_settings, promote_admin}` | plain `.route()` inside `gated_routes()` | operator surface |

`access::{create_request, get_own_request, withdraw_request, get_settings}` are **documented** — a
caller requesting access to their own instance is a library caller, not an operator.

`embed::dispatch` is internal (secret-gated) but is already annotated and already in the spec.
Left documented; removing it was not sanctioned. Flag as a follow-up question.

## Beats

### B1 — `ActInput` becomes a documented query-param type *(independent)*
- `crates/temper-core/src/types/authorship.rs`: add
  `#[cfg_attr(feature = "web-api", derive(utoipa::IntoParams))]` to `ActInput`, matching the
  gating already on its `ToSchema` derive. Precedent: `ResourceListParams`
  (`crates/temper-workflow/src/types/resource.rs:136`).
- `crates/temper-api/src/handlers/resources.rs:319` (`delete`) and
  `crates/temper-api/src/handlers/cognitive_maps.rs:40` (`reconcile`): add `ActInput` to the
  existing `params(...)` block alongside the `id` path param.
- Verify: `cargo check -p temper-api --all-features`. `InvocationId` and `ConfidenceBand` already
  derive `ToSchema` under `web-api`, so nested types resolve.

### B2 — annotate the 4 self-service `access` handlers *(independent)*
- `crates/temper-api/src/handlers/access.rs`: add `#[utoipa::path]` to `create_request`,
  `get_own_request`, `withdraw_request`, `get_settings`. Match the annotation shape of a sibling
  (`handlers/profiles.rs`). Add `tag = "Access"` to `ApiDoc`'s `tags(...)`.
- Response body types must derive `ToSchema` under `web-api`; add where missing and register in
  `components(schemas(...))`.
- Leave the 5 admin handlers unannotated.

### B3 — rewrite `routes.rs` onto `OpenApiRouter` *(depends: B1, B2)*
- Add `utoipa-axum = "0.2"` to `crates/temper-api/Cargo.toml`.
- Split `create_app` into registration fns returning `OpenApiRouter<AppState>`
  (`public_routes`, `auth_only_routes`, `gated_routes`) plus plain-`Router` `internal_routes` /
  `embed_internal_routes`. Layers stay in `create_app`.
- Every documented route registers via `.routes(routes!(…))`. Operator-only mounts use plain
  `.route()` **with a comment naming why**.
- Add `pub fn openapi_spec() -> utoipa::openapi::OpenApi`, seeded with
  `OpenApiRouter::with_openapi(ApiDoc::openapi())` so info/tags/`SecurityAddon` survive.
- `openapi.rs`: delete `paths(...)`. Keep `info`, `tags`, `modifiers`, `components(schemas(...))`.
- Swagger mount consumes `openapi_spec()` instead of `ApiDoc::openapi()`.
- Update the existing `openapi.rs` unit test to drive `openapi_spec()`.
- The 22 orphaned handlers (teams ×8, segments ×3, ingest ×2, contexts ×3, graph ×4,
  `resources::provenance`, `events::element_trail`) register automatically. Verify count rises.

### B4 — emit `openapi.json` as a checked-in artifact *(depends: B3)*
- `crates/temper-api/src/bin/emit-openapi.rs`: print `openapi_spec().to_pretty_json()`. No DB.
- `cargo make openapi` writes `openapi.json` at repo root. Model the task on `generate-ts-types`
  (`tools/cargo-make/main.toml:206`): a `script` task that fails loudly on empty output.
- `cargo make openapi-check` regenerates to a temp path and diffs against the committed file;
  fails with a message naming `cargo make openapi`.
- Commit the generated `openapi.json`.

### B5 — CI gate *(depends: B4)*
- `.github/scripts/check-openapi-routes.sh`: assert every plain `.route(` call in `routes.rs`
  matches the operator-only allowlist. Stable because `.route(` is now the ~7-line exception.
  Unit-test it the way `detect-ci-scope.sh` is tested (`test-detect-ci-scope.sh`).
- Wire `openapi-check` + the route-allowlist script into `code-quality.yml`'s `rust-quality` job.
  It reaches `ci-success` through the existing `code-quality` result — no `ci.yml` change needed.
- Docs-only changes still skip, per `detect-ci-scope.sh`. Correct: the spec can't drift on a
  docs-only diff.

## Verification

- `cargo make check` (fmt, clippy `-D warnings`, docs, machete — machete will flag `utoipa-axum`
  if B3 leaves it unused).
- `cargo nextest run -p temper-api --features test-db --test swagger_gating`
- `cargo make test` then `cargo make test-db`.
- `cargo make openapi && git diff --exit-code openapi.json` — proves the artifact is reproducible.
- Confirm `jq '.paths | length' openapi.json` accounts for all documented routes and that
  `/api/access/admin/promote` and `/internal/saml/reconcile` are **absent**.
- Confirm `jq '.paths."/api/resources/{id}".delete.parameters'` lists the six `ActInput` fields.

## Non-goals

Rust-side only. No Ruby, no `Surface` header (P2), no `correlation` (P3), no `sdk` entity (P1).
