# WS6 Service-Layer Collapse — scoping audit

The Task-8 atomic flip (`2026-06-22-ws6-endgame-collapse-code.md`) assumed only
`graph_service` + `event_service` were "raw-pool leak services" needing substrate ports.
Execution proved otherwise: collapsing temper-api to one schema requires **every** temper-api
`sqlx` macro to resolve against the substrate, and the **entire legacy service layer** still
targets the legacy `public` shape (`kb_context_id`, `kb_profiles.slug`, `kb_contexts.kb_owner_table`,
`kb_resource_edges`, `kb_doc_types`, `kb_resource_manifests`, legacy `kb_events` columns) — backing
**live** identity/access/context/sync/edge surfaces with no substrate replacement wired. This is the
design's own stated risk realized ("the split leaked to ≥4 read paths… assume other scaffolding
assumptions are similarly unaudited"); the surface-parity gate only covered resource reads, so it
never exercised contexts/access/profiles/sync.

This doc is the per-service disposition audit (4-cluster parallel read, every verdict `file:line`/psql-cited)
that the collapse design must fold in before Task 8 is executable.

## Substrate ground truth (psql, `temper_next`)
- `kb_resources` = `(id, title, origin_uri, body_hash, is_active, created, updated)` — **no** `kb_context_id`/`slug`/`kb_doc_type_id`. Context membership via `kb_resource_homes(resource_id, anchor_table, anchor_id, owner_profile_id)`; doc_type is a `kb_properties` row (`property_key='doc_type'`); slug §7-dissolved.
- `kb_profiles` = `(id, handle, display_name, system_access, created)` — `slug`→`handle`; the canonical graft re-adds only `email`+`preferences`.
- `kb_contexts` = `(id, owner_table, owner_id, slug NOT NULL, name, created)` — legacy `kb_owner_table`/`kb_owner_id` renamed; new NOT-NULL `slug`; no `updated`.
- `kb_edges` (NOT `kb_resource_edges`): `(…, source_table, source_id, target_table, target_id, edge_kind, polarity, label, is_folded, …)`.
- `kb_events` event-sourced: `(id, event_type_id, emitter_entity_id, topic_id, producing_anchor_table/id, correlation_id, payload, …)` — **no** `profile_id`/`device_id`/`scope_id`; events fire via `_event_append` inside the 02_functions mutations.
- `resources_visible_to` is **1-arg** in substrate (legacy was 3-arg). **No `contexts_visible_to`.**
- **Absent from substrate:** `vault_resources_browse`, `kb_resource_manifests`, `kb_current_chunks`, `kb_doc_types`, `kb_resource_edges`; functions `unified_search`/`graph_search`/`graph_resource_edges`/`resolve_event_type`/`sync_diff_for_device`/`kb_resource_uri`/`has_system_access`/`is_system_admin`.
- Substrate write path = `NextBackend` (→`DbBackend` post-collapse) + `temper_next::writes` → `02_functions` (`resource_create/update/delete/rehome`, `relationship_assert/fold/retype/reweight`, `property_set`). Substrate resource read path = `temper_next::readback` (`list`/`enriched_list`/`resource_row`/`body`/`meta`/`fts_search`/`vector_search`).

## Disposition map

