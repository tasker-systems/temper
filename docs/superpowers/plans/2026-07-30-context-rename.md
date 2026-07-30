# Context Rename — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended)
> or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal (the *what*):** outcome register `019fb4db-7732-78d2-9ad4-73d44b053c03` —
*"A context can be re-addressed by those who administer it, and by no one else."*
Read it with `temper resource show a-context-can-be-re-addressed-by-those-who-administer-it-and-by-no-one-else-019fb4db-7732-78d2-9ad4-73d44b053c03`.

**Spec (the *how*, approved):** `docs/superpowers/specs/2026-07-30-context-rename-design.md` —
**read it before starting.** This plan is an *index + sequence + grounding evidence* over that spec,
not a replacement for it. Every task cites the spec section its implementer must read.

**Architecture:** Add a rename act to contexts, mirroring the shape `reassign` already established
in the same file: an event-sourced mutation whose RBAC gate is an invariant of the plpgsql function,
fronted by a Rust pre-check that renders a **two-dialect** refusal (`403` to a reader, `404` to a
non-reader). The two dialects require one small change to shared authorization machinery —
`ScopedAuthority::denial_for(&self)` — which is the highest-risk edit in the design and gets its
own standing regression boundary.

**Tech stack:** Rust (sqlx, axum, clap, rmcp), plpgsql, PostgreSQL 17/18.

---

# Part 0 — Grounding evidence

Everything below is quoted from disk or is real command output, captured on branch
`jct/context-rename` at `09289eb8`. **Nothing in Part 1's tasks may assume a fact that is not
recorded here or verified by the implementer on disk (GD-1).** Where a task invents, it carries an
EXTEND/AMEND tag and cites the spec section authorizing it (GD-3).

## G1 — `kb_contexts`, live (`\d kb_contexts`)

```
                                     Table "public.kb_contexts"
           Column            |           Type           | Nullable |      Default
-----------------------------+--------------------------+----------+--------------------
 id                          | uuid                     | not null | uuid_generate_v7()
 owner_table                 | character varying(64)    | not null |
 owner_id                    | uuid                     | not null |
 slug                        | text                     | not null |
 name                        | text                     | not null |
 created                     | timestamp with time zone | not null | now()
 shape_materialized_event_id | uuid                     |          |
 telos_centroid              | vector(768)              |          |
Indexes:
    "kb_contexts_pkey" PRIMARY KEY, btree (id)
    "idx_kb_contexts_owner" btree (owner_table, owner_id)
    "kb_contexts_owner_table_owner_id_slug_key" UNIQUE CONSTRAINT, btree (owner_table, owner_id, slug)
Check constraints:
    "kb_contexts_owner_table_check" CHECK (owner_table::text = ANY (ARRAY['kb_profiles','kb_teams']))
```

Two consequences the tasks depend on:

1. **`UNIQUE (owner_table, owner_id, slug)` already exists.** The plan adds **no** constraint; the
   race backstop the register's `one-owner-never-holds-two-of-the-same-address` needs is on disk
   today. What is missing is only the `23505` → `409` mapping (G8).
2. **There is no `updated` column.** `context_service.rs:41` synthesizes it
   (`c.created AS "updated!"`). So a rename bumps no row-level timestamp, and
   `every-completed-rename-is-attributable` is discharged **entirely** by the event trail, never by
   the row. Any implementer tempted to "also update `updated`" is inventing a column.

## G2 — `contexts_readable_by` has no system-admin branch (live `\sf`)

```sql
CREATE OR REPLACE FUNCTION public.contexts_readable_by(p_profile uuid)
 RETURNS TABLE(context_id uuid) LANGUAGE sql STABLE
AS $function$
    WITH reachable_teams AS (
        SELECT DISTINCT a.team_id
        FROM profile_effective_teams(p_profile) e
        CROSS JOIN LATERAL team_ancestors(e.team_id) a
    )
    -- 1. personal context
    SELECT c.id FROM kb_contexts c
    WHERE c.owner_table = 'kb_profiles' AND c.owner_id = p_profile
    UNION
    -- 2. context OWNED by an enclosing team.
    SELECT c.id FROM kb_contexts c
    JOIN reachable_teams rt ON rt.team_id = c.owner_id
    WHERE c.owner_table = 'kb_teams'
    UNION
    -- 3. context SHARED to an enclosing team
    SELECT tc.context_id FROM kb_team_contexts tc
    JOIN reachable_teams rt ON rt.team_id = tc.team_id
    UNION
    -- 4. explicit read-grant on the context (profile-anchored, or team-anchored on a reachable team)
    SELECT g.subject_id FROM kb_access_grants g
    WHERE g.subject_table = 'kb_contexts' AND g.can_read
      AND ( (g.principal_table = 'kb_profiles' AND g.principal_id = p_profile)
         OR (g.principal_table = 'kb_teams'
               AND g.principal_id IN (SELECT team_id FROM reachable_teams)) );
$function$
```

```sql
CREATE OR REPLACE FUNCTION public.context_visible_to(p_principal uuid, p_context_id uuid)
 RETURNS boolean LANGUAGE sql STABLE
AS $function$ SELECT context_readable_by_profile(p_principal, p_context_id); $function$

CREATE OR REPLACE FUNCTION public.context_readable_by_profile(p_profile uuid, p_context uuid)
 RETURNS boolean LANGUAGE sql STABLE
AS $function$
    SELECT EXISTS (SELECT 1 FROM contexts_readable_by(p_profile) c WHERE c.context_id = p_context);
$function$

CREATE OR REPLACE FUNCTION public.is_system_admin(p_profile_id uuid)
 RETURNS boolean LANGUAGE sql STABLE
AS $function$
    SELECT EXISTS (SELECT 1 FROM kb_principal_governance g WHERE g.profile_id = p_profile_id)
$function$
```

**Verified: the spec's load-bearing claim holds.** `contexts_readable_by` has no system-admin arm,
so a visibility-first probe order would render `404` to a system admin renaming a context they do
not otherwise read. `SystemAdmin` must sit above the `context_visible_to` probe. Spec §"The gate",
probe order step 2-above-3.

## G3 — ⚠️ SPEC CORRECTION: the enclosing-team read direction is **inverted** in the spec

The spec states (spec §"The gate", CONFORM note):

> A maintainer of a *parent* team can therefore **read** a child team's context but not **rename**
> it — they resolve to `ReadOnly` and get `403`.

**That is false against the live schema.** Reads inherit **down** the tree, not up: `reachable_teams`
is *the caller's own teams expanded UP to their ancestors*, and arm 2 admits contexts owned by a team
in that set. So a member of a **descendant** team reads an **ancestor** team's context, and a
maintainer of a parent team does **not** reach a child team's context at all.

Probe, run in a rolled-back transaction against the dev DB:

```
              q                          | visible
-----------------------------------------+---------
 parent-maintainer reads CHILD-owned ctx | f
 child-member reads PARENT-owned ctx     | t
 child-member reads CHILD-owned ctx      | t
 parent-maintainer reads PARENT-owned ctx| t
```

**Why this matters and is not cosmetic.** The register's equivalence class 1 names
"enclosing-team membership" as one of four routes to read-without-administration. Built the spec's
way (actor = maintainer of the parent, context owned by the child) the actor is **`Invisible`** and
gets `404`, not `ReadOnly`/`403`. A test written to the spec's wording would fail, and the obvious
"fix" — widening the gate so a parent-maintainer resolves `ReadOnly` — would be a **real
authorization widening** shipped to make a test green.

**The asymmetry the spec is describing is real, in the other direction**, and the outcome it claims
(`ReadOnly` → `403`) is correct once the direction is fixed: a **maintainer of a descendant team**
reads an ancestor-team-owned context (arm 2) but `caller_administers_context` asks
`role_on_team(<owning team>, caller)` by direct membership (G6), so they are not an administrator.
**Route 2 must be built as "maintainer of a DESCENDANT team; context owned by the ancestor."**

The spec cites `two_sided.rs:127-132` as its evidence for this asymmetry. That comment is about the
**target-team bar** of the two-sided gate, not about context administration:

```rust
// crates/temper-services/src/authz/two_sided.rs:128-132
        // The target-team bar: `can_manage` (Owner|Maintainer) by DIRECT membership. Both gates
        // spelled this identically; it is `can_manage` and not `owner` on purpose.
```

The correct citation is `context_service.rs:430-437` (G6).

## G4 — All four read-without-administration routes, live

Probe run in a rolled-back transaction. Owning team `probe-owning`; `probe-desc` is its child;
`probe-other` is unrelated and the context is shared to it. "administers" reproduces
`caller_administers_context`'s team arm (direct `owner|maintainer` on the owning team).

```
              label               | readable | administers | sysadmin
----------------------------------+----------+-------------+----------
 1 lower role on owning team      | t        | f           | f
 2 maintainer of DESCENDANT team  | t        | f           | f
 3 share to a reachable team      | t        | f           | f
 4 explicit read grant            | t        | f           | f
 5 owner of owning team (control) | t        | t           | f
```

**All four routes are confirmed distinct-in-mechanism and identical-in-outcome at the predicate
level** (readable ∧ ¬administers ∧ ¬sysadmin). The register's class-1 claim survives this probe.
What the probe does **not** establish is that the *rename act* renders one `403` for all four — that
is what Task 11 owes, because it exercises the gate and the surface, not the predicates.

## G5 — `ScopedAuthority`, and ⚠️ SPEC CORRECTION: there are **nine** impls, not six

```rust
// crates/temper-services/src/authz/mod.rs:86-104
    async fn resolve(pool: &PgPool, caller: ProfileId, subject: Self::Subject) -> ApiResult<Self>;

    fn is_denial(&self) -> bool;

    /// How this domain renders a refusal.
    ///
    /// **Not boilerplate, and not always `Forbidden`.** Some gates refuse with `NotFound` on
    /// purpose, because the existence of the subject is itself the secret: [...]
    fn denial() -> ApiError;
```

```rust
// crates/temper-services/src/authz/mod.rs:142-152
pub(crate) async fn authorize<A: ScopedAuthority>(
    pool: &PgPool,
    caller: ProfileId,
    subject: A::Subject,
) -> ApiResult<Authorized<A>> {
    let authority = A::resolve(pool, caller, subject).await?;
    if authority.is_denial() {
        return Err(A::denial());
    }
    Ok(Authorized { authority, subject })
}
```

`rg -n "impl ScopedAuthority for" crates/` returns **nine**, and the spec says "six":

| # | Authority | File:line | `denial()` renders |
|---|---|---|---|
| 1 | `GrantAuthority` | `authz/grant.rs:23` | `ApiError::Forbidden` |
| 2 | `ConnectionControlAuthority` | `authz/connection.rs:70` | `ApiError::Forbidden` |
| 3 | `ConnectionAuthority` | `authz/connection.rs:118` | `ApiError::Forbidden` |
| 4 | `MachineAuthority` | `authz/machine.rs:19` | `ApiError::Forbidden` |
| 5 | `TwoSidedAuthority` | `authz/two_sided.rs:86` | `ApiError::Forbidden` |
| 6 | `AuditAuthority` | `authz/audit_gate.rs:268` | `ApiError::NotFound(FINDING_REFUSAL)` |
| 7 | `AuditorJobAuthority` | `authz/audit_gate.rs:396` | `ApiError::NotFound("cognitive map not found or not readable")` |
| 8 | `TeamReadAuthority` | `authz/read_gates.rs:40` | `ApiError::NotFound("team not found or not readable")` |
| 9 | `ActorHistoryAuthority` | `authz/read_gates.rs:93` | `ApiError::NotFound(ACTOR_HISTORY_REFUSAL)` |

