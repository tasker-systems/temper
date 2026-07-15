# Context ownership transfer — bind a context to a team

- **Date:** 2026-07-15
- **Status:** Design — draft for review
- **Goal:** Teams in Temper: usable multi-user collaboration surface (temper resource `019f25d6-e1a9-7360-8a35-6bdf8ef53940`), reopened
- **Task:** Context ownership transfer — bind a context to a team (`019f6398-be41-7581-a2bf-4d9cb478583f`)
- **Sibling precedent:** [2026-07-03 Resource ownership reassignment](2026-07-03-resource-ownership-reassignment-design.md) (the full-stack shape this mirrors — but see "Not event-sourced" below for where it deliberately diverges)

## Motivation — the Job To Be Done

A person starts a **personal** context (`@me/my-project`). The work becomes a team
effort. They want to bring that context to their team.

The prior Teams goal shipped one half of this — a personal context can be **read-shared**
to a team (`share_context` → `kb_team_contexts`, read-reach only). It did **not** ship a
way to hand the context *over*: there is no operation that changes a context's owner. This
spec covers that operation.

### The governing design stance (Cole, 2026-07-15)

> A personal context is, by design, personal. Letting people **read** it is fine. But
> **authorship** is precisely the purpose of a team — so if you want people to **write**
> into a shared context, that context must be **bound to a team**.

This makes context ownership transfer the **single, deliberate path to shared
authorship**. There is no per-user write-grant on a personal context (that surface is a
locked non-goal — see the T-D design task `019f6399-3c96…`). Read → `share_context`.
Write → own it as a team, via this transfer (or by creating the context team-owned in the
first place, which already works via `resolve_create_owner`).

### The data model already supports the destination

`kb_contexts` ownership is **polymorphic**: `owner_table VARCHAR CHECK (owner_table IN
('kb_profiles','kb_teams'))`, `owner_id UUID`
(`migrations/20260624000001_canonical_schema.sql:159`). Team-owned contexts are a
first-class, already-exercised shape (`create_context --owner +team`). What is missing is
only the operation to move an **existing** context to a team owner. The elegant
consequence (below): once `owner_table` flips to `'kb_teams'`, the **existing** authz
predicates make the team able to author — no new visibility code.

## What flips, and why the rest is free

Transfer is an **in-place** change to the two owner columns of a single `kb_contexts`
row — same `id`, same resources homed in it, same edges, same cogmap bindings:

```sql
UPDATE kb_contexts SET owner_table = 'kb_teams', owner_id = $team WHERE id = $context;
```

Everything downstream is already wired:

- **Team members can now author.** `context_authorable_by_profile`
  (`migrations/20260712000010_context_read_predicates.sql:171`) has a team-owned arm:
  direct membership in the owning team (active team, role ∈ {owner, maintainer, member})
  ⇒ may author the context's resources via the container-write cascade in
  `can_modify_resource` (`migrations/20260712000020_can_modify_active_floor.sql:65`).
  Watchers stay read-only. **No new predicate is written** — flipping the owner row is
  sufficient.
- **Team members can now read.** `contexts_readable_by`
  (`…context_read_predicates.sql:84`) team-owned arm admits the context to every member
  (and, for read, enclosing teams up the DAG).
- **The transferrer keeps access.** Authorization (below) requires the caller to be an
  owner/maintainer of the target team, so they are a member — after transfer they retain
  read+write via membership. No self-lockout is possible.

### Explicit non-goals (→ handed to the T-D safety/design task `019f6399-3c96…`)

- **Resource owner/originator do not move.** Every resource keeps its own
  `kb_resource_homes.owner_profile_id` and `originator_profile_id` (a resource owner is
  FK-bound to `kb_profiles` and *cannot* be a team). This op moves the **container**; the
  team gains authorship through the container-write cascade, not by re-owning each
  resource. The residual-access questions that follow — the original author's permanent
  `originator` read+write floor, surviving per-resource `kb_access_grants`, whether to
  bulk-reassign resource owners to a team member on transfer — are **T-D**.
- **No per-user context write-grant surface.** Locked non-goal (the stance above);
  recorded in T-D so nobody wires up `kb_access_grants(subject='kb_contexts', can_write)`
  as a "convenience."
