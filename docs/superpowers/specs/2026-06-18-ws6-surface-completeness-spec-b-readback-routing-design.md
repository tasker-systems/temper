# WS6 surface-completeness port — Spec B: readback routing

**Date:** 2026-06-18
**Status:** Design / spec. Second of two specs for the WS6 surface-completeness port; lands on the same branch (`jct/ws6-surface-completeness-addressing-collapse`) as **Spec A (addressing-model collapse)** and ships as a single A+B PR.
**Parent strategy:** `docs/superpowers/specs/2026-06-16-ws6-flip-readiness-strategy.md` (§ "Surface-completeness port", item 2).
**Companion spec:** `docs/superpowers/specs/2026-06-17-ws6-surface-completeness-spec-a-addressing-collapse-design.md` (the A/B seam is defined there, §1).

## What this is

Spec A collapsed the *addressing model* across every surface (decorated-ref / UUID, one resolver) and closed native-id write addressing. It deliberately left two read surfaces routed only to the **legacy** `public.*` backend, naming them as Spec B's job:

1. **`by_uri`** — the scoped `(owner, context, doc_type, ident)` resolver.
2. **MCP `get_resource` / `list_resources`** — the read tools that return the enriched resource shape.

Spec B finishes the surface-completeness port so that, under `flag=next`, these two surfaces are answered **without depending on `public.*`** — the bar the flip-readiness strategy sets ("no flip-with-a-gap"). It does so very differently for the two surfaces, because grounding this session corrected the task's original framing for **both**.

## Scope corrections established during brainstorming (both narrow/clarify the task)

The backlog task and an in-code comment (`crates/temper-mcp/src/tools/resources.rs:412-418`) described Spec B as "route `by_uri` and MCP relationship-enrichment through `temper_next` readback." Grounding the actual code showed two misstatements:

### Correction 1 — `by_uri` resolves by **slug**, which is unportable to `temper_next` by construction

`resource_service::resolve_by_uri` (`crates/temper-api/src/services/resource_service.rs:388-427`) resolves a resource by `ResolveByUriParams.ident` — a **slug or UUID** — scoped by `(owner, context, doc_type)`, querying the `vault_resources_browse` view in `public.*`. Slug is **§7-dissolved** in `temper_next` (`origin_uri` is the substrate addressing key; there is no slug column). The read-selector already documents this (`crates/temper-api/src/backend/read_selector.rs:6-9`): *"`by_uri` is NOT covered … slug is §7-dissolved … it stays on legacy under `next`."*

So a faithful `by_uri`-by-slug Next arm is **impossible**, not merely deferred — the lookup key does not exist in the substrate. The endpoint's only **live** caller is the CLI session→task edge link (`crates/temper-cli/src/commands/resource.rs:307-340`): on `temper resource create --type session --task <slug>`, it resolves the user-typed task slug → resource id via `resolve_by_uri`, then asserts the `advances` edge. The two endpoint tests (`crates/temper-api/tests/resources_by_uri_test.rs`) pass a **UUID** ident; only the live caller passes a slug.

