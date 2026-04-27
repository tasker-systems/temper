# Cloud-Mode Write Routing & Managed-Meta Expansion — Design Spec

**Date:** 2026-04-27
**Context:** `temper`
**Mode:** plan → build (this spec covers the plan output; implementation is a follow-on session)
**Effort:** medium
**Branch:** `jct/temper-cloud-mode-portable-memory` (continues the Part 3 PR; same scope)

**Related work:**
- Continues `docs/superpowers/specs/2026-04-22-cloud-first-routing-and-mode-collapse-design.md`. Read paths and local-mode write tails shipped under that spec across Sessions A + B + C + C.5; cloud-mode write paths and the server-side `ResourceUpdateRequest` expansion were deferred and are the subject of this spec.
- Predecessor task: `2026-04-22-unit-b-2-part-3-cloud-first-routing-mode-collapse` (closed 2026-04-27 with Scope Closure note).
- This task: `2026-04-27-unit-b-2-part-3b-cloud-mode-write-routing-managed-meta-expansion`.
- Sibling task spun out during this design: `2026-04-27-unify-resource-delete-cloud-first-explicit-only-manifest-cleanup` (delete unification, sequenced after Part 3B merge).

---

## Problem

Sessions A + B + C + C.5 of Part 3 shipped read-path cloud routing (`list`, `show` always API), local-mode write tails through `push_one_resource`, the publish-tail policy, and unified auth-path resolution. Two write surfaces remain unbuilt:

1. **Cloud-mode `create`** has no `VaultState::Cloud` branch. Today, every `temper resource create` invocation under cloud mode would attempt to resolve the vault, write a templated file to disk, and call `push_one_resource` — none of which is meaningful when the vault doesn't exist. The command would panic on `resolve_vault()`.
2. **Cloud-mode `update`** has the same gap, with an additional server-side blocker called out in the original Part 3 spec: `ResourceUpdateRequest` accepts only `{title, slug}` today. Any attempt at a managed-meta-bearing cloud update fails before it leaves the wire — the server has nowhere to put `--stage`, `--mode`, `--effort`, `--goal`, `--seq`, `--branch`, `--pr`, or `--status` mutations.

Plus three smaller residuals from the original Part 3 plan: a `sync run` cloud-mode redirect message, e2e coverage of cloud-mode round-trips, and final cleanup including deletion of the superseded `2026-04-19` dispatch plan.

Without these, cloud-mode is read-only at the resource layer — the canonical use case (Claude Code web sessions, Cursor cloud agents writing session notes back to a shared vault) is blocked.

## Approach

One operational invariant ties the design together. Pete framed it during brainstorming on 2026-04-27:

> "Locally, we treat the vault-on-disk as a stable and intentional resource. In cloud mode, we assume that all files on disk are there only for read-cache or base-then-update."

Under `VaultState::Cloud`, the source of truth is the server. Anything on disk — the `show` debounce cache from Session A, a scratch tmpfile holding the body for an upcoming update — is derivative, never authoritative. No write path may treat a cloud-mode disk artifact as the resource. This invariant is what justifies the rest of the design: cloud `create` skips the templated file entirely; cloud `update` builds its payload from arguments + stdin without round-tripping through YAML on disk; `sync run` errors out because there is no local state to reconcile against the server.

The shape of the change is symmetric with what Session A did for `show`: `match VaultState` at each command's divergence site, with both branches living side-by-side in the existing command file. No new modules. No `resource_cloud.rs`. The local-mode branches keep their existing `push_one_resource` flow unchanged in behavior; cloud-mode branches get added next to them.

A second invariant — flagged by Pete during the brainstorming — is that all chunk and hash computation must flow through one helper. Today, `actions::ingest::build_ingest_payload` is the shared primitive, and four call sites already converge on it (sync.rs ×2, add.rs ×2). Cloud-mode write paths will use the same helper. Where the helper's signature falls short for cloud needs (no `managed_meta` parameter, no caller-supplied body-trio extraction) we grow it; we do not introduce a parallel implementation.

## Server-Side: `ResourceUpdateRequest` Expansion