- **Cogmap bindings are untouched.** A context transfer does not alter `kb_team_cogmaps`
  or any cogmap-homed resource. The team↔cogmap boundary is governed separately.

## Event-sourced — recorded in the ledger

Ownership transfer is a **consequential, auditable act**, so it is written to the event
ledger like every other mutation — a full mirror of the sibling `resource_reassign`
(new `context_reassigned` event type + payload + `SeedAction` + projector + replay
wiring). The ledger record is the point: it preserves who moved the context, from which
owner to which, and when — history the plain `UPDATE` would throw away.

**Why this is safe even though contexts are otherwise non-evented.** `context_service`'s
`create`/`share`/`unshare` are plain, un-evented writes, and `kb_contexts` is an
**`INPUT_TABLE`** in replay — "non-projected input tables, copied verbatim into the replay
namespace" (`replay.rs:80`), *not* a projection rebuilt from events (unlike
`kb_resource_homes`, which is why *resource* reassign must be evented). The two facts
compose cleanly:

- **At append time**, the `context_reassign` SQL fn appends the event and its projector
  performs the actual `UPDATE kb_contexts SET owner_* = to_owner`. This is the single
  write path — no plain-UPDATE-plus-sidecar divergence.
- **At replay time**, `kb_contexts` is restored **verbatim** from the snapshot (already at
  the transferred owner), and then the `context_reassigned` projector re-applies the same
  owner — an **idempotent no-op**. `kb_contexts` is not in the projection-diff set (it is
  an input), so replay equivalence is unaffected. A replay-roundtrip test pins this.

The event type is seeded with `payload_schema = NULL` (mirroring `resource_reassigned`),
which keeps it out of the published-schema `TYPED_EVENT_NAMES` invariant.

> This supersedes the "not event-sourced" position of an earlier draft: the right test is
> not "does replay *require* it" but "is this act worth a ledger record" — and an
> ownership transfer is. `kb_contexts` being an input table makes the evented projector a
> safe idempotent-on-replay mutation, not a conflict.

## Layering (service-direct authz, event via the writes layer)

Authorization is service-direct — the `reassign_service` precedent: the service owns the
gate and enforces it before any write. The write itself routes through the substrate
**writes layer**, which fires the `context_reassigned` event (append + project). No
`Backend`-trait change.

New `context_service::reassign`, beside `share`/`unshare` in
`crates/temper-services/src/services/context_service.rs`, reusing that file's helpers:

```
pub async fn reassign(
    pool, caller: ProfileId, context_id: Uuid, to_team_id: Uuid,
) -> ApiResult<ReassignContextOutcome>
```

1. **Auth before writes** — the two-sided gate is *exactly* `can_share`'s shape (the
   target team becomes the new owner rather than a share recipient), so reuse it directly:
   `is_system_admin` bypass; refuse `is_gating_team` target; `can_manage(target_team)` AND
   `caller_administers_context(context)`. All three helpers already exist
   (`context_service.rs:369-420`).
2. **Existence** — `ensure_context_and_team_exist` (existing helper) → clean 404.
3. **Read the current owner** — for the event's `from_owner_*` audit fields and the
   idempotency check.
4. **Idempotent no-op** — if the context is already owned by `to_team_id`, return
   `reassigned: false` without emitting.
5. **Slug-collision guard** (new; see below) — 409 Conflict if the target team already
   owns a context with this slug (a service pre-check; the projector's `UNIQUE` is the
   backstop).
6. **The write** — resolve the emitter (`writes::resolve_emitter(pool, caller, "web")`)
   and fire `writes::reassign_context_with(pool, context, from_owner, to_owner, emitter,
   ctx)`, which invokes the `context_reassign` SQL fn (append `context_reassigned`
   anchored to `('kb_contexts', context_id)`, then project the owner change). Return the
   new `owner_ref` (the `+team-slug` decorated form, same `CASE` expression `create` uses).

Substrate additions (all mirroring the `ResourceReassign` precedent):

