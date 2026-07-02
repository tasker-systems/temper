# SAML Phase 2 — role + team provisioning (JIT reconcile-on-login) — design spec

**Date:** 2026-07-01 (revised 2026-07-02 — decisions 9–11 added after design review)
**Status:** Design (build/medium), reviewed + approved by Cole. Grounded against `main` after PR #231 (SAML Phase 1 shipped + deployed).
**Issue:** #224 — "SAML SP with profile, role, and team provisioning." Phase 1 (authn-only) shipped; this spec is **Phase 2**.
**Predecessor spec:** [2026-07-01-saml-sp-temper-authorization-server-design.md](2026-07-01-saml-sp-temper-authorization-server-design.md) (Phase 0+1).
**Branch:** `jct/saml-phase2-role-team-provisioning`.

> **Grounding note.** Every "as-built" claim below cites a verbatim `file:line` against `main` at this checkout
> and was confirmed by reading the migration/handler body, not inferred from a prior spec.

---

## 1. Problem

Phase 1 made Temper a native SAML Service Provider for **authentication only**: the temper-cloud OAuth
Authorization Server (AS) validates a SAML assertion, maps it to `{sub, email, email_verified}`, mints a
short-lived EdDSA JWT, and temper-api does profile JIT via `resolve_from_claims`. **No role or team membership
is assigned at login** — teams and roles come entirely from Temper-internal mechanisms.

Phase 2 adds **role + team provisioning driven by SAML-asserted groups**, reconciled on each login: an operator
maps IdP groups to `(team, role)` pairs, and at login Temper reconciles the user's IdP-driven memberships to
match the assertion — adding, updating, and revoking **only IdP-managed** memberships, never touching
Temper-native ones.

The honest limit, stated up front and unchanged from #224: **reconcile-on-login is eventual, not immediate.**
A user removed from a group in the IdP retains Temper access until their session expires and they next attempt a
fresh SAML login. Immediate deprovisioning is **SCIM (Phase 3)** and is explicitly out of scope here.

---

## 2. Grounded current state (Phase 1 as-built)

### 2.1 The AS maps an assertion to three claims — no groups anywhere

- `packages/temper-cloud/src/saml/sp.ts :: mapProfileToClaims` returns `MintedClaims { sub, email,
  email_verified }` (`mint.ts:12-16`). `email_verified` is hard-`true` — a validly signed assertion from the
  configured IdP is the verification.
- The ACS handler `POST /oauth/saml/acs` (`packages/temper-cloud/src/oauth/endpoints.ts:133`) validates the
  assertion (`:151`), maps claims (`:153`), and mints the token (`:211`). **No group attribute is read.**
- `kb_saml_idp` (`migrations/20260701000006_saml_as_tables.sql:21-34`) has PK `idp_key TEXT` and columns
  `idp_cert, idp_sso_url, idp_entity_id, sp_entity_id, acs_url, nameid_format, email_attr, stable_id_attr`.
  **There is no `groups_attr`.** The AS loads a single active IdP:
  `SELECT … FROM kb_saml_idp WHERE is_active = true LIMIT 1` (`saml/config.ts :: loadActiveIdp`).

### 2.2 Profile JIT exists; role/team JIT does not

- `crates/temper-services/src/services/profile_service.rs :: resolve_from_claims(pool, &AuthClaims)`
  (`:85`) resolves or creates a profile by `(auth_provider, auth_provider_user_id)`, reconciling by **verified**
  email when the link is absent (`reconcile_by_email :: 131`, gated on `email_verified == Some(true) :: 132`).
  It runs on **every authenticated request** (called from the API auth middleware).
- `AuthClaims` (`crates/temper-core/src/types/auth.rs:28-42`) = `{ provider, external_user_id, email,
  email_verified, exp, iat }`. **No `groups` field.**
- For SAML, `provider` is namespaced `saml:<idp-key>` and `external_user_id` is the stable NameID —
  established in Phase 1 so SAML links never collide with the OIDC `okta`/`auth0` providers.

### 2.3 Team membership is service-direct, and provenance does not exist

- `kb_team_members` (`migrations/20260624000001_canonical_schema.sql:191-198`) = `(team_id, profile_id, role,
  created)`, PK `(team_id, profile_id)` — **one row per (team, user)**. `team_role` enum =
  `owner | maintainer | member | watcher` (`:86`). **There is no `source`/provenance column.**
