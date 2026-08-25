# Context retirement — design

**Date:** 2026-08-25
**Branch:** `ani/context-delete`
**Supersedes:** the hard-delete design in PR #777
**Status:** approved for planning

---

## The problem

A context you no longer want has no exit. It stays in `temper context list`, stays
addressable, stays writeable, and keeps conferring read-reach to anyone it was shared
with. The only lever today is `temper context subscribe`/`unsubscribe`, which edits
`sync.subscriptions.contexts` in local config and has **no server effect** — it controls
what `temper pull` materializes, nothing else.

PR #777 proposed a hard delete guarded by a dependents check. The problem it names is
real; the mechanism is wrong, for a reason the schema states plainly (§2 below).

## What we are building

A context can be **retired**: it stops being visible, stops being writeable, and keeps
every row it ever homed. It can be **restored**. Retirement is reversible, un-evented,
and additive.

---

## 1. Grounding

Every claim below was printed from the live schema or read from the tree at
`ani/context-delete`. Excerpts, not narration.

### 1.1 `kb_contexts` has no lifecycle state

```
id                          uuid       not null  uuid_generate_v7()
owner_table                 varchar(64) not null
owner_id                    uuid       not null
slug                        text       not null
name                        text       not null
created                     timestamptz not null now()
shape_materialized_event_id uuid
telos_centroid              vector(768)
```

Constraints: `kb_contexts_pkey PRIMARY KEY (id)`,
`kb_contexts_owner_table_owner_id_slug_key UNIQUE (owner_table, owner_id, slug)`.

### 1.2 The read axis is one chokepoint behind three delegations

```sql
-- context_visible_to(p_principal, p_context_id)
SELECT context_readable_by_profile(p_principal, p_context_id);

-- context_readable_by_profile(p_profile, p_context)
SELECT EXISTS (SELECT 1 FROM contexts_readable_by(p_profile) c WHERE c.context_id = p_context);

-- contexts_readable_by(p_profile)
SELECT t.context_id FROM contexts_readable_by_teams(p_profile, <reachable teams>) t;
```

`contexts_readable_by_teams(p_profile, p_teams)` carries the only real logic — four arms:

1. personal context — `kb_contexts WHERE owner_table='kb_profiles' AND owner_id=p_profile`
2. context owned by an enclosing team — `kb_contexts WHERE owner_table='kb_teams' AND owner_id = ANY(p_teams)`
3. context shared to an enclosing team — `kb_team_contexts WHERE team_id = ANY(p_teams)`
4. explicit read-grant — `kb_access_grants WHERE subject_table='kb_contexts' AND can_read AND …`

Arms 1 and 2 select from `kb_contexts`. **Arms 3 and 4 do not** — they read
`kb_team_contexts` and `kb_access_grants` and never join the context row.

Only two database functions call it, both of which should inherit any floor placed there:

```
contexts_readable_by
resources_visible_to
```

### 1.3 The write axis is a second, separate chokepoint

`context_authorable_by_profile(p_profile, p_context)` — three arms: personal owner;
direct membership in the owning team with role in `('owner','maintainer','member')`
joined to `kb_teams t ON … AND t.is_active`; and `profile_explicit_grant(p_profile,
'write', 'kb_contexts', p_context)`.

Note the incumbent already floors on `kb_teams.is_active` in its team arm. This design
adds the same shape for the context's own flag.

### 1.4 Neither floor traps a resource

`can_modify_resource(p_profile, p_resource)` has four admitting arms. Only the last
routes through the context:

```sql
-- container-write cascade: whoever may author the home container may modify its nodes.
SELECT 1 FROM kb_resource_homes h
 WHERE h.resource_id = p_resource
   AND CASE h.anchor_table
         WHEN 'kb_cogmaps'  THEN cogmap_authorable_by_profile(p_profile, h.anchor_id)
         WHEN 'kb_contexts' THEN context_authorable_by_profile(p_profile, h.anchor_id)
         ELSE false
       END
```

The **first** arm is `kb_resource_homes h WHERE h.resource_id = p_resource AND
h.owner_profile_id = p_profile`. `resources_visible_to` has the same owner-first shape
across its six arms.

And re-homing write-gates the **destination only**
(`crates/temper-services/src/backend/db_backend.rs:1993-1995`):

```rust
if let Some(ctx_to) = cmd.move_to.as_ref().and_then(|m| m.context_to) {
    self.check_context_authorable(uuid::Uuid::from(ctx_to)).await?;
}
```

Consequence, load-bearing for this design: **flooring both predicates never traps a
resource from its own owner.** The owner keeps read and modify through the owner arms and
can move anything out of a retired context, because the source context is never consulted.

### 1.5 The incumbent: `kb_teams` soft-delete

