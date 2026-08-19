# Resource ownership reassignment (event-sourced)

- **Date:** 2026-07-03
- **Status:** Design — approved, pending implementation plan
- **Goal:** Teams in Temper: usable multi-user collaboration surface (temper resource `019f25d6-e1a9-7360-8a35-6bdf8ef53940`), scope task #3 "Resource ownership transfer" (`019f25d9-f040-7951-8fc8-3af35a791f5f`)
- **Supersedes:** the task's original offer/accept `kb_transfers` design (retired below)

## Motivation — Jobs To Be Done

The Teams goal re-homed the *purpose* of the retired I7 task. I7 bundled two resource
mechanics: `access_level` **sharing** and ownership **transfer**. The sharing half was
already replaced by context-shares (`kb_team_contexts`) + capability grants
(`kb_resource_access`). This spec covers what remains: transfer.

A JTBD pass narrowed transfer to exactly two situations that justify mutating a
resource's owner:

1. **Offboarding** — a member leaves; the resources they own that belong to the team's
   shared knowledge need a surviving owner, or they are orphaned (only the departed
   principal held the ownership access-floor). Bulk, admin-driven.
2. **Mis-attribution** — a resource carries the wrong owner (created under the wrong
   account). Correct it. Single-resource.

### Explicit non-goals

- **Authorial handoff** ("it's your baby to maintain now") and **personal → canonical
  promotion** ("I contribute my personal note as the team's"). These are *social*
  changes fully served by **sharing** (capability grants + context-shares). We do not
  fix a social problem with an ownership mutation.
- **Offer / accept / decline handshake.** The two-step `kb_transfers` lifecycle was
  built for consensual gifting (authorial handoff) — a cut job. Neither surviving job
  is consensual: offboarding is unilateral (the owner is gone); mis-attribution is a
  correction. Reassignment is therefore a single **authz-gated** action, not a
  handshake.
- **Copy-fork / supersession** (new resource + lineage edge + fold original). Attractive
  for authorial handoff, but wrong for both surviving jobs: offboarding must keep the
  *same* resource identity so every edge and cogmap membership stays intact;
  mis-attribution is a field correction, not a lineage event. Duplicating content +
  embeddings and shredding inbound edges serves no surviving job.
- **Cogmap-homed resources.** Reassignment touches **context-homed** resources only. A
  resource homed in a cogmap is a map interior, governed by join/telos, not by personal
  ownership. This is the team↔cogmap boundary: reassignment never re-owns a cogmap's
  interior. **Enforcement:** the admin path + bulk scope exclude it structurally (their
  queries require an `anchor_table='kb_contexts'` join); the owner path and the write
  primitive do not, so it is closed explicitly — a `reassign_service` guard rejects any
  non-`kb_contexts` home with `BadRequest`, and the `resource_reassign` SQL fn `RAISE`s as
  a backstop for any future caller of the writes layer.

## Core primitive: in-place, event-sourced owner change

Ownership lives as `owner_profile_id` on the resource's single `kb_resource_homes` row
(`migrations/20260624000001_canonical_schema.sql:282`). `resources_visible_to` and
`can_modify_resource` both grant the home's `owner_profile_id` **and**
`originator_profile_id` an automatic read/modify floor
(`migrations/20260624000002_canonical_functions.sql:125,164`). Ownership is thus the
*administrative identity* of a resource; everything else (letting other people read or
write) is already served by grants + context-shares.

Reassignment mutates `owner_profile_id` **in place** — same resource id, all edges and
cogmap memberships intact. `originator_profile_id` is **never** touched: it is immutable
provenance. A consequence: after reassignment the ex-owner keeps their originator
access-floor. This is correct for mis-attribution (you still authored it) and irrelevant
for offboarding (the departed principal is deactivated).

The mutation is **event-sourced**, like every other resource mutation — it is not the
service-direct/no-event path that team-membership provisioning uses. `kb_resource_homes`
is projected from the event stream (`replay.rs` rebuilds it), so an owner change must be
an event or a replay would clobber it.

### New event: `ResourceReassigned`

Sibling to the existing `ResourceRehomed` (rehome = anchor move; reassign = owner move —
distinct taxonomy tags). Payload:

```rust
// temper-substrate payloads.rs — mirrors ResourceRehomed
pub struct ResourceReassigned {
    pub resource_id: ResourceId,
    pub from_profile_id: ProfileId,  // audit: who it was taken from
    pub to_profile_id: ProfileId,
}
```

`from_profile_id` is recorded for a complete audit trail; the projector writes only the
new owner. Additions, all mirroring the `resource_rehomed` precedent:

- `temper-substrate/src/events.rs`: `EventKind::ResourceReassigned` + string mapping
  (`"resource_reassigned"`, events.rs:42/65/93); `SeedAction::ResourceReassign { resource,
  from_profile, to_profile, emitter }` + `event_type()` arm (events.rs:253/304) + `Fired`
  variant + fire logic (events.rs:~811).