- Membership writes are **service-direct**: `crates/temper-api/src/handlers/teams.rs:72` calls
  `team_service::add_member` directly — not routed through the resource `Backend`/`DbBackend` trait (that trait
  governs vault resources/edges, not RBAC). So a new provisioning service called by a thin handler matches the
  existing pattern.
- Precedent for reconcile-style membership sync already exists: `sync_system_membership()`
  (`migrations/20260624000002_canonical_functions.sql:58-81`), generalized by
  `20260629000002_auto_join_team_generalization.sql` so **any** team can carry an `auto_join_role`. Phase 2's
  reconcile is conceptually a per-user, group-driven cousin of that pattern.

### 2.4 The access-capability arc landed — but it is not team membership

- `kb_access_grants` (`migrations/20260630000001_access_grants_seam.sql:24-42`) is **resource/context/cogmap**
  access (rwx grants; `subject_table ∈ {kb_resources, kb_contexts, kb_cogmaps}`), **not** team membership.
  Phase 2's provenance marker therefore belongs on `kb_team_members`, not on the grants table. (This corrects a
  loose framing in #224 open-question 6.)

---

## 3. Locked design decisions

These were settled during brainstorming (2026-07-01):

1. **Provenance marker (net-new, Phase-2-owned).** Add `team_member_source` enum (`native | idp`) and a
   `source` column to `kb_team_members`, default `native`. Reconcile only ever touches `source='idp'` rows;
   `native` rows are sacred.
2. **Mapping home = a dedicated table**, `kb_saml_group_mappings`, keyed **per-IdP** on `idp_key`. Unmapped
   asserted groups are ignored; Temper never auto-creates a team from a group.
3. **First admin stays the SQL root step.** SAML is not part of system bootstrap. An `admins` group *may* map to
   owner of the gating team like any other mapping row, but it cannot bootstrap the gating team from nothing.
4. **Schema supports multi-IdP from day one; v1 ships single-IdP.** The mapping table keys on `idp_key`, so
   multi-IdP needs no schema change later — only the AS's single-active-IdP loader would change.
5. **Reconcile seam = C (AS calls an internal Rust endpoint pre-mint).** The AS extracts groups and calls a new
   authenticated temper-api endpoint that does the mapping + membership reconcile in Rust `team`/provisioning
   services, then the AS mints the token. Fallback if the internal channel is ever unwanted: seam B (carry
   `groups` as a signed JWT claim, reconcile in the Rust JIT path). C keeps all RBAC writes in Rust, fires
   exactly once per login, and keeps group names out of the token.
6. **Native-wins-skip** on native/idp overlap: if a `native` row already exists for `(team, user)`, idp reconcile
   skips that team entirely for that user — never overwrites, never deletes. Cost: idp cannot elevate/manage a
   user already native in that team (operator removes the native row to hand the team to idp). Chosen over a
   composite `(team_id, profile_id, source)` PK to avoid dedup-by-max-role churn across every membership query.
7. **Internal auth = shared secret in env** (`INTERNAL_RECONCILE_SECRET`), sent as a header, constant-time
   compared. Trust boundary = same Vercel deployment (AS and API are functions in one project sharing env).
   Alternative (reuse the AS signing key with a distinct audience/scope) rejected for v1 as a privileged-token
   path the endpoint would have to distinguish from user tokens.
8. **Fail-open + log** on reconcile failure: login completes and the token is minted; memberships reconcile on
   the next successful login. Authn never depends on the provisioning path being healthy. No security escalation
   beyond the already-accepted staleness window.

*Added 2026-07-02 after a design review of the "empty groups" edge (confirmed with Cole):*

9. **Signal-missing guard (empty ≠ absent).** `extractGroups` returns `null` when the assertion carries **no
   group signal** — either `groups_attr` is unconfigured, or the named attribute is absent from *this* assertion
   (e.g. a transient IdP misconfiguration). The ACS **skips the reconcile call entirely** on `null`, so a missing
   attribute never revokes memberships. Only an assertion that *carries* the attribute with an empty value list
   (a genuine "member of no mapped groups now") yields `[]` and reconciles, revoking stale `idp` rows. This bounds
   the original §6 "emptied-groups-revokes-all" behavior to a real deprovisioning signal.
