# A context can be created, shared, transferred and deleted — but never renamed

Design for the context rename surface: who may re-address a context, what the system refuses, and
why the refusal has to speak two dialects from one gate.

## The gap, and the scar that names it

`kb_contexts` has carried both halves of an identity since the canonical schema
(`migrations/20260624000001_canonical_schema.sql:159-167`):

```sql
CREATE TABLE kb_contexts (
    id           UUID PRIMARY KEY DEFAULT uuid_generate_v7(),
    owner_table  VARCHAR(64) NOT NULL CHECK (owner_table IN ('kb_profiles','kb_teams')),
    owner_id     UUID NOT NULL,
    slug         TEXT NOT NULL,        -- per-owner addressable handle
    name         TEXT NOT NULL,        -- display label (may collide across owners)
    created      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (owner_table, owner_id, slug)
);
```

Both are set once, at `create`, and no code path anywhere — CLI, API, MCP, substrate — ever writes
either again. Per-owner slug uniqueness is already enforced by the constraint above; the gap is
purely a missing surface, not a missing invariant.

The system already knows it needs this. `context_service::reassign` refuses a transfer whose slug
would collide under the new owner with (`context_service.rs:572-575`):

```rust
return Err(ApiError::Conflict(format!(
    "team already owns a context with slug '{}'; rename before transferring",
    cur.slug
)));
```

That message instructs the caller to perform an act the system does not offer. Closing the gap
discharges the instruction.

## Scope: one field, slug derived

Rename takes **a name**. The slug is `sluggify(name)`
(`temper_workflow::operations::sluggify`, already imported at `context_service.rs:20`). There is no
independent slug parameter.

The consequence is deliberate and must be stated plainly: **a rename re-addresses the context.**
After renaming `@me/temper` to `"Temper KB"`, the ref `@me/temper` no longer resolves and
`@me/temper-kb` does. Every stored `@owner/slug` string held by anyone, anywhere, is stale. This is
the cost of the single-field surface and it is accepted, not mitigated — see *Stated exclusion*.

## The gate

### Why the existing seam cannot express it

`ScopedAuthority` is the repo's scoped-authorization layer
(`docs/superpowers/specs/2026-07-22-scoped-authority-policy-layer-design.md`). Its refusal seam is a
**static, argument-free** method (`crates/temper-services/src/authz/mod.rs:104`):

```rust
fn denial() -> ApiError;
```

One dialect per gate. Every one of the **nine** existing authorities renders a single `ApiError`, and
`AuditAuthority` carries a test asserting both its denial arms are byte-identical `NotFound`
(`every_denial_renders_not_found`, `audit_gate.rs:897`) precisely so the write cannot become an
existence oracle.

Rename needs two dialects from one gate: `403` to a principal who can read the context but not
administer it, `404` to one who cannot see it at all.

**This is not an oracle.** The `403` goes only to principals who already read the context — they
learn nothing a `GET` would not already have told them. Refusal detail stays bounded by what the
caller already has standing to know. The policy is correct; it simply does not fit the current seam.

### EXTEND: `denial_for(&self)`

Add a defaulted method to `ScopedAuthority`, and have `authorize` call it:

```rust
fn denial_for(&self) -> ApiError { Self::denial() }
```

All nine existing impls are untouched — the default delegates to what they already declare.

**Nine, and the count is load-bearing.** `rg -n "impl ScopedAuthority for" crates/temper-services/src/authz/`
returns `GrantAuthority`, `ConnectionControlAuthority`, `ConnectionAuthority`, `MachineAuthority`,
`TwoSidedAuthority`, `AuditAuthority`, `AuditorJobAuthority`, `TeamReadAuthority`,
`ActorHistoryAuthority` — five refusing `Forbidden`, four refusing `NotFound`. An earlier draft of
this spec said six. `no-other-refusal-changes-its-voice` is a boundary over **all** of them, so a
regression suite written to "six" would leave three unguarded with no principle saying which three.

The property `read_gates.rs:68-70` leans on survives intact: *"`denial` is static and argument-free, so
it has no access to the slug it is refusing. The ambiguity is a property of the signature, not of
anyone remembering to preserve it."* `&self` exposes the **arm enum**, which carries no subject
data. A refusal still structurally cannot name the subject it refused.