The resolution is the post-migration addressing model itself: identity is the canonical UUID (Spec A's decorated ref strips the slug half and resolves trailing-UUID-only); the slug is a presentation nicety. The CLI **already holds** the task's identity — `find_task` (`crates/temper-cli/src/actions/task.rs:131`) returns a `TaskInfo` built from a `list_meta` row whose top-level `row.id` carries the resource id (currently discarded; `task_info_from_meta` consumes only `managed_meta`). So the slug→id network resolution is for an id already on hand. **Retire the caller; do not port the surface.**

### Correction 2 — MCP "enrichment" is **meta + context + doctype**, not relationship/edge reads

`EnrichedResource` (`crates/temper-mcp/src/tools/resources.rs:178-196`) has **no edge or relationship fields**. The "enrichment" the two tools perform is:

- `enrich_resources` (`resources.rs:244-263`) → `meta_service::get_meta_batch` (`crates/temper-api/src/services/meta_service.rs:72-109`) — managed/open meta from `public.kb_resource_manifests`.
- `build_enriched` (`resources.rs:202-236`) → `context_service::get_visible` (context name) + `doc_type_service::get_name_by_id` (doctype name).

All keyed by re-minted ids / context-ids that don't exist in `public` after synthesis — **that** is the real blocker, not relationships. And every one of these fields is **already reconstructed by `readback`** (`crates/temper-next/src/readback/mod.rs`): `readback::resource_row` (`mod.rs:426-499`) yields `title`/`origin_uri`/`is_active`/`context_name`/`doc_type_name`/`owner_handle`/workflow fields; `readback::meta` (`mod.rs:326-370`) yields the managed/open split + `doc_type`. **No relationship-read porting is needed** — the port assembles `EnrichedResource` from `readback` outputs.

## 1. The two parts

### Part 1 — Retire the slug-keyed `by_uri` caller (ships **live**, backend-agnostic)

This is not gated behind `next-backend` / `flag=next`. It is correct on the legacy backend today (the id comes from `list_meta`, which works on `public.*`) and is a prerequisite for `next` (where slug resolution is impossible). It is the addressing-model completion, finishing Spec A's narrative.

- Add `id: ResourceId` to `TaskInfo` (`crates/temper-cli/src/actions/types.rs:5`), populated from the `list_meta` `row.id` that `load_tasks` (`crates/temper-cli/src/actions/task.rs:50-95`) already fetches. `task_info_from_meta` (`task.rs:108`) gains the id as a parameter (it lives on the list row's top level, not in `managed_meta`).
- CLI session→task link (`resource.rs:307-340`): assert the `advances` edge with `task_info.id` directly. **Delete the `resolve_by_uri` round-trip** (`resource.rs:321-326`) — one fewer network call on session create.
- `by_uri` endpoint (`crates/temper-api/src/handlers/resources.rs:102-110`), `resource_service::resolve_by_uri`, the `temper-client` method (`crates/temper-client/src/resources.rs:104-120`), and the route/openapi entries: **left as legacy/test-only** — not routed to Next, not deleted. After this change nothing live resolves by slug; the endpoint persists only for its two tests and `relationship_write_test.rs`'s verification helper. Deleting it is a clean follow-up (vestigial post-Spec-A), explicitly **out of scope** for this spec.

**Net:** nothing live resolves by slug; the substrate's slug-less addressing is honored end-to-end; the session→task link is one round-trip cheaper.

### Part 2 — MCP `get_resource` / `list_resources`: full-fidelity Next routing (**gated**)

Behind the `next-backend` feature + in-DB `flag=next`, **gated OFF** by default (consistent with chunk 4b/4c). Today both tools are hard-wired to the legacy services — they are the only resource **reads** in MCP that do not route through `backend_selection` (writes already route via `select_backend`; MCP `search` routes via `read_selector::search_select` — `crates/temper-mcp/src/tools/search.rs:15`). This is the seam to extend.

**Unification refactor — `build_enriched` becomes backend-agnostic.** Today `build_enriched` (`resources.rs:202-236`) resolves context/doctype names via service calls internally. Refactor it to take resolved `context_name` + `doc_type_name` + `managed_meta` + `open_meta` as inputs — pure assembly, no internal DB calls. Each backend arm supplies them. This is the single assembly point both backends feed (CONFORMS to the "shared helpers at boundaries" code-quality rule).

- **`get_resource` (by id):** branch on `svc.api_state.backend_selection`.
  - Legacy: unchanged (resolve names via `context_service`/`doc_type_service`, meta via `get_meta_batch`).
  - Next: `readback::resource_row(id)` already returns `title`/`origin_uri`/`is_active`/`context_name`/`doc_type_name`; `readback::meta(id)` returns managed/open; `readback::body(id)` when `include_content=true`. `created`/`updated` are stamped read-time `now()` (synthesis-collapsed → §9 non-invariant; `ResourceRowParity` deliberately omits them — `readback/mod.rs:417-424`); `slug = None` (§7-dissolved). Assemble via the refactored `build_enriched`.
- **`list_resources` (filters: `context_name`, `doc_type_name`):** branch on `backend_selection`.
  - Legacy: unchanged.
  - Next: a **new batched readback projection** in temper-next — `readback::enriched_list` — returning, per row, `{re_minted_id, origin_uri, title, is_active, context_name, doc_type, stage, mode, effort, managed_meta, open_meta}`, visibility-scoped via `temper_next.resources_visible_to($principal)`, with `WHERE` filters on `context_name` + `doc_type` applied in SQL. One query, no N+1 (the existing `readback::list` — `mod.rs:244-290` — is the starting shape; it must additionally carry the resource id + `context_name` via the `kb_resource_homes → kb_contexts` join `resource_row` already uses, the per-row managed/open meta reconstruction, and the two filters). Assemble `Vec<EnrichedResource>`.
- **Exposure boundary:** add `read_selector` functions in temper-api (matching the `search_select` precedent at `crates/temper-api/src/backend/read_selector.rs:73-83`) that return the per-backend enrichment-ready data. `EnrichedResource` assembly stays in temper-mcp (the type lives there); temper-mcp stays at the "delegate to temper-api services" boundary. The Next arms of these selector functions are `#[cfg(feature = "next-backend")]`; without the feature they gate with the same `NotImplemented` as the existing selector arms.

**Fidelity note (why full, not §9-floor):** `context_name` is a §9 **invariant** (carried verbatim by `resource_row`) and doctype is a reconstructable property, so faithful context/doctype **filtering** under `next` is achievable — unlike chunk-4b's CLI `list` Next path (`readback::list`), which intentionally stayed at the floor (ignores filters, empty `context`, `ResourceSummary` carries 6 fields). Leaving `list_resources` at that floor would silently ignore its two filters under `next` — a behavioral gap "no-flip-with-a-gap" would force a revisit before chunk-5. Spec B closes it now.

## 2. Testing

- **Part 2 (gated):** e2e parity under the `next-backend` feature — `get_resource` and `list_resources` answered with `flag=next` match the legacy answers at the **§9 invariant floor** (ids, slug, and `created`/`updated` timestamps are non-invariants and excluded from the assertion), using the established mutate-`public`-after-synthesis control to prove the answer comes from `temper_next` not `public`. `list_resources` filter behavior (context_name + doc_type) asserted explicitly under `next`. The §9-floor parity is the same bar 4b proved for `list`/`get_meta`/`get_content`/`search`.
- **Part 1 (live):** a CLI/e2e test that `temper resource create --type session --task <slug>` asserts the `advances` edge to the correct task resource id **without** calling `resolve_by_uri` (assert the resulting edge's target id; the slug input still resolves through `find_task`'s local matching).
- **Build/run gotchas (carried from Spec A, recurring):**
  - `cargo make test-e2e` and bare `cargo nextest run -p temper-api` **hang** at test-list enumeration (the `mcp_*`/`relationship_*` and api bin targets block on `--list`). Run e2e via `cargo test -p temper-e2e --features test-db --test <file>` (libtest lists internally).
  - The e2e crate is **standalone** (`tests/e2e/`, not in the workspace), so `cargo make check` does not compile its `next-backend`-gated tests. Run them explicitly with `cargo test -p temper-e2e --features test-db,next-backend` and `SQLX_OFFLINE=true` (temper-next queries target the `temper_next` namespace; live validation against the `public` dev DB fails with type-inference errors).
  - `next-backend` builds need `SQLX_OFFLINE=true` generally; regenerate the temper-next cache with `cargo make prepare-next` after changing readback SQL, and `cargo make prepare-e2e` after changing e2e test SQL.

## 3. Non-goals

- **Relationship / graph enrichment in MCP** — does not exist (`EnrichedResource` has no edge fields); not invented here.
- **A `by_uri` Next arm** — unportable (slug §7-dissolved); the caller is retired instead.
- **Deleting the `by_uri` endpoint / `resolve_by_uri` service** — vestigial after Part 1, but a clean separate follow-up; kept legacy/test-only here.
- **Vault-projection-filename rename** (Adjudication-5 identity-out clause) — deferred to the flip with Spec A; behind it, whether the local vault projection survives as a feature at all.
- **Access-fidelity re-derivation under lenses, deployed-adapter `next-backend` enable, the chunk-5 flip** — downstream of this PR.

## 4. Why this is the right cut

Spec B is the last surface-completeness item before flip-readiness: with it, every read surface either answers from `temper_next` under `flag=next` (`list`/`show`/`get_meta`/`get_content`/`search` from 4b; `get_resource`/`list_resources` here) or has been retired as an addressing vestige (`by_uri`-by-slug). The remaining flip prerequisites — deployed-adapter feature enable, then the chunk-5 hard cutover — are no longer blocked on a surface that can only be answered from `public.*`. The A+B PR delivers the whole "surface-completeness port" item of the flip-readiness strategy as one coherent change.

## Connections

- Companion: `docs/superpowers/specs/2026-06-17-ws6-surface-completeness-spec-a-addressing-collapse-design.md` (A/B seam, §1).
- Parent strategy: `docs/superpowers/specs/2026-06-16-ws6-flip-readiness-strategy.md`.
- Adjudication (identity contract, §7 slug fate): `docs/superpowers/specs/2026-06-12-ws6-convergence-delta-adjudication-design.md`.
- Readback floor + `ResourceRowParity` invariants: `crates/temper-next/src/readback/mod.rs`.
- Goal: `substrate-kernel-to-cognitive-map` (temper context), workstream 6.