10. **Asserted-group discovery capture.** A new `kb_saml_seen_groups (idp_key, group_value, first_seen, last_seen)`
    table is upserted with **every** asserted group value (mapped or not) on each reconcile, so operators discover
    what the IdP actually sends and add mappings reactively via SQL — the mapping table never needs to be
    pre-populated, and an unmapped group is never a hard blocker. This is the lightweight bridge that makes a live
    mapping-management admin endpoint (still deferred, §11) unnecessary for v1.
11. **Profile deactivation is the authn lever, a sibling to reconcile (not part of it).** `kb_profiles` gains
    `is_active BOOLEAN NOT NULL DEFAULT true`; the API auth middleware rejects a resolved-but-deactivated profile
    (`401`). Reconcile manages **authz** (team memberships) and never touches `kb_profiles`; account existence is
    **authn**, controlled by this flag. Built as its own task on this branch. (Corrects a false premise surfaced
    in review: `kb_profiles` had **no** soft-delete — `profile_service.rs:278` said so verbatim, and `Profile.is_active`
    was a hardcoded `true` literal.) Auto-join memberships insert without a `source` ⇒ they are `native` ⇒ never
    touched by reconcile, so a no-groups login keeps every auto-join team.

---

## 4. Architecture — seam C flow

```
IdP ──(signed SAML Response)──▶ AS /oauth/saml/acs  (temper-cloud, TS)
                                   │ 1. validateAssertion  (Phase 1)
                                   │ 2. mapProfileToClaims  (Phase 1)  ─┐
                                   │ 3. extractGroups(profile, idp)     │ new
                                   │ 4. POST /internal/saml/reconcile ──┼──▶ temper-api (Rust)
                                   │        {provider, external_user_id, │       shared-secret gate
                                   │         email, email_verified,      │       ├─ resolve_from_claims (ensure profile)
                                   │         idp_key, groups[]}          │       ├─ load kb_saml_group_mappings[idp_key]
                                   │    (await; on error → log, proceed) │       └─ reconcile kb_team_members (source='idp')
                                   │ 5. mintAccessToken  (Phase 1)      ◀┘
                                   ▼
                              /oauth/token → client → temper-api (JIT backstop still runs)
```

**Once-per-login is structural, not guarded.** Reconcile fires only in the ACS handler. The refresh-token grant
(`/oauth/token` refresh) carries no new assertion, so it does **not** re-reconcile — no per-request guard is
needed. This is the concrete advantage of C over B (where JIT runs per request and would need an `iat`/jti dedup).

**`resolve_from_claims` stays in the API auth path** as the profile-JIT backstop. The reconcile endpoint calling
it first only means the profile exists a moment earlier; a reconcile failure never leaves a tokened user without
a profile.

---

## 5. Schema — one additive migration

`migrations/20260701000007_saml_group_provisioning.sql` (additive-only; safe under the `main` auto-deploy
invariant):

```sql
-- 1. Provenance on team membership. Existing rows are native by definition.
CREATE TYPE team_member_source AS ENUM ('native', 'idp');
ALTER TABLE kb_team_members
    ADD COLUMN source team_member_source NOT NULL DEFAULT 'native';

-- 2. The group→(team, role) mapping, per-IdP. Operator-maintained (SQL in v1).
CREATE TABLE kb_saml_group_mappings (
    idp_key      TEXT      NOT NULL REFERENCES kb_saml_idp(idp_key) ON DELETE CASCADE,
    group_value  TEXT      NOT NULL,   -- the exact asserted group string
    team_id      UUID      NOT NULL REFERENCES kb_teams(id) ON DELETE CASCADE,
    role         team_role NOT NULL,
    created      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (idp_key, group_value, team_id)
);
CREATE INDEX idx_kb_saml_group_mappings_idp ON kb_saml_group_mappings(idp_key);

-- 3. Which assertion attribute carries the group list. NULL ⇒ pure authn (no reconcile).
ALTER TABLE kb_saml_idp ADD COLUMN groups_attr TEXT;

-- 4. Discovery capture (decision 10): every asserted group value (mapped or not) is upserted
--    here on each reconcile, so operators see what the IdP sends and map reactively. Not read
--    by the reconcile diff.
CREATE TABLE kb_saml_seen_groups (
    idp_key     TEXT        NOT NULL REFERENCES kb_saml_idp(idp_key) ON DELETE CASCADE,
    group_value TEXT        NOT NULL,
    first_seen  TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (idp_key, group_value)
);
```

