# Team Read + Member Lifecycle — Design

**Date:** 2026-07-02
**Status:** Design — approved, pending spec review
**Goal:** `teams-in-temper` (Teams in Temper: usable multi-user collaboration surface)
**Task:** Team read + member lifecycle (`019f25d9-c112-7042-bf0c-62a0f6a1d981`)
**Supersedes (in part):** the retired **I7: Team Management & Resource Sharing**

## Context

Teams today can be created, listed, and have members added directly, but there is no
way to **view** a team and its members, **remove** a member, or **change a role**
through a proper endpoint. This slice fills those gaps against the model the system
actually grew into (team RBAC roles + context-share + capability grants), not the
retired `kb_team_resources`/`access_level` model the I7 task assumed.

### What exists today (grounding, file:line)

- Routes: only `GET/POST /api/teams` + `POST /api/teams/{id}/members`
  (`crates/temper-api/src/routes.rs:93-97`), all under the auth+system-access `gated`
  router.
- Service `team_service` is **service-direct** — no Backend-trait command, no event
  emission (teams are provisioning/infra, org-provisioning spec §2.6). Handlers are
  thin: extract `AuthUser` → one service call → typed row
  (`crates/temper-api/src/handlers/teams.rs`).
- Reusable authz helpers already exist, `pub(crate)`:
  `team_service::role_on_team(pool, team_id, profile_id) -> Option<TeamRole>` and
  `team_service::can_manage(role) -> bool` (owner|maintainer)
  (`crates/temper-services/src/services/team_service.rs:45-67`).
- `add_member` upserts role via `ON CONFLICT (team_id, profile_id) DO UPDATE SET role`
  (`team_service.rs:181-193`) — currently the only path to a role change.
- `kb_team_members` has a `source` column (`team_member_source` enum `native|idp`),
  added by `migrations/20260702000001_saml_group_provisioning.sql`. SAML reconcile
  owns `idp` rows and never touches `native` rows (native-wins-skip)
  (`crates/temper-services/src/services/saml_provisioning_service.rs`).
- `kb_teams` is `(id, slug, name, created, auto_join_role)` — **no `description`,
  no `is_active`** (`migrations/20260624000001_canonical_schema.sql:182-187`). Team
  metadata/soft-delete is a **separate task** (T5); out of scope here.
- CLI `team leave` currently calls `access().withdraw_request()` — it withdraws a
  pending **join-request**, NOT a membership deletion
  (`crates/temper-cli/src/commands/team.rs:99-133`). CLI slug→id resolution helper:
  `resolve_team_id` (`crates/temper-cli/src/actions/cogmap.rs:91-113`).

**No migration is required for this slice** — `source`, roles, and ownership all live
on existing columns.

## Decisions (resolved forks)

1. **`team leave` overload → split verbs.** `team leave <slug>` becomes self-leave
   (membership delete). The current join-request-withdrawal behavior is renamed to
   `team withdraw-request` (function `leave`→`withdraw_request`, body unchanged). One
   side-effect per verb.
2. **Last-owner guard → block with error.** Any operation that would drop a team to
   zero owners (removing the last owner, self-leave as last owner, or demoting the
   last owner) is refused with a clear message. Never orphan a team.
3. **idp-sourced rows → refuse.** The user-facing DELETE and PATCH member endpoints
   refuse rows with `source='idp'`: those are provisioned by SAML and owned by
   reconcile. Error steers the caller to the identity provider / group mapping.
4. **Role change → dedicated `PATCH`.** A new `PATCH /api/teams/:id/members/:pid`
   replaces the "re-POST to /members" upsert hack for role changes. `add_member`
   stays creation-only in intent. PATCH cannot create a member (404 if absent) and
   cannot set role `owner` (ownership is transferred, not granted — future T3).

## Architecture

Mirror the existing team pattern precisely. Three new endpoints, each a thin handler
dispatching one new `team_service` function that returns a typed row. Authz reuses
`role_on_team` + `can_manage`; auth checks precede every write.

### Endpoints

| Method | Path | Service fn | Success | Auth | Guards |
|---|---|---|---|---|---|
| `GET` | `/api/teams/{id}` | `team_detail` | 200 `TeamDetail` | member (any role) OR `is_system_admin`; else **404** | — |
| `DELETE` | `/api/teams/{id}/members/{profile_id}` | `remove_member` | 204 | owner/maintainer OR caller==target (self-leave) | `source='idp'`→409; last-owner→409 |
| `PATCH` | `/api/teams/{id}/members/{profile_id}` | `change_role` | 200 `TeamMemberRow` | owner/maintainer | member must exist→404; new role `owner`→**400**; `source='idp'`→409; demote-last-owner→409 |

**Visibility of team detail** returns **404** (not 403) when the caller is neither a
member nor a system admin — avoids leaking team existence to non-members (team slugs
are globally unique and used in share flows, so enumeration matters).

**Path identity** is the team UUID (`{id}`), matching `add_member`. The CLI resolves
slug→UUID client-side before calling. `profile_id` is a UUID.

### Service functions (`team_service.rs`)

- `team_detail(pool, caller, team_id) -> ApiResult<TeamDetail>`
  - Auth: `role_on_team(caller).is_some() || is_system_admin(caller)` else `NotFound`.
  - One query for the team row; one for members joining `kb_profiles` for `handle` and
    selecting `role` + `source`.
- `remove_member(pool, caller, team_id, target) -> ApiResult<()>`
  - Auth: `can_manage(caller_role)` OR `caller == target`.
  - Load target row (role + source); `NotFound` if absent.
  - Guard: `source == idp` → `Conflict`.
  - Guard: target role is `owner` AND `count_owners(team_id) == 1` → `Conflict`
    (last-owner).
  - `DELETE FROM kb_team_members WHERE team_id AND profile_id`.