### `ContextAdminAuthority`

```rust
pub(crate) enum ContextAdminAuthority {
    Administers,   // profile owner, or can_manage on the owning team
    SystemAdmin,   // admits; confers no ownership
    ReadOnly,      // visible but not administered  -> 403
    Invisible,     // not visible                   -> 404
}
```

Subject: `Uuid` (the context id).

**Probe order, and it is not the obvious one:**

1. `context_service::caller_administers_context` → `Administers`
2. `access_service::is_system_admin` → `SystemAdmin`
3. `context_visible_to` → `ReadOnly`
4. otherwise → `Invisible`

Admin sits at **2, not 1**, following the reasoning `TeamReadAuthority` already records
(`read_gates.rs:45-46`): the common caller is the administrator, and probing `is_system_admin` first
charges every one of them an extra round-trip.

Admin must sit **above 3**, and this is load-bearing. Printed live from the dev database:

```
$ \sf contexts_readable_by
CREATE OR REPLACE FUNCTION public.contexts_readable_by(p_profile uuid)
 RETURNS TABLE(context_id uuid) LANGUAGE sql STABLE
AS $function$
    -- 1. personal context
    -- 2. context OWNED by an enclosing team
    -- 3. context SHARED to an enclosing team
    -- 4. explicit read-grant on the context
$function$
```

There is **no system-admin branch**. `context_visible_to` → `context_readable_by_profile` →
`contexts_readable_by`, all three verified live. A visibility-first ordering would render `404` to a
system admin renaming a context they do not otherwise read — the exact actor the feature must admit.

Each arm **calls** its incumbent predicate. None restates one.

**`caller_administers_context` (`context_service.rs:415`) is the object-side probe, unchanged.** It
already answers exactly the question this feature asks: profile-owned ⇒ caller *is* the owner;
team-owned ⇒ `team_service::can_manage(role)`, i.e. Owner or Maintainer. It is `pub(crate)` and
currently reached only through `TwoSidedAuthority`; rename becomes its second consumer.

**CONFORM, deliberately: team administration is by DIRECT membership.** `caller_administers_context`
uses `team_service::role_on_team` (`context_service.rs:430-437`), while readability uses
`profile_effective_teams` + `team_ancestors`. A member of a team **beneath** the owning team can
therefore **read** a context but not **rename** it — they resolve to `ReadOnly` and get `403`.
Rename inherits that asymmetry. Changing it is a separate decision about the whole authorization
model, not a rename detail.

**The direction is the trap, and an earlier draft of this spec got it backwards.** Reads inherit
**down** the team tree, not up: `contexts_readable_by` expands the caller's teams *upward* to their
ancestors, so a thing attached to an ancestor is reachable by everyone beneath it. Probed live in a
rolled-back transaction:

```
                  probe                  | visible
-----------------------------------------+---------
 parent-maintainer reads CHILD-owned ctx | f
 child-member reads PARENT-owned ctx     | t
```

So a maintainer of an *enclosing* team holds **no standing at all** over a child team's context —
they resolve to `Invisible` and get `404`, not `ReadOnly` and `403`. This matters more than a
documentation nit: a test written to the wrong direction fails, and the obvious way to make it pass
is to widen the gate so a parent-maintainer resolves `ReadOnly` — **an authorization widening
shipped to turn a test green.** The register's *Closure* names this as route 2 of its four-route
equivalence class for exactly this reason.

### The `404` is the incumbent refusal

`Invisible` renders `ApiError::NotFound(CONTEXT_REFUSAL)` — the existing constant at
`context_service.rs:108`, whose docstring already explains why it is a constant rather than a
literal per site. Rename's refusal is therefore byte-identical to `get_visible`'s and
`resolve_context_ref`'s, and is covered by the same
`the_three_handle_slug_refusals_are_indistinguishable` reasoning.

## Names are canonicalized before they are stored — on **both** write paths

**AMEND**, and the only part of this design that changes an already-shipped surface's behavior. A
context name is persisted in canonical form: leading and trailing whitespace trimmed, and internal
runs of whitespace collapsed to a single space. One shared helper does it, and **both** `create` and
`rename` call it.