A **second additive migration** `20260702000002_profile_is_active.sql` (decision 11) adds the authn lever:

```sql
ALTER TABLE kb_profiles ADD COLUMN is_active BOOLEAN NOT NULL DEFAULT true;
```

Notes:
- PK `(idp_key, group_value, team_id)` lets one group map into multiple teams, and multiple groups into one team.
- `groups_attr` nullable keeps every Phase-1 IdP working unchanged: no `groups_attr` ⇒ `extractGroups` returns
  `null` ⇒ the AS skips reconcile entirely (decision 9) ⇒ no memberships change.

---

## 6. The reconcile algorithm

Runs in a new `crates/temper-services/src/services/saml_provisioning_service.rs`, transactional per login.

**Inputs:** the resolved `profile_id`, `idp_key`, and the asserted `groups: Vec<String>`. (The service is only
called when the assertion carried a group signal — the AS's signal-missing guard, decision 9, means a `null`
signal never reaches here; an empty `groups` slice IS a genuine "no groups now" and revokes.)

**Discovery capture (decision 10), first:** upsert every value in `groups` (mapped or not) into
`kb_saml_seen_groups` (`ON CONFLICT … DO UPDATE SET last_seen = now()`). Done autonomously (outside the reconcile
transaction) so discovery data survives even if the reconcile below fails.

**Compute the desired IdP-driven membership set:**

1. Load mappings: `SELECT group_value, team_id, role FROM kb_saml_group_mappings WHERE idp_key = $1`.
2. Filter to rows whose `group_value` is in the asserted `groups`.
3. Collapse to `desired: Map<team_id, team_role>` — when two asserted groups map to the same team with different
   roles, **highest role wins** (`owner > maintainer > member > watcher`).

**Reconcile against current IdP memberships** (`WHERE profile_id = $profile AND source = 'idp'`):

For each `team_id`:
- **Native guard first:** if a `source='native'` row exists for `(team_id, profile_id)`, **skip this team
  entirely** (no insert, no update, no delete). Native-wins.
- `desired` has it, no idp row → **INSERT** `(team_id, profile_id, role, source='idp')`.
- `desired` has it, idp row with a different role → **UPDATE** role.
- idp row exists, `desired` lacks it → **DELETE** the idp row (revocation).

**Edge cases:**
- **Attribute absent / `groups_attr` NULL** (no signal) → the AS's guard (decision 9) skips the reconcile call
  entirely; the service is not invoked and nothing is revoked. This is the transient-IdP-misconfiguration-safe path.
- **Attribute present but empty** (`groups = []`, a genuine "in no mapped groups now") → `desired` is empty →
  every existing `source='idp'` row for the user is revoked. Correct deprovisioning. Native/auto-join rows are
  never touched (they are `source='native'`).
- A mapping row referencing a `team_id` that no longer exists cannot occur (FK `ON DELETE CASCADE`).
- A `group_value` asserted but unmapped → ignored (no row in `desired`).

**Role-max helper** is a pure function over `team_role`, unit-tested in isolation.

---

## 7. Surfaces

### 7.1 TS AS (temper-cloud)

- `saml/sp.ts`: new pure `extractGroups(profile, idp): string[] | null` — reads the multi-valued `idp.groups_attr`
  attribute (reusing the existing `readAttr` narrowing). Returns `null` when there is no signal (`groups_attr`
  unconfigured, or the attribute absent from this assertion) and an array (possibly empty `[]`) when the attribute
  is present. Decision 9.
- `saml/config.ts`: add `groups_attr: string | null` to `SamlIdpRow` and to the `loadActiveIdp` SELECT.
- new `oauth/reconcile.ts`: `reconcileMemberships(payload): Promise<void>` — `POST`s to
  `${API_BASE_URL}/internal/saml/reconcile` with the shared-secret header; typed request body (no inline
  `json!`-style objects — a typed interface mirroring the Rust request struct). Throws on non-2xx.
- `oauth/endpoints.ts` ACS handler: after `mapProfileToClaims`, call `extractGroups`; **only when it returns
  non-`null`** call `await reconcileMemberships(...)` inside a `try/catch` that logs (pino) and proceeds on failure
  (fail-open), then mint as today. A `null` result skips reconcile (decision 9) — no call, no revocation.

### 7.2 Rust temper-api

- new `crates/temper-api/src/middleware/internal_auth.rs`: constant-time shared-secret header check against
  `INTERNAL_RECONCILE_SECRET`; 401 on mismatch. Distinct from the JWT `require_auth` middleware.