The number is load-bearing: `no-other-refusal-changes-its-voice` is a boundary over **all** of them.
A boundary suite written to the spec's "six" would leave three unguarded, and which three is
arbitrary. **Task 10 covers nine and asserts the count, so a tenth authority added later cannot
slip past the boundary silently.**

The property the spec says survives, quoted from disk:

```rust
// crates/temper-services/src/authz/read_gates.rs:68-70
    /// The message says "or not readable" for the same reason, and cannot say more: `denial` is
    /// static and argument-free, so it has no access to the slug it is refusing. The ambiguity is
    /// a property of the signature, not of anyone remembering to preserve it.
```

The existing byte-identity guard the spec points at is
`every_denial_renders_not_found` (`authz/audit_gate.rs:897`, spec cites `:914`).

## G6 — `caller_administers_context` is the object-side probe, unchanged

```rust
// crates/temper-services/src/services/context_service.rs:415-438
pub(crate) async fn caller_administers_context(
    pool: &PgPool,
    caller: ProfileId,
    context_id: uuid::Uuid,
) -> ApiResult<bool> {
    let Some(owner) = sqlx::query!(
        r#"SELECT owner_table AS "owner_table!", owner_id AS "owner_id!"
           FROM kb_contexts WHERE id = $1"#,
        context_id
    )
    .fetch_optional(pool)
    .await?
    else {
        return Ok(false);
    };
    match owner.owner_table.as_str() {
        "kb_profiles" => Ok(owner.owner_id == *caller),
        "kb_teams" => Ok(matches!(
            team_service::role_on_team(pool, owner.owner_id, caller).await?,
            Some(role) if team_service::can_manage(role)
        )),
        _ => Ok(false),
    }
}
```

`role_on_team` is direct membership only (`team_service.rs:47-62`: `WHERE team_id = $1 AND
profile_id = $2` against `kb_team_members`), and `can_manage` is `Owner | Maintainer`
(`team_service.rs:67-69`). **A missing context resolves to `false`**, which is why the arm ordering
in Task 5 must place `context_visible_to` before `Invisible` rather than treating `false` as
"absent".

It is `pub(crate)` with exactly one consumer today (`two_sided.rs:144`). Rename is its second.

## G7 — the `404` is an existing constant, not a new literal

```rust
// crates/temper-services/src/services/context_service.rs:99-108
/// A constant rather than a literal per site, for the reason `FINDING_REFUSAL` gives: four copies
/// of one defence is four defences that drift. `the_three_handle_slug_refusals_are_indistinguishable`
/// asserts the byte-identity directly.
///
/// The `@me` arm is deliberately **not** bound by this and still names the slug — that slug is the
/// caller's own, so echoing it discloses nothing they did not supply.
const CONTEXT_REFUSAL: &str = "context not found or not readable";
```

`CONTEXT_REFUSAL` is **private to `context_service`** today. The new authority lives in
`crates/temper-services/src/authz/`, so Task 5 must widen it to `pub(crate)` — a visibility change,
never a second literal. The byte-identity guard it feeds is
`crates/temper-api/tests/context_ref_resolve_test.rs:320`.

## G8 — `reassign`: the incumbent shape, and the `23505` hole it shares

```rust
// crates/temper-services/src/services/context_service.rs:551-576
    if cur.owner_table == "kb_teams" && cur.owner_id == to_team_id {
        return Ok(ReassignContextOutcome { /* ... */ reassigned: false, /* ... */ });
    }

    // The slug must be unique under the NEW owner — 409 rather than a silent re-slug or an
    // opaque UNIQUE violation surfacing from the projector.
    let collision = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM kb_contexts
             WHERE owner_table = 'kb_teams' AND owner_id = $1 AND slug = $2) AS "e!""#,
        to_team_id,
        cur.slug,
    )
    .fetch_one(pool)
    .await?;
    if collision {
        return Err(ApiError::Conflict(format!(
            "team already owns a context with slug '{}'; rename before transferring",
            cur.slug
        )));
    }
```

```rust
// crates/temper-services/src/services/context_service.rs:578-590
    let emitter = temper_substrate::writes::resolve_emitter(pool, caller, "web")
        .await
        .map_err(|e| ApiError::Internal(e.to_string()))?;
    temper_substrate::writes::reassign_context_with(
        pool,
        temper_substrate::ids::ContextId::from(context_id),
        (cur.owner_table.as_str(), cur.owner_id),
        ("kb_teams", to_team_id),
        emitter,
        temper_substrate::events::EventContext::default(),
    )
    .await
    .map_err(map_reassign_write_err)?;
```

```rust
// crates/temper-services/src/services/context_service.rs:661-668
fn map_reassign_write_err(e: anyhow::Error) -> ApiError {
    if let Some(sqlx::Error::Database(db)) = e.downcast_ref::<sqlx::Error>() {
        if db.code().as_deref() == Some("42501") {
            return ApiError::Forbidden;
        }
    }
    ApiError::Internal(e.to_string())
}
```

**The hole is confirmed on disk:** no `23505` arm, so a lost collision race falls through to
`ApiError::Internal` → **500**, where the pre-check renders **409**. Spec §"DECIDED — `reassign`'s
identical hole rides along in the same change" makes this in scope for this PR. Note that `"web"`
is hardcoded into `resolve_emitter` at every service site (`access_service.rs:371,410`,
`slack_disconnect_service.rs:107`, `machine_registration_service.rs:222`) — rename **conforms** to
that; the surface-of-origin is not recorded on the emitter, and changing that is not this work.

## G9 — the reassign migration, the template Task 2 mirrors

```sql
-- migrations/20260715000010_context_reassign_fns.sql:5-15
-- kb_contexts is a replay INPUT table (restored verbatim), not a projection, so this
-- projector is an idempotent re-apply on replay [...] This is why an evented context
-- mutation is safe even though context create/share/unshare are un-evented.

-- _event_append raises unless the event name is seeded. NULL payload_schema keeps it
-- out of the published-schema TYPED_EVENT_NAMES invariant (as resource_reassigned).
INSERT INTO kb_event_types (name, payload_schema, schema_version)
VALUES ('context_reassigned', NULL, 1)
ON CONFLICT (name) DO NOTHING;
```

```sql
-- migrations/20260715000010_context_reassign_fns.sql:19-30
CREATE FUNCTION _project_context_reassigned(p_event uuid, p_payload jsonb)
RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE v_context uuid := (p_payload->>'context_id')::uuid;
BEGIN
    UPDATE kb_contexts
       SET owner_table = p_payload->>'to_owner_table',
           owner_id    = (p_payload->>'to_owner_id')::uuid
     WHERE id = v_context;
    IF NOT FOUND THEN RAISE EXCEPTION 'context_reassign: context % not found', v_context; END IF;
    RETURN v_context;
END;
$$;
```

```sql
-- migrations/20260715000010_context_reassign_fns.sql:35-46 (the invariant the spec quotes)
-- Authorization is an INVARIANT of this function, not a caller pre-check: the RBAC gate lives
-- here, in the same transaction as the append+project, so there is no check-then-act window a
-- membership/ownership change could slip through. [...] Only
-- the mutation half authorizes; the projector (`_project_context_reassigned`, the replay path)
-- stays a pure re-apply, so historical events never re-authorize on replay.
```

The 5-param signature and the emitter→actor resolution, verbatim:

```sql
-- migrations/20260715000010_context_reassign_fns.sql:47-74
CREATE FUNCTION context_reassign(p_payload jsonb, p_emitter uuid,
                                 p_metadata jsonb DEFAULT '{}'::jsonb,
                                 p_invocation uuid DEFAULT NULL,
                                 p_correlation uuid DEFAULT NULL)
RETURNS uuid LANGUAGE plpgsql AS $$
[...]
    SELECT profile_id INTO v_actor FROM kb_entities WHERE id = p_emitter;
    IF v_actor IS NULL THEN
        RAISE EXCEPTION 'context_reassign: emitter % has no profile', p_emitter
              USING ERRCODE = '42501';
    END IF;
```

and the append+project tail:

```sql
-- migrations/20260715000010_context_reassign_fns.sql:115-118
    v_ev := _event_append('context_reassigned', p_emitter, 'kb_contexts', v_context, p_payload,
                          p_metadata => p_metadata, p_invocation => p_invocation,
                          p_correlation => p_correlation);
    RETURN _project_context_reassigned(v_ev, p_payload);
```

## G10 — substrate plumbing: the exact five edit sites, and the snapshot exclusion

```rust
// crates/temper-substrate/src/payloads.rs:800-810
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "scenario-schema", derive(schemars::JsonSchema))]
pub struct ContextReassigned {
    pub context_id: ContextId,
    pub from_owner_table: String,
    pub from_owner_id: Uuid,
    pub to_owner_table: String,
    pub to_owner_id: Uuid,
}
```

`rg -n "ContextReassign" crates/temper-substrate/src/` gives exactly the sites a rename must mirror:

| Site | What lives there |
|---|---|
| `events.rs:46` | `EventKind::ContextReassigned` variant |
| `events.rs:117` | `as_canonical_name` arm |
| `events.rs:158` | `from_canonical_name` arm |
| `events.rs:403-410` | `SeedAction::ContextReassign { .. }` variant |
| `events.rs:462` | `SeedAction::event_type()` arm |
| `events.rs:1194-1220` | the `fire_with` arm: builds the payload, `SELECT context_reassign($1,$2,$3,$4,$5)`, returns `Fired::Context` |
| `replay.rs:207` | content-sidecar match: folded into the `=> None` group |
| `replay.rs:540-546` | replay projector dispatch: `SELECT _project_context_reassigned($1,$2)` |
| `writes.rs:652-676` | `reassign_context_with` — `begin_scoped` → `fire_with` → `commit` |

**Snapshot exclusion verified, so the spec's "no schema-snapshot regeneration" holds.**
`TYPED_EVENT_NAMES` is `[&str; 21]` (`payloads.rs:1095`) and `context_reassigned` is **not** in it;
`tests/payload_schema.rs:28-49` names its 21 structs explicitly and `ContextReassigned` is **not**
among them; `snapshot_files_cover_exactly_the_typed_names` (`payload_schema.rs:52`) compares the
fixture directory against that same const. So a new `ContextRenamed` payload struct that mirrors
`ContextReassigned` (including its `cfg_attr(scenario-schema, JsonSchema)`) adds **no** snapshot
file and requires **no** `UPDATE_SCHEMA=1` run — *provided it is not added to `TYPED_EVENT_NAMES`*.
Adding it there would restale two suites and change what the boot-seed stamps into the registry.

`ContextReassign` appears **nowhere** in `src/scenario/` (`loader.rs`, `runner.rs`, `bootseed.rs`),
so no YAML scenario-DSL arm is owed.

## G11 — the surfaces, and ⚠️ SPEC CORRECTION: contexts are **service-direct**, not `DbBackend`

The spec §"Surfaces" says:

> Writes route surface → `DbBackend` → `writes::rename_context_with`, per the repo rule that
> surfaces dispatch one operations command per inbound call and never call write persistence
> directly.

**No context mutation on disk does this.** `rg -n "async fn" crates/temper-workflow/src/operations/backend.rs`
shows the `Backend` trait carries resource, relationship, citation-audit, facet, cogmap, invocation
and steward commands — and **zero context commands**. All three sibling context mutations go
surface → `context_service` → `writes::*`:

```rust
// crates/temper-api/src/handlers/contexts.rs:162-176 — the API's reassign handler
pub async fn reassign(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(context_id): Path<Uuid>,
    Json(body): Json<ReassignContextRequest>,
) -> ApiResult<Json<ReassignContextOutcome>> {
    let outcome = context_service::reassign(
        &state.pool,
        ProfileId::from(auth.0.profile().id),
        context_id,
        body.to_team_id,
    )
    .await?;
    Ok(Json(outcome))
}
```