`ResourceUpdateRequest` in `crates/temper-core/src/types/resource.rs` grows from `{title, slug}` to a partial-update shape that mirrors the fields a local-mode CLI invocation can already mutate:

```rust
pub struct ResourceUpdateRequest {
    pub title: Option<String>,                  // existing
    pub slug: Option<String>,                   // existing
    pub managed_meta: Option<ManagedMeta>,      // NEW — partial; merges into stored
    pub open_meta: Option<serde_json::Value>,   // NEW — partial; merges into stored
    pub content: Option<String>,                // NEW — body markdown
    pub content_hash: Option<String>,           // NEW — required iff content is Some
    pub chunks_packed: Option<String>,          // NEW — required iff content is Some
}
```

**Validation invariant**: `content`, `content_hash`, and `chunks_packed` form an all-or-nothing trio. The handler returns `400` if one is present without the others. The CLI either builds the trio together (when stdin or `--body` provides a body) or omits all three.

`ManagedMeta` is already round-trip-lossless via its `extra: HashMap<String, Value>` flatten bucket, and every typed field is `Option<T>`. That natural shape is what makes the partial-update model work without inventing new types — `None` means "untouched"; `Some(value)` means "set." `MetaUpdatePayload` (the existing full-replacement type used by sync) is left untouched; it serves a different model and stays in place for the manifest sync path.

**Service flow** (`resource_service::update`, `crates/temper-api/src/services/resource_service.rs:501`):

1. **Auth gate** — existing `can_modify_resource()` check, runs before any read or write.
2. **Load current state** — fetch the `kb_resource_manifests` row for stored `managed_meta`, `open_meta`, hashes, and `body_hash`.
3. **Merge metadata** — for each `Some` field on incoming `managed_meta`, overlay onto the stored `ManagedMeta`. The `extra` bucket merges by key (incoming wins per key). Same shape for `open_meta`. Recompute `managed_hash` and `open_hash` over the canonical-form output of the merged structs.
4. **Body path** — if `content` is `Some`:
   - If `content_hash` matches stored `body_hash`, skip body persistence entirely (no chunk insert, no rewire). Meta updates still apply.
   - Otherwise, run the chunk-dedupe primitive the ingest path already uses: walk the supplied `chunks_packed`, insert chunks whose hash isn't already in `kb_chunks`, rewire `kb_resource_chunks` to the new chunk set, update `body_hash` on the manifest row.
5. **Persist** — single transaction. `title`/`slug` to `kb_resources`; manifest row + new hashes to `kb_resource_manifests`; chunk rewiring to `kb_resource_chunks`. Bump `kb_resources.updated`.
6. **Return** — `ResourceRow`, augmented if needed so the CLI can confirm the apply landed and surface the new `content_hash` for the agent's next show-edit-cat cycle.

**Concurrency**: last-writer-wins. The server applies the PATCH unconditionally. No `If-Match` header, no `prev_content_hash` field. This matches the use case (single-actor agent sessions, short-lived) and is non-breakingly extensible — a future optional `prev_content_hash: Option<String>` field, validated when present, would add optimistic concurrency without breaking existing clients.

**Partial vs. clearing**: since every field on `ManagedMeta` is `Option<T>`, "field is `None`" means "untouched" — there is no in-band signal for "explicitly clear this field." This matches the local-mode CLI surface (no `--clear-goal` flag exists today either). Forward-compat note: when field-clearing semantics are needed, they will land as a sibling `PUT /api/resources/{id}` with a CLI `--clear-meta <fields>` surface — full-replacement model, separate endpoint, additive change. Out of scope for this unit.

**Surfaces touched**: `crates/temper-core/src/types/resource.rs` (struct), `crates/temper-api/src/services/resource_service.rs` (update fn), `crates/temper-api/src/handlers/resources.rs` (handler signature unchanged but utoipa schema regenerates), `crates/temper-api/src/openapi.rs` (regen), `bindings/` via `ts-rs` (regen). SQL macros in the update fn need `cargo sqlx prepare --workspace -- --all-features` after the new statements land.

## CLI Changes