`migrations/20260703000001_team_metadata_soft_delete.sql` is this feature, for teams.
Its header states the semantics we are copying:

> - DELETE sets `is_active = false`. All rows (members, grants, context-shares,
>   cogmap-joins, child-parent links) are PRESERVED — soft-delete is reversible
>   (recovery is a `is_active = true` DB write).
> - A soft-deleted team confers ZERO read-reach. Enforced at the two DAG primitives
>   below […] so every reachable-team branch inherits the exclusion.

The write half is a plain, un-evented `UPDATE`
(`crates/temper-services/src/services/team_service.rs:393-394`). No `team_deleted` event
type exists anywhere in `crates/` or `migrations/`. A soft-deleted team is also not
updatable — `team_service.rs:360` carries `AND is_active` in the update's `WHERE`, so the
freeze falls out of the same flag.

### 1.6 Why a hard delete could not ship

`kb_contexts` is a replay **input** table, restored verbatim
(`crates/temper-substrate/src/replay.rs:101-125`), and `kb_events` is one too. Both
context projectors raise on a missing row:

```
migrations/20260731000040_context_rename_fns.sql:48
    IF NOT FOUND THEN RAISE EXCEPTION 'context_rename: context % not found', v_context; END IF;

migrations/20260715000010_context_reassign_fns.sql:28
    IF NOT FOUND THEN RAISE EXCEPTION 'context_reassign: context % not found', v_context; END IF;
```

and `replay.rs:621-637` calls them unguarded. So `create → rename → hard delete → replay`
aborts. A soft-delete has no such problem: the flipped row rides in with the verbatim
restore, and no projector ever sees an absent context. **The property that made hard
delete unsafe is the same one that makes retirement safe.**

### 1.7 The additive rule

`DEPLOYING.md:68-72`:

> `additive` is a *definition*, not a vibe. The one question is: **does a binary that
> predates this migration keep working against the schema after it is applied?** If you
> cannot say yes, the migration is `shape-breaking`. Dropping something is shape-breaking

A `shape-breaking` migration halts the Vercel build (`temper-migrate --additive-only`) and
requires an operator-run cutover per target — temperkb.io and every enterprise
self-hosted site, each on its own cadence. This design stays `additive` and therefore
auto-deploys.

---

## 2. Design

### 2.1 Schema — one additive column

```sql
ALTER TABLE kb_contexts ADD COLUMN is_active BOOLEAN NOT NULL DEFAULT true;
```

`UNIQUE (owner_table, owner_id, slug)` is **not** touched. Declared `additive`: a
predating binary reads `kb_contexts` without the column and keeps working; every existing
row is born `true`.

### 2.2 Two floors, two `CREATE OR REPLACE`s

**Read** — inside `contexts_readable_by_teams`. Arms 1 and 2 take `AND c.is_active`; arms
3 and 4 gain an `EXISTS (SELECT 1 FROM kb_contexts c WHERE c.id = … AND c.is_active)`,
because they never join the context row. Because the other three read predicates are thin
delegations (§1.2), this single edit floors `context_visible_to`,
`context_readable_by_profile`, `contexts_readable_by`, `resources_visible_to`, and every
graph read that gates through them.

**Write** — inside `context_authorable_by_profile`, the same shape its team arm already
uses for `kb_teams.is_active`.

That is the entire enforcement surface. No new visibility function, and no second copy of
the four read arms anywhere.

### 2.3 Retire

`DELETE /api/contexts/{id}` — the verb teams already use for a soft-delete
(`handlers::teams::delete`). Gated by `ContextAdminAuthority`
(`crates/temper-services/src/authz/context_admin.rs:65-85`): administers the context, or
manages its owning team as owner/maintainer, or is an instance administrator. Auth runs
before any write.

One statement, un-evented, mirroring `team_service.rs:393`:

```sql
UPDATE kb_contexts
   SET is_active = false, slug = <mangled>
 WHERE id = $1 AND is_active
```

**The slug is mangled; the name is not.** `slug` is an address and may be lossy; `name` is
a display label and may not — the split rename's design already rests on. Mangling only
the address frees `@me/scratch` for immediate reuse (the outcome we want) while leaving
the `UNIQUE` constraint in place (the additive class we want). The retired row still reads
as `scratch` in a retired listing; only its address moves, to
`<slug>-retired-<short-id>`.

Nothing is swept and nothing cascades. Resources, homes, regions, edges, shares, grants
and connections all survive untouched. **There is no dependents guard** — retiring a
context that still homes things is the entire point of retiring rather than deleting.

Consequences, stated rather than discovered later:

- Your own resources stay visible and modifiable (§1.4) and can be moved out.
- A reader who reached those resources only through the container loses read and write.
- A `kb_connections` row homed here keeps a valid FK to a live row; it simply sits in a
  context nobody can address. Re-homing it is a separate concern, not gated by this.