- `change_role(pool, caller, team_id, target, new_role) -> ApiResult<TeamMemberRow>`
  - Auth: `can_manage(caller_role)`.
  - `new_role == owner` → `Conflict` (transfer, not grant).
  - Load target row; `NotFound` if absent (no upsert-create).
  - Guard: `source == idp` → `Conflict`.
  - Guard: current role `owner` AND `new_role != owner` AND `count_owners == 1` →
    `Conflict` (demote-last-owner).
  - `UPDATE kb_team_members SET role WHERE … RETURNING …`.
- Helpers: `count_owners(pool, team_id) -> i64` and the target-row load (role+source)
  factored so both mutators share it.

### Types (`temper-core::types::team`, `ts-rs` + `utoipa` derives to match siblings)

- `TeamDetail { id: Uuid, slug: String, name: String, created, auto_join_role:
  Option<TeamRole>, members: Vec<TeamMemberDetail> }`
- `TeamMemberDetail { profile_id: Uuid, handle: String, role: TeamRole, source:
  TeamMemberSource }`
- `ChangeRoleRequest { role: TeamRole }`
- `TeamMemberSource` enum (`native|idp`), `#[sqlx(type_name = "team_member_source",
  rename_all = "lowercase")]`, with `ts-rs`/`serde` derives. **Confirmed it does not
  exist yet** — the SAML provisioning service reads the column as `source::text AS
  "source: String"` and compares string literals (`saml_provisioning_service.rs:99,
  109`). Add the typed enum in `temper-core::types::team` for the new wire type; do
  **not** refactor the SAML service to use it (unrelated — leave the string reads).

### Client (`temper-client` teams.rs)

- `get(&self, team_id: Uuid) -> Result<TeamDetail>` → `GET /api/teams/{id}`
- `remove_member(&self, team_id: Uuid, profile_id: Uuid) -> Result<()>` → `DELETE …`
- `change_role(&self, team_id, profile_id, &ChangeRoleRequest) -> Result<TeamMemberRow>`
  → `PATCH …`

### CLI (`cli.rs` `TeamAction` + `commands/team.rs`)

- **Rename** `TeamAction::Leave` → `WithdrawRequest` (command `withdraw-request`); the
  handler fn `leave` → `withdraw_request`, body unchanged.
- `team show <slug>` — `resolve_team_id(client, slug)` → `client.teams().get(id)` →
  render `TeamDetail`. Distinct from `team status` (caller's own join-request state).
  Note `resolve_team_id` (`crates/temper-cli/src/actions/cogmap.rs:91-113`, reused
  as-is) only matches teams the **caller is a member of**; a system admin viewing a
  team they are not in passes the UUID directly (which `resolve_team_id` accepts).
- `team leave <slug>` — resolve team id; get caller's own profile id via
  `client.profile().get().await?.id` (`temper-client` `ProfileClient::get` →
  `/api/profile`, `Profile.id`), then `remove_member(team_id, own_id)` (hits the
  self-leave path).
- `team remove-member <team> <profile>` — resolve team id; `profile` is a UUID
  (matching `add-member`); `remove_member`.
- `team set-role <team> <profile> --role <r>` — resolve team id; `change_role`.
  `parse_role` already exists (`commands/team.rs:10-20`).

Profile-by-handle resolution for `remove-member`/`set-role` is **YAGNI** for this
slice (UUIDs match the existing `add-member` shape); revisit if it bites.

### Error mapping

- `NotFound` → 404 (non-visible team; PATCH target not a member).
- `Forbidden` → 403 (non-owner/maintainer mutating others; non-self on protected op).
- `Conflict` → 409 with a specific message for the state guards: `"cannot remove the
  last owner; transfer ownership or promote another member first"` (also for
  demote-last-owner) and `"this membership is provisioned by SAML; change it via the
  identity provider"`.
- `BadRequest` → 400 for the request-validity guard: `"cannot grant owner via role
  change; use ownership transfer"` (available `ApiError` variants: `NotFound`,
  `Unauthorized`, `Forbidden`, `SystemAccessRequired`, `BadRequest`, `Conflict`,
  `Internal` — `crates/temper-services/src/error.rs`).

## Testing

- **Service unit tests** (`#[sqlx::test]`, temper-services): the full auth+guard
  matrix per function — owner/maintainer/member/watcher/self/non-member callers;
  idp-row refusal; last-owner block (remove + demote); PATCH-on-nonmember 404;
  role=owner rejection; happy paths for detail/remove/change.
- **E2E** (`tests/e2e`, `cargo make test-e2e`): the membership-semantics matrix
  end-to-end through CLI ↔ API ↔ DB. Project convention: test-db-only green is a
  **false signal** for access/membership semantics — the e2e harness mints admins via
  direct `kb_team_members` owner-writes that test-db never exercises. Cover: owner
  views + removes + re-roles; member self-leaves; member cannot remove others;
  last-owner self-leave blocked; idp row not removable.
- **sqlx caches:** new macro queries in temper-services (and any test-target queries)
  require regeneration: `cargo sqlx prepare --workspace -- --all-features`, then
  `cargo make prepare-services` and `cargo make prepare-api` (per-crate, last).

## Out of scope (sibling tasks under `teams-in-temper`)

- Team `PATCH`/`DELETE` metadata + soft-delete and the `description`/`is_active`
  columns → **T5 (Team metadata + soft-delete)**.
- Invitations (invite/accept/decline) → **T2**.
- Ownership transfer (the `owner` role-grant path) → **T3**.
- Per-resource capability sharing → **T4**.
- Retiring dead `AccessLevel`/`TeamResource` remnants → **T6**.