- `temper-substrate/src/payloads.rs`: `ResourceReassigned` struct (payloads.rs:~430).
- `temper-substrate/src/replay.rs`: classify as a non-authored mutation (replay.rs:158,
  alongside `ResourceRehomed`) + projection dispatch arm calling
  `_project_resource_reassigned` (replay.rs:347).
- **Additive migration** (new file, no changes to existing tables) — paired SQL fns
  mirroring `resource_rehome` (`migrations/20260624000002_canonical_functions.sql:1109`):

  ```sql
  CREATE FUNCTION _project_resource_reassigned(p_event uuid, p_payload jsonb)
  RETURNS uuid LANGUAGE plpgsql AS $$
  DECLARE v_resource uuid := (p_payload->>'resource_id')::uuid;
  BEGIN
      UPDATE kb_resource_homes SET owner_profile_id = (p_payload->>'to_profile_id')::uuid
          WHERE resource_id = v_resource;
      IF NOT FOUND THEN RAISE EXCEPTION 'resource_reassign: resource % has no home', v_resource; END IF;
      RETURN v_resource;
  END; $$;

  CREATE FUNCTION resource_reassign(p_payload jsonb, p_emitter uuid)
  RETURNS uuid LANGUAGE plpgsql AS $$
  DECLARE v_ev uuid; v_anchor_tbl text; v_anchor uuid;
  BEGIN
      -- envelope anchor = the resource's CURRENT home (it does not move)
      SELECT anchor_table, anchor_id INTO v_anchor_tbl, v_anchor
        FROM kb_resource_homes WHERE resource_id = (p_payload->>'resource_id')::uuid;
      IF v_anchor IS NULL THEN RAISE EXCEPTION 'resource_reassign: resource has no home'; END IF;
      v_ev := _event_append('resource_reassigned', p_emitter, v_anchor_tbl, v_anchor, p_payload);
      RETURN _project_resource_reassigned(v_ev, p_payload);
  END; $$;
  ```

  The migration is purely additive (two new functions), so it is safe under
  additive-only-on-`main` auto-deploy.

### Layering (service-direct, event via the writes layer)

Reassignment is a **team-role-authorized administrative** write. Its authorization is
identical in shape to `invitation_service` / `team_service` (`role_on_team` /
`can_manage`), and those team-domain services are **service-direct** — handlers call
the service directly; the service composes persistence — an established carve-out from
the Backend-trait dispatch that the cognitive resource-CRUD surface uses. Reassignment
follows that precedent, and satisfies "event like everything else" by routing its write
through the substrate **writes layer** (which fires the event), not by inflating the
`Backend` trait with a bespoke-auth op.

- **`temper-services::services::reassign_service`** (new, mirrors `invitation_service`):
  - owns authorization (below) and enforces it *before* any write (auth-before-writes);
  - resolves the acting emitter via `writes::resolve_emitter(pool, caller_profile,
    "web")` (the API surface marker — the entry point is always temper-api);
  - **single:** `reassign_resource(pool, caller, resource_id, to_profile_id)` — auth,
    idempotency no-op check (current owner already `to`?), then
    `writes::reassign_resource_with(pool, resource, from, to, emitter, act)`.
  - **bulk:** `reassign_team_resources(pool, caller, team_id, from, to)` — auth +
    service-direct scope read (owned-AND-context-shared set), then one transaction
    looping `writes::reassign_resource_in_tx(&mut tx, …)` per resource.
- **`temper-substrate::writes`** (new fns, mirroring `update_resource_with` /
  `delete_resource_with` + their `_in_tx` variants): `reassign_resource_with` /
  `reassign_resource_in_tx`, each firing `SeedAction::ResourceReassign` via `fire_with`,
  which invokes the `resource_reassign` SQL fn (append + project).
- Handlers (`temper-api`) are thin: extract `AuthUser`, call the service, return the row
  — exactly the `handlers/invitations.rs` shape. No `Backend` trait change.

## Authorization

Two authorized paths. Idempotency (re-reassign to the current owner) is a service-layer
no-op: read current `owner_profile_id`; if already `to_profile_id`, return success
without emitting.

### Owner path (self-correction)
The current `owner_profile_id` may reassign their own resource to **any valid profile** —
their prerogative over a resource they own (mis-attribution self-fix).

### Admin path (offboarding + admin-assisted mis-attribution)
A caller may reassign resource `R` to profile `P` **iff** there exists a team `T` where:

- **(a)** `can_manage(T)` — caller is owner/maintainer of `T`
  (`team_service::can_manage` / `role_on_team`);
- **(b) from-reach:** `R` is homed in a context shared to `T`
  (`kb_team_contexts` — `R`'s `kb_resource_homes.anchor` is a `kb_contexts` id shared to
  `T`); and
- **(c) into-reach:** `P` is a member of `T` (`kb_team_members`).

(b) + (c) keep an admin's power inside their team's boundary: they cannot reassign
resources they do not govern, nor hand a team resource to a non-member.

## API surface

All routes in the **system-access-gated** router (`routes.rs`, the default-deny data
tier — unlike invitation accept/decline, no pre-access bootstrap concern exists here).

