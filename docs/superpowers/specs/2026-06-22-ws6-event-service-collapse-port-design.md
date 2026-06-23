# WS6 Event Service — collapse port + `/api/events?` deprecation

A focused design beat carved out of the WS6 migration-endgame collapse
(`2026-06-22-ws6-migration-endgame-design.md`). The endgame's §"Coincident code
changes" lists `event_service` among the **raw-pool leak services** that must be
rewritten onto the substrate shape coincident with the rename — but, unlike
`graph_service` (a mechanical port), `event_service` turned out to hide a genuine
design fork: the substrate `kb_events` table no longer has the columns its current
API is built on. This spec resolves that fork minimally so the collapse can proceed,
and explicitly names the larger rethink it defers.

---

## Problem — `kb_events` is no longer `kb_resource_events`

The legacy `event_service` reads direct-FK columns on `public.kb_events`:
`profile_id`, `device_id`, `kb_context_id`, `resource_id`, `event_type_id`,
`payload`, `created` (verified: `migrations/20260522000001_event_ledger_unification.sql:156-160`).

The canonical substrate `kb_events` (`schema-artifact/01_schema.sql:287-310`) **dropped
all of them** except `event_type_id`, `payload`, `created`. The Arc-1 event model is
emitter-entity / producing-anchor / payload:
- actor: `emitter_entity_id → kb_entities.id → kb_entities.profile_id`
- provenance: `producing_anchor_table ∈ ('kb_contexts','kb_cogmaps')` + `producing_anchor_id`
- resource linkage: **no relational column** — it lives inside `payload`/`references` JSONB.

So after the rename the service's queries `42703` (undefined column) and the events
surface breaks. It needs a real decision, not a search-and-replace.

### The two consumers — only one survives

| Endpoint | Service fn | Purpose | Fate |
|---|---|---|---|
| `GET /api/events?resource_id=&event_type=` | `list_visible` | resource/type **history feed** | **DEPRECATE** — built on the retired shape; **unused** (never invoked by UI/CLI/agent in practice) |
| `GET /api/events/{kb_context_id}/cursor` | `latest_event_id_for_context` | per-context **sync cursor** ("did anything change?") | **KEEP + port** — small, used by `temper-client` sync |

Reconstructing the history feed onto an un-indexed `payload->>'resource_id'` scan would
bake a fiction (a resource-centric feed) onto an event model that deliberately moved
away from it. Since nothing consumes it, we remove it rather than port it.

---

## Decision

1. **Port the cursor** (the only kept behavior) onto the substrate, minimally.
2. **Deprecate the `/api/events?` list query** and its entire consumer chain.
3. **Defer** the real "what is the event feed *for*" rethink to a separate future arc.

### 1. Cursor port

`latest_event_id_for_context(pool, profile_id, kb_context_id)` keeps its signature and
its `EventCursorResponse { latest_event_id: Option<Uuid> }` return. New query:

```sql
SELECT e.id
  FROM kb_events e
 WHERE e.producing_anchor_table = 'kb_contexts'
   AND e.producing_anchor_id   = $2          -- the context id
 ORDER BY e.occurred_at DESC
 LIMIT 1
```

**Why this resolves the context correctly.** `_event_append` stamps `producing_anchor`
to the mutated resource's *home anchor* (`02_functions.sql:748-750`, `:778` — from
`payload#>>'{home,table}/{home,id}'`). A context-homed resource's create/update/property/
delete events therefore carry `producing_anchor=(kb_contexts, C)` — exactly what the
cursor needs. (A *cogmap*-homed resource's events anchor to `(kb_cogmaps, …)` and fall
outside a context cursor — correct: the cursor is per-context, and this is the
intention-vs-RBAC distinction in miniature.)

**Visibility gate — context-ownership.** The substrate has `resources_visible_to(profile)`
and `can_modify_resource(profile, resource)` but **no context-access helper**
(`02_functions.sql:121,160`). `kb_contexts` is owned (`owner_table/owner_id ∈
('kb_profiles','kb_teams')`, `01_schema.sql:107-108`). The cursor gates on the caller
**owning** the context — directly (`owner_table='kb_profiles' AND owner_id=$1`) or via
team membership (`owner_table='kb_teams'` joined through `kb_team_members`). This
deliberately drops the old *resource-level* event visibility (the part that has no
substrate home and is exactly what the future arc must redesign). The cursor returns an
opaque "latest change timestamp" for a context the caller owns; an un-owned context id
yields `None`, leaking nothing.