**`create` is in scope deliberately.** A canonical-form invariant honoured by one of two write paths
is not an invariant — rename would be a repair affordance for a hole `create` keeps digging. That is
a frame-owner decision (2026-07-30) recorded against the register's
`a-stored-name-has-one-spelling`, not something this spec concluded on its own.

**It normalizes whitespace and nothing else.** It must **not** reuse `sluggify`'s ASCII fold: a slug
is an address and may be lossy, a name is a display label and may not. `Café` stays `Café`.

**`sluggify` was never the problem here.** It already collapses whitespace — it splits on runs of
non-alphanumerics and rejoins with a single `-` (`refs.rs:55-62`), so `"Temper  KB"` and
`"Temper KB"` both yield `temper-kb`. The defect this closes is entirely in the stored `name`.

## Refusals

In order:

| Condition | Response |
|---|---|
| derived slug is empty | `400 BadRequest` |
| **canonical name** equals the stored name | `200`, `renamed: false`, **no event** |
| derived slug taken by **another** context under the same owner | `409 Conflict`, naming the colliding context |
| caller lacks authority | `403` / `404` per the gate above |

**The no-op test compares the canonical NAME, never the derived slug.** Slug-comparison is the
natural thing to write and it is wrong twice over. A stored name that predates canonicalization —
`"Temper  KB"` — sluggifies identically to its own repaired form, so slug-comparison would decline
the one rename that fixes it, permanently. And two genuinely different names can share a slug
(`"Temper KB"` and `"Temper-KB"` both yield `temper-kb`), so it would silently swallow a real
display-name change and report success. The register's
`a-request-that-would-change-stored-state-is-never-declined-as-a-no-op` is exactly this boundary.

Consequently **a rename whose slug does not move is still a rename**: it writes `name`, emits, and
returns `renamed: true`, with `from_slug == to_slug` in the payload recording that the address
stayed put. Only a canonical name identical to the stored one is a no-op.

**One `400` check, not two.** An all-whitespace name canonicalizes to empty and an empty name
sluggifies to empty, so `"   "` and `"!!!"` both fall out of the single derived-slug-is-empty test.

**The collision check must exclude the context being renamed.** This is a real trap, not a
formality: `reassign`'s query (`context_service.rs:563-576`) has no self-exclusion and does not need
one, because reassign changes the *owner*, so the row can never match itself. Rename keeps the owner
fixed. Copied verbatim, a name-only rename would find its own row and **409 against its own slug** —
which is precisely the legacy-repair case the no-op rule above exists to enable.

**The empty-slug `400` is a deliberate divergence from `create`.** `next_unique_context_slug` falls
back to the literal `"context"` when `sluggify` yields nothing (`context_service.rs:259-266`). That
is tolerable at birth and wrong at rename: `--name "!!!"` must not silently re-address a context to
`context`.

**The `409` is a deliberate divergence from `create`.** `next_unique_context_slug` auto-suffixes on
collision — `notes`, then `notes-2` — so two same-named contexts coexist rather than 409ing
(`context_service.rs:333-336` states this as intended). Rename does **not** use that function. A
rename is a deliberate re-address, and silently landing on `notes-2` gives the caller an address
they did not ask for and were not told about. Refusal is the honest answer; the caller picks
another name.

The `409` names the colliding context. That is not a leak: reaching the gate at all means the
caller administers this context, so they are the owner or manage the owning team, and can already
enumerate that owner's contexts.

`UNIQUE (owner_table, owner_id, slug)` remains the backstop against the check-then-act race, exactly
as it already is for `reassign` (`context_service.rs:561-562`).

### The race path must render the same refusal as the pre-check, and today's mapper would not

`map_reassign_write_err` (`context_service.rs:661-668`) maps `42501` to `Forbidden` and lets
everything else fall through to `ApiError::Internal`:

```rust
fn map_reassign_write_err(e: anyhow::Error) -> ApiError {
    if let Some(sqlx::Error::Database(db)) = e.downcast_ref::<sqlx::Error>() {
        if db.code().as_deref() == Some("42501") { return ApiError::Forbidden; }
    }
    ApiError::Internal(e.to_string())
}
```