### Helper consolidation (single source of truth)

Two surgical edits in `crates/temper-cli/src/actions/ingest.rs`:

**(1) Grow `build_ingest_payload` signature** to accept the two fields it currently nulls out:

```rust
pub fn build_ingest_payload(
    content: &str,
    title: &str,
    context: &str,
    doc_type: &str,
    metadata: Option<serde_json::Value>,
    managed_meta: Option<ManagedMeta>,    // NEW
    open_meta: Option<serde_json::Value>, // NEW
) -> Result<IngestPayload>;
```

The four existing call sites (`sync.rs:1187`, `sync.rs:1766`, `add.rs:236`, `add.rs:886`) pass `None, None` for the new params — zero behavioral change at those sites. Cloud-create passes constructed values.

**(2) Extract inner helper** `compute_body_chunks(content) -> Result<BodyChunks>` returning `{ content_hash, chunks_packed }`. `build_ingest_payload` is refactored in place to call it (replacing five inline lines with a single call). Cloud-mode update calls `compute_body_chunks` directly to populate the body trio on `ResourceUpdateRequest`. One source of truth for chunk + hash extraction; two callers (the existing payload builder, the new update path) converge.

### Frontmatter-construction unification

Today, local-mode `temper resource create --type session` and the other per-doctype creators each build their initial `ManagedMeta` inline before serializing it to YAML for the file write. Cloud-mode needs the same `ManagedMeta` shape but skipping the YAML and the file. To avoid drift between local and cloud, extract a typed builder:

```rust
// crates/temper-cli/src/actions/frontmatter.rs (new)
pub fn build_managed_meta_for_create(args: NewResourceArgs<'_>) -> ManagedMeta;
```

Each per-doctype creator (`actions::session::create`, `actions::task::create`, `actions::goal::create`, `actions::research::create`, `actions::concept::create`, `actions::decision::create`) is migrated to call this builder. Local-mode then serializes the returned struct to YAML for the file. Cloud-mode passes the same struct directly into `build_ingest_payload`. After this lands, exactly one function knows what a session's, task's, goal's, etc., initial `managed_meta` looks like.

### `temper resource create` — cloud branch

In each per-doctype handler under `commands/resource.rs::create_*`, add `match VaultState` at the top:

- **`Cloud`**:
  1. Resolve body source per the resolution rules below.
  2. Call `frontmatter::build_managed_meta_for_create(...)` for the typed managed_meta.
  3. Call `build_ingest_payload(body, title, context, doc_type, None, Some(managed_meta), None)` → `client.ingest(payload)` → `POST /api/ingest`.
  4. Print canonical id and slug to stdout in the same JSON shape local-mode emits.
- **`Local`**: existing flow, now calling `build_managed_meta_for_create` then writing the YAML+body file and `push_one_resource`-ing as before.

### `temper resource update` — cloud branch

In `commands/resource.rs::update`, add `match VaultState` at the top:

- **`Cloud`**:
  1. Resolve `<slug>` to resource id via `client.show(slug)` (uses the existing API-routed `show` from Session A).
  2. Build a partial `ManagedMeta` from CLI flags — only fields the user passed are `Some`; everything else stays `None`.
  3. Build a partial `open_meta: Option<Value>` from `--tags`, `--relates-to`, `--references`, `--depends-on`, `--extends`, `--preceded-by`, `--derived-from`, `--aliases` — only keys the user passed are present.
  4. Resolve body source per the rules below. If a body is supplied, call `compute_body_chunks(content)` and attach the trio.
  5. Construct `ResourceUpdateRequest` (partial), call `client.update_resource(id, req)` → `PATCH /api/resources/{id}`.
  6. Print the updated slug and new `content_hash` to stdout.
- **`Local`**: existing flow unchanged.

### `temper sync run` — cloud guard

In `commands/sync.rs::run`, add an early `match VaultState` before any vault resolution:

- **`Cloud`** → return error with the exact message: `"cloud mode has no local vault to sync — use 'temper resource create' and 'temper resource update' directly. To sync, switch to local mode."`
- **`Local`** → existing flow.

