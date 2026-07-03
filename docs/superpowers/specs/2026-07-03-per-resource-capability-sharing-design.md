# Per-resource capability sharing — design

**Task:** `019f25da-0e31` (goal `019f25d6` — Teams in Temper, scope #4, the last core
collaboration beat).
**Date:** 2026-07-03
**Branch:** `jct/resource-capability-sharing`

## Premise correction (grounded)

The task text (2026-07-03) framed this as *"share a resource to a team by writing a
capability grant row into `kb_resource_access`."* Grounding against HEAD shows the
premise is stale in one load-bearing way, which makes the task **smaller** than
"medium":

- `kb_resource_access` was **dropped** in the D5 store-swap migration
  (`migrations/20260701000003_access_grants_store_migration.sql:156-161`,
  `DROP TABLE kb_resource_access`). The live store is the subject-polymorphic
  **`kb_access_grants`** (`migrations/20260630000001_access_grants_seam.sql:24-41`) —
  the same table the cogmap grant endpoints already write.
- The service writers are **already subject-polymorphic**:
  `access_service::grant_capability` / `revoke_capability`
  (`crates/temper-services/src/services/access_service.rs:86-146`) accept any
  `subject_table ∈ {kb_resources, kb_contexts, kb_cogmaps}`. They already gate auth via
  `can_administer_grant` = `is_system_admin OR can('kb_profiles', caller, 'grant',
  subject_table, subject_id)`.
- The read consumers already honor resource grants:
  `resources_visible_to` (a `can_read` team/profile grant makes the resource visible)
  and `can_modify_resource` (a `can_write` grant enables modify), both re-emitted over
  `kb_access_grants` in `migrations/20260701000003_...:46-133`.

So the model exists end-to-end. **This task adds only (1) the resource HTTP/CLI surface
on top of the existing service, and (2) one auth-seam fix so a resource owner can
administer grants on their own resource.**

## The one real gap: owner cannot administer grants

Tracing `can()` (`migrations/20260630000001_access_grants_seam.sql:102-115`):

```
can('kb_profiles', owner, 'grant', 'kb_resources', res)
  = profile_explicit_grant(...)      -- true only if an explicit can_grant row exists → false for a bare owner
    OR derived_access_profile(...)   -- has NO 'grant' arm for resources (only read→visible, write→modify) → false
```

So a resource **owner** fails `can_administer_grant` and could **not** share their own
resource — this directly breaks the acceptance criterion *"owner grants a team read."*
(Cogmaps dodge this: they have no owner and bootstrap via system-admin / explicit
grant.)

**Fix (decided): encode owner ⇒ grant in the SQL `can()` seam**, not in Rust. This is
the principled home — symmetric with how `can_modify_resource` says *"the home confers
modify to its principals"* — and completes the capability seam for every future caller
of `can(...,'grant', 'kb_resources', ...)`. Scoped to **`owner_profile_id` only**, not
`originator_profile_id`: originator is provenance, not access (an originator who
transferred ownership away must not retain grant admin).

## Design decisions (approved)

1. **CLI shape:** mirror the existing cogmap `grant`/`revoke` vocabulary, not the task's
   `share`/`unshare --cap`. One mental model across resources + cogmaps; reuses the exact
   `GrantCapabilityRequest` path; supports profile *or* team principals with the full
   rwx+grant boolean set.
2. **Auth home:** the SQL `can()` seam (additive migration), per above.
3. **Grant seam scope:** `owner_profile_id` only.

## Components

### 1. Migration (additive; additive-only-on-main safe)

New `migrations/<ts>_resource_grant_owner_seam.sql`:
`CREATE OR REPLACE FUNCTION derived_access_profile(...)` reproducing the current body
(`20260630000001:75-91`, the only definition — never redefined since) **verbatim**, with
one added arm before the `ELSE`:

```sql
WHEN p_subject_table = 'kb_resources' AND p_action = 'grant' THEN
    EXISTS (SELECT 1 FROM kb_resource_homes h
            WHERE h.resource_id = p_subject_id
              AND h.owner_profile_id = p_profile)
```

Effect: `can(owner,'grant',res) = true`, so the **unchanged** shared
`can_administer_grant` now passes for owners. `CREATE OR REPLACE FUNCTION` is
non-destructive DDL. Regenerate the sqlx cache after (the query is inside a SQL function,
not a Rust `query!`, so the workspace cache is unaffected — but any new test-target
`query!` macros need per-crate prepare; see §7).

### 2. Types (temper-core)