> `occurred_at` (not `created`) is the order key — it is the replay-stable event time
> (`01_schema.sql:306`); `created` is wall-clock insert time.

### 2. Deprecate the list query — exact removal scope

Remove the whole `list_visible` consumer chain (the plan pins exact lines):
- **API:** `event_service::list_visible` (+ its four query variants); `GET /api/events`
  route (`routes.rs:86`) and `handlers::events::list`.
- **MCP:** the `list_events` tool + its registration (delegates to `event_service`).
- **temper-client:** the events `list` method (`temper-client/src/events.rs`); keep the
  cursor call.
- **temper-core:** `EventListParams` and `EventRow` (+ their `openapi.rs` registrations
  and `ts-rs`/exports). **Keep** `EventCursorResponse`.
- **CLI:** any `temper events`/list command surface that calls the client list (audit +
  remove; the plan greps for it).
- **Tests:** `tests/e2e/tests/events_test.rs` (delete or reduce to a cursor-only test);
  the events assertion in `mcp_round_trip_test.rs`.

Keep: `GET /api/events/{kb_context_id}/cursor` → `handlers::events::cursor` →
`latest_event_id_for_context`, and `EventCursorResponse`.

### 3. Surface-parity gate consequence

The collapse acceptance gate (`tests/e2e/tests/surface_parity_next.rs`) tested **nine**
read surfaces; surface (9) was `GET /api/events?resource_id={id}` asserting the created
resource appears in its events feed. That surface is being deprecated, so:
- **Drop surface (9)** from the parity gate → it becomes an **eight-surface** gate.
- The cursor is not a resource-resolution surface (it returns an event id, not the
  resource), so it does not belong in the "every surface sees the one resource" gate.
  Add a **separate, small cursor test**: a `NextBackend` write into a context bumps that
  context's `latest_event_id`; an un-owned context returns `None`.

This *narrows* the gate's claim honestly — it no longer asserts a feed we deleted.

---

## Deferred — the future "intentional event model" arc (named, not designed here)

A separate spec + tasks, enabled by but out of scope for the collapse:

- **`originating_event_id` FK direction.** Add an FK from `kb_resources` (and eventually
  `kb_content_blocks`) to the event that created/mutated it — pk/fk over the current
  "ledger introspects its own payload" (`payload->>'resource_id'`) direction. The right
  shape, but the cursor does not need it, so the collapse changes nothing about the
  events *table*.
- **Events-as-mutations addressing content blocks** — content blocks become directly
  event-addressable, so the feed can answer "how did this block change."
- **Event visibility as *intention*, distinct from RBAC.** A cogmap-homed concept may be
  RBAC-visible while not all edges/events addressing it are — *which* events a viewer
  sees is an intention decision, not a resource-visibility derivation. This is the real
  replacement for the deprecated history feed; it deserves its own design.

---

## Testing

- **Cursor unit/integration:** context-owned write bumps `latest_event_id`; cogmap-homed
  write does **not** bump a context cursor; un-owned context → `None`; empty context →
  `None`. (e2e, `test-db`.)
- **Parity gate:** `surface_parity_next.rs` reduced to eight surfaces, green post-collapse
  (the endgame's acceptance criterion, amended).
- **Compile gate:** removing `EventListParams`/`EventRow` must leave the workspace
  compiling — the removal of every consumer (API/MCP/client/CLI/core/tests) is one atomic
  change (cross-crate; the pre-commit hook gates whole-workspace clippy).

---

## References

- Parent: `docs/superpowers/specs/2026-06-22-ws6-migration-endgame-design.md`
  (§"Coincident code changes" — this spec resolves its `event_service` leak-service bullet).
- Substrate event model: `schema-artifact/01_schema.sql:287-348` (kb_events / kb_entities /
  kb_invocations), `02_functions.sql:748-783` (`resource_create` / `_event_append`).
- Legacy shape: `migrations/20260522000001_event_ledger_unification.sql:156-160`.
- Sibling design beat (the other leak service, mechanical): `graph_service` rewrite in the
  endgame coincident-changes manifest.