- `events.rs`: `EventKind::ContextReassigned` (+ `as_canonical_name`/`from_canonical_name`
  `"context_reassigned"` arms); `SeedAction::ContextReassign { context, from_owner_table,
  from_owner_id, to_owner_table, to_owner_id, emitter }` + `event_type()` arm; a
  `Fired::Context(ContextId)` variant + the fire arm issuing `SELECT
  context_reassign($1,$2,$3,$4)`.
- `payloads.rs`: `ContextReassigned { context_id, from_owner_table, from_owner_id,
  to_owner_table, to_owner_id }` (owner is polymorphic → carry table+id for both ends;
  the `scenario-schema` `JsonSchema` derive regenerates `tests/scenario_schema.rs`'s
  snapshot).
- `replay.rs`: classify `ContextReassigned` as a non-content mutation (the `None` arm at
  `replay.rs:179`) + a projection-dispatch arm calling `_project_context_reassigned`.
- `writes.rs`: `reassign_context_with` (and `_in_tx` if a bulk path is ever needed; single
  suffices for v1).
- **Additive migration** — `INSERT INTO kb_event_types (name, payload_schema,
  schema_version) VALUES ('context_reassigned', NULL, 1) ON CONFLICT DO NOTHING;` plus the
  paired `_project_context_reassigned` (UPDATE owner, `RAISE` if the row is missing) and
  `context_reassign` (append-then-project, envelope-anchored to the context) SQL fns.
  Purely additive → safe under additive-only-on-`main`.

### Slug uniqueness across the owner boundary — the one genuinely new edge

`kb_contexts` has `UNIQUE (owner_table, owner_id, slug)`. A personal context's slug is
unique under *the person*; after transfer it must be unique under *the team*. If the
target team already owns a context with the same slug, the raw `UPDATE` violates the
constraint (an opaque 500).

**Recommendation: reject with 409 Conflict** and a message naming the collision, rather
than silently re-slugging. A `+team/slug` handle is an addressable reference; silently
mutating it on transfer would break existing references and surprise. The user renames
(or we expose a `--slug` override) and retries. Pre-check:

```sql
SELECT EXISTS(SELECT 1 FROM kb_contexts
              WHERE owner_table='kb_teams' AND owner_id=$team AND slug=$slug) ...
```

*(Alternative considered: re-slug via `next_unique_context_slug` under the new owner, as
`create` does on name collision. Rejected for v1 — silent handle churn. Revisit if the
409 proves annoying in practice.)*

## Direction scope

| Direction | v1? | Notes |
|---|---|---|
| **personal → team** | ✅ ships | The literal JTBD. `owner_table` `'kb_profiles'`→`'kb_teams'`. |
| team → team | ➖ trivial add | Same gate (`caller_administers_context` covers a team-owned source: `can_manage` the *current* owning team). Include if cheap; otherwise fast-follow. |
| team → personal (un-bind) | ❌ deferred | Reversal strips the whole team's authorship — larger blast radius, needs its own authz story (owner-only? target = caller's own profile?). Decide in a follow-up; **v1 is one-way**, which is a known limitation (reversibility follow-up noted below). |