| Service | Disposition | Size | Notes |
|---|---|---|---|
| `graph_service` | PORT (in Task 8) | — | Query 2 → `kb_edges`; Query 1 → ported `graph_subgraph_nodes` (done, T6) |
| `event_service` cursor | PORT (in Task 8) | — | `latest_event_id_for_context` → substrate `kb_events` (T8 step 4) |
| `access_service` | **GRAFT-SATISFIED + code fixes + event-port** | M | Tables graft-provided; BUT canonical graft is **missing `has_system_access`/`is_system_admin` functions** (add them, dropping the `kb_teams.is_active` predicate that no longer exists); drop `kb_teams.is_active` filter (:96); fix `kb_team_members` insert cols (drop `id`/`joined_at`/`invited_by_profile_id`); **PORT** `emit_join_request_event` to `_event_append` |
| `profile_service` | **PORT** | M | `slug`→`handle` throughout (+`generate_profile_slug`→`_handle`); reshape `temper-core::Profile` (drop `avatar_url`/`vault_config`/`is_active`/`updated`); rewrite `kb_contexts` auto-insert (`owner_table`/`owner_id`/`slug`); auth_links half graft-satisfied |
| `context_service` | **PORT** | M | Needs a substrate **context-visibility predicate** (`contexts_visible_to` doesn't exist — derive inline: owner OR `kb_team_contexts` share); resource-count via `kb_resource_homes`; `owner_table` rename + populate `slug`; **context-event decision** (drop, or `_event_append` with `producing_anchor='kb_contexts'`) |
| `sync_service` | **RETIRE** | S–M | All 3 fns rest on dead tables (`kb_resource_manifests`/`kb_device_sync_state`/`kb_doc_types`) + dissolved 3-hash tier-split + missing fns; **no production client caller** — `temper pull` already uses the event-cursor + readback. Re-home the lone e2e audit test; drop 3 routes + `temper-core::types::sync` |
| `doc_type_service` | **RETIRE** | S | `kb_doc_types` dies → temper-core schemas; `get_name_by_id` already dead; re-route `list_all` (MCP doc_types tool) to enumerate the schemas |
| `search_service` | **RETIRE** | S | Substrate search = `readback::vector_search`/`fts_search` (wired in `next_impl::search`). FLAG: substrate path is FTS-**or**-vector, **no** blended scoring / graph-expand / `validate_params` dim-check — re-home if those are contractual |
| `relationship_service` | **RETIRE** (writes subsumed) | S | All writes → `NextBackend` + 02_functions (idempotent `relationship_assert`); `validate_assertion_label` is a trivial PORT (lift the non-empty rule); `rebuild_edge_projection`/`reproject_pending_for_resource` are legacy-replay harness — retire |
| `edge_service` | **SPLIT** | M–L | `list_resource_edges` (the one LIVE read, `/api/resources/{id}/edges`) → **PORT** over `kb_edges`+`edges_visible_to` (`graph_resource_edges`+`peer_slug` are legacy-only); frontmatter→edge derivation (`extract_and_upsert_edges`/`reconcile_edges`) → **product decision** (port into NextBackend or drop); `extract_declarations_from_resource` is a reusable pure kernel |
| `resource_service` | **SPLIT** | L | Read fns: repoint the **3 direct surfaces** (MCP resources-protocol `src/resources.rs`, HTTP `?meta_only=true`, MCP `enrich_resources`) to `readback` equivalents (`list_visible_meta` needs a NEW `read_selector` arm); `get_by_slug` already dead. Write fns (`update`/`delete`/`check_can_modify`) RETIRE with the Legacy `DbBackend`. `get_visible` has the highest blast radius (8 live sites) |
| `meta_service` | **RETIRE** (repoint first) | S | `get_meta` dead once Legacy arm drops; `get_meta_batch` survives only via MCP enrichment + meta_only-list — repoint to `readback`, then retire |

## Architecture finding — reads do NOT all funnel through `read_selector`
The Task-8 Step-6 correction (route reads via `readback`) is necessary but **incomplete**. Three live surfaces call legacy read services **directly**, bypassing the selector, and HTTP `show` + all writes go through the Backend trait (Legacy `DbBackend`):
- MCP **resources-protocol** (`temper-mcp/src/resources.rs`) → `resource_service::{list_visible,get_content,get_visible}` (distinct from MCP *tools*, which do route through `read_selector`).
- HTTP `?meta_only=true` (`handlers/resources.rs:74-76`) → `resource_service::list_visible_meta` (no selector arm).
- MCP `enrich_resources` (`tools/resources.rs:236` → `meta_service::get_meta_batch`), reached from `update_resource` on both backends.

So full retirement requires retiring the `read_selector` Legacy arms **and** the Legacy `DbBackend` **and** repointing these three direct surfaces.

## Canonical-layer-draft gaps (the draft is incomplete)
1. **Missing functions** `has_system_access`/`is_system_admin` — the draft grafts tables + `kb_profiles` columns but no functions. Add them (drop the `kb_teams.is_active` predicate).
2. **Local artifact does not incorporate the graft.** The graft is a live-cutover DDL referencing `LEGACY.` carry-over. For local collapsed dev/CI (and for `prepare-api` to succeed), the identity/infra tables + the re-added `kb_profiles` columns must be present in the loaded schema. Decision: fold the graft's DDL half into `schema-artifact/01_schema.sql` (data carry-over stays runbook-only), or a local-dev seed.

## Cross-cutting
- **Legacy event emission** in access/context/relationship/edge/ingest (`append_event_tx` / `insert_event_and_audit` / `resolve_event_type`) has no substrate landing; substrate fires `_event_append` from inside the mutations. Mostly already implemented in `temper_next::writes` — confirm parity, retire the api-side emitters, port the few genuine emitters (join-request, context-created if kept).
- **doc-type-by-UUID is dead.** `handlers/resources.rs:183` + `read_selector.rs:129` pass `kb_doc_type_id`; the wire must carry the doc-type **name** (substrate stores it as a property string; `NextBackend` already passes the name through).
- **Heavy test surface** (`edge_ingest_test`, `relationship_projection_test`, `tests/e2e/mcp_*`, `audit_test`) calls these services directly — rewrite/retire alongside.

## Open product decisions (block the design — see the session for the dispatched question)
1. **Frontmatter→edge derivation** (`extract_and_upsert_edges` create / `reconcile_edges` update): not in `NextBackend`. Port into the substrate create/update path, or retire the feature?
2. **Pending-slug forward-reference reprojection** (`reproject_pending_for_resource`): no substrate equivalent (slug §7-dissolved). If frontmatter edges are kept, this semantic is lost unless redesigned (e.g. by origin_uri/id).
3. **Create-time schema validation + doc-type defaults + system-field strip + body-hash dedup** (`validate_managed_meta`/`apply_*_defaults`/`strip_system_managed_fields`/`find_by_body_hash`): `NextBackend::create_resource` runs none of these today. Lift the pure helpers into the substrate create path, or accept the contract change?
4. **Search semantics**: accept the substrate's FTS-or-vector path (no blended scoring, no graph-expand, no embedding-dim validation), or re-home `compute_weights`/`validate_params`/graph-expand into `next_impl::search`?
5. **Context-created event**: drop it (contexts are infra, not cognition), or model it via `_event_append`?

## Staging implication
The collapse is not one atomic Task 8. It is: identity-graft-into-artifact + the per-service ports/retires above + the MCP/handler wire-shape changes + the test rewrites, landing so that after de-qualification every temper-api macro resolves against one schema and `prepare-api` regenerates a coherent cache. The atomic-commit constraint still holds for the final flip, but the ports can land as independent additive prep (like T6) ahead of it.