```rust
// crates/temper-mcp/src/tools/contexts.rs:154-158 — and MCP says so in its own doc comment
/// Transfer a context's ownership to a team. SERVICE-DIRECT, authorized by
/// `context_service::reassign` (the two-sided `can_share` gate) before the write.
```

**Resolution carried by this plan:** the spec's *mechanism* sentence in §"The write is
event-sourced" — *"Rust side: `writes::rename_context_with`"* — is what is load-bearing, and it is
satisfied by `context_service::rename` calling `writes::rename_context_with`, exactly as
`reassign` does. The `DbBackend` clause in §"Surfaces" is unsupported by disk and, if taken
literally, would add the *only* context command to the `Backend` trait, obligating an impl on
`CloudBackend` (`temper-cli/src/cloud_backend/backend.rs:75` and `:551`) and an `ActInput`/
`#[act_span]` treatment its three siblings do not have — a new pattern for one act, beside three
that do it the other way, in the same file.

> **ESCALATION POINT — resolve before Task 6.** This plan proceeds **service-direct (CONFORM to
> `reassign`)**. If the controller wants the `DbBackend` letter of the spec instead, that is a
> larger change than one task and must also decide whether `reassign`/`share`/`unshare` migrate with
> it, because leaving one context write on the trait and three off it is worse than either.

The remaining surface facts:

- **Routes** — `crates/temper-api/src/routes.rs:102`: `.routes(routes!(handlers::contexts::reassign))`.
- **OpenAPI** — the handler's `#[utoipa::path]` block is at `handlers/contexts.rs:147-161`. A new
  route + response DTO restales `openapi.json`, the temper-rb gem and temper-ts `schema.ts` (see G14).
- **Client** — `crates/temper-client/src/contexts.rs:95-106`, `pub async fn reassign(&self,
  context_id: Uuid, body: &ReassignContextRequest) -> Result<ReassignContextOutcome>`, `POST` via
  `self.http.send_json(&Method::POST, &path, req, Some(&token))`.
- **CLI enum** — `crates/temper-cli/src/cli.rs:893-899` (`ContextAction::Transfer`).
- **CLI dispatch** — `crates/temper-cli/src/main.rs:430-442`, wrapped in
  `temper_cli::actions::runtime::with_client(|client| Box::pin(async move { ... }))`. Note the
  signature: **no config argument, and the closure body is `Box::pin`-ed.**
- **CLI action** — `crates/temper-cli/src/commands/context_cmd.rs:245-262` (`transfer_remote`),
  which resolves via `resolve_context_id_for_read` (`context_cmd.rs:291`, `@me`-accepting) — *not*
  `resolve_context_id` (`:167`, which refuses `@me`).
- **MCP** — the tool body at `crates/temper-mcp/src/tools/contexts.rs:159-178` **and** the
  `#[tool(description = ...)]`-annotated delegating method at
  `crates/temper-mcp/src/service.rs:735-745`. **Both edits are required**; without the `service.rs`
  method the tool is never exposed.

**⚠️ `resolve_context_id_for_read` lists client-side and matches locally** (`context_cmd.rs:167-198`
for the sibling; the read variant mirrors it), so a decorated ref naming a context the caller cannot
see fails in the CLI with *"not found among the contexts you can see"* and **never reaches the
server**. Any e2e test that means to exercise the `Invisible` → `404` arm through the CLI must pass
a **bare UUID**.

**⚠️ MCP's error mapper has no `Conflict` or `BadRequest` arm**:

```rust
// crates/temper-mcp/src/tools/contexts.rs:17-33
fn map_api_error(context: &str, err: ApiError) -> rmcp::ErrorData {
    match err {
        ApiError::Forbidden => rmcp::ErrorData::invalid_params(
            format!(
                "{context} requires that you administer the context and manage the target team \
                 (owner/maintainer), or that you are an instance administrator"
            ),
            None,
        ),
        ApiError::NotFound(msg) => {
            rmcp::ErrorData::invalid_params(format!("{context}: {msg}"), None)
        }
        other => rmcp::ErrorData::internal_error(format!("{context} failed: {other}"), None),
    }
}
```

So rename's `409` and `400` would land in `internal_error` — an agent would read "the server broke"
where the truth is "pick another name". And the `Forbidden` arm's message names a *target team*,
which rename does not have. Task 9 owes both.

## G12 — migration numbering

```
$ git ls-tree origin/main --name-only migrations/ | sort | tail -3
migrations/20260728000010_workflow_props_status.sql
migrations/20260730000010_facet_inner_key_grain.sql
migrations/templates
```

Highest on `main` is `20260730000010`. The new migration is **`20260730000020_context_rename_fns.sql`**.
Migrations are immutable once applied; if `main` moves above this number before merge, renumber
**above** the new high-water mark rather than editing the applied file.

## G13 — `sluggify`

```rust
// crates/temper-workflow/src/operations/refs.rs:39-62 (tail)
    folded
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|seg| !seg.is_empty())
        .collect::<Vec<_>>()
        .join("-")
```

Already imported in `context_service.rs:20` (`use temper_workflow::operations::sluggify;`). Empty
segments are filtered, so a non-empty result always starts with an ASCII alphanumeric — which is
what `context_ref::validate_slug` (`temper-core/src/context_ref.rs:56-67`) requires. Rename's
`400` fires exactly when `sluggify(name).is_empty()`.

The `create`-path fallback rename must **not** use:

```rust
// crates/temper-services/src/services/context_service.rs:259-266
    let base = {
        let s = sluggify(name);
        if s.is_empty() { "context".to_owned() } else { s }
    };
```

and the auto-suffix rename must **not** use:

```rust
// crates/temper-services/src/services/context_service.rs:333-336
/// The substrate enforces uniqueness on the generated slug
/// (`(owner_table, owner_id, slug)`), not the name — `next_unique_context_slug`
/// auto-suffixes on collision (scoped to this owner), so two contexts sharing a
/// name coexist under distinct slugs rather than 409ing.
```

Both divergences are spec §"Refusals", and both are stated there as deliberate.

## G14 — tiers, caches, generated artifacts

Tier definitions, from `tools/cargo-make/main.toml`:

| Task | Command | What it covers here |
|---|---|---|
| `cargo make test` | `nextest run --workspace --no-fail-fast` (+ `test-schema` dep) | pure-unit tests, incl. `authz` denial rendering (no pool) |
| `cargo make test-db` | `nextest run --workspace --features test-db` | `temper-services`' `#[cfg(all(test, feature = "test-db"))]` modules and `crates/temper-api/tests/*` |
| `cargo make test-e2e` | `nextest run --manifest-path tests/e2e/Cargo.toml --features test-db` | real Axum + Postgres + CLI + MCP |
| `cargo make test-artifacts` | `nextest run -p temper-substrate --features artifact-tests` | `tests/replay_roundtrip.rs` (`#![cfg(feature = "artifact-tests")]`) |

`.sqlx` caches present: `.sqlx` (workspace root), `crates/temper-services/.sqlx`, `tests/e2e/.sqlx`.
Per the `sqlx-query-cache` skill: after changing SQL/migrations run
`cargo sqlx prepare --workspace -- --all-features`, then `cargo make prepare-services`, then
`cargo make prepare-e2e` — **in that order, per-crate last**. `temper-api` has no per-crate cache,
deliberately; do not create one.

Per `generated-artifacts`: a new route + new response DTO restales `openapi.json`,
`clients/temper-rb/lib/temper/generated`, and `clients/temper-ts/src/generated/schema.ts` — all
three regenerate with `cargo make openapi`. New `ts_rs::TS` derives restale
`packages/temper-ui/src/lib/types/generated/*` via `cargo make generate-ts-types`, and
`check-ts-rs-drift.sh` uses `git status --porcelain`, so ts-rs output must be **committed**, not
merely staged, before `cargo make check` goes green.

## G15 — the three client-side caches the spec names (all verified)

| Cache | Verified site | Shape |
|---|---|---|
| Vault projection directory | `crates/temper-cli/src/projection.rs:459-478`, `prune_absent_files` derives `context_dir_name` from `rows.first().and_then(\|r\| r.context_name.clone())` | `prune_context` scans only the derived directory |
| Sync subscriptions | `crates/temper-core/src/types/config.rs:78-82`, `SyncSubscriptions { contexts: Vec<String> }` | bare strings |
| Generated skill file | `crates/temper-cli/src/commands/skill.rs:633`, `format_context_list(&config.contexts)` | rendered into `SKILL.md` |

All three are **out of scope** (spec §"Stated exclusion", register §"Stated silence"). Recorded here
so an implementer who trips over one knows it is named, not missed.

---

# Part 1 — Global constraints

- **Read the spec section each task cites.** The spec carries rationale this plan deliberately does
  not restate (GD-4). This plan is an index, not a summary.
- **Leave every gate to the controller.** Do **not** run `cargo build`, `cargo check`, `cargo test`,
  `cargo nextest`, `cargo clippy`, `cargo fmt`, `cargo make *`, or `cargo sqlx prepare`. A cold build
  here runs 4–15 minutes against a 120s tool timeout. Write the code; report what needs running.
- **`#[expect(lint, reason = "...")]`, never `#[allow]`.** All public types derive `Debug`.
- **Typed structs over `serde_json::json!()`** — payloads and wire types get structs.
- **Shared wire types live in `temper-core`** with `ts_rs` / `utoipa` / `schemars` derives, and both
  sides share them. Never hand-mirror a Rust struct in TypeScript or in a MCP input schema.
- **Auth before writes.** The gate resolves before any mutation, and the plpgsql invariant is the
  atomic backstop — not a substitute for the pre-check, and not substituted by it.
- **Migrations are immutable once applied** and must be numbered above `main`'s highest (G12).
- **No `.sqlx` regeneration by the implementer.** Note in your handoff *which* caches a task
  restaled (workspace / services / e2e) so the controller runs the right ritual in the right order.
- **Never `git checkout` to undo a probe edit.** Copy the file aside first and restore from the copy.
- **Do not add `context_renamed` to `TYPED_EVENT_NAMES` or to `tests/payload_schema.rs`** (G10).

## Both-surface obligation — and why it is already satisfied

Auth changes must land on **both** temper-api and temper-mcp or they drift. Here the design already
prevents that structurally: `ContextAdminAuthority` and `context_service::rename` live in
**temper-services**, which both surfaces depend on, and neither surface depends on the other. Both
call the same `context_service::rename`, so there is exactly one gate and no per-surface copy.
**What can still drift is the *rendering*** — MCP's `map_api_error` (G11) turns `Conflict`/
`BadRequest` into `internal_error` today. That is a real surface divergence and Task 9 owes it; the
authority decision itself cannot diverge.

## The two obligations the build inherits

Named by the spec §"Two obligations the build inherits, and they are the risky ones" and by the
register. Both are **sequenced work**, not sentences:

1. **`no-other-refusal-changes-its-voice`** — Task 1 makes the trait change; **Task 10** is the
   standing regression boundary over all **nine** existing authorities (G5), with a bite probe that
   must be demonstrated and recorded.
2. **The four-route equivalence class** — **Task 11** exercises each of the four routes to
   read-without-administration **separately**, at the gate and at every surface. Route 2's direction
   is corrected per G3.

---

# Part 2 — Tasks

Each task declares: a **CONFORM / EXTEND / AMEND** tag with its citation; the register **clauses**
it relates to; **`enables` or `witnesses`** (never both); its **test tier**; and its **acceptance
criteria**.

Tasks 1–9 are `enables`: they build mechanism, and the mechanism does not exist yet, so a witness
filed against them would bite against nothing. Tasks 10–11 are `witnesses`: their entire deliverable
is evidence over mechanism that already landed. **No task in this plan authors a witness record in
the vault** — the register's clauses stay declared-uncovered until the controller files coverage
against Tasks 10 and 11's suites at the end.