| Endpoint | Job | Body | Auth |
|---|---|---|---|
| `POST /api/resources/{id}/reassign` | mis-attribution | `{ to_profile_id }` | owner path OR admin path |
| `POST /api/teams/{id}/reassign` | offboarding | `{ from_profile_id, to_profile_id }` | admin path (`can_manage(T)`) |

### Bulk semantics
`POST /api/teams/{id}/reassign` reassigns, from `from_profile_id` to `to_profile_id`,
**every resource owned by `from_profile_id` and homed in a context shared to team `{id}`**
(constraint (b)), provided `to_profile_id` is a member of `{id}` (constraint (c)). Each
affected resource emits one `ResourceReassigned` event inside **one transaction**. An
empty match set is a success (no-op). The response reports the count / ids reassigned.

Wire types (`temper-core/src/types/`):
- `ReassignResourceRequest { to_profile_id: Uuid }` (single).
- `BulkReassignRequest { from_profile_id, to_profile_id }` — **reused** from the existing
  `transfer.rs` (survives the retirement below).

## CLI

- `temper resource reassign <ref> --to <profile-uuid>` (single) — `<ref>` resolved by the
  standard trailing-UUID `parse_ref`; PATCH-free dedicated action calling the new client
  method.
- `temper team reassign <team> --from <profile-uuid> --to <profile-uuid>` (bulk).

Recipient/from identified by **profile UUID**, matching the existing
`team add-member` / `set-role` / `remove-member` commands (team.rs:121-284). No
get-profile-by-handle endpoint exists; `@handle` resolution is a separable later nicety
and out of scope.

New `temper-client` methods: `resources().reassign(id, &req)` and
`teams().reassign(team_id, &req)`.

## Retiring the dead offer/accept substrate

Dropping the handshake orphans the offer/accept types. Split by risk:

- **This PR** — delete the now-dead Rust types in `temper-core/src/types/transfer.rs`:
  `ResourceTransfer`, `TransferRequest`, `TransferStatus` (and their tests + the
  `types/mod.rs` re-export). Keep `BulkReassignRequest` (reused). Rename the file to
  `reassign.rs` and add `ReassignResourceRequest`. Pure code, non-schema, zero-risk —
  this task is what orphans them.
- **Task #6 ("Retire dead access/transfer remnants")** — the `DROP TABLE kb_transfers`
  + `DROP TYPE transfer_status` migration. A destructive `DROP` must not ride an
  auto-deployed feature PR (additive-only-on-`main`); it belongs in #6's operator-run
  cleanup migration alongside the `AccessLevel` / `TeamResource` vault-era corpses. The
  table is inert (never written, nothing references it), so deferring the drop costs
  nothing.

## Testing

- **Service unit tests** (`#[sqlx::test]`, `test-db`), mirroring `invitation_service`
  tests:
  - owner-path reassign flips `resources_visible_to` / `can_modify_resource` from old
    owner to new owner;
  - originator retains access after reassignment (floor unchanged);
  - admin-path allowed only when (a)+(b)+(c) all hold; each of the three violated
    independently ⇒ `Forbidden`;
  - non-owner, non-admin ⇒ `Forbidden`;
  - re-reassign to current owner is an idempotent no-op;
  - bulk reassign hits exactly the owned-AND-context-shared set (a resource owned by
    `from` but *not* shared to `T` is untouched; a shared resource owned by someone else
    is untouched);
  - bulk into a non-member `to_profile` ⇒ `Forbidden`.
- **Replay test** — a `ResourceReassigned` event replays to the correct
  `owner_profile_id` (extend the substrate replay-roundtrip suite).
- **E2E** (`tests/e2e`, `test-db`) — one test driving `temper resource reassign` through
  CLI → client → API → DB and asserting the owner change is visible via
  `resources_visible_to`. Embed features not required.

## Grounding references

- `kb_transfers` table + `transfer_status` enum (to retire in #6):
  `migrations/20260624000001_canonical_schema.sql:112,393-405`.
- `kb_resource_homes.owner_profile_id`: `…schema.sql:282`.
- Visibility/modify read owner from homes:
  `migrations/20260624000002_canonical_functions.sql:125,164`.
- Rehome precedent (event + paired SQL fns): payload
  `temper-substrate/src/payloads.rs:430`; SeedAction/EventKind
  `temper-substrate/src/events.rs:253,304`; replay dispatch
  `temper-substrate/src/replay.rs:158,347`; SQL `…canonical_functions.sql:1109-1130`.
- Dead types: `temper-core/src/types/transfer.rs`; re-export `types/mod.rs:82`.
- Invitations template (service/handler/route/client/CLI shape): `invitation_service.rs`,
  `handlers/invitations.rs`, `routes.rs:40-113`, `commands/team.rs`.
- Auth helpers: `team_service::{can_manage, role_on_team}`.

## Open follow-ups (not this PR)

- `DROP TABLE kb_transfers` + `DROP TYPE transfer_status` → task #6.
- `@handle` → profile-id resolution (a `GET /api/profiles/{handle}` endpoint) as a CLI
  ergonomics nicety, shared with the `team` member commands.