`ResourceGrantBody { principal_table: String, principal_id: Uuid, can_read, can_write,
can_delete, can_grant: bool }` and `ResourceRevokeBody { principal_table, principal_id }`
— mirror `CogmapGrantBody` / `CogmapRevokeBody` (their home in temper-core, verified at
impl) with the same quad-derive (`serde` + `ts_rs::TS` / `utoipa::ToSchema` /
`schemars::JsonSchema` under the `typescript` / `web-api` / `mcp` features). Reuse the
existing polymorphic `GrantCapabilityRequest` / `RevokeCapabilityRequest` +
`GrantOutcome` / `RevokeOutcome` — **no new service-layer types**.

**Home:** co-locate the two resource bodies with the polymorphic request/outcome types
(or a small dedicated `access_grant` types module). Do **not** add them to
`crates/temper-core/src/types/access.rs` — that file holds the dead
`AccessLevel`/`TeamResource` and is retired by sibling task #6.

### 3. API handler + route

Add `grant` / `revoke` to the **existing** `crates/temper-api/src/handlers/resources.rs`
(no new module). Thin handlers mirroring `handlers::cognitive_maps::grant`/`revoke`:
widen the narrow HTTP body into `GrantCapabilityRequest` by injecting
`subject_table = "kb_resources"` and `subject_id = <path id>`, then delegate to
`access_service::grant_capability` / `revoke_capability` (auth lives in the service).
Route (`crates/temper-api/src/routes.rs`, mirroring the cogmap grants line):

```rust
.route(
    "/api/resources/{id}/grants",
    post(handlers::resources::grant).delete(handlers::resources::revoke),
)
```

Service-direct (not through `DbBackend`): grants are admin events, matching the cogmap
precedent (`access_service` header note, `access_service.rs:51-56`). Add utoipa
annotations mirroring the cogmap handlers.

### 4. Client (temper-client)

Add `grant(id, &ResourceGrantBody) -> GrantOutcome` and
`revoke(id, &ResourceRevokeBody) -> RevokeOutcome` to the **existing**
`crates/temper-client/src/resources.rs` (`ResourcesClient`), mirroring the
`cognitive_maps.rs` client methods (POST / DELETE-with-JSON-body to
`/api/resources/{id}/grants`).

### 5. CLI (temper-cli)

Add `Grant` / `Revoke` variants to `ResourceAction`
(`crates/temper-cli/src/cli.rs`), mirroring `CogmapCmd::Grant`/`Revoke`:

```
temper resource grant  <ref> [--to-profile <uuid> | --to-team <uuid|slug>] [--read] [--write] [--grant]
temper resource revoke <ref> [--from-profile <uuid> | --from-team <uuid|slug>]
```

Action fns mirror `actions/cogmap.rs::grant_api`/`revoke_api`; reuse the existing
`Principal` type + `resolve_team_id` helper (gives team-slug support). `can_read =
read || write || grant` (read forced on when a higher capability is set). `<ref>`
resolves locally via `temper_workflow::operations::parse_ref` (trailing-UUID). Exactly
one of `--to-profile`/`--to-team` (resp. `--from-*`) is required — enforce with clap
argument grouping as the cogmap variant does.

### 6. Tests (TDD — write first)

- **Service / SQL** (`temper-services` or e2e, whichever hosts the `can`/grant tests):
  after the seam migration, a resource **owner** passes `can_administer_grant` (i.e.
  `grant_capability` succeeds for the owner); a non-owner, non-admin profile **without**
  an explicit `can_grant` row gets `Forbidden`.
- **e2e** (`tests/e2e`, drives the production caller — the real `temper` CLI → API → DB):
  owner runs `temper resource grant <ref> --to-team <team> --read` → a member of that
  team now sees the resource via `resources_visible_to`; `--write` → `can_modify_resource`
  is true for that member; `temper resource revoke <ref> --from-team <team>` → visibility
  revoked. This *is* the acceptance criterion, end-to-end.

Follow `superpowers:test-driven-development`: red (assert the owner-grant path fails
before the seam fix / before the surface exists) → green → refactor.

### 7. sqlx cache

Regenerate after SQL / new `query!` macros:
`cargo sqlx prepare --workspace -- --all-features`, then `cargo make prepare-e2e`
(and `cargo make prepare-api` if a handler-target `query!` lands). The seam change is a
SQL-function body, so the workspace lib cache is unaffected by it directly; the driver is
any new Rust `query!` in tests.

## Out of scope

- Retiring dead `AccessLevel` / `TeamResource` in `types/access.rs` and the old
  `access_level` enum — sibling task #6 (`019f25da-3e89`).
- MCP surface for resource grants. Not in the acceptance criteria; the cogmap grant MCP
  tool is the template if we add it later. (Full-surface parity is the eventual intent —
  flagged here as a deliberate deferral, not an omission.)

## Acceptance

- Owner grants a team `read` on one resource → a team member sees it via
  `resources_visible_to`; `revoke` removes it; a `write` grant enables
  `can_modify_resource`. Verified through the real CLI in the e2e suite.
- `cargo make check` clean; targeted crate suites + the new e2e green.