- `kb_workflow_jobs.context_id` and `kb_team_contexts.context_id` both `ON DELETE
  CASCADE`, which now never fires — a further argument for soft over hard.

### 2.4 Restore

`POST /api/contexts/{id}/restore`, same gate, same un-evented shape. The address is
re-derived from the untouched `name` through `next_unique_context_slug`
(`crates/temper-services/src/services/context_service.rs:374`) — the incumbent `create`
already calls for exactly this collision, with its `notes` → `notes-2` auto-suffix.

If a new context claimed the original slug meanwhile, restore lands on the suffixed
address and **reports it in the response** rather than refusing. Handing back a different
address silently is the failure mode `rename` explicitly refuses; reporting it is not.

### 2.4.1 Addressing a retired context — the resolver, which is not free

Retirement moves a context out of reach of the machinery that addresses it, in two ways at
once, and both must be answered or `restore` cannot resolve its own argument:

1. The read floor (§2.2) hides it, and every CLI context verb resolves refs through
   `resolve_context_id_for_read`
   (`crates/temper-cli/src/commands/context_cmd.rs:360`), which gates on the read axis.
2. The slug was mangled (§2.3), so the ref the operator remembers — `@me/scratch` — no
   longer names the row even if they could see it.

So:

- **Retire's response carries the context id and the new mangled ref**, and the CLI
  prints both. An operator who retires something must leave with the handle they need to
  undo it; anything else makes retirement one-way in practice while claiming otherwise.
- **Restore resolves on the admin axis, not the read axis.** It accepts a UUID or the
  mangled ref, through the same admin-scoped door §2.5 builds for the retired listing —
  one door, two callers, no second copy of the ownership rule.
- **`context show` on a retired context admits on the admin axis too**, reporting
  `retired: true`. Listing a thing you cannot then inspect is an incoherent pair, and the
  admin axis is already the answer for the listing.

This is the one place the feature adds surface beyond two floors and two verbs, and it is
load-bearing rather than convenience: without it, `--retired` shows you a row you cannot
address, and restore is unreachable.

### 2.5 The retired listing, and the drift site it must not create

`temper context list --retired` rides the **admin** axis, not the read axis: you can only
see a retired context if you could have retired it. It adds no new *read*-visibility
function and does not widen the read gate — retired rows stay invisible to
`contexts_readable_by_teams` and everything downstream of it. What it does add is one
admin-scoped door, which `restore` and `context show` then reuse (§2.4.1) rather than each
growing their own.

The care point: `caller_administers_context`
(`crates/temper-services/src/services/context_service.rs:575`) is a **point** check over
one context id, and its team half is decided in Rust —
`team_service::role_on_team` plus `can_manage`, which is
`matches!(role, TeamRole::Owner | TeamRole::Maintainer)` at `team_service.rs:79-81`.
There is no SQL predicate for "teams I manage"; a `grep` for `manage` over `pg_proc`
returns nothing.

So a set-shaped SQL query would restate `can_manage` in a second language. Instead,
**Rust derives the manage-capable role list from `can_manage` itself and passes it as an
array parameter.** The SQL is parameterized by the rule rather than restating it, and
`can_manage` stays the single definition.

### 2.6 Surfaces

| Surface | Change |
|---|---|
| `DELETE /api/contexts/{id}` | Retire (soft). Reuses PR #777's route + handler. Response carries the id and the new mangled ref (§2.4.1). |
| `POST /api/contexts/{id}/restore` | New. |
| `GET /api/contexts?retired=true` | New filter, admin-axis scoped. |
| `GET /api/contexts/{id}` | Admits a retired context on the admin axis (§2.4.1); still `404` on the read axis. |
| `ContextRow` | New `retired: bool`, beside the existing computed `can_write`. |
| `temper-client` | `contexts().delete()` (kept), `contexts().restore()` (new). |
| `temper context delete <ref>` | Retire. Prints the id and the mangled ref. |
| `temper context restore <ref\|uuid>` | New. Resolves on the admin axis. |
| `temper context list --retired` | New. |
| OpenAPI, temper-rb, temper-ts | Regenerated; all changed codegen committed together. |

CLI output goes through `crate::format::render`, matching `resource delete`
(`crates/temper-cli/src/commands/resource.rs:1374`) rather than `output::success` — the
non-TTY contract is JSON, and an agent must be able to parse the result.

### 2.7 What happens to PR #777

**Survives:** the route and handler, the `ContextAdminAuthority` wiring, the existence
check and its `fetch_one`-on-`EXISTS` reasoning, the `temper-client` method, the CLI
command scaffolding, the `audit-elevation-claims.sh` claim-count bump, and the shape of
the e2e tests.