### `temper-client` additions

`crates/temper-client/src/client.rs` gains:

```rust
pub async fn update_resource(
    &self,
    id: ResourceId,
    req: &ResourceUpdateRequest,
) -> Result<ResourceRow, ApiError>;
```

`client.ingest()` should already exist (used by the sync push path). The plan stage verifies and adds it if missing.

### Body-source resolution

Resolution order, first match wins, applies to both cloud-mode `create` (when stdin/file provides a non-template body) and cloud-mode `update`:

1. `--body @<path>` — read file contents; ignore stdin.
2. `--body -` — read stdin explicitly. Errors if stdin is a TTY.
3. Implicit: stdin if non-TTY (auto-detection via `IsTerminal`); else doctype template (create only) or no body (update only).

The implicit path covers the agent-canonical flow: `temper resource show <slug>` → modify body in memory or to a temp file → `cat | temper resource update <slug> --stage done`. The explicit `--body @<path>` covers the case where an agent has produced a file as part of regular work and wants to ingest its contents directly.

### Invariants enforced in the cloud-mode write paths

1. Under `VaultState::Cloud`, no write code path calls `resolve_vault()`, `Manifest::*`, or any disk-write helper. Read-only access to disk (debounce cache, `--body @<path>` file reading) is allowed but never authoritative.
2. `--type` and `--context` parse identically across both modes.
3. Output shape is mode-agnostic — callers piping `temper resource create | jq .id` get the same JSON in both modes.
4. All chunk/hash/payload computation flows through `actions::ingest`. No module under `commands/` or `actions/` may compute `content_hash` or pack chunks independently.

## E2E Coverage

A new file `tests/e2e/tests/cloud_writes_test.rs`, modeled on the existing `publish_tail_test.rs` harness. All tests drive `TEMPER_VAULT_STATE=cloud` + `TEMPER_AUTH_PATH` via the `E2eTestApp` primitive — no vault dir on disk for the path under test.

| Test | Asserts |
|------|---------|
| `cloud_create_session_round_trip_via_show` | Cloud `temper resource create --type session --title "..."` posts to `/api/ingest`; a second cloud-mode session against the same DB pool retrieves the resource via `temper resource show <slug>` and recovers the original body + managed_meta. |
| `cloud_update_meta_only_partial_managed_meta` | Cloud `temper resource update <slug> --stage done` ships `ResourceUpdateRequest` with `managed_meta: Some({stage: "done", ..})` and no body trio; server merges over stored, untouched fields preserved. |
| `cloud_update_body_and_meta_in_one_request` | Cloud `cat new-body.md \| temper resource update <slug> --stage done` posts a single PATCH carrying both the body trio and managed_meta. Resource's `body_hash` and `managed_hash` both change in one round-trip. |
| `cloud_update_body_only_no_managed_meta` | Stdin body without managed-meta-mutating flags → managed_meta omitted on the wire (`None`), only body trio sent. Stored managed_meta untouched. |
| `cloud_update_chunk_dedupe_skips_unchanged` | Send body where `content_hash` matches stored — server short-circuits, no chunk insert/rewire. Verified via `kb_chunks` row-count assertion before/after. |
| `cloud_sync_run_redirects_with_message` | `temper sync run` under cloud mode returns the exact error string. |
| `cloud_list_returns_remote_only_resources` | Cloud `temper list` returns server rows including resources never pulled to a vault — already shipped under Session A; this is a regression-guard reaffirmation. |
| `local_mode_create_unchanged_after_helper_refactor` | Existing local-mode publish-tail test re-run after the helper consolidation lands, asserting bit-for-bit identical wire payload. Safety net for the `build_managed_meta_for_create` extraction. |

## Skill & Docs Guidance

Three guidance surfaces need updating so future agent sessions do not reach for the now-obsolete sync-push idiom:

- **`CLAUDE.md`** — add a "Cloud mode operations" subsection under "Key Patterns" covering: the show-edit-cat idiom as the canonical body-edit flow; `--body -` and `--body @<path>` as explicit alternatives; `update` implies push (no separate `sync push` needed in cloud mode); the "files-on-disk are derivative" invariant.
- **`/Users/petetaylor/.claude/skills/temper/reference.md`** — document cloud-mode versions of `create` and `update` alongside local. Note that `sync run` errors in cloud mode and what to do instead.
- **`/Users/petetaylor/.claude/skills/temper/subagent-guidance.md`** — add one principle: "When the active vault is in cloud mode, write paths route directly through `resource create` / `resource update` over the API. Do not invoke `sync push` / `sync run` — these will error. Body edits use the show-edit-cat idiom."

## Stale Plan Cleanup

Delete `docs/superpowers/plans/2026-04-19-unit-b-2-cloud-mode-dispatch.md` — superseded by the 2026-04-22 reframe and now fully obsolete. After deletion, grep for stragglers:

```
grep -rn "2026-04-19-unit-b-2-cloud-mode-dispatch" docs/ crates/ tests/ packages/
```

Resolve any hits. Likely only back-references in older session notes; those stay as historical record but get a one-line clarification at the top noting the plan has been superseded.

## Acceptance Criteria

- `cargo make check` passes (fmt, clippy with `-D warnings`, machete, biome).
- `cargo make test-all` passes (unit + integration across all features).
- `cargo make test-e2e` passes including all new cases in `cloud_writes_test.rs`.
- `cargo sqlx prepare --workspace -- --all-features` produces a clean cache after the expanded `update()` SQL.
- `cargo make generate-ts-types` produces a clean diff matching the new `ResourceUpdateRequest` shape; `packages/temper-ui` compiles against it.
- `! test -f crates/temper-cli/src/commands/resource_cloud.rs` — the parallel module never returns.
- Cloud `temper resource create --type session --title "T" --context temper <<<"body"` produces a resource readable via `temper resource show T` from a separate cloud-mode invocation against the same database.
- Cloud `temper resource update <slug> --stage done` succeeds with managed_meta-only payload; stored managed_meta merges (untouched fields preserved); `body_hash` unchanged.
- Cloud `cat body.md | temper resource update <slug> --stage done` succeeds with both body trio and managed_meta in a single PATCH; `body_hash` and `managed_hash` both update.
- Cloud `temper sync run` returns the exact redirect message.
- Local-mode `temper resource create` and `temper resource update` produce bit-for-bit identical wire payloads to pre-refactor (verified by the regression test).
- `docs/superpowers/plans/2026-04-19-unit-b-2-cloud-mode-dispatch.md` is removed; no references remain in the codebase.
- `CLAUDE.md`, `reference.md`, `subagent-guidance.md` updated; CLAUDE.md update is its own commit per `feedback_keep_claudemd_current`.

## Out of Scope

- **Cloud-mode resource delete + delete unification** — split out as task `2026-04-27-unify-resource-delete-cloud-first-explicit-only-manifest-cleanup`. Sequenced after Part 3B merge. Cloud-mode users still cannot delete at the end of Part 3B; that gap is owned by the new task.
- **`PUT /api/resources/{id}` with `--clear-meta`** — field-clearing semantics. Forward-compat noted above.
- **`If-Match` / `prev_content_hash`** — concurrency safeguards. Wire format extensible.
- **temper-core auth consolidation (Scope B)** — already deferred per `project_temper_core_auth_consolidation_deferred.md`.

## Risks & Mitigations

- **Helper consolidation drift**: extracting `build_managed_meta_for_create` and `compute_body_chunks` is real refactor surface in shipped code. The `local_mode_create_unchanged_after_helper_refactor` regression test pins the wire output bit-for-bit; the migration is gated behind it.
- **Schema regen sequencing**: `ResourceUpdateRequest` expansion regenerates utoipa OpenAPI and ts-rs TypeScript bindings. The plan stage orders SQL prepare → ts regen → UI compile so a partial state never lands.
- **`feature = "embed"` gate**: `build_ingest_payload` and `compute_body_chunks` are gated behind `embed` today. Cloud-mode write paths inherit this constraint — the temper binary must be built with `embed` for cloud writes to compile. This matches the existing local-mode constraint; no new requirement.