A `23505` unique violation therefore surfaces as a **500**. Rename's mapper must carry a `23505` arm
rendering the same `409` the pre-check renders, or the caller's experience depends on how quickly
they lost the race.

### DECIDED — `reassign`'s identical hole rides along in the same change

`reassign` has this same defect today. Its 409 collision pre-check (`context_service.rs:563-576`) is
followed by the same `UNIQUE` backstop and the same mapper, so a lost race there also renders 500
where the pre-check renders 409. Rename does not introduce it; rename's tests are what surface it.

**The argument for bundling:** the repo's own convention is that a fix whose story is *"this PR's
tests surfaced a pre-existing bug"* belongs in the PR that surfaced it, so the narrative stays
cohesive. Rename would otherwise ship a correct mapper next to an incorrect one in the same file,
which is a worse artifact than either alternative.

**The argument for extracting:** it is a distinct narrative and an independently revertable fix, and
mixed-narrative PRs are harder to review.

**Decided 2026-07-30: bundle.** The convention applies directly, and the decisive point is the
artifact — extracting would land a correct mapper beside an incorrect one in the same file, which is
worse than either alternative. The implementation plan must therefore treat `reassign`'s `23505`
mapping as in scope, and its commit must say that rename's tests are what surfaced it.

**No-op idempotency** mirrors `reassign`'s (`context_service.rs:551-559`): a rename that computes
the slug already in place returns `renamed: false` and emits nothing.

## The write is event-sourced

Precedent among context mutations is split, and the split has a stated reason. `create`, `share` and
`unshare` are plain writes under *"product decision 5: contexts are infrastructure"*
(`context_service.rs:10-11`). `reassign` is event-sourced, and its migration states why that is safe
(`migrations/20260715000010_context_reassign_fns.sql:5-8`):

> `kb_contexts` is a replay INPUT table (restored verbatim), not a projection, so this projector is
> an idempotent re-apply on replay. This is why an evented context mutation is safe even though
> context create/share/unshare are un-evented.

Rename follows `reassign`, for two reasons. It is the only other mutation of an **identity-bearing**
column — `share`/`unshare` change reach, `create` has no before-state, while `reassign` and `rename`
both move something other parties' stored references depend on, and a who/when trail is the point.
And it inherits the atomic-authorization property the reassign migration names
(`20260715000010:38-41`):

> Authorization is an INVARIANT of this function, not a caller pre-check: the RBAC gate lives here,
> in the same transaction as the append+project, so there is no check-then-act window a
> membership/ownership change could slip through.

Rename has that same window: a maintainer demoted between the Rust gate and the `UPDATE`.

### Shape

A new migration modeled on `20260715000010_context_reassign_fns.sql`:

- `INSERT INTO kb_event_types ('context_renamed', NULL, 1)`. The `NULL` payload_schema keeps the
  event out of the published-schema `TYPED_EVENT_NAMES` invariant, exactly as `context_reassigned`
  and `resource_reassigned` do (`20260715000010:11-15`) — so **no schema-snapshot regeneration**.
- `_project_context_renamed(p_event, p_payload)` — sets `name` and `slug`; raises if the context is
  absent. A pure re-apply that **never authorizes**, so replayed history is not re-adjudicated
  against present-day membership.
- `context_rename(p_payload, p_emitter, p_metadata, p_invocation, p_correlation)` — the 5-param
  act-context signature every mutation function has carried since `20260709000050`. Carries the RBAC
  gate as an invariant, raising `42501` (`insufficient_privilege`) so the service maps the race path
  to `403` rather than `500`.

The plpgsql gate is **admit/deny only**. It does not reproduce the `403`/`404` split — the Rust
pre-check already rendered that, and this function is reached only on the race path.

Rust side: `writes::rename_context_with`, `EventKind::ContextRenamed`
(`temper-substrate/src/events.rs:117,158`), and a projector arm in `replay.rs` (alongside
`replay.rs:541`).

**Payload** carries `context_id`, `from_name`, `from_slug`, `to_name`, `to_slug`. The projector
needs only the `to_*` fields; the `from_*` fields exist for the trail. `context_reassigned` carries
its `to_owner_*` fields the same way.

### What a rename does not touch