- new `crates/temper-api/src/handlers/internal_saml.rs`: `POST /internal/saml/reconcile`, thin — deserializes a
  typed `ReconcileRequest` (defined in `temper-core` with `ts-rs` derives — see §7.4),
  builds `AuthClaims`, calls `profile_service::resolve_from_claims`, then
  `saml_provisioning_service::reconcile_idp_memberships`. Returns 204.
- route registration alongside the existing `/oauth/*` app wiring; the internal route is **not** behind
  `require_auth`.

### 7.3 Rust temper-services

- new `saml_provisioning_service.rs` (§6). SQL via `sqlx::query!`/`query_as!` macros (compile-time checked;
  regenerate `.sqlx` per the workspace ritual + `cargo make prepare-services` since new service SQL is added).

### 7.4 Types

- `ReconcileRequest { provider, external_user_id, email, email_verified, idp_key, groups }` is a boundary type
  (Rust ↔ TS). Per the shared-types rule it lives in `temper-core` with `ts-rs` derives; the TS side imports the
  generated type rather than hand-writing a mirror.

### 7.5 Profile deactivation (authn control, decision 11)

Sibling to reconcile, deliberately separate from it:

- `kb_profiles.is_active BOOLEAN NOT NULL DEFAULT true` (second migration).
- `profile_service::get_by_id` reads the real column instead of the current hardcoded `true AS "is_active!"`
  literal; because every `resolve_from_claims` path returns through `get_by_id`, this surfaces the flag on all
  resolve paths. The `Profile` domain type already has `pub is_active: bool` — no type change.
- `middleware/auth.rs`: immediately after `resolve_from_claims`, reject a `!profile.is_active` profile with `401`
  (before injecting `AuthenticatedProfile`). This is the authn choke point: a deactivated account's token is
  useless on every API request, regardless of what the AS minted. Deactivation never deletes the profile or its
  memberships; re-activating restores access.

---

## 8. Config surface (operator SQL, v1)

Consistent with Phase 1 (IdP config is hand-written SQL) and with first-admin-is-SQL. No CLI/API management
surface for mappings in v1 — that is a clean later enhancement. Documented in `docs/guides/self-hosting-saml.md`
beside the existing `kb_saml_idp` INSERT:

```sql
-- Tell the SP which assertion attribute carries group membership.
UPDATE kb_saml_idp SET groups_attr = 'groups' WHERE idp_key = 'acme-okta';

-- Map IdP groups to Temper (team, role). Teams must already exist (created via the team surface / SQL).
INSERT INTO kb_saml_group_mappings (idp_key, group_value, team_id, role) VALUES
  ('acme-okta', 'engineering', '<team-uuid>', 'member'),
  ('acme-okta', 'eng-leads',   '<team-uuid>', 'maintainer'),
  ('acme-okta', 'temper-admins', '<gating-team-uuid>', 'owner');  -- admin-via-group (§3.3)
```

New env var: `INTERNAL_RECONCILE_SECRET` (both the AS function and the API function read it from shared project
env). Documented in the self-hosting env-var table.

---

## 9. Error handling

- **Reconcile call fails** (network, 5xx, timeout): AS logs (pino, structured) and proceeds to mint — fail-open.
- **Malformed / partial mapping data:** the reconcile service skips individually bad mapping rows and logs
  (tracing); one bad row never fails the whole reconcile or the login.
- **Shared-secret mismatch:** endpoint returns 401; AS treats it as a reconcile failure (fail-open) and logs at
  error level (this is a misconfiguration, not a user problem).
- **DB error mid-reconcile:** the reconcile transaction rolls back (no partial membership state); the endpoint
  returns 5xx; AS fails open. Memberships are unchanged, not half-applied.

---

## 10. Testing

- **Unit (Rust, no DB):** the pure reconcile-diff (add/update/delete/native-skip) and the role-max collapse —
  table-driven over hand-built `current`/`desired` sets.
- **Integration (Rust, `test-db`):** `reconcile_idp_memberships` against real Postgres — assert native rows
  survive untouched, idp rows are inserted/updated/revoked, an emptied group set revokes all idp rows, and a
  native+idp overlap on one team leaves the native row intact.
