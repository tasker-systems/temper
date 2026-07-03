# Team Invitations Wiring + System-Gate CLI Reconciliation

**Date:** 2026-07-03
**Task:** `019f25d9-d280-7221-a58a-38f55b5e84a5` (Team invitations wiring)
**Goal:** `019f25d6` — Teams in Temper: usable multi-user collaboration surface (scope #2)
**Mode/effort:** plan / medium

## Problem

The invitation substrate exists but is **inert** — table, enum, and core types are
defined with zero service/handler/route/CLI wiring:

- Table `kb_team_invitations` — `migrations/20260624000001_canonical_schema.sql:377-390`;
  enum `invitation_status` at `:111`.
- Types `TeamInvitation` / `InvitationStatus` — `crates/temper-core/src/types/invitation.rs`;
  only referenced by the re-export at `types/mod.rs:67`.

Before building the flow, the task requires deciding how invitations relate to the
existing **join-request** model, which appears to overlap. This spec captures that
decision, then specifies the invite → accept/decline flow and the CLI reconciliation.

## The model decision (join-requests vs invitations)

These are **orthogonal, coexisting mechanisms — neither subsumes the other.** They
only *looked* overlapping because one CLI verb (`team join`) is mis-scoped.

**Join-requests (`kb_join_requests`) = the system access gate.** Pull-based,
user→system, always the single `gating_team_slug`. Grounding:
- `access_service::create_join_request` hard-errors unless `access_mode = invite_only`
  and always resolves the team from `settings.gating_team_slug`, ignoring any team
  argument (`crates/temper-services/src/services/access_service.rs:388-450`).
- On approval it inserts membership of the **gating** team as `watcher`
  (`access_service.rs:601-611`), which is exactly what `has_system_access` checks —
  *direct* membership of the gating team, no DAG walk
  (`migrations/20260624000002_canonical_functions.sql:1397-1403`).

**Invitations (`kb_team_invitations`) = general team membership.** Push-based,
team→email→member, **any** team, token-redeemed.

**Boundary:** they coexist. Inviting someone **to the gating team** naturally doubles
as the push side of `invite_only` (gating-team membership *is* system access).
Inviting to a **non-gating** team while the system is `invite_only` grants team
membership but **not** system access — an accepted, documented gap, and moot in
production which runs `access_mode = open` (where `has_system_access` is
unconditionally true, `functions.sql:1396`).

### Consequence: the CLI was mis-filed

System access is an **authentication/entitlement** concern ("am I let into the system
at all?"), not a **collaboration** concern ("which team am I working with?"). The
gating *team* is merely the implementation substrate. So the system-gate verbs move
out of `temper team` and under `temper auth` (next to `login`/`logout`/`token`),
freeing `team join` for its correct meaning: accept a team invitation.

Today all three system-gate verbs live under `team` and `team join`'s `--team` arg is
silently ignored in dispatch (`crates/temper-cli/src/main.rs:340-344`,
`crates/temper-cli/src/commands/team.rs:25-52`).

## Design

### 1. Data layer — one additive migration

New migration `migrations/20260703NNNNNN_invitation_partial_unique.sql`:
- Drop the full `UNIQUE(team_id, invited_email)` constraint (inline in the CREATE
  TABLE, so Postgres auto-named it `kb_team_invitations_team_id_invited_email_key` —
  **the implementer confirms the exact name via `\d kb_team_invitations` before
  writing the DROP**).
- Add a **partial** unique index `WHERE status = 'pending'`, mirroring
  `idx_join_requests_one_pending` (`canonical_schema.sql` join-requests block).

Safe under the additive-only-on-`main` posture: the table is inert (zero rows in any
environment), and relaxing a uniqueness constraint cannot break existing data. History
rows (declined/expired/accepted) coexist; only a second **pending** invite for a
`(team, email)` pair conflicts. This makes invite an ordinary INSERT — no upsert
machinery — exactly like `create_join_request`.

### 2. Service layer — new `temper-services/src/services/invitation_service.rs`

Service-direct (no `Backend` trait, no event emission — invitations are
provisioning/infra, same precedent as `team_service` and `context_service`;
`team_service.rs:1-14`). Authorization precedes every write, reusing the existing
`role_on_team` + `can_manage` helpers (`team_service.rs:46,66`).

**Token generation:** 16 bytes from a CSPRNG, hex-encoded (32 chars, fits
`VARCHAR(128)`). **Must not** be `Uuid::now_v7()` — that is time-sortable and
guessable, unacceptable for a capability token. The implementer confirms a CSPRNG
dependency (`rand` / `getrandom`) is available in `temper-services` (or adds one).

Functions:

- `create_invitation(pool, caller: ProfileId, team_id, params: CreateInvitationParams)`
  — params struct `{ invited_email: String, role: TeamRole }` following the
  `CreateJoinRequestParams` / `ReviewRequestParams` convention (implementer matches
  their module home). Auth: `role_on_team` must be `Some(role)` with `can_manage(role)`
  (owner/maintainer), else `Forbidden`. Reject `role == Owner` with `BadRequest`
  ("ownership is transferred, not invited"). INSERT a `pending` row; a partial-unique
  violation → `Conflict` ("a pending invitation already exists for this email"). No
  membership-by-email precheck (an email need not map to an existing profile).
  Returns `TeamInvitation`.

- `list_invitations(pool, caller, team_id)` — auth `can_manage`; returns pending,
  non-expired invitations (`WHERE status='pending' AND expires_at > now()`).

- `accept_invitation(pool, caller: ProfileId, token)` — **bearer authority**: the
  128-bit token is the authority; membership is created for the *caller's* profile.
  Lookup by token → `NotFound`. Then:
  - `expired`, or `pending` with `expires_at < now()` → lazily `UPDATE status='expired'`
    and return `BadRequest`/`Gone` (no sweep job; expiry is checked at accept time).
  - `declined` → error (invitation was declined).
  - `accepted` → if caller is already the member, idempotent success; else `Conflict`
    ("invitation already redeemed").
  - `pending` (unexpired) → transaction mirroring `review_request`
    (`access_service.rs:575-628`): `INSERT INTO kb_team_members (team_id, profile_id,
    role) ... ON CONFLICT (team_id, profile_id) DO NOTHING`, then `UPDATE` the
    invitation to `accepted`. Returns `AcceptInvitationResponse`.
  Acceptance is idempotent by construction (`ON CONFLICT DO NOTHING` + the
  already-accepted branch).

- `decline_invitation(pool, caller, token)` — bearer; `pending` → `UPDATE
  status='declined'`; idempotent if already declined; `accepted` → `BadRequest`.

### 3. HTTP transport — `temper-api`

New `handlers/invitations.rs`; `ProfileId` extracted from the auth extension as in
existing handlers. Routes (`routes.rs`) split across the two existing tiers — this
split is load-bearing:

- `POST /api/teams/{id}/invite`, `GET /api/teams/{id}/invitations` →
  **system-access-gated group** (`routes.rs:44` onward). The inviter is an
  authenticated team admin who necessarily already has system access.
- `POST /api/invitations/{token}/accept`, `POST /api/invitations/{token}/decline` →
  **authenticated-but-NOT-gated group** (`routes.rs:20-40`, alongside `/api/access/*`).
  An invitee to the gating team must be able to accept **before** they have system
  access — otherwise the gate would make its own invitations un-redeemable.

### 4. Wire types — `temper-core` (ts-rs derives for the UI)

- `CreateInvitationRequest { invited_email: String, role: TeamRole }` — POST body.
- `AcceptInvitationResponse { team_id: Uuid, team_slug: String, role: TeamRole }` — so
  the CLI can print "You joined <team> as <role>."
- `TeamInvitation` already exists (`types/invitation.rs`); reused for list + create
  responses.

Typed structs throughout — no `serde_json::json!()` for structured payloads.

### 5. Client — `temper-client`

Add to the `teams()` surface: `invite(team_id, &CreateInvitationRequest)`,
`list_invitations(team_id)`, `accept_invitation(token)`, `decline_invitation(token)`.
The `access()` surface is unchanged — its `create_request` / `get_own_request` /
`withdraw_request` methods stay; only their CLI callers move.

### 6. CLI — `temper-cli`

**`auth` (entitlement — the system gate):**
- `auth request-access [--message]` — was `team join`; calls `access().create_request`.
- `auth withdraw-request` — was `team withdraw-request`.
- **Fold system-access state into the existing `auth status`** (`cli.rs:558`): it grows
  a `System access: granted | pending since <date> | none` line, sourced from
  `get_entitlements` (the `system_access` bool) + `get_own_request` (pending detail).
  Must render both `open` (→ "granted") and `invite_only` (→ request lifecycle)
  gracefully. Retires `team status` rather than adding a duplicate `auth access-status`.

**`team` (collaboration — any team):**
- `Join { token }` — repurposed from the old system-gate variant to a **positional
  `<token>`**; calls `accept_invitation`.
- `invite <email> --role <role>` — calls `invite`.
- `decline <token>` — calls `decline_invitation`.
- `invitations <team>` — lists pending (owner/maintainer).
- Unchanged: create, add-member, list, show, leave, remove-member, set-role, update,
  delete.

Dispatch rewired in `main.rs`; the ignored `--team` on `Join`/`Status` disappears.

### 7. Tests

- **Service** `#[sqlx::test]` in `invitation_service.rs` (file needs
  `#![cfg(feature = "test-db")]`): create (auth ok / `Forbidden` / owner-role rejected /
  pending-conflict / fresh invite succeeds after a prior decline); accept (happy →
  member row created; idempotent re-accept; expired → error + row flips to `expired`;
  declined → error; unknown token → `NotFound`); decline (happy + idempotent); list
  (auth-gated, pending-only).
- **e2e** (`tests/e2e/tests/`): full `temper team invite` → `temper team join <token>`
  → assert `kb_team_members` row, driving CLI → client → API → DB. Plus one
  `temper auth request-access` flow against an `invite_only` settings fixture to prove
  the reframed verb still routes to the gate.
- **sqlx cache:** workspace prepare for lib queries, then `cargo make prepare-services`
  / `prepare-api` / `prepare-e2e` for whichever test-target queries are added.

## Risks / open items

- **CSPRNG dependency** must be present in `temper-services` for token minting; if
  absent, adding `rand` is part of the work (flag, don't silently reach for uuid).
- **Decline is pure-bearer** — anyone holding the token can decline it (a mild
  griefing vector if a link leaks). Bounded by 128-bit token secrecy; accepted.
- **Non-gating-team invite in `invite_only`** confers team membership but not system
  access. Documented, acceptable, moot in prod (`open`).
- **`auth status`** must handle the `open` path (no join-request exists) without
  erroring — show "granted (open access)".

## Out of scope

- Ownership transfer (goal scope #3, `kb_transfers`).
- Per-resource capability sharing (scope #4).
- Email delivery of invitation links (tokens are returned/printed; delivery is a
  separate concern).
- UI surfaces (data + API + CLI first, per project sequencing).