The write is two columns of one row. Nothing else in the system is keyed by a context's slug:
resources are homed through `kb_resource_homes` on `(anchor_table, anchor_id)` where `anchor_id` is
a **UUID**, shares live in `kb_team_contexts` on `context_id`, and access grants key on
`subject_id`. So a rename cannot detach, re-home or hide any resource, cannot alter reach, and
cannot change who administers the context — not as a policy the implementation must uphold, but
because no code path relates any of those to the slug. It is stated here because it is the property
a reader will most want reassurance on, and "obvious from the schema" is not the same as written
down.

## Surfaces

| Surface | Addition |
|---|---|
| API | `POST /api/contexts/{id}/rename` |
| CLI | `temper context rename <ref> --name <name>` |
| MCP | `rename_context` |

The API path follows the verb-subpath shape `POST /api/contexts/{id}/reassign` already uses
(`handlers/contexts.rs:150`), not a `PATCH` on the collection member — rename is an act with a
refusal face, not a field edit.

MCP is included for parity: `contexts.rs` already exposes `create_context`, `get_context`,
`list_contexts`, `share_context`, `unshare_context` and `transfer_context`. A rename absent from
that set would be the only context act an agent cannot perform.

**Rename is service-direct, like every other context mutation.** An earlier draft of this spec said
writes route *surface → `DbBackend` → `writes::rename_context_with`*, citing the repo rule that
surfaces dispatch one operations command per inbound call. Disk contradicts it: the `Backend` trait
(`temper-workflow/src/operations/backend.rs`) has **zero** context commands, and `create`, `share`,
`unshare` and `reassign` are all service-direct on both surfaces — temper-mcp's own tool comments say
`SERVICE-DIRECT` in as many words.

Taking the rule literally here would make rename the **only** context command on the trait,
obligating a `CloudBackend` impl and `#[act_span]`/`ActInput` treatment that its three siblings do
not have — a structural divergence bought for nothing. The mechanism this spec actually requires
(`writes::rename_context_with`, called from the service, with the event appended and projected in
one transaction) is satisfied service-direct.

The both-surfaces obligation is discharged structurally rather than by discipline: the gate lives in
temper-services, so temper-api and temper-mcp cannot drift on *who is admitted*. What they **can**
still drift on is how a refusal is **rendered** — see the MCP note under *Surfaces* in the
implementation plan.

The outcome type carries `context_id`, `name`, `slug`, `owner_ref`, `renamed: bool` — mirroring
`ReassignContextOutcome` — **and the composed new ref**. The caller has just had their address
changed out from under them; making them reconstruct the new one from parts is the wrong place to
save a field.

**The composing helper is `decorated_context_ref` (`context_ref.rs:77-84`), and it has a trap.** Its
signature is `(owner_table, owner_addressable, context_slug)` and it prepends the sigil itself:

```rust
let sigil = if owner_table == "kb_teams" { '+' } else { '@' };
format!("{sigil}{owner_addressable}/{context_slug}")
```

`ContextRow.owner_ref` is **already** sigil-decorated — the SQL `CASE` at `context_service.rs:357-360`
emits `'+' || slug` / `'@' || handle`. Passing `owner_ref` as `owner_addressable` yields
`@@handle/slug`. Pass the undecorated handle or team slug. (An earlier draft of this spec named a
`format_context_ref` that does not exist.)

## Stated exclusion: client-side caches of the old slug

**Examined and deliberately excluded.** Three client-side things cache a context slug as a bare
string, and a rename staleness them all:

| Cache | Site | Effect |
|---|---|---|
| Vault projection directory | `projection.rs:465-473` derives the directory from `context_name` | The next `pull` writes a **new** directory; `prune_absent_files` scans only that new one, so the old directory survives with stale files |
| Sync subscriptions | `temper-core/src/types/config.rs:81`, `contexts: Vec<String>` | The subscription silently stops matching |
| Generated skill file | `commands/skill.rs:633`, `format_context_list` | The `## Contexts` list goes stale |

None is reconciled, and no advisory is printed.

**Reason.** Contexts are cloud-authoritative and the vault is a read-only projection cache; these
are caches of a server-side name, and a stale cache is refreshed by `temper pull` and `temper
skill`, not repaired by the write that staled it. A CLI-side fixup would also be **structurally
incomplete** — a rename issued from MCP, the UI, or another machine bypasses it entirely — and a
partial fixup is worse than none, because it teaches the operator that the problem is handled.