**Goes:** the dependents guard, its two count queries and their `.sqlx` entries,
`map_context_delete_err`, and the `409` responses. The stale root `.sqlx` deletion is
resolved by rebasing and regenerating, never by hand-merging.

**Corrected on the way through:** two doc comments in the PR describe machinery that does
not exist — a `23503` TOCTOU backstop for `kb_resource_homes`, which has no foreign key,
and a "cascade" for soft-deleted resources' home rows, which likewise does not exist.
Neither survives, because neither guard survives.

---

## 3. Testing

Witnesses authored during the build, each failing against the state its clause changes.

**The floors**
- A retired context vanishes from `context list`, from `context show <ref>`, and from
  `resources_visible_to` for a principal whose only reach was arm 3 (team share) or arm 4
  (explicit grant) — one isolated witness per arm, since a single test over a caller with
  several reaches cannot tell which arm closed.
- Authoring into a retired context is refused.
- The owner can still read, modify, and re-home their own resource out of a retired
  context. This is the anti-trap witness and must fail if either floor is over-applied.

**The verbs**
- Retire → the row survives with `is_active = false`; every homed resource, region, edge,
  share and grant is still present.
- Retire → `create` with the original name succeeds, proving the slug was freed.
- Restore → address re-derived; and restore-into-a-collision lands on the suffix and says
  so.
- Retire is idempotent-ish: a second retire of an already-retired context is a clean
  refusal, not a 500.

**Addressing (§2.4.1)** — the witnesses that keep retirement from being one-way in practice:
- Retire's response carries the id and the mangled ref, and the CLI prints both.
- `restore` resolves a retired context by UUID *and* by its mangled ref; resolving it by
  its **original** ref fails, which is the correct and documented behavior.
- `context show` on a retired context succeeds for an administrator with `retired: true`,
  and `404`s for a principal whose only reach was the read axis.

**Authorization** — the gap PR #777 would have inherited. Both its tests provision an
instance admin via `root_bootstrap_first_admin`, so no non-admin caller is ever exercised:
- a caller who may read but not administer gets `403`
- an unprivileged caller naming a foreign context gets the uniform `404`
- both for retire *and* restore

**Replay** — a round-trip over `create → rename → retire` completes. This is the witness
that the hard-delete design could not have passed (§1.6).

---

## 4. Out of scope

### Rejected

**A `temper-mcp` surface.** MCP will not inherit retire or restore. Retiring a context
implies relocating the resources homed in it, and that blast radius is beyond what the MCP
tool surface is intended to carry — it is a CLI-or-API-level concern. This is a reasoned
no, not a "later": if it is revisited, it should be revisited as a question about what MCP
is *for*, not as a gap in this feature.

**Local vault cleanup.** Retiring a context does not remove its projected `{context}/`
directory from the local vault. The vault is a read-only projection cache; a stale
directory is a cache miss, not state.

**Freeing the slug by relaxing the constraint.** Replacing `UNIQUE (owner_table,
owner_id, slug)` with a partial unique index `WHERE is_active` would free the slug more
directly, but it is `shape-breaking` under §1.7 — both by the explicit "dropping something
is shape-breaking" rule and on the merits, since a predating binary resolving `@me/scratch`
could then get two rows where it structurally assumed one. That buys an operator-run
cutover on every deployment target in exchange for a nicer retired-row address. Mangling
achieves the same user-visible outcome additively.

**An event for retirement.** No `context_retired` event type. Contexts are a replay input
table restored verbatim, so the flag rides in with the restore and an event would add a
projector with nothing to project. This matches teams, which emit nothing for their own
soft-delete.

### Deferred

**Re-homing a connection out of a retired context.** A connection homed in a retired
context keeps a valid FK and simply becomes unaddressable through its home. Giving
connections a re-home path is real work with its own design, and nothing in this feature
depends on it.

**Sweeping orphaned rows.** Retirement preserves everything by construction, so the
polymorphic referrers that a hard delete would have stranded — regions, edges,
subscriptions, grants, artifact shapes, properties — are simply not a problem here. If a
genuine hard delete is ever wanted for an empty context, that is a separate design and it
must confront those ten referrers, the three foreign keys, and §1.6.

---

## 5. Open questions

None blocking. Two judgment calls recorded rather than hidden:

- The mangled address format (`<slug>-retired-<short-id>`) is a choice, not a constraint.
  Any collision-free transform works; this one keeps the original readable.
- Whether a retired context should be excluded from semantic search is not addressed here.
  Search scopes through `resources_visible_to`, which inherits the floor (§1.2), so the
  behavior falls out — but it falls out rather than being designed, and that is worth a
  witness in the search suite if it ever matters.