---

## Task 1 — `ScopedAuthority::denial_for(&self)`

**Tag: AMEND** — changes shared authorization machinery (`crates/temper-services/src/authz/mod.rs:104,148`,
quoted in G5). Authorized by spec §"EXTEND: `denial_for(&self)`", and the register carries its own
EXTEND note under `no-other-refusal-changes-its-voice`.

**Clauses:** `no-other-refusal-changes-its-voice`, `a-refusal-never-names-what-it-withholds`,
and it is the precondition for `refusal-discloses-no-more-than-the-caller-already-holds`.
**`enables`.**

**Test tier:** none in this task, deliberately — see the note below.

**Spec sections to read:** "Why the existing seam cannot express it", "EXTEND: `denial_for(&self)`".

**Files:** modify `crates/temper-services/src/authz/mod.rs` only.

**The whole change, from the spec:**

```rust
fn denial_for(&self) -> ApiError { Self::denial() }
```

and `authorize` calls `denial_for` where it currently calls `A::denial()` (G5 quotes both sites).

**Invariants, quoted from the spec — carry them into the doc comment, do not paraphrase:**

> All six existing impls are untouched — the default delegates to what they already declare.

(Read "six" as **nine** — G5. The claim is unchanged; the count is not.)

> `&self` exposes the **arm enum**, which carries no subject data. A refusal still structurally
> cannot name the subject it refused.

**CONFORM — `denial()` stays on the trait.** The static method is what makes the no-subject property
structural (`read_gates.rs:68-70`, G5). `denial_for` is a *dispatch* widening, not a replacement.
Do not delete `denial()`, and do not give `denial_for` access to `Self::Subject`.

- [ ] **Step 1** — Add the defaulted method beside `denial()` in the trait, with a doc comment that
      states (a) why it exists (one gate, two dialects), (b) that overriding it on an existing
      authority is a change to that authority's refusal voice and is guarded by
      `crates/temper-services/src/authz/mod.rs`'s boundary suite (Task 10), and (c) the `&self`/no-subject
      property quoted above.
- [ ] **Step 2** — Change `authorize`'s denial arm to call `authority.denial_for()`. **`authorize`
      must have exactly one denial return path**; do not add a second branch that calls
      `A::denial()` on some condition. This is the second failure shape the spec names.
- [ ] **Step 3** — Touch no impl. Nine impls exist (G5) and every one of them must keep the default.

**Acceptance criteria:**
- `denial_for` exists on the trait with a body of exactly `Self::denial()`.
- `authorize` calls `denial_for` and nothing else in its denial path.
- `rg -n "fn denial_for" crates/temper-services/src/authz/` returns exactly **one** hit (the trait).
- No file under `crates/temper-services/src/authz/` other than `mod.rs` is modified.

**Why no test rides along.** The boundary this change opens is `no-other-refusal-changes-its-voice`,
and a suite asserting it *is* that clause's evidence. Landing it here would make this task both the
mechanism and its own witness — the exact miscast the goal discipline forbids. The suite is Task 10,
after every consumer of the trait is final.

---

## Task 2 — Migration `20260730000020_context_rename_fns.sql`

**Tag: CONFORM** — mirrors `migrations/20260715000010_context_reassign_fns.sql` (G9) in structure,
signature, error codes and comment discipline. **EXTEND** only in that no rename function exists;
authorized by spec §"Shape".

**Clauses:** `authority-is-decided-no-earlier-than-the-change`,
`replayed-history-is-not-re-adjudicated`, `one-owner-never-holds-two-of-the-same-address`
(the `42501`/`23505` raise surface), `every-completed-rename-is-attributable`. **`enables`.**

**Test tier:** `test-db` — the SQL-guard tests live in `context_service.rs`'s
`#[cfg(all(test, feature = "test-db"))]` module (Task 6). This task lands SQL only.

**Spec sections to read:** "The write is event-sourced", "Shape", "The race path must render the
same refusal as the pre-check".

**Files:** create `migrations/20260730000020_context_rename_fns.sql`.

**Mirror `20260715000010` exactly for the three pieces** (G9 quotes each):

1. `INSERT INTO kb_event_types (name, payload_schema, schema_version) VALUES ('context_renamed',
   NULL, 1) ON CONFLICT (name) DO NOTHING;` — **`NULL` payload_schema is load-bearing**: it keeps
   the name out of the published-schema invariant, which is what makes G10's "no snapshot
   regeneration" true.
2. `_project_context_renamed(p_event uuid, p_payload jsonb) RETURNS uuid` — sets **`name` and
   `slug`** from `to_name`/`to_slug`, `RAISE EXCEPTION` when `NOT FOUND`. Model on
   `_project_context_reassigned` line for line, including that it **never authorizes**.
3. `context_rename(p_payload jsonb, p_emitter uuid, p_metadata jsonb DEFAULT '{}'::jsonb,
   p_invocation uuid DEFAULT NULL, p_correlation uuid DEFAULT NULL) RETURNS uuid` — the 5-param
   act-context signature every mutation function has carried since `20260709000050`.

**The gate inside `context_rename`, per spec §"Shape":**

> The plpgsql gate is **admit/deny only**. It does not reproduce the `403`/`404` split — the Rust
> pre-check already rendered that, and this function is reached only on the race path.

So the gate is: resolve `v_actor` from `kb_entities.profile_id` for `p_emitter` (raising `42501`
when absent, as `context_reassign` does at `:70-74`); admit if `is_system_admin(v_actor)`; otherwise
require that the actor administers the context — profile-owned ⇒ `v_owner_id = v_actor`, team-owned
⇒ a direct `kb_team_members` row with `role IN ('owner','maintainer')`. Every refusal raises
`ERRCODE = '42501'`.

**CONFORM, and this is where a copy-paste bug would live:** `context_reassign` also carries a
*target-team* half (non-gating, owner/maintainer on the destination). **Rename has no target team.**
Copying that half in would refuse every rename. Copy only the "Context side" block
(`20260715000010:95-112`) and the admin bypass.

- [ ] **Step 1** — Write the header comment. State, in the migration's own words: that `kb_contexts`
      is a replay **input** table so the projector is an idempotent re-apply; that the RBAC gate is
      an invariant of the mutation half and not a caller pre-check; and that the projector never
      authorizes so replay is not re-adjudicated. Model on `20260715000010:1-8` and `:35-46`.
- [ ] **Step 2** — Seed the event type with `NULL` payload_schema.
- [ ] **Step 3** — `_project_context_renamed`. Both columns. `IF NOT FOUND THEN RAISE`.
- [ ] **Step 4** — `context_rename`: existence + current owner lookup, emitter→actor, admin bypass,
      context-side administration check, then `_event_append('context_renamed', p_emitter,
      'kb_contexts', v_context, p_payload, ...)` and `RETURN _project_context_renamed(v_ev, p_payload);`.
- [ ] **Step 5** — Confirm the file is **additive only**: `CREATE FUNCTION` (not `CREATE OR REPLACE`),
      one `INSERT ... ON CONFLICT DO NOTHING`, no `ALTER TABLE`, no `DROP`. The additive-only-on-`main`
      invariant is what keeps auto-deploy safe (`DEPLOYING.md`).

**Acceptance criteria:**
- File numbered above `20260730000010` (G12).
- Two `CREATE FUNCTION`s and one event-type `INSERT`; nothing else.
- `context_rename` raises `42501` on every refusal arm, and contains **no** target-team check.
- `_project_context_renamed` contains no `is_system_admin`, no `kb_team_members`, and no `kb_entities`.
- Report to the controller: this restales **all three** `.sqlx` caches once Task 6 lands queries
  against it; the migration alone restales none.

---

## Task 3 — Substrate plumbing: payload, event kind, seed action, write, replay

**Tag: CONFORM** — every edit site is enumerated in G10 with its `ContextReassigned` twin.

**Clauses:** `every-completed-rename-is-attributable` (the `from_*` payload fields),
`replayed-history-is-not-re-adjudicated` (the replay arm dispatches the pure projector). **`enables`.**