The residue is real and is named rather than hidden: **after a rename, the vault contains both the
old and the new context directory, and nothing says so.** The old directory's files still parse and
still carry the old context in frontmatter, which is a live trap for an agent grepping the vault.
This is a named remainder, not a filed task.

## Reconciliation with the outcome register

This spec is the *how* for goal `019fb4db-7732-78d2-9ad4-73d44b053c03`, whose clauses name no
mechanism. The mapping below is the reconciliation: which section of this spec is claimed to
discharge which clause. It is **not** a coverage claim — every clause in that register is currently
**declared-uncovered**, because witnesses are authored during the build, not before it.

| Clause | Discharged by |
|---|---|
| `rename-requires-administration` | *The gate* — `Administers` and `SystemAdmin` arms |
| `refusal-discloses-no-more-than-the-caller-already-holds` | *The `404` is the incumbent refusal* |
| `a-rename-lands-where-it-was-asked-or-nowhere` | *Refusals* — the empty-slug 400 and the collision 409, both divergences from `create` |
| `one-owner-never-holds-two-of-the-same-address` | The `UNIQUE` backstop **plus** the `23505` mapper arm |
| `every-completed-rename-is-attributable` | *The write is event-sourced* — the `from_*` payload fields |
| `authority-is-decided-no-earlier-than-the-change` | `context_rename`'s in-transaction RBAC invariant |
| `replayed-history-is-not-re-adjudicated` | `_project_context_renamed` never authorizes |
| `system-authority-never-becomes-ownership` | *What a rename does not touch* |
| `no-other-refusal-changes-its-voice` | *EXTEND: `denial_for(&self)`* — **see the obligation below** |
| `a-refusal-never-names-what-it-withholds` | *EXTEND: `denial_for(&self)`* — `&self` exposes the arm enum, never the subject |
| `a-context-never-loses-its-contents-to-a-rename` | *What a rename does not touch* |
| `a-reader-is-never-told-a-readable-context-is-absent` | *The gate* — the `ReadOnly` arm |

### Two obligations the build inherits, and they are the risky ones

**The trait change is the highest-risk edit in this design, and this spec discharges its clause only
by construction.** `denial_for(&self)` defaults to `Self::denial()`, so the nine existing authorities
"cannot" change behavior. That argument is sound and it is still only an argument — the failure
shape is an impl that overrides the default without meaning to, or an `authorize` that calls one
method on one path and the other on another. `no-other-refusal-changes-its-voice` is a standing
regression boundary on shared authorization machinery; the build owes it something that would fail
if the boundary moved, not a paragraph asserting it cannot.

**The register's first equivalence class is the one most likely to be wrong.** It claims all four
routes to read-without-administration — lower team role, enclosing-team membership, share to a
reachable team, explicit read grant — collapse to a single `ReadOnly` outcome. They arrive through
genuinely different machinery: three are `UNION` arms of `contexts_readable_by` and one is
`kb_access_grants`. The claim holds structurally, since `caller_administers_context` consults
neither, but a class claimed across four mechanisms is exactly the shape that hides an unexamined
cell. The build owes each route separately, not one representative.

## Out of scope

**Rejected** — decided against, not deferred:

- **An independent `--slug` parameter.** Rename is one field.
- **Auto-suffixing on rename collision.** See *Refusals*.
- **A `--force` flag to opt back into auto-suffixing.** It would exist only to re-enable the
  behavior the 409 was chosen to prevent.
- **Client-side cache reconciliation.** See *Stated exclusion*.

**Deferred** — wanted, not now:

- **Renaming teams and profiles.** `kb_teams.slug` is globally `UNIQUE` and appears in share flows,
  so a team rename has a wider blast radius and a different leak analysis. Out of this design.
- **Widening team-context administration to transitive membership.** See the CONFORM note in *The
  gate*; it is a decision about the authorization model, not about rename.
- **A redirect or alias from the old slug.** Would make rename non-breaking, at the cost of a new
  table and a resolution-order question in `resolve_context_ref`. Not attempted here.