The service signature takes a target **team**; if team→team lands in v1 it needs no
signature change (the source's current owner is read from the row). team→personal would
need a distinct entry point and is out of scope.

## API surface

System-access-gated router (`routes.rs`, default-deny data tier), beside the existing
context share/unshare routes.

| Endpoint | Job | Body | Auth |
|---|---|---|---|
| `POST /api/contexts/{id}/reassign` | bind context to team | `{ to_team_id: Uuid }` | two-sided `can_share`-shaped gate |

Thin handler (extract `AuthUser`, one service call, return the ack) — the
`handlers/reassign.rs` / `handlers/contexts.rs` shape.

Wire types (`temper-core/src/types/context.rs`, ts-rs + utoipa derives):
- `ReassignContextRequest { to_team_id: Uuid }`
- `ReassignContextOutcome { context_id: Uuid, owner_ref: String, reassigned: bool }`

> **OpenAPI/SDK churn:** new response DTOs restale `openapi.json` + the temper-rb gem +
> temper-ts `schema.ts`. Run `cargo make openapi` and stage all three (the drift gates
> compare against git). Per CLAUDE.md this is the three-artifact ritual.

## CLI

`temper context transfer <context-ref> <team-ref>` — `<context-ref>` via the standard
trailing-UUID `parse_ref`; `<team-ref>` via `resolve_team_id` (accepts slug / `+slug` /
decorated / UUID, `actions/cogmap.rs:113`). New client method
`contexts().reassign(context_id, &ReassignContextRequest)`.

(Verb choice: `transfer` reads better than `reassign` at the context grain and won't
collide with the existing `temper resource reassign` / `temper team reassign`. Open to
`context reassign` for symmetry — minor.)

## MCP tool

`transfer_context` in `crates/temper-mcp/src/tools/contexts.rs`, beside `share_context`:
input `{ context: Uuid, to_team: Uuid }`, delegating to `context_service::reassign`. This
is the agent-first path for the operation and lands **with** this task; broader team
lifecycle over MCP (create team / add member) remains the separate Seq 21 task
(`019f6399-13e2…`) — without it, an agent can transfer into a team it already manages but
can't yet create one over MCP.

## Testing

- **Service unit tests** (`#[sqlx::test]`, `test-db`), mirroring the `context_service`
  test module:
  - personal→team flips `owner_table/owner_id`; a target-team **member** (role member+)
    now passes `can_modify_resource` on a resource homed in the context; a **watcher**
    does not; a **non-member** sees no change;
  - the transferrer (target-team owner/maintainer) retains read+write post-transfer;
  - authz matrix — each independently ⇒ `Forbidden`: caller not a context administrator;
    caller not owner/maintainer of the target team; target is the gating/root team;
  - idempotent no-op when already owned by the target team (no event emitted);
  - **slug collision** under the target team ⇒ `Conflict` (409), owner unchanged;
  - `is_system_admin` bypass;
  - a `context_reassigned` event is emitted with correct `from_owner_*`/`to_owner_*`.
- **Replay-roundtrip** (`artifact-tests`) — a context transferred to a team survives
  snapshot → namespace reset → replay: the restored `kb_contexts` row + the
  `context_reassigned` projector converge on the team owner (idempotent). Extend the
  substrate replay suite.
- **E2E** (`tests/e2e`, `test-db`) — one test driving `temper context transfer` through
  CLI → client → API → DB, asserting `owner_ref` becomes `+team` and a team member can now
  author a resource in the context (via `can_modify_resource`). Embed features not
  required — run `cargo make test-e2e`.
- Regenerate sqlx caches after SQL changes (`cargo sqlx prepare --workspace …` + the
  per-crate services/api prepares if test SQL is touched).

## Grounding references

- `kb_contexts` polymorphic owner: `migrations/20260624000001_canonical_schema.sql:159`.
- `context_service` create (non-evented) / share / unshare / **the reusable authz
  helpers** `can_share` + `caller_administers_context` + `ensure_context_and_team_exist`:
  `crates/temper-services/src/services/context_service.rs:262-484`.
- Team-owned authoring arm (why the flip is sufficient):
  `migrations/20260712000010_context_read_predicates.sql:171` (`context_authorable_by_profile`);
  container-write cascade `migrations/20260712000020_can_modify_active_floor.sql:65`.
- Team-owned read arm: `…context_read_predicates.sql:84` (`contexts_readable_by`).
- Auth helpers: `team_service::{can_manage, role_on_team}`; `access_service::{is_system_admin, is_gating_team}`.
- Full-stack template (handler/route/client/CLI/wire-type shape): the resource-reassign
  siblings — `handlers/reassign.rs`, `routes.rs`, `temper-core/src/types/reassign.rs`,
  `commands/team.rs`; MCP share template `tools/contexts.rs` (`share_context`).

## Open follow-ups (not this task)

- **Residual access & transfer completeness** → T-D (`019f6399-3c96…`): originator floor,
  grant sweep, resource-owner reassignment on transfer, in-flight-ingest race.
- **Reversibility** (team → personal un-bind) — its own authz story; v1 is one-way.
- **team → team** transfer — include in v1 if cheap (same gate), else fast-follow.