**Test tier:** `test-db` for the write path (via Task 6's service tests); `test-artifacts` if a
replay-roundtrip case is added (`crates/temper-substrate/tests/replay_roundtrip.rs` is
`#![cfg(feature = "artifact-tests")]`, G14). **Do not add a roundtrip case unless a reviewer asks** —
`ContextReassigned` has none, and adding one only for rename would be an asymmetry.

**Spec sections to read:** "The write is event-sourced" → "Shape", "**Payload**".

**Files:** modify `crates/temper-substrate/src/payloads.rs`, `src/events.rs`, `src/replay.rs`,
`src/writes.rs`.

**Payload, per spec:**

> **Payload** carries `context_id`, `from_name`, `from_slug`, `to_name`, `to_slug`. The projector
> needs only the `to_*` fields; the `from_*` fields exist for the trail.

Mirror `payloads::ContextReassigned` (G10) including its `#[cfg_attr(feature = "scenario-schema",
derive(schemars::JsonSchema))]`, and mirror its doc comment's shape — the existing one says
*"`from_owner_*` is recorded for the audit trail; the projector writes only the new owner."*

- [ ] **Step 1** — `payloads::ContextRenamed`. `context_id: ContextId`, four `String` fields.
- [ ] **Step 2** — `EventKind::ContextRenamed` at `events.rs:46` beside `ContextReassigned`, plus
      its `as_canonical_name` arm (`:117`) and `from_canonical_name` arm (`:158`), both spelled
      `"context_renamed"`.
- [ ] **Step 3** — `SeedAction::ContextRename { context, from_name, from_slug, to_name, to_slug,
      emitter }` at `events.rs:403`, plus its `event_type()` arm at `:462`.
- [ ] **Step 4** — the `fire_with` arm at `events.rs:1194`: build the payload, `sqlx::query_scalar!(
      "SELECT context_rename($1,$2,$3,$4,$5)", ...)` with `ctx_meta` / `ctx_inv` / `ctx_corr`, and
      return `Ok(Fired::Context(ContextId::from(id)))`. Mirror the `ContextReassign` arm exactly,
      including the `.context("context_rename returned null")`.
- [ ] **Step 5** — `replay.rs`: add `EventKind::ContextRenamed` to the `=> None` group at `:207`
      (rename is not content-bearing), and a dispatch arm beside `:540` running
      `SELECT _project_context_renamed($1,$2)`.
- [ ] **Step 6** — `writes::rename_context_with(pool, context, from: (&str, &str), to: (&str, &str),
      emitter, ctx)` modelled on `writes.rs:652-676`: `begin_scoped` → `fire_with` → `tx.commit()`.
      Carry a doc comment noting, as `writes.rs:651` does, that `kb_contexts` is a replay input
      table so the projector is an idempotent re-apply.
- [ ] **Step 7** — **Do not touch** `payloads::TYPED_EVENT_NAMES` or
      `crates/temper-substrate/tests/payload_schema.rs` (G10). Adding the name there restales two
      snapshot suites and changes what the boot-seed stamps into `kb_event_types.payload_schema`.

**Acceptance criteria:**
- `rg -n "ContextRenamed|ContextRename" crates/temper-substrate/src/` returns hits at exactly the
  nine sites G10 enumerates for `ContextReassign`, and nowhere else.
- `TYPED_EVENT_NAMES` is still `[&str; 21]`; `tests/fixtures/payloads/` gains no file.
- `rename_context_with`'s signature is the only new `pub` item in `writes.rs`.
- The `fire_with` arm names the SQL function `context_rename` (matching Task 2 exactly).

---

## Task 4 — Wire types in `temper-core`

**Tag: CONFORM** — mirrors `ReassignContextRequest` / `ReassignContextOutcome`
(`crates/temper-core/src/types/context.rs:103-163`, derive stack quoted in G11's neighbourhood).

**Clauses:** supports the register's *"The caller is told the new address in the response, without
having to derive it"*. **`enables`.**

**Test tier:** unit (`cargo make test`) — serde round-trip only.

**Spec sections to read:** "Surfaces" (the outcome-type paragraph).

**Files:** modify `crates/temper-core/src/types/context.rs`.

**The outcome shape, per spec:**

> The outcome type carries `context_id`, `name`, `slug`, `owner_ref`, `renamed: bool` — mirroring
> `ReassignContextOutcome` — **and the composed new ref**. `owner_ref` + `slug` are the two halves a
> caller would otherwise have to assemble themselves [...] The caller has just had their address
> changed out from under them; making them reconstruct the new one from parts is the wrong place to
> save a field.

**⚠️ The spec names the composer `format_context_ref` (`context_ref.rs:75-83`). No such function
exists.** The real one is:

```rust
// crates/temper-core/src/context_ref.rs:77-84
pub fn decorated_context_ref(
    owner_table: &str,
    owner_addressable: &str,
    context_slug: &str,
) -> String {
    let sigil = if owner_table == "kb_teams" { '+' } else { '@' };
    format!("{sigil}{owner_addressable}/{context_slug}")
}
```

Its `owner_addressable` parameter is the **bare** handle/slug with no sigil, while
`ContextRow.owner_ref` (`context_service.rs:41-47`) is already **sigil-decorated** by a SQL `CASE`.
Do not feed one to the other. Composing `format!("{owner_ref}/{slug}")` from the already-decorated
`owner_ref` is the shape the service has on hand; whichever the implementer picks, the two must not
be mixed, and the choice belongs in **one** place, not at each surface.

- [ ] **Step 1** — `RenameContextRequest { name: String }`, with the four `cfg_attr` derives
      (`typescript` + `ts(export, export_to = "context.ts")`, `web-api`, `mcp`) and
      `#[derive(Debug, Clone, Serialize, Deserialize)]`, matching `ReassignContextRequest:106-115`.
- [ ] **Step 2** — `RenameContextOutcome { context_id, name, slug, owner_ref, context_ref, renamed }`,
      same derive stack plus `PartialEq, Eq` as `ReassignContextOutcome:149` carries. Document each
      field, and document `renamed: false` as the no-op arm.
- [ ] **Step 3** — Note in the handoff that this restales the ts-rs tree
      (`cargo make generate-ts-types`) and, once Task 7 lands the route, `openapi.json` + the gem +
      `schema.ts` (`cargo make openapi`). ts-rs output must be **committed**, not just staged (G14).

**Acceptance criteria:**
- Both types carry all four `cfg_attr` derive lines in the same order as their `Reassign` twins.
- `RenameContextRequest` has exactly one field. No `slug` parameter — spec §"Out of scope →
  Rejected": *"An independent `--slug` parameter. Rename is one field."*
- The composed ref field is named and documented as such, and its composition is not duplicated.

---

## Task 5 — `ContextAdminAuthority`

**Tag: EXTEND** — a new `ScopedAuthority` impl; authorized by spec §"`ContextAdminAuthority`".
**CONFORM** on every arm's predicate: each arm *calls* its incumbent (G2, G6), none restates one —
the rule `authz/mod.rs:12-16` states as this layer's whole reason for existing.

**Clauses:** `rename-requires-administration`,
`refusal-discloses-no-more-than-the-caller-already-holds`,
`a-reader-is-never-told-a-readable-context-is-absent`, `system-authority-never-becomes-ownership`.
**`enables`.**

**Test tier:** `test-db` — `resolve` takes a `&PgPool`.

**Spec sections to read:** "`ContextAdminAuthority`", "**Probe order, and it is not the obvious
one**", "The `404` is the incumbent refusal".

**Files:** create `crates/temper-services/src/authz/context_admin.rs`; modify
`crates/temper-services/src/authz/mod.rs` (`mod` + `pub(crate) use`); modify
`crates/temper-services/src/services/context_service.rs` (widen `CONTEXT_REFUSAL` visibility).

**The enum and the probe order, verbatim from the spec:**

```rust
pub(crate) enum ContextAdminAuthority {
    Administers,   // profile owner, or can_manage on the owning team
    SystemAdmin,   // admits; confers no ownership
    ReadOnly,      // visible but not administered  -> 403
    Invisible,     // not visible                   -> 404
}
```

> 1. `context_service::caller_administers_context` → `Administers`
> 2. `access_service::is_system_admin` → `SystemAdmin`
> 3. `context_visible_to` → `ReadOnly`
> 4. otherwise → `Invisible`

Both orderings are load-bearing and both have a stated reason. Carry them into doc comments:

> Admin sits at **2, not 1**, following the reasoning `TeamReadAuthority` already records
> (`read_gates.rs:45-46`): the common caller is the administrator, and probing `is_system_admin`
> first charges every one of them an extra round-trip.

> Admin must sit **above 3**, and this is load-bearing. [...] There is **no system-admin branch**
> [in `contexts_readable_by`]. A visibility-first ordering would render `404` to a system admin
> renaming a context they do not otherwise read — the exact actor the feature must admit.

G2 is the live evidence for the second. Cite it in the doc comment.

**Two `is_denial` arms, two dialects — this is the whole point of Task 1:**

- `is_denial` is true for **both** `ReadOnly` and `Invisible`.
- `denial()` (the static default) returns `ApiError::NotFound(CONTEXT_REFUSAL.to_string())` — the
  safe fallback, so a caller reaching the static path can never receive the more disclosive answer.
- `denial_for(&self)` returns `ApiError::Forbidden` for `ReadOnly` and
  `ApiError::NotFound(CONTEXT_REFUSAL.to_string())` for `Invisible`. The admitting arms are
  unreachable here (`authorize` calls it only when `is_denial`); handle them by delegating to
  `Self::denial()` rather than by `unreachable!()`.

**CONFORM — the `404` is the incumbent constant, not a new literal** (G7, spec §"The `404` is the
incumbent refusal"). `CONTEXT_REFUSAL` is currently private to `context_service`; widen it to
`pub(crate)` and import it. **Do not write the string a second time** — that is precisely what its
own doc comment (G7) forbids, and `the_three_handle_slug_refusals_are_indistinguishable`
(`crates/temper-api/tests/context_ref_resolve_test.rs:320`) is what would notice.

**Why the `403` is not an oracle** — carry the spec's argument into the module doc, because a later
"let's make the denials consistent" pass will read that doc before it reads the spec:

> **This is not an oracle.** The `403` goes only to principals who already read the context — they
> learn nothing a `GET` would not already have told them. Refusal detail stays bounded by what the
> caller already has standing to know.

**Subject:** `Uuid` (the context id), per spec. Not `ContextId` — `TwoSidedObject::Context(Uuid)`
and `TeamReadAuthority::Subject = Uuid` both use the bare `Uuid` at this seam.

- [ ] **Step 1** — Write the module, modelled on `read_gates.rs` (which is the closest sibling: a
      read-shaped gate whose refusal is `NotFound` and whose module doc explains why). Match its
      import list, its `#[derive(Debug, Clone, Copy, PartialEq, Eq)]`, and its comment density.
- [ ] **Step 2** — `resolve` in the spec's order, each arm calling its incumbent. `caller_administers_context`
      is `pub(crate)` already (G6) — import it, do not re-derive it.
- [ ] **Step 3** — `is_denial`, `denial`, `denial_for` as above.
- [ ] **Step 4** — Register in `authz/mod.rs`: `mod context_admin;` and
      `pub(crate) use context_admin::ContextAdminAuthority;`, placed alphabetically among the
      existing `mod`/`use` blocks (`mod.rs:28-56`).
- [ ] **Step 5** — Widen `CONTEXT_REFUSAL` to `pub(crate)` in `context_service.rs:108` and import it.
      Leave its doc comment intact and add one clause noting the second consumer.
- [ ] **Step 6** — Co-located `#[cfg(all(test, feature = "test-db"))] mod tests` covering
      **arm resolution only** (the mechanism's own unit hygiene): profile owner → `Administers`;
      team owner and team maintainer → `Administers`; system admin who cannot read → `SystemAdmin`;
      a stranger → `Invisible`; a nonexistent context id → `Invisible`. Follow the fixture idiom in
      `context_service.rs:789-846` (`mk_profile_ent`, `mk_team`, `add_member`, `mk_personal_context`,
      `mk_team_context`) — runtime `sqlx::query(...)` for fixture inserts, per project convention.
      **The four ReadOnly routes are NOT here** — they are Task 11, which is their witness.

**Acceptance criteria:**
- `resolve`'s body contains no SQL. Every arm is a call to `caller_administers_context`,
  `access_service::is_system_admin`, or the `context_visible_to` probe already spelled at
  `context_service.rs:228-245` (`ensure_context_visible`) — reuse it or its query, do not write a
  third copy of `SELECT context_visible_to($1, $2)`.
- `rg -n "not found or not readable" crates/temper-services/src/authz/context_admin.rs` returns
  **zero** hits — the string comes from `CONTEXT_REFUSAL`.
- `is_denial` returns true for exactly `ReadOnly` and `Invisible`.
- The probe order matches the spec's numbered list, and each ordering constraint has a doc comment
  citing its reason.

---

## Task 6 — name canonicalization, `context_service::rename`, and the bundled `reassign` `23505` fix

**Tag: AMEND** — adds a service function and **changes an existing one's error mapper**
(`context_service.rs:661-668`, G8). Authorized by spec §"DECIDED — `reassign`'s identical hole rides
along in the same change".

**Clauses:** `a-rename-lands-where-it-was-asked-or-nowhere`,
`one-owner-never-holds-two-of-the-same-address`, `every-completed-rename-is-attributable`,
`authority-is-decided-no-earlier-than-the-change`, `a-context-never-loses-its-contents-to-a-rename`,
`a-stored-name-has-one-spelling`,
`a-request-that-would-change-stored-state-is-never-declined-as-a-no-op`. **`enables`.**

**Test tier:** `test-db`.

**Spec sections to read:** "Scope: one field, slug derived", "Refusals" (the whole table and both
divergence paragraphs), "The race path must render the same refusal as the pre-check", "DECIDED",
"What a rename does not touch".

**Files:** modify `crates/temper-services/src/services/context_service.rs`.

**The refusal order is the spec's, and it is an order, not a set:**

| Condition | Response |
|---|---|
| derived slug is empty | `400 BadRequest` |
| **canonical name** equals the stored name | `200`, `renamed: false`, **no event** |
| derived slug taken by **another** context under the same owner | `409 Conflict`, naming the colliding context |
| caller lacks authority | `403` / `404` per `ContextAdminAuthority` |

**Auth before writes** puts the gate first in code even though it is last in the table: `authorize::<
ContextAdminAuthority>` runs before any of the three checks. Model the call on
`context_service.rs:531-536`.

**Both divergences from `create` are deliberate — carry the reasons, quoted:**

> **The empty-slug `400` is a deliberate divergence from `create`.** `next_unique_context_slug` falls
> back to the literal `"context"` when `sluggify` yields nothing [...] That is tolerable at birth and
> wrong at rename: `--name "!!!"` must not silently re-address a context to `context`.

> **The `409` is a deliberate divergence from `create`.** [...] Rename does **not** use that
> function. A rename is a deliberate re-address, and silently landing on `notes-2` gives the caller
> an address they did not ask for and were not told about.

So: **`rename` must not call `next_unique_context_slug`** (G13 quotes both of its offending
behaviours). It calls `sluggify` directly.

**The `409` names the colliding context, and that is not a leak** — spec: *"reaching the gate at all
means the caller administers this context, so they are the owner or manage the owning team, and can
already enumerate that owner's contexts."* Model the message on `context_service.rs:572-575` (G8).

**No-op idempotency** mirrors `reassign`'s (`:551-559`, G8) in shape, but **not** in what it
compares. **Settled 2026-07-30 — compare the canonical NAME, never the derived slug.** An earlier
draft of this plan said slug; the spec §"Refusals" now records why that is wrong twice over (a
pre-canonicalization name sluggifies identically to its own repaired form, so slug-comparison would
permanently decline the one rename that fixes it; and two different names can share a slug, so it
would swallow a real display-name change and report success).

So **a rename whose slug does not move is still a rename** — it writes `name`, emits, and returns
`renamed: true`, with `from_slug == to_slug` in the payload. Only a canonical name identical to the
stored one is a no-op. The register's
`a-request-that-would-change-stored-state-is-never-declined-as-a-no-op` is this boundary.

**⚠️ The collision check MUST exclude the context being renamed.** `reassign`'s query
(`context_service.rs:563-576`, G8) has no self-exclusion and correctly does not need one — it
changes the *owner*, so the row cannot match itself. Rename keeps the owner fixed. Copied verbatim,
a name-only rename reaches the collision check (it is no longer swallowed by the no-op arm) and
**409s against its own slug**, breaking exactly the legacy-repair case the no-op rule exists to
enable. Add `AND id <> <context_id>`.

**The bundled fix, verbatim from the spec:**

> A `23505` unique violation therefore surfaces as a **500**. Rename's mapper must carry a `23505`
> arm rendering the same `409` the pre-check renders, or the caller's experience depends on how
> quickly they lost the race.

> **Decided 2026-07-30: bundle.** [...] The implementation plan must therefore treat `reassign`'s
> `23505` mapping as in scope, and its commit must say that rename's tests are what surfaced it.

**SG-3 applies here.** The `42501`→`403` and `23505`→`409` mapping is now needed by two call sites
and would drift if written twice. Extract **one** mapper both use, or make `map_reassign_write_err`
the shared one and rename it. Do not leave two functions that differ only in the `409` message —
that is the "correct mapper beside an incorrect one" artifact the spec's DECIDED section exists to
prevent, reintroduced one level down.

- [ ] **Step 0 — the shared name canonicalizer, called by BOTH write paths.** Add one helper that
      trims and collapses internal whitespace runs to a single space. **It must not ASCII-fold** —
      a slug is an address and may be lossy, a name is a display label and may not (`Café` stays
      `Café`), so do **not** reuse `sluggify`'s fold (G13 shows what it does). Call it from `rename`
      **and from `create` (`context_service.rs:337-372`)**. Spec §"Names are canonicalized before
      they are stored — on both write paths": *"A canonical-form invariant honoured by one of two
      write paths is not an invariant — rename would be a repair affordance for a hole `create`
      keeps digging."* This is the plan's only change to already-shipped `create` behavior; call it
      out in that commit body. **AMEND**, authorized by that spec section and by the register's
      `a-stored-name-has-one-spelling`.
- [ ] **Step 1** — Extract/extend the write-error mapper to carry both a `42501` arm and a `23505`
      arm. Its doc comment must state that the `23505` arm exists because the pre-check and the
      race path must render the same refusal, and that `reassign` had the same hole.
- [ ] **Step 2** — Point `reassign` at it (`context_service.rs:590`). **No other change to
      `reassign`.**
- [ ] **Step 3** — `pub async fn rename(pool, caller: ProfileId, context_id: Uuid, name: &str) ->
      ApiResult<RenameContextOutcome>`: authorize → read current `(owner_table, owner_id, slug,
      name)` → **canonicalize the incoming name (Step 0)** → derive slug from the canonical name →
      empty-slug `400` → **no-op if canonical name == stored name** `200` → collision `409`
      (**self-excluded**) → `resolve_emitter(pool, caller, "web")` →
      `writes::rename_context_with(...)` mapped through the shared mapper → compose the outcome.
- [ ] **Step 4** — Compose `owner_ref` the way `reassign` does. **`team_owner_ref`
      (`context_service.rs:671-676`) is team-only** and will `fetch_one`-panic on a
      profile-owned context; rename must serve both owner kinds. The `CASE owner_table WHEN
      'kb_teams' THEN '+' || ... ELSE '@' || ...` expression at `context_service.rs:42-46` is the
      incumbent both-kinds spelling — reuse that shape rather than extending `team_owner_ref`.
- [ ] **Step 5** — Tests in the existing `#[cfg(all(test, feature = "test-db"))]` module. At minimum:
      empty-slug `400` for **both** `"   "` and `"!!!"`; no-op `renamed: false` **and zero new
      `kb_events` rows**; collision `409` with the colliding slug in the message; a successful rename
      changing **both** columns; plus the three the canonicalization decision adds —
      (a) a **name-only rename** (canonical name differs, slug identical) returns `renamed: true`,
      writes `name`, emits, and does **not** 409 against itself — the self-exclusion regression test;
      (b) a context whose stored name is non-canonical can be renamed to its own canonical form,
      which is the legacy-repair case and would fail under slug-comparison; and
      (c) `create` stores a canonical name — the Step 0 helper's second caller, untested otherwise;
      the
      SQL-guard test calling `context_rename` **directly** with an unauthorized emitter and asserting
      `42501` + unchanged row (model on `sql_guard_rejects_unauthorized_emitter_directly`,
      `context_service.rs:1063-1104`, which shows the `emitter_of` helper at `:1046`); and the
      admin-direct allow case (model on `:1108`).
- [ ] **Step 6** — A test for `a-context-never-loses-its-contents-to-a-rename`: home a resource in
      the context, rename, assert the `kb_resource_homes` row and the resource's readability are
      unchanged. Spec §"What a rename does not touch" argues this holds *because nothing is keyed by
      slug* — the test is cheap and makes the argument falsifiable rather than merely stated.

**Acceptance criteria:**
- `rg -n "next_unique_context_slug" crates/temper-services/src/services/context_service.rs` shows it
  called from `create` only.
- Exactly **one** name-canonicalizing helper, called from both `create` and `rename`, and it does not
  call `sluggify`.
- The no-op arm compares the canonical **name**. `rg` the function body: no comparison of a derived
  slug against `cur.slug` gates the no-op return.
- The collision query excludes the context being renamed.
- Exactly **one** write-error mapper function in the file, with both `42501` and `23505` arms, called
  by both `rename` and `reassign`.
- `rename` calls `authorize::<ContextAdminAuthority>` before every read and every write.
- The no-op arm performs no `writes::*` call.
- Report: this restales the **workspace** `.sqlx` cache and **`crates/temper-services/.sqlx`**
  (test-target queries), in that order.

---

## Task 7 — API route, handler, OpenAPI

**Tag: CONFORM** — mirrors `handlers::contexts::reassign` (`handlers/contexts.rs:147-176`, G11) and
its route registration (`routes.rs:102`).

**Clauses:** the register's closure class 3 (*"Surface of origin is immaterial"*). **`enables`.**

**Test tier:** e2e (Task 10/11 exercise it). No handler-level test — the handler is a five-line
passthrough, and testing it in isolation tests axum.

**Spec sections to read:** "Surfaces" (the API-path-shape paragraph).

**Files:** modify `crates/temper-api/src/handlers/contexts.rs`, `crates/temper-api/src/routes.rs`.

**Path shape, per spec:**

> The API path follows the verb-subpath shape `POST /api/contexts/{id}/reassign` already uses
> (`handlers/contexts.rs:150`), not a `PATCH` on the collection member — rename is an act with a
> refusal face, not a field edit.

- [ ] **Step 1** — `POST /api/contexts/{id}/rename`. Copy the `#[utoipa::path]` block from
      `handlers/contexts.rs:147-161` and adjust: `request_body = RenameContextRequest`,
      `body = RenameContextOutcome`, and responses for **200 / 400 / 403 / 404 / 409**. All five
      matter — the 403/404 split is the substance of the feature and an OpenAPI that documents only
      one of them describes a different act.
- [ ] **Step 2** — The handler body: extract `ProfileId::from(auth.0.profile().id)`, call
      `context_service::rename`, `Ok(Json(outcome))`. Nothing else. Mirror `reassign` line for line.
- [ ] **Step 3** — Register at `routes.rs` beside `:102`.
- [ ] **Step 4** — Report: `cargo make openapi` regenerates `openapi.json`, the temper-rb gem, and
      `clients/temper-ts/src/generated/schema.ts`; `cargo make generate-ts-types` regenerates the
      ts-rs tree for Task 4's types. Per G14 the ts-rs gate needs a **commit**, not just `git add`.

**Acceptance criteria:**
- The handler contains no authorization logic, no SQL, and no error mapping.
- The `#[utoipa::path]` block documents all five statuses.
- No committed generated artifact is hand-edited.

---

## Task 8 — `temper-client` method and `temper context rename`

**Tag: CONFORM** — mirrors `TemperClient::contexts().reassign` (`temper-client/src/contexts.rs:95-106`)
and the CLI's `transfer` trio (`cli.rs:891-897`, `main.rs:430-442`, `context_cmd.rs:245-262`), all
quoted in G11.

**Clauses:** closure class 3. **`enables`.**

**Test tier:** e2e (`tests/e2e/`, `test-db`) — the CLI path is only meaningfully exercised through
the real binary against the real server.

**Spec sections to read:** "Surfaces", "Scope: one field, slug derived" (the re-addressing
consequence the CLI must surface to the operator).

**Files:** modify `crates/temper-client/src/contexts.rs`, `crates/temper-cli/src/cli.rs`,
`crates/temper-cli/src/main.rs`, `crates/temper-cli/src/commands/context_cmd.rs`.

**CLI shape, per spec:** `temper context rename <ref> --name <name>`.

**Mirror, do not re-derive** (this is the deeper fix from the plan-authoring gates — reference the
pattern by `file:line` rather than regenerating it):

- The client method mirrors `contexts.rs:95-106`'s exact body shape, including
  `let token = self.http.resolve_token()?;` and
  `self.http.send_json(&Method::POST, &path, req, Some(&token)).await`.
- The `main.rs` dispatch mirrors `main.rs:430-442`: `temper_cli::actions::runtime::with_client(|client|
  Box::pin(async move { ... }))`. **`with_client` takes no config argument and the closure body is
  `Box::pin`-ed** — both are easy to get wrong from memory.
- The action mirrors `transfer_remote` (`context_cmd.rs:245-262`), including
  `resolve_context_id_for_read` (`:291`, the `@me`-accepting resolver — `transfer`'s choice, and
  rename's headline flow is `@me/my-project` too) and `crate::format::render(&outcome, fmt)`.

**CLI commands are thin wrappers.** Business logic lives in `src/actions/`; `context_cmd.rs` is the
established home for this family, so follow the file — but the action must contain no policy, only
resolve-call-render.

**Error mapping:** `map_share_err` (`context_cmd.rs:206-220`) enriches a bare `Forbidden` with the
*share* requirement ("...AND manage the target team"). **That message is wrong for rename** — rename
has no target team. Write a rename-specific arm, or parameterize; do not reuse the string.

- [ ] **Step 1** — `pub async fn rename(&self, context_id: Uuid, body: &RenameContextRequest) ->
      Result<RenameContextOutcome>` in `temper-client/src/contexts.rs`.
- [ ] **Step 2** — `ContextAction::Rename { context: String, #[arg(long)] name: String }` in
      `cli.rs` beside `Transfer` (`:893`), with a doc comment that states the re-addressing consequence plainly. Spec: *"After
      renaming `@me/temper` to `"Temper KB"`, the ref `@me/temper` no longer resolves and
      `@me/temper-kb` does. Every stored `@owner/slug` string held by anyone, anywhere, is stale."*
      **Also state that local vault directories and sync subscriptions are not updated** — spec
      §"Stated exclusion" chooses not to print a runtime advisory, but the help text is where an
      operator can be told once, at no structural cost.
- [ ] **Step 3** — Dispatch in `main.rs` beside `:430`.
- [ ] **Step 4** — `rename_remote` in `context_cmd.rs` beside `transfer_remote`.

**Acceptance criteria:**
- `with_client` is called with exactly one argument and a `Box::pin`-ed body.
- No new HTTP configuration — the client method uses the existing `self.http` helpers.
- The `Forbidden` message does not mention a target team.
- The `Rename` variant's doc comment states both the re-addressing consequence and that local vault
  directories / sync subscriptions are not updated — i.e. it is visible in `temper context rename --help`.

---

## Task 9 — MCP `rename_context`, and its error rendering

**Tag: CONFORM** on the tool shape (`tools/contexts.rs:159-178` + `service.rs:735-745`, G11).
**AMEND** on `map_api_error` (`tools/contexts.rs:17-33`), which has no `Conflict`/`BadRequest` arm
and whose `Forbidden` message names a target team.

**Clauses:** closure class 3; and `refusal-discloses-no-more-than-the-caller-already-holds` at the
MCP surface, since a rendering that collapses `403` and `404` into indistinguishable text would
break the pairing the register calls *"the substance of this element"*. **`enables`.**

**Test tier:** e2e (`tests/e2e/`, `test-db`) — MCP tools are exercised there
(`tests/e2e/tests/mcp_round_trip_test.rs`, `#![cfg(feature = "test-db")]`).

**Spec sections to read:** "Surfaces" (the MCP-parity paragraph).

**Files:** modify `crates/temper-mcp/src/tools/contexts.rs` and `crates/temper-mcp/src/service.rs`.

**Why MCP is in scope at all, per spec:**

> MCP is included for parity: `contexts.rs` already exposes `create_context`, `get_context`,
> `list_contexts`, `share_context`, `unshare_context` and `transfer_context`. A rename absent from
> that set would be the only context act an agent cannot perform.

- [ ] **Step 1** — `RenameContextInput { context: Uuid, name: String }` with
      `#[derive(Debug, Deserialize, JsonSchema)]`, mirroring `TransferContextInput`
      (`tools/contexts.rs:52-59`). Doc-comment every field — MCP renders them into the tool schema.
- [ ] **Step 2** — The tool body, SERVICE-DIRECT via `context_service::rename`, mirroring
      `transfer_context` (`tools/contexts.rs:159-178`) including its doc comment's SERVICE-DIRECT
      note.
- [ ] **Step 3** — Add `Conflict` and `BadRequest` arms to `map_api_error` so each renders as
      `invalid_params` carrying the service's own message. **Carry the message, do not replace it** —
      the `NotFound` arm already does this, and its comment (`tools/contexts.rs:26-27`) states the
      reason: the arm *"names which of the context or the team was unresolvable, which this arm
      could only guess at"*. The `409` names the colliding slug; that is the actionable half.
- [ ] **Step 4** — The `Forbidden` arm's message currently asserts a target-team requirement. Make
      it correct for both callers — either parameterize it on the `context` label already passed in,
      or split the arm. Do not leave rename rendering the share requirement.
- [ ] **Step 5** — The `#[tool(description = ...)]` delegating method in `service.rs` beside `:735`.
      **Both edits are required**; without `service.rs` the tool is never exposed.
- [ ] **Step 6** — Extend the existing `#[cfg(test)] mod tests` (`tools/contexts.rs:203-235`) with
      an input-deserialization test and an assertion that `Conflict` and `BadRequest` no longer land
      in `internal_error`.

**Acceptance criteria:**
- `map_api_error`'s `other =>` arm no longer swallows `Conflict` or `BadRequest`.
- The `Forbidden` message rendered for rename does not mention a target team.
- `rg -n "rename_context" crates/temper-mcp/src/` hits **both** `tools/contexts.rs` and `service.rs`.
- The tool body contains no authorization logic.

---

## Task 10 — ⚖️ The standing regression boundary on shared authorization machinery

**Tag: CONFORM** — asserts existing behaviour of nine impls enumerated in G5.

**Clauses:** `no-other-refusal-changes-its-voice`, `a-refusal-never-names-what-it-withholds`.
**`witnesses`** — every mechanism it measures landed in Tasks 1 and 5, and this task's entire
deliverable is evidence.

**Test tier:** unit (`cargo make test`). `denial()` and `denial_for()` take no pool, so the suite
needs no database and runs in the cheapest, most-often-run tier. That is deliberate: a standing
boundary that only runs under `test-db` is a boundary most contributors never trip.

**Spec sections to read:** "Two obligations the build inherits, and they are the risky ones" —
in particular:

> `no-other-refusal-changes-its-voice` is a standing regression boundary on shared authorization
> machinery; the build owes it something that would fail if the boundary moved, not a paragraph
> asserting it cannot.

**Files:** modify `crates/temper-services/src/authz/mod.rs` (add a `#[cfg(test)] mod tests`).

**What the suite asserts.** For **each of the nine** authorities in G5, for **each denial arm**:
`arm.denial_for()` and `A::denial()` must render identically — same `ApiError` discriminant **and**
same `Display` string. Comparing only the discriminant would let a `NotFound` message change
silently, and the messages are exactly what `a-refusal-never-names-what-it-withholds` is about.

Two further assertions, each guarding a distinct failure shape the spec names:

- **The count.** Assert that the suite covers nine authorities, with a comment saying that a tenth
  `impl ScopedAuthority` must be added here in the same PR that introduces it. A boundary that
  silently ignores new members is not standing.
- **The exception is declared, not discovered.** `ContextAdminAuthority` is the **only** authority
  whose `denial_for` diverges from `denial`. Assert that too — positively, by name — so the suite
  states *"exactly one authority speaks two dialects"* rather than *"these nine do not"*.

- [ ] **Step 1** — Copy `crates/temper-services/src/authz/mod.rs` aside before probing
      (`cp .../mod.rs /tmp/…`). **Never `git checkout` to undo a probe edit.**
- [ ] **Step 2** — Write the suite. Every arm value is `pub(crate)` and constructible from inside
      `temper-services`; some enums are re-exported from `mod.rs` (G5's table gives each file), so
      import them directly from their modules where `mod.rs` does not re-export.
- [ ] **Step 3 — the bite probe, and record it.** A test that passes because a defaulted method
      cannot differ is not yet evidence of anything. Prove it bites:
      1. Temporarily add `fn denial_for(&self) -> ApiError { ApiError::Forbidden }` to
         **`TeamReadAuthority`** — an authority whose `denial()` is `NotFound`, so the override is a
         real change of voice and exactly the failure the boundary exists to catch.
      2. Hand the file to the controller to run the unit tier. **Expected: FAIL**, naming
         `TeamReadAuthority`.
      3. Restore from the Step 1 copy. Re-run. **Expected: PASS.**
      4. Record both outcomes verbatim in the PR body. **An unprobed boundary is a paragraph.**
- [ ] **Step 4** — Assert `ContextAdminAuthority`'s two dialects positively: `ReadOnly.denial_for()`
      is `Forbidden`; `Invisible.denial_for()` is `NotFound(CONTEXT_REFUSAL)` and is **byte-identical
      to `get_visible`'s refusal** (G7) — read the constant, do not re-type the string.
- [ ] **Step 5** — Confirm `authz/audit_gate.rs:897`'s `every_denial_renders_not_found` still passes
      unchanged. It is the pre-existing instance of this same defence at n=1, and it must not need
      editing; if it does, the trait change moved something it should not have.

**Acceptance criteria:**
- Nine authorities covered; the count is asserted, not implied by the list's length.
- Every comparison checks discriminant **and** rendered string.
- The bite probe's FAIL output and the restored PASS output are pasted into the PR body.
- `every_denial_renders_not_found` is untouched.

**What this task does not do.** It does not file a witness record. It produces the evidence a
witness for `no-other-refusal-changes-its-voice` will cite; filing coverage against the register is
the controller's step, after the branch is green.

---

## Task 11 — ⚖️ The four-route equivalence class, exercised separately

**Tag: CONFORM** — every route is a `contexts_readable_by` arm or a `kb_access_grants` row, all four
proven live in G4; route 2's direction corrected per G3.

**Clauses:** `a-reader-is-never-told-a-readable-context-is-absent`,
`refusal-discloses-no-more-than-the-caller-already-holds`, `rename-requires-administration`,
`system-authority-never-becomes-ownership`, and the register's closure classes 1 and 3.
**`witnesses`** — all mechanism landed in Tasks 5–9.

**Test tier:** **two tiers, and both are needed.**
- `test-db` (`crates/temper-services/src/authz/context_admin.rs`'s test module) for the four routes
  at the **gate**: each resolves to `ReadOnly`, and `denial_for` renders `403`.
- `test-e2e` (`tests/e2e/tests/context_rename_e2e.rs`) for **surface parity**: the same refusal over
  HTTP, over the CLI, and over MCP — because class 3 claims the surface is immaterial and MCP
  renders through a mapper the gate never sees (G11).

**Spec sections to read:** "Two obligations the build inherits" — in particular:

> They arrive through genuinely different machinery: three are `UNION` arms of `contexts_readable_by`
> and one is `kb_access_grants`. The claim holds structurally, since `caller_administers_context`
> consults neither, but a class claimed across four mechanisms is exactly the shape that hides an
> unexamined cell. The build owes each route separately, not one representative.

**⚠️ Read G3 before writing route 2.** The spec's worked example is directionally inverted. Route 2's
actor is a **maintainer of a DESCENDANT team**, reading a context owned by the **ancestor** team.
Built the spec's way (parent-maintainer, child-owned context) the actor is `Invisible` and the test
will assert `403` and get `404`; "fixing" the gate to make that pass would be a real authorization
widening. G4's probe is the fixture shape to reproduce.

**The four routes, each its own test, each named for its mechanism** (G4's labels):

| # | Route | Mechanism |
|---|---|---|
| 1 | Member at a lower role on the **owning** team | `contexts_readable_by` arm 2 |
| 2 | Maintainer of a **descendant** team; context owned by the ancestor | `contexts_readable_by` arm 2 via `team_ancestors` |
| 3 | Owner of a team the context is **shared** to | `contexts_readable_by` arm 3 (`kb_team_contexts`) |
| 4 | Holder of an explicit profile-anchored **read grant** | `contexts_readable_by` arm 4 (`kb_access_grants`) |

**One representative is not four tests.** Parameterizing over a `Vec` of seeded profiles is fine;
collapsing to a single seeded actor is not — the whole point is that four mechanisms are being
checked, and a loop over four fixtures is four checks.

- [ ] **Step 1** — Gate-level: four `#[sqlx::test(migrations = "../../migrations")]` tests in
      `context_admin.rs`, each seeding exactly one route and asserting `resolve` → `ReadOnly` and
      `denial_for` → `ApiError::Forbidden`. Reproduce G4's fixture shape; `kb_access_grants` requires
      a non-null `granted_by_profile_id` (G4's first probe failed on exactly that).
- [ ] **Step 2** — The negative pole, in the same file: a stranger resolves `Invisible` and renders
      `NotFound(CONTEXT_REFUSAL)`. Without it the four `403`s prove nothing about the split — a gate
      that returned `403` to everyone would pass all four.
- [ ] **Step 3** — `system-authority-never-becomes-ownership`: a system admin renames a context they
      cannot otherwise read (the G2 case); assert the rename succeeds **and** that no
      `kb_team_members` row, no `kb_access_grants` row and no `kb_contexts.owner_*` value changed.
      The register's clause is about *residue*, so the assertion is about what did **not** appear.
- [ ] **Step 4** — E2E surface parity, `tests/e2e/tests/context_rename_e2e.rs`. Model the harness on
      `tests/e2e/tests/context_transfer_e2e.rs` (its `provision` / `root_bootstrap_first_admin`
      helpers at `:24-58` and the `common::approve` / `common::approved_admin` idiom). Assert, for
      **one** representative reader and for the stranger:
      - HTTP: `POST /api/contexts/{id}/rename` → `403` for the reader, `404` for the stranger.
      - CLI: via `common::run_temper_cli_with_token`, **passing a bare UUID** — the decorated form
        never reaches the server for an unreadable context (G11).
      - MCP: the same two callers through the `rename_context` tool, asserting the two refusals stay
        distinguishable after `map_api_error` (Task 9).
- [ ] **Step 5** — E2E happy path: an administrator renames; the old `@owner/slug` no longer
      resolves and the new one does; the response carries the composed new ref; a resource homed in
      the context is still readable and still homed. This is the register's *"Synchronously"* and
      *"Once everything settles"* faces, over server state only.
- [ ] **Step 6** — Attributability, `test-db`: after a rename, assert a `context_renamed` row in
      `kb_events` anchored to `kb_contexts`/the context id, whose payload carries `from_name`,
      `from_slug`, `to_name`, `to_slug`, and whose emitter resolves to the acting profile. **Assert
      the `from_*` values specifically** — a payload that carried only `to_*` would satisfy every
      other test in this plan and fail the clause outright.

**Acceptance criteria:**
- Four separately-named gate tests, one per mechanism, plus the stranger pole.
- Route 2's fixture has the descendant-team actor and the ancestor-owned context (G3).
- The e2e file exercises all three surfaces for the same two callers.
- The attributability test asserts the `from_*` fields by value, not by presence.
- Report: this restales **`crates/temper-services/.sqlx`** and **`tests/e2e/.sqlx`**.

---

## Task 12 — Documentation

**Tag: CONFORM** — follows the docs pattern the sibling transfer work established.

**Clauses:** none directly; it is the *"no door offers less than another without saying so"*
obligation applied to a named remainder. **`enables`.**

**Test tier:** none.

**Files:** modify `CLAUDE.md`; add a note wherever context operations are described for operators.

- [ ] **Step 1** — A `CLAUDE.md` Key Patterns entry covering: rename takes one field and derives the
      slug; the two-dialect gate and why the `403` is not an oracle; the `denial_for` seam and that
      Task 10's suite is its boundary; and that a rename **staleness the local vault** — after a
      rename the vault holds both the old and the new context directory and nothing says so.
- [ ] **Step 2** — Record the named remainders where a reader will meet them, in the spec's own
      terms: client-side caches (three sites, G15) are **excluded by decision, not missed**; and
      *"Rate-shaped axes are OPEN and unexamined"* — nothing bounds how often a context may be
      re-addressed.
- [ ] **Step 3** — Do **not** add a runtime advisory to the CLI beyond the `--help` text in Task 8.
      Spec §"Stated exclusion": *"a partial fixup is worse than none, because it teaches the operator
      that the problem is handled."*

**Acceptance criteria:**
- The `CLAUDE.md` entry names the vault double-directory residue explicitly.
- No new reconciliation code anywhere.

---

# Part 3 — What this plan does NOT cover

**Named remainders, carried forward from the register and the spec. None is a task, and none is an
oversight.**

- **Client-side caches of the old address.** All three sites are verified on disk (G15) and none is
  reconciled or warned about at runtime. Spec §"Stated exclusion"; register §"Stated silence".
  The residue is real: *"after a rename, the vault contains both the old and the new context
  directory, and nothing says so"*, and the old directory's files still parse and still carry the
  old context in frontmatter — a live trap for an agent grepping the vault. Task 12 documents it.
- **Rate-shaped axes.** The register: *"Rate-shaped axes are OPEN and unexamined. Nothing in this
  register bounds how *often* a context may be re-addressed, and nothing examines what a rapid
  rename loop does to the trail, to anything reading the address, or to anyone holding a
  reference."* No task adds a rate limit, and no test explores a rename loop.
- **Renaming teams and profiles.** `kb_teams.slug` is globally `UNIQUE` and appears in share flows;
  different blast radius, different disclosure analysis. Deferred.
- **Widening team-context administration to transitive membership.** The asymmetry between reads
  (which inherit down the tree, G3) and administration (direct membership, G6) is inherited, not
  resolved. Changing it is a decision about the authorization model.
- **A redirect or alias from the old slug.** Would need a new table and a resolution-order question
  in `resolve_context_ref`. Not attempted.
- **An independent `--slug` parameter, auto-suffixing on rename collision, and a `--force` flag.**
  All three Rejected in spec §"Out of scope", not deferred.
- **Migrating `share`/`unshare`/`create` to the `Backend` trait.** Out of scope regardless of how
  the G11 escalation resolves.
- **Recording the originating surface on the emitter.** `resolve_emitter(pool, caller, "web")` is
  hardcoded at every service site (G8). Rename conforms; it does not fix it.
- **A replay-roundtrip scenario for `context_renamed`.** `context_reassigned` has none (G10);
  adding one only for rename would be an asymmetry. Task 3 Step 5 wires the replay arm; proving
  roundtrip byte-identity is not owed here.

---

# Part 4 — Commit and PR sequencing

**One PR.** Three reasons, in order of weight:

1. **The bundled `reassign` fix demands it.** Spec §DECIDED: *"extracting would land a correct
   mapper beside an incorrect one in the same file, which is worse than either alternative"*, and
   the commit *"must say that rename's tests are what surfaced it"*. That is the repo's stated
   convention for a fix whose story is "this PR's tests surfaced a pre-existing bug" (`CLAUDE.md`,
   *Bundling fixes into the PR that surfaced them*).
2. **The trait change has no standalone story.** `denial_for` with no second dialect is a defaulted
   method nothing calls differently. Shipping Task 1 alone would land shared-machinery churn whose
   only justification lives in a PR that has not happened yet.
3. **The two obligation tasks measure the whole.** Task 10 covers nine authorities *including*
   `ContextAdminAuthority`; Task 11 crosses gate, HTTP, CLI and MCP. Neither can land before the
   thing it measures, and splitting the PR would strand both.

**Commits, one per task**, per the repo's per-beat convention. Suggested subjects:

| Task | Subject |
|---|---|
| 1 | `refactor(authz): denial_for(&self) so one gate can answer in two dialects` |
| 2 | `feat(db): context_rename + _project_context_renamed with an in-transaction RBAC invariant` |
| 3 | `feat(substrate): context_renamed event, payload, write and replay arm` |
| 4 | `feat(core): RenameContextRequest / RenameContextOutcome wire types` |
| 5 | `feat(authz): ContextAdminAuthority — administers / system-admin / read-only / invisible` |
| 6a | `fix(contexts): canonicalize context names on create and rename` |
| 6b | `feat(contexts): context_service::rename, and fix reassign's 23505 race rendering as 500` |
| 7 | `feat(api): POST /api/contexts/{id}/rename` |
| 8 | `feat(cli): temper context rename` |
| 9 | `feat(mcp): rename_context, and render Conflict/BadRequest as actionable params` |
| 10 | `test(authz): standing regression boundary — nine authorities keep their refusal voice` |
| 11 | `test: four routes to read-without-administration collapse to one 403` |
| 12 | `docs: context rename, its two dialects, and its named remainders` |

Task 6's commit body must state that **rename's tests are what surfaced `reassign`'s `23505`
hole** — the spec requires it by name.

**Branch:** `jct/context-rename` (already checked out). Merge `origin/main` before pushing. Push and
open a PR; never merge locally.

**Before flipping out of draft, the controller runs** (no implementer runs any of these):

```
cargo make check
cargo make test
cargo make test-db
cargo make test-e2e
cargo sqlx prepare --workspace -- --all-features
cargo make prepare-services
cargo make prepare-e2e
cargo make openapi
cargo make generate-ts-types
```

Order matters twice: the `.sqlx` ritual is workspace-first and per-crate-last (G14), and the ts-rs
gate reads `git status --porcelain`, so its output must be **committed** before `cargo make check`
goes green.

---

# Part 5 — Spec ↔ task reconciliation

The spec's own reconciliation table maps clauses to spec sections. This maps them to tasks. Every
clause remains **declared-uncovered** until the controller files coverage against Tasks 10 and 11.

| Clause | Spec section | Task(s) | enables / witnesses |
|---|---|---|---|
| `rename-requires-administration` | The gate | 5, 6 · 11 | enables · witnesses |
| `refusal-discloses-no-more-than-the-caller-already-holds` | The `404` is the incumbent refusal | 1, 5, 9 · 11 | enables · witnesses |
| `a-rename-lands-where-it-was-asked-or-nowhere` | Refusals | 6 | enables |
| `one-owner-never-holds-two-of-the-same-address` | UNIQUE backstop + `23505` mapper | 2, 6 | enables |
| `every-completed-rename-is-attributable` | The write is event-sourced | 2, 3, 6 · 11 | enables · witnesses |
| `authority-is-decided-no-earlier-than-the-change` | in-transaction RBAC invariant | 2, 6 | enables |
| `replayed-history-is-not-re-adjudicated` | `_project_context_renamed` never authorizes | 2, 3 | enables |
| `system-authority-never-becomes-ownership` | What a rename does not touch | 5 · 11 | enables · witnesses |
| `no-other-refusal-changes-its-voice` | EXTEND: `denial_for(&self)` | 1 · **10** | enables · **witnesses** |
| `a-refusal-never-names-what-it-withholds` | EXTEND: `denial_for(&self)` | 1, 5 · **10** | enables · **witnesses** |
| `a-context-never-loses-its-contents-to-a-rename` | What a rename does not touch | 6 | enables |
| `a-stored-name-has-one-spelling` | Names are canonicalized … on both write paths | 6 (Step 0) | enables |
| `a-request-that-would-change-stored-state-is-never-declined-as-a-no-op` | Refusals — the no-op test | 6 (Steps 3, 5a–b) | enables |
| `a-reader-is-never-told-a-readable-context-is-absent` | The gate — `ReadOnly` | 5 · **11** | enables · **witnesses** |
| Closure class 1 (four routes) | Two obligations the build inherits | **11** | **witnesses** |
| Closure class 2 (personal vs team) | The gate | 5, 6 | enables |
| Closure class 3 (surface immaterial) | Surfaces | 7, 8, 9 · 11 | enables · witnesses |

---

# Part 6 — Spec corrections an implementer must not smooth over

Four, each with disk or live-DB evidence in Part 0. **Read these before Task 1.**

1. **G3 — the enclosing-team read direction is inverted.** The spec's *"A maintainer of a parent
   team can read a child team's context"* is false; reads inherit **down** the tree. Affects Task
   11's route 2, and building it the spec's way risks a real authorization widening shipped to make
   a test pass.
2. **G5 — there are nine `ScopedAuthority` impls, not six.** Affects Task 10's coverage.
3. **G11 — context writes are service-direct; the `Backend` trait has no context commands.** The
   spec's `DbBackend` routing sentence is unsupported by disk. This plan proceeds service-direct
   (CONFORM to `reassign`) and marks the point for escalation.
4. **Task 4 — `format_context_ref` does not exist.** The composer is `decorated_context_ref`
   (`temper-core/src/context_ref.rs:77-84`), and its `owner_addressable` parameter is **undecorated**
   while `ContextRow.owner_ref` is already decorated.

Two smaller line-number drifts, harmless but noted so a reader does not think they are looking at
the wrong file: the spec cites `authz/mod.rs:103` for `denial()` (it is `:104`) and
`audit_gate.rs:914` for `every_denial_renders_not_found` (the fn is at `:897`).