- **e2e (SAML mock-IdP harness from Phase 1, embed/SAML job):** extend the mock assertion with a `groups`
  attribute; drive a full login and assert `kb_team_members` reflects the mapping; then a second login with the
  group removed revokes only the idp row. (Run under `cargo make test-e2e-embed` — the SAML e2e path is in the
  embed-gated tier.)
- **TS (temper-cloud, Vitest):** `extractGroups` reads multi-valued attributes and returns `[]` when
  `groups_attr` is null; `reconcileMemberships` posts the correct typed payload with the secret header; the ACS
  handler mints the token even when the reconcile call rejects (fail-open).
- **SQL cache:** regenerate `.sqlx` (`cargo sqlx prepare --workspace -- --all-features` then
  `cargo make prepare-services`, and `cargo make prepare-e2e` if the e2e suite gains macro queries).

---

## 11. Out of scope (explicit)

- **SCIM / immediate deprovisioning** — Phase 3. Reconcile-on-login's staleness window is accepted here.
- **Mapping-management CLI/API** — v1 is operator SQL; a `temper-as` surface is a later enhancement. The
  `kb_saml_seen_groups` discovery table (decision 10) is the v1 bridge: operators read observed group values from
  it and add mappings reactively, so a live admin endpoint is not needed to avoid pre-populating mappings.
- **Multi-IdP ACS routing** — schema supports it; v1 ships the single-active-IdP loader unchanged.
- **Encrypted assertions**, **IdP-initiated flow changes** — unchanged from Phase 1.
- **Composite (team, user, source) membership** — rejected in favor of native-wins-skip (§3.6).

---

## 12. Open items / carry-forwards

- **Admin-via-group + the org-provisioning-bootstrap arc.** The `temper-admins → gating-team owner` mapping row
  works mechanically, but the gating team and its slug are configured by the org-provisioning surface
  (`docs/superpowers/specs/2026-06-28-org-provisioning-bootstrap-surface-design.md`, still design-stage). Phase 2
  does not depend on that arc landing — it just documents that the *first* owner remains the SQL root step.
- **Emptied-groups semantics** (§6) — resolved by decision 9: an *absent* attribute is no-signal (skip, never
  revoke), a *present-but-empty* attribute is a genuine revoke. A future per-IdP "never revoke even on empty" flag
  remains possible but is not v1.
- **Multi-IdP reconcile scoping (corrects §3.4).** The reconcile diff reads *all* of a profile's non-`native`
  `kb_team_members` rows and treats them as belonging to the asserting IdP — correct under the single active IdP v1
  ships, but `kb_team_members` has no `idp_key` discriminator, so a second active IdP would let one IdP's login
  revoke another IdP's provisioned rows. §3.4's "multi-IdP needs no schema change later — only the loader changes"
  is therefore inaccurate: enabling multi-IdP also requires an `idp_key` column on `kb_team_members` (or equivalent)
  and scoping the reconcile read/revoke by it. Carry-forward for the multi-IdP arc; no v1 action.

---

## 13. References

**Code (`main` @ this checkout):**
- `packages/temper-cloud/src/saml/{sp,config}.ts`, `src/oauth/{endpoints,mint}.ts` — Phase 1 AS
- `crates/temper-services/src/services/profile_service.rs` — `resolve_from_claims`
- `crates/temper-services/src/services/team_service.rs`, `crates/temper-api/src/handlers/teams.rs` — membership writes
- `crates/temper-core/src/types/auth.rs` — `AuthClaims`
- `migrations/20260624000001_canonical_schema.sql` — `kb_team_members`, `team_role`
- `migrations/20260701000006_saml_as_tables.sql` — `kb_saml_idp`
- `migrations/20260630000001_access_grants_seam.sql` — `kb_access_grants` (resource access, not membership)
- `migrations/{20260624000002,20260629000002}` — `sync_system_membership` / auto-join generalization (reconcile precedent)

**Specs:**
- [2026-07-01-saml-sp-temper-authorization-server-design.md](2026-07-01-saml-sp-temper-authorization-server-design.md) — Phase 0+1
- [2026-06-28-org-provisioning-bootstrap-surface-design.md](2026-06-28-org-provisioning-bootstrap-surface-design.md) — gating-team/admin bootstrap
- [2026-06-30-generalized-access-capability-model-design.md](2026-06-30-generalized-access-capability-model-design.md) — `kb_access_grants` (rwx resource grants)

**Issue:** #224 — SAML SP with profile, role, and team provisioning.
