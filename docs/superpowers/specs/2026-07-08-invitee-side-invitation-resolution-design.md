# Invitee-Side Team Invitation Resolution (CLI + MCP)

**Date:** 2026-07-08
**Task:** `019f41f3-74ab-7ec0-8b0d-cb21662c51cb` (Invitee-side team invitation resolution)
**Goal:** `019f25d6` — Teams in Temper: usable multi-user collaboration surface (follow-on scope)
**Mode/effort:** build / medium

## Problem

Team invitations shipped (scope #2, PR #251) but the flow is only half a loop. Today
an invitation is discoverable in exactly two ways:

- **Inviter-side:** `temper team invitations --team <id>` — owner/maintainer view of a
  team's pending invites (`invitation_service::list_invitations`).
- **Token-bearer:** whoever presents the token accepts
  (`invitation_service::accept_invitation` — token is the sole authority).

There is **no "what teams have invited *me*?" surface**, and **no email is ever sent**
— the repo has no SMTP/mailer of any kind. So an invite created for
`person@x.com` only becomes redeemable if the inviter copies the token out of the CLI
output (`TeamInvitation.token` is rendered by `commands/team.rs:42`) and hands it over
out-of-band. The type's own doc-comment still describes an aspirational
"token-bearing URL, recipient clicks, email" flow that was **never built**
(`crates/temper-core/src/types/invitation.rs:25-28`).

## The reframe

OAuth/SAML self-serve join already solves "get into the system" — a person signs in
and a profile is auto-provisioned (`profile_service.rs:215`). So **email is a
correlator, not a delivery channel**: you invite `person@x.com` *before* they exist,
and once auth provisioning creates their profile, the invitation should **resolve to
that profile** so they can pull and accept their own invites. The listing *is* the
delivery — you fetch your own invite rather than having a token pushed to you.

This round builds that invitee-side resolution over CLI + MCP (+ the API read behind
them) and rounds out the MCP surface with accept/decline. The accept mechanism itself
is unchanged (token-bearer); the token is now self-served from your own listing.

## Resolving an invitation to a profile — safely

An invitation is keyed by `invited_email` (a string). To resolve it to a profile we
match that email against the caller's identities in `kb_profile_auth_links`.

### What the schema guarantees, and doesn't

`kb_profile_auth_links` (`migrations/20260624000001_canonical_schema.sql:331-342`):
- `UNIQUE(auth_provider, auth_provider_user_id)` — uniqueness is on the *identity*.
- `email VARCHAR(256)` is **nullable and NOT unique** (plain index `idx_auth_links_email`).
- There is **no `email_verified` column** — verification is known only on `claims` at
  sign-in time, never persisted to the row.

Provisioning is already a find-or-create with **verification-gated** email
reconciliation (`profile_service.rs:126-255`):
1. Exact `(provider, external_user_id)` match → existing profile.
2. `reconcile_by_email` — **only if `claims.email_verified == Some(true)`**
   (`:154`) — links a new identity into the *existing* profile holding that email.
3. Otherwise a brand-new profile is created carrying the email.

**Consequence:** a *verified* email always collapses to the first profile that holds
it, so verified emails resolve to exactly one profile. The residual gap is narrow:
because verification isn't persisted, stored data can't distinguish a verified from an
unverified email, and an **unverified** sign-in can still mint a second profile
carrying the same email — so an email *can* map to more than one profile. A naive
`invited_email → auth_links.email` match would then surface one person's invite to
another profile.

### Chosen approach — Option B: query-time uniqueness guard (no schema change)

The resolver only surfaces an invitation whose `invited_email` maps to **exactly one
profile system-wide**. An email that is ambiguous across profiles is **discounted** —
it never resolves, falling back to the existing token hand-off for that person.

- **Safe unconditionally:** ambiguity never leaks; it degrades to "no auto-resolution,"
  never to "resolves to the wrong profile."
- **Unambiguous and immediate for essentially everyone:** any email held by exactly one
  profile (the common case, and guaranteed for verified-first sign-ins) resolves
  straight away.
- **Token-fallback only** for the rare fragmented, all-unverified duplicate-account
  case — which never resolves *wrong*, just doesn't auto-surface.

The robust end-state (persist `email_verified`, match verified-only, plus an
account-merge story) is **deferred to a tracked follow-up** — it is not needed for
this round to be correct or safe.

### Matching rule

Match **case-insensitively** (`lower(invited_email) = lower(auth_link.email)`) — an
invite to `Person@X.com` must resolve against a stored `person@x.com`. The guard
counts distinct profiles for the lower-cased email.

## Design

### Layering

`list_for_profile` is a **read** → service-direct on both surfaces (reads stay
service-direct by design; no `DbBackend`, no operations command). It sits next to the
existing `invitation_service` methods, all of which are already service-direct.

### 1. Service — `crates/temper-services/src/services/invitation_service.rs`

New: `list_for_profile(pool, caller: ProfileId) -> ApiResult<Vec<InviteeInvitation>>`.

```sql
SELECT i.id, i.team_id, i.invited_email, i.invited_by_profile_id,
       i.role, i.token, i.status, i.expires_at, i.created,
       t.slug AS team_slug, t.name AS team_name
FROM kb_team_invitations i
JOIN kb_teams t ON t.id = i.team_id
WHERE i.status = 'pending'
  AND i.expires_at > now()
  AND t.is_active                                  -- dead teams' invites are moot
  AND lower(i.invited_email) IN (
        SELECT lower(al.email)
        FROM kb_profile_auth_links al
        WHERE al.profile_id = $1
          AND al.email IS NOT NULL
          AND (SELECT COUNT(DISTINCT al2.profile_id)
               FROM kb_profile_auth_links al2
               WHERE lower(al2.email) = lower(al.email)) = 1   -- Option B guard
      )
ORDER BY i.created DESC
```

Returns an empty vec for a caller with no email (e.g. an agent profile whose auth-link
email is `NULL`) — no error. Token is included in the row because the caller is
authorized to redeem their own invites; the listing is the delivery.

### 2. Core type — `crates/temper-core/src/types/invitation.rs`

New `InviteeInvitation` struct (typed, not inline JSON) — the `TeamInvitation` fields
plus `team_slug` and `team_name` for display. Derives `FromRow`, `Serialize`,
`Deserialize`, and the gated `ts_rs::TS` / `utoipa::ToSchema` / `schemars::JsonSchema`
(MCP) to match the other invitation types. Regenerate ts-rs bindings.

### 3. API — `crates/temper-api/src/handlers/invitations.rs` + `routes.rs`

- `GET /api/invitations` → `list_mine` handler, calling `invitation_service::list_for_profile`.
- Registered on the **un-gated** router block (alongside accept/decline,
  `routes.rs:40-45`), not the gated one: a person invited to the gating team must be
  able to *discover* that invite before they hold system access, exactly as they can
  already *accept* it un-gated. Requires auth, not system access.
- Add to OpenAPI (`openapi.rs`).

### 4. Client — `crates/temper-client/src/`

`client.teams().list_my_invitations() -> Result<Vec<InviteeInvitation>>`, next to the
existing `invite`/`accept_invitation`/`decline_invitation`/`list_invitations`.

### 5. CLI — new top-level `temper invitations`

A top-level command reads better than a team-scoped flag, because "my invites" span
teams and aren't addressed by a team id.

- Clap: new `Commands::Invitations` (no args) in `cli.rs`; dispatch in `main.rs` →
  `commands::invitations::list_mine` in a new `commands/invitations.rs` module (a
  top-level command earns its own module rather than crowding `team.rs`).
- Output routes through `crate::format::render` like the other invitation commands.
- The generated `reference.md` picks this up automatically from clap — no manual edit.

### 6. MCP — first invitation tools — `crates/temper-mcp/src/tools/`

Round out the loop so an agent can both see and act:
- `list_my_invitations` — no params; delegates to `invitation_service::list_for_profile`
  (services-direct read, mirroring `get_profile`).
- `accept_invitation { token }` and `decline_invitation { token }` — services-direct
  calls to the existing `invitation_service` methods.

Register in `tools/mod.rs` and the dispatch in `service.rs`. These are the first team/
invitation tools in MCP; follow the `get_profile` tool pattern for the reader and the
existing service-direct call pattern for the actions.

### 7. Docs

**`docs/guides/teams.md`** (new, human-facing) — sits with the other guides. Covers:
what a team is; roles (owner/maintainer/member/watcher); create + add existing members
by UUID; the invite → **self-serve resolve** → join loop told truthfully (email is a
correlator, no email is sent, ambiguous-email fallback is the token hand-off);
team-owned contexts; offboarding via `team reassign`; soft-delete.

**`crates/temper-cli/skill-content/teams.md`** (new, agent-facing, flows into the
generated skill). Must be **self-contained** (skill consumers have the skill files, not
the repo). Wiring mirrors `cognitive-maps.md` exactly:
- `include_str!` in `commands/skill.rs` (near `COGNITIVE_MAPS_MD`, `skill.rs:18`).
- Insert into the `files` map in `generate_skill_files_with_hash` (`skill.rs:510-513`).
- Add to `check_expected_files` (`skill.rs:390`).
- Add a router line to the "Supporting Files" list in `templates/skill.md:22-27`.

> **Build gotcha (must be verified, not assumed):** skill content is compiled into the
> `temper` binary via `include_str!` + compile-time Askama templates. Editing
> `skill-content/teams.md` has **no effect** until `cargo install --path
> crates/temper-cli` **then** `temper skill install`, and `temper skill check` will
> **not** flag the staleness (config-hash tracks config, not source content). The
> verification step is: rebuild → reinstall → eyeball the installed
> `~/.claude/skills/temper/teams.md`.

## Error handling

- Empty / no-match → empty list, HTTP 200. Never an error.
- Caller with no email → empty list (guard naturally excludes `NULL`).
- Ambiguous email → silently discounted (safe fallback), never surfaced.
- Auth required on all new surfaces; `GET /api/invitations` is un-gated on system
  access so pre-access invitees can discover gating-team invites.

## Testing

- **Service (`#[sqlx::test]`, `test-db`):** seed two profiles + teams + invitations.
  Assert: caller sees only pending, non-expired invites to their unambiguous email;
  an email held by two profiles is discounted for both; declined/expired excluded;
  soft-deleted team excluded; case-insensitive match works; NULL-email caller → empty.
- **e2e (production-caller level):** drive `temper invitations` end-to-end through the
  real CLI → API → DB (invite from profile A; provision + list as invitee B). Pair the
  direct-service test with this so the CLI/API wiring is actually exercised.
  *Gotcha:* no concurrent `test-e2e` runs.
- **sqlx cache:** the new `query!` is a service **lib** query → `cargo sqlx prepare
  --workspace -- --all-features` (+ `cargo make prepare-services`). If the e2e/test
  targets grow a macro query, `cargo make prepare-e2e`.

## Scope boundary — deferred to follow-up task

A separate task (filed against the same goal) captures the robust identity end-state:
**persist `email_verified` on `kb_profile_auth_links`** (populate from
`claims.email_verified` at provisioning; additive, main-safe migration; backfill
decision), match verified-only, **plus an account-merge story** to collapse
pre-existing unverified duplicates. Not built here; Option B stands alone, safely.

## Non-goals

- No email/SMTP delivery, no invitation URLs (self-serve pull replaces push).
- No `@handle`/profile-directory discovery — email correlation is sufficient; a
  people-directory is a distinct, larger feature.
- No change to how invitations are *created* or *accepted* (still email-keyed,
  token-bearer).
